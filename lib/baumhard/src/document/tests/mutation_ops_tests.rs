// SPDX-License-Identifier: MPL-2.0

//! Every op of `DocMutation` writes exactly the field it names.
//!
//! Each case applies one op to an element whose every field holds a
//! value of its own, and compares the **whole** result with the element
//! as the test edits it by hand — the field the op names set, nothing
//! else touched. So a case fails if the op misses its field or writes
//! any other, and it first asserts that the expected element differs
//! from the original, so an op that changes nothing cannot pass by
//! accident. The relative ops' cases include results past their bound,
//! which come back clamped, and an index the element lacks, which
//! changes nothing.

use crate::document::bounds::{MAX_CANVAS_COORD, MAX_FONT_SIZE_PT, MIN_FONT_SIZE_PT, MIN_REL_SIZE};
use crate::document::element::{
    CapData, ConnectorData, DefaultsData, DocElement, FlowStyle, FrameData, FrameStyle, LabelData,
    LabelStyle, LinkData, MapData, NodeData, NodeStyle, PaletteData, PathData, PortalEndData, PortalEndStyle,
    RingData, RingStyle, SectionData, SpanData, TextStyle, ThemeData, ThemeVariant, Triggers,
};
use crate::document::ids::NodeIdent;
use crate::document::mutation::inherited::{inherited_seed, DocInherited};
use crate::document::mutation::ops::{
    DefaultsOp, DocMutation, FrameOp, LabelOp, LinkOp, MapOp, NodeOp, PaletteOp, PathOp, PortalEndOp, RingOp,
    SectionOp, SpanOp, TextOp, ThemeOp,
};
use crate::document::tests::test_support::{example, palette_group};
use crate::document::values::{
    Align, Anchor, ColorRef, Direction, Edge, FitMode, FontSize, LinkKind, LinkTarget, PaletteId, Percent,
    Rgba, Shape, StartPoint, VarId, VerbId, Weight, ZoomWindow,
};
use crate::engine::mutation::Mutation;
use crate::font::fonts::AppFont;
use glam::Vec2;

/// A text style with every field set.
fn rich_text(size: FontSize) -> TextStyle {
    TextStyle {
        font: Some(AppFont::Any),
        size: Some(size),
        color: Some(ColorRef::Rgba(Rgba::opaque(1, 2, 3))),
        weight: Some(Weight::BOLD),
        italic: Some(false),
    }
}

/// A path style with every field set.
fn rich_flow() -> FlowStyle {
    FlowStyle {
        fit: Some(FitMode::Spread),
        spacing: Some(1.5),
        size: Some(14.0),
        color: Some(ColorRef::Var(VarId(0))),
        content: Some("-".to_string()),
    }
}

fn rich_node() -> DocElement {
    DocElement::Node(NodeData {
        ident: NodeIdent(9),
        channel: 2,
        position: Vec2::new(10.0, -20.0),
        min_size: Some(Vec2::new(100.0, 50.0)),
        style: NodeStyle {
            shape: Some(Shape::Ellipse),
            fill: Some(ColorRef::Rgba(Rgba::opaque(9, 9, 9))),
            fill_fade: Some(2.0),
            text: rich_text(FontSize::Pt(18.0)),
        },
        palette: Some(PaletteId(0)),
        zoom: Some(ZoomWindow {
            min: Some(0.5),
            max: None,
        }),
        folded: false,
        hidden: false,
        opacity: 0.75,
        on: Triggers {
            click: Some(VerbId(0)),
            hover: None,
        },
    })
}

fn rich_section() -> DocElement {
    DocElement::Section(SectionData {
        channel: 1,
        offset: Vec2::new(4.0, 5.0),
        min_size: Some(Vec2::new(30.0, 10.0)),
        align: Some(Align::Center),
        text: rich_text(FontSize::Pt(16.0)),
    })
}

