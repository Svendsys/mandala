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

use baumhard::util::geometry::is_positive_finite;

use crate::application::document::OptionEdit;
use crate::application::keybinds::{Action, WasmCompatibility};

use super::super::input_context_core::InputContextCore;
use super::cross_dispatch::DispatchOutcome;

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
/// to the dispatcher's existing match for the InteractionMode-clear or
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
    // NativeOnly branches (EdgeLabel / Portal editors,
    // InteractionMode clearing). Returning `Unhandled` from a mixed
    // arm means "the cross-platform slice ran (or wasn't applicable);
    // native may have more to do".
    match action {
        Action::CancelMode => {
            // Cross-platform slice: clear `last_click` so a
            // post-Esc click isn't paired with a pre-Esc one. The
            // mode side (clear `interaction_mode`, `hovered_node`,
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
                super::cross_dispatch::apply_open_text_edit_on_single(clean, &mut rc, core.text_edit_state)
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
        // ── Modal text-edit commit / cancel ───────────────────
        // Cross-platform — `text_edit::close_text_edit` is reachable
        // on both targets. WASM keyboard handler + click-outside
        // path AND native modal handler all funnel through these
        // arms; the close helper owns the `mem::replace(.., Closed)`
        // mode-exit + tree-revert / scene-rebuild lifecycle. Inlined
        // (not via `with_doc_rebuild`) because the helper signature
        // takes `text_edit_state` separately from the rebuild
        // bundle, and the closure shape can't split-borrow `core`.
        Action::TextEditCancel | Action::TextEditCommit => {
            let commit = matches!(action, Action::TextEditCommit);
            if let Some(doc) = core.document.as_deref_mut() {
                super::super::text_edit::close_text_edit(
                    commit,
                    doc,
                    core.text_edit_state,
                    core.mindmap_tree,
                    core.app_scene,
                    core.renderer,
                    core.scene_cache,
                );
            }
        }
        // ── Document-lifecycle ─────────────────────────────────
        Action::Undo => with_doc_rebuild(core, |rc| super::cross_dispatch::apply_undo(rc)),
        Action::DeleteSelection => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_delete_selection(rc))
        }
        Action::OrphanSelection => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_orphan_selection(rc))
        }
        Action::CreateOrphanNode => {
            let canvas_pos = core
                .renderer
                .screen_to_canvas(core.cursor_pos.0 as f32, core.cursor_pos.1 as f32);
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
        Action::PanCameraNorth => {
            super::cross_dispatch::apply_pan_camera(super::cross_dispatch::PanDir::North, core.renderer)
        }
        Action::PanCameraSouth => {
            super::cross_dispatch::apply_pan_camera(super::cross_dispatch::PanDir::South, core.renderer)
        }
        Action::PanCameraEast => {
            super::cross_dispatch::apply_pan_camera(super::cross_dispatch::PanDir::East, core.renderer)
        }
        Action::PanCameraWest => {
            super::cross_dispatch::apply_pan_camera(super::cross_dispatch::PanDir::West, core.renderer)
        }
        Action::CenterOnSelection => {
            // Read-only on document; doesn't fit `with_doc_rebuild`'s
            // `&mut RebuildContext` shape.
            if let Some(doc) = core.document.as_deref() {
                super::cross_dispatch::apply_center_on_selection(doc, core.renderer);
            }
        }
        Action::JumpToRoot => with_doc_rebuild(core, |rc| super::cross_dispatch::apply_jump_to_root(rc)),
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
        Action::SetEdgeCap { from, to } => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_set_edge_cap(from, to, rc))
        }
        Action::SetColor { axis, value } => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_color_axis(*axis, value, rc)
        }),
        Action::SetEdgeType(value) => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_set_edge_type(value, rc))
        }
        Action::SetEdgeDisplayMode(value) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_display_mode(value, rc)
        }),
        Action::ResetEdge(kind) => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_reset_edge(kind, rc))
        }
        Action::SetFontFamily(family) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_font_family(family, rc)
        }),
        Action::SetFont { slot, value } => {
            let parsed = match value.parse::<f32>() {
                Ok(v) if is_positive_finite(v) => v,
                _ => {
                    log::warn!("SetFont{{slot={:?}}}: invalid '{}'", slot, value);
                    return DispatchOutcome::Handled;
                }
            };
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_set_font_kv(*slot, parsed, rc)
            });
        }
        Action::SetEdgeLabelText(text) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_label_text(text, rc)
        }),
        Action::SetEdgeLabelPosition(pos) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_label_position(pos, rc)
        }),
        Action::SetSpacing(i) => with_doc_rebuild(core, |rc| super::cross_dispatch::apply_set_spacing(i, rc)),
        Action::SetZoom { bound, value } => {
            let parsed = match crate::application::console::commands::zoom::parse_zoom_payload(value) {
                Some(e) => e,
                None => {
                    log::warn!("SetZoom{{bound={:?}}}: invalid '{}'", bound, value);
                    return DispatchOutcome::Handled;
                }
            };
            let (min, max) = match bound {
                crate::application::keybinds::ZoomBound::Min => (parsed, OptionEdit::Keep),
                crate::application::keybinds::ZoomBound::Max => (OptionEdit::Keep, parsed),
            };
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_set_zoom_window(min, max, rc)
            });
        }
        Action::ClearZoom => with_doc_rebuild(core, |rc| super::cross_dispatch::apply_clear_zoom(rc)),
        Action::SetSectionOffsetDelta { dx, dy } => {
            let (Some(dx_v), Some(dy_v)) = (dx.parse::<f64>().ok(), dy.parse::<f64>().ok()) else {
                log::warn!("SetSectionOffsetDelta: invalid dx='{}' or dy='{}'", dx, dy);
                return DispatchOutcome::Handled;
            };
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_set_section_offset_delta(dx_v, dy_v, rc)
            });
        }
        Action::SetSectionSizeAbs { w, h } => {
            let (Some(w_v), Some(h_v)) = (w.parse::<f64>().ok(), h.parse::<f64>().ok()) else {
                log::warn!("SetSectionSizeAbs: invalid w='{}' or h='{}'", w, h);
                return DispatchOutcome::Handled;
            };
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_set_section_size(
                    Some(baumhard::mindmap::model::Size { width: w_v, height: h_v }),
                    rc,
                )
            });
        }
        Action::SetSectionSizeFillParent => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_section_size(None, rc)
        }),
        // ── Clipboard ─────────────────────────────────────────
        // Compatible because `clipboard::{read,write}_clipboard`
        // are logged stubs on WASM (pending async-clipboard) and
        // the trait-driven walk over `selection_targets` compiles
        // on both targets.
        Action::Copy => {
            if let Some(doc) = core.document.as_deref_mut() {
                let _ = super::cross_dispatch::apply_copy_or_cut(false, doc);
            }
        }
        // Cut mutates the source component's text — gate the
        // rebuild on at least one target accepting the cut,
        // mirroring the `apply_paste` shape.
        Action::Cut => with_doc_rebuild(core, |rc| {
            if super::cross_dispatch::apply_copy_or_cut(true, rc.document) {
                rc.rebuild_after_geometry_change();
            }
        }),
        Action::Paste => with_doc_rebuild(core, |rc| super::cross_dispatch::apply_paste(rc)),
        // ── Create-orphan-and-edit (keyboard shape) ───────────
        // Mouse-driven empty-canvas double-click stays in
        // `dispatch.rs` (DoubleClickActivate::Empty calls
        // `dispatch_create_orphan_and_edit` directly with
        // `DispatchHit::canvas_pos`). The keyboard-bound case
        // — and the WASM target which has no DispatchHit on this
        // path — uses `cursor_pos` here.
        Action::CreateOrphanNodeAndEdit => {
            let canvas_pos = core
                .renderer
                .screen_to_canvas(core.cursor_pos.0 as f32, core.cursor_pos.1 as f32);
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_create_orphan_node_and_edit(
                    canvas_pos,
                    &mut rc,
                    core.text_edit_state,
                );
            }
        }
        // ── TextEdit cursor primitives ────────────────────────
        // Pure state mutations on `text_edit_state`. The modal
        // handler `handle_text_edit_key` calls `dispatch_action`
        // and refreshes the preview tree afterwards, so arms
        // here only touch state — no rebuild.
        Action::TextEditCursorLeft
        | Action::TextEditCursorRight
        | Action::TextEditCursorUp
        | Action::TextEditCursorDown
        | Action::TextEditCursorHome
        | Action::TextEditCursorEnd
        | Action::TextEditCursorLeftSelect
        | Action::TextEditCursorRightSelect
        | Action::TextEditCursorUpSelect
        | Action::TextEditCursorDownSelect
        | Action::TextEditCursorHomeSelect
        | Action::TextEditCursorEndSelect
        | Action::TextEditDeleteBack
        | Action::TextEditDeleteForward
        | Action::TextEditWordLeft
        | Action::TextEditWordRight
        | Action::TextEditDeleteWordBack
        | Action::TextEditDeleteWordForward => {
            super::super::text_edit::apply_text_edit_action(action.clone(), core.text_edit_state);
        }
        // Catch-all for variants `dispatch_compatible` doesn't
        // own. Two cohorts reach here: NativeOnly arms (caller's
        // fall-through runs them on native; on WASM they're
        // silently skipped, which is fine — well-formed configs
        // don't bind NativeOnly Actions to WASM key combos), and
        // mixed-branch arms whose cross-platform slice fell
        // through (`EditSelection*` on a non-Single selection).
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
        let out = lift_mixed_branch_for_wasm_macro(&Action::CancelMode, DispatchOutcome::Unhandled);
        assert_eq!(out, DispatchOutcome::Handled);
    }

    #[test]
    fn edit_selection_unhandled_lifts_to_handled() {
        let out = lift_mixed_branch_for_wasm_macro(&Action::EditSelection, DispatchOutcome::Unhandled);
        assert_eq!(out, DispatchOutcome::Handled);
    }

    #[test]
    fn edit_selection_clean_unhandled_lifts_to_handled() {
        let out = lift_mixed_branch_for_wasm_macro(&Action::EditSelectionClean, DispatchOutcome::Unhandled);
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
            let out = lift_mixed_branch_for_wasm_macro(&action, DispatchOutcome::Unhandled);
            assert_eq!(
                out,
                DispatchOutcome::Unhandled,
                "lift must not promote PURE NativeOnly: {:?}",
                action,
            );
        }
    }
}
