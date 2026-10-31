// SPDX-License-Identifier: MPL-2.0

//! TextFlow's fuzz properties (design D29; TEST_CONVENTIONS §T14):
//! shaping and placing arbitrary strings, which a map hands the app in
//! every span. The flow_raw fuzz target shapes its bytes, read as
//! text, in every layout; flow_structured reads its bytes as choices
//! ([`Choices`](crate::util::tests::choices::Choices)) and builds spans
//! of hostile text in every face, size, weight and slant, then shapes
//! and places them as a box, around an outline, along a curve or at a
//! point, with every fit mode.
//!
//! What a flow must satisfy, whatever its text and its geometry:
//!
//! - **No panic**, under the font lock the caller holds.
//! - **Every glyph names where it came from:** its span is one of the
//!   flow's or [`FILLER_SPAN`](crate::flow::FILLER_SPAN), and its bytes are a range of that span's
//!   text that starts and ends on a character boundary — what a click on
//!   a linked glyph is resolved through.
//! - **The metrics are numbers:** every advance, size, pen position,
//!   ascent and descent is finite, and so is the line height.
//! - **Placement is finite and bounded** (#239, ruling R6): every placed
//!   position is finite whatever the path, every placed glyph names a
//!   glyph of the run it refers to, and no placement holds more than
//!   [`MAX_PLACED_GLYPHS`](crate::flow::MAX_PLACED_GLYPHS).
//! - **A box is a box:** its size is finite, non-negative, at least the
//!   usable part of the minimum asked for, and one glyph-span entry per
//!   laid-out glyph.
//!
//! The geometry is hostile on purpose (any `f32`, `NaN` and the
//! infinities included): placement's promise is "whatever the path".
//! What the document bounds is kept in its bounds — a `Fit` range
//! passes [`check_fit`](crate::document::bounds::check_fit), a start
//! point's `at` is in `0..=1`, sizes are in points — because placement
//! receives those from a loaded document, never raw.

use glam::Vec2;

use crate::document::bounds;
use crate::document::resolve::ResolvedText;
use crate::document::values::{
    Align, Direction, Edge, FitMode, FontSize, Percent, Rgba, Shape, StartPoint, Weight,
};
use crate::flow::{
    offset, outline, place_at_point, place_on_closed, place_on_open, shape_box, shape_run, Curve, FlowSpan,
    PlacedRun, Rect, ShapedBox, ShapedRun, FILLER_SPAN, MAX_PLACED_GLYPHS,
};
use crate::font::fonts::{self, acquire_font_system_write, AppFont, COMPILED_FONT_ID_MAP};
use crate::util::tests::choices::Choices;

/// The most characters a structured span holds: enough for a cut, a
/// repeat and a fit to have something to do, few enough that one input
/// shapes in milliseconds.
pub const MAX_SPAN_CHARS: u32 = 64;

/// Characters a structured span draws from besides any code point:
/// combining marks, ZWJ sequences, variation selectors, regional
/// indicators, right-to-left letters, bidi controls, every paragraph
/// separator, controls and box-drawing glyphs.
const INTERESTING: [char; 28] = [
    'a',
    ' ',
    '\u{301}',
    '\u{302}',
    '\u{200D}',
    '\u{FE0F}',
    '\u{1F469}',
    '\u{1F4BB}',
    '\u{1F1EF}',
    '\u{1F1F5}',
    '\u{5D0}',
    '\u{627}',
    '\u{644}',
    '\u{202E}',
    '\u{2066}',
    '\n',
    '\r',
    '\u{1C}',
    '\u{85}',
    '\u{2028}',
    '\u{2029}',
    '\t',
    '\u{0}',
    '\u{7F}',
    '─',
    '═',
    '\u{E000}',
    '\u{FFFD}',
];

