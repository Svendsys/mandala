// SPDX-License-Identifier: MPL-2.0

//! Journals and interpolation over the glyph instantiation: what
//! [`GfxElement`]'s state is (`same_state` — its payload's every field,
//! its flags, channel and id, never its subscribers), a journaled walk
//! of glyph mutations that reverts and replays exactly — a recolor
//! included, which `==` cannot see — and `Interpolable for GfxElement`
//! halfway and at both ends.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2): every `do_*` body
//! has a bench entry in `benches/test_bench.rs`.

use crate::core::primitives::{ApplyOperation, ColorFontRegion, ColorFontRegions, Flag, Flaggable, Range};
use crate::engine::anchor::Anchors;
use crate::engine::element::TreeElement;
use crate::engine::interpolate::Snap;
use crate::engine::journal::JournalBuilder;
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tree::{Placement, TreeError};
use crate::gfx_structs::area::{DeltaGlyphArea, EdgePadding, GlyphArea, GlyphAreaField, OutlineStyle};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::model::{GlyphLine, GlyphModel};
use crate::gfx_structs::mutator::{GfxMutation, GfxMutator};
use crate::gfx_structs::shape::NodeShape;
use crate::gfx_structs::tree::{GfxMutatorTree, GfxTree};
use crate::gfx_structs::util::hitbox::BoundingRectangle;
use crate::gfx_structs::zoom_visibility::ZoomVisibility;
use crate::util::ordered_vec2::OrderedVec2;
use glam::Vec2;
use indextree::NodeId;
use ordered_float::OrderedFloat;
use std::cell::RefCell;
use std::rc::Rc;

/// An area element: text `"glyph"`, scale 10, line height 12, at
/// `(0, 0)`, 100 × 40, one red region over its first two clusters, a
/// background, one hit rectangle, the `Mutable` flag, channel 1, id 7.
fn rich_area() -> GfxElement {
    let mut area = GlyphArea::new_with_str("glyph", 10.0, 12.0, Vec2::ZERO, Vec2::new(100.0, 40.0));
    area.regions = ColorFontRegions::new_from(vec![ColorFontRegion::new(
        Range::new(0, 2),
        None,
        Some([1.0, 0.0, 0.0, 1.0]),
    )]);
    area.background_color = Some([10, 20, 30, 255]);
    area.hitbox.add(BoundingRectangle::at_origin(100.0, 40.0));
    let mut element = GfxElement::new_area_non_indexed_with_id(area, 1, 7);
    element.set_flag(Flag::Mutable);
    element
}

/// Run `f` on the `GlyphArea` inside `element`.
fn with_area(element: &mut GfxElement, f: impl FnOnce(&mut GlyphArea)) {
    if let Some(area) = element.glyph_area_mut() {
        f(area);
    }
}

/// The element's channel, rewritten — no mutation reaches it, so the
/// tests write it directly through a variant match.
fn set_channel(element: &mut GfxElement, to: usize) {
    match element {
        GfxElement::GlyphArea { channel, .. }
        | GfxElement::GlyphModel { channel, .. }
        | GfxElement::GlyphRun { channel, .. }
        | GfxElement::Fill { channel, .. }
        | GfxElement::Void { channel, .. } => *channel = to,
    }
}

// ── same_state ────────────────────────────────────────────────────

#[test]
fn test_gfx_same_state_compares_every_state_field() {
    do_gfx_same_state_compares_every_state_field();
}

