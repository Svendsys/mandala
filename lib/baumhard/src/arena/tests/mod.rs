// SPDX-License-Identifier: MPL-2.0

//! The arena's tests: its structural operations held against a
//! reference model under the generational-id rules
//! ([`model_tests`](crate::arena::tests::model_tests)), its operations
//! and walks on a chain 100,000 deep
//! ([`deep_tests`](crate::arena::tests::deep_tests)), every operation
//! in an arena with room for it, which allocates nothing
//! ([`in_place_tests`](crate::arena::tests::in_place_tests)), and its
//! sizes on both targets
//! ([`layout_tests`](crate::arena::tests::layout_tests)).

/// The arena's walks and structural operations on a 100,000-deep chain.
pub mod deep_tests;
/// Every operation and walk, in an arena that has room for what they
/// create.
pub mod in_place_tests;
/// The arena's id, link and slot sizes, pinned per target.
pub mod layout_tests;
/// The arena against a reference model of trees, op by op.
pub mod model_tests;
