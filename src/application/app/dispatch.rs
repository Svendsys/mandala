// SPDX-License-Identifier: MPL-2.0

//! `dispatch_action` — the single entry point that runs `Action`
//! bodies. Mouse handlers, the keyboard handler, and (via Phase 9)
//! the WASM handler all funnel through here. Adding a new behaviour
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
/// `event_mouse_click`, future macro runtime) construct an
/// `InputHandlerContext` and call this.
///
/// `hit` carries mouse-event-only payload (what the click hit, where
/// the cursor was in canvas space). Keyboard / macro callers pass
/// `None`; mouse callers populate it before invoking the dispatcher.
///
/// The function is platform-gated to native today; Phase 9 brings
/// WASM into the same funnel.
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

        // Console / Picker / LabelEdit / TextEdit modal-context actions
        // are dispatched by their respective modal handlers, not here.
        // Falling through to `Unhandled` lets the keyboard handler's
        // contextual resolution own them.
        _ => {
            log::debug!("dispatch_action: {:?} not handled at Document context", action);
            DispatchOutcome::Unhandled
        }
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
}
