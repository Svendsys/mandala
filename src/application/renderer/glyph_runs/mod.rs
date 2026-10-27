// SPDX-License-Identifier: MPL-2.0

//! Positioned glyph runs, spike candidate (c): every glyph of a
//! `GlyphRun` handed to the vendored glyphon (`lib/glyphon`) as a
//! `PositionedGlyph`, prepared in the main text pass's own prepare and
//! drawn in its one draw, from glyphon's one atlas.

pub(crate) mod emit;
#[cfg(test)]
mod tests;

use baumhard::font::Color;
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::gfx_structs::run::GlyphRunData;
use glam::Vec2;
use glyphon::{LayoutGlyph, PositionedGlyph};

use self::emit::{emit_run, GlyphSprite};

/// What the last collected frame held.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GlyphRunCounts {
    /// Frames collected.
    pub(crate) prepares: u64,
    /// Runs handed in.
    pub(crate) runs: usize,
    /// Runs drawn.
    pub(crate) runs_drawn: usize,
    /// Glyphs handed to glyphon.
    pub(crate) instances: usize,
    /// Glyphs skipped.
    pub(crate) skipped: usize,
    /// Glyphs glyphon's atlas holds after the frame (all text).
    pub(crate) atlas_entries: usize,
    /// Glyphs glyphon's atlas has rasterized, all text, running.
    pub(crate) atlas_rasterized: u64,
}

/// The positioned glyphs of one frame's runs.
#[derive(Default)]
pub(crate) struct GlyphRunPass {
    pub(crate) positioned: Vec<PositionedGlyph>,
    pub(crate) counts: GlyphRunCounts,
}

impl GlyphRunPass {
    #[cfg(test)]
    pub(crate) fn counts(&self) -> GlyphRunCounts {
        self.counts
    }

    /// Flatten every glyph of `runs` that `camera` shows.
    pub(crate) fn collect<'r>(&mut self, camera: &Camera2D, runs: impl Iterator<Item = (&'r GlyphRunData, Vec2)>) {
        self.positioned.clear();
        let mut frame = GlyphRunCounts {
            prepares: self.counts.prepares + 1,
            ..GlyphRunCounts::default()
        };
        let mut layout: Vec<&'r LayoutGlyph> = Vec::new();
        let positioned = &mut self.positioned;
        for (run, offset) in runs {
            frame.runs += 1;
            let emitted = emit_run(run, offset, camera, &mut layout, &mut |sprite: GlyphSprite| {
                let [r, g, b, a] = sprite.color;
                positioned.push(PositionedGlyph {
                    cache_key: sprite.key,
                    x: sprite.pen.0,
                    y: sprite.pen.1,
                    color: Color::rgba(r, g, b, a),
                });
            });
            if emitted.drawn {
                frame.runs_drawn += 1;
            }
            frame.skipped += emitted.skipped;
        }
        frame.instances = self.positioned.len();
        self.counts = frame;
    }
}
