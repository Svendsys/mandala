// SPDX-License-Identifier: MPL-2.0

//! The classes of height a frame's touch checks can hold once its inks
//! are placed where the frame lies, and what is held back for them
//! (#371, rulings B46.1, B46.2). Every test here reads the engine's
//! counts against numbers worked out by hand or measured in the test's
//! own `f32` arithmetic, so none has a `do_*` twin.

use glam::Vec2;

use crate::flow::place::halves::{next_offset, placement_error, reachable_classes, solve_offset};
use crate::flow::place::INK_TOLERANCE;

/// The class of an extent as the touch check strips it, worked out here
/// apart from the engine (`strip_class`): the least power of two at
/// least the extent and [`INK_TOLERANCE`] together, a part in 2²⁰ more.
fn class_of(extent: f32) -> i32 {
    let tall = (f64::from(extent) + f64::from(INK_TOLERANCE)) * (1.0 + 1.0 / f64::from(1_u32 << 20));
    let mut class = -64;
    while 2.0_f64.powi(class) < tall {
        class += 1;
    }
    class
}

/// Ink extents across the font-size window, as shaped: every power of
/// two from 2⁻¹² to 2¹², each a hair either side, and the tallest an
/// ink of the window may be, less than 8,192.
fn window() -> Vec<f32> {
    let mut sizes = Vec::new();
    for k in -12..=12 {
        let at = 2.0_f32.powi(k);
        sizes.extend([at * 0.999, at, at * 1.001]);
    }
    sizes.push(8_191.999);
    sizes
}

/// The window's extents as square inks placed from coordinates no
/// further than `reach` from the origin, counted by the engine.
fn window_classes(reach: f64) -> usize {
    reachable_classes(window().into_iter().map(|s| (Vec2::splat(s), Some(reach))))
}

/// The ceiling documented on `touch_looks` (ruling B46.1): an ink of the
/// font-size window, placed from coordinates no further than `M` from the
/// origin, is less than `8,192 + (M + 8,192) × 2⁻²²` tall, so a check
/// holds the classes from −9 to that height's — 24 classes while it stays
/// within class 14, to `M` about 3.4 × 10¹⁰, and one more for each
/// doubling of `M` past it; and with `M` not finite, every class an `f32`
/// extent has, 140. Worked out here: at 2³⁶ an ink reaches 24,576 tall,
/// class 15; at 2³⁷, 40,960, class 16. Precondition: as placed, the
/// window's extents hold the classes −9 to 14 (by this file's own
/// `class_of`).
///
/// Control: with the reach ignored — each extent's own class, as R51.2
/// counted a side's ornament until B46.1 — red: 24 classes at 2³⁶.
#[test]
fn test_flow_halves_the_class_ceiling_grows_with_where_a_frame_lies() {
    let placed: std::collections::BTreeSet<i32> = window().into_iter().map(class_of).collect();
    assert_eq!(
        (placed.first().copied(), placed.last().copied(), placed.len()),
        (Some(-9), Some(14), 24),
        "precondition: the window's extents hold the classes −9 to 14"
    );
    assert_eq!(
        reachable_classes(window().into_iter().map(|s| (Vec2::splat(s), None))),
        24,
        "as placed, the window's extents are of 24 classes"
    );
    for (reach, classes) in [
        (0.0, 24),
        (1.0e3, 24),
        (1.0e6, 24),
        (3.4e10, 24),
        (2.0_f64.powi(36), 25),
        (2.0_f64.powi(37), 26),
        (f64::INFINITY, 140),
        (f64::NAN, 140),
    ] {
        assert_eq!(
            window_classes(reach),
            classes,
            "the window placed no further than {reach:e} from the origin reaches {classes} classes"
        );
    }
}