fn rich_span() -> DocElement {
    DocElement::Span(SpanData {
        text: "a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}e\u{0301}b".to_string(),
        style: rich_text(FontSize::Pt(12.0)),
        link: Some(LinkTarget::Url("https://example.org".to_string())),
    })
}

fn rich_frame() -> DocElement {
    DocElement::Frame(FrameData {
        style: FrameStyle {
            visible: Some(true),
            start: Some(StartPoint {
                edge: Edge::Left,
                at: 0.25,
            }),
            direction: Some(Direction::Clockwise),
            fit: Some(FitMode::Repeat),
            gap: Some(1.0),
            inset: Some(2.0),
            text: rich_text(FontSize::Rel(0.05)),
        },
    })
}

fn rich_ring() -> DocElement {
    DocElement::Ring(RingData {
        channel: 3,
        style: RingStyle {
            fit: Some(FitMode::Spread),
            text: rich_text(FontSize::Rel(0.07)),
        },
    })
}

fn rich_path() -> PathData {
    PathData {
        flow: rich_flow(),
        anchors: (Anchor::Top, Anchor::Bottom),
        control_points: vec![Vec2::new(1.0, 2.0), Vec2::new(3.0, 4.0)],
        visible: true,
        zoom: None,
    }
}

fn rich_connector() -> DocElement {
    DocElement::Connector(ConnectorData { path: rich_path() })
}

fn rich_link() -> DocElement {
    let (_, ids) = example();
    DocElement::Link(LinkData {
        to: ids.c,
        kind: LinkKind::Line,
        path: rich_path(),
    })
}

fn rich_cap() -> DocElement {
    DocElement::CapEnd(CapData {
        text: rich_text(FontSize::Pt(10.0)),
    })
}

fn rich_label() -> DocElement {
    DocElement::Label(LabelData {
        style: LabelStyle {
            t: Some(0.3),
            offset: Some(-4.0),
            text: rich_text(FontSize::Pt(11.0)),
        },
    })
}

fn rich_portal_end() -> DocElement {
    DocElement::PortalEnd(PortalEndData {
        end: 1,
        style: PortalEndStyle {
            border_t: Some(0.6),
            offset: Some(3.0),
            color: Some(ColorRef::Rgba(Rgba::opaque(5, 5, 5))),
            size: Some(40.0),
            content: Some("*".to_string()),
        },
    })
}

fn rich_theme() -> DocElement {
    DocElement::Theme(ThemeData {
        variables: vec!["accent".to_string(), "fg".to_string()],
        variants: vec![
            ThemeVariant {
                name: "dark".to_string(),
                values: vec![Rgba::opaque(1, 1, 1), Rgba::opaque(2, 2, 2)],
            },
            ThemeVariant {
                name: "light".to_string(),
                values: vec![Rgba::opaque(3, 3, 3), Rgba::opaque(4, 4, 4)],
            },
        ],
        active: 0,
    })
}

fn rich_palette() -> DocElement {
    DocElement::Palette(PaletteData {
        name: "rainbow".to_string(),
        groups: vec![palette_group(1), palette_group(2)],
    })
}

/// A state for ops that read none: the example map's, outside it.
fn outside() -> DocInherited {
    let (builder, _) = example();
    inherited_seed(builder.tree(), builder.root())
}

/// Apply `op` to `before` with `inherited`, and hold the result to
/// `before` as `want` edits it.
fn case(
    name: &str,
    before: &DocElement,
    op: DocMutation,
    inherited: &DocInherited,
    want: impl FnOnce(&mut DocElement),
) {
    let mut expected = before.clone();
    want(&mut expected);
    assert_ne!(&expected, before, "{name}: the case must change something");
    let mut got = before.clone();
    op.apply_to(&mut got, inherited);
    assert_eq!(got, expected, "{name}");
}

