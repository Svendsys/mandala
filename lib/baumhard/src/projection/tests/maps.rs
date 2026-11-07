// SPDX-License-Identifier: MPL-2.0

//! Maps for the projection's tests, written as `.mandala.json` and built
//! by the loader — the one way a document is made outside `document`
//! (rule 4) — and the elements found in them again by node id and kind.

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
