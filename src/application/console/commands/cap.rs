// SPDX-License-Identifier: MPL-2.0

//! `cap from=arrow to=none` — set the start/end cap glyph on the
//! selected edge. Edge-specific.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::helpers::{
    collect_kvs_or_usage, require_edge_or_portal, ApplyTally,
};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

pub const KEYS: &[&str] = &["from", "to"];
pub const NAMES: &[&str] = &["arrow", "circle", "diamond", "none"];

pub const COMMAND: Command = Command {
    name: "cap",
    aliases: &[],
    summary: "Set the start/end cap glyph of the selected edge",
    usage: "cap from=<arrow|circle|diamond|none> to=<arrow|circle|diamond|none>",
    tags: &["edge", "cap", "arrow", "end", "start"],
    applicable: edge_selected,
    complete: complete_cap,
    execute: execute_cap,
};

fn complete_cap(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
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
        CompletionContext::KvValue { key } if KEYS.iter().any(|k| k == key) => {
            prefix_filter(NAMES, state.partial)
        }
        _ => Vec::new(),
    }
}

pub(crate) fn resolve_cap(endpoint_from: bool, name: &str) -> Option<Option<&'static str>> {
    match (endpoint_from, name) {
        (_, "none") => Some(None),
        (_, "circle") => Some(Some("\u{25CF}")),
        (_, "diamond") => Some(Some("\u{25C6}")),
        (true, "arrow") => Some(Some("\u{25C0}")),
        (false, "arrow") => Some(Some("\u{25B6}")),
        _ => None,
    }
}

/// Mutation core: apply cap-glyph changes to the currently-selected
/// edge. Both the `cap` console verb and the parametric
/// `Action::SetEdgeCap` route through this helper. Invalid preset
/// names silently no-op the corresponding slot — the verb path
/// surfaces typed errors via `ApplyTally`; the Action path warns
/// upstream and returns `Handled` (no scrollback surface).
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_cap_to_selection(
    doc: &mut MindMapDocument,
    from: Option<&str>,
    to: Option<&str>,
) -> bool {
    let Some(er) = doc.selection.selected_edge_or_portal_edge() else {
        return false;
    };
    let mut changed = false;
    if let Some(v) = from {
        if let Some(glyph) = resolve_cap(true, v) {
            changed |= doc.set_edge_cap_start(&er, glyph);
        }
    }
    if let Some(v) = to {
        if let Some(glyph) = resolve_cap(false, v) {
            changed |= doc.set_edge_cap_end(&er, glyph);
        }
    }
    changed
}

fn execute_cap(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let er = match require_edge_or_portal(eff) {
        Ok(e) => e,
        Err(r) => return r,
    };
    let kvs = match collect_kvs_or_usage(args, "usage: cap from=<name> to=<name>") {
        Ok(k) => k,
        Err(r) => return r,
    };

    let mut tally = ApplyTally::new();
    for (k, v) in kvs {
        let is_from = match k.as_str() {
            "from" => true,
            "to" => false,
            other => {
                tally.note_error(format!("unknown key '{}'", other));
                continue;
            }
        };
        let Some(glyph) = resolve_cap(is_from, &v) else {
            tally.note_error(format!("'{}': expected arrow|circle|diamond|none", v));
            continue;
        };
        let changed = if is_from {
            eff.document.set_edge_cap_start(&er, glyph)
        } else {
            eff.document.set_edge_cap_end(&er, glyph)
        };
        tally.note(changed, || format!("cap {} already {}", k, v));
    }
    tally.finalize("cap")
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
    fn resolve_cap_picks_directional_arrow_glyph() {
        // `arrow` is direction-sensitive: from-side arrow is
        // ◀ (U+25C0), to-side arrow is ▶ (U+25B6).
        assert_eq!(resolve_cap(true, "arrow"), Some(Some("\u{25C0}")));
        assert_eq!(resolve_cap(false, "arrow"), Some(Some("\u{25B6}")));
        // `none` clears the cap (Some(None) — Some-wraps the
        // unset answer).
        assert_eq!(resolve_cap(true, "none"), Some(None));
        // Unknown name returns None (the outer Option = "no answer").
        assert_eq!(resolve_cap(true, "totally-invalid"), None);
    }

    #[test]
    fn apply_cap_to_selection_writes_both_ends() {
        let mut doc = doc_with_first_edge_selected();
        let _ = apply_cap_to_selection(&mut doc, Some("circle"), Some("diamond"));
        let cfg = doc.mindmap.edges[0]
            .glyph_connection
            .as_ref()
            .expect("body-glyph fork should leave a cfg");
        assert_eq!(cfg.cap_start.as_deref(), Some("\u{25CF}"));
        assert_eq!(cfg.cap_end.as_deref(), Some("\u{25C6}"));
    }

    #[test]
    fn apply_cap_to_selection_returns_false_with_no_selection() {
        let mut doc = load_test_doc();
        assert!(!apply_cap_to_selection(&mut doc, Some("arrow"), Some("none")));
    }

    #[test]
    fn apply_cap_to_selection_silently_skips_invalid_name() {
        let mut doc = doc_with_first_edge_selected();
        let original = doc.mindmap.edges[0].glyph_connection.clone();
        // Invalid `from` value — core silently no-ops the bad slot.
        assert!(!apply_cap_to_selection(&mut doc, Some("nonsense"), None));
        assert_eq!(doc.mindmap.edges[0].glyph_connection, original);
    }
}
