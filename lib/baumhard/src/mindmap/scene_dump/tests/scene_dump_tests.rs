// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::mindmap::scene_dump`]: that every dump is the
//! tree the renderer draws, named the way its hit test names it, with
//! the renderer's own cull verdict and the text its shaper slices.
//!
//! `pub mod tests` per `TEST_CONVENTIONS` §T2.2: the `do_*` bodies are
//! the serializer's criterion entries, so the fixture is built inside
//! each body and the bench measures load + build + dump together.

use std::collections::HashMap;

use glam::Vec2;

use crate::gfx_structs::camera::{Camera2D, Visibility};
use crate::mindmap::loader;
use crate::mindmap::model::{MindMap, TextRun, DISPLAY_MODE_PORTAL};
use crate::mindmap::scene_cache::{EdgeKey, SceneConnectionCache};
use crate::mindmap::scene_dump::{
    dump_edges, dump_labels, dump_node, dump_nodes, dump_portals, node_boxes, DumpFilter, Placement,
};
use crate::mindmap::tree_builder::{
    build_connection_elements, build_connection_label_tree, build_connection_tree, build_label_elements,
    build_mindmap_tree, build_portal_tree_from_pairs, connection_channel_index, connection_identity_sequence,
    node_clip_aabbs, portal_pair_data, EdgePathCache,
};

/// The canonical fixture, `maps/testament.mindmap.json`.
fn testament() -> MindMap {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.push("maps/testament.mindmap.json");
    loader::load_from_file(&path).expect("testament loads")
}

/// A 1280×720 camera at zoom 1 centered on `node_id`'s box.
fn camera_on(map: &MindMap, node_id: &str) -> Camera2D {
    let node = &map.nodes[node_id];
    let mut camera = Camera2D::new(1280, 720);
    camera.position = Vec2::new(
        (node.position.x + node.size.width / 2.0) as f32,
        (node.position.y + node.size.height / 2.0) as f32,
    );
    camera
}

/// The id of a node with children, in sorted order, so the fixture
/// choice is deterministic.
fn a_parent(map: &MindMap) -> String {
    let mut parents: Vec<&str> = map
        .nodes
        .values()
        .filter_map(|n| n.parent_id.as_deref())
        .collect();
    parents.sort();
    parents[0].to_string()
}

#[test]
fn test_scene_dump_nodes_are_the_tree_in_pre_order() {
    do_scene_dump_nodes_are_the_tree_in_pre_order();
}

/// One entry per drawn node, in the order the renderer walks the
/// arena, each placed exactly where its container area is and
/// carrying exactly the sections the tree built for it.
pub fn do_scene_dump_nodes_are_the_tree_in_pre_order() {
    let map = testament();
    let tree = build_mindmap_tree(&map);
    let camera = camera_on(&map, &a_parent(&map));
    let dump = dump_nodes(&tree, &map, &camera, DumpFilter::ALL);

    let walk_order: Vec<&str> = tree
        .tree
        .root()
        .descendants(&tree.tree.arena)
        .filter_map(|id| tree.mind_id_for_node(id))
        .collect();
    let dumped: Vec<&str> = dump.iter().map(|n| n.node_id.as_str()).collect();
    assert_eq!(dumped, walk_order);
    assert_eq!(dump.len(), tree.node_count());
    assert_eq!(dump.len(), map.nodes.len(), "testament folds nothing");

    for node in &dump {
        let arena_id = tree
            .arena_id_for(&node.node_id)
            .expect("dumped nodes are in the tree");
        let area = tree.tree.arena[arena_id]
            .get()
            .glyph_area()
            .expect("container area");
        assert_eq!(node.placement, Placement::of_area(area, Vec2::ZERO, &camera));
        assert_eq!(node.sections.len(), tree.section_count_for(&node.node_id));
        let indices: Vec<usize> = node.sections.iter().map(|s| s.section_idx).collect();
        let mut sorted = indices.clone();
        sorted.sort_unstable();
        assert_eq!(indices, sorted, "sections in model order");
        assert_eq!(Some(node.clone()), dump_node(&tree, &map, &camera, &node.node_id));
    }
    let boxes: Vec<(&str, Placement)> = node_boxes(&tree, &camera).collect();
    let placements: Vec<(&str, Placement)> = dump.iter().map(|n| (n.node_id.as_str(), n.placement)).collect();
    assert_eq!(
        boxes, placements,
        "the boxes are the dump's, without the sections"
    );
    assert_eq!(dump_node(&tree, &map, &camera, "no-such-node"), None);
}

