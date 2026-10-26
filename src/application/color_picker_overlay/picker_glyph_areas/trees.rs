// SPDX-License-Identifier: MPL-2.0

//! Tree- and mutator-tree builders the renderer calls from
//! `apply_color_picker_overlay_*`. The initial-build path produces a
//! freshly-allocated `GfxTree`; the layout and
//! dynamic mutator paths produce in-place `GfxMutatorTree`s
//! that target an already-registered overlay tree.

use baumhard::engine::tree::Placement;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::tree::{GfxMutatorTree, GfxTree};

use super::areas::PickerAreas;
use super::compute::compute_picker_areas;
use super::dynamic_context::PickerDynamicContext;
use crate::application::color_picker::{ColorPickerLayout, ColorPickerOverlayGeometry};
use crate::application::widgets::color_picker_widget::load_spec;
use baumhard::engine::builder::{build, BuildContext, MutatorNode};
use baumhard::gfx_structs::mutator::GfxMutation;
use baumhard::gfx_structs::template::{resolve_from_area, CellField};

/// Build the color-picker overlay tree from a geometry +
/// pre-computed layout. Iterates the
/// [channel-ascending][PickerAreas::ordered] list so the registered
/// tree's channels match what the mutator path will later target.
///
/// **Performance note**: this rebuilds every glyph on every
/// `rebuild_color_picker_overlay_buffers` call, which is reserved
/// for picker open / close and tree-shape changes. Per-frame
/// updates go through [`build_color_picker_overlay_dynamic_mutator`]
/// — same arena, slim per-cell delta.
pub(in crate::application::color_picker_overlay) fn build_color_picker_overlay_tree(
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
) -> GfxTree {
    let areas = compute_picker_areas(geometry, layout);
    let mut tree: GfxTree = GfxTree::new_non_indexed();
    for (channel, area) in areas.ordered {
        let area_element = GfxElement::new_area_non_indexed_with_id(area, channel, channel);
        // Under the root, which is always in the tree: the insert
        // cannot be refused, and a refusal would leave the cell out
        // rather than abort the overlay.
        if let Err(e) = tree.insert_child(tree.root, area_element, Placement::Last) {
            log::error!("color picker: could not add cell {channel}: {e}");
        }
    }
    tree
}

/// Build a [`GfxMutatorTree`] that updates an already-registered picker
/// tree to the current `(geometry, layout)` state without rebuilding
/// the arena. The tree shape is declared in
/// `widgets/color_picker.json`'s `mutator_spec` (the **layout** spec
/// — full per-cell field set); the per-cell `GlyphArea` values come
/// from [`PickerAreas`] via a [`PickerAreasContext`] adapter.
///
/// This is the §B2 "mutation, not rebuild" path for layout-change
/// events: initial open, viewport resize, and RMB size_scale drag.
/// Hover / HSV / chip frames go through
/// [`build_color_picker_overlay_dynamic_mutator`] instead — same
/// channel layout, slimmer per-section field lists.
pub(in crate::application::color_picker_overlay) fn build_color_picker_overlay_mutator(
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
) -> GfxMutatorTree {
    let spec = load_spec();
    let areas = compute_picker_areas(geometry, layout);
    let ctx = PickerAreasContext { areas: &areas };
    build_or_noop(&spec.mutator_spec, &ctx)
}

/// Per-frame [`GfxMutatorTree`] for the picker — the **dynamic** phase.
/// Walked from `widgets/color_picker.json`'s `dynamic_mutator_spec`,
/// which carries the same channel layout as `mutator_spec` but only
/// the per-section `CellField`s that actually change between hover /
/// HSV / drag frames (color, hover scale, hex text). Position,
/// bounds, line_height, and outline come from the layout phase and
/// stay untouched here.
///
/// Uses a [`PickerDynamicContext`] that resolves each template
/// field by field — each dynamic field is
/// computed from `(geometry, section, index)` on demand, so no
/// per-frame `Vec<(usize, GlyphArea)>`, no 60× `GlyphArea::new_with_str`,
/// no full-layout position/bounds math, and no per-cell outline
/// struct. The only clones that remain are the inevitable ones on
/// the mutator tree's `Vec<GlyphAreaField>` and its `Box<DeltaGlyphArea>`.
pub(in crate::application::color_picker_overlay) fn build_color_picker_overlay_dynamic_mutator(
    geometry: &ColorPickerOverlayGeometry,
    layout: &ColorPickerLayout,
) -> GfxMutatorTree {
    let spec = load_spec();
    let ctx = PickerDynamicContext::new(geometry, layout);
    build_or_noop(&spec.dynamic_mutator_spec, &ctx)
}

/// Build `spec` against `ctx`, or — when the spec is one no mutator
/// tree can express — log why and hand back an empty mutator tree,
/// whose walk changes nothing. The specs are the embedded
/// `widgets/color_picker.json`, so a refusal is JSON / Rust drift; the
/// picker is an interactive path and keeps its last frame rather than
/// aborting (CODE_CONVENTIONS §9).
fn build_or_noop<C: BuildContext<GfxElement, GfxMutation>>(
    spec: &MutatorNode<GfxElement, GfxMutation>,
    ctx: &C,
) -> GfxMutatorTree {
    build(spec, ctx).unwrap_or_else(|error| {
        log::error!("color picker: the widget's mutator spec does not build: {error}");
        GfxMutatorTree::new()
    })
}

/// Adapter implementing [`BuildContext`] on top of a precomputed
/// [`PickerAreas`] table: each cell's template resolves against that
/// cell's pre-built area. The picker spec uses no runtime counts,
/// runtime mutations, or macros, so the other trait methods keep their
/// `unreachable!()` defaults.
struct PickerAreasContext<'a> {
    areas: &'a PickerAreas,
}

impl<'a> BuildContext<GfxElement, GfxMutation> for PickerAreasContext<'a> {
    fn resolve(&self, section: &str, index: usize, template: &Vec<CellField>) -> GfxMutation {
        resolve_from_area(self.areas.area(section, index), template)
    }
}
