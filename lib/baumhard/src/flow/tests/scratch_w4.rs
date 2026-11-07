//! SCRATCH (#309 worker 4): not for commit.
use super::halves_tests::{frame_with, sweep_outlines, QUATRAIN};
use super::work_tests::Kind;
use crate::document::values::{FitMode, LineSpan, Rgba};
use crate::flow::RingLayout;

const ECCLESIASTES: &str = "and a time to every purpose under the heaven: ";

fn stats(v: &[f64]) -> (f64, f64, f64, f64) {
    let n = v.len().max(1) as f64;
    let mean = v.iter().sum::<f64>() / n;
    let sd = (v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n).sqrt();
    let lo = v.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    (mean, sd, lo, hi)
}

#[test]
fn scratch_bias() {
    use crate::flow::place::halves::{NO_CLOSED_FORM, XX_BIAS};
    for off in [false, true] {
        NO_CLOSED_FORM.with(|c| c.set(off));
        let mut by: Vec<(Kind, Vec<(f32, f32, f32, f32)>)> = Vec::new();
        for (_, shape, _, points) in sweep_outlines() {
            let kind = Kind::of(&shape);
            XX_BIAS.with(|b| *b.borrow_mut() = Some(Vec::new()));
            for text in [ECCLESIASTES, QUATRAIN] {
                for outward in [true, false] {
                    for fit in [FitMode::Fit { min: 0.5, max: 2.0 }, FitMode::Once { filler: None }] {
                        for sides in [None, Some(("·".to_owned(), Rgba::opaque(9, 9, 9)))]
                            .into_iter()
                            .take(if outward { 2 } else { 1 })
                        {
                            for span in [LineSpan::WHOLE, LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE), LineSpan::NATURAL] {
                                let layout = RingLayout { span, sides: sides.clone(), ..RingLayout::whole() };
                                let _ = frame_with(text, points, &shape, &fit, &layout, outward);
                            }
                        }
                    }
                }
            }
            let got = XX_BIAS.with(|b| b.borrow_mut().take()).unwrap_or_default();
            match by.iter_mut().find(|(k, _)| *k == kind) {
                Some((_, v)) => v.extend(got),
                None => by.push((kind, got)),
            }
        }
        NO_CLOSED_FORM.with(|c| c.set(false));
        by.sort_by_key(|(k, _)| *k);
        let mut all = Vec::new();
        for (kind, v) in &by {
            report(&format!("{} {kind:?}", if off { "OFF" } else { "ON " }), v);
            all.extend(v.iter().copied());
        }
        report(&format!("{} All", if off { "OFF" } else { "ON " }), &all);
    }
}

fn report(what: &str, v: &[(f32, f32, f32, f32)]) {
    let inw = |x: &&(f32, f32, f32, f32)| x.0 >= -2.5e-4 && x.0 <= 1e-3;
    let all: Vec<f64> = v.iter().filter(inw).map(|x| x.0 as f64).collect();
    let modeled: Vec<f64> = v.iter().filter(inw).filter(|x| x.1 > 0.5).map(|x| x.0 as f64).collect();
    let (a, b) = (stats(&all), stats(&modeled));
    eprintln!(
        "BIAS {what}: letters {} in window {} (out {}): gap-owed mean {:+.3e} sd {:.3e} | modeled {} mean {:+.3e} sd {:.3e}",
        v.len(), all.len(), v.len() - all.len(), a.0, a.1, modeled.len(), b.0, b.1
    );
}

