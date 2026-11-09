// SPDX-License-Identifier: MPL-2.0

//! `theme` — the map's theme, on the one writer (V1-C5, #246; D21):
//!
//! - `theme variant <name>` makes `name` the active variant
//!   (`ThemeOp::SetActive`): every element that reads a theme variable
//!   is drawn in that variant's colors;
//! - `theme set <var>=<color> … [variant=<name>]` sets each variable's
//!   value in a variant, the active one when none is named
//!   (`ThemeOp::SetValue`), a literal `#rrggbb[aa]` as the file holds it.
//!
//! A variable's name is the key of `theme set`'s pairs, so they are the
//! map's own names: the engine's kv reading, which knows only declared
//! keys, reads `variant=` alone, and the verb reads the rest.

use super::Command;
use crate::application::console::completion::{prefix_filter, Completion};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::Stop;
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{usage, Form, Grammar, Key, Slot, Subverb, Vocabulary};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::fields::map;

/// The theme's variants, by name.
fn variant_rows(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    let names: Vec<&str> = ctx
        .document
        .doc()
        .theme()
        .variants
        .iter()
        .map(|variant| variant.name.as_str())
        .collect();
    prefix_filter(&names, partial)
}

/// The theme's variables, each offered as `<name>=` to type a color
/// after.
fn variable_rows(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    ctx.document
        .doc()
        .theme()
        .variables
        .iter()
        .filter(|name| name.starts_with(partial))
        .map(|name| Completion {
            text: format!("{name}="),
            display: name.clone(),
            hint: None,
            font_family: None,
        })
        .collect()
}

const VARIANT: Vocabulary = Vocabulary::Rows {
    placeholder: "name",
    rows: variant_rows,
    sentinels: &[],
};

const KEYS: &[Key] = &[Key::new(
    "variant",
    "the variant to set; the active one by default",
    VARIANT,
)];

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("variant", "theme", "make a variant the active one")
        .taking(&[Form::slots(&[Slot::req(VARIANT)])]),
    Subverb::bare("set", "theme", "set variables' colors in a variant").taking(&[Form::slots(&[Slot::req(
        Vocabulary::Rows {
            placeholder: "var>=<color",
            rows: variable_rows,
            sentinels: &[],
        },
    )])
    .reading(&["variant"])]),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "theme",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS],
    bare: None,
};

/// Which handler each declared subverb runs (`spec::route`).
#[derive(Clone, Copy)]
enum Route {
    Variant,
    Set,
}

const ROUTES: &Routes<Route> = &[("variant", Route::Variant), ("set", Route::Set)];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub const COMMAND: Command = Command {
    name: "theme",
    aliases: &[],
    summary: "Switch the theme's variant, or set its variables' colors",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["palette", "dark", "light", "variable", "var"],
    execute: execute_theme,
};

fn execute_theme(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = match super::descend(&GRAMMAR, args) {
        Ok(descent) => descent,
        Err(refused) => return refused,
    };
    let Stop::Matched(subverb) = descent.stop else {
        return ExecResult::err(usage::no_arguments_message(&GRAMMAR));
    };
    let slots = descent.slot_value(args);
    match route::lookup(ROUTES, subverb) {
        Some(Route::Variant) => {
            if let Err(msg) = crate::application::console::spec::kvs::read_strict(&descent, args) {
                return ExecResult::err(msg);
            }
            let Some(name) = slots.get(0) else {
                return ExecResult::err(usage::subverb_usage(descent.level, subverb));
            };
            let steps = map::activate(eff.document(), name);
            super::edit(eff, steps, "theme variant")
        }
        Some(Route::Set) => {
            if let Some(extra) = slots.get(0) {
                return ExecResult::err(usage::extra_positional_message(descent.level, "theme set", extra));
            }
            let mut variant = None;
            let mut pairs = Vec::new();
            for (key, value) in args.kvs() {
                match key {
                    "variant" => variant = Some(value),
                    var => pairs.push((var, value)),
                }
            }
            let steps = map::set_theme(eff.document(), &pairs, variant);
            super::edit(eff, steps, "theme set")
        }
        None => ExecResult::err(route::miss(descent.level, subverb)),
    }
}
