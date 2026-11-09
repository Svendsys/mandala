// SPDX-License-Identifier: MPL-2.0

//! The animation tick: what one frame owes the animations running on
//! the map, on both targets (V1-D1; design §10.1, §9).
//!
//! A frame has an input-driven half and a time-driven half
//! (`app::heartbeat`); this is part of the time-driven half, and one
//! body for both targets, so they cannot advance an animation
//! differently. Native's frame runs it from the event loop and from a
//! clock step's heartbeats (`heartbeat`), the browser's from
//! `WindowEvent::RedrawRequested` and its own step (`run_wasm`).
//!
//! Two decisions live here so the targets cannot make them apart:
//! [`tick`] is *what* a frame does for the animations, and
//! [`keeps_ticking`] is *whether the loop owes another frame* for
//! them. Native's `needs_continuation` and the browser's "ask for the
//! next frame" both read the latter.
//!
//! Today the one animation is an animated edit
//! ([`MindMapDocument::tick_animation`]). V1-D2's presentation
//! instances join it here: another term in each of the two functions,
//! read from the same clock.

use crate::application::clock::{AppClock, ClockMode};
use crate::application::document::MindMapDocument;

/// Write every running animation at the clock's reading. Returns
/// whether anything was written: the caller's view sync then draws it
/// (`view::MapView::sync`, from the change feed). Nothing runs with no
/// document or no animation.
pub(super) fn tick(document: Option<&mut MindMapDocument>, clock: &AppClock) -> bool {
    document.is_some_and(|doc| doc.tick_animation(clock.now_ms()))
}

/// Whether a running animation owes the loop another frame: one is
/// running, and the clock moves on its own. Under a virtual clock the
/// frames that advance an animation are a step's, so it keeps nothing
/// awake.
pub(super) fn keeps_ticking(document: Option<&MindMapDocument>, clock: &AppClock) -> bool {
    document.is_some_and(MindMapDocument::is_animating) && clock.mode() == ClockMode::Real
}
