// SPDX-License-Identifier: MPL-2.0

//! The projection's fuzz properties held to account (TEST_CONVENTIONS
//! §T14): the loader targets' maps are mostly inside the projection's work
//! bound and both sides of it are reached (ruling B7.3), and the bound
//! refuses what it is there to refuse.

use serde_json::json;

use crate::document::bounds::MAX_CANVAS_COORD;
use crate::projection::tests::maps::{load_map, node};
use crate::projection::tests::projection_fuzz::{
    check_projects, projection_counts, under_ceiling, within_bound, EXTENT, MAX_UNITS, MAX_UNTANGLE_SAMPLES,
    MOST_RING_LINE_HEIGHT,
};
use crate::util::tests::choices::Choices;

#[test]
fn test_most_structured_loader_maps_are_projected() {
    let (projected0, skipped0, _) = projection_counts();
    let mut loaded = 0;
    for seed in 0..120 {
        let mut c = Choices::seeded(0x0B7_0000 + seed);
        let generated = crate::document::tests::format_fuzz::check_generated(
            &mut c,
            crate::document::tests::format_fuzz::MAX_GENERATED_ROWS,
        )
        .unwrap_or_else(|why| panic!("seed {seed}: {why}"));
        if generated.outcome.ends_with("loaded") {
            loaded += 1;
        }
    }
    // The counts are this thread's: exactly what this sweep added.
    let (projected, skipped, _) = projection_counts();
    let (projected, skipped) = (projected - projected0, skipped - skipped0);
    assert!(loaded > 20, "the sweep loads maps: {loaded}");
    assert!(projected > 0, "some loaded maps are projected");
    eprintln!("projection of structured loader maps: {projected} projected, {skipped} skipped");
    assert!(
        skipped * 2 < projected + skipped,
        "under half the loaded maps are skipped ({skipped} of {})",
        projected + skipped
    );
}

#[test]
fn test_the_projection_bound_refuses_far_coordinates_a_second_text_ring_and_too_many_units() {
    let ring = |id: u32, x: f32| {
        node(
            id,
            None,
            json!({"position": [x, 0.0], "frame": {"rings": [{"spans": [{"text": "one ring"}]}]}}),
        )
    };
    assert!(
        within_bound(&load_map(json!({"nodes": [ring(1, 0.0)]}))),
        "one ring of text near the origin is inside"
    );
    assert!(
        !within_bound(&load_map(json!({"nodes": [ring(1, EXTENT * 2.0)]}))),
        "a node past the extent is outside"
    );
    assert!(
        !within_bound(&load_map(json!({"nodes": [ring(1, 0.0), ring(2, 300.0)]}))),
        "a second ring of text is outside"
    );
    let chain = |nodes: u32| {
        let records: Vec<_> = (1..=nodes)
            .map(|id| match id {
                1 => node(id, None, json!({})),
                _ => node(id, Some(id - 1), json!({"connector": {}})),
            })
            .collect();
        load_map(json!({ "nodes": records }))
    };
    // A chain of n nodes draws n - 1 connectors.
    let most = MAX_UNITS as u32 + 1;
    assert!(
        within_bound(&chain(most)),
        "a chain of {} connectors is inside",
        most - 1
    );
    assert!(!within_bound(&chain(most + 1)), "one connector more is outside");
}

