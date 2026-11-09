// SPDX-License-Identifier: MPL-2.0

//! WASM event-loop body for [`super::Application::run`]. Builds a
//! [`WasmApp`] [`ApplicationHandler`] and hands it to winit-web's
//! [`EventLoopExtWebSys::spawn_app`]; the browser's main thread
//! drives the loop from there. Synchronous DOM setup (canvas
//! attach, `tabindex` focus, keydown preventDefault listener)
//! finishes inside [`run`] before `spawn_app` is called; the
//! async path (`Renderer::new`, document fetch, rAF render loop
//! install) is *spawned* before `spawn_app` via
//! `wasm_bindgen_futures::spawn_local` but resumes on later
//! microtask ticks, so [`WasmApp`]'s `renderer` / `input` cells
//! start `None` and the handler's match arms guard accordingly.
//!
//! The [`ApplicationHandler::window_event`] callback funnels into
//! [`WasmApp::handle_window_event`], which owns the dispatch
//! match: the arms fan out to per-arm `handle_*` inherent methods
//! defined in sibling `event_*.rs` files (RedrawRequested — the
//! browser's frame — Resized, ModifiersChanged, KeyboardInput,
//! CursorMoved, MouseInput, MouseWheel, Touch) plus an inline
//! `CloseRequested` no-op and a catch-all that drops the arms WASM
//! doesn't currently consume. Mirrors native's per-arm split under
//! `app/event_*.rs` so cross-target convergence reviews can diff
//! the two layouts side by side.

#![cfg(target_arch = "wasm32")]