/// Every property of `bytes` read as text (lossily: a raw input need
/// not be UTF-8), shaped as one span in each layout: the flow_raw
/// target's whole body.
pub fn check_flow_bytes(bytes: &[u8]) -> Result<(), String> {
    let text = String::from_utf8_lossy(bytes);
    let style = plain_style();
    let spans = [FlowSpan {
        text: &text,
        style: &style,
        link: None,
    }];
    fonts::init();
    let mut fs = acquire_font_system_write("flow fuzz");
    let shaped = shape_box(&spans, None, Align::Left, &mut fs);
    check_box(&spans, &shaped, None)?;
    let run = shape_run(&spans, 1.0, &mut fs);
    check_run(&spans, &run, "the run")?;
    let rect = Rect::new(Vec2::ZERO, Vec2::new(120.0, 40.0));
    let start = StartPoint {
        edge: Edge::Top,
        at: 0.0,
    };
    for fit in [
        FitMode::Repeat,
        FitMode::Spread,
        FitMode::Fit { min: 0.5, max: 2.0 },
        FitMode::Once {
            filler: Some("·".to_string()),
        },
    ] {
        let path = outline(
            rect,
            Shape::Rectangle {
                radius: Percent::new(50),
            },
        );
        let placed = place_on_closed(
            &run,
            &path,
            start,
            Direction::Clockwise,
            &fit,
            true,
            &style,
            &mut fs,
        );
        check_placed(&spans, &run, placed, "around a rectangle")?;
        let curve = Curve::Line(Vec2::ZERO, Vec2::new(200.0, 0.0));
        let placed = place_on_open(&run, &curve, &fit, 1.0, &style, &mut fs);
        check_placed(&spans, &run, placed, "along a line")?;
    }
    check_placement(&run, &place_at_point(&run, Vec2::ZERO), "at a point")
}

/// Every property of the flow `bytes` choose: the flow_structured
/// target's whole body.
pub fn check_flow_structured(bytes: &[u8]) -> Result<(), String> {
    check_flow_generated(&mut Choices::new(bytes))
}

/// [`check_flow_structured`] for choices from any source.
pub fn check_flow_generated(choices: &mut Choices<'_>) -> Result<(), String> {
    fonts::init();
    let faces = faces();
    let count = choices.below(4) as usize;
    let texts: Vec<String> = (0..count).map(|_| text(choices)).collect();
    let styles: Vec<ResolvedText> = (0..count).map(|_| style(choices, &faces)).collect();
    let spans: Vec<FlowSpan<'_>> = texts
        .iter()
        .zip(&styles)
        .map(|(text, style)| FlowSpan {
            text,
            style,
            link: None,
        })
        .collect();
    let flow = style(choices, &faces);
    let mut fs = acquire_font_system_write("flow fuzz");
    match choices.below(4) {
        0 => {
            let min = choices.one_in(2).then(|| any_vec2(choices));
            let align = *choices.pick(&[Align::Left, Align::Center, Align::Right]);
            let shaped = shape_box(&spans, min, align, &mut fs);
            check_box(&spans, &shaped, min)?;
            // `glyph_spans` lists the glyphs in the buffer's layout order.
            let glyphs = shaped
                .buffer
                .layout_runs()
                .flat_map(|run| {
                    run.glyphs
                        .iter()
                        .map(|glyph| (glyph.font_id, cluster(run.text, glyph)))
                        .collect::<Vec<_>>()
                })
                .zip(shaped.glyph_spans.iter())
                .map(|((face, text), (span, _))| (face, *span, text))
                .collect::<Vec<_>>();
            shapes_in_pinned_faces(&spans, glyphs, &mut fs).map_err(|why| format!("in a box: {why}"))
        }
        layout => {
            let scale = any_f32(choices);
            let run = shape_run(&spans, scale, &mut fs);
            check_run(&spans, &run, "the run")
                .map_err(|why| format!("{why}, at scale {scale}, of {spans:?}"))?;
            // `glyphs` lists them in visual order: a right-to-left line's
            // backward (`ShapedRun::layout_glyphs`).
            let glyphs = run
                .buffer
                .layout_runs()
                .flat_map(|line| {
                    let mut laid: Vec<_> = line
                        .glyphs
                        .iter()
                        .map(|glyph| (glyph.font_id, cluster(line.text, glyph)))
                        .collect();
                    if line.rtl {
                        laid.reverse();
                    }
                    laid
                })
                .zip(&run.glyphs)
                .map(|((face, text), metrics)| (face, metrics.span, text))
                .collect::<Vec<_>>();
            shapes_in_pinned_faces(&spans, glyphs, &mut fs).map_err(|why| format!("in the run: {why}"))?;
            match layout {
                1 => {
                    let rect = Rect::new(any_vec2(choices), any_vec2(choices));
                    let shape = if choices.one_in(2) {
                        Shape::Ellipse
                    } else {
                        Shape::Rectangle {
                            radius: Percent::new(choices.below(101) as u8),
                        }
                    };
                    let mut path = outline(rect, shape);
                    if choices.one_in(2) {
                        path = offset(&path, any_f32(choices));
                    }
                    let start = StartPoint {
                        edge: *choices.pick(&[Edge::Top, Edge::Right, Edge::Bottom, Edge::Left]),
                        at: choices.below(1001) as f32 / 1000.0,
                    };
                    let direction = *choices.pick(&[Direction::Clockwise, Direction::CounterClockwise]);
                    let fit = fit(choices, &texts);
                    let outward = choices.one_in(2);
                    let placed =
                        place_on_closed(&run, &path, start, direction, &fit, outward, &flow, &mut fs);
                    check_placed(&spans, &run, placed, "around an outline")
                }
                2 => {
                    let points: Vec<Vec2> = (0..choices.below(3)).map(|_| any_vec2(choices)).collect();
                    let Some(curve) = Curve::from_points(any_vec2(choices), &points, any_vec2(choices))
                    else {
                        return Err("two control points or fewer always make a curve".to_string());
                    };
                    let fit = fit(choices, &texts);
                    let spacing = any_f32(choices);
                    let placed = place_on_open(&run, &curve, &fit, spacing, &flow, &mut fs);
                    check_placed(&spans, &run, placed, "along a curve")
                }
                _ => check_placement(&run, &place_at_point(&run, any_vec2(choices)), "at a point"),
            }
        }
    }
}

