// SPDX-License-Identifier: MPL-2.0

//! The two bootstrap commands, `describe` and `ping` — the whole
//! command surface IPC-02 ships, enough to prove the transport and
//! the event-loop wake end to end.
//!
//! They are dispatched by hand here because the command registry
//! does not exist yet: IPC-03 builds it (`registry.rs`, with
//! compile-time descriptors and the reserved-word check) and moves
//! these two into `commands/meta.rs` as its first entries
//! (`format/ipc.md` §Command registry and `describe`). Until then
//! [`COMMANDS`] is the table `describe` renders and [`execute`]
//! dispatches against, so the discovery surface and the dispatch
//! surface are one list, as the registry will make them by
//! construction — and the tests below hold that list to the document.

use serde_json::{json, Map, Value};

use super::envelope::{code, IpcRequest, Reply, PROTOCOL_VERSION};
use super::Executed;

/// One parameter, as `describe` publishes it.
struct ParamDoc {
    name: &'static str,
    /// One of the protocol's type words: `string`, `integer`,
    /// `number`, `boolean`, `object`, `array`.
    ty: &'static str,
    required: bool,
    doc: &'static str,
}

/// One result field, as `describe` publishes it.
struct ResultDoc {
    name: &'static str,
    /// A type word, as for [`ParamDoc::ty`].
    ty: &'static str,
    doc: &'static str,
}

/// One command, as `describe` publishes it.
struct CommandDoc {
    name: &'static str,
    summary: &'static str,
    params: &'static [ParamDoc],
    result: &'static [ResultDoc],
}

/// The family `describe` reports for the two unnamespaced commands —
/// the "(bootstrap)" row of the family map.
const BOOTSTRAP_FAMILY: &str = "bootstrap";

/// Every command this build registers, in name order.
const COMMANDS: &[CommandDoc] = &[
    CommandDoc {
        name: "describe",
        summary: "Describe the command surface this build registers and the events it can send.",
        params: &[ParamDoc {
            name: "command",
            ty: "string",
            required: false,
            doc: "Describe just this command instead of the whole table. Does not narrow `events`.",
        }],
        result: &[
            ResultDoc {
                name: "protocol",
                ty: "integer",
                doc: "The protocol integer, as in `hello`.",
            },
            ResultDoc {
                name: "app",
                ty: "string",
                doc: "Always \"mandala\".",
            },
            ResultDoc {
                name: "version",
                ty: "string",
                doc: "The Cargo package version of this build.",
            },
            ResultDoc {
                name: "commands",
                ty: "array",
                doc: "One entry per command: name, family, summary, params, result.",
            },
            ResultDoc {
                name: "events",
                ty: "array",
                doc: "The complete event catalog: name and doc per class.",
            },
        ],
    },
    CommandDoc {
        name: "ping",
        summary: "Liveness probe; also the cheapest way to read the app clock.",
        params: &[],
        result: &[ResultDoc {
            name: "now_ms",
            ty: "number",
            doc: "The app's now_ms() clock, in milliseconds.",
        }],
    },
];

/// Every event class this build can send, with what it means.
const EVENTS: &[(&str, &str)] = &[
    (
        "connection_rejected",
        "Sent to a refused connection instead of `hello` while another controller is attached; then closed.",
    ),
    (
        "hello",
        "The greeting on accept: protocol, app, version, pid, and the loaded map's path (null before the first load).",
    ),
    (
        "shutdown",
        "Best-effort notice, before the socket closes, that the app is exiting.",
    ),
];

/// Execute one bootstrap request. Never panics: an unknown command, a
/// stray parameter or a mistyped value is an error reply, and the app
/// keeps running.
pub(crate) fn execute(request: &IpcRequest) -> Executed {
    let reply = match request.cmd.as_str() {
        "describe" => describe(request),
        "ping" => ping(request),
        other => Reply::error(
            request.id.clone(),
            code::UNKNOWN_CMD,
            format!(
                "no command '{other}' — this build registers {}; send `describe` for the full table",
                COMMANDS.iter().map(|c| c.name).collect::<Vec<_>>().join(", ")
            ),
            None,
        ),
    };
    // Neither bootstrap command changes what is on screen.
    Executed {
        reply,
        repaint: false,
    }
}

