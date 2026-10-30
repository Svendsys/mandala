// SPDX-License-Identifier: MPL-2.0

//! The readable rule (#291, rulings R25 and R25.1): every stretch of a
//! path flow reads left to right or top to bottom. Runs here are shaped
//! for real (the subject is real text on real outlines); the expected
//! reading direction is a property of the positions — box centers
//! moving the way the text is read — never a position the placement
//! computed. Where a test needs to know which stretch a glyph sits in,
//! it projects the glyph's center onto a dense sampling of the path,
//! which shares nothing with the walk but the path itself.

use glam::Vec2;

use crate::document::values::{Direction, Edge, FitMode, Percent, Shape, StartPoint};
use crate::flow::path::ArcPath;
use crate::flow::place::{
    place_on_closed_traced, place_on_open_traced, table_spacing, Trace, Travel, VERTICAL_EPSILON,
};
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{outline, Curve, Outline, PlacedRun, Rect, ShapedRun};

/// A line of verse, long enough to go around every fixture here.
const VERSE: &str = "To see a World in a Grain of Sand, And a Heaven in a Wild Flower";

/// Plain words: the prototype matrix's other text.
const POEM: &str = "the poem goes around the node exactly once";

fn rounded(w: f32, h: f32, percent: u8) -> Outline {
    outline(
        Rect::new(Vec2::ZERO, Vec2::new(w, h)),
        Shape::Rectangle {
            radius: Percent::new(percent),
        },
    )
}

fn ellipse(w: f32, h: f32) -> Outline {
    outline(Rect::new(Vec2::ZERO, Vec2::new(w, h)), Shape::Ellipse)
}

fn start(edge: Edge, at: f32) -> StartPoint {
    StartPoint { edge, at }
}

/// An S-curved connector: rightward, back leftward through its middle,
/// rightward again.
fn s_curve() -> Curve {
    Curve::Cubic(
        Vec2::ZERO,
        Vec2::new(800.0, 0.0),
        Vec2::new(-200.0, 400.0),
        Vec2::new(600.0, 400.0),
    )
}

/// A placement, the run it refers to, its trace, and the path it was
/// walked on (built as the placement builds it, for projection).
struct Placed {
    run: ShapedRun,
    placed: PlacedRun,
    trace: Trace,
    path: ArcPath,
}

/// `text` at 12 pt around `outline` from `from` in `direction`.
fn around(
    text: &str,
    outline: &Outline,
    from: StartPoint,
    direction: Direction,
    fit: &FitMode,
    outward: bool,
) -> Placed {
    let mut trace = Trace::default();
    let flow = style(12.0);
    let (run, rescaled, placed, spacing) = with_fonts(|fs| {
        let run = real_run(text, &flow, fs);
        let (rescaled, placed) = place_on_closed_traced(
            &run,
            outline,
            from,
            direction,
            fit,
            outward,
            &flow,
            fs,
            Some(&mut trace),
        );
        let spacing = table_spacing(&run, fit, &flow);
        (run, rescaled, placed, spacing)
    });
    Placed {
        run: rescaled.unwrap_or(run),
        placed,
        trace,
        path: ArcPath::closed(outline, from, direction, spacing),
    }
}

/// `text` at 12 pt along `curve` with `spacing`.
fn along(text: &str, curve: &Curve, fit: &FitMode, spacing: f32) -> Placed {
    let mut trace = Trace::default();
    let flow = style(12.0);
    let (run, rescaled, placed, table) = with_fonts(|fs| {
        let run = real_run(text, &flow, fs);
        let (rescaled, placed) = place_on_open_traced(&run, curve, fit, spacing, &flow, fs, Some(&mut trace));
        let table = table_spacing(&run, fit, &flow);
        (run, rescaled, placed, table)
    });
    Placed {
        run: rescaled.unwrap_or(run),
        placed,
        trace,
        path: ArcPath::open(curve, table),
    }
}

/// A grapheme cluster of a placement: its placed glyphs and its box.
struct ClusterBox {
    glyphs: std::ops::Range<usize>,
    corner: Vec2,
    size: Vec2,
}

impl ClusterBox {
    fn center(&self) -> Vec2 {
        self.corner + self.size * 0.5
    }
}

