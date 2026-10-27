// SPDX-License-Identifier: MPL-2.0

//! R13's acceptance on native: a chain 100,000 nodes deep, with the
//! deepest content a node holds at its leaf, through `load`,
//! `validate_subtree`, `check_savable` and `save`, and back, on a thread
//! with libtest's default stack. The same body runs on wasm32 in the
//! probe (`lib/baumhard/tests/wasm_font_lock.rs`).
//!
//! **The negative control is a process.** A walk that recursed on depth
//! would not fail an assertion: it would exhaust the stack and abort
//! the whole test binary. So the control re-runs this binary on the one
//! test, with a recursive walk planted in place of the iterative ones,
//! and asserts that the child died of a stack overflow. That proves the
//! chain is deep enough, and the thread small enough, for a recursion
//! to be caught.

use crate::document::format::load;
use crate::document::ids::Document;
use crate::document::tests::format_depth::{chain_text, deep_chain_survives_every_walk, CHAIN_DEPTH};
use indextree::NodeId;

/// Set in the child process of the control: run a recursive walk over
/// the chain instead.
const PLANT: &str = "MANDALA_PLANT_RECURSIVE_WALK";

/// libtest's default stack for a test thread. The test makes its own
/// thread of this size, so that a `RUST_MIN_STACK` in the environment
/// cannot make it pass on a larger one.
const DEFAULT_TEST_STACK: usize = 2 * 1024 * 1024;

/// How many elements are at or under `id`, counted by recursing on the
/// depth of the tree: the walk the format must never make.
fn recursive_count(doc: &Document, id: NodeId) -> usize {
    1 + doc
        .children(id)
        .map(|child| recursive_count(doc, child))
        .sum::<usize>()
}

#[test]
fn test_a_chain_100000_deep_survives_every_walk_on_a_default_stack() {
    let planted = std::env::var_os(PLANT).is_some();
    let run = std::thread::Builder::new()
        .stack_size(DEFAULT_TEST_STACK)
        .spawn(move || {
            if planted {
                let doc = load(&chain_text(CHAIN_DEPTH)).expect("the chain loads");
                let count = recursive_count(&doc, doc.root());
                return Err(format!(
                    "the planted recursive walk counted {count} elements and did not overflow"
                ));
            }
            deep_chain_survives_every_walk(CHAIN_DEPTH)
        })
        .expect("a thread with the default stack");
    let outcome = run.join().expect("the walk thread finished");
    assert_eq!(outcome, Ok(()));
}

#[test]
fn test_the_depth_test_goes_red_on_a_walk_that_recurses() {
    if std::env::var_os(PLANT).is_some() {
        return;
    }
    let exe = std::env::current_exe().expect("the test binary's path");
    let child = std::process::Command::new(exe)
        .args([
            "--exact",
            "document::tests::format_depth_tests::test_a_chain_100000_deep_survives_every_walk_on_a_default_stack",
            "--test-threads=1",
            "--nocapture",
        ])
        .env(PLANT, "1")
        .output()
        .expect("the test binary runs");
    let stderr = String::from_utf8_lossy(&child.stderr);
    assert!(!child.status.success(), "the planted recursion passed:\n{stderr}");
    assert!(
        stderr.contains("has overflowed its stack"),
        "the planted recursion failed, but not by exhausting the stack:\n{stderr}"
    );
}