/// The bound is on what a map's work grows with — its coordinates, its
/// rings of text, its units — and so on how far out a frame's rings lie
/// (ruling B47.1): its node's rectangle grown by its `inset`, a `gap` for
/// each ring inside its outermost and those rings' line heights, as the
/// projection grows its outline (`stages::clearance`), is held within
/// `EXTENT` as every other coordinate is. The loader holds an `inset` and a
/// `gap` to the canvas bound (#364), but a ring that far out is past what
/// the cap of a placement (R41) bounds the work of: the cap bounds its
/// glyphs, and far out each glyph reads its path a thousand times
/// (`test_every_dot_ring_the_bound_admits_takes_a_few_steps_a_glyph`). A
/// `spacing` parts a placement's glyphs and moves no outline: a map with
/// one at the canvas bound is inside it.
///
/// Controls: with the bound reading no frame's reach — #370's carve-out —
/// red on "an inset past the extent"; with it a unit wider, the same;
/// with the inner rings' line heights not counted, red on "an inner ring
/// of text past the extent"; with them counted for a ring of no span too,
/// red on "two gaps at the extent", whose inner rings are empty; with a
/// frame's own `gap` read and not its defaults', red on "a default gap
/// past the extent".
#[test]
fn test_the_projection_bound_holds_a_frames_outline_and_not_a_spacing() {
    let ring = json!({"fit": "repeat", "size": 0.25, "spans": [{"text": "·"}]});
    let framed = |frame: serde_json::Value| json!({"nodes": [node(1, None, json!({"min_size": [100.0, 40.0], "frame": frame }))]});
    // The node's far side is 100 along `x`: an outline reaching
    // `EXTENT - 100` past it reaches `EXTENT`.
    let edge = f64::from(EXTENT) - 100.0;
    let height = f64::from(MOST_RING_LINE_HEIGHT);
    let text = json!({"spans": [{"text": "a"}]});
    let path = |end: serde_json::Value| {
        let mut far = json!({"position": [500.0, 0.0]});
        for (key, value) in end.as_object().into_iter().flatten() {
            far[key] = value.clone();
        }
        json!({"nodes": [node(1, None, json!({})), node(2, Some(1), far)]})
    };
    let mut spaced = ring.clone();
    spaced["spacing"] = json!(MAX_CANVAS_COORD);
    let cases = [
        (
            "an inset of nothing",
            framed(json!({"inset": 0.0, "rings": [ring.clone()]})),
            true,
        ),
        (
            "an inset at the extent",
            framed(json!({"inset": edge, "rings": [ring.clone()]})),
            true,
        ),
        (
            "an inset past the extent",
            framed(json!({"inset": edge + 1.0, "rings": [ring.clone()]})),
            false,
        ),
        (
            "an inset at the canvas bound",
            framed(json!({"inset": MAX_CANVAS_COORD, "rings": [ring.clone()]})),
            false,
        ),
        (
            "two gaps at the extent",
            framed(json!({"inset": 0.0, "gap": edge / 2.0, "rings": [{}, {}, ring.clone()]})),
            true,
        ),
        (
            "two gaps past the extent",
            framed(json!({"inset": 0.0, "gap": edge / 2.0 + 1.0, "rings": [{}, {}, ring.clone()]})),
            false,
        ),
        (
            "a gap at the canvas bound",
            framed(json!({"inset": 0.0, "gap": MAX_CANVAS_COORD, "rings": [{}, ring.clone()]})),
            false,
        ),
        (
            "a gap at the canvas bound, its frame of one ring",
            framed(json!({"inset": 0.0, "gap": MAX_CANVAS_COORD, "rings": [ring.clone()]})),
            true,
        ),
        (
            "an empty inner ring at the extent",
            framed(json!({"inset": edge, "gap": 0.0, "rings": [{}, ring.clone()]})),
            true,
        ),
        (
            "an inner ring of text at the extent",
            framed(json!({"inset": edge - height, "gap": 0.0, "rings": [text.clone(), {}]})),
            true,
        ),
        (
            "an inner ring of text past the extent",
            framed(json!({"inset": edge - height + 1.0, "gap": 0.0, "rings": [text.clone(), {}]})),
            false,
        ),
        (
            "a ring of text at the canvas bound",
            framed(json!({"inset": MAX_CANVAS_COORD, "rings": [text.clone()]})),
            false,
        ),
        (
            "a default gap past the extent",
            json!({
                "defaults": {"frame": {"gap": MAX_CANVAS_COORD}},
                "nodes": [node(1, None, json!({"frame": {"inset": 0.0, "rings": [{}, ring.clone()]}}))],
            }),
            false,
        ),
        (
            "a ring's spacing at the canvas bound",
            framed(json!({"rings": [spaced]})),
            true,
        ),
        (
            "a frame's spacing at the canvas bound",
            framed(json!({"spacing": MAX_CANVAS_COORD, "rings": [ring.clone()]})),
            true,
        ),
        (
            "a connector's spacing at the canvas bound",
            path(json!({"connector": {"spacing": MAX_CANVAS_COORD}})),
            true,
        ),
        (
            "a link's spacing at the canvas bound",
            path(json!({"links": [{"to": 1, "kind": "line", "spacing": MAX_CANVAS_COORD}]})),
            true,
        ),
    ];
    for (what, map, inside) in cases {
        assert_eq!(within_bound(&load_map(map)), inside, "{what}");
    }
}

