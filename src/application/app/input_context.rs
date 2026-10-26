// SPDX-License-Identifier: MPL-2.0

//! Shared per-call context for the native event-loop dispatchers, and
//! the input arms every raw event enters through.
//!
//! Each handler reaches into the same mutable bundle on `InitState`.
//! `InputHandlerContext<'a>` borrows each persistent field as
//! `&'a mut T` so handlers can destructure it; per-event payloads stay
//! as separate function parameters. Built once per event in
//! [`super::run_native_init::InitState::input_context`].

#![cfg(not(target_arch = "wasm32"))]

use crate::application::platform::input::Modifiers as ModifiersState;

use crate::application::color_picker::ColorPickerState;
use crate::application::console::ConsoleState;
use crate::application::document::MindMapDocument;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::macros::MacroRegistry;
use crate::application::renderer::Renderer;
use crate::application::scene_host::AppScene;

use crate::application::platform::input::{ElementState, Key, MouseButton, MouseScrollDelta, TouchPhase};
use crate::application::platform::window::PhysicalPosition;

use super::single_line_edit::SingleLineEditor;
use super::text_edit::TextEditState;
use super::throttled_interaction::{ColorPickerHoverInteraction, DrainContext, ReleaseCommit};
use super::{event_cursor_moved, event_keyboard, event_mouse_click, DragState, InteractionMode, LastClick};

