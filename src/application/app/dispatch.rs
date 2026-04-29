// SPDX-License-Identifier: MPL-2.0

//! `dispatch_action` — the single entry point that runs `Action`
//! bodies on native. Mouse handlers and the keyboard handler funnel
//! through here. WASM has its own dispatch path today; the
//! convergence track is documented in `WASM_CONVERGENCE.md`.
//! Adding a new behaviour
//! is variant + default + arm, in that order; never inline a body in
//! a handler.

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec2;

use crate::application::document::SelectionState;
use crate::application::keybinds::Action;

use super::input_context::InputHandlerContext;
use super::{AppMode, ClickHit, DragState};
use super::{
    open_label_edit, open_portal_text_edit, open_text_edit,
};
use super::scene_rebuild::rebuild_all;
use super::click::rebuild_all_with_mode;
use super::color_picker_flow::{
    close_color_picker_standalone, open_color_picker_standalone,
};
use super::console_input::{
    rebuild_console_overlay, save_console_history, save_document_to_bound_path,
};
use crate::application::console::ConsoleState;

/// Per-event payload that mouse-driven Actions need but keyboard
/// dispatch doesn't. Populated by mouse handlers right before they
/// call `dispatch_action`; `None` for keyboard / macro callers.
#[derive(Debug, Clone)]
pub struct DispatchHit {
    /// What the click landed on. The `DoubleClickActivate` arm routes
    /// on this.
    pub click_hit: ClickHit,
    /// Canvas-space cursor position at the gesture's trigger time.
    /// Used by orphan-creation / open-editor arms.
    pub canvas_pos: Vec2,
}

// `DispatchOutcome` lives in `cross_dispatch` so the cross-platform
// `dispatch_macro_core::MacroDispatchTarget` trait can return it
// from both targets' impls. Re-imported here for the dispatch arms.
pub(in crate::application::app) use super::cross_dispatch::DispatchOutcome;

