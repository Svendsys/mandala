// SPDX-License-Identifier: MPL-2.0

//! What `GlyphRun` and `Fill` do together, and as elements among the
//! others: `same_state`, interpolation, predicate fields, the BVH hit
//! test, journals, the wire, events, and a chain 100,000 levels deep.
//! The elements alone are in `run_tests` and `fill_tests`.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2).

use std::cell::RefCell;
use std::rc::Rc;

use glam::Vec2;
use indextree::NodeId;

use crate::core::primitives::{Flag, Flaggable};
use crate::document::values::{Rgba, ZoomWindow};
use crate::engine::anchor::Anchors;
use crate::engine::element::TreeElement;
use crate::engine::interpolate::{Interpolable, Snap};
use crate::engine::journal::JournalBuilder;
use crate::engine::predicate::{Comparator, Predicate};
use crate::engine::tree::Placement;
use crate::flow::Rect;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::{GfxElement, GfxElementField, GfxElementType};
use crate::gfx_structs::fill::{FillData, FillShape};
use crate::gfx_structs::model::GlyphModel;
use crate::gfx_structs::mutator::{GfxMutation, GfxMutator, GlyphTreeEvent, GlyphTreeEventInstance};
use crate::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunOverrideOp};
use crate::gfx_structs::tests::fill_tests::{a_fill, a_fill_element};
use crate::gfx_structs::tests::run_tests::{apply, box_shape, four_glyph_shape, run_of};
use crate::gfx_structs::tree::{GfxMutatorTree, GfxTree};
use crate::util::doc_fixtures::{documented_json_block, format_doc_path};

// ── same_state ────────────────────────────────────────────────────