mod event_cursor_moved;
mod event_keyboard;
mod event_modifiers;
mod event_mouse_click;
mod event_mouse_wheel;
mod event_redraw_requested;
mod event_resized;
mod event_touch;
mod ipc;
mod ipc_target;
mod step;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{KeyEvent, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::platform::web::EventLoopExtWebSys;
use winit::window::WindowId;

use super::startup_load;
use super::{Application, ClickHit, LastClick};
use crate::application::common::RenderDecree;
use crate::application::document::MindMapDocument;
use crate::application::ipc::events::Observed;
use crate::application::ipc::injected::{InjectedQueue, InputEvent, Sample, Surface};
use crate::application::keybinds::{Action, ResolvedKeybinds};
use crate::application::platform::window::PhysicalPosition;
use crate::application::renderer::Renderer;
use crate::application::view::MapView;

/// Shared state between the WASM rAF render loop and the winit
/// event loop. Mirrors native's `InputHandlerContext` for the fields
/// both targets share. Module-level so the `cross_dispatch` /
/// `macro_core` helpers can take `&mut WasmInputState` directly —
/// closure-local types aren't reachable from sibling modules.
pub(super) struct WasmInputState {
    pub document: MindMapDocument,
    /// The document's view: the projector, the glyph layers and the
    /// overlay over them (`view::MapView`), the peer of native's
    /// `InitState.view`. Loaded with the document, synced by the
    /// frame (`event_redraw_requested`) and by every body that changes
    /// the document or the selection.
    pub view: Option<MapView>,
    pub last_click: Option<LastClick>,
    pub cursor_pos: (f64, f64),
    /// What the left press hit, held until its release clicks it:
    /// `None` on init, after a release consumed it, and while no press
    /// is down. The browser's whole drag-state machine — it has no
    /// drag to promote a press into.
    pub pending_click: Option<ClickHit>,
    pub modifiers: crate::application::platform::input::Modifiers,
    /// High-level interaction mode. Cross-platform field; the browser
    /// initializes it to `Default`.
    pub interaction_mode: super::InteractionMode,
    /// Macro registry. Mirrors native's `InputHandlerContext::macros`
    /// — the App and User tiers, loaded at startup. Consulted by the
    /// keyboard handler's Action → Macro → edit fall-through.
    pub macros: crate::application::macros::MacroRegistry,
    /// Touch gesture recognizer. Cross-platform peer of native's
    /// `InitState.touch_recognizer`. WASM mobile is the *primary*
    /// surface this targets (mobile-browser tap-and-hold is
    /// unreachable through `MouseInput`/`CursorMoved` events
    /// alone — winit dispatches them as
    /// `WindowEvent::Touch` instead).
    pub touch_recognizer: super::touch_gesture::TouchGestureRecognizer,
    /// The app clock (`crate::application::clock`), the peer of native's
    /// `InitState.clock`. Real until a `clock` command switches it,
    /// which only IPC carries here (`CLAUDE.md` "Virtual-clock
    /// stepping"). The browser's frame (`event_redraw_requested`) reads
    /// it the way native's loop frame does.
    pub clock: crate::application::clock::AppClock,
    /// The IPC injected-input queue (`ipc::injected`), the peer of
    /// native's `InitState.injected`: a `Pacing::PerFrame` command's
    /// samples wait here and the frame lands one per real-time frame
    /// (`event_redraw_requested`), or a step lands one per heartbeat
    /// (`step`), where a winit event lands. Empty unless an IPC
    /// controller is attached.
    pub injected: InjectedQueue,
    /// A redraw has been asked for that no frame has drawn yet: the
    /// browser's spelling of native's `RedrawLedger::is_current`, and
    /// the "rendered" half of `clock.wait until=settled`. Set wherever
    /// a redraw is asked for ([`WasmApp::ask_redraw`], the frame's own
    /// re-ask), cleared by a frame whose render ran.
    pub redraw_owed: bool,
    /// The canvas the renderer draws into: what `capture.screenshot`
    /// reads back on this target (`to_data_url`, in the same task as
    /// the draw).
    pub canvas: web_sys::HtmlCanvasElement,
}

/// The browser's event-handler context: the input state and the
/// renderer, borrowed out of [`WasmApp`]'s cells for one event, with
/// the keybind table beside them. The
/// peer of native's `InputHandlerContext`, and for the same reason:
/// every handler body is a method on it, so the three callers that
/// enter a body — a winit event (`WasmApp::handle_window_event`), an
/// injected IPC sample ([`Self::apply_injected`]) and an IPC command
/// (`ipc_target`) — reach the one body rather than three copies of it.
pub(super) struct Borrowed<'a> {
    pub(super) input: &'a mut WasmInputState,
    pub(super) renderer: &'a mut Renderer,
    pub(super) keybinds: &'a ResolvedKeybinds,
}

impl Borrowed<'_> {
    /// Deliver one injected-input sample to the `on_*` entry point its
    /// winit event reaches, with the sample's modifiers held for that
    /// one event (`ipc::injected::Sample`). Returns whether the arm
    /// asked for a redraw. The body native's `apply_injected` runs,
    /// over this target's entry points.
    pub(super) fn apply_injected(&mut self, sample: Sample<Surface>) -> bool {
        let reported = self.input.modifiers;
        if let Some(held) = sample.modifiers {
            self.input.modifiers = held;
        }
        let redraw = match sample.event {
            InputEvent::CursorMoved((x, y)) => self.on_cursor_moved(PhysicalPosition::new(x, y)),
            InputEvent::MouseButton { button, state } => self.on_mouse_input(state, button),
            InputEvent::Wheel(delta) => self.on_mouse_wheel(delta),
            InputEvent::Key { key, state } => self.on_keyboard_input(key, state),
            InputEvent::Touch {
                phase,
                pointer_id,
                at: (x, y),
            } => self.dispatch_touch_event(phase, pointer_id, PhysicalPosition::new(x, y)),
        };
        self.input.modifiers = reported;
        redraw
    }

    /// The app as the IPC event stream reads it ([`Observed`]): the
    /// document, the change records the view synced since the stream
    /// last looked, the camera and the interaction mode.
    pub(super) fn observed(&self) -> Observed<'_> {
        self.input.observed(self.renderer)
    }

    /// Bring the view and the renderer up to date with the document and
    /// its selection — the one pipeline (`MapView::sync`), which native's
    /// frame runs in `heartbeat::drain_inputs`. Returns whether it
    /// changed anything a frame draws.
    pub(super) fn sync_view(&mut self) -> bool {
        match self.input.view.as_mut() {
            Some(view) => view.sync(&mut self.input.document, self.renderer),
            None => false,
        }
    }
}

