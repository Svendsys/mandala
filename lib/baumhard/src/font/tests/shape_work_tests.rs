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

/// A code point no face draws, for the tests below: a private-use code
/// point of plane 16.
pub const UNDRAWN: char = '\u{10fff5}';

/// A font system of its own, not the locked global one, holding only
/// `fonts`' compiled faces: what it keeps — whether a face draws a code
/// point, among the rest — starts empty, so a body that builds one repeats
/// however often it runs in a process (a bench repeats it), and nothing it
/// does moves another test's kept answers.
fn private_font_system(fonts: &[AppFont]) -> FontSystem {
    crate::font::fonts::init();
    let mut db = cosmic_text::fontdb::Database::new();
    for font in fonts {
        let source = crate::font::fonts::FONT_SOURCES
            .get(font)
            .unwrap_or_else(|| panic!("precondition: {font:?} is a compiled face"));
        db.load_font_source(source.clone());
    }
    FontSystem::new_with_locale_and_db_and_fallback("en-US".to_string(), db, cosmic_text::PlatformFallback)
}

/// The family name `font`'s compiled face stores, in `fs` (any font
/// system holding that face).
fn family_in(fs: &FontSystem, font: AppFont) -> String {
    let probe = private_font_system(&[font]);
    let name = probe
        .db()
        .faces()
        .next()
        .and_then(|face| face.families.first())
        .map(|(name, _)| name.clone())
        .unwrap_or_else(|| panic!("precondition: {font:?}'s face names a family"));
    assert!(
        fs.db()
            .faces()
            .any(|face| face.families.iter().any(|(n, _)| *n == name)),
        "precondition: the font system holds {font:?}'s family"
    );
    name
}

/// `text` in the family `family`, shaped in the private font system `fs`;
/// and the shaper's work for it.
fn shaped_in_family(fs: &mut FontSystem, family: &str, text: &str) -> (Buffer, ShapeWork) {
    let attrs = Attrs::new()
        .metrics(Metrics::new(20.0, 24.0))
        .family(Family::Name(family));
    let mut buffer = Buffer::new_empty(Metrics::new(20.0, 24.0));
    buffer.set_wrap(Wrap::None);
    buffer.set_size(None, None);
    let before = shape_work();
    buffer.set_rich_text([(text, attrs.clone())], &attrs, SHAPING_ADVANCED, None);
    buffer.shape_until_scroll(fs, false);
    (buffer, shape_work().since(before))
}

/// The glyph ids `buffer` lays out, in order.
fn glyph_ids(buffer: &Buffer) -> Vec<u16> {
    buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.glyph_id))
        .collect()
}

#[test]
fn test_a_code_point_no_face_draws_is_asked_of_the_faces_once() {
    do_a_code_point_no_face_draws_is_asked_of_the_faces_once();
}

