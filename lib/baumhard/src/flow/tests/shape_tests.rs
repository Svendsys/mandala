// SPDX-License-Identifier: MPL-2.0

//! Shaping: span attributes reach cosmic-text, sizes are per span, a
//! run is one line, a box breaks at `\n` only and aligns its lines, and
//! every glyph maps back to its span and bytes.

use glam::Vec2;

use crate::document::values::{Align, Weight};
use crate::flow::shape::reshape;
use crate::flow::tests::support::{close, real_run, style, with_fonts};
use crate::flow::{shape_box, shape_run, span_attrs, FlowSpan, ShapedBox, FILLER_SPAN, LINE_HEIGHT_FACTOR};
use crate::font::{FontWeight, Metrics, Style};

/// `text` shaped as a box in one 20 pt span.
fn boxed(text: &str, min_size: Option<Vec2>, align: Align) -> ShapedBox {
    let style = style(20.0);
    with_fonts(|fs| {
        shape_box(
            &[FlowSpan {
                text,
                style: &style,
                link: None,
            }],
            min_size,
            align,
            fs,
        )
    })
}

#[test]
fn test_flow_span_attrs_carry_weight_slant_and_size() {
    do_flow_span_attrs_carry_weight_slant_and_size();
}

/// The attributes built for a span carry its weight, its slant and its
/// size times the scale (as metrics, so a span's size is its own); a
/// plain span is the control that the values come from the span.
pub fn do_flow_span_attrs_carry_weight_slant_and_size() {
    let mut bold_italic = style(30.0);
    bold_italic.weight = Weight::BOLD;
    bold_italic.italic = true;
    let plain = style(12.0);
    let span = |style| FlowSpan {
        text: "x",
        style,
        link: None,
    };

    let attrs = span_attrs(&span(&bold_italic), 2.0, Some("Galatia SIL"));
    assert_eq!(attrs.weight, FontWeight(700));
    assert_eq!(attrs.style, Style::Italic);
    assert_eq!(
        attrs.metrics_opt.map(Metrics::from),
        Some(Metrics::new(60.0, 60.0 * LINE_HEIGHT_FACTOR))
    );
    assert_eq!(
        attrs.color_opt,
        Some(crate::font::Color::rgba(255, 255, 255, 255))
    );

    let attrs = span_attrs(&span(&plain), 1.0, None);
    assert_eq!(attrs.weight, FontWeight(400));
    assert_eq!(attrs.style, Style::Normal);
    assert_eq!(
        attrs.metrics_opt.map(Metrics::from),
        Some(Metrics::new(12.0, 12.0 * LINE_HEIGHT_FACTOR))
    );
}

#[test]
fn test_flow_shape_run_keeps_each_span_attributes() {
    do_flow_shape_run_keeps_each_span_attributes();
}

/// Shaping hands each span's own attributes to cosmic-text: reading the
/// shaped line's attribute list at each span's first byte gives that
/// span's weight, slant and metrics — the bold, italic and size that
/// used to be dropped. The run's line height is the larger span's.
pub fn do_flow_shape_run_keeps_each_span_attributes() {
    let plain = style(12.0);
    let mut loud = style(30.0);
    loud.weight = Weight::BOLD;
    loud.italic = true;
    let run = with_fonts(|fs| {
        shape_run(
            &[
                FlowSpan {
                    text: "ab",
                    style: &plain,
                    link: None,
                },
                FlowSpan {
                    text: "cd",
                    style: &loud,
                    link: None,
                },
            ],
            1.0,
            fs,
        )
    });
    let list = run.buffer.lines[0].attrs_list();
    let first = list.get_span(0);
    assert_eq!(first.weight, FontWeight(400));
    assert_eq!(first.style, Style::Normal);
    assert_eq!(
        first.metrics_opt.map(Metrics::from),
        Some(Metrics::new(12.0, 12.0 * LINE_HEIGHT_FACTOR))
    );
    let second = list.get_span(2);
    assert_eq!(second.weight, FontWeight(700));
    assert_eq!(second.style, Style::Italic);
    assert_eq!(
        second.metrics_opt.map(Metrics::from),
        Some(Metrics::new(30.0, 30.0 * LINE_HEIGHT_FACTOR))
    );
    close(
        run.line_height,
        30.0 * LINE_HEIGHT_FACTOR,
        1e-4,
        "run line height",
    );
}

