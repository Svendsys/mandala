// SPDX-License-Identifier: MPL-2.0

//! **Map verbs, their triggers and links** (design §11; V1-D3, #252):
//! the app's half of `baumhard::document::verb`, which reads verbs and
//! runs them over a [`Performer`]. This is the performer — the open map,
//! where it is drawn, and the edits' funnel — and every route a verb is
//! invoked by:
//!
//! - **a click** on a node whose `on.click` names a verb runs it with
//!   `self` the node, before the click's selection, which is as it
//!   always is (`main`'s `OnClick` triggers fired first and selected
//!   all the same). A click on a **linked span** runs its verb with
//!   `self` the node the span belongs to, or opens its address
//!   (`platform::opener`); a span's link is the more specific target,
//!   so a click on one does not also run its node's `on.click`. A tap
//!   is a click;
//! - **a hover**: the pointer entering a node whose `on.hover` names a
//!   verb invokes it **held**, and leaving releases the hold — the steps
//!   it has not run are skipped and every `while` instance it started
//!   plays its way out. Moving from one node to another releases the
//!   first before invoking the second. Touch has no hover;
//! - **the console** (`verb run`), **IPC** (`act.verb`) and **macros**
//!   (a `verb` step), by id, on a node or none.
//!
//! **A verb never settles an animated edit to start a presentation.**
//! The console's `present start` settles one first, as every console
//! verb that reads the map does (D28); a hover runs at the pace of the
//! pointer and must not cut short the edit the user is watching, so
//! nothing here does. A verb's `edit` step does settle one — it is an
//! apply, and every apply does (D28) — and while an interactive
//! transaction is open it is skipped with a warning.
//!
//! The runs are the open map's ([`MindMapDocument::take_runs`]): a map
//! that is replaced takes its runs with it. The frame resumes the ones a
//! `wait` suspended (`Host::tick`, from `app::animation::tick`).

use baumhard::arena::NodeId;
use baumhard::document::element::DocElement;
use baumhard::document::ids::{Document, NodeIdent};
use baumhard::document::presentation::{InstanceId, PresentError};
use baumhard::document::values::{EditId, LinkTarget, PresentationId, VerbId};
use baumhard::document::verb::{Performer, RunId, VerbError};
use baumhard::document::writer::ChangeSource;
use baumhard::projection::Hit;

use super::edits::{run_edit, Drawn};
use crate::application::document::selection::selectable;
use crate::application::document::{Hover, MindMapDocument};
use crate::application::edits::EditRegistry;

/// What a verb runs against: the open map, where it is drawn (an edit
/// handler reads it), and the edits' funnel.
pub(in crate::application) struct Host<'h> {
    /// The open map.
    pub(in crate::application) doc: &'h mut MindMapDocument,
    /// Where it is drawn.
    pub(in crate::application) drawn: &'h mut dyn Drawn,
    /// The edits' tiers and handlers.
    pub(in crate::application) edits: &'h EditRegistry,
}

impl Performer for Host<'_> {
    fn document(&self) -> &Document {
        self.doc.doc()
    }

    fn selection(&self) -> Vec<NodeId> {
        self.doc.selection.iter().collect()
    }

    fn start(
        &mut self,
        id: PresentationId,
        nodes: &[NodeId],
        at_ms: f64,
    ) -> Result<Vec<InstanceId>, PresentError> {
        // A run asks only for a presentation the map defines.
        self.doc
            .start_defined(id, nodes, at_ms)
            .unwrap_or_else(|| Ok(Vec::new()))
    }

    fn stop(&mut self, id: PresentationId, nodes: &[NodeId], at_ms: f64) -> usize {
        self.doc.stop_defined(id, nodes, at_ms)
    }

    fn stop_instances(&mut self, ids: &[InstanceId], at_ms: f64) -> usize {
        self.doc.stop_instances(ids, at_ms)
    }

    fn edit(&mut self, id: EditId, nodes: &[NodeId], at_ms: f64) -> Result<(), String> {
        // The edit funnel names an edit by its id, as every route does:
        // the id, not the definition, is what is copied.
        let Some(id) = self
            .doc
            .doc()
            .definitions()
            .edits
            .get(usize::from(id.0))
            .map(|def| def.id.clone())
        else {
            return Err(format!(
                "no edit at {id:?}: a run asks only for an edit the map defines"
            ));
        };
        let id = id.as_str();
        // D28: an interactive transaction refuses every other change, and
        // a verb's edit step yields to it rather than fail the run.
        if let Some(interaction) = self.doc.open_interaction() {
            return Err(format!(
                "edit `{id}` skipped: {interaction:?} is open (D28), and a verb's edit waits for nobody"
            ));
        }
        if nodes.is_empty() {
            return Ok(());
        }
        run_edit(
            self.doc,
            self.drawn,
            self.edits,
            id,
            Some(nodes),
            ChangeSource::Verb,
            at_ms,
        )
    }
}

