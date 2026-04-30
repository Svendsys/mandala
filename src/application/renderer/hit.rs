// SPDX-License-Identifier: MPL-2.0

//! Hit-testing + camera-fit methods on `Renderer`. Operate against
//! the cached `*_hitboxes` maps and the camera.

use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use baumhard::mindmap::scene_builder::RenderScene;
use baumhard::mindmap::scene_cache::EdgeKey;
use glam::Vec2;
use rustc_hash::FxHashMap;
use std::hash::Hash;

use super::Renderer;

fn aabb_contains(pos: Vec2, min: Vec2, max: Vec2) -> bool {
    pos.x >= min.x && pos.x <= max.x && pos.y >= min.y && pos.y <= max.y
}

/// First key in `map` whose AABB contains `pos`; linear scan.
fn find_first_aabb_hit<K: Clone + Hash + Eq>(map: &FxHashMap<K, (Vec2, Vec2)>, pos: Vec2) -> Option<K> {
    for (key, (min, max)) in map {
        if aabb_contains(pos, *min, *max) {
            return Some(key.clone());
        }
    }
    None
}

impl Renderer {
    /// Fit the camera to show a RenderScene's content.
    pub fn fit_camera_to_scene(&mut self, scene: &RenderScene) {
        if scene.text_elements.is_empty() {
            return;
        }
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for elem in &scene.text_elements {
            let (x, y) = elem.position;
            let (w, h) = elem.size;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + w);
            max_y = max_y.max(y + h);
        }
        self.camera
            .apply_mutation(&baumhard::gfx_structs::camera::CameraMutation::FitToBounds {
                min: Vec2::new(min_x, min_y),
                max: Vec2::new(max_x, max_y),
                padding_fraction: 0.05,
            });
    }

    /// AABB hit test against the rendered label hitboxes. Returns
    /// true when `canvas_pos` falls inside the hitbox of the given
    /// edge's label. Used by the app to dispatch inline click-to-edit
    /// when a selected edge's label is clicked.
    pub fn hit_test_edge_label(&self, canvas_pos: Vec2, edge_key: &EdgeKey) -> bool {
        self.connection_label_hitboxes
            .get(edge_key)
            .is_some_and(|(min, max)| aabb_contains(canvas_pos, *min, *max))
    }

    /// Scan every registered edge-label hitbox and return the
    /// first owning [`EdgeKey`] whose AABB contains `canvas_pos`.
    /// Sibling of [`Self::hit_test_portal`] / [`Self::hit_test_portal_text`]
    /// but keyed by edge identity alone — edge labels have no
    /// endpoint split. Linear scan; label counts stay proportional
    /// to visible edges, so no spatial index is warranted.
    ///
    /// Used by the click dispatcher to route a label click to
    /// `SelectionState::EdgeLabel` without requiring the edge to
    /// already be selected.
    pub fn hit_test_any_edge_label(&self, canvas_pos: Vec2) -> Option<EdgeKey> {
        find_first_aabb_hit(&self.connection_label_hitboxes, canvas_pos)
    }

    /// Replace the connection-label hitbox map wholesale.
    /// Used by `update_connection_label_tree` once labels render
    /// through the canvas-scene tree path; the tree builder owns
    /// the AABB computation and hands the map over via this
    /// setter so `hit_test_edge_label` keeps working off the
    /// flat-pass hitbox map while label buffers migrate.
    pub fn set_connection_label_hitboxes(
        &mut self,
        hitboxes: std::collections::HashMap<EdgeKey, (Vec2, Vec2)>,
    ) {
        self.connection_label_hitboxes = hitboxes.into_iter().collect();
    }

    /// Replace the portal **icon** hitbox map wholesale. Called
    /// from `update_portal_tree` every time the portal tree is
    /// rebuilt or mutated — the tree builder owns the AABB
    /// computation and hands the map over via this setter so
    /// [`Self::hit_test_portal`] keeps working.
    pub fn set_portal_icon_hitboxes(
        &mut self,
        hitboxes: std::collections::HashMap<(EdgeKey, String), (Vec2, Vec2)>,
    ) {
        self.portal_icon_hitboxes = hitboxes.into_iter().collect();
    }

    /// Replace the portal **text** hitbox map wholesale. Sibling
    /// of [`Self::set_portal_icon_hitboxes`]. Text entries exist
    /// only for endpoints with non-empty text — empty-string
    /// slots register no entry here so text-less portals don't
    /// grow a phantom hot zone (see `tree_builder::portal` for
    /// the invariant).
    pub fn set_portal_text_hitboxes(
        &mut self,
        hitboxes: std::collections::HashMap<(EdgeKey, String), (Vec2, Vec2)>,
    ) {
        self.portal_text_hitboxes = hitboxes.into_iter().collect();
    }

    /// Hit-test portal **icon** markers at `canvas_pos`. Returns
    /// the `(EdgeKey, endpoint_node_id)` of the first icon whose
    /// AABB contains the point, or `None` if no icon is hit. The
    /// endpoint id is the node the hit marker sits above — the
    /// app uses the *other* endpoint as the double-click
    /// navigation target.
    ///
    /// Linear scan — portal counts stay in the dozens so a spatial
    /// index is not worth the maintenance cost. Consulted from
    /// `handle_click` as an alternate selection path, routed in
    /// before the edge hit test so clicks on a marker floating
    /// above a node's top-right corner don't accidentally fall
    /// through to an edge beneath. Pair with
    /// [`Self::hit_test_portal_text`] to distinguish icon clicks
    /// from text clicks — callers that want "any portal sub-part"
    /// check both in sequence.
    pub fn hit_test_portal(&self, canvas_pos: Vec2) -> Option<(EdgeKey, String)> {
        find_first_aabb_hit(&self.portal_icon_hitboxes, canvas_pos)
    }

    /// Hit-test portal **text** labels at `canvas_pos`. Sibling of
    /// [`Self::hit_test_portal`]. Text and icon AABBs don't overlap
    /// in practice (text sits beside the icon along the border
    /// normal), so the two hit-tests are mutually exclusive — but
    /// the event loop checks text first so per-channel routing
    /// stays deterministic.
    pub fn hit_test_portal_text(&self, canvas_pos: Vec2) -> Option<(EdgeKey, String)> {
        find_first_aabb_hit(&self.portal_text_hitboxes, canvas_pos)
    }

    /// Pan the camera so `target` (canvas coordinates) is centred
    /// on the viewport at the current zoom. Used by the portal
    /// double-click handler to jump to the other side of a portal
    /// edge. Pure pan — no dirty flag raised; the shader transform
    /// plus render-time `visible_at` handle the new view.
    pub fn set_camera_center(&mut self, target: Vec2) {
        self.camera
            .apply_mutation(&baumhard::gfx_structs::camera::CameraMutation::SetPosition {
                canvas_pos: target,
            });
    }

    /// Fit the camera to show a Baumhard tree's content.
    pub fn fit_camera_to_tree(&mut self, tree: &Tree<GfxElement, GfxMutator>) {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        let mut found_any = false;

        for descendant_id in tree.root().descendants(&tree.arena) {
            let element = match tree.arena.get(descendant_id) {
                Some(n) => n.get(),
                None => continue,
            };
            let area = match element.glyph_area() {
                Some(a) => a,
                None => continue,
            };
            let x = area.position.x.0;
            let y = area.position.y.0;
            let w = area.render_bounds.x.0;
            let h = area.render_bounds.y.0;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + w);
            max_y = max_y.max(y + h);
            found_any = true;
        }
        if found_any {
            self.camera
                .apply_mutation(&baumhard::gfx_structs::camera::CameraMutation::FitToBounds {
                    min: Vec2::new(min_x, min_y),
                    max: Vec2::new(max_x, max_y),
                    padding_fraction: 0.05,
                });
            // The fit typically changes both pan and zoom. Today this
            // is only called from `load_mindmap`, which follows up
            // with a full connection rebuild against the new zoom —
            // but raise `geometry_dirty` so any future caller (e.g.
            // a "fit to selection" command) automatically gets a
            // scene-cache flush + rebuild on the next frame instead
            // of silently leaving stale samples behind.
            self.connection_geometry_dirty = true;
        }
    }

    pub fn screen_to_canvas(&self, screen_x: f32, screen_y: f32) -> Vec2 {
        self.camera.screen_to_canvas(Vec2::new(screen_x, screen_y))
    }

    /// Size of one screen pixel in canvas units — used to convert
    /// screen-space tolerances (e.g. click tolerance) to canvas-space
    /// distances that stay visually consistent across zoom.
    pub fn canvas_per_pixel(&self) -> f32 {
        if self.camera.zoom > f32::EPSILON {
            1.0 / self.camera.zoom
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_hash::FxHashMap;

    /// Closed-interval semantics: a click landing exactly on
    /// `min` or `max` *hits*. The four hit-test bodies all
    /// previously open-coded the `>=` / `<=` predicate; locking
    /// the boundary here prevents a future open-vs-closed drift.
    #[test]
    fn aabb_contains_is_closed_interval_on_both_bounds() {
        let min = Vec2::new(10.0, 20.0);
        let max = Vec2::new(30.0, 50.0);
        // Strictly inside.
        assert!(aabb_contains(Vec2::new(15.0, 30.0), min, max));
        // Exactly on the min corner.
        assert!(aabb_contains(min, min, max));
        // Exactly on the max corner.
        assert!(aabb_contains(max, min, max));
        // Exactly on each edge midpoint.
        assert!(aabb_contains(Vec2::new(min.x, 30.0), min, max));
        assert!(aabb_contains(Vec2::new(max.x, 30.0), min, max));
        assert!(aabb_contains(Vec2::new(20.0, min.y), min, max));
        assert!(aabb_contains(Vec2::new(20.0, max.y), min, max));
        // Outside on every side.
        assert!(!aabb_contains(Vec2::new(min.x - 0.01, 30.0), min, max));
        assert!(!aabb_contains(Vec2::new(max.x + 0.01, 30.0), min, max));
        assert!(!aabb_contains(Vec2::new(20.0, min.y - 0.01), min, max));
        assert!(!aabb_contains(Vec2::new(20.0, max.y + 0.01), min, max));
    }

    /// Empty map yields `None` — the no-match base case every
    /// hit-test caller relies on for the "no element under the
    /// cursor" branch.
    #[test]
    fn find_first_aabb_hit_empty_map_yields_none() {
        let map: FxHashMap<EdgeKey, (Vec2, Vec2)> = FxHashMap::default();
        assert!(find_first_aabb_hit(&map, Vec2::new(5.0, 5.0)).is_none());
    }

    /// Cursor outside every entry yields `None`. Locks the
    /// "linear scan returns None when no AABB contains the
    /// point" contract.
    #[test]
    fn find_first_aabb_hit_no_match_yields_none() {
        let mut map: FxHashMap<EdgeKey, (Vec2, Vec2)> = FxHashMap::default();
        map.insert(
            EdgeKey::new("a", "b", "cross_link"),
            (Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0)),
        );
        map.insert(
            EdgeKey::new("c", "d", "cross_link"),
            (Vec2::new(20.0, 20.0), Vec2::new(30.0, 30.0)),
        );
        assert!(find_first_aabb_hit(&map, Vec2::new(15.0, 15.0)).is_none());
    }

    /// A point strictly inside one entry returns that entry's
    /// key. (Iteration order over `FxHashMap` is unspecified, so
    /// we don't assert which entry wins on overlap — that's by
    /// design and matches the pre-refactor behavior.)
    #[test]
    fn find_first_aabb_hit_returns_a_containing_key() {
        let mut map: FxHashMap<EdgeKey, (Vec2, Vec2)> = FxHashMap::default();
        let key_a = EdgeKey::new("a", "b", "cross_link");
        let key_c = EdgeKey::new("c", "d", "cross_link");
        map.insert(key_a.clone(), (Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0)));
        map.insert(key_c.clone(), (Vec2::new(20.0, 20.0), Vec2::new(30.0, 30.0)));
        let hit = find_first_aabb_hit(&map, Vec2::new(5.0, 5.0));
        assert_eq!(hit, Some(key_a));
    }
}