/// The densest ring of dots a map inside the bound can draw, at the
/// bound's edge (ruling B47.1): `·` with no spacing, at the least font size
/// — a fraction of a node 1 across — round an ellipse, a sharp and a
/// rounded rectangle, its outline reaching `EXTENT` by its `inset`, by its
/// `gap`s, and at either corner of the extent, `(name, map, the same map a
/// unit further out)`.
fn dense_rings_at_the_edge() -> Vec<(String, serde_json::Value, serde_json::Value)> {
    let dots = json!({"fit": "repeat", "size": 0.25, "spans": [{"text": "·"}]});
    let shapes = [
        ("ellipse", json!("ellipse")),
        ("sharp rectangle", json!({"rectangle": {"radius": 0}})),
        ("rounded rectangle", json!({"rectangle": {"radius": 30}})),
    ];
    let extent = f64::from(EXTENT);
    // A node 1 across at `at`, its outline grown `reach` by `frame`.
    let map = |shape: &serde_json::Value, at: f64, frame: serde_json::Value| {
        json!({"nodes": [node(1, None, json!({
            "position": [at, at],
            "min_size": [1.0, 1.0],
            "shape": shape,
            "frame": frame,
        }))]})
    };
    // A ring 2,000 out, its node at the far corner and at the near one.
    let corner = 2_000.0;
    let mut out = Vec::new();
    for (name, shape) in &shapes {
        let by_inset = |past: f64| json!({"inset": extent - 1.0 + past, "gap": 0.0, "rings": [dots.clone()]});
        out.push((
            format!("{name} round the origin, out to the extent by its inset"),
            map(shape, 0.0, by_inset(0.0)),
            map(shape, 0.0, by_inset(1.0)),
        ));
        let by_gaps = |past: f64| json!({"inset": 0.0, "gap": (extent - 1.0 + past) / 2.0, "rings": [{}, {}, dots.clone()]});
        out.push((
            format!("{name} round the origin, out to the extent by two gaps"),
            map(shape, 0.0, by_gaps(0.0)),
            map(shape, 0.0, by_gaps(2.0)),
        ));
        let ring = |past: f64| json!({"inset": corner + past, "gap": 0.0, "rings": [dots.clone()]});
        out.push((
            format!("{name} {corner} out at the extent's far corner"),
            map(shape, extent - 1.0 - corner, ring(0.0)),
            map(shape, extent - 1.0 - corner, ring(1.0)),
        ));
        out.push((
            format!("{name} {corner} out at the extent's near corner"),
            map(shape, corner - extent, ring(0.0)),
            map(shape, corner - extent, ring(1.0)),
        ));
    }
    out
}

/// The most glyph steps a glyph of a ring of dots at the cap takes, its
/// placement's work in all over the glyphs it places, inside the bound
/// (ruling B47.1): what the cap of a placement (R41) bounds a ring's work
/// by when its outline is near enough that its path's `f32` points part
/// its glyphs. The densest rings at the bound's edge take at most 4.58,
/// round an ellipse out to the extent; the finding, 9 `gap`s of 10⁹ out,
/// 62.3, nearly all of it strides that leave its ink where the `f32` grid
/// sets it (#376).
const DOT_STEPS_A_GLYPH: usize = 8;