/// Text in a face that draws its letters, with one code point no face
/// draws ([`UNDRAWN`]), in a font system of its own
/// (`private_font_system`, so it repeats): the first shaping asks the
/// font system's faces about it, once each at most, and tries no fallback
/// face; every later shaping, of that text or of any other holding the
/// code point, asks nothing and tries no fallback face either, and costs
/// what the same text without it costs. The published loop, the
/// precondition, tried the run in fallback faces every time.
pub fn do_a_code_point_no_face_draws_is_asked_of_the_faces_once() {
    let mut fs = private_font_system(&[AppFont::GalatiaSIL, AppFont::MandalaOrnaments]);
    let family = family_in(&fs, AppFont::GalatiaSIL);
    let undrawn = UNDRAWN;
    let text = format!("ab{undrawn}cd");
    let faces = fs.db().faces().count() as u64;
    let mut shaped = |text: &str| shaped_in_family(&mut fs, &family, text).1;

    let reference = merging(true, || shaped(&text));
    assert!(
        reference.fallback_rounds > 0,
        "precondition: the published loop tries fallback faces for a code point no face draws"
    );
    let plain = shaped("abcd");
    assert_eq!(
        plain.fallback_rounds, 0,
        "precondition: {family} draws the letters"
    );

    let first = shaped(&text);
    assert!(
        first.coverage_probes > 0 && first.coverage_probes <= faces,
        "the first shaping asks the faces about {undrawn:?}, each once at most: {} of {faces}",
        first.coverage_probes
    );
    assert_eq!(first.fallback_rounds, 0, "and tries no fallback face");

    for again in [text.clone(), format!("x{undrawn}y{undrawn}")] {
        let work = shaped(&again);
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
fn test_a_face_added_to_the_database_draws_what_no_face_drew() {
    do_a_face_added_to_the_database_draws_what_no_face_drew();
}

/// The kept answer that no face draws a code point is let go when the
/// font system's database changes (#304 review: `db_mut` clears it). In a
/// font system of its own holding one Latin face, an alef is drawn by no
/// face, and asked about; once the Hebrew face is loaded into that
/// database, the same text draws it.
pub fn do_a_face_added_to_the_database_draws_what_no_face_drew() {
    let mut fs = private_font_system(&[AppFont::GalatiaSIL]);
    let family = family_in(&fs, AppFont::GalatiaSIL);
    let text = "a\u{05d0}b";
    let (before, work) = shaped_in_family(&mut fs, &family, text);
    let before = glyph_ids(&before);
    assert!(
        before.len() == 3 && before[1] == 0 && before[0] != 0 && before[2] != 0,
        "precondition: the Latin face draws the letters and no face the alef: {before:?}"
    );
    assert!(
        work.coverage_probes > 0,
        "precondition: the alef is asked about, and its answer kept"
    );
    let hebrew = crate::font::fonts::FONT_SOURCES
        .get(&AppFont::NotoSansHebrewRegular)
        .expect("precondition: the Hebrew face is compiled")
        .clone();
    fs.db_mut().load_font_source(hebrew);
    let after = glyph_ids(&shaped_in_family(&mut fs, &family, text).0);
    assert!(
        after.len() == 3 && after[1] != 0,
        "a face added to the database draws the alef no face drew: {after:?}"
    );
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

/// A shaped glyph of bytes `start..end` with glyph id `id`, for the
/// merge's and the early stop's fixtures: every other field a plain value
/// the decisions under test do not read.
fn glyph(start: usize, end: usize, id: u16) -> cosmic_text::ShapeGlyph {
    cosmic_text::ShapeGlyph {
        start,
        end,
        x_advance: 1.0,
        y_advance: 0.0,
        x_offset: 0.0,
        y_offset: 0.0,
        ascent: 0.8,
        descent: 0.2,
        font_monospace_em_width: None,
        font_id: cosmic_text::fontdb::ID::dummy(),
        font_weight: cosmic_text::fontdb::Weight::NORMAL,
        glyph_id: id,
        color_opt: None,
        metadata: 0,
        cache_key_flags: cosmic_text::CacheKeyFlags::empty(),
        metrics_opt: None,
    }
}

/// `glyphs`' every field, for comparing two merges' results.
fn fields(glyphs: &[cosmic_text::ShapeGlyph]) -> Vec<String> {
    glyphs.iter().map(|glyph| format!("{glyph:?}")).collect()
}

#[test]
fn test_a_run_the_merge_cannot_reason_about_is_merged_as_published() {
    do_a_run_the_merge_cannot_reason_about_is_merged_as_published();
}

/// The linear merge reasons from the run's glyphs lying forward and in
/// visual order (#304 review, shape.rs:909): it finds the cluster a glyph
/// lies within by the glyph's first byte. A run glyph that ends before it
/// starts, which no shaping at harfrust's default cluster level makes,
/// would be judged otherwise by the published merge, so such a run is
/// merged by the published algorithm, and gives its result. A well-formed
/// run, the precondition, is merged the linear way to the same result.
pub fn do_a_run_the_merge_cannot_reason_about_is_merged_as_published() {
    use cosmic_text::mandala_test_hooks::fallback_round;
    // Nine bytes. The fallback draws bytes 3..6, which the run is missing.
    let fb = vec![glyph(0, 3, 0), glyph(3, 6, 7), glyph(6, 9, 0)];
    let fb_missing = [0, 6];
    let well = vec![glyph(0, 3, 5), glyph(3, 6, 0), glyph(6, 9, 5)];
    // Byte 7 to byte 4: lies within 3..6 by the published test (starts at
    // or after 3, ends by 6), and not by its first byte.
    let ill = vec![glyph(0, 3, 5), glyph(3, 6, 0), glyph(7, 4, 9), glyph(6, 9, 5)];
    for (what, run) in [("a well-formed run", well), ("a run with a backward glyph", ill)] {
        let merged = |published: bool| {
            let mut glyphs = run.clone();
            let mut missing = vec![3];
            fallback_round(
                &mut glyphs,
                &mut missing,
                fb.clone(),
                &fb_missing,
                0,
                9,
                false,
                published,
            );
            (fields(&glyphs), missing)
        };
        let (published, patched) = (merged(true), merged(false));
        assert!(
            published.0.len() < run.len() + 1 && published.1.is_empty(),
            "precondition: {what}: the published merge drew the missing cluster: {published:?}"
        );
        assert_eq!(
            patched, published,
            "{what}: the patched merge differs from the published one"
        );
    }
}

#[test]
fn test_a_fallback_the_merge_cannot_reason_about_is_merged_as_published() {
    do_a_fallback_the_merge_cannot_reason_about_is_merged_as_published();
}

/// The linear merge reasons from the fallback's cluster starts being
/// monotone in visual order, which makes the clusters it chooses disjoint
/// (#304 review 5813361973, shape.rs:656). A fallback listed out of order,
/// one cluster at bytes 6..9 and then one at 3..9 that covers it, both
/// over bytes the run is missing, would have the linear merge keep the
/// first cluster's glyph, where the published merge removes it when the
/// second is put in. So such a fallback is merged by the published
/// algorithm, and gives its result.
pub fn do_a_fallback_the_merge_cannot_reason_about_is_merged_as_published() {
    use cosmic_text::mandala_test_hooks::fallback_round;
    let run = vec![glyph(0, 3, 5), glyph(3, 6, 0), glyph(6, 9, 0)];
    let fb = vec![glyph(6, 9, 8), glyph(3, 9, 7)];
    assert!(
        fb[1].start < fb[0].start,
        "precondition: left to right, the fallback's cluster starts fall"
    );
    let merged = |published: bool| {
        let mut glyphs = run.clone();
        let mut missing = vec![3, 6];
        fallback_round(&mut glyphs, &mut missing, fb.clone(), &[], 0, 9, false, published);
        (fields(&glyphs), missing)
    };
    let (published, patched) = (merged(true), merged(false));
    assert!(
        published.0.len() == 2 && published.1.is_empty(),
        "precondition: the published merge leaves the run's first glyph and the covering cluster: {published:?}"
    );
    assert_eq!(
        patched, published,
        "a fallback out of order: the patched merge differs from the published one"
    );
}

/// A font system with no faces at all: every code point is one no face
/// draws, so the early stop's question turns on its guards alone.
fn faceless_font_system() -> FontSystem {
    FontSystem::new_with_locale_and_db_and_fallback(
        "en-US".to_string(),
        cosmic_text::fontdb::Database::new(),
        cosmic_text::PlatformFallback,
    )
}

#[test]
fn test_the_early_stop_needs_every_missing_entry_matched() {
    do_the_early_stop_needs_every_missing_entry_matched();
}

/// The early stop (#304 review, shape.rs:757) stops only when the missing
/// list and the run's missing glyphs match one to one. A missing entry
/// with no missing glyph at its byte, a case shaping does not make, is left
/// to the published loop, which keeps trying faces for it: no stop, though
/// the one missing glyph is a code point no face draws. The precondition:
/// with the entries matched, it stops.
pub fn do_the_early_stop_needs_every_missing_entry_matched() {
    use cosmic_text::mandala_test_hooks::stops_early;
    let mut fs = faceless_font_system();
    let weight = cosmic_text::fontdb::Weight::NORMAL;
    let glyphs = [glyph(0, 1, 0), glyph(1, 2, 3)];
    assert!(
        stops_early(&mut fs, &glyphs, &[0], "xy", 0, 2, false, weight),
        "precondition: one missing glyph, no face to draw it, its entry matched: the loop stops"
    );
    assert!(
        !stops_early(&mut fs, &glyphs, &[0, 1], "xy", 0, 2, false, weight),
        "an entry at byte 1, where no glyph is missing, must keep the loop going"
    );
}

#[test]
fn test_the_early_stop_leaves_a_cluster_of_several_code_points_to_the_faces() {
    do_the_early_stop_leaves_a_cluster_of_several_code_points_to_the_faces();
}

/// The early stop judges only clusters of one code point (#304 review,
/// shape.rs:757): a face may draw the composition of several (`e` and a
/// combining acute as `é`) though it draws none of them alone, so a
/// missing cluster of several is left to the faces, however undrawn its
/// first code point is. The precondition: the same code point alone stops
/// the loop.
pub fn do_the_early_stop_leaves_a_cluster_of_several_code_points_to_the_faces() {
    use cosmic_text::mandala_test_hooks::stops_early;
    let mut fs = faceless_font_system();
    let weight = cosmic_text::fontdb::Weight::NORMAL;
    assert!(
        stops_early(&mut fs, &[glyph(0, 1, 0)], &[0], "e", 0, 1, false, weight),
        "precondition: `e` alone, which no face here draws, stops the loop"
    );
    let composed = "e\u{0301}";
    assert!(
        !stops_early(
            &mut fs,
            &[glyph(0, composed.len(), 0)],
            &[0],
            composed,
            0,
            composed.len(),
            false,
            weight
        ),
        "a missing cluster of `e` and a combining acute must be left to the faces"
    );
}

#[test]
fn test_two_font_systems_never_answer_each_others_outlines() {
    do_two_font_systems_never_answer_each_others_outlines();
}

/// The outline cache keeps each entry for its font system (#304 reviews,
/// fonts.rs:1447 and :1054): a face id means a face only within its own
/// database, and two font systems of their own, each holding one face,
/// give their faces the same id. An ornament measured in the first is
/// read and kept; the same glyph id of the same face id, measured in the
/// second, is read off the second's face, not answered from the first's
/// entry; each is answered from its own entry the next time; and a font
/// system assigned in the first's place, at its very address, is not
/// answered from the first's entries either. The precondition: the two
/// faces have the same id.
pub fn do_two_font_systems_never_answer_each_others_outlines() {
    use crate::font::fonts::{glyph_outline_ink, outline_reads};
    use cosmic_text::SwashCache;
    let mut ornaments = private_font_system(&[AppFont::MandalaOrnaments]);
    let mut latin = private_font_system(&[AppFont::GalatiaSIL]);
    let first_face = |fs: &FontSystem| fs.db().faces().next().map(|face| face.id);
    assert_eq!(
        first_face(&ornaments),
        first_face(&latin),
        "precondition: the two font systems give their faces one id"
    );
    let family = family_in(&ornaments, AppFont::MandalaOrnaments);
    let (buffer, _) = shaped_in_family(&mut ornaments, &family, "\u{2726}");
    let laid = buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter().cloned())
        .next()
        .expect("precondition: the ornament is laid out");
    let mut cache = SwashCache::new();
    let mut measure = |fs: &mut FontSystem| {
        let before = outline_reads();
        let ink = glyph_outline_ink(fs, &mut cache, &laid);
        (ink, outline_reads() - before)
    };
    let first = measure(&mut ornaments);
    let other = measure(&mut latin);
    let first_again = measure(&mut ornaments);
    let other_again = measure(&mut latin);
    assert!(first.0.is_some(), "precondition: the ornament has ink");
    assert_eq!(first.1, 1, "the first font system reads its outline");
    assert_eq!(
        other.1, 1,
        "the second font system reads its own face's outline, not the first's entry"
    );
    assert_eq!(
        (first_again, other_again),
        ((first.0, 0), (other.0, 0)),
        "each is answered from its own entry the next time"
    );
    // A value put in the first's place, at its very address, as an
    // assignment through a guard would: its faces' ids start again too,
    // and it is not answered from the entry kept at that address.
    let address = std::ptr::from_ref(&ornaments) as usize;
    ornaments = private_font_system(&[AppFont::GalatiaSIL]);
    assert_eq!(
        std::ptr::from_ref(&ornaments) as usize,
        address,
        "precondition: the replacement is at the same address"
    );
    let replaced = measure(&mut ornaments);
    assert_eq!(
        replaced.1, 1,
        "a font system put in another's place reads its own face's outline"
    );
}

#[test]
fn test_whether_a_glyph_draws_is_read_once_whatever_its_size() {
    do_whether_a_glyph_draws_is_read_once_whatever_its_size();
}

/// Whether a glyph draws is read off the rasterizer once, at
/// `DRAWS_SIZE`, whatever size the glyph is asked about at (#333): a
/// bitmap emoji, 🌸 in MandalaEmoji, which has no outline, asked about
/// laid at 12, 512 and 4,096 points, is rasterized once and draws at each.
/// So no frame probing sizes rasterizes a glyph per size, nor an image as
/// large as its largest: the seed
/// `fuzz/seeds/flow_structured_every_face/pins_mandala_ornaments_28` had a
/// host's color emoji rasterized at 4,096 pixels per em on each probe,
/// past the fuzz timeout on CI.
/// Precondition: the emoji has no outline.
///
/// Control: the glyph asked about at its own size, red: three reads
/// ("3 rasterizations for three sizes").
pub fn do_whether_a_glyph_draws_is_read_once_whatever_its_size() {
    use crate::font::fonts::{draw_reads, glyph_draws, glyph_outline_ink};
    use cosmic_text::SwashCache;
    let mut emoji = private_font_system(&[AppFont::MandalaEmoji]);
    let family = family_in(&emoji, AppFont::MandalaEmoji);
    let (buffer, _) = shaped_in_family(&mut emoji, &family, "\u{1F338}");
    let laid = buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter().cloned())
        .next()
        .expect("precondition: the emoji is laid out");
    let mut cache = SwashCache::new();
    assert!(
        glyph_outline_ink(&mut emoji, &mut cache, &laid).is_none(),
        "precondition: the emoji has no outline"
    );
    let before = draw_reads();
    let draws: Vec<bool> = [12.0, 512.0, 4096.0]
        .into_iter()
        .map(|size| {
            let mut at = laid.clone();
            at.font_size = size;
            glyph_draws(&mut emoji, &mut cache, &at)
        })
        .collect();
    let reads = draw_reads() - before;
    assert_eq!(draws, [true, true, true], "the emoji draws at every size");
    assert_eq!(reads, 1, "{reads} rasterizations for three sizes");
}

#[test]
fn test_whether_a_glyph_draws_is_asked_again_where_the_first_size_draws_nothing() {
    do_whether_a_glyph_draws_is_asked_again_where_the_first_size_draws_nothing();
}

/// Where the rasterizer draws nothing for a glyph at `DRAWS_SIZE`, it is
/// asked again at `DRAWS_SIZE_AGAIN` before the glyph is taken to draw
/// nothing (#333, review 5834654837), and nowhere else: a glyph drawn at
/// the first size is asked once; one drawn at no size twice, and draws
/// nothing; and one drawn only from 100 pixels per em up — a bitmap of
/// six pixels in a 109-pixel strike, which the rasterizer scales to
/// `⌊6 · 16 / 109⌋ = 0` pixels at 16 and to 14 at 256 — twice, and
/// draws. The rasterizer's answers here are the test's own: no compiled
/// or host face holds such a bitmap
/// (`test_whether_a_glyph_draws_is_the_rasterizers_answer_at_every_size`).
///
/// Control: the second ask dropped, red: "a glyph drawn at no size is
/// asked twice" (once), and with that assertion stepped past, the small
/// bitmap "draws: false".
pub fn do_whether_a_glyph_draws_is_asked_again_where_the_first_size_draws_nothing() {
    use crate::font::fonts::{draws_at, DRAWS_SIZE, DRAWS_SIZE_AGAIN};
    let ask = |drawn_from: f32| {
        let mut asked = Vec::new();
        let draws = draws_at(|size| {
            asked.push(size);
            size >= drawn_from
        });
        (draws, asked)
    };
    assert_eq!(
        ask(1.0),
        (true, vec![DRAWS_SIZE]),
        "a glyph drawn at 16 is asked once"
    );
    assert_eq!(
        ask(f32::INFINITY),
        (false, vec![DRAWS_SIZE, DRAWS_SIZE_AGAIN]),
        "a glyph drawn at no size is asked twice"
    );
    let (draws, asked) = ask(100.0);
    assert_eq!(
        asked,
        [DRAWS_SIZE, DRAWS_SIZE_AGAIN],
        "a small bitmap is asked again"
    );
    assert!(draws, "a bitmap too small for 16 pixels per em draws: {draws}");
}

#[test]
fn test_two_font_systems_never_answer_each_others_draws() {
    do_two_font_systems_never_answer_each_others_draws();
}

/// `glyph_draws`'s cache keeps each answer for its font system (#333,
/// review 5834654837), as the outline cache keeps an outline
/// (`do_two_font_systems_never_answer_each_others_outlines`): glyph 1 of
/// the first face of a font system holding MandalaEmoji alone is its
/// bitmap 🌸, which draws, and glyph 1 of the first face of one holding
/// Galatia SIL alone, the same face id, is its `.null`, which draws
/// nothing. Each is answered from its own font system, the first time
/// and from its entry the next. Preconditions: the two faces have the
/// same id, and neither glyph has an outline.
///
/// Control: the key without the font system's instance number, red on
/// the second answer ("glyph 1 of Galatia SIL draws nothing: true").
pub fn do_two_font_systems_never_answer_each_others_draws() {
    use crate::font::fonts::{draw_reads, face_glyph_draws};
    use cosmic_text::{CacheKey, CacheKeyFlags, SwashCache};
    let mut emoji = private_font_system(&[AppFont::MandalaEmoji]);
    let mut latin = private_font_system(&[AppFont::GalatiaSIL]);
    let first_face = |fs: &FontSystem| fs.db().faces().next().map(|face| face.id);
    let id = first_face(&emoji).expect("precondition: the emoji face is loaded");
    assert_eq!(
        Some(id),
        first_face(&latin),
        "precondition: the two font systems give their faces one id"
    );
    let mut cache = SwashCache::new();
    let weight = cosmic_text::fontdb::Weight::NORMAL;
    for fs in [&mut emoji, &mut latin] {
        let (key, _, _) = CacheKey::new(id, 1, 16.0, (0.0, 0.0), weight, CacheKeyFlags::empty());
        assert!(
            cache
                .get_outline_commands_uncached(fs, key)
                .is_none_or(|commands| commands.is_empty()),
            "precondition: glyph 1 has no outline"
        );
    }
    let glyph = (id, 1, weight, CacheKeyFlags::empty());
    let mut ask = |fs: &mut FontSystem| {
        let before = draw_reads();
        let draws = face_glyph_draws(fs, &mut cache, glyph);
        (draws, draw_reads() - before)
    };
    let first = ask(&mut emoji);
    let other = ask(&mut latin);
    let (first_again, other_again) = (ask(&mut emoji), ask(&mut latin));
    assert!(
        first.0 && first.1 > 0,
        "glyph 1 of MandalaEmoji, its 🌸, draws: {first:?}"
    );
    assert!(
        !other.0 && other.1 > 0,
        "glyph 1 of Galatia SIL draws nothing: {}",
        other.0
    );
    assert_eq!(
        (first_again, other_again),
        ((first.0, 0), (other.0, 0)),
        "each is answered from its own entry the next time"
    );
}

/// The random differential's fragments, by kind: Latin and its
/// ligatures, combining marks, Hebrew and Arabic with their marks,
/// Devanagari, Tamil, Malayalam, Khmer, Myanmar, Tibetan, Thai, Hangul
/// jamo and syllables, emoji sequences (ZWJ, VS16, a skin tone, a flag, a
/// tag sequence), private-use code points and noncharacters, a tab, bidi
/// controls, box drawing and ornaments.
pub const FRAGMENTS: [&str; 24] = [
    "Sand",
    "office ffl fi",
    "e\u{0301}x\u{0308}\u{0323}",
    "\u{05e9}\u{05b8}\u{05dc}\u{05d5}\u{05b9}\u{05dd}",
    "\u{0645}\u{064e}\u{0631}\u{0652}\u{062d}\u{064e}\u{0628}\u{064b}\u{0627}",
    "\u{0915}\u{094d}\u{0937}\u{093f}",
    "\u{0ba4}\u{0bae}\u{0bbf}\u{0bb4}\u{0bcd}",
    "\u{0d2e}\u{0d32}\u{0d2f}\u{0d3e}\u{0d33}\u{0d02}",
    "\u{1781}\u{17d2}\u{1798}\u{17c2}\u{179a}",
    "\u{1019}\u{103c}\u{1014}\u{103a}\u{1019}\u{102c}",
    "\u{0f56}\u{0f7c}\u{0f51}\u{0f0b}",
    "\u{0e20}\u{0e32}\u{0e29}\u{0e32}\u{0e44}\u{0e17}\u{0e22}",
    "\u{1100}\u{1161}\u{11a8}\u{ac00}\u{d7a3}",
    "\u{1f469}\u{200d}\u{1f4bb}",
    "\u{2764}\u{fe0f}\u{1f44d}\u{1f3fd}",
    "\u{1f1e9}\u{1f1ea}",
    "\u{1f3f4}\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}",
    "\u{e000}\u{f8ff}\u{10fffd}",
    "\u{ffff}\u{fdd0}",
    "a\tb",
    "\u{202e}abc\u{202c}\u{2067}\u{05d0}\u{2069}",
    "\u{2500}\u{2550}\u{2501}\u{253c}",
    "\u{2726} \u{00b7} \u{2766}",
    " ",
];

#[test]
fn test_a_seeded_random_differential_finds_no_merge_divergence() {
    do_a_seeded_random_differential_finds_no_merge_divergence();
}

/// A seeded random differential of the patched fallback (#304 review,
/// shape.rs:909): `CASES` texts of one to four [`FRAGMENTS`], each
/// sometimes repeated up to 30 times, each shaped in a face drawn from
/// every compiled face and the default family, at a weight from 100 to
/// 900, sometimes italic, are laid out glyph for glyph alike by the linear
/// merge with the early stop and by the published merge
/// (`set_reference_merge`). The corpus is held to have fallen back and to
/// have stopped early. The seed is fixed, so a red run replays.
pub fn do_a_seeded_random_differential_finds_no_merge_divergence() {
    const CASES: usize = 300;
    crate::font::fonts::init();
    let mut faces = crate::flow::tests::flow_fuzz::every_face();
    faces.push(AppFont::Any);
    let mut choices = crate::util::tests::choices::Choices::seeded(304);
    let (mut fell_back, mut stopped) = (0, 0);
    for case in 0..CASES {
        let mut text = String::new();
        for _ in 0..=choices.below(4) {
            let fragment = *choices.pick(&FRAGMENTS);
            let times = if choices.one_in(4) {
                1 + choices.below(30) as usize
            } else {
                1
            };
            text.push_str(&fragment.repeat(times));
        }
        let face = *choices.pick(&faces);
        let weight = 100 * (1 + choices.below(9) as u16);
        let italic = choices.one_in(4);
        let shaped = |reference: bool| {
            merging(reference, || {
                under_lock(|fs| {
                    let mut name = String::new();
                    let attrs = attrs_of(fs, face, &mut name)
                        .weight(cosmic_text::Weight(weight))
                        .style(if italic {
                            cosmic_text::Style::Italic
                        } else {
                            cosmic_text::Style::Normal
                        });
                    let mut buffer = Buffer::new_empty(Metrics::new(20.0, 24.0));
                    buffer.set_wrap(Wrap::None);
                    buffer.set_size(None, None);
                    let before = shape_work();
                    buffer.set_rich_text([(text.as_str(), attrs.clone())], &attrs, SHAPING_ADVANCED, None);
                    buffer.shape_until_scroll(fs, false);
                    (layout_of(&buffer), shape_work().since(before))
                })
            })
        };
        let (linear, work) = shaped(false);
        let (published, published_work) = shaped(true);
        assert_eq!(
            linear,
            published,
            "case {case} (seed 304): {face:?} at {weight}{} {text:?}: the patched merge laid out \
             glyphs the published merge did not",
            if italic { " italic" } else { "" }
        );
        fell_back += usize::from(published_work.fallback_rounds > 0);
        stopped += usize::from(work.fallback_rounds < published_work.fallback_rounds);
    }
    assert!(
        fell_back > CASES / 2 && stopped > 0,
        "precondition: the corpus falls back ({fell_back} of {CASES}) and stops early ({stopped})"
    );
}

#[test]
fn test_the_shape_work_bodies_repeat() {
    // Every `do_*` body here is a bench body too (TEST_CONVENTIONS §T6),
    // which a bench calls over and over in one process, so each must hold
    // on a later call as on its first. What this loop shows is that a
    // body whose first call leaves an answer kept, or any other state its
    // assertions read, fails on its second call here. (The asked-once
    // body once drew fresh code points from a pool that ran out at call
    // 1,920, #304 review; it builds its own font system now. A few calls
    // cannot show a pool that size is not being drawn from: keeping no
    // state between calls is what makes it so.)
    for _ in 0..50 {
        do_a_code_point_no_face_draws_is_asked_of_the_faces_once();
    }
    for _ in 0..20 {
        do_a_face_added_to_the_database_draws_what_no_face_drew();
    }
    for _ in 0..5 {
        do_a_hundred_contiguous_undrawn_code_points_cost_one_shaping();
    }
    for _ in 0..3 {
        do_a_word_its_face_lacks_costs_fallback_work_linear_in_its_length();
    }
    for _ in 0..2 {
        do_the_fallback_merge_lays_out_what_the_published_merge_did();
        do_a_seeded_random_differential_finds_no_merge_divergence();
    }
    for _ in 0..20 {
        do_two_font_systems_never_answer_each_others_outlines();
        do_a_run_the_merge_cannot_reason_about_is_merged_as_published();
        do_a_fallback_the_merge_cannot_reason_about_is_merged_as_published();
        do_the_early_stop_needs_every_missing_entry_matched();
        do_the_early_stop_leaves_a_cluster_of_several_code_points_to_the_faces();
    }
}

/// Whether a glyph with no outline draws, as `glyph_draws` answers it at
/// its first size and its second, is the rasterizer's answer at a spread
/// of sizes, 4, 16, 128 and 1,024 pixels per em — it draws at one of them
/// — for every glyph with no outline of every compiled face, and of every
/// face the host has (#333, review 5834654837): the premise
/// `DRAWS_SIZE` rests on, that whether a glyph draws does not turn on
/// its size where a reader sees it. Each face is read in a font system of
/// its own kind, so no shaping elsewhere waits on the shared font lock.
///
/// Where `MANDALA_REQUIRE_FONTCONFIG_HOSTS` is set (CI, the gate) the
/// host must have faces to read; elsewhere a host with none reads the
/// compiled faces alone, with a note.
///
/// Control: no face here holds a bitmap too small for 16 pixels per em
/// (the review's scan of Noto Color Emoji's 3,976 glyphs and
/// MandalaEmoji's found none), so the second ask's control is the
/// synthetic one,
/// `test_whether_a_glyph_draws_is_asked_again_where_the_first_size_draws_nothing`,
/// and the truncation arithmetic it states. Here: `glyph_draws`
/// answering `false` for every glyph with no outline, red on every bitmap
/// ("3943 of 9844: MandalaEmoji-Regular glyph 1: drawn first at Some(4.0)
/// pixels per em, glyph_draws false", and Noto Color Emoji's).
/// (No `do_*` twin: it reads every glyph of every face.)
#[test]
fn test_whether_a_glyph_draws_is_the_rasterizers_answer_at_every_size() {
    use crate::font::fonts::{compiled_font_system, face_glyph_draws, outline_bounds};
    use cosmic_text::{CacheKey, CacheKeyFlags, SwashCache};
    crate::font::fonts::init();
    let mut hosts = {
        let mut db = cosmic_text::fontdb::Database::new();
        db.load_system_fonts();
        crate::font::fallback::new_font_system(db)
    };
    let host_faces = hosts.db().faces().count();
    if host_faces == 0 {
        assert!(
            std::env::var_os("MANDALA_REQUIRE_FONTCONFIG_HOSTS").is_none(),
            "precondition: the host has faces to read"
        );
        crate::util::test_skip::note_skipped(
            "test_whether_a_glyph_draws_is_the_rasterizers_answer_at_every_size: no host faces, the compiled ones read alone",
        );
    }
    let weight = cosmic_text::fontdb::Weight::NORMAL;
    let flags = CacheKeyFlags::empty();
    let mut cache = SwashCache::new();
    let mut faults = Vec::new();
    let (mut read, mut bitmaps) = (0usize, 0usize);
    for fs in [&mut compiled_font_system(), &mut hosts] {
        let faces: Vec<(cosmic_text::fontdb::ID, String)> = fs
            .db()
            .faces()
            .map(|face| (face.id, face.post_script_name.clone()))
            .collect();
        for (id, name) in faces {
            let Some(font) = fs.get_font(id, weight) else {
                continue;
            };
            let glyphs = font.as_swash().metrics(&[]).glyph_count;
            for glyph in 0..glyphs {
                let key = |size: f32| CacheKey::new(id, glyph, size, (0.0, 0.0), weight, flags).0;
                let outline = cache
                    .get_outline_commands_uncached(fs, key(16.0))
                    .and_then(|commands| outline_bounds(&commands));
                if outline.is_some() {
                    continue;
                }
                read += 1;
                let answer = face_glyph_draws(fs, &mut cache, (id, glyph, weight, flags));
                let drawn_at = [4.0, 16.0, 128.0, 1024.0].into_iter().find(|&size| {
                    cache
                        .get_image_uncached(fs, key(size))
                        .is_some_and(|image| image.placement.width > 0 && image.placement.height > 0)
                });
                bitmaps += usize::from(drawn_at.is_some());
                if answer != drawn_at.is_some() {
                    faults.push(format!(
                        "{name} glyph {glyph}: drawn first at {drawn_at:?} pixels per em, glyph_draws {answer}"
                    ));
                }
            }
        }
    }
    assert!(
        read > 0 && bitmaps > 0,
        "precondition: glyphs with no outline are read, some of them drawn ({read}, {bitmaps})"
    );
    assert!(
        faults.is_empty(),
        "{} of {read}:\n{}",
        faults.len(),
        faults.join("\n")
    );
}
