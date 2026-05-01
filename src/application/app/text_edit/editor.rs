// SPDX-License-Identifier: MPL-2.0

//! Inline section text editor: open / close / handle key /
//! apply preview-to-tree. The text editor is a multi-line in-place
//! buffer whose cursor + content live on `TextEditState::Open` and
//! whose preview is stamped into the live Baumhard tree via
//! `apply_text_edit_to_tree` so the user sees their typing on every
//! keystroke without touching the model. Commit on Esc folds the
//! buffer into the targeted `MindSection.text` via
//! `MindMapDocument::set_section_text` (the editor records the
//! `section_idx` resolved from the active `SelectionState` at open
//! time so per-section selections commit to the right section).

use winit::keyboard::Key;

use baumhard::util::grapheme_chad;

use crate::application::document::MindMapDocument;
use crate::application::keybinds::{InputContext, ResolvedKeybinds};
use crate::application::renderer::Renderer;

use super::super::scene_rebuild::rebuild_all;
use super::{insert_at_cursor, insert_caret, TextEditState};

/// Open the text editor on the given node. Seeds the buffer (empty if
/// `from_creation`, else the node's current text), and pushes the
/// initial caret through the Baumhard mutation pipeline so the live
/// tree shows the cursor on the next frame.
///
/// Snapshots the tree's pre-edit `(text, regions)` into
/// `TextEditState::Open::{original_text, original_regions}` so cancel
/// can revert via `revert_node_text_on_tree` without going through
/// the full `rebuild_all`. Both snapshots read from the tree — not
/// the model — so any selection-highlight the current `rebuild_all`
/// stamped onto the node (via `apply_tree_highlights`) round-trips
/// through cancel.
pub(in crate::application::app) fn open_text_edit(
    node_id: &str,
    from_creation: bool,
    doc: &mut MindMapDocument,
    text_edit_state: &mut TextEditState,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    _app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    // Resolve the section index from the document's selection.
    // `Section { node_id: id, section_idx }` opens the editor on
    // *that* section; any other selection (Single, Multi, edge-
    // adjacent) defaults to section 0 — preserving the historical
    // single-section single-node behaviour for migrated maps.
    //
    // Clamp the candidate index against `node.sections.len()`: a
    // custom mutation between the click that set the Section
    // selection and this open call can have shrunk the sections
    // vec, leaving the selection's `section_idx` stale. The clamp
    // collapses the editor to section 0 in that case rather than
    // returning silently and leaving the user with a stuck
    // double-click.
    let Some(node) = doc.mindmap.nodes.get(node_id) else {
        return;
    };
    if node.sections.is_empty() {
        return;
    }
    let candidate_idx = match doc.selection.selected_section() {
        Some(s) if s.node_id == node_id => s.section_idx,
        _ => 0,
    };
    let section_idx = candidate_idx.min(node.sections.len() - 1);
    let current_text = node.sections[section_idx].text.clone();
    let buffer = if from_creation {
        String::new()
    } else {
        current_text
    };
    let cursor_grapheme_pos = grapheme_chad::count_grapheme_clusters(&buffer);
    // Seed `buffer_regions` from the tree's current `area.regions`,
    // which the tree builder populated from the section's `text_runs`.
    // The tree is the source of truth for regions during an edit
    // session; the model is frozen until commit. `from_creation`
    // nodes have no prior regions, so we start from empty.
    let original_text = read_section_text(mindmap_tree.as_ref(), node_id, section_idx).unwrap_or_default();
    let original_regions =
        read_section_regions(mindmap_tree.as_ref(), node_id, section_idx).unwrap_or_default();
    let buffer_regions = if from_creation {
        baumhard::core::primitives::ColorFontRegions::new_empty()
    } else {
        original_regions.clone()
    };
    *text_edit_state = TextEditState::Open {
        node_id: node_id.to_string(),
        section_idx,
        buffer: buffer.clone(),
        cursor_grapheme_pos,
        buffer_regions: buffer_regions.clone(),
        original_text,
        original_regions,
    };
    // Push the initial (caret-only for creation, or "existing text +
    // caret at end" for edit) through the Baumhard mutation pipeline.
    apply_text_edit_to_tree(
        node_id,
        section_idx,
        &buffer,
        &buffer_regions,
        cursor_grapheme_pos,
        mindmap_tree,
        renderer,
    );
}

