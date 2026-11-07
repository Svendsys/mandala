// SPDX-License-Identifier: MPL-2.0

//! The dependency index, one test per kind of dependency (#241, Tests;
//! design §12.3), each asserting on what the update wrote
//! ([`LayerChanges`]) and on what it ran, beside an element the change
//! must not reach — the negative control each assertion owes is the
//! untouched element's, and every test ends on the oracle.

use std::sync::Arc;

use glam::Vec2;
use rustc_hash::FxHashSet;

use crate::arena::NodeId;
use crate::document::element::{
    DefaultsData, DocElement, DocKind, FrameData, PaletteData, RingData, RingStyle, TextStyle, ThemeData, ThemeVariant,
};
use crate::document::ids::Document;
use crate::document::mutation::ops::{DefaultsOp, DocMutation, NodeOp, SpanOp, TextOp, ThemeOp};
use crate::document::tests::test_support::{palette_group, DocumentBuilder};
use crate::document::tests::writer_support::{request, single};
use crate::document::values::{ColorRef, FitMode, FontSize, LinkKind, PaletteId, Rgba, VarId};
use crate::document::writer::{ChangeSource, EditRequest};
use crate::engine::journal::Touch;
use crate::font::fonts::FontSystemLease;
use crate::gfx_structs::run::RunShape;
use crate::projection::state::UnitBody;
use crate::projection::tests::support::Projected;
use crate::projection::{Layer, LayerChanges, Stage};

/// The document elements whose runs `changes` touched in `layer`, as the
/// projector maps them back.
fn touched(p: &Projected, changes: &LayerChanges, layer: Layer) -> FxHashSet<NodeId> {
    changes
        .journal(layer)
        .touched()
        .filter_map(|(glyph, _)| p.projector.element_of(layer, glyph))
        .collect()
}

/// The units a stage of which ran in the last call.
fn ran(p: &Projected, stage: Stage) -> FxHashSet<NodeId> {
    p.projector
        .ran
        .iter()
        .filter(|d| d.stage == stage)
        .map(|d| d.unit)
        .collect()
}

/// Apply `req` and sync, returning the layer changes (not discarded).
fn apply(p: &mut Projected, req: EditRequest) -> LayerChanges {
    p.doc.apply(req, ChangeSource::Action).unwrap();
    p.sync()
}

/// A node at `x` under `parent` with one section holding one span of
/// `text`; returns the node, the section and the span.
fn node_with_text(b: &mut DocumentBuilder, parent: NodeId, x: f32, text: &str) -> (NodeId, NodeId, NodeId) {
    let node = b.node(parent);
    b.edit_node(node, |n| n.position = Vec2::new(x, 0.0));
    let section = b.section(node, 0);
    let span = b.span(section, text);
    (node, section, span)
}

/// Set the span's own color.
fn color_span(b: &mut DocumentBuilder, span: NodeId, color: ColorRef) {
    b.edit(span, |e| {
        if let DocElement::Span(s) = e {
            s.style.color = Some(color);
        }
    });
}

/// A theme with one variable, `accent`, different in two variants.
fn two_variant_theme() -> ThemeData {
    ThemeData {
        variables: vec!["accent".into()],
        variants: vec![
            ThemeVariant {
                name: "dark".into(),
                values: vec![Rgba::opaque(200, 100, 0)],
            },
            ThemeVariant {
                name: "light".into(),
                values: vec![Rgba::opaque(10, 20, 30)],
            },
        ],
        active: 0,
    }
}

