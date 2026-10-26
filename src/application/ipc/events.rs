// SPDX-License-Identifier: MPL-2.0

//! The event stream and the revision counters (`format/ipc.md`
//! §`events`).
//!
//! **Observation, not instrumentation.** The app has no
//! change-notification machinery and this module adds none to it: no
//! setter reports, no walker emits. A [`Tracker`] is handed a
//! read-only [`Observed`] view of the app at the points the event loop
//! already passes through, remembers what it saw last, and reports the
//! difference. A document setter, a keystroke, a drag, an animation
//! tick and an IPC command are therefore all seen the same way, and a
//! 200-step macro is one difference. The document's own
//! `revision()` (`document/revision.rs`) is what makes "the model
//! changed" a comparison of two integers rather than a walk.
//!
//! **Two counters.** `document_revision` is the document's: committed
//! model changes, carried across `open` and `new`, untouched by a
//! preview, a selection or the camera. `revision` is this module's: it
//! advances once per observation in which *any* event class had
//! something to report, so a view-only change (a zoom, a selection)
//! moves it while the document's stays put. Both are monotonic per
//! process. An observation is the unit: two changes between two
//! observations advance `revision` once.
//!
//! **Delivery** is the main thread's: observations queue
//! [`Event`]s in a [`Session`], and the transport offers them to the
//! controller after the replies of the drain that produced them
//! (`transport::IpcServer::drain`). Events never pause execution. A
//! queue the controller is not draining is coalesced to the newest
//! event per class rather than grown.

use std::collections::VecDeque;

use serde_json::{json, Value};

use baumhard::gfx_structs::camera::Camera2D;

use crate::application::app::InteractionMode;
use crate::application::document::{MindMapDocument, SelectionState};

use super::envelope::{Event, Reply};
use super::space::camera_json;
use super::wait::{PendingWait, Settling};

/// Events a session holds back before the oldest is dropped. Only a
/// `verbose` subscription can reach it: otherwise the queue holds at
/// most one event per class.
const PENDING_CAP: usize = 4096;

/// The three transport-level classes, with what `describe` says they
/// mean: never subscription-gated, and not [`Class`]es.
pub(crate) const TRANSPORT_EVENTS: &[(&str, &str)] = &[
    (
        "connection_rejected",
        "Sent to a refused connection instead of `hello` while another controller is attached; then closed.",
    ),
    (
        "hello",
        "The greeting on accept: protocol, app, version, pid, and the loaded map's path (null before the first load).",
    ),
    (
        "shutdown",
        "Best-effort notice, before the socket closes, that the app is exiting.",
    ),
];

/// Whether `word` names one of the [`TRANSPORT_EVENTS`].
pub(crate) fn is_transport_event(word: &str) -> bool {
    TRANSPORT_EVENTS.iter().any(|(name, _)| *name == word)
}

/// A subscribable event class. The three transport-level classes
/// (`hello`, `shutdown`, `connection_rejected`) are not here: they
/// are never subscription-gated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    DocumentChanged,
    DocumentReplaced,
    SelectionChanged,
    CameraChanged,
    ModeChanged,
    AnimationStarted,
    AnimationCompleted,
}

impl Class {
    /// Every class, in `describe` order (by name).
    pub(crate) const ALL: [Class; 7] = [
        Class::AnimationCompleted,
        Class::AnimationStarted,
        Class::CameraChanged,
        Class::DocumentChanged,
        Class::DocumentReplaced,
        Class::ModeChanged,
        Class::SelectionChanged,
    ];

    /// The wire name.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Class::DocumentChanged => "document_changed",
            Class::DocumentReplaced => "document_replaced",
            Class::SelectionChanged => "selection_changed",
            Class::CameraChanged => "camera_changed",
            Class::ModeChanged => "mode_changed",
            Class::AnimationStarted => "animation_started",
            Class::AnimationCompleted => "animation_completed",
        }
    }

    /// What `describe` says the class means.
    pub(crate) const fn doc(self) -> &'static str {
        match self {
            Class::DocumentChanged => {
                "A committed document change: data carries revision, document_revision and the source (action, console, mutation, undo, input or ipc)."
            }
            Class::DocumentReplaced => {
                "An open or new replaced the document: data carries revision, document_revision, generation and the new path."
            }
            Class::SelectionChanged => "The selection changed: data carries revision and the selection, as query.selection reports it.",
            Class::CameraChanged => "The camera moved or the surface resized: data carries revision and the camera, as query.camera reports it.",
            Class::ModeChanged => "The interaction mode changed: data carries revision and the mode, as query.mode reports its kind.",
            Class::AnimationStarted => "Animations began: data carries revision and the animations (target_id, section_idx, mutation_id).",
            Class::AnimationCompleted => "Animations committed or were canceled: data carries revision and the animations (target_id, section_idx, mutation_id).",
        }
    }

    /// The class named `word`, if any.
    pub(crate) fn parse(word: &str) -> Option<Class> {
        Class::ALL.into_iter().find(|class| class.name() == word)
    }

    /// Whether the class's payload is a *delta*: only what is new
    /// since the last look, so an event cannot be replaced by a newer
    /// one without losing what it said. The others carry the state
    /// as of the observation, which a newer one supersedes.
    const fn is_delta(self) -> bool {
        matches!(self, Class::AnimationStarted | Class::AnimationCompleted)
    }

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A set of [`Class`]es.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Classes(u8);

impl Classes {
    pub(crate) fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(crate) fn contains(self, class: Class) -> bool {
        self.0 & class.bit() != 0
    }

    pub(crate) fn insert(&mut self, class: Class) {
        self.0 |= class.bit();
    }

    pub(crate) fn remove(&mut self, other: Classes) {
        self.0 &= !other.0;
    }

    /// The members, in [`Class::ALL`] order.
    pub(crate) fn iter(self) -> impl Iterator<Item = Class> {
        Class::ALL.into_iter().filter(move |class| self.contains(*class))
    }
}

