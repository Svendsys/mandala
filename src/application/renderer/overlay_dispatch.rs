// SPDX-License-Identifier: MPL-2.0

//! Overlay rebuild / mutator dispatchers — the four `Renderer` entry
//! points that bridge the app's `AppScene` overlay slots (console,
//! color picker) to the renderer's buffer pipeline.
//!
//! Each method either builds a fresh overlay tree (full rebuild) or
//! applies a `MutatorTree` delta in place (§B2 mutator path), then
//! calls `rebuild_overlay_scene_buffers` to refresh the shaped
//! buffers. The choice is dispatched by structural-signature
//! equality — see [`crate::application::scene_host::OverlayDispatch`].
//!
//! The shaped buffers are then brought up to date with the overlay
//! trees ([`Renderer::rebuild_overlay_scene_buffers`]) **area by area**:
//! each area keeps the buffers it was shaped into under a reuse key of
//! everything shaping reads, so a console keystroke reshapes the prompt
//! line and not the scrollback, and a picker hover the cells whose
//! content changed. That is not a scan for change in the document's
//! sense: it is one key comparison per area of an overlay that is
//! being rebuilt anyway.

use baumhard::font::fonts;

use super::color_picker;
use super::console_geometry::{compute_console_frame_layout, ConsoleOverlayGeometry};
use super::console_pass::{
    build_console_overlay_mutator, build_console_overlay_tree, console_overlay_signature,
};
use super::{MindMapTextBuffer, Renderer};
use baumhard::arena::NodeId;
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::scene::{Scene, SceneTreeId};
use rustc_hash::FxHashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;