/// A change to one field, applied to an element.
type Variation = (&'static str, fn(&mut GfxElement));

/// Run `f` on the `GlyphModel` inside `element`.
fn with_model(element: &mut GfxElement, f: impl FnOnce(&mut GlyphModel)) {
    if let Some(model) = element.glyph_model_mut() {
        f(model);
    }
}

/// One variation per state field of a `GlyphArea` element: every field
/// of the area, then the element's flags, channel and id.
const AREA_VARIATIONS: &[Variation] = &[
    ("text", |e| with_area(e, |a| a.text = "glyphs".to_string())),
    ("scale", |e| with_area(e, |a| a.scale = OrderedFloat(11.0))),
    ("line_height", |e| {
        with_area(e, |a| a.line_height = OrderedFloat(13.0))
    }),
    ("position", |e| {
        with_area(e, |a| a.position = OrderedVec2::new_f32(0.0, 1.0))
    }),
    ("render_bounds", |e| {
        with_area(e, |a| a.render_bounds = OrderedVec2::new_f32(100.0, 41.0))
    }),
    ("regions (color only)", |e| {
        with_area(e, |a| {
            a.regions
                .replace_regions(&ColorFontRegions::new_from(vec![ColorFontRegion::new(
                    Range::new(0, 2),
                    None,
                    Some([0.0, 0.0, 1.0, 1.0]),
                )]))
        })
    }),
    ("background_color", |e| {
        with_area(e, |a| a.background_color = Some([10, 20, 31, 255]))
    }),
    ("background_padding", |e| {
        with_area(e, |a| a.background_padding = EdgePadding::new(0.0, 0.0, 1.0, 0.0))
    }),
    ("align_center", |e| with_area(e, |a| a.align_center = true)),
    ("outline", |e| {
        with_area(e, |a| {
            a.outline = Some(OutlineStyle {
                color: [0, 0, 0, 255],
                px: 1.0,
            })
        })
    }),
    ("shape", |e| with_area(e, |a| a.shape = NodeShape::Ellipse)),
    ("zoom_visibility", |e| {
        with_area(e, |a| {
            a.zoom_visibility = ZoomVisibility::from_pair(Some(0.5), None)
        })
    }),
    ("hitbox", |e| {
        with_area(e, |a| a.hitbox.add(BoundingRectangle::at_origin(1.0, 1.0)))
    }),
    ("flags", |e| e.set_flag(Flag::Focused)),
    ("channel", |e| set_channel(e, 2)),
    ("unique_id", |e| e.set_unique_id(8)),
];

/// One variation per state field of a `GlyphModel` element.
const MODEL_VARIATIONS: &[Variation] = &[
    ("glyph_matrix", |e| {
        with_model(e, |m| m.glyph_matrix.push(GlyphLine::new()))
    }),
    ("layer", |e| with_model(e, |m| m.layer = 1)),
    ("position", |e| {
        with_model(e, |m| m.position = OrderedVec2::new_f32(0.0, 1.0))
    }),
    ("hitbox", |e| {
        with_model(e, |m| m.hitbox.add(BoundingRectangle::at_origin(1.0, 1.0)))
    }),
    ("flags", |e| e.set_flag(Flag::Focused)),
    ("channel", |e| set_channel(e, 2)),
    ("unique_id", |e| e.set_unique_id(8)),
];

/// One variation per state field of a `Void` element. `set_flag` is a
/// no-op on a void, so its flags are written directly.
const VOID_VARIATIONS: &[Variation] = &[
    ("flags", |e| {
        if let GfxElement::Void { flags, .. } = e {
            flags.insert(Flag::Focused);
        }
    }),
    ("channel", |e| set_channel(e, 2)),
    ("unique_id", |e| e.set_unique_id(8)),
];

/// Every variation in `variations`, applied to a clone of `base` one at
/// a time, must make an element that is not the same state as `base`,
/// asked either way round. A variation that changed nothing fails this
/// too, so no row can pass without varying its field.
fn assert_each_variation_changes_state(kind: &str, base: &GfxElement, variations: &[Variation]) {
    for (field, vary) in variations {
        let mut varied = base.clone();
        vary(&mut varied);
        assert!(
            !varied.same_state(base) && !base.same_state(&varied),
            "{kind}: a different `{field}` is a change of state"
        );
    }
}

/// `same_state` against every state field, one at a time, for each
/// variant. Each variation of the area fixture (`rich_area`) — every
/// `GlyphArea` field, then the element's flags, channel and id — of a
/// model and of a void is not the same state. A clone, and a copy with
/// one more event subscriber, are the same state: subscribers are
/// callbacks, not state. For a recolor and a hit box `==` says the two
/// are equal, which is why a journal cannot close on it. Two variants
/// never compare the same.
pub fn do_gfx_same_state_compares_every_state_field() {
    let base = rich_area();
    assert!(base.same_state(&base.clone()));
    assert_each_variation_changes_state("area", &base, AREA_VARIATIONS);

    for blind in ["regions (color only)", "hitbox"] {
        let (_, vary) = AREA_VARIATIONS.iter().find(|(field, _)| *field == blind).unwrap();
        let mut varied = base.clone();
        vary(&mut varied);
        assert!(
            varied == base,
            "`==` cannot see a change of `{blind}` — the fixture must keep that so"
        );
    }

    let mut subscribed = base.clone();
    subscribed
        .subscribers_mut()
        .push(Rc::new(RefCell::new(|_: &mut GfxElement, _| {})));
    assert!(subscribed.same_state(&base), "subscribers are not state");

    let model = GfxElement::new_model_non_indexed_with_id(GlyphModel::new(), 1, 7);
    assert!(model.same_state(&model.clone()));
    assert_each_variation_changes_state("model", &model, MODEL_VARIATIONS);

    let void = GfxElement::new_void_with_id(1, 7);
    assert!(void.same_state(&GfxElement::new_void_with_id(1, 7)));
    assert_each_variation_changes_state("void", &void, VOID_VARIATIONS);

    assert!(
        !void.same_state(&model) && !model.same_state(&base),
        "different variants are different states"
    );
}

#[test]
fn test_gfx_clone_keeps_every_field() {
    do_gfx_clone_keeps_every_field();
}

/// A clone carries the flags — a journal's before-image is a clone, so
/// a clone without them would revert every flag away — as well as the
/// payload, channel, id and subscriber handles.
pub fn do_gfx_clone_keeps_every_field() {
    let mut element = rich_area();
    element.set_flag(Flag::Focused);
    element
        .subscribers_mut()
        .push(Rc::new(RefCell::new(|_: &mut GfxElement, _| {})));
    let copy = element.clone();
    assert!(copy.flag_is_set(Flag::Mutable));
    assert!(copy.flag_is_set(Flag::Focused));
    assert_eq!(copy.channel(), 1);
    assert_eq!(copy.unique_id(), 7);
    assert_eq!(copy.subscribers_as_ref().len(), 1);
    assert!(copy.same_state(&element));
}

// ── journaled walks ───────────────────────────────────────────────

/// A glyph tree: a void root with three areas on channels 0, 0 and 1.
fn glyph_row() -> (GfxTree, Vec<NodeId>) {
    let mut tree = GfxTree::new_non_indexed();
    let root = tree.root;
    let ids = [(0, 1), (0, 2), (1, 3)]
        .into_iter()
        .map(|(channel, id)| {
            let mut element = rich_area();
            set_channel(&mut element, channel);
            element.set_unique_id(id);
            tree.insert_child(root, element, Placement::Last).unwrap()
        })
        .collect();
    (tree, ids)
}

/// Every node in pre-order, with its parent and element.
fn snapshot(tree: &GfxTree) -> Vec<(NodeId, Option<NodeId>, GfxElement)> {
    tree.descendants()
        .map(|id| (id, tree.arena()[id].parent(), tree.arena()[id].get().clone()))
        .collect()
}

/// Whether two snapshots have the same nodes in the same places, each
/// in the same state.
fn same_snapshot(
    a: &[(NodeId, Option<NodeId>, GfxElement)],
    b: &[(NodeId, Option<NodeId>, GfxElement)],
) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|((ia, pa, ea), (ib, pb, eb))| ia == ib && pa == pb && ea.same_state(eb))
}

