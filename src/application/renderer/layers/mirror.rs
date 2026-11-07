// SPDX-License-Identifier: MPL-2.0

//! One glyph layer's render state, shaped as the layer is: a slot per
//! element, linked to its parent, its first and last child and its two
//! siblings, so the draw order — the layer's pre-order — is a walk of
//! the links, and an element entering, leaving or moving is a splice of
//! a few links rather than a rebuild of any order.
//!
//! Slots live in one `Vec` and refer to one another by index; an element
//! is found by its glyph [`NodeId`] through one map, which only
//! [`LayerRenderer::apply`](super::LayerRenderer::apply) reads. The walk
//! a frame draws follows indices alone.

use baumhard::arena::NodeId;
use baumhard::gfx_structs::run::GlyphRunData;
use baumhard::gfx_structs::tree::GfxTree;
use rustc_hash::FxHashMap;

/// A slot's index in [`Mirror::slots`].
pub(super) type SlotId = u32;

/// What the renderer keeps for one `GlyphRun` element: the run as the
/// layer holds it — its shaping and placement by `Arc`, so keeping it
/// copies pointers and never shapes — and the stamp of the last change
/// to it.
#[derive(Debug)]
pub(crate) struct RunState {
    /// The run, as the element held it when it was last applied.
    pub(super) run: GlyphRunData,
    /// The [`LayerRenderer`](super::LayerRenderer)-wide count at which
    /// this state was last written: equal across two reads exactly when
    /// nothing wrote it in between.
    pub(super) stamp: u64,
}

impl RunState {
    /// The run this state draws.
    pub(crate) fn run(&self) -> &GlyphRunData {
        &self.run
    }

    /// When this state was last written ([`RunState::stamp`]).
    #[cfg(test)]
    pub(crate) fn stamp(&self) -> u64 {
        self.stamp
    }
}

/// One element of the layer: its place among the others and, for a run,
/// what it draws. A group (`Void`) has no state: it orders its children
/// and draws nothing.
#[derive(Debug, Default)]
pub(super) struct Slot {
    /// The element, or `None` for a free slot.
    pub(super) id: Option<NodeId>,
    pub(super) parent: Option<SlotId>,
    pub(super) first: Option<SlotId>,
    pub(super) last: Option<SlotId>,
    pub(super) prev: Option<SlotId>,
    pub(super) next: Option<SlotId>,
    /// What a run draws; `None` for a group.
    pub(super) state: Option<RunState>,
}

/// One layer's slots, its root and the index from glyph element to slot.
#[derive(Debug, Default)]
pub(super) struct Mirror {
    slots: Vec<Slot>,
    /// Slots freed by removals, reused before the `Vec` grows.
    free: Vec<SlotId>,
    index: FxHashMap<NodeId, SlotId>,
    root: Option<SlotId>,
}

impl Mirror {
    /// The mirror of `tree`, with `state` making each element's: one
    /// pre-order walk, each element appended as its parent's last child.
    /// The one full walk of a layer the renderer makes, at load.
    ///
    /// Cost: O(the layer's elements); one slot and one index entry each.
    pub(super) fn of(tree: &GfxTree, mut state: impl FnMut(NodeId) -> Option<RunState>) -> Mirror {
        let arena = tree.arena();
        let mut mirror = Mirror::default();
        let count = tree.root().descendants(arena).count();
        mirror.slots.reserve(count);
        mirror.index.reserve(count);
        for id in tree.root().descendants(arena) {
            let slot = mirror.alloc(id, state(id));
            match id.parent(arena).and_then(|parent| mirror.slot_of(parent)) {
                Some(parent) => {
                    let after = mirror.slots[parent as usize].last;
                    mirror.link(slot, parent, after);
                }
                None => mirror.root = Some(slot),
            }
        }
        mirror
    }

    /// The slot of `id`, if the mirror holds it. O(1).
    pub(super) fn slot_of(&self, id: NodeId) -> Option<SlotId> {
        self.index.get(&id).copied()
    }

    /// The slot at `slot`.
    pub(super) fn slot(&self, slot: SlotId) -> &Slot {
        &self.slots[slot as usize]
    }

    /// The slot at `slot`, to change its state.
    pub(super) fn slot_mut(&mut self, slot: SlotId) -> &mut Slot {
        &mut self.slots[slot as usize]
    }

    /// Whether `slot` hangs in the layer: the root, or under a parent.
    pub(super) fn is_attached(&self, slot: SlotId) -> bool {
        Some(slot) == self.root || self.slots[slot as usize].parent.is_some()
    }

