// SPDX-License-Identifier: MPL-2.0

//! One request, run: its steps on one journal builder, the writer's own
//! steps after them, and the checks, all or nothing (§2 of #238).
//!
//! 1. Every step of the request runs on one builder
//!    (`Document::run_steps`): each built from the document as the steps
//!    before it left it, validated, walked. A step that fails abandons
//!    the builder.
//! 2. **Idents.** Every node the request inserted, still in the tree and
//!    still [unassigned](crate::document::ids::NodeIdent::UNASSIGNED),
//!    is given the next ident, recorded on the same builder — so the
//!    assignment is part of the request's journal, undone and redone with
//!    it. Nodes are taken subtree by subtree, in the order the request
//!    created them, each subtree in pre-order. No ident past
//!    [`MAX_NODE_IDENT`](crate::document::bounds::MAX_NODE_IDENT) is
//!    issued: the request is refused instead.
//! 3. **Links.** Every link left leading into a subtree the request took
//!    out of the tree is removed, by one more step on the same builder
//!    (`compose::remove_links`). They are found from the document's
//!    index of incoming links and from the links the request itself
//!    inserted, never by a scan.
//! 4. The builder is closed, and the journal judged once
//!    ([`validate_applied`]): the shape of every row the request changed,
//!    the parents and the moved nodes included, and every inserted link.
//! 5. The document's budget is checked against this target's bounds.
//!
//! A failure at any point leaves the document exactly as it was: the
//! builder is abandoned, or the closed journal reverted and discarded,
//! and the ident counter put back.

use crate::document::element::DocElement;
use crate::document::ids::{Document, NodeIdent};
use crate::document::mutation::compose::remove_links;
use crate::document::mutation::step::DocStep;
use crate::document::mutation::validate::validate_applied;
use crate::document::writer::budget::Budget;
use crate::document::writer::error::ApplyError;
use crate::engine::journal::{Journal, JournalBuilder, Touch};
use crate::engine::tree::TreeError;
use indextree::NodeId;
use log::error;
use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

impl Document {
    /// Run `steps` as one request (see the module docs) and return its
    /// closed, judged journal, with the tree as it left it. The derived
    /// state — the indexes, the budget — is not moved yet: the caller
    /// [accounts](Document::account) for the journal once it keeps it.
    ///
    /// Cost: the steps', plus the journal's: per inserted subtree a
    /// pre-order of it, per removed subtree a pre-order of it with a
    /// lookup per node in the incoming-link index, and `validate_applied`.
    pub(super) fn run_request(&mut self, steps: Vec<DocStep>) -> Result<Journal<DocElement>, ApplyError> {
        let counter = self.next_ident;
        let result = self.run_request_unguarded(steps);
        if result.is_err() {
            // The idents the refused request was given were never seen.
            self.next_ident = counter;
        }
        result
    }

    fn run_request_unguarded(&mut self, steps: Vec<DocStep>) -> Result<Journal<DocElement>, ApplyError> {
        let builder = self.run_steps(steps, JournalBuilder::new())?;
        let builder = self.assign_idents(builder)?;
        let builder = self.remove_dangling_links(builder)?;
        let journal = builder.close(&self.tree);
        if let Err(error) = validate_applied(&self.tree, &journal) {
            self.drop_journal(journal);
            return Err(error.into());
        }
        if let Err(error) = self.within_budget(&journal) {
            self.drop_journal(journal);
            return Err(error);
        }
        Ok(journal)
    }

    /// Give every unassigned node the request inserted the next ident, on
    /// `builder`. Abandons the builder and refuses when no ident is left.
    fn assign_idents(
        &mut self,
        mut builder: JournalBuilder<DocElement>,
    ) -> Result<JournalBuilder<DocElement>, ApplyError> {
        let arena = self.tree.arena();
        let mut fresh = Vec::new();
        for top in builder.created_nodes() {
            if !self.tree.contains(top) {
                continue;
            }
            fresh.extend(top.descendants(arena).filter(|&id| {
                matches!(arena[id].get(), DocElement::Node(node) if node.ident == NodeIdent::UNASSIGNED)
            }));
        }
        for id in fresh {
            let Some(ident) = self.next_ident() else {
                builder.abandon(&mut self.tree);
                return Err(ApplyError::IdentSpaceExhausted {
                    max: crate::document::bounds::MAX_NODE_IDENT,
                });
            };
            // `next_ident` is at most `MAX_NODE_IDENT`, one below
            // `u32::MAX`, so this cannot wrap.
            self.next_ident = ident.0 + 1;
            self.tree
                .update_journaled(id, &mut builder, |element| {
                    if let DocElement::Node(node) = element {
                        node.ident = ident;
                    }
                })
                .map_err(|error: TreeError| ApplyError::Invalid(error.into()))?;
        }
        Ok(builder)
    }

