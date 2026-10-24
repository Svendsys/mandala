// SPDX-License-Identifier: MPL-2.0

//! Native event-loop body for [`super::Application::run`]. Uses
//! winit 0.30's `ApplicationHandler`; first-time init lives in
//! [`super::run_native_init::build`].

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use baumhard::mindmap::tree_builder::MindMapTree;
use winit::application::ApplicationHandler;
use winit::event::{Event, KeyEvent, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Window, WindowId};

use super::color_picker_flow::rebuild_color_picker_overlay;
use super::freeze_watchdog::FreezeWatchdog;
use super::heartbeat::{self, Beat};
use super::input_context::InputHandlerContext;
use super::ipc_target::IpcCommandTarget;
use super::run_native_init;
use super::settle::RedrawLedger;
use super::single_line_edit::SingleLineEditor;
use super::text_edit::TextEditState;
use super::{
    event_cursor_moved, event_keyboard, event_mouse_click, Application, DragState, InteractionMode,
    LastClick, Options,
};
use crate::application::capture::artifacts::Artifacts;
use crate::application::capture::record::Recorder;
use crate::application::clock::AppClock;
use crate::application::common::RenderDecree;
use crate::application::console::ConsoleState;
use crate::application::document::MindMapDocument;
use crate::application::ipc::injected::InjectedQueue;
use crate::application::ipc::{self, DrainOutcome, IpcServer, IpcWake, Settling};
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::platform::input::{ElementState, Key, MouseButton, MouseScrollDelta, TouchPhase};
use crate::application::platform::window::PhysicalPosition;
use crate::application::renderer::Renderer;

/// Wall-clock interval the event loop holds the FPS overlay's
/// numeric reading on screen after the last live frame before
/// flipping it to the "-" idle marker. Without this grace, an
/// active throttled drag's between-drain gaps (where
/// `needs_continuation` is briefly false while the throttle holds
/// the next drain) would flicker the overlay between the live
/// reading and "-" several times per second, defeating the
/// readout's diagnostic value. One second is long enough to read
/// the number on every drain frame yet short enough that genuine
/// idle visibly transitions to "-".
const FPS_IDLE_GRACE: std::time::Duration = std::time::Duration::from_secs(1);

/// Entry point called from `Application::run` on every non-WASM
/// target. Hands control to winit's event loop; returns when the
/// window is closed.
///
/// Spawns the freeze watchdog before handing off to winit so it
/// can catch a hang anywhere after the window is created, not just
/// inside `drain_inputs`. See
/// [`super::freeze_watchdog::FreezeWatchdog`] for the rationale —
/// short version: Mandala is single-threaded and a same-thread
/// `std::sync::RwLock` re-entry deadlock would otherwise hang
/// silently forever.
///
/// Starts the IPC listener beside it when `--ipc` was passed — before
/// the event loop takes over, so a bad socket path fails the launch
/// rather than the first client. The loop is built with [`IpcWake`]
/// as its user event either way; without the flag no proxy exists and
/// none is ever sent.
pub(super) fn run(app: Application) {
    let event_loop: EventLoop<IpcWake> = EventLoop::with_user_event()
        .build()
        .expect("Could not create an EventLoop");
    let options = app.into_options();
    let ipc = options.ipc_socket.as_deref().map(|path| {
        let proxy = event_loop.create_proxy();
        // A send fails only once the loop has exited, when there is
        // nothing left to wake.
        let wake = move || {
            let _ = proxy.send_event(IpcWake);
        };
        IpcServer::start(path, wake)
            .expect("ipc: the `--ipc` listener could not start (format/ipc.md \"Socket, flag, lifecycle\")")
    });
    let mut handler = NativeApp {
        options,
        init: None,
        watchdog: FreezeWatchdog::spawn(),
        ipc,
    };
    event_loop
        .run_app(&mut handler)
        .expect("Some kind of unexpected error appears to have taken place");
}

