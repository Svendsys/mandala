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

pub mod anchor;
pub mod body;
pub mod cap;
pub mod clock;
pub mod color;
pub mod font;
pub mod fps;
pub mod help;
pub mod label;
pub mod mode;
pub mod new;
pub mod node;
pub mod open;
pub mod range_kv;
pub mod record;
pub mod save;
pub mod screenshot;
pub mod section;
pub mod spacing;
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
    anchor::COMMAND,
    body::COMMAND,
    cap::COMMAND,
    color::COMMAND,
    font::COMMAND,
    fps::COMMAND,
    spacing::COMMAND,
    label::COMMAND,
    mode::COMMAND,
    save::COMMAND,
    screenshot::COMMAND,
    record::COMMAND,
    clock::COMMAND,
    open::COMMAND,
    new::COMMAND,
    node::COMMAND,
    section::COMMAND,
    zoom::COMMAND,
];

/// Look up a command by its name or any alias. Case-insensitive.
pub fn command_by_name(name: &str) -> Option<&'static Command> {
    let lower = name.to_ascii_lowercase();
    COMMANDS.iter().find(|c| {
        c.name.eq_ignore_ascii_case(&lower) || c.aliases.iter().any(|a| a.eq_ignore_ascii_case(&lower))
    })
}

/// The owner of the node, section and style verbs (`node`, `section`,
/// `color`, `font`, `zoom`), as a refusal names it.
pub(super) const FIELD_NODES_OWNER: &str = "V1-C5 (#246)";

/// The owner of the connector, link and label verbs (`anchor`,
/// `body`, `cap`, `label`, `spacing`), as a refusal names it.
pub(super) const FIELD_PATHS_OWNER: &str = "V1-C6 (#247)";

/// The owner of Resize mode and its handle drags (`mode resize`), as a
/// refusal names it.
pub(super) const TRANSACTIONS_OWNER: &str = "V1-C7 (#248)";

/// The owner of node editing (`mode node-edit`), as a refusal names it.
pub(super) const TEXT_OWNER: &str = "V1-C8 (#249)";

/// The answer a console verb that edited the old model gets until
/// `owner` rebuilds it on the one writer: an error naming the owner, and
/// the same at `debug!` — the console's peer of an unhandled Action
/// (`app::dispatch::families::unhandled`). The verb's grammar stays
/// registered, so its help and completion are the ones the owner
/// extends.
pub(super) fn rebuilt_by(verb: &str, owner: &str) -> ExecResult {
    log::debug!("console: `{verb}` is rebuilt on the one writer by {owner}; refused until then");
    ExecResult::err(format!("{verb}: rebuilt on the one writer by {owner}; not available yet"))
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
        assert!(command_by_name("AnChOr").is_some());
        assert_eq!(command_by_name("?").map(|c| c.name), Some("help"));
        assert_eq!(command_by_name("visibility").map(|c| c.name), Some("zoom"));
        assert!(command_by_name("nope").is_none());

        for name in [
            "help", "anchor", "body", "cap", "color", "font", "fps", "spacing", "label", "save", "open", "new",
            "zoom",
        ] {
            assert!(
                command_by_name(name).is_some(),
                "console verb '{name}' missing from registry"
            );
        }
    }

    /// A verb an owner rebuilds refuses with the owner named, whatever
    /// the line, takes no write on the document, and stays registered
    /// with its grammar for help and completion.
    #[test]
    fn test_a_rebuilt_verb_refuses_naming_its_owner() {
        for (verb, owner) in [
            ("node", FIELD_NODES_OWNER),
            ("section", FIELD_NODES_OWNER),
            ("color", FIELD_NODES_OWNER),
            ("font", FIELD_NODES_OWNER),
            ("zoom", FIELD_NODES_OWNER),
            ("anchor", FIELD_PATHS_OWNER),
            ("body", FIELD_PATHS_OWNER),
            ("cap", FIELD_PATHS_OWNER),
            ("label", FIELD_PATHS_OWNER),
            ("spacing", FIELD_PATHS_OWNER),
        ] {
            let command = command_by_name(verb).expect("registered");
            let mut doc = crate::application::document::MindMapDocument::new_blank(None, 50);
            let revision = doc.revision();
            let mut eff = ConsoleEffects::new(&mut doc);
            let line = format!("{verb} x=1");
            let crate::application::console::ParseResult::Ok { args, .. } =
                crate::application::console::parse(&line)
            else {
                panic!("{line} parses");
            };
            let args = crate::application::console::parser::Args::new(&args);
            match (command.execute)(&args, &mut eff) {
                ExecResult::Err(message) => assert!(message.contains(owner), "{verb}: {message}"),
                other => panic!("{verb} must refuse, ran: {other:?}"),
            }
            drop(eff);
            assert_eq!(doc.revision(), revision, "{verb} must not change the document");
            assert!(!command.usage_forms().is_empty(), "{verb} keeps its grammar");
        }
    }
}
