// SPDX-License-Identifier: MPL-2.0

//! Node text / background / border / text-colour / font-size setters + set_node_style_field helper.
//!
//! Part of the tests split for `document`. Helpers live in
//! `tests_common`; only the tests for this theme live here.
use super::tests_common::{first_testament_node_id, load_test_doc};
use super::*;

use baumhard::mindmap::model::{MindNode, MindSection, NodeLayout, NodeStyle, Position, Size, TextRun};
use baumhard::util::grapheme_chad::count_grapheme_clusters;

#[test]
fn test_set_node_text_updates_text_and_collapses_runs() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let changed = doc.set_node_text(&nid, "Hello world".to_string());
    assert!(changed);
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(node.sections[0].text, "Hello world");
    assert_eq!(node.sections[0].text_runs.len(), 1);
    assert_eq!(node.sections[0].text_runs[0].start, 0);
    assert_eq!(
        node.sections[0].text_runs[0].end,
        count_grapheme_clusters("Hello world")
    );
    assert!(doc.dirty);
    assert!(matches!(
        doc.undo_stack.last(),
        Some(UndoAction::EditNodeText { .. })
    ));
}

/// `set_section_text(node, idx, text)` writes through to the
/// requested section — section 0 gets the same behaviour as the
/// pre-section `set_node_text`, sections 1+ stay untouched
/// unless explicitly targeted. Pins the section-aware setter's
/// addressing for the per-section text-edit path.
#[test]
fn test_set_section_text_targets_specific_section() {
    use baumhard::mindmap::model::MindSection;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    // Materialise a multi-section node by appending a second
    // section to the existing testament root.
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections
            .push(MindSection::new_default("second".into(), vec![]));
    }
    doc.undo_stack.clear();
    doc.dirty = false;

    // Edit section 1 only — section 0 must stay untouched.
    let s0_before = doc.mindmap.nodes.get(&nid).unwrap().sections[0].text.clone();
    assert!(doc.set_section_text(&nid, 1, "rewrote section 1".to_string()));
    let n = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(n.sections[0].text, s0_before, "section 0 untouched");
    assert_eq!(n.sections[1].text, "rewrote section 1");
    // Undo restores both sections.
    assert!(doc.undo());
    let n = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(n.sections[1].text, "second");
}

/// §T1 Unicode-edge: `set_section_text` round-trips ZWJ-emoji,
/// combining marks, and flag emoji byte-for-byte; the auto-
/// regenerated text-run's `end` matches grapheme-cluster count
/// (not codepoint or byte count). Catches the
/// `count_grapheme_clusters` accidentally being swapped for
/// `chars().count()` or `len()` — a regression that would
/// silently truncate emoji text on the next render.
#[test]
fn test_set_section_text_grapheme_handling_for_emoji_and_combining() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let zwj = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let combining = "e\u{0301}";
    let flag = "\u{1F1EF}\u{1F1F5}";
    let combined = format!("{zwj} {combining} {flag}");
    assert!(doc.set_section_text(&nid, 0, combined.clone()));
    let n = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(n.sections[0].text, combined, "text round-trips byte-for-byte");
    let cluster_count = count_grapheme_clusters(&combined);
    assert!(
        n.sections[0].text_runs.iter().all(|r| r.end <= cluster_count),
        "every run.end must fit within the {} grapheme clusters",
        cluster_count
    );
    // Tightened: every run.end must EQUAL the cluster count
    // (not just `<=`), so a regression that emits zero runs or
    // truncates the auto-collapsed run by even one grapheme
    // trips the test. The `<=` form would silently pass a
    // dropped trailing emoji.
    let runs = &n.sections[0].text_runs;
    assert!(!runs.is_empty(), "auto-collapsed run must exist");
    assert_eq!(runs[0].start, 0, "auto-collapsed run starts at grapheme index 0");
    assert_eq!(
        runs[0].end, cluster_count,
        "auto-collapsed run ends at the cluster count, not the codepoint or byte count"
    );
}

// ── Section offset / size setters ──────────────────────────────
//
// Validation rules + rejection messages mirror
// `crates/maptool/src/verify/sections.rs`. Shared fixture lives
// at `tests_common::pinned_two_section_node`.

#[test]
fn test_set_section_offset_writes_and_round_trips_through_undo() {
    use super::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    assert_eq!(doc.set_section_offset(&id, 1, 20.0, 25.0), Ok(true));
    let s = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
    assert_eq!(s.offset.x, 20.0);
    assert_eq!(s.offset.y, 25.0);
    assert!(doc.undo());
    let restored = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
    assert_eq!(restored.offset.x, 10.0, "undo restores prior offset");
    assert_eq!(restored.offset.y, 10.0);
}

