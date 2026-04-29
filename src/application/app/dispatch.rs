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

/// Outcome of a `dispatch_action` call. The two non-`Handled` variants
/// let callers branch on whether the dispatcher recognized and ran the
/// action — used by the keyboard handler to decide whether to fall
/// through to custom-mutation lookup, and by the mouse handler to
/// decide whether the gesture consumed the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// The action was recognized and its body ran.
    Handled,
    /// The action's variant has no body in the current dispatcher
    /// (e.g. context-mismatched, or scaffolded ahead of its arm).
    /// Caller may fall through to lower-priority resolution.
    Unhandled,
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
                if doc.has_active_animations() {
                    doc.fast_forward_animations(ctx.mindmap_tree.as_mut());
                }
                if doc.undo() {
                    ctx.scene_cache.clear();
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
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
                if doc.apply_delete_selection() {
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::CreateOrphanNode => {
            if let Some(doc) = ctx.document.as_mut() {
                let canvas_pos = ctx
                    .renderer
                    .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
                doc.create_orphan_and_select(canvas_pos);
                rebuild_all(
                    doc,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
            DispatchOutcome::Handled
        }
        Action::OrphanSelection => {
            if let Some(doc) = ctx.document.as_mut() {
                if doc.apply_orphan_selection_with_undo() {
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::EditSelection | Action::EditSelectionClean => {
            let clean = matches!(action, Action::EditSelectionClean);
            if let Some(doc) = ctx.document.as_mut() {
                match doc.selection.clone() {
                    SelectionState::Single(id) => {
                        open_text_edit(
                            &id,
                            clean,
                            doc,
                            ctx.text_edit_state,
                            ctx.mindmap_tree,
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
            // Snapshot ↔ Off. Mirrors `fps on` / `fps off`.
            use crate::application::common::FpsDisplayMode;
            let next = match ctx.renderer.fps_display_mode() {
                FpsDisplayMode::Snapshot => FpsDisplayMode::Off,
                _ => FpsDisplayMode::Snapshot,
            };
            ctx.renderer.set_fps_display(next);
            DispatchOutcome::Handled
        }
        Action::ToggleFpsDebug => {
            // Debug ↔ Off. Mirrors `fps debug` / `fps off`.
            use crate::application::common::FpsDisplayMode;
            let next = match ctx.renderer.fps_display_mode() {
                FpsDisplayMode::Debug => FpsDisplayMode::Off,
                _ => FpsDisplayMode::Debug,
            };
            ctx.renderer.set_fps_display(next);
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
            // Step zoom centred on the cursor. Factor mirrors the
            // legacy hardcoded wheel handler (1.1 step) so wheel-bound
            // ZoomIn/ZoomOut behave identically to today's wheel zoom.
            let factor = if matches!(action, Action::ZoomIn) {
                1.1f32
            } else {
                1.0f32 / 1.1f32
            };
            ctx.renderer.process_decree(
                crate::application::common::RenderDecree::CameraZoom {
                    screen_x: ctx.cursor_pos.0 as f32,
                    screen_y: ctx.cursor_pos.1 as f32,
                    factor,
                },
            );
            DispatchOutcome::Handled
        }
        Action::ZoomReset => {
            // Reset zoom to 1.0 anchored at the screen centre (NOT
            // the cursor). A cursor-anchored zoom emits a `ZoomAt`
            // decree whose canvas-position formula shifts the camera
            // when the focus is off-centre — so a Ctrl+0 with the
            // cursor in the corner would scoot the view by 200+ px
            // instead of cleanly resetting in place. Computing the
            // factor inverse against current zoom keeps the
            // multiplicative ZoomAt path; using screen-centre as
            // the focus cancels the position shift algebraically.
            let zoom = ctx.renderer.camera_zoom().max(f32::EPSILON);
            ctx.renderer.process_decree(
                crate::application::common::RenderDecree::CameraZoom {
                    screen_x: ctx.renderer.surface_width() as f32 * 0.5,
                    screen_y: ctx.renderer.surface_height() as f32 * 0.5,
                    factor: 1.0f32 / zoom,
                },
            );
            DispatchOutcome::Handled
        }
        Action::ZoomFit => {
            // Fit the viewport to the current tree's bounds. Falls
            // back to a no-op when no tree is loaded yet.
            if let Some(tree) = ctx.mindmap_tree.as_ref() {
                ctx.renderer.fit_camera_to_tree(&tree.tree);
            }
            DispatchOutcome::Handled
        }
        Action::PanCameraNorth
        | Action::PanCameraSouth
        | Action::PanCameraEast
        | Action::PanCameraWest => {
            // Keyboard nudge — fixed step in screen pixels, then
            // converted to a CameraPan decree like the LeftDrag path
            // emits per cursor move. Step size matches a coarse but
            // perceptible nudge; users who want finer control bind a
            // smaller step manually (when the modifier-fallback or a
            // future per-arm step factor lands).
            const PAN_STEP_PX: f32 = 50.0;
            // Outer pattern guarantees one of the four — but the inner
            // `match action` has to be exhaustive over `Action`, and
            // `Action` is `#[non_exhaustive]`. Default to (0,0) so the
            // catch-all is a safe no-op rather than a panic in an
            // interactive path (CODE_CONVENTIONS §9). If a future
            // contributor extends the outer pattern, they need to
            // remember to extend this match too — the no-op fallback
            // is loud enough on a manual smoke-test (key does
            // nothing) to surface the omission.
            let (dx, dy) = match action {
                Action::PanCameraNorth => (0.0, -PAN_STEP_PX),
                Action::PanCameraSouth => (0.0, PAN_STEP_PX),
                Action::PanCameraEast => (-PAN_STEP_PX, 0.0),
                Action::PanCameraWest => (PAN_STEP_PX, 0.0),
                _ => (0.0, 0.0),
            };
            ctx.renderer.process_decree(
                crate::application::common::RenderDecree::CameraPan(dx, dy),
            );
            DispatchOutcome::Handled
        }
        Action::CenterOnSelection => {
            // Centre the camera on the centroid of the currently-
            // selected nodes. Falls back to a no-op when nothing is
            // selected (or only an edge / portal-marker selection,
            // which carries no point centroid).
            if let Some(doc) = ctx.document.as_ref() {
                let ids: Vec<&str> = doc.selection.selected_ids();
                if !ids.is_empty() {
                    let mut sum = glam::Vec2::ZERO;
                    let mut count = 0u32;
                    for id in &ids {
                        if let Some(node) = doc.mindmap.nodes.get(*id) {
                            sum += glam::Vec2::new(
                                node.position.x as f32 + node.size.width as f32 * 0.5,
                                node.position.y as f32 + node.size.height as f32 * 0.5,
                            );
                            count += 1;
                        }
                    }
                    if count > 0 {
                        ctx.renderer.set_camera_center(sum / count as f32);
                    }
                }
            }
            DispatchOutcome::Handled
        }
        Action::JumpToRoot => {
            // Select the document's first root node and centre on it.
            // "First" = id-sorted; when multiple roots exist this is
            // deterministic. No-op when the document is empty.
            if let Some(doc) = ctx.document.as_mut() {
                let target = doc.mindmap.root_nodes().first().map(|n| {
                    (
                        n.id.clone(),
                        glam::Vec2::new(
                            n.position.x as f32 + n.size.width as f32 * 0.5,
                            n.position.y as f32 + n.size.height as f32 * 0.5,
                        ),
                    )
                });
                if let Some((id, centre)) = target {
                    doc.selection = SelectionState::Single(id);
                    ctx.renderer.set_camera_center(centre);
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }

        // ── Selection Actions ────────────────────────────────
        Action::SelectAll => {
            // Only visible nodes — selecting hidden-by-fold descendants
            // would let a follow-up `DeleteSelection` silently nuke
            // subtrees the user can't see. Mirrors the click hit-test's
            // policy of skipping folded subtrees.
            if let Some(doc) = ctx.document.as_mut() {
                let all_ids: Vec<String> = doc
                    .mindmap
                    .nodes
                    .values()
                    .filter(|n| !doc.mindmap.is_hidden_by_fold(n))
                    .map(|n| n.id.clone())
                    .collect();
                doc.selection = SelectionState::from_ids(all_ids);
                rebuild_all(
                    doc,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
            DispatchOutcome::Handled
        }
        Action::DeselectAll => {
            if let Some(doc) = ctx.document.as_mut() {
                if !matches!(doc.selection, SelectionState::None) {
                    doc.selection = SelectionState::None;
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::InvertSelection => {
            // Only inverts node selections (None / Single / Multi).
            // Edge / EdgeLabel / Portal* selections are preserved
            // — inverting them would otherwise collapse to "select
            // every visible node" because their `selected_ids()` is
            // empty, which is unintuitive. Hidden-by-fold nodes are
            // filtered for the same reason as SelectAll above.
            if let Some(doc) = ctx.document.as_mut() {
                let invertable = matches!(
                    doc.selection,
                    SelectionState::None
                        | SelectionState::Single(_)
                        | SelectionState::Multi(_)
                );
                if invertable {
                    let selected: std::collections::HashSet<String> = doc
                        .selection
                        .selected_ids()
                        .into_iter()
                        .map(String::from)
                        .collect();
                    let inverted: Vec<String> = doc
                        .mindmap
                        .nodes
                        .values()
                        .filter(|n| {
                            !selected.contains(&n.id)
                                && !doc.mindmap.is_hidden_by_fold(n)
                        })
                        .map(|n| n.id.clone())
                        .collect();
                    doc.selection = SelectionState::from_ids(inverted);
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::SelectParent => {
            // Walk one step up the hierarchy from a single-node
            // selection. Multi / edge / unselected: no-op.
            if let Some(doc) = ctx.document.as_mut() {
                if let SelectionState::Single(nid) = doc.selection.clone() {
                    if let Some(parent_id) = doc
                        .mindmap
                        .nodes
                        .get(&nid)
                        .and_then(|n| n.parent_id.clone())
                    {
                        doc.selection = SelectionState::Single(parent_id);
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
        Action::SelectChild => {
            // Step into the first visible child (id-sorted) of the
            // selected single node. Skipping hidden children avoids
            // jumping the keyboard cursor into a folded subtree the
            // user can't see — mirrors the fold-aware click hit-test.
            if let Some(doc) = ctx.document.as_mut() {
                if let SelectionState::Single(nid) = doc.selection.clone() {
                    let first_child = doc
                        .mindmap
                        .children_of(&nid)
                        .into_iter()
                        .find(|c| !doc.mindmap.is_hidden_by_fold(c))
                        .map(|c| c.id.clone());
                    if let Some(child_id) = first_child {
                        doc.selection = SelectionState::Single(child_id);
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
        Action::SelectNextSibling | Action::SelectPrevSibling => {
            let forward = matches!(action, Action::SelectNextSibling);
            if let Some(doc) = ctx.document.as_mut() {
                if let SelectionState::Single(nid) = doc.selection.clone() {
                    let new_id = sibling_id(&doc.mindmap, &nid, forward);
                    if let Some(target) = new_id {
                        doc.selection = SelectionState::Single(target);
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
                let changed = crate::application::console::commands::anchor::apply_anchor_to_selection(
                    doc,
                    Some(from.as_str()),
                    Some(to.as_str()),
                );
                if changed {
                    ctx.scene_cache.clear();
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::SetEdgeBodyGlyph(ref preset) => {
            if let Some(doc) = ctx.document.as_mut() {
                let changed = crate::application::console::commands::body::apply_body_glyph_to_selection(
                    doc,
                    preset,
                );
                if changed {
                    ctx.scene_cache.clear();
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::SetBorderField { ref field, ref value } => {
            if let Some(doc) = ctx.document.as_mut() {
                let changed = crate::application::console::commands::border::apply_border_field_to_selection(
                    doc,
                    field,
                    value,
                );
                if changed {
                    ctx.scene_cache.clear();
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::SetEdgeCap { ref from, ref to } => {
            if let Some(doc) = ctx.document.as_mut() {
                let changed = crate::application::console::commands::cap::apply_cap_to_selection(
                    doc,
                    Some(from.as_str()),
                    Some(to.as_str()),
                );
                if changed {
                    ctx.scene_cache.clear();
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::SetColorBg(ref value)
        | Action::SetColorText(ref value)
        | Action::SetColorBorder(ref value) => {
            // Outer pattern guarantees one of three; inner match
            // picks the axis name. Same shape as the
            // PanCameraNorth/South/East/West fan-out.
            let axis: &str = match action {
                Action::SetColorBg(_) => "bg",
                Action::SetColorText(_) => "text",
                Action::SetColorBorder(_) => "border",
                _ => unreachable!("outer pattern guarantees a SetColor* variant"),
            };
            if let Some(doc) = ctx.document.as_mut() {
                let changed = crate::application::console::commands::color::apply_color_axis_to_selection(
                    doc, axis, value,
                );
                if changed {
                    ctx.scene_cache.clear();
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::SetEdgeType(ref value) => {
            if let Some(doc) = ctx.document.as_mut() {
                let changed = crate::application::console::commands::edge::apply_edge_type_to_selection(
                    doc, value,
                );
                if changed {
                    ctx.scene_cache.clear();
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::SetEdgeDisplayMode(ref value) => {
            if let Some(doc) = ctx.document.as_mut() {
                let changed = crate::application::console::commands::edge::apply_edge_display_mode_to_selection(
                    doc, value,
                );
                if changed {
                    ctx.scene_cache.clear();
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::ResetEdge(ref kind) => {
            if let Some(doc) = ctx.document.as_mut() {
                let changed = crate::application::console::commands::edge::apply_edge_reset_to_selection(
                    doc, kind,
                );
                if changed {
                    ctx.scene_cache.clear();
                    rebuild_all(
                        doc,
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
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

/// Resolve the id of the sibling immediately before / after `nid` in
/// the parent's children list (sorted by `id_sort_key` — Dewey-decimal
/// trailing-segment order, not lexicographic). Roots use the
/// document's `root_nodes()` ordering. Hidden-by-fold siblings are
/// skipped so keyboard navigation stays on visible nodes only.
/// Returns `None` when `nid` has no visible neighbour in the
/// requested direction.
fn sibling_id(
    map: &baumhard::mindmap::model::MindMap,
    nid: &str,
    forward: bool,
) -> Option<String> {
    let parent_id = map.nodes.get(nid).and_then(|n| n.parent_id.clone());
    // Build the sibling list with both id and hidden-state so the
    // walk past `nid` can skip folded entries efficiently.
    let siblings: Vec<(String, bool)> = match parent_id {
        Some(pid) => map
            .children_of(&pid)
            .iter()
            .map(|c| (c.id.clone(), map.is_hidden_by_fold(c)))
            .collect(),
        None => map
            .root_nodes()
            .iter()
            .map(|c| (c.id.clone(), map.is_hidden_by_fold(c)))
            .collect(),
    };
    let idx = siblings.iter().position(|(id, _)| id == nid)?;
    if forward {
        siblings
            .iter()
            .skip(idx + 1)
            .find(|(_, hidden)| !*hidden)
            .map(|(id, _)| id.clone())
    } else {
        siblings
            .iter()
            .take(idx)
            .rev()
            .find(|(_, hidden)| !*hidden)
            .map(|(id, _)| id.clone())
    }
}

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
    use crate::application::macros::{MacroStep, MacroTarget};
    let (mac, source) = match ctx.macros.get_with_source(macro_id) {
        Some((m, s)) => (m.clone(), s),
        None => {
            log::warn!("dispatch_macro: unknown macro id '{}'", macro_id);
            return false;
        }
    };
    let mut any_ran = false;
    for step in &mac.steps {
        match step {
            MacroStep::Action { action } => {
                // Privilege gate symmetric with `ConsoleLine` below.
                // Non-User tiers cannot fire destructive / clipboard /
                // I/O Actions. Fail-closed: a rejected privileged
                // step aborts the rest of the macro so a
                // `[DeleteSelection, ConsoleLine(rejected),
                // SaveDocument]` pattern can't sneak its outer steps
                // past the gate.
                if !source.allows_action(action) {
                    log::warn!(
                        "macro '{}' (source {:?}): Action {:?} rejected — \
                         tier may not invoke destructive / I/O Actions; \
                         aborting remaining steps",
                        macro_id, source, action
                    );
                    return any_ran;
                }
                let outcome = dispatch_action(action.clone(), ctx, None);
                if matches!(outcome, DispatchOutcome::Handled) {
                    any_ran = true;
                }
            }
            MacroStep::CustomMutation { id, target } => {
                let nid_opt: Option<String> = match target {
                    MacroTarget::CurrentSelection => {
                        ctx.document.as_ref().and_then(|d| {
                            if let SelectionState::Single(nid) = &d.selection {
                                Some(nid.clone())
                            } else {
                                None
                            }
                        })
                    }
                    MacroTarget::NodeId(s) => {
                        // Guard against typo'd or stale node ids: if
                        // the document doesn't have the named node
                        // we'd silently no-op (collect_affected_node_ids
                        // returns the literal id, snapshot loop filters
                        // missing, no mutation lands). Surface the
                        // problem instead.
                        if ctx
                            .document
                            .as_ref()
                            .map(|d| d.mindmap.nodes.contains_key(s))
                            .unwrap_or(false)
                        {
                            Some(s.clone())
                        } else {
                            log::warn!(
                                "macro step CustomMutation: node id '{}' not found",
                                s
                            );
                            continue;
                        }
                    }
                };
                let Some(nid) = nid_opt else {
                    log::debug!(
                        "macro step CustomMutation: no resolvable target; skipping id={}",
                        id
                    );
                    continue;
                };
                let cm = ctx
                    .document
                    .as_ref()
                    .and_then(|d| d.mutation_registry.get(id).cloned());
                let Some(cm) = cm else {
                    log::warn!("macro step: unknown custom-mutation id '{}'", id);
                    continue;
                };
                if let Some(doc) = ctx.document.as_mut() {
                    let now = super::now_ms() as u64;
                    if apply_keybind_custom_mutation(
                        doc,
                        ctx.mindmap_tree,
                        ctx.scene_cache,
                        &cm,
                        &nid,
                        now,
                    ) {
                        any_ran = true;
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
            MacroStep::ConsoleLine { line } => {
                // **Privilege gate.** `ConsoleLine` runs an arbitrary
                // console verb, including filesystem-touching ones.
                // Only `MacroSource::User` macros may carry it —
                // app-bundled, map-inline, and node-inline tiers
                // come from sources the user didn't necessarily
                // author, so they cannot do file I/O via macros.
                // Gate is active: App and Map tiers load today;
                // Inline is the only deferred tier. See
                // CODE_CONVENTIONS.md §3 carve-out.
                if !source.allows_console_line() {
                    // Fail-closed: a tier that's not allowed to run
                    // console verbs aborts the rest of the macro.
                    // `continue` would let post-gate Action steps
                    // still run, which combined with destructive
                    // Actions could leave the user in an unexpected
                    // state (e.g. `[DeleteSelection,
                    // ConsoleLine(rejected), SaveDocument]` would
                    // persist the post-delete state without the
                    // user's consent).
                    log::warn!(
                        "macro '{}' (source {:?}): ConsoleLine step rejected — \
                         only User-tier macros may run console verbs; \
                         aborting remaining steps",
                        macro_id, source
                    );
                    return any_ran;
                }
                // `execute_console_line` requires a loaded document
                // (it takes `&mut MindMapDocument`, not `Option`).
                // Macros fired before any document is loaded — i.e.
                // a `[ConsoleLine("open path/to/map.json")]` macro
                // bound to a startup hotkey — silently skip; users
                // who need the pre-load case should bind the path
                // through CLI args or the WASM `?map=` query param
                // rather than a macro. Logged at `warn!` so the
                // skip is visible.
                if let Some(doc) = ctx.document.as_mut() {
                    crate::application::app::console_input::exec::execute_console_line(
                        line,
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
                    any_ran = true;
                } else {
                    log::warn!(
                        "macro step ConsoleLine: no document loaded; skipping '{}'",
                        line
                    );
                }
            }
        }
    }
    any_ran
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
}
