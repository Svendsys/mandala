// SPDX-License-Identifier: MPL-2.0

//! Click dispatch + gesture end: LMB/RMB hit-test routing, wheel
//! commit, standalone selection commit, drag-anchor gesture start,
//! and mouse-up gesture release.

use crate::application::keybinds::Action;
use crate::application::platform::input::MouseButton;

use crate::application::document::MindMapDocument;

use super::super::throttled_interaction::ColorPickerHoverInteraction;
use super::commit::apply_picker_preview;

/// What the caller should do with a press the picker was offered.
///
/// The commit / cancel effects are user-named, so they are Actions
/// dispatched through the §3 funnel by the caller rather than run
/// here — the same shape `event_mouse_click.rs` already uses for
/// the text editor's click-outside `Action::TextEditCommit`. That
/// keeps this router renderer-free and keeps one implementation of
/// each picker effect (the `dispatch_action` arm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::application::app) enum PickerClick {
    /// Fully handled inside the picker (cell selection, gesture
    /// start, swallowed RMB). Stop dispatching the press.
    Consumed,
    /// Not the picker's press — let it reach the canvas.
    FallThrough,
    /// A user-named effect: dispatch this Action through
    /// `dispatch_action`, then treat the press as consumed.
    Dispatch(Action),
}

/// Click handler for the picker. Semantics:
///
/// - **Hue / SatCell / ValCell** — select the hovered value.
///   Copies the cell's HSV component into the picker's selected
///   `hue_deg`/`sat`/`val` and clears `hover_preview`. The wheel
///   stays open — users can click around freely to build up a
///   color before committing.
/// - **Commit** (࿕) — `Action::PickerCommit` out to the funnel,
///   whose arm picks the mode-dependent behavior:
///   - Contextual: commit current HSV to the bound target, close.
///   - Standalone: apply current HSV to each item in the document
///     selection; stay open. If the selection is empty, trigger the
///     error-flash animation hook.
/// - **DragAnchor** —
///   - LMB → start a wheel-move gesture (translates `center_override`).
///   - RMB → start a wheel-resize gesture (mutates `size_scale`).
///
///   The mouse-up event ends either gesture via
///   `end_color_picker_gesture`.
/// - **Outside** —
///   - Contextual: `Action::PickerCancel` out to the funnel
///     (restore original, close).
///   - Standalone: ignored (the persistent palette only closes via
///     `color picker off`).
///
/// `button` is `MouseButton::Left` or `MouseButton::Right`. The
/// caller (the `WindowEvent::MouseInput` branch) filters out other
/// buttons before reaching here.
///
/// The Standalone-mode outside-backdrop click is the one press that
/// returns [`PickerClick::FallThrough`] — the persistent palette
/// needs to coexist with the user interacting with the canvas
/// underneath it.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn handle_color_picker_click(
    cursor_pos: (f64, f64),
    button: MouseButton,
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    picker_hover: &mut ColorPickerHoverInteraction,
) -> PickerClick {
    use crate::application::color_picker::{
        hit_test_picker, hue_slot_to_degrees, sat_cell_to_value, val_cell_to_value, ColorPickerState,
        PickerGesture, PickerHit,
    };

    let hit = if let ColorPickerState::Open {
        layout: Some(layout), ..
    } = state
    {
        hit_test_picker(layout, cursor_pos.0 as f32, cursor_pos.1 as f32)
    } else {
        return PickerClick::FallThrough;
    };

    // RMB outside the DragAnchor region is a no-op for now — only
    // the empty region of the wheel disk (inside the circle, off
    // every interactive glyph) acts as a resize handle. That keeps
    // the gesture predictable: RMB on a hue/sat/val cell or a chip
    // doesn't accidentally resize while the user is also reading
    // the live preview. In Standalone mode we fall through so
    // the RMB can reach any future right-click menu on the canvas.
    if button == MouseButton::Right && !matches!(hit, PickerHit::DragAnchor) {
        return if state.is_standalone() {
            PickerClick::FallThrough
        } else {
            PickerClick::Consumed
        };
    }

    match hit {
        PickerHit::Outside => {
            if state.is_standalone() {
                // Standalone mode: the persistent palette only
                // closes via `color picker off`. Don't consume the
                // click — let it flow through to the canvas so the
                // user can still select nodes, create edges, etc.
                return PickerClick::FallThrough;
            }
            // Contextual mode: click outside cancels. Same
            // user-named effect the Esc key names, so it goes out
            // as `Action::PickerCancel` and the funnel runs it.
            return PickerClick::Dispatch(Action::PickerCancel);
        }
        PickerHit::Hue(slot) => {
            if let ColorPickerState::Open {
                hue_deg,
                hover_preview,
                ..
            } = state
            {
                *hue_deg = hue_slot_to_degrees(slot);
                *hover_preview = None;
            }
            apply_picker_preview(state, doc, picker_hover);
        }
        PickerHit::SatCell(i) => {
            if let ColorPickerState::Open {
                sat, hover_preview, ..
            } = state
            {
                *sat = sat_cell_to_value(i);
                *hover_preview = None;
            }
            apply_picker_preview(state, doc, picker_hover);
        }
        PickerHit::ValCell(i) => {
            if let ColorPickerState::Open {
                val, hover_preview, ..
            } = state
            {
                *val = val_cell_to_value(i);
                *hover_preview = None;
            }
            apply_picker_preview(state, doc, picker_hover);
        }
        PickerHit::Commit => {
            // The ࿕ button is the mouse spelling of
            // `Action::PickerCommit`; the funnel arm picks
            // Standalone (fan out across the selection, stay
            // open) vs. Contextual (write the bound target,
            // close), so the branch lives in exactly one place.
            return PickerClick::Dispatch(Action::PickerCommit);
        }
        PickerHit::DragAnchor => {
            // Start a gesture from anywhere inside the wheel disk
            // that's not on an interactive glyph (hit_test_picker
            // now gates on the circle, not the backdrop rect).
            // LMB → move (translate center_override); RMB → resize
            // (mutate size_scale). The two gestures are mutually
            // exclusive by construction — `gesture` only holds one
            // variant.
            if let ColorPickerState::Open {
                layout: Some(layout),
                gesture,
                size_scale,
                ..
            } = state
            {
                let cursor = (cursor_pos.0 as f32, cursor_pos.1 as f32);
                *gesture = Some(match button {
                    MouseButton::Left => PickerGesture::Move {
                        grab_offset: (layout.center.0 - cursor.0, layout.center.1 - cursor.1),
                    },
                    MouseButton::Right => {
                        // Floor the anchor radius so a grab very
                        // near the wheel center doesn't make a 1px
                        // cursor move into a 100% scale change.
                        // `font_size * 3.0` is comfortably outside
                        // the central ࿕ commit button's hit
                        // radius (`preview_size * 0.45`), so the
                        // floor is rarely hit in practice anyway.
                        let dx = cursor.0 - layout.center.0;
                        let dy = cursor.1 - layout.center.1;
                        let raw_r = (dx * dx + dy * dy).sqrt();
                        let anchor_radius = raw_r.max(layout.font_size * 3.0);
                        PickerGesture::Resize {
                            anchor_radius,
                            anchor_scale: *size_scale,
                            anchor_center: layout.center,
                        }
                    }
                    // Other buttons can't reach here — caller
                    // filters to Left/Right before dispatching.
                    _ => return PickerClick::FallThrough,
                });
            }
        }
    }
    PickerClick::Consumed
}

/// End an active picker gesture. Called on mouse-up while the
/// picker is open. Returns `true` if a gesture was active and the
/// caller should treat the release as consumed. Returns `false`
/// when no gesture was active (e.g. Standalone-mode press that
/// fell through to the canvas) so the release also falls through.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn end_color_picker_gesture(
    state: &mut crate::application::color_picker::ColorPickerState,
) -> bool {
    use crate::application::color_picker::ColorPickerState;
    if let ColorPickerState::Open { gesture, .. } = state {
        let was_active = gesture.is_some();
        *gesture = None;
        was_active
    } else {
        false
    }
}
