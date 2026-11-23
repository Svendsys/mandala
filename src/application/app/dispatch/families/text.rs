// SPDX-License-Identifier: MPL-2.0

//! **Text editing on spans** (V1-C8, #249): the section editor
//! (`app::text_edit`, both targets), the single-line editor
//! (`app::single_line_edit`, native) and node editing
//! (`InteractionMode::NodeEdit`), on the one writer — each editor session
//! a transaction ([`TextSession`], `LineSession`), each change one stage.
//!
//! Two halves, as on `main`:
//!
//! - [`dispatch`], asked on both targets before the cross-platform arms:
//!   every `TextEdit*` Action; `EnterNodeEdit*`, `EnterSectionEdit`;
//!   `EditSelection*` on a node or a section; `DoubleClickActivate` on a
//!   node, a section or empty canvas; `CreateOrphanNodeAndEdit`. It
//!   answers `None` for what is the native half's, so the native funnel
//!   asks that next.
//! - `dispatch_native`, asked by the native funnel after the transactions
//!   family: every `LabelEdit*` Action, `LabelEditOnSelection`,
//!   `EditSelection*` on a path, and `DoubleClickActivate` on a label, a
//!   ring, a cap, a portal end or a path's content. The browser has no
//!   single-line editor (CLAUDE.md "Single-line editor"): there these
//!   answer `Unhandled`, and a double-click on such an element is logged
//!   at `debug!` ([`opens_single_line`]).
//!
//! **Keys.** While an editor is open it steals the keyboard
//! (`event_keyboard`, `run_wasm::event_keyboard`): a key bound in its
//! context runs its Action through the funnel — commit, cancel and every
//! cursor and delete primitive — and any other key is a literal
//! ([`type_literal`], `type_line`), which is how an IME or dead-key
//! cluster arrives.
//!
//! **The view follows the editor**: every step here hands the view the
//! editor's focus (`MapView::follow_editor`) before it syncs, and the
//! frame does too, so the caret is drawn where the projector drew the
//! text.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::ids::Document;
use glam::Vec2;

use crate::application::app::input_context_core::InputContextCore;
use crate::application::app::interaction_mode::InteractionMode;
use crate::application::app::text_edit::{bound_action, Edit, TextEditor, TextSession};
use crate::application::document::selection::kind_of;
use crate::application::document::MindMapDocument;
use crate::application::keybinds::{Action, ActionKind, InputContext};
use crate::application::platform::input::{Key, Modifiers};
use crate::application::view::caret::EditorFocus;

use super::super::cross_dispatch::{DispatchHit, DispatchOutcome};

/// Why a text Action did nothing — one sentence each, the reason a
/// refused dispatch returns.
pub(in crate::application::app) struct TextRefusal;

impl TextRefusal {
    /// No map.
    pub(in crate::application::app) const NO_DOCUMENT: &'static str = "no document is loaded";
    /// No node or section is selected to edit.
    pub(in crate::application::app) const NO_NODE: &'static str = "select a node or a section to edit";
    /// `EnterSectionEdit` outside node editing.
    pub(in crate::application::app) const NOT_IN_NODE_EDIT: &'static str =
        "EnterSectionEdit works in node editing: enter it first";
    /// `EnterSectionEdit` with a selection that is not a section of the
    /// node being edited.
    pub(in crate::application::app) const NOT_ITS_SECTION: &'static str =
        "select a section of the node being edited";
    /// `DoubleClickActivate` reached with no pointer gesture behind it.
    pub(in crate::application::app) const NO_POINTER: &'static str =
        "DoubleClickActivate needs a pointer gesture to act on";
    /// No editor is open for its keys to reach.
    pub(in crate::application::app) const NO_EDITOR: &'static str = "no text editor is open";
}