#[test]
fn test_flow_shape_run_sizes_each_span() {
    do_flow_shape_run_sizes_each_span();
}

/// A span's size is real in shaping: the same letter in a 30 pt span
/// beside a 12 pt one advances 30/12 times as far. The expectation
/// comes from the face's linear scaling, not from a second shaping.
pub fn do_flow_shape_run_sizes_each_span() {
    let small = style(12.0);
    let large = style(30.0);
    let run = with_fonts(|fs| {
        shape_run(
            &[
                FlowSpan {
                    text: "m",
                    style: &small,
                    link: None,
                },
                FlowSpan {
                    text: "m",
                    style: &large,
                    link: None,
                },
            ],
            1.0,
            fs,
        )
    });
    assert_eq!(run.glyphs.len(), 2);
    assert!(run.glyphs[0].advance > 0.0);
    close(
        run.glyphs[1].advance / run.glyphs[0].advance,
        2.5,
        1e-3,
        "advance ratio",
    );
    close(
        run.glyphs[1].ascent / run.glyphs[0].ascent,
        2.5,
        1e-3,
        "ascent ratio",
    );
}

#[test]
fn test_flow_shape_run_maps_glyphs_to_spans() {
    do_flow_shape_run_maps_glyphs_to_spans();
}

/// Every glyph names its span and its bytes of that span's text, even
/// though the line it was shaped on concatenates the spans and leaves
/// out the line breaks a run does not draw.
pub fn do_flow_shape_run_maps_glyphs_to_spans() {
    let text = style(14.0);
    let run = with_fonts(|fs| {
        shape_run(
            &[
                FlowSpan {
                    text: "ab",
                    style: &text,
                    link: None,
                },
                FlowSpan {
                    text: "\ncd\r",
                    style: &text,
                    link: None,
                },
            ],
            1.0,
            fs,
        )
    });
    let mapped: Vec<_> = run.glyphs.iter().map(|g| (g.span, g.bytes.clone())).collect();
    assert_eq!(mapped, vec![(0, 0..1), (0, 1..2), (1, 1..2), (1, 2..3)]);
    assert_eq!(run.buffer.lines.len(), 1, "a run is one line");
    assert_eq!(run.buffer.lines[0].text(), "abcd");
    assert!(run.glyphs.iter().all(|g| g.grapheme_start));
}

#[test]
fn test_flow_shape_run_marks_grapheme_starts() {
    do_flow_shape_run_marks_grapheme_starts();
}

/// A combining mark the face has no precomposed form for is shaped as a
/// glyph of its own, in the same cluster as its base: that glyph does
/// not begin a grapheme, and the next letter does. Both glyphs of the
/// cluster report the cluster's bytes.
pub fn do_flow_shape_run_marks_grapheme_starts() {
    let run = with_fonts(|fs| real_run("q\u{0301}z", &style(20.0), fs));
    assert_eq!(
        run.glyphs.len(),
        3,
        "the fixture needs the mark shaped apart from its base: {:?}",
        run.glyphs
    );
    let starts: Vec<bool> = run.glyphs.iter().map(|g| g.grapheme_start).collect();
    assert_eq!(starts, vec![true, false, true]);
    assert_eq!(run.glyphs[0].bytes, 0..3);
    assert_eq!(run.glyphs[1].bytes, 0..3);
    assert_eq!(run.glyphs[2].bytes, 3..4);
}

#[test]
fn test_flow_shape_run_of_nothing() {
    do_flow_shape_run_of_nothing();
}

/// No text, or only line breaks, is a run with no glyphs and no height.
pub fn do_flow_shape_run_of_nothing() {
    for text in ["", "\n\r\n"] {
        let run = with_fonts(|fs| real_run(text, &style(12.0), fs));
        assert!(run.glyphs.is_empty(), "{text:?}");
        assert_eq!(run.line_height, 0.0, "{text:?}");
        assert_eq!(run.baseline(), 0.0, "{text:?}");
    }
}

#[test]
fn test_flow_layout_glyphs_follow_glyphs() {
    do_flow_layout_glyphs_follow_glyphs();
}

