// SPDX-License-Identifier: MPL-2.0

//! The elements an edit reaches (design §10.1, #250 §1 "Resolution"):
//! from the anchor nodes take the node set by `reach`, map it to
//! elements by `target`, keep those that pass the filter, and order them
//! in tree order without repeats. The result is the request's
//! `Anchors::Nodes`.

use crate::arena::NodeId;
use crate::document::edit::{Reach, Target};
use crate::document::element::{DocElement, DocKind};
use crate::document::ids::Document;
use crate::engine::anchor::in_tree_order;
use crate::engine::predicate::Predicate;
use rustc_hash::FxHashSet;

/// The elements `reach` and `target` name from `anchors` — each anchor
/// standing for its owning node — that pass `filter`, in tree order,
/// each once. O(the nodes reached and their children), plus the tree
/// order's sort: `descendants` and `subtree` walk the anchors' subtrees,
/// every other reach stays beside the anchors.
pub(super) fn elements(
    doc: &Document,
    anchors: &[NodeId],
    reach: Reach,
    target: Target,
    filter: Option<&Predicate<DocElement>>,
) -> Vec<NodeId> {
    let owners: FxHashSet<NodeId> = anchors.iter().filter_map(|&id| doc.owning_node(id)).collect();
    let mut nodes: FxHashSet<NodeId> = FxHashSet::default();
    for &node in &owners {
        reached(doc, node, reach, &mut nodes);
    }
    let mut found = Vec::new();
    for &node in &nodes {
        targeted(doc, node, target, &mut found);
    }
    found.retain(|&id| match (filter, doc.get(id)) {
        (Some(filter), Some(element)) => filter.test(element),
        (None, Some(_)) => true,
        (_, None) => false,
    });
    in_tree_order(doc.tree(), &found, false)
}

/// Whether `id` is a node.
fn is_node(doc: &Document, id: NodeId) -> bool {
    doc.get(id).is_some_and(|element| element.kind() == DocKind::Node)
}

/// The child nodes of `id`.
fn child_nodes(doc: &Document, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
    doc.children(id).filter(move |&child| is_node(doc, child))
}

/// Add the nodes `reach` names from `node` to `out`. Iterative: a deep
/// subtree costs its size, never stack.
fn reached(doc: &Document, node: NodeId, reach: Reach, out: &mut FxHashSet<NodeId>) {
    match reach {
        Reach::SelfNode => {
            out.insert(node);
        }
        Reach::Children => out.extend(child_nodes(doc, node)),
        Reach::Descendants | Reach::Subtree => {
            if reach == Reach::Subtree {
                out.insert(node);
            }
            let mut pending: Vec<NodeId> = child_nodes(doc, node).collect();
            while let Some(next) = pending.pop() {
                // A node already reached was reached with its subtree.
                if out.insert(next) {
                    pending.extend(child_nodes(doc, next));
                }
            }
        }
        Reach::Parent => out.extend(doc.parent(node).filter(|&parent| is_node(doc, parent))),
        Reach::Siblings => {
            if let Some(parent) = doc.parent(node) {
                out.extend(child_nodes(doc, parent).filter(|&sibling| sibling != node));
            }
        }
    }
}

/// Add the elements `target` names within `node` to `out`.
fn targeted(doc: &Document, node: NodeId, target: Target, out: &mut Vec<NodeId>) {
    let of = |parent: NodeId, kind: DocKind| {
        doc.children(parent)
            .filter(move |&child| doc.get(child).is_some_and(|element| element.kind() == kind))
    };
    match target {
        Target::Node => out.push(node),
        Target::Sections => out.extend(of(node, DocKind::Section)),
        Target::Spans => {
            for section in of(node, DocKind::Section) {
                out.extend(of(section, DocKind::Span));
            }
        }
        Target::Frame => out.extend(of(node, DocKind::Frame)),
        Target::Rings => {
            for frame in of(node, DocKind::Frame) {
                out.extend(of(frame, DocKind::Ring));
            }
        }
        Target::Connector => out.extend(of(node, DocKind::Connector)),
        Target::Links => out.extend(of(node, DocKind::Link)),
        Target::Labels => {
            for path in of(node, DocKind::Connector).chain(of(node, DocKind::Link)) {
                out.extend(of(path, DocKind::Label));
            }
        }
    }
}
