// SPDX-License-Identifier: MPL-2.0

//! The arena's structural operations against a reference model, op by
//! op, under the generational-id rules.
//!
//! The model is a plain one — each node a parent handle and an ordered
//! `Vec` of child handles, a removed node a tombstone — written with
//! none of the arena's code: no links, no free list, no generations. A
//! random sequence of operations runs on both; after every one, every
//! id the arena ever handed out is held to the model: a removed node's
//! id reads nothing (even once its slot holds another node), and a live
//! node's value, parent, row neighbors, children both ways, ancestors,
//! descendants and traversal are the model's. An operation the model
//! refuses must be refused by the arena with the same error, and leave
//! it as it was.

use crate::arena::{Arena, ArenaError, NodeEdge, NodeId};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use std::collections::VecDeque;

/// How many seeds [`do_arena_matches_a_reference_model`] runs.
const SEEDS: u64 = 24;
/// How many operations each seed runs.
const STEPS: usize = 600;

/// One node of the model.
#[derive(Clone)]
struct Model {
    value: u32,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// The reference: every node ever created, by handle (creation order);
/// `None` once removed. `ids[h]` is the arena's id for handle `h`.
///
/// It models the slots too, as the arena documents them: `free` holds
/// the freed slots oldest first — the order a removal frees them in,
/// each node after its subtree, the node itself last — and
/// `generations` each slot's next generation; a slot freed at
/// `u32::MAX` is retired and never joins `free`. Every new node must
/// take the head of `free`, or a new slot at the end when it is empty.
#[derive(Default)]
struct Reference {
    nodes: Vec<Option<Model>>,
    ids: Vec<NodeId>,
    free: VecDeque<usize>,
    generations: Vec<u32>,
}

impl Reference {
    fn live(&self) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|&h| self.nodes[h].is_some())
            .collect()
    }

    fn removed(&self) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|&h| self.nodes[h].is_none())
            .collect()
    }

    /// The slot of handle `h`'s id, counted from 0.
    fn slot(&self, h: usize) -> usize {
        let one_based: usize = self.ids[h].to_string().parse().expect("a slot number");
        one_based - 1
    }

    /// The arena handed out `new` for a node holding `value`: it must
    /// be the slot at the head of `free` at its next generation, or a
    /// new slot at the end — and an id never handed out before.
    fn created(&mut self, new: NodeId, value: u32, done: &mut Done) -> usize {
        assert!(
            !self.ids.contains(&new),
            "a new node's id {new:?} repeats an id handed out before"
        );
        let slot = match self.free.pop_front() {
            Some(slot) => {
                done.reused += 1;
                slot
            }
            None => {
                self.generations.push(0);
                self.generations.len() - 1
            }
        };
        assert_eq!(
            format!("{new:?}"),
            format!("NodeId({}@{})", slot + 1, self.generations[slot]),
            "a new node takes the oldest free slot, at its next generation"
        );
        self.create(new, value)
    }

    fn node(&self, h: usize) -> &Model {
        self.nodes[h].as_ref().expect("a live handle")
    }

    fn node_mut(&mut self, h: usize) -> &mut Model {
        self.nodes[h].as_mut().expect("a live handle")
    }

    /// `h` and every node above it, nearest first.
    fn ancestors(&self, h: usize) -> Vec<usize> {
        let mut out = vec![h];
        while let Some(parent) = self.node(*out.last().expect("non-empty")).parent {
            out.push(parent);
        }
        out
    }

    /// `h` and every node below it in pre-order, children first to last,
    /// through a stack of its own.
    fn descendants(&self, h: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![h];
        while let Some(next) = stack.pop() {
            out.push(next);
            stack.extend(self.node(next).children.iter().rev());
        }
        out
    }

    /// The edges of a depth-first walk from `h`.
    fn traverse(&self, h: usize) -> Vec<NodeEdge> {
        let mut out = Vec::new();
        let mut stack = vec![(h, false)];
        while let Some((next, closing)) = stack.pop() {
            if closing {
                out.push(NodeEdge::End(self.ids[next]));
                continue;
            }
            out.push(NodeEdge::Start(self.ids[next]));
            stack.push((next, true));
            stack.extend(self.node(next).children.iter().rev().map(|&c| (c, false)));
        }
        out
    }

    fn create(&mut self, id: NodeId, value: u32) -> usize {
        self.nodes.push(Some(Model {
            value,
            parent: None,
            children: Vec::new(),
        }));
        self.ids.push(id);
        self.nodes.len() - 1
    }

    fn detach(&mut self, h: usize) {
        if let Some(parent) = self.node(h).parent {
            self.node_mut(parent).children.retain(|&c| c != h);
        }
        self.node_mut(h).parent = None;
    }

    fn attach(&mut self, h: usize, parent: usize, at: usize) {
        self.node_mut(h).parent = Some(parent);
        self.node_mut(parent).children.insert(at, h);
    }

    fn position(&self, h: usize) -> usize {
        let parent = self.node(h).parent.expect("in a row");
        self.node(parent)
            .children
            .iter()
            .position(|&c| c == h)
            .expect("in its parent's row")
    }

    fn remove_subtree(&mut self, h: usize) {
        self.detach(h);
        // Post-order, children first to last: each node after its
        // subtree, `h` last.
        let mut order = Vec::new();
        let mut stack = vec![(h, false)];
        while let Some((next, closing)) = stack.pop() {
            if closing {
                order.push(next);
                continue;
            }
            stack.push((next, true));
            stack.extend(self.node(next).children.iter().rev().map(|&c| (c, false)));
        }
        for gone in order {
            self.nodes[gone] = None;
            let slot = self.slot(gone);
            if self.generations[slot] == u32::MAX {
                continue;
            }
            self.generations[slot] += 1;
            self.free.push_back(slot);
        }
    }

    /// The model's refusal of moving `h` below `parent`, the arena's
    /// cycle rule: `h` is `parent` or above it.
    fn cycle(&self, h: usize, parent: usize) -> bool {
        self.ancestors(parent).contains(&h)
    }
}

