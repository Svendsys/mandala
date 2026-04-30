// SPDX-License-Identifier: MPL-2.0

//! Edge mutations on `MindMapDocument` — every `set_edge_*` /
//! `reset_edge_*` / hit-test-handle method, sorted by which
//! conceptual axis they touch:
//!
//! - [`structural`]: hit-testing, position reset, anchor/curve
//!   toggles, edge-index lookup. Houses the shared helpers
//!   (`mutate_edge`, `commit_throttled_edge_drag`) that every
//!   per-axis setter routes through.
//! - [`style`]: visual styling — body glyph, caps, color, font
//!   sizing/family, spacing.
//! - [`label`]: edge label text, position-along-curve, and
//!   perpendicular offset.
//! - [`mode`]: edge type, display-mode, and style-reset toggles.
//! - [`portal`]: portal-edge lifecycle and portal-label
//!   mutations.
//! - [`inline`]: free-fn helpers (`ensure_glyph_connection_inline`,
//!   `option_f32_eps_eq`, `write_endpoint_field`, ...) reachable
//!   from `mutate_edge` closures that can't capture `Self`. The
//!   first style edit on a stock edge forks its
//!   `GlyphConnectionConfig` off the canvas defaults via
//!   `ensure_glyph_connection_inline` here before writing to it.

mod inline;
mod label;
mod mode;
mod portal;
mod structural;
mod style;

#[cfg(test)]
mod tests;
