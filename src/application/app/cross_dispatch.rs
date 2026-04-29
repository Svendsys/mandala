// SPDX-License-Identifier: MPL-2.0

//! Cross-platform `Action` arm bodies.
//!
//! Each function here implements one or more `Action::*` variant
//! bodies in a form callable from BOTH the native dispatcher
//! ([`super::dispatch::dispatch_action`]) and the WASM dispatcher
//! ([`super::run_wasm`]). The split exists because the two
//! dispatchers carry different context types — native has 21 fields
//! including console / picker / app_mode / modifiers; WASM has 9
//! fields, a strict subset. Arms whose bodies touch only the
//! shared subset live here; native-only arms stay in
//! [`super::dispatch`].
//!
//! This is the partial-Track-C path documented in
//! [`WASM_CONVERGENCE.md`]: incrementally lift arm bodies as they
//! turn out to need only cross-platform state, without waiting for
//! a full context-type unification. Each migration removes
//! duplication and the "keep in sync" maintenance tax that mirror
//! arms (Path A1) carry.
//!
//! Helpers take a [`RebuildContext`] when they need the rebuild
//! plumbing, or just `&mut Renderer` for renderer-only operations.
//! Both dispatchers construct the right shape at the call site.

use crate::application::common::RenderDecree;
use crate::application::document::{MindMapDocument, SelectionState};
use crate::application::renderer::Renderer;
use crate::application::scene_host::AppScene;
use baumhard::mindmap::scene_cache::SceneConnectionCache;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::scene_rebuild::rebuild_all;

/// Borrowed bundle of the shared rebuild plumbing — the minimum
/// surface every cross-platform mutating Action arm needs.
/// Constructed at the call site from whichever larger context
/// (`InputHandlerContext` on native, `WasmInputState` on WASM)
/// the dispatcher inherits.
pub(in crate::application::app) struct RebuildContext<'a> {
    pub document: &'a mut MindMapDocument,
    pub mindmap_tree: &'a mut Option<MindMapTree>,
    pub app_scene: &'a mut AppScene,
    pub renderer: &'a mut Renderer,
    pub scene_cache: &'a mut SceneConnectionCache,
}

impl<'a> RebuildContext<'a> {
    /// Trigger a full scene rebuild after a **geometry-changing**
    /// document mutation (border / color / font / spacing / edge
    /// type / etc.). Clears the connection sample cache because
    /// edge geometry may have shifted, then rebuilds tree +
    /// app-scene + renderer buffers.
    ///
    /// Use [`Self::rebuild_after_selection_change`] instead when
    /// the only thing that changed is `doc.selection`. Selection
    /// changes don't move edges, so the cached `sample_path`
    /// samples remain valid; clearing the cache forces a
    /// thousand-edge re-sample on every keyboard navigation
    /// keystroke for nothing.
    pub fn rebuild_after_geometry_change(&mut self) {
        self.scene_cache.clear();
        rebuild_all(
            self.document,
            self.mindmap_tree,
            self.app_scene,
            self.renderer,
            self.scene_cache,
        );
    }

    /// Trigger a scene rebuild after a **selection-only** mutation
    /// (`SelectAll`, `JumpToRoot`, `SelectParent`, etc.). Skips
    /// the connection-sample cache clear because edge geometry
    /// hasn't changed — the cache stays valid and per-edge
    /// `sample_path` work is reused on the rebuild. Saves a
    /// noticeable amount of work on dense maps where every key
    /// nav would otherwise force a full re-sample.
    pub fn rebuild_after_selection_change(&mut self) {
        rebuild_all(
            self.document,
            self.mindmap_tree,
            self.app_scene,
            self.renderer,
            self.scene_cache,
        );
    }
}

// ── RebuildContext construction macro ───────────────────────────

