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
    // Section selection routes to its owning node's text editor —
    // the editor today edits the section identified by the
    // selection's `section_idx` (the inner editor consults
    // `selected_section()`); this dispatch shim only needs to
    // resolve the owning node id for the entry point.
    let id = match rc.document.selection.clone() {
        SelectionState::Single(id) => id,
        SelectionState::Section(s) => s.node_id,
        _ => return false,
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
    // `selection_targets` emits one target per node for `Multi`
    // and one target per section for `MultiSection`; everything
    // else emits exactly one. We accumulate Section payloads
    // across the loop so a `MultiSection` cut/copy reaches every
    // section instead of stopping at the first (the pre-N3 path
    // broke on the first non-empty match — correct then because
    // every Multi-shape produced single-target clipboard
    // content, broken now because `MultiSection` produces N).
    let targets = selection_targets(&doc.selection);
    let mut section_texts: Vec<String> = Vec::new();
    let mut first_section_payload: Option<crate::application::document::SectionPayload> = None;
    let mut first_text: Option<String> = None;
    for tid in &targets {
        let mut view = view_for(doc, tid);
        let content = if is_cut {
            view.clipboard_cut()
        } else {
            view.clipboard_copy()
        };
        match content {
            ClipboardContent::Text(text) => {
                // Plain text targets (Single / Multi-node /
                // Edge / EdgeLabel / PortalLabel / PortalText)
                // remain single-shot. `Multi(ids)` emits N text
                // targets that all share `display_text`; taking
                // the first is the pre-N3 contract.
                if first_text.is_none() {
                    first_text = Some(text);
                }
                break;
            }
            ClipboardContent::Section { text, payload } => {
                if section_texts.is_empty() {
                    first_section_payload = Some(payload);
                }
                section_texts.push(text);
                // Continue iterating — for cut, every section
                // must have its text/runs cleared, not just the
                // first.
            }
            ClipboardContent::Empty | ClipboardContent::NotApplicable => {}
        }
    }
    if let Some(text) = first_text {
        crate::application::clipboard::write_clipboard(&text);
        return;
    }
    if !section_texts.is_empty() {
        // OS clipboard gets the joined plain text so cross-app
        // paste sees every selected section's content. Within-
        // app structured paste rides the in-process buffer; a
        // single-section copy round-trips per-run formatting +
        // section chrome via the structured payload, but a
        // multi-section copy falls back to text-only since
        // there's no `MultiSection` payload variant today
        // (deferred — would need a fan-out paste path on the
        // section-target clipboard read).
        let joined = section_texts.join("\n");
        crate::application::clipboard::write_clipboard(&joined);
        if section_texts.len() == 1 {
            if let Some(payload) = first_section_payload {
                crate::application::clipboard::write_section_clipboard(
                    section_texts.into_iter().next().expect("len == 1"),
                    payload,
                );
            }
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
