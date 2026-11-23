// SPDX-License-Identifier: MPL-2.0

//! Console command registry.
//!
//! Each command lives in its own submodule so the surface stays
//! scannable. The public `COMMANDS` slice gathers them in one place,
//! matching the `const PALETTE_ACTIONS` pattern — zero-cost startup,
//! no HashMap construction, and a by-id lookup is a
//! linear scan over a dozen entries.
//!
//! # Casing
//!
//! One rule, and the engine is now the one place that applies it:
//!
//! - **Command names, aliases and positional subverbs are matched
//!   case-insensitively.** `mode DEFAULT`, `fps ON` and
//!   `clock STEP 16` all run. The grammar descent lowercases
//!   once and the popup filters its partial the same way, so the
//!   two cannot disagree; before the declaration, five verbs
//!   honored the rule and the rest did not, which meant a word the
//!   popup listed could still be refused when typed out in full.
//! - **Kv *keys* are exact.** `screenshot WIDTH=9` is `unknown key
//!   'WIDTH'`, and the engine's key rows filter case-sensitively to
//!   say so. A key is a field name, not a word the user picks.
//! - **Kv *values* belong to the key's own parser.** A closed
//!   vocabulary is matched case-insensitively; a document-derived one
//!   is matched the way its own rows are found. A positional *slot
//!   word* from a closed vocabulary follows the same rule as a closed
//!   kv value. The console has no flag grammar; a `--` word is one
//!   more slot word. Like a closed kv value, a slot word outside its
//!   list is refused by name, never read as the slot's absence.
//! - **A refusal reports the token as typed.** Matching
//!   case-insensitively and *reporting* the normalized form are
//!   separate decisions, and only the first is intended: a user
//!   reads the message back looking for their own spelling, so a
//!   refused `Coral` is reported as `'Coral'`, never `'coral'`
//!   (#136).
//!
//! What makes the first bullet checkable rather than aspirational
//! is `console::tests::oracle_corpus`: every verb whose subverb
//! dispatch normalizes carries an upper-case corpus row beside its
//! lower-case one. (Named rather than linked — the module is
//! `#[cfg(test)]`, so rustdoc cannot see it even under
//! `--document-private-items`, and an intra-doc link to it fails
//! the `-D warnings` doc gate.)
//!
//! # Usage, tags and completion come from the grammar
//!
//! A verb declares one [`crate::application::console::spec::Grammar`]
//! and nothing else: [`Command::usage_forms`], [`Command::key_lines`],
//! [`Command::tag_list`] and [`Command::completions`] are all derived
//! from it, and the kv parse loop reads it too. Adding a key is one
//! table row, and `help <verb>` documents it in the same edit that
//! makes it parseable.
//!
//! That replaces a genuine defect rather than a stylistic one.
//! `Command` used to carry `usage` and `tags` as `&'static str`
//! literals that `help` printed verbatim while nothing derived them
//! from the verb's key list, so the three declarations could
//! disagree: a key added to a verb's key list was offered by the
//! popup on the next keystroke and stayed absent from `help <verb>`
//! until somebody wrote it in by hand. `font` and `color` each carried
//! that drift for `range=` — parseable, named in the verb's own
//! rejection, and documented nowhere. The two per-verb assertions
//! that closed those *instances* were themselves per-verb copies of
//! a check; `console::spec`'s own tests hold the invariant over every
//! declared level instead, in both directions — a form naming a key
//! its level does not declare, and a key no form prints.
//!
//! There is no second way to declare a verb. [`Command`] has no
//! `usage`, no `tags` and no `complete` field for a grammar to
//! disagree with — a framework with two adopters would leave two
//! grammars in the tree, which is worse than the one hand-rolled
//! grammar it replaced, so every verb reads the engine or none
//! would.

use super::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::console::completion::{Completion, CompletionState};
use crate::application::console::parser::Args;
use crate::application::console::spec::{self, Grammar};

pub mod clock;
pub mod color;
pub mod defaults;
pub mod edit;
pub mod font;
pub mod fps;
pub mod frame;
pub mod help;
pub mod label;
pub mod link;
pub mod mode;
pub mod new;
pub mod node;
pub mod open;
pub mod path;
pub mod portal;
pub mod present;
pub mod range;
pub mod record;
pub mod save;
pub mod screenshot;
pub mod section;
pub mod theme;
pub mod verb;
pub mod zoom;

