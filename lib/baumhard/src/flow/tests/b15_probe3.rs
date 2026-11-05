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

#[test]
fn b15_probe3_span_offsets() {
    use crate::flow::tests::halves_tests::{frame, scenes};
    for (name, shape, text, points) in scenes() {
        let mut row = Vec::new();
        for span in [LineSpan::NATURAL, LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE), LineSpan::WHOLE] {
            let layout = RingLayout { span, ..RingLayout::whole() };
            let (_, _, trace) = frame(text, points, &shape, &layout);
            row.push((trace.offset, trace.lines.iter().map(|l| (l.from, l.to)).collect::<Vec<_>>()));
        }
        eprintln!("PROBE span-offsets {name}: {row:?}");
    }
}

#[test]
fn b15_probe3_thin_looks() {
    use crate::flow::place::halves::BUDGET_OVERRIDE;
    use crate::flow::tests::halves_tests::{frame_with, with_seam, QUATRAIN};
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(1_000.0, 20.0)), Shape::Ellipse);
    let layout = RingLayout {
        span: LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
        sides: Some(("-=".repeat(20), Rgba::opaque(0, 0, 0))),
        ..RingLayout::whole()
    };
    let (_, _, trace) = with_seam(&BUDGET_OVERRIDE, 20_000, || {
        frame_with(QUATRAIN, 22.0, &shape, &FitMode::Fit { min: 0.5, max: 2.0 }, &layout, true)
    });
    eprintln!("PROBE thin looks {:?}", trace.looks);
}

#[test]
fn b15_probe3_nosides() {
    let text = "To see a World in a Grain of Sand";
    for (name, shape) in [
        ("ellipse", Shape::Ellipse),
        ("square", Shape::Rectangle { radius: Percent::new(0) }),
    ] {
        let shape = outline(Rect::new(Vec2::ZERO, Vec2::splat(200_000.0)), shape);
        let layout = RingLayout { span: LineSpan::NATURAL, ..RingLayout::whole() };
        let before = TRACKS.with(std::cell::Cell::get);
        let (_, placed, trace) = frame_with(text, 16.0, &shape, &FitMode::Once { filler: None }, &layout, true);
        eprintln!("PROBE nosides {name}: spent {} tracks {} glyphs {} clear {:?}", trace.spent, TRACKS.with(std::cell::Cell::get) - before, placed.glyphs.len(), trace.clear);
    }
}

#[test]
fn b15_probe3_sides_budgets() {
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
        for budget in [100_000, 150_000, 200_000, 250_000, 300_000, 350_000] {
            let (_, placed, trace) = with_seam(&BUDGET_OVERRIDE, budget, || {
                frame_with(text, 16.0, &shape, &FitMode::Once { filler: None }, &layout, true)
            });
            eprintln!("PROBE sides-budget {name} {budget}: spent {} glyphs {} sides {:?}", trace.spent, placed.glyphs.len(), trace.unheld.sides);
        }
    }
}

#[test]
fn b15_probe3_heavy() {
    let text = "and a time to every purpose under the heaven: ";
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(600.0, 12.0)), Shape::Ellipse);
    let layout = RingLayout { span: LineSpan::NATURAL, ..RingLayout::whole() };
    let before = TRACKS.with(std::cell::Cell::get);
    let (_, placed, trace) = frame_with(text, 18.0, &shape, &FitMode::Fit { min: 0.5, max: 2.0 }, &layout, false);
    eprintln!("PROBE heavy spent {} glyphs {} tracks {} placed {} unheld {:?}", trace.spent, trace.glyphs, TRACKS.with(std::cell::Cell::get) - before, placed.glyphs.len(), trace.unheld);
}

#[test]
fn b15_probe3_heavy2() {
    use crate::flow::place::halves::{BUILDS, INKED_STEPS, SHAPED_STEPS, TRACKED_STEPS};
    let text = "and a time to every purpose under the heaven: ";
    let shape = outline(Rect::new(Vec2::ZERO, Vec2::new(600.0, 12.0)), Shape::Ellipse);
    for span in [LineSpan::NATURAL, LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE)] {
        let layout = RingLayout { span, ..RingLayout::whole() };
        let get = || (TRACKS.with(|c| c.get()), TRACKED_STEPS.with(|c| c.get()), SHAPED_STEPS.with(|c| c.get()), INKED_STEPS.with(|c| c.get()), BUILDS.with(|c| c.get()));
        let b = get();
        let (_, _, trace) = frame_with(text, 18.0, &shape, &FitMode::Fit { min: 0.5, max: 2.0 }, &layout, false);
        let a = get();
        eprintln!("PROBE heavy2 {span:?}: spent {} tracks {} tracked {} shaped {} inked {} builds {} looks {:?}", trace.spent, a.0-b.0, a.1-b.1, a.2-b.2, a.3-b.3, a.4-b.4, trace.looks.worst);
    }
}