impl FromIterator<Class> for Classes {
    fn from_iter<I: IntoIterator<Item = Class>>(iter: I) -> Self {
        let mut set = Classes::default();
        for class in iter {
            set.insert(class);
        }
        set
    }
}

/// What the app looks like at an observation point: the parts every
/// event class reads, borrowed. Built by the app side from the state
/// it holds (the event loop's, or a handler context's), so a macro
/// step can be observed from inside the funnel that runs it.
pub(crate) struct Observed<'a> {
    /// `None` before a document exists.
    pub(crate) document: Option<&'a MindMapDocument>,
    pub(crate) camera: &'a Camera2D,
    pub(crate) mode: &'a InteractionMode,
}

/// The counters, as a reply carries them (`format/ipc.md` §Reply).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Revisions {
    /// This module's counter: advances when any event class had
    /// something to report.
    pub(crate) revision: u64,
    /// The document's counter (`MindMapDocument::revision`).
    pub(crate) document_revision: u64,
}

/// Who is driving the app at an observation. Decides the `source` a
/// `document_changed` reports when the change was an agent's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Driver {
    /// The app's own input handlers, which declare their funnel on the
    /// document.
    App,
    /// An IPC command.
    Ipc,
}

/// A camera, as the tracker compares it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CameraKey {
    position: (f32, f32),
    zoom: f32,
    viewport: (u32, u32),
}

impl CameraKey {
    fn of(camera: &Camera2D) -> Self {
        CameraKey {
            position: (camera.position.x, camera.position.y),
            zoom: camera.zoom,
            viewport: camera.viewport_size,
        }
    }
}

/// A running animation, as the tracker names it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AnimationKey {
    /// The animation's identity (`AnimationInstance::seq`): constant
    /// while it runs, new when the same `(target_id, section_idx,
    /// mutation_id)` is triggered again after it ended. Compared
    /// alone; the rest is for the payload.
    seq: u64,
    target_id: String,
    section_idx: Option<usize>,
    mutation_id: String,
}

impl AnimationKey {
    fn of(animation: &crate::application::document::AnimationInstance) -> Self {
        AnimationKey {
            seq: animation.seq,
            target_id: animation.target_id.clone(),
            section_idx: animation.section_idx,
            mutation_id: animation.mutation_id().to_string(),
        }
    }

    fn is(&self, animation: &crate::application::document::AnimationInstance) -> bool {
        self.seq == animation.seq
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.seq,
            "target_id": self.target_id,
            "section_idx": self.section_idx,
            "mutation_id": self.mutation_id,
        })
    }
}

/// What the last observation saw.
struct Seen {
    generation: u64,
    document_revision: u64,
    selection: SelectionState,
    camera: CameraKey,
    mode: InteractionMode,
    animations: Vec<AnimationKey>,
}

/// One event an observation produced, before it is queued.
pub(crate) struct Change {
    class: Class,
    data: Value,
}

/// Remembers what the app looked like at the last observation and
/// reports what differs. Lives as long as the process: its
/// `revision` must not restart when a controller reconnects.
#[derive(Default)]
pub(crate) struct Tracker {
    seen: Option<Seen>,
    revision: u64,
}

impl Tracker {
    /// The counters as of the last observation. `document_revision`
    /// is read from `now` rather than remembered, so it is exact.
    pub(crate) fn revisions(&self, now: &Observed<'_>) -> Revisions {
        Revisions {
            revision: self.revision,
            document_revision: now.document.map_or(0, MindMapDocument::revision),
        }
    }

    /// Compare `now` with what was seen last. The first observation
    /// only records a baseline: whatever the app already looked like
    /// is not a change.
    ///
    /// Advances `revision` once if any class differs, and returns the
    /// events for the classes in `wanted` that do, their payloads
    /// stamped with the new revision. A class outside `wanted` still
    /// counts toward `revision` and still updates what was seen: a
    /// later subscription reports changes from then on, not the
    /// history.
    ///
    /// Cost: integer, selection, camera and mode comparisons, and one
    /// pass over the running animations. Allocates only for a class
    /// that differs.
    pub(crate) fn observe(
        &mut self,
        now: &Observed<'_>,
        wanted: Classes,
        driver: Driver,
    ) -> (Revisions, Vec<Change>) {
        let generation = now.document.map_or(0, MindMapDocument::generation);
        let document_revision = now.document.map_or(0, MindMapDocument::revision);
        let none = SelectionState::None;
        let selection = now.document.map_or(&none, |doc| &doc.selection);
        let camera = CameraKey::of(now.camera);
        let animations: &[crate::application::document::AnimationInstance] =
            now.document.map_or(&[], |doc| &doc.active_animations);

        let Some(seen) = self.seen.as_mut() else {
            self.seen = Some(Seen {
                generation,
                document_revision,
                selection: selection.clone(),
                camera,
                mode: now.mode.clone(),
                animations: animations.iter().map(AnimationKey::of).collect(),
            });
            return (self.revisions(now), Vec::new());
        };

        let replaced = seen.generation != generation;
        let document = seen.document_revision != document_revision;
        let selected = seen.selection != *selection;
        let moved = seen.camera != camera;
        let moded = seen.mode != *now.mode;
        // As sets, not by position: a tick removes with `swap_remove`,
        // which reorders what survives.
        let animations_differ = seen.animations.len() != animations.len()
            || !animations
                .iter()
                .all(|anim| seen.animations.iter().any(|key| key.is(anim)));
        let any = replaced || document || selected || moved || moded || animations_differ;
        if !any {
            return (self.revisions(now), Vec::new());
        }
        self.revision += 1;
        let revision = self.revision;
        let stamped = |mut payload: serde_json::Map<String, Value>| {
            payload.insert("revision".into(), json!(revision));
            Value::Object(payload)
        };
        let mut changes = Vec::new();
        let mut push = |class: Class, payload: serde_json::Map<String, Value>| {
            if wanted.contains(class) {
                changes.push(Change {
                    class,
                    data: stamped(payload),
                });
            }
        };
        let object = |value: Value| match value {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };

        if document {
            let source = match driver {
                Driver::Ipc => "ipc",
                Driver::App => now
                    .document
                    .map_or("input", |doc| doc.last_change_source().word()),
            };
            push(
                Class::DocumentChanged,
                object(json!({ "document_revision": document_revision, "source": source })),
            );
        }
        if replaced {
            push(
                Class::DocumentReplaced,
                object(json!({
                    "document_revision": document_revision,
                    "generation": generation,
                    "path": now.document.and_then(|doc| doc.file_path.as_deref()),
                })),
            );
        }
        if selected {
            push(
                Class::SelectionChanged,
                object(json!({ "selection": selection.to_ipc_json() })),
            );
        }
        if moved {
            push(
                Class::CameraChanged,
                object(json!({ "camera": camera_json(now.camera) })),
            );
        }
        if moded {
            push(
                Class::ModeChanged,
                object(json!({ "mode": now.mode.to_ipc_json() })),
            );
        }
        if animations_differ {
            let started: Vec<Value> = animations
                .iter()
                .filter(|anim| !seen.animations.iter().any(|key| key.is(anim)))
                .map(|anim| AnimationKey::of(anim).to_json())
                .collect();
            let completed: Vec<Value> = seen
                .animations
                .iter()
                .filter(|key| !animations.iter().any(|anim| key.is(anim)))
                .map(AnimationKey::to_json)
                .collect();
            // Completed first: an animation that ended and was triggered
            // again since the last look must read as the old one ending
            // and the new one beginning, in that order.
            if !completed.is_empty() {
                push(
                    Class::AnimationCompleted,
                    object(json!({ "animations": completed })),
                );
            }
            if !started.is_empty() {
                push(Class::AnimationStarted, object(json!({ "animations": started })));
            }
        }

        seen.generation = generation;
        seen.document_revision = document_revision;
        if selected {
            seen.selection = selection.clone();
        }
        seen.camera = camera;
        if moded {
            seen.mode = now.mode.clone();
        }
        if animations_differ {
            seen.animations = animations.iter().map(AnimationKey::of).collect();
        }
        (
            Revisions {
                revision,
                document_revision,
            },
            changes,
        )
    }
}

