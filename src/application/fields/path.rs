// SPDX-License-Identifier: MPL-2.0

//! Field editing II (V1-C6, #247): the paths between nodes — a node's
//! connector, the line to its parent, and its links — their caps and
//! labels, a link's kind, and the two ends of a portal link (design
//! §5.1, §5.6, §6.1, D8, D13, D22, D34).
//!
//! **What each edit reaches.** Nodes, connectors and links are what can
//! be selected (`document::selection`); a cap, a label and a portal end
//! are reached through the path that holds them.
//!
//! | Edit | Reaches |
//! |---|---|
//! | [`set`], [`geometry`], [`caps`] | the selected connectors and line links |
//! | [`label`], [`delete_label`] | the labels of the selected connectors and line links |
//! | [`link_kind`] | the selected links |
//! | [`portal`] | one end of each selected portal link |
//!
//! A portal link draws no path, and the path fields it keeps are never
//! saved (`LinkData::path`), so the path edits pass over one; its
//! labels are its ends', which the span editors edit (V1-C8, #249), so
//! [`label`] declines on one. A connector is always a line (D22), so
//! [`link_kind`] declines on one. An edit whose selection reaches
//! nothing it edits is refused, naming what to select.
//!
//! **Content is text** (D8): what a path, a cap or a portal end draws is
//! its spans, or its `content` when it has none. Width and line style
//! are content and size (D13): a dotted line is `content="·"`, a heavy
//! one `content="━"`. There are no presets.
//!
//! **Geometry** (D34): a path is the default curve between its two ends,
//! or passes through its control points, or is `straight`, one segment
//! from side midpoint to side midpoint. [`Geometry::Straight`] makes it
//! straight, which drops its points; [`Geometry::Curve`] makes it the
//! default curve again, its points cleared; [`Geometry::Reset`] gives
//! back every field a path inherits, and its geometry and anchors, but
//! keeps what it holds — its spans, caps and label — and whether it is
//! drawn and the zoom it draws in, which are `path visible=` and
//! `zoom`'s.

use baumhard::arena::NodeId;
use baumhard::document::element::{CapData, DocElement, DocKind, LabelData, LabelStyle, SpanData};
use baumhard::document::ids::Document;
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{DocMutation, LabelOp, PathOp, PortalEndOp};
use baumhard::document::mutation::step::DocStep;
use baumhard::document::values::{Anchor, ColorRef, FitMode, LinkKind};
use baumhard::engine::structure::Subtree;

use super::section::plain;
use super::{Groups, Refused, Steps};
use crate::application::document::selection::kind_of;
use crate::application::document::MindMapDocument;
use crate::application::view::inspect::children_of_kind;

/// The steps of a path edit, or why it does not apply: a message, or a
/// decline the status line says ([`Refused::Declined`]).
pub(crate) type Built = Result<Vec<DocStep>, Refused>;

/// What a link kind asked of a connector alone answers (D22).
pub(crate) const LINK_KIND_ON_A_CONNECTOR: &str =
    "link: a connector is always a line (D22); select a link to make it a portal";

/// What a label asked of a portal link alone answers: its labels are
/// its ends', which the span editors edit.
pub(crate) const LABEL_ON_A_PORTAL: &str =
    "label: a portal link's labels are its ends'; they are edited with the span editors of V1-C8 (#249)";

/// The kind of link `id` is, or `None` when it is not a link.
fn link_kind_of(doc: &Document, id: NodeId) -> Option<LinkKind> {
    match doc.get(id) {
        Some(DocElement::Link(link)) => Some(link.kind),
        _ => None,
    }
}

/// Whether `id` is a path an edit of its fields reaches: a connector, or
/// a line link.
fn is_path(doc: &Document, id: NodeId) -> bool {
    match kind_of(doc, id) {
        Some(DocKind::Connector) => true,
        Some(DocKind::Link) => link_kind_of(doc, id) == Some(LinkKind::Line),
        _ => false,
    }
}

/// Whether a portal link is selected.
fn portal_selected(doc: &MindMapDocument) -> bool {
    let d = doc.doc();
    doc.selection
        .iter()
        .any(|id| link_kind_of(d, id) == Some(LinkKind::Portal))
}

/// The selected connectors and line links, in selection order, or the
/// refusal `verb` answers with when there is none.
fn paths(doc: &MindMapDocument, verb: &str) -> Result<Vec<NodeId>, String> {
    let d = doc.doc();
    let paths: Vec<NodeId> = doc.selection.iter().filter(|&id| is_path(d, id)).collect();
    if paths.is_empty() {
        return Err(match portal_selected(doc) {
            true => format!("{verb}: a portal link draws no path; select a connector or a line link"),
            false => format!("{verb}: select a connector or a line link"),
        });
    }
    Ok(paths)
}

