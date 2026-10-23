// SPDX-License-Identifier: MPL-2.0

//! Loopback tests: a real socket in a scratch directory, real
//! boundary threads, and a fake main thread that does exactly what
//! the event loop does — wait for a wake, drain, repeat — with the
//! bootstrap commands as its executor. No window and no GPU
//! (TEST_CONVENTIONS §T8), no async harness (§T10): `std` threads and
//! blocking client I/O with read timeouts, so a regression fails
//! rather than hangs.

use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use baumhard::util::test_temp::TempDir;
use serde_json::{json, Value};

use super::*;
use crate::application::ipc::{execute, Executed};

/// The registry with no app attached, as the main thread runs it
/// before the window exists. The loopback tests exercise the
/// transport, so the bootstrap commands, which need no app, are what
/// they send.
fn execute_detached(
    request: &crate::application::ipc::envelope::IpcRequest,
    session: &mut Session,
) -> Executed {
    execute(request, None, session)
}

const MAP: &str = "maps/fixture.mindmap.json";

/// How long any single expected frame may take. Generous: a loaded CI
/// runner is slow, and a timeout here only ever means a failure.
const FRAME_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the paused-reader test stops reading. Long enough for the
/// server's output to fill the socket buffer and the outbound queue,
/// and a small fraction of `STALLED_WRITE_TIMEOUT`, so a slow runner
/// has seconds of slack before a pause could read as a dead client.
const READER_PAUSE: Duration = Duration::from_millis(250);
const _: () = assert!(READER_PAUSE.as_millis() * 8 <= STALLED_WRITE_TIMEOUT.as_millis());

/// A server plus the wake channel its boundary threads signal.
struct Harness {
    _dir: TempDir,
    path: PathBuf,
    wake: mpsc::Sender<()>,
    wakes: Option<Receiver<()>>,
    server: Option<IpcServer>,
    stop: Arc<AtomicBool>,
    main: Option<JoinHandle<IpcServer>>,
}

impl Harness {
    /// A server whose main thread is not running yet — nothing drains
    /// until [`Harness::run_main`], or the test drains by hand.
    fn paused(label: &str) -> Self {
        let dir = TempDir::new(label);
        let path = dir.join("m.sock");
        let (wake, wakes) = mpsc::channel::<()>();
        let server_wake = wake.clone();
        let mut server = IpcServer::start(&path, move || {
            let _ = server_wake.send(());
        })
        .expect("the server starts in a private scratch dir");
        server.publish_map(Some(MAP));
        Harness {
            _dir: dir,
            path,
            wake,
            wakes: Some(wakes),
            server: Some(server),
            stop: Arc::new(AtomicBool::new(false)),
            main: None,
        }
    }

    /// A server with its fake main thread running.
    fn running(label: &str) -> Self {
        let mut harness = Harness::paused(label);
        harness.run_main();
        harness
    }

    /// Start the fake main thread: park until woken, drain, and drain
    /// again without parking while the drain reports backlog — the
    /// `ControlFlow::Wait` / `Poll` choice the event loop makes. It is
    /// purely wake-driven, so a lost wake shows up as a hung test
    /// (bounded by the client's read timeout) rather than being
    /// papered over by polling.
    fn run_main(&mut self) {
        let mut server = self.server.take().expect("not already running");
        let wakes = self.wakes.take().expect("not already running");
        let stop = Arc::clone(&self.stop);
        self.main = Some(thread::spawn(move || {
            while wakes.recv().is_ok() && !stop.load(Ordering::SeqCst) {
                while server.drain(execute_detached).backlog {}
            }
            server
        }));
    }

    fn server(&mut self) -> &mut IpcServer {
        self.server.as_mut().expect("the main thread is not running")
    }

    fn wakes(&self) -> &Receiver<()> {
        self.wakes.as_ref().expect("the main thread is not running")
    }

    /// Stop the fake main thread and shut the server down gracefully.
    fn shutdown(mut self) {
        if let Some(main) = self.main.take() {
            self.stop.store(true, Ordering::SeqCst);
            let _ = self.wake.send(());
            self.server = Some(main.join().expect("the fake main thread does not panic"));
        }
        self.server.take().expect("a server").shutdown();
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(main) = self.main.take() {
            self.stop.store(true, Ordering::SeqCst);
            let _ = self.wake.send(());
            let _ = main.join();
        }
    }
}

