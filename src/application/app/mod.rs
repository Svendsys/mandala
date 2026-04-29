// SPDX-License-Identifier: MPL-2.0

use std::collections::HashMap;
use std::sync::Arc;

mod scene_rebuild;
mod text_edit;

// Cross-platform — Action arm bodies that touch only state shared
// between native and WASM. Both `dispatch::dispatch_action` and
// `run_wasm`'s inline match call into here. See
// `WASM_CONVERGENCE.md` "partial Track C" for the rationale.
mod cross_dispatch;

// Cross-platform — `dispatch_macro` step loop + privilege gate,
// abstracted over `MacroDispatchTarget`. Native and WASM each
// provide an impl that wraps their respective context type.
// Lifted from `dispatch.rs::dispatch_macro` to single-source the
// privilege gate. See `WASM_CONVERGENCE.md` Track B.
mod dispatch_macro_core;

// Native-only — interactive modal state machines absent on WASM.
// See CLAUDE.md "Dual-target status".
#[cfg(not(target_arch = "wasm32"))]
mod click;
#[cfg(not(target_arch = "wasm32"))]
mod color_picker_flow;
#[cfg(not(target_arch = "wasm32"))]
mod console_input;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod dispatch;
#[cfg(not(target_arch = "wasm32"))]
mod drain_frame;
#[cfg(not(target_arch = "wasm32"))]
mod edge_drag;
#[cfg(not(target_arch = "wasm32"))]
mod edge_label_drag;
#[cfg(not(target_arch = "wasm32"))]
mod event_cursor_moved;
#[cfg(not(target_arch = "wasm32"))]
mod event_keyboard;
#[cfg(not(target_arch = "wasm32"))]
mod event_mouse_click;
#[cfg(not(target_arch = "wasm32"))]
mod freeze_watchdog;
#[cfg(not(target_arch = "wasm32"))]
mod input_context;
#[cfg(not(target_arch = "wasm32"))]
mod portal_label_drag;
#[cfg(not(target_arch = "wasm32"))]
mod label_edit;
#[cfg(not(target_arch = "wasm32"))]
mod run_native;
#[cfg(not(target_arch = "wasm32"))]
mod run_native_init;
#[cfg(not(target_arch = "wasm32"))]
mod throttled_interaction;
#[cfg(target_arch = "wasm32")]
mod run_wasm;

// FIELD COUNT: `InputHandlerContext` has 21 fields. Drift surface for
// new fields:
//   1. The struct in `app/input_context.rs`.
//   2. The `InitState::input_context()` builder in `run_native.rs`.
//   3. `dispatch_action`'s signature in `app/dispatch.rs` (the funnel
//      every handler ultimately calls).
// Input handlers (`event_keyboard.rs`, `event_mouse_click.rs`,
// `event_cursor_moved.rs`) take `ctx: &mut InputHandlerContext<'_>`
// and access fields via `ctx.foo`, so adding a field doesn't ripple
// through their bodies — Rust's split borrows let modal handlers
// receive `&mut ctx.console_state` etc. without re-destructuring.

// Cross-platform imports.
use scene_rebuild::{
    flush_canvas_scene_buffers, rebuild_after_selection_change, rebuild_all,
    rebuild_scene_only, update_border_tree_static,
    update_connection_label_tree, update_connection_tree, update_edge_handle_tree,
    update_portal_tree,
};
use text_edit::{close_text_edit, handle_text_edit_key, insert_at_cursor, open_text_edit, TextEditState};

#[cfg(not(target_arch = "wasm32"))]
use click::{
    handle_click, handle_connect_target_click, handle_reparent_target_click,
    rebuild_all_with_mode,
};
#[cfg(not(target_arch = "wasm32"))]
use color_picker_flow::{
    end_color_picker_gesture, handle_color_picker_click, handle_color_picker_key,
    handle_color_picker_mouse_move, rebuild_color_picker_overlay,
};
#[cfg(not(target_arch = "wasm32"))]
use console_input::{
    handle_console_key, load_console_history, save_console_history,
};
#[cfg(not(target_arch = "wasm32"))]
use edge_drag::apply_edge_handle_drag;
#[cfg(not(target_arch = "wasm32"))]
use label_edit::{
    close_label_edit, close_portal_text_edit, handle_label_edit_key,
    handle_portal_text_edit_key, open_label_edit, open_portal_text_edit,
    LabelEditState, PortalTextEditState,
};
#[cfg(not(target_arch = "wasm32"))]
use pollster::block_on;
#[cfg(not(target_arch = "wasm32"))]
use portal_label_drag::apply_portal_label_drag;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// Cross-platform monotonic clock in ms since first call. Native:
/// `Instant`. WASM: `performance.now()` (≥1ms quantised; fine for
/// the 400ms double-click window).
#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> f64 {
    use std::sync::OnceLock;
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

