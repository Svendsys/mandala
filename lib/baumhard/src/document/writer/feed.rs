// SPDX-License-Identifier: MPL-2.0

//! The change feed: one [`ChangeRecord`]
//! per change the writer makes, drained by
//! [`Document::take_changes`](crate::document::ids::Document::take_changes).
//!
//! A record names what changed — every element the change touched, with
//! how — and nothing about values: whoever reads the feed reads the
//! document as it now stands for each touched element (the projector,
//! V1-B7). An element that left the tree cannot be read any more, so a
//! record also carries the [address](crate::document::writer::feed::RemovedAddress)
//! each removed element had, captured when it left; that is what the IPC
//! event stream reports it by (V1-E2).

use crate::arena::NodeId;
use crate::document::element::{DocElement, DocKind};
use crate::document::ids::{owning_node, NodeIdent};
use crate::document::writer::{ChangeSource, Revision};
use crate::engine::element::TreeElement;
use crate::engine::journal::{Journal, Touch};
use crate::engine::tree::Tree;
use rustc_hash::FxHashMap;

/// One change the writer made: an apply, a stage, a commit, a cancel,
/// a base call of a transaction, an interpolation step of an animated
/// one, an undo or a redo.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangeRecord {
    /// Who made it. An undo is [`ChangeSource::Undo`] and a redo
    /// [`ChangeSource::Redo`], whoever asked for them; a transaction's
    /// records carry the source it was begun with.
    pub source: ChangeSource,
    /// The document's revision once the change was made. A stage, a
    /// cancel and a base call leave it where it was; an apply, a commit,
    /// an undo and a redo advance it.
    pub revision: Revision,
    /// Every element the change touched, with what happened to it, in the
    /// direction the change went: an element an undo takes out of the
    /// tree is [`Touch::Removed`], one it puts back
    /// [`Touch::Inserted`]. An element is named once per journal the
    /// change wrote, so a change that writes several — a commit, the undo
    /// of a transaction, a stage that replaces an earlier one — can name
    /// it more than once.
    pub touched: Vec<(NodeId, Touch)>,
    /// Every element the change took out of the tree, with the address it
    /// had there.
    pub removed: Vec<(NodeId, RemovedAddress)>,
}

/// Where an element was when it was taken out of the tree, in the terms
/// someone outside the document can name it by: the node it belonged to,
/// and its place among that node's elements.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RemovedAddress {
    /// The id of the node it belonged to — for a node, its own id;
    /// `None` for an element of the map's own row (the theme, a palette,
    /// the defaults).
    pub node: Option<NodeIdent>,
    /// What it was.
    pub kind: DocKind,
    /// Its channel ([`DocElement::channel`]): a node's, a section's, a
    /// ring's, a portal end's; `0` for every other kind.
    pub channel: usize,
    /// Its place among the siblings of its kind and channel under the
    /// element it hung from, counted from `0`: which span of a section,
    /// which link of a node. `None` for a node, which its id names
    /// (ruling R36 on #238).
    pub index: Option<usize>,
}

/// Where the rows that removed elements are read from.
pub(super) enum Arrangement<'j> {
    /// The tree as it stands: the elements are still in it, about to be
    /// taken out by a revert, a replay or an interpolation step.
    Current,
    /// The arrangement before `journal`, which the tree has just left:
    /// a request's journal, closed with its removals done.
    Before(&'j Journal<DocElement>),
}

