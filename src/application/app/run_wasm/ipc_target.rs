// SPDX-License-Identifier: MPL-2.0

//! The browser's [`CommandTarget`]: what an IPC command handler drives
//! the app through on this target. It holds the event-handler context
//! ([`Borrowed`]) every browser input handler receives, and each
//! method is one existing funnel and nothing else
//! (`work_plans/LLM_IPC.md` §D5, "Crossing the `app` boundary"): an
//! IPC-driven action or macro takes the same `dispatch_compatible`
//! arms, the same gates and the same undo entries as a keystroke
//! would, and injected raw input enters through the very `on_*`
//! methods the winit arms call. A screenshot is the canvas, drawn and
//! read back in one task.
//!
//! The peer of native's `app/ipc_target.rs`, method for method. What
//! differs is what the target does not have: no console runtime
//! ([`CommandTarget::run_console_line`] answers `None`, and the
//! registry refuses `act.console` before it is asked), no drag state
//! (`query.mode` reports `drag_state.kind: "None"` and no animation
//! pause), one modal (the node text editor), and no recorder or file
//! destination — the capture is [`CommandTarget::capture_canvas`].

#![cfg(target_arch = "wasm32")]

use glam::Vec2;
use serde_json::{json, Value};

use crate::application::app::animation_drain;
use crate::application::app::dispatch::{self, DispatchHit};
use crate::application::app::ipc_scene::{self, action_outcome, resolve_point, surface_of};
use crate::application::app::ipc_state;
use crate::application::capture::geometry;
use crate::application::clock::{AppClock, ClockMode, ClockRefusal, StepRequest, Stepped};
use crate::application::console::LineOutcome;
use crate::application::document::MindMapDocument;
use crate::application::ipc::events::Observed;
use crate::application::ipc::injected::{resolve_command, Delivery, Overloaded, Pacing, Sample, Surface};
use crate::application::ipc::space::{camera_json, Point};
use crate::application::ipc::{
    ActionOutcome, CanvasCapture, CanvasOutcome, CommandTarget, Injected, SceneView, Settling, ShellState,
    IPC_STEPS_LABEL, PROTOCOL_VERSION,
};
use crate::application::keybinds::Action;
use crate::application::macros::MacroStep;
use crate::application::source_tier::SourceTier;

use super::{step, Borrowed, WasmMacroDispatchTarget};

/// What `HtmlCanvasElement::to_data_url` puts before the base64 of a
/// PNG. The default image type is PNG, which is the one format
/// `capture.screenshot` serves.
const PNG_DATA_URL_PREFIX: &str = "data:image/png;base64,";

/// The app, as one IPC command sees it. Built per command, like a
/// [`Borrowed`] is built per event.
pub(super) struct IpcCommandTarget<'t, 'a> {
    app: &'t mut Borrowed<'a>,
    /// Set when a funnel reports it did something that may have
    /// changed pixels; read back through
    /// [`CommandTarget::changed_view`].
    changed_view: bool,
}

impl<'t, 'a> IpcCommandTarget<'t, 'a> {
    pub(super) fn new(app: &'t mut Borrowed<'a>) -> Self {
        IpcCommandTarget {
            app,
            changed_view: false,
        }
    }

    /// `point` in both spaces, `(surface, canvas)`, through the
    /// renderer's camera.
    fn resolve(&self, point: Point) -> (Vec2, Vec2) {
        resolve_point(self.app.renderer, point)
    }

    /// Stand a synthetic pointer at `point`, as a real one would have
    /// stood before its press: the cursor moves there, and the click
    /// hit is what the canonical hit test finds under it
    /// (`compute_click_hit`, the one mouse presses use, never a fork
    /// of it).
    fn place_pointer(&mut self, point: Point) -> DispatchHit {
        let (surface, canvas) = self.resolve(point);
        self.app.input.cursor_pos = (f64::from(surface.x), f64::from(surface.y));
        let parts = crate::application::app::compute_click_hit(
            canvas,
            self.app.input.mindmap_tree.as_mut(),
            &mut self.app.input.app_scene,
        );
        DispatchHit {
            click_hit: parts.click_hit,
            canvas_pos: canvas,
        }
    }

    /// The macro dispatch target over the same context, observing
    /// between steps through `after_step`.
    fn macro_target<'m>(
        &'m mut self,
        after_step: &'m mut dyn FnMut(Observed<'_>),
    ) -> WasmMacroDispatchTarget<'m> {
        WasmMacroDispatchTarget {
            input: self.app.input,
            renderer: self.app.renderer,
            keybinds: self.app.keybinds,
            after_step: Some(after_step),
        }
    }
}

