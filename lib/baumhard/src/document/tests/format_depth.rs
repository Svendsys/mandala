// SPDX-License-Identifier: MPL-2.0

//! A map as deep as a map can be, for R13's acceptance: one chain of
//! nodes, each the only child of the one before it, with the deepest
//! content a node holds at the leaf, through every walk the format makes
//! — `load`, `validate_subtree`, `check_savable`, `save` — and back.
//!
//! Public, and not only for the tests here: the wasm32 probe
//! (`lib/baumhard/tests/wasm_font_lock/probe.rs`) runs the same body on
//! wasm32's default stack, in a debug build, inside node.

use crate::document::element::DocElement;
use crate::document::format::{check_savable, load, save};
use crate::document::ids::Document;
use crate::document::tests::format_support::same_document;
use crate::engine::structure::Subtree;
use indextree::{NodeEdge, NodeId};

/// How deep R13's chain is.
pub const CHAIN_DEPTH: u32 = 100_000;

/// The leaf's content: the deepest a node's own JSON nests (design
/// B.7), a portal link whose second end has a label whose span links
/// out, nine levels below the node.
const LEAF: [&str; 2] = [
    r#""sections": [{"spans": [{"text": "the leaf"}]}]"#,
    r#""links": [{"to": 1, "kind": "portal", "ends": [{}, {"label": {"spans": [{"text": "nine levels down", "link": {"url": "https://example.org/deep"}}]}}]}]"#,
];

/// The text of a chain `depth` nodes deep, exactly as the saver writes
/// it: node 1 at the top, node `k` under node `k - 1`, the leaf last.
pub fn chain_text(depth: u32) -> String {
    let mut text = String::from("{\n  \"format\": 1,\n  \"name\": \"Chain\",\n  \"nodes\": [\n");
    for id in 1..=depth {
        if id > 1 {
            text.push_str(",\n");
        }
        // A block per node, a line per key (design B.9, ruling R20).
        text.push_str(&format!("    {{\n      \"id\": {id},\n"));
        if id > 1 {
            text.push_str(&format!("      \"parent\": {},\n", id - 1));
        }
        text.push_str("      \"position\": [0.0, 0.0]");
        if id == depth {
            for key in LEAF {
                text.push_str(",\n      ");
                text.push_str(key);
            }
        }
        text.push_str("\n    }");
    }
    text.push_str("\n  ]\n}\n");
    text
}

/// The subtree of `doc` under `top`, detached, as a structural insert
/// would carry it. Built iteratively, by the arena's own edges: entering
/// an element opens a level, leaving it closes one into its parent.
pub fn subtree_of(doc: &Document, top: NodeId) -> Option<Subtree<DocElement>> {
    let arena = doc.tree().arena();
    let mut open: Vec<Subtree<DocElement>> = Vec::new();
    for edge in top.traverse(arena) {
        match edge {
            NodeEdge::Start(id) => open.push(Subtree::leaf(doc.get(id)?.clone())),
            NodeEdge::End(_) => {
                let done = open.pop()?;
                match open.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => return Some(done),
                }
            }
        }
    }
    None
}

/// Drop `subtree` without recursing: `Subtree`'s own drop is recursive
/// (V1-A5 makes it iterative), and a chain this deep would exhaust the
/// stack on it. Each level is emptied before it is dropped.
pub fn dismantle(subtree: Subtree<DocElement>) {
    let mut pending = vec![subtree];
    while let Some(mut level) = pending.pop() {
        pending.append(&mut level.children);
    }
}

/// R13's acceptance, for a chain `depth` nodes deep: it loads, its whole
/// subtree passes `validate_subtree`, it passes `check_savable`, it saves
/// to exactly the text it was read from, and that text loads back as the
/// same document. `Err` says which step failed.
pub fn deep_chain_survives_every_walk(depth: u32) -> Result<(), String> {
    let text = chain_text(depth);
    let doc = load(&text).map_err(|error| format!("load: {error}"))?;
    let top = doc
        .children(doc.root())
        .find(|&child| doc.get(child).is_some_and(|element| element.as_node().is_some()))
        .ok_or("the chain has no top node")?;
    let subtree = subtree_of(&doc, top).ok_or("the chain has no subtree")?;
    let checked = Document::validate_subtree(&subtree).map_err(|error| format!("validate_subtree: {error}"));
    dismantle(subtree);
    checked?;
    check_savable(&doc).map_err(|error| format!("check_savable: {error}"))?;
    let saved = save(&doc).map_err(|error| format!("save: {error}"))?;
    if saved != text {
        return Err("save wrote another text than the one it read".to_string());
    }
    let again = load(&saved).map_err(|error| format!("reload: {error}"))?;
    same_document(&doc, &again).map_err(|why| format!("the reloaded chain differs: {why}"))
}

/// The negative control's planted walk: how many elements a chain
/// `depth` deep holds, counted by **recursing** on the depth of the
/// tree — the walk the format must never make. On a stack the size of
/// a default test thread, or wasm32's, a chain of [`CHAIN_DEPTH`]
/// exhausts it; the controls run it to prove that the acceptance test
/// would catch a walk that recursed.
pub fn planted_recursive_count(depth: u32) -> usize {
    fn count(doc: &Document, id: NodeId) -> usize {
        1 + doc.children(id).map(|child| count(doc, child)).sum::<usize>()
    }
    match load(&chain_text(depth)) {
        Ok(doc) => count(&doc, doc.root()),
        Err(_) => 0,
    }
}