#[cfg(target_arch = "wasm32")]
fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}
use glam::Vec2;
use wgpu::Instance;
use winit::event::{ElementState, Event, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ControlFlow;
use winit::keyboard::ModifiersState;
use winit::window::CursorIcon;
use winit::{event_loop::EventLoop, window::Window};

use crate::application::common::{InputMode, RenderDecree, WindowMode};
#[cfg(not(target_arch = "wasm32"))]
use crate::application::console::ConsoleState;
use crate::application::document::{
    apply_drag_delta, apply_tree_highlights, hit_test, hit_test_edge,
    rect_select, EdgeRef, MindMapDocument,
    SelectionState,
    UndoAction,
    HIGHLIGHT_COLOR, REPARENT_SOURCE_COLOR, REPARENT_TARGET_COLOR,
};
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::renderer::Renderer;
#[cfg(not(target_arch = "wasm32"))]
use throttled_interaction::ThrottledDrag;

#[cfg(not(target_arch = "wasm32"))]
use baumhard::mindmap::custom_mutation::{PlatformContext, Trigger};

/// Screen-space click tolerance (in pixels) for edge hit testing. Converted
/// to canvas units via `Renderer::canvas_per_pixel()` so the click target
/// stays visually stable across zoom levels.
#[cfg(not(target_arch = "wasm32"))]
const EDGE_HIT_TOLERANCE_PX: f32 = 8.0;

/// Screen-space click tolerance (in pixels) for edge grab-handle hit
/// testing. Slightly larger than the edge-path tolerance above
/// because handles are point-like and need a bit more grab-area
/// to feel forgiving.
#[cfg(not(target_arch = "wasm32"))]
const EDGE_HANDLE_HIT_TOLERANCE_PX: f32 = 12.0;


/// What a single click targeted. Used by [`LastClick`] + the
/// double-click detector so a portal-marker double-click (navigate)
/// is distinguishable from a node double-click (edit text) and from
/// empty-space double-click (create orphan). Two clicks "match" as
/// a double-click only when they have the same `ClickHit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClickHit {
    /// No node and no portal marker under the cursor. Empty-canvas
    /// double-click creates a new orphan unless an edge is selected.
    Empty,
    /// Cursor is inside node `id`'s AABB.
    Node(String),
    /// Cursor is inside a portal **icon** marker. `edge` identifies
    /// the owning portal-mode edge; `endpoint` is the node the
    /// hit marker sits above (the double-click pan target is the
    /// *other* endpoint).
    PortalMarker {
        edge: baumhard::mindmap::scene_cache::EdgeKey,
        endpoint: String,
    },
    /// Cursor is inside a portal **text** label — the glyph area
    /// sitting alongside a portal icon. Routes to
    /// `SelectionState::PortalText`, distinct from the icon so
    /// per-channel operations (color / font) target only the
    /// clicked sub-part. Double-click inherits the same
    /// pan-to-partner behaviour as `PortalMarker` — the
    /// endpoint identity is shared between icon and text.
    PortalText {
        edge: baumhard::mindmap::scene_cache::EdgeKey,
        endpoint: String,
    },
    /// Cursor is inside a line-mode edge's **label** AABB.
    /// Routes to `SelectionState::EdgeLabel` on single click so
    /// color / font / copy operations target the label instead
    /// of the edge body; double-click opens the inline label
    /// editor, matching the "click to select, dbl to edit"
    /// idiom the `Node` variant already follows.
    EdgeLabel(baumhard::mindmap::scene_cache::EdgeKey),
}

/// Records the previous left-click's time, screen position, and hit
/// target so a second click within a short time + distance window
/// is recognized as a double-click. Double-click fires on the second
/// `Pressed` event, not the second release. `time` is `f64`
/// milliseconds from the cross-platform `now_ms()` helper.
#[derive(Debug, Clone)]
struct LastClick {
    time: f64,
    screen_pos: (f64, f64),
    /// What the first click landed on. Two clicks whose `hit`
    /// values are equal (see [`ClickHit::PartialEq`]) qualify as a
    /// double-click.
    hit: ClickHit,
}