/// The renderer's walk over the buffer lists the glyphs in the order
/// `glyphs` describes them: same count, same advances, same clusters.
pub fn do_flow_layout_glyphs_follow_glyphs() {
    let run = with_fonts(|fs| real_run("Wave", &style(16.0), fs));
    let layout: Vec<_> = run.layout_glyphs().collect();
    assert_eq!(layout.len(), 4);
    assert_eq!(layout.len(), run.glyphs.len());
    for (laid, metrics) in layout.iter().zip(&run.glyphs) {
        assert_eq!(laid.w, metrics.advance);
        assert_eq!(laid.start..laid.end, metrics.bytes);
    }
}

#[test]
fn test_flow_baseline_centers_the_tallest_glyph() {
    do_flow_baseline_centers_the_tallest_glyph();
}

/// The baseline sits so the run's tallest ascent and deepest descent
/// are centered in the line height: with a 10 pt and a 20 pt span, the
/// slack above the 20 pt ascent equals the slack below its descent.
pub fn do_flow_baseline_centers_the_tallest_glyph() {
    let small = style(10.0);
    let large = style(20.0);
    let run = with_fonts(|fs| {
        shape_run(
            &[
                FlowSpan {
                    text: "a",
                    style: &small,
                    link: None,
                },
                FlowSpan {
                    text: "a",
                    style: &large,
                    link: None,
                },
            ],
            1.0,
            fs,
        )
    });
    let tall = &run.glyphs[1];
    let above = run.baseline() - tall.ascent;
    let below = run.line_height - run.baseline() - tall.descent;
    close(above, below, 1e-3, "slack above vs below");
    assert!(above > 0.0, "a 1.2 line height leaves slack");
}

#[test]
fn test_flow_reshape_rescales_and_keeps_the_mapping() {
    do_flow_reshape_rescales_and_keeps_the_mapping();
}

/// Reshaping a run at twice its size — what `Fit` does — keeps every
/// glyph's span and bytes and doubles every advance and the line
/// height.
pub fn do_flow_reshape_rescales_and_keeps_the_mapping() {
    let one = style(11.0);
    let two = style(17.0);
    let (run, doubled) = with_fonts(|fs| {
        let run = shape_run(
            &[
                FlowSpan {
                    text: "ab",
                    style: &one,
                    link: None,
                },
                FlowSpan {
                    text: "cd",
                    style: &two,
                    link: None,
                },
            ],
            1.0,
            fs,
        );
        let doubled = reshape(&run, 2.0, None, fs);
        (run, doubled)
    });
    assert_eq!(doubled.glyphs.len(), 4);
    assert_eq!(doubled.glyphs.len(), run.glyphs.len());
    for (before, after) in run.glyphs.iter().zip(&doubled.glyphs) {
        assert_eq!((before.span, &before.bytes), (after.span, &after.bytes));
        close(after.advance, before.advance * 2.0, 1e-3, "advance");
    }
    close(doubled.line_height, run.line_height * 2.0, 1e-3, "line height");
}

#[test]
fn test_flow_reshape_appends_a_filler_on_its_own_line() {
    do_flow_reshape_appends_a_filler_on_its_own_line();
}

/// A filler is shaped in the flow's style on a line of its own: the
/// text's glyphs are unchanged, and the filler's glyphs report
/// `FILLER_SPAN` and bytes of the filler text, at the flow's size.
pub fn do_flow_reshape_appends_a_filler_on_its_own_line() {
    let flow = style(24.0);
    let (run, filled) = with_fonts(|fs| {
        let run = real_run("ab", &style(12.0), fs);
        let filled = reshape(&run, 1.0, Some(("~-", &flow)), fs);
        (run, filled)
    });
    assert_eq!(filled.glyphs.len(), 4);
    assert_eq!(&filled.glyphs[..2], &run.glyphs[..]);
    assert_eq!(filled.glyphs[2].span, FILLER_SPAN);
    assert_eq!(filled.glyphs[2].bytes, 0..1);
    assert_eq!(filled.glyphs[3].bytes, 1..2);
    assert_eq!(filled.buffer.lines.len(), 2);
    close(
        filled.line_height,
        24.0 * LINE_HEIGHT_FACTOR,
        1e-4,
        "the taller filler's line height",
    );
}

