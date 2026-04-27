// SPDX-License-Identifier: MPL-2.0

//! Border tree builder tests — void-per-framed, frame filters, drag offset, theme resolution, stable channels, mutator round-trip, identity sequence.

use super::super::*;
use super::fixtures::*;
use crate::mindmap::border::{BORDER_APPROX_CHAR_WIDTH_FRAC, BORDER_CORNER_OVERLAP_FRAC};

#[test]
fn border_tree_has_one_void_parent_per_framed_node() {
    let map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 200.0, 0.0),
        ],
        vec![],
    );
    let tree = build_border_tree(&map, &HashMap::new());
    // Two framed nodes → two per-node Void parents under root.
    let parents: Vec<NodeId> = tree.root.children(&tree.arena).collect();
    assert_eq!(parents.len(), 2);
    for parent in parents {
        let element = tree.arena.get(parent).unwrap().get();
        assert!(element.glyph_area().is_none(), "per-node parent is Void");
        // Every parent has exactly 4 GlyphArea run children.
        let runs: Vec<NodeId> = parent.children(&tree.arena).collect();
        assert_eq!(runs.len(), 4);
        for run_id in runs {
            let run = tree.arena.get(run_id).unwrap().get();
            assert!(run.glyph_area().is_some(), "run is a GlyphArea");
        }
    }
}

#[test]
fn border_tree_skips_nodes_with_show_frame_false() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 200.0, 0.0),
        ],
        vec![],
    );
    map.nodes.get_mut("a").unwrap().style.show_frame = false;
    let tree = build_border_tree(&map, &HashMap::new());
    // Only `b` is framed → one per-node parent.
    let parents: Vec<NodeId> = tree.root.children(&tree.arena).collect();
    assert_eq!(parents.len(), 1);
}

#[test]
fn border_tree_skips_folded_nodes() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("parent", None, 0.0, 0.0),
            synthetic_node("child", Some("parent"), 0.0, 100.0),
        ],
        vec![],
    );
    map.nodes.get_mut("parent").unwrap().folded = true;
    let tree = build_border_tree(&map, &HashMap::new());
    // Parent itself still frames; child is hidden.
    let parents: Vec<NodeId> = tree.root.children(&tree.arena).collect();
    assert_eq!(parents.len(), 1);
}

#[test]
fn border_tree_applies_drag_offset() {
    let map = synthetic_map(vec![synthetic_node("a", None, 0.0, 0.0)], vec![]);
    let mut offsets: HashMap<String, (f32, f32)> = HashMap::new();
    offsets.insert("a".into(), (50.0, 25.0));
    let tree = build_border_tree(&map, &offsets);
    // Drag offset must show up on the *top* run's position.x
    // (which is `pos_x - approx_char_width`).
    let parent = tree.root.children(&tree.arena).next().unwrap();
    let top_run = parent.children(&tree.arena).next().unwrap();
    let area = tree
        .arena
        .get(top_run)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap();
    // pos_x + offset = 0 + 50 = 50, then shifted by
    // -approx_char_width (0.6 * font_size).
    let font_size = 14.0_f32;
    let approx_char_width = font_size * BORDER_APPROX_CHAR_WIDTH_FRAC;
    let expected_x = 50.0 - approx_char_width;
    assert!(
        (area.position.x.0 - expected_x).abs() < 0.001,
        "top-run x ({}) should match drag-applied layout ({})",
        area.position.x.0,
        expected_x
    );
    // y follows pos_y + offset - font_size + corner_overlap.
    let corner_overlap = font_size * BORDER_CORNER_OVERLAP_FRAC;
    let expected_y = 25.0 - font_size + corner_overlap;
    assert!((area.position.y.0 - expected_y).abs() < 0.001);
}