/// winit 0.30 `ApplicationHandler` implementor. Holds options
/// pre-resume; on the first `resumed()` it creates the window and
/// builds the fully-initialized [`InitState`]. Subsequent resume
/// callbacks (mobile resume-after-suspend) are idempotent thanks
/// to the `is_some()` guard.
struct NativeApp {
    options: Options,
    init: Option<InitState>,
    /// Freeze watchdog — ticked at the top of every `AboutToWait`
    /// drain and also on every window event, so a frame that
    /// hangs mid-drain or mid-event produces a diagnostic abort
    /// after `freeze_watchdog::FREEZE_THRESHOLD`.
    watchdog: FreezeWatchdog,
    /// The `--ipc` listener, when the flag was passed. Taken and shut
    /// down in [`ApplicationHandler::exiting`].
    ipc: Option<IpcServer>,
}

/// Execute queued IPC requests on the main thread, with the same access
/// an input handler has: each command gets an [`IpcCommandTarget`]
/// over the same state a winit handler is given.
/// `init` is `None` before the window exists, and a command that
/// needs the app then answers `no_document`. A free function over the
/// one field, so `about_to_wait` can call it while it holds
/// `InitState`.
fn drain_ipc(server: &mut Option<IpcServer>, init: Option<&mut InitState>) -> DrainOutcome {
    let Some(server) = server else {
        return DrainOutcome::default();
    };
    match init {
        Some(init) => {
            // Look at what the user just did before any command runs,
            // so a change of theirs is reported as theirs and not
            // folded into the first command's. Only when subscribed:
            // with nothing to report, an input handler's changes cost
            // no observation at all.
            if server.is_subscribed() {
                server.observe(&init.observed());
            }
            server.drain(|request, session| {
                let mut target = IpcCommandTarget::new(init);
                ipc::execute(request, Some(&mut target), session)
            })
        }
        None => server.drain(|request, session| ipc::execute(request, None, session)),
    }
}

/// Evaluate the controller's pending `clock.wait`, if any, against the
/// app as it stands (`format/ipc.md` §`clock.wait`). Called at the end
/// of every loop iteration, after its redraw has been asked for — so a
/// pixels-affecting command of that iteration counts as a frame still
/// owed — and after every window event, the frame's render among them,
/// which is when an owed frame stops being owed. A free function over
/// the one field, for the reason [`drain_ipc`] is.
fn evaluate_wait(server: &mut Option<IpcServer>, init: &InitState) {
    if let Some(server) = server {
        server.evaluate_wait(init.settling(), &init.observed());
    }
}