impl WasmInputState {
    /// Build the cross-platform `InputContextCore` view for
    /// `dispatch_action_core::dispatch_compatible`. `renderer` isn't a
    /// field on `WasmInputState` (held in the closure scope of `run`);
    /// the caller passes it in. Mirrors native's
    /// `InputHandlerContext::split_borrow` shape — short `'s` lifetime
    /// tied to `&mut self`, dropped before the next re-borrow of `self`.
    pub(super) fn input_context_core<'s>(
        &'s mut self,
        renderer: &'s mut Renderer,
    ) -> super::input_context_core::InputContextCore<'s> {
        super::input_context_core::InputContextCore {
            document: Some(&mut self.document),
            view: &mut self.view,
            renderer,
            last_click: &mut self.last_click,
            cursor_pos: &mut self.cursor_pos,
            modifiers: &self.modifiers,
            macros: &mut self.macros,
            interaction_mode: &mut self.interaction_mode,
        }
    }

    /// The app as the IPC event stream reads it, through `renderer`'s
    /// camera.
    fn observed<'s>(&'s self, renderer: &'s Renderer) -> Observed<'s> {
        Observed {
            document: Some(&self.document),
            changes: self.view.as_ref().map_or(&[], |view| view.pending_feed()),
            camera: renderer.camera(),
            mode: &self.interaction_mode,
        }
    }
}

/// WASM impl of `MacroDispatchTarget`. Wraps
/// `&mut WasmInputState` + `&mut Renderer` and forwards each
/// operation to the same helpers the keyboard handler uses.
/// Privilege gating happens in
/// `dispatch_macro_core::dispatch_macro` (single-source contract).
pub(super) struct WasmMacroDispatchTarget<'a> {
    pub(super) input: &'a mut WasmInputState,
    pub(super) renderer: &'a mut Renderer,
    /// The observer the IPC `verbose` event level hangs between
    /// steps (`MacroDispatchTarget::step_finished`); `None` from the
    /// keyboard, which observes nothing.
    pub(super) after_step: Option<&'a mut dyn FnMut(Observed<'_>)>,
}

impl<'a> super::dispatch::macro_core::MacroDispatchTarget for WasmMacroDispatchTarget<'a> {
    fn registry(&self) -> &crate::application::macros::MacroRegistry {
        &self.input.macros
    }

    fn dispatch_action(&mut self, action: Action) -> super::dispatch::DispatchOutcome {
        // The unified cross-platform dispatcher, the body native's
        // `dispatch::dispatch_action` delegates to as well. A macro
        // step carries no `DispatchHit`: it is a keystroke-level
        // instruction with no pointer event behind it.
        let mut core = self.input.input_context_core(self.renderer);
        super::dispatch::action_core::dispatch_compatible(&action, &mut core, None)
    }

    fn execute_console_line(&mut self, line: &str) -> bool {
        // WASM has no `execute_console_line` runtime
        // (`console_input` is `cfg(not(target_arch = "wasm32"))`).
        // The privilege gate already rejects ConsoleLine from
        // non-User tiers above this; User-tier ConsoleLine on WASM
        // logs warn + skips per `format/macros.md` §
        // ConsoleLine on WASM. The macro continues to the next
        // step — fail-soft, matching the User-tier "step failed"
        // posture native uses.
        //
        // Returns `false` so the macro's `any_ran` flag doesn't
        // bump on the no-op path — a `[ConsoleLine]`-only macro
        // on WASM correctly reports "didn't fire" so the keystroke
        // isn't artificially consumed.
        log::warn!(
            "macros: ConsoleLine step '{}' has no console runtime on WASM; skipping",
            baumhard::util::log::console_line_excerpt(line),
        );
        false
    }

