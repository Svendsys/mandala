// SPDX-License-Identifier: MPL-2.0

//! The shaper's font fallback, as patched (V1-B14, #304;
//! `vendor/cosmic-text/src/shape.rs`): its merge of a fallback face's
//! glyphs makes the glyphs the published merge made, and costs work
//! linear in the run where the published one's was quadratic; and a code
//! point no face draws is asked of the faces once per font system, after
//! which a run holding it tries no fallback face for it.
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
pub const FACES: [AppFont; 5] = [
    AppFont::Any,
    AppFont::MandalaOrnaments,
    AppFont::GalatiaSIL,
    AppFont::NotoSansHebrewRegular,
    AppFont::NotoSansDevanagariRegular,
];

/// The oracle's texts: plain words, ornaments glued to letters, right to
/// left and mixed direction, combining marks, conjuncts, code points no
/// face draws, emoji, ideographs, a tab, a ligature, and a long word.
pub const TEXTS: [&str; 15] = [
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

/// `f` under the font lock, which is held for `f` alone and never across
/// an assertion, so a failing one does not poison the lock for every other
/// font test.
fn under_lock<R>(f: impl FnOnce(&mut FontSystem) -> R) -> R {
    let mut guard = acquire_font_system_write("font::tests::shape_work_tests");
    f(&mut guard)
}

/// `text` in `font`, shaped and laid out on one unwrapped line; and the
/// shaper's work for it.
fn shaped(font: AppFont, text: &str) -> (Buffer, ShapeWork) {
    under_lock(|fs| shaped_in(fs, font, text))
}

/// [`shaped`], under the guard `fs`.
fn shaped_in(fs: &mut FontSystem, font: AppFont, text: &str) -> (Buffer, ShapeWork) {
    let mut name = String::new();
    let attrs = attrs_of(fs, font, &mut name);
    let mut buffer = Buffer::new(fs, Metrics::new(20.0, 24.0));
    buffer.set_wrap(Wrap::None);
    buffer.set_size(None, None);
    let before = shape_work();
    buffer.set_rich_text([(text, attrs.clone())], &attrs, SHAPING_ADVANCED, None);
    buffer.shape_until_scroll(fs, false);
    let after = shape_work();
    (buffer, after.since(before))
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
/// and loop lay it out, though the patched loop stops early at code
/// points no face draws. The corpus is held to have made the merge
/// choose and the loop stop: some shapings fell back, the linear merge
/// counted less work than the published one did over them, and the
/// patched loop shaped in fewer fallback faces.
pub fn do_the_fallback_merge_lays_out_what_the_published_merge_did() {
    crate::font::fonts::init();
    let long: Vec<String> = TEXTS.iter().map(|text| text.repeat(40)).collect();
    let mut linear_steps = 0;
    let mut reference_steps = 0;
    let mut rounds = 0;
    let mut reference_rounds = 0;
    for font in FACES {
        for text in TEXTS.iter().copied().chain(long.iter().map(String::as_str)) {
            let (linear, work) = merging(false, || shaped(font, text));
            let (reference, reference_work) = merging(true, || shaped(font, text));
            assert_eq!(
                layout_of(&linear),
                layout_of(&reference),
                "{font:?} {text:?}: the linear merge laid out glyphs the published merge did not"
            );
            assert!(
                work.fallback_rounds <= reference_work.fallback_rounds,
                "{font:?} {text:?}: the patched loop tried more fallback faces than the published one"
            );
            linear_steps += work.merge_steps;
            reference_steps += reference_work.merge_steps;
            rounds += work.fallback_rounds;
            reference_rounds += reference_work.fallback_rounds;
        }
    }
    assert!(
        rounds > 0 && linear_steps > 0,
        "precondition: the corpus fell back ({rounds} rounds), or the merge was never asked to choose"
    );
    assert!(
        rounds < reference_rounds,
        "precondition: the loop stopped early on some text ({rounds} rounds against {reference_rounds})"
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
    let face = AppFont::MandalaOrnaments;
    let at = |n: usize, reference: bool| merging(reference, || shaped(face, &word(n))).1;

    let reference = [at(N, true), at(2 * N, true)];
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

    // Once first, so that asking whether some face draws `a`, which the
    // font system answers once (`do_a_code_point_no_face_draws_...`), is
    // not counted at one length and not the others.
    at(N, false);
    let works = [at(N, false), at(2 * N, false), at(4 * N, false)];
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

/// A code point no face draws, for the hundred-in-a-row test below: a
/// private-use code point of plane 16 that no other test shapes.
pub const UNDRAWN: char = '\u{10fff5}';

/// The next of the private-use code points U+10F000 to U+10FEFF, which no
/// other test shapes and no face draws: a code point whose answer the
/// font system has not kept, on every call, so a test that needs one cold
/// gets one however often it runs in the process (a bench repeats it).
/// Nothing is cleared to get it, so no other test's kept answers move.
fn fresh_undrawn() -> char {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let k = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 0xf00;
    char::from_u32(0x10_f000 + k).unwrap_or('\u{10f000}')
}

#[test]
fn test_a_code_point_no_face_draws_is_asked_of_the_faces_once() {
    do_a_code_point_no_face_draws_is_asked_of_the_faces_once();
}

/// Text in a face that draws its letters, with one code point no face
/// draws, which the font system has not been asked about
/// ([`fresh_undrawn`]): the first shaping asks the font system's faces
/// about it, once each at most, and tries no fallback face; every later
/// shaping, of that text or of any other holding the code point, asks
/// nothing and tries no fallback face either, and costs what the same
/// text without it costs. The published loop, the precondition, tried
/// the run in fallback faces every time.
pub fn do_a_code_point_no_face_draws_is_asked_of_the_faces_once() {
    crate::font::fonts::init();
    let face = AppFont::GalatiaSIL;
    let undrawn = fresh_undrawn();
    let reference_text = format!("ab{}cd", fresh_undrawn());
    let text = format!("ab{undrawn}cd");
    let faces = under_lock(|fs| fs.db().faces().count() as u64);

    let reference = merging(true, || shaped(face, &reference_text)).1;
    assert!(
        reference.fallback_rounds > 0,
        "precondition: the published loop tries fallback faces for a code point no face draws"
    );
    let plain = shaped(face, "abcd").1;
    assert_eq!(
        plain.fallback_rounds, 0,
        "precondition: {face:?} draws the letters"
    );

    let first = shaped(face, &text).1;
    assert!(
        first.coverage_probes > 0 && first.coverage_probes <= faces,
        "the first shaping asks the faces about {undrawn:?}, each once at most: {} of {faces}",
        first.coverage_probes
    );
    assert_eq!(first.fallback_rounds, 0, "and tries no fallback face");

    for again in [text.clone(), format!("x{undrawn}y{undrawn}")] {
        let work = shaped(face, &again).1;
        assert_eq!(
            (work.coverage_probes, work.fallback_rounds),
            (0, 0),
            "{again:?}: the answer is kept, so nothing is asked and no fallback face is tried"
        );
        assert_eq!(
            work.shapes, plain.shapes,
            "{again:?}: shaped as often as the text without the code point"
        );
    }
}

#[test]
fn test_a_hundred_contiguous_undrawn_code_points_cost_one_shaping() {
    do_a_hundred_contiguous_undrawn_code_points_cost_one_shaping();
}

/// About a hundred contiguous code points no face draws (#304, from the
/// B9 review): what they cost does not grow with their number. Shaped
/// with letters around them, a hundred of them try no fallback face and
/// cost the shapings two of them cost; the published loop's rounds over
/// them, the precondition, are each a shaping of the whole run.
pub fn do_a_hundred_contiguous_undrawn_code_points_cost_one_shaping() {
    crate::font::fonts::init();
    let face = AppFont::GalatiaSIL;
    let run = |n: usize| format!("one {} two", UNDRAWN.to_string().repeat(n));
    // Once, so the code point's answer is kept before anything is read.
    shaped(face, &run(1));

    let reference = merging(true, || shaped(face, &run(100))).1;
    assert!(
        reference.fallback_rounds > 0,
        "precondition: the published loop shapes the run in fallback faces"
    );
    let two = shaped(face, &run(2)).1;
    let hundred = shaped(face, &run(100)).1;
    assert_eq!(
        (hundred.fallback_rounds, hundred.coverage_probes),
        (0, 0),
        "a hundred undrawn code points try no fallback face and ask nothing"
    );
    assert_eq!(hundred.shapes, two.shapes, "a hundred cost the shapings two cost");
}