    /// Remove, by one more step on `builder`, every link in the tree that
    /// leads into a subtree the request took out of it: the links the
    /// incoming-link index holds for each node of such a subtree, and the
    /// links the request itself inserted or changed.
    fn remove_dangling_links(
        &mut self,
        builder: JournalBuilder<DocElement>,
    ) -> Result<JournalBuilder<DocElement>, ApplyError> {
        let tree = &self.tree;
        let arena = tree.arena();
        let root = tree.root();
        let detached =
            |id: &NodeId| *id != root && arena.get(*id).is_some_and(|node| node.parent().is_none());
        let tops: Vec<NodeId> = builder.recorded_nodes().filter(detached).collect();
        if tops.is_empty() {
            return Ok(builder);
        }
        // The links this request put in place, by where they lead: the
        // index holds them only once the request is accounted for.
        let mut local: FxHashMap<NodeId, SmallVec<[NodeId; 2]>> = FxHashMap::default();
        let mut note = |id: NodeId| {
            if let DocElement::Link(link) = arena[id].get() {
                local.entry(link.to).or_default().push(id);
            }
        };
        for id in builder.recorded_nodes() {
            if tree.contains(id) {
                note(id);
            }
        }
        for top in builder.created_nodes() {
            if tree.contains(top) {
                top.descendants(arena).skip(1).for_each(&mut note);
            }
        }
        let mut doomed = Vec::new();
        let mut seen: FxHashSet<NodeId> = FxHashSet::default();
        for top in tops {
            for gone in top.descendants(arena) {
                if !matches!(arena[gone].get(), DocElement::Node(_)) {
                    continue;
                }
                let indexed = self.derived.incoming.get(&gone).into_iter().flatten();
                let inserted = local.get(&gone).into_iter().flatten();
                for &link in indexed.chain(inserted) {
                    let leads_here = matches!(
                        arena.get(link).map(|node| node.get()),
                        Some(DocElement::Link(data)) if data.to == gone
                    );
                    if leads_here && tree.contains(link) && seen.insert(link) {
                        doomed.push(link);
                    }
                }
            }
        }
        if doomed.is_empty() {
            return Ok(builder);
        }
        Ok(self.run_steps(vec![remove_links(doomed)], builder)?)
    }

    /// Refuse `journal` if accounting for it would take the document past
    /// this target's bounds. O(the journal's touches).
    fn within_budget(&self, journal: &Journal<DocElement>) -> Result<(), ApplyError> {
        let limits = self.writer.limits;
        if limits.elements.is_none() && limits.bytes.is_none() {
            return Ok(());
        }
        let mut budget: Budget = self.derived.budget;
        let arena = self.tree.arena();
        for (id, touch) in journal.touched() {
            let Some(current) = arena.get(id).map(|node| node.get()) else {
                continue;
            };
            match touch {
                Touch::Inserted => budget.enter(journal.after(id).unwrap_or(current)),
                Touch::Removed => budget.leave(journal.before(id).unwrap_or(current)),
                Touch::Field | Touch::Moved => {
                    budget.leave(journal.before(id).unwrap_or(current));
                    budget.enter(journal.after(id).unwrap_or(current));
                }
            }
        }
        if let Some(max) = limits.elements.filter(|&max| budget.elements > max) {
            return Err(ApplyError::TooManyElements {
                elements: budget.elements,
                max,
            });
        }
        if let Some(max) = limits.bytes.filter(|&max| budget.text > max) {
            return Err(ApplyError::TooMuchText {
                bytes: budget.text,
                max,
            });
        }
        Ok(())
    }

    /// Take a closed journal back out of the tree and free what it holds:
    /// a refused request, or a journal that changed nothing.
    pub(super) fn drop_journal(&mut self, journal: Journal<DocElement>) {
        if journal.is_empty() {
            journal.discard(&mut self.tree);
            return;
        }
        match journal.revert(&mut self.tree) {
            Ok(()) => journal.discard(&mut self.tree),
            Err(error) => error!("document writer: could not undo a refused request: {error}"),
        }
    }
}
