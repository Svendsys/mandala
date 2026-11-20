// SPDX-License-Identifier: MPL-2.0

//! The projection's fuzz properties held to account (TEST_CONVENTIONS
//! §T14): the loader targets' maps are mostly inside the projection's work
//! bound and both sides of it are reached (ruling B7.3), and the bound
//! refuses what it is there to refuse.

use serde_json::json;

use crate::document::bounds::MAX_CANVAS_COORD;
use crate::document::element::DocElement;
use crate::flow::tests::flow_fuzz::INPUT_STEPS;
use crate::flow::tests::flow_fuzz::MAX_FRAME_GLYPHS;
use crate::flow::LINE_HEIGHT_FACTOR;
use crate::projection::tests::maps::{load_map, node};
use crate::projection::tests::projection_fuzz::{
    check_projects, frame_steps, projection_counts, repeat_glyphs, rings_within_their_work, under_ceiling,
    widest_advance, within_bound, EXTENT, MAX_UNITS, MAX_UNTANGLE_SAMPLES, MOST_RING_GLYPHS,
    MOST_RING_LINE_HEIGHT, RING_LAYS_A_CHECK, RING_SETUP_STEPS, RING_STEPS_A_GLYPH,
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
    // A ring that lays its text once, so that how far out it lies is all
    // that can hold it outside; the cases of `spacing` lay one `Repeat`.
    let ring = json!({"fit": "spread", "size": 0.25, "spans": [{"text": "·"}]});
    let repeat = json!({"fit": "repeat", "size": 0.25, "spans": [{"text": "·"}]});
    let framed = |frame: serde_json::Value| one_node(json!({"min_size": [100.0, 40.0], "frame": frame}));
    // The node's far side is 100 along `x`: an outline reaching
    // `EXTENT - 100` past it reaches `EXTENT`.
    let edge = f64::from(EXTENT) - 100.0;
    let height = f64::from(MOST_RING_LINE_HEIGHT);
    let text = json!({"spans": [{"text": "a"}]});
    // Spaced, so that its glyphs are few wherever it lies (ruling B47.3).
    let vertical = json!({"fit": "repeat", "size": 1.0, "spacing": 800.0, "vertical": "|"});
    let corners = json!({"fit": "repeat", "size": 1.0, "spacing": 800.0, "corners": ["+", "+", "+", "+"]});
    let path = |end: serde_json::Value| {
        let mut far = json!({"position": [500.0, 0.0]});
        for (key, value) in end.as_object().into_iter().flatten() {
            far[key] = value.clone();
        }
        json!({"nodes": [node(1, None, json!({})), node(2, Some(1), far)]})
    };
    let mut spaced = repeat.clone();
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
            framed(json!({"spacing": MAX_CANVAS_COORD, "rings": [repeat.clone()]})),
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
            framed(json!({"rings": [{"fit": "spread", "vertical": "|".repeat(MAX_FRAME_GLYPHS)}]})),
            true,
        ),
        (
            "a vertical ring of a character more",
            framed(json!({"rings": [{"fit": "spread", "vertical": "|".repeat(MAX_FRAME_GLYPHS + 1)}]})),
            false,
        ),
        (
            "corners of a character more, the four together",
            framed(
                json!({"rings": [{"fit": "spread", "corners": ["+".repeat(MAX_FRAME_GLYPHS - 2), "+", "+", "+"]}]}),
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
        // Inside where it lies, outside for its work (ruling B47.3).
        ("the ellipse of one ring of vertical alone", ellipse(1), false),
        ("the ellipse of four rings of vertical alone", ellipse(4), false),
        // A default ignorable a face maps an em wide, which the shaper lays
        // as a glyph at no advance (the review of #378): 6,191 glyphs laid,
        // 41 copies, and every glyph of a copy is one the bound counts.
        (
            "a ring of `·` and 150 U+180E",
            one_node(json!({
                "position": [-350.0, -350.0],
                "min_size": [700.0, 700.0],
                "shape": "ellipse",
                "frame": {"inset": 0.0, "gap": 0.0, "rings": [
                    {"fit": "repeat", "size": 0.25, "spans": [{"text": format!("·{}", "\u{180E}".repeat(150))}]},
                ]},
            })),
            false,
        ),
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
/// times (`test_every_repeat_ring_the_bound_admits_fits_the_share`).
///
/// Controls, each run and seen red, the cases each names: with no frame's
/// outline held (`outline_near`) — #370's carve-out — the nine past the
/// extent or at the canvas bound by an `inset`, a `gap` or an inner ring;
/// with it a unit wider, the five a unit past it; with the inner rings'
/// line heights not counted, the three inner rings past the extent; with
/// them counted for a ring that lays no glyph too, "two gaps at the extent"
/// and "an empty inner ring at the extent"; with a frame's own `gap` read
/// and not its defaults', "a default gap past the extent"; with the
/// rectangle read from `min_size` alone, as before ruling B47.2, the eight
/// it draws past the extent — both sections, the ten characters, tabs and
/// lines, both of the review's sections and the connector; with a node's
/// own rectangle not held, the four with no frame among them; with no line
/// height for a ring of `vertical` or `corners` alone, "an inner vertical
/// ring past the extent" and its corners twin; with those texts not counted
/// as text, the five with a second ring of text or one too long, the
/// review's inner corners ring among them; with the widest advance read as
/// an em, the ten characters and the ten tabs past the extent; with a tab
/// read as one character, the tabs; with no line height in a box, "eleven
/// lines past the extent"; with no `Repeat` ring's work bounded (ruling
/// B47.3), "the ellipse of one ring of vertical alone" and "a ring of `·`
/// and 150 U+180E"; with a copy's glyphs counted as those that advance, the
/// latter; with the most glyphs halved, or a ring's `spacing` not read, "an
/// inner vertical ring at the extent". The review's probes held outside by
/// more than one rule go red with them together: its inner vertical ring
/// and the ellipse of one with neither the texts by direction nor the work
/// bounded, the ellipse of four with neither those texts' line heights nor
/// their count nor the work.
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

/// The end-to-end property of the bound (rulings B47.2, B47.3), read from
/// the projection: every node of `p` is drawn within `EXTENT`; every ring's
/// ink lies within `EXTENT` and its own line height twice — its outline is
/// within `EXTENT`, its glyphs stand on it a line high, and a glyph's ink
/// may stand past its line (a `|` round an ellipse at the extent inked 0.61
/// past its outline, a line of 0.6), which the second line holds and still
/// sets a ring at the extent apart from one past it, which the bound's
/// probes put 10⁸ out; and every `Repeat` ring lays no more glyphs than
/// [`repeat_glyphs`] read for it. Where the rings lie, what the bound reads
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
        // No more glyphs than the bound read for a `Repeat` ring.
        let bound = ring
            .frame
            .and_then(|frame| repeat_glyphs(p.doc.tree(), frame, *id));
        if let Some(bound) = bound.filter(|&bound| placed.placed.glyphs.len() > bound) {
            return Err(format!(
                "ring {id:?} lays {} glyphs, its bound {bound}",
                placed.placed.glyphs.len()
            ));
        }
    }
    Ok(())
}