impl ApplicationHandler<IpcWake> for NativeApp {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _: StartCause) {
        // ControlFlow is set authoritatively at the end of every
        // `about_to_wait` based on `InitState::needs_continuation`.
        // No-op here so we don't override that decision on each
        // iteration.
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.init.is_some() {
            return;
        }
        let window = event_loop
            .create_window(Window::default_attributes())
            .expect("Failed to create application window");
        self.init = Some(run_native_init::build(
            &self.options,
            Arc::new(window),
            self.watchdog.clone(),
        ));
        // Ping once as soon as the window is up so the watchdog
        // knows the main loop has reached a live state. Before
        // this point, the watchdog treats the zeroed atomic as
        // "still initializing" and doesn't enforce the threshold.
        // `unparked` also clears any stale parked state from a
        // prior suspend cycle.
        self.watchdog.unparked();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        // The watchdog's "is the main thread alive" semantics are
        // event-handler-completion-based under ControlFlow::Wait:
        // legitimate idle (no events) is not a hang, only a stuck
        // handler is. `unparked` ticks the activity clock and
        // clears the parked atomic so a hang inside this handler
        // still trips the threshold.
        self.watchdog.unparked();
        if let Some(init) = self.init.as_mut() {
            init.handle_event(event_loop, Event::WindowEvent { window_id, event });
            evaluate_wait(&mut self.ipc, init);
        }
    }

    /// An IPC boundary thread has queued work. The wake's whole job is
    /// to unpark the loop: winit runs `about_to_wait` after every
    /// iteration that delivered events, and the queue is drained there,
    /// once, in its place in the frame. Ticks the watchdog like every
    /// other handler.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _: IpcWake) {
        self.watchdog.unparked();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.watchdog.unparked();
        let ipc_outcome;
        if let Some(init) = self.init.as_mut() {
            // Pre-snapshot the continuation predicate: if the loop
            // was *entering* this iteration with continuous work
            // (animations, throttled-drag pending, mid-zoom), the
            // drain will do work that needs to land on screen.
            let was_continuing = init.needs_continuation();
            // The frame up to the recorder (`app::heartbeat`). In real
            // time: one injected-input sample, ahead of the input drain
            // — where a winit event lands — so a paced IPC drag's moves
            // are driven through the throttled drag between frames, as a
            // hand's are (format/ipc.md §`input`); then the drains and
            // the animation tick. An empty queue is the common case and
            // costs nothing; `needs_continuation` keeps the loop
            // iterating until the queue is empty, and `was_continuing`
            // above already holds the redraw this sample owes. Under a
            // virtual clock time has not moved, so only the input-driven
            // drains run: the sample and the tick are `clock.step`'s.
            init.input_context().drain_frame(Beat::Loop);
            // After the input drain, so a command observes every input
            // the user sent before it — throttled drag moves and zoom
            // steps included — and its effects are ordered after them.
            ipc_outcome = drain_ipc(&mut self.ipc, Some(init));
            // After the IPC drain, so a frame shows what the commands of
            // this iteration did, and before the continuation check,
            // which the frame's camera-geometry rebuild can change.
            init.input_context().record_tick(Beat::Loop);
            let still_continuing = init.needs_continuation();
            // Only when nothing else wants this iteration: no animation
            // frame or redraw still owed, no IPC left in the queue. The
            // loop is about to sleep, and the encode spends the sleep.
            if !still_continuing && !was_continuing && !ipc_outcome.repaint && !ipc_outcome.backlog {
                init.record_idle();
            }
            // Request a redraw if work happened this iteration
            // (`was_continuing`) OR if more work is pending
            // (`still_continuing`). The OR catches the last
            // animation frame: pre=true, post=false, still need
            // one redraw to land the final state. An IPC command that
            // changed pixels needs one too, as a mutating input
            // handler's `redraw_after` does.
            if was_continuing || still_continuing || ipc_outcome.repaint {
                init.request_redraw();
            }
            // Active → idle FPS transition: when the loop is about
            // to park and the overlay still shows a live numeric
            // reading, eventually flip it to the idle marker so the
            // overlay reads "FPS: -" while the app sleeps. Defer
            // the flip by `FPS_IDLE_GRACE` so brief between-drain
            // gaps during an active throttled drag don't flicker
            // the overlay between numeric and "-" — the user needs
            // a stable readout to actually use the diagnostic.
            //
            // `fps_idle_defer_deadline` returns `Some(when)` if the
            // last frame is younger than the grace; in that case
            // we hold the loop in `ControlFlow::WaitUntil(when)` so
            // it wakes at the deadline to commit the flip if no
            // events arrived in the meantime. Otherwise the grace
            // has already elapsed (or the FPS is already idle), so
            // we either commit the flip now or do nothing.
            let fps_on = init.renderer.fps_display_mode() != crate::application::common::FpsDisplayMode::Off;
            let fps_defer_deadline = if !still_continuing && fps_on {
                init.renderer.fps_idle_defer_deadline(FPS_IDLE_GRACE)
            } else {
                None
            };
            if !still_continuing && fps_on && fps_defer_deadline.is_none() && init.renderer.has_live_fps() {
                init.renderer.set_fps_idle();
                init.request_redraw();
            }
            // ControlFlow::Poll keeps the loop ticking so the next
            // about_to_wait drains again; ControlFlow::WaitUntil
            // parks until either the FPS-idle grace elapses or an
            // event arrives, whichever comes first; ControlFlow::Wait
            // parks the thread until an OS event arrives. The
            // choice is strictly post-drain — pre-drain state may
            // have been continuing while post-drain state is fully
            // idle.
            //
            // An active recording is a timer source: an idle canvas
            // would otherwise park with no heartbeat to take its next
            // frame (`format/ipc.md` §`capture.record_start`). So is a
            // pending `clock.wait`'s deadline, which is wall-clock under
            // either mode: it guards the client's liveness, and a parked
            // loop must still answer it.
            let wait_deadline = self.ipc.as_ref().and_then(IpcServer::wait_deadline);
            let wake_by = [
                fps_defer_deadline,
                super::record_flow::deadline(&init.recorder, &init.clock),
                wait_deadline,
            ]
            .into_iter()
            .flatten()
            .min();
            event_loop.set_control_flow(if still_continuing {
                ControlFlow::Poll
            } else if let Some(deadline) = wake_by {
                ControlFlow::WaitUntil(deadline)
            } else {
                ControlFlow::Wait
            });
            // After the redraw this iteration owes has been asked for.
            evaluate_wait(&mut self.ipc, init);
            // After every handler of the iteration has run, so a load,
            // a `new` or a save-as reaches the next client's `hello`.
            if let Some(server) = self.ipc.as_mut() {
                server.publish_map(init.document.as_ref().and_then(|doc| doc.file_path.as_deref()));
            }
        } else {
            // Before the window exists there is no input to order
            // against; requests that need no app state still answer.
            ipc_outcome = drain_ipc(&mut self.ipc, None);
        }
        // A drain that ran out of budget keeps the loop iterating until
        // the queue is empty.
        if ipc_outcome.backlog {
            event_loop.set_control_flow(ControlFlow::Poll);
        }
        // Mark parked AFTER setting ControlFlow so the watchdog
        // begins ignoring silence only once we've committed to
        // sleeping. If ControlFlow::Poll, the next about_to_wait
        // will call `unparked` again immediately.
        self.watchdog.parked();
    }

    /// Graceful exit: the client hears `shutdown`, the socket file is
    /// removed, and so is the artifacts dir with every server-chosen
    /// screenshot in it.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(server) = self.ipc.take() {
            server.shutdown();
        }
        if let Some(init) = self.init.as_mut() {
            // A recording in progress is finished first: its frames and
            // manifest are on disk, or — in the artifacts dir — go
            // with the dir just below.
            super::record_flow::shutdown(
                &mut init.recorder,
                &mut init.artifacts,
                &init.watchdog,
                &init.clock,
            );
            init.artifacts.remove();
        }
    }
}

