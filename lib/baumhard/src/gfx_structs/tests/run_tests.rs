// SPDX-License-Identifier: MPL-2.0

//! The `GlyphRun` element: its construction, its bounds, and the
//! glyph mutations that change it (`RunSet`, `RunOrigin`, `RunOpacity`,
//! `RunOverrides`). Fills are in `fill_tests`; what spans both element
//! types (interpolation, predicates, hit-testing, journals, the wire)
//! is in `glyph_layer_tests`.
//!
//! Runs are built from **synthetic** shapes — advances, positions and
//! spans the test chooses — so expected bounds are literals worked by
//! hand and no test shapes text through the font system it would
//! otherwise be checking against (TEST_CONVENTIONS §T13, shape 6).
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2); the tests that
//! drive a `debug_assert!` are plain `#[test]`s, a panic being the
//! subject (baumhard CONVENTIONS §B8).

use std::ops::Range;
use std::sync::Arc;

use glam::Vec2;

use crate::document::values::{Rgba, ZoomWindow};
use crate::engine::element::TreeElement;
use crate::engine::mutation::Mutation;
use crate::flow::tests::support::synthetic;
use crate::flow::{PlacedGlyph, PlacedRun, ShapedBox, FILLER_SPAN};
use crate::font::{Buffer, Metrics};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutation;
use crate::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunError, RunOverrideOp, RunShape};

/// A path shape: a synthetic run with these `advances` and
/// `line_height`, placed as `placed` says: `(glyph index, top-left)`.
/// The glyph index is into the run's glyphs, so `placed` can reuse one,
/// as `Repeat` does.
pub fn path_shape(advances: &[f32], line_height: f32, placed: &[(u32, Vec2)]) -> RunShape {
    RunShape::Path {
        run: Arc::new(synthetic(advances, line_height)),
        placed: Arc::new(PlacedRun {
            glyphs: placed
                .iter()
                .map(|&(glyph, pos)| PlacedGlyph { glyph, pos })
                .collect(),
            truncated: false,
        }),
    }
}

/// A box shape of `size` with `glyphs` laid-out glyphs, all in span 0.
pub fn box_shape(size: Vec2, glyphs: usize) -> RunShape {
    RunShape::Box(Arc::new(ShapedBox {
        buffer: Arc::new(Buffer::new_empty(Metrics::new(10.0, 12.0))),
        size,
        glyph_spans: (0..glyphs).map(|i| (0, i..i + 1)).collect(),
    }))
}

/// The four-glyph path shape most tests use. Glyph boxes are `advance ×
/// 20` at their top-left:
///
/// ```text
/// g0  advance 10  at (100, 50)   x 100..110  y 50..70
/// g1  advance 12  at (110, 50)   x 110..122  y 50..70
/// g2  advance  0  at (110, 50)   a mark on g1: no width
/// g3  advance  8  at (122, 70)   x 122..130  y 70..90
/// ```
///
/// Their union is `(100, 50)` to `(130, 90)`: top-left `(100, 50)`, size
/// `(30, 40)`.
pub fn four_glyph_shape() -> RunShape {
    path_shape(
        &[10.0, 12.0, 0.0, 8.0],
        20.0,
        &[
            (0, Vec2::new(100.0, 50.0)),
            (1, Vec2::new(110.0, 50.0)),
            (2, Vec2::new(110.0, 50.0)),
            (3, Vec2::new(122.0, 70.0)),
        ],
    )
}

/// A run of `shape` at `origin`, opaque, with no overrides and no zoom
/// window.
pub fn run_of(shape: RunShape, origin: Vec2) -> GlyphRunData {
    GlyphRunData::new(shape, origin, GlyphOverrides::none(), 1.0, None).unwrap()
}

/// A `GlyphRun` element (channel 0, id 5) holding the four-glyph shape
/// at the origin.
pub fn four_glyph_element() -> GfxElement {
    GfxElement::new_run(run_of(four_glyph_shape(), Vec2::ZERO), 0, 5)
}

