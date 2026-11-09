// SPDX-License-Identifier: MPL-2.0

//! `node` — the selected nodes' own fields, on the one writer (V1-C5,
//! #246): one line is one request, so one undo entry.
//!
//! - `node resize <w> <h>` pins each node's minimum size (D5);
//! - `node fit` clears it, so the node fits its content;
//! - `node shape=… radius=… opacity=… hidden=… folded=… channel=…
//!   palette=…` sets those fields (`crate::application::fields::node`).
//!   A node draws no fill (D12), so there is no `fill` or `fade`.
//!
//! `node edit` enters node editing, which V1-C8 (#249) rebuilds; until
//! then it refuses, naming that issue.

use super::Command;
use crate::application::console::completion::Completion;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::Stop;
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{
    free, kvs, usage, Bare, Form, Grammar, Key, Slot, Subverb, Vocabulary, Word,
};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::fields::node::{self, NodeFields};
use crate::application::fields::values;
use glam::Vec2;

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("resize", "geometry", "pin the node's width and height")
        .taking(&[Form::slots(&[Slot::req(free("w")), Slot::req(free("h"))])]),
    Subverb::bare("fit", "geometry", "fit the node to its content"),
    Subverb::bare("edit", "editor", "enter node-edit mode on the selection"),
];

const SHAPES: &[Word] = &[
    Word::new("rectangle", "corners rounded by `radius`"),
    Word::new("ellipse", "the ellipse inscribed in the node"),
];
const BOOLS: &[Word] = &[Word::bare("true"), Word::bare("false")];

/// The map's palettes, and `none`.
fn palette_rows(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    ctx.document
        .doc()
        .palettes()
        .map(|(_, palette)| palette.name.clone())
        .filter(|name| name.starts_with(partial))
        .map(|name| Completion {
            text: name.clone(),
            display: name,
            hint: None,
            font_family: None,
        })
        .collect()
}

const KEYS: &[Key] = &[
    Key::new(
        "shape",
        "the outline: rectangle | ellipse",
        Vocabulary::Words(SHAPES),
    ),
    Key::new(
        "radius",
        "corner radius, % of half the shorter side",
        free("0-100"),
    ),
    Key::new("opacity", "multiplies the subtree's alpha", free("0-1")),
    Key::new(
        "hidden",
        "hide the node and its subtree",
        Vocabulary::Words(BOOLS),
    ),
    Key::new("folded", "collapse its descendants", Vocabulary::Words(BOOLS)),
    Key::new("channel", "its order among its siblings", free("n")),
    Key::new(
        "palette",
        "color its subtree by depth from a palette",
        Vocabulary::Rows {
            placeholder: "name",
            rows: palette_rows,
            sentinels: &[Word::new("none", "unbind")],
        },
    ),
];

/// Which handler each declared subverb runs (`spec::route`).
#[derive(Clone, Copy)]
enum Route {
    Resize,
    Fit,
    Edit,
}

const ROUTES: &Routes<Route> = &[
    ("resize", Route::Resize),
    ("fit", Route::Fit),
    ("edit", Route::Edit),
];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub static GRAMMAR: Grammar = Grammar {
    label: "node",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS],
    bare: Some(Bare::new(
        "fields",
        &[Form::opt(&[
            "shape", "radius", "opacity", "hidden", "folded", "channel", "palette",
        ])],
    )),
};

pub const COMMAND: Command = Command {
    name: "node",
    aliases: &[],
    summary: "Resize the selected nodes, fit them to their content, or set their shape, opacity, visibility, fold, channel and palette",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["size", "shrink", "content", "node-edit", "outline", "round"],
    execute: execute_node,
};

fn execute_node(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = match super::descend(&GRAMMAR, args) {
        Ok(descent) => descent,
        Err(refused) => return refused,
    };
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    let subverb = match descent.stop {
        Stop::Matched(subverb) => subverb,
        _ if pairs.is_empty() => return ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
        _ => {
            let fields = match read_fields(eff, &pairs) {
                Ok(fields) => fields,
                Err(msg) => return ExecResult::err(msg),
            };
            let steps = node::fields(eff.document(), &fields);
            return super::edit(eff, steps, "node");
        }
    };
    match route::lookup(ROUTES, subverb) {
        Some(Route::Resize) => {
            let slots = descent.slot_value(args);
            let size = match (slots.get(0), slots.get(1)) {
                (Some(w), Some(h)) => values::number(w).and_then(|w| Ok(Vec2::new(w, values::number(h)?))),
                _ => return ExecResult::err(usage::subverb_usage(descent.level, subverb)),
            };
            let steps = size.and_then(|size| node::min_size(eff.document(), Some(size)));
            super::edit(eff, steps, "node resize")
        }
        Some(Route::Fit) => {
            let steps = node::min_size(eff.document(), None);
            super::edit(eff, steps, "node fit")
        }
        Some(Route::Edit) => super::rebuilt_by("node edit", super::TEXT_OWNER),
        None => ExecResult::err(route::miss(descent.level, subverb)),
    }
}

/// The fields a `node` line's keys set.
fn read_fields(eff: &ConsoleEffects, pairs: &[kvs::Pair]) -> Result<NodeFields, String> {
    let mut fields = NodeFields::default();
    for pair in pairs {
        let value = pair.value;
        match pair.key.name {
            "shape" => fields.shape = Some(node::shape_kind(value)?),
            "radius" => fields.radius = Some(values::percent(value)?),
            "opacity" => fields.opacity = Some(values::number(value)?),
            "hidden" => fields.hidden = Some(values::boolean(value)?),
            "folded" => fields.folded = Some(values::boolean(value)?),
            "channel" => fields.channel = Some(values::index(value)?),
            "palette" => fields.palette = Some(node::palette(eff.document(), value)?),
            other => log::error!("node: engine admitted unread key '{other}'"),
        }
    }
    Ok(fields)
}
