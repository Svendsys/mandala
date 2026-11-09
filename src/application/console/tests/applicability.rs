// SPDX-License-Identifier: MPL-2.0

//! Per-command `applicable` predicate tests — verifies that commands
//! hide / show themselves correctly based on the current selection.

use super::fixtures::{load_test_doc, select_first_path};
use crate::application::console::commands::command_by_name;
use crate::application::console::ConsoleContext;

#[test]
fn test_applicability_anchor_hidden_without_a_path() {
    let doc = load_test_doc();
    let ctx = ConsoleContext::from_document(&doc);
    let cmd = command_by_name("anchor").unwrap();
    assert!(!(cmd.applicable)(&ctx));
}

#[test]
fn test_applicability_anchor_visible_with_a_path() {
    let mut doc = load_test_doc();
    let _ = select_first_path(&mut doc);
    let ctx = ConsoleContext::from_document(&doc);
    let cmd = command_by_name("anchor").unwrap();
    assert!((cmd.applicable)(&ctx));
}

#[test]
fn test_applicability_help_always_true() {
    let doc = load_test_doc();
    let ctx = ConsoleContext::from_document(&doc);
    let cmd = command_by_name("help").unwrap();
    assert!((cmd.applicable)(&ctx));
}
