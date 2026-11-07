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
/// ([`private_font_system`], so it repeats): the first shaping asks the
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
fn test_the_shape_work_bodies_repeat() {
    // Every `do_*` body here is a bench body too (TEST_CONVENTIONS §T6),
    // which a bench calls over and over in one process: each must hold
    // on its hundredth call as on its first. The asked-once body held for
    // 1,920 calls before it ran out of fresh code points (#304 review);
    // it builds its own font system now, and this calls it well past any
    // pool a body could be drawing from in a run this long.
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
    }
    for _ in 0..20 {
        do_a_run_the_merge_cannot_reason_about_is_merged_as_published();
        do_the_early_stop_needs_every_missing_entry_matched();
        do_the_early_stop_leaves_a_cluster_of_several_code_points_to_the_faces();
    }
}
