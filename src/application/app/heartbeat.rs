// SPDX-License-Identifier: MPL-2.0

//! The heartbeat: what one frame of the app does, and the clock step
//! that runs it on virtual time (`format/ipc.md` §`clock`;
//! `work_plans/LLM_IPC.md` §D9).
//!
//! **A frame has an input-driven half and a time-driven half.** The
//! input-driven half — the throttled drag and the picker hover, the
//! rubber band, the camera-geometry rebuild — flushes what input
//! already did, and needs no clock. The time-driven half — one sample
//! of the IPC injected-input queue, the animation tick, a recording's
//! frame — is a function of *when* the frame runs. In real time the
//! two always run together, once per event-loop iteration, as they
//! always have. Under a virtual clock, time moves only when
//! [`InputHandlerContext::step`] moves it: the loop's own iterations
//! run the input-driven half alone, so a human's drag still lands while
//! the animations stand still, and the time-driven half runs inside the
//! step, at the virtual instants the step names. That split is what
//! makes a stepped run a pure function of its command sequence.
//!
//! **What a step runs.** A heartbeat every
//! [`HEARTBEAT_MS`](crate::application::clock::HEARTBEAT_MS) virtual
//! milliseconds, the last on the step's end ([`heartbeat_times`]), each
//! the whole frame — one injected sample, the drains, the tick, the
//! recorder. Between two of them, a running recording's frame boundary
//! gets a tick of its own ([`Beat::Boundary`]): the clock stops on the
//! boundary exactly, the frame drains and the recorder takes its frame,
//! so a stepped recording's frames are its `1000/fps` instants, each
//! rounded up to a whole millisecond, and nothing else. The rounding
//! keeps the virtual clock on whole milliseconds, which is what the
//! animation tick reads: a frame shows exactly the instant its `t_ms`
//! names. No injected sample lands on a boundary tick, so
//! recording a gesture never changes when the gesture's samples land.
//! Every heartbeat and every boundary tick ticks the freeze watchdog,
//! which is what makes a long step safe by construction: a step makes
//! progress, so only a single stuck heartbeat can trip the threshold.
//!
//! Native-only: the input-driven half is made of native's drag state
//! and picker, and the step driver below commands a clock only IPC can
//! switch (`CLAUDE.md` "Dual-target status", "Virtual-clock
//! stepping"). The animation tick it runs is not — `animation_drain`
//! is the body the browser's frame runs too.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::clock::{heartbeat_times, AppClock, ClockMode, ClockRefusal, StepRequest, Stepped};
use crate::application::document::MindMapDocument;
use crate::application::ipc::injected::{InjectedQueue, InputEvent, Sample};
use crate::application::platform::window::PhysicalPosition;

use super::capture_flow::CaptureScene;
use super::input_context::InputHandlerContext;
use super::ipc_target::surface_of;
use super::throttled_interaction::{ColorPickerHoverInteraction, ThrottledDrag, ThrottledInteraction};
use super::{animation_drain, drain_frame, record_flow, DragState};

/// Which frame is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Beat {
    /// One iteration of the event loop. In real time, the whole frame;
    /// under a virtual clock time has not moved, so the input-driven
    /// half alone.
    Loop,
    /// A step's heartbeat, on the
    /// [`HEARTBEAT_MS`](crate::application::clock::HEARTBEAT_MS)
    /// cadence: the whole frame.
    Cadence,
    /// A running recording's frame boundary, between two cadence
    /// heartbeats of a step: the whole frame but the injected sample.
    Boundary,
}

impl Beat {
    /// Whether this frame runs the time-driven half.
    fn moves_time(self, clock: &AppClock) -> bool {
        match self {
            Beat::Loop => !clock.is_virtual(),
            Beat::Cadence | Beat::Boundary => true,
        }
    }

    /// Whether this frame lands one injected-input sample.
    fn lands_injected(self, clock: &AppClock) -> bool {
        match self {
            Beat::Loop => !clock.is_virtual(),
            Beat::Cadence => true,
            Beat::Boundary => false,
        }
    }
}

