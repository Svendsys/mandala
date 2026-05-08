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
    // Flatten-to-fill-parent is only legal at offset (0, 0)
    // post the C3 effective-size fix; the fixture pins
    // section[1] at offset (10, 10), so reset before flattening.
    {
        let node = doc.mindmap.nodes.get_mut(&id).unwrap();
        node.sections[1].offset = baumhard::mindmap::model::Position { x: 0.0, y: 0.0 };
    }
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

/// Symmetric height-axis pin for the astronomical-typo guard —
/// ensures both width and height branches are reached.
#[test]
fn test_set_section_size_rejects_astronomical_height_with_verify_mirror_message() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let huge = Some(baumhard::mindmap::model::Size {
        width: 30.0,
        height: 25000.0,
    });
    assert!(doc
        .set_section_size(&id, 1, huge)
        .is_err_and(|m| m.contains("over 100× the node's height")));
}

/// Non-finite size component rejected by `validate_section_aabb`.
/// Pin both width and height branches so a regression that drops
/// one ships visibly.
#[test]
fn test_set_section_size_rejects_non_finite_components() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    let nan_w = Some(baumhard::mindmap::model::Size {
        width: f64::NAN,
        height: 30.0,
    });
    assert!(doc
        .set_section_size(&id, 1, nan_w)
        .is_err_and(|m| m.contains("size has non-finite component")));
    let inf_h = Some(baumhard::mindmap::model::Size {
        width: 30.0,
        height: f64::INFINITY,
    });
    assert!(doc
        .set_section_size(&id, 1, inf_h)
        .is_err_and(|m| m.contains("size has non-finite component")));
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

/// `set_section_size(None)` rejects when the section's existing
/// offset is non-zero — flatten-to-fill-parent on a section
/// pinned at `(5, 0)` would produce an effective AABB
/// `((5, 0), node.size)` that overflows the parent's right
/// edge. Closes the symmetric hole to `set_section_offset`'s
/// effective-size check.
#[test]
fn test_set_section_size_rejects_none_when_offset_nonzero() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // Move section[1] so it has an explicit non-zero offset
    // *and* an explicit size that fits at that offset.
    {
        let node = doc.mindmap.nodes.get_mut(&id).unwrap();
        node.sections[1].offset = baumhard::mindmap::model::Position { x: 5.0, y: 0.0 };
        node.sections[1].size = Some(baumhard::mindmap::model::Size {
            width: 50.0,
            height: 30.0,
        });
    }
    // Flatten to fill-parent — effective AABB becomes
    // ((5, 0), (200, 100)) — right=205 > node 200.
    assert!(doc
        .set_section_size(&id, 1, None)
        .is_err_and(|m| m.contains("extends past node right edge")));
}

/// `set_section_size(None)` accepts when offset is `(0, 0)` —
/// the canonical fill-parent shape.
#[test]
fn test_set_section_size_accepts_none_at_zero_offset() {
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    {
        let node = doc.mindmap.nodes.get_mut(&id).unwrap();
        node.sections[1].offset = baumhard::mindmap::model::Position { x: 0.0, y: 0.0 };
    }
    assert_eq!(doc.set_section_size(&id, 1, None), Ok(true));
    assert!(doc.mindmap.nodes[&id].sections[1].size.is_none());
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

// ── set_node_size / set_node_aabb (atomic node resize) ───────────

#[test]
fn test_set_node_size_writes_and_round_trips_through_undo() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    let before = doc.mindmap.nodes[&id].size;
    // Use a target large enough to fit any reasonable testament-
    // node text floor — `grow_one_node_to_fit_text` runs after
    // the setter and would bump a small target up to the text
    // floor, masking the round-trip pin.
    let target = Size {
        width: 800.0,
        height: 400.0,
    };
    assert_eq!(doc.set_node_size(&id, target), Ok(true));
    let after = doc.mindmap.nodes[&id].size;
    assert_eq!(after.width, 800.0);
    assert_eq!(after.height, 400.0);
    assert!(doc.undo());
    assert_eq!(doc.mindmap.nodes[&id].size, before, "undo restores prior size");
}