/// Build a [`RebuildContext`] from a context-like struct (native
/// [`super::input_context::InputHandlerContext`] or WASM
/// `WasmInputState`) plus an already-unwrapped
/// `&mut MindMapDocument`. Expands inline so the borrow-checker
/// accepts the disjoint per-field borrows; a `fn rebuild_ctx(&mut
/// self, doc)` builder would conflict with the active `doc` borrow
/// the caller's `if let Some(doc) = ctx.document.as_mut()` already
/// holds (re-borrowing `*ctx` while `doc` is live).
///
/// Both dispatchers compress the 6-line struct literal at every
/// rebuilding arm into a single `rebuild_ctx!(ctx, doc)` call.
macro_rules! rebuild_ctx {
    ($ctx:expr, $doc:expr) => {
        $crate::application::app::cross_dispatch::RebuildContext {
            document: $doc,
            mindmap_tree: $ctx.mindmap_tree,
            app_scene: $ctx.app_scene,
            renderer: $ctx.renderer,
            scene_cache: $ctx.scene_cache,
        }
    };
}
pub(in crate::application::app) use rebuild_ctx;

// ── Generic apply-then-rebuild ──────────────────────────────────

/// Run `apply` against the document and trigger a scene rebuild
/// when it returns `true`. Wraps the canonical "call mutation
/// core, conditionally rebuild" shape every parametric `Action`
/// arm uses so both dispatchers can express it as a one-liner.
///
/// Same arm shape as e.g.
/// ```text
/// apply_with_rebuild(&mut rc, |doc|
///     apply_anchor_to_selection(doc, Some(from), Some(to))
/// );
/// ```
pub(in crate::application::app) fn apply_with_rebuild<F>(
    rc: &mut RebuildContext<'_>,
    apply: F,
) where
    F: FnOnce(&mut MindMapDocument) -> bool,
{
    if apply(rc.document) {
        rc.rebuild_after_geometry_change();
    }
}

// ── Document-lifecycle helpers ──────────────────────────────────

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
    text_edit_state: &mut super::TextEditState,
) -> bool {
    let SelectionState::Single(id) = rc.document.selection.clone() else {
        return false;
    };
    super::text_edit::open_text_edit(
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

// ── Parametric Action arms (Compatible) ─────────────────────────
//
// Each thin wrapper takes the typed payload + a `RebuildContext`
// and delegates to the matching mutation core in
// `console/commands/`. The "call core, conditionally rebuild"
// shape every arm uses is centralised in `apply_with_rebuild`
// above. The dispatch arm body in either dispatcher shrinks to
// the wrapper call.
//
// Bodies for these arms are cross-platform — they touch only
// `MindMapDocument` setters, which exist on both targets. The 3
// filesystem variants (`OpenDocument` / `SaveDocumentAs` /
// `NewDocumentAt`) are NativeOnly and don't appear here; their
// arms stay in `dispatch.rs` and route through
// `execute_console_line` for the file I/O plumbing.

pub(in crate::application::app) fn apply_set_edge_anchor(
    from: &str,
    to: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::anchor::apply_anchor_to_selection(
            doc,
            Some(from),
            Some(to),
        )
    });
}

pub(in crate::application::app) fn apply_set_edge_body_glyph(
    preset: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::body::apply_body_glyph_to_selection(doc, preset)
    });
}

pub(in crate::application::app) fn apply_set_border_field(
    field: &str,
    value: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::border::apply_border_field_to_selection(
            doc, field, value,
        )
    });
}

pub(in crate::application::app) fn apply_set_edge_cap(
    from: &str,
    to: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::cap::apply_cap_to_selection(
            doc,
            Some(from),
            Some(to),
        )
    });
}

/// Which color axis a `SetColor*` Action targets. Sibling of
/// [`ZoomDir`] / [`PanDir`] — keeps the dispatcher fan-out typed
/// rather than stringly. Maps to the verb's `bg|text|border` kv
/// key at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum ColorAxis {
    Bg,
    Text,
    Border,
}

impl ColorAxis {
    /// The kv-key string the underlying verb-core accepts. Kept
    /// at the boundary so `apply_color_axis_to_selection` (which
    /// re-uses the verb's `apply_kvs` trait dispatch) doesn't need
    /// to grow a typed surface.
    fn as_kv_key(self) -> &'static str {
        match self {
            ColorAxis::Bg => "bg",
            ColorAxis::Text => "text",
            ColorAxis::Border => "border",
        }
    }
}

