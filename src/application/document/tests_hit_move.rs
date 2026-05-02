// SPDX-License-Identifier: MPL-2.0

//! Hit-testing, rect-select, selection, tree highlights, move, drag, animations.
//!
//! Part of the tests split for `document`. Helpers live in
//! `tests_common`; only the tests for this theme live here.
use super::tests_common::{load_test_doc, load_test_tree, TestNudgeMutation};
use super::*;

use baumhard::mindmap::animation::{AnimationTiming, Easing};
use baumhard::mindmap::custom_mutation::{CustomMutation as CM, TargetScope as TS};
use glam::Vec2;

#[test]
fn test_hit_test_direct_hit() {
    let mut tree = load_test_tree();
    // "Lord God" node (id: 0) — get its position from the tree
    let node_id = tree.arena_id_for("0").unwrap();
    let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
    let center = Vec2::new(
        area.position.x.0 + area.render_bounds.x.0 / 2.0,
        area.position.y.0 + area.render_bounds.y.0 / 2.0,
    );
    let result = hit_test(center, &mut tree);
    assert_eq!(result, Some("0".to_string()));
}

/// `hit_test_target` collapses single-section nodes (the
/// migration default) to `HitTarget::NodeContainer` — clicking
/// anywhere inside a node with one default section gives the
/// pre-section whole-node hit semantic.
#[test]
fn test_hit_test_target_single_section_collapses_to_node() {
    let mut tree = load_test_tree();
    let node_id = tree.arena_id_for("0").unwrap();
    let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
    let center = Vec2::new(
        area.position.x.0 + area.render_bounds.x.0 / 2.0,
        area.position.y.0 + area.render_bounds.y.0 / 2.0,
    );
    match hit_test_target(center, &mut tree) {
        Some(HitTarget::NodeContainer { node_id }) => {
            assert_eq!(node_id, "0");
        }
        other => panic!("expected NodeContainer, got {other:?}"),
    }
}

#[test]
fn test_hit_test_miss() {
    let mut tree = load_test_tree();
    // A point far away from any node
    let result = hit_test(Vec2::new(-99999.0, -99999.0), &mut tree);
    assert_eq!(result, None);
}

#[test]
fn test_hit_test_returns_smallest_on_overlap() {
    let mut tree = load_test_tree();
    // Find a parent-child pair where child is inside parent's bounds
    // "Lord God" (0) has children — find one whose bounds overlap
    let parent_id_str = "0";
    let parent_size = {
        let nid = tree.arena_id_for(parent_id_str).unwrap();
        let area = tree.tree.arena.get(nid).unwrap().get().glyph_area().unwrap();
        area.render_bounds.x.0 * area.render_bounds.y.0
    };

    // Collect candidate (mind_id, center) pairs first to release
    // the immutable borrow on tree.node_map before calling
    // hit_test (which needs &mut tree).
    let candidate: Option<(String, Vec2)> =
        tree.node_ids()
            .filter(|(id, _)| *id != parent_id_str)
            .find_map(|(mind_id, nid)| {
                let a = tree.tree.arena.get(nid)?.get().glyph_area()?;
                let child_size = a.render_bounds.x.0 * a.render_bounds.y.0;
                let child_center = Vec2::new(
                    a.position.x.0 + a.render_bounds.x.0 / 2.0,
                    a.position.y.0 + a.render_bounds.y.0 / 2.0,
                );
                if child_size < parent_size && point_in_node_aabb(child_center, parent_id_str, &tree) {
                    Some((mind_id.to_string(), child_center))
                } else {
                    None
                }
            });

    if let Some((expected_id, center)) = candidate {
        let result = hit_test(center, &mut tree);
        assert_eq!(
            result,
            Some(expected_id),
            "Should select smaller child node, not parent"
        );
    }
    // If no overlap found in test data, that's OK — test is structural
}