#[test]
fn test_set_section_offset_idempotent_no_op() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    assert_eq!(doc.set_section_offset(&id, 1, 10.0, 10.0), Ok(false));
    assert!(doc.undo_stack.is_empty(), "no-op must not push undo");
    assert!(!doc.dirty);
}

#[test]
fn test_set_section_offset_rejects_nan_and_inf() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    assert!(doc
        .set_section_offset(&id, 1, f64::NAN, 0.0)
        .is_err_and(|m| m.contains("non-finite")));
    assert!(doc
        .set_section_offset(&id, 1, f64::INFINITY, 0.0)
        .is_err_and(|m| m.contains("non-finite")));
}

#[test]
fn test_set_section_offset_rejects_negative_with_verify_mirror_message() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    assert!(doc
        .set_section_offset(&id, 1, -1.0, 0.0)
        .is_err_and(|m| m.contains("section[1].offset.x is negative")));
    assert!(doc
        .set_section_offset(&id, 1, 0.0, -2.0)
        .is_err_and(|m| m.contains("section[1].offset.y is negative")));
}

#[test]
fn test_set_section_offset_rejects_aabb_overflow_with_verify_mirror_message() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // section[1] size 50×30; offset (160,0) → right=210 > 200.
    assert!(doc
        .set_section_offset(&id, 1, 160.0, 0.0)
        .is_err_and(|m| m.contains("extends past node right edge")));
    // offset (0,80) → bottom=110 > 100.
    assert!(doc
        .set_section_offset(&id, 1, 0.0, 80.0)
        .is_err_and(|m| m.contains("extends past node bottom edge")));
}

#[test]
fn test_set_section_offset_unknown_section_returns_false() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    assert_eq!(doc.set_section_offset(&id, 99, 0.0, 0.0), Ok(false));
}

#[test]
fn test_set_section_size_writes_and_round_trips_through_undo() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let new_size = Some(baumhard::mindmap::model::Size {
        width: 80.0,
        height: 40.0,
    });
    assert_eq!(doc.set_section_size(&id, 1, new_size.clone()), Ok(true));
    assert_eq!(doc.mindmap.nodes.get(&id).unwrap().sections[1].size, new_size);
    assert!(doc.undo());
    assert_eq!(
        doc.mindmap.nodes.get(&id).unwrap().sections[1]
            .size
            .as_ref()
            .unwrap()
            .width,
        50.0,
        "undo restores prior size"
    );
}

#[test]
fn test_set_section_size_none_restores_fill_parent() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    assert_eq!(doc.set_section_size(&id, 1, None), Ok(true));
    assert!(doc.mindmap.nodes.get(&id).unwrap().sections[1].size.is_none());
}

#[test]
fn test_set_section_size_rejects_zero_and_negative_with_verify_mirror_message() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let zero = Some(baumhard::mindmap::model::Size {
        width: 0.0,
        height: 30.0,
    });
    assert!(doc
        .set_section_size(&id, 1, zero)
        .is_err_and(|m| m.contains("size.width is not positive")));
    let neg = Some(baumhard::mindmap::model::Size {
        width: 30.0,
        height: -5.0,
    });
    assert!(doc
        .set_section_size(&id, 1, neg)
        .is_err_and(|m| m.contains("size.height is not positive")));
}

#[test]
fn test_set_section_size_rejects_overflow_with_verify_mirror_message() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // Offset (10,10) + width 200 = 210 > node.size.width 200.
    let overflow = Some(baumhard::mindmap::model::Size {
        width: 200.0,
        height: 30.0,
    });
    assert!(doc
        .set_section_size(&id, 1, overflow)
        .is_err_and(|m| m.contains("extends past node right edge")));
}

#[test]
fn test_set_section_size_rejects_astronomical_with_verify_mirror_message() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // node 200×100, 100× = 20000. 25000 trips the typo guard.
    let huge = Some(baumhard::mindmap::model::Size {
        width: 25000.0,
        height: 30.0,
    });
    assert!(doc
        .set_section_size(&id, 1, huge)
        .is_err_and(|m| m.contains("over 100× the node's width")));
}

#[test]
fn test_set_section_size_idempotent_no_op() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let same = Some(baumhard::mindmap::model::Size {
        width: 50.0,
        height: 30.0,
    });
    assert_eq!(doc.set_section_size(&id, 1, same), Ok(false));
    assert!(doc.undo_stack.is_empty(), "no-op must not push undo");
}

/// `set_section_offset` rejects non-zero offset on a `None`-
/// sized (fill-parent) section — the section's effective size
/// is `node.size`, so any non-zero offset stretches past the
/// node's right / bottom edge. Mirrors the verify rule.
#[test]
fn test_set_section_offset_rejects_nonzero_on_none_sized_section() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // Section[0] is None-sized (fill-parent); the fixture only
    // pins section[1]'s explicit Some-size.
    assert!(
        doc.mindmap.nodes[&id].sections[0].size.is_none(),
        "fixture invariant"
    );
    let result = doc.set_section_offset(&id, 0, 5.0, 0.0);
    assert!(result.is_err_and(|m| m.contains("extends past node right edge")));
}