pub(in crate::application::app) fn apply_set_color_axis(
    axis: ColorAxis,
    value: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::color::apply_color_axis_to_selection(
            doc,
            axis.as_kv_key(),
            value,
        )
    });
}

pub(in crate::application::app) fn apply_set_edge_type(
    edge_type: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::edge::apply_edge_type_to_selection(doc, edge_type)
    });
}

pub(in crate::application::app) fn apply_set_edge_display_mode(
    mode: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::edge::apply_edge_display_mode_to_selection(
            doc, mode,
        )
    });
}

pub(in crate::application::app) fn apply_reset_edge(
    kind: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::edge::apply_edge_reset_to_selection(doc, kind)
    });
}

pub(in crate::application::app) fn apply_set_font_family(
    family: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::font::apply_font_family_to_selection(doc, family)
    });
}

/// Which font slot a `SetFontSize|Min|Max` Action targets. Sibling
/// of [`ColorAxis`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum FontSlot {
    Size,
    Min,
    Max,
}

impl FontSlot {
    fn as_kv_key(self) -> &'static str {
        match self {
            FontSlot::Size => "size",
            FontSlot::Min => "min",
            FontSlot::Max => "max",
        }
    }
}

/// `pt` is already-parsed (the dispatcher's caller is responsible
/// for parsing the user-facing `String` payload — invalid floats
/// emit a warn-log and skip the helper call entirely).
pub(in crate::application::app) fn apply_set_font_kv(
    slot: FontSlot,
    pt: f32,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::font::apply_font_kv_to_selection(
            doc,
            slot.as_kv_key(),
            pt,
        )
    });
}

pub(in crate::application::app) fn apply_set_edge_label_text(
    text: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::label::apply_label_text_to_selection(doc, text)
    });
}

pub(in crate::application::app) fn apply_set_edge_label_position(
    position: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::label::apply_label_position_to_selection(
            doc, position,
        )
    });
}

pub(in crate::application::app) fn apply_set_spacing(
    input: &str,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::spacing::apply_spacing_to_selection(doc, input)
    });
}

pub(in crate::application::app) fn apply_set_zoom_window(
    min: crate::application::document::OptionEdit<f32>,
    max: crate::application::document::OptionEdit<f32>,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::zoom::apply_zoom_to_selection(doc, min, max)
    });
}

pub(in crate::application::app) fn apply_clear_zoom(rc: &mut RebuildContext<'_>) {
    use crate::application::document::OptionEdit;
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::zoom::apply_zoom_to_selection(
            doc,
            OptionEdit::Clear,
            OptionEdit::Clear,
        )
    });
}

// ── FPS overlay ─────────────────────────────────────────────────

/// Toggle the FPS overlay between `Snapshot` and `Off`. Mirrors
/// `fps on` / `fps off`.
pub(in crate::application::app) fn apply_toggle_fps(renderer: &mut Renderer) {
    use crate::application::common::FpsDisplayMode;
    let next = match renderer.fps_display_mode() {
        FpsDisplayMode::Snapshot => FpsDisplayMode::Off,
        _ => FpsDisplayMode::Snapshot,
    };
    renderer.set_fps_display(next);
}

/// Toggle the FPS overlay between `Debug` and `Off`. Mirrors
/// `fps debug` / `fps off`.
pub(in crate::application::app) fn apply_toggle_fps_debug(renderer: &mut Renderer) {
    use crate::application::common::FpsDisplayMode;
    let next = match renderer.fps_display_mode() {
        FpsDisplayMode::Debug => FpsDisplayMode::Off,
        _ => FpsDisplayMode::Debug,
    };
    renderer.set_fps_display(next);
}

// ── Camera / zoom ───────────────────────────────────────────────

/// Direction of a single keyboard / wheel zoom step. Typed so
/// callers don't have to pass `&Action` and the helper doesn't
/// have to re-match it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum ZoomDir {
    In,
    Out,
}

