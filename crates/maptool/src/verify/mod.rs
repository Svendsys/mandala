// SPDX-License-Identifier: MPL-2.0

//! Structural invariant verification for `.mindmap.json` files.
//!
//! Verification is a boundary check, not a best-effort parse: a file
//! either satisfies every named invariant the format guarantees or it
//! doesn't, and each violation is reported as a specific, named
//! property (tree shape, Dewey-ID consistency, edge references, palette
//! references, named-enum membership, text-run bounds) rather than a
//! free-form error message. That separation is what makes `verify` safe
//! to run as a gate: the loader can be permissive about missing or
//! defaulted fields, and everything load-tolerant-but-structurally-
//! invalid still surfaces here.

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
    /// Construct a violation pinned to a node's id-as-location.
    /// The 12+ per-checker call sites that all wrote
    /// `Violation { category, location: node.id.clone(),
    /// message: format!(...) }` collapse to one line.
    pub fn node(
        category: &'static str,
        node: &MindNode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            category,
            location: node.id.clone(),
            message: message.into(),
        }
    }

    /// Construct a violation pinned to an edge's `edge[<idx>]`
    /// location stamp — the canonical format every per-checker
    /// previously open-coded with `format!("edge[{}]", i)`.
    pub fn edge(
        category: &'static str,
        edge_index: usize,
        message: impl Into<String>,
    ) -> Self {
        Self {
            category,
            location: format!("edge[{}]", edge_index),
            message: message.into(),
        }
    }

    /// Construct a violation with an arbitrary location string —
    /// the escape hatch for checks whose location isn't a node id
    /// or an edge index (palette names, drifted HashMap keys,
    /// pre-formatted location strings emitted by inner helpers).
    pub fn at(
        category: &'static str,
        location: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
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