#[test]
fn test_set_section_offset_accepts_zero_on_none_sized_section() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    assert!(
        doc.mindmap.nodes[&id].sections[0].size.is_none(),
        "fixture invariant"
    );
    // Already at (0,0) → no-op false; not an error.
    let result = doc.set_section_offset(&id, 0, 0.0, 0.0);
    assert_eq!(result, Ok(false));
}

// ── set_section_aabb (atomic offset+size for the resize gesture) ──

/// `set_section_aabb` accepts a W-grow gesture's final state —
/// section pinned at `offset.x = 90` with `size.width = 10` inside
/// a 100-wide node, gesture shrinks `offset.x` to 85 and grows
/// `size.width` to 15. Atomic validation against the **post-
/// mutation** AABB passes. The pre-fix `set_section_size` then
/// `set_section_offset` two-step rejected this transition because
/// `set_section_size(15)` validated against the *unchanged*
/// `offset.x = 90`, computing `right = 90 + 15 = 105 > 100`.
#[test]
fn test_set_section_aabb_accepts_w_grow_against_right_edge() {
    use baumhard::mindmap::model::{Position, Size};
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // Reposition section[1] to be flush against the right edge.
    {
        let node = doc.mindmap.nodes.get_mut(&id).unwrap();
        node.sections[1].offset = Position { x: 90.0, y: 10.0 };
        node.sections[1].size = Some(Size {
            width: 10.0,
            height: 30.0,
        });
    }
    doc.undo_stack.clear();
    doc.dirty = false;
    // W-grow: offset.x 90 → 85, size.width 10 → 15. Right edge
    // stays at 100.
    let result = doc.set_section_aabb(
        &id,
        1,
        Position { x: 85.0, y: 10.0 },
        Size {
            width: 15.0,
            height: 30.0,
        },
    );
    assert_eq!(result, Ok(true));
    let n = &doc.mindmap.nodes[&id];
    assert_eq!(n.sections[1].offset.x, 85.0);
    assert_eq!(n.sections[1].size.as_ref().unwrap().width, 15.0);
}

/// `set_section_aabb` rejects post-mutation overflow with the
/// verify-mirror message — same shape as `set_section_size` /
/// `set_section_offset`.
#[test]
fn test_set_section_aabb_rejects_post_mutation_overflow() {
    use baumhard::mindmap::model::{Position, Size};
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let result = doc.set_section_aabb(
        &id,
        1,
        Position { x: 50.0, y: 10.0 },
        Size {
            width: 200.0,
            height: 30.0,
        },
    );
    assert!(result.is_err_and(|m| m.contains("extends past node right edge")));
}

#[test]
fn test_set_section_aabb_rejects_negative_offset() {
    use baumhard::mindmap::model::{Position, Size};
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let result = doc.set_section_aabb(
        &id,
        1,
        Position { x: -5.0, y: 10.0 },
        Size {
            width: 50.0,
            height: 30.0,
        },
    );
    assert!(result.is_err_and(|m| m.contains("offset.x is negative")));
}

#[test]
fn test_set_section_aabb_rejects_non_positive_size() {
    use baumhard::mindmap::model::{Position, Size};
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let result = doc.set_section_aabb(
        &id,
        1,
        Position { x: 10.0, y: 10.0 },
        Size {
            width: 0.0,
            height: 30.0,
        },
    );
    assert!(result.is_err_and(|m| m.contains("is not positive")));
}

#[test]
fn test_set_section_aabb_idempotent_no_op() {
    use baumhard::mindmap::model::{Position, Size};
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let same_offset = Position { x: 10.0, y: 10.0 };
    let same_size = Size {
        width: 50.0,
        height: 30.0,
    };
    let undo_before = doc.undo_stack.len();
    assert_eq!(doc.set_section_aabb(&id, 1, same_offset, same_size), Ok(false));
    assert_eq!(doc.undo_stack.len(), undo_before, "no-op must not push undo");
}

#[test]
fn test_set_section_aabb_writes_through_one_undo_entry() {
    use baumhard::mindmap::model::{Position, Size};
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    doc.undo_stack.clear();
    let result = doc.set_section_aabb(
        &id,
        1,
        Position { x: 20.0, y: 15.0 },
        Size {
            width: 40.0,
            height: 25.0,
        },
    );
    assert_eq!(result, Ok(true));
    assert_eq!(doc.undo_stack.len(), 1, "one undo entry per atomic AABB write");
}