#[test]
fn border_tree_resolves_frame_color_through_theme_vars() {
    let mut map = synthetic_map(vec![synthetic_node("a", None, 0.0, 0.0)], vec![]);
    // Theme variable keys include the leading `--`, matching
    // the CSS-ish `var(--name)` syntax used in mindmap JSON.
    map.canvas
        .theme_variables
        .insert("--my-frame".into(), "#ff0000".into());
    map.nodes.get_mut("a").unwrap().style.frame_color = "var(--my-frame)".into();
    let tree = build_border_tree(&map, &HashMap::new());
    let parent = tree.root.children(&tree.arena).next().unwrap();
    let top_run = parent.children(&tree.arena).next().unwrap();
    let area = tree
        .arena
        .get(top_run)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap();
    let region = area.regions.all_regions()[0];
    let c = region.color.unwrap();
    // #ff0000 → red channel 1.0, green/blue 0.0.
    assert!((c[0] - 1.0).abs() < 0.01);
    assert!(c[1] < 0.01);
    assert!(c[2] < 0.01);
}

#[test]
fn border_tree_run_channels_are_stable_1_to_4() {
    // Top=1, Bottom=2, Left=3, Right=4. Stability matters
    // because mutator trees target runs by channel.
    use crate::gfx_structs::tree::BranchChannel;
    let map = synthetic_map(vec![synthetic_node("a", None, 0.0, 0.0)], vec![]);
    let tree = build_border_tree(&map, &HashMap::new());
    let parent = tree.root.children(&tree.arena).next().unwrap();
    let runs: Vec<_> = parent.children(&tree.arena).collect();
    let channels: Vec<usize> = runs
        .iter()
        .map(|id| tree.arena.get(*id).unwrap().get().channel())
        .collect();
    assert_eq!(channels, vec![1, 2, 3, 4]);
}

/// Per-node Void parents use the 1-based sorted index as
/// their channel, not a monotonic counter. Stability across
/// rebuilds is the prerequisite for the in-place mutator
/// path: `align_child_walks` matches mutator children to
/// target children by ascending channel, so two consecutive
/// `border_node_data` calls with the same identity must emit
/// the same channel set.
#[test]
fn border_parent_channels_are_sorted_index_based() {
    use crate::gfx_structs::tree::BranchChannel;
    // Three framed nodes; lexicographic order is a, b, c.
    let map = synthetic_map(
        vec![
            synthetic_node("c", None, 0.0, 0.0),
            synthetic_node("a", None, 100.0, 0.0),
            synthetic_node("b", None, 200.0, 0.0),
        ],
        vec![],
    );
    let tree = build_border_tree(&map, &HashMap::new());
    let parents: Vec<_> = tree.root.children(&tree.arena).collect();
    let channels: Vec<usize> = parents
        .iter()
        .map(|id| tree.arena.get(*id).unwrap().get().channel())
        .collect();
    assert_eq!(channels, vec![1, 2, 3]);
}

