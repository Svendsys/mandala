// SPDX-License-Identifier: MPL-2.0

//! Phase 4's structure (design B.14 steps 14–31): where a node of the
//! flat list goes in the tree, checked in O(1), and the close that
//! checks a node's row once all of its children are in. Then phase 5,
//! the links.
//!
//! **The list is the tree's depth-first pre-order.** A node comes right
//! after its parent if it is the first child, else right after the last
//! node of its previous sibling's subtree; a top-level node follows the
//! last node of the previous one's. So a node's `parent` is always on
//! the **chain** of the node listed before it — that node or one of its
//! ancestors — and the loader checks exactly that. Since a parent must
//! already be on the chain, no cycle, orphan or second parent can be
//! written.
//!
//! **The check is O(1).** The chain is the path from a top-level node to
//! the node listed last, `chain[d]` the node at depth `d` on it, and
//! every placed node remembers its depth. A node's parent `p` is on the
//! chain exactly when `chain[depth(p)]` is `p`: a node leaves the chain
//! once and never returns. `chain[depth]`, before the chain is cut back,
//! is the previous sibling (or the previous top-level node), whose
//! channel the new node's may not be below.
//!
//! **Nothing recurses on depth.** The chain is a heap `Vec` as long as
//! the tree is deep, and closing is a loop. Each node is closed once, so
//! all of the closing together is O(nodes).

use crate::document::bounds::{self, BoundError};
use crate::document::format::dto::NodeDto;
use crate::document::format::error::{render_path, LoadError, LoadErrorKind, Seg, CHAIN_SHOWN};
use crate::document::format::from_dto::{Builder, ChainEntry, LoadResult, PlacedNode};

impl Builder {
    /// Place the node `dto`, listed at `index`, in the tree, and build
    /// what it holds. Its own row is checked later, when it leaves the
    /// chain ([`Self::close`]). O(the node's content), plus one hash
    /// lookup per id and per `parent`.
    pub(super) fn node(&mut self, index: usize, dto: &NodeDto) -> LoadResult<()> {
        self.path.clear();
        self.path.extend([Seg::Key("nodes"), Seg::Index(index)]);
        self.current = Some((index, None));
        let result = self.place(index, dto);
        self.current = None;
        self.path.clear();
        result
    }

    /// [`Self::node`], with the path and the current node set.
    fn place(&mut self, index: usize, dto: &NodeDto) -> LoadResult<()> {
        // Step 14: the id, then (15) whether another node has it.
        let id = self.bounded("id", bounds::check_node_ident(dto.id))?;
        self.current = Some((index, Some(id)));
        if let Some(first) = self.idents.get(&id) {
            let first_index = first.index;
            return Err(self.error_at("id", LoadErrorKind::DuplicateId { id, first_index }));
        }
        // Step 16.
        let channel = self.channel(dto.channel)?;

        // Steps 17–24: the parent, which must be on the chain.
        let (depth, under) = match dto.parent {
            None => (0, self.tree.root()),
            Some(parent) => {
                let parent = self.bounded("parent", bounds::check_node_ident(parent))?;
                if parent == id {
                    return Err(self.error_at("parent", LoadErrorKind::ParentIsSelf { id }));
                }
                let Some(placed) = self.idents.get(&parent).copied() else {
                    return Err(self.error_at("parent", LoadErrorKind::ParentNotEarlier { parent }));
                };
                if self.chain.get(placed.depth).map(|entry| entry.node) != Some(placed.node) {
                    return Err(self.error_at("parent", self.not_on_chain(parent, placed)));
                }
                (placed.depth + 1, placed.node)
            }
        };

        // Steps 25–26: `chain[depth]` is the previous sibling, before the
        // chain is cut back to the new node's parent.
        if let Some(previous) = self.chain.get(depth).copied() {
            if channel < previous.channel {
                return Err(self.error_at(
                    "channel",
                    LoadErrorKind::SiblingsOutOfOrder {
                        previous_channel: u64::try_from(previous.channel).unwrap_or(u64::MAX),
                        previous_id: Some(previous.id),
                    },
                ));
            }
        }
        // Step 27: every node deeper than the parent leaves the chain;
        // its subtree is complete.
        self.close(depth)?;
        // Step 28.
        if depth == 0 && dto.connector.is_some() {
            return Err(self.error_at("connector", LoadErrorKind::ConnectorOnRoot));
        }

        // Steps 29–30: the node, then what it holds.
        let node = self.node_element(under, dto, id, channel)?;
        self.node_row(node, dto, depth == 0, (index, id))?;

        // Step 31.
        self.idents.insert(id, PlacedNode { node, depth, index });
        self.highest = self.highest.max(id);
        self.chain.push(ChainEntry {
            node,
            id,
            channel,
            index,
        });
        Ok(())
    }

