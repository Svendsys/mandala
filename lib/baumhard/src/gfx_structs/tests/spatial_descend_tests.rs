// SPDX-License-Identifier: MPL-2.0

//! Tests for [`Instruction::SpatialDescend`] and [`MouseEventData`]
//! (§T1).
//!
//! Covers event delivery, miss/no-op, innermost-node selection,
//! deep/wide trees, channel-override behavior, an instruction with
//! no payload, and MouseEventData construction/equality. Follows the
//! `do_*()` / `test_*()` benchmark-reuse split (§T2.2).
//!
//! Shared fixtures live in `subtree_aabb_tests`.

use glam::Vec2;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::core::primitives::ApplyOperation;
use crate::engine::mutator::Instruction;
use crate::engine::walker::walk_tree_from;
use crate::font::fonts;
use crate::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaField};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{
    GfxMutation, GfxMutator, GlyphTreeEvent, GlyphTreeEventInstance, MouseEventData,
};
use crate::gfx_structs::tree::{EventSubscriber, GfxMutatorTree, GfxTree};
use crate::util::ordered_vec2::OrderedVec2;

use crate::gfx_structs::tests::subtree_aabb_tests::{build_deep_chain, build_test_tree, build_wide_tree};

/// Build a MutatorTree that delivers a MouseEvent via SpatialDescend.
fn spatial_descend_mutator(x: f32, y: f32) -> GfxMutatorTree {
    let event = GfxMutation::Event(GlyphTreeEventInstance::new(
        GlyphTreeEvent::MouseEvent(MouseEventData::new(x, y)),
        0,
    ));
    let instruction = GfxMutator::Instruction {
        instruction: Instruction::SpatialDescend(OrderedVec2::new_f32(x, y)),
        channel: 0,
        mutation: Some(event),
    };
    GfxMutatorTree::new_with(instruction)
}

// ── event delivery ────────────────────────────────────────────────

#[test]
fn test_spatial_descend_delivers_event_to_leaf() {
    do_spatial_descend_delivers_event_to_leaf();
}

pub fn do_spatial_descend_delivers_event_to_leaf() {
    let mut tree = build_test_tree();
    let received = Arc::new(AtomicBool::new(false));
    let rc = received.clone();
    let sub: EventSubscriber = Rc::new(RefCell::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| {
            rc.store(true, Ordering::SeqCst);
        },
    ));
    let right_id = tree.root.children(&tree.arena).nth(1).unwrap();
    tree.arena
        .get_mut(right_id)
        .unwrap()
        .get_mut()
        .subscribers_mut()
        .push(sub);

    let mtree = spatial_descend_mutator(210.0, 210.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root, &());
    assert!(received.load(Ordering::SeqCst));
}

// ── which box wins ────────────────────────────────────────────────

/// Hang a subscriber on `id` that records its delivery in the
/// returned flag.
fn record_delivery(tree: &mut GfxTree, id: indextree::NodeId) -> Arc<AtomicBool> {
    let received = Arc::new(AtomicBool::new(false));
    let flag = received.clone();
    let sub: EventSubscriber = Rc::new(RefCell::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| {
            flag.store(true, Ordering::SeqCst);
        },
    ));
    tree.arena
        .get_mut(id)
        .unwrap()
        .get_mut()
        .subscribers_mut()
        .push(sub);
    received
}

/// Append an area at `pos` with `size` under `parent`.
fn append_box(tree: &mut GfxTree, parent: indextree::NodeId, pos: Vec2, size: Vec2) -> indextree::NodeId {
    let id = tree.arena.new_node(GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("box", 14.0, 14.0, pos, size),
        0,
    ));
    parent.append(id, &mut tree.arena);
    id
}

#[test]
fn test_spatial_descend_prefers_the_smallest_overlapping_box() {
    do_spatial_descend_prefers_the_smallest_overlapping_box();
}

