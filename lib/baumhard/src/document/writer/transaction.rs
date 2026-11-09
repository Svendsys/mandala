// SPDX-License-Identifier: MPL-2.0

//! Transactions: live interaction as one history entry (design §9, D28,
//! rulings R7 on #238).
//!
//! A transaction has a **base** and a **stage**.
//!
//! - [`Document::stage`](crate::document::ids::Document::stage) is what
//!   a drag, a resize, an editor keystroke or a color-picker hover calls
//!   each time: it reverts the previous stage, discards its journal, and
//!   applies the new request as the stage. Stages replace each other —
//!   a drag stages `position = start + total delta`, never an
//!   accumulation of per-frame deltas — so the document is exact at
//!   every frame, and what is drawn is the document.
//! - [`Document::apply_in`](crate::document::ids::Document::apply_in)
//!   applies a request into the base, where it stays across stages: the
//!   node an editor session creates keeps its `NodeId` and its ident
//!   however often the text in it is restaged. A base call lifts the
//!   current stage first (the caller restages on top of it).
//! - [`Document::commit`](crate::document::ids::Document::commit) makes
//!   the base calls and the last stage one history entry, in that order;
//!   [`Document::cancel`](crate::document::ids::Document::cancel) reverts
//!   them all, newest first, and discards them.
//!
//! **An animated transaction** (§10.1) stages the edit once, to learn its
//! journal, then writes it part-way at each frame
//! ([`Document::interpolate`](crate::document::ids::Document::interpolate)).
//!
//! **One open at a time, and who yields** (D28). While an interactive
//! transaction is open, an `apply` or a `begin` is refused with
//! [`ApplyError::TransactionOpen`]. An animated one yields: it is
//! fast-forwarded to its end and committed, and the change proceeds —
//! and stays committed if the change is then refused. An undo or a redo
//! is one visible step (ruling R36): it cancels an interactive
//! transaction and stops there, and makes an animated one an entry of
//! its own before it goes on.

use crate::arena::NodeId;
use crate::document::element::DocElement;
use crate::document::ids::Document;
use crate::document::writer::error::ApplyError;
use crate::document::writer::feed::{
    addresses, effects, field_inputs, reversed, Arrangement, RemovedAddress,
};
use crate::document::writer::upkeep::Direction;
use crate::document::writer::{Change, ChangeSource, EditRequest, Revision};
use crate::engine::interpolate::Snap;
use crate::engine::journal::{Journal, Touch};
use log::error;
use std::sync::atomic::{AtomicU64, Ordering};

/// The serial the next transaction gets, in any document: a handle names
/// one transaction of one document, never one of another.
static NEXT_TRANSACTION: AtomicU64 = AtomicU64::new(1);

/// Whether a transaction is someone's live interaction or an animated
/// edit (D28).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TransactionKind {
    /// A text editor session, a drag or a resize, the color picker, a
    /// frame preview: refuses every other change while it is open.
    Interactive,
    /// An animated edit: written part-way at each frame, and
    /// fast-forwarded to its end and committed when another change
    /// arrives.
    Animated,
}

/// The open transaction, as its owner holds it: the only handle that can
/// stage into it, commit it or cancel it. Not `Clone`; consumed by
/// [`Document::commit`] and
/// [`Document::cancel`]. A
/// transaction that ended another way — an undo cancelled it, another
/// change fast-forwarded it — leaves its handle stale, and every call
/// with it is refused ([`ApplyError::TransactionClosed`]).
#[must_use = "a transaction is committed or cancelled"]
#[derive(Debug, PartialEq, Eq)]
pub struct Transaction {
    serial: u64,
    kind: TransactionKind,
}

impl Transaction {
    /// Whether it is interactive or animated. O(1).
    pub fn kind(&self) -> TransactionKind {
        self.kind
    }
}

/// The open transaction, as the writer holds it.
#[derive(Clone, Debug)]
pub(super) struct Open {
    serial: u64,
    source: ChangeSource,
    kind: TransactionKind,
    /// The label of the request applied or staged last: the entry's,
    /// once committed.
    label: String,
    /// The base calls, oldest first.
    base: Vec<Journal<DocElement>>,
    /// What the base calls took out of the tree, with their addresses.
    base_removed: Vec<(NodeId, RemovedAddress)>,
    /// The current stage.
    stage: Option<Staged>,
}

