// SPDX-License-Identifier: MPL-2.0

//! `font family=… size=… weight=… italic=…` — the text style of every
//! selected element, on the one writer (V1-C5, #246): one line is one
//! request, so one undo entry (`crate::application::fields::style`).
//! A selected range of a section's text takes them on the spans in it.
//! `main`'s screen-space `min=` / `max=` clamps are gone with D23.
//!
//! `family=` takes exactly the names a map's `font` key takes
//! (`format::font`): a family this build compiles in. An installed host
//! font is listed by `font list` but cannot be named by a map yet
//! (#329), so it is refused here as the loader refuses it. `reset` on
//! any key clears the element's own, so it inherits.
//!
//! `font list` prints every loaded family, each in its own face.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::Stop;
use crate::application::console::spec::kvs;
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{usage, Bare, Form, Grammar, Key, Subverb, Vocabulary, Word};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult, OutputLine};
use crate::application::fields::style::{self, Font};
use crate::application::fields::values;

/// Point sizes the popup suggests. Any positive float is legal, so
/// these are *suggestions* rather than a vocabulary — hence
/// [`Vocabulary::Rows`], whose usage line prints `<pt>` and whose
/// popup prints the list.
pub const SIZE_PRESETS: &[&str] = &["10", "12", "14", "16", "18", "24", "32"];

const RESET: &[Word] = &[Word::new(values::RESET, "inherit")];

fn size_rows(_ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    prefix_filter(SIZE_PRESETS, partial)
}

fn family_rows(_ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    font_family_completions(partial)
}

const WEIGHTS: &[Word] = &[
    Word::bare("normal"),
    Word::bare("bold"),
    Word::new(values::RESET, "inherit"),
];
const BOOLS: &[Word] = &[
    Word::bare("true"),
    Word::bare("false"),
    Word::new(values::RESET, "inherit"),
];

const KEYS: &[Key] = &[
    Key::new(
        "family",
        "a font family a map can name",
        Vocabulary::Rows {
            placeholder: "family",
            rows: family_rows,
            sentinels: RESET,
        },
    ),
    Key::new(
        "size",
        "size in points",
        Vocabulary::Rows {
            placeholder: "pt",
            rows: size_rows,
            sentinels: RESET,
        },
    ),
    Key::new("weight", "normal, bold or 100…900", Vocabulary::Words(WEIGHTS)),
    Key::new("italic", "slanted or upright", Vocabulary::Words(BOOLS)),
];

const SUBVERBS: &[Subverb] = &[Subverb::bare(
    "list",
    "family",
    "list every loaded font, each rendered in its face",
)];

pub static GRAMMAR: Grammar = Grammar {
    label: "font",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS],
    bare: Some(Bare::new(
        "composed",
        &[Form::opt(&["family", "size", "weight", "italic"])],
    )),
};

/// Which handler each declared subverb runs (`spec::route`).
#[derive(Clone, Copy)]
enum Route {
    List,
}

const ROUTES: &Routes<Route> = &[("list", Route::List)];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub const COMMAND: Command = Command {
    name: "font",
    aliases: &[],
    summary: "Set the font family, size, weight and slant of the selection, or list fonts",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["bold", "italic", "pt", "smaller", "larger", "face"],
    execute: execute_font,
};

/// Build font-family completions: one entry per family a map can name
/// (`format::font`) whose name starts with `partial`
/// (case-insensitive). Each entry carries `font_family = Some(<name>)`
/// so the popup row shapes the candidate label in that very face.
///
/// Family names that contain whitespace are returned with the
/// inserted `text` wrapped in double quotes — the tokenizer would
/// otherwise split `Norse Bold` into two tokens, and the user would
/// hit "not a font a map can name" on the first chunk. Quoting at the
/// completion source means tab-accept always produces a parseable
/// command. The `display` row stays unquoted so the popup reads
/// naturally.
///
/// `partial` arrives already-unquoted: the tokenizer
/// (`parser::tokenize`) drops the leading `"` on an unterminated
/// quoted token, so a user mid-typing `font family="Nor` lands here
/// with `partial = "Nor"`, not `"\"Nor"`. No leading-quote
/// stripping is needed.
///
/// This is the console's *only* font-family vocabulary. A second copy
/// once omitted the quoting above, so tab-accepting any of the
/// whitespace-bearing families produced a line the verb then refused.
/// One body, so two surfaces cannot disagree about it again.
pub(crate) fn font_family_completions(partial: &str) -> Vec<Completion> {
    let partial_lc = partial.to_ascii_lowercase();
    baumhard::font::fonts::loaded_families_iter()
        .filter(|f| f.to_ascii_lowercase().starts_with(&partial_lc))
        .filter(|f| baumhard::document::format::font(f).is_some())
        .map(|family| {
            let needs_quoting = family.chars().any(char::is_whitespace);
            let text = if needs_quoting {
                format!("\"{}\"", family)
            } else {
                family.to_string()
            };
            Completion {
                text,
                display: family.to_string(),
                hint: None,
                font_family: Some(family.to_string()),
            }
        })
        .collect()
}

fn execute_font(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = match super::descend(&GRAMMAR, args) {
        Ok(descent) => descent,
        Err(refused) => return refused,
    };
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    if let Stop::Matched(subverb) = descent.stop {
        return match route::lookup(ROUTES, subverb) {
            Some(Route::List) => {
                let lines = baumhard::font::fonts::loaded_families_iter().map(|family| OutputLine {
                    text: family.to_string(),
                    font_family: Some(family.to_string()),
                });
                ExecResult::Lines(lines.collect())
            }
            None => ExecResult::err(route::miss(descent.level, subverb)),
        };
    }
    if pairs.is_empty() {
        return ExecResult::err(usage::no_arguments_message(&GRAMMAR));
    }
    let font = pairs.iter().try_fold(Font::default(), |mut font, pair| {
        let value = pair.value;
        match pair.key.name {
            "family" => font.family = Some(values::font(value)?),
            "size" => font.size = Some(values::or_reset(value, values::number)?),
            "weight" => font.weight = Some(values::or_reset(value, values::weight)?),
            "italic" => font.italic = Some(values::or_reset(value, values::boolean)?),
            other => log::error!("font: engine admitted unread key '{other}'"),
        }
        Ok::<_, String>(font)
    });
    let steps = font.and_then(|font| style::font(eff.document(), font));
    super::edit(eff, steps, "font")
}
