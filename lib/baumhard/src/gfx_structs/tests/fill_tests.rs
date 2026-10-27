// SPDX-License-Identifier: MPL-2.0

//! The `Fill` element and the glyph mutations that change it
//! (`FillSet`, `FillColor`, `FillOpacity`, `FillRect`). What spans both
//! new element types is in `glyph_layer_tests`.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2).

use glam::Vec2;

use crate::document::values::{Rgba, ZoomWindow};
use crate::engine::element::TreeElement;
use crate::flow::Rect;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::fill::{FillData, FillShape};
use crate::gfx_structs::mutator::GfxMutation;
use crate::gfx_structs::tests::run_tests::apply;

/// A fill of `rect` in a rounded rectangle (radius 8), 80% opaque
/// white, fade 6, opacity 0.75, with a zoom window.
pub fn a_fill() -> FillData {
    FillData {
        rect: Rect::new(Vec2::new(10.0, 20.0), Vec2::new(100.0, 50.0)),
        shape: FillShape::Rect { radius: 8.0 },
        color: Rgba::new(255, 255, 255, 204),
        fade: 6.0,
        opacity: 0.75,
        zoom: Some(ZoomWindow {
            min: Some(0.5),
            max: None,
        }),
    }
}

/// A `Fill` element (channel 1, id 6) holding [`a_fill`].
pub fn a_fill_element() -> GfxElement {
    GfxElement::new_fill(a_fill(), 1, 6)
}

/// The names of the fields of `a` and `b` that differ: `rect`, `shape`,
/// `color`, `fade`, `opacity`, `zoom`.
pub fn changed_fill_fields(a: &FillData, b: &FillData) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if a.rect != b.rect {
        changed.push("rect");
    }
    if a.shape != b.shape {
        changed.push("shape");
    }
    if a.color != b.color {
        changed.push("color");
    }
    if a.fade != b.fade {
        changed.push("fade");
    }
    if a.opacity != b.opacity {
        changed.push("opacity");
    }
    if a.zoom != b.zoom {
        changed.push("zoom");
    }
    changed
}

#[test]
fn test_fill_mutations_set_exactly_their_field() {
    do_fill_mutations_set_exactly_their_field();
}

/// `FillColor`, `FillOpacity` and `FillRect` each change their own
/// field and no other; `FillSet` replaces the whole payload, and a
/// rectangle with it. The element's channel and id are never touched.
pub fn do_fill_mutations_set_exactly_their_field() {
    let base = a_fill();

    let mut element = a_fill_element();
    apply(&mut element, GfxMutation::FillColor(Rgba::new(1, 2, 3, 4)));
    let after = element.fill().unwrap();
    assert_eq!(after.color, Rgba::new(1, 2, 3, 4));
    assert_eq!(changed_fill_fields(&base, after), vec!["color"]);

    let mut element = a_fill_element();
    apply(&mut element, GfxMutation::FillOpacity(0.1));
    let after = element.fill().unwrap();
    assert_eq!(after.opacity, 0.1);
    assert_eq!(changed_fill_fields(&base, after), vec!["opacity"]);

    let mut element = a_fill_element();
    let moved = Rect::new(Vec2::new(-5.0, 7.0), Vec2::new(30.0, 40.0));
    apply(&mut element, GfxMutation::FillRect(moved));
    let after = element.fill().unwrap();
    assert_eq!(after.rect, moved);
    assert_eq!(changed_fill_fields(&base, after), vec!["rect"]);
    assert_eq!(
        element.spatial_bounds(),
        Some((Vec2::new(-5.0, 7.0), Vec2::new(30.0, 40.0))),
        "the hit-test box is the rectangle"
    );

    let replacement = FillData {
        rect: Rect::new(Vec2::new(1.0, 2.0), Vec2::new(3.0, 4.0)),
        shape: FillShape::Ellipse,
        color: Rgba::new(9, 8, 7, 6),
        fade: 0.0,
        opacity: 1.0,
        zoom: None,
    };
    let mut element = a_fill_element();
    apply(&mut element, GfxMutation::FillSet(Box::new(replacement.clone())));
    assert_eq!(element.fill().unwrap(), &replacement);
    assert_eq!(
        changed_fill_fields(&base, element.fill().unwrap()),
        vec!["rect", "shape", "color", "fade", "opacity", "zoom"]
    );
    assert_eq!((element.channel(), element.unique_id()), (1, 6));
}

#[test]
fn test_fill_element_answers_position_and_debug() {
    do_fill_element_answers_position_and_debug();
}

/// A fill's element position is its rectangle's top-left, and setting
/// it moves the rectangle without resizing it. A fill with no area is
/// not hit-testable.
pub fn do_fill_element_answers_position_and_debug() {
    let mut element = a_fill_element();
    assert_eq!(element.position(), Vec2::new(10.0, 20.0));
    element.set_position(Vec2::new(0.0, -3.0));
    assert_eq!(
        element.fill().unwrap().rect,
        Rect::new(Vec2::new(0.0, -3.0), Vec2::new(100.0, 50.0))
    );
    assert_eq!(format!("{element:?}"), "Fill(id=6, ch=1)");

    for size in [
        Vec2::new(0.0, 50.0),
        Vec2::new(50.0, 0.0),
        Vec2::new(-1.0, 5.0),
        Vec2::NAN,
    ] {
        let mut flat = a_fill();
        flat.rect.size = size;
        assert_eq!(
            GfxElement::new_fill(flat, 0, 1).spatial_bounds(),
            None,
            "a fill of size {size} has no box"
        );
    }
}
