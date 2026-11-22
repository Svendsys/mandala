// SPDX-License-Identifier: MPL-2.0

//! `MindMapDocument` — the app's handle on one open map: the
//! [`Document`] (design §5, V1-B2..B4) and the app state that is *about*
//! the map without being part of it — where it was loaded from, what is
//! selected, the one open transaction, and the last refusal to show.
//!
//! **The document changes only through its writer.** Nothing here holds
//! `&mut` to the document's tree or its elements: a change is an
//! [`EditRequest`] handed to [`MindMapDocument::apply`], an undo or a
//! redo, and each of those is recorded in the document's change feed,
//! which [`crate::application::view`] drains to bring the glyph layers
//! up to date (`MindMapDocument::take_changes`). Selection is not a
//! change of the document and is drawn by the view's overlay.
//!
//! **D28 is reported in one place.** While an interactive transaction
//! is open (C7's drags, C8's editors), the writer refuses any other
//! `apply` or `begin` with [`ApplyError::TransactionOpen`].
//! [`MindMapDocument::apply`] and [`MindMapDocument::begin`] are the
//! only entries the app has to the writer's two refusable calls, and
//! both report a refusal the same way — a `warn!` and a short status
//! line ([`MindMapDocument::status`]) — so every family that edits
//! the map (V1-C4..C8, D1, D3) inherits the same answer to "what does
//! the user see?".

pub mod placard;
pub mod selection;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) mod tests_common;

// Both targets': the IPC module reads it on native over the socket
// and in the browser over its WebSocket (`format/ipc.md` §Transport).
mod ipc_state;

pub(crate) use ipc_state::{DocumentField, PointerMiss};
pub use selection::Selection;

use std::path::Path;

use baumhard::arena::NodeId;
use baumhard::document::edit::Animate;
use baumhard::document::format;
use baumhard::document::format::file::MAP_EXTENSION;
use baumhard::document::ids::Document;
use baumhard::document::presentation::{InstanceId, PresentError, PresentationDef};
use baumhard::document::values::PresentationId;
use baumhard::document::verb::{RunId, Runs};
use baumhard::document::writer::error::ApplyError;
use baumhard::document::writer::feed::ChangeRecord;
use baumhard::document::writer::transaction::{Transaction, TransactionKind};
use baumhard::document::writer::{ChangeSource, EditRequest, Revision};
use log::{info, warn};

/// An animated edit in progress: its own transaction, how it animates,
/// and when it started.
///
/// **It owns its handle.** The shared transaction slot
/// ([`MindMapDocument::transaction`]) holds interactions only — a drag,
/// an editor, a preview — so an interaction that begins while an edit
/// animates (and so, by D28, fast-forwards and commits it) can never be
/// staged into, committed or cancelled by the edit's tick: the tick
/// reaches nothing but this handle, and stops the moment the writer says
/// it is no longer open.
#[derive(Debug)]
struct EditAnimation {
    /// The animated transaction the edit is staged in.
    tx: Transaction,
    /// Its timing.
    animate: Animate,
    /// The app clock's reading when it started.
    started_ms: f64,
}

/// The status line a change refused by an open interactive transaction
/// shows (D28). Short, because it is a status line.
pub const TRANSACTION_OPEN_STATUS: &str = "busy: finish or cancel the edit in progress first";

/// What the one open transaction is for: the interaction that began it,
/// and the only one that may stage into it, commit it or cancel it — so
/// `frame commit` never ends a drag or an editor session that happens to
/// be open. Each interaction that opens a transaction (V1-C7's drags,
/// V1-C8's editors) names itself here.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Interaction {
    /// A frame preview (V1-C5): its stage is the previewed frame fields.
    FramePreview,
    /// An interaction the tests open, which nothing else stages into.
    #[cfg(test)]
    Test,
}

