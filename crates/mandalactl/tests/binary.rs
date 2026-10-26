// SPDX-License-Identifier: MPL-2.0

//! The shipped binary, as a shell runs it: real arguments, the real
//! `MANDALA_IPC` variable, real stdout / stderr and the real process
//! exit status. The library's own tests cover the same ground through
//! `run`; what only this file can see is `main.rs` — that the status
//! it returns is the one `run` decided, that the environment reaches
//! it, and that stdin is read for `raw -`.
//!
//! The server stands in for the app on its own endpoint: a Unix socket,
//! or on Windows a named pipe.

use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;

use serde_json::{json, Value};

/// Where a test's server listens: the `--ipc` value that reaches it,
/// and on Unix the private directory its socket lives in.
struct Endpoint {
    #[cfg(unix)]
    _dir: tempfile::TempDir,
    ipc: PathBuf,
}

/// An endpoint nobody listens on yet, under `name`.
#[cfg(unix)]
fn endpoint(name: &str) -> Endpoint {
    let dir = tempfile::tempdir().expect("a directory");
    let ipc = dir.path().join(format!("{name}.sock"));
    Endpoint { _dir: dir, ipc }
}

/// An endpoint nobody listens on yet, under `name`. A pipe name is
/// machine-wide, where a socket lives in its test's own directory, so
/// it carries the process id and a count.
#[cfg(windows)]
fn endpoint(name: &str) -> Endpoint {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    Endpoint {
        ipc: PathBuf::from(format!(
            "mandalactl-binary-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        )),
    }
}

/// A one-connection server: greets, then answers each request with
/// whatever `answer` makes of it, until the client hangs up.
fn serve_once(at: &Endpoint, answer: impl Fn(&Value) -> Value + Send + 'static) {
    #[cfg(unix)]
    let listener = UnixListener::bind(&at.ipc).expect("the socket binds");
    #[cfg(windows)]
    let listener = {
        use interprocess::os::windows::named_pipe::{pipe_mode, PipeListenerOptions};
        let name = at.ipc.to_str().expect("a test name is Unicode");
        let pipe = mandalactl::pipe_path(name).expect("a test name names a pipe");
        PipeListenerOptions::new()
            .path(pipe.as_str())
            .create_duplex::<pipe_mode::Bytes>()
            .expect("the pipe is created")
    };
    thread::spawn(move || {
        #[cfg(unix)]
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        #[cfg(unix)]
        let (reader, mut writer) = (stream.try_clone().expect("clone"), stream);
        #[cfg(windows)]
        let Ok(stream) = listener.accept() else {
            return;
        };
        #[cfg(windows)]
        let (reader, writer) = stream.split();
        #[cfg(windows)]
        let mut writer = &writer;
        let hello = json!({"event": "hello", "data": {"protocol": 1, "app": "mandala", "version": "0.1.0", "pid": 1, "map": null}});
        let _ = writeln!(writer, "{hello}");
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok_and(|n| n > 0) {
            let request: Value = serde_json::from_str(line.trim()).expect("the client sends JSON");
            let _ = writeln!(writer, "{}", answer(&request));
            line.clear();
        }
    });
}

fn mandalactl(args: &[&str], socket: Option<&Path>, stdin: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mandalactl"));
    command
        .args(args)
        .env_remove("MANDALA_IPC")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(socket) = socket {
        command.env("MANDALA_IPC", socket);
    }
    let mut child = command.spawn().expect("the binary starts");
    child
        .stdin
        .take()
        .expect("a piped stdin")
        .write_all(stdin.as_bytes())
        .expect("stdin is written");
    child.wait_with_output().expect("the binary finishes")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn test_exit_zero_and_a_greppable_line_when_the_app_says_ok() {
    let socket = endpoint("m");
    serve_once(
        &socket,
        |r| json!({"id": r["id"], "ok": true, "result": {"now_ms": 12.5}}),
    );
    let output = mandalactl(&["ping"], Some(&socket.ipc), "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(text(&output.stdout), "now_ms: 12.5\n");
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn test_exit_one_when_the_app_says_no() {
    let socket = endpoint("m");
    serve_once(
        &socket,
        |r| json!({"id": r["id"], "ok": false, "error": {"code": "no_document", "message": "load one"}}),
    );
    let output = mandalactl(&["query.selection"], Some(&socket.ipc), "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stdout), "");
    assert_eq!(text(&output.stderr), "error: no_document: load one\n");
}

#[test]
fn test_exit_two_when_nothing_is_listening() {
    let socket = endpoint("absent");
    let output = mandalactl(&["ping"], Some(&socket.ipc), "");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(&output.stdout), "");
    assert!(
        text(&output.stderr).contains("App not running"),
        "{}",
        text(&output.stderr)
    );
}

#[test]
fn test_exit_three_for_a_bad_command_line_and_for_no_socket_at_all() {
    let output = mandalactl(&["ping", "--no-such-flag"], None, "");
    assert_eq!(output.status.code(), Some(3));
    let output = mandalactl(&["ping"], None, "");
    assert_eq!(output.status.code(), Some(3));
    assert!(
        text(&output.stderr).contains("MANDALA_IPC"),
        "{}",
        text(&output.stderr)
    );
    let output = mandalactl(&[], None, "");
    assert_eq!(output.status.code(), Some(3));
}

#[test]
fn test_the_flag_names_the_socket_and_json_prints_the_envelope() {
    let socket = endpoint("m");
    serve_once(&socket, |r| json!({"id": r["id"], "ok": true, "result": {}}));
    let output = mandalactl(
        &[
            "--ipc",
            socket.ipc.to_str().expect("utf-8 path"),
            "ping",
            "--json",
        ],
        None,
        "",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(text(&output.stdout), "{\"id\":1,\"ok\":true,\"result\":{}}\n");
}

#[test]
fn test_raw_reads_the_request_from_standard_input() {
    let socket = endpoint("m");
    serve_once(
        &socket,
        |r| json!({"id": r["id"], "ok": true, "result": {"saw": r["cmd"]}}),
    );
    let output = mandalactl(&["raw", "-"], Some(&socket.ipc), "{\"cmd\":\"ping\"}");
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "saw: \"ping\"\n");
}

#[test]
fn test_help_and_version_exit_zero_with_no_socket() {
    let output = mandalactl(&["--help"], None, "");
    assert_eq!(output.status.code(), Some(0));
    assert!(text(&output.stdout).contains("EXIT CODES"));
    let output = mandalactl(&["--version"], None, "");
    assert_eq!(output.status.code(), Some(0));
    assert!(text(&output.stdout).starts_with("mandalactl "));
}