/// Round-trip: build a border tree at state A, apply the
/// mutator computed from state B, and the resulting tree's
/// per-channel GlyphAreas must match what
/// `build_border_tree(B)` produced directly. Picks the
/// picker-hover hot path as the canonical case: same nodes,
/// same frame flag, but a drag offset and a color change.
#[test]
fn border_mutator_round_trip_matches_full_rebuild() {
    use crate::core::primitives::Applicable;

    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 200.0, 0.0),
        ],
        vec![],
    );
    // Author a non-default zoom window on one of the nodes so
    // the parity loop actually exercises the border mutator
    // delta's `GlyphAreaField::ZoomVisibility` write (§B2):
    // a regression dropping that field from the assign delta
    // would leave `tree_a`'s four border runs at the unbounded
    // default while a fresh build picks up `{0.5, 2.0}`, and
    // the per-field assertion below trips on `zoom_visibility`.
    if let Some(node_a) = map.nodes.get_mut("a") {
        node_a.min_zoom_to_render = Some(0.5);
        node_a.max_zoom_to_render = Some(2.0);
    }

    // State A: no offsets.
    let mut tree_a = build_border_tree(&map, &HashMap::new());

    // State B: same identity, offset applied to node "a".
    let mut offsets = HashMap::new();
    offsets.insert("a".to_string(), (12.5, -6.0));

    let nodes_b = border_node_data(&map, &offsets);
    let mutator = build_border_mutator_tree_from_nodes(&nodes_b);
    mutator.apply_to(&mut tree_a);

    let expected = build_border_tree(&map, &offsets);

    let actual_parents: Vec<NodeId> =
        tree_a.root.children(&tree_a.arena).collect();
    let expected_parents: Vec<NodeId> =
        expected.root.children(&expected.arena).collect();
    assert_eq!(actual_parents.len(), expected_parents.len());
    // Full-field parity — text / position / bounds / scale /
    // line_height / regions / outline / zoom_visibility — so
    // any silent drift on a mutator-written field surfaces here.
    for (a_p, e_p) in actual_parents.iter().zip(expected_parents.iter()) {
        let a_runs: Vec<NodeId> = a_p.children(&tree_a.arena).collect();
        let e_runs: Vec<NodeId> = e_p.children(&expected.arena).collect();
        assert_eq!(a_runs.len(), e_runs.len());
        for (a, e) in a_runs.iter().zip(e_runs.iter()) {
            let a_area = tree_a.arena.get(*a).unwrap().get().glyph_area().unwrap();
            let e_area = expected.arena.get(*e).unwrap().get().glyph_area().unwrap();
            assert_eq!(a_area.text, e_area.text);
            assert_eq!(a_area.position, e_area.position);
            assert_eq!(a_area.render_bounds, e_area.render_bounds);
            assert_eq!(a_area.scale, e_area.scale);
            assert_eq!(a_area.line_height, e_area.line_height);
            assert_eq!(a_area.regions, e_area.regions);
            assert_eq!(a_area.outline, e_area.outline);
            assert_eq!(a_area.zoom_visibility, e_area.zoom_visibility);
        }
    }
}

/// A node's zoom-visibility window is stamped onto every one
/// of its four border `GlyphArea` runs — top, bottom, left,
/// right — so the frame renders only when the owning node
/// does. Without this assertion, a regression in
/// `BorderNodeData::zoom_visibility` propagation (either at
/// the initial-build stamp site in `border_node_data` or at
/// the `append_border_run` call chain) would ship a node that
/// vanishes above zoom 2× leaving four orphan frame fragments
/// on the canvas.
#[test]
fn border_runs_inherit_owning_node_zoom_visibility() {
    use crate::gfx_structs::zoom_visibility::ZoomVisibility;

    let mut map = synthetic_map(
        vec![synthetic_node("a", None, 0.0, 0.0)],
        vec![],
    );
    let window = ZoomVisibility { min: Some(1.0), max: Some(2.5) };
    if let Some(node) = map.nodes.get_mut("a") {
        node.min_zoom_to_render = Some(1.0);
        node.max_zoom_to_render = Some(2.5);
    }
    let tree = build_border_tree(&map, &HashMap::new());
    let parents: Vec<NodeId> = tree.root.children(&tree.arena).collect();
    assert_eq!(parents.len(), 1, "one framed node → one sub-tree");
    let runs: Vec<NodeId> = parents[0].children(&tree.arena).collect();
    assert_eq!(runs.len(), 4, "border sub-tree has four runs");
    for run in &runs {
        let area = tree.arena.get(*run).unwrap().get().glyph_area().unwrap();
        assert_eq!(area.zoom_visibility, window);
    }
}

/// Default path: a node with no authored window yields border
/// runs whose `zoom_visibility` is unbounded. Guards the
/// zero-cost default so pre-existing maps pay nothing.
#[test]
fn border_runs_default_to_unbounded_when_node_has_no_window() {
    use crate::gfx_structs::zoom_visibility::ZoomVisibility;

    let map = synthetic_map(
        vec![synthetic_node("a", None, 0.0, 0.0)],
        vec![],
    );
    let tree = build_border_tree(&map, &HashMap::new());
    let parents: Vec<NodeId> = tree.root.children(&tree.arena).collect();
    let runs: Vec<NodeId> = parents[0].children(&tree.arena).collect();
    for run in &runs {
        let area = tree.arena.get(*run).unwrap().get().glyph_area().unwrap();
        assert_eq!(area.zoom_visibility, ZoomVisibility::unbounded());
    }
}