impl Open {
    /// Whether it is interactive or animated. O(1).
    pub(super) fn kind(&self) -> TransactionKind {
        self.kind
    }
}

/// A stage, and where it stands.
#[derive(Clone, Debug)]
struct Staged {
    journal: Journal<DocElement>,
    /// What it took out of the tree, with their addresses.
    removed: Vec<(NodeId, RemovedAddress)>,
    /// How far it is written: `1` for a stage as applied.
    t: f32,
    /// The snap its last part-way write used.
    snap: Snap,
    /// Whether its structure and its snapping fields stand at their after
    /// side — what the derived state was last accounted for.
    after: bool,
}

/// What a change has done so far, for its record.
type Done = Change;

impl Done {
    /// `journal` written forward, or backward when `back`: its touches,
    /// and what they changed for the readers ([`effects`]).
    fn wrote(&mut self, journal: &Journal<DocElement>, back: bool) {
        self.touched.extend(
            journal
                .touched()
                .map(|(id, touch)| (id, if back { reversed(touch) } else { touch })),
        );
        effects(journal, back, &mut self.inputs, &mut self.rows);
    }
}

impl Document {
    /// Open a transaction for `source`, of `kind`.
    ///
    /// An open interactive transaction refuses it
    /// ([`ApplyError::TransactionOpen`]); an open animated one is
    /// fast-forwarded and committed first (D28), and stays committed. A
    /// source no change may declare is refused
    /// ([`ApplyError::Source`]), before anything is committed.
    ///
    /// Cost: O(1), or the commit of the animated transaction it yields.
    pub fn begin(&mut self, source: ChangeSource, kind: TransactionKind) -> Result<Transaction, ApplyError> {
        self.mutate(|doc| {
            ChangeSource::check(source)?;
            doc.yield_open()?;
            let serial = NEXT_TRANSACTION.fetch_add(1, Ordering::Relaxed);
            doc.writer.open = Some(Open {
                serial,
                source,
                kind,
                label: String::new(),
                base: Vec::new(),
                base_removed: Vec::new(),
                stage: None,
            });
            Ok(Transaction { serial, kind })
        })
    }

    /// Stage `req` in `tx`: revert the previous stage, then run `req`
    /// through the whole of [`Document::apply`]'s pipeline — validation,
    /// its walks, the writer's own steps, the checks — as the new stage,
    /// which is not in the history. On success the previous stage's
    /// journal is discarded; on failure it is put back, and the document
    /// is as it was before the call.
    ///
    /// Errors: [`ApplyError::TransactionClosed`] for a handle that is not
    /// the open transaction's, and every refusal of [`Document::apply`]
    /// but [`ApplyError::TransactionOpen`].
    ///
    /// Cost: the previous stage's revert and the request's run, each
    /// proportional to its journal.
    pub fn stage(&mut self, tx: &Transaction, req: EditRequest) -> Result<(), ApplyError> {
        self.mutate(|doc| doc.run_in(tx, req, false))
    }

    /// Apply `req` into `tx`'s base, where it stays across stages until
    /// the transaction commits or is cancelled: what a session needs
    /// once and then edits — the node `CreateOrphanNodeAndEdit` inserts
    /// before its text is staged into it. The current stage is lifted
    /// first and not restaged; the caller stages again on top.
    ///
    /// Errors and cost: [`Document::stage`]'s.
    pub fn apply_in(&mut self, tx: &Transaction, req: EditRequest) -> Result<(), ApplyError> {
        self.mutate(|doc| doc.run_in(tx, req, true))
    }

