// SPDX-License-Identifier: MPL-2.0

//! Style resolution: each tier of the chain, the palette tier, theme
//! variables, the ring's one size, `content`, and the read records.
//!
//! Every tier test sets the tier under test **and** the tiers behind
//! it to different values, so a value cannot pass by arriving from a
//! fallback (TEST_CONVENTIONS §T13.1, shape 2), and then removes the
//! tier under test and expects the next one — the negative control,
//! which fails if the tier was never the source.

use crate::arena::NodeId;
use crate::document::defaults::BUILT_IN;
use crate::document::element::{
    DefaultsData, DocElement, PaletteData, RingLayoutStyle, TextStyle, ThemeData, ThemeVariant,
};
use crate::document::resolve::{
    resolve_frame, resolve_label, resolve_node, resolve_path, resolve_portal_end, resolve_ring, resolve_text,
    BorderPoint, Content, Input, ResolvedText,
};
use crate::document::tests::test_support::{example, palette_group, DocumentBuilder, Example};
use crate::document::values::{
    ColorRef, FitMode, FontSize, LineSpan, PaletteField, PaletteId, Percent, PlainColor, Rgba, Shape, Sides,
    VarId, Weight,
};
use crate::font::fonts::AppFont;

const RED: Rgba = Rgba::opaque(0xff, 0, 0);
const GREEN: Rgba = Rgba::opaque(0, 0xff, 0);
const BLUE: Rgba = Rgba::opaque(0, 0, 0xff);
const YELLOW: Rgba = Rgba::opaque(0xff, 0xff, 0);
const CYAN: Rgba = Rgba::opaque(0, 0xff, 0xff);
const MAGENTA: Rgba = Rgba::opaque(0xff, 0, 0xff);

/// A [`ColorRef`] of a literal.
fn rgba(color: Rgba) -> Option<ColorRef> {
    Some(ColorRef::Rgba(color))
}

/// The resolved text of `owner`, and the inputs it read.
fn text(builder: &DocumentBuilder, owner: NodeId) -> (ResolvedText, Vec<Input>) {
    let mut reads = Vec::new();
    let resolved = resolve_text(builder.tree(), owner, &mut reads);
    (resolved, reads)
}

/// The resolved text color of `owner`.
fn color(builder: &DocumentBuilder, owner: NodeId) -> Rgba {
    text(builder, owner).0.color
}

/// Change the text style of the element at `id`.
fn style(builder: &mut DocumentBuilder, id: NodeId, f: impl FnOnce(&mut TextStyle)) {
    builder.edit(id, |e| match e {
        DocElement::Node(node) => f(&mut node.style.text),
        DocElement::Section(section) => f(&mut section.text),
        DocElement::Span(span) => f(&mut span.style),
        DocElement::Frame(frame) => f(&mut frame.style.text),
        DocElement::Ring(ring) => f(&mut ring.style.text),
        DocElement::CapStart(cap) | DocElement::CapEnd(cap) => f(&mut cap.text),
        DocElement::Label(label) => f(&mut label.style.text),
        other => panic!("no text style on a {:?}", other.kind()),
    });
}

/// Change the map's defaults.
fn defaults(builder: &mut DocumentBuilder, f: impl FnOnce(&mut DefaultsData)) {
    let id = builder.defaults_id();
    builder.edit(id, |e| match e {
        DocElement::Defaults(defaults) => f(defaults),
        other => panic!("not the defaults: {:?}", other.kind()),
    });
}

/// The example with a section and a span under the unbound root node
/// `d`; returns the span.
fn unbound_span() -> (DocumentBuilder, Example, NodeId, NodeId) {
    let (mut builder, ids) = example();
    let section = builder.section(ids.d, 0);
    let span = builder.span(section, "unbound");
    (builder, ids, section, span)
}

/// Every text tier of a section span set to its own color: span red,
/// section green, node blue, `Defaults.span` yellow, `Defaults.section`
/// cyan, `Defaults.node.text` magenta.
fn every_tier_colored() -> (DocumentBuilder, Example, NodeId, NodeId) {
    let (mut builder, ids, section, span) = unbound_span();
    style(&mut builder, span, |s| s.color = rgba(RED));
    style(&mut builder, section, |s| s.color = rgba(GREEN));
    style(&mut builder, ids.d, |s| s.color = rgba(BLUE));
    defaults(&mut builder, |d| {
        d.span.color = rgba(YELLOW);
        d.section.color = rgba(CYAN);
        d.node.text.color = rgba(MAGENTA);
    });
    (builder, ids, section, span)
}

#[test]
fn test_document_resolve_own_field_wins() {
    do_document_resolve_own_field_wins();
}

/// The span's own color wins over every tier behind it. Control: with
/// the span's color gone, the color is the section's.
pub fn do_document_resolve_own_field_wins() {
    let (mut builder, _, _, span) = every_tier_colored();
    assert_eq!(color(&builder, span), RED);
    style(&mut builder, span, |s| s.color = None);
    assert_eq!(color(&builder, span), GREEN);
}

#[test]
fn test_document_resolve_owner_chain() {
    do_document_resolve_owner_chain();
}

/// With the span unset, the section's color wins over the node's, and
/// the node's over `Defaults`. Controls: each removed in turn hands the
/// color to the next.
pub fn do_document_resolve_owner_chain() {
    let (mut builder, ids, section, span) = every_tier_colored();
    style(&mut builder, span, |s| s.color = None);
    assert_eq!(color(&builder, span), GREEN);
    style(&mut builder, section, |s| s.color = None);
    assert_eq!(color(&builder, span), BLUE);
    style(&mut builder, ids.d, |s| s.color = None);
    assert_ne!(color(&builder, span), BLUE);
}

#[test]
fn test_document_resolve_ring_owner_chain() {
    do_document_resolve_ring_owner_chain();
}

/// A ring span's color runs ring → frame → `Defaults.ring` →
/// `Defaults.frame` → `BUILT_IN`, and never through the node's text
/// (R4), which is set to blue throughout. Controls: each removed hands
/// the color to the next.
pub fn do_document_resolve_ring_owner_chain() {
    let (mut builder, ids) = example();
    builder.edit_node(ids.a, |n| n.palette = None);
    style(&mut builder, ids.ring, |s| s.color = rgba(RED));
    style(&mut builder, ids.frame, |s| s.color = rgba(GREEN));
    style(&mut builder, ids.a, |s| s.color = rgba(BLUE));
    defaults(&mut builder, |d| {
        d.ring.text.color = rgba(YELLOW);
        d.frame.text.color = rgba(CYAN);
    });
    assert_eq!(color(&builder, ids.ring_span), RED);
    style(&mut builder, ids.ring, |s| s.color = None);
    assert_eq!(color(&builder, ids.ring_span), GREEN);
    style(&mut builder, ids.frame, |s| s.color = None);
    assert_eq!(color(&builder, ids.ring_span), YELLOW, "not the node's blue");
    defaults(&mut builder, |d| d.ring.text.color = None);
    assert_eq!(color(&builder, ids.ring_span), CYAN);
    defaults(&mut builder, |d| d.frame.text.color = None);
    assert_eq!(color(&builder, ids.ring_span), BUILT_IN.ring.text.color);
}

#[test]
fn test_document_resolve_defaults_tier() {
    do_document_resolve_defaults_tier();
}

