// SPDX-License-Identifier: MPL-2.0

//! The windowed driver for [`super::Application::run`]: winit 0.30's
//! `ApplicationHandler`, which owns the event loop and the window and
//! runs the one loop body both native drivers share (`app::pump`) in
//! `about_to_wait`. First-time init lives in
//! [`super::run_native_init::build`]; the driver with no window at all
//! is `app::run_headless`.

#![cfg(not(target_arch = "wasm32"))]

use std::process::ExitCode;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{Event, KeyEvent, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use super::color_picker_flow::rebuild_color_picker_overlay;
use super::freeze_watchdog::FreezeWatchdog;
use super::pump::{self, Next, Running};
use super::run_native_init::{self, InitState};
use super::shell::Shell;
use super::{Application, OneShot, Options};
use crate::application::common::RenderDecree;
use crate::application::ipc::{IpcServer, IpcWake};

/// Entry point called from `Application::run` on every non-WASM
/// target that has a window. Hands control to winit's event loop;
/// returns when the window is closed, or when a one-shot launch has its
/// capture, with the code the process should end with.
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
pub(super) fn run(app: Application) -> ExitCode {
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
        one_shot: options.one_shot.clone(),
        options,
        init: None,
        watchdog: FreezeWatchdog::spawn(),
        ipc,
        exit_code: ExitCode::SUCCESS,
    };
    event_loop
        .run_app(&mut handler)
        .expect("Some kind of unexpected error appears to have taken place");
    handler.exit_code
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
    /// The `--screenshot` capture still owed, taken when it fires. A
    /// copy of `options.one_shot`, and not the same fact: that one is
    /// what the launch asked for, which `run_native_init::build` reads
    /// when the window exists (a one-shot launch starts on a virtual
    /// clock) and which stays true after the capture; this one is what
    /// the loop still owes.
    one_shot: Option<OneShot>,
    /// What [`run`] returns: success, unless a one-shot capture failed.
    exit_code: ExitCode,
}

impl ApplicationHandler<IpcWake> for NativeApp {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _: StartCause) {
        // ControlFlow is set authoritatively at the end of every
        // `about_to_wait` from what the iteration says comes next.
        // No-op here so we don't override that decision on each
        // iteration.
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.init.is_some() {
            return;
        }
        let mut attributes = Window::default_attributes();
        if let Some((width, height)) = self.options.surface_size {
            attributes = attributes.with_inner_size(PhysicalSize::new(width, height));
        }
        let window = event_loop
            .create_window(attributes)
            .expect("Failed to create application window");
        self.init = Some(run_native_init::build(
            &self.options,
            Shell::Window(Arc::new(window)),
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
            pump::evaluate_wait(&mut self.ipc, init);
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

    /// One iteration of the loop body both drivers share
    /// ([`pump::iterate`]), and the `ControlFlow` its answer names.
    /// Before the window exists there is no input to order against:
    /// requests that need no app state still answer.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.watchdog.unparked();
        let next = match self.init.as_mut() {
            Some(init) => pump::iterate(&mut Running {
                init,
                ipc: &mut self.ipc,
                one_shot: &mut self.one_shot,
            }),
            None if pump::drain_ipc(&mut self.ipc, None).backlog => Next::Now,
            None => Next::Idle,
        };
        // ControlFlow::Poll keeps the loop ticking so the next
        // about_to_wait drains again; ControlFlow::WaitUntil parks until
        // either the deadline or an event arrives, whichever comes
        // first; ControlFlow::Wait parks the thread until an OS event
        // arrives. The choice is strictly post-drain — pre-drain state
        // may have been continuing while post-drain state is fully idle.
        match next {
            Next::Now => event_loop.set_control_flow(ControlFlow::Poll),
            Next::Until(deadline) => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
            Next::Idle => event_loop.set_control_flow(ControlFlow::Wait),
            Next::Exit(code) => {
                self.exit_code = code;
                event_loop.exit();
            }
        }
        // Mark parked AFTER setting ControlFlow so the watchdog
        // begins ignoring silence only once we've committed to
        // sleeping. If ControlFlow::Poll, the next about_to_wait
        // will call `unparked` again immediately.
        self.watchdog.parked();
    }

    /// Graceful exit ([`pump::shut_down`]).
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        pump::shut_down(&mut self.ipc, self.init.as_mut());
    }
}

impl InitState {
    /// Per-event dispatch. Most of the per-event work lives in
    /// [`super::event_mouse_click`], [`super::event_cursor_moved`],
    /// and [`super::event_keyboard`]; this method handles the
    /// smaller arms (resize, close, wheel, modifiers) inline and
    /// delegates the larger ones.
    ///
    /// Mutating arms set `redraw_after = true` so the trailing
    /// [`InitState::request_redraw`] call queues one
    /// `WindowEvent::RedrawRequested` for the next event-loop
    /// iteration. The `RedrawRequested` arm itself is the only
    /// place [`InitState::draw_frame`] runs under a window —
    /// separating "decide to redraw" from "actually render" lets
    /// winit coalesce multiple `request_redraw` calls in one event
    /// chain into a single render.
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
                // The sole entry to the render path under a window.
                // Redraws are queued via `Window::request_redraw` from
                // event handlers (mutations) and from `about_to_wait`
                // (drain continuations); winit delivers exactly one
                // `RedrawRequested` per coalesced batch.
                self.draw_frame();
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
}