    /// Write `tx`'s stage part-way: at progress `t` (clamped to `0..=1`,
    /// `NaN` read as `0`), every field that interpolates blends between
    /// its value before the stage and after it, and every other field —
    /// and the structure — stands on the side `snap` picks
    /// ([`Journal::write_interpolated`]). An animated edit calls it at
    /// each frame. A transaction with no stage has nothing to write.
    ///
    /// Errors: [`ApplyError::TransactionClosed`], and
    /// [`ApplyError::NotAnimated`] for an interactive transaction.
    ///
    /// Cost: one interpolation per entry of the stage's journal; when the
    /// structure changes side, the derived state moves with it.
    pub fn interpolate(&mut self, tx: &Transaction, t: f32, snap: Snap) -> Result<(), ApplyError> {
        self.mutate(|doc| {
            let source = doc.open_for(tx)?.source;
            if tx.kind != TransactionKind::Animated {
                return Err(ApplyError::NotAnimated);
            }
            let Some(mut stage) = doc.open_mut().and_then(|open| open.stage.take()) else {
                return Ok(());
            };
            let mut done = Done::default();
            doc.write_stage(&mut stage, t, snap, &mut done);
            if let Some(open) = doc.open_mut() {
                open.stage = Some(stage);
            }
            doc.record(source, done);
            Ok(())
        })
    }

    /// End `tx`, making its base calls and its stage — fast-forwarded to
    /// its end, for an animated transaction — one history entry, exactly
    /// as [`Document::apply`] makes one of a request. Returns the new
    /// revision, or `None` when the transaction changed nothing (no
    /// entry, the revision unchanged) or `tx` is not the open one.
    ///
    /// Cost: what [`Document::apply`]'s history step costs; a
    /// fast-forward writes the stage once more.
    pub fn commit(&mut self, tx: Transaction) -> Option<Revision> {
        self.mutate(|doc| {
            doc.open_for(&tx).ok()?;
            doc.commit_open()
        })
    }

    /// Whether `tx` names the transaction open now. A handle is **stale**
    /// once its transaction ended by a route other than its own
    /// [`Self::commit`] or [`Self::cancel`], which consume it: an
    /// [`Self::undo`] or a [`Self::redo`] cancels an interactive
    /// transaction or finishes and commits an animated one (ruling R36),
    /// and an [`Self::apply`] or [`Self::begin`] finishes an animated one
    /// first (D28). Every call with a stale handle is refused
    /// ([`ApplyError::TransactionClosed`]); this is how its holder learns
    /// it went stale without making one. A handle of another document is
    /// never open here. O(1).
    pub fn is_open(&self, tx: &Transaction) -> bool {
        self.open_for(tx).is_ok()
    }

    /// End `tx`, reverting its stage and its base calls, newest first,
    /// and discarding their journals: the document is as it was when the
    /// transaction began. Nothing for a `tx` that is not the open one.
    ///
    /// Cost: a revert of each of its journals.
    pub fn cancel(&mut self, tx: Transaction) {
        self.mutate(|doc| {
            if doc.open_for(&tx).is_ok() {
                doc.cancel_open();
            }
        })
    }

    /// The open transaction, if `tx` is its handle.
    fn open_for(&self, tx: &Transaction) -> Result<&Open, ApplyError> {
        match &self.writer.open {
            Some(open) if open.serial == tx.serial => Ok(open),
            _ => Err(ApplyError::TransactionClosed),
        }
    }

    fn open_mut(&mut self) -> Option<&mut Open> {
        self.writer.open.as_mut()
    }

    /// D28, for a change that is about to start: refuse it while an
    /// interactive transaction is open; fast-forward and commit an
    /// animated one.
    pub(super) fn yield_open(&mut self) -> Result<(), ApplyError> {
        match self.writer.open.as_ref().map(|open| open.kind) {
            None => Ok(()),
            Some(TransactionKind::Interactive) => Err(ApplyError::TransactionOpen),
            Some(TransactionKind::Animated) => {
                self.commit_open();
                Ok(())
            }
        }
    }

