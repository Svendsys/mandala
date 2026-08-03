// SPDX-License-Identifier: MPL-2.0

//! Structural invariants for `.mindmap.json`: tree shape, Dewey IDs,
//! edge references, palette references, named enums, text-run
//! bounds, zoom-bound ordering, and keys the loader kept without
//! understanding. Each check returns `Vec<Violation>`; `verify()`
//! runs them all.

mod edges;
mod enums;
mod ids;
mod numeric;
mod palettes;
mod references;
mod sections;
mod tree;
mod unknown_keys;

#[cfg(test)]
mod test_helpers;

use baumhard::mindmap::model::{MindMap, MindNode};

/// Severity of a [`Violation`]. Warnings are printed but do not make
/// `maptool verify` exit nonzero — they flag likely mistakes (e.g.
/// section channel collisions) where the resulting behavior is still
/// well-defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub category: &'static str,
    pub location: String,
    pub message: String,
    pub severity: Severity,
}

impl Violation {
    /// Violation at a node's id.
    pub fn node(category: &'static str, node: &MindNode, message: impl Into<String>) -> Self {
        Self {
            category,
            location: node.id.clone(),
            message: message.into(),
            severity: Severity::Error,
        }
    }

    /// Warning at a node's id.
    pub fn node_warn(category: &'static str, node: &MindNode, message: impl Into<String>) -> Self {
        Self {
            category,
            location: node.id.clone(),
            message: message.into(),
            severity: Severity::Warning,
        }
    }

    /// Violation at `edge[<idx>]`.
    pub fn edge(category: &'static str, edge_index: usize, message: impl Into<String>) -> Self {
        Self {
            category,
            location: format!("edge[{}]", edge_index),
            message: message.into(),
            severity: Severity::Error,
        }
    }

    /// Violation at an arbitrary location (palette names, etc).
    pub fn at(category: &'static str, location: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            category,
            location: location.into(),
            message: message.into(),
            severity: Severity::Error,
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
    out.extend(edges::check(map));
    out.extend(palettes::check(map));
    out.extend(enums::check(map));
    out.extend(sections::check(map));
    out.extend(numeric::check(map));
    out.extend(unknown_keys::check(map));
    out.extend(unknown_keys::check_skipped_constructs(map));
    out
}

#[cfg(test)]
mod constructor_tests {
    use super::*;

    /// `Display` formats as `<category> @ <location>: <message>`.
    /// Pinned because any drift from this format would silently
    /// break downstream verify-output parsing.
    #[test]
    fn violation_display_format_is_stable() {
        let v = Violation::edge("references", 0, "from_id missing");
        assert_eq!(format!("{}", v), "references @ edge[0]: from_id missing");
    }
}

#[cfg(test)]
mod loader_parity_tests {
    /// **`verify` must diagnose exactly the maps the editor refuses,
    /// and this is the registry that makes the claim checkable.**
    ///
    /// `verify` loads through `loader::parse_for_inspection`, which
    /// deliberately skips `check_invariants` so a broken map can still
    /// be inspected. That is the right design and it carries an
    /// obligation: every rule the loader refuses on needs a counterpart
    /// here, or `verify` calls a map "valid" that the app will not
    /// open. The zero-section rule had none — `verify` exited 0 on a
    /// node with `"sections": []` while the editor refused the file,
    /// and nothing noticed because the divergence only became
    /// reachable when `verify` moved off the strict door.
    ///
    /// So the rule list is read out of `check_invariants`'s own body
    /// rather than restated. A rule added there with no row fails this
    /// test; a row naming a counterpart that no longer exists fails it
    /// too.
    ///
    /// What it cannot check is that the counterpart is *equivalent* —
    /// only that one is named and exists. Equivalence is what the
    /// per-rule tests in each verify module are for.
    #[test]
    fn test_every_loader_invariant_has_a_verify_counterpart() {
        // (the rule as `check_invariants` calls it, its counterpart here)
        let registry: &[(&str, &str)] = &[
            ("detect_zero_section_node", "sections::check via validate::zero_section_node"),
            ("detect_id_key_mismatch", "ids::check"),
            ("detect_parent_cycle", "tree::check"),
            ("detect_section_count_cap", "sections::check via validate::section_count"),
            ("map_numeric_domain", "numeric::check"),
            // Warns rather than refuses, and belongs here for the same
            // reason the refusals do: a load-time warning goes to a log
            // nobody reads, and `verify`'s nonzero exit is where it
            // becomes a moment somebody finds out.
            ("warn_on_duplicate_edges", "edges::check"),
        ];

        let loader = include_str!("../../../../lib/baumhard/src/mindmap/loader.rs");
        let body = loader
            .split_once("fn check_invariants(map: MindMap) -> Result<MindMap, String> {")
            .expect("check_invariants must still be spelled this way")
            .1
            .split_once("\n}\n")
            .expect("its body must end at a top-level brace")
            .0;

        // Every `foo(&map)` call in the body is a rule.
        let mut called: Vec<&str> = Vec::new();
        for line in body.lines() {
            let Some(at) = line.find("(&map)") else { continue };
            let name = line[..at]
                .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or("");
            if !name.is_empty() {
                called.push(name);
            }
        }
        assert!(
            called.len() >= 6,
            "the body scan found {} rules, fewer than the registry already knows about — \
             the parse is what broke, not the loader: {called:?}",
            called.len()
        );

        for rule in &called {
            assert!(
                registry.iter().any(|(name, _)| name == rule),
                "`loader::{rule}` inspects the whole map at load — refusing or warning — \
                 and has no row here. Add the `verify` check that reports the same thing. \
                 Without one, `maptool verify` answers `valid` for a map the editor will \
                 not open, which is the most misleading answer the tool can give."
            );
        }
        for (rule, _) in registry {
            assert!(
                called.contains(rule),
                "the registry has a row for `{rule}`, which `check_invariants` no longer \
                 calls. Delete the row, or find out why the loader stopped checking it."
            );
        }
        // And each named counterpart must exist.
        let here = include_str!("mod.rs");
        for (rule, counterpart) in registry {
            let module = counterpart.split("::").next().unwrap_or("");
            assert!(
                here.contains(&format!("{module}::check")),
                "the row for `{rule}` names `{counterpart}`, and `{module}::check` is not \
                 called by `verify`"
            );
        }
    }
}
