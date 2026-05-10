// SPDX-License-Identifier: MPL-2.0

//! Shared fixtures for the console test suite. Used by every sibling
//! test module via `use super::fixtures::*;`.

use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::{EdgeRef, MindMapDocument, SelectionState};

// Re-export the canonical testament-map loader so console tests
// inherit the cached, finalize-skipped doc from
// `document::tests_common`. Until this re-export landed, the
// console suite hand-built its own doc shell + reloaded the JSON
// every test, thrashing the FONT_SYSTEM lock.
//
// Visibility: `first_node_id` is widened to
// `pub(in crate::application::console)` so the per-command test
// modules under `console::commands::*` can reach it through the
// same path that surfaces `run`. `load_test_doc` stays narrower
// because the per-command modules already pull it through
// `document::tests_common::load_test_doc as fixture_doc` directly.
pub(super) use crate::application::document::tests_common::load_test_doc;
pub(in crate::application::console) use crate::application::document::tests_common::{
    first_testament_node_id as first_node_id, two_testament_node_ids,
};

/// Collapse a slice of `OutputLine` values into one `\n`-joined
/// `String`. Used by the substring assertions across the console
/// command tests — every callers does the same `.iter().map(|l|
/// l.text...).collect().join("\n")` chain otherwise.
pub(in crate::application::console) fn join_lines(
    lines: &[crate::application::console::OutputLine],
) -> String {
    lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Assert `result` is `ExecResult::Ok(_)` or `Lines(_)`,
/// panicking on `Err(_)`. Reach for `assert_exec_ok_strict`
/// when the test cares that the result is a single-line `Ok`
/// specifically (no auto-promote hint, no custom-preset hint).
pub(in crate::application::console) fn assert_exec_ok(result: ExecResult) {
    match result {
        ExecResult::Ok(_) | ExecResult::Lines(_) => {}
        other => panic!("expected Ok / Lines, got {:?}", other),
    }
}

/// Strict-`Ok` flavour: rejects `Lines` so a test that means
/// "no hint should fire" can pin the absence of the hint
/// instead of accepting any successful result.
pub(in crate::application::console) fn assert_exec_ok_strict(result: ExecResult) {
    match result {
        ExecResult::Ok(_) => {}
        other => panic!("expected single-line Ok (strict), got {:?}", other),
    }
}

/// Assert `result` is `ExecResult::Err(_)` whose message contains
/// `needle`. Surfaces both halves of the assertion in the panic
/// message so a substring drift doesn't print just `false`.
pub(in crate::application::console) fn assert_exec_err_contains(result: ExecResult, needle: &str) {
    match result {
        ExecResult::Err(s) => assert!(
            s.contains(needle),
            "expected Err containing {:?}, got Err({:?})",
            needle,
            s
        ),
        other => panic!("expected Err containing {:?}, got {:?}", needle, other),
    }
}

/// Pick the first edge in the map and point the selection at it.
/// Returns the edge ref so tests can assert against the mutated
/// fields afterwards.
pub(super) fn select_first_edge(doc: &mut MindMapDocument) -> EdgeRef {
    let edge = doc.mindmap.edges[0].clone();
    let er = EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type);
    doc.selection = SelectionState::Edge(er.clone());
    er
}

/// Parse `line`, run the resolved command against `doc`, and return
/// the `ExecResult`. Panics on parse failure — these are unit tests
/// with known-good input.
pub(in crate::application::console) fn run(line: &str, doc: &mut MindMapDocument) -> ExecResult {
    let (cmd, tokens) = match parse(line) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        ParseResult::Empty => panic!("empty input: {:?}", line),
        ParseResult::Unknown(s) => panic!("unknown command '{}' in {:?}", s, line),
    };
    let mut eff = ConsoleEffects::new(doc);
    (cmd.execute)(&Args::new(&tokens), &mut eff)
}