impl Borrowed<'_> {
    /// Whether the loop must keep iterating without further input: the
    /// browser's `needs_continuation`. No drag state and no picker
    /// here, so the sources are the two time-driven ones native shares
    /// with it — an animation ticking and injected input queued, each
    /// in real time only, since under a virtual clock the heartbeats
    /// that advance them are a step's.
    pub(super) fn needs_continuation(&self) -> bool {
        animation_drain::keeps_ticking(Some(&self.input.document), &self.input.clock)
            || (!self.input.injected.is_empty() && !self.input.clock.is_virtual())
    }

    /// What `clock.wait` reads ([`Settling`]), as native's
    /// `InitState::settling` reads it.
    pub(super) fn settling(&self) -> Settling {
        Settling {
            animations_active: self.input.document.has_active_animations(),
            continuing: self.needs_continuation(),
            redraw_owed: self.input.redraw_owed,
            virtual_clock: self.input.clock.is_virtual(),
        }
    }
}

impl CommandTarget for IpcCommandTarget<'_, '_> {
    fn dispatch_action(&mut self, action: Action, pointer: Option<Point>) -> ActionOutcome {
        // The synthetic pointer is part of this one dispatch, like the
        // `DispatchHit` it builds. The cursor goes back afterwards, so
        // the next real wheel zoom keeps agreeing with the user's
        // actual pointer, which never moved.
        let real_cursor = self.app.input.cursor_pos;
        let hit = pointer.map(|point| self.place_pointer(point));
        let outcome = {
            let mut core = self
                .app
                .input
                .input_context_core(self.app.renderer, self.app.keybinds);
            dispatch::action_core::dispatch_compatible(&action, &mut core, hit.as_ref())
        };
        self.app.input.cursor_pos = real_cursor;
        let outcome = action_outcome(outcome);
        self.changed_view |= outcome == ActionOutcome::Handled;
        outcome
    }

    fn run_console_line(&mut self, _line: &str) -> Option<LineOutcome> {
        // No console runtime on this target; the registry refuses
        // `act.console` as unsupported before a handler could ask.
        None
    }

    fn has_macro(&self, macro_id: &str) -> bool {
        self.app.input.macros.get_with_source(macro_id).is_some()
    }

    fn run_macro(&mut self, macro_id: &str, after_step: &mut dyn FnMut(Observed<'_>)) -> bool {
        let ran = dispatch::macro_core::dispatch_macro(macro_id, &mut self.macro_target(after_step));
        self.changed_view |= ran;
        ran
    }

    fn run_macro_steps(&mut self, steps: &[MacroStep], after_step: &mut dyn FnMut(Observed<'_>)) -> bool {
        // `User`: the steps were authored by the IPC client, and IPC
        // executes at the User tier (format/ipc.md §Trust model).
        let ran = dispatch::macro_core::run_macro_steps(
            IPC_STEPS_LABEL,
            steps,
            SourceTier::User,
            &mut self.macro_target(after_step),
        );
        self.changed_view |= ran;
        ran
    }

    fn observed(&self) -> Option<Observed<'_>> {
        Some(self.app.observed())
    }

    fn changed_view(&self) -> bool {
        self.changed_view
    }

    fn document(&self) -> Option<&MindMapDocument> {
        Some(&self.app.input.document)
    }

    fn animation_pause(&self) -> Option<u64> {
        // The pause is keyed to native's tree-mutating drags, which
        // this target has none of (`CLAUDE.md` "Animation pause
        // during a drag").
        None
    }

    fn shell_state(&self, which: ShellState) -> Value {
        let input = &*self.app.input;
        match which {
            ShellState::Camera => camera_json(self.app.renderer.camera()),
            ShellState::Mode => ipc_state::mode_to_ipc_json(
                &input.interaction_mode,
                ipc_state::no_drag_state_json(),
                ipc_state::OpenModals {
                    text: input.text_edit_state.is_open(),
                    ..ipc_state::OpenModals::default()
                },
                None,
                input.cursor_pos,
            ),
            ShellState::Status => json!({ "text": self.app.renderer.mode_status_text() }),
            ShellState::Editors => ipc_state::editors_to_ipc_json(
                Value::Null,
                Value::Null,
                Value::Null,
                input.text_edit_state.to_ipc_json(),
                Some(&input.document),
            ),
        }
    }

    fn scene(&self) -> Option<SceneView<'_>> {
        Some(SceneView {
            document: &self.app.input.document,
            mindmap_tree: self.app.input.mindmap_tree.as_ref(),
            app_scene: &self.app.input.app_scene,
            camera: self.app.renderer.camera(),
        })
    }