/// Apply `mutation` to `element` the way the walker does.
pub fn apply(element: &mut GfxElement, mutation: GfxMutation) {
    mutation.apply_to(element, &());
}

/// The names of the fields of `a` and `b` that differ: `shaped`
/// (identity), `origin`, `overrides`, `opacity`, `zoom`, `bounds`.
/// Compared field by field through the public accessors, so a test can
/// say "this mutation changed exactly these".
pub fn changed_run_fields(a: &GlyphRunData, b: &GlyphRunData) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if a.shaped() != b.shaped() {
        changed.push("shaped");
    }
    if a.origin() != b.origin() {
        changed.push("origin");
    }
    if a.overrides() != b.overrides() {
        changed.push("overrides");
    }
    if a.opacity() != b.opacity() {
        changed.push("opacity");
    }
    if a.zoom() != b.zoom() {
        changed.push("zoom");
    }
    if a.bounds() != b.bounds() {
        changed.push("bounds");
    }
    changed
}

const RED: Rgba = Rgba::new(255, 0, 0, 255);

// ── construction ──────────────────────────────────────────────────

#[test]
fn test_run_construction_validates_override_lengths() {
    do_run_construction_validates_override_lengths();
}

/// Overrides are empty or exactly as long as the placed glyphs, and
/// anything else is a construction error. "Placed" is the placement's
/// glyph count: a `Repeat`ed run with two shaped glyphs placed five
/// times takes five, not two; a box takes its laid-out glyph count.
pub fn do_run_construction_validates_override_lengths() {
    let build = |shape: RunShape, len: Option<usize>| {
        let overrides = len.map_or_else(GlyphOverrides::none, GlyphOverrides::new);
        GlyphRunData::new(shape, Vec2::ZERO, overrides, 1.0, None)
    };

    let four = four_glyph_shape();
    assert!(build(four.clone(), None).is_ok(), "no overrides");
    assert!(build(four.clone(), Some(4)).is_ok(), "one per placed glyph");
    assert_eq!(
        build(four.clone(), Some(3)).unwrap_err(),
        RunError::OverrideLength {
            overrides: 3,
            glyphs: 4
        },
        "one short"
    );
    assert_eq!(
        build(four, Some(5)).unwrap_err(),
        RunError::OverrideLength {
            overrides: 5,
            glyphs: 4
        },
        "one over"
    );

    // Two shaped glyphs, five placed: the length follows the placement.
    let repeated = || {
        path_shape(
            &[10.0, 10.0],
            20.0,
            &[
                (0, Vec2::new(0.0, 0.0)),
                (1, Vec2::new(10.0, 0.0)),
                (0, Vec2::new(20.0, 0.0)),
                (1, Vec2::new(30.0, 0.0)),
                (0, Vec2::new(40.0, 0.0)),
            ],
        )
    };
    assert_eq!(repeated().glyph_count(), 5, "the fixture reuses glyph indices");
    assert!(build(repeated(), Some(5)).is_ok(), "five placed glyphs");
    assert!(
        build(repeated(), Some(2)).is_err(),
        "the shaped glyph count is not the length"
    );

    let boxed = || box_shape(Vec2::new(80.0, 30.0), 3);
    assert!(build(boxed(), Some(3)).is_ok(), "a box's laid-out glyphs");
    assert!(build(boxed(), Some(4)).is_err());
}

// ── bounds ────────────────────────────────────────────────────────

#[test]
fn test_run_bounds_are_the_union_of_the_placed_glyph_boxes() {
    do_run_bounds_are_the_union_of_the_placed_glyph_boxes();
}

