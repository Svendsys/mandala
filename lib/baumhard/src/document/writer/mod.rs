// SPDX-License-Identifier: MPL-2.0

//! The writer: **the only way a document changes after it is loaded**
//! (design §8, §9; V1-B4, #238).
//!
//! Every change — an action, a console line, an IPC command, a drag, a
//! keystroke in an editor, an edit, a verb — is an
//! [`EditRequest`](crate::document::writer::EditRequest): the ordered
//! [steps](crate::document::mutation::step::DocStep) of one change and a
//! label. [`Document::apply`](crate::document::ids::Document::apply)
//! runs a request as one journal — all or nothing — and makes it one
//! history entry; [a transaction](crate::document::writer::transaction)
//! stages requests live and makes them one entry when it ends.
//! [`Document::undo`](crate::document::ids::Document::undo) reverts an
//! entry's journals and [`Document::redo`](crate::document::ids::Document::redo)
//! replays them: nothing is evaluated again, so an undo cannot disagree
//! with what the edit did, and there is no undo representation but the
//! journals.
//!
//! Beside the history, the writer keeps:
//!
//! - **the revision**, which every apply, commit, undo and redo
//!   advances, and **the saved position**, which
//!   [`Document::is_dirty`](crate::document::ids::Document::is_dirty)
//!   compares against — by identity, so undoing back to the saved state
//!   is clean again;
//! - **the [change feed](crate::document::writer::feed)**, one record per
//!   change, drained by whoever projects the document or reports on it;
//! - the document's derived state (`upkeep`): the ident index, the
//!   incoming links, the budget — moved by every journal it writes.
//!
//! Nothing else in the crate holds `&mut` to a document's tree or its
//! elements: `Document`'s fields are private to `document`, and
//! `util::document_surface` holds every `pub` item under it to returning
//! no `&mut` into them (§8.4).

/// Why the writer refused a change.
pub mod error;
/// The change feed's records.
pub mod feed;
/// Transactions: live interaction as one history entry.
pub mod transaction;

/// What a document costs on wasm32's two axes.
pub(crate) mod budget;
mod history;
mod request;
mod upkeep;

pub use history::HistoryInfo;
pub(crate) use upkeep::Derived;

use crate::arena::NodeId;
use crate::document::format::{self, Limits, SaveError};
use crate::document::ids::Document;
use crate::document::mutation::step::DocStep;
use crate::document::writer::error::ApplyError;
use crate::document::writer::feed::{addresses, reversed, Arrangement, ChangeRecord, RemovedAddress};
use crate::document::writer::history::History;
use crate::document::writer::transaction::{inserted_by, removed_by, Open, TransactionKind};
use crate::document::writer::upkeep::Direction;
use crate::engine::journal::Touch;
use log::error;

/// How many entries the history holds unless the app sets another limit
/// (D18). The user's `settings.json` sets it in the app (V1-C3).
pub const DEFAULT_HISTORY_LIMIT: usize = 50;

/// One change to make: its steps, and the name the history shows it by.
#[derive(Debug)]
pub struct EditRequest {
    /// The steps, run in order on one journal: each built from the
    /// document as the steps before it left it (ruling R8 on #238). A
    /// composite builder in [`compose`](crate::document::mutation::compose)
    /// returns them.
    pub steps: Vec<DocStep>,
    /// What the change is called: shown by `query.undo` and the console.
    pub label: String,
}

impl EditRequest {
    /// The request of `steps`, called `label`. No allocation beyond the
    /// label.
    pub fn new(steps: Vec<DocStep>, label: impl Into<String>) -> Self {
        EditRequest {
            steps,
            label: label.into(),
        }
    }
}

/// Who made a change. Every change declares one; there is no fallback.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ChangeSource {
    /// An `Action` dispatched by a key, a gesture or a menu.
    Action,
    /// A console line.
    Console,
    /// An IPC command.
    Ipc,
    /// Direct input: a drag, a resize, a keystroke in an editor.
    Input,
    /// An edit the map, the user or the app defines (§10.1).
    Edit,
    /// A step of a map verb (§11.1).
    Verb,
    /// An undo — the history's own; a change cannot declare it.
    Undo,
    /// A redo — the history's own; a change cannot declare it.
    Redo,
    /// The presentation layer (§10.2), whose changes never enter the
    /// history or advance the revision; a change cannot declare it.
    Presentation,
}