/// Where boxes overlap at the point, the smallest wins — here a child
/// inside its parent, so both contain the point and only the child
/// may receive the event. Among two boxes of exactly equal size the
/// first in pre-order keeps the hit.
pub fn do_spatial_descend_prefers_the_smallest_overlapping_box() {
    fonts::init();
    let mut tree = GfxTree::new_non_indexed();
    let root = tree.root;
    let outer = append_box(&mut tree, root, Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0));
    let inner = append_box(&mut tree, outer, Vec2::new(10.0, 10.0), Vec2::new(20.0, 20.0));
    let outer_received = record_delivery(&mut tree, outer);
    let inner_received = record_delivery(&mut tree, inner);

    let mtree = spatial_descend_mutator(15.0, 15.0);
    walk_tree_from(&mut tree, &mtree, root, mtree.root, &());
    assert!(
        inner_received.load(Ordering::SeqCst),
        "the smaller box holds the point"
    );
    assert!(
        !outer_received.load(Ordering::SeqCst),
        "the larger box loses to it"
    );

    let mut tree = GfxTree::new_non_indexed();
    let root = tree.root;
    let first = append_box(&mut tree, root, Vec2::new(0.0, 0.0), Vec2::new(40.0, 40.0));
    let second = append_box(&mut tree, root, Vec2::new(5.0, 5.0), Vec2::new(40.0, 40.0));
    let first_received = record_delivery(&mut tree, first);
    let second_received = record_delivery(&mut tree, second);

    let mtree = spatial_descend_mutator(20.0, 20.0);
    walk_tree_from(&mut tree, &mtree, root, mtree.root, &());
    assert!(
        first_received.load(Ordering::SeqCst),
        "an equal-size tie goes to the first in pre-order"
    );
    assert!(!second_received.load(Ordering::SeqCst));
}

#[test]
fn test_spatial_descend_skips_a_box_with_no_extent() {
    do_spatial_descend_skips_a_box_with_no_extent();
}

/// A zero-width area at the point is no hit — it has no box — even
/// though it would otherwise be the smallest; the box behind it
/// receives the event.
pub fn do_spatial_descend_skips_a_box_with_no_extent() {
    fonts::init();
    let mut tree = GfxTree::new_non_indexed();
    let root = tree.root;
    let backdrop = append_box(&mut tree, root, Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0));
    let flat = append_box(&mut tree, root, Vec2::new(10.0, 10.0), Vec2::new(0.0, 20.0));
    let backdrop_received = record_delivery(&mut tree, backdrop);
    let flat_received = record_delivery(&mut tree, flat);

    let mtree = spatial_descend_mutator(10.0, 15.0);
    walk_tree_from(&mut tree, &mtree, root, mtree.root, &());
    assert!(
        !flat_received.load(Ordering::SeqCst),
        "a zero-width area has no box to hit"
    );
    assert!(backdrop_received.load(Ordering::SeqCst));
}

// ── miss is no-op ─────────────────────────────────────────────────

#[test]
fn test_spatial_descend_miss_is_noop() {
    do_spatial_descend_miss_is_noop();
}

pub fn do_spatial_descend_miss_is_noop() {
    let mut tree = build_test_tree();
    let received = Arc::new(AtomicBool::new(false));
    let rc = received.clone();
    let sub: EventSubscriber = Rc::new(RefCell::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| {
            rc.store(true, Ordering::SeqCst);
        },
    ));
    let right_id = tree.root.children(&tree.arena).nth(1).unwrap();
    tree.arena
        .get_mut(right_id)
        .unwrap()
        .get_mut()
        .subscribers_mut()
        .push(sub);

    let mtree = spatial_descend_mutator(999.0, 999.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root, &());
    assert!(!received.load(Ordering::SeqCst));
}

// ── innermost node wins ───────────────────────────────────────────

#[test]
fn test_spatial_descend_finds_innermost_node() {
    do_spatial_descend_finds_innermost_node();
}

