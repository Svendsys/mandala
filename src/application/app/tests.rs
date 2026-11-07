// SPDX-License-Identifier: MPL-2.0

//! The app shell's pure decisions: double-click detection, wheel
//! decomposition, the one-shot NativeOnly warning, and the drag state
//! machine's ownership rules. Each is cursor / time / scroll math over
//! plain values, so exercising them here keeps the winit event loop out
//! of the test scaffold.

use super::*;

/// Two distinct element ids, for the hit a click records.
fn node(n: u32) -> baumhard::arena::NodeId {
    let mut doc = crate::application::document::MindMapDocument::new_blank(None, 50);
    for _ in 0..n {
        doc.create_root_for_test();
    }
    let id = crate::application::document::selection::every_node(doc.doc())
        .nth(n as usize - 1)
        .expect("the node just made");
    id
}

// -----------------------------------------------------------------
// Double-click detection
// -----------------------------------------------------------------

#[test]
fn test_double_click_same_target_within_window_fires() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Element(node(1)),
    };
    assert!(is_double_click(
        &prev,
        1100.0,
        (101.0, 100.0),
        &ClickHit::Element(node(1)),
    ));
}

#[test]
fn test_double_click_different_targets_does_not_fire() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Element(node(1)),
    };
    assert!(!is_double_click(
        &prev,
        1100.0,
        (100.0, 100.0),
        &ClickHit::Element(node(2)),
    ));
}

#[test]
fn test_double_click_too_far_apart_does_not_fire() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Empty,
    };
    // Distance = sqrt(20² + 0²) = 20px → dist² = 400, threshold = 256.
    assert!(!is_double_click(&prev, 1100.0, (120.0, 100.0), &ClickHit::Empty));
}

#[test]
fn test_double_click_expired_does_not_fire() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Empty,
    };
    assert!(!is_double_click(&prev, 1500.0, (100.0, 100.0), &ClickHit::Empty));
}

#[test]
fn test_double_click_empty_space_both_misses_fires() {
    // Both clicks landed on no node — valid double-click for
    // the "create orphan" gesture.
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (50.0, 50.0),
        hit: ClickHit::Empty,
    };
    assert!(is_double_click(&prev, 1150.0, (52.0, 51.0), &ClickHit::Empty));
}

#[test]
fn test_double_click_exact_boundary_does_not_fire() {
    // At exactly DOUBLE_CLICK_MS elapsed, should NOT fire (uses >= threshold).
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Empty,
    };
    assert!(!is_double_click(&prev, 1400.0, (100.0, 100.0), &ClickHit::Empty));
}

#[test]
fn test_double_click_just_under_boundary_fires() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Empty,
    };
    assert!(is_double_click(&prev, 1399.0, (100.0, 100.0), &ClickHit::Empty));
}

// -----------------------------------------------------------------
// Wheel decomposition
//
// `wheel_lines` and `wheel_gesture` are the two halves of "what did
// the user's scroll mean". Before unification the arithmetic lived
// at both wheel handlers and the browser's half never reached the
// second step at all — it went straight to a hardcoded 1.1x zoom.
// These pin the shared bodies both targets now run.
// -----------------------------------------------------------------

mod wheel_tests {
    use super::super::{wheel_gesture, wheel_lines};
    use crate::application::keybinds::MouseGesture;
    use crate::application::platform::input::MouseScrollDelta;

    /// A notched wheel reports whole lines and they pass through
    /// untouched — no divisor is applied to this shape.
    #[test]
    fn test_wheel_lines_passes_line_deltas_through_unscaled() {
        assert_eq!(wheel_lines(MouseScrollDelta::LineDelta(0.0, 1.0)), 1.0);
        assert_eq!(wheel_lines(MouseScrollDelta::LineDelta(0.0, -3.0)), -3.0);
    }

    /// The horizontal component is discarded: this app zooms on
    /// vertical scroll only, and a trackpad's sideways drift must
    /// not be read as a zoom.
    #[test]
    fn test_wheel_lines_ignores_the_horizontal_component() {
        assert_eq!(wheel_lines(MouseScrollDelta::LineDelta(99.0, 1.0)), 1.0);
    }