#[test]
fn test_scene_dump_node_verdicts_name_the_cull_reason() {
    do_scene_dump_node_verdicts_name_the_cull_reason();
}

/// The verdict is the renderer's: the node the camera is centered on
/// is visible, a node far from it is off-screen, and a node whose zoom
/// window excludes the camera's zoom is outside its window even with
/// the camera centered on it.
pub fn do_scene_dump_node_verdicts_name_the_cull_reason() {
    let mut map = testament();
    let mut ids: Vec<String> = map.nodes.keys().cloned().collect();
    ids.sort();
    let windowed = ids[0].clone();
    map.nodes
        .get_mut(&windowed)
        .expect("fixture node")
        .min_zoom_to_render = Some(3.0);
    let tree = build_mindmap_tree(&map);

    let on_windowed = camera_on(&map, &windowed);
    let node = dump_node(&tree, &map, &on_windowed, &windowed).expect("drawn");
    assert_eq!(node.placement.visibility, Visibility::OutsideZoomWindow);
    for section in &node.sections {
        assert_eq!(
            section.placement.visibility,
            Visibility::OutsideZoomWindow,
            "sections inherit the window"
        );
    }

    let other = ids[1].clone();
    let on_other = camera_on(&map, &other);
    assert_eq!(
        dump_node(&tree, &map, &on_other, &other)
            .expect("drawn")
            .placement
            .visibility,
        Visibility::Visible
    );
    let mut far = camera_on(&map, &other);
    far.position += Vec2::new(1.0e7, 1.0e7);
    assert_eq!(
        dump_node(&tree, &map, &far, &other)
            .expect("drawn")
            .placement
            .visibility,
        Visibility::Offscreen
    );

    // The surface rect is the canvas rect through the camera.
    let placed = dump_node(&tree, &map, &on_other, &other)
        .expect("drawn")
        .placement;
    let corner = on_other.canvas_to_screen(Vec2::new(placed.canvas.x, placed.canvas.y));
    assert_eq!((placed.surface.x, placed.surface.y), (corner.x, corner.y));
    assert_eq!(placed.surface.w, placed.canvas.w * on_other.zoom);
}

#[test]
fn test_scene_dump_a_fold_removes_the_descendants() {
    do_scene_dump_a_fold_removes_the_descendants();
}

/// A folded node is drawn and says it is folded; its descendants are
/// not drawn, and the model names the fold that hides each of them.
pub fn do_scene_dump_a_fold_removes_the_descendants() {
    let mut map = testament();
    let parent = a_parent(&map);
    map.nodes.get_mut(&parent).expect("fixture node").folded = true;
    let hidden: Vec<String> = map.all_descendants(&parent);
    assert!(!hidden.is_empty());
    let tree = build_mindmap_tree(&map);
    let camera = camera_on(&map, &parent);

    let dump = dump_nodes(&tree, &map, &camera, DumpFilter::ALL);
    assert_eq!(dump.len(), map.nodes.len() - hidden.len());
    let folded = dump
        .iter()
        .find(|n| n.node_id == parent)
        .expect("the folded node is drawn");
    assert!(folded.folded);
    for id in &hidden {
        assert!(
            dump_node(&tree, &map, &camera, id).is_none(),
            "{id} is folded away"
        );
        let ancestor = map.folding_ancestor(&map.nodes[id]).expect("hidden by a fold");
        assert_eq!(ancestor.id, parent);
    }
    assert!(map.folding_ancestor(&map.nodes[&parent]).is_none());
}

#[test]
fn test_scene_dump_section_text_is_the_covered_text() {
    do_scene_dump_section_text_is_the_covered_text();
}