/// Setter applies `grow_one_node_to_fit_text` after the size
/// write, so a request below the measured-text floor lands at
/// the floor rather than the requested value.
#[test]
fn test_set_node_size_below_text_floor_lands_at_floor() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    let tiny = Size {
        width: 5.0,
        height: 5.0,
    };
    assert_eq!(doc.set_node_size(&id, tiny), Ok(true));
    let after = doc.mindmap.nodes[&id].size;
    // Both axes must clear the requested tiny floor — a
    // regression that grows only one axis is the exact bug
    // shape we're guarding against.
    assert!(
        after.width > 5.0 && after.height > 5.0,
        "floor-respect must grow both axes above the tiny target ({}x{})",
        after.width, after.height
    );
}

#[test]
fn test_set_node_size_idempotent_no_op() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // Land at a known size above the text floor first — the
    // post-grow no-op gate compares the post-mutation size
    // against the pre-mutation size, so the second call must
    // match the post-grow shape of the first.
    let target = Size {
        width: 800.0,
        height: 400.0,
    };
    assert_eq!(doc.set_node_size(&id, target), Ok(true));
    let undo_before = doc.undo_stack.len();
    // Second call with the same target — post-grow size will
    // match (no border-grow on this fixture's unframed root),
    // so the gate fires and we return Ok(false).
    assert_eq!(doc.set_node_size(&id, target), Ok(false));
    assert_eq!(doc.undo_stack.len(), undo_before);
}

#[test]
fn test_set_node_size_rejects_non_finite() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    assert!(doc
        .set_node_size(
            &id,
            Size {
                width: f64::NAN,
                height: 10.0
            }
        )
        .is_err_and(|m| m.contains("non-finite")));
    assert!(doc
        .set_node_size(
            &id,
            Size {
                width: 10.0,
                height: f64::INFINITY
            }
        )
        .is_err_and(|m| m.contains("non-finite")));
}

#[test]
fn test_set_node_size_rejects_non_positive() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    assert!(doc
        .set_node_size(
            &id,
            Size {
                width: 0.0,
                height: 10.0
            }
        )
        .is_err_and(|m| m.contains("is not positive")));
    assert!(doc
        .set_node_size(
            &id,
            Size {
                width: 10.0,
                height: -5.0
            }
        )
        .is_err_and(|m| m.contains("is not positive")));
}

/// Framed-node idempotency: `set_node_size` on a framed
/// node, where `grow_one_node_to_fit_border` inflates the
/// post-write size, must still no-op on a repeated identical
/// call. Pre-fix the no-op gate compared `new_size` against
/// pre-mutation `node.size`, missed on every post-first call,
/// and stacked `EditNodeAabb` undo entries.
#[test]
fn test_set_node_size_idempotent_on_framed_node() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    doc.mindmap.nodes.get_mut(&id).unwrap().style.show_frame = true;
    doc.undo_stack.clear();
    let target = Size {
        width: 800.0,
        height: 400.0,
    };
    assert_eq!(doc.set_node_size(&id, target), Ok(true));
    let after_first = doc.mindmap.nodes[&id].size;
    let undo_after_first = doc.undo_stack.len();
    // Second identical call must be a no-op even though the
    // border-grow likely inflated the post-write size past
    // `target`.
    assert_eq!(doc.set_node_size(&id, target), Ok(false));
    assert_eq!(
        doc.undo_stack.len(),
        undo_after_first,
        "framed-node set_node_size must not stack undo entries"
    );
    assert_eq!(doc.mindmap.nodes[&id].size, after_first);
}

