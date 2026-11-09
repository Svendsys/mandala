// SPDX-License-Identifier: MPL-2.0

//! History apply_* helpers: `Undo` and `Redo`, routed to the document's
//! writer (`Document::undo` / `Document::redo`, design §8.2). Nothing is
//! evaluated again: an undo reverts the entry's journals and a redo
//! replays them, and the view brings the drawing up to date from the
//! change feed they leave, as for any change.

use super::ViewContext;

/// Undo the newest history entry. Returns whether anything changed.
pub(in crate::application::app) fn apply_undo(vc: &mut ViewContext<'_>) -> bool {
    let changed = vc.document.undo().is_some();
    vc.sync();
    changed
}

/// Redo the entry undone last. Returns whether anything changed.
pub(in crate::application::app) fn apply_redo(vc: &mut ViewContext<'_>) -> bool {
    let changed = vc.document.redo().is_some();
    vc.sync();
    changed
}