/// With the element tiers unset, `Defaults` gives the color, in the
/// owner chain's order: `span`, then `section`, then `node.text`.
/// Control: with `Defaults` emptied, `BUILT_IN` gives it.
pub fn do_document_resolve_defaults_tier() {
    let (mut builder, ids, section, span) = every_tier_colored();
    for id in [span, section, ids.d] {
        style(&mut builder, id, |s| s.color = None);
    }
    assert_eq!(color(&builder, span), YELLOW);
    defaults(&mut builder, |d| d.span.color = None);
    assert_eq!(color(&builder, span), CYAN);
    defaults(&mut builder, |d| d.section.color = None);
    assert_eq!(color(&builder, span), MAGENTA);
    defaults(&mut builder, |d| d.node.text.color = None);
    assert_eq!(color(&builder, span), BUILT_IN.section.color);
}

#[test]
fn test_document_resolve_built_in_tier() {
    do_document_resolve_built_in_tier();
}

/// Nothing set: `BUILT_IN`'s section text, every field. Control: a
/// `Defaults.node.text` that differs from `BUILT_IN` in every field
/// replaces every field.
pub fn do_document_resolve_built_in_tier() {
    let (mut builder, _, _, span) = unbound_span();
    assert_eq!(text(&builder, span).0, BUILT_IN.section);
    let other = TextStyle {
        font: Some(AppFont::Novem),
        size: Some(FontSize::Pt(31.0)),
        color: rgba(MAGENTA),
        weight: Some(Weight::BOLD),
        italic: Some(true),
    };
    assert_ne!(Some(BUILT_IN.section.font), other.font);
    assert_ne!(Some(BUILT_IN.section.size), other.size);
    assert_ne!(Some(BUILT_IN.section.weight), other.weight);
    assert_ne!(Some(BUILT_IN.section.italic), other.italic);
    defaults(&mut builder, |d| d.node.text = other);
    assert_eq!(
        text(&builder, span).0,
        ResolvedText {
            font: AppFont::Novem,
            size: FontSize::Pt(31.0),
            color: MAGENTA,
            weight: Weight::BOLD,
            italic: true,
        }
    );
}

#[test]
fn test_document_resolve_is_per_field() {
    do_document_resolve_is_per_field();
}

/// A span that sets only its face takes its color from the section and
/// everything else from `BUILT_IN`: one field set never stops another
/// from inheriting.
pub fn do_document_resolve_is_per_field() {
    let (mut builder, _, section, span) = unbound_span();
    style(&mut builder, span, |s| s.font = Some(AppFont::Novem));
    style(&mut builder, section, |s| {
        s.color = rgba(GREEN);
        s.font = Some(AppFont::Regal);
    });
    let resolved = text(&builder, span).0;
    assert_eq!(resolved.font, AppFont::Novem);
    assert_eq!(resolved.color, GREEN);
    assert_eq!(resolved.size, BUILT_IN.section.size);
}

#[test]
fn test_document_resolve_built_in_by_flow() {
    do_document_resolve_built_in_by_flow();
}

/// With nothing set and no palette, each flow falls back to its own
/// `BUILT_IN` text: section, ring, connector, a connector's cap, line
/// link, a link's cap, portal marker, and a label by its host —
/// connector, line link, portal end.
pub fn do_document_resolve_built_in_by_flow() {
    let (mut builder, ids) = example();
    builder.edit_node(ids.a, |n| n.palette = None);
    let line_cap = ids.line.children(builder.tree().arena()).nth(1).unwrap();
    let line_span = ids.line.children(builder.tree().arena()).next().unwrap();
    let cases = [
        (ids.a_body_span, BUILT_IN.section),
        (ids.ring_span, BUILT_IN.ring.text),
        (ids.b_connector_span, BUILT_IN.connector.text),
        (ids.b_cap, BUILT_IN.connector.text),
        (line_span, BUILT_IN.link.text),
        (line_cap, BUILT_IN.link.text),
        (ids.b_label, BUILT_IN.label.on_connector),
        (ids.line_label, BUILT_IN.label.on_link),
        (ids.portal_label, BUILT_IN.label.on_portal_end),
        (ids.portal_source, BUILT_IN.portal_end.text),
    ];
    for (owner, expected) in cases {
        assert_eq!(
            text(&builder, owner).0,
            expected,
            "{:?}",
            builder.tree().arena()[owner].get().kind()
        );
    }
    // The flows really are told apart: no two of these agree.
    let distinct = [
        BUILT_IN.section,
        BUILT_IN.ring.text,
        BUILT_IN.connector.text,
        BUILT_IN.link.text,
        BUILT_IN.label.on_connector,
        BUILT_IN.label.on_link,
        BUILT_IN.portal_end.text,
    ];
    for (i, a) in distinct.iter().enumerate() {
        for b in &distinct[i + 1..] {
            assert_ne!(a, b);
        }
    }
    // A portal label reads like its marker, 1:1 (today's
    // `resolve_portal_endpoint_text_style`), and like no path label.
    assert_eq!(BUILT_IN.label.on_portal_end, BUILT_IN.portal_end.text);
    assert_ne!(BUILT_IN.label.on_portal_end, BUILT_IN.label.on_link);
    assert_eq!(BUILT_IN.label.on_portal_end.size, FontSize::Pt(50.0));
    assert_eq!(BUILT_IN.label.on_link.color, Rgba::opaque(0xaa, 0x88, 0xcc));
    assert_eq!(BUILT_IN.label.on_connector.color, Rgba::opaque(0x88, 0x88, 0x88));
}

#[test]
fn test_document_resolve_path_and_portal_text_tiers() {
    do_document_resolve_path_and_portal_text_tiers();
}

/// A connector's span takes the connector's size and color, then
/// `Defaults.connector`'s; a portal marker takes the end's, then
/// `Defaults.portal_end`'s — never the node's text (R4), which is set
/// to green 40 pt throughout. Controls: each tier removed hands the
/// field to the next.
pub fn do_document_resolve_path_and_portal_text_tiers() {
    let (mut builder, ids) = example();
    builder.edit_node(ids.a, |n| n.palette = None);
    builder.edit(ids.b_connector, |e| {
        if let DocElement::Connector(c) = e {
            c.path.flow.size = Some(20.0);
            c.path.flow.color = rgba(RED);
        }
    });
    for node in [ids.a, ids.b] {
        style(&mut builder, node, |s| {
            s.color = rgba(GREEN);
            s.size = Some(FontSize::Pt(40.0));
        });
    }
    defaults(&mut builder, |d| {
        d.connector.color = rgba(BLUE);
        d.connector.size = Some(30.0);
        d.portal_end.color = rgba(YELLOW);
        d.portal_end.size = Some(60.0);
    });
    let resolved = text(&builder, ids.b_connector_span).0;
    assert_eq!((resolved.size, resolved.color), (FontSize::Pt(20.0), RED));
    builder.edit(ids.b_connector, |e| {
        if let DocElement::Connector(c) = e {
            c.path.flow.size = None;
            c.path.flow.color = None;
        }
    });
    let resolved = text(&builder, ids.b_connector_span).0;
    assert_eq!((resolved.size, resolved.color), (FontSize::Pt(30.0), BLUE));
    defaults(&mut builder, |d| {
        d.connector.color = None;
        d.connector.size = None;
    });
    let resolved = text(&builder, ids.b_connector_span).0;
    assert_eq!(
        (resolved.size, resolved.color),
        (BUILT_IN.connector.text.size, BUILT_IN.connector.text.color)
    );

    builder.edit(ids.portal_source, |e| {
        if let DocElement::PortalEnd(end) = e {
            end.style.color = rgba(CYAN);
        }
    });
    assert_eq!(color(&builder, ids.portal_source), CYAN);
    builder.edit(ids.portal_source, |e| {
        if let DocElement::PortalEnd(end) = e {
            end.style.color = None;
        }
    });
    let resolved = text(&builder, ids.portal_source).0;
    assert_eq!((resolved.size, resolved.color), (FontSize::Pt(60.0), YELLOW));
}

