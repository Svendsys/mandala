// SPDX-License-Identifier: MPL-2.0

//! The fuzz properties of authored glyph-layer mutations (design D29;
//! TEST_CONVENTIONS §T14): the run and fill variants of
//! [`GfxMutation`] a map can carry (`RunOrigin`, `RunOpacity`,
//! `RunOverrides`, `FillColor`, `FillOpacity`, `FillRect`;
//! `format/mutations.md`), applied by the journaled walker to a glyph
//! tree of runs and fills.
//!
//! The glyph_mutation_raw fuzz target reads its bytes as JSON — a list
//! of mutations or one — the way a file spells them, and applies what
//! parses; glyph_mutation_structured reads its bytes as choices
//! ([`Choices`](crate::util::tests::choices::Choices)) and builds both
//! the tree (box and path runs, fillers among their glyphs, placed
//! positions that are not finite) and the mutations, with values a JSON
//! literal can reach (`1e39` reads as infinity) and values it cannot
//! (`NaN`), ranges from zero to `usize::MAX`, backward and empty.
//!
//! After every apply:
//!
//! - **No panic.**
//! - **No non-finite box reaches an ancestor:** every subtree box the
//!   hit-test index holds is finite, and so is the tree's.
//! - **No allocation unbounded by the input:** a run's overrides are
//!   empty or exactly as long as its placed glyphs, whatever range a
//!   mutation names.
//! - **`same_state` is total:** every element holds the same state as
//!   its clone, `NaN`s included.
//! - **Interpolation is total and keeps its ends:** between each
//!   element's state before and after, at `t` in `0..=1` with either
//!   snap, it does not panic, and at `0` it holds the state before and
//!   at `1` the state after.
//! - **The journal is exact:** reverting restores every element as it
//!   was, and replaying reproduces the walk.

use std::sync::Arc;

use glam::Vec2;
use indextree::NodeId;

use crate::document::values::Rgba;
use crate::engine::anchor::Anchors;
use crate::engine::element::TreeElement;
use crate::engine::interpolate::{Interpolable, Snap};
use crate::engine::journal::JournalBuilder;
use crate::engine::tree::Placement;
use crate::flow::tests::support::synthetic;
use crate::flow::{PlacedGlyph, PlacedRun, Rect, FILLER_SPAN};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::fill::{FillData, FillShape};
use crate::gfx_structs::mutator::{GfxMutation, GfxMutator};
use crate::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunOverrideOp, RunShape};
use crate::gfx_structs::tree::{GfxMutatorTree, GfxTree};
use crate::util::tests::choices::Choices;

/// The most placed glyphs a structured run holds.
pub const MAX_RUN_GLYPHS: u32 = 64;

/// The most mutations one input applies.
pub const MAX_MUTATIONS: u32 = 32;

/// The mutations `bytes` spell as JSON: a list, one mutation, or an
/// object whose `mutations` is a list (the shape `format/mutations.md`'s
/// examples take). `None` when they spell none of these.
pub fn parse_mutations(bytes: &[u8]) -> Option<Vec<GfxMutation>> {
    #[derive(serde::Deserialize)]
    struct Listed {
        mutations: Vec<GfxMutation>,
    }
    serde_json::from_slice::<Vec<GfxMutation>>(bytes)
        .ok()
        .or_else(|| {
            serde_json::from_slice::<GfxMutation>(bytes)
                .ok()
                .map(|one| vec![one])
        })
        .or_else(|| {
            serde_json::from_slice::<Listed>(bytes)
                .ok()
                .map(|listed| listed.mutations)
        })
}

/// Every property of `bytes` read as JSON mutations
/// ([`parse_mutations`]) applied to the default tree: the
/// glyph_mutation_raw target's whole body. Bytes that are not mutations
/// are refused by the reader, which must not panic; that is all there is
/// to check of them.
pub fn check_glyph_mutations_json(bytes: &[u8]) -> Result<(), String> {
    let Some(mutations) = parse_mutations(bytes) else {
        return Ok(());
    };
    // Every mutation goes to every element, as a scope such as
    // `SelfAndDescendants` sends one list to elements of every kind.
    let (mut tree, targets) = build_tree(&mut Choices::new(&[]));
    let assigned: Vec<(usize, GfxMutation)> = (0..targets.len())
        .flat_map(|target| mutations.iter().map(move |mutation| (target, mutation.clone())))
        .collect();
    apply_and_check(&mut tree, &targets, assigned)
}

