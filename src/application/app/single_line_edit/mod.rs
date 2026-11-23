// SPDX-License-Identifier: MPL-2.0

//! **The single-line editor** (V1-C8, #249; design §9): one line of a
//! text owner's spans, edited as a transaction on the document's writer.
//! Native-only, as on `main` (CLAUDE.md "Single-line editor").
//!
//! Its owners are the text-bearing elements that are not sections
//! ([`SingleLineTarget`]): a **label** — on a connector, a line link or a
//! portal end — a **portal end**'s marker spans, a frame's **ring** (its
//! poem), a **connector**'s or a **line link**'s content spans, and a
//! **cap** at either end of a path. It is the section editor's scheme
//! ([`super::text_edit`]) on one line: a [`Draft`] of the owner's spans,
//! each change one `stage(replace_spans(owner, buffer.spans()))`, commit
//! one history entry, cancel the document exactly as it was.
//!
//! **A label it opens on that does not exist yet** — `LabelEditOnSelection`
//! or `path label edit` on a path with no label (ruling K3 on #249) — is
//! inserted into the transaction's base when the editor opens, where it
//! keeps its `NodeId` while the text in it is staged; committed empty, a
//! label this session created is canceled away, and a label that was
//! there before is removed, as `main` dropped an emptied label.
//!
//! **Keys** are `main`'s: a key bound in `InputContext::LabelEdit` runs
//! its `LabelEdit*` Action; any other key's printable payload is typed
//! (`text_edit::typed_payload`, `main`'s IME primitive). Enter commits and
//! Escape cancels (`LabelEditCommit`, `LabelEditCancel`), through the
//! funnel. The keybinds spelling keeps `label_edit_*` for every owner, as
//! `main` kept it for both of its two.

#![cfg(not(target_arch = "wasm32"))]

#[cfg(test)]
mod tests;

use baumhard::arena::NodeId;
use baumhard::document::edit_buffer::EditBuffer;
use baumhard::document::element::{DocElement, DocKind, LabelData};
use baumhard::document::mutation::compose::{insert, remove};
use baumhard::document::writer::transaction::TransactionKind;
use baumhard::document::writer::{ChangeSource, EditRequest, Revision};
use baumhard::engine::structure::Subtree;

use super::text_edit::draft::Step;
use super::text_edit::{typed_payload, Draft, Edit};
use crate::application::document::selection::kind_of;
use crate::application::document::{Interaction, MindMapDocument};
use crate::application::keybinds::{Action, InputContext, ResolvedKeybinds};
use crate::application::platform::input::{Key, Modifiers};

/// What the single-line editor edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum SingleLineTarget {
    /// An element that holds spans and is not a section: a label, a
    /// portal end, a ring, a connector, a line link or a cap.
    Owner(NodeId),
    /// The label of `host` — a connector, a line link or a portal end —
    /// which has none yet: inserted when the editor opens.
    NewLabel {
        /// The element the label is to be on.
        host: NodeId,
    },
}

/// Why the single-line editor did not open — one sentence each, the
/// reason a refused dispatch returns.
pub(in crate::application::app) struct LineRefusal;

impl LineRefusal {
    /// Another interactive transaction is open (D28).
    pub(in crate::application::app) const BUSY: &'static str =
        crate::application::document::TRANSACTION_OPEN_STATUS;
    /// The element is not one the single-line editor edits.
    pub(in crate::application::app) const NOT_A_LINE: &'static str =
        "the single-line editor edits a label, a ring, a cap, a portal end or a path's content";
    /// `LabelEditOnSelection` on a portal link, whose labels are its
    /// ends' (ruling K4 on #249).
    pub(in crate::application::app) const PORTAL_LABEL: &'static str =
        "a portal link's labels are its ends': double-click the label beside an end to edit it";
    /// The writer refused the label the editor was to open on.
    pub(in crate::application::app) const NO_LABEL: &'static str = "a label could not be added there";
}

impl SingleLineTarget {
    /// What the single-line editor edits for a press on `element`: the
    /// element itself, when it holds spans and is not a section. `None`
    /// for a section, a node, a frame, a portal link and anything else.
    /// O(1).
    pub(in crate::application::app) fn of_element(
        doc: &baumhard::document::ids::Document,
        element: NodeId,
    ) -> Option<Self> {
        match kind_of(doc, element)? {
            DocKind::Ring | DocKind::CapStart | DocKind::CapEnd | DocKind::Label | DocKind::PortalEnd => {
                Some(SingleLineTarget::Owner(element))
            }
            DocKind::Connector => Some(SingleLineTarget::Owner(element)),
            DocKind::Link if is_line_link(doc, element) => Some(SingleLineTarget::Owner(element)),
            _ => None,
        }
    }

