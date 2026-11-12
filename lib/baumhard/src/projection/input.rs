// SPDX-License-Identifier: MPL-2.0

//! The dependency index (design §12.3): which projection work read
//! which input, captured as it read it — never listed by hand.

use crate::arena::NodeId;
use crate::document::resolve::{Input as DocInput, ReadRecorder};
use rustc_hash::{FxHashMap, FxHashSet};

/// Something a piece of projection work read: one of the document's
/// resolution inputs ([`DocInput`], what the resolvers report), or a
/// fact the projection derived about another element and read through
/// its own state.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ProjectionInput {
    /// A resolution input of the document.
    Doc(DocInput),
    /// Where an element is drawn, as projection state holds it: a node's
    /// drawn rectangle and outline, a section's box within its node, a
    /// ring's placed line height (what the rings outside it clear), a
    /// path's curve, a portal end's marker.
    ///
    /// A ring's is its extent, what the ring just outside it clears.
    Geometry(NodeId),
    /// Whether a node is drawn, and the zoom window everything it draws
    /// renders in.
    Visibility(NodeId),
    /// A node's effective opacity: its own times every ancestor's.
    Opacity(NodeId),
    /// A node's drawn size, what its rings' relative sizes are taken of
    /// (#241 amendment 1). Marked with its [`Self::Geometry`] when the
    /// size changed, and not when the node only moved: a drag reshapes
    /// nothing.
    Size(NodeId),
    /// Which node a node hangs from, as projection state holds it: what
    /// its connector is drawn from. Marked when the projector re-hangs a
    /// kept node (`reparent`); a node it creates is read only by its own
    /// connector, created and resolved with it. So a connector reads its
    /// own node's parent and never the parent's whole children row
    /// (ruling B7.5 on #241, D15).
    Parent(NodeId),
    /// Which ring is just inside a ring in its frame, as the projection
    /// holds its rings: the one ring whose extent its placement clears.
    /// Marked for the ring a ring now lies just inside of when one comes
    /// into a frame or moves within it; a ring that leaves or moves marks
    /// its own [`Self::Geometry`], which the ring that lay outside it
    /// read, and a moved ring is placed by its own touch. So a ring reads the one ring inside it,
    /// never its frame's whole row, and an edit of one ring re-places the
    /// rings outside it alone (the review of #317).
    Inner(NodeId),
    /// The box a node's paths attach to (#296): the box round its drawn
    /// rings' inks, or its drawn rectangle where it draws none. Marked when
    /// a ring's ink box changed, or a ring left; a node with no ring marks
    /// its [`Self::Geometry`] instead, which the paths read too.
    Reach(NodeId),
    /// The region a node hides paths' glyphs by (#332): the box round its
    /// sections' text and its rings' inks, and its outline. Marked when it
    /// changed, once every ring is placed.
    Region(NodeId),
}

/// The stage of a unit's projection a dependency belongs to: what has to
/// run again when the input changes. A node has two stages, a flow
/// three; a drag, which changes geometry only, reaches only the
/// `Geometry` and `Place` stages, and nothing is resolved again (#241,
/// the B1 granularity note).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    /// A node's inherited facts: its palette binding, effective opacity,
    /// zoom window and depth.
    Carry,
    /// A node's drawn rectangle and outline.
    Geometry,
    /// A flow's resolution: its styles, its text, its shape key.
    Resolve,
    /// A flow's shaping, where its shape key reads geometry (a ring's
    /// relative size).
    Shape,
    /// A flow's placement.
    Place,
    /// Which of a connector's or line link's placed glyphs the nodes it
    /// does not end at hide (#332): a view decision over its placement,
    /// run after every path is laid.
    Hide,
}

/// One piece of projection work: a stage of one unit's projection.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Dependent {
    /// The document element whose projection it is: a node, a section,
    /// a ring, a connector or a link.
    pub unit: NodeId,
    /// The stage.
    pub stage: Stage,
}

impl Dependent {
    /// `stage` of `unit`. O(1).
    pub const fn new(unit: NodeId, stage: Stage) -> Self {
        Dependent { unit, stage }
    }
}

/// The dependency index: for each input, the work that read it
/// (forward), and for each piece of work, what it read (reverse), so its
/// old reads are cleared when it runs again.
///
/// Each forward list holds a piece of work at most once, and each reverse
/// entry names its place in that list, so clearing one piece of work is
/// O(what it read) however many others read the same input — the theme,
/// the defaults, which most of a map reads.
#[derive(Debug, Default)]
pub(crate) struct DepIndex {
    forward: FxHashMap<ProjectionInput, Vec<Dependent>>,
    reverse: FxHashMap<Dependent, Vec<(ProjectionInput, u32)>>,
    /// Scratch for [`Self::record`]'s dedup, kept to reuse its storage.
    seen: FxHashSet<ProjectionInput>,
}

impl DepIndex {
    /// Forget everything `dep` read. O(what it read).
    pub(crate) fn clear(&mut self, dep: Dependent) {
        let Some(reads) = self.reverse.remove(&dep) else {
            return;
        };
        for (input, at) in reads {
            let Some(list) = self.forward.get_mut(&input) else {
                continue;
            };
            let at = at as usize;
            list.swap_remove(at);
            if let Some(&moved) = list.get(at) {
                // The last entry took the hole: point its reverse entry at
                // its new place.
                if let Some(entries) = self.reverse.get_mut(&moved) {
                    if let Some(entry) = entries.iter_mut().find(|(i, _)| *i == input) {
                        entry.1 = at as u32;
                    }
                }
            }
            if list.is_empty() {
                self.forward.remove(&input);
            }
        }
    }

    /// Replace what `dep` read with `reads`, each once however often it
    /// was read. O(reads).
    pub(crate) fn record(&mut self, dep: Dependent, reads: &[ProjectionInput]) {
        self.clear(dep);
        if reads.is_empty() {
            return;
        }
        self.seen.clear();
        let mut entries: Vec<(ProjectionInput, u32)> = Vec::with_capacity(reads.len());
        for &input in reads {
            if !self.seen.insert(input) {
                continue;
            }
            let list = self.forward.entry(input).or_default();
            entries.push((input, list.len() as u32));
            list.push(dep);
        }
        self.reverse.insert(dep, entries);
    }

    /// The work that read `input`. O(1).
    pub(crate) fn dependents(&self, input: &ProjectionInput) -> &[Dependent] {
        self.forward.get(input).map_or(&[], Vec::as_slice)
    }

    /// How many (input, work) pairs the index holds. O(inputs). Test
    /// builds only.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.forward.values().map(Vec::len).sum()
    }
}

/// A [`ReadRecorder`] that collects what a resolution read, as projection
/// inputs, into a buffer the caller then records against its work.
pub(crate) struct Reads<'a>(pub(crate) &'a mut Vec<ProjectionInput>);

impl ReadRecorder for Reads<'_> {
    fn read(&mut self, input: DocInput) {
        self.0.push(ProjectionInput::Doc(input));
    }
}
