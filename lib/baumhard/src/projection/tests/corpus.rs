// SPDX-License-Identifier: MPL-2.0

//! The oracle's corpus (#241, Tests): seeded documents of fifty nodes or
//! more using everything the projection draws, and seeded edits drawing
//! on every op of the mutation vocabulary and every composite builder.
//! Built from [`Choices`], so a sweep's case replays from its seed.

use glam::Vec2;

use crate::arena::NodeId;
use crate::document::element::{DocElement, DocKind, PaletteGroup, RingData, SpanData};
use crate::document::ids::Document;
use crate::document::mutation::compose;
use crate::document::mutation::ops::{
    DefaultsOp, DocMutation, DocMutatorTree, FrameOp, LabelOp, LayoutOp, MapOp, NodeOp, PaletteOp, PathOp,
    PortalEndOp, RingOp, SectionOp, SpanOp, TextOp, ThemeOp,
};
use crate::document::mutation::step::{DocStep, DocWalk};
use crate::document::values::{
    Align, Anchor, ColorRef, Direction, Edge, FitMode, FontSize, LineSpan, LinkKind, LinkTarget,
    PaletteField, PaletteId, Percent, PlainColor, Rgba, Shape, Sides, StartPoint, VarId, Weight, ZoomWindow,
};
use crate::document::writer::EditRequest;
use crate::engine::anchor::Anchors;
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tree::Placement;
use crate::font::fonts::AppFont;
use crate::projection::tests::maps::load_map;
use crate::util::tests::choices::Choices;
use serde_json::{json, Value};

/// The words texts are made of: short, so frames lay quickly, with a
/// mark and a wide character among them.
const WORDS: &[&str] = &[
    "the", "earth", "was", "without", "form", "and", "void", "light", "café", "東京", "—", "x",
];

/// Some words, one to `max`.
fn words(c: &mut Choices, max: u32) -> String {
    let n = 1 + c.below(max);
    let mut out = String::new();
    for i in 0..n {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(c.pick(WORDS));
    }
    out
}

/// A color: a literal, a theme variable or a palette field.
fn color(c: &mut Choices) -> ColorRef {
    match c.below(4) {
        0 => ColorRef::Rgba(Rgba::opaque(c.below(256) as u8, c.below(256) as u8, 200)),
        1 => ColorRef::Var(VarId(c.below(3) as u16)),
        2 => ColorRef::Palette(*c.pick(&[PaletteField::Frame, PaletteField::Text, PaletteField::Title])),
        _ => ColorRef::Rgba(Rgba::opaque(40, 200, 90)),
    }
}

/// A fit mode for a frame: mostly the cheap ones.
fn fit(c: &mut Choices) -> FitMode {
    match c.below(5) {
        0 => FitMode::Repeat,
        1 => FitMode::Once { filler: None },
        2 => FitMode::Spread,
        3 => FitMode::Once {
            filler: Some("·".into()),
        },
        _ => FitMode::Fit { min: 0.5, max: 2.0 },
    }
}

/// A color's spelling: a literal, a theme variable or a palette field.
fn color_text(c: &mut Choices) -> Value {
    match c.below(4) {
        0 => json!(format!("#{:02x}{:02x}c8", c.below(256), c.below(256))),
        1 => json!(*c.pick(&["var(accent)", "var(fg)", "var(dim)"])),
        2 => json!(*c.pick(&["palette(frame)", "palette(text)", "palette(title)"])),
        _ => json!("#28c85a"),
    }
}

/// Text style keys, some set, merged into `out`: a size in points.
fn text_keys(c: &mut Choices, out: &mut Value) {
    if c.one_in(5) {
        out["font"] = json!("Novem");
    }
    if c.one_in(3) {
        out["size"] = json!(10.0 + c.below(20) as f32);
    }
    if c.one_in(2) {
        out["color"] = color_text(c);
    }
    if c.one_in(4) {
        out["weight"] = json!("bold");
    }
    if c.one_in(5) {
        out["italic"] = json!(true);
    }
}

/// A fit mode's spelling, for a frame: mostly the cheap ones.
fn fit_text(c: &mut Choices) -> Value {
    match c.below(5) {
        0 => json!("repeat"),
        1 => json!("once"),
        2 => json!("spread"),
        3 => json!({"once": {"filler": "·"}}),
        _ => json!("fit"),
    }
}

