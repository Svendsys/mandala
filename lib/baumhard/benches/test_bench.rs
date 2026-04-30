// SPDX-License-Identifier: MPL-2.0

use baumhard::core::tests::primitives_tests::*;
use baumhard::font::tests::attrs_tests::*;
use baumhard::font::tests::fonts_tests::*;
use baumhard::gfx_structs::tests::area_tests::*;
use baumhard::gfx_structs::tests::model_tests::*;
use baumhard::gfx_structs::tests::mutator_tests::*;
use baumhard::gfx_structs::tests::region_indexer_tests::*;
use baumhard::gfx_structs::tests::region_params_tests::*;
use baumhard::gfx_structs::tests::region_rect_tests::*;
use baumhard::gfx_structs::tests::scene_tests::*;
use baumhard::gfx_structs::tests::shape_tests::*;
use baumhard::gfx_structs::tests::tree_tests::*;
use baumhard::font::tests::hex_tests::*;
use baumhard::font::tests::metrics_tests::*;
use baumhard::gfx_structs::tests::tree_walker_tests::*;
use baumhard::gfx_structs::tests::zoom_visibility_tests::*;
use baumhard::util::tests::arena_utils_tests::*;
use baumhard::util::tests::color_tests::*;
use baumhard::util::tests::geometry_tests::*;
use baumhard::util::tests::grapheme_chad_tests::*;
use baumhard::util::tests::primes_test::do_primes;
use criterion::{criterion_group, criterion_main, Criterion};

use std::collections::HashMap;
use std::path::PathBuf;

use baumhard::mindmap::loader;
use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::scene_builder::{
    build_scene_with_cache, SceneSelectionContext,
};
use baumhard::mindmap::scene_cache::SceneConnectionCache;

/// Load the testament fixture for the drag-drain benchmark. Panics
/// if the fixture is missing — this is benchmark code, not a test,
/// and a missing fixture means the benchmark binary can't do its job.
fn load_testament_map() -> MindMap {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.push("maps/testament.mindmap.json");
    loader::load_from_file(&path).expect("testament map should load for bench")
}

/// One drain of the translate path: re-enter `build_scene_with_cache`
/// with a fresh offset carrying the same delta for every dragged
/// node. The cache is already warm from the previous drain, so
/// every internal edge of the "subtree" falls into the translate
/// path.
fn do_subtree_drag_translate_path(
    map: &MindMap,
    cache: &mut SceneConnectionCache,
    dragged_ids: &[String],
    dx: f32,
    dy: f32,
    zoom: f32,
) {
    let mut offsets: HashMap<String, (f32, f32)> = HashMap::with_capacity(dragged_ids.len());
    for id in dragged_ids {
        offsets.insert(id.clone(), (dx, dy));
    }
    let _ = build_scene_with_cache(
        map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        cache,
        zoom,
    );
}

/// Baseline: simulate the pre-translate-path behaviour by clearing
/// the cache before every drain. Every edge falls into the slow
/// path (`build_connection_path` + `sample_path`). The ratio
/// between this and `do_subtree_drag_translate_path` is the
/// headline number the translate path buys.
fn do_subtree_drag_slow_path(
    map: &MindMap,
    cache: &mut SceneConnectionCache,
    dragged_ids: &[String],
    dx: f32,
    dy: f32,
    zoom: f32,
) {
    cache.clear();
    let mut offsets: HashMap<String, (f32, f32)> = HashMap::with_capacity(dragged_ids.len());
    for id in dragged_ids {
        offsets.insert(id.clone(), (dx, dy));
    }
    let _ = build_scene_with_cache(
        map,
        &offsets,
        SceneSelectionContext::default(),
        None,
        None,
        cache,
        zoom,
    );
}