#[test]
fn test_flow_box_breaks_at_newlines() {
    do_flow_box_breaks_at_newlines();
}

/// A `\n` makes a second line, and the box is the widest line by the
/// two line heights; a `\r` is neither drawn nor a break.
pub fn do_flow_box_breaks_at_newlines() {
    let single = boxed("wi\rde", None, Align::Left);
    let shaped = boxed("wide\nab", None, Align::Left);
    let unbroken = boxed("wide", None, Align::Left);
    assert_eq!(single.buffer.layout_runs().count(), 1);
    assert_eq!(shaped.buffer.layout_runs().count(), 2);
    close(
        single.size.x,
        unbroken.size.x,
        1e-3,
        "a carriage return draws nothing",
    );
    close(
        shaped.size.x,
        unbroken.size.x,
        1e-3,
        "the box is as wide as its widest line",
    );
    let line = 20.0 * LINE_HEIGHT_FACTOR;
    close(shaped.size.y, 2.0 * line, 1e-3, "two lines tall");
    close(single.size.y, line, 1e-3, "one line tall");
    let spans: Vec<_> = shaped
        .glyph_spans
        .iter()
        .map(|(span, bytes)| (*span, bytes.clone()))
        .collect();
    assert_eq!(
        spans,
        vec![(0, 0..1), (0, 1..2), (0, 2..3), (0, 3..4), (0, 5..6), (0, 6..7)]
    );
}

#[test]
fn test_flow_box_counts_empty_lines() {
    do_flow_box_counts_empty_lines();
}

/// Every `\n` is a break, so an empty line between two others, a
/// trailing `\n` and a lone `\n` each add a line; empty text is one empty
/// line, as tall as its span's line height.
pub fn do_flow_box_counts_empty_lines() {
    let line = 20.0 * LINE_HEIGHT_FACTOR;
    for (text, lines) in [("ab\n\ncd", 3.0), ("ab\n", 2.0), ("\n", 2.0), ("", 1.0)] {
        close(boxed(text, None, Align::Left).size.y, lines * line, 1e-3, text);
    }
}

#[test]
fn test_flow_box_keeps_a_larger_minimum() {
    do_flow_box_keeps_a_larger_minimum();
}

/// A minimum size larger than the text is the box's size; a smaller one
/// is outgrown, per axis.
pub fn do_flow_box_keeps_a_larger_minimum() {
    let natural = boxed("hi", None, Align::Left).size;
    let large = boxed("hi", Some(Vec2::new(400.0, 300.0)), Align::Left).size;
    assert_eq!(large, Vec2::new(400.0, 300.0));
    let mixed = boxed("hi", Some(Vec2::new(400.0, 1.0)), Align::Left).size;
    assert!(natural.y > 1.0);
    assert_eq!(mixed, Vec2::new(400.0, natural.y));
}

#[test]
fn test_flow_box_aligns_lines() {
    do_flow_box_aligns_lines();
}

/// In a box wider than a line, `Center` starts the line half the slack
/// in, `Right` all of it, and `Left` none.
pub fn do_flow_box_aligns_lines() {
    let width = boxed("ab", None, Align::Left).size.x + 100.0;
    let first_x = |align| {
        let shaped = boxed("ab", Some(Vec2::new(width, 0.0)), align);
        shaped.buffer.layout_runs().next().unwrap().glyphs[0].x
    };
    close(first_x(Align::Left), 0.0, 1e-3, "left");
    close(first_x(Align::Center), 50.0, 1e-3, "center");
    close(first_x(Align::Right), 100.0, 1e-3, "right");
}

/// A `Rel` size reaching shaping is a caller error (#239 amendment 2,
/// ruling R6 reading 8): a debug build refuses it at once, and a release
/// build shapes at the minimum size. Plain `#[test]`: it drives a panic
/// (lib/baumhard/CONVENTIONS.md §B8, class 2).
#[test]
#[cfg_attr(debug_assertions, should_panic(expected = "reached shaping"))]
fn test_flow_rel_size_reaching_shaping_is_a_caller_error() {
    let mut relative = style(12.0);
    relative.size = crate::document::values::FontSize::Rel(0.1);
    assert_eq!(
        crate::flow::point_size(&relative),
        crate::font::fonts::MIN_FONT_SIZE_PT
    );
}
