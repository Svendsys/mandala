// SPDX-License-Identifier: MPL-2.0

//! Field cleanup pass: drop the obsolete `index` (now encoded in the
//! Dewey-decimal ID) and default `channel` to `0` on every node.

use serde_json::Value;

pub fn cleanup_nodes(root: &mut Value) {
    let Some(nodes) = super::nodes_obj_mut(root) else { return };
    for node in nodes.values_mut() {
        let obj = match node.as_object_mut() {
            Some(o) => o,
            None => continue,
        };
        obj.remove("index");
        obj.entry("channel").or_insert(Value::Number(0.into()));
    }
}
