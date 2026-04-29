// SPDX-License-Identifier: MPL-2.0

//! Cross-platform action dispatcher.
//!
//! Handles every Compatible-classified `Action` arm whose body
//! has been factored into a `cross_dispatch::apply_*` helper, plus
//! the cross-platform slice of two mixed-branch NativeOnly Actions
//! (`Action::CancelMode`'s `last_click` clear,
//! `Action::EditSelection*`-Single open). Returns `Handled` when
//! the body ran; `Unhandled` for variants this dispatcher doesn't
//! own — the caller's fall-through (native only) runs the
//! platform-specific arm.
//!
//! Both targets reach the same body:
//! - **Native**: `dispatch::dispatch_action` splits its
//!   `InputHandlerContext` into `(InputContextCore, NativeContextExt)`
//!   via `split_borrow` and calls into here first; on `Unhandled`,
//!   falls through to the native-only arm match in `dispatch.rs`.
//! - **WASM** (post-Track C wire-up in C3): `run_wasm`'s keyboard
//!   handler and `WasmMacroDispatchTarget::dispatch_action` call
//!   this directly with `&mut InputContextCore` built from
//!   `WasmInputState::input_context_core`.
//!
//! Track C from `WASM_CONVERGENCE.md`. Sibling to
//! `dispatch_macro_core` (Track B precedent). The arm coverage
//! matches what `dispatch_compatible_action_wasm` already wires
//! today — that function gets deleted in C3 once the WASM caller
//! moves over.

use crate::application::document::OptionEdit;
use crate::application::keybinds::{Action, WasmCompatibility};

use super::cross_dispatch::DispatchOutcome;
use super::input_context_core::InputContextCore;

