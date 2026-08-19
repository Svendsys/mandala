// SPDX-License-Identifier: MPL-2.0

//! Per-edge cache of connection glyph geometry.
//!
//! Why it exists: during a drag the scene builder would otherwise
//! re-sample every visible edge every frame, even though most edges
//! have not moved. For a 20,000-unit cross-link at typical spacing
//! that is ~1,667 Bezier point evaluations + a 256-entry arc-length
//! table rebuild per frame per such edge, which is more than enough
//! to blow the drag budget and stutter the interaction.
//!
//! This module lets the scene builder stash the **pre-clip** sampled
//! positions of each edge keyed by `(from_id, to_id, edge_type)`. On the
//! next frame, if neither endpoint of the edge has moved (i.e. neither
//! appears in the drag `offsets` map) the cached samples are reused — the
//! cheap `point_inside_any_node` clip filter still runs against the current
//! frame's `node_aabbs` so a stable edge still clips correctly around a
//! *moved* third node that passes through its path.
//!
//! Invariants:
//!
//! - The cache is always safe to drop. Clearing it just forces a full
//!   re-sample on the next build.
//! - Samples are stored in canvas space. Camera *pan* does not invalidate
//!   the cache. Camera *zoom*, however, DOES change the effective
//!   canvas-space font size (and therefore the sample spacing) via
//!   `GlyphConnectionConfig::effective_font_size_pt`, so zoom changes
//!   force a full re-sample. This is enforced automatically by
//!   `ensure_zoom`, which the scene builder calls on entry — callers
//!   don't need to remember to flush the cache on zoom themselves.
//! - **An entry holds no styling the frame could read back.** What a
//!   connection *looks like* — its body glyph, its cap glyphs, its font
//!   and its size — is resolved from the live `GlyphConnectionConfig` on
//!   every path, so no glyph edit can be served stale out of this
//!   module: there is nothing here to serve. What an entry does hold is
//!   the geometry plus the
//!   [`SampleParams`](crate::mindmap::scene_cache::SampleParams) that
//!   geometry was produced under, and both reuse doors —
//!   [`reusable`](crate::mindmap::scene_cache::SceneConnectionCache::reusable)
//!   and
//!   [`reusable_mut`](crate::mindmap::scene_cache::SceneConnectionCache::reusable_mut)
//!   — refuse an entry whose params no longer match the frame's.
//!
//! (Those three links are spelled crate-absolute on purpose. `mod.rs`
//! carries an outer `///` summary on `pub mod scene_cache;`, rustdoc
//! merges it with this header, and a relative link in the merged block
//! resolves against no module — the failure reports no file or line, so
//! it costs more to find than to avoid.)
//! - Structural edge changes the params cannot see — endpoint moves
//!   outside the drag `offsets` map, anchor and control-point edits,
//!   node resizes — are still handled by the caller clearing the
//!   relevant entries (`invalidate_edge`) or dropping the whole cache
//!   (`clear`), as is a theme-variable edit, since `color` is the one
//!   resolved value an entry still carries. Selection changes need
//!   neither — the selection override is applied per frame and never
//!   enters the cache.

use glam::Vec2;
use std::collections::HashMap;

use crate::font::metrics::monospace_advance;
use crate::mindmap::model::{GlyphConnectionConfig, MindEdge};
use crate::util::geometry::almost_equal;

/// Stable identity of a connection. Mirrors the `(from_id, to_id, edge_type)`
/// triple that the rest of the codebase uses to identify edges
/// (`document::EdgeRef` is the same shape).
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct EdgeKey {
    pub from_id: String,
    pub to_id: String,
    pub edge_type: String,
}

impl EdgeKey {
    /// Construct an `EdgeKey` from its three components. Accepts
    /// anything `Into<String>` so callers can pass `&str` or `String`
    /// without `.to_string()` boilerplate. O(1) + up to 3 allocations.
    pub fn new(from_id: impl Into<String>, to_id: impl Into<String>, edge_type: impl Into<String>) -> Self {
        Self {
            from_id: from_id.into(),
            to_id: to_id.into(),
            edge_type: edge_type.into(),
        }
    }

    /// Shorthand: derive the key from a `MindEdge`'s identity fields.
    /// O(1) + 3 string clones.
    pub fn from_edge(edge: &MindEdge) -> Self {
        Self::new(&edge.from_id, &edge.to_id, &edge.edge_type)
    }
}

