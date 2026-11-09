// SPDX-License-Identifier: MPL-2.0

use baumhard::arena::tests::deep_tests::*;
use baumhard::arena::tests::in_place_tests::*;
use baumhard::arena::tests::layout_tests::*;
use baumhard::arena::tests::model_tests::*;
use baumhard::core::tests::primitives_tests::*;
use baumhard::document::tests::bounds_tests::*;
use baumhard::document::tests::deep_tests::*;
use baumhard::document::tests::deps_tests::*;
use baumhard::document::tests::element_tests::*;
use baumhard::document::tests::format_tests::*;
use baumhard::document::tests::ids_tests::*;
use baumhard::document::tests::inspect_tests::*;
use baumhard::document::tests::journal_tests::*;
use baumhard::document::tests::mutation_compose_tests::*;
use baumhard::document::tests::mutation_inherited_tests::*;
use baumhard::document::tests::mutation_ops_tests::*;
use baumhard::document::tests::mutation_validate_tests::*;
use baumhard::document::tests::resolve_tests::*;
use baumhard::document::tests::shape_tests::*;
use baumhard::document::tests::structure_tests::*;
use baumhard::document::tests::writer_feed_tests::*;
use baumhard::document::tests::writer_tests::*;
use baumhard::document::tests::writer_transaction_tests::*;
use baumhard::engine::tests::anchor_tests::*;
use baumhard::engine::tests::builder_tests::*;
use baumhard::engine::tests::contains_tests::*;
use baumhard::engine::tests::deep_tests::*;
use baumhard::engine::tests::inherited_tests::*;
use baumhard::engine::tests::interpolate_tests::*;
use baumhard::engine::tests::journal_tests::*;
use baumhard::engine::tests::routing_tests::*;
use baumhard::engine::tests::sorted_tests::*;
use baumhard::engine::tests::structure_tests::*;
use baumhard::engine::tests::walker_tests::*;
use baumhard::flow::tests::fit_tests::*;
use baumhard::flow::tests::halves_tests::*;
use baumhard::flow::tests::lines_tests::*;
use baumhard::flow::tests::outline_tests::*;
use baumhard::flow::tests::pitch_tests::*;
use baumhard::flow::tests::place_tests::*;
use baumhard::flow::tests::readable_tests::*;
use baumhard::flow::tests::shape_tests::*;
use baumhard::font::tests::attrs_tests::*;
use baumhard::font::tests::color_tests::*;
use baumhard::font::tests::fonts_tests::*;
use baumhard::font::tests::hex_tests::*;
use baumhard::font::tests::metric_cache_tests::*;
use baumhard::font::tests::metrics_tests::*;
use baumhard::font::tests::name_rules_tests::*;
use baumhard::font::tests::ornament_tests::*;
use baumhard::font::tests::shape_work_tests::*;
use baumhard::gfx_structs::tests::area_tests::*;
use baumhard::gfx_structs::tests::builder_tests::*;
use baumhard::gfx_structs::tests::bvh_descent_tests::*;
use baumhard::gfx_structs::tests::camera_tests::*;
use baumhard::gfx_structs::tests::delta_tests::*;
use baumhard::gfx_structs::tests::element_tests::*;
use baumhard::gfx_structs::tests::glyph_layer_tests::*;
use baumhard::gfx_structs::tests::journal_tests::*;
use baumhard::gfx_structs::tests::map_children_tests::*;
use baumhard::gfx_structs::tests::model_tests::*;
use baumhard::gfx_structs::tests::mutator_tests::*;
use baumhard::gfx_structs::tests::predicate_tests::*;
use baumhard::gfx_structs::tests::region_indexer_tests::*;
use baumhard::gfx_structs::tests::region_params_tests::*;
use baumhard::gfx_structs::tests::region_rect_tests::*;
use baumhard::gfx_structs::tests::run_tests::*;
use baumhard::gfx_structs::tests::scene_tests::*;
use baumhard::gfx_structs::tests::shape_tests::*;
use baumhard::gfx_structs::tests::spatial_descend_tests::*;
use baumhard::gfx_structs::tests::subtree_aabb_tests::*;
use baumhard::gfx_structs::tests::tree_tests::*;
use baumhard::gfx_structs::tests::tree_walker_tests::*;
use baumhard::gfx_structs::tests::zoom_visibility_tests::*;
use baumhard::projection::tests::deep_tests::*;
use baumhard::projection::tests::projection_tests::*;
use baumhard::util::tests::arena_utils_tests::*;
use baumhard::util::tests::color_tests::*;
use baumhard::util::tests::geometry_tests::*;
use baumhard::util::tests::grapheme_chad_tests::*;
use baumhard::util::tests::ordered_vec2_tests::*;
use baumhard::util::tests::primes_test::{do_is_prime_above_the_sieve_ceiling, do_primes};
use baumhard::util::tests::rust_source_tests::*;
use baumhard::util::tests::shortest_float_tests::*;
use criterion::{criterion_group, criterion_main, Criterion};

