// SPDX-License-Identifier: MPL-2.0

//! Document-lifecycle apply_* helpers — undo, create / orphan /
//! delete / edit on the current selection, and the cross-platform
//! clipboard arms (copy / cut / paste). Each routes through the
//! shared rebuild plumbing so geometry-changing edits trigger a
//! full scene rebuild while read-only ones (copy) skip it.

use crate::application::document::{MindMapDocument, SelectionState};

use super::RebuildContext;

/// Walk the undo stack one step back. If an animation is in flight
/// when undo fires, fast-forward it first so the undo lands on a
/// settled scene state rather than mid-transition (otherwise the
/// undo'd write competes with the still-running animation envelope).
/// Both dispatchers route through this so the fast-forward
/// behaviour is platform-uniform — pre-Track-A WASM skipped it.
pub(in crate::application::app) fn apply_undo(rc: &mut RebuildContext<'_>) {
    if rc.document.has_active_animations() {
        rc.document.fast_forward_animations(rc.mindmap_tree.as_mut());
    }
    if rc.document.undo() {
        rc.rebuild_after_geometry_change();
    }
}

/// Create a new orphan node at the given canvas-space position
/// and select it. Triggers a geometry-change rebuild because the
/// new node may shift connection routes / introduce new edges.
pub(in crate::application::app) fn apply_create_orphan_node(
    canvas_pos: glam::Vec2,
    rc: &mut RebuildContext<'_>,
) {
    rc.document.create_orphan_and_select(canvas_pos);
    rc.rebuild_after_geometry_change();
}

/// Create a new orphan node at `canvas_pos`, select it, rebuild,
/// then open the inline text editor on the new node pre-cleared.
/// The keyboard-driven shape of `Action::CreateOrphanNodeAndEdit`.
///
/// Mouse-driven empty-canvas double-click reaches the same
/// outcome through `dispatch::dispatch_create_orphan_and_edit`
/// (which uses `DispatchHit::canvas_pos` instead of `cursor_pos`)
/// — that helper is called inline by the `DoubleClickActivate`
/// arm and stays in `dispatch.rs` because `DispatchHit` doesn't
/// flow through `dispatch_compatible`.
pub(in crate::application::app) fn apply_create_orphan_node_and_edit(
    canvas_pos: glam::Vec2,
    rc: &mut RebuildContext<'_>,
    text_edit_state: &mut super::super::super::text_edit::TextEditState,
) {
    let new_id = rc.document.create_orphan_and_select(canvas_pos);
    rc.rebuild_after_geometry_change();
    super::super::super::text_edit::open_text_edit(
        &new_id,
        true,
        rc.document,
        text_edit_state,
        rc.mindmap_tree,
        rc.app_scene,
        rc.renderer,
    );
}

/// Detach every currently-selected node from its parent. No-op
/// when nothing is selected or every selected node was already a
/// root.
pub(in crate::application::app) fn apply_orphan_selection(rc: &mut RebuildContext<'_>) {
    if rc.document.apply_orphan_selection_with_undo() {
        rc.rebuild_after_geometry_change();
    }
}

/// Delete the current selection. Pre-flight checks (selection
/// non-empty, deletable) live in the document method; this helper
/// just gates the rebuild.
pub(in crate::application::app) fn apply_delete_selection(rc: &mut RebuildContext<'_>) {
    if rc.document.apply_delete_selection() {
        rc.rebuild_after_geometry_change();
    }
}

/// Open the inline node text editor on a `Single`-selection.
/// Returns `true` when the editor opened (selection was Single
/// and the caller's editor-side bookkeeping should run); `false`
/// when the selection wasn't a single node (caller may fall
/// through to other branches — `Action::EditSelection` is
/// classified `NativeOnly` because the EdgeLabel and Portal
/// branches go to inline modal editors that only exist on
/// native).
///
/// The Single branch IS cross-platform: `open_text_edit`
/// (`text_edit/editor.rs`) compiles on both targets and is
/// renderer + document only.
pub(in crate::application::app) fn apply_open_text_edit_on_single(
    clean: bool,
    rc: &mut RebuildContext<'_>,
    text_edit_state: &mut super::super::super::text_edit::TextEditState,
) -> bool {
    let SelectionState::Single(id) = rc.document.selection.clone() else {
        return false;
    };
    super::super::super::text_edit::open_text_edit(
        &id,
        clean,
        rc.document,
        text_edit_state,
        rc.mindmap_tree,
        rc.app_scene,
        rc.renderer,
    );
    true
}

// ── Clipboard ───────────────────────────────────────────────────
//
// Cross-platform: `clipboard::{read,write}_clipboard` are logged
// stubs on WASM (pending the async-clipboard integration). The
// trait-driven walk over `selection_targets` is identical on both
// targets — `console::traits` compiles WASM-side because it only
// reaches into the document model, not the cfg-gated console
// runtime.

/// Copy or Cut the current selection's clipboard-eligible content
/// to the system clipboard. Cut additionally clears the source
/// component's text where the trait supports it. Read-only on the
/// document — no rebuild.
pub(in crate::application::app) fn apply_copy_or_cut(is_cut: bool, doc: &mut MindMapDocument) {
    use crate::application::console::traits::{
        selection_targets, view_for, ClipboardContent, HandlesCopy, HandlesCut,
    };
    let targets = selection_targets(&doc.selection);
    for tid in &targets {
        let mut view = view_for(doc, tid);
        let content = if is_cut {
            view.clipboard_cut()
        } else {
            view.clipboard_copy()
        };
        if let ClipboardContent::Text(text) = content {
            crate::application::clipboard::write_clipboard(&text);
            break;
        }
    }
}

/// Read the system clipboard and paste into every clipboard-eligible
/// target in the current selection. Triggers a geometry rebuild iff
/// at least one target accepted the paste.
pub(in crate::application::app) fn apply_paste(rc: &mut RebuildContext<'_>) {
    use crate::application::console::traits::{selection_targets, view_for, HandlesPaste, Outcome};
    let Some(text) = crate::application::clipboard::read_clipboard() else {
        return;
    };
    let targets = selection_targets(&rc.document.selection);
    let mut any_applied = false;
    for tid in &targets {
        let mut view = view_for(rc.document, tid);
        if let Outcome::Applied = view.clipboard_paste(&text) {
            any_applied = true;
        }
    }
    if any_applied {
        rc.rebuild_after_geometry_change();
    }
}
