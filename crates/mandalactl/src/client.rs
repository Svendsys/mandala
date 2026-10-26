// SPDX-License-Identifier: MPL-2.0

//! One connection to the app: connect, read the greeting, then send
//! requests and read their replies, each under a deadline
//! (`format/ipc.md` §Transport).
//!
//! **Three transports, one connection.** A native app's `--ipc`
//! endpoint is a Unix socket, or on Windows a named pipe (#169), each
//! speaking newline-delimited frames; a browser build speaks the same
//! frames as WebSocket text messages through `mandalactl bridge`
//! (`format/ipc.md` §Transport "Browser"). Each is a [`Link`]: a line
//! out, a line in, under a deadline. Everything above that — the
//! greeting, the `busy` retry, the exchange, the watch — is one body,
//! so they cannot drift.
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
//! refused `busy`. [`Connection::open`] retries a `busy` refusal for
//! [`BUSY_RETRY`], then reports it as what it is: another client is
//! attached. A refusal for any other reason is reported as given.
//!
//! **How far a request got is part of the answer.** [`Connection::exchange`]
//! says whether a failed request was `NotSent` (nothing can have run) or
//! `Unanswered` (it went out whole, so it may have), because that is what
//! decides whether the caller may repeat it.
//!
//! Blocking I/O and no runtime, the way the server's boundary threads
//! are. Each transport exists where its platform does: the Unix socket
//! on Unix, with its own read and write timeouts; the named pipe on
//! Windows, whose blocking reads and writes `pipe_wire` bounds with a
//! reader and a writer thread, so the line-framed link above it is the
//! same code on both; the WebSocket everywhere but the browser build of
//! this crate (which the workspace type-checks and never runs). Off
//! those, [`Connection::open`] fails naming the gap. On the browser build both are stubs, so the
//! machinery between them — the line reader, the send classifier —
//! has no caller there and is allowed to be dead on that target alone
//! (`lib.rs`).

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::wire::{decode_frame, Event, Frame, Hello, Outgoing, Reply};

/// How long a refusal for `busy` is retried before it is reported.
pub const BUSY_RETRY: Duration = Duration::from_secs(2);

/// What to do when the socket cannot be reached, appended to the
/// errors that mean "the app is not there".
#[cfg(not(windows))]
pub const LAUNCH_HINT: &str = "launch the app with: mandala <map.mindmap.json> --ipc <socket-path>";

/// What to do when the pipe cannot be reached, appended to the errors
/// that mean "the app is not there".
#[cfg(windows)]
pub const LAUNCH_HINT: &str = "launch the app with: mandala <map.mindmap.json> --ipc <name>";

/// The Windows pipe an `--ipc` value names: `--ipc <name>` is
/// `\\.\pipe\mandala-<name>` (`format/ipc.md` §Windows), the same value
/// the app was launched with. The app maps it by the same rule, and a
/// test on its side holds the two together.
///
/// Compiled on every platform, not only Windows, so that test can run
/// where CI runs everything.
pub fn pipe_path(name: &str) -> Result<String, String> {
    const PREFIX: &str = r"\\.\pipe\mandala-";
    const NAME_MAX: usize = 256;
    if name.is_empty() {
        return Err("`--ipc` needs a pipe name on Windows, e.g. --ipc mandala".to_string());
    }
    if name.contains('\0') {
        return Err(format!(
            "`--ipc {name:?}`: a pipe name cannot contain a NUL character"
        ));
    }
    if name.contains('\\') {
        return Err(format!(
            "`--ipc {name}`: on Windows the value names a pipe, not a path, and a pipe name cannot contain a backslash"
        ));
    }
    let path = format!("{PREFIX}{name}");
    let after_root = path.trim_start_matches(r"\\.\pipe\").encode_utf16().count();
    if after_root > NAME_MAX {
        return Err(format!(
            "`--ipc {name}`: the pipe name `mandala-{name}` is {after_root} UTF-16 units; Windows allows {NAME_MAX}"
        ));
    }
    Ok(path)
}

/// What to do when the relay cannot be reached.
pub const BRIDGE_HINT: &str = "start the relay with: mandalactl bridge, and open the page with its app URL \
                               (?ipc=…); then pass its controller URL here";

/// How far a request got — the fact that decides whether repeating it
/// is safe. Not an error type (CODE_CONVENTIONS §9): the two failures
/// carry the same plain `String` message every stage's failures do, and
/// differ in what they let the caller conclude.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The app answered.
    Answered(Exchange),
    /// The request's body did not go out whole, so it cannot have run:
    /// a truncated JSON object does not decode, and a truncated
    /// WebSocket frame is never delivered.
    NotSent(String),
    /// The request's body went out whole and no answer was read — the
    /// deadline passed, the connection closed, the app announced it was
    /// shutting down, or what came back was unreadable. It may have run.
    /// That includes a send that failed on the one-byte terminator: the
    /// server also runs a final line its peer closes without ending.
    Unanswered(String),
}

/// What [`Connection::watch_next`] found while waiting for an event.
#[derive(Debug, Clone, PartialEq)]
pub enum Watched {
    /// An unsolicited frame, with the line it arrived as (what
    /// `--json` prints).
    Event { event: crate::wire::Event, line: String },
    /// The connection ended: the app exited, or closed it.
    Closed,
    /// Nothing arrived within the wait. Not an end: a watch is quiet
    /// for as long as the app is.
    Quiet,
}