#[test]
fn test_gfx_journal_reverts_and_replays_a_glyph_walk_exactly() {
    do_gfx_journal_reverts_and_replays_a_glyph_walk_exactly();
}

/// A journaled walk over a glyph row moves and rescales the channel-0
/// areas and recolors the channel-1 area's region (a change `==` does
/// not see); a journaled update then moves the first area to channel 2
/// and flags it. The journal names all three areas — the recolor
/// included — `revert` restores the snapshot taken before (states,
/// flags and row order), and `replay` the one taken after.
pub fn do_gfx_journal_reverts_and_replays_a_glyph_walk_exactly() {
    let (mut tree, ids) = glyph_row();
    let root = tree.root;
    let before = snapshot(&tree);

    let mut mutators = GfxMutatorTree::new();
    let mroot = mutators.root;
    let reshape = GfxMutation::area_delta(DeltaGlyphArea::new(vec![
        GlyphAreaField::Position(OrderedVec2::new_f32(50.0, 60.0)),
        GlyphAreaField::Scale(OrderedFloat(20.0)),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]));
    mutators
        .insert_child(mroot, GfxMutator::new(reshape, 0), Placement::Last)
        .unwrap();
    let recolor = GfxMutation::area_delta(DeltaGlyphArea::new(vec![
        GlyphAreaField::ColorFontRegions(ColorFontRegions::new_from(vec![ColorFontRegion::new(
            Range::new(0, 2),
            None,
            Some([0.0, 1.0, 0.0, 1.0]),
        )])),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]));
    mutators
        .insert_child(mroot, GfxMutator::new(recolor, 1), Placement::Last)
        .unwrap();

    let mut builder = JournalBuilder::new();
    mutators
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![root]), |_, _| (), &mut builder)
        .unwrap();
    assert!(
        tree.arena()[ids[2]].get() == &before[3].2,
        "the recolor is invisible to `==`, so only `same_state` can journal it"
    );
    tree.update_journaled(ids[0], &mut builder, |element| {
        set_channel(element, 2);
        element.set_flag(Flag::Focused);
    })
    .unwrap();
    let order: Vec<NodeId> = root.children(tree.arena()).collect();
    assert_eq!(
        order,
        vec![ids[1], ids[2], ids[0]],
        "the update moved the first area last"
    );
    let after = snapshot(&tree);
    let journal = builder.close(&tree);
    let touched: Vec<NodeId> = journal.touched().map(|(id, _)| id).collect();
    assert_eq!(touched, vec![ids[0], ids[1], ids[2]]);

    journal.revert(&mut tree).unwrap();
    assert!(
        same_snapshot(&snapshot(&tree), &before),
        "revert restores the glyph row"
    );
    assert!(!tree.arena()[ids[0]].get().flag_is_set(Flag::Focused));
    assert!(
        tree.arena()[ids[0]].get().flag_is_set(Flag::Mutable),
        "the flag the area had is back"
    );
    tree.check_sorted().unwrap();
    journal.replay(&mut tree).unwrap();
    assert!(
        same_snapshot(&snapshot(&tree), &after),
        "replay reproduces the walk and the update"
    );
}