/// One entry in the console command registry. Kept small and
/// `'static` so the whole registry can live in a `const` slice.
#[derive(Clone, Copy)]
pub struct Command {
    /// Primary name — the token users type at position 0.
    pub name: &'static str,
    /// Alternative names. Case-insensitive in [`command_by_name`].
    pub aliases: &'static [&'static str],
    /// One-line summary shown in `help` with no args.
    pub summary: &'static str,
    /// The verb's declarative grammar (`console::spec`) — the
    /// single source of its usage forms, its `keys:` block, its
    /// tags, its completion popup, its kv parse loop and its hint
    /// surface.
    pub grammar: &'static Grammar,
    /// Search words the grammar does not contain — `wheel` under
    /// `color`, `lod` under `zoom`. Every structural word (subverb
    /// names, kv keys) is derived; this is only what is neither.
    pub synonyms: &'static [&'static str],
    /// Returns `true` when the command should appear in the filtered
    /// `help` list and in completion. Commands whose args are
    /// context-specific but whose verb is always meaningful should
    /// return `true` here and validate in `execute`.
    pub applicable: fn(&ConsoleContext) -> bool,
    /// Run the command. The dispatcher clears the scene cache and
    /// rebuilds after every non-`Err` result.
    pub execute: fn(&Args, &mut ConsoleEffects) -> ExecResult,
}

impl Command {
    /// The usage lines `help <cmd>` prints, one per form.
    pub fn usage_forms(&self) -> Vec<String> {
        spec::usage::forms(self.grammar)
    }

    /// One line per kv key the verb declares, printed by
    /// `help <cmd>` under a `keys:` block. Empty for a verb that
    /// declares no keys.
    pub fn key_lines(&self) -> Vec<String> {
        spec::usage::key_lines(self.grammar)
    }

    /// The search words `help <cmd>` publishes.
    pub fn tag_list(&self) -> Vec<&'static str> {
        spec::usage::tags(self.grammar, self.synonyms)
    }

    /// The completion popup for one cursor position.
    pub fn completions(&self, state: &CompletionState, ctx: &ConsoleContext) -> Vec<Completion> {
        spec::complete::completions(self.grammar, state, ctx)
    }
}

/// The global command registry. Order matters only for `help` — the
/// listing iterates this slice in declaration order.
pub const COMMANDS: &[Command] = &[
    help::COMMAND,
    color::COMMAND,
    edit::COMMAND,
    present::COMMAND,
    verb::COMMAND,
    font::COMMAND,
    fps::COMMAND,
    mode::COMMAND,
    save::COMMAND,
    screenshot::COMMAND,
    record::COMMAND,
    clock::COMMAND,
    open::COMMAND,
    new::COMMAND,
    node::COMMAND,
    section::COMMAND,
    frame::COMMAND,
    range::COMMAND,
    path::COMMAND,
    link::COMMAND,
    label::COMMAND,
    portal::COMMAND,
    theme::COMMAND,
    defaults::COMMAND,
    zoom::COMMAND,
];

/// Look up a command by its name or any alias. Case-insensitive.
pub fn command_by_name(name: &str) -> Option<&'static Command> {
    let lower = name.to_ascii_lowercase();
    COMMANDS.iter().find(|c| {
        c.name.eq_ignore_ascii_case(&lower) || c.aliases.iter().any(|a| a.eq_ignore_ascii_case(&lower))
    })
}

/// Run a field edit from the console (`crate::application::fields`):
/// its steps as one request declared `ChangeSource::Console`, so one
/// line is one undo entry. Answers `<label>: applied`, `<label>: no
/// change`, or the refusal's words.
pub(super) fn edit<E: Into<crate::application::fields::Refused>>(
    eff: &mut ConsoleEffects,
    steps: Result<Vec<baumhard::document::mutation::step::DocStep>, E>,
    label: &str,
) -> ExecResult {
    let source = baumhard::document::writer::ChangeSource::Console;
    match crate::application::fields::apply(eff.document_mut(), steps, label, source) {
        Ok(true) => ExecResult::ok_msg(format!("{label}: applied")),
        Ok(false) => ExecResult::ok_msg(format!("{label}: no change")),
        Err(refused) => ExecResult::err(refused.message()),
    }
}

/// The descent of a verb's line, refused with the engine's own words
/// when the word at a subverb slot names nothing, or a kv made the slot
/// kv form ([`spec::descent::Descent::quoting_hint`]).
pub(super) fn descend<'a>(
    grammar: &'static Grammar,
    args: &Args<'a>,
) -> Result<spec::descent::Descent<'a>, ExecResult> {
    use spec::descent::Stop;
    let descent = spec::descent::descend(grammar, args.tokens());
    match descent.stop {
        Stop::Unknown => Err(ExecResult::err(spec::usage::unknown_subverb_message(
            descent.level,
            descent.typed.unwrap_or_default(),
        ))),
        Stop::KvForm => Err(ExecResult::err(descent.quoting_hint(args))),
        Stop::Bare | Stop::Matched(_) => Ok(descent),
    }
}

