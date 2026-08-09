// SPDX-License-Identifier: MPL-2.0

//! Tree-to-buffer pipeline — walks a Baumhard `Tree`, shapes cosmic-text
//! buffers for each `GlyphArea`, and stores them (with optional background
//! rects) into the `Renderer`'s per-role buffer maps.

use baumhard::font::fonts;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use glam::Vec2;

use super::overlay_shape_cache::ShapedOverlayElement;
use super::tree_walker::{shape_one_element_into_buffers, walk_tree_into_buffers};
use super::Renderer;

impl Renderer {
    /// Rebuild text buffers from a Baumhard tree (nodes rendered from GlyphArea
    /// elements). This is the primary text-rendering path; borders and
    /// connections use their own `rebuild_*_buffers` methods alongside it.
    pub fn rebuild_buffers_from_tree(&mut self, tree: &Tree<GfxElement, GfxMutator>) {
        self.mindmap_buffers.clear();
        // Node backgrounds live on GlyphArea and are collected
        // fresh alongside the text buffers. The render pipeline
        // reads them back out each frame to draw solid fills behind
        // the text, with the camera transform baked in at the last
        // moment. Clearing here (rather than on every render call)
        // keeps the collect cost aligned with the tree rebuild
        // cadence — i.e. only when something structural changed.
        self.node_background_rects.clear();
        let mut font_system = fonts::acquire_font_system_write("rebuild_buffers_from_tree");
        walk_tree_into_buffers(
            tree,
            Vec2::ZERO,
            &mut font_system,
            |unique_id, buffer| {
                // The walker emits multiple buffers per element
                // when an outline halo is configured (one buffer
                // per halo offset, then the main glyph). Push onto
                // the entry's `Vec` so all of them survive in
                // emission order — `insert`-style replace would
                // collapse halos onto the main glyph silently.
                self.mindmap_buffers.entry(unique_id).or_default().push(buffer);
            },
            |rect| self.node_background_rects.push(rect),
        );
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
    /// Background rects authored by this same `unique_id` are
    /// removed from `node_background_rects` and re-collected from
    /// the fresh element so a section whose background colour just
    /// changed reflects the new fill *without* leaking duplicate
    /// stale rects per keystroke. Other elements' rects are
    /// untouched — the filter compares `NodeBackgroundRect.unique_id`
    /// directly.
    ///
    /// Silent no-op when `arena_id` doesn't resolve (e.g. the tree
    /// was rebuilt between the caller's lookup and this call). The
    /// caller is expected to fall back to a full
    /// `rebuild_buffers_from_tree` if it cannot guarantee the
    /// arena id is fresh.
    pub fn reshape_buffer_for(
        &mut self,
        arena_id: indextree::NodeId,
        tree: &Tree<GfxElement, GfxMutator>,
    ) {
        let Some(element) = tree.arena.get(arena_id).map(|n| n.get()) else {
            return;
        };
        let unique_id = element.unique_id();

        // Drop the existing buffers + background entries authored
        // by this element so the re-shape pass can write fresh
        // ones. Background rects use direct `unique_id` equality
        // so other elements' rects survive the filter.
        self.mindmap_buffers.remove(&unique_id);
        self.node_background_rects.retain(|rect| rect.unique_id != unique_id);

        let mut font_system = fonts::acquire_font_system_write("reshape_buffer_for");
        shape_one_element_into_buffers(
            element,
            Vec2::ZERO,
            &mut font_system,
            &mut |uid, buffer| {
                self.mindmap_buffers.entry(uid).or_default().push(buffer);
            },
            &mut |rect| self.node_background_rects.push(rect),
        );
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
    // Native-driver-only, with `rebuild_node_backgrounds_from_tree`:
    // both are drag-preview fast paths driven by the native drag
    // state machine.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn patch_drag_positions(&mut self, patches: &[(usize, (f32, f32))]) {
        for &(unique_id, new_pos) in patches {
            if let Some(bufs) = self.mindmap_buffers.get_mut(&unique_id) {
                for buf in bufs.iter_mut() {
                    buf.pos = (
                        new_pos.0 + buf.emission_offset.0,
                        new_pos.1 + buf.emission_offset.1,
                    );
                }
            }
        }
    }

    /// Rebuild only the `node_background_rects` from a tree, without
    /// reshaping any text buffers. Used during drag to keep background
    /// fills in sync with moved node positions.
    ///
    /// # Costs
    ///
    /// O(n) descendant walk, but no text shaping, no font-system
    /// lock — just position and color reads from the arena.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn rebuild_node_backgrounds_from_tree(&mut self, tree: &Tree<GfxElement, GfxMutator>) {
        self.node_background_rects.clear();
        // Single source of background-rect math: routes through the
        // walker's `extract_background_rect` so the drag-fast-path
        // and the full text-shaping pass can't drift on padding /
        // shape-id / zoom-visibility resolution.
        for descendant_id in tree.root().descendants(&tree.arena) {
            let Some(node) = tree.arena.get(descendant_id) else { continue };
            let element = node.get();
            let Some(area) = element.glyph_area() else { continue };
            if let Some(rect) = super::tree_walker::extract_background_rect(element, area, Vec2::ZERO) {
                self.node_background_rects.push(rect);
            }
        }
    }

