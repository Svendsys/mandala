// SPDX-License-Identifier: MPL-2.0

//! `color text=… border=… ring=…` — the colors of every selected
//! element, whatever its kind, on the one writer (V1-C5, #246): one line
//! is one request, so one undo entry. `text` is a node's, section's or
//! label's text and a path's, cap's or portal end's one color; `border`
//! is a node's frame, which its rings inherit (and a path's color, as on
//! `main`); `ring` is each ring's own (`crate::application::fields::style`).
//! A node draws no fill (D12), so `main`'s `bg` is gone.
//!
//! A color is spelled as the file spells one — `#rrggbb`, `#rrggbbaa`,
//! `var(name)` of the map's theme, `palette(frame|text|title)` — or
//! `reset`, which clears the element's own so it inherits. A selected
//! range of a section's text takes `text` on the spans in it; any other
//! part of a section's text is the `range` verb's.
//!
//! The axis subverbs open the glyph-wheel picker on the selection:
//! `color text` and `color border` on that axis, `color pick` on the text
//! axis (a node draws no fill, D12, so `main`'s default of its fill is its
//! text). The contextual picker is a transaction (V1-C7,
//! `app::color_picker_flow::session`): what it shows is staged on the
//! selection, its ࿕ commits one entry, and Esc leaves the map as it was.
//! `color picker on|off` opens and closes the standalone palette, which
//! holds no transaction.

use super::Command;
use crate::application::console::completion::Completion;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::Stop;
use crate::application::console::spec::kvs;
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{usage, Bare, Form, Grammar, Key, Slot, Subverb, Vocabulary, Word};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::fields::{style, values};
use crate::application::keybinds::ColorAxis;

/// The colors a map's own spelling offers: each theme variable as
/// `var(name)` and the three palette fields; `reset` is the sentinel.
fn color_rows(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    let vars = ctx
        .document
        .doc()
        .theme()
        .variables
        .iter()
        .map(|name| format!("var({name})"));
    let fields = ["palette(frame)", "palette(text)", "palette(title)"].map(String::from);
    vars.chain(fields)
        .filter(|row| row.starts_with(partial))
        .map(|row| Completion {
            text: row.clone(),
            display: row,
            hint: None,
            sample_face: None,
        })
        .collect()
}

/// A color value: `#rrggbb[aa]`, a theme variable, a palette field, or
/// `reset`.
pub(crate) const COLOR_VOCAB: Vocabulary = Vocabulary::Rows {
    placeholder: "#rrggbb[aa]|var(name)|palette(field)",
    rows: color_rows,
    sentinels: &[Word::new(values::RESET, "inherit")],
};

const ON_OFF: &[Word] = &[
    Word::new("on", "open the picker as a persistent standalone palette"),
    Word::new("off", "close any open picker"),
];

const KEYS: &[Key] = &[
    Key::new(
        "text",
        "text color; a path's, cap's or portal end's color",
        COLOR_VOCAB,
    ),
    Key::new(
        "border",
        "a node's frame color, which its rings inherit",
        COLOR_VOCAB,
    ),
    Key::new("ring", "each ring's own color", COLOR_VOCAB),
];

/// The picker subverbs, V1-C7's: `color text` opens the glyph wheel on
/// that axis, `color text=#fff` writes it directly. The two
/// vocabularies are separate namespaces, so one word can be in both.
const SUBVERBS: &[Subverb] = &[
    Subverb::bare("text", "picker", "pick a text color on the glyph wheel").gated(),
    Subverb::bare("border", "picker", "pick a frame color on the glyph wheel").gated(),
    Subverb::bare("pick", "picker", "pick a color for the current selection").gated(),
    Subverb::bare("picker", "picker", "open or close the standalone palette")
        .taking(&[Form::slots(&[Slot::req(Vocabulary::Words(ON_OFF))])])
        .gated(),
];

/// Which handler each declared subverb runs (`spec::route`): the
/// contextual picker on an axis, or the standalone palette.
#[derive(Clone, Copy)]
enum Route {
    Pick(ColorAxis),
    Palette,
}

const ROUTES: &Routes<Route> = &[
    ("text", Route::Pick(ColorAxis::Text)),
    ("border", Route::Pick(ColorAxis::Border)),
    ("pick", Route::Pick(ColorAxis::Text)),
    ("picker", Route::Palette),
];

/// What a picker subverb asks of the app, which owns the picker
/// (`app::color_picker_flow::session`): its report is the line's output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerLine {
    /// Open the contextual picker on the selection's color `axis`.
    Open(ColorAxis),
    /// Open the standalone palette (`color picker on`).
    Palette,
    /// Close whatever picker is open (`color picker off`).
    Close,
}

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub static GRAMMAR: Grammar = Grammar {
    label: "color",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS],
    bare: Some(Bare::new("composed", &[Form::opt(&["text", "border", "ring"])])),
};

pub const COMMAND: Command = Command {
    name: "color",
    aliases: &[],
    summary: "Set the text, frame or ring color of every selected element",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["wheel", "colour"],
    execute: execute_color,
};

fn execute_color(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = match super::descend(&GRAMMAR, args) {
        Ok(descent) => descent,
        Err(refused) => return refused,
    };
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    if let Stop::Matched(subverb) = descent.stop {
        let line = match route::lookup(ROUTES, subverb) {
            Some(Route::Pick(axis)) => PickerLine::Open(axis),
            Some(Route::Palette) => match descent.slot_value(args).get(0).map(str::to_ascii_lowercase).as_deref() {
                Some("on") => PickerLine::Palette,
                Some("off") => PickerLine::Close,
                _ => return ExecResult::err(usage::subverb_usage(descent.level, subverb)),
            },
            None => return ExecResult::err(route::miss(descent.level, subverb)),
        };
        eff.side_effect = Some(crate::application::console::ConsoleSideEffect::Picker(line));
        eff.close_console = true;
        return ExecResult::ok_empty();
    }
    if pairs.is_empty() {
        return ExecResult::err(usage::no_arguments_message(&GRAMMAR));
    }
    let doc = eff.document();
    let mut steps: crate::application::fields::Steps = Ok(Vec::new());
    for pair in &pairs {
        let axis: ColorAxis = match pair.key.name.parse() {
            Ok(axis) => axis,
            Err(_) => return ExecResult::err(format!("color: unknown key '{}'", pair.key.name)),
        };
        steps = steps.and_then(|mut steps: Vec<_>| {
            let color = values::color(doc.doc(), pair.value)?;
            steps.extend(style::color(doc, axis, color)?);
            Ok(steps)
        });
    }
    super::edit(eff, steps, "color")
}