/// A palette of three groups, one color a theme variable.
fn palette_text(n: u8) -> Value {
    let group = |k: u8| {
        let mut g = json!({"frame": format!("#{n:02x}{k:02x}0a")});
        g["text"] = if k == 1 {
            json!("var(fg)")
        } else {
            json!(format!("#{n:02x}{k:02x}14"))
        };
        if k == 0 {
            g["title"] = json!(format!("#{n:02x}{k:02x}1e"));
        }
        g
    };
    json!([group(0), group(1), group(2)])
}

/// The record of generated node `i`, `id` `i + 1`, under `parent`.
fn node_record(c: &mut Choices, i: usize, parent: Option<usize>) -> Value {
    let mut n = json!({"id": i + 1, "position": [(i % 8) as f32 * 220.0, (i / 8) as f32 * 160.0]});
    if let Some(p) = parent {
        n["parent"] = json!(p + 1);
    }
    n["shape"] = if c.one_in(4) {
        json!("ellipse")
    } else {
        json!({"rectangle": {"radius": c.below(5) * 25}})
    };
    if c.one_in(7) {
        n["palette"] = json!(*c.pick(&["p10", "p20"]));
    }
    if c.one_in(15) {
        n["hidden"] = json!(true);
    }
    if c.one_in(12) {
        n["folded"] = json!(true);
    }
    if c.one_in(6) {
        n["opacity"] = json!(0.5);
    }
    if c.one_in(3) {
        n["min_size"] = json!([80.0 + c.below(80) as f32, 40.0 + c.below(40) as f32]);
    }
    if c.one_in(4) {
        text_keys(c, &mut n);
    }
    let mut section_list = Vec::new();
    for channel in 0..1 + c.below(2) {
        let mut s = json!({});
        if channel > 0 {
            s["channel"] = json!(channel);
        }
        if c.one_in(4) {
            s["offset"] = json!([c.below(20) as f32, 10.0 + c.below(30) as f32]);
            s["align"] = json!("center");
        }
        let mut spans = Vec::new();
        for _ in 0..1 + c.below(3) {
            let mut span = json!({"text": words(c, 3)});
            text_keys(c, &mut span);
            spans.push(span);
        }
        s["spans"] = json!(spans);
        section_list.push(s);
    }
    n["sections"] = json!(section_list);
    if c.one_in(3) {
        let mut frame = json!({"fit": fit_text(c)});
        if c.one_in(3) {
            frame["gap"] = json!(4.0);
        }
        let mut rings = Vec::new();
        for channel in 0..1 + c.below(2) {
            let mut ring = json!({"size": 0.05 + c.below(10) as f32 * 0.01});
            if channel > 0 {
                ring["channel"] = json!(channel);
            }
            if c.one_in(3) {
                ring["fit"] = fit_text(c);
            }
            let text = if c.one_in(2) {
                "·".to_string()
            } else {
                words(c, 4)
            };
            ring["spans"] = json!([{"text": text}]);
            rings.push(ring);
        }
        frame["rings"] = json!(rings);
        n["frame"] = frame;
    }
    if parent.is_some() {
        let mut connector = json!({});
        if c.one_in(3) {
            connector["spans"] = json!([{"text": "─"}]);
        }
        if c.one_in(4) {
            connector["cap_end"] = json!({"spans": [{"text": "▶"}]});
        }
        if c.one_in(5) {
            connector["label"] =
                json!({"t": 0.25 + c.below(3) as f32 * 0.25, "spans": [{"text": words(c, 2)}]});
        }
        if c.one_in(4) {
            connector["control_points"] = json!([[c.below(100) as f32 - 50.0, c.below(100) as f32 - 50.0]]);
        }
        n["connector"] = connector;
    }
    n
}