/// The plain style the raw target shapes in.
fn plain_style() -> ResolvedText {
    ResolvedText {
        font: AppFont::Any,
        size: FontSize::Pt(16.0),
        color: Rgba::new(255, 255, 255, 255),
        weight: Weight::NORMAL,
        italic: false,
    }
}

/// Every face a span can name: each compiled-in `AppFont`, in name
/// order, and `Any`. Read from the compiled set itself rather than from
/// the family index, which attributes a family to `Any` when a host face
/// bearing its name comes first (the shadowing R30.1 removes): a sweep
/// that drew its faces from the index would stop pinning that family at
/// all, and so could not see a host face win its pin.
fn faces() -> Vec<AppFont> {
    let mut faces: Vec<AppFont> = COMPILED_FONT_ID_MAP
        .iter()
        .filter(|(_, ids)| !ids.is_empty())
        .map(|(font, _)| *font)
        .collect();
    faces.sort_by_key(|font| format!("{font:?}"));
    faces.push(AppFont::Any);
    faces
}

/// A resolved style: any face, any size in points (the shaper clamps
/// what is out of its domain, `NaN` included), any weight and slant.
fn style(choices: &mut Choices<'_>, faces: &[AppFont]) -> ResolvedText {
    let size = match choices.below(3) {
        0 => 16.0,
        1 => choices.below(200) as f32,
        _ => any_f32(choices),
    };
    ResolvedText {
        font: *choices.pick(faces),
        size: FontSize::Pt(size),
        color: Rgba::new(255, 255, 255, choices.below(256) as u8),
        weight: Weight::new(100 * (1 + choices.below(9) as u16)).unwrap_or(Weight::NORMAL),
        italic: choices.one_in(2),
    }
}

/// A text of up to [`MAX_SPAN_CHARS`] characters, each an interesting
/// one or any code point.
fn text(choices: &mut Choices<'_>) -> String {
    (0..choices.below(MAX_SPAN_CHARS))
        .map(|_| {
            // Any code point, rarely: one no face holds sends shaping
            // through every face in search of a fallback, which is slow.
            if choices.one_in(8) {
                char::from_u32(choices.below(0x11_0000)).unwrap_or('\u{FFFD}')
            } else {
                *choices.pick(&INTERESTING)
            }
        })
        .collect()
}

