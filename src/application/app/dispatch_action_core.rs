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

/// Run `f` against a `RebuildContext` built from `core`, IF the
/// document is loaded. Skips silently otherwise. Captures the
/// `if let Some(doc) = core.document.as_deref_mut() { let mut rc =
/// rebuild_ctx!(core, doc); f(&mut rc); }` pattern that 20+
/// document-mutating arms in [`dispatch_compatible`] previously
/// repeated inline.
///
/// CODE_CONVENTIONS §5: "If a function is needed in two or more
/// places, the answer is never to copy it." This helper closes
/// that gap inside the dispatcher.
fn with_doc_rebuild<F>(core: &mut InputContextCore<'_>, f: F)
where
    F: FnOnce(&mut super::cross_dispatch::RebuildContext<'_>),
{
    if let Some(doc) = core.document.as_deref_mut() {
        let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
        f(&mut rc);
    }
}

/// Lift `Unhandled → Handled` for the two mixed-branch arms whose
/// cross-platform slice IS the totality of what WASM can do
/// (`CancelMode`, `EditSelection*`). On native, `Unhandled` flows
/// to the dispatcher's existing match for the AppMode-clear or
/// EdgeLabel/Portal editor open. WASM has no such fall-through —
/// `WasmMacroDispatchTarget::dispatch_action` calls
/// [`dispatch_compatible`] directly, so the macro loop's
/// `any_ran` flag would stop bumping for these arms without this
/// lift.
///
/// Public-in-app so the WASM macro target's impl can call it,
/// and so unit tests under `#[cfg(test)]` can pin the contract
/// without spinning up a `WasmInputState`.
pub(in crate::application::app) fn lift_mixed_branch_for_wasm_macro(
    action: &Action,
    outcome: DispatchOutcome,
) -> DispatchOutcome {
    if matches!(outcome, DispatchOutcome::Unhandled)
        && matches!(
            action,
            Action::CancelMode | Action::EditSelection | Action::EditSelectionClean,
        )
    {
        DispatchOutcome::Handled
    } else {
        outcome
    }
}

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
        Action::Undo => with_doc_rebuild(core, |rc| super::cross_dispatch::apply_undo(rc)),
        Action::DeleteSelection => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_delete_selection(rc))
        }
        Action::OrphanSelection => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_orphan_selection(rc))
        }
        Action::CreateOrphanNode => {
            let canvas_pos = core.renderer.screen_to_canvas(
                core.cursor_pos.0 as f32,
                core.cursor_pos.1 as f32,
            );
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_create_orphan_node(canvas_pos, rc)
            });
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
            // Read-only on document; doesn't fit `with_doc_rebuild`'s
            // `&mut RebuildContext` shape.
            if let Some(doc) = core.document.as_deref() {
                super::cross_dispatch::apply_center_on_selection(doc, core.renderer);
            }
        }
        Action::JumpToRoot => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_jump_to_root(rc))
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
        | Action::SelectPrevSibling => with_doc_rebuild(core, |rc| match action {
            Action::SelectAll => super::cross_dispatch::apply_select_all(rc),
            Action::DeselectAll => super::cross_dispatch::apply_deselect_all(rc),
            Action::InvertSelection => super::cross_dispatch::apply_invert_selection(rc),
            Action::SelectParent => super::cross_dispatch::apply_select_parent(rc),
            Action::SelectChild => super::cross_dispatch::apply_select_child(rc),
            Action::SelectNextSibling => super::cross_dispatch::apply_select_sibling(true, rc),
            Action::SelectPrevSibling => super::cross_dispatch::apply_select_sibling(false, rc),
            // Safe fallback per CODE_CONVENTIONS §9 (interactive
            // paths fail-safe). Reachable only if a future Action
            // variant joins the outer cluster without an inner arm.
            _ => log::error!(
                "dispatch_compatible: selection fan-out missed inner-match: {:?}",
                action,
            ),
        }),
        // ── Parametric mutators ────────────────────────────────
        Action::SetEdgeAnchor { from, to } => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_anchor(from, to, rc)
        }),
        Action::SetEdgeBodyGlyph(preset) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_body_glyph(preset, rc)
        }),
        Action::SetBorderField { field, value } => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_border_field(field, value, rc)
        }),
        Action::SetEdgeCap { from, to } => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_cap(from, to, rc)
        }),
        Action::SetColorBg(value)
        | Action::SetColorText(value)
        | Action::SetColorBorder(value) => {
            let axis = match action {
                Action::SetColorBg(_) => super::cross_dispatch::ColorAxis::Bg,
                Action::SetColorText(_) => super::cross_dispatch::ColorAxis::Text,
                Action::SetColorBorder(_) => super::cross_dispatch::ColorAxis::Border,
                _ => {
                    log::error!(
                        "dispatch_compatible: color axis fan-out missed inner-match: {:?}",
                        action,
                    );
                    return DispatchOutcome::Handled;
                }
            };
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_set_color_axis(axis, value, rc)
            });
        }
        Action::SetEdgeType(value) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_type(value, rc)
        }),
        Action::SetEdgeDisplayMode(value) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_display_mode(value, rc)
        }),
        Action::ResetEdge(kind) => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_reset_edge(kind, rc))
        }
        Action::SetFontFamily(family) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_font_family(family, rc)
        }),
        Action::SetFontSize(pt) | Action::SetFontMin(pt) | Action::SetFontMax(pt) => {
            let slot = match action {
                Action::SetFontSize(_) => super::cross_dispatch::FontSlot::Size,
                Action::SetFontMin(_) => super::cross_dispatch::FontSlot::Min,
                Action::SetFontMax(_) => super::cross_dispatch::FontSlot::Max,
                _ => {
                    log::error!(
                        "dispatch_compatible: font slot fan-out missed inner-match: {:?}",
                        action,
                    );
                    return DispatchOutcome::Handled;
                }
            };
            let parsed = match pt.parse::<f32>() {
                Ok(v) if v.is_finite() && v > 0.0 => v,
                _ => {
                    log::warn!("SetFont{:?}: invalid '{}'", slot, pt);
                    return DispatchOutcome::Handled;
                }
            };
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_set_font_kv(slot, parsed, rc)
            });
        }
        Action::SetEdgeLabelText(text) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_label_text(text, rc)
        }),
        Action::SetEdgeLabelPosition(pos) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_label_position(pos, rc)
        }),
        Action::SetSpacing(i) => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_set_spacing(i, rc))
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
                _ => {
                    log::error!(
                        "dispatch_compatible: zoom min/max fan-out missed inner-match: {:?}",
                        action,
                    );
                    return DispatchOutcome::Handled;
                }
            };
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_set_zoom_window(min, max, rc)
            });
        }
        Action::ClearZoom => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_clear_zoom(rc))
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

