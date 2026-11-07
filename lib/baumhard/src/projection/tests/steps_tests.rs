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

use crate::document::element::PaletteData;
use crate::document::tests::test_support::{palette_group, DocumentBuilder};
use crate::document::resolve::climb_steps;
use crate::projection::tests::support::Projected;

/// How deep the chain goes: deep enough that a climb per node is a
/// hundred thousand steps, shallow enough to shape a section at every
/// node quickly.
const DEPTH: usize = 400;

/// A chain of [`DEPTH`] nodes below a root bound to a palette, every node
/// holding a section of a span that sets no color: each one's color is
/// the palette's, from a binding up to [`DEPTH`] levels above it.
fn chain() -> crate::document::ids::Document {
    let mut b = DocumentBuilder::new();
    let palette = b.palette(PaletteData {
        name: "far".into(),
        groups: vec![palette_group(10), palette_group(11)],
    });
    let root = b.root();
    let top = b.node(root);
    b.edit_node(top, |n| n.palette = Some(palette));
    let mut at = top;
    for _ in 0..DEPTH {
        at = b.node(at);
        let section = b.section(at, 0);
        b.span(section, "x");
    }
    b.build().unwrap()
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
    assert_eq!(p.projector.nodes.len(), DEPTH + 1, "the chain is projected");
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