    fn step_finished(&mut self) {
        if let Some(after_step) = self.after_step.as_mut() {
            after_step(self.input.observed(self.renderer));
        }
    }
}

/// winit 0.30 [`ApplicationHandler`] for the WASM target. Mirrors
/// the native `super::run_native`'s `NativeApp` shape so each
/// target's event-loop entry point reads the same way. Plain
/// code-span: that module is native-gated, so an intra-doc link to
/// it cannot resolve in this target's doc build.
///
/// Initial DOM setup (canvas attach, `tabindex` focus, keydown
/// preventDefault listener, async renderer construction, document
/// fetch, rAF render loop install) is kicked off inside [`run`]
/// before [`EventLoopExtWebSys::spawn_app`] hands control to winit;
/// the handler then receives [`WindowEvent`]s on the browser's main
/// thread until the tab is torn down. `spawn_app` requires the
/// handler be `'static`, which is why every shared field is
/// `Rc`/`Arc`-owned rather than borrowed.
///
/// The `winit::window::Window` itself is held by the [`Renderer`]
/// (`Arc<Window>` field, populated inside `Renderer::new` from a
/// clone made before this handler is constructed); the handler does
/// not need its own clone for keep-alive purposes and never reads
/// the window directly from event-handler context.
struct WasmApp {
    /// Shared with the rAF render loop set up in [`run`]. `None`
    /// until the async `Renderer::new` future inside `spawn_local`
    /// resolves.
    ///
    /// **Re-borrow contract.** Any code path that's already
    /// holding `self.renderer.borrow*()` (per-arm methods do this
    /// at the top of their body) must NOT call back into a
    /// dispatch path that re-clones this `Rc` and re-borrows it —
    /// `RefCell` defers the conflict to runtime panic. Today no
    /// such re-entry exists (cross_dispatch arms see only the
    /// `ViewContext` projection, not the outer `Rc`); future
    /// arms wiring `WasmInputState`-internal Rc handles must
    /// keep this contract.
    renderer: Rc<RefCell<Option<Renderer>>>,
    /// Shared with the rAF render loop. `None` until the document
    /// fetch and its view's load inside `spawn_local` complete.
    ///
    /// Same re-borrow contract as [`Self::renderer`] applies.
    input: Rc<RefCell<Option<WasmInputState>>>,
    /// Resolved keybind table — built once in [`run`] from
    /// `Options::keybind_config`, read on every key event. `Rc`
    /// because the IPC transport's message callback
    /// (`run_wasm::ipc`) enters the same handler bodies from its own
    /// task and needs the table there too.
    keybinds: Rc<ResolvedKeybinds>,
    /// The IPC transport's host: the same cells, pumped from the
    /// socket's callbacks and from the end of every frame
    /// (`run_wasm::ipc`). Off unless the page was opened with `?ipc=`.
    ipc: ipc::IpcHost,
}

impl ApplicationHandler for WasmApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // Canvas + window were created and DOM-attached before
        // `spawn_app` ran (browser DOM ordering is driven by JS
        // events, not winit's resume cycle), so first-fire is a
        // no-op. winit-web also re-fires `resumed` after a
        // bfcache restore; the empty body is idempotent so no
        // guard is needed (unlike `NativeApp::resumed`'s
        // `is_some()` check, which protects window re-creation).
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        self.handle_window_event(event);
    }
}

