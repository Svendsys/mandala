// SPDX-License-Identifier: MPL-2.0

//! The raw-input vocabulary the `input` family injects and the
//! queue that paces it (`format/ipc.md` §`input`).
//!
//! An [`InputEvent`] is one winit event's payload — the values the
//! native handlers already take as plain arguments through the
//! `platform::input` aliases — so the app delivers it to **the same
//! entry point** the winit event reaches, and nothing below that
//! point can tell the two apart. Positions are surface pixels: a
//! canvas position is converted once, when the command executes,
//! through the camera as it stands then. That is where a hand on a
//! mouse points too — at the screen, not at the world behind it —
//! and it keeps a pan-drag from chasing its own camera.
//!
//! **Pacing.** A drag's fidelity depends on its moves landing in
//! different frames: `drain_inputs` drives the throttled drag between
//! them, and a press, a dozen moves and a release run back to back in
//! one drain would skip `ThrottledInteraction` entirely and test a
//! path no human hits. So a [`Pacing::PerFrame`] command enqueues its
//! samples into the [`InjectedQueue`], and the heartbeat applies one
//! per frame, ahead of `drain_inputs`, the place a winit event lands.
//! A [`Pacing::Burst`] command — a move, a click, a key tap, a typed
//! string — applies now, in one ordered burst, as winit delivers
//! several events in one iteration; when samples are still queued it
//! waits behind them instead, so injected input always lands in the
//! order it was sent.

use std::collections::VecDeque;

use crate::application::platform::input::{
    ElementState, Key, Modifiers, MouseButton, MouseScrollDelta, TouchPhase,
};

/// The queue's capacity, in samples (`format/ipc.md` §`input`, the
/// pacing model). A command whose samples would not fit is refused
/// whole, `overloaded`, and enqueues nothing.
pub(crate) const INJECTED_QUEUE_CAP: usize = 1024;

/// One raw input event, as winit delivers it to the native handlers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InputEvent {
    /// `WindowEvent::CursorMoved`: the pointer is now at this surface
    /// position, in physical pixels.
    CursorMoved { x: f64, y: f64 },
    /// `WindowEvent::MouseInput`.
    MouseButton {
        button: MouseButton,
        state: ElementState,
    },
    /// `WindowEvent::MouseWheel`.
    Wheel(MouseScrollDelta),
    /// `WindowEvent::KeyboardInput`'s logical key and state.
    Key { key: Key, state: ElementState },
    /// `WindowEvent::Touch`, at a surface position in physical pixels.
    Touch {
        phase: TouchPhase,
        pointer_id: u64,
        x: f64,
        y: f64,
    },
}

/// One event and the modifiers held while it lands.
///
/// `None` leaves the modifier state the OS last reported in force.
/// `Some` holds those modifiers for this one event and puts the
/// reported state back afterwards — a drag carries them on every
/// sample, so a Shift+drag holds Shift at the threshold cross, where
/// the rubber band reads it, while the user's own keystrokes between
/// two frames still see their own modifiers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sample {
    pub(crate) event: InputEvent,
    pub(crate) modifiers: Option<Modifiers>,
}

impl Sample {
    pub(crate) fn new(event: InputEvent, modifiers: Option<Modifiers>) -> Self {
        Sample { event, modifiers }
    }
}

/// How a command's samples reach the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pacing {
    /// Apply now, in order, when nothing is queued; otherwise wait
    /// behind what is.
    Burst,
    /// One sample per heartbeat, always through the queue.
    PerFrame,
}

/// What [`InjectedQueue::admit`] decided.
#[derive(Debug, PartialEq)]
pub(crate) enum Delivery {
    /// The caller applies these now, in order.
    Now(Vec<Sample>),
    /// The samples are queued; the heartbeat applies them.
    Queued,
}

/// The queue would overflow: nothing was enqueued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Overloaded {
    /// Samples already queued.
    pub(crate) pending: usize,
}

/// The injected-input queue: samples waiting for their heartbeat.
/// Lives on the app's state beside the drag state it feeds, so the
/// heartbeat and a command see the one queue.
#[derive(Debug, Default)]
pub(crate) struct InjectedQueue {
    samples: VecDeque<Sample>,
}

