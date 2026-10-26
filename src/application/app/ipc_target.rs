// SPDX-License-Identifier: MPL-2.0

//! The native [`CommandTarget`]: what an IPC command handler drives
//! the app through. It holds the event loop's own state and builds
//! the [`InputHandlerContext`] every input handler receives, and each
//! method is one existing funnel and nothing else
//! (`work_plans/LLM_IPC.md` §D5, "Crossing the `app` boundary"). An
//! IPC-driven action, console line or macro therefore takes the same
//! arms, the same gates and the same undo entries as a keystroke
//! would, and injected raw input enters through the very
//! `on_*` methods `handle_event`'s winit arms call. A
//! screenshot is the screenshot engine (`capture_flow::capture`), the
//! function the `screenshot` console verb calls too.
//!
//! Native-only at this module boundary, with the module it serves
//! (`CLAUDE.md` "Dual-target status").

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec2;
use serde_json::{json, Value};

use crate::application::capture::plan::CaptureRequest;
use crate::application::capture::record::{RecordDestination, Started, Summary};
use crate::application::capture::{CaptureError, Destination};
use crate::application::clock::{AppClock, ClockMode, ClockRefusal, StepRequest, Stepped};
use crate::application::console::LineOutcome;
use crate::application::document::MindMapDocument;
use crate::application::ipc::events::Observed;
use crate::application::ipc::injected::{resolve_command, Delivery, Overloaded, Pacing, Sample, Surface};
use crate::application::ipc::space::{camera_json, Point, Space};
use crate::application::ipc::{CommandTarget, Injected, SceneView, Screenshot, ShellState};
use crate::application::keybinds::Action;
use crate::application::macros::MacroStep;
use crate::application::renderer::Renderer;
use crate::application::source_tier::SourceTier;

use super::capture_flow::{self, CaptureScene};
use super::console_input::exec::execute_console_line;
use super::dispatch::{self, DispatchHit, DispatchOutcome};
use super::input_context::InputHandlerContext;
use super::record_flow;
use super::run_native_init::InitState;
use super::{ipc_scene, ipc_state};

/// How an anonymous `act.macro` step sequence is named in the macro
/// dispatcher's diagnostics.
const IPC_STEPS_LABEL: &str = "(ipc steps)";

/// The app, as one IPC command sees it. Built per command, like an
/// [`InputHandlerContext`] is built per event.
pub(super) struct IpcCommandTarget<'a> {
    init: &'a mut InitState,
    /// Set when a funnel reports it did something that may have
    /// changed pixels; read back through
    /// [`CommandTarget::changed_view`].
    changed_view: bool,
}

impl<'a> IpcCommandTarget<'a> {
    pub(super) fn new(init: &'a mut InitState) -> Self {
        IpcCommandTarget {
            init,
            changed_view: false,
        }
    }

    /// The handler context, built as `handle_event` builds it per
    /// event.
    fn ctx(&mut self) -> InputHandlerContext<'_> {
        self.init.input_context()
    }

    /// `point` in both spaces, `(surface, canvas)`, through the
    /// renderer's camera.
    fn resolve(&self, point: Point) -> (Vec2, Vec2) {
        resolve_point(&self.init.renderer, point)
    }

    /// Stand a synthetic pointer at `point`, as a real one would have
    /// stood before its press: the cursor moves there, and the click
    /// hit is what the canonical hit test finds under it
    /// (`compute_click_hit`, the one mouse presses use, never a
    /// fork of it).
    fn place_pointer(&mut self, point: Point) -> DispatchHit {
        let (surface, canvas) = self.resolve(point);
        self.init.cursor_pos = (f64::from(surface.x), f64::from(surface.y));
        let parts =
            super::compute_click_hit(canvas, self.init.mindmap_tree.as_mut(), &mut self.init.app_scene);
        DispatchHit {
            click_hit: parts.click_hit,
            canvas_pos: canvas,
            // A synthetic pointer holds no button.
            held_by: None,
        }
    }
}