#[test]
fn test_selection_state_is_selected() {
    let none = SelectionState::None;
    assert!(!none.is_selected("123"));

    let single = SelectionState::Single("123".to_string());
    assert!(single.is_selected("123"));
    assert!(!single.is_selected("456"));

    let multi = SelectionState::Multi(vec!["123".to_string(), "456".to_string()]);
    assert!(multi.is_selected("123"));
    assert!(multi.is_selected("456"));
    assert!(!multi.is_selected("789"));

    // Section selection counts as "this owning node is selected" —
    // every per-node consumer (highlight, drag, chrome) gets the
    // natural answer.
    let section = SelectionState::Section(SectionSel::new("123", 1));
    assert!(section.is_selected("123"));
    assert!(!section.is_selected("456"));
    assert_eq!(section.selected_ids(), vec!["123"]);
    let s = section
        .selected_section()
        .expect("Section variant carries SectionSel");
    assert_eq!(s.node_id, "123");
    assert_eq!(s.section_idx, 1);
    // Other selection variants return `None` from
    // `selected_section()`.
    assert!(none.selected_section().is_none());
    assert!(single.selected_section().is_none());
}

#[test]
fn test_selection_state_from_ids_empty_is_none() {
    assert!(matches!(SelectionState::from_ids(vec![]), SelectionState::None));
}

#[test]
fn test_selection_state_from_ids_single_element_is_single() {
    match SelectionState::from_ids(vec!["alpha".to_string()]) {
        SelectionState::Single(id) => assert_eq!(id, "alpha"),
        other => panic!("expected Single, got {other:?}"),
    }
}

#[test]
fn test_selection_state_from_ids_two_elements_is_multi_preserving_order() {
    match SelectionState::from_ids(vec!["a".to_string(), "b".to_string()]) {
        SelectionState::Multi(ids) => assert_eq!(ids, vec!["a".to_string(), "b".to_string()]),
        other => panic!("expected Multi, got {other:?}"),
    }
}

#[test]
fn test_selection_state_from_ids_many_elements_is_multi_preserving_order() {
    let input = vec!["a".to_string(), "b".to_string(), "c".to_string(), "d".to_string()];
    match SelectionState::from_ids(input.clone()) {
        SelectionState::Multi(ids) => assert_eq!(ids, input),
        other => panic!("expected Multi, got {other:?}"),
    }
}