/// `placement_error` bounds what placing an ink rounds its extent by
/// (ruling B46.1): an extent `s` placed from a point `p` as
/// `(p + v₀, p + v₁)` and measured `(p + v₁) − (p + v₀)`, in `f32` as the
/// engine places and measures it, lies within the error of `s` for every
/// point, extent and corner here — 2,000 of them from a xorshift, at
/// magnitudes from 1 to 10³⁰, and the finding's own (#371): the
/// combining circumflex's ink, 0.984375 tall, placed 10⁶ from the origin
/// at 0.9375 or 1.0 by where — a corner half an `f32` off one rounds to
/// the even one — of classes 0 and 1, where its shaped extent's is 0.
/// And where the placements of one extent take two classes, the engine
/// counts both.
///
/// Control: with the error none, red (the finding's extent placed 1.0
/// tall, 0.015625 off); with it a quarter of what it is, red (an extent
/// of 17.049398 placed from −79,491.664 measures 17.054688, 5.3 × 10⁻³
/// off, past the quarter's 4.7 × 10⁻³).
#[test]
fn test_flow_halves_a_placed_extent_lies_within_its_error() {
    // The finding's glyph, 0.984375 tall, placed at 64 pens 10⁶ from the
    // origin, a fraction of a unit apart. Its ink's top lies half a
    // sixteenth off one: a tie, which rounds to the even sixteenth, by
    // the pen; its bottom a sixty-fourth past one, which rounds to it.
    let found: Vec<(f32, f32, f32)> = (0..64)
        .map(|k| (1.0e6 + k as f32 * 0.37, -14.468_75, 0.984_375))
        .collect();
    let mut cases = found.clone();
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..2_000 {
        let bits = next();
        let magnitude = 10.0_f32.powi((bits % 31) as i32);
        let unit = |b: u64| (b % 1_000_003) as f32 / 1_000_003.0;
        let p = (unit(bits >> 8) - 0.5) * 2.0 * magnitude;
        let low = (unit(bits >> 24) - 0.5) * 40.0;
        let size = unit(bits >> 40) * 30.0;
        cases.push((p, low, size));
    }
    let mut worst = 0.0_f64;
    for (p, low, size) in cases {
        let high = low + size;
        let shaped = high - low;
        let placed = (p + high) - (p + low);
        let reach = (f64::from(p) + f64::from(low))
            .abs()
            .max((f64::from(p) + f64::from(high)).abs());
        let off = (f64::from(placed) - f64::from(shaped)).abs();
        let error = placement_error(reach, shaped);
        worst = worst.max(off / (reach + f64::from(shaped)).max(f64::MIN_POSITIVE));
        assert!(
            off <= error,
            "an extent of {shaped} placed from {p} measures {placed}, {off:e} off, past its error of {error:e}"
        );
    }
    assert!(
        worst > 2.0_f64.powi(-25),
        "precondition: some placement rounds its extent by more than 2⁻²⁵ of its reach ({worst:e})"
    );
    // The finding's glyph: one extent as shaped, two classes as placed,
    // and the engine counts both.
    let classes: std::collections::BTreeSet<i32> = found
        .iter()
        .map(|&(p, low, size)| class_of((p + (low + size)) - (p + low)))
        .collect();
    assert_eq!(
        classes.into_iter().collect::<Vec<_>>(),
        [0, 1],
        "precondition: placed 10⁶ from the origin, the finding's extent takes classes 0 and 1"
    );
    assert_eq!(class_of(0.984_375), 0, "precondition: as shaped it is of class 0");
    let reach = 1.0e6 + 1.0;
    assert_eq!(
        reachable_classes([(Vec2::new(2.281_25, 0.984_375), Some(reach))]),
        2,
        "the engine counts both classes the finding's glyph takes placed"
    );
}

/// The offsets a frame's band curve is tried at are held to what its
/// inks can want (ruling B46.2, `next_offset`): no offset the inks want
/// lies past the bound, so no offset the solve settles at does either.
/// Two lays whose misses nearly agree — the inks wanting 1 at 0 and
/// 1.999 at 1, within a bound of 2 — send the secant to 1,000; held, it
/// tries 2, and the solve settles within the bound. Precondition: unheld,
/// the secant's step is 1,000.
///
/// Control: with `held_offset` holding nothing, red (an offset of 1,000
/// tried).
#[test]
fn test_flow_halves_an_offset_tried_is_held_to_what_the_inks_can_want() {
    let bound = 2.0_f32;
    // What the inks want, laid at `offset`: within the bound, whatever
    // the offset.
    let wants = |offset: f32| {
        if offset < 0.5 {
            1.0
        } else if offset < 1.5 {
            1.999
        } else {
            1.999_5
        }
    };
    let unheld = next_offset(
        1.0,
        wants(1.0) - 1.0,
        Some((0.0, wants(0.0))),
        wants(1.0),
        f32::INFINITY,
    );
    assert!(
        (unheld - 1_000.0).abs() < 1.0,
        "precondition: unheld, the secant steps to 1,000 ({unheld})"
    );
    assert_eq!(
        next_offset(1.0, wants(1.0) - 1.0, Some((0.0, wants(0.0))), wants(1.0), bound),
        bound,
        "held, the secant's step is the bound"
    );
    assert_eq!(
        next_offset(1.0, f32::NAN, None, -5.0, bound),
        -bound,
        "a fallback past the bound is held too"
    );
    let mut tried = Vec::new();
    let (settled, _) = solve_offset(
        0.0,
        bound,
        1e-2,
        8,
        |offset, _: Option<&f32>| {
            tried.push(offset);
            offset
        },
        |&offset| wants(offset),
        &|| false,
    );
    assert!(
        tried.iter().all(|o| o.abs() <= bound) && settled.abs() <= bound,
        "every offset tried, {tried:?}, and the one settled at, {settled}, within {bound}"
    );
    assert!(
        tried.len() >= 3,
        "precondition: the solve tries past its first two lays ({tried:?})"
    );
}

