// SPDX-License-Identifier: MPL-2.0

//! Cosmic-text font integration — the single blessed boundary
//! between baumhard's styled-region data model and the underlying
//! font system. `fonts` owns the compiled font table and the shared
//! `FONT_SYSTEM`; `attrs` translates `ColorFontRegions` into
//! cosmic-text `AttrsList`s.

/// `ColorFontRegions` → cosmic-text bridges (`attrs_list_from_regions`
/// for `Editor::insert_string`, `RegionFamilies` +
/// `rich_text_spans_from_regions` for `Buffer::set_rich_text`).
pub mod attrs;
/// Compiled-in font table, shared `FONT_SYSTEM`, cosmic-text editor
/// factories, and the text-measurement primitives.
pub mod fonts;
/// Hex-string → `cosmic_text::Color` bridge — the single entry point
/// renderer code uses to resolve a theme-variable hex into the
/// cosmic-text colour type without importing `cosmic_text` itself.
pub mod hex;
/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8).
pub mod tests;

/// Packed-RGBA colour, re-exported from `cosmic_text::Color` so
/// callers outside the renderer reach the cosmic-text type
/// without importing `cosmic_text` directly (§1).
pub use cosmic_text::Color;

/// Glyph-rasterization cache, re-exported from
/// `cosmic_text::SwashCache`. Owned by the caller — one per
/// measurement pass — so repeated calls share rasterization work.
pub use cosmic_text::SwashCache;

