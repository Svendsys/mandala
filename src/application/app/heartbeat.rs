// SPDX-License-Identifier: MPL-2.0

//! The heartbeat: what one frame of the app does, and the clock step
//! that runs it on virtual time (`format/ipc.md` §`clock`;
//! `work_plans/LLM_IPC.md` §D9).
//!
//! **A frame has an input-driven half and a time-driven half.** The
//! input-driven half — the rubber band — flushes what input already
//! did, and needs no clock. The time-driven half — one sample of the IPC
//! injected-input queue and a recording's frame — is a function of
//! *when* the frame runs. In real time the two always run together, once
//! per event-loop iteration. Under a virtual clock, time moves only when
//! [`InputHandlerContext::step`] moves it: the loop's own iterations run
//! the input-driven half alone, and the time-driven half runs inside the
//! step, at the virtual instants the step names. That split is what
//! makes a stepped run a pure function of its command sequence.
//!
//! **What a step runs.** A heartbeat every
//! [`HEARTBEAT_MS`](crate::application::clock::HEARTBEAT_MS) virtual
//! milliseconds, the last on the step's end
//! ([`heartbeat_times`](crate::application::clock::heartbeat_times)), each
//! the whole frame — one injected sample, the drains, the recorder.
//! Between two of them, a running recording's frame boundary gets a tick
//! of its own ([`Beat::Boundary`]): the clock stops on the boundary
//! exactly, the frame drains and the recorder takes its frame, so a
//! stepped recording's frames are its `1000/fps` instants, each rounded
//! up to a whole millisecond, and nothing else. No injected sample lands
//! on a boundary tick, so recording a gesture never changes when the
//! gesture's samples land. Every heartbeat and every boundary tick ticks
//! the freeze watchdog, which is what makes a long step safe by
//! construction: a step makes progress, so only a single stuck heartbeat
//! can trip the threshold.
//!
//! The animations that ran on this frame — the presentation layer and
//! animated edits — return with V1-D1 (#250), which owns the per-frame
//! tick, and V1-D2 (#251).
//!
//! Native-only: the input-driven half is made of native's drag state.
//! The step's schedule is not — [`run_step`] and the [`Steps`] trait it
//! drives live in `app::step`, and the browser's `run_wasm::step`
//! implements the trait over its own frame.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::clock::{AppClock, ClockMode, ClockRefusal, StepRequest, Stepped};
use crate::application::ipc::injected::{InjectedQueue, InputEvent, Sample};
use crate::application::platform::window::PhysicalPosition;

use super::capture_flow::CaptureScene;
use super::input_context::InputHandlerContext;
use super::ipc_scene::surface_of;
use super::step::{run_step, Beat, Steps};
use super::{drain_frame, record_flow};

/// Whether the loop owes another iteration without further input: a
/// queued injected sample, under a real clock. Under a virtual clock a
/// queued sample is owed to a future instant only a step reaches, and a
/// loop that polled for it would spin with nothing to do.
pub(super) fn needs_continuation(injected: &InjectedQueue, clock: &AppClock) -> bool {
    !injected.is_empty() && clock.mode() == ClockMode::Real
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
    /// frame lands one — ahead of the drains, where a winit event lands —
    /// then [`Self::drain_inputs`]. The recorder runs in
    /// [`Self::record_tick`], after whatever the caller orders between the
    /// two (the event loop runs the IPC drain there). Returns whether the
    /// view changed.
    pub(super) fn drain_frame(&mut self, beat: Beat) -> bool {
        if beat.lands_injected(self.clock) {
            let renderer = &*self.renderer;
            if let Some(sample) = self.injected.pop(|point| surface_of(renderer, point)) {
                let _ = self.apply_injected(sample);
            }
        }
        self.drain_inputs()
    }

    /// The per-frame drains: the rubber band (`drain_frame`), the view's
    /// following of the mode (`view::MapView::follow_mode`), then the
    /// view's sync — the one pipeline (`view::MapView::sync`) — so a
    /// change any path made reaches the layers this frame even if the
    /// path did not sync it itself; a sync with nothing to do does
    /// nothing. Does **not** render: the event loop decides whether to
    /// `request_redraw` from what this returns and the post-drain
    /// [`needs_continuation`], and the render runs from the
    /// `WindowEvent::RedrawRequested` arm. Returns whether the view
    /// changed.
    pub(super) fn drain_inputs(&mut self) -> bool {
        drain_frame::drain_rect_select(self.drag_state, self.document, self.view, self.renderer);
        match (self.document.as_mut(), self.view.as_mut()) {
            (Some(doc), Some(view)) => {
                view.follow_mode(self.interaction_mode);
                view.sync(doc, self.renderer)
            }
            _ => false,
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

impl Steps for InputHandlerContext<'_> {
    fn now_ms(&self) -> f64 {
        self.clock.now_ms()
    }

    fn next_frame_due(&self) -> Option<f64> {
        self.recorder.active.as_ref()?.next_due_ms()
    }

    fn beat(&mut self, beat: Beat, at_ms: f64) {
        self.clock.advance_to(at_ms);
        self.watchdog.tick();
        let _ = self.drain_frame(beat);
        self.record_tick(beat);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::capture::geometry::CameraInfo;
    use crate::application::capture::plan::CaptureRequest;
    use crate::application::capture::record::{Pacing, RecordConfig, Recording, Step};
    use crate::application::renderer::CapturedPixels;

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
                // The frame shows the whole-millisecond reading, which
                // `t_ms` names.
                assert_eq!(now, now.round(), "a whole-ms reading");
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
    /// more: samples waiting for a step are owed to future instants only
    /// a step reaches, and a loop that polled for them would spin with
    /// nothing to do. In real time they keep it awake, and an empty queue
    /// keeps it awake under neither.
    #[test]
    fn test_queued_input_keeps_only_a_real_clock_awake() {
        use crate::application::ipc::injected::{Pacing as InputPacing, Sample};
        use crate::application::ipc::space::{Point, Space};

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
        let real = AppClock::default();
        let frozen = virtual_clock();
        assert!(needs_continuation(&queued, &real), "queued input on a real clock");
        assert!(
            !needs_continuation(&queued, &frozen),
            "queued input on a virtual clock"
        );
        assert!(!needs_continuation(&empty, &real), "nothing on a real clock");
        assert!(!needs_continuation(&empty, &frozen), "nothing on a virtual clock");
    }
}