fn ping(request: &IpcRequest) -> Reply {
    if let Some(stray) = first_unknown_param(&request.params, &[]) {
        return stray_param(request, stray, "ping");
    }
    Reply::ok(
        request.id.clone(),
        json!({ "now_ms": crate::application::common::now_ms() }),
    )
}

fn describe(request: &IpcRequest) -> Reply {
    if let Some(stray) = first_unknown_param(&request.params, &["command"]) {
        return stray_param(request, stray, "describe");
    }
    let commands: Vec<Value> = match request.params.get("command") {
        None => COMMANDS.iter().map(describe_command).collect(),
        Some(Value::String(name)) => match COMMANDS.iter().find(|c| c.name == name) {
            Some(command) => vec![describe_command(command)],
            None => {
                return Reply::invalid_params(
                    request.id.clone(),
                    "command",
                    format!("no command '{name}' to describe; omit `command` for the full table"),
                )
            }
        },
        Some(_) => return Reply::invalid_params(request.id.clone(), "command", "`command` must be a string"),
    };
    let events: Vec<Value> = EVENTS
        .iter()
        .map(|(name, doc)| json!({ "name": name, "doc": doc }))
        .collect();
    Reply::ok(
        request.id.clone(),
        json!({
            "protocol": PROTOCOL_VERSION,
            "app": "mandala",
            "version": env!("CARGO_PKG_VERSION"),
            "commands": commands,
            "events": events,
        }),
    )
}

fn describe_command(command: &CommandDoc) -> Value {
    let params: Vec<Value> = command
        .params
        .iter()
        .map(|p| json!({ "name": p.name, "type": p.ty, "required": p.required, "doc": p.doc }))
        .collect();
    let result: Vec<Value> = command
        .result
        .iter()
        .map(|f| json!({ "name": f.name, "type": f.ty, "doc": f.doc }))
        .collect();
    json!({
        "name": command.name,
        "family": BOOTSTRAP_FAMILY,
        "summary": command.summary,
        "params": params,
        "result": result,
    })
}

/// The first parameter (in key order, so the answer is deterministic)
/// that is not in `allowed`.
fn first_unknown_param<'a>(params: &'a Map<String, Value>, allowed: &[&str]) -> Option<&'a str> {
    params
        .keys()
        .map(String::as_str)
        .find(|key| !allowed.contains(key))
}

