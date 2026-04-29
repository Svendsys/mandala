// SPDX-License-Identifier: MPL-2.0

//! `zoom min=<pt|unset> max=<pt|unset>` / `zoom clear` — set or
//! clear the zoom-visibility window on the current selection.
//!
//! Orthogonal to `font min=/max=` (which writes screen-space font
//! clamps): this command writes `min_zoom_to_render` /
//! `max_zoom_to_render` — the presence gate controlling whether
//! an element is rendered at all at the current camera zoom.
//!
//! Routing against the active selection:
//! - `Node`: writes `MindNode.{min,max}_zoom_to_render`.
//! - `Edge`: writes `MindEdge.{min,max}_zoom_to_render`.
//! - `EdgeLabel`: writes `label_config.{min,max}_zoom_to_render`
//!   (replace-not-intersect cascade vs. edge).
//! - `PortalLabel`: writes the owning edge's top-level pair —
//!   the icon inherits edge bounds, same posture as `font`.
//! - `PortalText`: writes `PortalEndpointState.{min,max}_zoom_to_render`
//!   (replace-not-intersect cascade vs. edge).
//! - `Multi`: fans out over each node id.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::{MindMapDocument, OptionEdit, SelectionState};

pub const KEYS: &[&str] = &["min", "max"];
pub const VERBS: &[&str] = &["clear"];
/// Preset zoom levels surfaced in completion. The camera clamps
/// to `[0.05, 5.0]` so values outside that range are accepted
/// but will never match a real camera zoom.
pub const VALUE_PRESETS: &[&str] = &["unset", "0.25", "0.5", "1.0", "1.5", "2.0", "3.0", "5.0"];

pub const COMMAND: Command = Command {
    name: "zoom",
    aliases: &["visibility"],
    summary: "Gate the selection's rendering on camera zoom level",
    usage: "zoom [min=<zoom|unset>] [max=<zoom|unset>]   |   zoom clear",
    tags: &[
        "zoom", "visibility", "presence", "render", "min", "max",
        "clamp", "unset", "clear", "layer", "lod",
    ],
    applicable: always,
    complete: complete_zoom,
    execute: execute_zoom,
};

fn complete_zoom(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index: 0 } => {
            // Position 0: either a verb (`clear`) or a kv key.
            let mut out = prefix_filter(VERBS, state.partial);
            for k in KEYS {
                if k.starts_with(state.partial) {
                    out.push(Completion {
                        text: format!("{}=", k),
                        display: format!("{}=", k),
                        hint: Some(
                            match *k {
                                "min" => "lower inclusive zoom bound (or `unset`)",
                                "max" => "upper inclusive zoom bound (or `unset`)",
                                _ => "zoom bound",
                            }
                            .into(),
                        ),
                        font_family: None,
                    });
                }
            }
            out
        }
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
        CompletionContext::KvValue { key } if KEYS.contains(&key.as_str()) => {
            prefix_filter(VALUE_PRESETS, state.partial)
        }
        _ => Vec::new(),
    }
}

/// Parse a kv value into a [`OptionEdit::Set`] or
/// [`OptionEdit::Clear`]. `unset` or empty string → Clear;
/// anything else must parse as a positive finite `f32`. Returns
/// an `ExecResult::Err` for malformed values so the console
/// surfaces a clear error instead of a silent no-op.
fn parse_bound(key: &str, value: &str) -> Result<OptionEdit<f32>, ExecResult> {
    if value.is_empty() || value.eq_ignore_ascii_case("unset") {
        return Ok(OptionEdit::Clear);
    }
    match value.parse::<f32>() {
        Ok(v) if v.is_finite() && v > 0.0 => Ok(OptionEdit::Set(v)),
        Ok(v) => Err(ExecResult::err(format!(
            "{}='{}' must be positive and finite or `unset`; got {}",
            key, value, v
        ))),
        Err(_) => Err(ExecResult::err(format!(
            "{}='{}' is not a number or `unset`",
            key, value
        ))),
    }
}