// ── interpolation ─────────────────────────────────────────────────

#[test]
fn test_gfx_interpolates_geometry_and_color_and_snaps_the_rest() {
    do_gfx_interpolates_geometry_and_color_and_snaps_the_rest();
}

/// One journaled update takes an area from `(0, 0)`, 100 × 40, scale
/// 10, line height 12, background `[10, 20, 30, 255]`, text `"glyph"`
/// to `(10, 20)`, 200 × 80, scale 20, line height 24, background
/// `[110, 120, 130, 55]`, text `"other"`. Halfway, the five
/// interpolating fields are their midpoints under either snap, and the
/// text is `"glyph"` under `Snap::End` and `"other"` under `Snap::Start`.
/// At `0` the tree equals the reverted tree and at `1` the replayed
/// one. A background on one side only snaps with the text. A model's
/// position interpolates the same way.
pub fn do_gfx_interpolates_geometry_and_color_and_snaps_the_rest() {
    let (mut tree, ids) = glyph_row();
    let target = ids[0];
    let model = tree
        .insert_child(
            target,
            GfxElement::new_model_non_indexed_with_id(GlyphModel::new(), 0, 9),
            Placement::Last,
        )
        .unwrap();
    let before = snapshot(&tree);
    let mut builder = JournalBuilder::new();
    tree.update_journaled(target, &mut builder, |element| {
        with_area(element, |area| {
            area.position = OrderedVec2::new_f32(10.0, 20.0);
            area.render_bounds = OrderedVec2::new_f32(200.0, 80.0);
            area.scale = OrderedFloat(20.0);
            area.line_height = OrderedFloat(24.0);
            area.background_color = Some([110, 120, 130, 55]);
            area.text = "other".to_string();
        });
    })
    .unwrap();
    tree.update_journaled(model, &mut builder, |element| {
        if let Some(model) = element.glyph_model_mut() {
            model.position = OrderedVec2::new_f32(-8.0, 4.0);
        }
    })
    .unwrap();
    let after = snapshot(&tree);
    let journal = builder.close(&tree);

    for (snap, text) in [(Snap::End, "glyph"), (Snap::Start, "other")] {
        journal.write_interpolated(&mut tree, 0.5, snap).unwrap();
        let area = tree.arena()[target].get().glyph_area().unwrap();
        assert_eq!(area.position, OrderedVec2::new_f32(5.0, 10.0), "{snap:?}");
        assert_eq!(area.render_bounds, OrderedVec2::new_f32(150.0, 60.0), "{snap:?}");
        assert_eq!(area.scale, OrderedFloat(15.0), "{snap:?}");
        assert_eq!(area.line_height, OrderedFloat(18.0), "{snap:?}");
        assert_eq!(area.background_color, Some([60, 70, 80, 155]), "{snap:?}");
        assert_eq!(area.text, text, "{snap:?}: the text snaps");
        let model_position = tree.arena()[model].get().glyph_model().unwrap().position;
        assert_eq!(model_position, OrderedVec2::new_f32(-4.0, 2.0), "{snap:?}");

        journal.write_interpolated(&mut tree, 0.0, snap).unwrap();
        assert!(
            same_snapshot(&snapshot(&tree), &before),
            "{snap:?}: t = 0 is the reverted tree"
        );
        journal.write_interpolated(&mut tree, 1.0, snap).unwrap();
        assert!(
            same_snapshot(&snapshot(&tree), &after),
            "{snap:?}: t = 1 is the replayed tree"
        );
    }

    // A background present on one side only has nothing to blend: it
    // snaps.
    let (mut tree, ids) = glyph_row();
    let mut builder = JournalBuilder::new();
    tree.update_journaled(ids[0], &mut builder, |element| {
        with_area(element, |area| area.background_color = None);
    })
    .unwrap();
    let journal = builder.close(&tree);
    journal.write_interpolated(&mut tree, 0.5, Snap::End).unwrap();
    let area = tree.arena()[ids[0]].get().glyph_area().unwrap();
    assert_eq!(area.background_color, Some([10, 20, 30, 255]));
    journal.write_interpolated(&mut tree, 0.5, Snap::Start).unwrap();
    let area = tree.arena()[ids[0]].get().glyph_area().unwrap();
    assert_eq!(area.background_color, None);
}

