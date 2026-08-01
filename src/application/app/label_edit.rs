// SPDX-License-Identifier: MPL-2.0

//! Inline edge-label editor: open / handle key / close. Opens an
//! in-place editor on a selected edge, routes printable / navigation
//! / commit keystrokes through `route_label_edit_key`, and commits or
//! cancels back to the model's `MindEdge.label` on Enter / Esc.

use crate::application::platform::input::Key;

use baumhard::util::grapheme_chad;

use crate::application::document::MindMapDocument;
use crate::application::keybinds::{InputContext, ResolvedKeybinds};
use crate::application::renderer::Renderer;

use super::scene_rebuild::{rebuild_all, CanvasFrame};
use super::text_edit::insert_caret;

/// Pure router for the label-edit *character-input* path. Inserts
/// printable chars from a `Key::Character` payload into the buffer.
///
/// Originally this also handled structural keys (Backspace, Delete,
/// arrows, Home, End) directly via `Key::Named` matching, but Phase 5
/// migrated those to `Action::LabelEdit*` variants that route through
/// `dispatch::apply_label_edit_action_to_buffer`. The structural-key
/// arms were stripped here so unbinding `label_edit_*` in
/// `keybinds.json` actually disables the key — the previous fallback
/// shadowed user config.
///
/// Returns `true` iff a printable character was inserted.
pub(super) fn route_label_edit_key(logical_key: &Key, buffer: &mut String, cursor: &mut usize) -> bool {
    if let Key::Character(c) = logical_key {
        // Control chars (and any non-printing payload winit attaches
        // to a structural key) are filtered, which mirrors the
        // original guard intent. Insert the remaining payload as one
        // counted splice so IME / dead-key multi-codepoint clusters
        // advance the grapheme cursor by their visible delta.
        let filtered: String = c.as_str().chars().filter(|ch| !ch.is_control()).collect();
        if !filtered.is_empty() {
            *cursor += grapheme_chad::insert_str_at_grapheme_counted(buffer, *cursor, &filtered);
            return true;
        }
    }
    false
}

// (`CanvasFrame::update_portal_tree` is used by the portal-text
// editor; the line-mode label editor deliberately does NOT touch
// the portal canvas because `label_edit_preview` only feeds
// the connection-label pass — see `tree_builder::connection_label`.)

/// Inline-edit state for a connection's label. When `Open`, all
/// keyboard input is routed to the label-edit handler (just like
/// `ConsoleState::Open` captures keys for the console input line).
/// Mutually exclusive with `ConsoleState::Open` — the console check
/// runs first, so opening the console while editing a label is a
/// no-op.
///
/// Mirrors `TextEditState` in shape (buffer + grapheme cursor): every
/// keystroke routes through `grapheme_chad` so backspace over an
/// emoji removes the whole cluster, not a stray byte. The buffer is
/// threaded into the connection-label pass via
/// `MindMapDocument::label_edit_preview`; the connection-label tree's
/// in-place mutator path picks up the new text + caret without
/// rebuilding the arena.
#[derive(Debug, Clone)]
pub(in crate::application::app) enum LabelEditState {
    Closed,
    Open {
        edge_ref: crate::application::document::EdgeRef,
        /// The in-progress buffer. Committed to
        /// `MindEdge.label` on Enter; discarded on Escape.
        buffer: String,
        /// Cursor position as a grapheme-cluster index into
        /// `buffer`. Valid range
        /// `[0, count_grapheme_clusters(buffer)]`. Stored in
        /// graphemes (not chars or bytes) so backspace over an
        /// emoji or ZWJ cluster removes the whole user-visible
        /// character — same invariant as
        /// `TextEditState::Open::cursor_grapheme_pos`.
        cursor_grapheme_pos: usize,
        /// The edge's label value at the moment edit mode opened.
        /// Used to restore state on Escape so the cancel is clean.
        original: Option<String>,
    },
}

impl LabelEditState {
    pub(in crate::application::app) fn is_open(&self) -> bool {
        matches!(self, LabelEditState::Open { .. })
    }

    /// Borrow the edge currently under edit, if any. Callers use
    /// this for the click-outside-to-commit check in
    /// `event_mouse_click`: a release that doesn't hit the edge
    /// whose label is open commits the buffer, mirroring the node
    /// text editor's "click outside the node AABB → commit" rule.
    pub(in crate::application::app) fn edited_edge_ref(
        &self,
    ) -> Option<&crate::application::document::EdgeRef> {
        match self {
            LabelEditState::Open { edge_ref, .. } => Some(edge_ref),
            LabelEditState::Closed => None,
        }
    }
}

/// Which single-line editor a selection names. Pure classification
/// of [`crate::application::document::SelectionState`], resolved by
/// [`resolve_edge_editor_plan`].
///
/// Three call sites open a single-line editor off the current
/// selection — `Action::EditSelection` / `EditSelectionClean`'s
/// non-node fall-through, `Action::LabelEditOnSelection`, and the
/// type-to-edit path in `event_keyboard.rs`. They used to carry
/// three copies of this match; the plan is the one copy.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::application::app) enum EdgeEditorPlan {
    /// The selection names no single-line editor target (a node /
    /// section / plain-edge / empty selection). Callers no-op.
    None,
    /// Open the edge-label editor on this edge.
    Label {
        edge_ref: crate::application::document::EdgeRef,
    },
    /// Open the portal-text editor on this edge endpoint.
    PortalText {
        edge_ref: crate::application::document::EdgeRef,
        endpoint_node_id: String,
    },
}

/// Resolve the [`EdgeEditorPlan`] a selection names. Pure — no
/// document lookup, no renderer — so the routing is testable
/// without standing up the app.
///
/// `PortalLabel` and `PortalText` both open the portal-text editor:
/// the label is the endpoint's glyph and the text is its caption,
/// and the editor edits the caption in both cases.
///
/// The match is deliberately **exhaustive** rather than
/// `_ => None`: `SelectionState` has ten variants today, and an
/// eleventh should be a build error here, so whoever adds it has to
/// decide whether it names a single-line editor instead of silently
/// inheriting "no".
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn resolve_edge_editor_plan(
    selection: &crate::application::document::SelectionState,
) -> EdgeEditorPlan {
    use crate::application::document::SelectionState;
    match selection {
        SelectionState::EdgeLabel(s) => EdgeEditorPlan::Label {
            edge_ref: s.edge_ref.clone(),
        },
        SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => EdgeEditorPlan::PortalText {
            edge_ref: s.edge_ref(),
            endpoint_node_id: s.endpoint_node_id.clone(),
        },
        // Node-scoped selections belong to the node text editor
        // (`apply_enter_node_edit` owns them, cross-platform); a
        // bare `Edge` selection has a body but no label under edit;
        // `None` has no target at all.
        SelectionState::None
        | SelectionState::Single(_)
        | SelectionState::Multi(_)
        | SelectionState::Section(_)
        | SelectionState::MultiSection(_)
        | SelectionState::SectionRange { .. }
        | SelectionState::Edge(_) => EdgeEditorPlan::None,
    }
}