/// Same framed-idempotency contract for `set_node_aabb`.
#[test]
fn test_set_node_aabb_idempotent_on_framed_node() {
    use baumhard::mindmap::model::{Position, Size};
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    doc.mindmap.nodes.get_mut(&id).unwrap().style.show_frame = true;
    doc.undo_stack.clear();
    let target_pos = Position { x: 100.0, y: 100.0 };
    let target_size = Size {
        width: 800.0,
        height: 400.0,
    };
    assert_eq!(doc.set_node_aabb(&id, target_pos, target_size), Ok(true));
    let after_first = doc.mindmap.nodes[&id].size;
    let undo_after_first = doc.undo_stack.len();
    assert_eq!(doc.set_node_aabb(&id, target_pos, target_size), Ok(false));
    assert_eq!(
        doc.undo_stack.len(),
        undo_after_first,
        "framed-node set_node_aabb must not stack undo entries"
    );
    assert_eq!(doc.mindmap.nodes[&id].size, after_first);
}

#[test]
fn test_set_node_size_rejects_astronomical_typo() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // Absolute ceiling at 1_000_000 — value past it trips the
    // typo guard. Independent of the prior-size baseline so a
    // tiny-to-large drag at the gesture's release-commit isn't
    // silently rejected.
    let huge = Size {
        width: 2_000_000.0,
        height: 10.0,
    };
    assert!(doc.set_node_size(&id, huge).is_err_and(|m| m.contains("exceeds the")));
}

/// `set_node_aabb` writes both fields atomically and pushes one
/// `EditNodeAabb` undo entry. Used by the resize gesture's
/// release-commit. Uses a target large enough to fit testament
/// text so the floor-respect pass leaves the requested size
/// untouched and the round-trip pin is exact.
#[test]
fn test_set_node_aabb_writes_position_and_size_atomically() {
    use baumhard::mindmap::model::{Position, Size};
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    let before_pos = doc.mindmap.nodes[&id].position;
    let before_size = doc.mindmap.nodes[&id].size;
    let new_pos = Position {
        x: before_pos.x + 10.0,
        y: before_pos.y + 5.0,
    };
    let new_size = Size {
        width: 800.0,
        height: 400.0,
    };
    let undo_before = doc.undo_stack.len();
    assert_eq!(doc.set_node_aabb(&id, new_pos, new_size), Ok(true));
    assert_eq!(doc.mindmap.nodes[&id].position, new_pos);
    assert_eq!(doc.mindmap.nodes[&id].size, new_size);
    assert_eq!(doc.undo_stack.len(), undo_before + 1);
    // Undo restores both.
    assert!(doc.undo());
    assert_eq!(doc.mindmap.nodes[&id].position, before_pos);
    assert_eq!(doc.mindmap.nodes[&id].size, before_size);
}

#[test]
fn test_set_node_aabb_idempotent_no_op() {
    use baumhard::mindmap::model::{Position, Size};
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // Land above the text floor first — same shape as the
    // sibling `set_node_size` idempotency test.
    let target_pos = Position { x: 100.0, y: 100.0 };
    let target_size = Size {
        width: 800.0,
        height: 400.0,
    };
    assert_eq!(doc.set_node_aabb(&id, target_pos, target_size), Ok(true));
    let undo_before = doc.undo_stack.len();
    assert_eq!(doc.set_node_aabb(&id, target_pos, target_size), Ok(false));
    assert_eq!(doc.undo_stack.len(), undo_before);
}

#[test]
fn test_set_node_aabb_accepts_negative_position() {
    use baumhard::mindmap::model::{Position, Size};
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // Nodes float freely on canvas; negative positions are legal.
    let result = doc.set_node_aabb(
        &id,
        Position { x: -50.0, y: -20.0 },
        Size {
            width: 60.0,
            height: 30.0,
        },
    );
    assert_eq!(result, Ok(true));
}

#[test]
fn test_set_node_aabb_rejects_non_finite_position() {
    use baumhard::mindmap::model::{Position, Size};
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    let result = doc.set_node_aabb(
        &id,
        Position { x: f64::NAN, y: 0.0 },
        Size {
            width: 60.0,
            height: 30.0,
        },
    );
    assert!(result.is_err_and(|m| m.contains("non-finite")));
}

