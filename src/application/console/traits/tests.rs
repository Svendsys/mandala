// SPDX-License-Identifier: MPL-2.0

//! Unit tests for the trait dispatcher's data types — color value
//! parsing, outcome helper, and selection materialization.

use super::*;
use crate::application::console::constants::{VAR_ACCENT, VAR_EDGE, VAR_FG};
use crate::application::document::SelectionState;

#[test]
fn test_parse_hex_ok() {
    assert_eq!(ColorValue::parse("#123").unwrap(), ColorValue::Hex("#123".into()));
    assert_eq!(
        ColorValue::parse("#009c15").unwrap(),
        ColorValue::Hex("#009c15".into())
    );
    assert_eq!(
        ColorValue::parse("#009c15ff").unwrap(),
        ColorValue::Hex("#009c15ff".into())
    );
}

#[test]
fn test_parse_hex_rejects_bad_length() {
    assert!(ColorValue::parse("#12").is_err());
    assert!(ColorValue::parse("#12345").is_err());
    assert!(ColorValue::parse("#zzzzzz").is_err());
}

#[test]
fn test_parse_var_tokens() {
    assert_eq!(ColorValue::parse("accent").unwrap(), ColorValue::Var(VAR_ACCENT));
    assert_eq!(ColorValue::parse("ACCENT").unwrap(), ColorValue::Var(VAR_ACCENT));
    assert_eq!(ColorValue::parse("fg").unwrap(), ColorValue::Var(VAR_FG));
    assert_eq!(ColorValue::parse("edge").unwrap(), ColorValue::Var(VAR_EDGE));
}

#[test]
fn test_parse_reset() {
    assert_eq!(ColorValue::parse("reset").unwrap(), ColorValue::Reset);
}

#[test]
fn test_parse_unknown_is_error() {
    assert!(ColorValue::parse("bogus").is_err());
}

#[test]
fn test_outcome_applied_helper() {
    assert_eq!(Outcome::applied(true), Outcome::Applied);
    assert_eq!(Outcome::applied(false), Outcome::Unchanged);
}

#[test]
fn test_selection_targets_for_each_variant() {
    use crate::application::document::EdgeRef;
    assert!(selection_targets(&SelectionState::None).is_empty());

    let ids = vec!["a".to_string(), "b".to_string()];
    let out = selection_targets(&SelectionState::Multi(ids.clone()));
    assert_eq!(out.len(), 2);

    let er = EdgeRef::new("a", "b", "cross_link");
    let out = selection_targets(&SelectionState::Edge(er));
    assert!(matches!(out.as_slice(), [TargetId::Edge(_)]));
}

#[test]
fn test_clipboard_content_variants() {
    let text = ClipboardContent::Text("#ff0000".into());
    assert!(matches!(text, ClipboardContent::Text(ref s) if s == "#ff0000"));

    let empty = ClipboardContent::Empty;
    assert!(matches!(empty, ClipboardContent::Empty));

    let na = ClipboardContent::NotApplicable;
    assert!(matches!(na, ClipboardContent::NotApplicable));
}

#[test]
fn test_clipboard_content_eq() {
    assert_eq!(
        ClipboardContent::Text("#abc".into()),
        ClipboardContent::Text("#abc".into()),
    );
    assert_ne!(
        ClipboardContent::Text("#abc".into()),
        ClipboardContent::Text("#def".into()),
    );
    assert_eq!(ClipboardContent::Empty, ClipboardContent::Empty);
    assert_eq!(ClipboardContent::NotApplicable, ClipboardContent::NotApplicable);
    assert_ne!(ClipboardContent::Empty, ClipboardContent::NotApplicable);
}

// ── apply_to_targets aggregation paths ─────────────────────────
//
// These pin the four outcome shapes the dispatcher folds into a
// `DispatchReport`: empty selection, all-NotApplicable, mixed
// Applied/Unchanged, and Invalid. The font command's
// happy-path / not-applicable cases live in `commands::font::tests`;
// here we cover the dispatcher mechanics directly via a stub
// closure that doesn't touch the doc.

use crate::application::document::MindMapDocument;