/// Double-click window in milliseconds. Matches GNOME/winit convention.
const DOUBLE_CLICK_MS: f64 = 400.0;

/// Double-click maximum distance² in screen-space pixels.
const DOUBLE_CLICK_DIST_SQ: f64 = 16.0 * 16.0;


/// Returns `true` when a new click-down qualifies as a double-click
/// given the previous click. Pure helper so cursor/time math can be
/// unit-tested without a winit event loop.
fn is_double_click(
    prev: &LastClick,
    new_time_ms: f64,
    new_screen_pos: (f64, f64),
    new_hit: &ClickHit,
) -> bool {
    let elapsed = new_time_ms - prev.time;
    if elapsed < 0.0 || elapsed >= DOUBLE_CLICK_MS {
        return false;
    }
    let dx = new_screen_pos.0 - prev.screen_pos.0;
    let dy = new_screen_pos.1 - prev.screen_pos.1;
    if dx * dx + dy * dy >= DOUBLE_CLICK_DIST_SQ {
        return false;
    }
    &prev.hit == new_hit
}

/// Bag of "what was hit" that the click dispatch on both
/// platforms needs. The collapsed `click_hit` is what
/// double-click detection compares against; the four
/// individual `Option`s are what the editor-state guards
/// (already-editing-same-target) and the WASM
/// `pending_click` snapshot consume — those checks need the
/// underlying hits, not just the collapsed enum.
pub(super) struct ClickHitParts {
    pub(super) click_hit: ClickHit,
    pub(super) hit_node: Option<String>,
    pub(super) portal_text_hit:
        Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    pub(super) portal_icon_hit:
        Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    pub(super) edge_label_hit:
        Option<baumhard::mindmap::scene_cache::EdgeKey>,
}

/// Pure router for "what did this click target?". Runs the
/// node → portal-text → portal-icon → edge-label priority
/// chain and folds the four hits into a single
/// [`ClickHitParts`]. Both the native click handler and the
/// WASM click handler previously open-coded byte-identical
/// versions of this body — they now both call here.
///
/// Priority rationale: node hits beat portal hits (a node
/// under a portal marker is the more common target).
/// Portal sub-parts are resolved text-first, then icon — the
/// two AABBs don't overlap in practice but the ordering keeps
/// routing deterministic if geometry ever places them
/// adjacent. Edge-label hits only register when no node /
/// portal sub-part has claimed the click — labels sit along
/// the connection path, and placing them behind the portal
/// check keeps the portal's "floating over a node" behaviour
/// correct even if a label happens to overlap.
pub(super) fn compute_click_hit(
    canvas_pos: glam::Vec2,
    mindmap_tree: Option<&mut baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &crate::application::renderer::Renderer,
) -> ClickHitParts {
    let hit_node = mindmap_tree
        .and_then(|tree| crate::application::document::hit_test(canvas_pos, tree));

    let portal_text_hit = if hit_node.is_none() {
        renderer.hit_test_portal_text(canvas_pos)
    } else {
        None
    };
    let portal_icon_hit =
        if hit_node.is_none() && portal_text_hit.is_none() {
            renderer.hit_test_portal(canvas_pos)
        } else {
            None
        };
    let edge_label_hit = if hit_node.is_none()
        && portal_text_hit.is_none()
        && portal_icon_hit.is_none()
    {
        renderer.hit_test_any_edge_label(canvas_pos)
    } else {
        None
    };

    let click_hit = click_hit_from_priority(
        &hit_node,
        &portal_text_hit,
        &portal_icon_hit,
        &edge_label_hit,
    );

    ClickHitParts {
        click_hit,
        hit_node,
        portal_text_hit,
        portal_icon_hit,
        edge_label_hit,
    }
}

