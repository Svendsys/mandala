// SPDX-License-Identifier: MPL-2.0

//! `InteractionMode` — the high-level interaction mode for the application.
//!
//! Drives:
//! - which clicks are absorbed (Reparent / Connect intercept the next
//!   left-click; NodeEdit reroutes section clicks; Resize captures the
//!   next click as a resize gesture).
//! - which mode-gated chrome is rendered (resize anchors, section frames,
//!   mode-specific highlight colors).
//! - how a `SelectionState` click resolves (e.g. a click on a section-area
//!   in NodeEdit produces `SelectionState::Section`; in Default it folds
//!   to `SelectionState::Single`).
//!
//! Cross-platform — replaces the native-only `AppMode` that lived inline
//! in `app/mod.rs` pre-Batch-1 of `SECTIONS_BORDERS_RESIZE_PLAN.md`. The
//! enum carries no GPU handles; it depends only on owned `String` ids
//! and the value types defined here, so both targets compile it.
//!
//! `SectionEdit` is intentionally **not** a variant. Section-text editing
//! is carried by `TextEditState::Open { node_id, section_idx, .. }` in
//! the modal-stealer cascade. The invariant `TextEditState::Open` ⇒
//! `InteractionMode::NodeEdit { node_id }` (matching id) is enforced by
//! the dispatcher arms that open / close the editor; this module only
//! defines the mode shape.
//!
//! `NodeEdit` and `Resize` are defined here but their predicate bodies
//! are stubs in this batch — no caller wires them yet. Their full
//! behaviour lands in Batch 2 (Resize) and Batch 3 (NodeEdit visuals)
//! per the plan §8. `Default::resize_handle_*` returns `None` and the
//! corresponding callers (`document/mod.rs:520-523`) keep their
//! selection-driven gates in this batch — the gate flip is the single
//! line change in Batch 2.

/// What the user is doing right now at the canvas level.
///
/// One of these is always active. Transitions go through `Action`
/// dispatch — there is no path that flips a mode by side-effect.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum InteractionMode {
    /// Default canvas mode. Click selects, drag moves, no mode-gated chrome.
    Default,

    /// User is choosing a new parent for `sources`. The next left-click on
    /// a node attaches them; left-click on empty canvas promotes them to
    /// root; Esc cancels. Direct migration from `AppMode::Reparent`.
    Reparent {
        sources: Vec<String>,
    },

    /// User is drawing a new cross_link edge from `source`. The next
    /// left-click on a target node creates the edge; left-click on empty
    /// canvas cancels. Esc also cancels. Direct migration from
    /// `AppMode::Connect`.
    Connect {
        source: String,
    },

    /// Editing the contents of `node_id`. Section-area clicks select the
    /// hit section (per `SelectionState::Section`); section drags move
    /// the section relative to the node. Section-text editing is reached
    /// by opening `TextEditState::Open` on a section while in this mode.
    /// Out-of-AABB click exits to `Default`.
    ///
    /// Wired in Batch 3 of the plan.
    NodeEdit {
        node_id: String,
    },

    /// Resize anchors are visible on `target`. Anchor drag transitions
    /// `DragState` through the existing `Throttled(NodeResize)` /
    /// `Throttled(SectionResize)` gestures. Click on the body (not on
    /// an anchor) exits to `Default`. Esc also exits.
    ///
    /// Wired in Batch 2 of the plan.
    Resize {
        target: ResizeTarget,
    },
}

/// What a `Resize` mode targets — a whole node or one section of one node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResizeTarget {
    Node(String),
    Section { node_id: String, section_idx: usize },
}

impl Default for InteractionMode {
    fn default() -> Self {
        InteractionMode::Default
    }
}

impl InteractionMode {
    /// True when this mode wants to absorb the next left-click as a
    /// mode-specific gesture rather than letting the standard click
    /// router handle it.
    ///
    /// Reparent / Connect intercept; Resize will intercept once Batch 2
    /// wires it; NodeEdit / Default do not intercept — clicks fall
    /// through to the regular hit-test + selection update.
    pub fn intercepts_left_click(&self) -> bool {
        match self {
            InteractionMode::Reparent { .. } | InteractionMode::Connect { .. } => true,
            InteractionMode::Resize { .. } => false, // Wired in Batch 2.
            InteractionMode::NodeEdit { .. } | InteractionMode::Default => false,
        }
    }

    /// True when a click on a section-area should produce
    /// `SelectionState::Section { node_id, section_idx }` rather than
    /// folding to `SelectionState::Single(node_id)`.
    ///
    /// In `Default` mode, single-section nodes always fold via the
    /// `hit_test_target` rule; multi-section nodes fold here in
    /// `Default` and only break out to `Section` in `NodeEdit { node_id }`
    /// for the matching node.
    ///
    /// Wired in Batch 3.
    pub fn click_resolves_to_section(&self, hit_node: &str) -> bool {
        match self {
            InteractionMode::NodeEdit { node_id } => node_id == hit_node,
            _ => false,
        }
    }

