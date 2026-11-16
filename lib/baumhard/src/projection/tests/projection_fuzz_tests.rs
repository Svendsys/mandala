// SPDX-License-Identifier: MPL-2.0

//! The projection's fuzz properties held to account (TEST_CONVENTIONS
//! §T14): the loader targets' maps are mostly inside the projection's work
//! bound and both sides of it are reached (ruling B7.3), and the bound
//! refuses what it is there to refuse.

use serde_json::json;

use crate::document::bounds::MAX_CANVAS_COORD;
use crate::flow::tests::flow_fuzz::MAX_FRAME_GLYPHS;
use crate::flow::LINE_HEIGHT_FACTOR;
use crate::projection::tests::maps::{load_map, node};
use crate::projection::tests::projection_fuzz::{
    check_projects, projection_counts, under_ceiling, widest_advance, within_bound, EXTENT, MAX_UNITS,
    MAX_UNTANGLE_SAMPLES, MOST_RING_LINE_HEIGHT,
};
use crate::projection::tests::support::Projected;
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

/// A map of one node `fields` gives, the bound's cases' common shape.
fn one_node(fields: serde_json::Value) -> serde_json::Value {
    json!({"nodes": [node(1, None, fields)]})
}

/// The maps the bound is held to by name, each with whether it is inside
/// (rulings B47.1, B47.2): `test_the_projection_bound_holds_a_nodes_drawn_rectangle_and_a_frames_outline`
/// asserts that, and `test_every_ring_the_bound_admits_lies_within_the_extent`
/// projects those inside. The frame's cases: its node's rectangle grown by
/// its `inset`, a `gap` for each ring inside its outermost and those
/// rings' line heights, as the projection grows its outline
/// (`stages::clearance`), within `EXTENT`, at it and a unit past it; a
/// ring inside that lays text by direction alone takes a line height, and
/// is a ring of text. The rectangle's: a section's `offset` and box grow
/// it (D5), the box's lines `LINE_HEIGHT_FACTOR` times their size high and
/// its characters the widest advance across, a tab eight; the review's
/// probes of #378 are among them, each outside. A `spacing` parts a
/// placement's glyphs and moves no outline: a map with one at the canvas
/// bound is inside.
fn bound_cases() -> Vec<(String, serde_json::Value, bool)> {
    let ring = json!({"fit": "repeat", "size": 0.25, "spans": [{"text": "·"}]});
    let framed = |frame: serde_json::Value| one_node(json!({"min_size": [100.0, 40.0], "frame": frame}));
    // The node's far side is 100 along `x`: an outline reaching
    // `EXTENT - 100` past it reaches `EXTENT`.
    let edge = f64::from(EXTENT) - 100.0;
    let height = f64::from(MOST_RING_LINE_HEIGHT);
    let text = json!({"spans": [{"text": "a"}]});
    let vertical = json!({"fit": "repeat", "size": 1.0, "vertical": "|"});
    let corners = json!({"fit": "repeat", "size": 1.0, "corners": ["+", "+", "+", "+"]});
    let path = |end: serde_json::Value| {
        let mut far = json!({"position": [500.0, 0.0]});
        for (key, value) in end.as_object().into_iter().flatten() {
            far[key] = value.clone();
        }
        json!({"nodes": [node(1, None, json!({})), node(2, Some(1), far)]})
    };
    let mut spaced = ring.clone();
    spaced["spacing"] = json!(MAX_CANVAS_COORD);
    // The review's probes (#378): a dot ring at the least size round a
    // node its one section draws past the extent, tall and wide, and the
    // ellipse of several rings of `vertical` alone.
    let dust = json!({"fit": "repeat", "size": 0.000001, "spans": [{"text": "·"}]});
    let grown = |text: String, start: &str| {
        one_node(json!({
            "min_size": [1.0, 1.0],
            "sections": [{"size": 4096.0, "spans": [{"text": text}]}],
            "frame": {"inset": 0.0, "start": start, "rings": [dust.clone()]},
        }))
    };
    let ellipse = |rings: usize| {
        let rings = vec![json!({"fit": "repeat", "size": 0.000001, "vertical": "|"}); rings];
        one_node(json!({
            "position": [-4000.0, -4000.0],
            "min_size": [8000.0, 8000.0],
            "shape": "ellipse",
            "frame": {"inset": 0.0, "gap": 0.0, "rings": rings},
        }))
    };
    // A section at `offset`, `min_size` large, under a dot ring.
    let section = |offset: [f64; 2], size: [f64; 2]| {
        one_node(json!({
            "min_size": [1.0, 1.0],
            "sections": [{"offset": offset, "min_size": size}],
            "frame": {"inset": 0.0, "rings": [ring.clone()]},
        }))
    };
    // A node of one section of `text` at `size`, no frame: its rectangle
    // is its box.
    let boxed =
        |text: &str, size: f64| one_node(json!({"sections": [{"size": size, "spans": [{"text": text}]}]}));
    // Ten characters across at a size whose bound is the extent, less and
    // more a part in a thousand; eleven lines down the same.
    let widest = f64::from(widest_advance());
    let across = f64::from(EXTENT) / (10.0 * widest);
    let down = f64::from(EXTENT) / (11.0 * f64::from(LINE_HEIGHT_FACTOR));
    let cases = vec![
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
        // A ring that lays text by direction alone (#324).
        (
            "an inner vertical ring at the extent",
            framed(json!({"inset": edge - height, "gap": 0.0, "rings": [vertical.clone(), {}]})),
            true,
        ),
        (
            "an inner vertical ring past the extent",
            framed(json!({"inset": edge - height + 1.0, "gap": 0.0, "rings": [vertical.clone(), {}]})),
            false,
        ),
        (
            "an inner corners ring at the extent",
            framed(json!({"inset": edge - height, "gap": 0.0, "rings": [corners.clone(), {}]})),
            true,
        ),
        (
            "an inner corners ring past the extent",
            framed(json!({"inset": edge - height + 1.0, "gap": 0.0, "rings": [corners.clone(), {}]})),
            false,
        ),
        (
            "a vertical ring beside a ring of text",
            framed(json!({"inset": 0.0, "rings": [vertical.clone(), ring.clone()]})),
            false,
        ),
        (
            "two corners rings",
            framed(json!({"inset": 0.0, "rings": [corners.clone(), corners.clone()]})),
            false,
        ),
        (
            "a vertical ring of the most characters",
            framed(json!({"rings": [{"fit": "repeat", "vertical": "|".repeat(MAX_FRAME_GLYPHS)}]})),
            true,
        ),
        (
            "a vertical ring of a character more",
            framed(json!({"rings": [{"fit": "repeat", "vertical": "|".repeat(MAX_FRAME_GLYPHS + 1)}]})),
            false,
        ),
        (
            "corners of a character more, the four together",
            framed(
                json!({"rings": [{"fit": "repeat", "corners": ["+".repeat(MAX_FRAME_GLYPHS - 2), "+", "+", "+"]}]}),
            ),
            false,
        ),
        // The rectangle the ring is laid round (D5).
        (
            "a section to the extent",
            section([4000.0, 0.0], [4192.0, 10.0]),
            true,
        ),
        (
            "a section past the extent",
            section([4000.0, 0.0], [4193.0, 10.0]),
            false,
        ),
        (
            "a section's offset and size alone past the extent",
            section([f64::from(EXTENT), 0.0], [f64::from(EXTENT), 10.0]),
            false,
        ),
        (
            "ten characters to the extent",
            boxed(&"M".repeat(10), across * 0.999),
            true,
        ),
        (
            "ten characters past the extent",
            boxed(&"M".repeat(10), across * 1.001),
            false,
        ),
        (
            "ten tabs past the extent",
            boxed(&"\t".repeat(10), across / 8.0 * 1.001),
            false,
        ),
        (
            "eleven lines to the extent",
            boxed(&"a\n".repeat(10), down * 0.999),
            true,
        ),
        (
            "eleven lines past the extent",
            boxed(&"a\n".repeat(10), down * 1.001),
            false,
        ),
        // The review's probes (#378).
        (
            "a 30,000-line section",
            grown("a\n".repeat(30_000), "bottom"),
            false,
        ),
        ("a 30,000-character line", grown("M".repeat(30_000), "top"), false),
        (
            "an inner vertical ring the bound read no height for",
            framed(
                json!({"inset": 0.0, "gap": 0.0, "rings": [{"fit": "repeat", "size": 1.0, "vertical": "|"}, ring.clone()]}),
            ),
            false,
        ),
        (
            "an inner corners ring the bound read no height for",
            framed(
                json!({"inset": 0.0, "gap": 0.0, "rings": [{"fit": "repeat", "size": 1.0, "corners": ["+", "+", "+", "+"]}, ring.clone()]}),
            ),
            false,
        ),
        ("the ellipse of one ring of vertical alone", ellipse(1), true),
        ("the ellipse of four rings of vertical alone", ellipse(4), false),
        // A path is drawn from the rectangles of its ends.
        (
            "a connector from a node its section draws past the extent",
            json!({"nodes": [
                node(1, None, json!({"min_size": [1.0, 1.0], "sections": [{"size": 4096.0, "spans": [{"text": "a\n".repeat(30_000)}]}]})),
                node(2, Some(1), json!({"position": [-6000.0, 0.0], "connector": {"fit": "repeat", "size": 0.5, "spans": [{"text": "·"}]}})),
            ]}),
            false,
        ),
    ];
    cases
        .into_iter()
        .map(|(what, map, inside)| (what.to_string(), map, inside))
        .collect()
}