impl Host<'_> {
    /// Run `f` with the map's runs, taken out of it while the steps they
    /// run reach the map, and put back after.
    fn with_runs<R>(&mut self, f: impl FnOnce(&mut baumhard::document::verb::Runs, &mut Self) -> R) -> R {
        let mut runs = self.doc.take_runs();
        let out = f(&mut runs, self);
        self.doc.put_runs(runs);
        out
    }

    /// Invoke the verb `id` on `on` — an element stands for its node,
    /// `None` for none — at `now_ms`: its steps run up to its first
    /// `wait`. `held` keeps the run until it is released (a hover's).
    ///
    /// Errors, with nothing run, worded for the caller to show: no verb
    /// has the id; the verb acts on the node it is run on and there is
    /// none.
    pub(in crate::application) fn invoke(
        &mut self,
        id: &str,
        on: Option<NodeId>,
        held: bool,
        now_ms: f64,
    ) -> Result<RunId, String> {
        let verb = self
            .doc
            .doc()
            .definitions()
            .verb_by_id(id)
            .map(|(verb, _)| verb)
            .ok_or_else(|| VerbError::Unknown { verb: id.to_string() }.to_string())?;
        self.invoke_id(verb, on, held, now_ms)
            .map_err(|error| error.to_string())
    }

    /// [`Self::invoke`] for a verb the map names by index — a trigger's
    /// or a link's.
    fn invoke_id(
        &mut self,
        verb: VerbId,
        on: Option<NodeId>,
        held: bool,
        now_ms: f64,
    ) -> Result<RunId, VerbError> {
        self.with_runs(|runs, host| runs.invoke(verb, on, held, now_ms, host))
    }

    /// Resume every run a `wait` suspended whose time has come at
    /// `now_ms`. Returns whether a step ran.
    pub(in crate::application) fn tick(&mut self, now_ms: f64) -> bool {
        self.with_runs(|runs, host| runs.tick(now_ms, host))
    }

    /// The pointer rests on `node` — a node whose `on.hover` names a
    /// verb, or `None` for anything else — at `now_ms`. Moving off the
    /// hovered node releases its hold; moving onto another invokes that
    /// one's verb held, after the release. Nothing for a pointer that
    /// stays where it was. Returns whether the hover changed.
    pub(in crate::application) fn hover(&mut self, node: Option<NodeId>, now_ms: f64) -> bool {
        let held = self.doc.hover();
        if held.map(|hover| hover.node) == node {
            return false;
        }
        if let Some(Hover { run: Some(run), .. }) = held {
            self.with_runs(|runs, host| runs.release(run, now_ms, host));
        }
        let entered = node.and_then(|node| Some((node, hover_verb(self.doc.doc(), node)?)));
        let hover = entered.map(|(node, verb)| Hover {
            node,
            run: match self.invoke_id(verb, Some(node), true, now_ms) {
                Ok(run) => Some(run),
                Err(error) => {
                    log::warn!("hover: {error}");
                    None
                }
            },
        });
        self.doc.set_hover(hover);
        true
    }

    /// A click (or a tap) landed on `hit` at `now_ms`: run what it
    /// triggers ([`click_of`]). Returns whether anything ran — a verb
    /// invoked, an address opened.
    pub(in crate::application) fn click(&mut self, hit: Option<&Hit>, now_ms: f64) -> bool {
        let Some(trigger) = hit.and_then(|hit| click_of(self.doc.doc(), hit)) else {
            return false;
        };
        match trigger {
            Click::Verb(verb, node) => match self.invoke_id(verb, Some(node), false, now_ms) {
                Ok(_) => true,
                Err(error) => {
                    log::warn!("click: {error}");
                    false
                }
            },
            Click::Url(url) => match crate::application::platform::opener::open_url(&url) {
                Ok(()) => true,
                Err(error) => {
                    log::warn!("link: {error}");
                    false
                }
            },
        }
    }
}

/// What a click on `hit` triggers: a linked span's verb (with `self` the
/// node the span belongs to) or address; else the clicked node's
/// `on.click`. A connector or a link clicked is not its node, as for the
/// selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::application) enum Click {
    /// Run the verb with `self` the node.
    Verb(VerbId, NodeId),
    /// Open the address.
    Url(String),
}