/// A generated map of fifty nodes or more: a random forest of nodes with
/// sections of spans, frames of relative rings, connectors with caps and
/// labels, line and portal links, two palettes bound here and there,
/// theme variables, hidden, folded and faded nodes. Written as a
/// `.mandala.json` — its nodes in the depth-first pre-order the flat list
/// takes (D30) — and loaded.
pub fn generated(c: &mut Choices) -> Document {
    let count = 50 + c.below(15) as usize;
    let parents: Vec<Option<usize>> = (0..count)
        .map(|i| (i > 0 && !c.one_in(8)).then(|| c.index(i)))
        .collect();
    let mut records: Vec<Value> = (0..count).map(|i| node_record(c, i, parents[i])).collect();
    for (i, record) in records.iter_mut().enumerate() {
        if !c.one_in(5) {
            continue;
        }
        let to = c.index(count);
        if to == i {
            continue;
        }
        let link = if c.one_in(3) {
            let mut end = json!({});
            if c.one_in(2) {
                end["label"] = json!({"spans": [{"text": "to"}]});
            }
            json!({"to": to + 1, "kind": "portal", "ends": [end, {}]})
        } else {
            let mut link = json!({"to": to + 1, "kind": "line"});
            if c.one_in(2) {
                link["spans"] = json!([{"text": "~"}]);
            }
            if c.one_in(3) {
                link["cap_start"] = json!({"spans": [{"text": "◆"}]});
            }
            if c.one_in(3) {
                link["label"] = json!({"spans": [{"text": "see"}]});
            }
            link
        };
        match record.get_mut("links").and_then(Value::as_array_mut) {
            Some(links) => links.push(link),
            None => record["links"] = json!([link]),
        }
    }
    // The flat list's order: depth-first pre-order, children in order.
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut stack: Vec<usize> = Vec::new();
    for (i, parent) in parents.iter().enumerate().rev() {
        match parent {
            Some(p) => children[*p].insert(0, i),
            None => stack.push(i),
        }
    }
    let mut nodes = Vec::with_capacity(count);
    while let Some(i) = stack.pop() {
        nodes.push(records[i].take());
        stack.extend(children[i].iter().rev());
    }
    let mut map = json!({
        "theme": {
            "active": "dark",
            "variants": {
                "dark": {"accent": "#d4a24c", "dim": "#5a5a5a", "fg": "#e0e0e0"},
                "light": {"accent": "#8a5a00", "dim": "#a0a0a0", "fg": "#202024"}
            }
        },
        "palettes": {"p10": palette_text(10), "p20": palette_text(20)},
        "nodes": nodes
    });
    if c.one_in(2) {
        map["defaults"] = json!({"connector": {"content": "─"}});
    }
    load_map(map)
}

/// The elements of `doc` of `kind`, in tree order. O(the document): a
/// test's choice of target, not projection work.
pub fn of_kind(doc: &Document, kind: DocKind) -> Vec<NodeId> {
    doc.tree()
        .descendants()
        .filter(|&id| doc.tree().arena()[id].get().kind() == kind)
        .collect()
}

/// A single-op request: `mutation` at `anchors`, a mutator of `kind`.
fn single(kind: DocKind, mutation: DocMutation, anchors: Vec<NodeId>) -> EditRequest {
    let mutator = DocMutatorTree::new_with(Mutator::new(mutation, BranchKey::new(kind, 0)));
    EditRequest::new(
        vec![DocStep::fixed(DocWalk::new(mutator, Anchors::Nodes(anchors)))],
        "generated",
    )
}

/// A request of the one structural `op` at `target`, a mutator of
/// `kind`, named `name`.
fn structure(op: StructOp<DocElement>, kind: DocKind, target: NodeId, name: &'static str) -> EditRequest {
    let mutator = DocMutatorTree::new_with(Mutator::new_structure(op, BranchKey::new(kind, 0)));
    EditRequest::new(
        vec![DocStep::fixed(DocWalk::new(
            mutator,
            Anchors::Nodes(vec![target]),
        ))],
        name,
    )
}

/// One request of `mutations` in turn, each a step of its own at
/// `target`, a mutator of `kind`, named `name`.
fn many(kind: DocKind, mutations: Vec<DocMutation>, target: NodeId, name: &'static str) -> EditRequest {
    let steps = mutations
        .into_iter()
        .flat_map(|m| single(kind, m, vec![target]).steps)
        .collect();
    EditRequest::new(steps, name)
}

/// A text op, its sizes in the unit of `rel`.
fn text_op(c: &mut Choices, rel: bool) -> (&'static str, TextOp) {
    match c.below(7) {
        0 => (
            "Text::SetFont",
            TextOp::SetFont(c.one_in(2).then_some(AppFont::Novem)),
        ),
        1 => (
            "Text::SetSize",
            TextOp::SetSize(Some(if rel {
                FontSize::Rel(0.04 + c.below(8) as f32 * 0.01)
            } else {
                FontSize::Pt(8.0 + c.below(30) as f32)
            })),
        ),
        2 => ("Text::GrowSize", TextOp::GrowSize(if rel { 0.01 } else { 2.0 })),
        3 => (
            "Text::ScaleSize",
            TextOp::ScaleSize(if c.one_in(2) { 1.5 } else { 0.5 }),
        ),
        4 => ("Text::SetColor", TextOp::SetColor(Some(color(c)))),
        5 => ("Text::SetWeight", TextOp::SetWeight(Some(Weight::BOLD))),
        _ => ("Text::SetItalic", TextOp::SetItalic(Some(c.one_in(2)))),
    }
}

