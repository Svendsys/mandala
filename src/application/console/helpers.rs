// SPDX-License-Identifier: MPL-2.0

//! Shared command-execution helpers.
//!
//! The bespoke verbs (`anchor` / `cap` / `body` / `edge` / `spacing`
//! / `label`) all follow the same shape: open with a couple of
//! guards (selected edge present, kvs non-empty), iterate kvs and
//! tally per-arg outcomes, and finalize into an [`ExecResult`]. The
//! helpers here are the shared bits — guards return
//! `Result<T, ExecResult>` so a verb can `?` past them and the
//! [`ApplyTally`] aggregator collects per-kv messages plus a
//! "did anything actually apply?" bit so the closing
//! [`ApplyTally::finalize`] picks the right Ok/Err shape.
//!
//! Out of scope: the trait-dispatch path (`color`, `font`, `zoom`)
//! already routes through [`traits::DispatchReport`] and a sibling
//! `finalize_report`. This module exists for the bespoke
//! direct-setter verbs that don't lift to that machinery.

use super::parser::Args;
use super::{ConsoleEffects, ExecResult};
use crate::application::document::EdgeRef;

/// Resolve the currently-selected edge or portal-mode edge (the
/// widened "selected edge" the bespoke verbs operate on). Returns
/// `Err(ExecResult::err("no edge selected"))` when no compatible
/// selection is active so the caller can `?` and forward the
/// error.
pub fn require_edge_or_portal(eff: &ConsoleEffects) -> Result<EdgeRef, ExecResult> {
    eff.document
        .selection
        .selected_edge_or_portal_edge()
        .ok_or_else(|| ExecResult::err("no edge selected"))
}

/// Collect every `key=value` token from `args` into an owned
/// `Vec`, returning `Err(ExecResult::err(usage))` when the
/// collection ends up empty. Mirrors the open of every kv-form
/// verb — the caller passes the verb's `usage:` string and the
/// helper substitutes it on the empty path.
///
/// Accepting `usage` as the empty-error message matches the prior
/// hand-rolled shape (`"usage: anchor from=<side> to=<side>"`)
/// without forcing a separate prefix.
pub fn collect_kvs_or_usage(
    args: &Args,
    usage: &str,
) -> Result<Vec<(String, String)>, ExecResult> {
    let kvs: Vec<(String, String)> = args
        .kvs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if kvs.is_empty() {
        return Err(ExecResult::err(usage.to_string()));
    }
    Ok(kvs)
}

/// Per-kv outcome aggregator for bespoke verbs. The pre-helper
/// shape was a hand-rolled `(messages: Vec<String>, any_applied:
/// bool)` pair plus a 6-line tail at the end of every verb that
/// folded the two into an [`ExecResult`]. This struct owns the
/// pair and exposes the same fold via [`ApplyTally::finalize`].
///
/// Why not just reuse [`super::traits::DispatchReport`]? The
/// trait-dispatch path also tracks per-target outcomes and
/// `all_failed` semantics that the bespoke verbs don't need; the
/// bespoke verbs only need messages + any-applied. Lifting them
/// onto `DispatchReport` would mean importing target-id machinery
/// they don't speak.
#[derive(Default)]
pub struct ApplyTally {
    pub messages: Vec<String>,
    pub any_applied: bool,
}

impl ApplyTally {
    /// Construct an empty tally. Use as the `let mut tally =
    /// ApplyTally::new()` opener at the top of a verb's kv-loop.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one kv outcome. `applied = true` flips the
    /// `any_applied` bit; `applied = false` invokes `msg` to
    /// produce a per-arg explanation that lands in `messages`.
    /// `msg` is a thunk so the caller doesn't allocate a String
    /// on the success path.
    pub fn note<F>(&mut self, applied: bool, msg: F)
    where
        F: FnOnce() -> String,
    {
        if applied {
            self.any_applied = true;
        } else {
            self.messages.push(msg());
        }
    }

    /// Push a hard error message — used by the kv-loop branches
    /// that detected an unparseable key/value before any setter
    /// got a chance to run. These messages are emitted regardless
    /// of `any_applied` because they aren't no-ops, they're bad
    /// input.
    pub fn note_error(&mut self, message: String) {
        self.messages.push(message);
    }

    /// Fold `(messages, any_applied)` into the `ExecResult` shape
    /// every verb closed with: an all-no-op or all-error pair
    /// becomes `Err(joined)`; mixed messages with at least one
    /// applied becomes `Lines`; the clean-success path becomes
    /// `Ok("<verb> applied")`.
    pub fn finalize(self, verb: &str) -> ExecResult {
        if !self.messages.is_empty() {
            if !self.any_applied {
                return ExecResult::err(self.messages.join("; "));
            }
            return ExecResult::lines(self.messages);
        }
        ExecResult::ok_msg(format!("{} applied", verb))
    }
}

/// Parse a positive finite `f32` from a kv value string,
/// returning a uniform error message on the failure paths the
/// bespoke verbs all reproduce verbatim. `key` is the kv name
/// (e.g. `"size"`, `"max"`, `"padding"`) — appears in the error
/// so the user knows which arg was bad.
///
/// Pre-helper, three verbs (`font`, `border`, `zoom`'s f32 path)
/// each reimplemented this with byte-identical wording.
pub fn parse_finite_pt(key: &str, value: &str) -> Result<f32, String> {
    match value.parse::<f32>() {
        Ok(n) if n.is_finite() && n > 0.0 => Ok(n),
        Ok(n) => Err(format!(
            "{}='{}' must be positive and finite; got {}",
            key, value, n
        )),
        Err(_) => Err(format!("{}='{}' is not a number", key, value)),
    }
}
