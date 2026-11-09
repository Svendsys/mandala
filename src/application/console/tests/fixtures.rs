// SPDX-License-Identifier: MPL-2.0

//! Shared fixtures for the console test suite.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;

use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::selection::every_node;
use crate::application::document::MindMapDocument;
use crate::application::view::inspect::children_of_kind;

/// The testament map: the console suite's one fixture.
pub(super) fn load_test_doc() -> MindMapDocument {
    crate::application::document::tests_common::testament()
}

/// Assert `result` is `ExecResult::Err(_)` whose message contains
/// `needle`. Surfaces both halves of the assertion in the panic message
/// so a substring drift doesn't print just `false`.
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

/// Select the map's first connector, and return it.
pub(super) fn select_first_path(doc: &mut MindMapDocument) -> NodeId {
    let path = every_node(doc.doc())
        .find_map(|n| children_of_kind(doc.doc(), n, DocKind::Connector).next())
        .expect("the testament map has a connector");
    doc.selection.select_only(path);
    path
}

/// Parse `line`, run the resolved command against `doc`, and return the
/// `ExecResult`. Panics on parse failure — these are unit tests with
/// known-good input.
pub(in crate::application::console) fn run(line: &str, doc: &mut MindMapDocument) -> ExecResult {
    let (cmd, tokens) = match parse(line) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        ParseResult::Empty => panic!("empty input: {:?}", line),
        ParseResult::Unknown(s) => panic!("unknown command '{}' in {:?}", s, line),
    };
    let mut eff = ConsoleEffects::new(doc);
    (cmd.execute)(&Args::new(&tokens), &mut eff)
}
