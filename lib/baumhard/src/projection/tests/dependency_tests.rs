// SPDX-License-Identifier: MPL-2.0

//! The dependency index, one test per kind of dependency (#241, Tests;
//! design §12.3), each asserting on what the update wrote
//! ([`LayerChanges`]) and on what it ran, beside an element the change
//! must not reach — the negative control each assertion owes is the
//! untouched element's — and every test ends on the oracle.

use std::sync::Arc;

use glam::Vec2;
use rustc_hash::FxHashSet;
use serde_json::{json, Value};

use crate::arena::NodeId;
use crate::document::element::{DefaultsData, DocElement, DocKind, TextStyle};
use crate::document::ids::Document;
use crate::document::mutation::ops::{DefaultsOp, DocMutation, NodeOp, RingOp, SpanOp, TextOp, ThemeOp};
use crate::document::tests::writer_support::{request, single};
use crate::document::values::{ColorRef, FontSize, PaletteId, Rgba};
use crate::document::writer::{ChangeSource, EditRequest};
use crate::engine::journal::Touch;
use crate::font::fonts::FontSystemLease;
use crate::gfx_structs::run::RunShape;
use crate::projection::state::UnitBody;
use crate::projection::tests::maps::{ident, kid, kids, load_map, node};
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

/// A node at `x` of one section of one span `span`, with `fields`.
fn text_node(id: u32, parent: Option<u32>, x: f32, span: Value, fields: Value) -> Value {
    let mut n = node(id, parent, fields);
    n["position"] = json!([x, 0.0]);
    n["sections"] = json!([{"spans": [span]}]);
    if parent.is_some() && n.get("connector").is_none() {
        n["connector"] = json!({});
    }
    n
}

/// The section of node `id`, and its span.
fn section_of(doc: &Document, id: u32) -> (NodeId, NodeId) {
    let section = kid(doc, ident(doc, id), DocKind::Section);
    (section, kid(doc, section, DocKind::Span))
}

/// A theme with `accent` different in two variants.
fn theme() -> Value {
    json!({"active": "dark", "variants": {"dark": {"accent": "#c86400"}, "light": {"accent": "#0a141e"}}})
}