#[test]
fn test_projection_a_theme_switch_reaches_exactly_the_readers_of_a_variable() {
    let mut b = DocumentBuilder::new();
    b.theme(two_variant_theme());
    let root = b.root();
    let (_, themed, themed_span) = node_with_text(&mut b, root, 0.0, "themed");
    color_span(&mut b, themed_span, ColorRef::Var(VarId(0)));
    let (_, literal, literal_span) = node_with_text(&mut b, root, 300.0, "literal");
    color_span(&mut b, literal_span, ColorRef::Rgba(Rgba::opaque(1, 2, 3)));
    let (_, plain, _) = node_with_text(&mut b, root, 600.0, "plain");
    let theme = b.theme_id();
    let mut p = Projected::new(b.build().unwrap());
    let changes = apply(
        &mut p,
        request(single(DocKind::Theme, DocMutation::Theme(ThemeOp::SetActive(1))), vec![theme], "switch"),
    );
    let sections = touched(&p, &changes, Layer::Sections);
    assert!(sections.contains(&themed), "the section that reads the variable: {sections:?}");
    assert!(!sections.contains(&literal), "a pure-Rgba section is not touched");
    assert!(!sections.contains(&plain), "nor one that reads no variable");
    let resolved = ran(&p, Stage::Resolve);
    assert_eq!(resolved, [themed].into_iter().collect(), "only the reader resolves again");
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_a_parents_section_text_moves_its_childs_connector() {
    let mut b = DocumentBuilder::new();
    let root = b.root();
    let (parent, _, parent_span) = node_with_text(&mut b, root, 0.0, "short");
    let child = b.node(parent);
    b.edit_node(child, |n| n.position = Vec2::new(0.0, 300.0));
    let connector = crate::document::tests::test_support::connector_of(&b, child);
    let (other, _, _) = node_with_text(&mut b, root, 900.0, "elsewhere");
    let other_child = b.node(other);
    b.edit_node(other_child, |n| n.position = Vec2::new(900.0, 300.0));
    let other_connector = crate::document::tests::test_support::connector_of(&b, other_child);
    let mut p = Projected::new(b.build().unwrap());
    let changes = apply(
        &mut p,
        request(
            single(
                DocKind::Span,
                DocMutation::Span(SpanOp::SetText("a far longer text\nover two lines".into())),
            ),
            vec![parent_span],
            "text",
        ),
    );
    let connections = touched(&p, &changes, Layer::Connections);
    assert!(connections.contains(&connector), "the child's connector moved: {connections:?}");
    assert!(!connections.contains(&other_connector), "another node's connector did not");
    assert!(
        !ran(&p, Stage::Resolve).contains(&connector),
        "the connector re-placed, and resolved nothing"
    );
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_a_defaults_change_reaches_only_its_inheritors() {
    let mut b = DocumentBuilder::new();
    let root = b.root();
    let (_, inheriting, _) = node_with_text(&mut b, root, 0.0, "inherits");
    let (_, own, own_span) = node_with_text(&mut b, root, 300.0, "its own");
    color_span(&mut b, own_span, ColorRef::Rgba(Rgba::opaque(9, 9, 9)));
    b.edit(own, |e| {
        if let DocElement::Section(s) = e {
            s.text.size = Some(FontSize::Pt(12.0));
        }
    });
    let defaults = b.defaults_id();
    let mut p = Projected::new(b.build().unwrap());
    let replaced = DefaultsData {
        section: TextStyle {
            color: Some(ColorRef::Rgba(Rgba::opaque(1, 2, 3))),
            size: Some(FontSize::Pt(30.0)),
            ..TextStyle::default()
        },
        ..DefaultsData::default()
    };
    let changes = apply(
        &mut p,
        request(
            single(DocKind::Defaults, DocMutation::Defaults(DefaultsOp::Replace(Box::new(replaced)))),
            vec![defaults],
            "defaults",
        ),
    );
    let sections = touched(&p, &changes, Layer::Sections);
    assert!(sections.contains(&inheriting), "the section that inherits: {sections:?}");
    assert!(!sections.contains(&own), "the one that sets both fields itself is not reached");
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_a_palette_binding_moved_to_another_ancestor_regroups_by_depth() {
    let mut b = DocumentBuilder::new();
    let palette = b.palette(PaletteData {
        name: "p".into(),
        groups: vec![palette_group(10), palette_group(11), palette_group(12)],
    });
    let root = b.root();
    let (top, _, _) = node_with_text(&mut b, root, 0.0, "top");
    b.edit_node(top, |n| n.palette = Some(palette));
    let (middle, _, _) = node_with_text(&mut b, top, 0.0, "middle");
    let (_, leaf_section, _) = node_with_text(&mut b, middle, 0.0, "leaf");
    let (_, apart, _) = node_with_text(&mut b, root, 600.0, "apart");
    let mut p = Projected::new(b.build().unwrap());
    let leaf_color = |p: &Projected| -> Rgba {
        let unit = &p.projector.units[&leaf_section];
        match &unit.body {
            UnitBody::Section(s) => s.flow.as_ref().unwrap().key.spans[0].1.color,
            _ => unreachable!(),
        }
    };
    assert_eq!(leaf_color(&p), Rgba::opaque(12, 12, 0), "two levels below the binding: group 2's title");
    let steps = [
        request(single(DocKind::Node, DocMutation::Node(NodeOp::SetPalette(None))), vec![top], "unbind"),
        request(
            single(DocKind::Node, DocMutation::Node(NodeOp::SetPalette(Some(PaletteId(0))))),
            vec![middle],
            "bind",
        ),
    ]
    .into_iter()
    .flat_map(|r| r.steps)
    .collect();
    let changes = apply(&mut p, EditRequest::new(steps, "move the binding"));
    assert_eq!(leaf_color(&p), Rgba::opaque(11, 11, 0), "one level below it now: group 1's");
    let sections = touched(&p, &changes, Layer::Sections);
    assert!(sections.contains(&leaf_section), "the leaf regrouped: {sections:?}");
    assert!(!sections.contains(&apart), "a node under neither is not reached");
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_hidden_removes_a_subtree_and_the_links_into_it() {
    let mut b = DocumentBuilder::new();
    let root = b.root();
    let (holder, _, _) = node_with_text(&mut b, root, 0.0, "holder");
    let (hidden, _, _) = node_with_text(&mut b, holder, 0.0, "hidden");
    let (below, _, _) = node_with_text(&mut b, hidden, 0.0, "below");
    let (outside, _, _) = node_with_text(&mut b, root, 600.0, "outside");
    let line = b.link(outside, below, LinkKind::Line);
    let portal = b.link(outside, hidden, LinkKind::Portal);
    let kept = b.link(outside, holder, LinkKind::Line);
    let mut p = Projected::new(b.build().unwrap());
    assert!(!p.projector.glyphs_of(line).is_empty(), "the fixture draws the link");
    let hide = |on: bool| request(single(DocKind::Node, DocMutation::Node(NodeOp::SetHidden(on))), vec![hidden], "hide");
    let changes = apply(&mut p, hide(true));
    for layer in Layer::ALL {
        let removed = changes
            .journal(layer)
            .touched()
            .filter(|&(_, t)| t == Touch::Removed)
            .count();
        assert!(removed >= 2, "{layer:?}: the hidden node's group and its child's leave ({removed})");
    }
    for node in [hidden, below] {
        assert!(p.projector.glyphs_of(node).is_empty(), "{node:?} has no group");
    }
    assert!(p.projector.glyphs_of(line).is_empty(), "the link into the subtree is gone");
    assert!(p.projector.glyphs_of(portal).is_empty(), "the portal to it too");
    assert!(!p.projector.glyphs_of(kept).is_empty(), "a link to a node still drawn stays");
    changes.discard(&mut p.layers);
    p.check().unwrap();
    let changes = apply(&mut p, hide(false));
    assert!(!p.projector.glyphs_of(below).is_empty(), "un-hiding restores the subtree");
    assert!(!p.projector.glyphs_of(line).is_empty(), "and the link");
    assert!(!p.projector.glyphs_of(portal).is_empty(), "and the portal");
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_an_ancestors_opacity_reaches_every_descendants_runs() {
    let mut b = DocumentBuilder::new();
    let root = b.root();
    let (top, top_section, _) = node_with_text(&mut b, root, 0.0, "top");
    let (middle, middle_section, _) = node_with_text(&mut b, top, 0.0, "middle");
    let (_, leaf_section, _) = node_with_text(&mut b, middle, 0.0, "leaf");
    let (_, apart, _) = node_with_text(&mut b, root, 600.0, "apart");
    let mut p = Projected::new(b.build().unwrap());
    let changes = apply(
        &mut p,
        request(single(DocKind::Node, DocMutation::Node(NodeOp::SetOpacity(0.5))), vec![top], "fade"),
    );
    let sections = touched(&p, &changes, Layer::Sections);
    for section in [top_section, middle_section, leaf_section] {
        assert!(sections.contains(&section), "{section:?} faded: {sections:?}");
        let opacity = p.projector.units[&section].slots[0].written.as_ref().unwrap().opacity();
        assert_eq!(opacity, 0.5, "at the ancestor's opacity");
    }
    assert!(!sections.contains(&apart), "a node beside the subtree is not");
    assert!(ran(&p, Stage::Resolve).is_empty(), "an opacity resolves nothing");
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

/// A node bound to a palette, with a ring and a child whose text
/// inherits the palette and whose own frame has a ring.
fn palette_subtree() -> (Document, NodeId, NodeId, NodeId) {
    let mut b = DocumentBuilder::new();
    let palette = b.palette(PaletteData {
        name: "p".into(),
        groups: vec![palette_group(10), palette_group(11)],
    });
    let root = b.root();
    let (top, _, _) = node_with_text(&mut b, root, 0.0, "the earth");
    b.edit_node(top, |n| n.palette = Some(palette));
    let frame = b.add(top, DocElement::Frame(FrameData::default()));
    let ring = b.add(
        frame,
        DocElement::Ring(RingData {
            channel: 0,
            style: RingStyle {
                fit: Some(FitMode::Repeat),
                ..RingStyle::default()
            },
        }),
    );
    b.span(ring, "·");
    let (_, child_section, _) = node_with_text(&mut b, top, 0.0, "without form");
    (b.build().unwrap(), top, child_section, ring)
}

/// The shaping `Arc` of a section's or ring's flow.
fn shaping(p: &Projected, unit: NodeId) -> usize {
    match &p.projector.units[&unit].body {
        UnitBody::Section(s) => Arc::as_ptr(&s.flow.as_ref().unwrap().shaped) as usize,
        UnitBody::Ring(r) => Arc::as_ptr(&r.flow.flow.as_ref().unwrap().shaped) as usize,
        _ => unreachable!(),
    }
}

#[test]
fn test_projection_a_drag_re_places_and_resolves_nothing() {
    let (doc, top, child_section, ring) = palette_subtree();
    let top_section = *p_sections(&doc, top).first().unwrap();
    let mut p = Projected::new(doc);
    let top_before = shaping(&p, top_section);
    let before = (shaping(&p, child_section), shaping(&p, ring));
    let changes = apply(
        &mut p,
        request(single(DocKind::Node, DocMutation::Node(NodeOp::Nudge(Vec2::new(40.0, 0.0)))), vec![top], "drag"),
    );
    assert!(
        ran(&p, Stage::Resolve).is_empty(),
        "a drag dirties no text resolution: {:?}",
        p.projector.ran
    );
    assert_eq!(
        (shaping(&p, child_section), shaping(&p, ring)),
        before,
        "and reshapes nothing"
    );
    assert!(
        !touched(&p, &changes, Layer::Frames).is_empty(),
        "the ring was re-placed"
    );
    changes.discard(&mut p.layers);
    p.check().unwrap();

    // The control: the dragged node's text size reshapes the text that
    // inherits it — its own sections' spans; a child node's text inherits
    // from that node, not from its parent.
    let changes = apply(
        &mut p,
        request(
            single(
                DocKind::Node,
                DocMutation::Node(NodeOp::Text(TextOp::SetSize(Some(FontSize::Pt(40.0))))),
            ),
            vec![top],
            "size",
        ),
    );
    assert!(ran(&p, Stage::Resolve).contains(&top_section), "the size is resolved again");
    assert_ne!(shaping(&p, top_section), top_before, "and the run that inherits it reshaped");
    assert_eq!(shaping(&p, child_section), before.0, "the child node's text inherits no node size");
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_a_move_reuses_shaping_and_a_resize_reshapes_only_relative_rings() {
    let (doc, top, child_section, ring) = palette_subtree();
    let top_section = *p_sections(&doc, top).first().unwrap();
    let mut p = Projected::new(doc);
    let before = (shaping(&p, top_section), shaping(&p, child_section), shaping(&p, ring));
    let changes = apply(
        &mut p,
        request(single(DocKind::Node, DocMutation::Node(NodeOp::SetPosition(Vec2::new(500.0, 80.0)))), vec![top], "move"),
    );
    assert_eq!(
        (shaping(&p, top_section), shaping(&p, child_section), shaping(&p, ring)),
        before,
        "a move reuses every shaping, pointer for pointer"
    );
    changes.discard(&mut p.layers);
    let changes = apply(
        &mut p,
        request(
            single(DocKind::Node, DocMutation::Node(NodeOp::SetMinSize(Some(Vec2::new(400.0, 300.0))))),
            vec![top],
            "resize",
        ),
    );
    assert_eq!(shaping(&p, top_section), before.0, "a resize keeps the section's shaping");
    assert_ne!(shaping(&p, ring), before.2, "and reshapes the ring, whose size is relative");
    assert!(
        matches!(
            p.projector.units[&ring].slots[0].written.as_ref().map(|r| r.shaped().clone()),
            Some(RunShape::Path { .. })
        ),
        "the ring is drawn"
    );
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

/// The sections of `node`.
fn p_sections(doc: &Document, node: NodeId) -> Vec<NodeId> {
    doc.children(node)
        .filter(|&c| doc.get(c).is_some_and(|e| e.kind() == DocKind::Section))
        .collect()
}

#[test]
fn test_projection_an_update_with_nothing_to_shape_takes_no_font_lock() {
    let mut b = DocumentBuilder::new();
    let root = b.root();
    let (node, _, span) = node_with_text(&mut b, root, 0.0, "boxes only");
    let mut p = Projected::new(b.build().unwrap());
    // Give back the lock the full projection's lease took, so each update
    // below starts from a lease that holds nothing.
    p.fonts = crate::projection::tests::support::fonts();
    p.doc
        .apply(
            request(single(DocKind::Node, DocMutation::Node(NodeOp::Nudge(Vec2::new(5.0, 5.0)))), vec![node], "nudge"),
            ChangeSource::Action,
        )
        .unwrap();
    let records = p.doc.take_changes();
    let mut fresh = FontSystemLease::new("no shaping");
    let changes = p.projector.update(&p.doc, &records, &mut p.layers, &mut fresh);
    assert!(!changes.is_empty(), "the section was re-placed");
    assert!(!fresh.holds(), "re-placing a box run takes no font lock");
    changes.discard(&mut p.layers);
    drop(fresh);

    // The control: a text edit shapes, and the lease takes the lock.
    p.doc
        .apply(
            request(single(DocKind::Span, DocMutation::Span(SpanOp::SetText("new".into()))), vec![span], "text"),
            ChangeSource::Action,
        )
        .unwrap();
    let records = p.doc.take_changes();
    let mut fresh = FontSystemLease::new("shaping");
    let changes = p.projector.update(&p.doc, &records, &mut p.layers, &mut fresh);
    assert!(fresh.holds(), "a reshape takes the lock through the lease");
    drop(fresh);
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_a_tiny_relative_ring_size_shapes_at_the_least_size() {
    use crate::font::fonts::MIN_FONT_SIZE_PT;
    let mut b = DocumentBuilder::new();
    let root = b.root();
    let (node, _, _) = node_with_text(&mut b, root, 0.0, "node");
    b.edit_node(node, |n| n.min_size = Some(Vec2::new(200.0, 100.0)));
    let frame = b.add(node, DocElement::Frame(FrameData::default()));
    let ring = b.add(
        frame,
        DocElement::Ring(RingData {
            channel: 0,
            style: RingStyle {
                fit: Some(FitMode::Repeat),
                text: TextStyle {
                    size: Some(FontSize::Rel(f32::MIN_POSITIVE)),
                    ..TextStyle::default()
                },
                ..RingStyle::default()
            },
        }),
    );
    b.span(ring, "·");
    let p = Projected::new(b.build().unwrap());
    let UnitBody::Ring(state) = &p.projector.units[&ring].body else {
        unreachable!()
    };
    let size = state.flow.flow.as_ref().unwrap().key.spans[0].1.size;
    assert_eq!(size, FontSize::Pt(MIN_FONT_SIZE_PT), "shaped at the least shapeable size");
    let stored = p.doc.get(ring).and_then(DocElement::text_style).and_then(|s| s.size);
    assert_eq!(stored, Some(FontSize::Rel(f32::MIN_POSITIVE)), "the stored size stays as written");
}
