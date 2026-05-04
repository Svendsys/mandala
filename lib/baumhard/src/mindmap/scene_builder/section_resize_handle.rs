// SPDX-License-Identifier: MPL-2.0

//! Section resize-handle emission for the currently-selected
//! section. Sibling of [`super::edge_handle`] — same role pattern,
//! different domain. One function emits 8 handles per `Some`-sized
//! section when the user has selected it; `None`-sized sections
//! (fill-parent — the migration default) get no handles since
//! there's no per-section AABB to stretch.

use std::fmt;

use glam::Vec2;

use super::SELECTED_EDGE_COLOR;

/// Which of the eight resize handles on a `Some`-sized section
/// the cursor is targeting. Order is the conventional clockwise-
/// from-top-left ladder used by most resize-box UIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeHandleSide {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

impl ResizeHandleSide {
    /// Resolve the per-axis growth direction of this handle into a
    /// pair of `(x_axis, y_axis)` factors. `+1` grows the size,
    /// `-1` shifts the offset and shrinks the size, `0` leaves the
    /// axis untouched. Edge-midpoint handles return `0` on the axis
    /// they don't move so the drain can multiply `(dx, dy)` directly
    /// without per-side branching.
    ///
    /// **Coordinate convention.** The result is "size delta along
    /// each axis" — for an N handle (`(0, -1)`), a positive cursor
    /// `dy` shrinks the section's height and grows `offset.y` by the
    /// same amount, so the bottom edge stays put.
    pub fn axis_factors(&self) -> (i8, i8) {
        match self {
            Self::NW => (-1, -1),
            Self::N => (0, -1),
            Self::NE => (1, -1),
            Self::E => (1, 0),
            Self::SE => (1, 1),
            Self::S => (0, 1),
            Self::SW => (-1, 1),
            Self::W => (-1, 0),
        }
    }

    /// Every variant — used by the scene builder to emit one handle
    /// per side and by tests to enumerate the cardinality.
    pub fn all() -> [Self; 8] {
        [
            Self::NW,
            Self::N,
            Self::NE,
            Self::E,
            Self::SE,
            Self::S,
            Self::SW,
            Self::W,
        ]
    }

    /// Stable per-side channel for the in-place mutator dispatch.
    /// Picked to avoid colliding with `edge_handle_channel_for`'s
    /// 1/2/3/100+ space — sections live in a separate canvas role
    /// so the spaces don't actually overlap, but a unique channel
    /// per side here is cheap and easy to reason about.
    pub fn channel(&self) -> usize {
        match self {
            Self::NW => 1,
            Self::N => 2,
            Self::NE => 3,
            Self::E => 4,
            Self::SE => 5,
            Self::S => 6,
            Self::SW => 7,
            Self::W => 8,
        }
    }
}

impl fmt::Display for ResizeHandleSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::NW => "nw",
            Self::N => "n",
            Self::NE => "ne",
            Self::E => "e",
            Self::SE => "se",
            Self::S => "s",
            Self::SW => "sw",
            Self::W => "w",
        };
        f.write_str(s)
    }
}

/// One resize-handle glyph emitted on top of a selected section.
/// Rendered as a small cosmic-text buffer in canvas space — the
/// renderer treats `section_resize_handles` as its own buffer
/// family since the handle set is small (8) and only exists for
/// the currently-selected `Some`-sized section.
pub struct SectionResizeHandleElement {
    /// Owning MindNode id — same id every per-node element keys on.
    pub node_id: String,
    /// Index into [`MindNode.sections`](crate::mindmap::model::MindNode::sections).
    pub section_idx: usize,
    /// Which of the 8 handles this element represents.
    pub side: ResizeHandleSide,
    /// Canvas-space center of the handle.
    pub position: (f32, f32),
    /// Glyph string (single char).
    pub glyph: String,
    /// Color as `#RRGGBB` hex.
    pub color: String,
    /// Font size in points.
    pub font_size_pt: f32,
}

/// Glyph used for section resize handles. A small open square reads
/// distinctly from the edge-handle diamond / midpoint hook so the
/// two families are visually disambiguated when they coexist on a
/// crowded screen.
pub const SECTION_RESIZE_HANDLE_GLYPH: &str = "\u{25A1}"; // □

/// Font size (in points) for the section resize-handle glyphs.
/// Same size as the edge handles — the two families share visual
/// weight so the eye reads them as a unified "grab here" idiom.
pub const SECTION_RESIZE_HANDLE_FONT_SIZE_PT: f32 = 14.0;

