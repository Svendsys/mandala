// SPDX-License-Identifier: MPL-2.0

//! The workspace's feature unifier (#284): a crate with no code, whose
//! dependencies are the point. See `crates/workspace-hack/Cargo.toml`
//! and `.config/hakari.toml`.

// Every crate root carries it (CODE_CONVENTIONS §9), this one included,
// so a line added here is held to the rule from its first commit.
#![warn(clippy::unwrap_used)]