    /// How many elements the mirror holds. O(1).
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.index.len()
    }

    /// A slot for `id`, holding `state`, under no parent yet. O(1),
    /// amortized: a freed slot is reused before the `Vec` grows.
    pub(super) fn alloc(&mut self, id: NodeId, state: Option<RunState>) -> SlotId {
        let fresh = Slot {
            id: Some(id),
            state,
            ..Slot::default()
        };
        let slot = match self.free.pop() {
            Some(slot) => {
                self.slots[slot as usize] = fresh;
                slot
            }
            None => {
                self.slots.push(fresh);
                (self.slots.len() - 1) as SlotId
            }
        };
        self.index.insert(id, slot);
        slot
    }

    /// Free `top` and every slot under it — the elements a removal took
    /// out of the layer — except the slots under it whose element `keep`
    /// says is still in the layer: one moved out of the subtree before
    /// the removal, which the slots still hold under `top`. A kept slot
    /// is unlinked, its own subtree left under it, and hangs under no
    /// parent until it is spliced into its row; nothing under it is
    /// looked at. Each freed slot is unlinked from nothing — what is
    /// freed goes whole — and `top` is first unlinked from its parent, so
    /// the rows that stay are spliced shut. Returns how many slots were
    /// freed.
    ///
    /// Cost: O(the slots freed, and the kept slots' siblings); `stack` is
    /// the caller's scratch, left empty.
    pub(super) fn free_subtree(
        &mut self,
        top: SlotId,
        stack: &mut Vec<SlotId>,
        keep: impl Fn(NodeId) -> bool,
    ) -> usize {
        self.unlink(top);
        let mut freed = 0;
        stack.clear();
        stack.push(top);
        while let Some(slot) = stack.pop() {
            let mut child = self.slots[slot as usize].first;
            while let Some(c) = child {
                child = self.slots[c as usize].next;
                if self.slots[c as usize].id.is_some_and(&keep) {
                    self.unlink(c);
                } else {
                    stack.push(c);
                }
            }
            if let Some(id) = self.slots[slot as usize].id.take() {
                self.index.remove(&id);
            }
            self.slots[slot as usize] = Slot::default();
            self.free.push(slot);
            freed += 1;
        }
        if self.root == Some(top) {
            self.root = None;
        }
        freed
    }

    /// Take `slot` out of its parent's row, splicing its siblings
    /// together; its own subtree stays under it. Nothing for a slot under
    /// no parent. O(1).
    pub(super) fn unlink(&mut self, slot: SlotId) {
        let (parent, prev, next) = {
            let s = &self.slots[slot as usize];
            (s.parent, s.prev, s.next)
        };
        let Some(parent) = parent else {
            return;
        };
        match prev {
            Some(p) => self.slots[p as usize].next = next,
            None => self.slots[parent as usize].first = next,
        }
        match next {
            Some(n) => self.slots[n as usize].prev = prev,
            None => self.slots[parent as usize].last = prev,
        }
        let s = &mut self.slots[slot as usize];
        s.parent = None;
        s.prev = None;
        s.next = None;
    }

    /// Put `slot`, which hangs under no parent, into `parent`'s row
    /// right after `after` — at the front when `after` is `None`. O(1).
    pub(super) fn link(&mut self, slot: SlotId, parent: SlotId, after: Option<SlotId>) {
        let next = match after {
            Some(a) => self.slots[a as usize].next,
            None => self.slots[parent as usize].first,
        };
        match after {
            Some(a) => self.slots[a as usize].next = Some(slot),
            None => self.slots[parent as usize].first = Some(slot),
        }
        match next {
            Some(n) => self.slots[n as usize].prev = Some(slot),
            None => self.slots[parent as usize].last = Some(slot),
        }
        let s = &mut self.slots[slot as usize];
        s.parent = Some(parent);
        s.prev = after;
        s.next = next;
    }

    /// Every run's state, in the layer's pre-order: the order its runs
    /// are drawn in, each over the ones before it.
    ///
    /// Cost: O(the layer's elements), following links; no allocation and
    /// no index lookup.
    pub(super) fn runs(&self) -> Runs<'_> {
        Runs {
            mirror: self,
            at: self.root,
        }
    }

    /// Every element's glyph id, in the layer's pre-order, from the root.
    /// For the tests that hold the mirror to its layer.
    #[cfg(test)]
    pub(super) fn ids(&self) -> Vec<NodeId> {
        let mut out = Vec::with_capacity(self.len());
        let mut at = self.root;
        while let Some(slot) = at {
            out.extend(self.slots[slot as usize].id);
            at = self.after(slot);
        }
        out
    }

    /// The slot after `slot` in pre-order: its first child, else the
    /// next sibling of it or of its nearest ancestor that has one.
    fn after(&self, slot: SlotId) -> Option<SlotId> {
        if let Some(first) = self.slots[slot as usize].first {
            return Some(first);
        }
        let mut at = slot;
        loop {
            let s = &self.slots[at as usize];
            if let Some(next) = s.next {
                return Some(next);
            }
            at = s.parent?;
        }
    }
}

/// [`Mirror::runs`].
pub(super) struct Runs<'a> {
    mirror: &'a Mirror,
    at: Option<SlotId>,
}

impl<'a> Iterator for Runs<'a> {
    type Item = &'a RunState;

    fn next(&mut self) -> Option<&'a RunState> {
        while let Some(slot) = self.at {
            self.at = self.mirror.after(slot);
            if let Some(state) = &self.mirror.slots[slot as usize].state {
                return Some(state);
            }
        }
        None
    }
}