/// Step zoom toward `(screen_x, screen_y)` (typically the cursor).
/// The factor mirrors the legacy hardcoded wheel handler (1.1×) so
/// wheel-bound `ZoomIn`/`ZoomOut` behave identically across targets.
pub(in crate::application::app) fn apply_zoom_step(
    dir: ZoomDir,
    cursor_pos: (f64, f64),
    renderer: &mut Renderer,
) {
    let factor = match dir {
        ZoomDir::In => 1.1f32,
        ZoomDir::Out => 1.0f32 / 1.1f32,
    };
    renderer.process_decree(RenderDecree::CameraZoom {
        screen_x: cursor_pos.0 as f32,
        screen_y: cursor_pos.1 as f32,
        factor,
    });
}

/// Reset zoom to 1.0 anchored at the screen centre (NOT the
/// cursor). A cursor-anchored zoom emits a `CameraZoom` decree
/// whose canvas-position formula shifts the camera when the focus
/// is off-centre — so a Ctrl+0 with the cursor in a corner would
/// scoot the view by 200+ px instead of cleanly resetting in
/// place. Computing the factor inverse against current zoom keeps
/// the multiplicative path; using screen-centre as the focus
/// cancels the position shift algebraically.
pub(in crate::application::app) fn apply_zoom_reset(renderer: &mut Renderer) {
    let zoom = renderer.camera_zoom().max(f32::EPSILON);
    renderer.process_decree(RenderDecree::CameraZoom {
        screen_x: renderer.surface_width() as f32 * 0.5,
        screen_y: renderer.surface_height() as f32 * 0.5,
        factor: 1.0f32 / zoom,
    });
}

/// Fit the viewport to the loaded tree's bounds. No-op when no
/// tree has been built yet.
pub(in crate::application::app) fn apply_zoom_fit(
    mindmap_tree: &Option<MindMapTree>,
    renderer: &mut Renderer,
) {
    if let Some(tree) = mindmap_tree.as_ref() {
        renderer.fit_camera_to_tree(&tree.tree);
    }
}

/// Direction of a single keyboard pan nudge. Typed so callers
/// don't have to pass `&Action` and the helper doesn't have to
/// re-match it. Geographic compass names mirror the
/// `Action::PanCamera*` variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum PanDir {
    North,
    South,
    East,
    West,
}

/// Keyboard nudge — fixed step in screen pixels, then converted
/// to a `CameraPan` decree like the LeftDrag path emits per cursor
/// move. Step size matches a coarse but perceptible nudge.
pub(in crate::application::app) fn apply_pan_camera(
    dir: PanDir,
    renderer: &mut Renderer,
) {
    const PAN_STEP_PX: f32 = 50.0;
    let (dx, dy) = match dir {
        PanDir::North => (0.0, -PAN_STEP_PX),
        PanDir::South => (0.0, PAN_STEP_PX),
        PanDir::East => (-PAN_STEP_PX, 0.0),
        PanDir::West => (PAN_STEP_PX, 0.0),
    };
    renderer.process_decree(RenderDecree::CameraPan(dx, dy));
}

/// Centre the camera on the centroid of the currently-selected
/// nodes. No-op when nothing is selected (or only an edge /
/// portal-marker selection, which carries no point centroid).
pub(in crate::application::app) fn apply_center_on_selection(
    document: &MindMapDocument,
    renderer: &mut Renderer,
) {
    let ids: Vec<&str> = document.selection.selected_ids();
    if ids.is_empty() {
        return;
    }
    let mut sum = glam::Vec2::ZERO;
    let mut count = 0u32;
    for id in &ids {
        if let Some(node) = document.mindmap.nodes.get(*id) {
            sum += glam::Vec2::new(
                node.position.x as f32 + node.size.width as f32 * 0.5,
                node.position.y as f32 + node.size.height as f32 * 0.5,
            );
            count += 1;
        }
    }
    if count > 0 {
        renderer.set_camera_center(sum / count as f32);
    }
}

// ── Selection ───────────────────────────────────────────────────
//
// Each Action arm is split into a pure-doc inner function (returns
// `bool` for "did the selection change"; cross-platform; unit-tested
// at the bottom of this module) and an outer `apply_*` wrapper that
// triggers the scene rebuild only when the inner function reports a
// change. Per `TEST_CONVENTIONS.md §T8`, renderer-touching outers
// are verified manually; the pure inners carry the test surface.

