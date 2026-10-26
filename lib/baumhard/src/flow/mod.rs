// SPDX-License-Identifier: MPL-2.0

//! TextFlow: all text the app draws, as one primitive (design §6, D8).
//!
//! > **A TextFlow is a sequence of spans plus a layout.**
//!
//! A section or a label is a **box** flow: its spans are shaped as
//! lines broken at `\n` only, in a box that grows to fit them (D26,
//! D5) — [`shape_box`](crate::flow::shape_box). Everything else is a
//! **path** flow: a frame's rings run around the node's
//! [outline](crate::flow::outline()), a connector's or link's content
//! along its [curve](crate::flow::Curve), a portal marker at a point. A
//! path flow is shaped once as a single line
//! ([`shape_run`](crate::flow::shape_run)) and then placed
//! ([`place_on_closed`](crate::flow::place_on_closed),
//! [`place_on_open`](crate::flow::place_on_open),
//! [`place_at_point`](crate::flow::place_at_point)): **shape once,
//! place many**. Moving a node re-places its flows; nothing is reshaped
//! unless the text, its fonts or its sizes changed, or `Fit` rescales
//! it.
//!
//! **Glyphs are never rotated** (D9). A curve is made by *positioning*
//! upright glyphs, each taking a step along the path that blends its
//! advance and its line height by the direction of travel — the way
//! ASCII art makes a curve. Box-drawing glyphs (`─`, `━`, `═`) are
//! ordinary content.
//!
//! What a drawn glyph came from is never lost: every shaped glyph names
//! its span and its bytes of that span's text
//! ([`GlyphMetrics`](crate::flow::GlyphMetrics),
//! [`ShapedBox::glyph_spans`](crate::flow::ShapedBox::glyph_spans)),
//! which is how a click on a linked span finds its link.
//!
//! This module is pure functions and the types they return. It builds
//! no glyph tree, caches nothing (the projection, V1-B7, owns caches)
//! and never takes the font lock: every shaping function takes the
//! `FontSystemWriteGuard` the caller holds. Sizes arrive in points;
//! converting a frame's relative size is the projection's job.
//!
//! Intra-doc links here are crate-absolute because `lib.rs` also puts
//! an outer `///` on this module (CODE_CONVENTIONS §8).

mod curve;
mod outline;
mod path;
mod place;
mod shape;
mod span;

pub use curve::Curve;
pub use outline::{offset, outline, ring_offset, Outline, Rect};
pub use path::MAX_TABLE_SAMPLES;
pub use place::{place_at_point, place_on_closed, place_on_open, PlacedGlyph, PlacedRun, MAX_PLACED_GLYPHS};
pub use shape::{shape_box, shape_run, GlyphMetrics, ShapedBox, ShapedRun, FILLER_SPAN};
pub use span::{point_size, span_attrs, FlowSpan, LINE_HEIGHT_FACTOR};

/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8).
/// `#[allow(clippy::unwrap_used)]` because this is a tests tree
/// (§T2.2): it carries no `cfg(test)` gate, so the crate-root lint sees
/// it as shipped code; `util::unwrap_posture` excludes these trees by
/// path.
#[allow(clippy::unwrap_used)]
pub mod tests;
