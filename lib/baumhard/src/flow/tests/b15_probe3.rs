// SPDX-License-Identifier: MPL-2.0
//! Scratch probe for #308 (not committed): the same frames in both trees.

use glam::Vec2;

use crate::document::values::{FitMode, LineSpan, Percent, Rgba, Shape};
use crate::flow::place::halves::{BUDGET_OVERRIDE, FLOOR_OVERRIDE, TRACKS};
use crate::flow::tests::halves_tests::{frame_with, with_seam, QUATRAIN};
use crate::flow::{outline, Rect, RingLayout};

#[test]
fn b15_probe3_sides() {
    let text = "To see a World in a Grain of Sand";
    for (name, shape) in [
        ("ellipse", Shape::Ellipse),
        ("square", Shape::Rectangle { radius: Percent::new(0) }),
    ] {
        let shape = outline(Rect::new(Vec2::ZERO, Vec2::splat(200_000.0)), shape);
        let layout = RingLayout {
            span: LineSpan::NATURAL,
            sides: Some(("x".repeat(200), Rgba::opaque(0, 0, 0))),
            ..RingLayout::whole()
        };
        let lay = || frame_with(text, 16.0, &shape, &FitMode::Once { filler: None }, &layout, true);
        let (_, placed, trace) = lay();
        eprintln!(
            "PROBE long-sides {name} own budget: spent {} budget {} glyphs {} sides {:?}",
            trace.spent,
            trace.budget,
            placed.glyphs.len(),
            trace.unheld.sides
        );
        let (_, placed, trace) = with_seam(&FLOOR_OVERRIDE, 2_000_000, lay);
        eprintln!(
            "PROBE long-sides {name} 2M: spent {} budget {} glyphs {} sides {:?}",
            trace.spent,
            trace.budget,
            placed.glyphs.len(),
            trace.unheld.sides
        );
    }
}

#[test]
fn b15_probe3_kept_short() {
    let text = "To see a World in a Grain of Sand";
    let shape = outline(
        Rect::new(Vec2::ZERO, Vec2::splat(2_000.0)),
        Shape::Rectangle { radius: Percent::new(0) },
    );
    let layout = RingLayout {
        span: LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
        sides: Some(("=".to_string(), Rgba::opaque(0, 0, 0))),
        ..RingLayout::whole()
    };
    let fit = FitMode::Once { filler: None };
    let (_, placed, trace) = frame_with(text, 16.0, &shape, &fit, &layout, true);
    eprintln!(
        "PROBE kept-short full: spent {} glyphs {} sides {:?}",
        trace.spent,
        placed.glyphs.len(),
        trace.unheld.sides
    );
    let mut last = None;
    for budget in (1_000..8_000).step_by(5) {
        let (_, placed, trace) =
            with_seam(&BUDGET_OVERRIDE, budget, || frame_with(text, 16.0, &shape, &fit, &layout, true));
        let now = (trace.unheld.sides.clone(), placed.glyphs.len());
        if last.as_ref() != Some(&now) {
            eprintln!("PROBE kept-short budget {budget}: {:?}", now);
            last = Some(now);
        }
    }
}

#[test]
fn b15_probe3_cut_apart() {
    let shape = outline(
        Rect::new(Vec2::ZERO, Vec2::new(600.0, 12.0)),
        Shape::Rectangle { radius: Percent::new(0) },
    );
    let layout = RingLayout {
        span: LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
        ..RingLayout::whole()
    };
    let before = TRACKS.with(std::cell::Cell::get);
    let (_, _, trace) = frame_with(QUATRAIN, 22.0, &shape, &FitMode::Once { filler: None }, &layout, false);
    eprintln!(
        "PROBE cut-apart tracks {} spent {} cut_apart {:?}",
        TRACKS.with(std::cell::Cell::get) - before,
        trace.spent,
        trace.cut_apart
    );
}
