// SPDX-License-Identifier: MPL-2.0

//! What projecting a long span costs, counted (V1-B14, #304): one span of
//! one word, the shape that took 596 s to project at 262 KB (#243), costs
//! shaping work linear in its length.
//!
//! The cost was cosmic-text's font fallback merge, quadratic in a run its
//! first face lacks; the patch is in `vendor/cosmic-text/src/shape.rs`,
//! and `font::tests::shape_work_tests` holds the merge itself. This holds
//! the projection to it, end to end, through the loader.

use cosmic_text::ShapeWork;

use crate::document::format::load;
use crate::projection::tests::support::fonts;
use crate::projection::{GlyphLayers, Projector};

/// A map of one node with one section of one span: `n` letters `a`, in
/// `font` when it is given.
fn one_span(n: usize, font: Option<&str>) -> String {
    let font = font.map_or_else(String::new, |font| format!(r#","font":"{font}""#));
    format!(
        r#"{{"format":1,"name":"m","nodes":[{{"id":1,"position":[0,0],"sections":[{{"spans":[{{"text":"{}"{font}}}]}}]}}]}}"#,
        "a".repeat(n)
    )
}

/// The shaping work of projecting `text` in full. The lease is let go
/// before the assertion, so a failing one does not poison the font lock.
fn projected(text: &str) -> ShapeWork {
    let doc = load(text).unwrap_or_else(|error| panic!("precondition: the map loads: {error}"));
    let mut layers = GlyphLayers::new();
    let mut fonts = fonts();
    let before = cosmic_text::shape_work();
    let projector = Projector::project_full(&doc, &mut layers, &mut fonts);
    let after = cosmic_text::shape_work();
    drop(fonts);
    assert_eq!(projector.nodes.len(), 1, "precondition: the node is projected");
    after.since(before)
}

/// The work of one span of `n`, `2n` and `4n` letters, in `font`.
fn at_three_lengths(n: usize, font: Option<&str>) -> [ShapeWork; 3] {
    // The counters are this thread's, so another test's shaping is not
    // counted between two readings, though each projection takes the font
    // lock afresh.
    // Once first: the font system answers once whether some face draws a
    // letter (`cosmic_text::ShapeWork::coverage_probes`), and that answer
    // is no part of what one more letter costs.
    projected(&one_span(n, font));
    [n, 2 * n, 4 * n].map(|n| projected(&one_span(n, font)))
}

/// Whether `count` is affine in the length over `works` (at `n`, `2n`,
/// `4n`): what `n` more letters add, `2n` more add twice over. Its slope
/// per letter, or why not.
fn slope(works: &[ShapeWork; 3], n: usize, count: fn(&ShapeWork) -> u64) -> Result<u64, String> {
    let [one, two, four] = works.each_ref().map(count);
    if four - two != 2 * (two - one) || (two - one) % n as u64 != 0 {
        return Err(format!(
            "{one}, {two} and {four} at {n}, {} and {} letters",
            2 * n,
            4 * n
        ));
    }
    Ok((two - one) / n as u64)
}

#[test]
fn test_a_long_span_projects_in_shaping_work_linear_in_its_length() {
    const N: usize = 4096;
    // Pinned to a face that holds none of its letters, so every glyph
    // falls back on every machine; and in the default family, the shape
    // #243 measured, which falls back on a machine without cosmic-text's
    // default face.
    for font in [Some("Mandala Ornaments"), None] {
        let works = at_three_lengths(N, font);
        if font.is_some() {
            assert!(
                works[0].fallback_rounds > 0,
                "precondition: the span's face lacks its letters, so it falls back: {works:?}"
            );
        }
        assert!(
            works.iter().all(|work| work.shapes == works[0].shapes),
            "{font:?}: the span is shaped as many times at every length: {works:?}"
        );
        let shaped = slope(&works, N, |work| work.bytes_shaped)
            .unwrap_or_else(|why| panic!("{font:?}: bytes shaped are not linear in the span: {why}"));
        let merged = slope(&works, N, |work| work.merge_steps)
            .unwrap_or_else(|why| panic!("{font:?}: merge steps are not linear in the span: {why}"));
        assert!(shaped >= 1, "precondition: the span is shaped");
        println!(
            "{font:?}: per byte of the span, {shaped} bytes shaped and {merged} merge steps ({works:?})"
        );
    }
}

#[test]
fn test_the_linear_slope_is_one_the_published_merge_does_not_have() {
    // The precondition of the test above, as a test: on the same span, the
    // published merge's counted work is not linear, so `slope` can tell.
    const N: usize = 1024;
    let was = cosmic_text::set_reference_merge(true);
    let works = at_three_lengths(N, Some("Mandala Ornaments"));
    cosmic_text::set_reference_merge(was);
    assert!(
        slope(&works, N, |work| work.merge_steps).is_err(),
        "the published merge's work on a span its face lacks grows faster than the span: {works:?}"
    );
}
