// SPDX-License-Identifier: MPL-2.0

//! `InteractionMode` — what the user is doing at the canvas level.
//!
//! The modes that edit the map arrived with the issues that rebuilt
//! their editing on the one writer: reparent and connect with V1-C4
//! (#245), resize with V1-C7 (#248), node editing with V1-C8 (#249).
//! A resize of a section is entered from a selected section, which only
//! node editing selects: in NodeEdit, a click on a section of the
//! active node selects that section.
//! Every surface that reports a mode — the IPC `query.mode`,
//! `Action::ExitMode` — reads this type.
//!
//! Cross-platform: it carries no GPU handles, so both targets compile
//! it. Reparent, connect and resize are entered only by native-only
//! Actions (as on `main`), so nothing on wasm32 constructs them;
//! NodeEdit is entered on both, by the section editor's openers, as on
//! `main`.

use baumhard::arena::NodeId;

/// What the user is doing right now at the canvas level. One is always
/// active; transitions go through `Action` dispatch.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub enum InteractionMode {
    /// The canvas's default mode: a click selects, a drag pans.
    #[default]
    Default,
    /// Choosing a new parent for `sources`: the next left click on a
    /// node moves them under it, one on empty canvas makes them roots
    /// (`Action::ReparentToTarget`). The hovered valid target is tinted.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Reparent {
        /// The nodes to move, in selection order.
        sources: Vec<NodeId>,
    },
    /// Choosing the node `sources` link to: the next left click on a
    /// node gives each a line link to it, one on empty canvas leaves the
    /// mode (`Action::ConnectToTarget`). The hovered target is tinted.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Connect {
        /// The nodes to link from, in selection order.
        sources: Vec<NodeId>,
    },
    /// Editing inside `node` (`Action::EnterNodeEdit`, `mode node-edit`):
    /// the other nodes are dimmed and, when it has more than one section,
    /// its sections are outlined (the view's NodeEdit chrome). A click on
    /// one of its sections selects that section — which a drag then
    /// moves, `EnterResizeMode` resizes and `EnterSectionEdit` opens the
    /// editor on — and a click outside it, or `Action::ExitMode`, leaves
    /// the mode.
    NodeEdit {
        /// The node being edited.
        node: NodeId,
    },
    /// The resize handles are shown on `target`: dragging one resizes
    /// it, as a transaction (`app::live`). A click that grabs no handle,
    /// and `Action::ExitMode`, leave the mode (`Action::EnterResizeMode`).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Resize {
        /// What the handles resize.
        target: ResizeTarget,
    },
}

/// What a resize mode's handles resize: a node, or one of its sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub enum ResizeTarget {
    /// A node, by its arena id: its minimum size, and for a top or left
    /// handle its position.
    Node(NodeId),
    /// A section, by its arena id: its minimum size, and for a top or
    /// left handle its offset.
    Section(NodeId),
}

impl ResizeTarget {
    /// What the resize mode resizes for `selection` of `doc`: its one
    /// element, when that is a node or a section (a section is selected
    /// only by node editing, V1-C8). `None` for anything else. What
    /// `Action::EnterResizeMode` and `mode resize` enter on. O(1).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn of(
        doc: &baumhard::document::ids::Document,
        selection: &crate::application::document::Selection,
    ) -> Option<Self> {
        use baumhard::document::element::DocKind;
        let mut selected = selection.iter();
        let (Some(only), None) = (selected.next(), selected.next()) else {
            return None;
        };
        match crate::application::document::selection::kind_of(doc, only)? {
            DocKind::Node => Some(ResizeTarget::Node(only)),
            DocKind::Section => Some(ResizeTarget::Section(only)),
            _ => None,
        }
    }
}

impl InteractionMode {
    /// The nodes a target-picking mode acts on: reparent's and connect's
    /// sources, none in any other mode.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn sources(&self) -> &[NodeId] {
        match self {
            InteractionMode::Reparent { sources } | InteractionMode::Connect { sources } => sources,
            InteractionMode::Default | InteractionMode::NodeEdit { .. } | InteractionMode::Resize { .. } => {
                &[]
            }
        }
    }

    /// The node being edited while NodeEdit is active. O(1).
    pub fn node_edit(&self) -> Option<NodeId> {
        match self {
            InteractionMode::NodeEdit { node } => Some(*node),
            _ => None,
        }
    }

    /// The resize target while the resize mode is active. O(1).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn resize_target(&self) -> Option<ResizeTarget> {
        match self {
            InteractionMode::Resize { target } => Some(*target),
            _ => None,
        }
    }

    /// What the status line says while the mode is active, as on
    /// `main`: `reparent: 2 sources — click a target node`, `resize:
    /// node — drag a corner or edge`. `None` for the default mode.
    /// Derived from the mode, so nothing else that writes the status
    /// line can erase it while the mode lasts.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn status_line(&self) -> Option<String> {
        let verb = match self {
            InteractionMode::Reparent { .. } => "reparent",
            InteractionMode::Connect { .. } => "connect",
            InteractionMode::Resize { target } => {
                let what = match target {
                    ResizeTarget::Node(_) => "node",
                    ResizeTarget::Section(_) => "section",
                };
                return Some(format!("resize: {what} \u{2014} drag a corner or edge"));
            }
            InteractionMode::NodeEdit { .. } => {
                return Some("node edit: pick a section \u{2014} Enter edits it, Esc leaves".to_string());
            }
            InteractionMode::Default => return None,
        };
        let n = self.sources().len();
        let plural = if n == 1 { "" } else { "s" };
        Some(format!("{verb}: {n} source{plural} \u{2014} click a target node"))
    }
}

#[cfg(test)]
mod tests {
    use super::InteractionMode;

    /// The mode a fresh app starts in is the default one.
    #[test]
    fn test_interaction_mode_starts_default() {
        assert_eq!(InteractionMode::default(), InteractionMode::Default);
    }
}