/// Seed a single-line editor's `(buffer, cursor)` pair on open.
///
/// `clean` is `Action::EditSelectionClean`'s empty-buffer contract
/// — the single-line counterpart of `open_text_edit`'s
/// `from_creation`. The cursor always lands at the end of whatever
/// was seeded (0 on the clean path, end-of-text otherwise), and it
/// is counted in grapheme clusters so an existing label ending in
/// an emoji or ZWJ sequence puts the caret after the whole cluster.
///
/// Shared by [`open_label_edit`] and [`open_portal_text_edit`] so
/// the two editors cannot disagree about what `clean` means.
#[cfg(not(target_arch = "wasm32"))]
fn seed_edit_buffer(clean: bool, original: Option<&str>) -> (String, usize) {
    let buffer = if clean {
        String::new()
    } else {
        original.unwrap_or_default().to_string()
    };
    let cursor = grapheme_chad::count_grapheme_clusters(&buffer);
    (buffer, cursor)
}

/// What a single-line editor lifecycle step owes the renderer.
///
/// The `*_core` functions below are renderer-free: they mutate the
/// model, the editor state and the preview slot, then *name* the
/// canvas work their caller must run instead of running it. The
/// shells (`open_*` / `handle_*` / `close_*`) are the only place
/// that touches `Renderer` / `AppScene`.
///
/// Splitting the decree from its execution is what lets the whole
/// editor lifecycle be driven in tests — TEST_CONVENTIONS §T8 keeps
/// live wgpu out of the harness, so a lifecycle that only exists
/// inside a `&mut Renderer` signature cannot be tested at all.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum EditRefresh {
    /// Nothing observable changed; skip the canvas work entirely.
    None,
    /// Re-project only the canvas the live preview feeds — the
    /// connection-label role for an edge label, the portal role for
    /// portal text. The rest of the scene cannot have changed,
    /// which is what keeps per-keystroke typing cheap.
    Preview,
    /// Full `rebuild_all`: the model changed, or the editor stopped
    /// existing, so the preview override has to leave every pass.
    All,
}

/// One editing primitive reaching an open single-line buffer.
///
/// The two arms are the two halves of CODE_CONVENTIONS §3's modal
/// carve-out: rebindable cursor / delete verbs arrive as a resolved
/// [`crate::application::keybinds::Action`], while character
/// insertion arrives as the literal winit key payload because no
/// `Action` body can carry an IME cluster.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub(in crate::application::app) enum EditInput<'k> {
    /// A resolved `LabelEdit*` cursor / delete action.
    Action(crate::application::keybinds::Action),
    /// A literal key payload — printable characters and IME /
    /// dead-key clusters.
    Key(&'k Key),
}

/// Renderer-free half of [`open_label_edit`]. Returns the canvas
/// work the caller owes, or [`EditRefresh::None`] when the edge
/// vanished and the editor stayed closed.
#[cfg(not(target_arch = "wasm32"))]
fn open_label_edit_core(
    edge_ref: &crate::application::document::EdgeRef,
    clean: bool,
    doc: &mut MindMapDocument,
    label_edit_state: &mut LabelEditState,
) -> EditRefresh {
    let edge = match doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)) {
        Some(e) => e,
        None => {
            log::warn!(
                "open_label_edit: edge {}→{} ({}) not found; editor stays closed",
                edge_ref.from_id,
                edge_ref.to_id,
                edge_ref.edge_type
            );
            return EditRefresh::None;
        }
    };
    let original = edge.label.clone();
    let (buffer, cursor_grapheme_pos) = seed_edit_buffer(clean, original.as_deref());
    *label_edit_state = LabelEditState::Open {
        edge_ref: edge_ref.clone(),
        buffer: buffer.clone(),
        cursor_grapheme_pos,
        original,
    };
    // Store the preview on the document so every subsequent
    // `doc.frame_overrides` call picks it up automatically — no
    // renderer field, no read-time override, no belt-and-suspenders
    // branch.
    let edge_key = baumhard::mindmap::scene_cache::EdgeKey::from(edge_ref);
    doc.label_edit_preview = Some((edge_key, insert_caret(&buffer, cursor_grapheme_pos)));
    EditRefresh::Preview
}

/// Renderer-free half of [`handle_label_edit_key`].
#[cfg(not(target_arch = "wasm32"))]
fn handle_label_edit_key_core(
    input: EditInput<'_>,
    label_edit_state: &mut LabelEditState,
    doc: &mut MindMapDocument,
) -> EditRefresh {
    let changed = match input {
        EditInput::Action(a) => {
            crate::application::app::dispatch::apply_label_edit_action(a, label_edit_state)
        }
        EditInput::Key(logical_key) => {
            let Some((buffer, cursor)) = (match label_edit_state {
                LabelEditState::Open {
                    buffer,
                    cursor_grapheme_pos,
                    ..
                } => Some((buffer, cursor_grapheme_pos)),
                LabelEditState::Closed => None,
            }) else {
                return EditRefresh::None;
            };
            route_label_edit_key(logical_key, buffer, cursor)
        }
    };

    if !changed {
        return EditRefresh::None;
    }

    // Refresh the preview on the document so the caret + edited text
    // render on the next frame. The connection-label tree's in-place
    // mutator path picks up the new text without rebuilding the
    // arena because the per-edge identity sequence stays constant
    // during a label edit.
    if let LabelEditState::Open {
        edge_ref,
        buffer,
        cursor_grapheme_pos,
        ..
    } = label_edit_state
    {
        let edge_key = baumhard::mindmap::scene_cache::EdgeKey::from(&*edge_ref);
        doc.label_edit_preview = Some((edge_key, insert_caret(buffer, *cursor_grapheme_pos)));
        return EditRefresh::Preview;
    }
    EditRefresh::None
}

/// Renderer-free half of [`close_label_edit`].
#[cfg(not(target_arch = "wasm32"))]
fn close_label_edit_core(
    commit: bool,
    doc: &mut MindMapDocument,
    label_edit_state: &mut LabelEditState,
) -> EditRefresh {
    let (edge_ref, buffer, original) = match std::mem::replace(label_edit_state, LabelEditState::Closed) {
        LabelEditState::Open {
            edge_ref,
            buffer,
            original,
            ..
        } => (edge_ref, buffer, original),
        LabelEditState::Closed => return EditRefresh::None,
    };
    doc.label_edit_preview = None;
    if commit {
        let new_val = if buffer.is_empty() { None } else { Some(buffer) };
        // Only push undo if the committed value actually differs from the
        // pre-edit original — avoids a dead undo entry on unchanged text.
        if new_val != original {
            doc.set_edge_label(&edge_ref, new_val);
        }
    }
    EditRefresh::All
}