#[test]
fn test_document_resolve_node_text_stays_out_of_other_flows() {
    do_document_resolve_node_text_stays_out_of_other_flows();
}

/// R4: a node's text size and color, `Defaults.span.size` and
/// `Defaults.node.text`'s size and color each leave a connector span, a
/// portal marker, a label and a ring at their own tier — and each still
/// reaches a section span (the control: the setting is live). The face
/// keeps the whole chain, and reaches all of them.
pub fn do_document_resolve_node_text_stays_out_of_other_flows() {
    let (base, ids) = example();
    let flows = [
        ids.b_connector_span,
        ids.portal_source,
        ids.b_label,
        ids.ring_span,
    ];
    let untouched: Vec<ResolvedText> = flows.iter().map(|&id| text(&base, id).0).collect();
    type Setting = fn(&mut DocumentBuilder, &Example);
    let settings: [(&str, Setting); 4] = [
        ("node text size", |b, ids| {
            for node in [ids.a, ids.b] {
                style(b, node, |s| s.size = Some(FontSize::Pt(40.0)));
            }
        }),
        ("node text color", |b, ids| {
            for node in [ids.a, ids.b] {
                style(b, node, |s| s.color = rgba(RED));
            }
        }),
        ("Defaults.span.size", |b, _| {
            defaults(b, |d| d.span.size = Some(FontSize::Pt(40.0)))
        }),
        ("Defaults.node.text", |b, _| {
            defaults(b, |d| {
                d.node.text.size = Some(FontSize::Pt(40.0));
                d.node.text.color = rgba(RED);
            })
        }),
    ];
    for (what, set) in settings {
        let (mut builder, _) = example();
        let section_before = text(&builder, ids.a_body_span).0;
        set(&mut builder, &ids);
        let section_after = text(&builder, ids.a_body_span).0;
        assert_ne!(section_before, section_after, "{what} must reach a section span");
        for (&id, expected) in flows.iter().zip(&untouched) {
            assert_eq!(
                text(&builder, id).0,
                *expected,
                "{what} must not reach a {:?}",
                builder.tree().arena()[id].get().kind()
            );
        }
    }
    let (mut builder, _) = example();
    defaults(&mut builder, |d| d.node.text.font = Some(AppFont::Novem));
    for id in flows.into_iter().chain([ids.a_body_span]) {
        assert_eq!(
            text(&builder, id).0.font,
            AppFont::Novem,
            "the face keeps the whole chain"
        );
    }
}

#[test]
fn test_document_resolve_cap_takes_its_path() {
    do_document_resolve_cap_takes_its_path();
}

/// A cap belongs to its path, as a ring to its frame: a connector's own
/// size and color reach its cap ahead of `Defaults.connector`, and the
/// cap's own color outranks the connector's. The cap records the
/// connector's style. Controls: without the connector's own style the
/// cap falls to `Defaults.connector`, then `BUILT_IN`.
pub fn do_document_resolve_cap_takes_its_path() {
    let (mut builder, ids) = example();
    defaults(&mut builder, |d| d.connector.color = rgba(BLUE));
    builder.edit(ids.b_connector, |e| {
        if let DocElement::Connector(c) = e {
            c.path.flow.color = rgba(RED);
            c.path.flow.size = Some(30.0);
        }
    });
    let (cap, reads) = text(&builder, ids.b_cap);
    assert_eq!((cap.size, cap.color), (FontSize::Pt(30.0), RED));
    assert!(reads.contains(&Input::Style(ids.b_connector)));
    style(&mut builder, ids.b_cap, |s| s.color = rgba(GREEN));
    let cap = text(&builder, ids.b_cap).0;
    assert_eq!(
        (cap.size, cap.color),
        (FontSize::Pt(30.0), GREEN),
        "the cap's own color first"
    );
    style(&mut builder, ids.b_cap, |s| s.color = None);
    builder.edit(ids.b_connector, |e| {
        if let DocElement::Connector(c) = e {
            c.path.flow.color = None;
            c.path.flow.size = None;
        }
    });
    let cap = text(&builder, ids.b_cap).0;
    assert_eq!((cap.size, cap.color), (BUILT_IN.connector.text.size, BLUE));
}

#[test]
fn test_document_resolve_ring_size_never_reads_points() {
    do_document_resolve_ring_size_never_reads_points();
}

/// A ring's size comes from the ring, the frame, `Defaults.ring`,
/// `Defaults.frame` and `BUILT_IN` — never from the span, the node,
/// `Defaults.span` or `Defaults.node`, which all set points here.
/// Control: a section span of the same node does take the node's points.
pub fn do_document_resolve_ring_size_never_reads_points() {
    let (mut builder, ids) = example();
    style(&mut builder, ids.ring_span, |s| s.size = Some(FontSize::Pt(9.0)));
    style(&mut builder, ids.a, |s| s.size = Some(FontSize::Pt(30.0)));
    defaults(&mut builder, |d| {
        d.span.size = Some(FontSize::Pt(11.0));
        d.node.text.size = Some(FontSize::Pt(13.0));
    });
    assert_eq!(text(&builder, ids.ring_span).0.size, FontSize::Rel(0.06));
    assert_eq!(BUILT_IN.ring.text.size, FontSize::Rel(0.06));
    assert_eq!(
        text(&builder, ids.a_body_span).0.size,
        FontSize::Pt(30.0),
        "a section span of the same node reads the node's points"
    );

    let steps: [(&str, FontSize); 4] = [
        ("defaults.frame", FontSize::Rel(0.2)),
        ("defaults.ring", FontSize::Rel(0.3)),
        ("frame", FontSize::Rel(0.4)),
        ("ring", FontSize::Rel(0.5)),
    ];
    for (tier, size) in steps {
        match tier {
            "defaults.frame" => defaults(&mut builder, |d| d.frame.text.size = Some(size)),
            "defaults.ring" => defaults(&mut builder, |d| d.ring.text.size = Some(size)),
            "frame" => style(&mut builder, ids.frame, |s| s.size = Some(size)),
            _ => style(&mut builder, ids.ring, |s| s.size = Some(size)),
        }
        assert_eq!(
            text(&builder, ids.ring_span).0.size,
            size,
            "{tier} outranks the tiers below it"
        );
    }
}

/// Give each of `nodes` the text color `palette(frame)`, so its text
/// reads the palette group its binding gives it at its depth.
fn frame_colored(builder: &mut DocumentBuilder, nodes: &[NodeId]) {
    for &id in nodes {
        style(builder, id, |s| {
            s.color = Some(ColorRef::Palette(PaletteField::Frame))
        });
    }
}

#[test]
fn test_document_resolve_palette_by_depth() {
    do_document_resolve_palette_by_depth();
}

/// The palette tier by depth: `a` binds the three-group palette, so a
/// color naming the group's `frame` is group 0's on `a`, group 1's on
/// `b` and group 2's on its grandchild `c`. Control: on `d`, which
/// nothing binds, the reference falls through to `Defaults.node`, and
/// an explicit color on `c` outranks it.
pub fn do_document_resolve_palette_by_depth() {
    let (mut builder, ids) = example();
    defaults(&mut builder, |d| d.node.text.color = rgba(MAGENTA));
    frame_colored(&mut builder, &[ids.a, ids.b, ids.c, ids.d]);
    assert_eq!(color(&builder, ids.a), Rgba::opaque(0, 10, 0));
    assert_eq!(color(&builder, ids.b), Rgba::opaque(0, 11, 0));
    assert_eq!(color(&builder, ids.c), Rgba::opaque(0, 12, 0));
    assert_eq!(
        color(&builder, ids.d),
        MAGENTA,
        "an unbound node falls through to Defaults"
    );
    style(&mut builder, ids.c, |s| s.color = rgba(RED));
    assert_eq!(color(&builder, ids.c), RED);
}