/// Every input to a connection's glyph sampling that comes from
/// configuration rather than from geometry — which is to say,
/// everything that decides *where along the path* the samples land
/// and how many of them there are.
///
/// The list is short because the sampler's is:
/// `sample_path(&path, effective_spacing, budget)` reads a step and a
/// cap, `build_connection_path` reads only geometry, and
/// [`monospace_advance`] scales the font size by a constant with no
/// glyph input at all. So the *body glyph*, the *cap glyphs* and the
/// *font family* are absent on purpose — none of them can move a
/// sample, and a struct that compared them would refuse reuse of
/// geometry that is in fact still correct.
///
/// The point of having the type is that the cache's two reuse doors
/// compare *this whole value* rather than a hand-listed subset of an
/// entry's fields. Before it existed the translate path compared three
/// fields and the cache-hit fast path compared none, which is the same
/// defect one level down: whichever list is shorter is the one that
/// serves stale geometry. A field added here is compared by both doors
/// or by neither, and never by one (#36 item 7).
///
/// Plain `Copy` data — two floats and a `usize`, no allocation.
#[derive(Clone, Copy, Debug)]
pub struct SampleParams {
    /// `GlyphConnectionConfig::effective_font_size_pt(camera_zoom)`:
    /// the canvas-space size after the screen-space clamp. A camera
    /// zoom change and a `font_size_pt` / `min_` / `max_` edit both
    /// land here, which is why the field is the *effective* size
    /// rather than the authored one.
    pub font_size_pt: f32,
    /// `GlyphConnectionConfig::spacing` — added to the glyph advance
    /// to get the arc-length step between consecutive body glyphs.
    pub spacing: f32,
    /// The per-path allowance
    /// [`crate::mindmap::connection::per_path_sample_budget`] handed
    /// this pass. Scene-wide in origin but per-edge in effect: it caps
    /// *this* edge's sample count, so a map whose edge count changed
    /// can produce a different sample sequence from identical per-edge
    /// config.
    pub sample_budget: usize,
}

impl SampleParams {
    /// Snapshot the sampling inputs one pass is about to read for one
    /// edge.
    ///
    /// Takes `camera_zoom` rather than a pre-computed size so the
    /// screen-space clamp is applied in exactly one place; a caller
    /// that needs the canvas-space size reads [`Self::font_size_pt`]
    /// back off the result.
    ///
    /// Cost: the arithmetic of
    /// `GlyphConnectionConfig::effective_font_size_pt`. No allocation.
    pub fn snapshot(config: &GlyphConnectionConfig, camera_zoom: f32, sample_budget: usize) -> Self {
        Self {
            font_size_pt: config.effective_font_size_pt(camera_zoom),
            spacing: config.spacing,
            sample_budget,
        }
    }

    /// Arc-length step between consecutive body glyphs: one monospace
    /// advance at [`Self::font_size_pt`] plus [`Self::spacing`].
    ///
    /// The sampler reads its step through this method so the step and
    /// the value the cache compares cannot be derived two different
    /// ways. Cost: O(1).
    pub fn sample_spacing(&self) -> f32 {
        monospace_advance(self.font_size_pt) + self.spacing
    }

    /// Whether samples taken under `self` are the ones `other` would
    /// produce — the whole freshness test for cached geometry, in one
    /// place.
    ///
    /// Deliberately **not** a `PartialEq` impl. Both float fields are
    /// compared with [`almost_equal`] rather than `==`, and
    /// almost-equality is not transitive, so it is not an equivalence
    /// relation and writing it as `==` would promise a contract this
    /// cannot keep. The tolerance is wanted on both: `font_size_pt` is
    /// an arithmetic result that drifts in its last bits under the
    /// sub-`ZOOM_EPSILON` zoom wobble `ensure_zoom` deliberately
    /// tolerates, and a `spacing` difference below the tolerance moves
    /// no sample by more than the same tolerance.
    ///
    /// Cost: O(1).
    pub fn matches(&self, other: &Self) -> bool {
        self.sample_budget == other.sample_budget
            && almost_equal(self.font_size_pt, other.font_size_pt)
            && almost_equal(self.spacing, other.spacing)
    }
}

