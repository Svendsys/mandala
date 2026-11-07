// SPDX-License-Identifier: MPL-2.0

//! The projection's fuzz properties held to account (TEST_CONVENTIONS
//! §T14): the loader targets' maps are mostly inside the projection's work
//! bound and both sides of it are reached (ruling B7.3), and the bound
//! refuses what it is there to refuse.

use serde_json::json;

use crate::projection::tests::maps::{load_map, node};
use crate::projection::tests::projection_fuzz::{projection_counts, within_bound, EXTENT};
use crate::util::tests::choices::Choices;

#[test]
fn test_most_structured_loader_maps_are_projected() {
    let (projected0, skipped0) = projection_counts();
    let mut loaded = 0;
    for seed in 0..120 {
        let mut c = Choices::seeded(0x0B7_0000 + seed);
        let generated = crate::document::tests::format_fuzz::check_generated(
            &mut c,
            crate::document::tests::format_fuzz::MAX_GENERATED_ROWS,
        )
        .unwrap_or_else(|why| panic!("seed {seed}: {why}"));
        if generated.outcome.ends_with("loaded") {
            loaded += 1;
        }
    }
    let (projected, skipped) = projection_counts();
    // Other tests on other threads count too: the counts only grow, and
    // this sweep's share is what it added at least.
    let (projected, skipped) = (projected - projected0, skipped - skipped0);
    assert!(loaded > 20, "the sweep loads maps: {loaded}");
    assert!(projected > 0, "some loaded maps are projected");
    eprintln!("projection of structured loader maps: {projected} projected, {skipped} skipped");
    assert!(
        skipped * 2 < projected + skipped,
        "under half the loaded maps are skipped ({skipped} of {})",
        projected + skipped
    );
}

#[test]
fn test_the_projection_bound_refuses_far_coordinates_and_a_second_text_ring() {
    let ring = |id: u32, x: f32| {
        node(
            id,
            None,
            json!({"position": [x, 0.0], "frame": {"rings": [{"spans": [{"text": "one ring"}]}]}}),
        )
    };
    assert!(
        within_bound(&load_map(json!({"nodes": [ring(1, 0.0)]}))),
        "one ring of text near the origin is inside"
    );
    assert!(
        !within_bound(&load_map(json!({"nodes": [ring(1, EXTENT * 2.0)]}))),
        "a node past the extent is outside"
    );
    assert!(
        !within_bound(&load_map(json!({"nodes": [ring(1, 0.0), ring(2, 300.0)]}))),
        "a second ring of text is outside"
    );
}
