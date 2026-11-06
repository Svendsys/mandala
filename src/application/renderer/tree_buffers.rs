// SPDX-License-Identifier: MPL-2.0

//! Tree-to-buffer pipeline — walks a Baumhard `Tree`, shapes cosmic-text
//! buffers for each `GlyphArea`, and stores them into the `Renderer`'s
//! per-role buffer maps. Nothing here collects a fill: a map draws no
//! rect (V1-B12).

use baumhard::font::fonts;
use baumhard::gfx_structs::tree::GfxTree;
use glam::Vec2;

use super::scene_shape_cache::{self, ScenePassKind};
use super::tree_walker::{shape_one_element_into_buffers, walk_tree_into_buffers};
use super::Renderer;

impl Renderer {
    /// Rebuild text buffers from a Baumhard tree (nodes rendered from GlyphArea
    /// elements). This is the primary text-rendering path; borders and
    /// connections use their own `rebuild_*_buffers` methods alongside it.
    pub fn rebuild_buffers_from_tree(&mut self, tree: &GfxTree) {
        self.mindmap_buffers.clear();
        let mut font_system = fonts::acquire_font_system_write("rebuild_buffers_from_tree");
        walk_tree_into_buffers(tree, Vec2::ZERO, &mut font_system, |unique_id, buffer| {
            // The walker emits multiple buffers per element
            // when an outline halo is configured (one buffer
            // per halo offset, then the main glyph). Push onto
            // the entry's `Vec` so all of them survive in
            // emission order — `insert`-style replace would
            // collapse halos onto the main glyph silently.
            self.mindmap_buffers.entry(unique_id).or_default().push(buffer);
        });
    }

    /// Re-shape one element's text buffers in place, keyed by the
    /// element's arena `NodeId` and `unique_id`. Used by the per-
    /// keystroke text-edit path on a multi-section node — pre-fix
    /// the path called [`Self::rebuild_buffers_from_tree`] which
    /// walked the entire arena and re-shaped every section across
    /// every node, an O(N×sections) cost paid on every keypress.
    /// The keyed reshape drops this to O(halos+1) buffers per
    /// element.
    ///
    /// Drops every existing buffer keyed by `unique_id` (the main
    /// glyph plus any halos) before re-shaping; the re-shape pass
    /// writes the new buffers back into the same `Vec` entry.
    ///
    /// Silent no-op when `arena_id` doesn't resolve (e.g. the tree
    /// was rebuilt between the caller's lookup and this call). The
    /// caller is expected to fall back to a full
    /// `rebuild_buffers_from_tree` if it cannot guarantee the
    /// arena id is fresh.
    pub fn reshape_buffer_for(&mut self, arena_id: baumhard::arena::NodeId, tree: &GfxTree) {
        let Some(element) = tree.arena().get(arena_id).map(|n| n.get()) else {
            return;
        };
        let unique_id = element.unique_id();

        // Drop the existing buffers authored by this element so the
        // re-shape pass can write fresh ones.
        self.mindmap_buffers.remove(&unique_id);

        let mut font_system = fonts::acquire_font_system_write("reshape_buffer_for");
        shape_one_element_into_buffers(element, Vec2::ZERO, &mut font_system, &mut |uid, buffer| {
            self.mindmap_buffers.entry(uid).or_default().push(buffer);
        });
    }

