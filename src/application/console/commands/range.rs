// SPDX-License-Identifier: MPL-2.0

//! `range <start> <end> color=… font=… size=… weight=… italic=…` —
//! style grapheme clusters `start..end` of the selected section's text,
//! on the one writer (V1-C5, #246): the spans are split at both ends,
//! the ones inside given the style, and the section normalized
//! (`compose::style_range`), all as one request. It replaced the
//! `section=` / `range=` keys `color` and `font` took on `main`.
//!
//! The section is the selected one, or a selected node's one section
//! (`crate::application::fields::section::targets`). Indices count
//! grapheme clusters, so an emoji of five code points is one.

use baumhard::document::mutation::ops::TextOp;
use baumhard::document::values::FontSize;

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::node_or_section_selected;
use crate::application::console::spec::kvs;
use crate::application::console::spec::{free, usage, Bare, Form, Grammar, Key, Slot};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::fields::{section, values};

const KEYS: &[Key] = &[
    Key::new(
        "color",
        "text color, as a map spells one, or `reset`",
        free("color"),
    ),
    Key::new("font", "font family a map can name, or `reset`", free("family")),
    Key::new("size", "font size in points, or `reset`", free("pt")),
    Key::new("weight", "normal, bold or 100…900, or `reset`", free("weight")),
    Key::new("italic", "true or false, or `reset`", free("bool")),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "range",
    subverb_sets: &[],
    key_sets: &[KEYS],
    bare: Some(Bare::new(
        "style",
        &[Form::slots(&[Slot::req(free("start")), Slot::req(free("end"))])
            .reading(&["color", "font", "size", "weight", "italic"])],
    )),
};

pub const COMMAND: Command = Command {
    name: "range",
    aliases: &[],
    summary: "Style part of the selected section's text, by grapheme",
    applicable: node_or_section_selected,
    grammar: &GRAMMAR,
    synonyms: &["span", "style", "bold", "italic", "grapheme"],
    execute: execute_range,
};

fn execute_range(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = match super::descend(&GRAMMAR, args) {
        Ok(descent) => descent,
        Err(refused) => return refused,
    };
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    let slots = descent.slot_value(args);
    let (Some(start), Some(end)) = (slots.get(0), slots.get(1)) else {
        return ExecResult::err(usage::no_arguments_message(&GRAMMAR));
    };
    let doc = eff.document();
    let ops = pairs
        .iter()
        .filter_map(|pair| {
            let value = pair.value;
            let op = match pair.key.name {
                "color" => values::color(doc.doc(), value).map(TextOp::SetColor),
                "font" => values::font(value).map(TextOp::SetFont),
                "size" => {
                    values::or_reset(value, values::number).map(|v| TextOp::SetSize(v.map(FontSize::Pt)))
                }
                "weight" => values::or_reset(value, values::weight).map(TextOp::SetWeight),
                "italic" => values::or_reset(value, values::boolean).map(TextOp::SetItalic),
                other => {
                    log::error!("range: engine admitted unread key '{other}'");
                    return None;
                }
            };
            Some(op)
        })
        .collect::<Result<Vec<TextOp>, String>>();
    let steps = ops.and_then(|ops| {
        if ops.is_empty() {
            return Err("range: give a style: color=, font=, size=, weight= or italic=".to_string());
        }
        section::style_range(doc, values::index(start)?, values::index(end)?, ops)
    });
    super::edit(eff, steps, "range")
}