/// Every map of the grid the bound admits — each ring of
/// [`dense_rings_at_the_edge`] — laid to the cap, takes at most
/// [`DOT_STEPS_A_GLYPH`] glyph steps a glyph placed, its placement's work
/// in all (`LAST_RING`; ruling B47.1), and a unit further out each is
/// outside. The loader finding `a_ring_too_far_out_to_project_in_time` is
/// among the maps tried, and is outside the bound, so it is not laid.
///
/// Controls: with the bound reading no frame's reach — #370's carve-out —
/// the finding is inside and laid, red on its steps a glyph (62.3); with
/// the bound a unit wider, red on the first ring's unit further out.
#[test]
fn test_every_dot_ring_the_bound_admits_takes_a_few_steps_a_glyph() {
    use crate::flow::{LAST_RING, MAX_PLACED_GLYPHS};
    use crate::projection::tests::support::Projected;
    let grid = dense_rings_at_the_edge();
    let finding = std::str::from_utf8(include_bytes!(
        "../../document/tests/fuzz_findings/a_ring_too_far_out_to_project_in_time.mandala.json"
    ))
    .expect("the finding is text");
    let mut maps: Vec<(String, crate::document::ids::Document)> = grid
        .iter()
        .map(|(what, map, _)| (what.clone(), load_map(map.clone())))
        .collect();
    maps.push((
        "the finding".to_string(),
        crate::document::format::load(finding).expect("the finding loads"),
    ));
    let mut held = 0;
    for (what, doc) in maps {
        if !within_bound(&doc) {
            continue;
        }
        LAST_RING.with(|last| last.set(None));
        drop(Projected::new(doc));
        let ring = LAST_RING
            .with(std::cell::Cell::get)
            .unwrap_or_else(|| panic!("{what}: a ring is laid"));
        eprintln!(
            "{what}: {} glyphs, {} steps, {:.2} a glyph",
            ring.placed,
            ring.spent,
            ring.spent as f64 / ring.placed as f64
        );
        assert_eq!(ring.placed, MAX_PLACED_GLYPHS, "{what}: laid to the cap");
        assert!(
            ring.spent <= DOT_STEPS_A_GLYPH * ring.placed,
            "{what}: {} steps over {} glyphs, past {DOT_STEPS_A_GLYPH} a glyph",
            ring.spent,
            ring.placed
        );
        held += 1;
    }
    assert_eq!(
        held,
        grid.len(),
        "every ring of the grid, and not the finding, is held"
    );
    for (what, _, further) in &grid {
        assert!(
            !within_bound(&load_map(further.clone())),
            "{what}, a unit further out, is outside"
        );
    }
}

/// The bound reads a frame's reach no shorter than the projection grows
/// its outermost ring's outline by (ruling B47.1): its `inset`, its `gap`
/// a ring inside it and those rings' line heights as they were placed
/// (`RingState::extent`). Read where the line heights are at their most —
/// rings of a node 5,000 across at its whole size, a `Fit` free to scale
/// up, a size from the map's ring defaults — where rings inside hold no
/// span or a span of no text, and where the frame's `inset` and `gap` are
/// the map's defaults'.
///
/// Controls: with the bound counting no ring's line height, red on the
/// rings at their whole size (4,922.2 grown, 7 read); with a frame's own
/// `gap` read and not its defaults', red on the defaults' (100 grown, 40
/// read).
#[test]
fn test_the_bound_reads_no_less_reach_than_the_projection_grows_an_outline_by() {
    use crate::projection::state::UnitBody;
    use crate::projection::tests::projection_fuzz::outline_reach;
    use crate::projection::tests::support::Projected;
    let big = |frame: serde_json::Value| json!({"nodes": [node(1, None, json!({"min_size": [5000.0, 5000.0], "frame": frame}))]});
    let whole = json!({"size": 1.0, "spans": [{"text": "MANDALA"}]});
    let maps = [
        (
            "rings at their whole size",
            big(json!({"inset": 3.0, "gap": 2.0, "rings": [whole.clone(), whole.clone(), {}]})),
        ),
        (
            "a fit free to scale up",
            big(json!({"rings": [
                {"fit": {"fit": {"min": 0.5, "max": 4.0}}, "size": 0.5, "spans": [{"text": "A"}]},
                {},
            ]})),
        ),
        (
            "a size from the ring defaults",
            json!({
                "defaults": {"ring": {"size": 1.0}},
                "nodes": [node(1, None, json!({"min_size": [5000.0, 5000.0], "frame": {"rings": [
                    {"spans": [{"text": "MANDALA"}]},
                    {},
                ]}}))],
            }),
        ),
        (
            "empty rings and a span of no text inside",
            big(
                json!({"gap": 5.0, "rings": [{}, {"spans": [{"text": ""}]}, {}, {}, {"spans": [{"text": "·"}]}]}),
            ),
        ),
        (
            "the inset and the gap the defaults'",
            json!({
                "defaults": {"frame": {"inset": 40.0, "gap": 30.0}},
                "nodes": [node(1, None, json!({"min_size": [100.0, 100.0], "frame": {"rings": [{}, {}, whole.clone()]}}))],
            }),
        ),
    ];
    for (what, map) in maps {
        let doc = load_map(map);
        let frame = doc
            .tree()
            .descendants()
            .find(|&id| {
                matches!(
                    doc.tree().arena()[id].get(),
                    crate::document::element::DocElement::Frame(_)
                )
            })
            .unwrap_or_else(|| panic!("{what}: a frame"));
        let reach = outline_reach(doc.tree(), frame);
        let projected = Projected::new(doc);
        let mut rings: Vec<_> = projected
            .projector
            .units
            .iter()
            .filter_map(|(&id, unit)| match &unit.body {
                UnitBody::Ring(ring) if ring.frame == Some(frame) => Some((id, ring)),
                _ => None,
            })
            .collect();
        rings.sort_by_key(|(id, _)| *id);
        assert!(rings.len() > 1, "{what}: rings inside its outermost");
        let (_, outermost) = rings[rings.len() - 1];
        let (_, inside) = rings[rings.len() - 2];
        let resolved = &outermost
            .resolved
            .as_ref()
            .unwrap_or_else(|| panic!("{what}: placed"))
            .frame;
        let (heights, count) = inside.extent.unwrap_or_else(|| panic!("{what}: placed inside"));
        let grown =
            f64::from(resolved.inset) + f64::from(heights) + f64::from(count) * f64::from(resolved.gap);
        // The projection sums in `f32`, which may round a sum up.
        assert!(
            grown <= reach * (1.0 + 4.0 * f64::from(f32::EPSILON)),
            "{what}: the projection grows its outline {grown}, the bound reads {reach}"
        );
    }
}

