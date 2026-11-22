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
//! and takes the font lock only through the
//! [`FontSystemLease`](crate::font::fonts::FontSystemLease) the caller
//! passes: a function that reads a font takes it from the lease, and a
//! placement gives it back for the work that reads none (#308). Sizes
//! arrive in points;
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

pub use curve::{chain_point, Curve};
pub use outline::{offset, offset_keeping_corners, outline, ring_offset, Outline, Rect};
/// The arc-length walk, for the projection's single-point questions — a
/// label at a fraction of its curve, a portal end at a fraction of its
/// outline (V1-B7) — answered by the walk placement itself uses.
pub(crate) use path::ArcPath;
/// A curve in its own frame, in `f64`: what a path's distances are
/// measured on, its table's and the projection's samples' (#313, #356).
pub(crate) use path::Local;
pub use path::{MAX_PATH_LENGTH, MAX_TABLE_SAMPLES};
/// The glyph steps a directional ring's one reshape charges, before its
/// lay reads its budget: what the projection harness's bound counts for
/// such a ring (#387).
pub(crate) use place::directional::reshape_steps;
/// The glyph steps a directional ring charges before its lay reads its
/// budget, its reshape and its inking: what the flow harnesses bound such
/// a ring's work by before they lay it (#387).
pub(crate) use place::directional::setup_steps;
/// The last directional ring laid on this thread, its glyphs and its work,
/// a test counter: what the projection harness's bound is held to for a
/// ring that lays text by direction (ruling B47.2).
#[cfg(test)]
pub(crate) use place::directional::LAST_DIRECTIONAL;
/// The last directional ring's reshape, as it charged it, a test counter:
/// what the projection harness's count is held to (#387).
#[cfg(test)]
pub(crate) use place::directional::LAST_RESHAPE;
/// A run with every glyph's ink measured, text too: what a text ring's
/// paths stop at (#296).
pub(crate) use place::halves::inked;
/// The last frame laid by halves on this thread, its text's glyphs and its
/// work, a test counter: what the projection harness's bound holds a text
/// frame's work to (ruling B47.3).
#[cfg(test)]
pub(crate) use place::halves::LAST_FRAME;
/// The glyph steps a `.notdef` pass is weighed at, a test's yardstick for
/// a lay that paid none (#387).
#[cfg(test)]
pub(crate) use place::halves::SHAPE_NOTDEF_STEPS;
/// The last `Repeat` ring laid round a closed path on this thread, a test
/// counter (#310): what the projection harness's bound is held to
/// (ruling B47.1).
#[cfg(test)]
pub(crate) use place::LAST_RING;
/// The most glyph steps a ring's placement has taken on this thread since
/// it was cleared, a test and fuzz counter: what the projection harness
/// holds each ring's work to (ruling B47.3).
#[cfg(any(test, fuzzing))]
pub(crate) use place::MOST_RING_LAY;
pub use place::{
    pattern_gap, place_at_point, place_on_closed, place_on_open, place_pitched, PathEnd, PlacedGlyph,
    PlacedRun, RingLayout, MAX_PLACED_GLYPHS,
};
/// Whether shaping hands a character to cosmic-text as a space, a run
/// being one line: what the projection harness's bound reads a ring's
/// line breaks as (#387).
pub(crate) use shape::is_paragraph_separator;
/// The `.notdef` passes every reshape that shaped paid, a test counter:
/// what a frame's lay pays, which the projection harness's exclusion is
/// held to (#387).
#[cfg(test)]
pub(crate) use shape::RESHAPE_NOTDEF_STEPS;
pub use shape::{shape_box, shape_run, GlyphMetrics, Ink, ShapedBox, ShapedRun, FILLER_SPAN};
pub use span::{point_size, span_attrs, FlowSpan, LINE_HEIGHT_FACTOR};

/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8).
/// `#[allow(clippy::unwrap_used)]` because this is a tests tree
/// (§T2.2): it carries no `cfg(test)` gate, so the crate-root lint sees
/// it as shipped code; `util::unwrap_posture` excludes these trees by
/// path.
#[allow(clippy::unwrap_used)]
pub mod tests;