/// Load a fresh testament-map doc — same fixture pattern other
/// tests in this crate use. The closure-based op never reads the
/// doc, so any well-formed fixture works.
fn fresh_doc() -> MindMapDocument {
    let path = format!(
        "{}/maps/testament.mindmap.json",
        env!("CARGO_MANIFEST_DIR")
    );
    MindMapDocument::load(&path).expect("testament map loads")
}

#[test]
fn test_apply_to_targets_empty_selection_reports_no_target() {
    let mut doc = fresh_doc();
    doc.selection = SelectionState::None;
    let report = apply_to_targets(&mut doc, |_| Outcome::Applied);
    assert!(report.all_failed);
    assert!(!report.any_applied);
    assert_eq!(report.messages.len(), 1);
    assert!(report.messages[0].contains("no target"));
}

#[test]
fn test_apply_to_targets_multi_node_fanout_aggregates_applied() {
    let mut doc = fresh_doc();
    let ids: Vec<String> = doc.mindmap.nodes.keys().take(3).cloned().collect();
    assert_eq!(ids.len(), 3);
    doc.selection = SelectionState::Multi(ids);

    let mut calls = 0;
    let report = apply_to_targets(&mut doc, |_view| {
        calls += 1;
        Outcome::Applied
    });
    assert_eq!(calls, 3, "op called once per multi-selected node");
    assert!(report.any_applied);
    assert!(!report.all_failed);
    assert!(report.messages.is_empty());
}

#[test]
fn test_apply_to_targets_all_not_applicable_surfaces_label() {
    let mut doc = fresh_doc();
    let ids: Vec<String> = doc.mindmap.nodes.keys().take(2).cloned().collect();
    doc.selection = SelectionState::Multi(ids);
    let report = apply_to_targets(&mut doc, |_| Outcome::NotApplicable);
    assert!(!report.any_applied);
    assert!(report.all_failed);
    assert_eq!(report.messages.len(), 1);
    assert!(report.messages[0].contains("not applicable to nodes"));
}

#[test]
fn test_apply_to_targets_invalid_surfaces_per_target_messages() {
    let mut doc = fresh_doc();
    let ids: Vec<String> = doc.mindmap.nodes.keys().take(2).cloned().collect();
    doc.selection = SelectionState::Multi(ids);
    let mut idx = 0;
    let report = apply_to_targets(&mut doc, |_| {
        idx += 1;
        Outcome::Invalid(format!("bad-{}", idx))
    });
    assert!(!report.any_applied);
    assert!(report.all_failed);
    // Both invalid messages survive; order matches call order.
    assert_eq!(report.messages.len(), 2);
    assert!(report.messages.iter().any(|m| m.contains("bad-1")));
    assert!(report.messages.iter().any(|m| m.contains("bad-2")));
}

#[test]
fn test_apply_to_targets_mixed_applied_and_unchanged_succeeds_silently() {
    let mut doc = fresh_doc();
    let ids: Vec<String> = doc.mindmap.nodes.keys().take(3).cloned().collect();
    doc.selection = SelectionState::Multi(ids);
    let mut tick = 0;
    let report = apply_to_targets(&mut doc, |_| {
        tick += 1;
        if tick == 1 {
            Outcome::Applied
        } else {
            Outcome::Unchanged
        }
    });
    // At least one Applied → success, no "already set" message
    // (that's reserved for "every target reported Unchanged").
    assert!(report.any_applied);
    assert!(!report.all_failed);
    assert!(report.messages.is_empty());
}

#[test]
fn test_apply_to_targets_all_unchanged_reports_already_set() {
    let mut doc = fresh_doc();
    let ids: Vec<String> = doc.mindmap.nodes.keys().take(2).cloned().collect();
    doc.selection = SelectionState::Multi(ids);
    let report = apply_to_targets(&mut doc, |_| Outcome::Unchanged);
    // No Applied means `any_applied = false`; "already set" surfaces as a
    // success-with-message — `all_failed` stays false because at
    // least one pair (the unchanged path) succeeded.
    assert!(!report.any_applied);
    assert!(!report.all_failed);
    assert_eq!(report.messages.len(), 1);
    assert!(report.messages[0].contains("already set"));
}
