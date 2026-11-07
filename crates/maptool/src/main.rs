// SPDX-License-Identifier: MPL-2.0

// CODE_CONVENTIONS §9 closes with "Bare `unwrap()` outside tests is
// a bug", and this is the half of that rule an editor can tell you
// about while you type. `util::unwrap_posture` is the other half —
// it reads the workspace's source text and fails every `./test.sh`
// run; the lint fails only `./test.sh --lint`, a hard gate since
// clippy's baseline reached zero, but one you have to ask for. Two
// mechanisms rather than one because they disagree usefully: the lint sees
// post-expansion code the text scan cannot read, and the scan sees
// the `pub mod tests;` trees the lint has to be told about.
//
// The `cfg_attr` is what keeps the lint off test code. A
// `#[cfg(test)] mod` does not exist in the build where the lint is
// live, and in the build where it does exist the whole crate is
// allowed — so `unwrap()` stays the right spelling in a test and a
// bug everywhere else.
#![warn(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

//! `maptool` — the command-line tool for `.mandala.json` files.
//!
//! Its verbs are rebuilt as
//! clients of `baumhard::document::ids::Document` by V1-E1 (#253):
//! `show`, `grep`, `export` and `verify` read through it, and `apply`
//! writes through `Document::apply`. Until then every invocation says
//! so and exits `2`, the status a command line that cannot be run is
//! answered with.

use std::process::ExitCode;

/// What every invocation prints, on stderr.
const REBUILT: &str = "maptool: rebuilt in V1-E1 (#253): its verbs return as clients of the \
.mandala.json document";

fn main() -> ExitCode {
    baumhard::util::log::init();
    eprintln!("{REBUILT}");
    ExitCode::from(2)
}