    /// Refresh the screen-space buffer list for every tree the app
    /// has registered into [`crate::application::scene_host::AppScene`].
    /// Walks the scene in layer order and produces one list in draw
    /// order; callers do not need to know about individual overlays.
    ///
    /// **Only elements whose shaping inputs changed are re-shaped.**
    /// Each walk position is checked against the
    /// [`ShapedOverlayElement`] that occupied it last pass, and a
    /// match keeps the existing buffers untouched. Every overlay
    /// mutator apply routes here — picker hover, picker HSV drag,
    /// console keystroke, console scrollback — and each of those
    /// writes every slot's fields whether or not the value moved, so
    /// before this check the pass re-shaped the picker's ~58 outlined
    /// cells (9 buffers each) or the console's ~50 rows on every
    /// event. It now shapes the cells whose `GlyphArea` differs from
    /// the one it was last shaped from. See
    /// [`super::overlay_shape_cache`] for the reuse rule and why it
    /// needs no cooperation from the writers.
    ///
    /// # Costs
    ///
    /// O(sum of descendants) across every tree in the scene for the
    /// walk and the equality checks, plus one `cosmic_text::Buffer`
    /// allocation and shaping per *changed* non-empty `GlyphArea`
    /// (times `halos + 1`). The `FONT_SYSTEM` write guard is
    /// acquired lazily on the first element that actually needs
    /// shaping, so an all-hit pass takes no lock at all. Empty
    /// scenes short-circuit cheaply.
    pub fn rebuild_overlay_scene_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        // Taken out of `self` so the per-element shaping closures can
        // borrow it mutably while `self`'s other fields stay
        // reachable; put back unconditionally below.
        let mut shaped = std::mem::take(&mut self.overlay_scene_buffers);
        let ids = app_scene.overlay_ids_in_layer_order();
        if ids.is_empty() {
            shaped.clear();
            self.overlay_scene_buffers = shaped;
            return;
        }
        // Lazily acquired: a pass in which every element still
        // matches its cached inputs never touches the lock.
        let mut font_system: Option<std::sync::RwLockWriteGuard<'static, baumhard::font::FontSystem>> =
            None;
        let mut slot = 0usize;
        for id in ids {
            let Some(entry) = app_scene.overlay_scene().get(id) else {
                continue;
            };
            if !entry.visible() {
                continue;
            }
            let tree = entry.tree();
            let offset = entry.offset();
            for descendant_id in tree.root().descendants(&tree.arena) {
                let Some(element) = tree.arena.get(descendant_id).map(|n| n.get()) else {
                    continue;
                };
                if shaped
                    .get(slot)
                    .is_some_and(|cached| cached.still_matches(id, element, offset))
                {
                    slot += 1;
                    continue;
                }
                let font_system = font_system
                    .get_or_insert_with(|| fonts::acquire_font_system_write("rebuild_overlay_scene_buffers"));
                let mut buffers = Vec::new();
                shape_one_element_into_buffers(
                    element,
                    offset,
                    font_system,
                    &mut |_unique_id, buffer| buffers.push(buffer),
                    &mut |_rect| {
                        // Overlay-tree background fills aren't wired to
                        // a screen-space rect pipeline yet. When a
                        // screen-space overlay actually needs background
                        // fills, add a dedicated
                        // `overlay_scene_background_rects` field and a
                        // screen-space draw pass.
                    },
                );
                let fresh = ShapedOverlayElement::new(id, element, offset, buffers);
                match shaped.get_mut(slot) {
                    Some(existing) => *existing = fresh,
                    None => shaped.push(fresh),
                }
                slot += 1;
            }
        }
        // Anything past the last walked position belongs to an
        // overlay that has since been unregistered or shortened.
        shaped.truncate(slot);
        self.overlay_scene_buffers = shaped;
    }

    /// Rebuild the canvas-space buffer list for every tree the app
    /// has registered into
    /// [`crate::application::scene_host::AppScene`]'s canvas sub-scene
    /// (borders, connections, portals, edge handles, connection
    /// labels — whichever have migrated). These buffers feed the
    /// camera-transformed main pass alongside the mindmap's own
    /// buffer map.
    ///
    /// # Costs
    ///
    /// O(sum of descendants) across every canvas tree. Allocates a
    /// `cosmic_text::Buffer` per non-empty `GlyphArea`. Empty
    /// sub-scenes short-circuit cheaply.
    pub fn rebuild_canvas_scene_buffers(&mut self, app_scene: &mut crate::application::scene_host::AppScene) {
        self.canvas_scene_buffers.clear();
        self.canvas_scene_background_rects.clear();
        let ids = app_scene.canvas_ids_in_layer_order();
        if ids.is_empty() {
            return;
        }
        let mut font_system = fonts::acquire_font_system_write("rebuild_canvas_scene_buffers");
        for id in ids {
            let Some(entry) = app_scene.canvas_scene().get(id) else {
                continue;
            };
            if !entry.visible() {
                continue;
            }
            walk_tree_into_buffers(
                entry.tree(),
                entry.offset(),
                &mut font_system,
                |_unique_id, buffer| {
                    self.canvas_scene_buffers.push(buffer);
                },
                |rect| {
                    self.canvas_scene_background_rects.push(rect);
                },
            );
        }
    }
}
