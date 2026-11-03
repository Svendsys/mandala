// SPDX-License-Identifier: MPL-2.0

//! A frame's work budget (#294, rulings R44.2, R46.1, R47.2): what runs
//! past it once it is spent, and the share held back for `untouched`.
//! Every test here sets a test seam or reads a test counter, so none has
//! a `do_*` twin.

use glam::Vec2;

use crate::document::values::{FitMode, LineSpan, Percent, Rgba, Shape};
use crate::flow::place::halves::{
    overshoot_steps, spent_seam, untouched_steps, Looks, BUDGET_OVERRIDE, PAST_BUDGET, UNTOUCHED_TAKEN,
};
use crate::flow::tests::flow_fuzz::ornament_run;
use crate::flow::tests::halves_tests::{
    frame_of_spans, frame_with, looks_as_measured, looks_ratios, mixed_sizes, spans_run, with_seam, QUATRAIN,
};
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{outline, Outline, Rect, RingLayout};

/// The text size every frame here is laid at.
const POINTS: f32 = 22.0;

/// The budgets the grid lays each frame with round a small outline: one
/// already spent when the frame starts, and two it spends partway.
const BUDGETS: [usize; 3] = [1, 2_000, 20_000];

/// The budgets round a large outline, whose side ornament's copies take
/// most of a frame's work: these spend it partway through laying them,
/// in a floor's row of copies or a solve of their gap.
const LARGE_BUDGETS: [usize; 7] = [1, 2_000, 20_000, 30_000, 40_000, 50_000, 60_000];

/// The grid's long side ornament: 40 bytes.
fn long_sides() -> String {
    "-=".repeat(20)
}

/// The large grid's long mixed side ornaments (ruling R50.2): the
/// reviewer's four rows (review 5745584645) — thin glyphs alone, and one
/// or two wide ones among them, whose touch checks looked at 10.7 to 28.9
/// pairs a box when every box shared the tallest's strip height — and
/// the fuzzable `"﷽|||"`, a wider glyph still.
fn mixed_sides() -> [String; 5] {
    [
        "|".repeat(40),
        format!("⸻{}", "|".repeat(40)),
        format!("⸻⸻{}", "|".repeat(60)),
        format!("⸻{}", "|".repeat(100)),
        "﷽|||".to_string(),
    ]
}

/// The budgets round the large sharp square with a long mixed side
/// ornament (ruling R50.2): spent before the frame's first arc, while it
/// is laid, and while the second is.
const MIXED_BUDGETS: [usize; 3] = [1, 60_000, 200_000];

/// The budgets round the large sharp square with the reviewer's 14-size
/// text five times over (ruling R51.2): spent while its lines are laid,
/// before its first arc, and while its arcs are.
const MIXED_SIZE_BUDGETS: [usize; 4] = [1, 100_000, 400_000, 700_000];

/// The budgets round the large sharp square with the widest of
/// [`mixed_sides`], the reviewer's `"⸻" + "|".repeat(100)` (ruling
/// R50.4): spent with an arc's copies laid and its touch check still to
/// run, the first arc's and then the second's — where, with the side
/// check's reserve removed, a take crosses the budget by 28,840 and
/// 22,042 glyph steps, past the 14,459 `overshoot_steps` allows.
const WIDEST_BUDGETS: [usize; 2] = [660_000, 780_000];

/// The outline of the grid's sharp rectangle: 600 × 12, where the
/// quatrain's two lines face each other inward and the last resorts run.
fn flat() -> Outline {
    outline(
        Rect::new(Vec2::ZERO, Vec2::new(600.0, 12.0)),
        Shape::Rectangle {
            radius: Percent::new(0),
        },
    )
}

/// What one frame of the grid did past its budget.
struct Past {
    what: String,
    /// The glyph steps it took past its budget ([`PAST_BUDGET`]): each
    /// take once its budget read as spent, and the part past it of a take
    /// that crossed it (ruling R50.4); and all it took.
    past: usize,
    spent: usize,
    /// What it may take past its budget (`overshoot_steps`), and its
    /// budget as the bound reads it: no less than `untouched`'s share.
    allowed: usize,
    held: usize,
    overlap: bool,
    /// The most glyph steps an `untouched` of it took, and its estimate.
    untouched: (usize, usize),
    /// The most pairs its touch checks looked at a box.
    looks: Looks,
}

