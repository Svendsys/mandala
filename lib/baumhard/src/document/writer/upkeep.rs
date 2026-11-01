// SPDX-License-Identifier: MPL-2.0

//! What a document derives from its tree and keeps beside it — the
//! ident index, the [incoming-link index](crate::document::writer::upkeep::IncomingIndex)
//! and the [budget](crate::document::writer::budget::Budget) — and the
//! one function that keeps all three in step with every journal the
//! writer writes ([`Document::account`]).
//!
//! **Built once, then moved by journals.** The loader builds each as it
//! places the elements it reads (the ident index node by node, the
//! incoming links as it resolves each link's `to`, the budget as it
//! appends); nothing scans the tree for them afterwards. Each journal
//! the writer writes — an apply, a stage, a commit's fast-forward, an
//! undo, a redo, a cancel — moves them by exactly the elements it puts
//! into the tree, takes out of it, or changes: per touched element, it
//! takes the element's old state out of the indexes and puts its new one
//! in. A journal names every element of a subtree that enters or leaves,
//! so a nested link or node is never missed, and the cost is the
//! journal's.

use crate::document::element::DocElement;
use crate::document::ids::{Definitions, Document, IdentIndex, NodeIdent};
use crate::document::writer::budget::Budget;
use crate::engine::journal::{Journal, Touch};
use crate::engine::tree::Tree;
use indextree::NodeId;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

/// The links that lead to each node: node → the links whose `to` names
/// it, in no particular order. A node no link leads to has no entry.
pub(crate) type IncomingIndex = FxHashMap<NodeId, SmallVec<[NodeId; 2]>>;

/// What a document derives from its tree beside the ident index: the
/// links into each node and the document's budget.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Derived {
    /// The links into each node.
    pub(in crate::document) incoming: IncomingIndex,
    /// The elements and the bytes of text the document holds.
    pub(in crate::document) budget: Budget,
}

impl Derived {
    /// Derive both from `tree` and `definitions` by one walk of the tree:
    /// for a document built without the loader — the test builder, and
    /// the tests that tamper with a loaded tree — never for an edit.
    ///
    /// Cost: O(elements), one hash insert per link.
    pub(crate) fn scan(tree: &Tree<DocElement>, definitions: &Definitions) -> Derived {
        let mut derived = Derived::default();
        for id in tree.descendants() {
            let element = tree.arena()[id].get();
            derived.budget.enter(element);
            if let DocElement::Link(link) = element {
                derived.link_in(link.to, id);
            }
        }
        derived.definitions(definitions);
        derived
    }

    /// Count `definitions` into the budget, as the loader counts each one
    /// it reads. O(1).
    pub(crate) fn definitions(&mut self, definitions: &Definitions) {
        let Definitions {
            edits,
            presentations,
            verbs,
        } = definitions;
        let count = edits.len() + presentations.len() + verbs.len();
        self.budget.elements = self
            .budget
            .elements
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
    }

    /// Count `element`, which a load has just appended, into the budget.
    /// O(its strings).
    pub(crate) fn appended(&mut self, element: &DocElement) {
        self.budget.enter(element);
    }

    /// Note that `link` leads to `to`. O(1).
    pub(crate) fn link_in(&mut self, to: NodeId, link: NodeId) {
        let links = self.incoming.entry(to).or_default();
        if !links.contains(&link) {
            links.push(link);
        }
    }

    /// Note that `link` no longer leads to `to`. O(the links into `to`).
    fn link_out(&mut self, to: NodeId, link: NodeId) {
        if let Some(links) = self.incoming.get_mut(&to) {
            links.retain(|l| *l != link);
            if links.is_empty() {
                self.incoming.remove(&to);
            }
        }
    }
}

/// Which way a journal has just been written.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    /// Forward: closed from its request, or replayed.
    Forward,
    /// Backward: reverted.
    Backward,
}

impl Document {
    /// Move the ident index, the incoming links and the budget by what
    /// `journal`, just written in `direction`, changed: every touched
    /// element leaves them in the state it had, and enters them in the
    /// state it has. Call it once per write, after the write and before
    /// the journal is discarded, with the tree in the arrangement the
    /// write left.
    ///
    /// Cost: O(the journal's touches), each O(1) but a link's, which is
    /// O(the links into its target).
    pub(super) fn account(&mut self, journal: &Journal<DocElement>, direction: Direction) {
        let Document {
            tree,
            ident_index,
            derived,
            ..
        } = self;
        for (id, touch) in journal.touched() {
            let Some(current) = tree.arena().get(id).map(|node| node.get()) else {
                continue;
            };
            let before = journal.before(id).unwrap_or(current);
            let after = journal.after(id).unwrap_or(current);
            let (in_before, in_after) = match touch {
                Touch::Inserted => (false, true),
                Touch::Removed => (true, false),
                Touch::Field | Touch::Moved => (true, true),
            };
            let ((was_in, was), (is_in, is)) = match direction {
                Direction::Forward => ((in_before, before), (in_after, after)),
                Direction::Backward => ((in_after, after), (in_before, before)),
            };
            if was_in {
                leave(ident_index, derived, id, was);
            }
            if is_in {
                enter(ident_index, derived, id, is);
            }
        }
    }
}

/// `element`, at `id`, leaving the derived state.
fn leave(idents: &mut IdentIndex, derived: &mut Derived, id: NodeId, element: &DocElement) {
    derived.budget.leave(element);
    match element {
        DocElement::Node(node) => {
            if idents.get(&node.ident) == Some(&id) {
                idents.remove(&node.ident);
            }
        }
        DocElement::Link(link) => derived.link_out(link.to, id),
        _ => {}
    }
}

/// `element`, at `id`, entering the derived state.
fn enter(idents: &mut IdentIndex, derived: &mut Derived, id: NodeId, element: &DocElement) {
    derived.budget.enter(element);
    match element {
        DocElement::Node(node) if node.ident != NodeIdent::UNASSIGNED => {
            idents.insert(node.ident, id);
        }
        DocElement::Link(link) => derived.link_in(link.to, id),
        _ => {}
    }
}