/// The steps that set `mutations` on every one of `paths`: one walk per
/// kind among them (connectors, line links).
fn on_paths(doc: &Document, paths: &[NodeId], mutations: Vec<DocMutation>) -> Steps {
    let mut groups = Groups::default();
    for &path in paths {
        groups.add(doc, path, (), || mutations.clone());
    }
    groups.steps(doc)
}

/// What `path <key>=<value> …` sets on each selected path: each field
/// given, `None` inside it to inherit.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PathFields {
    /// The text it draws when it has no spans.
    pub content: Option<Option<String>>,
    pub fit: Option<Option<FitMode>>,
    /// The extra space between its glyphs.
    pub spacing: Option<Option<f32>>,
    /// Its text size, in points.
    pub size: Option<Option<f32>>,
    pub color: Option<Option<ColorRef>>,
    /// Where it attaches at its start and at its end.
    pub anchors: Option<(Anchor, Anchor)>,
    /// Whether it is drawn.
    pub visible: Option<bool>,
}

impl PathFields {
    /// The ops it makes, in the order its fields are listed.
    fn ops(&self) -> Vec<DocMutation> {
        let mut ops = Vec::new();
        ops.extend(self.content.clone().map(PathOp::SetContent));
        ops.extend(self.fit.clone().map(PathOp::SetFit));
        ops.extend(self.spacing.map(PathOp::SetSpacing));
        ops.extend(self.size.map(PathOp::SetSize));
        ops.extend(self.color.map(PathOp::SetColor));
        ops.extend(self.anchors.map(|(from, to)| PathOp::SetAnchors(from, to)));
        ops.extend(self.visible.map(PathOp::SetVisible));
        ops.into_iter().map(DocMutation::Path).collect()
    }
}

/// The steps that set `fields` on every selected connector and line
/// link.
pub(crate) fn set(doc: &MindMapDocument, fields: &PathFields) -> Steps {
    let ops = fields.ops();
    if ops.is_empty() {
        return Err("path: give content=, fit=, spacing=, size=, color=, anchors= or visible=".to_string());
    }
    on_paths(doc.doc(), &paths(doc, "path")?, ops)
}

/// What [`geometry`] makes of a path's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Geometry {
    /// One straight segment between its ends; its control points are
    /// dropped (`PathOp::SetStraight`).
    Straight,
    /// The default curve between its ends: not straight, no control
    /// points.
    Curve,
    /// Every field it inherits inherited again, its geometry the default
    /// curve and both anchors `auto`; its spans, caps, label, `visible`
    /// and zoom kept.
    Reset,
}

impl Geometry {
    /// The ops it makes.
    fn ops(self) -> Vec<DocMutation> {
        let curve = [PathOp::SetControlPoints(Vec::new()), PathOp::SetStraight(false)];
        let ops: Vec<PathOp> = match self {
            Geometry::Straight => vec![PathOp::SetStraight(true)],
            Geometry::Curve => curve.to_vec(),
            Geometry::Reset => [
                PathOp::SetContent(None),
                PathOp::SetFit(None),
                PathOp::SetSpacing(None),
                PathOp::SetSize(None),
                PathOp::SetColor(None),
                PathOp::SetAnchors(Anchor::Auto, Anchor::Auto),
            ]
            .into_iter()
            .chain(curve)
            .collect(),
        };
        ops.into_iter().map(DocMutation::Path).collect()
    }
}

/// The steps that make `what` of every selected connector and line
/// link: straight, the default curve, or its defaults back.
pub(crate) fn geometry(doc: &MindMapDocument, what: Geometry) -> Steps {
    on_paths(doc.doc(), &paths(doc, "path")?, what.ops())
}

/// The child of `kind` of `owner`, if it has one: a cap, a label.
fn child(doc: &Document, owner: NodeId, kind: DocKind) -> Option<NodeId> {
    children_of_kind(doc, owner, kind).next()
}

/// `root` holding one span of `text`.
fn holding(root: DocElement, text: &str) -> Subtree<DocElement> {
    Subtree {
        root,
        children: plain(text)
            .into_iter()
            .map(|span| Subtree::leaf(DocElement::Span(span)))
            .collect(),
    }
}