/// Every id ever handed out, held to the model (see the module docs).
/// Returns how many removed ids were checked to read nothing.
fn check(context: &str, arena: &Arena<u32>, model: &Reference, full: bool) -> usize {
    let mut stale = 0;
    assert_eq!(arena.live_count(), model.live().len(), "{context}: live count");
    for (h, slot) in model.nodes.iter().enumerate() {
        let id = model.ids[h];
        let Some(m) = slot else {
            assert!(
                arena.get(id).is_none(),
                "{context}: removed {id:?} still reads a node"
            );
            assert!(id.is_removed(arena), "{context}: removed {id:?} is not removed");
            stale += 1;
            continue;
        };
        let node = arena
            .get(id)
            .unwrap_or_else(|| panic!("{context}: live {id:?} reads nothing"));
        assert_eq!(*node.get(), m.value, "{context}: value of {id:?}");
        assert_eq!(
            node.parent(),
            m.parent.map(|p| model.ids[p]),
            "{context}: parent of {id:?}"
        );
        assert_eq!(
            id.parent(arena),
            node.parent(),
            "{context}: NodeId::parent of {id:?}"
        );
        assert_eq!(
            (
                id.first_child(arena),
                id.last_child(arena),
                id.previous_sibling(arena),
                id.next_sibling(arena)
            ),
            (
                node.first_child(),
                node.last_child(),
                node.previous_sibling(),
                node.next_sibling()
            ),
            "{context}: the NodeId forms of {id:?}'s links"
        );
        let children: Vec<NodeId> = m.children.iter().map(|&c| model.ids[c]).collect();
        assert_eq!(
            id.children(arena).collect::<Vec<_>>(),
            children,
            "{context}: children of {id:?}"
        );
        let backward: Vec<NodeId> = children.iter().rev().copied().collect();
        assert_eq!(
            id.children(arena).rev().collect::<Vec<_>>(),
            backward,
            "{context}: children of {id:?}, from the back"
        );
        assert_eq!(
            node.first_child(),
            children.first().copied(),
            "{context}: first child of {id:?}"
        );
        assert_eq!(
            node.last_child(),
            children.last().copied(),
            "{context}: last child of {id:?}"
        );
        let (previous, next) = match m.parent {
            Some(parent) => {
                let row = &model.node(parent).children;
                let at = model.position(h);
                (
                    at.checked_sub(1).map(|i| model.ids[row[i]]),
                    row.get(at + 1).map(|&c| model.ids[c]),
                )
            }
            None => (None, None),
        };
        assert_eq!(
            node.previous_sibling(),
            previous,
            "{context}: previous sibling of {id:?}"
        );
        assert_eq!(node.next_sibling(), next, "{context}: next sibling of {id:?}");
        let above: Vec<NodeId> = model.ancestors(h).into_iter().map(|a| model.ids[a]).collect();
        assert_eq!(
            id.ancestors(arena).collect::<Vec<_>>(),
            above,
            "{context}: ancestors of {id:?}"
        );
        if full || m.parent.is_none() {
            let below: Vec<NodeId> = model.descendants(h).into_iter().map(|d| model.ids[d]).collect();
            assert_eq!(
                id.descendants(arena).collect::<Vec<_>>(),
                below,
                "{context}: descendants of {id:?}"
            );
            assert_eq!(
                id.traverse(arena).collect::<Vec<_>>(),
                model.traverse(h),
                "{context}: traverse of {id:?}"
            );
        }
    }
    stale
}