#[test]
fn test_document_resolve_palette_clamps_and_nearest_binding_wins() {
    do_document_resolve_palette_clamps_and_nearest_binding_wins();
}

/// Past the last group the last group applies; a binding nearer the
/// node wins over one further up.
pub fn do_document_resolve_palette_clamps_and_nearest_binding_wins() {
    let (mut builder, ids) = example();
    let short = builder.palette(PaletteData {
        name: "short".to_string(),
        groups: vec![palette_group(20), palette_group(21)],
    });
    builder.edit_node(ids.a, |n| n.palette = Some(short));
    frame_colored(&mut builder, &[ids.a, ids.b, ids.c]);
    let fill = |builder: &DocumentBuilder, id| color(builder, id);
    assert_eq!(fill(&builder, ids.b), Rgba::opaque(0, 21, 0));
    assert_eq!(
        fill(&builder, ids.c),
        Rgba::opaque(0, 21, 0),
        "depth 2 clamps to the last group"
    );

    builder.edit_node(ids.b, |n| n.palette = Some(PaletteId(0)));
    assert_eq!(
        fill(&builder, ids.b),
        Rgba::opaque(0, 10, 0),
        "b binds palette 0 itself: group 0"
    );
    assert_eq!(fill(&builder, ids.c), Rgba::opaque(0, 11, 0));
    assert_eq!(fill(&builder, ids.a), Rgba::opaque(0, 20, 0));
}

#[test]
fn test_document_resolve_palette_field_mapping() {
    do_document_resolve_palette_field_mapping();
}

/// Section text takes the group's `text`, the node's first section its
/// `title`, ring text its `frame`; connector, label, link and portal
/// text are not palette-colored. Control: a section color set on the
/// node outranks the palette tier.
pub fn do_document_resolve_palette_field_mapping() {
    let (mut builder, ids) = example();
    assert_eq!(color(&builder, ids.a_title_span), Rgba::opaque(10, 10, 0));
    assert_eq!(color(&builder, ids.a_body_span), Rgba::opaque(0, 0, 10));
    assert_eq!(
        color(&builder, ids.b_span),
        Rgba::opaque(11, 11, 0),
        "b's only section is its first"
    );
    assert_eq!(color(&builder, ids.ring_span), Rgba::opaque(0, 10, 0));
    assert_eq!(
        color(&builder, ids.b_connector_span),
        BUILT_IN.connector.text.color
    );
    assert_eq!(color(&builder, ids.b_label), BUILT_IN.label.on_connector.color);
    assert_eq!(color(&builder, ids.portal_source), BUILT_IN.portal_end.text.color);
    style(&mut builder, ids.a, |s| s.color = rgba(RED));
    assert_eq!(color(&builder, ids.a_body_span), RED);
}

#[test]
fn test_document_resolve_untitled_group_titles_with_text() {
    do_document_resolve_untitled_group_titles_with_text();
}

/// R5: a group with no `title` gives the first section its `text`, in
/// the palette tier — not a later tier. Control: with a `title`, the
/// first section takes it.
pub fn do_document_resolve_untitled_group_titles_with_text() {
    let (mut builder, ids) = example();
    defaults(&mut builder, |d| d.section.color = rgba(MAGENTA));
    assert_eq!(
        color(&builder, ids.a_title_span),
        Rgba::opaque(10, 10, 0),
        "titled: title"
    );
    let palette = builder.root().children(builder.tree().arena()).nth(1).unwrap();
    builder.edit(palette, |e| {
        if let DocElement::Palette(p) = e {
            p.groups[0].title = None;
        }
    });
    assert_eq!(
        color(&builder, ids.a_title_span),
        Rgba::opaque(0, 0, 10),
        "untitled: text, not Defaults"
    );
}

#[test]
fn test_document_resolve_first_section_reads_the_row() {
    do_document_resolve_first_section_reads_the_row();
}

/// Which section is first is a row read: a section span of `b` records
/// `Row(b)`, and adding a section ahead of it changes its color from
/// the group's `title` to its `text`. Control: a span whose own color is
/// set reads no row.
pub fn do_document_resolve_first_section_reads_the_row() {
    let (mut builder, ids) = example();
    let (resolved, reads) = text(&builder, ids.b_span);
    assert_eq!(resolved.color, Rgba::opaque(11, 11, 0));
    assert!(reads.contains(&Input::Row(ids.b)));
    builder.edit(ids.b_section, |e| {
        if let DocElement::Section(s) = e {
            s.channel = 5;
        }
    });
    builder.section(ids.b, 0);
    assert_eq!(
        color(&builder, ids.b_span),
        Rgba::opaque(0, 0, 11),
        "no longer the first section"
    );
    style(&mut builder, ids.b_span, |s| s.color = rgba(RED));
    assert!(!text(&builder, ids.b_span).1.contains(&Input::Row(ids.b)));
}

#[test]
fn test_document_resolve_explicit_palette_reference() {
    do_document_resolve_explicit_palette_reference();
}

/// An explicit `Palette(field)` resolves against the owning node's
/// binding, on any element — a connector span included — and with no
/// binding falls through to the next tier.
pub fn do_document_resolve_explicit_palette_reference() {
    let (mut builder, ids, section, span) = unbound_span();
    style(&mut builder, ids.b_connector_span, |s| {
        s.color = Some(ColorRef::Palette(PaletteField::Frame))
    });
    assert_eq!(color(&builder, ids.b_connector_span), Rgba::opaque(0, 11, 0));

    style(&mut builder, span, |s| {
        s.color = Some(ColorRef::Palette(PaletteField::Title))
    });
    style(&mut builder, section, |s| s.color = rgba(GREEN));
    assert_eq!(
        color(&builder, span),
        GREEN,
        "d has no binding, so the section's color applies"
    );
}

#[test]
fn test_document_resolve_var_through_the_active_variant() {
    do_document_resolve_var_through_the_active_variant();
}

/// A `Var` reads the active variant; switching the variant switches the
/// color. Control: an `Rgba` beside it does not change. A variable the
/// theme lacks falls through.
pub fn do_document_resolve_var_through_the_active_variant() {
    let (mut builder, ids, section, span) = unbound_span();
    style(&mut builder, span, |s| s.color = Some(ColorRef::Var(VarId(0))));
    style(&mut builder, ids.b_span, |s| s.color = rgba(RED));
    assert_eq!(color(&builder, span), Rgba::opaque(0xd4, 0xa2, 0x4c));
    let theme = builder.theme_id();
    builder.edit(theme, |e| {
        if let DocElement::Theme(theme) = e {
            theme.active = 1;
        }
    });
    assert_eq!(color(&builder, span), Rgba::opaque(0x8a, 0x5a, 0x00));
    assert_eq!(color(&builder, ids.b_span), RED);

    style(&mut builder, span, |s| s.color = Some(ColorRef::Var(VarId(9))));
    style(&mut builder, section, |s| s.color = rgba(GREEN));
    assert_eq!(color(&builder, span), GREEN);
}

#[test]
fn test_document_records_only_the_span_for_a_complete_rgba_span() {
    do_document_records_only_the_span_for_a_complete_rgba_span();
}