/// The cached geometry for a single edge, plus the [`SampleParams`]
/// it was produced under — together sufficient to rebuild the edge's
/// `ConnectionElement` without recomputing the path.
///
/// `pre_clip_positions` holds the raw sampled points BEFORE the
/// `point_inside_any_node` clip filter runs. We keep them pre-clip so a
/// moved-but-unrelated node's AABB can still push glyphs out of the
/// connection on the next frame: the clip filter is cheap (arithmetic over
/// cached `Vec2`s), the sampler is not.
///
/// **The caps are not stored, because they were never separate data.**
/// A cap sits at the first or last sampled position and carries a glyph
/// named in the live `GlyphConnectionConfig`, so [`Self::cap_positions`]
/// reads both ends of `pre_clip_positions` and the emitting pass pairs
/// them with the glyphs. Storing them was a second copy of both halves
/// that [`Self::translate`] then had to keep in step.
///
/// **Nor is anything else the frame draws with.** Body glyph, font
/// family and font size were held here and read back by the reuse
/// paths, which is what made a glyph edit invisible until something
/// flushed the cache. `color` is the exception and stays, because
/// resolving it walks the edge's theme cascade rather than reading one
/// config field; a theme-variable edit is correspondingly still the
/// caller's to invalidate.
///
/// `base_from` / `base_to` record the endpoint canvas positions that the
/// samples were taken at (i.e. `model.pos + offset_at_write`). When the next
/// frame brings a drag offset that moves both endpoints by the same delta
/// — the common subtree-drag case — the scene builder can skip the Bezier
/// sampler entirely and just translate the cached samples by that shared
/// delta. Anything that changes the edge's *shape* (endpoints moving by
/// different deltas, control-point edits, font-size / zoom clamp
/// transitions) falls through to a full resample.
#[derive(Clone, Debug)]
pub struct CachedConnection {
    pub pre_clip_positions: Vec<Vec2>,
    pub sample_params: SampleParams,
    pub color: String,
    pub base_from: Vec2,
    pub base_to: Vec2,
}

impl CachedConnection {
    /// The canvas positions the start and end caps occupy: the first
    /// and last sampled points, in that order.
    ///
    /// The order is not a convention — `sample_path` walks the path
    /// from the source anchor to the target anchor, so the first
    /// sample *is* where the start cap belongs. Both are `None` for an
    /// entry with no samples: the production path never stores one (an
    /// edge that samples to nothing is dropped from the cache
    /// instead), but the type is `pub` and a consumer can build one, so
    /// the ends are read rather than assumed. Cost: O(1).
    pub fn cap_positions(&self) -> (Option<Vec2>, Option<Vec2>) {
        (
            self.pre_clip_positions.first().copied(),
            self.pre_clip_positions.last().copied(),
        )
    }

    /// Rigid-body translate of this entry's geometry in place.
    /// Shifts `pre_clip_positions` by `delta` and stamps `base_from` /
    /// `base_to` to the new reference endpoints. The caps ride along
    /// because [`Self::cap_positions`] reads them off those same
    /// points.
    ///
    /// Why the whole entry is mutated instead of being rebuilt and
    /// handed back to [`SceneConnectionCache::insert`]: this runs on
    /// every internal edge of a subtree drag every drain. Routing
    /// through `insert` would reindex both `by_node` buckets (two
    /// `retain` scans + two `push` calls per edge) and clone `color` —
    /// none of which change under a pure translation.
    ///
    /// [`Self::sample_params`] is left alone by design: a rigid
    /// translation changes where the samples are, never what they were
    /// sampled under, and the caller reached this method only by
    /// presenting matching params to
    /// [`SceneConnectionCache::reusable_mut`].
    ///
    /// Cost: one pass over `pre_clip_positions`.
    pub fn translate(&mut self, delta: Vec2, new_base_from: Vec2, new_base_to: Vec2) {
        for p in &mut self.pre_clip_positions {
            *p += delta;
        }
        self.base_from = new_base_from;
        self.base_to = new_base_to;
    }
}

/// Per-edge cache of sampled connection geometry, plus a reverse
/// index from node ID → edges that touch it so a drag of node N
/// dirties the right edges in `O(k_N)` instead of walking the whole
/// edge list. Owned by the app's document / renderer glue and passed
/// into [`crate::mindmap::tree_builder::build_connection_elements`] on
/// each frame.
#[derive(Default, Debug)]
pub struct SceneConnectionCache {
    entries: HashMap<EdgeKey, CachedConnection>,
    by_node: HashMap<String, Vec<EdgeKey>>,
    /// Camera zoom level at which the cached samples were taken. `None`
    /// means "cache is empty / zoom unknown". When the scene builder is
    /// asked to build at a zoom that differs from this (beyond
    /// `ZOOM_EPSILON`), `ensure_zoom` flushes the cache so stale sample
    /// spacings don't leak into the new frame. Kept out of
    /// `CachedConnection` because it's a whole-cache property, not a
    /// per-edge one.
    scene_zoom: Option<f32>,
}