/// `copies` of the first seed's map (`line_link_beside_connector_1`) side
/// by side, 3,000 apart, each with a second line link: a node 5,182.7
/// above its two children, with a line link at the default link size to
/// each, beside their connectors.
fn seed_copies(copies: u32) -> crate::document::ids::Document {
    let mut nodes = Vec::new();
    for k in 0..copies {
        let (x, id) = (3000.0 * k as f64 - 3000.0, 10 * k + 10);
        nodes.push(json!({"id": id, "position": [x, 5182.7446],
            "links": [{"to": id + 3, "kind": "line"}, {"to": id + 7, "kind": "line"}]}));
        nodes.push(json!({"id": id + 7, "parent": id, "position": [x, 0.0],
            "sections": [{"channel": 2, "spans": [{"text": "\u{fe0f}\u{20e3}", "size": 12.5}]}, {"channel": 3}, {"channel": 3}]}));
        nodes.push(
            json!({"id": id + 3, "parent": id, "position": [x, 0.0], "sections": [], "frame": {"rings": []},
            "connector": {"spans": []}, "links": [{"to": id, "kind": "portal"}]}),
        );
    }
    load_map(json!({"defaults": {"link": {"size": 0.5}}, "nodes": nodes}))
}

/// What `check_projects` did with `doc`: whether it projected it, and
/// whether it abandoned it as cut short, and the samples `untangle` took.
fn checked(doc: &crate::document::ids::Document) -> (bool, bool, usize) {
    use crate::projection::geometry::UNTANGLE_SAMPLED;
    let (projected0, _, cut0) = projection_counts();
    UNTANGLE_SAMPLED.with(|c| c.set(0));
    check_projects(doc).unwrap();
    let (projected, _, cut) = projection_counts();
    (
        projected > projected0,
        cut > cut0,
        UNTANGLE_SAMPLED.with(std::cell::Cell::get),
    )
}

#[test]
fn test_a_map_whose_untangling_reaches_the_ceiling_is_abandoned_and_counted() {
    // One copy of the first seed's map with a second line link takes
    // fewer samples in a check than `MAX_UNTANGLE_SAMPLES`, and is
    // projected and checked; two take more, and the check is abandoned
    // when they reach it, counted as cut short and not as projected, and
    // no more samples are taken than the ceiling and the one lane that
    // reached it — here at most three curves at the sampling's cap (#346).
    // The failing inputs are a ceiling half as high, which abandons the
    // one copy; samples never taken from it, which projects the two; and
    // samples not counted, though a cut means more than the ceiling was
    // sampled.
    for doc in [seed_copies(1), seed_copies(2)] {
        assert!(within_bound(&doc), "precondition: inside the coordinate bound");
    }
    let (projected, cut, taken) = checked(&seed_copies(1));
    assert!(
        projected && !cut && 0 < taken && taken <= MAX_UNTANGLE_SAMPLES,
        "one copy: projected {projected}, cut {cut}, {taken} samples"
    );
    let (projected, cut, taken) = checked(&seed_copies(2));
    assert!(
        !projected && cut && MAX_UNTANGLE_SAMPLES < taken && taken <= MAX_UNTANGLE_SAMPLES + 3 * 65_536,
        "two copies: projected {projected}, cut {cut}, {taken} samples"
    );
}

