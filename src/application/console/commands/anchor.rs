// SPDX-License-Identifier: MPL-2.0

//! `anchor from=top to=auto` — edge anchor side setter.
//!
//! Component-specific (edge only); the anchor concept doesn't
//! generalize to nodes or portals, so this bypasses the trait layer
//! and calls `set_edge_anchor` directly.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion, CompletionContext, CompletionState};
use crate::application::console::helpers::{
    collect_kvs_or_usage, require_edge_or_portal, ApplyTally,
};
use crate::application::console::parser::Args;
use crate::application::console::predicates::edge_selected;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

pub const SIDES: &[&str] = &["auto", "top", "right", "bottom", "left"];
pub const KEYS: &[&str] = &["from", "to"];

pub const COMMAND: Command = Command {
    name: "anchor",
    aliases: &[],
    summary: "Set the from/to anchor side of the selected edge",
    usage: "anchor from=<side> to=<side>   (side: auto|top|right|bottom|left)",
    tags: &["edge", "anchor", "side"],
    applicable: edge_selected,
    complete: complete_anchor,
    execute: execute_anchor,
};

fn complete_anchor(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
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
            prefix_filter(SIDES, state.partial)
        }
        _ => Vec::new(),
    }
}

fn side_value(name: &str) -> Option<&str> {
    match name {
        "auto" | "top" | "right" | "bottom" | "left" => Some(name),
        _ => None,
    }
}

fn execute_anchor(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let er = match require_edge_or_portal(eff) {
        Ok(e) => e,
        Err(r) => return r,
    };
    let kvs = match collect_kvs_or_usage(args, "usage: anchor from=<side> to=<side>") {
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
        let Some(val) = side_value(&v) else {
            tally.note_error(format!("'{}': expected auto|top|right|bottom|left", v));
            continue;
        };
        let changed = eff.document.set_edge_anchor(&er, is_from, val);
        tally.note(changed, || format!("{} already {}", k, v));
    }
    tally.finalize("anchor")
}
