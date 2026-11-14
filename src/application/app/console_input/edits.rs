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
use rustc_hash::{FxHashMap, FxHashSet};

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
            let animating = doc.is_animating();
            with_drawn(ctx.view.as_mut(), ctx.renderer, |drawn| {
                let geometry = drawn.synced(doc);
                match registry.find(doc.doc(), id) {
                    Some((def, tier)) => {
                        inspect(doc.doc(), geometry, registry, def, tier, &selection, animating)
                    }
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
///
/// It reads the map as it stands and does not finish an edit that is
/// still animating, as `edit apply` would first; when one is, the
/// output says so (`animating`).
fn inspect(
    doc: &Document,
    geometry: &dyn Geometry,
    registry: &EditRegistry,
    def: &EditDef,
    tier: EditTier,
    selection: &[NodeId],
    animating: bool,
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
    if animating {
        lines.insert(
            1,
            "resolved at the current frame: an edit is still animating, which `edit apply` would finish first"
                .to_string(),
        );
    }
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
        let starts: FxHashSet<NodeId> = starts.into_iter().collect();
        let nested: Vec<NodeId> = given.iter().copied().filter(|id| !starts.contains(id)).collect();
        for (inner, outer) in outer_starts(|id| doc.parent(id), &starts, &nested) {
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

/// The walk start each of `nested` lies inside: its nearest ancestor in
/// `starts`, or `None` (an anchor of another kind, dropped for that).
/// Each climb stops at the first node an earlier one answered, and
/// every node it passed is answered with it, so the work is linear in
/// the nodes climbed through, whatever the depth (§20 rule 11).
fn outer_starts(
    parent: impl Fn(NodeId) -> Option<NodeId>,
    starts: &FxHashSet<NodeId>,
    nested: &[NodeId],
) -> Vec<(NodeId, Option<NodeId>)> {
    let mut known: FxHashMap<NodeId, Option<NodeId>> = FxHashMap::default();
    nested
        .iter()
        .map(|&node| {
            let mut path = vec![node];
            let mut at = parent(node);
            let outer = loop {
                match at {
                    None => break None,
                    Some(up) if starts.contains(&up) => break Some(up),
                    Some(up) => match known.get(&up) {
                        Some(&outer) => break outer,
                        None => {
                            path.push(up);
                            at = parent(up);
                        }
                    },
                }
            };
            for passed in path {
                known.insert(passed, outer);
            }
            (node, outer)
        })
        .collect()
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
"edits":[{"id":"align","name":"Align","reach":"self","target":"node","mutator":{"void":{"kind":"node","channel":0,"children":[{"macro":{"kind":"section","channel":0,"mutations":[{"section":{"set_align":"right"}}]}}]}}},
 {"id":"nudge","name":"Nudge","reach":"self","target":"node","mutations":[{"node":{"nudge":[10.0,0.0]}}],"animate":{"ms":400}}],
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
        let ExecResult::Lines(lines) = inspect(
            doc.doc(),
            &Undrawn,
            &registry,
            def,
            tier,
            &[node(2), node(1)],
            false,
        ) else {
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

    /// **The nested anchors are attributed in work linear in what is
    /// climbed** (review of #331, §20 rule 11). Two chains of 2,000
    /// nodes each, every node an anchor and each chain's top the walk's
    /// start: every other node is found inside its own chain's top, and
    /// the climbs together take at most one step per node.
    ///
    /// Control: an unmemoized climb took 3,998,000 steps for 3,998 nodes.
    #[test]
    fn test_nested_anchors_are_attributed_in_one_step_per_node() {
        const DEPTH: u32 = 2_000;
        let mut nodes = Vec::new();
        for chain in 0..2 {
            let base = chain * DEPTH;
            nodes.push(format!(
                r#"{{"id":{},"channel":{chain},"position":[0,0]}}"#,
                base + 1
            ));
            for i in 2..=DEPTH {
                nodes.push(format!(
                    r#"{{"id":{},"parent":{},"position":[0,0]}}"#,
                    base + i,
                    base + i - 1
                ));
            }
        }
        let text = format!(r#"{{"format":1,"name":"t","nodes":[{}]}}"#, nodes.join(","));
        let doc = MindMapDocument::from_json_str(&text, None, 50).expect("the chains load");
        let d = doc.doc();
        let node = |ident| d.node_by_ident(NodeIdent(ident)).expect("the node");
        let tops = [node(1), node(DEPTH + 1)];
        let starts: FxHashSet<NodeId> = tops.iter().copied().collect();
        let nested: Vec<NodeId> = (0..2)
            .flat_map(|chain| (2..=DEPTH).map(move |i| chain * DEPTH + i))
            .map(node)
            .collect();
        let steps = std::cell::Cell::new(0usize);
        let found = outer_starts(
            |id| {
                steps.set(steps.get() + 1);
                d.parent(id)
            },
            &starts,
            &nested,
        );
        for (inner, outer) in &found {
            let chain =
                usize::from(nested.iter().position(|n| n == inner).expect("asked") >= (DEPTH as usize - 1));
            assert_eq!(*outer, Some(tops[chain]));
        }
        assert!(
            steps.get() <= nested.len() + 2,
            "{} steps for {} nodes",
            steps.get(),
            nested.len()
        );
    }

    /// **`edit inspect` says when an edit is still animating** (review of
    /// #331): it reads the map as it stands, where `edit apply` would
    /// finish the animation first.
    ///
    /// Control: with nothing animating, the line is absent.
    #[test]
    fn test_inspect_says_when_an_edit_is_animating() {
        use crate::application::app::dispatch::families::edits::run_edit;
        use baumhard::document::writer::ChangeSource;
        let mut doc = MindMapDocument::from_json_str(MAP, None, 50).expect("the map loads");
        let one = doc.doc().node_by_ident(NodeIdent(1)).expect("node 1");
        let registry = EditRegistry::new(Vec::new(), Vec::new());
        let note = |doc: &MindMapDocument| {
            let (def, tier) = registry.find(doc.doc(), "align").expect("the map's edit");
            let ExecResult::Lines(lines) = inspect(
                doc.doc(),
                &Undrawn,
                &registry,
                def,
                tier,
                &[one],
                doc.is_animating(),
            ) else {
                panic!("inspect answers in lines");
            };
            lines.iter().any(|line| line.text.contains("still animating"))
        };
        assert!(!note(&doc), "control: nothing animates yet");
        run_edit(
            &mut doc,
            &mut Undrawn,
            &registry,
            "nudge",
            Some(&[one]),
            ChangeSource::Edit,
            0.0,
        )
        .expect("the nudge starts");
        doc.tick_animation(100.0);
        assert!(doc.is_animating(), "precondition");
        assert!(note(&doc), "the line says the map is mid-animation");
    }
}
