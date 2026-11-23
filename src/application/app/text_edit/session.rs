// SPDX-License-Identifier: MPL-2.0

//! One section editor session: a [`Draft`] of the section's spans, its
//! node, and what closing it does. Renderer-free: every step takes the
//! document alone, so the whole lifecycle runs in the suite without a
//! GPU (TEST_CONVENTIONS §T8).

use baumhard::arena::NodeId;
use baumhard::document::edit_buffer::EditBuffer;
use baumhard::document::element::DocKind;
use baumhard::document::mutation::compose::create_root;
use baumhard::document::writer::transaction::TransactionKind;
use baumhard::document::writer::{ChangeSource, EditRequest, Revision};
use glam::Vec2;

use super::draft::{Draft, Step};
use super::{typed_payload, Edit};
use crate::application::document::selection::kind_of;
use crate::application::document::{Interaction, MindMapDocument};
use crate::application::keybinds::Action;
use crate::application::platform::input::Key;

/// Why an editor did not open — one sentence each, the reason a refused
/// dispatch returns. Named so the tests hold the reply to the arm.
pub(in crate::application::app) struct EditRefusal;

impl EditRefusal {
    /// Another interactive transaction is open (D28); the status line
    /// says so too.
    pub(in crate::application::app) const BUSY: &'static str =
        crate::application::document::TRANSACTION_OPEN_STATUS;
    /// The element named is not a section of a node.
    pub(in crate::application::app) const NOT_A_SECTION: &'static str = "the section editor edits a section";
    /// The orphan could not be placed: the writer refused the node.
    pub(in crate::application::app) const NO_ORPHAN: &'static str = "a node could not be created there";
}

/// The label a session's history entry carries.
const EDIT_LABEL: &str = "edit text";

/// The label of the entry a `CreateOrphanNodeAndEdit` session commits.
const CREATE_LABEL: &str = "create node";

/// One section editor session. See the [module docs](super).
#[derive(Debug)]
pub(in crate::application::app) struct TextSession {
    /// The section's text, its caret, its transaction.
    draft: Draft,
    /// The section's node.
    node: NodeId,
    /// Whether closing the editor also leaves NodeEdit: an editor that
    /// `EnterNodeEdit` opened on a node of one section, which has nothing
    /// else to pick, as on `main`.
    exit_to_default_on_close: bool,
    /// Whether the session created its node (`CreateOrphanNodeAndEdit`).
    created: bool,
}

