// SPDX-License-Identifier: MPL-2.0

//! `spacing value=4.0` or `spacing value=tight` — glyph-spacing
//! setter for the selected edge. Accepts named presets
//! (tight / normal / wide) or a raw float in the preset's unit.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::helpers::require_edge_or_portal;
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

pub const PRESETS: &[(&str, f32)] = &[("tight", 0.0), ("normal", 2.0), ("wide", 6.0)];
pub const VALUE_PRESETS: &[&str] = &["tight", "normal", "wide"];
pub const KEYS: &[&str] = &["value"];

pub const COMMAND: Command = Command {
    name: "spacing",
    aliases: &[],
    summary: "Set the glyph spacing of the selected edge",
    usage: "spacing value=<tight|normal|wide | <float>>",
    tags: &["edge", "spacing", "tight", "wide"],
    applicable: edge_selected,
    complete: complete_spacing,
    execute: execute_spacing,
};

fn complete_spacing(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { .. } => KEYS
            .iter()
            .filter(|k| k.starts_with(state.partial))
            .map(|k| Completion {
                text: format!("{}=", k),
                display: format!("{}=", k),
                hint: None,
                font_family: None,
            })
            .collect(),
        CompletionContext::KvValue { key } if key == "value" => {
            prefix_filter(VALUE_PRESETS, state.partial)
        }
        _ => Vec::new(),
    }
}

fn execute_spacing(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    if let Err(r) = require_edge_or_portal(eff) {
        return r;
    }
    let v = match args.kv("value") {
        Some(v) => v,
        None => return ExecResult::err("usage: spacing value=<tight|normal|wide | <float>>"),
    };
    if resolve_spacing_value(v).is_none() {
        return ExecResult::err(format!(
            "'{}' must be a preset (tight|normal|wide) or a float",
            v
        ));
    }
    // Route through the mutation core — same setter, single
    // source of truth with the parametric `Action::SetSpacing` arm.
    if apply_spacing_to_selection(eff.document, v) {
        ExecResult::ok_msg(format!("spacing set to {}", v))
    } else {
        ExecResult::ok_msg(format!("spacing already {}", v))
    }
}

/// Resolve a spacing value (preset name or float) to its f32. Used
/// by both the verb body and the parametric Action arm.
pub(crate) fn resolve_spacing_value(input: &str) -> Option<f32> {
    if let Some((_, preset)) = PRESETS.iter().find(|(n, _)| *n == input) {
        return Some(*preset);
    }
    input.parse::<f32>().ok().filter(|f| f.is_finite())
}

/// Mutation core: apply a spacing value (preset name or finite float)
/// to the selected edge. Returns `true` when the value changed.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_spacing_to_selection(doc: &mut MindMapDocument, input: &str) -> bool {
    let Some(er) = doc.selection.selected_edge_or_portal_edge() else {
        return false;
    };
    let Some(value) = resolve_spacing_value(input) else {
        return false;
    };
    doc.set_edge_spacing(&er, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::{EdgeLabelSel, EdgeRef, SelectionState};

    fn doc_with_first_edge_selected() -> MindMapDocument {
        let mut doc = load_test_doc();
        let e = doc.mindmap.edges.first().expect("testament edges");
        let er = EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type);
        doc.selection = SelectionState::EdgeLabel(EdgeLabelSel::new(er));
        doc
    }

    #[test]
    fn resolve_spacing_value_handles_presets_and_floats() {
        assert_eq!(resolve_spacing_value("tight"), Some(0.0));
        assert_eq!(resolve_spacing_value("normal"), Some(2.0));
        assert_eq!(resolve_spacing_value("wide"), Some(6.0));
        assert_eq!(resolve_spacing_value("3.5"), Some(3.5));
        assert_eq!(resolve_spacing_value("not-a-value"), None);
        assert_eq!(resolve_spacing_value("inf"), None);
    }

    #[test]
    fn apply_spacing_to_selection_writes_preset() {
        let mut doc = doc_with_first_edge_selected();
        let _ = apply_spacing_to_selection(&mut doc, "wide");
        let cfg = doc.mindmap.edges[0]
            .glyph_connection
            .as_ref()
            .expect("glyph_connection should fork on first spacing write");
        assert!((cfg.spacing - 6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn apply_spacing_to_selection_no_op_with_no_selection() {
        let mut doc = load_test_doc();
        assert!(!apply_spacing_to_selection(&mut doc, "wide"));
    }

    #[test]
    fn apply_spacing_to_selection_silently_skips_invalid_input() {
        let mut doc = doc_with_first_edge_selected();
        let original = doc.mindmap.edges[0].glyph_connection.clone();
        assert!(!apply_spacing_to_selection(&mut doc, "definitely-not-a-value"));
        assert_eq!(doc.mindmap.edges[0].glyph_connection, original);
    }

    #[test]
    fn apply_spacing_to_selection_returns_false_for_node_selection() {
        // L1 — spacing is edge-only; a node selection no-ops.
        let mut doc = load_test_doc();
        let id = doc.mindmap.nodes.keys().next().unwrap().clone();
        doc.selection = SelectionState::Single(id);
        assert!(!apply_spacing_to_selection(&mut doc, "wide"));
    }
}
