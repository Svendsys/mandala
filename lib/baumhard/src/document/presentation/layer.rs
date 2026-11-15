// SPDX-License-Identifier: MPL-2.0

//! The presentation layer of a document (#251 §3; design §10.2): the
//! instances playing on it, in the order they started, each applied on
//! top of the map and none of it part of the map.
//!
//! ```text
//! committed document ⊕ open transaction ⊕ presentation layer   →  projection
//! ```
//!
//! **A document-layer instance holds the journal of its current
//! application.** Applying an instance at progress `t` runs its mutator
//! at its elements under a journal — the state at `t = 1` — and writes
//! that journal at `t`
//! ([`Journal::write_interpolated`]). The journals stack in start order:
//! each one's before-images are the document with every older instance
//! applied.
//!
//! **Any change underneath** — an apply, a stage, a commit, a cancel, an
//! undo, a redo — runs inside the writer's one funnel, which writes the
//! layer back out first, newest instance first, makes the change, and
//! applies every instance again at its `t`, oldest first. So a history
//! journal holds the change alone, a save writes the map alone, and a
//! presentation always sits on the map as it now stands. When the call
//! changed nothing in the tree — a save, a refused request — the
//! journals set aside are written forward again as they are.
//!
//! **A frame in which some instances moved** writes back out from the
//! oldest instance that moved through the newest, and applies those
//! again. Instances started before it are not touched.
//!
//! **Nothing here is in the history, the revision, the saved state or
//! the document's derived indexes.** What the layer writes reaches the
//! change feed as records of
//! [`ChangeSource::Presentation`], at the revision the document already
//! has, naming only the elements whose state a write changed; the
//! projector draws from them as from any record.
//! [`Document::presentation_epoch`] counts what the instances
//! themselves did.
//!
//! **The layer can be set aside** ([`Document::suspend_presentations`]):
//! whoever is about to read the map to build a change from what it
//! reads — a copy, a layout, a drag's start — reads the map itself.
//!
//! **A glyph-layer instance writes nothing here.** It lives its
//! lifecycle in this list, and whoever owns the glyph layers reads it
//! ([`Document::presentations`]) and applies its run mutations there.

use crate::arena::NodeId;
use crate::document::edit::reach::elements;
use crate::document::edit::resolve;
use crate::document::edit::Reach;
use crate::document::element::DocElement;
use crate::document::ids::Document;
use crate::document::mutation::error::MutationError;
use crate::document::mutation::inherited::inherited_seed;
use crate::document::mutation::ops::DocMutatorTree;
use crate::document::mutation::validate::{validate_applied, validate_for_presentation};
use crate::document::presentation::lifecycle::{Direction, Lifecycle};
use crate::document::presentation::{
    takes_time, Play, PresentError, PresentationBody, PresentationDef, PresentationLayer, LOOP_TAKES_NO_TIME,
};
use crate::document::writer::error::ApplyError;
use crate::document::writer::feed::field_inputs;
use crate::document::writer::{Change, ChangeSource, Writer};
use crate::engine::anchor::Anchors;
use crate::engine::element::TreeElement;
use crate::engine::interpolate::Snap;
use crate::engine::journal::{Journal, JournalBuilder, Touch};
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use crate::engine::tree::{MutatorTree, Tree};
use crate::gfx_structs::mutator::GfxMutation;
use log::{error, warn};
use rustc_hash::FxHashSet;
use std::borrow::Cow;
use std::sync::Arc;

/// One instance: one presentation started on one node. Unique among the
/// instances a document ever held, and increasing in start order.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstanceId(pub u64);

/// An instance, as [`Document::presentations`] reports it.
#[derive(Copy, Clone, Debug)]
pub struct InstanceInfo<'a> {
    /// Which instance.
    pub id: InstanceId,
    /// The presentation's id.
    pub presentation: &'a str,
    /// The node it was started on.
    pub node: NodeId,
    /// The vocabulary of its body.
    pub layer: PresentationLayer,
    /// How it lives.
    pub play: Play,
    /// Its progress, `0..=1`, at the last clock reading it was given.
    pub t: f32,
    /// Which way it is going.
    pub direction: Direction,
    /// Whether its `t` moves with time: on its way in or out.
    pub moving: bool,
    /// Whether it plays on until somebody stops it: a loop that was not
    /// stopped.
    pub unending: bool,
    /// When the fields that do not blend switch, on the way it is going.
    pub snap: Snap,
    /// The elements it plays on: its target within its node, as its
    /// filter kept them when it started, in tree order.
    pub elements: &'a [NodeId],
    /// A glyph-layer instance's run mutations; `None` on the document
    /// layer.
    pub glyph: Option<&'a [GfxMutation]>,
    /// Why the document refuses what it writes, on the map as it now
    /// stands: it is applied nowhere until an edit makes room for it.
    pub refusal: Option<&'a ApplyError>,
}