/// A path run's bounds are the union of its placed glyphs' boxes
/// (`pos .. pos + (advance, line_height)`) shifted by the origin; a
/// combining mark's zero-width box adds nothing; a reused glyph index
/// is boxed once per placement; a placement naming a glyph the run
/// lacks, or at a position that is not finite, adds nothing. A box
/// run's bounds are its shaped size at the origin. A run with nothing
/// to box has no spatial bounds at all.
pub fn do_run_bounds_are_the_union_of_the_placed_glyph_boxes() {
    let at = |origin: Vec2| run_of(four_glyph_shape(), origin);
    assert_eq!(
        at(Vec2::ZERO).bounds(),
        (Vec2::new(100.0, 50.0), Vec2::new(30.0, 40.0))
    );
    assert_eq!(
        at(Vec2::new(5.0, -10.0)).bounds(),
        (Vec2::new(105.0, 40.0), Vec2::new(30.0, 40.0)),
        "the origin offsets every glyph"
    );

    // Two glyphs placed four times, as `Repeat` places them: the last
    // copy ends at x = 40.
    let repeated = run_of(
        path_shape(
            &[10.0, 10.0],
            20.0,
            &[
                (0, Vec2::new(0.0, 0.0)),
                (1, Vec2::new(10.0, 0.0)),
                (0, Vec2::new(20.0, 0.0)),
                (1, Vec2::new(30.0, 0.0)),
            ],
        ),
        Vec2::ZERO,
    );
    assert_eq!(repeated.bounds(), (Vec2::ZERO, Vec2::new(40.0, 20.0)));

    // A glyph index past the run, and a position that is not finite,
    // are skipped; the one real glyph is the bounds.
    let degenerate = run_of(
        path_shape(
            &[10.0],
            20.0,
            &[
                (9, Vec2::new(-500.0, -500.0)),
                (0, Vec2::new(f32::NAN, 0.0)),
                (0, Vec2::new(3.0, 4.0)),
            ],
        ),
        Vec2::ZERO,
    );
    assert_eq!(degenerate.bounds(), (Vec2::new(3.0, 4.0), Vec2::new(10.0, 20.0)));

    let boxed = run_of(box_shape(Vec2::new(80.0, 30.0), 3), Vec2::new(7.0, 9.0));
    assert_eq!(boxed.bounds(), (Vec2::new(7.0, 9.0), Vec2::new(80.0, 30.0)));

    // Nothing placed, and only a mark (a box with no width): no box.
    let empty = GfxElement::new_run(run_of(path_shape(&[10.0], 20.0, &[]), Vec2::ZERO), 0, 1);
    assert_eq!(empty.spatial_bounds(), None);
    let mark = GfxElement::new_run(
        run_of(path_shape(&[0.0], 20.0, &[(0, Vec2::ZERO)]), Vec2::ZERO),
        0,
        1,
    );
    assert_eq!(mark.spatial_bounds(), None, "a zero-width box is not a box");
    let element = GfxElement::new_run(at(Vec2::ZERO), 0, 1);
    assert_eq!(
        element.spatial_bounds(),
        Some((Vec2::new(100.0, 50.0), Vec2::new(30.0, 40.0))),
        "the element answers with the run's bounds"
    );
}

// ── RunOrigin / RunOpacity / RunSet ───────────────────────────────

#[test]
fn test_run_origin_moves_the_bounds_and_nothing_else() {
    do_run_origin_moves_the_bounds_and_nothing_else();
}

/// `RunOrigin` sets the origin and the bounds follow; the shaping,
/// overrides, opacity and zoom are untouched.
pub fn do_run_origin_moves_the_bounds_and_nothing_else() {
    let mut element = GfxElement::new_run(
        GlyphRunData::new(
            four_glyph_shape(),
            Vec2::ZERO,
            GlyphOverrides::new(4),
            0.5,
            Some(ZoomWindow {
                min: Some(1.0),
                max: None,
            }),
        )
        .unwrap(),
        0,
        5,
    );
    let before = element.glyph_run().unwrap().clone();
    apply(&mut element, GfxMutation::RunOrigin(Vec2::new(50.0, 60.0)));
    let after = element.glyph_run().unwrap();
    assert_eq!(after.origin(), Vec2::new(50.0, 60.0));
    assert_eq!(
        after.bounds(),
        (Vec2::new(150.0, 110.0), Vec2::new(30.0, 40.0)),
        "the union of the glyph boxes, moved by the origin"
    );
    assert_eq!(changed_run_fields(&before, after), vec!["origin", "bounds"]);
}

