// SPDX-License-Identifier: MPL-2.0

//! The arena on a chain [`ARENA_DEPTH`] levels deep (design §20 rule 11):
//! every walk, every structural operation and every trait an owned
//! arena carries — `Clone`, `PartialEq`, `Debug`, `Drop` — runs on a
//! default test thread and, through `tests/wasm_font_lock.rs`, on
//! wasm32's default stack in a debug build, where anything that
//! recursed once per level would overflow.

use crate::arena::{Arena, NodeEdge, NodeId};
use std::fmt::Write;

/// The depth of the chain.
pub const ARENA_DEPTH: usize = 100_000;

/// A chain of `depth` nodes, each the only child of the one before,
/// holding its level; and the ids top to leaf. Built by value inserts.
pub fn arena_chain(depth: usize) -> (Arena<usize>, Vec<NodeId>) {
    let mut arena = Arena::new();
    let mut ids = Vec::with_capacity(depth);
    let mut at = arena.new_node(0);
    ids.push(at);
    for level in 1..depth {
        at = at.append_value(level, &mut arena);
        ids.push(at);
    }
    (arena, ids)
}

/// A `fmt::Write` that only counts what it is handed.
struct Count(usize);

impl Write for Count {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0 += s.len();
        Ok(())
    }
}

#[test]
fn test_arena_deep_chain_walks() {
    do_arena_deep_chain_walks();
}

/// The walks: descendants in pre-order, every edge of the traversal,
/// the climb from the leaf, and the children of every level.
pub fn do_arena_deep_chain_walks() {
    let (arena, ids) = arena_chain(ARENA_DEPTH);
    let (top, leaf) = (ids[0], ids[ARENA_DEPTH - 1]);
    let levels: Vec<usize> = top.descendants(&arena).map(|id| *arena[id].get()).collect();
    assert_eq!(
        levels,
        (0..ARENA_DEPTH).collect::<Vec<_>>(),
        "descendants in pre-order"
    );
    let mut edges = top.traverse(&arena);
    assert_eq!(edges.next(), Some(NodeEdge::Start(top)));
    assert_eq!(edges.by_ref().count(), 2 * ARENA_DEPTH - 1, "every start and end");
    assert_eq!(
        leaf.ancestors(&arena).count(),
        ARENA_DEPTH,
        "the climb from the leaf"
    );
    assert_eq!(leaf.ancestors(&arena).last(), Some(top));
    let children: usize = ids.iter().map(|&id| id.children(&arena).count()).sum();
    assert_eq!(children, ARENA_DEPTH - 1, "each level but the leaf has one child");
}

#[test]
fn test_arena_deep_chain_owned_traits() {
    do_arena_deep_chain_owned_traits();
}

/// The arena's `Clone`, `PartialEq`, `Debug` and `Drop` on the chain:
/// the clone is equal, a changed value makes it unequal, the debug
/// form is written in full, and both drop.
pub fn do_arena_deep_chain_owned_traits() {
    let (arena, ids) = arena_chain(ARENA_DEPTH);
    let mut copy = arena.clone();
    assert!(copy == arena, "a clone is equal");
    *copy[ids[ARENA_DEPTH - 1]].get_mut() += 1;
    assert!(copy != arena, "a changed leaf is not");
    let mut written = Count(0);
    write!(written, "{arena:?}").expect("a counting writer does not fail");
    assert!(
        written.0 > ARENA_DEPTH,
        "the debug form names every node ({} bytes)",
        written.0
    );
    drop(copy);
    drop(arena);
}

#[test]
fn test_arena_deep_chain_structural_operations() {
    do_arena_deep_chain_structural_operations();
}

/// The structural operations at the bottom and the top of the chain:
/// a value insert and a same-row re-arrangement at the leaf, a refused
/// cycle (the whole chain moved below its own leaf), a move of the
/// whole chain below another node, and the removal of the whole chain,
/// which frees every slot.
pub fn do_arena_deep_chain_structural_operations() {
    let (mut arena, ids) = arena_chain(ARENA_DEPTH);
    let (top, leaf) = (ids[0], ids[ARENA_DEPTH - 1]);
    let parent = ids[ARENA_DEPTH - 2];
    let sibling = parent.append_value(ARENA_DEPTH, &mut arena);
    assert_eq!(parent.children(&arena).collect::<Vec<_>>(), vec![leaf, sibling]);
    leaf.detach(&mut arena);
    leaf.reattach_in_row(parent, Some(sibling), &mut arena)
        .expect("the leaf goes back into its own row");
    assert_eq!(
        parent.children(&arena).collect::<Vec<_>>(),
        vec![sibling, leaf],
        "the leaf re-arranged after its sibling"
    );

    assert_eq!(
        leaf.checked_append(top, &mut arena),
        Err(crate::arena::ArenaError::Cycle { node: top, to: leaf }),
        "the chain cannot hang below its own leaf"
    );
    let other = arena.new_node(usize::MAX);
    other
        .checked_append(top, &mut arena)
        .expect("the chain moves below another node");
    assert_eq!(
        leaf.ancestors(&arena).last(),
        Some(other),
        "the whole chain came along"
    );
    assert_eq!(other.descendants(&arena).count(), ARENA_DEPTH + 2);

    top.remove_subtree(&mut arena);
    assert_eq!(arena.live_count(), 1, "only the other node is left");
    assert!(
        ids.iter().all(|&id| id.is_removed(&arena)),
        "every id of the chain is stale"
    );
    assert_eq!(other.children(&arena).count(), 0);
}
