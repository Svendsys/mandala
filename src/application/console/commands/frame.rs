// SPDX-License-Identifier: MPL-2.0

//! `frame` — the selected nodes' frames and their rings, on the one
//! writer (V1-C5, #246; design §6, D8, D10, D25). One line is one
//! request, so one undo entry; a preview is a transaction (§9).
//!
//! - `frame on | off` shows or hides each frame; `on` gives a node with
//!   no frame one ring of `─` repeated around it;
//! - `frame start=… direction=… fit=… gap=… inset=…` sets frame fields;
//! - `frame ring add [text=…] [size=…]` adds a ring outside the others,
//!   holding its text as one span; `frame ring <i> text=… | size=… |
//!   fit=…` sets ring `i`'s (innermost first), and `frame ring <i>
//!   delete` removes it;
//! - `frame preview <keys>` shows the fields without making an edit;
//!   `frame commit` makes the preview one undo entry, and `frame cancel`
//!   (or `ExitMode`) reverts it.
//!
//! Values are read in the console's spellings
//! (`crate::application::fields::values`): `start=top-left|…|left` or
//! `<edge>:<at>`, `direction=cw|ccw`, `fit=once[:<filler>]|repeat|
//! spread|fit[:<min>,<max>]`; `reset` inherits.

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::Stop;
use crate::application::console::spec::kvs::{self, Pair};
use crate::application::console::spec::route::{self, Routes};
use crate::application::console::spec::{
    free, free_words, usage, Bare, Form, Grammar, Key, Slot, Subverb, Vocabulary, Word,
};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::Interaction;
use crate::application::fields::frame::{self, FrameField, RingFields};
use crate::application::fields::values;

const RESET: &[Word] = &[Word::new("reset", "inherit")];
const STARTS: &[Word] = &[
    Word::bare("top-left"),
    Word::bare("top"),
    Word::bare("top-right"),
    Word::bare("right"),
    Word::bare("bottom-right"),
    Word::bare("bottom"),
    Word::bare("bottom-left"),
    Word::bare("left"),
    Word::new("reset", "inherit"),
];
const DIRECTIONS: &[Word] = &[
    Word::new("cw", "clockwise"),
    Word::new("ccw", "counter-clockwise"),
    Word::new("reset", "inherit"),
];
const FITS: &[Word] = &[
    Word::new("once", "the text once; `once:<filler>` fills the rest"),
    Word::new("repeat", "the text repeated round the outline"),
    Word::new("spread", "the text once, spread to the ends"),
    Word::new("fit", "the text once, scaled to fit; `fit:<min>,<max>`"),
    Word::new("reset", "inherit"),
];

/// The frame field keys, which `frame` and `frame preview` read.
const KEYS: &[Key] = &[
    Key::new(
        "start",
        "where the text starts; or <edge>:<at>",
        free_words("place", STARTS),
    ),
    Key::new(
        "direction",
        "which way the text runs: cw | ccw",
        Vocabulary::Words(DIRECTIONS),
    ),
    Key::new(
        "fit",
        "what the rings' text does round the outline",
        free_words("mode", FITS),
    ),
    Key::new("gap", "the space between rings", free_words("px", RESET)),
    Key::new(
        "inset",
        "the first ring's distance from the outline",
        free_words("px", RESET),
    ),
];

/// The ring keys `frame ring` reads.
const RING_KEYS: &[Key] = &[
    Key::new("text", "the ring's text, one span", free("text")),
    Key::new(
        "size",
        "font size, a fraction of the node's shorter side",
        free_words("rel", RESET),
    ),
    Key::new(
        "fit",
        "the ring's fit, over the frame's",
        free_words("mode", FITS),
    ),
];

const ADD: &[Word] = &[Word::new("add", "add a ring outside the others")];
const DELETE: &[Word] = &[Word::new("delete", "remove the ring")];

/// `frame ring …`: no subverbs, so its index sits in the first slot.
pub static RING: Grammar = Grammar {
    label: "frame ring",
    subverb_sets: &[],
    key_sets: &[RING_KEYS],
    bare: Some(Bare::new(
        "rings",
        &[
            Form::slots(&[Slot::req(Vocabulary::Words(ADD))]).reading(&["text", "size"]),
            Form::slots(&[Slot::req(free("i")), Slot::opt(Vocabulary::Words(DELETE))])
                .reading(&["text", "size", "fit"]),
        ],
    )),
};

const FIELDS: &[&str] = &["start", "direction", "fit", "gap", "inset"];

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("on", "visibility", "show the frame; a node with none gets one"),
    Subverb::bare("off", "visibility", "hide the frame"),
    Subverb::nested("ring", "rings", "add, set or delete a ring", &RING),
    Subverb::bare("preview", "preview", "show frame fields without an edit")
        .taking(&[Form::opt(FIELDS)])
        .gated(),
    Subverb::bare("commit", "preview", "make the preview one undo entry"),
    Subverb::bare("cancel", "preview", "revert the preview"),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "frame",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS],
    bare: Some(Bare::new("fields", &[Form::opt(FIELDS)])),
};