// ── Auto-fit on Some-sized sections ────────────────────────────
//
// `grow_one_node_to_fit_text` contributes the larger of measured
// text and (when set) user-pinned size to the floor — user intent
// survives when text fits, text overflow still grows the parent.

#[test]
fn test_auto_fit_some_sized_section_grows_parent_when_text_overflows() {
    use super::grow_one_node_to_fit_text;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections.clear();
        node.sections
            .push(baumhard::mindmap::model::MindSection::new_default(
                "x".repeat(500),
                Vec::new(),
            ));
        node.sections[0].size = Some(baumhard::mindmap::model::Size {
            width: 10.0,
            height: 10.0,
        });
        node.size.width = 10.0;
        node.size.height = 10.0;
    }
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    grow_one_node_to_fit_text(node);
    // 500 'x' characters at 14pt should produce a measured text
    // block of hundreds of pixels (well over 100). A regression
    // that drops "grow to fit text" to "grow by 1 unit per call"
    // would pass the loose `> 10.0` form; this lower bound traps
    // it.
    assert!(
        node.size.width >= 100.0,
        "500-char text must grow parent past 100; got {}",
        node.size.width
    );
}

#[test]
fn test_auto_fit_some_sized_section_keeps_user_size_when_text_fits() {
    use super::grow_one_node_to_fit_text;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections.clear();
        node.sections
            .push(baumhard::mindmap::model::MindSection::new_default(
                String::new(),
                Vec::new(),
            ));
        node.sections[0].size = Some(baumhard::mindmap::model::Size {
            width: 200.0,
            height: 80.0,
        });
        node.size.width = 50.0;
        node.size.height = 50.0;
    }
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    grow_one_node_to_fit_text(node);
    assert!(
        node.size.width >= 200.0,
        "user-pinned section size must pull the parent floor up: width={}",
        node.size.width
    );
    assert!(
        node.size.height >= 80.0,
        "user-pinned section size must pull the parent floor up: height={}",
        node.size.height
    );
}

/// `Some`-sized section where text *also* fits within the user
/// size: ensures the floor takes max(text, user-size) — a
/// regression that always picks user-size and ignores text would
/// pass the previous two tests but fail here.
#[test]
fn test_auto_fit_some_sized_section_text_dominates_when_larger() {
    use super::grow_one_node_to_fit_text;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections.clear();
        node.sections
            .push(baumhard::mindmap::model::MindSection::new_default(
                "x".repeat(500),
                Vec::new(),
            ));
        // User pinned 50×50, but text needs much more — text
        // wins.
        node.sections[0].size = Some(baumhard::mindmap::model::Size {
            width: 50.0,
            height: 50.0,
        });
        node.size.width = 50.0;
        node.size.height = 50.0;
    }
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    grow_one_node_to_fit_text(node);
    assert!(
        node.size.width >= 100.0,
        "text must dominate the floor when larger than user size; got {}",
        node.size.width
    );
}

#[test]
fn test_auto_fit_none_sized_section_unchanged_regression() {
    use super::grow_one_node_to_fit_text;
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections.clear();
        node.sections
            .push(baumhard::mindmap::model::MindSection::new_default(
                "x".repeat(200),
                Vec::new(),
            ));
        node.sections[0].size = None;
        node.size.width = 10.0;
        node.size.height = 10.0;
    }
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    grow_one_node_to_fit_text(node);
    assert!(
        node.size.width >= 100.0,
        "None-sized section auto-fit must grow parent past 100; got {}",
        node.size.width
    );
}

/// Out-of-range section index is a no-op — neither push undo
/// nor flip dirty. Mirrors `set_node_text` no-op contract.
#[test]
fn test_set_section_text_out_of_range_is_noop() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(!doc.set_section_text(&nid, 99, "nope".to_string()));
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

#[test]
fn test_set_node_text_noop_on_unchanged() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let current = doc.mindmap.nodes.get(&nid).unwrap().sections[0].text.clone();
    doc.undo_stack.clear();
    doc.dirty = false;
    let changed = doc.set_node_text(&nid, current);
    assert!(!changed);
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

#[test]
fn test_set_node_text_undo_round_trip() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let before_text = doc.mindmap.nodes.get(&nid).unwrap().sections[0].text.clone();
    let before_runs_len = doc.mindmap.nodes.get(&nid).unwrap().sections[0].text_runs.len();
    let before_first_run_color = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
        .text_runs
        .first()
        .map(|r| r.color.clone());
    assert!(doc.set_node_text(&nid, "mutated".to_string()));
    assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().sections[0].text, "mutated");
    assert!(doc.undo());
    let restored = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(restored.sections[0].text, before_text);
    // TextRun doesn't implement PartialEq, so compare the parts
    // we care about: count + first run's color.
    assert_eq!(restored.sections[0].text_runs.len(), before_runs_len);
    assert_eq!(
        restored.sections[0].text_runs.first().map(|r| r.color.clone()),
        before_first_run_color
    );
}