/// All the state that was previously owned by the move-closure
/// body of the native event loop. Constructed in
/// [`NativeApp::resumed`] via
/// [`super::run_native_init::build`] once the window exists;
/// then [`Self::handle_event`] runs the original per-event match
/// body against these fields (via `self.X` for each access).
pub(super) struct InitState {
    pub(super) window: Arc<Window>,
    pub(super) renderer: Renderer,
    pub(super) document: Option<MindMapDocument>,
    pub(super) mindmap_tree: Option<MindMapTree>,
    pub(super) scene_cache: baumhard::mindmap::scene_cache::SceneConnectionCache,
    pub(super) app_scene: crate::application::scene_host::AppScene,
    pub(super) cursor_pos: (f64, f64),
    pub(super) drag_state: DragState,
    pub(super) interaction_mode: InteractionMode,
    pub(super) console_state: ConsoleState,
    pub(super) console_history: Vec<String>,
    pub(super) single_line_edit_state: SingleLineEditor,
    pub(super) text_edit_state: TextEditState,
    pub(super) color_picker_state: crate::application::color_picker::ColorPickerState,
    pub(super) last_click: Option<LastClick>,
    pub(super) hovered_node: Option<String>,
    pub(super) modifiers: ModifiersState,
    pub(super) cursor_is_hand: bool,
    /// Last cursor icon written via `Window::set_cursor`. Used by
    /// the cursor_moved handler to dedup redundant `set_cursor`
    /// calls — winit dedupes these on macOS / X11 but NOT on
    /// Windows (every call → `LoadCursorW` + `SetCursor` + mutex
    /// lock) or Wayland (calls into pointer manager every time),
    /// so the per-event cursor icon update needs an
    /// application-side gate. Initialized to `Default` to match
    /// the as-launched cursor.
    pub(super) cursor_icon_last: CursorIcon,
    /// Throttled, coexistent-with-drag color-picker hover.
    /// Continues to update independently of the active drag
    /// variant (if any), hence a sibling field rather than a
    /// `ThrottledDrag` variant.
    pub(super) picker_hover: super::throttled_interaction::ColorPickerHoverInteraction,
    pub(super) keybinds: ResolvedKeybinds,
    /// User-defined macro registry. Loaded once at startup
    /// (`run_native_init::build`) from `~/.config/mandala/macros.json`;
    /// queried at dispatch time via `keybinds.macro_for(...)`.
    pub(super) macros: crate::application::macros::MacroRegistry,
    /// Touch gesture state machine. Fed by `WindowEvent::Touch`
    /// events (winit); emits a tap, a long-press, a one-finger pan
    /// step or a two-finger pinch step. Cross-platform peer of WASM's
    /// `WasmInputState.touch_recognizer`. See `app/touch_gesture.rs`
    /// for the machine and CONCEPTS §5 for where each emission goes.
    pub(super) touch_recognizer: super::touch_gesture::TouchGestureRecognizer,
    /// App-clock ms at which the current tree-mutating drag began
    /// suppressing the animation tick — `Some(now_ms)` while a
    /// `MovingNode` / `MovingSection` / `SectionResize` /
    /// `NodeResize` drag is in flight, `None` otherwise. On
    /// release we shift each active animation's `start_ms`
    /// forward by the drag duration so the post-release tick
    /// resumes the animation where the drag froze it, instead
    /// of snapping to its `to` state on the first post-release
    /// frame (the `tick_animations` body computes `elapsed = now
    /// - start_ms`, and without the shift the time elapsed during
    /// the drag would land past `total`).
    pub(super) anim_pause_start_ms: Option<u64>,
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
}

