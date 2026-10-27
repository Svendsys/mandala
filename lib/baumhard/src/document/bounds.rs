// SPDX-License-Identifier: MPL-2.0

//! Every numeric bound a document value is held to, in one place.
//!
//! A map file is untrusted input, and the numbers in it reach code that
//! is not defensive: a shaper that asserts a non-zero line height, an
//! `f32::clamp` that panics on inverted bounds, a sampler that sizes a
//! buffer from a distance. So each number is checked once, where it
//! enters the document — the loader (V1-B2) and the mutations
//! (V1-B3) — against the bound named here, and nothing behind that
//! boundary checks again.
//!
//! Each bound is a `pub const`, or a `check_*` function that returns
//! the value when it is in bounds and a
//! [`BoundError`](crate::document::bounds::BoundError) naming the bound
//! when it is not. A checker never repairs a value; the one function
//! here that does,
//! [`clamp_canvas_coord`](crate::document::bounds::clamp_canvas_coord),
//! is for *computed* coordinates, which have no author to refuse.

/// The smallest and largest font size in points, defined beside the
/// shaper that enforces them (`crate::font::fonts`) and re-exported so
/// the document and the shaper cannot disagree.
pub use crate::font::fonts::{MAX_FONT_SIZE_PT, MIN_FONT_SIZE_PT};

/// Ceiling on the magnitude of a canvas coordinate: node positions,
/// section offsets and control points. Coordinates are signed, so it
/// bounds `|v|`. Its job is to reject values whose *differences*
/// overflow a sample count, not to limit how large a canvas may be.
pub const MAX_CANVAS_COORD: f32 = 1.0e9;

/// The largest [`Percent`](crate::document::values::Percent).
pub const MAX_PERCENT: u8 = 100;

/// The most control points a path may have: 0 is a line, 1 a quadratic
/// and 2 a cubic Bézier.
pub const MAX_CONTROL_POINTS: usize = 2;

/// The largest ident a node may carry — read at load or issued by the
/// counter, one ceiling for both, so whatever the counter issues loads
/// again. One below `u32::MAX`, so the counter, one past the highest
/// ident, always fits. A document may carry this ident and loads; its
/// counter then has no ident left to issue
/// ([`Document::next_ident`](crate::document::ids::Document::next_ident)
/// is `None`), and the writer refuses to issue one past it
/// ([`DocumentError::IdentSpaceExhausted`](crate::document::ids::DocumentError::IdentSpaceExhausted))
/// rather than issue one twice.
pub const MAX_NODE_IDENT: u32 = u32::MAX - 1;

/// The largest channel a node, section or ring may carry. A channel is
/// a `usize` in the document, and `u32::MAX` is the most a `usize`
/// holds on every target (wasm32's is 32 bits), so a map that is valid
/// on one target is valid on all of them (ruling R11.1 on #236).
pub const MAX_CHANNEL: usize = u32::MAX as usize;

/// The most palettes a map may have: a
/// [`PaletteId`](crate::document::values::PaletteId) is a `u16`.
pub const MAX_PALETTES: usize = 1 << 16;

/// The most theme variables a map may have: a
/// [`VarId`](crate::document::values::VarId) is a `u16`.
pub const MAX_THEME_VARIABLES: usize = 1 << 16;

/// The most theme variants a map may have: the active one is named by a
/// `u16`.
pub const MAX_THEME_VARIANTS: usize = 1 << 16;

/// The most verbs a map may define: a
/// [`VerbId`](crate::document::values::VerbId) is a `u16`.
pub const MAX_VERBS: usize = 1 << 16;

/// The most edits a map may define: an
/// [`EditId`](crate::document::values::EditId) is a `u16`.
pub const MAX_EDITS: usize = 1 << 16;

/// The most presentations a map may define: a
/// [`PresentationId`](crate::document::values::PresentationId) is a
/// `u16`.
pub const MAX_PRESENTATIONS: usize = 1 << 16;

/// The lightest [`Weight`](crate::document::values::Weight).
pub const MIN_WEIGHT: u16 = 100;

/// The heaviest [`Weight`](crate::document::values::Weight).
pub const MAX_WEIGHT: u16 = 900;

/// The step between two [`Weight`](crate::document::values::Weight)s.
pub const WEIGHT_STEP: u16 = 100;