#[test]
fn test_the_untangle_ceiling_is_unset_but_for_a_check() {
    // A test that does not set the harness's ceiling untangles in full:
    // unset on a fresh thread, unset and unmarked again once a check is
    // done — one cut short, and one that panics. The failing inputs are
    // the ceiling left set after a check, and the ceiling and the mark
    // cleared only when a check returns.
    use crate::projection::geometry::{UNTANGLE_CEILING, UNTANGLE_CUT};
    let clear = || {
        (
            UNTANGLE_CEILING.with(std::cell::Cell::get),
            UNTANGLE_CUT.with(std::cell::Cell::get),
        )
    };
    assert_eq!(clear(), (None, false), "unset on a fresh thread");
    let (_, cut, _) = checked(&seed_copies(2));
    assert!(cut, "precondition: the check was cut short");
    assert_eq!(clear(), (None, false), "unset after a check cut short");
    let panicked = std::panic::catch_unwind(|| {
        under_ceiling(|| {
            UNTANGLE_CUT.with(|cut| cut.set(true));
            panic!("a check that panics");
        })
    });
    assert!(panicked.is_err(), "precondition: the check panicked");
    assert_eq!(clear(), (None, false), "unset after a check that panicked");
}

#[test]
fn test_the_untangle_ceiling_is_overshot_by_one_lane_at_most() {
    // A junction of 128 thin straight paths, each sampled at the cap, so
    // that its first round alone would take four times the ceiling: under
    // the ceiling, `untangle` takes at most the ceiling and the one lane
    // that reaches it, and marks the check; it takes more than the
    // ceiling, since that is what cuts it. The failing inputs are the
    // ceiling taken from once a round, after every lane of it is sampled,
    // and samples not counted.
    use crate::projection::geometry::{untangle, Lane, UNTANGLE_SAMPLED};
    use glam::Vec2;
    let lanes: Vec<Lane> = (0..128)
        .map(|k| Lane {
            start: (Vec2::new(k as f32, 0.0), Vec2::Y, 0.0),
            hold: 0.0,
            end: (Vec2::new(k as f32, 2000.0), -Vec2::Y, 0.0),
            via: Vec::new(),
            width: 0.01,
            straight: true,
        })
        .collect();
    UNTANGLE_SAMPLED.with(|c| c.set(0));
    let (_, cut) = under_ceiling(|| untangle(&lanes));
    let sampled = UNTANGLE_SAMPLED.with(std::cell::Cell::get);
    assert!(
        cut && 128 * 65_536 > 4 * MAX_UNTANGLE_SAMPLES,
        "precondition: cut, and one round four times the ceiling"
    );
    assert!(
        MAX_UNTANGLE_SAMPLES < sampled && sampled <= MAX_UNTANGLE_SAMPLES + 65_536,
        "{sampled} samples taken under a ceiling of {MAX_UNTANGLE_SAMPLES}"
    );
}

#[test]
fn test_both_seeds_of_the_untangling_timeout_are_projected_in_full() {
    // The two loader seeds this ceiling was made for (#346), each a line
    // link beside a connector: checked in full, not cut short.
    let dir = crate::util::doc_fixtures::repo_path("fuzz/seeds/load_structured");
    for seed in ["line_link_beside_connector_1", "line_link_beside_connector_2"] {
        let bytes = std::fs::read(dir.join(seed)).unwrap();
        let (projected0, _, cut0) = projection_counts();
        crate::document::tests::format_fuzz::check_structured(&bytes).unwrap();
        let (projected, _, cut) = projection_counts();
        assert_eq!(
            (projected - projected0, cut - cut0),
            (1, 0),
            "{seed}: projected, not cut"
        );
    }
}