/// Set the document's selection to every visible node — hidden-by-
/// fold descendants are excluded so a follow-up `DeleteSelection`
/// can't silently nuke subtrees the user can't see. Returns `false`
/// only when the document has no visible nodes (empty doc); does
/// NOT detect "selection was already the same" (would require
/// `SelectionState: PartialEq`, which the enum doesn't derive
/// today). Matches the pre-Track-A unconditional-rebuild
/// behaviour for non-empty docs.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(in crate::application::app) fn select_all_in(doc: &mut MindMapDocument) -> bool {
    let all_ids: Vec<String> = doc
        .mindmap
        .nodes
        .values()
        .filter(|n| !doc.mindmap.is_hidden_by_fold(n))
        .map(|n| n.id.clone())
        .collect();
    if all_ids.is_empty() {
        return false;
    }
    doc.selection = SelectionState::from_ids(all_ids);
    true
}

pub(in crate::application::app) fn apply_select_all(rc: &mut RebuildContext<'_>) {
    if select_all_in(rc.document) {
        rc.rebuild_after_selection_change();
    }
}

/// Clear the selection. Returns `false` (no rebuild needed) when
/// nothing was selected.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(in crate::application::app) fn deselect_all_in(doc: &mut MindMapDocument) -> bool {
    if matches!(doc.selection, SelectionState::None) {
        return false;
    }
    doc.selection = SelectionState::None;
    true
}

pub(in crate::application::app) fn apply_deselect_all(rc: &mut RebuildContext<'_>) {
    if deselect_all_in(rc.document) {
        rc.rebuild_after_selection_change();
    }
}

/// Invert the current node selection. Edge / EdgeLabel / Portal*
/// selections are preserved (their `selected_ids()` is empty, so
/// inverting would collapse to "select every visible node" —
/// unintuitive). Hidden-by-fold nodes are filtered for the same
/// reason as `select_all_in`. Returns `true` when the selection
/// changed.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(in crate::application::app) fn invert_selection_in(doc: &mut MindMapDocument) -> bool {
    let invertable = matches!(
        doc.selection,
        SelectionState::None
            | SelectionState::Single(_)
            | SelectionState::Multi(_)
    );
    if !invertable {
        return false;
    }
    let selected: std::collections::HashSet<String> = doc
        .selection
        .selected_ids()
        .into_iter()
        .map(String::from)
        .collect();
    let inverted: Vec<String> = doc
        .mindmap
        .nodes
        .values()
        .filter(|n| !selected.contains(&n.id) && !doc.mindmap.is_hidden_by_fold(n))
        .map(|n| n.id.clone())
        .collect();
    doc.selection = SelectionState::from_ids(inverted);
    true
}

pub(in crate::application::app) fn apply_invert_selection(rc: &mut RebuildContext<'_>) {
    if invert_selection_in(rc.document) {
        rc.rebuild_after_selection_change();
    }
}

/// Walk one step up the hierarchy from a single-node selection.
/// No-op when the selection isn't a single node or the node has
/// no parent. Returns `true` when the selection changed.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(in crate::application::app) fn select_parent_in(doc: &mut MindMapDocument) -> bool {
    let SelectionState::Single(nid) = doc.selection.clone() else {
        return false;
    };
    let Some(parent_id) = doc
        .mindmap
        .nodes
        .get(&nid)
        .and_then(|n| n.parent_id.clone())
    else {
        return false;
    };
    doc.selection = SelectionState::Single(parent_id);
    true
}

pub(in crate::application::app) fn apply_select_parent(rc: &mut RebuildContext<'_>) {
    if select_parent_in(rc.document) {
        rc.rebuild_after_selection_change();
    }
}

/// Step into the first visible child (id-sorted) of the selected
/// single node. Folded children are skipped — keyboard navigation
/// shouldn't jump into a subtree the user can't see; mirrors the
/// fold-aware click hit-test policy. Returns `true` when the
/// selection changed.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(in crate::application::app) fn select_child_in(doc: &mut MindMapDocument) -> bool {
    let SelectionState::Single(nid) = doc.selection.clone() else {
        return false;
    };
    let Some(child_id) = doc
        .mindmap
        .children_of(&nid)
        .into_iter()
        .find(|c| !doc.mindmap.is_hidden_by_fold(c))
        .map(|c| c.id.clone())
    else {
        return false;
    };
    doc.selection = SelectionState::Single(child_id);
    true
}

