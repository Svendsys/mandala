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
//!
//! A level that composes a shared subverb set routes the whole set
//! to that set's own dispatcher through a [`Delegates`] row rather
//! than through a hand-written arm, so the hand-off is data too: the
//! names a dispatcher reports to the registry test are read off the
//! same rows [`lookup_or_delegate`] reads, and deleting the row both
//! drops those names from the report and leaves its variant
//! unconstructed.

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

/// One dispatcher's hand-offs: each shared subverb set it composes,
/// paired with the variant that runs the set's own dispatcher.
pub type Delegates<R> = [(&'static [Subverb], R)];

/// [`lookup`], then the first [`Delegates`] row whose set declares
/// `subverb` (by name, see [`declares`]).
pub fn lookup_or_delegate<R: Copy>(
    routes: &Routes<R>,
    delegates: &Delegates<R>,
    subverb: &Subverb,
) -> Option<R> {
    lookup(routes, subverb).or_else(|| {
        delegates
            .iter()
            .find(|(set, _)| declares(set, subverb))
            .map(|(_, r)| *r)
    })
}

/// Whether `set` declares `subverb`'s name. For a level that
/// composes a shared subverb set and hands the whole set to the
/// set's own dispatcher — `border`'s seven per-field subverbs, which
/// `canvas border` and `canvas section-frame [focused]` compose too.
///
/// By name rather than by address: the shared sets are `const`s, and
/// a `const` is copied at each use, so the row a descent hands back
/// need not sit at the address the dispatcher's copy does. A name is
/// enough because names are unique within a level
/// (`test_names_are_unique_within_a_level`), so on a level that
/// composes `set` a row spelled like one of `set`'s *is* that row.
pub fn declares(set: &[Subverb], subverb: &Subverb) -> bool {
    set.iter().any(|s| s.name == subverb.name)
}

/// The rejection for a matched subverb no table routes. Unreachable
/// while every dispatcher's table covers its levels; logged at
/// `error!` because reaching it means that test was not run.
pub fn miss(level: &Grammar, subverb: &Subverb) -> String {
    log::error!(
        "{}: subverb '{}' is declared but no dispatcher routes it",
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

/// The shared sets a [`Delegates`] table hands on, for the registry
/// test to check each is held against its own table.
#[cfg(test)]
pub fn sets<R>(delegates: &Delegates<R>) -> Vec<&'static [Subverb]> {
    delegates.iter().map(|(set, _)| *set).collect()
}

/// [`names`], plus every name a [`Delegates`] row hands on. The
/// delegated set's own table is held against the set separately.
#[cfg(test)]
pub fn names_with<R>(routes: &Routes<R>, delegates: &Delegates<R>) -> Vec<&'static str> {
    let mut out = names(routes);
    out.extend(delegates.iter().flat_map(|(set, _)| set.iter().map(|s| s.name)));
    out
}
