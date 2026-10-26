// SPDX-License-Identifier: MPL-2.0

//! Topology mutations — creating / deleting / moving / reparenting
//! nodes. Everything that reshapes which nodes exist, where they
//! sit, and who their parent is. Also carries `delete_nodes` —
//! the set-of-nodes remove that also rips touching edges.

use std::collections::HashSet;

use glam::Vec2;

use baumhard::mindmap::model::{dewey_cmp, MindEdge, MindNode, Position};

use super::defaults::{default_cross_link_edge, default_orphan_node, default_parent_child_edge};
use super::types::{EdgeRef, ReparentUndoData, SelectionState};
use super::undo_action::{OrphanedChild, UndoAction};
use super::MindMapDocument;

impl MindMapDocument {
    /// Remove a set of nodes from the map as one operation, re-rooting
    /// every child of a removed node that is not itself removed (it
    /// becomes a root under a fresh Dewey id, its own id-prefix
    /// descendants cascading with it) and removing every edge that
    /// touched a removed node. Returns one [`UndoAction::DeleteNodes`]
    /// that restores all of it in one step, or `None` when no id in
    /// `node_ids` names a live node — then nothing was changed.
    ///
    /// The set is resolved first — duplicates dropped, missing ids
    /// ignored — and every member is removed before any child is
    /// re-rooted. That is what makes the result a function of the set
    /// rather than of the order the ids arrive in: deleting a parent
    /// and its child together removes both, and the child's own
    /// children are re-rooted exactly as they would be by deleting the
    /// child alone. Looping a single-node delete over the ids cannot
    /// do this (#172): the first removal renames the second node's id
    /// out from under it. "Delete parent and child" therefore means
    /// *the set*, not the subtree — a descendant that was not selected
    /// survives, re-rooted, as it does under a single delete.
    ///
    /// Costs: O(N) over the nodes to resolve the set and find the
    /// orphans, O(E) over the edges, plus one `cascade_rename` (itself
    /// O(N + E)) per orphan. Allocates the removed nodes and edges
    /// into the undo entry.
    pub fn delete_nodes(&mut self, node_ids: &[String]) -> Option<UndoAction> {
        // Resolve the set. `dewey_cmp` is a total order on ids, so the
        // undo entry and everything derived from it are the same
        // whichever order the selection listed the ids in.
        let mut victim_set: HashSet<&str> = HashSet::with_capacity(node_ids.len());
        let mut victims: Vec<String> = Vec::with_capacity(node_ids.len());
        for id in node_ids {
            if self.mindmap.nodes.contains_key(id) && victim_set.insert(id.as_str()) {
                victims.push(id.clone());
            }
        }
        if victims.is_empty() {
            return None;
        }
        victims.sort_by(|a, b| dewey_cmp(a, b));

        // Mint fresh root ids strictly above every live id's *leading*
        // Dewey segment — computed while every victim is still present
        // so their own segments count. A root at or above this value
        // has an entirely unused `id` / `id.` prefix, which buys two
        // guarantees at once:
        //   * no re-rooted child can ever be handed a victim's id (or a
        //     Dewey prefix of one) — the collision that let undo
        //     overwrite a live node and destroy data (issue #1); and
        //   * the cascade into that fresh prefix can never collide with
        //     an unrelated node, so `cascade_rename`'s guard never
        //     trips here.
        let first_root = self.next_free_root_segment();

        // The orphans: every surviving node whose parent is a victim.
        // Found by `parent_id` rather than by id prefix, because ids and
        // tree structure can diverge — `apply_reparent` /
        // `apply_orphan_selection` re-point `parent_id` without
        // re-keying — and the structure is the hierarchy. Sorted by the
        // full Dewey order so fresh roots are assigned the same way
        // regardless of `HashMap` iteration order (stable across runs,
        // platforms and saved output); `id_sort_key` reads the last
        // segment only and would tie two orphans from different
        // subtrees, leaving the tie to the map's hash seed.
        let mut orphaned_children: Vec<OrphanedChild> = self
            .mindmap
            .nodes
            .values()
            .filter(|n| !victim_set.contains(n.id.as_str()))
            .filter_map(|n| {
                let parent_id = n.parent_id.as_deref()?;
                victim_set.contains(parent_id).then(|| OrphanedChild {
                    original_id: n.id.clone(),
                    root_id: String::new(),
                    parent_id: parent_id.to_owned(),
                })
            })
            .collect();
        orphaned_children.sort_by(|a, b| dewey_cmp(&a.original_id, &b.original_id));
        for (root_segment, orphan) in (first_root..).zip(orphaned_children.iter_mut()) {
            orphan.root_id = root_segment.to_string();
        }

        // Remove every victim *before* any cascade. A structural child's
        // id may be a Dewey prefix of its parent's (child "1" whose
        // parent is "1.0"); cascading that child while the parent is
        // still keyed would sweep the parent up by prefix and strand its
        // removal. Removing the whole set first makes every cascade
        // blind to it. Regression: `tests_delete::test_delete_node_*prefix*`.
        let nodes: Vec<MindNode> = victims
            .iter()
            .map(|id| {
                self.mindmap
                    .nodes
                    .remove(id)
                    .expect("every victim was checked for presence while the set was resolved")
            })
            .collect();

        // Re-root the orphans, walking the list *backwards*. The fresh
        // ids were assigned in Dewey order, where a prefix sorts before
        // what it prefixes, so the reverse walk re-roots "1.0.2.5"
        // before "1.0.2": the cascade of the shorter id can then never
        // sweep a pending orphan along by prefix and leave it under a
        // name this loop no longer knows. Both can be orphans of one
        // delete only when ids and structure have diverged, which they
        // can (above). Undo walks the list forwards, reversing the
        // cascades in true LIFO order.
        for index in (0..orphaned_children.len()).rev() {
            let orphan = &orphaned_children[index];
            if !self.cascade_rename(&orphan.original_id, &orphan.root_id) {
                // Unreachable in normal operation: `first_root` sits
                // above every live leading segment, so the target prefix
                // is empty. If a future divergence ever trips the guard,
                // degrade without corrupting (CODE_CONVENTIONS §9):
                // reverse the orphans already re-rooted, put every
                // victim back, and report the delete as a no-op rather
                // than leave a dangling child.
                log::error!(
                    "delete_nodes: re-rooting child '{}' of '{}' to '{}' was refused; \
                     aborting delete to avoid corrupting the map",
                    orphan.original_id,
                    orphan.parent_id,
                    orphan.root_id
                );
                for done in &orphaned_children[index + 1..] {
                    self.cascade_rename(&done.root_id, &done.original_id);
                    if let Some(child) = self.mindmap.nodes.get_mut(&done.original_id) {
                        child.parent_id = Some(done.parent_id.clone());
                    }
                }
                for node in nodes {
                    self.mindmap.nodes.insert(node.id.clone(), node);
                }
                return None;
            }
            if let Some(child) = self.mindmap.nodes.get_mut(&orphan.root_id) {
                child.parent_id = None;
            }
        }

        // Collect every edge that touches a victim, with its
        // pre-removal index so undo re-inserts each where it was.
        let removed_edges: Vec<(usize, MindEdge)> = self
            .mindmap
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| victim_set.contains(e.from_id.as_str()) || victim_set.contains(e.to_id.as_str()))
            .map(|(i, e)| (i, e.clone()))
            .collect();
        self.mindmap
            .edges
            .retain(|e| !victim_set.contains(e.from_id.as_str()) && !victim_set.contains(e.to_id.as_str()));

