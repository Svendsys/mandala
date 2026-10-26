// SPDX-License-Identifier: MPL-2.0

//! The capture engine's scratch scene: where a screenshot or a
//! recording frame framed at a zoom other than the window's gets its
//! zoom-dependent canvas roles from, so that the live scene is never
//! written for a picture of it.
//!
//! **What is scratch, and what stays live.** Connections (with the
//! edge handles the connection pass emits beside them), connection
//! labels and portals are only right for the zoom they were projected
//! at — their glyph sizes are clamped in screen points
//! ([`CanvasRole::zoom_dependent`](crate::application::scene_host::CanvasRole::zoom_dependent)).
//! A frame at another zoom needs
//! them projected again, and this module projects them into an
//! `AppScene` of the capture's own, with a `SceneConnectionCache` of
//! its own, through the very body the live scene is projected through
//! after a camera change
//! (`scene_rebuild::CanvasFrame::update_zoom_dependent_roles`). The
//! node text, the node backgrounds, the borders, the section frames
//! and the resize handles are the same at every zoom, so the frame
//! draws them from the live set as the window would; the renderer
//! interleaves the two by layer (`renderer::compose_capture_canvas`)
//! and `Renderer::prepare_frame` reads the composition for that one
//! frame (`CanvasSource::Composed`) — pointed at it by the caller,
//! never left pointed at it.
//!
//! **What this keeps out of the live scene, by construction.** The
//! scratch `AppScene` carries its own signatures and its own hit
//! indices (`portal_hit_index`, `connection_label_hit_index`,
//! `connection_index`), stamped by the same dispatchers that stamp
//! the live ones; nothing here holds a `&mut` to the live `AppScene`,
//! the live `SceneConnectionCache` or the renderer's live set, so a
//! capture cannot leave any of them in a capture's state whatever is
//! added to the capture path later. The §B2 dispatch and the
//! renderer's shape cache both work on the scratch as they do on the
//! live: across the frames of a recording at one zoom, the scratch
//! trees take the in-place mutator arm and the scratch shaped output
//! is reused, so a frame costs the data pass and what actually moved.
//!
//! **A node drag's offsets reach it.** A `MovingNode` drag holds the
//! dragged nodes' offsets outside the model until release
//! (`MovingNodeInteraction::drained_offsets`); the live scene is
//! projected with them on every drained frame, and the scratch is
//! projected with the same map, so a capture mid-drag shows the
//! dragged node's connections where the window shows them. This is
//! what makes the `busy` refusal unnecessary: there is no longer a
//! projection from the model alone to refuse.
//!
//! **The scratch cache follows the live cache's statements.** Every
//! site that tells the live `SceneConnectionCache` the model's
//! geometry moved — a drag release, an undo, an animation tick, an
//! edge edit — does so through `clear` or `invalidate_edge`, which the
//! live cache counts ([`SceneConnectionCache::invalidations`]). The
//! scratch records the count it last projected under and drops its
//! own entries when the count has moved, so it is stale exactly when
//! the live cache was told it would be, without any of those sites
//! knowing a second cache exists. A capture zoom that changes between
//! captures is the scratch cache's own business, as it is the live
//! cache's (`ensure_zoom`).
//!
//! **One statement the live cache never hears, and the scratch needs:
//! a drag ending.** The live scene is projected on every drained
//! frame with the drag's current offsets, so when the release commits
//! the total into the model the live cache's entries already sit
//! where the model now is (and the one case where they do not — a
//! delta the throttle left undrained — is a `clear`). The scratch is
//! projected only when a capture asks, so its entries for the dragged
//! nodes' edges sit at the model plus whatever offsets the *last
//! capture* saw, which the release may have overtaken. The scratch
//! therefore keeps the node ids its last projection's offsets named,
//! and a node absent from the next projection's map has every edge
//! touching it invalidated (`edges_touching`) — exactly the entries
//! sampled at an offset position, and no others, so a recording that
//! runs on past a drag re-samples the dragged subtree's edges once
//! and keeps the rest warm.
//!
//! Native-only with the engine (`CLAUDE.md` "Dual-target status",
//! "Screenshot capture").