/// A span that sets every field, its color an `Rgba`, reads itself and
/// nothing else: not the theme, not its owners, not the defaults.
/// Control: the same span with a `Var` color also reads the theme.
pub fn do_document_records_only_the_span_for_a_complete_rgba_span() {
    let (mut builder, _, _, span) = unbound_span();
    style(&mut builder, span, |s| {
        *s = TextStyle {
            font: Some(AppFont::Novem),
            size: Some(FontSize::Pt(12.0)),
            color: rgba(RED),
            weight: Some(Weight::NORMAL),
            italic: Some(false),
        }
    });
    assert_eq!(text(&builder, span).1, vec![Input::Style(span)]);
    style(&mut builder, span, |s| s.color = Some(ColorRef::Var(VarId(0))));
    assert_eq!(text(&builder, span).1, vec![Input::Style(span), Input::Theme]);
}

#[test]
fn test_document_records_every_tier_it_looks_at() {
    do_document_records_every_tier_it_looks_at();
}

/// An unstyled section span of `b` reads every tier — its own style,
/// its section's, its node's, the node's row, the palette walk and the
/// defaults — even though most say nothing, and nothing off its chain:
/// not `b`'s connector, not `c`, not the other root node, and of `a`
/// only its `palette` field, not its style.
pub fn do_document_records_every_tier_it_looks_at() {
    let (builder, ids) = example();
    let reads = text(&builder, ids.b_span).1;
    for expected in [
        Input::Style(ids.b_span),
        Input::Style(ids.b_section),
        Input::Style(ids.b),
        Input::Row(ids.b),
        Input::Depth(ids.b),
        Input::PaletteBinding(ids.b),
        Input::PaletteBinding(ids.a),
        Input::Depth(ids.a),
        Input::Palette(PaletteId(0)),
        Input::Defaults,
    ] {
        assert!(
            reads.contains(&expected),
            "{expected:?} must be recorded: {reads:?}"
        );
    }
    for unexpected in [
        Input::Style(ids.b_connector),
        Input::Style(ids.c),
        Input::Style(ids.d),
        Input::Style(ids.frame),
        Input::Style(ids.a),
        Input::Row(ids.a),
        Input::Theme,
    ] {
        assert!(
            !reads.contains(&unexpected),
            "{unexpected:?} is off the chain: {reads:?}"
        );
    }
}

#[test]
fn test_document_records_the_palette_binding() {
    do_document_records_the_palette_binding();
}

/// A `Palette` color records the node's depth, the `palette` field of
/// every node of the walk up, the binding's depth and the palette — and the theme only when
/// the group's color is a `Var`. A span of a node with no binding walks
/// up and records no palette.
pub fn do_document_records_the_palette_binding() {
    let (mut builder, ids) = example();
    style(&mut builder, ids.b_span, |s| {
        *s = TextStyle {
            font: Some(AppFont::Novem),
            size: Some(FontSize::Pt(12.0)),
            color: Some(ColorRef::Palette(PaletteField::Text)),
            weight: Some(Weight::NORMAL),
            italic: Some(false),
        }
    });
    let reads = text(&builder, ids.b_span).1;
    assert_eq!(
        reads,
        vec![
            Input::Style(ids.b_span),
            Input::Depth(ids.b),
            Input::PaletteBinding(ids.b),
            Input::PaletteBinding(ids.a),
            Input::Depth(ids.a),
            Input::Palette(PaletteId(0)),
        ]
    );

    let palette = builder.root().children(builder.tree().arena()).nth(1).unwrap();
    builder.edit(palette, |e| {
        if let DocElement::Palette(p) = e {
            p.groups[1].text = PlainColor::Var(VarId(1));
        }
    });
    let (resolved, reads) = text(&builder, ids.b_span);
    assert_eq!(resolved.color, Rgba::opaque(0xe0, 0xe0, 0xe0));
    assert!(reads.contains(&Input::Theme));

    let (mut builder, ids, _, span) = unbound_span();
    style(&mut builder, span, |s| {
        s.color = Some(ColorRef::Palette(PaletteField::Text))
    });
    let reads = text(&builder, span).1;
    assert!(reads.contains(&Input::Depth(ids.d)));
    assert!(!reads.iter().any(|r| matches!(r, Input::Palette(_))));
}

#[test]
fn test_document_records_node_and_frame_reads() {
    do_document_records_node_and_frame_reads();
}

/// `resolve_node` with its shape set reads the node's style alone; with
/// the shape unset it reads `Defaults` too, and never a palette: a shape
/// is not palette-colored, and a node has no fill (V1-B12). A ring that
/// sets its whole style reads no frame; one that does not, does.
pub fn do_document_records_node_and_frame_reads() {
    let (mut builder, ids) = example();
    builder.edit_node(ids.b, |n| {
        n.style.shape = Some(Shape::Ellipse);
    });
    let mut reads = Vec::new();
    resolve_node(builder.tree(), ids.b, &mut reads);
    assert_eq!(reads, vec![Input::Style(ids.b)]);
    builder.edit_node(ids.b, |n| n.style.shape = None);
    let mut reads = Vec::new();
    resolve_node(builder.tree(), ids.b, &mut reads);
    assert_eq!(reads, vec![Input::Style(ids.b), Input::Defaults]);
    assert!(
        !reads
            .iter()
            .any(|read| matches!(read, Input::Palette(_) | Input::PaletteBinding(_))),
        "b is bound to a palette, and its node resolution reads none"
    );

    style(&mut builder, ids.ring, |s| {
        *s = TextStyle {
            font: Some(AppFont::Novem),
            size: Some(FontSize::Rel(0.1)),
            color: rgba(RED),
            weight: Some(Weight::NORMAL),
            italic: Some(false),
        }
    });
    let reads = text(&builder, ids.ring_span).1;
    assert_eq!(reads, vec![Input::Style(ids.ring_span), Input::Style(ids.ring)]);
    style(&mut builder, ids.ring, |s| s.italic = None);
    assert!(text(&builder, ids.ring_span).1.contains(&Input::Style(ids.frame)));
}

#[test]
fn test_document_resolve_connector_content() {
    do_document_resolve_connector_content();
}

/// A connector with no spans draws `Defaults.connector.content`, then
/// `BUILT_IN`'s; its own content outranks both. A connector with a span
/// draws its spans, whatever any tier's content says, and does not read
/// the defaults for it.
pub fn do_document_resolve_connector_content() {
    let (mut builder, ids) = example();
    let content = |builder: &DocumentBuilder, id| resolve_path(builder.tree(), id, &mut ()).content;
    assert_eq!(
        content(&builder, ids.c_connector),
        Content::Text("\u{00B7}".to_string())
    );
    defaults(&mut builder, |d| d.connector.content = Some("=".to_string()));
    assert_eq!(content(&builder, ids.c_connector), Content::Text("=".to_string()));
    builder.edit(ids.c_connector, |e| {
        if let DocElement::Connector(c) = e {
            c.path.flow.content = Some("~".to_string());
        }
    });
    assert_eq!(content(&builder, ids.c_connector), Content::Text("~".to_string()));

    assert_eq!(content(&builder, ids.b_connector), Content::Spans);
    builder.edit(ids.b_connector, |e| {
        if let DocElement::Connector(c) = e {
            c.path.flow.content = Some("~".to_string());
            c.path.flow.fit = Some(FitMode::Spread);
            c.path.flow.spacing = Some(1.0);
        }
    });
    let mut reads = Vec::new();
    let resolved = resolve_path(builder.tree(), ids.b_connector, &mut reads);
    assert_eq!(resolved.content, Content::Spans);
    assert_eq!(
        reads,
        vec![Input::Style(ids.b_connector), Input::Row(ids.b_connector)]
    );
}

