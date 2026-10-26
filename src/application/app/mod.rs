// SPDX-License-Identifier: MPL-2.0

//! Application shell: winit event loop, modal state machines,
//! and the dispatch funnel that ties them together. [`Application`]
//! is the binary entry point's root; [`Application::run`]
//! transfers control to the per-target run loop
//! (`run_native` / `run_wasm`) which builds the appropriate
//! `ApplicationHandler` and hands it to winit. Both are plain
//! code-spans, not intra-doc links: each is `cfg`-gated to one
//! target and rustdoc resolves links against the *active*
//! target's module tree, so a link to either breaks the doc
//! build for the other target.
//!
//! **Dispatch funnel.** Every user-driven action — keyboard,
//! mouse-click, console verb, macro replay — flows through
//! `dispatch::dispatch_action` (CODE_CONVENTIONS §3) — a plain
//! code-span for the same reason: it is re-exported from the
//! native-gated `dispatch::native`, so the link does not resolve
//! on `wasm32-unknown-unknown`. Per-event
//! handlers (in `event_keyboard`, `event_mouse_click`,
//! `event_cursor_moved` on native; the per-arm methods of
//! `run_wasm::WasmApp` on WASM) recognize an input gesture,
//! resolve it to an [`crate::application::keybinds::Action`],
//! and call into the funnel. Adding a new behavior is
//! variant + default + arm, in that order; never inline a body
//! in a handler.
//!
//! **Modal state machines.** `text_edit`, `single_line_edit`,
//! `console_input`, and `color_picker_flow` steal keyboard input
//! when open (the §3 carve-out for modals that own the literal
//! `winit::Key` payload). Mouse handlers continue to run; modal
//! commit / cancel routes through `Action::TextEditCommit` /
//! `LabelEditCancel` etc. The two text editors share one steal /
//! release ladder in `modal_editor`.
//!
//! **Cross-platform shape.** Pure logic (gesture recognition,
//! viewport math, hit testing, `Action` resolution) lives in
//! `cfg`-untagged free functions so it compiles for both
//! native and WASM. The native vs. WASM divergence is largely
//! confined to the run-loop entry point; cross-platform
//! `Action` arms route through [`dispatch::action_core`]'s
//! `dispatch_compatible`. See `work_plans/WASM_CONVERGENCE.md` for the
//! current convergence status.

mod animation_drain;
mod scene_rebuild;
mod text_edit;

// Dispatch funnel — `cross_dispatch` (shared apply_* helpers),
// `action_core` (Compatible-Action dispatcher), `macro_core`
// (cross-platform macro step loop + privilege gate), and `native`
// (native dispatch_action wrapper that adds the NativeOnly arm
// match). The directory's `mod.rs` re-exports the public surface
// so callers stay terse.
pub(crate) mod dispatch;

// Native-only — interactive modal state machines absent on WASM.
// See CLAUDE.md "Dual-target status".
#[cfg(not(target_arch = "wasm32"))]
mod capture_flow;
#[cfg(not(target_arch = "wasm32"))]
mod capture_scene;
#[cfg(not(target_arch = "wasm32"))]
mod click;
mod click_triggers;
#[cfg(not(target_arch = "wasm32"))]
mod color_picker_flow;
#[cfg(not(target_arch = "wasm32"))]
mod console_input;
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
pub(crate) mod fire_custom_mutation;
#[cfg(not(target_arch = "wasm32"))]
mod freeze_watchdog;
#[cfg(not(target_arch = "wasm32"))]
mod heartbeat;
mod step;
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) use heartbeat::{step_document, SteppedDocument};
#[cfg(not(target_arch = "wasm32"))]
mod input_context;
mod interaction_mode;
mod ipc_scene;
mod ipc_state;
#[cfg(not(target_arch = "wasm32"))]
mod ipc_target;
#[cfg(not(target_arch = "wasm32"))]
mod pump;
#[cfg(not(target_arch = "wasm32"))]
mod record_flow;
// Cross-platform context-bundles for the unified `dispatch_action`
// funnel. Track C from `work_plans/WASM_CONVERGENCE.md` (final convergence step).
mod input_context_core;
mod modal_editor;
#[cfg(not(target_arch = "wasm32"))]
mod portal_label_drag;
#[cfg(not(target_arch = "wasm32"))]
mod run_headless;
#[cfg(not(target_arch = "wasm32"))]
mod run_native;
#[cfg(not(target_arch = "wasm32"))]
mod run_native_init;
#[cfg(target_arch = "wasm32")]
mod run_wasm;
#[cfg(not(target_arch = "wasm32"))]
mod settle;
#[cfg(not(target_arch = "wasm32"))]
mod shell;
#[cfg(not(target_arch = "wasm32"))]
mod single_line_edit;
// Cross-platform: both init paths resolve the initial map load
// through it, which is what keeps the two from surfacing a load
// failure differently (CODE_CONVENTIONS §4).
mod startup_load;
#[cfg(not(target_arch = "wasm32"))]
mod throttled_interaction;
mod touch_gesture;
mod touch_handle;

// Drift surface for a new `InputHandlerContext` field: the struct in
// `input_context.rs`, the `InitState::input_context()` builder in
// `run_native_init.rs`, and `dispatch_action`'s signature in
// `dispatch/native.rs`.

#[cfg(not(target_arch = "wasm32"))]
use crate::application::document::EdgeRef;
#[cfg(not(target_arch = "wasm32"))]
use glam::Vec2;
#[cfg(not(target_arch = "wasm32"))]
use throttled_interaction::ThrottledDrag;

#[cfg(target_arch = "wasm32")]
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use winit::{event_loop::EventLoop, window::Window};

/// Screen-space click tolerance (in pixels) for edge hit testing. Converted
/// to canvas units via `Renderer::canvas_per_pixel()` so the click target
/// stays visually stable across zoom levels.
///
/// Cross-platform: a connection path is the last rung of the tap's
/// hit chain (`dispatch::cross_dispatch::pointer`'s tap body) on
/// both targets, and a curve cannot be hit by the scene's
/// bounding-volume descent — it needs a tolerance. The constant was
/// native-gated for as long as native's click handler was its only
/// reader.
const EDGE_HIT_TOLERANCE_PX: f32 = 8.0;

