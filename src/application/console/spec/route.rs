// SPDX-License-Identifier: MPL-2.0

//! The fifth surface: which handler a matched subverb runs.
//!
//! The declaration answers four surfaces on its own — the parse
//! loop, completion, `usage` and the hint popup — so a row added to
//! a level's subverbs is right on all four by the same edit. What a
//! subverb *does* stays in the verb, and every verb used to choose it
//! with a `match subverb.name` whose last arm was `_ =>`: correct for
//! the names declared on the day it was written, and silently wrong
//! for the next one. A new `fps snapshot` row would have completed,
//! appeared in `help`, parsed, and then toggled the rolling average,
//! because that is what the catch-all ran (#155).
//!
//! A dispatcher now declares a [`Routes`] table instead — each
//! declared name paired with a variant of the verb's own route enum
//! — and matches on the variant. The two halves close the gap from
//! both sides:
//!
//! - **The handler side is the compiler's.** The match is over an
//!   enum, so it is exhaustive without a catch-all. Every route enum
//!   is private to its dispatcher and has one payload-free variant
//!   per row, so a variant whose row is deleted is constructed
//!   nowhere: dead code, which the `-D warnings` clippy gate refuses.
//!   That holds only under those two rules; a public enum (another
//!   module may construct it) or a variant carrying its distinction
//!   in a payload (another row still constructs it) would leave
//!   clippy quiet and the name side below the only check.
//! - **The name side is a test's.** `console::spec::tests` holds
//!   every dispatcher's table against the subverbs its levels
//!   declare, in both directions, and fails on a declared subverb no
//!   table routes, naming it. (Named rather than linked — the module
//!   is `#[cfg(test)]`.)
//!
//! The arm a lookup miss takes, [`miss`], is therefore unreachable
//! while that test holds. It degrades to an error naming the
//! subverb rather than panicking, which is what CODE_CONVENTIONS §9
//! asks of an interactive path.
//!
//! A subverb that opens a child level is never routed: the descent
//! steps through it, so it never arrives as `Stop::Matched`.

use super::{Grammar, Subverb};

/// One dispatcher's routing: each declared subverb name it handles,
/// paired with the variant its match runs. Names are the
/// declaration's canonical spelling — the descent matches typed
/// words case-insensitively and hands back the declared row, so a
/// lookup compares exactly.
pub type Routes<R> = [(&'static str, R)];

/// The route `subverb` takes, or `None` when the table has no row
/// for it. `None` is the [`miss`] arm.
pub fn lookup<R: Copy>(routes: &Routes<R>, subverb: &Subverb) -> Option<R> {
    routes
        .iter()
        .find(|(name, _)| *name == subverb.name)
        .map(|(_, r)| *r)
}

/// The rejection for a matched subverb no table routes. Unreachable
/// while every dispatcher's table covers its levels; logged at
/// `error!` because reaching it means that test was not run.
pub fn miss(level: &Grammar, subverb: &Subverb) -> String {
    log::error!(
        "console: {} subverb '{}' is declared but no dispatcher routes it",
        level.label,
        subverb.name
    );
    format!("internal: {} {} routing miss", level.label, subverb.name)
}

/// The names a table routes, for the registry test to hold against
/// the declaration.
#[cfg(test)]
pub fn names<R>(routes: &Routes<R>) -> Vec<&'static str> {
    routes.iter().map(|(name, _)| *name).collect()
}