    /// The label of `host` — a connector, a line link or a portal end —
    /// as the editor opens on it: the label it has, or a new one. Refused
    /// for a portal link, whose labels are its ends' (ruling K4), and for
    /// anything that holds no label. O(the host's children).
    pub(in crate::application::app) fn label_of(
        doc: &baumhard::document::ids::Document,
        host: NodeId,
    ) -> Result<Self, &'static str> {
        match kind_of(doc, host) {
            Some(DocKind::Link) if !is_line_link(doc, host) => return Err(LineRefusal::PORTAL_LABEL),
            Some(DocKind::Connector | DocKind::Link | DocKind::PortalEnd) => {}
            _ => return Err(LineRefusal::NOT_A_LINE),
        }
        Ok(match label_child(doc, host) {
            Some(label) => SingleLineTarget::Owner(label),
            None => SingleLineTarget::NewLabel { host },
        })
    }
}

/// Whether `id` is a line link. O(1).
fn is_line_link(doc: &baumhard::document::ids::Document, id: NodeId) -> bool {
    matches!(
        doc.get(id),
        Some(DocElement::Link(link)) if link.kind == baumhard::document::values::LinkKind::Line
    )
}

/// The label `host` holds, if it holds one. O(its children).
fn label_child(doc: &baumhard::document::ids::Document, host: NodeId) -> Option<NodeId> {
    doc.children(host)
        .find(|&child| kind_of(doc, child) == Some(DocKind::Label))
}

/// The single-line editor: closed, or one session.
#[derive(Debug, Default)]
pub(in crate::application::app) enum SingleLineEditor {
    /// No editor is open.
    #[default]
    Closed,
    /// One session.
    Open(LineSession),
}

impl SingleLineEditor {
    /// Whether an editor is open. O(1).
    pub(in crate::application::app) fn is_open(&self) -> bool {
        matches!(self, SingleLineEditor::Open(_))
    }

    /// The open session. O(1).
    pub(in crate::application::app) fn session(&self) -> Option<&LineSession> {
        match self {
            SingleLineEditor::Open(session) => Some(session),
            SingleLineEditor::Closed => None,
        }
    }

    /// The open session, mutably. O(1).
    pub(in crate::application::app) fn session_mut(&mut self) -> Option<&mut LineSession> {
        match self {
            SingleLineEditor::Open(session) => Some(session),
            SingleLineEditor::Closed => None,
        }
    }

    /// Close the editor and hand back its session. O(1).
    pub(in crate::application::app) fn take(&mut self) -> Option<LineSession> {
        match std::mem::take(self) {
            SingleLineEditor::Open(session) => Some(session),
            SingleLineEditor::Closed => None,
        }
    }
}

/// The label of an entry a single-line session commits.
const LABEL: &str = "edit text";

/// One single-line editor session. See the [module docs](self).
#[derive(Debug)]
pub(in crate::application::app) struct LineSession {
    /// The owner's text, its caret, its transaction.
    draft: Draft,
    /// Whether the owner is a label: committed empty, it is removed.
    is_label: bool,
    /// Whether the session inserted the label it edits.
    created_label: bool,
}