/// Threshold for "zoom changed enough to invalidate cached samples".
/// The sample spacing is `effective_font * 0.6 + spacing`, so sub-0.1%
/// zoom deltas shift spacing by a fraction of a pixel — cheaper to
/// ignore than to rebuild. 0.1% matches the lower bound of what a user
/// can deliberately produce with the wheel-zoom step (10%).
const ZOOM_EPSILON: f32 = 1.0e-3;

impl SceneConnectionCache {
    /// Construct an empty cache. Same as `Self::default()` — the
    /// explicit constructor exists so callers don't have to know the
    /// `Default` trait is derived. Allocation-free.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop everything. Used at drag-drop, undo, reparent, edge CRUD, fold
    /// toggle, theme-variable change — the cheap "when in doubt, flush"
    /// path. The next scene build re-populates the cache.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.by_node.clear();
        self.scene_zoom = None;
    }

    /// Ensure the cache is consistent with `camera_zoom`. If the stored
    /// zoom differs from the incoming one beyond `ZOOM_EPSILON`, drop
    /// all cached samples; either way, stamp the new zoom. Called by
    /// the connection pass on entry so the invariant is enforced
    /// locally instead of requiring every caller to remember to flush
    /// on zoom changes.
    ///
    /// When `scene_zoom` is `None` (fresh cache, post-`clear`, or
    /// pre-stamp) we just stamp without invalidating — any existing
    /// entries are assumed to be correct for `camera_zoom` (in
    /// production the scene builder stamps before inserting).
    pub fn ensure_zoom(&mut self, camera_zoom: f32) {
        let z = camera_zoom.max(f32::EPSILON);
        if let Some(prev) = self.scene_zoom {
            if (prev - z).abs() > ZOOM_EPSILON {
                self.entries.clear();
                self.by_node.clear();
            }
        }
        self.scene_zoom = Some(z);
    }

    /// `true` iff no cached entries are currently held. O(1).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of cached edge entries. O(1).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Look up a cached entry **whatever it was sampled under** — for
    /// diagnostics, tests, and any consumer that wants to see what is
    /// held rather than to reuse it.
    ///
    /// Reuse goes through [`Self::reusable`] / [`Self::reusable_mut`],
    /// which is why this is not called `get`: a call site that reads
    /// geometry out of *this* method and draws with it is skipping the
    /// freshness check, and the name should say so where it is
    /// written. Cost: one hash lookup.
    pub fn inspect(&self, key: &EdgeKey) -> Option<&CachedConnection> {
        self.entries.get(key)
    }

    /// The read-only reuse door: the entry for `key`, but only if its
    /// samples were taken under `want`.
    ///
    /// This is what the scene builder's cache-hit fast path holds. A
    /// `None` here means "resample", never "no such edge" — the two
    /// are indistinguishable to the caller on purpose, because the
    /// correct response to both is the same and a caller that could
    /// tell them apart could act on the difference.
    ///
    /// Cost: one hash lookup plus [`SampleParams::matches`].
    pub fn reusable(&self, key: &EdgeKey, want: &SampleParams) -> Option<&CachedConnection> {
        self.entries
            .get(key)
            .filter(|entry| entry.sample_params.matches(want))
    }

    /// The mutating reuse door: as [`Self::reusable`], but handing out
    /// the `&mut` that [`CachedConnection::translate`] needs.
    ///
    /// This is what the scene builder's translate path holds: the same
    /// borrow answers "is this edge cached and sampled the way this
    /// frame wants?", performs the translate, and is reborrowed as
    /// shared to emit the element — one hash lookup for the whole
    /// path, and no re-lookup whose success has to be asserted after
    /// the fact.
    ///
    /// Leaving `by_node` alone is sound rather than a shortcut: that
    /// index maps a node id to the edges that touch it, and the node
    /// ids live in the [`EdgeKey`], not in the entry this borrow
    /// reaches. Changing *which* nodes an edge connects is therefore
    /// changing its key, which is [`Self::invalidate_edge`] followed
    /// by [`Self::insert`] — the two paths that do re-index.
    ///
    /// Cost: one hash lookup plus [`SampleParams::matches`].
    pub fn reusable_mut(&mut self, key: &EdgeKey, want: &SampleParams) -> Option<&mut CachedConnection> {
        self.entries
            .get_mut(key)
            .filter(|entry| entry.sample_params.matches(want))
    }

    /// Insert or replace an entry, keeping the `by_node` reverse index in
    /// sync. Scene-builder writes (both "fresh sample" and "resample because
    /// endpoint moved") go through this.
    pub fn insert(&mut self, key: EdgeKey, entry: CachedConnection) {
        // Remove the key from any stale `by_node` bucket first — we can't
        // know the old endpoints without looking at the previous entry, so
        // the simplest correct thing is to strip the key from both new
        // endpoints' buckets and re-add it. In practice insertions come
        // paired with the current endpoints in the live map, so the
        // `from_id` / `to_id` on `key` are already the up-to-date ones.
        self.by_node
            .entry(key.from_id.clone())
            .or_default()
            .retain(|k| k != &key);
        self.by_node
            .entry(key.to_id.clone())
            .or_default()
            .retain(|k| k != &key);

        self.entries.insert(key.clone(), entry);

        self.by_node
            .entry(key.from_id.clone())
            .or_default()
            .push(key.clone());
        self.by_node.entry(key.to_id.clone()).or_default().push(key);
    }

    /// Which edges touch the given node? Used by the drag drain to mark
    /// dirty edges.
    pub fn edges_touching(&self, node_id: &str) -> &[EdgeKey] {
        self.by_node.get(node_id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Drop a single edge (key-direct invalidation). Keeps the reverse
    /// index in sync.
    pub fn invalidate_edge(&mut self, key: &EdgeKey) {
        if self.entries.remove(key).is_none() {
            return;
        }
        if let Some(bucket) = self.by_node.get_mut(&key.from_id) {
            bucket.retain(|k| k != key);
        }
        if let Some(bucket) = self.by_node.get_mut(&key.to_id) {
            bucket.retain(|k| k != key);
        }
    }

    /// After a scene build, evict any cache entries whose keys are not in
    /// the "seen this frame" set. Handles edges that were deleted from the
    /// model between builds.
    pub fn retain_keys(&mut self, seen: &std::collections::HashSet<EdgeKey>) {
        let to_evict: Vec<EdgeKey> = self
            .entries
            .keys()
            .filter(|k| !seen.contains(*k))
            .cloned()
            .collect();
        for key in to_evict {
            self.invalidate_edge(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The params every fixture entry in this module is sampled
    /// under, so a test that wants a reuse *hit* asks with this and a
    /// test that wants a *miss* asks with something else.
    fn mk_params() -> SampleParams {
        SampleParams {
            font_size_pt: 12.0,
            spacing: 0.0,
            sample_budget: 1000,
        }
    }

    fn mk_entry(color: &str) -> CachedConnection {
        CachedConnection {
            pre_clip_positions: vec![Vec2::new(1.0, 2.0), Vec2::new(3.0, 4.0)],
            sample_params: mk_params(),
            color: color.into(),
            base_from: Vec2::ZERO,
            base_to: Vec2::ZERO,
        }
    }

    #[test]
    fn insert_and_get_round_trips() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));
        assert_eq!(cache.inspect(&key).unwrap().color, "#fff");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn edges_touching_indexes_both_endpoints() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));
        assert_eq!(cache.edges_touching("a"), std::slice::from_ref(&key));
        assert_eq!(cache.edges_touching("b"), std::slice::from_ref(&key));
        assert!(cache.edges_touching("c").is_empty());
    }

    #[test]
    fn edges_touching_handles_multiple_edges_per_node() {
        let mut cache = SceneConnectionCache::new();
        let k1 = EdgeKey::new("hub", "a", "cross_link");
        let k2 = EdgeKey::new("hub", "b", "cross_link");
        let k3 = EdgeKey::new("c", "hub", "parent_child");
        cache.insert(k1.clone(), mk_entry("#111"));
        cache.insert(k2.clone(), mk_entry("#222"));
        cache.insert(k3.clone(), mk_entry("#333"));

        let touching: std::collections::HashSet<&EdgeKey> = cache.edges_touching("hub").iter().collect();
        assert_eq!(touching.len(), 3);
        assert!(touching.contains(&k1));
        assert!(touching.contains(&k2));
        assert!(touching.contains(&k3));
    }

    #[test]
    fn invalidate_edge_removes_from_entries_and_index() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));
        cache.invalidate_edge(&key);
        assert!(cache.inspect(&key).is_none());
        assert!(cache.edges_touching("a").is_empty());
        assert!(cache.edges_touching("b").is_empty());
    }

    #[test]
    fn clear_empties_everything() {
        let mut cache = SceneConnectionCache::new();
        cache.insert(EdgeKey::new("a", "b", "cross_link"), mk_entry("#fff"));
        cache.insert(EdgeKey::new("b", "c", "cross_link"), mk_entry("#000"));
        cache.clear();
        assert!(cache.is_empty());
        assert!(cache.edges_touching("a").is_empty());
        assert!(cache.edges_touching("b").is_empty());
    }

    #[test]
    fn retain_keys_evicts_unseen() {
        use std::collections::HashSet;
        let mut cache = SceneConnectionCache::new();
        let kept = EdgeKey::new("a", "b", "cross_link");
        let evicted = EdgeKey::new("c", "d", "cross_link");
        cache.insert(kept.clone(), mk_entry("#111"));
        cache.insert(evicted.clone(), mk_entry("#222"));

        let mut seen = HashSet::new();
        seen.insert(kept.clone());
        cache.retain_keys(&seen);

        assert!(cache.inspect(&kept).is_some());
        assert!(cache.inspect(&evicted).is_none());
        assert!(cache.edges_touching("c").is_empty());
        assert!(cache.edges_touching("d").is_empty());
    }

    #[test]
    fn reinsert_same_key_does_not_duplicate_index_entries() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#111"));
        cache.insert(key.clone(), mk_entry("#222"));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.edges_touching("a").len(), 1);
        assert_eq!(cache.edges_touching("b").len(), 1);
        assert_eq!(cache.inspect(&key).unwrap().color, "#222");
    }

    #[test]
    fn ensure_zoom_preserves_cache_on_matching_zoom() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));
        cache.ensure_zoom(1.0);
        // Same zoom again — nothing should change.
        cache.ensure_zoom(1.0);
        assert!(cache.inspect(&key).is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn ensure_zoom_invalidates_on_zoom_change() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));
        cache.ensure_zoom(1.0);
        // Wheel-tick to 1.1 — entries must be dropped.
        cache.ensure_zoom(1.1);
        assert!(cache.inspect(&key).is_none());
        assert!(cache.edges_touching("a").is_empty());
    }

    #[test]
    fn ensure_zoom_tolerates_sub_epsilon_drift() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));
        cache.ensure_zoom(1.0);
        // Tiny floating-point drift well below ZOOM_EPSILON (1e-3).
        cache.ensure_zoom(1.0 + 1.0e-6);
        assert!(
            cache.inspect(&key).is_some(),
            "sub-epsilon drift should not flush"
        );
    }

    #[test]
    fn ensure_zoom_after_clear_just_stamps() {
        let mut cache = SceneConnectionCache::new();
        // Empty cache + ensure_zoom should just stamp, not panic or
        // touch anything.
        cache.ensure_zoom(0.5);
        cache.insert(EdgeKey::new("a", "b", "cross_link"), mk_entry("#111"));
        // Same zoom — preserved.
        cache.ensure_zoom(0.5);
        assert_eq!(cache.len(), 1);
    }

    /// `EdgeKey::new` accepts mixed `&str` and `String` payloads via
    /// `Into<String>`. Pins the constructor surface — callers
    /// shouldn't have to know which side wants what.
    #[test]
    fn edge_key_new_accepts_mixed_str_and_string_inputs() {
        let from = String::from("a");
        let k1 = EdgeKey::new(&from, "b", "cross_link");
        let k2 = EdgeKey::new("a", String::from("b"), "cross_link");
        let k3 = EdgeKey::new(from.clone(), "b".to_string(), "cross_link");
        assert_eq!(k1, k2);
        assert_eq!(k1, k3);
        assert_eq!(k1.from_id, "a");
        assert_eq!(k1.to_id, "b");
        assert_eq!(k1.edge_type, "cross_link");
    }

    /// `EdgeKey::from_edge` produces the same key as the
    /// `EdgeKey::new(from, to, type)` triple constructed from the
    /// edge's identity fields. Pins the round-trip — a regression
    /// where `from_edge` looked at `to_id` first or used a different
    /// edge_type would show up here.
    #[test]
    fn edge_key_from_edge_matches_explicit_new() {
        let edge = crate::mindmap::test_helpers::synthetic_edge("alpha", "beta", "auto", "auto");
        let k_from = EdgeKey::from_edge(&edge);
        // `synthetic_edge` defaults edge_type to `cross_link`.
        let k_new = EdgeKey::new("alpha", "beta", "cross_link");
        assert_eq!(k_from, k_new);
    }

    /// Edge-deletion eviction: when a caller drops an edge from
    /// the model, `invalidate_edge` must remove BOTH the cache
    /// entry AND the reverse-index buckets for both endpoints.
    /// A regression that forgot to clear `by_node` would leak
    /// stale `EdgeKey`s in `edges_touching` results, which the
    /// drag-drain consumes — leading to "ghost edges" that
    /// repaint with no model backing.
    #[test]
    fn invalidate_edge_removes_from_both_endpoint_buckets_on_deletion() {
        let mut cache = SceneConnectionCache::new();
        let kept = EdgeKey::new("hub", "a", "cross_link");
        let evicted = EdgeKey::new("hub", "b", "cross_link");
        cache.insert(kept.clone(), mk_entry("#111"));
        cache.insert(evicted.clone(), mk_entry("#222"));

        cache.invalidate_edge(&evicted);

        // Hub side: only the kept edge survives in the bucket.
        let hub_bucket: std::collections::HashSet<&EdgeKey> = cache.edges_touching("hub").iter().collect();
        assert_eq!(hub_bucket.len(), 1);
        assert!(hub_bucket.contains(&kept));
        // Other endpoint's bucket is now empty.
        assert!(cache.edges_touching("b").is_empty());
        // Forward lookup is gone too.
        assert!(cache.inspect(&evicted).is_none());
        // Sibling edge is untouched.
        assert!(cache.inspect(&kept).is_some());
    }

    /// Cache-miss semantics: `get` on an unknown key returns
    /// `None` cleanly without mutating cache state. The drag-tick
    /// hot path asks the cache "do you know this edge?" hundreds
    /// of times per frame; a miss must be cheap and side-effect
    /// free.
    #[test]
    fn get_on_missing_key_returns_none_without_side_effects() {
        let mut cache = SceneConnectionCache::new();
        let known = EdgeKey::new("a", "b", "cross_link");
        cache.insert(known.clone(), mk_entry("#fff"));
        let len_before = cache.len();

        let missing = EdgeKey::new("x", "y", "cross_link");
        assert!(cache.inspect(&missing).is_none());

        // No phantom insertion on miss; reverse index untouched.
        assert_eq!(cache.len(), len_before);
        assert!(cache.edges_touching("x").is_empty());
        assert!(cache.edges_touching("y").is_empty());
        assert!(cache.inspect(&known).is_some());
    }

    /// Rename guard for the miss-semantics test above: the accessor
    /// it names is `inspect`, and `inspect` deliberately answers
    /// "what is stored" rather than "what may be reused".
    #[test]
    fn inspect_returns_an_entry_the_reuse_doors_would_refuse() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));

        let mut other = mk_params();
        other.font_size_pt += 10.0;

        assert!(
            cache.inspect(&key).is_some(),
            "inspect answers regardless of what the entry was sampled under"
        );
        assert!(
            cache.reusable(&key, &other).is_none(),
            "the reuse door must refuse the same entry"
        );
    }

    /// The whole of item 7's guard in one assertion per field: an
    /// entry sampled under one set of params is not reusable under
    /// any other.
    ///
    /// Each case names the edit that produces it — a zoom step or an
    /// `edge font size` verb for the first, an `edge spacing` verb
    /// for the second, adding or deleting an edge for the third.
    #[test]
    fn reusable_refuses_an_entry_sampled_under_different_params() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));
        let same = mk_params();
        assert!(
            cache.reusable(&key, &same).is_some(),
            "precondition: matching params must be reusable, or every case below is vacuous"
        );

        for (label, mutate) in [
            (
                "font_size_pt",
                (|p: &mut SampleParams| p.font_size_pt += 1.0) as fn(&mut SampleParams),
            ),
            ("spacing", |p: &mut SampleParams| p.spacing += 1.0),
            ("sample_budget", |p: &mut SampleParams| p.sample_budget += 1),
        ] {
            let mut want = mk_params();
            mutate(&mut want);
            assert!(
                cache.reusable(&key, &want).is_none(),
                "a changed `{label}` must make the entry non-reusable"
            );
            assert!(
                cache.reusable_mut(&key, &want).is_none(),
                "the mutating door must refuse a changed `{label}` too"
            );
        }
    }

    /// Sub-tolerance drift is not a config change. `ensure_zoom`
    /// deliberately keeps the cache across a zoom wobble below
    /// `ZOOM_EPSILON`; a params check that used `==` would then
    /// resample every one of those frames anyway, undoing the
    /// tolerance one level up.
    #[test]
    fn reusable_tolerates_sub_epsilon_params_drift() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));

        let mut wobbled = mk_params();
        wobbled.font_size_pt += 1.0e-7;
        wobbled.spacing += 1.0e-7;
        assert!(
            cache.reusable(&key, &wobbled).is_some(),
            "drift far below the almost-equal tolerance must not force a resample"
        );
    }

    /// The caps are the ends of the sample array, in order — the
    /// invariant `emit_connection_element` reads them under.
    #[test]
    fn cap_positions_reports_the_first_and_last_sample_in_that_order() {
        let entry = CachedConnection {
            pre_clip_positions: vec![Vec2::new(1.0, 2.0), Vec2::new(50.0, 2.0), Vec2::new(99.0, 2.0)],
            sample_params: mk_params(),
            color: "#fff".into(),
            base_from: Vec2::ZERO,
            base_to: Vec2::ZERO,
        };
        assert_eq!(
            entry.cap_positions(),
            (Some(Vec2::new(1.0, 2.0)), Some(Vec2::new(99.0, 2.0)))
        );
    }

    /// A one-sample entry puts both caps on the same point rather
    /// than dropping one, and an empty one reports neither. Neither
    /// shape is reachable from the production sampler, which is why
    /// the method reads the ends instead of indexing them.
    #[test]
    fn cap_positions_handles_one_sample_and_none() {
        let mut entry = CachedConnection {
            pre_clip_positions: vec![Vec2::new(7.0, 7.0)],
            sample_params: mk_params(),
            color: "#fff".into(),
            base_from: Vec2::ZERO,
            base_to: Vec2::ZERO,
        };
        assert_eq!(
            entry.cap_positions(),
            (Some(Vec2::new(7.0, 7.0)), Some(Vec2::new(7.0, 7.0)))
        );
        entry.pre_clip_positions.clear();
        assert_eq!(entry.cap_positions(), (None, None));
    }

    /// `translate` moves the caps because it moves the points they
    /// are read off, and leaves the sampling params alone because a
    /// rigid shift changes where samples are, not what they were
    /// taken under.
    #[test]
    fn translate_carries_the_caps_and_leaves_the_params_alone() {
        let mut entry = CachedConnection {
            pre_clip_positions: vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)],
            sample_params: mk_params(),
            color: "#fff".into(),
            base_from: Vec2::new(0.0, 0.0),
            base_to: Vec2::new(10.0, 0.0),
        };
        let before = entry.sample_params;

        entry.translate(Vec2::new(3.0, -4.0), Vec2::new(3.0, -4.0), Vec2::new(13.0, -4.0));

        assert_eq!(
            entry.cap_positions(),
            (Some(Vec2::new(3.0, -4.0)), Some(Vec2::new(13.0, -4.0))),
            "both caps must ride the translation"
        );
        assert_eq!(entry.base_from, Vec2::new(3.0, -4.0));
        assert_eq!(entry.base_to, Vec2::new(13.0, -4.0));
        assert!(
            before.matches(&entry.sample_params),
            "a translation must not disturb the params the entry is keyed on for reuse"
        );
    }

    /// `sample_spacing` is the arc-length step the sampler walks and
    /// the value the cache compares, derived once. Computed here
    /// from the documented ratio rather than by calling
    /// `monospace_advance` again, so a change to that ratio has to
    /// be a decision rather than a silent agreement.
    #[test]
    fn sample_spacing_is_the_glyph_advance_plus_the_configured_gap() {
        let params = SampleParams {
            font_size_pt: 10.0,
            spacing: 2.5,
            sample_budget: 100,
        };
        // MONOSPACE_ADVANCE_RATIO is 0.6, so 10pt advances 6.0.
        assert!(
            almost_equal(params.sample_spacing(), 8.5),
            "expected 6.0 + 2.5, got {}",
            params.sample_spacing()
        );
    }

    /// `snapshot` reads the *effective* font size, so the
    /// screen-space clamp is inside the value the cache compares
    /// rather than outside it. At zoom 4 a 12pt authored size wants
    /// 48pt on screen; a 20pt ceiling pulls it back to 20 on screen,
    /// which is 5pt in canvas space.
    #[test]
    fn snapshot_records_the_clamped_effective_font_size() {
        let config = GlyphConnectionConfig {
            font_size_pt: 12.0,
            min_font_size_pt: 8.0,
            max_font_size_pt: 20.0,
            spacing: 1.5,
            ..GlyphConnectionConfig::default()
        };
        let params = SampleParams::snapshot(&config, 4.0, 77);
        assert!(
            almost_equal(params.font_size_pt, 5.0),
            "expected the 20pt screen ceiling divided back through zoom 4, got {}",
            params.font_size_pt
        );
        assert!(almost_equal(params.spacing, 1.5));
        assert_eq!(params.sample_budget, 77);
    }
}
