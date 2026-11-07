// SPDX-License-Identifier: MPL-2.0

//! Reading the view from outside: where an element is drawn, what text
//! it shows, and the name the IPC surface gives it — what `scene.*`,
//! `query.selection`, the event stream and a capture's geometry sidecar
//! report. Cross-platform, read-only.
//!
//! **An element's extent is the union of its glyph runs.** The projector
//! knows which runs every element produced (`Projector::glyphs_of`), and
//! each run knows its bounds, so a node, a section, a path, a label and
//! a portal end are measured the same way: by what the renderer draws for
//! them, and nothing else. An element that draws nothing (a hidden node,
//! an empty section) has no extent.
//!
//! **The wire name of an element is its place in the document**
//! (`format/ipc.md` §"Naming elements"): `{"node": <ident>}` for a node,
//! and below a node the kinds on the way down, each with its index among
//! its parent's children of that kind — `{"node": 3, "section": 1}`,
//! `{"node": 3, "link": 0, "label": 0}` — or `true` for the kinds a parent
//! holds at most one of (`{"node": 3, "connector": true}`). Integer node
//! idents are stable while the document is open, so a name read from one
//! reply addresses the same element in the next.

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, DocKind};
use baumhard::document::ids::{Document, NodeIdent};
use baumhard::flow::Rect;
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::gfx_structs::run::GlyphRunData;
use baumhard::projection::{GlyphLayers, Layer, Projector};
use glam::Vec2;
use serde_json::{Map, Value};

use crate::application::renderer::content_bounds;

/// The glyph runs `element` produced, each with its layer: a node's own
/// runs in the sections and frames layers — its groups' children, not
/// its child nodes' — or the runs of any other element. O(the runs).
pub(crate) fn runs_of(doc: &Document, projector: &Projector, layers: &GlyphLayers, element: NodeId) -> Vec<(Layer, NodeId)> {
    let is_run = |layer: Layer, glyph: NodeId| {
        layers
            .tree(layer)
            .get(glyph)
            .is_some_and(|node| node.get().glyph_run().is_some())
    };
    match doc.get(element).map(DocElement::kind) {
        Some(DocKind::Node) => projector
            .glyphs_of(element)
            .into_iter()
            .filter(|(layer, _)| matches!(layer, Layer::Sections | Layer::Frames))
            .flat_map(|(layer, group)| {
                let tree = layers.tree(layer);
                group
                    .children(tree.arena())
                    .filter(|&child| is_run(layer, child))
                    .map(move |child| (layer, child))
                    .collect::<Vec<_>>()
            })
            .collect(),
        Some(_) => projector
            .glyphs_of(element)
            .into_iter()
            .filter(|&(layer, glyph)| is_run(layer, glyph))
            .collect(),
        None => Vec::new(),
    }
}

/// The run data of `runs`, in order. No allocation.
fn run_data<'l>(layers: &'l GlyphLayers, runs: &'l [(Layer, NodeId)]) -> impl Iterator<Item = &'l GlyphRunData> + 'l {
    runs.iter()
        .filter_map(|&(layer, glyph)| layers.tree(layer).get(glyph)?.get().glyph_run())
}

/// The canvas-space box `element` is drawn in: the union of its runs'
/// bounds, or `None` when it draws nothing. O(its runs).
pub(crate) fn element_bounds(doc: &Document, projector: &Projector, layers: &GlyphLayers, element: NodeId) -> Option<Rect> {
    let runs = runs_of(doc, projector, layers, element);
    let (min, max) = content_bounds(run_data(layers, &runs))?;
    Some(Rect::new(min, max - min))
}

/// Whether `element` is drawn at `camera`, as the wire says it:
/// `"visible"` when one of its runs is inside its zoom window and on
/// screen, `"outside_zoom_window"` when none is inside its zoom window (a
/// pan would not bring it back), `"offscreen"` otherwise; `None` when it
/// draws nothing at all. O(its runs).
pub(crate) fn element_visibility(
    doc: &Document,
    projector: &Projector,
    layers: &GlyphLayers,
    element: NodeId,
    camera: &Camera2D,
) -> Option<&'static str> {
    let runs = runs_of(doc, projector, layers, element);
    let mut any = false;
    let mut in_window = false;
    for run in run_data(layers, &runs) {
        any = true;
        let inside = run.zoom().is_none_or(|window| {
            window.min.is_none_or(|min| camera.zoom >= min) && window.max.is_none_or(|max| camera.zoom <= max)
        });
        if !inside {
            continue;
        }
        in_window = true;
        let (min, size) = run.bounds();
        if camera.is_visible(min, size) {
            return Some("visible");
        }
    }
    match (any, in_window) {
        (false, _) => None,
        (true, false) => Some("outside_zoom_window"),
        (true, true) => Some("offscreen"),
    }
}