/// The classes of height of `sizes` across whichever axis holds more,
/// worked out here by [`class_of`].
fn classes_of(sizes: &[Vec2]) -> usize {
    [0, 1]
        .into_iter()
        .map(|axis| {
            sizes
                .iter()
                .map(|s| class_of(s[axis]))
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        })
        .max()
        .unwrap_or(0)
}

/// `text` at `points`, inked as a frame inks it: the ink sizes of its
/// glyphs, and of its clusters — each the box round its glyphs' inks,
/// each glyph's set its pen's distance into the cluster — worked out
/// here from the glyphs'.
fn inks_of(text: &str, points: f32) -> (Vec<Vec2>, Vec<Vec2>) {
    use crate::flow::place::halves::inked;
    use crate::flow::tests::support::{real_run, style, with_fonts};
    let run = with_fonts(|fs| {
        let run = real_run(text, &style(points), fs);
        inked(&run, fs.fonts())
    });
    let glyphs = run
        .glyphs
        .iter()
        .filter_map(|g| g.ink.map(|i| i.max - i.min))
        .collect();
    let mut clusters: Vec<Vec<&crate::flow::GlyphMetrics>> = Vec::new();
    for glyph in &run.glyphs {
        match clusters.last_mut() {
            Some(cluster) if !glyph.grapheme_start => cluster.push(glyph),
            _ => clusters.push(vec![glyph]),
        }
    }
    let clusters = clusters
        .iter()
        .filter_map(|cluster| {
            let left = cluster.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
            cluster
                .iter()
                .filter_map(|g| {
                    g.ink.map(|i| {
                        (
                            i.min + Vec2::new(g.x - left, 0.0),
                            i.max + Vec2::new(g.x - left, 0.0),
                        )
                    })
                })
                .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
                .map(|(min, max)| max - min)
        })
        .collect();
    (glyphs, clusters)
}

/// `text` at `points` laid outward as a `Spread` frame round a 400 × 200
/// rectangle of rounded corners whose corner is at `origin`: the worst
/// weighing of its lines for a touch, as `LINE_CLASSES` notes it — the
/// classes of their boxes as placed, and those `untouched` was held back
/// for.
fn lines_held(text: &str, points: f32, origin: f32) -> (usize, usize) {
    use crate::document::values::{FitMode, Percent, Shape};
    use crate::flow::place::halves::LINE_CLASSES;
    use crate::flow::tests::halves_tests::frame_with;
    use crate::flow::{outline, Rect, RingLayout};
    let shape = outline(
        Rect::new(Vec2::splat(origin), Vec2::new(400.0, 200.0)),
        Shape::Rectangle {
            radius: Percent::new(20),
        },
    );
    LINE_CLASSES.with(|most| most.set((0, usize::MAX)));
    frame_with(text, points, &shape, &FitMode::Spread, &RingLayout::whole(), true);
    LINE_CLASSES.with(std::cell::Cell::get)
}