/// The quatrain at [`POINTS`], as the grid's text: one span.
fn quatrain() -> Vec<(String, f32)> {
    vec![(QUATRAIN.to_string(), POINTS)]
}

/// `text`, its spans each at its size, laid round `shape` in every fit
/// mode, at each of `spans`, with each of `sides` in the style of
/// [`POINTS`], in both directions, at each of `budgets`: what each frame
/// took past its budget.
fn grid(
    shape: &Outline,
    name: &str,
    text: &[(String, f32)],
    spans: &[(&str, LineSpan)],
    sides: &[Option<String>],
    budgets: &[usize],
) -> Vec<Past> {
    let fits = [
        ("Fit", FitMode::Fit { min: 0.5, max: 2.0 }),
        ("Once", FitMode::Once { filler: None }),
        ("Spread", FitMode::Spread),
    ];
    let flow = style(POINTS);
    let mut out = Vec::new();
    for (fit_name, fit) in &fits {
        let sized = matches!(fit, FitMode::Fit { .. });
        for (span_name, span) in spans {
            for side in sides {
                let layout = RingLayout {
                    span: *span,
                    sides: side.clone().map(|s| (s, Rgba::opaque(0, 0, 0))),
                    ..RingLayout::whole()
                };
                let (run, ornament) =
                    with_fonts(|fs| (spans_run(text, fs), ornament_run(&layout, &flow, 1.0, fs)));
                let allowed = overshoot_steps(&run, ornament.as_ref(), sized);
                for outward in [true, false] {
                    for &budget in budgets {
                        PAST_BUDGET.with(|past| past.set(0));
                        UNTOUCHED_TAKEN.with(|most| most.set((0, 0)));
                        let (_, _, trace) = with_seam(&BUDGET_OVERRIDE, budget, || {
                            frame_of_spans(text, POINTS, shape, fit, &layout, outward)
                        });
                        let sides_name = side.as_ref().map_or(0, |s| s.len());
                        out.push(Past {
                            what: format!(
                                "{name}, {} spans of {} glyphs, {fit_name}, span {span_name}, {sides_name}-byte sides, {}, budget {budget}",
                                text.len(),
                                run.glyphs.len(),
                                if outward { "outward" } else { "inward" }
                            ),
                            past: PAST_BUDGET.with(std::cell::Cell::get),
                            spent: trace.spent,
                            allowed,
                            held: budget.max(untouched_steps(trace.glyphs)),
                            overlap: trace.unheld.overlap,
                            untouched: {
                                let (taken, inks) = UNTOUCHED_TAKEN.with(std::cell::Cell::get);
                                (taken, untouched_steps(inks))
                            },
                            looks: trace.looks,
                        });
                    }
                }
            }
        }
    }
    out
}

/// Which grid [`holds_the_bound`] lays.
#[derive(Clone, Copy)]
enum Size {
    /// Every span, with no sides, a short ornament and a long one, at
    /// each of [`BUDGETS`].
    Small,
    /// Span natural with the long ornament, at each of [`LARGE_BUDGETS`].
    Large,
    /// Span natural with each of [`mixed_sides`], at each of
    /// [`MIXED_BUDGETS`], and with the widest at each of
    /// [`WIDEST_BUDGETS`].
    Mixed,
    /// The reviewer's 14-size text ([`mixed_sizes`]) at span natural with
    /// `x` on its sides: once with 60 dots, at each of [`BUDGETS`], and
    /// five times over with 3,000 dots a span, at each of
    /// [`MIXED_SIZE_BUDGETS`].
    MixedSizes,
}

