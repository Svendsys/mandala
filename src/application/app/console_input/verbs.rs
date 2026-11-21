// SPDX-License-Identifier: MPL-2.0

//! The executor's half of the `verb` console verb: the line it read
//! (`console::commands::verb`), run against the open map on the app
//! clock — through the one runner every route takes
//! (`dispatch::families::verbs`) — and what it did, worded for the
//! scrollback.

use baumhard::document::element::DocElement;
use baumhard::document::ids::Document;
use baumhard::document::verb::run::RunInfo;

use crate::application::app::dispatch::families::edits::with_drawn;
use crate::application::app::dispatch::families::verbs::{node_named, Host};
use crate::application::console::commands::verb::VerbLine;
use crate::application::console::ExecResult;

use super::super::input_context::InputHandlerContext;

/// Do what `line` asks, and say what happened.
pub(super) fn take(ctx: &mut InputHandlerContext<'_>, line: &VerbLine) -> ExecResult {
    let Some(doc) = ctx.document.as_mut() else {
        return ExecResult::err("verb: no map is open");
    };
    match line {
        VerbLine::Run { id, node } => {
            let on = match node.map(|ident| node_named(doc.doc(), ident)).transpose() {
                Ok(on) => on,
                Err(message) => return ExecResult::err(format!("verb run: {message}")),
            };
            let now_ms = ctx.clock.now_ms();
            let ran = with_drawn(ctx.view.as_mut(), ctx.renderer, |drawn| {
                Host {
                    doc,
                    drawn,
                    edits: ctx.edits,
                }
                .invoke(id, on, false, now_ms)
            });
            match ran {
                Ok(_) => ExecResult::ok_msg(format!("verb: ran `{id}`")),
                Err(message) => ExecResult::err(format!("verb run: {message}")),
            }
        }
        VerbLine::List => ExecResult::lines(list(doc.doc(), doc.runs().runs())),
    }
}

/// `verb list`'s lines: each verb by id and name, marked as changing the
/// map when it has an `edit` step (its own or a verb's it runs) and as a
/// performance otherwise (design §11.1), then the runs active.
fn list(doc: &Document, runs: impl Iterator<Item = RunInfo>) -> Vec<String> {
    let definitions = doc.definitions();
    let mut lines: Vec<String> = definitions
        .verbs
        .iter()
        .map(|verb| {
            let kind = if verb.changes_the_map(definitions) {
                "changes the map"
            } else {
                "performance"
            };
            format!("{}  {}  ({kind})", verb.id, verb.name)
        })
        .collect();
    if lines.is_empty() {
        lines.push("no verbs".to_string());
    }
    let running: Vec<String> = runs.map(|run| run_line(doc, &run)).collect();
    if running.is_empty() {
        lines.push("running: none".to_string());
    } else {
        lines.push(format!("running ({}):", running.len()));
        lines.extend(running);
    }
    lines
}

/// One active run, as `verb list` words it: its verb, its node, and
/// whether it waits or is held.
fn run_line(doc: &Document, run: &RunInfo) -> String {
    let verb = doc
        .definitions()
        .verbs
        .get(usize::from(run.verb.0))
        .map_or("?", |verb| verb.id.as_str());
    let on = match run.self_node.and_then(|node| doc.get(node)) {
        Some(DocElement::Node(node)) => format!(" on node {}", node.ident.0),
        Some(_) | None if run.self_node.is_some() => " on a node no longer in the map".to_string(),
        _ => String::new(),
    };
    let state = match (run.resumes_at_ms, run.held) {
        (Some(at), _) => format!("waits until {at:.0} ms"),
        (None, true) => "held".to_string(),
        (None, false) => "done".to_string(),
    };
    format!("  #{} {verb}{on}: {state}", run.id.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::document::format::load;

    /// `verb list` marks a verb with an `edit` step — its own, or one of
    /// a verb it runs — as changing the map, and the others as a
    /// performance. Control: the verb that only presents is a
    /// performance, though it runs inside one that edits.
    #[test]
    fn test_verb_list_marks_the_verbs_that_change_the_map() {
        let doc = load(
            r#"{"format":1,"name":"t",
            "edits":[{"id":"e","name":"E","reach":"self","target":"node","mutations":[{"node":{"set_opacity":0.5}}]}],
            "presentations":[{"id":"p","name":"P","play":"hold","target":"node","mutations":[]}],
            "verbs":[
              {"id":"show","name":"Show","steps":[{"present":"p","on":"selection"}]},
              {"id":"fade","name":"Fade","steps":[{"edit":"e","on":"selection"}]},
              {"id":"both","name":"Both","steps":[{"verb":"show"},{"verb":"fade"}]}
            ]}"#,
        )
        .expect("loads");
        assert_eq!(
            list(&doc, std::iter::empty()),
            [
                "show  Show  (performance)",
                "fade  Fade  (changes the map)",
                "both  Both  (changes the map)",
                "running: none",
            ]
        );
    }
}
