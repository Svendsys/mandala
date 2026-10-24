// SPDX-License-Identifier: MPL-2.0

//! One invocation, start to exit code: parse the command line, build
//! the request, talk to the app, print the reply.
//!
//! **The exit code is the contract.** `0` — the app answered
//! `ok:true`. `1` — it answered `ok:false` ("the server said no").
//! `2` — it could not be reached, or the request did not go out whole:
//! not running, busy, refused, or speaking another protocol — nothing
//! can have run, so repeating is safe. `3` — the command line was wrong
//! and nothing was sent. `4` — the request *did* go out and the
//! invocation did not complete: no reply came, or the `--wait-settled`
//! wait failed after a command that succeeded — it may have run (or,
//! for the wait, it did), so a caller reads the app's state before
//! repeating anything that changes it. An agent branches on these
//! before it reads a word of output, which is why `2` never means "you
//! typed something wrong" (that is `3`, where conventional parsers put
//! `2`) and never means "this may have happened" (that is `4`).
//!
//! **Errors are `String`s, classified by the stage that made them**
//! (CODE_CONVENTIONS §9 allows no error types): a failure while
//! reading the command line is a `3`, one before the request left a
//! `2`, one after it left a `4`, and a reply with `ok:false` a `1` — no
//! enum of causes is needed because the stage already says which.
//!
//! All output goes through the writers in [`Io`], so a test reads
//! exactly what a shell would have shown.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::assemble;
use crate::cli::{self, Invocation, Options, Parsed};
use crate::client::{Connection, Exchange, Outcome, Watched};
use crate::help;
use crate::params::{build_params, tokenize, Signature, Token};
use crate::render;
use crate::verbs::{self, Effect, Render};
use crate::wire::{self, ErrorBody, Outgoing, Reply, PROTOCOL, REQUEST_CAP};

/// How an invocation ended; [`Exit::code`] is the process exit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// The app answered `ok:true`.
    Success,
    /// The app answered `ok:false`.
    CommandError,
    /// The app could not be reached or understood, and the request did
    /// not go out whole: nothing ran.
    TransportError,
    /// The command line was wrong; nothing was sent.
    UsageError,
    /// The request went out and the invocation did not complete: no
    /// reply, or the settle wait failed after the command succeeded.
    Unconfirmed,
}

impl Exit {
    /// The process exit status.
    pub fn code(self) -> u8 {
        match self {
            Exit::Success => 0,
            Exit::CommandError => 1,
            Exit::TransportError => 2,
            Exit::UsageError => 3,
            Exit::Unconfirmed => 4,
        }
    }
}

/// Where an invocation reads and writes.
pub struct Io<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    /// The shell's working directory, against which a relative path
    /// argument is made absolute; `None` when it could not be read.
    pub cwd: Option<&'a Path>,
    /// Read only by `raw -`.
    pub stdin: &'a mut dyn Read,
}

/// The extra patience a `clock.wait` — `--wait-settled`'s or one asked
/// for by name — gets beyond its own deadline, so the app's structured
/// `timeout` reply arrives before this client gives up on the
/// connection.
const WAIT_GRACE: Duration = Duration::from_secs(2);

/// How long one wait for an event lasts before the watch looks again.
/// A watch has no deadline of its own: it ends on `--count`, on the app
/// going away, or when its reader does.
const WATCH_POLL: Duration = Duration::from_secs(60);

/// The server's bound on `clock.wait`'s `timeout_ms`.
const WAIT_MAX_MS: u128 = 60_000;

/// The `timeout_ms` the server gives a `clock.wait` that names none.
const WAIT_DEFAULT_MS: u64 = 10_000;

const NO_SOCKET: &str = "no socket path. Pass --ipc <path> or set MANDALA_IPC. Start the app with: \
                         mandala <map.mindmap.json> --ipc \"$XDG_RUNTIME_DIR/mandala.sock\"";

