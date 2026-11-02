// SPDX-License-Identifier: MPL-2.0

// Cargo compiles a package's `[build-dependencies]` only when the
// package has a build script, and the generated manifest's
// build-dependency tables are how the workspace-hack gives the
// proc-macro and build-script graph the target graph's feature sets. So
// this script exists and does nothing.
//
// A crate root of its own, so it carries the lint `lib.rs` does
// (CODE_CONVENTIONS §9).
#![warn(clippy::unwrap_used)]

fn main() {}