/// The bound is on what a map's work grows with — its coordinates, its
/// rings of text, its units — and so on where a node is drawn and how far
/// out a frame's rings lie (rulings B47.1, B47.2; [`bound_cases`]). The
/// loader holds an `inset`, a `gap`, an `offset` and a `min_size` to the
/// canvas bound (#364) and a text to its bytes, but a ring that far out is
/// past what the cap of a placement (R41) bounds the work of: the cap
/// bounds its glyphs, and far out each glyph reads its path a thousand
/// times (`test_every_dot_ring_the_bound_admits_takes_a_few_steps_a_glyph`).
///
/// Controls, each run and seen red, the cases each names: with the bound
/// reading no frame's reach — #370's carve-out — the nine past the extent
/// or at the canvas bound by an `inset`, a `gap` or an inner ring; with it
/// a unit wider, "an inset past the extent"; with the inner rings' line
/// heights not counted, "an inner ring of text past the extent"; with them
/// counted for a ring that lays no glyph too, "two gaps at the extent",
/// whose inner rings are empty; with a frame's own `gap` read and not its
/// defaults', "a default gap past the extent"; with the rectangle read from
/// `min_size` alone, as before ruling B47.2, the eight it draws past the
/// extent — both sections, the ten characters, tabs and lines, both of the
/// review's sections and the connector; with a node's own rectangle not
/// held, the four with no frame among them; with no line height for a ring
/// of `vertical` or `corners` alone, "an inner vertical ring past the
/// extent" and its corners twin; with those texts not counted as text, the
/// six with a second ring of text or one too long, the review's two rings
/// the bound read no height for among them; with neither, those and the
/// review's ellipse of four rings; with the widest advance read as an em,
/// "ten characters past the extent"; with a tab read as one character,
/// "ten tabs past the extent"; with no line height in a box, "eleven lines
/// past the extent".
#[test]
fn test_the_projection_bound_holds_a_nodes_drawn_rectangle_and_a_frames_outline() {
    // Every case read, and every one the bound gets wrong named.
    let wrong: Vec<String> = bound_cases()
        .into_iter()
        .filter(|(_, map, inside)| within_bound(&load_map(map.clone())) != *inside)
        .map(|(what, _, inside)| format!("{what} (inside: {inside})"))
        .collect();
    assert!(wrong.is_empty(), "the bound reads these wrong: {wrong:#?}");
}

