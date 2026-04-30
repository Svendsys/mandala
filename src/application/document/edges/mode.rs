// SPDX-License-Identifier: MPL-2.0

//! Edge type / display-mode / style-reset — toggles on the edge structural shape.


use baumhard::mindmap::model::{
    is_portal_edge, DISPLAY_MODE_LINE, DISPLAY_MODE_PORTAL,
};

use super::super::types::{EdgeRef, SelectionState};
use super::super::undo_action::UndoAction;
use super::super::MindMapDocument;

impl MindMapDocument {
    /// Change the `edge_type` of an edge. Refuses the change (returns
    /// `false`) if it would create a duplicate `(from_id, to_id,
    /// new_type)` against another edge. On success updates
    /// `self.selection` to a fresh `EdgeRef` with the new type so the
    /// edge stays selected.
    pub fn set_edge_type(&mut self, edge_ref: &EdgeRef, new_type: &str) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        if self.mindmap.edges[idx].edge_type == new_type {
            return false;
        }
        // Duplicate guard: refuse if some OTHER edge already has the same
        // (from_id, to_id, new_type) triple.
        let from_id = self.mindmap.edges[idx].from_id.clone();
        let to_id = self.mindmap.edges[idx].to_id.clone();
        let duplicate = self.mindmap.edges.iter().enumerate().any(|(i, e)| {
            i != idx
                && e.from_id == from_id
                && e.to_id == to_id
                && e.edge_type == new_type
        });
        if duplicate {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].edge_type = new_type.to_string();
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        // Refresh the selection EdgeRef so the app keeps the edge selected
        // under its new identity.
        if let SelectionState::Edge(ref cur) = self.selection {
            if cur == edge_ref {
                self.selection = SelectionState::Edge(EdgeRef::new(
                    from_id,
                    to_id,
                    new_type,
                ));
            }
        }
        true
    }

    /// Clear `glyph_connection` on the edge, reverting it to the
    /// canvas-level default style. Returns `true` if the edge existed
    /// and had a per-edge override to clear.
    pub fn reset_edge_style_to_default(&mut self, edge_ref: &EdgeRef) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        if self.mindmap.edges[idx].glyph_connection.is_none() {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].glyph_connection = None;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Switch an edge's `display_mode` between `"line"` and `"portal"`.
    /// `None` / `"line"` → the usual path form; `"portal"` → two
    /// glyph markers above each endpoint, no line between. Unknown
    /// values are rejected with `false`. Returns `false` on no-op
    /// (value already matches, edge not found). Undoes via the
    /// standard `EditEdge { index, before }` path.
    pub fn set_edge_display_mode(&mut self, edge_ref: &EdgeRef, mode: &str) -> bool {
        if mode != DISPLAY_MODE_LINE && mode != DISPLAY_MODE_PORTAL {
            return false;
        }
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let current_is_portal = is_portal_edge(&self.mindmap.edges[idx]);
        let want_portal = mode == DISPLAY_MODE_PORTAL;
        if current_is_portal == want_portal {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].display_mode = if want_portal {
            Some(DISPLAY_MODE_PORTAL.to_string())
        } else {
            None
        };
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }
}
