// SPDX-License-Identifier: MPL-2.0

//! Console command registry.
//!
//! Each command lives in its own submodule so the surface stays
//! scannable. The public `COMMANDS` slice gathers them in one place,
//! matching the `const PALETTE_ACTIONS` pattern — zero-cost startup,
//! no HashMap construction, and `action_by_id`-style lookup is a
//! linear scan over a dozen entries.
//!
//! # Casing
//!
//! One rule, and it is the one every completer already assumes:
//!
//! - **Command names, aliases and positional subverbs are matched
//!   case-insensitively.** `mode DEFAULT`, `font SET Norse` and
//!   `border PRESET heavy` all run. Every subverb popup filters
//!   its partial case-insensitively and inserts the canonical
//!   spelling, so this is what the popup has always promised; five
//!   verbs honored it and the rest did not, which meant a word the
//!   popup listed could still be refused when typed out in full.
//! - **Kv *keys* are exact.** `border TOP=x` is `unknown key
//!   'TOP'`, and `kv_key_completions` filters case-sensitively to
//!   say so. A key is a field name, not a word the user picks.
//! - **Kv *values* belong to the key's own parser.** Most are
//!   case-insensitive (`preset=HEAVY`, `color=ACCENT`,
//!   `side=TOP`); a palette name is stored verbatim and compared
//!   as written. Each value completer matches the way its parser
//!   does, and the oracle corpus pins the pair.
//!
//! What makes the first bullet checkable rather than aspirational
//! is `console::tests::oracle_corpus`: every verb whose subverb
//! dispatch normalizes carries an upper-case corpus row beside its
//! lower-case one. (Named rather than linked — the module is
//! `#[cfg(test)]`, so rustdoc cannot see it even under
//! `--document-private-items`, and an intra-doc link to it fails
//! the `-D warnings` doc gate.)
//!
//! # Usage and tags are hand-written
//!
//! [`Command::usage`] and [`Command::tags`] are `&'static str`
//! literals and `help` prints them verbatim (`help.rs::help_for`).
//! Nothing derives them from the verb's own `KEYS` list, so the
//! three declarations can disagree: a key added to `KEYS` is
//! offered by the popup on the next keystroke and stays absent
//! from `help <verb>` until somebody writes it in by hand. `font`
//! and `color` each carried that drift for `range=` — parseable,
//! named in the verb's own rejection, and documented nowhere.
//!
//! Those two verbs now assert that every key in their `KEYS`
//! appears in both literals. That closes the two *instances* and
//! deliberately not the *mechanism*: the assertion is itself a
//! per-verb copy, so the nine other `KEYS`-bearing verbs have
//! neither the check nor any reason they would not need it.
//!
//! What holds the generalization up is not reach. `KEYS` is a free
//! const per module with no field on [`Command`] to read it
//! through, but [`Command::complete`] *is* a field, and driving
//! each registry entry's own completer at a kv-key slot and
//! collecting the rows it emits ending in `=` recovers the same
//! vocabulary for all eleven — no new field, and no need to parse
//! anything. (`baumhard::util::source_scan` is the `syn`-backed
//! machinery this class of repository check already uses, and its
//! `RUST_ROOTS` covers `src`, so the source-reading route is open
//! too.)
//!
//! What holds it up is that the answer such a walk returns is not
//! yet a rule. It reports `canvas` offering `top= bottom= left=
//! right= tl= tr= bl= br=` and `section` offering `font= color=
//! palette= field= padding= top= …`, and neither verb names one of
//! those in its `usage` or `tags` — correctly, because neither
//! owns them: both borrow the whole `border` keyset and say so as
//! `<key>=<value>`, pointing at the vocabulary documented under
//! `border` rather than transcribing it. A check that fails those
//! two is wrong; a check that exempts them by name is a list
//! rather than a rule. The missing piece is therefore a per-verb
//! policy — spells its keys out, versus delegates to a documented
//! keyset — and that decision belongs with the declarative-grammar
//! work #27 tracks, where `usage` stops being hand-written at all.
//! Until then: a key added to any verb's `KEYS` is added to that
//! verb's `usage` and `tags` by hand, in the same edit.

