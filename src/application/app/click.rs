// SPDX-License-Identifier: MPL-2.0

//! Click-target handlers for the native event loop: default click,
//! reparent-target click, connect-target click, plus the
//! mode-aware variant of `rebuild_all`. WASM is gated out at the
//! parent module's `#[cfg]`.

#![cfg(not(target_arch = "wasm32"))]

use baumhard::mindmap::custom_mutation::PlatformContext;

use super::click_triggers::fire_onclick_triggers;
use super::scene_rebuild::{rebuild_all, rebuild_scene_only};
use super::{now_ms, InteractionMode, EDGE_HIT_TOLERANCE_PX};
use crate::application::document::{
    apply_tree_highlights, hit_test_edge, MindMapDocument, SectionSel, SelectionState,
    REPARENT_SOURCE_COLOR, REPARENT_TARGET_COLOR,
};
use crate::application::renderer::Renderer;

/// Handle a click event: update selection, rebuild tree with highlight.
/// When the node hit test misses, falls through to edge hit testing so
/// the user can click on a connection path to select it. If the clicked
/// node has an `OnClick` trigger binding, the bound custom mutation fires
/// (both node mutations and any document actions) after the selection
/// update.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn handle_click(
    hit: Option<String>,
    hit_section: Option<usize>,
    cursor_pos: (f64, f64),
    shift_pressed: bool,
    document: &mut Option<MindMapDocument>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let doc = match document.as_mut() {
        Some(d) => d,
        None => return,
    };

    // OnClick triggers fire before the selection update so that
    // document actions (theme switches etc.) take effect before
    // the scene rebuild below picks up the new state.
    if let Some(id) = hit.as_ref() {
        fire_onclick_triggers(
            doc, mindmap_tree, scene_cache, id, hit_section,
            PlatformContext::Desktop, now_ms() as u64,
        );
    }

    // Update selection state
    match (&hit, shift_pressed) {
        // Click on a specific section in a multi-section node:
        // route to `SelectionState::Section` so per-section verbs
        // (text edit, font, color) target that section. Single-
        // section nodes always have `hit_section = None` from
        // `hit_test_target`, falling through to the
        // whole-node-Single arm below.
        (Some(id), false) => {
            if let Some(section_idx) = hit_section {
                doc.selection = SelectionState::Section(SectionSel {
                    node_id: id.clone(),
                    section_idx,
                });
            } else {
                doc.selection = SelectionState::Single(id.clone());
            }
        }
        (Some(id), true) => {
            // Shift+click: toggle node — or section, when the
            // hit lands on a specific section in a multi-section
            // node — in/out of the multi-selection.
            if let Some(section_idx) = hit_section {
                // Section-side shift+click: extends Section ↔
                // MultiSection. Cross-node section sets are
                // legal; the dedup'd-by-(node_id, section_idx)
                // identity is the load-bearing invariant.
                let new_sec = SectionSel {
                    node_id: id.clone(),
                    section_idx,
                };
                match &doc.selection {
                    SelectionState::Section(existing) if existing == &new_sec => {
                        // Toggle off — same section re-clicked.
                        doc.selection = SelectionState::None;
                    }
                    SelectionState::Section(existing) => {
                        doc.selection =
                            SelectionState::MultiSection(vec![existing.clone(), new_sec]);
                    }
                    SelectionState::MultiSection(existing) => {
                        let mut secs = existing.clone();
                        if let Some(pos) = secs.iter().position(|s| s == &new_sec) {
                            secs.remove(pos);
                            doc.selection = SelectionState::from_sections(secs);
                        } else {
                            secs.push(new_sec);
                            doc.selection = SelectionState::MultiSection(secs);
                        }
                    }
                    _ => {
                        // From any non-section state, shift+click
                        // on a section starts a fresh `Section`
                        // selection — gives the user a clean path
                        // to build a MultiSection by additional
                        // shift+clicks.
                        doc.selection = SelectionState::Section(new_sec);
                    }
                }
            } else {
            // Whole-node shift+click — existing behaviour
            // (toggle node in/out of Multi).
            match &doc.selection {
                // Any non-Single selection collapses to a fresh
                // Single on shift+click of a different node —
                // the user's intent is "start tracking this
                // node" rather than "extend whatever set was
                // here."
                SelectionState::None
                | SelectionState::Edge(_)
                | SelectionState::EdgeLabel(_)
                | SelectionState::PortalLabel(_)
                | SelectionState::PortalText(_)
                | SelectionState::Section(_)
                | SelectionState::MultiSection(_)
                | SelectionState::SectionRange { .. } => {
                    doc.selection = SelectionState::Single(id.clone());
                }
                SelectionState::Single(existing) => {
                    if existing == id {
                        doc.selection = SelectionState::None;
                    } else {
                        doc.selection = SelectionState::Multi(vec![existing.clone(), id.clone()]);
                    }
                }
                SelectionState::Multi(existing) => {
                    let mut ids = existing.clone();
                    if let Some(pos) = ids.iter().position(|i| i == id) {
                        ids.remove(pos);
                        doc.selection = SelectionState::from_ids(ids);
                    } else {
                        ids.push(id.clone());
                        doc.selection = SelectionState::Multi(ids);
                    }
                }
            }
            }
        }
        (None, false) => {
            // Node miss — fall through: first try portal markers
            // (label glyphs attached to their endpoint nodes),
            // then edge hit testing, then finally deselect. A
            // portal-marker click selects the specific label
            // via `SelectionState::PortalLabel { .. }` so wheel
            // / copy / paste / cut / drag all operate on just
            // that endpoint's state; double-click is handled
            // separately by the event loop and pans the camera
            // to the opposite endpoint.
            let canvas_pos = renderer.screen_to_canvas(cursor_pos.0 as f32, cursor_pos.1 as f32);
            // Portal sub-part precedence: text first, icon next.
            // Text and icon AABBs don't overlap in practice (text
            // sits beside the icon along the border normal), so
            // only one of these hits at a time — the ordering
            // keeps routing deterministic even if future layout
            // changes make them adjacent.
            if let Some((edge_key, endpoint)) = renderer.hit_test_portal_text(canvas_pos) {
                doc.selection = SelectionState::PortalText(crate::application::document::PortalLabelSel {
                    edge_key,
                    endpoint_node_id: endpoint,
                });
            } else if let Some((edge_key, endpoint)) = renderer.hit_test_portal(canvas_pos) {
                doc.selection = SelectionState::PortalLabel(crate::application::document::PortalLabelSel {
                    edge_key,
                    endpoint_node_id: endpoint,
                });
            } else {
                let tolerance = EDGE_HIT_TOLERANCE_PX * renderer.canvas_per_pixel();
                let edge_hit = hit_test_edge(canvas_pos, &doc.mindmap, tolerance);
                doc.selection = match edge_hit {
                    Some(edge_ref) => SelectionState::Edge(edge_ref),
                    None => SelectionState::None,
                };
            }
        }
        (None, true) => {
            // Shift+click on empty space: keep current selection (no edge
            // hit test — shift is reserved for multi-node).
        }
    }

    // Rebuild tree with selection highlight applied
    rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
}