use std::collections::HashMap;

use baumhard::mindmap::scene_cache::{EdgeKey, SceneConnectionCache};

use crate::application::document::MindMapDocument;
use crate::application::scene_host::AppScene;
#[cfg(test)]
use crate::application::scene_host::CanvasRole;

use super::scene_rebuild::CanvasFrame;
use super::InteractionMode;

/// The scratch scene and its cache, owned by the app state beside the
/// recorder and the artifacts dir, and reused across captures.
pub(super) struct ScratchCanvas {
    scene: AppScene,
    cache: SceneConnectionCache,
    /// The live cache's [`SceneConnectionCache::invalidations`] this
    /// scratch cache last projected under; see the module header.
    mirrored_invalidations: u64,
    /// The node ids the last projection's `offsets` named — the nodes
    /// whose edges the scratch cache holds at an offset position. Empty
    /// outside a drag. Rewritten only when the set changes, which is
    /// once at a drag's start and once at its end: a recording's frames
    /// through a drag compare it and clone nothing.
    offset_nodes: Vec<String>,
}

impl Default for ScratchCanvas {
    fn default() -> Self {
        Self::new()
    }
}

impl ScratchCanvas {
    /// An empty scratch: no roles registered, nothing cached. The
    /// first projection registers every zoom-dependent role.
    pub(super) fn new() -> Self {
        ScratchCanvas {
            scene: AppScene::new(),
            cache: SceneConnectionCache::new(),
            mirrored_invalidations: 0,
            offset_nodes: Vec::new(),
        }
    }

    /// Project the zoom-dependent roles of `doc` at `zoom`, with
    /// `offsets` (a node drag's drained offsets, or an empty map) and
    /// the chrome `interaction_mode` asks for, into the scratch scene —
    /// after dropping what the scratch cache can no longer vouch for:
    /// everything, if `live_cache` has been told the model moved since
    /// the last projection; the edges of every node that has left the
    /// offsets map since then (the module header's "a drag ending").
    ///
    /// The scratch scene is then what the live scene would be after a
    /// camera change to `zoom` with those offsets, and
    /// `Renderer::rebuild_capture_canvas_buffers` reads it.
    ///
    /// # Costs
    ///
    /// One `CanvasFrame` assembly and the three zoom-dependent
    /// projections; the connection pass re-samples every visible edge
    /// on the first capture at a zoom and only the edges `offsets`
    /// moves on the next at the same zoom, unless the live cache was
    /// invalidated in between. Plus one hash lookup per node the last
    /// offsets named, and on a drag's start or end one clone of the
    /// map's keys.
    pub(super) fn project(
        &mut self,
        zoom: f32,
        doc: &MindMapDocument,
        interaction_mode: &InteractionMode,
        offsets: &HashMap<String, (f32, f32)>,
        live_cache: &SceneConnectionCache,
    ) {
        let invalidations = live_cache.invalidations();
        if invalidations != self.mirrored_invalidations {
            self.cache.clear();
            self.mirrored_invalidations = invalidations;
        }
        self.release_departed_offsets(offsets);
        let mut frame = CanvasFrame::new(doc, offsets, interaction_mode.resize_handle_overrides(), zoom);
        frame.update_zoom_dependent_roles(&mut self.cache, &mut self.scene);
    }