// ── compute_one_node_text_floor (the shared floor helper) ────────

/// Pinned section size dominates measured text in the floor
/// computation. Locks the "size as floor" contract directly on
/// the helper, not just through its consumers.
#[test]
fn test_compute_one_node_text_floor_pinned_size_acts_as_floor() {
    use super::compute_one_node_text_floor;
    use baumhard::mindmap::model::Size;
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // Pin section[1] way past any text floor.
    doc.mindmap.nodes.get_mut(&id).unwrap().sections[1].size = Some(Size {
        width: 500.0,
        height: 200.0,
    });
    let node = &doc.mindmap.nodes[&id];
    let (w, h) = compute_one_node_text_floor(node);
    // section[1]'s offset+size = (10+500, 10+200) = (510, 210).
    assert!(w >= 510.0, "pinned width must propagate, got {}", w);
    assert!(h >= 210.0, "pinned height must propagate, got {}", h);
}

/// A non-finite section offset is skipped — the verifier flags
/// it elsewhere, and a NaN propagating into the floor would
/// corrupt every downstream `node.size` reader.
#[test]
fn test_compute_one_node_text_floor_skips_non_finite_offset() {
    use super::compute_one_node_text_floor;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    {
        let n = doc.mindmap.nodes.get_mut(&id).unwrap();
        n.sections[0].offset = baumhard::mindmap::model::Position {
            x: f64::NAN,
            y: 0.0,
        };
    }
    let (w, h) = compute_one_node_text_floor(&doc.mindmap.nodes[&id]);
    assert!(w.is_finite());
    assert!(h.is_finite());
}

// ── fit_node_to_content (auto-fit shrink path) ──────────────────

/// `fit_node_to_content` shrinks an over-sized node to its
/// measured-text floor and pushes one `EditNodeAabb` undo
/// entry. The path that lets users recover from a manual resize
/// that pinned the node larger than its content.
#[test]
fn test_fit_node_to_content_shrinks_to_floor() {
    use baumhard::mindmap::model::Size;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    doc.mindmap.nodes.get_mut(&id).unwrap().size = Size {
        width: 5000.0,
        height: 5000.0,
    };
    doc.undo_stack.clear();
    assert_eq!(doc.fit_node_to_content(&id), Ok(true));
    let after = doc.mindmap.nodes[&id].size;
    assert!(
        after.width < 5000.0 && after.height < 5000.0,
        "fit-to-content must shrink the node"
    );
    // Undo restores the prior (over-sized) state.
    assert!(doc.undo());
    assert_eq!(doc.mindmap.nodes[&id].size.width, 5000.0);
}

#[test]
fn test_fit_node_to_content_idempotent_no_op() {
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // First call lands at the floor.
    assert_eq!(doc.fit_node_to_content(&id), Ok(true));
    let undo_after_first = doc.undo_stack.len();
    // Second call is a no-op.
    assert_eq!(doc.fit_node_to_content(&id), Ok(false));
    assert_eq!(
        doc.undo_stack.len(),
        undo_after_first,
        "second fit-to-content must not push another undo entry"
    );
}

#[test]
fn test_fit_node_to_content_unknown_node_returns_false() {
    let mut doc = load_test_doc();
    assert_eq!(doc.fit_node_to_content("nope"), Ok(false));
}

#[test]
fn test_fit_node_to_content_pinned_section_size_acts_as_floor() {
    use baumhard::mindmap::model::Size;
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // section[1] is pinned at (10, 10) size 50×30 — its
    // contribution to the floor is offset+size = (60, 40).
    // The fit-to-content target is the max of the pinned-section
    // floor and section[0]'s text-driven size; assert that the
    // pinned axis floor survives. (Section[0] may pull width
    // past 60 via testament text, so we assert >= rather than
    // == on width; height has no large contributor in section[0]
    // beyond default padding so the pinned 40 is the dominant
    // axis there.)
    doc.mindmap.nodes.get_mut(&id).unwrap().size = Size {
        width: 5000.0,
        height: 5000.0,
    };
    doc.undo_stack.clear();
    assert_eq!(doc.fit_node_to_content(&id), Ok(true));
    let after = doc.mindmap.nodes[&id].size;
    assert!(
        after.width >= 60.0 && after.height >= 40.0,
        "pinned section[1]'s offset+size contribution must survive, got {}×{}",
        after.width,
        after.height,
    );
}

