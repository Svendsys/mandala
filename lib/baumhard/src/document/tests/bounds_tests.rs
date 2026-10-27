// SPDX-License-Identifier: MPL-2.0

//! One accept and one reject per bound, each at the edge: the accepted
//! value is the bound itself where the bound is inclusive, and the
//! rejected one is the nearest value past it.

use crate::document::bounds::*;
use crate::document::values::{Percent, Weight};

#[test]
fn test_document_bounds_canvas_coord() {
    do_document_bounds_canvas_coord();
}

/// ±`MAX_CANVAS_COORD` is in bounds, past it is not, nor is `NaN`; the
/// clamp pins a computed value to the bound and `NaN` to 0.
pub fn do_document_bounds_canvas_coord() {
    assert_eq!(check_canvas_coord(-MAX_CANVAS_COORD), Ok(-MAX_CANVAS_COORD));
    assert_eq!(
        check_canvas_coord(2.0e9),
        Err(BoundError::Above {
            value: 2.0e9,
            max: MAX_CANVAS_COORD
        })
    );
    assert!(matches!(
        check_canvas_coord(f32::NAN),
        Err(BoundError::NotFinite(_))
    ));
    assert_eq!(clamp_canvas_coord(5.0), 5.0);
    assert_eq!(clamp_canvas_coord(1.0e30), MAX_CANVAS_COORD);
    assert_eq!(clamp_canvas_coord(-1.0e30), -MAX_CANVAS_COORD);
    assert_eq!(clamp_canvas_coord(f32::NAN), 0.0);
    assert_eq!(clamp_canvas_coord(f32::INFINITY), 0.0);
}

#[test]
fn test_document_bounds_font_size_pt() {
    do_document_bounds_font_size_pt();
}

/// The shaper's own window: `MIN_FONT_SIZE_PT..=MAX_FONT_SIZE_PT`.
pub fn do_document_bounds_font_size_pt() {
    assert_eq!(check_font_size_pt(MIN_FONT_SIZE_PT), Ok(MIN_FONT_SIZE_PT));
    assert_eq!(check_font_size_pt(MAX_FONT_SIZE_PT), Ok(MAX_FONT_SIZE_PT));
    assert!(matches!(
        check_font_size_pt(0.4),
        Err(BoundError::Below { inclusive: true, .. })
    ));
    assert!(matches!(
        check_font_size_pt(4097.0),
        Err(BoundError::Above { .. })
    ));
}

#[test]
fn test_document_bounds_rel_size() {
    do_document_bounds_rel_size();
}

/// `(0.0, 1.0]`: 1 is in, 0 is out, and so is anything past 1.
pub fn do_document_bounds_rel_size() {
    assert_eq!(check_rel_size(1.0), Ok(1.0));
    assert_eq!(check_rel_size(0.06), Ok(0.06));
    assert_eq!(
        check_rel_size(0.0),
        Err(BoundError::Below {
            value: 0.0,
            min: 0.0,
            inclusive: false
        })
    );
    assert!(matches!(check_rel_size(1.01), Err(BoundError::Above { .. })));
}

#[test]
fn test_document_bounds_unit_interval() {
    do_document_bounds_unit_interval();
}

/// Opacity, label `t` and portal `border_t`: `0.0..=1.0`.
pub fn do_document_bounds_unit_interval() {
    assert_eq!(check_unit_interval(0.0), Ok(0.0));
    assert_eq!(check_unit_interval(1.0), Ok(1.0));
    assert!(matches!(
        check_unit_interval(-0.01),
        Err(BoundError::Below { .. })
    ));
    assert!(matches!(check_unit_interval(1.01), Err(BoundError::Above { .. })));
    assert!(matches!(
        check_unit_interval(f32::NAN),
        Err(BoundError::NotFinite(_))
    ));
}

#[test]
fn test_document_bounds_non_negative() {
    do_document_bounds_non_negative();
}

/// `fill_fade`, `gap`, `inset`, `spacing`: finite and `≥ 0`.
pub fn do_document_bounds_non_negative() {
    assert_eq!(check_non_negative(0.0), Ok(0.0));
    assert_eq!(check_non_negative(1.0e6), Ok(1.0e6));
    assert!(matches!(check_non_negative(-0.5), Err(BoundError::Below { .. })));
    assert!(matches!(
        check_non_negative(f32::INFINITY),
        Err(BoundError::NotFinite(_))
    ));
}

#[test]
fn test_document_bounds_fit() {
    do_document_bounds_fit();
}