#[test]
fn test_set_node_text_multiline_with_newlines() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    assert!(doc.set_node_text(&nid, "line 1\nline 2\nline 3".to_string()));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(node.sections[0].text, "line 1\nline 2\nline 3");
    // Collapsed single run spans the full char count, including newlines.
    assert_eq!(node.sections[0].text_runs.len(), 1);
    assert_eq!(
        node.sections[0].text_runs[0].end,
        count_grapheme_clusters("line 1\nline 2\nline 3")
    );
}

#[test]
fn test_set_node_text_unknown_id_returns_false() {
    let mut doc = load_test_doc();
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(!doc.set_node_text("nonexistent-id", "x".to_string()));
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

#[test]
fn test_set_node_text_inherits_first_run_formatting() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    // Force a specific first-run formatting we can check for.
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        if node.sections[0].text_runs.is_empty() {
            let end = count_grapheme_clusters(&node.sections[0].text);
            node.sections[0].text_runs.push(TextRun {
                start: 0,
                end,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".to_string(),
                size_pt: 24,
                color: "#ffffff".to_string(),
                hyperlink: None,
            });
        }
        node.sections[0].text_runs[0].bold = true;
        node.sections[0].text_runs[0].color = "#abcdef".to_string();
        node.sections[0].text_runs[0].size_pt = 33;
    }
    assert!(doc.set_node_text(&nid, "rewritten".to_string()));
    let run = &doc.mindmap.nodes.get(&nid).unwrap().sections[0].text_runs[0];
    assert!(run.bold);
    assert_eq!(run.color, "#abcdef");
    assert_eq!(run.size_pt, 33);
}

// -----------------------------------------------------------------
// Node style setters (bg / border / text color, font size)
// -----------------------------------------------------------------

#[test]
fn test_set_node_bg_color_round_trips_through_undo() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let before = doc
        .mindmap
        .nodes
        .get(&nid)
        .unwrap()
        .style
        .background_color
        .clone();
    assert!(doc.set_node_bg_color(&nid, "#123456".to_string()));
    assert_eq!(
        doc.mindmap.nodes.get(&nid).unwrap().style.background_color,
        "#123456"
    );
    assert!(matches!(
        doc.undo_stack.last(),
        Some(UndoAction::EditNodeStyle { .. })
    ));
    assert!(doc.undo());
    assert_eq!(
        doc.mindmap.nodes.get(&nid).unwrap().style.background_color,
        before
    );
}

#[test]
fn test_set_node_bg_color_unchanged_is_noop() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let current = doc
        .mindmap
        .nodes
        .get(&nid)
        .unwrap()
        .style
        .background_color
        .clone();
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(!doc.set_node_bg_color(&nid, current));
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

#[test]
fn test_set_node_border_color_writes_frame_color() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    assert!(doc.set_node_border_color(&nid, "#ff00ff".to_string()));
    assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().style.frame_color, "#ff00ff");
}

/// First-edit materialization of `node.style.border` uses
/// `default_glyph_border_config()` (private to `nodes/border.rs`).
/// Pin the resulting `preset` to `"light"` so a regression to
/// `"rounded"` — the previous default — surfaces here. The
/// trigger is any kv edit that *touches a config field*; we
/// use `padding=` because it's a leaf field with no other
/// behaviour entanglement.
#[test]
fn test_default_border_config_first_edit_materialises_light_preset() {
    use crate::application::document::{BorderConfigEdits, OptionEdit};
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    // Strip any pre-existing per-node border so we exercise the
    // `get_or_insert_with(default_glyph_border_config)` path.
    doc.mindmap.nodes.get_mut(&nid).unwrap().style.border = None;
    let mut edits = BorderConfigEdits::default();
    edits.padding = OptionEdit::Set(8.0);
    let outcome = doc.set_node_border_config(&nid, edits);
    assert!(outcome.changed);
    let cfg = doc
        .mindmap
        .nodes
        .get(&nid)
        .unwrap()
        .style
        .border
        .as_ref()
        .expect("first-edit materialised the per-node config");
    assert_eq!(cfg.preset, "light");
}

