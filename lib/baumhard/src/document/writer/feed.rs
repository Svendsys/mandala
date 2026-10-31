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

use crate::document::element::{DocElement, DocKind};
use crate::document::ids::{owning_node, NodeIdent};
use crate::document::writer::{ChangeSource, Revision};
use crate::engine::element::TreeElement;
use crate::engine::journal::{Journal, Touch};
use crate::engine::key::BranchKey;
use crate::engine::tree::Tree;
use indextree::NodeId;
use rustc_hash::{FxHashMap, FxHashSet};

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
    /// which link of a node.
    pub index: usize,
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
/// Cost: per element O(1) — the node it belongs to is carried down from
/// its parent, read for the parent of each subtree's top by one climb
/// within that parent's node ([`owning_node`]), never per level of the
/// tree (§20 rule 11); per distinct parent, one read of its row.
/// Allocates the result, a rank per sibling of a parent read, and the
/// owner of each element.
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
    let mut reader = journal.map(Journal::rows_before);
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
    let mut ranks: FxHashMap<NodeId, usize> = FxHashMap::default();
    let mut read: FxHashSet<NodeId> = FxHashSet::default();
    let mut row = Vec::new();
    let mut out = Vec::new();
    for id in leaving {
        let Some(element) = state(id) else {
            continue;
        };
        let parent = parent_of(id);
        if let Some(parent) = parent.filter(|&parent| read.insert(parent)) {
            match reader.as_mut() {
                Some(reader) => reader.row(tree, parent, &mut row),
                None => {
                    row.clear();
                    row.extend(parent.children(arena));
                }
            }
            // A row is sorted by key, so the siblings of one key are a
            // run: a rank is the distance from the run's start.
            let mut run: Option<(BranchKey<DocKind>, usize)> = None;
            for &sibling in &row {
                let Some(key) = state(sibling).map(TreeElement::key) else {
                    continue;
                };
                let rank = match run {
                    Some((current, rank)) if current == key => rank + 1,
                    _ => 0,
                };
                run = Some((key, rank));
                ranks.insert(sibling, rank);
            }
        }
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
                index: ranks.get(&id).copied().unwrap_or(0),
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