/// A change to one field, applied to an element.
type Variation = (&'static str, fn(&mut GfxElement));

/// A run's payload with a value in every field, around `shape`.
fn base_run_data(shape: crate::gfx_structs::run::RunShape) -> GlyphRunData {
    GlyphRunData::new(
        shape,
        Vec2::new(1.0, 2.0),
        GlyphOverrides::new(4),
        0.5,
        Some(ZoomWindow {
            min: Some(1.0),
            max: Some(4.0),
        }),
    )
    .unwrap()
}

/// The run fixture the variations vary: channel 1, id 7, flagged.
fn base_run() -> GfxElement {
    let mut element = GfxElement::new_run(base_run_data(four_glyph_shape()), 1, 7);
    element.set_flag(Flag::Mutable);
    element
}

/// The same fixture with a different `shaped` value built beside it.
/// `base_run` cannot be cloned for this: a clone shares the shaping.
fn base_run_with_shape(shape: crate::gfx_structs::run::RunShape) -> GfxElement {
    let mut element = GfxElement::new_run(base_run_data(shape), 1, 7);
    element.set_flag(Flag::Mutable);
    element
}

/// The element's channel, rewritten: no mutation reaches it.
fn set_channel(element: &mut GfxElement, to: usize) {
    match element {
        GfxElement::GlyphRun { channel, .. } | GfxElement::Fill { channel, .. } => *channel = to,
        _ => unreachable!("the variations are for runs and fills"),
    }
}

/// One variation per state field of a run element.
const RUN_VARIATIONS: &[Variation] = &[
    ("shaped (a second shaping, glyph for glyph alike)", |e| {
        // Everything the same except the `Arc`s: the identity of the
        // shaping is state.
        apply(
            e,
            GfxMutation::RunSet(Box::new(base_run_data(four_glyph_shape()))),
        );
    }),
    ("origin", |e| {
        apply(e, GfxMutation::RunOrigin(Vec2::new(1.0, 3.0)))
    }),
    ("opacity", |e| apply(e, GfxMutation::RunOpacity(0.6))),
    ("overrides (offset)", |e| {
        apply(
            e,
            GfxMutation::RunOverrides(RunOverrideOp::Offset {
                range: 3..4,
                value: Vec2::new(0.0, 1.0),
            }),
        )
    }),
    ("overrides (color)", |e| {
        apply(
            e,
            GfxMutation::RunOverrides(RunOverrideOp::Color {
                range: 0..1,
                value: Some(Rgba::new(1, 2, 3, 4)),
            }),
        )
    }),
    ("overrides (scale)", |e| {
        apply(
            e,
            GfxMutation::RunOverrides(RunOverrideOp::Scale {
                range: 2..3,
                value: 1.5,
            }),
        )
    }),
    ("overrides (cleared)", |e| {
        apply(e, GfxMutation::RunOverrides(RunOverrideOp::Clear))
    }),
    ("flags", |e| e.set_flag(Flag::Focused)),
    ("channel", |e| set_channel(e, 2)),
    ("unique_id", |e| e.set_unique_id(8)),
];

/// One variation per state field of a fill element.
const FILL_VARIATIONS: &[Variation] = &[
    ("rect origin", |e| {
        apply(
            e,
            GfxMutation::FillRect(Rect::new(Vec2::new(10.0, 21.0), Vec2::new(100.0, 50.0))),
        )
    }),
    ("rect size", |e| {
        apply(
            e,
            GfxMutation::FillRect(Rect::new(Vec2::new(10.0, 20.0), Vec2::new(100.0, 51.0))),
        )
    }),
    ("shape (radius)", |e| {
        let mut fill = a_fill();
        fill.shape = FillShape::Rect { radius: 9.0 };
        apply(e, GfxMutation::FillSet(Box::new(fill)))
    }),
    ("shape (kind)", |e| {
        let mut fill = a_fill();
        fill.shape = FillShape::Ellipse;
        apply(e, GfxMutation::FillSet(Box::new(fill)))
    }),
    ("color", |e| {
        apply(e, GfxMutation::FillColor(Rgba::new(255, 255, 255, 203)))
    }),
    ("fade", |e| {
        let mut fill = a_fill();
        fill.fade = 7.0;
        apply(e, GfxMutation::FillSet(Box::new(fill)))
    }),
    ("opacity", |e| apply(e, GfxMutation::FillOpacity(0.7))),
    ("zoom (min)", |e| {
        let mut fill = a_fill();
        fill.zoom = Some(ZoomWindow {
            min: Some(0.6),
            max: None,
        });
        apply(e, GfxMutation::FillSet(Box::new(fill)))
    }),
    ("zoom (max)", |e| {
        let mut fill = a_fill();
        fill.zoom = Some(ZoomWindow {
            min: Some(0.5),
            max: Some(9.0),
        });
        apply(e, GfxMutation::FillSet(Box::new(fill)))
    }),
    ("zoom (removed)", |e| {
        let mut fill = a_fill();
        fill.zoom = None;
        apply(e, GfxMutation::FillSet(Box::new(fill)))
    }),
    ("flags", |e| e.set_flag(Flag::Focused)),
    ("channel", |e| set_channel(e, 2)),
    ("unique_id", |e| e.set_unique_id(8)),
];

/// Each variation changes the base's state, and the base compares the
/// same as its own clone.
fn assert_each_variation_changes_state(kind: &str, base: &GfxElement, variations: &[Variation]) {
    for (field, vary) in variations {
        let mut varied = base.clone();
        vary(&mut varied);
        assert!(
            !base.same_state(&varied),
            "{kind}: a different `{field}` is a change of state"
        );
        assert!(!varied.same_state(base), "{kind}: `{field}`, the other way round");
    }
}

#[test]
fn test_glyph_layer_same_state_compares_every_state_field() {
    do_glyph_layer_same_state_compares_every_state_field();
}

/// `same_state` against every state field of a run and of a fill, one
/// at a time. A clone is the same state, since it shares the shaping;
/// a second shaping that comes out glyph for glyph alike is not
/// (`RunShape` is compared by `Arc` identity). A `NaN` payload equals
/// itself, so an element holding one is still the same state as its own
/// clone. Subscribers are not state. The new variants never compare the
/// same as another variant or as each other.
pub fn do_glyph_layer_same_state_compares_every_state_field() {
    let run = base_run();
    assert!(run.same_state(&run.clone()));
    assert_each_variation_changes_state("run", &run, RUN_VARIATIONS);

    // The `==` of the element agrees about identity too.
    let other_shaping = base_run_with_shape(four_glyph_shape());
    assert!(run != other_shaping, "a second shaping is not equal");
    assert!(run == run.clone());

    let fill = a_fill_element();
    assert!(fill.same_state(&fill.clone()));
    assert_each_variation_changes_state("fill", &fill, FILL_VARIATIONS);

    let mut subscribed = run.clone();
    subscribed
        .subscribers_mut()
        .push(Rc::new(RefCell::new(|_: &mut GfxElement, _| {})));
    assert!(subscribed.same_state(&run), "subscribers are not state");

    let mut nan = run.clone();
    apply(&mut nan, GfxMutation::RunOpacity(f32::NAN));
    assert!(nan.same_state(&nan.clone()), "NaN is the same state as NaN");
    assert!(!nan.same_state(&run));
    let mut nan_fill = fill.clone();
    apply(&mut nan_fill, GfxMutation::FillOpacity(f32::NAN));
    assert!(nan_fill.same_state(&nan_fill.clone()));

    let area =
        GfxElement::new_area_non_indexed_with_id(GlyphArea::new(1.0, 10.0, Vec2::ZERO, Vec2::ONE), 1, 7);
    let model = GfxElement::new_model_non_indexed_with_id(GlyphModel::new(), 1, 7);
    let void = GfxElement::new_void_with_id(1, 7);
    for (name, other) in [("area", &area), ("model", &model), ("void", &void)] {
        assert!(!run.same_state(other) && !other.same_state(&run), "run vs {name}");
        assert!(
            !fill.same_state(other) && !other.same_state(&fill),
            "fill vs {name}"
        );
    }
    assert!(!run.same_state(&fill) && !fill.same_state(&run));
}

// ── events ────────────────────────────────────────────────────────

#[test]
fn test_glyph_layer_elements_deliver_events_to_their_subscribers() {
    do_glyph_layer_elements_deliver_events_to_their_subscribers();
}

/// An `Event` mutation reaches the subscribers of a run and of a fill,
/// as it does for an area, and changes no state.
pub fn do_glyph_layer_elements_deliver_events_to_their_subscribers() {
    for mut element in [base_run(), a_fill_element()] {
        let seen = Rc::new(RefCell::new(0_usize));
        let sink = Rc::clone(&seen);
        element.subscribers_mut().push(Rc::new(RefCell::new(
            move |_: &mut GfxElement, event: GlyphTreeEventInstance| {
                if matches!(event.event_type, GlyphTreeEvent::NoopEvent(9)) {
                    *sink.borrow_mut() += 1;
                }
            },
        )));
        let before = element.clone();
        apply(
            &mut element,
            GfxMutation::Event(GlyphTreeEventInstance::new(GlyphTreeEvent::NoopEvent(9), 0)),
        );
        assert_eq!(*seen.borrow(), 1, "{element:?}: the subscriber ran once");
        assert!(element.same_state(&before), "an event changes no state");
    }
}

// ── interpolation ─────────────────────────────────────────────────

/// A run on the four-glyph shape with `ops` applied, opacity `opacity`
/// and origin `origin`.
fn run_with(
    shape: crate::gfx_structs::run::RunShape,
    origin: Vec2,
    opacity: f32,
    ops: Vec<RunOverrideOp>,
) -> GfxElement {
    let mut element = GfxElement::new_run(run_of(shape, Vec2::ZERO), 0, 5);
    apply(&mut element, GfxMutation::RunOrigin(origin));
    apply(&mut element, GfxMutation::RunOpacity(opacity));
    for op in ops {
        apply(&mut element, GfxMutation::RunOverrides(op));
    }
    element
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn test_run_interpolates_origin_opacity_and_overrides_and_snaps_the_shape() {
    do_run_interpolates_origin_opacity_and_overrides_and_snaps_the_shape();
}

/// Between two runs of one shaping, halfway: `origin` and `opacity`
/// are their midpoints and the bounds follow the origin; every override
/// vector is element-wise their midpoints (a color per channel only
/// where both sides have one, and otherwise the side the snap picks). A
/// run of another shaping keeps the picked side's `shaped` and `zoom`
/// until its snap point; the whole element at `t = 0` is the before
/// and at `t = 1` the after.
pub fn do_run_interpolates_origin_opacity_and_overrides_and_snaps_the_shape() {
    let shape = four_glyph_shape();
    let before = run_with(
        shape.clone(),
        Vec2::new(0.0, 0.0),
        0.2,
        vec![
            RunOverrideOp::Offset {
                range: 0..4,
                value: Vec2::ZERO,
            },
            RunOverrideOp::Color {
                range: 0..2,
                value: Some(Rgba::new(0, 100, 200, 0)),
            },
            RunOverrideOp::Scale {
                range: 0..4,
                value: 1.0,
            },
        ],
    );
    let after = run_with(
        shape.clone(),
        Vec2::new(10.0, -20.0),
        1.0,
        vec![
            RunOverrideOp::Offset {
                range: 0..4,
                value: Vec2::new(10.0, 20.0),
            },
            RunOverrideOp::Color {
                range: 1..3,
                value: Some(Rgba::new(100, 0, 0, 254)),
            },
            RunOverrideOp::Scale {
                range: 0..4,
                value: 3.0,
            },
        ],
    );

    for snap in [Snap::End, Snap::Start] {
        let mid = GfxElement::interpolate(&before, &after, 0.5, snap);
        let run = mid.glyph_run().unwrap();
        assert_eq!(run.origin(), Vec2::new(5.0, -10.0), "{snap:?}: origin");
        assert_eq!(
            run.bounds(),
            (Vec2::new(105.0, 40.0), Vec2::new(30.0, 40.0)),
            "{snap:?}: the bounds follow the origin"
        );
        assert!(close(run.opacity(), 0.6), "{snap:?}: opacity {}", run.opacity());
        assert_eq!(
            run.overrides().offset(),
            &[Vec2::new(5.0, 10.0); 4],
            "{snap:?}: offsets"
        );
        assert_eq!(run.overrides().scale(), &[2.0; 4], "{snap:?}: scales");
        // Glyph 1 is colored on both sides: a per-channel midpoint.
        // Glyph 0 only before, glyph 2 only after: no blend, so the side
        // the snap picks (End: before, Start: after). Glyph 3 neither.
        let colors = run.overrides().color();
        assert_eq!(
            colors[1],
            Some(Rgba::new(50, 50, 100, 127)),
            "{snap:?}: glyph 1 blends"
        );
        assert_eq!(colors[3], None, "{snap:?}: neither side colors glyph 3");
        match snap {
            Snap::End => {
                assert_eq!(colors[0], Some(Rgba::new(0, 100, 200, 0)));
                assert_eq!(colors[2], None);
            }
            Snap::Start => {
                assert_eq!(colors[0], None);
                assert_eq!(colors[2], Some(Rgba::new(100, 0, 0, 254)));
            }
        }
    }

    // The endpoints are the elements themselves.
    assert!(GfxElement::interpolate(&before, &after, 0.0, Snap::End).same_state(&before));
    assert!(GfxElement::interpolate(&before, &after, 1.0, Snap::End).same_state(&after));
    assert!(GfxElement::interpolate(&before, &after, 0.0, Snap::Start).same_state(&before));
    assert!(GfxElement::interpolate(&before, &after, 1.0, Snap::Start).same_state(&after));

    // `shaped` and `zoom` snap: two runs of different shapings, and zoom
    // windows.
    let boxed = box_shape(Vec2::new(80.0, 30.0), 4);
    let mut from = run_with(shape.clone(), Vec2::ZERO, 1.0, vec![]);
    let mut to = run_with(boxed.clone(), Vec2::new(10.0, 10.0), 0.0, vec![]);
    let window = |min| {
        GfxMutation::RunSet(Box::new(
            GlyphRunData::new(
                shape.clone(),
                Vec2::ZERO,
                GlyphOverrides::none(),
                1.0,
                Some(ZoomWindow {
                    min: Some(min),
                    max: None,
                }),
            )
            .unwrap(),
        ))
    };
    apply(&mut from, window(1.0));
    let to_window = ZoomWindow {
        min: Some(2.0),
        max: None,
    };
    apply(
        &mut to,
        GfxMutation::RunSet(Box::new(
            GlyphRunData::new(
                boxed.clone(),
                Vec2::new(10.0, 10.0),
                GlyphOverrides::none(),
                0.0,
                Some(to_window),
            )
            .unwrap(),
        )),
    );
    let before_run = from.glyph_run().unwrap();
    let after_run = to.glyph_run().unwrap();
    let end = GfxElement::interpolate(&from, &to, 0.5, Snap::End);
    let end_run = end.glyph_run().unwrap();
    assert!(
        end_run.shaped() == before_run.shaped(),
        "End: the shaping is still the before's"
    );
    assert_eq!(end_run.zoom(), before_run.zoom());
    assert!(close(end_run.opacity(), 0.5), "its opacity still interpolates");
    let start = GfxElement::interpolate(&from, &to, 0.5, Snap::Start);
    let start_run = start.glyph_run().unwrap();
    assert!(
        start_run.shaped() == after_run.shaped(),
        "Start: the shaping is the after's"
    );
    assert_eq!(start_run.zoom(), after_run.zoom());
    let done = GfxElement::interpolate(&from, &to, 1.0, Snap::End);
    assert!(
        done.glyph_run().unwrap().shaped() == after_run.shaped(),
        "End takes the after at t = 1"
    );
    assert_eq!(
        done.glyph_run().unwrap().bounds(),
        (Vec2::new(10.0, 10.0), Vec2::new(80.0, 30.0)),
        "the bounds are the after shaping's, at the after origin"
    );
    // The midpoint of a run whose bounds come from the picked shaping
    // at the interpolated origin: the before's box moved halfway.
    assert_eq!(
        end_run.bounds(),
        (Vec2::new(105.0, 55.0), Vec2::new(30.0, 40.0)),
        "End at 0.5: the before's glyph boxes, the origin halfway to (10, 10)"
    );

    // Overrides of different lengths do not blend: a run with none and a
    // run with some take the picked side's whole vectors.
    let plain = run_with(shape.clone(), Vec2::ZERO, 1.0, vec![]);
    let coloured = run_with(
        shape,
        Vec2::ZERO,
        1.0,
        vec![RunOverrideOp::Scale {
            range: 0..4,
            value: 3.0,
        }],
    );
    let end = GfxElement::interpolate(&plain, &coloured, 0.5, Snap::End);
    assert!(end.glyph_run().unwrap().overrides().is_empty());
    let start = GfxElement::interpolate(&plain, &coloured, 0.5, Snap::Start);
    assert_eq!(start.glyph_run().unwrap().overrides().scale(), &[3.0; 4]);
}

#[test]
fn test_fill_interpolates_geometry_and_color_and_snaps_the_shape() {
    do_fill_interpolates_geometry_and_color_and_snaps_the_shape();
}

/// Between two fills, halfway: `rect` (origin and size), `color` (per
/// channel, rounded), `fade` and `opacity` are their midpoints; `shape`
/// and `zoom` take the side their snap picks. The endpoints are the
/// fills themselves.
pub fn do_fill_interpolates_geometry_and_color_and_snaps_the_shape() {
    let before = GfxElement::new_fill(
        FillData {
            rect: Rect::new(Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)),
            shape: FillShape::Rect { radius: 4.0 },
            color: Rgba::new(0, 100, 200, 0),
            fade: 0.0,
            opacity: 0.0,
            zoom: Some(ZoomWindow {
                min: Some(1.0),
                max: None,
            }),
        },
        2,
        9,
    );
    let after = GfxElement::new_fill(
        FillData {
            rect: Rect::new(Vec2::new(20.0, 40.0), Vec2::new(200.0, 150.0)),
            shape: FillShape::Ellipse,
            color: Rgba::new(100, 0, 50, 254),
            fade: 10.0,
            opacity: 1.0,
            zoom: None,
        },
        2,
        9,
    );
    for (snap, shape, zoom) in [
        (
            Snap::End,
            FillShape::Rect { radius: 4.0 },
            Some(ZoomWindow {
                min: Some(1.0),
                max: None,
            }),
        ),
        (Snap::Start, FillShape::Ellipse, None),
    ] {
        let mid = GfxElement::interpolate(&before, &after, 0.5, snap);
        let fill = mid.fill().unwrap();
        assert_eq!(
            fill.rect,
            Rect::new(Vec2::new(10.0, 20.0), Vec2::new(150.0, 100.0)),
            "{snap:?}: rect"
        );
        assert_eq!(
            fill.color,
            Rgba::new(50, 50, 125, 127),
            "{snap:?}: color per channel"
        );
        assert!(close(fill.fade, 5.0), "{snap:?}: fade");
        assert!(close(fill.opacity, 0.5), "{snap:?}: opacity");
        assert_eq!(fill.shape, shape, "{snap:?}: shape snaps");
        assert_eq!(fill.zoom, zoom, "{snap:?}: zoom snaps");
        assert_eq!(
            mid.spatial_bounds(),
            Some((Vec2::new(10.0, 20.0), Vec2::new(150.0, 100.0)))
        );
    }
    for snap in [Snap::End, Snap::Start] {
        assert!(GfxElement::interpolate(&before, &after, 0.0, snap).same_state(&before));
        assert!(GfxElement::interpolate(&before, &after, 1.0, snap).same_state(&after));
    }
}

#[test]
fn test_interpolation_between_other_variants_still_snaps_whole() {
    do_interpolation_between_other_variants_still_snaps_whole();
}

/// A run and a fill, a run and an area: different variants share no
/// interpolating field, so the whole element snaps with the snap, and a
/// model still interpolates its position (D24).
pub fn do_interpolation_between_other_variants_still_snaps_whole() {
    let run = base_run();
    let fill = a_fill_element();
    for snap in [Snap::End, Snap::Start] {
        let at_half = GfxElement::interpolate(&run, &fill, 0.5, snap);
        let expected = if snap == Snap::End { &run } else { &fill };
        assert!(at_half.same_state(expected), "{snap:?}");
    }

    let mut before = GfxElement::new_model_non_indexed(GlyphModel::new(), 0, 1);
    let mut after = before.clone();
    before.set_position(Vec2::new(0.0, 0.0));
    after.set_position(Vec2::new(10.0, -20.0));
    let mid = GfxElement::interpolate(&before, &after, 0.5, Snap::End);
    assert_eq!(
        mid.position(),
        Vec2::new(5.0, -10.0),
        "a model's position still interpolates"
    );
}

// ── predicates ────────────────────────────────────────────────────

/// A predicate with one `(field, comparator)`.
fn only(field: GfxElementField, comparator: Comparator) -> Predicate<GfxElement> {
    Predicate {
        fields: vec![(field, comparator)],
        always_match: false,
    }
}

#[test]
fn test_predicates_select_runs_and_fills_by_element_type_and_opacity() {
    do_predicates_select_runs_and_fills_by_element_type_and_opacity();
}

/// `ElementType` selects by variant: of one area, one model, one void,
/// one run and one fill, `Equals(false)` picks exactly the named
/// variant and `Equals(true)` exactly the others; `Exists` is always
/// present, and an ordering comparator on a tag matches nothing.
/// `Opacity` compares a run's or a fill's opacity with the ordering
/// comparators and matches no other element.
pub fn do_predicates_select_runs_and_fills_by_element_type_and_opacity() {
    let elements = [
        (
            "area",
            GfxElement::new_area_non_indexed(GlyphArea::new(1.0, 10.0, Vec2::ZERO, Vec2::ONE), 0),
        ),
        (
            "model",
            GfxElement::new_model_non_indexed(GlyphModel::new(), 0, 1),
        ),
        ("void", GfxElement::new_void(0)),
        ("run", base_run()),
        ("fill", a_fill_element()),
    ];
    for wanted in ["run", "fill", "area", "model", "void"] {
        let tag = match wanted {
            "run" => GfxElementType::GlyphRun,
            "fill" => GfxElementType::Fill,
            "area" => GfxElementType::GlyphArea,
            "model" => GfxElementType::GlyphModel,
            _ => GfxElementType::Void,
        };
        let equals = only(GfxElementField::ElementType(tag), Comparator::Equals(false));
        let differs = only(GfxElementField::ElementType(tag), Comparator::Equals(true));
        let matched: Vec<&str> = elements
            .iter()
            .filter(|(_, e)| equals.test(e))
            .map(|(n, _)| *n)
            .collect();
        assert_eq!(matched, vec![wanted], "Equals(false) on {wanted}");
        let others: Vec<&str> = elements
            .iter()
            .filter(|(_, e)| differs.test(e))
            .map(|(n, _)| *n)
            .collect();
        assert_eq!(others.len(), 4, "Equals(true) on {wanted} matches the other four");
        assert!(!others.contains(&wanted));
    }
    let tag = GfxElementType::GlyphRun;
    assert!(only(GfxElementField::ElementType(tag), Comparator::Exists(false)).test(&elements[2].1));
    assert!(!only(GfxElementField::ElementType(tag), Comparator::Exists(true)).test(&elements[2].1));
    assert!(
        !only(GfxElementField::ElementType(tag), Comparator::GreaterThan(false)).test(&elements[3].1),
        "tags are unordered"
    );

    // Opacity: the run holds 0.5, the fill 0.75.
    use ordered_float::OrderedFloat as Of;
    let opacity = |at: f32, comparator| only(GfxElementField::Opacity(Of(at)), comparator);
    assert!(opacity(0.5, Comparator::Equals(false)).test(&elements[3].1));
    assert!(!opacity(0.6, Comparator::Equals(false)).test(&elements[3].1));
    assert!(
        opacity(0.6, Comparator::LessThan(false)).test(&elements[3].1),
        "0.5 < 0.6"
    );
    assert!(
        !opacity(0.5, Comparator::GreaterThan(false)).test(&elements[3].1),
        "0.5 is not > 0.5"
    );
    assert!(
        opacity(0.6, Comparator::GreaterThan(false)).test(&elements[4].1),
        "0.75 > 0.6"
    );
    for (name, element) in &elements[..3] {
        for comparator in [
            Comparator::Equals(true),
            Comparator::LessThan(true),
            Comparator::GreaterThan(true),
        ] {
            assert!(
                !opacity(0.5, comparator).test(element),
                "{name} has no opacity, so no opacity predicate matches it, negated or not"
            );
        }
    }
}

// ── hit-testing ───────────────────────────────────────────────────

/// The tree the hit tests use: a fill `(0, 0)`–`(60, 40)` on channel 0
/// with an area-free void root, and the four-glyph run (bounds `(100,
/// 50)`–`(130, 90)`) on channel 1.
fn hit_tree() -> (GfxTree, NodeId, NodeId) {
    let mut tree = GfxTree::new_non_indexed();
    let root = tree.root;
    let fill = GfxElement::new_fill(
        FillData {
            rect: Rect::new(Vec2::new(0.0, 0.0), Vec2::new(60.0, 40.0)),
            ..a_fill()
        },
        0,
        1,
    );
    let fill = tree.insert_child(root, fill, Placement::Last).unwrap();
    let run = tree
        .insert_child(
            root,
            GfxElement::new_run(run_of(four_glyph_shape(), Vec2::ZERO), 1, 2),
            Placement::Last,
        )
        .unwrap();
    (tree, fill, run)
}

#[test]
fn test_descendant_at_finds_runs_and_fills_by_their_boxes() {
    do_descendant_at_finds_runs_and_fills_by_their_boxes();
}

/// The BVH hit test finds a `GlyphRun` by its bounds and a `Fill` by
/// its rectangle — at their edges (closed boxes), not outside them — and
/// the smallest box wins where they overlap. A run's bounds are the
/// union of its glyph boxes, so a point in the union that no glyph box
/// covers still hits. After a mutation moves the run, the hit follows
/// it. Negative: a point outside both finds nothing, and neither does a
/// point where the run was.
pub fn do_descendant_at_finds_runs_and_fills_by_their_boxes() {
    let (mut tree, fill, run) = hit_tree();
    assert_eq!(
        tree.descendant_at(Vec2::new(30.0, 20.0)),
        Some(fill),
        "inside the fill"
    );
    assert_eq!(
        tree.descendant_at(Vec2::new(0.0, 0.0)),
        Some(fill),
        "the fill's corner"
    );
    assert_eq!(
        tree.descendant_at(Vec2::new(60.0, 40.0)),
        Some(fill),
        "the far corner is inside"
    );
    assert_eq!(
        tree.descendant_at(Vec2::new(110.0, 60.0)),
        Some(run),
        "inside the run, on a glyph"
    );
    assert_eq!(
        tree.descendant_at(Vec2::new(126.0, 55.0)),
        Some(run),
        "inside the union of the glyph boxes, where no glyph box is"
    );
    assert_eq!(
        tree.descendant_at(Vec2::new(130.0, 90.0)),
        Some(run),
        "the run's far corner"
    );
    for outside in [
        Vec2::new(80.0, 20.0),
        Vec2::new(60.1, 40.1),
        Vec2::new(99.9, 50.0),
        Vec2::new(130.1, 90.0),
        Vec2::new(500.0, 500.0),
        Vec2::new(-1.0, -1.0),
    ] {
        assert_eq!(tree.descendant_at(outside), None, "{outside} is in neither box");
    }

    // The smallest box wins: a fill covering the run's box loses to it.
    let big = GfxElement::new_fill(
        FillData {
            rect: Rect::new(Vec2::new(90.0, 40.0), Vec2::new(100.0, 100.0)),
            ..a_fill()
        },
        2,
        3,
    );
    let root = tree.root;
    let big = tree.insert_child(root, big, Placement::Last).unwrap();
    assert_eq!(
        tree.descendant_at(Vec2::new(110.0, 60.0)),
        Some(run),
        "the run is smaller than the fill under it"
    );
    assert_eq!(
        tree.descendant_at(Vec2::new(170.0, 120.0)),
        Some(big),
        "outside the run, the big fill"
    );

    // Mutating the run moves what is hit: the cache is dropped with it.
    let mut mutators = GfxMutatorTree::new();
    let mroot = mutators.root;
    mutators
        .insert_child(
            mroot,
            GfxMutator::new(GfxMutation::RunOrigin(Vec2::new(-100.0, -50.0)), 1),
            Placement::Last,
        )
        .unwrap();
    mutators.apply_to(&mut tree, &());
    assert_eq!(
        tree.descendant_at(Vec2::new(10.0, 10.0)),
        Some(run),
        "the run now covers (0, 0)–(30, 40)"
    );
    assert_eq!(
        tree.descendant_at(Vec2::new(50.0, 30.0)),
        Some(fill),
        "and no longer reaches the fill's right half"
    );
    assert_eq!(
        tree.descendant_at(Vec2::new(110.0, 60.0)),
        Some(big),
        "or the place it was"
    );
}

#[test]
fn test_descendants_aabb_covers_runs_and_fills() {
    do_descendants_aabb_covers_runs_and_fills();
}

/// The tree's overall box is the union of every spatial descendant, a
/// run and a fill included — not only the areas.
pub fn do_descendants_aabb_covers_runs_and_fills() {
    let (tree, _, _) = hit_tree();
    assert_eq!(
        tree.descendants_aabb(),
        Some((Vec2::new(0.0, 0.0), Vec2::new(130.0, 90.0))),
        "the fill's corner to the run's"
    );
    let mut empty = GfxTree::new_non_indexed();
    let root = empty.root;
    empty
        .insert_child(
            root,
            GfxElement::new_run(run_of(box_shape(Vec2::ZERO, 0), Vec2::ZERO), 0, 1),
            Placement::Last,
        )
        .unwrap();
    assert_eq!(empty.descendants_aabb(), None, "a run with no box adds none");
}

// ── journals ──────────────────────────────────────────────────────

/// Every node in pre-order with its element.
fn snapshot(tree: &GfxTree) -> Vec<(NodeId, GfxElement)> {
    tree.descendants()
        .map(|id| (id, tree.arena()[id].get().clone()))
        .collect()
}

fn same_snapshot(a: &[(NodeId, GfxElement)], b: &[(NodeId, GfxElement)]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|((ia, ea), (ib, eb))| ia == ib && ea.same_state(eb))
}