/// Fold `older`'s animations ahead of `newer`'s: the two deltas as
/// one, in the order the animations began or ended.
fn fold_animations(older: &mut Event, newer: &mut Event) {
    let Some(Value::Array(mut merged)) = older.data_mut().get_mut("animations").map(Value::take) else {
        return;
    };
    if let Some(Value::Array(newer_animations)) = newer.data_mut().get_mut("animations").map(Value::take) {
        merged.extend(newer_animations);
    }
    newer.data_mut()["animations"] = Value::Array(merged);
}

/// What a controller asked to hear.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Subscriptions {
    pub(crate) classes: Classes,
    /// Report every observation separately, macro steps included,
    /// instead of coalescing.
    pub(crate) verbose: bool,
}

/// An event waiting for the controller's queue to have room.
pub(crate) struct Queued {
    pub(crate) class: Class,
    pub(crate) event: Event,
}

/// One IPC connection's state, owned by the transport and lent to each
/// command: the process-lifetime [`Tracker`], the controller's
/// subscriptions and the events not yet delivered — and the
/// controller's pending `clock.wait` (`super::wait`), which lives here
/// because it lives exactly as long as the controller does.
#[derive(Default)]
pub(crate) struct Session {
    tracker: Tracker,
    subscriptions: Subscriptions,
    pending: VecDeque<Queued>,
    wait: Option<PendingWait>,
}

impl Session {
    /// A new controller: no subscriptions, nothing queued, no wait
    /// pending — a disconnect cancels the departed controller's. The
    /// tracker, and with it the revision, carries on.
    pub(crate) fn reset_controller(&mut self) {
        self.subscriptions = Subscriptions::default();
        self.pending.clear();
        self.wait = None;
    }

    /// The controller's pending `clock.wait`.
    pub(crate) fn wait(&self) -> Option<&PendingWait> {
        self.wait.as_ref()
    }

    /// Hold `wait`'s reply back until it resolves. The caller has
    /// checked that none is pending: waits are exclusive.
    pub(crate) fn hold_wait(&mut self, wait: PendingWait) {
        self.wait = Some(wait);
    }

    /// The pending wait's reply, when it is due at `now` given `app`,
    /// which also ends the wait.
    pub(crate) fn resolve_wait(&mut self, now: std::time::Instant, app: Settling) -> Option<Reply> {
        let reply = self.wait.as_ref()?.resolve(now, app)?;
        self.wait = None;
        Some(reply)
    }

    pub(crate) fn subscriptions(&self) -> Subscriptions {
        self.subscriptions
    }

    /// Whether any class is subscribed. The event loop observes the
    /// app between commands only when this is true.
    pub(crate) fn is_subscribed(&self) -> bool {
        !self.subscriptions.classes.is_empty()
    }

    /// Add `classes`, and set the verbose level.
    pub(crate) fn subscribe(&mut self, classes: Classes, verbose: bool) {
        for class in classes.iter() {
            self.subscriptions.classes.insert(class);
        }
        self.subscriptions.verbose = verbose;
    }

    /// Drop `classes` (all of them when `None`), and any event of a
    /// dropped class still waiting.
    pub(crate) fn unsubscribe(&mut self, classes: Option<Classes>) {
        let dropped = classes.unwrap_or(self.subscriptions.classes);
        self.subscriptions.classes.remove(dropped);
        self.pending.retain(|queued| !dropped.contains(queued.class));
        if self.subscriptions.classes.is_empty() {
            self.subscriptions.verbose = false;
        }
    }

    /// Observe the app and queue what the subscribed classes report.
    /// Returns the counters, which a reply carries.
    pub(crate) fn observe(&mut self, now: &Observed<'_>, driver: Driver) -> Revisions {
        let (revisions, changes) = self.tracker.observe(now, self.subscriptions.classes, driver);
        self.queue(changes);
        revisions
    }