/// Apply `op` to `before` and require it unchanged.
fn unchanged(name: &str, before: &DocElement, op: DocMutation, inherited: &DocInherited) {
    let mut got = before.clone();
    op.apply_to(&mut got, inherited);
    assert_eq!(&got, before, "{name}: nothing changes");
}

/// The edit a text op's case makes by hand.
type TextEdit = Box<dyn Fn(&mut TextStyle)>;

fn node(e: &mut DocElement) -> &mut NodeData {
    match e {
        DocElement::Node(n) => n,
        other => panic!("not a node: {other:?}"),
    }
}

fn section(e: &mut DocElement) -> &mut SectionData {
    match e {
        DocElement::Section(s) => s,
        other => panic!("not a section: {other:?}"),
    }
}

fn span(e: &mut DocElement) -> &mut SpanData {
    match e {
        DocElement::Span(s) => s,
        other => panic!("not a span: {other:?}"),
    }
}

fn frame(e: &mut DocElement) -> &mut FrameStyle {
    match e {
        DocElement::Frame(f) => &mut f.style,
        other => panic!("not a frame: {other:?}"),
    }
}

fn ring(e: &mut DocElement) -> &mut RingData {
    match e {
        DocElement::Ring(r) => r,
        other => panic!("not a ring: {other:?}"),
    }
}

fn path(e: &mut DocElement) -> &mut PathData {
    match e {
        DocElement::Connector(c) => &mut c.path,
        DocElement::Link(l) => &mut l.path,
        other => panic!("not a path: {other:?}"),
    }
}

fn label_style(e: &mut DocElement) -> &mut LabelStyle {
    match e {
        DocElement::Label(l) => &mut l.style,
        other => panic!("not a label: {other:?}"),
    }
}

fn end_style(e: &mut DocElement) -> &mut PortalEndStyle {
    match e {
        DocElement::PortalEnd(p) => &mut p.style,
        other => panic!("not a portal end: {other:?}"),
    }
}

fn text(e: &mut DocElement) -> &mut TextStyle {
    match e {
        DocElement::Node(n) => &mut n.style.text,
        DocElement::Section(s) => &mut s.text,
        DocElement::Span(s) => &mut s.style,
        DocElement::Frame(f) => &mut f.style.text,
        DocElement::Ring(r) => &mut r.style.text,
        DocElement::CapStart(c) | DocElement::CapEnd(c) => &mut c.text,
        DocElement::Label(l) => &mut l.style.text,
        other => panic!("no text style: {other:?}"),
    }
}

#[test]
fn test_document_node_ops_write_their_field() {
    do_document_node_ops_write_their_field();
}

