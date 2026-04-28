// SPDX-License-Identifier: MPL-2.0

//! Glyph-wheel color picker flow: open / commit / cancel / per-frame
//! handlers + the §B2 dispatcher (`rebuild_color_picker_overlay`)
//! the event loop calls each frame. Public surface stays
//! `pub(in crate::application::app)` — `console_input` calls the
//! lifecycle entries, the event loop calls the rest.

mod commit;
mod geometry;
mod key;
mod mouse;
mod click;
mod open;
mod rebuild;

pub(in crate::application::app) use click::{end_color_picker_gesture, handle_color_picker_click};
pub(in crate::application::app) use commit::close_color_picker_standalone;
pub(in crate::application::app) use key::handle_color_picker_key;
pub(in crate::application::app) use mouse::handle_color_picker_mouse_move;
pub(in crate::application::app) use open::{open_color_picker_contextual, open_color_picker_standalone};
pub(in crate::application::app) use rebuild::rebuild_color_picker_overlay;
