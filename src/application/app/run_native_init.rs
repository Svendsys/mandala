// SPDX-License-Identifier: MPL-2.0

//! The native app's state, [`InitState`], and its first-run
//! initialization, [`build`] — called once by whichever native driver
//! runs: `run_native::NativeApp::resumed` when the window exists, or
//! `run_headless::run` at launch, with no window at all.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use crate::application::platform::input::Modifiers as ModifiersState;
use pollster::block_on;
use winit::dpi::PhysicalSize;
use winit::window::CursorIcon;

use super::console_input::load_console_history;
use super::freeze_watchdog::FreezeWatchdog;
use super::heartbeat;
use super::input_context::InputHandlerContext;
use super::settle::RedrawLedger;
use super::shell::Shell;
use super::startup_load;
use super::{DragState, InteractionMode, LastClick, Options};
use crate::application::capture::artifacts::Artifacts;
use crate::application::capture::record::Recorder;
use crate::application::clock::{AppClock, ClockMode};
use crate::application::common::{FpsDisplayMode, RenderDecree};
use crate::application::console::ConsoleState;
use crate::application::document::MindMapDocument;
use crate::application::ipc::injected::InjectedQueue;
use crate::application::ipc::Settling;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::renderer::Renderer;
use crate::application::view::MapView;

/// Wall-clock interval the loop holds the FPS overlay's numeric
/// reading on screen after the last live frame before flipping it to
/// the "-" idle marker ([`InitState::fps_idle`]). Without this grace,
/// the gaps between bursts of input (where `needs_continuation` is
/// briefly false) would flicker the overlay between the live reading and
/// "-" several times per second, defeating the readout's diagnostic
/// value. One second is long enough to read the number yet short enough
/// that genuine idle visibly transitions to "-".
const FPS_IDLE_GRACE: std::time::Duration = std::time::Duration::from_secs(1);

/// The native app's whole state: what every handler, every frame and
/// every IPC command reads and writes. Built by [`build`] — once the
/// window exists under winit (`run_native::NativeApp::resumed`), at
/// launch with no window (`run_headless`) — and driven from there by
/// whichever of the two drivers built it, through the one loop body
/// both run (`app::pump`).
pub(super) struct InitState {
    /// What the app is shown in: a window, or nothing
    /// (`app::shell`).
    pub(super) shell: Shell,
    pub(super) renderer: Renderer,
    pub(super) document: Option<MindMapDocument>,
    /// The document's view (`crate::application::view`): `Some` exactly
    /// when `document` is, installed with it.
    pub(super) view: Option<MapView>,
    pub(super) app_scene: crate::application::scene_host::AppScene,
    pub(super) cursor_pos: (f64, f64),
    pub(super) drag_state: DragState,
    pub(super) interaction_mode: InteractionMode,
    pub(super) console_state: ConsoleState,
    pub(super) console_history: Vec<String>,
    pub(super) color_picker_state: crate::application::color_picker::ColorPickerState,
    pub(super) last_click: Option<LastClick>,
    pub(super) modifiers: ModifiersState,
    pub(super) cursor_is_hand: bool,
    /// Last cursor icon set through the shell (`Shell::set_cursor`,
    /// which is `Window::set_cursor` under a window and nothing with
    /// none). Used by the cursor_moved handler to dedup redundant
    /// `set_cursor` calls — winit dedupes these on macOS / X11 but NOT
    /// on Windows (every call → `LoadCursorW` + `SetCursor` + mutex
    /// lock) or Wayland (calls into pointer manager every time), so the
    /// per-event cursor icon update needs an application-side gate.
    /// Initialized to `Default` to match the as-launched cursor.
    pub(super) cursor_icon_last: CursorIcon,
    pub(super) keybinds: ResolvedKeybinds,
    /// User-defined macro registry. Loaded once at startup
    /// (`run_native_init::build`) from `~/.config/mandala/macros.json`;
    /// queried at dispatch time via `keybinds.macro_for(...)`.
    pub(super) macros: crate::application::macros::MacroRegistry,
    /// The edits' App and User tiers and the handlers, loaded once at
    /// startup (`application::edits`); a map's own are its document's.
    pub(super) edits: crate::application::edits::EditRegistry,
    /// Touch gesture state machine. Fed by `WindowEvent::Touch`
    /// events (winit); emits a tap, a long-press, a one-finger pan
    /// step or a two-finger pinch step. Cross-platform peer of WASM's
    /// `WasmInputState.touch_recognizer`. See `app/touch_gesture.rs`
    /// for the machine and CONCEPTS §5 for where each emission goes.
    pub(super) touch_recognizer: super::touch_gesture::TouchGestureRecognizer,
    /// Raw input the IPC `input` family queued for the heartbeat,
    /// applied one sample per frame ahead of `drain_inputs`
    /// (`format/ipc.md` §`input`). Always empty without `--ipc`.
    pub(super) injected: InjectedQueue,
    /// Where a screenshot lands when its caller names no path: made on
    /// first use, bounded, and removed in `NativeApp::exiting`
    /// (`format/ipc.md` §`capture`, "Artifact lifecycle").
    pub(super) artifacts: Artifacts,
    /// The frame-sequence recording in progress, or how the last one
    /// ended (`app::record_flow`). App state, not session state: it
    /// outlives the client that started it.
    pub(super) recorder: Recorder,
    /// A handle on the event loop's freeze watchdog, for main-thread
    /// work that runs long while making progress — a screenshot ticks
    /// it per band and per encoded chunk (`app::capture_flow`), a
    /// clock step per heartbeat (`app::heartbeat`).
    pub(super) watchdog: FreezeWatchdog,
    /// The app clock every behavior reads (`crate::application::clock`).
    /// App state, not session state: a virtual clock stays virtual
    /// across the client that set it, so `mandalactl`'s one command per
    /// connection can drive it.
    pub(super) clock: AppClock,
    /// Every redraw asked for and the last one drawn: the "rendered"
    /// half of `clock.wait until=settled` (`app::settle`).
    pub(super) redraws: RedrawLedger,
    /// `Action::Quit` was dispatched: the loop ends at its next exit
    /// check (`app::pump::iterate`), gracefully, in either driver.
    pub(super) quit_requested: bool,
}