/// What a start made of a definition, shared by the instances of that
/// start: the body with its names resolved against the map.
#[derive(Debug)]
struct Started {
    id: String,
    play: Play,
    body: Body,
}

/// A body, ready to apply.
#[derive(Debug)]
enum Body {
    /// One macro of the definition's mutations, keyed to its target's
    /// kind: applied at each element.
    Document(DocMutatorTree),
    /// The run mutations, in order.
    Glyph(Vec<GfxMutation>),
}

/// A document-layer instance's current application.
#[derive(Clone, Debug)]
struct Applied {
    /// What applying it at `t = 1` changed.
    journal: Journal<DocElement>,
    /// How far the journal is written.
    t: f32,
    /// The snap it was written with.
    snap: Snap,
}

impl Applied {
    /// What this application makes of `element`, as far as a comparison
    /// needs it: nothing when the journal does not hold the element or is
    /// written at `0`, where the element stands as whatever is under it.
    fn part(&self, element: NodeId) -> Option<(&DocElement, &DocElement, f32, Snap)> {
        if self.t <= 0.0 {
            return None;
        }
        let before = self.journal.before(element)?;
        let after = self.journal.after(element)?;
        Some((before, after, self.t, self.snap))
    }
}

/// One instance.
#[derive(Clone, Debug)]
struct Instance {
    id: InstanceId,
    started: Arc<Started>,
    node: NodeId,
    elements: Vec<NodeId>,
    life: Lifecycle,
    applied: Option<Applied>,
    refusal: Option<ApplyError>,
}

impl Instance {
    fn on_document(&self) -> bool {
        matches!(self.started.body, Body::Document(_))
    }
}

/// Applications written back out of the tree, oldest first, and the
/// tree's stamp once they were out.
#[derive(Clone, Debug, Default)]
struct Lifted {
    journals: Vec<(InstanceId, Applied)>,
    stamp: u64,
}

/// A layer set aside by a suspension.
#[derive(Clone, Debug)]
struct Aside {
    lifted: Lifted,
    /// Where the record of the setting aside sits in the change feed,
    /// and how often the feed had been drained then; `None` when setting
    /// aside changed nothing, and so recorded nothing.
    record: Option<(usize, u64)>,
}

/// The layer's state, beside the tree. See the [module docs](self).
#[derive(Clone, Debug, Default)]
pub(in crate::document) struct Presentations {
    /// The instances, in start order.
    instances: Vec<Instance>,
    next_id: u64,
    epoch: u64,
    /// How many suspensions are open.
    suspensions: u32,
    aside: Option<Aside>,
}

impl Document {
    /// Start `def` on each of `nodes` — an element stands for the node
    /// it belongs to — at the clock reading `now_ms`, and return the
    /// instance of each node it now plays on, in the order given: one
    /// instance per node (amendment 2 on #251).
    ///
    /// - On a node where it is already playing it **restarts** in place:
    ///   the instance there turns to its way in from where it is. It
    ///   never stacks a second one.
    /// - Its elements are resolved once, here: its target within the
    ///   node, kept by its filter, read from the map itself with every
    ///   presentation written back out. A node where that names nothing
    ///   gets no instance, and neither does an id that is not in the
    ///   tree.
    /// - Its names are resolved against this map, and a document-layer
    ///   body is judged as the writer would judge a request, structural
    ///   commands refused
    ///   ([`validate_for_presentation`]).
    ///
    /// All or nothing: a refusal starts no instance and restarts none.
    ///
    /// Errors: [`PresentError::UnknownName`] and [`PresentError::Value`]
    /// for a name this map lacks, a value its field does not take, a
    /// body that is not its layer's, a loop that takes no time;
    /// [`PresentError::Refused`] for what the document refuses — the
    /// words are the writer's, and on wasm32 they include the platform's
    /// bounds ([`ApplyError::TooManyElements`],
    /// [`ApplyError::TooMuchText`]).
    ///
    /// Cost: the layer's reach, out and back, and per node the
    /// resolution of its elements and one application.
    pub fn start_presentation(
        &mut self,
        def: &PresentationDef,
        nodes: &[NodeId],
        now_ms: f64,
    ) -> Result<Vec<InstanceId>, PresentError> {
        let lifted = (self.presentations.suspensions == 0).then(|| self.lift_layer(0));
        let result = self.start_on_the_map(def, nodes, now_ms);
        if let Some(lifted) = lifted {
            let change = self.restore_layer(0, lifted);
            self.push_change(ChangeSource::Presentation, change);
        }
        if result.as_ref().is_ok_and(|started| !started.is_empty()) {
            self.presentations.epoch += 1;
        }
        result
    }