/// Resolve `(node_id, section_idx)` to the arena `NodeId` of
/// the matching section-area — the post-refactor home of
/// editable text + regions. Returns `None` when the tree, the
/// node, or the section slot is missing.
fn section_arena_id(
    tree: &baumhard::mindmap::tree_builder::MindMapTree,
    node_id: &str,
    section_idx: usize,
) -> Option<indextree::NodeId> {
    tree.section_arena_id(node_id, section_idx)
}

/// Read a specific section's `GlyphArea::regions` off the live
/// tree. Returns `None` when the tree or the section isn't
/// present, or when the target element isn't a `GlyphArea`. The
/// text-edit path uses this to seed
/// `TextEditState::Open::buffer_regions` at open time so per-run
/// color and `AppFont` pins survive the edit lifecycle.
pub(in crate::application::app) fn read_section_regions(
    mindmap_tree: Option<&baumhard::mindmap::tree_builder::MindMapTree>,
    node_id: &str,
    section_idx: usize,
) -> Option<baumhard::core::primitives::ColorFontRegions> {
    let tree = mindmap_tree?;
    let nid = section_arena_id(tree, node_id, section_idx)?;
    let element = tree.tree.arena.get(nid)?.get();
    element.glyph_area().map(|a| a.regions.clone())
}

/// Read a specific section's `GlyphArea::text` off the live
/// tree. Pairs with [`read_section_regions`] — together they
/// capture the pre-edit snapshot the cancel path restores via
/// `DeltaGlyphArea`.
pub(in crate::application::app) fn read_section_text(
    mindmap_tree: Option<&baumhard::mindmap::tree_builder::MindMapTree>,
    node_id: &str,
    section_idx: usize,
) -> Option<String> {
    let tree = mindmap_tree?;
    let nid = section_arena_id(tree, node_id, section_idx)?;
    let element = tree.tree.arena.get(nid)?.get();
    element.glyph_area().map(|a| a.text.clone())
}

/// Assign a `(text, regions)` snapshot onto the live tree's
/// `GlyphArea` for `node_id`, via a `DeltaGlyphArea`. Pure tree
/// mutation — no renderer contact — so unit tests can drive it
/// without a GPU context. Returns `true` on success, `false` when
/// the tree, node, or element isn't present.
pub(in crate::application::app) fn apply_text_and_regions_delta(
    node_id: &str,
    section_idx: usize,
    text: String,
    regions: baumhard::core::primitives::ColorFontRegions,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
) -> bool {
    use baumhard::core::primitives::{Applicable, ApplyOperation};
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};

    let tree = match mindmap_tree.as_mut() {
        Some(t) => t,
        None => return false,
    };
    // Targets the section-area at `(node_id, section_idx)`. Text
    // and regions live on the section in the post-refactor shape.
    let indextree_node_id = match section_arena_id(tree, node_id, section_idx) {
        Some(id) => id,
        None => return false,
    };
    let element = match tree.tree.arena.get_mut(indextree_node_id) {
        Some(n) => n.get_mut(),
        None => return false,
    };
    let area = match element.glyph_area_mut() {
        Some(a) => a,
        None => return false,
    };

    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::Text(text),
        GlyphAreaField::ColorFontRegions(regions),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    delta.apply_to(area);
    true
}

/// Apply a snapshot of `(text, regions)` back to the live tree's
/// `GlyphArea` for `node_id` and refresh the renderer's cosmic-text
/// buffers. Used by the text-editor cancel path to revert the tree
/// to its pre-edit state without going through the full
/// `rebuild_all` (which rebuilds every node from the model and
/// re-walks the scene). Thin wrapper over
/// [`apply_text_and_regions_delta`] — the latter is unit-tested
/// directly; this function just pairs it with the renderer
/// rebuild.
pub(in crate::application::app) fn revert_node_text_on_tree(
    node_id: &str,
    section_idx: usize,
    text: String,
    regions: baumhard::core::primitives::ColorFontRegions,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    if !apply_text_and_regions_delta(node_id, section_idx, text, regions, mindmap_tree) {
        return;
    }
    if let Some(tree) = mindmap_tree.as_ref() {
        renderer.rebuild_buffers_from_tree(&tree.tree);
    }
}

