// SPDX-License-Identifier: MPL-2.0

//! Shaping: a flow's spans become one cosmic-text [`Buffer`], shaped
//! once, plus the per-glyph facts placement and hit-testing read.
//!
//! Two shapes come out of it:
//!
//! - A [`ShapedRun`] is **one line**, never wrapped: the input of the
//!   path and point flows, whose glyphs [`place`](crate::flow::place)
//!   positions one by one.
//! - A [`ShapedBox`] is a block of lines broken at `\n` only (D26),
//!   sized to fit its text and at least its minimum size, its lines
//!   aligned within its width: the section and label flows.
//!
//! Neither keeps a per-glyph buffer. A run's glyphs are listed in
//! [`ShapedRun::glyphs`] in the order the buffer lays them out, so the
//! renderer can walk [`ShapedRun::layout_glyphs`] beside a
//! [`PlacedRun`](crate::flow::PlacedRun) and draw glyph *i* of the one
//! buffer at placed position *i*.
//!
//! **Span mapping.** cosmic-text reports each glyph's cluster as bytes
//! of the *line* it shaped, and the line is not the caller's text: the
//! spans are concatenated, and no paragraph separator (the private
//! `is_paragraph_separator` names them) reaches it but a box's `\n`
//! (D26, ruling R9.5): a box leaves the others out, and a run lays each
//! one — a `\r\n` pair as one — as a space that maps back to it, marked
//! [`GlyphMetrics::line_break`] (ruling R27). Shaping
//! therefore records a piece table — which bytes of which span each
//! stretch of each line came from — and maps every glyph back through
//! it, so [`GlyphMetrics::span`] and [`GlyphMetrics::bytes`] name the
//! caller's span and bytes of the caller's text.
//!
//! **Faces** (ruling R30 on #278). A span pinned to an `AppFont` is
//! handed to cosmic-text with its family's name and the nearest weight
//! the family's compiled faces have ([`PinnedFamily`], resolved once per
//! shaping by the private `families_of`), never the weight it asked for:
//! cosmic-text keeps a pinned family only at an exact weight, and draws a
//! weight the family lacks in whatever host face has it. So a span is
//! drawn in its family's compiled faces on every machine, and the tab
//! check below measures the face the tab is shaped in.
//!
//! **Tabs** (rulings R24, R24.1 and R24.2 on #278). A tab is drawn as
//! cosmic-text draws it: a space widened to the next tab stop, eight of
//! the face's spaces apart from the start of its line. A face whose
//! space has no width cannot measure a stop — cosmic-text would give the
//! tab, and every glyph after it on its line, a `NaN` advance and
//! position — so the decision is made **per span, from that span's face,
//! before shaping**: a span whose face's space has no finite, positive
//! advance has its tabs drawn as spaces (one byte for one, so its span
//! bytes are unchanged), and every other span keeps its tab stops, on
//! its own line or beside it.
//!
//! That decision measures a span's primary face, and a tab cosmic-text
//! shapes in a fallback face can still come back non-finite. So the
//! laid-out widths and positions of text that holds a tab are checked
//! after shaping, as a **last resort** (R24.2): when one is not finite,
//! the text is laid out again with its tabs as spaces — on the failing
//! lines only, in a box, so a line that came out finite keeps its stops;
//! on the whole run, for a run — and the event is logged (`warn!`). No
//! `NaN` from a tab reaches placement or the renderer.

use std::ops::Range;
use std::sync::Arc;

use glam::Vec2;

use crate::document::resolve::ResolvedText;
use crate::document::values::Align;
use crate::flow::span::{span_attrs, FlowSpan, LINE_HEIGHT_FACTOR};
use crate::font::fonts::{
    clamp_font_metric, glyph_outline_ink, pinned_family, AppFont, FontSystemLease, PinnedFamily,
    MIN_FONT_SIZE_PT,
};
use crate::font::{Attrs, Buffer, FontSystem, LayoutGlyph, Metrics, SwashCache, Wrap, SHAPING_ADVANCED};
use crate::util::grapheme_chad;

/// The span index a filler glyph reports: it belongs to no span of the
/// flow, so it is never a link (ruling R6).
///
/// A `FitMode::Once` filler is drawn in the flow's own style, not any
/// span's, and is appended to the run by placement
/// ([`place_on_closed`](crate::flow::place_on_closed),
/// [`place_on_open`](crate::flow::place_on_open)); its glyphs carry
/// this value in [`GlyphMetrics::span`], and their
/// [`GlyphMetrics::bytes`] index the filler text as drawn, its
/// paragraph separators left out.
/// A glyph whose span cannot be found reports it too, so it is never
/// taken for a link.
pub const FILLER_SPAN: usize = usize::MAX;

