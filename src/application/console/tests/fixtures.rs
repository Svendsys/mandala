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
pub(super) use crate::application::document::tests_common::load_test_doc;

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
pub(super) fn run(line: &str, doc: &mut MindMapDocument) -> ExecResult {
    let (cmd, tokens) = match parse(line) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        ParseResult::Empty => panic!("empty input: {:?}", line),
        ParseResult::Unknown(s) => panic!("unknown command '{}' in {:?}", s, line),
    };
    let mut eff = ConsoleEffects::new(doc);
    (cmd.execute)(&Args::new(&tokens), &mut eff)
}