/// A section whose runs leave a gap shows only the covered graphemes,
/// and the dump reports those rather than the model text.
pub fn do_scene_dump_section_text_is_the_covered_text() {
    let mut map = testament();
    let mut ids: Vec<String> = map.nodes.keys().cloned().collect();
    ids.sort();
    let id = ids[0].clone();
    let section = &mut map.nodes.get_mut(&id).expect("fixture node").sections[0];
    section.text = "seen hidden seen".to_string();
    let template = section
        .text_runs
        .first()
        .cloned()
        .expect("testament sections carry runs");
    section.text_runs = vec![
        TextRun {
            start: 0,
            end: 5,
            ..template.clone()
        },
        TextRun {
            start: 12,
            end: 16,
            ..template
        },
    ];
    let tree = build_mindmap_tree(&map);
    let node = dump_node(&tree, &map, &camera_on(&map, &id), &id).expect("drawn");
    let first = node
        .sections
        .iter()
        .find(|s| s.section_idx == 0)
        .expect("section 0");
    assert_eq!(first.text.as_deref(), Some("seen seen"));
}

#[test]
fn test_scene_dump_edge_roles_are_named_by_their_index() {
    do_scene_dump_edge_roles_are_named_by_their_index();
}

/// Connections, labels and portal markers, each named through the
/// index its role's dispatcher stamps: one connection entry per
/// element that drew a glyph, in slice order, counting its glyphs; one
/// label per labeled edge, carrying its text; an icon and a caption
/// per portal endpoint.
pub fn do_scene_dump_edge_roles_are_named_by_their_index() {
    let mut map = testament();
    let portal_index = map
        .edges
        .iter()
        .position(|e| e.label.is_none())
        .expect("an unlabeled edge");
    map.edges[portal_index].display_mode = Some(DISPLAY_MODE_PORTAL.to_string());
    let portal_key = {
        let e = &map.edges[portal_index];
        EdgeKey::new(e.from_id.as_str(), e.to_id.as_str(), e.edge_type.as_str())
    };
    let offsets: HashMap<String, (f32, f32)> = HashMap::new();
    let hidden = map.fold_hidden_set();
    let mut cache = SceneConnectionCache::new();
    cache.ensure_zoom(1.0);
    let mut paths = EdgePathCache::new(&map, &offsets);
    let clip = node_clip_aabbs(&map, &offsets, None, &hidden);
    let (connections, _) = build_connection_elements(
        &map, &offsets, &clip, None, None, &mut cache, &mut paths, 1.0, &hidden,
    );
    let camera = Camera2D::new(1280, 720);

    let tree = build_connection_tree(&connections);
    let index = connection_channel_index(connection_identity_sequence(&connections));
    let edges = dump_edges(&tree, Vec2::ZERO, &camera, &index, DumpFilter::ALL);
    let drawn: Vec<(&EdgeKey, usize)> = connections
        .iter()
        .map(|c| {
            let caps = usize::from(c.cap_start.is_some()) + usize::from(c.cap_end.is_some());
            (&c.edge_key, c.glyph_positions.len() + caps)
        })
        .filter(|(_, glyphs)| *glyphs > 0)
        .collect();
    let dumped: Vec<(&EdgeKey, usize)> = edges.iter().map(|e| (&e.edge, e.glyphs)).collect();
    assert_eq!(dumped, drawn);
    assert!(
        !edges.iter().any(|e| e.edge == portal_key),
        "a portal edge draws no path"
    );
    for edge in &edges {
        assert!(edge.visible_glyphs <= edge.glyphs);
        assert_eq!(edge.placement.visibility.is_visible(), edge.visible_glyphs > 0);
    }

    let labels = build_label_elements(&map, None, None, None, &mut paths, 1.0, &hidden);
    let label_tree = build_connection_label_tree(&labels);
    let dumped = dump_labels(
        &label_tree.tree,
        Vec2::ZERO,
        &camera,
        &label_tree.hit_index,
        DumpFilter::ALL,
    );
    assert_eq!(dumped.len(), labels.len());
    for (dump, element) in dumped.iter().zip(&labels) {
        assert_eq!(dump.edge, element.edge_key);
        assert_eq!(dump.text.as_deref(), Some(element.text.as_str()));
    }

    let pairs = portal_pair_data(&map, &offsets, None, None, None, None, 1.0, &hidden);
    let portal_tree = build_portal_tree_from_pairs(&pairs);
    let markers = dump_portals(
        &portal_tree.tree,
        Vec2::ZERO,
        &camera,
        &portal_tree.hit_index,
        DumpFilter::ALL,
    );
    assert!(!markers.is_empty());
    assert!(markers.iter().all(|m| m.edge == portal_key));
    let endpoints: std::collections::BTreeSet<&str> =
        markers.iter().map(|m| m.endpoint_node_id.as_str()).collect();
    assert_eq!(
        endpoints,
        [portal_key.from_id.as_str(), portal_key.to_id.as_str()]
            .into_iter()
            .collect()
    );
}

