// SPDX-License-Identifier: MPL-2.0

//! Engine tests, run against [`test_element::TestElement`](crate::engine::tests::test_element::TestElement) — an
//! instantiation that shares nothing with the glyph trees — so a
//! passing walker test here is evidence the engine is generic, not
//! merely that it still works for glyphs. The glyph instantiation's
//! own walker, mutator, predicate and builder tests live in
//! `gfx_structs::tests`.

pub mod anchor_tests;
pub mod builder_tests;
pub mod inherited_tests;
pub mod interpolate_tests;
pub mod journal_fixtures;
pub mod journal_tests;
pub mod routing_tests;
pub mod sorted_tests;
pub mod structure_tests;
pub mod test_element;
pub mod walker_tests;