    /// The refusal for a `parent` that came earlier but whose subtree has
    /// ended: where it is, and the ids a node here could name — the node
    /// listed last, then its ancestors, nearest first.
    fn not_on_chain(&self, parent: u32, placed: PlacedNode) -> LoadErrorKind {
        let expected = self
            .chain
            .iter()
            .rev()
            .take(CHAIN_SHOWN)
            .map(|entry| entry.id)
            .collect();
        LoadErrorKind::ParentNotOnChain {
            parent,
            parent_index: placed.index,
            expected,
            truncated: self.chain.len() > CHAIN_SHOWN,
        }
    }

    /// Close the chain down to `depth` entries: every node deeper leaves
    /// it, and since its subtree is complete, its row is checked against
    /// the shape rules. `close(0)` at the end of the list closes the
    /// rest. A row that fails is a defect in the loader, which checks
    /// what the shape rules check before it builds; the error names the
    /// node it is about.
    pub(super) fn close(&mut self, depth: usize) -> LoadResult<()> {
        while self.chain.len() > depth {
            let Some(entry) = self.chain.pop() else {
                break;
            };
            self.checked += 1;
            if let Err(error) = crate::document::shape::validate_shape(&self.tree, entry.node) {
                let path = render_path(&[Seg::Key("nodes"), Seg::Index(entry.index)]);
                return Err(LoadError {
                    node: Some(self.context_of(entry.index, entry.id)),
                    ..LoadError::new(
                        path,
                        LoadErrorKind::Internal {
                            message: error.to_string(),
                        },
                    )
                });
            }
        }
        Ok(())
    }

    /// The ident a link's `to` names, checked against the bound.
    pub(super) fn link_target(&self, to: u64) -> LoadResult<u32> {
        bounds::check_node_ident(to)
            .map_err(|error: BoundError| self.error_at("to", LoadErrorKind::Bound { error }))
    }

    /// Phase 5: resolve every recorded link's `to`, in the order the
    /// links were met, now that every node has an arena id. O(links).
    pub(super) fn resolve_links(&mut self) -> LoadResult<()> {
        let links = std::mem::take(&mut self.links);
        for link in links {
            let target = self.idents.get(&link.to).map(|placed| placed.node);
            let (owner_index, owner_id) = link.owner_at;
            let fail = |builder: &Builder, kind| LoadError {
                node: Some(builder.context_of(owner_index, owner_id)),
                ..LoadError::new(render_path(&link.to_path()), kind)
            };
            let Some(target) = target else {
                return Err(fail(self, LoadErrorKind::DanglingLink { to: link.to }));
            };
            if target == link.owner {
                return Err(fail(self, LoadErrorKind::SelfLink));
            }
            let updated = self.tree.update_known(link.link, |element| {
                if let crate::document::element::DocElement::Link(data) = element {
                    data.to = target;
                }
            });
            if let Err(error) = updated {
                return Err(fail(
                    self,
                    LoadErrorKind::Internal {
                        message: error.to_string(),
                    },
                ));
            }
        }
        Ok(())
    }
}
