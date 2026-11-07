// SPDX-License-Identifier: MPL-2.0

//! The projection's tests: the oracle, the dependency kinds, reshaping
//! against re-placing, hit testing, depth, and the bench bodies.

/// The oracle's corpus: generated documents and edits.
pub mod corpus;
/// The oracle's sweeps over the corpus.
#[cfg(test)]
mod oracle_tests;
/// The projected document and the oracle's check.
pub mod support;
/// One test per kind of dependency.
#[cfg(test)]
mod dependency_tests;
/// Hit testing.
#[cfg(test)]
mod hit_tests;
/// A chain as deep as memory allows.
pub mod deep_tests;
/// The binding carried down, counted.
#[cfg(test)]
mod steps_tests;
/// The oracle's comparison, and its controls.
#[cfg(test)]
mod equivalent_tests;