/// How many rings and nodes [`rings_within_the_extent`] read, and the
/// furthest any reached past the origin.
#[derive(Default)]
struct Reached {
    rings: usize,
    nodes: usize,
    furthest: f32,
}

/// The end-to-end property of the bound (ruling B47.2): every node of
/// `p` is drawn within `EXTENT`, and every ring's ink within `EXTENT` and
/// its own line height twice — its outline is within `EXTENT`, its glyphs
/// stand on it a line high, and a glyph's ink may stand past its line: a
/// `|` round the grid's ellipse inks 0.61 past its outline, a line of 0.6.
/// The second line holds that and still sets a ring at the extent apart
/// from one past it, which the bound's probes put 10⁸ out. Read from the
/// projection, not from the bound's own arithmetic: what the bound reads
/// of a frame (`outline_reach`) and of a node (`drawn_bound`) is not
/// consulted. Adds what it read to `reached`; `Err` names the first node
/// or ring past it.
fn rings_within_the_extent(p: &Projected, reached: &mut Reached) -> Result<(), String> {
    use crate::projection::state::UnitBody;
    let within = |near: glam::Vec2, far: glam::Vec2, slack: f32| {
        let most = EXTENT + slack;
        near.is_finite() && far.is_finite() && near.min_element() >= -most && far.max_element() <= most
    };
    for (id, n) in &p.projector.nodes {
        reached.nodes += 1;
        reached.furthest = reached
            .furthest
            .max(n.rect.max().max_element())
            .max(-n.rect.origin.min_element());
        if !within(n.rect.origin, n.rect.max(), 0.0) {
            return Err(format!("node {id:?} is drawn at {:?}", n.rect));
        }
    }
    for (id, unit) in &p.projector.units {
        let UnitBody::Ring(ring) = &unit.body else {
            continue;
        };
        let (Some(ink), Some(placed)) = (ring.ink, ring.flow.placed.as_ref()) else {
            continue;
        };
        reached.rings += 1;
        reached.furthest = reached
            .furthest
            .max(ink.max().max_element())
            .max(-ink.origin.min_element());
        if !within(ink.origin, ink.max(), 2.0 * placed.run.line_height) {
            return Err(format!(
                "ring {id:?} is inked at {ink:?}, its line {} high",
                placed.run.line_height
            ));
        }
    }
    Ok(())
}