impl WasmApp {
    /// Per-event dispatch — the arms fan out into per-arm methods on
    /// [`Borrowed`], defined in sibling `event_*.rs` files: the
    /// `RedrawRequested` arm is the browser's frame (an injected sample,
    /// the view's sync, then the renderer's `process()`, the sole entry
    /// to render on web), the rest are input. Plus an inline
    /// `CloseRequested` no-op (browser tabs don't really close) and a
    /// `_ => {}` catch-all that silently drops the arms winit fires
    /// that WASM doesn't currently consume (IME composition, focus
    /// changes). Each fan-out arm extracts only the fields that arm
    /// needs, and hands the rest to the `on_*` entry point an injected
    /// IPC sample of the same event reaches ([`Borrowed::apply_injected`]),
    /// so nothing below that point can tell the two apart.
    ///
    /// An arm says whether it changed pixels; the trailing
    /// [`Self::ask_redraw`] hands one redraw request to winit-web,
    /// which translates to an internal rAF dispatch. The browser only
    /// paints when the canvas actually changed — and, while injected
    /// input is queued, on every frame, because the frame itself asks
    /// for the next one.
    fn handle_window_event(&mut self, event: WindowEvent) {
        let redraw_after = match event {
            WindowEvent::RedrawRequested => {
                self.handle_redraw_requested();
                false
            }
            WindowEvent::Resized(s) => {
                self.handle_resized(s);
                true
            }
            WindowEvent::CloseRequested => {
                // WASM doesn't really close
                false
            }
            WindowEvent::ModifiersChanged(m) => {
                self.handle_modifiers_changed(m);
                // Modifier-only changes don't paint anything until
                // a paired mouse/keyboard event acts on them.
                false
            }
            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    state, logical_key, ..
                },
                ..
            } => self
                .with_borrowed(|b| b.on_keyboard_input(logical_key, state))
                .unwrap_or(false),
            WindowEvent::CursorMoved { position, .. } => self
                .with_borrowed(|b| b.on_cursor_moved(position))
                .unwrap_or(false),
            WindowEvent::MouseInput { state, button, .. } => self
                .with_borrowed(|b| b.on_mouse_input(state, button))
                .unwrap_or(false),
            WindowEvent::MouseWheel { delta, .. } => {
                self.with_borrowed(|b| b.on_mouse_wheel(delta)).unwrap_or(false)
            }
            WindowEvent::Touch(touch) => self
                .with_borrowed(|b| b.dispatch_touch_event(touch.phase, touch.id, touch.location))
                .unwrap_or(false),
            // Catch-all for winit `WindowEvent` variants WASM
            // doesn't yet route. The notable un-wired ones:
            //
            // - `WindowEvent::Ime` — IME composition strings.
            //   Required for non-Latin text editing inside the
            //   inline node-text editor. Modal-handler-side
            //   work; the dispatch funnel doesn't see literal
            //   IME payloads (§3 carve-out for `winit::Key`).
            // - `WindowEvent::Focused`, `CursorEntered` /
            //   `CursorLeft` — informational; could drive a
            //   "canvas inactive" overlay in the future.
            //
            // Drop silently rather than log because winit fires
            // every variant on every event tick; a log would
            // burn 60Hz × variants. When any of the above gets
            // wired, lift the corresponding match arm out of
            // this catch-all into a dedicated handle_* method.
            _ => false,
        };
        if redraw_after {
            self.ask_redraw();
        }
    }

    /// Run `f` over the event-handler context, or nothing before init
    /// completes: the two cells are filled in one task, the renderer
    /// first, so an input state implies a renderer.
    ///
    /// **Re-borrow contract.** `f` holds both cells' borrows for its
    /// whole run, so it must not reach anything that re-borrows them —
    /// the IPC transport's callback (`run_wasm::ipc`) runs in its own
    /// task and never from inside a handler, which is what keeps the
    /// contract.
    pub(super) fn with_borrowed<R>(&self, f: impl FnOnce(&mut Borrowed<'_>) -> R) -> Option<R> {
        let mut input_borrow = self.input.borrow_mut();
        let mut renderer_borrow = self.renderer.borrow_mut();
        let (Some(input), Some(renderer)) = (input_borrow.as_mut(), renderer_borrow.as_mut()) else {
            return None;
        };
        let mut borrowed = Borrowed {
            input,
            renderer,
            keybinds: &self.keybinds,
        };
        Some(f(&mut borrowed))
    }

    /// After a frame: the transport's pump, which evaluates a pending
    /// `clock.wait` against the app as the frame left it and delivers
    /// what the frame's handlers changed. A no-op without `?ipc=`.
    pub(super) fn ipc_after_frame(&self) {
        self.ipc.pump();
    }

    /// Ask winit-web for one frame, and record that it is owed
    /// (`WasmInputState::redraw_owed`): the one place a redraw is asked
    /// for outside the frame itself, so `clock.wait until=settled`
    /// sees every ask.
    pub(super) fn ask_redraw(&self) {
        if let Some(input) = self.input.borrow_mut().as_mut() {
            input.redraw_owed = true;
        }
        if let Some(r) = self.renderer.borrow().as_ref() {
            r.request_redraw();
        }
    }
}

