// SPDX-License-Identifier: MPL-2.0

//! Glyph-wheel color picker flow: the picker's open path, its key,
//! click and mouse routing, its geometry and the overlay rebuild
//! (`rebuild_color_picker_overlay`).
//!
//! **Unreachable until V1-C7 (#248).** The picker's Actions
//! (`OpenColorPicker`, `CloseColorPicker`, `Picker*`) are V1-C7's
//! family (`dispatch::families::transactions`), which rebuilds the
//! picker as a transaction: open is `Document::begin`, each hover
//! change stages the color op for the target, a click commits and a
//! close or cancel cancels. What is here is the widget's own input
//! handling, with no document anywhere in it; the places a stage
//! belongs are marked.

mod click;
mod geometry;
mod key;
mod mouse;
mod open;
mod rebuild;

pub(in crate::application::app) use rebuild::rebuild_color_picker_overlay;