/// Build the 8-handle set for a single selected section, given the
/// section's already-resolved (offset-applied) canvas-space AABB.
/// Called at most once per scene build (for the selected section
/// only), so the cost is trivial and needs no cache.
///
/// Returns an empty vector when `size` is `None` (fill-parent
/// sections have no per-section AABB to stretch — the parent's
/// auto-fit floor owns their dimensions). Callers must not invoke
/// this on a deselected section; selection-gating lives in
/// [`super::builder::build_scene_with_cache`].
pub fn build_section_resize_handles(
    node_id: &str,
    section_idx: usize,
    section_pos: Vec2,
    section_size: Option<Vec2>,
) -> Vec<SectionResizeHandleElement> {
    let size = match section_size {
        Some(s) => s,
        None => return Vec::new(),
    };

    let (x, y) = (section_pos.x, section_pos.y);
    let (w, h) = (size.x, size.y);
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    let right = x + w;
    let bottom = y + h;

    let positions = [
        (ResizeHandleSide::NW, (x, y)),
        (ResizeHandleSide::N, (cx, y)),
        (ResizeHandleSide::NE, (right, y)),
        (ResizeHandleSide::E, (right, cy)),
        (ResizeHandleSide::SE, (right, bottom)),
        (ResizeHandleSide::S, (cx, bottom)),
        (ResizeHandleSide::SW, (x, bottom)),
        (ResizeHandleSide::W, (x, cy)),
    ];

    positions
        .into_iter()
        .map(|(side, position)| SectionResizeHandleElement {
            node_id: node_id.to_string(),
            section_idx,
            side,
            position,
            glyph: SECTION_RESIZE_HANDLE_GLYPH.to_string(),
            color: SELECTED_EDGE_COLOR.to_string(),
            font_size_pt: SECTION_RESIZE_HANDLE_FONT_SIZE_PT,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin every variant's axis factors. Drift on any one of these
    /// produces a silent off-by-sign bug in the resize drain — the
    /// section grows when it should shrink, or vice versa.
    #[test]
    fn axis_factors_pin_per_side() {
        assert_eq!(ResizeHandleSide::NW.axis_factors(), (-1, -1));
        assert_eq!(ResizeHandleSide::N.axis_factors(), (0, -1));
        assert_eq!(ResizeHandleSide::NE.axis_factors(), (1, -1));
        assert_eq!(ResizeHandleSide::E.axis_factors(), (1, 0));
        assert_eq!(ResizeHandleSide::SE.axis_factors(), (1, 1));
        assert_eq!(ResizeHandleSide::S.axis_factors(), (0, 1));
        assert_eq!(ResizeHandleSide::SW.axis_factors(), (-1, 1));
        assert_eq!(ResizeHandleSide::W.axis_factors(), (-1, 0));
    }

    #[test]
    fn all_returns_eight_unique_variants() {
        let all = ResizeHandleSide::all();
        assert_eq!(all.len(), 8);
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i], all[j], "duplicate variant in all()");
            }
        }
    }

    #[test]
    fn channel_is_unique_per_side() {
        let mut seen = std::collections::HashSet::new();
        for s in ResizeHandleSide::all() {
            assert!(seen.insert(s.channel()), "duplicate channel for {:?}", s);
        }
    }

    /// `None`-sized sections (fill-parent) get no handles.
    #[test]
    fn build_returns_empty_for_none_sized_section() {
        let handles = build_section_resize_handles("0", 0, Vec2::ZERO, None);
        assert!(handles.is_empty());
    }

    /// `Some`-sized sections get exactly 8 handles, one per side.
    #[test]
    fn build_emits_eight_handles_for_some_sized_section() {
        let handles =
            build_section_resize_handles("0", 1, Vec2::new(10.0, 20.0), Some(Vec2::new(100.0, 50.0)));
        assert_eq!(handles.len(), 8);
        let mut sides: Vec<ResizeHandleSide> = handles.iter().map(|h| h.side).collect();
        sides.sort_by_key(|s| s.channel());
        let mut expected: Vec<ResizeHandleSide> = ResizeHandleSide::all().to_vec();
        expected.sort_by_key(|s| s.channel());
        assert_eq!(sides, expected);
    }

    /// Pin the four corner positions and the four edge-mid
    /// positions for a fixed section AABB. Drift here would shift
    /// every handle visually by the same amount — easy to catch.
    #[test]
    fn build_handle_positions_match_section_aabb() {
        let handles =
            build_section_resize_handles("0", 0, Vec2::new(10.0, 20.0), Some(Vec2::new(100.0, 40.0)));
        let by_side: std::collections::HashMap<ResizeHandleSide, (f32, f32)> =
            handles.iter().map(|h| (h.side, h.position)).collect();
        // Corners.
        assert_eq!(by_side[&ResizeHandleSide::NW], (10.0, 20.0));
        assert_eq!(by_side[&ResizeHandleSide::NE], (110.0, 20.0));
        assert_eq!(by_side[&ResizeHandleSide::SW], (10.0, 60.0));
        assert_eq!(by_side[&ResizeHandleSide::SE], (110.0, 60.0));
        // Edge mids.
        assert_eq!(by_side[&ResizeHandleSide::N], (60.0, 20.0));
        assert_eq!(by_side[&ResizeHandleSide::S], (60.0, 60.0));
        assert_eq!(by_side[&ResizeHandleSide::W], (10.0, 40.0));
        assert_eq!(by_side[&ResizeHandleSide::E], (110.0, 40.0));
    }

    #[test]
    fn display_uses_lowercase_compass_strings() {
        assert_eq!(format!("{}", ResizeHandleSide::NW), "nw");
        assert_eq!(format!("{}", ResizeHandleSide::E), "e");
        assert_eq!(format!("{}", ResizeHandleSide::SE), "se");
    }
}