/// What shaping measured about one glyph of a [`ShapedRun`].
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphMetrics {
    /// The index of the span the glyph's cluster starts in, in the slice
    /// given to [`shape_run`]; [`FILLER_SPAN`] for a filler glyph. A
    /// cluster that straddles two spans (a combining mark styled apart
    /// from its base) belongs to the span it starts in.
    pub span: usize,
    /// The glyph's cluster as bytes of that span's text, clipped to the
    /// span. Every glyph of one cluster reports the same range, and a
    /// ligature reports all the characters it covers.
    pub bytes: Range<usize>,
    /// The horizontal advance, in points, at the size it was shaped at.
    pub advance: f32,
    /// The font size, in points, the glyph was shaped at. `Fit` reads it
    /// to predict the glyph's advance at another scale: the reshape
    /// floors every size at [`MIN_FONT_SIZE_PT`], so a glyph's advance
    /// does not shrink with the scale below that size.
    pub size: f32,
    /// The glyph's pen position in its shaped line, in points from the
    /// line's left edge. Placement sets each glyph of a grapheme cluster
    /// at its pen position relative to the cluster's leftmost, so a mark
    /// sits on its base as shaped (ruling R9.1) — which is not always its
    /// base's advance on: a right-to-left mark shares its letter's pen
    /// position.
    pub x: f32,
    /// The face's ascent at the glyph's size: how far above the
    /// baseline the face reaches.
    pub ascent: f32,
    /// The face's descent at the glyph's size, positive: how far below
    /// the baseline the face reaches.
    pub descent: f32,
    /// Whether this glyph begins a grapheme cluster, in the (visual) order
    /// the glyphs are listed. A glyph that continues the previous glyph's
    /// cluster (a mark shaped as a glyph of its own) is `false`, so a
    /// cut that keeps whole graphemes keeps whole runs of glyphs from
    /// one `true` to the next (ruling R6, reading 1).
    pub grapheme_start: bool,
    /// Whether the text may be parted between this glyph and the one
    /// before it in visual order without tearing a word (#291, ruling
    /// R26): a Unicode word boundary (UAX #29) with whitespace on one
    /// side or a word on both (as between two ideographs), so "Sand,"
    /// and "well-known" stay whole. `true` at a line's first glyph and
    /// where the two glyphs are not neighbors in the text (a bidi
    /// reorder); `false` inside a grapheme cluster. Placement keeps a
    /// word in one stretch with it, and parts a frame's two lines at one
    /// (ruling R27).
    pub word_break: bool,
    /// Whether the glyph's grapheme is whitespace: what a frame's two
    /// lines drop where they part, and at their ends (ruling R27).
    pub blank: bool,
    /// Whether the glyph is the space a run lays where its text held a
    /// line break (a paragraph separator: `\n`, `\r\n`, U+2029 and the
    /// rest). Its [`bytes`](Self::bytes) are the break's. A frame's
    /// two-line layout parts its lines at the first one (ruling R27);
    /// any other path flow reads it as the space it is.
    pub line_break: bool,
    /// Whether the glyph's grapheme is a letter or a digit
    /// (alphanumeric): text a reader reads in a direction. A flow with no
    /// such glyph — an ornament, `✦ ·` or `─` — has no reading direction,
    /// and placement does not reverse any part of it (#291, ruling R29).
    pub letter: bool,
    /// Where the glyph's ink lies, measured from its outline (#291,
    /// ruling R31): what placement steps an ornament's glyphs by, so a
    /// ring's gaps are the gaps a reader sees. Measured only in a run
    /// that is an ornament — no glyph a [`letter`](Self::letter) — since
    /// text keeps its line-height boxes (ruling R9.2); `None` there, and
    /// for a glyph with no outline (a space, a bitmap-only emoji).
    pub ink: Option<Ink>,
}

/// A glyph's ink: the bounding box of its outline, relative to its pen
/// on the baseline, in points at the size it was shaped at, `y` pointing
/// down — where a renderer drawing the glyph at that pen puts its ink,
/// the shaper's offsets included
/// ([`glyph_outline_ink`]).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Ink {
    /// The box's top-left corner: its left edge, and how far above the
    /// baseline the ink reaches (negative above it).
    pub min: Vec2,
    /// The box's bottom-right corner.
    pub max: Vec2,
}

/// A flow shaped as one line, for placement along a path or at a point.
///
/// Built by [`shape_run`]; the glyph placement functions return a new
/// one when they rescale it (`Fit`) or append a filler (`Once`).
#[derive(Clone, Debug)]
pub struct ShapedRun {
    /// The shaped text. One line, or two after placement appended a
    /// `Once` filler as a line of its own (so the filler and the text
    /// never shape or reorder across each other). Shared: placing a run
    /// many times, and repeating it along a path, never copies or
    /// reshapes it.
    pub buffer: Arc<Buffer>,
    /// One entry per glyph, in the order [`ShapedRun::layout_glyphs`]
    /// yields the buffer's glyphs: line by line, each line in visual
    /// order, left to right by pen position. (cosmic-text lists a
    /// right-to-left line's glyphs right to left; they are reversed
    /// here.) [`PlacedGlyph::glyph`](crate::flow::PlacedGlyph::glyph)
    /// indexes this.
    pub glyphs: Vec<GlyphMetrics>,
    /// The run's line height: the largest of its spans' (point size ×
    /// [`LINE_HEIGHT_FACTOR`]). Placement gives every glyph a box this
    /// tall (`h` in #239 §5), so one run keeps one vertical rhythm.
    /// `0.0` for a run with nothing to draw.
    pub line_height: f32,
    /// Which bytes of which span each stretch of each line came from:
    /// how a glyph is mapped back to its span, and how the run is
    /// reshaped (rescaled, or given a filler) without its spans.
    pub(crate) pieces: Vec<Piece>,
}

/// A contiguous stretch of one shaped line that came from one span.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Piece {
    /// The span index, or [`FILLER_SPAN`].
    pub(crate) span: usize,
    /// Where the stretch starts in the span's text.
    pub(crate) span_start: usize,
    /// The buffer line it is on.
    pub(crate) line: usize,
    /// Where the stretch starts in that line's text.
    pub(crate) line_start: usize,
    /// Its length in bytes.
    pub(crate) len: usize,
    /// For the space a run lays where its text held a line break
    /// (#291, ruling R27), the break's length in bytes of the span: the
    /// space maps back to the break. 0 for every other stretch.
    pub(crate) line_break: usize,
}

impl ShapedRun {
    /// The buffer's laid-out glyphs, in the order [`ShapedRun::glyphs`]
    /// lists them: glyph *i* here is the glyph `glyphs[i]` describes.
    /// This is what a renderer draws at each
    /// [`PlacedGlyph`](crate::flow::PlacedGlyph)'s position.
    ///
    /// Cost: a lazy walk over the buffer's layout lines; no allocation.
    /// Collect it once per use: finding glyph *i* by `nth` walks *i*
    /// glyphs.
    pub fn layout_glyphs(&self) -> impl Iterator<Item = &LayoutGlyph> + '_ {
        self.buffer
            .layout_runs()
            .flat_map(|run| visual(run.glyphs, run.rtl))
    }

    /// Where the run's baseline sits below the top of a glyph's placed
    /// box, which is [`ShapedRun::line_height`] tall: the line height's
    /// slack split evenly above and below the run's tallest ascent and
    /// deepest descent, as cosmic-text centers a line.
    ///
    /// One value for the whole run, so glyphs of different sizes on one
    /// stretch of path share a baseline, as they would on a line of
    /// text. A renderer draws glyph *i* with its baseline at
    /// `placed.pos.y + baseline()`. `0.0` for an empty run.
    ///
    /// Cost: O(glyphs).
    pub fn baseline(&self) -> f32 {
        let ascent = self.glyphs.iter().map(|g| g.ascent).fold(0.0_f32, f32::max);
        let descent = self.glyphs.iter().map(|g| g.descent).fold(0.0_f32, f32::max);
        if self.glyphs.is_empty() {
            return 0.0;
        }
        (self.line_height - (ascent + descent)) / 2.0 + ascent
    }
}