/// One random element of `kind`, if `doc` has one.
fn pick(c: &mut Choices, doc: &Document, kind: DocKind) -> Option<NodeId> {
    let all = of_kind(doc, kind);
    (!all.is_empty()).then(|| all[c.index(all.len())])
}

/// A generated edit of `doc`: its name — the op, or the composite
/// builder — and its request; `None` when the document has nothing the
/// edit could target. Every op of the vocabulary and every composite is
/// drawn ([`EDITS`] names them all).
pub fn edit(c: &mut Choices, doc: &Document) -> Option<(&'static str, EditRequest)> {
    let choice = c.below(EDITS.len() as u32) as usize;
    edit_named(c, doc, EDITS[choice])
}

/// The edit [`EDITS`] calls `name`, its targets and values drawn from
/// `c`: what a sweep that walks the vocabulary in turn asks for. `None`
/// when the document has nothing it could target.
pub fn edit_named(
    c: &mut Choices,
    doc: &Document,
    name: &'static str,
) -> Option<(&'static str, EditRequest)> {
    use DocKind as K;
    let req = match name {
        "Map::SetName" => single(
            K::Map,
            DocMutation::Map(MapOp::SetName("renamed".into())),
            vec![doc.root()],
        ),
        "Map::SetBackground" => single(
            K::Map,
            DocMutation::Map(MapOp::SetBackground(PlainColor::Var(VarId(2)))),
            vec![doc.root()],
        ),
        "Theme::SetActive" => single(
            K::Theme,
            DocMutation::Theme(ThemeOp::SetActive(c.below(2) as u16)),
            vec![pick(c, doc, K::Theme)?],
        ),
        "Theme::SetValue" => single(
            K::Theme,
            DocMutation::Theme(ThemeOp::SetValue {
                variant: 0,
                var: VarId(c.below(3) as u16),
                value: Rgba::opaque(c.below(256) as u8, 30, 30),
            }),
            vec![pick(c, doc, K::Theme)?],
        ),
        "Palette::SetGroup" => single(
            K::Palette,
            DocMutation::Palette(PaletteOp::SetGroup {
                index: c.below(3) as usize,
                group: PaletteGroup {
                    frame: PlainColor::Rgba(Rgba::opaque(1, 2, 3)),
                    text: PlainColor::Var(VarId(0)),
                    title: None,
                },
            }),
            vec![pick(c, doc, K::Palette)?],
        ),
        "Defaults::Replace" => {
            let mut defaults = doc.defaults().clone();
            defaults.label.t = Some(0.75);
            defaults.section.color = Some(ColorRef::Var(VarId(0)));
            defaults.frame.fit = Some(FitMode::Repeat);
            single(
                K::Defaults,
                DocMutation::Defaults(DefaultsOp::Replace(Box::new(defaults))),
                vec![pick(c, doc, K::Defaults)?],
            )
        }
        n if n.starts_with("Node::") && n != "Node::SetPalette near the root" => {
            let node = pick(c, doc, K::Node)?;
            let op = match n {
                "Node::SetPosition" => {
                    NodeOp::SetPosition(Vec2::new(c.below(1500) as f32, c.below(1200) as f32))
                }
                "Node::Nudge" => NodeOp::Nudge(Vec2::new(c.below(60) as f32 - 30.0, 7.0)),
                "Node::SetMinSize" => NodeOp::SetMinSize(
                    c.one_in(3)
                        .then(|| Vec2::new(60.0 + c.below(200) as f32, 50.0 + c.below(90) as f32)),
                ),
                "Node::SetShape" => NodeOp::SetShape(Some(if c.one_in(2) {
                    Shape::Ellipse
                } else {
                    Shape::Rectangle {
                        radius: Percent::new(50),
                    }
                })),
                "Node::SetPalette" => NodeOp::SetPalette(c.one_in(3).then(|| PaletteId(c.below(2) as u16))),
                "Node::SetZoom" => NodeOp::SetZoom(c.one_in(2).then_some(ZoomWindow {
                    min: Some(0.5),
                    max: None,
                })),
                "Node::SetFolded" => NodeOp::SetFolded(c.one_in(2)),
                "Node::ToggleFolded" => NodeOp::ToggleFolded,
                "Node::SetHidden" => NodeOp::SetHidden(c.one_in(2)),
                "Node::SetOpacity" => NodeOp::SetOpacity(*c.pick(&[0.0, 0.25, 0.5, 1.0])),
                "Node::SetClick" => NodeOp::SetClick(None),
                "Node::SetHover" => NodeOp::SetHover(None),
                "Node::SetChannel" => NodeOp::SetChannel(c.below(4) as usize),
                _ => NodeOp::Text(text_op(c, false).1),
            };
            single(K::Node, DocMutation::Node(op), vec![node])
        }
        n if n.starts_with("Section::") => {
            let section = pick(c, doc, K::Section)?;
            let op = match n {
                "Section::SetOffset" => {
                    SectionOp::SetOffset(Vec2::new(c.below(30) as f32, c.below(40) as f32))
                }
                "Section::Nudge" => SectionOp::Nudge(Vec2::new(3.0, 4.0)),
                "Section::SetMinSize" => SectionOp::SetMinSize(c.one_in(2).then_some(Vec2::new(150.0, 60.0))),
                "Section::SetAlign" => {
                    SectionOp::SetAlign(Some(*c.pick(&[Align::Left, Align::Center, Align::Right])))
                }
                "Section::SetChannel" => SectionOp::SetChannel(c.below(3) as usize),
                _ => SectionOp::Text(text_op(c, false).1),
            };
            single(K::Section, DocMutation::Section(op), vec![section])
        }
        n if n.starts_with("Span::") => {
            let span = pick(c, doc, K::Span)?;
            // A span inside a frame takes no size of its own.
            let in_frame = doc
                .parent(span)
                .and_then(|p| doc.get(p))
                .is_some_and(|e| e.kind() == K::Ring);
            let op = match n {
                "Span::SetText" => SpanOp::SetText(words(c, 3)),
                "Span::InsertText" => SpanOp::InsertText {
                    at: c.below(4) as usize,
                    text: "ab".into(),
                },
                "Span::DeleteText" => SpanOp::DeleteText { start: 0, end: 1 },
                "Span::SetLink" => SpanOp::SetLink(Some(LinkTarget::Url("https://example.com".into()))),
                _ => {
                    let (_, op) = text_op(c, false);
                    if in_frame
                        && matches!(
                            op,
                            TextOp::SetSize(_) | TextOp::GrowSize(_) | TextOp::ScaleSize(_)
                        )
                    {
                        SpanOp::Text(TextOp::SetItalic(Some(true)))
                    } else {
                        SpanOp::Text(op)
                    }
                }
            };
            single(K::Span, DocMutation::Span(op), vec![span])
        }
        n if n.starts_with("Frame::") => {
            let frame = pick(c, doc, K::Frame)?;
            let op = match n {
                "Frame::SetVisible" => FrameOp::SetVisible(Some(c.one_in(2))),
                "Frame::SetStart" => FrameOp::SetStart(Some(StartPoint {
                    edge: *c.pick(&[Edge::Top, Edge::Right, Edge::Bottom, Edge::Left]),
                    at: 0.5,
                })),
                "Frame::SetDirection" => FrameOp::SetDirection(Some(Direction::CounterClockwise)),
                "Frame::SetFit" => FrameOp::SetFit(Some(fit(c))),
                "Frame::SetGap" => FrameOp::SetGap(Some(c.below(10) as f32)),
                "Frame::SetInset" => FrameOp::SetInset(Some(c.below(10) as f32)),
                "Frame::Layout" => FrameOp::Layout(layout_op(c)),
                _ => FrameOp::Text(match text_op(c, true).1 {
                    TextOp::GrowSize(_) | TextOp::ScaleSize(_) => TextOp::SetItalic(Some(true)),
                    op => op,
                }),
            };
            single(K::Frame, DocMutation::Frame(op), vec![frame])
        }
        n if n.starts_with("Ring::") => {
            let ring = pick(c, doc, K::Ring)?;
            let op = match n {
                "Ring::SetFit" => RingOp::SetFit(c.one_in(2).then(|| fit(c))),
                "Ring::Layout" => RingOp::Layout(layout_op(c)),
                "Ring::SetChannel" => RingOp::SetChannel(c.below(3) as usize),
                _ => RingOp::Text(text_op(c, true).1),
            };
            single(K::Ring, DocMutation::Ring(op), vec![ring])
        }
        n if n.starts_with("Path::") => {
            let kind = if c.one_in(2) { K::Connector } else { K::Link };
            let path = pick(c, doc, kind)?;
            let op = match n {
                "Path::SetFit" => PathOp::SetFit(Some(
                    c.pick(&[FitMode::Repeat, FitMode::Spread, FitMode::Once { filler: None }])
                        .clone(),
                )),
                "Path::SetSpacing" => PathOp::SetSpacing(Some(c.below(5) as f32)),
                "Path::SetSize" => PathOp::SetSize(Some(8.0 + c.below(20) as f32)),
                "Path::SetColor" => PathOp::SetColor(Some(color(c))),
                "Path::SetContent" => PathOp::SetContent(Some("=".into())),
                "Path::SetAnchors" => PathOp::SetAnchors(
                    *c.pick(&[Anchor::Auto, Anchor::Top, Anchor::Right]),
                    *c.pick(&[Anchor::Auto, Anchor::Bottom, Anchor::Left]),
                ),
                "Path::SetControlPoints" => PathOp::SetControlPoints(match c.below(3) {
                    0 => Vec::new(),
                    1 => vec![Vec2::new(40.0, -60.0)],
                    _ => vec![Vec2::new(40.0, -60.0), Vec2::new(-30.0, 50.0)],
                }),
                "Path::MoveControlPoint" => PathOp::MoveControlPoint {
                    index: 0,
                    to: Vec2::new(c.below(80) as f32, 20.0),
                },
                "Path::SetVisible" => PathOp::SetVisible(c.one_in(2)),
                _ => PathOp::SetZoom(Some(ZoomWindow {
                    min: None,
                    max: Some(4.0),
                })),
            };
            single(kind, DocMutation::Path(op), vec![path])
        }
        "Cap::Text" => {
            let kind = if c.one_in(2) { K::CapStart } else { K::CapEnd };
            let cap = pick(c, doc, kind)?;
            single(kind, DocMutation::Cap(text_op(c, false).1), vec![cap])
        }
        n if n.starts_with("Label::") => {
            let label = pick(c, doc, K::Label)?;
            let op = match n {
                "Label::SetT" => LabelOp::SetT(Some(c.below(5) as f32 * 0.25)),
                "Label::SetOffset" => LabelOp::SetOffset(Some(c.below(20) as f32 - 10.0)),
                _ => LabelOp::Text(text_op(c, false).1),
            };
            single(K::Label, DocMutation::Label(op), vec![label])
        }
        n if n.starts_with("PortalEnd::") => {
            let end = pick(c, doc, K::PortalEnd)?;
            let op = match n {
                "PortalEnd::SetBorderT" => {
                    PortalEndOp::SetBorderT(c.one_in(3).then(|| c.below(4) as f32 * 0.25))
                }
                "PortalEnd::SetOffset" => PortalEndOp::SetOffset(Some(c.below(10) as f32)),
                "PortalEnd::SetColor" => PortalEndOp::SetColor(Some(color(c))),
                "PortalEnd::SetSize" => PortalEndOp::SetSize(Some(10.0 + c.below(10) as f32)),
                _ => PortalEndOp::SetContent(Some("◉".into())),
            };
            single(K::PortalEnd, DocMutation::PortalEnd(op), vec![end])
        }
        "Node::SetPalette near the root" => {
            // A root node, or a child of one: a rebinding whose carried
            // binding changes for a whole subtree (ruling B7.2).
            let roots: Vec<NodeId> = doc
                .children(doc.root())
                .filter(|&c| doc.get(c).is_some_and(|e| e.kind() == K::Node))
                .collect();
            if roots.is_empty() {
                return None;
            }
            let mut node = roots[c.index(roots.len())];
            if c.one_in(2) {
                let children: Vec<NodeId> = doc
                    .children(node)
                    .filter(|&n| doc.get(n).is_some_and(|e| e.kind() == K::Node))
                    .collect();
                if !children.is_empty() {
                    node = children[c.index(children.len())];
                }
            }
            let palette = (!c.one_in(4)).then(|| PaletteId(c.below(2) as u16));
            single(
                K::Node,
                DocMutation::Node(NodeOp::SetPalette(palette)),
                vec![node],
            )
        }
        "compose::create_child" => {
            let parent = pick(c, doc, K::Node)?;
            EditRequest::new(
                compose::create_child(doc, parent, Vec2::new(30.0, 40.0)).ok()?,
                name,
            )
        }
        "compose::create_root" => {
            EditRequest::new(compose::create_root(doc, Vec2::new(900.0, 60.0)).ok()?, name)
        }
        "compose::delete" => {
            let node = pick(c, doc, K::Node)?;
            EditRequest::new(compose::delete(doc, &[node]).ok()?, name)
        }
        "compose::remove_links" => {
            EditRequest::new(vec![compose::remove_links(vec![pick(c, doc, K::Link)?])], name)
        }
        "compose::reparent" => {
            let node = pick(c, doc, K::Node)?;
            let to = pick(c, doc, K::Node)?;
            EditRequest::new(compose::reparent(doc, &[node], to).ok()?, name)
        }
        "compose::set_link_kind" => {
            let link = pick(c, doc, K::Link)?;
            let kind = match doc.get(link) {
                Some(DocElement::Link(l)) if l.kind == LinkKind::Line => LinkKind::Portal,
                _ => LinkKind::Line,
            };
            EditRequest::new(compose::set_link_kind(doc, link, kind).ok()?, name)
        }
        "compose::style_range" => {
            let section = pick(c, doc, K::Section)?;
            EditRequest::new(
                compose::style_range(
                    doc,
                    section,
                    0,
                    3,
                    TextOp::SetColor(Some(ColorRef::Var(VarId(0)))),
                )
                .ok()?,
                name,
            )
        }
        "compose::insert_text" => {
            let section = pick(c, doc, K::Section)?;
            EditRequest::new(compose::insert_text(doc, section, 2, " new".into()).ok()?, name)
        }
        "compose::delete_text" => {
            let section = pick(c, doc, K::Section)?;
            EditRequest::new(compose::delete_text(doc, section, 1, 4).ok()?, name)
        }
        "compose::split_section" => {
            let section = pick(c, doc, K::Section)?;
            EditRequest::new(compose::split_section(doc, section, 2).ok()?, name)
        }
        "compose::add_section" => {
            let node = pick(c, doc, K::Node)?;
            EditRequest::new(compose::add_section(doc, node, None).ok()?, name)
        }
        // A ring inserted just inside another of its frame, on that
        // ring's channel: the ring it lands inside of clears it, and
        // every ring outside moves out.
        "structure::insert a ring" => {
            let outer = pick(c, doc, K::Ring)?;
            let frame = doc.parent(outer)?;
            let Some(DocElement::Ring(r)) = doc.get(outer) else {
                return None;
            };
            let ring = Subtree {
                root: DocElement::Ring(RingData {
                    channel: r.channel,
                    style: Default::default(),
                }),
                children: vec![Subtree::leaf(DocElement::Span(SpanData {
                    text: words(c, 2),
                    style: Default::default(),
                    link: None,
                }))],
            };
            structure(
                StructOp::Insert {
                    subtree: ring,
                    at: Placement::Before(outer),
                },
                K::Frame,
                frame,
                name,
            )
        }
        // A ring removed from its frame: the rings outside it move in.
        "structure::remove a ring" => structure(StructOp::Remove, K::Ring, pick(c, doc, K::Ring)?, name),
        // Several steps on one element in one request, so one record
        // carries them all: a structural step and a field step, or a
        // geometry field and a field its resolution reads (the review of
        // #317).
        "multi::reparent and restyle" => {
            let node = pick(c, doc, K::Node)?;
            let to = pick(c, doc, K::Node)?;
            let size = TextOp::SetSize(Some(FontSize::Pt(8.0 + c.below(30) as f32)));
            let mut steps = compose::reparent(doc, &[node], to).ok()?;
            steps.extend(single(K::Node, DocMutation::Node(NodeOp::Text(size)), vec![node]).steps);
            EditRequest::new(steps, name)
        }
        "multi::move and resize" => {
            let node = pick(c, doc, K::Node)?;
            let to = Vec2::new(c.below(1500) as f32, c.below(1200) as f32);
            let size = Vec2::new(80.0 + c.below(300) as f32, 40.0 + c.below(200) as f32);
            many(
                K::Node,
                vec![
                    DocMutation::Node(NodeOp::SetPosition(to)),
                    DocMutation::Node(NodeOp::SetMinSize(Some(size))),
                ],
                node,
                name,
            )
        }
        "multi::nudge, align and restyle a section" => {
            let section = pick(c, doc, K::Section)?;
            let align = *c.pick(&[Align::Left, Align::Center, Align::Right]);
            many(
                K::Section,
                vec![
                    DocMutation::Section(SectionOp::Nudge(Vec2::new(5.0, -3.0))),
                    DocMutation::Section(SectionOp::SetAlign(Some(align))),
                    DocMutation::Section(SectionOp::Text(text_op(c, false).1)),
                ],
                section,
                name,
            )
        }
        "multi::drag and restyle a label" => {
            let label = pick(c, doc, K::Label)?;
            many(
                K::Label,
                vec![
                    DocMutation::Label(LabelOp::SetT(Some(c.below(5) as f32 * 0.25))),
                    DocMutation::Label(LabelOp::SetOffset(Some(c.below(20) as f32 - 10.0))),
                    DocMutation::Label(LabelOp::Text(text_op(c, false).1)),
                ],
                label,
                name,
            )
        }
        "multi::drag a marker and change its content" => {
            let end = pick(c, doc, K::PortalEnd)?;
            many(
                K::PortalEnd,
                vec![
                    DocMutation::PortalEnd(PortalEndOp::SetBorderT(Some(c.below(4) as f32 * 0.25))),
                    DocMutation::PortalEnd(PortalEndOp::SetContent(Some("◆".into()))),
                ],
                end,
                name,
            )
        }
        "multi::bend and show or hide a path" => {
            let kind = if c.one_in(2) { K::Connector } else { K::Link };
            let path = pick(c, doc, kind)?;
            many(
                kind,
                vec![
                    DocMutation::Path(PathOp::SetControlPoints(vec![Vec2::new(30.0, -40.0)])),
                    DocMutation::Path(PathOp::SetVisible(c.one_in(2))),
                    DocMutation::Path(PathOp::MoveControlPoint {
                        index: 0,
                        to: Vec2::new(c.below(80) as f32, 20.0),
                    }),
                ],
                path,
                name,
            )
        }
        _ => return None,
    };
    Some((name, req))
}

