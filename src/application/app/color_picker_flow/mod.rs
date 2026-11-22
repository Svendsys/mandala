// SPDX-License-Identifier: MPL-2.0

//! Glyph-wheel color picker flow: the picker's open path, its key,
//! click and mouse routing, its geometry, the overlay rebuild
//! (`rebuild_color_picker_overlay`), and its session on the one writer
//! ([`session`]): the contextual picker is an interactive transaction —
//! opened with `begin`, its color staged on its target once a frame,
//! committed by the ࿕ click and cancelled by Esc or a click outside it
//! (V1-C7, #248). The standalone palette holds none.
//!
//! The picker's Actions (`OpenColorPicker`, `CloseColorPicker`,
//! `Picker*`) reach [`session::dispatch`] through the transactions
//! family (`dispatch::families::transactions`). The widget's own input —
//! the hover, a click on a cell, the wheel's move and resize, a nudge —
//! changes what the wheel shows and nothing else: only the session's
//! drain writes the document.

mod click;
mod geometry;
mod key;
mod mouse;
mod open;
mod rebuild;
pub(in crate::application::app) mod session;

pub(in crate::application::app) use click::{
    end_color_picker_gesture, handle_color_picker_click, PickerClick,
};
pub(in crate::application::app) use key::{handle_color_picker_clipboard_key, picker_op_for};
pub(in crate::application::app) use mouse::handle_color_picker_mouse_move;
pub(in crate::application::app) use rebuild::rebuild_color_picker_overlay;
pub(in crate::application::app) use session::dispatch;