/// A flow shaped as a block of lines, for box layout.
#[derive(Clone, Debug)]
pub struct ShapedBox {
    /// The shaped text, laid out at the box's width with its lines
    /// aligned: drawn with its top-left at the box's origin, it shows
    /// the box.
    pub buffer: Arc<Buffer>,
    /// The box's size: the larger of the minimum size asked for and the
    /// text's natural size (its widest line by the sum of its line
    /// heights), per axis.
    pub size: Vec2,
    /// For each glyph, in layout order (line by line, each line in
    /// visual order — the order `buffer.layout_runs()` yields them), the
    /// span its cluster starts in and its cluster as bytes of that
    /// span's text: how a hit on a glyph finds its span.
    pub glyph_spans: Vec<(usize /* span */, Range<usize> /* bytes */)>,
}

/// What becomes of a flow's paragraph separators
/// ([`is_paragraph_separator`]): a box flow breaks its line at `\n` (D26,
/// ruling R9.5) and leaves the others out; a path flow is one line, so it
/// breaks at none and lays each as a space (ruling R27).
#[derive(Copy, Clone, PartialEq)]
enum Breaks {
    /// Every paragraph separator is laid as a space marked a line break;
    /// a `\r\n` pair is one.
    None,
    /// `\n` breaks the line; every other paragraph separator is left out.
    Newline,
}

/// Whether `ch` is a paragraph separator: Unicode bidi class B —
/// U+000A (`\n`), U+000D (`\r`), U+001C–U+001E, U+0085 (NEL) and U+2029
/// (PARAGRAPH SEPARATOR).
///
/// cosmic-text starts a new line at every one of them, which would break
/// a run into two lines and a box where D26 does not, and would strand
/// the glyphs after it outside the piece table. So shaping never hands
/// one to cosmic-text, except the `\n` a box breaks at (ruling R9.5).
fn is_paragraph_separator(ch: char) -> bool {
    matches!(
        ch,
        '\n' | '\r' | '\u{1C}' | '\u{1D}' | '\u{1E}' | '\u{85}' | '\u{2029}'
    )
}

/// The text and attributes handed to cosmic-text, and the piece table
/// that maps what it lays out back to the caller's spans.
struct Assembly<'f> {
    /// `(text, attrs)` pairs for cosmic-text's `set_rich_text`.
    rich: Vec<(&'f str, Attrs<'f>)>,
    /// One entry per drawn stretch, in line order.
    pieces: Vec<Piece>,
    /// The largest line height of any drawn stretch: a run's line
    /// height.
    line_height: f32,
    /// The largest font size of any drawn stretch.
    font_size: f32,
    /// The largest line height and font size of any span, drawn or not:
    /// how tall a line with nothing on it is.
    span_metrics: Metrics,
}

/// The size and line height `attrs` carry.
fn metrics_of(attrs: &Attrs<'_>) -> Metrics {
    attrs
        .metrics_opt
        .map(Metrics::from)
        .unwrap_or(Metrics::new(0.0, 0.0))
}

/// Split each span into the stretches that are drawn, attach its
/// attributes, and record where each stretch lands.
///
/// With `Breaks::Newline`, each `\n` is handed to cosmic-text as a
/// paragraph break, and a flow whose last break has nothing after it
/// gets one more, so it ends with the empty line an editor shows there
/// (cosmic-text drops a trailing empty line; D26 breaks at every `\n`).
///
/// `families` holds each span's pinned family (same indexing as
/// `spans`). Cost: O(text) scan for line breaks, one `Vec` entry per
/// stretch.
fn assemble<'f>(
    spans: &[FlowSpan<'f>],
    families: &'f [Option<PinnedFamily>],
    scale: f32,
    breaks: Breaks,
) -> Assembly<'f> {
    let mut assembly = Assembly {
        rich: Vec::with_capacity(spans.len()),
        pieces: Vec::with_capacity(spans.len()),
        line_height: 0.0,
        font_size: 0.0,
        span_metrics: Metrics::new(0.0, 0.0),
    };
    let mut line = 0usize;
    let mut line_len = 0usize;
    // The attributes of the last break, while nothing has been drawn
    // after it.
    let mut open_break: Option<Attrs<'f>> = None;
    for (index, span) in spans.iter().enumerate() {
        let attrs = span_attrs(span, scale, families[index].as_ref());
        let metrics = metrics_of(&attrs);
        assembly.span_metrics.font_size = assembly.span_metrics.font_size.max(metrics.font_size);
        assembly.span_metrics.line_height = assembly.span_metrics.line_height.max(metrics.line_height);
        let text = span.text;
        let push = |assembly: &mut Assembly<'f>,
                    open_break: &mut Option<Attrs<'f>>,
                    start: usize,
                    end: usize,
                    line: usize,
                    line_len: &mut usize| {
            if start >= end {
                return;
            }
            *open_break = None;
            assembly.rich.push((&text[start..end], attrs.clone()));
            assembly.pieces.push(Piece {
                span: index,
                span_start: start,
                line,
                line_start: *line_len,
                len: end - start,
                line_break: 0,
            });
            *line_len += end - start;
            assembly.line_height = assembly.line_height.max(metrics.line_height);
            assembly.font_size = assembly.font_size.max(metrics.font_size);
        };
        let mut start = 0usize;
        for (at, ch) in text.match_indices(is_paragraph_separator) {
            push(&mut assembly, &mut open_break, start, at, line, &mut line_len);
            if breaks == Breaks::None && ch == "\n" && at > 0 && at == start && text[..at].ends_with('\r') {
                // The second of a pair (`\r\n`): one break, one space.
                if let Some(space) = assembly.pieces.last_mut().filter(|piece| piece.line_break > 0) {
                    space.line_break += ch.len();
                }
            } else if breaks == Breaks::None {
                // A run is one line: its break is laid as a space, which
                // a frame's two-line layout parts its lines at (ruling
                // R27) and any other path flow reads as a space.
                assembly.rich.push((" ", attrs.clone()));
                assembly.pieces.push(Piece {
                    span: index,
                    span_start: at,
                    line,
                    line_start: line_len,
                    len: 1,
                    line_break: ch.len(),
                });
                line_len += 1;
                assembly.line_height = assembly.line_height.max(metrics.line_height);
                assembly.font_size = assembly.font_size.max(metrics.font_size);
            }
            if breaks == Breaks::Newline && ch == "\n" {
                // The break itself is handed to cosmic-text so it splits
                // the paragraph there; it belongs to no piece.
                assembly.rich.push(("\n", attrs.clone()));
                open_break = Some(attrs.clone());
                line += 1;
                line_len = 0;
            }
            start = at + ch.len();
        }
        push(
            &mut assembly,
            &mut open_break,
            start,
            text.len(),
            line,
            &mut line_len,
        );
    }
    if let Some(attrs) = open_break {
        assembly.rich.push(("\n", attrs));
    }
    assembly
}

