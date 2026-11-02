// SPDX-License-Identifier: MPL-2.0

//! The arena's tests: its structural operations held against a
//! reference model under the generational-id rules
//! ([`model_tests`]), and its operations and walks on a chain 100,000
//! deep ([`deep_tests`]).

/// The arena's walks and structural operations on a 100,000-deep chain.
pub mod deep_tests;
/// The arena against a reference model of trees, op by op.
pub mod model_tests;
