// SPDX-License-Identifier: MPL-2.0

//! Section-specific tree-builder tests — guards the post-section
//! refactor's invariants:
//!
//! - Each `MindNode` produces one container `GlyphArea` (chrome
//!   only, empty text) plus one section-area + one section-model
//!   per [`MindSection`](crate::mindmap::model::MindSection).
//! - `MindMapTree.section_map` keys on `(mind_id, section_idx)`
//!   and resolves to the section-area's arena `NodeId`.
//! - Section-areas are flagged `Flag::SectionRoot`; the container
//!   is not.
//! - Multi-section nodes append section-areas in `MindSection`
//!   order; channels default to the section's index when authored
//!   `MindSection.channel == 0` and idx > 0.
//! - The `owning_mind_id` climb on the tree returns the parent
//!   MindNode whether the start arena id is a container, a
//!   section-area, or a section-model.

use super::super::*;
use super::fixtures::*;
use crate::core::primitives::{Flag, Flaggable};
use crate::gfx_structs::element::GfxElementType;
use crate::mindmap::model::{MindSection, Position, Size};

#[test]
fn test_container_is_empty_text_section_carries_glyphs() {
    let mut node = synthetic_node("n", None, 0.0, 0.0);
    node.sections[0].text = "hello".into();
    let map = synthetic_map(vec![node], vec![]);
    let result = build_mindmap_tree(&map);

    let container_id = result.arena_id_for("n").unwrap();
    let container = result.tree.arena.get(container_id).unwrap().get();
    let container_area = container.glyph_area().expect("container is a GlyphArea");
    assert!(
        container_area.text.is_empty(),
        "container area must carry no glyphs"
    );
    assert!(
        !container.flag_is_set(Flag::SectionRoot),
        "container must NOT be flagged SectionRoot"
    );

    let section_id = result.section_arena_id("n", 0).unwrap();
    let section = result.tree.arena.get(section_id).unwrap().get();
    let section_area = section.glyph_area().expect("section-area is a GlyphArea");
    assert_eq!(section_area.text, "hello");
    assert!(
        section.flag_is_set(Flag::SectionRoot),
        "section-area must be flagged SectionRoot"
    );
}

#[test]
fn test_section_model_is_glyph_model_child_of_section_area() {
    let map = synthetic_map(vec![synthetic_node("n", None, 0.0, 0.0)], vec![]);
    let result = build_mindmap_tree(&map);

    let section_id = result.section_arena_id("n", 0).unwrap();
    let model_id = section_id
        .children(&result.tree.arena)
        .next()
        .expect("section-area has a model child");
    let model_element = result.tree.arena.get(model_id).unwrap().get();
    assert_eq!(
        model_element.get_type(),
        GfxElementType::GlyphModel,
        "section-model is a GlyphModel"
    );
    // Model inherits SectionRoot for fast climbs.
    assert!(model_element.flag_is_set(Flag::SectionRoot));
}

#[test]
fn test_multi_section_node_emits_one_subtree_per_section() {
    let mut node = synthetic_node("n", None, 0.0, 0.0);
    node.sections = vec![
        MindSection::new_default("first".into(), vec![]),
        MindSection::new_default("second".into(), vec![]),
        MindSection::new_default("third".into(), vec![]),
    ];
    let map = synthetic_map(vec![node], vec![]);
    let result = build_mindmap_tree(&map);

    for (idx, expected) in ["first", "second", "third"].iter().enumerate() {
        let section_id = result
            .section_map
            .get(&("n".to_string(), idx))
            .unwrap_or_else(|| panic!("section {} missing from section_map", idx));
        let area = result
            .tree
            .arena
            .get(*section_id)
            .unwrap()
            .get()
            .glyph_area()
            .unwrap();
        assert_eq!(area.text, *expected);
    }
}

