// SPDX-License-Identifier: MPL-2.0

//! The undo history: the entries applied, oldest first, and the redo
//! branch, each entry one request or one committed transaction — its
//! journals, its source, its label and the revision it produced.
//!
//! **Bounded** (D18): at most
//! [`DEFAULT_HISTORY_LIMIT`](crate::document::writer::DEFAULT_HISTORY_LIMIT)
//! entries unless the app sets another limit. An entry that leaves the
//! history — the oldest past the limit, or the redo branch a new change
//! clears — has its journals discarded, which frees the subtrees they
//! hold out of the tree: an applied entry's removals, an undone entry's
//! insertions. Nothing else frees them.
//!
//! **Positions have identities.** Every entry carries a serial no other
//! entry of the document ever has, and the state with no entry applied
//! carries one too, so "the document is as it was saved" is a question
//! about *which* state, never about a number of steps: an edit undone
//! back to the saved state is clean, and a different edit that brings
//! the history to the same length is not.

use crate::document::element::DocElement;
use crate::document::writer::{ChangeSource, Revision};
use crate::engine::journal::Journal;
use crate::engine::tree::Tree;
use std::collections::VecDeque;

/// One history entry.
#[derive(Clone, Debug)]
pub(super) struct HistoryEntry {
    /// What it did: one journal for a request, the base calls and then
    /// the final stage for a transaction. Undone newest first, redone
    /// oldest first.
    pub journals: Vec<Journal<DocElement>>,
    /// Who made it.
    pub source: ChangeSource,
    /// What it is called.
    pub label: String,
    /// The revision its change last produced: when it was applied, or
    /// when it was last redone.
    pub revision: Revision,
    /// Its identity, for the saved position.
    pub serial: u64,
}

/// What [`Document::history`](crate::document::ids::Document::history)
/// yields for one applied entry.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HistoryInfo<'h> {
    /// The entry's label, as its request named it.
    pub label: &'h str,
    /// Who made it.
    pub source: ChangeSource,
    /// The revision its change last produced.
    pub revision: Revision,
}

/// The history. See the module docs.
#[derive(Clone, Debug)]
pub(super) struct History {
    /// Applied, oldest first.
    applied: VecDeque<HistoryEntry>,
    /// Undone, the next to redo last.
    undone: Vec<HistoryEntry>,
    /// The most entries held, applied and undone together.
    limit: usize,
    /// The serial the next entry gets.
    next_serial: u64,
    /// The serial of the state with no entry applied: `0` for the
    /// document as it was loaded, the serial of the newest entry
    /// dropped off the end otherwise.
    base: u64,
    /// The position [`Self::mark_saved`] last recorded.
    saved: u64,
}

impl History {
    /// An empty history holding at most `limit` entries, saved where it
    /// stands. O(1).
    pub(super) fn new(limit: usize) -> Self {
        History {
            applied: VecDeque::new(),
            undone: Vec::new(),
            limit,
            next_serial: 1,
            base: 0,
            saved: 0,
        }
    }

    /// The identity of the state the document is in: the newest applied
    /// entry's serial, or the base's. O(1).
    pub(super) fn position(&self) -> u64 {
        self.applied.back().map_or(self.base, |entry| entry.serial)
    }

    /// Record the current position as the saved one. O(1).
    pub(super) fn mark_saved(&mut self) {
        self.saved = self.position();
    }

    /// Whether the document has moved from the saved position. O(1).
    pub(super) fn is_dirty(&self) -> bool {
        self.position() != self.saved
    }

    /// Push a new entry: clear the redo branch, then trim to the limit,
    /// discarding the journals of every entry that leaves.
    ///
    /// Cost: O(1) plus [`Journal::discard`] of each entry dropped.
    pub(super) fn push(
        &mut self,
        tree: &mut Tree<DocElement>,
        journals: Vec<Journal<DocElement>>,
        source: ChangeSource,
        label: String,
        revision: Revision,
    ) {
        for entry in self.undone.drain(..) {
            discard(tree, entry);
        }
        let serial = self.next_serial;
        self.next_serial += 1;
        self.applied.push_back(HistoryEntry {
            journals,
            source,
            label,
            revision,
            serial,
        });
        self.trim(tree);
    }

    /// Set the limit and trim to it. O(1) plus the discards.
    pub(super) fn set_limit(&mut self, tree: &mut Tree<DocElement>, limit: usize) {
        self.limit = limit;
        self.trim(tree);
    }

    /// Drop entries until at most `limit` are held: the oldest applied
    /// first, then — with none applied left — the undone entry that would
    /// be redone last.
    fn trim(&mut self, tree: &mut Tree<DocElement>) {
        while self.applied.len() + self.undone.len() > self.limit {
            if let Some(oldest) = self.applied.pop_front() {
                self.base = oldest.serial;
                discard(tree, oldest);
            } else if !self.undone.is_empty() {
                let farthest = self.undone.remove(0);
                discard(tree, farthest);
            } else {
                break;
            }
        }
    }

    /// Take the newest applied entry, to undo it. O(1).
    pub(super) fn take_applied(&mut self) -> Option<HistoryEntry> {
        self.applied.pop_back()
    }

    /// Put back an entry [`Self::take_applied`] gave, whose undo failed.
    pub(super) fn restore_applied(&mut self, entry: HistoryEntry) {
        self.applied.push_back(entry);
    }

    /// Hold an undone entry for redo. O(1).
    pub(super) fn push_undone(&mut self, entry: HistoryEntry) {
        self.undone.push(entry);
    }

    /// Take the next entry to redo. O(1).
    pub(super) fn take_undone(&mut self) -> Option<HistoryEntry> {
        self.undone.pop()
    }

    /// Hold a redone entry as applied again, without touching the redo
    /// branch. O(1).
    pub(super) fn push_redone(&mut self, entry: HistoryEntry) {
        self.applied.push_back(entry);
    }

    /// The applied entries, oldest first.
    pub(super) fn applied(&self) -> impl Iterator<Item = HistoryInfo<'_>> + '_ {
        self.applied.iter().map(|entry| HistoryInfo {
            label: &entry.label,
            source: entry.source,
            revision: entry.revision,
        })
    }

    /// How many entries a redo can reach. O(1).
    pub(super) fn redo_len(&self) -> usize {
        self.undone.len()
    }

    /// The most entries held. O(1).
    pub(super) fn limit(&self) -> usize {
        self.limit
    }
}

/// Free what `entry`'s journals hold out of the tree: they leave the
/// history, and no journal can restore them again.
fn discard(tree: &mut Tree<DocElement>, entry: HistoryEntry) {
    for journal in entry.journals {
        journal.discard(tree);
    }
}
