// SPDX-License-Identifier: MPL-2.0

//! Ways to write a map as text, for the format's tests: a small valid
//! map around whatever a test wants to put in it.

use crate::document::format::{load, LoadError};
use crate::document::ids::Document;

/// The theme every hand-written map here uses: one variant, two
/// variables.
pub const THEME: &str = r##""theme":{"active":"d","variants":{"d":{"bg":"#000000","fg":"#ffffff"}}}"##;

/// A map whose top level is `THEME` and `extra` (a run of `,"key":value`
/// pairs) and whose `nodes` are `nodes`.
pub fn map_with(extra: &str, nodes: &str) -> String {
    format!(r#"{{"format":1,"name":"t",{THEME}{extra},"nodes":[{nodes}]}}"#)
}

/// A map holding one top-level node with id 1 at the origin, to which
/// `body` (a run of `,"key":value` pairs) is added.
pub fn map_of(body: &str) -> String {
    map_with("", &node_of(1, body))
}

/// A map holding a top-level node with id 1, to which `body` is added,
/// and its child, node 2, to which `child` is added: the flat list's
/// way of writing a node under another.
pub fn map_of_two(body: &str, child: &str) -> String {
    map_with(
        "",
        &format!(
            "{},{}",
            node_of(1, body),
            node_of(2, &format!(r#","parent":1{child}"#))
        ),
    )
}

/// A node with `id` at the origin, to which `body` (a run of
/// `,"key":value` pairs) is added.
pub fn node_of(id: u32, body: &str) -> String {
    format!(r#"{{"id":{id},"position":[0,0]{body}}}"#)
}

/// A map with one node, loaded, or its error.
pub fn load_node(body: &str) -> Result<Document, LoadError> {
    load(&map_of(body))
}

/// The error a load of `json` gives; the load must fail.
pub fn rejection(json: &str) -> LoadError {
    match load(json) {
        Ok(_) => panic!("the map loaded, and it should not have:\n{json}"),
        Err(error) => error,
    }
}