    /// The body of [`Document::stage`] (`into_base` false) and
    /// [`Document::apply_in`] (true).
    fn run_in(&mut self, tx: &Transaction, req: EditRequest, into_base: bool) -> Result<(), ApplyError> {
        let source = self.open_for(tx)?.source;
        let mut done = Done::default();
        let previous = self.open_mut().and_then(|open| open.stage.take());
        let previous = previous.map(|mut stage| {
            let was = (stage.t, stage.snap);
            self.lift(&mut stage, &mut done);
            (stage, was)
        });
        let EditRequest { steps, label } = req;
        let journal = match self.run_request(steps) {
            Ok(journal) => journal,
            Err(error) => {
                // Put the stage back where it stood: the call changed
                // nothing, so it records nothing.
                if let Some((mut stage, (t, snap))) = previous {
                    self.write_stage(&mut stage, t, snap, &mut Done::default());
                    if let Some(open) = self.open_mut() {
                        open.stage = Some(stage);
                    }
                }
                return Err(error);
            }
        };
        if let Some((stage, _)) = previous {
            stage.journal.discard(&mut self.tree);
        }
        let removed = addresses(&self.tree, removed_by(&journal), Arrangement::Before(&journal));
        self.account(&journal, Direction::Forward);
        done.wrote(&journal, false);
        done.removed.extend(removed.iter().copied());
        if let Some(open) = self.open_mut() {
            open.label = label;
            if into_base {
                open.base.push(journal);
                open.base_removed.extend(removed);
            } else {
                open.stage = Some(Staged {
                    journal,
                    removed,
                    t: 1.0,
                    snap: Snap::End,
                    after: true,
                });
            }
        }
        self.record(source, done);
        Ok(())
    }

    /// Write `stage` back to where it started — `t = 0`, which is what a
    /// revert leaves — keeping its journal.
    fn lift(&mut self, stage: &mut Staged, done: &mut Done) {
        self.write_stage(stage, 0.0, stage.snap, done);
    }

    /// Write `stage` at `t` under `snap`, moving the derived state when
    /// its structure changes side and noting what it touched and removed
    /// in `done`. A stage written at `1` is replayed and one at `0`
    /// reverted outright; anything between is interpolated.
    fn write_stage(&mut self, stage: &mut Staged, t: f32, snap: Snap, done: &mut Done) {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        let after = snap.takes_after(t);
        let crossing = after != stage.after;
        if crossing {
            let leaving: Vec<NodeId> = if after {
                removed_by(&stage.journal).collect()
            } else {
                inserted_by(&stage.journal).collect()
            };
            done.removed
                .extend(addresses(&self.tree, leaving, Arrangement::Current));
        }
        let at_end = stage.t >= 1.0 && stage.after;
        let at_start = stage.t <= 0.0 && !stage.after;
        let written = if (t >= 1.0 && at_end) || (t <= 0.0 && at_start) {
            stage.t = t;
            return;
        } else if t >= 1.0 && at_start {
            stage.journal.replay(&mut self.tree)
        } else if t <= 0.0 && at_end {
            stage.journal.revert(&mut self.tree)
        } else {
            stage.journal.write_interpolated(&mut self.tree, t, snap)
        };
        if let Err(error) = written {
            error!("document writer: could not write the staged change: {error}");
            return;
        }
        if crossing {
            let direction = if after {
                Direction::Forward
            } else {
                Direction::Backward
            };
            self.account(&stage.journal, direction);
            done.wrote(&stage.journal, !after);
        } else {
            // Only values moved: every element the stage holds in the
            // tree on this side was rewritten, within what its journal's
            // two images differ in — which is what each reports as its
            // inputs, whichever way between them the write went.
            for (id, touch) in stage.journal.touched() {
                let held = match touch {
                    Touch::Inserted => after,
                    Touch::Removed => !after,
                    Touch::Field | Touch::Moved => true,
                };
                if held {
                    done.touched.push((id, Touch::Field));
                    field_inputs(&stage.journal, id, false, &mut done.inputs);
                }
            }
        }
        stage.t = t;
        stage.snap = snap;
        stage.after = after;
    }