/// Why a value is out of bounds. Every variant carries the offending
/// value and the bound it broke.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum BoundError {
    /// `NaN` or an infinity, where a finite number is required.
    NotFinite(f32),
    /// Below the lowest allowed value. `inclusive` says whether `min`
    /// itself is allowed.
    Below {
        /// The value.
        value: f32,
        /// The bound.
        min: f32,
        /// Whether `min` itself is in bounds.
        inclusive: bool,
    },
    /// Above the highest allowed value, which is itself allowed.
    Above {
        /// The value.
        value: f32,
        /// The bound.
        max: f32,
    },
    /// A range whose lower end is above its upper end.
    Inverted {
        /// The lower end.
        min: f32,
        /// The upper end.
        max: f32,
    },
    /// A weight that is not one of `100, 200, …, 900`.
    NotAWeight(u16),
    /// A node ident outside `1..=`[`MAX_NODE_IDENT`]; `0` is reserved
    /// for "not yet assigned".
    NotAnIdent(u64),
    /// A channel above [`MAX_CHANNEL`].
    NotAChannel(u64),
    /// More items than the bound allows.
    TooMany {
        /// How many there are.
        count: usize,
        /// How many are allowed.
        max: usize,
    },
}

impl std::fmt::Display for BoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoundError::NotFinite(value) => write!(f, "{value} is not a finite number"),
            BoundError::Below {
                value,
                min,
                inclusive: true,
            } => write!(f, "{value} is below the minimum {min}"),
            BoundError::Below {
                value,
                min,
                inclusive: false,
            } => write!(f, "{value} is not above {min}"),
            BoundError::Above { value, max } => write!(f, "{value} is above the maximum {max}"),
            BoundError::Inverted { min, max } => write!(f, "the range {min}..={max} is empty"),
            BoundError::NotAnIdent(value) => {
                write!(f, "{value} is not a node id: 1..={MAX_NODE_IDENT}")
            }
            BoundError::NotAChannel(value) => {
                write!(f, "{value} is not a channel: 0..={MAX_CHANNEL}")
            }
            BoundError::NotAWeight(value) => write!(
                f,
                "{value} is not a weight: {MIN_WEIGHT}..={MAX_WEIGHT} in steps of {WEIGHT_STEP}"
            ),
            BoundError::TooMany { count, max } => write!(f, "{count} items, at most {max} allowed"),
        }
    }
}

impl std::error::Error for BoundError {}

/// `value` if it is finite. O(1).
fn finite(value: f32) -> Result<f32, BoundError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(BoundError::NotFinite(value))
    }
}

/// `value` if it is finite and in `min..=max`. O(1).
fn in_closed(value: f32, min: f32, max: f32) -> Result<f32, BoundError> {
    let value = finite(value)?;
    if value < min {
        Err(BoundError::Below {
            value,
            min,
            inclusive: true,
        })
    } else if value > max {
        Err(BoundError::Above { value, max })
    } else {
        Ok(value)
    }
}

/// A canvas coordinate if it is finite and within
/// ±[`MAX_CANVAS_COORD`]. For coordinates an author or a mutation
/// supplies; a computed one is clamped instead
/// ([`clamp_canvas_coord`]). O(1).
pub fn check_canvas_coord(value: f32) -> Result<f32, BoundError> {
    in_closed(value, -MAX_CANVAS_COORD, MAX_CANVAS_COORD)
}

/// Bring a **computed** canvas coordinate inside ±[`MAX_CANVAS_COORD`].
///
/// A derived position — a layout placing a child at `parent + radius`,
/// a move applied to a subtree — has no author to refuse, and refusing
/// it would drop a node out of its layout; pinning it at the boundary
/// keeps the document valid and the node visible. A non-finite input
/// becomes `0.0`: `NaN` has no nearest edge. O(1).
pub fn clamp_canvas_coord(value: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(-MAX_CANVAS_COORD, MAX_CANVAS_COORD)
}

/// A font size in points if it is in
/// [`MIN_FONT_SIZE_PT`]`..=`[`MAX_FONT_SIZE_PT`]. O(1).
pub fn check_font_size_pt(value: f32) -> Result<f32, BoundError> {
    in_closed(value, MIN_FONT_SIZE_PT, MAX_FONT_SIZE_PT)
}

/// A [`Rel`](crate::document::values::FontSize::Rel) font size if it
/// is in `(0.0, 1.0]`: a positive fraction of the node's shorter side,
/// at most all of it. O(1).
pub fn check_rel_size(value: f32) -> Result<f32, BoundError> {
    let value = finite(value)?;
    if value <= 0.0 {
        Err(BoundError::Below {
            value,
            min: 0.0,
            inclusive: false,
        })
    } else if value > 1.0 {
        Err(BoundError::Above { value, max: 1.0 })
    } else {
        Ok(value)
    }
}

