// SPDX-License-Identifier: MPL-2.0

//! The event stream and the revision counters (`format/ipc.md`
//! §`events`).
//!
//! **Observation.** A [`Tracker`] is handed a read-only [`Observed`]
//! view of the app at the points the event loop already passes through,
//! remembers what it saw last, and reports the difference. A keystroke,
//! a console line and an IPC command are therefore all seen the same
//! way, and a 200-step macro is one difference. The document's own
//! revision (`Document::revision`) is what makes "the model changed" a
//! comparison of two integers rather than a walk, and the change records
//! the view drained since the last iteration ([`Observed::changes`],
//! `view::MapView::pending_feed`) say who made it. Events built from the
//! records themselves — what changed, element by element — are V1-E2's.
//!
//! **Two counters.** `document_revision` is the document's: committed
//! model changes, untouched by a selection or the camera; a replaced
//! document starts its own, which `document_replaced` announces with the
//! new document's generation. `revision` is this module's: it advances
//! once per observation in which *any* event class had something to
//! report, so a view-only change (a zoom, a selection) moves it while the
//! document's stays put, and it is monotonic per process. An observation
//! is the unit: two changes between two observations advance `revision`
//! once.
//!
//! **Delivery** is the main thread's: observations queue
//! [`Event`]s in a [`Session`], and the transport offers them to the
//! controller after the replies of the drain that produced them
//! (`transport::IpcServer::drain`). Events never pause execution. A
//! queue the controller is not draining is coalesced to the newest
//! event per class rather than grown.

use std::collections::VecDeque;

use serde_json::{json, Value};

use baumhard::arena::NodeId;
use baumhard::document::writer::feed::ChangeRecord;
use baumhard::gfx_structs::camera::Camera2D;

use crate::application::app::InteractionMode;
use crate::application::document::MindMapDocument;

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
}