/// One reply, with the line it arrived as (what `--json` prints).
#[derive(Debug, Clone, PartialEq)]
pub struct Exchange {
    pub reply: Reply,
    pub line: String,
}

/// Where the app is: the one thing the two transports differ in before
/// a connection exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The `--ipc` endpoint of a native app: its socket path, or on
    /// Windows its pipe name.
    Ipc(PathBuf),
    /// The controller URL of a `mandalactl bridge` a browser app is
    /// connected to.
    Ws(String),
}

impl Target {
    /// Whether this is the browser transport, where `capture.screenshot`
    /// returns its PNG inline for the client to write.
    pub fn is_ws(&self) -> bool {
        matches!(self, Target::Ws(_))
    }
}

/// One line read under a deadline.
enum Line {
    Text(String),
    Eof,
    TimedOut,
}

/// A transport: a line out and a line in, each under a deadline. Every
/// implementation is private to this module — the line-framed stream
/// (a Unix socket, or the Windows pipe) and the WebSocket — and the
/// connection above them is the one surface.
trait Link {
    /// Write one frame. A failure is [`Outcome::NotSent`] only while the
    /// request cannot have reached the app whole, [`Outcome::Unanswered`]
    /// once it may have.
    fn send(&mut self, request: &Outgoing, deadline: Instant, timeout: Duration) -> Result<(), Box<Outcome>>;
    /// The next non-empty frame, before `deadline`.
    fn read_line(&mut self, deadline: Instant) -> Result<Line, String>;
}

/// The first frame of a connection.
enum Greeting {
    Hello(Hello),
    /// `connection_rejected`, with its `reason`.
    Rejected(String),
}

/// An open, greeted connection.
pub struct Connection {
    link: Box<dyn Link>,
    next_id: u64,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("next_id", &self.next_id)
            .finish()
    }
}