impl InitState {
    /// Build the [`InputHandlerContext`] view over this state for a
    /// single dispatcher call. Rebuilt per event because the
    /// returned borrow is tied to `&mut self` — `'_` expires as
    /// soon as the handler returns.
    pub(super) fn input_context(&mut self) -> InputHandlerContext<'_> {
        InputHandlerContext {
            document: &mut self.document,
            view: &mut self.view,
            app_scene: &mut self.app_scene,
            renderer: &mut self.renderer,
            drag_state: &mut self.drag_state,
            interaction_mode: &mut self.interaction_mode,
            console_state: &mut self.console_state,
            console_history: &mut self.console_history,
            color_picker_state: &mut self.color_picker_state,
            last_click: &mut self.last_click,
            cursor_pos: &mut self.cursor_pos,
            modifiers: &mut self.modifiers,
            cursor_is_hand: &mut self.cursor_is_hand,
            cursor_icon_last: &mut self.cursor_icon_last,
            keybinds: &self.keybinds,
            macros: &mut self.macros,
            edits: &self.edits,
            artifacts: &mut self.artifacts,
            recorder: &mut self.recorder,
            watchdog: &self.watchdog,
            shell: &self.shell,
            touch_recognizer: &mut self.touch_recognizer,
            injected: &mut self.injected,
            clock: &mut self.clock,
            quit_requested: &mut self.quit_requested,
        }
    }

    /// Ask the shell for a frame, and write the ask into the
    /// [`RedrawLedger`]. Every redraw the app requests goes through
    /// here, so the ledger's "drawn" means "everything asked for so far
    /// has been drawn".
    pub(super) fn request_redraw(&mut self) {
        self.redraws.ask();
        self.shell.request_redraw();
    }

    /// Draw the frame asked for — the sole entry to the render path,
    /// run when winit delivers `RedrawRequested`, or when the headless
    /// driver finds a frame owed (`RedrawLedger::frame_owed`) — and tell
    /// the ledger how it went, which is what `clock.wait until=settled`
    /// and a one-shot capture read. Asks coalesce: one frame draws every
    /// ask before it.
    pub(super) fn draw_frame(&mut self) {
        let asked = self.redraws.asked();
        match self.renderer.process() {
            Ok(()) => self.redraws.drew(asked),
            Err(skip) => {
                self.redraws.skipped(skip.is_transient());
                // Nothing else would ask again with no input to follow,
                // leaving the window — and a `settled` wait — on the
                // last frame drawn. A lasting skip is not retried:
                // retrying would spin, and the redraw stays owed.
                if skip.is_transient() {
                    self.request_redraw();
                }
            }
        }
    }

    /// What `clock.wait` reads at an evaluation point
    /// (`format/ipc.md` §`clock.wait`).
    pub(super) fn settling(&self) -> Settling {
        Settling {
            continuing: self.needs_continuation(),
            redraw_owed: !self.redraws.is_current(),
            virtual_clock: self.clock.is_virtual(),
        }
    }

    /// The FPS overlay's active-to-idle transition, given whether work
    /// continues: when the loop is about to park and the overlay still
    /// shows a live reading, flip it to the idle marker so it reads
    /// "FPS: -" while the app sleeps, and ask for the frame that shows
    /// it. The flip waits out [`FPS_IDLE_GRACE`] after the last frame so
    /// the gaps between bursts of input don't flicker the overlay
    /// between numeric and "-" — the user needs a stable readout to
    /// actually use the diagnostic. Returns the instant the
    /// grace ends while it has not, which the loop wakes at to commit
    /// the flip if no event arrived in the meantime.
    pub(super) fn fps_idle(&mut self, continuing: bool) -> Option<std::time::Instant> {
        if continuing || self.renderer.fps_display_mode() == FpsDisplayMode::Off {
            return None;
        }
        let deadline = self.renderer.fps_idle_defer_deadline(FPS_IDLE_GRACE);
        if deadline.is_none() && self.renderer.has_live_fps() {
            self.renderer.set_fps_idle();
            self.request_redraw();
        }
        deadline
    }

    /// The loop is about to park with a recording running: encode what
    /// waits in the time before its next frame (`app::record_flow::idle`).
    pub(super) fn record_idle(&mut self) {
        super::record_flow::idle(
            &mut self.recorder,
            &mut self.artifacts,
            &self.watchdog,
            &self.clock,
        );
    }

    /// True iff the event loop should keep iterating without
    /// further input: [`heartbeat::needs_continuation`] over this
    /// state. Drives the choice between `ControlFlow::Wait` (idle,
    /// park until next OS event) and `ControlFlow::Poll` (continuous
    /// frames).
    pub(super) fn needs_continuation(&self) -> bool {
        heartbeat::needs_continuation(&self.injected, &self.clock, self.document.as_ref())
    }
}