fn criterion_benchmark(c: &mut Criterion) {
    // glyph_model //
    c.bench_function("matrix_place_in_1", |b| b.iter(|| matrix_place_in_1()));
    c.bench_function("matrix_place_in_2", |b| b.iter(|| matrix_place_in_2()));
    c.bench_function("matrix_place_in_3", |b| b.iter(|| matrix_place_in_3()));
    c.bench_function("matrix_add_assign_1", |b| b.iter(|| matrix_add_assign_1()));
    c.bench_function("matrix_add_assign_2", |b| b.iter(|| matrix_add_assign_2()));
    c.bench_function("line_add_assign_1", |b| b.iter(|| line_add_assign_1()));
    c.bench_function("line_add_assign_2", |b| b.iter(|| line_add_assign_2()));
    c.bench_function("line_add_assign_3", |b| b.iter(|| line_add_assign_3()));
    c.bench_function("line_add_assign_4", |b| b.iter(|| line_add_assign_4()));
    c.bench_function("component_of_index", |b| b.iter(|| component_of_index()));
    c.bench_function("index_of_component", |b| b.iter(|| index_of_component()));
    c.bench_function("expanding_insert_1", |b| b.iter(|| expanding_insert_1()));
    c.bench_function("expanding_insert_2", |b| b.iter(|| expanding_insert_2()));
    c.bench_function("expanding_insert_3", |b| b.iter(|| expanding_insert_3()));
    c.bench_function("expanding_insert_4", |b| b.iter(|| expanding_insert_4()));
    c.bench_function("expanding_insert_5", |b| b.iter(|| expanding_insert_5()));
    c.bench_function("expanding_insert_6", |b| b.iter(|| expanding_insert_6()));
    c.bench_function("expanding_insert_7", |b| b.iter(|| expanding_insert_7()));
    c.bench_function("overriding_insert_1", |b| b.iter(|| overriding_insert_1()));
    c.bench_function("overriding_insert_2", |b| b.iter(|| overriding_insert_2()));
    c.bench_function("overriding_insert_3", |b| b.iter(|| overriding_insert_3()));
    c.bench_function("overriding_insert_4", |b| b.iter(|| overriding_insert_4()));
    c.bench_function("overriding_insert_5", |b| b.iter(|| overriding_insert_5()));
    c.bench_function("overriding_insert_6", |b| b.iter(|| overriding_insert_6()));
    c.bench_function("overriding_insert_7", |b| b.iter(|| overriding_insert_7()));
    c.bench_function("overriding_insert_8", |b| b.iter(|| overriding_insert_8()));
    c.bench_function("overriding_insert_9", |b| b.iter(|| overriding_insert_9()));
    c.bench_function("overriding_insert_10", |b| b.iter(|| overriding_insert_10()));
    c.bench_function("overriding_insert_11", |b| b.iter(|| overriding_insert_11()));
    c.bench_function("overriding_insert_12", |b| b.iter(|| overriding_insert_12()));
    c.bench_function("overriding_insert_13", |b| b.iter(|| overriding_insert_13()));
    // glyph_area //
    c.bench_function("outline_default_is_none", |b| b.iter(|| do_outline_default_is_none()));
    c.bench_function("outline_assign_round_trip", |b| b.iter(|| do_outline_assign_round_trip()));
    c.bench_function("outline_subtract_clears", |b| b.iter(|| do_outline_subtract_clears()));
    c.bench_function("outline_changes_hash", |b| b.iter(|| do_outline_changes_hash()));
    c.bench_function("outline_field_add_picks_rhs", |b| b.iter(|| do_outline_field_add_picks_rhs()));
    c.bench_function("shape_default_is_rectangle", |b| b.iter(|| do_shape_default_is_rectangle()));
    c.bench_function("shape_assign_round_trip", |b| b.iter(|| do_shape_assign_round_trip()));
    c.bench_function("shape_subtract_resets_to_rectangle", |b| b.iter(|| do_shape_subtract_resets_to_rectangle()));
    c.bench_function("shape_changes_hash", |b| b.iter(|| do_shape_changes_hash()));
    c.bench_function("shape_field_add_picks_rhs", |b| b.iter(|| do_shape_field_add_picks_rhs()));
    // zoom_visibility //
    c.bench_function("zoom_visibility_default_is_unbounded", |b| b.iter(|| do_default_is_unbounded()));
    c.bench_function("zoom_visibility_unbounded_contains_full_camera_range", |b| b.iter(|| do_unbounded_contains_full_camera_range()));
    c.bench_function("zoom_visibility_min_only_is_inclusive", |b| b.iter(|| do_min_only_is_inclusive()));
    c.bench_function("zoom_visibility_max_only_is_inclusive", |b| b.iter(|| do_max_only_is_inclusive()));
    c.bench_function("zoom_visibility_closed_window_renders_inside_band", |b| b.iter(|| do_closed_window_renders_inside_band()));
    c.bench_function("zoom_visibility_single_point_band_is_inclusive", |b| b.iter(|| do_single_point_band_is_inclusive()));
    c.bench_function("zoom_visibility_inverted_band_never_contains", |b| b.iter(|| do_inverted_band_never_contains()));
    c.bench_function("zoom_visibility_nan_zoom_never_contains", |b| b.iter(|| do_nan_zoom_never_contains()));
    c.bench_function("zoom_visibility_try_new_enforces_invariants", |b| b.iter(|| do_try_new_enforces_invariants()));
    c.bench_function("zoom_visibility_assign_round_trip", |b| b.iter(|| do_zoom_visibility_assign_round_trip()));
    c.bench_function("zoom_visibility_subtract_resets_to_unbounded", |b| b.iter(|| do_zoom_visibility_subtract_resets_to_unbounded()));
    c.bench_function("zoom_visibility_field_add_picks_rhs", |b| b.iter(|| do_zoom_visibility_field_add_picks_rhs()));
    c.bench_function("zoom_visibility_changes_hash", |b| b.iter(|| do_zoom_visibility_changes_hash()));
    c.bench_function("zoom_visibility_default_is_skipped_in_json", |b| b.iter(|| do_zoom_visibility_default_is_skipped_in_json()));
    // shape math (point-in-shape / shape-vs-AABB) //
    c.bench_function("shape_from_style_string_known_names", |b| b.iter(|| do_shape_from_style_string_known_names()));
    c.bench_function("shape_from_style_string_empty_and_unknown_fall_back_to_rectangle", |b| b.iter(|| do_shape_from_style_string_empty_and_unknown_fall_back_to_rectangle()));
    c.bench_function("shape_rectangle_contains_local", |b| b.iter(|| do_shape_rectangle_contains_local()));
    c.bench_function("shape_ellipse_contains_centre_and_rim", |b| b.iter(|| do_shape_ellipse_contains_centre_and_rim()));
    c.bench_function("shape_ellipse_rejects_aabb_corners", |b| b.iter(|| do_shape_ellipse_rejects_aabb_corners()));
    c.bench_function("shape_ellipse_handles_stretched_conic", |b| b.iter(|| do_shape_ellipse_handles_stretched_conic()));
    c.bench_function("shape_degenerate_bounds_never_hit", |b| b.iter(|| do_shape_degenerate_bounds_never_hit()));
    c.bench_function("shape_ellipse_intersects_aabb_fully_inside", |b| b.iter(|| do_shape_ellipse_intersects_aabb_fully_inside()));
    c.bench_function("shape_ellipse_intersects_aabb_corner_only", |b| b.iter(|| do_shape_ellipse_intersects_aabb_corner_only()));
    c.bench_function("shape_ellipse_intersects_aabb_straddling_rim", |b| b.iter(|| do_shape_ellipse_intersects_aabb_straddling_rim()));
    c.bench_function("shape_ellipse_intersects_aabb_fully_outside", |b| b.iter(|| do_shape_ellipse_intersects_aabb_fully_outside()));
    c.bench_function("shape_shader_ids_are_stable", |b| b.iter(|| do_shape_shader_ids_are_stable()));
    // glyph_tree //
    c.bench_function("basics_solo_mutation", |b| b.iter(|| basics_solo_mutation()));
    c.bench_function("model_block_commands", |b| b.iter(|| model_block_commands()));
    c.bench_function("area_block_commands", |b| b.iter(|| area_block_commands()));
    c.bench_function("complex_tree_mutation", |b| b.iter(|| complex_tree_mutation()));
    c.bench_function("simple_tree_mutation", |b| b.iter(|| simple_tree_mutation()));
    c.bench_function("repeat_while_skip_while", |b| b.iter(|| repeat_while_skip_while()));
    c.bench_function("repeat_while_without_children_is_noop", |b| {
        b.iter(|| repeat_while_without_children_is_noop())
    });
    c.bench_function("event_propagation_complex", |b| b.iter(|| event_propagation_complex_symmetric()));
    c.bench_function("event_propagation_simple", |b| b.iter(|| event_propagation_simple()));
    c.bench_function("mutator_macro_applies_all_mutations_in_order", |b| {
        b.iter(|| do_mutator_macro_applies_all_mutations_in_order())
    });
    c.bench_function("mutator_macro_empty_is_noop", |b| {
        b.iter(|| do_mutator_macro_empty_is_noop())
    });
    c.bench_function("mutator_void_is_noop_when_applied_directly", |b| {
        b.iter(|| do_mutator_void_is_noop_when_applied_directly())
    });
    c.bench_function("mutator_void_preserves_channel_alignment_in_tree_walk", |b| {
        b.iter(|| do_mutator_void_preserves_channel_alignment_in_tree_walk())
    });
    // regions //
    c.bench_function("region_params_new_sunny_day", |b| b.iter(|| do_region_params_new_sunny_day()));
    c.bench_function("region_indexer_initialise", |b| b.iter(|| do_region_indexer_initialize()));
    c.bench_function("region_indexer_insert_and_remove", |b| b.iter(|| do_region_indexer_insert_and_remove()));
    c.bench_function("region_params_non_divisor_target", |b| b.iter(|| do_region_params_non_divisor_target()));
    c.bench_function("region_params_pixel_to_region", |b| b.iter(|| do_region_params_pixel_to_region()));
    c.bench_function("region_params_region_to_pixel", |b| b.iter(|| do_region_params_region_to_pixel()));
    c.bench_function("region_rect_exhaustive_4x4_grid", |b| b.iter(|| do_rect_exhaustive_4x4_grid()));
    // grapheme_chad //
    c.bench_function("slice_to_newline", |b| b.iter(|| do_slice_to_newline()));
    c.bench_function("split_graphemes", |b| b.iter(|| do_split_graphemes()));
    c.bench_function("find_byte_index_of_grapheme", |b| b.iter(|| do_find_byte_index_of_grapheme()));
    c.bench_function("replace_graphemes_until_newline", |b| b.iter(|| do_replace_graphemes_until_newline()));
    c.bench_function("count_grapheme_clusters", |b| b.iter(|| do_count_grapheme_clusters()));
    c.bench_function("find_nth_line_byte_indices", |b| b.iter(|| do_find_nth_line_byte_indices()));
    c.bench_function("find_nth_line_grapheme_indices", |b| b.iter(|| do_find_nth_line_grapheme_indices()));
    c.bench_function("remove_prefix_unicode", |b| b.iter(|| do_remove_prefix_unicode()));
    c.bench_function("insert_new_lines", |b| b.iter(|| do_insert_new_lines()));
    c.bench_function("push_spaces", |b| b.iter(|| do_push_spaces()));
    c.bench_function("count_number_of_lines", |b| b.iter(|| do_count_number_of_lines()));
    c.bench_function("truncate_unicode", |b| b.iter(|| do_truncate_unicode()));
    c.bench_function("insert_str_at_grapheme", |b| b.iter(|| do_insert_str_at_grapheme()));
    c.bench_function("delete_grapheme_at", |b| b.iter(|| do_delete_grapheme_at()));
    c.bench_function("grapheme_display_width", |b| b.iter(|| do_grapheme_display_width()));
    c.bench_function("truncate_to_display_width", |b| b.iter(|| do_truncate_to_display_width()));
    c.bench_function("word_left", |b| b.iter(|| do_word_left()));
    c.bench_function("word_right", |b| b.iter(|| do_word_right()));
    // geometry //
    c.bench_function("90_deg_rotation", |b| b.iter(|| do_90_deg_rotation()));
    c.bench_function("180_deg_rotation", |b| b.iter(|| do_180_deg_rotation()));
    c.bench_function("non_origin_pivot_rotation", |b| b.iter(|| do_non_origin_pivot_rotation()));
    c.bench_function("0_deg_rotation", |b| b.iter(|| do_0_deg_rotation()));
    c.bench_function("pixel_functions", |b| b.iter(|| do_pixel_functions()));
    c.bench_function("almost_equal", |b| b.iter(|| do_almost_equal()));
    c.bench_function("almost_equal_vec2", |b| b.iter(|| do_almost_equal_vec2()));
    c.bench_function("is_positive_finite", |b| b.iter(|| do_is_positive_finite()));
    c.bench_function("is_non_negative_finite_f64", |b| {
        b.iter(|| do_is_non_negative_finite_f64())
    });
    // font / metrics //
    c.bench_function("monospace_advance_zero_is_zero", |b| {
        b.iter(|| do_monospace_advance_zero_is_zero())
    });
    c.bench_function("monospace_advance_scales_linearly", |b| {
        b.iter(|| do_monospace_advance_scales_linearly())
    });
    c.bench_function("monospace_advance_ratio_is_zero_point_six", |b| {
        b.iter(|| do_monospace_advance_ratio_is_zero_point_six())
    });
    // color //
    c.bench_function("from_hex", |b| b.iter(|| do_from_hex()));
    c.bench_function("from_hex_lazy_static", |b| b.iter(|| do_from_hex_lazy_static()));
    c.bench_function("from_hex_garbage_falls_back_to_black", |b| {
        b.iter(|| do_from_hex_garbage_falls_back_to_black())
    });
    c.bench_function("rgba_hex_macros", |b| b.iter(|| do_rgba_hex_macros()));
    c.bench_function("hex_to_rgba_three_digit", |b| b.iter(|| do_hex_to_rgba_three_digit()));
    c.bench_function("hex_to_rgba_four_digit", |b| b.iter(|| do_hex_to_rgba_four_digit()));
    c.bench_function("hex_to_rgba_six_digit", |b| b.iter(|| do_hex_to_rgba_six_digit()));
    c.bench_function("hex_to_rgba_eight_digit", |b| b.iter(|| do_hex_to_rgba_eight_digit()));
    c.bench_function("hex_to_rgba_rejects_invalid_length", |b| {
        b.iter(|| do_hex_to_rgba_rejects_invalid_length())
    });
    c.bench_function("hex_to_rgba_rejects_non_hex_char", |b| {
        b.iter(|| do_hex_to_rgba_rejects_non_hex_char())
    });
    c.bench_function("hex_to_cosmic_color_round_trip", |b| {
        b.iter(|| do_hex_to_cosmic_color_round_trip())
    });
    // primitives //
    c.bench_function("overlaps", |b| b.iter(|| do_overlaps()));
    c.bench_function("split_and_separate_1", |b| b.iter(|| do_split_and_separate_1()));
    c.bench_function("split_and_separate_2", |b| b.iter(|| do_split_and_separate_2()));
    c.bench_function("submit_region_drops_inverted_range", |b| {
        b.iter(|| do_submit_region_drops_inverted_range())
    });
    c.bench_function("single_span_empty_is_empty", |b| {
        b.iter(|| do_single_span_empty_is_empty())
    });
    c.bench_function("single_span_non_empty_covers_range", |b| {
        b.iter(|| do_single_span_non_empty_covers_range())
    });
    c.bench_function("single_span_none_color_none_font", |b| {
        b.iter(|| do_single_span_none_color_none_font())
    });
    c.bench_function("shrink_regions_after_fully_right_shifts_left", |b| {
        b.iter(|| do_shrink_regions_after_fully_right_shifts_left())
    });
    c.bench_function("shrink_regions_after_spanning_region_absorbs", |b| {
        b.iter(|| do_shrink_regions_after_spanning_region_absorbs())
    });
    c.bench_function("shrink_regions_after_fully_inside_collapses", |b| {
        b.iter(|| do_shrink_regions_after_fully_inside_collapses())
    });
    c.bench_function("shrink_regions_after_left_partial_clamps", |b| {
        b.iter(|| do_shrink_regions_after_left_partial_clamps())
    });
    c.bench_function("shrink_regions_after_right_partial_clamps", |b| {
        b.iter(|| do_shrink_regions_after_right_partial_clamps())
    });
    c.bench_function("shrink_regions_after_zero_magnitude_is_noop", |b| {
        b.iter(|| do_shrink_regions_after_zero_magnitude_is_noop())
    });
    c.bench_function("insert_regions_at_straddling_region_absorbs", |b| {
        b.iter(|| do_insert_regions_at_straddling_region_absorbs())
    });
    c.bench_function("insert_regions_at_left_adjacent_region_absorbs", |b| {
        b.iter(|| do_insert_regions_at_left_adjacent_region_absorbs())
    });
    c.bench_function("insert_regions_at_shifts_right_regions", |b| {
        b.iter(|| do_insert_regions_at_shifts_right_regions())
    });
    c.bench_function("insert_regions_at_zero_position_shifts_all", |b| {
        b.iter(|| do_insert_regions_at_zero_position_shifts_all())
    });
    c.bench_function("insert_regions_at_empty_returns_false", |b| {
        b.iter(|| do_insert_regions_at_empty_returns_false())
    });
    // font / ink-bounds //
    c.bench_function("measure_glyph_ink_bounds_latin_has_positive_advance", |b| {
        b.iter(|| do_measure_glyph_ink_bounds_latin_has_positive_advance())
    });
    c.bench_function("measure_glyph_ink_bounds_tibetan_svasti_has_sidebearing", |b| {
        b.iter(|| do_measure_glyph_ink_bounds_tibetan_svasti_has_sidebearing())
    });
    c.bench_function("measure_glyph_ink_bounds_empty_string_is_zero", |b| {
        b.iter(|| do_measure_glyph_ink_bounds_empty_string_is_zero())
    });
    c.bench_function("measure_glyph_ink_bounds_x_offset_from_advance_center", |b| {
        b.iter(|| do_measure_glyph_ink_bounds_x_offset_from_advance_center())
    });
    c.bench_function("measure_glyph_ink_bounds_reports_baseline_line_y", |b| {
        b.iter(|| do_measure_glyph_ink_bounds_reports_baseline_line_y())
    });
    c.bench_function("measure_glyph_ink_bounds_y_offset_from_box_center", |b| {
        b.iter(|| do_measure_glyph_ink_bounds_y_offset_from_box_center())
    });
    c.bench_function("measure_text_block_unbounded_empty_is_zero", |b| {
        b.iter(|| do_measure_text_block_unbounded_empty_is_zero())
    });
    c.bench_function("measure_text_block_unbounded_single_line_nonzero", |b| {
        b.iter(|| do_measure_text_block_unbounded_single_line_nonzero())
    });
    c.bench_function("measure_text_block_unbounded_multiline_width_is_widest_line", |b| {
        b.iter(|| do_measure_text_block_unbounded_multiline_width_is_widest_line())
    });
    c.bench_function("measure_text_block_unbounded_width_scales_with_font_size", |b| {
        b.iter(|| do_measure_text_block_unbounded_width_scales_with_font_size())
    });
    // font / region-attrs bridges //
    c.bench_function("attrs_list_from_empty_regions_yields_no_spans", |b| {
        b.iter(|| do_attrs_list_from_empty_regions_yields_no_spans())
    });
    c.bench_function("attrs_list_from_single_color_region_emits_one_span", |b| {
        b.iter(|| do_attrs_list_from_single_color_region_emits_one_span())
    });
    c.bench_function("attrs_list_from_two_regions_emits_two_spans", |b| {
        b.iter(|| do_attrs_list_from_two_regions_emits_two_spans())
    });
    c.bench_function("attrs_list_pins_family_name_when_region_carries_app_font", |b| {
        b.iter(|| do_attrs_list_pins_family_name_when_region_carries_app_font())
    });
    c.bench_function("attrs_list_falls_back_to_monospace_when_region_has_no_font", |b| {
        b.iter(|| do_attrs_list_falls_back_to_monospace_when_region_has_no_font())
    });
    c.bench_function("rich_text_spans_empty_regions_yield_single_whole_text_span", |b| {
        b.iter(|| do_rich_text_spans_empty_regions_yield_single_whole_text_span())
    });
    c.bench_function("rich_text_spans_two_regions_slice_text_per_range", |b| {
        b.iter(|| do_rich_text_spans_two_regions_slice_text_per_range())
    });
    c.bench_function("rich_text_spans_drop_zero_width_regions", |b| {
        b.iter(|| do_rich_text_spans_drop_zero_width_regions())
    });
    c.bench_function("rich_text_spans_color_override_recolors_every_span", |b| {
        b.iter(|| do_rich_text_spans_color_override_recolors_every_span())
    });
    c.bench_function("rich_text_spans_color_override_applies_to_uncolored_region", |b| {
        b.iter(|| do_rich_text_spans_color_override_applies_to_uncolored_region())
    });
    c.bench_function("rich_text_spans_color_override_drops_zero_width_regions", |b| {
        b.iter(|| do_rich_text_spans_color_override_drops_zero_width_regions())
    });
    c.bench_function("rich_text_spans_pin_family_name_when_region_has_app_font", |b| {
        b.iter(|| do_rich_text_spans_pin_family_name_when_region_has_app_font())
    });
    c.bench_function("rich_text_spans_no_family_pin_when_region_has_no_font", |b| {
        b.iter(|| do_rich_text_spans_no_family_pin_when_region_has_no_font())
    });
    c.bench_function("rich_text_spans_clamps_out_of_range_region_end", |b| {
        b.iter(|| do_rich_text_spans_clamps_out_of_range_region_end())
    });
    c.bench_function("rich_text_spans_clamps_fully_out_of_range_region", |b| {
        b.iter(|| do_rich_text_spans_clamps_fully_out_of_range_region())
    });
    c.bench_function("rich_text_spans_empty_text_with_region_yields_no_spans", |b| {
        b.iter(|| do_rich_text_spans_empty_text_with_region_yields_no_spans())
    });
    // font family enumeration / lookup //
    c.bench_function("list_loaded_families_is_nonempty_sorted_unique", |b| {
        b.iter(|| do_list_loaded_families_is_nonempty_sorted_unique())
    });
    c.bench_function("app_font_by_family_round_trips", |b| {
        b.iter(|| do_app_font_by_family_round_trips())
    });
    c.bench_function("app_font_by_family_unknown_returns_none", |b| {
        b.iter(|| do_app_font_by_family_unknown_returns_none())
    });
    c.bench_function("loaded_families_iter_matches_owned_list", |b| {
        b.iter(|| do_loaded_families_iter_matches_owned_list())
    });
    // scene + hit-test //
    c.bench_function("descendant_at_hits_single_area", |b| b.iter(|| do_descendant_at_hits_single_area()));
    c.bench_function("descendant_at_prefers_smallest", |b| b.iter(|| do_descendant_at_prefers_smallest()));
    c.bench_function("descendant_near_grants_slack", |b| b.iter(|| do_descendant_near_grants_slack()));
    c.bench_function("descendants_aabb", |b| b.iter(|| do_descendants_aabb_covers_all_areas()));
    c.bench_function("descendants_aabb_invalidated_by_mutator", |b| b.iter(|| do_descendants_aabb_cache_invalidated_by_mutator()));
    c.bench_function("scene_component_at", |b| b.iter(|| do_scene_insert_and_component_at()));
    c.bench_function("scene_layer_order_hit_priority", |b| b.iter(|| do_scene_layer_order_controls_hit_priority()));
    c.bench_function("scene_offset_hit_test", |b| b.iter(|| do_scene_offset_is_applied_to_hit_test()));
    // arena_utils //
    c.bench_function("arena_utils_clone", |b| b.iter(|| do_clone()));
    // primes //
    c.bench_function("primes", |b| b.iter(|| do_primes()));

    // subtree-drag drain at zoom 1 and 30. Caches are warmed outside
    // `iter()` so the first-frame cold miss doesn't dominate the sample.
    let bench_map = load_testament_map();
    let dragged_ids: Vec<String> = bench_map.nodes.keys().cloned().collect();
    let mut translate_cache_1 = SceneConnectionCache::new();
    do_subtree_drag_translate_path(&bench_map, &mut translate_cache_1, &dragged_ids, 0.0, 0.0, 1.0);
    let mut slow_cache_1 = SceneConnectionCache::new();
    c.bench_function("subtree_drag_translate_path_zoom_1", |b| {
        let mut i = 0u32;
        b.iter(|| {
            i = i.wrapping_add(1);
            let dx = (i as f32) * 0.1;
            let dy = (i as f32) * 0.05;
            do_subtree_drag_translate_path(&bench_map, &mut translate_cache_1, &dragged_ids, dx, dy, 1.0);
        })
    });
    c.bench_function("subtree_drag_slow_path_zoom_1", |b| {
        let mut i = 0u32;
        b.iter(|| {
            i = i.wrapping_add(1);
            let dx = (i as f32) * 0.1;
            let dy = (i as f32) * 0.05;
            do_subtree_drag_slow_path(&bench_map, &mut slow_cache_1, &dragged_ids, dx, dy, 1.0);
        })
    });
    let mut translate_cache_30 = SceneConnectionCache::new();
    do_subtree_drag_translate_path(&bench_map, &mut translate_cache_30, &dragged_ids, 0.0, 0.0, 30.0);
    let mut slow_cache_30 = SceneConnectionCache::new();
    c.bench_function("subtree_drag_translate_path_zoom_30", |b| {
        let mut i = 0u32;
        b.iter(|| {
            i = i.wrapping_add(1);
            let dx = (i as f32) * 0.1;
            let dy = (i as f32) * 0.05;
            do_subtree_drag_translate_path(&bench_map, &mut translate_cache_30, &dragged_ids, dx, dy, 30.0);
        })
    });
    c.bench_function("subtree_drag_slow_path_zoom_30", |b| {
        let mut i = 0u32;
        b.iter(|| {
            i = i.wrapping_add(1);
            let dx = (i as f32) * 0.1;
            let dy = (i as f32) * 0.05;
            do_subtree_drag_slow_path(&bench_map, &mut slow_cache_30, &dragged_ids, dx, dy, 30.0);
        })
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