/// Every `NodeOp`, on a node whose every field is set.
pub fn do_document_node_ops_write_their_field() {
    let n = rich_node();
    let s = outside();
    let v = Vec2::new(-7.0, 8.0);
    case(
        "SetPosition",
        &n,
        DocMutation::Node(NodeOp::SetPosition(v)),
        &s,
        |e| node(e).position = v,
    );
    case("Nudge", &n, DocMutation::Node(NodeOp::Nudge(v)), &s, |e| {
        node(e).position = Vec2::new(3.0, -12.0)
    });
    case(
        "SetMinSize",
        &n,
        DocMutation::Node(NodeOp::SetMinSize(None)),
        &s,
        |e| node(e).min_size = None,
    );
    let shape = Some(Shape::Rectangle {
        radius: Percent::new(40),
    });
    case(
        "SetShape",
        &n,
        DocMutation::Node(NodeOp::SetShape(shape)),
        &s,
        |e| node(e).style.shape = shape,
    );
    let fill = Some(ColorRef::Var(VarId(1)));
    case("SetFill", &n, DocMutation::Node(NodeOp::SetFill(fill)), &s, |e| {
        node(e).style.fill = fill
    });
    case(
        "SetFillFade",
        &n,
        DocMutation::Node(NodeOp::SetFillFade(Some(6.0))),
        &s,
        |e| node(e).style.fill_fade = Some(6.0),
    );
    case(
        "SetPalette",
        &n,
        DocMutation::Node(NodeOp::SetPalette(None)),
        &s,
        |e| node(e).palette = None,
    );
    let zoom = Some(ZoomWindow {
        min: None,
        max: Some(3.0),
    });
    case("SetZoom", &n, DocMutation::Node(NodeOp::SetZoom(zoom)), &s, |e| {
        node(e).zoom = zoom
    });
    case(
        "SetFolded",
        &n,
        DocMutation::Node(NodeOp::SetFolded(true)),
        &s,
        |e| node(e).folded = true,
    );
    case(
        "ToggleFolded",
        &n,
        DocMutation::Node(NodeOp::ToggleFolded),
        &s,
        |e| node(e).folded = true,
    );
    let mut folded = n.clone();
    node(&mut folded).folded = true;
    case(
        "ToggleFolded back",
        &folded,
        DocMutation::Node(NodeOp::ToggleFolded),
        &s,
        |e| node(e).folded = false,
    );
    case(
        "SetHidden",
        &n,
        DocMutation::Node(NodeOp::SetHidden(true)),
        &s,
        |e| node(e).hidden = true,
    );
    case(
        "SetOpacity",
        &n,
        DocMutation::Node(NodeOp::SetOpacity(0.25)),
        &s,
        |e| node(e).opacity = 0.25,
    );
    case(
        "SetClick",
        &n,
        DocMutation::Node(NodeOp::SetClick(None)),
        &s,
        |e| node(e).on.click = None,
    );
    case(
        "SetHover",
        &n,
        DocMutation::Node(NodeOp::SetHover(Some(VerbId(0)))),
        &s,
        |e| node(e).on.hover = Some(VerbId(0)),
    );
    case(
        "SetChannel",
        &n,
        DocMutation::Node(NodeOp::SetChannel(5)),
        &s,
        |e| node(e).channel = 5,
    );
    case(
        "Text",
        &n,
        DocMutation::Node(NodeOp::Text(TextOp::SetItalic(Some(true)))),
        &s,
        |e| node(e).style.text.italic = Some(true),
    );
}

#[test]
fn test_document_section_and_span_ops_write_their_field() {
    do_document_section_and_span_ops_write_their_field();
}

/// Every `SectionOp` and `SpanOp`; the span's text edits count grapheme
/// clusters, so the family emoji (five code points) and the `e` with its
/// combining mark are one index each.
pub fn do_document_section_and_span_ops_write_their_field() {
    let s = outside();
    let sec = rich_section();
    let v = Vec2::new(2.0, 2.0);
    case(
        "SetOffset",
        &sec,
        DocMutation::Section(SectionOp::SetOffset(v)),
        &s,
        |e| section(e).offset = v,
    );
    case(
        "Nudge",
        &sec,
        DocMutation::Section(SectionOp::Nudge(v)),
        &s,
        |e| section(e).offset = Vec2::new(6.0, 7.0),
    );
    case(
        "SetMinSize",
        &sec,
        DocMutation::Section(SectionOp::SetMinSize(Some(v))),
        &s,
        |e| section(e).min_size = Some(v),
    );
    case(
        "SetAlign",
        &sec,
        DocMutation::Section(SectionOp::SetAlign(None)),
        &s,
        |e| section(e).align = None,
    );
    case(
        "SetChannel",
        &sec,
        DocMutation::Section(SectionOp::SetChannel(0)),
        &s,
        |e| section(e).channel = 0,
    );
    case(
        "Text",
        &sec,
        DocMutation::Section(SectionOp::Text(TextOp::SetWeight(None))),
        &s,
        |e| section(e).text.weight = None,
    );

    let sp = rich_span();
    case(
        "SetText",
        &sp,
        DocMutation::Span(SpanOp::SetText("new".to_string())),
        &s,
        |e| span(e).text = "new".to_string(),
    );
    let insert = |at| {
        DocMutation::Span(SpanOp::InsertText {
            at,
            text: "X".to_string(),
        })
    };
    case("InsertText after the emoji", &sp, insert(2), &s, |e| {
        span(e).text = "a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}Xe\u{0301}b".to_string()
    });
    case("InsertText past the end appends", &sp, insert(99), &s, |e| {
        span(e).text.push('X')
    });
    case(
        "DeleteText the emoji and the marked e",
        &sp,
        DocMutation::Span(SpanOp::DeleteText { start: 1, end: 3 }),
        &s,
        |e| span(e).text = "ab".to_string(),
    );
    case(
        "DeleteText clamps its end",
        &sp,
        DocMutation::Span(SpanOp::DeleteText { start: 3, end: 50 }),
        &s,
        |e| span(e).text = "a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}e\u{0301}".to_string(),
    );
    unchanged(
        "DeleteText past the end",
        &sp,
        DocMutation::Span(SpanOp::DeleteText { start: 4, end: 9 }),
        &s,
    );
    let verb = Some(LinkTarget::Verb(VerbId(0)));
    case(
        "SetLink",
        &sp,
        DocMutation::Span(SpanOp::SetLink(verb.clone())),
        &s,
        |e| span(e).link = verb,
    );
    case(
        "Text",
        &sp,
        DocMutation::Span(SpanOp::Text(TextOp::SetFont(None))),
        &s,
        |e| span(e).style.font = None,
    );
}

