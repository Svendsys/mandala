// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::mutator_builder`]. `pub mod` (not
//! `#[cfg(test)] mod`) per §T2.2 / §B8 — the structural shape
//! stays open so a future bench-reusable `do_*()` companion can
//! land alongside the existing `#[test]` bodies without a parent
//! reshuffle. Today none exist; the inner file gates its imports
//! with `#[cfg(test)]` so non-test builds don't see them as
//! unused.

pub mod mutator_builder_tests;