pub fn do_spatial_descend_finds_innermost_node() {
    let mut tree = build_test_tree();
    let left_received = Arc::new(AtomicBool::new(false));
    let child_received = Arc::new(AtomicBool::new(false));

    let lr = left_received.clone();
    let left_sub: EventSubscriber = Rc::new(RefCell::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| {
            lr.store(true, Ordering::SeqCst);
        },
    ));
    let cr = child_received.clone();
    let child_sub: EventSubscriber = Rc::new(RefCell::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| {
            cr.store(true, Ordering::SeqCst);
        },
    ));

    let left_id = tree.root.children(&tree.arena).next().unwrap();
    let left_child_id = left_id.children(&tree.arena).next().unwrap();
    tree.arena
        .get_mut(left_id)
        .unwrap()
        .get_mut()
        .subscribers_mut()
        .push(left_sub);
    tree.arena
        .get_mut(left_child_id)
        .unwrap()
        .get_mut()
        .subscribers_mut()
        .push(child_sub);

    let mtree = spatial_descend_mutator(25.0, 42.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root, &());

    assert!(child_received.load(Ordering::SeqCst));
    assert!(!left_received.load(Ordering::SeqCst));
}

// ── deep chain ────────────────────────────────────────────────────

#[test]
fn test_spatial_descend_deep_chain_delivers_to_leaf() {
    do_spatial_descend_deep_chain_delivers_to_leaf();
}

pub fn do_spatial_descend_deep_chain_delivers_to_leaf() {
    let mut tree = build_deep_chain();
    let received = Arc::new(AtomicBool::new(false));
    let rc = received.clone();
    let sub: EventSubscriber = Rc::new(RefCell::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| {
            rc.store(true, Ordering::SeqCst);
        },
    ));

    // Walk to deepest node.
    let mut node = tree.root;
    while let Some(cid) = tree.arena.get(node).unwrap().first_child() {
        node = cid;
    }
    tree.arena
        .get_mut(node)
        .unwrap()
        .get_mut()
        .subscribers_mut()
        .push(sub);

    let mtree = spatial_descend_mutator(105.0, 105.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root, &());
    assert!(received.load(Ordering::SeqCst));
}

// ── wide tree hits correct child ──────────────────────────────────

#[test]
fn test_spatial_descend_wide_tree_hits_correct_child() {
    do_spatial_descend_wide_tree_hits_correct_child();
}

pub fn do_spatial_descend_wide_tree_hits_correct_child() {
    let mut tree = build_wide_tree();
    let hit_ids: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));

    let children: Vec<_> = tree.root.children(&tree.arena).collect();
    for (i, &cid) in children.iter().enumerate() {
        let ids = hit_ids.clone();
        let sub: EventSubscriber = Rc::new(RefCell::new(
            move |_: &mut GfxElement, _: GlyphTreeEventInstance| {
                ids.borrow_mut().push(i);
            },
        ));
        tree.arena
            .get_mut(cid)
            .unwrap()
            .get_mut()
            .subscribers_mut()
            .push(sub);
    }

    let mtree = spatial_descend_mutator(760.0, 10.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root, &());
    assert_eq!(*hit_ids.borrow(), vec![15]);
}

// ── no payload ────────────────────────────────────────

#[test]
fn test_spatial_descend_no_mutation_is_noop() {
    do_spatial_descend_no_mutation_is_noop();
}

pub fn do_spatial_descend_no_mutation_is_noop() {
    let mut tree = build_test_tree();
    let received = Arc::new(AtomicBool::new(false));
    let rc = received.clone();
    let sub: EventSubscriber = Rc::new(RefCell::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| {
            rc.store(true, Ordering::SeqCst);
        },
    ));
    let right_id = tree.root.children(&tree.arena).nth(1).unwrap();
    tree.arena
        .get_mut(right_id)
        .unwrap()
        .get_mut()
        .subscribers_mut()
        .push(sub);

    let instruction = GfxMutator::Instruction {
        instruction: Instruction::SpatialDescend(OrderedVec2::new_f32(210.0, 210.0)),
        channel: 0,
        mutation: None,
    };
    let mtree = GfxMutatorTree::new_with(instruction);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root, &());
    assert!(!received.load(Ordering::SeqCst));
}

// ── channel override ──────────────────────────────────────────────

#[test]
fn test_spatial_descend_ignores_channel_mismatch() {
    do_spatial_descend_ignores_channel_mismatch();
}

