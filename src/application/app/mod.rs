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
//! **The map is drawn through one pipeline.** A handler that changed
//! the document or the selection ends by syncing the view
//! (`crate::application::view::MapView::sync`): the document's change
//! feed drained once, the projector updated, the overlays re-stamped,
//! the renderer applied. A camera change syncs nothing.
//!
//! **Modal state machines.** `console_input` steals keyboard input
//! while the console is open (the §3 carve-out for modals that own the
//! literal `winit::Key` payload). The map's own editors return with
//! V1-C8 (#249), the color picker with V1-C7 (#248).
//!
//! **Cross-platform shape.** Pure logic (gesture recognition,
//! viewport math, hit testing, `Action` resolution) lives in
//! `cfg`-untagged free functions so it compiles for both
//! native and WASM. The native vs. WASM divergence is largely
//! confined to the run-loop entry point; cross-platform
//! `Action` arms route through [`dispatch::action_core`]'s
//! `dispatch_compatible`. See `work_plans/WASM_CONVERGENCE.md` for the
//! current convergence status.

// Dispatch funnel — `cross_dispatch` (shared apply_* helpers),
// `action_core` (Compatible-Action dispatcher), `macro_core`
// (cross-platform macro step loop + privilege gate), `families`
// (the Actions an owner issue rebuilds) and `native` (native
// dispatch_action wrapper that adds the NativeOnly arm match). The
// directory's `mod.rs` re-exports the public surface so callers stay
// terse.
pub(crate) mod dispatch;

// Native-only — interactive modal state machines absent on WASM.
// See CLAUDE.md "Dual-target status".
#[cfg(not(target_arch = "wasm32"))]
mod capture_flow;
// The picker's open, draw and input paths, kept for V1-C7 (#248) to
// put on a transaction: nothing opens the picker until then
// (`Action::OpenColorPicker` is unhandled, the `color pick` console
// form refuses), so the whole module is unreachable. The `expect`
// fires the day it is reachable again, and goes with it.
#[cfg(not(target_arch = "wasm32"))]
#[expect(
    dead_code,
    reason = "the color picker returns on a transaction with V1-C7 (#248)"
)]
mod color_picker_flow;
#[cfg(not(target_arch = "wasm32"))]
mod console_input;
#[cfg(not(target_arch = "wasm32"))]
mod drain_frame;
#[cfg(not(target_arch = "wasm32"))]
mod event_cursor_moved;
#[cfg(not(target_arch = "wasm32"))]
mod event_keyboard;
#[cfg(not(target_arch = "wasm32"))]
mod event_mouse_click;
#[cfg(not(target_arch = "wasm32"))]
mod freeze_watchdog;
#[cfg(not(target_arch = "wasm32"))]
mod heartbeat;
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
mod step;
// Cross-platform context-bundles for the unified `dispatch_action`
// funnel. Track C from `work_plans/WASM_CONVERGENCE.md` (final convergence step).
mod input_context_core;
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
// Cross-platform: both init paths resolve the initial map load
// through it, which is what keeps the two from surfacing a load
// failure differently (CODE_CONVENTIONS §4).
mod startup_load;
mod touch_gesture;

// Drift surface for a new `InputHandlerContext` field: the struct in
// `input_context.rs`, the `InitState::input_context()` builder in
// `run_native_init.rs`, and `dispatch_action`'s signature in
// `dispatch/native.rs`.

#[cfg(not(target_arch = "wasm32"))]
use glam::Vec2;

#[cfg(target_arch = "wasm32")]
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use winit::{event_loop::EventLoop, window::Window};

use baumhard::arena::NodeId;

use crate::application::document::selection::selectable;
use crate::application::document::MindMapDocument;
use crate::application::view::MapView;

/// Minimum pointer travel (**physical** pixels) before a press stops
/// reading as a click and starts reading as a drag. Splits "this
/// was a click" from "this is a drag" — small enough that
/// intentional drags engage immediately, large enough that
/// trembling fingers don't accidentally start a drag from a click.
///
/// **One threshold for every pointer**, mouse and finger alike:
/// the mouse arms in `event_cursor_moved.rs` (left-button
/// `Pending` and right-button `PendingRight`) and both emit points
/// of [`touch_gesture::TouchGestureRecognizer`] (the long-press
/// cancel and the two-finger centroid step) compare against
/// [`POINTER_DRAG_THRESHOLD_SQ_PX`] below.
///
/// **Physical, not logical**, and both readers say so because both
/// paths feed the comparison winit's raw device coordinates:
/// `handle_cursor_moved` stores `WindowEvent::CursorMoved`'s
/// `PhysicalPosition` into `cursor_pos` unconverted, and
/// `dispatch_touch_event` hands the recognizer `touch.location`, a
/// `PhysicalPosition` too. Parity is unaffected — both pointers are
/// measured with the same ruler, which is the property this constant
/// exists for — but the *number* scales with the display: on a 2x
/// screen 5 physical px is 2.5 logical px of slop, not 5.
const POINTER_DRAG_THRESHOLD_PX: f64 = 5.0;