/// A value in `0.0..=1.0`: a node's `opacity`, a label's `t` along
/// its path, and a portal end's `border_t` around its node's outline.
/// O(1).
pub fn check_unit_interval(value: f32) -> Result<f32, BoundError> {
    in_closed(value, 0.0, 1.0)
}

/// A finite value `≥ 0`: a node's `fill_fade`, a frame's `gap` and
/// `inset`, and a path's `spacing`. O(1).
pub fn check_non_negative(value: f32) -> Result<f32, BoundError> {
    in_closed(value, 0.0, f32::MAX)
}

/// The scale range of a [`FitMode::Fit`](crate::document::values::FitMode::Fit)
/// if `0 < min ≤ max`, both finite. O(1).
pub fn check_fit(min: f32, max: f32) -> Result<(f32, f32), BoundError> {
    let min = finite(min)?;
    let max = finite(max)?;
    if min <= 0.0 {
        return Err(BoundError::Below {
            value: min,
            min: 0.0,
            inclusive: false,
        });
    }
    if min > max {
        return Err(BoundError::Inverted { min, max });
    }
    Ok((min, max))
}

/// A percentage if it is at most [`MAX_PERCENT`]. For a value read
/// from outside, where an out-of-range number is an error rather than
/// something to clamp ([`Percent::new`](crate::document::values::Percent::new)
/// clamps). O(1).
pub fn check_percent(value: u8) -> Result<u8, BoundError> {
    if value > MAX_PERCENT {
        Err(BoundError::Above {
            value: f32::from(value),
            max: f32::from(MAX_PERCENT),
        })
    } else {
        Ok(value)
    }
}

/// A weight if it is one of [`MIN_WEIGHT`]`..=`[`MAX_WEIGHT`] in steps
/// of [`WEIGHT_STEP`].
/// [`Weight::new`](crate::document::values::Weight::new) goes through
/// this. O(1).
pub fn check_weight(value: u16) -> Result<u16, BoundError> {
    if (MIN_WEIGHT..=MAX_WEIGHT).contains(&value) && value.is_multiple_of(WEIGHT_STEP) {
        Ok(value)
    } else {
        Err(BoundError::NotAWeight(value))
    }
}

/// A path's control-point count if it is at most
/// [`MAX_CONTROL_POINTS`]. O(1).
pub fn check_control_points(count: usize) -> Result<usize, BoundError> {
    if count > MAX_CONTROL_POINTS {
        Err(BoundError::TooMany {
            count,
            max: MAX_CONTROL_POINTS,
        })
    } else {
        Ok(count)
    }
}

/// A node ident if it is in `1..=`[`MAX_NODE_IDENT`], as the `u32` an
/// ident is. Takes a `u64` so that a file's `id`, `parent` or `to` past
/// `u32::MAX` is refused with this bound rather than with a reader's
/// message about the width of a type. O(1).
pub fn check_node_ident(value: u64) -> Result<u32, BoundError> {
    match u32::try_from(value) {
        Ok(ident) if (1..=MAX_NODE_IDENT).contains(&ident) => Ok(ident),
        _ => Err(BoundError::NotAnIdent(value)),
    }
}

/// A channel if it is at most [`MAX_CHANNEL`]: a node's, a section's or
/// a ring's. O(1).
// On wasm32 a `usize` is 32 bits, so `MAX_CHANNEL` is its maximum and the
// comparison is always true there; on 64-bit targets it is not. One body
// for every target is the point.
#[allow(clippy::absurd_extreme_comparisons)]
pub fn check_channel(channel: usize) -> Result<(), BoundError> {
    if channel <= MAX_CHANNEL {
        Ok(())
    } else {
        Err(BoundError::NotAChannel(
            u64::try_from(channel).unwrap_or(u64::MAX),
        ))
    }
}

/// A count if it is at most `max`: the index bounds
/// ([`MAX_PALETTES`], [`MAX_THEME_VARIABLES`], [`MAX_THEME_VARIANTS`],
/// [`MAX_VERBS`], [`MAX_EDITS`], [`MAX_PRESENTATIONS`]). O(1).
pub fn check_count(count: usize, max: usize) -> Result<usize, BoundError> {
    if count > max {
        Err(BoundError::TooMany { count, max })
    } else {
        Ok(count)
    }
}