/// SpatialDescend delivers based on position, not channel.
pub fn do_spatial_descend_ignores_channel_mismatch() {
    fonts::init();
    let mut tree = GfxTree::new_non_indexed();
    let area = GfxElement::new_area_non_indexed(
        GlyphArea::new_with_str("ch5", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(50.0, 50.0)),
        5, // channel 5
    );

    let received = Arc::new(AtomicBool::new(false));
    let rc = received.clone();
    let sub: EventSubscriber = Rc::new(RefCell::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| {
            rc.store(true, Ordering::SeqCst);
        },
    ));

    let id = tree.arena.new_node(area);
    tree.root.append(id, &mut tree.arena);
    tree.arena
        .get_mut(id)
        .unwrap()
        .get_mut()
        .subscribers_mut()
        .push(sub);

    // SpatialDescend channel 0 should still find channel-5 node.
    let mtree = spatial_descend_mutator(25.0, 25.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &mtree, root, mtree.root, &());
    assert!(received.load(Ordering::SeqCst));
}

#[test]
fn test_apply_to_redirties_subtree_aabbs_after_spatial_descend() {
    do_apply_to_redirties_subtree_aabbs_after_spatial_descend();
}

/// A walk that hit-tests (`SpatialDescend`) and then moves a sibling
/// in the same `apply_to` must leave the tree's subtree AABBs dirty,
/// so post-apply hit-tests see the moved geometry. The memo is made
/// fresh before the apply — otherwise a tree that was never queried
/// is dirty from construction and the assertions below would hold
/// whether or not the apply invalidated anything.
pub fn do_apply_to_redirties_subtree_aabbs_after_spatial_descend() {
    fonts::init();
    let mut tree = GfxTree::new_non_indexed();
    let probe_id = tree.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str("probe", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(50.0, 50.0)),
        0,
        1,
    ));
    let moved_id = tree.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str("moved", 14.0, 14.0, Vec2::new(100.0, 0.0), Vec2::new(50.0, 50.0)),
        1,
        2,
    ));
    tree.root.append(probe_id, &mut tree.arena);
    tree.root.append(moved_id, &mut tree.arena);

    let mut mutator = GfxMutatorTree::new();
    let spatial_id = mutator.arena.new_node(GfxMutator::Instruction {
        instruction: Instruction::SpatialDescend(OrderedVec2::new_f32(10.0, 10.0)),
        channel: 0,
        mutation: None,
    });
    let move_id = mutator.arena.new_node(GfxMutator::new(
        GfxMutation::area_delta(DeltaGlyphArea::new(vec![
            GlyphAreaField::Operation(ApplyOperation::Add),
            GlyphAreaField::position(300.0, 0.0),
        ])),
        1,
    ));
    mutator.root.append(spatial_id, &mut mutator.arena);
    mutator.root.append(move_id, &mut mutator.arena);

    assert_eq!(
        tree.descendant_at(Vec2::new(110.0, 10.0)),
        Some(moved_id),
        "precondition: the memo is computed with `moved` at its original place"
    );

    mutator.apply_to(&mut tree, &());

    assert_eq!(tree.descendant_at(Vec2::new(410.0, 10.0)), Some(moved_id));
    assert!(tree.descendant_at(Vec2::new(110.0, 10.0)).is_none());
}

#[test]
fn test_walker_redirties_subtree_aabbs_between_spatial_descends() {
    do_walker_redirties_subtree_aabbs_between_spatial_descends();
}