/// Hold every frame of the grid round `shape` to R47.2's bound: no more
/// than `overshoot_steps` taken past its budget — once it is spent, and
/// of a take that crosses it the part past it (R50.4) — no more in all
/// than its budget and that, no two inks touching, an `untouched` within
/// its share of the budget (R46.1), and no touch check looking at more
/// pairs a box than measured (R48.2, R50.2, R51.2: `looks_as_measured`), in the grid
/// `outline` names ([`Size`]).
fn holds_the_bound(shape: &Outline, name: &str, outline: Size) {
    let frames = match outline {
        Size::Large => grid(
            shape,
            name,
            &quatrain(),
            &[("natural", LineSpan::NATURAL)],
            &[Some(long_sides())],
            &LARGE_BUDGETS,
        ),
        Size::Mixed => {
            let mut frames = grid(
                shape,
                name,
                &quatrain(),
                &[("natural", LineSpan::NATURAL)],
                &mixed_sides().map(Some),
                &MIXED_BUDGETS,
            );
            frames.extend(grid(
                shape,
                name,
                &quatrain(),
                &[("natural", LineSpan::NATURAL)],
                &[Some(format!("⸻{}", "|".repeat(100)))],
                &WIDEST_BUDGETS,
            ));
            frames
        }
        Size::MixedSizes => {
            let sides = [Some("x".to_string())];
            let natural = [("natural", LineSpan::NATURAL)];
            let mut frames = grid(shape, name, &mixed_sizes(1, 60), &natural, &sides, &BUDGETS);
            frames.extend(grid(
                shape,
                name,
                &mixed_sizes(5, 3_000),
                &natural,
                &sides,
                &MIXED_SIZE_BUDGETS,
            ));
            frames
        }
        Size::Small => grid(
            shape,
            name,
            &quatrain(),
            &[
                ("whole", LineSpan::WHOLE),
                ("½", LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE)),
                ("natural", LineSpan::NATURAL),
            ],
            &[None, Some("❧".to_string()), Some(long_sides())],
            &BUDGETS,
        ),
    };
    let spent = frames.iter().filter(|f| f.past > 0).count();
    assert!(
        spent * 4 >= frames.len(),
        "precondition, {name}: a quarter of the frames or more work past a spent budget ({spent} of {})",
        frames.len()
    );
    let over: Vec<String> = frames
        .iter()
        .filter(|f| {
            f.past > f.allowed
                || f.spent > f.held + f.allowed
                || f.overlap
                || f.untouched.0 > f.untouched.1
                || looks_as_measured(&f.looks).is_err()
        })
        .map(|f| {
            let broken: Vec<&str> = [
                (f.past > f.allowed, "PAST_BUDGET"),
                (f.spent > f.held + f.allowed, "in all"),
                (f.overlap, "overlap"),
                (f.untouched.0 > f.untouched.1, "untouched"),
                (looks_as_measured(&f.looks).is_err(), "looks"),
            ]
            .into_iter()
            .filter_map(|(broken, what)| broken.then_some(what))
            .collect();
            format!(
                "{} [{}]: {} past the budget, {} in all, against {} past {} (overlap {}); `untouched` took {} of its {}; its touch checks: {}",
                f.what,
                broken.join(", "),
                f.past,
                f.spent,
                f.allowed,
                f.held,
                f.overlap,
                f.untouched.0,
                f.untouched.1,
                looks_ratios(&f.looks)
            )
        })
        .collect();
    assert!(
        over.is_empty(),
        "{} of {} frames past the bound:\n{}",
        over.len(),
        frames.len(),
        over.join("\n")
    );
}

/// R47.2 over a grid (review 5739829769): a frame reads its budget
/// before each build, reshape, lay and side copy, so once it reads as
/// spent the frame takes no more than `overshoot_steps` — one build's
/// shaping and inking, R27's reshape, and a lay of each line — and in
/// all no more than its budget and that, whatever its fit mode, span,
/// sides and direction: the quatrain round the 600 × 12 sharp rectangle.
///
/// Controls (each red in at least one outline's grid): the shrink's
/// floor build unread (round the three small outlines); the side fill's
/// floor row unread (round the two large ones); `falsi`'s own read
/// removed (round the 1,000 × 50 ellipse); `untouched_steps` halved
/// (round the 600 × 12 rectangle).
#[test]
fn test_flow_budget_a_frame_past_its_budget_takes_its_overshoot_at_most_flat() {
    holds_the_bound(&flat(), "600 × 12 sharp", Size::Small);
}

/// As above, round a 600 ellipse.
#[test]
fn test_flow_budget_a_frame_past_its_budget_takes_its_overshoot_at_most_round() {
    holds_the_bound(
        &outline(Rect::new(Vec2::ZERO, Vec2::splat(600.0)), Shape::Ellipse),
        "600 ellipse",
        Size::Small,
    );
}