/// The clusters of `placed`, regrouped here as the renderer groups them:
/// a cluster continues while a glyph does not begin a grapheme and names
/// the shaped glyph after the previous one. Each box is its glyphs'
/// summed advance wide and one line high; its corner is its first
/// glyph's position less that glyph's pen distance from the cluster's
/// leftmost pen.
fn boxes(run: &ShapedRun, placed: &PlacedRun) -> Vec<ClusterBox> {
    let glyphs = &placed.glyphs;
    let metrics = |k: usize| &run.glyphs[glyphs[k].glyph as usize];
    let mut out: Vec<ClusterBox> = Vec::new();
    let mut i = 0;
    while i < glyphs.len() {
        let mut end = i + 1;
        while end < glyphs.len()
            && !metrics(end).grapheme_start
            && glyphs[end].glyph == glyphs[end - 1].glyph + 1
        {
            end += 1;
        }
        let left = (i..end).map(|k| metrics(k).x).fold(f32::INFINITY, f32::min);
        let width: f32 = (i..end).map(|k| metrics(k).advance).sum();
        out.push(ClusterBox {
            glyphs: i..end,
            corner: glyphs[i].pos - Vec2::new(metrics(i).x - left, 0.0),
            size: Vec2::new(width, run.line_height),
        });
        i = end;
    }
    out
}

/// The distance along `path` of the point nearest `p`, by a dense
/// sampling of the path every tenth of a point.
fn arc_position(path: &ArcPath, p: Vec2) -> f32 {
    let length = path.length();
    let samples = (length / 0.1).ceil() as usize;
    (0..=samples)
        .map(|k| length * k as f32 / samples as f32)
        .map(|s| (s, path.at(s).0.distance(p)))
        .fold(
            (0.0, f32::INFINITY),
            |best, next| if next.1 < best.1 { next } else { best },
        )
        .0
}

/// Which traced stretch placed glyph `index` belongs to.
fn stretch_of(trace: &Trace, index: usize) -> usize {
    trace
        .stretches
        .iter()
        .position(|stretch| stretch.glyphs.contains(&index))
        .expect("every walked glyph is in a stretch")
}

/// Every stretch reads forward: in text order, a leftward stretch's
/// cluster centers move right and a vertical stretch's move down. Says
/// how many consecutive pairs each kind held, for the caller's
/// preconditions.
fn assert_reads_forward(what: &str, placed: &Placed) -> (usize, usize) {
    let (mut leftward, mut vertical) = (0, 0);
    let clusters = boxes(&placed.run, &placed.placed);
    for (k, stretch) in placed.trace.stretches.iter().enumerate() {
        let inside: Vec<&ClusterBox> = clusters
            .iter()
            .filter(|cluster| stretch.glyphs.contains(&cluster.glyphs.start))
            .collect();
        for pair in inside.windows(2) {
            let (a, b) = (pair[0].center(), pair[1].center());
            match stretch.travel {
                Travel::Leftward => assert!(
                    b.x > a.x,
                    "{what}: leftward stretch {k} reads right to left: {a} then {b}"
                ),
                Travel::Downward | Travel::Upward => assert!(
                    b.y > a.y,
                    "{what}: {:?} stretch {k} reads bottom to top: {a} then {b}",
                    stretch.travel
                ),
                Travel::Rightward => {}
            }
        }
        let pairs = inside.len().saturating_sub(1);
        match stretch.travel {
            Travel::Leftward => leftward += pairs,
            Travel::Downward | Travel::Upward => vertical += pairs,
            Travel::Rightward => {}
        }
    }
    (leftward, vertical)
}

/// The readable glyphs are step 1's, in step 1's order, less at most a
/// suffix the zero-gap cut removed; and each stretch's glyphs lie in one
/// contiguous part of the path, the stretches in step 1's order. Returns
/// how many glyphs the cut removed.
fn assert_keeps_the_walk(what: &str, placed: &Placed) -> usize {
    let walked: Vec<u32> = placed.trace.walked.iter().map(|g| g.glyph).collect();
    let readable: Vec<u32> = placed.placed.glyphs.iter().map(|g| g.glyph).collect();
    assert!(
        readable.len() <= walked.len() && readable[..] == walked[..readable.len()],
        "{what}: the readable glyphs are not step 1's less a suffix"
    );
    let mut clusters: Vec<(f32, usize)> = boxes(&placed.run, &placed.placed)
        .iter()
        .map(|cluster| {
            let s = arc_position(&placed.path, cluster.center());
            (s, stretch_of(&placed.trace, cluster.glyphs.start))
        })
        .collect();
    clusters.sort_by(|a, b| a.0.total_cmp(&b.0));
    for pair in clusters.windows(2) {
        assert!(
            pair[0].1 <= pair[1].1,
            "{what}: stretch {} reaches past stretch {} along the path (at {} and {})",
            pair[0].1,
            pair[1].1,
            pair[0].0,
            pair[1].0
        );
    }
    walked.len() - readable.len()
}

