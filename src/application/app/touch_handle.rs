// SPDX-License-Identifier: MPL-2.0

//! A finger on a resize handle, on native (#160).
//!
//! Every touch goes to the gesture recognizer, and the recognizer's
//! one-finger drag is a camera pan. That was the whole touch
//! vocabulary, so a finger in `InteractionMode::Resize` could enter
//! the mode by long-press and then not resize anything: dragging a
//! handle panned the canvas out from under it. A touch-only user had
//! no way to resize at all, on either target.
//!
//! This is the one exception, and it is deliberately narrow. **A
//! finger that starts a gesture on a resize handle, in `Resize` mode,
//! is the left button**: its start, moves and lift drive the mouse
//! press, cursor-move and release handlers, so it captures the handle,
//! promotes to the `Throttled` resize at the drag threshold and
//! commits with its undo entry on lift — the path a mouse takes,
//! with nothing duplicated. It is never fed to the recognizer, so it
//! cannot also pan. Every other touch, in every mode, goes to the
//! recognizer exactly as before: one-finger pan and pinch keep their
//! meaning everywhere off a handle.
//!
//! Native-only because the drag state is. The browser still lacks
//! the `Resize` chrome and the handle drag (CLAUDE.md, the `LongPress`
//! → `EnterResizeMode` entry).

#![cfg(not(target_arch = "wasm32"))]

use super::touch_gesture::Phase;

/// What one touch event does, decided before anything runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TouchRoute {
    /// The gesture recognizer, as every touch did before #160.
    Recognizer,
    /// The finger holding a handle: drive the left-button path.
    Handle(HandleStep),
    /// Another finger while one holds a handle: dropped. Feeding it to
    /// the recognizer would let it pan the canvas under the handle
    /// being dragged; driving the button path with it would be a
    /// second press on a button already down.
    Ignore,
}

/// The left-button event a handle finger's touch event becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HandleStep {
    /// The finger landed on the handle: the pointer moves there, then
    /// the left button goes down.
    Press,
    /// The finger moved: the pointer moves with it.
    Move,
    /// The finger lifted, or the OS cancelled it: the pointer moves
    /// there, then the left button comes up, which commits the drag. A
    /// cancel commits too, for the reason focus loss does
    /// (`throttled_interaction::release::Release::FocusLost`): the
    /// lift that would have ended the drag is not coming.
    Release,
}

/// Which finger, if any, holds a resize handle. One per app, beside
/// the recognizer it diverts fingers from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct HandleTouch {
    finger: Option<u64>,
}

impl HandleTouch {
    /// Route one touch event, updating which finger holds the handle.
    ///
    /// `recognizer_idle` and `on_handle` are read only for a `Started`
    /// with no finger holding a handle — the one event that can claim
    /// one. A finger claims the handle when it **starts a gesture**
    /// (the recognizer tracks no other finger: a second finger landing
    /// on a handle is the other half of a pinch, and stays one) **on a
    /// handle** (`on_handle`, from `event_mouse_click::resize_handle_press`,
    /// which answers nothing outside `Resize` mode). Both are the
    /// caller's to compute, so this stays a plain-value decision the
    /// harness can drive.
    pub(super) fn route(
        &mut self,
        phase: Phase,
        id: u64,
        recognizer_idle: bool,
        on_handle: bool,
    ) -> TouchRoute {
        match self.finger {
            Some(held) if held == id => match phase {
                Phase::Moved => TouchRoute::Handle(HandleStep::Move),
                Phase::Ended | Phase::Cancelled => {
                    self.finger = None;
                    TouchRoute::Handle(HandleStep::Release)
                }
                // The same id starting again means its lift was never
                // delivered. End the drag it holds; the new touch
                // starts nothing until it lands again.
                Phase::Started => {
                    self.finger = None;
                    TouchRoute::Handle(HandleStep::Release)
                }
            },
            Some(_) => TouchRoute::Ignore,
            None if phase == Phase::Started && recognizer_idle && on_handle => {
                self.finger = Some(id);
                TouchRoute::Handle(HandleStep::Press)
            }
            None => TouchRoute::Recognizer,
        }
    }

