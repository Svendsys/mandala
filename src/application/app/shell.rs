// SPDX-License-Identifier: MPL-2.0

//! What the app is shown in: a winit window, or nothing (`--headless`).
//!
//! **The whole of the app's reach into its window is two calls**, and
//! both go through here: asking for a frame, and setting the cursor
//! icon. Everything else a frame needs — the surface size, the camera's
//! viewport, every screen-space layout — reads the renderer, which is
//! sized from the window once and resized by `RenderDecree`s, never by
//! asking the window again. So with no window, both calls have a
//! meaning to keep rather than a hole to paper over: a frame asked for
//! is a frame the headless driver owes and draws before it next parks
//! (`app::run_headless`), exactly as winit delivers a requested
//! `RedrawRequested`; and a cursor icon is state no one can see, so
//! setting it is nothing at all. The cursor bookkeeping beside it
//! (`InitState::cursor_icon_last`) still updates, so the two drivers
//! leave the app in the same state after the same input.
//!
//! Native-only: the browser's shell is its canvas (`run_wasm`).

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use winit::window::{CursorIcon, Window};

/// The window the app is drawn in, or the absence of one.
pub(super) enum Shell {
    /// A winit window, whose `RedrawRequested` the event loop delivers.
    Window(Arc<Window>),
    /// No window: frames are owed to the headless driver, which draws
    /// one whenever `redraw_requested` is set and clears it.
    Headless { redraw_requested: bool },
}

impl Shell {
    /// A shell with no window, owing its first frame — as a new window
    /// is drawn once when it is first exposed, without being asked.
    pub(super) fn headless() -> Shell {
        Shell::Headless {
            redraw_requested: true,
        }
    }

    /// Ask for a frame: winit's request on a window, the headless
    /// driver's flag without one.
    pub(super) fn request_redraw(&mut self) {
        match self {
            Shell::Window(window) => window.request_redraw(),
            Shell::Headless { redraw_requested } => *redraw_requested = true,
        }
    }

    /// Whether a frame is owed to the headless driver, clearing the ask.
    /// Always `false` on a window, whose frames winit delivers.
    pub(super) fn take_redraw_request(&mut self) -> bool {
        match self {
            Shell::Window(_) => false,
            Shell::Headless { redraw_requested } => std::mem::take(redraw_requested),
        }
    }

    /// Show `icon` as the pointer: a window's cursor, and nothing with no
    /// window to point in.
    pub(super) fn set_cursor(&self, icon: CursorIcon) {
        if let Shell::Window(window) = self {
            window.set_cursor(icon);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame asked for headless is owed once: taking it clears it, and
    /// asks coalesce as winit's do into one `RedrawRequested`.
    #[test]
    fn test_a_headless_frame_is_owed_once_however_often_it_is_asked_for() {
        let mut shell = Shell::headless();
        assert!(shell.take_redraw_request(), "the first frame is owed unasked");
        assert!(!shell.take_redraw_request(), "nothing asked since");
        shell.request_redraw();
        shell.request_redraw();
        assert!(shell.take_redraw_request());
        assert!(!shell.take_redraw_request(), "the two asks were one frame");
        // A cursor with nowhere to show is no call at all.
        shell.set_cursor(CursorIcon::Pointer);
        assert!(!shell.take_redraw_request());
    }
}