/// What `untouched`'s share is held back for counts a line's inks by
/// cluster (ruling B46.2): each box a line lays is its cluster's ink, a
/// base and its combining marks in one, which can be of a class none of
/// its glyphs is. `o`, `o` with a diaeresis above and below, and a
/// hyphen, at 40 pt near the origin: the glyphs' inks are of two classes
/// (the `o`s', and the marks' and the hyphen's, each under 4 tall), the
/// clusters' of three, the marked `o` past 32 tall. Every weighing of the
/// frame's lines holds no more classes than were held back for
/// `untouched`. Precondition: by this file's own arithmetic, the glyphs'
/// two classes and the clusters' three, and the lines weighed.
///
/// Control: with the share held back for the glyphs' inks as shaped, as
/// until B46.2, red (3 classes weighed, 2 held back for).
#[test]
fn test_flow_halves_untouched_is_held_back_for_its_lines_inks_by_cluster() {
    let text = "o o\u{308}\u{324} -";
    let (glyphs, clusters) = inks_of(text, 40.0);
    assert_eq!(
        (classes_of(&glyphs), classes_of(&clusters)),
        (2, 3),
        "precondition: the glyphs' inks are of 2 classes, the clusters' of 3 ({glyphs:?}; {clusters:?})"
    );
    let (weighed, held) = lines_held(text, 40.0, 0.0);
    assert!(held != usize::MAX, "precondition: the frame's lines are weighed");
    assert!(
        weighed <= held,
        "a weighing of the frame's lines held {weighed} classes, past the {held} `untouched` was held back for"
    );
    assert_eq!(
        weighed, 3,
        "precondition: a weighing holds the clusters' 3 classes"
    );
}

/// What `untouched`'s share is held back for counts the classes its
/// lines' inks can take placed where the frame lies (ruling B46.2): `o`s
/// and `r`s, sized so that the `o`'s ink is a hair under the top of
/// class 4, 15.97 tall, laid round an outline 10⁶ from the origin, where
/// an `f32` is a sixteenth apart: a box is its ink's half size either
/// side of a center, 7.985, which rounds to 8 there, so every `o`'s box
/// is placed 16.0 tall, of class 5, and the `r`s' stay of class 4. Every
/// weighing of the frame's lines holds no more classes than were held
/// back for `untouched`. Preconditions: the `o`'s and the `r`'s inks as
/// shaped are of class 4 across their height and their width; at the
/// origin the lines' boxes are of that one class; 10⁶ from it, of two.
///
/// Control: with the share held back for the clusters' inks as shaped,
/// unwidened, red (2 classes weighed, 1 held back for).
#[test]
fn test_flow_halves_untouched_is_held_back_for_where_its_lines_lie() {
    let (glyphs, _) = inks_of("o", 40.0);
    let points = 40.0 * 15.97 / glyphs[0].y;
    let (sized, _) = inks_of("or", points);
    assert_eq!(
        classes_of(&sized),
        1,
        "precondition: the `o`'s and the `r`'s inks at {points} pt, {sized:?}, are of one class"
    );
    assert_eq!(class_of(sized[0].y), 4, "precondition: of class 4");
    let text = "or or or or or or or or";
    let (near, _) = lines_held(text, points, 0.0);
    assert_eq!(
        near, 1,
        "precondition: near the origin the lines' boxes are of one class"
    );
    let (weighed, held) = lines_held(text, points, 1.0e6);
    assert_eq!(
        weighed, 2,
        "precondition: 10⁶ from the origin some are placed a class taller"
    );
    assert!(
        weighed <= held,
        "a weighing of the frame's lines held {weighed} classes, past the {held} `untouched` was held back for"
    );
}

/// The flow fuzz properties of rulings R51.2 and B46.2: a frame whose
/// side arc's touch check held more classes of height than its arc held
/// back for fails, and so does one with a check of `untouched` that held
/// more than its share was held back for; each at what it was held back
/// for holds.
///
/// Control: with either property never failing, red (that one).
#[test]
fn test_the_flow_properties_find_a_check_past_the_classes_held_back_for_it() {
    use crate::flow::place::halves::{frame_work_steps, Looks};
    use crate::flow::place::Trace;
    use crate::flow::tests::flow_fuzz::InputWork;
    use crate::flow::tests::support::{real_run, style, with_fonts};
    let run = with_fonts(|fs| real_run("plain text", &style(16.0), fs));
    let frame = |looks: Looks| {
        let trace = Trace {
            spent: frame_work_steps(&run, None, false, 0, 0),
            looks,
            ..Trace::default()
        };
        InputWork::default().frame(&run, None, false, 0, 0, &trace, "a frame")
    };
    let side = |classes, reserved| Looks {
        classes,
        reserved,
        ..Looks::default()
    };
    let untouched = |held, reserved| Looks {
        untouched: (held, reserved),
        ..Looks::default()
    };
    assert!(
        frame(side(3, 3)).is_ok() && frame(untouched(3, 3)).is_ok(),
        "precondition: a frame whose checks hold what was held back for them holds"
    );
    let past_side = frame(side(4, 3));
    assert!(
        past_side
            .as_ref()
            .is_err_and(|why| why.contains("its arc held back for")),
        "a side arc's check past its arc's classes fails: {past_side:?}"
    );
    let past_untouched = frame(untouched(4, 3));
    assert!(
        past_untouched
            .as_ref()
            .is_err_and(|why| why.contains("its share was held back for")),
        "a check of `untouched` past its share's classes fails: {past_untouched:?}"
    );
}