#[test]
fn test_document_frame_and_ring_ops_write_their_field() {
    do_document_frame_and_ring_ops_write_their_field();
}

/// Every `FrameOp` and `RingOp`. A ring's size is its text style's: one
/// op writes it.
pub fn do_document_frame_and_ring_ops_write_their_field() {
    let s = outside();
    let f = rich_frame();
    case(
        "SetVisible",
        &f,
        DocMutation::Frame(FrameOp::SetVisible(Some(false))),
        &s,
        |e| frame(e).visible = Some(false),
    );
    let start = Some(StartPoint {
        edge: Edge::Bottom,
        at: 0.5,
    });
    case(
        "SetStart",
        &f,
        DocMutation::Frame(FrameOp::SetStart(start)),
        &s,
        |e| frame(e).start = start,
    );
    let direction = Some(Direction::CounterClockwise);
    case(
        "SetDirection",
        &f,
        DocMutation::Frame(FrameOp::SetDirection(direction)),
        &s,
        |e| frame(e).direction = direction,
    );
    let fit = Some(FitMode::Fit { min: 0.5, max: 1.5 });
    case(
        "SetFit",
        &f,
        DocMutation::Frame(FrameOp::SetFit(fit.clone())),
        &s,
        |e| frame(e).fit = fit.clone(),
    );
    case(
        "SetGap",
        &f,
        DocMutation::Frame(FrameOp::SetGap(Some(3.0))),
        &s,
        |e| frame(e).gap = Some(3.0),
    );
    case(
        "SetInset",
        &f,
        DocMutation::Frame(FrameOp::SetInset(None)),
        &s,
        |e| frame(e).inset = None,
    );
    case(
        "Text",
        &f,
        DocMutation::Frame(FrameOp::Text(TextOp::SetSize(Some(FontSize::Rel(0.2))))),
        &s,
        |e| frame(e).text.size = Some(FontSize::Rel(0.2)),
    );

    let r = rich_ring();
    case("SetFit", &r, DocMutation::Ring(RingOp::SetFit(None)), &s, |e| {
        ring(e).style.fit = None
    });
    case(
        "SetChannel",
        &r,
        DocMutation::Ring(RingOp::SetChannel(0)),
        &s,
        |e| ring(e).channel = 0,
    );
    case(
        "Text size",
        &r,
        DocMutation::Ring(RingOp::Text(TextOp::SetSize(Some(FontSize::Rel(0.1))))),
        &s,
        |e| ring(e).style.text.size = Some(FontSize::Rel(0.1)),
    );
}