#[test]
fn test_run_opacity_sets_only_the_opacity() {
    do_run_opacity_sets_only_the_opacity();
}

/// `RunOpacity` sets the opacity; the bounds do not move.
pub fn do_run_opacity_sets_only_the_opacity() {
    let mut element = GfxElement::new_run(run_of(four_glyph_shape(), Vec2::new(5.0, 6.0)), 0, 5);
    let before = element.glyph_run().unwrap().clone();
    apply(&mut element, GfxMutation::RunOpacity(0.25));
    let after = element.glyph_run().unwrap();
    assert_eq!(after.opacity(), 0.25);
    assert_eq!(changed_run_fields(&before, after), vec!["opacity"]);
}

#[test]
fn test_run_set_replaces_the_run_and_recomputes_its_bounds() {
    do_run_set_replaces_the_run_and_recomputes_its_bounds();
}

/// `RunSet` replaces the whole payload: a path run becomes a box run,
/// with its own overrides, opacity and zoom window, and the bounds are
/// the new run's, not the old run's moved.
pub fn do_run_set_replaces_the_run_and_recomputes_its_bounds() {
    let mut element = four_glyph_element();
    let replacement = GlyphRunData::new(
        box_shape(Vec2::new(80.0, 30.0), 2),
        Vec2::new(7.0, 9.0),
        GlyphOverrides::new(2),
        0.5,
        Some(ZoomWindow {
            min: None,
            max: Some(3.0),
        }),
    )
    .unwrap();
    apply(&mut element, GfxMutation::RunSet(Box::new(replacement.clone())));
    let after = element.glyph_run().unwrap();
    assert!(after == &replacement, "the whole payload is the replacement");
    assert_eq!(after.bounds(), (Vec2::new(7.0, 9.0), Vec2::new(80.0, 30.0)));
    assert_eq!(after.overrides().len(), 2);
    assert_eq!(after.opacity(), 0.5);
    assert_eq!(
        element.channel(),
        0,
        "the element's own metadata is not the payload"
    );
    assert_eq!(element.unique_id(), 5);
}

// ── overrides ─────────────────────────────────────────────────────

/// The run of `shape`, with `ops` applied through the mutation.
fn with_ops(shape: RunShape, ops: Vec<RunOverrideOp>) -> GfxElement {
    with_ops_on(&GfxElement::new_run(run_of(shape, Vec2::ZERO), 0, 5), ops)
}

/// A clone of `base` with `ops` applied through the mutation. The clone
/// shares `base`'s shaping, so the two compare by state.
fn with_ops_on(base: &GfxElement, ops: Vec<RunOverrideOp>) -> GfxElement {
    let mut element = base.clone();
    for op in ops {
        apply(&mut element, GfxMutation::RunOverrides(op));
    }
    element
}

#[test]
fn test_run_override_ops_write_their_range_and_materialize_the_rest_neutral() {
    do_run_override_ops_write_their_range_and_materialize_the_rest_neutral();
}

