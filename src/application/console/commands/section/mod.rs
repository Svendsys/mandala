// SPDX-License-Identifier: MPL-2.0

//! `section` — a section's place, size, alignment and text, and adding,
//! deleting and splitting sections, on the one writer (V1-C5, #246):
//! one line is one request, so one undo entry.
//!
//! A line reaches the selected sections, or a selected node's section
//! `section=<idx>`, or its one section when it has one
//! (`crate::application::fields::section::targets`). `section text`
//! replaces a section's spans with one span; styled text is edited by
//! the text editor and the `range` verb. `section edit` opens the text
//! editor, which V1-C8 (#249) rebuilds; until then it refuses, naming
//! that issue.

pub(crate) mod grammar;

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::node_or_section_selected;
use crate::application::console::spec::descent::Stop;
use crate::application::console::spec::kvs::{self, Pair};
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::usage;
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::fields::section::{self, Field};
use crate::application::fields::values;
use baumhard::document::element::DocElement;
use glam::Vec2;

/// Which handler each declared subverb runs (`spec::route`).
#[derive(Clone, Copy)]
enum Route {
    Move,
    Resize,
    Align,
    Show,
    Text,
    Edit,
    Add,
    Delete,
    Split,
}

const ROUTES: &Routes<Route> = &[
    ("move", Route::Move),
    ("resize", Route::Resize),
    ("align", Route::Align),
    ("show", Route::Show),
    ("text", Route::Text),
    ("edit", Route::Edit),
    ("add", Route::Add),
    ("delete", Route::Delete),
    ("split", Route::Split),
];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub const COMMAND: Command = Command {
    name: "section",
    aliases: &[],
    summary: "Move, resize, align, set the text of, inspect, add, delete or split a section",
    applicable: node_or_section_selected,
    grammar: &grammar::SECTION,
    // Every subverb and key is derived. `info`, `offset` and `size` are
    // the words a user greps for that the grammar does not contain.
    synonyms: &["info", "offset", "size"],
    execute: execute_section,
};

fn execute_section(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = match super::descend(&grammar::SECTION, args) {
        Ok(descent) => descent,
        Err(refused) => return refused,
    };
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    let Stop::Matched(subverb) = descent.stop else {
        return ExecResult::err(usage::no_arguments_message(&grammar::SECTION));
    };
    let index = match kvs::value(&pairs, "section").map(values::index).transpose() {
        Ok(index) => index,
        Err(msg) => return ExecResult::err(msg),
    };
    let doc = eff.document();
    let (steps, label) = match route::lookup(ROUTES, subverb) {
        Some(Route::Move) => (
            moved(&pairs).and_then(|field| section::set(doc, index, field)),
            "section move",
        ),
        Some(Route::Resize) => {
            let size = match descent.slot_value(args).get(0) {
                Some(_) => Ok(None),
                None => xy(&pairs, "w", "h").map(Some),
            };
            (
                size.and_then(|size| section::set(doc, index, Field::MinSize(size))),
                "section resize",
            )
        }
        Some(Route::Align) => {
            let word = descent.slot_value(args).get(0).unwrap_or_default();
            let align = values::or_reset(word, values::align);
            (
                align.and_then(|align| section::set(doc, index, Field::Align(align))),
                "section align",
            )
        }
        Some(Route::Show) => return show(eff, index),
        Some(Route::Text) => {
            let text = descent
                .slot_value(args)
                .get(0)
                .or_else(|| kvs::value(&pairs, "text"))
                .unwrap_or_default();
            (section::text(doc, index, text), "section text")
        }
        Some(Route::Edit) => return super::rebuilt_by("section edit", super::TEXT_OWNER),
        Some(Route::Add) => {
            let at = kvs::value(&pairs, "at").map(values::index).transpose();
            let text = kvs::value(&pairs, "text").unwrap_or_default();
            (at.and_then(|at| section::add(doc, at, text)), "section add")
        }
        Some(Route::Delete) => (section::delete(doc, index), "section delete"),
        Some(Route::Split) => {
            let at = kvs::value(&pairs, "at")
                .map(values::index)
                .unwrap_or(Err("section split: give at=<grapheme>".into()));
            (at.and_then(|at| section::split(doc, index, at)), "section split")
        }
        None => return ExecResult::err(route::miss(descent.level, subverb)),
    };
    super::edit(eff, steps, label)
}

/// The two numbers `x_key` and `y_key` give, both required.
fn xy(pairs: &[Pair], x_key: &str, y_key: &str) -> Result<Vec2, String> {
    match (kvs::value(pairs, x_key), kvs::value(pairs, y_key)) {
        (Some(x), Some(y)) => Ok(Vec2::new(values::number(x)?, values::number(y)?)),
        _ => Err(format!("section: give both {x_key}= and {y_key}=")),
    }
}

/// `section move`'s field: a delta (`dx=`/`dy=`) or an offset
/// (`x=`/`y=`), never a mix.
fn moved(pairs: &[Pair]) -> Result<Field, String> {
    let has = |key| kvs::value(pairs, key).is_some();
    match (has("dx") || has("dy"), has("x") || has("y")) {
        (true, false) => xy(pairs, "dx", "dy").map(Field::Nudge),
        (false, true) => xy(pairs, "x", "y").map(Field::Offset),
        _ => Err("section move: give dx= and dy= (a delta) or x= and y= (an offset), not both".to_string()),
    }
}

/// `section show`: each reached section's own fields and text, one line
/// each. Reads only.
fn show(eff: &ConsoleEffects, index: Option<usize>) -> ExecResult {
    let doc = eff.document();
    let sections = match section::targets(doc, index) {
        Ok(sections) => sections,
        Err(msg) => return ExecResult::err(msg),
    };
    let d = doc.doc();
    let lines = sections.into_iter().filter_map(|id| {
        let Some(DocElement::Section(s)) = d.get(id) else { return None };
        let size = s
            .min_size
            .map_or("fit".to_string(), |v| format!("{}x{}", v.x, v.y));
        Some(format!(
            "section: offset {},{} size {size} align {} text {:?}",
            s.offset.x,
            s.offset.y,
            s.align
                .map_or("inherited".to_string(), |a| format!("{a:?}").to_lowercase()),
            crate::application::view::inspect::element_text(d, id)
        ))
    });
    ExecResult::lines(lines.collect::<Vec<_>>())
}