impl LineSession {
    /// Open the editor on `target`: begin its transaction (and for a new
    /// label, insert it into the transaction's base) and hold the owner's
    /// spans, the caret at the end. With `clean`, the editor starts empty,
    /// as `EditSelectionClean` does.
    ///
    /// Errors: [`LineRefusal::NOT_A_LINE`] for an owner the editor does
    /// not edit, [`LineRefusal::BUSY`] (D28), [`LineRefusal::NO_LABEL`]
    /// when the writer refuses the new label — and then nothing is left
    /// open.
    pub(in crate::application::app) fn open(
        doc: &mut MindMapDocument,
        target: SingleLineTarget,
        clean: bool,
    ) -> Result<Self, &'static str> {
        let insert_label = match target {
            SingleLineTarget::Owner(owner) => {
                SingleLineTarget::of_element(doc.doc(), owner).ok_or(LineRefusal::NOT_A_LINE)?;
                None
            }
            SingleLineTarget::NewLabel { host } => {
                let kind = kind_of(doc.doc(), host).ok_or(LineRefusal::NOT_A_LINE)?;
                let steps = insert(
                    doc.doc(),
                    kind,
                    &[host],
                    Subtree::leaf(DocElement::Label(LabelData::default())),
                )
                .map_err(|_| LineRefusal::NOT_A_LINE)?;
                Some((host, steps))
            }
        };
        if !doc.begin(
            ChangeSource::Input,
            TransactionKind::Interactive,
            Interaction::SingleLineEdit,
        ) {
            return Err(LineRefusal::BUSY);
        }
        let owner = match (target, insert_label) {
            (SingleLineTarget::Owner(owner), _) => Some(owner),
            (_, Some((host, steps))) => doc
                .apply_in(Interaction::SingleLineEdit, EditRequest::new(steps, LABEL))
                .ok()
                .and_then(|()| label_child(doc.doc(), host)),
            (SingleLineTarget::NewLabel { .. }, None) => None,
        };
        let opened = owner.and_then(|owner| {
            EditBuffer::from_owner(doc.doc(), owner)
                .ok()
                .map(|buffer| (owner, buffer))
        });
        let Some((owner, buffer)) = opened else {
            doc.cancel(Interaction::SingleLineEdit);
            return Err(LineRefusal::NO_LABEL);
        };
        let mut session = LineSession {
            draft: Draft::new(owner, Interaction::SingleLineEdit, buffer),
            is_label: kind_of(doc.doc(), owner) == Some(DocKind::Label),
            created_label: matches!(target, SingleLineTarget::NewLabel { .. }),
        };
        if clean {
            session.draft.clear();
            session.draft.stage(doc, LABEL);
        }
        Ok(session)
    }

    /// The element edited. O(1).
    pub(in crate::application::app) fn owner(&self) -> NodeId {
        self.draft.owner()
    }

    /// The owner's text, its caret. O(1).
    pub(in crate::application::app) fn draft(&self) -> &Draft {
        &self.draft
    }

    /// Whether the session inserted the label it edits. O(1).
    pub(in crate::application::app) fn created_label(&self) -> bool {
        self.created_label
    }

    /// Whether its transaction is still the open one. O(1).
    pub(in crate::application::app) fn is_live(&self, doc: &MindMapDocument) -> bool {
        self.draft.is_live(doc)
    }

    /// Run a `LabelEdit*` cursor, delete or insert-space Action.
    /// `Edit::None` for any other Action, commit and cancel among them,
    /// which end the session. O(the text).
    pub(in crate::application::app) fn apply_action(&mut self, action: &Action) -> Edit {
        let step = match action {
            Action::LabelEditCursorLeft => Step::Left,
            Action::LabelEditCursorRight => Step::Right,
            Action::LabelEditCursorHome => Step::Home,
            Action::LabelEditCursorEnd => Step::End,
            Action::LabelEditDeleteBack => Step::DeleteBack,
            Action::LabelEditDeleteForward => Step::DeleteForward,
            Action::LabelEditInsertSpace => Step::Space,
            _ => return Edit::None,
        };
        self.draft.step(step, false)
    }

    /// A key no `LabelEdit*` binding claims: its printable payload,
    /// typed as one counted splice; a key with none does nothing — a
    /// single line takes no line break and no tab. O(the text).
    pub(in crate::application::app) fn type_key(&mut self, logical_key: &Key) -> Edit {
        typed_payload(logical_key).map_or(Edit::None, |text| self.draft.insert(&text))
    }

    /// Stage the buffer (see [`Draft::stage`]).
    pub(in crate::application::app) fn stage(&mut self, doc: &mut MindMapDocument) -> bool {
        self.draft.stage(doc, LABEL)
    }

    /// End the session, writing it as one history entry. Committed empty,
    /// a label is removed — a label this session created is canceled
    /// away, leaving no entry — as `main` dropped an emptied label; any
    /// other owner keeps no spans, and draws its `content` where it has
    /// one. A session that changed nothing cancels, leaving no dead entry.
    /// Nothing for a session the writer already ended.
    pub(in crate::application::app) fn commit(mut self, doc: &mut MindMapDocument) -> Option<Revision> {
        if !self.is_live(doc) {
            return None;
        }
        let emptied_label = self.is_label && self.draft.buffer().grapheme_len() == 0;
        if (emptied_label && self.created_label) || (!self.created_label && self.draft.unchanged()) {
            doc.cancel(Interaction::SingleLineEdit);
            return None;
        }
        if emptied_label {
            let owner = self.draft.owner();
            let removed = remove(doc.doc(), DocKind::Label, &[owner])
                .ok()
                .is_some_and(|steps| {
                    doc.stage(Interaction::SingleLineEdit, EditRequest::new(steps, LABEL))
                        .is_ok()
                });
            if !removed {
                // The emptied text stays staged, as any owner's would.
                self.draft.stage(doc, LABEL);
            }
        }
        doc.commit(Interaction::SingleLineEdit)
    }

    /// End the session without writing it: the document is exactly as
    /// before it opened — a label it inserted gone with its text.
    pub(in crate::application::app) fn cancel(self, doc: &mut MindMapDocument) {
        doc.cancel(Interaction::SingleLineEdit);
    }
}

/// The `LabelEdit*` Action `key` is bound to in `InputContext::LabelEdit`,
/// with `modifiers`, or `None` when the key is a literal.
pub(in crate::application::app) fn bound_action(
    key_name: Option<&str>,
    modifiers: &Modifiers,
    keybinds: &ResolvedKeybinds,
) -> Option<Action> {
    key_name.and_then(|name| {
        keybinds.action_for_context(
            InputContext::LabelEdit,
            name,
            modifiers.control_key(),
            modifiers.shift_key(),
            modifiers.alt_key(),
        )
    })
}