impl TextSession {
    /// Open an editor on `section`: begin its transaction and hold the
    /// section's spans. With `clean`, the editor starts empty — the
    /// section's text cleared in the first stage — as `EditSelectionClean`
    /// does on `main`. The caret is at the end.
    ///
    /// Errors: [`EditRefusal::NOT_A_SECTION`] for anything but a section
    /// of a node; [`EditRefusal::BUSY`] when another interactive
    /// transaction is open (the document reports it, D28).
    pub(in crate::application::app) fn open(
        doc: &mut MindMapDocument,
        section: NodeId,
        clean: bool,
        exit_to_default_on_close: bool,
    ) -> Result<Self, &'static str> {
        let node = section_node(doc, section).ok_or(EditRefusal::NOT_A_SECTION)?;
        let buffer = EditBuffer::from_owner(doc.doc(), section).map_err(|_| EditRefusal::NOT_A_SECTION)?;
        if !doc.begin(
            ChangeSource::Input,
            TransactionKind::Interactive,
            Interaction::TextEdit,
        ) {
            return Err(EditRefusal::BUSY);
        }
        let mut session = TextSession {
            draft: Draft::new(section, Interaction::TextEdit, buffer),
            node,
            exit_to_default_on_close,
            created: false,
        };
        if clean {
            session.draft.clear();
            session.stage(doc);
        }
        Ok(session)
    }

    /// Open `CreateOrphanNodeAndEdit`'s session: begin the transaction,
    /// apply a new root node at `position` into its **base** — where it
    /// keeps its `NodeId` and its ident for the whole session (ruling R7
    /// on #238) — and hold its empty section. The new node is read once,
    /// here, as the map's last root node, the one the insert placed last;
    /// from here on the session holds its ids and never looks for it again.
    ///
    /// Errors: [`EditRefusal::BUSY`] (D28), [`EditRefusal::NO_ORPHAN`]
    /// when the writer refuses the node — and then nothing is left open.
    pub(in crate::application::app) fn open_orphan(
        doc: &mut MindMapDocument,
        position: Vec2,
    ) -> Result<Self, &'static str> {
        let steps = create_root(doc.doc(), position).map_err(|_| EditRefusal::NO_ORPHAN)?;
        if !doc.begin(
            ChangeSource::Input,
            TransactionKind::Interactive,
            Interaction::TextEdit,
        ) {
            return Err(EditRefusal::BUSY);
        }
        let created = doc
            .apply_in(Interaction::TextEdit, EditRequest::new(steps, CREATE_LABEL))
            .ok()
            .and_then(|()| {
                let d = doc.doc();
                let node = d
                    .children(d.root())
                    .filter(|&id| kind_of(d, id) == Some(DocKind::Node))
                    .last()?;
                let section = d
                    .children(node)
                    .find(|&id| kind_of(d, id) == Some(DocKind::Section))?;
                let buffer = EditBuffer::from_owner(d, section).ok()?;
                Some((node, section, buffer))
            });
        let Some((node, section, buffer)) = created else {
            doc.cancel(Interaction::TextEdit);
            return Err(EditRefusal::NO_ORPHAN);
        };
        Ok(TextSession {
            draft: Draft::new(section, Interaction::TextEdit, buffer),
            node,
            exit_to_default_on_close: false,
            created: true,
        })
    }

    /// The section edited. O(1).
    pub(in crate::application::app) fn section(&self) -> NodeId {
        self.draft.owner()
    }

    /// Its node. O(1).
    pub(in crate::application::app) fn node(&self) -> NodeId {
        self.node
    }

    /// The section's text, its caret and its anchor. O(1).
    pub(in crate::application::app) fn draft(&self) -> &Draft {
        &self.draft
    }

    /// Whether closing it also leaves NodeEdit. O(1).
    pub(in crate::application::app) fn exit_to_default_on_close(&self) -> bool {
        self.exit_to_default_on_close
    }

    /// Whether the session created its node. O(1).
    pub(in crate::application::app) fn created(&self) -> bool {
        self.created
    }

    /// Whether its transaction is still the open one (see
    /// [`Draft::is_live`]). O(1).
    pub(in crate::application::app) fn is_live(&self, doc: &MindMapDocument) -> bool {
        self.draft.is_live(doc)
    }

    /// Run a `TextEdit*` cursor, delete or insert-space Action, as on
    /// `main`: the `*Select` moves keep the anchor, every other Action
    /// drops it. Commit and cancel are not this; they end the session.
    /// `Edit::None` for any other Action. O(the text).
    pub(in crate::application::app) fn apply_action(&mut self, action: &Action) -> Edit {
        let (step, select) = match action {
            Action::TextEditCursorLeft => (Step::Left, false),
            Action::TextEditCursorRight => (Step::Right, false),
            Action::TextEditCursorUp => (Step::Up, false),
            Action::TextEditCursorDown => (Step::Down, false),
            Action::TextEditCursorHome => (Step::Home, false),
            Action::TextEditCursorEnd => (Step::End, false),
            Action::TextEditCursorLeftSelect => (Step::Left, true),
            Action::TextEditCursorRightSelect => (Step::Right, true),
            Action::TextEditCursorUpSelect => (Step::Up, true),
            Action::TextEditCursorDownSelect => (Step::Down, true),
            Action::TextEditCursorHomeSelect => (Step::Home, true),
            Action::TextEditCursorEndSelect => (Step::End, true),
            Action::TextEditWordLeft => (Step::WordLeft, false),
            Action::TextEditWordRight => (Step::WordRight, false),
            Action::TextEditDeleteBack => (Step::DeleteBack, false),
            Action::TextEditDeleteForward => (Step::DeleteForward, false),
            Action::TextEditDeleteWordBack => (Step::DeleteWordBack, false),
            Action::TextEditDeleteWordForward => (Step::DeleteWordForward, false),
            Action::TextEditInsertSpace => (Step::Space, false),
            _ => return Edit::None,
        };
        self.draft.step(step, select)
    }

    /// A key no `TextEdit*` binding claims: a literal. Enter is a line
    /// break and Tab a tab — this is a paragraph editor, not an outliner,
    /// as on `main` — and a `Key::Character` payload its printable
    /// characters, inserted as one counted splice ([`typed_payload`]).
    /// Any insertion drops the anchor. O(the text).
    pub(in crate::application::app) fn type_key(
        &mut self,
        key_name: Option<&str>,
        logical_key: &Key,
    ) -> Edit {
        match key_name {
            Some("enter") => self.draft.insert("\n"),
            Some("tab") => self.draft.insert("\t"),
            _ => typed_payload(logical_key).map_or(Edit::None, |text| self.draft.insert(&text)),
        }
    }

    /// Stage the buffer (see [`Draft::stage`]). Returns whether the stage
    /// was accepted.
    pub(in crate::application::app) fn stage(&mut self, doc: &mut MindMapDocument) -> bool {
        let label = if self.created { CREATE_LABEL } else { EDIT_LABEL };
        self.draft.stage(doc, label)
    }

    /// End the session, writing it: its stage — and for an orphan, the
    /// node — becomes one history entry. A session that changed nothing
    /// cancels instead, so it leaves no dead entry, as `main` skipped one;
    /// an orphan always commits, its node being the change. A live
    /// shift-select becomes the selection — the swept range of the
    /// section, which the per-section verbs act on — and an orphan's node
    /// is selected. Nothing for a session the writer already ended.
    pub(in crate::application::app) fn commit(self, doc: &mut MindMapDocument) -> Option<Revision> {
        if !self.is_live(doc) {
            return None;
        }
        let revision = if !self.created && self.draft.unchanged() {
            doc.cancel(Interaction::TextEdit);
            None
        } else {
            doc.commit(Interaction::TextEdit)
        };
        let caret = self.draft.caret();
        match self.draft.anchor().filter(|&anchor| anchor != caret) {
            Some(anchor) => doc.selection.select_range(self.section(), anchor, caret),
            None if self.created => {
                doc.selection.select_only(self.node);
            }
            None => {}
        }
        revision
    }

    /// End the session without writing it: the document is exactly as
    /// before it opened — an orphan's node gone with its text.
    pub(in crate::application::app) fn cancel(self, doc: &mut MindMapDocument) {
        doc.cancel(Interaction::TextEdit);
    }
}

/// The node `section` belongs to, when it is a section of a node.
fn section_node(doc: &MindMapDocument, section: NodeId) -> Option<NodeId> {
    let d = doc.doc();
    if kind_of(d, section) != Some(DocKind::Section) {
        return None;
    }
    d.parent(section)
        .filter(|&node| kind_of(d, node) == Some(DocKind::Node))
}
