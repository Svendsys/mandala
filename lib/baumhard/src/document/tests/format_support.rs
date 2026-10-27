// SPDX-License-Identifier: MPL-2.0

//! What the format's tests and their benchmark bodies share: the
//! fixture corpus, ways to walk a loaded document, and
//! [`same_document`](crate::document::tests::format_support::same_document),
//! which compares two documents by structure.

use crate::document::element::{DocElement, DocKind, NodeData};
use crate::document::ids::{Document, NodeIdent};
use crate::document::values::LinkKind;
use indextree::NodeId;

/// The fixture corpus, by name: every key at least once
/// (`test_the_fixtures_use_every_key_the_format_defines`), every fit,
/// start and color spelling. Each is the exact text the saver writes.
pub const FIXTURES: [(&str, &str); 7] = [
    ("minimal", include_str!("../format/fixtures/minimal.mandala.json")),
    ("full", include_str!("../format/fixtures/full.mandala.json")),
    ("portals", include_str!("../format/fixtures/portals.mandala.json")),
    ("frames", include_str!("../format/fixtures/frames.mandala.json")),
    ("fits", include_str!("../format/fixtures/fits.mandala.json")),
    ("starts", include_str!("../format/fixtures/starts.mandala.json")),
    ("colors", include_str!("../format/fixtures/colors.mandala.json")),
];

/// The text [`save`](crate::document::format::save) writes for `doc`;
/// the document must be savable.
pub fn written(doc: &Document) -> String {
    crate::document::format::save(doc).unwrap_or_else(|error| panic!("the document must save: {error}"))
}

/// The text of the fixture `name`.
pub fn fixture(name: &str) -> &'static str {
    FIXTURES
        .iter()
        .find(|(fixture, _)| *fixture == name)
        .map(|(_, text)| *text)
        .unwrap_or_else(|| panic!("no fixture named {name}"))
}

/// The children of `id` of one kind, in sibling order.
pub fn kids(doc: &Document, id: NodeId, kind: DocKind) -> Vec<(NodeId, &DocElement)> {
    doc.children(id)
        .filter_map(|child| doc.get(child).map(|element| (child, element)))
        .filter(|(_, element)| element.kind() == kind)
        .collect()
}

/// The `n`th child of `id` of one kind.
pub fn kid(doc: &Document, id: NodeId, kind: DocKind, n: usize) -> (NodeId, &DocElement) {
    kids(doc, id, kind)
        .into_iter()
        .nth(n)
        .unwrap_or_else(|| panic!("no {kind:?} number {n} under {id}"))
}

/// The node with `ident`, and its data.
pub fn node(doc: &Document, ident: u32) -> (NodeId, &NodeData) {
    let id = doc
        .node_by_ident(NodeIdent(ident))
        .unwrap_or_else(|| panic!("no node {ident}"));
    match doc.get(id) {
        Some(DocElement::Node(data)) => (id, data),
        other => panic!("node {ident} is {other:?}"),
    }
}

/// Whether two documents are the same map: the same tree element for
/// element, with a link's target compared by the ident it names (an
/// arena id means nothing between two documents), the same definitions
/// and the same ident counter. `Err` says where they first differ.
pub fn same_document(a: &Document, b: &Document) -> Result<(), String> {
    if a.definitions() != b.definitions() {
        return Err("the definitions differ".to_string());
    }
    if a.next_ident() != b.next_ident() {
        return Err("the ident counters differ".to_string());
    }
    let mut left = a.tree().descendants();
    let mut right = b.tree().descendants();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ok(()),
            (Some(l), Some(r)) => {
                let (Some(x), Some(y)) = (a.get(l), b.get(r)) else {
                    return Err("an element is missing from its tree".to_string());
                };
                let levels = |doc: &Document, id: NodeId| id.ancestors(doc.tree().arena()).count();
                if levels(a, l) != levels(b, r) {
                    return Err(format!("{:?} sits at another place", x.kind()));
                }
                match (x, y) {
                    (DocElement::Link(p), DocElement::Link(q)) => {
                        let target = |doc: &Document, id| match doc.get(id) {
                            Some(DocElement::Node(node)) => Some(node.ident),
                            _ => None,
                        };
                        // A portal link's path is kept within a session and
                        // never saved (`LinkData::path`): it is the one
                        // thing a save and a load do not carry over.
                        let same_path = p.kind == LinkKind::Portal || p.path == q.path;
                        if target(a, p.to) != target(b, q.to) || p.kind != q.kind || !same_path {
                            return Err(format!("a link differs: {p:?} against {q:?}"));
                        }
                    }
                    (x, y) if x != y => return Err(format!("{x:?} differs from {y:?}")),
                    _ => {}
                }
            }
            _ => return Err("the trees have different sizes".to_string()),
        }
    }
}