impl InjectedQueue {
    /// Samples still waiting.
    pub(crate) fn len(&self) -> usize {
        self.samples.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Route one command's samples: hand them back to apply now, or
    /// queue all of them, or refuse all of them. A refusal changes
    /// nothing, so a client that backs off and resends loses no
    /// sample and doubles none.
    pub(crate) fn admit(&mut self, samples: Vec<Sample>, pacing: Pacing) -> Result<Delivery, Overloaded> {
        if pacing == Pacing::Burst && self.samples.is_empty() {
            return Ok(Delivery::Now(samples));
        }
        if self.samples.len() + samples.len() > INJECTED_QUEUE_CAP {
            return Err(Overloaded {
                pending: self.samples.len(),
            });
        }
        self.samples.extend(samples);
        Ok(Delivery::Queued)
    }

    /// The next sample for this heartbeat.
    pub(crate) fn pop(&mut self) -> Option<Sample> {
        self.samples.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved(x: f64) -> Sample {
        Sample::new(InputEvent::CursorMoved { x, y: 0.0 }, None)
    }

    #[test]
    fn test_a_burst_applies_now_when_nothing_is_queued() {
        let mut queue = InjectedQueue::default();
        let burst = vec![moved(1.0), moved(2.0)];
        assert_eq!(
            queue.admit(burst.clone(), Pacing::Burst),
            Ok(Delivery::Now(burst))
        );
        assert!(queue.is_empty(), "a burst applied now leaves nothing behind");
    }

    #[test]
    fn test_per_frame_samples_queue_and_pop_in_send_order() {
        let mut queue = InjectedQueue::default();
        assert_eq!(
            queue.admit(vec![moved(1.0), moved(2.0)], Pacing::PerFrame),
            Ok(Delivery::Queued)
        );
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.pop(), Some(moved(1.0)));
        assert_eq!(queue.pop(), Some(moved(2.0)));
        assert_eq!(queue.pop(), None);
    }

    /// Order is kept across commands: a burst sent while a gesture is
    /// still queued waits behind it rather than overtaking it.
    #[test]
    fn test_a_burst_waits_behind_queued_samples() {
        let mut queue = InjectedQueue::default();
        queue.admit(vec![moved(1.0)], Pacing::PerFrame).expect("fits");
        assert_eq!(queue.admit(vec![moved(2.0)], Pacing::Burst), Ok(Delivery::Queued));
        assert_eq!(queue.pop(), Some(moved(1.0)));
        assert_eq!(queue.pop(), Some(moved(2.0)));
    }

    /// Overflow refuses the whole command and enqueues none of it, so
    /// a resend after backing off neither drops nor doubles a sample.
    #[test]
    fn test_overflow_refuses_the_whole_command_and_keeps_the_queue() {
        let mut queue = InjectedQueue::default();
        let fill: Vec<Sample> = (0..INJECTED_QUEUE_CAP - 1).map(|i| moved(i as f64)).collect();
        queue
            .admit(fill, Pacing::PerFrame)
            .expect("one under the cap fits");
        assert_eq!(
            queue.admit(vec![moved(-1.0), moved(-2.0)], Pacing::PerFrame),
            Err(Overloaded {
                pending: INJECTED_QUEUE_CAP - 1
            })
        );
        assert_eq!(
            queue.admit(vec![moved(-1.0), moved(-2.0)], Pacing::Burst),
            Err(Overloaded {
                pending: INJECTED_QUEUE_CAP - 1
            }),
            "a burst behind a full queue is refused the same way"
        );
        assert_eq!(queue.len(), INJECTED_QUEUE_CAP - 1, "nothing was enqueued");
        assert_eq!(
            queue.admit(vec![moved(-1.0)], Pacing::PerFrame),
            Ok(Delivery::Queued)
        );
        assert_eq!(queue.len(), INJECTED_QUEUE_CAP, "exactly the cap fits");
    }

    /// **No handler knows injected input** (issue #66's acceptance
    /// criterion: no special-casing, grep-clean). Every file the input
    /// handlers live in, and the `InitState::on_*` entry points the
    /// injected samples are delivered to, is read whole, and none of
    /// them names the injection vocabulary. A branch on "was this
    /// injected" would have to name it, so an injected event and a
    /// winit one cannot be told apart below the entry point.
    #[test]
    fn test_no_input_handler_names_injected_input() {
        use baumhard::util::doc_fixtures::repo_path;

        const VOCABULARY: [&str; 4] = ["injected", "Injected", "InputEvent", "ipc::"];
        let mut files: Vec<std::path::PathBuf> = [
            "event_mouse_click.rs",
            "event_cursor_moved.rs",
            "event_keyboard.rs",
            "click.rs",
            "touch_gesture.rs",
            "modal_editor.rs",
        ]
        .iter()
        .map(|f| repo_path(&format!("src/application/app/{f}")))
        .collect();
        for dir in [
            "throttled_interaction",
            "dispatch",
            "dispatch/cross_dispatch",
            "text_edit",
            "single_line_edit",
            "console_input",
            "color_picker_flow",
        ] {
            let dir = repo_path(&format!("src/application/app/{dir}"));
            for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
                let path = entry.expect("dir entry").path();
                if path.extension().is_some_and(|ext| ext == "rs") {
                    files.push(path);
                }
            }
        }
        let mut sources: Vec<(String, String)> = files
            .iter()
            .map(|path| {
                let text =
                    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                (path.display().to_string(), text)
            })
            .collect();
        let run_native =
            std::fs::read_to_string(repo_path("src/application/app/run_native.rs")).expect("readable");
        let start = run_native
            .find("    pub(super) fn on_mouse_input(")
            .expect("run_native.rs has the on_* entry points");
        let end = run_native[start..]
            .find("    /// Per-frame drain")
            .expect("the entry points end where drain_inputs' doc starts");
        sources.push((
            "run_native.rs `InitState::on_*`".into(),
            run_native[start..start + end].to_string(),
        ));
        for (name, text) in sources {
            for word in VOCABULARY {
                assert!(
                    !text.contains(word),
                    "{name} names `{word}`: an input handler must not know whether its event was injected"
                );
            }
        }
    }

    /// The cap is the one the protocol document pins.
    #[test]
    fn test_the_cap_is_the_documented_one() {
        use baumhard::util::doc_fixtures::{repo_path, section_text};
        let section = section_text(
            &repo_path("format/ipc.md"),
            "## `input` — synthetic raw input (IPC-06)",
        );
        assert!(
            section.contains(&format!("cap **{INJECTED_QUEUE_CAP}**")),
            "format/ipc.md §`input` must pin the injected-input queue's cap at {INJECTED_QUEUE_CAP}"
        );
    }
}
