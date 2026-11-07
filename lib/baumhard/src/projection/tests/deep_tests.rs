// SPDX-License-Identifier: MPL-2.0

//! The projection of a document as deep as memory allows (design D30,
//! §20 rule 11): a chain of [`DEEP`] nodes projected in full, edited at
//! its leaf, hidden and shown again whole, moved whole, held to the
//! oracle and hit at its leaf — every walk the projector makes over it
//! iterative, on a default test thread and through the wasm32 node probe
//! (`tests/wasm_font_lock.rs`) on wasm32's default stack.
//!
//! The chain's connectors are not drawn: what is under test is the walks,
//! and a hundred thousand connectors shaped and placed would measure
//! shaping. Its palette is bound at the top and its text is at the leaf,
//! so the leaf's color is carried a hundred thousand levels down.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2).

use glam::Vec2;

use crate::arena::NodeId;
use crate::document::element::{DocElement, DocKind, PaletteData};
use crate::document::ids::Document;
use crate::document::mutation::compose::reparent;
use crate::document::mutation::ops::{DocMutation, NodeOp, SpanOp};
use crate::document::tests::deep_tests::DEEP;
use crate::document::tests::test_support::{palette_group, DocumentBuilder};
use crate::document::tests::writer_support::{request, single};
use crate::document::values::Rgba;
use crate::document::writer::EditRequest;
use crate::projection::state::UnitBody;
use crate::projection::tests::support::Projected;

/// The chain: its document, the node at its top, a second root, the leaf
/// and the leaf's section and span.
pub struct DeepChain {
    /// The document.
    pub doc: Document,
    /// The top of the chain, below the root node bound to the palette.
    pub top: NodeId,
    /// A second root node.
    pub other: NodeId,
    /// The leaf.
    pub leaf: NodeId,
    /// The leaf's section.
    pub section: NodeId,
    /// The leaf's span.
    pub span: NodeId,
}

/// A chain `depth` nodes deep below a root node bound to a palette, each
/// connector hidden, the leaf holding a section of one span that sets no
/// color. Built by appending: building it climbs nothing.
pub fn deep_chain(depth: usize) -> DeepChain {
    let mut b = DocumentBuilder::new();
    let palette = b.palette(PaletteData {
        name: "deep".into(),
        groups: vec![palette_group(10), palette_group(11), palette_group(12)],
    });
    let root = b.root();
    let holder = b.node(root);
    b.edit_node(holder, |n| n.palette = Some(palette));
    let other = b.node(root);
    b.edit_node(other, |n| n.position = Vec2::new(-500.0, 0.0));
    let mut at = holder;
    let mut top = None;
    for _ in 0..depth {
        at = b.node(at);
        top.get_or_insert(at);
        let connector = crate::document::tests::test_support::connector_of(&b, at);
        b.edit(connector, |e| {
            if let DocElement::Connector(c) = e {
                c.path.visible = false;
            }
        });
    }
    let section = b.section(at, 0);
    let span = b.span(section, "the deepest text");
    DeepChain {
        doc: b.build().unwrap(),
        top: top.unwrap_or(holder),
        other,
        leaf: at,
        section,
        span,
    }
}

/// The color the leaf's section was shaped in.
fn leaf_color(p: &Projected, section: NodeId) -> Rgba {
    match &p.projector.units[&section].body {
        UnitBody::Section(s) => s.flow.as_ref().unwrap().key.spans[0].1.color,
        _ => unreachable!(),
    }
}

#[test]
fn test_projection_deep_chain_projects_edits_hides_moves_and_hits() {
    do_projection_deep_chain_projects_edits_hides_moves_and_hits();
}

/// [`DEEP`] levels: projected in full, the leaf carrying the palette's
/// last group down from the top (the clamp, design §5.4); its text edited
/// and only it re-projected; the chain hidden from its top — every group
/// below it out of every layer — and shown again; moved whole under
/// another root; the layers held to a full projection; the leaf hit.
pub fn do_projection_deep_chain_projects_edits_hides_moves_and_hits() {
    let chain = deep_chain(DEEP);
    let (top, other, leaf, section, span) = (chain.top, chain.other, chain.leaf, chain.section, chain.span);
    let mut p = Projected::new(chain.doc);
    assert_eq!(p.projector.nodes.len(), DEEP + 2, "every node of the chain is projected");
    assert_eq!(
        leaf_color(&p, section),
        Rgba::opaque(12, 12, 0),
        "the leaf takes the last group's title, carried down the chain"
    );
    p.apply(request(
        single(DocKind::Span, DocMutation::Span(SpanOp::SetText("edited at the leaf".into()))),
        vec![span],
        "edit",
    ))
    .unwrap();
    let hide = |on: bool| request(single(DocKind::Node, DocMutation::Node(NodeOp::SetHidden(on))), vec![top], "hide");
    p.apply(hide(true)).unwrap();
    assert!(p.projector.glyphs_of(leaf).is_empty(), "hidden from the top, the leaf has no group");
    assert_eq!(p.projector.nodes.len(), 2, "only the root nodes are left");
    p.apply(hide(false)).unwrap();
    assert!(!p.projector.glyphs_of(leaf).is_empty(), "shown again");
    let steps = reparent(&p.doc, &[top], other).unwrap();
    p.apply(EditRequest::new(steps, "move the chain")).unwrap();
    assert_eq!(p.projector.nodes[&leaf].depth, DEEP, "the leaf is as deep below its new root");
    p.check().unwrap();
    let rect = p.projector.node_rect(leaf).unwrap();
    let hit = p.projector.hit(&mut p.layers, rect.origin + Vec2::splat(2.0));
    assert_eq!(hit.map(|h| h.node), Some(leaf), "the leaf is hit");
}