pub(in crate::application::app) fn apply_select_child(rc: &mut RebuildContext<'_>) {
    if select_child_in(rc.document) {
        rc.rebuild_after_selection_change();
    }
}

/// Step to the next or previous visible sibling of the selected
/// single node. `forward = true` walks toward the next sibling;
/// `false` walks back. No-op when the selection isn't a single
/// node, or when no visible neighbour exists in the requested
/// direction. Returns `true` when the selection changed.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(in crate::application::app) fn select_sibling_in(
    doc: &mut MindMapDocument,
    forward: bool,
) -> bool {
    let SelectionState::Single(nid) = doc.selection.clone() else {
        return false;
    };
    let Some(target) = sibling_id(&doc.mindmap, &nid, forward) else {
        return false;
    };
    doc.selection = SelectionState::Single(target);
    true
}

pub(in crate::application::app) fn apply_select_sibling(
    forward: bool,
    rc: &mut RebuildContext<'_>,
) {
    if select_sibling_in(rc.document, forward) {
        rc.rebuild_after_selection_change();
    }
}

/// Find the next or previous visible sibling of `nid` under the
/// same parent (or among root nodes when `nid` is a root). Skips
/// folded entries so keyboard navigation matches the fold-aware
/// click hit-test. Returns `None` when `nid` has no visible
/// neighbour in the requested direction.
fn sibling_id(
    map: &baumhard::mindmap::model::MindMap,
    nid: &str,
    forward: bool,
) -> Option<String> {
    let parent_id = map.nodes.get(nid).and_then(|n| n.parent_id.clone());
    let siblings: Vec<(String, bool)> = match parent_id {
        Some(pid) => map
            .children_of(&pid)
            .iter()
            .map(|c| (c.id.clone(), map.is_hidden_by_fold(c)))
            .collect(),
        None => map
            .root_nodes()
            .iter()
            .map(|c| (c.id.clone(), map.is_hidden_by_fold(c)))
            .collect(),
    };
    let idx = siblings.iter().position(|(id, _)| id == nid)?;
    if forward {
        siblings
            .iter()
            .skip(idx + 1)
            .find(|(_, hidden)| !*hidden)
            .map(|(id, _)| id.clone())
    } else {
        siblings
            .iter()
            .take(idx)
            .rev()
            .find(|(_, hidden)| !*hidden)
            .map(|(id, _)| id.clone())
    }
}

/// Set the document's selection to its first root node (id-sorted)
/// and return the canvas-space centre the camera should jump to.
/// Returns `None` when the document is empty (and selection is
/// untouched).
#[must_use = "Some(centre) is the camera target — drop with `let _ = …` to skip the camera move"]
pub(in crate::application::app) fn jump_to_root_in(
    doc: &mut MindMapDocument,
) -> Option<glam::Vec2> {
    let (id, centre) = doc.mindmap.root_nodes().first().map(|n| {
        (
            n.id.clone(),
            glam::Vec2::new(
                n.position.x as f32 + n.size.width as f32 * 0.5,
                n.position.y as f32 + n.size.height as f32 * 0.5,
            ),
        )
    })?;
    doc.selection = SelectionState::Single(id);
    Some(centre)
}

/// Select the document's first root node and centre the camera on
/// it. No-op when the document is empty.
pub(in crate::application::app) fn apply_jump_to_root(rc: &mut RebuildContext<'_>) {
    if let Some(centre) = jump_to_root_in(rc.document) {
        rc.renderer.set_camera_center(centre);
        rc.rebuild_after_selection_change();
    }
}

#[cfg(test)]
mod tests {
    //! Pure-doc-mutation tests for the selection helpers. The
    //! renderer-touching `apply_*` wrappers are out of scope per
    //! `TEST_CONVENTIONS.md §T8` (no live wgpu in unit tests); the
    //! tests below cover the cross-platform inner functions
    //! (`select_all_in`, `deselect_all_in`, etc.) that carry the
    //! actual logic. A regression in any of these silently changes
    //! WASM and native behaviour identically — the type-checker
    //! won't catch it.