use super::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::console::completion::{Completion, CompletionState};
use crate::application::console::parser::Args;

pub mod anchor;
pub mod body;
pub mod border;
pub mod canvas;
pub mod cap;
pub mod color;
pub mod edge;
pub mod font;
pub mod fps;
pub mod help;
pub mod label;
pub mod mode;
pub mod mutation;
pub mod new;
pub mod node;
pub mod open;
pub mod range_kv;
pub mod save;
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
    /// Full usage line shown in `help <cmd>`. Conventionally starts
    /// with the command name: `"anchor set <from|to> <side>"`.
    pub usage: &'static str,
    /// Extra search tokens printed by `help <cmd>` so a user
    /// grepping the command list can find "pick" under `color`
    /// even though the name doesn't include it.
    pub tags: &'static [&'static str],
    /// Returns `true` when the command should appear in the filtered
    /// `help` list and in completion. Commands whose args are
    /// context-specific but whose verb is always meaningful should
    /// return `true` here and validate in `execute`.
    pub applicable: fn(&ConsoleContext) -> bool,
    /// Build completion candidates for the token currently under the
    /// cursor. Return an empty `Vec` when the command can't offer
    /// any useful completion for that position.
    pub complete: fn(&CompletionState, &ConsoleContext) -> Vec<Completion>,
    /// Run the command. The dispatcher clears the scene cache and
    /// rebuilds after every non-`Err` result.
    pub execute: fn(&Args, &mut ConsoleEffects) -> ExecResult,
}

/// The global command registry. Order matters only for `help` — the
/// listing iterates this slice in declaration order.
pub const COMMANDS: &[Command] = &[
    help::COMMAND,
    anchor::COMMAND,
    body::COMMAND,
    border::COMMAND,
    canvas::COMMAND,
    cap::COMMAND,
    color::COMMAND,
    edge::COMMAND,
    font::COMMAND,
    fps::COMMAND,
    spacing::COMMAND,
    label::COMMAND,
    mode::COMMAND,
    mutation::COMMAND,
    save::COMMAND,
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

/// Single-source success-or-no-op message for verbs that aggregate
/// across a set of selection targets. `verb` is the noun the user
/// typed (`"font"`, `"zoom"`, `"color"`, …); `kind` is the
/// selection scope (`"node"`, `"section"`, `"edge"`, …); `changed`
/// is whether at least one target actually mutated.
///
/// Two formats: `"<verb> applied to <kind>"` on change, `"<verb>:
/// no change on <kind>"` on no-op. Mirrors the previous open-coded
/// `finalize` helpers in `commands/font.rs` and `commands/zoom.rs`.
pub(super) fn applied_or_no_change(verb: &str, kind: &str, changed: bool) -> ExecResult {
    if changed {
        ExecResult::ok_msg(format!("{verb} applied to {kind}"))
    } else {
        ExecResult::ok_msg(format!("{verb}: no change on {kind}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lookup is case-insensitive across both names and aliases,
    /// returns `None` on unknown, and resolves aliases to their
    /// canonical entry. Also pins the registered verb names — the
    /// compiler enforces that the `COMMANDS` slice exists, but a
    /// typo in `name: "border"` → `"boder"` would compile and
    /// silently break user-facing console input without this list.
    #[test]
    fn test_command_by_name_lookup() {
        assert!(command_by_name("HELP").is_some());
        assert!(command_by_name("AnChOr").is_some());
        assert_eq!(command_by_name("?").map(|c| c.name), Some("help"));
        assert_eq!(command_by_name("visibility").map(|c| c.name), Some("zoom"));
        assert!(command_by_name("nope").is_none());

        for name in [
            "help", "anchor", "body", "border", "cap", "color", "edge", "font", "fps", "spacing", "label",
            "mutation", "save", "open", "new", "zoom",
        ] {
            assert!(
                command_by_name(name).is_some(),
                "console verb '{name}' missing from registry"
            );
        }
    }
}