/// Setting text color rewrites `style.text_color` and every run
/// whose color matched the pre-edit default. A run the user
/// colored by hand (mismatched) keeps its override.
#[test]
fn test_set_node_text_color_preserves_per_run_overrides() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    // Seed the node with a known default and two runs: one
    // matching the default, one hand-colored. Pin
    // `sections[0].text` to a string of known grapheme count so
    // the runs (`0..3`, `3..6`) survive the `clamp_runs_to_text`
    // pass `set_node_text_color` runs — without this, the second
    // run gets dropped when `first_testament_node_id` happens to
    // pick a node whose section text is shorter than 4 graphemes
    // (HashMap iteration order varies per process, so the test
    // was intermittently flaky).
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections[0].text = "abcdef".into();
        node.style.text_color = "#dddddd".into();
        node.sections[0].text_runs = vec![
            TextRun {
                start: 0,
                end: 3,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 24,
                color: "#dddddd".into(), // matches default
                hyperlink: None,
            },
            TextRun {
                start: 3,
                end: 6,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 24,
                color: "#abcdef".into(), // user override
                hyperlink: None,
            },
        ];
    }
    assert!(doc.set_node_text_color(&nid, "#111111".into()));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(node.style.text_color, "#111111");
    assert_eq!(
        node.sections[0].text_runs[0].color, "#111111",
        "default-following run should update"
    );
    assert_eq!(
        node.sections[0].text_runs[1].color, "#abcdef",
        "per-run override should be preserved"
    );
}

#[test]
fn test_set_node_text_color_round_trips_through_undo() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.style.text_color = "#dddddd".into();
        for run in node.sections[0].text_runs.iter_mut() {
            run.color = "#dddddd".into();
        }
    }
    let before_default = doc.mindmap.nodes.get(&nid).unwrap().style.text_color.clone();
    let before_run_colors: Vec<String> = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
        .text_runs
        .iter()
        .map(|r| r.color.clone())
        .collect();
    assert!(doc.set_node_text_color(&nid, "#222222".into()));
    assert!(doc.undo());
    let restored = doc.mindmap.nodes.get(&nid).unwrap();
    assert_eq!(restored.style.text_color, before_default);
    let restored_colors: Vec<String> = restored.sections[0]
        .text_runs
        .iter()
        .map(|r| r.color.clone())
        .collect();
    assert_eq!(restored_colors, before_run_colors);
}

#[test]
fn test_set_node_font_size_writes_all_runs_and_round_trips() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let before_sizes: Vec<u32> = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
        .text_runs
        .iter()
        .map(|r| r.size_pt)
        .collect();
    assert!(doc.set_node_font_size(&nid, 48.0));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(node.sections[0].text_runs.iter().all(|r| r.size_pt == 48));
    assert!(doc.undo());
    let after_sizes: Vec<u32> = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
        .text_runs
        .iter()
        .map(|r| r.size_pt)
        .collect();
    assert_eq!(after_sizes, before_sizes);
}

#[test]
fn test_set_node_font_size_clamps_below_one() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    assert!(doc.set_node_font_size(&nid, 0.5));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(node.sections[0].text_runs.iter().all(|r| r.size_pt == 1));
}

#[test]
fn test_set_node_style_unknown_id_returns_false() {
    let mut doc = load_test_doc();
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(!doc.set_node_bg_color("nope", "#000".into()));
    assert!(!doc.set_node_border_color("nope", "#000".into()));
    assert!(!doc.set_node_text_color("nope", "#000".into()));
    assert!(!doc.set_node_font_size("nope", 10.0));
    assert!(!doc.set_node_font_family("nope", Some("Norse")));
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

#[test]
fn test_set_node_font_family_writes_all_runs_and_round_trips() {
    baumhard::font::fonts::init();
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let before_fonts: Vec<String> = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
        .text_runs
        .iter()
        .map(|r| r.font.clone())
        .collect();
    // Pick a loaded family that doesn't already match every
    // existing run — keeps the test self-healing against
    // future fixture changes.
    let target = baumhard::font::fonts::loaded_families_iter()
        .find(|f| !before_fonts.iter().any(|b| b == f))
        .map(str::to_string)
        .expect("at least one loaded family must differ from the fixture");
    assert!(doc.set_node_font_family(&nid, Some(&target)));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(node.sections[0].text_runs.iter().all(|r| r.font == target));
    // Idempotent re-set is a no-op.
    let stack_len = doc.undo_stack.len();
    assert!(!doc.set_node_font_family(&nid, Some(&target)));
    assert_eq!(doc.undo_stack.len(), stack_len);
    // Undo restores the prior heterogeneous state.
    assert!(doc.undo());
    let after_fonts: Vec<String> = doc.mindmap.nodes.get(&nid).unwrap().sections[0]
        .text_runs
        .iter()
        .map(|r| r.font.clone())
        .collect();
    assert_eq!(after_fonts, before_fonts);
}

/// Pinning a wide-advance face on a node previously sized for
/// a narrow monospace must grow the box so the new text fits.
/// The setter calls `grow_one_node_to_fit_text` after mutating
/// the runs; before the fix, font changes left the rect at its
/// prior size and the new text overflowed the right edge.
#[test]
fn test_set_node_font_family_grows_node_to_fit_new_face() {
    baumhard::font::fonts::init();
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);

    // Shrink the node *below* its measured floor so the per-edit
    // re-fit has something concrete to grow back. Note: the
    // production loader's `grow_node_sizes_to_fit_text` would
    // never leave a node this small, but the test fixture is
    // already loaded so we shrink in place to set up the
    // measurement.
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    node.size.width = 1.0;
    node.size.height = 1.0;

    // Use whatever family the fixture already references so the
    // setter doesn't bail out as "already". If the fixture's
    // first run carries the empty sentinel, pin to a real
    // family instead.
    let pin = baumhard::font::fonts::loaded_families_iter()
        .next()
        .map(str::to_string)
        .expect("at least one loaded family");
    assert!(doc.set_node_font_family(&nid, Some(&pin)));

    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(
        node.size.width > 1.0 && node.size.height > 1.0,
        "set_node_font_family must re-fit the node box; got {}×{}",
        node.size.width,
        node.size.height
    );
}