/// Every property of the tree and mutations `bytes` choose: the
/// glyph_mutation_structured target's whole body.
pub fn check_glyph_mutations_structured(bytes: &[u8]) -> Result<(), String> {
    check_glyph_mutations_generated(&mut Choices::new(bytes))
}

/// [`check_glyph_mutations_structured`] for choices from any source.
pub fn check_glyph_mutations_generated(choices: &mut Choices<'_>) -> Result<(), String> {
    let (mut tree, targets, assigned) = generate(choices);
    apply_and_check(&mut tree, &targets, assigned)
}

/// The tree, its targets, and the mutations each target is sent, that
/// `choices` describe.
pub fn generate(choices: &mut Choices<'_>) -> (GfxTree, Vec<Target>, Vec<(usize, GfxMutation)>) {
    let (tree, targets) = build_tree(choices);
    let count = choices.below(MAX_MUTATIONS);
    let assigned: Vec<(usize, GfxMutation)> = (0..count)
        .map(|_| (choices.index(targets.len()), mutation(choices)))
        .collect();
    (tree, targets, assigned)
}

/// Where a mutation can be sent: a path of channels from the root.
pub type Target = Vec<usize>;

/// The tree the mutations meet, and the channel path of every element in
/// it: under the root a box run, a path run, a fill and an area (which
/// run and fill mutations must leave alone), then a void holding a path
/// run and a fill, so that a box has an ancestor below the root.
fn build_tree(choices: &mut Choices<'_>) -> (GfxTree, Vec<Target>) {
    let mut tree = GfxTree::new_non_indexed();
    let root = tree.root;
    let add = |tree: &mut GfxTree, parent: NodeId, element: GfxElement| {
        tree.insert_child(parent, element, Placement::Last)
            .expect("the parent was just inserted")
    };
    let glyphs = |choices: &mut Choices<'_>| 1 + choices.below(MAX_RUN_GLYPHS) as usize;
    let box_glyphs = glyphs(choices);
    add(
        &mut tree,
        root,
        GfxElement::new_run(box_run(box_glyphs, choices), 0, 1),
    );
    let path_glyphs = glyphs(choices);
    add(
        &mut tree,
        root,
        GfxElement::new_run(path_run(path_glyphs, choices), 1, 2),
    );
    add(&mut tree, root, GfxElement::new_fill(fill(choices), 2, 3));
    add(
        &mut tree,
        root,
        GfxElement::new_area_non_indexed(
            GlyphArea::new(12.0, 14.0, Vec2::new(5.0, 5.0), Vec2::new(40.0, 20.0)),
            3,
        ),
    );
    let void = add(&mut tree, root, GfxElement::new_void(4));
    let nested_glyphs = glyphs(choices);
    add(
        &mut tree,
        void,
        GfxElement::new_run(path_run(nested_glyphs, choices), 0, 5),
    );
    add(&mut tree, void, GfxElement::new_fill(fill(choices), 1, 6));
    let targets = vec![vec![0], vec![1], vec![2], vec![3], vec![4, 0], vec![4, 1]];
    (tree, targets)
}

/// A box run of `glyphs` laid-out glyphs, its size finite as shaping
/// makes it.
fn box_run(glyphs: usize, choices: &mut Choices<'_>) -> GlyphRunData {
    let size = Vec2::new(choices.below(500) as f32, choices.below(200) as f32);
    let shape = RunShape::Box(Arc::new(crate::flow::ShapedBox {
        buffer: Arc::new(crate::font::Buffer::new_empty(crate::font::Metrics::new(
            10.0, 12.0,
        ))),
        size,
        glyph_spans: (0..glyphs).map(|i| (0, i..i + 1)).collect(),
    }));
    GlyphRunData::new(shape, any_vec2(choices), GlyphOverrides::none(), 1.0, None)
        .expect("no overrides fit any run")
}

/// A path run of `glyphs` placed glyphs, some of them filler, placed at
/// positions that need not be finite (a hand-built or corrupted
/// placement: the run's box must skip what is not).
fn path_run(glyphs: usize, choices: &mut Choices<'_>) -> GlyphRunData {
    let mut shaped = synthetic(&vec![10.0; glyphs], 20.0);
    for glyph in shaped.glyphs.iter_mut() {
        if choices.one_in(4) {
            glyph.span = FILLER_SPAN;
        }
    }
    let placed = (0..glyphs)
        .map(|i| PlacedGlyph {
            glyph: i as u32,
            pos: if choices.one_in(8) {
                any_vec2(choices)
            } else {
                Vec2::new(10.0 * i as f32, 0.0)
            },
        })
        .collect();
    let shape = RunShape::Path {
        run: Arc::new(shaped),
        placed: Arc::new(PlacedRun {
            glyphs: placed,
            truncated: false,
        }),
    };
    let overrides = if choices.one_in(2) {
        GlyphOverrides::new(glyphs)
    } else {
        GlyphOverrides::none()
    };
    GlyphRunData::new(shape, Vec2::ZERO, overrides, 1.0, None).expect("the overrides are as long as the run")
}