impl ChangeSource {
    /// The source's name, as IPC reports it. O(1).
    pub fn name(self) -> &'static str {
        match self {
            ChangeSource::Action => "action",
            ChangeSource::Console => "console",
            ChangeSource::Ipc => "ipc",
            ChangeSource::Input => "input",
            ChangeSource::Edit => "edit",
            ChangeSource::Verb => "verb",
            ChangeSource::Undo => "undo",
            ChangeSource::Redo => "redo",
            ChangeSource::Presentation => "presentation",
        }
    }

    /// Refuse a source a change cannot declare. O(1).
    fn check(source: ChangeSource) -> Result<(), ApplyError> {
        match source {
            ChangeSource::Undo | ChangeSource::Redo | ChangeSource::Presentation => {
                Err(ApplyError::Source(source))
            }
            _ => Ok(()),
        }
    }
}

/// A document revision: advanced by every apply, commit, undo and redo,
/// never by a stage or by the presentation layer.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(pub u64);

/// What [`Document::settle_open`] did.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Settled {
    /// Nothing was open, or an animation that changed nothing.
    Nothing,
    /// An interactive transaction was cancelled, and took something back.
    Cancelled,
    /// An interactive transaction that had changed nothing was cancelled.
    CancelledNothing,
    /// An animated transaction was committed as an entry.
    Committed,
}

/// The writer's state beside the tree.
#[derive(Clone, Debug)]
pub(crate) struct Writer {
    history: History,
    revision: u64,
    feed: Vec<ChangeRecord>,
    open: Option<Open>,
    /// The bounds a change is held to: this target's
    /// ([`format::MAX_ELEMENTS`], [`format::MAX_MAP_BYTES`]).
    limits: Limits,
}

impl Default for Writer {
    fn default() -> Self {
        Writer {
            history: History::new(DEFAULT_HISTORY_LIMIT),
            revision: 0,
            feed: Vec::new(),
            open: None,
            limits: format::LIMITS,
        }
    }
}

