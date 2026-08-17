// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::font::metric_cache`] — the per-`(face,
//! font_size_pt, grapheme)` measured-metric caches behind the
//! border-rail layout math: [`glyph_advance`], [`glyph_ink`],
//! [`cluster_width`], and the `*_with` variants for callers already
//! holding the `FONT_SYSTEM` write guard (§B5).
//!
//! Follows the `do_*()` / `test_*()` split from §T2.2: every `do_*`
//! body is benchmarkable from `benches/test_bench.rs`. The
//! re-entrancy regression at the bottom is a plain `#[test]` on
//! purpose: its body drives a deliberate lock timeout with the
//! `FONT_SYSTEM` write guard held and catches the resulting panic
//! under `catch_unwind`, so an iteration measures the 200 ms
//! timeout budget and nothing else — §B8 opt-out class 2.
//!
//! Grapheme × size keys are chosen per test so that each body's
//! first call is a genuine cache miss under `cargo test` — no other
//! test in the crate warms the same key.

use crate::font::fonts;
use crate::font::fonts::acquire_font_system_write;
use crate::font::metric_cache::{
    cluster_width, glyph_advance, glyph_advance_with, glyph_ink, glyph_ink_with,
};

#[test]
fn test_glyph_advance_cache_hit_matches_miss() {
    do_glyph_advance_cache_hit_matches_miss();
}

/// Cache hit returns the same value as a fresh shape call. Tests
/// the cache mechanism, not any specific advance value (which is
/// font-version-dependent).
pub fn do_glyph_advance_cache_hit_matches_miss() {
    fonts::init();
    let first = glyph_advance(None, 18.0, "│");
    let second = glyph_advance(None, 18.0, "│");
    assert_eq!(first, second);
    assert!(first > 0.0, "│ should have positive advance, got {first}");
}

#[test]
fn test_glyph_advance_distinct_per_grapheme() {
    do_glyph_advance_distinct_per_grapheme();
}

/// Different grapheme clusters get separately measured advances:
/// the grapheme is part of the cache key, so `│` and `│││` are two
/// keys and not one. The input that fails this is a key that drops
/// its grapheme component — both calls then resolve to whichever
/// `(face, 22.5)` entry landed first and the two widths come back
/// identical.
///
/// Measured as one glyph against three of the same glyph shaped as
/// a single cluster rather than as two unrelated characters,
/// because the relation then needs no knowledge of the face:
/// whatever `│` advances to, `│││` advances to three of it, absent
/// a ligature over the run or pair kerning between the copies. The
/// vendored font applies neither — the measured ratio is exactly
/// `3`, and the assertion's 2.5–3.5 band leaves about 16% of
/// headroom on each side. Two *different*
/// characters would need the test to know the shipped font's
/// metrics — `│` and `+` are the same width in a monospace fallback
/// — which is why the body this replaced asserted only that both
/// were positive and so passed with the grapheme dropped from both
/// cache keys.
pub fn do_glyph_advance_distinct_per_grapheme() {
    fonts::init();
    // (22.5, "│"/"│││") is warmed by no other test, so both calls
    // shape rather than reading a neighbor's entry.
    let single = glyph_advance(None, 22.5, "│");
    let tripled = glyph_advance(None, 22.5, "│││");
    assert!(single > 0.0, "│ should have positive advance, got {single}");
    assert!(
        tripled > single,
        "│││ ({tripled}) must advance further than │ ({single}); equal means \
         one cache entry answered both calls"
    );
    let ratio = tripled / single;
    assert!(
        (2.5..=3.5).contains(&ratio),
        "│││ / │ advance ratio should be near 3.0; got {ratio}"
    );
}

#[test]
fn test_glyph_ink_distinct_per_grapheme() {
    do_glyph_ink_distinct_per_grapheme();
}

