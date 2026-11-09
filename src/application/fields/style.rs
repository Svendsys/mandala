// SPDX-License-Identifier: MPL-2.0

//! Style across the selection: `color`, `font` and `zoom`, applied to
//! every selected element whatever its kind — nodes, sections, paths
//! (connectors and links), labels, caps and portal ends — and to a
//! selected range of a section's text ([`Selection::range`]).
//!
//! | Selected | `text` color | `border` color | `ring` color | font | zoom |
//! |---|---|---|---|---|---|
//! | a node | its text | its frame's | each ring's own | its text | its window |
//! | a section | its text | — | — | its text | — |
//! | a range of a section | the spans in it | — | — | the spans in it | — |
//! | a connector or a link | its color | its color | — | size its own; face, weight, slant its spans' | its window |
//! | a label or a cap | its text | its text | — | its text | — |
//! | a portal end | its marker | its marker | — | size its own; the rest its spans' | — |
//!
//! `border` on a path, a label, a cap or a portal end is `text`, as on
//! `main`, where each had one color. A key that reaches nothing selected
//! is refused, naming the key.
//!
//! [`Selection::range`]: crate::application::document::Selection::range

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, DocKind};
use baumhard::document::ids::Document;
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{
    DocMutation, FrameOp, LabelOp, NodeOp, PathOp, PortalEndOp, RingOp, SectionOp, SpanOp, TextOp,
};
use baumhard::document::values::{ColorRef, FontSize, Weight, ZoomWindow};
use baumhard::font::fonts::AppFont;

use super::{children, Groups, Steps};
use crate::application::document::selection::kind_of;
use crate::application::document::MindMapDocument;
use crate::application::keybinds::{ColorAxis, ZoomBound};

/// `op` as the mutation of an element of `kind` that holds a text
/// style, or `None` for a kind that holds none of its own.
pub(crate) fn text_op(kind: DocKind, op: TextOp) -> Option<DocMutation> {
    Some(match kind {
        DocKind::Node => DocMutation::Node(NodeOp::Text(op)),
        DocKind::Section => DocMutation::Section(SectionOp::Text(op)),
        DocKind::Span => DocMutation::Span(SpanOp::Text(op)),
        DocKind::Frame => DocMutation::Frame(FrameOp::Text(op)),
        DocKind::Ring => DocMutation::Ring(RingOp::Text(op)),
        DocKind::Label => DocMutation::Label(LabelOp::Text(op)),
        DocKind::CapStart | DocKind::CapEnd => DocMutation::Cap(op),
        _ => return None,
    })
}

/// The selection's range, when it has one and its element is a section.
fn range(doc: &MindMapDocument) -> Option<(NodeId, usize, usize)> {
    doc.selection
        .range()
        .filter(|&(section, _, _)| kind_of(doc.doc(), section) == Some(DocKind::Section))
}

/// The frame of `node`, if it has one.
pub(crate) fn frame_of(doc: &Document, node: NodeId) -> Option<NodeId> {
    children(doc, node, DocKind::Frame).next()
}

/// The steps that set color `axis` to `color` (`None` inherits) on
/// every selected element it reaches.
pub(crate) fn color(doc: &MindMapDocument, axis: ColorAxis, color: Option<ColorRef>) -> Steps {
    let d = doc.doc();
    let key: &'static str = axis.into();
    if let (ColorAxis::Text, Some((section, start, end))) = (axis, range(doc)) {
        return compose::style_range(d, section, start, end, TextOp::SetColor(color))
            .map_err(|e| e.to_string());
    }
    let mut groups = Groups::default();
    let text = |kind| text_op(kind, TextOp::SetColor(color));
    for id in doc.selection.iter() {
        let Some(kind) = kind_of(d, id) else { continue };
        match (kind, axis) {
            (DocKind::Connector | DocKind::Link, ColorAxis::Text | ColorAxis::Border) => {
                groups.add(d, id, vec![DocMutation::Path(PathOp::SetColor(color))])
            }
            (DocKind::PortalEnd, ColorAxis::Text | ColorAxis::Border) => {
                groups.add(d, id, vec![DocMutation::PortalEnd(PortalEndOp::SetColor(color))])
            }
            (DocKind::Label | DocKind::CapStart | DocKind::CapEnd, ColorAxis::Text | ColorAxis::Border)
            | (DocKind::Node | DocKind::Section | DocKind::Span, ColorAxis::Text)
            | (DocKind::Frame | DocKind::Ring, ColorAxis::Border)
            | (DocKind::Ring, ColorAxis::Ring) => groups.add(d, id, text(kind).into_iter().collect()),
            (DocKind::Node, ColorAxis::Border) => {
                if let Some(frame) = frame_of(d, id) {
                    groups.add(d, frame, text(DocKind::Frame).into_iter().collect());
                }
            }
            (DocKind::Node | DocKind::Frame, ColorAxis::Ring) => {
                let frame = if kind == DocKind::Node {
                    frame_of(d, id)
                } else {
                    Some(id)
                };
                for ring in frame
                    .into_iter()
                    .flat_map(|frame| children(d, frame, DocKind::Ring))
                {
                    groups.add(d, ring, text(DocKind::Ring).into_iter().collect());
                }
            }
            _ => {}
        }
    }
    if groups.is_empty() {
        return Err(format!("color {key}: nothing selected takes a {key} color"));
    }
    groups.steps(d)
}

