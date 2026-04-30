// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::mutator_builder`]. The inner module is
//! gated `#[cfg(test)]` per §T2.1 — it carries only `#[test]`
//! functions, no `do_*()` bodies, and is unreferenced by
//! `benches/test_bench.rs`. The enclosing `pub mod tests;`
//! shape stays consistent with the rest of `lib/baumhard/src/`
//! so a future bench-reusable companion can land alongside without
//! a structural reshuffle.

#[cfg(test)]
pub mod mutator_builder_tests;
