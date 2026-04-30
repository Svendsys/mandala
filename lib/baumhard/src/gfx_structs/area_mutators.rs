// SPDX-License-Identifier: MPL-2.0

//! Command and delta mutators for `GlyphArea` — the two `Applicable`
//! implementations that the tree walker dispatches through.

use crate::core::primitives::{Applicable, ApplyOperation, ColorFontRegions, Range};
use crate::font::fonts::AppFont;
use crate::gfx_structs::shape::NodeShape;
use crate::util::color::FloatRgba;
use glam::f32::Vec2;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::ops::Add;
use strum_macros::{Display, EnumIter};

use super::area::GlyphArea;
use super::area_fields::{GlyphAreaField, GlyphAreaFieldType, OutlineStyle};
use super::zoom_visibility::ZoomVisibility;

////////////////////////////////////////
/////// GlyphAreaCommand Mutator ///////
///////////////////////////////////////

/// Tag enum for [`GlyphAreaCommand`] — identifies the command kind
/// without carrying payload. Used as a key in `HashSet`/`HashMap`
/// look-ups where the caller needs to know *which* command was
/// scheduled but not its parameters.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Eq, Hash, EnumIter, Display)]
pub enum GlyphAreaCommandType {
    /// Remove grapheme clusters from the front of the text.
    PopFront,
    /// Remove grapheme clusters from the back of the text.
    PopBack,
    /// Shift position left by a pixel delta.
    NudgeLeft,
    /// Shift position right by a pixel delta.
    NudgeRight,
    /// Shift position down by a pixel delta.
    NudgeDown,
    /// Shift position up by a pixel delta.
    NudgeUp,
    /// Teleport position to an absolute (x, y).
    MoveTo,
    /// Increase font scale by a delta.
    GrowFont,
    /// Decrease font scale by a delta.
    ShrinkFont,
    /// Replace font scale with an absolute value.
    SetFontSize,
    /// Replace the line-height multiplier.
    SetLineHeight,
    /// Increase line-height by a delta.
    GrowLineHeight,
    /// Decrease line-height by a delta.
    ShrinkLineHeight,
    /// Replace render bounds with absolute (w, h).
    SetBounds,
    /// Assign a font to a character range.
    SetRegionFont,
    /// Assign a colour to a character range.
    SetRegionColor,
    /// Remove the colour/font region at a character range.
    DeleteColorFontRegion,
    /// Move an existing region's span to a new character range.
    ChangeRegionRange,
}

/// Imperative mutation command applied to a [`GlyphArea`] via its
/// [`Applicable`] impl. Unlike [`DeltaGlyphArea`] (which is
/// arithmetic — `Add`/`Assign`/`Subtract`), a command performs a
/// single named operation whose semantics are fixed. All variants
/// are O(1) except the `ColorFontRegion`-touching ones, which are
/// O(n) in the number of existing regions.
#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub enum GlyphAreaCommand {
    /// Remove `n` grapheme clusters from the front of the text.
    PopFront(usize),
    /// Remove `n` grapheme clusters from the back of the text.
    PopBack(usize),
    /// Shift position left by the given pixel delta.
    NudgeLeft(f32),
    /// Shift position right by the given pixel delta.
    NudgeRight(f32),
    /// Shift position down by the given pixel delta.
    NudgeDown(f32),
    /// Shift position up by the given pixel delta.
    NudgeUp(f32),
    /// Teleport position to absolute `(x, y)`.
    MoveTo(f32, f32),
    /// Increase font scale by a delta.
    GrowFont(f32),
    /// Decrease font scale by a delta.
    ShrinkFont(f32),
    /// Replace font scale with an absolute value.
    SetFontSize(f32),
    /// Replace the line-height multiplier.
    SetLineHeight(f32),
    /// Increase line-height by a delta.
    GrowLineHeight(f32),
    /// Decrease line-height by a delta.
    ShrinkLineHeight(f32),
    /// Replace render bounds with absolute `(w, h)`.
    SetBounds(f32, f32),
    /// Assign a font to the given character range. O(n) in region count.
    SetRegionFont(Range, AppFont),
    /// Assign a colour to the given character range. O(n) in region count.
    SetRegionColor(Range, FloatRgba),
    /// Remove the colour/font region at the given character range.
    /// O(n) in region count.
    DeleteColorFontRegion(Range),
    /// Move an existing region from `current` to `new` range. O(n) in
    /// region count.
    ChangeRegionRange(Range, Range),
}