/// The steps that give every one of `paths` the child `make` builds, or
/// the spans of `spans` on the one it has: the child of `kind` is
/// inserted where a path has none — one insert per kind of path — and
/// its spans replaced where it has one.
fn give(
    doc: &Document,
    paths: &[NodeId],
    kind: DocKind,
    spans: &[SpanData],
    make: impl Fn() -> Subtree<DocElement>,
) -> Steps {
    let mut held = Vec::new();
    let mut bare = Groups::default();
    for &path in paths {
        match child(doc, path, kind) {
            Some(existing) => held.push(existing),
            None => bare.add(doc, path, (), Vec::new),
        }
    }
    let mut steps = compose::replace_spans(doc, &held, spans).map_err(|e| e.to_string())?;
    for (_, owners) in bare.members() {
        let parent = owners.first().and_then(|&owner| kind_of(doc, owner));
        if let Some(parent) = parent {
            steps.extend(compose::insert(doc, parent, &owners, make()).map_err(|e| e.to_string())?);
        }
    }
    Ok(steps)
}

/// The steps that set one cap, of `kind`, on every one of `paths`: its
/// one span holding `text`, inserted where a path has no such cap; or,
/// for `None`, the cap removed where one has it.
fn cap(doc: &Document, paths: &[NodeId], kind: DocKind, text: Option<&str>) -> Steps {
    match text {
        Some("") => Err("path cap: give a cap's text, or `none` to remove it".to_string()),
        Some(text) => {
            let root = || match kind {
                DocKind::CapStart => DocElement::CapStart(CapData::default()),
                _ => DocElement::CapEnd(CapData::default()),
            };
            give(doc, paths, kind, &plain(text), || holding(root(), text))
        }
        None => {
            let caps: Vec<NodeId> = paths.iter().filter_map(|&path| child(doc, path, kind)).collect();
            compose::remove(doc, kind, &caps).map_err(|e| e.to_string())
        }
    }
}

/// The steps that set the start cap (`start`) and the end cap (`end`)
/// of every selected connector and line link, each given: a text makes
/// the cap one span of it, inserting the cap where there is none, and
/// `None` inside removes it.
pub(crate) fn caps(doc: &MindMapDocument, start: Option<Option<&str>>, end: Option<Option<&str>>) -> Steps {
    if start.is_none() && end.is_none() {
        return Err("path cap: give start= or end=".to_string());
    }
    let paths = paths(doc, "path cap")?;
    let d = doc.doc();
    let mut steps = Vec::new();
    for (kind, text) in [(DocKind::CapStart, start), (DocKind::CapEnd, end)] {
        if let Some(text) = text {
            steps.extend(cap(d, &paths, kind, text)?);
        }
    }
    Ok(steps)
}

/// The selected links, every other element left out, or why there is
/// none: a decline when connectors alone are selected.
fn links(doc: &MindMapDocument) -> Result<Vec<NodeId>, Refused> {
    let d = doc.doc();
    let links: Vec<NodeId> = doc
        .selection
        .iter()
        .filter(|&id| kind_of(d, id) == Some(DocKind::Link))
        .collect();
    if links.is_empty() {
        let connector = doc
            .selection
            .iter()
            .any(|id| kind_of(d, id) == Some(DocKind::Connector));
        return Err(match connector {
            true => Refused::Declined(LINK_KIND_ON_A_CONNECTOR),
            false => Refused::Because("link: select a link".to_string()),
        });
    }
    Ok(links)
}

/// The steps that switch every selected link to `kind`, with the
/// children it holds (`compose::set_link_kind`): a portal gets its two
/// ends, and a line link that becomes one loses its spans, caps and
/// label, which an undo gives back. A link already of `kind` is left as
/// it is; a connector, always a line, is passed over, and alone is
/// declined.
pub(crate) fn link_kind(doc: &MindMapDocument, kind: LinkKind) -> Built {
    let links = links(doc)?;
    compose::set_link_kind(doc.doc(), &links, kind).map_err(|e| Refused::Because(e.to_string()))
}

/// What `label <key>=<value> …` sets on each selected path's label:
/// each field given, `None` inside `t` and `offset` to inherit.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LabelFields {
    /// Its text, as one span; a path with no label is given one.
    pub text: Option<String>,
    /// Its place along the path, from 0 to 1.
    pub t: Option<Option<f32>>,
    /// Its distance from the path.
    pub offset: Option<Option<f32>>,
}

impl LabelFields {
    /// The ops it makes on a label.
    fn ops(&self) -> Vec<DocMutation> {
        let mut ops = Vec::new();
        ops.extend(self.t.map(LabelOp::SetT));
        ops.extend(self.offset.map(LabelOp::SetOffset));
        ops.into_iter().map(DocMutation::Label).collect()
    }

    /// A new label with these fields.
    fn label(&self) -> LabelData {
        LabelData {
            style: LabelStyle {
                t: self.t.flatten(),
                offset: self.offset.flatten(),
                ..LabelStyle::default()
            },
        }
    }
}