/// The text the shrinking frames here lay unless one names its own.
const OMO: &str = "omo omo omo omo omo omo";

/// `text` at 22 pt laid inward as a `Fit` frame within ¼ to ×2 round a
/// sharp `w` × 12 rectangle, a frame that shrinks, with its budget set to
/// `budget` where given: the run as laid, its trace, how its builds held
/// `untouched`'s share, and the most its work may take, its budget and
/// its overshoot (`overshoot_steps` of its run as shaped).
fn shrinking_frame(
    text: &str,
    w: f32,
    budget: Option<usize>,
) -> (
    crate::flow::ShapedRun,
    crate::flow::place::Trace,
    crate::flow::place::halves::ShareTrace,
    usize,
) {
    use crate::document::values::{FitMode, Percent, Shape};
    use crate::flow::place::halves::{overshoot_steps, BUDGET_OVERRIDE, SHARE_CLASSES};
    use crate::flow::tests::halves_tests::{frame_with, with_seam};
    use crate::flow::tests::support::{real_run, style, with_fonts};
    use crate::flow::{outline, Rect, RingLayout};
    let shape = outline(
        Rect::new(Vec2::ZERO, Vec2::new(w, 12.0)),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    );
    let fit = FitMode::Fit { min: 0.25, max: 2.0 };
    let lay = || frame_with(text, 22.0, &shape, &fit, &RingLayout::whole(), false);
    let (run, _, trace) = match budget {
        Some(budget) => with_seam(&BUDGET_OVERRIDE, budget, lay),
        None => lay(),
    };
    let share = SHARE_CLASSES.with(std::cell::Cell::get);
    let shaped = with_fonts(|fs| real_run(text, &style(22.0), fs));
    let most = trace.budget + overshoot_steps(&shaped, None, true, trace.tables, 0);
    (run, trace, share, most)
}

/// `untouched`'s share covers every build of a frame that shrinks, the
/// one kept among them, whichever it is (ruling B46.3): held for no fewer
/// classes than any build before held it for, so a build whose inks take
/// fewer leaves it as it was. `"omo omo omo omo omo omo"` inward round a
/// sharp 290.04 × 12 rectangle at a budget of 5,000 glyph steps shrinks,
/// its last build laid taking one class fewer than the builds before it
/// and the probe after it refused; the share it ends with is held for the
/// most of them, and the frame ends within its budget and its overshoot.
/// Precondition: a build fell, and the frame kept a build of its shrink.
///
/// Control: with the share held for each build's classes alone, red (the
/// share ends held for 1 class, a build before it took 2).
#[test]
fn test_flow_halves_untouched_s_share_covers_every_build_of_a_shrink() {
    let (_, trace, share, most) = shrinking_frame(OMO, 290.04, Some(5_000));
    assert!(
        share.fell >= 1 && !share.kept_first,
        "precondition: the frame shrinks, a build taking fewer classes than one before it ({share:?})"
    );
    assert!(
        share.held >= share.most,
        "the share ends held for {}, a build of the frame took {} classes ({share:?})",
        share.held,
        share.most
    );
    assert!(
        trace.spent <= most,
        "the frame took {} glyph steps, past the {most} its budget and overshoot give",
        trace.spent
    );
}