#[test]
fn test_journal_round_trips_a_tree_of_runs_and_fills() {
    do_journal_round_trips_a_tree_of_runs_and_fills();
}

/// A journaled walk changes a run (origin, opacity, a range of
/// overrides, in one macro), replaces another run's shaping with
/// `RunSet`, recolors a fill and replaces another fill — and leaves a
/// sibling area alone. The journal names exactly the four changed
/// elements, `revert` restores the snapshot taken before (the very
/// `Arc`s included: `same_state` compares shaping by identity), and
/// `replay` the one taken after.
pub fn do_journal_round_trips_a_tree_of_runs_and_fills() {
    let mut tree = GfxTree::new_non_indexed();
    let root = tree.root;
    let mut ids = Vec::new();
    for (channel, element) in [
        (
            0,
            GfxElement::new_run(run_of(four_glyph_shape(), Vec2::ZERO), 0, 1),
        ),
        (
            1,
            GfxElement::new_run(
                run_of(box_shape(Vec2::new(40.0, 10.0), 2), Vec2::new(3.0, 3.0)),
                1,
                2,
            ),
        ),
        (2, GfxElement::new_fill(a_fill(), 2, 6)),
        (
            3,
            GfxElement::new_fill(
                FillData {
                    opacity: 1.0,
                    ..a_fill()
                },
                3,
                3,
            ),
        ),
        (
            4,
            GfxElement::new_area_non_indexed(GlyphArea::new(1.0, 10.0, Vec2::ZERO, Vec2::ONE), 4),
        ),
    ] {
        assert_eq!(element.channel(), channel);
        ids.push(tree.insert_child(root, element, Placement::Last).unwrap());
    }
    let before = snapshot(&tree);

    let replacement_run = GlyphRunData::new(
        box_shape(Vec2::new(99.0, 11.0), 2),
        Vec2::new(8.0, 8.0),
        GlyphOverrides::new(2),
        0.25,
        None,
    )
    .unwrap();
    let replacement_fill = FillData {
        shape: FillShape::Ellipse,
        ..a_fill()
    };
    let mut mutators = GfxMutatorTree::new();
    let mroot = mutators.root;
    for mutator in [
        GfxMutator::new_macro(
            vec![
                GfxMutation::RunOrigin(Vec2::new(50.0, 60.0)),
                GfxMutation::RunOpacity(0.5),
                GfxMutation::RunOverrides(RunOverrideOp::Offset {
                    range: 1..3,
                    value: Vec2::new(0.0, -2.0),
                }),
            ],
            0,
        ),
        GfxMutator::new(GfxMutation::RunSet(Box::new(replacement_run)), 1),
        GfxMutator::new(GfxMutation::FillColor(Rgba::new(10, 20, 30, 255)), 2),
        GfxMutator::new(GfxMutation::FillSet(Box::new(replacement_fill)), 3),
    ] {
        mutators.insert_child(mroot, mutator, Placement::Last).unwrap();
    }
    let mut builder = JournalBuilder::new();
    mutators
        .apply_at_journaled(&mut tree, &Anchors::Nodes(vec![root]), |_, _| (), &mut builder)
        .unwrap();
    let after = snapshot(&tree);
    let journal = builder.close(&tree);
    let touched: Vec<NodeId> = journal.touched().map(|(id, _)| id).collect();
    assert_eq!(
        touched,
        ids[..4].to_vec(),
        "the four changed elements, not the area"
    );
    assert!(!same_snapshot(&before, &after), "the walk changed the tree");

    journal.revert(&mut tree).unwrap();
    assert!(
        same_snapshot(&snapshot(&tree), &before),
        "revert restores runs and fills"
    );
    tree.check_sorted().unwrap();
    journal.replay(&mut tree).unwrap();
    assert!(
        same_snapshot(&snapshot(&tree), &after),
        "replay reproduces the walk"
    );
}

