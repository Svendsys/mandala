// SPDX-License-Identifier: MPL-2.0

//! The executor's half of the `edit` verb: the line the verb read
//! (`console::commands::edit`), run against the app's edit registry and
//! the open map, and what it did, worded for the scrollback. `apply`
//! goes through the funnel every route takes
//! (`dispatch::families::edits::run_edit`), on the app clock.

use baumhard::arena::NodeId;
use baumhard::document::edit::{EditBody, EditDef, Reach, Target};
use baumhard::document::ids::{Document, NodeIdent};
use baumhard::document::writer::ChangeSource;

use crate::application::console::commands::edit::EditLine;
use crate::application::console::ExecResult;
use crate::application::edits::EditTier;

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
                    format!("  {} · {}", reach_name(def.reach), target_name(def.target)),
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
            match ctx.edits.find(doc.doc(), id) {
                Some((def, tier)) => inspect(doc.doc(), def, tier, &selection),
                None => ExecResult::err(format!("edit inspect: no edit is called `{id}`")),
            }
        }
    }
}

/// `edit inspect`: the resolved reach and target, the anchors the edit
/// would use at `selection`, its body and its tier, and the walk it
/// runs, as the walker will run it.
fn inspect(doc: &Document, def: &EditDef, tier: EditTier, selection: &[NodeId]) -> ExecResult {
    let elements = match def.elements(doc, selection) {
        Ok(elements) => elements,
        Err(error) => return ExecResult::err(format!("edit inspect: {error}")),
    };
    let shown: Vec<String> = elements.iter().map(|&id| element_name(doc, id)).collect();
    let walk = match &def.body {
        EditBody::Mutations(_) => format!(
            "walk: one Macro of the mutations, keyed `{:?}`, applied at each of the {} anchors, root only",
            def.target.kind(),
            elements.len()
        ),
        EditBody::Mutator(_) => format!(
            "walk: the mutator, anchored at each of the {} anchors; an anchor inside another's subtree is reached through it",
            elements.len()
        ),
        EditBody::Handler(name) => format!(
            "walks: the handler `{name}` reads the {} nodes and where they are drawn, and returns one walk per write",
            elements.len()
        ),
    };
    ExecResult::lines([
        format!("{} — {} ({})", def.id, def.name, tier.name()),
        format!(
            "reach: {} · target: {}",
            reach_name(def.reach),
            target_name(def.target)
        ),
        format!(
            "anchors ({}): {}",
            elements.len(),
            if shown.is_empty() {
                "none — select a node".to_string()
            } else {
                shown.join(", ")
            }
        ),
        format!("body: {}", def.body_json()),
        walk,
        animation(def),
    ])
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
    use baumhard::document::edit::Easing;
    use baumhard::engine::interpolate::Snap;
    let Some(a) = def.animate else {
        return "instant".to_string();
    };
    let easing = match a.easing {
        Easing::Linear => "linear",
        Easing::EaseIn => "ease_in",
        Easing::EaseOut => "ease_out",
        Easing::EaseInOut => "ease_in_out",
    };
    let snap = match a.snap {
        Snap::Start => "start",
        Snap::End => "end",
    };
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

/// A reach as the file spells it.
fn reach_name(reach: Reach) -> &'static str {
    match reach {
        Reach::SelfNode => "self",
        Reach::Children => "children",
        Reach::Descendants => "descendants",
        Reach::Subtree => "subtree",
        Reach::Parent => "parent",
        Reach::Siblings => "siblings",
    }
}

/// A target as the file spells it.
fn target_name(target: Target) -> &'static str {
    match target {
        Target::Node => "node",
        Target::Sections => "sections",
        Target::Spans => "spans",
        Target::Frame => "frame",
        Target::Rings => "rings",
        Target::Connector => "connector",
        Target::Links => "links",
        Target::Labels => "labels",
    }
}