/// As above, round a 50:1 ellipse, 1,000 × 20, where `Fit` frames shrink.
#[test]
fn test_flow_budget_a_frame_past_its_budget_takes_its_overshoot_at_most_thin() {
    holds_the_bound(
        &outline(Rect::new(Vec2::ZERO, Vec2::new(1_000.0, 20.0)), Shape::Ellipse),
        "1,000 × 20 ellipse",
        Size::Small,
    );
}

/// As above, round a 1,000 × 50 ellipse, where a `Fit` frame's solves
/// past a spent budget, `falsi`'s read removed, take it past the bound.
#[test]
fn test_flow_budget_a_frame_past_its_budget_takes_its_overshoot_at_most_wide() {
    holds_the_bound(
        &outline(Rect::new(Vec2::ZERO, Vec2::new(1_000.0, 50.0)), Shape::Ellipse),
        "1,000 × 50 ellipse",
        Size::Small,
    );
}

/// As above, round a 20,000 sharp square with the long ornament, whose
/// copies down its straight sides take most of the frame's work: budgets
/// spent while a side's floor row is laid, or its gap solved.
#[test]
fn test_flow_budget_a_frame_past_its_budget_takes_its_overshoot_at_most_large_square() {
    holds_the_bound(
        &outline(
            Rect::new(Vec2::ZERO, Vec2::splat(20_000.0)),
            Shape::Rectangle {
                radius: Percent::new(0),
            },
        ),
        "20,000 sharp square",
        Size::Large,
    );
}

/// As above, round a 20,000 ellipse.
#[test]
fn test_flow_budget_a_frame_past_its_budget_takes_its_overshoot_at_most_large_round() {
    holds_the_bound(
        &outline(Rect::new(Vec2::ZERO, Vec2::splat(20_000.0)), Shape::Ellipse),
        "20,000 ellipse",
        Size::Large,
    );
}

/// As above, round the 20,000 sharp square with each long mixed
/// ornament (ruling R50.2): the reviewer's four rows, whose touch checks
/// looked at up to 28.9 pairs a box when every box shared the tallest's
/// strip height — the widest ran 56,938 glyph steps past the bound — and
/// `"﷽|||"`.
///
/// Control: with the side check's reserve removed, `PAST_BUDGET` red.
#[test]
fn test_flow_budget_a_frame_past_its_budget_takes_its_overshoot_at_most_large_mixed() {
    holds_the_bound(
        &outline(
            Rect::new(Vec2::ZERO, Vec2::splat(20_000.0)),
            Shape::Rectangle {
                radius: Percent::new(0),
            },
        ),
        "20,000 sharp square",
        Size::Mixed,
    );
}

/// As above, round the 20,000 sharp square with the reviewer's 14-size
/// text (ruling R51.2, review 5751828253), once with 60 dots and five
/// times over with 3,000 dots a span, `x` on its sides: a touch check
/// of it holds a class of height for each size, and looks at a few pairs
/// a box for each.
///
/// Control: with the side check's reserve at R50's 18 looks a box
/// whatever the classes, red.
#[test]
fn test_flow_budget_a_frame_past_its_budget_takes_its_overshoot_at_most_large_mixed_sizes() {
    holds_the_bound(
        &outline(
            Rect::new(Vec2::ZERO, Vec2::splat(20_000.0)),
            Shape::Rectangle {
                radius: Percent::new(0),
            },
        ),
        "20,000 sharp square",
        Size::MixedSizes,
    );
}

/// R46.1: the share of a frame's budget held back for `untouched` is
/// held: a frame's work reads as spent once it has taken its budget less
/// `untouched_steps` of its glyphs, and not one step before.
///
/// Control: with no share held back, red.
#[test]
fn test_flow_budget_untouched_s_share_is_held_back() {
    let budget = 1_000_000;
    for glyphs in [1, 10, 1_000, 65_536] {
        let reserve = untouched_steps(glyphs);
        assert!(
            reserve > 0 && reserve < budget,
            "precondition: {glyphs} glyphs hold back {reserve}"
        );
        assert!(
            !spent_seam(glyphs, budget, budget - reserve - 1),
            "{glyphs} glyphs: spent a step before the budget less its share of {reserve}"
        );
        assert!(
            spent_seam(glyphs, budget, budget - reserve),
            "{glyphs} glyphs: not spent at the budget less its share of {reserve}"
        );
    }
}

