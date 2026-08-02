// SPDX-License-Identifier: MPL-2.0

//! Numeric-domain invariants — the same rules the loader enforces at
//! the trust boundary, reported here instead of refused.
//!
//! **The two must agree, and this is what makes them.** The loader
//! rejects a map whose numbers would reach an `assert!` inside the
//! text shaper, an inverted `f32::clamp`, or an allocation sized
//! from authored geometry (`baumhard::mindmap::model::validate`).
//! `verify` deliberately loads through the lenient door
//! (`loader::parse_for_inspection`) so it can still inspect those
//! files — but a looser door without the matching rules would mean
//! `verify` printing "valid" for a map the editor refuses to open,
//! which is the most misleading answer the tool could give.
//!
//! So the checks are not restated here. Every rule comes from the
//! same `validate::*_violations` helpers the loader calls, and the
//! only work this module does is stamping each message with the
//! document location `verify` reports in.

use baumhard::mindmap::model::{validate, MindMap};

use super::Violation;

/// Report every numeric-domain violation in `map`, in the order the
/// loader would have hit them: canvas, then map-level mutations,
/// then nodes (sorted by id, with their inline mutations), then
/// edges.
///
/// Cost: one pass over nodes, sections, text runs, edges, control
/// points and mutations — the same walk the loader's sweep makes.
pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for message in validate::canvas_numeric_violations(&map.canvas) {
        out.push(Violation::at("numeric", "canvas", message));
    }

    for (idx, mutation) in map.custom_mutations.iter().enumerate() {
        for message in validate::mutation_numeric_violations(mutation) {
            out.push(Violation::at(
                "numeric",
                format!("custom_mutations[{idx}]"),
                message,
            ));
        }
    }

    // Sorted so the report is stable across `HashMap` iteration
    // order — the same reason the loader's sweep sorts.
    let mut nodes: Vec<_> = map.nodes.values().collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    for node in nodes {
        for message in validate::node_numeric_violations(node) {
            out.push(Violation::node("numeric", node, message));
        }
        for (idx, mutation) in node.inline_mutations.iter().enumerate() {
            for message in validate::mutation_numeric_violations(mutation) {
                out.push(Violation::node(
                    "numeric",
                    node,
                    format!("inline_mutations[{idx}]: {message}"),
                ));
            }
        }
    }

    for (idx, edge) in map.edges.iter().enumerate() {
        for message in validate::edge_numeric_violations(edge) {
            out.push(Violation::edge("numeric", idx, message));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::test_helpers::node;
    use baumhard::mindmap::border::default_glyph_border_config;

    /// **The tool and the editor must agree about what is valid.**
    /// A map carrying a number the loader refuses must not be
    /// reported clean here, or `verify` sends an author looking for
    /// a problem it just told them they did not have.
    #[test]
    fn test_numeric_check_flags_what_the_loader_refuses() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        let mut border = default_glyph_border_config();
        border.font_size_pt = 0.0;
        map.nodes.get_mut("0").unwrap().style.border = Some(border);

        let violations = check(&map);
        assert!(
            violations.iter().any(|v| v.message.contains("font_size_pt")),
            "a zero border font size must be reported: {violations:?}"
        );
        assert!(
            violations.iter().all(|v| v.category == "numeric"),
            "every violation here is a numeric-domain one"
        );
    }

    /// The canonical fixture is the control: it loads through the
    /// strict door, so it must produce nothing here.
    #[test]
    fn test_numeric_check_is_silent_on_a_valid_map() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("0.0".into(), node("0.0", Some("0")));
        assert!(check(&map).is_empty(), "a well-formed map has no numeric violations");
    }
}