/// Which [`PlatformContext`](baumhard::mindmap::custom_mutation::PlatformContext)
/// this build's input paths report when they fire a node's `OnClick`
/// triggers, so an authored mutation gated on `["Desktop"]` or
/// `["Web"]` fires where its author said it should.
///
/// Chosen at compile time, which is why it is a constant rather than
/// a parameter — and why the `Touch` variant is *not* selected here
/// even by the tap path. Which platform a build is remains a build
/// fact; whether an individual gesture should re-label it is a
/// question about the authoring format (`format/mutations.md`), not
/// about the input vocabulary, and answering it "yes" would silently
/// stop every `["Web"]`-gated trigger from firing under a finger in
/// the browser.
#[cfg(not(target_arch = "wasm32"))]
const PLATFORM_CONTEXT: baumhard::mindmap::custom_mutation::PlatformContext =
    baumhard::mindmap::custom_mutation::PlatformContext::Desktop;

/// WASM peer of the native `PLATFORM_CONTEXT` above; see its doc for
/// why the choice is a build fact.
#[cfg(target_arch = "wasm32")]
const PLATFORM_CONTEXT: baumhard::mindmap::custom_mutation::PlatformContext =
    baumhard::mindmap::custom_mutation::PlatformContext::Web;

/// Screen-space click tolerance (in pixels) for grab-handle hit
/// testing — applies uniformly to edge handles and section / node
/// resize handles. With explicit `InteractionMode::Resize` gating
/// handle visibility, the press-time hit-test only competes with
/// itself (no body-vs-handle ambiguity), so 8px is generous for a
/// 14pt ☐ glyph at standard zoom. Touch / accessibility tuning
/// will need this to grow — `KeybindConfig`-side configurability
/// is a future seam.
#[cfg(not(target_arch = "wasm32"))]
const HANDLE_HIT_TOLERANCE_PX: f32 = 8.0;

/// Minimum pointer travel (**physical** pixels) before a press stops
/// reading as a click and starts reading as a drag. Splits "this
/// was a click" from "this is a drag" — small enough that
/// intentional drags engage immediately, large enough that
/// trembling fingers don't accidentally start a drag-to-move from
/// a click.
///
/// **One threshold for every pointer**, mouse and finger alike:
/// the mouse arms in `event_cursor_moved.rs` (left-button
/// `Pending` and right-button `PendingRight`) and both emit points
/// of [`touch_gesture::TouchGestureRecognizer`] (the long-press
/// cancel and the two-finger centroid step) compare against
/// [`POINTER_DRAG_THRESHOLD_SQ_PX`] below. The touch recognizer
/// used to carry its own `MOVE_THRESHOLD_PX = 4.0` under a comment
/// claiming it mirrored this one at the same value; it did not,
/// and a finger consequently promoted to a drag a pixel earlier
/// than a mouse did.
///
/// **Physical, not logical**, and both readers say so because both
/// paths feed the comparison winit's raw device coordinates:
/// `handle_cursor_moved` stores `WindowEvent::CursorMoved`'s
/// `PhysicalPosition` into `cursor_pos` unconverted, and
/// `dispatch_touch_event` hands the recognizer `touch.location`, a
/// `PhysicalPosition` too. There is no `winit::window::Window::scale_factor` call anywhere
/// under `src/application` to convert either. Parity is unaffected —
/// both pointers are measured with the same ruler, which is the
/// property this constant exists for — but the *number* scales with
/// the display: on a 2x screen 5 physical px is 2.5 logical px of
/// slop, not 5. Making the budget display-independent means dividing
/// through by `Window::scale_factor` at both readers, which is the
/// open coordinate-space item in
/// `work_plans/SECTIONS_BORDERS_RESIZE_PLAN.md` §"Touch coordinate-
/// space drift" and is a behavior change on every HiDPI machine, so
/// it is not folded in here.
const POINTER_DRAG_THRESHOLD_PX: f64 = 5.0;

/// [`POINTER_DRAG_THRESHOLD_PX`] squared — the form every
/// comparison actually uses, so no pointer path pays a `sqrt` per
/// motion event. Derived rather than written out so the two cannot
/// drift.
const POINTER_DRAG_THRESHOLD_SQ_PX: f64 = POINTER_DRAG_THRESHOLD_PX * POINTER_DRAG_THRESHOLD_PX;

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
    /// Cursor is inside node `id`'s AABB. `section_idx` is `Some`
    /// when the click resolved to a specific section of a multi-
    /// section node (mirroring `HitTarget::Section`); `None` for
    /// single-section nodes and chrome-only hits. The
    /// `PartialEq`-derived double-click compare honors the
    /// section index, so two slow clicks on different sections
    /// of the same node correctly *don't* count as a double-click,
    /// and a genuine same-section double-click routes the
    /// editor-open path to the targeted section instead of
    /// silently defaulting to section 0.
    Node(String, Option<usize>),
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
    /// pan-to-partner behavior as `PortalMarker` — the
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
/// milliseconds on the app clock (`AppClock::now_ms`), so under a
/// virtual clock the window is virtual milliseconds too.
#[derive(Debug, Clone)]
struct LastClick {
    time: f64,
    screen_pos: (f64, f64),
    /// What the first click landed on. Two clicks whose `hit`
    /// values compare equal under `ClickHit`'s derived `PartialEq`
    /// qualify as a double-click.
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
    if !(0.0..DOUBLE_CLICK_MS).contains(&elapsed) {
        return false;
    }
    let dx = new_screen_pos.0 - prev.screen_pos.0;
    let dy = new_screen_pos.1 - prev.screen_pos.1;
    if dx * dx + dy * dy >= DOUBLE_CLICK_DIST_SQ {
        return false;
    }
    &prev.hit == new_hit
}

/// Scroll delta as a signed line count, for the wheel-gesture
/// lookup.
///
/// winit reports two shapes and they are not interchangeable: a
/// notched wheel reports whole lines, a trackpad or a
/// high-resolution wheel reports pixels. The `/ 50.0` divisor is the
/// pixels-per-line convention this app has always used; it is here
/// rather than at the two call sites so a change to it cannot land
/// on one target only.
///
/// Only the sign is consulted by the gesture lookup today
/// (`> 0.0` → `WheelUp`, otherwise `WheelDown`), but the magnitude is
/// preserved: the console's scrollback accumulates fractional lines
/// through it, and a future per-notch zoom step would want it too.
fn wheel_lines(delta: crate::application::platform::input::MouseScrollDelta) -> f64 {
    use crate::application::platform::input::MouseScrollDelta;
    match delta {
        MouseScrollDelta::LineDelta(_, y) => y as f64,
        MouseScrollDelta::PixelDelta(pos) => pos.y / 50.0,
    }
}

/// The `MouseGesture` a scroll of `lines` names. Split from
/// [`wheel_lines`] because the sign convention — zero and negative
/// both scroll down — is a decision, not arithmetic, and both
/// targets have to make the same one.
fn wheel_gesture(lines: f64) -> crate::application::keybinds::MouseGesture {
    if lines > 0.0 {
        crate::application::keybinds::MouseGesture::WheelUp
    } else {
        crate::application::keybinds::MouseGesture::WheelDown
    }
}

