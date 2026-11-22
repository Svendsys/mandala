// SPDX-License-Identifier: MPL-2.0

//! `font family=… size=… weight=… italic=…` — the text style of every
//! selected element, on the one writer (V1-C5, #246): one line is one
//! request, so one undo entry (`crate::application::fields::style`).
//! A selected range of a section's text takes them on the spans in it.
//! `main`'s screen-space `min=` / `max=` clamps are gone with D23.
//!
//! `family=` takes exactly the names a map's `font` key takes
//! (`format::font`): a family this build compiles in, or an installed
//! one, whether or not this machine has it (#329). `reset` on any key
//! clears the element's own, so it inherits.
//!
//! `font list` prints every loaded family, each in its own face, an
//! installed one marked `(installed)`; the completion of `family=` marks
//! it the same way.
//!
//! `font replace <family> <font>` rewrites every reference the map makes
//! to the installed family `<family>` to `<font>`, as one undoable edit
//! (`missing_fonts::replace`, the `ReplaceFontFamily` Action's body);
//! `font rescan` looks the host up again for the families the map lacks
//! (`missing_fonts::rescan`), once the user has installed one.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::Stop;
use crate::application::console::spec::kvs;
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{usage, Bare, Form, Grammar, Key, Slot, Subverb, Vocabulary, Word};
use crate::application::console::{
    ConsoleContext, ConsoleEffects, ConsoleSideEffect, ExecResult, OutputLine,
};
use crate::application::fields::style::{self, Font};
use crate::application::fields::values;
use crate::application::missing_fonts;

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

fn named_family_rows(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    named_family_completions(ctx, partial)
}

const SUBVERBS: &[Subverb] = &[
    Subverb::bare(
        "list",
        "family",
        "list every loaded font, each rendered in its face",
    ),
    Subverb::bare(
        "replace",
        "family",
        "rewrite the map's references to an installed font, one undoable edit",
    )
    .taking(&[Form::slots(&[
        Slot::req(Vocabulary::Rows {
            placeholder: "family",
            rows: named_family_rows,
            sentinels: &[],
        }),
        Slot::req(Vocabulary::Rows {
            placeholder: "font",
            rows: family_rows,
            sentinels: &[],
        }),
    ])]),
    Subverb::bare(
        "rescan",
        "family",
        "look for the map's missing fonts again, once installed",
    ),
];

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
    Replace,
    Rescan,
}

const ROUTES: &Routes<Route> = &[
    ("list", Route::List),
    ("replace", Route::Replace),
    ("rescan", Route::Rescan),
];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub const COMMAND: Command = Command {
    name: "font",
    aliases: &[],
    summary: "Set the font family, size, weight and slant of the selection; list, replace or rescan fonts",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["bold", "italic", "pt", "smaller", "larger", "face"],
    execute: execute_font,
};

/// Build font-family completions: one entry per loaded family a map can
/// name (`format::font`) whose name starts with `partial`
/// (case-insensitive) — the compiled ones, and on native the installed
/// ones, hinted `installed` (#329). Each entry carries
/// `font_family = Some(<name>)` so the popup row shapes the candidate
/// label in that very face — where the face maps every character of the
/// name ([`row_face`]); a symbol face that lacks them, and an installed
/// family, are drawn in the console's default face.
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
    use baumhard::font::fonts::FamilySource;
    let partial_lc = partial.to_ascii_lowercase();
    baumhard::font::fonts::loaded_families()
        .iter()
        .filter(|f| f.name.to_ascii_lowercase().starts_with(&partial_lc))
        .filter(|f| baumhard::document::format::font(&f.name).is_ok())
        .map(|family| {
            let hint = (family.source == FamilySource::Installed).then(|| INSTALLED.to_string());
            family_completion(&family.name, hint, true)
        })
        .collect()
}