/// A build past a frame's first whose share the budget its builds before
/// left no longer holds is refused (ruling B46.3, `Work::holds_share`):
/// `"omo omo omo omo omo omo"` inward round a sharp 348.2 × 12 rectangle
/// shrinks at its own budget; at a budget of 2,750 glyph steps its floor's
/// build holds the share as it was, 2 classes, but its own reshape and
/// inking take what the frame has spent past the budget less that share,
/// so it is refused, and the frame keeps its first build, at R27's size,
/// and ends within its budget and its overshoot. The ruling's condition
/// as it reads: the refusal fires whether or not the share grows, so a
/// frame whose budget binds keeps R27's size here where, unrefused, it
/// would keep the floor's — 11 of its 23 glyphs laid either way. (A share
/// that grows is the case below.) Precondition: at its own budget the
/// frame shrinks.
///
/// Control: with no build refused, red (the floor's build is laid, and
/// kept).
#[test]
fn test_flow_halves_a_shrink_s_build_the_budget_left_cannot_hold_the_share_for_is_refused() {
    let (free, _, free_share, _) = shrinking_frame(OMO, 348.2, None);
    assert!(
        !free_share.kept_first && free_share.refused == 0,
        "precondition: at its own budget the frame shrinks ({free_share:?})"
    );
    let (run, trace, share, most) = shrinking_frame(OMO, 348.2, Some(2_750));
    assert!(
        share.refused == 1 && share.kept_first,
        "at 2,750 glyph steps the floor's build is refused, and the frame keeps its first ({share:?})"
    );
    assert!(
        run.glyphs[0].size > free.glyphs[0].size,
        "kept at R27's size, {}, above the size it shrinks to, {}",
        run.glyphs[0].size,
        free.glyphs[0].size
    );
    assert!(
        trace.spent <= most,
        "the frame took {} glyph steps, past the {most} its budget and overshoot give",
        trace.spent
    );
}

/// A build past a frame's first whose inks would grow `untouched`'s share
/// past what the budget left is refused (ruling B46.3,
/// `Work::holds_share`): `"Hxg ö̤ Hxg ö̤ Hxg ö̤ Hxg"` inward round a sharp
/// 200 × 12 rectangle at a budget of 2,650 glyph steps. Its first build's
/// inks take 1 class, its floor's 2: the share would grow from 288 steps
/// to 354 with 2,326 spent, which 2,650 holds at 1 class and not at 2.
/// The floor's build is refused, and no build past the first is held for
/// with what the frame has spent and its share past its budget
/// (`ShareTrace::room_past`). Precondition: the first build's inks take 1
/// class.
///
/// Control: with the share the build would hold read without its classes
/// (`holds_share` reading the share as held), red (the floor's build held
/// for 2 classes, 32 steps past the budget).
#[test]
fn test_flow_halves_a_shrink_s_build_whose_share_would_grow_past_the_budget_is_refused() {
    let (_, trace, share, most) = shrinking_frame(
        "Hxg o\u{308}\u{324} Hxg o\u{308}\u{324} Hxg o\u{308}\u{324} Hxg",
        200.0,
        Some(2_650),
    );
    assert!(
        share.first == 1,
        "precondition: the first build's inks take 1 class ({share:?})"
    );
    assert!(
        share.room_past.is_none_or(|past| past <= 0),
        "a build past the first was held for {} steps past the budget ({share:?})",
        share.room_past.unwrap_or(0)
    );
    assert!(
        share.refused == 1 && share.kept_first,
        "the floor's build, its share grown past the budget, is refused ({share:?})"
    );
    assert!(
        trace.spent <= most,
        "the frame took {} glyph steps, past the {most} its budget and overshoot give",
        trace.spent
    );
}