    /// Invalidate the edges of every node the last projection's
    /// offsets named and `offsets` does not, and record `offsets`'
    /// nodes as the last ones if the set changed.
    fn release_departed_offsets(&mut self, offsets: &HashMap<String, (f32, f32)>) {
        let departed = self.offset_nodes.iter().any(|node| !offsets.contains_key(node));
        if departed {
            let stale: Vec<EdgeKey> = self
                .offset_nodes
                .iter()
                .filter(|node| !offsets.contains_key(*node))
                .flat_map(|node| self.cache.edges_touching(node).iter().cloned())
                .collect();
            for key in &stale {
                self.cache.invalidate_edge(key);
            }
        }
        if departed || self.offset_nodes.len() != offsets.len() {
            self.offset_nodes = offsets.keys().cloned().collect();
        }
    }

    /// The scratch scene, for the renderer to walk into the capture
    /// canvas.
    pub(super) fn scene_mut(&mut self) -> &mut AppScene {
        &mut self.scene
    }

    /// The scratch scene, read-only: for the test that holds it to
    /// the live scene's shape.
    #[cfg(test)]
    pub(super) fn scene(&self) -> &AppScene {
        &self.scene
    }

    /// The roles a projection registers, named so a test can hold
    /// them to [`CanvasRole::zoom_dependent`].
    #[cfg(test)]
    pub(super) fn registered_roles(&self) -> Vec<CanvasRole> {
        CanvasRole::ALL
            .into_iter()
            .filter(|role| self.scene.canvas_id(*role).is_some())
            .collect()
    }

    /// How many edges the scratch cache holds, for the test that
    /// watches it follow the live cache's invalidations.
    #[cfg(test)]
    pub(super) fn cached_edges(&self) -> usize {
        self.cache.len()
    }