/// The two rect relations the `scene` family's overlap and pick
/// answers rest on: touching edges do not overlap, and a union
/// contains both inputs.
#[test]
fn test_scene_rect_overlap_and_union() {
    use crate::mindmap::scene_dump::SceneRect;

    let a = SceneRect::new(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0));
    let touching = SceneRect::new(Vec2::new(10.0, 0.0), Vec2::new(5.0, 5.0));
    let inside = SceneRect::new(Vec2::new(2.0, 2.0), Vec2::new(2.0, 2.0));
    let apart = SceneRect::new(Vec2::new(-30.0, 40.0), Vec2::new(5.0, 5.0));
    assert!(!a.overlaps(&touching));
    assert!(a.overlaps(&inside) && inside.overlaps(&a));
    assert!(!a.overlaps(&apart));
    let both = a.union(&apart);
    assert_eq!(both, SceneRect::new(Vec2::new(-30.0, 0.0), Vec2::new(40.0, 45.0)));
    assert!(both.contains(Vec2::new(-30.0, 45.0)) && both.contains(Vec2::new(10.0, 0.0)));
    assert!(!both.contains(Vec2::new(10.5, 0.0)));
}

/// A dump serializes as the flat shape `format/ipc.md` documents:
/// the placement's three keys sit beside the element's own.
#[test]
fn test_scene_dump_serializes_flat() {
    let map = testament();
    let tree = build_mindmap_tree(&map);
    let id = a_parent(&map);
    let camera = camera_on(&map, &id);
    let value =
        serde_json::to_value(dump_node(&tree, &map, &camera, &id).expect("drawn")).expect("serializes");
    for key in ["node_id", "folded", "canvas", "surface", "visibility", "sections"] {
        assert!(value.get(key).is_some(), "{key} in {value}");
    }
    assert_eq!(value["visibility"], "visible");
    for key in ["x", "y", "w", "h"] {
        assert!(value["canvas"][key].is_number());
    }
    let section = &value["sections"][0];
    for key in ["section_idx", "canvas", "surface", "visibility", "text"] {
        assert!(section.get(key).is_some(), "{key} in {section}");
    }
}

#[test]
fn test_scene_dump_placement_spans_what_the_renderer_draws() {
    do_scene_dump_placement_spans_what_the_renderer_draws();
}

