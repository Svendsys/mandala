// SPDX-License-Identifier: MPL-2.0

//! One connection to the app's `--ipc` socket: connect, read the
//! greeting, then send requests and read their replies, each under a
//! deadline (`format/ipc.md` §Transport).
//!
//! **Nothing here can hang the caller.** Every wait has a deadline: the
//! connect (a thread, because `std` has no connect timeout), the
//! greeting, each reply, and each write. A deadline that passes is an
//! error that says what to suspect, since the likeliest cause is the
//! app and not the socket.
//!
//! **A busy slot is retried, briefly.** The server serves one
//! controller at a time and notices a departed one "a moment after the
//! socket closes", so two `mandalactl` calls in a row — the commonest
//! thing an agent's shell does — can find the slot still held and be
//! refused `busy`. [`Connection::open`] retries for [`BUSY_RETRY`], then
//! reports the refusal as what it is: another client is attached.
//!
//! Blocking `std` I/O and no runtime, the way the server's boundary
//! threads are. Unix sockets only; elsewhere [`Connection::open`]
//! fails naming the gap, the shape `ipc`'s own non-Unix stub has.

use std::time::Duration;

use crate::wire::Reply;

/// How long a refusal for `busy` is retried before it is reported.
pub const BUSY_RETRY: Duration = Duration::from_secs(2);

/// What to do when the socket cannot be reached, appended to the
/// errors that mean "the app is not there".
pub const LAUNCH_HINT: &str = "launch the app with: mandala <map.mindmap.json> --ipc <socket-path>";

/// One reply, with the line it arrived as (what `--json` prints).
#[derive(Debug, Clone, PartialEq)]
pub struct Exchange {
    pub reply: Reply,
    pub line: String,
}

#[cfg(unix)]
pub use self::unix::Connection;

#[cfg(not(unix))]
pub use self::unsupported::Connection;

#[cfg(unix)]
mod unix {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use serde_json::Value;

    use super::{Exchange, BUSY_RETRY, LAUNCH_HINT};
    use crate::wire::{decode_frame, frame_line, Event, Frame, Hello, FRAME_CAP};

    /// An open, greeted connection.
    #[derive(Debug)]
    pub struct Connection {
        reader: BufReader<UnixStream>,
        next_id: u64,
    }

    /// The first frame of a connection.
    enum Greeting {
        Hello(Hello),
        /// `connection_rejected`, with its `reason`.
        Rejected(String),
    }

    /// One line read under a deadline.
    enum Line {
        Text(String),
        Eof,
        TimedOut,
    }

    impl Connection {
        /// Connect to `path`, read the greeting and check it. `timeout`
        /// bounds the connect and each wait for the greeting; `force`
        /// overrides a protocol mismatch. The `Ok` side carries the
        /// connection and, when `force` overrode something, a note for
        /// stderr.
        pub fn open(
            path: &Path,
            timeout: Duration,
            force: bool,
        ) -> Result<(Connection, Option<String>), String> {
            let started = Instant::now();
            let mut pause = Duration::from_millis(20);
            loop {
                let stream = connect(path, timeout)?;
                let mut connection = Connection {
                    reader: BufReader::new(stream),
                    next_id: 1,
                };
                match connection.greeting(timeout)? {
                    Greeting::Hello(hello) => {
                        let note = hello.accept(force)?;
                        return Ok((connection, note));
                    }
                    Greeting::Rejected(reason) => {
                        if started.elapsed() + pause > BUSY_RETRY.min(timeout) {
                            return Err(format!(
                                "the app refused the connection ({reason}): another client holds the controller slot. \
                                 Only one client is served at a time; wait for it to disconnect and retry"
                            ));
                        }
                        thread::sleep(pause);
                        pause = (pause * 2).min(Duration::from_millis(250));
                    }
                }
            }
        }

        /// A request id not used before on this connection.
        pub fn next_id(&mut self) -> Value {
            let id = self.next_id;
            self.next_id += 1;
            Value::from(id)
        }