/// Cross-platform action dispatcher. Returns `Handled` when the
/// arm body ran; `Unhandled` for variants this dispatcher doesn't
/// own (NativeOnly Actions without a cross-platform slice, or
/// mixed-branch Actions whose cross-platform slice didn't apply
/// — caller's fall-through runs the native arm).
pub(in crate::application::app) fn dispatch_compatible(
    action: &Action,
    core: &mut InputContextCore<'_>,
) -> DispatchOutcome {
    // Mixed-branch arms — handle the cross-platform slice here.
    // Caller's fall-through (native only) handles the residual
    // NativeOnly branches (EdgeLabel / Portal editors, AppMode
    // clearing). Returning `Unhandled` from a mixed arm means
    // "the cross-platform slice ran (or wasn't applicable);
    // native may have more to do".
    match action {
        Action::CancelMode => {
            // Cross-platform slice: clear `last_click` so a
            // post-Esc click isn't paired with a pre-Esc one.
            // The AppMode side (clear `app_mode`, `hovered_node`,
            // rebuild_all_with_mode) is native-only — caller's
            // fall-through runs it.
            *core.last_click = None;
            return DispatchOutcome::Unhandled;
        }
        Action::EditSelection | Action::EditSelectionClean => {
            // Cross-platform slice: Single-selection branch
            // opens the inline node text editor via
            // `apply_open_text_edit_on_single`. Returns true
            // iff selection was Single (and the editor opened).
            // EdgeLabel + Portal branches are native-only; on
            // false return, caller's fall-through tries them.
            let clean = matches!(action, Action::EditSelectionClean);
            let opened = if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::RebuildContext {
                    document: doc,
                    mindmap_tree: core.mindmap_tree,
                    app_scene: core.app_scene,
                    renderer: core.renderer,
                    scene_cache: core.scene_cache,
                };
                super::cross_dispatch::apply_open_text_edit_on_single(
                    clean,
                    &mut rc,
                    core.text_edit_state,
                )
            } else {
                false
            };
            return if opened {
                DispatchOutcome::Handled
            } else {
                DispatchOutcome::Unhandled
            };
        }
        _ => {}
    }

    // Compatible-classified Actions only beyond this point.
    // NativeOnly returns `Unhandled` so the caller's fall-through
    // runs them (on native) or they're silently skipped (on WASM,
    // where the keystroke wasn't bound anyway in well-formed
    // configs).
    if action.wasm_compatibility() == WasmCompatibility::NativeOnly {
        return DispatchOutcome::Unhandled;
    }

    match action {
        // ── Document-lifecycle ─────────────────────────────────
        Action::Undo => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_undo(&mut rc);
            }
        }
        Action::DeleteSelection => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_delete_selection(&mut rc);
            }
        }
        Action::OrphanSelection => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_orphan_selection(&mut rc);
            }
        }
        Action::CreateOrphanNode => {
            let canvas_pos = core.renderer.screen_to_canvas(
                core.cursor_pos.0 as f32,
                core.cursor_pos.1 as f32,
            );
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_create_orphan_node(canvas_pos, &mut rc);
            }
        }
        // ── Camera / zoom ──────────────────────────────────────
        Action::ZoomIn => super::cross_dispatch::apply_zoom_step(
            super::cross_dispatch::ZoomDir::In,
            *core.cursor_pos,
            core.renderer,
        ),
        Action::ZoomOut => super::cross_dispatch::apply_zoom_step(
            super::cross_dispatch::ZoomDir::Out,
            *core.cursor_pos,
            core.renderer,
        ),
        Action::ZoomReset => super::cross_dispatch::apply_zoom_reset(core.renderer),
        Action::ZoomFit => super::cross_dispatch::apply_zoom_fit(core.mindmap_tree, core.renderer),
        Action::PanCameraNorth => super::cross_dispatch::apply_pan_camera(
            super::cross_dispatch::PanDir::North,
            core.renderer,
        ),
        Action::PanCameraSouth => super::cross_dispatch::apply_pan_camera(
            super::cross_dispatch::PanDir::South,
            core.renderer,
        ),
        Action::PanCameraEast => super::cross_dispatch::apply_pan_camera(
            super::cross_dispatch::PanDir::East,
            core.renderer,
        ),
        Action::PanCameraWest => super::cross_dispatch::apply_pan_camera(
            super::cross_dispatch::PanDir::West,
            core.renderer,
        ),
        Action::CenterOnSelection => {
            if let Some(doc) = core.document.as_deref() {
                super::cross_dispatch::apply_center_on_selection(doc, core.renderer);
            }
        }
        Action::JumpToRoot => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_jump_to_root(&mut rc);
            }
        }
        // ── FPS overlay ────────────────────────────────────────
        Action::ToggleFps => super::cross_dispatch::apply_toggle_fps(core.renderer),
        Action::ToggleFpsDebug => super::cross_dispatch::apply_toggle_fps_debug(core.renderer),
        // ── Selection navigation ───────────────────────────────
        Action::SelectAll
        | Action::DeselectAll
        | Action::InvertSelection
        | Action::SelectParent
        | Action::SelectChild
        | Action::SelectNextSibling
        | Action::SelectPrevSibling => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                match action {
                    Action::SelectAll => super::cross_dispatch::apply_select_all(&mut rc),
                    Action::DeselectAll => super::cross_dispatch::apply_deselect_all(&mut rc),
                    Action::InvertSelection => {
                        super::cross_dispatch::apply_invert_selection(&mut rc)
                    }
                    Action::SelectParent => {
                        super::cross_dispatch::apply_select_parent(&mut rc)
                    }
                    Action::SelectChild => {
                        super::cross_dispatch::apply_select_child(&mut rc)
                    }
                    Action::SelectNextSibling => {
                        super::cross_dispatch::apply_select_sibling(true, &mut rc)
                    }
                    Action::SelectPrevSibling => {
                        super::cross_dispatch::apply_select_sibling(false, &mut rc)
                    }
                    _ => unreachable!("selection fan-out outer match exhaustive"),
                }
            }
        }
        // ── Parametric mutators ────────────────────────────────
        Action::SetEdgeAnchor { from, to } => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_set_edge_anchor(from, to, &mut rc);
            }
        }
        Action::SetEdgeBodyGlyph(preset) => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_set_edge_body_glyph(preset, &mut rc);
            }
        }
        Action::SetBorderField { field, value } => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_set_border_field(field, value, &mut rc);
            }
        }
        Action::SetEdgeCap { from, to } => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_set_edge_cap(from, to, &mut rc);
            }
        }
        Action::SetColorBg(value)
        | Action::SetColorText(value)
        | Action::SetColorBorder(value) => {
            let axis = match action {
                Action::SetColorBg(_) => super::cross_dispatch::ColorAxis::Bg,
                Action::SetColorText(_) => super::cross_dispatch::ColorAxis::Text,
                Action::SetColorBorder(_) => super::cross_dispatch::ColorAxis::Border,
                _ => unreachable!("color axis fan-out exhaustive"),
            };
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_set_color_axis(axis, value, &mut rc);
            }
        }
        Action::SetEdgeType(value) => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_set_edge_type(value, &mut rc);
            }
        }
        Action::SetEdgeDisplayMode(value) => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_set_edge_display_mode(value, &mut rc);
            }
        }
        Action::ResetEdge(kind) => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_reset_edge(kind, &mut rc);
            }
        }
        Action::SetFontFamily(family) => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_set_font_family(family, &mut rc);
            }
        }
        Action::SetFontSize(pt) | Action::SetFontMin(pt) | Action::SetFontMax(pt) => {
            let slot = match action {
                Action::SetFontSize(_) => super::cross_dispatch::FontSlot::Size,
                Action::SetFontMin(_) => super::cross_dispatch::FontSlot::Min,
                Action::SetFontMax(_) => super::cross_dispatch::FontSlot::Max,
                _ => unreachable!("font slot fan-out exhaustive"),
            };
            let parsed = match pt.parse::<f32>() {
                Ok(v) if v.is_finite() && v > 0.0 => v,
                _ => {
                    log::warn!("SetFont{:?}: invalid '{}'", slot, pt);
                    return DispatchOutcome::Handled;
                }
            };
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_set_font_kv(slot, parsed, &mut rc);
            }
        }
        Action::SetEdgeLabelText(text) => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_set_edge_label_text(text, &mut rc);
            }
        }
        Action::SetEdgeLabelPosition(pos) => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_set_edge_label_position(pos, &mut rc);
            }
        }
        Action::SetSpacing(i) => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_set_spacing(i, &mut rc);
            }
        }
        Action::SetZoomMin(payload) | Action::SetZoomMax(payload) => {
            let parsed = match crate::application::console::commands::zoom::parse_zoom_payload(
                payload,
            ) {
                Some(e) => e,
                None => {
                    log::warn!("set_zoom_*: invalid '{}'", payload);
                    return DispatchOutcome::Handled;
                }
            };
            let (min, max) = match action {
                Action::SetZoomMin(_) => (parsed, OptionEdit::Keep),
                Action::SetZoomMax(_) => (OptionEdit::Keep, parsed),
                _ => unreachable!("zoom min/max fan-out exhaustive"),
            };
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_set_zoom_window(min, max, &mut rc);
            }
        }
        Action::ClearZoom => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_clear_zoom(&mut rc);
            }
        }
        // Compatible-classified but not wired here yet (Copy /
        // Cut / Paste — clipboard stubs on WASM; CreateOrphan-
        // NodeAndEdit; TextEdit cursor primitives — modal-steal
        // routed). Caller's fall-through (native only) catches
        // them. Returning `Unhandled` mirrors the catch-all
        // posture in `dispatch_compatible_action_wasm` pre-Track-C.
        _ => return DispatchOutcome::Unhandled,
    }
    DispatchOutcome::Handled
}
