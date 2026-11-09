// SPDX-License-Identifier: MPL-2.0

//! `mode` — change the active high-level interaction mode.
//!
//! Verbs:
//! - `mode default` — exit any active mode and return to `Default`.
//!   Equivalent to pressing the `ExitMode` keybind (Esc by default).
//! - `mode resize` — Resize mode on the current selection. Its chrome
//!   and its handle drags are a transaction, rebuilt by V1-C7 (#248);
//!   until then the subverb refuses naming that issue.
//! - `mode node-edit` — NodeEdit mode on the selection's node, rebuilt
//!   with node editing by V1-C8 (#249); until then the subverb refuses
//!   naming that issue.
//!
//! `mode show` is intentionally absent: no surface reports the active
//! mode to the console, so there is no verb for it.

use super::Command;
use crate::application::app::InteractionMode;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{kvs, usage, Grammar, Subverb};
use crate::application::console::{ConsoleEffects, ConsoleSideEffect, ExecResult};

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("default", "modes", "leave the current mode and return to Default"),
    Subverb::bare("resize", "modes", "enter Resize on the current selection"),
    Subverb::bare(
        "node-edit",
        "modes",
        "enter NodeEdit on the selection's owning node",
    ),
];

/// Which handler each declared subverb runs (`spec::route`).
#[derive(Clone, Copy)]
enum Route {
    Default,
    Resize,
    NodeEdit,
}

const ROUTES: &Routes<Route> = &[
    ("default", Route::Default),
    ("resize", Route::Resize),
    ("node-edit", Route::NodeEdit),
];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub static GRAMMAR: Grammar = Grammar {
    label: "mode",
    subverb_sets: &[SUBVERBS],
    key_sets: &[],
    bare: None,
};

pub const COMMAND: Command = Command {
    name: "mode",
    aliases: &[],
    summary: "Change the active interaction mode (default / resize / node-edit)",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["interaction", "exit", "edit"],
    execute: execute_mode,
};

fn execute_mode(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Subverb names are matched case-insensitively by the descent,
    // console-wide (`commands/mod.rs` § Casing); the unknown-subverb
    // message still echoes what the user typed.
    let descent = descend(&GRAMMAR, args.tokens());
    if let Err(msg) = kvs::read_strict(&descent, args) {
        return ExecResult::err(msg);
    }
    let route = match descent.stop {
        Stop::Matched(subverb) => match route::lookup(ROUTES, subverb) {
            Some(route) => route,
            None => return ExecResult::err(route::miss(descent.level, subverb)),
        },
        Stop::Bare => return ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
        _ => {
            return ExecResult::err(usage::unknown_subverb_message(
                descent.level,
                descent.typed.unwrap_or_default(),
            ))
        }
    };
    match route {
        Route::Default => {
            eff.side_effect = Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Default));
            eff.close_console = true;
            ExecResult::ok_msg("mode: returning to Default")
        }
        Route::Resize => super::rebuilt_by("mode resize", super::TRANSACTIONS_OWNER),
        Route::NodeEdit => super::rebuilt_by("mode node-edit", super::TEXT_OWNER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::parser::parse;
    use crate::application::console::parser::ParseResult;
    use crate::application::console::Args;
    use crate::application::document::MindMapDocument;

    /// Parse `line` and run the `mode` verb body against `doc`,
    /// returning the result, the side effect the dispatcher would
    /// consume, and whether the console closes.
    fn run_mode(line: &str, doc: &mut MindMapDocument) -> (ExecResult, Option<ConsoleSideEffect>, bool) {
        let tokens = match parse(line) {
            ParseResult::Ok { cmd: _, args } => args,
            ParseResult::Empty => panic!("empty input: {:?}", line),
            ParseResult::Unknown(s) => panic!("unknown command '{}' in {:?}", s, line),
        };
        let mut eff = ConsoleEffects::new(doc);
        let result = execute_mode(&Args::new(&tokens), &mut eff);
        let close = eff.close_console;
        let side = eff.side_effect.take();
        (result, side, close)
    }

    #[test]
    fn test_mode_default_emits_set_default_side_effect() {
        let mut doc = MindMapDocument::new_blank(None, 50);
        let (result, side, close) = run_mode("mode default", &mut doc);
        assert!(matches!(result, ExecResult::Ok { .. }));
        assert!(matches!(
            side,
            Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Default))
        ));
        assert!(close);
    }

    /// The modes an owner rebuilds refuse with the owner named, and
    /// request nothing of the dispatcher.
    #[test]
    fn test_mode_resize_and_node_edit_refuse_naming_their_owners() {
        for (line, owner) in [
            ("mode resize", super::super::TRANSACTIONS_OWNER),
            ("mode node-edit", super::super::TEXT_OWNER),
        ] {
            let mut doc = MindMapDocument::new_blank(None, 50);
            let (result, side, close) = run_mode(line, &mut doc);
            match result {
                ExecResult::Err(message) => assert!(message.contains(owner), "{line}: {message}"),
                other => panic!("{line} must refuse: {other:?}"),
            }
            assert!(side.is_none(), "{line}");
            assert!(!close, "{line}");
        }
    }

    #[test]
    fn test_mode_unknown_subverb_lists_the_three() {
        let mut doc = MindMapDocument::new_blank(None, 50);
        let (result, _, _) = run_mode("mode sideways", &mut doc);
        match result {
            ExecResult::Err(message) => {
                for word in ["default", "resize", "node-edit"] {
                    assert!(message.contains(word), "{message}");
                }
            }
            other => panic!("expected an error: {other:?}"),
        }
    }
}
