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
//! is written in the redraw ledger (`app::settle`), which every ask
//! goes through anyway and which the headless driver reads to draw the
//! frames it owes before it next parks (`app::run_headless`), as winit
//! delivers a requested `RedrawRequested`; and a cursor icon is state
//! no one can see, so setting it is nothing at all. The cursor
//! bookkeeping beside it (`InitState::cursor_icon_last`) still updates,
//! so the two drivers leave the app in the same state after the same
//! input.
//!
//! Native-only: the browser's shell is its canvas (`run_wasm`).

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use winit::window::{CursorIcon, Window};

/// The window the app is drawn in, or the absence of one.
pub(super) enum Shell {
    /// A winit window, whose `RedrawRequested` the event loop delivers.
    Window(Arc<Window>),
    /// No window. The frames it owes are the redraw ledger's to say
    /// (`RedrawLedger::frame_owed`), which every ask already goes
    /// through, so the shell keeps no count of its own.
    Headless,
}

impl Shell {
    /// Ask for a frame: winit's request on a window. With no window the
    /// ask is the ledger's alone, which the caller has already written.
    pub(super) fn request_redraw(&self) {
        if let Shell::Window(window) = self {
            window.request_redraw();
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