/// No two cluster boxes of `placed` overlap by more than a hundredth of
/// a point. Every pair is checked: a placement that wrapped past a
/// loop's start would overlap clusters far apart in text order.
fn assert_no_overlap(what: &str, placed: &Placed) {
    let clusters = boxes(&placed.run, &placed.placed);
    for (i, a) in clusters.iter().enumerate() {
        for b in &clusters[i + 1..] {
            let overlap = (a.corner + a.size).min(b.corner + b.size) - a.corner.max(b.corner);
            assert!(
                overlap.x <= 0.01 || overlap.y <= 0.01,
                "{what}: clusters at {} and {} overlap by {overlap}",
                a.corner,
                b.corner
            );
        }
    }
}

/// The fixtures every reading test walks: a rounded rectangle and an
/// ellipse each way round from the middle of the top, and an S-curved
/// connector.
fn fixtures(fit: &FitMode, outward: bool) -> Vec<(String, Placed)> {
    let rect = rounded(300.0, 160.0, 25);
    let oval = ellipse(300.0, 160.0);
    let top = start(Edge::Top, 0.5);
    let mut out = Vec::new();
    for direction in [Direction::Clockwise, Direction::CounterClockwise] {
        out.push((
            format!("rounded rect {direction:?} {fit:?}"),
            around(VERSE, &rect, top, direction, fit, outward),
        ));
        out.push((
            format!("ellipse {direction:?} {fit:?}"),
            around(VERSE, &oval, top, direction, fit, outward),
        ));
    }
    out.push((format!("S-curve {fit:?}"), along(VERSE, &s_curve(), fit, 0.0)));
    out
}

#[test]
fn test_flow_readable_every_stretch_reads_forward() {
    do_flow_readable_every_stretch_reads_forward();
}

/// Around a rounded rectangle and an ellipse, each way round, and along
/// an S-curved connector, every leftward stretch reads left to right and
/// every vertical stretch top to bottom: in text order, box centers move
/// right, or down (#291 §1 step 3). Frames are pushed outward, as drawn.
pub fn do_flow_readable_every_stretch_reads_forward() {
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    for (what, placed) in fixtures(&fit, true) {
        let (leftward, vertical) = assert_reads_forward(&what, &placed);
        assert!(
            leftward >= 3,
            "{what}: the fixture needs a leftward stretch of several clusters"
        );
        if what.starts_with("rounded") {
            assert!(
                vertical >= 3,
                "{what}: the fixture needs a vertical stretch of several clusters"
            );
        }
    }
}

#[test]
fn test_flow_readable_a_loop_reads_like_a_seal() {
    do_flow_readable_a_loop_reads_like_a_seal();
}

/// A rounded rectangle read from the middle of its top side is a seal:
/// clockwise, the top runs rightward, the right side down, the bottom
/// leftward and the left side up — the last two placed reversed — and
/// back along the top; counter-clockwise, the same sides mirrored. With
/// `do_flow_readable_every_stretch_reads_forward`, every side reads left
/// to right or top to bottom.
pub fn do_flow_readable_a_loop_reads_like_a_seal() {
    use Travel::{Downward, Leftward, Rightward, Upward};
    let rect = rounded(300.0, 160.0, 25);
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    for (direction, expected) in [
        (
            Direction::Clockwise,
            [Rightward, Downward, Leftward, Upward, Rightward],
        ),
        (
            Direction::CounterClockwise,
            [Leftward, Downward, Rightward, Upward, Leftward],
        ),
    ] {
        let placed = around(VERSE, &rect, start(Edge::Top, 0.5), direction, &fit, true);
        let travels: Vec<Travel> = placed.trace.stretches.iter().map(|s| s.travel).collect();
        assert_eq!(travels, expected, "{direction:?}");
    }
}

#[test]
fn test_flow_readable_keeps_the_walks_glyphs_in_their_stretches() {
    do_flow_readable_keeps_the_walks_glyphs_in_their_stretches();
}