/// Transition into inline label edit mode for the given edge. Seeds
/// the buffer from the edge's current label (or the empty string)
/// and installs a preview override on the renderer so the caret
/// shows up immediately. If the edge isn't in `doc.mindmap.edges`
/// any more (e.g. an undo popped it between selection and the open
/// gesture), logs via `log::warn!` and returns without mutating
/// state — callers read `label_edit_state.is_open()` to detect
/// this case.
///
/// `clean` opens on an empty buffer instead of the existing label
/// — `Action::EditSelectionClean`'s documented contract, and the
/// single-line counterpart of `open_text_edit`'s `from_creation`.
/// `original` still holds the pre-edit label either way, so Esc
/// restores it and an unchanged commit still skips the undo push.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn open_label_edit(
    edge_ref: &crate::application::document::EdgeRef,
    clean: bool,
    doc: &mut MindMapDocument,
    label_edit_state: &mut LabelEditState,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    if open_label_edit_core(edge_ref, clean, doc, label_edit_state) == EditRefresh::None {
        return;
    }
    // Rebuild the connection-label canvas so the caret is visible
    // immediately. The caller already ran `rebuild_all` before this,
    // so the rest of the canvas is fresh. We deliberately touch only
    // the label role — `label_edit_preview` reaches no other pass,
    // and portal-mode edges have no line label to preview, so the
    // portal canvas cannot change on a label-edit keystroke. Keeping
    // the other roles out of the hot path is the core "fast label
    // typing" fix. Resize-handle overrides are irrelevant here, so
    // this passes `none()` rather than threading mode through every
    // caller.
    let offsets = std::collections::HashMap::new();
    CanvasFrame::new(
        doc,
        &offsets,
        crate::application::document::InteractionModeOverrides::none(),
        renderer.camera_zoom(),
    )
    .update_connection_label_tree(app_scene);
}

/// Route a keystroke to the inline label editor. Cancel and commit
/// are resolved through `action_for_context(InputContext::LabelEdit)`;
/// navigation and character input stay as direct key checks (they
/// are structural text-editing primitives, not rebindable actions).
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn handle_label_edit_key(
    key_name: &Option<String>,
    logical_key: &Key,
    ctrl: bool,
    shift: bool,
    alt: bool,
    keybinds: &ResolvedKeybinds,
    label_edit_state: &mut LabelEditState,
    doc: &mut MindMapDocument,
    _mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    _scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let name = key_name.as_deref();
    let action = name.and_then(|n| keybinds.action_for_context(InputContext::LabelEdit, n, ctrl, shift, alt));
    // `LabelEditCommit` / `LabelEditCancel` are funneled via the
    // keyboard handler's pre-filter. This handler reaches only the
    // literal-Key character + cursor primitive paths.
    //
    // Action-driven path: cursor / delete primitives route through
    // `dispatch::apply_label_edit_action`. Falls through to
    // `route_label_edit_key` for character input not bound to an
    // Action.
    let input = match action {
        Some(a) => EditInput::Action(a),
        None => EditInput::Key(logical_key),
    };
    if handle_label_edit_key_core(input, label_edit_state, doc) == EditRefresh::Preview {
        // Same scope as `open_label_edit`: only the connection-
        // label canvas reads `label_edit_preview`, so every other
        // role stays untouched on every keystroke. Resize-handle
        // overrides are irrelevant, so pass `none()` rather than
        // threading mode through every caller.
        let offsets = std::collections::HashMap::new();
        CanvasFrame::new(
            doc,
            &offsets,
            crate::application::document::InteractionModeOverrides::none(),
            renderer.camera_zoom(),
        )
        .update_connection_label_tree(app_scene);
    }
}

/// Close the inline label editor. If `commit` is true, writes the
/// current buffer into the edge's label (via
/// `document.set_edge_label`) and pushes an undo entry. If false,
/// restores the pre-edit label (equivalent to discarding the buffer)
/// — no undo push because we never mutated the model during typing.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn close_label_edit(
    commit: bool,
    doc: &mut MindMapDocument,
    interaction_mode: &super::InteractionMode,
    label_edit_state: &mut LabelEditState,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    if close_label_edit_core(commit, doc, label_edit_state) == EditRefresh::None {
        return;
    }
    // Rebuild so the label reflects the model state (or vanishes if
    // the buffer was empty + original was None).
    rebuild_all(
        doc,
        interaction_mode,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
    );
}

/// Inline-edit state for a portal label's text. Parallel to
/// [`LabelEditState`] but keyed to `(edge_ref, endpoint_node_id)`
/// — portal labels are per-endpoint, so the editor needs both
/// parts of the identity. Routes keystrokes through
/// `route_label_edit_key` just like the edge-label editor so
/// grapheme semantics (emoji / ZWJ backspace, arrow-key walking)
/// behave identically.
///
/// Mutually exclusive with `LabelEditState`: the event loop's
/// keystroke routing checks the portal editor first (rarer form
/// so the short-circuit is usually cheap), then falls through
/// to the edge-label editor.
#[derive(Debug, Clone)]
pub(in crate::application::app) enum PortalTextEditState {
    Closed,
    Open {
        edge_ref: crate::application::document::EdgeRef,
        endpoint_node_id: String,
        /// The in-progress buffer. Committed to
        /// `PortalEndpointState.text` on Enter; discarded on
        /// Escape.
        buffer: String,
        /// Cursor position as a grapheme-cluster index into
        /// `buffer`. Same invariant as `LabelEditState::Open`.
        cursor_grapheme_pos: usize,
        /// The endpoint's text value at the moment edit mode
        /// opened. Used to restore state on Escape and to skip
        /// undo entries on unchanged commit.
        original: Option<String>,
    },
}

impl PortalTextEditState {
    pub(in crate::application::app) fn is_open(&self) -> bool {
        matches!(self, PortalTextEditState::Open { .. })
    }

    /// Borrow the `(edge_ref, endpoint_node_id)` currently under
    /// edit, if any. Mirrors [`LabelEditState::edited_edge_ref`]
    /// — the click-outside-to-commit check needs both the edge
    /// identity and the endpoint so it can compare against the
    /// `PortalHit` that
    /// [`AppScene::portal_at`](crate::application::scene_host::AppScene::portal_at)
    /// returns.
    pub(in crate::application::app) fn edited_endpoint(
        &self,
    ) -> Option<(&crate::application::document::EdgeRef, &str)> {
        match self {
            PortalTextEditState::Open {
                edge_ref,
                endpoint_node_id,
                ..
            } => Some((edge_ref, endpoint_node_id.as_str())),
            PortalTextEditState::Closed => None,
        }
    }
}

