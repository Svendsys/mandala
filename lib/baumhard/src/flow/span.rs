// SPDX-License-Identifier: MPL-2.0

//! A flow's input — its spans, each a piece of text with a resolved
//! style — and the one place a resolved style becomes the cosmic-text
//! attributes it is shaped with.
//!
//! Every attribute a span carries is real here: its face, its size
//! (through [`Attrs::metrics`], so two spans of one line can differ in
//! size), its weight, its slant and its color. Nothing is collapsed to
//! a line-wide value and nothing passes through `ColorFontRegions`.

use crate::document::resolve::ResolvedText;
use crate::document::values::{FontSize, LinkTarget};
use crate::font::fonts::{clamp_font_metric, MIN_FONT_SIZE_PT};
use crate::font::{Attrs, Color, Family, FontWeight, Metrics, Style};

/// The ratio of a line's height to its font size: every span's line
/// height is its point size times this.
///
/// 1.2 is the typographic default cosmic-text and CSS both take for
/// unspecified leading, and the value the section layout has always
/// used. The document has no line-height field, so this is not a
/// fallback; it is the whole answer, and this is its one definition.
pub const LINE_HEIGHT_FACTOR: f32 = 1.2;

/// One piece of a flow's text with the style it is drawn in: the input
/// to [`shape_run`](crate::flow::shape_run) and
/// [`shape_box`](crate::flow::shape_box).
///
/// A flow is a slice of these in reading order. The index of a span in
/// that slice is what [`GlyphMetrics::span`](crate::flow::GlyphMetrics::span)
/// and [`ShapedBox::glyph_spans`](crate::flow::ShapedBox::glyph_spans)
/// report for each drawn glyph, so a hit on a glyph finds its span and,
/// through `link`, where it leads.
#[derive(Copy, Clone, Debug)]
pub struct FlowSpan<'a> {
    /// The text. A path flow is one line, so no paragraph separator in
    /// it (`\n`, `\r`, U+001C–U+001E, U+0085, U+2029) is drawn; a box
    /// flow breaks lines at `\n` and draws none of the others (D26,
    /// ruling R9.5).
    pub text: &'a str,
    /// The resolved style. Its size must be in points: a frame's `Rel`
    /// size is converted by the projection before it reaches shaping
    /// (#239 amendment 2), and a `Rel` that arrives anyway is a caller
    /// error ([`point_size`]).
    pub style: &'a ResolvedText,
    /// Where the span leads when it is clicked, if anywhere. Shaping
    /// does not read it; it rides along so the caller holds a span and
    /// its link together.
    pub link: Option<&'a LinkTarget>,
}

/// `style`'s size in points.
///
/// Shaping and placement receive point sizes; converting a frame's
/// [`Rel`](FontSize::Rel) size with the drawn rectangle is the
/// projection's job (#239 amendment 2). A `Rel` here is therefore a
/// caller error: it fails a `debug_assert!`, and in a release build it
/// shapes at [`MIN_FONT_SIZE_PT`] with a warning rather than guessing
/// a rectangle (ruling R6, reading 8).
///
/// A `Pt` size is forced into the shaper's domain with
/// [`clamp_font_metric`], so no style can hand cosmic-text a zero line
/// height. Cost: O(1).
pub fn point_size(style: &ResolvedText) -> f32 {
    match style.size {
        FontSize::Pt(points) => clamp_font_metric(points, MIN_FONT_SIZE_PT),
        FontSize::Rel(fraction) => {
            debug_assert!(
                false,
                "flow: a Rel size ({fraction}) reached shaping; the projection converts it to points"
            );
            log::warn!("flow: a relative size reached shaping; shaping at the minimum size instead");
            MIN_FONT_SIZE_PT
        }
    }
}

/// The cosmic-text attributes `span` is shaped with, at `scale` times
/// its size.
///
/// - **Face:** `Family::Name(family)` when `family` is `Some`, which is
///   how a caller pins the span's `AppFont` (it resolves the face's
///   family name once, against the font system it holds). `None` keeps
///   cosmic-text's default family — what an `AppFont::Any` span asks
///   for, or a face the font system does not know.
/// - **Size:** through [`Attrs::metrics`], at `scale ×` [`point_size`],
///   clamped to the shaper's domain, with a line height
///   [`LINE_HEIGHT_FACTOR`] times that.
/// - **Weight** from the style's [`Weight`](crate::document::values::Weight),
///   **slant** `Style::Italic` when the style is italic, and the
///   style's **color**.
///
/// Cost: O(1), no allocation; the returned attributes borrow `family`.
pub fn span_attrs<'f>(span: &FlowSpan<'_>, scale: f32, family: Option<&'f str>) -> Attrs<'f> {
    let size = clamp_font_metric(point_size(span.style) * scale, MIN_FONT_SIZE_PT);
    let [r, g, b, a] = span.style.color.0;
    let attrs = Attrs::new()
        .metrics(Metrics::new(size, size * LINE_HEIGHT_FACTOR))
        .weight(FontWeight(span.style.weight.get()))
        .style(if span.style.italic {
            Style::Italic
        } else {
            Style::Normal
        })
        .color(Color::rgba(r, g, b, a));
    match family {
        Some(name) => attrs.family(Family::Name(name)),
        None => attrs,
    }
}
