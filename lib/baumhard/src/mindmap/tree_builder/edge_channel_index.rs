// SPDX-License-Identifier: MPL-2.0

//! [`EdgeChannelIndex`] — the channel → [`EdgeKey`] name table for a
//! canvas tree whose root's children are one per edge.
//!
//! Two roles have that shape. The connection-label tree is flat: one
//! `GlyphArea` leaf per labeled edge, directly under the root. The
//! connection tree is one level deeper: one `Void` per edge under the
//! root, holding that edge's cap and body glyphs. In both, the root's
//! child at 1-based channel `n` *is* the `n`-th edge of the element
//! slice the tree was built from, so one index names both. A hit or a
//! walk anywhere inside an edge's subtree climbs to that child and
//! reads its channel; the index supplies the `EdgeKey` behind it and
//! nothing else — the geometry stays in the tree.

use indextree::NodeId;

use crate::engine::element::TreeElement;
use crate::gfx_structs::tree::GfxTree;
use crate::mindmap::scene_cache::EdgeKey;

/// The `EdgeKey` of each per-edge child of a canvas tree's root, by
/// that child's 1-based channel. Built from the same element slice
/// as the tree (or its mutator), in the same order, so the two cannot
/// disagree about which edge owns which channel. Sibling of
/// [`PortalHitIndex`](super::portal::PortalHitIndex) for the deeper
/// portal shape.
///
/// # Costs
///
/// Construction is O(edges) with one `EdgeKey` clone each, made by
/// the caller. [`Self::resolve`] is O(depth) arena lookups — one for
/// a label leaf, two for a connection glyph — plus a slice index.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EdgeChannelIndex {
    edges: Vec<EdgeKey>,
}

impl EdgeChannelIndex {
    /// Name the edges in tree-insertion order: the first key owns
    /// channel 1. Callers pass the identity sequence of the slice the
    /// tree is built from — [`connection_label_identity_sequence`]
    /// for labels, each element's `edge_key` for connections.
    ///
    /// [`connection_label_identity_sequence`]: super::connection_label::connection_label_identity_sequence
    ///
    /// O(edges); one vector allocation.
    pub fn from_keys(keys: impl IntoIterator<Item = EdgeKey>) -> Self {
        EdgeChannelIndex {
            edges: keys.into_iter().collect(),
        }
    }

    /// Number of edges the index names.
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// Whether the index names no edges — i.e. nothing in its tree
    /// can resolve to an edge.
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// The edge owning the root child at 1-based `channel`, or `None`
    /// for channel 0 and for a channel past the end. O(1).
    pub fn key_for_channel(&self, channel: usize) -> Option<&EdgeKey> {
        self.edges.get(channel.checked_sub(1)?)
    }

    /// Name the edge an arena node of `tree` belongs to: climb from
    /// `node` to the root's child it sits under and look up that
    /// child's channel.
    ///
    /// `node` must come from the tree this index was built alongside.
    /// Returns `None` for the tree root, for a node not in the arena,
    /// and for a channel the index does not cover, so a mismatched
    /// tree/index pair degrades to "no edge" rather than naming the
    /// wrong one.
    ///
    /// # Costs
    ///
    /// One arena lookup per level climbed, one slice index, one
    /// `EdgeKey` clone on a hit. No allocation otherwise.
    pub fn resolve(&self, tree: &GfxTree, node: NodeId) -> Option<EdgeKey> {
        let edge_root = Self::edge_root(tree, node)?;
        let channel = tree.arena().get(edge_root)?.get().channel();
        self.key_for_channel(channel).cloned()
    }

    /// The root's child `node` sits under (itself, when it is one).
    /// `None` for the root and for a node outside the arena.
    fn edge_root(tree: &GfxTree, mut node: NodeId) -> Option<NodeId> {
        loop {
            let parent = tree.arena().get(node)?.parent()?;
            if parent == tree.root {
                return Some(node);
            }
            node = parent;
        }
    }
}
