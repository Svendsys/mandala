// SPDX-License-Identifier: MPL-2.0

//! Leaf utilities shared across baumhard: small-scale geometry,
//! grapheme-aware string ops, colour math, prime sieve, hashable
//! vectors, and arena-tree helpers. Nothing here depends on the
//! renderer, the GPU, or the mindmap model.

/// Arena-wide subtree copy helpers built on `indextree`.
pub mod arena_utils;
/// Core `Color` type, arithmetic, and compile-time colour-literal
/// macros.
pub mod color;
/// Hex ↔ RGB ↔ HSV plus theme-variable resolution.
pub mod color_conversion;
/// Small-scale 2D geometry: pivot rotation, epsilon compare,
/// pixel-space ordering.
pub mod geometry;
/// Grapheme-cluster aware text primitives — reach for these from
/// the app crate rather than byte-indexing a `String` (§B3).
pub mod grapheme_chad;
/// Logger initialisation — `init()` selects the right backend per
/// target. Macro callsites keep using `log::warn!` etc. directly,
/// since `log` is the universal Rust facade.
pub mod log;
/// Hashable, `Eq`-able 2D float vector (each axis wrapped in
/// `OrderedFloat`).
pub mod ordered_vec2;
/// Reference palettes — internal seeds and example constants.
pub mod palettes;
/// Lazy Sieve of Eratosthenes — the prime table the region-params
/// grid chooser consults to avoid prime dimension factors.
pub mod primes;
/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8).
pub mod tests;