/// One client connection with line-oriented helpers.
struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    fn connect(path: &Path) -> Self {
        let stream = UnixStream::connect(path).expect("connect to the server");
        stream
            .set_read_timeout(Some(FRAME_TIMEOUT))
            .expect("read timeout");
        Client {
            reader: BufReader::new(stream.try_clone().expect("clone")),
            writer: stream,
        }
    }

    /// Connect as the controller and return the `hello` greeting.
    /// Retries while the previous controller is still being retired —
    /// its reader clears the busy flag a moment after the disconnect.
    fn attach(path: &Path) -> (Self, Value) {
        let deadline = Instant::now() + FRAME_TIMEOUT;
        loop {
            let mut client = Client::connect(path);
            let first = client.next();
            if first["event"] == json!("hello") {
                return (client, first);
            }
            assert_eq!(first["event"], json!("connection_rejected"), "{first}");
            assert!(
                Instant::now() < deadline,
                "the previous controller was never retired"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn send(&mut self, line: &str) {
        self.writer.write_all(line.as_bytes()).expect("write");
        self.writer.write_all(b"\n").expect("write");
    }

    fn next(&mut self) -> Value {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .expect("a frame within the timeout");
        assert!(
            n > 0,
            "the server closed the connection where a frame was expected"
        );
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("frame is not JSON ({e}): {line:?}"))
    }

    fn at_eof(&mut self) -> bool {
        let mut line = String::new();
        matches!(self.reader.read_line(&mut line), Ok(0))
    }
}

#[test]
fn test_ipc_loopback_greets_with_hello_then_answers_ping() {
    let harness = Harness::running("ipc-hello");
    let (mut client, hello) = Client::attach(&harness.path);
    assert_eq!(hello["data"]["protocol"], json!(1));
    assert_eq!(hello["data"]["app"], json!("mandala"));
    assert_eq!(hello["data"]["pid"], json!(std::process::id()));
    assert_eq!(hello["data"]["map"], json!(MAP));

    client.send(r#"{"id":1,"cmd":"ping"}"#);
    let reply = client.next();
    assert_eq!(reply["id"], json!(1));
    assert_eq!(reply["ok"], json!(true));
    assert!(reply["result"]["now_ms"].is_number());
}

#[test]
fn test_ipc_loopback_wakes_an_idle_main_thread() {
    // The acceptance criterion behind the whole design: the event loop
    // parks, and a request must wake it rather than wait for an
    // unrelated event. Here nothing drains unless a wake arrives.
    let mut harness = Harness::paused("ipc-wake");
    let mut client = Client::connect(&harness.path);
    assert_eq!(client.next()["event"], json!("hello"));
    harness
        .wakes()
        .recv_timeout(FRAME_TIMEOUT)
        .expect("a connection wakes the loop");
    harness.server().drain(execute_detached);

    client.send(r#"{"id":"w","cmd":"ping"}"#);
    harness
        .wakes()
        .recv_timeout(FRAME_TIMEOUT)
        .expect("a request wakes the loop");
    assert_eq!(harness.server().drain(execute_detached), DrainOutcome::default());
    assert_eq!(client.next()["id"], json!("w"));
}

#[test]
fn test_ipc_loopback_answers_garbage_and_the_session_survives() {
    let harness = Harness::running("ipc-garbage");
    let (mut client, _) = Client::attach(&harness.path);

    client.send("this is not json");
    client.send("");
    client.send(&"x".repeat(INBOUND_LINE_CAP_FOR_TESTS + 1));
    client.send(r#"{"cmd":"ping"}"#);
    client.send(r#"{"id":5,"cmd":"ping"}"#);

    let garbage = client.next();
    assert_eq!(garbage["error"]["code"], json!("parse_error"));
    assert_eq!(garbage["error"]["data"]["line"], json!(0));
    let oversized = client.next();
    assert_eq!(oversized["error"]["code"], json!("parse_error"));
    assert_eq!(
        oversized["error"]["data"]["line"],
        json!(2),
        "the blank line still counts"
    );
    let no_id = client.next();
    assert_eq!(no_id["error"]["code"], json!("invalid_params"));
    assert_eq!(no_id["id"], Value::Null);
    let ping = client.next();
    assert_eq!(ping["id"], json!(5));
    assert_eq!(
        ping["ok"],
        json!(true),
        "the stream is still usable after all of it"
    );
}

/// The inbound cap the server enforces, named here so the test above
/// reads as what it means.
const INBOUND_LINE_CAP_FOR_TESTS: usize = crate::application::ipc::envelope::INBOUND_LINE_CAP;

#[test]
fn test_ipc_loopback_refuses_a_second_client_immediately() {
    let harness = Harness::running("ipc-busy");
    let (mut first, _) = Client::attach(&harness.path);

    let mut second = Client::connect(&harness.path);
    assert_eq!(
        second.next(),
        json!({ "event": "connection_rejected", "data": { "reason": "busy" } })
    );
    assert!(
        second.at_eof(),
        "a refused connection is closed after its one frame"
    );

    first.send(r#"{"id":1,"cmd":"ping"}"#);
    assert_eq!(first.next()["id"], json!(1), "the controller is undisturbed");
}

#[test]
fn test_ipc_loopback_a_new_controller_starts_a_fresh_session() {
    let harness = Harness::running("ipc-reconnect");
    let (mut first, _) = Client::attach(&harness.path);
    first.send(r#"{"id":1,"cmd":"ping"}"#);
    assert_eq!(first.next()["id"], json!(1));
    drop(first);

    let (mut second, hello) = Client::attach(&harness.path);
    assert_eq!(
        hello["event"],
        json!("hello"),
        "a reconnecting client is greeted afresh"
    );
    second.send(r#"{"id":2,"cmd":"ping"}"#);
    assert_eq!(second.next()["id"], json!(2));
}

#[test]
fn test_ipc_loopback_half_close_still_gets_every_reply() {
    // `printf '{"id":1,"cmd":"ping"}\n' | nc -U sock` closes its write
    // half after the request. The disconnect must not overtake the
    // requests that preceded it.
    let harness = Harness::running("ipc-half-close");
    let (mut client, _) = Client::attach(&harness.path);
    for id in 0..3 {
        client.send(&format!(r#"{{"id":{id},"cmd":"ping"}}"#));
    }
    client.writer.shutdown(Shutdown::Write).expect("half-close");
    for id in 0..3 {
        assert_eq!(client.next()["id"], json!(id));
    }
    assert!(client.at_eof(), "the server closes once every reply is written");
}

#[test]
fn test_ipc_loopback_answers_a_pipelined_burst_in_full_and_in_order() {
    // The main thread executes a burst as fast as it can, while the
    // writer sends whatever is queued in one write per wakeup. Whether
    // the outbound queue fills depends on how the writer thread is
    // scheduled; either way the client reading normally must get every
    // reply — paused, never dropped. (With one write per frame, this
    // burst filled the queue hundreds of times.)
    let harness = Harness::running("ipc-burst");
    let (mut client, _) = Client::attach(&harness.path);
    let burst: String = (0..1000)
        .map(|id| format!("{{\"id\":{id},\"cmd\":\"ping\"}}\n"))
        .collect();
    client
        .writer
        .write_all(burst.as_bytes())
        .expect("write the burst");
    for id in 0..1000 {
        let reply = client.next();
        assert_eq!(reply["id"], json!(id));
        assert_eq!(reply["ok"], json!(true));
    }
}

#[test]
fn test_ipc_loopback_a_client_that_pauses_reading_is_paused_not_dropped() {
    // Enough output to fill the kernel's socket buffer and then the
    // outbound queue while the client is not reading: the main thread
    // pauses this controller, and resumes when the client reads.
    let harness = Harness::running("ipc-slow-reader");
    let (mut client, _) = Client::attach(&harness.path);
    let writer = client.writer.try_clone().expect("clone");
    let sender = thread::spawn(move || {
        let mut writer = writer;
        for id in 0..1000 {
            writeln!(writer, r#"{{"id":{id},"cmd":"describe"}}"#).expect("write");
        }
    });
    thread::sleep(READER_PAUSE);
    for id in 0..1000 {
        assert_eq!(client.next()["id"], json!(id), "nothing lost while paused");
    }
    sender.join().expect("sender");
}

#[test]
fn test_ipc_loopback_drops_a_client_that_stops_reading() {
    let harness = Harness::running("ipc-dead-reader");
    let (client, _) = Client::attach(&harness.path);
    let mut writer = client.writer.try_clone().expect("clone");
    // Never read. Keep asking for large replies until the server gives
    // up on the connection, which ends these writes too.
    let sender = thread::spawn(move || {
        for id in 0.. {
            if writeln!(writer, r#"{{"id":{id},"cmd":"describe"}}"#).is_err() {
                return;
            }
        }
    });
    // The dead controller is retired, so a new one can attach.
    let (mut next, _) = Client::attach(&harness.path);
    next.send(r#"{"id":"after","cmd":"ping"}"#);
    assert_eq!(next.next()["id"], json!("after"));
    drop(client);
    sender.join().expect("sender");
}

#[test]
fn test_ipc_loopback_overload_answers_overloaded_and_loses_nothing_queued() {
    let mut harness = Harness::paused("ipc-overload");
    let mut client = Client::connect(&harness.path);
    assert_eq!(client.next()["event"], json!("hello"));
    let burst: String = (0..=REQUEST_QUEUE_CAP)
        .map(|id| format!("{{\"id\":{id},\"cmd\":\"ping\"}}\n"))
        .collect();
    client
        .writer
        .write_all(burst.as_bytes())
        .expect("write the burst");

    // Nothing is draining, so the reader answers the request past the
    // cap itself — the only reply that can arrive before a drain.
    let refused = client.next();
    assert_eq!(refused["id"], json!(REQUEST_QUEUE_CAP));
    assert_eq!(refused["error"]["code"], json!("overloaded"));

    harness.run_main();
    let _ = harness.wake.send(());
    for id in 0..REQUEST_QUEUE_CAP {
        assert_eq!(client.next()["id"], json!(id), "every queued request is answered");
    }
}

#[test]
fn test_ipc_loopback_hello_reports_the_published_map() {
    let mut harness = Harness::paused("ipc-map");
    let (first, hello) = Client::attach(&harness.path);
    assert_eq!(hello["data"]["map"], json!(MAP));
    drop(first);

    harness.server().publish_map(None);
    let (second, hello) = Client::attach(&harness.path);
    assert_eq!(hello["data"]["map"], Value::Null, "null when no file is bound");
    drop(second);

    harness.server().publish_map(Some("other.mindmap.json"));
    let (_third, hello) = Client::attach(&harness.path);
    assert_eq!(
        hello["data"]["map"],
        json!("other.mindmap.json"),
        "a load reaches the next greeting"
    );
}

#[test]
fn test_ipc_loopback_shutdown_says_so_and_removes_the_socket() {
    let harness = Harness::running("ipc-shutdown");
    let (mut client, _) = Client::attach(&harness.path);
    client.send(r#"{"id":1,"cmd":"ping"}"#);
    assert_eq!(client.next()["id"], json!(1));

    let path = harness.path.clone();
    harness.shutdown();
    assert_eq!(client.next(), json!({ "event": "shutdown", "data": {} }));
    assert!(client.at_eof(), "the connection closes after `shutdown`");
    assert!(!path.exists(), "a graceful exit removes the socket file");
}

#[test]
fn test_ipc_loopback_shutdown_reaches_a_controller_paused_mid_burst() {
    // The window closes while the controller is paused on a full queue:
    // the reply it holds and the `shutdown` event both still arrive,
    // after every reply already queued, so a client reading normally
    // can tell "app exited" from "connection died".
    let harness = Harness::running("ipc-shutdown-paused");
    let (mut client, _) = Client::attach(&harness.path);
    let burst: String = (0..1000)
        .map(|id| format!("{{\"id\":{id},\"cmd\":\"describe\"}}\n"))
        .collect();
    client
        .writer
        .write_all(burst.as_bytes())
        .expect("write the burst");
    thread::sleep(READER_PAUSE);

    let exiting = thread::spawn(move || harness.shutdown());
    let mut answered = 0;
    loop {
        let frame = client.next();
        if frame["event"] == json!("shutdown") {
            break;
        }
        assert_eq!(
            frame["id"],
            json!(answered),
            "replies arrive in order, none skipped"
        );
        answered += 1;
    }
    assert!(answered > 0, "the burst was under way when the app exited");
    assert!(client.at_eof(), "the connection closes after `shutdown`");
    exiting.join().expect("shutdown does not panic");
}

#[test]
fn test_controller_offer_pauses_on_a_full_queue_and_reports_a_closed_one() {
    let (outbound, frames) = mpsc::sync_channel::<Outbound>(1);
    let (hangup, _peer) = UnixStream::pair().expect("pair");
    let (_flushed_tx, flushed) = mpsc::channel();
    let controller = Controller {
        generation: 1,
        outbound,
        waiting: Arc::new(AtomicBool::new(false)),
        events_queued: Arc::new(AtomicUsize::new(0)),
        hangup,
        flushed,
    };
    let frame = || Outbound::Frame(Frame::Event(crate::application::ipc::envelope::Event::shutdown()));

    assert!(matches!(controller.offer(frame()), Offer::Queued));
    assert!(
        matches!(controller.offer(frame()), Offer::Full(_)),
        "the queue holds one"
    );
    assert!(
        controller.waiting.load(Ordering::SeqCst),
        "a pause raises the flag the writer wakes on"
    );
    drop(frames);
    assert!(matches!(controller.offer(frame()), Offer::Closed));
}

// ── events (IPC-14) ──────────────────────────────────────────────────

use crate::application::ipc::events::fixture::EventsApp;

/// Run one drain the way the event loop does with an app attached:
/// every request through the registry against `app`.
fn drain_with(harness: &mut Harness, app: &mut EventsApp) -> DrainOutcome {
    harness
        .server()
        .drain(|request, session| execute(request, Some(app), session))
}

/// Block until the boundary threads have woken the loop, then drain.
fn wake_and_drain(harness: &mut Harness, app: &mut EventsApp) {
    harness
        .wakes()
        .recv_timeout(FRAME_TIMEOUT)
        .expect("the boundary threads wake the loop");
    drain_with(harness, app);
}

/// A connected controller of a server whose main thread is driven by
/// hand, past its `hello`.
fn attached(label: &str) -> (Harness, Client, EventsApp) {
    let mut harness = Harness::paused(label);
    let mut app = EventsApp::new();
    let mut client = Client::connect(&harness.path);
    assert_eq!(client.next()["event"], json!("hello"));
    wake_and_drain(&mut harness, &mut app);
    (harness, client, app)
}

#[test]
fn test_ipc_loopback_an_event_follows_the_reply_to_the_command_that_caused_it() {
    let (mut harness, mut client, mut app) = attached("ipc-event-order");
    client.send(r#"{"id":1,"cmd":"events.subscribe","classes":["document_changed"]}"#);
    wake_and_drain(&mut harness, &mut app);
    assert_eq!(client.next()["result"]["subscribed"], json!(["document_changed"]));

    client.send(r#"{"id":2,"cmd":"act.console","line":"any"}"#);
    wake_and_drain(&mut harness, &mut app);
    let reply = client.next();
    assert_eq!(reply["id"], json!(2));
    assert_eq!(reply["revision"], json!(1));
    assert_eq!(
        client.next(),
        json!({
            "event": "document_changed",
            "data": { "revision": 1, "document_revision": 1, "source": "ipc" }
        }),
        "the event is behind the reply that announces it"
    );

    client.send(r#"{"id":3,"cmd":"ping"}"#);
    wake_and_drain(&mut harness, &mut app);
    assert_eq!(client.next()["id"], json!(3), "nothing else was waiting");
    assert_eq!(
        harness
            .server()
            .controller
            .as_ref()
            .expect("attached")
            .events_queued
            .load(Ordering::SeqCst),
        0,
        "the outbound thread gave the event's lane slot back"
    );
}

#[test]
fn test_ipc_loopback_a_polling_client_is_never_sent_an_event() {
    let (mut harness, mut client, mut app) = attached("ipc-event-silent");
    // One request per drain: two sent back to back can arrive on
    // either side of the drain, and the second would then wait for a
    // wake nothing is going to follow with.
    client.send(r#"{"id":1,"cmd":"act.console","line":"any"}"#);
    wake_and_drain(&mut harness, &mut app);
    let first = client.next();
    client.send(r#"{"id":2,"cmd":"ping"}"#);
    wake_and_drain(&mut harness, &mut app);
    let second = client.next();
    assert_eq!((&first["id"], &second["id"]), (&json!(1), &json!(2)));
    assert_eq!(
        second["document_revision"],
        json!(1),
        "the poller sees the revision instead"
    );
}

#[test]
fn test_ipc_loopback_a_change_the_user_made_is_reported_as_theirs() {
    let (mut harness, mut client, mut app) = attached("ipc-event-user");
    client.send(r#"{"id":1,"cmd":"events.subscribe","classes":["document_changed","camera_changed"]}"#);
    wake_and_drain(&mut harness, &mut app);
    client.next();

    // What `about_to_wait` does between the input drain and the IPC
    // drain while something is subscribed.
    assert!(harness.server().is_subscribed());
    app.doc
        .with_source(crate::application::document::ChangeSource::Action, |doc| {
            doc.mark_dirty()
        });
    app.camera.zoom = 2.0;
    let now = crate::application::ipc::CommandTarget::observed(&app).expect("observable");
    harness.server().observe(&now);
    drain_with(&mut harness, &mut app);

    let first = client.next();
    let second = client.next();
    assert_eq!(first["event"], json!("document_changed"));
    assert_eq!(first["data"]["source"], json!("action"));
    assert_eq!(second["event"], json!("camera_changed"));
    assert_eq!(
        second["data"]["revision"], first["data"]["revision"],
        "one observation"
    );
}

#[test]
fn test_ipc_loopback_a_new_controller_starts_unsubscribed_and_the_revision_carries_on() {
    let (mut harness, mut first, mut app) = attached("ipc-event-reconnect");
    first.send(r#"{"id":1,"cmd":"events.subscribe","classes":["document_changed"]}"#);
    wake_and_drain(&mut harness, &mut app);
    first.next();
    first.send(r#"{"id":2,"cmd":"act.console","line":"any"}"#);
    wake_and_drain(&mut harness, &mut app);
    let before = first.next()["revision"].as_u64().expect("revision");
    drop(first);
    // The reader notices the close and wakes the loop with the
    // disconnect.
    wake_and_drain(&mut harness, &mut app);

    let deadline = Instant::now() + FRAME_TIMEOUT;
    let mut second = loop {
        let mut client = Client::connect(&harness.path);
        if client.next()["event"] == json!("hello") {
            break client;
        }
        assert!(
            Instant::now() < deadline,
            "the first controller was never retired"
        );
        thread::sleep(Duration::from_millis(10));
    };
    wake_and_drain(&mut harness, &mut app);
    assert!(
        !harness.server().is_subscribed(),
        "subscriptions do not outlive their connection"
    );

    second.send(r#"{"id":3,"cmd":"act.console","line":"any"}"#);
    wake_and_drain(&mut harness, &mut app);
    let reply = second.next();
    assert_eq!(
        reply["revision"],
        json!(before + 1),
        "the counter is the process's"
    );
    second.send(r#"{"id":4,"cmd":"ping"}"#);
    wake_and_drain(&mut harness, &mut app);
    assert_eq!(
        second.next()["id"],
        json!(4),
        "no event reached the unsubscribed successor"
    );
}

#[test]
fn test_events_a_full_queue_holds_them_back_and_a_newer_one_replaces_an_older() {
    use std::os::unix::net::UnixStream;

    let mut harness = Harness::paused("ipc-event-full");
    let (queue, room) = mpsc::sync_channel(1);
    let (hangup, _peer) = UnixStream::pair().expect("a socket pair");
    let (_flushed_tx, flushed) = mpsc::channel();
    let waiting = Arc::new(AtomicBool::new(false));
    let server = harness.server();
    server.controller = Some(Controller {
        generation: 1,
        outbound: queue.clone(),
        waiting: Arc::clone(&waiting),
        events_queued: Arc::new(AtomicUsize::new(0)),
        hangup,
        flushed,
    });
    // The controller's queue is full of something it has not read.
    queue
        .try_send(Outbound::Frame(Frame::Event(Event::shutdown())))
        .expect("room for one");

    let mut app = EventsApp::new();
    server.session.subscribe(
        [crate::application::ipc::events::Class::DocumentChanged]
            .into_iter()
            .collect(),
        false,
    );
    server
        .session
        .baseline(&crate::application::ipc::CommandTarget::observed(&app).expect("observable"));
    for _ in 0..3 {
        app.doc.mark_dirty();
        server.observe(&crate::application::ipc::CommandTarget::observed(&app).expect("observable"));
        server.deliver_events();
    }
    assert!(
        waiting.load(Ordering::SeqCst),
        "a held-back event asks the outbound thread to wake the loop"
    );

    // The reader catches up; the next delivery gets the newest only.
    room.recv().expect("the filler frame");
    server.deliver_events();
    match room.try_recv().expect("the held event was delivered") {
        Outbound::Event(event) => {
            let value = serde_json::to_value(&event).expect("serializes");
            assert_eq!(value["data"]["document_revision"], json!(3), "the newest, once");
        }
        _ => panic!("expected an event frame"),
    }
    assert!(room.try_recv().is_err(), "the two older ones were coalesced away");
}

#[test]
fn test_events_wait_behind_a_reply_the_controller_has_not_taken() {
    let mut harness = Harness::paused("ipc-event-behind-reply");
    let (queue, room) = mpsc::sync_channel(8);
    let (hangup, _peer) = UnixStream::pair().expect("a socket pair");
    let (_flushed_tx, flushed) = mpsc::channel();
    let server = harness.server();
    server.controller = Some(Controller {
        generation: 1,
        outbound: queue,
        waiting: Arc::new(AtomicBool::new(false)),
        events_queued: Arc::new(AtomicUsize::new(0)),
        hangup,
        flushed,
    });
    let mut app = EventsApp::new();
    server.session.subscribe(
        [crate::application::ipc::events::Class::DocumentChanged]
            .into_iter()
            .collect(),
        false,
    );
    server
        .session
        .baseline(&crate::application::ipc::CommandTarget::observed(&app).expect("observable"));
    app.doc.mark_dirty();
    server.observe(&crate::application::ipc::CommandTarget::observed(&app).expect("observable"));
    server.stalled = Some(Outbound::Frame(Frame::Reply(
        crate::application::ipc::envelope::Reply::ok(json!(9), json!({})),
    )));

    server.deliver_events();
    assert!(
        room.try_recv().is_err(),
        "an event never overtakes a reply the controller is still owed"
    );
    assert!(server.session.pop_pending().is_some(), "and it is kept, not lost");
}

#[test]
fn test_events_have_a_lane_of_their_own_and_never_take_a_replys_slot() {
    use crate::application::ipc::envelope::Reply;

    let (outbound, frames) = mpsc::sync_channel::<Outbound>(OUTBOUND_QUEUE_CAP + EVENT_QUEUE_CAP);
    let (hangup, _peer) = UnixStream::pair().expect("pair");
    let (_flushed_tx, flushed) = mpsc::channel();
    let controller = Controller {
        generation: 1,
        outbound,
        waiting: Arc::new(AtomicBool::new(false)),
        events_queued: Arc::new(AtomicUsize::new(0)),
        hangup,
        flushed,
    };
    let event = || Event::change("camera_changed", json!({ "revision": 1 }));

    for _ in 0..EVENT_QUEUE_CAP {
        assert!(matches!(controller.offer_event(event()), Offer::Queued));
    }
    assert!(
        matches!(controller.offer_event(event()), Offer::Full(Outbound::Event(_))),
        "the lane is full, though the queue is not"
    );
    assert!(
        controller.waiting.load(Ordering::SeqCst),
        "a refused event asks the outbound thread to wake the loop"
    );
    for _ in 0..OUTBOUND_QUEUE_CAP {
        let reply = Outbound::Frame(Frame::Reply(Reply::ok(json!(1), json!({}))));
        assert!(
            matches!(controller.offer(reply), Offer::Queued),
            "a full event lane leaves every reply slot free"
        );
    }

    // The outbound thread takes an event and gives its slot back.
    match frames.recv().expect("the first event") {
        Outbound::Event(_) => {
            controller.events_queued.fetch_sub(1, Ordering::SeqCst);
        }
        _ => panic!("events were queued first"),
    }
    drop(frames);
    assert!(
        matches!(controller.offer_event(event()), Offer::Closed),
        "a closed connection is reported, and it does not leak a slot"
    );
    assert_eq!(
        controller.events_queued.load(Ordering::SeqCst),
        EVENT_QUEUE_CAP - 1,
        "a failed offer gives its slot back"
    );
}
