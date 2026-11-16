// SPDX-License-Identifier: MPL-2.0

//! Maps for the projection's tests, written as `.mandala.json` and built
//! by the loader — the one way a document is made outside `document`
//! (rule 4) — and the elements found in them again by node id and kind.

use glam::Vec2;
use serde_json::{json, Value};

use crate::arena::NodeId;
use crate::document::element::DocKind;
use crate::document::format::load;
use crate::document::ids::{Document, NodeIdent};

/// The map of `value`'s fields — `nodes`, and whatever header keys it
/// sets — loaded. Panics, naming the loader's refusal, on a map the
/// loader refuses: a test's map is meant to load.
pub fn load_map(mut value: Value) -> Document {
    if let Some(map) = value.as_object_mut() {
        map.entry("format").or_insert(json!(1));
        map.entry("name").or_insert(json!("test"));
    }
    let text = serde_json::to_string(&value).expect("a JSON value writes");
    load(&text).unwrap_or_else(|e| panic!("a test map loads: {e}\n{text}"))
}

/// A node record: `id`, its `parent` when it has one, a position at the
/// origin, and `fields` merged in (a `position` there replaces it).
pub fn node(id: u32, parent: Option<u32>, fields: Value) -> Value {
    let mut out = json!({"id": id, "position": [0.0, 0.0]});
    if let Some(parent) = parent {
        out["parent"] = json!(parent);
    }
    if let (Some(out), Some(fields)) = (out.as_object_mut(), fields.as_object()) {
        for (k, v) in fields {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

/// The node `doc` gives the id `id`.
pub fn ident(doc: &Document, id: u32) -> NodeId {
    doc.node_by_ident(NodeIdent(id)).expect("the map has the node")
}

/// The children of `parent` of `kind`, in order.
pub fn kids(doc: &Document, parent: NodeId, kind: DocKind) -> Vec<NodeId> {
    doc.children(parent)
        .filter(|&c| doc.get(c).is_some_and(|e| e.kind() == kind))
        .collect()
}

/// The first child of `parent` of `kind`.
pub fn kid(doc: &Document, parent: NodeId, kind: DocKind) -> NodeId {
    kids(doc, parent, kind)[0]
}

/// A section of one span of `text`.
pub fn text_section(text: &str) -> Value {
    json!({"spans": [{"text": text}]})
}

/// Where the blocker, node 3, stands off both paths.
pub const AWAY: Vec2 = Vec2::new(480.0, 1000.0);

/// Where the blocker stands across path P, the connector of node 2.
pub const ON_P: Vec2 = Vec2::new(480.0, -10.0);

/// Where the blocker stands across path Q, the connector of node 5.
pub const ON_Q: Vec2 = Vec2::new(480.0, 1990.0);

/// The blocker's text: one line, 30 points, whose glyphs' cells are
/// what it hides by (#332).
pub fn blocker_text() -> Value {
    json!([{"size": 30.0, "spans": [{"text": "HIDE"}]}])
}

/// A blocker and two paths (#332): two straight connectors 900 long, P
/// from node 1 to node 2 along y = 20 and Q from node 4 to node 5 along
/// y = 2020, eight cells of the grid apart; and node 3 at `at`, which
/// ends neither: [`blocker_text`] in a box at least 60 by 60, and
/// `blocker`'s fields merged over that. `path` is merged into both
/// connectors.
pub fn two_paths(at: Vec2, blocker: Value, path: Value) -> Document {
    let box_ = json!({"min_size": [100.0, 40.0]});
    let connector = merged(json!({"straight": true}), path);
    let blocker = merged(
        json!({"min_size": [60.0, 60.0], "position": [at.x, at.y], "sections": blocker_text()}),
        blocker,
    );
    let at = |x: f32, y: f32, mut fields: Value| {
        fields["position"] = json!([x, y]);
        fields
    };
    let child = |x: f32, y: f32| {
        let mut fields = at(x, y, box_.clone());
        fields["connector"] = connector.clone();
        fields
    };
    let (p, q) = (child(1000.0, 0.0), child(1000.0, 2000.0));
    load_map(json!({"nodes": [
        node(1, None, at(0.0, 0.0, box_.clone())),
        node(2, Some(1), p),
        node(3, None, blocker),
        node(4, None, at(0.0, 2000.0, box_.clone())),
        node(5, Some(4), q),
    ]}))
}

/// Lengths a map can carry that put a rectangle past what an `i64` counts
/// of the body index's cells (#364): 4·10¹¹, the least found to; the nine
/// frame insets the defect was seen at, 5·10¹¹ to 10³⁷; and 4·10³⁷, found
/// beside them. A box that far each way from a node spans at least
/// 3,125,000,000 cells a side.
pub const HUGE_LENGTHS: [f32; 11] = [
    4.0e11, 5.0e11, 6.0e11, 1.0e12, 1.0e13, 1.0e16, 1.0e19, 1.0e25, 1.0e30, 1.0e37, 4.0e37,
];

/// A node's drawn rectangle, 100 by 40 at the origin, grown by `length`
/// each way: the box round a ring set `length` out from it.
pub fn huge_box(length: f32) -> crate::flow::Rect {
    crate::flow::Rect::new(Vec2::ZERO, Vec2::new(100.0, 40.0)).grown(length)
}

/// The boxes the hide stage tries `path`'s laid glyphs by: each glyph's
/// box grown by the path's gap.
pub fn grown_boxes(p: &crate::projection::tests::support::Projected, path: NodeId) -> Vec<crate::flow::Rect> {
    use crate::gfx_structs::run::RunShape;
    use crate::projection::stages::{gap_and_size, glyph_ink};
    use crate::projection::state::UnitBody;
    let Some(UnitBody::Line(line)) = p.projector.units.get(&path).map(|u| &u.body) else {
        panic!("{path:?} is a line");
    };
    let gap = gap_and_size(line).0.max(0.0);
    let Some(RunShape::Path { run: shaped, placed }) = line.laid[0].as_ref().map(|run| run.shaped()) else {
        panic!("{path:?}'s body is laid");
    };
    placed
        .glyphs
        .iter()
        .filter_map(|g| glyph_ink(shaped, g))
        .map(|ink| ink.grown(gap))
        .collect()
}

/// How many glyphs of `path`'s body are laid, and how many of them the
/// hide stage leaves to be drawn.
pub fn body_glyphs(p: &crate::projection::tests::support::Projected, path: NodeId) -> (usize, usize) {
    let count =
        |run: Option<&crate::gfx_structs::run::GlyphRunData>| run.map_or(0, |r| r.shaped().glyph_count());
    let drawn = p
        .projector
        .units
        .get(&path)
        .and_then(|u| u.slots[0].want.as_ref());
    (count(p.projector.laid(path)), count(drawn))
}

/// `fields` with `more` merged in.
pub fn merged(mut fields: Value, more: Value) -> Value {
    if let (Some(out), Some(more)) = (fields.as_object_mut(), more.as_object()) {
        for (k, v) in more {
            out.insert(k.clone(), v.clone());
        }
    }
    fields
}