    use super::*;
    use crate::application::document::tests_common::load_test_doc;

    fn first_node_id(doc: &MindMapDocument) -> String {
        doc.mindmap
            .nodes
            .keys()
            .next()
            .expect("test fixture has nodes")
            .clone()
    }

    fn first_root_id(doc: &MindMapDocument) -> String {
        doc.mindmap
            .root_nodes()
            .first()
            .expect("test fixture has at least one root")
            .id
            .clone()
    }

    #[test]
    fn select_all_in_picks_every_visible_node() {
        let mut doc = load_test_doc();
        let visible_count = doc
            .mindmap
            .nodes
            .values()
            .filter(|n| !doc.mindmap.is_hidden_by_fold(n))
            .count();
        assert!(visible_count > 0, "fixture has visible nodes");
        let changed = select_all_in(&mut doc);
        assert!(changed);
        let selected = doc.selection.selected_ids();
        assert_eq!(selected.len(), visible_count);
    }

    #[test]
    fn select_all_in_excludes_folded_descendants() {
        let mut doc = load_test_doc();
        // Pick a non-leaf root and fold it.
        let root_id = first_root_id(&doc);
        let descendant_count = doc.mindmap.all_descendants(&root_id).len();
        assert!(
            descendant_count > 0,
            "test fixture root must have descendants",
        );
        doc.mindmap
            .nodes
            .get_mut(&root_id)
            .unwrap()
            .folded = true;
        let total_visible_before_fold = doc.mindmap.nodes.len();
        let _ = select_all_in(&mut doc);
        let selected = doc.selection.selected_ids();
        // The folded root itself is still visible; only its
        // descendants are hidden.
        assert!(selected.iter().any(|id| *id == root_id));
        assert_eq!(selected.len(), total_visible_before_fold - descendant_count);
    }

    #[test]
    fn select_all_in_returns_false_when_no_visible_nodes() {
        let mut doc = MindMapDocument::new_blank(None);
        // Empty document → no visible nodes → no-op + false.
        assert!(!select_all_in(&mut doc));
        assert!(matches!(doc.selection, SelectionState::None));
    }

    #[test]
    fn deselect_all_in_clears_selection() {
        let mut doc = load_test_doc();
        doc.selection = SelectionState::Single(first_node_id(&doc));
        assert!(deselect_all_in(&mut doc));
        assert!(matches!(doc.selection, SelectionState::None));
    }

    #[test]
    fn deselect_all_in_returns_false_when_already_none() {
        let mut doc = load_test_doc();
        // Default selection is None.
        assert!(!deselect_all_in(&mut doc));
    }

    #[test]
    fn invert_selection_in_skips_edge_selection() {
        let mut doc = load_test_doc();
        let edge = doc
            .mindmap
            .edges
            .first()
            .expect("fixture has edges")
            .clone();
        let er = crate::application::document::EdgeRef::new(
            &edge.from_id,
            &edge.to_id,
            &edge.edge_type,
        );
        doc.selection = SelectionState::Edge(er.clone());
        // Edge selections are NOT invertable — the helper preserves
        // them (selecting "every visible node" via inversion would
        // be unintuitive).
        assert!(!invert_selection_in(&mut doc));
        assert!(matches!(doc.selection, SelectionState::Edge(_)));
    }

    #[test]
    fn invert_selection_in_inverts_node_selection() {
        let mut doc = load_test_doc();
        let pivot = first_node_id(&doc);
        doc.selection = SelectionState::Single(pivot.clone());
        assert!(invert_selection_in(&mut doc));
        // Pivot is no longer in the selection.
        assert!(!doc.selection.selected_ids().iter().any(|id| **id == pivot));
        // Every other visible node IS in the selection.
        let expected = doc
            .mindmap
            .nodes
            .values()
            .filter(|n| n.id != pivot && !doc.mindmap.is_hidden_by_fold(n))
            .count();
        assert_eq!(doc.selection.selected_ids().len(), expected);
    }

