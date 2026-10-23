// SPDX-License-Identifier: MPL-2.0

//! How a family handler says no: one [`Refusal`] value a pure
//! helper can return and a test can compare, mapped onto the error
//! reply in one place ([`refuse`]). Shared by the `query`, `scene` and
//! `capture` families, which refuse for the same reasons and must word
//! the missing-document case the same way.

use crate::application::ipc::envelope::{code, Reply};
use crate::application::ipc::registry::Call;

/// Why a command was refused, before the reply that carries it is
/// built. Pure helpers return this so they can be tested against a
/// real document without the registry's request plumbing.
#[derive(Debug, PartialEq)]
pub(super) enum Refusal {
    /// A parameter is wrong: `invalid_params`, naming it.
    Invalid(&'static str, String),
    /// What the request named is not there: `not_found`.
    NotFound(String),
    /// The command could not be honored: `internal`.
    Internal(String),
    /// The app is in the middle of something the command would
    /// disturb, and retrying once it is done will work: `busy`.
    Busy(String),
}

/// The error reply for `refusal`.
pub(super) fn refuse(call: &Call<'_>, refusal: Refusal) -> Reply {
    match refusal {
        Refusal::Invalid(param, message) => call.invalid_params(param, message),
        Refusal::NotFound(message) => call.error(code::NOT_FOUND, message, None),
        Refusal::Internal(message) => call.error(code::INTERNAL, message, None),
        Refusal::Busy(message) => call.error(code::BUSY, message, None),
    }
}

/// `no_document` for the app that has started but holds no document
/// (its initial load failed). The registry's own `no_document` says
/// the app is still starting, which would mislead here.
pub(super) fn no_loaded_document(call: &Call<'_>) -> Reply {
    call.error(
        code::NO_DOCUMENT,
        "no document is loaded: send `act.console` with `open <path>` to load one",
        None,
    )
}
