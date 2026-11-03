// SPDX-License-Identifier: MPL-2.0

//! The fuzz properties of authored glyph-layer mutations (design D29;
//! TEST_CONVENTIONS §T14): the run variants of
//! [`GfxMutation`](crate::gfx_structs::mutator::GfxMutation) a map can carry (`RunOrigin`, `RunOpacity`,
//! `RunOverrides`; `format/mutations.md`), applied by the journaled
//! walker to a glyph tree of runs and an area. The glyph layer has no
//! fill (V1-B12), so a fill mutation in the JSON half does not parse.
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

use crate::arena::NodeId;
use glam::Vec2;

use crate::document::values::Rgba;
use crate::engine::anchor::Anchors;
use crate::engine::element::TreeElement;
use crate::engine::interpolate::{Interpolable, Snap};
use crate::engine::journal::JournalBuilder;
use crate::engine::tree::Placement;
use crate::flow::tests::support::synthetic;
use crate::flow::{PlacedGlyph, PlacedRun, FILLER_SPAN};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutation, GfxMutator};
use crate::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunOverrideOp, RunShape};
use crate::gfx_structs::tree::{GfxMutatorTree, GfxTree};
use crate::util::tests::choices::{encode, Choices};

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

/// The choices [`build_tree`] reads to make the tree the JSON half
/// applies its mutations to: a fixed, committed sequence (found by a
/// seeded search, `Choices::seeded(14)`, redone when V1-B12 took the
/// fills out of the tree), so that the raw target's
/// tree is one whose properties can fail. An empty source builds a tree
/// of one-glyph runs, empty boxes and no overrides, where an override
/// range of any length is clipped to nothing (review of #293, blocking
/// item 2). [`json_tree_holds_what_it_must`] holds it to that, in
/// `glyph_layer_fuzz_tests`.
pub const JSON_TREE: [u32; 181] = [
    3, 3758329380, 2192621188, 1667755775, 1214215971, 1605771418, 4239586978, 1467622202, 2284909942,
    2350978616, 1012444861, 411534631, 3147365897, 1929510698, 2486282666, 2136871276, 1426704726,
    4114148952, 3761754936, 4107941960, 2311611759, 512631264, 1629462704, 4183042205, 1755873444,
    3136863240, 528095454, 2030827960, 749382863, 355519905, 3591732316, 3444651782, 4250555725, 2274637740,
    3525185508, 3003521689, 1807811587, 3075810731, 1007743666, 1388598952, 275397411, 3595427321,
    1745764154, 3382096433, 1226989182, 2252388694, 3557565666, 4051606431, 3666583160, 2547521538,
    1181751203, 2427761749, 3577510732, 4160100164, 1420602107, 3131813687, 1243069568, 3905692288,
    2073667552, 2920541516, 3279823910, 860511193, 1881303136, 1824046202, 1348017712, 419682225, 1727770862,
    1383117778, 2185830777, 315908131, 3774094447, 1548181739, 3602058295, 2766173641, 1160648362,
    4071534152, 3639353980, 2662148473, 1027381412, 734366917, 2304322382, 3111728173, 1999204761,
    2401429916, 1303626999, 3280997447, 1784014710, 2129412088, 4092228688, 4257091182, 1709113730,
    2321961176, 447216144, 3092265312, 280090650, 3739155783, 4235165091, 1837108081, 3194026876, 1581556385,
    2190993112, 2368872978, 3577184107, 1512866426, 3588470132, 3489795718, 1955001045, 1871307200,
    1910094967, 2400123238, 2460603120, 3525712962, 821254452, 2987746058, 3053648330, 1353504282,
    1608322788, 3786684078, 155565068, 3276108848, 2429033954, 3534656642, 1735089008, 2662879676, 233121755,
    1234354669, 1831252497, 654271697, 4055236023, 1490835997, 2127753755, 1097732091, 348607285, 2652765909,
    2537945440, 1761217702, 976706740, 4195424635, 3418085848, 926562044, 2503478365, 621246673, 1073479183,
    3209026598, 686879860, 674206611, 1686015784, 4237905382, 1841989670, 2259869084, 1391133043, 3409476301,
    3340354635, 4136279449, 1058193078, 2879552980, 1938697746, 1797135576, 2351043258, 2469815641,
    2645603195, 2085255328, 2682596317, 1189730386, 3794507639, 1624940483, 1552414353, 1236198969,
    2377668528, 716901350, 3645794232, 379668158, 3830695120, 907807754, 1812828117, 2031252554, 42581712,
    941519793, 4053101315, 3198394187, 491390704,
];