#[test]
fn test_apply_tree_highlights_via_walker() {
    let mut tree = load_test_tree();
    // Post-refactor: regions live on the section-area, not the
    // container. Read through the section_map.
    let section_id = tree.section_arena_id("0", 0).unwrap();

    // Before highlight: original color (white).
    let area = tree
        .tree
        .arena
        .get(section_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap();
    let original_color = area.regions.all_regions()[0].color.unwrap();
    assert!(
        (original_color[0] - 1.0).abs() < 0.01,
        "Expected white before highlight"
    );

    // Apply highlight via the new mutator-driven path.
    apply_tree_highlights(&mut tree, std::iter::once(("0", None, HIGHLIGHT_COLOR)));

    // After highlight: cyan on section-area's regions.
    let area = tree
        .tree
        .arena
        .get(section_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap();
    let highlighted_color = area.regions.all_regions()[0].color.unwrap();
    assert!((highlighted_color[0] - HIGHLIGHT_COLOR[0]).abs() < 0.01);
    assert!((highlighted_color[1] - HIGHLIGHT_COLOR[1]).abs() < 0.01);
    assert!((highlighted_color[2] - HIGHLIGHT_COLOR[2]).abs() < 0.01);
}

#[test]
fn test_apply_tree_highlights_does_not_affect_others() {
    let mut tree = load_test_tree();

    // Pick a different node and copy its regions before mutation.
    let other_id = tree
        .node_ids()
        .map(|(k, _)| k)
        .find(|k| *k != "0")
        .unwrap()
        .to_string();
    let other_node_id = tree.arena_id_for(&other_id).unwrap();
    let before = tree
        .tree
        .arena
        .get(other_node_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .regions
        .clone();

    apply_tree_highlights(&mut tree, std::iter::once(("0", None, HIGHLIGHT_COLOR)));

    let after = tree
        .tree
        .arena
        .get(other_node_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .regions
        .clone();
    assert_eq!(before, after, "Unselected node colors should not change");
}

#[test]
fn test_apply_tree_highlights_later_pair_overrides_earlier() {
    // The reparent-mode flow relies on source-orange overriding the
    // previously-applied selection-cyan on the same node. Verify the
    // last-write-wins semantics of apply_tree_highlights.
    let mut tree = load_test_tree();
    // Regions live on the section-area, not the container.
    let section_id = tree.section_arena_id("0", 0).unwrap();

    apply_tree_highlights(
        &mut tree,
        vec![("0", None, HIGHLIGHT_COLOR), ("0", None, REPARENT_SOURCE_COLOR)],
    );

    let area = tree
        .tree
        .arena
        .get(section_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap();
    let c = area.regions.all_regions()[0].color.unwrap();
    assert!((c[0] - REPARENT_SOURCE_COLOR[0]).abs() < 0.01);
    assert!((c[1] - REPARENT_SOURCE_COLOR[1]).abs() < 0.01);
    assert!((c[2] - REPARENT_SOURCE_COLOR[2]).abs() < 0.01);
}

#[test]
fn test_move_subtree_updates_all_positions() {
    let mut doc = load_test_doc();
    let node_id = "0"; // Lord God
    let descendants = doc.mindmap.all_descendants(node_id);
    assert!(!descendants.is_empty(), "Lord God should have descendants");

    // Record original positions
    let orig_pos: Vec<(String, f64, f64)> = std::iter::once(node_id.to_string())
        .chain(descendants.iter().cloned())
        .filter_map(|id| {
            let n = doc.mindmap.nodes.get(&id)?;
            Some((id, n.position.x, n.position.y))
        })
        .collect();

    let dx = 50.0;
    let dy = -30.0;
    doc.apply_move_subtree(node_id, dx, dy);

    for (id, ox, oy) in &orig_pos {
        let n = doc.mindmap.nodes.get(id).unwrap();
        assert!(
            (n.position.x - (ox + dx)).abs() < 0.001,
            "Node {} x not shifted",
            id
        );
        assert!(
            (n.position.y - (oy + dy)).abs() < 0.001,
            "Node {} y not shifted",
            id
        );
    }
}

#[test]
fn test_move_subtree_preserves_relative_positions() {
    let mut doc = load_test_doc();
    let node_id = "0";
    let descendants = doc.mindmap.all_descendants(node_id);

    // Record relative offsets from parent to each descendant
    let parent = doc.mindmap.nodes.get(node_id).unwrap();
    let offsets: Vec<(String, f64, f64)> = descendants
        .iter()
        .filter_map(|id| {
            let n = doc.mindmap.nodes.get(id)?;
            Some((
                id.clone(),
                n.position.x - parent.position.x,
                n.position.y - parent.position.y,
            ))
        })
        .collect();

    doc.apply_move_subtree(node_id, 100.0, 200.0);

    let parent = doc.mindmap.nodes.get(node_id).unwrap();
    for (id, dx, dy) in &offsets {
        let n = doc.mindmap.nodes.get(id).unwrap();
        let actual_dx = n.position.x - parent.position.x;
        let actual_dy = n.position.y - parent.position.y;
        assert!(
            (actual_dx - dx).abs() < 0.001,
            "Relative x offset changed for {}",
            id
        );
        assert!(
            (actual_dy - dy).abs() < 0.001,
            "Relative y offset changed for {}",
            id
        );
    }
}

#[test]
fn test_move_single_only_affects_target() {
    let mut doc = load_test_doc();
    let node_id = "0";
    let descendants = doc.mindmap.all_descendants(node_id);

    // Record descendant positions before
    let before: Vec<(String, f64, f64)> = descendants
        .iter()
        .filter_map(|id| {
            let n = doc.mindmap.nodes.get(id)?;
            Some((id.clone(), n.position.x, n.position.y))
        })
        .collect();

    doc.apply_move_single(node_id, 100.0, 200.0);

    // Descendants should be unchanged
    for (id, ox, oy) in &before {
        let n = doc.mindmap.nodes.get(id).unwrap();
        assert!(
            (n.position.x - ox).abs() < 0.001,
            "Descendant {} x changed unexpectedly",
            id
        );
        assert!(
            (n.position.y - oy).abs() < 0.001,
            "Descendant {} y changed unexpectedly",
            id
        );
    }

    // The target node still exists; we don't assert on its
    // exact post-move position here (the test pre-condition
    // captures only descendant positions).
    assert!(doc.mindmap.nodes.contains_key(node_id));
}

/// `start_animation` records an instance, snapshots from/to,
/// and `has_active_animations` flips true. The mutation never
/// touches the model — that's the boundary commit at completion.
#[test]
fn test_start_animation_records_instance_without_committing() {
    let mut doc = load_test_doc();
    let node_id = "0".to_string();
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    let cm = make_test_mutation_with_timing(
        "nudge-anim",
        TS::SelfOnly,
        Some(AnimationTiming {
            duration_ms: 200,
            delay_ms: 0,
            easing: Easing::Linear,
            then: None,
        }),
    );
    doc.mutation_registry.insert(cm.id.clone(), cm.clone());
    assert!(!doc.has_active_animations());
    doc.start_animation(&cm, &node_id, 1_000);
    assert!(doc.has_active_animations());
    assert_eq!(doc.active_animations.len(), 1);

    // Model untouched at start.
    let pos_now = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    assert!((pos_now - orig_x).abs() < 1e-6);

    // From / to snapshots reflect the nudge (test mutation is
    // NudgeRight(10.0)).
    let inst = &doc.active_animations[0];
    assert!((inst.from_node.position.x - orig_x).abs() < 1e-6);
    assert!((inst.to_node.position.x - orig_x - 10.0).abs() < 1e-6);
}

/// `tick_animations` at the linear midpoint writes the mean of
/// from / to into `mindmap.nodes`. Pins the per-tick blend
/// math against the canonical `lerp_f32` helper.
#[test]
fn test_tick_animations_linear_midpoint_blend() {
    let mut doc = load_test_doc();
    let node_id = "0".to_string();
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    let cm = make_test_mutation_with_timing(
        "nudge-anim",
        TS::SelfOnly,
        Some(AnimationTiming {
            duration_ms: 200,
            delay_ms: 0,
            easing: Easing::Linear,
            then: None,
        }),
    );
    doc.mutation_registry.insert(cm.id.clone(), cm.clone());
    doc.start_animation(&cm, &node_id, 1_000);

    // Tick at the midpoint (start + 100ms of 200ms duration).
    let advanced = doc.tick_animations(1_100, None);
    assert!(advanced);
    let mid_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    // NudgeRight(10.0) at t=0.5 → +5.0 from origin.
    assert!(
        (mid_x - orig_x - 5.0).abs() < 1e-3,
        "midpoint x = {mid_x}, expected {}",
        orig_x + 5.0
    );

    // Animation still active mid-flight.
    assert!(doc.has_active_animations());
}

/// At `t >= 1.0` the animation completes: the final state is
/// applied (matching the instant-mode result), the instance
/// is dropped, and `has_active_animations` flips back to false.
#[test]
fn test_tick_animations_completes_and_clears() {
    let mut doc = load_test_doc();
    let node_id = "0".to_string();
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    let cm = make_test_mutation_with_timing(
        "nudge-anim",
        TS::SelfOnly,
        Some(AnimationTiming {
            duration_ms: 100,
            delay_ms: 0,
            easing: Easing::Linear,
            then: None,
        }),
    );
    doc.mutation_registry.insert(cm.id.clone(), cm.clone());
    doc.start_animation(&cm, &node_id, 0);

    // Tick past the duration. Without a tree, the model is set
    // to the `to` snapshot directly.
    let advanced = doc.tick_animations(150, None);
    assert!(advanced);
    assert!(!doc.has_active_animations());
    let final_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    // Default test-mutation `NudgeRight(10.0)` lands at +10.
    assert!((final_x - orig_x - 10.0).abs() < 1e-3);
}

/// Ctrl+Z mid-animation fast-forwards to the completion
/// state, pushes the animation's undo entry, and then the
/// undo pops it — net effect is that Ctrl+Z during an
/// animated transition reverses the animation in one
/// keystroke, same as Ctrl+Z after natural completion. Pins
/// the §4 "no half-features" contract the review called
/// out: without this, Ctrl+Z during an animation pops the
/// *previous* action, a silent user-visible regression.
#[test]
fn test_fast_forward_then_undo_reverses_animation() {
    let mut doc = load_test_doc();
    let node_id = "0".to_string();
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    let cm = make_test_mutation_with_timing(
        "nudge-anim",
        TS::SelfOnly,
        Some(AnimationTiming {
            duration_ms: 1_000,
            delay_ms: 0,
            easing: Easing::Linear,
            then: None,
        }),
    );
    doc.mutation_registry.insert(cm.id.clone(), cm.clone());
    doc.start_animation(&cm, &node_id, 0);

    // Fast-forward (simulating Ctrl+Z entry in the event
    // loop). A tree is required because
    // `apply_custom_mutation` routes through it.
    let mut tree = doc.build_tree();
    doc.fast_forward_animations(Some(&mut tree));
    assert!(!doc.has_active_animations());
    let after_ff = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    assert!(
        (after_ff - orig_x - 10.0).abs() < 1e-3,
        "post fast-forward x = {after_ff}, expected {}",
        orig_x + 10.0
    );

    // Undo pops the entry fast-forward pushed. Position
    // returns to the original.
    let popped = doc.undo();
    assert!(popped, "undo must pop the fast-forward's entry");
    let after_undo = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    assert!(
        (after_undo - orig_x).abs() < 1e-3,
        "post undo x = {after_undo}, expected {orig_x}"
    );
}

/// Re-triggering the same `(mutation_id, node_id)` mid-flight
/// is a silent no-op — otherwise a held button could spawn dozens
/// of overlapping instances and the blend would overshoot.
#[test]
fn test_start_animation_re_trigger_mid_flight_is_noop() {
    let mut doc = load_test_doc();
    let node_id = "0".to_string();
    let cm = make_test_mutation_with_timing(
        "nudge-anim",
        TS::SelfOnly,
        Some(AnimationTiming {
            duration_ms: 200,
            delay_ms: 0,
            easing: Easing::Linear,
            then: None,
        }),
    );
    doc.mutation_registry.insert(cm.id.clone(), cm.clone());

    doc.start_animation(&cm, &node_id, 1_000);
    doc.start_animation(&cm, &node_id, 1_050);
    doc.start_animation(&cm, &node_id, 1_100);

    assert_eq!(doc.active_animations.len(), 1);
    assert_eq!(doc.active_animations[0].start_ms, 1_000);
}

fn make_test_mutation_with_timing(
    id: &str,
    scope: TS,
    timing: Option<baumhard::mindmap::animation::AnimationTiming>,
) -> CM {
    let mut b = TestNudgeMutation::new(id, scope).magnitude(10.0);
    if let Some(t) = timing {
        b = b.timing(t);
    }
    b.build()
}

#[test]
fn test_move_returns_original_positions() {
    let mut doc = load_test_doc();
    let node_id = "0";
    let orig_x = doc.mindmap.nodes.get(node_id).unwrap().position.x;
    let orig_y = doc.mindmap.nodes.get(node_id).unwrap().position.y;

    let undo_data = doc.apply_move_subtree(node_id, 50.0, 50.0);
    let target_entry = undo_data.iter().find(|(id, _)| id == node_id).unwrap();
    assert!((target_entry.1.x - orig_x).abs() < 0.001);
    assert!((target_entry.1.y - orig_y).abs() < 0.001);
}

#[test]
fn test_undo_restores_positions() {
    let mut doc = load_test_doc();
    let node_id = "0";

    // Record original positions
    let orig_x = doc.mindmap.nodes.get(node_id).unwrap().position.x;
    let orig_y = doc.mindmap.nodes.get(node_id).unwrap().position.y;

    // Move and push undo
    let undo_data = doc.apply_move_subtree(node_id, 100.0, 200.0);
    doc.undo_stack.push(UndoAction::MoveNodes {
        original_positions: undo_data,
    });

    // Verify moved
    assert!((doc.mindmap.nodes.get(node_id).unwrap().position.x - (orig_x + 100.0)).abs() < 0.001);

    // Undo
    assert!(doc.undo());

    // Verify restored
    assert!((doc.mindmap.nodes.get(node_id).unwrap().position.x - orig_x).abs() < 0.001);
    assert!((doc.mindmap.nodes.get(node_id).unwrap().position.y - orig_y).abs() < 0.001);
}

#[test]
fn test_apply_drag_delta() {
    let doc = load_test_doc();
    let mut tree = doc.build_tree();
    let node_id = "0";

    let tree_nid = tree.arena_id_for(node_id).unwrap();
    let orig_x = tree
        .tree
        .arena
        .get(tree_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    let orig_y = tree
        .tree
        .arena
        .get(tree_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .y
        .0;

    apply_drag_delta(&mut tree, node_id, 25.0, -15.0, false);

    let new_x = tree
        .tree
        .arena
        .get(tree_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    let new_y = tree
        .tree
        .arena
        .get(tree_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .y
        .0;
    assert!((new_x - (orig_x + 25.0)).abs() < 0.001);
    assert!((new_y - (orig_y - 15.0)).abs() < 0.001);
}

#[test]
fn test_apply_drag_delta_with_descendants() {
    let doc = load_test_doc();
    let mut tree = doc.build_tree();
    let node_id = "0";

    // Find a child of Lord God in the tree
    let child_ids: Vec<String> = doc.mindmap.all_descendants(node_id);
    assert!(!child_ids.is_empty());
    let child_id = &child_ids[0];
    let child_tree_nid = tree.arena_id_for(child_id).unwrap();
    let child_orig_x = tree
        .tree
        .arena
        .get(child_tree_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;

    apply_drag_delta(&mut tree, node_id, 30.0, 20.0, true);

    let child_new_x = tree
        .tree
        .arena
        .get(child_tree_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    assert!(
        (child_new_x - (child_orig_x + 30.0)).abs() < 0.001,
        "Descendant should be shifted when include_descendants=true"
    );
}

#[test]
fn test_dedup_subtree_roots() {
    let doc = load_test_doc();
    let parent_id = "0"; // Lord God
    let descendants = doc.mindmap.all_descendants(parent_id);
    assert!(!descendants.is_empty());
    let child_id = &descendants[0];

    // If both parent and child are selected, only parent should be a root
    let ids = vec![parent_id.to_string(), child_id.clone()];
    let roots = doc.dedup_subtree_roots(&ids);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0], parent_id);
}

#[test]
fn test_apply_move_multiple_no_double_movement() {
    let mut doc = load_test_doc();
    let parent_id = "0";
    let descendants = doc.mindmap.all_descendants(parent_id);
    let child_id = &descendants[0];

    let child_orig_x = doc.mindmap.nodes.get(child_id).unwrap().position.x;

    // Move both parent and child as subtrees — child should only move once (via parent)
    let ids = vec![parent_id.to_string(), child_id.clone()];
    doc.apply_move_multiple(&ids, 50.0, 0.0, false);

    let child_new_x = doc.mindmap.nodes.get(child_id).unwrap().position.x;
    assert!(
        (child_new_x - (child_orig_x + 50.0)).abs() < 0.001,
        "Child should be moved exactly once, not twice"
    );
}

#[test]
fn test_rect_select_finds_nodes_in_region() {
    let tree = load_test_tree();
    // Get position/bounds of "Lord God" to build a rect that contains it
    let node_id = tree.arena_id_for("0").unwrap();
    let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
    let x = area.position.x.0;
    let y = area.position.y.0;
    let w = area.render_bounds.x.0;
    let h = area.render_bounds.y.0;

    // A rect that exactly contains this node should select it
    let hits = rect_select(
        Vec2::new(x - 1.0, y - 1.0),
        Vec2::new(x + w + 1.0, y + h + 1.0),
        &tree,
    );
    assert!(hits.contains(&"0".to_string()), "Should find Lord God in rect");
}

#[test]
fn test_rect_select_misses_distant_nodes() {
    let tree = load_test_tree();
    // A rect far from any node should select nothing
    let hits = rect_select(
        Vec2::new(-99999.0, -99999.0),
        Vec2::new(-99998.0, -99998.0),
        &tree,
    );
    assert!(hits.is_empty(), "Should find no nodes in distant rect");
}

// --- NodeShape integration tests ---
//
// These exercise the end-to-end wiring: flip a node's `shape` on
// the tree side to `Ellipse`, then assert that hit_test /
// point_in_node_aabb / rect_select all honour the new geometry.
// The baumhard-level math is covered exhaustively in
// `lib/baumhard/src/gfx_structs/shape.rs#tests`; these tests
// guard the plumbing.

fn set_node_shape_ellipse(tree: &mut baumhard::mindmap::tree_builder::MindMapTree, node_id: &str) {
    use baumhard::gfx_structs::shape::NodeShape;
    let nid = tree.arena_id_for(node_id).expect("node exists");
    let node = tree.tree.arena.get_mut(nid).expect("arena has node");
    let area = node.get_mut().glyph_area_mut().expect("node is a GlyphArea");
    area.shape = NodeShape::Ellipse;
}

fn node_bounds(tree: &baumhard::mindmap::tree_builder::MindMapTree, node_id: &str) -> (Vec2, Vec2) {
    let nid = tree.arena_id_for(node_id).unwrap();
    let area = tree.tree.arena.get(nid).unwrap().get().glyph_area().unwrap();
    (
        Vec2::new(area.position.x.0, area.position.y.0),
        Vec2::new(area.render_bounds.x.0, area.render_bounds.y.0),
    )
}

#[test]
fn test_hit_test_ellipse_centre_hits() {
    let mut tree = load_test_tree();
    set_node_shape_ellipse(&mut tree, "0");
    let (pos, bounds) = node_bounds(&tree, "0");
    let centre = pos + bounds * 0.5;
    assert_eq!(hit_test(centre, &mut tree), Some("0".to_string()));
}

#[test]
fn test_hit_test_ellipse_aabb_corner_misses() {
    let mut tree = load_test_tree();
    set_node_shape_ellipse(&mut tree, "0");
    let (pos, _bounds) = node_bounds(&tree, "0");
    // Epsilon just inside the AABB corner — rectangle would
    // pick it, ellipse must not.
    let near_corner = pos + Vec2::new(0.5, 0.5);
    let hit = hit_test(near_corner, &mut tree);
    assert_ne!(
        hit,
        Some("0".to_string()),
        "Ellipse hit-test must reject AABB corner clicks"
    );
}

#[test]
fn test_point_in_node_aabb_is_shape_aware() {
    let mut tree = load_test_tree();
    set_node_shape_ellipse(&mut tree, "0");
    let (pos, bounds) = node_bounds(&tree, "0");
    let centre = pos + bounds * 0.5;
    let near_corner = pos + Vec2::new(0.5, 0.5);
    assert!(
        point_in_node_aabb(centre, "0", &tree),
        "Centre must count as inside the ellipse"
    );
    assert!(
        !point_in_node_aabb(near_corner, "0", &tree),
        "AABB corner must NOT count as inside the ellipse"
    );
}

#[test]
fn test_rect_select_ignores_ellipse_aabb_corner_only() {
    let mut tree = load_test_tree();
    set_node_shape_ellipse(&mut tree, "0");
    let (pos, _bounds) = node_bounds(&tree, "0");
    // Tiny selection rect tucked into the top-left corner —
    // inside the AABB, outside the ellipse. The old pure-AABB
    // `rect_select` would have returned "0"; the shape-aware
    // version must not.
    let hits = rect_select(pos, pos + Vec2::new(2.0, 2.0), &tree);
    assert!(
        !hits.contains(&"0".to_string()),
        "Rect-select inside ellipse's AABB corner must miss"
    );
}

#[test]
fn test_rect_select_still_catches_ellipse_through_centre() {
    let mut tree = load_test_tree();
    set_node_shape_ellipse(&mut tree, "0");
    let (pos, bounds) = node_bounds(&tree, "0");
    // Selection rect crossing the centre of the ellipse must
    // still register a hit.
    let centre = pos + bounds * 0.5;
    let hits = rect_select(centre - Vec2::new(5.0, 5.0), centre + Vec2::new(5.0, 5.0), &tree);
    assert!(
        hits.contains(&"0".to_string()),
        "Rect-select crossing the ellipse centre must hit"
    );
}

/// `point_in_node_aabb` consults the cached subtree AABB so a
/// click on a section that overflows the container's AABB still
/// counts as "inside the node". Pre-fix, the function read only
/// the container's render_bounds, leaving overflowing sections
/// unhittable. Anchors the documented "click-outside-commit"
/// gesture for multi-section nodes.
#[test]
fn test_point_in_node_aabb_includes_overflowing_section() {
    use super::tests_common::doc_with_one_orphan_node;
    use baumhard::mindmap::model::{MindSection, Position, Size};
    let mut doc = doc_with_one_orphan_node();
    // Append a second section positioned past the container's
    // right edge. The container's AABB is [0,0]→[240,60]; the
    // overflow section sits at offset (300, 0) with its own
    // 100×40 size, putting its left edge well past the container.
    {
        let node = doc.mindmap.nodes.get_mut("0").unwrap();
        let mut overflow = MindSection::new_default("over".into(), vec![]);
        overflow.offset = Position { x: 300.0, y: 0.0 };
        overflow.size = Some(Size { width: 100.0, height: 40.0 });
        node.sections.push(overflow);
    }
    let mut tree = doc.build_tree();
    // Force the subtree-AABB cache to populate. The runtime hot
    // path invalidates / recomputes on every render and hit-test;
    // tests have to ask for it explicitly.
    tree.tree.ensure_subtree_aabbs();

    // A point well inside the overflow section but outside the
    // container's own AABB.
    let in_overflow = Vec2::new(350.0, 20.0);
    assert!(
        point_in_node_aabb(in_overflow, "0", &tree),
        "point inside overflow section must register as inside node 0"
    );
    // A point that's outside both the container AND the overflow
    // section's bounding box must still miss.
    let outside_all = Vec2::new(500.0, 200.0);
    assert!(
        !point_in_node_aabb(outside_all, "0", &tree),
        "point outside both container and overflow must miss"
    );
}

/// `collect_patches_recursive` (drag delta) only emits patches
/// for `GlyphArea`-bearing elements; `GlyphModel` siblings of
/// section-areas (the structural seam children) are skipped so
/// `patch_drag_positions` doesn't pay a cold hash miss per drag
/// tick on every section's model child. Pins the filter — a
/// future refactor that re-flattens the conditional would
/// silently regress drag performance and the test would fail.
#[test]
fn test_collect_patches_recursive_skips_glyph_model_children() {
    use crate::application::document::apply_drag_delta_and_collect_patches;
    use baumhard::core::primitives::{Flag, Flaggable};
    use baumhard::mindmap::model::MindSection;
    use crate::application::document::tests_common::doc_with_one_orphan_node;

    let mut doc = doc_with_one_orphan_node();
    {
        // Multi-section node — 3 sections × (section-area +
        // section-model) = 6 arena entries. Patches should
        // count only the 3 section-areas + 1 container = 4.
        let node = doc.mindmap.nodes.get_mut("0").unwrap();
        node.sections.push(MindSection::new_default("two".into(), Vec::new()));
        node.sections.push(MindSection::new_default("three".into(), Vec::new()));
    }
    let mut tree = doc.build_tree();
    let mut patches: Vec<(usize, (f32, f32))> = Vec::new();
    apply_drag_delta_and_collect_patches(&mut tree, "0", 5.0, 7.0, false, &mut patches);

    // Every patch's unique_id must correspond to a GlyphArea-
    // bearing element in the arena. GlyphModel elements (the
    // structural section-model siblings) carry the
    // `Flag::SectionRoot` marker but no glyph_area; verify
    // none of those leaked into the patch list.
    for (uid, _pos) in &patches {
        let element = tree
            .tree
            .arena
            .iter()
            .find(|n| n.get().unique_id() == *uid)
            .map(|n| n.get())
            .expect("patch references a real arena entry");
        assert!(
            element.glyph_area().is_some(),
            "patch with unique_id {uid} references a non-GlyphArea element ({:?})",
            element.flag_is_set(Flag::SectionRoot)
        );
    }
    // 1 container + 3 section-areas = 4 GlyphArea entries; the
    // 3 section-models are skipped.
    assert_eq!(
        patches.len(),
        4,
        "expected 4 patches (1 container + 3 section-areas), got {}",
        patches.len()
    );
}

// --- Custom mutation registry & application tests ---