impl Document {
    /// **The one place a change to the document runs.** Every call that
    /// changes the tree — [`Self::apply`], [`Self::undo`],
    /// [`Self::redo`], [`Self::begin`] (which may commit an animation),
    /// [`Self::stage`], [`Self::apply_in`], [`Self::interpolate`],
    /// [`Self::commit`], [`Self::cancel`], and [`Self::save_json`]'s
    /// bracket around an open transaction — runs inside it, and none
    /// runs inside another.
    ///
    /// It is the hook V1-D2's presentation layer goes in: revert the
    /// active presentations, newest first, run `f`, then apply each again
    /// at its current progress — so a history journal holds only the
    /// change, and a presentation always sits on the map as it now
    /// stands. Until then it runs `f`.
    fn mutate<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        f(self)
    }

    /// Apply `req`, declared by `source`, as one history entry, and
    /// return the new revision (§2 of #238).
    ///
    /// The request's steps run on one journal; then the writer gives
    /// every node it inserted the next ident, removes every link left
    /// leading into what it removed, and judges what it did — the shape
    /// of every row it changed, every link it inserted, this target's
    /// budget. A refused request leaves nothing of itself applied, and the
    /// document as it was but for an animated transaction that was open:
    /// D28 finishes and commits that first, and it stays committed. A
    /// request that changes nothing returns the current revision and
    /// makes no entry.
    ///
    /// Errors: [`ApplyError::TransactionOpen`] while an interactive
    /// transaction is open (an animated one is fast-forwarded and
    /// committed first, D28); [`ApplyError::Source`];
    /// [`ApplyError::Invalid`]; [`ApplyError::IdentSpaceExhausted`];
    /// [`ApplyError::TooManyElements`], [`ApplyError::TooMuchText`].
    ///
    /// Cost: the steps', plus O(the journal) for the writer's own steps,
    /// the checks and the history — never a scan of the document. The
    /// redo branch it clears and the entry it pushes past the limit are
    /// discarded.
    pub fn apply(&mut self, req: EditRequest, source: ChangeSource) -> Result<Revision, ApplyError> {
        self.mutate(|doc| {
            ChangeSource::check(source)?;
            doc.yield_open()?;
            let EditRequest { steps, label } = req;
            let journal = doc.run_request(steps)?;
            if journal.is_empty() {
                journal.discard(&mut doc.tree);
                return Ok(doc.revision());
            }
            let removed = addresses(&doc.tree, removed_by(&journal), Arrangement::Before(&journal));
            doc.account(&journal, Direction::Forward);
            let touched = journal.touched().collect();
            let revision = doc.advance();
            doc.writer
                .history
                .push(&mut doc.tree, vec![journal], source, label, revision);
            doc.push_change(source, touched, removed);
            Ok(revision)
        })
    }

    /// Undo the newest entry: revert its journals, newest first. Returns
    /// the new revision, or `None` when nothing changed.
    ///
    /// With a transaction open, one undo is one visible step (ruling R36
    /// on #238, R37): an interactive transaction is cancelled and the undo
    /// stops there, returning the current revision when the cancel took
    /// something back and `None` when the transaction had changed nothing;
    /// an animated one
    /// yields as D28 has it — fast-forwarded and committed — and the undo
    /// then reverts that entry, which a redo brings back.
    ///
    /// Cost: a revert of each of the entry's journals, and O(its
    /// touches) for the derived state and the record.
    pub fn undo(&mut self) -> Option<Revision> {
        self.mutate(|doc| {
            match doc.settle_open() {
                Settled::Cancelled => return Some(doc.revision()),
                Settled::CancelledNothing => return None,
                Settled::Nothing | Settled::Committed => {}
            }
            let mut entry = doc.writer.history.take_applied()?;
            let mut touched = Vec::new();
            let mut removed = Vec::new();
            for (i, journal) in entry.journals.iter().enumerate().rev() {
                let leaving = addresses(&doc.tree, inserted_by(journal), Arrangement::Current);
                if let Err(error) = journal.revert(&mut doc.tree) {
                    error!("document writer: could not undo {:?}: {error}", entry.label);
                    // Put back what was undone, so the entry stays whole.
                    for undone in &entry.journals[i + 1..] {
                        if let Err(error) = undone.replay(&mut doc.tree) {
                            error!(
                                "document writer: could not restore an entry after a failed undo: {error}"
                            );
                        }
                        doc.account(undone, Direction::Forward);
                    }
                    doc.writer.history.restore_applied(entry);
                    return None;
                }
                doc.account(journal, Direction::Backward);
                touched.extend(journal.touched().map(|(id, touch)| (id, reversed(touch))));
                removed.extend(leaving);
            }
            let revision = doc.advance();
            entry.revision = revision;
            doc.writer.history.push_undone(entry);
            doc.push_change(ChangeSource::Undo, touched, removed);
            Some(revision)
        })
    }

    /// Redo the entry undone last: replay its journals, oldest first.
    /// Returns the new revision, or `None` when nothing changed.
    ///
    /// With a transaction open, as [`Self::undo`]: an interactive one is
    /// cancelled and the redo stops there; an animated one is
    /// fast-forwarded and committed, which clears the redo branch, so the
    /// redo then has nothing left to replay and returns the revision the
    /// commit made.
    ///
    /// Cost: [`Self::undo`]'s.
    pub fn redo(&mut self) -> Option<Revision> {
        self.mutate(|doc| {
            let settled = doc.settle_open();
            match settled {
                Settled::Cancelled => return Some(doc.revision()),
                Settled::CancelledNothing => return None,
                Settled::Nothing | Settled::Committed => {}
            }
            let Some(mut entry) = doc.writer.history.take_undone() else {
                return (settled == Settled::Committed).then(|| doc.revision());
            };
            let mut touched = Vec::new();
            let mut removed = Vec::new();
            for (i, journal) in entry.journals.iter().enumerate() {
                let leaving = addresses(&doc.tree, removed_by(journal), Arrangement::Current);
                if let Err(error) = journal.replay(&mut doc.tree) {
                    error!("document writer: could not redo {:?}: {error}", entry.label);
                    for redone in entry.journals[..i].iter().rev() {
                        if let Err(error) = redone.revert(&mut doc.tree) {
                            error!(
                                "document writer: could not restore an entry after a failed redo: {error}"
                            );
                        }
                        doc.account(redone, Direction::Backward);
                    }
                    doc.writer.history.push_undone(entry);
                    return None;
                }
                doc.account(journal, Direction::Forward);
                touched.extend(journal.touched());
                removed.extend(leaving);
            }
            let revision = doc.advance();
            entry.revision = revision;
            doc.writer.history.push_redone(entry);
            doc.push_change(ChangeSource::Redo, touched, removed);
            Some(revision)
        })
    }

    /// The current revision. O(1).
    pub fn revision(&self) -> Revision {
        Revision(self.writer.revision)
    }

    /// Whether the document has changed since [`Self::mark_saved`]: its
    /// committed state is not the one that was saved. Undoing back to the
    /// saved state is clean; a different change that leaves as many
    /// entries is not. A change in progress (an open transaction) is not
    /// committed, and does not count. O(1).
    pub fn is_dirty(&self) -> bool {
        self.writer.history.is_dirty()
    }

    /// Record the committed state as saved: what the app calls once it
    /// has written [`Self::save_json`]'s text. O(1).
    pub fn mark_saved(&mut self) {
        self.writer.history.mark_saved();
    }

    /// The document as a `.mandala.json` ([`format::save`]): its committed
    /// state. With a transaction open, its stage and its base calls are
    /// written back out of the tree first and put back after — the
    /// journals kept, nothing recorded — so what is saved never holds a
    /// change in progress.
    ///
    /// Errors: whatever [`format::save`] refuses ([`SaveError`]).
    ///
    /// Cost: [`format::save`]'s, plus a revert and a replay of each of
    /// the open transaction's journals.
    pub fn save_json(&mut self) -> Result<String, SaveError> {
        self.mutate(|doc| {
            doc.lift_open();
            let saved = format::save(doc);
            doc.restore_open();
            saved
        })
    }

    /// Hold at most `limit` entries, dropping the oldest — then, with
    /// none applied left, the undone ones furthest from a redo — and
    /// discarding their journals. O(1) plus the discards.
    pub fn set_history_limit(&mut self, limit: usize) {
        self.writer.history.set_limit(&mut self.tree, limit);
    }

    /// The most entries the history holds. O(1).
    pub fn history_limit(&self) -> usize {
        self.writer.history.limit()
    }

    /// The applied entries, oldest first: label, source, revision. No
    /// allocation.
    pub fn history(&self) -> impl Iterator<Item = HistoryInfo<'_>> + '_ {
        self.writer.history.applied()
    }

    /// How many entries a redo can reach. O(1).
    pub fn redo_len(&self) -> usize {
        self.writer.history.redo_len()
    }

    /// Drain the change feed: every record since the last call, oldest
    /// first. O(1); the records are moved out.
    pub fn take_changes(&mut self) -> Vec<ChangeRecord> {
        std::mem::take(&mut self.writer.feed)
    }

    /// The links that lead to `node`, as the writer's index holds them,
    /// in no particular order: for the tests that hold the index to a
    /// count made from scratch. O(1).
    pub(crate) fn incoming_links(&self, node: NodeId) -> &[NodeId] {
        self.derived
            .incoming
            .get(&node)
            .map_or(&[], |links| links.as_slice())
    }

    /// What the document derives beside its ident index, as the writer
    /// has kept it: for the same tests. O(1).
    pub(crate) fn derived(&self) -> &Derived {
        &self.derived
    }

    /// Hold changes to `limits` in place of this target's: for the tests
    /// of the wasm32 bounds, on every target. O(1).
    pub(crate) fn set_limits(&mut self, limits: Limits) {
        self.writer.limits = limits;
    }

    /// What an undo or a redo does first with the open transaction
    /// (ruling R36 on #238): cancel an interactive one, fast-forward and
    /// commit an animated one.
    fn settle_open(&mut self) -> Settled {
        match self.writer.open.as_ref().map(|open| open.kind()) {
            None => Settled::Nothing,
            Some(TransactionKind::Interactive) => {
                if self.cancel_open() {
                    Settled::Cancelled
                } else {
                    Settled::CancelledNothing
                }
            }
            Some(TransactionKind::Animated) => match self.commit_open() {
                Some(_) => Settled::Committed,
                None => Settled::Nothing,
            },
        }
    }

    /// Advance the revision and return it. O(1).
    fn advance(&mut self) -> Revision {
        self.writer.revision += 1;
        Revision(self.writer.revision)
    }

    /// Append a record at the current revision, unless it touched
    /// nothing. O(1).
    fn push_change(
        &mut self,
        source: ChangeSource,
        touched: Vec<(NodeId, Touch)>,
        removed: Vec<(NodeId, RemovedAddress)>,
    ) {
        if touched.is_empty() && removed.is_empty() {
            return;
        }
        self.writer.feed.push(ChangeRecord {
            source,
            revision: self.revision(),
            touched,
            removed,
        });
    }
}
