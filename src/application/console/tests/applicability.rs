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

/// The paths fixture (`app/dispatch/families/fixtures/paths.mandala.json`)
/// with node 1's link `nth` alone selected: link 0 is a line link to
/// node 3, link 2 a portal link to node 4.
fn paths_doc_with_link(nth: usize) -> crate::application::document::MindMapDocument {
    use baumhard::document::element::DocKind;
    use baumhard::document::ids::NodeIdent;
    let mut doc = crate::application::document::MindMapDocument::from_json_str(
        include_str!("../../app/dispatch/families/fixtures/paths.mandala.json"),
        None,
        baumhard::document::writer::DEFAULT_HISTORY_LIMIT,
    )
    .expect("the paths fixture loads");
    let node = doc.doc().node_by_ident(NodeIdent(1)).expect("node 1");
    let link = crate::application::view::inspect::children_of_kind(doc.doc(), node, DocKind::Link)
        .nth(nth)
        .expect("the link");
    doc.selection.select_only(link);
    doc
}

/// A portal link alone admits `link` and `portal`, and not `path` or
/// `label`: it draws no path, and its labels are its ends'. Control: a
/// line link alone admits `path`, `label` and `link`, and not `portal`.
#[test]
fn test_applicability_follows_a_links_kind() {
    let applicable = |doc: &crate::application::document::MindMapDocument, verb: &str| {
        (command_by_name(verb).unwrap().applicable)(&ConsoleContext::from_document(doc))
    };
    let portal = paths_doc_with_link(2);
    for (verb, offered) in [
        ("path", false),
        ("label", false),
        ("link", true),
        ("portal", true),
    ] {
        assert_eq!(
            applicable(&portal, verb),
            offered,
            "{verb} with a portal link alone"
        );
    }
    let line = paths_doc_with_link(0);
    for (verb, offered) in [("path", true), ("label", true), ("link", true), ("portal", false)] {
        assert_eq!(
            applicable(&line, verb),
            offered,
            "control: {verb} with a line link alone"
        );
    }
}

#[test]
fn test_applicability_help_always_true() {
    let doc = load_test_doc();
    let ctx = ConsoleContext::from_document(&doc);
    let cmd = command_by_name("help").unwrap();
    assert!((cmd.applicable)(&ctx));
}