/// Is an inline editor already open on the thing this press landed
/// on?
///
/// When it is, the press must **not** be promoted to a double-click:
/// the double-click would re-open the editor, and re-opening re-seeds
/// the buffer from the committed model value, silently discarding
/// whatever the user had typed. The press instead falls through and
/// the matching release is swallowed as a click-inside.
///
/// Three plain inputs so the guard is pinnable without an event loop
/// (`TEST_CONVENTIONS §T9`):
/// - `edit_node_id` — the node the multi-line text editor has open,
///   `None` when it is closed.
/// - `hit_node` — the node this press landed on, `None` for
///   empty canvas and for non-node hits.
/// - `single_line_match` — whether the single-line editor (edge
///   label / portal caption) is open on *this* press's target. It is
///   passed in rather than resolved here because that editor is
///   native-only today; the browser passes `false`.
///
/// The two editor states are mutually exclusive by construction (the
/// keyboard steal claims whichever opened first), so the `||` is
/// belt-and-braces rather than a case that arises in practice — but
/// a guard that only covered one of them would be a live bug the
/// moment that stops holding.
fn already_editing_same_target(
    edit_node_id: Option<&str>,
    hit_node: Option<&str>,
    single_line_match: bool,
) -> bool {
    edit_node_id.is_some_and(|id| hit_node == Some(id)) || single_line_match
}

/// Warn once per latch that an input resolved to an Action the
/// browser has no body for, so the gesture is a no-op. Returns
/// whether this call is the one that emitted.
///
/// Every input class that consults the keybind table needs this. A
/// user who binds a `NativeOnly` Action to a gesture gets *literally
/// nothing* otherwise — no log, no chrome, no model change — which
/// was rejected as a blocking review finding on the touch path
/// (`Whole-PR review BLK-1`) and is no more acceptable on the
/// double-click or the wheel now that both consult the table too.
/// `warned` is a per-call-site latch so one input class going quiet
/// cannot silence another; `remedy` names what the user can do about
/// it and what unblocks parity.
///
/// A *sanctioned* carve-out — a residual the browser is documented as
/// not finishing, like the edge-label single-line editor — is not
/// this. Those log at `debug!` at their own call site: they are
/// expected, and `warn!` survives into release (`CODE_CONVENTIONS
/// §9`) where a per-double-click warning would be noise. The
/// classification is
/// [`dispatch::classify_unhandled_pointer_dispatch`], not the call
/// site's guesswork.
///
/// **Cross-platform on purpose, though every caller is under
/// `run_wasm/`.** Nothing in the body is browser-specific — an
/// `AtomicBool`, three borrowed values and a `log::warn!` — and while
/// it lived inside the `#![cfg(target_arch = "wasm32")]` module
/// `cargo test` could not reach it, which is what §T9 forbids for
/// platform-shared logic. The `bool` return is what makes the
/// one-shot semantics assertable without a global log sink: this
/// workspace has no log-capture test facility, and installing one
/// would be a process-global mock that §T10 rules out. Callers may
/// ignore it.
// Consumer is `run_wasm/`; the host build reaches this only from
// `#[cfg(test)]`, so arm the lint on wasm32 and silence it here
// rather than blanket-allowing on both targets — if the browser
// caller goes away, wasm32 says so.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) fn warn_unhandled_native_only_once(
    warned: &std::sync::atomic::AtomicBool,
    gesture: &str,
    action: &crate::application::keybinds::Action,
    remedy: &str,
) -> bool {
    if warned.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    log::warn!(
        "run_wasm: input '{}' is bound to a NativeOnly action ({:?}) with no \
         browser body — the gesture is a no-op. {}",
        gesture,
        action,
        remedy,
    );
    true
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
    /// Section index inside `hit_node`, when the click landed on a
    /// specific section in a multi-section node. `None` for clicks
    /// on single-section nodes (chrome semantics) and for empty-canvas
    /// clicks.
    pub(super) hit_section_idx: Option<usize>,
    pub(super) portal_text_hit: Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    pub(super) portal_icon_hit: Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    pub(super) edge_label_hit: Option<baumhard::mindmap::scene_cache::EdgeKey>,
}

