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

fn resolve_cap(endpoint_from: bool, name: &str) -> Option<Option<&'static str>> {
    match (endpoint_from, name) {
        (_, "none") => Some(None),
        (_, "circle") => Some(Some("\u{25CF}")),
        (_, "diamond") => Some(Some("\u{25C6}")),
        (true, "arrow") => Some(Some("\u{25C0}")),
        (false, "arrow") => Some(Some("\u{25B6}")),
        _ => None,
    }
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