/// In every fit mode, the readable placement holds step 1's glyphs in
/// step 1's order, and each stretch's glyphs lie in one contiguous part
/// of the path, stretches in step 1's order (#291 §1 step 1, ruling
/// R25): the same glyphs per stretch as the unreversed walk. Glyph
/// centers sit on the path (no outward push), so each projects onto it
/// exactly. Only a mode that fills the path with copies — `Repeat`,
/// `Once` with a filler — may lose a suffix to the zero-gap cut; the
/// rest have slack, or are `Fit`, which never cuts above `min`.
pub fn do_flow_readable_keeps_the_walks_glyphs_in_their_stretches() {
    let modes = [
        (FitMode::Once { filler: None }, false),
        (
            FitMode::Once {
                filler: Some("·".to_owned()),
            },
            true,
        ),
        (FitMode::Repeat, true),
        (FitMode::Spread, false),
        (FitMode::Fit { min: 0.5, max: 2.0 }, false),
    ];
    for (fit, fills) in &modes {
        for (what, placed) in fixtures(fit, false) {
            let reversed = placed
                .trace
                .stretches
                .iter()
                .filter(|s| s.travel.is_backward() && s.glyphs.len() > 1)
                .count();
            assert!(reversed > 0, "{what}: the fixture needs a reversed stretch");
            let cut = assert_keeps_the_walk(&what, &placed);
            if !fills {
                assert_eq!(cut, 0, "{what}: cut");
            }
            assert!(!placed.placed.truncated, "{what}: truncated");
        }
    }
}

#[test]
fn test_flow_readable_hysteresis_keeps_a_loop_to_two_stretches() {
    do_flow_readable_hysteresis_keeps_a_loop_to_two_stretches();
}

/// A loop with no straight vertical side — a circle, and an ellipse a
/// hundred times wider than tall — reads as two stretches: its sides
/// turn through vertical too briefly to be stretches of their own and
/// join the top and the bottom (#291 §1 step 2, the hysteresis). Each
/// is started at its rightmost point, so the seam splits no stretch.
///
/// The circle turns slowly enough that several of its steps land within
/// ε of plumb: without the hysteresis each side is a vertical stretch.
/// The flat ellipse turns through 180° at each tip within about one
/// step, so no step there is within ε of plumb; what it holds is that a
/// step turned well toward vertical (`|cos θ| < 0.6`) at a tip still
/// makes no stretch of its own.
pub fn do_flow_readable_hysteresis_keeps_a_loop_to_two_stretches() {
    let right = start(Edge::Right, 0.5);
    let steepest = |placed: &Placed| {
        placed
            .trace
            .chords
            .iter()
            .map(|dir| dir.x.abs())
            .fold(f32::INFINITY, f32::min)
    };
    let circle = around(
        VERSE,
        &ellipse(600.0, 600.0),
        right,
        Direction::Clockwise,
        &FitMode::Repeat,
        true,
    );
    let plumb = circle
        .trace
        .chords
        .iter()
        .filter(|dir| dir.x.abs() <= VERTICAL_EPSILON)
        .count();
    assert!(plumb >= 2, "circle: the fixture needs steps within ε of plumb");
    let flat = around(
        VERSE,
        &ellipse(4000.0, 40.0),
        right,
        Direction::Clockwise,
        &FitMode::Repeat,
        true,
    );
    assert!(
        steepest(&flat) < 0.6,
        "100:1 ellipse: the fixture needs a step turned toward vertical at a tip, not {}",
        steepest(&flat)
    );
    for (what, placed) in [("circle", circle), ("100:1 ellipse", flat)] {
        let travels: Vec<Travel> = placed.trace.stretches.iter().map(|s| s.travel).collect();
        assert_eq!(travels.len(), 2, "{what}: {travels:?}");
    }
}

/// The prototype matrix of ruling R25.1: four outlines, four start
/// points, each way round.
fn matrix() -> Vec<(String, Outline, StartPoint, Direction)> {
    let outlines = [
        ("rounded rect", rounded(300.0, 160.0, 25)),
        ("pill", rounded(300.0, 160.0, 100)),
        ("ellipse", ellipse(300.0, 160.0)),
        ("circle", ellipse(200.0, 200.0)),
    ];
    let starts = [
        start(Edge::Top, 0.0),
        start(Edge::Top, 0.5),
        start(Edge::Bottom, 0.2),
        start(Edge::Left, 0.7),
    ];
    let mut out = Vec::new();
    for (name, outline) in &outlines {
        for from in starts {
            for direction in [Direction::Clockwise, Direction::CounterClockwise] {
                let what = format!("{name} from {:?} {} {direction:?}", from.edge, from.at);
                out.push((what, outline.clone(), from, direction));
            }
        }
    }
    out
}

#[test]
fn test_flow_readable_fit_keeps_the_poem_whole() {
    do_flow_readable_fit_keeps_the_poem_whole();
}

