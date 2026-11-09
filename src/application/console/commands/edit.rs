// SPDX-License-Identifier: MPL-2.0

//! `edit list`, `edit apply <id> [node-id …]`, `edit help <id>` and
//! `edit inspect <id>` — the console's door to edits (design §10.1,
//! `format/edits.md`).
//!
//! The verb only reads the line, into an [`EditLine`] side effect: the
//! edits live in the app's registry and the open map, and the executor
//! (`app::console_input::edits`) runs `apply` through the funnel every
//! route takes (`dispatch::families::edits::run_edit`), with the app
//! clock an animated edit runs on.

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{free, kvs, usage, Form, Grammar, Slot, Subverb};
use crate::application::console::{ConsoleEffects, ConsoleSideEffect, ExecResult};

/// One `edit` line, read: what the executor does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditLine {
    /// List every edit that runs on the open map.
    List,
    /// Run the edit `id` at the nodes with these ids, or at the
    /// selection when there are none.
    Apply {
        /// The edit's id.
        id: String,
        /// The node ids to anchor it at.
        nodes: Vec<u32>,
    },
    /// Describe the edit `id`.
    Help(String),
    /// Say what the edit `id` would do at the selection.
    Inspect(String),
}

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("list", "edits", "list the edits that run on this map"),
    Subverb::bare(
        "apply",
        "edits",
        "run an edit at the selection, or at the nodes with these ids",
    )
    .taking(&[Form::slots(&[
        Slot::req(free("id")),
        Slot::opt(free("node-id …")),
    ])]),
    Subverb::bare("help", "edits", "describe an edit and where it comes from")
        .taking(&[Form::slots(&[Slot::req(free("id"))])]),
    Subverb::bare(
        "inspect",
        "edits",
        "show what an edit would reach at the selection, and its body",
    )
    .taking(&[Form::slots(&[Slot::req(free("id"))])]),
];

/// Which handler each declared subverb runs (`spec::route`).
#[derive(Clone, Copy)]
enum Route {
    List,
    Apply,
    Help,
    Inspect,
}

const ROUTES: &Routes<Route> = &[
    ("list", Route::List),
    ("apply", Route::Apply),
    ("help", Route::Help),
    ("inspect", Route::Inspect),
];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub static GRAMMAR: Grammar = Grammar {
    label: "edit",
    subverb_sets: &[SUBVERBS],
    key_sets: &[],
    bare: None,
};

pub const COMMAND: Command = Command {
    name: "edit",
    aliases: &["edits"],
    summary: "List, run, describe or inspect the map's edits (list / apply / help / inspect)",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["flower", "cascade", "layout", "grow", "shrink", "nudge"],
    execute: execute_edit,
};

fn execute_edit(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
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
    // `apply` takes any number of node ids, which the grammar's slots
    // cannot count; every other form is held to its slots.
    if !matches!(route, Route::Apply) {
        if let Err(msg) = kvs::read_strict(&descent, args) {
            return ExecResult::err(msg);
        }
    }
    let id = || args.positional(1).map(str::to_string);
    let line = match route {
        Route::List => EditLine::List,
        Route::Apply => {
            let Some(id) = id() else {
                return ExecResult::err("edit apply: which edit? `edit apply <id> [node-id …]`");
            };
            let mut nodes = Vec::new();
            for word in args.positionals().skip(2) {
                match word.parse::<u32>() {
                    Ok(node) => nodes.push(node),
                    Err(_) => return ExecResult::err(format!("edit apply: '{word}' is not a node id")),
                }
            }
            EditLine::Apply { id, nodes }
        }
        Route::Help => match id() {
            Some(id) => EditLine::Help(id),
            None => return ExecResult::err("edit help: which edit? `edit help <id>`"),
        },
        Route::Inspect => match id() {
            Some(id) => EditLine::Inspect(id),
            None => return ExecResult::err("edit inspect: which edit? `edit inspect <id>`"),
        },
    };
    eff.side_effect = Some(ConsoleSideEffect::Edit(line));
    ExecResult::ok_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::load_test_doc;

    fn read(line: &str) -> Result<EditLine, String> {
        let mut doc = load_test_doc();
        let tokens: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        let mut eff = ConsoleEffects::new(&mut doc);
        match (
            execute_edit(&Args::new(&tokens), &mut eff),
            eff.side_effect.take(),
        ) {
            (ExecResult::Err(msg), _) => Err(msg),
            (_, Some(ConsoleSideEffect::Edit(line))) => Ok(line),
            (other, effect) => panic!("{line}: {other:?} {effect:?}"),
        }
    }

    /// Each form reads into the line the executor runs, and a malformed
    /// one is refused before anything runs.
    #[test]
    fn test_edit_lines_read_into_what_the_executor_runs() {
        assert_eq!(read("list"), Ok(EditLine::List));
        assert_eq!(
            read("apply flower"),
            Ok(EditLine::Apply {
                id: "flower".into(),
                nodes: vec![]
            })
        );
        assert_eq!(
            read("apply nudge-right-50 3 7"),
            Ok(EditLine::Apply {
                id: "nudge-right-50".into(),
                nodes: vec![3, 7]
            })
        );
        assert_eq!(read("help cascade"), Ok(EditLine::Help("cascade".into())));
        assert_eq!(read("inspect cascade"), Ok(EditLine::Inspect("cascade".into())));
        // Controls: what does not read is refused.
        assert!(read("apply").is_err(), "an id is required");
        assert!(read("apply flower 1.2").is_err(), "a node id is an integer");
        assert!(read("help").is_err(), "an id is required");
        assert!(read("list extra").is_err(), "list takes nothing");
        assert!(read("frobnicate").is_err());
    }
}
