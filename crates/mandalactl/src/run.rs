// SPDX-License-Identifier: MPL-2.0

//! One invocation, start to exit code: parse the command line, build
//! the request, talk to the app, print the reply.
//!
//! **The exit code is the contract.** `0` — the app answered
//! `ok:true`. `1` — it answered `ok:false` ("the server said no").
//! `2` — it could not be talked to: not running, busy, silent, gone,
//! or speaking another protocol. `3` — the command line was wrong and
//! nothing was sent. An agent branches on these before it reads a word
//! of output, and `2` in particular never means "you typed something
//! wrong" — which is why a usage error is not `2`, where conventional
//! parsers put it.
//!
//! **Errors are `String`s, classified by the stage that made them**
//! (CODE_CONVENTIONS §9 allows no error types): a failure while
//! reading the command line is a `3`, one while reaching the app a `2`,
//! and a reply with `ok:false` a `1` — no enum of causes is needed
//! because the stage already says which.
//!
//! All output goes through the writers in [`Io`], so a test reads
//! exactly what a shell would have shown.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::cli::{self, Invocation, Options, Parsed};
use crate::client::Connection;
use crate::help;
use crate::params::{build_params, tokenize, Signature, Token};
use crate::render;
use crate::verbs::{self, Effect, Render};
use crate::wire::{self, ErrorBody, Reply, PROTOCOL, REQUEST_CAP};

/// How an invocation ended; [`Exit::code`] is the process exit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// The app answered `ok:true`.
    Success,
    /// The app answered `ok:false`.
    CommandError,
    /// The app could not be reached or understood.
    TransportError,
    /// The command line was wrong; nothing was sent.
    UsageError,
}

impl Exit {
    /// The process exit status.
    pub fn code(self) -> u8 {
        match self {
            Exit::Success => 0,
            Exit::CommandError => 1,
            Exit::TransportError => 2,
            Exit::UsageError => 3,
        }
    }
}

/// Where an invocation reads and writes.
pub struct Io<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    /// Read only by `raw -`.
    pub stdin: &'a mut dyn Read,
}

/// The extra patience `--wait-settled`'s wait gets beyond its own
/// deadline, so the app's structured `timeout` reply arrives before
/// this client gives up on the connection.
const WAIT_GRACE: Duration = Duration::from_secs(2);

/// The server's bound on `clock.wait`'s `timeout_ms`.
const WAIT_MAX_MS: u128 = 60_000;

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
    let Invocation {
        options,
        command,
        words,
    } = invocation;
    let prepared = match prepare(&command, &words, &options, io.stdin) {
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
    let (frame, render) = match finish(prepared, &mut connection, &options) {
        Ok(Ok(done)) => done,
        Ok(Err(why)) => return usage(io, &why),
        Err(why) => return transport(io, &why),
    };
    let exchange = match connection.exchange(&frame, options.timeout) {
        Ok(exchange) => exchange,
        Err(why) => return transport(io, &why),
    };
    if !report(io, &options, render, &exchange.reply, &exchange.line, None) {
        return Exit::CommandError;
    }
    if !options.wait_settled {
        return Exit::Success;
    }
    let wait = settle_frame(connection.next_id(), options.timeout);
    let exchange = match connection.exchange(&wait, options.timeout + WAIT_GRACE) {
        Ok(exchange) => exchange,
        Err(why) => return transport(io, &why),
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
        Exit::CommandError
    }
}

/// Whether the app serves `clock.wait`, asked *before* the command goes
/// out: a wait found missing after the command ran would exit `1` over a
/// command that succeeded, and an agent branching on that code would
/// retry a mutation that had already happened.
fn can_settle(connection: &mut Connection, timeout: Duration) -> Result<bool, String> {
    let id = connection.next_id();
    let mut params = Map::new();
    params.insert("command".to_string(), Value::String("clock.wait".to_string()));
    let request = wire::request_frame(&id, "describe", &params)?;
    Ok(connection.exchange(&request, timeout)?.reply.ok)
}

/// Everything that needs no connection: the verb's arguments typed
/// from the table, or `raw`'s frame read and checked.
fn prepare(
    command: &str,
    words: &[String],
    options: &Options,
    stdin: &mut dyn Read,
) -> Result<Prepared, String> {
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
        params: build_params(&Signature::of(verb), &tokens)?,
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

/// The request frame, once the connection can supply an id — and, for
/// a command the table lacks, the app's `describe` of it to type the
/// arguments with. The outer `Err` is a transport failure, the inner
/// a usage error.
fn finish(
    prepared: Prepared,
    connection: &mut Connection,
    options: &Options,
) -> Result<Result<(Value, Render), String>, String> {
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
            let frame = build_params(&signature, &tokens)
                .and_then(|params| wire::request_frame(&id, &command, &params));
            (frame, Render::Flat)
        }
    };
    // The cap is checked here, on the frame that will be sent, so a
    // request the server would refuse is a usage error and never goes.
    Ok(frame.and_then(|frame| wire::frame_line(&frame).map(|_| (frame, render))))
}

/// What the app says `command` takes, or [`Signature::unknown`] when
/// it says nothing — so a misspelled command reaches the server and is
/// answered with the server's own `unknown_cmd` and its suggestion.
fn described(connection: &mut Connection, command: &str, timeout: Duration) -> Result<Signature, String> {
    let id = connection.next_id();
    let mut params = Map::new();
    params.insert("command".to_string(), Value::String(command.to_string()));
    let request = wire::request_frame(&id, "describe", &params)?;
    let exchange = connection.exchange(&request, timeout)?;
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

/// `clock.wait until=settled`, patient for as long as the client is.
fn settle_frame(id: Value, timeout: Duration) -> Value {
    let wait_ms = timeout.as_millis().clamp(1, WAIT_MAX_MS);
    json!({"id": id, "cmd": "clock.wait", "until": "settled", "timeout_ms": wait_ms as u64})
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

fn usage(io: &mut Io<'_>, why: &str) -> Exit {
    emit(io.err, &format!("mandalactl: {why}\nTry `mandalactl --help`.\n"));
    Exit::UsageError
}

fn transport(io: &mut Io<'_>, why: &str) -> Exit {
    emit(io.err, &format!("mandalactl: {why}\n"));
    Exit::TransportError
}

/// Write, ignoring failure: a reader that went away (`| head`) is not
/// an error worth a panic or a different exit code.
fn emit(writer: &mut dyn Write, text: &str) {
    let _ = writer.write_all(text.as_bytes());
}