/// The open map. See the [module docs](self).
pub struct MindMapDocument {
    /// The map. Private: it changes only through [`Self::apply`],
    /// [`Self::undo`] and [`Self::redo`] (and V1-C7's transactions).
    doc: Document,
    /// Where the map was loaded from or last saved to; `None` for a map
    /// that has never been on disk, which `save` refuses without a path.
    pub file_path: Option<String>,
    /// What is selected.
    pub selection: Selection,
    /// The animated edit in progress ([`Self::run_edit`]), which holds
    /// its own transaction; the per-frame tick writes it.
    animation: Option<EditAnimation>,
    /// The one open interaction's transaction (D28) and which interaction
    /// it is for, or `None`: interactions only — an animated edit holds
    /// its own ([`EditAnimation`]). Opened by [`Self::begin`], staged into
    /// by [`Self::stage`] and ended by [`Self::commit`] or
    /// [`Self::cancel`], each for the interaction that began it.
    transaction: Option<(Transaction, Interaction)>,
    /// The status line the last refusal or decline left, until the next
    /// change the writer accepts.
    status: Option<&'static str>,
    /// The notice that the map names fonts this machine lacks
    /// (`missing_fonts::notice`, #329): shown when the map is opened, when
    /// the user replaces a family or rescans ([`Self::note_missing_fonts`]),
    /// and whenever a change — an edit, an undo, a redo — changes which
    /// families are missing ([`Self::refresh_font_notice`]); dismissed
    /// with the status line ([`Self::dismiss_status`]), and kept dismissed
    /// until what is missing changes again.
    font_notice: Option<String>,
    /// The families [`Self::font_notice`] was last computed for, by name
    /// and sorted: what tells a change that altered what is missing from
    /// one that did not.
    missing_fonts: Vec<String>,
    /// What Copy and Cut last took from this map (`clipboard`). Kept with
    /// the document because a copied section names its theme and faces.
    pub clipboard: Option<crate::application::clipboard::Clipboard>,
    /// Whether this is [`Self::load_failure_placard`]'s document.
    load_failure: bool,
    /// Which document of the process this is ([`Self::generation`]).
    generation: u64,
    /// The map verbs running on it (design §11; V1-D3), which the
    /// frame resumes (`families::verbs`). They name its nodes, so they
    /// go with it when it is replaced.
    runs: Runs,
    /// The node the pointer rests on whose `on.hover` names a verb, and
    /// the held run that hover invoked, which leaving it releases.
    hover: Option<Hover>,
}

/// A hover held on a node (design §11.2): the node, and the run its
/// `on.hover` verb was invoked as — `None` when the invocation was
/// refused, so that leaving the node has nothing to release.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    /// The node the pointer rests on.
    pub node: NodeId,
    /// The held run.
    pub run: Option<RunId>,
}