/// What a `font` line or Action sets: each field given, `None` inside
/// the field to inherit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Font {
    pub family: Option<Option<AppFont>>,
    pub size: Option<Option<f32>>,
    pub weight: Option<Option<Weight>>,
    pub italic: Option<Option<bool>>,
}

impl Font {
    /// The text ops it makes, size in points, in the order the fields
    /// are listed. `with_size` leaves the size out, for the elements
    /// whose size is not a text style's.
    fn ops(&self, with_size: bool) -> Vec<TextOp> {
        let mut ops = Vec::new();
        ops.extend(self.family.map(TextOp::SetFont));
        if with_size {
            ops.extend(self.size.map(|size| TextOp::SetSize(size.map(FontSize::Pt))));
        }
        ops.extend(self.weight.map(TextOp::SetWeight));
        ops.extend(self.italic.map(TextOp::SetItalic));
        ops
    }
}

/// The steps that set `font` on every selected element.
pub(crate) fn font(doc: &MindMapDocument, font: Font) -> Steps {
    let d = doc.doc();
    if let Some((section, start, end)) = range(doc) {
        let mut steps = Vec::new();
        for op in font.ops(true) {
            steps.extend(compose::style_range(d, section, start, end, op).map_err(|e| e.to_string())?);
        }
        return Ok(steps);
    }
    let mut groups = Groups::default();
    for id in doc.selection.iter() {
        let Some(kind) = kind_of(d, id) else { continue };
        match kind {
            DocKind::Connector | DocKind::Link | DocKind::PortalEnd => {
                let size = font.size.map(|size| match kind {
                    DocKind::PortalEnd => DocMutation::PortalEnd(PortalEndOp::SetSize(size)),
                    _ => DocMutation::Path(PathOp::SetSize(size)),
                });
                groups.add(d, id, size.into_iter().collect());
                let spans: Vec<DocMutation> = font
                    .ops(false)
                    .into_iter()
                    .filter_map(|op| text_op(DocKind::Span, op))
                    .collect();
                for span in children(d, id, DocKind::Span) {
                    groups.add(d, span, spans.clone());
                }
            }
            DocKind::Node
            | DocKind::Section
            | DocKind::Span
            | DocKind::Label
            | DocKind::CapStart
            | DocKind::CapEnd => groups.add(
                d,
                id,
                font.ops(true)
                    .into_iter()
                    .filter_map(|op| text_op(kind, op))
                    .collect(),
            ),
            _ => {}
        }
    }
    if groups.is_empty() {
        return Err("font: nothing selected takes these font fields".to_string());
    }
    groups.steps(d)
}

/// The zoom window an element of `doc` holds now.
fn window_of(doc: &Document, id: NodeId) -> Option<ZoomWindow> {
    match doc.get(id) {
        Some(DocElement::Node(node)) => node.zoom,
        Some(DocElement::Connector(connector)) => connector.path.zoom,
        Some(DocElement::Link(link)) => link.path.zoom,
        _ => None,
    }
}

/// A change to zoom windows: each bound given is set, `None` inside
/// opening it; a bound not given keeps each element's own. Both opened
/// is no window.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Zoom {
    pub min: Option<Option<f32>>,
    pub max: Option<Option<f32>>,
}

impl Zoom {
    /// Both bounds opened: `zoom clear`.
    pub(crate) const CLEAR: Zoom = Zoom {
        min: Some(None),
        max: Some(None),
    };

    /// One bound, set to `value`.
    pub(crate) fn bound(bound: ZoomBound, value: Option<f32>) -> Zoom {
        match bound {
            ZoomBound::Min => Zoom {
                min: Some(value),
                ..Zoom::default()
            },
            ZoomBound::Max => Zoom {
                max: Some(value),
                ..Zoom::default()
            },
        }
    }
}

/// The steps that apply `change` to every selected node's and path's
/// zoom window, one walk per distinct window that results.
pub(crate) fn zoom(doc: &MindMapDocument, change: Zoom) -> Steps {
    let d = doc.doc();
    let mut groups = Groups::default();
    for id in doc.selection.iter() {
        let kind = kind_of(d, id);
        if !matches!(kind, Some(DocKind::Node | DocKind::Connector | DocKind::Link)) {
            continue;
        }
        let old = window_of(d, id).unwrap_or(ZoomWindow { min: None, max: None });
        let window = ZoomWindow {
            min: change.min.unwrap_or(old.min),
            max: change.max.unwrap_or(old.max),
        };
        let window = (window.min.is_some() || window.max.is_some()).then_some(window);
        let mutation = match kind {
            Some(DocKind::Node) => DocMutation::Node(NodeOp::SetZoom(window)),
            _ => DocMutation::Path(PathOp::SetZoom(window)),
        };
        groups.add(d, id, vec![mutation]);
    }
    if groups.is_empty() {
        return Err("zoom: select a node, a connector or a link".to_string());
    }
    groups.steps(d)
}