    /// The body of [`Self::start_presentation`], with the layer out of
    /// the tree: resolve, judge, and add or restart the instances.
    fn start_on_the_map(
        &mut self,
        def: &PresentationDef,
        nodes: &[NodeId],
        now_ms: f64,
    ) -> Result<Vec<InstanceId>, PresentError> {
        let value = |field: &'static str, message: &str| PresentError::Value {
            presentation: def.id.clone(),
            field,
            message: message.to_string(),
        };
        let out = def.out();
        if !takes_time(def.play, &def.animate, &out) {
            return Err(value("play", LOOP_TAKES_NO_TIME));
        }
        let body = match (def.layer, &def.body) {
            (PresentationLayer::Document, PresentationBody::Mutations(list)) => {
                let mutations = list
                    .0
                    .iter()
                    .map(|m| resolve::mutation(m, Some(self)))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|fault| PresentError::of(fault, &def.id))?;
                Body::Document(MutatorTree::new_with(Mutator::new_macro(
                    mutations,
                    BranchKey::new(def.target.kind(), 0),
                )))
            }
            (PresentationLayer::Glyph, PresentationBody::Glyph(ops)) => Body::Glyph(ops.mutations()),
            (PresentationLayer::Document, PresentationBody::Glyph(_)) => {
                return Err(value(
                    "layer",
                    "the document layer takes `mutations`, not `glyph`",
                ));
            }
            (PresentationLayer::Glyph, PresentationBody::Mutations(_)) => {
                return Err(value("layer", "the glyph layer takes `glyph`, not `mutations`"));
            }
        };
        let filter = match &def.filter {
            Some(filter) => Some(
                resolve::predicate(&filter.0, Some(self))
                    .map_err(|fault| PresentError::of(fault, &def.id))?,
            ),
            None => None,
        };
        let started = Arc::new(Started {
            id: def.id.clone(),
            play: def.play,
            body,
        });
        let mut seen: FxHashSet<NodeId> = FxHashSet::default();
        // What each node becomes, decided for all of them before any
        // instance is touched.
        enum Planned {
            Restart(usize),
            Fresh(NodeId, Vec<NodeId>),
        }
        let mut plan: Vec<Planned> = Vec::new();
        for &given in nodes {
            let Some(node) = self.owning_node(given) else {
                warn!(
                    "presentation `{}`: {given} is not in a node of this map; skipped",
                    def.id
                );
                continue;
            };
            if !seen.insert(node) {
                continue;
            }
            let playing = self
                .presentations
                .instances
                .iter()
                .position(|instance| instance.node == node && instance.started.id == def.id);
            if let Some(index) = playing {
                plan.push(Planned::Restart(index));
                continue;
            }
            let reached = elements(self, &[node], Reach::SelfNode, def.target, filter.as_ref());
            if reached.is_empty() {
                continue;
            }
            self.try_application(&started, &reached)
                .map_err(|error| PresentError::Refused {
                    presentation: def.id.clone(),
                    error,
                })?;
            plan.push(Planned::Fresh(node, reached));
        }
        let layer = &mut self.presentations;
        let mut ids = Vec::with_capacity(plan.len());
        for planned in plan {
            match planned {
                Planned::Restart(index) => {
                    let instance = &mut layer.instances[index];
                    instance.life.restart(now_ms);
                    ids.push(instance.id);
                }
                Planned::Fresh(node, reached) => {
                    let id = InstanceId(layer.next_id);
                    layer.next_id += 1;
                    layer.instances.push(Instance {
                        id,
                        started: Arc::clone(&started),
                        node,
                        elements: reached,
                        life: Lifecycle::start(def.play, def.animate, out, now_ms),
                        applied: None,
                        refusal: None,
                    });
                    ids.push(id);
                }
            }
        }
        // One that is over as it starts — an instant `once` — played
        // nothing, and is not kept.
        layer.instances.retain(|instance| !instance.life.ended());
        Ok(ids)
    }

    /// Stop the presentation `id` on each of `nodes` — an element stands
    /// for its node — at the clock reading `now_ms`: each instance there
    /// plays its way out from where it is, and a loop finishes the cycle
    /// it is in ([`Lifecycle`]'s rules). Returns how many instances it
    /// stopped. Nothing is written here; the next
    /// [`Self::tick_presentations`] writes where they are.
    ///
    /// Cost: O(the instances) per node.
    pub fn stop_presentation(&mut self, id: &str, nodes: &[NodeId], now_ms: f64) -> usize {
        let owners: FxHashSet<NodeId> = nodes
            .iter()
            .filter_map(|&given| self.owning_node(given))
            .collect();
        let mut stopped = 0;
        for instance in &mut self.presentations.instances {
            if instance.started.id == id && owners.contains(&instance.node) {
                instance.life.stop(now_ms);
                stopped += 1;
            }
        }
        if stopped > 0 {
            self.presentations.epoch += 1;
        }
        stopped
    }

    /// Bring every instance to the clock reading `now_ms`: write the
    /// document-layer instances whose progress moved — back out from the
    /// oldest that moved through the newest, then on again (see the
    /// [module docs](self)) — and drop the ones that ended. Returns
    /// whether a frame owes a draw: something was written here, or a
    /// glyph-layer instance moved or ended.
    ///
    /// Cost: O(the instances) to read the clock into each, and the reach
    /// of the instances from the oldest that moved on.
    pub fn tick_presentations(&mut self, now_ms: f64) -> bool {
        self.drop_instances_of_freed_nodes();
        let layer = &mut self.presentations;
        let mut first_written: Option<usize> = None;
        let mut owes_draw = false;
        let mut moved = false;
        for (index, instance) in layer.instances.iter_mut().enumerate() {
            let (direction, was) = (instance.life.direction(), instance.life.t());
            let t = instance.life.advance(now_ms);
            let ended = instance.life.ended();
            moved |= ended || t != was || instance.life.direction() != direction;
            let rewrite = match (&instance.applied, instance.on_document()) {
                (Some(applied), _) => ended || applied.t != t || applied.snap != instance.life.snap(),
                (None, true) => false,
                (None, false) => ended || t != was,
            };
            if rewrite {
                owes_draw = true;
                if instance.applied.is_some() && first_written.is_none() {
                    first_written = Some(index);
                }
            }
        }
        if !moved {
            return false;
        }
        layer.epoch += 1;
        let Some(first) = first_written else {
            layer.instances.retain(|instance| !instance.life.ended());
            return owes_draw;
        };
        let lifted = self.lift_layer(first);
        // The ended ones before `first` hold no application: dropping
        // them moves `first` down by as many.
        let instances = &mut self.presentations.instances;
        let dropped_before = instances[..first]
            .iter()
            .filter(|instance| instance.life.ended())
            .count();
        instances.retain(|instance| !instance.life.ended());
        let change = self.restore_layer(first - dropped_before, lifted);
        self.push_change(ChangeSource::Presentation, change);
        owes_draw
    }

    /// The instances playing, in start order: what `active()` is in #251
    /// §5. No allocation.
    pub fn presentations(&self) -> impl Iterator<Item = InstanceInfo<'_>> + '_ {
        self.presentations.instances.iter().map(|instance| InstanceInfo {
            id: instance.id,
            presentation: &instance.started.id,
            node: instance.node,
            layer: match instance.started.body {
                Body::Document(_) => PresentationLayer::Document,
                Body::Glyph(_) => PresentationLayer::Glyph,
            },
            play: instance.started.play,
            t: instance.life.t(),
            direction: instance.life.direction(),
            moving: instance.life.moving(),
            unending: instance.life.unending(),
            snap: instance.life.snap(),
            elements: &instance.elements,
            glyph: match &instance.started.body {
                Body::Document(_) => None,
                Body::Glyph(ops) => Some(ops.as_slice()),
            },
            refusal: instance.refusal.as_ref(),
        })
    }

    /// How often the instances themselves have changed: advanced by
    /// every start and restart, every stop, and every clock reading at
    /// which an instance moved, turned or ended. Never by an edit
    /// underneath, and the document's revision never by any of these.
    /// O(1).
    pub fn presentation_epoch(&self) -> u64 {
        self.presentations.epoch
    }

    /// Whether an instance is on its way in or out, so that time moves
    /// what is drawn. O(the instances).
    pub fn presentations_moving(&self) -> bool {
        self.presentations
            .instances
            .iter()
            .any(|instance| instance.life.moving())
    }

    /// Set the document layer aside: every instance is written back out
    /// of the tree, its journal, its place and its progress kept, until
    /// the matching [`Self::resume_presentations`]. In between the
    /// document reads as the map itself, the writer's funnel does nothing
    /// extra for the layer, and the instances go on living their
    /// lifecycles unseen.
    ///
    /// **What it is for**: a change built from values read beforehand —
    /// a copy, a layout that reads positions, a drag's start — reads the
    /// map, never a performance of it, so nothing a presentation shows
    /// can be committed (ruling D2.1 §2 on #251).
    ///
    /// Suspensions nest: the layer comes back with the resume that
    /// matches the first. **A suspension that is never resumed leaves
    /// the layer out**: nothing of it is drawn, though every instance is
    /// still listed. A caller that can return early or unwind between
    /// the two holds [`Self::suspended`]'s guard instead, which resumes
    /// when it is dropped.
    ///
    /// The feed gets one record of what coming out changed, so a
    /// projection made in between is of the map itself.
    ///
    /// Cost: the layer's reach.
    pub fn suspend_presentations(&mut self) {
        self.presentations.suspensions += 1;
        if self.presentations.suspensions > 1 {
            return;
        }
        let lifted = self.lift_layer(0);
        let mut change = Change::default();
        for (_, applied) in &lifted.journals {
            touch_all(&self.tree, applied, &mut change);
        }
        let record = (!change.touched.is_empty()).then(|| (self.writer.feed.len(), self.writer.drains));
        self.push_change(ChangeSource::Presentation, change);
        self.presentations.aside = Some(Aside { lifted, record });
    }

    /// Put the layer back after [`Self::suspend_presentations`]: every
    /// instance is applied again at the progress it now has, on the map
    /// as it now stands. Nothing for a resume no suspension matches.
    ///
    /// When nothing drained the feed in between, the record of the
    /// setting aside is taken back out of it and this records only what
    /// differs from before the suspension — nothing at all, when the map
    /// did not change and no instance moved.
    ///
    /// Cost: the layer's reach.
    pub fn resume_presentations(&mut self) {
        let layer = &mut self.presentations;
        if layer.suspensions == 0 {
            return;
        }
        layer.suspensions -= 1;
        if layer.suspensions > 0 {
            return;
        }
        let Some(Aside { lifted, record }) = layer.aside.take() else {
            return;
        };
        let mut change = self.restore_layer(0, lifted);
        match record {
            // Nobody read the record of the setting aside: it comes back
            // out of the feed, and what is left is the difference from
            // before the suspension.
            Some((index, drains)) if drains == self.writer.drains && index < self.writer.feed.len() => {
                self.writer.feed.remove(index);
            }
            // It was read: whoever read it holds the map itself, so
            // everything the layer shows is news to it.
            Some(_) => {
                change = Change::default();
                for instance in &self.presentations.instances {
                    if let Some(applied) = &instance.applied {
                        touch_all(&self.tree, applied, &mut change);
                    }
                }
            }
            // Setting aside changed nothing, so nothing was recorded:
            // the difference from then is the whole of it.
            None => {}
        }
        self.push_change(ChangeSource::Presentation, change);
    }

    /// [`Self::suspend_presentations`], as a guard: the document with
    /// its layer set aside, which is put back when the guard is dropped
    /// — at the end of its scope, on an early return, and while a panic
    /// unwinds.
    pub fn suspended(&mut self) -> Suspended<'_> {
        self.suspend_presentations();
        Suspended { doc: self }
    }

    /// The map itself, to read: this document when no presentation is
    /// applied on it, and otherwise a copy of its tree with the layer
    /// written back out — the map as a save would write it, a change an
    /// open transaction holds included. For a reader that holds the
    /// document shared and must describe the map, never the performance:
    /// [`format::save`](crate::document::format::save) reads through it,
    /// so no save of any kind writes what a presentation shows.
    ///
    /// The copy carries no history, no feed and no instances: it is for
    /// reading.
    ///
    /// Cost: O(1) with nothing applied; otherwise one copy of the tree
    /// and its indexes, and the layer's reach.
    pub fn without_presentations(&self) -> Cow<'_, Document> {
        let applied = || {
            self.presentations
                .instances
                .iter()
                .rev()
                .filter_map(|instance| instance.applied.as_ref())
        };
        if applied().next().is_none() {
            return Cow::Borrowed(self);
        }
        let mut copy = Document {
            tree: self.tree.clone(),
            ident_index: self.ident_index.clone(),
            next_ident: self.next_ident,
            definitions: self.definitions.clone(),
            derived: self.derived.clone(),
            writer: Writer::default(),
            presentations: Presentations::default(),
        };
        for applied in applied() {
            // A copy of the journal, so the instance's own keeps the
            // side it was written on.
            let journal = applied.journal.clone();
            if let Err(error) = journal.revert(&mut copy.tree) {
                error!("presentation: could not be written back out of a copy of the map: {error}");
            }
        }
        Cow::Owned(copy)
    }

    /// Whether the document layer is set aside. O(1).
    pub fn presentations_suspended(&self) -> bool {
        self.presentations.suspensions > 0
    }

    /// Whether the writer's funnel has a layer to take out and put back:
    /// it is not set aside, and an instance writes the document — one
    /// applied nowhere now included, since the change may be what lets
    /// it apply. O(the instances).
    pub(in crate::document) fn layer_follows_changes(&self) -> bool {
        self.presentations.suspensions == 0 && self.presentations.instances.iter().any(Instance::on_document)
    }

    /// The funnel's first half: write the whole layer back out.
    pub(in crate::document) fn lift_presentations(&mut self) -> LayerOut {
        LayerOut(self.lift_layer(0))
    }

    /// The funnel's second half: apply every instance again, and record
    /// what the layer now shows differently.
    pub(in crate::document) fn restore_presentations(&mut self, out: LayerOut) {
        self.drop_instances_of_freed_nodes();
        let change = self.restore_layer(0, out.0);
        self.push_change(ChangeSource::Presentation, change);
    }

    /// Write the applications of the instances from `from` on back out
    /// of the tree, newest first, and hand them over.
    fn lift_layer(&mut self, from: usize) -> Lifted {
        let mut journals = Vec::new();
        for instance in self.presentations.instances[from..].iter_mut().rev() {
            let Some(applied) = instance.applied.take() else {
                continue;
            };
            #[cfg(test)]
            work::count(work::Kind::Lifted);
            if let Err(error) = applied.journal.revert(&mut self.tree) {
                error!(
                    "presentation `{}`: could not be written back out: {error}",
                    instance.started.id
                );
            }
            journals.push((instance.id, applied));
        }
        journals.reverse();
        Lifted {
            journals,
            stamp: self.tree.stamp(),
        }
    }

    /// Apply the instances from `from` on, oldest first, each at the
    /// progress it has, given what [`Self::lift_layer`] took out — of
    /// those instances, and of any dropped since. Returns what the layer
    /// now shows differently from then.
    ///
    /// An application set aside is written forward again as it is when
    /// nothing under it can have changed: the tree stands where the lift
    /// left it, every instance before it stands on the side it stood on,
    /// and none of them wrote an element this one holds differently.
    /// Otherwise the instance is applied afresh.
    fn restore_layer(&mut self, from: usize, lifted: Lifted) -> Change {
        let mut change = Change::default();
        let mut old = lifted.journals.into_iter().peekable();
        // Whether the tree is, stamp for stamp, where each journal set
        // aside expects it.
        let mut in_step = self.tree.stamp() == lifted.stamp;
        // The elements written differently from before, so far.
        let mut rewritten: FxHashSet<NodeId> = FxHashSet::default();
        for index in from..self.presentations.instances.len() {
            let instance = &self.presentations.instances[index];
            let (id, t, snap) = (instance.id, instance.life.t(), instance.life.snap());
            let on_document = instance.on_document();
            // The ones set aside are in start order, as the instances
            // are: any before this one belongs to an instance dropped.
            let mut previous = None;
            while let Some((old_id, _)) = old.peek() {
                match (*old_id).cmp(&id) {
                    std::cmp::Ordering::Less => {
                        if let Some((_, gone)) = old.next() {
                            touch_all(&self.tree, &gone, &mut change);
                            rewritten.extend(gone.journal.touched().map(|(element, _)| element));
                            in_step &= !gone.snap.takes_after(gone.t);
                            gone.journal.discard(&mut self.tree);
                        }
                    }
                    std::cmp::Ordering::Equal => previous = old.next().map(|(_, applied)| applied),
                    std::cmp::Ordering::Greater => break,
                }
            }
            if !on_document {
                continue;
            }
            let kept = previous.as_ref().is_some_and(|applied| {
                in_step
                    && applied
                        .journal
                        .touched()
                        .all(|(element, _)| !rewritten.contains(&element))
            });
            let written = match previous {
                Some(applied) if kept => match applied.journal.write_interpolated(&mut self.tree, t, snap) {
                    Ok(()) => {
                        #[cfg(test)]
                        work::count(work::Kind::Rewritten);
                        let same_side = snap.takes_after(t) == applied.snap.takes_after(applied.t);
                        if applied.t != t || applied.snap != snap {
                            let now = Applied {
                                journal: applied.journal,
                                t,
                                snap,
                            };
                            touch_all(&self.tree, &now, &mut change);
                            rewritten.extend(now.journal.touched().map(|(element, _)| element));
                            in_step &= same_side;
                            Some(now)
                        } else {
                            Some(applied)
                        }
                    }
                    Err(error) => {
                        error!("presentation: an application set aside could not be written again: {error}");
                        in_step = false;
                        let fresh = self.apply_instance(index, t, snap);
                        self.compare(Some(&applied), fresh.as_ref(), &mut change, &mut rewritten);
                        applied.journal.discard(&mut self.tree);
                        fresh
                    }
                },
                previous => {
                    let fresh = self.apply_instance(index, t, snap);
                    // A journal made now is at stamps no journal set
                    // aside knows.
                    in_step &= previous.is_none() && fresh.is_none();
                    self.compare(previous.as_ref(), fresh.as_ref(), &mut change, &mut rewritten);
                    if let Some(applied) = previous {
                        applied.journal.discard(&mut self.tree);
                    }
                    fresh
                }
            };
            self.presentations.instances[index].applied = written;
        }
        for (_, gone) in old {
            touch_all(&self.tree, &gone, &mut change);
            gone.journal.discard(&mut self.tree);
        }
        change
    }

    /// Note in `change`, and in `rewritten`, every element the instance
    /// shows differently now that `fresh` stands where `previous` stood:
    /// one whose two images, progress or snap differ between the two, or
    /// that only one of them holds.
    fn compare(
        &self,
        previous: Option<&Applied>,
        fresh: Option<&Applied>,
        change: &mut Change,
        rewritten: &mut FxHashSet<NodeId>,
    ) {
        let held = |applied: Option<&Applied>| {
            applied
                .into_iter()
                .flat_map(|applied| applied.journal.touched().map(|(element, _)| element))
                .collect::<Vec<NodeId>>()
        };
        let mut seen: FxHashSet<NodeId> = FxHashSet::default();
        for element in held(previous).into_iter().chain(held(fresh)) {
            if !seen.insert(element) {
                continue;
            }
            let was = previous.and_then(|applied| applied.part(element));
            let is = fresh.and_then(|applied| applied.part(element));
            let same = match (was, is) {
                (None, None) => true,
                // Written whole, an application shows its after-image,
                // whatever it stands on.
                (Some((before_a, after_a, t_a, snap_a)), Some((before_b, after_b, t_b, snap_b))) => {
                    t_a == t_b
                        && snap_a == snap_b
                        && after_a.same_state(after_b)
                        && (t_a >= 1.0 || before_a.same_state(before_b))
                }
                _ => false,
            };
            if same {
                continue;
            }
            rewritten.insert(element);
            if !self.tree.contains(element) {
                continue;
            }
            change.touched.push((element, Touch::Field));
            for applied in [previous, fresh].into_iter().flatten() {
                field_inputs(&applied.journal, element, false, &mut change.inputs);
            }
        }
    }

    /// Apply the instance at `index` to the tree as it stands and write
    /// it at `t`: its application, or `None` when it has nothing to
    /// apply — a glyph-layer body, no element left in the tree — or the
    /// document refuses it, which the instance then remembers.
    fn apply_instance(&mut self, index: usize, t: f32, snap: Snap) -> Option<Applied> {
        let instance = &self.presentations.instances[index];
        let started = Arc::clone(&instance.started);
        let elements = instance.elements.clone();
        let outcome = self.application(&started, &elements);
        let instance = &mut self.presentations.instances[index];
        match outcome {
            Ok(journal) => {
                instance.refusal = None;
                let journal = journal?;
                if t < 1.0 {
                    if let Err(error) = journal.write_interpolated(&mut self.tree, t, snap) {
                        error!(
                            "presentation `{}`: could not be written at {t}: {error}",
                            started.id
                        );
                    }
                }
                Some(Applied { journal, t, snap })
            }
            Err(refusal) => {
                if instance.refusal.as_ref() != Some(&refusal) {
                    warn!(
                        "presentation `{}`: not applied on the map as it stands: {refusal}",
                        started.id
                    );
                }
                instance.refusal = Some(refusal);
                None
            }
        }
    }

    /// Whether `started` applies at `elements` on the tree as it stands:
    /// applied, judged and taken back out.
    fn try_application(&mut self, started: &Started, elements: &[NodeId]) -> Result<(), ApplyError> {
        if let Some(journal) = self.application(started, elements)? {
            self.drop_journal(journal);
        }
        Ok(())
    }

    /// Apply `started` at those of `elements` that are in the tree, as
    /// the writer applies a request's walk — judged before
    /// ([`validate_for_presentation`]: no structural command) and after
    /// ([`validate_applied`], this target's budget) — and return the
    /// journal with the tree as it left it; `None` for a glyph-layer
    /// body, for no element in the tree, and for a walk that changed
    /// nothing. A refusal leaves the tree as it was.
    ///
    /// Nothing is accounted into the document's derived state: the
    /// indexes and the budget describe the map.
    fn application(
        &mut self,
        started: &Started,
        elements: &[NodeId],
    ) -> Result<Option<Journal<DocElement>>, ApplyError> {
        let Body::Document(mutator) = &started.body else {
            return Ok(None);
        };
        let anchors: Vec<NodeId> = elements
            .iter()
            .copied()
            .filter(|&element| self.tree.contains(element))
            .collect();
        if anchors.is_empty() {
            return Ok(None);
        }
        validate_for_presentation(mutator, self)?;
        #[cfg(test)]
        work::count(work::Kind::Walked);
        let mut builder = JournalBuilder::new();
        mutator
            .apply_at_journaled(
                &mut self.tree,
                &Anchors::Nodes(anchors),
                inherited_seed,
                &mut builder,
            )
            .map_err(MutationError::from)?;
        let journal = builder.close(&self.tree);
        let checked = validate_applied(&self.tree, &journal)
            .map_err(ApplyError::from)
            .and_then(|()| self.within_budget(&journal));
        if let Err(error) = checked {
            self.drop_journal(journal);
            return Err(error);
        }
        if journal.is_empty() {
            journal.discard(&mut self.tree);
            return Ok(None);
        }
        Ok(Some(journal))
    }

    /// Drop every instance whose node the arena no longer holds: the
    /// history entry that removed it is gone, so nothing can bring it
    /// back. One whose node is only out of the tree stays, applied
    /// nowhere, and plays again if an undo puts the node back.
    fn drop_instances_of_freed_nodes(&mut self) {
        let Document {
            tree, presentations, ..
        } = self;
        let before = presentations.instances.len();
        presentations
            .instances
            .retain(|instance| tree.get(instance.node).is_some());
        if presentations.instances.len() != before {
            presentations.epoch += 1;
        }
    }
}