/// [`POINTER_DRAG_THRESHOLD_PX`] squared — the form every
/// comparison actually uses, so no pointer path pays a `sqrt` per
/// motion event. Derived rather than written out so the two cannot
/// drift.
const POINTER_DRAG_THRESHOLD_SQ_PX: f64 = POINTER_DRAG_THRESHOLD_PX * POINTER_DRAG_THRESHOLD_PX;

/// What a single click targeted: the element a click there selects
/// ([`selectable`]: a node, a connector or a link), or nothing. Two
/// clicks "match" as a double-click only when they hit the same thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClickHit {
    /// No element under the cursor.
    Empty,
    /// The element under the cursor that a click selects.
    Element(NodeId),
}

impl ClickHit {
    /// The element hit, if any. Native: what reads it is the drag state's
    /// wire form (`app::ipc_state`), and the browser has no drag state.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn element(self) -> Option<NodeId> {
        match self {
            ClickHit::Empty => None,
            ClickHit::Element(id) => Some(id),
        }
    }
}

/// What a click at `canvas_pos` hits, through the projector's hit test
/// (`MapView::hit`): never by reading a glyph layer for document state.
/// The one hit chain both targets' click and tap paths run.
pub(crate) fn compute_click_hit(
    doc: Option<&MindMapDocument>,
    view: Option<&mut MapView>,
    canvas_pos: glam::Vec2,
) -> ClickHit {
    match (doc, view) {
        (Some(doc), Some(view)) => view.hit(canvas_pos).map_or(ClickHit::Empty, |hit| {
            ClickHit::Element(selectable(doc.doc(), &hit))
        }),
        _ => ClickHit::Empty,
    }
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
    /// What the first click landed on.
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

/// Warn once per latch that an input resolved to an Action the
/// browser has no body for, so the gesture is a no-op. Returns
/// whether this call is the one that emitted.
///
/// Every input class that consults the keybind table needs this. A
/// user who binds a `NativeOnly` Action to a gesture gets *literally
/// nothing* otherwise — no log, no chrome, no model change. `warned` is
/// a per-call-site latch so one input class going quiet cannot silence
/// another; `remedy` names what the user can do about it and what
/// unblocks parity.
///
/// **Cross-platform on purpose, though every caller is under
/// `run_wasm/`.** Nothing in the body is browser-specific, and while it
/// lived inside the `#![cfg(target_arch = "wasm32")]` module `cargo
/// test` could not reach it, which is what §T9 forbids for
/// platform-shared logic. The `bool` return is what makes the one-shot
/// semantics assertable without a global log sink.
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

// Re-export the mode enum so the console layer can carry
// `InteractionMode` inside `ConsoleSideEffect` and consumers across
// `application::*` reach a uniform path.
pub(in crate::application) use interaction_mode::InteractionMode;

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
    /// left press or focus loss.
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
/// The Action cannot see which input reached it: `dispatch_action`
/// strips the gesture down to an [`Action`](crate::application::keybinds::Action).
/// So the arm arms every pan [`PanOwner::Toggled`], and a held input
/// that dispatched the arming Action claims it afterwards
/// (`dispatch::native::dispatch_held_action`, through
/// [`DragState::hand_armed_pan_to`]).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum PanOwner {
    /// Momentary: ends on this input's release, and on no other
    /// release.
    Held(HeldInput),
    /// Modal: armed by an input with no release. Ends on the next
    /// `Action::PanCanvas` that finds it.
    Toggled,
}