/// `rect` on the surface `camera` draws: its top-left corner moved and
/// its size scaled by the camera. O(1).
pub(crate) fn surface_rect(rect: Rect, camera: &Camera2D) -> Rect {
    Rect::new(camera.canvas_to_screen(rect.origin), rect.size * camera.pixels_per_unit())
}

/// The text `element` holds: every span below it, in document order,
/// concatenated. Empty for an element with no spans. O(its subtree).
pub(crate) fn element_text(doc: &Document, element: NodeId) -> String {
    let tree = doc.tree();
    element
        .descendants(tree.arena())
        .filter_map(|id| match doc.get(id) {
            Some(DocElement::Span(span)) => Some(span.text.as_str()),
            _ => None,
        })
        .collect()
}

/// The elements of `kind` directly under `parent`, in document order.
pub(crate) fn children_of_kind(doc: &Document, parent: NodeId, kind: DocKind) -> impl Iterator<Item = NodeId> + '_ {
    doc.children(parent)
        .filter(move |&child| doc.get(child).map(DocElement::kind) == Some(kind))
}

/// The wire word for an element kind below a node, and whether a parent
/// holds at most one of it (named `true` rather than by index).
fn kind_word(kind: DocKind) -> Option<(&'static str, bool)> {
    Some(match kind {
        DocKind::Section => ("section", false),
        DocKind::Span => ("span", false),
        DocKind::Frame => ("frame", true),
        DocKind::Ring => ("ring", false),
        DocKind::Connector => ("connector", true),
        DocKind::CapStart => ("cap_start", true),
        DocKind::CapEnd => ("cap_end", true),
        DocKind::Label => ("label", false),
        DocKind::Link => ("link", false),
        DocKind::PortalEnd => ("portal_end", false),
        DocKind::Map | DocKind::Theme | DocKind::Palette | DocKind::Defaults | DocKind::Node => return None,
    })
}

/// The ident of `node`, or `None` for an element that is not a node.
pub(crate) fn ident_of(doc: &Document, node: NodeId) -> Option<NodeIdent> {
    match doc.get(node) {
        Some(DocElement::Node(data)) => Some(data.ident),
        _ => None,
    }
}

/// The wire name of `element` ([module docs](self)), or `null` for one
/// that belongs to no node (the map, the theme, a palette, the defaults)
/// or that the document does not hold. O(its depth within the node, times
/// its siblings).
pub(crate) fn element_ref(doc: &Document, element: NodeId) -> Value {
    let Some(node) = doc.owning_node(element) else {
        return Value::Null;
    };
    let Some(ident) = ident_of(doc, node) else {
        return Value::Null;
    };
    let mut path = Vec::new();
    let mut at = element;
    while at != node {
        path.push(at);
        match doc.parent(at) {
            Some(parent) => at = parent,
            None => return Value::Null,
        }
    }
    let mut name = Map::new();
    name.insert("node".to_string(), Value::from(ident.0));
    for &step in path.iter().rev() {
        let Some(kind) = doc.get(step).map(DocElement::kind) else {
            return Value::Null;
        };
        let Some((word, unique)) = kind_word(kind) else {
            return Value::Null;
        };
        let value = if unique {
            Value::Bool(true)
        } else {
            let parent = doc.parent(step).expect("a step below the node has a parent");
            let index = children_of_kind(doc, parent, kind)
                .position(|sibling| sibling == step)
                .expect("an element is among its parent's children");
            Value::from(index)
        };
        name.insert(word.to_string(), value);
    }
    Value::Object(name)
}

/// The node a request names by its integer ident, or the message to
/// refuse it with.
pub(crate) fn node_named(doc: &Document, value: &Value) -> Result<NodeId, String> {
    let ident = value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| format!("a node is named by its integer id, not {value}"))?;
    doc.node_by_ident(NodeIdent(ident))
        .ok_or_else(|| format!("no node {ident} in the document"))
}

/// A canvas rect as the wire carries it: `{x, y, w, h}`.
pub(crate) fn rect_json(rect: Rect) -> Value {
    serde_json::json!({
        "x": f32_json(rect.origin.x),
        "y": f32_json(rect.origin.y),
        "w": f32_json(rect.size.x),
        "h": f32_json(rect.size.y),
    })
}

/// A point as the wire carries it: `{x, y}`.
pub(crate) fn point_json(point: Vec2) -> Value {
    serde_json::json!({ "x": f32_json(point.x), "y": f32_json(point.y) })
}

/// An `f32` as the JSON number a reader expects: the shortest decimal
/// that names it (`0.1`, not `0.10000000149011612`), through Baumhard's
/// [`shortest_f64`](baumhard::util::shortest_float::shortest_f64). A
/// non-finite value, which JSON cannot carry, is `null`.
pub(crate) fn f32_json(value: f32) -> Value {
    baumhard::util::shortest_float::shortest_f64(value)
        .and_then(serde_json::Number::from_f64)
        .map_or(Value::Null, Value::Number)
}