/// Quote a free-form string (typically a filesystem path) so the
/// console parser sees it as a single token. Wraps with `"..."`
/// unconditionally and escapes both `\` (→ `\\`) and `"` (→ `\"`)
/// so Windows-style paths and embedded quotes round-trip cleanly
/// through `parser::tokenize`'s quoted-string handling. Order
/// matters: backslash MUST be escaped before quote, otherwise a
/// path ending in `\` produces an unterminated quoted token.
/// Used by the parametric filesystem Action arms.
fn quote_console_arg(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() + 2);
    escaped.push('"');
    for ch in s.chars() {
        if ch == '\\' || ch == '"' {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push('"');
    escaped
}

/// Run an `Action` against the live application context. The body of
/// every Document-level action lives here; handlers (`event_keyboard`,
/// `event_mouse_click`, the macro runtime via `dispatch_macro`)
/// construct an `InputHandlerContext` and call this.
///
/// `hit` carries mouse-event-only payload (what the click hit, where
/// the cursor was in canvas space). Keyboard / macro callers pass
/// `None`; mouse callers populate it before invoking the dispatcher.
///
/// The function is platform-gated to native today;
/// `WASM_CONVERGENCE.md` documents the path to bringing WASM
/// into the same funnel.
pub(in crate::application::app) fn dispatch_action(
    action: Action,
    ctx: &mut InputHandlerContext<'_>,
    hit: Option<&DispatchHit>,
) -> DispatchOutcome {
    match action {
        Action::OpenConsole => {
            if ctx.console_state.is_open() {
                save_console_history(ctx.console_history);
                *ctx.console_state = ConsoleState::Closed;
                ctx.renderer.rebuild_console_overlay_buffers(ctx.app_scene, None);
            } else {
                *ctx.console_state = ConsoleState::open(ctx.console_history.clone());
                if let Some(doc) = ctx.document.as_ref() {
                    rebuild_console_overlay(
                        ctx.console_state,
                        doc,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.keybinds,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::Undo => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_undo(&mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::CancelMode => {
            if matches!(*ctx.app_mode, AppMode::Reparent { .. } | AppMode::Connect { .. }) {
                *ctx.app_mode = AppMode::Normal;
                *ctx.hovered_node = None;
                *ctx.last_click = None;
                if let Some(doc) = ctx.document.as_ref() {
                    rebuild_all_with_mode(
                        doc,
                        ctx.app_mode,
                        ctx.hovered_node.as_deref(),
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::EnterReparentMode => {
            if let Some(doc) = ctx.document.as_ref() {
                let sel: Vec<String> = doc
                    .selection
                    .selected_ids()
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                if !sel.is_empty() {
                    *ctx.app_mode = AppMode::Reparent { sources: sel };
                    *ctx.hovered_node = None;
                    *ctx.last_click = None;
                    rebuild_all_with_mode(
                        doc,
                        ctx.app_mode,
                        ctx.hovered_node.as_deref(),
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::EnterConnectMode => {
            if let Some(doc) = ctx.document.as_ref() {
                if let SelectionState::Single(source) = &doc.selection {
                    *ctx.app_mode = AppMode::Connect {
                        source: source.clone(),
                    };
                    *ctx.hovered_node = None;
                    *ctx.last_click = None;
                    rebuild_all_with_mode(
                        doc,
                        ctx.app_mode,
                        ctx.hovered_node.as_deref(),
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::DeleteSelection => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_delete_selection(&mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::CreateOrphanNode => {
            // Cursor position is screen-space; convert before
            // entering the cross-platform helper so the helper's
            // signature stays renderer-agnostic.
            let canvas_pos = ctx
                .renderer
                .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_create_orphan_node(canvas_pos, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::OrphanSelection => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_orphan_selection(&mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::EditSelection | Action::EditSelectionClean => {
            let clean = matches!(action, Action::EditSelectionClean);
            if let Some(doc) = ctx.document.as_mut() {
                // Single branch is Compatible — route through the
                // shared cross_dispatch helper. EdgeLabel + Portal
                // branches are NativeOnly and stay inline.
                let single_handled = {
                    let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                    super::cross_dispatch::apply_open_text_edit_on_single(
                        clean,
                        &mut rc,
                        ctx.text_edit_state,
                    )
                };
                if !single_handled {
                    match doc.selection.clone() {
                        SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => {
                            let er = s.edge_ref();
                            open_portal_text_edit(
                                &er,
                                &s.endpoint_node_id,
                                doc,
                                ctx.portal_text_edit_state,
                                ctx.app_scene,
                                ctx.renderer,
                            );
                        }
                        SelectionState::EdgeLabel(s) => {
                            open_label_edit(
                                &s.edge_ref,
                                doc,
                                ctx.label_edit_state,
                                ctx.app_scene,
                                ctx.renderer,
                            );
                        }
                        _ => {}
                    }
                }
            }
            DispatchOutcome::Handled
        }
        Action::Copy | Action::Cut => {
            use crate::application::console::traits::{
                selection_targets, view_for, ClipboardContent, HandlesCopy, HandlesCut,
            };
            let is_cut = matches!(action, Action::Cut);
            if let Some(doc) = ctx.document.as_mut() {
                let targets = selection_targets(&doc.selection);
                for tid in &targets {
                    let mut view = view_for(doc, tid);
                    let content = if is_cut {
                        view.clipboard_cut()
                    } else {
                        view.clipboard_copy()
                    };
                    if let ClipboardContent::Text(text) = content {
                        crate::application::clipboard::write_clipboard(&text);
                        break;
                    }
                }
            }
            DispatchOutcome::Handled
        }
        Action::Paste => {
            use crate::application::console::traits::{
                selection_targets, view_for, HandlesPaste, Outcome,
            };
            if let Some(text) = crate::application::clipboard::read_clipboard() {
                if let Some(doc) = ctx.document.as_mut() {
                    let targets = selection_targets(&doc.selection);
                    let mut any_applied = false;
                    for tid in &targets {
                        let mut view = view_for(doc, tid);
                        if let Outcome::Applied = view.clipboard_paste(&text) {
                            any_applied = true;
                        }
                    }
                    if any_applied {
                        rebuild_all(
                            doc,
                            ctx.mindmap_tree,
                            ctx.app_scene,
                            ctx.renderer,
                            ctx.scene_cache,
                        );
                    }
                }
            }
            DispatchOutcome::Handled
        }
        Action::SaveDocument => {
            if let Some(doc) = ctx.document.as_mut() {
                save_document_to_bound_path(doc, ctx.console_state);
            }
            DispatchOutcome::Handled
        }

        // ── Mouse-gesture Actions ──────────────────────────────
        Action::DoubleClickActivate => {
            // Routes by what the press hit. The mouse handler populates
            // `hit` before calling here; without it we have no target
            // and silently no-op (the gesture was bound but fired from
            // a non-mouse source like a macro that didn't carry hit
            // context).
            let Some(h) = hit else {
                log::debug!("DoubleClickActivate: no DispatchHit; skipping");
                return DispatchOutcome::Handled;
            };
            match &h.click_hit {
                ClickHit::Node(node_id) => {
                    if let Some(doc) = ctx.document.as_mut() {
                        let nid = node_id.clone();
                        doc.selection = SelectionState::Single(nid.clone());
                        rebuild_all(
                            doc,
                            ctx.mindmap_tree,
                            ctx.app_scene,
                            ctx.renderer,
                            ctx.scene_cache,
                        );
                        open_text_edit(
                            &nid,
                            false,
                            doc,
                            ctx.text_edit_state,
                            ctx.mindmap_tree,
                            ctx.app_scene,
                            ctx.renderer,
                        );
                    }
                }
                ClickHit::PortalMarker { edge, endpoint }
                | ClickHit::PortalText { edge, endpoint } => {
                    // Pan to the partner endpoint of the portal-mode
                    // edge — the node "on the other side."
                    let other_id = if *endpoint == edge.from_id {
                        edge.to_id.clone()
                    } else {
                        edge.from_id.clone()
                    };
                    if let Some(doc) = ctx.document.as_ref() {
                        if let Some(node) = doc.mindmap.nodes.get(&other_id) {
                            let target = glam::Vec2::new(
                                node.position.x as f32 + node.size.width as f32 * 0.5,
                                node.position.y as f32 + node.size.height as f32 * 0.5,
                            );
                            ctx.renderer.set_camera_center(target);
                        }
                    }
                    if let Some(doc) = ctx.document.as_mut() {
                        doc.selection = SelectionState::Edge(
                            crate::application::document::EdgeRef::new(
                                &edge.from_id,
                                &edge.to_id,
                                &edge.edge_type,
                            ),
                        );
                        rebuild_all(
                            doc,
                            ctx.mindmap_tree,
                            ctx.app_scene,
                            ctx.renderer,
                            ctx.scene_cache,
                        );
                    }
                }
                ClickHit::EdgeLabel(edge_key) => {
                    if let Some(doc) = ctx.document.as_mut() {
                        let er = crate::application::document::EdgeRef::new(
                            edge_key.from_id.as_str(),
                            edge_key.to_id.as_str(),
                            edge_key.edge_type.as_str(),
                        );
                        let prev = doc.selection.clone();
                        doc.selection = SelectionState::EdgeLabel(
                            crate::application::document::EdgeLabelSel::new(er.clone()),
                        );
                        super::scene_rebuild::rebuild_after_selection_change(
                            &prev,
                            doc,
                            ctx.mindmap_tree,
                            ctx.app_scene,
                            ctx.renderer,
                            ctx.scene_cache,
                        );
                        open_label_edit(
                            &er,
                            doc,
                            ctx.label_edit_state,
                            ctx.app_scene,
                            ctx.renderer,
                        );
                    }
                }
                ClickHit::Empty => {
                    // Empty-canvas double-click: only fire
                    // CreateOrphanNodeAndEdit if the user has explicitly
                    // bound it (any binding counts as opt-in). Ships
                    // unbound by default — empty-canvas double-click
                    // is a no-op out of the box per user request.
                    let edge_selected = ctx.document.as_ref()
                        .map(|d| matches!(d.selection, SelectionState::Edge(_)))
                        .unwrap_or(false);
                    if !edge_selected
                        && ctx.keybinds.has_any_binding_for(Action::CreateOrphanNodeAndEdit)
                    {
                        dispatch_create_orphan_and_edit(ctx, h);
                    }
                }
            }
            DispatchOutcome::Handled
        }
        Action::CreateOrphanNodeAndEdit => {
            // Direct invocation (e.g. from a key binding). When dispatched
            // via DoubleClickActivate's empty-canvas path, the helper is
            // called inline.
            if let Some(h) = hit {
                dispatch_create_orphan_and_edit(ctx, h);
            } else if let Some(doc) = ctx.document.as_mut() {
                let canvas_pos = ctx
                    .renderer
                    .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
                let new_id = doc.create_orphan_and_select(canvas_pos);
                rebuild_all(
                    doc,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
                open_text_edit(
                    &new_id,
                    true,
                    doc,
                    ctx.text_edit_state,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                );
            }
            DispatchOutcome::Handled
        }
        Action::PanCanvas => {
            // Continuous gesture: enter pan mode for the duration of
            // the press. The mouse-release handler unconditionally
            // resets `drag_state` to `None`, so this arm only needs
            // to handle the press side.
            *ctx.drag_state = DragState::Panning;
            DispatchOutcome::Handled
        }
        // ── Console-verb Actions ───────────────────────────────
        Action::OpenColorPicker => {
            // Mirror `color picker on`: open the standalone palette.
            if let Some(doc) = ctx.document.as_mut() {
                open_color_picker_standalone(
                    doc,
                    ctx.color_picker_state,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
            DispatchOutcome::Handled
        }
        Action::CloseColorPicker => {
            // Mirror `color picker off`.
            if let Some(doc) = ctx.document.as_mut() {
                close_color_picker_standalone(
                    ctx.color_picker_state,
                    doc,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
            DispatchOutcome::Handled
        }
        Action::ToggleFps => {
            super::cross_dispatch::apply_toggle_fps(ctx.renderer);
            DispatchOutcome::Handled
        }
        Action::ToggleFpsDebug => {
            super::cross_dispatch::apply_toggle_fps_debug(ctx.renderer);
            DispatchOutcome::Handled
        }
        Action::LabelEditOnSelection => {
            // Mirror `label edit`: open the inline editor on the
            // currently-selected edge / portal-endpoint.
            if let Some(doc) = ctx.document.as_mut() {
                match doc.selection.clone() {
                    SelectionState::EdgeLabel(s) => {
                        open_label_edit(
                            &s.edge_ref,
                            doc,
                            ctx.label_edit_state,
                            ctx.app_scene,
                            ctx.renderer,
                        );
                    }
                    SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => {
                        let er = s.edge_ref();
                        open_portal_text_edit(
                            &er,
                            &s.endpoint_node_id,
                            doc,
                            ctx.portal_text_edit_state,
                            ctx.app_scene,
                            ctx.renderer,
                        );
                    }
                    _ => {
                        log::debug!(
                            "LabelEditOnSelection: selection is not an edge / portal endpoint; no-op"
                        );
                    }
                }
            }
            DispatchOutcome::Handled
        }

        Action::ZoomIn | Action::ZoomOut => {
            use super::cross_dispatch::ZoomDir;
            let dir = match action {
                Action::ZoomIn => ZoomDir::In,
                Action::ZoomOut => ZoomDir::Out,
                // Safe-fallback for `#[non_exhaustive]` Action: a
                // future variant added to the outer or-pattern
                // without updating the inner match would otherwise
                // panic in an interactive path.
                _ => {
                    log::error!("Zoom fan-out missed inner-match variant: {:?}", action);
                    return DispatchOutcome::Handled;
                }
            };
            super::cross_dispatch::apply_zoom_step(dir, *ctx.cursor_pos, ctx.renderer);
            DispatchOutcome::Handled
        }
        Action::ZoomReset => {
            super::cross_dispatch::apply_zoom_reset(ctx.renderer);
            DispatchOutcome::Handled
        }
        Action::ZoomFit => {
            super::cross_dispatch::apply_zoom_fit(ctx.mindmap_tree, ctx.renderer);
            DispatchOutcome::Handled
        }
        Action::PanCameraNorth
        | Action::PanCameraSouth
        | Action::PanCameraEast
        | Action::PanCameraWest => {
            use super::cross_dispatch::PanDir;
            let dir = match action {
                Action::PanCameraNorth => PanDir::North,
                Action::PanCameraSouth => PanDir::South,
                Action::PanCameraEast => PanDir::East,
                Action::PanCameraWest => PanDir::West,
                _ => {
                    log::error!(
                        "PanCamera fan-out missed inner-match variant: {:?}",
                        action,
                    );
                    return DispatchOutcome::Handled;
                }
            };
            super::cross_dispatch::apply_pan_camera(dir, ctx.renderer);
            DispatchOutcome::Handled
        }
        Action::CenterOnSelection => {
            if let Some(doc) = ctx.document.as_ref() {
                super::cross_dispatch::apply_center_on_selection(doc, ctx.renderer);
            }
            DispatchOutcome::Handled
        }
        Action::JumpToRoot => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_jump_to_root(&mut rc);
            }
            DispatchOutcome::Handled
        }

        // ── Selection Actions ────────────────────────────────
        Action::SelectAll
        | Action::DeselectAll
        | Action::InvertSelection
        | Action::SelectParent
        | Action::SelectChild
        | Action::SelectNextSibling
        | Action::SelectPrevSibling => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                match action {
                    Action::SelectAll => super::cross_dispatch::apply_select_all(&mut rc),
                    Action::DeselectAll => super::cross_dispatch::apply_deselect_all(&mut rc),
                    Action::InvertSelection => {
                        super::cross_dispatch::apply_invert_selection(&mut rc)
                    }
                    Action::SelectParent => {
                        super::cross_dispatch::apply_select_parent(&mut rc)
                    }
                    Action::SelectChild => super::cross_dispatch::apply_select_child(&mut rc),
                    Action::SelectNextSibling => {
                        super::cross_dispatch::apply_select_sibling(true, &mut rc)
                    }
                    Action::SelectPrevSibling => {
                        super::cross_dispatch::apply_select_sibling(false, &mut rc)
                    }
                    // Outer pattern guarantees one of the seven, but
                    // `Action` is `#[non_exhaustive]`. Safe-fallback
                    // log + no-op per CODE_CONVENTIONS §9 for
                    // interactive paths.
                    _ => log::error!(
                        "Selection fan-out missed inner-match variant: {:?}",
                        action,
                    ),
                }
            }
            DispatchOutcome::Handled
        }

        // ── TextEdit cursor primitives ────────────────────────
        // Each arm mutates `ctx.text_edit_state` in place. The modal
        // handler `handle_text_edit_key` calls `dispatch_action` and
        // refreshes the preview tree afterwards, so arms here only
        // touch state — they don't need to rebuild.
        Action::TextEditCursorLeft
        | Action::TextEditCursorRight
        | Action::TextEditCursorUp
        | Action::TextEditCursorDown
        | Action::TextEditCursorHome
        | Action::TextEditCursorEnd
        | Action::TextEditDeleteBack
        | Action::TextEditDeleteForward
        | Action::TextEditWordLeft
        | Action::TextEditWordRight
        | Action::TextEditDeleteWordBack
        | Action::TextEditDeleteWordForward => {
            apply_text_edit_action(action, ctx.text_edit_state);
            DispatchOutcome::Handled
        }
        // `TextEditCommit` falls through to the catch-all `Unhandled`;
        // the modal handler at `text_edit/editor.rs` owns the renderer-
        // touching close-and-rebuild path. The dead `Action::TextEditCommit
        // => Unhandled` arm that used to live here was structurally
        // identical to the catch-all and added no information.

        // ── LabelEdit cursor primitives ───────────────────────
        Action::LabelEditCursorLeft
        | Action::LabelEditCursorRight
        | Action::LabelEditCursorHome
        | Action::LabelEditCursorEnd
        | Action::LabelEditDeleteBack
        | Action::LabelEditDeleteForward => {
            apply_label_edit_action(action, ctx.label_edit_state);
            DispatchOutcome::Handled
        }

        // ── Parametric console-verb Actions ────────────────────
        // Each routes through the verb's `pub(crate) apply_*` core
        // — single source of truth with the typed console verb.
        // On a successful change, trigger a full scene rebuild
        // mirroring the verb dispatcher's post-execute drain.
        Action::SetEdgeAnchor { ref from, ref to } => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_set_edge_anchor(from, to, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::SetEdgeBodyGlyph(ref preset) => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_set_edge_body_glyph(preset, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::SetBorderField { ref field, ref value } => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_set_border_field(field, value, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::SetEdgeCap { ref from, ref to } => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_set_edge_cap(from, to, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::SetColorBg(ref value)
        | Action::SetColorText(ref value)
        | Action::SetColorBorder(ref value) => {
            use super::cross_dispatch::ColorAxis;
            let axis = match action {
                Action::SetColorBg(_) => ColorAxis::Bg,
                Action::SetColorText(_) => ColorAxis::Text,
                Action::SetColorBorder(_) => ColorAxis::Border,
                _ => {
                    log::error!(
                        "SetColor* fan-out missed inner-match variant: {:?}",
                        action,
                    );
                    return DispatchOutcome::Handled;
                }
            };
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_set_color_axis(axis, value, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::SetEdgeType(ref value) => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_set_edge_type(value, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::SetEdgeDisplayMode(ref value) => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_set_edge_display_mode(value, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::ResetEdge(ref kind) => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_reset_edge(kind, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::SetFontFamily(ref family) => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_set_font_family(family, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::SetFontSize(ref pt)
        | Action::SetFontMin(ref pt)
        | Action::SetFontMax(ref pt) => {
            use super::cross_dispatch::FontSlot;
            let slot = match action {
                Action::SetFontSize(_) => FontSlot::Size,
                Action::SetFontMin(_) => FontSlot::Min,
                Action::SetFontMax(_) => FontSlot::Max,
                _ => {
                    log::error!(
                        "SetFont* fan-out missed inner-match variant: {:?}",
                        action,
                    );
                    return DispatchOutcome::Handled;
                }
            };
            // Best-effort parse; non-finite / non-positive silently
            // no-op (the verb path surfaces typed errors).
            let parsed = match pt.parse::<f32>() {
                Ok(v) if v.is_finite() && v > 0.0 => v,
                _ => {
                    log::warn!("SetFont{:?}: invalid '{}'", slot, pt);
                    return DispatchOutcome::Handled;
                }
            };
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_set_font_kv(slot, parsed, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::SetEdgeLabelText(ref text) => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_set_edge_label_text(text, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::SetEdgeLabelPosition(ref position) => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_set_edge_label_position(position, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::SetSpacing(ref input) => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_set_spacing(input, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::SetZoomMin(ref payload) | Action::SetZoomMax(ref payload) => {
            use crate::application::document::OptionEdit;
            let parsed = match crate::application::console::commands::zoom::parse_zoom_payload(payload) {
                Some(e) => e,
                None => {
                    log::warn!(
                        "set_zoom_*: invalid zoom payload '{}' — must be a positive finite float or 'unset'",
                        payload,
                    );
                    return DispatchOutcome::Handled;
                }
            };
            let (min, max) = match action {
                Action::SetZoomMin(_) => (parsed, OptionEdit::Keep),
                Action::SetZoomMax(_) => (OptionEdit::Keep, parsed),
                _ => {
                    log::error!(
                        "SetZoom* fan-out missed inner-match variant: {:?}",
                        action,
                    );
                    return DispatchOutcome::Handled;
                }
            };
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_set_zoom_window(min, max, &mut rc);
            }
            DispatchOutcome::Handled
        }
        Action::ClearZoom => {
            if let Some(doc) = ctx.document.as_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(ctx, doc);
                super::cross_dispatch::apply_clear_zoom(&mut rc);
            }
            DispatchOutcome::Handled
        }
        // ── Filesystem variants (NativeOnly) ────────────────────
        // Dispatch arms route through `execute_console_line` so the
        // existing `replace_document` / `dirty` / `file_path`
        // plumbing on `ConsoleEffects` is reused. The whole module
        // is already `cfg(not(target_arch = "wasm32"))`, so no
        // additional cfg gate is needed.
        Action::OpenDocument(ref path)
        | Action::SaveDocumentAs(ref path)
        | Action::NewDocumentAt(ref path) => {
            let verb = match action {
                Action::OpenDocument(_) => "open",
                Action::SaveDocumentAs(_) => "save",
                Action::NewDocumentAt(_) => "new",
                _ => {
                    log::error!(
                        "fs-variant fan-out missed inner-match variant: {:?}",
                        action,
                    );
                    return DispatchOutcome::Handled;
                }
            };
            let line = format!("{} {}", verb, quote_console_arg(path));
            if let Some(doc) = ctx.document.as_mut() {
                crate::application::app::console_input::exec::execute_console_line(
                    &line,
                    ctx.console_state,
                    ctx.label_edit_state,
                    ctx.portal_text_edit_state,
                    ctx.color_picker_state,
                    doc,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                    ctx.macros,
                );
            } else {
                log::warn!(
                    "{}: no document loaded; skipping '{}'",
                    verb, line
                );
            }
            DispatchOutcome::Handled
        }

        // Console / Picker / LabelEdit / TextEdit modal-context actions
        // not handled above (e.g. cancel/commit) are dispatched by their
        // respective modal handlers. Falling through to `Unhandled`
        // lets the keyboard handler's contextual resolution own them.
        _ => {
            log::debug!("dispatch_action: {:?} not handled at Document context", action);
            DispatchOutcome::Unhandled
        }
    }
}

// `apply_text_edit_action` lives in `text_edit/mod.rs` (cross-platform)
// so the WASM build can call it from the editor's modal handler.
// Brought into scope here for the dispatch arm above. Not re-exported
// — external callers should reach the function via `text_edit::`
// directly (see `text_edit/editor.rs` for the canonical caller).
use super::text_edit::apply_text_edit_action;

/// Apply a LabelEdit cursor / delete primitive to a generic
/// `(buffer, cursor)` pair. Both `LabelEditState` and
/// `PortalTextEditState` share the same single-line semantics; this
/// helper is generic over the carrier so the dispatch arms can fan
/// out into either modal. Returns `true` when state changed.
pub(in crate::application::app) fn apply_label_edit_action_to_buffer(
    action: Action,
    buffer: &mut String,
    cursor: &mut usize,
) -> bool {
    use super::text_edit::{delete_at_cursor, delete_before_cursor};
    use baumhard::util::grapheme_chad;
    let before = *cursor;
    let len_before = buffer.len();
    match action {
        Action::LabelEditCursorLeft => {
            if *cursor > 0 {
                *cursor -= 1;
            }
        }
        Action::LabelEditCursorRight => {
            if *cursor < grapheme_chad::count_grapheme_clusters(buffer) {
                *cursor += 1;
            }
        }
        Action::LabelEditCursorHome => {
            *cursor = 0;
        }
        Action::LabelEditCursorEnd => {
            *cursor = grapheme_chad::count_grapheme_clusters(buffer);
        }
        Action::LabelEditDeleteBack => {
            if *cursor > 0 {
                *cursor = delete_before_cursor(buffer, *cursor);
            }
        }
        Action::LabelEditDeleteForward => {
            if *cursor < grapheme_chad::count_grapheme_clusters(buffer) {
                *cursor = delete_at_cursor(buffer, *cursor);
            }
        }
        _ => {}
    }
    *cursor != before || buffer.len() != len_before
}

/// Convenience wrapper for the dispatch-table call site that takes
/// the LabelEditState carrier directly.
pub(in crate::application::app) fn apply_label_edit_action(
    action: Action,
    state: &mut super::LabelEditState,
) -> bool {
    use super::LabelEditState;
    let LabelEditState::Open {
        buffer,
        cursor_grapheme_pos,
        ..
    } = state else { return false; };
    apply_label_edit_action_to_buffer(action, buffer, cursor_grapheme_pos)
}

// `sibling_id` lifted to `cross_dispatch.rs` so the WASM dispatcher
// can reach the same fold-aware navigation logic.

/// Run a macro by id against the current `InputHandlerContext`.
/// Iterates the macro's steps in order, forwarding each through the
/// matching dispatch surface:
/// - `MacroStep::Action` → `dispatch_action`
/// - `MacroStep::CustomMutation` → `apply_keybind_custom_mutation`
///   (selection-fallback target resolution)
/// - `MacroStep::ConsoleLine` → `console_input::execute_console_line`
///
/// Steps are run sequentially; a step that fails (e.g. an unbound
/// custom-mutation id, or an Action that returns Unhandled) logs and
/// the next step still runs. This matches "best-effort macro" — if a
/// later step depends on an earlier one, the macro author can split
/// it into two macros.
///
/// Returns `true` if any step ran successfully.
pub(in crate::application::app) fn dispatch_macro(
    macro_id: &str,
    ctx: &mut InputHandlerContext<'_>,
) -> bool {
    // Body lifted to `dispatch_macro_core` (cross-platform); this
    // shim wraps `ctx` in a `NativeMacroDispatchTarget` so the
    // native dispatch chain calls the same step loop the WASM
    // dispatcher uses. The privilege gate is single-sourced there.
    let mut target = NativeMacroDispatchTarget { ctx };
    super::dispatch_macro_core::dispatch_macro(macro_id, &mut target)
}

/// Native impl of [`super::dispatch_macro_core::MacroDispatchTarget`].
/// Wraps `&mut InputHandlerContext` and forwards each operation to
/// the existing native helpers (`dispatch_action`,
/// `apply_keybind_custom_mutation`, `execute_console_line`).
struct NativeMacroDispatchTarget<'a, 'b> {
    ctx: &'a mut InputHandlerContext<'b>,
}

impl<'a, 'b> super::dispatch_macro_core::MacroDispatchTarget for NativeMacroDispatchTarget<'a, 'b> {
    fn registry(&self) -> &crate::application::macros::MacroRegistry {
        self.ctx.macros
    }

    fn dispatch_action(&mut self, action: Action) -> DispatchOutcome {
        super::dispatch::dispatch_action(action, self.ctx, None)
    }

    fn apply_custom_mutation(&mut self, id: &str, node_id: &str) -> bool {
        // Lookup mutation, apply via the existing
        // `apply_keybind_custom_mutation` helper, rebuild scene if
        // applied. Mirrors the `MacroStep::CustomMutation` body
        // pre-Commit-3 (lines 1067-1094 of the prior dispatch.rs).
        let cm = self
            .ctx
            .document
            .as_ref()
            .and_then(|d| d.mutation_registry.get(id).cloned());
        let Some(cm) = cm else {
            log::warn!("macro step: unknown custom-mutation id '{}'", id);
            return false;
        };
        let Some(doc) = self.ctx.document.as_mut() else {
            return false;
        };
        let now = super::now_ms() as u64;
        if apply_keybind_custom_mutation(
            doc,
            self.ctx.mindmap_tree,
            self.ctx.scene_cache,
            &cm,
            node_id,
            now,
        ) {
            rebuild_all(
                doc,
                self.ctx.mindmap_tree,
                self.ctx.app_scene,
                self.ctx.renderer,
                self.ctx.scene_cache,
            );
            true
        } else {
            false
        }
    }

    fn execute_console_line(&mut self, line: &str) {
        // `execute_console_line` requires a loaded document (takes
        // `&mut MindMapDocument`, not `Option`). Macros fired before
        // any document is loaded silently skip.
        if let Some(doc) = self.ctx.document.as_mut() {
            crate::application::app::console_input::exec::execute_console_line(
                line,
                self.ctx.console_state,
                self.ctx.label_edit_state,
                self.ctx.portal_text_edit_state,
                self.ctx.color_picker_state,
                doc,
                self.ctx.mindmap_tree,
                self.ctx.app_scene,
                self.ctx.renderer,
                self.ctx.scene_cache,
                self.ctx.macros,
            );
        } else {
            log::warn!(
                "macro step ConsoleLine: no document loaded; skipping '{}'",
                line,
            );
        }
    }

    fn current_selection_node_id(&self) -> Option<String> {
        self.ctx.document.as_ref().and_then(|d| {
            if let SelectionState::Single(nid) = &d.selection {
                Some(nid.clone())
            } else {
                None
            }
        })
    }

    fn has_node(&self, node_id: &str) -> bool {
        self.ctx
            .document
            .as_ref()
            .map(|d| d.mindmap.nodes.contains_key(node_id))
            .unwrap_or(false)
    }
}

/// Pure inner helper for the keybind-triggered custom-mutation path.
/// Runs the same animation-aware apply + always-`apply_document_actions`
/// sequence the click-trigger path at `click.rs:35-64` uses, but
/// without touching the renderer. Returns `true` when the mutation
/// was applied.
///
/// Factored out so unit tests can lock the Phase-7 parity contract
/// (no `apply_document_actions` skipping, no missed animation
/// envelope) without needing a wgpu renderer per `TEST_CONVENTIONS.md
/// §T8`. The caller is responsible for the post-apply scene rebuild.
///
/// `pub(crate)` because the parity tests in
/// `crate::application::document::tests_mutations` import it.
pub(crate) fn apply_keybind_custom_mutation(
    doc: &mut crate::application::document::MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    cm: &baumhard::mindmap::custom_mutation::CustomMutation,
    node_id: &str,
    now_ms: u64,
) -> bool {
    if cm.timing.as_ref().is_some_and(|t| t.duration_ms > 0) {
        doc.start_animation(cm, node_id, now_ms);
    } else if let Some(tree) = mindmap_tree.as_mut() {
        doc.apply_custom_mutation(cm, node_id, Some(tree));
        scene_cache.clear();
    } else {
        // No tree available and no animation requested — nothing to apply.
        return false;
    }
    // Phase-7 parity: always invoke document actions, regardless of
    // whether the mutation animated or applied directly.
    doc.apply_document_actions(cm);
    true
}

/// Resolve a custom-mutation key binding and apply it through the same
/// path the click-trigger handler at `click.rs:35-64` uses: animation-
/// aware (`start_animation` when `timing.duration_ms > 0`), and always
/// invoking `apply_document_actions`. Returns `true` when a mutation
/// was found and applied.
///
/// Phase-7 fix: the previous keyboard-side fall-through at
/// `event_keyboard.rs:528-553` skipped both `apply_document_actions`
/// and the timing envelope, so document-action and animated mutations
/// silently mis-fired when triggered from a key. This helper unifies
/// the two paths through `apply_keybind_custom_mutation`.
pub(in crate::application::app) fn dispatch_custom_mutation_for_key(
    ctx: &mut InputHandlerContext<'_>,
    key_name: &str,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> bool {
    let id = match ctx
        .keybinds
        .custom_mutation_for(key_name, ctrl, shift, alt)
    {
        Some(s) => s.to_string(),
        None => return false,
    };
    let Some(doc) = ctx.document.as_mut() else {
        return false;
    };
    let SelectionState::Single(nid) = doc.selection.clone() else {
        return false;
    };
    let Some(cm) = doc.mutation_registry.get(&id).cloned() else {
        return false;
    };
    let now = super::now_ms() as u64;
    let applied = apply_keybind_custom_mutation(
        doc,
        ctx.mindmap_tree,
        ctx.scene_cache,
        &cm,
        &nid,
        now,
    );
    if applied {
        rebuild_all(
            doc,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
        );
    }
    applied
}

/// Inline helper for the empty-canvas orphan-and-edit gesture so
/// `DoubleClickActivate` and `CreateOrphanNodeAndEdit` share one
/// implementation.
fn dispatch_create_orphan_and_edit(
    ctx: &mut InputHandlerContext<'_>,
    hit: &DispatchHit,
) {
    if let Some(doc) = ctx.document.as_mut() {
        let new_id = doc.create_orphan_and_select(hit.canvas_pos);
        rebuild_all(
            doc,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
        );
        open_text_edit(
            &new_id,
            true,
            doc,
            ctx.text_edit_state,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
        );
    }
}

#[cfg(test)]
mod tests {
    // Most dispatch arms touch the renderer (wgpu) which is forbidden
    // in tests per `TEST_CONVENTIONS.md §T8`. Per-arm pure helpers
    // would be tested here; for now the whole funnel is exercised
    // manually via `./run.sh` and through end-to-end integration on
    // top of the keybind tests in `keybinds/tests.rs` (which exercise
    // the resolver, not the dispatch bodies).
    //
    // When adding new arms whose bodies factor cleanly into a pure
    // helper, add the helper test here.
    #[test]
    fn dispatch_action_module_compiles() {
        // Smoke test: the module's public surface is reachable.
        // Replaced by per-arm tests in later phases.
    }

    #[test]
    fn quote_console_arg_wraps_plain_path_in_double_quotes() {
        assert_eq!(super::quote_console_arg("/tmp/x.json"), "\"/tmp/x.json\"");
    }

    #[test]
    fn quote_console_arg_handles_paths_with_spaces() {
        // Embedded whitespace is the whole reason quoting exists —
        // the tokenizer would otherwise split the path into multiple
        // positionals.
        assert_eq!(
            super::quote_console_arg("/tmp/some dir/x.json"),
            "\"/tmp/some dir/x.json\"",
        );
    }

    #[test]
    fn quote_console_arg_escapes_embedded_double_quotes() {
        // A literal `"` inside the path becomes `\"` so the
        // tokenizer doesn't terminate the quoted token early.
        assert_eq!(
            super::quote_console_arg(r#"/tmp/he said "hi"/x.json"#),
            r#""/tmp/he said \"hi\"/x.json""#,
        );
    }

    #[test]
    fn quote_console_arg_escapes_backslashes_for_windows_paths() {
        // Windows path: every `\` becomes `\\` so the tokenizer
        // doesn't consume the next char as part of an escape, and
        // a path ending in `\` doesn't unterminate the quote.
        assert_eq!(
            super::quote_console_arg(r"C:\Users\foo\map.json"),
            r#""C:\\Users\\foo\\map.json""#,
        );
    }

    #[test]
    fn quote_console_arg_handles_path_ending_in_backslash() {
        // Pre-fix this would produce `"C:\\foo\"` — an unterminated
        // quoted token. With the backslash escape it produces
        // `"C:\\foo\\"` which round-trips cleanly.
        assert_eq!(super::quote_console_arg(r"C:\foo\"), r#""C:\\foo\\""#);
    }
}