/// What a click on `hit` triggers on `doc`, if anything. O(the element's
/// depth within its node).
pub(in crate::application) fn click_of(doc: &Document, hit: &Hit) -> Option<Click> {
    if let Some(span) = hit.span {
        if let Some(DocElement::Span(data)) = doc.get(span) {
            match &data.link {
                Some(LinkTarget::Url(url)) => return Some(Click::Url(url.clone())),
                Some(LinkTarget::Verb(verb)) => {
                    if let Some(node) = doc.owning_node(span) {
                        return Some(Click::Verb(*verb, node));
                    }
                }
                None => {}
            }
        }
    }
    let node = trigger_node(doc, hit)?;
    let verb = doc.get(node)?.as_node()?.on.click?;
    Some(Click::Verb(verb, node))
}

/// The node `hit` is on for a trigger: the node the selection would take
/// for it — never a connector or a link, which are not their node. O(the
/// element's depth within its node).
fn trigger_node(doc: &Document, hit: &Hit) -> Option<NodeId> {
    let element = selectable(doc, hit);
    doc.get(element)?.as_node().map(|_| element)
}

/// The verb `node`'s `on.hover` names.
fn hover_verb(doc: &Document, node: NodeId) -> Option<VerbId> {
    doc.get(node)?.as_node()?.on.hover
}

/// The node the pointer on `hit` hovers for a trigger: one whose
/// `on.hover` names a verb, else `None`.
pub(in crate::application) fn hovered(doc: &Document, hit: Option<&Hit>) -> Option<NodeId> {
    let node = trigger_node(doc, hit?)?;
    hover_verb(doc, node).map(|_| node)
}

/// Whether a click on `hit` would trigger something: what the pointer
/// cursor shows over (#252 §3).
pub(in crate::application) fn points_at(doc: &Document, hit: Option<&Hit>) -> bool {
    hit.is_some_and(|hit| click_of(doc, hit).is_some())
}

/// Run a macro's `Verb` step: the map verb `id`, on the node `target`
/// resolves — the one selected element's node, none with nothing
/// selected, or the node with the ident it names. A selection of more
/// than one element, and an ident the map lacks, skip the step with a
/// warning: a verb runs on one node. Returns whether the verb ran; a
/// refusal is logged and does not count.
pub(in crate::application::app) fn run_step(
    doc: &mut MindMapDocument,
    drawn: &mut dyn Drawn,
    edits: &EditRegistry,
    id: &str,
    target: &crate::application::macros::MacroTarget,
    now_ms: f64,
) -> bool {
    use crate::application::macros::MacroTarget;
    let on = match target {
        MacroTarget::CurrentSelection => {
            let mut selected = doc.selection.iter();
            match (selected.next(), selected.next()) {
                (one, None) => one,
                (_, Some(_)) => {
                    log::warn!(
                        "macros: Verb step `{id}`: a verb runs on one node, and more are selected; skipping"
                    );
                    return false;
                }
            }
        }
        MacroTarget::NodeId(ident) => match doc.doc().node_by_ident(NodeIdent(*ident)) {
            Some(node) => Some(node),
            None => {
                log::warn!("macros: Verb step `{id}`: no node has the id {ident}; skipping");
                return false;
            }
        },
    };
    match (Host { doc, drawn, edits }).invoke(id, on, false, now_ms) {
        Ok(_) => true,
        Err(message) => {
            log::warn!("macros: Verb step: {message}");
            false
        }
    }
}

/// Run the map verb `id` on the node with the ident `node`, or on none,
/// at `now_ms`: IPC's `act.verb`, on both targets. A refusal is worded
/// for the reply.
pub(in crate::application::app) fn run_named(
    doc: &mut MindMapDocument,
    drawn: &mut dyn Drawn,
    edits: &EditRegistry,
    id: &str,
    node: Option<u32>,
    now_ms: f64,
) -> Result<(), String> {
    let on = node.map(|ident| node_named(doc.doc(), ident)).transpose()?;
    (Host { doc, drawn, edits })
        .invoke(id, on, false, now_ms)
        .map(drop)
}

/// The node `ident` names, for a caller outside the map (the console,
/// IPC), or a refusal worded for it.
pub(in crate::application) fn node_named(doc: &Document, ident: u32) -> Result<NodeId, String> {
    doc.node_by_ident(NodeIdent(ident))
        .ok_or_else(|| format!("no node has the id {ident}"))
}