    /// Patch the canvas-space position of moved nodes' buffers in
    /// place. Avoids reshaping text when only position changed (the
    /// common case during a drag).
    ///
    /// For each `(unique_id, new_pos)` pair, looks up the existing
    /// buffer entry by key and overwrites every buffer's `pos`
    /// field. Buffers for nodes not in the patch set are left
    /// untouched; their shaped text and position remain valid.
    ///
    /// **Halos.** When an element has an outline halo, the walker
    /// emits one buffer per halo offset (positioned at
    /// `area.position + (dx, dy)` per `OutlineStyle::offsets()`)
    /// then the main glyph, and each of them records the `(dx, dy)`
    /// it was stamped at in
    /// [`MindMapTextBuffer::emission_offset`](super::MindMapTextBuffer#structfield.emission_offset).
    /// The patch re-derives `pos = new_pos + emission_offset`, so a
    /// dragged element's halos keep their ring around the main
    /// glyph. Writing `new_pos` flat — which this path used to do —
    /// collapsed every halo onto the main glyph; that was latent
    /// only because no mindmap-tree element sets `area.outline`
    /// today, and a latent landmine is still the finding (§5).
    ///
    /// # Costs
    ///
    /// O(patch_set_size × halos+1) — no text shaping, no font-system
    /// lock, no allocation. Halos count is 0 for mindmap nodes
    /// today, so the constant collapses on the common path.
    // Native-driver-only: a drag-preview fast path driven by the
    // native drag state machine.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn patch_drag_positions(&mut self, patches: &[(usize, (f32, f32))]) {
        for &(unique_id, new_pos) in patches {
            if let Some(bufs) = self.mindmap_buffers.get_mut(&unique_id) {
                // Canvas-space: the mindmap tree is walked at a zero
                // offset, so the patch re-derives at one too.
                for buf in bufs.iter_mut() {
                    buf.place_at(new_pos, Vec2::ZERO);
                }
            }
        }
    }

    /// Refresh the screen-space shaped output for every tree the app
    /// has registered into [`crate::application::scene_host::AppScene`]'s
    /// overlay sub-scene. Walks the scene in layer order and produces
    /// one entry per walked element in draw order; callers do not need
    /// to know about individual overlays.
    ///
    /// **Only elements whose shaping inputs changed are re-shaped**;
    /// an element whose position alone changed has its buffers moved
    /// in place, halo stamps included.
    /// Every overlay mutator apply routes here — picker hover, picker
    /// HSV drag, console keystroke, console scrollback — and each of
    /// those writes every slot's fields whether or not the value
    /// moved, so before the reuse check the pass re-shaped the
    /// picker's ~58 outlined cells (9 buffers each) or the console's
    /// ~50 rows on every event. See [`super::scene_shape_cache`] for
    /// the reuse rule and why it needs no cooperation from the
    /// writers.
    ///
    /// # Costs
    ///
    /// [`scene_shape_cache::refresh`]'s, over the overlay sub-scene.
    pub fn rebuild_overlay_scene_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        let ids = app_scene.overlay_ids_in_layer_order();
        scene_shape_cache::refresh(
            app_scene.overlay_scene(),
            &ids,
            &mut self.overlay_scene_elements,
            ScenePassKind::Overlay,
        );
    }

    /// Refresh the canvas-space shaped output for every tree the app
    /// has registered into
    /// [`crate::application::scene_host::AppScene`]'s canvas
    /// sub-scene — borders, connections, portals, edge handles,
    /// connection labels, section frames and the two resize-handle
    /// roles. These buffers feed the camera-transformed main pass
    /// alongside the mindmap's own.
    ///
    /// **Only elements whose shaping inputs changed are re-shaped**,
    /// by the same rule and the same body the overlay pass above
    /// uses. That is what makes the ten
    /// `flush_canvas_scene_buffers` call sites cost what they
    /// changed rather than what the canvas holds: each of them
    /// re-projects the roles its interaction can move and then
    /// flushes, and a role no `update_*` touched presents the walk
    /// with the elements it already shaped. An edge-label drag
    /// re-projects one role of the eight; a camera move re-projects
    /// four.
    ///
    /// # Costs
    ///
    /// [`scene_shape_cache::refresh`]'s, over the canvas sub-scene.
    pub fn rebuild_canvas_scene_buffers(&mut self, app_scene: &mut crate::application::scene_host::AppScene) {
        let ids = app_scene.canvas_ids_in_layer_order();
        scene_shape_cache::refresh(
            app_scene.canvas_scene(),
            &ids,
            &mut self.canvas_scene_elements,
            ScenePassKind::Canvas,
        );
    }

    /// Bring the capture canvas in step with the capture engine's
    /// `scratch` scene and compose the frame it will draw: refresh the
    /// scratch set from the scratch scene's trees by the rule
    /// [`Self::rebuild_canvas_scene_buffers`] uses for the live one,
    /// then lay out the segments that interleave it with `live`'s
    /// zoom-independent roles
    /// ([`scene_shape_cache::compose_capture_canvas`]). The next frame
    /// prepared with `CanvasSource::Composed` draws exactly that.
    ///
    /// Writes nothing the window reads: `canvas_scene_elements` is
    /// borrowed, not touched, and the live scene only answers which
    /// tree each of its roles holds.
    ///
    /// # Costs
    ///
    /// [`scene_shape_cache::refresh`]'s over the scratch scene — which
    /// across the frames of a recording at one zoom is the walk and the
    /// comparison, since the scratch set keeps its shaped output like
    /// the live one does — plus the composition's eight span scans.
    // Native-only caller: the capture engine.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn rebuild_capture_canvas_buffers(
        &mut self,
        live: &crate::application::scene_host::AppScene,
        scratch: &mut crate::application::scene_host::AppScene,
    ) {
        let ids = scratch.canvas_ids_in_layer_order();
        scene_shape_cache::refresh(
            scratch.canvas_scene(),
            &ids,
            &mut self.capture_canvas.shaped,
            ScenePassKind::Canvas,
        );
        scene_shape_cache::compose_capture_canvas(
            live,
            scratch,
            &self.canvas_scene_elements,
            &mut self.capture_canvas,
        );
    }
}