/// `point` in both spaces, `(surface, canvas)`, through `renderer`'s
/// camera as it stands now.
fn resolve_point(renderer: &Renderer, point: Point) -> (Vec2, Vec2) {
    match point.space {
        Space::Surface => (
            Vec2::new(point.x, point.y),
            renderer.screen_to_canvas(point.x, point.y),
        ),
        Space::Canvas => {
            let canvas = Vec2::new(point.x, point.y);
            (renderer.canvas_to_screen(canvas), canvas)
        }
    }
}

/// Where `point` lands on the surface, through `renderer`'s camera as
/// it stands now: how an injected event's position is resolved.
pub(super) fn surface_of(renderer: &Renderer, point: Point) -> Surface {
    let (surface, _) = resolve_point(renderer, point);
    (f64::from(surface.x), f64::from(surface.y))
}

impl InitState {
    /// The app as the IPC event stream reads it
    /// ([`Observed`]): the document, the camera and the interaction
    /// mode.
    pub(super) fn observed(&self) -> Observed<'_> {
        Observed {
            document: self.document.as_ref(),
            camera: self.renderer.camera(),
            mode: &self.interaction_mode,
        }
    }
}

impl CommandTarget for IpcCommandTarget<'_> {
    fn dispatch_action(&mut self, action: Action, pointer: Option<Point>) -> bool {
        // The synthetic pointer is part of this one dispatch, like the
        // `DispatchHit` it builds. The cursor goes back afterwards, so
        // the hover state, the cursor icon and the next real wheel
        // zoom all keep agreeing with the user's actual pointer, which
        // never moved.
        let real_cursor = self.init.cursor_pos;
        let hit = pointer.map(|point| self.place_pointer(point));
        let handled = matches!(
            dispatch::dispatch_action(action, &mut self.ctx(), hit.as_ref()),
            DispatchOutcome::Handled
        );
        self.init.cursor_pos = real_cursor;
        self.changed_view |= handled;
        handled
    }

    fn run_console_line(&mut self, line: &str) -> Option<LineOutcome> {
        let run = execute_console_line(&mut self.ctx(), line)?;
        self.changed_view |= run.changed_view;
        Some(run.outcome)
    }

    fn has_macro(&self, macro_id: &str) -> bool {
        self.init.macros.get_with_source(macro_id).is_some()
    }

    fn run_macro(&mut self, macro_id: &str, after_step: &mut dyn FnMut(Observed<'_>)) -> bool {
        let ran = dispatch::dispatch_macro(
            macro_id,
            &mut self.ctx(),
            Some(&mut |ctx: &InputHandlerContext<'_>| after_step(ctx.observed())),
        );
        self.changed_view |= ran;
        ran
    }

    fn run_macro_steps(&mut self, steps: &[MacroStep], after_step: &mut dyn FnMut(Observed<'_>)) -> bool {
        // `User`: the steps were authored by the IPC client, and IPC
        // executes at the User tier (format/ipc.md §Trust model).
        let ran = dispatch::run_macro_steps(
            IPC_STEPS_LABEL,
            steps,
            SourceTier::User,
            &mut self.ctx(),
            Some(&mut |ctx: &InputHandlerContext<'_>| after_step(ctx.observed())),
        );
        self.changed_view |= ran;
        ran
    }

    fn observed(&self) -> Option<Observed<'_>> {
        Some(self.init.observed())
    }

    fn changed_view(&self) -> bool {
        self.changed_view
    }

    fn document(&self) -> Option<&MindMapDocument> {
        self.init.document.as_ref()
    }

    fn animation_pause(&self) -> Option<u64> {
        self.init.anim_pause_start_ms
    }

    fn shell_state(&self, which: ShellState) -> Value {
        match which {
            ShellState::Camera => camera_json(self.init.renderer.camera()),
            ShellState::Mode => ipc_state::mode_to_ipc_json(
                &self.init.interaction_mode,
                &self.init.drag_state,
                &self.init.console_state,
                &self.init.color_picker_state,
                &self.init.single_line_edit_state,
                &self.init.text_edit_state,
                self.init.hovered_node.as_deref(),
                self.init.cursor_pos,
            ),
            ShellState::Status => json!({ "text": self.init.renderer.mode_status_text() }),
            ShellState::Editors => ipc_state::editors_to_ipc_json(
                &self.init.console_state,
                &self.init.color_picker_state,
                &self.init.single_line_edit_state,
                &self.init.text_edit_state,
                self.init.document.as_ref(),
            ),
        }
    }

    fn scene(&self) -> Option<SceneView<'_>> {
        Some(SceneView {
            document: self.init.document.as_ref()?,
            mindmap_tree: self.init.mindmap_tree.as_ref(),
            app_scene: &self.init.app_scene,
            camera: self.init.renderer.camera(),
        })
    }

    fn hit_test(&mut self, point: Point) -> Option<Value> {
        let (surface, canvas) = self.resolve(point);
        let canvas_per_pixel = dispatch::CanvasPerPixel::of(&self.init.renderer);
        let doc = self.init.document.as_ref()?;
        Some(ipc_scene::hit_report(
            canvas,
            surface,
            self.init.mindmap_tree.as_mut(),
            &mut self.init.app_scene,
            &doc.mindmap,
            canvas_per_pixel,
        ))
    }

    fn surface_point(&self, point: Point) -> (f64, f64) {
        surface_of(&self.init.renderer, point)
    }

    fn inject(&mut self, samples: Vec<Sample>, pacing: Pacing) -> Result<Injected, Overloaded> {
        let applied = match self.init.injected.admit(samples, pacing)? {
            Delivery::Now(samples) => {
                // One camera for the whole burst, the one in force
                // now: a move the burst makes cannot re-aim its own
                // later events.
                let renderer = &self.init.renderer;
                let resolved: Vec<Sample<Surface>> =
                    resolve_command(samples, |point| surface_of(renderer, point)).collect();
                let mut ctx = self.init.input_context();
                for sample in resolved {
                    self.changed_view |= ctx.apply_injected(sample);
                }
                true
            }
            // The heartbeat requests the frames: a non-empty queue is
            // a continuation source, so the loop keeps iterating and
            // redrawing until the last sample has landed.
            Delivery::Queued => false,
        };
        Ok(Injected {
            applied,
            pending: self.init.injected.len(),
        })
    }

    fn injected_pending(&self) -> usize {
        self.init.injected.len()
    }

    fn capture(
        &mut self,
        request: &CaptureRequest,
        destination: Destination,
        sidecar: bool,
    ) -> Option<Result<Screenshot, CaptureError>> {
        let init = &mut *self.init;
        init.document.as_ref()?;
        let (scene, artifacts, _) = CaptureScene::of_init(init);
        let taken = capture_flow::capture(scene, request, destination, sidecar, artifacts);
        // After the capture, which ran the camera-geometry rebuild the
        // next frame owed: what is left is what a frame would still
        // move on from.
        let settled = !init.needs_continuation();
        Some(taken.map(|outcome| Screenshot { outcome, settled }))
    }

    fn record_start(
        &mut self,
        request: &CaptureRequest,
        fps: u32,
        max_frames: u32,
        destination: RecordDestination,
    ) -> Option<Result<Started, CaptureError>> {
        let init = &mut *self.init;
        init.document.as_ref()?;
        let clock = init.clock.clone();
        let (scene, artifacts, recorder) = CaptureScene::of_init(init);
        Some(record_flow::start(
            scene,
            recorder,
            artifacts,
            request,
            fps,
            max_frames,
            destination,
            &clock,
        ))
    }

    fn record_stop(&mut self) -> Result<Summary, CaptureError> {
        let init = &mut *self.init;
        record_flow::stop(
            &mut init.recorder,
            &mut init.artifacts,
            &init.watchdog,
            &init.clock,
        )
    }

    fn clock(&self) -> &AppClock {
        &self.init.clock
    }

    fn set_clock_mode(&mut self, mode: ClockMode) -> Result<bool, ClockRefusal> {
        // A switch moves no reading and changes nothing a frame shows:
        // the next frame draws what this one would have.
        self.init.input_context().set_clock_mode(mode)
    }

    fn step_clock(&mut self, request: StepRequest) -> Result<Stepped, ClockRefusal> {
        let stepped = self.init.input_context().step(request)?;
        // Time moved, and with it anything time drives: one frame to
        // show where the step left the app.
        self.changed_view = true;
        Ok(stepped)
    }
}
