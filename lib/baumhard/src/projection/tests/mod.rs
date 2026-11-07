// SPDX-License-Identifier: MPL-2.0

//! The projection's tests: the oracle, the dependency kinds, reshaping
//! against re-placing, hit testing, depth, and the bench bodies.

/// The oracle's corpus: generated documents and edits.
pub mod corpus;
/// A chain as deep as memory allows.
pub mod deep_tests;
/// One test per kind of dependency.
#[cfg(test)]
mod dependency_tests;
/// The oracle's comparison, and its controls.
#[cfg(test)]
mod equivalent_tests;
/// Hit testing.
#[cfg(test)]
mod hit_tests;
/// Maps written as `.mandala.json`, and their elements found again.
pub mod maps;
/// The oracle's sweeps over the corpus.
#[cfg(test)]
mod oracle_tests;
/// The projection's fuzz properties: a loaded map projects, and
/// edit sequences keep it equal to a full projection.
pub mod projection_fuzz;
/// The fuzz properties' own tests.
#[cfg(test)]
mod projection_fuzz_tests;
/// The bench bodies: a full projection, and an update.
pub mod projection_tests;
/// The binding carried down, counted.
#[cfg(test)]
mod steps_tests;
/// The projected document and the oracle's check.
pub mod support;