/// Which handler each declared subverb runs (`spec::route`).
#[derive(Clone, Copy)]
enum Route {
    On,
    Off,
    Preview,
    Commit,
    Cancel,
}

const ROUTES: &Routes<Route> = &[
    ("on", Route::On),
    ("off", Route::Off),
    ("preview", Route::Preview),
    ("commit", Route::Commit),
    ("cancel", Route::Cancel),
];

/// The names [`ROUTES`] handles, for the registry test in
/// `console::spec::tests`.
#[cfg(test)]
pub(crate) fn routed_names() -> Vec<&'static str> {
    route::names(ROUTES)
}

pub const COMMAND: Command = Command {
    name: "frame",
    aliases: &[],
    summary: "Show, hide, set or preview the selected nodes' frames, and add, set or delete their rings",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["border", "outline", "ring", "stanza", "poem"],
    execute: execute_frame,
};

fn execute_frame(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = match super::descend(&GRAMMAR, args) {
        Ok(descent) => descent,
        Err(refused) => return refused,
    };
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    if std::ptr::eq(descent.level, &RING) {
        let slots = descent.slot_value(args);
        return ring(eff, slots.get(0), slots.get(1), &pairs);
    }
    let subverb = match descent.stop {
        Stop::Matched(subverb) => subverb,
        _ if pairs.is_empty() => return ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
        _ => {
            let steps = fields(&pairs).and_then(|fields| frame::set(eff.document(), &fields));
            return super::edit(eff, steps, "frame");
        }
    };
    let doc = eff.document_mut();
    match route::lookup(ROUTES, subverb) {
        Some(Route::On) => {
            let steps = frame::visible(doc, true);
            super::edit(eff, steps, "frame on")
        }
        Some(Route::Off) => {
            let steps = frame::visible(doc, false);
            super::edit(eff, steps, "frame off")
        }
        Some(Route::Preview) => {
            let staged = fields(&pairs)
                .map_err(crate::application::fields::Refused::Because)
                .and_then(|fields| match fields.is_empty() {
                    true => Err(crate::application::fields::Refused::Because(
                        usage::subverb_usage(descent.level, subverb),
                    )),
                    false => frame::preview(doc, &fields, baumhard::document::writer::ChangeSource::Console),
                });
            match staged {
                Ok(()) => ExecResult::ok_msg(
                    "frame preview: staged; `frame commit` keeps it, `frame cancel` reverts it",
                ),
                Err(refused) => ExecResult::err(refused.message()),
            }
        }
        Some(Route::Commit) => match doc.open_interaction() {
            Some(Interaction::FramePreview) => match doc.commit(Interaction::FramePreview) {
                Some(_) => ExecResult::ok_msg("frame preview: committed"),
                None => ExecResult::ok_msg("frame preview: no change"),
            },
            _ => ExecResult::err("frame commit: no frame preview is open"),
        },
        Some(Route::Cancel) => match doc.cancel(Interaction::FramePreview) {
            true => ExecResult::ok_msg("frame preview: cancelled"),
            false => ExecResult::err("frame cancel: no frame preview is open"),
        },
        None => ExecResult::err(route::miss(descent.level, subverb)),
    }
}

/// The frame fields a line's keys set.
fn fields(pairs: &[Pair]) -> Result<Vec<FrameField>, String> {
    pairs
        .iter()
        .map(|pair| FrameField::read(pair.key.name, pair.value))
        .collect()
}

/// `frame ring <add|i> [delete] [keys]`.
fn ring(eff: &mut ConsoleEffects, first: Option<&str>, second: Option<&str>, pairs: &[Pair]) -> ExecResult {
    let doc = eff.document();
    let size = kvs::value(pairs, "size")
        .map(|v| values::or_reset(v, values::number))
        .transpose();
    let (steps, label) = match first {
        None => return ExecResult::err(usage::no_arguments_message(&RING)),
        Some(word) if word.eq_ignore_ascii_case("add") => (
            size.and_then(|size| frame::add_ring(doc, kvs::value(pairs, "text"), size.flatten())),
            "frame ring add",
        ),
        Some(index) => {
            let index = match values::index(index) {
                Ok(index) => index,
                Err(msg) => {
                    return ExecResult::err(format!("frame ring: {msg}; give a ring's index or `add`"))
                }
            };
            match second {
                Some(_) if !pairs.is_empty() => {
                    return ExecResult::err("frame ring <i> delete: takes no keys");
                }
                Some(_) => (frame::delete_ring(doc, index), "frame ring delete"),
                None => {
                    let fit = kvs::value(pairs, "fit")
                        .map(|v| values::or_reset(v, values::fit))
                        .transpose();
                    let ring = size.and_then(|size| {
                        Ok(RingFields {
                            text: kvs::value(pairs, "text").map(str::to_string),
                            size,
                            fit: fit?,
                        })
                    });
                    (
                        ring.and_then(|ring| frame::set_ring(doc, index, &ring)),
                        "frame ring",
                    )
                }
            }
        }
    };
    super::edit(eff, steps, label)
}