impl InitState {
    /// Build the [`InputHandlerContext`] view over this state for a
    /// single dispatcher call. Rebuilt per event because the
    /// returned borrow is tied to `&mut self` — `'_` expires as
    /// soon as the handler returns.
    pub(super) fn input_context(&mut self) -> InputHandlerContext<'_> {
        InputHandlerContext {
            document: &mut self.document,
            mindmap_tree: &mut self.mindmap_tree,
            app_scene: &mut self.app_scene,
            renderer: &mut self.renderer,
            scene_cache: &mut self.scene_cache,
            drag_state: &mut self.drag_state,
            interaction_mode: &mut self.interaction_mode,
            console_state: &mut self.console_state,
            console_history: &mut self.console_history,
            single_line_edit_state: &mut self.single_line_edit_state,
            text_edit_state: &mut self.text_edit_state,
            color_picker_state: &mut self.color_picker_state,
            last_click: &mut self.last_click,
            hovered_node: &mut self.hovered_node,
            cursor_pos: &mut self.cursor_pos,
            modifiers: &mut self.modifiers,
            cursor_is_hand: &mut self.cursor_is_hand,
            cursor_icon_last: &mut self.cursor_icon_last,
            picker_hover: &mut self.picker_hover,
            keybinds: &self.keybinds,
            macros: &mut self.macros,
            artifacts: &mut self.artifacts,
            recorder: &mut self.recorder,
            watchdog: &self.watchdog,
            window: &self.window,
            touch_recognizer: &mut self.touch_recognizer,
            anim_pause_start_ms: &mut self.anim_pause_start_ms,
            injected: &mut self.injected,
            clock: &mut self.clock,
        }
    }

    /// Ask winit for a frame, and write the ask into the
    /// [`RedrawLedger`]. Every redraw the app requests goes through
    /// here, so the ledger's "drawn" means "everything asked for so far
    /// has been drawn".
    pub(super) fn request_redraw(&mut self) {
        self.redraws.ask();
        self.window.request_redraw();
    }

    /// What `clock.wait` reads at an evaluation point
    /// (`format/ipc.md` §`clock.wait`).
    pub(super) fn settling(&self) -> Settling {
        Settling {
            animations_active: self
                .document
                .as_ref()
                .is_some_and(MindMapDocument::has_active_animations),
            continuing: self.needs_continuation(),
            redraw_owed: !self.redraws.is_current(),
            virtual_clock: self.clock.is_virtual(),
        }
    }

    /// Per-event dispatch. Most of the per-event work lives in
    /// [`super::event_mouse_click`], [`super::event_cursor_moved`],
    /// and [`super::event_keyboard`]; this method handles the
    /// smaller arms (resize, close, wheel, modifiers) inline and
    /// delegates the larger ones.
    ///
    /// Mutating arms set `redraw_after = true` so the trailing
    /// [`winit::window::Window::request_redraw`] call queues one
    /// `WindowEvent::RedrawRequested` for the next event-loop
    /// iteration. The `RedrawRequested` arm itself is the only
    /// place [`crate::application::renderer::Renderer::process`]
    /// runs — separating "decide to redraw" from "actually
    /// render" lets winit coalesce multiple `request_redraw`
    /// calls in one event chain into a single render.
    pub(super) fn handle_event(&mut self, event_loop: &ActiveEventLoop, event: winit::event::Event<()>) {
        let mut redraw_after = false;
        match event {
            //// WINDOW SPECIFIC ////
            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                self.renderer
                    .process_decree(RenderDecree::SetSurfaceSize(size.width, size.height));
                // Glyph-wheel color picker caches its layout in
                // OpenPicker::layout; the
                // cached values include the screen-space backdrop
                // and per-glyph positions, so a resize would
                // leave hit-tests aimed at the old geometry and
                // the renderer's overlay buffers anchored at the
                // pre-resize coordinates.
                if self.color_picker_state.is_open() {
                    rebuild_color_picker_overlay(
                        &mut self.color_picker_state,
                        &mut self.app_scene,
                        &mut self.renderer,
                    );
                }
                redraw_after = true;
            }
            Event::WindowEvent {
                event: WindowEvent::RedrawRequested,
                ..
            } => {
                // Sole entry to the render path. Redraws are queued
                // via `Window::request_redraw` from event handlers
                // (mutations) and from `about_to_wait` (drain
                // continuations); winit delivers exactly one
                // `RedrawRequested` per coalesced batch. The ledger
                // learns whether this frame drew what was asked for,
                // which is what `clock.wait until=settled` reads.
                let asked = self.redraws.asked();
                match self.renderer.process() {
                    Ok(()) => self.redraws.drew(asked),
                    // Nothing else would ask again with no input to
                    // follow, leaving the window — and a `settled` wait
                    // — on the last frame drawn.
                    Err(skip) if skip.is_transient() => self.request_redraw(),
                    // Retrying would spin; the redraw stays owed.
                    Err(_) => {}
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                self.renderer.process_decree(RenderDecree::Terminate);
                event_loop.exit();
            }
            //// MOUSE ////
            Event::WindowEvent {
                event: WindowEvent::MouseInput { state, button, .. },
                ..
            } => {
                redraw_after = self.input_context().on_mouse_input(state, button);
            }
            Event::WindowEvent {
                event: WindowEvent::MouseWheel { delta, .. },
                ..
            } => {
                redraw_after = self.input_context().on_mouse_wheel(delta);
            }
            Event::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. },
                ..
            } => {
                redraw_after = self.input_context().on_cursor_moved(position);
            }
            //// TOUCH ////
            Event::WindowEvent {
                event: WindowEvent::Touch(touch),
                ..
            } => {
                redraw_after =
                    self.input_context()
                        .dispatch_touch_event(touch.phase, touch.id, touch.location);
            }
            //// KEYBOARD ////
            Event::WindowEvent {
                event: WindowEvent::ModifiersChanged(mods),
                ..
            } => {
                self.modifiers = mods.state();
                // Modifier-only changes don't paint anything until
                // a paired mouse/keyboard event acts on them.
            }
            Event::WindowEvent {
                event:
                    WindowEvent::KeyboardInput {
                        event:
                            KeyEvent {
                                logical_key, state, ..
                            },
                        ..
                    },
                ..
            } => {
                redraw_after = self.input_context().on_keyboard_input(logical_key, state);
            }
            _ => {}
        }
        if redraw_after {
            self.request_redraw();
        }
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
        heartbeat::needs_continuation(
            &self.drag_state,
            &self.picker_hover,
            self.document.as_ref(),
            self.renderer.connection_geometry_dirty(),
            &self.injected,
            &self.clock,
        )
    }
}