/// What a sweep did, so a precondition can say the sweep reached each
/// path it claims to.
#[derive(Default, Debug)]
struct Done {
    reused: usize,
    stale_checked: usize,
    cycles_refused: usize,
    moved: usize,
    reattached: usize,
    removed: usize,
    no_parent_refused: usize,
    reattach_refused: usize,
    stale_refused: usize,
    stale_ignored: usize,
}

/// Pick a handle at random from `handles`.
fn pick(rng: &mut StdRng, handles: &[usize]) -> usize {
    handles[rng.random_range(0..handles.len())]
}

/// Run one operation, chosen at random, on both the arena and the model.
/// Now and then `h` or `other` is a removed handle, whose id is stale:
/// the operation must then refuse it as documented, or ignore it, and
/// leave the arena as it was.
fn step(rng: &mut StdRng, arena: &mut Arena<u32>, model: &mut Reference, done: &mut Done) {
    let live = model.live();
    let gone = model.removed();
    let value = rng.random_range(0..1_000_000);
    if live.is_empty() {
        let id = arena.new_node(value);
        model.created(id, value, done);
        return;
    }
    let choose = |rng: &mut StdRng| {
        if !gone.is_empty() && rng.random_bool(0.12) {
            pick(rng, &gone)
        } else {
            pick(rng, &live)
        }
    };
    let h = choose(rng);
    let other = choose(rng);
    let (id, other_id) = (model.ids[h], model.ids[other]);
    let (h_stale, other_stale) = (model.nodes[h].is_none(), model.nodes[other].is_none());
    if h_stale || other_stale {
        stale_step(rng, arena, model, done, h, other);
        return;
    }
    match rng.random_range(0..13) {
        0 => {
            let new = arena.new_node(value);
            model.created(new, value, done);
        }
        1 => {
            let new = id.append_value(value, arena);
            let n = model.created(new, value, done);
            let at = model.node(h).children.len();
            model.attach(n, h, at);
        }
        2 => {
            let new = id.prepend_value(value, arena);
            let n = model.created(new, value, done);
            model.attach(n, h, 0);
        }
        3 | 4 => {
            let Some(parent) = model.node(h).parent else {
                return;
            };
            let after = rng.random_bool(0.5);
            let new = if after {
                id.insert_after_value(value, arena)
            } else {
                id.insert_before_value(value, arena)
            };
            let at = model.position(h) + usize::from(after);
            let n = model.created(new, value, done);
            model.attach(n, parent, at);
        }
        5 => {
            id.detach(arena);
            model.detach(h);
        }
        6 => {
            // Remove, mostly a small subtree, so the model keeps nodes.
            if model.descendants(h).len() > 8 && rng.random_bool(0.8) {
                return;
            }
            id.remove_subtree(arena);
            model.remove_subtree(h);
            done.removed += 1;
        }
        7 | 8 => {
            // Move `other` below `h`, at the end or the front.
            let append = rng.random_bool(0.5);
            let result = if append {
                id.checked_append(other_id, arena)
            } else {
                id.checked_prepend(other_id, arena)
            };
            if model.cycle(other, h) {
                assert_eq!(
                    result,
                    Err(ArenaError::Cycle {
                        node: other_id,
                        to: id
                    })
                );
                done.cycles_refused += 1;
            } else {
                assert_eq!(result, Ok(()));
                model.detach(other);
                let at = if append { model.node(h).children.len() } else { 0 };
                model.attach(other, h, at);
                done.moved += 1;
            }
        }
        9 | 10 => {
            // Move `other` beside `h`.
            let after = rng.random_bool(0.5);
            let result = if after {
                id.checked_insert_after(other_id, arena)
            } else {
                id.checked_insert_before(other_id, arena)
            };
            let Some(parent) = model.node(h).parent else {
                assert_eq!(result, Err(ArenaError::NoParent(id)));
                done.no_parent_refused += 1;
                return;
            };
            if other == h {
                assert_eq!(result, Err(ArenaError::Cycle { node: id, to: id }));
                done.cycles_refused += 1;
            } else if model.cycle(other, parent) {
                assert_eq!(
                    result,
                    Err(ArenaError::Cycle {
                        node: other_id,
                        to: model.ids[parent]
                    })
                );
                done.cycles_refused += 1;
            } else {
                assert_eq!(result, Ok(()));
                model.detach(other);
                let at = model.position(h) + usize::from(after);
                model.attach(other, parent, at);
                done.moved += 1;
            }
        }
        11 => {
            // Re-arrange `h` within its row: detach it, then put it back
            // at a random place in the same row.
            let Some(parent) = model.node(h).parent else {
                return;
            };
            let parent_id = model.ids[parent];
            id.detach(arena);
            model.detach(h);
            let row = model.node(parent).children.clone();
            let at = rng.random_range(0..=row.len());
            let previous = at.checked_sub(1).map(|i| model.ids[row[i]]);
            assert_eq!(id.reattach_in_row(parent_id, previous, arena), Ok(()));
            model.attach(h, parent, at);
            done.reattached += 1;
        }
        _ => {
            // The same-row re-attach's O(1) refusals, none of which
            // changes anything: a node still in a row; for a detached
            // node, itself as its parent, and a previous sibling that is
            // not under the parent named.
            let before = arena.clone();
            if let Some(parent) = model.node(h).parent {
                let result = id.reattach_in_row(model.ids[parent], None, arena);
                assert_eq!(result, Err(ArenaError::Attached(id)));
            } else if rng.random_bool(0.3) {
                let result = id.reattach_in_row(id, None, arena);
                assert_eq!(
                    result,
                    Err(ArenaError::Cycle { node: id, to: id }),
                    "a node as its own parent"
                );
            } else {
                // `h` itself qualifies, so there is always one.
                let Some(&elsewhere) = live.iter().find(|&&y| model.node(y).parent != Some(other)) else {
                    return;
                };
                let result = id.reattach_in_row(other_id, Some(model.ids[elsewhere]), arena);
                let expected = if other == h {
                    ArenaError::Cycle { node: id, to: id }
                } else {
                    ArenaError::NotAChild {
                        parent: other_id,
                        node: model.ids[elsewhere],
                    }
                };
                assert_eq!(result, Err(expected));
            }
            assert!(*arena == before, "a refused re-attach changed the arena");
            done.reattach_refused += 1;
        }
    }
}

