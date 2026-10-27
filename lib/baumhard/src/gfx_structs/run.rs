// SPDX-License-Identifier: MPL-2.0

//! `GlyphRun` — the glyph layer's one element for a whole TextFlow
//! (design §12.1, §13.2): a reference to what was shaped, the placement
//! that positions it, and the per-glyph presentation effects layered on
//! top.
//!
//! **A run is one element whatever its glyph count.** It never splits
//! into per-glyph elements. What it holds is shared, not copied:
//!
//! - [`RunShape::Box`](crate::gfx_structs::run::RunShape::Box) is a box flow (a section, a label) — the shaped
//!   box, drawn whole with its top-left at the run's `origin`.
//! - [`RunShape::Path`](crate::gfx_structs::run::RunShape::Path) is a path flow (a frame ring, a connection, a
//!   portal marker) — the shaped run and the placement of its glyphs on
//!   the canvas, one position per placed glyph. Point placement
//!   ([`place_at_point`](crate::flow::place_at_point)) is also this
//!   shape.
//!
//! Both hold `Arc`s, so cloning a run — a journal's before-image, an
//! animation frame — copies pointers, not shaping. Re-placing a run
//! (moving a node) swaps the `placed` pointer and reshapes nothing.
//!
//! **Overrides** ([`GlyphOverrides`](crate::gfx_structs::run::GlyphOverrides)) are per-glyph presentation effects
//! — an offset, a color, a scale — that glyph-layer presentations write
//! through [`RunOverrideOp`](crate::gfx_structs::run::RunOverrideOp). They are indexed by **placed glyph**:
//! index *i* is the *i*-th [`PlacedGlyph`](crate::flow::PlacedGlyph) of a
//! path run (not the *i*-th of its [`ShapedRun::glyphs`](crate::flow::ShapedRun::glyphs), because
//! `Repeat` and fillers reuse indices), and the *i*-th glyph of a box
//! run's layout (its [`ShapedBox::glyph_spans`](crate::flow::ShapedBox::glyph_spans) entry). A range op skips
//! filler glyphs (`span ==` [`FILLER_SPAN`](crate::flow::FILLER_SPAN)): a filler is padding the
//! placement added, not text a presentation addresses.
//!
//! **Bounds.** A run's box for hit-testing is computed when its shape or
//! origin is set: for a box run the shaped size at the origin; for a
//! path run the union of its placed glyphs' boxes (`pos .. pos +
//! (advance, line_height)`, offset by the origin). Overrides do not move
//! it — a glyph offset out of the box is not hit.
//!
//! The fields are private because they are tied together: `bounds`
//! derives from `shaped` and `origin`, and `overrides` is as long as
//! the placed glyphs. A run is built by [`GlyphRunData::new`](crate::gfx_structs::run::GlyphRunData::new), which
//! checks the second, and changes only through the glyph mutations
//! (`RunSet`, `RunOrigin`, `RunOpacity`, `RunOverrides`), which keep
//! both.

use std::ops::Range;
use std::sync::Arc;

use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::document::values::{Rgba, ZoomWindow};
use crate::flow::{PlacedRun, ShapedBox, ShapedRun, FILLER_SPAN};
use crate::gfx_structs::fill::{same_f32, same_vec2, same_zoom};

/// What was shaped, and how it is placed.
///
/// Equality is identity: two shapes are equal when they hold the very
/// same `Arc`s ([`Arc::ptr_eq`]) — the identity of the shaping, since a
/// [`ShapedRun`] and a [`ShapedBox`] hold a text buffer and have no
/// value equality. A re-shaped or re-placed run is therefore never
/// equal to the one it replaced, even when the glyphs come out alike.
#[derive(Clone, Debug)]
pub enum RunShape {
    /// A box flow: lines broken at `\n`, drawn whole with its top-left
    /// at the run's `origin`.
    Box(Arc<ShapedBox>),
    /// A path (or point) flow: the glyphs of `run` placed on the canvas
    /// by `placed`.
    Path {
        /// The run the placement refers to: the run placement returned
        /// when it rescaled or filled the one it was given (`Fit`, a
        /// `Once` filler), otherwise the run that was placed. Never the
        /// input run in the first case.
        run: Arc<ShapedRun>,
        /// Where each glyph sits on the canvas, before the run's
        /// `origin` offset.
        placed: Arc<PlacedRun>,
    },
}

impl PartialEq for RunShape {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (RunShape::Box(a), RunShape::Box(b)) => Arc::ptr_eq(a, b),
            (RunShape::Path { run: ra, placed: pa }, RunShape::Path { run: rb, placed: pb }) => {
                Arc::ptr_eq(ra, rb) && Arc::ptr_eq(pa, pb)
            }
            _ => false,
        }
    }
}