#[test]
fn test_document_path_ops_write_their_field() {
    do_document_path_ops_write_their_field();
}

/// Every `PathOp`, on a connector and on a link alike, and `LinkOp`.
/// A control point moved past the canvas bound is clamped to it, and one
/// the path does not have is left alone.
pub fn do_document_path_ops_write_their_field() {
    let s = outside();
    for p in [rich_connector(), rich_link()] {
        let fit = Some(FitMode::Once { filler: None });
        case(
            "SetFit",
            &p,
            DocMutation::Path(PathOp::SetFit(fit.clone())),
            &s,
            |e| path(e).flow.fit = fit.clone(),
        );
        case(
            "SetSpacing",
            &p,
            DocMutation::Path(PathOp::SetSpacing(None)),
            &s,
            |e| path(e).flow.spacing = None,
        );
        case(
            "SetSize",
            &p,
            DocMutation::Path(PathOp::SetSize(Some(20.0))),
            &s,
            |e| path(e).flow.size = Some(20.0),
        );
        let color = Some(ColorRef::Rgba(Rgba::opaque(7, 8, 9)));
        case(
            "SetColor",
            &p,
            DocMutation::Path(PathOp::SetColor(color)),
            &s,
            |e| path(e).flow.color = color,
        );
        case(
            "SetContent",
            &p,
            DocMutation::Path(PathOp::SetContent(Some("=".to_string()))),
            &s,
            |e| path(e).flow.content = Some("=".to_string()),
        );
        case(
            "SetAnchors",
            &p,
            DocMutation::Path(PathOp::SetAnchors(Anchor::Left, Anchor::Auto)),
            &s,
            |e| path(e).anchors = (Anchor::Left, Anchor::Auto),
        );
        case(
            "SetControlPoints",
            &p,
            DocMutation::Path(PathOp::SetControlPoints(Vec::new())),
            &s,
            |e| path(e).control_points.clear(),
        );
        let to = Vec2::new(9.0, 1.0e12);
        case(
            "MoveControlPoint",
            &p,
            DocMutation::Path(PathOp::MoveControlPoint { index: 1, to }),
            &s,
            |e| path(e).control_points[1] = Vec2::new(9.0, MAX_CANVAS_COORD),
        );
        unchanged(
            "MoveControlPoint past the points",
            &p,
            DocMutation::Path(PathOp::MoveControlPoint { index: 2, to }),
            &s,
        );
        case(
            "SetVisible",
            &p,
            DocMutation::Path(PathOp::SetVisible(false)),
            &s,
            |e| path(e).visible = false,
        );
        let zoom = Some(ZoomWindow {
            min: Some(1.0),
            max: Some(2.0),
        });
        case("SetZoom", &p, DocMutation::Path(PathOp::SetZoom(zoom)), &s, |e| {
            path(e).zoom = zoom
        });
    }
    case(
        "SetKind",
        &rich_link(),
        DocMutation::Link(LinkOp::SetKind(LinkKind::Portal)),
        &s,
        |e| match e {
            DocElement::Link(l) => l.kind = LinkKind::Portal,
            _ => unreachable!(),
        },
    );
}

#[test]
fn test_document_cap_label_and_portal_end_ops_write_their_field() {
    do_document_cap_label_and_portal_end_ops_write_their_field();
}