/// Run one invocation. `args` are the command line without the
/// program name; `env_ipc` is `$MANDALA_IPC`, if set.
pub fn run(args: &[String], env_ipc: Option<&str>, io: &mut Io<'_>) -> Exit {
    match cli::parse(args) {
        Err(why) => usage(io, &why),
        Ok(Parsed::Version) => {
            emit(
                io.out,
                &format!("mandalactl {} (protocol {PROTOCOL})\n", env!("CARGO_PKG_VERSION")),
            );
            Exit::Success
        }
        Ok(Parsed::Help(None)) => {
            emit(io.out, &help::overview());
            Exit::Success
        }
        Ok(Parsed::Help(Some(name))) => match help::command_help(&name) {
            Ok(text) => {
                emit(io.out, &text);
                Exit::Success
            }
            Err(why) => usage(io, &why),
        },
        Ok(Parsed::Invoke(invocation)) => invoke(invocation, env_ipc, io),
    }
}

/// What to send, as far as the command line alone can tell.
enum Prepared {
    /// A verb the table knows: parameters built, ready to send.
    Typed {
        command: String,
        params: Map<String, Value>,
        render: Render,
    },
    /// `raw`: a whole frame, as the user wrote it.
    Raw {
        frame: Map<String, Value>,
        render: Render,
    },
    /// A command the table does not know: its arguments can be typed
    /// only once the app has described it.
    Undescribed { command: String, tokens: Vec<Token> },
}

fn invoke(invocation: Invocation, env_ipc: Option<&str>, io: &mut Io<'_>) -> Exit {
    // The one command that is the client's own: it reads a recording's
    // files and never opens the socket, so it needs no app to be
    // running and no `--ipc`.
    if invocation.command == assemble::COMMAND {
        return assemble::run(&invocation, io);
    }
    let Invocation {
        options,
        command,
        words,
    } = invocation;
    let prepared = match prepare(&command, &words, &options, io.cwd, io.stdin) {
        Ok(prepared) => prepared,
        Err(why) => return usage(io, &why),
    };
    let socket = match socket_path(&options, env_ipc) {
        Ok(path) => path,
        Err(why) => return usage(io, &why),
    };
    let (mut connection, note) = match Connection::open(&socket, options.timeout, options.force) {
        Ok(opened) => opened,
        Err(why) => return transport(io, &why),
    };
    if let Some(note) = note {
        emit(io.err, &format!("warning: {note}\n"));
    }
    if options.wait_settled {
        match can_settle(&mut connection, options.timeout) {
            Ok(true) => {}
            Ok(false) => {
                emit(
                    io.err,
                    "mandalactl: --wait-settled needs `clock.wait`, which this app does not serve; nothing was sent. \
                     Run the command without --wait-settled\n",
                );
                return Exit::CommandError;
            }
            Err(why) => return transport(io, &why),
        }
    }
    let (request, render) = match finish(prepared, &mut connection, io.cwd, &options) {
        Ok(Ok(done)) => done,
        Ok(Err(why)) => return usage(io, &why),
        Err(why) => return transport(io, &why),
    };
    let exchange = match connection.exchange(&request, patience(&request, options.timeout)) {
        Outcome::Answered(exchange) => exchange,
        Outcome::NotSent(why) => return transport(io, &why),
        Outcome::Unanswered(why) => return unconfirmed(io, &why),
    };
    if !report(io, &options, render, &exchange.reply, &exchange.line, None) {
        return Exit::CommandError;
    }
    // A subscription lives as long as its connection, so a subscribe
    // that returned here would drop what it just set up.
    if command == WATCHING_COMMAND {
        return watch(connection, io, &options);
    }
    if !options.wait_settled {
        return Exit::Success;
    }
    let wait = match settle_request(connection.next_id(), options.timeout) {
        Ok(wait) => wait,
        Err(why) => return unsettled(io, &why),
    };
    let exchange = match connection.exchange(&wait, patience(&wait, options.timeout)) {
        Outcome::Answered(exchange) => exchange,
        Outcome::NotSent(why) | Outcome::Unanswered(why) => return unsettled(io, &why),
    };
    let settled = report(
        io,
        &options,
        Render::Flat,
        &exchange.reply,
        &exchange.line,
        Some("wait"),
    );
    if settled {
        Exit::Success
    } else {
        unsettled(io, "the wait for the app to settle failed (above)")
    }
}

