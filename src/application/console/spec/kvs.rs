// SPDX-License-Identifier: MPL-2.0

//! The kv parse loop, written once.
//!
//! Every verb used to open its own copy: iterate `args.kvs()`,
//! match the key against a hand-kept list, and reject the rest with
//! its own wording. Eleven verbs, eleven copies, and the rejection
//! wording differed at nearly every one. Worse, the copies only ever
//! asked "is this key in my list" — never "does the *form* the user
//! typed read it" — so `border preset heavy color=#fff` staged the
//! preset and discarded the color without a word, identically on
//! four surfaces.
//!
//! [`read`] asks both questions from the one declaration: the key
//! must exist at this level, and the matched subverb's forms must
//! name it.

use crate::application::console::parser::Args;

use super::descent::Stop;
use super::{Descent, Grammar, Key};

/// One accepted `key=value`, resolved to the [`Key`] that declared
/// it so the caller reads a typed declaration rather than a string.
pub struct Pair<'a> {
    pub key: &'static Key,
    pub value: &'a str,
}

/// The kv pairs on this line, in the order they were typed, or the
/// rejection for the first one the matched form does not read.
///
/// The check is *per form*, not per level. A key the level declares
/// but this subverb does not name is refused by name and pointed at
/// the form that does read it, rather than being silently dropped.
pub fn read<'a>(descent: &Descent, args: &'a Args) -> Result<Vec<Pair<'a>>, String> {
    // No form matched, so there is nothing to say about the keys:
    // the caller owns the unknown-subverb or quoting rejection, and
    // that is the error the user needs. Reporting a key problem
    // first would answer a question they did not ask.
    if matches!(descent.stop, Stop::Unknown | Stop::KvForm) {
        return Ok(Vec::new());
    }
    let level = descent.level;
    let readable = readable_keys(descent);
    let full = full_label(descent);
    let mut out = Vec::new();
    for (k, v) in args.kvs() {
        if !readable.contains(&k) {
            return Err(unread_key_message(level, &full, &readable, k));
        }
        match level.key(k) {
            Some(key) => out.push(Pair { key, value: v }),
            // Unreachable while `test_every_form_key_is_declared_by_its_level`
            // holds — a form can only name a key the level declares.
            // Degrading rather than panicking keeps the guarantee
            // CODE_CONVENTIONS §9 asks of an interactive path.
            None => return Err(format!("{}: unknown key '{}'", full, k)),
        }
    }
    Ok(out)
}

/// [`read`], plus the rejection for a positional past everything
/// the matched form declares.
///
/// The two refusals every form owes, in one call: a kv it does not
/// read, and an argument it has no slot for. Before the engine
/// these were a `reject_extras` on four subverbs, a
/// `reject_extra_positional` on three more, and nothing at all on
/// the rest — `border preset heavy color=#fff` dropped the color in
/// silence on four surfaces.
pub fn read_strict<'a>(descent: &Descent, args: &'a Args) -> Result<Vec<Pair<'a>>, String> {
    let pairs = read(descent, args)?;
    if matches!(descent.stop, Stop::Unknown | Stop::KvForm) {
        return Ok(pairs);
    }
    let slots = descent
        .subverb()
        .map(|s| s.slot_count())
        .unwrap_or_else(|| descent.level.bare.as_ref().map(|b| b.slot_count()).unwrap_or(0));
    if let Some(extra) = descent.extra_positional(args, slots) {
        return Err(super::usage::extra_positional_message(
            descent.level,
            &full_label(descent),
            extra,
        ));
    }
    Ok(pairs)
}

/// Fetch one key's value from an already-read pair list. `None`
/// when the key was not on the line; the **last** occurrence wins,
/// which is the shell intuition a repeated key carries.
pub fn value<'a>(pairs: &[Pair<'a>], name: &str) -> Option<&'a str> {
    pairs.iter().rfind(|p| p.key.name == name).map(|p| p.value)
}

/// The keys the form the user actually typed reads.
pub fn readable_keys(descent: &Descent) -> Vec<&'static str> {
    match descent.stop {
        Stop::Matched(subverb) => subverb.readable_keys(),
        Stop::Bare => descent
            .level
            .bare
            .as_ref()
            .map(|b| b.readable_keys())
            .unwrap_or_default(),
        Stop::Unknown | Stop::KvForm => Vec::new(),
    }
}

/// The words a message from this stop leads with: the level's label
/// plus the subverb that matched, so a rejection is copy-pasteable
/// for the form that printed it.
pub fn full_label(descent: &Descent) -> String {
    match descent.stop {
        Stop::Matched(subverb) => format!("{} {}", descent.level.label, subverb.name),
        _ => descent.level.label.to_string(),
    }
}

fn unread_key_message(grammar: &'static Grammar, full: &str, readable: &[&'static str], key: &str) -> String {
    let mut out = format!("{}: unknown key '{}'; ", full, key);
    if readable.is_empty() {
        out.push_str(&format!("{} takes no keys", full));
    } else {
        out.push_str(&format!("{} reads {}", full, readable.join(" | ")));
    }
    // A key the *level* declares, refused because this form does
    // not read it, is a different mistake from a typo — say where
    // it does belong. `border preset heavy color=#fff` used to
    // drop the color in silence on four surfaces.
    if grammar.key(key).is_some() {
        if let Some(bare) = &grammar.bare {
            if bare.readable_keys().contains(&key) {
                out.push_str(&format!(
                    ". `{}=` belongs to the composed form: `{} {}=<value>`",
                    key, grammar.label, key
                ));
            }
        }
    }
    out
}