/// One operation with a stale `h` or `other`: the documented refusal,
/// in the documented order (the node the method is called on first,
/// then the one it is handed), or for `detach` and `remove_subtree` on
/// a stale id nothing at all — and the arena as it was.
fn stale_step(
    rng: &mut StdRng,
    arena: &mut Arena<u32>,
    model: &Reference,
    done: &mut Done,
    h: usize,
    other: usize,
) {
    let (id, other_id) = (model.ids[h], model.ids[other]);
    let (h_stale, other_stale) = (model.nodes[h].is_none(), model.nodes[other].is_none());
    let before = arena.clone();
    let first_stale = if h_stale { id } else { other_id };
    match rng.random_range(0..8) {
        0 if h_stale => {
            id.detach(arena);
            done.stale_ignored += 1;
        }
        1 if h_stale => {
            id.remove_subtree(arena);
            done.stale_ignored += 1;
        }
        0..=2 => {
            assert_eq!(
                id.checked_append(other_id, arena),
                Err(ArenaError::Stale(first_stale))
            );
            done.stale_refused += 1;
        }
        3 => {
            assert_eq!(
                id.checked_prepend(other_id, arena),
                Err(ArenaError::Stale(first_stale))
            );
            done.stale_refused += 1;
        }
        4 => {
            assert_eq!(
                id.checked_insert_after(other_id, arena),
                Err(ArenaError::Stale(first_stale))
            );
            done.stale_refused += 1;
        }
        5 => {
            assert_eq!(
                id.checked_insert_before(other_id, arena),
                Err(ArenaError::Stale(first_stale))
            );
            done.stale_refused += 1;
        }
        _ => {
            // The re-attach: a stale node, else (for a detached node) a
            // stale parent or a stale previous sibling.
            let result = if h_stale {
                id.reattach_in_row(other_id, None, arena)
            } else if model.node(h).parent.is_some() {
                assert_eq!(
                    id.reattach_in_row(other_id, None, arena),
                    Err(ArenaError::Attached(id)),
                    "an attached node is refused before its stale parent"
                );
                assert!(*arena == before, "a refused operation changed the arena");
                done.stale_refused += 1;
                return;
            } else if rng.random_bool(0.5) {
                id.reattach_in_row(other_id, None, arena)
            } else {
                let Some(parent) = model.live().into_iter().find(|&p| p != h) else {
                    return;
                };
                let result = id.reattach_in_row(model.ids[parent], Some(other_id), arena);
                assert_eq!(
                    result,
                    Err(ArenaError::Stale(other_id)),
                    "a stale previous sibling"
                );
                assert!(*arena == before, "a refused operation changed the arena");
                done.stale_refused += 1;
                return;
            };
            assert_eq!(result, Err(ArenaError::Stale(first_stale)));
            done.stale_refused += 1;
        }
    }
    assert!(*arena == before, "an operation on a stale id changed the arena");
    let _ = other_stale;
}

