// SPDX-License-Identifier: MPL-2.0

//! `InteractionMode` — what the user is doing at the canvas level.
//!
//! The modes that edit the map arrive with the issues that rebuild
//! their editing on the one writer: reparent and connect with V1-C4
//! (#245), resize with V1-C7 (#248), node editing with V1-C8 (#249).
//! Every surface that reports a mode — the IPC `query.mode`,
//! `Action::ExitMode` — reads this type.
//!
//! Cross-platform: it carries no GPU handles, so both targets compile
//! it. Reparent and connect are entered only by native-only Actions (as
//! on `main`), so nothing on wasm32 constructs them.

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
}

impl InteractionMode {
    /// The nodes a target-picking mode acts on: reparent's and connect's
    /// sources, none in any other mode.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn sources(&self) -> &[NodeId] {
        match self {
            InteractionMode::Reparent { sources } | InteractionMode::Connect { sources } => sources,
            InteractionMode::Default => &[],
        }
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