// ── the wire ──────────────────────────────────────────────────────

#[test]
fn test_authored_glyph_layer_mutations_have_the_documented_wire_shape() {
    do_authored_glyph_layer_mutations_have_the_documented_wire_shape();
}

/// The authored run and fill mutations serialize to the spellings
/// `format/mutations.md` publishes, and the example block in that doc
/// — read from the file, not restated — parses to exactly the
/// mutations it describes. `RunSet` and `FillSet`, which hold shaped
/// text, are `#[serde(skip)]`: serializing one is an error and no file
/// can spell one.
pub fn do_authored_glyph_layer_mutations_have_the_documented_wire_shape() {
    let authored = [
        (GfxMutation::RunOpacity(0.5), r#"{"RunOpacity":0.5}"#),
        (
            GfxMutation::RunOrigin(Vec2::new(12.0, -4.0)),
            r#"{"RunOrigin":[12.0,-4.0]}"#,
        ),
        (
            GfxMutation::RunOverrides(RunOverrideOp::Offset {
                range: 0..3,
                value: Vec2::new(0.0, -2.0),
            }),
            r#"{"RunOverrides":{"Offset":{"range":{"start":0,"end":3},"value":[0.0,-2.0]}}}"#,
        ),
        (
            GfxMutation::RunOverrides(RunOverrideOp::Color {
                range: 0..3,
                value: Some(Rgba::new(255, 0, 0, 255)),
            }),
            r#"{"RunOverrides":{"Color":{"range":{"start":0,"end":3},"value":[255,0,0,255]}}}"#,
        ),
        (
            GfxMutation::RunOverrides(RunOverrideOp::Color {
                range: 4..6,
                value: None,
            }),
            r#"{"RunOverrides":{"Color":{"range":{"start":4,"end":6},"value":null}}}"#,
        ),
        (
            GfxMutation::RunOverrides(RunOverrideOp::Scale {
                range: 3..6,
                value: 1.5,
            }),
            r#"{"RunOverrides":{"Scale":{"range":{"start":3,"end":6},"value":1.5}}}"#,
        ),
        (
            GfxMutation::RunOverrides(RunOverrideOp::Clear),
            r#"{"RunOverrides":"Clear"}"#,
        ),
        (
            GfxMutation::FillColor(Rgba::new(30, 60, 90, 255)),
            r#"{"FillColor":[30,60,90,255]}"#,
        ),
        (GfxMutation::FillOpacity(0.25), r#"{"FillOpacity":0.25}"#),
        (
            GfxMutation::FillRect(Rect::new(Vec2::new(0.0, 0.0), Vec2::new(120.0, 60.0))),
            r#"{"FillRect":{"origin":[0.0,0.0],"size":[120.0,60.0]}}"#,
        ),
    ];
    for (mutation, wire) in &authored {
        assert_eq!(&serde_json::to_string(mutation).unwrap(), wire);
        let parsed: GfxMutation = serde_json::from_str(wire).unwrap();
        assert_eq!(&parsed, mutation, "{wire} parses back");
    }

    let run_set = GfxMutation::RunSet(Box::new(run_of(four_glyph_shape(), Vec2::ZERO)));
    assert!(
        serde_json::to_string(&run_set).is_err(),
        "RunSet has no wire form"
    );
    let fill_set = GfxMutation::FillSet(Box::new(a_fill()));
    assert!(
        serde_json::to_string(&fill_set).is_err(),
        "FillSet has no wire form"
    );
    assert!(
        serde_json::from_str::<GfxMutation>(r#"{"RunSet":{}}"#).is_err(),
        "no file spells RunSet"
    );
    assert!(
        serde_json::from_str::<GfxMutation>(r#"{"FillSet":{}}"#).is_err(),
        "no file spells FillSet"
    );

    // The doc's example, read from the doc.
    let doc = documented_json_block(
        &format_doc_path("mutations.md"),
        "### Glyph run and fill mutations",
        0,
    );
    #[derive(serde::Deserialize)]
    struct Documented {
        mutations: Vec<GfxMutation>,
    }
    let documented: Documented = serde_json::from_str(&doc)
        .unwrap_or_else(|e| panic!("the example in format/mutations.md must parse: {e}\n{doc}"));
    let expected: Vec<GfxMutation> = authored.into_iter().map(|(m, _)| m).collect();
    assert_eq!(
        documented.mutations, expected,
        "the doc publishes exactly these, in this order"
    );

    // The predicate spellings, the same way.
    let predicates = documented_json_block(
        &format_doc_path("mutations.md"),
        "### Glyph run and fill mutations",
        1,
    );
    let fields: Vec<GfxElementField> = serde_json::from_str(&predicates).unwrap_or_else(|e| {
        panic!("the predicate example in format/mutations.md must parse: {e}\n{predicates}")
    });
    assert_eq!(fields.len(), 3);
    assert!(matches!(
        fields[0],
        GfxElementField::ElementType(GfxElementType::GlyphRun)
    ));
    assert!(matches!(
        fields[1],
        GfxElementField::ElementType(GfxElementType::Fill)
    ));
    assert!(matches!(&fields[2], GfxElementField::Opacity(o) if o.0 == 0.5));
}

// ── depth ─────────────────────────────────────────────────────────

/// How deep the deep test goes: §20 rule 11's chain.
const DEEP: usize = 100_000;

#[test]
fn test_hit_test_reaches_a_run_and_a_fill_in_a_deep_chain() {
    do_hit_test_reaches_a_run_and_a_fill_in_a_deep_chain();
}

/// A glyph tree `DEEP` levels deep, a fill and then a run under its
/// bottom void — a map is untrusted and its depth bounded by memory
/// alone (D30) — is hit-tested, boxed and dropped on a default thread:
/// the passes that learned to read a run's and a fill's box
/// (`ensure_subtree_aabbs`, `descendant_at`, `descendants_aabb`) iterate,
/// and spend nothing per level beyond one entry. The same body runs
/// under wasm32 in debug in the `wasm_font_lock` probe (§20 rule 11).
pub fn do_hit_test_reaches_a_run_and_a_fill_in_a_deep_chain() {
    let mut tree = GfxTree::new_non_indexed();
    let mut at = tree.root;
    for _ in 0..DEEP {
        at = tree
            .insert_child(at, GfxElement::new_void(0), Placement::Last)
            .unwrap();
    }
    let fill = GfxElement::new_fill(
        FillData {
            rect: Rect::new(Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0)),
            ..a_fill()
        },
        0,
        1,
    );
    let fill = tree.insert_child(at, fill, Placement::Last).unwrap();
    let run = GfxElement::new_run(
        run_of(box_shape(Vec2::new(20.0, 20.0), 1), Vec2::new(10.0, 10.0)),
        0,
        2,
    );
    let run = tree.insert_child(fill, run, Placement::Last).unwrap();

    assert_eq!(
        tree.descendant_at(Vec2::new(15.0, 15.0)),
        Some(run),
        "the run, the smaller box"
    );
    assert_eq!(
        tree.descendant_at(Vec2::new(80.0, 80.0)),
        Some(fill),
        "the fill, beside it"
    );
    assert_eq!(tree.descendant_at(Vec2::new(500.0, 500.0)), None);
    assert_eq!(
        tree.descendants_aabb(),
        Some((Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0)))
    );
    assert_eq!(
        tree.subtree_aabb(tree.root),
        Some((Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0))),
        "the box reaches the root through every level"
    );
}