/// A fill whose rectangle is any rectangle.
fn fill(choices: &mut Choices<'_>) -> FillData {
    FillData {
        rect: Rect::new(any_vec2(choices), any_vec2(choices)),
        shape: FillShape::Rect { radius: 4.0 },
        color: Rgba::new(10, 20, 30, 255),
        fade: 2.0,
        opacity: 1.0,
        zoom: None,
    }
}

/// A number: plain, out of `f32`'s range as a JSON literal reads it
/// (`1e39` is infinity), or any bit pattern.
fn any_f32(choices: &mut Choices<'_>) -> f32 {
    match choices.below(4) {
        0 => choices.below(200) as f32 - 100.0,
        1 => *choices.pick(&[
            0.0,
            -0.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::MAX,
            -f32::MAX,
            1e30,
            f32::MIN_POSITIVE,
        ]),
        _ => choices.any_f32(),
    }
}

fn any_vec2(choices: &mut Choices<'_>) -> Vec2 {
    Vec2::new(any_f32(choices), any_f32(choices))
}

/// A range of placed glyphs: inside, past the end, backward, empty, or
/// to `usize::MAX`.
fn range(choices: &mut Choices<'_>) -> std::ops::Range<usize> {
    let end = |choices: &mut Choices<'_>| match choices.below(4) {
        0 => choices.index(MAX_RUN_GLYPHS as usize + 4),
        1 => usize::MAX,
        2 => 0,
        _ => choices.next_u32() as usize,
    };
    let start = end(choices);
    start..end(choices)
}

/// One authored run or fill mutation.
fn mutation(choices: &mut Choices<'_>) -> GfxMutation {
    match choices.below(9) {
        0 => GfxMutation::RunOrigin(any_vec2(choices)),
        1 => GfxMutation::RunOpacity(any_f32(choices)),
        2 => GfxMutation::RunOverrides(RunOverrideOp::Offset {
            range: range(choices),
            value: any_vec2(choices),
        }),
        3 => GfxMutation::RunOverrides(RunOverrideOp::Color {
            range: range(choices),
            value: choices
                .one_in(2)
                .then(|| Rgba::new(choices.below(256) as u8, 0, 0, choices.below(256) as u8)),
        }),
        4 => GfxMutation::RunOverrides(RunOverrideOp::Scale {
            range: range(choices),
            value: any_f32(choices),
        }),
        5 => GfxMutation::RunOverrides(RunOverrideOp::Clear),
        6 => GfxMutation::FillColor(Rgba::new(
            choices.below(256) as u8,
            choices.below(256) as u8,
            choices.below(256) as u8,
            choices.below(256) as u8,
        )),
        7 => GfxMutation::FillOpacity(any_f32(choices)),
        _ => GfxMutation::FillRect(Rect::new(any_vec2(choices), any_vec2(choices))),
    }
}

/// Every node in pre-order with its element.
fn snapshot(tree: &GfxTree) -> Vec<(NodeId, GfxElement)> {
    tree.descendants()
        .filter_map(|id| tree.get(id).map(|node| (id, node.get().clone())))
        .collect()
}

/// Where two snapshots first differ, if they do.
fn differs(a: &[(NodeId, GfxElement)], b: &[(NodeId, GfxElement)]) -> Option<String> {
    if a.len() != b.len() {
        return Some(format!("{} elements against {}", a.len(), b.len()));
    }
    a.iter()
        .zip(b)
        .find(|((ia, ea), (ib, eb))| ia != ib || !ea.same_state(eb))
        .map(|((id, element), (_, other))| format!("{id}: {element:?} against {other:?}"))
}