    /// Pixel deltas divide by the app's 50-px-per-line convention.
    /// Pinned as an exact value so a change to the divisor is a
    /// visible test edit rather than a silent shift in trackpad
    /// sensitivity on one target.
    #[test]
    fn test_wheel_lines_converts_pixel_deltas_at_fifty_px_per_line() {
        let d = MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 100.0));
        assert_eq!(wheel_lines(d), 2.0);
        let up = MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, -25.0));
        assert_eq!(wheel_lines(up), -0.5);
    }

    #[test]
    fn test_wheel_gesture_maps_positive_to_up_and_negative_to_down() {
        assert_eq!(wheel_gesture(1.0), MouseGesture::WheelUp);
        assert_eq!(wheel_gesture(0.01), MouseGesture::WheelUp);
        assert_eq!(wheel_gesture(-1.0), MouseGesture::WheelDown);
    }

    /// Zero is a decision, not arithmetic: a scroll event with no
    /// vertical component resolves to `WheelDown`, matching the
    /// `> 0.0` test both handlers used before unification. Pinned
    /// because it is the one input where a `>=` would diverge, and
    /// because both targets must make the same call.
    #[test]
    fn test_wheel_gesture_treats_zero_as_down() {
        assert_eq!(wheel_gesture(0.0), MouseGesture::WheelDown);
        assert_eq!(wheel_gesture(-0.0), MouseGesture::WheelDown);
    }
}

// -----------------------------------------------------------------
// One-shot NativeOnly-binding warn latch
//
// `warn_unhandled_native_only_once` is what a user gets when they
// bind a NativeOnly Action to a browser gesture: one line, once. It
// lived inside `run_wasm`, which is `#![cfg(target_arch = "wasm32")]`
// and therefore out of `cargo test`'s reach, though nothing in it is
// browser-specific (§T9). It is cross-platform now, and its return
// value is the emission decision — the only way to assert "exactly
// one emission" in a workspace with no log-capture facility and a
// §T10 rule against process-global mocks.
// -----------------------------------------------------------------

mod native_only_warn_latch_tests {
    use super::super::warn_unhandled_native_only_once;
    use crate::application::keybinds::Action;
    use std::sync::atomic::AtomicBool;

    fn warn(latch: &AtomicBool) -> bool {
        warn_unhandled_native_only_once(latch, "DoubleClick", &Action::OpenConsole, "remedy")
    }

    /// Once means once. The second call and every call after it are
    /// silent, so a gesture the user repeats does not fill the
    /// console — `warn!` survives into release (§9).
    #[test]
    fn test_warn_unhandled_native_only_once_emits_exactly_once() {
        let latch = AtomicBool::new(false);
        assert!(warn(&latch), "the first call must emit");
        assert!(!warn(&latch), "the second call must not emit");
        assert!(!warn(&latch), "and neither must any later one");
    }

    /// The latch is per-input-class, and that is the whole reason it
    /// is a parameter rather than a `static` inside the function: a
    /// dead double-click binding must not silence a dead wheel
    /// binding, which is what one shared latch across three call
    /// sites would do.
    #[test]
    fn test_warn_unhandled_native_only_once_latches_are_independent() {
        let double_click = AtomicBool::new(false);
        let wheel = AtomicBool::new(false);
        assert!(warn(&double_click));
        assert!(!warn(&double_click));
        assert!(warn(&wheel), "a separate input class gets its own line");
    }

    /// A latch that arrives already set stays silent — the caller's
    /// `static` is `false` at startup and never reset, so "already
    /// warned in this session" is the steady state.
    #[test]
    fn test_warn_unhandled_native_only_once_respects_a_pre_set_latch() {
        let latch = AtomicBool::new(true);
        assert!(!warn(&latch));
    }
}

// -----------------------------------------------------------------
// Drag-state abandonment guard
// -----------------------------------------------------------------

/// [`super::DragState::would_abandon_gesture`] — the class of states a
/// freshly-armed gesture may not overwrite — and the pan ownership
/// rules (#153).
mod abandonment_guard {
    use super::*;

    /// A right press pending its threshold is the one state a left
    /// press may not replace: its `RightDrag` would be lost. Every
    /// other state owes the model nothing, and a left press has to keep
    /// re-arming from them or a click after a release the window never
    /// delivered would do nothing at all.
    #[test]
    fn test_a_left_press_rearms_from_every_state_but_a_pending_right_press() {
        assert!(DragState::PendingRight {
            start_pos: (0.0, 0.0),
            start_canvas: glam::Vec2::ZERO,
            hit: ClickHit::Empty,
        }
        .would_abandon_gesture());
        for drag in [
            DragState::None,
            DragState::Panning {
                owner: PanOwner::Toggled,
            },
            DragState::SelectingRect {
                start_canvas: glam::Vec2::ZERO,
                current_canvas: glam::Vec2::new(10.0, 10.0),
            },
            DragState::Pending {
                start_pos: (0.0, 0.0),
                hit: ClickHit::Empty,
            },
        ] {
            assert!(
                !drag.would_abandon_gesture(),
                "a left press must still re-arm from {:?}",
                std::mem::discriminant(&drag)
            );
        }
    }

    fn left() -> HeldInput {
        HeldInput::Button(crate::application::platform::input::MouseButton::Left)
    }