/// The command after which the invocation stays connected and prints
/// what the app pushes.
const WATCHING_COMMAND: &str = "events.subscribe";

/// Print each event the app pushes until `--count` of them have, the
/// app says it is shutting down, or whoever reads stdout goes away.
///
/// The subscribing reply is already printed. An event is one line:
/// with `--json` the frame as received, otherwise its class and its
/// data as JSON. `shutdown` is the app ending the watch and is not
/// counted or printed as an event; a connection that closes without it
/// is the app aborting, which is a transport fault (`2`), since a
/// subscription is safe to repeat.
fn watch(mut connection: Connection, io: &mut Io<'_>, options: &Options) -> Exit {
    let mut printed = 0u64;
    while options.count.is_none_or(|count| printed < count) {
        match connection.watch_next(WATCH_POLL) {
            Err(why) => return transport(io, &why),
            Ok(Watched::Quiet) => {}
            Ok(Watched::Closed) => {
                return transport(
                    io,
                    "the app closed the connection without saying it was shutting down: it aborted",
                )
            }
            Ok(Watched::Event { event, line }) => {
                if event.name == "shutdown" {
                    emit(
                        io.err,
                        "mandalactl: the app is shutting down; the watch is over\n",
                    );
                    return Exit::Success;
                }
                let text = if options.json {
                    format!("{line}\n")
                } else {
                    format!("{} {}\n", event.name, event.data)
                };
                if io.out.write_all(text.as_bytes()).is_err() || io.out.flush().is_err() {
                    return Exit::Success;
                }
                printed += 1;
            }
        }
    }
    Exit::Success
}

/// Whether the app serves `clock.wait`, asked *before* the command goes
/// out: a wait found missing after the command ran would leave a
/// command that succeeded reported as failed, and a caller branching on
/// that would repeat a mutation that had already happened.
fn can_settle(connection: &mut Connection, timeout: Duration) -> Result<bool, String> {
    Ok(describe_one(connection, "clock.wait", timeout)?.reply.ok)
}

/// Everything that needs no connection: the verb's arguments typed
/// from the table, or `raw`'s frame read and checked.
fn prepare(
    command: &str,
    words: &[String],
    options: &Options,
    cwd: Option<&Path>,
    stdin: &mut dyn Read,
) -> Result<Prepared, String> {
    if options.count.is_some() && command != WATCHING_COMMAND {
        return Err(format!(
            "--count counts the events a watch prints, and only `{WATCHING_COMMAND}` watches"
        ));
    }
    if command == "raw" {
        return raw_frame(words, options, stdin);
    }
    if options.wait_settled {
        refuse_wait_on_read_only(command)?;
    }
    let tokens = tokenize(words);
    let Some(verb) = verbs::lookup(command) else {
        return Ok(Prepared::Undescribed {
            command: command.to_string(),
            tokens,
        });
    };
    Ok(Prepared::Typed {
        command: command.to_string(),
        params: build_params(&Signature::of(verb), &tokens, cwd)?,
        render: verb.render,
    })
}

/// `--wait-settled` on a command the table knows only reads: refused,
/// the same for every spelling that reaches it, `raw` included. A
/// command the table lacks is let through, since nothing says whether
/// it changes the app.
fn refuse_wait_on_read_only(command: &str) -> Result<(), String> {
    match verbs::lookup(command) {
        Some(verb) if verb.effect == Effect::Reads => Err(format!(
            "--wait-settled does nothing for `{command}`, which does not change the app. Put it on the command that \
             does (e.g. input.click_node ... --wait-settled), then read"
        )),
        _ => Ok(()),
    }
}