/// A number: plain, or any `f32` at all.
fn any_f32(choices: &mut Choices<'_>) -> f32 {
    match choices.below(4) {
        0 => 1.0,
        1 => choices.below(2000) as f32 - 1000.0,
        2 => *choices.pick(&[
            0.0,
            -0.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1e30,
            -1e30,
            f32::MIN_POSITIVE,
        ]),
        _ => choices.any_f32(),
    }
}

fn any_vec2(choices: &mut Choices<'_>) -> Vec2 {
    Vec2::new(any_f32(choices), any_f32(choices))
}

/// A fit mode a document can hold: a `Fit` range that passes
/// `check_fit` (the built-in one otherwise), and a filler from the
/// flow's own texts.
fn fit(choices: &mut Choices<'_>, texts: &[String]) -> FitMode {
    match choices.below(4) {
        0 => FitMode::Repeat,
        1 => FitMode::Spread,
        2 => {
            let (min, max) = (any_f32(choices), any_f32(choices));
            match bounds::check_fit(min, max) {
                Ok((min, max)) => FitMode::Fit { min, max },
                Err(_) => FitMode::Fit { min: 0.5, max: 2.0 },
            }
        }
        _ => FitMode::Once {
            filler: (!texts.is_empty() && choices.one_in(2)).then(|| choices.pick(texts).clone()),
        },
    }
}

/// `bytes` names whole characters of `text`.
fn on_boundaries(text: &str, bytes: &std::ops::Range<usize>) -> bool {
    bytes.start <= bytes.end
        && bytes.end <= text.len()
        && text.is_char_boundary(bytes.start)
        && text.is_char_boundary(bytes.end)
}

