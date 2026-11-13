// SPDX-License-Identifier: MPL-2.0

//! The projection's fuzz properties held to account (TEST_CONVENTIONS
//! §T14): the loader targets' maps are mostly inside the projection's work
//! bound and both sides of it are reached (ruling B7.3), and the bound
//! refuses what it is there to refuse.

use serde_json::json;

use crate::projection::tests::maps::{load_map, node};
use crate::projection::tests::projection_fuzz::{
    check_projects, projection_counts, under_ceiling, within_bound, EXTENT, MAX_UNITS, MAX_UNTANGLE_SAMPLES,
};
use crate::util::tests::choices::Choices;

#[test]
fn test_most_structured_loader_maps_are_projected() {
    let (projected0, skipped0, _) = projection_counts();
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
    // The counts are this thread's: exactly what this sweep added.
    let (projected, skipped, _) = projection_counts();
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
fn test_the_projection_bound_refuses_far_coordinates_a_second_text_ring_and_too_many_units() {
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
    let chain = |nodes: u32| {
        let records: Vec<_> = (1..=nodes)
            .map(|id| match id {
                1 => node(id, None, json!({})),
                _ => node(id, Some(id - 1), json!({"connector": {}})),
            })
            .collect();
        load_map(json!({ "nodes": records }))
    };
    // A chain of n nodes draws n - 1 connectors.
    let most = MAX_UNITS as u32 + 1;
    assert!(
        within_bound(&chain(most)),
        "a chain of {} connectors is inside",
        most - 1
    );
    assert!(!within_bound(&chain(most + 1)), "one connector more is outside");
}

/// `copies` of the first seed's map (`line_link_beside_connector_1`) side
/// by side, 3,000 apart, each with a second line link: a node 5,182.7
/// above its two children, with a line link at the default link size to
/// each, beside their connectors.
fn seed_copies(copies: u32) -> crate::document::ids::Document {
    let mut nodes = Vec::new();
    for k in 0..copies {
        let (x, id) = (3000.0 * k as f64 - 3000.0, 10 * k + 10);
        nodes.push(json!({"id": id, "position": [x, 5182.7446],
            "links": [{"to": id + 3, "kind": "line"}, {"to": id + 7, "kind": "line"}]}));
        nodes.push(json!({"id": id + 7, "parent": id, "position": [x, 0.0],
            "sections": [{"channel": 2, "spans": [{"text": "\u{fe0f}\u{20e3}", "size": 12.5}]}, {"channel": 3}, {"channel": 3}]}));
        nodes.push(
            json!({"id": id + 3, "parent": id, "position": [x, 0.0], "sections": [], "frame": {"rings": []},
            "connector": {"spans": []}, "links": [{"to": id, "kind": "portal"}]}),
        );
    }
    load_map(json!({"defaults": {"link": {"size": 0.5}}, "nodes": nodes}))
}

/// What `check_projects` did with `doc`: whether it projected it, and
/// whether it abandoned it as cut short, and the samples `untangle` took.
fn checked(doc: &crate::document::ids::Document) -> (bool, bool, usize) {
    use crate::projection::geometry::UNTANGLE_SAMPLED;
    let (projected0, _, cut0) = projection_counts();
    UNTANGLE_SAMPLED.with(|c| c.set(0));
    check_projects(doc).unwrap();
    let (projected, _, cut) = projection_counts();
    (
        projected > projected0,
        cut > cut0,
        UNTANGLE_SAMPLED.with(std::cell::Cell::get),
    )
}

#[test]
fn test_a_map_whose_untangling_reaches_the_ceiling_is_abandoned_and_counted() {
    // One copy of the first seed's map with a second line link takes
    // fewer samples in a check than `MAX_UNTANGLE_SAMPLES`, and is
    // projected and checked; two take more, and the check is abandoned
    // when they reach it, counted as cut short and not as projected, and
    // no more samples are taken than the ceiling and the one lane that
    // reached it — here at most three curves at the sampling's cap (#346).
    // The failing inputs are a ceiling half as high, which abandons the
    // one copy; samples never taken from it, which projects the two; and
    // samples not counted, which a cut, more than the ceiling sampled,
    // cannot be.
    for doc in [seed_copies(1), seed_copies(2)] {
        assert!(within_bound(&doc), "precondition: inside the coordinate bound");
    }
    let (projected, cut, taken) = checked(&seed_copies(1));
    assert!(
        projected && !cut && 0 < taken && taken <= MAX_UNTANGLE_SAMPLES,
        "one copy: projected {projected}, cut {cut}, {taken} samples"
    );
    let (projected, cut, taken) = checked(&seed_copies(2));
    assert!(
        !projected && cut && MAX_UNTANGLE_SAMPLES < taken && taken <= MAX_UNTANGLE_SAMPLES + 3 * 65_536,
        "two copies: projected {projected}, cut {cut}, {taken} samples"
    );
}

#[test]
fn test_the_untangle_ceiling_is_unset_but_for_a_check() {
    // A test that does not set the harness's ceiling untangles in full:
    // unset on a fresh thread, unset and unmarked again once a check is
    // done — one cut short, and one that panics. The failing inputs are
    // the ceiling left set after a check, and the ceiling and the mark
    // cleared only when a check returns.
    use crate::projection::geometry::{UNTANGLE_CEILING, UNTANGLE_CUT};
    let clear = || {
        (
            UNTANGLE_CEILING.with(std::cell::Cell::get),
            UNTANGLE_CUT.with(std::cell::Cell::get),
        )
    };
    assert_eq!(clear(), (None, false), "unset on a fresh thread");
    let (_, cut, _) = checked(&seed_copies(2));
    assert!(cut, "precondition: the check was cut short");
    assert_eq!(clear(), (None, false), "unset after a check cut short");
    let panicked = std::panic::catch_unwind(|| {
        under_ceiling(|| {
            UNTANGLE_CUT.with(|cut| cut.set(true));
            panic!("a check that panics");
        })
    });
    assert!(panicked.is_err(), "precondition: the check panicked");
    assert_eq!(clear(), (None, false), "unset after a check that panicked");
}

#[test]
fn test_the_untangle_ceiling_is_overshot_by_one_lane_at_most() {
    // A junction of 128 thin straight paths, each sampled at the cap, so
    // that its first round alone would take four times the ceiling: under
    // the ceiling, `untangle` takes at most the ceiling and the one lane
    // that reaches it — more than the ceiling, which is what cuts it — and
    // marks the check. The failing inputs are the ceiling taken from once
    // a round, after every lane of it is sampled, and samples not
    // counted.
    use crate::projection::geometry::{untangle, Lane, UNTANGLE_SAMPLED};
    use glam::Vec2;
    let lanes: Vec<Lane> = (0..128)
        .map(|k| Lane {
            start: (Vec2::new(k as f32, 0.0), Vec2::Y, 0.0),
            hold: 0.0,
            end: (Vec2::new(k as f32, 2000.0), -Vec2::Y, 0.0),
            via: Vec::new(),
            width: 0.01,
            straight: true,
        })
        .collect();
    UNTANGLE_SAMPLED.with(|c| c.set(0));
    let (_, cut) = under_ceiling(|| untangle(&lanes));
    let sampled = UNTANGLE_SAMPLED.with(std::cell::Cell::get);
    assert!(
        cut && 128 * 65_536 > 4 * MAX_UNTANGLE_SAMPLES,
        "precondition: cut, and one round four times the ceiling"
    );
    assert!(
        MAX_UNTANGLE_SAMPLES < sampled && sampled <= MAX_UNTANGLE_SAMPLES + 65_536,
        "{sampled} samples taken under a ceiling of {MAX_UNTANGLE_SAMPLES}"
    );
}

#[test]
fn test_both_seeds_of_the_untangling_timeout_are_projected_in_full() {
    // The two loader seeds this ceiling was made for (#346), each a line
    // link beside a connector: checked in full, not cut short.
    let dir = crate::util::doc_fixtures::repo_path("fuzz/seeds/load_structured");
    for seed in ["line_link_beside_connector_1", "line_link_beside_connector_2"] {
        let bytes = std::fs::read(dir.join(seed)).unwrap();
        let (projected0, _, cut0) = projection_counts();
        crate::document::tests::format_fuzz::check_structured(&bytes).unwrap();
        let (projected, _, cut) = projection_counts();
        assert_eq!(
            (projected - projected0, cut - cut0),
            (1, 0),
            "{seed}: projected, not cut"
        );
    }
}
