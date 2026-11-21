// SPDX-License-Identifier: MPL-2.0

//! `verb list` and `verb run <id> [node-id]` — the console's door to the
//! map's verbs (design §11; `format/verbs.md`).
//!
//! The verb only reads the line, into a [`VerbLine`] side effect: a map
//! verb runs on the app clock, and its `edit` steps through the edits'
//! funnel, which the executor (`app::console_input::verbs`) has.
//!
//! "Verb" names two things here: this console verb, and the map verbs it
//! lists and runs (design §11.3 "map verb").

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{free, kvs, usage, Form, Grammar, Slot, Subverb};
use crate::application::console::{ConsoleEffects, ConsoleSideEffect, ExecResult};

/// One `verb` line, read: what the executor does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerbLine {
    /// Run the map verb `id`, on the node with the id `node`, or on none.
    Run {
        /// The map verb's id.
        id: String,
        /// The id of the node to run it on — its `self`.
        node: Option<u32>,
    },
    /// List the map's verbs, each marked as a change of the map or a
    /// performance, and the runs active.
    List,
}

const SUBVERBS: &[Subverb] = &[
    Subverb::bare(
        "run",
        "map verbs",
        "run one of the map's verbs, on the node with this id or on none",
    )
    .taking(&[Form::slots(&[Slot::req(free("id")), Slot::opt(free("node-id"))])]),
    Subverb::bare(
        "list",
        "map verbs",
        "list the map's verbs — which change the map, which perform it — and the runs active",
    ),
];

/// Which handler each declared subverb runs (`spec::route`).
#[derive(Clone, Copy)]
enum Route {
    Run,
    List,
}

const ROUTES: &Routes<Route> = &[("run", Route::Run), ("list", Route::List)];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub static GRAMMAR: Grammar = Grammar {
    label: "verb",
    subverb_sets: &[SUBVERBS],
    key_sets: &[],
    bare: None,
};

pub const COMMAND: Command = Command {
    name: "verb",
    aliases: &["verbs"],
    summary: "Run or list the map's verbs: its interface, which triggers and links invoke (run / list)",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["invoke", "trigger", "interface"],
    execute: execute_verb,
};

fn execute_verb(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = descend(&GRAMMAR, args.tokens());
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
    if let Err(msg) = kvs::read_strict(&descent, args) {
        return ExecResult::err(msg);
    }
    let line = match route {
        Route::List => VerbLine::List,
        Route::Run => {
            let Some(id) = args.positional(1) else {
                return ExecResult::err("verb run: which verb? `verb run <id> [node-id]`");
            };
            let node = match args.positional(2) {
                None => None,
                Some(word) => match word.parse::<u32>() {
                    Ok(node) => Some(node),
                    Err(_) => return ExecResult::err(format!("verb run: '{word}' is not a node id")),
                },
            };
            VerbLine::Run {
                id: id.to_string(),
                node,
            }
        }
    };
    eff.side_effect = Some(ConsoleSideEffect::Verb(line));
    ExecResult::ok_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::load_test_doc;

    fn read(line: &str) -> Result<VerbLine, String> {
        let mut doc = load_test_doc();
        let tokens: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        let mut eff = ConsoleEffects::new(&mut doc);
        match (
            execute_verb(&Args::new(&tokens), &mut eff),
            eff.side_effect.take(),
        ) {
            (ExecResult::Err(msg), _) => Err(msg),
            (_, Some(ConsoleSideEffect::Verb(line))) => Ok(line),
            (other, effect) => panic!("{line}: {other:?} {effect:?}"),
        }
    }

    /// Each form reads into the line the executor runs, and a malformed
    /// one is refused before anything runs.
    #[test]
    fn test_verb_lines_read_into_what_the_executor_runs() {
        assert_eq!(read("list"), Ok(VerbLine::List));
        assert_eq!(
            read("run begin"),
            Ok(VerbLine::Run {
                id: "begin".into(),
                node: None
            })
        );
        assert_eq!(
            read("run next-1 4"),
            Ok(VerbLine::Run {
                id: "next-1".into(),
                node: Some(4)
            })
        );
        // Controls: what does not read is refused.
        assert!(read("run").is_err(), "an id is required");
        assert!(read("run go x").is_err(), "a node id is an integer");
        assert!(read("run go 1 2").is_err(), "one node at most");
        assert!(read("list extra").is_err(), "list takes nothing");
        assert!(read("frobnicate").is_err());
    }
}