/// A glyph's span and bytes name a span of `spans` and whole characters
/// of its text, or the filler.
fn names_its_source(
    spans: &[FlowSpan<'_>],
    span: usize,
    bytes: &std::ops::Range<usize>,
) -> Result<(), String> {
    if span == FILLER_SPAN {
        return Ok(());
    }
    let Some(source) = spans.get(span) else {
        return Err(format!("a glyph names span {span} of {}", spans.len()));
    };
    if !on_boundaries(source.text, bytes) {
        return Err(format!(
            "a glyph names bytes {bytes:?} of span {span}, {:?}, which are not whole characters of it",
            source.text
        ));
    }
    Ok(())
}

/// The properties of a shaped run: every glyph names its source, every
/// metric is a number, and the glyphs are the buffer's.
pub fn check_run(spans: &[FlowSpan<'_>], run: &ShapedRun, what: &str) -> Result<(), String> {
    if !run.line_height.is_finite() || run.line_height < 0.0 {
        return Err(format!("{what}: line height {}", run.line_height));
    }
    let laid_out = run.layout_glyphs().count();
    if laid_out != run.glyphs.len() {
        return Err(format!(
            "{what}: {} metrics for {laid_out} laid-out glyphs",
            run.glyphs.len()
        ));
    }
    for glyph in &run.glyphs {
        names_its_source(spans, glyph.span, &glyph.bytes).map_err(|why| format!("{what}: {why}"))?;
        for (name, value) in [
            ("advance", glyph.advance),
            ("size", glyph.size),
            ("x", glyph.x),
            ("ascent", glyph.ascent),
            ("descent", glyph.descent),
        ] {
            if !value.is_finite() {
                return Err(format!("{what}: a glyph's {name} is {value}"));
            }
        }
    }
    if !run.baseline().is_finite() {
        return Err(format!("{what}: the baseline is {}", run.baseline()));
    }
    Ok(())
}

/// The properties of a placement and the run it refers to.
fn check_placed(
    spans: &[FlowSpan<'_>],
    run: &ShapedRun,
    (replaced, placed): (Option<ShapedRun>, PlacedRun),
    what: &str,
) -> Result<(), String> {
    let refers_to = match &replaced {
        Some(new) => {
            check_run(spans, new, &format!("{what}: the run placement made"))?;
            new
        }
        None => run,
    };
    check_placement(refers_to, &placed, what)
}

/// Every placed glyph is finite, names a glyph of `run`, and there are
/// at most [`MAX_PLACED_GLYPHS`].
pub fn check_placement(run: &ShapedRun, placed: &PlacedRun, what: &str) -> Result<(), String> {
    if placed.glyphs.len() > MAX_PLACED_GLYPHS {
        return Err(format!("{what}: {} glyphs placed", placed.glyphs.len()));
    }
    for glyph in &placed.glyphs {
        if !glyph.pos.is_finite() {
            return Err(format!("{what}: a glyph placed at {}", glyph.pos));
        }
        if glyph.glyph as usize >= run.glyphs.len() {
            return Err(format!(
                "{what}: placed glyph {} of {}",
                glyph.glyph,
                run.glyphs.len()
            ));
        }
    }
    Ok(())
}

/// The properties of a shaped box: a finite, non-negative size at least
/// the usable minimum, and one source per laid-out glyph.
pub fn check_box(spans: &[FlowSpan<'_>], shaped: &ShapedBox, min: Option<Vec2>) -> Result<(), String> {
    let size = shaped.size;
    if !size.is_finite() || size.x < 0.0 || size.y < 0.0 {
        return Err(format!("a box of size {size}"));
    }
    if let Some(min) = min {
        for (axis, asked, got) in [("x", min.x, size.x), ("y", min.y, size.y)] {
            if asked.is_finite() && asked > got {
                return Err(format!(
                    "a box {got} on {axis}, below the minimum {asked} asked for"
                ));
            }
        }
    }
    let laid_out: usize = shaped.buffer.layout_runs().map(|run| run.glyphs.len()).sum();
    if laid_out != shaped.glyph_spans.len() {
        return Err(format!(
            "{} glyph sources for {laid_out} laid-out glyphs",
            shaped.glyph_spans.len()
        ));
    }
    for (span, bytes) in &shaped.glyph_spans {
        names_its_source(spans, *span, bytes).map_err(|why| format!("a box: {why}"))?;
    }
    Ok(())
}

/// The cluster `glyph` was shaped from, as text of its line.
fn cluster(line: &str, glyph: &crate::font::LayoutGlyph) -> String {
    line.get(glyph.start..glyph.end).unwrap_or_default().to_string()
}

/// Ruling R30's property: every glyph of a span pinned to a compiled
/// family is drawn in one of that family's compiled faces, unless its
/// cluster holds a character none of them has (which cosmic-text draws
/// in a fallback face, as it must). `glyphs` are the laid-out glyphs,
/// each with its face, the span its cluster starts in, and the cluster
/// as cosmic-text shaped it (the bytes of its line, which may run on
/// into the next span); a filler glyph ([`FILLER_SPAN`]) is not a
/// span's and is not checked. `Err` names the first glyph drawn in
/// another face.
///
/// Without the weight resolved within the family
/// ([`PinnedFamily::weight_for`](crate::font::fonts::PinnedFamily::weight_for)),
/// a span at a weight its family lacks is drawn in any host face of that
/// exact weight; the test build registers such faces on every machine
/// (`fonts::register_host_decoys`), so this property catches it there.
pub fn shapes_in_pinned_faces(
    spans: &[FlowSpan<'_>],
    glyphs: impl IntoIterator<Item = (cosmic_text::fontdb::ID, usize, String)>,
    fs: &mut crate::font::FontSystem,
) -> Result<(), String> {
    let pins: Vec<Option<crate::font::fonts::PinnedFamily>> = spans
        .iter()
        .map(|span| match span.style.font {
            AppFont::Any => None,
            font => crate::font::fonts::pinned_family(fs, font),
        })
        .collect();
    for (face, span, text) in glyphs {
        let Some(Some(pin)) = pins.get(span) else {
            continue;
        };
        if pin.faces.contains(&face) {
            continue;
        }
        let held = |ch: char| {
            // cosmic-text shapes a tab as a space.
            let ch = if ch == '\t' { ' ' } else { ch };
            pin.faces.iter().any(|&id| {
                fs.get_font(id, cosmic_text::fontdb::Weight(400))
                    .is_some_and(|font| font.as_swash().charmap().map(ch) != 0)
            })
        };
        if !text.is_empty() && text.chars().all(held) {
            let family = fs.db().face(face).map(|info| info.post_script_name.clone());
            return Err(format!(
                "span {span} ({:?}) is pinned to {:?} (weights {:?}), and its cluster {text:?}, which the family holds, was drawn in {family:?}",
                spans[span],
                pin.name, pin.weights
            ));
        }
    }
    Ok(())
}