/// `set_node_font_size` likewise has to re-fit — the same
/// regression as the family case, just driven by the size
/// channel.
#[test]
fn test_set_node_font_size_grows_node_to_fit_new_size() {
    baumhard::font::fonts::init();
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    node.size.width = 1.0;
    node.size.height = 1.0;
    // Pick a size different from whatever the fixture's first
    // run uses so the setter actually applies. 96 pt is well
    // above any default.
    assert!(doc.set_node_font_size(&nid, 96.0));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(
        node.size.width > 1.0 && node.size.height > 1.0,
        "set_node_font_size must re-fit the node box; got {}×{}",
        node.size.width,
        node.size.height
    );
}

/// Pinning a wide display face must measure with that face, not
/// cosmic-text's default monospace. Pre-fix,
/// `measure_text_block_unbounded` shaped with `Attrs::new()`
/// regardless of the run's `font` field, so a node pinned to a
/// wide face under-measured by 30–60% and the box undersized.
/// This test compares the floor reached by two consecutive
/// font-family pins on the same fixture node — one to a face
/// with a known wide advance, one to a known narrow face — and
/// asserts the wide-face floor is strictly larger. If the
/// measurement reverts to font-blind, both pins land at the
/// monospace floor and the assertion fires.
#[test]
fn test_set_node_font_family_wide_face_grows_more_than_narrow() {
    baumhard::font::fonts::init();
    // Strategy: shape "MMMMMMMM" through every loaded face,
    // pick the narrowest and widest measured advance, and
    // compare the two floors. This is fixture-resilient — we
    // don't rely on any particular family being bundled, just
    // on at least two faces having distinct advances (which is
    // the case for the >40 bundled families).
    let families: Vec<String> = baumhard::font::fonts::loaded_families_iter()
        .map(str::to_string)
        .collect();
    if families.len() < 2 {
        // Not enough variety to discriminate; skip without
        // failing the suite.
        return;
    }

    // Measure each family's advance for "MMMMMMMM" at 14 pt;
    // pick narrowest and widest. Skip families that resolve to
    // None for app_font_by_family (shouldn't happen given the
    // iter source, but defensive).
    let mut measurements: Vec<(String, f32)> = Vec::new();
    for fam in &families {
        let app_font = match baumhard::font::fonts::app_font_by_family(fam) {
            Some(f) => f,
            None => continue,
        };
        let mut fs = baumhard::font::fonts::acquire_font_system_write("tests::wide_vs_narrow_measure");
        let block = baumhard::font::fonts::measure_text_block_unbounded(
            &mut fs,
            "MMMMMMMM",
            14.0,
            16.8,
            Some(app_font),
        );
        drop(fs);
        measurements.push((fam.clone(), block.width));
    }
    measurements.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    if measurements.len() < 2 || measurements.first().unwrap().1 <= 0.0 {
        return;
    }
    let narrow_fam = measurements.first().unwrap().0.clone();
    let wide_fam = measurements.last().unwrap().0.clone();
    if (measurements.last().unwrap().1 - measurements.first().unwrap().1).abs() < 1.0 {
        // Insufficient spread — bundled set may be pathologically
        // uniform. Don't assert.
        return;
    }

    // Apply each family in turn to a fresh node and read the
    // resulting size.
    let measure_floor = |fam: &str| -> f64 {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.size.width = 1.0;
        node.size.height = 1.0;
        assert!(doc.set_node_font_family(&nid, Some(fam)));
        doc.mindmap.nodes.get(&nid).unwrap().size.width
    };

    let narrow_floor = measure_floor(&narrow_fam);
    let wide_floor = measure_floor(&wide_fam);
    assert!(
        wide_floor > narrow_floor,
        "wide face '{}' floor ({}) should exceed narrow face '{}' floor ({}); \
             likely measure_text_block_unbounded reverted to font-blind",
        wide_fam,
        wide_floor,
        narrow_fam,
        narrow_floor
    );
}