impl Class {
    /// Every class, in `describe` order (by name).
    pub(crate) const ALL: [Class; 5] = [
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
        }
    }

    /// What `describe` says the class means.
    pub(crate) const fn doc(self) -> &'static str {
        match self {
            Class::DocumentChanged => {
                "A committed document change: data carries revision, document_revision and the source (action, console, ipc, input, edit, verb, undo or redo)."
            }
            Class::DocumentReplaced => {
                "An open or new replaced the document: data carries revision, document_revision, generation and the new path."
            }
            Class::SelectionChanged => "The selection changed: data carries revision and the selection, as query.selection reports it.",
            Class::CameraChanged => "The camera moved or the surface resized: data carries revision and the camera, as query.camera reports it.",
            Class::ModeChanged => "The interaction mode changed: data carries revision and the mode, as query.mode reports its kind.",
        }
    }

    /// The class named `word`, if any.
    pub(crate) fn parse(word: &str) -> Option<Class> {
        Class::ALL.into_iter().find(|class| class.name() == word)
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
    /// The document's change records the view has drained since the
    /// event loop last cleared them (`view::MapView::pending_feed`),
    /// oldest first: who made the changes an observation sees.
    pub(crate) changes: &'a [ChangeRecord],
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
    /// The app's own input handlers, whose changes carry their source.
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

/// What the last observation saw.
struct Seen {
    generation: u64,
    document_revision: u64,
    selection: Vec<NodeId>,
    camera: CameraKey,
    mode: InteractionMode,
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
            document_revision: now.document.map_or(0, |doc| doc.revision().0),
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
    /// Cost: integer, selection, camera and mode comparisons. Allocates
    /// only for a class that differs.
    pub(crate) fn observe(
        &mut self,
        now: &Observed<'_>,
        wanted: Classes,
        driver: Driver,
    ) -> (Revisions, Vec<Change>) {
        let generation = now.document.map_or(0, MindMapDocument::generation);
        let document_revision = now.document.map_or(0, |doc| doc.revision().0);
        let selection: Vec<NodeId> = now
            .document
            .map_or_else(Vec::new, |doc| doc.selection.iter().collect());
        let camera = CameraKey::of(now.camera);

        let Some(seen) = self.seen.as_mut() else {
            self.seen = Some(Seen {
                generation,
                document_revision,
                selection,
                camera,
                mode: now.mode.clone(),
            });
            return (self.revisions(now), Vec::new());
        };

        let replaced = seen.generation != generation;
        let document = !replaced && seen.document_revision != document_revision;
        let selected = replaced || seen.selection != selection;
        let moved = seen.camera != camera;
        let moded = seen.mode != *now.mode;
        let any = replaced || document || selected || moved || moded;
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
                Driver::App => now.changes.last().map_or("input", |record| record.source.name()),
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
                object(json!({
                    "selection": now.document.map_or_else(
                        || json!({"count": 0, "elements": []}),
                        MindMapDocument::ipc_selection_json,
                    )
                })),
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
        seen.generation = generation;
        seen.document_revision = document_revision;
        if selected {
            seen.selection = selection;
        }
        seen.camera = camera;
        if moded {
            seen.mode = now.mode.clone();
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
    pub(crate) fn resolve_wait(&mut self, now: web_time::Instant, app: Settling) -> Option<Reply> {
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
                self.pending.retain(|queued| queued.class != class);
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
        if !self.subscriptions.verbose && self.pending.iter().any(|waiting| waiting.class == queued.class) {
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
    //! renderer, which only the GPU-backed tier of `TEST_CONVENTIONS`
    //! §T8 builds (`app/tests_gpu.rs` drives the real one).

    use serde_json::{json, Value};

    use super::Observed;
    use crate::application::app::InteractionMode;
    use crate::application::console::{ExecResult, LineOutcome};
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
                doc: MindMapDocument::new_blank(None, 50),
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
            self.doc.create_root_for_test();
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
                self.doc.create_root_for_test();
                after_step(Observed {
                    document: Some(&self.doc),
                    changes: &[],
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
                changes: &[],
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
    use baumhard::document::writer::{ChangeSource, EditRequest};

    /// A world to observe: a document, the change records drained from
    /// it, a camera and a mode, that a test edits between observations.
    struct World {
        doc: MindMapDocument,
        changes: Vec<ChangeRecord>,
        camera: Camera2D,
        mode: InteractionMode,
    }

    impl World {
        fn new() -> Self {
            World {
                doc: MindMapDocument::new_blank(None, 50),
                changes: Vec::new(),
                camera: Camera2D::new(800, 600),
                mode: InteractionMode::Default,
            }
        }

        /// One committed change, made by `source`, its records drained
        /// as the view drains them.
        fn change(&mut self, source: ChangeSource) {
            let steps = baumhard::document::mutation::compose::create_root(self.doc.doc(), glam::Vec2::ZERO)
                .expect("composes");
            self.doc
                .apply(
                    EditRequest {
                        steps,
                        label: "test".into(),
                    },
                    source,
                )
                .expect("applies");
            self.changes.extend(self.doc.take_changes());
        }

        fn now(&self) -> Observed<'_> {
            Observed {
                document: Some(&self.doc),
                changes: &self.changes,
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
        world.change(ChangeSource::Console);
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
        world.change(ChangeSource::Console);
        let (_, changes) = tracker.observe(&world.now(), ALL(), Driver::Ipc);
        assert_eq!(find(&changes, Class::DocumentChanged)["source"], json!("ipc"));
    }

    #[test]
    fn test_a_class_nobody_asked_for_still_moves_the_revision_but_reports_nothing() {
        let mut world = World::new();
        let mut tracker = baselined(&world);
        world.change(ChangeSource::Action);
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
        world.change(ChangeSource::Action);
        world.change(ChangeSource::Action);
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
        world.change(ChangeSource::Action);
        let mut tracker = baselined(&world);
        let node = crate::application::document::selection::every_node(world.doc.doc())
            .next()
            .expect("the node just made");
        world.doc.selection.select_only(node);
        let (revisions, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert_eq!(names(&changes), ["selection_changed"]);
        let ident = crate::application::view::inspect::ident_of(world.doc.doc(), node)
            .expect("a node")
            .0;
        assert_eq!(
            find(&changes, Class::SelectionChanged)["selection"],
            json!({ "count": 1, "elements": [{ "node": ident }] })
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
    fn test_a_camera_change_carries_its_projection() {
        let mut world = World::new();
        let mut tracker = baselined(&world);
        world.camera.zoom = 2.0;
        world.camera.position = glam::Vec2::new(5.0, -7.0);
        let (_, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert_eq!(names(&changes), ["camera_changed"]);
        assert_eq!(
            find(&changes, Class::CameraChanged)["camera"],
            camera_json(&world.camera)
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
    fn test_a_replaced_document_reports_its_path_and_generation() {
        let mut world = World::new();
        world.change(ChangeSource::Action);
        world.change(ChangeSource::Action);
        let mut tracker = baselined(&world);
        let next = MindMapDocument::new_blank(Some("maps/next.mandala.json".into()), 50);
        let generation = next.generation();
        assert_ne!(generation, world.doc.generation());
        world.doc = next;
        world.changes.clear();
        let (revisions, changes) = tracker.observe(&world.now(), ALL(), Driver::App);
        assert_eq!(
            names(&changes),
            ["document_replaced", "selection_changed"],
            "a replaced document is not a change of the one before it"
        );
        assert_eq!(
            find(&changes, Class::DocumentReplaced),
            &json!({
                "revision": 1,
                "document_revision": 0,
                "generation": generation,
                "path": "maps/next.mandala.json",
            })
        );
        assert_eq!(
            revisions.document_revision, 0,
            "the new document counts its own changes"
        );
        assert_eq!(revisions.revision, 1, "the stream's counter carries on");
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
            world.change(ChangeSource::Action);
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
            world.change(ChangeSource::Action);
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
        world.change(ChangeSource::Action);
        session.observe_step(&world.now());
        assert!(drain(&mut session).is_empty());
    }

    #[test]
    fn test_unsubscribing_drops_the_events_still_waiting() {
        let mut world = World::new();
        let mut session = Session::default();
        session.subscribe(ALL(), false);
        session.baseline(&world.now());
        world.change(ChangeSource::Action);
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
        world.change(ChangeSource::Action);
        session.observe(&world.now(), Driver::App);
        let held = session.pop_pending().expect("queued");
        world.camera.zoom = 3.0;
        session.observe(&world.now(), Driver::App);
        session.requeue(held);
        let order: Vec<&str> = drain(&mut session).iter().map(|(name, _)| *name).collect();
        assert_eq!(order, ["document_changed", "camera_changed"]);

        world.change(ChangeSource::Action);
        session.observe(&world.now(), Driver::App);
        let held = session.pop_pending().expect("queued");
        world.change(ChangeSource::Action);
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
        world.change(ChangeSource::Action);
        let before = session.observe(&world.now(), Driver::App);
        session.reset_controller();
        assert!(!session.is_subscribed());
        assert!(drain(&mut session).is_empty());
        world.change(ChangeSource::Action);
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
            world.change(ChangeSource::Action);
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