/// R46.1: `untouched` takes no more than the share of the budget held
/// back for it, `untouched_steps` of the inks it is given: the quatrain
/// inward round the 600 × 12 sharp rectangle, `Once` at span whole with
/// a `❧` on its sides, whose budget of 20,000 is spent while its lines
/// still touch — where `untouched` takes the most of its share the grid
/// above found, 36 of 48 glyph steps.
///
/// Control: with `untouched_steps` halved, red.
#[test]
fn test_flow_budget_untouched_takes_its_share_at_most() {
    let layout = RingLayout {
        sides: Some(("❧".to_string(), Rgba::opaque(0, 0, 0))),
        ..RingLayout::whole()
    };
    UNTOUCHED_TAKEN.with(|most| most.set((0, 0)));
    with_seam(&BUDGET_OVERRIDE, 20_000, || {
        frame_with(
            QUATRAIN,
            POINTS,
            &flat(),
            &FitMode::Once { filler: None },
            &layout,
            false,
        )
    });
    let (taken, inks) = UNTOUCHED_TAKEN.with(std::cell::Cell::get);
    assert!(
        taken > 0 && inks > 0,
        "precondition: the frame reaches `untouched` ({taken} steps over {inks} inks)"
    );
    assert!(
        taken <= untouched_steps(inks),
        "`untouched` took {taken} glyph steps over {inks} inks, past its share of {}",
        untouched_steps(inks)
    );
}

/// R46.1: R27's fit reads the frame's budget before each reshape past
/// the first, so a frame whose budget is spent reshapes once: the
/// quatrain at 9 pt round a 200 × 100 ellipse, `Fit` within ½ to ×2,
/// whose first reshape lays a line past its span and is reshaped again
/// at its default budget.
///
/// Control: with that read removed, it reshapes twice at a budget of 1.
#[test]
fn test_flow_budget_r27_reshapes_once_past_a_spent_budget() {
    use crate::flow::place::halves::FIT_RESHAPES;
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(200.0, 100.0)), Shape::Ellipse);
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    let reshapes = |budget: Option<usize>| {
        FIT_RESHAPES.with(|count| count.set(0));
        let lay = || frame_with(QUATRAIN, 9.0, &shape, &fit, &RingLayout::whole(), true);
        match budget {
            Some(budget) => with_seam(&BUDGET_OVERRIDE, budget, lay),
            None => lay(),
        };
        FIT_RESHAPES.with(std::cell::Cell::get)
    };
    let free = reshapes(None);
    assert!(
        free >= 2,
        "precondition: R27 reshapes the text again at its default budget ({free} reshapes)"
    );
    let spent = reshapes(Some(1));
    assert_eq!(
        spent, 1,
        "R27 reshapes {spent} times at a budget of 1, once at most"
    );
}

/// R46.1: a `Once` filler's loops read the frame's budget before each
/// copy, so at a budget of 1 R27 places a handful of copies at most,
/// where at its own budget it fills its spans: `·` round a 20,000
/// ellipse.
///
/// Control: with both loops' reads removed, thousands at a budget of 1.
#[test]
fn test_flow_budget_a_filler_past_a_spent_budget_places_a_handful() {
    use crate::flow::place::place_lines;
    use crate::flow::tests::support::style;
    use crate::flow::FILLER_SPAN;
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::splat(20_000.0)), Shape::Ellipse);
    let flow = style(16.0);
    let copies = |budget: Option<usize>| {
        let lay = || {
            with_fonts(|fs| {
                let run = real_run("To see a World", &flow, fs);
                let fit = FitMode::Once {
                    filler: Some("·".to_string()),
                };
                let (filled, placed) = place_lines(&run, &shape, &fit, true, &flow, fs, None);
                let run = filled.unwrap_or(run);
                placed
                    .glyphs
                    .iter()
                    .filter(|g| run.glyphs[g.glyph as usize].span == FILLER_SPAN)
                    .count()
            })
        };
        match budget {
            Some(budget) => with_seam(&BUDGET_OVERRIDE, budget, lay),
            None => lay(),
        }
    };
    let free = copies(None);
    assert!(
        free > 1_000,
        "precondition: the filler fills its spans ({free} copies)"
    );
    let spent = copies(Some(1));
    assert!(
        spent <= 4,
        "{spent} filler copies placed at a budget of 1, a handful at most"
    );
}
