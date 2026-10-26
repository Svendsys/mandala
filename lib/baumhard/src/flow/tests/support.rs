// SPDX-License-Identifier: MPL-2.0

//! Fixtures for the flow tests: resolved styles, runs shaped for real,
//! and synthetic runs whose advances the test chooses.
//!
//! A synthetic run is what most placement tests want: placement reads
//! only a run's advances, its line height and where its grapheme
//! clusters begin, so a run with round-number advances lets a test
//! state the expected geometry as literals instead of computing it
//! through the shaper the code under test also uses (TEST_CONVENTIONS
//! §T13, shape 6). Real shaping is used where the subject is shaping,
//! or a mode that reshapes (`Fit`, a `Once` filler).
//!
//! **The font lock is held only while shaping or placing**
//! ([`with_fonts`](crate::flow::tests::support::with_fonts)), never
//! across an assertion: a failing assertion under the guard would
//! poison the process-wide lock and fail every other font test with it,
//! hiding which one went red.

use std::sync::Arc;

use glam::Vec2;

use crate::document::resolve::ResolvedText;
use crate::document::values::{FontSize, Rgba, Weight};
use crate::flow::{shape_run, FlowSpan, GlyphMetrics, ShapedRun};
use crate::font::fonts::{self, acquire_font_system_write, AppFont, FontSystemWriteGuard};
use crate::font::{Buffer, Metrics};

/// A Latin face compiled into the binary, so real shaping does not
/// depend on the machine's fonts.
pub(crate) const FACE: AppFont = AppFont::GalatiaSIL;

/// A plain resolved style at `points`, in [`FACE`].
pub(crate) fn style(points: f32) -> ResolvedText {
    ResolvedText {
        font: FACE,
        size: FontSize::Pt(points),
        color: Rgba::opaque(255, 255, 255),
        weight: Weight::NORMAL,
        italic: false,
    }
}

/// Run `f` with the font system initialized and this thread holding its
/// guard, and release the guard before returning `f`'s result.
pub(crate) fn with_fonts<R>(f: impl FnOnce(&mut FontSystemWriteGuard) -> R) -> R {
    fonts::init();
    let mut fs = acquire_font_system_write("flow::tests");
    f(&mut fs)
}

/// `text` shaped for real in one span of `style`, at scale 1.
pub(crate) fn real_run(text: &str, style: &ResolvedText, fs: &mut FontSystemWriteGuard) -> ShapedRun {
    shape_run(
        &[FlowSpan {
            text,
            style,
            link: None,
        }],
        1.0,
        fs,
    )
}

/// A run of glyphs with these advances and this line height, every
/// glyph its own grapheme cluster.
pub(crate) fn synthetic(advances: &[f32], line_height: f32) -> ShapedRun {
    clustered(
        &advances.iter().map(|&w| (w, true)).collect::<Vec<_>>(),
        line_height,
    )
}

/// A run of glyphs `(advance, begins a grapheme cluster)`.
pub(crate) fn clustered(glyphs: &[(f32, bool)], line_height: f32) -> ShapedRun {
    ShapedRun {
        buffer: Arc::new(Buffer::new_empty(Metrics::new(line_height, line_height))),
        glyphs: glyphs
            .iter()
            .enumerate()
            .map(|(i, &(advance, grapheme_start))| GlyphMetrics {
                span: 0,
                bytes: i..i + 1,
                advance,
                ascent: line_height * 0.6,
                descent: line_height * 0.2,
                grapheme_start,
            })
            .collect(),
        line_height,
        pieces: Vec::new(),
    }
}

/// The center of placed glyph `pos`'s `w × h` box.
pub(crate) fn center(pos: Vec2, w: f32, h: f32) -> Vec2 {
    pos + Vec2::new(w, h) * 0.5
}

/// Assert `a` and `b` are within `tolerance`, saying what was compared.
pub(crate) fn close(a: f32, b: f32, tolerance: f32, what: &str) {
    assert!(
        (a - b).abs() <= tolerance,
        "{what}: {a} vs {b} (tolerance {tolerance})"
    );
}
