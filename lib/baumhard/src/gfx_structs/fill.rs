// SPDX-License-Identifier: MPL-2.0

//! `Fill` — the glyph layer's node background (design §12.1, §13.4,
//! D12): a color shaped like the node's outline, with a soft edge fade.
//!
//! A fill is one element per node, not one per anything it covers. It
//! carries the rectangle it fills, the [`FillShape`] it takes inside
//! that rectangle, its color, how far its alpha falls off at the edge
//! (`fade`), its opacity and an optional zoom window. The renderer
//! (V1-C2) draws it with the shader fill that already exists; nothing
//! here draws.
//!
//! The rectangle is [`flow::Rect`](crate::flow::Rect), the same one the
//! node's frame runs around, and a rounded fill's corner radius is the
//! radius [`outline`](crate::flow::outline) gives that rectangle — so
//! the fill and the frame share one outline (§13.4).
//!
//! A fill's box for hit-testing is its rectangle: an ellipse fill is
//! hit anywhere in its rectangle, not only inside the ellipse.

use crate::document::values::{Rgba, ZoomWindow};
use crate::flow::Rect;

/// The shape a [`FillData`] takes inside its rectangle.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum FillShape {
    /// The rectangle with circular corners of `radius`, in canvas
    /// units. `0` is a sharp rectangle; half the shorter side is a
    /// circle on a square and a pill on a longer rectangle.
    Rect {
        /// The corner radius, in canvas units.
        radius: f32,
    },
    /// The ellipse inscribed in the rectangle.
    Ellipse,
}

/// A node's background: what one `Fill` element holds.
///
/// Every field is plain data, so the type carries no invariant a
/// constructor would have to keep and its fields are public. A fill
/// changes only through the mutations that name its fields
/// ([`GfxMutation::FillSet`](crate::gfx_structs::mutator::GfxMutation::FillSet),
/// `FillColor`, `FillOpacity`, `FillRect`), and — as for every element
/// — a tree gives no `&mut` access to one outside the walker.
#[derive(Clone, Debug, PartialEq)]
pub struct FillData {
    /// The rectangle the fill covers: its top-left corner and size, in
    /// canvas units. Also the fill's hit-test box.
    pub rect: Rect,
    /// The shape drawn inside `rect`.
    pub shape: FillShape,
    /// The fill color, alpha included. `opacity` multiplies the alpha.
    pub color: Rgba,
    /// How far in from the edge the alpha falls off smoothly, in
    /// canvas units; `0` is a hard edge.
    pub fade: f32,
    /// Multiplied into the color's alpha: `1.0` draws the color as it
    /// is, `0.0` draws nothing.
    pub opacity: f32,
    /// The zoom range the fill renders in. Evaluated at render time,
    /// never by projection.
    pub zoom: Option<ZoomWindow>,
}

impl FillData {
    /// Whether `self` and `other` hold the same state: every field,
    /// floats compared as [`GlyphArea::same_state`](crate::gfx_structs::area::GlyphArea::same_state)
    /// compares them (a `NaN` equals a `NaN`, so an element holding one
    /// is the same state as itself). O(1).
    pub fn same_state(&self, other: &Self) -> bool {
        same_rect(&self.rect, &other.rect)
            && same_shape(&self.shape, &other.shape)
            && self.color == other.color
            && same_f32(self.fade, other.fade)
            && same_f32(self.opacity, other.opacity)
            && same_zoom(&self.zoom, &other.zoom)
    }
}

/// Whether two floats are the same state: equal, or both `NaN`.
pub(crate) fn same_f32(a: f32, b: f32) -> bool {
    a == b || (a.is_nan() && b.is_nan())
}

/// Whether two vectors are the same state, per axis, as [`same_f32`].
pub(crate) fn same_vec2(a: glam::Vec2, b: glam::Vec2) -> bool {
    same_f32(a.x, b.x) && same_f32(a.y, b.y)
}

/// Whether two rectangles are the same state, as [`same_f32`].
fn same_rect(a: &Rect, b: &Rect) -> bool {
    same_vec2(a.origin, b.origin) && same_vec2(a.size, b.size)
}

fn same_shape(a: &FillShape, b: &FillShape) -> bool {
    match (a, b) {
        (FillShape::Rect { radius: a }, FillShape::Rect { radius: b }) => same_f32(*a, *b),
        (FillShape::Ellipse, FillShape::Ellipse) => true,
        _ => false,
    }
}

/// Whether two optional zoom windows are the same state, as
/// [`same_f32`] compares each bound.
pub(crate) fn same_zoom(a: &Option<ZoomWindow>, b: &Option<ZoomWindow>) -> bool {
    fn same_bound(a: Option<f32>, b: Option<f32>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => same_f32(a, b),
            _ => false,
        }
    }
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => same_bound(a.min, b.min) && same_bound(a.max, b.max),
        _ => false,
    }
}
