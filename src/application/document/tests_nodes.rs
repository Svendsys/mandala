// SPDX-License-Identifier: MPL-2.0

//! Node text / background / border / text-colour / font-size setters + set_node_style_field helper.
//!
//! Part of the tests split for `document`. Helpers live in
//! `tests_common`; only the tests for this theme live here.
use super::*;
use super::tests_common::{
    first_testament_edge_ref, first_testament_node_id, load_test_doc, load_test_tree,
    pick_test_edge, test_map_path,
};

use baumhard::gfx_structs::area::GlyphAreaCommand;
use baumhard::gfx_structs::mutator::Mutation;
use baumhard::mindmap::animation::{AnimationTiming, Easing};
use baumhard::mindmap::custom_mutation::{
    apply_mutations_to_element, CustomMutation as CM, DocumentAction,
    MutationBehavior as MB, PlatformContext as PC, TargetScope as TS,
    Trigger as Tr, TriggerBinding as TB,
};
use baumhard::mindmap::model::{
    Canvas, GlyphConnectionConfig, MindEdge, MindNode, NodeLayout, NodeStyle, Position, Size,
    TextRun, PORTAL_GLYPH_PRESETS,
};
use baumhard::mindmap::scene_builder::EdgeHandleKind;
use baumhard::mindmap::model::ControlPoint;
use baumhard::util::grapheme_chad::count_grapheme_clusters;
use glam::Vec2;