/// Every map of the bound's cases ([`bound_cases`]) and of the reach's
/// ([`reach_cases`]) that the bound admits, projected, draws every node
/// within `EXTENT` and every ring's ink within `EXTENT` and its own line
/// height, twice for ink past its line ([`rings_within_the_extent`];
/// ruling B47.2); the grid of the
/// densest rings is held to the same where it is projected
/// (`test_every_dot_ring_the_bound_admits_takes_a_few_steps_a_glyph`). The
/// review's probes are among the cases, each outside, so each is held to
/// it only as a control lets it in. Some ring reaches the extent's last
/// hundredth, so the property is read where it can fail.
///
/// Controls, each run and seen red, the maps each names: with the bound
/// reading the rectangle from `min_size` alone, six it then admits —
/// "a 30,000-line section", a node drawn 147,416,300 high, the review's
/// 30,000-character line, 110,952,560 across, the connector from the
/// first, both sections and "eleven lines past the extent"; with a node's
/// own rectangle not held, that connector and those lines; with no line
/// height in a box, those lines; with the bound reading no frame's reach
/// — #370's carve-out — the four with a ring 10⁹ out; with a frame's own
/// `gap` read and not its defaults', "a default gap past the extent"; with
/// the bound a unit wider, "a section past the extent", a node drawn 8,193
/// across.
#[test]
fn test_every_ring_the_bound_admits_lies_within_the_extent() {
    let maps = bound_cases().into_iter().map(|(what, map, _)| (what, map)).chain(
        reach_cases()
            .into_iter()
            .map(|(what, map)| (what.to_string(), map)),
    );
    let mut reached = Reached::default();
    let mut admitted = 0;
    // Every map admitted read, and every one drawn past the extent named.
    let mut past = Vec::new();
    for (what, map) in maps {
        let doc = load_map(map);
        if !within_bound(&doc) {
            continue;
        }
        admitted += 1;
        let p = Projected::new(doc);
        if let Err(why) = rings_within_the_extent(&p, &mut reached) {
            past.push(format!("{what}: {why}"));
        }
    }
    assert!(past.is_empty(), "admitted, and drawn past the extent: {past:#?}");
    eprintln!(
        "{admitted} maps admitted: {} nodes and {} rings, the furthest {} out",
        reached.nodes, reached.rings, reached.furthest
    );
    assert!(
        reached.rings >= 10 && reached.furthest >= EXTENT * 0.99,
        "the property is read at the extent: {} rings, the furthest {} out",
        reached.rings,
        reached.furthest
    );
}

