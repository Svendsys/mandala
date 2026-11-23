// SPDX-License-Identifier: MPL-2.0

//! **NodeEdit's section frames** (V1-C8, #249; design §12.5): while
//! NodeEdit is active on a node of more than one section, each of its
//! sections is outlined in the chrome layer ([`super::outline`]) — thin,
//! and heavy for the focused one, the section being edited or else the
//! one selected — as `main` outlined them: the cue that tells the user
//! what there is to pick.
//!
//! **A pool of slots, grown in place.** The frames' group is planted
//! empty when the view is built; a node with more sections than the pool
//! has slots grows it by structural inserts under the group, journaled
//! like every chrome write (`MapView::sync`), and a slot it no longer
//! needs is hidden, never removed. Each slot is four runs, replaced
//! (`RunSet`) only when what it outlines moves.
//!
//! **Updated only when what it is built from changes**: the active node
//! or the focused section, or a sync pass that may have moved the node's
//! geometry (`Projector::geometry_moved`), which a section's box moves
//! with. A pan or a zoom changes none of them and writes nothing.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::ids::Document;
use baumhard::engine::anchor::Anchors;
use baumhard::engine::structure::{StructOp, Subtree};
use baumhard::engine::tree::Placement;
use baumhard::flow::Rect;
use baumhard::font::fonts::FontSystemLease;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::{GfxMutation, GfxMutator};
use baumhard::gfx_structs::tree::{GfxMutatorTree, GfxTree};
use baumhard::projection::{Projector, RUN_CHANNEL};
use glam::Vec2;

use super::outline::{self, OutlineStyle};
use crate::application::document::selection::kind_of;

/// The channel of the frames' group under the chrome's root: after the
/// caret's ([`super::caret::CARET_GROUP_CHANNEL`]).
pub(crate) const FRAMES_GROUP_CHANNEL: usize = 5;

/// A section's frame: light box-drawing lines, half-transparent.
const THIN: OutlineStyle = OutlineStyle {
    corners: ['\u{250C}', '\u{2510}', '\u{2514}', '\u{2518}'],
    horizontal: '\u{2500}',
    vertical: "\u{2502}",
    alpha: 150,
};

/// The focused section's frame: heavy lines, opaque.
const HEAVY: OutlineStyle = OutlineStyle {
    corners: ['\u{250F}', '\u{2513}', '\u{2517}', '\u{251B}'],
    horizontal: '\u{2501}',
    vertical: "\u{2503}",
    alpha: 255,
};

/// One frame as it is wanted or shown: the section's drawn box, and
/// whether it is the focused one.
pub(crate) type Frame = (Rect, bool);

/// The section frames in the chrome. See the [module docs](self).
pub(crate) struct SectionFrames {
    /// The frames' group under the chrome's root.
    group: NodeId,
    /// Each slot's four runs: top, bottom, left, right.
    slots: Vec<[NodeId; 4]>,
    /// What each slot shows.
    shown: Vec<Option<Frame>>,
    /// The node it outlines the sections of.
    node: Option<NodeId>,
    /// The focused section.
    focus: Option<NodeId>,
}

impl SectionFrames {
    /// Put the frames' group in `chrome`, empty, under the root. Called
    /// once, when the chrome is built.
    pub(crate) fn plant(chrome: &mut GfxTree) -> SectionFrames {
        let group = chrome
            .insert_child(
                chrome.root(),
                GfxElement::new_void(FRAMES_GROUP_CHANNEL),
                Placement::Last,
            )
            .expect("the chrome's root takes a group");
        SectionFrames {
            group,
            slots: Vec::new(),
            shown: Vec::new(),
            node: None,
            focus: None,
        }
    }