fn execute_zoom(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Positional `clear` verb: treat as `min=unset max=unset`.
    let (min_edit, max_edit) = match args.positional(0) {
        Some("clear") => (OptionEdit::Clear, OptionEdit::Clear),
        Some(other) => {
            return ExecResult::err(format!(
                "unknown verb '{other}' — usage: zoom [min=<zoom|unset>] [max=<zoom|unset>]   |   zoom clear"
            ))
        }
        None => {
            let mut min = OptionEdit::Keep;
            let mut max = OptionEdit::Keep;
            let mut saw_any = false;
            for (k, v) in args.kvs() {
                saw_any = true;
                match k {
                    "min" => match parse_bound("min", v) {
                        Ok(e) => min = e,
                        Err(err) => return err,
                    },
                    "max" => match parse_bound("max", v) {
                        Ok(e) => max = e,
                        Err(err) => return err,
                    },
                    other => return ExecResult::err(format!("unknown key '{other}'")),
                }
            }
            if !saw_any {
                return ExecResult::err(
                    "usage: zoom [min=<zoom|unset>] [max=<zoom|unset>]   |   zoom clear",
                );
            }
            (min, max)
        }
    };

    if matches!(min_edit, OptionEdit::Keep) && matches!(max_edit, OptionEdit::Keep) {
        return ExecResult::err("zoom: nothing to set");
    }

    // Reject obviously-inverted explicit bounds up front so the
    // user sees a clear error instead of a silent no-op from the
    // setter's inverted-bounds guard. Mirrors the `font` command.
    if let (OptionEdit::Set(lo), OptionEdit::Set(hi)) = (min_edit, max_edit) {
        if lo > hi {
            return ExecResult::err(format!(
                "zoom: min={lo} > max={hi} (inverted bounds)"
            ));
        }
    }

    let doc = &mut eff.document;
    match doc.selection.clone() {
        SelectionState::Single(id) => {
            finalize("node", doc.set_node_zoom_visibility(&id, min_edit, max_edit))
        }
        SelectionState::Multi(ids) => {
            let mut changed = 0usize;
            for id in &ids {
                if doc.set_node_zoom_visibility(id, min_edit, max_edit) {
                    changed += 1;
                }
            }
            if changed == 0 {
                ExecResult::ok_msg("zoom: no change")
            } else {
                ExecResult::ok_msg(format!("zoom: applied to {changed} node(s)"))
            }
        }
        SelectionState::Edge(er) => {
            finalize("edge", doc.set_edge_zoom_visibility(&er, min_edit, max_edit))
        }
        SelectionState::EdgeLabel(s) => {
            finalize(
                "edge label",
                doc.set_edge_label_zoom_visibility(&s.edge_ref, min_edit, max_edit),
            )
        }
        SelectionState::PortalLabel(s) => {
            // Portal icon routes to the owning edge's top-level
            // pair (same sink as `Edge`). Mirrors the `font`
            // command's portal-label posture.
            finalize(
                "portal label",
                doc.set_edge_zoom_visibility(&s.edge_ref(), min_edit, max_edit),
            )
        }
        SelectionState::PortalText(s) => {
            finalize(
                "portal text",
                doc.set_portal_endpoint_zoom_visibility(
                    &s.edge_ref(),
                    &s.endpoint_node_id,
                    min_edit,
                    max_edit,
                ),
            )
        }
        SelectionState::None => ExecResult::err("zoom: no selection"),
    }
}

fn finalize(kind: &str, changed: bool) -> ExecResult {
    if changed {
        ExecResult::ok_msg(format!("zoom applied to {kind}"))
    } else {
        ExecResult::ok_msg(format!("zoom: no change on {kind}"))
    }
}

/// Parse a parametric Action's payload string into an
/// [`OptionEdit<f32>`]. Returns `None` for malformed values; the
/// Action arm warn-logs and proceeds. Mirrors `parse_bound` on the
/// verb side but without the `ExecResult` wrapping.
pub(crate) fn parse_zoom_payload(value: &str) -> Option<OptionEdit<f32>> {
    if value.is_empty() || value.eq_ignore_ascii_case("unset") {
        return Some(OptionEdit::Clear);
    }
    match value.parse::<f32>() {
        Ok(v) if v.is_finite() && v > 0.0 => Some(OptionEdit::Set(v)),
        _ => None,
    }
}

