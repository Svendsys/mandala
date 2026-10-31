// SPDX-License-Identifier: MPL-2.0

pub mod arena_utils_tests;
/// The choice source of the structure-aware fuzz targets
/// (TEST_CONVENTIONS §T14).
pub mod choices;
#[cfg(test)]
mod choices_tests;
pub mod color_tests;
/// The fuzz targets and their seeds, held to the properties on stable.
/// Reads `fuzz/` off the filesystem, which wasm32 does not have.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod fuzz_seed_tests;
pub mod geometry_tests;
pub mod grapheme_chad_tests;
pub mod ordered_vec2_tests;
pub mod primes_test;
pub mod shortest_float_tests;
// Follows `util::rust_source`'s own gate: it reads files off the
// filesystem, which wasm32 does not have.
#[cfg(not(target_arch = "wasm32"))]
pub mod rust_source_tests;
