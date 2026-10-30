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
    Curve::Cubic(
        Vec2::ZERO,
        Vec2::new(900.0, 0.0),
        Vec2::new(-300.0, 150.0),
        Vec2::new(600.0, 150.0),
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

/// A loop read from the middle of its top side is a seal: clockwise,
/// the top runs rightward, the right side down, the bottom leftward and
/// the left side up — the last two placed reversed — and back along the
/// top; counter-clockwise, the same sides mirrored. So reads a rounded
/// rectangle, and so, with ε = cos 45° classing each step by its
/// dominant direction, do a circle and an ellipse (ruling R26). With
/// `do_flow_readable_every_stretch_reads_forward`, every side reads left
/// to right or top to bottom.
pub fn do_flow_readable_a_loop_reads_like_a_seal() {
    use Travel::{Downward, Leftward, Rightward, Upward};
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
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
            let placed = around(VERSE, &outline, start(Edge::Top, 0.5), direction, &fit, true);
            let travels: Vec<Travel> = placed.trace.stretches.iter().map(|s| s.travel).collect();
            assert_eq!(travels, expected, "{what} {direction:?}");
        }
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
fn test_flow_readable_hysteresis_keeps_a_flat_loop_to_two_stretches() {
    do_flow_readable_hysteresis_keeps_a_flat_loop_to_two_stretches();
}

/// An ellipse a hundred times wider than tall reads as two stretches,
/// its top and its bottom: each tip turns through vertical within about
/// a step, too briefly to be a stretch of its own (#291 §1 step 2, the
/// hysteresis). Started at its rightmost point, so the seam splits no
/// stretch. The fixture is checked to take a step at a tip within ε of
/// plumb, which without the hysteresis would be a vertical stretch of a
/// cluster or two.
pub fn do_flow_readable_hysteresis_keeps_a_flat_loop_to_two_stretches() {
    let flat = around(
        VERSE,
        &ellipse(4000.0, 40.0),
        start(Edge::Right, 0.5),
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
                out.push((what, *outline, from, direction));
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
    for (what, text, from, direction, fit, copy) in [
        (
            "Repeat",
            VERSE,
            start(Edge::Top, 0.0),
            Direction::CounterClockwise,
            FitMode::Repeat,
            1,
        ),
        (
            "Once with a filler",
            POEM,
            start(Edge::Top, 0.5),
            Direction::CounterClockwise,
            filler,
            3,
        ),
    ] {
        let placed = around(text, &rect, from, direction, &fit, true);
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
/// rectangle walked clockwise — down its right side, up its left —
/// consecutive clusters on either side, in one stretch, move down in
/// text order. The text has no word break, so no word is moved across a
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

/// The README's Blake scene: its outer ring, 1360 × 720 at 30 pt, and
/// its inner, 860 × 380 at 30 pt.
const BLAKE_OUTER: &str = "To see a World in a Grain of Sand, And a Heaven in a Wild Flower, \
                           Hold Infinity in the palm of your hand, And Eternity in an hour. ";
const BLAKE_INNER: &str = "The heavens declare the glory of God; and the firmament sheweth his handywork. ";

/// The README's Ecclesiastes scene: the frame around its center node.
const ECCLESIASTES: &str = "and a time to every purpose under the heaven: ";

#[test]
fn test_flow_readable_keeps_a_torn_word_whole() {
    do_flow_readable_keeps_a_torn_word_whole();
}

/// A word the walk tears across a stretch boundary is laid whole in one
/// stretch (ruling R26): "glyph" across the right side and the bottom of
/// an ellipse, "Flower" across the right side and the bottom of a wider
/// one — the README's frames and Blake scenes, as drawn. Torn, a word
/// next to a reversed stretch would have its head at one end of the
/// stretch and its tail at the other. Each fixture is checked to tear
/// its word in the walk; the placement must still read forward.
pub fn do_flow_readable_keeps_a_torn_word_whole() {
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    let top_left = start(Edge::Top, 0.0);
    for (what, text, points, outline, word) in [
        (
            "frames ellipse",
            ELLIPSE_LINE,
            20.0,
            ellipse(230.0, 190.0),
            "glyph",
        ),
        (
            "Blake outer ring",
            BLAKE_OUTER,
            30.0,
            ellipse(1360.0, 720.0),
            "Flower",
        ),
    ] {
        let placed = around_at(text, points, &outline, top_left, Direction::Clockwise, &fit, true);
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
/// "heaven:" — torn by the boundary beside the path's start, where the
/// text ends, is laid whole, punctuation and all (ruling R26): no word
/// break parts a word from its punctuation, and the stretch that ends at
/// the start may give up all but its trailing space. Torn, the word's
/// head would end the left side, read top to bottom, and its tail sit
/// alone above it at the start. The README's Blake inner ring and the
/// frame around Ecclesiastes' center node, as drawn; each is checked to
/// tear its word in the walk.
pub fn do_flow_readable_keeps_a_flows_last_word_whole() {
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    let top_left = start(Edge::Top, 0.0);
    for (what, text, points, outline, word) in [
        (
            "Blake inner ring",
            BLAKE_INNER,
            30.0,
            ellipse(860.0, 380.0),
            "handywork.",
        ),
        (
            "Ecclesiastes",
            ECCLESIASTES,
            38.0,
            ellipse(560.0, 230.0),
            "heaven:",
        ),
    ] {
        let placed = around_at(text, points, &outline, top_left, Direction::Clockwise, &fit, true);
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
/// more than half of a stretch. Each part reads forward. The word is 45
/// letters, between short words, on a rounded rectangle: checked to be
/// torn by the walk at a boundary between two inner stretches.
pub fn do_flow_readable_parts_a_word_too_long_to_move() {
    let text = "a b pneumonoultramicroscopicsilicovolcanoconiosis c d e f g h i j k l m n o p";
    let word = "pneumonoultramicroscopicsilicovolcanoconiosis";
    let placed = around(
        text,
        &rounded(300.0, 160.0, 25),
        start(Edge::Top, 0.5),
        Direction::Clockwise,
        &FitMode::Fit { min: 0.5, max: 2.0 },
        true,
    );
    let (torn, laid) = torn_and_laid(&placed, text, word);
    assert!(torn, "the fixture needs the word torn by the walk");
    assert!(laid > 1, "the word was moved whole, past half a stretch");
    assert_reads_forward("a long word", &placed);
}