fn criterion_benchmark(c: &mut Criterion) {
    // glyph_model //
    c.bench_function("matrix_place_in_1", |b| b.iter(do_matrix_place_in_1));
    c.bench_function("matrix_place_in_2", |b| b.iter(do_matrix_place_in_2));
    c.bench_function("matrix_place_in_3", |b| b.iter(do_matrix_place_in_3));
    c.bench_function("matrix_place_in_multiline_component", |b| {
        b.iter(do_matrix_place_in_multiline_component)
    });
    c.bench_function("matrix_place_in_fusing_component", |b| {
        b.iter(do_matrix_place_in_fusing_component)
    });
    c.bench_function("matrix_add_assign_1", |b| b.iter(do_matrix_add_assign_1));
    c.bench_function("matrix_add_assign_2", |b| b.iter(do_matrix_add_assign_2));
    c.bench_function("line_add_assign_1", |b| b.iter(do_line_add_assign_1));
    c.bench_function("line_add_assign_2", |b| b.iter(do_line_add_assign_2));
    c.bench_function("line_add_assign_3", |b| b.iter(do_line_add_assign_3));
    c.bench_function("line_add_assign_4", |b| b.iter(do_line_add_assign_4));
    c.bench_function("line_ignore_initial_space_multibyte_indent", |b| {
        b.iter(do_line_ignore_initial_space_multibyte_indent)
    });
    c.bench_function("line_ignore_initial_space_crlf_indent", |b| {
        b.iter(do_line_ignore_initial_space_crlf_indent)
    });
    c.bench_function("line_ignore_initial_space_zwj_content", |b| {
        b.iter(do_line_ignore_initial_space_zwj_content)
    });
    c.bench_function("line_ignore_initial_space_sub_assign_rhs_longer_than_lhs", |b| {
        b.iter(do_line_ignore_initial_space_sub_assign_rhs_longer_than_lhs)
    });
    c.bench_function("line_ignore_initial_space_mul_assign_rhs_longer_than_lhs", |b| {
        b.iter(do_line_ignore_initial_space_mul_assign_rhs_longer_than_lhs)
    });
    c.bench_function(
        "line_ignore_initial_space_sub_assign_uses_lhs_color_when_present",
        |b| b.iter(do_line_ignore_initial_space_sub_assign_uses_lhs_color_when_present),
    );
    c.bench_function("line_ignore_initial_space_surplus_rhs_runs_append", |b| {
        b.iter(do_line_ignore_initial_space_surplus_rhs_runs_append)
    });
    c.bench_function(
        "line_ignore_initial_space_all_whitespace_rhs_paints_nothing",
        |b| b.iter(do_line_ignore_initial_space_all_whitespace_rhs_paints_nothing),
    );
    c.bench_function("line_runs_paint_by_column_with_the_flag_off", |b| {
        b.iter(do_line_runs_paint_by_column_with_the_flag_off)
    });
    c.bench_function("line_surplus_runs_paint_at_their_own_rhs_offsets", |b| {
        b.iter(do_line_surplus_runs_paint_at_their_own_rhs_offsets)
    });
    c.bench_function("line_surplus_runs_overwrite_a_longer_lhs_in_place", |b| {
        b.iter(do_line_surplus_runs_overwrite_a_longer_lhs_in_place)
    });
    c.bench_function("overriding_insert_is_partition_independent", |b| {
        b.iter(do_overriding_insert_is_partition_independent)
    });
    c.bench_function("component_of_index", |b| b.iter(do_component_of_index));
    c.bench_function("index_of_component", |b| b.iter(do_index_of_component));
    c.bench_function("expanding_insert_1", |b| b.iter(do_expanding_insert_1));
    c.bench_function("expanding_insert_2", |b| b.iter(do_expanding_insert_2));
    c.bench_function("expanding_insert_3", |b| b.iter(do_expanding_insert_3));
    c.bench_function("expanding_insert_4", |b| b.iter(do_expanding_insert_4));
    c.bench_function("expanding_insert_5", |b| b.iter(do_expanding_insert_5));
    c.bench_function("expanding_insert_6", |b| b.iter(do_expanding_insert_6));
    c.bench_function("expanding_insert_7", |b| b.iter(do_expanding_insert_7));
    c.bench_function("overriding_insert_1", |b| b.iter(do_overriding_insert_1));
    c.bench_function("overriding_insert_2", |b| b.iter(do_overriding_insert_2));
    c.bench_function("overriding_insert_3", |b| b.iter(do_overriding_insert_3));
    c.bench_function("overriding_insert_4", |b| b.iter(do_overriding_insert_4));
    c.bench_function("overriding_insert_5", |b| b.iter(do_overriding_insert_5));
    c.bench_function("overriding_insert_6", |b| b.iter(do_overriding_insert_6));
    c.bench_function("overriding_insert_7", |b| b.iter(do_overriding_insert_7));
    c.bench_function("overriding_insert_8", |b| b.iter(do_overriding_insert_8));
    c.bench_function("overriding_insert_9", |b| b.iter(do_overriding_insert_9));
    c.bench_function("overriding_insert_10", |b| b.iter(do_overriding_insert_10));
    c.bench_function("overriding_insert_11", |b| b.iter(do_overriding_insert_11));
    c.bench_function("overriding_insert_12", |b| b.iter(do_overriding_insert_12));
    c.bench_function("overriding_insert_13", |b| b.iter(do_overriding_insert_13));
    c.bench_function("delta_glyph_line_assign_applies_through_apply_to", |b| {
        b.iter(do_delta_glyph_line_assign_applies_through_apply_to)
    });
    c.bench_function("delta_glyph_lines_assign_applies_through_apply_to", |b| {
        b.iter(do_delta_glyph_lines_assign_applies_through_apply_to)
    });
    c.bench_function("delta_glyph_line_delete_clears_line", |b| {
        b.iter(do_delta_glyph_line_delete_clears_line)
    });
    c.bench_function("model_layer_subtract_saturates_at_zero", |b| {
        b.iter(do_model_layer_subtract_saturates_at_zero)
    });
    c.bench_function("delta_glyph_matrix_noop_and_delete_ignore_payload", |b| {
        b.iter(do_delta_glyph_matrix_noop_and_delete_ignore_payload)
    });
    c.bench_function("delta_glyph_matrix_applies_repeatedly", |b| {
        b.iter(do_delta_glyph_matrix_applies_repeatedly)
    });
    c.bench_function("matrix_add_assign_absorbs_taller_rhs", |b| {
        b.iter(do_matrix_add_assign_absorbs_taller_rhs)
    });
    c.bench_function("matrix_destructive_assigns_drop_surplus_rhs_rows", |b| {
        b.iter(do_matrix_destructive_assigns_drop_surplus_rhs_rows)
    });
    c.bench_function("component_add_assign_appends_text", |b| {
        b.iter(do_component_add_assign_appends_text)
    });
    c.bench_function("model_rotate_moves_position_around_pivot", |b| {
        b.iter(do_model_rotate_moves_position_around_pivot)
    });
    // The three `model_tests.rs` bodies the section above left out:
    // the scalar multiply that has no `*_2` sibling, and the two
    // destructive-op pins that hold `Delete`/`Subtract` to *not*
    // growing a matrix to reach a line that is not there.
    c.bench_function("matrix_mul_assign_1", |b| b.iter(do_matrix_mul_assign_1));
    c.bench_function("delta_glyph_line_destructive_ops_do_not_grow_missing_line", |b| {
        b.iter(do_delta_glyph_line_destructive_ops_do_not_grow_missing_line)
    });
    c.bench_function(
        "delta_glyph_lines_destructive_ops_do_not_grow_missing_lines",
        |b| b.iter(do_delta_glyph_lines_destructive_ops_do_not_grow_missing_lines),
    );
    // shared delta plumbing //
    c.bench_function("area_field_tags_cover_every_field", |b| {
        b.iter(do_area_field_tags_cover_every_field)
    });
    c.bench_function("model_field_tags_cover_every_field", |b| {
        b.iter(do_model_field_tags_cover_every_field)
    });
    c.bench_function("same_type_ignores_payload", |b| {
        b.iter(do_same_type_ignores_payload)
    });
    c.bench_function("pipeline_tags_are_derived", |b| {
        b.iter(do_pipeline_tags_are_derived)
    });
    c.bench_function("delta_new_collapses_duplicates_and_defaults_to_noop", |b| {
        b.iter(do_delta_new_collapses_duplicates_and_defaults_to_noop)
    });
    c.bench_function("delta_add_merges_both_sides", |b| {
        b.iter(do_delta_add_merges_both_sides)
    });
    c.bench_function("operation_key_matches_control_variant", |b| {
        b.iter(do_operation_key_matches_control_variant)
    });
    c.bench_function("apply_ref_matches_apply", |b| b.iter(do_apply_ref_matches_apply));
    c.bench_function("apply_reaches_every_area_field", |b| {
        b.iter(do_apply_reaches_every_area_field)
    });
    c.bench_function("apply_reaches_every_model_field", |b| {
        b.iter(do_apply_reaches_every_model_field)
    });
    // glyph_area //
    c.bench_function("outline_assign_round_trip", |b| {
        b.iter(do_outline_assign_round_trip)
    });
    c.bench_function("outline_subtract_clears", |b| b.iter(do_outline_subtract_clears));
    c.bench_function("outline_changes_hash", |b| b.iter(do_outline_changes_hash));
    c.bench_function("outline_field_add_picks_rhs", |b| {
        b.iter(do_outline_field_add_picks_rhs)
    });
    c.bench_function("shape_default_is_rectangle", |b| {
        b.iter(do_shape_default_is_rectangle)
    });
    c.bench_function("shape_assign_round_trip", |b| b.iter(do_shape_assign_round_trip));
    c.bench_function("shape_subtract_resets_to_rectangle", |b| {
        b.iter(do_shape_subtract_resets_to_rectangle)
    });
    c.bench_function("shape_changes_hash", |b| b.iter(do_shape_changes_hash));
    c.bench_function("shape_field_add_picks_rhs", |b| {
        b.iter(do_shape_field_add_picks_rhs)
    });
    c.bench_function(
        "change_region_range_missing_region_warns_and_leaves_area_intact",
        |b| b.iter(do_change_region_range_missing_region_warns_and_leaves_area_intact),
    );
    c.bench_function("delta_text_delete_clears_text", |b| {
        b.iter(do_delta_text_delete_clears_text)
    });
    c.bench_function("delta_regions_delete_clears_regions", |b| {
        b.iter(do_delta_regions_delete_clears_regions)
    });
    c.bench_function("area_rotate_moves_position_around_pivot", |b| {
        b.iter(do_area_rotate_moves_position_around_pivot)
    });
    c.bench_function("area_rotate_about_self_is_identity", |b| {
        b.iter(do_area_rotate_about_self_is_identity)
    });
    c.bench_function("area_rotate_matches_siblings", |b| {
        b.iter(do_area_rotate_matches_siblings)
    });
    c.bench_function("area_rotate_command_applies", |b| {
        b.iter(do_area_rotate_command_applies)
    });
    c.bench_function("area_rotate_command_json_wire_shape", |b| {
        b.iter(do_area_rotate_command_json_wire_shape)
    });
    // The canonical eight-direction outline stamp — the offset
    // table every bordered area expands through.
    c.bench_function("outline_offsets_canonical_8_stamp", |b| {
        b.iter(do_outline_offsets_canonical_8_stamp)
    });
    // zoom_visibility //
    c.bench_function("zoom_visibility_unbounded_contains_full_camera_range", |b| {
        b.iter(do_unbounded_contains_full_camera_range)
    });
    c.bench_function("zoom_visibility_inclusive_band_on_every_authored_shape", |b| {
        b.iter(do_inclusive_band_on_every_authored_shape)
    });
    c.bench_function("zoom_visibility_inverted_band_never_contains", |b| {
        b.iter(do_inverted_band_never_contains)
    });
    c.bench_function("zoom_visibility_nan_zoom_never_contains", |b| {
        b.iter(do_nan_zoom_never_contains)
    });
    c.bench_function("zoom_visibility_try_new_enforces_invariants", |b| {
        b.iter(do_try_new_enforces_invariants)
    });
    c.bench_function("zoom_visibility_assign_round_trip", |b| {
        b.iter(do_zoom_visibility_assign_round_trip)
    });
    c.bench_function("zoom_visibility_subtract_resets_to_unbounded", |b| {
        b.iter(do_zoom_visibility_subtract_resets_to_unbounded)
    });
    c.bench_function("zoom_visibility_field_add_picks_rhs", |b| {
        b.iter(do_zoom_visibility_field_add_picks_rhs)
    });
    c.bench_function("zoom_visibility_changes_hash", |b| {
        b.iter(do_zoom_visibility_changes_hash)
    });
    c.bench_function("zoom_visibility_default_is_skipped_in_json", |b| {
        b.iter(do_zoom_visibility_default_is_skipped_in_json)
    });
    // shape math (point-in-shape / shape-vs-AABB) //
    // Written as `b.iter(do_*)` rather than `b.iter(|| do_*())`: the
    // closure form around a bare zero-argument call is
    // `clippy::redundant_closure`, and the sweep that cleared those
    // left none of them in this file. The `b.iter(|| …)` entries
    // further down are not the other spelling of this — each passes
    // an argument or builds per-iteration state, which a function
    // reference cannot express.
    // Loads the demo map from disk on every iteration — a file-read
    // benchmark, not a classifier one. Kept in the harness for the
    // §B8 one-bench-per-`do_*()` contract; read its number as I/O.
    c.bench_function("shape_rectangle_contains_local", |b| {
        b.iter(do_shape_rectangle_contains_local)
    });
    c.bench_function("shape_ellipse_contains_center_and_rim", |b| {
        b.iter(do_shape_ellipse_contains_center_and_rim)
    });
    c.bench_function("shape_ellipse_rejects_aabb_corners", |b| {
        b.iter(do_shape_ellipse_rejects_aabb_corners)
    });
    c.bench_function("shape_ellipse_handles_stretched_conic", |b| {
        b.iter(do_shape_ellipse_handles_stretched_conic)
    });
    c.bench_function("shape_degenerate_bounds_never_hit", |b| {
        b.iter(do_shape_degenerate_bounds_never_hit)
    });
    c.bench_function("shape_ellipse_intersects_aabb_fully_inside", |b| {
        b.iter(do_shape_ellipse_intersects_aabb_fully_inside)
    });
    c.bench_function("shape_ellipse_intersects_aabb_corner_only", |b| {
        b.iter(do_shape_ellipse_intersects_aabb_corner_only)
    });
    c.bench_function("shape_ellipse_intersects_aabb_straddling_rim", |b| {
        b.iter(do_shape_ellipse_intersects_aabb_straddling_rim)
    });
    c.bench_function("shape_ellipse_intersects_aabb_fully_outside", |b| {
        b.iter(do_shape_ellipse_intersects_aabb_fully_outside)
    });
    // glyph_tree //
    c.bench_function("basics_solo_mutation", |b| b.iter(do_basics_solo_mutation));
    c.bench_function("model_block_commands", |b| b.iter(do_model_block_commands));
    c.bench_function("area_block_commands", |b| b.iter(do_area_block_commands));
    c.bench_function("complex_tree_mutation", |b| b.iter(do_complex_tree_mutation));
    c.bench_function("simple_tree_mutation", |b| b.iter(do_simple_tree_mutation));
    c.bench_function("repeat_while_skip_while", |b| b.iter(do_repeat_while_skip_while));
    c.bench_function("repeat_while_without_children_is_noop", |b| {
        b.iter(do_repeat_while_without_children_is_noop)
    });
    c.bench_function("event_propagation_complex_symmetric", |b| {
        b.iter(do_event_propagation_complex_symmetric)
    });
    c.bench_function("event_propagation_simple", |b| {
        b.iter(do_event_propagation_simple)
    });
    c.bench_function("mutator_macro_applies_all_mutations_in_order", |b| {
        b.iter(do_mutator_macro_applies_all_mutations_in_order)
    });
    c.bench_function("mutator_macro_empty_is_noop", |b| {
        b.iter(do_mutator_macro_empty_is_noop)
    });
    c.bench_function("mutator_void_is_noop_when_applied_directly", |b| {
        b.iter(do_mutator_void_is_noop_when_applied_directly)
    });
    c.bench_function("mutator_void_preserves_channel_alignment_in_tree_walk", |b| {
        b.iter(do_mutator_void_preserves_channel_alignment_in_tree_walk)
    });
    // `walk_tree` proper: channel alignment, RepeatWhile
    // continuation, and the deep-chain / wide-fan-out shapes. §B7
    // puts everything inside `walk_tree_from` under the hot-path
    // rules, so these are the bodies whose numbers those rules are
    // argued from.
    c.bench_function("macro_applies_all_mutations_in_order", |b| {
        b.iter(do_macro_applies_all_mutations_in_order)
    });
    c.bench_function("macro_with_empty_mutations_is_noop", |b| {
        b.iter(do_macro_with_empty_mutations_is_noop)
    });
    c.bench_function("mutator_without_payload_is_noop", |b| {
        b.iter(do_mutator_without_payload_is_noop)
    });
    c.bench_function("single_mutator_channel_filter_in_align_child_walks", |b| {
        b.iter(do_single_mutator_channel_filter_in_align_child_walks)
    });
    c.bench_function("direct_walk_at_mismatched_channels_is_noop", |b| {
        b.iter(do_direct_walk_at_mismatched_channels_is_noop)
    });
    c.bench_function("deep_chain_walk_reaches_every_node", |b| {
        b.iter(do_deep_chain_walk_reaches_every_node)
    });
    c.bench_function("wide_fan_out_applies_to_all_matching_siblings", |b| {
        b.iter(do_wide_fan_out_applies_to_all_matching_siblings)
    });
    c.bench_function("applying_same_delta_twice_accumulates", |b| {
        b.iter(do_applying_same_delta_twice_accumulates)
    });
    c.bench_function("mutation_is_deterministic_across_tree_clones", |b| {
        b.iter(do_mutation_is_deterministic_across_tree_clones)
    });
    c.bench_function("clone_preserves_unique_id_and_channel", |b| {
        b.iter(do_clone_preserves_unique_id_and_channel)
    });
    c.bench_function(
        "repeat_while_aligns_targets_whatever_their_insertion_order",
        |b| b.iter(do_repeat_while_aligns_targets_whatever_their_insertion_order),
    );
    c.bench_function(
        "repeat_while_merge_advance_does_not_drop_mutator_without_target",
        |b| b.iter(do_repeat_while_merge_advance_does_not_drop_mutator_without_target),
    );
    c.bench_function("default_terminator_resumes_whatever_the_insertion_order", |b| {
        b.iter(do_default_terminator_resumes_whatever_the_insertion_order)
    });
    // `Mutation` and `Instruction` at the single-node scale —
    // AreaDelta / AreaCommand / None, both `Instruction` variants,
    // `MutatorTree::apply_to`, and the `writes_the_same` predicate
    // that decides whether a delta is worth walking for at all.
    c.bench_function("mutation_area_delta_applies_field", |b| {
        b.iter(do_mutation_area_delta_applies_field)
    });
    c.bench_function("mutation_area_command_nudge_right", |b| {
        b.iter(do_mutation_area_command_nudge_right)
    });
    c.bench_function("mutation_noop_leaves_tree_unchanged", |b| {
        b.iter(do_mutation_noop_leaves_tree_unchanged)
    });
    c.bench_function("writes_the_same_is_reflexive_where_partial_eq_is_not", |b| {
        b.iter(do_writes_the_same_is_reflexive_where_partial_eq_is_not)
    });
    c.bench_function("writes_the_same_separates_values_that_write_differently", |b| {
        b.iter(do_writes_the_same_separates_values_that_write_differently)
    });
    c.bench_function("instruction_repeat_while_always_true", |b| {
        b.iter(do_instruction_repeat_while_always_true)
    });
    c.bench_function("instruction_mutator_applies_its_attached_mutation", |b| {
        b.iter(do_instruction_mutator_applies_its_attached_mutation)
    });
    c.bench_function("mutator_tree_applies_to_target", |b| {
        b.iter(do_mutator_tree_applies_to_target)
    });
    // regions //
    c.bench_function("region_params_new_sunny_day", |b| {
        b.iter(do_region_params_new_sunny_day)
    });
    c.bench_function("region_indexer_initialize", |b| {
        b.iter(do_region_indexer_initialize)
    });
    c.bench_function("region_indexer_insert_and_remove", |b| {
        b.iter(do_region_indexer_insert_and_remove)
    });
    c.bench_function("region_params_non_divisor_target", |b| {
        b.iter(do_region_params_non_divisor_target)
    });
    c.bench_function("region_params_pixel_to_region", |b| {
        b.iter(do_region_params_pixel_to_region)
    });
    c.bench_function("region_params_region_to_pixel", |b| {
        b.iter(do_region_params_region_to_pixel)
    });
    c.bench_function("region_rect_exhaustive_4x4_grid", |b| {
        b.iter(do_rect_exhaustive_4x4_grid)
    });
    // `RegionParams` pixel/region math — the rest of
    // `region_params_tests.rs`, whose four longest-standing entries
    // are above. Every one is arithmetic over a resolution and a
    // factor, which is what runs per pointer event once the index is
    // wired (§B6).
    c.bench_function("region_params_resolution_1x1", |b| {
        b.iter(do_region_params_resolution_1x1)
    });
    c.bench_function("region_params_factor_one", |b| {
        b.iter(do_region_params_factor_one)
    });
    c.bench_function("region_params_factor_equals_dimension", |b| {
        b.iter(do_region_params_factor_equals_dimension)
    });
    c.bench_function("region_params_pixel_region_roundtrip", |b| {
        b.iter(do_region_params_pixel_region_roundtrip)
    });
    c.bench_function("region_params_roundtrip_asymmetric", |b| {
        b.iter(do_region_params_roundtrip_asymmetric)
    });
    c.bench_function("region_params_boundary_pixels", |b| {
        b.iter(do_region_params_boundary_pixels)
    });
    c.bench_function("region_params_asymmetric", |b| {
        b.iter(do_region_params_asymmetric)
    });
    c.bench_function("region_params_very_wide", |b| b.iter(do_region_params_very_wide));
    c.bench_function("region_params_exhaustive_4x4", |b| {
        b.iter(do_region_params_exhaustive_4x4)
    });
    c.bench_function("region_params_exhaustive_12x12", |b| {
        b.iter(do_region_params_exhaustive_12x12)
    });
    c.bench_function("region_params_adapt_changes_resolution", |b| {
        b.iter(do_region_params_adapt_changes_resolution)
    });
    c.bench_function("region_params_adapt_chained", |b| {
        b.iter(do_region_params_adapt_chained)
    });
    // `RegionIndexer` insert / remove / query — the forward and
    // reverse index that makes selection highlighting O(log n)
    // instead of O(n) (§B6).
    c.bench_function("region_indexer_initialize_with", |b| {
        b.iter(do_region_indexer_initialize_with)
    });
    c.bench_function("region_indexer_initialize_zero", |b| {
        b.iter(do_region_indexer_initialize_zero)
    });
    c.bench_function("region_indexer_reinitialize_clears_forward_index", |b| {
        b.iter(do_region_indexer_reinitialize_clears_forward_index)
    });
    c.bench_function("region_indexer_multiple_elements_in_one_region", |b| {
        b.iter(do_region_indexer_multiple_elements_in_one_region)
    });
    c.bench_function("region_indexer_one_element_in_multiple_regions", |b| {
        b.iter(do_region_indexer_one_element_in_multiple_regions)
    });
    c.bench_function("region_indexer_empty_region_query", |b| {
        b.iter(do_region_indexer_empty_region_query)
    });
    c.bench_function("region_indexer_boundary_regions", |b| {
        b.iter(do_region_indexer_boundary_regions)
    });
    c.bench_function("region_indexer_duplicate_insert_is_idempotent", |b| {
        b.iter(do_region_indexer_duplicate_insert_is_idempotent)
    });
    c.bench_function("region_indexer_remove_nonexistent_is_noop", |b| {
        b.iter(do_region_indexer_remove_nonexistent_is_noop)
    });
    c.bench_function("region_indexer_no_reverse_index", |b| {
        b.iter(do_region_indexer_no_reverse_index)
    });
    c.bench_function("region_indexer_reverse_index_unknown_element", |b| {
        b.iter(do_region_indexer_reverse_index_unknown_element)
    });
    c.bench_function("region_indexer_reinitialize_stale_reverse_index", |b| {
        b.iter(do_region_indexer_reinitialize_stale_reverse_index)
    });
    c.bench_function("region_indexer_default", |b| b.iter(do_region_indexer_default));
    c.bench_function("region_indexer_element_id_zero", |b| {
        b.iter(do_region_indexer_element_id_zero)
    });
    c.bench_function("region_indexer_element_id_max", |b| {
        b.iter(do_region_indexer_element_id_max)
    });
    c.bench_function("region_indexer_clone_is_independent", |b| {
        b.iter(do_region_indexer_clone_is_independent)
    });
    c.bench_function("region_indexer_initialize_with_zero_axis", |b| {
        b.iter(do_region_indexer_initialize_with_zero_axis)
    });
    c.bench_function("region_indexer_remove_wrong_region_no_damage", |b| {
        b.iter(do_region_indexer_remove_wrong_region_no_damage)
    });
    c.bench_function("region_indexer_single_region", |b| {
        b.iter(do_region_indexer_single_region)
    });
    c.bench_function("region_indexer_insert_at_scale", |b| {
        b.iter(do_region_indexer_insert_at_scale)
    });
    // `calculate_regions_intersected_by_rectangle` across the grid
    // shapes: origin blocks, single cells, strips, offsets, the
    // out-of-bounds error paths, and the 12x12 exhaustive sweep whose
    // 4x4 sibling is above.
    c.bench_function("rect_origin_4x4_block", |b| b.iter(do_rect_origin_4x4_block));
    c.bench_function("rect_origin_4x5_block", |b| b.iter(do_rect_origin_4x5_block));
    c.bench_function("rect_origin_5x4_block", |b| b.iter(do_rect_origin_5x4_block));
    c.bench_function("rect_origin_3x2_block", |b| b.iter(do_rect_origin_3x2_block));
    c.bench_function("rect_single_cell", |b| b.iter(do_rect_single_cell));
    c.bench_function("rect_single_pixel_each_corner", |b| {
        b.iter(do_rect_single_pixel_each_corner)
    });
    c.bench_function("rect_single_pixel_center", |b| {
        b.iter(do_rect_single_pixel_center)
    });
    c.bench_function("rect_full_width_single_row", |b| {
        b.iter(do_rect_full_width_single_row)
    });
    c.bench_function("rect_full_height_single_column", |b| {
        b.iter(do_rect_full_height_single_column)
    });
    c.bench_function("rect_full_grid", |b| b.iter(do_rect_full_grid));
    c.bench_function("rect_thin_vertical_strip", |b| {
        b.iter(do_rect_thin_vertical_strip)
    });
    c.bench_function("rect_thin_horizontal_strip", |b| {
        b.iter(do_rect_thin_horizontal_strip)
    });
    c.bench_function("rect_offset_single_row", |b| b.iter(do_rect_offset_single_row));
    c.bench_function("rect_offset_multi_row", |b| b.iter(do_rect_offset_multi_row));
    c.bench_function("rect_offset_last_column_multi_row", |b| {
        b.iter(do_rect_offset_last_column_multi_row)
    });
    c.bench_function("rect_center_3x3", |b| b.iter(do_rect_center_3x3));
    c.bench_function("rect_bottom_right_2x2", |b| b.iter(do_rect_bottom_right_2x2));
    c.bench_function("rect_asymmetric_grid", |b| b.iter(do_rect_asymmetric_grid));
    c.bench_function("rect_start_after_end", |b| b.iter(do_rect_start_after_end));
    c.bench_function("rect_start_out_of_bounds", |b| {
        b.iter(do_rect_start_out_of_bounds)
    });
    c.bench_function("rect_end_out_of_bounds", |b| b.iter(do_rect_end_out_of_bounds));
    c.bench_function("rect_exhaustive_12x12_grid", |b| {
        b.iter(do_rect_exhaustive_12x12_grid)
    });
    // grapheme_chad //
    c.bench_function("slice_to_newline", |b| b.iter(do_slice_to_newline));
    c.bench_function("split_graphemes", |b| b.iter(do_split_graphemes));
    c.bench_function("find_byte_index_of_grapheme", |b| {
        b.iter(do_find_byte_index_of_grapheme)
    });
    c.bench_function("byte_indices_of_graphemes", |b| {
        b.iter(do_byte_indices_of_graphemes)
    });
    c.bench_function("apply_operation_metric_clamp", |b| {
        b.iter(do_apply_operation_clamps_the_shaper_metrics)
    });
    c.bench_function("font_metric_setters_clamp", |b| {
        b.iter(do_font_metric_setters_clamp_to_the_shaper_domain)
    });
    c.bench_function("replace_graphemes_until_newline", |b| {
        b.iter(do_replace_graphemes_until_newline)
    });
    c.bench_function("replace_substring_matches_the_byte_splice_reference", |b| {
        b.iter(do_replace_substring_matches_the_byte_splice_reference)
    });
    c.bench_function("replace_substring_refuses_a_mid_character_range", |b| {
        b.iter(do_replace_substring_refuses_a_mid_character_range)
    });
    c.bench_function(
        "split_off_graphemes_matches_the_collect_and_concat_reference",
        |b| b.iter(do_split_off_graphemes_matches_the_collect_and_concat_reference),
    );
    // Reads one module's source per iteration, the same shape as
    // `the_metric_cache_probes_without_owning_the_cluster` below:
    // after the first iteration what moves is the parse and the brace
    // walk, not the disk.
    c.bench_function("the_text_edit_primitives_carry_no_whole_buffer_copy", |b| {
        b.iter(do_the_text_edit_primitives_carry_no_whole_buffer_copy)
    });
    c.bench_function("count_grapheme_clusters", |b| b.iter(do_count_grapheme_clusters));
    c.bench_function("grapheme_byte_starts", |b| b.iter(do_grapheme_byte_starts));
    c.bench_function("first_non_whitespace_grapheme", |b| {
        b.iter(do_first_non_whitespace_grapheme)
    });
    c.bench_function("find_nth_line_grapheme_indices", |b| {
        b.iter(do_find_nth_line_grapheme_indices)
    });
    c.bench_function("line_model_is_coherent", |b| b.iter(do_line_model_is_coherent));
    c.bench_function("remove_prefix_unicode", |b| b.iter(do_remove_prefix_unicode));
    c.bench_function("insert_new_lines", |b| b.iter(do_insert_new_lines));
    c.bench_function("push_spaces", |b| b.iter(do_push_spaces));
    c.bench_function("count_number_of_lines", |b| b.iter(do_count_number_of_lines));
    c.bench_function("truncate_unicode", |b| b.iter(do_truncate_unicode));
    c.bench_function("insert_str_at_grapheme", |b| b.iter(do_insert_str_at_grapheme));
    c.bench_function("insert_str_at_grapheme_counted", |b| {
        b.iter(do_insert_str_at_grapheme_counted)
    });
    c.bench_function("delete_grapheme_at", |b| b.iter(do_delete_grapheme_at));
    c.bench_function("delete_grapheme_range", |b| b.iter(do_delete_grapheme_range));
    c.bench_function("grapheme_display_width", |b| b.iter(do_grapheme_display_width));
    c.bench_function("truncate_to_display_width", |b| {
        b.iter(do_truncate_to_display_width)
    });
    c.bench_function("truncate_to_byte_budget", |b| b.iter(do_truncate_to_byte_budget));
    c.bench_function("wrap_to_display_width", |b| b.iter(do_wrap_to_display_width));
    c.bench_function("word_left", |b| b.iter(do_word_left));
    c.bench_function("word_right", |b| b.iter(do_word_right));
    c.bench_function("prev_word_boundary_ws", |b| b.iter(do_prev_word_boundary_ws));
    c.bench_function("token_start_ws", |b| b.iter(do_token_start_ws));
    c.bench_function("take_graphemes", |b| b.iter(do_take_graphemes));
    c.bench_function("line_bounds_at", |b| b.iter(do_line_bounds_at));
    c.bench_function("insert_spaces", |b| b.iter(do_insert_spaces));
    c.bench_function("split_graphemes_owned", |b| b.iter(do_split_graphemes_owned));
    c.bench_function("join_graphemes", |b| b.iter(do_join_graphemes));
    // rust_source // — every `do_*()` the module exports, which is
    // the whole rule this file follows: an entry per body, and a
    // body that should not be measured is a plain `#[test]` rather
    // than a `do_*()` with an exemption. The tree-wide sweep is the
    // one that took that opt-out; `production_code` did not, and
    // reads one file per iteration like the two `shape_*` entries
    // above.
    c.bench_function("strip_comments", |b| {
        b.iter(do_strip_comments_removes_only_comments)
    });
    c.bench_function("strip_comments_preserves_line_count", |b| {
        b.iter(do_strip_comments_preserves_line_count)
    });
    c.bench_function("strip_comments_survives_unterminated_input", |b| {
        b.iter(do_strip_comments_survives_unterminated_input)
    });
    c.bench_function("above_test_modules", |b| {
        b.iter(do_above_test_modules_cuts_at_the_module_only)
    });
    c.bench_function("braced_block_after", |b| {
        b.iter(do_braced_block_after_matches_one_item)
    });
    c.bench_function("string_literals", |b| {
        b.iter(do_string_literals_returns_every_literal)
    });
    c.bench_function("blank_string_literals_keeps_code_and_offsets", |b| {
        b.iter(do_blank_string_literals_keeps_code_and_offsets)
    });
    c.bench_function("comment_text_keeps_comments_and_drops_code", |b| {
        b.iter(do_comment_text_keeps_comments_and_drops_code)
    });
    c.bench_function("statements", |b| b.iter(do_statements_split_at_the_right_places));
    c.bench_function("production_code_returns_code_without_prose", |b| {
        b.iter(do_production_code_returns_code_without_prose)
    });
    // geometry //
    c.bench_function("90_deg_rotation", |b| b.iter(do_90_deg_rotation));
    c.bench_function("180_deg_rotation", |b| b.iter(do_180_deg_rotation));
    c.bench_function("non_origin_pivot_rotation", |b| {
        b.iter(do_non_origin_pivot_rotation)
    });
    c.bench_function("0_deg_rotation", |b| b.iter(do_0_deg_rotation));
    c.bench_function("pixel_functions", |b| b.iter(do_pixel_functions));
    c.bench_function("aabb_contains_includes_every_boundary", |b| {
        b.iter(do_aabb_contains_includes_every_boundary)
    });
    c.bench_function("aabb_contains_rejects_on_each_axis_independently", |b| {
        b.iter(do_aabb_contains_rejects_on_each_axis_independently)
    });
    c.bench_function("almost_equal", |b| b.iter(do_almost_equal));
    c.bench_function("almost_equal_f64_is_tighter_than_its_f32_sibling", |b| {
        b.iter(do_almost_equal_f64_is_tighter_than_its_f32_sibling)
    });
    c.bench_function("almost_equal_vec2", |b| b.iter(do_almost_equal_vec2));
    c.bench_function("is_positive_finite", |b| b.iter(do_is_positive_finite));
    c.bench_function("is_non_negative_finite_f64", |b| {
        b.iter(do_is_non_negative_finite_f64)
    });
    c.bench_function("option_almost_equal", |b| b.iter(do_option_almost_equal));
    // font / metrics //
    c.bench_function("monospace_advance_scales_linearly", |b| {
        b.iter(do_monospace_advance_scales_linearly)
    });
    // font / metric cache //
    c.bench_function("glyph_advance_cache_hit_matches_miss", |b| {
        b.iter(do_glyph_advance_cache_hit_matches_miss)
    });
    c.bench_function("glyph_advance_distinct_per_grapheme", |b| {
        b.iter(do_glyph_advance_distinct_per_grapheme)
    });
    c.bench_function("cluster_width_sums_per_grapheme", |b| {
        b.iter(do_cluster_width_sums_per_grapheme)
    });
    c.bench_function("glyph_advance_scales_with_size", |b| {
        b.iter(do_glyph_advance_scales_with_size)
    });
    c.bench_function("glyph_ink_distinct_per_size", |b| {
        b.iter(do_glyph_ink_distinct_per_size)
    });
    c.bench_function("glyph_ink_distinct_per_grapheme", |b| {
        b.iter(do_glyph_ink_distinct_per_grapheme)
    });
    c.bench_function("glyph_advance_with_shapes_cold_key_under_held_guard", |b| {
        b.iter(do_glyph_advance_with_shapes_cold_key_under_held_guard)
    });
    c.bench_function("glyph_ink_with_cold_key_under_held_guard", |b| {
        b.iter(do_glyph_ink_with_cold_key_under_held_guard)
    });
    c.bench_function("glyph_advance_distinct_per_face", |b| {
        b.iter(do_glyph_advance_distinct_per_face)
    });
    // Reads one module's source per iteration, the same shape as
    // `every_hex_entry_point_is_downstream_of_the_one_parser` above:
    // after the first iteration what moves is the parse and the
    // brace walk, not the disk.
    c.bench_function("the_metric_cache_probes_without_owning_the_cluster", |b| {
        b.iter(do_the_metric_cache_probes_without_owning_the_cluster)
    });
    // font / name rules //
    c.bench_function("decode_name_record_utf16_be_ascii", |b| {
        b.iter(do_decode_name_record_utf16_be_ascii)
    });
    c.bench_function("decode_name_record_utf16_be_non_ascii_does_not_panic", |b| {
        b.iter(do_decode_name_record_utf16_be_non_ascii_does_not_panic)
    });
    c.bench_function("decode_name_record_survives_malformed_input", |b| {
        b.iter(do_decode_name_record_survives_malformed_input)
    });
    c.bench_function("ascii_font_name_reduces_to_identifier_material", |b| {
        b.iter(do_ascii_font_name_reduces_to_identifier_material)
    });
    c.bench_function("camel_case_joins_and_rotates_digits", |b| {
        b.iter(do_camel_case_joins_and_rotates_digits)
    });
    c.bench_function("camel_case_capitalizes_after_rotating", |b| {
        b.iter(do_camel_case_capitalizes_after_rotating)
    });
    c.bench_function("camel_case_rejects_unusable_names", |b| {
        b.iter(do_camel_case_rejects_unusable_names)
    });
    c.bench_function("rotate_leading_digits", |b| b.iter(do_rotate_leading_digits));
    c.bench_function("capitalize_first_preserves_internal_case", |b| {
        b.iter(do_capitalize_first_preserves_internal_case)
    });
    c.bench_function("fallback_sanitize_from_file_stem", |b| {
        b.iter(do_fallback_sanitize_from_file_stem)
    });
    c.bench_function("fallback_sanitize_respects_length_ceiling", |b| {
        b.iter(do_fallback_sanitize_respects_length_ceiling)
    });
    c.bench_function("variant_name_prefers_name_table_then_stem", |b| {
        b.iter(do_variant_name_prefers_name_table_then_stem)
    });
    c.bench_function("font_extension_matching_is_case_insensitive", |b| {
        b.iter(do_font_extension_matching_is_case_insensitive)
    });
    c.bench_function("family_key_groups_container_pairs", |b| {
        b.iter(do_family_key_groups_container_pairs)
    });
    c.bench_function("font_candidate_display_name_falls_back_to_stem", |b| {
        b.iter(do_font_candidate_display_name_falls_back_to_stem)
    });
    c.bench_function("select_font_variants_sorts_by_variant_name", |b| {
        b.iter(do_select_font_variants_sorts_by_variant_name)
    });
    c.bench_function("select_font_variants_is_order_independent", |b| {
        b.iter(do_select_font_variants_is_order_independent)
    });
    c.bench_function("select_font_variants_prefers_ttf_container", |b| {
        b.iter(do_select_font_variants_prefers_ttf_container)
    });
    c.bench_function("select_font_variants_breaks_ties_by_path", |b| {
        b.iter(do_select_font_variants_breaks_ties_by_path)
    });
    c.bench_function("select_font_variants_keeps_distinct_styles_of_one_family", |b| {
        b.iter(do_select_font_variants_keeps_distinct_styles_of_one_family)
    });
    c.bench_function(
        "select_font_variants_keeps_same_container_faces_of_one_family",
        |b| b.iter(do_select_font_variants_keeps_same_container_faces_of_one_family),
    );
    c.bench_function("select_font_variants_collapses_one_face_in_two_containers", |b| {
        b.iter(do_select_font_variants_collapses_one_face_in_two_containers)
    });
    c.bench_function("select_font_variants_renames_name_collisions", |b| {
        b.iter(do_select_font_variants_renames_name_collisions)
    });
    c.bench_function("select_font_variants_reserves_the_any_sentinel", |b| {
        b.iter(do_select_font_variants_reserves_the_any_sentinel)
    });
    c.bench_function("select_font_variants_reserves_the_self_keyword", |b| {
        b.iter(do_select_font_variants_reserves_the_self_keyword)
    });
    c.bench_function("no_variant_is_a_rust_keyword", |b| {
        b.iter(do_no_variant_is_a_rust_keyword)
    });
    c.bench_function("select_font_variants_skips_unnamable_files", |b| {
        b.iter(do_select_font_variants_skips_unnamable_files)
    });
    c.bench_function("generated_app_font_table_is_ordered_and_unique", |b| {
        b.iter(do_generated_app_font_table_is_ordered_and_unique)
    });
    // color //
    c.bench_function("from_hex", |b| b.iter(do_from_hex));
    c.bench_function("from_hex_lazy_static", |b| b.iter(do_from_hex_lazy_static));
    c.bench_function("from_hex_garbage_falls_back_to_black", |b| {
        b.iter(do_from_hex_garbage_falls_back_to_black)
    });
    c.bench_function("hex_to_color_parses_bytes_at_compile_time", |b| {
        b.iter(do_hex_to_color_parses_bytes_at_compile_time)
    });
    // Reads the two hex-bearing modules' source per iteration, the
    // same shape as `production_code_returns_code_without_prose`
    // above: after the first iteration what moves is the two parses
    // and the closure walk, not the disk.
    c.bench_function("every_hex_entry_point_is_downstream_of_the_one_parser", |b| {
        b.iter(do_every_hex_entry_point_is_downstream_of_the_one_parser)
    });
    c.bench_function("color_with_alpha_replaces_only_the_alpha_channel", |b| {
        b.iter(do_color_with_alpha_replaces_only_the_alpha_channel)
    });
    c.bench_function("hex_to_rgba_three_digit", |b| b.iter(do_hex_to_rgba_three_digit));
    c.bench_function("hex_to_rgba_four_digit", |b| b.iter(do_hex_to_rgba_four_digit));
    c.bench_function("hex_to_rgba_six_digit", |b| b.iter(do_hex_to_rgba_six_digit));
    c.bench_function("hex_to_rgba_eight_digit", |b| b.iter(do_hex_to_rgba_eight_digit));
    c.bench_function("hex_to_rgba_rejects_invalid_length", |b| {
        b.iter(do_hex_to_rgba_rejects_invalid_length)
    });
    c.bench_function("hex_to_rgba_rejects_non_hex_char", |b| {
        b.iter(do_hex_to_rgba_rejects_non_hex_char)
    });
    c.bench_function("cosmic_color_from_color_carries_every_channel", |b| {
        b.iter(do_cosmic_color_from_color_carries_every_channel)
    });
    c.bench_function("cosmic_color_from_color_agrees_with_the_float_bridge", |b| {
        b.iter(do_cosmic_color_from_color_agrees_with_the_float_bridge)
    });
    c.bench_function("hex_to_cosmic_color_round_trip", |b| {
        b.iter(do_hex_to_cosmic_color_round_trip)
    });
    c.bench_function("resolve_var_hit", |b| b.iter(do_resolve_var_hit));
    c.bench_function("resolve_var_miss_returns_raw", |b| {
        b.iter(do_resolve_var_miss_returns_raw)
    });
    c.bench_function("resolve_var_plain_hex_passes_through", |b| {
        b.iter(do_resolve_var_plain_hex_passes_through)
    });
    c.bench_function("resolve_var_malformed_passes_through", |b| {
        b.iter(do_resolve_var_malformed_passes_through)
    });
    c.bench_function("resolve_var_tolerates_whitespace_inside", |b| {
        b.iter(do_resolve_var_tolerates_whitespace_inside)
    });
    c.bench_function("resolve_var_single_level_no_recursion", |b| {
        b.iter(do_resolve_var_single_level_no_recursion)
    });
    c.bench_function("hex_to_rgba_safe_good_input", |b| {
        b.iter(do_hex_to_rgba_safe_good_input)
    });
    c.bench_function("hex_to_rgba_safe_garbage_returns_fallback", |b| {
        b.iter(do_hex_to_rgba_safe_garbage_returns_fallback)
    });
    c.bench_function("hex_with_alpha_scaled_halves_opaque_input", |b| {
        b.iter(do_hex_with_alpha_scaled_halves_opaque_input)
    });
    c.bench_function("hex_with_alpha_scaled_factor_one_round_trips", |b| {
        b.iter(do_hex_with_alpha_scaled_factor_one_round_trips)
    });
    c.bench_function("hex_with_alpha_scaled_factor_zero_zeros_alpha", |b| {
        b.iter(do_hex_with_alpha_scaled_factor_zero_zeros_alpha)
    });
    c.bench_function(
        "hex_with_alpha_scaled_factor_above_one_clamps_to_full_alpha",
        |b| b.iter(do_hex_with_alpha_scaled_factor_above_one_clamps_to_full_alpha),
    );
    c.bench_function("hex_with_alpha_scaled_preserves_rgb_on_8_char_input", |b| {
        b.iter(do_hex_with_alpha_scaled_preserves_rgb_on_8_char_input)
    });
    c.bench_function("hex_with_alpha_scaled_parse_failure_passes_through", |b| {
        b.iter(do_hex_with_alpha_scaled_parse_failure_passes_through)
    });
    c.bench_function("hex_with_alpha_scaled_composes", |b| {
        b.iter(do_hex_with_alpha_scaled_composes)
    });
    c.bench_function("hex_to_rgba_safe_with_alpha", |b| {
        b.iter(do_hex_to_rgba_safe_with_alpha)
    });
    c.bench_function("hex_to_rgba_safe_no_panic_on_malformed_batch", |b| {
        b.iter(do_hex_to_rgba_safe_no_panic_on_malformed_batch)
    });
    c.bench_function("hex_to_rgba_safe_short_hex_expands_each_nibble", |b| {
        b.iter(do_hex_to_rgba_safe_short_hex_expands_each_nibble)
    });
    c.bench_function("hex_to_rgba_safe_accepts_valid_6_and_8_char_both_cases", |b| {
        b.iter(do_hex_to_rgba_safe_accepts_valid_6_and_8_char_both_cases)
    });
    c.bench_function("resolve_var_large_theme_map_zero_copy_passthrough", |b| {
        b.iter(do_resolve_var_large_theme_map_zero_copy_passthrough)
    });
    c.bench_function("resolve_var_passthrough_on_unknown_is_verbatim", |b| {
        b.iter(do_resolve_var_passthrough_on_unknown_is_verbatim)
    });
    c.bench_function("hsv_to_rgb_primaries", |b| b.iter(do_hsv_to_rgb_primaries));
    c.bench_function("hsv_to_rgb_grayscale_ignores_hue", |b| {
        b.iter(do_hsv_to_rgb_grayscale_ignores_hue)
    });
    c.bench_function("hsv_to_rgb_wraps_hue", |b| b.iter(do_hsv_to_rgb_wraps_hue));
    c.bench_function("rgb_to_hsv_primaries", |b| b.iter(do_rgb_to_hsv_primaries));
    c.bench_function("hsv_hex_roundtrip_named_colors", |b| {
        b.iter(do_hsv_hex_roundtrip_named_colors)
    });
    c.bench_function("hex_to_hsv_safe_rejects_garbage", |b| {
        b.iter(do_hex_to_hsv_safe_rejects_garbage)
    });
    c.bench_function("hsv_to_hex_emits_six_char_format", |b| {
        b.iter(do_hsv_to_hex_emits_six_char_format)
    });
    c.bench_function("rgba_to_hex_drops_alpha_only_when_saturated_opaque", |b| {
        b.iter(do_rgba_to_hex_drops_alpha_only_when_saturated_opaque)
    });
    c.bench_function("color_add_wraps_per_channel_modulo_256", |b| {
        b.iter(do_color_add_wraps_per_channel_modulo_256)
    });
    c.bench_function("color_sub_wraps_underflow_modulo_256", |b| {
        b.iter(do_color_sub_wraps_underflow_modulo_256)
    });
    c.bench_function("color_mul_wraps_overflow_modulo_256", |b| {
        b.iter(do_color_mul_wraps_overflow_modulo_256)
    });
    c.bench_function("color_div_per_channel", |b| b.iter(do_color_div_per_channel));
    c.bench_function("color_to_float_does_not_collapse_mid_range_channels", |b| {
        b.iter(do_color_to_float_does_not_collapse_mid_range_channels)
    });
    c.bench_function("color_new_f32_to_float_round_trips_within_one_byte", |b| {
        b.iter(do_color_new_f32_to_float_round_trips_within_one_byte)
    });
    c.bench_function("is_valid_hex_color_matches_hex_parser", |b| {
        b.iter(do_is_valid_hex_color_matches_hex_parser)
    });
    c.bench_function("parse_var_name_tolerates_inner_whitespace", |b| {
        b.iter(do_parse_var_name_tolerates_inner_whitespace)
    });
    c.bench_function("parse_var_name_rejects_malformed_refs", |b| {
        b.iter(do_parse_var_name_rejects_malformed_refs)
    });
    // primitives //
    c.bench_function("overlaps", |b| b.iter(do_overlaps));
    c.bench_function("split_and_separate_1", |b| b.iter(do_split_and_separate_1));
    c.bench_function("split_and_separate_2", |b| b.iter(do_split_and_separate_2));
    c.bench_function("split_and_separate_truth_table", |b| {
        b.iter(do_split_and_separate_truth_table)
    });
    c.bench_function("split_and_separate_preserves_payload_on_both_halves", |b| {
        b.iter(do_split_and_separate_preserves_payload_on_both_halves)
    });
    c.bench_function("split_and_separate_overflow_drops_the_whole_call", |b| {
        b.iter(do_split_and_separate_overflow_drops_the_whole_call)
    });
    c.bench_function("split_and_separate_precondition_violations_propagate", |b| {
        b.iter(do_split_and_separate_precondition_violations_propagate)
    });
    c.bench_function("range_checked_push_right", |b| {
        b.iter(do_range_checked_push_right)
    });
    c.bench_function("shift_regions_after_overflow_drops_the_whole_call", |b| {
        b.iter(do_shift_regions_after_overflow_drops_the_whole_call)
    });
    c.bench_function("insert_regions_at_overflow_drops_the_whole_call", |b| {
        b.iter(do_insert_regions_at_overflow_drops_the_whole_call)
    });
    c.bench_function("insertion_primitives_differ_only_at_the_three_seams", |b| {
        b.iter(do_insertion_primitives_differ_only_at_the_three_seams)
    });
    c.bench_function("submit_region_drops_inverted_range", |b| {
        b.iter(do_submit_region_drops_inverted_range)
    });
    c.bench_function("single_span_empty_is_empty", |b| {
        b.iter(do_single_span_empty_is_empty)
    });
    c.bench_function("single_span_non_empty_covers_range", |b| {
        b.iter(do_single_span_non_empty_covers_range)
    });
    c.bench_function("single_span_none_color_none_font", |b| {
        b.iter(do_single_span_none_color_none_font)
    });
    c.bench_function("region_shift_and_shrink_disagree_at_the_seam", |b| {
        b.iter(do_region_shift_and_shrink_disagree_at_the_seam)
    });
    c.bench_function("shrink_regions_after_fully_right_shifts_left", |b| {
        b.iter(do_shrink_regions_after_fully_right_shifts_left)
    });
    c.bench_function("shrink_regions_after_spanning_region_absorbs", |b| {
        b.iter(do_shrink_regions_after_spanning_region_absorbs)
    });
    c.bench_function("shrink_regions_after_fully_inside_collapses", |b| {
        b.iter(do_shrink_regions_after_fully_inside_collapses)
    });
    c.bench_function("shrink_regions_after_left_partial_clamps", |b| {
        b.iter(do_shrink_regions_after_left_partial_clamps)
    });
    c.bench_function("shrink_regions_after_right_partial_clamps", |b| {
        b.iter(do_shrink_regions_after_right_partial_clamps)
    });
    c.bench_function("shrink_regions_after_zero_magnitude_is_noop", |b| {
        b.iter(do_shrink_regions_after_zero_magnitude_is_noop)
    });
    c.bench_function("insert_regions_at_straddling_region_absorbs", |b| {
        b.iter(do_insert_regions_at_straddling_region_absorbs)
    });
    c.bench_function("insert_regions_at_left_adjacent_region_absorbs", |b| {
        b.iter(do_insert_regions_at_left_adjacent_region_absorbs)
    });
    c.bench_function("insert_regions_at_shifts_right_regions", |b| {
        b.iter(do_insert_regions_at_shifts_right_regions)
    });
    c.bench_function("insert_regions_at_zero_position_shifts_all", |b| {
        b.iter(do_insert_regions_at_zero_position_shifts_all)
    });
    c.bench_function("insert_regions_at_empty_returns_false", |b| {
        b.iter(do_insert_regions_at_empty_returns_false)
    });
    c.bench_function("same_content_separates_a_recolor_from_equality", |b| {
        b.iter(do_same_content_separates_a_recolor_from_equality)
    });
    c.bench_function("same_content_compares_whole_tables", |b| {
        b.iter(do_same_content_compares_whole_tables)
    });
    // font / ink-bounds //
    c.bench_function("measure_glyph_ink_bounds_latin_has_positive_advance", |b| {
        b.iter(do_measure_glyph_ink_bounds_latin_has_positive_advance)
    });
    c.bench_function("outline_bounds_follows_the_curve", |b| {
        b.iter(do_outline_bounds_follows_the_curve)
    });
    c.bench_function(
        "flow_a_closed_paths_table_samples_are_known_before_it_is_built",
        |b| b.iter(do_flow_a_closed_paths_table_samples_are_known_before_it_is_built),
    );
    c.bench_function("flow_a_closed_paths_length_is_finite_as_its_build_says", |b| {
        b.iter(do_flow_a_closed_paths_length_is_finite_as_its_build_says)
    });
    c.bench_function("nearest_weight_follows_css_fonts_4", |b| {
        b.iter(do_nearest_weight_follows_css_fonts_4)
    });
    c.bench_function("a_pinned_family_lists_its_compiled_weights_and_faces", |b| {
        b.iter(do_a_pinned_family_lists_its_compiled_weights_and_faces)
    });
    c.bench_function(
        "only_the_default_face_is_variable_and_its_pin_spans_its_axis",
        |b| b.iter(do_only_the_default_face_is_variable_and_its_pin_spans_its_axis),
    );
    c.bench_function(
        "no_host_face_bears_a_compiled_name_or_comes_before_a_compiled_face",
        |b| b.iter(do_no_host_face_bears_a_compiled_name_or_comes_before_a_compiled_face),
    );
    c.bench_function(
        "no_compiled_family_has_a_slant_at_a_weight_its_other_slant_lacks",
        |b| b.iter(do_no_compiled_family_has_a_slant_at_a_weight_its_other_slant_lacks),
    );
    // The ornament fallback (#297): two character-map reads, one file
    // read and parse, and a pinned-family lookup under the font guard.
    c.bench_function("each_ornament_face_holds_exactly_its_listed_code_points", |b| {
        b.iter(do_each_ornament_face_holds_exactly_its_listed_code_points)
    });
    c.bench_function(
        "the_regeneration_script_subsets_exactly_the_listed_code_points",
        |b| b.iter(do_the_regeneration_script_subsets_exactly_the_listed_code_points),
    );
    c.bench_function(
        "the_ornament_faces_hold_no_code_point_the_default_face_holds",
        |b| b.iter(do_the_ornament_faces_hold_no_code_point_the_default_face_holds),
    );
    c.bench_function("every_built_in_glyph_is_held_by_a_compiled_fallback_face", |b| {
        b.iter(do_every_built_in_glyph_is_held_by_a_compiled_fallback_face)
    });
    c.bench_function("each_fallback_names_its_faces_family_at_every_weight", |b| {
        b.iter(do_each_fallback_names_its_faces_family_at_every_weight)
    });
    // The fallback shaped (#228, #297): ornaments pinned and unpinned at
    // every weight, and text that must not reach an ornament face.
    c.bench_function("a_pinned_span_draws_each_ornament_in_its_compiled_face", |b| {
        b.iter(do_a_pinned_span_draws_each_ornament_in_its_compiled_face)
    });
    c.bench_function(
        "an_unpinned_span_draws_each_ornament_in_its_compiled_face_at_every_weight",
        |b| b.iter(do_an_unpinned_span_draws_each_ornament_in_its_compiled_face_at_every_weight),
    );
    c.bench_function("no_text_glyph_is_drawn_by_an_ornament_face", |b| {
        b.iter(do_no_text_glyph_is_drawn_by_an_ornament_face)
    });
    c.bench_function(
        "a_hebrew_run_is_drawn_in_noto_sans_hebrew_not_public_pixel",
        |b| b.iter(do_a_hebrew_run_is_drawn_in_noto_sans_hebrew_not_public_pixel),
    );
    // The patched font fallback (#304): every oracle text, and a long
    // word of each, shaped in five faces with both merges; and one word
    // of letters pinned to a face that holds none of them, shaped with
    // the published merge at two lengths and with the linear one at
    // three, after a warm-up.
    c.bench_function("the_fallback_merge_lays_out_what_the_published_merge_did", |b| {
        b.iter(do_the_fallback_merge_lays_out_what_the_published_merge_did)
    });
    c.bench_function(
        "a_word_its_face_lacks_costs_fallback_work_linear_in_its_length",
        |b| b.iter(do_a_word_its_face_lacks_costs_fallback_work_linear_in_its_length),
    );
    // A code point no face draws, in a font system of its own built for
    // each call: asked of its faces once, then kept; a face loaded into
    // such a font system draws what none drew; and a hundred of them in a
    // run of the global font system, shaped with their answer kept.
    c.bench_function("a_code_point_no_face_draws_is_asked_of_the_faces_once", |b| {
        b.iter(do_a_code_point_no_face_draws_is_asked_of_the_faces_once)
    });
    c.bench_function("a_face_added_to_the_database_draws_what_no_face_drew", |b| {
        b.iter(do_a_face_added_to_the_database_draws_what_no_face_drew)
    });
    // The seeded random differential: 300 random texts, faces, weights
    // and slants, each shaped with both merges.
    c.bench_function("a_seeded_random_differential_finds_no_merge_divergence", |b| {
        b.iter(do_a_seeded_random_differential_finds_no_merge_divergence)
    });
    // The outline cache keeps each entry for its font system: one glyph
    // id of one face id measured in two font systems of their own, twice
    // each.
    c.bench_function("two_font_systems_never_answer_each_others_outlines", |b| {
        b.iter(do_two_font_systems_never_answer_each_others_outlines)
    });
    // The patched merge's and early stop's guards, on fixtures built of
    // shaped glyphs (and a font system with no faces).
    c.bench_function(
        "a_run_the_merge_cannot_reason_about_is_merged_as_published",
        |b| b.iter(do_a_run_the_merge_cannot_reason_about_is_merged_as_published),
    );
    c.bench_function(
        "a_fallback_the_merge_cannot_reason_about_is_merged_as_published",
        |b| b.iter(do_a_fallback_the_merge_cannot_reason_about_is_merged_as_published),
    );
    c.bench_function("the_early_stop_needs_every_missing_entry_matched", |b| {
        b.iter(do_the_early_stop_needs_every_missing_entry_matched)
    });
    c.bench_function(
        "the_early_stop_leaves_a_cluster_of_several_code_points_to_the_faces",
        |b| b.iter(do_the_early_stop_leaves_a_cluster_of_several_code_points_to_the_faces),
    );
    c.bench_function("a_hundred_contiguous_undrawn_code_points_cost_one_shaping", |b| {
        b.iter(do_a_hundred_contiguous_undrawn_code_points_cost_one_shaping)
    });
    c.bench_function("measure_glyph_ink_bounds_tibetan_svasti_has_sidebearing", |b| {
        b.iter(do_measure_glyph_ink_bounds_tibetan_svasti_has_sidebearing)
    });
    c.bench_function("measure_glyph_ink_bounds_empty_string_is_zero", |b| {
        b.iter(do_measure_glyph_ink_bounds_empty_string_is_zero)
    });
    c.bench_function("measure_glyph_ink_bounds_x_offset_from_advance_center", |b| {
        b.iter(do_measure_glyph_ink_bounds_x_offset_from_advance_center)
    });
    c.bench_function("measure_glyph_ink_bounds_reports_baseline_line_y", |b| {
        b.iter(do_measure_glyph_ink_bounds_reports_baseline_line_y)
    });
    c.bench_function("measure_glyph_ink_bounds_y_offset_from_box_center", |b| {
        b.iter(do_measure_glyph_ink_bounds_y_offset_from_box_center)
    });
    c.bench_function("measure_text_block_unbounded_empty_is_zero", |b| {
        b.iter(do_measure_text_block_unbounded_empty_is_zero)
    });
    c.bench_function("measure_text_block_unbounded_single_line_nonzero", |b| {
        b.iter(do_measure_text_block_unbounded_single_line_nonzero)
    });
    c.bench_function(
        "measure_text_block_unbounded_multiline_width_is_widest_line",
        |b| b.iter(do_measure_text_block_unbounded_multiline_width_is_widest_line),
    );
    c.bench_function("measure_text_block_unbounded_width_scales_with_font_size", |b| {
        b.iter(do_measure_text_block_unbounded_width_scales_with_font_size)
    });
    // font / region-attrs bridges //
    c.bench_function("attrs_list_from_empty_regions_yields_no_spans", |b| {
        b.iter(do_attrs_list_from_empty_regions_yields_no_spans)
    });
    c.bench_function("attrs_list_from_single_color_region_emits_one_span", |b| {
        b.iter(do_attrs_list_from_single_color_region_emits_one_span)
    });
    c.bench_function("attrs_list_from_two_regions_emits_two_spans", |b| {
        b.iter(do_attrs_list_from_two_regions_emits_two_spans)
    });
    c.bench_function("attrs_list_pins_family_name_when_region_carries_app_font", |b| {
        b.iter(do_attrs_list_pins_family_name_when_region_carries_app_font)
    });
    c.bench_function("attrs_list_asks_for_no_family_when_region_has_no_font", |b| {
        b.iter(do_attrs_list_asks_for_no_family_when_region_has_no_font)
    });
    c.bench_function(
        "rich_text_spans_empty_regions_yield_single_whole_text_span",
        |b| b.iter(do_rich_text_spans_empty_regions_yield_single_whole_text_span),
    );
    c.bench_function("rich_text_spans_two_regions_slice_text_per_range", |b| {
        b.iter(do_rich_text_spans_two_regions_slice_text_per_range)
    });
    c.bench_function("rich_text_spans_drop_zero_width_regions", |b| {
        b.iter(do_rich_text_spans_drop_zero_width_regions)
    });
    c.bench_function("rich_text_spans_color_override_recolors_every_span", |b| {
        b.iter(do_rich_text_spans_color_override_recolors_every_span)
    });
    c.bench_function(
        "rich_text_spans_color_override_applies_to_uncolored_region",
        |b| b.iter(do_rich_text_spans_color_override_applies_to_uncolored_region),
    );
    c.bench_function("rich_text_spans_color_override_drops_zero_width_regions", |b| {
        b.iter(do_rich_text_spans_color_override_drops_zero_width_regions)
    });
    c.bench_function("rich_text_spans_pin_family_name_when_region_has_app_font", |b| {
        b.iter(do_rich_text_spans_pin_family_name_when_region_has_app_font)
    });
    c.bench_function("rich_text_spans_no_family_pin_when_region_has_no_font", |b| {
        b.iter(do_rich_text_spans_no_family_pin_when_region_has_no_font)
    });
    c.bench_function("rich_text_spans_clamps_out_of_range_region_end", |b| {
        b.iter(do_rich_text_spans_clamps_out_of_range_region_end)
    });
    c.bench_function("rich_text_spans_clamps_fully_out_of_range_region", |b| {
        b.iter(do_rich_text_spans_clamps_fully_out_of_range_region)
    });
    c.bench_function("rich_text_spans_empty_text_with_region_yields_no_spans", |b| {
        b.iter(do_rich_text_spans_empty_text_with_region_yields_no_spans)
    });
    c.bench_function("rich_text_spans_into_refills_rather_than_appends", |b| {
        b.iter(do_rich_text_spans_into_refills_rather_than_appends)
    });
    c.bench_function("rich_text_spans_into_keeps_the_no_region_whole_text_span", |b| {
        b.iter(do_rich_text_spans_into_keeps_the_no_region_whole_text_span)
    });
    c.bench_function("shortest_f64_prints_and_round_trips_like_the_f32", |b| {
        b.iter(do_shortest_f64_prints_and_round_trips_like_the_f32)
    });
    c.bench_function("displayed_text_is_the_concatenated_covered_slices", |b| {
        b.iter(do_displayed_text_is_the_concatenated_covered_slices)
    });
    c.bench_function("displayed_text_without_regions_is_the_whole_text", |b| {
        b.iter(do_displayed_text_without_regions_is_the_whole_text)
    });
    // The grapheme-boundary slicing paths on both bridges: a region
    // range that lands mid-cluster is the input that turns a byte
    // slice into a panic, and the ZWJ cases are the widest clusters
    // the app actually renders.
    c.bench_function("attrs_list_slice_at_zwj_grapheme_boundary", |b| {
        b.iter(do_attrs_list_slice_at_zwj_grapheme_boundary)
    });
    c.bench_function("rich_text_spans_slice_at_grapheme_boundary", |b| {
        b.iter(do_rich_text_spans_slice_at_grapheme_boundary)
    });
    c.bench_function("rich_text_spans_slice_at_zwj_grapheme_boundary", |b| {
        b.iter(do_rich_text_spans_slice_at_zwj_grapheme_boundary)
    });
    // font family enumeration / lookup //
    c.bench_function("list_loaded_families_is_nonempty_sorted_unique", |b| {
        b.iter(do_list_loaded_families_is_nonempty_sorted_unique)
    });
    c.bench_function("app_font_by_family_round_trips", |b| {
        b.iter(do_app_font_by_family_round_trips)
    });
    c.bench_function("app_font_by_family_unknown_returns_none", |b| {
        b.iter(do_app_font_by_family_unknown_returns_none)
    });
    c.bench_function("loaded_families_iter_matches_owned_list", |b| {
        b.iter(do_loaded_families_iter_matches_owned_list)
    });
    c.bench_function("family_name_of_round_trips", |b| {
        b.iter(do_family_name_of_round_trips)
    });
    // scene + hit-test //
    c.bench_function("descendant_at_hits_single_area", |b| {
        b.iter(do_descendant_at_hits_single_area)
    });
    c.bench_function("descendant_at_prefers_smallest", |b| {
        b.iter(do_descendant_at_prefers_smallest)
    });
    c.bench_function("descendant_near_grants_slack", |b| {
        b.iter(do_descendant_near_grants_slack)
    });
    c.bench_function("descendants_aabb", |b| {
        b.iter(do_descendants_aabb_covers_all_areas)
    });
    c.bench_function("descendants_aabb_invalidated_by_mutator", |b| {
        b.iter(do_descendants_aabb_cache_invalidated_by_mutator)
    });
    c.bench_function("scene_component_at", |b| b.iter(do_scene_insert_and_component_at));
    c.bench_function("scene_layer_order_hit_priority", |b| {
        b.iter(do_scene_layer_order_controls_hit_priority)
    });
    c.bench_function("scene_offset_hit_test", |b| {
        b.iter(do_scene_offset_is_applied_to_hit_test)
    });
    // Passed by path rather than wrapped in `|| f()` — the
    // surrounding file predates `clippy::redundant_closure` being
    // clean and carries 165 instances of the wrapped form; new
    // entries do not add to that count.
    c.bench_function("scene_component_in", |b| {
        b.iter(do_scene_component_in_scopes_the_hit_to_one_tree)
    });
    c.bench_function("scene_component_in_offset_visibility", |b| {
        b.iter(do_scene_component_in_honors_offset_and_visibility)
    });
    c.bench_function("scene_component_in_overlap_smallest_area", |b| {
        b.iter(do_scene_component_in_resolves_overlap_by_smallest_area)
    });
    // `Scene` bookkeeping around the hit tests above: the
    // empty-tree miss, visibility skipping, removal, ownership
    // hand-back, and the insertion-stable layer order the hit
    // priority rests on.
    c.bench_function("descendant_at_returns_none_on_empty_tree", |b| {
        b.iter(do_descendant_at_returns_none_on_empty_tree)
    });
    c.bench_function("scene_invisible_trees_are_skipped", |b| {
        b.iter(do_scene_invisible_trees_are_skipped)
    });
    c.bench_function("scene_remove_drops_entry", |b| {
        b.iter(do_scene_remove_drops_entry)
    });
    c.bench_function("scene_entry_into_tree_returns_ownership", |b| {
        b.iter(do_scene_entry_into_tree_returns_ownership)
    });
    c.bench_function("scene_ids_in_layer_order_is_stable_by_insertion", |b| {
        b.iter(do_scene_ids_in_layer_order_is_stable_by_insertion)
    });
    // subtree AABBs (ensure/invalidate) //
    c.bench_function("subtree_aabb_leaf_equals_own_bounds", |b| {
        b.iter(do_subtree_aabb_leaf_equals_own_bounds)
    });
    c.bench_function("subtree_aabb_parent_encloses_children", |b| {
        b.iter(do_subtree_aabb_parent_encloses_children)
    });
    c.bench_function("subtree_aabb_root_encloses_entire_tree", |b| {
        b.iter(do_subtree_aabb_root_encloses_entire_tree)
    });
    c.bench_function("subtree_aabb_invalidated_by_mutation", |b| {
        b.iter(do_subtree_aabb_invalidated_by_mutation)
    });
    c.bench_function("subtree_aabb_void_tree_is_none", |b| {
        b.iter(do_subtree_aabb_void_tree_is_none)
    });
    c.bench_function("subtree_aabb_ensure_is_idempotent", |b| {
        b.iter(do_subtree_aabb_ensure_is_idempotent)
    });
    c.bench_function("subtree_aabb_deep_chain_propagates_to_root", |b| {
        b.iter(do_subtree_aabb_deep_chain_propagates_to_root)
    });
    c.bench_function("ensure_subtree_aabbs_keeps_the_descendants_aabb_memo", |b| {
        b.iter(do_ensure_subtree_aabbs_keeps_the_descendants_aabb_memo)
    });
    c.bench_function("subtree_aabb_wide_tree_root_covers_all", |b| {
        b.iter(do_subtree_aabb_wide_tree_root_covers_all)
    });
    c.bench_function("subtree_aabb_single_area_node", |b| {
        b.iter(do_subtree_aabb_single_area_node)
    });
    c.bench_function("subtree_aabb_zero_bounds_area_ignored", |b| {
        b.iter(do_subtree_aabb_zero_bounds_area_ignored)
    });
    c.bench_function("subtree_aabb_negative_position", |b| {
        b.iter(do_subtree_aabb_negative_position)
    });
    c.bench_function("subtree_aabb_merge_none_base_with_child", |b| {
        b.iter(do_subtree_aabb_merge_none_base_with_child)
    });
    c.bench_function("subtree_aabb_merge_two_disjoint_children", |b| {
        b.iter(do_subtree_aabb_merge_two_disjoint_children)
    });
    c.bench_function("subtree_aabb_all_children_zero_bounds", |b| {
        b.iter(do_subtree_aabb_all_children_zero_bounds)
    });
    // BVH-accelerated descendant_at / descendant_near //
    c.bench_function("descendant_at_finds_leaf_via_bvh", |b| {
        b.iter(do_descendant_at_finds_leaf_via_bvh)
    });
    c.bench_function("descendant_at_prunes_disjoint_subtree", |b| {
        b.iter(do_descendant_at_prunes_disjoint_subtree)
    });
    c.bench_function("descendant_at_returns_none_on_miss", |b| {
        b.iter(do_descendant_at_returns_none_on_miss)
    });
    c.bench_function("descendant_at_smallest_area_wins", |b| {
        b.iter(do_descendant_at_smallest_area_wins)
    });
    c.bench_function("descendant_at_deep_chain_finds_leaf", |b| {
        b.iter(do_descendant_at_deep_chain_finds_leaf)
    });
    c.bench_function("descendant_at_deep_chain_miss", |b| {
        b.iter(do_descendant_at_deep_chain_miss)
    });
    c.bench_function("descendant_at_wide_tree_finds_correct_child", |b| {
        b.iter(do_descendant_at_wide_tree_finds_correct_child)
    });
    c.bench_function("descendant_at_wide_tree_gap_is_miss", |b| {
        b.iter(do_descendant_at_wide_tree_gap_is_miss)
    });
    c.bench_function("descendant_at_overlapping_siblings", |b| {
        b.iter(do_descendant_at_overlapping_siblings)
    });
    c.bench_function("descendant_near_negative_coords", |b| {
        b.iter(do_descendant_near_negative_coords)
    });
    c.bench_function("bvh_descend_skips_glyph_model_nodes", |b| {
        b.iter(do_bvh_descend_skips_glyph_model_nodes)
    });
    c.bench_function("bvh_descend_point_on_exact_boundary", |b| {
        b.iter(do_bvh_descend_point_on_exact_boundary)
    });
    c.bench_function("bvh_far_edge_hit_survives_float_rounding", |b| {
        b.iter(do_bvh_far_edge_hit_survives_float_rounding)
    });
    c.bench_function("bvh_point_in_subtree_aabb_but_outside_own_area", |b| {
        b.iter(do_bvh_point_in_subtree_aabb_but_outside_own_area)
    });
    c.bench_function("descendant_near_slack_expands_hit_region", |b| {
        b.iter(do_descendant_near_slack_expands_hit_region)
    });
    c.bench_function("descendant_near_slack_smallest_area_still_wins", |b| {
        b.iter(do_descendant_near_slack_smallest_area_still_wins)
    });
    // SpatialDescend event delivery //
    c.bench_function("spatial_descend_delivers_event_to_leaf", |b| {
        b.iter(do_spatial_descend_delivers_event_to_leaf)
    });
    c.bench_function("spatial_descend_prefers_the_smallest_overlapping_box", |b| {
        b.iter(do_spatial_descend_prefers_the_smallest_overlapping_box)
    });
    c.bench_function("spatial_descend_skips_a_box_with_no_extent", |b| {
        b.iter(do_spatial_descend_skips_a_box_with_no_extent)
    });
    c.bench_function("spatial_descend_miss_is_noop", |b| {
        b.iter(do_spatial_descend_miss_is_noop)
    });
    c.bench_function("spatial_descend_finds_innermost_node", |b| {
        b.iter(do_spatial_descend_finds_innermost_node)
    });
    c.bench_function("spatial_descend_deep_chain_delivers_to_leaf", |b| {
        b.iter(do_spatial_descend_deep_chain_delivers_to_leaf)
    });
    c.bench_function("spatial_descend_wide_tree_hits_correct_child", |b| {
        b.iter(do_spatial_descend_wide_tree_hits_correct_child)
    });
    c.bench_function("spatial_descend_no_mutation_is_noop", |b| {
        b.iter(do_spatial_descend_no_mutation_is_noop)
    });
    c.bench_function("spatial_descend_ignores_channel_mismatch", |b| {
        b.iter(do_spatial_descend_ignores_channel_mismatch)
    });
    c.bench_function("apply_to_redirties_subtree_aabbs_after_spatial_descend", |b| {
        b.iter(do_apply_to_redirties_subtree_aabbs_after_spatial_descend)
    });
    c.bench_function("walker_redirties_subtree_aabbs_between_spatial_descends", |b| {
        b.iter(do_walker_redirties_subtree_aabbs_between_spatial_descends)
    });
    c.bench_function("mouse_event_data_round_trips_constructor_inputs", |b| {
        b.iter(do_mouse_event_data_round_trips_constructor_inputs)
    });
    c.bench_function("glyph_tree_event_mouse_carries_data", |b| {
        b.iter(do_glyph_tree_event_mouse_carries_data)
    });
    // MapChildren zip walker //
    c.bench_function("one_to_one_zip_applies_each_child_by_position", |b| {
        b.iter(do_one_to_one_zip_applies_each_child_by_position)
    });
    c.bench_function("zip_shorter_mutator_than_target", |b| {
        b.iter(do_zip_shorter_mutator_than_target)
    });
    c.bench_function("zip_shorter_target_than_mutator", |b| {
        b.iter(do_zip_shorter_target_than_mutator)
    });
    c.bench_function("zip_empty_mutator_children_is_noop", |b| {
        b.iter(do_zip_empty_mutator_children_is_noop)
    });
    c.bench_function("zip_empty_target_children_is_noop", |b| {
        b.iter(do_zip_empty_target_children_is_noop)
    });
    c.bench_function("zip_empty_both_sides_is_noop", |b| {
        b.iter(do_zip_empty_both_sides_is_noop)
    });
    c.bench_function("nested_map_children_descends_recursively", |b| {
        b.iter(do_nested_map_children_descends_recursively)
    });
    c.bench_function("instruction_carrying_mutation_applies_to_current_target", |b| {
        b.iter(do_instruction_carrying_mutation_applies_to_current_target)
    });
    c.bench_function("instruction_mutation_skipped_on_channel_mismatch", |b| {
        b.iter(do_instruction_mutation_skipped_on_channel_mismatch)
    });
    c.bench_function("compose_repeat_inside_map_children", |b| {
        b.iter(do_compose_repeat_inside_map_children)
    });
    c.bench_function("map_children_ignores_sibling_channels_when_unequal_counts", |b| {
        b.iter(do_map_children_ignores_sibling_channels_when_unequal_counts)
    });
    // the document: shape rules, identity, resolution and its read
    // records, the predicate surface, the bounds //
    c.bench_function("document_bounds_ornament", |b| {
        b.iter(do_document_bounds_ornament)
    });
    c.bench_function("document_bounds_canvas_coord", |b| {
        b.iter(do_document_bounds_canvas_coord)
    });
    c.bench_function("document_bounds_font_size_pt", |b| {
        b.iter(do_document_bounds_font_size_pt)
    });
    c.bench_function("document_bounds_rel_size", |b| {
        b.iter(do_document_bounds_rel_size)
    });
    c.bench_function("document_bounds_unit_interval", |b| {
        b.iter(do_document_bounds_unit_interval)
    });
    c.bench_function("document_bounds_non_negative", |b| {
        b.iter(do_document_bounds_non_negative)
    });
    c.bench_function("document_bounds_fit", |b| b.iter(do_document_bounds_fit));
    c.bench_function("document_bounds_percent", |b| b.iter(do_document_bounds_percent));
    c.bench_function("document_bounds_weight", |b| b.iter(do_document_bounds_weight));
    c.bench_function("document_bounds_control_points", |b| {
        b.iter(do_document_bounds_control_points)
    });
    c.bench_function("document_bounds_computed_font_sizes", |b| {
        b.iter(do_document_bounds_computed_font_sizes)
    });
    c.bench_function("document_bounds_extent", |b| b.iter(do_document_bounds_extent));
    c.bench_function("document_bounds_zoom_window", |b| {
        b.iter(do_document_bounds_zoom_window)
    });
    c.bench_function("document_bounds_channel", |b| b.iter(do_document_bounds_channel));
    c.bench_function("document_kind_order_is_the_declared_order", |b| {
        b.iter(do_document_kind_order_is_the_declared_order)
    });
    c.bench_function("document_rows_are_laid_out_by_kind", |b| {
        b.iter(do_document_rows_are_laid_out_by_kind)
    });
    c.bench_function("document_walk_routes_by_kind", |b| {
        b.iter(do_document_walk_routes_by_kind)
    });
    c.bench_function("document_anchored_walk_reaches_the_selection", |b| {
        b.iter(do_document_anchored_walk_reaches_the_selection)
    });
    c.bench_function("document_walk_moves_a_rekeyed_element", |b| {
        b.iter(do_document_walk_moves_a_rekeyed_element)
    });
    c.bench_function("document_element_keys", |b| b.iter(do_document_element_keys));
    c.bench_function("document_idents_index_every_node", |b| {
        b.iter(do_document_idents_index_every_node)
    });
    c.bench_function("document_ident_counter_starts_past_the_highest", |b| {
        b.iter(do_document_ident_counter_starts_past_the_highest)
    });
    c.bench_function("document_refuses_a_duplicate_or_unassigned_ident", |b| {
        b.iter(do_document_refuses_a_duplicate_or_unassigned_ident)
    });
    c.bench_function("document_tree_reads", |b| b.iter(do_document_tree_reads));
    c.bench_function("document_depth_counts_node_ancestors", |b| {
        b.iter(do_document_depth_counts_node_ancestors)
    });
    c.bench_function("document_owning_node", |b| b.iter(do_document_owning_node));
    c.bench_function("document_map_reads", |b| b.iter(do_document_map_reads));
    c.bench_function("document_inspect_kind_and_channel", |b| {
        b.iter(do_document_inspect_kind_and_channel)
    });
    c.bench_function("document_inspect_a_field_the_kind_lacks_falls_through", |b| {
        b.iter(do_document_inspect_a_field_the_kind_lacks_falls_through)
    });
    c.bench_function("document_inspect_spacing_reads_frames_and_rings", |b| {
        b.iter(do_document_inspect_spacing_reads_frames_and_rings)
    });
    c.bench_function("document_inspect_optional_fields", |b| {
        b.iter(do_document_inspect_optional_fields)
    });
    c.bench_function("document_inspect_values_are_stored_not_resolved", |b| {
        b.iter(do_document_inspect_values_are_stored_not_resolved)
    });
    c.bench_function("document_inspect_text_fields_across_kinds", |b| {
        b.iter(do_document_inspect_text_fields_across_kinds)
    });
    c.bench_function("document_field_json_spelling", |b| {
        b.iter(do_document_field_json_spelling)
    });
    c.bench_function("document_resolve_own_field_wins", |b| {
        b.iter(do_document_resolve_own_field_wins)
    });
    c.bench_function("document_resolve_owner_chain", |b| {
        b.iter(do_document_resolve_owner_chain)
    });
    c.bench_function("document_resolve_ring_owner_chain", |b| {
        b.iter(do_document_resolve_ring_owner_chain)
    });
    c.bench_function("document_resolve_ring_layout_chain", |b| {
        b.iter(do_document_resolve_ring_layout_chain)
    });
    c.bench_function("document_resolve_ring_sides_color", |b| {
        b.iter(do_document_resolve_ring_sides_color)
    });
    c.bench_function("document_resolve_ring_sides_var_records_the_theme", |b| {
        b.iter(do_document_resolve_ring_sides_var_records_the_theme)
    });
    c.bench_function("document_resolve_ring_once_filler_is_its_sides", |b| {
        b.iter(do_document_resolve_ring_once_filler_is_its_sides)
    });
    c.bench_function("document_resolve_defaults_tier", |b| {
        b.iter(do_document_resolve_defaults_tier)
    });
    c.bench_function("document_resolve_built_in_tier", |b| {
        b.iter(do_document_resolve_built_in_tier)
    });
    c.bench_function("document_resolve_is_per_field", |b| {
        b.iter(do_document_resolve_is_per_field)
    });
    c.bench_function("document_resolve_built_in_by_flow", |b| {
        b.iter(do_document_resolve_built_in_by_flow)
    });
    c.bench_function("document_resolve_path_and_portal_text_tiers", |b| {
        b.iter(do_document_resolve_path_and_portal_text_tiers)
    });
    c.bench_function("document_resolve_ring_size_never_reads_points", |b| {
        b.iter(do_document_resolve_ring_size_never_reads_points)
    });
    c.bench_function("document_resolve_palette_by_depth", |b| {
        b.iter(do_document_resolve_palette_by_depth)
    });
    c.bench_function("document_resolve_palette_clamps_and_nearest_binding_wins", |b| {
        b.iter(do_document_resolve_palette_clamps_and_nearest_binding_wins)
    });
    c.bench_function("document_resolve_palette_field_mapping", |b| {
        b.iter(do_document_resolve_palette_field_mapping)
    });
    c.bench_function("document_resolve_explicit_palette_reference", |b| {
        b.iter(do_document_resolve_explicit_palette_reference)
    });
    c.bench_function("document_resolve_var_through_the_active_variant", |b| {
        b.iter(do_document_resolve_var_through_the_active_variant)
    });
    c.bench_function("document_records_only_the_span_for_a_complete_rgba_span", |b| {
        b.iter(do_document_records_only_the_span_for_a_complete_rgba_span)
    });
    c.bench_function("document_records_every_tier_it_looks_at", |b| {
        b.iter(do_document_records_every_tier_it_looks_at)
    });
    c.bench_function("document_records_the_palette_binding", |b| {
        b.iter(do_document_records_the_palette_binding)
    });
    c.bench_function("document_records_node_and_frame_reads", |b| {
        b.iter(do_document_records_node_and_frame_reads)
    });
    c.bench_function("document_resolve_connector_content", |b| {
        b.iter(do_document_resolve_connector_content)
    });
    c.bench_function("document_resolve_portal_end_content_and_position", |b| {
        b.iter(do_document_resolve_portal_end_content_and_position)
    });
    c.bench_function("document_resolve_frame_ring_path_and_label", |b| {
        b.iter(do_document_resolve_frame_ring_path_and_label)
    });
    c.bench_function("document_resolve_node_chain", |b| {
        b.iter(do_document_resolve_node_chain)
    });
    c.bench_function(
        "document_resolve_carried_binding_resolves_what_the_climb_does",
        |b| b.iter(do_document_resolve_carried_binding_resolves_what_the_climb_does),
    );
    c.bench_function("document_resolve_background", |b| {
        b.iter(do_document_resolve_background)
    });
    c.bench_function("document_shape_accepts_the_full_example", |b| {
        b.iter(do_document_shape_accepts_the_full_example)
    });
    c.bench_function("document_shape_map_row", |b| b.iter(do_document_shape_map_row));
    c.bench_function("document_shape_node_row", |b| b.iter(do_document_shape_node_row));
    c.bench_function("document_bounds_node_ident", |b| {
        b.iter(do_document_bounds_node_ident)
    });
    c.bench_function("document_bounds_counts", |b| b.iter(do_document_bounds_counts));
    c.bench_function("document_resolve_cap_takes_its_path", |b| {
        b.iter(do_document_resolve_cap_takes_its_path)
    });
    c.bench_function("document_journal_reverts_and_replays_exactly", |b| {
        b.iter(do_document_journal_reverts_and_replays_exactly)
    });
    c.bench_function("document_same_state_sees_every_field", |b| {
        b.iter(do_document_same_state_sees_every_field)
    });
    c.bench_function("document_write_interpolated", |b| {
        b.iter(do_document_write_interpolated)
    });
    c.bench_function("document_interpolate_blends_numbers_and_colors", |b| {
        b.iter(do_document_interpolate_blends_numbers_and_colors)
    });
    // the document's mutation vocabulary (V1-B3) //
    c.bench_function("document_interpolation_follows_the_listed_fields", |b| {
        b.iter(do_document_interpolation_follows_the_listed_fields)
    });
    c.bench_function("document_node_ops_write_their_field", |b| {
        b.iter(do_document_node_ops_write_their_field)
    });
    c.bench_function("document_section_and_span_ops_write_their_field", |b| {
        b.iter(do_document_section_and_span_ops_write_their_field)
    });
    c.bench_function("document_frame_and_ring_ops_write_their_field", |b| {
        b.iter(do_document_frame_and_ring_ops_write_their_field)
    });
    c.bench_function("document_path_ops_write_their_field", |b| {
        b.iter(do_document_path_ops_write_their_field)
    });
    c.bench_function("document_cap_label_and_portal_end_ops_write_their_field", |b| {
        b.iter(do_document_cap_label_and_portal_end_ops_write_their_field)
    });
    c.bench_function(
        "document_map_theme_palette_and_defaults_ops_write_their_field",
        |b| b.iter(do_document_map_theme_palette_and_defaults_ops_write_their_field),
    );
    c.bench_function(
        "document_relative_size_ops_clamp_and_take_the_place_s_unit",
        |b| b.iter(do_document_relative_size_ops_clamp_and_take_the_place_s_unit),
    );
    c.bench_function("document_seed_matches_the_resolver_for_every_kind", |b| {
        b.iter(do_document_seed_matches_the_resolver_for_every_kind)
    });
    c.bench_function(
        "document_walk_from_the_map_matches_the_resolver_for_every_kind",
        |b| b.iter(do_document_walk_from_the_map_matches_the_resolver_for_every_kind),
    );
    c.bench_function("document_grow_size_grows_the_inherited_size", |b| {
        b.iter(do_document_grow_size_grows_the_inherited_size)
    });
    c.bench_function("document_grow_size_never_crosses_a_flow", |b| {
        b.iter(do_document_grow_size_never_crosses_a_flow)
    });
    c.bench_function("document_validate_refuses_a_payload_of_another_kind", |b| {
        b.iter(do_document_validate_refuses_a_payload_of_another_kind)
    });
    c.bench_function("document_validate_refuses_every_literal_out_of_bounds", |b| {
        b.iter(do_document_validate_refuses_every_literal_out_of_bounds)
    });
    c.bench_function("document_background_is_opaque_or_a_variable", |b| {
        b.iter(do_document_background_is_opaque_or_a_variable)
    });
    c.bench_function("document_validate_holds_sizes_to_their_unit", |b| {
        b.iter(do_document_validate_holds_sizes_to_their_unit)
    });
    c.bench_function("document_validate_refuses_a_missing_reference", |b| {
        b.iter(do_document_validate_refuses_a_missing_reference)
    });
    c.bench_function("document_validate_holds_structure_to_the_document_s_rules", |b| {
        b.iter(do_document_validate_holds_structure_to_the_document_s_rules)
    });
    c.bench_function(
        "document_validate_refuses_positional_and_spatial_instructions",
        |b| b.iter(do_document_validate_refuses_positional_and_spatial_instructions),
    );
    c.bench_function("document_validate_refuses_a_bare_kind_switch", |b| {
        b.iter(do_document_validate_refuses_a_bare_kind_switch)
    });
    c.bench_function("document_presentations_change_fields_only", |b| {
        b.iter(do_document_presentations_change_fields_only)
    });
    c.bench_function("document_validate_applied_judges_what_the_steps_did", |b| {
        b.iter(do_document_validate_applied_judges_what_the_steps_did)
    });
    c.bench_function("document_run_steps_validates_and_is_all_or_nothing", |b| {
        b.iter(do_document_run_steps_validates_and_is_all_or_nothing)
    });
    c.bench_function("document_validate_refuses_the_fallback_font", |b| {
        b.iter(do_document_validate_refuses_the_fallback_font)
    });
    c.bench_function("document_validate_bounds_channels", |b| {
        b.iter(do_document_validate_bounds_channels)
    });
    c.bench_function("document_validate_refuses_an_inverted_range", |b| {
        b.iter(do_document_validate_refuses_an_inverted_range)
    });
    c.bench_function("document_validate_applied_judges_nested_links", |b| {
        b.iter(do_document_validate_applied_judges_nested_links)
    });
    c.bench_function("document_validate_applied_skips_removed_parents", |b| {
        b.iter(do_document_validate_applied_skips_removed_parents)
    });
    c.bench_function("document_validate_walks_a_deep_mutator", |b| {
        b.iter(do_document_validate_walks_a_deep_mutator)
    });
    c.bench_function("document_validate_walks_a_deep_insert", |b| {
        b.iter(do_document_validate_walks_a_deep_insert)
    });
    c.bench_function("projection_full_projection_of_a_generated_map", |b| {
        b.iter(do_projection_full_projection_of_a_generated_map)
    });
    c.bench_function("projection_update_after_a_single_node_edit", |b| {
        b.iter(do_projection_update_after_a_single_node_edit)
    });
    c.bench_function("projection_nodes_in_bands_over_a_grid", |b| {
        b.iter(do_projection_nodes_in_bands_over_a_grid)
    });
    c.bench_function("projection_deep_chain_projects_edits_hides_moves_and_hits", |b| {
        b.iter(do_projection_deep_chain_projects_edits_hides_moves_and_hits)
    });
    c.bench_function("document_deep_chain_full_walk", |b| {
        b.iter(do_document_deep_chain_full_walk)
    });
    c.bench_function("document_deep_chain_repeat_while_to_the_leaf", |b| {
        b.iter(do_document_deep_chain_repeat_while_to_the_leaf)
    });
    c.bench_function("document_deep_chain_journaled_edit_at_the_leaf", |b| {
        b.iter(do_document_deep_chain_journaled_edit_at_the_leaf)
    });
    c.bench_function("document_deep_chain_moves_whole", |b| {
        b.iter(do_document_deep_chain_moves_whole)
    });
    c.bench_function("document_deep_subtree_validates_and_inserts", |b| {
        b.iter(do_document_deep_subtree_validates_and_inserts)
    });
    c.bench_function("document_deep_chain_resolves_at_the_leaf", |b| {
        b.iter(do_document_deep_chain_resolves_at_the_leaf)
    });
    c.bench_function("document_deep_chain_through_the_writer", |b| {
        b.iter(do_document_deep_chain_through_the_writer)
    });
    c.bench_function("writer_refuses_a_request_whole", |b| {
        b.iter(do_writer_refuses_a_request_whole)
    });
    c.bench_function(
        "writer_undo_and_redo_are_exact_for_every_composite_builder",
        |b| b.iter(do_writer_undo_and_redo_are_exact_for_every_composite_builder),
    );
    c.bench_function("writer_makes_one_entry_of_a_request_with_many_anchors", |b| {
        b.iter(do_writer_makes_one_entry_of_a_request_with_many_anchors)
    });
    c.bench_function("document_reads_nothing_a_change_removed", |b| {
        b.iter(do_document_reads_nothing_a_change_removed)
    });
    c.bench_function("writer_dirty_is_the_saved_state_not_a_count", |b| {
        b.iter(do_writer_dirty_is_the_saved_state_not_a_count)
    });
    c.bench_function("writer_history_keeps_its_limit_and_frees_what_leaves_it", |b| {
        b.iter(do_writer_history_keeps_its_limit_and_frees_what_leaves_it)
    });
    c.bench_function("writer_removes_the_links_into_what_a_request_removes", |b| {
        b.iter(do_writer_removes_the_links_into_what_a_request_removes)
    });
    c.bench_function("writer_gives_inserted_nodes_fresh_idents", |b| {
        b.iter(do_writer_gives_inserted_nodes_fresh_idents)
    });
    c.bench_function("writer_numbers_a_nested_insert_once_in_pre_order", |b| {
        b.iter(do_writer_numbers_a_nested_insert_once_in_pre_order)
    });
    c.bench_function("writer_stops_at_the_last_ident", |b| {
        b.iter(do_writer_stops_at_the_last_ident)
    });
    c.bench_function("writer_feed_names_what_each_change_touched", |b| {
        b.iter(do_writer_feed_names_what_each_change_touched)
    });
    c.bench_function(
        "writer_feed_reports_input_kinds_and_undo_and_redo_report_them_again",
        |b| b.iter(do_writer_feed_reports_input_kinds_and_undo_and_redo_report_them_again),
    );
    c.bench_function(
        "writer_feed_a_stage_replacing_a_stage_reports_both_journals",
        |b| b.iter(do_writer_feed_a_stage_replacing_a_stage_reports_both_journals),
    );
    c.bench_function(
        "writer_feed_removing_a_first_section_reports_its_nodes_row",
        |b| b.iter(do_writer_feed_removing_a_first_section_reports_its_nodes_row),
    );
    c.bench_function("writer_feed_a_move_reports_both_parents", |b| {
        b.iter(do_writer_feed_a_move_reports_both_parents)
    });
    c.bench_function(
        "writer_feed_a_move_and_a_restyle_in_one_request_report_the_style",
        |b| b.iter(do_writer_feed_a_move_and_a_restyle_in_one_request_report_the_style),
    );
    c.bench_function("writer_holds_edits_to_the_platform_budget", |b| {
        b.iter(do_writer_holds_edits_to_the_platform_budget)
    });
    c.bench_function("writer_starts_from_what_the_loader_derived", |b| {
        b.iter(do_writer_starts_from_what_the_loader_derived)
    });
    c.bench_function("writer_refuses_the_sources_a_change_cannot_declare", |b| {
        b.iter(do_writer_refuses_the_sources_a_change_cannot_declare)
    });
    c.bench_function(
        "writer_session_of_edits_undos_and_transactions_stays_exact",
        |b| b.iter(do_writer_session_of_edits_undos_and_transactions_stays_exact),
    );
    c.bench_function("writer_apply_of_a_fifty_anchor_request", |b| {
        b.iter(do_writer_apply_of_a_fifty_anchor_request)
    });
    c.bench_function("writer_thirty_stages_commit_as_one_apply_of_the_last", |b| {
        b.iter(do_writer_thirty_stages_commit_as_one_apply_of_the_last)
    });
    c.bench_function("writer_a_handle_from_before_an_undo_reads_not_open", |b| {
        b.iter(do_writer_a_handle_from_before_an_undo_reads_not_open)
    });
    c.bench_function("writer_cancel_restores_the_document", |b| {
        b.iter(do_writer_cancel_restores_the_document)
    });
    c.bench_function("writer_one_transaction_at_a_time", |b| {
        b.iter(do_writer_one_transaction_at_a_time)
    });
    c.bench_function("writer_undo_and_redo_cancel_an_open_transaction", |b| {
        b.iter(do_writer_undo_and_redo_cancel_an_open_transaction)
    });
    c.bench_function("writer_base_keeps_a_created_node_across_stages", |b| {
        b.iter(do_writer_base_keeps_a_created_node_across_stages)
    });
    c.bench_function("writer_saves_the_committed_state_with_a_stage_open", |b| {
        b.iter(do_writer_saves_the_committed_state_with_a_stage_open)
    });
    c.bench_function("writer_an_animated_transaction_yields_by_finishing", |b| {
        b.iter(do_writer_an_animated_transaction_yields_by_finishing)
    });
    c.bench_function("writer_records_every_stage_commit_and_cancel", |b| {
        b.iter(do_writer_records_every_stage_commit_and_cancel)
    });
    c.bench_function("document_compose_normalizes_in_log_steps", |b| {
        b.iter(do_document_compose_normalizes_in_log_steps)
    });
    c.bench_function("document_compose_trims_a_deep_selection_once_per_ancestor", |b| {
        b.iter(do_document_compose_trims_a_deep_selection_once_per_ancestor)
    });
    c.bench_function("document_compose_creates_nodes", |b| {
        b.iter(do_document_compose_creates_nodes)
    });
    c.bench_function("document_compose_deletes_nodes_and_links", |b| {
        b.iter(do_document_compose_deletes_nodes_and_links)
    });
    c.bench_function("document_compose_reparents", |b| {
        b.iter(do_document_compose_reparents)
    });
    c.bench_function("document_compose_removes_elements", |b| {
        b.iter(do_document_compose_removes_elements)
    });
    c.bench_function("document_compose_connects", |b| {
        b.iter(do_document_compose_connects)
    });
    c.bench_function("document_compose_replaces_spans", |b| {
        b.iter(do_document_compose_replaces_spans)
    });
    c.bench_function("document_compose_switches_link_kind", |b| {
        b.iter(do_document_compose_switches_link_kind)
    });
    c.bench_function("document_compose_styles_a_range", |b| {
        b.iter(do_document_compose_styles_a_range)
    });
    c.bench_function(
        "document_compose_sets_inserts_and_adds_a_section_with_spans",
        |b| b.iter(do_document_compose_sets_inserts_and_adds_a_section_with_spans),
    );
    c.bench_function("document_compose_normalizes_spans", |b| {
        b.iter(do_document_compose_normalizes_spans)
    });
    c.bench_function("document_compose_inserts_and_deletes_text", |b| {
        b.iter(do_document_compose_inserts_and_deletes_text)
    });
    c.bench_function("document_compose_splits_and_adds_sections", |b| {
        b.iter(do_document_compose_splits_and_adds_sections)
    });
    c.bench_function("document_never_reissues_an_ident", |b| {
        b.iter(do_document_never_reissues_an_ident)
    });
    c.bench_function("document_refuses_more_than_an_index_addresses", |b| {
        b.iter(do_document_refuses_more_than_an_index_addresses)
    });
    c.bench_function(
        "document_validate_row_is_order_independent_and_names_no_id",
        |b| b.iter(do_document_validate_row_is_order_independent_and_names_no_id),
    );
    c.bench_function("document_validate_subtree_judges_every_row", |b| {
        b.iter(do_document_validate_subtree_judges_every_row)
    });
    c.bench_function("document_span_splits_and_merges_at_grapheme_boundaries", |b| {
        b.iter(do_document_span_splits_and_merges_at_grapheme_boundaries)
    });
    c.bench_function(
        "document_span_merge_is_refused_for_different_styles_or_links",
        |b| b.iter(do_document_span_merge_is_refused_for_different_styles_or_links),
    );
    c.bench_function("document_span_split_and_merge_are_journaled_commands", |b| {
        b.iter(do_document_span_split_and_merge_are_journaled_commands)
    });
    c.bench_function("document_geometry_touches_no_input", |b| {
        b.iter(do_document_geometry_touches_no_input)
    });
    c.bench_function("document_inputs_changed_by_kind", |b| {
        b.iter(do_document_inputs_changed_by_kind)
    });
    c.bench_function("document_records_and_changes_agree", |b| {
        b.iter(do_document_records_and_changes_agree)
    });
    c.bench_function("document_resolve_node_text_stays_out_of_other_flows", |b| {
        b.iter(do_document_resolve_node_text_stays_out_of_other_flows)
    });
    c.bench_function("document_resolve_untitled_group_titles_with_text", |b| {
        b.iter(do_document_resolve_untitled_group_titles_with_text)
    });
    c.bench_function("document_resolve_first_section_reads_the_row", |b| {
        b.iter(do_document_resolve_first_section_reads_the_row)
    });
    c.bench_function("document_shape_section_row", |b| {
        b.iter(do_document_shape_section_row)
    });
    c.bench_function("document_shape_frame_row", |b| {
        b.iter(do_document_shape_frame_row)
    });
    c.bench_function("document_shape_span_holder_rows", |b| {
        b.iter(do_document_shape_span_holder_rows)
    });
    c.bench_function("document_shape_connector_row", |b| {
        b.iter(do_document_shape_connector_row)
    });
    c.bench_function("document_shape_line_link_row", |b| {
        b.iter(do_document_shape_line_link_row)
    });
    c.bench_function("document_shape_portal_link_row", |b| {
        b.iter(do_document_shape_portal_link_row)
    });
    c.bench_function("document_shape_portal_end_row", |b| {
        b.iter(do_document_shape_portal_end_row)
    });
    c.bench_function("document_shape_leaf_rows", |b| {
        b.iter(do_document_shape_leaf_rows)
    });
    c.bench_function("document_shape_checks_only_its_own_row", |b| {
        b.iter(do_document_shape_checks_only_its_own_row)
    });
    // the format: load and save //
    c.bench_function("format_fixtures_round_trip_byte_for_byte", |b| {
        b.iter(do_format_fixtures_round_trip_byte_for_byte)
    });
    c.bench_function("format_full_fixture_reads_as_written", |b| {
        b.iter(do_format_full_fixture_reads_as_written)
    });
    c.bench_function("format_builder_documents_round_trip", |b| {
        b.iter(do_format_builder_documents_round_trip)
    });
    // TextFlow: shaping, outlines and the arc-length walk, placement,
    // and the fit modes //
    c.bench_function("flow_fit_fills_a_line_exactly", |b| {
        b.iter(do_flow_fit_fills_a_line_exactly)
    });
    c.bench_function("flow_fit_fills_a_steep_line_with_an_ornament", |b| {
        b.iter(do_flow_fit_fills_a_steep_line_with_an_ornament)
    });
    c.bench_function("flow_fit_clamped_at_max_spreads_the_rest", |b| {
        b.iter(do_flow_fit_clamped_at_max_spreads_the_rest)
    });
    c.bench_function("flow_fit_clamped_at_min_cuts_without_overlap", |b| {
        b.iter(do_flow_fit_clamped_at_min_cuts_without_overlap)
    });
    c.bench_function("flow_repeat_reuses_glyph_indices", |b| {
        b.iter(do_flow_repeat_reuses_glyph_indices)
    });
    c.bench_function("flow_repeat_cuts_at_a_grapheme", |b| {
        b.iter(do_flow_repeat_cuts_at_a_grapheme)
    });
    c.bench_function("flow_once_with_a_filler_fills_the_rest", |b| {
        b.iter(do_flow_once_with_a_filler_fills_the_rest)
    });
    c.bench_function("flow_once_places_the_text_once", |b| {
        b.iter(do_flow_once_places_the_text_once)
    });
    c.bench_function("flow_spread_ends_on_a_line", |b| {
        b.iter(do_flow_spread_ends_on_a_line)
    });
    c.bench_function("flow_spread_too_long_is_cut", |b| {
        b.iter(do_flow_spread_too_long_is_cut)
    });
    c.bench_function(
        "flow_outline_corner_radius_is_a_share_of_half_the_shorter_side",
        |b| b.iter(do_flow_outline_corner_radius_is_a_share_of_half_the_shorter_side),
    );
    c.bench_function("flow_offset_grows_the_rect_and_the_radius", |b| {
        b.iter(do_flow_offset_grows_the_rect_and_the_radius)
    });
    c.bench_function("flow_ring_offset_sums_inner_rings_and_gaps", |b| {
        b.iter(do_flow_ring_offset_sums_inner_rings_and_gaps)
    });
    c.bench_function("flow_pill_has_straight_sides_of_w_minus_h", |b| {
        b.iter(do_flow_pill_has_straight_sides_of_w_minus_h)
    });
    c.bench_function("flow_start_points_on_a_rounded_rect", |b| {
        b.iter(do_flow_start_points_on_a_rounded_rect)
    });
    c.bench_function("flow_start_points_on_an_ellipse", |b| {
        b.iter(do_flow_start_points_on_an_ellipse)
    });
    c.bench_function("flow_ellipse_walk_stays_on_the_ellipse", |b| {
        b.iter(do_flow_ellipse_walk_stays_on_the_ellipse)
    });
    c.bench_function(
        "flow_offset_curve_holds_its_distance_round_a_corner_its_seam_splits_a_side_of",
        |b| b.iter(do_flow_offset_curve_holds_its_distance_round_a_corner_its_seam_splits_a_side_of),
    );
    c.bench_function("flow_bezier_walk_measures_arc_length", |b| {
        b.iter(do_flow_bezier_walk_measures_arc_length)
    });
    c.bench_function("flow_open_path_holds_its_ends", |b| {
        b.iter(do_flow_open_path_holds_its_ends)
    });
    c.bench_function("flow_outline_support_faces_its_direction", |b| {
        b.iter(do_flow_outline_support_faces_its_direction)
    });
    c.bench_function("flow_path_nearest_finds_its_own_points", |b| {
        b.iter(do_flow_path_nearest_finds_its_own_points)
    });
    c.bench_function("flow_path_measures_past_the_root_of_the_largest_f32", |b| {
        b.iter(do_flow_path_measures_past_the_root_of_the_largest_f32)
    });
    c.bench_function("flow_pitched_holds_the_nearer_whole_count_evenly", |b| {
        b.iter(do_flow_pitched_holds_the_nearer_whole_count_evenly)
    });
    c.bench_function("flow_pitched_keeps_one_ink_gap_in_every_direction", |b| {
        b.iter(do_flow_pitched_keeps_one_ink_gap_in_every_direction)
    });
    c.bench_function("flow_pitched_measures_each_end_to_the_boxes_beside_it", |b| {
        b.iter(do_flow_pitched_measures_each_end_to_the_boxes_beside_it)
    });
    c.bench_function("flow_pitched_measures_the_far_end_of_a_long_path", |b| {
        b.iter(do_flow_pitched_measures_the_far_end_of_a_long_path)
    });
    c.bench_function("flow_pitched_lays_no_text_and_no_empty_path", |b| {
        b.iter(do_flow_pitched_lays_no_text_and_no_empty_path)
    });
    c.bench_function("flow_place_horizontal_steps_by_advance", |b| {
        b.iter(do_flow_place_horizontal_steps_by_advance)
    });
    c.bench_function("flow_place_vertical_steps_by_line_height", |b| {
        b.iter(do_flow_place_vertical_steps_by_line_height)
    });
    c.bench_function("flow_place_diagonal_step_blends_advance_and_height", |b| {
        b.iter(do_flow_place_diagonal_step_blends_advance_and_height)
    });
    c.bench_function("flow_place_sharp_rect_touches_every_side", |b| {
        b.iter(do_flow_place_sharp_rect_touches_every_side)
    });
    c.bench_function("flow_place_round_square_is_a_circle", |b| {
        b.iter(do_flow_place_round_square_is_a_circle)
    });
    c.bench_function("flow_place_start_and_direction", |b| {
        b.iter(do_flow_place_start_and_direction)
    });
    c.bench_function("flow_place_rings_stack_outward", |b| {
        b.iter(do_flow_place_rings_stack_outward)
    });
    c.bench_function("flow_place_step_follows_the_chord_at_a_corner", |b| {
        b.iter(do_flow_place_step_follows_the_chord_at_a_corner)
    });
    c.bench_function("flow_place_on_an_ellipse", |b| {
        b.iter(do_flow_place_on_an_ellipse)
    });
    c.bench_function("flow_place_on_a_bezier", |b| b.iter(do_flow_place_on_a_bezier));
    c.bench_function("flow_place_at_point_centers_the_line", |b| {
        b.iter(do_flow_place_at_point_centers_the_line)
    });
    c.bench_function("flow_span_attrs_carry_weight_slant_and_size", |b| {
        b.iter(do_flow_span_attrs_carry_weight_slant_and_size)
    });
    c.bench_function("flow_shape_run_keeps_each_span_attributes", |b| {
        b.iter(do_flow_shape_run_keeps_each_span_attributes)
    });
    c.bench_function("flow_shape_run_sizes_each_span", |b| {
        b.iter(do_flow_shape_run_sizes_each_span)
    });
    c.bench_function("flow_shape_run_maps_glyphs_to_spans", |b| {
        b.iter(do_flow_shape_run_maps_glyphs_to_spans)
    });
    c.bench_function("flow_shape_run_marks_grapheme_starts", |b| {
        b.iter(do_flow_shape_run_marks_grapheme_starts)
    });
    c.bench_function("flow_shape_run_marks_letters", |b| {
        b.iter(do_flow_shape_run_marks_letters)
    });
    c.bench_function("flow_shape_run_measures_an_ornaments_ink", |b| {
        b.iter(do_flow_shape_run_measures_an_ornaments_ink)
    });
    c.bench_function("flow_shape_run_of_nothing", |b| {
        b.iter(do_flow_shape_run_of_nothing)
    });
    c.bench_function("flow_layout_glyphs_follow_glyphs", |b| {
        b.iter(do_flow_layout_glyphs_follow_glyphs)
    });
    c.bench_function("flow_baseline_centers_the_tallest_glyph", |b| {
        b.iter(do_flow_baseline_centers_the_tallest_glyph)
    });
    c.bench_function("flow_reshape_rescales_and_keeps_the_mapping", |b| {
        b.iter(do_flow_reshape_rescales_and_keeps_the_mapping)
    });
    c.bench_function("flow_reshape_appends_a_filler_on_its_own_line", |b| {
        b.iter(do_flow_reshape_appends_a_filler_on_its_own_line)
    });
    c.bench_function("flow_box_breaks_at_newlines", |b| {
        b.iter(do_flow_box_breaks_at_newlines)
    });
    c.bench_function("flow_box_counts_empty_lines", |b| {
        b.iter(do_flow_box_counts_empty_lines)
    });
    c.bench_function("flow_box_keeps_a_larger_minimum", |b| {
        b.iter(do_flow_box_keeps_a_larger_minimum)
    });
    c.bench_function("flow_box_aligns_lines", |b| b.iter(do_flow_box_aligns_lines));
    c.bench_function("flow_box_reads_a_bad_minimum_as_none", |b| {
        b.iter(do_flow_box_reads_a_bad_minimum_as_none)
    });
    c.bench_function("flow_curve_point_and_derivative_by_hand", |b| {
        b.iter(do_flow_curve_point_and_derivative_by_hand)
    });
    c.bench_function("flow_cut_keeps_the_reading_start", |b| {
        b.iter(do_flow_cut_keeps_the_reading_start)
    });
    c.bench_function("flow_place_corner_moves_continuously", |b| {
        b.iter(do_flow_place_corner_moves_continuously)
    });
    c.bench_function("flow_place_keeps_marks_with_their_letters", |b| {
        b.iter(do_flow_place_keeps_marks_with_their_letters)
    });
    c.bench_function("flow_separators_never_break_a_box", |b| {
        b.iter(do_flow_separators_never_break_a_box)
    });
    c.bench_function("flow_separators_never_break_a_run", |b| {
        b.iter(do_flow_separators_never_break_a_run)
    });
    c.bench_function("flow_separators_survive_a_reshape", |b| {
        b.iter(do_flow_separators_survive_a_reshape)
    });
    c.bench_function("flow_shape_run_maps_a_zwj_emoji", |b| {
        b.iter(do_flow_shape_run_maps_a_zwj_emoji)
    });
    c.bench_function("flow_shape_run_maps_bidi_text", |b| {
        b.iter(do_flow_shape_run_maps_bidi_text)
    });
    c.bench_function("flow_place_sets_glyphs_at_their_pen_positions", |b| {
        b.iter(do_flow_place_sets_glyphs_at_their_pen_positions)
    });
    c.bench_function("flow_fit_models_the_size_floor", |b| {
        b.iter(do_flow_fit_models_the_size_floor)
    });
    c.bench_function("flow_non_finite_curves_build_no_table", |b| {
        b.iter(do_flow_non_finite_curves_build_no_table)
    });
    c.bench_function("flow_a_curve_faster_than_the_largest_f32_is_tabled", |b| {
        b.iter(do_flow_a_curve_faster_than_the_largest_f32_is_tabled)
    });
    c.bench_function(
        "flow_a_ring_that_never_closes_counts_its_copies_in_few_asks",
        |b| b.iter(do_flow_a_ring_that_never_closes_counts_its_copies_in_few_asks),
    );
    c.bench_function("flow_place_keeps_every_position_finite", |b| {
        b.iter(do_flow_place_keeps_every_position_finite)
    });
    c.bench_function("flow_table_spacing_has_a_floor", |b| {
        b.iter(do_flow_table_spacing_has_a_floor)
    });
    // the readable rule (#291): every stretch reads forward //
    c.bench_function("flow_readable_every_stretch_reads_forward", |b| {
        b.iter(do_flow_readable_every_stretch_reads_forward)
    });
    c.bench_function("flow_readable_a_loop_reads_like_a_seal", |b| {
        b.iter(do_flow_readable_a_loop_reads_like_a_seal)
    });
    c.bench_function("flow_readable_keeps_the_walks_glyphs_in_their_stretches", |b| {
        b.iter(do_flow_readable_keeps_the_walks_glyphs_in_their_stretches)
    });
    c.bench_function(
        "flow_readable_hysteresis_keeps_a_flat_loop_to_two_stretches",
        |b| b.iter(do_flow_readable_hysteresis_keeps_a_flat_loop_to_two_stretches),
    );
    c.bench_function("flow_readable_keeps_a_torn_word_whole", |b| {
        b.iter(do_flow_readable_keeps_a_torn_word_whole)
    });
    c.bench_function("flow_readable_keeps_a_flows_last_word_whole", |b| {
        b.iter(do_flow_readable_keeps_a_flows_last_word_whole)
    });
    c.bench_function("flow_readable_parts_a_word_too_long_to_move", |b| {
        b.iter(do_flow_readable_parts_a_word_too_long_to_move)
    });
    c.bench_function("flow_readable_fit_keeps_a_connectors_text_whole", |b| {
        b.iter(do_flow_readable_fit_keeps_a_connectors_text_whole)
    });
    c.bench_function("flow_readable_cuts_copies_rather_than_overlap", |b| {
        b.iter(do_flow_readable_cuts_copies_rather_than_overlap)
    });
    c.bench_function("flow_readable_a_short_side_reads_top_to_bottom", |b| {
        b.iter(do_flow_readable_a_short_side_reads_top_to_bottom)
    });
    c.bench_function("flow_readable_stops_where_the_path_turns", |b| {
        b.iter(do_flow_readable_stops_where_the_path_turns)
    });
    c.bench_function("flow_readable_a_repeat_ring_closes_evenly", |b| {
        b.iter(do_flow_readable_a_repeat_ring_closes_evenly)
    });
    c.bench_function("flow_readable_an_ornament_ring_is_evenly_inked", |b| {
        b.iter(do_flow_readable_an_ornament_ring_is_evenly_inked)
    });
    // a text frame in two lines (#291, ruling R27) //
    c.bench_function("flow_lines_balance_a_sentence", |b| {
        b.iter(do_flow_lines_balance_a_sentence)
    });
    c.bench_function("flow_lines_part_at_a_line_break", |b| {
        b.iter(do_flow_lines_part_at_a_line_break)
    });
    c.bench_function("flow_lines_keep_one_word_on_top", |b| {
        b.iter(do_flow_lines_keep_one_word_on_top)
    });
    c.bench_function("flow_lines_fit_to_the_fuller_line", |b| {
        b.iter(do_flow_lines_fit_to_the_fuller_line)
    });
    c.bench_function("flow_lines_cut_the_bottom_line_at_min", |b| {
        b.iter(do_flow_lines_cut_the_bottom_line_at_min)
    });
    c.bench_function("flow_lines_spread_over_their_spans", |b| {
        b.iter(do_flow_lines_spread_over_their_spans)
    });
    c.bench_function("flow_lines_center_and_fill", |b| {
        b.iter(do_flow_lines_center_and_fill)
    });
    c.bench_function("flow_lines_fill_a_frame_as_wide_as_an_f32_reaches", |b| {
        b.iter(do_flow_lines_fill_a_frame_as_wide_as_an_f32_reaches)
    });
    c.bench_function("flow_lines_fit_sweeps_a_sharp_rect", |b| {
        b.iter(do_flow_lines_fit_sweeps_a_sharp_rect)
    });
    c.bench_function("flow_lines_spread_sweeps_a_sharp_rect", |b| {
        b.iter(do_flow_lines_spread_sweeps_a_sharp_rect)
    });
    c.bench_function("flow_lines_space_glyphs_level", |b| {
        b.iter(do_flow_lines_space_glyphs_level)
    });
    c.bench_function("flow_lines_never_overlap_on_the_steepest_slope", |b| {
        b.iter(do_flow_lines_never_overlap_on_the_steepest_slope)
    });
    // a text frame's lines along its diagonal halves (#291, rulings
    // R32, R33, R33.1) //
    c.bench_function("flow_halves_track_evenly_and_read_down", |b| {
        b.iter(do_flow_halves_track_evenly_and_read_down)
    });
    c.bench_function("flow_halves_junctions_are_breaks", |b| {
        b.iter(do_flow_halves_junctions_are_breaks)
    });
    c.bench_function("flow_halves_span_moves_the_ends_monotonically", |b| {
        b.iter(do_flow_halves_span_moves_the_ends_monotonically)
    });
    c.bench_function("flow_halves_tracking_grows_with_the_gap", |b| {
        b.iter(do_flow_halves_tracking_grows_with_the_gap)
    });
    c.bench_function(
        "flow_halves_closed_form_roots_are_the_first_at_what_is_owed",
        |b| b.iter(do_flow_halves_closed_form_roots_are_the_first_at_what_is_owed),
    );
    c.bench_function("flow_halves_a_side_arc_is_filled_from_its_model_lays", |b| {
        b.iter(do_flow_halves_a_side_arc_is_filled_from_its_model_lays)
    });
    c.bench_function("flow_halves_sides_hold_the_junctions", |b| {
        b.iter(do_flow_halves_sides_hold_the_junctions)
    });
    c.bench_function("flow_halves_no_line_is_laid_tighter_than_its_font", |b| {
        b.iter(do_flow_halves_no_line_is_laid_tighter_than_its_font)
    });
    c.bench_function("flow_halves_a_repeat_ring_takes_its_spacing", |b| {
        b.iter(do_flow_halves_a_repeat_ring_takes_its_spacing)
    });
    c.bench_function("flow_halves_a_resolved_ring_lays_as_it_says", |b| {
        b.iter(do_flow_halves_a_resolved_ring_lays_as_it_says)
    });
    c.bench_function("flow_halves_hold_on_every_aspect", |b| {
        b.iter(do_flow_halves_hold_on_every_aspect)
    });
    c.bench_function("flow_halves_a_crowded_frame_shrinks_or_cuts", |b| {
        b.iter(do_flow_halves_a_crowded_frame_shrinks_or_cuts)
    });
    c.bench_function(
        "flow_halves_ascenders_on_a_lower_half_hang_off_one_baseline",
        |b| b.iter(do_flow_halves_ascenders_on_a_lower_half_hang_off_one_baseline),
    );
    c.bench_function("flow_halves_a_line_keeps_its_half", |b| {
        b.iter(do_flow_halves_a_line_keeps_its_half)
    });
    c.bench_function("flow_halves_a_glyph_with_no_outline_is_laid", |b| {
        b.iter(do_flow_halves_a_glyph_with_no_outline_is_laid)
    });
    c.bench_function("flow_halves_spread_and_once_and_inward", |b| {
        b.iter(do_flow_halves_spread_and_once_and_inward)
    });
    c.bench_function("flow_halves_once_holds_what_r27_cuts", |b| {
        b.iter(do_flow_halves_once_holds_what_r27_cuts)
    });
    c.bench_function("flow_halves_a_junction_holds_at_most_its_copies", |b| {
        b.iter(do_flow_halves_a_junction_holds_at_most_its_copies)
    });
    c.bench_function("flow_halves_a_part_never_inverts", |b| {
        b.iter(do_flow_halves_a_part_never_inverts)
    });
    c.bench_function("flow_halves_a_frame_as_long_as_an_f32_is_placed_finitely", |b| {
        b.iter(do_flow_halves_a_frame_as_long_as_an_f32_is_placed_finitely)
    });
    c.bench_function("flow_halves_an_end_past_a_sharp_corner_faces_away_from_it", |b| {
        b.iter(do_flow_halves_an_end_past_a_sharp_corner_faces_away_from_it)
    });
    c.bench_function("flow_halves_a_copy_at_its_floor_clears_its_neighbors", |b| {
        b.iter(do_flow_halves_a_copy_at_its_floor_clears_its_neighbors)
    });
    c.bench_function("flow_halves_a_one_word_line_keeps_its_neighbor_whole", |b| {
        b.iter(do_flow_halves_a_one_word_line_keeps_its_neighbor_whole)
    });
    c.bench_function("flow_halves_a_frame_its_lines_can_hold_is_not_shrunk", |b| {
        b.iter(do_flow_halves_a_frame_its_lines_can_hold_is_not_shrunk)
    });
    c.bench_function(
        "flow_halves_the_break_holds_below_whole_round_a_flat_outline_inward",
        |b| b.iter(do_flow_halves_the_break_holds_below_whole_round_a_flat_outline_inward),
    );
    c.bench_function("flow_halves_a_side_that_would_touch_is_left_out", |b| {
        b.iter(do_flow_halves_a_side_that_would_touch_is_left_out)
    });
    c.bench_function("flow_halves_the_break_holds_at_the_span_laid", |b| {
        b.iter(do_flow_halves_the_break_holds_at_the_span_laid)
    });
    c.bench_function(
        "flow_halves_an_outline_whose_halves_are_not_numbers_keeps_r27s_lines",
        |b| b.iter(do_flow_halves_an_outline_whose_halves_are_not_numbers_keeps_r27s_lines),
    );
    // the engine over a non-glyph element: the walker matrix, inherited
    // state, and the builder //
    c.bench_function("engine_walk_aligns_mutators_by_channel", |b| {
        b.iter(do_engine_walk_aligns_mutators_by_channel)
    });
    c.bench_function("engine_walk_broadcasts_to_equal_channels", |b| {
        b.iter(do_engine_walk_broadcasts_to_equal_channels)
    });
    c.bench_function("engine_walk_void_gates_descent_without_applying", |b| {
        b.iter(do_engine_walk_void_gates_descent_without_applying)
    });
    c.bench_function("engine_walk_nested_macros_apply_in_order", |b| {
        b.iter(do_engine_walk_nested_macros_apply_in_order)
    });
    c.bench_function("engine_repeat_while_stops_where_the_predicate_fails", |b| {
        b.iter(do_engine_repeat_while_stops_where_the_predicate_fails)
    });
    c.bench_function("engine_repeat_while_judges_each_branch_on_its_own", |b| {
        b.iter(do_engine_repeat_while_judges_each_branch_on_its_own)
    });
    c.bench_function("engine_repeat_while_resumes_with_the_terminator", |b| {
        b.iter(do_engine_repeat_while_resumes_with_the_terminator)
    });
    c.bench_function("engine_repeat_while_descends_by_kind", |b| {
        b.iter(do_engine_repeat_while_descends_by_kind)
    });
    c.bench_function("engine_map_children_pairs_by_position", |b| {
        b.iter(do_engine_map_children_pairs_by_position)
    });
    c.bench_function(
        "engine_apply_invalidates_the_cache_per_mutation_and_once_more",
        |b| b.iter(do_engine_apply_invalidates_the_cache_per_mutation_and_once_more),
    );
    c.bench_function("engine_walk_wide_and_deep_tree", |b| {
        b.iter(do_engine_walk_wide_and_deep_tree)
    });
    c.bench_function("engine_deep_chain_full_walk", |b| {
        b.iter(do_engine_deep_chain_full_walk)
    });
    c.bench_function("engine_deep_chain_repeat_while_to_the_leaf", |b| {
        b.iter(do_engine_deep_chain_repeat_while_to_the_leaf)
    });
    c.bench_function("engine_deep_chain_anchored_walks", |b| {
        b.iter(do_engine_deep_chain_anchored_walks)
    });
    c.bench_function("engine_deep_chain_journaled_edit_at_the_leaf", |b| {
        b.iter(do_engine_deep_chain_journaled_edit_at_the_leaf)
    });
    c.bench_function("engine_deep_chain_moves_whole", |b| {
        b.iter(do_engine_deep_chain_moves_whole)
    });
    c.bench_function("engine_deep_subtree_inserts_and_is_a_value", |b| {
        b.iter(do_engine_deep_subtree_inserts_and_is_a_value)
    });
    c.bench_function("engine_deep_walks_inherit_once_per_level", |b| {
        b.iter(do_engine_deep_walks_inherit_once_per_level)
    });
    c.bench_function("engine_contains_follows_removal_revert_replay_and_discard", |b| {
        b.iter(do_engine_contains_follows_removal_revert_replay_and_discard)
    });
    c.bench_function("engine_subtree_impls_match_the_derived_ones", |b| {
        b.iter(do_engine_subtree_impls_match_the_derived_ones)
    });
    c.bench_function(
        "builder_ast_from_json_nests_no_deeper_than_serde_json_allows",
        |b| b.iter(do_builder_ast_from_json_nests_no_deeper_than_serde_json_allows),
    );
    c.bench_function("a_journal_is_not_reverted_under_a_later_one", |b| {
        b.iter(do_a_journal_is_not_reverted_under_a_later_one)
    });
    c.bench_function("an_empty_journal_leaves_the_history_restorable", |b| {
        b.iter(do_an_empty_journal_leaves_the_history_restorable)
    });
    c.bench_function("an_unjournaled_structural_change_ends_the_history", |b| {
        b.iter(do_an_unjournaled_structural_change_ends_the_history)
    });
    c.bench_function("an_edit_between_one_builders_calls_ends_the_history", |b| {
        b.iter(do_an_edit_between_one_builders_calls_ends_the_history)
    });
    c.bench_function("interleaved_builders_end_the_history", |b| {
        b.iter(do_interleaved_builders_end_the_history)
    });
    c.bench_function("an_empty_builder_closed_after_another_keeps_its_change", |b| {
        b.iter(do_an_empty_builder_closed_after_another_keeps_its_change)
    });
    c.bench_function("engine_journaled_walk_wide_and_deep_tree", |b| {
        b.iter(do_engine_journaled_walk_wide_and_deep_tree)
    });
    // journals: exact revert / replay, one entry per element, positions,
    // all-or-nothing calls //
    c.bench_function("journal_revert_and_replay_are_exact", |b| {
        b.iter(do_journal_revert_and_replay_are_exact)
    });
    c.bench_function("journal_records_one_entry_per_element", |b| {
        b.iter(do_journal_records_one_entry_per_element)
    });
    c.bench_function("journal_of_an_apply_that_changes_nothing_is_empty", |b| {
        b.iter(do_journal_of_an_apply_that_changes_nothing_is_empty)
    });
    c.bench_function("journal_records_an_element_touched_twice_in_one_walk_once", |b| {
        b.iter(do_journal_records_an_element_touched_twice_in_one_walk_once)
    });
    c.bench_function("journal_restores_the_order_of_siblings_a_walk_rekeyed", |b| {
        b.iter(do_journal_restores_the_order_of_siblings_a_walk_rekeyed)
    });
    c.bench_function(
        "journal_restores_an_element_moved_twice_around_another_move",
        |b| b.iter(do_journal_restores_an_element_moved_twice_around_another_move),
    );
    c.bench_function(
        "journal_restores_an_element_whose_neighbors_moved_before_it_was_recorded",
        |b| b.iter(do_journal_restores_an_element_whose_neighbors_moved_before_it_was_recorded),
    );
    c.bench_function(
        "journal_restores_an_element_a_walk_moved_a_sibling_in_front_of",
        |b| b.iter(do_journal_restores_an_element_a_walk_moved_a_sibling_in_front_of),
    );
    c.bench_function("journal_puts_back_an_element_that_followed_a_moved_one", |b| {
        b.iter(do_journal_puts_back_an_element_that_followed_a_moved_one)
    });
    c.bench_function("journal_of_a_move_there_and_back_is_empty", |b| {
        b.iter(do_journal_of_a_move_there_and_back_is_empty)
    });
    c.bench_function(
        "a_failing_journaled_call_leaves_the_tree_as_the_builder_found_it",
        |b| b.iter(do_a_failing_journaled_call_leaves_the_tree_as_the_builder_found_it),
    );
    c.bench_function("abandon_reverts_everything_the_builder_recorded", |b| {
        b.iter(do_abandon_reverts_everything_the_builder_recorded)
    });
    c.bench_function("revert_and_replay_refuse_a_stale_node_before_writing", |b| {
        b.iter(do_revert_and_replay_refuse_a_stale_node_before_writing)
    });
    c.bench_function("journal_names_every_change_as_a_field_touch", |b| {
        b.iter(do_journal_names_every_change_as_a_field_touch)
    });
    c.bench_function("revert_refuses_a_tree_the_journal_does_not_fit", |b| {
        b.iter(do_revert_refuses_a_tree_the_journal_does_not_fit)
    });
    c.bench_function("journal_skips_elements_visited_but_not_applied_to", |b| {
        b.iter(do_journal_skips_elements_visited_but_not_applied_to)
    });
    // interpolation: Interpolate, Snap, write_interpolated //
    c.bench_function("interpolate_returns_the_endpoints_exactly", |b| {
        b.iter(do_interpolate_returns_the_endpoints_exactly)
    });
    c.bench_function("interpolate_is_linear_between_the_endpoints", |b| {
        b.iter(do_interpolate_is_linear_between_the_endpoints)
    });
    c.bench_function("snap_switches_where_it_says", |b| {
        b.iter(do_snap_switches_where_it_says)
    });
    c.bench_function("write_interpolated_at_the_ends_equals_revert_and_replay", |b| {
        b.iter(do_write_interpolated_at_the_ends_equals_revert_and_replay)
    });
    c.bench_function("write_interpolated_halfway_lerps_value_and_snaps_the_rest", |b| {
        b.iter(do_write_interpolated_halfway_lerps_value_and_snaps_the_rest)
    });
    // journals and interpolation over the glyph instantiation //
    c.bench_function("gfx_same_state_compares_every_state_field", |b| {
        b.iter(do_gfx_same_state_compares_every_state_field)
    });
    c.bench_function("gfx_clone_keeps_every_field", |b| {
        b.iter(do_gfx_clone_keeps_every_field)
    });
    c.bench_function("gfx_journal_reverts_and_replays_a_glyph_walk_exactly", |b| {
        b.iter(do_gfx_journal_reverts_and_replays_a_glyph_walk_exactly)
    });
    c.bench_function("gfx_interpolates_geometry_and_snaps_the_rest", |b| {
        b.iter(do_gfx_interpolates_geometry_and_snaps_the_rest)
    });
    // glyph runs and fills: the two glyph-layer element types //
    c.bench_function("run_construction_validates_override_lengths", |b| {
        b.iter(do_run_construction_validates_override_lengths)
    });
    c.bench_function("run_bounds_are_the_union_of_the_placed_glyph_boxes", |b| {
        b.iter(do_run_bounds_are_the_union_of_the_placed_glyph_boxes)
    });
    c.bench_function("run_origin_moves_the_bounds_and_nothing_else", |b| {
        b.iter(do_run_origin_moves_the_bounds_and_nothing_else)
    });
    c.bench_function("run_opacity_sets_only_the_opacity", |b| {
        b.iter(do_run_opacity_sets_only_the_opacity)
    });
    c.bench_function("run_set_replaces_the_run_and_recomputes_its_bounds", |b| {
        b.iter(do_run_set_replaces_the_run_and_recomputes_its_bounds)
    });
    c.bench_function(
        "run_override_ops_write_their_range_and_materialize_the_rest_neutral",
        |b| b.iter(do_run_override_ops_write_their_range_and_materialize_the_rest_neutral),
    );
    c.bench_function("run_override_ranges_are_clamped_to_the_placed_glyphs", |b| {
        b.iter(do_run_override_ranges_are_clamped_to_the_placed_glyphs)
    });
    c.bench_function("run_override_ops_skip_filler_glyphs", |b| {
        b.iter(do_run_override_ops_skip_filler_glyphs)
    });
    c.bench_function("run_set_reuses_the_override_buffers", |b| {
        b.iter(do_run_set_reuses_the_override_buffers)
    });
    c.bench_function("run_clone_from_reuses_the_override_buffers", |b| {
        b.iter(do_run_clone_from_reuses_the_override_buffers)
    });
    c.bench_function("run_element_answers_position_and_flags_like_any_element", |b| {
        b.iter(do_run_element_answers_position_and_flags_like_any_element)
    });
    c.bench_function("authored_run_mutations_leave_other_elements_unchanged", |b| {
        b.iter(do_authored_run_mutations_leave_other_elements_unchanged)
    });
    c.bench_function(
        "run_mutations_leave_other_elements_alone_and_other_mutations_leave_runs_alone",
        |b| b.iter(do_run_mutations_leave_other_elements_alone_and_other_mutations_leave_runs_alone),
    );
    c.bench_function("glyph_layer_same_state_compares_every_state_field", |b| {
        b.iter(do_glyph_layer_same_state_compares_every_state_field)
    });
    c.bench_function("glyph_layer_elements_deliver_events_to_their_subscribers", |b| {
        b.iter(do_glyph_layer_elements_deliver_events_to_their_subscribers)
    });
    c.bench_function(
        "run_interpolates_origin_opacity_and_overrides_and_snaps_the_shape",
        |b| b.iter(do_run_interpolates_origin_opacity_and_overrides_and_snaps_the_shape),
    );
    c.bench_function("interpolation_between_other_variants_still_snaps_whole", |b| {
        b.iter(do_interpolation_between_other_variants_still_snaps_whole)
    });
    c.bench_function("predicates_select_runs_by_element_type_and_opacity", |b| {
        b.iter(do_predicates_select_runs_by_element_type_and_opacity)
    });
    c.bench_function("descendant_at_misses_a_rings_empty_interior", |b| {
        b.iter(do_descendant_at_misses_a_rings_empty_interior)
    });
    c.bench_function("descendant_at_misses_a_diagonal_connections_empty_bounds", |b| {
        b.iter(do_descendant_at_misses_a_diagonal_connections_empty_bounds)
    });
    c.bench_function("descendant_at_finds_runs_by_their_boxes", |b| {
        b.iter(do_descendant_at_finds_runs_by_their_boxes)
    });
    c.bench_function("descendants_aabb_covers_runs", |b| {
        b.iter(do_descendants_aabb_covers_runs)
    });
    c.bench_function("journal_round_trips_a_tree_of_runs", |b| {
        b.iter(do_journal_round_trips_a_tree_of_runs)
    });
    c.bench_function("authored_glyph_layer_mutations_have_the_wire_shape", |b| {
        b.iter(do_authored_glyph_layer_mutations_have_the_wire_shape)
    });
    c.bench_function("format_doc_publishes_the_authored_glyph_layer_mutations", |b| {
        b.iter(do_format_doc_publishes_the_authored_glyph_layer_mutations)
    });
    c.bench_function("hostile_authored_values_degrade_without_panicking", |b| {
        b.iter(do_hostile_authored_values_degrade_without_panicking)
    });
    c.bench_function("hit_test_reaches_a_run_in_a_deep_chain", |b| {
        b.iter(do_hit_test_reaches_a_run_in_a_deep_chain)
    });
    // structural commands: insert, remove, move, split, merge //
    c.bench_function("structure_move_under_its_own_subtree_is_a_cycle", |b| {
        b.iter(do_structure_move_under_its_own_subtree_is_a_cycle)
    });
    c.bench_function(
        "structure_a_command_on_a_node_removed_earlier_in_the_walk_fails",
        |b| b.iter(do_structure_a_command_on_a_node_removed_earlier_in_the_walk_fails),
    );
    c.bench_function("structure_merge_refuses_a_sibling_with_children", |b| {
        b.iter(do_structure_merge_refuses_a_sibling_with_children)
    });
    c.bench_function("structure_refusals_name_what_is_wrong_and_change_nothing", |b| {
        b.iter(do_structure_refusals_name_what_is_wrong_and_change_nothing)
    });
    c.bench_function("structure_commands_run_after_the_walk", |b| {
        b.iter(do_structure_commands_run_after_the_walk)
    });
    c.bench_function("structure_inserts_land_where_their_placement_says", |b| {
        b.iter(do_structure_inserts_land_where_their_placement_says)
    });
    c.bench_function("structure_revert_and_replay_put_back_the_same_nodes", |b| {
        b.iter(do_structure_revert_and_replay_put_back_the_same_nodes)
    });
    c.bench_function("structure_split_then_merge_round_trips_a_value", |b| {
        b.iter(do_structure_split_then_merge_round_trips_a_value)
    });
    c.bench_function("structure_insert_then_rekey_reverts_and_replays_exactly", |b| {
        b.iter(do_structure_insert_then_rekey_reverts_and_replays_exactly)
    });
    c.bench_function("structure_rekey_then_insert_reverts_and_replays_exactly", |b| {
        b.iter(do_structure_rekey_then_insert_reverts_and_replays_exactly)
    });
    c.bench_function(
        "structure_a_reparented_node_is_restored_against_both_parents",
        |b| b.iter(do_structure_a_reparented_node_is_restored_against_both_parents),
    );
    c.bench_function("structure_touched_names_whole_subtrees_in_pre_order", |b| {
        b.iter(do_structure_touched_names_whole_subtrees_in_pre_order)
    });
    c.bench_function("structure_touched_reads_each_node_s_own_membership", |b| {
        b.iter(do_structure_touched_reads_each_node_s_own_membership)
    });
    c.bench_function(
        "structure_discard_stales_a_detached_node_and_keeps_an_attached_one",
        |b| b.iter(do_structure_discard_stales_a_detached_node_and_keeps_an_attached_one),
    );
    c.bench_function("structure_discard_frees_by_the_side_last_written", |b| {
        b.iter(do_structure_discard_frees_by_the_side_last_written)
    });
    c.bench_function(
        "structure_discard_leaves_a_subtree_a_later_journal_removed",
        |b| b.iter(do_structure_discard_leaves_a_subtree_a_later_journal_removed),
    );
    c.bench_function("structure_snaps_in_write_interpolated", |b| {
        b.iter(do_structure_snaps_in_write_interpolated)
    });
    c.bench_function(
        "structure_without_a_journal_is_all_or_nothing_and_frees_what_it_removes",
        |b| b.iter(do_structure_without_a_journal_is_all_or_nothing_and_frees_what_it_removes),
    );
    c.bench_function("structure_is_never_serialized", |b| {
        b.iter(do_structure_is_never_serialized)
    });
    c.bench_function("structure_batch_on_a_wide_tree", |b| {
        b.iter(do_structure_batch_on_a_wide_tree)
    });
    c.bench_function("structure_discard_frees_exactly_what_the_journal_holds", |b| {
        b.iter(do_structure_discard_frees_exactly_what_the_journal_holds)
    });
    c.bench_function("structure_journal_names_the_parents_a_node_left_and_took", |b| {
        b.iter(do_structure_journal_names_the_parents_a_node_left_and_took)
    });
    c.bench_function("structure_touched_names_an_entering_subtree_from_its_top", |b| {
        b.iter(do_structure_touched_names_an_entering_subtree_from_its_top)
    });
    c.bench_function(
        "structure_touched_names_a_removed_subtree_in_one_pre_order",
        |b| b.iter(do_structure_touched_names_a_removed_subtree_in_one_pre_order),
    );
    c.bench_function("structure_walk_tree_from_refuses_a_node_out_of_the_tree", |b| {
        b.iter(do_structure_walk_tree_from_refuses_a_node_out_of_the_tree)
    });
    c.bench_function("structure_rebuilding_a_row_as_it_stood_before_is_one_pass", |b| {
        b.iter(do_structure_rebuilding_a_row_as_it_stood_before_is_one_pass)
    });
    c.bench_function("journal_rows_before_name_each_siblings_predecessor", |b| {
        b.iter(do_journal_rows_before_name_each_siblings_predecessor)
    });
    c.bench_function("journal_builder_names_what_it_recorded_and_created", |b| {
        b.iter(do_journal_builder_names_what_it_recorded_and_created)
    });
    c.bench_function("structure_discard_frees_only_what_is_still_detached", |b| {
        b.iter(do_structure_discard_frees_only_what_is_still_detached)
    });
    c.bench_function("structure_refuses_a_split_or_merge_that_changes_the_key", |b| {
        b.iter(do_structure_refuses_a_split_or_merge_that_changes_the_key)
    });
    c.bench_function(
        "gfx_structure_inserts_removes_and_moves_but_never_splits_or_merges",
        |b| b.iter(do_gfx_structure_inserts_removes_and_moves_but_never_splits_or_merges),
    );
    // routing keys, the sorted-children invariant and anchored walks //
    c.bench_function("engine_walk_routes_by_kind_and_channel", |b| {
        b.iter(do_engine_walk_routes_by_kind_and_channel)
    });
    c.bench_function("engine_walk_merges_rows_of_two_kinds", |b| {
        b.iter(do_engine_walk_merges_rows_of_two_kinds)
    });
    c.bench_function("branch_key_json_has_two_required_fields", |b| {
        b.iter(do_branch_key_json_has_two_required_fields)
    });
    c.bench_function("mutator_and_ast_json_carry_the_kind", |b| {
        b.iter(do_mutator_and_ast_json_carry_the_kind)
    });
    c.bench_function(
        "engine_build_repeat_emits_its_kind_on_consecutive_channels",
        |b| b.iter(do_engine_build_repeat_emits_its_kind_on_consecutive_channels),
    );
    c.bench_function("engine_build_refuses_a_template_of_another_kind", |b| {
        b.iter(do_engine_build_refuses_a_template_of_another_kind)
    });
    c.bench_function("engine_build_inserts_children_in_key_order", |b| {
        b.iter(do_engine_build_inserts_children_in_key_order)
    });
    c.bench_function("insert_child_places_within_the_run_of_its_key", |b| {
        b.iter(do_insert_child_places_within_the_run_of_its_key)
    });
    c.bench_function("insert_child_refuses_a_placement_across_keys", |b| {
        b.iter(do_insert_child_refuses_a_placement_across_keys)
    });
    c.bench_function("update_moves_an_element_whose_key_changed", |b| {
        b.iter(do_update_moves_an_element_whose_key_changed)
    });
    c.bench_function("update_of_a_stale_node_calls_nothing", |b| {
        b.iter(do_update_of_a_stale_node_calls_nothing)
    });
    c.bench_function("remove_subtree_frees_the_subtree_and_keeps_the_row", |b| {
        b.iter(do_remove_subtree_frees_the_subtree_and_keeps_the_row)
    });
    c.bench_function("walk_moves_the_children_whose_key_it_changed", |b| {
        b.iter(do_walk_moves_the_children_whose_key_it_changed)
    });
    c.bench_function("walk_sees_the_row_as_it_was_until_it_ends", |b| {
        b.iter(do_walk_sees_the_row_as_it_was_until_it_ends)
    });
    c.bench_function("check_sorted_finds_a_row_out_of_order", |b| {
        b.iter(do_check_sorted_finds_a_row_out_of_order)
    });
    c.bench_function("every_tree_method_that_changes_it_invalidates_the_cache", |b| {
        b.iter(do_every_tree_method_that_changes_it_invalidates_the_cache)
    });
    c.bench_function("apply_at_drops_anchors_nested_in_another", |b| {
        b.iter(do_apply_at_drops_anchors_nested_in_another)
    });
    c.bench_function("apply_at_decides_nesting_among_anchors_of_the_root_kind", |b| {
        b.iter(do_apply_at_decides_nesting_among_anchors_of_the_root_kind)
    });
    c.bench_function("anchors_are_resolved_once_and_walked_in_tree_order", |b| {
        b.iter(do_anchors_are_resolved_once_and_walked_in_tree_order)
    });
    c.bench_function("apply_at_walks_a_repeated_anchor_once", |b| {
        b.iter(do_apply_at_walks_a_repeated_anchor_once)
    });
    c.bench_function("reaches_below_root_reads_the_shape", |b| {
        b.iter(do_reaches_below_root_reads_the_shape)
    });
    c.bench_function("apply_at_matches_the_root_by_kind_alone", |b| {
        b.iter(do_apply_at_matches_the_root_by_kind_alone)
    });
    c.bench_function("apply_to_anchors_at_the_root", |b| {
        b.iter(do_apply_to_anchors_at_the_root)
    });
    c.bench_function("parent_of_resolves_to_the_distinct_parents", |b| {
        b.iter(do_parent_of_resolves_to_the_distinct_parents)
    });
    c.bench_function("siblings_of_resolves_to_same_kind_siblings", |b| {
        b.iter(do_siblings_of_resolves_to_same_kind_siblings)
    });
    c.bench_function("apply_at_seeds_every_anchor_before_walking_any", |b| {
        b.iter(do_apply_at_seeds_every_anchor_before_walking_any)
    });
    c.bench_function("a_stale_anchor_applies_nothing", |b| {
        b.iter(do_a_stale_anchor_applies_nothing)
    });
    c.bench_function("apply_at_moves_an_anchor_whose_key_it_changed", |b| {
        b.iter(do_apply_at_moves_an_anchor_whose_key_it_changed)
    });
    c.bench_function("inherited_state_reaches_a_grandchild", |b| {
        b.iter(do_inherited_state_reaches_a_grandchild)
    });
    c.bench_function("inherited_state_is_read_after_the_ancestors_own_mutations", |b| {
        b.iter(do_inherited_state_is_read_after_the_ancestors_own_mutations)
    });
    c.bench_function("inherited_state_leaves_a_sibling_subtree_alone", |b| {
        b.iter(do_inherited_state_leaves_a_sibling_subtree_alone)
    });
    c.bench_function("inherited_state_threads_through_repeat_while", |b| {
        b.iter(do_inherited_state_threads_through_repeat_while)
    });
    c.bench_function("inherited_state_threads_through_map_children", |b| {
        b.iter(do_inherited_state_threads_through_map_children)
    });
    c.bench_function("inherited_state_reaches_the_terminator", |b| {
        b.iter(do_inherited_state_reaches_the_terminator)
    });
    c.bench_function("inherited_state_reaches_a_spatial_descend_hit", |b| {
        b.iter(do_inherited_state_reaches_a_spatial_descend_hit)
    });
    c.bench_function("engine_build_resolves_templates_and_runtime_payloads", |b| {
        b.iter(do_engine_build_resolves_templates_and_runtime_payloads)
    });
    c.bench_function("engine_build_refuses_a_root_repeat", |b| {
        b.iter(do_engine_build_refuses_a_root_repeat)
    });
    c.bench_function("engine_build_refuses_spatial_descend_without_geometry", |b| {
        b.iter(do_engine_build_refuses_spatial_descend_without_geometry)
    });
    c.bench_function("repeat_literal_expands_to_consecutive_channels", |b| {
        b.iter(do_repeat_literal_expands_to_consecutive_channels)
    });
    c.bench_function("repeat_skip_indices_stride_channels", |b| {
        b.iter(do_repeat_skip_indices_stride_channels)
    });
    c.bench_function("multiple_sections_concatenate_in_declaration_order", |b| {
        b.iter(do_multiple_sections_concatenate_in_declaration_order)
    });
    c.bench_function("template_resolves_against_each_iterations_area", |b| {
        b.iter(do_template_resolves_against_each_iterations_area)
    });
    c.bench_function("iter_section_keys_walks_in_order", |b| {
        b.iter(do_iter_section_keys_walks_in_order)
    });
    c.bench_function("repeat_runtime_count_consults_context", |b| {
        b.iter(do_repeat_runtime_count_consults_context)
    });
    c.bench_function("void_root_with_no_children", |b| {
        b.iter(do_void_root_with_no_children)
    });
    c.bench_function("single_as_tree_root_with_literal_channel", |b| {
        b.iter(do_single_as_tree_root_with_literal_channel)
    });
    c.bench_function("mutation_src_runtime_consults_context", |b| {
        b.iter(do_mutation_src_runtime_consults_context)
    });
    c.bench_function("macro_pulls_mutation_list_from_runtime_context", |b| {
        b.iter(do_macro_pulls_mutation_list_from_runtime_context)
    });
    c.bench_function("instruction_with_children_mirrors_scope_topology", |b| {
        b.iter(do_instruction_with_children_mirrors_scope_topology)
    });
    c.bench_function(
        "instruction_spec_repeat_while_always_true_materializes_predicate",
        |b| b.iter(do_instruction_spec_repeat_while_always_true_materializes_predicate),
    );
    c.bench_function("instruction_spec_spatial_descend_carries_its_point", |b| {
        b.iter(do_instruction_spec_spatial_descend_carries_its_point)
    });
    c.bench_function("repeat_template_wrapping_void_threads_iter_channel", |b| {
        b.iter(do_repeat_template_wrapping_void_threads_iter_channel)
    });
    c.bench_function("repeat_at_tree_root_is_a_build_error", |b| {
        b.iter(do_repeat_at_tree_root_is_a_build_error)
    });
    c.bench_function("channel_src_section_index_outside_repeat_is_a_build_error", |b| {
        b.iter(do_channel_src_section_index_outside_repeat_is_a_build_error)
    });
    c.bench_function("template_outside_repeat_is_a_build_error", |b| {
        b.iter(do_template_outside_repeat_is_a_build_error)
    });
    c.bench_function("repeat_as_template_is_a_build_error", |b| {
        b.iter(do_repeat_as_template_is_a_build_error)
    });
    c.bench_function("spatial_descend_builds_only_for_a_spatial_element", |b| {
        b.iter(do_spatial_descend_builds_only_for_a_spatial_element)
    });
    c.bench_function("json_roundtrip_minimal_repeat_tree", |b| {
        b.iter(do_json_roundtrip_minimal_repeat_tree)
    });
    c.bench_function("json_skip_indices_defaults_to_empty", |b| {
        b.iter(do_json_skip_indices_defaults_to_empty)
    });
    c.bench_function("json_instruction_mutation_defaults_to_none", |b| {
        b.iter(do_json_instruction_mutation_defaults_to_none)
    });
    c.bench_function("json_mutation_src_runtime_carries_its_label", |b| {
        b.iter(do_json_mutation_src_runtime_carries_its_label)
    });
    c.bench_function("json_instruction_spec_map_children_materializes_correctly", |b| {
        b.iter(do_json_instruction_spec_map_children_materializes_correctly)
    });
    c.bench_function(
        "json_round_trip_macro_with_literal_mutations_preserves_shape",
        |b| b.iter(do_json_round_trip_macro_with_literal_mutations_preserves_shape),
    );
    c.bench_function("json_round_trip_instruction_map_children_preserves_shape", |b| {
        b.iter(do_json_round_trip_instruction_map_children_preserves_shape)
    });
    c.bench_function(
        "json_round_trip_repeat_inside_map_children_preserves_shape",
        |b| b.iter(do_json_round_trip_repeat_inside_map_children_preserves_shape),
    );
    c.bench_function("json_round_trip_repeat_while_predicate_preserves_shape", |b| {
        b.iter(do_json_round_trip_repeat_while_predicate_preserves_shape)
    });
    // camera viewport math //
    c.bench_function("canvas_to_screen_round_trips", |b| {
        b.iter(do_canvas_to_screen_round_trips)
    });
    c.bench_function("screen_to_canvas_identity_at_default", |b| {
        b.iter(do_screen_to_canvas_identity_at_default)
    });
    c.bench_function("zoom_at_preserves_point_under_cursor", |b| {
        b.iter(do_zoom_at_preserves_point_under_cursor)
    });
    c.bench_function("pan_shifts_viewport", |b| b.iter(do_pan_shifts_viewport));
    c.bench_function("fit_to_bounds_with_single_element", |b| {
        b.iter(do_fit_to_bounds_with_single_element)
    });
    c.bench_function("fit_to_bounds_empty", |b| b.iter(do_fit_to_bounds_empty));
    c.bench_function("camera_mutation_pan", |b| b.iter(do_camera_mutation_pan));
    c.bench_function("camera_mutation_zoom", |b| b.iter(do_camera_mutation_zoom));
    c.bench_function("camera_mutation_set_zoom_clamps_to_bounds", |b| {
        b.iter(do_camera_mutation_set_zoom_clamps_to_bounds)
    });
    c.bench_function("camera_mutation_fit_to_bounds_matches_imperative", |b| {
        b.iter(do_camera_mutation_fit_to_bounds_matches_imperative)
    });
    c.bench_function("camera_mutation_set_position_assigns_directly", |b| {
        b.iter(do_camera_mutation_set_position_assigns_directly)
    });
    c.bench_function("pixel_ratio_scales_pixels_not_zoom_windows", |b| {
        b.iter(do_pixel_ratio_scales_pixels_not_zoom_windows)
    });
    c.bench_function("fit_zoom_reads_the_view_not_the_surface", |b| {
        b.iter(do_fit_zoom_reads_the_view_not_the_surface)
    });
    // scene introspection (fixture load + build + dump) //
    c.bench_function("camera_visibility_names_each_reason", |b| {
        b.iter(do_visibility_names_each_reason)
    });
    c.bench_function("camera_visibility_agrees_with_the_conjunction_it_replaced", |b| {
        b.iter(do_visibility_agrees_with_the_conjunction_it_replaced)
    });
    // predicates + comparators //
    c.bench_function("comparator_equal_f32", |b| b.iter(do_comparator_equal_f32));
    c.bench_function("comparator_not_equal_f32", |b| {
        b.iter(do_comparator_not_equal_f32)
    });
    c.bench_function("comparator_less_than_f32", |b| {
        b.iter(do_comparator_less_than_f32)
    });
    c.bench_function("comparator_greater_than_f32", |b| {
        b.iter(do_comparator_greater_than_f32)
    });
    c.bench_function("comparator_greater_equal_f32", |b| {
        b.iter(do_comparator_greater_equal_f32)
    });
    c.bench_function("comparator_less_equal_f32", |b| {
        b.iter(do_comparator_less_equal_f32)
    });
    c.bench_function("predicate_always_true_matches_anything", |b| {
        b.iter(do_predicate_always_true_matches_anything)
    });
    c.bench_function("predicate_matches_field_value", |b| {
        b.iter(do_predicate_matches_field_value)
    });
    c.bench_function("predicate_text_with_greater_than_degrades_to_false", |b| {
        b.iter(do_predicate_text_with_greater_than_degrades_to_false)
    });
    c.bench_function("predicate_region_this_with_equals_degrades_to_false", |b| {
        b.iter(do_predicate_region_this_with_equals_degrades_to_false)
    });
    c.bench_function("predicate_region_font_with_greater_than_degrades_to_false", |b| {
        b.iter(do_predicate_region_font_with_greater_than_degrades_to_false)
    });
    c.bench_function("predicate_region_color_with_less_than_degrades_to_false", |b| {
        b.iter(do_predicate_region_color_with_less_than_degrades_to_false)
    });
    c.bench_function("predicate_glyph_lines_with_equals_degrades_to_false", |b| {
        b.iter(do_predicate_glyph_lines_with_equals_degrades_to_false)
    });
    c.bench_function(
        "predicate_glyph_matrix_with_greater_than_degrades_to_false",
        |b| b.iter(do_predicate_glyph_matrix_with_greater_than_degrades_to_false),
    );
    c.bench_function("predicate_glyph_matrix_with_less_than_degrades_to_false", |b| {
        b.iter(do_predicate_glyph_matrix_with_less_than_degrades_to_false)
    });
    c.bench_function("predicate_glyph_area_field_on_void_degrades_to_false", |b| {
        b.iter(do_predicate_glyph_area_field_on_void_degrades_to_false)
    });
    c.bench_function("predicate_flag_equals_matches_set_flag", |b| {
        b.iter(do_predicate_flag_equals_matches_set_flag)
    });
    c.bench_function("predicate_flag_equals_negated_matches_clear_flag", |b| {
        b.iter(do_predicate_flag_equals_negated_matches_clear_flag)
    });
    c.bench_function("predicate_flag_with_greater_than_degrades_to_false", |b| {
        b.iter(do_predicate_flag_with_greater_than_degrades_to_false)
    });
    c.bench_function("predicate_truth_table", |b| b.iter(do_predicate_truth_table));
    // GfxElement constructors + accessors //
    c.bench_function("new_area_constructs_glyph_area_variant", |b| {
        b.iter(do_new_area_constructs_glyph_area_variant)
    });
    c.bench_function("new_void_constructs_void_variant", |b| {
        b.iter(do_new_void_constructs_void_variant)
    });
    c.bench_function("flags_accessor_round_trips", |b| {
        b.iter(do_flags_accessor_round_trips)
    });
    c.bench_function("event_subscribers_add_and_check", |b| {
        b.iter(do_event_subscribers_add_and_check)
    });
    c.bench_function("event_subscribers_observe_dispatched_event", |b| {
        b.iter(do_event_subscribers_observe_dispatched_event)
    });
    c.bench_function("event_subscriber_can_capture_rc_refcell_state", |b| {
        b.iter(do_event_subscriber_can_capture_rc_refcell_state)
    });
    c.bench_function(
        "event_subscriber_can_mutate_subscriber_list_during_delivery",
        |b| b.iter(do_event_subscriber_can_mutate_subscriber_list_during_delivery),
    );
    // ordered_vec2 //
    c.bench_function("ordered_vec2_round_trips_through_hashmap", |b| {
        b.iter(do_ordered_vec2_round_trips_through_hashmap)
    });
    c.bench_function("ordered_vec2_distinguishes_close_floats_in_hashset", |b| {
        b.iter(do_ordered_vec2_distinguishes_close_floats_in_hashset)
    });
    // arena //
    c.bench_function("arena_matches_a_reference_model", |b| {
        b.iter(do_arena_matches_a_reference_model)
    });
    c.bench_function("arena_deep_chain_walks", |b| b.iter(do_arena_deep_chain_walks));
    c.bench_function("arena_deep_chain_owned_traits", |b| {
        b.iter(do_arena_deep_chain_owned_traits)
    });
    c.bench_function("arena_deep_chain_structural_operations", |b| {
        b.iter(do_arena_deep_chain_structural_operations)
    });
    // Includes building the fixture; `arena_every_operation_in_place`
    // below times the operations alone.
    c.bench_function("arena_operations_in_place", |b| {
        b.iter(do_arena_operations_in_place)
    });
    // A handful of `size_of`s compared with constants: the row is near
    // zero by design, and is here because §B8 gives every body one.
    c.bench_function("arena_slot_and_id_sizes", |b| b.iter(do_arena_slot_and_id_sizes));
    // Every structural operation and walk in an arena with room for
    // them (V1-A7, #301), the fixture built outside the timed body.
    c.bench_function("arena_every_operation_in_place", |b| {
        b.iter_batched(
            in_place_fixture,
            |mut fixture| every_operation_in_place(&mut fixture),
            criterion::BatchSize::LargeInput,
        )
    });
    // arena_utils //
    c.bench_function("arena_utils_clone", |b| b.iter(do_clone));
    // primes //
    c.bench_function("primes", |b| b.iter(do_primes));
    c.bench_function("is_prime_above_the_sieve_ceiling", |b| {
        b.iter(do_is_prime_above_the_sieve_ceiling)
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