impl GlyphAreaCommand {
    /// Discriminant tag for this command, useful for set/map keys.
    /// O(1), no allocation.
    pub fn variant(&self) -> GlyphAreaCommandType {
        match self {
            GlyphAreaCommand::PopFront(_) => GlyphAreaCommandType::PopFront,
            GlyphAreaCommand::PopBack(_) => GlyphAreaCommandType::PopBack,
            GlyphAreaCommand::NudgeLeft(_) => GlyphAreaCommandType::NudgeLeft,
            GlyphAreaCommand::NudgeRight(_) => GlyphAreaCommandType::NudgeRight,
            GlyphAreaCommand::NudgeDown(_) => GlyphAreaCommandType::NudgeDown,
            GlyphAreaCommand::NudgeUp(_) => GlyphAreaCommandType::NudgeUp,
            GlyphAreaCommand::MoveTo(_, _) => GlyphAreaCommandType::MoveTo,
            GlyphAreaCommand::GrowFont(_) => GlyphAreaCommandType::GrowFont,
            GlyphAreaCommand::ShrinkFont(_) => GlyphAreaCommandType::ShrinkFont,
            GlyphAreaCommand::SetFontSize(_) => GlyphAreaCommandType::SetFontSize,
            GlyphAreaCommand::SetLineHeight(_) => GlyphAreaCommandType::SetLineHeight,
            GlyphAreaCommand::GrowLineHeight(_) => GlyphAreaCommandType::GrowLineHeight,
            GlyphAreaCommand::ShrinkLineHeight(_) => GlyphAreaCommandType::ShrinkLineHeight,
            GlyphAreaCommand::SetBounds(_, _) => GlyphAreaCommandType::SetBounds,
            GlyphAreaCommand::SetRegionFont(_, _) => GlyphAreaCommandType::SetRegionFont,
            GlyphAreaCommand::SetRegionColor(_, _) => GlyphAreaCommandType::SetRegionColor,
            GlyphAreaCommand::DeleteColorFontRegion(_) => GlyphAreaCommandType::DeleteColorFontRegion,
            GlyphAreaCommand::ChangeRegionRange { .. } => GlyphAreaCommandType::ChangeRegionRange,
        }
    }
}

impl Applicable<GlyphArea> for GlyphAreaCommand {
    fn apply_to(&self, target: &mut GlyphArea) {
        match self {
            GlyphAreaCommand::PopFront(pop_count) => target.pop_front(*pop_count),
            GlyphAreaCommand::PopBack(pop_count) => target.pop_back(*pop_count),
            GlyphAreaCommand::MoveTo(x, y) => {
                target.set_position((*x, *y));
            }
            GlyphAreaCommand::NudgeLeft(value) => {
                target.nudge_left(*value);
            }
            GlyphAreaCommand::NudgeRight(value) => {
                target.nudge_right(*value);
            }
            GlyphAreaCommand::NudgeDown(value) => {
                target.nudge_down(*value);
            }
            GlyphAreaCommand::NudgeUp(value) => {
                target.nudge_up(*value);
            }
            GlyphAreaCommand::GrowFont(value) => {
                target.grow_font(value);
            }
            GlyphAreaCommand::ShrinkFont(value) => {
                target.shrink_font(value);
            }
            GlyphAreaCommand::SetBounds(x, y) => {
                target.set_bounds((*x, *y));
            }
            GlyphAreaCommand::DeleteColorFontRegion(range) => {
                target.delete_color_font_region(range);
            }
            GlyphAreaCommand::ChangeRegionRange(current_range, new_range) => {
                target.change_region_range(current_range, new_range);
            }
            GlyphAreaCommand::SetRegionFont(range, font) => {
                target.set_region_font(range, font);
            }
            GlyphAreaCommand::SetRegionColor(range, color) => {
                target.set_region_color(range, color);
            }
            GlyphAreaCommand::SetFontSize(font_size) => {
                target.set_font_size(font_size);
            }
            GlyphAreaCommand::SetLineHeight(line_height) => {
                target.set_line_height(line_height);
            }
            GlyphAreaCommand::GrowLineHeight(line_height) => {
                target.grow_line_height(line_height);
            }
            GlyphAreaCommand::ShrinkLineHeight(line_height) => {
                target.shrink_line_height(line_height);
            }
        }
    }
}