/// `Fit` keeps every glyph and never overlaps, however its text is
/// reversed (ruling R25.1): over the prototype matrix — four outlines,
/// four start points, each way round — and two texts, an unclamped `Fit`
/// places the whole text, untruncated, and no two cluster boxes overlap.
/// Reversing a stretch changes how long its text is, so a scale chosen
/// for the text as walked can leave the text as read too long even at no
/// gap; `Fit`'s scale search measures the text as read.
pub fn do_flow_readable_fit_keeps_the_poem_whole() {
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    let mut reversed = 0;
    for (what, outline, from, direction) in matrix() {
        for text in [POEM, VERSE] {
            let what = format!("{what}, {text:?}");
            let placed = around(text, &outline, from, direction, &fit, true);
            reversed += placed
                .trace
                .stretches
                .iter()
                .filter(|s| s.travel.is_backward() && s.glyphs.len() > 1)
                .count();
            assert!(!placed.placed.truncated, "{what}: truncated");
            assert_eq!(placed.placed.glyphs.len(), placed.run.glyphs.len(), "{what}: cut");
            assert_no_overlap(&what, &placed);
        }
    }
    assert!(
        reversed >= 64,
        "the matrix needs reversed stretches, not {reversed}"
    );
}

#[test]
fn test_flow_readable_cuts_copies_rather_than_overlap() {
    do_flow_readable_cuts_copies_rather_than_overlap();
}

/// With no slack — a `Repeat` or a `Once` filler fills its loop with no
/// gap, and the text read in order is longer than the text walked — the
/// spacing stays at zero and the placement is cut rather than overlapped
/// (ruling R25): a suffix of step 1's glyphs, whole repeat clusters or
/// whole filler copies (`~:~` is three), and the run is not truncated,
/// since no text was cut. No two cluster boxes overlap.
pub fn do_flow_readable_cuts_copies_rather_than_overlap() {
    let rect = rounded(300.0, 160.0, 25);
    let filler = FitMode::Once {
        filler: Some("~:~".to_owned()),
    };
    for (what, from, direction, fit, copy) in [
        (
            "Repeat",
            start(Edge::Bottom, 0.2),
            Direction::Clockwise,
            FitMode::Repeat,
            1,
        ),
        (
            "Once with a filler",
            start(Edge::Top, 0.5),
            Direction::CounterClockwise,
            filler,
            3,
        ),
    ] {
        let placed = around(POEM, &rect, from, direction, &fit, true);
        assert_no_overlap(what, &placed);
        let cut = assert_keeps_the_walk(what, &placed);
        assert!(cut > 0, "{what}: the fixture needs a placement with no slack");
        assert_eq!(cut % copy, 0, "{what}: cut {cut}, not whole copies of {copy}");
        assert!(
            !placed.placed.truncated,
            "{what}: dropping copies is no truncation"
        );
    }
}

#[test]
fn test_flow_readable_a_short_side_reads_top_to_bottom() {
    do_flow_readable_a_short_side_reads_top_to_bottom();
}

/// A straight side too short to be a stretch of its own — 40 points, under
/// three line heights of 14.4 — joins the stretch that reads it top to
/// bottom (#291 §1 step 2): its steps have no horizontal motion at all,
/// so going down it joins the rightward stretch (placed as walked) and
/// going up the leftward one (placed reversed). On a 300 × 50 rectangle
/// each way round, consecutive clusters on either side, in one stretch,
/// move down in text order. Glyph centers sit on the outline.
pub fn do_flow_readable_a_short_side_reads_top_to_bottom() {
    let rect = rounded(300.0, 50.0, 20);
    for direction in [Direction::Clockwise, Direction::CounterClockwise] {
        let placed = around(
            VERSE,
            &rect,
            start(Edge::Top, 0.5),
            direction,
            &FitMode::Repeat,
            false,
        );
        assert!(
            placed
                .trace
                .stretches
                .iter()
                .all(|s| matches!(s.travel, Travel::Rightward | Travel::Leftward)),
            "{direction:?}: the fixture needs sides too short to be stretches"
        );
        let on_side = |c: &ClusterBox| {
            let p = c.center();
            (p.x.abs() < 1e-3 || (p.x - 300.0).abs() < 1e-3) && p.y > 5.0 && p.y < 45.0
        };
        let clusters = boxes(&placed.run, &placed.placed);
        let mut pairs = [0, 0];
        for pair in clusters.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let same = stretch_of(&placed.trace, a.glyphs.start) == stretch_of(&placed.trace, b.glyphs.start);
            if on_side(a) && on_side(b) && same && (a.center().x - b.center().x).abs() < 1e-3 {
                pairs[usize::from(a.center().x > 150.0)] += 1;
                assert!(
                    b.center().y > a.center().y,
                    "{direction:?}: a short side reads bottom to top: {} then {}",
                    a.center(),
                    b.center()
                );
            }
        }
        assert!(
            pairs[0] > 0 && pairs[1] > 0,
            "{direction:?}: the fixture needs clusters stacked on both sides, not {pairs:?}"
        );
    }
}
