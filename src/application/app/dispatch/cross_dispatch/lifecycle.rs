// SPDX-License-Identifier: MPL-2.0

//! Document-lifecycle apply_* helpers — undo, create / orphan /
//! delete / edit on the current selection, and the cross-platform
//! clipboard arms (copy / cut / paste). Each routes through the
//! shared rebuild plumbing so geometry-changing edits trigger a
//! full scene rebuild while read-only ones (copy) skip it.

use crate::application::document::MindMapDocument;

use super::RebuildContext;

/// Joiner between multi-target clipboard fragments. Doubled `\n`
/// so the paste path can split unambiguously even when an
/// individual section's text contains `\n` line breaks from the
/// inline editor.
const MULTI_TARGET_SEPARATOR: &str = "\n\n";

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

/// Enter NodeEdit mode on the currently-selected node. Resolves the
/// owning node via `selection.primary_node_id()` (Single / Section /
/// SectionRange). Multi / MultiSection / edge / None warn and bail.
///
/// **Single-section short-circuit**: when the active node has
/// `sections.len() <= 1`, opens the text editor on section 0 in the
/// same call. This preserves today's "Enter on a node opens the
/// editor" UX for legacy migrated maps. Multi-section nodes stop
/// at NodeEdit mode — the user picks a section (click or
/// `section edit <idx>` console verb) and presses Enter again to
/// enter SectionEdit.
///
/// `clean: true` opens the editor with an empty buffer (mirrors
/// `EditSelectionClean`'s posture) — only used in the
/// single-section short-circuit path.
pub(in crate::application::app) fn apply_enter_node_edit(
    clean: bool,
    rc: &mut RebuildContext<'_>,
    text_edit_state: &mut super::super::super::text_edit::TextEditState,
) -> bool {
    use super::super::super::interaction_mode::InteractionMode;

    let Some(node_id) = rc.document.selection.primary_node_id().map(str::to_string) else {
        log::warn!(
            "EnterNodeEdit: selection has no primary node \
             (Multi / MultiSection / Edge / None) — nothing to edit"
        );
        return false;
    };

    *rc.interaction_mode = InteractionMode::NodeEdit { node_id: node_id.clone() };

    // Single-section short-circuit: open the editor immediately.
    // `open_text_edit` triggers its own rebuild, so we don't run
    // `rebuild_after_selection_change` separately here.
    let section_count = rc
        .document
        .mindmap
        .nodes
        .get(&node_id)
        .map(|n| n.sections.len())
        .unwrap_or(0);
    if section_count <= 1 {
        // Short-circuit path: this call ALSO flipped mode to NodeEdit,
        // so on close the editor must revert mode to Default — a
        // single-section node has nothing else to edit, so leaving
        // the user in NodeEdit + dimming is a UX dead-end.
        super::super::super::text_edit::open_text_edit_with_close_target(
            &node_id,
            clean,
            true, // exit_to_default_on_close
            rc.document,
            text_edit_state,
            rc.mindmap_tree,
            rc.app_scene,
            rc.renderer,
        );
    } else {
        // Multi-section: stop at NodeEdit so the user can pick
        // a section. Rebuild so the dimming + status-bar visuals
        // catch the mode change.
        rc.rebuild_after_selection_change();
    }
    true
}

/// Open the section text editor on the active section while in
/// NodeEdit mode. Preconditions:
/// - `interaction_mode == InteractionMode::NodeEdit { node_id }`.
/// - Selection picks the section: `Section(s)` / `SectionRange { sel: s, .. }`
///   use `s.section_idx`; `Single(node_id)` defaults to section 0.
///
/// `clean: true` opens the editor with an empty buffer.
///
/// Returns `true` if the editor opened. Mode stays at `NodeEdit`
/// (closing the editor returns to `NodeEdit`, not `Default`).
pub(in crate::application::app) fn apply_enter_section_edit(
    clean: bool,
    rc: &mut RebuildContext<'_>,
    text_edit_state: &mut super::super::super::text_edit::TextEditState,
) -> bool {
    use super::super::super::interaction_mode::InteractionMode;

    let active_node = match &*rc.interaction_mode {
        InteractionMode::NodeEdit { node_id } => node_id.clone(),
        _ => {
            log::warn!("EnterSectionEdit: no active NodeEdit mode; nothing to edit");
            return false;
        }
    };
    // Selection's owning node must match the active NodeEdit
    // target. A mismatch means selection drifted to a different
    // node (e.g. a click on a sibling) — the user wants to switch
    // node targets, not edit a section of the wrong one.
    let owner = rc.document.selection.primary_node_id();
    if owner.map_or(true, |id| id != active_node) {
        log::warn!(
            "EnterSectionEdit: selection owner ≠ active NodeEdit node ({:?} vs {})",
            owner, active_node
        );
        return false;
    }

    super::super::super::text_edit::open_text_edit(
        &active_node,
        clean,
        rc.document,
        text_edit_state,
        rc.mindmap_tree,
        rc.app_scene,
        rc.renderer,
    );
    true
}