/// Every property of `bytes` read as JSON mutations
/// ([`parse_mutations`]) applied to the tree [`JSON_TREE`] builds: the
/// glyph_mutation_raw target's whole body. Bytes that are not mutations
/// are refused by the reader, which must not panic; that is all there is
/// to check of them.
pub fn check_glyph_mutations_json(bytes: &[u8]) -> Result<(), String> {
    let Some(mutations) = parse_mutations(bytes) else {
        return Ok(());
    };
    // Every mutation goes to every element, as a scope such as
    // `SelfAndDescendants` sends one list to elements of every kind.
    let (mut tree, targets) = build_tree(&mut Choices::new(&encode(&JSON_TREE)));
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
/// it: under the root a box run, a path run and an area (which run
/// mutations must leave alone), then a void holding a path run, so that
/// a box has an ancestor below the root.
pub fn build_tree(choices: &mut Choices<'_>) -> (GfxTree, Vec<Target>) {
    let mut tree = GfxTree::new_non_indexed();
    let root = tree.root();
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
    add(
        &mut tree,
        root,
        GfxElement::new_area_non_indexed(
            GlyphArea::new(12.0, 14.0, Vec2::new(5.0, 5.0), Vec2::new(40.0, 20.0)),
            2,
        ),
    );
    let void = add(&mut tree, root, GfxElement::new_void(3));
    let nested_glyphs = glyphs(choices);
    add(
        &mut tree,
        void,
        GfxElement::new_run(path_run(nested_glyphs, choices), 0, 5),
    );
    let targets = vec![vec![0], vec![1], vec![2], vec![3], vec![3, 0]];
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

/// One authored run mutation.
fn mutation(choices: &mut Choices<'_>) -> GfxMutation {
    match choices.below(6) {
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
        _ => GfxMutation::RunOverrides(RunOverrideOp::Clear),
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
    let mroot = mutators.root();
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
    let root = tree.root();
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

/// What the tree the JSON half applies mutations to must hold, so that
/// its properties can fail there (review of #293, blocking item 2): a
/// box run and two path runs of several glyphs, filler among a path
/// run's, a box of positive size, two runs without overrides
/// (so a range op materializes them) and one with them materialized.
/// `Err` names the first that does not hold.
pub fn json_tree_holds_what_it_must(tree: &GfxTree) -> Result<(), String> {
    let elements: Vec<&GfxElement> = tree
        .descendants()
        .filter_map(|id| tree.get(id).map(|node| node.get()))
        .collect();
    let runs: Vec<&GlyphRunData> = elements
        .iter()
        .filter_map(|element| element.glyph_run())
        .collect();
    let positive = |size: Vec2| size.is_finite() && size.x > 0.0 && size.y > 0.0;
    if runs.len() != 3 {
        return Err(format!("{} runs, not three", runs.len()));
    }
    let boxes: Vec<&&GlyphRunData> = runs
        .iter()
        .filter(|run| matches!(run.shaped(), RunShape::Box(_)))
        .collect();
    match boxes.as_slice() {
        [run] if run.shaped().glyph_count() >= 4 && positive(run.bounds().1) => {}
        _ => return Err("the box run has fewer than 4 glyphs or no positive box".to_string()),
    }
    let paths: Vec<&&GlyphRunData> = runs
        .iter()
        .filter(|run| matches!(run.shaped(), RunShape::Path { .. }))
        .collect();
    if paths.len() != 2 || paths.iter().any(|run| run.shaped().glyph_count() < 4) {
        return Err("two path runs of at least 4 glyphs each".to_string());
    }
    if !paths.iter().any(|run| {
        let count = run.shaped().glyph_count();
        let fillers = (0..count).filter(|&i| run.shaped().is_filler(i)).count();
        fillers > 0 && fillers < count
    }) {
        return Err("no path run mixes filler and text glyphs".to_string());
    }
    let materialized = runs.iter().filter(|run| !run.overrides().is_empty()).count();
    if materialized != 1 {
        return Err(format!("{materialized} runs with overrides, not one"));
    }
    Ok(())
}