fn raw_frame(words: &[String], options: &Options, stdin: &mut dyn Read) -> Result<Prepared, String> {
    // `--` ends flags for every command, `raw` included; the parser
    // keeps it for the typed path's tokenizer, which raw never runs.
    let words = words.strip_prefix(&["--".to_string()]).unwrap_or(words);
    let [source] = words else {
        return Err(
            "`raw` takes exactly one argument, the JSON request (quote it as one word), or `-` to read stdin"
                .to_string(),
        );
    };
    let text = if source == "-" {
        let mut bytes = Vec::new();
        stdin
            .take(REQUEST_CAP as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("could not read the request from stdin: {e}"))?;
        if bytes.len() > REQUEST_CAP {
            return Err(format!(
                "the request on stdin is over {REQUEST_CAP} bytes (1 MiB), the longest line the server reads"
            ));
        }
        String::from_utf8(bytes).map_err(|_| "the request on stdin is not UTF-8".to_string())?
    } else {
        source.clone()
    };
    if text.trim().is_empty() {
        return Err("the request is empty".to_string());
    }
    let Value::Object(frame) =
        serde_json::from_str(&text).map_err(|e| format!("the request is not valid JSON: {e}"))?
    else {
        return Err("the request must be a JSON object, e.g. {\"cmd\":\"ping\"}".to_string());
    };
    let Some(command) = frame.get("cmd").and_then(Value::as_str) else {
        return Err("the request has no string \"cmd\"".to_string());
    };
    if options.wait_settled {
        refuse_wait_on_read_only(command)?;
    }
    let render = verbs::lookup(command).map_or(Render::Flat, |verb| verb.render);
    Ok(Prepared::Raw { frame, render })
}

/// The request, once the connection can supply an id — and, for a
/// command the table lacks, the app's `describe` of it to type the
/// arguments with. The outer `Err` is a transport failure, the inner a
/// usage error.
fn finish(
    prepared: Prepared,
    connection: &mut Connection,
    cwd: Option<&Path>,
    options: &Options,
) -> Result<Result<(Outgoing, Render), String>, String> {
    let (frame, render) = match prepared {
        Prepared::Typed {
            command,
            params,
            render,
        } => {
            let id = connection.next_id();
            (wire::request_frame(&id, &command, &params), render)
        }
        Prepared::Raw { mut frame, render } => {
            if !frame.contains_key("id") {
                frame.insert("id".to_string(), connection.next_id());
            }
            (Ok(Value::Object(frame)), render)
        }
        Prepared::Undescribed { command, tokens } => {
            let signature = described(connection, &command, options.timeout)?;
            let id = connection.next_id();
            let frame = build_params(&signature, &tokens, cwd)
                .and_then(|params| wire::request_frame(&id, &command, &params));
            (frame, Render::Flat)
        }
    };
    // Serialized once, here, with the server's line cap checked on the
    // way: a request it would refuse is a usage error and never goes.
    Ok(frame
        .and_then(|frame| wire::outgoing(&frame))
        .map(|request| (request, render)))
}

/// What the app says `command` takes, or [`Signature::unknown`] when
/// it says nothing — so a misspelled command reaches the server and is
/// answered with the server's own `unknown_cmd` and its suggestion.
fn described(connection: &mut Connection, command: &str, timeout: Duration) -> Result<Signature, String> {
    let exchange = describe_one(connection, command, timeout)?;
    let entry = exchange
        .reply
        .result
        .get("commands")
        .and_then(Value::as_array)
        .and_then(|commands| commands.first())
        .filter(|_| exchange.reply.ok);
    match entry {
        Some(entry) => Signature::from_describe(entry),
        None => Ok(Signature::unknown(command)),
    }
}

/// `describe command=<name>`: the one place that request is built, so
/// the capability check and the argument typing cannot disagree about
/// how the app is asked. A read-only exchange, so a failure at either
/// stage is the same transport fault.
fn describe_one(connection: &mut Connection, name: &str, timeout: Duration) -> Result<Exchange, String> {
    let id = connection.next_id();
    let mut params = Map::new();
    params.insert("command".to_string(), Value::String(name.to_string()));
    let request = wire::outgoing(&wire::request_frame(&id, "describe", &params)?)?;
    match connection.exchange(&request, timeout) {
        Outcome::Answered(exchange) => Ok(exchange),
        Outcome::NotSent(why) | Outcome::Unanswered(why) => Err(why),
    }
}