/// Tracks the current drag interaction state.
///
/// The drags that edit the map — moving a node or a section, resizing,
/// the connector handles and the labels — return as transactions with
/// V1-C7 (#248). Until then a left drag dispatches whatever `LeftDrag`
/// is bound to (by default `Action::PanCanvas`), wherever it starts,
/// and a Shift+drag on empty canvas is the rubber band.
#[cfg(not(target_arch = "wasm32"))]
enum DragState {
    /// No drag in progress.
    None,
    /// The left button is down and the cursor has not moved past the
    /// drag threshold yet.
    Pending {
        /// Where the press was, in surface pixels.
        start_pos: (f64, f64),
        /// What the press landed on.
        hit: ClickHit,
    },
    /// The right button is down and the cursor has not moved past the
    /// drag threshold yet. The threshold cross dispatches whatever
    /// `MouseGesture::RightDrag` is bound to with a hit built from the
    /// press ([`Self::right_drag_hit`]); a release without movement fires
    /// the `MouseGesture::RightClick` binding.
    PendingRight {
        start_pos: (f64, f64),
        /// Canvas-space cursor position at press time, so a gesture
        /// anchored at the press is anchored where the user pressed.
        start_canvas: glam::Vec2,
        /// What the press landed on.
        hit: ClickHit,
    },
    /// Panning the camera: every cursor move emits a `CameraPan`
    /// decree directly. No tree or document is touched.
    ///
    /// `owner` is which release ends it — see [`PanOwner`].
    Panning { owner: PanOwner },
    /// Shift+drag on empty space: rubber-band selection rectangle.
    ///
    /// **This variant is the authority** for the rectangle drawn and for
    /// the covered set the selection highlight tints
    /// (`MapView::set_band_targets`): `drain_frame::drain_rect_select`
    /// re-derives both from it every frame, so no exit from this state
    /// has to remember to clean up.
    SelectingRect {
        /// Canvas-space corner where the drag started.
        start_canvas: Vec2,
        /// Canvas-space corner at current cursor position.
        current_canvas: Vec2,
    },
}

#[cfg(not(target_arch = "wasm32"))]
impl DragState {
    /// True when overwriting this state with a freshly-armed gesture
    /// would abandon one that nothing else will finish: a
    /// `PendingRight` holds a user-named gesture (`RightClick`, or
    /// whatever `RightDrag` is bound to) that has not fired yet.
    /// `Pending` / `Panning` / `SelectingRect` owe nothing and are
    /// re-derived from the next press or the next cursor sample.
    fn would_abandon_gesture(&self) -> bool {
        matches!(self, Self::PendingRight { .. })
    }

    /// True when this is a pan `input` holds, i.e. one that `input`'s
    /// release ends. A [`PanOwner::Toggled`] pan is held by nothing,
    /// so no release ends it.
    fn is_pan_held_by(&self, input: &HeldInput) -> bool {
        matches!(self, Self::Panning { owner: PanOwner::Held(held) } if held == input)
    }

    /// End the pan `input` holds, if this is one; leave every other
    /// state — any other pan included — exactly as it is.
    fn end_pan_held_by(&mut self, input: &HeldInput) {
        if self.is_pan_held_by(input) {
            *self = Self::None;
        }
    }

    /// Hand a pan the dispatch just armed to the held `input` that
    /// dispatched it. `was_panning` is the state *before* the dispatch,
    /// and is the whole of the "just armed" test: a dispatch that
    /// started from a pan cannot have armed one, so a toggled pan found
    /// afterwards is the one that was already running and is not this
    /// input's to claim.
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
    /// deliver, so none is left waiting for it. A press whose release
    /// never came is not a click, so neither the click nor the
    /// `RightClick` binding fires; a pan and a rubber band end.
    fn end_on_focus_loss(&mut self) {
        *self = Self::None;
    }

    /// The hit a `RightDrag` threshold cross dispatches with, built from
    /// the pending right press: its press-time hit, its press-time
    /// canvas position, and the right button as the holder. `None` for
    /// every other state.
    fn right_drag_hit(&self) -> Option<dispatch::DispatchHit> {
        let Self::PendingRight {
            start_canvas, hit, ..
        } = self
        else {
            return None;
        };
        Some(dispatch::DispatchHit {
            click_hit: *hit,
            canvas_pos: *start_canvas,
            held_by: Some(crate::application::platform::input::MouseButton::Right),
        })
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
    /// Path to the `.mandala.json` file to load at startup.
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

// Unit tests for pure helpers (cursor math, double-click detection).
// What needs the renderer is `tests_gpu`'s, below.
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
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests_gpu_background;
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests_gpu_demo_checker;
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests_gpu_demo_maps;
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests_gpu_glyph_runs;
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests_gpu_layers;
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests_gpu_structure;
