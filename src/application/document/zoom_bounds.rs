// SPDX-License-Identifier: MPL-2.0

//! Document-level setters for the zoom-visibility pair
//! (`min_zoom_to_render` / `max_zoom_to_render`) across every
//! authored site: `MindNode`, `MindEdge`, `EdgeLabelConfig`,
//! `PortalEndpointState`.
//!
//! The setters share a common posture: each takes a pair of
//! [`OptionEdit<f32>`] values (one for `min`, one for `max`), so
//! the console — which can receive `zoom min=1.5`,
//! `zoom max=unset`, or both together — maps cleanly onto one
//! atomic call. Each returns `true` when the value actually
//! changed so callers can report "no-op" vs. "changed" without
//! re-reading the model.
//!
//! Validation mirrors the verifier + `ZoomVisibility::try_new`:
//! non-finite bounds and inverted (`min > max`) pairs are
//! rejected as a no-op with `false`. Interactive paths must not
//! panic (`CODE_CONVENTIONS.md` §9), so these setters log a
//! warning and return `false` rather than raising.

use log::warn;

use super::nodes::{validate_zoom_pair, OptionEdit};
use super::{EdgeRef, MindMapDocument};

impl MindMapDocument {
    /// Write the edge's top-level zoom-visibility window. The
    /// full edge is snapshotted into the undo stack via
    /// [`super::UndoAction::EditEdge`]. Returns `true` when
    /// either side changed. Rejects non-finite or inverted pairs
    /// as a no-op. See [`super::OptionEdit<f32>`] for per-side
    /// semantics.
    pub fn set_edge_zoom_visibility(
        &mut self,
        edge_ref: &EdgeRef,
        min: OptionEdit<f32>,
        max: OptionEdit<f32>,
    ) -> bool {
        self.mutate_edge(edge_ref, |edge, _canvas| {
            let new_min = min.apply(edge.min_zoom_to_render);
            let new_max = max.apply(edge.max_zoom_to_render);
            if !validate_zoom_pair(new_min, new_max) {
                warn!(
                    "set_edge_zoom_visibility: rejected invalid pair min={:?} max={:?}",
                    new_min, new_max
                );
                return false;
            }
            if new_min == edge.min_zoom_to_render && new_max == edge.max_zoom_to_render {
                return false;
            }
            edge.min_zoom_to_render = new_min;
            edge.max_zoom_to_render = new_max;
            true
        })
    }

    /// Write the edge's label-level zoom-visibility window. Forks
    /// a fresh `EdgeLabelConfig` on the edge if one wasn't already
    /// present — mirrors how
    /// [`MindMapDocument::set_edge_label_position`] and sibling
    /// label setters handle the config's lazy allocation. Returns
    /// `true` when either side changed.
    pub fn set_edge_label_zoom_visibility(
        &mut self,
        edge_ref: &EdgeRef,
        min: OptionEdit<f32>,
        max: OptionEdit<f32>,
    ) -> bool {
        self.mutate_edge(edge_ref, |edge, _canvas| {
            let (cur_min, cur_max) = edge
                .label_config
                .as_ref()
                .map(|c| (c.min_zoom_to_render, c.max_zoom_to_render))
                .unwrap_or((None, None));
            let new_min = min.apply(cur_min);
            let new_max = max.apply(cur_max);
            if !validate_zoom_pair(new_min, new_max) {
                warn!(
                    "set_edge_label_zoom_visibility: rejected invalid pair min={:?} max={:?}",
                    new_min, new_max
                );
                return false;
            }
            if new_min == cur_min && new_max == cur_max {
                return false;
            }
            let cfg = edge.label_config.get_or_insert_with(Default::default);
            cfg.min_zoom_to_render = new_min;
            cfg.max_zoom_to_render = new_max;
            true
        })
    }

