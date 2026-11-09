// SPDX-License-Identifier: MPL-2.0

//! A node's own fields, on every selected node: its size (D5), its
//! outline (D11), and `hidden`, `opacity`, `folded`, `channel` and its
//! palette (D17). A node draws no fill (D12), so there is no fill and no
//! fade to set.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{DocMutation, NodeOp};
use baumhard::document::values::{PaletteId, Shape};
use glam::Vec2;

use super::{selected, Steps};
use crate::application::document::MindMapDocument;

/// What a `node` line sets: each field given, the same on every node.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NodeFields {
    /// The outline (`shape=`, `radius=`).
    pub shape: Option<Shape>,
    pub opacity: Option<f32>,
    pub hidden: Option<bool>,
    pub folded: Option<bool>,
    pub channel: Option<usize>,
    /// `palette=`: `Some(None)` unbinds.
    pub palette: Option<Option<PaletteId>>,
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
fn nodes(doc: &MindMapDocument) -> Result<Vec<NodeId>, String> {
    let nodes = selected(doc, DocKind::Node);
    if nodes.is_empty() {
        return Err("node: select a node".to_string());
    }
    Ok(nodes)
}

/// The steps that set every selected node's minimum size: `Some` pins
/// it, `None` fits the node to its content (`node fit`, D5).
pub(crate) fn min_size(doc: &MindMapDocument, size: Option<Vec2>) -> Steps {
    let set = vec![DocMutation::Node(NodeOp::SetMinSize(size))];
    compose::set(doc.doc(), DocKind::Node, set, &nodes(doc)?).map_err(|e| e.to_string())
}

/// The steps that set `fields` on every selected node, as one walk.
pub(crate) fn fields(doc: &MindMapDocument, fields: &NodeFields) -> Steps {
    let mut ops = Vec::new();
    ops.extend(fields.shape.map(|shape| NodeOp::SetShape(Some(shape))));
    ops.extend(fields.opacity.map(NodeOp::SetOpacity));
    ops.extend(fields.hidden.map(NodeOp::SetHidden));
    ops.extend(fields.folded.map(NodeOp::SetFolded));
    ops.extend(fields.channel.map(NodeOp::SetChannel));
    ops.extend(fields.palette.map(NodeOp::SetPalette));
    if ops.is_empty() {
        return Err("node: give a field to set".to_string());
    }
    let ops = ops.into_iter().map(DocMutation::Node).collect();
    compose::set(doc.doc(), DocKind::Node, ops, &nodes(doc)?).map_err(|e| e.to_string())
}