/// A layout op.
fn layout_op(c: &mut Choices) -> LayoutOp {
    match c.below(3) {
        0 => LayoutOp::SetSpan(LineSpan::new(c.below(3) as f32 * 0.5)),
        1 => LayoutOp::SetSides(Some(if c.one_in(2) {
            Sides::Off
        } else {
            Sides::On {
                content: "✦".into(),
                color: None,
            }
        })),
        _ => LayoutOp::SetSpacing(Some(c.below(4) as f32)),
    }
}

/// Every edit [`edit`] draws: each op of the vocabulary — a `Text` op
/// named per owner, its seven text ops drawn within it — each
/// composite builder, a ring inserted inside another and a ring removed
/// (`structure::`: no op or builder adds or takes one ring), and
/// requests of several steps on one element (`multi::`). `LinkOp::SetKind` is drawn as
/// `compose::set_link_kind`, the one shape the validator lets it take.
pub const EDITS: &[&str] = &[
    "Map::SetName",
    "Map::SetBackground",
    "Theme::SetActive",
    "Theme::SetValue",
    "Palette::SetGroup",
    "Defaults::Replace",
    "Node::SetPosition",
    "Node::Nudge",
    "Node::SetMinSize",
    "Node::SetShape",
    "Node::SetPalette",
    "Node::SetPalette near the root",
    "Node::SetZoom",
    "Node::SetFolded",
    "Node::ToggleFolded",
    "Node::SetHidden",
    "Node::SetOpacity",
    "Node::SetClick",
    "Node::SetHover",
    "Node::SetChannel",
    "Node::Text",
    "Section::SetOffset",
    "Section::Nudge",
    "Section::SetMinSize",
    "Section::SetAlign",
    "Section::SetChannel",
    "Section::Text",
    "Span::SetText",
    "Span::InsertText",
    "Span::DeleteText",
    "Span::SetLink",
    "Span::Text",
    "Frame::SetVisible",
    "Frame::SetStart",
    "Frame::SetDirection",
    "Frame::SetFit",
    "Frame::SetGap",
    "Frame::SetInset",
    "Frame::Layout",
    "Frame::Text",
    "Ring::SetFit",
    "Ring::Layout",
    "Ring::SetChannel",
    "Ring::Text",
    "Path::SetFit",
    "Path::SetSpacing",
    "Path::SetSize",
    "Path::SetColor",
    "Path::SetContent",
    "Path::SetAnchors",
    "Path::SetControlPoints",
    "Path::MoveControlPoint",
    "Path::SetVisible",
    "Path::SetZoom",
    "Cap::Text",
    "Label::SetT",
    "Label::SetOffset",
    "Label::Text",
    "PortalEnd::SetBorderT",
    "PortalEnd::SetOffset",
    "PortalEnd::SetColor",
    "PortalEnd::SetSize",
    "PortalEnd::SetContent",
    "compose::create_child",
    "compose::create_root",
    "compose::delete",
    "compose::remove_links",
    "compose::reparent",
    "compose::set_link_kind",
    "compose::style_range",
    "compose::insert_text",
    "compose::delete_text",
    "compose::split_section",
    "compose::add_section",
    "structure::insert a ring",
    "structure::remove a ring",
    "multi::reparent and restyle",
    "multi::move and resize",
    "multi::nudge, align and restyle a section",
    "multi::drag and restyle a label",
    "multi::drag a marker and change its content",
    "multi::bend and show or hide a path",
];
