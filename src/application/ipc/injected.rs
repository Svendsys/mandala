// SPDX-License-Identifier: MPL-2.0

//! The raw-input vocabulary the `input` family injects and the
//! queue that paces it (`format/ipc.md` §`input`).
//!
//! An [`InputEvent`] is one winit event's payload — the values the
//! native handlers already take as plain arguments through the
//! `platform::input` aliases — so the app delivers it to **the same
//! entry point** the winit event reaches, and nothing below that
//! point can tell the two apart.
//!
//! **Positions** reach the handlers as surface pixels. A command
//! builds its events with [`Point`]s in either space, and they are
//! resolved once per command, through the camera as it stands when
//! the command's first event is applied. That is where a hand on a
//! mouse points — at the screen, not at the world behind it — so a
//! drag keeps one camera for all its frames and a pan-drag does not
//! chase its own camera. And a command that waited behind a
//! camera-moving gesture aims through the camera that gesture left,
//! not the one it found when it was sent.
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

use super::space::Point;

/// The queue's capacity, in samples (`format/ipc.md` §`input`, the
/// pacing model). A command whose samples would not fit is refused
/// whole, `overloaded`, and enqueues nothing.
pub(crate) const INJECTED_QUEUE_CAP: usize = 1024;

/// A resolved position: surface pixels, physical, as winit reports a
/// pointer.
pub(crate) type Surface = (f64, f64);

/// One raw input event, as winit delivers it to the native handlers.
/// `P` is where it happens: a [`Point`] as a command builds it, a
/// [`Surface`] position once resolved for the handler.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InputEvent<P = Surface> {
    /// `WindowEvent::CursorMoved`: the pointer is now here.
    CursorMoved(P),
    /// `WindowEvent::MouseInput`.
    MouseButton {
        button: MouseButton,
        state: ElementState,
    },
    /// `WindowEvent::MouseWheel`.
    Wheel(MouseScrollDelta),
    /// `WindowEvent::KeyboardInput`'s logical key and state.
    Key { key: Key, state: ElementState },
    /// `WindowEvent::Touch`: one finger, here.
    Touch {
        phase: TouchPhase,
        pointer_id: u64,
        at: P,
    },
}