#[test]
fn test_document_resolve_portal_end_content_and_position() {
    do_document_resolve_portal_end_content_and_position();
}

/// A portal end with no spans draws `BUILT_IN`'s marker, then
/// `Defaults.portal_end.content`, then its own; one with a span draws
/// it. With no `border_t` anywhere it faces the other end; a set one
/// places it.
pub fn do_document_resolve_portal_end_content_and_position() {
    let (mut builder, ids) = example();
    let end = |builder: &DocumentBuilder, id| resolve_portal_end(builder.tree(), id, &mut ());
    assert_eq!(
        end(&builder, ids.portal_target).content,
        Content::Text("\u{25C8}".to_string())
    );
    assert_eq!(end(&builder, ids.portal_target).border_t, BorderPoint::Auto);
    defaults(&mut builder, |d| {
        d.portal_end.content = Some("*".to_string());
        d.portal_end.border_t = Some(0.25);
    });
    assert_eq!(
        end(&builder, ids.portal_target).content,
        Content::Text("*".to_string())
    );
    assert_eq!(end(&builder, ids.portal_target).border_t, BorderPoint::At(0.25));
    builder.edit(ids.portal_target, |e| {
        if let DocElement::PortalEnd(end) = e {
            end.style.content = Some("+".to_string());
            end.style.border_t = Some(0.75);
        }
    });
    assert_eq!(
        end(&builder, ids.portal_target).content,
        Content::Text("+".to_string())
    );
    assert_eq!(end(&builder, ids.portal_target).border_t, BorderPoint::At(0.75));
    assert_eq!(end(&builder, ids.portal_source).content, Content::Spans);
}

#[test]
fn test_document_resolve_ring_layout_chain() {
    do_document_resolve_ring_layout_chain();
}

/// A ring's span, sides and spacing (#291) each run ring → frame →
/// `Defaults.ring` → `Defaults.frame` → `BUILT_IN`'s frame, on their
/// own: every tier sets all three to its own values, and as each tier
/// is cleared the next gives all three — and a tier that sets one only
/// gives that one, the others passing it by. `sides: Off` is a value,
/// not an absence: a ring's `Off` hides the frame's ornament.
pub fn do_document_resolve_ring_layout_chain() {
    let (mut builder, ids) = example();
    let ring = |b: &DocumentBuilder| {
        let r = resolve_ring(b.tree(), ids.ring, &mut ());
        (r.span.get(), r.sides.map(|s| s.content), r.spacing)
    };
    let on = |content: &str| Sides::On {
        content: content.to_string(),
        color: None,
    };
    let tier = |k: f32, content: &str| RingLayoutStyle {
        span: LineSpan::new(k / 10.0),
        sides: Some(on(content)),
        spacing: Some(k),
    };
    assert_eq!(BUILT_IN.frame.sides, Sides::Off);
    let built_in = (BUILT_IN.frame.span.get(), None, BUILT_IN.frame.spacing);
    assert_eq!(ring(&builder), built_in);
    assert_eq!(built_in, (1.0, None, 0.0), "whole, no sides, no spacing");
    defaults(&mut builder, |d| {
        d.frame.ring_layout = tier(1.0, "a");
        d.ring.ring_layout = tier(2.0, "b");
    });
    builder.edit(ids.frame, |e| {
        if let DocElement::Frame(f) = e {
            f.style.ring_layout = tier(3.0, "c");
        }
    });
    let edit_ring = |b: &mut DocumentBuilder, layout: RingLayoutStyle| {
        b.edit(ids.ring, |e| {
            if let DocElement::Ring(r) = e {
                r.style.ring_layout = layout;
            }
        });
    };
    edit_ring(&mut builder, tier(4.0, "d"));
    assert_eq!(
        ring(&builder),
        (0.4, Some("d".to_string()), 4.0),
        "the ring's own"
    );
    edit_ring(&mut builder, RingLayoutStyle::default());
    assert_eq!(ring(&builder), (0.3, Some("c".to_string()), 3.0), "the frame's");
    builder.edit(ids.frame, |e| {
        if let DocElement::Frame(f) = e {
            f.style.ring_layout = RingLayoutStyle::default();
        }
    });
    assert_eq!(ring(&builder), (0.2, Some("b".to_string()), 2.0), "Defaults.ring");
    defaults(&mut builder, |d| d.ring.ring_layout = RingLayoutStyle::default());
    assert_eq!(
        ring(&builder),
        (0.1, Some("a".to_string()), 1.0),
        "Defaults.frame"
    );

    // Each field on its own: the ring sets its span only, the frame its
    // sides only; the spacing comes from `Defaults.frame`.
    edit_ring(
        &mut builder,
        RingLayoutStyle {
            span: Some(LineSpan::NATURAL),
            ..RingLayoutStyle::default()
        },
    );
    builder.edit(ids.frame, |e| {
        if let DocElement::Frame(f) = e {
            f.style.ring_layout.sides = Some(on("e"));
        }
    });
    assert_eq!(ring(&builder), (0.0, Some("e".to_string()), 1.0));
    edit_ring(
        &mut builder,
        RingLayoutStyle {
            sides: Some(Sides::Off),
            ..RingLayoutStyle::default()
        },
    );
    assert_eq!(
        ring(&builder),
        (0.1, None, 1.0),
        "a ring's Off hides the frame's ornament"
    );
    defaults(&mut builder, |d| d.frame.ring_layout = RingLayoutStyle::default());
    edit_ring(&mut builder, RingLayoutStyle::default());
    builder.edit(ids.frame, |e| {
        if let DocElement::Frame(f) = e {
            f.style.ring_layout = RingLayoutStyle::default();
        }
    });
    assert_eq!(ring(&builder), built_in, "every tier cleared");
}

#[test]
fn test_document_resolve_ring_sides_color() {
    do_document_resolve_ring_sides_color();
}

/// A ring's sides ornament takes its own color when it has one, else
/// the ring's text color: both resolved, a theme variable through the
/// active variant. Controls: the ornament's color removed hands it the
/// ring's; the ring's text color changed moves it along.
pub fn do_document_resolve_ring_sides_color() {
    let (mut builder, ids) = example();
    builder.edit_node(ids.a, |n| n.palette = None);
    let sides = |b: &DocumentBuilder| resolve_ring(b.tree(), ids.ring, &mut ()).sides.map(|s| s.color);
    let set = |b: &mut DocumentBuilder, color: Option<ColorRef>| {
        b.edit(ids.ring, |e| {
            if let DocElement::Ring(r) = e {
                r.style.ring_layout.sides = Some(Sides::On {
                    content: "✦".to_string(),
                    color,
                });
            }
        });
    };
    style(&mut builder, ids.ring, |s| s.color = rgba(RED));
    assert_eq!(sides(&builder), None, "no sides, no color");
    set(&mut builder, rgba(GREEN));
    assert_eq!(sides(&builder), Some(GREEN), "its own");
    set(&mut builder, None);
    assert_eq!(sides(&builder), Some(RED), "the ring's text color");
    style(&mut builder, ids.ring, |s| s.color = rgba(BLUE));
    assert_eq!(sides(&builder), Some(BLUE), "the ring's text color, again");
}

#[test]
fn test_document_resolve_ring_sides_var_records_the_theme() {
    do_document_resolve_ring_sides_var_records_the_theme();
}