/// Rebuild tree, connections, and borders like `rebuild_all`, but additionally
/// overlays reparent-mode highlights on top of the normal selection highlight.
/// `hovered_node` is the node currently under the cursor (highlighted green as
/// the drop target) when in reparent mode; it is ignored in Normal mode.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn rebuild_all_with_mode(
    doc: &MindMapDocument,
    interaction_mode: &InteractionMode,
    hovered_node: Option<&str>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let mut new_tree = doc.build_tree();

    // Build a single flat list of (mind_node_id, color) pairs that
    // `apply_tree_highlights` applies via baumhard's mutator/walker.
    // Order matters: later entries override earlier ones via the
    // repeated `SetRegionColor` mutation, so selection (cyan) is
    // listed first, then mode-specific source (orange), then the
    // hovered target (green). This matches the previous behavior
    // where reparent_source_highlight was documented to override
    // selection_highlight on conflict.
    // Highlight tuples are `(node_id, section_idx?, color)`. A
    // Section / MultiSection narrow the highlight to the
    // selected sections only; mode-driven Reparent / Connect
    // highlights always paint every section (the gesture is
    // whole-node). Routes through the canonical
    // `selection_highlight_entries` helper so the three
    // selection-rebuild sites (here, `rebuild_all`, and the
    // threshold-cross promotion's `rebuild_selection_highlight`)
    // share one mapping.
    let mut highlights = super::scene_rebuild::selection_highlight_entries(&doc.selection);
    match interaction_mode {
        InteractionMode::Reparent { sources } => {
            for s in sources {
                highlights.push((s.as_str(), None, REPARENT_SOURCE_COLOR));
            }
            if let Some(h) = hovered_node {
                if !sources.iter().any(|s| s == h) {
                    highlights.push((h, None, REPARENT_TARGET_COLOR));
                }
            }
        }
        InteractionMode::Connect { source } => {
            highlights.push((source.as_str(), None, REPARENT_SOURCE_COLOR));
            if let Some(h) = hovered_node {
                if h != source {
                    highlights.push((h, None, REPARENT_TARGET_COLOR));
                }
            }
        }
        // Default / NodeEdit / Resize do not contribute mode-specific
        // highlights (NodeEdit dimming + Resize tinting land in
        // Batches 2-3 of SECTIONS_BORDERS_RESIZE_PLAN.md and use
        // separate scene-builder seams).
        InteractionMode::Default | InteractionMode::NodeEdit { .. } | InteractionMode::Resize { .. } => {}
    }
    apply_tree_highlights(&mut new_tree, highlights);
    renderer.rebuild_buffers_from_tree(&new_tree.tree);

    rebuild_scene_only(doc, app_scene, renderer, scene_cache);

    *mindmap_tree = Some(new_tree);
}

// `handle_connect_target_click` / `handle_reparent_target_click`
// removed — the click handler dispatches through the funnel as
// `Action::ConnectToTarget(target_id)` / `Action::ReparentToTarget(target)`.
// See `dispatch.rs`'s arms.
