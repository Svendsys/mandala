// SPDX-License-Identifier: MPL-2.0

//! TextFlow tests: shaping and its span mapping, outlines and the
//! arc-length walk, placement geometry, and the fit modes' end
//! conditions. Geometric assertions only — positions, distances and
//! counts, never pixels (TEST_CONVENTIONS §T10).

/// A frame's work budget: past it, and `untouched`'s share of it.
#[cfg(test)]
mod budget_tests;
pub mod fit_tests;
/// TextFlow's fuzz properties, which the `fuzz/` targets call
/// (TEST_CONVENTIONS §T14).
pub mod flow_fuzz;
#[cfg(test)]
mod flow_fuzz_tests;
pub mod halves_tests;
pub mod lines_tests;
pub mod outline_tests;
pub mod pitch_tests;
pub mod place_tests;
pub mod readable_tests;
/// The exact work removals of #309, each pinned by its counter.
#[cfg(test)]
mod removal_tests;
/// A `Repeat` ring's work budget: past it, and what it holds back.
#[cfg(test)]
mod ring_budget_tests;
pub mod shape_tests;
/// Fixtures: resolved styles, real and synthetic runs.
pub(crate) mod support;
/// A frame's work counted by kind of outline (#309).
#[cfg(test)]
mod work_tests;