/// Idempotency must hold for **framed** nodes too. Pre-fix, a
/// framed node's `grow_one_node_to_fit_border` pulled `n.size`
/// past the bare text floor, so the no-op gate (which compared
/// against the bare floor) missed on every call after the
/// first — repeated `fit_node_to_content` calls stacked
/// `EditNodeAabb` undo entries. Post-fix, the gate compares the
/// *post-border-grow* size against `before_size`, which holds
/// across calls.
#[test]
fn test_fit_node_to_content_idempotent_on_framed_node() {
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // Force the testament root to wear a frame.
    doc.mindmap.nodes.get_mut(&id).unwrap().style.show_frame = true;
    doc.undo_stack.clear();
    // First call lands at the framed floor.
    assert_eq!(doc.fit_node_to_content(&id), Ok(true));
    let undo_after_first = doc.undo_stack.len();
    let size_after_first = doc.mindmap.nodes[&id].size;
    // Second call must be a no-op even though the border-grow
    // pulled the post-floor size up past the bare text floor.
    assert_eq!(doc.fit_node_to_content(&id), Ok(false));
    assert_eq!(
        doc.undo_stack.len(),
        undo_after_first,
        "framed-node fit-to-content must not stack undo entries"
    );
    assert_eq!(doc.mindmap.nodes[&id].size, size_after_first);
}

/// `fit_node_to_content` rejects with the verify-mirror-style
/// message when the floor is non-finite — exercises the
/// finite-check guard added in the self-audit fixup. We force
/// the rejection by clearing every section's text and runs;
/// `compute_one_node_text_floor` then yields a (pad-only,
/// pad-only) tuple. Empty-text sections still produce a finite
/// positive floor (pad), so this test in practice exercises the
/// idempotent-`<=0` rejection only when the loader-rejected
/// "every section empty" state is forced through the model
/// directly. Synthesize the unreachable state here to pin the
/// rejection-path coverage.
#[test]
fn test_fit_node_to_content_rejects_unmeasurable_floor() {
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // Construct an unreachable-via-loader state: a single
    // section with NaN offset. `compute_one_node_text_floor`
    // skips non-finite-offset sections, so floor stays (0, 0).
    {
        let n = doc.mindmap.nodes.get_mut(&id).unwrap();
        n.sections.clear();
        n.sections.push(baumhard::mindmap::model::MindSection::new_default(
            "x".into(),
            Vec::new(),
        ));
        n.sections[0].offset = baumhard::mindmap::model::Position {
            x: f64::NAN,
            y: 0.0,
        };
    }
    let result = doc.fit_node_to_content(&id);
    assert!(
        result.is_err_and(|m| m.contains("no measurable text")),
        "expected unmeasurable-floor error"
    );
}

