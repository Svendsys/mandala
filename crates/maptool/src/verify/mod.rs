// SPDX-License-Identifier: MPL-2.0

//! Structural invariants for `.mindmap.json`: tree shape, Dewey IDs,
//! edge references, palette references, named enums, text-run
//! bounds, zoom-bound ordering. Each check returns `Vec<Violation>`;
//! `verify()` runs them all.

mod enums;
mod ids;
mod palettes;
mod references;
mod text_runs;
mod tree;
mod zoom_bounds;

#[cfg(test)]
mod test_helpers;

use baumhard::mindmap::model::{MindMap, MindNode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub category: &'static str,
    pub location: String,
    pub message: String,
}

impl Violation {
    /// Violation at a node's id.
    pub fn node(category: &'static str, node: &MindNode, message: impl Into<String>) -> Self {
        Self {
            category,
            location: node.id.clone(),
            message: message.into(),
        }
    }

    /// Violation at `edge[<idx>]`.
    pub fn edge(category: &'static str, edge_index: usize, message: impl Into<String>) -> Self {
        Self {
            category,
            location: format!("edge[{}]", edge_index),
            message: message.into(),
        }
    }

    /// Violation at an arbitrary location (palette names, etc).
    pub fn at(category: &'static str, location: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            category,
            location: location.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} @ {}: {}", self.category, self.location, self.message)
    }
}

/// Run all invariant checks and return every violation found.
/// An empty Vec means the file is valid.
pub fn verify(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();
    out.extend(tree::check(map));
    out.extend(ids::check(map));
    out.extend(references::check(map));
    out.extend(palettes::check(map));
    out.extend(enums::check(map));
    out.extend(text_runs::check(map));
    out.extend(zoom_bounds::check(map));
    out
}

#[cfg(test)]
mod constructor_tests {
    use super::*;
    use crate::verify::test_helpers::node;

    #[test]
    fn violation_node_uses_node_id_as_location() {
        let n = node("0.3.1", None);
        let v = Violation::node("test_cat", &n, "boom");
        assert_eq!(v.category, "test_cat");
        assert_eq!(v.location, "0.3.1");
        assert_eq!(v.message, "boom");
    }

    #[test]
    fn violation_edge_uses_bracket_index_stamp() {
        let v = Violation::edge("test_cat", 7, "boom");
        assert_eq!(v.category, "test_cat");
        assert_eq!(v.location, "edge[7]");
        assert_eq!(v.message, "boom");
    }

    #[test]
    fn violation_at_passes_location_through() {
        let v = Violation::at("test_cat", "palette[coral]", "boom");
        assert_eq!(v.category, "test_cat");
        assert_eq!(v.location, "palette[coral]");
        assert_eq!(v.message, "boom");
    }

    /// `Display` formats as `<category> @ <location>: <message>`.
    /// Pinned because any drift from this format would silently
    /// break downstream verify-output parsing.
    #[test]
    fn violation_display_format_is_stable() {
        let v = Violation::edge("references", 0, "from_id missing");
        assert_eq!(format!("{}", v), "references @ edge[0]: from_id missing");
    }
}
