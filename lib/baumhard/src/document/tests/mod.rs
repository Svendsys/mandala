// SPDX-License-Identifier: MPL-2.0

//! Document tests: the shape rules, identity and the read API, style
//! resolution and its read records, the predicate surface, the bounds,
//! the engine instantiation, and the mutation vocabulary. Each builds its documents through
//! `test_support::DocumentBuilder`,
//! mostly from `test_support::example`.

pub mod bounds_tests;
pub mod deps_tests;
pub mod element_tests;
/// R13's deep chain, shared with the wasm32 probe.
pub mod format_depth;
#[cfg(test)]
mod format_depth_tests;
#[cfg(test)]
mod format_error_tests;
#[cfg(test)]
mod format_flat_tests;
#[cfg(test)]
mod format_order_tests;
#[cfg(test)]
mod format_read_tests;
#[cfg(test)]
mod format_reject_tests;
#[cfg(test)]
mod format_robust_tests;
#[cfg(test)]
mod format_savable_tests;
#[cfg(test)]
mod format_save_tests;
#[cfg(test)]
mod format_schema_tests;
#[cfg(test)]
mod format_strict_tests;
/// What the format's tests share: the fixture corpus and ways to read
/// a loaded document.
pub(crate) mod format_support;
#[cfg(test)]
mod format_sweep_tests;
pub mod format_tests;
#[cfg(test)]
mod format_text;
#[cfg(test)]
mod format_writer_tests;
pub mod ids_tests;
pub mod inspect_tests;
pub mod journal_tests;
pub mod mutation_compose_tests;
pub mod mutation_inherited_tests;
pub mod mutation_ops_tests;
pub mod mutation_validate_tests;
#[cfg(test)]
mod place_tests;
pub mod resolve_tests;
pub mod shape_tests;
pub mod structure_tests;
/// Crate-private: the tests and their bench bodies build documents with
/// it, and nothing else may — a document is built by the loader and
/// changed by its writer (rule 4). `util::document_surface` holds every
/// other file of the workspace to not naming it.
pub(crate) mod test_support;
