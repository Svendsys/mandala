// SPDX-License-Identifier: MPL-2.0

//! TextFlow tests: shaping and its span mapping, outlines and the
//! arc-length walk, placement geometry, and the fit modes' end
//! conditions. Geometric assertions only — positions, distances and
//! counts, never pixels (TEST_CONVENTIONS §T10).

pub mod fit_tests;
pub mod outline_tests;
pub mod place_tests;
pub mod readable_tests;
pub mod shape_tests;
/// Fixtures: resolved styles, real and synthetic runs.
pub(crate) mod support;
