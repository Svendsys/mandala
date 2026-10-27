// SPDX-License-Identifier: MPL-2.0

//! How far a map's text expands into memory while it loads, measured,
//! and wasm32's [`MAX_MAP_BYTES`] held to it (design §C.8).
//!
//! **Why.** On wasm32 the address space is 4 GiB, and a failed
//! allocation traps and kills the page. `MAX_MAP_BYTES` is the only
//! thing that refuses a map the browser cannot hold before it tries, so
//! it must leave room for what the text becomes while it loads: the
//! text itself, the tree, the index of node extents, the ident maps,
//! the chain. This test loads the densest maps the format allows —
//! nodes with nothing but an id, a parent and a position, which each
//! become a node and a connector, and nodes with one short label — under
//! a counting allocator, takes the peak heap of each load, and asserts
//! that the peak per byte of text is at most the ratio the ceiling is
//! derived from. A later change that makes an element bigger fails here
//! instead of silently letting the browser trap.
//!
//! **Native bounds wasm32.** Pointers and `usize` are twice as wide
//! here as on wasm32, so the ratio measured natively is at least what
//! wasm32 pays.
//!
//! **One process per map.** The allocator's peak cannot be reset, so
//! the test runs itself again for each map, and each child reports one
//! peak.

use baumhard::document::format::{
    load, MAX_MAP_BYTES, WASM32_EXPANSION, WASM32_LOAD_BUDGET, WASM32_MAX_MAP_BYTES,
};
use cap::Cap;
use std::alloc::System;

#[global_allocator]
static ALLOCATOR: Cap<System> = Cap::new(System, usize::MAX);

/// Set in a child: the map it measures.
const SHAPE: &str = "MANDALA_FORMAT_MEMORY_SHAPE";

/// How many nodes each measured map holds: enough that the map's own
/// cost dwarfs the process's baseline.
const NODES: u32 = 200_000;

/// The text of a map of `shape`, written as densely as the format
/// allows (compact, no whitespace), built into one allocation of the
/// size it needs so that building it costs no peak of its own.
fn text_of(shape: &str) -> String {
    let node = |id: u32| match shape {
        // Every node under the first: a node and a connector each.
        "fan" => match id {
            1 => r#"{"id":1,"position":[0,0]}"#.to_string(),
            _ => format!(r#"{{"id":{id},"parent":1,"position":[0,0]}}"#),
        },
        // Each node under the one before: the chain is as long as the map.
        "chain" => match id {
            1 => r#"{"id":1,"position":[0,0]}"#.to_string(),
            _ => format!(r#"{{"id":{id},"parent":{},"position":[0,0]}}"#, id - 1),
        },
        // A short label on each: a node, a connector, a section and a span.
        _ => match id {
            1 => r#"{"id":1,"position":[0,0]}"#.to_string(),
            _ => format!(
                r#"{{"id":{id},"parent":1,"position":[0,0],"sections":[{{"spans":[{{"text":"label"}}]}}]}}"#
            ),
        },
    };
    let size: usize = 64 + (1..=NODES).map(|id| node(id).len() + 1).sum::<usize>();
    let mut text = String::with_capacity(size);
    text.push_str(r#"{"format":1,"name":"m","nodes":["#);
    for id in 1..=NODES {
        if id > 1 {
            text.push(',');
        }
        text.push_str(&node(id));
    }
    text.push_str("]}");
    text
}

/// In a child: load the map, and report the peak heap over its text.
fn measure(shape: &str) {
    let text = text_of(shape);
    let doc = load(&text).expect("the measured map loads");
    let peak = ALLOCATOR.max_allocated();
    drop(doc);
    println!("PEAK {shape} {peak} {}", text.len());
}

#[test]
fn test_wasm32_max_map_bytes_leaves_room_for_what_the_text_becomes() {
    if let Ok(shape) = std::env::var(SHAPE) {
        return measure(&shape);
    }
    // The ceiling is the budget over the text and what it expands into.
    assert_eq!(WASM32_MAX_MAP_BYTES, WASM32_LOAD_BUDGET / (1 + WASM32_EXPANSION));
    #[cfg(target_arch = "wasm32")]
    assert_eq!(MAX_MAP_BYTES, Some(WASM32_MAX_MAP_BYTES));
    #[cfg(not(target_arch = "wasm32"))]
    assert_eq!(MAX_MAP_BYTES, None);

    let exe = std::env::current_exe().expect("the test binary");
    let mut worst = 0.0f64;
    for shape in ["fan", "chain", "labels"] {
        let out = std::process::Command::new(&exe)
            .args([
                "--exact",
                "test_wasm32_max_map_bytes_leaves_room_for_what_the_text_becomes",
                "--nocapture",
            ])
            .env(SHAPE, shape)
            .output()
            .expect("the test binary runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{shape}: {stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let line = stdout
            .lines()
            .find(|line| line.starts_with(&format!("PEAK {shape} ")))
            .unwrap_or_else(|| panic!("{shape} reported no peak:\n{stdout}"));
        let numbers: Vec<f64> = line.split(' ').skip(2).map(|n| n.parse().unwrap()).collect();
        let (peak, text) = (numbers[0], numbers[1]);
        // What the text became, per byte of it, beside the text itself.
        let expansion = (peak - text) / text;
        println!("{shape}: {text} bytes of text, a peak of {peak} bytes: {expansion:.2} bytes per byte beside the text");
        assert!(text > 5.0e6, "precondition: {shape} is {text} bytes");
        worst = worst.max(expansion);
    }
    assert!(
        worst <= WASM32_EXPANSION as f64,
        "a map's text now expands to {worst:.2} bytes per byte beside itself, past the {} \
         WASM32_MAX_MAP_BYTES is derived from: a browser loading a map under the ceiling could \
         trap. Re-derive the ratio, or find what grew.",
        WASM32_EXPANSION
    );
    // And the ratio is not stale the other way by much: it is the
    // measured one, rounded up to a whole number.
    assert!(
        worst + 1.0 > WASM32_EXPANSION as f64,
        "the measured expansion {worst:.2} is far below the pinned {}; pin it again",
        WASM32_EXPANSION
    );
}