    /// Take the first look at the app, when there has not been one:
    /// whatever it already looks like is the baseline, not a change.
    /// A no-op afterwards.
    pub(crate) fn baseline(&mut self, now: &Observed<'_>) {
        if self.tracker.seen.is_none() {
            self.observe(now, Driver::App);
        }
    }

    /// Observe between the steps of a macro, which only a `verbose`
    /// subscription asks for: an observation per step, none coalesced.
    pub(crate) fn observe_step(&mut self, now: &Observed<'_>) {
        if self.subscriptions.verbose {
            self.observe(now, Driver::Ipc);
        }
    }

    /// Queue `changes`: coalesced to the newest of each class unless
    /// the subscription is `verbose`.
    fn queue(&mut self, changes: Vec<Change>) {
        for Change { class, data } in changes {
            if !self.subscriptions.verbose {
                if class.is_delta() {
                    // A delta event is folded into the newest one of its
                    // class that is waiting, in its place, so what it
                    // says survives. Only when no delta of the *other*
                    // class was queued after that one: folding across it
                    // would announce a restart ahead of the completion
                    // it follows, so the new delta goes to the back.
                    let waiting = self
                        .pending
                        .iter()
                        .rposition(|queued| queued.class == class)
                        .filter(|at| {
                            !self
                                .pending
                                .iter()
                                .skip(at + 1)
                                .any(|queued| queued.class.is_delta())
                        });
                    if let Some(at) = waiting {
                        let mut newer = Event::change(class.name(), data);
                        fold_animations(&mut self.pending[at].event, &mut newer);
                        self.pending[at].event = newer;
                        continue;
                    }
                } else {
                    self.pending.retain(|queued| queued.class != class);
                }
            }
            if self.pending.len() == PENDING_CAP {
                self.pending.pop_front();
            }
            self.pending.push_back(Queued {
                class,
                event: Event::change(class.name(), data),
            });
        }
    }

    /// The oldest waiting event, for the transport to offer.
    pub(crate) fn pop_pending(&mut self) -> Option<Queued> {
        self.pending.pop_front()
    }

