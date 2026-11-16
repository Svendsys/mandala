// SPDX-License-Identifier: MPL-2.0

//! The executor's half of the `present` verb: the line the verb read
//! (`console::commands::present`), run against the app's presentation
//! registry and the open map on the app clock, and what it did, worded
//! for the scrollback.

use baumhard::arena::NodeId;
use baumhard::document::element::DocElement;
use baumhard::document::ids::{Document, NodeIdent};
use baumhard::document::presentation::InstanceInfo;

use crate::application::console::commands::present::PresentLine;
use crate::application::console::ExecResult;
use crate::application::document::MindMapDocument;

use super::super::input_context::InputHandlerContext;

/// Do what `line` asks, and say what happened.
pub(super) fn take(ctx: &mut InputHandlerContext<'_>, line: &PresentLine) -> ExecResult {
    let Some(doc) = ctx.document.as_mut() else {
        return ExecResult::err("present: no map is open");
    };
    let now_ms = ctx.clock.now_ms();
    match line {
        PresentLine::Start { id, nodes } => {
            let on = match named_or_selected(doc, nodes, "start") {
                Ok(on) => on,
                Err(message) => return ExecResult::err(message),
            };
            // The definition is copied out: it may be the map's own, and
            // starting it takes the map exclusively.
            let Some(def) = ctx.presentations.find(doc.doc(), id).map(|(def, _)| def.clone()) else {
                return ExecResult::err(format!("present start: no presentation is called `{id}`"));
            };
            match doc.start_presentation(&def, &on, now_ms) {
                Ok(started) if started.is_empty() => ExecResult::ok_msg(format!(
                    "present: `{id}` has nothing to play on there (its target is {})",
                    def.target.spelling()
                )),
                Ok(started) => ExecResult::ok_msg(format!(
                    "present: `{id}` is playing on {}",
                    count(started.len(), "node")
                )),
                Err(error) => ExecResult::err(error.to_string()),
            }
        }
        PresentLine::Stop { id, nodes } => {
            let on = match named_or_selected(doc, nodes, "stop") {
                Ok(on) => on,
                Err(message) => return ExecResult::err(message),
            };
            match doc.stop_presentation(id, &on, now_ms) {
                0 => ExecResult::err(format!("present stop: `{id}` is not playing there")),
                stopped => {
                    ExecResult::ok_msg(format!("present: stopped `{id}` on {}", count(stopped, "node")))
                }
            }
        }
        PresentLine::List => {
            let mut lines: Vec<String> = ctx
                .presentations
                .list(doc.doc())
                .into_iter()
                .map(|(def, tier)| {
                    format!(
                        "{}  {}  ({} · {} · {})",
                        def.id,
                        def.name,
                        def.play.spelling(),
                        def.layer.spelling(),
                        tier.name()
                    )
                })
                .collect();
            if lines.is_empty() {
                lines.push("no presentations".to_string());
            }
            let playing: Vec<String> = doc
                .doc()
                .presentations()
                .map(|instance| instance_line(doc.doc(), &instance))
                .collect();
            if playing.is_empty() {
                lines.push("playing: none".to_string());
            } else {
                lines.push(format!("playing ({}):", playing.len()));
                lines.extend(playing);
            }
            ExecResult::lines(lines)
        }
    }
}

/// `n` of `what`, in words: `1 node`, `3 nodes`.
fn count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

/// The nodes a line names by id, or the selection when it names none.
/// A named id the map lacks, and nothing named with nothing selected,
/// are refusals: the line says where, and where is nowhere.
fn named_or_selected(doc: &MindMapDocument, nodes: &[u32], verb: &str) -> Result<Vec<NodeId>, String> {
    if nodes.is_empty() {
        let selection: Vec<NodeId> = doc.selection.iter().collect();
        if selection.is_empty() {
            return Err(format!(
                "present {verb}: on which nodes? select some, or name them by id"
            ));
        }
        return Ok(selection);
    }
    nodes
        .iter()
        .map(|&ident| {
            doc.doc()
                .node_by_ident(NodeIdent(ident))
                .ok_or_else(|| format!("present {verb}: no node has the id {ident}"))
        })
        .collect()
}

/// One playing instance, as `present list` words it: its presentation,
/// its node, its progress, and which way it is going.
fn instance_line(doc: &Document, instance: &InstanceInfo<'_>) -> String {
    let node = match doc.get(instance.node) {
        Some(DocElement::Node(node)) => format!("node {}", node.ident.0),
        _ => "a node no longer in the map".to_string(),
    };
    let mut line = format!(
        "  #{} {} on {node}: t {:.3}, {}",
        instance.id.0,
        instance.presentation,
        instance.t,
        instance.direction.name()
    );
    if let Some(refusal) = instance.refusal {
        line.push_str(&format!(" — not applied: {refusal}"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::presentations::PresentationRegistry;
    use crate::application::presentations::PresentationTier;

    /// `present list` words an instance by its node's id, its progress
    /// and its direction, and the definitions by their lifecycle, layer
    /// and tier.
    #[test]
    fn test_an_instance_is_listed_by_its_node_progress_and_direction() {
        let registry = PresentationRegistry::bundled();
        let mut doc = load_test_doc();
        let node = doc
            .doc()
            .node_by_ident(NodeIdent(1))
            .expect("the test map has a node 1");
        let (lift, tier) = registry.find(doc.doc(), "lift").expect("bundled");
        assert_eq!(tier, PresentationTier::App);
        let lift = lift.clone();
        doc.start_presentation(&lift, &[node], 0.0).unwrap();
        doc.tick_presentations(75.0);
        let lines: Vec<String> = doc
            .doc()
            .presentations()
            .map(|instance| instance_line(doc.doc(), &instance))
            .collect();
        // 75 of `lift`'s 150 ms, eased out: 1 - (1 - 0.5)².
        assert_eq!(lines, ["  #0 lift on node 1: t 0.750, in"]);
        doc.tick_presentations(150.0);
        doc.stop_presentation("lift", &[node], 150.0);
        let line = instance_line(doc.doc(), &doc.doc().presentations().next().unwrap());
        assert_eq!(line, "  #0 lift on node 1: t 1.000, out");
        assert_eq!(count(1, "node"), "1 node");
        assert_eq!(count(2, "node"), "2 nodes");
        // Nothing named and nothing selected is a refusal; a named id the
        // map lacks is another.
        assert!(named_or_selected(&doc, &[], "start").is_err());
        assert_eq!(named_or_selected(&doc, &[1], "start"), Ok(vec![node]));
        assert_eq!(
            named_or_selected(&doc, &[999_999], "stop"),
            Err("present stop: no node has the id 999999".to_string())
        );
    }
}
