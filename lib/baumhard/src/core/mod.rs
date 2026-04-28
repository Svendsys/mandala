// SPDX-License-Identifier: MPL-2.0

//! Core data types and animation primitives shared across baumhard.

/// Animation timeline + mutator traits.
pub mod animation;
/// Ranges, styled regions, `ApplyOperation`, anchors, flags, and the
/// `Applicable` trait.
pub mod primitives;
/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8).
pub mod tests;