/// `Fit { min, max }`: `0 < min ≤ max`.
pub fn do_document_bounds_fit() {
    assert_eq!(check_fit(0.5, 0.5), Ok((0.5, 0.5)));
    assert_eq!(check_fit(0.5, 2.0), Ok((0.5, 2.0)));
    assert!(matches!(
        check_fit(0.0, 2.0),
        Err(BoundError::Below { inclusive: false, .. })
    ));
    assert_eq!(
        check_fit(2.0, 0.5),
        Err(BoundError::Inverted { min: 2.0, max: 0.5 })
    );
    assert!(matches!(check_fit(0.5, f32::NAN), Err(BoundError::NotFinite(_))));
}

#[test]
fn test_document_bounds_percent() {
    do_document_bounds_percent();
}

/// 100 is in, 101 is out; `Percent::new` clamps instead.
pub fn do_document_bounds_percent() {
    assert_eq!(check_percent(MAX_PERCENT), Ok(100));
    assert!(matches!(check_percent(101), Err(BoundError::Above { .. })));
    assert_eq!(Percent::new(250).get(), 100);
    assert_eq!(Percent::new(40).get(), 40);
}

#[test]
fn test_document_bounds_weight() {
    do_document_bounds_weight();
}

/// 100, 200, …, 900 and nothing between or outside.
pub fn do_document_bounds_weight() {
    assert_eq!(check_weight(100), Ok(100));
    assert_eq!(check_weight(900), Ok(900));
    assert_eq!(Weight::new(700), Ok(Weight::BOLD));
    for refused in [0, 450, 1000] {
        assert_eq!(check_weight(refused), Err(BoundError::NotAWeight(refused)));
    }
    assert!(Weight::new(450).is_err());
}

#[test]
fn test_document_bounds_control_points() {
    do_document_bounds_control_points();
}

/// Two control points — a cubic — are in; three are out.
pub fn do_document_bounds_control_points() {
    assert_eq!(MAX_CONTROL_POINTS, 2);
    assert_eq!(check_control_points(2), Ok(2));
    assert_eq!(
        check_control_points(3),
        Err(BoundError::TooMany { count: 3, max: 2 })
    );
}

#[test]
fn test_document_bounds_node_ident() {
    do_document_bounds_node_ident();
}

/// `1..=MAX_NODE_IDENT`: the reserved 0 and `u32::MAX` are out.
pub fn do_document_bounds_node_ident() {
    assert_eq!(check_node_ident(1), Ok(1));
    assert_eq!(check_node_ident(u64::from(MAX_NODE_IDENT)), Ok(MAX_NODE_IDENT));
    assert_eq!(check_node_ident(0), Err(BoundError::NotAnIdent(0)));
    assert_eq!(
        check_node_ident(u64::from(u32::MAX)),
        Err(BoundError::NotAnIdent(u64::from(u32::MAX)))
    );
    // Past the width of an ident is the same bound, not another error.
    assert_eq!(check_node_ident(1 << 32), Err(BoundError::NotAnIdent(1 << 32)));
}

/// `0..=MAX_CHANNEL`; one past it is refused with the value it was given.
#[test]
fn test_document_bounds_channel() {
    assert_eq!(check_channel(0), Ok(()));
    assert_eq!(check_channel(MAX_CHANNEL), Ok(()));
    // Where a `usize` is wide enough to hold a channel past the bound
    // (not wasm32, whose `usize` is the bound).
    #[cfg(target_pointer_width = "64")]
    {
        let past = MAX_CHANNEL + 1;
        let shown = u64::try_from(past).unwrap();
        assert_eq!(check_channel(past), Err(BoundError::NotAChannel(shown)));
        assert_eq!(check_channel(usize::MAX), Err(BoundError::NotAChannel(u64::MAX)));
    }
}

#[test]
fn test_document_bounds_counts() {
    do_document_bounds_counts();
}

/// Every index bound admits exactly as many items as its `u16` index
/// addresses, 65536, and refuses one more.
pub fn do_document_bounds_counts() {
    for max in [
        MAX_PALETTES,
        MAX_THEME_VARIABLES,
        MAX_THEME_VARIANTS,
        MAX_VERBS,
        MAX_EDITS,
        MAX_PRESENTATIONS,
    ] {
        assert_eq!(max, usize::from(u16::MAX) + 1);
        assert_eq!(check_count(max, max), Ok(max));
        assert_eq!(
            check_count(max + 1, max),
            Err(BoundError::TooMany { count: max + 1, max })
        );
    }
}