impl Renderer {
    /// Bring the chrome pass's buffers up to date with every visible
    /// overlay tree — the console's, the color picker's — in layer order
    /// and each tree in pre-order, so later areas draw over earlier ones
    /// ([`reshape_overlays`]).
    ///
    /// # Costs
    ///
    /// One walk of the overlay trees and one reuse-key comparison per
    /// area; a shaping only for an area whose key changed, under one
    /// `FONT_SYSTEM` write guard taken at the first such area.
    pub fn rebuild_overlay_scene_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        let ids = app_scene.overlay_ids_in_layer_order();
        reshape_overlays(&mut self.overlay_shapes, app_scene.overlay_scene(), &ids);
    }

    /// Rebuild the console overlay buffers. When `geometry` is
    /// `None`, the console is closed — clear the buffer list and the
    /// backdrop, and return. When `Some`, lay out a bottom-anchored
    /// glyph-rendered strip: sacred border, scrollback region,
    /// optional completion popup, and the prompt line with cursor.
    ///
    /// Everything is positioned in screen coordinates (the walk
    /// lands in `OverlayShapes::elements`, which the render pass
    /// draws with `scale = 1.0`), so the console stays a fixed size
    /// regardless of canvas zoom.
    pub fn rebuild_console_overlay_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry: Option<&ConsoleOverlayGeometry>,
    ) {
        use crate::application::scene_host::{OverlayDispatch, OverlayRole};

        let Some(geometry) = geometry else {
            // Closed: drop the backdrop, drop the tree, refresh
            // overlay buffers so the console disappears. The
            // structural-signature cache lives on `AppScene` and
            // is cleared inside `unregister_overlay`, so the next
            // reopen starts from a clean slate.
            self.console_backdrop = None;
            app_scene.unregister_overlay(OverlayRole::Console);
            self.rebuild_overlay_scene_buffers(app_scene);
            return;
        };

        let layout =
            compute_console_frame_layout(geometry, self.surface_size.0 as f32, self.surface_size.1 as f32);
        self.console_backdrop = Some(layout.backdrop_rect());
        self.ensure_backdrop_cell();
        let signature = console_overlay_signature(&layout);

        // §B2 dispatch: if the structural signature
        // (`scrollback_rows` × `completion_rows`) hasn't changed
        // since the last build, the existing tree's slot count
        // still matches and we apply an in-place mutator that
        // overwrites every slot's variable fields. Window resize
        // is the only typical event that shifts the signature, so
        // the mutator path covers every keystroke / scrollback-
        // grow / completion-update / Tab-cycle frame.
        match app_scene.overlay_dispatch(OverlayRole::Console, signature) {
            OverlayDispatch::InPlaceMutator => {
                let mutator = {
                    let mut font_system = fonts::acquire_font_system_write("overlay_dispatch");
                    build_console_overlay_mutator(geometry, &layout, &mut font_system)
                };
                app_scene.apply_overlay_mutator(OverlayRole::Console, &mutator);
            }
            OverlayDispatch::FullRebuild => {
                // Build the tree under the FONT_SYSTEM lock — we
                // need it for `measure_max_glyph_advance` only.
                // Tree construction itself doesn't shape; that
                // happens during the overlay-scene walk below.
                let tree = {
                    let mut font_system = fonts::acquire_font_system_write("overlay_dispatch");
                    build_console_overlay_tree(geometry, &layout, &mut font_system)
                };
                app_scene.register_overlay(OverlayRole::Console, tree, glam::Vec2::ZERO);
                app_scene.set_overlay_signature(OverlayRole::Console, signature);
            }
        }
        self.rebuild_overlay_scene_buffers(app_scene);
    }

    /// Build the picker's overlay tree from `geometry`, register
    /// it under [`OverlayRole::ColorPicker`](crate::application::scene_host::OverlayRole),
    /// and walk the overlay sub-scene into
    /// `OverlayShapes::elements`. `None` means the picker is closed
    /// — drops the backdrop, unregisters the tree, refreshes
    /// overlay buffers so it disappears.
    ///
    /// Called by `open_color_picker`, the `Resized` handler, and
    /// the hover / chip-focus / commit / cancel paths in
    /// `app::rebuild_color_picker_overlay`.
    ///
    /// Open and close are the two events that rebuild the arena. The
    /// hover / HSV / chip paths do not come through here — they mutate
    /// the registered arena in place; see
    /// [`Self::apply_color_picker_overlay_dynamic_mutator`].
    pub fn rebuild_color_picker_overlay_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry_and_layout: Option<(
            &crate::application::color_picker::ColorPickerOverlayGeometry,
            &crate::application::color_picker::ColorPickerLayout,
        )>,
    ) {
        self.color_picker_backdrop =
            color_picker::prepare_overlay_for_rebuild(app_scene, geometry_and_layout);
        if self.color_picker_backdrop.is_some() {
            self.ensure_backdrop_cell();
        }
        self.rebuild_overlay_scene_buffers(app_scene);
    }

    /// §B2 mutation path — apply the **layout-phase** delta to the
    /// picker overlay tree without rebuilding the arena. Pairs with
    /// [`crate::application::color_picker_overlay::build_mutator`]:
    /// every variable field on every picker GlyphArea is overwritten
    /// via an `Assign` `DeltaGlyphArea` keyed by stable channel.
    ///
    /// Use this only when something the layout depends on actually
    /// changed (viewport resize, RMB size_scale drag, drag-move
    /// repositioning the wheel). Per-frame hover/HSV/chip updates
    /// should call [`Self::apply_color_picker_overlay_dynamic_mutator`]
    /// instead — same arena, slimmer per-cell delta. Open / close
    /// still use [`Self::rebuild_color_picker_overlay_buffers`]
    /// because the arena needs to be created or torn down. Calls
    /// `rebuild_overlay_scene_buffers` afterward, which shapes the
    /// overlay again.
    pub fn apply_color_picker_overlay_mutator(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
        layout: &crate::application::color_picker::ColorPickerLayout,
    ) {
        color_picker::apply_layout_mutator(app_scene, geometry, layout);
        self.rebuild_overlay_scene_buffers(app_scene);
    }

    /// §B2 mutation path — apply the **dynamic-phase** delta to the
    /// picker overlay tree. Pairs with
    /// [`crate::application::color_picker_overlay::build_dynamic_mutator`]:
    /// only the per-frame fields (color regions, hover scale, hex
    /// text) are written; layout-phase fields stay as the previous
    /// layout-mutator wrote them.
    ///
    /// This is the per-frame hot path for hover / HSV / chip-focus
    /// updates — the picker's element set, position, and bounds are
    /// unchanged. Calls `rebuild_overlay_scene_buffers` afterward,
    /// which shapes the overlay again.
    pub fn apply_color_picker_overlay_dynamic_mutator(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
        layout: &crate::application::color_picker::ColorPickerLayout,
    ) {
        color_picker::apply_dynamic_mutator(app_scene, geometry, layout);
        self.rebuild_overlay_scene_buffers(app_scene);
    }
}