/// The node or path the selection names for `owner`: its node for a
/// section or a ring, its path for a label, a cap or a path's content, its
/// link for a portal end and its label. O(the depth to it, at most
/// three).
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        dead_code,
        reason = "its reader is the single-line editor's focus, which is native"
    )
)]
pub(in crate::application::app) fn host_of(doc: &Document, owner: NodeId) -> NodeId {
    let mut at = owner;
    loop {
        match kind_of(doc, at) {
            Some(DocKind::Node | DocKind::Connector | DocKind::Link) | None => return at,
            Some(_) => match doc.parent(at) {
                Some(parent) => at = parent,
                None => return at,
            },
        }
    }
}

/// What the view follows of the section editor: its focus, or `None`
/// when it is closed. O(1).
pub(in crate::application::app) fn text_focus(text: &TextEditor) -> Option<EditorFocus> {
    text.session().map(|session| EditorFocus {
        owner: session.section(),
        host: session.node(),
        caret: session.draft().caret(),
        anchor: session.draft().anchor(),
    })
}

/// Sync the view of `core`, handing it the section editor's focus first
/// when `follow`: the one pipeline, after a step that changed the editor,
/// the document or the selection.
fn resync(core: &mut InputContextCore<'_>, follow: bool) {
    let (Some(doc), Some(view)) = (core.document.as_deref_mut(), core.view.as_mut()) else {
        return;
    };
    if follow {
        view.follow_editor(text_focus(core.text_edit));
    }
    view.follow_node_edit(core.interaction_mode);
    view.sync(doc, core.renderer);
}

/// Close a section editor session whose transaction the writer ended
/// another way — an undo cancels it (ruling R36) — with nothing written:
/// what the frame runs before it syncs. Returns whether one was closed.
/// O(1).
pub(in crate::application::app) fn drop_stale_text(
    text: &mut TextEditor,
    doc: &MindMapDocument,
    mode: &mut InteractionMode,
) -> bool {
    let stale = text.session().is_some_and(|session| !session.is_live(doc));
    if stale {
        if let Some(session) = text.take() {
            leave_node_edit_after(&session, mode);
        }
    }
    stale
}

/// Leave NodeEdit when `session` was opened by `EnterNodeEdit` on a node
/// of one section, as on `main`: closing its editor leaves nothing to
/// pick.
fn leave_node_edit_after(session: &TextSession, mode: &mut InteractionMode) {
    if session.exit_to_default_on_close() && mode.node_edit().is_some() {
        *mode = InteractionMode::Default;
    }
}

/// The family's cross-platform answer to `action`, or `None` for an
/// Action it does not own here. See the [module docs](self).
pub(in crate::application::app) fn dispatch(
    action: &Action,
    core: &mut InputContextCore<'_>,
    hit: Option<&DispatchHit>,
) -> Option<DispatchOutcome> {
    Some(match action {
        Action::TextEditCommit => close_text(core, true),
        Action::TextEditCancel => close_text(core, false),
        action if ActionKind::from(action).context() == InputContext::TextEdit => step_text(core, action),
        Action::EnterNodeEdit => enter_node_edit(core, false),
        Action::EnterNodeEditClean => enter_node_edit(core, true),
        Action::EnterSectionEdit => enter_section_edit(core),
        Action::EditSelection | Action::EditSelectionClean => {
            let clean = *action == Action::EditSelectionClean;
            match core.document.as_deref().and_then(selected_node) {
                Some(_) => enter_node_edit(core, clean),
                None => return None,
            }
        }
        Action::DoubleClickActivate => return double_click(core, hit),
        Action::CreateOrphanNodeAndEdit => {
            let at = match hit {
                Some(hit) => hit.canvas_pos,
                None => core
                    .renderer
                    .screen_to_canvas(core.cursor_pos.0 as f32, core.cursor_pos.1 as f32),
            };
            create_orphan(core, at)
        }
        _ => return None,
    })
}