/// A walker-applied move between two spatial descents must be seen
/// by both consumers of geometry: the second descent (which reads each
/// element's live box) and a hit-test after a bare `walk_tree_from`
/// (which reads the subtree-AABB memo, so the walk itself must dirty
/// it — there is no `apply_to` around it to do so). The memo is made
/// fresh before the move so that dirtiness is the walk's doing.
pub fn do_walker_redirties_subtree_aabbs_between_spatial_descends() {
    fonts::init();
    let mut tree = GfxTree::new_non_indexed();
    let probe_id = tree.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str("probe", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(50.0, 50.0)),
        0,
        1,
    ));
    let moved_id = tree.arena.new_node(GfxElement::new_area_non_indexed_with_id(
        GlyphArea::new_with_str("moved", 14.0, 14.0, Vec2::new(100.0, 0.0), Vec2::new(50.0, 50.0)),
        1,
        2,
    ));
    tree.root.append(probe_id, &mut tree.arena);
    tree.root.append(moved_id, &mut tree.arena);

    let received = Arc::new(AtomicBool::new(false));
    let rc = received.clone();
    let sub: EventSubscriber = Rc::new(RefCell::new(
        move |_: &mut GfxElement, _: GlyphTreeEventInstance| {
            rc.store(true, Ordering::SeqCst);
        },
    ));
    tree.arena
        .get_mut(moved_id)
        .unwrap()
        .get_mut()
        .subscribers_mut()
        .push(sub);

    let first_spatial = GfxMutatorTree::new_with(GfxMutator::Instruction {
        instruction: Instruction::SpatialDescend(OrderedVec2::new_f32(10.0, 10.0)),
        channel: 0,
        mutation: None,
    });
    let root = tree.root;
    walk_tree_from(&mut tree, &first_spatial, root, first_spatial.root, &());

    let mut move_mutator = GfxMutatorTree::new();
    let move_id = move_mutator.arena.new_node(GfxMutator::new(
        GfxMutation::area_delta(DeltaGlyphArea::new(vec![
            GlyphAreaField::Operation(ApplyOperation::Add),
            GlyphAreaField::position(300.0, 0.0),
        ])),
        1,
    ));
    move_mutator.root.append(move_id, &mut move_mutator.arena);
    assert_eq!(
        tree.descendant_at(Vec2::new(110.0, 10.0)),
        Some(moved_id),
        "precondition: the memo is computed with `moved` at its original place"
    );
    let root = tree.root;
    walk_tree_from(&mut tree, &move_mutator, root, move_mutator.root, &());
    assert_eq!(
        tree.descendant_at(Vec2::new(410.0, 10.0)),
        Some(moved_id),
        "a bare walk_tree_from must leave the memo dirty after it moves an area"
    );

    let second_spatial = spatial_descend_mutator(410.0, 10.0);
    let root = tree.root;
    walk_tree_from(&mut tree, &second_spatial, root, second_spatial.root, &());

    assert!(received.load(Ordering::SeqCst));
}

// =====================================================================
// MouseEventData
// =====================================================================

#[test]
fn test_mouse_event_data_round_trips_constructor_inputs() {
    do_mouse_event_data_round_trips_constructor_inputs();
}

/// `MouseEventData::new` round-trips through to the named fields,
/// `PartialEq` distinguishes distinct payloads, and the OrderedFloat
/// wrapper accepts every f32 boundary value (saturating, signed, and
/// subnormal) without nudging them.
pub fn do_mouse_event_data_round_trips_constructor_inputs() {
    let a = MouseEventData::new(42.5, -10.0);
    let b = MouseEventData::new(42.5, -10.0);
    let c = MouseEventData::new(0.0, 0.0);
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_eq!(a.x.0, 42.5_f32);
    assert_eq!(a.y.0, -10.0_f32);

    let extreme = MouseEventData::new(f32::MAX, f32::MIN);
    assert_eq!(extreme.x.0, f32::MAX);
    assert_eq!(extreme.y.0, f32::MIN);

    // Subnormal boundary — OrderedFloat must not coerce away from
    // `MIN_POSITIVE` (a real coordinate any high-DPI device can land on).
    let subnormal = MouseEventData::new(f32::MIN_POSITIVE, -f32::MIN_POSITIVE);
    assert_eq!(subnormal.x.0, f32::MIN_POSITIVE);
    assert_eq!(subnormal.y.0, -f32::MIN_POSITIVE);
}

#[test]
fn test_glyph_tree_event_mouse_carries_data() {
    do_glyph_tree_event_mouse_carries_data();
}

pub fn do_glyph_tree_event_mouse_carries_data() {
    let data = MouseEventData::new(100.0, 200.0);
    let event = GlyphTreeEventInstance::new(GlyphTreeEvent::MouseEvent(data), 12345);
    assert_eq!(event.event_time_millis, 12345);
    match &event.event_type {
        GlyphTreeEvent::MouseEvent(d) => {
            assert_eq!(d.x.0, 100.0_f32);
            assert_eq!(d.y.0, 200.0_f32);
        }
        other => panic!("expected MouseEvent, got {:?}", other),
    }
    assert_eq!(event.clone().event_type, event.event_type);
}