/// Resolve the current `SelectionState` into a `ResizeTarget` and
/// flip the active interaction mode to `Resize { target }`. On a
/// non-resizable selection logs the resolution failure and leaves
/// mode untouched. Cross-platform: touches mode + model + scene
/// rebuild only.
///
/// Resolution logic is shared with the `mode resize` console verb
/// via [`super::super::super::interaction_mode::resolve_resize_target`].
pub(in crate::application::app) fn apply_enter_resize_mode(rc: &mut RebuildContext<'_>) {
    use super::super::super::interaction_mode::{
        resolve_resize_target, InteractionMode, ResizeTargetError,
    };

    match resolve_resize_target(&rc.document.selection, &rc.document.mindmap) {
        Ok(target) => {
            *rc.interaction_mode = InteractionMode::Resize { target };
            rc.rebuild_after_selection_change();
        }
        Err(ResizeTargetError::NoSelection) => {
            log::warn!("EnterResizeMode: no selection; nothing to resize");
        }
        Err(ResizeTargetError::MultiTarget) => {
            log::warn!(
                "EnterResizeMode: multi-target selection — single-target only; \
                 select a single node or section first"
            );
        }
        Err(ResizeTargetError::SectionFillParent { node_id, section_idx }) => {
            log::warn!(
                "EnterResizeMode: section {}[{}] is fill-parent (no Some size); cannot resize",
                node_id, section_idx,
            );
        }
        Err(ResizeTargetError::EdgeOrPortal) => {
            log::warn!("EnterResizeMode: edge / label / portal selection — not resizable");
        }
    }
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
///
/// Multi-target selections (`Multi(ids)`, `MultiSection`) fan out
/// over every target and join the produced texts with
/// `MULTI_TARGET_SEPARATOR` so the paste path can reverse-split by
/// counting separators. The in-process structured buffer is
/// cleared up-front because a multi-section copy has no payload
/// variant today; a stale single-section payload from a prior
/// copy would otherwise win the byte-equal probe on the next paste.
pub(in crate::application::app) fn apply_copy_or_cut(is_cut: bool, doc: &mut MindMapDocument) -> bool {
    use crate::application::console::traits::{
        selection_targets, view_for, ClipboardContent, HandlesCopy, HandlesCut,
    };
    crate::application::clipboard::clear_section_clipboard();

    let targets = selection_targets(&doc.selection);
    let mut text_payloads: Vec<String> = Vec::new();
    let mut section_texts: Vec<String> = Vec::new();
    let mut first_section_payload: Option<crate::application::document::SectionPayload> = None;
    let mut any_target_accepted = false;
    for tid in &targets {
        let mut view = view_for(doc, tid);
        let content = if is_cut {
            view.clipboard_cut()
        } else {
            view.clipboard_copy()
        };
        match content {
            ClipboardContent::Text(text) => {
                text_payloads.push(text);
                any_target_accepted = true;
            }
            ClipboardContent::Section { text, payload } => {
                if section_texts.is_empty() {
                    first_section_payload = Some(payload);
                }
                section_texts.push(text);
                any_target_accepted = true;
            }
            ClipboardContent::Empty | ClipboardContent::NotApplicable => {}
        }
    }
    if !text_payloads.is_empty() {
        let joined = text_payloads.join(MULTI_TARGET_SEPARATOR);
        crate::application::clipboard::write_clipboard(&joined);
        return any_target_accepted;
    }
    if !section_texts.is_empty() {
        let joined = section_texts.join(MULTI_TARGET_SEPARATOR);
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
    any_target_accepted
}

/// Read the system clipboard and paste into every clipboard-eligible
/// target in the current selection. Triggers a geometry rebuild iff
/// at least one target accepted the paste.
///
/// For multi-target selections, the OS clipboard is split on
/// `MULTI_TARGET_SEPARATOR` and zipped 1:1 when fragment count
/// equals target count (round-trip from a Mandala multi-target
/// copy); otherwise the full blob broadcasts to every target
/// (cross-app paste, count mismatch).
///
/// **Broadcast structured-buffer guard.** When the multi-target
/// path falls back to broadcast, the in-process `SECTION_BUFFER`
/// is cleared up-front. Otherwise a structured payload from a
/// prior single-section copy would survive the byte-equal probe
/// inside each per-target `clipboard_paste` call and silently
/// apply the same `SectionPayload` (offset, size, channel,
/// trigger_bindings) to every target — distinct sections of the
/// same node would end up with identical geometry, corrupting
/// the model.
pub(in crate::application::app) fn apply_paste(rc: &mut RebuildContext<'_>) {
    use crate::application::console::traits::{selection_targets, view_for, HandlesPaste, Outcome};
    let Some(text) = crate::application::clipboard::read_clipboard() else {
        return;
    };
    let targets = selection_targets(&rc.document.selection);
    let fragments = split_paste_for_targets(&text, targets.len());
    if is_broadcast_paste(fragments.is_none(), targets.len()) {
        crate::application::clipboard::clear_section_clipboard();
    }
    let mut any_applied = false;
    for (i, tid) in targets.iter().enumerate() {
        let mut view = view_for(rc.document, tid);
        let to_paste: &str = match &fragments {
            Some(frags) => frags[i],
            None => &text,
        };
        if let Outcome::Applied = view.clipboard_paste(to_paste) {
            any_applied = true;
        }
    }
    if any_applied {
        rc.rebuild_after_geometry_change();
    }
}

/// Decide whether the OS clipboard text should be split 1:1 with
/// the paste targets (round-trip from a Mandala multi-target copy)
/// or broadcast verbatim to every target (cross-app paste / count
/// mismatch). Returns `Some(fragments)` when split-and-zip applies,
/// `None` when the caller should broadcast `text` to every target.
///
/// The split is gated on `target_count > 1` AND
/// `fragments.len() == target_count`. A single `\n\n` in a
/// pasted-from-another-app blob with a 2-target selection that
/// happens to match by coincidence will round-trip; that's
/// indistinguishable from a Mandala copy by content alone, so the
/// behaviour matches what the user typed on the source side.
/// True when a paste must clear the structured section buffer:
/// fragments couldn't be split per-target AND there's more than
/// one target. Without the clear, a stale single-section payload
/// would broadcast its non-text fields to every target.
fn is_broadcast_paste(fragments_is_none: bool, target_count: usize) -> bool {
    fragments_is_none && target_count > 1
}

fn split_paste_for_targets(text: &str, target_count: usize) -> Option<Vec<&str>> {
    if target_count <= 1 {
        return None;
    }
    let fragments: Vec<&str> = text.split(MULTI_TARGET_SEPARATOR).collect();
    if fragments.len() == target_count {
        Some(fragments)
    } else {
        None
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::{apply_copy_or_cut, split_paste_for_targets, MULTI_TARGET_SEPARATOR};
    use crate::application::clipboard::{
        clear_section_clipboard, read_section_clipboard, write_section_clipboard,
    };
    use crate::application::document::tests_common::pinned_two_section_node;
    use crate::application::document::{SectionPayload, SectionSel, SelectionState};

    /// `\n\n` not `\n` — single-newline collisions with
    /// intra-section editor line breaks are the whole reason the
    /// joiner moved to two newlines.
    #[test]
    fn test_multi_target_separator_is_doubled_newline() {
        assert_eq!(MULTI_TARGET_SEPARATOR, "\n\n");
    }

    /// Single-target paste always broadcasts verbatim (the split
    /// gate is `target_count > 1`). Pins that a single section
    /// receiving a `"foo\n\nbar"` blob writes the whole blob, not
    /// just `"foo"`.
    #[test]
    fn test_single_target_skips_split() {
        assert!(split_paste_for_targets("foo\n\nbar", 1).is_none());
        assert!(split_paste_for_targets("foo", 1).is_none());
    }

    /// Multi-target with matching fragment count zips: the round
    /// trip from a 3-target copy lands one fragment per target.
    #[test]
    fn test_matching_fragment_count_zips() {
        let frags = split_paste_for_targets("a\n\nb\n\nc", 3).expect("zip");
        assert_eq!(frags, vec!["a", "b", "c"]);
    }

    /// Mismatched fragment count broadcasts (e.g. cross-app paste
    /// of plain text into a multi-section selection).
    #[test]
    fn test_mismatched_fragment_count_broadcasts() {
        // 1 fragment, 3 targets — broadcast.
        assert!(split_paste_for_targets("plain text", 3).is_none());
        // 2 fragments, 3 targets — broadcast.
        assert!(split_paste_for_targets("a\n\nb", 3).is_none());
        // 4 fragments, 3 targets — broadcast.
        assert!(split_paste_for_targets("a\n\nb\n\nc\n\nd", 3).is_none());
    }

    /// Empty fragments are preserved by `str::split` — a copied
    /// section whose text is empty round-trips to an empty paste,
    /// not a dropped target.
    #[test]
    fn test_empty_fragment_preserved_in_zip() {
        let frags = split_paste_for_targets("a\n\n\n\nc", 3).expect("zip");
        assert_eq!(frags, vec!["a", "", "c"]);
    }

    /// **Stale-buffer guard.** A leftover single-section payload
    /// from a prior copy must not survive a multi-section copy —
    /// otherwise the structured paste path would silently
    /// substitute one section's payload for the joined OS-clipboard
    /// blob. Pins that `apply_copy_or_cut` clears the in-process
    /// buffer up-front.
    #[test]
    fn test_multi_section_copy_clears_stale_section_buffer() {
        clear_section_clipboard();
        // Seed a stale buffer with a probe text the multi-section
        // copy below will NOT regenerate.
        let seed_payload = SectionPayload {
            text_runs: Vec::new(),
            offset: baumhard::mindmap::model::Position { x: 0.0, y: 0.0 },
            size: None,
            channel: None,
            trigger_bindings: Vec::new(),
        };
        write_section_clipboard("stale-probe".into(), seed_payload);
        assert!(read_section_clipboard("stale-probe").is_some());

        // Now do a MultiSection copy — the multi-section branch
        // doesn't write the structured buffer (no MultiSection
        // payload variant exists), so the only correctness gate
        // is the up-front clear.
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::MultiSection(vec![
            SectionSel::new(&id, 0),
            SectionSel::new(&id, 1),
        ]);
        apply_copy_or_cut(false, &mut doc);

        assert!(
            read_section_clipboard("stale-probe").is_none(),
            "stale seeded buffer must be cleared by multi-section copy"
        );
    }

    /// Single-section copy DOES populate the structured buffer so
    /// within-app section→section paste round-trips per-run
    /// formatting. Pins that the up-front clear doesn't break the
    /// single-section structured path.
    #[test]
    fn test_single_section_copy_writes_structured_buffer() {
        clear_section_clipboard();
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel::new(&id, 1));
        let probe_text = doc.mindmap.nodes.get(&id).unwrap().sections[1].text.clone();
        apply_copy_or_cut(false, &mut doc);
        assert!(
            read_section_clipboard(&probe_text).is_some(),
            "single-section copy must populate the structured buffer"
        );
    }

    /// MultiSection copy does NOT populate the structured buffer
    /// (no payload variant) — the within-app paste falls back to
    /// the OS clipboard's joined blob. Pins the
    /// no-payload-on-multi contract that the stale-buffer guard
    /// relies on.
    #[test]
    fn test_multi_section_copy_skips_structured_buffer() {
        clear_section_clipboard();
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::MultiSection(vec![
            SectionSel::new(&id, 0),
            SectionSel::new(&id, 1),
        ]);
        let s0_text = doc.mindmap.nodes.get(&id).unwrap().sections[0].text.clone();
        apply_copy_or_cut(false, &mut doc);
        assert!(
            read_section_clipboard(&s0_text).is_none(),
            "multi-section copy must not populate the structured buffer"
        );
    }

    /// **End-to-end multi-section round-trip.** Copy two sections
    /// (joined to OS clipboard with `\n\n`), then drive the
    /// per-target paste machinery against a *different*
    /// `MultiSection` set with the same cardinality and verify
    /// each section receives the correct fragment 1:1. Pins the
    /// copy → split → paste contract end-to-end (the helper
    /// tests pin each leg in isolation; this fills the
    /// integration gap the cross-cutting reviewer flagged).
    ///
    /// `apply_paste` is bypassed because it needs a full
    /// `RebuildContext` (renderer, scene, tree) — we inline the
    /// pure parts here: `read_clipboard`, `selection_targets`,
    /// `split_paste_for_targets`, and `view.clipboard_paste` per
    /// target. That's the same plumbing `apply_paste` calls; the
    /// only piece skipped is the rebuild side-effect.
    #[test]
    fn test_multi_section_copy_paste_round_trip() {
        use crate::application::clipboard::read_clipboard;
        use crate::application::console::traits::{
            selection_targets, view_for, HandlesPaste, Outcome,
        };

        clear_section_clipboard();
        let (mut doc, id) = pinned_two_section_node();
        // Make the section texts asymmetric so a swap or
        // first-wins bug would be visible in the assertions.
        doc.mindmap.nodes.get_mut(&id).unwrap().sections[0].text = "alpha".into();
        doc.mindmap.nodes.get_mut(&id).unwrap().sections[1].text = "beta".into();
        doc.selection = SelectionState::MultiSection(vec![
            SectionSel::new(&id, 0),
            SectionSel::new(&id, 1),
        ]);
        apply_copy_or_cut(false, &mut doc);

        // Mutate the source sections to make sure the paste
        // overwrites with the captured clipboard content rather
        // than no-op'ing because they happen to match.
        doc.mindmap.nodes.get_mut(&id).unwrap().sections[0].text = "wiped".into();
        doc.mindmap.nodes.get_mut(&id).unwrap().sections[1].text = "wiped".into();

        // Drive the same paste plumbing `apply_paste` runs.
        let Some(text) = read_clipboard() else {
            // CI without a real clipboard — `arboard` returned
            // nothing on read. Skip rather than fail; the
            // helper-level tests still pin the contract.
            return;
        };
        let targets = selection_targets(&doc.selection);
        assert_eq!(targets.len(), 2);
        let fragments = super::split_paste_for_targets(&text, targets.len())
            .expect("count matches → zip applies");
        for (i, tid) in targets.iter().enumerate() {
            let mut view = view_for(&mut doc, tid);
            assert_eq!(
                view.clipboard_paste(fragments[i]),
                Outcome::Applied,
                "per-target paste must apply"
            );
        }

        // Round-trip lands fragments 1:1.
        let s0 = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text;
        let s1 = &doc.mindmap.nodes.get(&id).unwrap().sections[1].text;
        assert_eq!(s0, "alpha", "section 0 must round-trip its own copy");
        assert_eq!(s1, "beta", "section 1 must round-trip its own copy");
    }

    /// **Broadcast structured-buffer guard.** A single-section
    /// copy seeds the in-process `SECTION_BUFFER` with one
    /// `SectionPayload`. A subsequent paste against a
    /// `MultiSection` of size 2 falls through to broadcast
    /// (1 fragment, 2 targets, count mismatch). Without the
    /// `clear_section_clipboard()` call inside `apply_paste`'s
    /// broadcast path, every per-target `clipboard_paste`
    /// would byte-equal-probe the stale buffer and apply the
    /// same `SectionPayload` (offset / size / channel /
    /// `is_broadcast_paste` predicate that gates the
    /// structured-buffer clear inside `apply_paste`. True only
    /// when fragments couldn't be split AND there are 2+
    /// targets — a stale single-section payload would otherwise
    /// broadcast its non-text fields to every target.
    #[test]
    fn test_is_broadcast_paste_predicate() {
        use super::is_broadcast_paste;
        assert!(is_broadcast_paste(true, 2));
        assert!(is_broadcast_paste(true, 5));
        assert!(!is_broadcast_paste(true, 1));
        assert!(!is_broadcast_paste(false, 2));
        assert!(!is_broadcast_paste(false, 1));
    }
}