/// Pinned `section.size` past the absolute typo ceiling
/// (`MAX_NODE_AXIS = 1_000_000`) propagates through the floor.
/// `fit_node_to_content` must route the candidate through
/// `check_node_size_typo` like the sibling node-size setters,
/// so the typo is caught even when it arrives via a pinned
/// section rather than a direct setter argument.
#[test]
fn test_fit_node_to_content_rejects_astronomical_pinned_section() {
    use baumhard::mindmap::model::Size;
    let (mut doc, id) = super::tests_common::pinned_two_section_node();
    // Pin section[1] to a width that exceeds the typo ceiling.
    doc.mindmap.nodes.get_mut(&id).unwrap().sections[1].size = Some(Size {
        width: 2_000_000.0,
        height: 30.0,
    });
    let result = doc.fit_node_to_content(&id);
    assert!(
        result.is_err_and(|m| m.contains("exceeds the")),
        "pinned-section typo must be caught at fit-to-content"
    );
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
            default_section_frame_border: None,
            default_focused_section_frame_border: None,
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

// ── Range-targeted section setters (Tier 2C-N4-B) ─────────────────

/// Set a colour on `[range_start, range_end)` inside one section
/// — pins the simplest happy path (range entirely inside one run).
#[test]
fn test_set_section_text_color_range_inside_one_run() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "LiberationSans");
    // Apply blue to a sub-range and verify the section now has
    // three runs: original-colour | blue | original-colour.
    let applied = doc.set_section_text_color_range(&id, 0, 1, 9, "#abcdef".into());
    assert!(applied);
    let runs = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs;
    assert_eq!(runs.len(), 3, "expected three runs after range carve-out");
    assert_eq!(runs[1].color, "#abcdef");
}

/// Range that exactly matches an existing run's color is a
/// no-op — the range setter detects pre/post equality and pops
/// the spurious undo entry.
#[test]
fn test_set_section_text_color_range_no_op_no_undo() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "LiberationSans");
    let original_color = "#ffffff".to_string();
    let undo_before = doc.undo_stack.len();
    let applied = doc.set_section_text_color_range(&id, 0, 1, 3, original_color);
    assert!(!applied, "no-op write must return false");
    assert_eq!(
        doc.undo_stack.len(),
        undo_before,
        "no-op write must not push an undo entry"
    );
}

/// Range setter clamps `range_end` to the section's grapheme
/// count. A range of `[2, 9999)` on a 10-grapheme section
/// behaves like `[2, 10)`.
#[test]
fn test_set_section_text_color_range_clamps_end_to_grapheme_count() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "LiberationSans");
    let total = 10usize;
    let applied = doc.set_section_text_color_range(&id, 0, 1, total + 100, "#abcdef".into());
    assert!(applied, "clamped range must still apply");
    let runs = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs;
    let last = runs.last().expect("at least one run");
    assert!(
        last.end <= total,
        "post-mutation runs must respect grapheme count: last.end={} > total={}",
        last.end,
        total
    );
}

/// Range with empty bounds (`start == end`) is a no-op and
/// doesn't push an undo entry.
#[test]
fn test_set_section_text_color_range_empty_returns_false() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    let undo_before = doc.undo_stack.len();
    assert!(!doc.set_section_text_color_range(&id, 0, 5, 5, "#abcdef".into()));
    assert!(!doc.set_section_text_color_range(&id, 0, 7, 3, "#abcdef".into()));
    assert_eq!(doc.undo_stack.len(), undo_before);
}

/// Range setter on a missing section returns false without
/// crashing or pushing undo.
#[test]
fn test_set_section_text_color_range_missing_section_returns_false() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    assert!(!doc.set_section_text_color_range(&id, 99, 0, 5, "#abcdef".into()));
    assert!(!doc.set_section_text_color_range("does-not-exist", 0, 0, 5, "#abcdef".into()));
}

/// Range setter pushes one undo entry and Ctrl+Z restores the
/// pre-write run set byte-for-byte.
#[test]
fn test_set_section_text_color_range_undo_round_trip() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "LiberationSans");
    let pre = doc.mindmap.nodes.get(&id).unwrap().sections[0]
        .text_runs
        .clone();
    assert!(doc.set_section_text_color_range(&id, 0, 1, 9, "#abcdef".into()));
    assert!(doc.undo());
    let post = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs;
    assert_eq!(post, &pre, "undo must restore pre-write runs");
}

/// Range setter for font size carries through the
/// `grow_one_node_to_fit_text` re-measure. A larger size on a
/// sub-range can grow the node's AABB.
#[test]
fn test_set_section_font_size_range_triggers_grow() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "LiberationSans");
    let pre_w = doc.mindmap.nodes.get(&id).unwrap().size.width;
    // Apply a much larger font to the whole section's range —
    // forces the grow pass and the post-write width should be
    // at least the pre-write width.
    assert!(doc.set_section_font_size_range(&id, 0, 0, 10, 96.0));
    let post_w = doc.mindmap.nodes.get(&id).unwrap().size.width;
    assert!(post_w >= pre_w, "grow pass must monotonically widen the node");
}