/// Pure priority-ladder for `ClickHit` construction. Given the
/// four already-resolved hit options, returns the highest-priority
/// `ClickHit` variant that's `Some`. Priority order: node beats
/// portal-text beats portal-icon beats edge-label beats empty.
///
/// Separated from [`compute_click_hit`] so the priority contract
/// can be unit-tested without a `Renderer`. The cascade gating
/// inside `compute_click_hit` already guarantees that at most one
/// of the lower-priority options is `Some` at a time, but this
/// ladder remains correct when callers pass overlapping hits — the
/// ladder is the canonical tie-breaker.
fn click_hit_from_priority(
    hit_node: &Option<String>,
    portal_text_hit: &Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    portal_icon_hit: &Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    edge_label_hit: &Option<baumhard::mindmap::scene_cache::EdgeKey>,
) -> ClickHit {
    if let Some(id) = hit_node {
        ClickHit::Node(id.clone())
    } else if let Some((key, ep)) = portal_text_hit {
        ClickHit::PortalText {
            edge: key.clone(),
            endpoint: ep.clone(),
        }
    } else if let Some((key, ep)) = portal_icon_hit {
        ClickHit::PortalMarker {
            edge: key.clone(),
            endpoint: ep.clone(),
        }
    } else if let Some(key) = edge_label_hit {
        ClickHit::EdgeLabel(key.clone())
    } else {
        ClickHit::Empty
    }
}

/// Pure router for the label-edit *character-input* path. Inserts
/// printable chars from a `Key::Character` payload into the buffer.
///
/// Originally this also handled structural keys (Backspace, Delete,
/// arrows, Home, End) directly via `Key::Named` matching, but Phase 5
/// migrated those to `Action::LabelEdit*` variants that route through
/// `dispatch::apply_label_edit_action_to_buffer`. The structural-key
/// arms were stripped here so unbinding `label_edit_*` in
/// `keybinds.json` actually disables the key — the previous fallback
/// shadowed user config.
///
/// Returns `true` iff a printable character was inserted.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn route_label_edit_key(
    logical_key: &winit::keyboard::Key,
    buffer: &mut String,
    cursor: &mut usize,
) -> bool {
    use winit::keyboard::Key;
    if let Key::Character(c) = logical_key {
        // `Key::Character` payloads can carry IME / dead-key multi-
        // char sequences, so iterate. Control chars (and any non-
        // printing payload winit attaches to a structural key) are
        // filtered, which mirrors the original guard intent — the
        // "huge pause icon on backspace" hole is also closed by the
        // structural-key migration to actions, since Backspace is now
        // dispatched as `Action::LabelEditDeleteBack` before this
        // router ever runs.
        let mut changed = false;
        for ch in c.as_str().chars() {
            if !ch.is_control() {
                *cursor = insert_at_cursor(buffer, *cursor, ch);
                changed = true;
            }
        }
        return changed;
    }
    false
}


/// Tracks the high-level interaction mode. Normal handles the usual
/// select/drag/pan flow; Reparent mode is entered via Ctrl+P and captures
/// the next left-click as a "choose reparent target" gesture. Connect mode
/// is entered via Ctrl+D and captures the next left-click as a "choose
/// connection target" gesture to create a cross_link edge.
#[cfg(not(target_arch = "wasm32"))]
enum AppMode {
    Normal,
    /// Reparent mode: the user is choosing a new parent for `sources`.
    /// The next left-click on a node attaches all sources as its last children;
    /// a left-click on empty canvas promotes them to root. Esc cancels.
    Reparent { sources: Vec<String> },
    /// Connect mode: the user is drawing a new cross_link edge from `source`.
    /// The next left-click on a target node creates the edge; a left-click
    /// on empty canvas cancels. Esc also cancels.
    Connect { source: String },
}

