// SPDX-License-Identifier: MPL-2.0

//! `present start <id> [node-id …]`, `present stop <id> [node-id …]` and
//! `present list` — the console's door to presentations (design §10.2,
//! `format/presentations.md`).
//!
//! The verb only reads the line, into a [`PresentLine`] side effect:
//! the presentations live in the app's registry and the open map, an
//! instance lives on the app clock, and the executor
//! (`app::console_input::presentations`) has both.

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{free, kvs, usage, Form, Grammar, Slot, Subverb};
use crate::application::console::{ConsoleEffects, ConsoleSideEffect, ExecResult};

/// One `present` line, read: what the executor does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresentLine {
    /// Start the presentation `id` on the nodes with these ids, or on
    /// the selection when there are none.
    Start {
        /// The presentation's id.
        id: String,
        /// The node ids to start it on.
        nodes: Vec<u32>,
    },
    /// Stop the presentation `id` on the nodes with these ids, or on
    /// the selection when there are none.
    Stop {
        /// The presentation's id.
        id: String,
        /// The node ids to stop it on.
        nodes: Vec<u32>,
    },
    /// List the presentations that play on the open map, and the
    /// instances playing.
    List,
}

const SUBVERBS: &[Subverb] = &[
    Subverb::bare(
        "start",
        "presentations",
        "start a presentation on the selection, or on the nodes with these ids",
    )
    .taking(&[Form::slots(&[
        Slot::req(free("id")),
        Slot::opt(free("node-id …")),
    ])]),
    Subverb::bare(
        "stop",
        "presentations",
        "stop a presentation on the selection, or on the nodes with these ids: it plays its way out",
    )
    .taking(&[Form::slots(&[
        Slot::req(free("id")),
        Slot::opt(free("node-id …")),
    ])]),
    Subverb::bare(
        "list",
        "presentations",
        "list the presentations that play on this map, and the instances playing",
    ),
];

/// Which handler each declared subverb runs (`spec::route`).
#[derive(Clone, Copy)]
enum Route {
    Start,
    Stop,
    List,
}

const ROUTES: &Routes<Route> = &[
    ("start", Route::Start),
    ("stop", Route::Stop),
    ("list", Route::List),
];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub static GRAMMAR: Grammar = Grammar {
    label: "present",
    subverb_sets: &[SUBVERBS],
    key_sets: &[],
    bare: None,
};

pub const COMMAND: Command = Command {
    name: "present",
    aliases: &["presentation", "presentations"],
    summary:
        "Start, stop or list presentations: performances that never change the map (start / stop / list)",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["play", "perform", "animate", "reveal", "conceal", "pulse", "lift"],
    execute: execute_present,
};

fn execute_present(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
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
    // `start` and `stop` take any number of node ids, which the
    // grammar's slots cannot count; `list` is held to its slots.
    if matches!(route, Route::List) {
        if let Err(msg) = kvs::read_strict(&descent, args) {
            return ExecResult::err(msg);
        }
    }
    let on = |verb: &str| -> Result<(String, Vec<u32>), String> {
        let Some(id) = args.positional(1) else {
            return Err(format!(
                "present {verb}: which presentation? `present {verb} <id> [node-id …]`"
            ));
        };
        let mut nodes = Vec::new();
        for word in args.positionals().skip(2) {
            match word.parse::<u32>() {
                Ok(node) => nodes.push(node),
                Err(_) => return Err(format!("present {verb}: '{word}' is not a node id")),
            }
        }
        Ok((id.to_string(), nodes))
    };
    let line = match route {
        Route::Start => match on("start") {
            Ok((id, nodes)) => PresentLine::Start { id, nodes },
            Err(message) => return ExecResult::err(message),
        },
        Route::Stop => match on("stop") {
            Ok((id, nodes)) => PresentLine::Stop { id, nodes },
            Err(message) => return ExecResult::err(message),
        },
        Route::List => PresentLine::List,
    };
    eff.side_effect = Some(ConsoleSideEffect::Present(line));
    ExecResult::ok_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::load_test_doc;

    fn read(line: &str) -> Result<PresentLine, String> {
        let mut doc = load_test_doc();
        let tokens: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        let mut eff = ConsoleEffects::new(&mut doc);
        match (
            execute_present(&Args::new(&tokens), &mut eff),
            eff.side_effect.take(),
        ) {
            (ExecResult::Err(msg), _) => Err(msg),
            (_, Some(ConsoleSideEffect::Present(line))) => Ok(line),
            (other, effect) => panic!("{line}: {other:?} {effect:?}"),
        }
    }

    /// Each form reads into the line the executor runs, and a malformed
    /// one is refused before anything runs.
    #[test]
    fn test_present_lines_read_into_what_the_executor_runs() {
        assert_eq!(read("list"), Ok(PresentLine::List));
        assert_eq!(
            read("start lift"),
            Ok(PresentLine::Start {
                id: "lift".into(),
                nodes: vec![]
            })
        );
        assert_eq!(
            read("start concealed 3 7"),
            Ok(PresentLine::Start {
                id: "concealed".into(),
                nodes: vec![3, 7]
            })
        );
        assert_eq!(
            read("stop concealed 3"),
            Ok(PresentLine::Stop {
                id: "concealed".into(),
                nodes: vec![3]
            })
        );
        assert_eq!(
            read("stop lift"),
            Ok(PresentLine::Stop {
                id: "lift".into(),
                nodes: vec![]
            })
        );
        // Controls: what does not read is refused.
        assert!(read("start").is_err(), "an id is required");
        assert!(read("stop").is_err(), "an id is required");
        assert!(read("start lift 1.2").is_err(), "a node id is an integer");
        assert!(read("stop lift x").is_err(), "a node id is an integer");
        assert!(read("list extra").is_err(), "list takes nothing");
        assert!(read("frobnicate").is_err());
    }
}
