// SPDX-License-Identifier: MPL-2.0

//! A node's own fields, on every selected node: its size (D5), its
//! outline (D11), and `hidden`, `opacity`, `folded`, `channel` and its
//! palette (D17). A node draws no fill (D12), so there is no fill and no
//! fade to set.

use baumhard::document::element::{DocElement, DocKind};
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{DocMutation, NodeOp};
use baumhard::document::values::{PaletteId, Shape};
use glam::Vec2;

use super::{selected, values, Groups, Steps};
use crate::application::document::MindMapDocument;

/// What a `node` line sets: each field given.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NodeFields {
    /// `shape=`: the outline, its radius kept when it stays a
    /// rectangle and `radius` is not given.
    pub shape: Option<ShapeKind>,
    /// `radius=`: the rectangle's corner radius; a node that is not a
    /// rectangle becomes one.
    pub radius: Option<u8>,
    pub opacity: Option<f32>,
    pub hidden: Option<bool>,
    pub folded: Option<bool>,
    pub channel: Option<usize>,
    /// `palette=`: `Some(None)` unbinds.
    pub palette: Option<Option<PaletteId>>,
}

/// The outline `shape=` names.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum ShapeKind {
    Rectangle,
    Ellipse,
}

/// `shape=`'s word.
pub(crate) fn shape_kind(text: &str) -> Result<ShapeKind, String> {
    match text.to_ascii_lowercase().as_str() {
        "rectangle" => Ok(ShapeKind::Rectangle),
        "ellipse" => Ok(ShapeKind::Ellipse),
        _ => Err(format!("'{text}' is not a shape: `rectangle` or `ellipse`")),
    }
}

/// The palette `name` names in `doc`, or `none` to unbind.
pub(crate) fn palette(doc: &MindMapDocument, name: &str) -> Result<Option<PaletteId>, String> {
    if name.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    doc.doc()
        .palettes()
        .find(|(_, palette)| palette.name == name)
        .map(|(id, _)| Some(id))
        .ok_or_else(|| format!("'{name}' is not a palette of this map"))
}

/// The selected nodes, or the refusal that says to select one.
fn nodes(doc: &MindMapDocument, verb: &str) -> Result<Vec<baumhard::arena::NodeId>, String> {
    let nodes = selected(doc, DocKind::Node);
    if nodes.is_empty() {
        return Err(format!("{verb}: select a node"));
    }
    Ok(nodes)
}

/// The steps that set every selected node's minimum size: `Some` pins
/// it, `None` fits the node to its content (`node fit`, D5).
pub(crate) fn min_size(doc: &MindMapDocument, size: Option<Vec2>) -> Steps {
    compose::set(
        doc.doc(),
        DocKind::Node,
        vec![DocMutation::Node(NodeOp::SetMinSize(size))],
        &nodes(doc, "node")?,
    )
    .map_err(|e| e.to_string())
}

/// The steps that set `fields` on every selected node, in one walk per
/// distinct outcome.
pub(crate) fn fields(doc: &MindMapDocument, fields: &NodeFields) -> Steps {
    let nodes = nodes(doc, "node")?;
    let d = doc.doc();
    let mut groups = Groups::default();
    for node in nodes {
        let Some(DocElement::Node(data)) = d.get(node) else { continue };
        let mut ops = Vec::new();
        let kept_radius = match data.style.shape {
            Some(Shape::Rectangle { radius }) => radius.get(),
            _ => 0,
        };
        let shape = match (fields.shape, fields.radius) {
            (Some(ShapeKind::Ellipse), _) => Some(Shape::Ellipse),
            (Some(ShapeKind::Rectangle), radius) => Some(values::rectangle(radius.unwrap_or(kept_radius))),
            (None, Some(radius)) => Some(values::rectangle(radius)),
            (None, None) => None,
        };
        ops.extend(shape.map(|shape| NodeOp::SetShape(Some(shape))));
        ops.extend(fields.opacity.map(NodeOp::SetOpacity));
        ops.extend(fields.hidden.map(NodeOp::SetHidden));
        ops.extend(fields.folded.map(NodeOp::SetFolded));
        ops.extend(fields.channel.map(NodeOp::SetChannel));
        ops.extend(fields.palette.map(NodeOp::SetPalette));
        groups.add(d, node, ops.into_iter().map(DocMutation::Node).collect());
    }
    if groups.is_empty() {
        return Err("node: give a field to set".to_string());
    }
    groups.steps(d)
}