/// Tracks the current drag interaction state.
///
/// The four continuous, high-rate-input-driven drag variants
/// (`MovingNode`, `EdgeHandle`, `PortalLabel`, `EdgeLabel`) are
/// collapsed behind the `Throttled` tag. Each carries its
/// pending-state and adaptive throttle as an interaction struct
/// implementing [`throttled_interaction::ThrottledInteraction`];
/// the per-frame drain in
/// [`run_native::InitState::drain_frame`] dispatches through
/// [`ThrottledDrag::as_dyn_mut`] without naming the active kind.
/// Adding a fifth throttled drag is a new variant on
/// `ThrottledDrag` + a struct + a trait impl; nothing about this
/// enum needs to grow.
///
/// `Panning` and `SelectingRect` are *not* throttled — panning is
/// a camera-only decree (no mutation) and rect-select is a
/// lightweight overlay redraw.
#[cfg(not(target_arch = "wasm32"))]
enum DragState {
    /// No drag in progress.
    None,
    /// Mouse is down but hasn't moved past the drag threshold yet.
    Pending {
        start_pos: (f64, f64),
        hit_node: Option<String>,
        /// If an edge was selected at mouse-down time and the cursor
        /// landed on one of that edge's grab-handles, this records
        /// which handle the user is about to drag. Populated in
        /// `MouseInput::Pressed`, consumed at the threshold-cross
        /// transition in `CursorMoved`. Takes precedence over
        /// `hit_node` — clicking a handle always wins over clicking
        /// the node behind it.
        hit_edge_handle: Option<(EdgeRef, baumhard::mindmap::scene_builder::EdgeHandleKind)>,
        /// If the cursor landed on a portal marker at mouse-down,
        /// this records `(edge_key, endpoint_node_id)` so a drag
        /// past threshold transitions to `Throttled(PortalLabel)`.
        /// Takes precedence over `hit_node` — the marker sits
        /// above a node, but clicking the marker is "grab this
        /// label," not "move this node." Independent of
        /// `hit_edge_handle` because portal-mode edges don't
        /// expose edge-handles in the first place.
        hit_portal_label: Option<(
            baumhard::mindmap::scene_cache::EdgeKey,
            String,
        )>,
        /// If the cursor landed on an edge-label AABB at
        /// mouse-down, this records the owning edge key so a
        /// drag past threshold transitions to
        /// `Throttled(EdgeLabel)`. Takes precedence over
        /// `hit_node` — a label hovering over a node behind
        /// it should move as a label, not a node.
        hit_edge_label: Option<baumhard::mindmap::scene_cache::EdgeKey>,
    },
    /// Dragging to pan the camera (started on empty space).
    /// Unthrottled — emits a `CameraPan` decree directly, no
    /// tree or model mutation involved.
    Panning,
    /// Shift+drag on empty space: rubber-band selection rectangle.
    /// Unthrottled — overlay rectangle plus preview highlight is
    /// cheap enough to run every frame.
    SelectingRect {
        /// Canvas-space corner where the drag started.
        start_canvas: Vec2,
        /// Canvas-space corner at current cursor position.
        current_canvas: Vec2,
    },
    /// One of the four throttled, mutation-heavy drag gestures —
    /// see [`ThrottledDrag`] for variants. All four share the
    /// same adaptive-throttle shell via
    /// [`throttled_interaction::ThrottledInteraction`].
    Throttled(ThrottledDrag),
}

/// Application root: winit window + event loop, launches the rendering pipeline.
#[cfg(target_arch = "wasm32")]
pub struct Application {
    options: Options,
    event_loop: EventLoop<()>,
    window: Arc<Window>,
}

#[cfg(not(target_arch = "wasm32"))]
pub struct Application {
    options: Options,
}

impl Application {
    #[cfg(target_arch = "wasm32")]
    pub fn new(options: Options) -> Self {
        let event_loop = EventLoop::new().expect("Could not create an EventLoop");

        // Pre-creating the window here on winit 0.30 is deprecated in
        // favour of `ActiveEventLoop::create_window` inside
        // `ApplicationHandler::resumed`. The native path takes that
        // route; the WASM path still pre-creates because
        // `run_wasm::run` attaches the canvas and installs DOM event
        // listeners before the event loop starts.
        #[allow(deprecated)]
        let window = event_loop
            .create_window(Window::default_attributes())
            .expect("Failed to create application window");

        Application {
            options,
            event_loop,
            window: Arc::new(window),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(options: Options) -> Self {
        Application { options }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn run(self) {
        run_native::run(self)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn run(self) {
        run_wasm::run(self)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn into_options(self) -> Options {
        self.options
    }
}

/// Launch options for the application.
#[derive(Clone)]
pub struct Options {
    pub launch_gpu_prefer_low_power: bool,
    pub should_exit: bool,
    pub window_mode: WindowMode,
    pub ui_scale: i8,
    pub window_title_text: &'static str,
    pub input_mode: InputMode,
    pub avail_cores: usize,
    pub render_must_be_main: bool,
    pub mindmap_path: String,
    /// The user's keybinding configuration (already loaded from file or
    /// defaults). The event loop resolves this into a `ResolvedKeybinds`
    /// at startup and dispatches keyboard events through it.
    pub keybind_config: crate::application::keybinds::KeybindConfig,
}

// Unit tests for pure helpers (cursor math, caret insertion,
// double-click detection, Baumhard mutation round-trip). Event-loop
// integration is verified manually via `cargo run`.

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests;
