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
//! One function holds the table:
//! [`validate_row`](crate::document::shape::validate_row) judges one
//! element and the kinds of its children, in any order, naming no arena
//! id — so it serves a row of the tree and a row of a detached subtree
//! alike. [`validate_shape`](crate::document::shape::validate_shape) is
//! that check for an element of the tree: the loader (V1-B2) calls it
//! for every element it builds; a structural mutation (V1-B3, V1-B4)
//! calls it for every parent it changed and every node it moved. A
//! check is one pass over one row, with no allocation.
//!
//! **A node's connector rule belongs to where it sits**: exactly one
//! connector under a node, none on a root node. A subtree's root has no
//! parent until it is inserted, so its row is judged with the parent it
//! will be inserted under (`under_node`), or by `validate_shape` once it
//! is in place.

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

/// Why one row breaks the shape rules. Names kinds and positions, never
/// an arena id, so a row with no ids — a detached subtree's — can be
/// judged too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowError {
    /// The tree's root is not a [`Map`](DocKind::Map).
    RootNotMap {
        /// The root's kind.
        kind: DocKind,
    },
    /// A [`Map`](DocKind::Map) that is not the tree's root.
    MapNotRoot,
    /// A child of a kind its parent may not hold.
    ChildNotAllowed {
        /// The parent's kind.
        parent_kind: DocKind,
        /// The child's position among the children as given.
        position: usize,
        /// The child's kind.
        child_kind: DocKind,
    },
    /// The wrong number of children of one kind.
    Count {
        /// The kind counted.
        kind: DocKind,
        /// How many there are.
        found: usize,
        /// How many the parent may have.
        allowed: Cardinality,
    },
    /// A portal link's two ends are not one on channel 0 and one on
    /// channel 1.
    PortalEnds,
}

impl std::fmt::Display for RowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RowError::RootNotMap { kind } => write!(f, "the root is a {kind:?}, not a Map"),
            RowError::MapNotRoot => f.write_str("a map below the root"),
            RowError::ChildNotAllowed {
                parent_kind,
                position,
                child_kind,
            } => write!(
                f,
                "a {parent_kind:?} cannot hold a {child_kind:?} (child {position})"
            ),
            RowError::Count { kind, found, allowed } => {
                write!(f, "{found} {kind:?} children; {allowed:?} allowed")
            }
            RowError::PortalEnds => {
                f.write_str("a portal link needs one end on channel 0 and one on channel 1")
            }
        }
    }
}

impl std::error::Error for RowError {}

/// Why an element of a tree breaks the shape rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShapeError {
    /// The id is not in the tree.
    StaleNode(NodeId),
    /// The row of `element` breaks a rule.
    Row {
        /// The element whose row was judged.
        element: NodeId,
        /// What is wrong with it.
        error: RowError,
    },
}

impl std::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShapeError::StaleNode(id) => write!(f, "node {id} is not in the tree"),
            ShapeError::Row { element, error } => write!(f, "{element}: {error}"),
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

/// Judge one row: `parent` and its children, given in any order. The
/// parent is a map exactly when `is_root`; each child is of a kind the
/// parent may hold; each kind appears as often as it may; a portal
/// link's ends are one on channel 0 and one on channel 1. `under_node`
/// is whether the parent's own parent is a node, which decides a node's
/// connector: exactly one under a node, none on a root node.
///
/// The children are not judged below their own row. Cost: one pass over
/// the children and one over the kinds; no allocation.
pub fn validate_row<'a>(
    parent: &DocElement,
    is_root: bool,
    under_node: bool,
    children: impl IntoIterator<Item = &'a DocElement>,
) -> Result<(), RowError> {
    if is_root && parent.kind() != DocKind::Map {
        return Err(RowError::RootNotMap { kind: parent.kind() });
    }
    if !is_root && parent.kind() == DocKind::Map {
        return Err(RowError::MapNotRoot);
    }
    let mut counts = [0usize; DocKind::ALL.len()];
    let mut ends = [false; 2];
    for (position, child) in children.into_iter().enumerate() {
        let child_kind = child.kind();
        if cardinality(parent, under_node, child_kind).is_none() {
            return Err(RowError::ChildNotAllowed {
                parent_kind: parent.kind(),
                position,
                child_kind,
            });
        }
        counts[child_kind as usize] += 1;
        if let DocElement::PortalEnd(end) = child {
            if let Some(seen) = ends.get_mut(usize::from(end.end)) {
                *seen = true;
            }
        }
    }
    for kind in DocKind::ALL {
        let Some(allowed) = cardinality(parent, under_node, kind) else {
            continue;
        };
        let found = counts[kind as usize];
        if !allowed.admits(found) {
            return Err(RowError::Count { kind, found, allowed });
        }
    }
    if let DocElement::Link(link) = parent {
        // Exactly two ends, counted above; together they must be the
        // set {0, 1}, in whatever order they come.
        if link.kind == LinkKind::Portal && ends != [true, true] {
            return Err(RowError::PortalEnds);
        }
    }
    Ok(())
}

/// [`validate_row`] for the element at `id` of `tree`: its own row, and
/// a node's connector rule from its place in the tree.
///
/// Cost: one pass over `id`'s children and one over the kinds; no
/// allocation.
pub fn validate_shape(tree: &Tree<DocElement>, id: NodeId) -> Result<(), ShapeError> {
    let Some(node) = tree.get(id) else {
        return Err(ShapeError::StaleNode(id));
    };
    let under_node = node
        .parent()
        .is_some_and(|p| tree.arena()[p].get().kind() == DocKind::Node);
    let children = id.children(tree.arena()).map(|c| tree.arena()[c].get());
    validate_row(node.get(), id == tree.root(), under_node, children)
        .map_err(|error| ShapeError::Row { element: id, error })
}