impl InputHandlerContext<'_> {
    /// Translate a winit `Touch` event's phase, finger id and
    /// location into a recognizer step and run it. Takes the payload
    /// rather than the `Touch` struct so the IPC `input.touch`
    /// command reaches this same entry point (its `DeviceId` cannot
    /// be built outside winit). Returns true when the event drove any state
    /// transition or ran a gesture (the caller should request a
    /// redraw on true). Modifier state is fixed at all-false —
    /// touch devices have no modifier keys, and the keybind table's
    /// `LongPress` binding doesn't carry Ctrl/Shift/Alt either.
    ///
    /// **Long-press timing wake-up gap**: `tick` is called only
    /// from the events themselves (not on a wall-clock timer),
    /// so a finger held with literally zero `Moved` events
    /// between Started and Ended would miss the long-press
    /// emission. In practice touch hardware emits sub-pixel
    /// jitter `Moved` events constantly while a finger is down,
    /// so the gap is theoretical — and the recognizer refuses to
    /// call such a hold a tap either way, on the elapsed time
    /// rather than on the emission having happened. A future
    /// improvement would set `ControlFlow::WaitUntil(started_at +
    /// LONG_PRESS_MS)` from the recognizer's `OneFinger` state.
    ///
    /// Timed on the app clock (`AppClock::instant`), so under a
    /// virtual clock a long-press is virtual milliseconds held.
    pub(super) fn dispatch_touch_event(
        &mut self,
        phase: TouchPhase,
        id: u64,
        location: PhysicalPosition<f64>,
    ) -> bool {
        use super::touch_gesture::Phase;
        // Phase translation, recognizer ingest + tick, and the
        // routing of what it recognized are `cross_dispatch::pointer`'s
        // `drive_touch_event`; the browser runs the same body, and so
        // does `apply_touch_effect` below. What is native's own is the
        // `Dispatch` arm — it goes through `dispatch_action` so a
        // `NativeOnly` Action bound to a gesture (`EnterResizeMode`)
        // reaches its arm.
        let phase = super::dispatch::touch_phase(phase);
        let pos = (location.x, location.y);
        // No gesture recognized — but the recognizer may have moved
        // its internal state (e.g. Started → OneFinger). The caller
        // still wants a redraw on Started/Moved so any
        // cursor-following overlay updates.
        let idle_redraw = matches!(phase, Phase::Started | Phase::Moved);
        let Some(step) = super::dispatch::drive_touch_event(
            &mut *self.touch_recognizer,
            self.keybinds,
            phase,
            id,
            pos,
            // The app clock's instant: a long-press is a duration on
            // the same axis as every other behavior's timing.
            self.clock.instant(),
        ) else {
            return idle_redraw;
        };
        match step {
            super::dispatch::TouchStep::Dispatch(d) => {
                *self.cursor_pos = d.cursor_pos;
                let Some(a) = d.action else {
                    return idle_redraw;
                };
                let _ = super::dispatch::dispatch_action(a, self, None);
                true
            }
            super::dispatch::TouchStep::Effect(effect) => {
                let (mut core, _native) = self.split_borrow();
                super::dispatch::apply_touch_effect(effect, &mut core);
                // The camera-geometry reprojection a pinch owes is
                // picked up by `drain_camera_geometry_rebuild` in the
                // frame this redraw requests, under the same
                // renderer-side dirty flag the wheel path uses.
                true
            }
        }
    }

    /// `WindowEvent::MouseInput`. Returns whether to request a redraw.
    ///
    /// This, the three `on_*` methods below and
    /// [`Self::dispatch_touch_event`] are `handle_event`'s input arms,
    /// lifted out so the IPC `input` family reaches the handlers through
    /// the same bodies a winit event does (`format/ipc.md` §`input`):
    /// from here down, nothing can tell the two apart. They live on the
    /// context rather than on `InitState` so the heartbeat
    /// (`app::heartbeat`) reaches them from wherever it runs — the event
    /// loop, or a `clock step` inside a console line.
    pub(super) fn on_mouse_input(&mut self, state: ElementState, button: MouseButton) -> bool {
        event_mouse_click::handle_mouse_input(state, button, self);
        true
    }

    /// `WindowEvent::MouseWheel`. Returns whether to request a redraw.
    pub(super) fn on_mouse_wheel(&mut self, delta: MouseScrollDelta) -> bool {
        let scroll_y = crate::application::app::wheel_lines(delta);
        // While the console is open, the wheel scrolls the
        // scrollback rather than zooming the canvas — mouse
        // events should follow keyboard focus. Fractional
        // deltas accumulate via `accumulate_wheel_lines` so
        // sub-line-per-tick scrolls don't round to zero.
        if self.console_state.is_open() {
            let lines = if let crate::application::console::ConsoleState::Open { wheel_accum, .. } =
                &mut *self.console_state
            {
                crate::application::app::console_input::accumulate_wheel_lines(wheel_accum, scroll_y as f32)
            } else {
                0
            };
            if lines == 0 {
                return false;
            }
            crate::application::app::console_input::scroll_console_by_lines(self.console_state, lines);
            if let Some(doc) = self.document.as_ref() {
                crate::application::app::console_input::rebuild_console_overlay(
                    self.console_state,
                    doc,
                    self.app_scene,
                    self.renderer,
                    self.keybinds,
                );
            }
            return true;
        }
        // Wheel zoom is routed through `dispatch_action` so
        // users can rebind `WheelUp` / `WheelDown` to any
        // Action (or unbind them entirely). Defaults bind
        // both to `ZoomIn` / `ZoomOut`. If the user
        // explicitly clears the bindings, wheel events are
        // silently ignored.
        let gesture_name = crate::application::app::wheel_gesture(scroll_y).key_name();
        // `action_for_gesture` falls back to the unmodified
        // binding when no exact-modifier match exists, so
        // `Ctrl+Wheel` keeps zooming even though only
        // `WheelUp` / `WheelDown` are bound in defaults —
        // pre-branch behavior was modifier-agnostic and
        // we preserve it here without forcing every user
        // to enumerate modifier permutations.
        let action = self.keybinds.action_for_gesture(
            gesture_name,
            self.modifiers.control_key(),
            self.modifiers.shift_key(),
            self.modifiers.alt_key(),
        );
        let Some(a) = action else {
            return false;
        };
        let _ = crate::application::app::dispatch::dispatch_action(a, self, None);
        true
    }

    /// `WindowEvent::CursorMoved`. Returns whether to request a redraw.
    pub(super) fn on_cursor_moved(&mut self, position: PhysicalPosition<f64>) -> bool {
        let window = self.window;
        event_cursor_moved::handle_cursor_moved(position, window, self);
        // Hover that doesn't drag and doesn't open the
        // picker doesn't change visuals — no redraw needed.
        // Any active drag (Pending → Throttled etc.) or an
        // open picker requires a fresh frame so the drag
        // preview / hover marker tracks the cursor.
        !matches!(*self.drag_state, DragState::None) || self.color_picker_state.is_open()
    }

    /// `WindowEvent::KeyboardInput`. Only a press reaches the
    /// handler; a release changes nothing. Returns whether to request
    /// a redraw.
    pub(super) fn on_keyboard_input(&mut self, logical_key: Key, state: ElementState) -> bool {
        if state != ElementState::Pressed {
            return false;
        }
        event_keyboard::handle_keyboard_input(logical_key, self);
        true
    }
}