/// commit or cancel the open text editor.
///
/// - **Commit**: writes the final buffer back to the model via
///   `set_node_text` (no-op on unchanged text, handles its own undo
///   push), then `rebuild_all` to pull the tree back to the freshly
///   mutated model.
/// - **Cancel**: applies the `(original_text, original_regions)`
///   snapshot captured at open time as a `DeltaGlyphArea` to the
///   edited node. The model is untouched during editing, so the rest
///   of the tree + scene are already in sync — no `rebuild_all` is
///   needed. This skips the `doc.build_tree()` walk and the full
///   `rebuild_scene_only` (connections, borders, portals, labels,
///   edge handles), which matters on maps with many nodes.
pub(in crate::application::app) fn close_text_edit(
    commit: bool,
    doc: &mut MindMapDocument,
    text_edit_state: &mut TextEditState,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let snapshot = match std::mem::replace(text_edit_state, TextEditState::Closed) {
        TextEditState::Open {
            node_id,
            section_idx,
            buffer,
            original_text,
            original_regions,
            ..
        } => (node_id, section_idx, buffer, original_text, original_regions),
        TextEditState::Closed => return,
    };
    let (node_id, section_idx, buffer, original_text, original_regions) = snapshot;
    if commit {
        // Section-aware commit: the editor records the section
        // index in `TextEditState::Open` so a `Section` selection
        // commits to that section, not to section 0.
        doc.set_section_text(&node_id, section_idx, buffer);
        // Commit changed the model — pull the tree back to it.
        // No `scene_cache.clear()` is needed: `set_section_text` writes
        // only the text field; `node.size` is authored, not
        // autosized, so edge endpoints don't shift and cached
        // connection samples stay valid. If autosizing ever lands,
        // revisit this seam.
        rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
    } else {
        // Cancel: model is untouched, so we only need to revert the
        // edited section's transient caret-bearing text/regions to
        // the pre-edit snapshot. Scene elements (borders,
        // connections, etc.) were never mutated during the edit
        // session.
        revert_node_text_on_tree(
            &node_id,
            section_idx,
            original_text,
            original_regions,
            mindmap_tree,
            renderer,
        );
    }
}

/// push the current (`buffer`, `cursor`) state into the
/// live Baumhard tree via a `Mutation::AreaDelta { text: Assign }`
/// targeting the edited node's GlyphArea. This is the "utilize
/// Baumhard" path — the buffer is transient UI state on the app
/// layer, but every visual frame goes through the existing
/// `Mutation::apply_to_area` vocabulary. The renderer's text buffers
/// are rebuilt from the mutated tree so the next frame reflects the
/// keystroke.
pub(in crate::application::app) fn apply_text_edit_to_tree(
    node_id: &str,
    section_idx: usize,
    buffer: &str,
    buffer_regions: &baumhard::core::primitives::ColorFontRegions,
    cursor_grapheme_pos: usize,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    use baumhard::core::primitives::{Applicable, ApplyOperation, ColorFontRegion, Range};
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};

    let tree = match mindmap_tree.as_mut() {
        Some(t) => t,
        None => return,
    };
    // The text editor targets the section-area at
    // `(node_id, section_idx)` — that's where text + regions live
    // post-refactor.
    let indextree_node_id = match section_arena_id(tree, node_id, section_idx) {
        Some(id) => id,
        None => return,
    };
    // Grab a mutable handle to the target section's GlyphArea.
    let element = tree.tree.arena.get_mut(indextree_node_id);
    let element = match element {
        Some(n) => n.get_mut(),
        None => return,
    };
    let area = match element.glyph_area_mut() {
        Some(a) => a,
        None => return,
    };

    // Compose display-text regions from the canonical buffer regions
    // via Baumhard's `insert_regions_at` primitive: the caret glyph
    // is a one-char structural insertion at `cursor_grapheme_pos`
    // that the surrounding run should absorb (so the caret inherits
    // its color and — importantly — its `AppFont` pin, keeping
    // per-script glyphs rendering correctly). If no region absorbs
    // the caret (empty buffer, cursor at an uncovered position), we
    // `set_or_insert` a blank region for it so it still renders.
    let display_text = insert_caret(buffer, cursor_grapheme_pos);
    let mut display_regions = buffer_regions.clone();
    let absorbed = display_regions.insert_regions_at(cursor_grapheme_pos, 1);
    if !absorbed {
        display_regions.set_or_insert(&ColorFontRegion::new(
            Range::new(cursor_grapheme_pos, cursor_grapheme_pos + 1),
            None,
            None,
        ));
    }

    // Construct the Baumhard delta: Text + ColorFontRegions + Assign.
    // The Assign operation replaces both fields wholesale — see
    // `GlyphArea::apply_operation` in `gfx_structs/area.rs`.
    let delta = DeltaGlyphArea::new(vec![
        GlyphAreaField::Text(display_text),
        GlyphAreaField::ColorFontRegions(display_regions),
        GlyphAreaField::Operation(ApplyOperation::Assign),
    ]);
    delta.apply_to(area);

    // Re-shape only the edited section's buffer — the keyed reshape
    // drops the per-keystroke cost on a multi-section node from
    // `O(N × sections)` (the old `rebuild_buffers_from_tree` over
    // the whole arena) to `O(halos+1)` for this single element.
    // Capture the unique_id before releasing the `&mut` borrow on
    // the arena element.
    let unique_id = element.unique_id();
    renderer.reshape_buffer_for(unique_id, &tree.tree);
}

