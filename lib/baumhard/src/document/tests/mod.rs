// SPDX-License-Identifier: MPL-2.0

//! Document tests: the shape rules, identity and the read API, style
//! resolution and its read records, the predicate surface, the bounds,
//! and the engine instantiation. Each builds its documents through
//! `test_support::DocumentBuilder`,
//! mostly from `test_support::example`.

pub mod bounds_tests;
pub mod deps_tests;
pub mod element_tests;
pub mod ids_tests;
pub mod inspect_tests;
pub mod journal_tests;
pub mod resolve_tests;
pub mod shape_tests;
pub mod structure_tests;
/// Crate-private: the tests and their bench bodies build documents with
/// it, and nothing else may — a document is built by the loader and
/// changed by its writer (rule 4). `util::document_surface` holds every
/// other file of the workspace to not naming it.
pub(crate) mod test_support;