/// Run the browser event loop against `app`.
pub(super) fn run(mut app: Application) {
    use wasm_bindgen::JsCast;
    use winit::platform::web::WindowExtWebSys;

    baumhard::font::fonts::init();

    // Load keybindings from localStorage (never the URL — see
    // `load_web_storage_only`) with a defaults fallback. Failure is non-fatal —
    // see KeybindConfig::load_for_web().
    app.options.keybind_config = crate::application::keybinds::KeybindConfig::load_for_web();

    // Attach canvas to DOM
    let canvas = app.window.canvas().expect("Failed to get canvas");
    let web_window = web_sys::window().expect("No global window");
    let document = web_window.document().expect("No document");
    let body = document.body().expect("No body");
    body.append_child(&canvas).expect("Failed to append canvas");
    canvas.set_width(
        web_window
            .inner_width()
            .expect("web_window.inner_width before first frame")
            .as_f64()
            .expect("inner_width JsValue is f64") as u32,
    );
    canvas.set_height(
        web_window
            .inner_height()
            .expect("web_window.inner_height before first frame")
            .as_f64()
            .expect("inner_height JsValue is f64") as u32,
    );
    let cw = canvas.width();
    let ch = canvas.height();
    log::info!("wasm: canvas sized {}x{}", cw, ch);
    if cw == 0 || ch == 0 {
        log::warn!("wasm: canvas has zero dimension — render surface will be empty");
    }

    // Canvas must be focusable for keyboard events to reach winit.
    // Without tabindex, an HTMLCanvasElement never receives focus.
    canvas.set_attribute("tabindex", "0").ok();
    let _ = canvas.focus();

    // Re-focus on mousedown so clicking the canvas after tabbing
    // to another element restores keyboard input.
    {
        let canvas_for_focus = canvas.clone();
        let focus_cb =
            wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
                let _ = canvas_for_focus.focus();
            });
        canvas
            .add_event_listener_with_callback("mousedown", focus_cb.as_ref().unchecked_ref())
            .ok();
        focus_cb.forget(); // leak — lives for the page lifetime
    }

    // Suppress the browser's default context menu on right-click
    // so the canvas can use right-button gestures
    // (`MouseGesture::RightClick` / `RightDrag`,
    // `Action::FastResizeStart`). Without this, every
    // right-press on the canvas would pop the browser's context
    // menu and the gesture would never reach the threshold-cross.
    //
    // **Shift+RightClick bypass**: holding Shift on the right-press
    // lets the browser's default context menu through. Standard
    // browser convention for "let me get to the OS / dev menu past
    // a page handler" — users debugging the canvas (Inspect Element,
    // screenshotting glyph layout, filing bug reports) get the
    // ergonomic path back. Shift modifier is also unused by any
    // shipped right-button binding, so the convention doesn't
    // collide with anything.
    {
        let pd_cb =
            wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |evt: web_sys::Event| {
                use wasm_bindgen::JsCast;
                let allow_default = evt
                    .dyn_ref::<web_sys::MouseEvent>()
                    .map(|me| me.shift_key())
                    .unwrap_or(false);
                if !allow_default {
                    evt.prevent_default();
                }
            });
        canvas
            .add_event_listener_with_callback("contextmenu", pd_cb.as_ref().unchecked_ref())
            .ok();
        pd_cb.forget(); // leak — lives for the page lifetime
    }

    // No keydown listener suppresses the browser's defaults: the one
    // that did ran while the node text editor was open, and the editor
    // returns with V1-C8 (#249).

    let renderer_window = Arc::clone(&app.window);

    // On WASM, check for ?map= query parameter to override the default
    // path, and ?ipc= for the IPC relay to connect to (off without it).
    let search = web_sys::window()
        .expect("No global window")
        .location()
        .search()
        .unwrap_or_default();
    let mindmap_path = {
        let mut map_path: Option<String> = None;
        let trimmed = search.trim_start_matches('?');
        for pair in trimmed.split('&') {
            if let Some(val) = pair.strip_prefix("map=") {
                map_path = Some(val.to_string());
            }
        }
        map_path.unwrap_or_else(|| app.options.mindmap_path.clone())
    };
    let ipc_url = match crate::application::ipc::browser::ipc_url_from_query(&search) {
        Ok(url) => url,
        Err(why) => {
            log::error!("ipc: {why}; IPC is off");
            None
        }
    };

    // Shared state between the rAF render loop and the winit event
    // loop. Two RefCells so input handlers can borrow InputState
    // and Renderer simultaneously without conflict.
    // `WasmInputState` is declared at module scope (above `fn run`)
    // so cross-platform helpers can take it by `&mut`. Construction of the actual instance happens later in
    // `spawn_local`.

    let renderer_rc: Rc<RefCell<Option<Renderer>>> = Rc::new(RefCell::new(None));
    let input_rc: Rc<RefCell<Option<WasmInputState>>> = Rc::new(RefCell::new(None));

    // Clone Rcs for the spawn_local init future
    let renderer_for_init = renderer_rc.clone();
    let input_for_init = input_rc.clone();

    // Renderer init is async on the browser (adapter + surface setup
    // are Promise-backed). Spawn as a future so the event loop doesn't
    // block waiting for wgpu.
    wasm_bindgen_futures::spawn_local(async move {
        let mut renderer = Renderer::bootstrap_wasm(renderer_window, canvas.clone()).await;
        // Kept beside the renderer for the canvas read-back
        // `capture.screenshot` does on this target.
        let state_canvas = canvas.clone();
        log::info!("WASM: adapter + surface + renderer ready");

        let size = canvas.width();
        let height = canvas.height();
        renderer.process_decree(RenderDecree::SetSurfaceSize(size, height));
        log::info!("WASM: surface configured {}x{}", size, height);

        // The user's settings, before any document: the history limit
        // is the document's from the moment it is installed (D18). The
        // browser reads them from localStorage, never the URL.
        let settings = crate::application::user_config::settings::load();

        // The initial map load, in one call — the browser peer of
        // native's `startup_load::native_startup_document`. Both
        // halves of the browser's load can fail (the fetch, where a
        // `?map=` pointing at nothing gives an HTTP 404, and the
        // parse) and `startup_load` owns both: a rejection becomes
        // the load-failure placard carrying the loader's own message,
        // so the browser build reports a bad map exactly as the
        // desktop build does (CODE_CONVENTIONS §4). No `Result`
        // reaches this file, so there is nowhere here for the two
        // targets to start disagreeing.
        let mut doc = startup_load::wasm_startup_document(&mindmap_path, settings.history_limit).await;

        // The one full projection of the document's life, the renderer
        // loaded with it and cleared to the map's background, and the
        // camera fit to what it draws: the calls native's init makes
        // (`run_native_init`), so the two targets cannot start
        // disagreeing about what is on the canvas.
        let view = MapView::load(&mut doc, &mut renderer);
        renderer.fit_camera_to_content();

        renderer.process_decree(RenderDecree::StartRender);
        log::info!("WASM: StartRender dispatched, rAF loop starting");

        // Pre-warm the render pipeline: one full render cycle so
        // the wgpu driver compiles pipeline shaders, the swapchain
        // allocates its first backing image, and the glyph atlas
        // is populated before the first user-driven frame.
        renderer.prewarm();

        // Populate the shared state now that init is complete.
        *renderer_for_init.borrow_mut() = Some(renderer);

        // The App and User tiers from the bundled JSON / localStorage.
        // Body is `macros::loader::build_macro_registry`, which
        // native's init calls too — including the `log::info!` shape,
        // so cross-target log triage stays uniform. A map carries no
        // macros: its verbs replace them.
        let macros = crate::application::macros::loader::build_macro_registry();

        // Unconditional now that a rejected load yields a placard
        // document: the input state exists even when the map did
        // not, so the canvas that carries the error message is a
        // live canvas — pan, zoom and selection all work on it —
        // rather than an inert one.
        *input_for_init.borrow_mut() = Some(WasmInputState {
            document: doc,
            view: Some(view),
            last_click: None,
            cursor_pos: (0.0, 0.0),
            pending_click: None,
            modifiers: crate::application::platform::input::Modifiers::empty(),
            interaction_mode: super::InteractionMode::Default,
            macros,
            touch_recognizer: super::touch_gesture::TouchGestureRecognizer::new(),
            clock: crate::application::clock::AppClock::default(),
            injected: InjectedQueue::default(),
            // The first frame is asked for just below.
            redraw_owed: true,
            canvas: state_canvas,
        });

        // Event-driven redraw model: instead of an unconditional
        // self-rescheduling rAF (which kept the main thread busy at
        // 60Hz even when the canvas was visually idle), we kick a
        // single `Window::request_redraw` here to paint the first
        // frame and rely on `WasmApp::handle_window_event`'s
        // `WindowEvent::RedrawRequested` arm to do all subsequent
        // renders. Mutating event handlers call
        // `Renderer::request_redraw` after their work, which
        // winit-web translates to an internal rAF dispatch — so the
        // browser only paints when the canvas actually changed. The
        // one self-rescheduling case is the frame itself, while
        // injected input is queued (`event_redraw_requested`): it stops
        // asking the frame the last sample lands.
        if let Some(r) = renderer_for_init.borrow().as_ref() {
            r.request_redraw();
        }
    });

    // Resolve the keybind config once. `action_for(key, ctrl, shift, alt)`
    // answers the dispatch question for every keydown.
    let keybinds: Rc<ResolvedKeybinds> = Rc::new(app.options.keybind_config.resolve());

    // Hand control to winit. The handler owns the renderer / input
    // cells (shared with the rAF render loop) and the resolved keybind
    // table. `spawn_app` returns immediately on
    // the web target — the browser's main thread keeps running and
    // dispatches events through the handler until the tab tears down.
    // The renderer (constructed inside `spawn_local`) already holds
    // an `Arc<Window>` clone made before this handler is built, so
    // the window outlives the loop without the handler needing its
    // own clone.
    // The transport, over the same cells the handler owns. Opened
    // before the app exists: a controller that attaches early is
    // greeted and answered `no_document` until init completes, as a
    // native client that connects before the window is.
    let ipc = ipc::IpcHost {
        input: Rc::clone(&input_rc),
        renderer: Rc::clone(&renderer_rc),
        keybinds: Rc::clone(&keybinds),
        ipc: Rc::new(RefCell::new(None)),
    };
    if let Some(url) = ipc_url {
        ipc.connect(url);
    }
    let handler = WasmApp {
        renderer: renderer_rc,
        input: input_rc,
        keybinds,
        ipc,
    };
    app.event_loop.spawn_app(handler);
}