/// Range setter for font family clears / pins per-grapheme.
/// Pin: applying a family different from the section's runs
/// changes the in-range runs' `font` field.
#[test]
fn test_set_section_font_family_range_writes_in_range_only() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    // Override section[0].text + run to a known length so the
    // test isn't sensitive to which testament node `pinned_…`
    // happens to pick (HashMap iteration order isn't stable).
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "DejaVuSans");
    let original_font = "DejaVuSans".to_string();
    let target_font = "LiberationSans".to_string();
    assert!(doc.set_section_font_family_range(&id, 0, 1, 4, Some(&target_font)));
    let runs = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs;
    // Find the in-range run and the out-of-range runs.
    let in_range: Vec<_> = runs.iter().filter(|r| r.start >= 1 && r.end <= 4).collect();
    let out_of_range: Vec<_> = runs.iter().filter(|r| r.end <= 1 || r.start >= 4).collect();
    assert!(!in_range.is_empty(), "expected at least one in-range run");
    for r in in_range {
        assert_eq!(r.font, target_font);
    }
    for r in out_of_range {
        assert_eq!(r.font, original_font);
    }
}

/// Gap-fill: applying a colour on a range that falls in a gap
/// (no covering run) inserts a fresh run carrying the colour.
/// Pins the foundation gap N4-A.1's `insert_run` primitive
/// closes — without it, the user's "make graphemes 5..8 blue"
/// would silently no-op when no run covers that range.
#[test]
fn test_set_section_text_color_range_fills_gap() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    // Override section[0].text + run to a known length so the
    // test isn't sensitive to HashMap iteration order picking a
    // testament node with short section text.
    set_section_zero_text_and_single_run(&mut doc, &id, "abcdefghij", "LiberationSans");
    {
        let n = doc.mindmap.nodes.get_mut(&id).unwrap();
        let s = &mut n.sections[0];
        // Shrink the run to [0, 3) so [3, 10) is a gap.
        s.text_runs[0].end = 3;
    }
    let runs_before = doc.mindmap.nodes.get(&id).unwrap().sections[0]
        .text_runs
        .len();
    assert!(doc.set_section_text_color_range(&id, 0, 5, 8, "#123456".into()));
    let runs = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs;
    assert!(
        runs.len() > runs_before,
        "gap-fill must add at least one run"
    );
    let new_run = runs.iter().find(|r| r.start == 5 && r.end == 8);
    assert!(new_run.is_some(), "expected a new run covering [5, 8)");
    assert_eq!(new_run.unwrap().color, "#123456");
}

/// Test helper: overwrite section[0]'s text with a known string
/// and replace its runs with a single full-coverage run carrying
/// the given font. Used by range-setter tests that need a
/// deterministic grapheme count — `first_testament_node_id` runs
/// over `HashMap` iteration order, which isn't stable across
/// test orderings, so the fixture's text length varies.
fn set_section_zero_text_and_single_run(
    doc: &mut MindMapDocument,
    node_id: &str,
    text: &str,
    font: &str,
) {
    let total = count_grapheme_clusters(text);
    let n = doc.mindmap.nodes.get_mut(node_id).expect("node exists");
    let s = &mut n.sections[0];
    s.text = text.to_string();
    s.text_runs.clear();
    s.text_runs.push(TextRun {
        start: 0,
        end: total,
        bold: false,
        italic: false,
        underline: false,
        font: font.to_string(),
        size_pt: 14,
        color: "#ffffff".to_string(),
        hyperlink: None,
    });
    // Reset undo so the round-trip test can probe `undo()` on
    // the range mutation alone.
    doc.undo_stack.clear();
    doc.dirty = false;
}
