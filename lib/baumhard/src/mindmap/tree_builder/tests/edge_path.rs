// SPDX-License-Identifier: MPL-2.0

//! The per-frame [`EdgePathCache`]: one build per edge across the
//! three passes that want one, none at all for an edge no pass asks
//! about, and the endpoint-rect resolution the memo shares with the
//! sampler.

use super::super::{EdgePathCache, SceneSelectionContext};
use super::fixtures::*;
use crate::mindmap::connection::ConnectionPath;
use crate::mindmap::model::{GlyphConnectionConfig, MindMap};
use crate::mindmap::scene_cache::SceneConnectionCache;
use crate::mindmap::test_helpers::synthetic_edge;
use glam::Vec2;
use std::collections::HashMap;

/// The two-node fixture with a label on its single edge, so the
/// label pass has something to lay out.
fn labeled_edge_map() -> MindMap {
    let mut map = two_node_edge_map();
    map.edges[0].label = Some("hello".to_string());
    map
}

/// One connection path per edge per frame, however many passes want
/// it.
///
/// The fixture edge is labeled *and* selected, and the scene cache
/// starts empty, so all three askers fire in one rebuild: the
/// sampler on its slow path, the selected edge's grab-handles, and
/// the label layout. Each used to call `build_connection_path`
/// itself.
///
/// Input that makes it fail: any of the three going back to building
/// its own — the count goes to 2 or 3, and the assertion names which.
#[test]
fn test_one_connection_path_serves_the_sampler_the_handles_and_the_label() {
    let map = labeled_edge_map();
    let mut cache = SceneConnectionCache::new();
    let roles = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext {
            edge: Some(("a", "b", "cross_link")),
            ..Default::default()
        },
        None,
        None,
        None,
        &mut cache,
        1.0,
    );

    // Preconditions: all three consumers really did run, or the
    // count below is 1 because two of them never asked.
    assert_eq!(
        roles.connection_elements.len(),
        1,
        "precondition: the sampler must have emitted this edge"
    );
    assert!(
        !roles.edge_handles.is_empty(),
        "precondition: the selected edge must have emitted grab-handles"
    );
    assert_eq!(
        roles.connection_label_elements.len(),
        1,
        "precondition: the labeled edge must have emitted a label"
    );
    assert!(
        cache.is_empty() || cache.len() == 1,
        "precondition: the sampler took its slow path and wrote the cache"
    );

    assert_eq!(
        roles.paths_built, 1,
        "sampler, handles and label share one built path per edge per frame"
    );
}

/// Nothing is built for an edge no pass needs a path for.
///
/// The failure this guards against is the obvious wrong fix for the
/// item above: resolve every edge's path up front and hand out a
/// slice. That trades three builds on a few edges for one build on
/// all of them, and on a static frame — cache warm, no label, no
/// selection — the right answer is zero.
///
/// Input that makes it fail: making the memo eager, in which case
/// the second build reports one path for an edge nothing drew from a
/// path.
#[test]
fn test_no_connection_path_is_built_for_an_edge_no_pass_asks_about() {
    // Unlabeled and never selected.
    let map = two_node_edge_map();
    let mut cache = SceneConnectionCache::new();

    let first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_eq!(
        first.paths_built, 1,
        "precondition: the first build misses the scene cache and does sample"
    );
    assert_eq!(
        cache.len(),
        1,
        "precondition: the first build filled the scene cache"
    );

    let second = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_eq!(
        second.connection_elements.len(),
        1,
        "precondition: the edge still renders, from the scene cache"
    );
    assert_eq!(
        second.paths_built, 0,
        "a cache-hit edge with no label and no selection needs no path at all"
    );
}

/// A labeled edge still gets its one path on a scene-cache hit — the
/// label pass has no cache of its own, so it is the one asker left.
///
/// This is the other half of the test above: without it, "0 paths on
/// a warm frame" would also pass on an implementation that never
/// built a path for labels at all, which would put every label at
/// the origin.
#[test]
fn test_a_labeled_edge_still_gets_its_path_on_a_scene_cache_hit() {
    let map = labeled_edge_map();
    let mut cache = SceneConnectionCache::new();
    let _first = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    let second = project_with_cache(
        &map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut cache,
        1.0,
    );
    assert_eq!(
        second.paths_built, 1,
        "the label pass has no cache of its own and must still resolve the path"
    );
    // And the label landed somewhere on the edge rather than at the
    // origin: the fixture runs from a's right edge (x=40) to b's left
    // edge (x=400) at y=20, so the midpoint label sits near (220, 20).
    let label = &second.connection_label_elements[0];
    let center = (
        label.position.0 + label.bounds.0 * 0.5,
        label.position.1 + label.bounds.1 * 0.5,
    );
    assert!(
        crate::util::geometry::almost_equal(center.0, 220.0)
            && crate::util::geometry::almost_equal(center.1, 20.0),
        "label should sit at the path midpoint, got {center:?}"
    );
}

