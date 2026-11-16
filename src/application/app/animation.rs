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
//! Two kinds of animation run here, on the one clock: an animated edit
//! ([`MindMapDocument::tick_animation`]), which ends in a history
//! entry, and the presentation instances
//! ([`MindMapDocument::tick_presentations`]), which never change the
//! map (design §10).

use crate::application::clock::{AppClock, ClockMode};
use crate::application::document::MindMapDocument;

/// Write every running animation at the clock's reading: the
/// presentation instances first, then the animated edit, whose write
/// goes through the writer's funnel and so leaves the instances on top
/// where they now are. Returns whether anything was written: the
/// caller's view sync then draws it (`view::MapView::sync`, from the
/// change feed and the presentations' epoch). Nothing runs with no
/// document or no animation.
pub(super) fn tick(document: Option<&mut MindMapDocument>, clock: &AppClock) -> bool {
    document.is_some_and(|doc| {
        let now_ms = clock.now_ms();
        let presented = doc.tick_presentations(now_ms);
        let edited = doc.tick_animation(now_ms);
        presented || edited
    })
}

/// Whether a running animation owes the loop another frame: an animated
/// edit is running or a presentation instance is on its way in or out,
/// and the clock moves on its own. One that holds at its end owes
/// nothing. Under a virtual clock the frames that advance an animation
/// are a step's, so it keeps nothing awake.
pub(super) fn keeps_ticking(document: Option<&MindMapDocument>, clock: &AppClock) -> bool {
    document.is_some_and(|doc| doc.is_animating() || doc.presentations_moving())
        && clock.mode() == ClockMode::Real
}
