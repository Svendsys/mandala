// SPDX-License-Identifier: MPL-2.0

//! The shaper's font fallback, as patched (V1-B14, #304;
//! `vendor/cosmic-text/src/shape.rs`): its merge of a fallback face's
//! glyphs makes the glyphs the published merge made, and costs work
//! linear in the run where the published one's was quadratic.
//!
//! Work is read off the shaper's own counters
//! ([`cosmic_text::ShapeWork`]), never a clock. They count this thread's
//! shaping, so another test shaping on its own thread between the two
//! reads of a difference does not move it.

use cosmic_text::{
    set_reference_merge, shape_work, Attrs, Buffer, Family, FontSystem, Metrics, ShapeWork, Wrap,
};

use crate::font::fonts::{acquire_font_system_write, pinned_family, AppFont};
use crate::font::SHAPING_ADVANCED;

/// The faces the oracle shapes its texts in: the default family, a face
/// holding almost nothing (so every letter falls back), a Latin text
/// face, and two faces of scripts with marks and reordering, one of them
/// right to left.
const FACES: [AppFont; 5] = [
    AppFont::Any,
    AppFont::MandalaOrnaments,
    AppFont::GalatiaSIL,
    AppFont::NotoSansHebrewRegular,
    AppFont::NotoSansDevanagariRegular,
];

/// The oracle's texts: plain words, ornaments glued to letters, right to
/// left and mixed direction, combining marks, conjuncts, code points no
/// face draws, emoji, ideographs, a tab, a ligature, and a long word.
const TEXTS: [&str; 15] = [
    "aaaa",
    "Sand, well-known \u{2726} star",
    "a\u{2726}b\u{2766}c\u{00b7}d",
    "\u{05e9}\u{05dc}\u{05d5}\u{05dd} \u{05e2}\u{05d5}\u{05dc}\u{05dd} \u{2726}",
    "abc \u{05e9}\u{05dc}\u{05d5}\u{05dd} def",
    "\u{0928}\u{092e}\u{0938}\u{094d}\u{0924}\u{0947} \u{0926}\u{0941}\u{0928}\u{093f}\u{092f}\u{093e}",
    "e\u{0301}x\u{0308}\u{0323} n\u{0303}",
    "\u{ffff}\u{fffe}a\u{10ffff}\u{e000}b",
    "\u{1f338}Frames\u{1f338}",
    "\u{65e5}\u{672c}\u{8a9e}\u{30c6}\u{30ad}\u{30b9}\u{30c8}",
    "a\tb\t\u{2726}",
    "\u{0645}\u{0631}\u{062d}\u{0628}\u{0627} abc",
    "\u{fb01} fi ffl office",
    "\u{2500}\u{2500}\u{2500}\u{2550}\u{2550}\u{2501}\u{2501}",
    "xyzzy\u{05d0}\u{0915}\u{2726}\u{1f338}\u{ffff}xyzzy",
];

/// `font`'s attributes at 20 points: its compiled family, or the
/// default family for [`AppFont::Any`].
fn attrs_of<'a>(fs: &FontSystem, font: AppFont, name: &'a mut String) -> Attrs<'a> {
    let attrs = Attrs::new().metrics(Metrics::new(20.0, 24.0));
    if font == AppFont::Any {
        return attrs;
    }
    *name = pinned_family(fs, font)
        .unwrap_or_else(|| panic!("precondition: {font:?} is a compiled family"))
        .name;
    attrs.family(Family::Name(name.as_str()))
}

/// `text` in `font`, shaped and laid out on one unwrapped line; and the
/// shaper's work for it. Under the caller's guard (`fs`).
fn shaped(fs: &mut FontSystem, font: AppFont, text: &str) -> (Buffer, ShapeWork) {
    let mut name = String::new();
    let attrs = attrs_of(fs, font, &mut name);
    let mut buffer = Buffer::new(fs, Metrics::new(20.0, 24.0));
    buffer.set_wrap(Wrap::None);
    buffer.set_size(None, None);
    let before = shape_work();
    buffer.set_rich_text([(text, attrs.clone())], &attrs, SHAPING_ADVANCED, None);
    buffer.shape_until_scroll(fs, false);
    let after = shape_work();
    (buffer, difference(after, before))
}

/// `after - before`, counter by counter.
fn difference(after: ShapeWork, before: ShapeWork) -> ShapeWork {
    ShapeWork {
        shapes: after.shapes - before.shapes,
        bytes_shaped: after.bytes_shaped - before.bytes_shaped,
        fallback_rounds: after.fallback_rounds - before.fallback_rounds,
        merge_steps: after.merge_steps - before.merge_steps,
    }
}

/// Everything a laid-out buffer says about its glyphs: each line's width
/// and each glyph's every field.
fn layout_of(buffer: &Buffer) -> Vec<String> {
    buffer
        .layout_runs()
        .flat_map(|run| {
            std::iter::once(format!("line {} w {}", run.line_i, run.line_w))
                .chain(run.glyphs.iter().map(|glyph| format!("{glyph:?}")))
        })
        .collect()
}

/// `f` with this thread merging by the published algorithm when
/// `reference` is set; the previous setting restored after.
fn merging<R>(reference: bool, f: impl FnOnce() -> R) -> R {
    let was = set_reference_merge(reference);
    let out = f();
    set_reference_merge(was);
    out
}