/// Whether the event loop must keep iterating without further input
/// (`InitState::needs_continuation`, and the first conditions of
/// `clock.wait until=settled`). Sources of continuation:
///
/// - **A throttled drag with pending state**, and **the picker hover
///   with pending state**: a deferred drain would otherwise be
///   stranded after the last cursor event.
/// - **Connection geometry dirty**: a camera change landed during a
///   node drag that suppressed the rebuild.
/// - **In real time only, an active animation and queued injected
///   input**: each needs the next heartbeat. Under a virtual clock the
///   heartbeats that advance them are the step's, so neither keeps the
///   loop awake: with time frozen there is nothing an iteration could
///   do for them. The animation term is
///   [`animation_drain::keeps_ticking`], which the browser's frame
///   reads for the same decision.
///
/// `None` / `Pending` / `Panning` / `SelectingRect` drag states are
/// event-driven only. The FPS overlay is deliberately not a source: a
/// diagnostic observes behavior and must not change it.
///
/// `geometry_dirty` is the renderer's connection-geometry flag, read by
/// the caller, so the predicate is pinned without a renderer.
pub(super) fn needs_continuation(
    drag_state: &DragState,
    picker_hover: &ColorPickerHoverInteraction,
    document: Option<&MindMapDocument>,
    geometry_dirty: bool,
    injected: &InjectedQueue,
    clock: &AppClock,
) -> bool {
    let drag_pending = matches!(
        drag_state,
        DragState::Throttled(d) if d.as_dyn().needs_continuation()
    );
    let input_driven = drag_pending || picker_hover.has_pending() || geometry_dirty;
    let injected_pending = !injected.is_empty() && clock.mode() == ClockMode::Real;
    input_driven || animation_drain::keeps_ticking(document, clock) || injected_pending
}

