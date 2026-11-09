// SPDX-License-Identifier: MPL-2.0

//! The executor's half of the `edit` verb: the line the verb read
//! (`console::commands::edit`), run against the app's edit registry and
//! the open map, and what it did, worded for the scrollback. `apply`
//! goes through the funnel every route takes
//! (`dispatch::families::edits::run_edit`), on the app clock.

use baumhard::arena::NodeId;
use baumhard::document::edit::EditDef;
use baumhard::document::ids::{Document, NodeIdent};
use baumhard::document::writer::ChangeSource;
use baumhard::engine::anchor::Anchors;
use baumhard::engine::mutator::Mutator;
use baumhard::projection::Geometry;

use crate::application::console::commands::edit::EditLine;
use crate::application::console::ExecResult;
use crate::application::edits::{EditRegistry, EditTier};

use super::super::dispatch::families::edits::{run_edit, with_drawn};
use super::super::input_context::InputHandlerContext;

/// Do what `line` asks, and say what happened.
pub(super) fn take(ctx: &mut InputHandlerContext<'_>, line: &EditLine) -> ExecResult {
    let Some(doc) = ctx.document.as_mut() else {
        return ExecResult::err("edit: no map is open");
    };
    match line {
        EditLine::List => {
            let lines: Vec<String> = ctx
                .edits
                .list(doc.doc())
                .into_iter()
                .map(|(def, tier)| format!("{}  {}  ({})", def.id, def.name, tier.name()))
                .collect();
            if lines.is_empty() {
                ExecResult::ok_msg("no edits")
            } else {
                ExecResult::lines(lines)
            }
        }
        EditLine::Apply { id, nodes } => {
            let mut anchors = Vec::with_capacity(nodes.len());
            for &ident in nodes {
                match doc.doc().node_by_ident(NodeIdent(ident)) {
                    Some(node) => anchors.push(node),
                    None => return ExecResult::err(format!("edit apply: no node has the id {ident}")),
                }
            }
            let at = (!anchors.is_empty()).then_some(anchors.as_slice());
            let now_ms = ctx.clock.now_ms();
            let registry = ctx.edits;
            let ran = with_drawn(ctx.view.as_mut(), ctx.renderer, |drawn| {
                run_edit(doc, drawn, registry, id, at, ChangeSource::Console, now_ms)
            });
            match ran {
                Ok(()) if doc.is_animating() => ExecResult::ok_msg(format!("edit: `{id}` is animating")),
                Ok(()) => ExecResult::ok_msg(format!("edit: ran `{id}`")),
                Err(message) => ExecResult::err(message),
            }
        }
        EditLine::Help(id) => match ctx.edits.find(doc.doc(), id) {
            Some((def, tier)) => {
                let mut lines = vec![
                    format!("{} — {}", def.id, def.name),
                    format!("  from the {} tier", tier.name()),
                    format!("  {} · {}", def.reach.spelling(), def.target.spelling()),
                ];
                if !def.description.is_empty() {
                    lines.push(format!("  {}", def.description));
                }
                lines.push(format!("  {}", animation(def)));
                ExecResult::lines(lines)
            }
            None => ExecResult::err(format!("edit help: no edit is called `{id}`")),
        },
        EditLine::Inspect(id) => {
            let selection: Vec<NodeId> = doc.selection.iter().collect();
            let registry = ctx.edits;
            with_drawn(ctx.view.as_mut(), ctx.renderer, |drawn| {
                let geometry = drawn.synced(doc);
                match registry.find(doc.doc(), id) {
                    Some((def, tier)) => inspect(doc.doc(), geometry, registry, def, tier, &selection),
                    None => ExecResult::err(format!("edit inspect: no edit is called `{id}`")),
                }
            })
        }
    }
}

