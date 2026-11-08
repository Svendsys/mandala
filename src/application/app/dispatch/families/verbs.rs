// SPDX-License-Identifier: MPL-2.0

//! Map verbs and their triggers (design §11) — a node's `on.click` and
//! `on.hover`, a span's link — run by V1-D3 (#252). A click selects as it
//! always does; running the verb the clicked node names waits for D3.

use baumhard::arena::NodeId;
use baumhard::document::ids::Document;

/// The issue that runs map verbs.
pub(in crate::application::app) const OWNER: &str = "V1-D3 (#252)";

/// A click landed on `node`: the verb its `on.click` names runs once
/// V1-D3 lands. Until then a node that names one says so at `debug!`.
pub(in crate::application::app) fn node_clicked(doc: &Document, node: NodeId) {
    if let Some(verb) = doc.get(node).and_then(|e| e.as_node()).and_then(|n| n.on.click) {
        log::debug!("dispatch: the click verb {verb:?} is run by {OWNER}; not run until then");
    }
}