/// Apply each mutation in `assigned` (a target index and a mutation) to
/// its target through one journaled walk from the root, then check
/// every property.
fn apply_and_check(
    tree: &mut GfxTree,
    targets: &[Target],
    assigned: Vec<(usize, GfxMutation)>,
) -> Result<(), String> {
    let mut mutators = GfxMutatorTree::new();
    let mroot = mutators.root;
    // One macro per top-level target, and under a void mutator one per
    // nested target, each holding its mutations in order.
    let mut top: Vec<(usize, Vec<GfxMutation>)> = Vec::new();
    let mut nested: Vec<(usize, Vec<GfxMutation>)> = Vec::new();
    for (target, mutation) in assigned {
        let path = &targets[target];
        let (row, channel) = match path.as_slice() {
            [channel] => (&mut top, *channel),
            [_, channel] => (&mut nested, *channel),
            _ => continue,
        };
        match row.iter_mut().find(|(c, _)| *c == channel) {
            Some((_, list)) => list.push(mutation),
            None => row.push((channel, vec![mutation])),
        }
    }
    top.sort_by_key(|(channel, _)| *channel);
    nested.sort_by_key(|(channel, _)| *channel);
    for (channel, list) in top {
        if channel != 4 {
            mutators
                .insert_child(mroot, GfxMutator::new_macro(list, channel), Placement::Last)
                .map_err(|error| format!("a mutator row: {error:?}"))?;
        }
    }
    if !nested.is_empty() {
        let void = mutators
            .insert_child(mroot, GfxMutator::new_void(4), Placement::Last)
            .map_err(|error| format!("a mutator row: {error:?}"))?;
        for (channel, list) in nested {
            mutators
                .insert_child(void, GfxMutator::new_macro(list, channel), Placement::Last)
                .map_err(|error| format!("a mutator row: {error:?}"))?;
        }
    }

    let before = snapshot(tree);
    let mut builder = JournalBuilder::new();
    let root = tree.root;
    mutators
        .apply_at_journaled(tree, &Anchors::Nodes(vec![root]), |_, _| (), &mut builder)
        .map_err(|error| format!("the walk refused: {error:?}"))?;
    let after = snapshot(tree);
    let journal = builder.close(tree);

    for (id, element) in &after {
        if !element.same_state(&element.clone()) {
            return Err(format!(
                "{id} does not hold the same state as its clone: {element:?}"
            ));
        }
        if let Some(run) = element.glyph_run() {
            let overrides = run.overrides().len();
            if overrides != 0 && overrides != run.shaped().glyph_count() {
                return Err(format!(
                    "{id}: {overrides} overrides for {} placed glyphs",
                    run.shaped().glyph_count()
                ));
            }
        }
    }
    boxes_are_finite(tree)?;
    for ((id, from), (_, to)) in before.iter().zip(&after) {
        for snap in [Snap::Start, Snap::End] {
            for t in [0.0, 0.25, 0.5, 1.0] {
                let between = GfxElement::interpolate(from, to, t, snap);
                if t == 0.0 && !between.same_state(from) {
                    return Err(format!(
                        "{id}: at t = 0 ({snap:?}) the state is not the one before"
                    ));
                }
                if t == 1.0 && !between.same_state(to) {
                    return Err(format!(
                        "{id}: at t = 1 ({snap:?}) the state is not the one after"
                    ));
                }
            }
        }
    }
    journal
        .revert(tree)
        .map_err(|error| format!("revert refused: {error:?}"))?;
    if let Some(why) = differs(&snapshot(tree), &before) {
        return Err(format!("revert did not restore the tree: {why}"));
    }
    journal
        .replay(tree)
        .map_err(|error| format!("replay refused: {error:?}"))?;
    if let Some(why) = differs(&snapshot(tree), &after) {
        return Err(format!("replay did not reproduce the walk: {why}"));
    }
    boxes_are_finite(tree)
}

/// Every subtree box the hit-test index holds, and the tree's box, is
/// finite, and a hit test anywhere answers without panicking.
pub fn boxes_are_finite(tree: &mut GfxTree) -> Result<(), String> {
    tree.ensure_subtree_aabbs();
    let ids: Vec<NodeId> = tree.descendants().collect();
    for id in ids {
        if let Some((min, max)) = tree.subtree_aabb(id) {
            if !(min.is_finite() && max.is_finite()) {
                return Err(format!("the subtree box of {id} is {min}..{max}"));
            }
        }
    }
    if let Some((min, max)) = tree.descendants_aabb() {
        if !(min.is_finite() && max.is_finite()) {
            return Err(format!("the tree's box is {min}..{max}"));
        }
    }
    for point in [
        Vec2::ZERO,
        Vec2::new(15.0, 15.0),
        Vec2::splat(f32::NAN),
        Vec2::splat(f32::INFINITY),
    ] {
        let _ = tree.descendant_at(point);
    }
    Ok(())
}
