// SPDX-License-Identifier: MPL-2.0
//! Scratch probe for #308 / B15.1 (not committed).

use glam::Vec2;

use crate::document::values::{Direction, Edge, FitMode, LineSpan, Percent, Shape, StartPoint};
use crate::flow::path::ArcPath;
use crate::flow::place::MIN_TABLE_SPACING;
use crate::flow::tests::halves_tests::{frame_with, QUATRAIN};
const ECCLESIASTES: &str = "and a time to every purpose under the heaven: ";
use crate::flow::tests::support::{style, with_fonts};
use crate::flow::{outline, shape_run, FlowSpan, Outline, Rect, RingLayout, FILLER_SPAN};
use crate::font::fonts::glyph_outline_ink;
use crate::font::{FontSystem, SwashCache};

/// x-height per pt of the probe style, measured independently.
fn x_per_pt(points: f32) -> f32 {
    with_fonts(|fs| {
        let flow = style(points);
        let run = shape_run(
            &[FlowSpan {
                text: "x",
                style: &flow,
                link: None,
            }],
            1.0,
            fs,
        );
        let fs: &mut FontSystem = fs;
        let mut cache = SwashCache::new();
        let g = run.layout_glyphs().next().cloned().unwrap();
        let (min, _) = glyph_outline_ink(fs, &mut cache, &g).unwrap();
        -min.y / g.font_size
    })
}

pub(crate) fn probe(name: &str, shape: &Outline, text: &str, points: f32, layout: &RingLayout, fit: &FitMode, outward: bool) {
    let (run, placed, trace) = frame_with(text, points, shape, fit, layout, outward);
    let xpp = x_per_pt(points);
    let left = StartPoint { edge: Edge::Left, at: 0.5 };
    let path = ArcPath::closed(shape, left, Direction::Clockwise, MIN_TABLE_SPACING);
    let boxes: Vec<Option<(Vec2, Vec2)>> = with_fonts(|fs| {
        let fs: &mut FontSystem = fs;
        let mut cache = SwashCache::new();
        let laid: Vec<_> = run.layout_glyphs().cloned().collect();
        placed
            .glyphs
            .iter()
            .map(|g| glyph_outline_ink(fs, &mut cache, &laid[g.glyph as usize]))
            .collect()
    });
    let d = trace.offset.unwrap_or(f32::NAN);
    let mut all = Vec::new();
    for (k, g) in placed.glyphs.iter().enumerate() {
        let m = &run.glyphs[g.glyph as usize];
        if m.blank {
            continue;
        }
        let Some((min, max)) = boxes[k] else { continue };
        let pen = g.pos + Vec2::new(0.0, run.baseline());
        let ay = if m.span == FILLER_SPAN { 0.5 * (min.y + max.y) } else { -0.5 * xpp * m.size };
        let a = pen + Vec2::new(0.5 * (min.x + max.x), ay);
        let s = path.nearest(a);
        let (q, _) = path.at(s);
        let dist = (a - q).length() * if path_inside(&path, a, s) { -1.0 } else { 1.0 };
        all.push((k, dist, m.span == FILLER_SPAN));
    }
    for l in &trace.lines {
        let (mut letters, mut words) = (Vec::new(), Vec::new());
        let mut prev: Option<(Vec2, Vec2)> = None;
        let mut spaced = false;
        let mut spaces = Vec::new();
        for k in l.glyphs.clone() {
            let m = &run.glyphs[placed.glyphs[k].glyph as usize];
            if m.blank { spaced = true; spaces.push(m.advance); continue; }
            let Some((min, max)) = boxes[k] else { continue };
            let pen = placed.glyphs[k].pos + Vec2::new(0.0, run.baseline());
            let ink = (pen + min, pen + max);
            if let Some(p) = prev {
                let g = (ink.0 - p.1).max(p.0 - ink.1).max(Vec2::ZERO).length();
                if spaced { words.push(g) } else { letters.push(g) }
            }
            prev = Some(ink);
            spaced = false;
        }
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
        let mx = |v: &[f32]| v.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let mn = |v: &[f32]| v.iter().cloned().fold(f32::INFINITY, f32::min);
        println!("   first {:?} last {:?}", l.glyphs.clone().find_map(|k| boxes[k].map(|(a,b)| { let pen = placed.glyphs[k].pos + Vec2::new(0.0, run.baseline()); (pen+a, pen+b) })), l.glyphs.clone().rev().find_map(|k| boxes[k].map(|(a,b)| { let pen = placed.glyphs[k].pos + Vec2::new(0.0, run.baseline()); (pen+a, pen+b) })));
        println!("   line half {} part {:.2}..{:.2} letters {:.3}..{:.3} mean {:.3}; words {:.3}..{:.3} mean {:.3}; nominal word {:.3}", l.half, l.from, l.to, mn(&letters), mx(&letters), mean(&letters), mn(&words), mx(&words), mean(&words), mx(&spaces) + 2.0 * mean(&letters));
    }
    let lo = all.iter().map(|a| a.1).fold(f32::INFINITY, f32::min);
    let hi = all.iter().map(|a| a.1).fold(f32::NEG_INFINITY, f32::max);
    let res = all.iter().map(|a| (a.1 - d).abs()).fold(0.0_f32, f32::max);
    println!(
        "{name} span {:.2} {}: d {d:.4} anchors {lo:.4}..{hi:.4} spread {:.4} residual {res:.4} lines {} sides {} unheld {:?}",
        layout.span.get(),
        if outward { "out" } else { "in" },
        hi - lo,
        trace.lines.len(),
        all.iter().filter(|a| a.2).count(),
        trace.unheld
    );
}

fn path_inside(path: &ArcPath, p: Vec2, s: f32) -> bool {
    let (q, t) = path.at(s);
    (p - q).dot(path.outward(t)) < 0.0
}

#[test]
fn b15_probe() {
    let rect = |w: f32, h: f32| Rect::new(Vec2::ZERO, Vec2::new(w, h));
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    let shape = outline(rect(600.0, 12.0), Shape::Ellipse);
    let layout = RingLayout { span: LineSpan::new(0.5).unwrap(), sides: None, ..RingLayout::whole() };
    let (_, _, trace) = frame_with(ECCLESIASTES, 18.0, &shape, &fit, &layout, false);
    println!("SPENT {} glyphs {}", trace.spent, trace.glyphs);
    let _ = QUATRAIN;
    let _ = probe;
}