    /// The scratch cache's pre-clip samples for `edge`, for the tests
    /// that watch it re-sample — read through `inspect`, which is the
    /// cache's own "what is held" door.
    #[cfg(test)]
    pub(super) fn cached_samples(&self, edge: &EdgeKey) -> Option<Vec<glam::Vec2>> {
        self.cache
            .inspect(edge)
            .map(|entry| entry.pre_clip_positions.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::renderer::{
        compose_capture_canvas, refresh, CanvasSegment, CaptureCanvas, ScenePassKind, ShapedSceneElement,
        ShapedSet,
    };
    use baumhard::gfx_structs::scene::SceneTreeId;
    use baumhard::mindmap::scene_cache::EdgeKey;
    use glam::Vec2;

    /// Project every canvas role into `app_scene` at `zoom`, as
    /// `rebuild_scene_only` does.
    fn project_live(
        doc: &MindMapDocument,
        app_scene: &mut AppScene,
        cache: &mut SceneConnectionCache,
        zoom: f32,
    ) {
        let offsets = HashMap::new();
        CanvasFrame::new(
            doc,
            &offsets,
            InteractionMode::Default.resize_handle_overrides(),
            zoom,
        )
        .update_all(cache, app_scene);
    }

    /// Walk `app_scene`'s canvas sub-scene into `shaped`, as the
    /// renderer's flush does.
    fn flush(app_scene: &mut AppScene, shaped: &mut Vec<ShapedSceneElement>) -> (usize, usize, usize) {
        let ids = app_scene.canvas_ids_in_layer_order();
        let counts = refresh(app_scene.canvas_scene(), &ids, shaped, ScenePassKind::Canvas);
        (counts.walked, counts.reshaped, counts.patched)
    }

    /// Everything a capture must leave alone on the live scene, read
    /// out so two readings can be compared: per role, the registered
    /// tree's id, both signatures and its element count; plus the
    /// three hit indices.
    fn live_fingerprint(app_scene: &AppScene) -> Vec<String> {
        let mut out = Vec::new();
        for role in CanvasRole::ALL {
            let id = app_scene.canvas_id(role);
            let elements = app_scene.canvas_entry(role).map(|entry| entry.tree().arena.len());
            out.push(format!(
                "{role:?}: id={id:?} structure={:?} content={:?} elements={elements:?}",
                app_scene.canvas_signature(role),
                app_scene.canvas_content_signature(role),
            ));
        }
        out.push(format!("portals: {:?}", app_scene.portal_index()));
        for role in [CanvasRole::Connections, CanvasRole::ConnectionLabels] {
            out.push(format!(
                "{role:?} index: {:?}",
                app_scene.edge_channel_index(role)
            ));
        }
        out
    }

    /// The live cache as a capture must leave it: its size and every
    /// sample of one edge, so a re-sample at another zoom shows.
    fn cache_fingerprint(cache: &SceneConnectionCache, key: &EdgeKey) -> (usize, Option<Vec<Vec2>>) {
        (
            cache.len(),
            cache.inspect(key).map(|entry| entry.pre_clip_positions.clone()),
        )
    }

    /// The canvas positions of every glyph leaf of the scratch
    /// connection tree's child for `edge`, as drawn.
    fn connection_leaf_positions(scene: &AppScene, edge: &EdgeKey) -> Vec<Vec2> {
        let entry = scene
            .canvas_entry(CanvasRole::Connections)
            .expect("the scratch registers the connection role");
        let index = scene
            .edge_channel_index(CanvasRole::Connections)
            .expect("connections carry an edge index");
        let tree = entry.tree();
        // The index names a glyph by the root child it sits under, so
        // each edge's child is found through one of its own glyphs.
        let edge_node = tree
            .root()
            .children(&tree.arena)
            .find(|child| {
                child
                    .children(&tree.arena)
                    .next()
                    .and_then(|glyph| index.resolve(tree, glyph))
                    .as_ref()
                    == Some(edge)
            })
            .expect("the edge is drawn");
        edge_node
            .descendants(&tree.arena)
            .filter_map(|id| tree.arena.get(id).and_then(|node| node.get().glyph_area()))
            .map(|area| Vec2::new(area.position.x.0, area.position.y.0) + entry.offset())
            .collect()
    }

    /// Whether two canvas positions agree to a hundredth of a unit —
    /// the comparison every "re-sampled where the model now is" check
    /// here makes. `almost_equal_vec2`'s tolerance is sized for unit
    /// values and is a last-bit miss at the fixture's coordinates
    /// (around a thousand units), where a sample re-derived through
    /// the model's f64 positions and one translated in f32 agree to
    /// far better than a glyph's width and not to the bit.
    fn same_place(a: Vec2, b: Vec2) -> bool {
        (a - b).length() <= 1e-2
    }

    /// A labeled edge of the canonical fixture, from a parent with
    /// children of its own.
    const EDGE: (&str, &str, &str) = ("1", "1.2", "parent_child");

    /// The zoom every scratch projection here is taken at, against a
    /// live scene at 1.0. Chosen so the screen-point clamp on the
    /// connection font bites — the fixture's 12 pt and 18 pt bodies
    /// fall below the 8 pt floor at it, and the window of the unclamped
    /// zooms (two thirds to about ten, for 12 pt) is exactly where a
    /// zoom change moves no sample and a test at 2.5 proved nothing.
    const CAPTURE_ZOOM: f32 = 0.25;

    /// **The acceptance criterion, CPU-side.** A projection at another
    /// zoom through the scratch writes nothing to the live `AppScene`
    /// (ids, both signatures, element counts, the three hit indices),
    /// the live `SceneConnectionCache` (size and samples) or the live
    /// shaped set (a refresh after it re-shapes and patches nothing).
    /// The negative control projects the same zoom into the live scene
    /// the way the camera rebuild does, and every one of those readings
    /// moves.
    #[test]
    fn test_a_scratch_projection_at_another_zoom_writes_nothing_live() {
        baumhard::font::fonts::init();
        let doc = load_test_doc();
        let key = EdgeKey::new(EDGE.0, EDGE.1, EDGE.2);

        let mut live = AppScene::new();
        let mut live_cache = SceneConnectionCache::new();
        project_live(&doc, &mut live, &mut live_cache, 1.0);
        let mut live_shaped = Vec::new();
        let (walked, _, _) = flush(&mut live, &mut live_shaped);
        assert!(
            walked > 100,
            "the fixture must fill the live canvas: {walked} positions"
        );
        let before = live_fingerprint(&live);
        let cache_before = cache_fingerprint(&live_cache, &key);
        assert!(
            cache_before.1.is_some(),
            "the fixture edge must be cached live, or the cache reading proves nothing"
        );

        let mut scratch = ScratchCanvas::new();
        scratch.project(
            CAPTURE_ZOOM,
            &doc,
            &InteractionMode::Default,
            &HashMap::new(),
            &live_cache,
        );
        let mut scratch_shaped = Vec::new();
        let (scratch_walked, _, _) = flush(scratch.scene_mut(), &mut scratch_shaped);
        assert!(
            scratch_walked > 10,
            "the scratch must hold a real projection: {scratch_walked} positions"
        );

        assert_eq!(live_fingerprint(&live), before, "the live scene must not change");
        assert_eq!(
            cache_fingerprint(&live_cache, &key),
            cache_before,
            "the live cache must not change"
        );
        assert_eq!(
            flush(&mut live, &mut live_shaped),
            (walked, 0, 0),
            "the live shaped set must have nothing to re-shape or move"
        );

        // Control: the same zoom projected into the live scene, which
        // is what the capture did before it had a scratch.
        let offsets = HashMap::new();
        CanvasFrame::new(
            &doc,
            &offsets,
            InteractionMode::Default.resize_handle_overrides(),
            CAPTURE_ZOOM,
        )
        .update_zoom_dependent_roles(&mut live_cache, &mut live);
        assert_ne!(
            live_fingerprint(&live),
            before,
            "the control must move the live scene, or the assertions above are vacuous"
        );
        let cache_after = cache_fingerprint(&live_cache, &key);
        assert_ne!(
            cache_after.1, cache_before.1,
            "the control must re-sample the live cache"
        );
        let (_, reshaped, patched) = flush(&mut live, &mut live_shaped);
        assert!(
            reshaped + patched > 0,
            "the control must touch the live shaped set"
        );
    }

    /// The scratch registers exactly the roles
    /// [`CanvasRole::zoom_dependent`] names — the split the composition
    /// draws by — and nothing of the other four, which the frame takes
    /// from the live scene. Control: the live projection registers all
    /// eight.
    #[test]
    fn test_the_scratch_holds_exactly_the_zoom_dependent_roles() {
        baumhard::font::fonts::init();
        let doc = load_test_doc();
        let live_cache = SceneConnectionCache::new();
        let mut scratch = ScratchCanvas::new();
        assert!(scratch.registered_roles().is_empty());

        scratch.project(
            CAPTURE_ZOOM,
            &doc,
            &InteractionMode::Default,
            &HashMap::new(),
            &live_cache,
        );

        let expected: Vec<CanvasRole> = CanvasRole::ALL
            .into_iter()
            .filter(|role| role.zoom_dependent())
            .collect();
        assert_eq!(expected.len(), 4, "four of the eight roles read the zoom");
        assert_eq!(scratch.registered_roles(), expected);

        let mut live = AppScene::new();
        let mut cache = SceneConnectionCache::new();
        project_live(&doc, &mut live, &mut cache, 1.0);
        assert!(
            CanvasRole::ALL.iter().all(|role| live.canvas_id(*role).is_some()),
            "the control registers every role"
        );
    }

    /// A node drag's offsets reach the scratch: with `1.2` and its
    /// subtree offset, the edge inside that subtree is drawn translated
    /// by exactly the offset (both endpoints moved together), and the
    /// edge into the subtree is re-sampled rather than translated. The
    /// same offsets again change nothing, and no offsets put every
    /// glyph back.
    #[test]
    fn test_drag_offsets_move_the_scratch_connections_with_the_nodes() {
        baumhard::font::fonts::init();
        let doc = load_test_doc();
        let live_cache = SceneConnectionCache::new();
        let mode = InteractionMode::Default;
        let inside = EdgeKey::new("1.2", "1.2.0", "parent_child");
        let into = EdgeKey::new(EDGE.0, EDGE.1, EDGE.2);
        assert!(
            doc.mindmap.nodes.contains_key("1.2.0"),
            "the fixture must give `1.2` a child, or the subtree edge does not exist"
        );

        let mut scratch = ScratchCanvas::new();
        scratch.project(CAPTURE_ZOOM, &doc, &mode, &HashMap::new(), &live_cache);
        let inside_at_rest = connection_leaf_positions(scratch.scene(), &inside);
        let into_at_rest = connection_leaf_positions(scratch.scene(), &into);
        assert!(
            inside_at_rest.len() > 2 && into_at_rest.len() > 2,
            "both edges draw glyphs"
        );

        let delta = Vec2::new(300.0, -40.0);
        let mut offsets: HashMap<String, (f32, f32)> = HashMap::new();
        for id in std::iter::once("1.2".to_string()).chain(doc.mindmap.all_descendants("1.2")) {
            offsets.insert(id, (delta.x, delta.y));
        }
        scratch.project(CAPTURE_ZOOM, &doc, &mode, &offsets, &live_cache);
        let inside_moved = connection_leaf_positions(scratch.scene(), &inside);
        assert_eq!(inside_moved.len(), inside_at_rest.len());
        for (moved, rest) in inside_moved.iter().zip(&inside_at_rest) {
            assert!(
                same_place(*moved, *rest + delta),
                "a glyph of the subtree's own edge must ride the offset: {moved:?} vs {rest:?} + {delta:?}"
            );
        }
        let into_moved = connection_leaf_positions(scratch.scene(), &into);
        assert_ne!(
            into_moved, into_at_rest,
            "the edge into the subtree must be redrawn"
        );
        assert!(
            into_moved
                .iter()
                .zip(&into_at_rest)
                .any(|(moved, rest)| !same_place(*moved, *rest + delta)),
            "an edge with one endpoint moved is re-sampled, not translated"
        );

        let mut shaped = Vec::new();
        flush(scratch.scene_mut(), &mut shaped);
        scratch.project(CAPTURE_ZOOM, &doc, &mode, &offsets, &live_cache);
        let (_, reshaped, patched) = flush(scratch.scene_mut(), &mut shaped);
        assert_eq!((reshaped, patched), (0, 0), "the same offsets again move nothing");

        // The drag ends with the model where it was (a canceled drag):
        // the nodes leave the map, the edges sampled at their offset
        // positions are re-sampled, and every glyph is back.
        scratch.project(CAPTURE_ZOOM, &doc, &mode, &HashMap::new(), &live_cache);
        assert_eq!(
            connection_leaf_positions(scratch.scene(), &inside),
            inside_at_rest
        );
        assert_eq!(connection_leaf_positions(scratch.scene(), &into), into_at_rest);
    }

    /// The statement the live cache never hears: a drag ends, and the
    /// model absorbs offsets the scratch's last projection only saw
    /// half of. The nodes leaving the offsets map is what re-samples
    /// their edges from the model; the live cache is deliberately not
    /// cleared here, as a release with nothing undrained does not
    /// clear it.
    #[test]
    fn test_a_drag_that_ended_has_its_edges_re_sampled_from_the_model() {
        baumhard::font::fonts::init();
        let mut doc = load_test_doc();
        let live_cache = SceneConnectionCache::new();
        let mode = InteractionMode::Default;
        let inside = EdgeKey::new("1.2", "1.2.0", "parent_child");
        let subtree: Vec<String> = std::iter::once("1.2".to_string())
            .chain(doc.mindmap.all_descendants("1.2"))
            .collect();

        let mut scratch = ScratchCanvas::new();
        scratch.project(CAPTURE_ZOOM, &doc, &mode, &HashMap::new(), &live_cache);
        let at_rest = scratch.cached_samples(&inside).expect("the edge is cached");

        // The last capture of the drag saw it halfway.
        let total = Vec2::new(400.0, 120.0);
        let halfway: HashMap<String, (f32, f32)> = subtree
            .iter()
            .map(|id| (id.clone(), (total.x / 2.0, total.y / 2.0)))
            .collect();
        scratch.project(CAPTURE_ZOOM, &doc, &mode, &halfway, &live_cache);
        let seen = scratch.cached_samples(&inside).expect("the edge is cached");
        assert!(
            seen.iter()
                .zip(&at_rest)
                .all(|(s, r)| same_place(*s, *r + total / 2.0)),
            "precondition: the scratch holds the halfway samples, or the re-sample below proves nothing"
        );

        // The release: the whole total lands in the model, no offsets
        // remain, and no cache is told.
        for id in &subtree {
            let node = doc.mindmap.nodes.get_mut(id).expect("fixture node");
            node.position.x += f64::from(total.x);
            node.position.y += f64::from(total.y);
        }
        scratch.project(CAPTURE_ZOOM, &doc, &mode, &HashMap::new(), &live_cache);
        let after = scratch.cached_samples(&inside).expect("the edge is cached");
        assert_eq!(after.len(), at_rest.len());
        for (moved, rest) in after.iter().zip(&at_rest) {
            assert!(
                same_place(*moved, *rest + total),
                "the edge must be re-sampled where the model now is: {moved:?} vs {rest:?} + {total:?}"
            );
        }
        assert!(
            scratch.cached_edges() > 1,
            "only the departed nodes' edges were dropped; the rest of the cache stayed warm"
        );
    }

    /// The scratch cache drops its entries when — and only when — the
    /// live cache has been told the model moved. Shown as correctness:
    /// a node moved in the model behind the cache's back leaves the
    /// scratch drawing the old position (the debt the cache's own
    /// header names), and a `clear` on the *live* cache is what makes
    /// the next scratch projection draw the new one.
    #[test]
    fn test_the_scratch_cache_follows_the_live_caches_invalidations() {
        baumhard::font::fonts::init();
        let mut doc = load_test_doc();
        let mut live_cache = SceneConnectionCache::new();
        let mode = InteractionMode::Default;
        let inside = EdgeKey::new("1.2", "1.2.0", "parent_child");

        let mut scratch = ScratchCanvas::new();
        scratch.project(CAPTURE_ZOOM, &doc, &mode, &HashMap::new(), &live_cache);
        let at_rest = scratch.cached_samples(&inside).expect("the edge is cached");
        let held = scratch.cached_edges();
        assert!(held > 0, "the scratch cache must hold something to drop");

        // The model moves without telling any cache: `1.2`'s whole
        // subtree, so the edge inside it is a pure translation the
        // cache would serve stale.
        let delta = Vec2::new(0.0, 500.0);
        for id in std::iter::once("1.2".to_string()).chain(doc.mindmap.all_descendants("1.2")) {
            let node = doc.mindmap.nodes.get_mut(&id).expect("fixture node");
            node.position.x += f64::from(delta.x);
            node.position.y += f64::from(delta.y);
        }
        scratch.project(CAPTURE_ZOOM, &doc, &mode, &HashMap::new(), &live_cache);
        assert_eq!(
            scratch.cached_samples(&inside).as_ref(),
            Some(&at_rest),
            "control: with no invalidation the scratch serves the stale samples"
        );
        assert_eq!(scratch.cached_edges(), held);

        // Then the live cache is told, as every mutation path tells it.
        live_cache.clear();
        scratch.project(CAPTURE_ZOOM, &doc, &mode, &HashMap::new(), &live_cache);
        let after = scratch.cached_samples(&inside).expect("the edge is cached again");
        assert_eq!(after.len(), at_rest.len());
        for (moved, rest) in after.iter().zip(&at_rest) {
            assert!(
                same_place(*moved, *rest + delta),
                "after the live cache's clear the scratch re-samples: {moved:?} vs {rest:?} + {delta:?}"
            );
        }

        // And one statement is mirrored once: the same count again
        // keeps the scratch cache warm.
        let warm = scratch.cached_edges();
        scratch.project(CAPTURE_ZOOM, &doc, &mode, &HashMap::new(), &live_cache);
        assert_eq!(scratch.cached_edges(), warm);
    }

    /// The composition draws every role from the set that holds the
    /// right projection of it, in layer order: the live set's span for
    /// a zoom-independent role, the scratch set's for a zoom-dependent
    /// one, each span exactly the walk positions the role's tree
    /// occupies. Control: the live set alone, every role from it, is a
    /// different sequence.
    #[test]
    fn test_the_composition_interleaves_live_and_scratch_spans_by_layer() {
        baumhard::font::fonts::init();
        let doc = load_test_doc();
        let mut live = AppScene::new();
        let mut live_cache = SceneConnectionCache::new();
        project_live(&doc, &mut live, &mut live_cache, 1.0);
        let mut live_shaped = Vec::new();
        flush(&mut live, &mut live_shaped);

        let mut scratch = ScratchCanvas::new();
        scratch.project(
            CAPTURE_ZOOM,
            &doc,
            &InteractionMode::Default,
            &HashMap::new(),
            &live_cache,
        );
        let mut capture = CaptureCanvas::default();
        flush(scratch.scene_mut(), capture.shaped_mut());

        compose_capture_canvas(&live, scratch.scene(), &live_shaped, &mut capture);

        // Expected: computed from the role split and each set's walk
        // order, not from the composition.
        let mut roles = CanvasRole::ALL;
        roles.sort_by_key(|role| role.layer());
        let span = |shaped: &[ShapedSceneElement], id: SceneTreeId| {
            let first = shaped
                .iter()
                .position(|element| element.tree() == id)
                .expect("every registered role walks at least its root");
            let len = shaped[first..].iter().take_while(|e| e.tree() == id).count();
            first..first + len
        };
        let expected: Vec<CanvasSegment> = roles
            .iter()
            .map(|role| {
                if role.zoom_dependent() {
                    let id = scratch.scene().canvas_id(*role).expect("scratch role");
                    CanvasSegment {
                        set: ShapedSet::Scratch,
                        range: span(capture.shaped(), id),
                    }
                } else {
                    let id = live.canvas_id(*role).expect("live role");
                    CanvasSegment {
                        set: ShapedSet::Live,
                        range: span(&live_shaped, id),
                    }
                }
            })
            .collect();
        assert_eq!(capture.segments(), expected.as_slice());
        assert_eq!(
            capture
                .segments()
                .iter()
                .filter(|s| s.set == ShapedSet::Scratch)
                .count(),
            4
        );
        let covered: usize = capture.segments().iter().map(|s| s.range.len()).sum();
        assert!(
            covered > 100,
            "the composition must cover real content: {covered} positions"
        );
        let scratch_covered: usize = capture
            .segments()
            .iter()
            .filter(|s| s.set == ShapedSet::Scratch)
            .map(|s| s.range.len())
            .sum();
        assert_eq!(
            scratch_covered,
            capture.shaped().len(),
            "the four scratch spans are the whole scratch set"
        );

        // Control: the same composition with no scratch roles registered
        // draws only the live spans.
        let mut empty = CaptureCanvas::default();
        compose_capture_canvas(&live, &AppScene::new(), &live_shaped, &mut empty);
        assert_eq!(empty.segments().len(), 4);
        assert!(empty.segments().iter().all(|s| s.set == ShapedSet::Live));
        assert_ne!(empty.segments(), capture.segments());
    }
}
