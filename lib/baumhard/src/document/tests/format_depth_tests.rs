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

use crate::document::element::DocElement;
use crate::document::format::{check_savable, load, save};
use crate::document::ids::NodeIdent;
use crate::document::tests::format_depth::{
    deep_chain_survives_every_walk, planted_recursive_count, CHAIN_DEPTH,
};
use crate::engine::tree::ancestor_steps;

/// Set in the child process of the control: run a recursive walk over
/// the chain instead.
const PLANT: &str = "MANDALA_PLANT_RECURSIVE_WALK";

/// libtest's default stack for a test thread. The test makes its own
/// thread of this size, so that a `RUST_MIN_STACK` in the environment
/// cannot make it pass on a larger one.
const DEFAULT_TEST_STACK: usize = 2 * 1024 * 1024;

#[test]
fn test_a_chain_100000_deep_survives_every_walk_on_a_default_stack() {
    let planted = std::env::var_os(PLANT).is_some();
    let run = std::thread::Builder::new()
        .stack_size(DEFAULT_TEST_STACK)
        .spawn(move || {
            if planted {
                let count = planted_recursive_count(CHAIN_DEPTH);
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

/// How deep the linked chain is: deep enough that one membership walk
/// per link would count millions of ancestor steps.
const LINKED_DEPTH: u32 = 2_000;

/// A chain `depth` nodes deep in which every node holds one line link:
/// node 1's names the leaf, every other node's names node 1, so every
/// link but the first is resolved to a node above it.
fn linked_chain_text(depth: u32) -> String {
    let mut text = String::from("{\n  \"format\": 1,\n  \"name\": \"Linked\",\n  \"nodes\": [\n");
    for id in 1..=depth {
        if id > 1 {
            text.push_str(",\n");
        }
        text.push_str(&format!("    {{\n      \"id\": {id},\n"));
        if id > 1 {
            text.push_str(&format!("      \"parent\": {},\n", id - 1));
        }
        let to = if id == 1 { depth } else { 1 };
        text.push_str(&format!(
            "      \"position\": [0.0, 0.0],\n      \"links\": [{{\"to\": {to}, \"kind\": \"line\"}}]\n    }}"
        ));
    }
    text.push_str("\n  ]\n}\n");
    text
}

/// R17.1: a link's `to` is written without walking its node's
/// ancestors, so a link costs the same at any depth and a deep map full
/// of links loads, checks and saves in time linear in its size. Counted
/// in ancestor steps rather than timed: [`ancestor_steps`] counts every
/// step [`Tree::contains`](crate::engine::tree::Tree::contains) takes,
/// and a membership check per link would take one per level above it.
#[test]
fn test_a_link_at_every_depth_resolves_without_walking_its_ancestors() {
    let text = linked_chain_text(LINKED_DEPTH);
    ancestor_steps::take();
    let doc = load(&text).expect("the linked chain loads");
    assert_eq!(ancestor_steps::take(), 0, "load walked a node's ancestors");
    check_savable(&doc).expect("the linked chain is savable");
    assert_eq!(
        ancestor_steps::take(),
        0,
        "check_savable walked a node's ancestors"
    );
    let saved = save(&doc).expect("the linked chain saves");
    assert_eq!(ancestor_steps::take(), 0, "save walked a node's ancestors");
    assert_eq!(saved, text, "the linked chain saves as it was read");

    // Every link was resolved, to the node it names: node 1's to the
    // leaf, every other to node 1.
    let first = doc.node_by_ident(NodeIdent(1)).expect("node 1");
    let leaf = doc.node_by_ident(NodeIdent(LINKED_DEPTH)).expect("the leaf");
    let mut links = 0;
    for id in doc.tree().descendants() {
        if let Some(DocElement::Link(link)) = doc.get(id) {
            let owner = doc.owning_node(id).expect("a link's node");
            let expected = if owner == first { leaf } else { first };
            assert_eq!(link.to, expected, "a link resolved to the wrong node");
            links += 1;
        }
    }
    assert_eq!(links, LINKED_DEPTH as usize);
}