/// The ink cache keys on the grapheme too, and it is a *separate*
/// `FxHashMap` from the advance cache — so the sibling assertion
/// above says nothing about this one. Same construction, same
/// failing input: an `INK_EXTENT_CACHE` key without its grapheme
/// component answers both calls from one entry and the advances
/// come back equal.
///
/// `advance` rather than `ink_height` is the field that separates
/// the two: three copies of one glyph are three times as wide and
/// exactly as tall, so a height comparison would be equal on
/// correct code.
pub fn do_glyph_ink_distinct_per_grapheme() {
    fonts::init();
    // (26.5, "│"/"│││") is cold in INK_EXTENT_CACHE for the same
    // reason (22.5, …) is cold in ADVANCE_CACHE.
    let single = glyph_ink(None, 26.5, "│");
    let tripled = glyph_ink(None, 26.5, "│││");
    assert!(
        single.advance > 0.0,
        "│ should have positive ink advance, got {}",
        single.advance
    );
    assert!(
        tripled.advance > single.advance,
        "│││ ({}) must advance further than │ ({}); equal means one ink-cache \
         entry answered both calls",
        tripled.advance,
        single.advance
    );
    let ratio = tripled.advance / single.advance;
    assert!(
        (2.5..=3.5).contains(&ratio),
        "│││ / │ ink advance ratio should be near 3.0; got {ratio}"
    );
}

#[test]
fn test_cluster_width_sums_per_grapheme() {
    do_cluster_width_sums_per_grapheme();
}

/// Multi-grapheme clusters shape as a unit. `cluster_width` for
/// `["◆", "·"]` should equal `glyph_advance("◆") +
/// glyph_advance("·")` — the sum-of-parts shape is the contract the
/// border rail math relies on.
pub fn do_cluster_width_sums_per_grapheme() {
    fonts::init();
    let graphemes = vec!["◆".to_string(), "·".to_string()];
    let summed = cluster_width(None, 18.0, &graphemes);
    let direct = glyph_advance(None, 18.0, "◆") + glyph_advance(None, 18.0, "·");
    assert!(
        (summed - direct).abs() < 0.01,
        "cluster_width should equal sum of per-grapheme advances; got {summed} vs {direct}",
    );
}

#[test]
fn test_glyph_advance_scales_with_size() {
    do_glyph_advance_scales_with_size();
}

/// Different `size_pt` values produce different advances. Sanity
/// that the cache key includes the size.
pub fn do_glyph_advance_scales_with_size() {
    fonts::init();
    let small = glyph_advance(None, 12.0, "█");
    let big = glyph_advance(None, 24.0, "█");
    // 24pt should be roughly 2× 12pt for the same glyph. Not
    // strictly 2× due to hinting/sub-pixel rounding; tolerance
    // stays wide on purpose.
    assert!(
        big > small,
        "24pt advance ({big}) should exceed 12pt advance ({small})"
    );
    let ratio = big / small;
    assert!(
        (1.5..=3.0).contains(&ratio),
        "24/12 advance ratio should be near 2.0; got {ratio}"
    );
}

#[test]
fn test_glyph_ink_distinct_per_size() {
    do_glyph_ink_distinct_per_size();
}

/// The unlocked [`glyph_ink`] wrapper measures per size: the same
/// grapheme at doubled `size_pt` must report strictly more ink
/// height. The input that fails this is a cache key that drops the
/// size component — both calls then resolve to whichever entry
/// landed first and the two heights come back equal. The claim
/// holds on the no-ink fallback path too, where `ink_height` is
/// `size_pt` by construction.
pub fn do_glyph_ink_distinct_per_size() {
    fonts::init();
    let small = glyph_ink(None, 19.0, "◆");
    let large = glyph_ink(None, 38.0, "◆");
    assert!(
        large.ink_height > small.ink_height,
        "38pt ink height ({}) must exceed 19pt ink height ({})",
        large.ink_height,
        small.ink_height
    );
    assert!(small.ink_height > 0.0, "19pt ◆ ink height should be positive");
}