////////////////////////////////////////
/////// DeltaGlyphArea Mutator ////////
///////////////////////////////////////

/// A field-set delta targeting a [`GlyphArea`]. The map keys guarantee
/// at most one entry per field type; the value carries the new payload
/// and (via the co-located `Operation` entry) the arithmetic semantics
/// to apply. Applied via [`Applicable::apply_to`], which dispatches
/// into [`GlyphArea::apply_operation`].
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct DeltaGlyphArea {
    /// One entry per touched field type. `GlyphAreaFieldType::ApplyOperation`
    /// is a sibling entry that carries the global `Add`/`Assign`/`Subtract`
    /// mode for the rest of the delta.
    pub fields: FxHashMap<GlyphAreaFieldType, GlyphAreaField>,
}

impl Applicable<GlyphArea> for DeltaGlyphArea {
    fn apply_to(&self, target: &mut GlyphArea) {
        target.apply_operation(&self)
    }
}

impl Add for DeltaGlyphArea {
    type Output = DeltaGlyphArea;

    fn add(self, rhs: Self) -> Self::Output {
        let mut fields = FxHashMap::default();
        for (key, value) in self.fields {
            if let Some(other_value) = rhs.fields.get(&key) {
                // If both sides have the same field, add them together
                fields.insert(key, value + other_value.clone());
            } else {
                // If only one side has the field, just copy it over
                fields.insert(key, value);
            }
        }
        // Copy over any fields that are only in the rhs
        for (key, value) in rhs.fields {
            if !fields.contains_key(&key) {
                fields.insert(key, value);
            }
        }
        DeltaGlyphArea { fields }
    }
}

impl DeltaGlyphArea {
    /// Collect a vector of fields into a delta, keyed by their
    /// `GlyphAreaFieldType`. Duplicate variants collapse to the last
    /// entry in `fields` (FxHashMap semantics). O(n) over `fields`.
    pub fn new(fields: Vec<GlyphAreaField>) -> DeltaGlyphArea {
        let mut field_map = FxHashMap::default();
        for field in fields {
            field_map.insert(field.variant(), field);
        }

        DeltaGlyphArea { fields: field_map }
    }

    /// Build a full-coverage `Assign` delta that mirrors every
    /// per-glyph field of `area` the in-place mutator path needs to
    /// re-stamp on a tree leaf. Emits `Text`, `position`, `bounds`,
    /// `scale`, `line_height`, `ColorFontRegions`, `Outline`, and
    /// `ZoomVisibility` (the latter required per `lib/baumhard/CONVENTIONS.md`
    /// §B2 — without it a mutator rebuild silently resets each
    /// element's authored zoom window to `Default`), with
    /// `ApplyOperation::Assign` as the global mode.
    ///
    /// Single source of truth for the per-leaf delta shape every
    /// `tree_builder/*::build_*_mutator_tree` function needs; lifting
    /// it to baumhard means any consumer (border, connection,
    /// connection_label, edge_handle, portal — and any future
    /// renderable element type) shares one definition of "what fields
    /// need to be re-asserted to keep the leaf in sync with its
    /// source area." Adding a new per-leaf field becomes a one-line
    /// change here, fanning out to every consumer.
    ///
    /// Cost: clones the area's text, regions, and outline; one
    /// 9-entry `FxHashMap`. No font-system access, no shaping.
    pub fn full_assign_from(area: &GlyphArea) -> DeltaGlyphArea {
        DeltaGlyphArea::new(vec![
            GlyphAreaField::Text(area.text.clone()),
            GlyphAreaField::position(area.position.x.0, area.position.y.0),
            GlyphAreaField::bounds(area.render_bounds.x.0, area.render_bounds.y.0),
            GlyphAreaField::scale(area.scale.0),
            GlyphAreaField::line_height(area.line_height.0),
            GlyphAreaField::ColorFontRegions(area.regions.clone()),
            GlyphAreaField::Outline(area.outline),
            GlyphAreaField::ZoomVisibility(area.zoom_visibility),
            GlyphAreaField::Operation(ApplyOperation::Assign),
        ])
    }