/// The offset a frame's inks want lies within the bound every offset it
/// tries is held to (ruling B46.2's premise, held as B46.3 asks): over
/// every frame of the demo maps, and `... .. .` round a sharp 400 × 200
/// rectangle with `—|` on its sides, inward and outward, every offset
/// `Layout::wants` and the side relay read is no further from none than
/// the bound (`WANTS_PAST`). Precondition: each wanted an offset.
///
/// Control: with the bound read from the inks' half sizes alone, without
/// the rise of their centers from their anchors (`ink_reach`'s anchor and
/// center), red (the outward frame wants 0.039 past it). With it read
/// without the ornament's inks, or without `REACH_SLACK`, it stays green:
/// the bound is read from what bounds where an ink is placed as well as
/// what it wants, and its anchor's and its center's sideways terms leave
/// it from 3.5 pt to 19 pt above what any of the frames a sweep laid
/// wants (the demo maps' and 50 round this rectangle: five texts, five
/// ornaments, inward and outward), more than either term.
#[test]
fn test_flow_halves_every_offset_the_inks_want_lies_within_the_bound() {
    use crate::document::values::{FitMode, Percent, Rgba, Shape};
    use crate::flow::place::halves::WANTS_PAST;
    use crate::flow::tests::halves_tests::{demo_map_frames, frame_with};
    use crate::flow::{outline, Rect, RingLayout};
    WANTS_PAST.with(|most| most.set(f64::NAN));
    let frames = demo_map_frames();
    let demo = WANTS_PAST.with(std::cell::Cell::get);
    assert!(
        !frames.is_empty() && !demo.is_nan(),
        "precondition: the demo maps' frames want offsets"
    );
    assert!(
        demo <= 0.0,
        "a demo map's frame wanted an offset {demo} past its bound"
    );
    let shape = outline(
        Rect::new(Vec2::ZERO, Vec2::new(400.0, 200.0)),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    );
    let layout = RingLayout {
        sides: Some(("—|".to_string(), Rgba::opaque(0, 0, 0))),
        ..RingLayout::whole()
    };
    for outward in [false, true] {
        WANTS_PAST.with(|most| most.set(f64::NAN));
        frame_with("... .. .", 16.0, &shape, &FitMode::Spread, &layout, outward);
        let sided = WANTS_PAST.with(std::cell::Cell::get);
        assert!(
            !sided.is_nan(),
            "precondition: the side frame, outward {outward}, wants offsets"
        );
        assert!(
            sided <= 0.0,
            "the side frame, outward {outward}, wanted an offset {sided} past its bound"
        );
    }
}

/// `"omo omo"` at `points` laid inward round a sharp 400 × 200 rectangle
/// whose middle lies 2³⁰ from the origin on both axes — its inks placed
/// either side of 2³⁰, where an `f32` is 64 apart below and 128 above —
/// with combining circumflexes on its sides: its trace, and the worst
/// weighing of its lines (`LINE_CLASSES`).
fn far_frame(points: f32) -> (crate::flow::place::Trace, (usize, usize)) {
    use crate::document::values::{FitMode, Percent, Rgba, Shape};
    use crate::flow::place::halves::LINE_CLASSES;
    use crate::flow::tests::halves_tests::frame_with;
    use crate::flow::{outline, Rect, RingLayout};
    let middle = 2.0_f32.powi(30);
    let shape = outline(
        Rect::new(Vec2::new(middle - 200.0, middle - 100.0), Vec2::new(400.0, 200.0)),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    );
    let layout = RingLayout {
        sides: Some(("\u{302}".to_string(), Rgba::opaque(0, 0, 0))),
        ..RingLayout::whole()
    };
    LINE_CLASSES.with(|most| most.set((0, usize::MAX)));
    let (_, _, trace) = frame_with("omo omo", points, &shape, &FitMode::Spread, &layout, false);
    (trace, LINE_CLASSES.with(std::cell::Cell::get))
}

/// A frame some 10⁹ from the origin, inward round a sharp rectangle — its
/// band curve folded inside each corner — with marks on its sides, holds
/// both class properties (rulings B46.1, B46.2): no side arc's check holds
/// more classes than its arc held back for, no weighing of its lines more
/// than `untouched`'s share was held back for, and no check looks at more
/// than `touch_looks` a box. `"omo omo"` at 300 pt: its inks lie either
/// side of 2³⁰, where an `f32` is 64 apart below and 128 above, so its
/// lines' boxes are placed of two classes where their inks as shaped are
/// of one. Precondition: a weighing of its lines held two classes. (Its
/// side arcs' checks end in their sweeps, holding no class, so of the two
/// properties it is the lines' that this frame can break.)
///
/// Control: with the outline's reach left out of where its inks are
/// placed (`Halves::reach` none), red (2 classes weighed, 1 held back
/// for).
#[test]
fn test_flow_halves_a_frame_far_out_holds_both_class_properties() {
    let (trace, (weighed, held)) = far_frame(300.0);
    let looks = trace.looks;
    assert!(
        weighed == 2 && held != usize::MAX,
        "precondition: a weighing of the lines held two classes ({weighed})"
    );
    assert!(
        looks.classes <= looks.reserved,
        "a side arc's check held {} classes, past the {} its arc held back for",
        looks.classes,
        looks.reserved
    );
    assert!(
        weighed <= held,
        "a weighing of the lines held {weighed} classes, past the {held} `untouched` was held back for"
    );
    assert!(
        looks.untouched.0 <= looks.untouched.1 && looks.within(),
        "a check of `untouched` past its share, or one past `touch_looks`: {looks:?}"
    );
}