/// A sides color given by a theme variable resolves through the active
/// variant and records the theme read, so a change to the variable
/// marks the ring's resolution stale. Control: the same ring with a
/// literal color records no theme read through its sides.
pub fn do_document_resolve_ring_sides_var_records_the_theme() {
    let (mut builder, ids) = example();
    builder.edit_node(ids.a, |n| n.palette = None);
    style(&mut builder, ids.ring, |s| s.color = rgba(RED));
    let set = |b: &mut DocumentBuilder, color: ColorRef| {
        b.edit(ids.ring, |e| {
            if let DocElement::Ring(r) = e {
                r.style.ring_layout.sides = Some(Sides::On {
                    content: "✦".to_string(),
                    color: Some(color),
                });
            }
        });
    };
    let theme_reads = |b: &DocumentBuilder| {
        let mut reads: Vec<Input> = Vec::new();
        let ring = resolve_ring(b.tree(), ids.ring, &mut reads);
        (
            ring.sides.map(|s| s.color),
            reads.iter().filter(|r| **r == Input::Theme).count(),
        )
    };
    set(&mut builder, ColorRef::Rgba(GREEN));
    assert_eq!(
        theme_reads(&builder),
        (Some(GREEN), 0),
        "a literal reads no theme"
    );
    set(&mut builder, ColorRef::Var(VarId(0)));
    let (color, reads) = theme_reads(&builder);
    assert!(reads > 0, "a variable records the theme read");
    let theme = builder.tree().arena().iter().find_map(|(_, n)| match n.get() {
        DocElement::Theme(t) => Some(t.clone()),
        _ => None,
    });
    let expected = theme.and_then(|t| {
        t.variants
            .get(usize::from(t.active))
            .and_then(|v| v.values.first().copied())
    });
    assert_eq!(color, expected, "the active variant's value");
}

#[test]
fn test_document_resolve_ring_once_filler_is_its_sides() {
    do_document_resolve_ring_once_filler_is_its_sides();
}

/// A ring whose fit is a `once` with a filler, and whose sides are unset
/// all the way down, has the filler as its sides, in its text color
/// (ruling R34). Controls: sides set to an ornament win, and so does
/// `false`; a `once` with no filler has none. A filler or sides that
/// `check_ornament` refuses — blank, zero-width, past 256 bytes — is
/// none, so a resolved ornament always has something to draw (ruling
/// R35.4).
pub fn do_document_resolve_ring_once_filler_is_its_sides() {
    let (mut builder, ids) = example();
    builder.edit_node(ids.a, |n| n.palette = None);
    style(&mut builder, ids.ring, |s| s.color = rgba(RED));
    let edit = |b: &mut DocumentBuilder, fit: FitMode, sides: Option<Sides>| {
        b.edit(ids.ring, |e| {
            if let DocElement::Ring(r) = e {
                r.style.fit = Some(fit);
                r.style.ring_layout.sides = sides;
            }
        });
    };
    let sides = |b: &DocumentBuilder| {
        resolve_ring(b.tree(), ids.ring, &mut ())
            .sides
            .map(|s| (s.content, s.color))
    };
    let filler = || FitMode::Once {
        filler: Some("·".to_string()),
    };
    edit(&mut builder, filler(), None);
    assert_eq!(
        sides(&builder),
        Some(("·".to_string(), RED)),
        "the filler, in the ring's color"
    );
    edit(
        &mut builder,
        filler(),
        Some(Sides::On {
            content: "✦".to_string(),
            color: None,
        }),
    );
    assert_eq!(
        sides(&builder),
        Some(("✦".to_string(), RED)),
        "an explicit ornament wins"
    );
    edit(&mut builder, filler(), Some(Sides::Off));
    assert_eq!(sides(&builder), None, "an explicit false wins");
    edit(&mut builder, FitMode::Once { filler: None }, None);
    assert_eq!(sides(&builder), None, "no filler, no sides");
    // Ruling R35.4: what resolves always has something to draw, however
    // the document was built. A loaded or validated one never holds
    // these; one built in code can.
    for bad in ["", "   ", "\u{200B}", &"·".repeat(4000)] {
        let once = FitMode::Once {
            filler: Some(bad.to_string()),
        };
        edit(&mut builder, once, None);
        assert_eq!(sides(&builder), None, "a filler {bad:.12?} is no sides");
        edit(
            &mut builder,
            FitMode::Once { filler: None },
            Some(Sides::On {
                content: bad.to_string(),
                color: None,
            }),
        );
        assert_eq!(sides(&builder), None, "sides {bad:.12?} are none");
    }
}

#[test]
fn test_document_resolve_frame_ring_path_and_label() {
    do_document_resolve_frame_ring_path_and_label();
}

/// The other resolvers' chains: a frame is shown by default and its
/// own `visible` outranks `Defaults.frame`'s; a ring's fit comes from
/// the ring, the frame, `Defaults.ring`, `Defaults.frame`, `BUILT_IN`;
/// a path's fit and spacing, and a label's position, from the element,
/// then `Defaults`, then `BUILT_IN`.
pub fn do_document_resolve_frame_ring_path_and_label() {
    let (mut builder, ids) = example();
    let frame = |b: &DocumentBuilder| resolve_frame(b.tree(), ids.frame, &mut ());
    assert!(frame(&builder).visible, "D25: shown unless it says otherwise");
    assert_eq!(frame(&builder).fit, FitMode::Fit { min: 0.5, max: 2.0 });
    defaults(&mut builder, |d| d.frame.visible = Some(false));
    assert!(!frame(&builder).visible);
    builder.edit(ids.frame, |e| {
        if let DocElement::Frame(f) = e {
            f.style.visible = Some(true);
        }
    });
    assert!(frame(&builder).visible);

    let ring_fit = |b: &DocumentBuilder| resolve_ring(b.tree(), ids.ring, &mut ()).fit;
    assert_eq!(ring_fit(&builder), BUILT_IN.frame.fit);
    defaults(&mut builder, |d| d.frame.fit = Some(FitMode::Repeat));
    assert_eq!(ring_fit(&builder), FitMode::Repeat);
    defaults(&mut builder, |d| d.ring.fit = Some(FitMode::Spread));
    assert_eq!(ring_fit(&builder), FitMode::Spread);
    builder.edit(ids.frame, |e| {
        if let DocElement::Frame(f) = e {
            f.style.fit = Some(FitMode::Once { filler: None });
        }
    });
    assert_eq!(ring_fit(&builder), FitMode::Once { filler: None });
    builder.edit(ids.ring, |e| {
        if let DocElement::Ring(r) = e {
            r.style.fit = Some(FitMode::Fit { min: 1.0, max: 1.5 });
        }
    });
    assert_eq!(ring_fit(&builder), FitMode::Fit { min: 1.0, max: 1.5 });

    let path = |b: &DocumentBuilder| resolve_path(b.tree(), ids.line, &mut ());
    assert_eq!(
        (path(&builder).fit, path(&builder).spacing),
        (FitMode::Repeat, 0.0)
    );
    defaults(&mut builder, |d| {
        d.link.spacing = Some(2.0);
        d.connector.spacing = Some(9.0);
    });
    assert_eq!(
        path(&builder).spacing,
        2.0,
        "a link reads Defaults.link, not Defaults.connector"
    );

    let label = |b: &DocumentBuilder| resolve_label(b.tree(), ids.b_label, &mut ());
    assert_eq!((label(&builder).t, label(&builder).offset), (0.5, 0.0));
    defaults(&mut builder, |d| d.label.t = Some(0.2));
    assert_eq!(label(&builder).t, 0.2);
    builder.edit(ids.b_label, |e| {
        if let DocElement::Label(l) = e {
            l.style.t = Some(0.9);
        }
    });
    assert_eq!(label(&builder).t, 0.9);
}

