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

use baumhard::document::edit::Animate;
use baumhard::document::format::{self, LoadError};
use baumhard::document::ids::Document;
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

/// The one file extension a map is read from and written to (D19).
pub const MAP_EXTENSION: &str = ".mandala.json";

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

/// Whether `path` names a map: it ends in [`MAP_EXTENSION`]. A path that
/// does not is refused by every surface that opens or saves one, with
/// [`extension_refusal`]'s words.
pub fn has_map_extension(path: &str) -> bool {
    path.ends_with(MAP_EXTENSION)
}

/// The message a path without [`MAP_EXTENSION`] is refused with: it names
/// the extension it wanted and, through `excerpt`, the path's tail.
pub fn extension_refusal(path: &str) -> String {
    format!(
        "{}: not a map; a map's file name ends in `{MAP_EXTENSION}`",
        baumhard::util::log::tilde_path(Path::new(path))
    )
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
    /// What Copy and Cut last took from this map (`clipboard`). Kept with
    /// the document because a copied section names its theme and faces.
    pub clipboard: Option<crate::application::clipboard::Clipboard>,
    /// Whether this is [`Self::load_failure_placard`]'s document.
    load_failure: bool,
    /// Which document of the process this is ([`Self::generation`]).
    generation: u64,
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
            clipboard: None,
            load_failure: false,
            generation: NEXT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Read the map at `path`: refused unless it ends in
    /// [`MAP_EXTENSION`], then read and loaded by
    /// [`format::load_from_bytes`]. The browser has no files (its read
    /// fails and says so) and loads through [`Self::from_json_str`].
    ///
    /// **The error is returned, not reported.** Reporting belongs to the
    /// surface that asked — the console `open` verb prints it, startup
    /// draws it on the canvas through [`Self::load_failure_placard`].
    pub fn load(path: &str, history_limit: usize) -> Result<Self, String> {
        if !has_map_extension(path) {
            return Err(extension_refusal(path));
        }
        let bytes = std::fs::read(path).map_err(|e| {
            format!(
                "{}: could not read: {e}",
                baumhard::util::log::tilde_path(Path::new(path))
            )
        })?;
        let doc = format::load_from_bytes(&bytes).map_err(|e| load_message(path, &e))?;
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
        match self.doc.apply(req, source) {
            Ok(revision) => {
                self.status = None;
                // An animated transaction was finished and committed
                // first (D28): its handle no longer names anything.
                self.forget_closed_transaction();
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
        match self.doc.begin(source, kind) {
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
        }
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
        self.doc.commit(tx)
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

    /// Dismiss the status line a refusal or a decline left: what
    /// `ExitMode` does. O(1).
    pub fn dismiss_status(&mut self) {
        self.status = None;
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
        let revision = self.doc.undo();
        self.forget_closed_transaction();
        self.after_history_move();
        revision
    }

    /// Redo the history entry undone last. See [`Self::undo`].
    pub fn redo(&mut self) -> Option<Revision> {
        let revision = self.doc.redo();
        self.forget_closed_transaction();
        self.after_history_move();
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

    /// Write the map to `path` — refused unless it ends in
    /// [`MAP_EXTENSION`] — through a temporary file beside it and a
    /// rename, so a failed write never leaves half a map where a whole one
    /// was; then record the saved state and bind the document to `path`.
    pub fn save_to(&mut self, path: &str) -> Result<(), String> {
        if !has_map_extension(path) {
            return Err(extension_refusal(path));
        }
        let text = self.save_json()?;
        write_atomically(Path::new(path), text.as_bytes())?;
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
            self.doc.commit(animation.tx);
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
        let tx = match self.doc.begin(source, TransactionKind::Animated) {
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

/// The loader's error for `path`, its message prefixed by where it was
/// read from: most of the loader's messages do not name the file.
fn load_message(path: &str, error: &LoadError) -> String {
    format!("{}: {error}", baumhard::util::log::tilde_path(Path::new(path)))
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

/// Write `bytes` to `path` through a temporary file in the same
/// directory and a rename over `path`.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let shown = || baumhard::util::log::tilde_path(path);
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| format!("{}: not a file name", shown()))?;
    let mut temp = dir.join(name);
    temp.as_mut_os_string().push(".saving");
    let written = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    written.map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("{}: could not write: {e}", shown())
    })
}