impl<P> InputEvent<P> {
    /// The same event with its position, if it has one, mapped.
    fn resolve<Q>(self, resolve: &mut impl FnMut(P) -> Q) -> InputEvent<Q> {
        match self {
            InputEvent::CursorMoved(at) => InputEvent::CursorMoved(resolve(at)),
            InputEvent::MouseButton { button, state } => InputEvent::MouseButton { button, state },
            InputEvent::Wheel(delta) => InputEvent::Wheel(delta),
            InputEvent::Key { key, state } => InputEvent::Key { key, state },
            InputEvent::Touch {
                phase,
                pointer_id,
                at,
            } => InputEvent::Touch {
                phase,
                pointer_id,
                at: resolve(at),
            },
        }
    }
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
pub(crate) struct Sample<P = Point> {
    pub(crate) event: InputEvent<P>,
    pub(crate) modifiers: Option<Modifiers>,
}

impl<P> Sample<P> {
    pub(crate) fn new(event: InputEvent<P>, modifiers: Option<Modifiers>) -> Self {
        Sample { event, modifiers }
    }
}

/// Resolve one command's samples through `resolve`, called with the
/// camera of the moment the command starts applying.
pub(crate) fn resolve_command(
    samples: impl IntoIterator<Item = Sample>,
    mut resolve: impl FnMut(Point) -> Surface,
) -> impl Iterator<Item = Sample<Surface>> {
    samples.into_iter().map(move |sample| Sample {
        event: sample.event.resolve(&mut resolve),
        modifiers: sample.modifiers,
    })
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
    /// The caller resolves these and applies them now, in order.
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
///
/// Kept per command, so each command's positions resolve together
/// when its first sample is applied (the module doc's rule).
#[derive(Debug, Default)]
pub(crate) struct InjectedQueue {
    /// The rest of the command now landing, already resolved.
    landing: VecDeque<Sample<Surface>>,
    /// Commands not started yet, positions unresolved, oldest first.
    waiting: VecDeque<Vec<Sample>>,
    /// Samples across both.
    len: usize,
}

impl InjectedQueue {
    /// Samples still waiting.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Route one command's samples: hand them back to apply now, or
    /// queue all of them, or refuse all of them. A refusal changes
    /// nothing, so a client that backs off and resends loses no
    /// sample and doubles none.
    pub(crate) fn admit(&mut self, samples: Vec<Sample>, pacing: Pacing) -> Result<Delivery, Overloaded> {
        if pacing == Pacing::Burst && self.is_empty() {
            return Ok(Delivery::Now(samples));
        }
        if self.len + samples.len() > INJECTED_QUEUE_CAP {
            return Err(Overloaded { pending: self.len });
        }
        self.len += samples.len();
        if !samples.is_empty() {
            self.waiting.push_back(samples);
        }
        Ok(Delivery::Queued)
    }

    /// The next sample for this heartbeat. When it opens a command,
    /// `resolve` places that whole command, through the camera as it
    /// stands now.
    pub(crate) fn pop(&mut self, resolve: impl FnMut(Point) -> Surface) -> Option<Sample<Surface>> {
        if self.landing.is_empty() {
            let next = self.waiting.pop_front()?;
            self.landing = resolve_command(next, resolve).collect();
        }
        let sample = self.landing.pop_front()?;
        self.len -= 1;
        Some(sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::application::ipc::space::Space;

    fn moved(x: f32) -> Sample {
        Sample::new(
            InputEvent::CursorMoved(Point {
                x,
                y: 0.0,
                space: Space::Canvas,
            }),
            None,
        )
    }

    fn landed(x: f64) -> Sample<Surface> {
        Sample::new(InputEvent::CursorMoved((x, 0.0)), None)
    }

    /// A stand-in camera: canvas x lands at `x + shift` on the surface.
    fn through(shift: f64) -> impl FnMut(Point) -> Surface {
        move |p| (f64::from(p.x) + shift, f64::from(p.y))
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
        assert_eq!(queue.pop(through(0.0)), Some(landed(1.0)));
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.pop(through(0.0)), Some(landed(2.0)));
        assert_eq!(queue.pop(through(0.0)), None);
        assert!(queue.is_empty());
    }

    /// Order is kept across commands: a burst sent while a gesture is
    /// still queued waits behind it rather than overtaking it.
    #[test]
    fn test_a_burst_waits_behind_queued_samples() {
        let mut queue = InjectedQueue::default();
        queue.admit(vec![moved(1.0)], Pacing::PerFrame).expect("fits");
        assert_eq!(queue.admit(vec![moved(2.0)], Pacing::Burst), Ok(Delivery::Queued));
        assert_eq!(queue.pop(through(0.0)), Some(landed(1.0)));
        assert_eq!(queue.pop(through(0.0)), Some(landed(2.0)));
    }

    /// A command's positions resolve when its first sample lands, all
    /// through the one camera of that moment: a drag that pans keeps
    /// its own camera for its later frames, and a command queued
    /// behind it aims through the camera the drag left behind, not
    /// the one in force when it was sent.
    #[test]
    fn test_each_command_resolves_through_the_camera_at_its_first_sample() {
        let mut queue = InjectedQueue::default();
        queue
            .admit(vec![moved(1.0), moved(2.0)], Pacing::PerFrame)
            .expect("fits");
        queue.admit(vec![moved(3.0)], Pacing::Burst).expect("fits");
        assert_eq!(queue.pop(through(100.0)), Some(landed(101.0)));
        assert_eq!(
            queue.pop(through(500.0)),
            Some(landed(102.0)),
            "the drag's second frame keeps the camera its first one resolved through"
        );
        assert_eq!(
            queue.pop(through(500.0)),
            Some(landed(503.0)),
            "the queued burst aims through the camera of its own moment"
        );
    }

    /// Overflow refuses the whole command and enqueues none of it, so
    /// a resend after backing off neither drops nor doubles a sample.
    #[test]
    fn test_overflow_refuses_the_whole_command_and_keeps_the_queue() {
        let mut queue = InjectedQueue::default();
        let fill: Vec<Sample> = (0..INJECTED_QUEUE_CAP - 1).map(|i| moved(i as f32)).collect();
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
    /// handlers live in, and the `on_*` entry points the
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
        let entry_points =
            std::fs::read_to_string(repo_path("src/application/app/input_context.rs")).expect("readable");
        // From the touch entry point to the end of the file, which the
        // `on_*` methods close: every method an injected sample reaches.
        // (The context struct above them carries the queue the frame
        // drains, which is the frame's business, not a handler's.)
        let start = entry_points
            .find("    pub(super) fn dispatch_touch_event(")
            .expect("input_context.rs has the touch entry point");
        assert!(
            entry_points[start..].contains("    pub(super) fn on_keyboard_input("),
            "the on_* entry points follow the touch entry point"
        );
        sources.push((
            "input_context.rs input entry points".into(),
            entry_points[start..].to_string(),
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