/// NodeEdit on the node the selection edits — the one selected node, or
/// the node of the one selected section — for `mode node-edit` and `node
/// edit`, as on `main`: the mode is entered and the console closes; a
/// section is picked, then edited, from there. `verb` names the line in a
/// refusal.
pub(super) fn enter_node_edit(eff: &mut ConsoleEffects, verb: &str) -> ExecResult {
    use baumhard::document::element::DocKind;
    let doc = eff.document();
    let node = only_selected(doc).and_then(|only| {
        match crate::application::document::selection::kind_of(doc.doc(), only)? {
            DocKind::Node => Some(only),
            DocKind::Section => doc.doc().parent(only),
            _ => None,
        }
    });
    let Some(node) = node else {
        return ExecResult::err(format!("{verb}: select one node or one of its sections"));
    };
    eff.side_effect = Some(
        crate::application::console::ConsoleSideEffect::SetInteractionMode(
            crate::application::app::InteractionMode::NodeEdit { node },
        ),
    );
    eff.close_console = true;
    // "pick" only where there is something to pick: a node of one section
    // has nothing to choose between (as the status line says, #249).
    let sections = doc_sections(eff, node);
    if sections > 1 {
        ExecResult::ok_msg(format!("{verb}: editing the node; pick a section"))
    } else {
        ExecResult::ok_msg(format!("{verb}: editing the node"))
    }
}

/// How many sections `node` has in the console's document. O(its
/// children).
fn doc_sections(eff: &mut ConsoleEffects, node: baumhard::arena::NodeId) -> usize {
    use baumhard::document::element::DocKind;
    let doc = eff.document();
    doc.doc()
        .children(node)
        .filter(|&child| {
            crate::application::document::selection::kind_of(doc.doc(), child) == Some(DocKind::Section)
        })
        .count()
}

/// Open the editor `open` names, closing the console: what the verbs that
/// open an editor hand the executor. `what` is the line's acknowledgment.
pub(super) fn open_editor(
    eff: &mut ConsoleEffects,
    open: crate::application::console::EditorOpen,
    what: &str,
) -> ExecResult {
    eff.side_effect = Some(crate::application::console::ConsoleSideEffect::OpenEditor(open));
    eff.close_console = true;
    ExecResult::ok_msg(what.to_string())
}

/// The one selected element, when exactly one is selected. O(1).
pub(super) fn only_selected(
    doc: &crate::application::document::MindMapDocument,
) -> Option<baumhard::arena::NodeId> {
    let mut selected = doc.selection.iter();
    match (selected.next(), selected.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lookup is case-insensitive across both names and aliases,
    /// returns `None` on unknown, and resolves aliases to their
    /// canonical entry. Also pins the registered verb names — the
    /// compiler enforces that the `COMMANDS` slice exists, but a
    /// typo in `name: "label"` → `"lable"` would compile and
    /// silently break user-facing console input without this list.
    #[test]
    fn test_command_by_name_lookup() {
        assert!(command_by_name("HELP").is_some());
        assert!(command_by_name("PaTh").is_some());
        assert_eq!(command_by_name("?").map(|c| c.name), Some("help"));
        assert_eq!(command_by_name("visibility").map(|c| c.name), Some("zoom"));
        assert!(command_by_name("nope").is_none());

        for name in [
            "help", "path", "link", "label", "portal", "color", "font", "fps", "save", "open", "new", "zoom",
        ] {
            assert!(
                command_by_name(name).is_some(),
                "console verb '{name}' missing from registry"
            );
        }
    }

    /// **The path verbs V1-C6 (#247) replaced are gone, with no alias**:
    /// `edge`, `anchor`, `body`, `cap` and `spacing` name no verb, so a
    /// line naming one is refused as unknown rather than run by its
    /// replacement. Control: each replacement is registered under its own
    /// name.
    #[test]
    fn test_the_replaced_path_verbs_name_nothing() {
        for gone in ["edge", "anchor", "body", "cap", "spacing"] {
            assert!(
                command_by_name(gone).is_none(),
                "`{gone}` was replaced and must not resolve"
            );
        }
        for name in ["path", "link", "label", "portal"] {
            assert_eq!(
                command_by_name(name).map(|c| c.name),
                Some(name),
                "control: `{name}`"
            );
        }
    }
}