/// Borrowed view of the persistent state every interactive-path
/// dispatcher reads and writes. Built once per event by
/// [`crate::application::app::run_native_init::InitState::input_context`]
/// and passed to `handle_mouse_input`, `handle_cursor_moved`,
/// `handle_keyboard_input`, and `submit_line`.
///
/// The lifetime `'a` ties every field borrow to a single `&mut
/// InitState` — the struct is a re-packaging of existing borrows,
/// not a new owner of state.
pub(in crate::application::app) struct InputHandlerContext<'a> {
    /// The loaded mindmap document, or `None` before the first
    /// successful `loader::load_from_file`.
    pub document: &'a mut Option<MindMapDocument>,
    /// Baumhard tree projection of the document. Rebuilt / mutated
    /// in lockstep with `document`.
    pub mindmap_tree: &'a mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    /// App-layer scene host owning every tree-rendered component.
    pub app_scene: &'a mut AppScene,
    /// The active renderer.
    pub renderer: &'a mut Renderer,
    /// Per-edge connection glyph cache.
    pub scene_cache: &'a mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    /// Current pointer / drag state machine.
    pub drag_state: &'a mut DragState,
    /// High-level interaction mode (`Default`, `Reparent`, `Connect`,
    /// `NodeEdit`, `Resize`). Drives click routing, mode-gated chrome
    /// (resize anchors, target highlights), and selection resolution.
    /// Mutated by mode-transition Action arms (`EnterReparentMode`,
    /// `EnterConnectMode`, `EnterResizeMode`, `ExitMode`,
    /// `ReparentToTarget`, `ConnectToTarget`).
    pub interaction_mode: &'a mut InteractionMode,
    /// Console (slash-command overlay) state.
    pub console_state: &'a mut ConsoleState,
    /// Console command-history ring.
    pub console_history: &'a mut Vec<String>,
    /// Inline single-line editor state (edge label / portal
    /// caption).
    pub single_line_edit_state: &'a mut SingleLineEditor,
    /// Inline node text editor state.
    pub text_edit_state: &'a mut TextEditState,
    /// Glyph-wheel color-picker state.
    pub color_picker_state: &'a mut ColorPickerState,
    /// Previous click (time, position, hit) for double-click detection.
    pub last_click: &'a mut Option<LastClick>,
    /// The node the cursor is currently over, if any.
    pub hovered_node: &'a mut Option<String>,
    /// Last-known cursor position in screen space.
    pub cursor_pos: &'a mut (f64, f64),
    /// Modifier snapshot maintained by `ModifiersChanged` events.
    /// `&mut` only so an injected sample can hold its own modifiers
    /// for the one event it is (`Self::apply_injected`); no handler
    /// writes it.
    pub modifiers: &'a mut ModifiersState,
    /// Per-frame cursor-icon flag — flipped to "hand" over a button
    /// node by the cursor-move handler.
    pub cursor_is_hand: &'a mut bool,
    /// Last cursor icon written via `Window::set_cursor`. The
    /// cursor_moved handler skips the per-frame `set_cursor`
    /// when the desired icon equals this. Required because winit
    /// dedupes `set_cursor` only on macOS / X11; Windows + Wayland
    /// re-issue the underlying OS call every time.
    pub cursor_icon_last: &'a mut winit::window::CursorIcon,
    /// Throttled color-picker hover interaction. The picker's
    /// input paths set `.dirty` when HSV state changes; the
    /// per-frame drain rebuilds the scene + overlay through the
    /// unified adaptive-throttle shell.
    pub picker_hover: &'a mut ColorPickerHoverInteraction,
    /// Resolved user keybinds. `&` (immutable) — no dispatch path
    /// mutates keybinds; query methods (`action_for_*`, `macro_for`,
    /// `custom_mutation_for`, `has_*`) are all `&self`. Track-C
    /// review reflagged the original `&mut` as dead.
    pub keybinds: &'a ResolvedKeybinds,
    /// Macro registry (App + User tiers loaded at startup; Map tier
    /// re-loaded whenever a document is replaced via `open` / `new`).
    /// Mutable so the document-replace path in
    /// `execute_console_line` can rebuild the Map tier without
    /// taking a separate ad-hoc borrow.
    pub macros: &'a mut MacroRegistry,
    /// Where a server-chosen screenshot lands, and the dir a
    /// caller-chosen path may overwrite into.
    pub artifacts: &'a mut crate::application::capture::artifacts::Artifacts,
    /// The frame-sequence recording in progress, for the console's
    /// `record` verb to start and stop.
    pub recorder: &'a mut crate::application::capture::record::Recorder,
    /// The freeze watchdog, for a screenshot to tick as it progresses
    /// and for a clock step to tick once per heartbeat.
    pub watchdog: &'a super::freeze_watchdog::FreezeWatchdog,
    /// What the app is shown in, which the cursor-move handler sets the
    /// cursor icon on — a no-op with no window (`app::shell`).
    pub shell: &'a super::shell::Shell,
    /// Touch gesture state machine (`super::touch_gesture`).
    pub touch_recognizer: &'a mut super::touch_gesture::TouchGestureRecognizer,
    /// The finger holding a resize handle, if one does
    /// (`super::touch_handle`).
    pub touch_handle: &'a mut super::touch_handle::HandleTouch,
    /// The app-clock ms at which a tree-mutating drag suspended the
    /// animation tick, or `None` while ticks run. Read and written by
    /// the frame drain ([`Self::drain_frame`]).
    pub anim_pause_start_ms: &'a mut Option<u64>,
    /// Raw input the IPC `input` family queued for the heartbeat
    /// (`format/ipc.md` §`input`). Always empty without `--ipc`.
    pub injected: &'a mut crate::application::ipc::injected::InjectedQueue,
    /// Set by `Action::Quit`: the loop ends at its next exit check
    /// (`app::pump::iterate`).
    pub quit_requested: &'a mut bool,
    /// The app clock every behavior reads (`crate::application::clock`).
    /// `&mut` because a `clock` console line can switch or step it.
    pub clock: &'a mut crate::application::clock::AppClock,
}