/// Pure router for "what did this click target?". Runs the
/// node → portal → edge-label priority chain and folds the
/// resolved hits into a single [`ClickHitParts`]. Both the
/// native click handler and the WASM click handler previously
/// open-coded byte-identical versions of this body — they now
/// both call here.
///
/// Priority rationale: node hits beat portal hits (a node
/// under a portal marker is the more common target). Edge-label
/// hits only register when no node / portal sub-part has claimed
/// the click — labels sit along the connection path, and placing
/// them behind the portal check keeps the portal's "floating over
/// a node" behavior correct even if a label happens to overlap.
///
/// The portal rung is a **single** query. Icon and text are
/// sibling leaves of one tree, so
/// [`AppScene::portal_at`](crate::application::scene_host::AppScene::portal_at)
/// resolves both in one BVH descent and names which sub-part won;
/// `portal_text_hit` and `portal_icon_hit` are two views of that
/// one answer and can never both be `Some`. The predecessor ran
/// two independent scans over two hash maps and relied on
/// check-text-first to break a tie the maps could not see.
pub(super) fn compute_click_hit(
    canvas_pos: glam::Vec2,
    mindmap_tree: Option<&mut baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
) -> ClickHitParts {
    use baumhard::mindmap::tree_builder::PortalPart;

    let (hit_node, hit_section_idx) = match mindmap_tree {
        Some(tree) => match crate::application::document::hit_test_target(canvas_pos, tree) {
            Some(crate::application::document::HitTarget::NodeContainer { node_id }) => (Some(node_id), None),
            Some(crate::application::document::HitTarget::Section { node_id, section_idx }) => {
                (Some(node_id), Some(section_idx))
            }
            None => (None, None),
        },
        None => (None, None),
    };

    let portal_hit = if hit_node.is_none() {
        app_scene.portal_at(canvas_pos)
    } else {
        None
    };
    let portal_claimed = portal_hit.is_some();
    let (portal_text_hit, portal_icon_hit) = match portal_hit {
        Some(hit) => {
            let endpoint = (hit.edge_key, hit.endpoint_node_id);
            match hit.part {
                PortalPart::Text => (Some(endpoint), None),
                PortalPart::Icon => (None, Some(endpoint)),
            }
        }
        None => (None, None),
    };
    let edge_label_hit = if hit_node.is_none() && !portal_claimed {
        app_scene.edge_label_at(canvas_pos)
    } else {
        None
    };

    let click_hit = click_hit_from_priority(
        &hit_node,
        hit_section_idx,
        &portal_text_hit,
        &portal_icon_hit,
        &edge_label_hit,
    );

    ClickHitParts {
        click_hit,
        hit_node,
        hit_section_idx,
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
    hit_section_idx: Option<usize>,
    portal_text_hit: &Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    portal_icon_hit: &Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    edge_label_hit: &Option<baumhard::mindmap::scene_cache::EdgeKey>,
) -> ClickHit {
    if let Some(id) = hit_node {
        ClickHit::Node(id.clone(), hit_section_idx)
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

/// The portal-icon hit a left press hands on to
/// [`DragState::Pending`], given the node hit the same click chain
/// resolved.
///
/// **This is why the click path and the drag path cannot disagree
/// about a node under a portal marker.** They run one hit chain,
/// not two: `compute_click_hit` gives the node priority, and a
/// press stores the portal hit for the drag promotion only when
/// that chain left `hit_node` empty. The promotion order in
/// `event_cursor_moved.rs` does list portal-label above node — but
/// over a set where at most one of them is populated, so what that
/// listing resolves is the node against the three *handle* hits
/// the press captures outside the chain (edge handle, node resize,
/// section resize), each gated on a mode or a selection instead.
///
/// The rule was an inline `if hit_node.is_none()` guard inside the
/// press handler, which needs a live event loop to reach and so
/// could not be pinned (`TEST_CONVENTIONS §T9`). A July-2026
/// review read the promotion order without it and reported the two
/// paths as targeting different things on the same press; naming
/// the rule is what makes that answerable rather than re-derivable.
///
/// Only the icon sub-part is a drag affordance — sliding a
/// portal's text along the border is not a gesture — so a
/// text-side hit yields `None` here regardless of the node.
#[cfg(not(target_arch = "wasm32"))]
fn portal_label_drag_capture(
    portal_icon_hit: &Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    hit_node: Option<&str>,
) -> Option<(baumhard::mindmap::scene_cache::EdgeKey, String)> {
    match portal_icon_hit {
        Some((key, endpoint)) if hit_node.is_none() => Some((key.clone(), endpoint.clone())),
        _ => None,
    }
}

// Re-export the mode enum, the shared selection→target resolver,
// and the resolver's typed error so the console layer can carry
// `InteractionMode` inside `ConsoleSideEffect` and consumers across
// `application::*` reach a uniform path. Full doc + variant prose
// lives in `interaction_mode.rs`.
pub(in crate::application) use interaction_mode::{
    resolve_resize_target, InteractionMode, ResizeTargetError,
};
// `ResizeTarget` is constructed only inside `interaction_mode.rs`
// (the resolver returns it); production consumers pattern-match
// the result without naming the type. Tests assert on the variant
// shape, so the re-export is `#[cfg(test)]`-gated to avoid an
// unused-import warning on non-test builds. Production code
// (`ipc_state.rs`) imports it from `interaction_mode` directly; the
// gate moves when a non-test consumer needs this path.
#[cfg(test)]
pub(in crate::application) use interaction_mode::ResizeTarget;

/// Everything a left mouse-down captured, held until the cursor
/// either crosses the drag threshold (promoting to a
/// [`ThrottledDrag`]) or comes back up without moving (routing as a
/// click). Boxed inside [`DragState::Pending`] — see there.
#[cfg(not(target_arch = "wasm32"))]
struct PendingPress {
    start_pos: (f64, f64),
    hit_node: Option<String>,
    /// Index inside `hit_node.sections` when the press landed
    /// on a specific section in a multi-section node. `None`
    /// for empty-canvas, single-section, or non-node hits.
    /// Threads through to `handle_click` on the release path
    /// so the post-press selection update can reach for
    /// `SelectionState::Section` when appropriate.
    hit_section_idx: Option<usize>,
    /// If an edge was selected at mouse-down time and the cursor
    /// landed on one of that edge's grab-handles, this records
    /// which handle the user is about to drag. Populated in
    /// `MouseInput::Pressed`, consumed at the threshold-cross
    /// transition in `CursorMoved`. Takes precedence over
    /// `hit_node` — clicking a handle always wins over clicking
    /// the node behind it.
    hit_edge_handle: Option<(EdgeRef, baumhard::mindmap::tree_builder::EdgeHandleKind)>,
    /// If the cursor landed on a portal marker at mouse-down,
    /// this records `(edge_key, endpoint_node_id)` so a drag
    /// past threshold transitions to `Throttled(PortalLabel)`.
    /// Takes precedence over `hit_node` — the marker sits
    /// above a node, but clicking the marker is "grab this
    /// label," not "move this node." Independent of
    /// `hit_edge_handle` because portal-mode edges don't
    /// expose edge-handles in the first place.
    hit_portal_label: Option<(baumhard::mindmap::scene_cache::EdgeKey, String)>,
    /// If the cursor landed on an edge-label AABB at
    /// mouse-down, this records the owning edge key so a
    /// drag past threshold transitions to
    /// `Throttled(EdgeLabel)`. Takes precedence over
    /// `hit_node` — a label hovering over a node behind
    /// it should move as a label, not a node.
    hit_edge_label: Option<baumhard::mindmap::scene_cache::EdgeKey>,
    /// If a section is currently selected and the cursor
    /// landed on one of its 8 resize handles, this records
    /// `(node_id, section_idx, side)` so a drag past
    /// threshold transitions to `Throttled(SectionResize)`.
    /// Takes precedence over `hit_node` — a handle sits on
    /// the section's edge and clicking it is a resize, not
    /// a section drag.
    hit_section_resize_handle: Option<(String, usize, baumhard::mindmap::tree_builder::ResizeHandleSide)>,
    /// If a node is currently `Single`-selected and the
    /// cursor landed on one of its 8 resize handles, this
    /// records `(node_id, side)` so a drag past threshold
    /// transitions to `Throttled(NodeResize)`. Takes
    /// precedence over `hit_node` — clicking a handle on
    /// a selected node is a resize, not a re-selection or
    /// a move-node drag.
    hit_node_resize_handle: Option<(String, baumhard::mindmap::tree_builder::ResizeHandleSide)>,
}

/// One thing a press can have landed on that promotes a drag past the
/// threshold, **in the precedence the threshold-cross step resolves
/// them** (`event_cursor_moved.rs`). This list is the single source of
/// that order: [`PendingPress::first_grab`] reads it, the threshold-
/// cross `match` dispatches on what `first_grab` returns, and
/// `query.mode` reports [`PendingPress::grabs`] in it, so the three
/// cannot disagree. Portal *text* is intentionally absent: dragging a
/// portal's text sub-part is not a gesture, the icon carries the drag.
///
/// The two resize handles lead (#218). They are captured only in
/// `InteractionMode::Resize`, on the target's own handles, so outside
/// that mode the order is what it was. Inside it, the mode exists for
/// those handles, while a label is grabbable in every mode and stays so
/// once the mode is left. The press reads the same list for
/// double-click detection ([`PendingPress::is_handle_press`]), so what a
/// press would drag is what it is.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grab {
    EdgeLabel,
    PortalLabel,
    EdgeHandle,
    NodeResizeHandle,
    SectionResizeHandle,
}

#[cfg(not(target_arch = "wasm32"))]
impl Grab {
    /// Every grab, first-resolved first.
    const PRECEDENCE: [Grab; 5] = [
        Grab::NodeResizeHandle,
        Grab::SectionResizeHandle,
        Grab::EdgeLabel,
        Grab::PortalLabel,
        Grab::EdgeHandle,
    ];

    /// Whether this grab is a handle: a drag affordance with no click
    /// meaning of its own, unlike a label, which a double-click edits.
    fn is_handle(self) -> bool {
        match self {
            Grab::NodeResizeHandle | Grab::SectionResizeHandle | Grab::EdgeHandle => true,
            Grab::EdgeLabel | Grab::PortalLabel => false,
        }
    }

    /// The grab's name as `query.mode` reports it.
    fn word(self) -> &'static str {
        match self {
            Grab::EdgeLabel => "edge_label",
            Grab::PortalLabel => "portal_label",
            Grab::EdgeHandle => "edge_handle",
            Grab::NodeResizeHandle => "node_resize_handle",
            Grab::SectionResizeHandle => "section_resize_handle",
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PendingPress {
    /// Whether the press recorded `grab`.
    fn has(&self, grab: Grab) -> bool {
        match grab {
            Grab::EdgeLabel => self.hit_edge_label.is_some(),
            Grab::PortalLabel => self.hit_portal_label.is_some(),
            Grab::EdgeHandle => self.hit_edge_handle.is_some(),
            Grab::NodeResizeHandle => self.hit_node_resize_handle.is_some(),
            Grab::SectionResizeHandle => self.hit_section_resize_handle.is_some(),
        }
    }

    /// Every grab the press recorded, in [`Grab::PRECEDENCE`] order.
    fn grabs(&self) -> impl Iterator<Item = Grab> + '_ {
        Grab::PRECEDENCE.into_iter().filter(|grab| self.has(*grab))
    }

    /// The grab the threshold-cross step resolves, if any: the first
    /// recorded one in [`Grab::PRECEDENCE`]. `None` leaves the press
    /// to the node-move / rect-select / pan arms.
    fn first_grab(&self) -> Option<Grab> {
        self.grabs().next()
    }

    /// Whether the drag this press would become is a handle's (#218).
    /// Such a press never takes part in double-click detection: it
    /// neither completes a double-click nor starts one, so a second
    /// press on a handle grabs it rather than opening an editor, and a
    /// handle grab followed by a quick press elsewhere is no
    /// double-click either. A press whose first grab is a label, or
    /// that grabbed nothing, double-clicks as any press does.
    fn is_handle_press(&self) -> bool {
        self.first_grab().is_some_and(Grab::is_handle)
    }
}

/// An input that is *held*: it is down now and will come up, and
/// coming up is how the user says "stop". The two kinds the window
/// delivers a release for.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum HeldInput {
    /// A mouse button.
    Button(crate::application::platform::input::MouseButton),
    /// A key, by the name `keybinds::key_to_name` gives it on the
    /// press, which the release must spell the same way to end the
    /// pan. `key_to_name` only lowercases, so for a letter a Shift
    /// lifted first still matches; for a key whose shifted character
    /// differs (`!` over `1`) it does not, and that pan lasts until a
    /// left press or focus loss. Narrow in practice — key lookups have
    /// no modifier fallback, so a shifted-punctuation binding is
    /// mostly unreachable — but it is why the claim stops at letters.
    Key(String),
}

/// What ends a [`DragState::Panning`].
///
/// **A pan belongs to the input that armed it** (#153). An input
/// that is held ends the pan on its own release — momentary, the
/// mouse's posture and the Space-to-pan posture of most canvas
/// tools. An input that has no release — a macro step, a console
/// line, an IPC `act.action`, and a touch `LongPress`, since
/// [`HeldInput`] has no finger — cannot be momentary, so its pan is
/// modal: it toggles.
///
/// Whichever owner a pan has, a left *press* still re-arms `Pending`
/// over it (`DragState::would_abandon_gesture` keeps `Panning` out of
/// the class a press may not overwrite): `Panning` owes the model
/// nothing, and the ownership rule is about which *release* ends a
/// pan, not about presses.
///
/// The Action cannot see which input reached it: `dispatch_action`
/// strips the gesture down to an [`Action`](crate::application::keybinds::Action).
/// So the arm arms every pan [`PanOwner::Toggled`], and a held input
/// that dispatched the arming Action claims it afterwards
/// (`dispatch::native::dispatch_held_action`, through
/// [`DragState::hand_armed_pan_to`]) — the same read-the-decision-back
/// shape `event_cursor_moved::emits_first_pan_delta` already uses.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum PanOwner {
    /// Momentary: ends on this input's release, and on no other
    /// release. A release of any other button or key leaves it
    /// running.
    Held(HeldInput),
    /// Modal: armed by an input with no release. Ends on the next
    /// `Action::PanCanvas` that finds it — from a key, a macro, the
    /// console or IPC alike — and no button or key release ends it.
    Toggled,
}

/// Tracks the current drag interaction state.
///
/// Continuous, high-rate-input-driven drag variants
/// (`MovingNode`, `MovingSection`, `SectionResize`, `NodeResize`,
/// `EdgeHandle`, `PortalLabel`, `EdgeLabel`) are collapsed behind
/// the `Throttled` tag. Each carries its pending-state and
/// adaptive throttle as an interaction struct implementing
/// [`throttled_interaction::ThrottledInteraction`]; the per-frame
/// drain in [`input_context::InputHandlerContext::drain_inputs`] dispatches
/// through [`ThrottledDrag::as_dyn_mut`] without naming the
/// active kind. Adding a new throttled drag is a new variant on
/// `ThrottledDrag` + a struct + a trait impl; nothing about this
/// enum needs to grow.
///
/// `Panning` and `SelectingRect` are *not* throttled, for two
/// different reasons. Panning is a camera-only decree with no
/// mutation to defer. Rect-select's per-frame work is the overlay
/// rectangle, which tracks the pointer and so must never be
/// deferred; the expensive half — repainting the covered nodes'
/// highlight — is gated on the covered *set* changing rather than
/// on a frame counter, which skips the work outright on a frame
/// that crosses no node boundary instead of postponing it. This
/// sentence used to read "rect-select is a lightweight overlay
/// redraw", which it was not: every drain ran a full
/// `doc.build_tree()` and a cosmic-text buffer rebuild (#37).
#[cfg(not(target_arch = "wasm32"))]
enum DragState {
    /// No drag in progress.
    None,
    /// Mouse is down but hasn't moved past the drag threshold yet.
    ///
    /// Boxed: the press-time hit chain is eight fields wide and made
    /// this the widest `DragState` variant by a factor of six over
    /// its nearest neighbor — `PendingPress` is 384 bytes against
    /// `PendingRight`'s 64 — which every other state, including the
    /// `None` that is live almost all the time, would otherwise pay
    /// for. One allocation per mouse-down.
    ///
    /// With `Pending` and `Throttled` boxed, `PendingRight` is the
    /// widest variant left and `DragState` is 64 bytes (912 before
    /// this pass). `PendingRight` stays unboxed deliberately: at 64
    /// bytes it is not the outlier the other two were, and boxing it
    /// would trade 40 bytes of stack for an allocation on every
    /// right-button press.
    Pending(Box<PendingPress>),
    /// Right-button is down + cursor hasn't moved past the drag
    /// threshold. Press-time hit captures the body of any node /
    /// section under the cursor (no edge-handle / portal-label /
    /// resize-handle precedence — fast-resize is body-only by
    /// design). The threshold cross takes it down to `None` and
    /// dispatches whatever `MouseGesture::RightDrag` is bound to, with
    /// a hit built from the press ([`Self::right_drag_hit`]) — by
    /// default `Action::FastResizeStart`, which promotes to
    /// `Throttled(NodeResize | SectionResize)`, and `PanCanvas` when
    /// the user binds it there (#194); release-without-movement fires
    /// the bound `MouseGesture::RightClick` action (default
    /// unbound) and resets to `None`.
    PendingRight {
        start_pos: (f64, f64),
        /// Canvas-space cursor position at press time (already
        /// converted via `Renderer::screen_to_canvas`). Carried
        /// through to `Action::FastResizeStart` so the corner
        /// anchor is computed from where the user *pressed*, not
        /// from where the cursor sat at threshold-cross: the
        /// quadrant is determined at press time, not continuously.
        /// Without this snapshot the user gets
        /// whichever corner they happened to drag *toward*, which
        /// inverts the gesture's intent on small nodes.
        start_canvas: glam::Vec2,
        /// Press-time hit, body-only. `None` for a press on
        /// empty canvas — release fires `RightClick` regardless
        /// of where the cursor is, but the threshold-cross arm
        /// won't promote to `FastResizeStart` without a node
        /// target.
        hit_node: Option<String>,
        /// Section index inside `hit_node.sections` when the
        /// press landed on a specific section in a multi-section
        /// node. Mirrors the `hit_section_idx` semantics on
        /// `Pending`. `None` for empty-canvas / single-section /
        /// non-node hits.
        hit_section_idx: Option<usize>,
    },
    /// Panning the camera: every cursor move emits a `CameraPan`
    /// decree directly. Unthrottled, no tree or model mutation
    /// involved.
    ///
    /// `owner` is which release ends it — see [`PanOwner`]; a left
    /// press re-arms over it whoever owns it. Before #153 the
    /// variant carried nothing, so a middle release ended a
    /// left-drag pan, a left release ended a middle-started one, and
    /// a key-armed pan had no release that belonged to it at all.
    Panning { owner: PanOwner },
    /// Shift+drag on empty space: rubber-band selection rectangle.
    ///
    /// Unthrottled: the overlay rectangle is redrawn every frame
    /// because it tracks the pointer. The covered-node preview is
    /// not free and is not run every frame — it is memoized on
    /// `MindMapDocument::rect_select_preview` and repainted only
    /// when the covered set changes. This comment used to claim the
    /// preview was "cheap enough to run every frame"; it was a full
    /// arena build plus a text-buffer rebuild (#37).
    ///
    /// **This variant is the authority for both.** The overlay
    /// rectangle and the covered set are its projection, and
    /// `drain_frame::drain_rect_select` re-derives them from it
    /// every frame rather than trusting each exit from this state to
    /// clean up after itself — which matters because the covered set
    /// is document state *every* node-tree rebuild in the app reads,
    /// and this variant has more ways out than the release arm that
    /// used to be the only one dropping it.
    SelectingRect {
        /// Canvas-space corner where the drag started.
        start_canvas: Vec2,
        /// Canvas-space corner at current cursor position.
        current_canvas: Vec2,
    },
    /// One of the throttled, mutation-heavy drag gestures —
    /// see [`ThrottledDrag`] for variants. All share the same
    /// adaptive-throttle shell via
    /// [`throttled_interaction::ThrottledInteraction`].
    ///
    /// Boxed: the widest `ThrottledDrag` variant carries a full
    /// pre-drag `MindEdge` snapshot and is more than twice the size
    /// of `Pending`, so an inline payload would make every
    /// `DragState` — including the `None` that is live all but a
    /// few seconds of a session — pay for it. One allocation per
    /// gesture start, at the threshold cross.
    Throttled(Box<ThrottledDrag>),
}

#[cfg(not(target_arch = "wasm32"))]
impl DragState {
    /// Enter a throttled drag. The boxing is an implementation
    /// detail of the variant, not something the nine promotion
    /// sites — seven in `event_cursor_moved`, two in
    /// `dispatch::native` — should each have to spell.
    fn throttled(drag: ThrottledDrag) -> Self {
        Self::Throttled(Box::new(drag))
    }

    /// True while a node is being dragged. That drag keeps its
    /// connections current itself, frame by frame, with offsets the
    /// model does not hold until release — so a camera-geometry rebuild
    /// in the middle of it, which projects from the model, would put
    /// them back where they were. The per-frame drain skips the rebuild
    /// then, and a capture at another zoom projects its scratch scene
    /// with the same offsets ([`Self::moving_node_offsets`]).
    fn is_moving_node(&self) -> bool {
        self.moving_node_offsets().is_some()
    }

    /// The offsets a node drag has drained into the tree so far
    /// (`MovingNodeInteraction::drained_offsets`), while one is in
    /// flight: the map that makes a projection from the model land
    /// where the window shows the dragged nodes. `None` outside a node
    /// drag — every other state leaves the model where the tree is.
    fn moving_node_offsets(&self) -> Option<&std::collections::HashMap<String, (f32, f32)>> {
        match self {
            Self::Throttled(d) => match &**d {
                ThrottledDrag::MovingNode(moving) => Some(moving.drained_offsets()),
                _ => None,
            },
            _ => None,
        }
    }

    /// True when overwriting this state with a freshly-armed
    /// gesture would abandon one that nothing else will finish.
    ///
    /// Two states are in the class, and both fail silently:
    ///
    /// - `Throttled(..)` owes the model a write plus an undo entry,
    ///   and `commit_on_release_core` is the only place either
    ///   happens. Drop it and the tree keeps the dragged offsets
    ///   until the next model rebuild snaps them back. Reachable
    ///   from a *right*-started fast-resize, which is in flight
    ///   while the left button is still free.
    /// - `PendingRight` holds a user-named gesture (`RightClick`, or
    ///   whatever `RightDrag` is bound to) that has not fired yet.
    ///
    /// `Pending` / `Panning` / `SelectingRect` are deliberately
    /// outside it: none owes the model anything, and each is
    /// re-derived from the next press or the next cursor sample.
    /// Refusing a left press there would strand a click after a
    /// release the window never delivered instead of re-arming from
    /// it, which is the behavior those three have always had. Focus
    /// loss no longer reaches that case — [`Self::end_on_focus_loss`]
    /// resolves every state when the window loses focus — but a
    /// release the OS eats with the window still focused does, and
    /// nothing else would recover from it.
    ///
    /// It lives on the enum rather than beside any one caller
    /// because it is a property of the state, and two dispatch
    /// surfaces read it: the left-button press in
    /// `event_mouse_click` and the `Action::PanCanvas` arm in
    /// `dispatch::native`, which the keyboard and every macro tier
    /// reach as well as the middle button.
    fn would_abandon_gesture(&self) -> bool {
        matches!(self, Self::PendingRight { .. } | Self::Throttled(_))
    }

    /// True when this is a pan `input` holds, i.e. one that `input`'s
    /// release ends. A [`PanOwner::Toggled`] pan is held by nothing,
    /// so no release ends it.
    fn is_pan_held_by(&self, input: &HeldInput) -> bool {
        matches!(self, Self::Panning { owner: PanOwner::Held(held) } if held == input)
    }

    /// End the pan `input` holds, if this is one; leave every other
    /// state — any other pan included — exactly as it is. The body
    /// the left, right and key releases end a pan through; the middle
    /// button's pure route asks [`Self::is_pan_held_by`] instead.
    fn end_pan_held_by(&mut self, input: &HeldInput) {
        if self.is_pan_held_by(input) {
            *self = Self::None;
        }
    }

    /// Hand a pan the dispatch just armed to the held `input` that
    /// dispatched it.
    ///
    /// `was_panning` is the state *before* the dispatch, and is the
    /// whole of the "just armed" test: a dispatch that started from
    /// a pan cannot have armed one (`Action::PanCanvas` either ends a
    /// toggled pan or refuses a held one), so a toggled pan found
    /// afterwards is the one that was already running and is not
    /// this input's to claim. Without it, any key — bound to
    /// `zoom_in`, say — pressed during a toggled pan would turn it
    /// into a momentary pan that key's release then ends.
    fn hand_armed_pan_to(&mut self, was_panning: bool, input: HeldInput) {
        if was_panning {
            return;
        }
        if let Self::Panning { owner } = self {
            if *owner == PanOwner::Toggled {
                *owner = PanOwner::Held(input);
            }
        }
    }

    /// True for any pan, whoever owns it.
    fn is_panning(&self) -> bool {
        matches!(self, Self::Panning { .. })
    }

    /// Resolve the drag state when the window loses focus: every
    /// gesture's owning release is now one the window may never
    /// deliver, so none is left waiting for it.
    ///
    /// The state always ends at `None`. What it owed decides the rest:
    ///
    /// - `Throttled(..)` owes the model a write and an undo entry, and
    ///   is handed back for the caller to **commit** — through
    ///   `release::commit_if_resolved` with `Release::FocusLost`,
    ///   the gate that answers `Commit` for it whichever button
    ///   started the drag. Committing keeps what the user was looking
    ///   at and pushes an undo entry, so the gesture stays reversible;
    ///   discarding it would snap the canvas back with nothing to
    ///   redo it from.
    /// - `Pending` / `PendingRight` are dropped. A press whose release
    ///   never came is not a click, so neither `handle_click` nor the
    ///   `RightClick` binding fires.
    /// - `Panning` (any owner) and `SelectingRect` are dropped. Neither
    ///   owes the model anything; the per-frame drain ends the
    ///   rubber band's two artifacts on the next frame, as it does for
    ///   every other way out of `SelectingRect`.
    ///
    /// A toggled pan ends too, although its end was never a release:
    /// returning to the window with the camera already following the
    /// pointer is a pan nobody can see they armed.
    fn end_on_focus_loss(&mut self) -> Option<Box<ThrottledDrag>> {
        match std::mem::replace(self, Self::None) {
            Self::Throttled(drag) => Some(drag),
            Self::None
            | Self::Pending(_)
            | Self::PendingRight { .. }
            | Self::Panning { .. }
            | Self::SelectingRect { .. } => None,
        }
    }

    /// The hit a `RightDrag` threshold cross dispatches with, built from
    /// the pending right press: its press-time body hit, its press-time
    /// canvas position, and the right button as the holder. `None` for
    /// every other state.
    ///
    /// Press-time on purpose, both halves: `Action::FastResizeStart`
    /// anchors its corner at `canvas_pos`, and the corner the user
    /// means is the one nearest where they pressed, not the one the
    /// cursor happened to cross the threshold toward
    /// (`SECTIONS_BORDERS_RESIZE_PLAN.md` §6.3). A pure read, so the
    /// harness pins it without the cursor handler's renderer.
    fn right_drag_hit(&self) -> Option<dispatch::DispatchHit> {
        let Self::PendingRight {
            start_canvas,
            hit_node,
            hit_section_idx,
            ..
        } = self
        else {
            return None;
        };
        let click_hit = match hit_node {
            Some(id) => ClickHit::Node(id.clone(), *hit_section_idx),
            None => ClickHit::Empty,
        };
        Some(dispatch::DispatchHit {
            click_hit,
            canvas_pos: *start_canvas,
            held_by: Some(crate::application::platform::input::MouseButton::Right),
        })
    }

    /// Where the pending press began, in surface pixels: the state a
    /// cursor move can promote to a drag. A promoted gesture starts
    /// here, not at the move that crossed the threshold
    /// (`event_cursor_moved::start_promoted_gesture`). `None` for
    /// every state no move promotes.
    #[cfg(not(target_arch = "wasm32"))]
    fn press_origin(&self) -> Option<(f64, f64)> {
        match self {
            Self::Pending(press) => Some(press.start_pos),
            Self::PendingRight { start_pos, .. } => Some(*start_pos),
            _ => None,
        }
    }
}

/// Application root — owns the launch options and (on WASM only)
/// the pre-created winit `EventLoop` + canvas `Window`. Constructed
/// from `main.rs` via [`Application::new`]; control transfers to
/// winit on [`Application::run`].
///
/// **WASM variant.** WASM has to attach the canvas to the DOM
/// before the browser's main thread starts dispatching events, so
/// it pre-creates the window and the event loop in
/// [`Application::new`] and hands them to `run_wasm::run` together.
/// The `#[allow(deprecated)]` on the WASM constructor's
/// `event_loop.create_window(...)` call records this asymmetry —
/// ditto the `event_loop` and `window` fields, which only exist on
/// the WASM side.
#[cfg(target_arch = "wasm32")]
pub struct Application {
    options: Options,
    event_loop: EventLoop<()>,
    window: Arc<Window>,
}

/// Application root — owns the launch options. Constructed from
/// `main.rs` via [`Application::new`]; control transfers to winit
/// on [`Application::run`].
///
/// **Native variant.** Native creates the window inside winit's
/// `ApplicationHandler::resumed` callback (the modern winit 0.30
/// path), so the struct here only carries [`Options`]. The window
/// itself lives on the run-loop's `InitState`, materialized lazily
/// on first resume.
#[cfg(not(target_arch = "wasm32"))]
pub struct Application {
    options: Options,
}

impl Application {
    #[cfg(target_arch = "wasm32")]
    pub fn new(options: Options) -> Self {
        let event_loop = EventLoop::new().expect("Could not create an EventLoop");

        // Pre-creating the window here on winit 0.30 is deprecated in
        // favor of `ActiveEventLoop::create_window` inside
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

    /// Run the app until it ends, and say how: under a window when it
    /// has one, with none under `--headless`. A one-shot launch's code
    /// says whether its capture was written; every other launch ends
    /// in success.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run(self) -> std::process::ExitCode {
        if self.options.headless {
            run_headless::run(self.options)
        } else {
            run_native::run(self)
        }
    }

    /// Hand the page to the browser's event loop, which outlives this
    /// call; the page is the app's lifetime, not the process.
    #[cfg(target_arch = "wasm32")]
    pub fn run(self) -> std::process::ExitCode {
        run_wasm::run(self);
        std::process::ExitCode::SUCCESS
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn into_options(self) -> Options {
        self.options
    }
}

/// Launch options assembled by `main.rs` from CLI flags + env
/// detection, frozen into [`Application`] at startup. Read once
/// per launch; never mutated post-construction.
#[derive(Clone)]
pub struct Options {
    /// Path to the `.mindmap.json` file to load at startup.
    /// Native: filesystem path; WASM: a fetch-relative URL
    /// resolved against the page origin.
    pub mindmap_path: String,
    /// The user's keybinding configuration (already loaded from file or
    /// defaults). The event loop resolves this into a `ResolvedKeybinds`
    /// at startup and dispatches keyboard events through it.
    pub keybind_config: crate::application::keybinds::KeybindConfig,
    /// `--ipc <path>`: the Unix socket to serve the IPC protocol on, or
    /// on Windows the name of the pipe, `\\.\pipe\mandala-<name>`
    /// (`format/ipc.md`). `None` — the default — means no endpoint and
    /// no IPC thread. Native-only: browsers cannot serve either.
    #[cfg(not(target_arch = "wasm32"))]
    pub ipc_socket: Option<std::path::PathBuf>,
    /// `--headless`: no window, no display and no event loop — the
    /// headless driver (`run_headless`) runs the app instead of winit,
    /// with IPC and the one-shot as its only inputs and outputs.
    #[cfg(not(target_arch = "wasm32"))]
    pub headless: bool,
    /// `--size <W>x<H>`: the size, in pixels, of what frames are drawn
    /// for — the window's inner size, or the headless target's. `None`
    /// lets the window open at the size it opens at, and a headless app
    /// take `run_native_init::HEADLESS_SURFACE_SIZE`.
    #[cfg(not(target_arch = "wasm32"))]
    pub surface_size: Option<(u32, u32)>,
    /// `--adapter`: which GPU adapters the renderer may run on.
    #[cfg(not(target_arch = "wasm32"))]
    pub adapter: crate::application::renderer::AdapterPolicy,
    /// `--screenshot <path>`: capture the map once it has settled, write
    /// it there, and exit. The loop's exit condition — "settled on a
    /// rendered frame, then exit" — is this field, in either driver.
    #[cfg(not(target_arch = "wasm32"))]
    pub one_shot: Option<OneShot>,
}

/// A one-shot launch's capture (`--screenshot`): the window's view, at
/// the surface size, written with its geometry sidecar to `path`, after
/// which the process exits — `0` when it was written, `1` when it could
/// not be (`format/ipc.md` §"Headless and one-shot launches").
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneShot {
    /// Absolute: the command line's path read against the working
    /// directory, as a shell reads it.
    pub path: std::path::PathBuf,
    /// `--overwrite`: replace a file already at `path` (and its
    /// sidecar), which the write guard otherwise refuses.
    pub overwrite: bool,
}

/// The node tree and every canvas role tree for `doc` at
/// `camera_zoom`, projected through the same `CanvasFrame` passes a
/// full rebuild runs — the drawn scene, minus the GPU. For tests
/// outside `app` (the IPC `scene` family's) that need a real scene
/// without a renderer (TEST_CONVENTIONS §T8).
#[cfg(test)]
pub(crate) fn project_scene_for_tests(
    doc: &crate::application::document::MindMapDocument,
    camera_zoom: f32,
) -> (
    baumhard::mindmap::tree_builder::MindMapTree,
    crate::application::scene_host::AppScene,
) {
    let mut app_scene = crate::application::scene_host::AppScene::new();
    let mut cache = baumhard::mindmap::scene_cache::SceneConnectionCache::default();
    let offsets = std::collections::HashMap::new();
    scene_rebuild::CanvasFrame::new(
        doc,
        &offsets,
        InteractionMode::Default.resize_handle_overrides(),
        camera_zoom,
    )
    .update_all(&mut cache, &mut app_scene);
    (doc.build_tree(), app_scene)
}

// Unit tests for pure helpers (cursor math, caret insertion,
// double-click detection, Baumhard mutation round-trip). What needs
// the renderer is `tests_gpu`'s, below.

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests;

// The GPU-backed half of the suite (TEST_CONVENTIONS §T8): a whole
// native app with no window on whatever adapter the machine has —
// Mesa's lavapipe on a GPU-less box — skipped with a note where there
// is none, and never skipped in CI.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod gpu_harness;
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests_gpu;