    /// Put back an event the controller's queue had no room for, ahead
    /// of everything that waited behind it — unless a newer event of
    /// its class has queued meanwhile, which supersedes it.
    pub(crate) fn requeue(&mut self, queued: Queued) {
        // A delta event is never superseded: it goes back in front of
        // the newer one of its class, which the next queue folds into.
        if !self.subscriptions.verbose
            && !queued.class.is_delta()
            && self.pending.iter().any(|waiting| waiting.class == queued.class)
        {
            return;
        }
        self.pending.push_front(queued);
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    //! An app for the event layer to look at, behind the
    //! `CommandTarget` seam: a real document, a real camera and a
    //! real interaction mode, with the funnels an event test drives
    //! (a console line, a macro's steps) reduced to the document
    //! change they stand for. A real `IpcCommandTarget` needs a GPU
    //! renderer, which `TEST_CONVENTIONS` §T8 rules out.

    use serde_json::{json, Value};

    use super::Observed;
    use crate::application::app::InteractionMode;
    use crate::application::console::{ExecResult, LineOutcome};
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::MindMapDocument;
    use crate::application::ipc::injected::{Overloaded, Pacing, Sample};
    use crate::application::ipc::space::Point;
    use crate::application::ipc::{CommandTarget, Injected, SceneView, Screenshot, ShellState};
    use crate::application::keybinds::Action;
    use crate::application::macros::MacroStep;
    use baumhard::gfx_structs::camera::Camera2D;

    pub(crate) struct EventsApp {
        pub(crate) doc: MindMapDocument,
        pub(crate) camera: Camera2D,
        pub(crate) mode: InteractionMode,
        /// Whether the funnels reported a pixel change.
        pub(crate) changed: bool,
        pub(crate) clock: crate::application::clock::AppClock,
    }

    impl EventsApp {
        pub(crate) fn new() -> Self {
            EventsApp {
                doc: load_test_doc(),
                camera: Camera2D::new(800, 600),
                mode: InteractionMode::Default,
                changed: false,
                clock: crate::application::clock::AppClock::default(),
            }
        }
    }

    impl CommandTarget for EventsApp {
        fn dispatch_action(&mut self, _: Action, _: Option<Point>) -> crate::application::ipc::ActionOutcome {
            panic!("an event test must not dispatch an Action");
        }
        /// Every console line stands for one committed document
        /// change, whatever it says.
        fn run_console_line(&mut self, _: &str) -> Option<LineOutcome> {
            self.doc.mark_dirty();
            self.changed = true;
            Some(LineOutcome::Ran(ExecResult::Ok(String::new())))
        }
        fn has_macro(&self, _: &str) -> bool {
            false
        }
        fn run_macro(&mut self, _: &str, _: &mut dyn FnMut(Observed<'_>)) -> bool {
            panic!("an event test must not run a registered macro");
        }
        /// One document change per step, observed after each, as the
        /// real step loop does.
        fn run_macro_steps(&mut self, steps: &[MacroStep], after_step: &mut dyn FnMut(Observed<'_>)) -> bool {
            for _ in steps {
                self.doc.mark_dirty();
                after_step(Observed {
                    document: Some(&self.doc),
                    camera: &self.camera,
                    mode: &self.mode,
                });
            }
            self.changed = true;
            !steps.is_empty()
        }
        fn observed(&self) -> Option<Observed<'_>> {
            Some(Observed {
                document: Some(&self.doc),
                camera: &self.camera,
                mode: &self.mode,
            })
        }
        fn changed_view(&self) -> bool {
            self.changed
        }
        fn document(&self) -> Option<&MindMapDocument> {
            Some(&self.doc)
        }
        fn animation_pause(&self) -> Option<u64> {
            None
        }
        fn shell_state(&self, which: ShellState) -> Value {
            json!({ "shell_state": format!("{which:?}") })
        }
        fn scene(&self) -> Option<SceneView<'_>> {
            panic!("an event test must not read the scene");
        }
        fn hit_test(&mut self, _: Point) -> Option<Value> {
            panic!("an event test must not hit-test the scene");
        }
        fn surface_point(&self, _: Point) -> (f64, f64) {
            panic!("an event test must not place a pointer");
        }
        fn inject(&mut self, _: Vec<Sample>, _: Pacing) -> Result<Injected, Overloaded> {
            panic!("an event test must not inject input");
        }
        fn injected_pending(&self) -> usize {
            panic!("an event test must not read the injected-input queue");
        }
        fn capture(
            &mut self,
            _: &crate::application::capture::plan::CaptureRequest,
            _: crate::application::capture::Destination,
            _: bool,
        ) -> Option<Result<Screenshot, crate::application::capture::CaptureError>> {
            panic!("an event test must not take a screenshot");
        }
        fn record_start(
            &mut self,
            _: &crate::application::capture::plan::CaptureRequest,
            _: u32,
            _: u32,
            _: crate::application::capture::record::RecordDestination,
        ) -> Option<
            Result<crate::application::capture::record::Started, crate::application::capture::CaptureError>,
        > {
            panic!("an event test must not start a recording");
        }
        fn record_stop(
            &mut self,
        ) -> Result<crate::application::capture::record::Summary, crate::application::capture::CaptureError>
        {
            panic!("an event test must not stop a recording");
        }
        fn clock(&self) -> &crate::application::clock::AppClock {
            &self.clock
        }
        fn set_clock_mode(
            &mut self,
            _: crate::application::clock::ClockMode,
        ) -> Result<bool, crate::application::clock::ClockRefusal> {
            panic!("an event test must not switch the clock");
        }
        fn step_clock(
            &mut self,
            _: crate::application::clock::StepRequest,
        ) -> Result<crate::application::clock::Stepped, crate::application::clock::ClockRefusal> {
            panic!("an event test must not step the clock");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::{
        first_testament_node_id, load_test_doc, two_testament_node_ids, TestNudgeMutation,
    };
    use crate::application::document::{ChangeSource, SelectionState};
    use baumhard::mindmap::animation::{AnimationTiming, Easing};
    use baumhard::mindmap::custom_mutation::TargetScope;

    /// A world to observe: a document, a camera and a mode, that a
    /// test edits between observations.
    struct World {
        doc: MindMapDocument,
        camera: Camera2D,
        mode: InteractionMode,
    }

    impl World {
        fn new() -> Self {
            World {
                doc: load_test_doc(),
                camera: Camera2D::new(800, 600),
                mode: InteractionMode::Default,
            }
        }

        fn now(&self) -> Observed<'_> {
            Observed {
                document: Some(&self.doc),
                camera: &self.camera,
                mode: &self.mode,
            }
        }
    }

    const ALL: fn() -> Classes = || Class::ALL.into_iter().collect();

    fn names(changes: &[Change]) -> Vec<&'static str> {
        changes.iter().map(|change| change.class.name()).collect()
    }

    fn find(changes: &[Change], class: Class) -> &Value {
        &changes
            .iter()
            .find(|change| change.class == class)
            .unwrap_or_else(|| panic!("no {} among {:?}", class.name(), names(changes)))
            .data
    }

    /// A tracker already past its baseline, so the next observation
    /// can report.
    fn baselined(world: &World) -> Tracker {
        let mut tracker = Tracker::default();
        let (_, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert!(changes.is_empty(), "the first look is the baseline, not a change");
        tracker
    }

    #[test]
    fn test_class_names_are_unique_snake_case_and_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for class in Class::ALL {
            assert!(seen.insert(class.name()), "{} twice", class.name());
            assert!(class.name().chars().all(|c| c.is_ascii_lowercase() || c == '_'));
            assert!(!class.doc().trim().is_empty());
            assert_eq!(Class::parse(class.name()), Some(class));
        }
        assert_eq!(
            Class::parse("hello"),
            None,
            "transport classes are not subscribable"
        );
        let by_name: Vec<&str> = Class::ALL.iter().map(|c| c.name()).collect();
        let mut sorted = by_name.clone();
        sorted.sort_unstable();
        assert_eq!(by_name, sorted, "ALL is in describe (name) order");
    }

    #[test]
    fn test_classes_is_a_set_over_the_classes() {
        let mut set = Classes::default();
        assert!(set.is_empty());
        set.insert(Class::CameraChanged);
        set.insert(Class::ModeChanged);
        set.insert(Class::CameraChanged);
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            [Class::CameraChanged, Class::ModeChanged]
        );
        set.remove([Class::CameraChanged].into_iter().collect());
        assert!(!set.contains(Class::CameraChanged) && set.contains(Class::ModeChanged));
    }

