// SPDX-License-Identifier: MPL-2.0

//! Test modules for [`crate::mindmap::scene_dump`].
//!
//! Declared `pub mod` (not `#[cfg(test)] mod`) per §T2.2 so the
//! criterion bench harness at `lib/baumhard/benches/test_bench.rs`
//! can reach the `do_*()` bodies — the serializer's bench rows.

pub mod scene_dump_tests;