/// The densest ring of dots a map inside the bound can draw, at the
/// bound's edge (ruling B47.1): `·` with no spacing, at the least font size
/// — a fraction of a node 1 across — round an ellipse, a sharp and a
/// rounded rectangle, its outline reaching `EXTENT` by its `inset`, by its
/// `gap`s, and at either corner of the extent; and a ring of `|` laid by
/// direction alone (#324; ruling B47.2) out to the extent by its inset,
/// `(name, map, the same map a unit further out)`.
fn dense_rings_at_the_edge() -> Vec<(String, serde_json::Value, serde_json::Value)> {
    let dots = json!({"fit": "repeat", "size": 0.25, "spans": [{"text": "·"}]});
    let bars = json!({"fit": "repeat", "size": 0.25, "vertical": "|"});
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
        let bars_by_inset =
            |past: f64| json!({"inset": extent - 1.0 + past, "gap": 0.0, "rings": [bars.clone()]});
        out.push((
            format!("{name} round the origin, a ring of vertical alone out to the extent by its inset"),
            map(shape, 0.0, bars_by_inset(0.0)),
            map(shape, 0.0, bars_by_inset(1.0)),
        ));
        let by_gaps = |past: f64| json!({"inset": 0.0, "gap": (extent - 1.0 + past) / 2.0, "rings": [{}, {}, dots.clone()]});
        out.push((
            format!("{name} round the origin, out to the extent by two gaps"),
            map(shape, 0.0, by_gaps(0.0)),
            map(shape, 0.0, by_gaps(1.0)),
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
    // The review's ellipse (#378), 8,000 across, its one ring of `|` at
    // the least size, out to the extent by its inset.
    let wide = |past: f64| {
        json!({"nodes": [node(1, None, json!({
            "position": [-4000.0, -4000.0],
            "min_size": [8000.0, 8000.0],
            "shape": "ellipse",
            "frame": {"inset": extent - 4000.0 + past, "gap": 0.0, "rings": [
                {"fit": "repeat", "size": 0.000001, "vertical": "|"},
            ]},
        }))]})
    };
    out.push((
        "an ellipse 8,000 across, a ring of vertical alone out to the extent by its inset".to_string(),
        wide(0.0),
        wide(1.0),
    ));
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
///
/// **It is a ratchet, not a work bound: no number of steps a glyph is**
/// (ruling B47.2, derived here). A check lays a ring again in every
/// projection it makes — its first, the sweep's comparison, and for its
/// one edit up to four updates, each with the oracle's projection: ten at
/// most; these maps' checks make three (a ring of spans) and five (a ring
/// of `vertical`). The input's share at `flow_fuzz::ASAN_STEP_NANOS` a
/// step is `flow_fuzz::INPUT_STEPS`, 464,396 steps, which over ten lays
/// of `MAX_PLACED_GLYPHS` glyphs is 0.71 steps a glyph: less than any ring
/// laid to the cap takes. Measured instead, each map run alone through
/// `cargo fuzz run load_raw <file> -- -runs=1` (ASan, the load average
/// some 3) against `flow_fuzz::FUZZ_TIMEOUT_SECONDS`, 20 s: the grid's
/// dots round an ellipse out to the extent, 299,958 steps a lay, 5.9 s —
/// some 6.6 µs a step over its three lays; its ring of `|` the same way,
/// 1,273,916 steps a lay, 66 s; and, inside the bound but not in this
/// grid, a dot ring 1,300 out round a node 1 across, 52,210 glyphs under
/// the cap at 1,907,788 steps a lay, 40 s, and a ring of `|` 3,050 out,
/// 2,509,574 steps a lay, 118 s. A ring a little short of the cap closes
/// its spacing, which one at the cap does not, and takes some 40 steps a
/// glyph wherever it lies. The bound holds where a ring lies, which is
/// what #374 asked of it; what bounds a `Repeat` ring's work in this
/// harness, near the origin as far out, it does not, and that is open
/// (the review of #378).
const DOT_STEPS_A_GLYPH: usize = 8;

/// The most glyph steps a glyph of a ring of `vertical` alone at the cap
/// takes inside the bound, as [`DOT_STEPS_A_GLYPH`] for a ring of spans
/// and a ratchet as it is: the grid's take at most 19.44, round an
/// ellipse out to the extent; a ring of `vertical` a little short of the
/// cap takes some 40 a glyph (`place::directional`'s pitched lays, ruling
/// B18.1 §4).
const DIRECTIONAL_STEPS_A_GLYPH: usize = 24;

/// Every map of the grid the bound admits — each ring of
/// [`dense_rings_at_the_edge`] — laid to the cap, takes at most
/// [`DOT_STEPS_A_GLYPH`] glyph steps a glyph placed, or
/// [`DIRECTIONAL_STEPS_A_GLYPH`] for a ring of `vertical` alone, its
/// placement's work in all (`LAST_RING`, `LAST_DIRECTIONAL`; ruling
/// B47.1), draws its ring within the extent
/// ([`rings_within_the_extent`]), and a unit further out each is outside.
/// The loader findings of #374 and #375 are among the maps tried, and are
/// outside the bound, so neither is laid.
///
/// Controls: with the bound reading no frame's reach — #370's carve-out —
/// the finding of #374 is inside and laid, red on its ring inked 9·10⁹ out;
/// with the bound a unit wider, red on the first ring's unit further out.
#[test]
fn test_every_dot_ring_the_bound_admits_takes_a_few_steps_a_glyph() {
    use crate::flow::{LAST_DIRECTIONAL, LAST_RING, MAX_PLACED_GLYPHS};
    let grid = dense_rings_at_the_edge();
    let findings: [(&str, &[u8]); 2] = [
        (
            "the finding of #374",
            include_bytes!(
                "../../document/tests/fuzz_findings/a_ring_too_far_out_to_project_in_time.mandala.json"
            ),
        ),
        (
            "the finding of #375",
            include_bytes!(
                "../../document/tests/fuzz_findings/a_ring_spaced_too_far_out_to_project_in_time.mandala.json"
            ),
        ),
    ];
    let mut maps: Vec<(String, crate::document::ids::Document)> = grid
        .iter()
        .map(|(what, map, _)| (what.clone(), load_map(map.clone())))
        .collect();
    for (what, bytes) in findings {
        let text = std::str::from_utf8(bytes).expect("the finding is text");
        maps.push((
            what.to_string(),
            crate::document::format::load(text).expect("the finding loads"),
        ));
    }
    let mut held = 0;
    let mut reached = Reached::default();
    for (what, doc) in maps {
        if !within_bound(&doc) {
            continue;
        }
        LAST_RING.with(|last| last.set(None));
        LAST_DIRECTIONAL.with(|last| last.set(None));
        let projected = Projected::new(doc);
        rings_within_the_extent(&projected, &mut reached).unwrap_or_else(|why| panic!("{what}: {why}"));
        drop(projected);
        // A ring of its spans, or one laid by direction: one of each map.
        let ((placed, spent), most) = LAST_RING
            .with(std::cell::Cell::get)
            .map(|ring| ((ring.placed, ring.spent), DOT_STEPS_A_GLYPH))
            .or_else(|| {
                LAST_DIRECTIONAL
                    .with(std::cell::Cell::get)
                    .map(|laid| (laid, DIRECTIONAL_STEPS_A_GLYPH))
            })
            .unwrap_or_else(|| panic!("{what}: a ring is laid"));
        eprintln!(
            "{what}: {placed} glyphs, {spent} steps, {:.2} a glyph",
            spent as f64 / placed as f64
        );
        assert_eq!(placed, MAX_PLACED_GLYPHS, "{what}: laid to the cap");
        assert!(
            spent <= most * placed,
            "{what}: {spent} steps over {placed} glyphs, past {most} a glyph"
        );
        held += 1;
    }
    assert_eq!(
        held,
        grid.len(),
        "every ring of the grid, and neither finding, is held"
    );
    for (what, _, further) in &grid {
        assert!(
            !within_bound(&load_map(further.clone())),
            "{what}, a unit further out, is outside"
        );
    }
}

/// The maps the bound's reach is held to by the projection's
/// (`test_the_bound_reads_no_less_reach_than_the_projection_grows_an_outline_by`),
/// each a frame of rings inside its outermost: where their line heights
/// are at their most — rings of a node 5,000 across at its whole size, a
/// `Fit` free to scale up, a size from the map's ring defaults — where
/// they hold no span or a span of no text, where they lay text by
/// direction alone (#324), and where the frame's `inset` and `gap` are the
/// map's defaults'.
fn reach_cases() -> Vec<(&'static str, serde_json::Value)> {
    let big = |frame: serde_json::Value| json!({"nodes": [node(1, None, json!({"min_size": [5000.0, 5000.0], "frame": frame}))]});
    let whole = json!({"size": 1.0, "spans": [{"text": "MANDALA"}]});
    vec![
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
            "a ring of vertical alone at its whole size",
            big(
                json!({"inset": 3.0, "gap": 2.0, "rings": [{"fit": "repeat", "size": 1.0, "vertical": "|"}, {}]}),
            ),
        ),
        (
            "a ring of corners alone at its whole size",
            big(json!({"inset": 3.0, "gap": 2.0, "rings": [
                {"fit": "repeat", "size": 1.0, "corners": ["+", "+", "+", "+"]},
                {},
            ]})),
        ),
        (
            "the inset and the gap the defaults'",
            json!({
                "defaults": {"frame": {"inset": 40.0, "gap": 30.0}},
                "nodes": [node(1, None, json!({"min_size": [100.0, 100.0], "frame": {"rings": [{}, {}, whole.clone()]}}))],
            }),
        ),
    ]
}

/// The bound reads a frame's reach no shorter than the projection grows
/// its outermost ring's outline by (ruling B47.1): its `inset`, its `gap`
/// a ring inside it and those rings' line heights as they were placed
/// (`RingState::extent`), over [`reach_cases`]. Where the ring then lies
/// is held end to end by
/// `test_every_ring_the_bound_admits_lies_within_the_extent`.
///
/// Controls: with the bound counting no ring's line height, red on the
/// rings at their whole size (4,922.2 grown, 7 read); with no line height
/// for a ring of `vertical` or `corners` alone, red on the ring of
/// `vertical` (4,920.2 grown, 5 read); with a frame's own `gap` read and
/// not its defaults', red on the defaults' (100 grown, 40 read).
#[test]
fn test_the_bound_reads_no_less_reach_than_the_projection_grows_an_outline_by() {
    use crate::projection::state::UnitBody;
    use crate::projection::tests::projection_fuzz::outline_reach;
    for (what, map) in reach_cases() {
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