    /// Write a portal endpoint's zoom-visibility window. Forks a
    /// fresh `PortalEndpointState` on the owning edge if one
    /// wasn't already present. `endpoint_node_id` must equal
    /// either `edge.from_id` (writes `portal_from`) or
    /// `edge.to_id` (writes `portal_to`); any other value returns
    /// `false`.
    pub fn set_portal_endpoint_zoom_visibility(
        &mut self,
        edge_ref: &EdgeRef,
        endpoint_node_id: &str,
        min: OptionEdit<f32>,
        max: OptionEdit<f32>,
    ) -> bool {
        self.mutate_edge(edge_ref, |edge, _canvas| {
            let is_from = endpoint_node_id == edge.from_id;
            let is_to = endpoint_node_id == edge.to_id;
            if !is_from && !is_to {
                return false;
            }
            let slot = if is_from {
                &mut edge.portal_from
            } else {
                &mut edge.portal_to
            };
            let (cur_min, cur_max) = slot
                .as_ref()
                .map(|s| (s.min_zoom_to_render, s.max_zoom_to_render))
                .unwrap_or((None, None));
            let new_min = min.apply(cur_min);
            let new_max = max.apply(cur_max);
            if !validate_zoom_pair(new_min, new_max) {
                warn!(
                    "set_portal_endpoint_zoom_visibility: rejected invalid pair min={:?} max={:?}",
                    new_min, new_max
                );
                return false;
            }
            if new_min == cur_min && new_max == cur_max {
                return false;
            }
            let state = slot.get_or_insert_with(Default::default);
            state.min_zoom_to_render = new_min;
            state.max_zoom_to_render = new_max;
            true
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::{
        doc_with_one_edge as doc_with_edge, doc_with_one_orphan_node as doc_with_node,
    };

    #[test]
    fn option_edit_apply_is_keep_clear_set() {
        assert_eq!(OptionEdit::<f32>::Keep.apply(Some(1.0)), Some(1.0));
        assert_eq!(OptionEdit::<f32>::Keep.apply(None), None);
        assert_eq!(OptionEdit::<f32>::Clear.apply(Some(1.0)), None);
        assert_eq!(OptionEdit::Set(2.0_f32).apply(Some(1.0)), Some(2.0));
        assert_eq!(OptionEdit::Set(2.0_f32).apply(None), Some(2.0));
    }

    #[test]
    fn set_node_zoom_sets_both_bounds() {
        let mut doc = doc_with_node();
        let changed = doc.set_node_zoom_visibility("0", OptionEdit::Set(0.5), OptionEdit::Set(2.0));
        assert!(changed);
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert_eq!(node.min_zoom_to_render, Some(0.5));
        assert_eq!(node.max_zoom_to_render, Some(2.0));
    }

    #[test]
    fn set_node_zoom_keep_leaves_value_untouched() {
        let mut doc = doc_with_node();
        // Seed with a bound then only edit the other side.
        doc.set_node_zoom_visibility("0", OptionEdit::Set(0.5), OptionEdit::Set(2.0));
        let changed = doc.set_node_zoom_visibility("0", OptionEdit::Keep, OptionEdit::Set(3.0));
        assert!(changed);
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert_eq!(node.min_zoom_to_render, Some(0.5));
        assert_eq!(node.max_zoom_to_render, Some(3.0));
    }

    #[test]
    fn set_node_zoom_clear_sets_to_none() {
        let mut doc = doc_with_node();
        doc.set_node_zoom_visibility("0", OptionEdit::Set(0.5), OptionEdit::Set(2.0));
        let changed = doc.set_node_zoom_visibility("0", OptionEdit::Clear, OptionEdit::Clear);
        assert!(changed);
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert!(node.min_zoom_to_render.is_none());
        assert!(node.max_zoom_to_render.is_none());
    }

    #[test]
    fn set_node_zoom_rejects_inverted_pair() {
        let mut doc = doc_with_node();
        let changed = doc.set_node_zoom_visibility("0", OptionEdit::Set(3.0), OptionEdit::Set(1.0));
        assert!(!changed, "inverted pair must be rejected as no-op");
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert!(node.min_zoom_to_render.is_none());
        assert!(node.max_zoom_to_render.is_none());
    }

    #[test]
    fn set_node_zoom_rejects_non_finite() {
        let mut doc = doc_with_node();
        assert!(!doc.set_node_zoom_visibility("0", OptionEdit::Set(f32::NAN), OptionEdit::Keep,));
        assert!(!doc.set_node_zoom_visibility("0", OptionEdit::Keep, OptionEdit::Set(f32::INFINITY),));
    }

    #[test]
    fn set_node_zoom_undo_restores_previous_pair() {
        let mut doc = doc_with_node();
        doc.set_node_zoom_visibility("0", OptionEdit::Set(0.5), OptionEdit::Set(2.0));
        doc.set_node_zoom_visibility("0", OptionEdit::Set(1.0), OptionEdit::Set(3.0));
        assert!(doc.undo());
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert_eq!(node.min_zoom_to_render, Some(0.5));
        assert_eq!(node.max_zoom_to_render, Some(2.0));
    }

    #[test]
    fn set_edge_zoom_round_trips_through_undo() {
        let (mut doc, er) = doc_with_edge();
        doc.set_edge_zoom_visibility(&er, OptionEdit::Set(0.5), OptionEdit::Set(2.0));
        let before_idx = doc.mindmap.edges.iter().position(|e| er.matches(e)).unwrap();
        assert_eq!(doc.mindmap.edges[before_idx].min_zoom_to_render, Some(0.5));
        assert_eq!(doc.mindmap.edges[before_idx].max_zoom_to_render, Some(2.0));
        assert!(doc.undo());
        assert!(doc.mindmap.edges[before_idx].min_zoom_to_render.is_none());
        assert!(doc.mindmap.edges[before_idx].max_zoom_to_render.is_none());
    }

    #[test]
    fn set_edge_label_zoom_forks_label_config() {
        let (mut doc, er) = doc_with_edge();
        assert!(doc.mindmap.edges[0].label_config.is_none());
        let changed = doc.set_edge_label_zoom_visibility(&er, OptionEdit::Set(1.5), OptionEdit::Keep);
        assert!(changed);
        let cfg = doc.mindmap.edges[0]
            .label_config
            .as_ref()
            .expect("label_config forked");
        assert_eq!(cfg.min_zoom_to_render, Some(1.5));
        assert!(cfg.max_zoom_to_render.is_none());
    }

    #[test]
    fn set_portal_endpoint_zoom_routes_by_endpoint_node_id() {
        let (mut doc, er) = doc_with_edge();
        assert!(doc.set_portal_endpoint_zoom_visibility(
            &er,
            "0",
            OptionEdit::Set(1.0),
            OptionEdit::Set(4.0),
        ));
        let e = &doc.mindmap.edges[0];
        assert!(e.portal_from.as_ref().is_some());
        assert!(e.portal_to.is_none(), "other endpoint untouched");
        let from = e.portal_from.as_ref().unwrap();
        assert_eq!(from.min_zoom_to_render, Some(1.0));
        assert_eq!(from.max_zoom_to_render, Some(4.0));
    }

    #[test]
    fn set_portal_endpoint_zoom_rejects_stranger_node_id() {
        let (mut doc, er) = doc_with_edge();
        assert!(!doc.set_portal_endpoint_zoom_visibility(
            &er,
            "not_an_endpoint",
            OptionEdit::Set(1.0),
            OptionEdit::Keep,
        ));
    }

    #[test]
    fn setters_short_circuit_noop_when_all_keep() {
        let mut doc = doc_with_node();
        let changed = doc.set_node_zoom_visibility("0", OptionEdit::Keep, OptionEdit::Keep);
        assert!(!changed);
        assert!(doc.undo_stack.is_empty(), "no-op must not push undo");
    }
}