#[cfg(test)]
mod tests {
    //! Unit coverage for the mixed-branch outcome lift. The full
    //! `dispatch_compatible` body needs a `Renderer` per
    //! `TEST_CONVENTIONS.md §T8` so it isn't testable headless;
    //! the lift helper, however, is pure data — pin its contract
    //! here so the WASM-macro `any_ran` regression flagged by the
    //! Track-C parity reviewer can't recur silently.
    use super::*;
    use crate::application::keybinds::Action;

    #[test]
    fn cancel_mode_unhandled_lifts_to_handled() {
        let out = lift_mixed_branch_for_wasm_macro(
            &Action::CancelMode,
            DispatchOutcome::Unhandled,
        );
        assert_eq!(out, DispatchOutcome::Handled);
    }

    #[test]
    fn edit_selection_unhandled_lifts_to_handled() {
        let out = lift_mixed_branch_for_wasm_macro(
            &Action::EditSelection,
            DispatchOutcome::Unhandled,
        );
        assert_eq!(out, DispatchOutcome::Handled);
    }

    #[test]
    fn edit_selection_clean_unhandled_lifts_to_handled() {
        let out = lift_mixed_branch_for_wasm_macro(
            &Action::EditSelectionClean,
            DispatchOutcome::Unhandled,
        );
        assert_eq!(out, DispatchOutcome::Handled);
    }

    #[test]
    fn handled_passes_through_for_mixed_branch_arms() {
        // The lift only flips Unhandled→Handled; an already-Handled
        // outcome is passed through untouched. (EditSelection on a
        // Single selection returns Handled from the cross-platform
        // dispatcher; we don't want to alter that.)
        for action in [
            Action::CancelMode,
            Action::EditSelection,
            Action::EditSelectionClean,
        ] {
            let out = lift_mixed_branch_for_wasm_macro(&action, DispatchOutcome::Handled);
            assert_eq!(out, DispatchOutcome::Handled, "action={:?}", action);
        }
    }

    #[test]
    fn non_mixed_branch_actions_pass_through_both_outcomes() {
        // PURE Compatible arms: their dispatch_compatible behaviour
        // is authoritative on both targets. The lift must NOT alter
        // outcomes for them — if cross-platform returned Unhandled
        // for `Undo` (e.g. no document loaded), it stays Unhandled.
        let cases = [
            (Action::Undo, DispatchOutcome::Unhandled),
            (Action::Undo, DispatchOutcome::Handled),
            (Action::ZoomIn, DispatchOutcome::Unhandled),
            (Action::ZoomReset, DispatchOutcome::Handled),
            (Action::SelectAll, DispatchOutcome::Unhandled),
        ];
        for (action, outcome) in cases {
            let out = lift_mixed_branch_for_wasm_macro(&action, outcome);
            assert_eq!(out, outcome, "action={:?}", action);
        }
    }

    #[test]
    fn pure_native_only_unhandled_stays_unhandled() {
        // PURE NativeOnly arms (not in the mixed-branch set): the
        // lift must NOT promote these. A WASM macro firing
        // `Action::OpenConsole` should report `any_ran=false` because
        // the dispatcher really did nothing.
        let cases = [
            Action::OpenConsole,
            Action::EnterReparentMode,
            Action::EnterConnectMode,
            Action::SaveDocument,
        ];
        for action in cases {
            let out =
                lift_mixed_branch_for_wasm_macro(&action, DispatchOutcome::Unhandled);
            assert_eq!(
                out,
                DispatchOutcome::Unhandled,
                "lift must not promote PURE NativeOnly: {:?}",
                action,
            );
        }
    }
}