impl<'a> InputHandlerContext<'a> {
    /// The app as an IPC change event reads it
    /// ([`crate::application::ipc::events::Observed`]). A read-only
    /// view: it is what lets an observer run between the steps of a
    /// macro, from inside the funnel that is executing them.
    pub(in crate::application::app) fn observed(&self) -> crate::application::ipc::events::Observed<'_> {
        crate::application::ipc::events::Observed {
            document: self.document.as_ref(),
            camera: self.renderer.camera(),
            mode: &*self.interaction_mode,
        }
    }

    /// Renderer-free slice for a throttled drag's release commit —
    /// see
    /// [`super::throttled_interaction::release`]. The commit body
    /// mutates the model and the scene cache, then hands back a
    /// [`super::throttled_interaction::ReleaseRefresh`] the caller
    /// runs against [`Self::drain_context`].
    pub(in crate::application::app) fn release_commit(&mut self) -> ReleaseCommit<'_> {
        ReleaseCommit {
            document: &mut *self.document,
            mindmap_tree: &mut *self.mindmap_tree,
            scene_cache: &mut *self.scene_cache,
        }
    }

    /// The per-frame drain bundle, rebuilt from the event-loop
    /// context. `drain_inputs` builds the same struct from
    /// `InitState` directly; the release path reaches it from here
    /// so a drag's release-commit and its per-frame drains share
    /// one context type.
    pub(in crate::application::app) fn drain_context(&mut self) -> DrainContext<'_> {
        DrainContext {
            document: &mut *self.document,
            mindmap_tree: &mut *self.mindmap_tree,
            app_scene: &mut *self.app_scene,
            renderer: &mut *self.renderer,
            scene_cache: &mut *self.scene_cache,
            color_picker_state: &mut *self.color_picker_state,
            interaction_mode: &*self.interaction_mode,
            throttled: !self.clock.is_virtual(),
        }
    }

    /// Decompose into the cross-platform
    /// [`super::input_context_core::InputContextCore`] +
    /// native-only [`super::input_context_core::NativeContextExt`]
    /// pair the unified `dispatch_action` expects post-Track-C.
    /// Each returned view re-borrows from `self` so the original
    /// `InputHandlerContext` is borrowed as `&mut` for the
    /// duration; the views are dropped before the dispatcher
    /// returns. WASM constructs an `InputContextCore` directly via
    /// `WasmInputState::input_context_core` and passes `None` for
    /// the extension.
    ///
    /// `'s` is shorter than `'a` — the views borrow from `self`,
    /// not from the original outer borrow source.
    pub fn split_borrow<'s>(
        &'s mut self,
    ) -> (
        super::input_context_core::InputContextCore<'s>,
        super::input_context_core::NativeContextExt<'s>,
    ) {
        (
            super::input_context_core::InputContextCore {
                document: self.document.as_mut(),
                mindmap_tree: &mut *self.mindmap_tree,
                app_scene: &mut *self.app_scene,
                renderer: &mut *self.renderer,
                scene_cache: &mut *self.scene_cache,
                text_edit_state: &mut *self.text_edit_state,
                last_click: &mut *self.last_click,
                cursor_pos: &mut *self.cursor_pos,
                modifiers: &*self.modifiers,
                keybinds: self.keybinds,
                macros: &mut *self.macros,
                interaction_mode: &mut *self.interaction_mode,
                clock: &*self.clock,
            },
            super::input_context_core::NativeContextExt {
                drag_state: &mut *self.drag_state,
                console_state: &mut *self.console_state,
                console_history: &mut *self.console_history,
                single_line_edit_state: &mut *self.single_line_edit_state,
                color_picker_state: &mut *self.color_picker_state,
                hovered_node: &mut *self.hovered_node,
                cursor_is_hand: &mut *self.cursor_is_hand,
                cursor_icon_last: &mut *self.cursor_icon_last,
                picker_hover: &mut *self.picker_hover,
            },
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
        // A finger that starts a gesture on a resize handle is the
        // left button instead (#160, `touch_handle`), and never reaches
        // the recognizer. The handle question is asked only for the
        // one event that can claim a handle.
        let recognizer_idle = self.touch_recognizer.is_idle();
        let on_handle = phase == Phase::Started && recognizer_idle && self.finger_lands_on_resize_handle(pos);
        match self.touch_handle.route(phase, id, recognizer_idle, on_handle) {
            super::touch_handle::TouchRoute::Recognizer => {}
            super::touch_handle::TouchRoute::Ignore => return false,
            super::touch_handle::TouchRoute::Handle(step) => return self.drive_handle_finger(step, location),
        }
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

    /// True when a finger at `pos` (surface pixels) lands on a resize
    /// handle the left press would capture — the same question, through
    /// the same `event_mouse_click::resize_handle_press`, so a finger and
    /// a mouse cannot disagree about what is a handle. Nothing outside
    /// `InteractionMode::Resize`.
    fn finger_lands_on_resize_handle(&self, pos: (f64, f64)) -> bool {
        let canvas_pos = self.renderer.screen_to_canvas(pos.0 as f32, pos.1 as f32);
        event_mouse_click::resize_handle_press(
            self.document.as_ref(),
            self.interaction_mode,
            canvas_pos,
            super::HANDLE_HIT_TOLERANCE_PX * self.renderer.canvas_per_pixel(),
        )
        .any()
    }

    /// Run a handle finger's step as the left-button events it stands
    /// for, through the same `on_*` entry points a mouse reaches: the
    /// press captures the handle, the moves promote and drive the
    /// `Throttled` resize, and the release commits it with its undo
    /// entry. Returns whether to request a redraw.
    fn drive_handle_finger(
        &mut self,
        step: super::touch_handle::HandleStep,
        location: PhysicalPosition<f64>,
    ) -> bool {
        use super::touch_handle::HandleStep;
        let moved = self.on_cursor_moved(location);
        match step {
            HandleStep::Press => self.on_mouse_input(ElementState::Pressed, MouseButton::Left),
            HandleStep::Move => moved,
            HandleStep::Release => self.on_mouse_input(ElementState::Released, MouseButton::Left),
        }
    }

    /// `WindowEvent::MouseInput`. Returns whether to request a redraw.
    ///
    /// This, the three `on_*` methods below and
    /// [`Self::dispatch_touch_event`] are winit's `handle_event` input
    /// arms, lifted out so the IPC `input` family reaches the handlers
    /// through the same bodies a winit event does (`format/ipc.md`
    /// §`input`): from here down, nothing can tell the two apart — and
    /// with no window (`--headless`) they are the only way input
    /// arrives at all. They live on the context rather than on
    /// `InitState` so the heartbeat (`app::heartbeat`) reaches them from
    /// wherever it runs — either driver's loop, or a `clock step` inside
    /// a console line.
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
        let shell = self.shell;
        event_cursor_moved::handle_cursor_moved(position, shell, self);
        // Hover that doesn't drag and doesn't open the
        // picker doesn't change visuals — no redraw needed.
        // Any active drag (Pending → Throttled etc.) or an
        // open picker requires a fresh frame so the drag
        // preview / hover marker tracks the cursor.
        !matches!(*self.drag_state, DragState::None) || self.color_picker_state.is_open()
    }

    /// `WindowEvent::KeyboardInput`. A press reaches the full
    /// handler; a release only ends a pan the key holds, which
    /// changes no pixels. Returns whether to request a redraw.
    pub(super) fn on_keyboard_input(&mut self, logical_key: Key, state: ElementState) -> bool {
        if state != ElementState::Pressed {
            event_keyboard::handle_key_release(&logical_key, self);
            return false;
        }
        event_keyboard::handle_keyboard_input(logical_key, self);
        true
    }

    /// `WindowEvent::Focused(false)`: the window may never deliver the
    /// release any in-flight gesture is waiting for, so resolve them
    /// all now (#153) rather than leave one unreachable.
    ///
    /// The drag state ends at `None` (`DragState::end_on_focus_loss`
    /// says what each variant owes); a throttled drag is committed
    /// through the same release gate a button release uses, with
    /// `Release::FocusLost`, so it lands with its undo entry exactly
    /// as the owning release would have landed it. A color-picker
    /// wheel gesture ends the way its release ends it. Returns whether
    /// to request a redraw.
    pub(super) fn on_focus_lost(&mut self) -> bool {
        // A finger holding a resize handle loses its claim with the
        // drag it was driving, which the commit below resolves.
        self.touch_handle.forget();
        let picker_gesture_ended =
            super::color_picker_flow::end_color_picker_gesture(self.color_picker_state);
        let was_idle = matches!(*self.drag_state, DragState::None);
        if let Some(mut drag) = self.drag_state.end_on_focus_loss() {
            use super::throttled_interaction::release::{commit_if_resolved, Release};
            if let Some(refresh) = commit_if_resolved(Release::FocusLost, &mut drag, self.release_commit()) {
                refresh.execute(self.drain_context());
            }
        }
        picker_gesture_ended || !was_idle
    }
}