fn stray_param(request: &IpcRequest, stray: &str, cmd: &str) -> Reply {
    Reply::invalid_params(
        request.id.clone(),
        stray,
        format!("`{cmd}` takes no parameter `{stray}`; `describe` lists what it accepts"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ipc::envelope::{decode_line, Decoded, Frame, OUTBOUND_FRAME_CAP};

    /// Run `frame` through the real decoder and `execute`, and return
    /// the reply as it would appear on the wire.
    fn run(frame: &str) -> Value {
        let Decoded::Request(request) = decode_line(frame.as_bytes(), 0, 1) else {
            panic!("{frame} must decode to a request");
        };
        let executed = execute(&request);
        assert!(!executed.repaint, "bootstrap commands never change pixels");
        let bytes = Frame::Reply(executed.reply)
            .encode(OUTBOUND_FRAME_CAP)
            .expect("encodes");
        serde_json::from_slice(&bytes).expect("JSON")
    }

    #[test]
    fn test_ping_reports_the_app_clock() {
        let before = crate::application::common::now_ms();
        let reply = run(r#"{"id":1,"cmd":"ping"}"#);
        assert_eq!(reply["id"], json!(1));
        assert_eq!(reply["ok"], json!(true));
        let now = reply["result"]["now_ms"].as_f64().expect("now_ms is a number");
        assert!(now >= before, "the clock is monotonic: {now} < {before}");
    }

    #[test]
    fn test_ping_rejects_any_parameter() {
        let reply = run(r#"{"id":"p","cmd":"ping","verbose":true}"#);
        assert_eq!(reply["id"], json!("p"));
        assert_eq!(reply["error"]["code"], json!("invalid_params"));
        assert_eq!(reply["error"]["data"]["param"], json!("verbose"));
    }

    #[test]
    fn test_unknown_command_is_unknown_cmd_naming_what_exists() {
        let reply = run(r#"{"id":2,"cmd":"clock.stepp"}"#);
        assert_eq!(reply["id"], json!(2));
        assert_eq!(reply["error"]["code"], json!("unknown_cmd"));
        let message = reply["error"]["message"].as_str().expect("message");
        assert!(
            message.contains("describe") && message.contains("ping"),
            "{message}"
        );
    }

    #[test]
    fn test_describe_renders_the_whole_table() {
        let reply = run(r#"{"id":3,"cmd":"describe"}"#);
        let result = &reply["result"];
        assert_eq!(result["protocol"], json!(PROTOCOL_VERSION));
        assert_eq!(result["app"], json!("mandala"));
        assert_eq!(result["version"], json!(env!("CARGO_PKG_VERSION")));
        let names: Vec<&str> = result["commands"]
            .as_array()
            .expect("commands array")
            .iter()
            .map(|c| c["name"].as_str().expect("name"))
            .collect();
        assert_eq!(names, ["describe", "ping"]);
        let describe = &result["commands"][0];
        assert_eq!(describe["family"], json!("bootstrap"));
        assert_eq!(
            describe["params"],
            json!([{
                "name": "command", "type": "string", "required": false,
                "doc": "Describe just this command instead of the whole table. Does not narrow `events`."
            }])
        );
        assert_eq!(result["commands"][1]["result"][0]["name"], json!("now_ms"));
        assert_eq!(result["events"].as_array().expect("events").len(), EVENTS.len());
    }

    #[test]
    fn test_describe_command_narrows_commands_but_not_events() {
        let reply = run(r#"{"id":4,"cmd":"describe","command":"ping"}"#);
        let commands = reply["result"]["commands"].as_array().expect("commands");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["name"], json!("ping"));
        assert_eq!(
            reply["result"]["events"].as_array().expect("events").len(),
            EVENTS.len()
        );
    }

    #[test]
    fn test_describe_rejects_an_unknown_or_mistyped_command_filter() {
        for frame in [
            r#"{"id":5,"cmd":"describe","command":"nope"}"#,
            r#"{"id":5,"cmd":"describe","command":7}"#,
        ] {
            let reply = run(frame);
            assert_eq!(reply["error"]["code"], json!("invalid_params"), "{frame}");
            assert_eq!(reply["error"]["data"]["param"], json!("command"), "{frame}");
        }
        let reply = run(r#"{"id":5,"cmd":"describe","cmd_name":"ping"}"#);
        assert_eq!(reply["error"]["data"]["param"], json!("cmd_name"));
    }

    #[test]
    fn test_describe_uses_only_the_protocol_type_words() {
        const TYPE_WORDS: &[&str] = &["string", "integer", "number", "boolean", "object", "array"];
        for command in COMMANDS {
            let fields = command.params.iter().map(|p| (p.name, p.ty));
            for (name, ty) in fields.chain(command.result.iter().map(|r| (r.name, r.ty))) {
                assert!(TYPE_WORDS.contains(&ty), "{}.{name}: {ty}", command.name);
            }
            for param in command.params {
                assert!(
                    param.name != "id" && param.name != "cmd",
                    "{} declares the reserved parameter `{}`",
                    command.name,
                    param.name
                );
            }
        }
    }

    /// **`describe` stays byte-honest against `format/ipc.md`.**
    ///
    /// The document's family map lists, for the bootstrap row, the
    /// commands this table must hold — no more, no fewer — and each of
    /// them has its own reference heading. Every event class this
    /// build can send is one the document reserves.
    #[test]
    fn test_describe_matches_the_documented_surface() {
        use baumhard::util::doc_fixtures::{documented_paragraph, names_token, repo_path, section_text};

        let doc = repo_path("format/ipc.md");
        let family_map = documented_paragraph(&doc, "## Command families", "| family | module");
        let row = family_map
            .lines()
            .find(|line| line.starts_with("| *(bootstrap)*"))
            .expect("the family map keeps a bootstrap row");
        let documented: Vec<&str> = row
            .rsplit('|')
            .nth(1)
            .expect("the row ends with its command column")
            .split(',')
            .map(|name| name.trim().trim_matches('`'))
            .collect();
        let registered: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
        assert_eq!(
            registered, documented,
            "the bootstrap row and `describe` disagree"
        );

        for command in COMMANDS {
            section_text(&doc, &format!("### `{}`", command.name));
        }

        let reserved = documented_paragraph(
            &doc,
            "## `events` — event stream + revision (IPC-14)",
            "Reserved event classes",
        );
        for (event, _) in EVENTS {
            assert!(
                names_token(&reserved, event),
                "event `{event}` is not a reserved class"
            );
        }
    }
}