#[test]
fn test_the_fallback_merge_lays_out_what_the_published_merge_did() {
    do_the_fallback_merge_lays_out_what_the_published_merge_did();
}

/// The oracle: every text of [`TEXTS`] in every face of [`FACES`], and a
/// long word of each, is laid out glyph for glyph as the published merge
/// lays it out. The corpus is held to have made the merge choose: some
/// shapings fell back, and the linear merge counted less work than the
/// published one did over them.
pub fn do_the_fallback_merge_lays_out_what_the_published_merge_did() {
    crate::font::fonts::init();
    let long: Vec<String> = TEXTS.iter().map(|text| text.repeat(40)).collect();
    let mut fs = acquire_font_system_write("test_the_fallback_merge_lays_out_what_the_published_merge_did");
    let mut linear_steps = 0;
    let mut reference_steps = 0;
    let mut rounds = 0;
    for font in FACES {
        for text in TEXTS.iter().copied().chain(long.iter().map(String::as_str)) {
            let (linear, work) = merging(false, || shaped(&mut fs, font, text));
            let (reference, reference_work) = merging(true, || shaped(&mut fs, font, text));
            assert_eq!(
                layout_of(&linear),
                layout_of(&reference),
                "{font:?} {text:?}: the linear merge laid out glyphs the published merge did not"
            );
            assert_eq!(
                (work.shapes, work.bytes_shaped, work.fallback_rounds),
                (
                    reference_work.shapes,
                    reference_work.bytes_shaped,
                    reference_work.fallback_rounds
                ),
                "{font:?} {text:?}: the two merges shaped in different faces"
            );
            linear_steps += work.merge_steps;
            reference_steps += reference_work.merge_steps;
            rounds += work.fallback_rounds;
        }
    }
    assert!(
        rounds > 0 && linear_steps > 0,
        "precondition: the corpus fell back ({rounds} rounds), or the merge was never asked to choose"
    );
    assert!(
        linear_steps < reference_steps,
        "precondition: the published merge counts its scans and shifts ({reference_steps} steps against {linear_steps})"
    );
}

#[test]
fn test_a_word_its_face_lacks_costs_fallback_work_linear_in_its_length() {
    do_a_word_its_face_lacks_costs_fallback_work_linear_in_its_length();
}

/// One word of `n` letters, pinned to a face that holds none of them, so
/// every glyph is missing until a fallback face draws it, at `n`, `2n`
/// and `4n`: the bytes shaped and the merge's steps are each linear in
/// `n` — a fixed number of steps per byte, the same at every length.
///
/// The precondition is the published merge on the same words: its
/// counted work quadruples when the word doubles, so the counters see
/// the quadratic cost the patch removed (the 596 s of a 262 KB span).
pub fn do_a_word_its_face_lacks_costs_fallback_work_linear_in_its_length() {
    crate::font::fonts::init();
    const N: usize = 1000;
    let word = |n: usize| "a".repeat(n);
    let mut fs =
        acquire_font_system_write("test_a_word_its_face_lacks_costs_fallback_work_linear_in_its_length");
    let face = AppFont::MandalaOrnaments;
    let at =
        |fs: &mut FontSystem, n: usize, reference: bool| merging(reference, || shaped(fs, face, &word(n))).1;

    let reference = [at(&mut fs, N, true), at(&mut fs, 2 * N, true)];
    assert!(
        reference[0].fallback_rounds > 0,
        "precondition: {face:?} lacks `a`, so the word falls back"
    );
    assert!(
        reference[1].merge_steps >= 3 * reference[0].merge_steps,
        "precondition: the published merge's counted work grows with the square of the word \
         ({} steps at {N} letters, {} at {})",
        reference[0].merge_steps,
        reference[1].merge_steps,
        2 * N
    );

    let works = [
        at(&mut fs, N, false),
        at(&mut fs, 2 * N, false),
        at(&mut fs, 4 * N, false),
    ];
    assert!(
        works
            .iter()
            .all(|work| work.fallback_rounds == works[0].fallback_rounds),
        "the word is shaped in as many faces at every length: {works:?}"
    );
    // Linear means affine in the length: what the word adds from n to 2n
    // it adds twice over from 2n to 4n, so the second difference is
    // zero, and the slope is a whole number of units per letter. (The
    // constant part is the line's ellipsis, which cosmic-text shapes
    // beside every line, and the merge's few steps per round.)
    let slope = |count: fn(&ShapeWork) -> u64, what: &str| {
        let [one, two, four] = works.each_ref().map(count);
        assert_eq!(
            four - two,
            2 * (two - one),
            "{what}: {one}, {two} and {four} at {N}, {} and {} letters are not linear in the length",
            2 * N,
            4 * N
        );
        assert_eq!((two - one) % N as u64, 0, "{what}: a whole number per letter");
        (two - one) / N as u64
    };
    let shaped_per_byte = slope(|work| work.bytes_shaped, "bytes shaped");
    let merged_per_byte = slope(|work| work.merge_steps, "merge steps");
    assert!(
        shaped_per_byte >= 2,
        "precondition: the word is shaped in its face and at least one fallback face ({shaped_per_byte} per letter)"
    );
    assert!(
        merged_per_byte > 0,
        "precondition: the merge did the fallback's work"
    );
}
