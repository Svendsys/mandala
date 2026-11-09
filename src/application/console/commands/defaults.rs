// SPDX-License-Identifier: MPL-2.0

//! `defaults <group>.<field>=<value> …` — the map's defaults, on the one
//! writer (V1-C5, #246; design §5.4): the defaults element is replaced
//! whole (`DefaultsOp::Replace`) with the named fields changed and every
//! other field as it was, so one line is one request and one undo entry.
//!
//! The groups and fields are the file's (`format/schema.md`
//! §"defaults"), values are read in the console's spellings
//! (`crate::application::fields::values`), and `reset` unsets a field so
//! the built-in applies. A frame's and a ring's `size` is a fraction of
//! the node's shorter side; every other `size` is in points.

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::kvs;
use crate::application::console::spec::{free, usage, Bare, Form, Grammar, Key};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::fields::map;

/// One group's keys and their names: `group.field` for each field, with
/// the hint `help defaults` prints beside it.
macro_rules! group {
    ($keys:ident, $names:ident, $group:literal: [$($field:literal => $hint:literal),* $(,)?]) => {
        const $keys: &[Key] = &[$(Key::new(concat!($group, ".", $field), $hint, free("value"))),*];
        const $names: &[&str] = &[$(concat!($group, ".", $field)),*];
    };
}

/// A group whose text style is points-sized, its fields first.
macro_rules! text_group {
    ($keys:ident, $names:ident, $group:literal $(, $field:literal => $hint:literal)*) => {
        group!($keys, $names, $group: [
            $($field => $hint,)*
            "font" => "font family a map can name",
            "size" => "font size in points",
            "color" => "text color, as a map spells one",
            "weight" => "normal, bold or 100…900",
            "italic" => "true or false",
        ]);
    };
}

text_group!(NODE, NODE_NAMES, "node", "shape" => "ellipse, rectangle or rectangle:<0-100>");
text_group!(SECTION, SECTION_NAMES, "section");
text_group!(SPAN, SPAN_NAMES, "span");
text_group!(LABEL, LABEL_NAMES, "label", "t" => "position along the path, 0 to 1", "offset" => "distance from the path");
group!(FRAME, FRAME_NAMES, "frame": [
    "visible" => "true or false",
    "start" => "top-left … left, or <edge>:<at>",
    "direction" => "cw or ccw",
    "fit" => "once[:<filler>], repeat, spread or fit[:<min>,<max>]",
    "gap" => "space between rings",
    "inset" => "first ring's distance from the outline",
    "span" => "natural, whole or 0 to 1",
    "sides" => "an ornament's text, or false",
    "spacing" => "space between a repeat ring's glyphs",
    "font" => "font family a map can name",
    "size" => "font size, a fraction of the node's shorter side",
    "color" => "text color, as a map spells one",
    "weight" => "normal, bold or 100…900",
    "italic" => "true or false",
]);
group!(RING, RING_NAMES, "ring": [
    "fit" => "once[:<filler>], repeat, spread or fit[:<min>,<max>]",
    "span" => "natural, whole or 0 to 1",
    "sides" => "an ornament's text, or false",
    "spacing" => "space between a repeat ring's glyphs",
    "font" => "font family a map can name",
    "size" => "font size, a fraction of the node's shorter side",
    "color" => "text color, as a map spells one",
    "weight" => "normal, bold or 100…900",
    "italic" => "true or false",
]);
group!(CONNECTOR, CONNECTOR_NAMES, "connector": [
    "fit" => "once[:<filler>], repeat, spread or fit[:<min>,<max>]",
    "spacing" => "space between glyphs",
    "size" => "text size in points",
    "color" => "color, as a map spells one",
    "content" => "the text drawn when it has none",
]);
group!(LINK, LINK_NAMES, "link": [
    "fit" => "once[:<filler>], repeat, spread or fit[:<min>,<max>]",
    "spacing" => "space between glyphs",
    "size" => "text size in points",
    "color" => "color, as a map spells one",
    "content" => "the text drawn when it has none",
]);
group!(PORTAL_END, PORTAL_END_NAMES, "portal_end": [
    "border_t" => "position round the outline, 0 to 1",
    "offset" => "distance from the outline",
    "color" => "marker color, as a map spells one",
    "size" => "marker size in points",
    "content" => "the marker drawn when it has none",
]);