/// The chrome pass's shaped overlay buffers, and what each area was
/// shaped from: the reuse that keeps a keystroke from reshaping the
/// whole console ([`reshape_overlays`]).
#[derive(Default)]
pub(crate) struct OverlayShapes {
    /// Every buffer, in draw order: what the chrome pass draws.
    pub(crate) elements: Vec<MindMapTextBuffer>,
    /// Per area — its overlay tree and its element — its reuse key and
    /// the run of [`Self::elements`] its stamps are.
    index: FxHashMap<(SceneTreeId, NodeId), (u64, Range<usize>)>,
    /// How many areas have been shaped, over the life of the store.
    /// Test builds only.
    #[cfg(test)]
    pub(crate) shaped: usize,
}

/// Bring `shapes` up to date with the overlay trees `ids` names in
/// `scene`: an area whose reuse key ([`reuse_key`]) is the one it was
/// shaped under keeps its buffers, moved into place with their
/// positions written anew; every other area is shaped. An area that is
/// gone drops its buffers. Pure over its inputs and the font system, so
/// the tests drive it without a renderer.
///
/// Cost: O(the trees' elements) for the walk and the keys; one shaping
/// per area whose key changed, under one font guard taken at the first.
pub(crate) fn reshape_overlays(shapes: &mut OverlayShapes, scene: &Scene, ids: &[SceneTreeId]) {
    let mut previous: Vec<Option<MindMapTextBuffer>> = std::mem::take(&mut shapes.elements)
        .into_iter()
        .map(Some)
        .collect();
    let old = std::mem::take(&mut shapes.index);
    let mut font_system = None;
    for &id in ids {
        let Some(entry) = scene.get(id) else {
            continue;
        };
        if !entry.visible() {
            continue;
        }
        let (tree, offset) = (entry.tree(), entry.offset());
        for element_id in tree.root().descendants(tree.arena()) {
            let Some(node) = tree.arena().get(element_id) else {
                continue;
            };
            let element = node.get();
            let Some(area) = element.glyph_area() else {
                continue;
            };
            if area.text.is_empty() {
                continue;
            }
            let key = reuse_key(area);
            let start = shapes.elements.len();
            let kept = old.get(&(id, element_id)).filter(|(was, _)| *was == key);
            match kept {
                Some((_, range)) => {
                    let anchor = (area.position.x.0, area.position.y.0);
                    for (slot, stamp) in range.clone().zip(area.text_stamp_offsets()) {
                        let Some(mut buffer) = previous[slot].take() else {
                            continue;
                        };
                        buffer.pos = MindMapTextBuffer::stamp_pos(anchor, stamp, offset);
                        shapes.elements.push(buffer);
                    }
                }
                None => {
                    #[cfg(test)]
                    {
                        shapes.shaped += 1;
                    }
                    let fs = font_system.get_or_insert_with(|| {
                        fonts::acquire_font_system_write("overlay_dispatch::reshape_overlays")
                    });
                    let elements = &mut shapes.elements;
                    super::tree_walker::shape_one_element_into_buffers(
                        element,
                        offset,
                        fs,
                        &mut |_, buffer| elements.push(buffer),
                    );
                }
            }
            shapes
                .index
                .insert((id, element_id), (key, start..shapes.elements.len()));
        }
    }
}

/// Everything shaping an area reads, hashed: its text and regions, its
/// scale, line height and bounds, its alignment and its halo (its
/// color and width, through `OutlineStyle`'s own `Hash`). Its
/// position is not in it — a moved area keeps its buffers, placed at
/// [`GlyphArea::text_stamp_offsets`], the order
/// [`super::tree_walker::shape_one_element_into_buffers`] yields them
/// in. `test_every_input_shaping_reads_reshapes_its_area_and_only_it`
/// holds the key to every input, one row each. No allocation.
fn reuse_key(area: &GlyphArea) -> u64 {
    let mut hasher = rustc_hash::FxHasher::default();
    area.text.hash(&mut hasher);
    // Every range, font pin and color: the derived `Hash` is range
    // identity alone, so a color-only change would keep stale buffers.
    area.regions.hash_content(&mut hasher);
    area.scale.0.to_bits().hash(&mut hasher);
    area.line_height.0.to_bits().hash(&mut hasher);
    area.render_bounds.x.0.to_bits().hash(&mut hasher);
    area.render_bounds.y.0.to_bits().hash(&mut hasher);
    area.align_center.hash(&mut hasher);
    area.halo().hash(&mut hasher);
    hasher.finish()
}
