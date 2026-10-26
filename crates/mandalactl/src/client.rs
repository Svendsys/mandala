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
//! are. On Unix the stream is the socket, with its own read and write
//! timeouts; on Windows it is the app's named pipe (#169), whose
//! blocking reads and writes `pipe_wire` bounds with a reader and a
//! writer thread, so the connection above is the same code on both.
//! Elsewhere (the wasm32 type-check) [`Connection::open`] fails naming
//! the gap.

use std::time::Duration;

use crate::wire::Reply;

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

/// How far a request got — the fact that decides whether repeating it
/// is safe. Not an error type (CODE_CONVENTIONS §9): the two failures
/// carry the same plain `String` message every stage's failures do, and
/// differ in what they let the caller conclude.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The app answered.
    Answered(Exchange),
    /// The request's body did not go out whole, so it cannot have run:
    /// a truncated JSON object does not decode.
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

#[cfg(any(unix, windows))]
pub use self::stream::Connection;

#[cfg(not(any(unix, windows)))]
pub use self::unsupported::Connection;

#[cfg(any(unix, windows))]
mod stream {
    use std::io::{BufRead, BufReader, Write};
    #[cfg(unix)]
    use std::os::unix::net::UnixStream as Stream;
    use std::path::Path;
    #[cfg(unix)]
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use serde_json::Value;

    #[cfg(windows)]
    use super::pipe_wire::{connect, PipeWire as Stream};
    #[cfg(unix)]
    use super::LAUNCH_HINT;
    use super::{Exchange, Outcome, Watched, BUSY_RETRY};
    use crate::wire::{decode_frame, Event, Frame, Hello, Outgoing, FRAME_CAP};

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

    /// An open, greeted connection.
    #[derive(Debug)]
    pub struct Connection {
        reader: BufReader<Stream>,
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
                    Greeting::Rejected(reason) if reason != "busy" => {
                        return Err(format!(
                            "the app refused the connection (reason: {reason}); retrying will not change that"
                        ));
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
            if let Err(failed) = self.send(request, deadline, timeout) {
                return *failed;
            }
            loop {
                let line = match self.read_line(deadline) {
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
            match self.read_line(Instant::now() + wait)? {
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
            let why = match kind {
                std::io::ErrorKind::TimedOut => silence(&format!("room to send `{command}`"), timeout),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset => format!(
                    "the app closed the connection while `{command}` was being sent: it exited, or aborted"
                ),
                std::io::ErrorKind::WriteZero => {
                    format!("the app stopped reading while `{command}` was being sent")
                }
                other => format!("could not send `{command}`: {other}"),
            };
            // Bytes a write gave up on may still land, so they count as
            // sent: claiming `NotSent` for a request that then runs is the
            // one answer this must never give.
            let written = written + self.reader.get_ref().in_flight();
            // Boxed because an `Outcome` can hold a whole reply and this is
            // only ever a failure.
            Err(Box::new(send_failure(written, request.line.len(), why)))
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

    /// What a send that stopped after `written` bytes means for a request
    /// whose body is `body_len` bytes (its terminator is one more).
    fn send_failure(written: usize, body_len: usize, why: String) -> Outcome {
        if written >= body_len {
            Outcome::Unanswered(why)
        } else {
            Outcome::NotSent(why)
        }
    }

    fn cap_error() -> String {
        format!(
            "the app sent a frame over {FRAME_CAP} bytes, which its own cap forbids; refusing to buffer it"
        )
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
    #[cfg(unix)]
    fn connect(path: &Path, timeout: Duration) -> Result<Stream, String> {
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
                let chunk = wait(&self.chunks, self.read_timeout.get())??;
                self.read_pending = false;
                self.leftover = chunk;
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
mod unsupported {
    use std::path::Path;
    use std::time::Duration;

    use serde_json::Value;

    use super::{Outcome, Watched};
    use crate::wire::Outgoing;

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
            Err(
                "mandalactl cannot reach the app on this platform: the `--ipc` transport is a Unix socket, \
                 or a named pipe on Windows (format/ipc.md \"Transport\")"
                    .to_string(),
            )
        }

        pub fn next_id(&mut self) -> Value {
            match self.never {}
        }

        pub fn exchange(&mut self, _request: &Outgoing, _timeout: Duration) -> Outcome {
            match self.never {}
        }

        pub fn watch_next(&mut self, _wait: Duration) -> Result<Watched, String> {
            match self.never {}
        }
    }
}