#[test]
fn test_glyph_advance_with_shapes_cold_key_under_held_guard() {
    do_glyph_advance_with_shapes_cold_key_under_held_guard();
}

/// `glyph_advance_with` shapes a COLD key while the write guard is
/// held — measured FIRST so the call reaches the actual shape path
/// rather than a cache hit — and the unlocked wrapper then agrees
/// on the value the `_with` shape cached. This is exactly the
/// renderer's path: measure under a held guard with no nested
/// acquire.
pub fn do_glyph_advance_with_shapes_cold_key_under_held_guard() {
    fonts::init();
    // (27.0, "┃") is warmed by no other test, so the first call
    // misses the cache and drives the shape path under the guard.
    let with = {
        let mut guard = acquire_font_system_write("metric_cache_tests::glyph_advance_with_cold");
        glyph_advance_with(&mut guard, None, 27.0, "┃")
    };
    // The unlocked wrapper now hits the cache the `_with` shape
    // filled; the values must match.
    let unlocked = glyph_advance(None, 27.0, "┃");
    assert_eq!(
        with, unlocked,
        "glyph_advance_with cold shape must equal the wrapper's cached value"
    );
    assert!(with > 0.0, "┃ should have positive advance, got {with}");
}

#[test]
fn test_glyph_ink_with_cold_key_under_held_guard() {
    do_glyph_ink_with_cold_key_under_held_guard();
}

/// A different (uncached) key measured *only* through the `_with`
/// variant while the guard is held proves cold keys shape without a
/// nested acquire — the exact renderer scenario P0-06 guards
/// against.
pub fn do_glyph_ink_with_cold_key_under_held_guard() {
    fonts::init();
    let mut guard = acquire_font_system_write("metric_cache_tests::glyph_ink_with");
    let ink = glyph_ink_with(&mut guard, None, 23.0, "◆");
    drop(guard);
    assert!(ink.advance > 0.0, "◆ advance should be positive");
    assert!(ink.ink_height > 0.0, "◆ ink_height should be positive");
}

/// Freeze-hardening regression, the metric-cache twin of
/// `fonts::test_acquire_font_system_write_panics_on_timeout`: a
/// cache **miss** taken while the `FONT_SYSTEM` write guard is
/// already held must route through `acquire_font_system_write` and
/// PANIC with the site tag — not hang forever on a re-entrant
/// `RwLock::write()`. This is the exact deadlock the renderer's
/// border-rebuild loop would hit if it called `glyph_advance`
/// (instead of `glyph_advance_with`) on a cold key.
///
/// We hold the guard on the test thread and drive the miss under
/// `catch_unwind`, then drop the guard cleanly: the panic is caught
/// while the guard is still live, so it never unwinds past the
/// guard and `FONT_SYSTEM` is left un-poisoned for sibling tests. A
/// short acquire timeout keeps the test fast — the panic path and
/// message are identical to production's 5 s budget.
///
/// Plain `#[test]` with no `do_*` body: §B8 opt-out class 2, the
/// body drives a panic. Every iteration would wait out the 200 ms
/// acquire budget before the panic it exists to catch, so the fold
/// of wrapper and body is required here rather than forbidden.
#[test]
fn test_glyph_advance_miss_under_held_guard_panics_not_hangs() {
    use crate::font::metric_cache::glyph_advance_with_timeout;
    use std::time::Duration;

    fonts::init();
    // A grapheme + size pair no other test warms, so the call is
    // guaranteed to miss the cache and reach the acquire.
    let cold = "\u{2591}\u{2593}reentry";
    let guard = acquire_font_system_write("metric_cache_reentrancy_test_holder");
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        glyph_advance_with_timeout(None, 41.5, cold, Duration::from_millis(200))
    }));
    drop(guard);
    let payload = outcome.expect_err("re-entrant glyph_advance miss must panic, not hang");
    let msg = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("");
    assert!(
        msg.contains("metric_cache::glyph_advance"),
        "panic must name the metric-cache site; got: {msg:?}"
    );
}