    /// Commit the open transaction: see [`Document::commit`].
    pub(super) fn commit_open(&mut self) -> Option<Revision> {
        let mut open = self.writer.open.take()?;
        let mut done = Done::default();
        let mut journals = std::mem::take(&mut open.base);
        let mut removed = std::mem::take(&mut open.base_removed);
        if let Some(mut stage) = open.stage.take() {
            if stage.t < 1.0 || !stage.after {
                let snap = stage.snap;
                self.write_stage(&mut stage, 1.0, snap, &mut Done::default());
            }
            journals.push(stage.journal);
            removed.extend(stage.removed);
        }
        let (kept, empty): (Vec<_>, Vec<_>) = journals.into_iter().partition(|journal| !journal.is_empty());
        for journal in empty {
            journal.discard(&mut self.tree);
        }
        if kept.is_empty() {
            return None;
        }
        for journal in &kept {
            done.wrote(journal, false);
        }
        done.removed = removed;
        let revision = self.advance();
        self.writer
            .history
            .push(&mut self.tree, kept, open.source, open.label, revision);
        self.record(open.source, done);
        Some(revision)
    }

    /// Cancel the open transaction: see [`Document::cancel`]. Whether the
    /// cancel took anything back — a transaction that changed nothing
    /// cancels to nothing.
    pub(super) fn cancel_open(&mut self) -> bool {
        let Some(mut open) = self.writer.open.take() else {
            return false;
        };
        let mut done = Done::default();
        if let Some(mut stage) = open.stage.take() {
            self.lift(&mut stage, &mut done);
            stage.journal.discard(&mut self.tree);
        }
        for journal in std::mem::take(&mut open.base).into_iter().rev() {
            done.removed
                .extend(addresses(&self.tree, inserted_by(&journal), Arrangement::Current));
            match journal.revert(&mut self.tree) {
                Ok(()) => {
                    self.account(&journal, Direction::Backward);
                    done.wrote(&journal, true);
                }
                Err(error) => error!("document writer: could not revert a cancelled change: {error}"),
            }
            journal.discard(&mut self.tree);
        }
        let reverted = !done.touched.is_empty() || !done.removed.is_empty();
        self.record(open.source, done);
        reverted
    }

    /// For a save: write the open transaction's stage and base back to
    /// the committed state, newest first, keeping their journals — they
    /// are written forward again by [`Self::restore_open`]. Nothing is
    /// accounted for and nothing recorded: the pair leaves the document
    /// as it found it.
    pub(super) fn lift_open(&mut self) {
        let Some(open) = self.writer.open.as_ref() else {
            return;
        };
        if let Some(stage) = &open.stage {
            if let Err(error) = stage.journal.write_interpolated(&mut self.tree, 0.0, stage.snap) {
                error!("document writer: could not lift the staged change to save: {error}");
            }
        }
        for journal in open.base.iter().rev() {
            if let Err(error) = journal.revert(&mut self.tree) {
                error!("document writer: could not lift a change in progress to save: {error}");
            }
        }
    }

    /// Put back what [`Self::lift_open`] lifted: the base oldest first,
    /// then the stage where it stood.
    pub(super) fn restore_open(&mut self) {
        let Some(open) = self.writer.open.as_ref() else {
            return;
        };
        for journal in &open.base {
            if let Err(error) = journal.replay(&mut self.tree) {
                error!("document writer: could not restore a change in progress after saving: {error}");
            }
        }
        if let Some(stage) = &open.stage {
            if let Err(error) = stage
                .journal
                .write_interpolated(&mut self.tree, stage.t, stage.snap)
            {
                error!("document writer: could not restore the staged change after saving: {error}");
            }
        }
    }

    /// Append `done` to the change feed under `source`, at the current
    /// revision, unless it touched nothing.
    fn record(&mut self, source: ChangeSource, done: Done) {
        self.push_change(source, done);
    }
}

/// The elements `journal` takes out of the tree, written forward.
pub(super) fn removed_by(journal: &Journal<DocElement>) -> impl Iterator<Item = NodeId> + '_ {
    journal
        .touched()
        .filter(|&(_, touch)| touch == Touch::Removed)
        .map(|(id, _)| id)
}

/// The elements `journal` puts into the tree, written forward — the ones
/// a revert takes out.
pub(super) fn inserted_by(journal: &Journal<DocElement>) -> impl Iterator<Item = NodeId> + '_ {
    journal
        .touched()
        .filter(|&(_, touch)| touch == Touch::Inserted)
        .map(|(id, _)| id)
}
