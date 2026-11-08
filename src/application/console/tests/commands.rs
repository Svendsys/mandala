// SPDX-License-Identifier: MPL-2.0

//! Per-command execution tests for the verbs that run in V1-C3: each
//! test runs a single command line through `parse → execute` and
//! asserts what it reports. The verbs an owner rebuilds are pinned in
//! `commands::tests` and the oracle.

use super::fixtures::{load_test_doc, run};
use crate::application::console::ExecResult;

#[test]
fn test_help_no_args_returns_lines() {
    let mut doc = load_test_doc();
    match run("help", &mut doc) {
        ExecResult::Lines(lines) => assert!(lines.len() > 1),
        other => panic!("expected Lines, got {:?}", other),
    }
}

#[test]
fn test_help_with_known_command_prints_usage() {
    let mut doc = load_test_doc();
    match run("help anchor", &mut doc) {
        ExecResult::Lines(lines) => {
            assert!(lines.iter().any(|l| l.text.contains("anchor")));
            assert!(lines.iter().any(|l| l.text.contains("usage:")));
        }
        other => panic!("expected Lines, got {:?}", other),
    }
}

#[test]
fn test_help_unknown_command_reports_error() {
    let mut doc = load_test_doc();
    assert!(matches!(run("help nope", &mut doc), ExecResult::Err(_)));
}

/// The deleted verbs are gone: their names parse as unknown commands.
#[test]
fn test_the_verbs_whose_replacement_is_named_differently_are_gone() {
    for verb in ["border", "canvas", "edge", "mutation"] {
        assert!(
            matches!(
                crate::application::console::parse(verb),
                crate::application::console::ParseResult::Unknown(_)
            ),
            "`{verb}` must no longer be a console verb"
        );
    }
}