// ── structural commands ───────────────────────────────────────────

#[test]
fn test_gfx_structure_inserts_removes_and_moves_but_never_splits_or_merges() {
    do_gfx_structure_inserts_removes_and_moves_but_never_splits_or_merges();
}

/// The glyph instantiation takes the generic structural commands and
/// refuses the two it does not define. A split and a merge — the
/// default `split` and `merge`, which answer `None` — are
/// `Unsplittable` and `Unmergeable` and change nothing; the merge's
/// target has a childless next sibling of its key, so only the
/// element's own refusal is left to refuse it. Then, on the same row,
/// one journaled builder inserts an area, removes another and moves
/// the third under the inserted one; `revert` restores the snapshot
/// from before and `replay` the one from after, with the same ids.
pub fn do_gfx_structure_inserts_removes_and_moves_but_never_splits_or_merges() {
    let (mut tree, ids) = glyph_row();
    let root = tree.root;
    let command = |op| GfxMutatorTree::new_with(GfxMutator::new_structure(op, 0));
    let before = snapshot(&tree);
    let result = command(StructOp::Split { at: () }).apply_at_journaled(
        &mut tree,
        &Anchors::Nodes(vec![ids[0]]),
        |_, _| (),
        &mut JournalBuilder::new(),
    );
    assert_eq!(result, Err(TreeError::Unsplittable(ids[0])));
    assert_eq!(
        tree.arena()[ids[0]].get().key(),
        tree.arena()[ids[1]].get().key(),
        "the merge target's next sibling has its key"
    );
    assert!(tree.arena()[ids[1]].first_child().is_none(), "and no children");
    let result = command(StructOp::Merge).apply_at_journaled(
        &mut tree,
        &Anchors::Nodes(vec![ids[0]]),
        |_, _| (),
        &mut JournalBuilder::new(),
    );
    assert_eq!(result, Err(TreeError::Unmergeable(ids[0])));
    assert!(
        same_snapshot(&snapshot(&tree), &before),
        "the refusals change nothing"
    );

    let mut inserted = rich_area();
    set_channel(&mut inserted, 0);
    inserted.set_unique_id(9);
    let mut builder = JournalBuilder::new();
    command(StructOp::Insert {
        subtree: Subtree::leaf(inserted),
        at: Placement::Last,
    })
    .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![root]), |_, _| (), &mut builder)
    .unwrap();
    let new = root.children(tree.arena()).nth(2).unwrap();
    command(StructOp::Remove)
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![ids[1]]), |_, _| (), &mut builder)
        .unwrap();
    command(StructOp::Move {
        to: new,
        at: Placement::Last,
    })
    .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![ids[2]]), |_, _| (), &mut builder)
    .unwrap();
    assert_eq!(root.children(tree.arena()).collect::<Vec<_>>(), vec![ids[0], new]);
    assert_eq!(new.children(tree.arena()).collect::<Vec<_>>(), vec![ids[2]]);
    let after = snapshot(&tree);
    let journal = builder.close(&tree);
    journal.revert(&mut tree).unwrap();
    assert!(
        same_snapshot(&snapshot(&tree), &before),
        "revert restores the glyph row"
    );
    journal.replay(&mut tree).unwrap();
    assert!(same_snapshot(&snapshot(&tree), &after), "replay reproduces it");
}
