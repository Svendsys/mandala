// SPDX-License-Identifier: MPL-2.0

//! The document revision (`revision.rs`): what advances it, what
//! must not, and what it says about the funnel that advanced it.
//! The console grammar's half of the contract is
//! `console::tests::revision`; this is the document's own.

use super::tests_common::{first_testament_node_id, load_test_doc, pick_test_edge, TestNudgeMutation};
use super::*;

use baumhard::mindmap::animation::{AnimationTiming, Easing};
use baumhard::mindmap::custom_mutation::TargetScope;

#[test]
fn test_a_fresh_document_is_at_revision_zero_and_generation_zero() {
    let doc = load_test_doc();
    assert_eq!(doc.revision(), 0);
    assert_eq!(doc.generation(), 0);
    assert!(!doc.dirty);
    assert_eq!(doc.last_change_source(), ChangeSource::Input);
}

#[test]
fn test_mark_dirty_sets_the_flag_and_advances_the_revision_once() {
    let mut doc = load_test_doc();
    doc.mark_dirty();
    assert!(doc.dirty);
    assert_eq!(doc.revision(), 1);
    doc.mark_dirty();
    assert_eq!(doc.revision(), 2, "every call is a change");
}

#[test]
fn test_a_setter_advances_the_revision_and_a_no_op_setter_does_not() {
    let mut doc = load_test_doc();
    let (edge, _) = pick_test_edge(&doc);
    assert!(doc.set_edge_color(&edge, Some("#123456")));
    assert_eq!(doc.revision(), 1);
    assert!(
        !doc.set_edge_color(&edge, Some("#123456")),
        "same value: nothing to do"
    );
    assert_eq!(doc.revision(), 1, "a no-op commits nothing");
}

#[test]
fn test_the_innermost_funnel_names_the_source_and_driving_is_handed_back() {
    let mut doc = load_test_doc();
    let outer = doc.enter_source(ChangeSource::Action);
    assert_eq!(outer, ChangeSource::Input, "nothing was driving");
    doc.mark_dirty();
    assert_eq!(doc.last_change_source(), ChangeSource::Action);

    doc.with_source(ChangeSource::Console, |doc| doc.mark_dirty());
    assert_eq!(doc.last_change_source(), ChangeSource::Console);

    doc.mark_dirty();
    assert_eq!(
        doc.last_change_source(),
        ChangeSource::Action,
        "the inner funnel returned driving to the outer one"
    );
    doc.leave_source(outer);
    doc.mark_dirty();
    assert_eq!(doc.last_change_source(), ChangeSource::Input);
}

#[test]
fn test_undo_advances_the_revision_without_dirtying_and_names_itself() {
    let mut doc = load_test_doc();
    doc.create_orphan_and_select(glam::Vec2::new(10.0, 20.0));
    let after_edit = doc.revision();
    assert_eq!(after_edit, 1);
    doc.dirty = false;

    let outer = doc.enter_source(ChangeSource::Action);
    assert!(doc.undo());
    doc.leave_source(outer);
    assert_eq!(doc.revision(), after_edit + 1);
    assert_eq!(
        doc.last_change_source(),
        ChangeSource::Undo,
        "an undo reached through an Action is still an undo"
    );
    assert!(
        !doc.dirty,
        "undo advances the revision and leaves the unsaved flag alone"
    );
}

#[test]
fn test_an_undo_with_nothing_to_undo_changes_nothing() {
    let mut doc = load_test_doc();
    assert!(!doc.undo());
    assert_eq!(doc.revision(), 0);
}

#[test]
fn test_previews_and_selection_leave_the_revision_alone() {
    let mut doc = load_test_doc();
    let node = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(node.clone());
    doc.selection = SelectionState::None;
    assert!(doc.set_rect_select_preview(vec![node.clone()]));
    doc.take_rect_select_preview();
    doc.label_edit_preview = None;
    assert_eq!(doc.revision(), 0);
    assert!(!doc.dirty);
}

#[test]
fn test_a_custom_mutation_advances_the_revision_as_a_mutation() {
    let mut doc = load_test_doc();
    let node = first_testament_node_id(&doc);
    let cm = TestNudgeMutation::new("nudge", TargetScope::SelfOnly)
        .magnitude(10.0)
        .build();
    let outer = doc.enter_source(ChangeSource::Action);
    let mut tree = doc.build_tree();
    doc.apply_custom_mutation(&cm, &node, Some(&mut tree));
    doc.leave_source(outer);
    assert!(doc.revision() > 0, "a custom mutation is a committed change");
    assert_eq!(doc.last_change_source(), ChangeSource::Mutation);
}

#[test]
fn test_an_animated_mutation_advances_the_revision_when_it_commits_not_when_it_starts() {
    let mut doc = load_test_doc();
    let node = first_testament_node_id(&doc);
    let cm = TestNudgeMutation::new("slide", TargetScope::SelfOnly)
        .magnitude(100.0)
        .timing(AnimationTiming {
            duration_ms: 500,
            delay_ms: 0,
            easing: Easing::Linear,
            then: None,
        })
        .build();
    doc.start_animation(&cm, &node, 0);
    assert_eq!(doc.revision(), 0, "a started animation has changed nothing yet");

    doc.tick_animations(250, None);
    let midway = doc.revision();
    doc.tick_animations(1_000, None);
    assert!(
        doc.revision() > midway,
        "the commit at the end of the animation is the change"
    );
    assert!(!doc.has_active_animations());
    assert_eq!(doc.last_change_source(), ChangeSource::Mutation);
}

#[test]
fn test_a_successor_continues_the_counter_and_moves_the_generation_on() {
    let mut old = load_test_doc();
    old.mark_dirty();
    old.mark_dirty();
    let mut next = load_test_doc();
    next.succeed(&old);
    assert_eq!(next.revision(), 3, "the swap is one more change");
    assert_eq!(next.generation(), 1);
    assert!(!next.dirty, "a replacement is not unsaved work");

    let mut third = load_test_doc();
    third.succeed(&next);
    assert_eq!((third.revision(), third.generation()), (4, 2));
}

/// **No setter sets the unsaved flag around the revision.**
/// `mark_dirty` is the one place that writes `dirty = true`, which is
/// what makes the revision complete without a second call beside each
/// setter. The compiler cannot hold that — the field is public, for
/// the tests and for the `dirty` readers — so the production sources
/// do.
#[test]
fn test_no_production_code_sets_dirty_directly() {
    use baumhard::util::rust_source::production_code;
    use std::path::Path;

    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let root = baumhard::util::doc_fixtures::repo_path("src");
    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut offenders = Vec::new();
    let mut scanned = 0;
    for path in files {
        let relative = path
            .strip_prefix(root.parent().expect("src has a parent"))
            .expect("under the repo")
            .to_string_lossy()
            .to_string();
        let name = path.file_name().expect("file").to_string_lossy();
        // Whole-file test modules: out-of-line `mod tests;` siblings.
        if name.starts_with("test") || relative.contains("/tests/") || relative.ends_with("revision.rs") {
            continue;
        }
        scanned += 1;
        for (i, line) in production_code(&relative).lines().enumerate() {
            let code = line.trim();
            if code.contains(".dirty = true") || code.contains(".dirty |= true") {
                offenders.push(format!("{relative}:{}", i + 1));
            }
        }
    }
    assert!(scanned > 100, "the walk found only {scanned} production sources");
    assert!(
        offenders.is_empty(),
        "these set `dirty` without advancing the revision; call `mark_dirty()`:\n  {}",
        offenders.join("\n  ")
    );
}