/// Every map of the bound's cases ([`bound_cases`]) and of the reach's
/// ([`reach_cases`]) that the bound admits, projected, draws every node
/// within `EXTENT` and every ring's ink within `EXTENT` and its own line
/// height, twice for ink past its line ([`rings_within_the_extent`];
/// ruling B47.2), and lays each ring within the work the bound read for it
/// — a `Repeat` ring no more glyphs than its bound and no more steps than
/// `RING_STEPS_A_GLYPH` a glyph of it and `RING_SETUP_STEPS`, a frame laid
/// by halves no more than `frame_steps` ([`rings_within_their_work`], which
/// every check of a fuzz run makes too; ruling B47.3). The grid of the
/// densest rings is held to the same where it is projected
/// (`test_every_repeat_ring_the_bound_admits_fits_the_share`). The
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
/// height in a box, those lines; with no frame's outline held
/// (`outline_near`, #370's carve-out), the four with a ring inked 7·10⁸
/// out; with a frame's own `gap` read and not its defaults', "a default
/// gap past the extent"; with the bound a unit wider, "a section past the
/// extent", a node drawn 8,193 across; with no reach in a `Repeat` ring's
/// perimeter, "an inner vertical ring at the extent", 28 glyphs laid of 5
/// bounded; with a copy's glyphs counted as those that advance, "a ring of
/// `·` and 150 U+180E", 6,191 glyphs laid of 71 bounded, on both
/// properties.
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
        crate::flow::MOST_RING_LAY.with(|most| most.set(0));
        let p = Projected::new(doc);
        if let Err(why) = rings_within_the_extent(&p, &mut reached) {
            past.push(format!("{what}: {why}"));
        }
        let most = crate::flow::MOST_RING_LAY.with(std::cell::Cell::get);
        if let Err(why) = rings_within_their_work(&p, most) {
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

/// A map of one node `across` wide and high, of `shape`, its frame one
/// `ring` `inset` out.
fn ringed(across: f64, shape: &serde_json::Value, inset: f64, ring: &serde_json::Value) -> serde_json::Value {
    one_node(json!({
        "min_size": [across, across],
        "shape": shape,
        "frame": {"inset": inset, "gap": 0.0, "rings": [ring.clone()]},
    }))
}

/// The densest `Repeat` rings the bound admits (ruling B47.3), each at the
/// edge of [`MOST_RING_GLYPHS`], the furthest whole `inset` it admits:
/// `(name, map, the same ring a unit further out)`. `·` at a quarter of a
/// node 1 across, the least size, round an ellipse, a sharp and a rounded
/// rectangle; `MANDALA ` the same way round an ellipse; `*` at a twentieth
/// of the largest node it is admitted round at all, 7 across; `·` spaced,
/// and a ring of `|` laid by direction spaced, whose `spacing` is what
/// bounds their glyphs; and the two of a fallback face the second sweep
/// found most steps a glyph in (`RING_STEPS_A_GLYPH`): `😀·` at a twentieth
/// of an ellipse 1,000 across, the most steps a lay, and `·\u{1BCA0}`, the
/// review of #378's, round the largest node it is admitted round, 9
/// across. Their faces are the compiled ones, so the edges are the same on
/// every host.
fn densest_admitted_rings() -> Vec<(String, serde_json::Value, serde_json::Value)> {
    let ellipse = json!("ellipse");
    let dots = json!({"fit": "repeat", "size": 0.25, "spans": [{"text": "·"}]});
    let rows = [
        ("`·` round an ellipse", 1.0, ellipse.clone(), dots.clone(), 1.0),
        (
            "`·` round a sharp rectangle",
            1.0,
            json!({"rectangle": {"radius": 0}}),
            dots.clone(),
            1.0,
        ),
        (
            "`·` round a rounded rectangle",
            1.0,
            json!({"rectangle": {"radius": 30}}),
            dots,
            1.0,
        ),
        (
            "`MANDALA ` round an ellipse",
            1.0,
            ellipse.clone(),
            json!({"fit": "repeat", "size": 0.25, "spans": [{"text": "MANDALA "}]}),
            4.0,
        ),
        (
            "`*` round an ellipse 7 across",
            7.0,
            ellipse.clone(),
            json!({"fit": "repeat", "size": 0.05, "spans": [{"text": "*"}]}),
            0.0,
        ),
        (
            "`·` spaced round an ellipse",
            1.0,
            ellipse.clone(),
            json!({"fit": "repeat", "size": 0.25, "spacing": 0.3, "spans": [{"text": "·"}]}),
            6.0,
        ),
        (
            "a ring of `|` by direction, spaced, round an ellipse",
            1.0,
            ellipse.clone(),
            json!({"fit": "repeat", "size": 0.25, "spacing": 2.0, "vertical": "|"}),
            28.0,
        ),
        (
            "`😀·` round an ellipse 1,000 across",
            1000.0,
            ellipse.clone(),
            json!({"fit": "repeat", "size": 0.05, "spans": [{"text": "😀·"}]}),
            63.0,
        ),
        (
            "`·\\u{1BCA0}` round an ellipse 9 across",
            9.0,
            ellipse,
            json!({"fit": "repeat", "size": 0.05, "spans": [{"text": "·\u{1BCA0}"}]}),
            0.0,
        ),
    ];
    rows.into_iter()
        .map(|(name, across, shape, ring, inset)| {
            (
                format!("{name}, {inset} out"),
                ringed(across, &shape, inset, &ring),
                ringed(across, &shape, inset + 1.0, &ring),
            )
        })
        .collect()
}

/// The maps the review of #378 measured under ASan, inside the bound before
/// ruling B47.3, each a `Repeat` ring laid to or near the cap, and its time
/// of a check (`cargo fuzz run load_raw <file> -- -runs=1`).
fn measured_rings() -> Vec<(&'static str, serde_json::Value)> {
    let ellipse = json!("ellipse");
    let dots = json!({"fit": "repeat", "size": 0.25, "spans": [{"text": "·"}]});
    let bars = json!({"fit": "repeat", "size": 0.25, "vertical": "|"});
    let extent = f64::from(EXTENT) - 1.0;
    vec![
        (
            "dots round an ellipse out to the extent, 5.9 s",
            ringed(1.0, &ellipse, extent, &dots),
        ),
        (
            "a ring of `|` out to the extent, 66 s",
            ringed(1.0, &ellipse, extent, &bars),
        ),
        (
            "the review's 8,000 ellipse, one 1e-6 ring of `|`, 64 s",
            one_node(json!({
                "position": [-4000.0, -4000.0],
                "min_size": [8000.0, 8000.0],
                "shape": "ellipse",
                "frame": {"inset": 0.0, "gap": 0.0, "rings": [{"fit": "repeat", "size": 0.000001, "vertical": "|"}]},
            })),
        ),
        (
            "dots 1,300 out, 52,210 glyphs, 40 s",
            ringed(1.0, &ellipse, 1300.0, &dots),
        ),
        (
            "a ring of `|` 3,050 out, 118 s",
            ringed(1.0, &ellipse, 3050.0, &bars),
        ),
    ]
}

/// The bound's arithmetic (ruling B47.3): a ring of [`MOST_RING_GLYPHS`]
/// glyphs, at [`RING_STEPS_A_GLYPH`] a glyph and [`RING_SETUP_STEPS`], laid
/// [`RING_LAYS_A_CHECK`] times, fits `INPUT_STEPS`, and one of a glyph more
/// does not.
#[test]
fn test_the_most_ring_glyphs_fill_the_inputs_share() {
    let lays = |glyphs: usize| RING_LAYS_A_CHECK * (RING_STEPS_A_GLYPH * glyphs + RING_SETUP_STEPS);
    assert!(
        lays(MOST_RING_GLYPHS) <= INPUT_STEPS,
        "{} steps",
        lays(MOST_RING_GLYPHS)
    );
    assert!(
        lays(MOST_RING_GLYPHS + 1) > INPUT_STEPS,
        "{} steps",
        lays(MOST_RING_GLYPHS + 1)
    );
}

/// Every one of the densest `Repeat` rings the bound admits
/// ([`densest_admitted_rings`]), projected, lays no more glyphs than the
/// bound read for it and takes no more than [`RING_STEPS_A_GLYPH`] a glyph
/// of that and [`RING_SETUP_STEPS`] (`LAST_RING`, `LAST_DIRECTIONAL`;
/// ruling B47.3), as every check of a fuzz run holds it
/// ([`rings_within_their_work`]), and draws within the extent
/// ([`rings_within_the_extent`]); each a unit further out is outside. The
/// rings the review measured under ASan ([`measured_rings`]) and the
/// three loader findings of a ring far out are outside, so none is laid.
///
/// Under ASan (`cargo fuzz run load_raw <file> -- -runs=1`, the load
/// average some 2), against `FUZZ_TIMEOUT_SECONDS`, 20 s, each input's
/// run as libFuzzer reports it: `😀·` round an ellipse 1,000 across, the
/// most steps a lay, 0.87 s; `·\u{1BCA0}`, 0.50 s; `·` round an ellipse,
/// 0.15 s.
///
/// Controls, each run and seen red: with no `Repeat` ring's work bounded,
/// every measured ring inside; with the most glyphs halved, "`·` round an
/// ellipse, 1 out" outside; with a ring's `spacing` not read, "`·` spaced
/// round an ellipse, 6 out" outside; with a ring by direction stepped by
/// its advance, the ring of `|` a unit further out inside; with a copy's
/// advance read double, and with no `RING_STEP_SLACK`, "`·` round an
/// ellipse" a unit further out inside; with a quarter of
/// `RING_STEPS_A_GLYPH` held here, and with a quarter in the check's own
/// property, `😀·`, 34,987 steps a lay of 17,728 allowed; with no reach in
/// a ring's perimeter, the dots out to the extent and 1,300 out inside.
#[test]
fn test_every_repeat_ring_the_bound_admits_fits_the_share() {
    use crate::flow::{LAST_DIRECTIONAL, LAST_RING};
    let findings: [(&str, &[u8]); 3] = [
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
        (
            "the finding of #382",
            include_bytes!(
                "../../document/tests/fuzz_findings/a_ring_unspaced_too_far_out_to_project_in_time.mandala.json"
            ),
        ),
    ];
    let mut outside: Vec<(String, crate::document::ids::Document)> = measured_rings()
        .into_iter()
        .map(|(what, map)| (what.to_string(), load_map(map)))
        .collect();
    for (what, bytes) in findings {
        let text = std::str::from_utf8(bytes).expect("the finding is text");
        outside.push((
            what.to_string(),
            crate::document::format::load(text).expect("the finding loads"),
        ));
    }
    let admitted: Vec<String> = outside
        .iter()
        .filter(|(_, doc)| within_bound(doc))
        .map(|(what, _)| what.clone())
        .collect();
    assert!(admitted.is_empty(), "inside the bound: {admitted:?}");
    let grid = densest_admitted_rings();
    let mut reached = Reached::default();
    for (what, map, further) in &grid {
        assert!(
            !within_bound(&load_map(further.clone())),
            "{what}, a unit further out, is inside"
        );
        let doc = load_map(map.clone());
        assert!(within_bound(&doc), "{what} is outside");
        let frame = doc
            .tree()
            .descendants()
            .find(|&id| matches!(doc.tree().arena()[id].get(), DocElement::Frame(_)))
            .unwrap_or_else(|| panic!("{what}: a frame"));
        let ring = frame
            .children(doc.tree().arena())
            .next()
            .unwrap_or_else(|| panic!("{what}: a ring"));
        let bound =
            repeat_glyphs(doc.tree(), frame, ring).unwrap_or_else(|| panic!("{what}: a `Repeat` ring"));
        LAST_RING.with(|last| last.set(None));
        LAST_DIRECTIONAL.with(|last| last.set(None));
        crate::flow::MOST_RING_LAY.with(|most| most.set(0));
        let projected = Projected::new(doc);
        rings_within_the_extent(&projected, &mut reached).unwrap_or_else(|why| panic!("{what}: {why}"));
        // What every check of a fuzz run holds the ring to.
        let most = crate::flow::MOST_RING_LAY.with(std::cell::Cell::get);
        rings_within_their_work(&projected, most).unwrap_or_else(|why| panic!("{what}: {why}"));
        drop(projected);
        let (placed, spent) = LAST_RING
            .with(std::cell::Cell::get)
            .map(|ring| (ring.placed, ring.spent))
            .or_else(|| LAST_DIRECTIONAL.with(std::cell::Cell::get))
            .unwrap_or_else(|| panic!("{what}: a ring is laid"));
        eprintln!("{what}: {placed} glyphs of {bound} bounded, {spent} steps");
        assert!(bound <= MOST_RING_GLYPHS, "{what}: {bound} glyphs bounded");
        assert!(placed <= bound, "{what}: {placed} glyphs laid, {bound} bounded");
        assert!(
            spent <= RING_STEPS_A_GLYPH * bound + RING_SETUP_STEPS,
            "{what}: {spent} steps, past {RING_STEPS_A_GLYPH} a glyph of {bound} and {RING_SETUP_STEPS}"
        );
    }
}

/// A map of one ellipse node `across` wide and high, about the origin, its
/// frame one ring of `text` laid `fit` at `size`, with `sides` its side
/// ornament where it has one.
fn framed_text(
    across: f64,
    fit: serde_json::Value,
    size: f64,
    text: &str,
    sides: Option<&str>,
) -> serde_json::Value {
    let mut ring = json!({"fit": fit, "size": size, "spans": [{"text": text}]});
    if let Some(sides) = sides {
        ring["sides"] = json!({"content": sides});
    }
    one_node(json!({
        "position": [-across / 2.0, -across / 2.0],
        "min_size": [across, across],
        "shape": "ellipse",
        "frame": {"inset": 0.0, "gap": 0.0, "rings": [ring]},
    }))
}

/// Text of `n` characters, `pattern` repeated.
fn text_of(pattern: &str, n: usize) -> String {
    pattern.chars().cycle().take(n).collect()
}

/// The frames laid by halves the bound admitted before ruling B47.3 held a
/// frame's work to the lays a check makes, each measured under ASan, its
/// time of a check (`cargo fuzz run load_raw <file> -- -runs=1`), three
/// lays: an ornament of 60 characters on 80 letters laid `fit`, and on 151
/// characters of seven scripts laid `once`, round an ellipse 1,000 across.
fn measured_frames() -> Vec<(&'static str, serde_json::Value)> {
    let heavy = text_of("✦你😀Aب", 60);
    vec![
        (
            "80 letters `fit`, a 60-character ornament, 18.4 s",
            framed_text(
                1000.0,
                json!({"fit": {"min": 0.25, "max": 4.0}}),
                0.05,
                &text_of("MANDALA mindmap glyph ring text ", 80),
                Some(&heavy),
            ),
        ),
        (
            "151 characters of seven scripts `once`, a 60-character ornament, 6.0 s",
            framed_text(
                1000.0,
                json!({"once": {"filler": "·"}}),
                0.05,
                &text_of("Aא你😀ب Ωж", 151),
                Some(&heavy),
            ),
        ),
    ]
}

/// The densest frames laid by halves the bound admits (ruling B47.3), by
/// the steps their lay took in the sweep `FRAME_STEPS_A_GLYPH` is stated
/// from: 151 letters `spread` round an ellipse 1,000 across, the most steps
/// a lay; 151 characters of seven scripts `once` with a `✦`, the slowest;
/// and 40 letters `spread` with a `✦` round one 100 across, the nearest its
/// bound.
fn densest_admitted_frames() -> Vec<(&'static str, serde_json::Value)> {
    let latin = text_of("MANDALA mindmap glyph ring text ", 151);
    vec![
        (
            "151 letters `spread`, 1,000 across",
            framed_text(1000.0, json!("spread"), 0.05, &latin, None),
        ),
        (
            "151 characters of seven scripts `once`, a `✦`, 100 across",
            framed_text(
                100.0,
                json!({"once": {"filler": "·"}}),
                0.5,
                &text_of("Aא你😀ب Ωж", 151),
                Some("✦"),
            ),
        ),
        (
            "40 letters `spread`, a `✦`, 100 across",
            framed_text(
                100.0,
                json!("spread"),
                0.5,
                &text_of("MANDALA mindmap glyph ring text ", 40),
                Some("✦"),
            ),
        ),
    ]
}

/// A frame laid by halves is laid once in each projection a check makes,
/// up to [`RING_LAYS_A_CHECK`], and its work is held to that (ruling
/// B47.3), not to the one lay the flow harnesses budget a frame for. Each
/// measured frame ([`measured_frames`]) took no more than the input's
/// share in one lay, which the old bound admitted it for, and takes more
/// in ten: the bound holds it outside. Each of the densest frames the
/// bound admits ([`densest_admitted_frames`]), projected, takes no more
/// than [`frame_steps`] read for it (`LAST_FRAME`), as every check of a
/// fuzz run holds it ([`rings_within_their_work`]), and ten of its lays
/// fit the share.
///
/// Controls, each run and seen red: with no frame's work bounded, the
/// first measured frame inside; with its side ornament's glyphs not
/// counted, the same; with one lay given the whole share, as the flow
/// harnesses give it, the same; with a quarter of
/// `FRAME_STEPS_A_SIDE_GLYPH`, the 40 letters with a `✦` past their bound;
/// with a third of `FRAME_STEPS_A_GLYPH`, the 151 letters past theirs;
/// with a quarter of [`frame_steps`] allowed in the check's own property,
/// the 151 letters, 22,872 steps a lay of 9,296 allowed.
#[test]
fn test_every_text_frame_the_bound_admits_fits_the_share() {
    use crate::flow::LAST_FRAME;
    let lay = |doc: crate::document::ids::Document, what: &str| {
        LAST_FRAME.with(|last| last.set(None));
        drop(Projected::new(doc));
        LAST_FRAME
            .with(std::cell::Cell::get)
            .unwrap_or_else(|| panic!("{what}: a frame is laid"))
            .1
    };
    for (k, (what, map)) in measured_frames().into_iter().enumerate() {
        let doc = load_map(map);
        assert!(!within_bound(&doc), "{what}: inside the bound");
        let spent = lay(doc, what);
        eprintln!("{what}: {spent} steps a lay");
        assert!(
            spent <= INPUT_STEPS,
            "{what}: {spent} steps a lay, past the share in one"
        );
        // The first is past the share in ten lays by its counted steps;
        // the second by its time, its shaping in many faces counted by
        // none (see `FRAME_STEPS_A_LAY`).
        if k == 0 {
            assert!(
                RING_LAYS_A_CHECK * spent > INPUT_STEPS,
                "{what}: ten lays of {spent} steps within the share"
            );
        }
    }
    for (what, map) in densest_admitted_frames() {
        let doc = load_map(map);
        assert!(within_bound(&doc), "{what}: outside the bound");
        let frame = doc
            .tree()
            .descendants()
            .find(|&id| matches!(doc.tree().arena()[id].get(), DocElement::Frame(_)))
            .unwrap_or_else(|| panic!("{what}: a frame"));
        let ring = frame
            .children(doc.tree().arena())
            .next()
            .unwrap_or_else(|| panic!("{what}: a ring"));
        let bound = frame_steps(doc.tree(), frame, ring).unwrap_or_else(|| panic!("{what}: laid by halves"));
        LAST_FRAME.with(|last| last.set(None));
        crate::flow::MOST_RING_LAY.with(|most| most.set(0));
        let projected = Projected::new(doc);
        // What every check of a fuzz run holds the frame to.
        let most = crate::flow::MOST_RING_LAY.with(std::cell::Cell::get);
        rings_within_their_work(&projected, most).unwrap_or_else(|why| panic!("{what}: {why}"));
        drop(projected);
        let spent = LAST_FRAME
            .with(std::cell::Cell::get)
            .unwrap_or_else(|| panic!("{what}: a frame is laid"))
            .1;
        eprintln!("{what}: {spent} steps a lay, {bound} bounded");
        assert!(spent <= bound, "{what}: {spent} steps a lay, {bound} bounded");
        assert!(
            RING_LAYS_A_CHECK * spent <= INPUT_STEPS,
            "{what}: ten lays of {spent} steps past the share"
        );
    }
}

/// Every [`DEFAULT_IGNORABLE`] code point the bound reads as advancing by
/// nothing is laid at no advance by the shaper (the review of #378): each
/// shaped between two `a`s in the default face, on this host's faces, the
/// run is no wider than the two `a`s alone, though some faces map these
/// code points to glyphs a whole em wide (U+180E, U+17B4, the tags). And
/// the bound reads them so: `narrowest_char` gives each nothing.
///
/// Controls, run and seen red: with the whole of Unicode's property in the
/// table, the Hangul fillers and the shorthand format controls laid an em
/// wide; with U+180E read at the narrowest glyph a face maps it to, red on
/// its advance read.
#[test]
fn test_the_default_ignorables_are_laid_at_no_advance() {
    use crate::document::resolve::ResolvedText;
    use crate::document::values::{FontSize, Rgba, Weight};
    use crate::flow::{shape_run, FlowSpan};
    use crate::font::fonts::{AppFont, FontSystemLease};
    use crate::projection::tests::projection_fuzz::{face_metrics, DEFAULT_IGNORABLE};
    crate::font::fonts::init();
    // Read before the lease below is taken: it takes one of its own.
    let faces = face_metrics();
    let style = ResolvedText {
        font: AppFont::Any,
        size: FontSize::Pt(100.0),
        color: Rgba::opaque(255, 255, 255),
        weight: Weight::NORMAL,
        italic: false,
    };
    let mut fs = FontSystemLease::new("the default ignorables");
    let mut width = |text: &str| -> f32 {
        let span = FlowSpan {
            text,
            style: &style,
            link: None,
        };
        shape_run(&[span], 1.0, &mut fs)
            .glyphs
            .iter()
            .map(|g| g.advance)
            .sum()
    };
    let plain = width("aa");
    let mut wide = Vec::new();
    let mut read = 0;
    for &(first, last) in &DEFAULT_IGNORABLE {
        for code in first..=last {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            read += 1;
            let laid = width(&format!("a{c}a"));
            if laid > plain + 1e-3 {
                wide.push(format!("U+{code:04X}: {laid} against {plain}"));
            }
            assert_eq!(
                faces.narrowest_char(AppFont::Any, c),
                0.0,
                "U+{code:04X} read as advancing"
            );
        }
    }
    assert!(read > 4_000, "{read} code points read");
    assert!(wide.is_empty(), "laid wider than nothing: {wide:#?}");
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