/// The selected connectors and line links, for a label edit: declined
/// when a portal link alone is selected.
fn labeled(doc: &MindMapDocument) -> Result<Vec<NodeId>, Refused> {
    paths(doc, "label").map_err(|why| match portal_selected(doc) {
        true => Refused::Declined(LABEL_ON_A_PORTAL),
        false => Refused::Because(why),
    })
}

/// The steps that set `fields` on the label of every selected connector
/// and line link. With a text, a path with no label is given one holding
/// it, with the label's other fields; without one, only the paths that
/// have a label are reached, and a selection whose paths have none is
/// refused.
pub(crate) fn label(doc: &MindMapDocument, fields: &LabelFields) -> Built {
    let paths = labeled(doc)?;
    let d = doc.doc();
    let ops = fields.ops();
    match &fields.text {
        Some(text) if text.is_empty() => Err("label: give the label's text, or `label delete` to remove it"
            .to_string()
            .into()),
        Some(text) => {
            let mut steps = give(d, &paths, DocKind::Label, &plain(text), || {
                holding(DocElement::Label(fields.label()), text)
            })?;
            let held: Vec<NodeId> = paths
                .iter()
                .filter_map(|&path| child(d, path, DocKind::Label))
                .collect();
            steps.extend(compose::set(d, DocKind::Label, ops, &held).map_err(|e| e.to_string())?);
            Ok(steps)
        }
        None if ops.is_empty() => Err("label: give text=, t=, offset= or delete".to_string().into()),
        None => {
            let held: Vec<NodeId> = paths
                .iter()
                .filter_map(|&path| child(d, path, DocKind::Label))
                .collect();
            if held.is_empty() {
                return Err("label: no selected path has a label; `label text=` gives it one"
                    .to_string()
                    .into());
            }
            compose::set(d, DocKind::Label, ops, &held).map_err(|e| e.to_string().into())
        }
    }
}

/// The steps that remove the label of every selected connector and line
/// link that has one.
pub(crate) fn delete_label(doc: &MindMapDocument) -> Built {
    let paths = labeled(doc)?;
    let d = doc.doc();
    let held: Vec<NodeId> = paths
        .iter()
        .filter_map(|&path| child(d, path, DocKind::Label))
        .collect();
    compose::remove(d, DocKind::Label, &held).map_err(|e| e.to_string().into())
}

/// What `portal end=<0|1> <key>=<value> …` sets on that end of each
/// selected portal link: each field given, `None` inside it to inherit.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PortalFields {
    /// Its place around its node's outline, from 0 to 1.
    pub border_t: Option<Option<f32>>,
    /// Its distance from the outline.
    pub offset: Option<Option<f32>>,
    /// The marker it draws when it has no spans.
    pub content: Option<Option<String>>,
    /// Its marker's size, in points.
    pub size: Option<Option<f32>>,
    pub color: Option<Option<ColorRef>>,
}

impl PortalFields {
    /// The ops it makes, in the order its fields are listed.
    fn ops(&self) -> Vec<DocMutation> {
        let mut ops = Vec::new();
        ops.extend(self.border_t.map(PortalEndOp::SetBorderT));
        ops.extend(self.offset.map(PortalEndOp::SetOffset));
        ops.extend(self.content.clone().map(PortalEndOp::SetContent));
        ops.extend(self.size.map(PortalEndOp::SetSize));
        ops.extend(self.color.map(PortalEndOp::SetColor));
        ops.into_iter().map(DocMutation::PortalEnd).collect()
    }
}

/// The steps that set `fields` on end `end` — 0 the source, 1 the target
/// — of every selected portal link.
pub(crate) fn portal(doc: &MindMapDocument, end: u8, fields: &PortalFields) -> Steps {
    let ops = fields.ops();
    if ops.is_empty() {
        return Err("portal: give border_t=, offset=, content=, size= or color=".to_string());
    }
    let d = doc.doc();
    let ends: Vec<NodeId> = doc
        .selection
        .iter()
        .filter(|&id| link_kind_of(d, id) == Some(LinkKind::Portal))
        .filter_map(|link| {
            children_of_kind(d, link, DocKind::PortalEnd)
                .find(|&id| matches!(d.get(id), Some(DocElement::PortalEnd(e)) if e.end == end))
        })
        .collect();
    if ends.is_empty() {
        return Err("portal: select a portal link; `link kind=portal` makes a link one".to_string());
    }
    compose::set(d, DocKind::PortalEnd, ops, &ends).map_err(|e| e.to_string())
}