impl InputHandlerContext<'_> {
    /// Deliver one injected-input sample to the `on_*` entry point its
    /// winit event reaches, with the sample's modifiers held for that
    /// one event (`ipc::injected::Sample`). Returns whether the arm
    /// asked for a redraw. Called by the frame for a queued sample and
    /// by the IPC target for a burst.
    pub(super) fn apply_injected(&mut self, sample: Sample<super::super::ipc::injected::Surface>) -> bool {
        let reported = *self.modifiers;
        if let Some(held) = sample.modifiers {
            *self.modifiers = held;
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
        *self.modifiers = reported;
        redraw
    }

    /// The frame up to the recorder: one injected-input sample when the
    /// frame lands one — ahead of the drains, where a winit event lands,
    /// so a paced drag's moves are driven through the throttled drag
    /// between frames, as a hand's are (`format/ipc.md` §`input`) — then
    /// [`Self::drain_inputs`]. The recorder runs in
    /// [`Self::record_tick`], after whatever the caller orders between
    /// the two (the event loop runs the IPC drain there).
    pub(super) fn drain_frame(&mut self, beat: Beat) {
        if beat.lands_injected(self.clock) {
            let renderer = &*self.renderer;
            if let Some(sample) = self.injected.pop(|point| surface_of(renderer, point)) {
                // The redraw this sample owes is the frame's own: the
                // event loop's `was_continuing` holds it in real time,
                // and a step's caller repaints once at its end.
                let _ = self.apply_injected(sample);
            }
        }
        self.drain_inputs(beat.moves_time(self.clock));
    }

    /// The per-frame drains: drive the active throttled drag (if any)
    /// and the always-live picker-hover interaction through the
    /// unified [`ThrottledInteraction::drive`] shell, then the
    /// non-throttled drains (rect-select overlay, camera rebuild), and
    /// — when `ticks` — the animation tick. Does **not** render: the
    /// event loop decides whether to `request_redraw` from the
    /// post-drain [`needs_continuation`], and the render runs
    /// from the `WindowEvent::RedrawRequested` arm.
    ///
    /// Under a virtual clock the throttled drives drain every frame
    /// that has something pending. The throttle trades how often a
    /// drag's mutation runs for staying inside a real frame's budget;
    /// a virtual heartbeat has no real deadline to protect, so
    /// skipping a drain would buy nothing and would make what a
    /// stepped frame shows depend on how long the machine took to
    /// draw the last one. The throttle's measurements stay wall-clock
    /// either way.
    pub(super) fn drain_inputs(&mut self, ticks: bool) {
        use super::throttled_interaction::DrainContext;

        // Only the moving-node drag needs to suppress the camera
        // rebuild (it handles offset geometry itself each drain).
        // Snapshot this before the drive() borrow takes `&mut
        // self.drag_state`. Suppresses the camera-driven
        // geometry rebuild while a node is being moved (the
        // drag's own per-frame mutator already keeps the scene
        // current; the camera rebuild would interleave with
        // stale model-side state). `MovingSection` /
        // `SectionResize` / `NodeResize` deliberately don't
        // qualify — sections don't move the parent, and node
        // resize gestures' tree mutations are re-applied by
        // the next drain after any competing rebuild.
        let is_moving_node = self.drag_state.is_moving_node();

        // Suppress the animation tick during drags that mutate
        // the tree per-frame (`MovingNode`, `MovingSection`,
        // `SectionResize`, `NodeResize`). An animation tick on
        // the same frame routes through `sync_node_from_tree`,
        // which would observe the in-progress mid-drag state
        // and write it to the model + undo stack. Edge /
        // portal-label drags don't qualify — they touch the
        // document, not the tree.
        let is_drag_with_tree_mutation = matches!(
            *self.drag_state,
            DragState::Throttled(ref d)
                if matches!(
                    **d,
                    ThrottledDrag::MovingNode(_)
                        | ThrottledDrag::MovingSection(_)
                        | ThrottledDrag::SectionResize(_)
                        | ThrottledDrag::NodeResize(_)
                )
        );
        let throttled = !self.clock.is_virtual();

        if let DragState::Throttled(ref mut kind) = *self.drag_state {
            kind.as_dyn_mut().drive(DrainContext {
                document: &mut *self.document,
                mindmap_tree: &mut *self.mindmap_tree,
                app_scene: &mut *self.app_scene,
                renderer: &mut *self.renderer,
                scene_cache: &mut *self.scene_cache,
                color_picker_state: &mut *self.color_picker_state,
                interaction_mode: &*self.interaction_mode,
                throttled,
            });
        }

        if self.color_picker_state.is_open() {
            self.picker_hover.drive(DrainContext {
                document: &mut *self.document,
                mindmap_tree: &mut *self.mindmap_tree,
                app_scene: &mut *self.app_scene,
                renderer: &mut *self.renderer,
                scene_cache: &mut *self.scene_cache,
                color_picker_state: &mut *self.color_picker_state,
                interaction_mode: &*self.interaction_mode,
                throttled,
            });
        } else if self.picker_hover.has_pending() {
            // Picker closed while a throttle-deferred hover drain
            // was still queued. The drain body's closed-picker
            // branch (`color_picker_hover.rs:88-92`) is a no-op
            // rebuild that just clears both flags — going through
            // the throttle would still strand them for the throttle
            // window's worth of frames, during which
            // `needs_continuation` reads `has_pending=true` and
            // pins the loop in `ControlFlow::Poll`. Two boolean
            // writes, no GPU work; bypass the throttle and clear
            // immediately.
            self.picker_hover.clear_pending();
        }

        // Unconditional, and takes the whole drag state: this drain
        // holds the rubber band's on-screen state to the state that
        // authorizes it, so the frames where no rubber band is live
        // are exactly the frames it has to run on to notice that.
        drain_frame::drain_rect_select(
            self.drag_state,
            self.document,
            self.interaction_mode,
            self.mindmap_tree,
            self.renderer,
        );

        drain_frame::drain_camera_geometry_rebuild(
            is_moving_node,
            self.document,
            self.interaction_mode,
            self.app_scene,
            self.renderer,
            self.scene_cache,
        );

        // Animation pause/resume — when entering a tree-mutating
        // drag, capture the app clock; when exiting, shift each
        // active animation's `start_ms` forward by the drag
        // duration so the post-release tick resumes the
        // animation where it froze instead of snapping to its
        // `to` state. Without this, a multi-second drag would
        // leave an in-flight animation observing
        // `elapsed >= total` on the first post-release frame.
        let now = self.clock.now_whole_ms();
        match (is_drag_with_tree_mutation, *self.anim_pause_start_ms) {
            (true, None) => {
                *self.anim_pause_start_ms = Some(now);
            }
            (false, Some(pause_start)) => {
                let pause_duration = now.saturating_sub(pause_start);
                if let Some(doc) = self.document.as_mut() {
                    doc.shift_active_animations_start_ms(pause_duration);
                }
                *self.anim_pause_start_ms = None;
            }
            _ => {}
        }

        if ticks && !is_drag_with_tree_mutation {
            animation_drain::drain_animation_tick(
                now,
                self.document.as_mut(),
                self.interaction_mode,
                self.mindmap_tree,
                self.app_scene,
                self.renderer,
                self.scene_cache,
            );
        }
    }

    /// Take the active recording's next frame if one is due: a no-op
    /// when nothing is recording, and on a frame that does not move
    /// time (`app::record_flow`).
    pub(super) fn record_tick(&mut self, beat: Beat) {
        if self.recorder.active.is_none() || !beat.moves_time(self.clock) {
            return;
        }
        let now = self.clock.now_ms();
        let (scene, artifacts, recorder) = CaptureScene::of_context(self);
        record_flow::tick(scene, recorder, artifacts, now);
    }

    /// Advance a virtual clock by `request`, running the frame at every
    /// heartbeat and at every frame boundary of a running recording
    /// along the way ([`run_step`]). Synchronous: when this returns,
    /// the app is what it is at the step's end, and the caller owes one
    /// redraw to show it.
    pub(super) fn step(&mut self, request: StepRequest) -> Result<Stepped, ClockRefusal> {
        if !self.clock.is_virtual() {
            return Err(ClockRefusal::Real);
        }
        if !self.clock.begin_step() {
            return Err(ClockRefusal::Stepping);
        }
        let frames_run = run_step(self, request);
        self.clock.end_step();
        Ok(Stepped {
            now_ms: self.clock.now_ms(),
            frames_run,
        })
    }

    /// Switch the clock to `mode`. Refused mid-step and while a
    /// recording runs ([`AppClock::switch_mode`]). Returns whether the
    /// mode changed.
    pub(super) fn set_clock_mode(&mut self, mode: ClockMode) -> Result<bool, ClockRefusal> {
        self.clock.switch_mode(mode, self.recorder.active.is_some())
    }
}

