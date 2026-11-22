// SPDX-License-Identifier: MPL-2.0

//! Per-command `applicable` predicate tests — verifies that commands
//! hide / show themselves correctly based on the current selection.

use super::fixtures::{load_test_doc, select_first_path};
use crate::application::console::commands::command_by_name;
use crate::application::console::ConsoleContext;

#[test]
fn test_applicability_path_hidden_without_a_path() {
    let doc = load_test_doc();
    let ctx = ConsoleContext::from_document(&doc);
    let cmd = command_by_name("path").unwrap();
    assert!(!(cmd.applicable)(&ctx));
}

#[test]
fn test_applicability_path_visible_with_a_path() {
    let mut doc = load_test_doc();
    let _ = select_first_path(&mut doc);
    let ctx = ConsoleContext::from_document(&doc);
    let cmd = command_by_name("path").unwrap();
    assert!((cmd.applicable)(&ctx));
}

/// `link` and `portal` want a link: a connector, always a line (D22),
/// does not admit them, where it does admit `path` (the control, in the
/// test above).
#[test]
fn test_applicability_link_verbs_hidden_with_a_connector_alone() {
    let mut doc = load_test_doc();
    let _ = select_first_path(&mut doc);
    let ctx = ConsoleContext::from_document(&doc);
    for verb in ["link", "portal"] {
        let cmd = command_by_name(verb).unwrap();
        assert!(!(cmd.applicable)(&ctx), "{verb} is hidden for a connector");
    }
}

#[test]
fn test_applicability_help_always_true() {
    let doc = load_test_doc();
    let ctx = ConsoleContext::from_document(&doc);
    let cmd = command_by_name("help").unwrap();
    assert!((cmd.applicable)(&ctx));
}