/// How long to wait for `request`'s reply: `timeout` (`--timeout`),
/// unless it is a `clock.wait` whose own deadline, plus [`WAIT_GRACE`],
/// is later. The app holds a wait's reply until its condition holds or
/// its `timeout_ms` passes, and answers the latter with a structured
/// `timeout`; a client that gave up first would report an exchange it
/// cannot confirm instead of the app's answer. A `timeout_ms` the app
/// will refuse is refused at once, so the server's bound caps it.
fn patience(request: &Outgoing, timeout: Duration) -> Duration {
    let frame: Value = serde_json::from_str(&request.line).unwrap_or(Value::Null);
    if frame["cmd"] != "clock.wait" {
        return timeout;
    }
    let wait_ms = frame["timeout_ms"].as_u64().unwrap_or(WAIT_DEFAULT_MS);
    let wait = Duration::from_millis(wait_ms.min(WAIT_MAX_MS as u64));
    timeout.max(wait + WAIT_GRACE)
}

/// `clock.wait until=settled`, patient for as long as the client is.
fn settle_request(id: Value, timeout: Duration) -> Result<Outgoing, String> {
    let wait_ms = timeout.as_millis().clamp(1, WAIT_MAX_MS);
    wire::outgoing(&json!({"id": id, "cmd": "clock.wait", "until": "settled", "timeout_ms": wait_ms as u64}))
}

fn socket_path(options: &Options, env_ipc: Option<&str>) -> Result<PathBuf, String> {
    let chosen = options
        .ipc
        .as_deref()
        .filter(|p| !p.is_empty())
        .or(env_ipc.filter(|p| !p.is_empty()));
    chosen.map(PathBuf::from).ok_or_else(|| NO_SOCKET.to_string())
}

/// Print one reply the way the mode asks. `true` when it was
/// `ok:true`. `prefix` roots a default-mode result under a name
/// (`wait`), so two results in one invocation stay distinguishable.
fn report(
    io: &mut Io<'_>,
    options: &Options,
    render: Render,
    reply: &Reply,
    line: &str,
    prefix: Option<&str>,
) -> bool {
    if options.json {
        emit(io.out, &format!("{line}\n"));
    } else {
        for warning in &reply.warnings {
            emit(
                io.err,
                &format!("warning: {}: {}\n", warning.code, warning.message),
            );
        }
        if reply.ok {
            let text = match prefix {
                Some(prefix) => render::prefixed(prefix, &reply.result),
                None => render::result_text(render, &reply.result),
            };
            emit(io.out, &text);
        }
    }
    if let Some(error) = &reply.error {
        emit(io.err, &error_line(error));
    }
    reply.ok
}

fn error_line(error: &ErrorBody) -> String {
    match &error.data {
        Some(data) => format!("error: {}: {} {data}\n", error.code, error.message),
        None => format!("error: {}: {}\n", error.code, error.message),
    }
}

pub(crate) fn usage(io: &mut Io<'_>, why: &str) -> Exit {
    emit(io.err, &format!("mandalactl: {why}\nTry `mandalactl --help`.\n"));
    Exit::UsageError
}

fn transport(io: &mut Io<'_>, why: &str) -> Exit {
    emit(io.err, &format!("mandalactl: {why}\n"));
    Exit::TransportError
}

/// The request went out whole and was not answered. Said in so many
/// words, because the one thing the caller must not do is assume it did
/// not run.
fn unconfirmed(io: &mut Io<'_>, why: &str) -> Exit {
    emit(
        io.err,
        &format!(
            "mandalactl: {why}\nThe request was sent, so it may have run. Read the app's state (a query) before \
             repeating a command that changes it.\n"
        ),
    );
    Exit::Unconfirmed
}

/// The command itself succeeded and the wait for the app to settle
/// did not. The opposite fact from [`unconfirmed`], and said the
/// opposite way: the command is done, and must not be repeated.
fn unsettled(io: &mut Io<'_>, why: &str) -> Exit {
    emit(
        io.err,
        &format!(
            "mandalactl: {why}\nThe command itself succeeded (its result is above); do not repeat it. \
             Poll a read-only command (query.animations, say) until the app looks settled.\n"
        ),
    );
    Exit::Unconfirmed
}

/// Write, ignoring failure: a reader that went away (`| head`) is not
/// an error worth a panic or a different exit code.
pub(crate) fn emit(writer: &mut dyn Write, text: &str) {
    let _ = writer.write_all(text.as_bytes());
}
