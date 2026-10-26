// SPDX-License-Identifier: MPL-2.0

//! Test support: a real server on a real socket, and the invocation
//! harness that talks to it.
//!
//! No mocks (TEST_CONVENTIONS §T10). The "fake server" is a listener
//! in a thread that speaks the protocol's bytes — a Unix socket, or on
//! Windows a named pipe, the two endpoints the app serves — so what the
//! tests exercise is the client's real connect, framing, deadlines and
//! reads. It is a stand-in for the *app*, not for any part of this
//! crate.

use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};

use crate::run::{run, Env, Io};
use crate::Exit;

/// The working directory every [`invoke`] pretends to run in, spelled
/// as a shell on each platform would hand it over.
#[cfg(unix)]
pub const WORKING_DIRECTORY: &str = "/work/maps";
#[cfg(windows)]
pub const WORKING_DIRECTORY: &str = r"C:\work\maps";

/// What a shell would have shown for one invocation.
#[derive(Debug)]
pub struct Outcome {
    pub exit: Exit,
    pub out: String,
    pub err: String,
}

/// Run `mandalactl <line>` in-process with `env_ipc` as `$MANDALA_IPC`
/// and `stdin` as standard input.
pub fn invoke(line: &str, env_ipc: Option<&str>, stdin: &str) -> Outcome {
    invoke_env(
        line,
        Env {
            ipc: env_ipc,
            ws: None,
        },
        stdin,
    )
}

/// [`invoke`] with the whole environment given.
pub fn invoke_env(line: &str, env: Env<'_>, stdin: &str) -> Outcome {
    let words = split(line);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut input = stdin.as_bytes();
    let exit = run(
        &words,
        env,
        &mut Io {
            out: &mut out,
            err: &mut err,
            cwd: Some(Path::new(WORKING_DIRECTORY)),
            stdin: &mut input,
        },
    );
    Outcome {
        exit,
        out: String::from_utf8_lossy(&out).into_owned(),
        err: String::from_utf8_lossy(&err).into_owned(),
    }
}

/// [`invoke`] against `server`, with the socket given by `--ipc`.
pub fn against(server: &FakeServer, line: &str) -> Outcome {
    invoke(&format!("--ipc {} {line}", server.path().display()), None, "")
}

pub use crate::words::split;

/// The greeting a real app sends first.
pub fn hello() -> Value {
    json!({"event": "hello", "data": {"protocol": 1, "app": "mandala", "version": "0.1.0", "pid": 4242, "map": null}})
}

/// A reply the way the server shapes one.
pub fn ok(id: &Value, result: Value) -> Value {
    json!({"id": id, "ok": true, "result": result})
}

/// An error reply the way the server shapes one.
pub fn no(id: &Value, code: &str, message: &str) -> Value {
    json!({"id": id, "ok": false, "error": {"code": code, "message": message}})
}

#[cfg(unix)]
type Sender = UnixStream;
#[cfg(unix)]
type Receiver = UnixStream;
#[cfg(windows)]
type Sender = pipe::SendHalf;
#[cfg(windows)]
type Receiver = pipe::RecvHalf;

/// One accepted connection, as the server end sees it.
pub struct Peer {
    stream: Sender,
    reader: BufReader<Receiver>,
}

impl Peer {
    #[cfg(unix)]
    fn new(stream: UnixStream) -> Peer {
        let reader = BufReader::new(stream.try_clone().expect("the socket can be cloned"));
        Peer { stream, reader }
    }

    #[cfg(windows)]
    fn new(stream: pipe::Duplex) -> Peer {
        let (recv, send) = stream.split();
        Peer {
            stream: send,
            reader: BufReader::new(recv),
        }
    }

    /// Write one frame as one line.
    pub fn send(&mut self, frame: &Value) {
        self.send_raw(&format!("{frame}\n"));
    }

    /// Write bytes as given. Not flushed on a pipe: there a flush waits
    /// until the client has read everything, which a scenario whose
    /// client stops reading must not be made to wait for. What is
    /// written stays readable after the connection ends (`interprocess`
    /// hands an unflushed stream to a background flush when dropped).
    pub fn send_raw(&mut self, text: &str) {
        let mut stream = &self.stream;
        let _ = stream.write_all(text.as_bytes());
        #[cfg(unix)]
        let _ = stream.flush();
    }