        self.mark_dirty();
        Some(UndoAction::DeleteNodes {
            nodes,
            removed_edges,
            orphaned_children,
        })
    }

    /// Rename a node and all its descendants from `old_id` to `new_id`,
    /// updating the node's `id` field, `parent_id` of descendants,
    /// and all edge/portal references.
    ///
    /// Returns `true` when the rename was applied, `false` when it was
    /// refused because a target id is already occupied by an unrelated node
    /// (in which case nothing is mutated). Callers that must stay consistent
    /// on refusal — `delete_nodes` — check the result and roll back.
    pub(super) fn cascade_rename(&mut self, old_id: &str, new_id: &str) -> bool {
        // Collect the full old→new mapping: the node itself + all descendants.
        let old_prefix = format!("{}.", old_id);
        let renames: Vec<(String, String)> = self
            .mindmap
            .nodes
            .keys()
            .filter(|k| *k == old_id || k.starts_with(&old_prefix))
            .map(|k| {
                let new_k = if k == old_id {
                    new_id.to_string()
                } else {
                    format!("{}{}", new_id, &k[old_id.len()..])
                };
                (k.clone(), new_k)
            })
            .collect();

        // old → new lookup, built once up front and shared by both the
        // collision guard (its keys are exactly the old ids being vacated)
        // and the parent_id / edge rewrites below. Replaces the former inner
        // `for (ro, rn) in &renames` linear scans (CODE_CONVENTIONS §5):
        // O(renames) + O(edges) instead of O(renames²) + O(edges·renames).
        let rename_map: std::collections::HashMap<&str, &str> =
            renames.iter().map(|(o, n)| (o.as_str(), n.as_str())).collect();

        // Defense in depth (CODE_CONVENTIONS §9: degrade, don't corrupt).
        // A target id already occupied by a node *outside* this rename set
        // (i.e. not one of the old ids we're vacating) means the caller
        // minted a colliding id — the historical delete+undo corruption.
        // Refuse the whole rename rather than let the `nodes.insert(new, ..)`
        // below silently clobber a live node. With the `delete_nodes` fix this
        // never fires; it is a backstop against any future bad caller.
        for (_, new) in &renames {
            if !rename_map.contains_key(new.as_str()) && self.mindmap.nodes.contains_key(new) {
                log::error!(
                    "cascade_rename({old_id} -> {new_id}): target id '{new}' is already \
                     occupied by an unrelated node; aborting rename to avoid corrupting the map"
                );
                return false;
            }
        }

        // Rename nodes in the HashMap.
        for (old, new) in &renames {
            if let Some(mut node) = self.mindmap.nodes.remove(old) {
                node.id = new.clone();
                // Update parent_id if it references another renamed node.
                if let Some(pid) = node.parent_id.as_deref() {
                    if let Some(new_pid) = rename_map.get(pid) {
                        node.parent_id = Some((*new_pid).to_string());
                    }
                }
                self.mindmap.nodes.insert(new.clone(), node);
            }
        }

        // Update edge references.
        for edge in &mut self.mindmap.edges {
            if let Some(new) = rename_map.get(edge.from_id.as_str()) {
                edge.from_id = (*new).to_string();
            }
            if let Some(new) = rename_map.get(edge.to_id.as_str()) {
                edge.to_id = (*new).to_string();
            }
        }
        true
    }

    /// One past the highest *leading* Dewey segment across every node id in
    /// the map (`0` for an empty map). A root minted at or above this value
    /// has an entirely unused `id` / `id.` prefix, so re-rooting a subtree
    /// there can neither reuse a live id nor collide with one.
    ///
    /// `delete_nodes` uses it to re-root orphans without ever reusing (a
    /// prefix of) an id it is about to delete. For a structurally-consistent
    /// map this equals `fresh_child_id(None)`; the two diverge only when an
    /// intermediate root has been deleted, leaving dotted ids whose leading
    /// segment no longer has a matching root.
    fn next_free_root_segment(&self) -> usize {
        self.mindmap
            .nodes
            .keys()
            .filter_map(|id| id.split('.').next().and_then(|seg| seg.parse::<usize>().ok()))
            .max()
            .map_or(0, |m| m + 1)
    }

    /// Create a new unattached (orphan) node at the given canvas position.
    /// Returns the new node's Dewey-decimal id.
    pub fn apply_create_orphan_node(&mut self, position: Vec2) -> String {
        let id = self.fresh_child_id(None);
        let node = default_orphan_node(&id, position);
        self.mindmap.nodes.insert(id.clone(), node);
        id
    }

    /// Detach each node in `node_ids` from its parent, promoting it to root.
    pub fn apply_orphan_selection(&mut self, node_ids: &[String]) -> ReparentUndoData {
        self.apply_reparent(node_ids, None)
    }

    /// Create an orphan node, push undo, select it, mark dirty.
    pub fn create_orphan_and_select(&mut self, canvas_pos: Vec2) -> String {
        let new_id = self.apply_create_orphan_node(canvas_pos);
        self.undo_stack.push(UndoAction::CreateNode {
            node_id: new_id.clone(),
        });
        self.selection = SelectionState::Single(new_id.clone());
        self.mark_dirty();
        new_id
    }

    /// Orphan every selected node with undo support.
    pub fn apply_orphan_selection_with_undo(&mut self) -> bool {
        let sel: Vec<String> = self
            .selection
            .selected_ids()
            .iter()
            .map(|s| s.to_string())
            .collect();
        if sel.is_empty() {
            return false;
        }
        let undo_data = self.apply_orphan_selection(&sel);
        if undo_data.entries.is_empty() {
            return false;
        }
        self.undo_stack.push(UndoAction::ReparentNodes {
            entries: undo_data.entries,
            old_edges: undo_data.old_edges,
        });
        self.mark_dirty();
        true
    }

    /// Delete whatever is currently selected. Edge deletion covers
    /// both line-mode and portal-mode edges — they're the same entity
    /// under `display_mode`, so one `DeleteEdge` undo variant handles
    /// both forms.
    pub fn apply_delete_selection(&mut self) -> bool {
        enum DelKind {
            Edge(EdgeRef),
            /// One or many — `delete_nodes` takes the set, so a single
            /// node and a multi-selection are the same arm.
            Nodes(Vec<String>),
        }
        let kind = match &self.selection {
            SelectionState::Edge(e) => Some(DelKind::Edge(e.clone())),
            SelectionState::Single(id) => Some(DelKind::Nodes(vec![id.clone()])),
            // Delete on a section-selection deletes the *whole
            // owning node*. Per-section deletion (a future verb
            // like `section delete`) is the dedicated entry
            // point; the bare Delete keystroke targets the node.
            SelectionState::Section(s) => Some(DelKind::Nodes(vec![s.node_id.clone()])),
            // SectionRange: same shape as Section — the bare
            // Delete keystroke targets the owning node, not
            // the sub-range (range deletion would be a
            // dedicated verb).
            SelectionState::SectionRange { sel, .. } => Some(DelKind::Nodes(vec![sel.node_id.clone()])),
            // MultiSection delete targets the deduplicated set
            // of owning nodes — routes through the shared
            // `dedup_owning_node_ids` helper.
            SelectionState::MultiSection(_) => Some(DelKind::Nodes(self.selection.dedup_owning_node_ids())),
            SelectionState::Multi(ids) => Some(DelKind::Nodes(ids.clone())),
            // Delete on any edge-sub-part selection (label, icon,
            // text) targets the owning edge — the sub-parts are
            // render forms of the same edge, and deleting one
            // would leave a dangling structure. Consistent with
            // the user's mental model: "delete this portal" /
            // "delete this label" = "delete the edge that made it".
            SelectionState::EdgeLabel(s) => Some(DelKind::Edge(s.edge_ref.clone())),
            SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => {
                Some(DelKind::Edge(s.edge_ref()))
            }
            SelectionState::None => None,
        };
        match kind {
            Some(DelKind::Edge(edge_ref)) => {
                if let Some((index, edge)) = self.remove_edge(&edge_ref) {
                    self.undo_stack.push(UndoAction::DeleteEdge { index, edge });
                    self.selection = SelectionState::None;
                    self.mark_dirty();
                    return true;
                }
            }
            // The whole set in one `delete_nodes` call and one undo
            // entry: one keystroke, one `Undo` (#172).
            Some(DelKind::Nodes(ids)) => {
                if let Some(undo) = self.delete_nodes(&ids) {
                    self.undo_stack.push(undo);
                    self.selection = SelectionState::None;
                    return true;
                }
            }
            None => {}
        }
        false
    }

    /// Generate the next available Dewey-decimal child ID under `parent`.
    /// `None` = root level. E.g. if roots "0", "1", "2" exist, returns "3".
    /// If parent "1" has children "1.0", "1.2", returns "1.3".
    pub(super) fn fresh_child_id(&self, parent: Option<&str>) -> String {
        let prefix = match parent {
            Some(p) => format!("{}.", p),
            None => String::new(),
        };
        let max_segment: Option<usize> = self
            .mindmap
            .nodes
            .keys()
            .filter_map(|id| {
                let suffix = id.strip_prefix(&prefix)?;
                // Only direct children — no further dots
                if suffix.contains('.') {
                    return None;
                }
                suffix.parse::<usize>().ok()
            })
            .max();
        let next = max_segment.map(|m| m + 1).unwrap_or(0);
        format!("{}{}", prefix, next)
    }

    /// Create a default-styled cross_link edge between two nodes.
    // Native-driver-only, with the four below: edge creation and
    // node moves are reached from native-gated click / drag
    // handlers and console verbs.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn create_cross_link_edge(&mut self, source_id: &str, target_id: &str) -> Option<usize> {
        if source_id == target_id {
            return None;
        }
        if !self.mindmap.nodes.contains_key(source_id) || !self.mindmap.nodes.contains_key(target_id) {
            return None;
        }
        let exists = self
            .mindmap
            .edges
            .iter()
            .any(|e| e.edge_type == "cross_link" && e.from_id == source_id && e.to_id == target_id);
        if exists {
            return None;
        }
        let edge = default_cross_link_edge(source_id, target_id);
        self.mindmap.edges.push(edge);
        Some(self.mindmap.edges.len() - 1)
    }

    /// Apply a position delta to a node and all its descendants.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn apply_move_subtree(&mut self, node_id: &str, dx: f64, dy: f64) -> Vec<(String, Position)> {
        let mut ids = vec![node_id.to_string()];
        ids.extend(self.mindmap.all_descendants(node_id));
        let mut original_positions = Vec::with_capacity(ids.len());
        for id in &ids {
            if let Some(node) = self.mindmap.nodes.get_mut(id) {
                original_positions.push((id.clone(), node.position));
                node.offset_position_clamped(dx, dy);
            }
        }
        original_positions
    }

    /// Apply a position delta to a single node only.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn apply_move_single(&mut self, node_id: &str, dx: f64, dy: f64) -> Option<(String, Position)> {
        if let Some(node) = self.mindmap.nodes.get_mut(node_id) {
            let original = (node_id.to_string(), node.position);
            node.offset_position_clamped(dx, dy);
            Some(original)
        } else {
            None
        }
    }

    /// Move multiple root nodes at once, with subtree deduplication.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn apply_move_multiple(
        &mut self,
        node_ids: &[String],
        dx: f64,
        dy: f64,
        individual: bool,
    ) -> Vec<(String, Position)> {
        if individual {
            let mut undo_data = Vec::new();
            for nid in node_ids {
                undo_data.extend(self.apply_move_single(nid, dx, dy));
            }
            return undo_data;
        }
        let roots = self.dedup_subtree_roots(node_ids);
        let mut undo_data = Vec::new();
        for nid in &roots {
            undo_data.extend(self.apply_move_subtree(nid, dx, dy));
        }
        undo_data
    }

    /// Filter a list of node IDs to only the "roots" — nodes not
    /// descendant of any other node in the list.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) fn dedup_subtree_roots(&self, node_ids: &[String]) -> Vec<String> {
        let id_set: HashSet<&str> = node_ids.iter().map(|s| s.as_str()).collect();
        node_ids
            .iter()
            .filter(|id| {
                let mut current = self
                    .mindmap
                    .nodes
                    .get(id.as_str())
                    .and_then(|n| n.parent_id.as_deref());
                while let Some(pid) = current {
                    if id_set.contains(pid) {
                        return false;
                    }
                    current = self.mindmap.nodes.get(pid).and_then(|n| n.parent_id.as_deref());
                }
                true
            })
            .cloned()
            .collect()
    }

    /// Reparent a set of nodes under `new_parent_id` (None = promote to root).
    /// Updates `parent_id` and parent_child edges. Returns undo data.
    pub fn apply_reparent(&mut self, node_ids: &[String], new_parent_id: Option<&str>) -> ReparentUndoData {
        let old_edges = self.mindmap.edges.clone();

        let mut entries: Vec<(String, Option<String>)> = Vec::new();
        for source_id in node_ids {
            if !self.mindmap.nodes.contains_key(source_id) {
                continue;
            }
            if let Some(target) = new_parent_id {
                if self.mindmap.is_ancestor_or_self(source_id, target) {
                    continue;
                }
            }
            let node = match self.mindmap.nodes.get_mut(source_id) {
                Some(n) => n,
                None => continue,
            };
            entries.push((source_id.clone(), node.parent_id.clone()));
            node.parent_id = new_parent_id.map(|s| s.to_string());

            // Update parent_child edges
            let old_edge_pos = self
                .mindmap
                .edges
                .iter()
                .position(|e| e.edge_type == "parent_child" && e.to_id == *source_id);
            match (old_edge_pos, new_parent_id) {
                (Some(idx), Some(new_parent)) => {
                    self.mindmap.edges[idx].from_id = new_parent.to_string();
                    self.mindmap.edges[idx].control_points.clear();
                }
                (Some(idx), None) => {
                    self.mindmap.edges.remove(idx);
                }
                (None, Some(new_parent)) => {
                    self.mindmap
                        .edges
                        .push(default_parent_child_edge(new_parent, source_id));
                }
                (None, None) => {}
            }
        }
        ReparentUndoData { entries, old_edges }
    }
}