/// Renderer-free half of [`open_portal_text_edit`].
#[cfg(not(target_arch = "wasm32"))]
fn open_portal_text_edit_core(
    edge_ref: &crate::application::document::EdgeRef,
    endpoint_node_id: &str,
    clean: bool,
    doc: &mut MindMapDocument,
    state: &mut PortalTextEditState,
) -> EditRefresh {
    // Verify the edge + endpoint still exist before entering
    // edit mode. If either vanished between the selection and
    // the open gesture (e.g. an undo raced with EditSelection),
    // log and return rather than install a stale editor.
    let edge = match doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)) {
        Some(e) => e,
        None => {
            log::warn!(
                "open_portal_text_edit: edge {}→{} ({}) not found; editor stays closed",
                edge_ref.from_id,
                edge_ref.to_id,
                edge_ref.edge_type
            );
            return EditRefresh::None;
        }
    };
    if endpoint_node_id != edge.from_id && endpoint_node_id != edge.to_id {
        log::warn!(
            "open_portal_text_edit: endpoint {} is neither from ({}) nor to ({}); editor stays closed",
            endpoint_node_id,
            edge.from_id,
            edge.to_id,
        );
        return EditRefresh::None;
    }
    let original =
        baumhard::mindmap::model::portal_endpoint_state(edge, endpoint_node_id).and_then(|s| s.text.clone());
    let (buffer, cursor_grapheme_pos) = seed_edit_buffer(clean, original.as_deref());
    *state = PortalTextEditState::Open {
        edge_ref: edge_ref.clone(),
        endpoint_node_id: endpoint_node_id.to_string(),
        buffer: buffer.clone(),
        cursor_grapheme_pos,
        original,
    };
    let edge_key = baumhard::mindmap::scene_cache::EdgeKey::from(edge_ref);
    doc.portal_text_edit_preview = Some((
        edge_key,
        endpoint_node_id.to_string(),
        insert_caret(&buffer, cursor_grapheme_pos),
    ));
    EditRefresh::Preview
}

/// Renderer-free half of [`handle_portal_text_edit_key`], including
/// the mid-edit `edge_still_valid` guard.
#[cfg(not(target_arch = "wasm32"))]
fn handle_portal_text_edit_key_core(
    input: EditInput<'_>,
    state: &mut PortalTextEditState,
    doc: &mut MindMapDocument,
) -> EditRefresh {
    // If the edge disappeared or flipped out of portal mode
    // while the editor was open (e.g. an Undo popped the edge,
    // or a console command switched `display_mode` to line),
    // close the editor without committing. Without this guard a
    // commit would silently no-op via `set_portal_label_text`'s
    // edge-not-found path, and the preview would keep re-
    // installing a dangling buffer on every keystroke.
    let edge_still_valid = match state {
        PortalTextEditState::Open {
            edge_ref,
            endpoint_node_id,
            ..
        } => doc
            .mindmap
            .edges
            .iter()
            .find(|e| edge_ref.matches(e))
            .map(|e| {
                baumhard::mindmap::model::is_portal_edge(e)
                    && (endpoint_node_id == &e.from_id || endpoint_node_id == &e.to_id)
            })
            .unwrap_or(false),
        PortalTextEditState::Closed => return EditRefresh::None,
    };
    if !edge_still_valid {
        return close_portal_text_edit_core(false, doc, state);
    }

    let changed = {
        let Some((buffer, cursor)) = (match state {
            PortalTextEditState::Open {
                buffer,
                cursor_grapheme_pos,
                ..
            } => Some((buffer, cursor_grapheme_pos)),
            PortalTextEditState::Closed => None,
        }) else {
            return EditRefresh::None;
        };
        match input {
            EditInput::Action(a) => {
                crate::application::app::dispatch::apply_label_edit_action_to_buffer(a, buffer, cursor)
            }
            EditInput::Key(logical_key) => route_label_edit_key(logical_key, buffer, cursor),
        }
    };
    if !changed {
        return EditRefresh::None;
    }

    if let PortalTextEditState::Open {
        edge_ref,
        endpoint_node_id,
        buffer,
        cursor_grapheme_pos,
        ..
    } = state
    {
        let edge_key = baumhard::mindmap::scene_cache::EdgeKey::from(&*edge_ref);
        doc.portal_text_edit_preview = Some((
            edge_key,
            endpoint_node_id.clone(),
            insert_caret(buffer, *cursor_grapheme_pos),
        ));
        return EditRefresh::Preview;
    }
    EditRefresh::None
}

/// Renderer-free half of [`close_portal_text_edit`].
#[cfg(not(target_arch = "wasm32"))]
fn close_portal_text_edit_core(
    commit: bool,
    doc: &mut MindMapDocument,
    state: &mut PortalTextEditState,
) -> EditRefresh {
    let (edge_ref, endpoint_node_id, buffer, original) =
        match std::mem::replace(state, PortalTextEditState::Closed) {
            PortalTextEditState::Open {
                edge_ref,
                endpoint_node_id,
                buffer,
                original,
                ..
            } => (edge_ref, endpoint_node_id, buffer, original),
            PortalTextEditState::Closed => return EditRefresh::None,
        };
    doc.portal_text_edit_preview = None;
    if commit {
        let new_val = if buffer.is_empty() { None } else { Some(buffer) };
        if new_val != original {
            doc.set_portal_label_text(&edge_ref, &endpoint_node_id, new_val);
        }
    }
    EditRefresh::All
}

/// Transition into inline portal-text edit mode for the given
/// endpoint. Seeds the buffer from the endpoint's current text,
/// installs a preview override on the document so the caret
/// shows up immediately, and runs a portal-tree update so the
/// caret renders on the next frame.
///
/// `clean` opens on an empty buffer instead of the endpoint's
/// existing text — same contract as [`open_label_edit`]'s `clean`.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn open_portal_text_edit(
    edge_ref: &crate::application::document::EdgeRef,
    endpoint_node_id: &str,
    clean: bool,
    doc: &mut MindMapDocument,
    state: &mut PortalTextEditState,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    // Callers read `state.is_open()` to detect the skipped-open
    // case; they don't need a return value to disambiguate.
    if open_portal_text_edit_core(edge_ref, endpoint_node_id, clean, doc, state) == EditRefresh::None {
        return;
    }
    let offsets = std::collections::HashMap::new();
    CanvasFrame::new(
        doc,
        &offsets,
        crate::application::document::InteractionModeOverrides::none(),
        renderer.camera_zoom(),
    )
    .update_portal_tree(app_scene);
}

/// Route a keystroke to the inline portal-text editor. Mirrors
/// `handle_label_edit_key` — commit / cancel resolve through
/// `InputContext::LabelEdit` (shared with the edge-label editor
/// since the two editors use the same key semantics) and
/// navigation / character input go through the shared
/// `route_label_edit_key` router.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn handle_portal_text_edit_key(
    key_name: &Option<String>,
    logical_key: &Key,
    ctrl: bool,
    shift: bool,
    alt: bool,
    keybinds: &ResolvedKeybinds,
    state: &mut PortalTextEditState,
    doc: &mut MindMapDocument,
    interaction_mode: &super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let name = key_name.as_deref();
    let action = name.and_then(|n| keybinds.action_for_context(InputContext::LabelEdit, n, ctrl, shift, alt));

    // `LabelEditCommit` / `LabelEditCancel` are funneled via the
    // keyboard handler's pre-filter. The portal-text editor reuses
    // `LabelEdit*` Actions (it shares `InputContext::LabelEdit`);
    // the dispatch arm picks the open state. The edge-validity
    // guard inside the core stays out of the funnel (pre-funnel
    // state-machine bookkeeping per CODE_CONVENTIONS §3 carve-out).
    let input = match action {
        Some(a) => EditInput::Action(a),
        None => EditInput::Key(logical_key),
    };
    match handle_portal_text_edit_key_core(input, state, doc) {
        EditRefresh::None => {}
        EditRefresh::Preview => {
            let offsets = std::collections::HashMap::new();
            CanvasFrame::new(
                doc,
                &offsets,
                crate::application::document::InteractionModeOverrides::none(),
                renderer.camera_zoom(),
            )
            .update_portal_tree(app_scene);
        }
        // The `edge_still_valid` guard tripped and the core closed
        // the editor without committing.
        EditRefresh::All => {
            rebuild_all(
                doc,
                interaction_mode,
                mindmap_tree,
                app_scene,
                renderer,
                scene_cache,
            );
        }
    }
}

