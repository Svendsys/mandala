// SPDX-License-Identifier: MPL-2.0

//! The projection carries the palette binding down its walk (ruling R16
//! on #282, B7.2 on #241): projecting a deep chain whose every node's
//! text reads a binding far above it climbs nothing, and the dependency
//! index holds a number of reads linear in the nodes. `cfg(test)`: it
//! reads the climb's test-only step counter.
//!
//! The negative control each owes plants the per-node climb — every
//! resolution given `Binding::Climb` in place of the carried binding —
//! and sees both go red: the climb steps grow with the square of the
//! depth, and so does the index, one `palette` read per ancestor.
//!
//! A frame's rings are the same shape sideways: each clears the rings
//! inside it, and reads only the one just inside it, whose extent
//! carries the rest. Its index is linear in the rings (the control reads
//! every ring inside, and goes red), and growing the innermost ring moves
//! every ring outside it (the control marks no ring outside, and the
//! oracle goes red).

use serde_json::json;

use crate::document::resolve::climb_steps;
use crate::projection::tests::maps::{load_map, node};
use crate::projection::tests::support::Projected;

/// How deep the chain goes: deep enough that a climb per node is eighty
/// thousand steps, shallow enough to shape a section at every node
/// quickly.
const DEPTH: u32 = 400;

/// A chain of [`DEPTH`] nodes below a root bound to a palette, every node
/// holding a section of a span that sets no color: each one's color is
/// the palette's, from a binding up to [`DEPTH`] levels above it.
fn chain() -> crate::document::ids::Document {
    let mut nodes = vec![node(1, None, json!({"palette": "far"}))];
    for id in 2..=DEPTH + 1 {
        nodes.push(node(
            id,
            Some(id - 1),
            json!({"connector": {}, "sections": [{"spans": [{"text": "x"}]}]}),
        ));
    }
    load_map(json!({
        "palettes": {"far": [
            {"frame": "#000a00", "text": "#00000a"},
            {"frame": "#000b00", "text": "#00000b"}
        ]},
        "nodes": nodes
    }))
}

#[test]
fn test_projection_carries_the_binding_and_climbs_nothing() {
    let doc = chain();
    let before = climb_steps::taken();
    let p = Projected::new(doc);
    let climbed = climb_steps::taken() - before;
    assert_eq!(
        climbed, 0,
        "every node's binding is carried down from its parent's, never climbed for"
    );
    assert_eq!(
        p.projector.nodes.len(),
        DEPTH as usize + 1,
        "the chain is projected"
    );
}

#[test]
fn test_projection_the_dependency_index_is_linear_in_a_deep_chain() {
    let p = Projected::new(chain());
    let reads = p.projector.dependency_count();
    let units = p.projector.units.len() + p.projector.nodes.len();
    // Each node's carry, measure, connector and section read a handful of
    // inputs each: nothing per ancestor.
    assert!(
        reads <= 40 * units,
        "{reads} reads for {units} nodes and units: linear in the chain, not in its square"
    );
    assert!(reads >= units, "the index holds the chain's reads: {reads}");
}

/// How many rings the many-ringed frame holds: enough that each ring
/// reading every ring inside it is some forty-five thousand reads.
const RINGS: u32 = 300;

/// A node framed by `rings` rings, each on its own channel; `text` is
/// each ring's one span, or none.
fn framed(rings: u32, text: Option<&str>) -> crate::document::ids::Document {
    let rings: Vec<_> = (0..rings)
        .map(|channel| {
            let mut ring = json!({"channel": channel, "fit": "repeat", "size": 0.06});
            if let Some(text) = text {
                ring["spans"] = json!([{"text": text}]);
            }
            ring
        })
        .collect();
    load_map(json!({"nodes": [node(1, None, json!({
        "min_size": [200.0, 100.0],
        "sections": [{"spans": [{"text": "framed"}]}],
        "frame": {"gap": 2.0, "rings": rings}
    }))]}))
}

#[test]
fn test_projection_a_frames_rings_read_one_ring_each() {
    let p = Projected::new(framed(RINGS, None));
    let reads = p.projector.dependency_count();
    let units = p.projector.units.len() + p.projector.nodes.len();
    assert!(units > RINGS as usize, "every ring is a unit: {units}");
    // Each ring reads its node, its frame's row and fields, its own
    // fields and the one ring just inside it: nothing per ring inside.
    assert!(
        reads <= 40 * units,
        "{reads} reads for {units} units: linear in the rings, not in their square"
    );
}

#[test]
fn test_projection_an_inner_ring_grown_moves_every_ring_outside_it() {
    use crate::document::element::DocKind;
    use crate::document::mutation::ops::{DocMutation, RingOp, TextOp};
    use crate::document::tests::writer_support::{request, single};
    use crate::document::values::FontSize;
    use crate::projection::state::UnitBody;
    use crate::projection::tests::maps::{ident, kid, kids};

    let doc = framed(3, Some("·"));
    let rings = kids(&doc, kid(&doc, ident(&doc, 1), DocKind::Frame), DocKind::Ring);
    let extent = |p: &Projected, ring| match &p.projector.units[&ring].body {
        UnitBody::Ring(state) => state.extent,
        _ => unreachable!("a ring's unit"),
    };
    let mut p = Projected::new(doc);
    let before: Vec<_> = rings.iter().map(|&r| extent(&p, r)).collect();
    p.apply(request(
        single(
            DocKind::Ring,
            DocMutation::Ring(RingOp::Text(TextOp::SetSize(Some(FontSize::Rel(0.2))))),
        ),
        vec![rings[0]],
        "grow the innermost",
    ))
    .unwrap();
    for (at, &ring) in rings.iter().enumerate() {
        assert_ne!(
            extent(&p, ring),
            before[at],
            "ring {at} clears a taller innermost ring"
        );
    }
    p.check().unwrap();
}
