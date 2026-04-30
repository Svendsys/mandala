// SPDX-License-Identifier: MPL-2.0

//! Single source of truth for the picker's per-section `GlyphArea`
//! set, plus the tree- and mutator-builders that wrap it into the
//! shapes the renderer registers. The initial-build path and the §B2
//! mutator paths cannot drift because they all read from the same
//! [`areas::PickerAreas`] table built by
//! [`compute::compute_picker_areas`].
//!
//! Section names ("title", "hue_ring", "hint", "sat_bar", "val_bar",
//! "preview", "hex") must match the `mutator_spec.sections[*].section`
//! strings in `widgets/color_picker.json` — the spec's channel layout
//! is authoritative.

mod areas;
mod compute;
mod dynamic_context;
mod make_area;
mod sections;
mod trees;

pub(super) use trees::{
    build_color_picker_overlay_dynamic_mutator, build_color_picker_overlay_mutator,
    build_color_picker_overlay_tree,
};

#[cfg(test)]
pub(super) use compute::picker_glyph_areas;
