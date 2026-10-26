// SPDX-License-Identifier: MPL-2.0

//! Interpolation: the state between a journal's before and after.
//!
//! A [`Journal`](crate::engine::journal::Journal) holds, for every
//! element it changed, the element before and after. An element type
//! that is [`Interpolable`](crate::engine::interpolate::Interpolable) declares which of its fields
//! move smoothly between the two ([`Interpolate`](crate::engine::interpolate::Interpolate)) and which
//! switch from one to the other at a declared point ([`Snap`](crate::engine::interpolate::Snap)). With
//! that, [`Journal::write_interpolated`](crate::engine::journal::Journal::write_interpolated)
//! shows any change part-way, so a numeric, position or color change
//! animates without animation code of its own.
//!
//! # Exact endpoints
//!
//! Every [`Interpolate`](crate::engine::interpolate::Interpolate) here returns `from` itself at
//! `t == 0` and `to` itself at `t == 1`, not a value computed from them.
//! `from + (to - from) * 1.0` is not always `to` in floating point, and
//! a journal written at `t = 1` has to equal the journal replayed.

use glam::Vec2;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

use crate::engine::element::TreeElement;
use crate::util::ordered_vec2::OrderedVec2;

/// A value that can be blended between two of its kind.
///
/// `lerp(from, to, t)` is `from` at `t == 0`, `to` at `t == 1` — exactly,
/// both — and the straight line between them for `t` in between. `t`
/// outside `0..=1` extrapolates; callers that must not, clamp.
pub trait Interpolate: Sized {
    /// The value at progress `t` from `from` to `to`.
    fn lerp(from: &Self, to: &Self, t: f32) -> Self;
}

/// When a field that does not interpolate switches from the before
/// value to the after value.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Snap {
    /// Switch as soon as the change starts: the after value for every
    /// `t > 0`.
    Start,
    /// Switch when the change ends: the after value only at `t == 1`.
    End,
}

impl Snap {
    /// Whether a snapping field takes the after value at `t`. O(1).
    pub fn takes_after(self, t: f32) -> bool {
        match self {
            Snap::Start => t > 0.0,
            Snap::End => t >= 1.0,
        }
    }

    /// `before` or `after`, whichever a snapping field holds at `t`.
    /// O(1).
    pub fn pick<'a, T>(self, before: &'a T, after: &'a T, t: f32) -> &'a T {
        if self.takes_after(t) {
            after
        } else {
            before
        }
    }
}

/// An element type whose state can be shown between a before and an
/// after, field by field.
pub trait Interpolable: TreeElement {
    /// The element between `before` and `after` at progress `t`, in
    /// `0..=1`. Fields that interpolate use [`Interpolate::lerp`]; every
    /// other field takes `before` or `after` by `snap`
    /// ([`Snap::takes_after`]): [`Snap::Start`] takes `after` for
    /// `t > 0`, [`Snap::End`] takes `after` only at `t == 1`.
    ///
    /// At `t == 0` the result must hold the [same
    /// state](TreeElement::same_state) as `before`, and at `t == 1` as
    /// `after`, whatever the snap.
    fn interpolate(before: &Self, after: &Self, t: f32, snap: Snap) -> Self;
}

/// `from` at `t == 0`, `to` at `t == 1`, `from + (to - from) * t`
/// between. O(1).
#[inline]
fn lerp_f32(from: f32, to: f32, t: f32) -> f32 {
    if t == 0.0 {
        from
    } else if t == 1.0 {
        to
    } else {
        from + (to - from) * t
    }
}

impl Interpolate for f32 {
    fn lerp(from: &f32, to: &f32, t: f32) -> f32 {
        lerp_f32(*from, *to, t)
    }
}

impl Interpolate for f64 {
    /// In `f64` throughout; `t` is widened.
    fn lerp(from: &f64, to: &f64, t: f32) -> f64 {
        if t == 0.0 {
            *from
        } else if t == 1.0 {
            *to
        } else {
            from + (to - from) * f64::from(t)
        }
    }
}

impl Interpolate for OrderedFloat<f32> {
    fn lerp(from: &OrderedFloat<f32>, to: &OrderedFloat<f32>, t: f32) -> OrderedFloat<f32> {
        OrderedFloat(lerp_f32(from.0, to.0, t))
    }
}

impl Interpolate for Vec2 {
    /// Per axis.
    fn lerp(from: &Vec2, to: &Vec2, t: f32) -> Vec2 {
        Vec2::new(lerp_f32(from.x, to.x, t), lerp_f32(from.y, to.y, t))
    }
}

impl Interpolate for OrderedVec2 {
    /// Per axis.
    fn lerp(from: &OrderedVec2, to: &OrderedVec2, t: f32) -> OrderedVec2 {
        OrderedVec2::new_f32(lerp_f32(from.x(), to.x(), t), lerp_f32(from.y(), to.y(), t))
    }
}

impl Interpolate for [f32; 4] {
    /// Per channel — an RGBA color in floats.
    fn lerp(from: &[f32; 4], to: &[f32; 4], t: f32) -> [f32; 4] {
        std::array::from_fn(|i| lerp_f32(from[i], to[i], t))
    }
}

impl Interpolate for [u8; 4] {
    /// Per channel — an RGBA color in bytes — rounded to the nearest
    /// byte and held within `0..=255` when `t` extrapolates.
    fn lerp(from: &[u8; 4], to: &[u8; 4], t: f32) -> [u8; 4] {
        std::array::from_fn(|i| {
            let value = lerp_f32(f32::from(from[i]), f32::from(to[i]), t).round();
            // `as` saturates a float into `u8`'s range, and reads NaN
            // as 0 — the clamp the doc promises.
            value as u8
        })
    }
}