use super::defaults::default_cross_link_edge;


    #[test]
    fn test_set_node_text_updates_text_and_collapses_runs() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let changed = doc.set_node_text(&nid, "Hello world".to_string());
        assert!(changed);
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert_eq!(node.text, "Hello world");
        assert_eq!(node.text_runs.len(), 1);
        assert_eq!(node.text_runs[0].start, 0);
        assert_eq!(node.text_runs[0].end, count_grapheme_clusters("Hello world"));
        assert!(doc.dirty);
        assert!(matches!(
            doc.undo_stack.last(),
            Some(UndoAction::EditNodeText { .. })
        ));
    }

    #[test]
    fn test_set_node_text_noop_on_unchanged() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let current = doc.mindmap.nodes.get(&nid).unwrap().text.clone();
        doc.undo_stack.clear();
        doc.dirty = false;
        let changed = doc.set_node_text(&nid, current);
        assert!(!changed);
        assert!(doc.undo_stack.is_empty());
        assert!(!doc.dirty);
    }

    #[test]
    fn test_set_node_text_undo_round_trip() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let before_text = doc.mindmap.nodes.get(&nid).unwrap().text.clone();
        let before_runs_len = doc.mindmap.nodes.get(&nid).unwrap().text_runs.len();
        let before_first_run_color = doc
            .mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .text_runs
            .first()
            .map(|r| r.color.clone());
        assert!(doc.set_node_text(&nid, "mutated".to_string()));
        assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().text, "mutated");
        assert!(doc.undo());
        let restored = doc.mindmap.nodes.get(&nid).unwrap();
        assert_eq!(restored.text, before_text);
        // TextRun doesn't implement PartialEq, so compare the parts
        // we care about: count + first run's color.
        assert_eq!(restored.text_runs.len(), before_runs_len);
        assert_eq!(
            restored.text_runs.first().map(|r| r.color.clone()),
            before_first_run_color
        );
    }

    #[test]
    fn test_set_node_text_multiline_with_newlines() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        assert!(doc.set_node_text(&nid, "line 1\nline 2\nline 3".to_string()));
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert_eq!(node.text, "line 1\nline 2\nline 3");
        // Collapsed single run spans the full char count, including newlines.
        assert_eq!(node.text_runs.len(), 1);
        assert_eq!(node.text_runs[0].end, count_grapheme_clusters("line 1\nline 2\nline 3"));
    }

    #[test]
    fn test_set_node_text_unknown_id_returns_false() {
        let mut doc = load_test_doc();
        doc.undo_stack.clear();
        doc.dirty = false;
        assert!(!doc.set_node_text("nonexistent-id", "x".to_string()));
        assert!(doc.undo_stack.is_empty());
        assert!(!doc.dirty);
    }

    #[test]
    fn test_set_node_text_inherits_first_run_formatting() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        // Force a specific first-run formatting we can check for.
        {
            let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
            if node.text_runs.is_empty() {
                node.text_runs.push(TextRun {
                    start: 0,
                    end: count_grapheme_clusters(&node.text),
                    bold: false,
                    italic: false,
                    underline: false,
                    font: "LiberationSans".to_string(),
                    size_pt: 24,
                    color: "#ffffff".to_string(),
                    hyperlink: None,
                });
            }
            node.text_runs[0].bold = true;
            node.text_runs[0].color = "#abcdef".to_string();
            node.text_runs[0].size_pt = 33;
        }
        assert!(doc.set_node_text(&nid, "rewritten".to_string()));
        let run = &doc.mindmap.nodes.get(&nid).unwrap().text_runs[0];
        assert!(run.bold);
        assert_eq!(run.color, "#abcdef");
        assert_eq!(run.size_pt, 33);
    }

    // -----------------------------------------------------------------
    // Node style setters (bg / border / text color, font size)
    // -----------------------------------------------------------------

    #[test]
    fn test_set_node_bg_color_round_trips_through_undo() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let before = doc.mindmap.nodes.get(&nid).unwrap().style.background_color.clone();
        assert!(doc.set_node_bg_color(&nid, "#123456".to_string()));
        assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().style.background_color, "#123456");
        assert!(matches!(
            doc.undo_stack.last(),
            Some(UndoAction::EditNodeStyle { .. })
        ));
        assert!(doc.undo());
        assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().style.background_color, before);
    }

    #[test]
    fn test_set_node_bg_color_unchanged_is_noop() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let current = doc.mindmap.nodes.get(&nid).unwrap().style.background_color.clone();
        doc.undo_stack.clear();
        doc.dirty = false;
        assert!(!doc.set_node_bg_color(&nid, current));
        assert!(doc.undo_stack.is_empty());
        assert!(!doc.dirty);
    }

    #[test]
    fn test_set_node_border_color_writes_frame_color() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        assert!(doc.set_node_border_color(&nid, "#ff00ff".to_string()));
        assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().style.frame_color, "#ff00ff");
    }

    /// Setting text color rewrites `style.text_color` and every run
    /// whose color matched the pre-edit default. A run the user
    /// colored by hand (mismatched) keeps its override.
    #[test]
    fn test_set_node_text_color_preserves_per_run_overrides() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        // Seed the node with a known default and two runs: one
        // matching the default, one hand-colored.
        {
            let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
            node.style.text_color = "#dddddd".into();
            node.text_runs = vec![
                TextRun {
                    start: 0, end: 3,
                    bold: false, italic: false, underline: false,
                    font: "LiberationSans".into(), size_pt: 24,
                    color: "#dddddd".into(), // matches default
                    hyperlink: None,
                },
                TextRun {
                    start: 3, end: 6,
                    bold: false, italic: false, underline: false,
                    font: "LiberationSans".into(), size_pt: 24,
                    color: "#abcdef".into(), // user override
                    hyperlink: None,
                },
            ];
        }
        assert!(doc.set_node_text_color(&nid, "#111111".into()));
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert_eq!(node.style.text_color, "#111111");
        assert_eq!(node.text_runs[0].color, "#111111", "default-following run should update");
        assert_eq!(node.text_runs[1].color, "#abcdef", "per-run override should be preserved");
    }

    #[test]
    fn test_set_node_text_color_round_trips_through_undo() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        {
            let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
            node.style.text_color = "#dddddd".into();
            for run in node.text_runs.iter_mut() {
                run.color = "#dddddd".into();
            }
        }
        let before_default = doc.mindmap.nodes.get(&nid).unwrap().style.text_color.clone();
        let before_run_colors: Vec<String> = doc
            .mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .text_runs
            .iter()
            .map(|r| r.color.clone())
            .collect();
        assert!(doc.set_node_text_color(&nid, "#222222".into()));
        assert!(doc.undo());
        let restored = doc.mindmap.nodes.get(&nid).unwrap();
        assert_eq!(restored.style.text_color, before_default);
        let restored_colors: Vec<String> =
            restored.text_runs.iter().map(|r| r.color.clone()).collect();
        assert_eq!(restored_colors, before_run_colors);
    }

    #[test]
    fn test_set_node_font_size_writes_all_runs_and_round_trips() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let before_sizes: Vec<u32> = doc
            .mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .text_runs
            .iter()
            .map(|r| r.size_pt)
            .collect();
        assert!(doc.set_node_font_size(&nid, 48.0));
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert!(node.text_runs.iter().all(|r| r.size_pt == 48));
        assert!(doc.undo());
        let after_sizes: Vec<u32> = doc
            .mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .text_runs
            .iter()
            .map(|r| r.size_pt)
            .collect();
        assert_eq!(after_sizes, before_sizes);
    }

    #[test]
    fn test_set_node_font_size_clamps_below_one() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        assert!(doc.set_node_font_size(&nid, 0.5));
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert!(node.text_runs.iter().all(|r| r.size_pt == 1));
    }

    #[test]
    fn test_set_node_style_unknown_id_returns_false() {
        let mut doc = load_test_doc();
        doc.undo_stack.clear();
        doc.dirty = false;
        assert!(!doc.set_node_bg_color("nope", "#000".into()));
        assert!(!doc.set_node_border_color("nope", "#000".into()));
        assert!(!doc.set_node_text_color("nope", "#000".into()));
        assert!(!doc.set_node_font_size("nope", 10.0));
        assert!(!doc.set_node_font_family("nope", Some("Norse")));
        assert!(doc.undo_stack.is_empty());
        assert!(!doc.dirty);
    }

    #[test]
    fn test_set_node_font_family_writes_all_runs_and_round_trips() {
        baumhard::font::fonts::init();
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let before_fonts: Vec<String> = doc
            .mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .text_runs
            .iter()
            .map(|r| r.font.clone())
            .collect();
        // Pick a loaded family that doesn't already match every
        // existing run — keeps the test self-healing against
        // future fixture changes.
        let target = baumhard::font::fonts::loaded_families_iter()
            .find(|f| !before_fonts.iter().any(|b| b == f))
            .map(str::to_string)
            .expect("at least one loaded family must differ from the fixture");
        assert!(doc.set_node_font_family(&nid, Some(&target)));
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert!(node.text_runs.iter().all(|r| r.font == target));
        // Idempotent re-set is a no-op.
        let stack_len = doc.undo_stack.len();
        assert!(!doc.set_node_font_family(&nid, Some(&target)));
        assert_eq!(doc.undo_stack.len(), stack_len);
        // Undo restores the prior heterogeneous state.
        assert!(doc.undo());
        let after_fonts: Vec<String> = doc
            .mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .text_runs
            .iter()
            .map(|r| r.font.clone())
            .collect();
        assert_eq!(after_fonts, before_fonts);
    }

    #[test]
    fn test_set_node_font_family_none_clears_every_run() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        // Pin the runs to a known family first so the clear has
        // something to clear.
        baumhard::font::fonts::init();
        let target = baumhard::font::fonts::loaded_families_iter()
            .next()
            .map(str::to_string)
            .expect("at least one loaded family");
        assert!(doc.set_node_font_family(&nid, Some(&target)));
        // Now clear with None — every run should hold the empty
        // sentinel that the tree builder reads as "use default".
        assert!(doc.set_node_font_family(&nid, None));
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert!(node.text_runs.iter().all(|r| r.font.is_empty()));
        // Re-clear is a no-op.
        let stack_len = doc.undo_stack.len();
        assert!(!doc.set_node_font_family(&nid, None));
        assert_eq!(doc.undo_stack.len(), stack_len);
    }

    /// `grow_node_sizes_to_fit_borders` runs at finalize so a
    /// map loaded with a wide static side pattern on a tiny node
    /// grows the node automatically — the same monotonic posture
    /// as `grow_node_sizes_to_fit_text`. Without this floor the
    /// renderer would clip the static prefix at load time.
    #[test]
    fn finalize_grows_nodes_to_fit_border_static_parts() {
        use std::collections::HashMap;
        use baumhard::mindmap::model::{
            Canvas, CustomBorderGlyphs, GlyphBorderConfig, MindMap,
        };

        let mut nodes = HashMap::new();
        let style = NodeStyle {
            background_color: "#000".into(),
            frame_color: "#fff".into(),
            text_color: "#fff".into(),
            shape: "rectangle".into(),
            corner_radius_percent: 0.0,
            frame_thickness: 1.0,
            show_frame: true,
            show_shadow: false,
            border: Some(GlyphBorderConfig {
                preset: "custom".into(),
                font: None,
                font_size_pt: 14.0,
                color: None,
                glyphs: Some(CustomBorderGlyphs {
                    top: "##########(*)##########".into(),
                    bottom: "-".into(),
                    left: "|".into(),
                    right: "|".into(),
                    top_left: "<".into(),
                    top_right: ">".into(),
                    bottom_left: "<".into(),
                    bottom_right: ">".into(),
                }),
                padding: 4.0,
                color_palette: None,
                color_palette_field: None,
            }),
        };
        nodes.insert(
            "0".into(),
            MindNode {
                id: "0".into(),
                parent_id: None,
                position: Position { x: 0.0, y: 0.0 },
                size: Size { width: 5.0, height: 5.0 },
                text: "n".into(),
                text_runs: vec![],
                style,
                layout: NodeLayout {
                    layout_type: "map".into(),
                    direction: "auto".into(),
                    spacing: 0.0,
                },
                folded: false,
                notes: String::new(),
                color_schema: None,
                channel: 0,
                trigger_bindings: vec![],
                inline_mutations: vec![],
                min_zoom_to_render: None,
                max_zoom_to_render: None,
            },
        );
        let map = MindMap {
            version: "1.0".into(),
            name: "fixture".into(),
            canvas: Canvas {
                background_color: "#000".into(),
                default_border: None,
                default_connection: None,
                theme_variables: HashMap::new(),
                theme_variants: HashMap::new(),
            },
            palettes: HashMap::new(),
            nodes,
            edges: vec![],
            custom_mutations: vec![],
        };
        // Round-trip through JSON to exercise the finalize hook
        // — `MindMapDocument::from_json_str` calls `finalize`,
        // which runs both grow passes. Direct construction skips
        // it.
        let json = serde_json::to_string(&map).expect("serialises");
        let doc = MindMapDocument::from_json_str(&json, None)
            .expect("loads through finalize");
        let n = doc.mindmap.nodes.get("0").expect("node 0 exists");
        assert!(
            n.size.width > 5.0,
            "load-time floor must grow the box to fit the border statics; \
             got width={}",
            n.size.width,
        );
    }