/// The face a row naming the family `name`, and drawing `drawn` — the
/// name, quoted where the completion inserts it quoted — is drawn in:
/// its own, where a compiled face of it maps every character of `drawn`
/// (its character map, `compiled_family_maps`), so the row shows the
/// face; else none — the console's default face — so a face that lacks
/// a letter of its own name, or the quote around it, draws no `.notdef`
/// boxes, and an installed family, which the console cannot pin, is
/// drawn as it would be anyway. Cost: one character-map read per face of
/// the family.
pub(crate) fn row_face(name: &str, drawn: &str) -> Option<String> {
    baumhard::font::fonts::compiled_family_maps(name, drawn).then(|| name.to_string())
}

/// What `font list` and the completion mark an installed family with.
const INSTALLED: &str = "installed";

/// What the completion marks a family the map names, but this machine
/// lacks, with.
const MISSING: &str = "missing";

/// One completion row for the family `name`, quoted where it holds
/// whitespace so tab-accept produces a line the tokenizer keeps whole;
/// drawn in its own face where `own_face` asks for it and the face can
/// draw the row ([`row_face`]), else in the console's default face.
fn family_completion(name: &str, hint: Option<String>, own_face: bool) -> Completion {
    let text = if name.chars().any(char::is_whitespace) {
        format!("\"{name}\"")
    } else {
        name.to_string()
    };
    let font_family = if own_face { row_face(name, &text) } else { None };
    Completion {
        text,
        display: name.to_string(),
        hint,
        font_family,
    }
}

/// The installed families the open map names whose name starts with
/// `partial` (case-insensitive), for `font replace`'s first slot: those
/// this machine lacks hinted `missing` and drawn in the console's face,
/// the rest hinted `installed` and drawn in their own. Cost: one walk of
/// the map ([`baumhard::document::fonts::installed_families`]).
fn named_family_completions(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    let partial_lc = partial.to_ascii_lowercase();
    baumhard::document::fonts::installed_families(ctx.document.doc())
        .into_iter()
        .filter(|named| {
            named
                .family
                .as_str()
                .to_ascii_lowercase()
                .starts_with(&partial_lc)
        })
        .map(|named| {
            let name = named.family.as_str();
            if baumhard::font::fonts::installed_family_loaded(name) {
                family_completion(name, Some(INSTALLED.to_string()), true)
            } else {
                family_completion(name, Some(MISSING.to_string()), false)
            }
        })
        .collect()
}

/// `font replace <family> <with>`: the one replacement body
/// (`missing_fonts::replace`), and what is still missing after it.
fn replace(eff: &mut ConsoleEffects, family: &str, with: &str) -> ExecResult {
    let source = baumhard::document::writer::ChangeSource::Console;
    match missing_fonts::replace(eff.document_mut(), family, with, source) {
        Ok(false) => ExecResult::ok_msg(format!("font replace: the map names \"{family}\" nowhere")),
        Ok(true) => match eff.document().font_notice() {
            Some(still) => ExecResult::ok_msg(format!("font replace: \"{family}\" is now {with}; {still}")),
            None => ExecResult::ok_msg(format!(
                "font replace: \"{family}\" is now {with}; no font is missing"
            )),
        },
        Err(refused) => ExecResult::err(refused.message()),
    }
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
                let lines = baumhard::font::fonts::loaded_families()
                    .iter()
                    .map(|family| OutputLine {
                        text: match family.source {
                            baumhard::font::fonts::FamilySource::Installed => {
                                format!("{}  ({INSTALLED})", family.name)
                            }
                            baumhard::font::fonts::FamilySource::Compiled(_) => family.name.clone(),
                        },
                        font_family: row_face(&family.name, &family.name),
                    })
                    .collect();
                ExecResult::Lines(lines)
            }
            Some(Route::Replace) => {
                let (Some(family), Some(with)) = (args.positional(1), args.positional(2)) else {
                    return ExecResult::err(
                        "font replace: which font, with which? `font replace <family> <font>`",
                    );
                };
                replace(eff, family, with)
            }
            Some(Route::Rescan) => {
                eff.side_effect = Some(ConsoleSideEffect::RescanFonts);
                ExecResult::ok_empty()
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