impl RunShape {
    /// How many glyphs the run draws, the length an override vector
    /// must have: a path run's placed glyphs, or a box run's laid-out
    /// glyphs. O(1).
    pub fn glyph_count(&self) -> usize {
        match self {
            RunShape::Box(shaped) => shaped.glyph_spans.len(),
            RunShape::Path { placed, .. } => placed.glyphs.len(),
        }
    }

    /// Whether placed glyph `index` is filler: padding a `Once` frame
    /// added, which no span owns ([`FILLER_SPAN`]). `false` past the
    /// end. O(1).
    pub(crate) fn is_filler(&self, index: usize) -> bool {
        match self {
            RunShape::Box(shaped) => shaped
                .glyph_spans
                .get(index)
                .is_some_and(|(span, _)| *span == FILLER_SPAN),
            RunShape::Path { run, placed } => placed
                .glyphs
                .get(index)
                .and_then(|glyph| run.glyphs.get(glyph.glyph as usize))
                .is_some_and(|metrics| metrics.span == FILLER_SPAN),
        }
    }

    /// The shape's box relative to the run's origin, as `(top_left,
    /// size)`: a box run's shaped size from the origin itself; a path
    /// run's union of its placed glyphs' boxes. A glyph that does not
    /// name a glyph of the run, or whose box is not finite, adds
    /// nothing, so the result is finite. Zero size when nothing
    /// contributes.
    ///
    /// Cost: O(1) for a box; O(placed glyphs) for a path.
    fn extent(&self) -> (Vec2, Vec2) {
        match self {
            RunShape::Box(shaped) => (Vec2::ZERO, shaped.size),
            RunShape::Path { run, placed } => {
                let mut min = Vec2::splat(f32::INFINITY);
                let mut max = Vec2::splat(f32::NEG_INFINITY);
                for glyph in &placed.glyphs {
                    let Some(metrics) = run.glyphs.get(glyph.glyph as usize) else {
                        continue;
                    };
                    let corner = glyph.pos;
                    let far = corner + Vec2::new(metrics.advance, run.line_height);
                    if !(corner.is_finite() && far.is_finite()) {
                        continue;
                    }
                    min = min.min(corner);
                    max = max.max(far);
                }
                if min.x <= max.x && min.y <= max.y {
                    (min, max - min)
                } else {
                    (Vec2::ZERO, Vec2::ZERO)
                }
            }
        }
    }
}

/// Per-glyph presentation effects, indexed by placed glyph.
///
/// Each of the three vectors is empty (no override of that kind) or
/// exactly as long as the run's [`RunShape::glyph_count`], and the three
/// always have the same length: [`GlyphOverrides::none`] is all empty,
/// and [`GlyphOverrides::new`] fills all three with neutral values.
/// A range op on a run without overrides first materializes `new(len)`.
///
/// Neutral means "draw the glyph as shaped": a zero offset, no color
/// (`None`: the glyph's own color) and scale `1.0`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphOverrides {
    offset: Vec<Vec2>,
    color: Vec<Option<Rgba>>,
    scale: Vec<f32>,
}

impl GlyphOverrides {
    /// No overrides: three empty vectors. O(1), no allocation.
    pub fn none() -> Self {
        GlyphOverrides::default()
    }

    /// Neutral overrides for `len` placed glyphs: a zero offset, no
    /// color and scale `1.0` each. O(len); three allocations.
    pub fn new(len: usize) -> Self {
        GlyphOverrides {
            offset: vec![Vec2::ZERO; len],
            color: vec![None; len],
            scale: vec![1.0; len],
        }
    }

    /// How many placed glyphs these overrides cover; `0` for
    /// [`none`](Self::none). O(1).
    pub fn len(&self) -> usize {
        self.offset.len()
    }

    /// Whether there are no overrides. O(1).
    pub fn is_empty(&self) -> bool {
        self.offset.is_empty()
    }

    /// Each placed glyph's offset, added to its position; empty when
    /// there are no overrides. O(1).
    pub fn offset(&self) -> &[Vec2] {
        &self.offset
    }

    /// Each placed glyph's color, replacing its own when `Some`; empty
    /// when there are no overrides. O(1).
    pub fn color(&self) -> &[Option<Rgba>] {
        &self.color
    }

    /// Each placed glyph's scale, a multiple of its size; empty when
    /// there are no overrides. O(1).
    pub fn scale(&self) -> &[f32] {
        &self.scale
    }