    /// The global arithmetic mode this delta applies with
    /// (`Assign` / `Add` / `Subtract`), or `Noop` when no
    /// `Operation` entry is present. O(1).
    pub fn operation_variant(&self) -> ApplyOperation {
        if let Some(GlyphAreaField::Operation(operation)) =
            self.fields.get(&GlyphAreaFieldType::ApplyOperation)
        {
            *operation
        } else {
            ApplyOperation::Noop
        }
    }

    /// Return the delta's colour/font region payload, if any. O(1).
    pub fn color_font_regions(&self) -> Option<&ColorFontRegions> {
        if let Some(GlyphAreaField::ColorFontRegions(color_font_regions)) =
            self.fields.get(&GlyphAreaFieldType::ColorFontRegions)
        {
            Some(color_font_regions)
        } else {
            None
        }
    }

    /// Return the delta's position payload as a `Vec2`, if any. O(1).
    pub fn position(&self) -> Option<Vec2> {
        if let Some(GlyphAreaField::Position(x)) = self.fields.get(&GlyphAreaFieldType::Position) {
            Some(x.to_vec2())
        } else {
            None
        }
    }

    /// Return the delta's scale payload, if any. O(1).
    pub fn scale(&self) -> Option<f32> {
        if let Some(GlyphAreaField::Scale(scale)) = self.fields.get(&GlyphAreaFieldType::Scale) {
            Some(scale.0)
        } else {
            None
        }
    }

    /// Return the delta's line-height payload, if any. O(1).
    pub fn line_height(&self) -> Option<f32> {
        if let Some(GlyphAreaField::LineHeight(line_height)) =
            self.fields.get(&GlyphAreaFieldType::LineHeight)
        {
            Some(line_height.0)
        } else {
            None
        }
    }

    /// Borrow the delta's text payload, if any. O(1).
    pub fn text_ref(&self) -> Option<&str> {
        if let Some(GlyphAreaField::Text(text)) = self.fields.get(&GlyphAreaFieldType::Text) {
            Some(text)
        } else {
            None
        }
    }

    /// Return the delta's bounds payload as a `Vec2`, if any. O(1).
    pub fn bounds(&self) -> Option<Vec2> {
        if let Some(GlyphAreaField::Bounds(x)) = self.fields.get(&GlyphAreaFieldType::Bounds) {
            Some(x.to_vec2())
        } else {
            None
        }
    }

    /// Returns the delta's [`OutlineStyle`] payload if one was set
    /// on construction. Outer `Option` distinguishes "no outline
    /// field in this delta" (returns `None`) from "the delta
    /// explicitly clears the outline" (returns `Some(None)`); the
    /// latter is how a mutator removes a previously-set halo.
    pub fn outline(&self) -> Option<Option<OutlineStyle>> {
        if let Some(GlyphAreaField::Outline(outline)) = self.fields.get(&GlyphAreaFieldType::Outline) {
            Some(*outline)
        } else {
            None
        }
    }

    /// Returns the delta's [`NodeShape`] payload if one was set on
    /// construction. `None` means the delta leaves the area's
    /// `shape` field alone; `Some(shape)` means it replaces (under
    /// Assign/Add) or resets-to-rectangle (under Subtract). O(1).
    pub fn shape(&self) -> Option<NodeShape> {
        if let Some(GlyphAreaField::Shape(shape)) = self.fields.get(&GlyphAreaFieldType::Shape) {
            Some(*shape)
        } else {
            None
        }
    }

