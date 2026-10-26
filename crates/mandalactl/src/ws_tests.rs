// SPDX-License-Identifier: MPL-2.0

//! Whole invocations over the WebSocket transport, against a relay
//! and a stand-in for the browser app on real loopback sockets:
//! the relay's rules end to end, the client's `--ws` path, and the
//! screenshot the client writes for a browser app. Nothing is mocked
//! (TEST_CONVENTIONS §T10): the stand-in app is a `tungstenite` client
//! in a thread that speaks the protocol's frames, so what runs is the
//! real handshake, framing, deadlines and reads on both sides of the
//! relay.

use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use tungstenite::{Message, WebSocket};

use crate::bridge::{self, Role};
use crate::run::Env;
use crate::testing::{hello, invoke, invoke_env, ok, Outcome};
use crate::wire::{BRIDGE_ATTACHED, BRIDGE_DETACHED};
use crate::Exit;

/// A relay on a fresh loopback port, serving in a background thread
/// for the rest of the process.
struct Relay {
    address: std::net::SocketAddr,
    token: String,
}

impl Relay {
    fn start() -> Relay {
        let listener = bridge::bind("127.0.0.1:0").expect("loopback binds");
        let address = listener.local_addr().expect("bound");
        let token = "t3st-t0ken".to_string();
        let serving = token.clone();
        thread::spawn(move || bridge::serve(listener, serving));
        Relay { address, token }
    }

    fn url(&self, role: Role) -> String {
        bridge::url_for(self.address, role, &self.token)
    }

    /// `mandalactl --ws <controller url> <line>`.
    fn invoke(&self, line: &str) -> Outcome {
        invoke(&format!("--ws {} {line}", self.url(Role::Controller)), None, "")
    }
}

/// The browser app, as the relay sees it: a WebSocket client on the
/// app role that greets each attached controller and answers its
/// requests with `answer`'s frames.
struct FakeApp {
    seen: Arc<Mutex<Vec<Value>>>,
    attached: Arc<Mutex<u32>>,
}

impl FakeApp {
    fn connect(relay: &Relay, answer: impl Fn(&Value) -> Vec<Value> + Send + 'static) -> FakeApp {
        let (mut socket, _) = connect_until_up(&relay.url(Role::App));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let attached = Arc::new(Mutex::new(0));
        let (log, count) = (Arc::clone(&seen), Arc::clone(&attached));
        thread::spawn(move || loop {
            let text = match socket.read() {
                Ok(Message::Text(text)) => text.as_str().to_string(),
                Ok(Message::Close(_)) | Err(_) => return,
                Ok(_) => continue,
            };
            if text == BRIDGE_ATTACHED {
                *count.lock().expect("not poisoned") += 1;
                send(&mut socket, &hello());
                continue;
            }
            if text == BRIDGE_DETACHED {
                continue;
            }
            let request: Value = serde_json::from_str(&text).expect("the client sends JSON");
            log.lock().expect("not poisoned").push(request.clone());
            for frame in answer(&request) {
                send(&mut socket, &frame);
            }
        });
        FakeApp { seen, attached }
    }

    fn requests(&self) -> Vec<Value> {
        self.seen.lock().expect("not poisoned").clone()
    }

    fn attachments(&self) -> u32 {
        *self.attached.lock().expect("not poisoned")
    }
}

fn send(socket: &mut WebSocket<TcpStream>, frame: &Value) {
    socket
        .send(Message::Text(frame.to_string().into()))
        .expect("the relay takes the frame");
}