/// Toggling `show_frame = false` on a node shifts the
/// identity sequence so the dispatcher in
/// `update_border_tree_with_offsets` falls back to a full
/// rebuild. Without this, applying a mutator against a tree
/// whose parent set has changed would silently misalign.
#[test]
fn border_identity_sequence_changes_on_show_frame_toggle() {
    let mut map = synthetic_map(
        vec![
            synthetic_node("a", None, 0.0, 0.0),
            synthetic_node("b", None, 200.0, 0.0),
        ],
        vec![],
    );
    let before =
        border_identity_sequence(&border_node_data(&map, &HashMap::new()));
    assert_eq!(before, vec!["a".to_string(), "b".to_string()]);

    map.nodes.get_mut("b").unwrap().style.show_frame = false;
    let after =
        border_identity_sequence(&border_node_data(&map, &HashMap::new()));
    assert_eq!(after, vec!["a".to_string()]);
    assert_ne!(before, after);
}

/// `row_count` for the side columns must use `.ceil()` rather
/// than `.round()` — with `.round()`, a node whose `size_y / fs`
/// rounds *down* (e.g. 100/14 = 7.14 → 7 rows = 98 px on a 100 px
/// node) leaves the bottom row's corner cell hanging below the
/// last `│`, rendering as a visible gap at BL/BR.
///
/// We assert structurally rather than by counting characters: the
/// emitted left-column text must contain at least
/// `ceil(size_y / fs)` clusters (separated by `\n`). For
/// nh=100, fs=14 this is 8.
#[test]
fn border_tree_left_column_rows_use_ceil_not_round() {
    let mut map = synthetic_map(vec![synthetic_node("a", None, 0.0, 0.0)], vec![]);
    let node = map.nodes.get_mut("a").unwrap();
    node.size.width = 200.0;
    node.size.height = 100.0;

    let tree = build_border_tree(&map, &HashMap::new());
    // Walk to the per-node Void parent, then to the LEFT column
    // (channel 3 in the tree-builder convention).
    let mut left_col_text: Option<String> = None;
    for parent in tree.root.children(&tree.arena) {
        for run_id in parent.children(&tree.arena) {
            let run = tree.arena.get(run_id).unwrap().get();
            let area = run.glyph_area().unwrap();
            // Channel 3 is the left column (top=1, bottom=2,
            // left=3, right=4). Disambiguate by content shape: a
            // vertical column has internal `\n` separators.
            if area.text.contains('\n') && area.position.x.0 < 50.0 {
                left_col_text = Some(area.text.clone());
                break;
            }
        }
    }
    let text = left_col_text.expect("left column run found in tree");
    let cluster_count = text
        .split('\n')
        .filter(|s| !s.is_empty())
        .count();
    // ceil(100 / 14) = 8 clusters, NOT round(100/14) = 7.
    assert_eq!(
        cluster_count, 8,
        "left column should have ceil(100/14)=8 rows, got {}: '{}'",
        cluster_count, text
    );
}

/// The `append_border_run` helper sizes its `ColorFontRegions`
/// span to the text's grapheme-cluster count, not its codepoint
/// count. Current production BorderGlyphSet only emits ASCII-range
/// single-codepoint chars so the two counts agree there — this
/// test exercises the helper directly with a ZWJ emoji so a
/// future custom-border preset (or a revert to `.chars().count()`)
/// regresses loudly. Mirrors the defensive comment on the
/// grapheme-count site itself.
#[test]
fn append_border_run_region_sized_by_grapheme_cluster_count_not_codepoints() {
    use crate::gfx_structs::tree::Tree;

    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let parent = tree.arena.new_node(GfxElement::new_void_with_id(0, 0));
    tree.root.append(parent, &mut tree.arena);

    super::super::border::append_border_run(
        &mut tree,
        parent,
        1,
        1,
        // 👨‍👩‍👧 — 5 codepoints joined by ZWJ, 1 grapheme cluster.
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        12.0,
        (0.0, 0.0),
        (100.0, 20.0),
        [1.0, 1.0, 1.0, 1.0],
        crate::gfx_structs::zoom_visibility::ZoomVisibility::unbounded(),
        &[],
        0,
    );

    let run = parent.children(&tree.arena).next().unwrap();
    let area = tree.arena.get(run).unwrap().get().glyph_area().unwrap();
    let regions = area.regions.all_regions();
    assert_eq!(regions.len(), 1);
    assert_eq!(
        regions[0].range.end - regions[0].range.start,
        1,
        "region must cover 1 grapheme cluster, not 5 codepoints"
    );
}

