// SPDX-License-Identifier: MPL-2.0

//! `mode` — change the active high-level interaction mode.
//!
//! Verbs:
//! - `mode default` — exit any active mode and return to `Default`.
//!   Equivalent to pressing the `ExitMode` keybind (Esc by default).
//! - `mode resize` — Resize mode on the selected node (or section): its
//!   handles are drawn, and dragging one resizes it, as a transaction
//!   (`Action::EnterResizeMode`'s mode; V1-C7). Refused for a selection
//!   that is not one node or section.
//! - `mode node-edit` — NodeEdit mode on the selection's node (V1-C8):
//!   the other nodes dimmed, its sections outlined, a click on one
//!   selecting it. Refused for a selection that is not one node or one of
//!   its sections.
//!
//! `mode show` is intentionally absent: no surface reports the active
//! mode to the console, so there is no verb for it.

use super::Command;
use crate::application::app::{InteractionMode, ResizeTarget};

/// What `mode resize` says for a selection it cannot resize.
pub(crate) const RESIZE_NEEDS_ONE: &str = "mode resize: select one node or section to resize";
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{kvs, usage, Grammar, Subverb};
use crate::application::console::{ConsoleEffects, ConsoleSideEffect, ExecResult};

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("default", "modes", "leave the current mode and return to Default"),
    Subverb::bare("resize", "modes", "enter Resize on the current selection"),
    Subverb::bare(
        "node-edit",
        "modes",
        "enter NodeEdit on the selection's owning node",
    ),
];

/// Which handler each declared subverb runs (`spec::route`).
#[derive(Clone, Copy)]
enum Route {
    Default,
    Resize,
    NodeEdit,
}

const ROUTES: &Routes<Route> = &[
    ("default", Route::Default),
    ("resize", Route::Resize),
    ("node-edit", Route::NodeEdit),
];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub static GRAMMAR: Grammar = Grammar {
    label: "mode",
    subverb_sets: &[SUBVERBS],
    key_sets: &[],
    bare: None,
};

pub const COMMAND: Command = Command {
    name: "mode",
    aliases: &[],
    summary: "Change the active interaction mode (default / resize / node-edit)",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["interaction", "exit", "edit"],
    execute: execute_mode,
};

fn execute_mode(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Subverb names are matched case-insensitively by the descent,
    // console-wide (`commands/mod.rs` § Casing); the unknown-subverb
    // message still echoes what the user typed.
    let descent = descend(&GRAMMAR, args.tokens());
    if let Err(msg) = kvs::read_strict(&descent, args) {
        return ExecResult::err(msg);
    }
    let route = match descent.stop {
        Stop::Matched(subverb) => match route::lookup(ROUTES, subverb) {
            Some(route) => route,
            None => return ExecResult::err(route::miss(descent.level, subverb)),
        },
        Stop::Bare => return ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
        _ => {
            return ExecResult::err(usage::unknown_subverb_message(
                descent.level,
                descent.typed.unwrap_or_default(),
            ))
        }
    };
    match route {
        Route::Default => {
            eff.side_effect = Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Default));
            eff.close_console = true;
            ExecResult::ok_msg("mode: returning to Default")
        }
        Route::Resize => {
            let doc = eff.document();
            match ResizeTarget::of(doc.doc(), &doc.selection) {
                Some(target) => {
                    eff.side_effect = Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Resize {
                        target,
                    }));
                    eff.close_console = true;
                    ExecResult::ok_msg("mode: Resize — drag a corner or edge")
                }
                None => ExecResult::err(RESIZE_NEEDS_ONE),
            }
        }
        Route::NodeEdit => super::enter_node_edit(eff, "mode node-edit"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::parser::parse;
    use crate::application::console::parser::ParseResult;
    use crate::application::console::Args;
    use crate::application::document::MindMapDocument;

    /// Parse `line` and run the `mode` verb body against `doc`,
    /// returning the result, the side effect the dispatcher would
    /// consume, and whether the console closes.
    fn run_mode(line: &str, doc: &mut MindMapDocument) -> (ExecResult, Option<ConsoleSideEffect>, bool) {
        let tokens = match parse(line) {
            ParseResult::Ok { cmd: _, args } => args,
            ParseResult::Empty => panic!("empty input: {:?}", line),
            ParseResult::Unknown(s) => panic!("unknown command '{}' in {:?}", s, line),
        };
        let mut eff = ConsoleEffects::new(doc);
        let result = execute_mode(&Args::new(&tokens), &mut eff);
        let close = eff.close_console;
        let side = eff.side_effect.take();
        (result, side, close)
    }

    #[test]
    fn test_mode_default_emits_set_default_side_effect() {
        let mut doc = MindMapDocument::new_blank(None, 50);
        let (result, side, close) = run_mode("mode default", &mut doc);
        assert!(matches!(result, ExecResult::Ok { .. }));
        assert!(matches!(
            side,
            Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Default))
        ));
        assert!(close);
    }

    /// `mode resize` enters Resize on the one selected node, and closes
    /// the console. Controls: with nothing selected, and with two nodes
    /// selected, it refuses and requests nothing.
    #[test]
    fn test_mode_resize_enters_resize_on_the_one_selected_node() {
        let mut doc = MindMapDocument::new_blank(None, 50);
        doc.create_root_for_test();
        let (refused, side, close) = run_mode("mode resize", &mut doc);
        assert!(
            matches!(&refused, ExecResult::Err(m) if m == RESIZE_NEEDS_ONE),
            "nothing selected: {refused:?}"
        );
        assert!(side.is_none() && !close);
        let node = baumhard::document::ids::NodeIdent(1);
        let node = doc.doc().node_by_ident(node).expect("the root is node 1");
        doc.selection.select_only(node);
        let (result, side, close) = run_mode("mode resize", &mut doc);
        assert!(matches!(result, ExecResult::Ok { .. }), "{result:?}");
        assert!(matches!(
            side,
            Some(ConsoleSideEffect::SetInteractionMode(InteractionMode::Resize {
                target: ResizeTarget::Node(n)
            })) if n == node
        ));
        assert!(close);
        doc.create_root_for_test();
        let other = doc
            .doc()
            .node_by_ident(baumhard::document::ids::NodeIdent(2))
            .expect("the second root is node 2");
        doc.selection.set([node, other]);
        let (refused, side, _) = run_mode("mode resize", &mut doc);
        assert!(
            matches!(&refused, ExecResult::Err(m) if m == RESIZE_NEEDS_ONE),
            "two selected: {refused:?}"
        );
        assert!(side.is_none());
    }

    /// `mode node-edit` refuses with its owner named, and requests
    /// nothing of the dispatcher.
    #[test]
    fn test_mode_node_edit_refuses_naming_its_owner() {
        let mut doc = MindMapDocument::new_blank(None, 50);
        let (result, side, close) = run_mode("mode node-edit", &mut doc);
        match result {
            ExecResult::Err(message) => assert!(message.contains(super::super::TEXT_OWNER), "{message}"),
            other => panic!("mode node-edit must refuse: {other:?}"),
        }
        assert!(side.is_none());
        assert!(!close);
    }

    #[test]
    fn test_mode_unknown_subverb_lists_the_three() {
        let mut doc = MindMapDocument::new_blank(None, 50);
        let (result, _, _) = run_mode("mode sideways", &mut doc);
        match result {
            ExecResult::Err(message) => {
                for word in ["default", "resize", "node-edit"] {
                    assert!(message.contains(word), "{message}");
                }
            }
            other => panic!("expected an error: {other:?}"),
        }
    }
}