        /// Send `request` (a whole frame with an `id`) and return the
        /// reply that answers it, waiting at most `timeout`.
        ///
        /// A reply answers the request when its `id` equals the
        /// request's, or is `null` — the server's way of answering a
        /// line it could not read, and with one request in flight that
        /// line can only have been this one. Events other than
        /// `shutdown` are skipped: nothing here subscribes to any.
        pub fn exchange(&mut self, request: &Value, timeout: Duration) -> Result<Exchange, String> {
            let command = request
                .get("cmd")
                .and_then(Value::as_str)
                .unwrap_or("the request")
                .to_string();
            let id = request.get("id").cloned().unwrap_or(Value::Null);
            let line = frame_line(request)?;
            let deadline = Instant::now() + timeout;
            self.send(&line, &command, timeout)?;
            loop {
                match self.read_line(deadline)? {
                    Line::TimedOut => return Err(silence(&format!("reply to `{command}`"), timeout)),
                    Line::Eof => {
                        return Err(format!(
                            "the app closed the connection before replying to `{command}`: it exited, or aborted \
                             (the freeze watchdog aborts a wedged app after 10 s)"
                        ))
                    }
                    Line::Text(text) => match decode_frame(&text)? {
                        Frame::Event(event) => {
                            if event.name == "shutdown" {
                                return Err(format!("the app is shutting down; `{command}` was not answered"));
                            }
                        }
                        Frame::Reply(reply) => {
                            if reply.id == id || reply.id.is_null() {
                                return Ok(Exchange { reply, line: text });
                            }
                            return Err(format!(
                                "the app answered request id {} while `{command}` (id {id}) was pending",
                                reply.id
                            ));
                        }
                    },
                }
            }
        }

        fn greeting(&mut self, timeout: Duration) -> Result<Greeting, String> {
            let deadline = Instant::now() + timeout;
            match self.read_line(deadline)? {
                Line::TimedOut => Err(silence("greeting", timeout)),
                Line::Eof => Err(
                    "the connection closed before the greeting: not a Mandala `--ipc` socket, or the app is exiting".to_string(),
                ),
                Line::Text(text) => match decode_frame(&text)? {
                    Frame::Event(Event { name, data }) if name == "hello" => Ok(Greeting::Hello(Hello::of(&data))),
                    Frame::Event(Event { name, data }) if name == "connection_rejected" => Ok(Greeting::Rejected(
                        data.get("reason").and_then(Value::as_str).unwrap_or("busy").to_string(),
                    )),
                    other => Err(format!("expected a `hello` greeting, the server sent {other:?}")),
                },
            }
        }