pub static GRAMMAR: Grammar = Grammar {
    label: "defaults",
    subverb_sets: &[],
    key_sets: &[
        NODE, SECTION, FRAME, RING, CONNECTOR, LINK, LABEL, PORTAL_END, SPAN,
    ],
    bare: Some(Bare::new(
        "defaults",
        &[
            Form::opt(NODE_NAMES),
            Form::opt(SECTION_NAMES),
            Form::opt(FRAME_NAMES),
            Form::opt(RING_NAMES),
            Form::opt(CONNECTOR_NAMES),
            Form::opt(LINK_NAMES),
            Form::opt(LABEL_NAMES),
            Form::opt(PORTAL_END_NAMES),
            Form::opt(SPAN_NAMES),
        ],
    )),
};

pub const COMMAND: Command = Command {
    name: "defaults",
    aliases: &[],
    summary: "Set the map's defaults, field by field",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["default", "style", "inherit"],
    execute: execute_defaults,
};

fn execute_defaults(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = match super::descend(&GRAMMAR, args) {
        Ok(descent) => descent,
        Err(refused) => return refused,
    };
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    if pairs.is_empty() {
        return ExecResult::err(usage::no_arguments_message(&GRAMMAR));
    }
    let changes: Vec<(&str, &str)> = pairs.iter().map(|pair| (pair.key.name, pair.value)).collect();
    let steps = map::defaults(eff.document(), &changes);
    super::edit(eff, steps, "defaults")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::MindMapDocument;

    /// **Every key `defaults` declares is a field `set_default` sets**,
    /// and every group it reads is declared: each key, given a value of
    /// its kind, changes the defaults; `reset` then puts them back. A key
    /// the grammar offered and the setter refused would be a field help
    /// lists and the verb rejects. Control: a key no group has is
    /// refused.
    #[test]
    fn test_every_declared_default_is_a_field_the_verb_sets() {
        let doc = MindMapDocument::new_blank(None, 50);
        let base = doc.doc().defaults().clone();
        let face = baumhard::font::fonts::loaded_families_iter()
            .find(|family| baumhard::document::format::font(family).is_some())
            .expect("a compiled face a map can name");
        let sample = |key: &str| match key.rsplit('.').next().unwrap_or_default() {
            "shape" => "ellipse",
            "font" => face,
            "color" => "#102030",
            "weight" => "bold",
            "italic" | "visible" => "true",
            "start" => "top",
            "direction" => "ccw",
            "fit" => "repeat",
            "span" => "whole",
            "sides" => "✦",
            "content" => "·",
            "t" | "border_t" => "0.25",
            "size" => "0.1",
            _ => "3",
        };
        let mut groups = Vec::new();
        for key in GRAMMAR.keys() {
            let mut defaults = base.clone();
            map::set_default(doc.doc(), &mut defaults, key.name, sample(key.name))
                .unwrap_or_else(|e| panic!("{}: {e}", key.name));
            assert_ne!(defaults, base, "{} changes the defaults", key.name);
            map::set_default(doc.doc(), &mut defaults, key.name, "reset").unwrap();
            assert_eq!(defaults, base, "{} resets", key.name);
            let group = key.name.split('.').next().unwrap();
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
        assert_eq!(groups, map::GROUPS, "every group, in the file's order");
        let mut defaults = base.clone();
        assert!(map::set_default(doc.doc(), &mut defaults, "frame.nope", "1").is_err());
        assert!(map::set_default(doc.doc(), &mut defaults, "canvas.size", "1").is_err());
    }
}