/// The generation the next [`MindMapDocument`] gets.
static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl MindMapDocument {
    /// The handle on `doc`, loaded from `file_path`, with nothing
    /// selected and the history held to `history_limit` (D18).
    pub fn new(mut doc: Document, file_path: Option<String>, history_limit: usize) -> Self {
        doc.set_history_limit(history_limit);
        MindMapDocument {
            doc,
            file_path,
            selection: Selection::new(),
            transaction: None,
            animation: None,
            status: None,
            font_notice: None,
            missing_fonts: Vec::new(),
            clipboard: None,
            load_failure: false,
            generation: NEXT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            runs: Runs::default(),
            hover: None,
        }
    }

    /// Read the map at `path` through [`format::file::read`]: refused
    /// unless it ends in [`MAP_EXTENSION`], then read and loaded by
    /// [`format::load_from_bytes`]. The browser has no files (its read
    /// fails and says so) and loads through [`Self::from_json_str`].
    ///
    /// **The error is returned, not reported.** Reporting belongs to the
    /// surface that asked — the console `open` verb prints it, startup
    /// draws it on the canvas through [`Self::load_failure_placard`].
    pub fn load(path: &str, history_limit: usize) -> Result<Self, String> {
        let doc = format::file::read(path)?;
        log_loaded(&doc);
        Ok(Self::new(doc, Some(path.to_string()), history_limit))
    }

    /// Load a map from its text; `file_path` is where it came from (a
    /// path or a URL), or `None`. Reports nothing, as [`Self::load`].
    pub fn from_json_str(
        json: &str,
        file_path: Option<String>,
        history_limit: usize,
    ) -> Result<Self, String> {
        let doc = format::load(json).map_err(|e| e.to_string())?;
        log_loaded(&doc);
        Ok(Self::new(doc, file_path, history_limit))
    }

    /// A map with no nodes, named after `file_path`'s file name (or
    /// "untitled"), bound to `file_path`. What `new` makes.
    pub fn new_blank(file_path: Option<String>, history_limit: usize) -> Self {
        let name = file_path
            .as_deref()
            .and_then(|p| Path::new(p).file_name().and_then(|n| n.to_str()))
            .map(|n| n.strip_suffix(MAP_EXTENSION).unwrap_or(n).to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "untitled".to_string());
        Self::new(
            document_of(serde_json::json!({ "format": 1, "name": name })),
            file_path,
            history_limit,
        )
    }

    /// The map a rejected load is drawn as: one node carrying `message` —
    /// the loader's own words — under the `source` that was asked for, so
    /// a launch that cannot load its map still opens a window that says
    /// why (CODE_CONVENTIONS §9).
    ///
    /// **Bound to no file path, on purpose.** `save` refuses a document
    /// with none, so a reflexive save cannot overwrite the file that
    /// failed to load, which is likely the only copy of what its author
    /// was editing.
    pub fn load_failure_placard(source: &str, message: &str, history_limit: usize) -> Self {
        // The source on a line of its own, unless the message already
        // names it (`placard::message_names_source`).
        let text = if placard::message_names_source(source, message) {
            format!("could not load the map\n{message}")
        } else {
            format!("could not load {source}\n{message}")
        };
        let placard = serde_json::json!({
            "format": 1,
            "name": "load failure",
            "nodes": [{
                "id": 1,
                "position": [0.0, 0.0],
                "sections": [{ "spans": [
                    { "text": text, "size": 18.0, "color": "#ff6b6b" }
                ]}]
            }]
        });
        let mut doc = Self::new(document_of(placard), None, history_limit);
        doc.load_failure = true;
        doc
    }

    /// Whether this document is [`Self::load_failure_placard`]'s.
    // Native-only reader: the one-shot capture; the browser has no
    // command line to ask for one.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn is_load_failure_placard(&self) -> bool {
        self.load_failure
    }

    /// Which document of the process this is: every document built gets
    /// the next number, so a change of generation is a replaced document
    /// — what the IPC event stream reports as `document_replaced`. O(1).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The map, to read. O(1).
    pub fn doc(&self) -> &Document {
        &self.doc
    }

    /// The map's name. O(1).
    pub fn name(&self) -> &str {
        self.doc.name()
    }

    /// Whether the map has changed since it was loaded or last saved. O(1).
    pub fn is_dirty(&self) -> bool {
        self.doc.is_dirty()
    }

    /// The map's revision: advanced by every change, undo and redo. O(1).
    pub fn revision(&self) -> Revision {
        self.doc.revision()
    }

    /// **The app's one entry to `Document::apply`** (§8): apply `req`,
    /// declared by `source`, as one history entry. A refusal is
    /// reported ([`Self::report_refusal`]) and returned; an accepted
    /// change clears the status line.
    pub fn apply(&mut self, req: EditRequest, source: ChangeSource) -> Result<Revision, ApplyError> {
        let label = req.label.clone();
        let from = self.doc.pending_changes().len();
        match self.doc.apply(req, source) {
            Ok(revision) => {
                self.status = None;
                // An animated transaction was finished and committed
                // first (D28): its handle no longer names anything.
                self.forget_closed_transaction();
                self.refresh_font_notice(from);
                Ok(revision)
            }
            Err(error) => {
                self.report_refusal(&label, &error);
                Err(error)
            }
        }
    }

    /// Open a transaction of `kind` for `interaction`, declared by
    /// `source` (§9), and hold its handle: the one an editor, a drag or a
    /// preview stages into. A refusal is reported as [`Self::apply`]'s
    /// is, and `false` returned.
    pub fn begin(&mut self, source: ChangeSource, kind: TransactionKind, interaction: Interaction) -> bool {
        // The writer may finish and commit an animated edit first (D28):
        // the notice follows that commit as it follows any.
        let from = self.doc.pending_changes().len();
        let begun = match self.doc.begin(source, kind) {
            Ok(transaction) => {
                self.transaction = Some((transaction, interaction));
                // An animated edit the writer finished first (D28) no
                // longer names anything.
                self.forget_closed_transaction();
                self.status = None;
                true
            }
            Err(error) => {
                self.report_refusal("begin", &error);
                false
            }
        };
        self.refresh_font_notice(from);
        begun
    }

    /// What the open transaction is for, or `None` when none is open.
    /// O(1).
    pub fn open_interaction(&self) -> Option<Interaction> {
        self.transaction.as_ref().map(|(_, interaction)| *interaction)
    }

    /// Stage `req` in the transaction `interaction` began (§9): it
    /// replaces the previous stage. Refused with
    /// [`ApplyError::TransactionClosed`] when no transaction of
    /// `interaction` is open; a refusal is reported as [`Self::apply`]'s
    /// is.
    pub fn stage(&mut self, interaction: Interaction, req: EditRequest) -> Result<(), ApplyError> {
        let label = req.label.clone();
        let result = match &self.transaction {
            Some((tx, open)) if *open == interaction => self.doc.stage(tx, req),
            _ => Err(ApplyError::TransactionClosed),
        };
        match &result {
            Ok(()) => self.status = None,
            Err(error) => self.report_refusal(&label, error),
        }
        result
    }

    /// Commit the transaction `interaction` began: its stage becomes one
    /// history entry. Returns the new revision, or `None` when it
    /// changed nothing or no transaction of `interaction` is open.
    pub fn commit(&mut self, interaction: Interaction) -> Option<Revision> {
        let tx = self.take_transaction(interaction)?;
        let from = self.doc.pending_changes().len();
        let revision = self.doc.commit(tx);
        self.refresh_font_notice(from);
        revision
    }

    /// Cancel the transaction `interaction` began: the document is as it
    /// was before it. Returns whether one was open.
    pub fn cancel(&mut self, interaction: Interaction) -> bool {
        match self.take_transaction(interaction) {
            Some(tx) => {
                self.doc.cancel(tx);
                true
            }
            None => false,
        }
    }

    /// The handle of the transaction `interaction` began, taken out of
    /// the slot; `None`, and the slot left alone, for any other.
    fn take_transaction(&mut self, interaction: Interaction) -> Option<Transaction> {
        match self.transaction.take() {
            Some((tx, open)) if open == interaction => Some(tx),
            other => {
                self.transaction = other;
                None
            }
        }
    }

    /// The status line a refusal or a decline left, if the writer has
    /// accepted nothing since. O(1).
    pub fn status(&self) -> Option<&'static str> {
        self.status
    }

    /// Dismiss the status line a refusal or a decline left, and the
    /// missing-font notice: what `ExitMode` does. O(1).
    pub fn dismiss_status(&mut self) {
        self.status = None;
        self.font_notice = None;
    }

    /// The notice that the map names fonts this machine lacks, if it is
    /// showing (#329): the status line shows it when no mode and no
    /// refusal has one. O(1).
    pub fn font_notice(&self) -> Option<&str> {
        self.font_notice.as_deref()
    }

    /// Show the notice for `missing`, the families the map names that this
    /// machine lacks, or clear it when there are none: what opening a map,
    /// a replacement and a rescan do (`missing_fonts`), each a moment the
    /// user is told about fonts whatever was dismissed before. O(the
    /// names' length).
    pub fn note_missing_fonts(&mut self, missing: &[baumhard::document::fonts::MissingFamily]) {
        self.missing_fonts = missing.iter().map(|missing| missing.family.to_string()).collect();
        self.font_notice = crate::application::missing_fonts::notice(missing);
    }

    /// Keep the missing-font notice true to the map after a change, read
    /// from the records the change left on the feed past `from` (#329, the
    /// review of #390): an undo of a replacement brings its family back,
    /// and `font family="Nope Sans"` names one. When which families are
    /// missing changed, the notice shows the new set (or clears, when
    /// none is); when it did not, the notice is left as it is, so one the
    /// user dismissed stays dismissed.
    ///
    /// Proportional: [`missing_families`](baumhard::document::fonts::missing_families)
    /// walks the map only when the change wrote an installed family where
    /// it wrote ([`touched_names_installed`](baumhard::document::fonts::touched_names_installed)),
    /// or a family is missing and the change wrote a style or the
    /// defaults, or put an element in the tree or took one out
    /// ([`touched_style`](baumhard::document::fonts::touched_style)) —
    /// the only changes that can add or take away a missing family. A map
    /// that names no installed family pays one read per element touched.
    fn refresh_font_notice(&mut self, from: usize) {
        let pending = self.doc.pending_changes();
        let records = &pending[from.min(pending.len())..];
        if records.is_empty() {
            return;
        }
        let may_add = baumhard::document::fonts::touched_names_installed(&self.doc, records);
        let may_take = !self.missing_fonts.is_empty() && baumhard::document::fonts::touched_style(records);
        if !may_add && !may_take {
            return;
        }
        let missing = baumhard::document::fonts::missing_families(&self.doc);
        let names: Vec<String> = missing.iter().map(|missing| missing.family.to_string()).collect();
        if names != self.missing_fonts {
            self.missing_fonts = names;
            self.font_notice = crate::application::missing_fonts::notice(&missing);
        }
    }

    /// The one report of a refused change (D28): a `warn!` naming the
    /// change and why, and — for [`ApplyError::TransactionOpen`] — the
    /// status line [`TRANSACTION_OPEN_STATUS`], since that refusal is
    /// one the user caused by doing two things at once.
    pub fn report_refusal(&mut self, label: &str, error: &ApplyError) {
        warn!(
            "document: {:?} was refused: {error}",
            baumhard::util::log::excerpt(label)
        );
        if *error == ApplyError::TransactionOpen {
            self.status = Some(TRANSACTION_OPEN_STATUS);
        }
    }

    /// Decline an action that has nothing to ask the writer for — a
    /// connector alone, which cannot be deleted — and say `why` on the
    /// status line, as a refusal does. Logged at `debug!`.
    pub fn decline(&mut self, why: &'static str) {
        log::debug!("document: declined: {why}");
        self.status = Some(why);
    }

    /// Undo the newest history entry. With a transaction open, the
    /// writer settles it first (an interactive one is cancelled and that
    /// is the undo; ruling R36 on #238). Returns the new revision, or
    /// `None` when nothing changed.
    pub fn undo(&mut self) -> Option<Revision> {
        let from = self.doc.pending_changes().len();
        let revision = self.doc.undo();
        self.forget_closed_transaction();
        self.after_history_move();
        self.refresh_font_notice(from);
        revision
    }

    /// Redo the history entry undone last. See [`Self::undo`].
    pub fn redo(&mut self) -> Option<Revision> {
        let from = self.doc.pending_changes().len();
        let revision = self.doc.redo();
        self.forget_closed_transaction();
        self.after_history_move();
        self.refresh_font_notice(from);
        revision
    }

    /// Drain the change feed: every record since the last call, oldest
    /// first. Called once per change by the view's sync, which hands the
    /// records to the projector and the IPC event stream. O(1).
    pub fn take_changes(&mut self) -> Vec<ChangeRecord> {
        self.doc.take_changes()
    }

    /// The most entries the history holds (D18).
    pub fn history_limit(&self) -> usize {
        self.doc.history_limit()
    }

    /// The map as a `.mandala.json`: its committed state.
    pub fn save_json(&mut self) -> Result<String, String> {
        self.doc.save_json().map_err(|e| e.to_string())
    }

    /// Write the map to `path` through [`format::file::write`] — refused
    /// unless it ends in [`MAP_EXTENSION`], then written to a temporary
    /// file beside it and renamed, so a failed write never leaves half a
    /// map where a whole one was; then record the saved state and bind the
    /// document to `path`.
    pub fn save_to(&mut self, path: &str) -> Result<(), String> {
        // The write rule first: a save it refuses serializes nothing.
        format::file::write_target(path)?;
        let text = self.save_json()?;
        format::file::write(path, &text)?;
        self.doc.mark_saved();
        self.file_path = Some(path.to_string());
        info!(
            "save: saved to {}",
            baumhard::util::log::tilde_path(Path::new(path))
        );
        Ok(())
    }

    /// Drop each transaction handle the writer has closed: an
    /// interaction an undo or a redo cancelled, and an animated edit
    /// another change, an undo or a redo finished (D28, R36). Each is
    /// asked of the writer by its own handle.
    fn forget_closed_transaction(&mut self) {
        if self
            .transaction
            .as_ref()
            .is_some_and(|(tx, _)| !self.doc.is_open(tx))
        {
            self.transaction = None;
        }
        if self.animation.as_ref().is_some_and(|a| !self.doc.is_open(&a.tx)) {
            self.animation = None;
        }
    }

    /// Fast-forward the animated edit in progress to its end and commit
    /// it as its one entry: what a change that is about to read the map
    /// does first, so it reads the map the edit leaves, never a frame of
    /// it. Nothing when no edit animates. O(its journal).
    pub fn finish_animation(&mut self) {
        self.forget_closed_transaction();
        if let Some(animation) = self.animation.take() {
            let from = self.doc.pending_changes().len();
            self.doc.commit(animation.tx);
            self.refresh_font_notice(from);
        }
    }

    /// **Run an edit's request** (design §10.1): applied as one history
    /// entry, or — with `animate` — staged as an animated transaction
    /// that [`Self::tick_animation`] writes part-way each frame from
    /// `now_ms` on and commits as one entry at its end. Either way an
    /// edit that changes nothing makes no entry.
    ///
    /// Who yields is the writer's (D28): an animated edit already
    /// running is finished and committed first; an open interactive
    /// transaction refuses this one, reported as [`Self::apply`]
    /// reports a refusal.
    ///
    /// Cost: the request's; animated, its stage and one write at `t`.
    pub fn run_edit(
        &mut self,
        req: EditRequest,
        animate: Option<Animate>,
        source: ChangeSource,
        now_ms: f64,
    ) -> Result<(), ApplyError> {
        let Some(animate) = animate else {
            return self.apply(req, source).map(|_| ());
        };
        let label = req.label.clone();
        let from = self.doc.pending_changes().len();
        let begun = self.doc.begin(source, TransactionKind::Animated);
        // `begin` finished and committed the animation it found, if any:
        // the notice follows that commit as it follows any.
        self.refresh_font_notice(from);
        let tx = match begun {
            Ok(tx) => tx,
            Err(error) => {
                self.report_refusal(&label, &error);
                return Err(error);
            }
        };
        // `begin` finished the animation it found, if any.
        self.forget_closed_transaction();
        let staged = self
            .doc
            .stage(&tx, req)
            .and_then(|()| self.doc.stage_progress(&tx, animate.progress(0.0), animate.snap));
        if let Err(error) = staged {
            self.doc.cancel(tx);
            self.report_refusal(&label, &error);
            return Err(error);
        }
        self.animation = Some(EditAnimation {
            tx,
            animate,
            started_ms: now_ms,
        });
        self.status = None;
        Ok(())
    }

    /// Write the animated edit in progress at `now_ms`: part-way while it
    /// runs, and committed as one history entry once it has run its
    /// course. Returns whether it wrote anything. Nothing runs when no
    /// edit is animating, or the one that was has been finished by
    /// another change. O(its journal).
    pub fn tick_animation(&mut self, now_ms: f64) -> bool {
        self.forget_closed_transaction();
        let Some(animation) = self.animation.as_ref() else {
            return false;
        };
        let elapsed = now_ms - animation.started_ms;
        if animation.animate.is_done(elapsed) {
            self.finish_animation();
            return true;
        }
        let (t, snap) = (animation.animate.progress(elapsed), animation.animate.snap);
        if let Err(error) = self.doc.stage_progress(&animation.tx, t, snap) {
            log::error!("document: the animated edit could not be written at {t}: {error}");
        }
        true
    }

    /// Whether an animated edit is running: what keeps the frame loop
    /// ticking. Asked of the writer by the edit's own handle, so an edit
    /// another change finished is not running. O(1).
    pub fn is_animating(&self) -> bool {
        self.animation.as_ref().is_some_and(|a| self.doc.is_open(&a.tx))
    }

    /// Start the presentation `def` on `nodes` at the app clock's
    /// reading `now_ms`: one instance per node, a restart where it is
    /// already playing (`Document::start_presentation`). Never a change
    /// of the map: no history entry, no revision, no status line. A
    /// refusal starts nothing and is returned for the caller to show.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "what starts and stops a presentation by its definition is the console shell, which is \
                      native; the map's triggers start and stop the map's own by id (`start_defined`), on \
                      both targets"
        )
    )]
    pub fn start_presentation(
        &mut self,
        def: &PresentationDef,
        nodes: &[NodeId],
        now_ms: f64,
    ) -> Result<Vec<InstanceId>, PresentError> {
        self.doc.start_presentation(def, nodes, now_ms)
    }

    /// Stop the presentation `id` on `nodes` at `now_ms`: each instance
    /// there plays its way out (`Document::stop_presentation`). Returns
    /// how many it stopped.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "what starts and stops a presentation by its definition is the console shell, which is \
                      native; the map's triggers start and stop the map's own by id (`start_defined`), on \
                      both targets"
        )
    )]
    pub fn stop_presentation(&mut self, id: &str, nodes: &[NodeId], now_ms: f64) -> usize {
        self.doc.stop_presentation(id, nodes, now_ms)
    }

    /// Start the map's own presentation `id` on `nodes` at `now_ms`, as
    /// [`Self::start_presentation`] starts a definition, read where the
    /// map keeps it (`Document::start_defined`): what a verb's `present`
    /// step starts. `None` when the map defines none at `id`.
    pub fn start_defined(
        &mut self,
        id: PresentationId,
        nodes: &[NodeId],
        now_ms: f64,
    ) -> Option<Result<Vec<InstanceId>, PresentError>> {
        self.doc.start_defined(id, nodes, now_ms)
    }

    /// Stop the map's own presentation `id` on `nodes` at `now_ms`
    /// (`Document::stop_defined`): what a verb's `stop` step stops.
    /// Returns how many instances it stopped.
    pub fn stop_defined(&mut self, id: PresentationId, nodes: &[NodeId], now_ms: f64) -> usize {
        self.doc.stop_defined(id, nodes, now_ms)
    }

    /// Stop the instances `ids` that still play, at `now_ms`
    /// (`Document::stop_instances`): what a held verb run's release
    /// stops. Returns how many it stopped.
    pub fn stop_instances(&mut self, ids: &[InstanceId], now_ms: f64) -> usize {
        self.doc.stop_instances(ids, now_ms)
    }

    /// The verb runs active on the map, taken out for the runner
    /// (`families::verbs`) to drive while it lends the document to the
    /// runs' steps; [`Self::put_runs`] gives them back. Nothing else
    /// holds them in between: a step reaches the map, never the runs.
    pub(in crate::application) fn take_runs(&mut self) -> Runs {
        std::mem::take(&mut self.runs)
    }

    /// Give back the runs [`Self::take_runs`] took, with what the runner
    /// did to them.
    pub(in crate::application) fn put_runs(&mut self, runs: Runs) {
        self.runs = runs;
    }

    /// The verb runs active on the map, to read. O(1).
    pub fn runs(&self) -> &Runs {
        &self.runs
    }

    /// The hover held on a node, if any. O(1).
    pub fn hover(&self) -> Option<Hover> {
        self.hover
    }

    /// Set the hover held on a node: the runner's, when the pointer
    /// enters or leaves one (`families::verbs::hover`).
    pub(in crate::application) fn set_hover(&mut self, hover: Option<Hover>) {
        self.hover = hover;
    }

    /// Bring every presentation instance to `now_ms`
    /// (`Document::tick_presentations`). Returns whether the frame owes
    /// a draw.
    pub fn tick_presentations(&mut self, now_ms: f64) -> bool {
        self.doc.tick_presentations(now_ms)
    }

    /// Whether a presentation instance is on its way in or out: what
    /// keeps the frame loop ticking beside an animated edit. O(the
    /// instances).
    pub fn presentations_moving(&self) -> bool {
        self.doc.presentations_moving()
    }

    /// Set the presentation layer aside (`Document::suspend_presentations`):
    /// until [`Self::resume_presentations`] takes the returned
    /// [`Suspension`] back, the document reads as the map itself; the
    /// pair nests. For the Action funnel's guard
    /// (`dispatch::the_map::OnTheMap`), which hands its context down and
    /// so cannot hold [`Self::suspended`]'s. Anything else holds that
    /// guard: a suspension never resumed leaves the layer out, so one is
    /// a value that must be given back, not a call that can be left
    /// unmatched ([`Suspension`] says what catches one that is not).
    pub(in crate::application) fn suspend_presentations(&mut self) -> Suspension {
        self.doc.suspend_presentations();
        Suspension(())
    }

    /// Put the presentation layer back (`Document::resume_presentations`),
    /// taking back the [`Suspension`] that set it aside.
    pub(in crate::application) fn resume_presentations(&mut self, suspension: Suspension) {
        self.doc.resume_presentations();
        suspension.given_back();
    }

    /// The map with its presentation layer set aside, as a guard: what
    /// builds a change from values it reads — a copy, an edit's filter, a
    /// layout — reads through it, and the layer is back when it is
    /// dropped, however its scope ends.
    pub fn suspended(&mut self) -> MapItself<'_> {
        self.doc.suspend_presentations();
        MapItself { doc: self }
    }

    /// Whether an animation is moving: an animated edit is running, a
    /// presentation instance is on its way in or out, or a verb run waits
    /// on the clock to go on with its steps. What `clock.wait
    /// until=animations_complete` waits out. O(the instances and the
    /// runs).
    pub fn animations_moving(&self) -> bool {
        self.is_animating() || self.presentations_moving() || self.runs.waiting()
    }

    /// Whether a presentation plays on with nobody stopping it: a `loop`
    /// that was not stopped, which keeps the animations from ever coming
    /// to rest. O(the instances).
    pub fn presentation_loops(&self) -> bool {
        self.doc.presentations().any(|instance| instance.unending)
    }

    /// Drop the selected elements the document no longer holds: what a
    /// change that removed them leaves selected. Returns whether any was
    /// dropped. O(the selection).
    pub fn retain_live_selection(&mut self) -> bool {
        self.selection.retain_live(&self.doc)
    }

    /// What an undo or a redo leaves to tidy: selected elements it
    /// removed, and a status line it made stale.
    fn after_history_move(&mut self) {
        self.retain_live_selection();
        self.status = None;
    }

    /// Apply `req` through the writer exactly as [`Self::apply`] does,
    /// for the tests that drive the history without an editing family
    /// (the families build their own requests from the selection).
    #[cfg(test)]
    pub(crate) fn apply_for_test(&mut self, req: EditRequest) -> Result<Revision, ApplyError> {
        self.apply(req, ChangeSource::Action)
    }

    /// One real edit, for the tests that need the document changed and
    /// do not care how: a new root node with one empty section, through
    /// [`Self::apply_for_test`]. Returns the revision it made.
    #[cfg(test)]
    pub(crate) fn create_root_for_test(&mut self) -> Revision {
        let steps =
            baumhard::document::mutation::compose::create_root(&self.doc, glam::Vec2::new(40.0, 40.0))
                .expect("a root node composes");
        self.apply_for_test(EditRequest {
            steps,
            label: "test: create root".to_string(),
        })
        .expect("the root applies")
    }
}