/// Connect to the relay, retrying briefly: the listener thread may not
/// have reached `accept` yet.
fn connect_until_up(url: &str) -> (WebSocket<TcpStream>, tungstenite::handshake::client::Response) {
    let mut last = None;
    for _ in 0..50 {
        let (host, port) = url_host_port(url);
        if let Ok(stream) = TcpStream::connect((host.as_str(), port)) {
            match tungstenite::client(url, stream) {
                Ok(connected) => return connected,
                Err(e) => last = Some(format!("{e}")),
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("the relay never answered at {url}: {last:?}");
}

fn url_host_port(url: &str) -> (String, u16) {
    let rest = url.strip_prefix("ws://").expect("a ws url");
    let authority = rest.split('/').next().expect("an authority");
    let (host, port) = authority.rsplit_once(':').expect("host:port");
    (host.to_string(), port.parse().expect("a port"))
}

/// An app that answers everything `ok:true` with `result`.
fn app_always(relay: &Relay, result: Value) -> FakeApp {
    FakeApp::connect(relay, move |request| vec![ok(&request["id"], result.clone())])
}

/// Wait until the relay has told the app it is attached `n` times.
fn wait_attachments(app: &FakeApp, n: u32) {
    for _ in 0..100 {
        if app.attachments() >= n {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("the app was attached {} times, not {n}", app.attachments());
}

// ---- the relay's rules, end to end -----------------------------------------

#[test]
fn test_a_command_crosses_the_relay_and_its_reply_comes_back() {
    let relay = Relay::start();
    let app = app_always(&relay, json!({"now_ms": 12.5}));
    let outcome = relay.invoke("ping");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_eq!(outcome.out, "now_ms: 12.5\n");
    assert_eq!(app.requests(), vec![json!({"id": 1, "cmd": "ping"})]);
    assert_eq!(app.attachments(), 1, "one controller, one greeting");
}

#[test]
fn test_each_invocation_is_a_fresh_controller_greeted_afresh() {
    let relay = Relay::start();
    let app = app_always(&relay, json!({}));
    for _ in 0..3 {
        let outcome = relay.invoke("ping");
        assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    }
    wait_attachments(&app, 3);
    assert_eq!(app.requests().len(), 3);
}

#[test]
fn test_a_controller_before_the_page_is_refused_with_the_remedy() {
    let relay = Relay::start();
    let outcome = relay.invoke("ping");
    assert_eq!(outcome.exit, Exit::TransportError, "{}", outcome.err);
    assert!(
        outcome.err.contains("no_app") && outcome.err.contains("?ipc="),
        "{}",
        outcome.err
    );
    assert_eq!(outcome.out, "");
}

#[test]
fn test_a_wrong_token_is_refused_at_the_handshake() {
    let relay = Relay::start();
    let _app = app_always(&relay, json!({}));
    let wrong = format!("ws://{}/controller/not-the-token", relay.address);
    let outcome = invoke(&format!("--ws {wrong} ping"), None, "");
    assert_eq!(outcome.exit, Exit::TransportError, "{}", outcome.err);
    assert!(
        outcome.err.contains("refused the WebSocket handshake") && outcome.err.contains("403"),
        "{}",
        outcome.err
    );
}

#[test]
fn test_a_second_controller_is_busy_while_the_first_holds_the_slot() {
    let relay = Relay::start();
    let _app = app_always(&relay, json!({}));
    // The first controller: a raw tungstenite client that stays attached.
    let (mut held, _) = connect_until_up(&relay.url(Role::Controller));
    let greeting = held.read().expect("the greeting");
    assert!(
        matches!(greeting, Message::Text(ref t) if t.as_str().contains("\"hello\"")),
        "{greeting:?}"
    );
    let outcome = invoke(
        &format!("--ws {} --timeout 1 ping", relay.url(Role::Controller)),
        None,
        "",
    );
    assert_eq!(outcome.exit, Exit::TransportError, "{}", outcome.err);
    assert!(
        outcome.err.contains("busy") && outcome.err.contains("another client"),
        "{}",
        outcome.err
    );
    held.close(None).expect("closes");
}

#[test]
fn test_the_page_leaving_shuts_its_controller_down() {
    let relay = Relay::start();
    // An app that greets and then drops its socket on the first request.
    let (mut socket, _) = connect_until_up(&relay.url(Role::App));
    thread::spawn(move || loop {
        match socket.read() {
            Ok(Message::Text(text)) if text.as_str() == BRIDGE_ATTACHED => send(&mut socket, &hello()),
            Ok(Message::Text(_)) => {
                drop(socket);
                return;
            }
            Ok(Message::Close(_)) | Err(_) => return,
            Ok(_) => {}
        }
    });
    let outcome = relay.invoke("ping");
    assert_eq!(outcome.exit, Exit::Unconfirmed, "{}", outcome.err);
    assert!(outcome.err.contains("shutting down"), "{}", outcome.err);
}

#[test]
fn test_a_second_page_is_closed_and_the_first_keeps_serving() {
    let relay = Relay::start();
    // The relay admits a role inside its handshake, before the `101`
    // (`bridge::connection`), so the first page is counted the moment
    // its handshake returns and the second page, connecting right
    // after, is the second.
    let app = app_always(&relay, json!({"first": true}));
    let (mut second, _) = connect_until_up(&relay.url(Role::App));
    let _ = second.get_ref().set_read_timeout(Some(Duration::from_secs(5)));
    // A close, not a read that merely timed out: a second page the relay
    // had taken for the app would sit there unanswered, and a timeout
    // must not pass for its eviction.
    let closed = match second.read() {
        Ok(Message::Close(_)) => true,
        Err(tungstenite::Error::Io(e)) => !matches!(
            e.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ),
        Err(_) => true,
        Ok(_) => false,
    };
    assert!(closed, "the second page is closed at once");
    let outcome = relay.invoke("ping");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_eq!(outcome.out, "first: true\n");
    assert_eq!(app.requests().len(), 1);
}

// ---- the client's --ws path ------------------------------------------------

#[test]
fn test_the_ws_variable_names_the_relay_when_no_flag_does_and_two_variables_are_refused() {
    let relay = Relay::start();
    let _app = app_always(&relay, json!({}));
    let url = relay.url(Role::Controller);
    let outcome = invoke_env(
        "ping",
        Env {
            ipc: None,
            ws: Some(&url),
        },
        "",
    );
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    let both = invoke_env(
        "ping",
        Env {
            ipc: Some("/nonexistent.sock"),
            ws: Some(&url),
        },
        "",
    );
    assert_eq!(both.exit, Exit::UsageError, "{}", both.err);
    assert!(
        both.err.contains("MANDALA_IPC") && both.err.contains("MANDALA_WS"),
        "{}",
        both.err
    );
    let flagged = invoke_env(
        &format!("--ws {url} ping"),
        Env {
            ipc: Some("/nonexistent.sock"),
            ws: Some("ws://127.0.0.1:1/controller/x"),
        },
        "",
    );
    assert_eq!(flagged.exit, Exit::Success, "a flag settles it: {}", flagged.err);
}

#[test]
fn test_no_relay_at_the_url_says_so_and_how_to_start_one() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let address = listener.local_addr().expect("bound");
    drop(listener);
    let outcome = invoke(&format!("--ws ws://{address}/controller/t ping"), None, "");
    assert_eq!(outcome.exit, Exit::TransportError, "{}", outcome.err);
    assert!(
        outcome.err.contains("nothing is listening") && outcome.err.contains("mandalactl bridge"),
        "{}",
        outcome.err
    );
}

#[test]
fn test_a_silent_app_behind_the_relay_times_out_without_hanging() {
    let relay = Relay::start();
    let _app = FakeApp::connect(&relay, |_| vec![]);
    let started = std::time::Instant::now();
    let outcome = invoke(
        &format!("--ws {} --timeout 0.5 ping", relay.url(Role::Controller)),
        None,
        "",
    );
    assert_eq!(outcome.exit, Exit::Unconfirmed, "{}", outcome.err);
    assert!(
        outcome.err.contains("no reply to `ping` within 0.5 s"),
        "{}",
        outcome.err
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn test_typed_arguments_and_raw_frames_reach_the_page_as_over_the_socket() {
    let relay = Relay::start();
    let app = app_always(&relay, json!({}));
    assert_eq!(
        relay.invoke("scene.hit_test 412 96 space=surface").exit,
        Exit::Success
    );
    assert_eq!(
        relay.invoke("raw '{\"cmd\":\"ping\",\"id\":\"r\"}'").exit,
        Exit::Success
    );
    assert_eq!(
        app.requests(),
        vec![
            json!({"id": 1, "cmd": "scene.hit_test", "x": 412, "y": 96, "space": "surface"}),
            json!({"id": "r", "cmd": "ping"}),
        ]
    );
}

// ---- the screenshot the client writes --------------------------------------

/// A one-pixel PNG, as a browser's `toDataURL` would carry it.
const PNG_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

fn screenshot_app(relay: &Relay, with_sidecar: bool) -> FakeApp {
    FakeApp::connect(relay, move |request| {
        let mut result = json!({
            "rendered": true, "width": 1, "height": 1, "scale": 1.0,
            "camera": {"center_x": 0.0, "center_y": 0.0, "zoom": 1.0}, "settled": true,
            "png_base64": PNG_BASE64,
        });
        if with_sidecar {
            result["sidecar"] = json!({"protocol": 1, "nodes": []});
        }
        vec![ok(&request["id"], result)]
    })
}

#[test]
fn test_a_screenshot_over_ws_is_written_by_the_client_with_its_sidecar_and_reported_as_files() {
    let relay = Relay::start();
    let app = screenshot_app(&relay, true);
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("shot.png");
    let outcome = relay.invoke(&format!("capture.screenshot {}", path.display()));
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    let png = std::fs::read(&path).expect("the PNG was written");
    assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']);
    let sidecar_path = dir.path().join("shot.png.geometry.json");
    let sidecar: Value =
        serde_json::from_slice(&std::fs::read(&sidecar_path).expect("the sidecar")).expect("JSON");
    assert_eq!(sidecar, json!({"protocol": 1, "nodes": []}));
    assert!(
        outcome
            .out
            .contains(&format!("path: {:?}\n", path.to_string_lossy())),
        "{}",
        outcome.out
    );
    assert!(
        outcome
            .out
            .contains(&format!("sidecar_path: {:?}\n", sidecar_path.to_string_lossy())),
        "{}",
        outcome.out
    );
    assert!(
        !outcome.out.contains("png_base64"),
        "the image is not printed: {}",
        outcome.out
    );
    // The file parameters never reached the page; the rest did.
    assert_eq!(
        app.requests(),
        vec![json!({"id": 1, "cmd": "capture.screenshot"})]
    );
}

#[test]
fn test_a_screenshot_over_ws_with_json_prints_the_rewritten_envelope() {
    let relay = Relay::start();
    let _app = screenshot_app(&relay, false);
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("shot.png");
    let outcome = relay.invoke(&format!(
        "--json capture.screenshot {} sidecar=false",
        path.display()
    ));
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    let frame: Value = serde_json::from_str(outcome.out.trim()).expect("one JSON line");
    assert_eq!(frame["result"]["path"], json!(path.to_string_lossy()));
    assert!(frame["result"].get("png_base64").is_none());
    assert!(
        frame["result"].get("sidecar_path").is_none(),
        "no sidecar was returned"
    );
    assert!(!dir.path().join("shot.png.geometry.json").exists());
}

#[test]
fn test_a_screenshot_over_ws_needs_a_path_and_refuses_a_file_in_the_way() {
    let relay = Relay::start();
    let app = screenshot_app(&relay, true);
    let outcome = relay.invoke("capture.screenshot");
    assert_eq!(outcome.exit, Exit::UsageError, "{}", outcome.err);
    assert!(outcome.err.contains("needs a path"), "{}", outcome.err);
    assert!(app.requests().is_empty(), "nothing was sent");
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("taken.png");
    std::fs::write(&path, b"old").expect("writes");
    let outcome = relay.invoke(&format!("capture.screenshot {}", path.display()));
    assert_eq!(outcome.exit, Exit::CommandError, "{}", outcome.err);
    assert!(
        outcome.err.contains("exists") && outcome.err.contains("overwrite=true"),
        "{}",
        outcome.err
    );
    assert_eq!(std::fs::read(&path).expect("kept"), b"old");
    let outcome = relay.invoke(&format!("capture.screenshot {} overwrite=true", path.display()));
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_ne!(std::fs::read(&path).expect("replaced"), b"old");
    assert_eq!(app.requests().len(), 2, "overwrite stayed on the client's side");
}

/// **The client's write never follows a symlink**, the guarantee
/// native's writer gives (`format/ipc.md` §`capture.screenshot`): a
/// dangling one is not resolved into a new file at its target, and one
/// pointing at a file is not written through even with `overwrite`.
/// The control is the neighbor test's plain file, which `overwrite`
/// does replace.
#[cfg(unix)]
#[test]
fn test_a_screenshot_over_ws_never_writes_through_a_symlink() {
    let relay = Relay::start();
    let app = screenshot_app(&relay, true);
    let dir = tempfile::tempdir().expect("a directory");

    let dangling = dir.path().join("dangling.png");
    let nowhere = dir.path().join("nowhere.png");
    std::os::unix::fs::symlink(&nowhere, &dangling).expect("symlinks");
    for extra in ["", " overwrite=true"] {
        let outcome = relay.invoke(&format!("capture.screenshot {}{extra}", dangling.display()));
        assert_eq!(outcome.exit, Exit::CommandError, "{extra}: {}", outcome.err);
        assert!(outcome.err.contains("symlink"), "{}", outcome.err);
        assert!(!nowhere.exists(), "{extra}: the link's target was not created");
    }

    let target = dir.path().join("precious.png");
    std::fs::write(&target, b"keep").expect("writes");
    let link = dir.path().join("link.png");
    std::os::unix::fs::symlink(&target, &link).expect("symlinks");
    let outcome = relay.invoke(&format!("capture.screenshot {} overwrite=true", link.display()));
    assert_eq!(outcome.exit, Exit::CommandError, "{}", outcome.err);
    assert_eq!(std::fs::read(&target).expect("kept"), b"keep");
    assert!(std::fs::symlink_metadata(&link)
        .expect("still there")
        .file_type()
        .is_symlink());
    assert_eq!(
        app.requests().len(),
        3,
        "each capture ran; only the write was refused"
    );
}

/// **Whatever is already at the write's sibling name is left alone**:
/// the client writes `<path>.<pid>.partial` first, and an entry there
/// that it did not create — a crash's leftover, the user's own file, a
/// planted link — is refused by name, never removed or written through.
/// The control is the same capture once the name is free, which writes.
#[cfg(unix)]
#[test]
fn test_a_screenshot_over_ws_leaves_an_entry_at_its_partial_name_alone() {
    let relay = Relay::start();
    let _app = screenshot_app(&relay, false);
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("shot.png");
    // `invoke` runs the client in this process, so the pid is ours.
    let partial = dir
        .path()
        .join(format!("shot.png.{}.partial", std::process::id()));
    let target = dir.path().join("precious");
    std::fs::write(&target, b"keep").expect("writes");
    std::os::unix::fs::symlink(&target, &partial).expect("symlinks");
    for extra in ["", " overwrite=true"] {
        let outcome = relay.invoke(&format!("capture.screenshot {}{extra}", path.display()));
        assert_eq!(outcome.exit, Exit::CommandError, "{extra}: {}", outcome.err);
        assert!(outcome.err.contains("partial"), "{}", outcome.err);
        assert!(
            std::fs::symlink_metadata(&partial)
                .expect("still there")
                .file_type()
                .is_symlink(),
            "{extra}: the entry at the partial name was removed"
        );
        assert_eq!(std::fs::read(&target).expect("kept"), b"keep");
        assert!(!path.exists(), "{extra}: nothing was published");
    }
    // Control: the name free, the capture writes.
    std::fs::remove_file(&partial).expect("removes");
    let outcome = relay.invoke(&format!("capture.screenshot {}", path.display()));
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert!(path.exists());
    assert!(!partial.exists(), "the client's own partial is cleaned up");
}

#[test]
fn test_a_screenshot_the_page_did_not_render_writes_nothing() {
    let relay = Relay::start();
    let _app = FakeApp::connect(&relay, |request| {
        vec![ok(
            &request["id"],
            json!({"rendered": false, "skipped": "font_system_contended", "width": 1, "height": 1,
                   "scale": 1.0, "camera": {"center_x": 0.0, "center_y": 0.0, "zoom": 1.0}, "settled": true}),
        )]
    });
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("none.png");
    let outcome = relay.invoke(&format!("capture.screenshot {}", path.display()));
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert!(outcome.out.contains("rendered: false"), "{}", outcome.out);
    assert!(!path.exists());
}

#[test]
fn test_the_bridge_command_refuses_client_flags_and_bad_arguments_without_listening() {
    for line in [
        "bridge --ipc /s",
        "bridge --ws ws://h/controller/t",
        "bridge --wait-settled",
        "bridge port=1",
        "bridge 41816",
        "bridge token=",
    ] {
        let outcome = invoke(line, None, "");
        assert_eq!(outcome.exit, Exit::UsageError, "`{line}`: {}", outcome.err);
    }
}