    /// Forget the finger holding a handle without driving its release.
    /// For focus loss, which resolves the drag itself
    /// (`DragState::end_on_focus_loss`): the lift, if it ever arrives,
    /// then has nothing left to end and goes to the recognizer.
    pub(super) fn forget(&mut self) {
        self.finger = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A finger that starts on a handle drives the button, start to
    /// lift** — and is never the recognizer's. The control is the same
    /// sequence with `on_handle` false, which goes to the recognizer
    /// at every step: that is the pre-#160 behavior, where the drag
    /// panned the camera instead of resizing.
    #[test]
    fn test_a_finger_on_a_handle_is_the_left_button_until_it_lifts() {
        let mut touch = HandleTouch::default();
        assert_eq!(
            touch.route(Phase::Started, 7, true, true),
            TouchRoute::Handle(HandleStep::Press)
        );
        assert_eq!(
            touch.route(Phase::Moved, 7, true, false),
            TouchRoute::Handle(HandleStep::Move),
            "once claimed, the finger stays the handle's wherever it moves"
        );
        assert_eq!(
            touch.route(Phase::Ended, 7, true, false),
            TouchRoute::Handle(HandleStep::Release)
        );
        assert_eq!(
            touch.route(Phase::Started, 8, true, false),
            TouchRoute::Recognizer,
            "the lift released the claim"
        );

        let mut touch = HandleTouch::default();
        for phase in [Phase::Started, Phase::Moved, Phase::Ended] {
            assert_eq!(
                touch.route(phase, 7, true, false),
                TouchRoute::Recognizer,
                "off a handle every touch is the recognizer's"
            );
        }
    }

    /// **A second finger never takes a handle**, so a pinch that
    /// happens to land one finger on a handle stays a pinch. The
    /// control is the first row: the same `Started` on a handle, with
    /// the recognizer idle, does claim it.
    #[test]
    fn test_only_the_first_finger_of_a_gesture_can_claim_a_handle() {
        let mut touch = HandleTouch::default();
        assert_eq!(
            touch.route(Phase::Started, 1, true, true),
            TouchRoute::Handle(HandleStep::Press)
        );
        let mut touch = HandleTouch::default();
        assert_eq!(
            touch.route(Phase::Started, 2, false, true),
            TouchRoute::Recognizer,
            "with one finger already tracked, this is the second half of a pinch"
        );
    }

    /// While one finger holds a handle, every other finger is dropped:
    /// to the recognizer it would pan the canvas under the drag, and to
    /// the button path it would be a second press. The holding
    /// finger's own events still route — the control rows.
    #[test]
    fn test_other_fingers_are_ignored_while_one_holds_a_handle() {
        let mut touch = HandleTouch::default();
        touch.route(Phase::Started, 1, true, true);
        for phase in [Phase::Started, Phase::Moved, Phase::Ended, Phase::Cancelled] {
            assert_eq!(touch.route(phase, 2, true, true), TouchRoute::Ignore, "{phase:?}");
        }
        assert_eq!(
            touch.route(Phase::Moved, 1, true, false),
            TouchRoute::Handle(HandleStep::Move)
        );
    }

    /// A cancel commits like a lift, and a lost lift is ended by the
    /// same id starting again — neither leaves the handle held by a
    /// finger that is gone. `forget` drops the claim without a release,
    /// for focus loss, which commits the drag itself.
    #[test]
    fn test_no_way_out_leaves_a_handle_held() {
        let mut touch = HandleTouch::default();
        touch.route(Phase::Started, 1, true, true);
        assert_eq!(
            touch.route(Phase::Cancelled, 1, true, true),
            TouchRoute::Handle(HandleStep::Release)
        );
        assert_eq!(touch, HandleTouch::default());

        touch.route(Phase::Started, 1, true, true);
        assert_eq!(
            touch.route(Phase::Started, 1, true, true),
            TouchRoute::Handle(HandleStep::Release),
            "a second Started for the held id ends the drag its lost lift owed"
        );
        assert_eq!(touch, HandleTouch::default());

        touch.route(Phase::Started, 1, true, true);
        touch.forget();
        assert_eq!(touch.route(Phase::Ended, 1, true, true), TouchRoute::Recognizer);
    }
}