/// That the presentation layer of a [`MindMapDocument`] was set aside
/// ([`MindMapDocument::suspend_presentations`]): only that call makes
/// one, and only [`MindMapDocument::resume_presentations`] takes it
/// back. One is caught when it is not given back:
///
/// - discarding it where it is made is a warning (`#[must_use]`), and
///   so an error under the gate's `-D warnings`;
/// - dropping it any other way — `let _ =`, a `drop`, the end of a
///   scope — logs an `error!` and, in a debug build, fails a
///   `debug_assert!` (unless a panic is already unwinding, where a
///   second one would abort): the layer stays out, which no reader of
///   the map should be left with.
///
/// The one deliberate way to let one go without resuming is
/// [`Self::forget`], for a guard whose document is gone.
#[must_use = "a suspension never given back to `resume_presentations` leaves the presentation layer out"]
pub(in crate::application) struct Suspension(());

impl Suspension {
    /// Given back: the layer is resumed, so dropping this is not a slip.
    fn given_back(self) {
        std::mem::forget(self);
    }

    /// Let this go without resuming, on purpose: the document it set
    /// aside is no longer the one at hand (a guard whose context lost
    /// its document), so there is nothing to give it back to.
    pub(in crate::application) fn forget(self) {
        std::mem::forget(self);
    }
}