#[test]
fn scratch_solves() {
    use crate::flow::place::halves::{MODEL_LETTERS, SIDE_STEP_LOOKUPS, XX_SOLVE};
    MODEL_LETTERS.with(|c| c.set(0));
    let mut by: Vec<(Kind, [usize; 8], (usize, usize), usize)> = Vec::new();
    for (_, shape, _, points) in sweep_outlines() {
        let kind = Kind::of(&shape);
        XX_SOLVE.with(|w| w.set([0; 8]));
        let b = SIDE_STEP_LOOKUPS.with(std::cell::Cell::get);
        let mut sides = 0;
        for text in [ECCLESIASTES, QUATRAIN] {
            for fit in [FitMode::Fit { min: 0.5, max: 2.0 }, FitMode::Once { filler: None }] {
                for span in [LineSpan::WHOLE, LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE), LineSpan::NATURAL] {
                    let layout = RingLayout { span, sides: Some(("·".to_owned(), Rgba::opaque(9, 9, 9))), ..RingLayout::whole() };
                    let (_, placed, trace) = frame_with(text, points, &shape, &fit, &layout, true);
                    let t: usize = trace.lines.iter().map(|l| l.glyphs.len()).sum();
                    sides += placed.glyphs.len() - t;
                }
            }
        }
        let a = SIDE_STEP_LOOKUPS.with(std::cell::Cell::get);
        let w = XX_SOLVE.with(|w| w.get());
        match by.iter_mut().find(|x| x.0 == kind) {
            Some(x) => { for i in 0..8 { x.1[i] += w[i]; } x.2.0 += a.0 - b.0; x.2.1 += a.1 - b.1; x.3 += sides; }
            None => by.push((kind, w, (a.0 - b.0, a.1 - b.1), sides)),
        }
    }
    eprintln!("MODEL letters {}", MODEL_LETTERS.with(std::cell::Cell::get));
    for (k, w, l, sides) in by {
        eprintln!("PHASE {k:?}: floor-row lookups {} model-lay lookups {} real-lay lookups {}", w[5], w[6], w[7]);
        eprintln!("SOLVES {k:?}: fills {} solves {} (floor answers {}) real lays {} first-probe {} | side steps {} lookups {} sides {} ({:.1}/glyph)",
            w[4], w[0], w[3], w[1], w[2], l.0, l.1, sides, l.1 as f64 / sides.max(1) as f64);
    }
}

#[test]
fn scratch_long_square() {
    use crate::document::values::{Percent, Shape};
    use crate::flow::place::halves::{FLOOR_OVERRIDE, MODEL_LETTERS, NO_CLOSED_FORM, SIDE_STEP_LOOKUPS, STEP_LOOKUPS, XX_PAIR, XX_SOLVE};
    use crate::flow::{outline, Rect};
    use glam::Vec2;
    for (name, sh) in [("square", Shape::Rectangle { radius: Percent::new(0) }), ("ellipse", Shape::Ellipse)] {
        for off in [false, true] {
            if std::env::var_os("XX_DBG").is_some() && (off || name == "ellipse") { continue; }
            NO_CLOSED_FORM.with(|c| c.set(off));
            XX_SOLVE.with(|w| w.set([0; 8]));
            XX_PAIR.with(|w| w.set([0; 8]));
            MODEL_LETTERS.with(|c| c.set(0));
            let shape = outline(Rect::new(Vec2::ZERO, Vec2::splat(200_000.0)), sh.clone());
            let layout = RingLayout { span: LineSpan::NATURAL, sides: Some(("x".repeat(200), Rgba::opaque(0, 0, 0))), ..RingLayout::whole() };
            let l0 = crate::flow::path::PATH_LOOKUPS.with(std::cell::Cell::get);
            let s0 = SIDE_STEP_LOOKUPS.with(std::cell::Cell::get);
            let t0 = STEP_LOOKUPS.with(std::cell::Cell::get);
            FLOOR_OVERRIDE.with(|c| c.set(Some(2_000_000)));
            let (_, placed, trace) = frame_with("To see a World in a Grain of Sand", 16.0, &shape, &FitMode::Once { filler: None }, &layout, true);
            FLOOR_OVERRIDE.with(|c| c.set(None));
            let l1 = crate::flow::path::PATH_LOOKUPS.with(std::cell::Cell::get);
            let s1 = SIDE_STEP_LOOKUPS.with(std::cell::Cell::get);
            let t1 = STEP_LOOKUPS.with(std::cell::Cell::get);
            let w = XX_SOLVE.with(|w| w.get());
            eprintln!("LONG {name} off {off}: glyphs {} lookups {} ({:.1}/glyph) spent {} | side steps {} side lookups {} | all steps {} lookups {} | fills {} solves {} real lays {} first {} | floor-row {} model-lay {} real-lay {} | model letters {} | pair calls {} rounds {} in-window {} broke {}",
                placed.glyphs.len(), l1 - l0, (l1 - l0) as f64 / placed.glyphs.len() as f64, trace.spent,
                s1.0 - s0.0, s1.1 - s0.1, t1.0 - t0.0, t1.1 - t0.1, w[4], w[0], w[1], w[2], w[5], w[6], w[7], MODEL_LETTERS.with(std::cell::Cell::get), XX_PAIR.with(|w| w.get())[0], XX_PAIR.with(|w| w.get())[1], XX_PAIR.with(|w| w.get())[2], XX_PAIR.with(|w| w.get())[3]);
        }
    }
    NO_CLOSED_FORM.with(|c| c.set(false));
}