/// Every `TextOp` that sets a field, on a cap; every `LabelOp` and
/// `PortalEndOp`.
pub fn do_document_cap_label_and_portal_end_ops_write_their_field() {
    let s = outside();
    let cap = rich_cap();
    let color = Some(ColorRef::Palette(crate::document::values::PaletteField::Text));
    let ops: Vec<(TextOp, TextEdit)> = vec![
        (TextOp::SetFont(None), Box::new(|t| t.font = None)),
        (
            TextOp::SetSize(Some(FontSize::Pt(9.0))),
            Box::new(|t| t.size = Some(FontSize::Pt(9.0))),
        ),
        (TextOp::SetColor(color), Box::new(move |t| t.color = color)),
        (
            TextOp::SetWeight(Some(Weight::NORMAL)),
            Box::new(|t| t.weight = Some(Weight::NORMAL)),
        ),
        (TextOp::SetItalic(None), Box::new(|t| t.italic = None)),
    ];
    for (op, want) in ops {
        let name = format!("Cap {op:?}");
        case(&name, &cap, DocMutation::Cap(op), &s, |e| want(text(e)));
    }

    let label = rich_label();
    case(
        "SetT",
        &label,
        DocMutation::Label(LabelOp::SetT(Some(0.9))),
        &s,
        |e| label_style(e).t = Some(0.9),
    );
    case(
        "SetOffset",
        &label,
        DocMutation::Label(LabelOp::SetOffset(None)),
        &s,
        |e| label_style(e).offset = None,
    );
    case(
        "Text",
        &label,
        DocMutation::Label(LabelOp::Text(TextOp::SetItalic(Some(true)))),
        &s,
        |e| label_style(e).text.italic = Some(true),
    );

    let end = rich_portal_end();
    case(
        "SetBorderT",
        &end,
        DocMutation::PortalEnd(PortalEndOp::SetBorderT(None)),
        &s,
        |e| end_style(e).border_t = None,
    );
    case(
        "SetOffset",
        &end,
        DocMutation::PortalEnd(PortalEndOp::SetOffset(Some(1.0))),
        &s,
        |e| end_style(e).offset = Some(1.0),
    );
    case(
        "SetColor",
        &end,
        DocMutation::PortalEnd(PortalEndOp::SetColor(None)),
        &s,
        |e| end_style(e).color = None,
    );
    case(
        "SetSize",
        &end,
        DocMutation::PortalEnd(PortalEndOp::SetSize(Some(60.0))),
        &s,
        |e| end_style(e).size = Some(60.0),
    );
    case(
        "SetContent",
        &end,
        DocMutation::PortalEnd(PortalEndOp::SetContent(None)),
        &s,
        |e| end_style(e).content = None,
    );
}

#[test]
fn test_document_map_theme_palette_and_defaults_ops_write_their_field() {
    do_document_map_theme_palette_and_defaults_ops_write_their_field();
}

/// `MapOp`, `ThemeOp`, `PaletteOp` and `DefaultsOp`. A variant, value
/// or group the element does not have is left alone.
pub fn do_document_map_theme_palette_and_defaults_ops_write_their_field() {
    let s = outside();
    let map = DocElement::Map(MapData {
        name: "old".to_string(),
    });
    case(
        "SetName",
        &map,
        DocMutation::Map(MapOp::SetName("new".to_string())),
        &s,
        |e| {
            if let DocElement::Map(m) = e {
                m.name = "new".to_string();
            }
        },
    );

    let theme = rich_theme();
    case(
        "SetActive",
        &theme,
        DocMutation::Theme(ThemeOp::SetActive(1)),
        &s,
        |e| {
            let DocElement::Theme(t) = e else { unreachable!() };
            t.active = 1;
        },
    );
    let value = Rgba::opaque(200, 100, 50);
    case(
        "SetValue",
        &theme,
        DocMutation::Theme(ThemeOp::SetValue {
            variant: 1,
            var: VarId(0),
            value,
        }),
        &s,
        |e| {
            let DocElement::Theme(t) = e else { unreachable!() };
            t.variants[1].values[0] = value;
        },
    );
    unchanged(
        "SetActive past the variants",
        &theme,
        DocMutation::Theme(ThemeOp::SetActive(2)),
        &s,
    );
    unchanged(
        "SetValue past the variables",
        &theme,
        DocMutation::Theme(ThemeOp::SetValue {
            variant: 0,
            var: VarId(2),
            value,
        }),
        &s,
    );

    let palette = rich_palette();
    let group = palette_group(42);
    case(
        "SetGroup",
        &palette,
        DocMutation::Palette(PaletteOp::SetGroup { index: 1, group }),
        &s,
        |e| {
            let DocElement::Palette(p) = e else { unreachable!() };
            p.groups[1] = group;
        },
    );
    unchanged(
        "SetGroup past the groups",
        &palette,
        DocMutation::Palette(PaletteOp::SetGroup { index: 2, group }),
        &s,
    );

    let defaults = DocElement::Defaults(Box::default());
    let mut replacement = DefaultsData::default();
    replacement.connector.size = Some(30.0);
    replacement.label.text.color = Some(ColorRef::Rgba(Rgba::opaque(1, 1, 1)));
    let boxed = Box::new(replacement.clone());
    case(
        "Replace",
        &defaults,
        DocMutation::Defaults(DefaultsOp::Replace(boxed)),
        &s,
        |e| *e = DocElement::Defaults(Box::new(replacement)),
    );
}