    /// The node that should receive auto-emitted resize handles this
    /// frame, or `None` if no node should.
    ///
    /// Wired in Batch 2 — until then, the scene-builder gate at
    /// `document/mod.rs:520-523` continues to read selection directly,
    /// and this method is unused. Returns `None` for every mode in this
    /// batch (the selection-driven path is what's live).
    pub fn resize_handle_node(&self) -> Option<&str> {
        match self {
            InteractionMode::Resize {
                target: ResizeTarget::Node(id),
            } => Some(id.as_str()),
            _ => None,
        }
    }

    /// The section that should receive auto-emitted resize handles this
    /// frame, or `None` if no section should.
    ///
    /// Same wiring story as `resize_handle_node` — the call site in
    /// `document/mod.rs:520-523` flips to consume this in Batch 2.
    pub fn resize_handle_section(&self) -> Option<(&str, usize)> {
        match self {
            InteractionMode::Resize {
                target: ResizeTarget::Section { node_id, section_idx },
            } => Some((node_id.as_str(), *section_idx)),
            _ => None,
        }
    }

    /// True if this mode is `Reparent` or `Connect` — the two pre-existing
    /// modes that capture the next click as a "choose target" gesture.
    /// Used by handlers that previously matched on `AppMode::Reparent { .. } |
    /// AppMode::Connect { .. }` to update hover highlights.
    pub fn is_target_picker(&self) -> bool {
        matches!(self, InteractionMode::Reparent { .. } | InteractionMode::Connect { .. })
    }

    /// Resolve this mode into the `ResizeHandleOverrides` value the
    /// scene-builder consumes. Single source of truth for "should this
    /// frame emit handles and on what target?" — every scene-rebuild
    /// call site reads through this method rather than reaching for
    /// the two `resize_handle_*` predicates separately.
    pub fn resize_handle_overrides(
        &self,
    ) -> crate::application::document::ResizeHandleOverrides<'_> {
        crate::application::document::ResizeHandleOverrides {
            node: self.resize_handle_node(),
            section: self.resize_handle_section(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_default() {
        assert_eq!(InteractionMode::default(), InteractionMode::Default);
    }

    #[test]
    fn default_mode_does_not_intercept_or_pick_targets() {
        let m = InteractionMode::Default;
        assert!(!m.intercepts_left_click());
        assert!(!m.click_resolves_to_section("any"));
        assert_eq!(m.resize_handle_node(), None);
        assert_eq!(m.resize_handle_section(), None);
        assert!(!m.is_target_picker());
    }

    #[test]
    fn reparent_intercepts_and_is_target_picker() {
        let m = InteractionMode::Reparent {
            sources: vec!["0".into()],
        };
        assert!(m.intercepts_left_click());
        assert!(m.is_target_picker());
        assert!(!m.click_resolves_to_section("0"));
        assert_eq!(m.resize_handle_node(), None);
    }

    #[test]
    fn connect_intercepts_and_is_target_picker() {
        let m = InteractionMode::Connect { source: "0".into() };
        assert!(m.intercepts_left_click());
        assert!(m.is_target_picker());
        assert!(!m.click_resolves_to_section("0"));
        assert_eq!(m.resize_handle_node(), None);
    }

    #[test]
    fn node_edit_routes_clicks_to_section_for_matching_node_only() {
        let m = InteractionMode::NodeEdit { node_id: "0.1".into() };
        assert!(m.click_resolves_to_section("0.1"));
        assert!(!m.click_resolves_to_section("0"));
        assert!(!m.click_resolves_to_section("0.2"));
        assert!(!m.intercepts_left_click());
        assert!(!m.is_target_picker());
        assert_eq!(m.resize_handle_node(), None);
        assert_eq!(m.resize_handle_section(), None);
    }

    #[test]
    fn resize_node_target_resolves_to_node_handles_only() {
        let m = InteractionMode::Resize {
            target: ResizeTarget::Node("0".into()),
        };
        assert_eq!(m.resize_handle_node(), Some("0"));
        assert_eq!(m.resize_handle_section(), None);
        assert!(!m.click_resolves_to_section("0"));
        assert!(!m.is_target_picker());
    }

    #[test]
    fn resize_section_target_resolves_to_section_handles_only() {
        let m = InteractionMode::Resize {
            target: ResizeTarget::Section {
                node_id: "0".into(),
                section_idx: 1,
            },
        };
        assert_eq!(m.resize_handle_node(), None);
        assert_eq!(m.resize_handle_section(), Some(("0", 1)));
    }

    #[test]
    fn resize_target_node_and_section_are_distinct() {
        let n = ResizeTarget::Node("0".into());
        let s = ResizeTarget::Section { node_id: "0".into(), section_idx: 0 };
        assert_ne!(n, s);
    }
}
