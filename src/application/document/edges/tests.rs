// SPDX-License-Identifier: MPL-2.0

//! Tests for `edges/{structural,style,label,mode,portal}.rs`. Lifted
//! whole from the original `edges.rs` `mod tests { ... }` block when
//! that file split into the `edges/` directory.

use baumhard::mindmap::model::PortalEndpointState;

use super::inline::{
    ensure_glyph_connection_inline, option_f32_eps_eq, write_endpoint_field,
};
use crate::application::document::tests_common::doc_with_one_edge as doc_with_edge;
use crate::application::document::types::EdgeRef;
use crate::application::document::undo_action::UndoAction;

/// `mutate_edge` returning `false` from the closure rolls
/// back any in-place fork via `ensure_glyph_connection_inline`
/// — the live model returns to the pre-closure state, no
/// `EditEdge` undo entry is pushed, and `dirty` stays where
/// it was. Critical for the no-op contract every per-field
/// setter (`set_edge_color`, `set_edge_cap_start`, ...) routes
/// through.
#[test]
fn mutate_edge_rolls_back_fork_on_false_return() {
    let (mut doc, er) = doc_with_edge();
    // Pre-condition: this edge has no glyph_connection.
    assert!(doc.mindmap.edges[0].glyph_connection.is_none());
    doc.dirty = false;

    let returned = doc.mutate_edge(&er, |edge, canvas| {
        // Force a fork — but then return false.
        let _cfg = ensure_glyph_connection_inline(edge, canvas);
        false
    });

    assert!(!returned);
    // Fork must have rolled back.
    assert!(
        doc.mindmap.edges[0].glyph_connection.is_none(),
        "the fork survived a false-return rollback"
    );
    assert!(doc.undo_stack.is_empty(), "no undo entry on false-return");
    assert!(!doc.dirty, "dirty stays unset on false-return");
}

/// On a `true` return, `mutate_edge` pushes one `EditEdge`
/// undo entry carrying the *pre-fork* state, sets `dirty`,
/// and leaves the closure's mutations in place. Locks the
/// other half of the contract.
#[test]
fn mutate_edge_pushes_undo_with_pre_fork_state_on_true() {
    let (mut doc, er) = doc_with_edge();
    assert!(doc.mindmap.edges[0].glyph_connection.is_none());
    doc.dirty = false;

    let returned = doc.mutate_edge(&er, |edge, canvas| {
        ensure_glyph_connection_inline(edge, canvas).body =
            "X".to_string();
        true
    });

    assert!(returned);
    assert_eq!(
        doc.mindmap.edges[0].glyph_connection.as_ref().unwrap().body,
        "X"
    );
    assert!(doc.dirty);
    assert_eq!(doc.undo_stack.len(), 1);
    match &doc.undo_stack[0] {
        UndoAction::EditEdge { before, .. } => {
            assert!(
                before.glyph_connection.is_none(),
                "undo snapshot must carry the pre-fork None"
            );
        }
        other => panic!("expected EditEdge, got {:?}", other),
    }
}

/// `commit_throttled_edge_drag` with a `|_, _| true`
/// predicate (the EdgeHandle release path) pushes one
/// `EditEdge` undo entry carrying the supplied `original`
/// snapshot, regardless of whether the live edge changed.
/// The drag-threshold contract (every reaching release is
/// post-mutation, so always commit).
#[test]
fn commit_throttled_edge_drag_always_commits_on_true_predicate() {
    let (mut doc, er) = doc_with_edge();
    doc.dirty = false;
    let original = doc.mindmap.edges[0].clone();
    doc.commit_throttled_edge_drag(&er, original, |_, _| true);
    assert!(doc.dirty, "true-predicate must mark dirty");
    assert_eq!(doc.undo_stack.len(), 1);
    assert!(matches!(
        &doc.undo_stack[0],
        UndoAction::EditEdge { .. }
    ));
}

/// A `false` predicate skips the undo push and dirty flag
/// — the per-frame paths' "no field changed" no-op.
#[test]
fn commit_throttled_edge_drag_skips_on_false_predicate() {
    let (mut doc, er) = doc_with_edge();
    doc.dirty = false;
    let original = doc.mindmap.edges[0].clone();
    doc.commit_throttled_edge_drag(&er, original, |_, _| false);
    assert!(!doc.dirty);
    assert!(doc.undo_stack.is_empty());
}