#[test]
fn test_arena_matches_a_reference_model() {
    do_arena_matches_a_reference_model();
}

/// Random sequences of every structural operation, on the arena and on
/// the model (`Reference`), held together after every operation
/// (`check`); every few operations the descendants and traversal of
/// every node too. Preconditions: each kind of operation ran, refused
/// cycles included, freed slots were reused — so the stale-id checks
/// were made against slots that hold another node — and stale ids were
/// handed to every operation that takes one.
pub fn do_arena_matches_a_reference_model() {
    let mut done = Done::default();
    for seed in 0..SEEDS {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut arena = Arena::new();
        let mut model = Reference::default();
        let first = arena.new_node(0);
        model.created(first, 0, &mut done);
        for i in 0..STEPS {
            step(&mut rng, &mut arena, &mut model, &mut done);
            done.stale_checked += check(&format!("seed {seed} step {i}"), &arena, &model, i % 50 == 0);
        }
        // Remove everything left: the arena must end with no live node.
        for top in model.live() {
            if model.nodes[top].as_ref().is_some_and(|m| m.parent.is_none()) {
                model.ids[top].remove_subtree(&mut arena);
                model.remove_subtree(top);
            }
        }
        done.stale_checked += check(&format!("seed {seed} emptied"), &arena, &model, true);
        assert_eq!(arena.live_count(), 0, "seed {seed}: a node outlived its tree");
    }
    for (what, count) in [
        ("slots reused", done.reused),
        ("stale ids checked", done.stale_checked),
        ("cycles refused", done.cycles_refused),
        ("moves", done.moved),
        ("same-row re-attaches", done.reattached),
        ("removals", done.removed),
        ("sibling inserts refused for no parent", done.no_parent_refused),
        ("same-row re-attaches refused", done.reattach_refused),
        ("stale ids refused", done.stale_refused),
        ("stale ids ignored", done.stale_ignored),
    ] {
        assert!(count > 0, "precondition: the sweep made {what} ({done:?})");
    }
}