#[test]
fn test_section_offset_and_size_resolve_to_absolute_aabb() {
    // Place a section at offset (10, 20) with explicit size 50x30
    // inside a 200x200 node at canvas position (100, 200). The
    // section-area should land at (110, 220) with bounds 50x30.
    let mut node = synthetic_node("n", None, 100.0, 200.0);
    node.size = Size {
        width: 200.0,
        height: 200.0,
    };
    node.sections[0].offset = Position { x: 10.0, y: 20.0 };
    node.sections[0].size = Some(Size {
        width: 50.0,
        height: 30.0,
    });
    let map = synthetic_map(vec![node], vec![]);
    let result = build_mindmap_tree(&map);

    let section_id = result.section_arena_id("n", 0).unwrap();
    let area = result
        .tree
        .arena
        .get(section_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap();
    assert!((area.position.x.0 - 110.0).abs() < 1e-3);
    assert!((area.position.y.0 - 220.0).abs() < 1e-3);
    assert!((area.render_bounds.x.0 - 50.0).abs() < 1e-3);
    assert!((area.render_bounds.y.0 - 30.0).abs() < 1e-3);
}

#[test]
fn test_section_size_none_inherits_node_aabb() {
    // The default migration shape: section.size = None → fill the
    // parent node's bounds.
    let mut node = synthetic_node("n", None, 50.0, 60.0);
    node.size = Size {
        width: 320.0,
        height: 100.0,
    };
    assert!(node.sections[0].size.is_none());
    let map = synthetic_map(vec![node], vec![]);
    let result = build_mindmap_tree(&map);

    let section_id = result.section_arena_id("n", 0).unwrap();
    let area = result
        .tree
        .arena
        .get(section_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap();
    assert!((area.render_bounds.x.0 - 320.0).abs() < 1e-3);
    assert!((area.render_bounds.y.0 - 100.0).abs() < 1e-3);
}

#[test]
fn test_owning_mind_id_climbs_from_section_area_and_model() {
    let map = synthetic_map(vec![synthetic_node("n", None, 0.0, 0.0)], vec![]);
    let result = build_mindmap_tree(&map);

    let container_id = result.arena_id_for("n").unwrap();
    let section_id = result.section_arena_id("n", 0).unwrap();
    let model_id = section_id
        .children(&result.tree.arena)
        .next()
        .expect("section-model present");

    assert_eq!(result.owning_mind_id(container_id), Some("n"));
    assert_eq!(result.owning_mind_id(section_id), Some("n"));
    assert_eq!(result.owning_mind_id(model_id), Some("n"));
}

#[test]
fn test_section_for_node_returns_index_only_for_section_areas() {
    let mut node = synthetic_node("n", None, 0.0, 0.0);
    node.sections = vec![
        MindSection::new_default("a".into(), vec![]),
        MindSection::new_default("b".into(), vec![]),
    ];
    let map = synthetic_map(vec![node], vec![]);
    let result = build_mindmap_tree(&map);

    let container_id = result.arena_id_for("n").unwrap();
    assert_eq!(
        result.section_for_node(container_id),
        None,
        "container is not a section"
    );

    for idx in 0..2 {
        let section_id = result.section_arena_id("n", idx).unwrap();
        assert_eq!(result.section_for_node(section_id), Some(("n", idx)));
    }
}

#[test]
fn test_default_section_channel_falls_through_to_index() {
    // Three sections, all left at the serde default channel 0:
    // the tree builder substitutes the index for sections idx>0,
    // so each section gets a unique channel without explicit
    // authoring.
    let mut node = synthetic_node("n", None, 0.0, 0.0);
    node.sections = vec![
        MindSection::new_default("a".into(), vec![]),
        MindSection::new_default("b".into(), vec![]),
        MindSection::new_default("c".into(), vec![]),
    ];
    let map = synthetic_map(vec![node], vec![]);
    let result = build_mindmap_tree(&map);

    use crate::gfx_structs::tree::BranchChannel;
    for (idx, expected_channel) in [(0usize, 0usize), (1, 1), (2, 2)] {
        let section_id = result.section_arena_id("n", idx).unwrap();
        let element = result.tree.arena.get(section_id).unwrap().get();
        assert_eq!(
            element.channel(),
            expected_channel,
            "section idx={} expected channel={}",
            idx,
            expected_channel
        );
    }
}

/// `section_count_for` reports per-mind-id section counts so the
/// hit-test single-section-fold heuristic doesn't have to walk
/// the arena per click. Empty / missing nodes report 0.
#[test]
fn test_section_count_for_reports_authored_count() {
    let mut single = synthetic_node("single", None, 0.0, 0.0);
    let _ = &mut single;
    let mut multi = synthetic_node("multi", None, 100.0, 0.0);
    multi.sections = vec![
        MindSection::new_default("a".into(), vec![]),
        MindSection::new_default("b".into(), vec![]),
        MindSection::new_default("c".into(), vec![]),
    ];
    let map = synthetic_map(vec![single, multi], vec![]);
    let result = build_mindmap_tree(&map);
    assert_eq!(result.section_count_for("single"), 1);
    assert_eq!(result.section_count_for("multi"), 3);
    assert_eq!(result.section_count_for("does-not-exist"), 0);
}

/// Section text content emits as a `TextElement` with a stable
/// `section_idx` matching its position in `MindNode.sections`.
/// Pins the multi-section path through the scene builder that
/// today's single-section fixtures don't reach.
#[test]
fn test_multi_section_emits_distinct_text_elements() {
    use crate::mindmap::scene_builder::build_scene;
    let mut node = synthetic_node("n", None, 0.0, 0.0);
    node.sections = vec![
        MindSection::new_default("first".into(), vec![]),
        MindSection::new_default("second".into(), vec![]),
        MindSection::new_default("".into(), vec![]), // empty → no element
    ];
    let map = synthetic_map(vec![node], vec![]);
    let scene = build_scene(&map, 1.0);
    assert_eq!(scene.text_elements.len(), 2);
    let by_idx: std::collections::HashMap<usize, &str> = scene
        .text_elements
        .iter()
        .map(|e| (e.section_idx, e.text.as_str()))
        .collect();
    assert_eq!(by_idx.get(&0), Some(&"first"));
    assert_eq!(by_idx.get(&1), Some(&"second"));
    assert!(!by_idx.contains_key(&2), "empty section emits nothing");
}