    fn middle() -> HeldInput {
        HeldInput::Button(crate::application::platform::input::MouseButton::Middle)
    }

    fn held_pan(input: HeldInput) -> DragState {
        DragState::Panning {
            owner: PanOwner::Held(input),
        }
    }

    fn toggled_pan() -> DragState {
        DragState::Panning {
            owner: PanOwner::Toggled,
        }
    }

    /// **A pan ends on the release of the input that holds it, and on
    /// no other** (#153). The diagonal is the rule; each off-diagonal
    /// row is a release that used to end the pan and must not —
    /// pre-#153 the left release arm matched `Panning` outright, so a
    /// left release ended a middle-started pan and a key-armed one.
    ///
    /// The first assertion of each pair is the negative control for
    /// the second: the same release, the same variant, with the owner
    /// that *does* match.
    #[test]
    fn test_a_release_ends_only_the_pan_its_own_input_holds() {
        let space = HeldInput::Key("space".into());
        let mut pan = held_pan(left());
        pan.end_pan_held_by(&left());
        assert!(
            matches!(pan, DragState::None),
            "left release ends the left-drag pan"
        );

        for (owner, released) in [
            (middle(), left()),
            (left(), middle()),
            (space.clone(), left()),
            (left(), space.clone()),
            (HeldInput::Key("p".into()), space.clone()),
        ] {
            let mut pan = held_pan(owner.clone());
            pan.end_pan_held_by(&released);
            assert!(
                pan.is_pan_held_by(&owner),
                "a {released:?} release must leave the pan {owner:?} holds"
            );
        }
    }

    /// A toggled pan has no release at all: it was armed by an input
    /// that has none, so no button and no key can be the one that
    /// ends it. Only the next `pan_canvas` does
    /// (`dispatch::native`'s `test_pan_canvas_ends_a_toggled_pan`).
    #[test]
    fn test_no_release_ends_a_toggled_pan() {
        for released in [left(), middle(), HeldInput::Key("space".into())] {
            let mut pan = toggled_pan();
            pan.end_pan_held_by(&released);
            assert!(
                matches!(
                    pan,
                    DragState::Panning {
                        owner: PanOwner::Toggled
                    }
                ),
                "a {released:?} release must not end a toggled pan"
            );
        }
    }

    /// The held input that dispatched the arming `PanCanvas` claims
    /// the pan; the negative control is the same call with
    /// `was_panning` set, where the pan was already running before
    /// the dispatch and the toggled owner must survive.
    ///
    /// The second half fails on a claim that ignored `was_panning`:
    /// then a key bound to anything — `zoom_in`, say — pressed during
    /// a macro-toggled pan would take it over, and that key's release
    /// would end a pan the user toggled on and never toggled off.
    #[test]
    fn test_a_held_input_claims_only_the_pan_its_dispatch_armed() {
        let mut pan = toggled_pan();
        pan.hand_armed_pan_to(false, middle());
        assert!(
            pan.is_pan_held_by(&middle()),
            "the middle press claims the pan it armed"
        );

        let mut pan = toggled_pan();
        pan.hand_armed_pan_to(true, HeldInput::Key("z".into()));
        assert!(
            matches!(
                pan,
                DragState::Panning {
                    owner: PanOwner::Toggled
                }
            ),
            "a key whose dispatch armed nothing must not claim a running toggled pan"
        );

        let mut pan = held_pan(left());
        pan.hand_armed_pan_to(false, middle());
        assert!(pan.is_pan_held_by(&left()), "a held pan is never re-owned");

        let mut idle = DragState::None;
        idle.hand_armed_pan_to(false, middle());
        assert!(matches!(idle, DragState::None), "nothing armed, nothing claimed");
    }

    /// **Focus loss leaves no gesture waiting for a release** (#153):
    /// every state ends at `None`.
    #[test]
    fn test_focus_loss_ends_every_gesture() {
        for mut drag in [
            DragState::None,
            toggled_pan(),
            held_pan(middle()),
            held_pan(HeldInput::Key("space".into())),
            DragState::SelectingRect {
                start_canvas: glam::Vec2::ZERO,
                current_canvas: glam::Vec2::ONE,
            },
            DragState::PendingRight {
                start_pos: (0.0, 0.0),
                start_canvas: glam::Vec2::ZERO,
                hit: ClickHit::Empty,
            },
            DragState::Pending {
                start_pos: (0.0, 0.0),
                hit: ClickHit::Empty,
            },
        ] {
            let before = std::mem::discriminant(&drag);
            drag.end_on_focus_loss();
            assert!(
                matches!(drag, DragState::None),
                "{before:?} must not outlive focus loss"
            );
        }
    }
}