#[test]
fn test_document_resolve_node_chain() {
    do_document_resolve_node_chain();
}

/// A node's shape: its own, then `Defaults.node`, then `BUILT_IN` — a
/// sharp rectangle.
pub fn do_document_resolve_node_chain() {
    let (mut builder, ids) = example();
    let node = |b: &DocumentBuilder| resolve_node(b.tree(), ids.d, &mut ());
    assert_eq!(
        node(&builder).shape,
        Shape::Rectangle {
            radius: Percent::new(0)
        }
    );
    defaults(&mut builder, |d| {
        d.node.shape = Some(Shape::Ellipse);
    });
    assert_eq!(node(&builder).shape, Shape::Ellipse);
    builder.edit_node(ids.d, |n| {
        n.style.shape = Some(Shape::Rectangle {
            radius: Percent::new(40),
        });
    });
    assert_eq!(
        node(&builder).shape,
        Shape::Rectangle {
            radius: Percent::new(40)
        }
    );
}

#[test]
fn test_document_resolve_background() {
    do_document_resolve_background();
}

/// The map's background (V1-B12): black when the map gives none, a
/// literal as it is, and a theme variable in the active variant —
/// opaque, whatever alpha the variable holds, since nothing is drawn
/// behind it. A literal reads the map's style alone; a variable reads
/// the theme too, so a variant switch dirties it. Controls: switching
/// the variant changes a variable background and leaves a literal one
/// as it was; a variable the theme lacks falls back to black.
pub fn do_document_resolve_background() {
    use crate::document::defaults::DEFAULT_BACKGROUND;
    use crate::document::resolve::resolve_background;

    let (mut builder, _) = example();
    let root = builder.root();
    let background = |builder: &DocumentBuilder| {
        let mut reads = Vec::new();
        let color = resolve_background(builder.tree(), &mut reads);
        (color, reads)
    };
    assert_eq!(
        background(&builder),
        (DEFAULT_BACKGROUND, vec![Input::Style(root)])
    );
    assert_eq!(DEFAULT_BACKGROUND, Rgba::opaque(0, 0, 0), "black when unset");

    let set = |builder: &mut DocumentBuilder, color: PlainColor| {
        builder.edit(root, |e| {
            if let DocElement::Map(map) = e {
                map.background = color;
            }
        });
    };
    set(&mut builder, PlainColor::Rgba(Rgba::opaque(1, 2, 3)));
    assert_eq!(
        background(&builder),
        (Rgba::opaque(1, 2, 3), vec![Input::Style(root)])
    );

    builder.theme(ThemeData {
        variables: vec!["bg".to_string()],
        variants: vec![
            ThemeVariant {
                name: "dark".to_string(),
                values: vec![Rgba::new(10, 20, 30, 0x40)],
            },
            ThemeVariant {
                name: "light".to_string(),
                values: vec![Rgba::opaque(200, 210, 220)],
            },
        ],
        active: 0,
    });
    let literal = background(&builder).0;
    set(&mut builder, PlainColor::Var(VarId(0)));
    assert_eq!(
        background(&builder),
        (Rgba::opaque(10, 20, 30), vec![Input::Style(root), Input::Theme]),
        "the variable's alpha is not applied"
    );
    let theme = builder.theme_id();
    builder.edit(theme, |e| {
        if let DocElement::Theme(theme) = e {
            theme.active = 1;
        }
    });
    assert_eq!(
        background(&builder).0,
        Rgba::opaque(200, 210, 220),
        "the active variant's"
    );
    set(&mut builder, PlainColor::Rgba(Rgba::opaque(1, 2, 3)));
    assert_eq!(
        background(&builder).0,
        literal,
        "a literal does not follow the variant"
    );
    set(&mut builder, PlainColor::Var(VarId(7)));
    assert_eq!(
        background(&builder).0,
        DEFAULT_BACKGROUND,
        "a variable the theme lacks"
    );
}

#[test]
fn test_document_resolve_carried_binding_resolves_what_the_climb_does() {
    do_document_resolve_carried_binding_resolves_what_the_climb_does();
}

/// A walk that carries each node's palette binding down with
/// [`carry_binding`](crate::document::resolve::carry_binding) resolves
/// every text and every ring to exactly what the one-off climb resolves,
/// and records the one carried value of the owning node in place of the
/// climb's `palette` reads of every node above it. The example's palette
/// colors three depths differently, and a node two levels below the
/// binding takes the group two levels down, so a carried value one level
/// off resolves another color.
pub fn do_document_resolve_carried_binding_resolves_what_the_climb_does() {
    use crate::document::element::DocKind;
    use crate::document::ids::owning_node;
    use crate::document::resolve::{
        carry_binding, resolve_ring_in, resolve_text_in, Binding, CarriedBinding,
    };
    let (builder, ids) = example();
    let tree = builder.tree();
    // Top-down: the tree's descendants are in pre-order, so a node's
    // parent is carried before the node is.
    let mut carried: rustc_hash::FxHashMap<NodeId, Option<CarriedBinding>> = Default::default();
    for id in tree.descendants() {
        if tree.arena()[id].get().kind() != DocKind::Node {
            continue;
        }
        let parent = id
            .parent(tree.arena())
            .and_then(|p| carried.get(&p).copied())
            .flatten();
        carried.insert(id, carry_binding(tree, id, parent, &mut ()));
    }
    assert_eq!(
        carried[&ids.c],
        Some(CarriedBinding {
            palette: crate::document::values::PaletteId(0),
            steps: 2
        }),
        "c is two levels below a, which binds the palette"
    );
    assert_eq!(
        resolve_text(tree, ids.b_span, &mut ()).color,
        Rgba::opaque(11, 11, 0),
        "the fixture colors b's span, its first section's, from its group's title: the palette tier is reached"
    );
    let mut compared = 0;
    for id in tree.descendants() {
        let element = tree.arena()[id].get();
        let Some(node) = owning_node(tree, id) else {
            continue;
        };
        let binding = Binding::Carried(carried[&node]);
        let holds_text = element.text_style().is_some()
            || matches!(
                element.kind(),
                DocKind::Connector | DocKind::Link | DocKind::PortalEnd
            );
        if holds_text {
            assert_eq!(
                resolve_text_in(tree, id, binding, &mut ()),
                resolve_text(tree, id, &mut ()),
                "the text of {:?}",
                element.kind()
            );
            compared += 1;
        }
        if element.kind() == DocKind::Ring {
            assert_eq!(
                resolve_ring_in(tree, id, binding, &mut ()),
                crate::document::resolve::resolve_ring(tree, id, &mut ()),
                "the ring"
            );
        }
    }
    assert!(compared > 20, "every text of the example compared: {compared}");

    // The reads: the carried value of the owning node, and nothing above.
    // (That it climbs nothing is counted by the projection's deep-chain
    // test, which reads the climb's test-only step counter.)
    let mut carried_reads: Vec<Input> = Vec::new();
    resolve_text_in(tree, ids.b_span, Binding::Carried(carried[&ids.b]), &mut carried_reads);
    assert!(carried_reads.contains(&Input::Binding(ids.b)));
    assert!(
        !carried_reads.contains(&Input::PaletteBinding(ids.a)),
        "nothing above b is read: {carried_reads:?}"
    );
    let mut climbed_reads: Vec<Input> = Vec::new();
    resolve_text(tree, ids.b_span, &mut climbed_reads);
    assert!(
        climbed_reads.contains(&Input::PaletteBinding(ids.a)),
        "and reads the binding node's palette field: {climbed_reads:?}"
    );
}