/// The whole layer, written back out by the writer's funnel.
pub(in crate::document) struct LayerOut(Lifted);

/// Note every element of `applied` that it shows away from what is under
/// it — all of them, unless it is written at `0` — in `change`.
fn touch_all(tree: &Tree<DocElement>, applied: &Applied, change: &mut Change) {
    if applied.t <= 0.0 {
        return;
    }
    for (element, _) in applied.journal.touched() {
        if tree.contains(element) {
            change.touched.push((element, Touch::Field));
            field_inputs(&applied.journal, element, false, &mut change.inputs);
        }
    }
}

/// A document with its presentation layer set aside
/// ([`Document::suspended`]): reads through it see the map itself, and
/// the layer comes back when it is dropped.
pub struct Suspended<'d> {
    doc: &'d mut Document,
}

impl std::ops::Deref for Suspended<'_> {
    type Target = Document;

    fn deref(&self) -> &Document {
        self.doc
    }
}

impl std::ops::DerefMut for Suspended<'_> {
    fn deref_mut(&mut self) -> &mut Document {
        self.doc
    }
}

impl Drop for Suspended<'_> {
    fn drop(&mut self) {
        self.doc.resume_presentations();
    }
}

/// What the layer has done on this thread, counted: what the tests that
/// hold its work to the instances a change reaches read. Compiled for
/// tests only.
#[cfg(test)]
pub(crate) mod work {
    use std::cell::Cell;