/// What a step drives: a clock it stops at each instant the step
/// names, a recording whose boundaries it stops at too, and the frame
/// it runs there. The handler context is the app's; the tests drive a
/// real recording on a real clock through the same [`run_step`], so the
/// schedule they pin is the one the app runs.
pub(super) trait Steps {
    /// The clock's reading.
    fn now_ms(&self) -> f64;
    /// When the running recording's next frame is due, on the same
    /// clock; `None` with nothing recording or the recording full.
    fn next_frame_due(&self) -> Option<f64>;
    /// Stop the clock at `at_ms` and run the frame `beat` names there.
    fn beat(&mut self, beat: Beat, at_ms: f64);
}

/// One step's loop (see the module doc): a [`Beat::Cadence`] heartbeat
/// at each of [`heartbeat_times`], and before each, a
/// [`Beat::Boundary`] tick at every frame boundary of a running
/// recording that falls strictly between it and the last instant run —
/// one on a heartbeat is the heartbeat's to take. Returns the cadence
/// heartbeats run.
pub(super) fn run_step(app: &mut impl Steps, request: StepRequest) -> u64 {
    let start = app.now_ms();
    let mut frames_run = 0;
    for at in heartbeat_times(start, request) {
        // Ends: each tick stops the clock on `due`, so a boundary the
        // recorder did not move past fails `due > now` next time round.
        while let Some(due) = app
            .next_frame_due()
            .filter(|due| *due > app.now_ms() && *due < at)
        {
            app.beat(Beat::Boundary, due);
        }
        app.beat(Beat::Cadence, at);
        frames_run += 1;
    }
    frames_run
}

impl Steps for InputHandlerContext<'_> {
    fn now_ms(&self) -> f64 {
        self.clock.now_ms()
    }

    fn next_frame_due(&self) -> Option<f64> {
        self.recorder.active.as_ref()?.next_due_ms()
    }

    fn beat(&mut self, beat: Beat, at_ms: f64) {
        self.clock.advance_to(at_ms);
        // Once per heartbeat: a long step makes progress, so only one
        // stuck heartbeat can trip the threshold.
        self.watchdog.tick();
        self.drain_frame(beat);
        self.record_tick(beat);
    }
}

/// What a step does to a document with no window: the animation tick
/// at each instant, and the tree rebuilt from the model after a tick
/// that advanced, as `drain_animation_tick`'s `rebuild_all` rebuilds it.
/// The time-driven half of a frame minus the renderer
/// (TEST_CONVENTIONS §T8), driven by the production [`run_step`]: what
/// the heartbeat-level determinism tests here and the scene-dump one
/// in `ipc::commands::scene` step.
#[cfg(test)]
pub(crate) struct SteppedDocument<'a> {
    pub(crate) clock: &'a mut AppClock,
    pub(crate) doc: &'a mut MindMapDocument,
    pub(crate) tree: &'a mut baumhard::mindmap::tree_builder::MindMapTree,
}

