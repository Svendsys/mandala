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
    /// node — drag a corner or edge`. `None` for the default mode, and
    /// for NodeEdit, whose line reads the document
    /// ([`Self::node_edit_line`]). Derived from the mode, so nothing else
    /// that writes the status line can erase it while the mode lasts.
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
            InteractionMode::NodeEdit { .. } | InteractionMode::Default => return None,
        };
        let n = self.sources().len();
        let plural = if n == 1 { "" } else { "s" };
        Some(format!("{verb}: {n} source{plural} \u{2014} click a target node"))
    }

    /// What the status line says while NodeEdit edits `node`, as on
    /// `main` (`scene_rebuild::mode_status_line`), with the node's integer
    /// id: `editing: <id>` on a node of one section, which has nothing to
    /// pick; on a node of more, `editing: <id> — section [N of M]`, `N`
    /// the place of `focus` — the section an editor has open, or else the
    /// one selected — among its `M` sections counted from one, and `-`
    /// while no section of the node is focused. `None` for a node the
    /// document no longer holds.
    ///
    /// Cost: O(the node's children).
    pub fn node_edit_line(
        doc: &baumhard::document::ids::Document,
        node: NodeId,
        focus: Option<NodeId>,
    ) -> Option<String> {
        use baumhard::document::element::DocKind;
        let ident = crate::application::view::inspect::ident_of(doc, node)?;
        let sections: Vec<NodeId> = doc
            .children(node)
            .filter(|&child| {
                crate::application::document::selection::kind_of(doc, child) == Some(DocKind::Section)
            })
            .collect();
        if sections.len() <= 1 {
            return Some(format!("editing: {}", ident.0));
        }
        let place = focus
            .and_then(|focus| sections.iter().position(|&section| section == focus))
            .map_or_else(|| "-".to_string(), |index| (index + 1).to_string());
        Some(format!(
            "editing: {} \u{2014} section [{place} of {}]",
            ident.0,
            sections.len()
        ))
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

    use crate::application::app::text_edit::tests::text_fixture;
    use crate::application::view::inspect::children_of_kind;
    use baumhard::document::element::DocKind;
    use baumhard::document::ids::NodeIdent;

    /// The text fixture's node `ident` and its sections.
    fn node_and_sections(
        doc: &crate::application::document::MindMapDocument,
        ident: u32,
    ) -> (baumhard::arena::NodeId, Vec<baumhard::arena::NodeId>) {
        let node = doc.doc().node_by_ident(NodeIdent(ident)).expect("in the fixture");
        (
            node,
            children_of_kind(doc.doc(), node, DocKind::Section).collect(),
        )
    }

    /// `main`'s `test_mode_status_line_default_is_none` (gone): the
    /// default mode has no line, and neither does NodeEdit from the mode
    /// alone, whose line reads the document. Control: a mode that has one
    /// says it.
    #[test]
    fn test_status_line_is_none_for_default_and_read_from_the_document_for_node_edit() {
        let doc = text_fixture();
        let (node, _) = node_and_sections(&doc, 1);
        assert_eq!(InteractionMode::Default.status_line(), None);
        assert_eq!(InteractionMode::NodeEdit { node }.status_line(), None);
        assert!(
            InteractionMode::Reparent { sources: vec![node] }
                .status_line()
                .is_some(),
            "control: a mode with a line of its own says it"
        );
    }

    /// `main`'s
    /// `test_mode_status_line_node_edit_single_section_uses_short_form` (gone):
    /// a node of one section reads `editing: <id>` — nothing to pick, no
    /// `[N of M]` — whatever is focused. Control: a node of two takes the
    /// long form.
    #[test]
    fn test_node_edit_line_on_a_node_of_one_section_is_the_short_form() {
        let doc = text_fixture();
        let (node, sections) = node_and_sections(&doc, 2);
        for focus in [None, sections.first().copied()] {
            assert_eq!(
                InteractionMode::node_edit_line(doc.doc(), node, focus).as_deref(),
                Some("editing: 2")
            );
        }
        let (two, _) = node_and_sections(&doc, 1);
        let long = InteractionMode::node_edit_line(doc.doc(), two, None).expect("held");
        assert!(long.contains("section ["), "control: {long:?}");
    }

    /// `main`'s
    /// `test_mode_status_line_node_edit_multi_section_renders_n_of_m` (gone):
    /// the focused section's place among the node's, from one. Control:
    /// the first section reads `1`, so the `2` is the second's place, not
    /// a constant.
    #[test]
    fn test_node_edit_line_on_a_node_of_several_sections_renders_n_of_m() {
        let doc = text_fixture();
        let (node, sections) = node_and_sections(&doc, 1);
        assert_eq!(
            InteractionMode::node_edit_line(doc.doc(), node, Some(sections[1])).as_deref(),
            Some("editing: 1 \u{2014} section [2 of 2]")
        );
        assert_eq!(
            InteractionMode::node_edit_line(doc.doc(), node, Some(sections[0])).as_deref(),
            Some("editing: 1 \u{2014} section [1 of 2]"),
            "control"
        );
    }

    /// `main`'s
    /// `test_mode_status_line_node_edit_single_no_section_picked_renders_dash` (gone)
    /// and `…_section_owner_mismatch_renders_dash`: no focus, or a focus
    /// that is not a section of the edited node (another node's section,
    /// the node itself), reads `[- of M]`, never a made-up index. Control:
    /// a section of the node reads its place.
    #[test]
    fn test_node_edit_line_with_no_section_of_the_node_focused_renders_a_dash() {
        let doc = text_fixture();
        let (node, sections) = node_and_sections(&doc, 1);
        let (other, others) = node_and_sections(&doc, 2);
        for focus in [None, Some(node), Some(other), others.first().copied()] {
            assert_eq!(
                InteractionMode::node_edit_line(doc.doc(), node, focus).as_deref(),
                Some("editing: 1 \u{2014} section [- of 2]"),
                "{focus:?}"
            );
        }
        assert!(
            InteractionMode::node_edit_line(doc.doc(), node, Some(sections[0]))
                .is_some_and(|line| line.contains("[1 of 2]")),
            "control"
        );
    }
}