#[test]
fn test_document_relative_size_ops_clamp_and_take_the_place_s_unit() {
    do_document_relative_size_ops_clamp_and_take_the_place_s_unit();
}

/// `GrowSize` and `ScaleSize` act on the element's own size when it
/// sets one, keep its unit — points outside a frame, relative inside —
/// and clamp the result to that unit's bound; a node nudged past the
/// canvas is pinned to its edge.
pub fn do_document_relative_size_ops_clamp_and_take_the_place_s_unit() {
    let s = outside();
    let sec = rich_section();
    case(
        "GrowSize",
        &sec,
        DocMutation::Section(SectionOp::Text(TextOp::GrowSize(2.0))),
        &s,
        |e| text(e).size = Some(FontSize::Pt(18.0)),
    );
    case(
        "ScaleSize",
        &sec,
        DocMutation::Section(SectionOp::Text(TextOp::ScaleSize(1.5))),
        &s,
        |e| text(e).size = Some(FontSize::Pt(24.0)),
    );
    case(
        "GrowSize past the maximum",
        &sec,
        DocMutation::Section(SectionOp::Text(TextOp::GrowSize(1.0e6))),
        &s,
        |e| text(e).size = Some(FontSize::Pt(MAX_FONT_SIZE_PT)),
    );
    case(
        "GrowSize below the minimum",
        &sec,
        DocMutation::Section(SectionOp::Text(TextOp::GrowSize(-100.0))),
        &s,
        |e| text(e).size = Some(FontSize::Pt(MIN_FONT_SIZE_PT)),
    );

    // A frame: its own place is in a frame, whatever its parent handed.
    let f = rich_frame();
    assert!(
        !s.in_frame,
        "the state handed to a frame is a node's, outside any frame"
    );
    case(
        "ScaleSize on a frame",
        &f,
        DocMutation::Frame(FrameOp::Text(TextOp::ScaleSize(2.0))),
        &s,
        |e| text(e).size = Some(FontSize::Rel(0.1)),
    );
    case(
        "ScaleSize on a frame past 1",
        &f,
        DocMutation::Frame(FrameOp::Text(TextOp::ScaleSize(100.0))),
        &s,
        |e| text(e).size = Some(FontSize::Rel(1.0)),
    );
    case(
        "GrowSize on a frame below 0",
        &f,
        DocMutation::Frame(FrameOp::Text(TextOp::GrowSize(-1.0))),
        &s,
        |e| text(e).size = Some(FontSize::Rel(MIN_REL_SIZE)),
    );

    let n = rich_node();
    case(
        "Nudge past the canvas",
        &n,
        DocMutation::Node(NodeOp::Nudge(Vec2::new(-2.0e9, 3.0e9))),
        &s,
        |e| node(e).position = Vec2::new(-MAX_CANVAS_COORD, MAX_CANVAS_COORD),
    );
}