    /// One kind of work.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub(crate) enum Kind {
        /// An application written back out of the tree.
        Lifted,
        /// An application set aside written forward again, as it was or
        /// at another progress.
        Rewritten,
        /// An instance's mutator walked afresh.
        Walked,
    }

    thread_local! {
        static COUNTS: Cell<[usize; 3]> = const { Cell::new([0; 3]) };
    }

    /// One more of `kind`.
    pub(super) fn count(kind: Kind) {
        COUNTS.with(|cell| {
            let mut counts = cell.get();
            counts[kind as usize] += 1;
            cell.set(counts);
        });
    }

    /// How much of each kind this thread has done so far:
    /// `[lifted, rewritten, walked]`.
    pub(crate) fn done() -> [usize; 3] {
        COUNTS.with(Cell::get)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::element::DocKind;
    use crate::document::format::load;
    use crate::document::mutation::ops::{DocMutation, NodeOp};
    use crate::document::tests::format_support::{node, same_document};
    use crate::engine::structure::StructOp;
    use crate::engine::tree::Placement;

    const MAP: &str = r#"{"format":1,"name":"t","nodes":[{"id":1,"position":[0,0],"sections":[{"spans":[{"text":"a"}]}]}]}"#;

    /// No file can spell a structural command in a presentation, and no
    /// definition can hold one: this hands the layer a mutator with one
    /// directly, where every body arrives once its names are resolved.
    /// The document refuses it by name, and nothing of it is applied —
    /// the field write beside the command included.
    ///
    /// Control: judged by `validate` alone, which accepts the command,
    /// the application removes the node's section and the layer would
    /// hold it as a performance.
    #[test]
    fn test_the_layer_refuses_a_structural_mutator() {
        let mut doc = load(MAP).unwrap();
        let one = node(&doc, 1).0;
        let hide = Mutator::new_macro(
            vec![DocMutation::Node(NodeOp::SetHidden(true))],
            BranchKey::new(DocKind::Node, 0),
        );
        // Control: the field write alone applies.
        let fields_only = Started {
            id: "p".to_string(),
            play: Play::Hold,
            body: Body::Document(MutatorTree::new_with(hide.clone())),
        };
        assert_eq!(doc.try_application(&fields_only, &[one]), Ok(()));

        let mut mutator: DocMutatorTree = MutatorTree::new_with(hide);
        let root = mutator.root();
        mutator
            .insert_child(
                root,
                Mutator::new_structure(StructOp::Remove, BranchKey::new(DocKind::Section, 0)),
                Placement::Last,
            )
            .unwrap();
        let structural = Started {
            id: "p".to_string(),
            play: Play::Hold,
            body: Body::Document(mutator),
        };
        let refused = doc.try_application(&structural, &[one]);
        assert!(
            matches!(
                refused,
                Err(ApplyError::Invalid(MutationError::PresentationStructure { .. }))
            ),
            "{refused:?}"
        );
        assert_eq!(
            refused.unwrap_err().to_string(),
            "mutator [0]: a presentation changes fields only, never structure"
        );
        assert!(same_document(&doc, &load(MAP).unwrap()).is_ok());
    }
}
