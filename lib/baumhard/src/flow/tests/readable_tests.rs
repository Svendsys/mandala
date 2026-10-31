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
    place_lines, place_on_closed_traced, place_on_open_traced, table_spacing, Trace, Travel, VERTICAL_EPSILON,
};
use crate::flow::tests::support::{real_run, style, with_fonts};
use crate::flow::{outline, Curve, Outline, PlacedRun, Rect, RingLayout, ShapedRun};

/// A line of verse, long enough to go around every fixture here.
const VERSE: &str = "To see a World in a Grain of Sand, And a Heaven in a Wild Flower";

/// Letters with no word break: one word, too long for a boundary to move.
const LETTERS: &str = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";

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
    s_curve_at(1.0)
}

/// That S-curve at `scale`.
fn s_curve_at(scale: f32) -> Curve {
    Curve::Cubic(
        Vec2::ZERO,
        Vec2::new(900.0, 0.0) * scale,
        Vec2::new(-300.0, 150.0) * scale,
        Vec2::new(600.0, 150.0) * scale,
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
    around_at(text, 12.0, outline, from, direction, fit, outward)
}

/// `text` at `points` around `outline` from `from` in `direction`.
fn around_at(
    text: &str,
    points: f32,
    outline: &Outline,
    from: StartPoint,
    direction: Direction,
    fit: &FitMode,
    outward: bool,
) -> Placed {
    let mut trace = Trace::default();
    let flow = style(points);
    let (run, rescaled, placed, spacing) = with_fonts(|fs| {
        let run = real_run(text, &flow, fs);
        // A ring walks the outline; a frame is R27's lines, which the
        // readable rule shapes (the halves above them: `halves_tests`).
        let (rescaled, placed) = match fit {
            FitMode::Repeat => place_on_closed_traced(
                &run,
                outline,
                from,
                direction,
                fit,
                &RingLayout::whole(),
                outward,
                &flow,
                fs,
                Some(&mut trace),
            ),
            _ => place_lines(&run, outline, fit, outward, &flow, fs, Some(&mut trace)),
        };
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

/// Every stretch reads forward — "left to right or top to bottom"
/// (#291): in text order, each cluster's center lies right of the one
/// before it, or below it. Text moving left without moving down, or up
/// without moving right, reads backward. A stretch keeps to its own way
/// along its own part of the path; where it runs on into a neighbor read
/// the same way (`PathWalk::borrow_room`), it reads that neighbor's
/// way, which is forward too — so the test holds the reading direction,
/// not one axis per stretch. Says how many consecutive pairs leftward
/// and vertical stretches held, for the caller's preconditions.
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
            assert!(
                b.x > a.x || b.y > a.y,
                "{what}: {:?} stretch {k} reads backward: {a} then {b}",
                stretch.travel
            );
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

/// No two clusters overlap along the path (ruling R25): the steps the
/// lay took, in order along the path, each start at or after the one
/// before ends, and the last ends within the path — on a loop, before it
/// would wrap onto the first — allowing `Fit`'s solver tolerance, 0.05
/// pt, the float noise of a reshape `Fit` lets a lay run past the path's
/// end by (ruling R9.3). Along the path is where a gap is a gap: the
/// walk's chord steps can let two upright boxes touch at a corner where
/// a curve turns, whatever order they are read in.
fn assert_no_overlap(what: &str, placed: &Placed) {
    let mut laid = placed.trace.laid.clone();
    assert!(!laid.is_empty(), "{what}: the trace needs the lay's steps");
    laid.sort_by(|a, b| a.0.total_cmp(&b.0));
    for pair in laid.windows(2) {
        let ((s, step), (next, _)) = (pair[0], pair[1]);
        assert!(
            next >= s + step - 1e-3,
            "{what}: a step at {next} starts inside the one at {s}, {step} long"
        );
    }
    let (s, step) = laid[laid.len() - 1];
    assert!(
        s + step <= placed.path.length() + 0.05,
        "{what}: the last step ends at {}, past the path's {}",
        s + step,
        placed.path.length()
    );
    assert!(
        laid[0].0 >= -1e-3,
        "{what}: the first step starts before the path"
    );
}

/// A U-turn: rightward, down, back leftward — at `scale` of 700 × 300.
fn u_turn(scale: f32) -> Curve {
    Curve::Cubic(
        Vec2::ZERO,
        Vec2::new(700.0 * scale, 0.0),
        Vec2::new(700.0 * scale, 300.0 * scale),
        Vec2::new(0.0, 300.0 * scale),
    )
}

/// A hook: rightward, down, and back past its start.
fn hook() -> Curve {
    hook_at(1.0)
}

/// That hook at `scale`.
fn hook_at(scale: f32) -> Curve {
    Curve::Cubic(
        Vec2::ZERO,
        Vec2::new(400.0, 0.0) * scale,
        Vec2::new(400.0, 250.0) * scale,
        Vec2::new(-50.0, 250.0) * scale,
    )
}

/// The fixtures every reading test walks: a `Repeat` ring — the one
/// kind of frame ring walked whole around its outline (ruling R27) — on
/// a rounded rectangle and an ellipse, each way round from the middle of
/// the top, and an S-curved connector placed with `fit`.
fn fixtures(fit: &FitMode, outward: bool) -> Vec<(String, Placed)> {
    let rect = rounded(300.0, 160.0, 25);
    let oval = ellipse(300.0, 160.0);
    let top = start(Edge::Top, 0.5);
    let ring = FitMode::Repeat;
    let mut out = Vec::new();
    for direction in [Direction::Clockwise, Direction::CounterClockwise] {
        out.push((
            format!("rounded rect ring {direction:?}"),
            around(VERSE, &rect, top, direction, &ring, outward),
        ));
        out.push((
            format!("ellipse ring {direction:?}"),
            around(VERSE, &oval, top, direction, &ring, outward),
        ));
    }
    out.push((format!("S-curve {fit:?}"), along(VERSE, &s_curve(), fit, 0.0)));
    out
}

#[test]
fn test_flow_readable_every_stretch_reads_forward() {
    do_flow_readable_every_stretch_reads_forward();
}

/// Around a rounded rectangle and an ellipse, each way round, a `Repeat`
/// ring, and along an S-curved connector, every stretch reads left to
/// right or top to bottom (#291 §1 step 3). Rings are pushed outward, as
/// drawn.
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

/// A `Repeat` ring read from the middle of its top side is a seal:
/// clockwise, the top runs rightward, the right side down, the bottom
/// leftward and the left side up — the last two placed reversed — and
/// back along the top; counter-clockwise, the same sides mirrored. So
/// reads a rounded rectangle, and so, with ε = cos 45° classing each
/// step by its dominant direction, do a circle and an ellipse (ruling
/// R26). With `do_flow_readable_every_stretch_reads_forward`, every side
/// reads left to right or top to bottom.
pub fn do_flow_readable_a_loop_reads_like_a_seal() {
    use Travel::{Downward, Leftward, Rightward, Upward};
    for (what, outline) in [
        ("rounded rect", rounded(300.0, 160.0, 25)),
        ("circle", ellipse(300.0, 300.0)),
        ("ellipse", ellipse(300.0, 160.0)),
    ] {
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
            let placed = around(
                VERSE,
                &outline,
                start(Edge::Top, 0.5),
                direction,
                &FitMode::Repeat,
                true,
            );
            let travels: Vec<Travel> = placed.trace.stretches.iter().map(|s| s.travel).collect();
            assert_eq!(travels, expected, "{what} {direction:?}");
        }
    }
}

#[test]
fn test_flow_readable_keeps_the_walks_glyphs_in_their_stretches() {
    do_flow_readable_keeps_the_walks_glyphs_in_their_stretches();
}

/// In every fit mode along a connector, and in a `Repeat` ring, the
/// readable placement holds step 1's glyphs in step 1's order, and each
/// stretch's glyphs lie in one contiguous part of the path, stretches in
/// step 1's order (#291 §1 step 1, ruling R25): the same glyphs per
/// stretch as the unreversed walk. Glyph centers sit on the path (no
/// outward push), so each projects onto it exactly. Only a placement
/// that fills its path with copies — a `Repeat` ring — may lose a suffix
/// to the zero-gap cut; the connector's modes all have room to spare, or
/// are `Fit`, which never cuts above `min`.
pub fn do_flow_readable_keeps_the_walks_glyphs_in_their_stretches() {
    let modes = [
        FitMode::Once { filler: None },
        FitMode::Once {
            filler: Some("·".to_owned()),
        },
        FitMode::Repeat,
        FitMode::Spread,
        FitMode::Fit { min: 0.5, max: 2.0 },
    ];
    for fit in &modes {
        for (what, placed) in fixtures(fit, false) {
            let reversed = placed
                .trace
                .stretches
                .iter()
                .filter(|s| s.travel.is_backward() && s.glyphs.len() > 1)
                .count();
            assert!(reversed > 0, "{what}: the fixture needs a reversed stretch");
            let cut = assert_keeps_the_walk(&what, &placed);
            let fills = what.contains("ring") || *fit == FitMode::Repeat;
            if !fills {
                assert_eq!(cut, 0, "{what}: cut");
            }
            assert!(!placed.placed.truncated, "{what}: truncated");
        }
    }
}

#[test]
fn test_flow_readable_hysteresis_keeps_a_flat_loop_to_two_stretches() {
    do_flow_readable_hysteresis_keeps_a_flat_loop_to_two_stretches();
}

/// An ellipse a hundred times wider than tall reads as two stretches,
/// its top and its bottom: each tip turns through vertical within about
/// a step, too briefly to be a stretch of its own (#291 §1 step 2, the
/// hysteresis). Started on its right tip, just above its rightmost
/// point, so the seam splits no stretch, and where the ring's even
/// spacing (ruling R29) puts a step at a tip within ε of plumb — checked
/// here — which without the hysteresis would be a vertical stretch of a
/// cluster or two.
pub fn do_flow_readable_hysteresis_keeps_a_flat_loop_to_two_stretches() {
    let flat = around(
        VERSE,
        &ellipse(4000.0, 40.0),
        start(Edge::Right, 0.45),
        Direction::Clockwise,
        &FitMode::Repeat,
        true,
    );
    let plumb = flat
        .trace
        .chords
        .iter()
        .filter(|dir| dir.x.abs() <= VERTICAL_EPSILON)
        .count();
    assert!(plumb > 0, "the fixture needs a step within ε of plumb at a tip");
    let travels: Vec<Travel> = flat.trace.stretches.iter().map(|s| s.travel).collect();
    assert_eq!(travels, [Travel::Leftward, Travel::Rightward]);
}

/// The connectors the R25.1 sweep walks: an S-curve, two U-turns and a
/// hook, each holding stretches read the other way.
fn connectors() -> Vec<(&'static str, Curve)> {
    vec![
        ("S-curve", s_curve()),
        ("U-turn", u_turn(1.0)),
        ("small U-turn", u_turn(0.7)),
        ("hook", hook()),
        ("small S-curve", s_curve_at(0.5)),
        ("small hook", hook_at(0.5)),
    ]
}

#[test]
fn test_flow_readable_fit_keeps_a_connectors_text_whole() {
    do_flow_readable_fit_keeps_a_connectors_text_whole();
}

/// `Fit` along a connector keeps every glyph, never overlaps and reads
/// forward, however its text is reversed (rulings R25.1, R26): over six
/// connectors and five texts, an unclamped `Fit` places the whole text,
/// untruncated, no two cluster boxes overlap, and every stretch reads
/// left to right or top to bottom. Reversing a stretch changes how long
/// its text is, and a word moved whole takes room, so a scale chosen for
/// the text as walked can leave the text as read too long, or crowding a
/// stretch into one read the other way; `Fit`'s scale search measures
/// the text as read.
pub fn do_flow_readable_fit_keeps_a_connectors_text_whole() {
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    let mut reversed = 0;
    for (name, curve) in connectors() {
        for text in [POEM, VERSE, ELLIPSE_LINE, BLAKE_INNER, ECCLESIASTES] {
            let what = format!("{name}, {text:?}");
            let placed = along(text, &curve, &fit, 0.0);
            reversed += placed
                .trace
                .stretches
                .iter()
                .filter(|s| s.travel.is_backward() && s.glyphs.len() > 1)
                .count();
            assert!(!placed.placed.truncated, "{what}: truncated");
            assert_eq!(placed.placed.glyphs.len(), placed.run.glyphs.len(), "{what}: cut");
            assert_no_overlap(&what, &placed);
            assert_reads_forward(&what, &placed);
        }
    }
    assert!(
        reversed >= 16,
        "the sweep needs reversed stretches, not {reversed}"
    );
}

#[test]
fn test_flow_readable_cuts_copies_rather_than_overlap() {
    do_flow_readable_cuts_copies_rather_than_overlap();
}

/// With no slack — a connector's `Repeat` or `Once` filler fills its
/// curve with no gap, and the text read in order is longer than the text
/// walked — the spacing stays at zero and the placement is cut rather
/// than overlapped (ruling R25): a suffix of step 1's glyphs, whole
/// repeat clusters or whole filler copies (`·•·` is three), and the run
/// is not truncated, since no text was cut. No two cluster boxes
/// overlap. (A closed ring is never cut: it holds whole copies, evenly
/// spaced, ruling R29.)
pub fn do_flow_readable_cuts_copies_rather_than_overlap() {
    let pattern = along(VERSE, &u_turn(1.1), &FitMode::Repeat, 0.0);
    let filler = FitMode::Once {
        filler: Some("·•·".to_owned()),
    };
    let curve = along(POEM, &u_turn(0.5), &filler, 0.0);
    for (what, placed, copy) in [("Repeat", pattern, 1), ("Once with a filler", curve, 3)] {
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

/// A straight side too short to be a stretch of its own — 28.8 points,
/// under three line heights of 14.4 — joins the stretch that reads it
/// top to bottom (#291 §1 step 2): its steps have no horizontal motion at
/// all, so going down it joins the rightward stretch (placed as walked)
/// and going up the leftward one (placed reversed). On a 300 × 32
/// rectangle walked clockwise — down its right side, up its left — each
/// side's clusters, read top to bottom, are consecutive in the text. The text has no word break, so no word is moved across a
/// boundary (ruling R26) to part the clusters on a side. Glyph centers
/// sit on the outline.
pub fn do_flow_readable_a_short_side_reads_top_to_bottom() {
    let rect = rounded(300.0, 32.0, 10);
    {
        let direction = Direction::Clockwise;
        let placed = around(
            LETTERS,
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
            (p.x.abs() < 1e-3 || (p.x - 300.0).abs() < 1e-3) && p.y > 1.6 && p.y < 30.4
        };
        let clusters = boxes(&placed.run, &placed.placed);
        for (side, x) in [("left", 0.0_f32), ("right", 300.0)] {
            // The side's clusters, in text order, and top to bottom.
            let on: Vec<(usize, f32)> = clusters
                .iter()
                .enumerate()
                .filter(|(_, c)| on_side(c) && (c.center().x - x).abs() < 1e-3)
                .map(|(k, c)| (k, c.center().y))
                .collect();
            assert!(
                on.len() >= 2,
                "{direction:?}: the fixture needs clusters stacked on the {side} side, not {}",
                on.len()
            );
            let mut down = on.clone();
            down.sort_by(|a, b| a.1.total_cmp(&b.1));
            let read: Vec<usize> = down.iter().map(|&(k, _)| k).collect();
            let consecutive = read.windows(2).all(|pair| pair[1] == pair[0] + 1);
            assert!(
                consecutive,
                "{direction:?}: the {side} side does not read top to bottom: clusters {read:?}, top to bottom"
            );
        }
    }
}

/// The placed glyphs (indices into the placement) whose cluster starts
/// inside `word`, the first occurrence of it in `text`.
fn word_glyphs(placed: &Placed, text: &str, word: &str) -> Vec<usize> {
    let start = text.find(word).expect("the word is in the text");
    let bytes = start..start + word.len();
    (0..placed.placed.glyphs.len())
        .filter(|&i| {
            bytes.contains(
                &placed.run.glyphs[placed.placed.glyphs[i].glyph as usize]
                    .bytes
                    .start,
            )
        })
        .collect()
}

/// How many of `stretches` the glyphs `glyphs` fall in.
fn stretches_holding(stretches: &[std::ops::Range<usize>], glyphs: &[usize]) -> usize {
    stretches
        .iter()
        .filter(|range| glyphs.iter().any(|g| range.contains(g)))
        .count()
}

/// Whether the walk tore `word` — its glyphs fell in two stretches
/// before any boundary moved — and how many stretches hold it as laid.
fn torn_and_laid(placed: &Placed, text: &str, word: &str) -> (bool, usize) {
    let glyphs = word_glyphs(placed, text, word);
    assert!(!glyphs.is_empty(), "{word:?}: the fixture needs the word placed");
    let laid: Vec<std::ops::Range<usize>> = placed.trace.stretches.iter().map(|s| s.glyphs.clone()).collect();
    (
        stretches_holding(&placed.trace.walked_stretches, &glyphs) > 1,
        stretches_holding(&laid, &glyphs),
    )
}

/// The README's frames scene: an ellipse of 230 × 190 at 20 pt.
const ELLIPSE_LINE: &str = "and an ellipse, every glyph upright, never rotated · ";

/// The README's Blake scene: its inner ring's text.
const BLAKE_INNER: &str = "The heavens declare the glory of God; and the firmament sheweth his handywork. ";

/// The README's Ecclesiastes scene: the frame around its center node.
const ECCLESIASTES: &str = "and a time to every purpose under the heaven: ";

#[test]
fn test_flow_readable_keeps_a_torn_word_whole() {
    do_flow_readable_keeps_a_torn_word_whole();
}

/// A word the walk tears across a stretch boundary of a connector is
/// laid whole in one stretch (ruling R26): "glyph" on an S-curve,
/// "Sand," — its comma with it — on a U-turn. Torn next to a reversed
/// stretch, a word would have its head at one end of the stretch and its
/// tail at the other. Each fixture is checked to tear its word in the
/// walk; the placement must still read forward.
pub fn do_flow_readable_keeps_a_torn_word_whole() {
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    for (what, text, curve, word) in [
        ("S-curve", ELLIPSE_LINE, s_curve(), "glyph"),
        ("U-turn", VERSE, u_turn(1.0), "Sand,"),
    ] {
        let placed = along(text, &curve, &fit, 0.0);
        let (torn, laid) = torn_and_laid(&placed, text, word);
        assert!(torn, "{what}: the fixture needs {word:?} torn by the walk");
        assert_eq!(laid, 1, "{what}: {word:?} is not whole in one stretch");
        assert_reads_forward(what, &placed);
        assert!(!placed.placed.truncated, "{what}: truncated");
    }
}

#[test]
fn test_flow_readable_keeps_a_flows_last_word_whole() {
    do_flow_readable_keeps_a_flows_last_word_whole();
}

/// A flow's last word and the punctuation on it — "handywork.",
/// "heaven:" — torn by the boundary into the last stretch, where the
/// text ends, is laid whole, punctuation and all (ruling R26): no word
/// break parts a word from its punctuation, and the last stretch may
/// give up all but its trailing space. Torn, the word's tail would sit
/// alone in a stretch read the other way. Each fixture is checked to
/// tear its word in the walk.
pub fn do_flow_readable_keeps_a_flows_last_word_whole() {
    let once = FitMode::Once { filler: None };
    for (what, text, curve, word) in [
        ("S-curve", BLAKE_INNER, s_curve(), "handywork."),
        ("hook", ECCLESIASTES, hook(), "heaven:"),
        ("U-turn at 0.6", ECCLESIASTES, u_turn(0.6), "heaven:"),
        (
            "hook at 0.96",
            "the poem goes around the node exactly once. ",
            hook_at(0.96),
            "once.",
        ),
    ] {
        let placed = along(text, &curve, &once, 0.0);
        let (torn, laid) = torn_and_laid(&placed, text, word);
        assert!(torn, "{what}: the fixture needs {word:?} torn by the walk");
        assert_eq!(laid, 1, "{what}: {word:?} is not whole in one stretch");
        assert_reads_forward(what, &placed);
    }
}

#[test]
fn test_flow_readable_parts_a_word_too_long_to_move() {
    do_flow_readable_parts_a_word_too_long_to_move();
}

/// A word longer than half of either stretch it straddles stays parted
/// at the boundary (ruling R26's fallback): moving it whole would take
/// more than half of a stretch. Each part reads forward. A word of 45
/// letters between short words on a hook, and "purpose" beside the short
/// vertical stretch of a small U-turn: each checked to be torn by the
/// walk.
pub fn do_flow_readable_parts_a_word_too_long_to_move() {
    let long = "a b pneumonoultramicroscopicsilicovolcanoconiosis c d e f g h i j k l m n o p";
    let season = "To every thing there is a season, and a time to every purpose under the heaven:";
    for (what, text, curve, word) in [
        (
            "a long word",
            long,
            hook(),
            "pneumonoultramicroscopicsilicovolcanoconiosis",
        ),
        ("a short stretch", season, u_turn(0.7), "purpose"),
    ] {
        let placed = along(text, &curve, &FitMode::Once { filler: None }, 0.0);
        let (torn, laid) = torn_and_laid(&placed, text, word);
        assert!(torn, "{what}: the fixture needs {word:?} torn by the walk");
        assert!(laid > 1, "{what}: {word:?} was moved whole, past half a stretch");
        assert_reads_forward(what, &placed);
    }
}

#[test]
fn test_flow_readable_stops_where_the_path_turns() {
    do_flow_readable_stops_where_the_path_turns();
}

/// Text that ends short of its connector keeps its last stretch to the
/// part of the path that runs that stretch's way (#291): past the text's
/// end an S-curve turns from leftward back to rightward, and a word moved
/// whole into the last stretch, or a stretch reversed into more room than
/// it took walked, may not run on into the turn, where its clusters,
/// placed last to first, would read backward. `Once` on two S-curves, each
/// text ending in the curve's leftward middle: every stretch reads
/// forward, nothing overlaps, and the text is whole.
pub fn do_flow_readable_stops_where_the_path_turns() {
    let words = "wonderfully extraordinary circumstances overwhelmingly";
    let season = "To every thing there is a season, and a time to every purpose under the heaven:";
    for (what, text, curve) in [
        ("S-curve at 0.8", words, s_curve_at(0.8)),
        ("S-curve at 1.1", season, s_curve_at(1.1)),
    ] {
        let placed = along(text, &curve, &FitMode::Once { filler: None }, 0.0);
        let (leftward, _) = assert_reads_forward(what, &placed);
        assert!(
            leftward > 0,
            "{what}: the fixture needs text on the leftward middle"
        );
        let last = placed.trace.stretches.last().expect("stretches");
        assert_eq!(
            last.travel,
            Travel::Leftward,
            "{what}: the fixture needs the text to end leftward"
        );
        assert_no_overlap(what, &placed);
        assert!(!placed.placed.truncated, "{what}: truncated");
    }
}

#[test]
fn test_flow_readable_a_repeat_ring_closes_evenly() {
    do_flow_readable_a_repeat_ring_closes_evenly();
}

/// A `Repeat` ring around a closed outline holds a whole number of
/// copies and closes without a seam (ruling R29): the clusters placed are
/// whole copies of the pattern, and, for a text pattern, the gap between
/// every two consecutive clusters along the path — the seam from the last
/// back round to the first included — is the same, within half a point,
/// its gap solved as it is read. An ornament (no letter or digit) is
/// spaced by its ink instead (ruling R31; its gaps are
/// `do_flow_readable_an_ornament_ring_is_evenly_inked`'s) and placed as
/// walked, so a glyph-and-space ornament alternates all the way round,
/// never two spaces or two glyphs side by side, and its spaces step their
/// advance, however steep the ring turns. Five outlines, the README
/// scenes' six ornaments and two text patterns, each way round.
pub fn do_flow_readable_a_repeat_ring_closes_evenly() {
    let outlines = [
        ("rounded rect", rounded(300.0, 160.0, 25)),
        ("sharp rect", rounded(260.0, 140.0, 0)),
        ("pill", rounded(400.0, 120.0, 100)),
        ("circle", rounded(300.0, 300.0, 100)),
        ("ellipse", ellipse(300.0, 200.0)),
    ];
    let top = start(Edge::Top, 0.0);
    for (name, shape) in &outlines {
        for pattern in ["✦ ", "❦ ", "· ", "+ ", "~ ", "·:·", "To see ", "Heaven "] {
            for direction in [Direction::Clockwise, Direction::CounterClockwise] {
                let what = format!("{name} {pattern:?} {direction:?}");
                let placed = around(pattern, shape, top, direction, &FitMode::Repeat, false);
                assert!(!placed.placed.truncated, "{what}: truncated");
                let per_copy = placed.run.glyphs.iter().filter(|g| g.grapheme_start).count();
                let mut laid = placed.trace.laid.clone();
                assert_eq!(laid.len() % per_copy, 0, "{what}: a part-copy");
                laid.sort_by(|a, b| a.0.total_cmp(&b.0));
                let mut gaps: Vec<f32> = laid
                    .windows(2)
                    .map(|pair| pair[1].0 - (pair[0].0 + pair[0].1))
                    .collect();
                let (first, last) = (laid[0], laid[laid.len() - 1]);
                gaps.push(placed.path.length() + first.0 - (last.0 + last.1));
                let (least, most) = spread_of(gaps.iter().copied());
                let ornament = !placed.run.glyphs.iter().any(|g| g.letter);
                assert!(
                    ornament || most - least <= 0.5,
                    "{what}: gaps from {least} to {most}, the seam {}",
                    gaps[gaps.len() - 1]
                );
                // A glyph-and-space ornament alternates all the way round,
                // the seam and every turn of the outline included: no two
                // spaces meet (a gap) and no two glyphs (a stray).
                // An ornament's space has no ink, and steps its advance on
                // the steep turns of a ring too, not a whole line.
                if ornament {
                    let blanks: Vec<(bool, f32)> = boxes(&placed.run, &placed.placed)
                        .iter()
                        .map(|cluster| {
                            let glyph =
                                &placed.run.glyphs[placed.placed.glyphs[cluster.glyphs.start].glyph as usize];
                            (glyph.blank, cluster.size.x)
                        })
                        .collect();
                    for ((blank, advance), (_, step)) in blanks.iter().zip(&placed.trace.laid) {
                        assert!(
                            !blank || (*step - advance).abs() <= 1e-3,
                            "{what}: a space steps {step}, its advance {advance}"
                        );
                    }
                }
                if per_copy == 2 {
                    // Where each cluster sits: an ornament's glyph by its
                    // ink and a space by the middle of its advance on the
                    // baseline, which is what is set on the path (ruling
                    // R31); text by its box.
                    let baseline = Vec2::new(0.0, placed.run.baseline());
                    let mut along: Vec<(f32, bool)> = boxes(&placed.run, &placed.placed)
                        .iter()
                        .map(|cluster| {
                            let glyph =
                                &placed.run.glyphs[placed.placed.glyphs[cluster.glyphs.start].glyph as usize];
                            let at = match (ornament, glyph.ink) {
                                (true, Some(ink)) => cluster.corner + baseline + (ink.min + ink.max) * 0.5,
                                (true, None) => {
                                    cluster.corner + baseline + Vec2::new(cluster.size.x * 0.5, 0.0)
                                }
                                (false, _) => cluster.center(),
                            };
                            (arc_position(&placed.path, at), glyph.blank)
                        })
                        .collect();
                    along.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let next = along.iter().cycle().skip(1);
                    for (k, (here, there)) in along.iter().zip(next).enumerate() {
                        assert_ne!(here.1, there.1, "{what}: clusters {k} and the next are alike");
                    }
                }
            }
        }
    }
}

#[test]
fn test_flow_readable_an_ornament_ring_is_evenly_inked() {
    do_flow_readable_an_ornament_ring_is_evenly_inked();
}

/// The ink of placed glyph `k` on the canvas, as its `(min, max)`
/// corners: its ink box from shaping, at its pen.
fn ink_box(run: &ShapedRun, placed: &PlacedRun, k: usize) -> Option<(Vec2, Vec2)> {
    let glyph = placed.glyphs[k];
    let pen = glyph.pos + Vec2::new(0.0, run.baseline());
    run.glyphs[glyph.glyph as usize]
        .ink
        .map(|ink| (pen + ink.min, pen + ink.max))
}

/// The distance between two boxes, 0 where they meet.
fn box_gap(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> f32 {
    let dx = (b.0.x - a.1.x).max(a.0.x - b.1.x).max(0.0);
    let dy = (b.0.y - a.1.y).max(a.0.y - b.1.y).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// The spread of `gaps`, `(least, most)`.
fn spread_of(gaps: impl IntoIterator<Item = f32>) -> (f32, f32) {
    gaps.into_iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), g| {
            (a.min(g), b.max(g))
        })
}

/// An ornament ring's inks are as far apart all the way round as a
/// reader sees them (ruling R31), the seam from the last back round to
/// the first included, where the outline runs level and where it runs
/// steep, round a sharp corner and a tight one, on the outline and
/// pushed outward:
///
/// - every two consecutive glyphs' ink boxes are between `1/1.5` and
///   `1.5` times as far apart as any other two of the same two glyphs
///   of the pattern (a star and the dot after it; the dot and the star
///   after that);
/// - and, on an outline with no sharp corner, each copy's ink — its
///   glyphs' ink boxes as one — is between `1/1.5` and `1.5` times as
///   far from the next as any other copy is. (Round a sharp corner a
///   copy is an L, and the box round it is not its ink.)
///
/// Stepped by line-high boxes, a ring's glyphs part further on its steep
/// sides than on its level ones by the line height less the ink's.
pub fn do_flow_readable_an_ornament_ring_is_evenly_inked() {
    let outlines = [
        ("rounded rect", rounded(300.0, 160.0, 25), true),
        ("sharp rect", rounded(260.0, 140.0, 0), false),
        ("pill", rounded(400.0, 120.0, 100), true),
        ("circle", rounded(300.0, 300.0, 100), true),
        ("ellipse", ellipse(300.0, 200.0), true),
    ];
    let top = start(Edge::Top, 0.0);
    for (name, shape, smooth) in &outlines {
        for pattern in ["✦ ", "❦ ", "· ", "+ ", "~ ", "·:·", "✦ · "] {
            for direction in [Direction::Clockwise, Direction::CounterClockwise] {
                for outward in [false, true] {
                    let what = format!("{name} {pattern:?} {direction:?} outward {outward}");
                    let placed = around(pattern, shape, top, direction, &FitMode::Repeat, outward);
                    let (run, laid) = (&placed.run, &placed.placed);
                    for glyph in &run.glyphs {
                        assert_eq!(
                            glyph.ink.is_some(),
                            !glyph.blank,
                            "{what}: the fixture's ink {glyph:?}"
                        );
                    }
                    let unit = run.glyphs.iter().filter(|glyph| glyph.ink.is_some()).count();
                    let inked: Vec<(u32, (Vec2, Vec2))> = (0..laid.glyphs.len())
                        .filter_map(|k| ink_box(run, laid, k).map(|ink| (laid.glyphs[k].glyph, ink)))
                        .collect();
                    assert_eq!(inked.len() % unit, 0, "{what}: a part-copy");
                    assert!(inked.len() / unit >= 8, "{what}: {} copies", inked.len() / unit);
                    // Consecutive glyphs, by which two of the pattern they are.
                    let mut pairs: Vec<((u32, u32), f32)> = Vec::new();
                    for k in 0..inked.len() {
                        let (here, next) = (inked[k], inked[(k + 1) % inked.len()]);
                        pairs.push(((here.0, next.0), box_gap(here.1, next.1)));
                    }
                    let mut kinds: Vec<(u32, u32)> = pairs.iter().map(|pair| pair.0).collect();
                    kinds.sort_unstable();
                    kinds.dedup();
                    for kind in kinds {
                        let gaps = pairs.iter().filter(|pair| pair.0 == kind).map(|pair| pair.1);
                        let (least, most) = spread_of(gaps);
                        assert!(
                            least > 0.0 && most <= 1.5 * least,
                            "{what}: glyphs {kind:?} from {least} to {most} apart"
                        );
                    }
                    if !smooth {
                        continue;
                    }
                    let copies: Vec<(Vec2, Vec2)> = inked
                        .chunks(unit)
                        .map(|copy| {
                            copy.iter()
                                .fold(copy[0].1, |(min, max), &(_, (a, b))| (min.min(a), max.max(b)))
                        })
                        .collect();
                    let mut gaps: Vec<f32> =
                        copies.windows(2).map(|pair| box_gap(pair[0], pair[1])).collect();
                    gaps.push(box_gap(copies[copies.len() - 1], copies[0]));
                    let seam = gaps[gaps.len() - 1];
                    let (least, most) = spread_of(gaps.iter().copied());
                    assert!(
                        least > 0.0 && most <= 1.5 * least,
                        "{what}: copies from {least} to {most} apart, the seam {seam}"
                    );
                }
            }
        }
    }
}
