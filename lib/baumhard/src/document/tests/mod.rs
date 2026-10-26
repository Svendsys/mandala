// SPDX-License-Identifier: MPL-2.0

//! Document tests: the shape rules, identity and the read API, style
//! resolution and its read records, the predicate surface, the bounds,
//! and the engine instantiation. Each builds its documents through
//! [`DocumentBuilder`](crate::document::tests::test_support::DocumentBuilder),
//! mostly from [`example`](crate::document::tests::test_support::example).

pub mod bounds_tests;
pub mod element_tests;
pub mod ids_tests;
pub mod inspect_tests;
pub mod resolve_tests;
pub mod shape_tests;
pub mod test_support;