    #[test]
    fn test_the_first_observation_is_a_baseline_and_a_quiet_one_changes_nothing() {
        let world = World::new();
        let mut tracker = Tracker::default();
        let (first, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert!(changes.is_empty());
        assert_eq!(
            first,
            Revisions {
                revision: 0,
                document_revision: 0
            }
        );
        let (second, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert!(changes.is_empty());
        assert_eq!(second, first, "looking again advances nothing");
    }

    #[test]
    fn test_a_document_change_reports_both_counters_and_the_source() {
        let mut world = World::new();
        let mut tracker = baselined(&world);
        world
            .doc
            .with_source(ChangeSource::Console, |doc| doc.mark_dirty());
        let (revisions, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert_eq!(names(&changes), ["document_changed"]);
        assert_eq!(
            find(&changes, Class::DocumentChanged),
            &json!({ "revision": 1, "document_revision": 1, "source": "console" })
        );
        assert_eq!(
            revisions,
            Revisions {
                revision: 1,
                document_revision: 1
            }
        );
    }

    #[test]
    fn test_an_agents_command_is_the_source_whatever_funnel_it_went_through() {
        let mut world = World::new();
        let mut tracker = baselined(&world);
        world.doc.with_source(ChangeSource::Undo, |doc| doc.mark_dirty());
        let (_, changes) = tracker.observe(&world.now(), ALL(), Driver::Ipc);
        assert_eq!(find(&changes, Class::DocumentChanged)["source"], json!("ipc"));
    }

    #[test]
    fn test_a_class_nobody_asked_for_still_moves_the_revision_but_reports_nothing() {
        let mut world = World::new();
        let mut tracker = baselined(&world);
        world.doc.mark_dirty();
        let (revisions, changes) = tracker.observe(&world.now(), Classes::default(), Driver::App);
        assert!(changes.is_empty());
        assert_eq!(
            revisions.revision, 1,
            "the counter does not depend on who is listening"
        );
        let (_, again) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert!(
            again.is_empty(),
            "subscribing later reports from then on, not the history"
        );
    }

    #[test]
    fn test_two_changes_between_observations_advance_the_revision_once() {
        let mut world = World::new();
        let mut tracker = baselined(&world);
        world.doc.mark_dirty();
        world.doc.mark_dirty();
        world.camera.zoom = 2.0;
        let (revisions, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert_eq!(names(&changes), ["document_changed", "camera_changed"]);
        assert_eq!(
            revisions,
            Revisions {
                revision: 1,
                document_revision: 2
            }
        );
        assert_eq!(find(&changes, Class::DocumentChanged)["revision"], json!(1));
    }

    #[test]
    fn test_a_view_change_moves_the_revision_and_leaves_the_document_revision() {
        let mut world = World::new();
        let mut tracker = baselined(&world);
        let node = first_testament_node_id(&world.doc);
        world.doc.selection = SelectionState::Single(node.clone());
        let (revisions, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert_eq!(names(&changes), ["selection_changed"]);
        assert_eq!(
            find(&changes, Class::SelectionChanged)["selection"],
            json!({ "kind": "Single", "node_id": node })
        );
        assert_eq!(
            revisions,
            Revisions {
                revision: 1,
                document_revision: 0
            }
        );
    }

    #[test]
    fn test_camera_and_mode_changes_carry_their_projections() {
        let mut world = World::new();
        let mut tracker = baselined(&world);
        world.camera.zoom = 2.0;
        world.camera.position = glam::Vec2::new(5.0, -7.0);
        world.mode = InteractionMode::Connect { source: "n".into() };
        let (_, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert_eq!(names(&changes), ["camera_changed", "mode_changed"]);
        assert_eq!(
            find(&changes, Class::CameraChanged)["camera"],
            camera_json(&world.camera)
        );
        assert_eq!(
            find(&changes, Class::ModeChanged)["mode"],
            json!({ "kind": "Connect", "source": "n" })
        );
    }

    #[test]
    fn test_a_surface_resize_is_a_camera_change() {
        let mut world = World::new();
        let mut tracker = baselined(&world);
        world.camera.viewport_size = (1024, 768);
        let (_, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert_eq!(names(&changes), ["camera_changed"]);
    }

    #[test]
    fn test_an_animation_starting_and_committing_are_two_events() {
        let mut world = World::new();
        let mut tracker = baselined(&world);
        let node = first_testament_node_id(&world.doc);
        let cm = TestNudgeMutation::new("slide", TargetScope::SelfOnly)
            .magnitude(100.0)
            .timing(AnimationTiming {
                duration_ms: 500,
                delay_ms: 0,
                easing: Easing::Linear,
                then: None,
            })
            .build();
        world.doc.start_animation(&cm, &node, 0);
        let (_, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert_eq!(names(&changes), ["animation_started"]);
        let entry = json!({ "id": 1, "target_id": node, "section_idx": null, "mutation_id": "slide" });
        assert_eq!(
            find(&changes, Class::AnimationStarted)["animations"],
            json!([entry.clone()])
        );

        let (_, quiet) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert!(quiet.is_empty(), "a running animation is not a change");

        world.doc.tick_animations(1_000, None);
        let (_, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert_eq!(names(&changes), ["document_changed", "animation_completed"]);
        assert_eq!(
            find(&changes, Class::AnimationCompleted)["animations"],
            json!([entry])
        );
        assert_eq!(
            find(&changes, Class::DocumentChanged)["source"],
            json!("mutation")
        );
    }

    fn slide(id: &str) -> baumhard::mindmap::custom_mutation::CustomMutation {
        TestNudgeMutation::new(id, TargetScope::SelfOnly)
            .magnitude(100.0)
            .timing(AnimationTiming {
                duration_ms: 500,
                delay_ms: 0,
                easing: Easing::Linear,
                then: None,
            })
            .build()
    }

    fn animation_targets(data: &Value) -> Vec<String> {
        data["animations"]
            .as_array()
            .expect("animations")
            .iter()
            .map(|entry| entry["target_id"].as_str().expect("target").to_string())
            .collect()
    }

    /// The animation events a session holds, in delivery order, as
    /// `(class, targets)`.
    fn animation_events(session: &mut Session) -> Vec<(&'static str, Vec<String>)> {
        drain(session)
            .into_iter()
            .filter(|(name, _)| name.starts_with("animation"))
            .map(|(name, data)| (name, animation_targets(&data)))
            .collect()
    }

    #[test]
    fn test_animations_that_start_in_one_drain_are_one_event_carrying_all_of_them() {
        let mut world = World::new();
        let (a, b) = two_testament_node_ids(&world.doc);
        let mut session = Session::default();
        session.subscribe(ALL(), false);
        session.baseline(&world.now());
        world.doc.start_animation(&slide("slide"), &a, 0);
        session.observe(&world.now(), Driver::Ipc);
        world.doc.start_animation(&slide("slide"), &b, 0);
        session.observe(&world.now(), Driver::Ipc);
        let events = drain(&mut session);
        assert_eq!(events.len(), 1, "coalesced");
        assert_eq!(events[0].0, "animation_started");
        assert_eq!(animation_targets(&events[0].1), [a, b], "neither start is lost");
        assert_eq!(
            events[0].1["revision"],
            json!(2),
            "stamped with the newest revision"
        );
    }

    #[test]
    fn test_animations_in_one_burst_fold_into_one_event_per_class_in_order() {
        let mut world = World::new();
        let (a, b) = two_testament_node_ids(&world.doc);
        let mut session = Session::default();
        session.subscribe(ALL(), false);
        session.baseline(&world.now());
        world.doc.start_animation(&slide("slide"), &a, 0);
        session.observe(&world.now(), Driver::App);
        world.doc.start_animation(&slide("slide"), &b, 0);
        session.observe(&world.now(), Driver::App);
        world.doc.tick_animations(1_000, None);
        session.observe(&world.now(), Driver::App);
        assert_eq!(
            animation_events(&mut session),
            [
                ("animation_started", vec![a.clone(), b.clone()]),
                ("animation_completed", vec![a, b]),
            ]
        );
    }

    #[test]
    fn test_a_new_delta_is_not_folded_across_the_other_class_queued_after_its_sibling() {
        let mut world = World::new();
        let (a, x) = two_testament_node_ids(&world.doc);
        let mut session = Session::default();
        session.subscribe(ALL(), false);
        world.doc.start_animation(&slide("slide"), &a, 0);
        session.baseline(&world.now());
        // started{x} and completed{a} wait, in that order. `x` starts
        // late enough to still be running when `a` ends.
        world.doc.start_animation(&slide("slide"), &x, 900);
        session.observe(&world.now(), Driver::App);
        world.doc.tick_animations(1_000, None);
        session.observe(&world.now(), Driver::App);
        // `a` is triggered again.
        world.doc.start_animation(&slide("slide"), &a, 2_000);
        session.observe(&world.now(), Driver::App);
        assert_eq!(
            animation_events(&mut session),
            [
                ("animation_started", vec![x]),
                ("animation_completed", vec![a.clone()]),
                ("animation_started", vec![a]),
            ],
            "the restart arrives after the completion it follows, and on its own"
        );
    }

    #[test]
    fn test_a_held_back_delta_is_never_superseded() {
        let mut world = World::new();
        let (a, b) = two_testament_node_ids(&world.doc);
        let mut session = Session::default();
        session.subscribe(ALL(), false);
        session.baseline(&world.now());
        world.doc.start_animation(&slide("slide"), &a, 0);
        session.observe(&world.now(), Driver::App);
        let held = session.pop_pending().expect("queued");
        world.doc.start_animation(&slide("slide"), &b, 0);
        session.observe(&world.now(), Driver::App);
        session.requeue(held);
        assert_eq!(
            animation_events(&mut session),
            [("animation_started", vec![a]), ("animation_started", vec![b])],
            "both delivered, the held one first"
        );
    }

    #[test]
    fn test_an_animation_restarted_between_observations_is_a_completion_then_a_start_with_a_new_id() {
        let mut world = World::new();
        let node = first_testament_node_id(&world.doc);
        world.doc.start_animation(&slide("slide"), &node, 0);
        let mut tracker = baselined(&world);
        world.doc.tick_animations(1_000, None);
        world.doc.start_animation(&slide("slide"), &node, 2_000);
        let (revisions, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert!(revisions.revision > 0);
        assert_eq!(
            names(&changes),
            ["document_changed", "animation_completed", "animation_started"],
            "the old one ends before the new one begins"
        );
        let ended = &find(&changes, Class::AnimationCompleted)["animations"][0];
        let began = &find(&changes, Class::AnimationStarted)["animations"][0];
        assert_eq!(ended["target_id"], began["target_id"]);
        assert_eq!(ended["mutation_id"], began["mutation_id"]);
        assert_ne!(ended["id"], began["id"], "the payload says which one is new");
    }

    #[test]
    fn test_an_animations_id_is_the_same_in_the_query_and_in_its_events() {
        let mut world = World::new();
        let node = first_testament_node_id(&world.doc);
        let mut tracker = baselined(&world);
        world.doc.start_animation(&slide("slide"), &node, 0);
        let (_, started) = tracker.observe(&world.now(), ALL(), Driver::App);
        let queried = world.doc.active_animations[0].to_ipc_json(100, false)["id"].clone();
        assert_eq!(
            find(&started, Class::AnimationStarted)["animations"][0]["id"],
            queried
        );

        world.doc.tick_animations(1_000, None);
        let (_, completed) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert_eq!(
            find(&completed, Class::AnimationCompleted)["animations"][0]["id"],
            queried,
            "the completion names the animation the query listed"
        );
    }

    #[test]
    fn test_shifting_the_start_of_running_animations_is_not_a_change() {
        // A tree-mutating drag released after a pause moves every
        // running animation's start forward so none snaps to its end.
        let mut world = World::new();
        let (a, b) = two_testament_node_ids(&world.doc);
        world.doc.start_animation(&slide("slide"), &a, 0);
        world.doc.start_animation(&slide("slide"), &b, 100);
        let mut tracker = baselined(&world);
        world.doc.shift_active_animations_start_ms(750);
        let (revisions, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert!(changes.is_empty(), "{:?}", names(&changes));
        assert_eq!(revisions.revision, 0);
    }

    #[test]
    fn test_reordering_the_running_animations_is_not_a_change() {
        let mut world = World::new();
        let (a, b) = two_testament_node_ids(&world.doc);
        world.doc.start_animation(&slide("slide"), &a, 0);
        world.doc.start_animation(&slide("slide"), &b, 0);
        let mut tracker = baselined(&world);
        world.doc.active_animations.reverse();
        let (revisions, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert!(changes.is_empty());
        assert_eq!(
            revisions.revision, 0,
            "the same animations in another order: nothing happened"
        );
    }

    #[test]
    fn test_a_replaced_document_reports_its_path_and_never_lowers_the_counters() {
        let mut world = World::new();
        world.doc.mark_dirty();
        world.doc.mark_dirty();
        let mut tracker = baselined(&world);
        let mut next = load_test_doc();
        next.file_path = Some("maps/next.mindmap.json".into());
        next.succeed(&world.doc);
        world.doc = next;
        let (revisions, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert_eq!(names(&changes), ["document_changed", "document_replaced"]);
        assert_eq!(
            find(&changes, Class::DocumentReplaced),
            &json!({
                "revision": 1,
                "document_revision": 3,
                "generation": 1,
                "path": "maps/next.mindmap.json",
            })
        );
        assert_eq!(revisions.document_revision, 3);
    }

    #[test]
    fn test_a_session_starts_silent_and_subscribing_is_per_class() {
        let mut session = Session::default();
        assert!(!session.is_subscribed());
        session.subscribe([Class::DocumentChanged].into_iter().collect(), false);
        session.subscribe([Class::CameraChanged].into_iter().collect(), true);
        let subscribed = session.subscriptions();
        assert_eq!(
            subscribed.classes.iter().collect::<Vec<_>>(),
            [Class::CameraChanged, Class::DocumentChanged]
        );
        assert!(subscribed.verbose);
        session.unsubscribe(Some([Class::CameraChanged].into_iter().collect()));
        assert_eq!(
            session.subscriptions().classes.iter().collect::<Vec<_>>(),
            [Class::DocumentChanged]
        );
        session.unsubscribe(None);
        assert!(!session.is_subscribed());
        assert!(
            !session.subscriptions().verbose,
            "verbose goes with the last subscription"
        );
    }

    fn drain(session: &mut Session) -> Vec<(&'static str, Value)> {
        let mut out = Vec::new();
        while let Some(queued) = session.pop_pending() {
            out.push((
                queued.class.name(),
                serde_json::to_value(&queued.event).expect("serializes")["data"].clone(),
            ));
        }
        out
    }

    #[test]
    fn test_unflushed_events_coalesce_to_the_newest_per_class() {
        let mut world = World::new();
        let mut session = Session::default();
        session.subscribe(ALL(), false);
        session.baseline(&world.now());
        for _ in 0..3 {
            world.doc.mark_dirty();
            session.observe(&world.now(), Driver::App);
        }
        world.camera.zoom = 3.0;
        session.observe(&world.now(), Driver::App);
        let events = drain(&mut session);
        let order: Vec<&str> = events.iter().map(|(name, _)| *name).collect();
        assert_eq!(order, ["document_changed", "camera_changed"]);
        assert_eq!(
            events[0].1["document_revision"],
            json!(3),
            "the final state, once"
        );
    }

    #[test]
    fn test_a_verbose_session_keeps_every_observation() {
        let mut world = World::new();
        let mut session = Session::default();
        session.subscribe([Class::DocumentChanged].into_iter().collect(), true);
        session.baseline(&world.now());
        for _ in 0..3 {
            world.doc.mark_dirty();
            session.observe_step(&world.now());
        }
        let revisions: Vec<Value> = drain(&mut session)
            .into_iter()
            .map(|(_, data)| data["document_revision"].clone())
            .collect();
        assert_eq!(revisions, [json!(1), json!(2), json!(3)]);
    }

    #[test]
    fn test_a_quiet_session_does_not_observe_between_steps() {
        let mut world = World::new();
        let mut session = Session::default();
        session.subscribe([Class::DocumentChanged].into_iter().collect(), false);
        session.baseline(&world.now());
        world.doc.mark_dirty();
        session.observe_step(&world.now());
        assert!(drain(&mut session).is_empty());
    }

    #[test]
    fn test_unsubscribing_drops_the_events_still_waiting() {
        let mut world = World::new();
        let mut session = Session::default();
        session.subscribe(ALL(), false);
        session.baseline(&world.now());
        world.doc.mark_dirty();
        world.camera.zoom = 3.0;
        session.observe(&world.now(), Driver::App);
        session.unsubscribe(Some([Class::CameraChanged].into_iter().collect()));
        let order: Vec<&str> = drain(&mut session).iter().map(|(name, _)| *name).collect();
        assert_eq!(order, ["document_changed"]);
    }

    #[test]
    fn test_a_requeued_event_goes_out_first_and_a_newer_one_replaces_it() {
        let mut world = World::new();
        let mut session = Session::default();
        session.subscribe(ALL(), false);
        session.baseline(&world.now());
        world.doc.mark_dirty();
        session.observe(&world.now(), Driver::App);
        let held = session.pop_pending().expect("queued");
        world.camera.zoom = 3.0;
        session.observe(&world.now(), Driver::App);
        session.requeue(held);
        let order: Vec<&str> = drain(&mut session).iter().map(|(name, _)| *name).collect();
        assert_eq!(order, ["document_changed", "camera_changed"]);

        world.doc.mark_dirty();
        session.observe(&world.now(), Driver::App);
        let held = session.pop_pending().expect("queued");
        world.doc.mark_dirty();
        session.observe(&world.now(), Driver::App);
        session.requeue(held);
        let events = drain(&mut session);
        assert_eq!(
            events.len(),
            1,
            "the newer document_changed replaced the held one"
        );
        assert_eq!(events[0].1["document_revision"], json!(3));
    }

    #[test]
    fn test_a_new_controller_starts_clean_but_the_revision_carries_on() {
        let mut world = World::new();
        let mut session = Session::default();
        session.subscribe(ALL(), true);
        session.baseline(&world.now());
        world.doc.mark_dirty();
        let before = session.observe(&world.now(), Driver::App);
        session.reset_controller();
        assert!(!session.is_subscribed());
        assert!(drain(&mut session).is_empty());
        world.doc.mark_dirty();
        let after = session.observe(&world.now(), Driver::App);
        assert_eq!(
            after.revision,
            before.revision + 1,
            "the counter is the process's"
        );
    }

    #[test]
    fn test_a_verbose_queue_is_bounded() {
        let mut world = World::new();
        let mut session = Session::default();
        session.subscribe([Class::DocumentChanged].into_iter().collect(), true);
        session.baseline(&world.now());
        for _ in 0..PENDING_CAP + 10 {
            world.doc.mark_dirty();
            session.observe_step(&world.now());
        }
        let events = drain(&mut session);
        assert_eq!(events.len(), PENDING_CAP);
        assert_eq!(
            events.last().expect("events").1["document_revision"],
            json!(PENDING_CAP + 10),
            "the oldest were dropped, not the newest"
        );
    }
}
