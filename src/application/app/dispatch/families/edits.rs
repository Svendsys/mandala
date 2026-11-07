// SPDX-License-Identifier: MPL-2.0

//! Edits — named, saved, undoable map changes (design §10.1) — fired from
//! a key bound in `edit_bindings` or a macro's edit step, rebuilt on the
//! one writer by V1-D1 (#250). Custom mutations, which these replace,
//! were deleted with the old model.

use super::super::cross_dispatch::DispatchOutcome;

/// The issue that fires edits.
pub(in crate::application::app) const OWNER: &str = "V1-D1 (#250)";

/// Fire the edit `edit_id` that a key or a macro step names: unhandled
/// until V1-D1, with a `debug!` naming it.
pub(in crate::application::app) fn fire(edit_id: &str) -> DispatchOutcome {
    log::debug!("dispatch: edit {edit_id:?} is fired by {OWNER}; unhandled until then");
    DispatchOutcome::Unhandled
}