/// The address of each of `leaving` — elements in `tree` in the
/// arrangement `rows` names, in pre-order, as a journal's touches name
/// them — in that arrangement.
///
/// Cost: per element, the node it belongs to is carried down from its
/// parent — read for the parent of each subtree's top by one climb
/// within that parent's node ([`owning_node`]), never per level of the
/// tree (§20 rule 11) — and, for an element other than a node, its rank
/// is found by walking back through the siblings of its own key alone,
/// each walked through once per call however many leave together (a
/// memo of the ranks found)
/// ([`RowsBefore::prev`](crate::engine::journal::RowsBefore::prev) in the arrangement before a journal, the arena
/// otherwise). No row is read whole: the work is the journal's, and the
/// same-key run of each removed element that is not a node (ruling R36).
/// Allocates the result and the owner of each element.
pub(super) fn addresses(
    tree: &Tree<DocElement>,
    leaving: impl IntoIterator<Item = NodeId>,
    rows: Arrangement<'_>,
) -> Vec<(NodeId, RemovedAddress)> {
    let arena = tree.arena();
    let journal = match rows {
        Arrangement::Current => None,
        Arrangement::Before(journal) => Some(journal),
    };
    let reader = journal.map(Journal::rows_before);
    // The element at `id` in the arrangement read: the journal's
    // before-image when it changed it, the arena's otherwise.
    let state = |id: NodeId| -> Option<&DocElement> {
        journal
            .and_then(|journal| journal.before(id))
            .or_else(|| arena.get(id).map(|node| node.get()))
    };
    // Where `id` hung in that arrangement: its parent in the arena, or —
    // at the top of a subtree the journal took out — the parent the
    // journal recorded.
    let parent_of = |id: NodeId| -> Option<NodeId> {
        arena.get(id)?.parent().or_else(|| {
            journal
                .and_then(|journal| journal.parents(id))
                .and_then(|(before, _)| before)
        })
    };
    // The ident of the node each element read so far belongs to. A
    // journal names a subtree from its top down, so an element's parent
    // is here before it is, unless the parent stayed in the tree.
    let mut owners: FxHashMap<NodeId, Option<NodeIdent>> = FxHashMap::default();
    let ident_of = |id: NodeId| state(id).and_then(DocElement::as_node).map(|node| node.ident);
    // The sibling right before `id` under `parent`, in the arrangement
    // read.
    let prev = |parent: NodeId, id: NodeId| -> Option<NodeId> {
        #[cfg(test)]
        steps::count();
        match &reader {
            Some(reader) => reader.prev(tree, parent, id),
            None => arena.get(id)?.previous_sibling(),
        }
    };
    // The rank of every sibling this call has walked through, and the
    // siblings of the walk under way.
    let mut ranks: FxHashMap<NodeId, usize> = FxHashMap::default();
    let mut passed: Vec<NodeId> = Vec::new();
    let mut out = Vec::new();
    for id in leaving {
        let Some(element) = state(id) else {
            continue;
        };
        let parent = parent_of(id);
        // A row is sorted by key, so the siblings of one key are a run,
        // and a rank is the distance back to the run's start.
        let index = match (element, parent) {
            (DocElement::Node(_), _) => None,
            (_, None) => Some(0),
            (_, Some(parent)) => {
                // Walk back to the run's start, or to a sibling this call
                // has ranked already, then rank every sibling passed: each
                // sibling is walked through once per call, however many of
                // its run leave together.
                let key = element.key();
                passed.clear();
                let mut at = id;
                let mut base = 0;
                loop {
                    passed.push(at);
                    match prev(parent, at).filter(|&b| state(b).map(TreeElement::key) == Some(key)) {
                        Some(before) => match ranks.get(&before) {
                            Some(&rank) => {
                                base = rank + 1;
                                break;
                            }
                            None => at = before,
                        },
                        None => break,
                    }
                }
                for (offset, &sibling) in passed.iter().rev().enumerate() {
                    ranks.insert(sibling, base + offset);
                }
                ranks.get(&id).copied()
            }
        };
        let node = match (element, parent) {
            (DocElement::Node(node), _) => Some(node.ident),
            (_, None) => None,
            (_, Some(parent)) => match owners.get(&parent) {
                Some(&owner) => owner,
                None => owning_node(tree, parent).and_then(ident_of),
            },
        };
        owners.insert(id, node);
        out.push((
            id,
            RemovedAddress {
                node,
                kind: element.kind(),
                channel: element.channel(),
                index,
            },
        ));
    }
    out
}

/// `touch` as seen from the other side of its journal: what an undo does
/// to an element the edit inserted is to remove it.
pub(super) fn reversed(touch: Touch) -> Touch {
    match touch {
        Touch::Inserted => Touch::Removed,
        Touch::Removed => Touch::Inserted,
        other => other,
    }
}

/// How many siblings [`addresses`] has stepped back through on this
/// thread: what the test that holds it to the journal reads. Compiled for
/// tests only.
#[cfg(test)]
pub(crate) mod steps {
    use std::cell::Cell;

    thread_local! {
        static STEPS: Cell<usize> = const { Cell::new(0) };
    }

    /// One more sibling read.
    pub(super) fn count() {
        STEPS.with(|cell| cell.set(cell.get() + 1));
    }

    /// The siblings read on this thread so far.
    pub(crate) fn taken() -> usize {
        STEPS.with(Cell::get)
    }
}