/// Close the inline portal-text editor. If `commit` is true,
/// writes the buffer to `PortalEndpointState.text` (with undo
/// entry) when the value actually differs from the pre-edit
/// original. Restores the pre-edit state on cancel.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn close_portal_text_edit(
    commit: bool,
    doc: &mut MindMapDocument,
    interaction_mode: &super::InteractionMode,
    state: &mut PortalTextEditState,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    if close_portal_text_edit_core(commit, doc, state) == EditRefresh::None {
        return;
    }
    rebuild_all(
        doc,
        interaction_mode,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
    );
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    //! Label-edit key-routing tests — backspace / delete / arrow /
    //! home / end / printable-char behavior for
    //! [`super::route_label_edit_key`], the pure keyboard router
    //! both the label editor and its text-edit sibling consume.
    //! No winit event loop needed; the router is a pure function.

    use super::*;
    use crate::application::app::dispatch::apply_label_edit_action_to_buffer;
    use crate::application::document::{EdgeLabelSel, EdgeRef, PortalLabelSel, SectionSel, SelectionState};
    use crate::application::keybinds::Action;
    use crate::application::platform::input::{Key, SmolStr};

    fn ch(s: &str) -> Key {
        Key::Character(SmolStr::new(s))
    }

    // ── Selection → single-line editor routing ──────────────────
    //
    // One resolver behind `Action::EditSelection*`,
    // `Action::LabelEditOnSelection` and the type-to-edit path;
    // these pin the mapping the three used to each re-derive.

    #[test]
    fn test_resolve_edge_editor_plan_routes_edge_label_to_the_label_editor() {
        let er = EdgeRef::new("a", "b", "cross_link");
        let sel = SelectionState::EdgeLabel(EdgeLabelSel::new(er.clone()));
        assert_eq!(
            resolve_edge_editor_plan(&sel),
            EdgeEditorPlan::Label { edge_ref: er }
        );
    }

    /// `PortalLabel` and `PortalText` are two selections on the same
    /// endpoint (its glyph and its caption) and both edit the
    /// caption, so both route to the portal-text editor.
    #[test]
    fn test_resolve_edge_editor_plan_routes_both_portal_selections_to_the_portal_editor() {
        let er = EdgeRef::new("a", "b", "portal");
        let expected = EdgeEditorPlan::PortalText {
            edge_ref: er.clone(),
            endpoint_node_id: "a".to_string(),
        };
        let label_sel = PortalLabelSel {
            edge_key: baumhard::mindmap::scene_cache::EdgeKey::from(&er),
            endpoint_node_id: "a".to_string(),
        };
        assert_eq!(
            resolve_edge_editor_plan(&SelectionState::PortalLabel(label_sel.clone())),
            expected
        );
        assert_eq!(
            resolve_edge_editor_plan(&SelectionState::PortalText(label_sel)),
            expected
        );
    }

    /// Node-scoped and empty selections belong to the node text
    /// editor (or to nothing) — the single-line editors decline.
    /// All seven non-editor variants, so the list plus the three
    /// editor cases above covers every `SelectionState` arm.
    #[test]
    fn test_resolve_edge_editor_plan_declines_non_edge_selections() {
        for sel in [
            SelectionState::None,
            SelectionState::Single("a".into()),
            SelectionState::Multi(vec!["a".into(), "b".into()]),
            SelectionState::Section(SectionSel::new("a", 0)),
            SelectionState::MultiSection(vec![SectionSel::new("a", 0), SectionSel::new("b", 1)]),
            SelectionState::SectionRange {
                sel: SectionSel::new("a", 0),
                range: (0, 2),
            },
            SelectionState::Edge(EdgeRef::new("a", "b", "cross_link")),
        ] {
            assert_eq!(
                resolve_edge_editor_plan(&sel),
                EdgeEditorPlan::None,
                "{:?} must not open a single-line editor",
                sel
            );
        }
    }

    // ── `clean` buffer seeding ──────────────────────────────────
    //
    // `Action::EditSelectionClean`'s empty-buffer contract. Before
    // the fix the dispatch arm computed the flag into `let _clean`
    // and threw it away, so the contract held on nodes and silently
    // didn't on edge labels / portal endpoints.

    #[test]
    fn test_seed_edit_buffer_clean_opens_empty_with_the_cursor_at_zero() {
        assert_eq!(seed_edit_buffer(true, Some("existing label")), (String::new(), 0));
    }

    #[test]
    fn test_seed_edit_buffer_non_clean_seeds_existing_text_cursor_at_end() {
        assert_eq!(seed_edit_buffer(false, Some("café")), ("café".to_string(), 4));
    }

    /// The cursor is counted in grapheme clusters, so an existing
    /// label ending in a ZWJ emoji sequence puts the caret after
    /// the whole cluster rather than mid-sequence.
    #[test]
    fn test_seed_edit_buffer_counts_the_cursor_in_grapheme_clusters() {
        let family = "a\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(seed_edit_buffer(false, Some(family)), (family.to_string(), 2));
    }

    /// An unlabeled edge opens on an empty buffer either way — the
    /// `clean` path differs only when there is text to suppress.
    #[test]
    fn test_seed_edit_buffer_handles_a_missing_original() {
        assert_eq!(seed_edit_buffer(false, None), (String::new(), 0));
        assert_eq!(seed_edit_buffer(true, None), (String::new(), 0));
    }

    // Structural-key behavior (Backspace / Delete / Arrow*/Home/End)
    // moved from `route_label_edit_key` to
    // `dispatch::apply_label_edit_action_to_buffer` in Phase 5. Tests
    // migrated alongside.

    #[test]
    fn test_label_edit_backspace_deletes_grapheme_before_cursor() {
        let mut buf = String::from("café");
        let mut cursor = 4;
        let changed = apply_label_edit_action_to_buffer(Action::LabelEditDeleteBack, &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "caf");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_label_edit_backspace_at_zero_is_noop() {
        let mut buf = String::from("abc");
        let mut cursor = 0;
        let changed = apply_label_edit_action_to_buffer(Action::LabelEditDeleteBack, &mut buf, &mut cursor);
        assert!(!changed);
        assert_eq!(buf, "abc");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn test_label_edit_delete_at_end_is_noop() {
        let mut buf = String::from("abc");
        let mut cursor = 3;
        let changed =
            apply_label_edit_action_to_buffer(Action::LabelEditDeleteForward, &mut buf, &mut cursor);
        assert!(!changed);
        assert_eq!(buf, "abc");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_label_edit_delete_removes_grapheme_at_cursor() {
        let mut buf = String::from("abc");
        let mut cursor = 1;
        let changed =
            apply_label_edit_action_to_buffer(Action::LabelEditDeleteForward, &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "ac");
        assert_eq!(cursor, 1);
    }

    #[test]
    fn test_label_edit_arrow_left_right_walks_graphemes() {
        let mut buf = String::from("café");
        let mut cursor = 4;
        assert!(apply_label_edit_action_to_buffer(
            Action::LabelEditCursorLeft,
            &mut buf,
            &mut cursor
        ));
        assert_eq!(cursor, 3);
        assert!(apply_label_edit_action_to_buffer(
            Action::LabelEditCursorLeft,
            &mut buf,
            &mut cursor
        ));
        assert_eq!(cursor, 2);
        assert!(apply_label_edit_action_to_buffer(
            Action::LabelEditCursorRight,
            &mut buf,
            &mut cursor
        ));
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_label_edit_arrow_left_at_zero_is_noop() {
        let mut buf = String::from("abc");
        let mut cursor = 0;
        assert!(!apply_label_edit_action_to_buffer(
            Action::LabelEditCursorLeft,
            &mut buf,
            &mut cursor
        ));
        assert_eq!(cursor, 0);
    }

    #[test]
    fn test_label_edit_home_end_jump_to_ends() {
        let mut buf = String::from("café");
        let mut cursor = 2;
        assert!(apply_label_edit_action_to_buffer(
            Action::LabelEditCursorHome,
            &mut buf,
            &mut cursor
        ));
        assert_eq!(cursor, 0);
        assert!(!apply_label_edit_action_to_buffer(
            Action::LabelEditCursorHome,
            &mut buf,
            &mut cursor
        ));
        assert_eq!(cursor, 0);
        assert!(apply_label_edit_action_to_buffer(
            Action::LabelEditCursorEnd,
            &mut buf,
            &mut cursor
        ));
        assert_eq!(cursor, 4);
        assert!(!apply_label_edit_action_to_buffer(
            Action::LabelEditCursorEnd,
            &mut buf,
            &mut cursor
        ));
        assert_eq!(cursor, 4);
    }

    #[test]
    fn test_route_label_edit_printable_inserts_and_advances() {
        let mut buf = String::from("ab");
        let mut cursor = 1;
        let changed = route_label_edit_key(&ch("X"), &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "aXb");
        assert_eq!(cursor, 2);
    }

    /// IME / dead-key sequences can arrive as multi-char strings.
    /// The cursor advances by visible grapheme clusters, not by
    /// codepoints.
    #[test]
    fn test_route_label_edit_multichar_typed_payload() {
        let mut buf = String::from("");
        let mut cursor = 0;
        let changed = route_label_edit_key(&ch("né"), &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "né");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn test_route_label_edit_ime_payload_advances_by_grapheme_delta() {
        let mut buf = String::from("");
        let mut cursor = 0;
        let jamo = "\u{1112}\u{1161}\u{11AB}";
        let changed = route_label_edit_key(&ch(jamo), &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, jamo);
        assert_eq!(cursor, 1);
    }

    #[test]
    fn test_route_label_edit_combining_mark_merge_does_not_overadvance_cursor() {
        let mut buf = String::from("e");
        let mut cursor = 1;
        let changed = route_label_edit_key(&ch("\u{0301}"), &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "e\u{0301}");
        assert_eq!(cursor, 1);
    }

    /// Control characters in a typed payload are filtered out.
    /// Pins the regression where an IME sequence like `"a\t"`
    /// would otherwise insert a literal tab.
    #[test]
    fn test_route_label_edit_typed_control_chars_are_skipped() {
        let mut buf = String::from("");
        let mut cursor = 0;
        let changed = route_label_edit_key(&ch("a\tb"), &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "ab");
        assert_eq!(cursor, 2);
    }

    /// After Phase 5 the structural keys (Backspace / Delete /
    /// arrows / Home / End) flow through
    /// `dispatch::apply_label_edit_action_to_buffer`, not the
    /// router. The router now ignores all `Key::Named` events; only
    /// `Key::Character` printable payloads are inserted. This pins
    /// the new contract — a future regression that re-introduces
    /// `Key::Named(Backspace) → delete` would leak the structural
    /// behavior past the action layer and let users no longer
    /// disable the key by clearing the binding.
    #[test]
    fn test_route_named_key_is_noop_post_phase_5() {
        use crate::application::platform::input::NamedKey;
        let mut buf = String::from("abc");
        let mut cursor = 3;
        let changed = route_label_edit_key(&Key::Named(NamedKey::Backspace), &mut buf, &mut cursor);
        assert!(!changed);
        assert_eq!(buf, "abc");
        assert_eq!(cursor, 3);

        let changed = route_label_edit_key(&Key::Named(NamedKey::Delete), &mut buf, &mut cursor);
        assert!(!changed);
        assert_eq!(buf, "abc");
        assert_eq!(cursor, 3);
    }

    /// Unprintable winit `Key` variants (dead keys, identifier-only,
    /// modifiers reaching the dispatcher) must not insert anything
    /// and must not panic.
    #[test]
    fn test_route_unhandled_key_is_noop() {
        use crate::application::platform::input::NamedKey;
        let mut buf = String::from("abc");
        let mut cursor = 1;
        let changed = route_label_edit_key(&Key::Named(NamedKey::Shift), &mut buf, &mut cursor);
        assert!(!changed);
        assert_eq!(buf, "abc");
        assert_eq!(cursor, 1);
    }

    // ── Differential oracle over the editor lifecycle ───────────
    //
    // Every scenario below drives a *renderer-free* editor core
    // over a scripted input sequence and renders one line per step
    // describing everything the step could observably change:
    // the refresh decree, the editor's buffer + grapheme cursor,
    // the target's committed model value, the staged preview
    // string, and the undo-stack depth.
    //
    // The expected traces are the oracle. They were authored
    // against the two separate `LabelEditState` /
    // `PortalTextEditState` implementations; a refactor that
    // unifies them has to reproduce them line for line.
    mod oracle {
        use super::super::*;
        use crate::application::document::{EdgeRef, MindMapDocument};
        use crate::application::keybinds::Action;
        use crate::application::platform::input::{Key, SmolStr};

        const FROM_ID: &str = "node-a";
        const TO_ID: &str = "node-b";
        const EDGE_TYPE: &str = "cross_link";
        /// Pre-edit text on both targets, so an edge-label trace and
        /// a portal-text trace over the same script are directly
        /// comparable.
        const SEED_TEXT: &str = "hi";

        /// Which single-line editor a scenario drives.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum TargetKind {
            EdgeLabel,
            PortalText,
        }

        /// One scripted step. `DeleteEdge` / `FlipToLine` are
        /// environment mutations that model something happening
        /// *underneath* an open editor (an undo popping the edge, a
        /// console command flipping `display_mode`).
        #[derive(Debug, Clone)]
        enum Step {
            Open { clean: bool },
            Type(&'static str),
            Act(Action),
            DeleteEdge,
            FlipToLine,
            Close { commit: bool },
        }

        fn edge_ref() -> EdgeRef {
            EdgeRef::new(FROM_ID, TO_ID, EDGE_TYPE)
        }

        fn key(s: &str) -> Key {
            Key::Character(SmolStr::new(s))
        }

        /// Two nodes plus one edge carrying [`SEED_TEXT`] on the
        /// field the `kind`'s editor owns.
        fn fixture_doc(kind: TargetKind) -> MindMapDocument {
            use crate::application::document::defaults::{default_cross_link_edge, default_orphan_node};
            use glam::Vec2;

            let mut edge = default_cross_link_edge(FROM_ID, TO_ID);
            match kind {
                TargetKind::EdgeLabel => edge.label = Some(SEED_TEXT.to_string()),
                TargetKind::PortalText => {
                    edge.display_mode = Some(baumhard::mindmap::model::DISPLAY_MODE_PORTAL.to_string());
                    edge.portal_from = Some(baumhard::mindmap::model::PortalEndpointState {
                        text: Some(SEED_TEXT.to_string()),
                        ..Default::default()
                    });
                }
            }
            let json = serde_json::json!({
                "version": "1.0",
                "name": "oracle",
                "canvas": {"background_color": "#000000"},
                "nodes": {
                    FROM_ID: default_orphan_node(FROM_ID, Vec2::new(0.0, 0.0)),
                    TO_ID: default_orphan_node(TO_ID, Vec2::new(400.0, 0.0)),
                },
                "edges": [edge],
            })
            .to_string();
            MindMapDocument::from_json_str(&json, None).expect("oracle fixture JSON must parse")
        }

        /// The target's committed model value.
        fn model_value(kind: TargetKind, doc: &MindMapDocument) -> Option<String> {
            let edge = doc.mindmap.edges.first()?;
            match kind {
                TargetKind::EdgeLabel => edge.label.clone(),
                TargetKind::PortalText => edge.portal_from.as_ref().and_then(|s| s.text.clone()),
            }
        }

        /// The preview string staged for the renderer, if any.
        fn preview(kind: TargetKind, doc: &MindMapDocument) -> Option<String> {
            match kind {
                TargetKind::EdgeLabel => doc.label_edit_preview.as_ref().map(|(_, s)| s.clone()),
                TargetKind::PortalText => doc.portal_text_edit_preview.as_ref().map(|(_, _, s)| s.clone()),
            }
        }

        fn render(
            refresh: EditRefresh,
            editor: Option<(&str, usize)>,
            value: Option<String>,
            prev: Option<String>,
            undo: usize,
        ) -> String {
            let editor = match editor {
                Some((buf, cur)) => format!("open[{}]@{}", buf, cur),
                None => "closed".to_string(),
            };
            // Rendered by hand rather than through `{:?}` on the
            // `Option<String>`s: `Debug` escapes ZWJ / combining
            // marks, which would make a grapheme trace unreadable
            // in exactly the case it exists to pin.
            let opt = |v: Option<String>| match v {
                Some(s) => format!("\"{}\"", s),
                None => "-".to_string(),
            };
            format!(
                "{:?} {} value={} preview={} undo={}",
                refresh,
                editor,
                opt(value),
                opt(prev),
                undo
            )
        }

        /// Drive `script` against `kind`'s editor core, one trace
        /// line per step.
        fn run(kind: TargetKind, script: &[Step]) -> Vec<String> {
            let mut doc = fixture_doc(kind);
            let mut label = LabelEditState::Closed;
            let mut portal = PortalTextEditState::Closed;
            let mut trace = Vec::new();
            for step in script {
                let refresh = match step {
                    Step::DeleteEdge => {
                        doc.mindmap.edges.clear();
                        EditRefresh::None
                    }
                    Step::FlipToLine => {
                        for e in doc.mindmap.edges.iter_mut() {
                            e.display_mode = None;
                        }
                        EditRefresh::None
                    }
                    Step::Open { clean } => match kind {
                        TargetKind::EdgeLabel => {
                            open_label_edit_core(&edge_ref(), *clean, &mut doc, &mut label)
                        }
                        TargetKind::PortalText => {
                            open_portal_text_edit_core(&edge_ref(), FROM_ID, *clean, &mut doc, &mut portal)
                        }
                    },
                    Step::Type(s) => {
                        let k = key(s);
                        match kind {
                            TargetKind::EdgeLabel => {
                                handle_label_edit_key_core(EditInput::Key(&k), &mut label, &mut doc)
                            }
                            TargetKind::PortalText => {
                                handle_portal_text_edit_key_core(EditInput::Key(&k), &mut portal, &mut doc)
                            }
                        }
                    }
                    Step::Act(a) => match kind {
                        TargetKind::EdgeLabel => {
                            handle_label_edit_key_core(EditInput::Action(a.clone()), &mut label, &mut doc)
                        }
                        TargetKind::PortalText => handle_portal_text_edit_key_core(
                            EditInput::Action(a.clone()),
                            &mut portal,
                            &mut doc,
                        ),
                    },
                    Step::Close { commit } => match kind {
                        TargetKind::EdgeLabel => close_label_edit_core(*commit, &mut doc, &mut label),
                        TargetKind::PortalText => close_portal_text_edit_core(*commit, &mut doc, &mut portal),
                    },
                };
                let editor = match kind {
                    TargetKind::EdgeLabel => match &label {
                        LabelEditState::Open {
                            buffer,
                            cursor_grapheme_pos,
                            ..
                        } => Some((buffer.as_str(), *cursor_grapheme_pos)),
                        LabelEditState::Closed => None,
                    },
                    TargetKind::PortalText => match &portal {
                        PortalTextEditState::Open {
                            buffer,
                            cursor_grapheme_pos,
                            ..
                        } => Some((buffer.as_str(), *cursor_grapheme_pos)),
                        PortalTextEditState::Closed => None,
                    },
                };
                trace.push(render(
                    refresh,
                    editor,
                    model_value(kind, &doc),
                    preview(kind, &doc),
                    doc.undo_stack.len(),
                ));
            }
            trace
        }

        /// Assert a script produces `expected` on **both** editors.
        /// Every scenario the two editors are supposed to agree on
        /// goes through here, so the pair is pinned as one editor
        /// even while two implementations exist.
        fn assert_both(script: &[Step], expected: &[&str]) {
            for kind in [TargetKind::EdgeLabel, TargetKind::PortalText] {
                assert_eq!(
                    run(kind, script),
                    expected,
                    "{:?} diverged from the shared single-line editor trace",
                    kind
                );
            }
        }

        #[test]
        fn test_oracle_open_type_commit() {
            assert_both(
                &[
                    Step::Open { clean: false },
                    Step::Type("!"),
                    Step::Close { commit: true },
                ],
                &[
                    r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                    r#"Preview open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                    r#"All closed value="hi!" preview=- undo=1"#,
                ],
            );
        }

        #[test]
        fn test_oracle_open_type_cancel_restores_and_pushes_no_undo() {
            assert_both(
                &[
                    Step::Open { clean: false },
                    Step::Type("!"),
                    Step::Close { commit: false },
                ],
                &[
                    r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                    r#"Preview open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                    r#"All closed value="hi" preview=- undo=0"#,
                ],
            );
        }

        /// `Action::EditSelectionClean`'s empty-buffer contract all
        /// the way through commit.
        #[test]
        fn test_oracle_clean_open_replaces_the_value() {
            assert_both(
                &[
                    Step::Open { clean: true },
                    Step::Type("x"),
                    Step::Close { commit: true },
                ],
                &[
                    r#"Preview open[]@0 value="hi" preview="|" undo=0"#,
                    r#"Preview open[x]@1 value="hi" preview="x|" undo=0"#,
                    r#"All closed value="x" preview=- undo=1"#,
                ],
            );
        }

        /// Cursor / delete primitives arriving as resolved Actions
        /// move the same buffer the literal-key path does.
        #[test]
        fn test_oracle_cursor_primitives() {
            assert_both(
                &[
                    Step::Open { clean: false },
                    Step::Act(Action::LabelEditCursorLeft),
                    Step::Act(Action::LabelEditDeleteBack),
                    Step::Close { commit: true },
                ],
                &[
                    r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                    r#"Preview open[hi]@1 value="hi" preview="h|i" undo=0"#,
                    r#"Preview open[i]@0 value="hi" preview="|i" undo=0"#,
                    r#"All closed value="i" preview=- undo=1"#,
                ],
            );
        }

        /// An unchanged commit must not leave a dead undo entry.
        #[test]
        fn test_oracle_unchanged_commit_pushes_no_undo() {
            assert_both(
                &[Step::Open { clean: false }, Step::Close { commit: true }],
                &[
                    r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                    r#"All closed value="hi" preview=- undo=0"#,
                ],
            );
        }

        /// Committing an empty buffer clears the model value (both
        /// setters normalize `Some("")` to `None`).
        #[test]
        fn test_oracle_empty_commit_clears_the_value() {
            assert_both(
                &[Step::Open { clean: true }, Step::Close { commit: true }],
                &[
                    r#"Preview open[]@0 value="hi" preview="|" undo=0"#,
                    r#"All closed value=- preview=- undo=1"#,
                ],
            );
        }

        /// A ZWJ family sequence is one grapheme: the cursor
        /// advances by 1, not by the codepoint count.
        #[test]
        fn test_oracle_zwj_cluster_advances_one_grapheme() {
            assert_both(
                &[
                    Step::Open { clean: false },
                    Step::Type("\u{1F469}\u{200D}\u{1F467}"),
                    Step::Close { commit: true },
                ],
                &[
                    r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                    "Preview open[hi\u{1F469}\u{200D}\u{1F467}]@3 value=\"hi\" \
                     preview=\"hi\u{1F469}\u{200D}\u{1F467}|\" undo=0",
                    "All closed value=\"hi\u{1F469}\u{200D}\u{1F467}\" preview=- undo=1",
                ],
            );
        }

        /// The edge is deleted underneath an open editor and the
        /// user then commits. Both editors keep the buffer, both
        /// find nothing to write, and neither pushes an undo entry.
        #[test]
        fn test_oracle_edge_deleted_underneath_then_commit() {
            assert_both(
                &[
                    Step::Open { clean: false },
                    Step::Type("!"),
                    Step::DeleteEdge,
                    Step::Close { commit: true },
                ],
                &[
                    r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                    r#"Preview open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                    r#"None open[hi!]@3 value=- preview="hi!|" undo=0"#,
                    r#"All closed value=- preview=- undo=0"#,
                ],
            );
        }

        /// **The one intentional behavioral asymmetry.** A keystroke
        /// arriving after the edge vanished trips the portal
        /// editor's `edge_still_valid` guard — it closes without
        /// committing, so the following commit is a no-op on a
        /// closed editor. The edge-label editor has no such guard
        /// and keeps typing into a buffer whose target is gone;
        /// its commit then no-ops inside `set_edge_label`.
        ///
        /// Unifying the two editors must preserve *both* columns.
        /// Deleting the guard makes the portal column look like the
        /// edge-label one, which is exactly what this pins.
        #[test]
        fn test_oracle_keystroke_after_edge_deleted_diverges_by_design() {
            let script = [
                Step::Open { clean: false },
                Step::Type("!"),
                Step::DeleteEdge,
                Step::Type("?"),
                Step::Close { commit: true },
            ];
            let shared_prefix = [
                r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                r#"Preview open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                r#"None open[hi!]@3 value=- preview="hi!|" undo=0"#,
            ];

            let mut edge_label = shared_prefix.to_vec();
            edge_label.push(r#"Preview open[hi!?]@4 value=- preview="hi!?|" undo=0"#);
            edge_label.push(r#"All closed value=- preview=- undo=0"#);
            assert_eq!(run(TargetKind::EdgeLabel, &script), edge_label);

            let mut portal = shared_prefix.to_vec();
            portal.push(r#"All closed value=- preview=- undo=0"#);
            portal.push(r#"None closed value=- preview=- undo=0"#);
            assert_eq!(run(TargetKind::PortalText, &script), portal);
        }

        /// Second half of the `edge_still_valid` guard: the edge
        /// survives but leaves portal mode, so the endpoint text the
        /// editor is bound to stops existing as a rendered element.
        /// The guard closes the editor without committing, leaving
        /// the pre-edit value intact.
        #[test]
        fn test_oracle_portal_edge_flipped_to_line_mode_closes_without_commit() {
            assert_eq!(
                run(
                    TargetKind::PortalText,
                    &[
                        Step::Open { clean: false },
                        Step::Type("!"),
                        Step::FlipToLine,
                        Step::Type("?"),
                        Step::Close { commit: true },
                    ],
                ),
                vec![
                    r#"Preview open[hi]@2 value="hi" preview="hi|" undo=0"#,
                    r#"Preview open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                    r#"None open[hi!]@3 value="hi" preview="hi!|" undo=0"#,
                    r#"All closed value="hi" preview=- undo=0"#,
                    r#"None closed value="hi" preview=- undo=0"#,
                ]
            );
        }

        /// Opening on a target that already vanished leaves the
        /// editor closed and stages no preview.
        #[test]
        fn test_oracle_open_on_missing_target_is_a_noop() {
            assert_both(
                &[Step::DeleteEdge, Step::Open { clean: false }, Step::Type("x")],
                &[
                    "None closed value=- preview=- undo=0",
                    "None closed value=- preview=- undo=0",
                    "None closed value=- preview=- undo=0",
                ],
            );
        }
    }
}