/// The memo answers a repeat ask from storage.
#[test]
fn test_edge_path_cache_builds_each_edge_once() {
    let map = two_node_edge_map();
    let offsets = HashMap::new();
    let mut paths = EdgePathCache::new(&map, &offsets);
    assert_eq!(paths.built(), 0, "a fresh memo has built nothing");

    let first = paths.path(0).expect("the fixture edge resolves").clone();
    assert_eq!(paths.built(), 1);
    let second = paths.path(0).expect("the fixture edge resolves").clone();
    assert_eq!(paths.built(), 1, "the second ask must not build");

    let (ConnectionPath::Straight { start: s1, end: e1 }, ConnectionPath::Straight { start: s2, end: e2 }) =
        (first, second)
    else {
        panic!("the fixture edge has no control points, so its path is straight");
    };
    assert_eq!((s1, e1), (s2, e2), "both asks must describe the same path");
}

/// Out-of-range and dangling edges answer `None` and build nothing,
/// rather than panicking on the index or caching a path resolved
/// from a missing node.
#[test]
fn test_edge_path_cache_declines_a_dangling_or_out_of_range_edge() {
    let mut map = two_node_edge_map();
    map.edges
        .push(synthetic_edge("a", "nonexistent", "right", "left"));
    let offsets = HashMap::new();
    let mut paths = EdgePathCache::new(&map, &offsets);

    assert!(
        paths.path(1).is_none(),
        "an edge whose target is missing has no path"
    );
    assert!(paths.path(99).is_none(), "an out-of-range index has no path");
    assert_eq!(paths.built(), 0, "neither may occupy a slot");

    // The healthy sibling is unaffected.
    assert!(paths.path(0).is_some());
    assert_eq!(paths.built(), 1);
}

/// The memo resolves an edge's endpoints through the same drag
/// offsets the sampler does, so a shared path is the *moved* path
/// rather than the committed one.
///
/// Input that makes it fail: a memo that read `node.pos_vec2()`
/// directly — the path would be right on a static frame and a whole
/// drag delta wrong on every drag frame, which is the sort of thing
/// that only shows up in motion.
#[test]
fn test_edge_path_cache_applies_the_drag_offsets() {
    let map = two_node_edge_map();
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (0.0, 60.0));

    let no_offset = HashMap::new();
    let mut still = EdgePathCache::new(&map, &no_offset);
    let mut dragged = EdgePathCache::new(&map, &offsets);

    let (ConnectionPath::Straight { start: rest, .. }, ConnectionPath::Straight { start: moved, .. }) = (
        still.path(0).expect("static path").clone(),
        dragged.path(0).expect("dragged path").clone(),
    ) else {
        panic!("the fixture edge has no control points, so its path is straight");
    };
    assert!(
        crate::util::geometry::almost_equal(moved.y - rest.y, 60.0),
        "the source anchor must ride node a's +60 y offset: {} -> {}",
        rest.y,
        moved.y
    );
}

/// `offset_node_rect` is the one spelling of "this node's live
/// rectangle" — the size passes through untouched and only the
/// position moves.
#[test]
fn test_offset_node_rect_moves_the_position_and_not_the_size() {
    let node = sized_node("n", 10.0, 20.0, 30.0, 40.0, false);
    let mut offsets = HashMap::new();
    offsets.insert("n".to_string(), (3.0, -7.0));

    let (pos, size) = super::super::offset_node_rect(&node, &offsets);
    assert_eq!(pos, Vec2::new(13.0, 13.0));
    assert_eq!(size, Vec2::new(30.0, 40.0));

    let (unmoved, same_size) = super::super::offset_node_rect(&node, &HashMap::new());
    assert_eq!(unmoved, Vec2::new(10.0, 20.0), "an unlisted node does not move");
    assert_eq!(same_size, size);
}

/// A glyph-config edit does not change where the path runs, so a
/// path already in the memo is still the right one for it.
///
/// This is the boundary between the two memos this pass now carries:
/// `SceneConnectionCache` refuses geometry sampled under different
/// `SampleParams` (#36 item 7), while `EdgePathCache` correctly does
/// not care, because `build_connection_path` reads no config at all.
/// Asserting it keeps a future "be safe, key the path on the config
/// too" from landing unnoticed.
#[test]
fn test_the_connection_path_does_not_depend_on_the_glyph_config() {
    let base = two_node_edge_map();
    let mut restyled = two_node_edge_map();
    restyled.edges[0].glyph_connection = Some(GlyphConnectionConfig {
        body: "#".into(),
        cap_start: Some("<".into()),
        cap_end: Some(">".into()),
        font: Some("Some Other Family".into()),
        font_size_pt: 40.0,
        spacing: 12.0,
        ..GlyphConnectionConfig::default()
    });

    let offsets = HashMap::new();
    let mut a = EdgePathCache::new(&base, &offsets);
    let mut b = EdgePathCache::new(&restyled, &offsets);
    let (ConnectionPath::Straight { start: s1, end: e1 }, ConnectionPath::Straight { start: s2, end: e2 }) = (
        a.path(0).expect("base path").clone(),
        b.path(0).expect("restyled path").clone(),
    ) else {
        panic!("the fixture edge has no control points, so its path is straight");
    };
    assert_eq!(
        (s1, e1),
        (s2, e2),
        "the path is geometry; the glyph config is not"
    );
}