/// An element is placed by everything the renderer draws for it, each
/// part culled as the renderer culls it: a padded background fill
/// whose box sits just off-screen but whose padding crosses the
/// viewport edge is visible, and so is a text whose main glyph is off
/// the edge while one outline-halo stamp is on it. Each rect is the
/// union of the parts. The camera sees canvas x in `[-800, 0)`, so a
/// box starting at `x = 0` misses it by nothing.
pub fn do_scene_dump_placement_spans_what_the_renderer_draws() {
    use crate::gfx_structs::area::{EdgePadding, GlyphArea};
    use crate::gfx_structs::area_fields::OutlineStyle;

    let mut camera = Camera2D::new(800, 600);
    camera.position = Vec2::new(-400.0, 0.0);
    let boxed = |area: &GlyphArea| {
        Placement::of_rect(
            area.position(),
            Vec2::new(100.0, 40.0),
            area.zoom_visibility,
            &camera,
        )
    };

    let mut filled = GlyphArea::new(16.0, 16.0, Vec2::ZERO, Vec2::new(100.0, 40.0));
    filled.background_color = Some([1, 2, 3, 255]);
    filled.background_padding = EdgePadding::new(5.0, 6.0, 7.0, 8.0);
    assert_eq!(
        boxed(&filled).visibility,
        Visibility::Offscreen,
        "precondition: the bare box misses"
    );
    let placed = Placement::of_area(&filled, Vec2::ZERO, &camera);
    assert_eq!(
        placed.visibility,
        Visibility::Visible,
        "the padding is drawn, and it is on screen"
    );
    assert_eq!(
        (placed.canvas.x, placed.canvas.y, placed.canvas.w, placed.canvas.h),
        (-8.0, -5.0, 114.0, 52.0),
        "the fill grown per edge: left 8, top 5, right 6, bottom 7"
    );
    assert_eq!(
        filled.background_rect(),
        Some((Vec2::new(-8.0, -5.0), Vec2::new(114.0, 52.0))),
        "the rect the renderer's fill extraction reads"
    );

    let mut haloed = GlyphArea::new_with_str("x", 16.0, 16.0, Vec2::ZERO, Vec2::new(100.0, 40.0));
    haloed.outline = Some(OutlineStyle {
        color: [0, 0, 0, 255],
        px: 3.0,
    });
    assert_eq!(
        boxed(&haloed).visibility,
        Visibility::Offscreen,
        "precondition: the main glyph misses"
    );
    let placed = Placement::of_area(&haloed, Vec2::ZERO, &camera);
    assert_eq!(
        placed.visibility,
        Visibility::Visible,
        "the leftward halo stamp is drawn on screen"
    );
    assert_eq!(
        (placed.canvas.x, placed.canvas.w),
        (-3.0, 106.0),
        "every stamp, both sides"
    );
    assert_eq!(
        haloed.text_stamp_offsets().count(),
        9,
        "eight halo stamps and the main glyph"
    );

    let bare = GlyphArea::new(16.0, 16.0, Vec2::new(-200.0, 0.0), Vec2::new(100.0, 40.0));
    assert_eq!(bare.text_stamp_offsets().count(), 0);
    assert_eq!(
        Placement::of_area(&bare, Vec2::ZERO, &camera),
        boxed(&bare),
        "an area that draws nothing is placed by its box"
    );
}

#[test]
fn test_scene_dump_filter_keeps_before_it_resolves_text() {
    do_scene_dump_filter_keeps_before_it_resolves_text();
}

/// [`DumpFilter::VISIBLE`] keeps exactly the visible part of the full
/// dump — a node when it or a section is visible, with only its visible
/// sections — and [`DumpFilter::VISIBLE_IDS`] keeps the same elements
/// with no text resolved.
pub fn do_scene_dump_filter_keeps_before_it_resolves_text() {
    let map = testament();
    let tree = build_mindmap_tree(&map);
    let camera = camera_on(&map, &a_parent(&map));
    let all = dump_nodes(&tree, &map, &camera, DumpFilter::ALL);
    let mut expected = all.clone();
    expected.retain_mut(|node| {
        node.sections.retain(|s| s.placement.visibility.is_visible());
        node.placement.visibility.is_visible() || !node.sections.is_empty()
    });
    assert!(
        !expected.is_empty() && expected.len() < all.len(),
        "the camera shows part of the map"
    );

    let visible = dump_nodes(&tree, &map, &camera, DumpFilter::VISIBLE);
    assert_eq!(visible, expected);
    let ids = dump_nodes(&tree, &map, &camera, DumpFilter::VISIBLE_IDS);
    let untexted: Vec<(String, Vec<usize>)> = ids
        .iter()
        .map(|n| {
            (
                n.node_id.clone(),
                n.sections.iter().map(|s| s.section_idx).collect(),
            )
        })
        .collect();
    let texted: Vec<(String, Vec<usize>)> = visible
        .iter()
        .map(|n| {
            (
                n.node_id.clone(),
                n.sections.iter().map(|s| s.section_idx).collect(),
            )
        })
        .collect();
    assert_eq!(untexted, texted);
    assert!(ids.iter().flat_map(|n| &n.sections).all(|s| s.text.is_none()));
    assert!(visible.iter().flat_map(|n| &n.sections).all(|s| s.text.is_some()));
}
