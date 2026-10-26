// SPDX-License-Identifier: MPL-2.0

//! The shape rules: which kinds may sit under which, and how many.
//!
//! | Parent | Children |
//! |---|---|
//! | Map (the root; exactly one) | Theme (exactly 1), Palette\*, Defaults (exactly 1), Node\* |
//! | Node | Section\*, Frame (≤ 1), Connector (exactly 1 under a node, none on a root node), Link\*, Node\* |
//! | Section | Span\* |
//! | Frame | Ring\* |
//! | Ring, CapStart, CapEnd, Label | Span\* |
//! | Connector | Span\*, CapStart (≤ 1), CapEnd (≤ 1), Label (≤ 1) |
//! | Link, `kind = Line` | Span\*, CapStart (≤ 1), CapEnd (≤ 1), Label (≤ 1) |
//! | Link, `kind = Portal` | PortalEnd (exactly 2: channel 0, the source end, and channel 1, the target end) |
//! | PortalEnd | Span\* (the marker), Label (≤ 1) |
//! | Span, Theme, Palette, Defaults | none |
//!
//! [`validate_shape`](crate::document::shape::validate_shape) checks one
//! element against the table: its own children, and, for a node, its
//! own connector rule. The loader
//! (V1-B2) calls it for every element it builds; a structural mutation
//! (V1-B3, V1-B4) calls it for every parent it changed and every node
//! it moved. A check is one pass over one row, with no allocation.

use crate::document::element::{DocElement, DocKind};
use crate::document::values::LinkKind;
use crate::engine::tree::Tree;
use indextree::NodeId;

/// How many children of one kind a parent may have.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Cardinality {
    /// Any number.
    Any,
    /// At most this many.
    AtMost(usize),
    /// Exactly this many.
    Exactly(usize),
}

impl Cardinality {
    /// Whether `count` children satisfy it. O(1).
    pub fn admits(self, count: usize) -> bool {
        match self {
            Cardinality::Any => true,
            Cardinality::AtMost(max) => count <= max,
            Cardinality::Exactly(n) => count == n,
        }
    }
}

/// Why an element breaks the shape rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShapeError {
    /// The id is not in the tree.
    StaleNode(NodeId),
    /// The tree's root is not a [`Map`](DocKind::Map).
    RootNotMap {
        /// The root's kind.
        kind: DocKind,
    },
    /// A [`Map`](DocKind::Map) that is not the tree's root.
    MapNotRoot(NodeId),
    /// A child of a kind its parent may not hold.
    ChildNotAllowed {
        /// The parent.
        parent: NodeId,
        /// The parent's kind.
        parent_kind: DocKind,
        /// The child.
        child: NodeId,
        /// The child's kind.
        child_kind: DocKind,
    },
    /// The wrong number of children of one kind.
    Count {
        /// The parent.
        parent: NodeId,
        /// The kind counted.
        kind: DocKind,
        /// How many there are.
        found: usize,
        /// How many the parent may have.
        allowed: Cardinality,
    },
    /// A portal link's two ends are not channel 0 and channel 1.
    PortalEnds {
        /// The link.
        link: NodeId,
    },
}

impl std::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShapeError::StaleNode(id) => write!(f, "node {id} is not in the tree"),
            ShapeError::RootNotMap { kind } => write!(f, "the root is a {kind:?}, not a Map"),
            ShapeError::MapNotRoot(id) => write!(f, "map {id} is not the root"),
            ShapeError::ChildNotAllowed {
                parent,
                parent_kind,
                child,
                child_kind,
            } => write!(f, "{parent_kind:?} {parent} cannot hold {child_kind:?} {child}"),
            ShapeError::Count {
                parent,
                kind,
                found,
                allowed,
            } => write!(f, "{parent} has {found} {kind:?} children; {allowed:?} allowed"),
            ShapeError::PortalEnds { link } => {
                write!(
                    f,
                    "portal link {link} needs one end on channel 0 and one on channel 1"
                )
            }
        }
    }
}