impl Drop for Suspension {
    fn drop(&mut self) {
        log::error!(
            "document: a presentation suspension was dropped without being resumed; the layer stays out"
        );
        if !std::thread::panicking() {
            debug_assert!(false, "a Suspension was dropped without `resume_presentations`");
        }
    }
}

/// The open map with its presentation layer set aside
/// ([`MindMapDocument::suspended`]): it derefs to the document, which
/// reads as the map itself, and the layer comes back when this is
/// dropped — at the end of its scope, on an early return, and while a
/// panic unwinds.
pub struct MapItself<'d> {
    doc: &'d mut MindMapDocument,
}

impl std::ops::Deref for MapItself<'_> {
    type Target = MindMapDocument;

    fn deref(&self) -> &MindMapDocument {
        self.doc
    }
}

impl std::ops::DerefMut for MapItself<'_> {
    fn deref_mut(&mut self) -> &mut MindMapDocument {
        self.doc
    }
}

impl Drop for MapItself<'_> {
    fn drop(&mut self) {
        self.doc.doc.resume_presentations();
    }
}

/// A document of `value`, a map the app builds itself and so knows to be
/// valid: a load that refuses it is a bug in that map, logged, and
/// answered with the empty map.
fn document_of(value: serde_json::Value) -> Document {
    match format::load(&value.to_string()) {
        Ok(doc) => doc,
        Err(error) => {
            log::error!("document: a map the app builds was refused: {error}");
            format::load(r#"{"format": 1, "name": "untitled"}"#)
                .expect("the smallest map the format has is a map it loads")
        }
    }
}

/// Announce a map that really did load. `info!`: compiled out of
/// release entirely (CODE_CONVENTIONS §9).
fn log_loaded(doc: &Document) {
    info!("document: loaded map {:?}", doc.name());
}