#[cfg(test)]
impl Steps for SteppedDocument<'_> {
    fn now_ms(&self) -> f64 {
        self.clock.now_ms()
    }

    fn next_frame_due(&self) -> Option<f64> {
        None
    }

    fn beat(&mut self, _: Beat, at_ms: f64) {
        self.clock.advance_to(at_ms);
        if self
            .doc
            .tick_animations(self.clock.now_whole_ms(), Some(&mut *self.tree))
        {
            *self.tree = self.doc.build_tree();
        }
    }
}

/// [`run_step`] over a [`SteppedDocument`], for a test outside this
/// module.
#[cfg(test)]
pub(crate) fn step_document(stepped: &mut SteppedDocument<'_>, request: StepRequest) -> u64 {
    run_step(stepped, request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::capture::geometry::CameraInfo;
    use crate::application::capture::plan::CaptureRequest;
    use crate::application::capture::record::{Pacing, RecordConfig, Recording, Step};
    use crate::application::document::tests_common::{
        first_testament_node_id, load_test_doc, TestNudgeMutation,
    };
    use crate::application::renderer::CapturedPixels;
    use baumhard::mindmap::animation::{AnimationTiming, Easing};
    use baumhard::mindmap::custom_mutation::TargetScope;

    fn virtual_clock() -> AppClock {
        let mut clock = AppClock::default();
        clock.set_mode(ClockMode::Virtual);
        clock
    }

    /// A real recording on a real virtual clock, stepped by the
    /// production loop: each instant the loop stops at is logged, and
    /// a frame the recording owes there is taken, as `record_tick`
    /// takes one.
    struct SteppedRecording {
        clock: AppClock,
        recording: Recording,
        /// The clock's reading when the recording began.
        start: f64,
        /// Each instant the loop stopped at, from the start.
        beats: Vec<(Beat, f64)>,
    }

    impl SteppedRecording {
        fn at_fps(fps: u32) -> Self {
            let clock = virtual_clock();
            let config = RecordConfig {
                fps,
                max_frames: 3600,
                width: 2,
                height: 1,
                scale: 1.0,
                request: CaptureRequest::default(),
                pacing: Pacing::Stepped,
            };
            let start = clock.now_ms();
            let mut stepped = SteppedRecording {
                recording: Recording::begin(config, "/unused".into(), start, None),
                clock,
                start,
                beats: Vec::new(),
            };
            // `record_flow::start` takes the first frame before it replies.
            stepped.take_due_frame();
            stepped
        }

        fn take_due_frame(&mut self) {
            let now = self.clock.now_ms();
            if let Step::Capture { t_ms, .. } = self.recording.step(now) {
                // What the frame shows is what the animation tick read:
                // the whole-millisecond reading, which `t_ms` names.
                assert_eq!(now, self.clock.now_whole_ms() as f64, "a whole-ms reading");
                assert_eq!(t_ms, now - self.start, "stamped with the reading it shows");
                let pixels = CapturedPixels {
                    width: 2,
                    height: 1,
                    rgba: vec![0; 8],
                };
                let camera = CameraInfo {
                    center_x: 0.0,
                    center_y: 0.0,
                    zoom: 1.0,
                };
                self.recording.push_frame(t_ms, 0, camera, pixels);
            }
        }

        fn frame_times(&self) -> Vec<f64> {
            self.recording.frames().iter().map(|f| f.t_ms).collect()
        }
    }

    impl Steps for SteppedRecording {
        fn now_ms(&self) -> f64 {
            self.clock.now_ms()
        }
        fn next_frame_due(&self) -> Option<f64> {
            self.recording.next_due_ms()
        }
        fn beat(&mut self, beat: Beat, at_ms: f64) {
            self.clock.advance_to(at_ms);
            self.beats.push((beat, at_ms - self.start));
            self.take_due_frame();
        }
    }

    /// **`clock.step ms=1000` at `fps=30` yields exactly 30 frames**
    /// (`format/ipc.md` §`capture.record_start`, the IPC-08 ↔ IPC-09
    /// interlock), each stamped with its `1000/fps` boundary rounded up
    /// to the whole millisecond the clock stopped on, with no gap — and
    /// the step still runs exactly `ceil(1000 / 16)` cadence heartbeats,
    /// the boundary ticks beside them.
    #[test]
    fn test_a_stepped_second_at_thirty_fps_is_thirty_frames_on_their_boundaries() {
        let mut stepped = SteppedRecording::at_fps(30);
        let frames_run = run_step(&mut stepped, StepRequest::Ms(1000));
        assert_eq!(frames_run, 63);
        let times = stepped.frame_times();
        assert_eq!(times.len(), 31, "the start frame and thirty stepped ones");
        for (index, t_ms) in times.iter().enumerate() {
            let boundary = (index as u64 * 1000).div_ceil(30) as f64;
            assert_eq!(*t_ms, boundary, "frame {index}");
        }
        assert_eq!(
            stepped.recording.dropped_frames(),
            0,
            "no boundary is stepped over"
        );
        let cadence = stepped
            .beats
            .iter()
            .filter(|(beat, _)| *beat == Beat::Cadence)
            .count();
        assert_eq!(
            cadence, 63,
            "boundary ticks are beside the cadence, not instead of it"
        );
    }

    /// The same frames whether the second is stepped in one go or in
    /// sixteen-millisecond heartbeats one at a time: the schedule is a
    /// function of virtual time, not of how the agent chunks it.
    #[test]
    fn test_a_stepped_recording_does_not_depend_on_how_the_steps_are_chunked() {
        let mut whole = SteppedRecording::at_fps(24);
        run_step(&mut whole, StepRequest::Ms(2000));
        let mut chunked = SteppedRecording::at_fps(24);
        for _ in 0..125 {
            run_step(&mut chunked, StepRequest::Frames(1));
        }
        assert_eq!(whole.frame_times(), chunked.frame_times());
        assert_eq!(whole.frame_times().len(), 49);
    }

    /// A boundary that falls on a cadence heartbeat is that
    /// heartbeat's: at 50 fps every fourth boundary (80 ms) is a
    /// multiple of 16, and gets no tick of its own.
    #[test]
    fn test_a_boundary_on_a_heartbeat_is_the_heartbeats_to_take() {
        let mut stepped = SteppedRecording::at_fps(50);
        run_step(&mut stepped, StepRequest::Ms(80));
        let boundary_ticks: Vec<f64> = stepped
            .beats
            .iter()
            .filter(|(beat, _)| *beat == Beat::Boundary)
            .map(|(_, at)| *at)
            .collect();
        assert_eq!(boundary_ticks.len(), 3, "20, 40 and 60 ms; 80 is a heartbeat's");
        assert_eq!(stepped.frame_times(), [0.0, 20.0, 40.0, 60.0, 80.0]);
    }

    fn slide(duration_ms: u32) -> baumhard::mindmap::custom_mutation::CustomMutation {
        TestNudgeMutation::new("slide", TargetScope::SelfOnly)
            .magnitude(100.0)
            .timing(AnimationTiming {
                duration_ms,
                delay_ms: 0,
                easing: Easing::Linear,
                then: None,
            })
            .build()
    }

    /// **The acceptance criterion, at the heartbeat.** A 3 s animation
    /// stepped `ms=500` six times stands at exactly 1/6, 2/6 … of its
    /// travel after each step, and the sixth step lands it at `from +
    /// delta`, byte for byte the node an instant apply of the same
    /// mutation leaves (#3's invariant, PR #58) — undo entry included.
    #[test]
    fn test_six_half_second_steps_land_a_three_second_animation_as_an_instant_apply_would() {
        let mut doc = load_test_doc();
        let node = first_testament_node_id(&doc);
        let from_x = doc.mindmap.nodes[&node].position.x;
        let mut tree = doc.build_tree();
        let mut clock = virtual_clock();
        doc.start_animation(&slide(3000), &node, clock.now_whole_ms());
        let mut stepped = SteppedDocument {
            clock: &mut clock,
            doc: &mut doc,
            tree: &mut tree,
        };
        for step in 1..=6 {
            assert_eq!(step_document(&mut stepped, StepRequest::Ms(500)), 32);
            let x = stepped.doc.mindmap.nodes[&node].position.x;
            let expected = from_x + 100.0 * f64::from(step) / 6.0;
            assert!(
                (x - expected).abs() < 1e-3,
                "after step {step} the node is at {x}, not {expected}"
            );
        }
        assert!(!doc.has_active_animations(), "the sixth step lands on the end");

        let mut instant = load_test_doc();
        let mut instant_tree = instant.build_tree();
        let mut cm = slide(3000);
        cm.timing = None;
        instant.apply_custom_mutation(&cm, &node, Some(&mut instant_tree));
        assert_eq!(
            serde_json::to_string(&doc.mindmap.nodes[&node]).expect("serializes"),
            serde_json::to_string(&instant.mindmap.nodes[&node]).expect("serializes"),
            "the stepped commit is the instant apply"
        );
        assert_eq!(doc.undo_stack.len(), instant.undo_stack.len());
    }

    /// The tick lands where the clock says, whatever the clock read
    /// when virtual time began: two runs engaged at different readings
    /// leave the document byte-identical.
    #[test]
    fn test_a_stepped_animation_does_not_depend_on_when_virtual_time_began() {
        let run = |head_start_ms: f64| {
            let mut doc = load_test_doc();
            let node = first_testament_node_id(&doc);
            let mut tree = doc.build_tree();
            let mut clock = virtual_clock();
            let start = clock.now_ms() + head_start_ms;
            clock.advance_to(start);
            doc.start_animation(&slide(700), &node, clock.now_whole_ms());
            let mut stepped = SteppedDocument {
                clock: &mut clock,
                doc: &mut doc,
                tree: &mut tree,
            };
            step_document(&mut stepped, StepRequest::Ms(333));
            serde_json::to_string(&doc.mindmap).expect("serializes")
        };
        assert_eq!(run(0.0), run(12_345.0));
    }

    /// Double-click timing on the virtual axis: a second press exactly
    /// one step short of the window pairs with the first, one landing on
    /// the window does not — the edge an agent could not hold still
    /// before there was a clock to hold.
    #[test]
    fn test_a_double_click_on_virtual_time_is_decided_at_exactly_the_threshold() {
        use super::super::{is_double_click, ClickHit, LastClick, DOUBLE_CLICK_MS};
        for (gap_ms, pairs) in [(DOUBLE_CLICK_MS - 1.0, true), (DOUBLE_CLICK_MS, false)] {
            let mut clock = virtual_clock();
            let first = LastClick {
                time: clock.now_ms(),
                screen_pos: (10.0, 10.0),
                hit: ClickHit::Empty,
            };
            clock.advance_to(clock.now_ms() + gap_ms);
            assert_eq!(
                is_double_click(&first, clock.now_ms(), (10.0, 10.0), &ClickHit::Empty),
                pairs,
                "a second press {gap_ms} virtual ms after the first"
            );
        }
    }

    /// Under a virtual clock what time drives keeps the loop awake no
    /// more: an animation frozen between steps and samples waiting for
    /// a step are owed to future instants only a step reaches, and a
    /// loop that polled for them would spin with nothing to do. In real
    /// time both are continuation sources, as they always were; and
    /// what input drives keeps the loop awake under either clock.
    #[test]
    fn test_time_driven_work_keeps_only_a_real_clock_awake() {
        use crate::application::ipc::injected::{Pacing as InputPacing, Sample};
        use crate::application::ipc::space::{Point, Space};

        let mut animating = load_test_doc();
        let node = first_testament_node_id(&animating);
        animating.start_animation(&slide(500), &node, 0);
        let mut queued = InjectedQueue::default();
        let sample = |x| Sample {
            event: InputEvent::CursorMoved(Point {
                x,
                y: 0.0,
                space: Space::Surface,
            }),
            modifiers: None,
        };
        queued
            .admit(vec![sample(1.0), sample(2.0)], InputPacing::PerFrame)
            .expect("room");
        let empty = InjectedQueue::default();
        let hover = ColorPickerHoverInteraction::new();
        let real = AppClock::default();
        let frozen = virtual_clock();
        let idle = load_test_doc();

        for (document, injected, geometry_dirty, on_real, on_virtual, case) in [
            (&animating, &empty, false, true, false, "an animation"),
            (&idle, &queued, false, true, false, "queued input"),
            (&idle, &empty, true, true, true, "a camera rebuild"),
            (&idle, &empty, false, false, false, "nothing"),
        ] {
            for (clock, expected) in [(&real, on_real), (&frozen, on_virtual)] {
                assert_eq!(
                    needs_continuation(
                        &DragState::None,
                        &hover,
                        Some(document),
                        geometry_dirty,
                        injected,
                        clock
                    ),
                    expected,
                    "{case} on a {} clock",
                    clock.mode().word()
                );
            }
        }
    }
}