/// Mutation core: apply a zoom-visibility edit pair to the current
/// selection. Selection-aware in the same way the verb is — node /
/// multi-node / edge / edge-label / portal-label / portal-text each
/// route to their own setter. Returns `true` when at least one
/// target actually changed.
pub(crate) fn apply_zoom_to_selection(
    doc: &mut MindMapDocument,
    min: OptionEdit<f32>,
    max: OptionEdit<f32>,
) -> bool {
    if matches!(min, OptionEdit::Keep) && matches!(max, OptionEdit::Keep) {
        return false;
    }
    // Reject inverted explicit bounds — the Action surface has no
    // scrollback, so we silently no-op rather than write half the
    // edit and surprise the user.
    if let (OptionEdit::Set(lo), OptionEdit::Set(hi)) = (min, max) {
        if lo > hi {
            return false;
        }
    }
    match doc.selection.clone() {
        SelectionState::Single(id) => doc.set_node_zoom_visibility(&id, min, max),
        SelectionState::Multi(ids) => {
            let mut changed = false;
            for id in &ids {
                changed |= doc.set_node_zoom_visibility(id, min, max);
            }
            changed
        }
        SelectionState::Edge(er) => doc.set_edge_zoom_visibility(&er, min, max),
        SelectionState::EdgeLabel(s) => {
            doc.set_edge_label_zoom_visibility(&s.edge_ref, min, max)
        }
        SelectionState::PortalLabel(s) => {
            doc.set_edge_zoom_visibility(&s.edge_ref(), min, max)
        }
        SelectionState::PortalText(s) => {
            doc.set_portal_endpoint_zoom_visibility(&s.edge_ref(), &s.endpoint_node_id, min, max)
        }
        SelectionState::None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bound_unset_is_clear() {
        assert_eq!(
            parse_bound("min", "unset").expect("parses"),
            OptionEdit::Clear
        );
        assert_eq!(
            parse_bound("min", "").expect("parses"),
            OptionEdit::Clear
        );
        assert_eq!(
            parse_bound("min", "UNSET").expect("case-insensitive"),
            OptionEdit::Clear
        );
    }

    #[test]
    fn parse_bound_numeric_is_set() {
        assert_eq!(
            parse_bound("min", "1.5").expect("parses"),
            OptionEdit::Set(1.5)
        );
        assert_eq!(
            parse_bound("max", "0.05").expect("parses"),
            OptionEdit::Set(0.05)
        );
    }

    #[test]
    fn parse_bound_rejects_non_positive() {
        assert!(parse_bound("min", "0").is_err());
        assert!(parse_bound("min", "-1.0").is_err());
    }

    #[test]
    fn parse_bound_rejects_non_finite() {
        assert!(parse_bound("min", "NaN").is_err());
        assert!(parse_bound("max", "inf").is_err());
    }

    #[test]
    fn parse_bound_rejects_garbage() {
        assert!(parse_bound("min", "potato").is_err());
    }

    // Mutation-core tests for the parametric Action arms ─────────
    use crate::application::document::tests_common::{first_testament_node_id, load_test_doc};

    #[test]
    fn parse_zoom_payload_unset_is_clear() {
        assert_eq!(parse_zoom_payload("unset"), Some(OptionEdit::Clear));
        assert_eq!(parse_zoom_payload(""), Some(OptionEdit::Clear));
    }

    #[test]
    fn parse_zoom_payload_finite_positive_is_set() {
        assert_eq!(parse_zoom_payload("0.5"), Some(OptionEdit::Set(0.5)));
        assert_eq!(parse_zoom_payload("2.0"), Some(OptionEdit::Set(2.0)));
    }

    #[test]
    fn parse_zoom_payload_rejects_non_finite_and_non_positive() {
        assert_eq!(parse_zoom_payload("0"), None);
        assert_eq!(parse_zoom_payload("-1"), None);
        assert_eq!(parse_zoom_payload("NaN"), None);
        assert_eq!(parse_zoom_payload("inf"), None);
        assert_eq!(parse_zoom_payload("garbage"), None);
    }

    #[test]
    fn apply_zoom_to_selection_writes_min_on_node() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id.clone());
        let changed = apply_zoom_to_selection(
            &mut doc,
            OptionEdit::Set(0.5),
            OptionEdit::Keep,
        );
        assert!(changed);
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().min_zoom_to_render,
            Some(0.5),
        );
    }

    #[test]
    fn apply_zoom_to_selection_keep_keep_is_noop() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id);
        assert!(!apply_zoom_to_selection(
            &mut doc,
            OptionEdit::Keep,
            OptionEdit::Keep,
        ));
    }

    #[test]
    fn apply_zoom_to_selection_inverted_bounds_silent_no_op() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id);
        // min=2.0, max=0.5 — inverted; the core silently no-ops
        // (Action arm has no scrollback to surface a typed error).
        assert!(!apply_zoom_to_selection(
            &mut doc,
            OptionEdit::Set(2.0),
            OptionEdit::Set(0.5),
        ));
    }

    #[test]
    fn apply_zoom_to_selection_clear_drops_overrides() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id.clone());
        // Set first so clear has something to drop.
        let _ = apply_zoom_to_selection(
            &mut doc,
            OptionEdit::Set(0.5),
            OptionEdit::Set(2.0),
        );
        let cleared = apply_zoom_to_selection(
            &mut doc,
            OptionEdit::Clear,
            OptionEdit::Clear,
        );
        assert!(cleared);
        let node = doc.mindmap.nodes.get(&id).unwrap();
        assert!(node.min_zoom_to_render.is_none());
        assert!(node.max_zoom_to_render.is_none());
    }
}