/// Each op writes only its own vector over only its range. The first
/// write to a run without overrides materializes `GlyphOverrides::new`
/// for all of it — so the other two vectors are there, neutral — and a
/// later op on another vector keeps what the first wrote. `Clear` drops
/// them all.
pub fn do_run_override_ops_write_their_range_and_materialize_the_rest_neutral() {
    let offset = Vec2::new(4.0, 5.0);
    let neutral_offsets = [Vec2::ZERO; 4];
    let neutral_colors: [Option<Rgba>; 4] = [None; 4];
    let neutral_scales = [1.0_f32; 4];

    let base = four_glyph_element();
    let element = with_ops_on(
        &base,
        vec![RunOverrideOp::Offset {
            range: 1..3,
            value: offset,
        }],
    );
    let run = element.glyph_run().unwrap();
    assert_eq!(run.overrides().len(), 4, "materialized to the placed glyph count");
    assert_eq!(
        run.overrides().offset(),
        &[Vec2::ZERO, offset, offset, Vec2::ZERO]
    );
    assert_eq!(
        run.overrides().color(),
        &neutral_colors,
        "colors materialized neutral"
    );
    assert_eq!(
        run.overrides().scale(),
        &neutral_scales,
        "scales materialized neutral"
    );
    assert_eq!(
        changed_run_fields(base.glyph_run().unwrap(), run),
        vec!["overrides"],
        "an override op changes the overrides and nothing else, the bounds included"
    );

    let element = with_ops(
        four_glyph_shape(),
        vec![RunOverrideOp::Color {
            range: 0..2,
            value: Some(RED),
        }],
    );
    let run = element.glyph_run().unwrap();
    assert_eq!(run.overrides().color(), &[Some(RED), Some(RED), None, None]);
    assert_eq!(run.overrides().offset(), &neutral_offsets);
    assert_eq!(run.overrides().scale(), &neutral_scales);

    let element = with_ops(
        four_glyph_shape(),
        vec![RunOverrideOp::Scale {
            range: 3..4,
            value: 2.5,
        }],
    );
    let run = element.glyph_run().unwrap();
    assert_eq!(run.overrides().scale(), &[1.0, 1.0, 1.0, 2.5]);
    assert_eq!(run.overrides().offset(), &neutral_offsets);
    assert_eq!(run.overrides().color(), &neutral_colors);

    // Ops compose: a color over glyphs 0..2 after an offset over 1..3
    // leaves the offsets where they were.
    let element = with_ops(
        four_glyph_shape(),
        vec![
            RunOverrideOp::Offset {
                range: 1..3,
                value: offset,
            },
            RunOverrideOp::Color {
                range: 0..2,
                value: Some(RED),
            },
        ],
    );
    let run = element.glyph_run().unwrap();
    assert_eq!(
        run.overrides().offset(),
        &[Vec2::ZERO, offset, offset, Vec2::ZERO]
    );
    assert_eq!(run.overrides().color(), &[Some(RED), Some(RED), None, None]);

    // `value: None` returns a colored glyph to its own color.
    let element = with_ops(
        four_glyph_shape(),
        vec![
            RunOverrideOp::Color {
                range: 0..4,
                value: Some(RED),
            },
            RunOverrideOp::Color {
                range: 1..3,
                value: None,
            },
        ],
    );
    assert_eq!(
        element.glyph_run().unwrap().overrides().color(),
        &[Some(RED), None, None, Some(RED)]
    );

    // Clear drops every override.
    let element = with_ops(
        four_glyph_shape(),
        vec![
            RunOverrideOp::Scale {
                range: 0..4,
                value: 3.0,
            },
            RunOverrideOp::Clear,
        ],
    );
    let run = element.glyph_run().unwrap();
    assert!(run.overrides().is_empty());
    assert_eq!(run.overrides(), &GlyphOverrides::none());
}

#[test]
fn test_run_override_ranges_are_clamped_to_the_placed_glyphs() {
    do_run_override_ranges_are_clamped_to_the_placed_glyphs();
}

/// A range is clamped to the placed glyph count: one that runs past the
/// end writes up to it; one that starts past the end, runs backward or
/// is empty writes nothing — and does not even materialize the
/// overrides. The count is the **placed** glyphs: on a run that reuses
/// glyph indices, a range reaches past the shaped glyph count.
pub fn do_run_override_ranges_are_clamped_to_the_placed_glyphs() {
    let element = with_ops(
        four_glyph_shape(),
        vec![RunOverrideOp::Scale {
            range: 2..usize::MAX,
            value: 9.0,
        }],
    );
    assert_eq!(
        element.glyph_run().unwrap().overrides().scale(),
        &[1.0, 1.0, 9.0, 9.0],
        "a range past the end writes up to the end"
    );

    let none = four_glyph_element();
    for (name, range) in [
        ("past the end", 4..9),
        ("far past the end", 100..200),
        ("backward", Range { start: 3, end: 1 }),
        ("empty", 2..2),
    ] {
        let element = with_ops_on(
            &none,
            vec![RunOverrideOp::Offset {
                range,
                value: Vec2::new(1.0, 1.0),
            }],
        );
        assert!(
            element.same_state(&none),
            "a range {name} writes nothing and leaves the run exactly as it was"
        );
    }

    // Five placements of two shaped glyphs.
    let repeated = path_shape(
        &[10.0, 10.0],
        20.0,
        &[
            (0, Vec2::new(0.0, 0.0)),
            (1, Vec2::new(10.0, 0.0)),
            (0, Vec2::new(20.0, 0.0)),
            (1, Vec2::new(30.0, 0.0)),
            (0, Vec2::new(40.0, 0.0)),
        ],
    );
    let element = with_ops(
        repeated,
        vec![RunOverrideOp::Scale {
            range: 3..99,
            value: 2.0,
        }],
    );
    assert_eq!(
        element.glyph_run().unwrap().overrides().scale(),
        &[1.0, 1.0, 1.0, 2.0, 2.0],
        "the range indexes placed glyphs, which outnumber the shaped ones"
    );
}