    /// Returns the delta's [`ZoomVisibility`] payload if one was
    /// set on construction. `None` means the delta leaves the
    /// area's `zoom_visibility` alone; `Some(window)` means it
    /// replaces (under Assign/Add) or resets-to-unbounded (under
    /// Subtract). O(1).
    pub fn zoom_visibility(&self) -> Option<ZoomVisibility> {
        if let Some(GlyphAreaField::ZoomVisibility(window)) =
            self.fields.get(&GlyphAreaFieldType::ZoomVisibility)
        {
            Some(*window)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::primitives::ColorFontRegions;
    use crate::gfx_structs::area::GlyphArea;
    use crate::gfx_structs::shape::NodeShape;
    use crate::gfx_structs::zoom_visibility::ZoomVisibility;
    use glam::Vec2;

    /// `full_assign_from` emits exactly the nine fields the
    /// in-place mutator path needs (Text / position / bounds /
    /// scale / line_height / regions / Outline / ZoomVisibility /
    /// Operation(Assign)). Locks the contract against a future
    /// silent omission of any per-leaf field — `lib/baumhard/CONVENTIONS.md
    /// §B2`.
    #[test]
    fn full_assign_from_emits_all_nine_fields() {
        let area = GlyphArea::new_with_str("test", 12.0, 12.0, Vec2::new(1.0, 2.0), Vec2::new(100.0, 50.0));
        let delta = DeltaGlyphArea::full_assign_from(&area);
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::Text));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::Position));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::Bounds));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::Scale));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::LineHeight));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::ColorFontRegions));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::Outline));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::ZoomVisibility));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::ApplyOperation));
        assert_eq!(delta.operation_variant(), ApplyOperation::Assign);
    }

    /// The fields the helper emits round-trip through the
    /// `apply_to` path: applying the delta to a fresh `GlyphArea`
    /// reproduces the source's per-leaf state.
    #[test]
    fn full_assign_from_round_trips_through_apply_to() {
        use crate::core::primitives::Applicable;

        let mut source = GlyphArea::new_with_str(
            "round-trip",
            14.0,
            16.0,
            Vec2::new(7.0, 11.0),
            Vec2::new(200.0, 80.0),
        );
        source.zoom_visibility = ZoomVisibility::try_new(Some(0.5), Some(2.0)).unwrap();
        source.regions = ColorFontRegions::single_span(
            crate::util::grapheme_chad::count_grapheme_clusters("round-trip"),
            Some([1.0, 0.5, 0.25, 1.0]),
            None,
        );

        let delta = DeltaGlyphArea::full_assign_from(&source);
        let mut target = GlyphArea::new(0.0, 0.0, Vec2::ZERO, Vec2::ZERO);
        // Pre-condition: target shape differs from source shape so
        // any field that fails to overwrite would still be visible
        // in the post-state.
        target.shape = NodeShape::Ellipse;
        delta.apply_to(&mut target);

        assert_eq!(target.text, source.text);
        assert_eq!(target.scale, source.scale);
        assert_eq!(target.line_height, source.line_height);
        assert_eq!(target.position, source.position);
        assert_eq!(target.render_bounds, source.render_bounds);
        assert_eq!(target.regions, source.regions);
        assert_eq!(target.outline, source.outline);
        assert_eq!(target.zoom_visibility, source.zoom_visibility);
        // `Shape` is intentionally NOT in the full-assign field set —
        // it's policy, not per-leaf identity. Verify it stayed at
        // the pre-apply value (Ellipse) rather than being reset.
        assert_eq!(target.shape, NodeShape::Ellipse);
    }

    /// Authored zoom-visibility windows survive the round-trip.
    /// Locks the §B2 latent-bug fix that made `edge_handle.rs`'s
    /// mutator stop omitting `ZoomVisibility` from its delta.
    #[test]
    fn full_assign_from_preserves_authored_zoom_window() {
        use crate::core::primitives::Applicable;

        let mut source = GlyphArea::new(12.0, 12.0, Vec2::ZERO, Vec2::new(10.0, 10.0));
        source.zoom_visibility = ZoomVisibility::try_new(Some(1.5), Some(3.0)).unwrap();

        let delta = DeltaGlyphArea::full_assign_from(&source);
        let mut target = GlyphArea::new(0.0, 0.0, Vec2::ZERO, Vec2::ZERO);
        // Target starts with unbounded zoom — if the helper omits
        // ZoomVisibility from the delta, this assert fails.
        assert!(target.zoom_visibility.is_default());
        delta.apply_to(&mut target);
        assert_eq!(target.zoom_visibility, source.zoom_visibility);
    }
}