impl std::error::Error for ShapeError {}

/// How many children of kind `child` an element `parent` may hold, or
/// `None` when it may hold none of that kind at all. `under_node` is
/// whether `parent`'s own parent is a [`Node`](DocKind::Node), which is
/// what decides a node's connector. O(1).
pub fn cardinality(parent: &DocElement, under_node: bool, child: DocKind) -> Option<Cardinality> {
    use Cardinality::{Any, AtMost, Exactly};
    use DocKind as K;
    match (parent.kind(), child) {
        (K::Map, K::Theme | K::Defaults) => Some(Exactly(1)),
        (K::Map, K::Palette | K::Node) => Some(Any),
        (K::Node, K::Section | K::Link | K::Node) => Some(Any),
        (K::Node, K::Frame) => Some(AtMost(1)),
        (K::Node, K::Connector) => under_node.then_some(Exactly(1)),
        (K::Section | K::Ring | K::CapStart | K::CapEnd | K::Label, K::Span) => Some(Any),
        (K::Frame, K::Ring) => Some(Any),
        (K::Connector, K::Span) => Some(Any),
        (K::Connector, K::CapStart | K::CapEnd | K::Label) => Some(AtMost(1)),
        (K::Link, _) => match parent {
            DocElement::Link(link) if link.kind == LinkKind::Portal => {
                (child == K::PortalEnd).then_some(Exactly(2))
            }
            _ => match child {
                K::Span => Some(Any),
                K::CapStart | K::CapEnd | K::Label => Some(AtMost(1)),
                _ => None,
            },
        },
        (K::PortalEnd, K::Span) => Some(Any),
        (K::PortalEnd, K::Label) => Some(AtMost(1)),
        _ => None,
    }
}

/// Check the element at `id` against the shape rules: that it is a map
/// exactly when it is the root, that each of its children is of a kind
/// it may hold, that each kind appears as often as it may, and — for a
/// portal link — that its ends are channels 0 and 1. A node's connector
/// rule is its own: exactly one under a node, none on a root node.
///
/// The children are not checked below their own row; call this for each
/// element whose row changed.
///
/// Cost: one pass over `id`'s children and one over the kinds; no
/// allocation.
pub fn validate_shape(tree: &Tree<DocElement>, id: NodeId) -> Result<(), ShapeError> {
    let Some(node) = tree.get(id) else {
        return Err(ShapeError::StaleNode(id));
    };
    let element = node.get();
    let is_root = id == tree.root();
    if is_root && element.kind() != DocKind::Map {
        return Err(ShapeError::RootNotMap { kind: element.kind() });
    }
    if !is_root && element.kind() == DocKind::Map {
        return Err(ShapeError::MapNotRoot(id));
    }
    let under_node = node
        .parent()
        .is_some_and(|p| tree.arena()[p].get().kind() == DocKind::Node);

    let mut counts = [0usize; DocKind::ALL.len()];
    for child in id.children(tree.arena()) {
        let child_kind = tree.arena()[child].get().kind();
        if cardinality(element, under_node, child_kind).is_none() {
            return Err(ShapeError::ChildNotAllowed {
                parent: id,
                parent_kind: element.kind(),
                child,
                child_kind,
            });
        }
        counts[child_kind as usize] += 1;
    }
    for kind in DocKind::ALL {
        let Some(allowed) = cardinality(element, under_node, kind) else {
            continue;
        };
        let found = counts[kind as usize];
        if !allowed.admits(found) {
            return Err(ShapeError::Count {
                parent: id,
                kind,
                found,
                allowed,
            });
        }
    }
    if let DocElement::Link(link) = element {
        if link.kind == LinkKind::Portal {
            let mut ends = id.children(tree.arena()).map(|c| tree.arena()[c].get().channel());
            if (ends.next(), ends.next()) != (Some(0), Some(1)) {
                return Err(ShapeError::PortalEnds { link: id });
            }
        }
    }
    Ok(())
}
