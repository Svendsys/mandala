// SPDX-License-Identifier: MPL-2.0

//! The projection's tests: the oracle, the dependency kinds, reshaping
//! against re-placing, hit testing, depth, and the bench bodies.

/// Connector geometry: sides, curves, fans and a junction's paths held
/// clear of each other.
#[cfg(test)]
mod connector_tests;
pub mod corpus;
/// A chain as deep as memory allows.
pub mod deep_tests;
/// One test per kind of dependency.
#[cfg(test)]
mod dependency_tests;
/// The oracle's comparison, and its controls.
#[cfg(test)]
mod equivalent_tests;
/// Nodes hide the paths behind them (#332).
#[cfg(test)]
mod hide_tests;
/// Hit testing.
#[cfg(test)]
mod hit_tests;
/// Maps written as `.mandala.json`, and their elements found again.
pub mod maps;
/// The work bound's count of the shaper's `.notdef` passes (#387), on the
/// compiled faces alone.
#[cfg(test)]
mod notdef_bound_tests;
/// The oracle's sweeps over the corpus.
#[cfg(test)]
mod oracle_tests;
/// A glyph-layer presentation's body as run mutations, spelled op by
/// op.
#[cfg(test)]
mod presentation_glyph_tests;
/// The projection's fuzz properties: a loaded map projects, and
/// edit sequences keep it equal to a full projection.
pub mod projection_fuzz;
/// The fuzz properties' own tests.
#[cfg(test)]
mod projection_fuzz_tests;
/// The bench bodies: a full projection, an update, and a rubber band's
/// query.
pub mod projection_tests;
/// The shaping cache: one shaping per key (#304, ruling B7.4).
#[cfg(test)]
mod shape_cache_tests;
/// A long span's shaping work, counted (#304).
#[cfg(test)]
mod shaping_work_tests;
/// The binding carried down, counted.
#[cfg(test)]
mod steps_tests;
/// The projected document and the oracle's check.
pub mod support;