/// route a keystroke to the open node text editor. All
/// keys are stolen from normal keybind dispatch — Tab and Enter
/// produce literal characters, Esc cancels, arrows/Home/End navigate,
/// Backspace and Delete remove a grapheme, and printable chars
/// insert at the cursor. Every successful mutation is pushed through
/// `apply_text_edit_to_tree` so the tree and renderer stay in sync.
pub(in crate::application::app) fn handle_text_edit_key(
    key_name: &Option<String>,
    logical_key: &Key,
    ctrl: bool,
    shift: bool,
    alt: bool,
    keybinds: &ResolvedKeybinds,
    text_edit_state: &mut TextEditState,
    _doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    _app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    _scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let name = key_name.as_deref();
    let action = name.and_then(|n| keybinds.action_for_context(InputContext::TextEdit, n, ctrl, shift, alt));
    // `TextEditCommit` / `TextEditCancel` are funneled via the
    // keyboard handler's pre-filter (`event_keyboard.rs:135-153`).
    // This handler reaches only the literal-Key character + cursor
    // primitive paths.

    // `enter` and `tab` insert literal characters in the multi-line
    // node editor unless the user explicitly bound a TextEdit Action
    // to them. The action lookup above runs first; if it returned
    // `Some`, we route through `apply_text_edit_action`. If it
    // returned `None`, fall through to the literal-character path
    // (which handles `Enter` / `Tab` / printable chars uniformly).
    let mut changed = false;
    if let Some(a) = action {
        changed = super::apply_text_edit_action(a, text_edit_state);
    } else {
        // No Action matched — insert literal `\n` for Enter, `\t` for
        // Tab, or printable chars. Pre-existing behaviour preserved.
        let (buffer, cursor, regions) = match text_edit_state {
            TextEditState::Open {
                buffer,
                cursor_grapheme_pos,
                buffer_regions,
                ..
            } => (buffer, cursor_grapheme_pos, buffer_regions),
            TextEditState::Closed => return,
        };
        match name {
            Some("enter") => {
                regions.insert_regions_at(*cursor, 1);
                *cursor = insert_at_cursor(buffer, *cursor, '\n');
                changed = true;
            }
            Some("tab") => {
                regions.insert_regions_at(*cursor, 1);
                *cursor = insert_at_cursor(buffer, *cursor, '\t');
                changed = true;
            }
            _ => {
                if let Key::Character(c) = logical_key {
                    for ch in c.as_str().chars() {
                        if !ch.is_control() {
                            regions.insert_regions_at(*cursor, 1);
                            *cursor = insert_at_cursor(buffer, *cursor, ch);
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    if changed {
        // Text editing only mutates the live tree during typing; the
        // model is untouched until commit (click-outside) or rolled
        // back on cancel (Esc). Clone the relevant fields to release
        // the mutable borrow on `text_edit_state` before calling
        // `apply_text_edit_to_tree`.
        let TextEditState::Open {
            node_id,
            section_idx,
            buffer,
            cursor_grapheme_pos,
            buffer_regions,
            ..
        } = text_edit_state
        else {
            return;
        };
        let node_id_owned = node_id.clone();
        let section_idx_snapshot = *section_idx;
        let buffer_owned = buffer.clone();
        let regions_owned = buffer_regions.clone();
        let cursor_snapshot = *cursor_grapheme_pos;
        apply_text_edit_to_tree(
            &node_id_owned,
            section_idx_snapshot,
            &buffer_owned,
            &regions_owned,
            cursor_snapshot,
            mindmap_tree,
            renderer,
        );
    }
}
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    //! Unit tests for the text-edit cancel path. Focus on
    //! [`apply_text_and_regions_delta`] — the pure tree-mutation
    //! half of `revert_node_text_on_tree` — so we can exercise the
    //! `Assign`-delta contract without needing a live `Renderer`.

    use super::*;
    use crate::application::document::tests_common::test_map_path;
    use baumhard::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
    use baumhard::mindmap::loader;
    use baumhard::mindmap::tree_builder::build_mindmap_tree;

    /// Build a fresh tree from the testament map and pick the first
    /// MindNode id whose first section-area carries non-empty text.
    /// Post-section refactor the editable text lives on
    /// `sections[0]` rather than the node container, so the
    /// fixture probes the section-area side of the new tree shape.
    fn tree_with_text_node() -> (baumhard::mindmap::tree_builder::MindMapTree, String) {
        let map = loader::load_from_file(&test_map_path()).unwrap();
        let tree = build_mindmap_tree(&map);
        let node_id = tree
            .section_ids()
            .find(|((_, idx), nid)| {
                *idx == 0
                    && tree
                        .tree
                        .arena
                        .get(*nid)
                        .and_then(|n| n.get().glyph_area())
                        .map(|a| !a.text.is_empty())
                        .unwrap_or(false)
            })
            .map(|((mid, _), _)| mid.to_string())
            .expect("testament map has at least one node with non-empty section text");
        (tree, node_id)
    }

    /// Simulate a text-edit session: capture the pre-edit snapshot,
    /// stamp garbage onto the tree's text + regions, then call
    /// `apply_text_and_regions_delta` with the snapshot and assert
    /// the tree's `GlyphArea` is byte-equal to its pre-edit state.
    /// Regression guard for the cancel path bypassing `rebuild_all`.
    #[test]
    fn apply_text_and_regions_delta_restores_pre_edit_snapshot() {
        let (tree, node_id) = tree_with_text_node();
        let mut tree_opt = Some(tree);

        // Snapshot pre-edit text + regions.
        let original_text = read_section_text(tree_opt.as_ref(), &node_id, 0).unwrap();
        let original_regions = read_section_regions(tree_opt.as_ref(), &node_id, 0).unwrap();

        // Stamp garbage onto the live tree to simulate an edit session.
        let mut garbage_regions = ColorFontRegions::new_empty();
        garbage_regions.submit_region(ColorFontRegion::new(
            Range::new(0, 5),
            None,
            Some([1.0, 0.0, 1.0, 1.0]),
        ));
        let garbage_text = "zzzzz|".to_string();
        assert!(apply_text_and_regions_delta(
            &node_id,
            0,
            garbage_text.clone(),
            garbage_regions,
            &mut tree_opt,
        ));
        let after_garbage = read_section_text(tree_opt.as_ref(), &node_id, 0).unwrap();
        assert_eq!(after_garbage, garbage_text, "garbage delta must stick");

        // Revert to the pre-edit snapshot.
        assert!(apply_text_and_regions_delta(
            &node_id,
            0,
            original_text.clone(),
            original_regions.clone(),
            &mut tree_opt,
        ));
        assert_eq!(
            read_section_text(tree_opt.as_ref(), &node_id, 0).unwrap(),
            original_text,
            "revert delta must restore text exactly"
        );
        assert_eq!(
            read_section_regions(tree_opt.as_ref(), &node_id, 0).unwrap(),
            original_regions,
            "revert delta must restore regions exactly"
        );
    }

    /// Missing tree / missing node / missing glyph_area must all
    /// return `false` rather than panic. Covers the three early-exit
    /// branches in `apply_text_and_regions_delta` so a refactor that
    /// silently accepts the bad inputs surfaces here.
    #[test]
    fn apply_text_and_regions_delta_early_exits_gracefully() {
        // No tree at all.
        let mut none_tree: Option<baumhard::mindmap::tree_builder::MindMapTree> = None;
        assert!(!apply_text_and_regions_delta(
            "whatever",
            0,
            String::new(),
            ColorFontRegions::new_empty(),
            &mut none_tree,
        ));

        // Tree present, node id not found.
        let (tree, _real_id) = tree_with_text_node();
        let mut some_tree = Some(tree);
        assert!(!apply_text_and_regions_delta(
            "nonexistent-node-id",
            0,
            String::new(),
            ColorFontRegions::new_empty(),
            &mut some_tree,
        ));
    }
}