    /// The frames `node` wants, focused on `focus`: one per drawn section,
    /// in sibling order, when it has more than one — a node of one section
    /// has nothing to pick, and its frame would only repeat its own, as on
    /// `main`. O(its children).
    pub(crate) fn wanted(
        doc: &Document,
        projector: &Projector,
        node: Option<NodeId>,
        focus: Option<NodeId>,
    ) -> Vec<Frame> {
        let Some(node) = node else {
            return Vec::new();
        };
        let sections: Vec<NodeId> = doc
            .children(node)
            .filter(|&child| kind_of(doc, child) == Some(DocKind::Section))
            .collect();
        if sections.len() < 2 {
            return Vec::new();
        }
        sections
            .into_iter()
            .filter_map(|section| {
                let rect = projector
                    .section_rect(section)
                    .filter(|r| r.size.x > 0.0 && r.size.y > 0.0 && r.size.is_finite())?;
                Some((rect, focus == Some(section)))
            })
            .collect()
    }

    /// Whether a pass must look at the frames again: the node or the focus
    /// changed, or `follow` says the node's geometry may have moved. O(1).
    pub(crate) fn due(&self, node: Option<NodeId>, focus: Option<NodeId>, follow: bool) -> bool {
        node != self.node || focus != self.focus || (follow && node.is_some())
    }

    /// The structural inserts that give the pool `count` slots, each four
    /// hidden runs under the group; nothing when it has them. Applied, the
    /// new runs are read back with [`Self::adopt`].
    pub(crate) fn grow(&self, count: usize) -> Vec<(GfxMutatorTree, Anchors)> {
        let missing = count.saturating_sub(self.slots.len());
        if missing == 0 {
            return Vec::new();
        }
        let mut fonts = FontSystemLease::new("view::frames::grow");
        (0..missing * 4)
            .map(|_| {
                let run = outline::run_of("", Vec2::ZERO, 0.0, THIN.alpha, &mut fonts);
                let insert = StructOp::Insert {
                    subtree: Subtree::leaf(GfxElement::new_run(run, RUN_CHANNEL, 0)),
                    at: Placement::Last,
                };
                (
                    GfxMutatorTree::new_with(GfxMutator::new_structure(insert, RUN_CHANNEL)),
                    Anchors::Nodes(vec![self.group]),
                )
            })
            .collect()
    }

    /// Take the runs [`Self::grow`]'s inserts put under the group as the
    /// pool's new slots, four by four, in order. O(the group's runs).
    pub(crate) fn adopt(&mut self, chrome: &GfxTree) {
        let runs: Vec<NodeId> = self.group.children(chrome.arena()).collect();
        for &chunk in runs.as_chunks::<4>().0.iter().skip(self.slots.len()) {
            self.slots.push(chunk);
            self.shown.push(None);
        }
    }

    /// The chrome mutators that show `want` — slot by slot, replacing the
    /// four runs of every slot whose frame changed and hiding the slots no
    /// frame needs — and take `node` and `focus` as what it shows. The
    /// pool must hold a slot per frame ([`Self::grow`]); a frame past it is
    /// left out.
    ///
    /// Cost: four box shapings per slot that changed.
    pub(crate) fn update(
        &mut self,
        node: Option<NodeId>,
        focus: Option<NodeId>,
        want: &[Frame],
    ) -> Vec<(GfxMutatorTree, Anchors)> {
        self.node = node;
        self.focus = focus;
        let mut fonts = FontSystemLease::new("view::frames");
        let mut out = Vec::new();
        for (slot, runs) in self.slots.iter().enumerate() {
            let next = want.get(slot).copied();
            if self.shown[slot] == next {
                continue;
            }
            self.shown[slot] = next;
            let style = match next {
                Some((_, true)) => &HEAVY,
                _ => &THIN,
            };
            let built = outline::runs_for(next.map(|(rect, _)| rect), style, &mut fonts);
            for (run, &id) in built.into_iter().zip(runs) {
                out.push((
                    GfxMutatorTree::new_with(GfxMutator::new(
                        GfxMutation::RunSet(Box::new(run)),
                        RUN_CHANNEL,
                    )),
                    Anchors::Nodes(vec![id]),
                ));
            }
        }
        out
    }

    /// How many slots the pool holds. Test builds only.
    #[cfg(test)]
    pub(crate) fn slots(&self) -> usize {
        self.slots.len()
    }

    /// What each slot shows. Test builds only.
    #[cfg(test)]
    pub(crate) fn shown(&self) -> &[Option<Frame>] {
        &self.shown
    }
}
