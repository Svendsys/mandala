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
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use serde_json::Value;

    use super::{Exchange, Outcome, BUSY_RETRY, LAUNCH_HINT};
    use crate::wire::{decode_frame, Event, Frame, Hello, Outgoing, FRAME_CAP};

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

#[cfg(not(unix))]
mod unsupported {
    use std::path::Path;
    use std::time::Duration;

    use serde_json::Value;

    use super::Outcome;
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
            Err("mandalactl cannot reach the app on this platform yet: Windows needs the named-pipe adapter, \
                 tracked in #169 (format/ipc.md \"Windows\")"
                .to_string())
        }

        pub fn next_id(&mut self) -> Value {
            match self.never {}
        }

        pub fn exchange(&mut self, _request: &Outgoing, _timeout: Duration) -> Outcome {
            match self.never {}
        }
    }
}