    fn hit_test(&mut self, point: Point) -> Option<Value> {
        let (surface, canvas) = self.resolve(point);
        let canvas_per_pixel = dispatch::CanvasPerPixel::of(self.app.renderer);
        let input = &mut *self.app.input;
        Some(ipc_scene::hit_report(
            canvas,
            surface,
            input.mindmap_tree.as_mut(),
            &mut input.app_scene,
            &input.document.mindmap,
            canvas_per_pixel,
        ))
    }

    fn surface_point(&self, point: Point) -> (f64, f64) {
        surface_of(self.app.renderer, point)
    }

    fn inject(&mut self, samples: Vec<Sample>, pacing: Pacing) -> Result<Injected, Overloaded> {
        let applied = match self.app.input.injected.admit(samples, pacing)? {
            Delivery::Now(samples) => {
                // One camera for the whole burst, the one in force
                // now: a move the burst makes cannot re-aim its own
                // later events.
                let renderer = &*self.app.renderer;
                let resolved: Vec<Sample<Surface>> =
                    resolve_command(samples, |point| surface_of(renderer, point)).collect();
                for sample in resolved {
                    self.changed_view |= self.app.apply_injected(sample);
                }
                true
            }
            // The frame lands the rest: a non-empty queue keeps the
            // frame asking for the next one until the last sample has
            // landed (`event_redraw_requested`), and the repaint this
            // command reports asks for the first.
            Delivery::Queued => {
                self.changed_view = true;
                false
            }
        };
        Ok(Injected {
            applied,
            pending: self.app.input.injected.len(),
        })
    }

    fn injected_pending(&self) -> usize {
        self.app.input.injected.len()
    }

    fn capture_canvas(&mut self, sidecar: bool) -> Option<Result<CanvasCapture, String>> {
        let camera = *self.app.renderer.camera();
        // Draw now, into the canvas, and read it back before this task
        // ends. On WebGL (no `preserveDrawingBuffer`) that is what
        // holds the frame just drawn: the buffer is cleared once the
        // task yields. On WebGPU, which presents the canvas's current
        // texture at the rendering update, whether a same-task read
        // sees this frame or the last presented one is unverified
        // (`format/ipc.md` §`capture.screenshot`); if it is the
        // latter, this becomes a two-step: draw now, read at the next
        // `RedrawRequested`.
        let outcome = match self.app.renderer.process() {
            Ok(()) => {
                self.app.input.redraw_owed = false;
                let data_url = match self.app.input.canvas.to_data_url() {
                    Ok(url) => url,
                    Err(e) => return Some(Err(format!("the canvas could not be read back: {e:?}"))),
                };
                let Some(png_base64) = data_url.strip_prefix(PNG_DATA_URL_PREFIX) else {
                    return Some(Err(format!(
                        "the canvas read back as something other than a PNG data URL: {}…",
                        &data_url[..data_url.len().min(32)]
                    )));
                };
                let input = &*self.app.input;
                let sidecar = sidecar.then(|| {
                    geometry::sidecar(
                        input.mindmap_tree.as_ref(),
                        &input.document.mindmap,
                        &camera,
                        PROTOCOL_VERSION,
                        input.document.revision(),
                    )
                });
                CanvasOutcome::Drawn {
                    camera,
                    png_base64: png_base64.to_string(),
                    sidecar,
                }
            }
            Err(skip) => CanvasOutcome::Skipped { camera, skip },
        };
        // What is left is what a frame would still move on from.
        let settled = !self.app.needs_continuation();
        Some(Ok(CanvasCapture { outcome, settled }))
    }

    fn clock(&self) -> &AppClock {
        &self.app.input.clock
    }

    fn set_clock_mode(&mut self, mode: ClockMode) -> Result<bool, ClockRefusal> {
        // A switch moves no reading and changes nothing a frame shows.
        // Nothing records on this target, so a switch is never refused
        // for a recording.
        self.app.input.clock.switch_mode(mode, false)
    }

    fn step_clock(&mut self, request: StepRequest) -> Result<Stepped, ClockRefusal> {
        let stepped = step::step(self.app, request)?;
        // Time moved, and with it anything time drives: one frame to
        // show where the step left the app.
        self.changed_view = true;
        Ok(stepped)
    }
}