/// Each span's pinned family and its compiled weights, resolved once against `fs`:
/// `None` for `AppFont::Any`, or a face the font system does not know.
fn families_of<'s>(
    styles: impl Iterator<Item = &'s ResolvedText>,
    fs: &FontSystem,
) -> Vec<Option<PinnedFamily>> {
    styles
        .map(|style| match style.font {
            AppFont::Any => None,
            font => pinned_family(fs, font),
        })
        .collect()
}

/// A buffer with no wrapping, no size limit and metrics no smaller than
/// the shaper accepts, holding `rich`: a box's lines when `align` is
/// given, a run otherwise.
///
/// Its tabs have already been decided per span ([`untabbed`]). When the
/// text holds a tab, the laid-out widths and positions are checked, and
/// when one is not finite — a face [`space_measures`] passed whose tab
/// stop still fails, such as a fallback face — the text is laid out
/// again with its tabs as spaces, byte for byte: in a box on the lines
/// that failed alone, in a run throughout (R24.2). The event is logged.
///
/// Cost: one shaping pass, and one pass over the laid-out glyphs when
/// the text holds a tab; a second shaping pass, and one copy of each
/// stretch that holds a tab on a failing line, only when that check
/// fails.
fn new_buffer(
    fs: &mut FontSystem,
    rich: Vec<(&str, Attrs<'_>)>,
    font_size: f32,
    line_height: f32,
    align: Option<Align>,
) -> Buffer {
    let font_size = font_size.max(MIN_FONT_SIZE_PT);
    let line_height = line_height.max(MIN_FONT_SIZE_PT * LINE_HEIGHT_FACTOR);
    let metrics = Metrics::new(font_size, line_height);
    if !rich.iter().any(|(text, _)| text.contains('\t')) {
        return filled_buffer(fs, rich, metrics, align);
    }
    let buffer = filled_buffer(fs, rich.clone(), metrics, align);
    let failing = non_finite_lines(&buffer);
    if failing.is_empty() {
        return buffer;
    }
    let whole = align.is_none();
    log::warn!(
        "flow: a tab stop laid out a width or position that is not finite; laying out {} with its tabs as spaces",
        if whole { "the run" } else { "the failing lines" }
    );
    #[cfg(test)]
    TABS_RELAID.with(|count| count.set(count.get() + 1));
    // The line each stretch is on: a box's lines are split by the `\n`
    // stretches `assemble` hands over.
    let mut line = 0usize;
    let spaced: Vec<Option<String>> = rich
        .iter()
        .map(|(text, _)| {
            let on = line;
            if *text == "\n" {
                line += 1;
            }
            ((whole || failing.contains(&on)) && text.contains('\t')).then(|| text.replace('\t', " "))
        })
        .collect();
    let relaid: Vec<(&str, Attrs<'_>)> = rich
        .iter()
        .zip(&spaced)
        .map(|((text, attrs), spaced)| (spaced.as_deref().unwrap_or(*text), attrs.clone()))
        .collect();
    let buffer = filled_buffer(fs, relaid, metrics, align);
    if !non_finite_lines(&buffer).is_empty() {
        log::warn!(
            "flow: a laid-out width or position is still not finite with every failing tab as a space"
        );
    }
    buffer
}

#[cfg(test)]
thread_local! {
    /// Test hook (R24.2): while it names a face on a thread, the
    /// last-resort check ([`non_finite_lines`]) counts every tab glyph
    /// laid out in that face as a width that is not finite — what a
    /// fallback face's `0 / 0` tab stop is. So the check, and the second
    /// layout it leads to, are driven the same way on every machine,
    /// whatever its font set makes of a real face's tab.
    pub(crate) static TABS_NOT_FINITE_IN: std::cell::Cell<Option<AppFont>> =
        const { std::cell::Cell::new(None) };
    /// Test hook (R24.2): while set on a thread, [`space_measures`]
    /// passes every face, so a tab in a face that cannot measure its stop
    /// (SilhousTwo, drawn in SilhousTwo on every machine since R30)
    /// reaches cosmic-text, as one shaped in a fallback face would, and
    /// is laid out as a real `0 / 0` that only the last-resort check in
    /// [`new_buffer`] stands between and placement.
    pub(crate) static TRUST_EVERY_FACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test counter (R24.2): how many times [`new_buffer`] has laid text
    /// out again with its tabs as spaces on this thread.
    pub(crate) static TABS_RELAID: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test hook (#304): while set on a thread, [`reshape`] shapes a
    /// size-only rescale again rather than laying out the run's shaping
    /// at the new size ([`resized_buffer`]): the reference the
    /// resized buffer is held to.
    pub(crate) static RESHAPE_BY_SHAPING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The lines of `buffer` the last-resort check finds failing: what a test
/// of the check reads back.
#[cfg(test)]
pub(crate) fn failing_lines(buffer: &Buffer) -> Vec<usize> {
    non_finite_lines(buffer)
}

/// Whether the test hook [`TABS_NOT_FINITE_IN`] counts `glyph`, laid out
/// on a line of text `line`, as not finite: a tab in the face it names.
#[cfg(test)]
fn hooked_tab(line: &str, glyph: &LayoutGlyph) -> bool {
    TABS_NOT_FINITE_IN.with(std::cell::Cell::get).is_some_and(|font| {
        line.get(glyph.start..glyph.end) == Some("\t")
            && crate::font::fonts::COMPILED_FONT_ID_MAP
                .get(&font)
                .is_some_and(|ids| ids.contains(&glyph.font_id))
    })
}

/// A buffer at `metrics` holding `rich`, shaped and laid out unwrapped.
///
/// The buffer starts empty (`Buffer::new_empty`), not as `Buffer::new`
/// makes one: that shapes an empty line, and its ellipsis in the default
/// family, for `set_rich_text` to throw away (#304).
fn filled_buffer(
    fs: &mut FontSystem,
    rich: Vec<(&str, Attrs<'_>)>,
    metrics: Metrics,
    align: Option<Align>,
) -> Buffer {
    let mut buffer = Buffer::new_empty(metrics);
    buffer.set_wrap(Wrap::None);
    buffer.set_size(None, None);
    let defaults = Attrs::new().metrics(metrics);
    buffer.set_rich_text(rich, &defaults, SHAPING_ADVANCED, align.map(cosmic_align));
    buffer.shape_until_scroll(fs, false);
    buffer
}

/// Whether the face `attrs` select can measure a tab stop: its space,
/// shaped alone, has a finite, positive advance (cosmic-text widens a tab
/// to a multiple of it, so a zero advance is a `0 / 0` stop).
///
/// Cost: one shaping of one character.
fn space_measures(attrs: &Attrs<'_>, fs: &mut FontSystem) -> bool {
    #[cfg(test)]
    if TRUST_EVERY_FACE.with(std::cell::Cell::get) {
        return true;
    }
    let metrics = metrics_of(attrs);
    let size = metrics.font_size.max(MIN_FONT_SIZE_PT);
    let metrics = Metrics::new(size, size * LINE_HEIGHT_FACTOR);
    let buffer = filled_buffer(fs, vec![(" ", attrs.clone())], metrics, None);
    let mut glyphs = buffer.layout_runs().flat_map(|run| run.glyphs.iter());
    glyphs
        .next()
        .is_some_and(|glyph| glyph.w.is_finite() && glyph.w > 0.0)
}

/// A face as [`untabbed`] tells faces apart: its family (`None` for the
/// default), weight and slant.
type Face<'a> = (Option<&'a str>, u16, bool);

/// Each span's text with its tabs drawn as spaces when its face cannot
/// measure a tab stop ([`space_measures`]), decided per span and before
/// shaping (R24.1): `None` keeps the span's own text, tabs and all. The
/// replacement is byte for byte, so a glyph's span bytes name the same
/// characters of the caller's text. Empty when no span holds a tab.
///
/// Cost: nothing for text without a tab; otherwise one space shaped per
/// distinct face, weight and slant among the spans that hold one, and
/// one copy of each such span's text whose face fails.
fn untabbed(
    spans: &[FlowSpan<'_>],
    families: &[Option<PinnedFamily>],
    scale: f32,
    fs: &mut FontSystem,
) -> Vec<Option<String>> {
    if !spans.iter().any(|span| span.text.contains('\t')) {
        return Vec::new();
    }
    let mut measured: Vec<(Face<'_>, bool)> = Vec::new();
    spans
        .iter()
        .enumerate()
        .map(|(index, span)| {
            if !span.text.contains('\t') {
                return None;
            }
            let family = families.get(index).and_then(Option::as_ref);
            let face = (
                family.map(|pin| pin.name.as_str()),
                span.style.weight.get(),
                span.style.italic,
            );
            let measures = match measured.iter().find(|(seen, _)| *seen == face) {
                Some((_, measures)) => *measures,
                None => {
                    let measures = space_measures(&span_attrs(span, scale, family), fs);
                    measured.push((face, measures));
                    measures
                }
            };
            (!measures).then(|| span.text.replace('\t', " "))
        })
        .collect()
}

/// `spans`, each span whose entry of `untabbed` is `Some` reading that
/// text instead of its own.
fn with_untabbed<'s>(spans: &[FlowSpan<'s>], untabbed: &'s [Option<String>]) -> Vec<FlowSpan<'s>> {
    spans
        .iter()
        .enumerate()
        .map(
            |(index, span)| match untabbed.get(index).and_then(Option::as_deref) {
                Some(text) => FlowSpan { text, ..*span },
                None => *span,
            },
        )
        .collect()
}

/// The buffer lines on which a laid-out width, or a glyph's width or
/// position, is not finite.
fn non_finite_lines(buffer: &Buffer) -> Vec<usize> {
    let mut lines: Vec<usize> = buffer
        .layout_runs()
        .filter(|run| {
            !run.line_w.is_finite()
                || run.glyphs.iter().any(|glyph| {
                    #[cfg(test)]
                    if hooked_tab(run.text, glyph) {
                        return true;
                    }
                    !glyph.w.is_finite() || !glyph.x.is_finite()
                })
        })
        .map(|run| run.line_i)
        .collect();
    lines.dedup();
    lines
}

/// The document's alignment as cosmic-text's.
fn cosmic_align(align: Align) -> crate::font::Align {
    match align {
        Align::Left => crate::font::Align::Left,
        Align::Center => crate::font::Align::Center,
        Align::Right => crate::font::Align::Right,
    }
}

/// The piece a glyph on `line` starting at line byte `at` came from.
/// `pieces` is in line order, so this is a binary search.
fn piece_of(pieces: &[Piece], line: usize, at: usize) -> Option<&Piece> {
    let index = pieces.partition_point(|p| (p.line, p.line_start) <= (line, at));
    let piece = pieces.get(index.checked_sub(1)?)?;
    (piece.line == line).then_some(piece)
}

/// `glyph`'s span, its cluster as bytes of that span's text, and whether
/// it is the space a run lays for a line break.
fn span_bytes(pieces: &[Piece], line: usize, glyph: &LayoutGlyph) -> (usize, Range<usize>, bool) {
    // Every glyph starts inside a piece, since cosmic-text shapes only
    // the text it was handed and no separator that would move a glyph to
    // a line of its own reaches it. If that ever stops being true, the
    // glyph names no span — never a real one, whose link a click would
    // follow (ruling R9.5).
    let Some(piece) = piece_of(pieces, line, glyph.start) else {
        debug_assert!(
            false,
            "flow: a glyph at line {line} byte {} has no piece",
            glyph.start
        );
        return (FILLER_SPAN, 0..0, false);
    };
    if piece.line_break > 0 {
        // The space laid for a line break maps to the break.
        return (
            piece.span,
            piece.span_start..piece.span_start + piece.line_break,
            true,
        );
    }
    let offset = glyph.start - piece.line_start;
    let end = glyph.end.min(piece.line_start + piece.len).max(glyph.start) - piece.line_start;
    (
        piece.span,
        piece.span_start + offset..piece.span_start + end,
        false,
    )
}

/// A laid-out line's glyphs in visual order, left to right: cosmic-text
/// lists a right-to-left line's glyphs right to left (its pen positions
/// fall along the list), so those are walked backward. No allocation.
fn visual(glyphs: &[LayoutGlyph], rtl: bool) -> impl Iterator<Item = &LayoutGlyph> + '_ {
    let last = glyphs.len().saturating_sub(1);
    (0..glyphs.len()).map(move |i| &glyphs[if rtl { last - i } else { i }])
}

/// The ascent and descent, per unit of size, assumed for a glyph whose
/// face the font system cannot hand back. Unreachable in practice —
/// cosmic-text lays out glyphs only in faces it loaded — but a frame
/// must not fail on it (CODE_CONVENTIONS §9); these are a typical Latin
/// face's proportions.
const UNKNOWN_FACE_EXTENT: (f32, f32) = (0.8, 0.2);

/// The per-glyph facts of a shaped buffer: span and bytes through the
/// piece table, advance, the face's ascent and descent at the glyph's
/// size, where each grapheme cluster begins, and where the text may be
/// parted between words ([`word_breaks`]).
///
/// Cost: one grapheme walk and one word-boundary walk per line (to learn
/// its cluster starts and word breaks), one binary search per glyph into
/// the piece table and one into the line's cluster starts, one more per
/// cluster into its word breaks, and one font lookup per change of face.
fn glyph_metrics(buffer: &Buffer, pieces: &[Piece], fs: &mut FontSystem) -> Vec<GlyphMetrics> {
    let mut out = Vec::new();
    // The last glyph whose face was looked up, and that face's ascent
    // and descent per unit of size: consecutive glyphs mostly share one.
    let mut face: Option<(&LayoutGlyph, f32, f32)> = None;
    let mut starts: Vec<usize> = Vec::new();
    let mut breaks: Vec<usize> = Vec::new();
    let mut current_line = usize::MAX;
    let mut previous_cluster: Option<(usize, usize)> = None;
    for run in buffer.layout_runs() {
        if run.line_i != current_line {
            current_line = run.line_i;
            starts.clear();
            starts.extend(grapheme_chad::grapheme_byte_starts(run.text));
            word_breaks(run.text, &mut breaks);
            previous_cluster = None;
        }
        for glyph in visual(run.glyphs, run.rtl) {
            let (span, bytes, line_break) = span_bytes(pieces, run.line_i, glyph);
            let (ascent, descent) = match face {
                Some((seen, ascent, descent))
                    if seen.font_id == glyph.font_id && seen.font_weight == glyph.font_weight =>
                {
                    (ascent, descent)
                }
                _ => {
                    let (ascent, descent) = fs
                        .get_font(glyph.font_id, glyph.font_weight)
                        .map(|font| {
                            let metrics = font.metrics();
                            let em = f32::from(metrics.units_per_em.max(1));
                            (metrics.ascent / em, -metrics.descent / em)
                        })
                        .unwrap_or(UNKNOWN_FACE_EXTENT);
                    face = Some((glyph, ascent, descent));
                    (ascent, descent)
                }
            };
            let cluster = starts.partition_point(|&s| s <= glyph.start);
            let key = (run.line_i, cluster);
            let grapheme_start = previous_cluster != Some(key);
            // This glyph's grapheme is `cluster - 1`. Between it and the
            // previous glyph's, the text parts where the later of the two
            // starts (a ligature before it may cover several graphemes).
            let word_break = grapheme_start
                && match previous_cluster {
                    Some((_, before)) if before < cluster => {
                        breaks.binary_search(&starts[cluster - 1]).is_ok()
                    }
                    Some((_, before)) => breaks.binary_search(&starts[before - 1]).is_ok(),
                    None => true,
                };
            previous_cluster = Some(key);
            let first = starts
                .get(cluster.wrapping_sub(1))
                .and_then(|&at| run.text.get(at..))
                .and_then(|rest| rest.chars().next());
            let blank = first.is_some_and(char::is_whitespace);
            let letter = first.is_some_and(char::is_alphanumeric);
            out.push(GlyphMetrics {
                span,
                bytes,
                advance: glyph.w,
                size: glyph.font_size,
                x: glyph.x,
                ascent: ascent * glyph.font_size,
                descent: descent * glyph.font_size,
                grapheme_start,
                word_break,
                blank,
                line_break,
                letter,
                ink: None,
            });
        }
    }
    if !out.iter().any(|glyph| glyph.letter) {
        measure_ink(buffer, &mut out, fs);
    }
    out
}

/// Every glyph's [`GlyphMetrics::ink`], into `glyphs` (which lists the
/// buffer's glyphs in [`ShapedRun::layout_glyphs`]'s order), but a
/// space's, which has none to keep apart.
///
/// Cost: one outline scale per glyph that is not a space, and one
/// scaler context for the run.
fn measure_ink(buffer: &Buffer, glyphs: &mut [GlyphMetrics], fs: &mut FontSystem) {
    let mut cache = SwashCache::new();
    let laid = buffer.layout_runs().flat_map(|run| visual(run.glyphs, run.rtl));
    for (metrics, laid) in glyphs.iter_mut().zip(laid) {
        if !metrics.blank {
            metrics.ink = glyph_outline_ink(fs, &mut cache, laid).map(|(min, max)| Ink { min, max });
        }
    }
}

/// The byte offsets in `text` where it may be parted without tearing a
/// word, into `out` (cleared), ascending: each Unicode word boundary
/// (UAX #29) with whitespace on either side, or a word — a segment
/// holding a letter or digit — on both. A boundary beside punctuation
/// is not one, so "Sand," and "well-known" stay whole.
///
/// Cost: one word-boundary walk of `text`; no allocation beyond `out`.
fn word_breaks(text: &str, out: &mut Vec<usize>) {
    use unicode_segmentation::UnicodeSegmentation;
    out.clear();
    let blank = |segment: &str| segment.chars().all(char::is_whitespace);
    let word = |segment: &str| segment.chars().any(char::is_alphanumeric);
    let mut before: Option<&str> = None;
    for (offset, segment) in text.split_word_bound_indices() {
        if let Some(before) = before {
            if blank(before) || blank(segment) || (word(before) && word(segment)) {
                out.push(offset);
            }
        }
        before = Some(segment);
    }
}

/// Shape `spans` as one line, every span's size times `scale`.
///
/// The input of the path and point flows. Each span is shaped with its
/// own face, size, weight, slant and color ([`span_attrs`]); a paragraph
/// separator in a span (`\n`, `\r\n`, `\r`, U+001C–U+001E, U+0085,
/// U+2029) breaks no line, since a run is one line: it is laid as a space
/// marked [`GlyphMetrics::line_break`], where a frame's two-line layout
/// parts its lines (ruling R27). `scale` is what `Fit` uses to
/// rescale a flow (#239 §6); a caller shaping at the declared size
/// passes `1.0`.
///
/// Spans' sizes must be in points ([`point_size`](crate::flow::point_size)).
///
/// **Precondition: call [`fonts::init`](crate::font::fonts::init) before
/// the lease first takes the lock.** Resolving a span's face reads the
/// compiled font table, whose first read registers the fonts under the
/// font lock; with the table still cold, that read would re-enter the
/// lock the lease holds and panic. The app initializes at startup; any
/// other caller (a tool, a fuzz target) does it once before its first
/// lease or guard.
///
/// Cost: one cosmic-text buffer and one shaping pass over the text,
/// under the lock `fs` holds (an acquire first when it holds none,
/// [`FontSystemLease::fonts`]); one family-name lookup and `String` per
/// span; one grapheme walk; the piece table and glyph list.
pub fn shape_run(spans: &[FlowSpan<'_>], scale: f32, fs: &mut FontSystemLease) -> ShapedRun {
    let fs = fs.fonts();
    let families = families_of(spans.iter().map(|span| span.style), fs);
    let untabbed = untabbed(spans, &families, scale, fs);
    let spans = with_untabbed(spans, &untabbed);
    let assembly = assemble(&spans, &families, scale, Breaks::None);
    let run = run_from_assembly(assembly, fs);
    #[cfg(test)]
    crate::flow::place::halves::SHAPED_STEPS
        .with(|count| count.set(count.get() + crate::flow::place::halves::shaping_steps(&run)));
    run
}

/// Build a run from assembled stretches: shape them on one buffer and
/// measure its glyphs.
fn run_from_assembly(assembly: Assembly<'_>, fs: &mut FontSystem) -> ShapedRun {
    let line_height = assembly.line_height;
    let buffer = new_buffer(fs, assembly.rich, assembly.font_size, line_height, None);
    let glyphs = glyph_metrics(&buffer, &assembly.pieces, fs);
    ShapedRun {
        buffer: Arc::new(buffer),
        glyphs,
        line_height,
        pieces: assembly.pieces,
    }
}

/// `run` shaped again with every size times `factor`, and, when
/// `filler` is given, its text appended on a line of its own in the
/// given style.
///
/// How placement rescales a run for `Fit` and gives it a `Once` filler
/// without the spans it was shaped from: the buffer still holds every
/// drawn stretch and its attributes, and the piece table says where
/// each came from, so the same stretches are handed to cosmic-text
/// again with scaled metrics. The text's glyphs come out in the same
/// order with the same spans and bytes.
///
/// The filler goes on a line of its own so that neither the text's
/// shaping (kerning, joining, bidi order) nor the filler's depends on
/// the other: the text's glyphs, and their indices, are those of `run`.
///
/// **A size-only reshape shapes nothing** (#304): with no filler, the
/// new buffer takes `run`'s shaping, its glyphs' sizes read from the
/// scaled attributes ([`resized_buffer`]), and is only laid out — which
/// is what shaping it again would make, since shaping is in units of the
/// em. A run whose text holds a tab, or that already carries a filler,
/// is shaped again, as before.
///
/// Cost: with a filler, one shaping pass over the run's text and the
/// filler; without one, one layout pass and no shaping. Either way the
/// piece table and glyph list, and attributes per drawn stretch.
pub(crate) fn reshape(
    run: &ShapedRun,
    factor: f32,
    filler: Option<(&str, &ResolvedText)>,
    fs: &mut FontSystem,
) -> ShapedRun {
    // The filler's text and family name outlive the attribute list that
    // borrows them.
    // The filler is one line, so no paragraph separator in it is drawn.
    let filler_text: String = filler.map_or_else(String::new, |(text, _)| {
        text.chars().filter(|&ch| !is_paragraph_separator(ch)).collect()
    });
    let filler_family = filler.and_then(|(_, style)| families_of(std::iter::once(style), fs).pop().flatten());
    // The filler is a span of its own: its tabs keep their stops unless
    // its face cannot measure one (R24.1).
    let filler_text = match filler {
        Some((text, style)) if filler_text.contains('\t') => {
            let span = FlowSpan {
                text,
                style,
                link: None,
            };
            if space_measures(&span_attrs(&span, factor, filler_family.as_ref()), fs) {
                filler_text
            } else {
                filler_text.replace('\t', " ")
            }
        }
        _ => filler_text,
    };

    let mut rich: Vec<(&str, Attrs<'_>)> = Vec::with_capacity(run.pieces.len() + 2);
    let mut pieces = Vec::with_capacity(run.pieces.len() + 1);
    let mut line_height = 0.0_f32;
    let mut font_size = 0.0_f32;
    let mut track = |attrs: &Attrs<'_>| {
        let metrics = metrics_of(attrs);
        line_height = line_height.max(metrics.line_height);
        font_size = font_size.max(metrics.font_size);
    };
    let mut line_len = 0usize;
    for piece in run.pieces.iter().filter(|piece| piece.span != FILLER_SPAN) {
        // Every piece is on a line of the buffer it was shaped into, since
        // no separator reaches cosmic-text to move it (ruling R9.5). A
        // piece that is not there would be text silently dropped, so a
        // debug build refuses it; a release build keeps the frame.
        let slice = run
            .buffer
            .lines
            .get(piece.line)
            .and_then(|line| line.text().get(piece.line_start..piece.line_start + piece.len));
        let (Some(line), Some(slice)) = (run.buffer.lines.get(piece.line), slice) else {
            debug_assert!(
                false,
                "flow: a piece of span {} is not in its shaped line",
                piece.span
            );
            continue;
        };
        let attrs = scaled(&line.attrs_list().get_span(piece.line_start), factor);
        track(&attrs);
        rich.push((slice, attrs));
        pieces.push(Piece {
            span: piece.span,
            span_start: piece.span_start,
            line: 0,
            line_start: line_len,
            len: slice.len(),
            line_break: piece.line_break,
        });
        line_len += slice.len();
    }
    if let Some((text, style)) = filler.filter(|_| !filler_text.is_empty()) {
        let span = FlowSpan {
            text,
            style,
            link: None,
        };
        let attrs = span_attrs(&span, factor, filler_family.as_ref());
        track(&attrs);
        rich.push(("\n", attrs.clone()));
        rich.push((filler_text.as_str(), attrs));
        pieces.push(Piece {
            span: FILLER_SPAN,
            span_start: 0,
            line: 1,
            line_start: 0,
            len: filler_text.len(),
            line_break: 0,
        });
    }
    let resized = (filler_text.is_empty() || filler.is_none())
        .then(|| resized_buffer(fs, &run.buffer, &rich, font_size, line_height))
        .flatten();
    let buffer = match resized {
        Some(buffer) => buffer,
        None => new_buffer(fs, rich, font_size, line_height, None),
    };
    let glyphs = glyph_metrics(&buffer, &pieces, fs);
    let run = ShapedRun {
        buffer: Arc::new(buffer),
        glyphs,
        line_height,
        pieces,
    };
    #[cfg(test)]
    crate::flow::place::halves::SHAPED_STEPS
        .with(|count| count.set(count.get() + crate::flow::place::halves::shaping_steps(&run)));
    run
}

/// `rich`, the stretches of `shaped`'s one line with their attributes
/// rescaled, laid out at their new sizes in a buffer that takes `shaped`'s
/// shaping instead of shaping them again
/// ([`BufferLine::adopt_shape`](cosmic_text::BufferLine::adopt_shape)):
/// glyph for glyph what [`new_buffer`] would lay out, since the
/// attributes differ from the ones `shaped` was shaped with only in their
/// metrics, and shaping is in units of the em.
///
/// `None`, to be shaped instead, when `shaped` is not one line (it holds
/// a filler), when the text holds a tab (whose last-resort check
/// [`new_buffer`] makes after shaping), or when the line will not take
/// the shaping. In test builds, also while [`RESHAPE_BY_SHAPING`] is set.
///
/// Cost: one layout pass; no shaping.
fn resized_buffer(
    fs: &mut FontSystem,
    shaped: &Buffer,
    rich: &[(&str, Attrs<'_>)],
    font_size: f32,
    line_height: f32,
) -> Option<Buffer> {
    #[cfg(test)]
    if RESHAPE_BY_SHAPING.with(std::cell::Cell::get) {
        return None;
    }
    let [from] = shaped.lines.as_slice() else {
        return None;
    };
    if rich.iter().any(|(text, _)| text.contains('\t')) {
        return None;
    }
    let font_size = font_size.max(MIN_FONT_SIZE_PT);
    let line_height = line_height.max(MIN_FONT_SIZE_PT * LINE_HEIGHT_FACTOR);
    let metrics = Metrics::new(font_size, line_height);
    let mut buffer = Buffer::new_empty(metrics);
    buffer.set_wrap(Wrap::None);
    buffer.set_size(None, None);
    let defaults = Attrs::new().metrics(metrics);
    buffer.set_rich_text(rich.iter().cloned(), &defaults, SHAPING_ADVANCED, None);
    let [line] = buffer.lines.as_mut_slice() else {
        return None;
    };
    if !line.adopt_shape(from) {
        return None;
    }
    buffer.shape_until_scroll(fs, false);
    Some(buffer)
}

/// `attrs` with its size and line height times `factor`, clamped to
/// the shaper's domain.
fn scaled<'a>(attrs: &Attrs<'a>, factor: f32) -> Attrs<'a> {
    let size = clamp_font_metric(metrics_of(attrs).font_size * factor, MIN_FONT_SIZE_PT);
    attrs
        .clone()
        .metrics(Metrics::new(size, size * LINE_HEIGHT_FACTOR))
}

/// `min` with every axis held to a finite, non-negative length: glam's
/// `max` passes a `NaN` through, and a box must not be `NaN` wide.
fn usable_min_size(min: Vec2) -> Vec2 {
    let axis = |v: f32| if v.is_finite() { v.max(0.0) } else { 0.0 };
    Vec2::new(axis(min.x), axis(min.y))
}

/// Shape `spans` as a box: lines broken at `\n` only (D26), the box
/// sized to the larger of `min_size` and the text's natural size, and
/// every line aligned within the box's width by `align`.
///
/// The natural size is the widest line by the sum of the lines' heights
/// (each line as tall as its tallest span's line height, an empty line
/// as tall as the span it sits in, and empty text one empty line). A
/// `\n` with nothing after it ends the box with an empty line, as an
/// editor shows it; no spans at all is an empty box. Each span keeps its
/// own face, size, weight, slant and color ([`span_attrs`]); no other
/// paragraph separator (`\r`, U+001C–U+001E, U+0085, U+2029) is drawn
/// or breaks a line. A minimum size is read per axis as a finite,
/// non-negative length: a `NaN`, infinite or negative axis asks for
/// nothing.
///
/// Spans' sizes must be in points ([`point_size`](crate::flow::point_size)),
/// and [`fonts::init`](crate::font::fonts::init) must have run before the
/// lease first took the lock, as for [`shape_run`].
///
/// Cost: one cosmic-text buffer, shaped once and laid out twice (once
/// unbounded to measure, once at the box's width to align — the second
/// pass re-lays-out without reshaping), under the lock `fs` holds (an
/// acquire first when it holds none); one family-name lookup and `String` per span; the piece
/// table and the glyph-span list.
pub fn shape_box(
    spans: &[FlowSpan<'_>],
    min_size: Option<Vec2>,
    align: Align,
    fs: &mut FontSystemLease,
) -> ShapedBox {
    let fs = fs.fonts();
    let families = families_of(spans.iter().map(|span| span.style), fs);
    let untabbed = untabbed(spans, &families, 1.0, fs);
    let spans = &with_untabbed(spans, &untabbed);
    let assembly = assemble(spans, &families, 1.0, Breaks::Newline);
    // A line with nothing on it — the whole box, when the text is empty
    // — is as tall as the spans' line height.
    let empty_line = assembly.span_metrics;
    let pieces = assembly.pieces;
    let mut buffer = new_buffer(
        fs,
        assembly.rich,
        empty_line.font_size,
        empty_line.line_height,
        Some(align),
    );

    let mut natural = Vec2::ZERO;
    if !spans.is_empty() {
        for run in buffer.layout_runs() {
            natural.x = natural.x.max(run.line_w);
            natural.y += run.line_height;
        }
    }
    let size = min_size.map_or(natural, |min| natural.max(usable_min_size(min)));
    buffer.set_size(Some(size.x), None);
    buffer.shape_until_scroll(fs, false);

    let glyph_spans = buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter().map(move |glyph| (run.line_i, glyph)))
        .map(|(line, glyph)| {
            let (span, bytes, _) = span_bytes(&pieces, line, glyph);
            (span, bytes)
        })
        .collect();
    ShapedBox {
        buffer: Arc::new(buffer),
        size,
        glyph_spans,
    }
}