    /// The next request line, parsed; `None` at end of input.
    pub fn read_request(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(serde_json::from_str(line.trim()).expect("the client sends JSON")),
        }
    }

    /// Greet, then answer every request with `answer`'s frames until
    /// the client disconnects.
    pub fn serve(&mut self, seen: &Mutex<Vec<Value>>, answer: &dyn Fn(&Value) -> Vec<Value>) {
        self.send(&hello());
        while let Some(request) = self.read_request() {
            seen.lock()
                .expect("the log is not poisoned")
                .push(request.clone());
            for frame in answer(&request) {
                self.send(&frame);
            }
        }
    }
}

/// A server on a socket in a private temporary directory, or on
/// Windows on a pipe of a name no other test uses.
pub struct FakeServer {
    #[cfg(unix)]
    _dir: tempfile::TempDir,
    /// The `--ipc` value that reaches it.
    path: PathBuf,
    seen: Arc<Mutex<Vec<Value>>>,
    connections: Arc<AtomicUsize>,
}

impl FakeServer {
    /// Accept connections forever, handing each (with its index from
    /// 0) to `behavior`. The thread is left running: it ends with the
    /// test process, and a test that connects fewer times than it
    /// might must not be made to wait for the rest.
    pub fn start(
        behavior: impl Fn(usize, &mut Peer, &Mutex<Vec<Value>>) + Send + Sync + 'static,
    ) -> FakeServer {
        #[cfg(unix)]
        let dir = tempfile::tempdir().expect("a temporary directory");
        #[cfg(unix)]
        let (path, listener) = {
            let path = dir.path().join("m.sock");
            let listener = UnixListener::bind(&path).expect("the socket binds");
            (path, listener)
        };
        #[cfg(windows)]
        let (path, listener) = pipe::listen();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let (log, count) = (Arc::clone(&seen), Arc::clone(&connections));
        thread::spawn(move || {
            let behavior = Arc::new(behavior);
            for stream in listener.incoming().flatten() {
                let index = count.fetch_add(1, Ordering::SeqCst);
                let (behavior, log) = (Arc::clone(&behavior), Arc::clone(&log));
                thread::spawn(move || behavior(index, &mut Peer::new(stream), &log));
            }
        });
        FakeServer {
            #[cfg(unix)]
            _dir: dir,
            path,
            seen,
            connections,
        }
    }

    /// A server that greets and answers every request with `answer`.
    pub fn answering(answer: impl Fn(&Value) -> Vec<Value> + Send + Sync + 'static) -> FakeServer {
        FakeServer::start(move |_, peer, seen| peer.serve(seen, &answer))
    }

    /// A server that answers `ok:true` with `result` to everything.
    pub fn always(result: Value) -> FakeServer {
        FakeServer::answering(move |request| vec![ok(&request["id"], result.clone())])
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every request received, in order.
    pub fn requests(&self) -> Vec<Value> {
        self.seen.lock().expect("the log is not poisoned").clone()
    }

    /// How many connections have been accepted.
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

/// The pipe end of the fake server: `interprocess`'s blocking listener,
/// which is what `mandalactl`'s own pipe client is built on.
#[cfg(windows)]
mod pipe {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use interprocess::os::windows::named_pipe::{
        pipe_mode, DuplexPipeStream, PipeListener, PipeListenerOptions, RecvPipeStream, SendPipeStream,
    };

    pub type Duplex = DuplexPipeStream<pipe_mode::Bytes>;
    pub type RecvHalf = RecvPipeStream<pipe_mode::Bytes>;
    pub type SendHalf = SendPipeStream<pipe_mode::Bytes>;

    /// Listen on a pipe named for this test alone, and give back the
    /// `--ipc` value that names it. A pipe name is machine-wide, where
    /// a socket lives in its test's own directory.
    pub fn listen() -> (PathBuf, PipeListener<pipe_mode::Bytes, pipe_mode::Bytes>) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let name = format!(
            "mandalactl-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        );
        let path = crate::pipe_path(&name).expect("a test name names a pipe");
        let listener = PipeListenerOptions::new()
            .path(path.as_str())
            .create_duplex::<pipe_mode::Bytes>()
            .expect("the pipe is created");
        (PathBuf::from(name), listener)
    }
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn test_the_splitter_reads_quotes_the_way_a_shell_does() {
        assert_eq!(split("a 'b c' \"d e\" f=g''"), ["a", "b c", "d e", "f=g"]);
        assert_eq!(split("x ''"), ["x", ""]);
    }
}