    #[test]
    fn select_parent_in_walks_up_one_level() {
        let mut doc = load_test_doc();
        // Pick a non-root node to start.
        let child_id = doc
            .mindmap
            .nodes
            .values()
            .find(|n| n.parent_id.is_some())
            .expect("fixture has a non-root node")
            .id
            .clone();
        let parent_id = doc.mindmap.nodes[&child_id]
            .parent_id
            .clone()
            .unwrap();
        doc.selection = SelectionState::Single(child_id);
        assert!(select_parent_in(&mut doc));
        assert!(matches!(
            doc.selection,
            SelectionState::Single(ref s) if s == &parent_id
        ));
    }

    #[test]
    fn select_parent_in_no_op_at_root() {
        let mut doc = load_test_doc();
        let root_id = first_root_id(&doc);
        doc.selection = SelectionState::Single(root_id.clone());
        // Roots have no parent — no-op + false.
        assert!(!select_parent_in(&mut doc));
        assert!(matches!(
            doc.selection,
            SelectionState::Single(ref s) if s == &root_id
        ));
    }

    #[test]
    fn select_parent_in_no_op_for_multi_selection() {
        let mut doc = load_test_doc();
        let ids: Vec<String> = doc.mindmap.nodes.keys().take(2).cloned().collect();
        doc.selection = SelectionState::from_ids(ids);
        assert!(!select_parent_in(&mut doc));
    }

    #[test]
    fn select_child_in_steps_into_first_visible_child() {
        let mut doc = load_test_doc();
        let parent_id = doc
            .mindmap
            .nodes
            .values()
            .find(|n| !doc.mindmap.children_of(&n.id).is_empty())
            .expect("fixture has a parent node")
            .id
            .clone();
        let expected_child = doc
            .mindmap
            .children_of(&parent_id)
            .into_iter()
            .find(|c| !doc.mindmap.is_hidden_by_fold(c))
            .expect("at least one visible child")
            .id
            .clone();
        doc.selection = SelectionState::Single(parent_id);
        assert!(select_child_in(&mut doc));
        assert!(matches!(
            doc.selection,
            SelectionState::Single(ref s) if s == &expected_child
        ));
    }

    #[test]
    fn select_child_in_no_op_for_leaf() {
        let mut doc = load_test_doc();
        // Find a node with no children.
        let leaf_id = doc
            .mindmap
            .nodes
            .values()
            .find(|n| doc.mindmap.children_of(&n.id).is_empty())
            .expect("fixture has a leaf")
            .id
            .clone();
        doc.selection = SelectionState::Single(leaf_id);
        assert!(!select_child_in(&mut doc));
    }

    #[test]
    fn select_sibling_in_walks_visible_neighbour() {
        let mut doc = load_test_doc();
        // Find a node with at least one sibling.
        let (start_id, _next_id) = doc
            .mindmap
            .nodes
            .values()
            .filter_map(|n| {
                let parent = n.parent_id.as_ref()?;
                let siblings = doc.mindmap.children_of(parent);
                if siblings.len() < 2 {
                    return None;
                }
                let idx = siblings.iter().position(|s| s.id == n.id)?;
                let next = siblings.get(idx + 1)?.id.clone();
                Some((n.id.clone(), next))
            })
            .next()
            .expect("fixture has at least one node with a next sibling");
        doc.selection = SelectionState::Single(start_id);
        assert!(select_sibling_in(&mut doc, true));
        // Walking back returns to the previous sibling.
        assert!(select_sibling_in(&mut doc, false));
    }

    #[test]
    fn jump_to_root_in_returns_first_root_centre_and_selects_it() {
        let mut doc = load_test_doc();
        let expected_root = first_root_id(&doc);
        let centre = jump_to_root_in(&mut doc).expect("non-empty fixture");
        assert!(centre.x.is_finite() && centre.y.is_finite());
        assert!(matches!(
            doc.selection,
            SelectionState::Single(ref s) if s == &expected_root
        ));
    }

    #[test]
    fn jump_to_root_in_returns_none_for_empty_doc() {
        let mut doc = MindMapDocument::new_blank(None);
        assert!(jump_to_root_in(&mut doc).is_none());
        assert!(matches!(doc.selection, SelectionState::None));
    }
}