    /// Whether `self` and `other` hold the same state: every override,
    /// floats compared so that a `NaN` equals a `NaN`. O(len).
    pub(crate) fn same_state(&self, other: &Self) -> bool {
        self.offset.len() == other.offset.len()
            && self
                .offset
                .iter()
                .zip(&other.offset)
                .all(|(a, b)| same_vec2(*a, *b))
            && self.color == other.color
            && self.scale.len() == other.scale.len()
            && self.scale.iter().zip(&other.scale).all(|(a, b)| same_f32(*a, *b))
    }

    /// Interpolation writes: the overrides between `from` and `to` at
    /// `t`, element-wise per vector when both vectors have the same
    /// length, the vectors of `base` otherwise. `base` is the overrides
    /// the element's snap picked. O(len).
    pub(crate) fn lerped(from: &Self, to: &Self, base: &Self, t: f32) -> Self {
        use crate::engine::interpolate::Interpolate;
        let mut out = base.clone();
        if from.offset.len() == to.offset.len() {
            out.offset = from
                .offset
                .iter()
                .zip(&to.offset)
                .map(|(a, b)| <Vec2 as Interpolate>::lerp(a, b, t))
                .collect();
        }
        if from.color.len() == to.color.len() {
            // A glyph colored on both sides blends per channel; a glyph
            // colored on one side only has nothing to blend with, so it
            // takes the side `base` was picked from.
            out.color = from
                .color
                .iter()
                .zip(&to.color)
                .zip(&base.color)
                .map(|((a, b), picked)| match (a, b) {
                    (Some(a), Some(b)) => Some(Rgba::lerp(a, b, t)),
                    _ => *picked,
                })
                .collect();
        }
        if from.scale.len() == to.scale.len() {
            out.scale = from
                .scale
                .iter()
                .zip(&to.scale)
                .map(|(a, b)| f32::lerp(a, b, t))
                .collect();
        }
        out
    }
}

/// An edit to a run's [`GlyphOverrides`]: what the glyph-layer
/// presentations write.
///
/// `range` indexes placed glyphs and is clamped to the run's glyph
/// count, so a range past the end is an empty range, not an error.
/// Filler glyphs in a range are skipped ([`FILLER_SPAN`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RunOverrideOp {
    /// Set the offset of every glyph in `range` to `value`.
    Offset {
        /// The placed glyphs to write.
        range: Range<usize>,
        /// The offset, in canvas units, added to each glyph's position.
        value: Vec2,
    },
    /// Set the color override of every glyph in `range`; `None` returns
    /// them to their own color.
    Color {
        /// The placed glyphs to write.
        range: Range<usize>,
        /// The color, or `None` for no override.
        value: Option<Rgba>,
    },
    /// Set the scale of every glyph in `range` to `value`, a multiple of
    /// its shaped size.
    Scale {
        /// The placed glyphs to write.
        range: Range<usize>,
        /// The scale; `1.0` is as shaped.
        value: f32,
    },
    /// Drop every override: the run is drawn as shaped.
    Clear,
}

/// Why a [`GlyphRunData`] could not be built.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RunError {
    /// The overrides are neither empty nor as long as the run's placed
    /// glyphs.
    OverrideLength {
        /// How many glyphs the overrides cover.
        overrides: usize,
        /// How many placed glyphs the run has.
        glyphs: usize,
    },
}

/// What one `GlyphRun` element holds. See the [module
/// docs](crate::gfx_structs::run).
///
/// Cloning copies the two `Arc`s and the override vectors (if any), and
/// shares the shaping.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRunData {
    shaped: RunShape,
    origin: Vec2,
    overrides: GlyphOverrides,
    opacity: f32,
    zoom: Option<ZoomWindow>,
    /// The shape's box relative to `origin`: `(top_left, size)`.
    /// Derived from `shaped` when it is set, so moving the run is O(1).
    extent: (Vec2, Vec2),
}

