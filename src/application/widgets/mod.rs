// SPDX-License-Identifier: MPL-2.0

//! Widget definitions loaded from embedded JSON.
//!
//! Today the glyph-wheel color picker's static structure (glyphs,
//! size scales, chip list, copy) lives in `color_picker.json`,
//! loaded once at startup. The pure-function layout math stays in
//! Rust (it depends on measured glyph advances and screen
//! dimensions that JSON can't express).

pub mod color_picker_widget;