/// The one selected element, when exactly one is selected. O(1).
pub(in crate::application::app) fn only_selected(doc: &MindMapDocument) -> Option<NodeId> {
    let mut selected = doc.selection.iter();
    match (selected.next(), selected.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// The node a node or section selection edits: the one selected node, or
/// the node of the one selected section. `None` for anything else.
fn selected_node(doc: &MindMapDocument) -> Option<NodeId> {
    let only = only_selected(doc)?;
    match kind_of(doc.doc(), only)? {
        DocKind::Node => Some(only),
        DocKind::Section => doc.doc().parent(only),
        _ => None,
    }
}

/// The sections of `node`, in sibling order.
fn sections_of(doc: &Document, node: NodeId) -> Vec<NodeId> {
    doc.children(node)
        .filter(|&child| kind_of(doc, child) == Some(DocKind::Section))
        .collect()
}

/// Open the section editor on `section`, as `open` says, and follow it.
fn open_text(
    core: &mut InputContextCore<'_>,
    section: NodeId,
    clean: bool,
    exit_to_default_on_close: bool,
) -> DispatchOutcome {
    let Some(doc) = core.document.as_deref_mut() else {
        return DispatchOutcome::Refused(TextRefusal::NO_DOCUMENT);
    };
    match TextSession::open(doc, section, clean, exit_to_default_on_close) {
        Ok(session) => {
            *core.text_edit = TextEditor::Open(session);
            resync(core, true);
            DispatchOutcome::Handled
        }
        Err(why) => {
            // A D28 refusal leaves its status line for the frame to draw.
            resync(core, false);
            DispatchOutcome::Refused(why)
        }
    }
}

/// `EnterNodeEdit` (and `EditSelection` on a node or a section): NodeEdit
/// on the selection's node. On a node of one section the editor opens on
/// it at once and closing it leaves NodeEdit again, as on `main`; on a
/// node of more, the mode is entered and the user picks a section. `clean`
/// opens that editor empty.
fn enter_node_edit(core: &mut InputContextCore<'_>, clean: bool) -> DispatchOutcome {
    let Some(doc) = core.document.as_deref_mut() else {
        return DispatchOutcome::Refused(TextRefusal::NO_DOCUMENT);
    };
    let Some(node) = selected_node(doc) else {
        return DispatchOutcome::Refused(TextRefusal::NO_NODE);
    };
    let sections = sections_of(doc.doc(), node);
    *core.interaction_mode = InteractionMode::NodeEdit { node };
    match sections.as_slice() {
        [only] => {
            doc.selection.select_only(node);
            let outcome = open_text(core, *only, clean, true);
            if !matches!(outcome, DispatchOutcome::Handled) {
                *core.interaction_mode = InteractionMode::Default;
            }
            outcome
        }
        _ => {
            if !doc.selection.contains(node) {
                doc.selection.select_only(node);
            }
            resync(core, false);
            DispatchOutcome::Handled
        }
    }
}

/// `EnterSectionEdit`: in NodeEdit, the editor on the selected section of
/// the node being edited — its first section when the node itself is
/// selected. Closing the editor returns to NodeEdit, to pick another.
fn enter_section_edit(core: &mut InputContextCore<'_>) -> DispatchOutcome {
    let Some(node) = core.interaction_mode.node_edit() else {
        return DispatchOutcome::Refused(TextRefusal::NOT_IN_NODE_EDIT);
    };
    let Some(doc) = core.document.as_deref() else {
        return DispatchOutcome::Refused(TextRefusal::NO_DOCUMENT);
    };
    let section = match only_selected(doc) {
        Some(only) if only == node => sections_of(doc.doc(), node).first().copied(),
        Some(only)
            if kind_of(doc.doc(), only) == Some(DocKind::Section) && doc.doc().parent(only) == Some(node) =>
        {
            Some(only)
        }
        _ => None,
    };
    match section {
        Some(section) => open_text(core, section, false, false),
        None => DispatchOutcome::Refused(TextRefusal::NOT_ITS_SECTION),
    }
}

/// `CreateOrphanNodeAndEdit`: one transaction for the node and its text
/// (ruling R7 on #238) — the node at `at`, the editor on its section;
/// commit is one entry, cancel leaves no node.
fn create_orphan(core: &mut InputContextCore<'_>, at: Vec2) -> DispatchOutcome {
    let Some(doc) = core.document.as_deref_mut() else {
        return DispatchOutcome::Refused(TextRefusal::NO_DOCUMENT);
    };
    match TextSession::open_orphan(doc, at) {
        Ok(session) => {
            *core.text_edit = TextEditor::Open(session);
            resync(core, true);
            DispatchOutcome::Handled
        }
        Err(why) => {
            resync(core, false);
            DispatchOutcome::Refused(why)
        }
    }
}

/// Whether a double-click on `element` is the single-line editor's: a
/// label, a ring, a cap, a portal end or a path's content — what the
/// browser, which has no single-line editor, logs and leaves (CLAUDE.md
/// "Single-line editor"). O(1).
pub(in crate::application::app) fn opens_single_line(doc: &Document, element: Option<NodeId>) -> bool {
    element.is_some_and(|element| {
        matches!(
            kind_of(doc, element),
            Some(
                DocKind::Ring
                    | DocKind::CapStart
                    | DocKind::CapEnd
                    | DocKind::Label
                    | DocKind::PortalEnd
                    | DocKind::Connector
                    | DocKind::Link
            )
        )
    })
}

/// `DoubleClickActivate`, its cross-platform half: edits the exact element
/// the press hit (ruling K3 on #249). A section opens the section editor on
/// itself, a node's body on its first section — the node selected, or in
/// NodeEdit on that node the section — and empty canvas creates a node to
/// edit, when the user bound `CreateOrphanNodeAndEdit` anywhere and no path
/// is selected, as on `main`. `None` for the single-line editor's
/// elements, which are the native half's.
fn double_click(core: &mut InputContextCore<'_>, hit: Option<&DispatchHit>) -> Option<DispatchOutcome> {
    let Some(hit) = hit else {
        return Some(DispatchOutcome::Refused(TextRefusal::NO_POINTER));
    };
    let Some(doc) = core.document.as_deref_mut() else {
        return Some(DispatchOutcome::Refused(TextRefusal::NO_DOCUMENT));
    };
    if opens_single_line(doc.doc(), hit.element) {
        return None;
    }
    let section = match hit
        .element
        .and_then(|element| Some((element, kind_of(doc.doc(), element)?)))
    {
        Some((section, DocKind::Section)) => Some(section),
        Some((node, DocKind::Node)) => sections_of(doc.doc(), node).first().copied(),
        Some(_) => return Some(DispatchOutcome::Unhandled),
        None => None,
    };
    let Some(section) = section else {
        if hit.element.is_some() {
            return Some(DispatchOutcome::Unhandled);
        }
        let path_selected = doc
            .selection
            .iter()
            .any(|id| matches!(kind_of(doc.doc(), id), Some(DocKind::Connector | DocKind::Link)));
        if path_selected || !core.keybinds.has_any_binding_for(Action::CreateOrphanNodeAndEdit) {
            return Some(DispatchOutcome::Unhandled);
        }
        return Some(create_orphan(core, hit.canvas_pos));
    };
    let node = doc.doc().parent(section)?;
    if core.interaction_mode.node_edit() == Some(node) {
        doc.selection.select_only(section);
    } else {
        doc.selection.select_only(node);
    }
    Some(open_text(core, section, false, false))
}

/// `TextEditCommit` or `TextEditCancel`: end the section editor session,
/// writing it or not, leave NodeEdit when the session was NodeEdit's
/// single-section short-circuit, and sync.
fn close_text(core: &mut InputContextCore<'_>, commit: bool) -> DispatchOutcome {
    let Some(session) = core.text_edit.take() else {
        return DispatchOutcome::Refused(TextRefusal::NO_EDITOR);
    };
    let Some(doc) = core.document.as_deref_mut() else {
        return DispatchOutcome::Refused(TextRefusal::NO_DOCUMENT);
    };
    leave_node_edit_after(&session, core.interaction_mode);
    if commit {
        session.commit(doc);
    } else {
        session.cancel(doc);
    }
    resync(core, true);
    DispatchOutcome::Handled
}

/// A `TextEdit*` cursor, delete or insert-space Action on the open
/// section editor: the buffer staged when its text changed, the caret
/// followed either way. A session the writer already ended is closed,
/// with nothing written.
fn step_text(core: &mut InputContextCore<'_>, action: &Action) -> DispatchOutcome {
    let Some(doc) = core.document.as_deref_mut() else {
        return DispatchOutcome::Refused(TextRefusal::NO_DOCUMENT);
    };
    if drop_stale_text(core.text_edit, doc, core.interaction_mode) {
        resync(core, true);
        return DispatchOutcome::Refused(TextRefusal::NO_EDITOR);
    }
    let Some(session) = core.text_edit.session_mut() else {
        return DispatchOutcome::Refused(TextRefusal::NO_EDITOR);
    };
    match session.apply_action(action) {
        Edit::None => return DispatchOutcome::Handled,
        Edit::Caret => {}
        Edit::Text => {
            session.stage(doc);
        }
    }
    resync(core, true);
    DispatchOutcome::Handled
}

/// A key the open section editor's context binds no Action to: a
/// literal, typed at the caret and staged (`TextSession::type_key`).
/// Returns whether it changed anything. Both targets' keyboard ladders
/// run it.
pub(in crate::application::app) fn type_literal(
    core: &mut InputContextCore<'_>,
    key_name: Option<&str>,
    logical_key: &Key,
) -> bool {
    let Some(doc) = core.document.as_deref_mut() else {
        return false;
    };
    if drop_stale_text(core.text_edit, doc, core.interaction_mode) {
        resync(core, true);
        return false;
    }
    let Some(session) = core.text_edit.session_mut() else {
        return false;
    };
    match session.type_key(key_name, logical_key) {
        Edit::None => false,
        Edit::Caret => {
            resync(core, true);
            true
        }
        Edit::Text => {
            session.stage(doc);
            resync(core, true);
            true
        }
    }
}

/// What a key does while the section editor is open: the `TextEdit*`
/// Action it is bound to, or `None` for a literal. Any other Action a key
/// would reach is swallowed — the editor owns the keyboard — as on
/// `main`. O(the context's bindings).
pub(in crate::application::app) fn text_key(
    key_name: Option<&str>,
    modifiers: &Modifiers,
    keybinds: &crate::application::keybinds::ResolvedKeybinds,
) -> Option<Action> {
    bound_action(key_name, modifiers, keybinds)
}

/// The NodeEdit half of a click (native, which routes clicks inside a
/// node): a click on a section of the node being edited selects that
/// section; one elsewhere on that node selects the node; one outside it
/// leaves NodeEdit — the selection back on the node — and returns `false`
/// so the click goes on to select what it landed on. Returns whether the
/// click was NodeEdit's. O(the depth to the element hit).
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn node_edit_click(
    doc: &mut MindMapDocument,
    mode: &mut InteractionMode,
    landed: Option<&baumhard::projection::Hit>,
) -> bool {
    let Some(node) = mode.node_edit() else {
        return false;
    };
    match landed {
        Some(hit) if hit.node == node => {
            let section = (kind_of(doc.doc(), hit.element) == Some(DocKind::Section)
                && doc.doc().parent(hit.element) == Some(node))
            .then_some(hit.element);
            doc.selection.select_only(section.unwrap_or(node));
            true
        }
        _ => {
            leave_node_edit(doc, mode);
            false
        }
    }
}

/// Leave NodeEdit: the mode back to `Default`, and a selection inside the
/// edited node — a section, a range of one — lifted back to the node, so
/// the per-node verbs keep working, as on `main`. O(1).
pub(in crate::application::app) fn leave_node_edit(doc: &mut MindMapDocument, mode: &mut InteractionMode) {
    let Some(node) = mode.node_edit() else {
        return;
    };
    *mode = InteractionMode::Default;
    let inside = doc
        .selection
        .first()
        .is_some_and(|first| first == node || doc.doc().parent(first) == Some(node));
    if inside {
        doc.selection.select_only(node);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) use native::{
    dispatch_native, drop_stale_line, editing_under, line_focus, open_from_console, release_keeps_editing,
    type_line, type_to_edit,
};

/// The native half: the single-line editor.
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use baumhard::arena::NodeId;
    use baumhard::document::element::DocKind;

    use super::{host_of, only_selected, text_focus, DispatchHit, DispatchOutcome, TextRefusal};
    use crate::application::app::input_context::InputHandlerContext;
    use crate::application::app::single_line_edit::{
        LineRefusal, LineSession, SingleLineEditor, SingleLineTarget,
    };
    use crate::application::app::text_edit::Edit;
    use crate::application::document::selection::kind_of;
    use crate::application::document::MindMapDocument;
    use crate::application::keybinds::{Action, ActionKind, InputContext};
    use crate::application::platform::input::Key;
    use crate::application::view::caret::EditorFocus;

    /// What the view follows of the single-line editor: its focus, or
    /// `None` when it is closed.
    pub(in crate::application::app) fn line_focus(
        line: &SingleLineEditor,
        doc: &baumhard::document::ids::Document,
    ) -> Option<EditorFocus> {
        line.session().map(|session| EditorFocus {
            owner: session.owner(),
            host: host_of(doc, session.owner()),
            caret: session.draft().caret(),
            anchor: None,
        })
    }

    /// Sync the view of `ctx`, handing it both editors' focus — at most one
    /// is open (D28).
    fn resync(ctx: &mut InputHandlerContext<'_>) {
        let (Some(doc), Some(view)) = (ctx.document.as_mut(), ctx.view.as_mut()) else {
            return;
        };
        view.follow_editor(text_focus(ctx.text_edit).or_else(|| line_focus(ctx.single_line_edit, doc.doc())));
        view.sync(doc, ctx.renderer);
    }

    /// Close a single-line session the writer ended another way (an undo),
    /// with nothing written. Returns whether one was closed. O(1).
    pub(in crate::application::app) fn drop_stale_line(
        line: &mut SingleLineEditor,
        doc: &MindMapDocument,
    ) -> bool {
        let stale = line.session().is_some_and(|session| !session.is_live(doc));
        if stale {
            line.take();
        }
        stale
    }

    /// The family's native answer to `action`, or `None` for an Action it
    /// does not own here.
    pub(in crate::application::app) fn dispatch_native(
        action: &Action,
        ctx: &mut InputHandlerContext<'_>,
        hit: Option<&DispatchHit>,
    ) -> Option<DispatchOutcome> {
        Some(match action {
            Action::LabelEditCommit => close_line(ctx, true),
            Action::LabelEditCancel => close_line(ctx, false),
            action if ActionKind::from(action).context() == InputContext::LabelEdit => step_line(ctx, action),
            Action::LabelEditOnSelection => label_on_selection(ctx, false),
            Action::EditSelection | Action::EditSelectionClean => {
                let on_a_path = ctx.document.as_ref().is_some_and(|doc| {
                    only_selected(doc).is_some_and(|only| {
                        matches!(kind_of(doc.doc(), only), Some(DocKind::Connector | DocKind::Link))
                    })
                });
                if !on_a_path {
                    return Some(DispatchOutcome::Refused(TextRefusal::NO_NODE));
                }
                label_on_selection(ctx, *action == Action::EditSelectionClean)
            }
            Action::DoubleClickActivate => {
                let hit = hit?;
                let doc = ctx.document.as_ref()?;
                let target = SingleLineTarget::of_element(doc.doc(), hit.element?)?;
                open_line(ctx, target, false)
            }
            _ => return None,
        })
    }

    /// Open the single-line editor on `target`, and follow it.
    fn open_line(
        ctx: &mut InputHandlerContext<'_>,
        target: SingleLineTarget,
        clean: bool,
    ) -> DispatchOutcome {
        let Some(doc) = ctx.document.as_mut() else {
            return DispatchOutcome::Refused(TextRefusal::NO_DOCUMENT);
        };
        match LineSession::open(doc, target, clean) {
            Ok(session) => {
                *ctx.single_line_edit = SingleLineEditor::Open(session);
                resync(ctx);
                DispatchOutcome::Handled
            }
            Err(why) => {
                resync(ctx);
                DispatchOutcome::Refused(why)
            }
        }
    }

    /// `LabelEditOnSelection`, and `EditSelection*` on a path: the editor
    /// on the label of the one selected connector or line link — inserted
    /// first when it has none (ruling K3) — refused on a portal link,
    /// whose labels are its ends' (ruling K4), with the status line naming
    /// the way in.
    fn label_on_selection(ctx: &mut InputHandlerContext<'_>, clean: bool) -> DispatchOutcome {
        let Some(doc) = ctx.document.as_mut() else {
            return DispatchOutcome::Refused(TextRefusal::NO_DOCUMENT);
        };
        let Some(only) = only_selected(doc) else {
            return DispatchOutcome::Refused(LineRefusal::NOT_A_LINE);
        };
        match SingleLineTarget::label_of(doc.doc(), only) {
            Ok(target) => open_line(ctx, target, clean),
            Err(why) => {
                if why == LineRefusal::PORTAL_LABEL {
                    doc.decline(why);
                    resync(ctx);
                }
                DispatchOutcome::Refused(why)
            }
        }
    }

    /// `LabelEditCommit` or `LabelEditCancel`: end the session, writing it
    /// or not, and sync.
    fn close_line(ctx: &mut InputHandlerContext<'_>, commit: bool) -> DispatchOutcome {
        let Some(session) = ctx.single_line_edit.take() else {
            return DispatchOutcome::Refused(TextRefusal::NO_EDITOR);
        };
        let Some(doc) = ctx.document.as_mut() else {
            return DispatchOutcome::Refused(TextRefusal::NO_DOCUMENT);
        };
        if commit {
            session.commit(doc);
        } else {
            session.cancel(doc);
        }
        resync(ctx);
        DispatchOutcome::Handled
    }

    /// A `LabelEdit*` cursor, delete or insert-space Action on the open
    /// single-line editor.
    fn step_line(ctx: &mut InputHandlerContext<'_>, action: &Action) -> DispatchOutcome {
        let Some(doc) = ctx.document.as_mut() else {
            return DispatchOutcome::Refused(TextRefusal::NO_DOCUMENT);
        };
        if drop_stale_line(ctx.single_line_edit, doc) {
            resync(ctx);
            return DispatchOutcome::Refused(TextRefusal::NO_EDITOR);
        }
        let Some(session) = ctx.single_line_edit.session_mut() else {
            return DispatchOutcome::Refused(TextRefusal::NO_EDITOR);
        };
        match session.apply_action(action) {
            Edit::None => return DispatchOutcome::Handled,
            Edit::Caret => {}
            Edit::Text => {
                session.stage(doc);
            }
        }
        resync(ctx);
        DispatchOutcome::Handled
    }

    /// A key the open single-line editor's context binds no Action to: its
    /// printable payload, typed and staged. Returns whether it changed
    /// anything.
    pub(in crate::application::app) fn type_line(
        ctx: &mut InputHandlerContext<'_>,
        logical_key: &Key,
    ) -> bool {
        let Some(doc) = ctx.document.as_mut() else {
            return false;
        };
        if drop_stale_line(ctx.single_line_edit, doc) {
            resync(ctx);
            return false;
        }
        let Some(session) = ctx.single_line_edit.session_mut() else {
            return false;
        };
        if session.type_key(logical_key) == Edit::None {
            return false;
        }
        session.stage(doc);
        resync(ctx);
        true
    }

    /// Type-to-edit, as on `main`: a printable key no binding claims, with
    /// one connector or line link selected, opens the editor on its label
    /// — the label's text kept — and types the key into it. Returns whether
    /// it did.
    pub(in crate::application::app) fn type_to_edit(
        ctx: &mut InputHandlerContext<'_>,
        logical_key: &Key,
    ) -> bool {
        if ctx.modifiers.control_key() || ctx.modifiers.alt_key() {
            return false;
        }
        if crate::application::app::text_edit::typed_payload(logical_key).is_none() {
            return false;
        }
        let target = {
            let Some(doc) = ctx.document.as_ref() else {
                return false;
            };
            let Some(only) = only_selected(doc) else {
                return false;
            };
            if !crate::application::view::handles::is_line_path(doc.doc(), only) {
                return false;
            }
            let Ok(target) = SingleLineTarget::label_of(doc.doc(), only) else {
                return false;
            };
            target
        };
        if open_line(ctx, target, false) != DispatchOutcome::Handled {
            return false;
        }
        type_line(ctx, logical_key);
        true
    }

    /// Open the editor a console line named (`ConsoleSideEffect::OpenEditor`):
    /// the section editor on a section, the single-line editor on an owner
    /// or on a path's label. Returns why it did not open.
    pub(in crate::application::app) fn open_from_console(
        ctx: &mut InputHandlerContext<'_>,
        open: crate::application::console::EditorOpen,
    ) -> Result<(), &'static str> {
        use crate::application::console::EditorOpen;
        let target = {
            let Some(doc) = ctx.document.as_ref() else {
                return Err(TextRefusal::NO_DOCUMENT);
            };
            match open {
                EditorOpen::Section(section) => {
                    let (mut core, _) = ctx.split_borrow();
                    return match super::open_text(&mut core, section, false, false) {
                        DispatchOutcome::Handled => Ok(()),
                        DispatchOutcome::Refused(why) => Err(why),
                        DispatchOutcome::Unhandled => Err(TextRefusal::NO_DOCUMENT),
                    };
                }
                EditorOpen::Line(owner) => {
                    SingleLineTarget::of_element(doc.doc(), owner).ok_or(LineRefusal::NOT_A_LINE)?
                }
                EditorOpen::Label(host) => SingleLineTarget::label_of(doc.doc(), host)?,
            }
        };
        match open_line(ctx, target, false) {
            DispatchOutcome::Handled => Ok(()),
            DispatchOutcome::Refused(why) => Err(why),
            DispatchOutcome::Unhandled => Err(TextRefusal::NO_DOCUMENT),
        }
    }

    /// Whether a press that `landed` is on what an open editor edits: the
    /// section editor's node, the single-line editor's owner. O(1).
    pub(in crate::application::app) fn editing_under(
        ctx: &InputHandlerContext<'_>,
        landed: Option<&baumhard::projection::Hit>,
    ) -> bool {
        let Some(hit) = landed else {
            return false;
        };
        ctx.text_edit.session().is_some_and(|s| hit.node == s.node())
            || ctx
                .single_line_edit
                .session()
                .is_some_and(|s| hit.element == s.owner())
    }

    /// The modal half of a left release, as on `main`: a release on what an
    /// open editor edits keeps it open and is consumed (`true`); a release
    /// anywhere else commits it through the funnel and goes on to click
    /// (`false`). The section editor edits its node; the single-line editor
    /// its owner.
    pub(in crate::application::app) fn release_keeps_editing(
        ctx: &mut InputHandlerContext<'_>,
        landed: Option<&baumhard::projection::Hit>,
    ) -> bool {
        let (text_node, line_owner): (Option<NodeId>, Option<NodeId>) = (
            ctx.text_edit.session().map(|s| s.node()),
            ctx.single_line_edit.session().map(|s| s.owner()),
        );
        if let Some(node) = text_node {
            if landed.is_some_and(|hit| hit.node == node) {
                return true;
            }
            let _ = crate::application::app::dispatch::dispatch_action(Action::TextEditCommit, ctx, None);
        }
        if let Some(owner) = line_owner {
            if landed.is_some_and(|hit| hit.element == owner) {
                return true;
            }
            let _ = crate::application::app::dispatch::dispatch_action(Action::LabelEditCommit, ctx, None);
        }
        false
    }
}