#[test]
fn test_projection_a_theme_switch_reaches_exactly_the_readers_of_a_variable() {
    let doc = load_map(json!({
        "theme": theme(),
        "nodes": [
            text_node(1, None, 0.0, json!({"text": "themed", "color": "var(accent)"}), json!({})),
            text_node(2, None, 300.0, json!({"text": "literal", "color": "#010203"}), json!({})),
            text_node(3, None, 600.0, json!({"text": "plain"}), json!({})),
        ]
    }));
    let (themed, literal, plain) = (
        section_of(&doc, 1).0,
        section_of(&doc, 2).0,
        section_of(&doc, 3).0,
    );
    let theme = kid(&doc, doc.root(), DocKind::Theme);
    let mut p = Projected::new(doc);
    let changes = apply(
        &mut p,
        request(
            single(DocKind::Theme, DocMutation::Theme(ThemeOp::SetActive(1))),
            vec![theme],
            "switch",
        ),
    );
    let sections = touched(&p, &changes, Layer::Sections);
    assert!(
        sections.contains(&themed),
        "the section that reads the variable: {sections:?}"
    );
    assert!(!sections.contains(&literal), "a pure-Rgba section is not touched");
    assert!(!sections.contains(&plain), "nor one that reads no variable");
    let resolved = ran(&p, Stage::Resolve);
    assert_eq!(
        resolved,
        [themed].into_iter().collect(),
        "only the reader resolves again"
    );
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_a_parents_section_text_moves_its_childs_connector() {
    let doc = load_map(json!({"nodes": [
        text_node(1, None, 0.0, json!({"text": "short"}), json!({})),
        node(2, Some(1), json!({"position": [0.0, 300.0], "connector": {}})),
        text_node(3, None, 900.0, json!({"text": "elsewhere"}), json!({})),
        node(4, Some(3), json!({"position": [900.0, 300.0], "connector": {}})),
    ]}));
    let parent_span = section_of(&doc, 1).1;
    let connector = kid(&doc, ident(&doc, 2), DocKind::Connector);
    let other_connector = kid(&doc, ident(&doc, 4), DocKind::Connector);
    let mut p = Projected::new(doc);
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
    assert!(
        connections.contains(&connector),
        "the child's connector moved: {connections:?}"
    );
    assert!(
        !connections.contains(&other_connector),
        "another node's connector did not"
    );
    assert!(
        !ran(&p, Stage::Resolve).contains(&connector),
        "the connector re-placed, and resolved nothing"
    );
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_a_defaults_change_reaches_only_its_inheritors() {
    let doc = load_map(json!({"nodes": [
        text_node(1, None, 0.0, json!({"text": "inherits"}), json!({})),
        text_node(2, None, 300.0, json!({"text": "its own", "color": "#090909", "size": 12.0}), json!({})),
    ]}));
    let (inheriting, own) = (section_of(&doc, 1).0, section_of(&doc, 2).0);
    let defaults = kid(&doc, doc.root(), DocKind::Defaults);
    let mut p = Projected::new(doc);
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
            single(
                DocKind::Defaults,
                DocMutation::Defaults(DefaultsOp::Replace(Box::new(replaced))),
            ),
            vec![defaults],
            "defaults",
        ),
    );
    let sections = touched(&p, &changes, Layer::Sections);
    assert!(
        sections.contains(&inheriting),
        "the section that inherits: {sections:?}"
    );
    assert!(
        !sections.contains(&own),
        "the one that sets both fields itself is not reached"
    );
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

/// A palette of three groups whose title colors name their depth:
/// `(n, n, 0)` for group `n` of `10, 11, 12`.
fn depth_palette() -> Value {
    json!([
        {"frame": "#000a00", "text": "#00000a", "title": "#0a0a00"},
        {"frame": "#000b00", "text": "#00000b", "title": "#0b0b00"},
        {"frame": "#000c00", "text": "#00000c", "title": "#0c0c00"}
    ])
}

#[test]
fn test_projection_a_palette_binding_moved_to_another_ancestor_regroups_by_depth() {
    let doc = load_map(json!({
        "palettes": {"p": depth_palette()},
        "nodes": [
            text_node(1, None, 0.0, json!({"text": "top"}), json!({"palette": "p"})),
            text_node(2, Some(1), 0.0, json!({"text": "middle"}), json!({})),
            text_node(3, Some(2), 0.0, json!({"text": "leaf"}), json!({})),
            text_node(4, None, 600.0, json!({"text": "apart"}), json!({})),
        ]
    }));
    let (top, middle) = (ident(&doc, 1), ident(&doc, 2));
    let (leaf_section, apart) = (section_of(&doc, 3).0, section_of(&doc, 4).0);
    let mut p = Projected::new(doc);
    let leaf_color = |p: &Projected| -> Rgba {
        match &p.projector.units[&leaf_section].body {
            UnitBody::Section(s) => s.flow.as_ref().unwrap().key.spans[0].1.color,
            _ => unreachable!(),
        }
    };
    assert_eq!(
        leaf_color(&p),
        Rgba::opaque(12, 12, 0),
        "two levels below the binding: group 2's title"
    );
    let steps = [
        request(
            single(DocKind::Node, DocMutation::Node(NodeOp::SetPalette(None))),
            vec![top],
            "unbind",
        ),
        request(
            single(
                DocKind::Node,
                DocMutation::Node(NodeOp::SetPalette(Some(PaletteId(0)))),
            ),
            vec![middle],
            "bind",
        ),
    ]
    .into_iter()
    .flat_map(|r| r.steps)
    .collect();
    let changes = apply(&mut p, EditRequest::new(steps, "move the binding"));
    assert_eq!(
        leaf_color(&p),
        Rgba::opaque(11, 11, 0),
        "one level below it now: group 1's"
    );
    let sections = touched(&p, &changes, Layer::Sections);
    assert!(
        sections.contains(&leaf_section),
        "the leaf regrouped: {sections:?}"
    );
    assert!(!sections.contains(&apart), "a node under neither is not reached");
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_hidden_removes_a_subtree_and_the_links_into_it() {
    let doc = load_map(json!({"nodes": [
        text_node(1, None, 0.0, json!({"text": "holder"}), json!({})),
        text_node(2, Some(1), 0.0, json!({"text": "hidden"}), json!({})),
        text_node(3, Some(2), 0.0, json!({"text": "below"}), json!({})),
        text_node(4, None, 600.0, json!({"text": "outside"}), json!({"links": [
            {"to": 3, "kind": "line"},
            {"to": 2, "kind": "portal"},
            {"to": 1, "kind": "line"}
        ]})),
    ]}));
    let (hidden, below) = (ident(&doc, 2), ident(&doc, 3));
    let links = kids(&doc, ident(&doc, 4), DocKind::Link);
    let (line, portal, kept) = (links[0], links[1], links[2]);
    let mut p = Projected::new(doc);
    assert!(
        !p.projector.glyphs_of(line).is_empty(),
        "the fixture draws the link"
    );
    let hide = |on: bool| {
        request(
            single(DocKind::Node, DocMutation::Node(NodeOp::SetHidden(on))),
            vec![hidden],
            "hide",
        )
    };
    let changes = apply(&mut p, hide(true));
    for layer in Layer::ALL {
        let removed = changes
            .journal(layer)
            .touched()
            .filter(|&(_, t)| t == Touch::Removed)
            .count();
        assert!(
            removed >= 2,
            "{layer:?}: the hidden node's group and its child's leave ({removed})"
        );
    }
    for node in [hidden, below] {
        assert!(p.projector.glyphs_of(node).is_empty(), "{node:?} has no group");
    }
    assert!(
        p.projector.glyphs_of(line).is_empty(),
        "the link into the subtree is gone"
    );
    assert!(p.projector.glyphs_of(portal).is_empty(), "the portal to it too");
    assert!(
        !p.projector.glyphs_of(kept).is_empty(),
        "a link to a node still drawn stays"
    );
    changes.discard(&mut p.layers);
    p.check().unwrap();
    let changes = apply(&mut p, hide(false));
    assert!(
        !p.projector.glyphs_of(below).is_empty(),
        "un-hiding restores the subtree"
    );
    assert!(!p.projector.glyphs_of(line).is_empty(), "and the link");
    assert!(!p.projector.glyphs_of(portal).is_empty(), "and the portal");
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_an_ancestors_opacity_reaches_every_descendants_runs() {
    let doc = load_map(json!({"nodes": [
        text_node(1, None, 0.0, json!({"text": "top"}), json!({})),
        text_node(2, Some(1), 0.0, json!({"text": "middle"}), json!({})),
        text_node(3, Some(2), 0.0, json!({"text": "leaf"}), json!({})),
        text_node(4, None, 600.0, json!({"text": "apart"}), json!({})),
    ]}));
    let top = ident(&doc, 1);
    let faded: Vec<NodeId> = (1..=3).map(|n| section_of(&doc, n).0).collect();
    let apart = section_of(&doc, 4).0;
    let mut p = Projected::new(doc);
    let changes = apply(
        &mut p,
        request(
            single(DocKind::Node, DocMutation::Node(NodeOp::SetOpacity(0.5))),
            vec![top],
            "fade",
        ),
    );
    let sections = touched(&p, &changes, Layer::Sections);
    for section in faded {
        assert!(sections.contains(&section), "{section:?} faded: {sections:?}");
        let opacity = p.projector.units[&section].slots[0]
            .written
            .as_ref()
            .unwrap()
            .opacity();
        assert_eq!(opacity, 0.5, "at the ancestor's opacity");
    }
    assert!(!sections.contains(&apart), "a node beside the subtree is not");
    assert!(ran(&p, Stage::Resolve).is_empty(), "an opacity resolves nothing");
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

/// A node bound to a palette, with a `repeat` ring, and a child whose
/// text inherits the palette: the map, the node, its section, the
/// child's section and the ring.
fn palette_subtree() -> (Document, NodeId, NodeId, NodeId, NodeId) {
    let doc = load_map(json!({
        "palettes": {"p": depth_palette()},
        "nodes": [
            text_node(1, None, 0.0, json!({"text": "the earth"}), json!({
                "palette": "p",
                "frame": {"rings": [{"fit": "repeat", "spans": [{"text": "·"}]}]}
            })),
            text_node(2, Some(1), 0.0, json!({"text": "without form"}), json!({})),
        ]
    }));
    let top = ident(&doc, 1);
    let ring = kid(&doc, kid(&doc, top, DocKind::Frame), DocKind::Ring);
    let (top_section, child_section) = (section_of(&doc, 1).0, section_of(&doc, 2).0);
    (doc, top, top_section, child_section, ring)
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
    let (doc, top, top_section, child_section, ring) = palette_subtree();
    let mut p = Projected::new(doc);
    let before = (
        shaping(&p, top_section),
        shaping(&p, child_section),
        shaping(&p, ring),
    );
    let changes = apply(
        &mut p,
        request(
            single(
                DocKind::Node,
                DocMutation::Node(NodeOp::Nudge(Vec2::new(40.0, 0.0))),
            ),
            vec![top],
            "drag",
        ),
    );
    assert!(
        ran(&p, Stage::Resolve).is_empty(),
        "a drag dirties no text resolution: {:?}",
        p.projector.ran
    );
    assert_eq!(
        (
            shaping(&p, top_section),
            shaping(&p, child_section),
            shaping(&p, ring)
        ),
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
    assert!(
        ran(&p, Stage::Resolve).contains(&top_section),
        "the size is resolved again"
    );
    assert_ne!(
        shaping(&p, top_section),
        before.0,
        "and the run that inherits it reshaped"
    );
    assert_eq!(
        shaping(&p, child_section),
        before.1,
        "the child node's text inherits no node size"
    );
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_a_move_reuses_shaping_and_a_resize_reshapes_only_relative_rings() {
    let (doc, top, top_section, child_section, ring) = palette_subtree();
    let mut p = Projected::new(doc);
    let before = (
        shaping(&p, top_section),
        shaping(&p, child_section),
        shaping(&p, ring),
    );
    let changes = apply(
        &mut p,
        request(
            single(
                DocKind::Node,
                DocMutation::Node(NodeOp::SetPosition(Vec2::new(500.0, 80.0))),
            ),
            vec![top],
            "move",
        ),
    );
    assert_eq!(
        (
            shaping(&p, top_section),
            shaping(&p, child_section),
            shaping(&p, ring)
        ),
        before,
        "a move reuses every shaping, pointer for pointer"
    );
    changes.discard(&mut p.layers);
    let changes = apply(
        &mut p,
        request(
            single(
                DocKind::Node,
                DocMutation::Node(NodeOp::SetMinSize(Some(Vec2::new(400.0, 300.0)))),
            ),
            vec![top],
            "resize",
        ),
    );
    assert_eq!(
        shaping(&p, top_section),
        before.0,
        "a resize keeps the section's shaping"
    );
    assert_ne!(
        shaping(&p, ring),
        before.2,
        "and reshapes the ring, whose size is relative"
    );
    assert!(
        matches!(
            p.projector.units[&ring].slots[0]
                .written
                .as_ref()
                .map(|r| r.shaped().clone()),
            Some(RunShape::Path { .. })
        ),
        "the ring is drawn"
    );
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_an_update_with_nothing_to_shape_takes_no_font_lock() {
    let doc = load_map(json!({"nodes": [text_node(1, None, 0.0, json!({"text": "boxes only"}), json!({}))]}));
    let (node, span) = (ident(&doc, 1), section_of(&doc, 1).1);
    let mut p = Projected::new(doc);
    p.doc
        .apply(
            request(
                single(
                    DocKind::Node,
                    DocMutation::Node(NodeOp::Nudge(Vec2::new(5.0, 5.0))),
                ),
                vec![node],
                "nudge",
            ),
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
            request(
                single(DocKind::Span, DocMutation::Span(SpanOp::SetText("new".into()))),
                vec![span],
                "text",
            ),
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
    // A run of `ScaleSize` steps can drive a stored relative size as low
    // as `f32::MIN_POSITIVE` (R14.10); an edit writes it here.
    let doc = load_map(
        json!({"nodes": [text_node(1, None, 0.0, json!({"text": "node"}), json!({
            "min_size": [200.0, 100.0],
            "frame": {"rings": [{"fit": "repeat", "size": 0.06, "spans": [{"text": "·"}]}]}
        }))]}),
    );
    let ring = kid(&doc, kid(&doc, ident(&doc, 1), DocKind::Frame), DocKind::Ring);
    let mut p = Projected::new(doc);
    p.apply(request(
        single(
            DocKind::Ring,
            DocMutation::Ring(RingOp::Text(TextOp::SetSize(Some(FontSize::Rel(
                f32::MIN_POSITIVE,
            ))))),
        ),
        vec![ring],
        "tiny",
    ))
    .unwrap();
    let stored = p
        .doc
        .get(ring)
        .and_then(DocElement::text_style)
        .and_then(|s| s.size);
    assert_eq!(
        stored,
        Some(FontSize::Rel(f32::MIN_POSITIVE)),
        "the stored size stays as written"
    );
    let UnitBody::Ring(state) = &p.projector.units[&ring].body else { unreachable!() };
    let size = state.flow.flow.as_ref().unwrap().key.spans[0].1.size;
    assert_eq!(
        size,
        FontSize::Pt(MIN_FONT_SIZE_PT),
        "shaped at the least shapeable size"
    );
    p.check().unwrap();
}

#[test]
fn test_projection_work_that_computes_the_same_value_writes_nothing() {
    let doc = load_map(json!({"nodes": [
        text_node(1, None, 0.0, json!({"text": "from"}), json!({"links": [
            {"to": 2, "kind": "line", "spans": [{"text": "~"}],
             "label": {"t": 0.25, "offset": 4.0, "spans": [{"text": "its own place"}]}}
        ]})),
        text_node(2, None, 500.0, json!({"text": "to"}), json!({})),
    ]}));
    let link = kid(&doc, ident(&doc, 1), DocKind::Link);
    let defaults = kid(&doc, doc.root(), DocKind::Defaults);
    let mut p = Projected::new(doc);
    let mut replaced = DefaultsData::default();
    replaced.label.t = Some(0.75);
    let changes = apply(
        &mut p,
        request(
            single(
                DocKind::Defaults,
                DocMutation::Defaults(DefaultsOp::Replace(Box::new(replaced))),
            ),
            vec![defaults],
            "defaults",
        ),
    );
    assert!(
        ran(&p, Stage::Place).contains(&link),
        "the link read the defaults, so it placed again"
    );
    assert!(
        changes.is_empty(),
        "and placed exactly where it was: nothing is written"
    );
    changes.discard(&mut p.layers);
    p.check().unwrap();
}

#[test]
fn test_projection_compares_against_what_it_wrote_not_what_an_overlay_wrote() {
    use crate::engine::anchor::Anchors;
    use crate::gfx_structs::mutator::{GfxMutation, GfxMutator};
    use crate::gfx_structs::tree::GfxMutatorTree;
    let doc = load_map(json!({"nodes": [
        text_node(1, None, 0.0, json!({"text": "tinted"}), json!({})),
        text_node(2, None, 400.0, json!({"text": "moved"}), json!({})),
    ]}));
    let (tinted, _) = section_of(&doc, 1);
    let defaults = kid(&doc, doc.root(), DocKind::Defaults);
    let mut p = Projected::new(doc);
    // An overlay — a selection tint, a glyph-layer presentation — writes
    // the live run, beside the projector (#241 amendment 2).
    let glyph = p.projector.units[&tinted].slots[0].glyph.unwrap();
    GfxMutatorTree::new_with(GfxMutator::new(GfxMutation::RunOpacity(0.2), 0))
        .apply_at(
            p.layers.tree_mut(Layer::Sections),
            &Anchors::Nodes(vec![glyph]),
            |_, _| (),
        )
        .unwrap();
    // A defaults change every section reads and none uses: each one
    // resolves and places again, and comes out as it was.
    let mut replaced = DefaultsData::default();
    replaced.label.t = Some(0.75);
    let changes = apply(
        &mut p,
        request(
            single(
                DocKind::Defaults,
                DocMutation::Defaults(DefaultsOp::Replace(Box::new(replaced))),
            ),
            vec![defaults],
            "defaults",
        ),
    );
    assert!(
        ran(&p, Stage::Place).contains(&tinted),
        "the overlaid section placed again"
    );
    let sections = touched(&p, &changes, Layer::Sections);
    assert!(
        !sections.contains(&tinted),
        "its run equals what the projector wrote, so it is not written over the overlay: {sections:?}"
    );
    let live = p.layers.sections.arena()[glyph]
        .get()
        .glyph_run()
        .unwrap()
        .opacity();
    assert_eq!(live, 0.2, "the overlay stays where it is");
    changes.discard(&mut p.layers);
}

#[test]
fn test_projection_a_sides_color_change_rewrites_the_reused_shaping() {
    use crate::document::mutation::ops::LayoutOp;
    use crate::document::values::Sides;
    // A ring's sides are laid by its placement, in a run that replaces
    // the shaping (`settle`): a new color keeps the shaping key and every
    // glyph id, and must still reach the run that is drawn.
    let doc = load_map(
        json!({"nodes": [text_node(1, None, 0.0, json!({"text": "node"}), json!({
            "min_size": [320.0, 200.0],
            "frame": {"rings": [{"size": 0.08, "sides": {"content": "✦", "color": "#102030"},
                                 "spans": [{"text": "ring"}]}]}
        }))]}),
    );
    let ring = kid(&doc, kid(&doc, ident(&doc, 1), DocKind::Frame), DocKind::Ring);
    let placed = |p: &Projected| match &p.projector.units[&ring].body {
        UnitBody::Ring(state) => state.flow.placed.clone().expect("the ring is placed"),
        _ => unreachable!("a ring's unit"),
    };
    let mut p = Projected::new(doc);
    let before = placed(&p);
    // Precondition: the run drawn is the placement's own, not the shaping.
    assert!(
        !Arc::ptr_eq(&before.run, &before.input),
        "the sides replace the shaped run"
    );
    p.apply(request(
        single(
            DocKind::Ring,
            DocMutation::Ring(RingOp::Layout(LayoutOp::SetSides(Some(Sides::On {
                content: "✦".into(),
                color: Some(ColorRef::Rgba(Rgba::opaque(200, 10, 10))),
            })))),
        ),
        vec![ring],
        "recolor the sides",
    ))
    .unwrap();
    let after = placed(&p);
    assert!(
        Arc::ptr_eq(&after.input, &before.input),
        "the shaping is kept: its key did not change"
    );
    assert!(
        !Arc::ptr_eq(&after.run, &before.run),
        "the recolored run is a new one"
    );
    p.check().unwrap();
}

/// How many children the wide row holds: enough that a connector reading
/// its parent's row is two hundred resolutions an edit.
const WIDE: u32 = 200;

/// Node 1 with [`WIDE`] children, each with a labeled connector, and a
/// second root, node 2, to move one of them to.
fn wide_row() -> Document {
    let mut nodes = vec![node(1, None, json!({}))];
    for i in 0..WIDE {
        nodes.push(node(
            10 + i,
            Some(1),
            json!({
                "position": [i as f32 * 60.0, 300.0],
                "connector": {"label": {"spans": [{"text": "c"}]}}
            }),
        ));
    }
    // The flat list is in pre-order: the second root after the first's
    // subtree.
    nodes.push(node(2, None, json!({"position": [0.0, 900.0]})));
    load_map(json!({ "nodes": nodes }))
}

/// How many connectors a stage of which ran in the last call.
fn connectors_ran(p: &Projected, stage: Stage) -> usize {
    ran(p, stage)
        .into_iter()
        .filter(|&u| p.doc.get(u).is_some_and(|e| e.kind() == DocKind::Connector))
        .count()
}

#[test]
fn test_projection_an_insert_into_a_wide_row_runs_one_connector() {
    use crate::document::mutation::compose;
    let doc = wide_row();
    let parent = ident(&doc, 1);
    let mut p = Projected::new(doc);
    let steps = compose::create_child(&p.doc, parent, Vec2::new(30.0, 600.0)).unwrap();
    p.apply(EditRequest::new(steps, "one more child")).unwrap();
    // The new child's connector, and none of its siblings'.
    for stage in [Stage::Resolve, Stage::Place] {
        let n = connectors_ran(&p, stage);
        assert!(
            n <= 1,
            "{n} connectors ran {stage:?} for one child added to a row of {WIDE}"
        );
    }
    p.check().unwrap();
}

#[test]
fn test_projection_a_move_out_of_a_wide_row_runs_one_connector() {
    use crate::document::mutation::compose;
    let doc = wide_row();
    let (moved, to) = (ident(&doc, 10 + WIDE / 2), ident(&doc, 2));
    let mut p = Projected::new(doc);
    let steps = compose::reparent(&p.doc, &[moved], to).unwrap();
    p.apply(EditRequest::new(steps, "move one out")).unwrap();
    // The moved child's connector, drawn from its new parent, and none
    // of the siblings it left.
    for stage in [Stage::Resolve, Stage::Place] {
        let n = connectors_ran(&p, stage);
        assert!(
            n <= 1,
            "{n} connectors ran {stage:?} for one child moved out of a row of {WIDE}"
        );
    }
    assert_eq!(
        connectors_ran(&p, Stage::Resolve),
        1,
        "the moved child's connector resolves again"
    );
    p.check().unwrap();
}

/// Node 1, holding a portal link to node 2; node 2 under it, its
/// connector labeled and bent by one control point. Every element a
/// geometry-only drag moves, with text beside it that a text change
/// re-resolves.
fn dragged_parts() -> Document {
    load_map(json!({"nodes": [
        text_node(1, None, 0.0, json!({"text": "from"}), json!({
            "links": [{"to": 2, "kind": "portal", "ends": [{}, {}]}]
        })),
        text_node(2, Some(1), 400.0, json!({"text": "to"}), json!({
            "connector": {
                "label": {"t": 0.5, "spans": [{"text": "label"}]},
                "control_points": [[10.0, -20.0]]
            }
        })),
    ]}))
}

/// Apply one `mutation` of `kind` to `target` and return the units whose
/// `Resolve` ran.
fn resolved_by(p: &mut Projected, kind: DocKind, mutation: DocMutation, target: NodeId) -> FxHashSet<NodeId> {
    apply(p, request(single(kind, mutation), vec![target], "edit")).discard(&mut p.layers);
    ran(p, Stage::Resolve)
}

#[test]
fn test_projection_a_geometry_only_drag_of_a_unit_part_resolves_nothing() {
    use crate::document::mutation::ops::{LabelOp, PathOp, PortalEndOp, SectionOp};
    let doc = dragged_parts();
    let (one, two) = (ident(&doc, 1), ident(&doc, 2));
    let connector = kid(&doc, two, DocKind::Connector);
    let label = kid(&doc, connector, DocKind::Label);
    let label_span = kid(&doc, label, DocKind::Span);
    let portal = kid(&doc, one, DocKind::Link);
    let end = kid(&doc, portal, DocKind::PortalEnd);
    let (section, section_span) = section_of(&doc, 2);
    let mut p = Projected::new(doc);

    // A label dragged along its connector.
    let r = resolved_by(&mut p, DocKind::Label, DocMutation::Label(LabelOp::SetT(Some(0.8))), label);
    assert!(r.is_empty(), "a label drag resolves nothing: {r:?}");
    p.check().unwrap();
    // The control: the label's text.
    let r = resolved_by(&mut p, DocKind::Span, DocMutation::Span(SpanOp::SetText("moved".into())), label_span);
    assert!(r.contains(&connector), "the label's text resolves its connector: {r:?}");
    p.check().unwrap();

    // A portal marker dragged round its node.
    let r = resolved_by(
        &mut p,
        DocKind::PortalEnd,
        DocMutation::PortalEnd(PortalEndOp::SetBorderT(Some(0.3))),
        end,
    );
    assert!(r.is_empty(), "a marker drag resolves nothing: {r:?}");
    p.check().unwrap();
    // The control: the marker's text.
    let r = resolved_by(
        &mut p,
        DocKind::PortalEnd,
        DocMutation::PortalEnd(PortalEndOp::SetContent(Some("◆".into()))),
        end,
    );
    assert!(r.contains(&portal), "the marker's text resolves its link: {r:?}");
    p.check().unwrap();

    // A section nudged within its node.
    let r = resolved_by(
        &mut p,
        DocKind::Section,
        DocMutation::Section(SectionOp::Nudge(Vec2::new(6.0, 4.0))),
        section,
    );
    assert!(r.is_empty(), "a section nudge resolves nothing: {r:?}");
    p.check().unwrap();
    // The control: the section's text.
    let r = resolved_by(&mut p, DocKind::Span, DocMutation::Span(SpanOp::SetText("too".into())), section_span);
    assert!(r.contains(&section), "the section's text resolves it: {r:?}");
    p.check().unwrap();

    // A control point moved.
    let r = resolved_by(
        &mut p,
        DocKind::Connector,
        DocMutation::Path(PathOp::SetControlPoints(vec![Vec2::new(-30.0, 25.0)])),
        connector,
    );
    assert!(r.is_empty(), "a control-point move resolves nothing: {r:?}");
    p.check().unwrap();
    // The control: the connector's text.
    let r = resolved_by(
        &mut p,
        DocKind::Connector,
        DocMutation::Path(PathOp::SetContent(Some("~".into()))),
        connector,
    );
    assert!(r.contains(&connector), "the connector's text resolves it: {r:?}");
    p.check().unwrap();
}

#[test]
fn test_projection_a_drag_carries_and_shapes_nothing_a_resize_and_a_fade_do() {
    let (doc, top, _, _, ring) = palette_subtree();
    let mut p = Projected::new(doc);
    apply(
        &mut p,
        request(
            single(DocKind::Node, DocMutation::Node(NodeOp::Nudge(Vec2::new(40.0, 0.0)))),
            vec![top],
            "drag",
        ),
    )
    .discard(&mut p.layers);
    assert!(ran(&p, Stage::Carry).is_empty(), "a drag carries nothing: {:?}", p.projector.ran);
    assert!(ran(&p, Stage::Shape).is_empty(), "nor shapes anything: {:?}", p.projector.ran);
    p.check().unwrap();

    // The controls: a resize reshapes the relative ring, and a fade
    // carries the node again.
    apply(
        &mut p,
        request(
            single(DocKind::Node, DocMutation::Node(NodeOp::SetMinSize(Some(Vec2::new(500.0, 300.0))))),
            vec![top],
            "resize",
        ),
    )
    .discard(&mut p.layers);
    assert!(ran(&p, Stage::Shape).contains(&ring), "a resize reshapes the relative ring");
    p.check().unwrap();
    apply(
        &mut p,
        request(
            single(DocKind::Node, DocMutation::Node(NodeOp::SetOpacity(0.5))),
            vec![top],
            "fade",
        ),
    )
    .discard(&mut p.layers);
    assert!(ran(&p, Stage::Carry).contains(&top), "a fade carries the node again");
    p.check().unwrap();
}
