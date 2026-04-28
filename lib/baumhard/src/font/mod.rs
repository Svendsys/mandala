// SPDX-License-Identifier: MPL-2.0

//! Cosmic-text font integration — the single blessed boundary
//! between baumhard's styled-region data model and the underlying
//! font system. `fonts` owns the compiled font table and the shared
//! `FONT_SYSTEM`; `attrs` translates `ColorFontRegions` into
//! cosmic-text `AttrsList`s.

pub mod attrs;
pub mod fonts;
pub mod tests;

/// Packed-RGBA colour, re-exported from `cosmic_text::Color` so
/// callers outside the renderer reach the cosmic-text type
/// without importing `cosmic_text` directly (§1).
pub use cosmic_text::Color;

/// Glyph-rasterization cache, re-exported from
/// `cosmic_text::SwashCache`. Owned by the caller — one per
/// measurement pass — so repeated calls share rasterization work.
pub use cosmic_text::SwashCache;