/// Per-node `GlyphBorderConfig` reaches the tree's emitted text:
/// when the user authors a custom side pattern, the border-tree
/// builder's resolved style consumes it through
/// `BorderStyle::top_text` / `bottom_text` etc. Pre-fix, the
/// builder ignored `node.style.border` entirely; this test fails
/// loudly on a regression to that behaviour.
#[test]
fn border_tree_honors_custom_side_pattern() {
    use crate::mindmap::model::{CustomBorderGlyphs, GlyphBorderConfig};

    let mut map = synthetic_map(vec![synthetic_node("a", None, 0.0, 0.0)], vec![]);
    let node = map.nodes.get_mut("a").unwrap();
    // Wide enough that the fitter picks at least one full fill
    // iteration; the synthetic node's 80px width × the default
    // ~14pt border font gives a generous cluster budget.
    node.size.width = 400.0;
    node.size.height = 80.0;
    node.style.border = Some(GlyphBorderConfig {
        preset: "custom".into(),
        font: None,
        font_size_pt: 14.0,
        color: None,
        glyphs: Some(CustomBorderGlyphs {
            top: "###(*)###".into(),
            bottom: "+=##=+".into(),
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
    });

    let tree = build_border_tree(&map, &HashMap::new());
    let parent = tree.root.children(&tree.arena).next().unwrap();
    let runs: Vec<_> = parent.children(&tree.arena).collect();
    assert_eq!(runs.len(), 4, "expect top/bottom/left/right runs");

    let top_text = &tree
        .arena
        .get(runs[0])
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .text;
    // Top row starts with '<', ends with '>' (the configured corners),
    // and contains '#' / '*' (the prefix-fill-suffix pattern).
    assert!(top_text.starts_with('<'), "got: '{}'", top_text);
    assert!(top_text.ends_with('>'), "got: '{}'", top_text);
    assert!(top_text.contains('*'), "got: '{}'", top_text);
}

/// Mutator-vs-fresh-build parity when the user changes a side
/// pattern between snapshots — specifically, the in-place
/// `build_border_mutator_tree_from_nodes` path must rewrite each
/// run's `text` field through `GlyphAreaField::Text(...)` so the
/// rendered glyphs match a fresh build's output. A regression
/// dropping the text field from the mutator delta (the kind of
/// bug that survives the no-pattern-change parity test above)
/// shows up here as a `text` mismatch on every run.
#[test]
fn border_mutator_picks_up_pattern_change() {
    use crate::core::primitives::Applicable;
    use crate::mindmap::model::{CustomBorderGlyphs, GlyphBorderConfig};

    let mut map = synthetic_map(
        vec![synthetic_node("a", None, 0.0, 0.0)],
        vec![],
    );
    map.nodes.get_mut("a").unwrap().size.width = 400.0;
    map.nodes.get_mut("a").unwrap().size.height = 80.0;

    // State A: simple atomic-repeat side glyphs.
    map.nodes.get_mut("a").unwrap().style.border = Some(GlyphBorderConfig {
        preset: "custom".into(),
        font: None,
        font_size_pt: 14.0,
        color: None,
        glyphs: Some(CustomBorderGlyphs {
            top: "-".into(),
            bottom: "-".into(),
            left: "|".into(),
            right: "|".into(),
            top_left: "+".into(),
            top_right: "+".into(),
            bottom_left: "+".into(),
            bottom_right: "+".into(),
        }),
        padding: 4.0,
        color_palette: None,
        color_palette_field: None,
    });
    let mut tree_a = build_border_tree(&map, &HashMap::new());

    // State B: same node, new prefix/fill/suffix top pattern.
    map.nodes
        .get_mut("a")
        .unwrap()
        .style
        .border
        .as_mut()
        .unwrap()
        .glyphs
        .as_mut()
        .unwrap()
        .top = "###(*)###".into();

    let nodes_b = border_node_data(&map, &HashMap::new());
    let mutator = build_border_mutator_tree_from_nodes(&nodes_b);
    mutator.apply_to(&mut tree_a);

    let expected = build_border_tree(&map, &HashMap::new());

    // Compare the top run's text on both trees. If the mutator
    // dropped the text field, `tree_a`'s top run would still
    // carry the State-A `+----+` text while `expected` carries
    // the State-B prefix/fill/suffix output.
    let actual_top = tree_a
        .arena
        .get(
            tree_a
                .root
                .children(&tree_a.arena)
                .next()
                .unwrap()
                .children(&tree_a.arena)
                .next()
                .unwrap(),
        )
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .text
        .clone();
    let expected_top = expected
        .arena
        .get(
            expected
                .root
                .children(&expected.arena)
                .next()
                .unwrap()
                .children(&expected.arena)
                .next()
                .unwrap(),
        )
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .text
        .clone();
    assert_eq!(
        actual_top, expected_top,
        "mutator path must rewrite top text after pattern change"
    );
    assert!(
        actual_top.contains('*'),
        "expected fill glyph in updated top text; got: '{}'",
        actual_top
    );
}

/// `color_palette` resolution: when the cfg names a palette that
/// exists, the per-cluster regions on each run pick up colours
/// from that palette (one region per cluster). The
/// `BorderNodeData.palette_cycle` resolution fans out per-side
/// from a single name → group mapping.
#[test]
fn border_tree_honors_palette_cycling() {
    use crate::mindmap::model::{
        ColorGroup, GlyphBorderConfig, Palette,
    };

    let mut map = synthetic_map(vec![synthetic_node("a", None, 0.0, 0.0)], vec![]);
    map.palettes.insert(
        "rainbow".into(),
        Palette {
            groups: vec![
                ColorGroup {
                    background: "#000000".into(),
                    frame: "#ff0000".into(),
                    text: "#000000".into(),
                    title: "#000000".into(),
                },
                ColorGroup {
                    background: "#000000".into(),
                    frame: "#00ff00".into(),
                    text: "#000000".into(),
                    title: "#000000".into(),
                },
            ],
        },
    );
    let node = map.nodes.get_mut("a").unwrap();
    node.size.width = 400.0;
    node.style.border = Some(GlyphBorderConfig {
        preset: "rounded".into(),
        font: None,
        font_size_pt: 14.0,
        color: None,
        glyphs: None,
        padding: 4.0,
        color_palette: Some("rainbow".into()),
        color_palette_field: Some("frame".into()),
    });

    let tree = build_border_tree(&map, &HashMap::new());
    let parent = tree.root.children(&tree.arena).next().unwrap();
    let top = parent.children(&tree.arena).next().unwrap();
    let area = tree.arena.get(top).unwrap().get().glyph_area().unwrap();
    let regions = area.regions.all_regions();
    assert!(
        regions.len() >= 2,
        "palette cycling should emit one region per cluster (got {})",
        regions.len()
    );
    // First region is red, second is green per the palette order.
    let r0 = regions[0].color.unwrap();
    let r1 = regions[1].color.unwrap();
    assert!(
        (r0[0] - 1.0).abs() < 0.05 && r0[1] < 0.05,
        "first cluster should be red; got {:?}",
        r0
    );
    assert!(
        r1[0] < 0.05 && (r1[1] - 1.0).abs() < 0.05,
        "second cluster should be green; got {:?}",
        r1
    );
}