#[test]
fn test_run_override_ops_skip_filler_glyphs() {
    do_run_override_ops_skip_filler_glyphs();
}

/// A glyph whose span is `FILLER_SPAN` is padding the placement added
/// (a `Once` frame's filler). A range op passes over it: the text
/// around it is written and it keeps its neutral override. `Clear` and
/// the unaddressed glyphs are unaffected.
pub fn do_run_override_ops_skip_filler_glyphs() {
    let mut shaped = synthetic(&[10.0, 10.0, 10.0, 10.0], 20.0);
    shaped.glyphs[2].span = FILLER_SPAN;
    shaped.glyphs[3].span = FILLER_SPAN;
    let placed = (0..4)
        .map(|i| PlacedGlyph {
            glyph: i,
            pos: Vec2::new(10.0 * i as f32, 0.0),
        })
        .collect();
    let shape = RunShape::Path {
        run: Arc::new(shaped),
        placed: Arc::new(PlacedRun {
            glyphs: placed,
            truncated: false,
        }),
    };
    let target = Vec2::new(3.0, 3.0);
    let element = with_ops(
        shape.clone(),
        vec![
            RunOverrideOp::Offset {
                range: 0..4,
                value: target,
            },
            RunOverrideOp::Color {
                range: 1..4,
                value: Some(RED),
            },
            RunOverrideOp::Scale {
                range: 0..4,
                value: 2.0,
            },
        ],
    );
    let overrides = element.glyph_run().unwrap().overrides();
    assert_eq!(overrides.offset(), &[target, target, Vec2::ZERO, Vec2::ZERO]);
    assert_eq!(overrides.color(), &[None, Some(RED), None, None]);
    assert_eq!(overrides.scale(), &[2.0, 2.0, 1.0, 1.0]);

    // Only filler in range: nothing to write.
    let element = with_ops(
        shape,
        vec![RunOverrideOp::Offset {
            range: 2..4,
            value: target,
        }],
    );
    assert_eq!(
        element.glyph_run().unwrap().overrides().offset(),
        &[Vec2::ZERO; 4],
        "filler keeps its neutral offset"
    );
}

// ── element metadata ──────────────────────────────────────────────

#[test]
fn test_run_element_answers_position_and_flags_like_any_element() {
    do_run_element_answers_position_and_flags_like_any_element();
}

/// A run's element position is the top-left of its bounds, and setting
/// it moves the run so that is where its bounds start (the origin shifts
/// by the difference); flags, channel and id are the element's own, as
/// for an area.
pub fn do_run_element_answers_position_and_flags_like_any_element() {
    use crate::core::primitives::{Flag, Flaggable};
    let mut element = GfxElement::new_run(run_of(four_glyph_shape(), Vec2::new(5.0, 5.0)), 3, 11);
    assert_eq!(element.position(), Vec2::new(105.0, 55.0));
    element.set_position(Vec2::new(0.0, 0.0));
    assert_eq!(element.position(), Vec2::ZERO);
    assert_eq!(
        element.glyph_run().unwrap().origin(),
        Vec2::new(-100.0, -50.0),
        "the origin is the shift that puts the bounds at the position"
    );
    assert_eq!(element.channel(), 3);
    assert_eq!(element.unique_id(), 11);
    assert!(!element.flag_is_set(Flag::Focused));
    element.set_flag(Flag::Focused);
    assert!(element.flag_is_set(Flag::Focused));
    element.clear_flag(Flag::Focused);
    assert!(!element.flag_is_set(Flag::Focused));
    assert_eq!(format!("{element:?}"), "GlyphRun(id=11, ch=3, glyphs=4)");
    assert!(element.glyph_area().is_none() && element.glyph_model().is_none() && element.fill().is_none());
}