/// `set_node_text` must re-fit on text change — pre-fix the
/// inline editor's commit path could overflow because the box
/// stayed at its prior size while the new text grew.
#[test]
fn test_set_node_text_grows_node_to_fit_longer_text() {
    baumhard::font::fonts::init();
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
    node.size.width = 1.0;
    node.size.height = 1.0;
    let long_text = "this is some text that is meaningfully longer than a tiny box".to_string();
    assert!(doc.set_node_text(&nid, long_text));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(
        node.size.width > 1.0 && node.size.height > 1.0,
        "set_node_text must re-fit the node box; got {}×{}",
        node.size.width,
        node.size.height
    );
}

#[test]
fn test_set_node_font_family_none_clears_every_run() {
    let mut doc = load_test_doc();
    let nid = first_testament_node_id(&doc);
    // Pin the runs to a known family first so the clear has
    // something to clear.
    baumhard::font::fonts::init();
    let target = baumhard::font::fonts::loaded_families_iter()
        .next()
        .map(str::to_string)
        .expect("at least one loaded family");
    assert!(doc.set_node_font_family(&nid, Some(&target)));
    // Now clear with None — every run should hold the empty
    // sentinel that the tree builder reads as "use default".
    assert!(doc.set_node_font_family(&nid, None));
    let node = doc.mindmap.nodes.get(&nid).unwrap();
    assert!(node.sections[0].text_runs.iter().all(|r| r.font.is_empty()));
    // Re-clear is a no-op.
    let stack_len = doc.undo_stack.len();
    assert!(!doc.set_node_font_family(&nid, None));
    assert_eq!(doc.undo_stack.len(), stack_len);
}

/// `grow_node_sizes_to_fit_borders` runs at finalize so a
/// map loaded with a wide static side pattern on a tiny node
/// grows the node automatically — the same monotonic posture
/// as `grow_node_sizes_to_fit_text`. Without this floor the
/// renderer would clip the static prefix at load time.
#[test]
fn finalize_grows_nodes_to_fit_border_static_parts() {
    use baumhard::mindmap::model::{Canvas, CustomBorderGlyphs, GlyphBorderConfig, MindMap};
    use std::collections::HashMap;

    let mut nodes = HashMap::new();
    let style = NodeStyle {
        background_color: "#000".into(),
        frame_color: "#fff".into(),
        text_color: "#fff".into(),
        shape: "rectangle".into(),
        corner_radius_percent: 0.0,
        frame_thickness: 1.0,
        show_frame: true,
        show_shadow: false,
        border: Some(GlyphBorderConfig {
            preset: "custom".into(),
            font: None,
            font_size_pt: 14.0,
            color: None,
            glyphs: Some(CustomBorderGlyphs {
                top: "##########(*)##########".into(),
                bottom: "-".into(),
                left: "|".into(),
                right: "|".into(),
                top_left: "<".into(),
                top_right: ">".into(),
                bottom_left: "<".into(),
                bottom_right: ">".into(),
            }),
            padding: 4.0,
            color_palette: None,
            color_palette_field: None,
        }),
    };
    nodes.insert(
        "0".into(),
        MindNode {
            id: "0".into(),
            parent_id: None,
            position: Position { x: 0.0, y: 0.0 },
            size: Size {
                width: 5.0,
                height: 5.0,
            },
            sections: vec![MindSection::new_default("n".into(), vec![])],
            style,
            layout: NodeLayout {
                layout_type: "map".into(),
                direction: "auto".into(),
                spacing: 0.0,
            },
            folded: false,
            notes: String::new(),
            color_schema: None,
            channel: 0,
            trigger_bindings: vec![],
            inline_mutations: vec![],
            inline_macros: Vec::new(),
            min_zoom_to_render: None,
            max_zoom_to_render: None,
        },
    );
    let map = MindMap {
        version: "1.0".into(),
        name: "fixture".into(),
        canvas: Canvas {
            background_color: "#000".into(),
            default_border: None,
            default_connection: None,
            theme_variables: HashMap::new(),
            theme_variants: HashMap::new(),
        },
        palettes: HashMap::new(),
        nodes,
        edges: vec![],
        custom_mutations: vec![],
        macros: vec![],
    };
    // Round-trip through JSON to exercise the finalize hook
    // — `MindMapDocument::from_json_str` calls `finalize`,
    // which runs both grow passes. Direct construction skips
    // it.
    let json = serde_json::to_string(&map).expect("serialises");
    let doc = MindMapDocument::from_json_str(&json, None).expect("loads through finalize");
    let n = doc.mindmap.nodes.get("0").expect("node 0 exists");
    assert!(
        n.size.width > 5.0,
        "load-time floor must grow the box to fit the border statics; \
             got width={}",
        n.size.width,
    );
}