impl Connection {
    /// Connect to `target`, read the greeting and check it. `timeout`
    /// bounds the connect and each wait for the greeting; `force`
    /// overrides a protocol mismatch. The `Ok` side carries the
    /// connection and, when `force` overrode something, a note for
    /// stderr.
    pub fn open(
        target: &Target,
        timeout: Duration,
        force: bool,
    ) -> Result<(Connection, Option<String>), String> {
        let started = Instant::now();
        let mut pause = Duration::from_millis(20);
        loop {
            let link = connect(target, timeout)?;
            let mut connection = Connection { link, next_id: 1 };
            match connection.greeting(timeout)? {
                Greeting::Hello(hello) => {
                    let note = hello.accept(force)?;
                    return Ok((connection, note));
                }
                Greeting::Rejected(reason) if reason != "busy" => {
                    return Err(rejected_for(&reason));
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

    /// Send `request` and return the reply that answers it, waiting
    /// at most `timeout` for it.
    ///
    /// The [`Outcome`] says how far the request got, which is the
    /// fact a caller needs before deciding whether to repeat it:
    /// `NotSent` (nothing can have run) or `Unanswered` (it went out
    /// whole and no reply was read, so it may have).
    ///
    /// A reply answers the request when its `id` equals the
    /// request's, or is `null` — the server's way of answering a
    /// line it could not read, and with one request in flight that
    /// line can only have been this one. Events other than
    /// `shutdown` are skipped: nothing here subscribes to any.
    pub fn exchange(&mut self, request: &Outgoing, timeout: Duration) -> Outcome {
        let command = &request.command;
        let deadline = Instant::now() + timeout;
        if let Err(failed) = self.link.send(request, deadline, timeout) {
            return *failed;
        }
        loop {
            let line = match self.link.read_line(deadline) {
                Ok(Line::Text(text)) => text,
                Ok(Line::TimedOut) => {
                    return Outcome::Unanswered(silence(&format!("reply to `{command}`"), timeout))
                }
                Ok(Line::Eof) => {
                    return Outcome::Unanswered(format!(
                        "the app closed the connection before replying to `{command}`: it exited, or aborted \
                         (the freeze watchdog aborts a wedged app after 10 s)"
                    ))
                }
                Err(why) => return Outcome::Unanswered(why),
            };
            match decode_frame(&line) {
                Err(why) => return Outcome::Unanswered(why),
                Ok(Frame::Event(event)) => {
                    if event.name == "shutdown" {
                        return Outcome::Unanswered(format!(
                            "the app is shutting down; `{command}` was not answered"
                        ));
                    }
                }
                Ok(Frame::Reply(reply)) => {
                    if reply.id == request.id || reply.id.is_null() {
                        return Outcome::Answered(Exchange { reply, line });
                    }
                    return Outcome::Unanswered(format!(
                        "the app answered request id {} while `{command}` (id {}) was pending",
                        reply.id, request.id
                    ));
                }
            }
        }
    }

    /// Wait up to `wait` for the next unsolicited frame, once a
    /// request has subscribed this connection to some. A reply
    /// arriving here answers nothing (nothing is pending), which is
    /// a fault of the server's, reported as one.
    pub fn watch_next(&mut self, wait: Duration) -> Result<Watched, String> {
        match self.link.read_line(Instant::now() + wait)? {
            Line::TimedOut => Ok(Watched::Quiet),
            Line::Eof => Ok(Watched::Closed),
            Line::Text(line) => match decode_frame(&line)? {
                Frame::Event(event) => Ok(Watched::Event { event, line }),
                Frame::Reply(reply) => Err(format!(
                    "the app sent a reply (id {}) while no request was pending",
                    reply.id
                )),
            },
        }
    }

    fn greeting(&mut self, timeout: Duration) -> Result<Greeting, String> {
        let deadline = Instant::now() + timeout;
        match self.link.read_line(deadline)? {
            Line::TimedOut => Err(silence("greeting", timeout)),
            Line::Eof => Err(
                "the connection closed before the greeting: not a Mandala `--ipc` socket or bridge, or the app is \
                 exiting"
                    .to_string(),
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
}

/// A refusal that retrying will not change, in words that name the
/// next step. The relay's own reasons are spelled out; any other is
/// reported as given.
fn rejected_for(reason: &str) -> String {
    match reason {
        "no_app" => {
            "the relay refused the connection (reason: no_app): no browser app is connected to it. Open the \
                     page with the relay's app URL (?ipc=…) and retry"
                .to_string()
        }
        _ => format!("the app refused the connection (reason: {reason}); retrying will not change that"),
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

fn cap_error() -> String {
    format!(
        "the app sent a frame over {} bytes, which its own cap forbids; refusing to buffer it",
        crate::wire::FRAME_CAP
    )
}

/// What a send that stopped after `written` bytes means for a request
/// whose body is `body_len` bytes (its terminator is one more).
fn send_failure(written: usize, body_len: usize, why: String) -> Outcome {
    if written >= body_len {
        Outcome::Unanswered(why)
    } else {
        Outcome::NotSent(why)
    }
}

/// Write all of `bytes`, asking `write` for one chunk at a time with
/// the time left before `deadline` (it arms the socket timeout and
/// writes). `Err` carries how many bytes went out and why it
/// stopped; a deadline that passed, or a write that would block, is
/// [`std::io::ErrorKind::TimedOut`].
fn write_all_within(
    write: &mut dyn FnMut(&[u8], Duration) -> std::io::Result<usize>,
    bytes: &[u8],
    deadline: Instant,
) -> Result<(), (usize, std::io::ErrorKind)> {
    use std::io::ErrorKind;
    let mut written = 0;
    while written < bytes.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err((written, ErrorKind::TimedOut));
        }
        match write(&bytes[written..], remaining) {
            Ok(0) => return Err((written, ErrorKind::WriteZero)),
            Ok(n) => written += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock => return Err((written, ErrorKind::TimedOut)),
            Err(e) => return Err((written, e.kind())),
        }
    }
    Ok(())
}

/// Why a send stopped, in words that name what to suspect.
fn send_error(command: &str, kind: std::io::ErrorKind, timeout: Duration) -> String {
    match kind {
        std::io::ErrorKind::TimedOut => silence(&format!("room to send `{command}`"), timeout),
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset => {
            format!("the app closed the connection while `{command}` was being sent: it exited, or aborted")
        }
        std::io::ErrorKind::WriteZero => format!("the app stopped reading while `{command}` was being sent"),
        other => format!("could not send `{command}`: {other}"),
    }
}

/// Open the transport `target` names, giving up after `timeout`.
fn connect(target: &Target, timeout: Duration) -> Result<Box<dyn Link>, String> {
    match target {
        Target::Ipc(path) => stream::connect(path, timeout),
        Target::Ws(url) => ws::connect(url, timeout),
    }
}

/// The line-framed transport of a native app's `--ipc` endpoint:
/// newline-delimited frames over a byte stream that is the Unix socket
/// on Unix and the named pipe on Windows (#169). One body for both:
/// the pipe's `pipe_wire::PipeWire` (Windows-only, so not linked) gives
/// the per-call read and write timeouts a socket has natively, and
/// says, through [`stream::Wire`], how many bytes of a write that gave
/// up may still reach the app.
#[cfg(any(unix, windows))]
mod stream {
    use std::io::{BufRead, BufReader, Write};
    #[cfg(unix)]
    use std::os::unix::net::UnixStream as Stream;
    use std::path::Path;
    #[cfg(unix)]
    use std::sync::mpsc;
    #[cfg(unix)]
    use std::thread;
    use std::time::{Duration, Instant};

    #[cfg(windows)]
    use super::pipe_wire::connect as connect_stream;
    #[cfg(windows)]
    use super::pipe_wire::PipeWire as Stream;
    #[cfg(unix)]
    use super::LAUNCH_HINT;
    use super::{cap_error, send_error, send_failure, write_all_within, Line, Link, Outcome};
    use crate::wire::{Outgoing, FRAME_CAP};

    /// What a stream says beyond `Read` and `Write`: how many bytes of
    /// a write that gave up may still reach the app. A socket write
    /// that times out has written nothing more than it reported; the
    /// Windows pipe's runs on a thread that may still finish it.
    pub(super) trait Wire {
        fn in_flight(&self) -> usize {
            0
        }
    }

    #[cfg(unix)]
    impl Wire for Stream {}

    /// Newline-delimited frames over the endpoint's stream.
    struct StreamLink {
        reader: BufReader<Stream>,
    }

    impl Link for StreamLink {
        /// Write the request line and its terminator as one buffer, every
        /// chunk under whatever is left of `deadline`: a peer that accepts
        /// a few bytes at a time must not stretch one send past the
        /// timeout.
        ///
        /// A failure is [`Outcome::NotSent`] only while the request's body
        /// is incomplete — a truncated JSON object cannot decode, so
        /// nothing can run. Once the whole body is written the request
        /// *can* run, terminator or not: the server executes a final line
        /// its peer closes without terminating, and closing is exactly
        /// what this process does next. A failure then is
        /// [`Outcome::Unanswered`].
        fn send(
            &mut self,
            request: &Outgoing,
            deadline: Instant,
            timeout: Duration,
        ) -> Result<(), Box<Outcome>> {
            let command = &request.command;
            let mut frame = Vec::with_capacity(request.line.len() + 1);
            frame.extend_from_slice(request.line.as_bytes());
            frame.push(b'\n');
            let stream = self.reader.get_mut();
            let written = write_all_within(
                &mut |chunk, remaining| {
                    stream.set_write_timeout(Some(remaining))?;
                    stream.write(chunk)
                },
                &frame,
                deadline,
            );
            let Err((written, kind)) = written else {
                return Ok(());
            };
            // Bytes a write gave up on may still land, so they count as
            // sent: claiming `NotSent` for a request that then runs is the
            // one answer this must never give.
            let written = written + self.reader.get_ref().in_flight();
            // Boxed because an `Outcome` can hold a whole reply and this is
            // only ever a failure.
            Err(Box::new(send_failure(
                written,
                request.line.len(),
                send_error(command, kind, timeout),
            )))
        }

        /// The next non-empty line, at most [`FRAME_CAP`] bytes, before
        /// `deadline`.
        ///
        /// The deadline is re-armed before every fill of the buffer, not
        /// once per line: a socket timeout bounds one `read`, so a reply
        /// that trickles in a chunk at a time would otherwise restart
        /// the clock with each chunk and outlast any `--timeout`.
        fn read_line(&mut self, deadline: Instant) -> Result<Line, String> {
            let mut bytes: Vec<u8> = Vec::new();
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Ok(Line::TimedOut);
                }
                self.reader
                    .get_ref()
                    .set_read_timeout(Some(remaining))
                    .map_err(|e| format!("could not set the read deadline: {e}"))?;
                let available = match self.reader.fill_buf() {
                    Ok(available) => available,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
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
                };
                let at_eof = available.is_empty();
                let (take, terminated) = match available.iter().position(|b| *b == b'\n') {
                    Some(newline) => (newline + 1, true),
                    None => (available.len(), false),
                };
                bytes.extend_from_slice(&available[..take]);
                self.reader.consume(take);
                // The cap counts the frame, not its line ending (a `\r`
                // and a `\n` at most), and bounds what is buffered.
                if bytes.len() > FRAME_CAP + 2 {
                    return Err(cap_error());
                }
                if !(terminated || at_eof) {
                    continue;
                }
                let frame_len = bytes
                    .iter()
                    .rposition(|b| *b != b'\n' && *b != b'\r')
                    .map_or(0, |last| last + 1);
                if frame_len > FRAME_CAP {
                    return Err(cap_error());
                }
                let text = String::from_utf8(std::mem::take(&mut bytes))
                    .map_err(|_| "the app sent a line that is not UTF-8".to_string())?;
                let text = text.trim();
                if !text.is_empty() {
                    return Ok(Line::Text(text.to_string()));
                }
                if at_eof {
                    return Ok(Line::Eof);
                }
            }
        }
    }

    /// Connect to the endpoint `path` names, giving up after
    /// `timeout`.
    pub(super) fn connect(path: &Path, timeout: Duration) -> Result<Box<dyn Link>, String> {
        let stream = connect_stream(path, timeout)?;
        Ok(Box::new(StreamLink {
            reader: BufReader::new(stream),
        }))
    }

    /// Connect to the socket, giving up after `timeout`. `std` offers
    /// no connect deadline, so the connect runs on a thread this one
    /// abandons when the time is up — a connect that never finishes
    /// costs a thread until the process exits, which a one-shot CLI
    /// does at once.
    #[cfg(unix)]
    fn connect_stream(path: &Path, timeout: Duration) -> Result<Stream, String> {
        let (sender, receiver) = mpsc::channel();
        let target = path.to_path_buf();
        thread::spawn(move || {
            // The receiver is gone when the deadline has passed; the
            // late stream is then dropped, which closes it.
            let _ = sender.send(Stream::connect(target));
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
    #[cfg(unix)]
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

/// The app's named pipe as a byte stream with the per-call read and
/// write timeouts a Unix socket has natively (#169).
///
/// `interprocess`'s blocking pipe client cannot bound a read or a write,
/// and the connection above bounds every one. So each blocking call
/// runs on a thread of its own — one reader, one writer — and this side
/// waits for its result with the timeout the caller set, the way
/// `connect` already does for the connect. A wait that times out leaves
/// its call running: the reader's result is kept for the next read, and
/// a write's bytes are reported by [`stream::Wire::in_flight`], because
/// they may still reach the app.
///
/// The reader reads only when a read is asked for. One that drained the
/// pipe ahead of the caller would buffer without bound, which is what
/// the frame cap above exists to refuse.
#[cfg(windows)]
mod pipe_wire {
    use std::cell::Cell;
    use std::io::{self, ErrorKind, Read, Write};
    use std::path::Path;
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
    use std::thread;
    use std::time::Duration;

    use interprocess::os::windows::named_pipe::{pipe_mode, DuplexPipeStream};
    use interprocess::ConnectWaitMode;

    use super::stream::Wire;
    use super::{pipe_path, LAUNCH_HINT};

    /// The most one read asks the reader thread for.
    const READ_CHUNK: usize = 64 << 10;

    /// The most one write hands the writer thread. A write that times
    /// out cannot be canceled, so whatever it carried may still reach
    /// the app (`in_flight`); bounding it keeps that to one chunk, so a
    /// request whose body did not all leave is known to be incomplete —
    /// and an incomplete line is one the app never runs.
    const WRITE_CHUNK: usize = 64 << 10;

    #[derive(Debug)]
    pub struct PipeWire {
        read_timeout: Cell<Option<Duration>>,
        write_timeout: Cell<Option<Duration>>,
        reads: Sender<usize>,
        chunks: Receiver<io::Result<Vec<u8>>>,
        /// A read was asked for and its result has not been taken.
        read_pending: bool,
        /// Bytes a read returned beyond what its caller had room for.
        leftover: Vec<u8>,
        writes: Sender<Vec<u8>>,
        written: Receiver<io::Result<usize>>,
        /// The size of a write that timed out and may still finish.
        in_flight: usize,
    }

    impl PipeWire {
        fn new(stream: DuplexPipeStream<pipe_mode::Bytes>) -> PipeWire {
            let (recv, send) = stream.split();
            let (reads, wanted) = mpsc::channel::<usize>();
            let (chunk_tx, chunks) = mpsc::channel();
            thread::spawn(move || {
                for size in wanted {
                    let mut chunk = vec![0; size];
                    let read = (&recv).read(&mut chunk).map(|n| {
                        chunk.truncate(n);
                        chunk
                    });
                    if chunk_tx.send(read).is_err() {
                        return;
                    }
                }
            });
            let (writes, pending) = mpsc::channel::<Vec<u8>>();
            let (written_tx, written) = mpsc::channel();
            thread::spawn(move || {
                for bytes in pending {
                    if written_tx.send((&send).write(&bytes)).is_err() {
                        return;
                    }
                }
            });
            PipeWire {
                read_timeout: Cell::new(None),
                write_timeout: Cell::new(None),
                reads,
                chunks,
                read_pending: false,
                leftover: Vec::new(),
                writes,
                written,
                in_flight: 0,
            }
        }

        /// How long each read waits; `UnixStream::set_read_timeout`'s
        /// contract.
        pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.read_timeout.set(timeout);
            Ok(())
        }

        /// How long each write waits; `UnixStream::set_write_timeout`'s
        /// contract.
        pub fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.write_timeout.set(timeout);
            Ok(())
        }
    }

    impl Wire for PipeWire {
        fn in_flight(&self) -> usize {
            self.in_flight
        }
    }

    /// Wait for one result from a helper thread, as long as `timeout`
    /// allows: a timeout is `TimedOut`, a thread gone is a broken pipe.
    fn wait<T>(
        results: &Receiver<io::Result<T>>,
        timeout: Option<Duration>,
    ) -> Result<io::Result<T>, io::Error> {
        let got = match timeout {
            Some(timeout) => results.recv_timeout(timeout),
            None => results.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match got {
            Ok(result) => Ok(result),
            Err(RecvTimeoutError::Timeout) => Err(io::Error::from(ErrorKind::TimedOut)),
            Err(RecvTimeoutError::Disconnected) => Err(io::Error::from(ErrorKind::BrokenPipe)),
        }
    }

    impl Read for PipeWire {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            if self.leftover.is_empty() {
                if !self.read_pending {
                    self.reads
                        .send(buf.len().min(READ_CHUNK))
                        .map_err(|_| io::Error::from(ErrorKind::BrokenPipe))?;
                    self.read_pending = true;
                }
                // A timeout leaves the read pending; a result taken off
                // the channel, error or not, ends it.
                let read = wait(&self.chunks, self.read_timeout.get())?;
                self.read_pending = false;
                self.leftover = read?;
                if self.leftover.is_empty() {
                    return Ok(0);
                }
            }
            let n = self.leftover.len().min(buf.len());
            buf[..n].copy_from_slice(&self.leftover[..n]);
            self.leftover.drain(..n);
            Ok(n)
        }
    }

    impl Write for PipeWire {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.in_flight > 0 {
                // An earlier write never reported back: whatever is sent
                // now could overtake it.
                return Err(io::Error::from(ErrorKind::TimedOut));
            }
            // A partial write, which the sender loops on.
            let bytes = &bytes[..bytes.len().min(WRITE_CHUNK)];
            self.writes
                .send(bytes.to_vec())
                .map_err(|_| io::Error::from(ErrorKind::BrokenPipe))?;
            match wait(&self.written, self.write_timeout.get()) {
                Ok(result) => result,
                Err(e) => {
                    if e.kind() == ErrorKind::TimedOut {
                        self.in_flight = bytes.len();
                    }
                    Err(e)
                }
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Connect to the pipe `--ipc <name>` names, giving up after
    /// `timeout`. Unlike a socket's, a pipe's connect takes a deadline
    /// of its own: it waits for a free instance with `WaitNamedPipeW`,
    /// which the server always has one of but for the instant between
    /// handing one over and making the next.
    pub fn connect(path: &Path, timeout: Duration) -> Result<PipeWire, String> {
        let name = path
            .to_str()
            .ok_or_else(|| format!("`--ipc {}`: a pipe name must be valid Unicode", path.display()))?;
        let pipe = pipe_path(name)?;
        DuplexPipeStream::<pipe_mode::Bytes>::connect_by_path_with_wait_mode(
            pipe.as_str(),
            ConnectWaitMode::Timeout(timeout),
        )
        .map(PipeWire::new)
        .map_err(|e| unreachable(&pipe, &e, timeout))
    }

    /// Why `connect` failed, in words that name the next step.
    fn unreachable(pipe: &str, error: &io::Error, timeout: Duration) -> String {
        match error.kind() {
            ErrorKind::TimedOut => format!(
                "timed out after {:.1} s connecting to {pipe}. App busy or frozen? A wedged app is aborted by its \
                 freeze watchdog after 10 s",
                timeout.as_secs_f64()
            ),
            ErrorKind::NotFound => {
                format!("no pipe {pipe}. App not running, or started without --ipc? {LAUNCH_HINT}")
            }
            ErrorKind::PermissionDenied => format!(
                "permission denied on {pipe}: the pipe is owner-only; run as the user who launched the app"
            ),
            _ => format!("could not connect to {pipe}: {error}"),
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod stream {
    use std::path::Path;
    use std::time::Duration;

    use super::Link;

    pub(super) fn connect(_path: &Path, _timeout: Duration) -> Result<Box<dyn Link>, String> {
        Err(
            "mandalactl cannot reach a native app on this platform: the `--ipc` transport is a Unix socket, \
             or a named pipe on Windows (format/ipc.md \"Transport\"). A browser app is reachable through \
             `mandalactl bridge` and --ws"
                .to_string(),
        )
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod ws {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
    use std::time::{Duration, Instant};

    use tungstenite::protocol::WebSocketConfig;
    use tungstenite::{Message, WebSocket};

    use super::{cap_error, send_error, send_failure, Line, Link, Outcome, BRIDGE_HINT};
    use crate::wire::{redacted, Outgoing, FRAME_CAP};

    /// The WebSocket transport: one text message per frame, through
    /// `mandalactl bridge` (`format/ipc.md` §Transport "Browser").
    struct WsLink {
        socket: WebSocket<Counted>,
    }

    /// The stream, counting the bytes written through it: how a send
    /// that failed is told apart as `NotSent` or `Unanswered`
    /// ([`WsLink::send`]).
    struct Counted {
        stream: TcpStream,
        written: usize,
    }

    impl Read for Counted {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.stream.read(buf)
        }
    }

    impl Write for Counted {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let n = self.stream.write(buf)?;
            self.written += n;
            Ok(n)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.stream.flush()
        }
    }

    /// The bytes a client text frame of `payload_len` bytes occupies on
    /// the wire: the two-byte header, the extended length a payload of
    /// 126 bytes or more needs (two bytes, or eight from 64 KiB), the
    /// four-byte mask every client frame carries, and the payload.
    pub(super) fn frame_len(payload_len: usize) -> usize {
        let extended = match payload_len {
            0..=125 => 0,
            126..=65_535 => 2,
            _ => 8,
        };
        2 + extended + 4 + payload_len
    }

    impl Link for WsLink {
        /// Send the frame as one text message. A WebSocket frame names
        /// its length in its header, so a frame cut short is never
        /// delivered: a send that stopped before the whole frame went
        /// out is [`Outcome::NotSent`], and one that failed after it is
        /// [`Outcome::Unanswered`].
        fn send(
            &mut self,
            request: &Outgoing,
            deadline: Instant,
            timeout: Duration,
        ) -> Result<(), Box<Outcome>> {
            let command = &request.command;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Box::new(Outcome::NotSent(send_error(
                    command,
                    std::io::ErrorKind::TimedOut,
                    timeout,
                ))));
            }
            let before = self.socket.get_ref().written;
            let sent = self
                .socket
                .get_ref()
                .stream
                .set_write_timeout(Some(remaining))
                .map_err(tungstenite::Error::Io)
                .and_then(|()| self.socket.send(Message::Text(request.line.as_str().into())));
            let Err(error) = sent else {
                return Ok(());
            };
            let written = self.socket.get_ref().written - before;
            let kind = match error {
                tungstenite::Error::Io(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::io::ErrorKind::TimedOut
                }
                tungstenite::Error::Io(e) => e.kind(),
                tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => {
                    std::io::ErrorKind::BrokenPipe
                }
                other => {
                    return Err(Box::new(send_failure(
                        written,
                        frame_len(request.line.len()),
                        format!("could not send `{command}`: {other}"),
                    )))
                }
            };
            // The whole frame, header and mask included, is what must
            // have gone out for the request to be deliverable.
            Err(Box::new(send_failure(
                written,
                frame_len(request.line.len()),
                send_error(command, kind, timeout),
            )))
        }

        /// The next text message, at most [`FRAME_CAP`] bytes, before
        /// `deadline`. Pings are answered by the codec, binary messages
        /// are nothing the protocol spells and are skipped, and a close
        /// is the end.
        fn read_line(&mut self, deadline: Instant) -> Result<Line, String> {
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Ok(Line::TimedOut);
                }
                self.socket
                    .get_ref()
                    .stream
                    .set_read_timeout(Some(remaining))
                    .map_err(|e| format!("could not set the read deadline: {e}"))?;
                match self.socket.read() {
                    Ok(Message::Text(text)) => {
                        let text = text.as_str().trim();
                        if !text.is_empty() {
                            return Ok(Line::Text(text.to_string()));
                        }
                    }
                    Ok(Message::Close(_)) => return Ok(Line::Eof),
                    Ok(Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
                    Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                        return Ok(Line::Eof)
                    }
                    Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(tungstenite::Error::Io(e))
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        return Ok(Line::TimedOut)
                    }
                    Err(tungstenite::Error::Io(e))
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::UnexpectedEof
                        ) =>
                    {
                        return Ok(Line::Eof)
                    }
                    Err(tungstenite::Error::Capacity(_)) => return Err(cap_error()),
                    Err(e) => return Err(format!("reading from the relay failed: {e}")),
                }
            }
        }
    }

    /// Open a WebSocket to `url`, giving up after `timeout`: the TCP
    /// connect under a deadline, then the handshake under what is left
    /// of it.
    pub(super) fn connect(url: &str, timeout: Duration) -> Result<Box<dyn Link>, String> {
        let deadline = Instant::now() + timeout;
        // What the errors below show of `url`: the token stays out of
        // every message (`wire::redacted`).
        let shown = redacted(url);
        let (host, port) = host_port(url)?;
        let addresses: Vec<SocketAddr> = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| format!("cannot resolve {host}:{port} from {shown}: {e}"))?
            .collect();
        let mut last = None;
        let mut stream = None;
        for address in &addresses {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match TcpStream::connect_timeout(address, remaining) {
                Ok(connected) => {
                    stream = Some(connected);
                    break;
                }
                Err(e) => last = Some(e),
            }
        }
        let Some(stream) = stream else {
            return Err(match last {
                Some(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => format!(
                    "nothing is listening at {host}:{port} ({shown}): the relay is not running. {BRIDGE_HINT}"
                ),
                Some(e) => format!("could not connect to {shown}: {e}. {BRIDGE_HINT}"),
                None => format!("{shown} resolves to no address"),
            });
        };
        let _ = stream.set_nodelay(true);
        let remaining = deadline.saturating_duration_since(Instant::now());
        stream
            .set_read_timeout(Some(remaining.max(Duration::from_millis(1))))
            .and_then(|()| stream.set_write_timeout(Some(remaining.max(Duration::from_millis(1)))))
            .map_err(|e| format!("could not set the handshake deadline: {e}"))?;
        let config = WebSocketConfig::default().max_message_size(Some(FRAME_CAP));
        let (socket, _response) =
            tungstenite::client::client_with_config(url, Counted { stream, written: 0 }, Some(config))
                .map_err(|e| handshake_failed(&shown, e))?;
        Ok(Box::new(WsLink { socket }))
    }

    /// The host and port a `ws://` or `wss://` URL names. `wss://` is
    /// refused: the relay is loopback and plain, and this client links
    /// no TLS.
    fn host_port(url: &str) -> Result<(String, u16), String> {
        let shown = redacted(url);
        let Some(rest) = url.strip_prefix("ws://") else {
            if url.starts_with("wss://") {
                return Err(format!(
                    "{shown}: wss:// is not supported; the relay is a plain ws:// on loopback"
                ));
            }
            return Err(format!(
                "{shown} is not a WebSocket URL: it must start with ws://, e.g. ws://127.0.0.1:41816/controller/<token>"
            ));
        };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !host.ends_with(']') || host.starts_with('[') => (
                host,
                port.parse::<u16>()
                    .map_err(|_| format!("{shown}: `{port}` is not a port number"))?,
            ),
            _ => (authority, 80),
        };
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if host.is_empty() {
            return Err(format!("{shown} names no host"));
        }
        Ok((host.to_string(), port))
    }

    /// Why the handshake failed, in words that name the next step: a
    /// refused role (the relay's 4xx) is told apart from a peer that
    /// is not a relay at all. `shown` is the URL as errors show it.
    fn handshake_failed(
        shown: &str,
        error: tungstenite::handshake::HandshakeError<tungstenite::ClientHandshake<Counted>>,
    ) -> String {
        use tungstenite::handshake::HandshakeError;
        match error {
            HandshakeError::Failure(tungstenite::Error::Http(response)) => {
                let body = response
                    .body()
                    .as_deref()
                    .map(|bytes| String::from_utf8_lossy(bytes).trim().to_string())
                    .unwrap_or_default();
                format!(
                    "the relay refused the WebSocket handshake for {shown} ({}): {body}",
                    response.status()
                )
            }
            HandshakeError::Failure(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                format!("no WebSocket handshake reply from {shown} in time. Is that a mandalactl bridge? {BRIDGE_HINT}")
            }
            HandshakeError::Interrupted(_) => {
                format!("the WebSocket handshake with {shown} did not complete in time")
            }
            HandshakeError::Failure(e) => {
                format!("the WebSocket handshake with {shown} failed: {e}. {BRIDGE_HINT}")
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_a_client_frame_is_its_header_mask_and_payload() {
            assert_eq!(frame_len(0), 6);
            assert_eq!(frame_len(125), 2 + 4 + 125);
            assert_eq!(frame_len(126), 2 + 2 + 4 + 126, "two extended length bytes");
            assert_eq!(frame_len(65_535), 2 + 2 + 4 + 65_535);
            assert_eq!(
                frame_len(65_536),
                2 + 8 + 4 + 65_536,
                "eight extended length bytes"
            );
        }

        #[test]
        fn test_host_and_port_are_read_from_a_ws_url() {
            assert_eq!(
                host_port("ws://127.0.0.1:41816/controller/t0k"),
                Ok(("127.0.0.1".into(), 41816))
            );
            assert_eq!(host_port("ws://localhost/x"), Ok(("localhost".into(), 80)));
            assert_eq!(host_port("ws://[::1]:7/x"), Ok(("::1".into(), 7)));
            assert!(host_port("wss://127.0.0.1:1/x")
                .expect_err("no TLS")
                .contains("wss://"));
            assert!(host_port("http://127.0.0.1:1/x")
                .expect_err("not ws")
                .contains("ws://"));
            assert!(host_port("ws://127.0.0.1:port/x").is_err());
            assert!(host_port("ws:///x").is_err());
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod ws {
    use std::time::Duration;

    use super::Link;

    pub(super) fn connect(_url: &str, _timeout: Duration) -> Result<Box<dyn Link>, String> {
        Err("this build of mandalactl links no WebSocket client; the browser is the app's side of the relay, \
             not the controller's"
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    /// A sink that takes `room` bytes and then fails with `kind`.
    fn limited(room: usize, kind: ErrorKind) -> impl FnMut(&[u8], Duration) -> std::io::Result<usize> {
        let mut left = room;
        move |chunk, _| {
            if left == 0 {
                return Err(kind.into());
            }
            let n = chunk.len().min(left).min(7);
            left -= n;
            Ok(n)
        }
    }

    fn later() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    #[test]
    fn test_a_write_that_completes_reports_nothing() {
        let bytes = vec![b'x'; 100];
        assert_eq!(
            write_all_within(&mut limited(100, ErrorKind::WouldBlock), &bytes, later()),
            Ok(())
        );
    }

    #[test]
    fn test_a_write_reports_how_far_it_got_and_why_it_stopped() {
        let bytes = vec![b'x'; 100];
        assert_eq!(
            write_all_within(&mut limited(40, ErrorKind::WouldBlock), &bytes, later()),
            Err((40, ErrorKind::TimedOut)),
            "a write that would block is the deadline"
        );
        assert_eq!(
            write_all_within(&mut limited(40, ErrorKind::BrokenPipe), &bytes, later()),
            Err((40, ErrorKind::BrokenPipe))
        );
    }

    #[test]
    fn test_a_passed_deadline_stops_before_the_first_byte() {
        let bytes = vec![b'x'; 10];
        let passed = Instant::now() - Duration::from_secs(1);
        assert_eq!(
            write_all_within(&mut limited(10, ErrorKind::WouldBlock), &bytes, passed),
            Err((0, ErrorKind::TimedOut))
        );
    }

    /// The boundary between "cannot have run" and "may have": the end
    /// of the body, not the newline after it.
    #[test]
    fn test_a_send_that_stops_before_the_body_ends_is_not_sent_and_after_it_is_unanswered() {
        let body = 99; // a 99-byte line plus its terminator is 100 bytes
        let why = || "stopped".to_string();
        assert!(matches!(send_failure(0, body, why()), Outcome::NotSent(_)));
        assert!(matches!(send_failure(98, body, why()), Outcome::NotSent(_)));
        assert!(
            matches!(send_failure(99, body, why()), Outcome::Unanswered(_)),
            "body complete, terminator unwritten: the server reads EOF as the end of the line and runs it"
        );
    }

    #[test]
    fn test_a_stop_on_the_terminator_alone_is_unanswered_end_to_end() {
        let body = vec![b'x'; 99];
        let mut frame = body.clone();
        frame.push(b'\n');
        let Err((written, _)) = write_all_within(&mut limited(99, ErrorKind::TimedOut), &frame, later())
        else {
            panic!("the sink refuses the terminator");
        };
        assert_eq!(written, 99);
        assert!(matches!(
            send_failure(written, body.len(), String::new()),
            Outcome::Unanswered(_)
        ));
    }

    #[test]
    fn test_the_relays_no_app_refusal_names_the_remedy_and_others_are_reported_as_given() {
        let no_app = rejected_for("no_app");
        assert!(
            no_app.contains("no browser app") && no_app.contains("?ipc="),
            "{no_app}"
        );
        let other = rejected_for("shutting_down");
        assert!(
            other.contains("shutting_down") && other.contains("retrying will not change"),
            "{other}"
        );
    }
}
