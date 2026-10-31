// SPDX-License-Identifier: MPL-2.0

//! TEMPORARY CI-only probe from the review of #293: which face shapes a
//! SilhousTwo tab on this machine. It fails on purpose so that CI prints
//! what it found; the commit that adds it is reverted before merge.

use crate::document::resolve::ResolvedText;
use crate::document::values::{FontSize, Rgba, Weight};
use crate::flow::tests::support::with_fonts;
use crate::flow::{shape_run, FlowSpan};
use crate::font::fonts::AppFont;

#[test]
fn test_ci_font_probe_which_face_shapes_a_silhoustwo_tab() {
    let mut out = String::new();
    with_fonts(|fs| {
        out += &format!("faces in the db: {}\n", fs.db().faces().count());
        for face in fs.db().faces() {
            let families: Vec<&str> = face.families.iter().map(|(name, _)| name.as_str()).collect();
            if families
                .iter()
                .any(|f| ["Noto Sans", "DejaVu Sans", "FreeSans"].contains(f))
            {
                out += &format!(
                    "  fallback face {families:?} weight {} {:?} {}\n",
                    face.weight.0, face.style, face.post_script_name
                );
            }
        }
        for (weight, italic) in [(200u16, true), (400, false)] {
            let style = ResolvedText {
                font: AppFont::SilhousTwo,
                size: FontSize::Pt(16.0),
                color: Rgba::new(255, 255, 255, 255),
                weight: Weight::new(weight).unwrap(),
                italic,
            };
            let run = shape_run(
                &[FlowSpan {
                    text: "a \tb",
                    style: &style,
                    link: None,
                }],
                1.0,
                fs,
            );
            for glyph in run.layout_glyphs() {
                let face = fs.db().face(glyph.font_id);
                out += &format!(
                    "SilhousTwo {weight} italic={italic}: bytes {}..{} glyph {} w {} face {:?} {}\n",
                    glyph.start,
                    glyph.end,
                    glyph.glyph_id,
                    glyph.w,
                    face.and_then(|f| f.families.first().map(|(n, _)| n.clone())),
                    face.map(|f| f.post_script_name.clone()).unwrap_or_default(),
                );
            }
        }
    });
    panic!("CI font probe (review of #293), fails on purpose:\n{out}");
}