/// Predicate runs against the *current* edge state (may
/// differ from `original`). Verifies the closure receives
/// the live edge as its first arg and the snapshot as the
/// second.
#[test]
fn commit_throttled_edge_drag_predicate_sees_current_and_original() {
    let (mut doc, er) = doc_with_edge();
    let original = doc.mindmap.edges[0].clone();
    // Mutate the live edge's color so current != original.
    doc.mindmap.edges[0].color = "#abcdef".to_string();
    let mut saw_current_color: Option<String> = None;
    let mut saw_original_color: Option<String> = None;
    doc.commit_throttled_edge_drag(&er, original, |current, original| {
        saw_current_color = Some(current.color.clone());
        saw_original_color = Some(original.color.clone());
        true
    });
    assert_eq!(saw_current_color.as_deref(), Some("#abcdef"));
    assert_ne!(saw_current_color, saw_original_color);
}

/// No-op when the edge_ref doesn't resolve — the snapshot
/// can outlive the edge if a concurrent delete fires.
#[test]
fn commit_throttled_edge_drag_noop_when_edge_missing() {
    let (mut doc, _er) = doc_with_edge();
    let stale_er = EdgeRef::new("nope-from", "nope-to", "cross_link");
    let original = doc.mindmap.edges[0].clone();
    doc.dirty = false;
    doc.commit_throttled_edge_drag(&stale_er, original, |_, _| true);
    assert!(!doc.dirty);
    assert!(doc.undo_stack.is_empty());
}

/// `option_f32_eps_eq` treats values within `EPSILON` as
/// equal, including the `(None, None)` and `(Some, Some)`
/// cases. The `(None, Some)` and `(Some, None)` mismatches
/// are not equal.
#[test]
fn option_f32_eps_eq_treats_epsilon_as_equal() {
    assert!(option_f32_eps_eq(None, None));
    assert!(option_f32_eps_eq(Some(1.0), Some(1.0)));
    assert!(option_f32_eps_eq(
        Some(1.0),
        Some(1.0 + f32::EPSILON / 2.0),
    ));
    assert!(!option_f32_eps_eq(None, Some(0.0)));
    assert!(!option_f32_eps_eq(Some(0.0), None));
    assert!(!option_f32_eps_eq(Some(1.0), Some(2.0)));
}

/// `write_endpoint_field` scrubs the slot back to `None` when
/// a `None` write would leave the state entirely default —
/// the "no undo droppings on unchanged selections" contract.
#[test]
fn write_endpoint_field_scrubs_default_state_on_clear() {
    // Seed a slot with one field set.
    let mut slot: Option<PortalEndpointState> = Some(PortalEndpointState {
        color: Some("#ff0000".to_string()),
        ..Default::default()
    });
    // Clearing the only set field should scrub the slot back to None.
    write_endpoint_field(&mut slot, None::<String>, |s, v| s.color = v);
    assert!(slot.is_none(), "scrub did not collapse to None");
}

/// When the slot has *other* fields set, scrubbing one field
/// to `None` keeps the slot alive — only the all-default
/// state collapses.
#[test]
fn write_endpoint_field_keeps_slot_when_other_fields_remain() {
    let mut slot: Option<PortalEndpointState> = Some(PortalEndpointState {
        color: Some("#ff0000".to_string()),
        text: Some("annotation".to_string()),
        ..Default::default()
    });
    write_endpoint_field(&mut slot, None::<String>, |s, v| s.color = v);
    assert!(slot.is_some(), "slot scrubbed despite remaining fields");
    let s = slot.unwrap();
    assert!(s.color.is_none());
    assert_eq!(s.text.as_deref(), Some("annotation"));
}

/// Writing `Some(value)` lazily forks a default slot when
/// none was set yet — the sibling "lazy install" contract.
#[test]
fn write_endpoint_field_lazily_forks_default_slot() {
    let mut slot: Option<PortalEndpointState> = None;
    write_endpoint_field(
        &mut slot,
        Some("#abcdef".to_string()),
        |s, v| s.color = v,
    );
    assert!(slot.is_some());
    assert_eq!(slot.unwrap().color.as_deref(), Some("#abcdef"));
}