/// `edit inspect`: the resolved reach and target, the elements the edit
/// reaches at `selection`, its body and its tier, and its walks as the
/// walker runs them — a handler run to get them, which only reads —
/// each with the anchors it starts from, and apart from those the ones
/// it does not start from because they lie inside another's subtree.
fn inspect(
    doc: &Document,
    geometry: &dyn Geometry,
    registry: &EditRegistry,
    def: &EditDef,
    tier: EditTier,
    selection: &[NodeId],
) -> ExecResult {
    let fail = |error: &dyn std::fmt::Display| ExecResult::err(format!("edit inspect: {error}"));
    let elements = match def.elements(doc, selection) {
        Ok(elements) => elements,
        Err(error) => return fail(&error),
    };
    let walks = match def.walks(doc, geometry, registry.handlers(), selection) {
        Ok(walks) => walks,
        Err(error) => return fail(&error),
    };
    let names = |ids: &[NodeId]| {
        ids.iter()
            .map(|&id| element_name(doc, id))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut lines = vec![
        format!("{} — {} ({})", def.id, def.name, tier.name()),
        format!(
            "reach: {} · target: {}",
            def.reach.spelling(),
            def.target.spelling()
        ),
        if elements.is_empty() {
            "reached: none — select a node".to_string()
        } else {
            format!("reached ({}): {}", elements.len(), names(&elements))
        },
        format!("body: {}", def.body_json()),
        format!("walks ({}):", walks.len()),
    ];
    for (i, walk) in walks.iter().enumerate() {
        let starts = match walk.mutator.walk_starts(doc.tree(), &walk.anchors) {
            Ok(starts) => starts,
            Err(error) => return fail(&error),
        };
        let what = match walk.mutator.arena()[walk.mutator.root()].get() {
            Mutator::Single { mutation, .. } => format!("{mutation:?}"),
            Mutator::Macro { mutations, .. } => format!("a macro of {} mutations", mutations.len()),
            _ => "the mutator".to_string(),
        };
        let below = if walk.mutator.reaches_below_root() {
            ", and below"
        } else {
            ""
        };
        lines.push(format!("  {}. {what}, at {}{below}", i + 1, names(&starts)));
        let given: &[NodeId] = match &walk.anchors {
            Anchors::Nodes(nodes) => nodes,
            _ => &[],
        };
        for &inner in given.iter().filter(|id| !starts.contains(id)) {
            let outer =
                std::iter::successors(doc.parent(inner), |&id| doc.parent(id)).find(|id| starts.contains(id));
            if let Some(outer) = outer {
                lines.push(format!(
                    "     not from {}: inside {}, changed only where the walk from it reaches",
                    element_name(doc, inner),
                    element_name(doc, outer)
                ));
            }
        }
    }
    lines.push(animation(def));
    ExecResult::lines(lines)
}

/// An element, for a person: its kind and its node's id.
fn element_name(doc: &Document, id: NodeId) -> String {
    let kind = doc.get(id).map(|element| element.kind());
    let ident = doc.owning_node(id).and_then(|node| match doc.get(node) {
        Some(baumhard::document::element::DocElement::Node(data)) => Some(data.ident.0),
        _ => None,
    });
    match (kind, ident) {
        (Some(baumhard::document::element::DocKind::Node), Some(ident)) => format!("node {ident}"),
        (Some(kind), Some(ident)) => format!("{kind:?} of node {ident}").to_lowercase(),
        _ => format!("{id}"),
    }
}

/// How an edit animates, in words.
fn animation(def: &EditDef) -> String {
    let Some(a) = def.animate else {
        return "instant".to_string();
    };
    let (easing, snap) = (a.easing.spelling(), a.snap.spelling());
    let delay = if a.delay_ms > 0 {
        format!(" after {} ms", a.delay_ms)
    } else {
        String::new()
    };
    format!(
        "animates over {} ms{delay}, {easing}, snapping at the {snap}",
        a.ms
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::dispatch::families::edits::Undrawn;
    use crate::application::document::MindMapDocument;

    /// Node 1 with its child node 2, and an edit that aligns a node's
    /// sections: a mutator that reaches below its root.
    const MAP: &str = r##"{"format":1,"name":"t",
"edits":[{"id":"align","name":"Align","reach":"self","target":"node","mutator":{"void":{"kind":"node","channel":0,"children":[{"macro":{"kind":"section","channel":0,"mutations":[{"section":{"set_align":"right"}}]}}]}}}],
"nodes":[{"id":1,"position":[0,0],"sections":[{"spans":[{"text":"a"}]}]},
{"id":2,"parent":1,"position":[0,100],"sections":[{"spans":[{"text":"b"}]}]}]}"##;

    /// **`edit inspect` names where the walk starts, and the anchor it
    /// does not start from** (review of #331). With nodes 1 and 2
    /// selected the walk starts at node 1 only; node 2 is listed apart,
    /// inside node 1.
    ///
    /// Control: when it printed the reached elements as the anchors,
    /// node 2 read as one the walk starts from.
    #[test]
    fn test_inspect_shows_the_walk_starts_and_the_nested_anchor_apart() {
        let doc = MindMapDocument::from_json_str(MAP, None, 50).expect("the map loads");
        let node = |ident| doc.doc().node_by_ident(NodeIdent(ident)).expect("the node");
        let registry = EditRegistry::new(Vec::new(), Vec::new());
        let (def, tier) = registry.find(doc.doc(), "align").expect("the map's edit");
        let ExecResult::Lines(lines) =
            inspect(doc.doc(), &Undrawn, &registry, def, tier, &[node(2), node(1)])
        else {
            panic!("inspect answers in lines");
        };
        let text: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert!(text.contains(&"reached (2): node 1, node 2"), "{text:#?}");
        assert!(
            text.contains(&"  1. the mutator, at node 1, and below"),
            "{text:#?}"
        );
        assert!(
            text.contains(
                &"     not from node 2: inside node 1, changed only where the walk from it reaches"
            ),
            "{text:#?}"
        );
    }
}