// ── mismatches ────────────────────────────────────────────────────

#[test]
fn test_run_mutations_leave_other_elements_alone_and_other_mutations_leave_runs_alone() {
    do_run_mutations_leave_other_elements_alone_and_other_mutations_leave_runs_alone();
}

/// Area and model mutations that reach a run or a fill change nothing,
/// as they change nothing at a `Void`; the element is exactly as it was.
pub fn do_run_mutations_leave_other_elements_alone_and_other_mutations_leave_runs_alone() {
    use crate::gfx_structs::area::GlyphAreaCommand;
    use crate::gfx_structs::model::GlyphModelCommand;
    let run = four_glyph_element();
    let mut target = run.clone();
    apply(
        &mut target,
        GfxMutation::area_command(GlyphAreaCommand::NudgeRight(10.0)),
    );
    apply(
        &mut target,
        GfxMutation::model_command(GlyphModelCommand::NudgeDown(1.0)),
    );
    assert!(
        target.same_state(&run),
        "area and model mutations leave a run alone"
    );
}

#[cfg(all(test, debug_assertions))]
mod refusals {
    use super::*;
    use crate::flow::Rect;
    use crate::gfx_structs::area::GlyphArea;
    use crate::gfx_structs::model::GlyphModel;

    fn an_area() -> GfxElement {
        GfxElement::new_area_non_indexed(GlyphArea::new(1.0, 10.0, Vec2::ZERO, Vec2::new(10.0, 10.0)), 0)
    }

    #[test]
    #[should_panic(expected = "reached a GlyphArea")]
    fn test_a_run_mutation_at_an_area_trips_the_debug_assert() {
        apply(&mut an_area(), GfxMutation::RunOpacity(0.5));
    }

    #[test]
    #[should_panic(expected = "reached a GlyphModel")]
    fn test_a_run_mutation_at_a_model_trips_the_debug_assert() {
        let mut model = GfxElement::new_model_non_indexed(GlyphModel::new(), 0, 1);
        apply(&mut model, GfxMutation::RunOrigin(Vec2::ONE));
    }

    #[test]
    #[should_panic(expected = "reached a Void")]
    fn test_a_run_mutation_at_a_void_trips_the_debug_assert() {
        apply(
            &mut GfxElement::new_void(0),
            GfxMutation::RunOverrides(RunOverrideOp::Clear),
        );
    }

    #[test]
    #[should_panic(expected = "reached a Fill")]
    fn test_a_run_mutation_at_a_fill_trips_the_debug_assert() {
        let fill = crate::gfx_structs::fill::FillData {
            rect: Rect::new(Vec2::ZERO, Vec2::new(10.0, 10.0)),
            shape: crate::gfx_structs::fill::FillShape::Ellipse,
            color: RED,
            fade: 0.0,
            opacity: 1.0,
            zoom: None,
        };
        apply(
            &mut GfxElement::new_fill(fill, 0, 1),
            GfxMutation::RunOpacity(0.5),
        );
    }

    #[test]
    #[should_panic(expected = "reached a GlyphRun")]
    fn test_a_fill_mutation_at_a_run_trips_the_debug_assert() {
        apply(&mut four_glyph_element(), GfxMutation::FillOpacity(0.5));
    }

    #[test]
    #[should_panic(expected = "reached a GlyphArea")]
    fn test_a_fill_mutation_at_an_area_trips_the_debug_assert() {
        apply(&mut an_area(), GfxMutation::FillColor(RED));
    }
}