impl GlyphRunData {
    /// A run of `shaped` at `origin`, drawn with `overrides`, multiplied
    /// by `opacity` (`1.0` is opaque) and rendered inside `zoom`.
    ///
    /// `origin` is the canvas position of a box run's top-left, and the
    /// offset applied to every placed glyph of a path run. The bounds are
    /// computed here.
    ///
    /// Errors with [`RunError::OverrideLength`] when `overrides` is
    /// neither empty nor as long as the run's placed glyphs
    /// ([`RunShape::glyph_count`]).
    ///
    /// Cost: O(placed glyphs) for the bounds of a path run; O(1) for a
    /// box run.
    pub fn new(
        shaped: RunShape,
        origin: Vec2,
        overrides: GlyphOverrides,
        opacity: f32,
        zoom: Option<ZoomWindow>,
    ) -> Result<Self, RunError> {
        let glyphs = shaped.glyph_count();
        if !overrides.is_empty() && overrides.len() != glyphs {
            return Err(RunError::OverrideLength {
                overrides: overrides.len(),
                glyphs,
            });
        }
        let extent = shaped.extent();
        Ok(GlyphRunData {
            shaped,
            origin,
            overrides,
            opacity,
            zoom,
            extent,
        })
    }

    /// What was shaped, and how it is placed. O(1).
    pub fn shaped(&self) -> &RunShape {
        &self.shaped
    }

    /// The canvas position of a box run's top-left, or the offset
    /// applied to every placed glyph of a path run. O(1).
    pub fn origin(&self) -> Vec2 {
        self.origin
    }

    /// The per-glyph presentation effects; empty means none. O(1).
    pub fn overrides(&self) -> &GlyphOverrides {
        &self.overrides
    }

    /// Multiplied into every glyph's alpha. O(1).
    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    /// The zoom range the run renders in, evaluated at render time and
    /// never by projection. O(1).
    pub fn zoom(&self) -> Option<ZoomWindow> {
        self.zoom
    }

    /// The run's canvas box as `(top_left, size)`, the form
    /// [`TreeElement::spatial_bounds`](crate::engine::element::TreeElement::spatial_bounds)
    /// answers: a box run's shaped size at the origin, a path run's union
    /// of its glyph boxes offset by the origin. O(1).
    pub fn bounds(&self) -> (Vec2, Vec2) {
        (self.origin + self.extent.0, self.extent.1)
    }

    /// Whether `self` and `other` hold the same state: the shape by
    /// identity ([`RunShape`]'s equality) and every other field by
    /// value, floats compared so that a `NaN` equals a `NaN`. The bounds are
    /// derived from the shape and the origin and are not compared.
    ///
    /// Cost: O(placed glyphs) with overrides; O(1) without.
    pub fn same_state(&self, other: &Self) -> bool {
        self.shaped == other.shaped
            && same_vec2(self.origin, other.origin)
            && self.overrides.same_state(&other.overrides)
            && same_f32(self.opacity, other.opacity)
            && same_zoom(&self.zoom, &other.zoom)
    }

    /// Move the run: set the origin. The bounds follow. O(1).
    pub(crate) fn set_origin(&mut self, origin: Vec2) {
        self.origin = origin;
    }

    /// Set the opacity. O(1).
    pub(crate) fn set_opacity(&mut self, opacity: f32) {
        self.opacity = opacity;
    }

    /// Apply one override op. A range is clamped to the glyph count and
    /// skips filler glyphs; the first write to a run without overrides
    /// materializes [`GlyphOverrides::new`] for it, and an op that
    /// writes nothing leaves the run untouched.
    ///
    /// Cost: O(glyphs in range); O(glyph count) once, to materialize.
    pub(crate) fn apply_override(&mut self, op: &RunOverrideOp) {
        let count = self.shaped.glyph_count();
        let range = match op {
            RunOverrideOp::Clear => {
                self.overrides = GlyphOverrides::none();
                return;
            }
            RunOverrideOp::Offset { range, .. }
            | RunOverrideOp::Color { range, .. }
            | RunOverrideOp::Scale { range, .. } => clamp_range(range, count),
        };
        if range.is_empty() {
            return;
        }
        if self.overrides.is_empty() {
            self.overrides = GlyphOverrides::new(count);
        }
        for index in range {
            if self.shaped.is_filler(index) {
                continue;
            }
            match op {
                RunOverrideOp::Offset { value, .. } => self.overrides.offset[index] = *value,
                RunOverrideOp::Color { value, .. } => self.overrides.color[index] = *value,
                RunOverrideOp::Scale { value, .. } => self.overrides.scale[index] = *value,
                RunOverrideOp::Clear => {}
            }
        }
    }

    /// The overrides, replaced by the interpolation of two runs'. See
    /// [`GlyphOverrides::lerped`]. O(len).
    pub(crate) fn set_overrides(&mut self, overrides: GlyphOverrides) {
        self.overrides = overrides;
    }
}

/// `range` with its end clamped to `len`. A range that starts at or past
/// the end it is left with, or runs backward, is empty.
fn clamp_range(range: &Range<usize>, len: usize) -> Range<usize> {
    range.start..range.end.min(len)
}