/// Build the fully-initialized [`InitState`] around `shell`: a freshly
/// created window, or none (`--headless`), whose frames are
/// `options.surface_size` pixels large — [`HEADLESS_SURFACE_SIZE`] when
/// the launch named none.
///
/// A load failure is not a missing document: it resolves through
/// [`startup_load::native_startup_document`] to the load-failure
/// placard, which is installed by the same code that installs a real
/// map. Everything
/// below the load therefore runs unconditionally — the window opens
/// with the loader's message on the canvas, the camera fits it,
/// input works, and `open` can be typed into the console without
/// restarting the process.
pub(super) fn build(options: &Options, shell: Shell, watchdog: FreezeWatchdog) -> InitState {
    baumhard::font::fonts::init();

    let (mut renderer, size) = match &shell {
        Shell::Window(window) => (
            block_on(Renderer::bootstrap_native(Arc::clone(window), options.adapter)),
            window.inner_size(),
        ),
        Shell::Headless => {
            let (width, height) = options.surface_size.unwrap_or(HEADLESS_SURFACE_SIZE);
            let size = PhysicalSize::new(width, height);
            (
                block_on(Renderer::bootstrap_headless(size, options.adapter)),
                size,
            )
        }
    };

    // Configure initial surface size.
    renderer.process_decree(RenderDecree::SetSurfaceSize(size.width, size.height));

    // The screen-space overlays' scene: the console and the color
    // picker register their trees in it while they are open.
    let app_scene = crate::application::scene_host::AppScene::new();

    // The user's settings, before any document: the history limit is
    // the document's from the moment it is installed (D18).
    let settings = crate::application::user_config::settings::load();

    // The initial map load, in one call. A rejection is not a
    // missing document: `startup_load` turns it into the load-failure
    // placard carrying the loader's own message, and everything below
    // installs that placard by the same code that installs a real
    // map. The `Result` never reaches this file, which is what keeps
    // the two targets from reporting the same failure differently —
    // the browser's peer call is `wasm_startup_document`.
    let mut doc = startup_load::native_startup_document(&options.mindmap_path, settings.history_limit);

    // The one full projection of the document's life (§3 "Load"), the
    // renderer loaded with it and cleared to the map's background, and
    // the camera fit to what it draws. The browser's load path makes the
    // same calls.
    let view = MapView::load(&mut doc, &mut renderer);
    renderer.fit_camera_to_content();

    // Start rendering.
    renderer.process_decree(RenderDecree::StartRender);

    // Pre-warm the render pipeline: one full render cycle so the
    // wgpu driver compiles pipeline shaders, the swapchain
    // allocates its first backing image, and the glyph atlas is
    // populated before the first user-driven frame. Without this,
    // those costs (commonly 50-300ms total on first Vulkan/Metal
    // pipeline bind) would land on the user's first interaction.
    renderer.prewarm();

    let keybinds: ResolvedKeybinds = options.keybind_config.resolve();
    // Cross-session history loaded from disk on startup; appended
    // to on every Enter; written back on close.
    let console_history: Vec<String> = load_console_history();

    // The App and User tiers from the platform loaders. Body is
    // `macros::loader::build_macro_registry`, which the browser's init
    // calls too. A map carries no macros: its verbs replace them (§11.3).
    let macros = crate::application::macros::loader::build_macro_registry();
    // The edits' App and User tiers, on both targets the same way.
    let edits = crate::application::edits::EditRegistry::load();

    // Real, reading the wall clock, for a launch a person watches:
    // only `clock.set_mode` or the `clock` verb leaves it. Virtual with
    // no window, and for a one-shot capture: nobody is watching time
    // pass, so time moves only when a `clock.step` moves it — a
    // stepped run is a pure function of its commands, and the one-shot
    // captures the map at its first instant (`format/ipc.md` §"Headless
    // and one-shot launches").
    let mut clock = AppClock::default();
    if matches!(shell, Shell::Headless) || options.one_shot.is_some() {
        clock.set_mode(ClockMode::Virtual);
    }

    InitState {
        shell,
        renderer,
        document: Some(doc),
        view: Some(view),
        app_scene,
        cursor_pos: (0.0, 0.0),
        drag_state: DragState::None,
        interaction_mode: InteractionMode::Default,
        console_state: ConsoleState::Closed,
        console_history,
        color_picker_state: crate::application::color_picker::ColorPickerState::Closed,
        last_click: None,
        modifiers: ModifiersState::empty(),
        // True while the cursor is over a node whose click runs a verb.
        // Tracked so we only call set_cursor on transitions instead of
        // every CursorMoved event.
        cursor_is_hand: false,
        // Last cursor icon written via Window::set_cursor — used by
        // the cursor_moved handler to skip redundant per-frame
        // set_cursor calls on platforms (Windows, Wayland) where
        // winit doesn't dedup. Initialized to Default to match the
        // as-launched cursor.
        cursor_icon_last: CursorIcon::Default,
        keybinds,

        macros,
        edits,
        // Touch gesture recognizer. State machine starts Idle;
        // first `WindowEvent::Touch` lands a finger.
        touch_recognizer: super::touch_gesture::TouchGestureRecognizer::new(),
        injected: InjectedQueue::default(),
        artifacts: Artifacts::for_this_process(),
        recorder: Recorder::default(),
        watchdog,
        clock,
        redraws: RedrawLedger::default(),
        quit_requested: false,
    }
}

/// The size of a headless app's frames when the launch names none
/// (`--size`): the size a winit window that asks for none was observed
/// to open at under Xvfb, so a headless capture and a windowed one of
/// the same map frame the same view.
pub(super) const HEADLESS_SURFACE_SIZE: (u32, u32) = (800, 600);