        fn send(&mut self, line: &str, command: &str, timeout: Duration) -> Result<(), String> {
            let stream = self.reader.get_mut();
            stream
                .set_write_timeout(Some(timeout))
                .map_err(|e| format!("could not set the write deadline: {e}"))?;
            let mut frame = Vec::with_capacity(line.len() + 1);
            frame.extend_from_slice(line.as_bytes());
            frame.push(b'\n');
            stream.write_all(&frame).map_err(|e| match e.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
                    silence(&format!("room to send `{command}`"), timeout)
                }
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset => format!(
                    "the app closed the connection while `{command}` was being sent: it exited, or aborted"
                ),
                _ => format!("could not send `{command}`: {e}"),
            })
        }

        /// The next non-empty line, at most [`FRAME_CAP`] bytes, before
        /// `deadline`.
        fn read_line(&mut self, deadline: Instant) -> Result<Line, String> {
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Ok(Line::TimedOut);
                }
                self.reader
                    .get_ref()
                    .set_read_timeout(Some(remaining))
                    .map_err(|e| format!("could not set the read deadline: {e}"))?;
                let mut bytes = Vec::new();
                // Room for the cap, a `\r` and the terminator: the
                // server's cap counts the frame, not its line ending.
                let read = (&mut self.reader)
                    .take(FRAME_CAP as u64 + 2)
                    .read_until(b'\n', &mut bytes);
                match read {
                    Ok(0) => return Ok(Line::Eof),
                    Ok(_) => {}
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        return Ok(Line::TimedOut)
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return Ok(Line::Eof),
                    Err(e) => return Err(format!("reading from the app failed: {e}")),
                }
                let frame_len = bytes
                    .iter()
                    .rposition(|b| *b != b'\n' && *b != b'\r')
                    .map_or(0, |last| last + 1);
                if frame_len > FRAME_CAP {
                    return Err(format!(
                        "the app sent a frame over {FRAME_CAP} bytes, which its own cap forbids; refusing to buffer it"
                    ));
                }
                let text = String::from_utf8(bytes)
                    .map_err(|_| "the app sent a line that is not UTF-8".to_string())?;
                let text = text.trim();
                if !text.is_empty() {
                    return Ok(Line::Text(text.to_string()));
                }
            }
        }
    }

    /// The error for a deadline that passed with nothing received.
    fn silence(what: &str, timeout: Duration) -> String {
        format!(
            "no {what} within {:.1} s. App busy or frozen? A command that legitimately runs long \
             needs a larger --timeout; a wedged app is aborted by its freeze watchdog after 10 s",
            timeout.as_secs_f64()
        )
    }

    /// Connect, giving up after `timeout`. `std` offers no connect
    /// deadline, so the connect runs on a thread this one abandons when
    /// the time is up — a connect that never finishes costs a thread
    /// until the process exits, which a one-shot CLI does at once.
    fn connect(path: &Path, timeout: Duration) -> Result<UnixStream, String> {
        let (sender, receiver) = mpsc::channel();
        let target = path.to_path_buf();
        thread::spawn(move || {
            // The receiver is gone when the deadline has passed; the
            // late stream is then dropped, which closes it.
            let _ = sender.send(UnixStream::connect(target));
        });
        match receiver.recv_timeout(timeout) {
            Ok(Ok(stream)) => Ok(stream),
            Ok(Err(e)) => Err(unreachable(path, &e)),
            Err(_) => Err(format!(
                "timed out after {:.1} s connecting to {}. App busy or frozen? A wedged app is aborted by its \
                 freeze watchdog after 10 s",
                timeout.as_secs_f64(),
                path.display()
            )),
        }
    }

    /// Why `connect` failed, in words that name the next step.
    fn unreachable(path: &Path, error: &std::io::Error) -> String {
        use std::io::ErrorKind;
        let shown = path.display();
        match error.kind() {
            ErrorKind::NotFound => format!("no socket at {shown}. App not running, or started without --ipc? {LAUNCH_HINT}"),
            ErrorKind::ConnectionRefused => format!(
                "{shown} exists but nothing is listening: a stale socket from an app that exited without cleaning up. \
                 App not running? {LAUNCH_HINT}"
            ),
            ErrorKind::PermissionDenied => format!(
                "permission denied on {shown}: the socket is owner-only (0600); run as the user who launched the app"
            ),
            ErrorKind::InvalidInput => format!(
                "{shown} cannot be a socket address ({error}); a Unix socket path holds at most 107 bytes on Linux, 103 elsewhere"
            ),
            _ => format!("could not connect to {shown}: {error}"),
        }
    }
}

#[cfg(not(unix))]
mod unsupported {
    use std::path::Path;
    use std::time::Duration;

    use serde_json::Value;

    use super::Exchange;

    /// Stand-in for the connection where the transport does not exist
    /// yet. [`Connection::open`] is the only way to obtain one and
    /// always fails, so the other methods are unreachable by
    /// construction — the uninhabited field proves it to the compiler.
    #[derive(Debug)]
    pub struct Connection {
        never: std::convert::Infallible,
    }

    impl Connection {
        pub fn open(
            _path: &Path,
            _timeout: Duration,
            _force: bool,
        ) -> Result<(Connection, Option<String>), String> {
            Err("mandalactl cannot reach the app on this platform yet: Windows needs the named-pipe adapter, \
                 tracked in #169 (format/ipc.md \"Windows\")"
                .to_string())
        }

        pub fn next_id(&mut self) -> Value {
            match self.never {}
        }

        pub fn exchange(&mut self, _request: &Value, _timeout: Duration) -> Result<Exchange, String> {
            match self.never {}
        }
    }
}
