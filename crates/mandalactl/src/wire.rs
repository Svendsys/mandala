// SPDX-License-Identifier: MPL-2.0

//! The envelope as this client sees it: the request it writes, the
//! frames it reads back, and the greeting it checks before sending
//! anything (`format/ipc.md` §Envelope, §Handshake and versioning).
//!
//! Pure functions over values — no socket — so the encode/decode
//! contract is tested byte for byte without one. Decoding is
//! deliberately tolerant: the protocol grows by adding keys, commands
//! and warning codes, and "clients must tolerate unknown keys in
//! replies and events", so a frame is read for the keys this client
//! uses and every other key is ignored.

use serde_json::{Map, Value};

/// The one protocol integer this client speaks; the server's `hello`
/// must carry it (`--force` overrides). A test in the app crate holds
/// it equal to the server's own constant.
pub const PROTOCOL: u64 = 1;

/// The server's inbound line cap: a longer request is answered with a
/// `parse_error`, so the client refuses it first, by name.
pub const REQUEST_CAP: usize = 1 << 20;

/// The server's outbound frame cap: a reply larger than this is
/// replaced by `reply_too_large`, so no honest frame exceeds it and
/// the reader can bound its memory by it.
pub const FRAME_CAP: usize = 8 << 20;

/// The two control frames `mandalactl bridge` sends a browser app
/// (`format/ipc.md` §Transport "Browser"): a controller attached to
/// the relay, so the app greets it, and it left, so the app resets its
/// session. A JSON object with this one key and no other, which no
/// request is; a controller's frame that looks like one is dropped by
/// the relay. The app crate's tests hold its own spelling equal to
/// these.
pub const BRIDGE_KEY: &str = "bridge";
pub const BRIDGE_ATTACHED: &str = r#"{"bridge":"attached"}"#;
pub const BRIDGE_DETACHED: &str = r#"{"bridge":"detached"}"#;

/// Whether `text` is one of the relay's control frames, or a frame
/// a controller could pass off as one: a JSON object whose only key is
/// [`BRIDGE_KEY`].
pub fn is_bridge_frame(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| {
            value
                .as_object()
                .map(|object| object.len() == 1 && object.contains_key(BRIDGE_KEY))
        })
        .unwrap_or(false)
}

/// `url` with everything after its first path segment replaced by `…`:
/// `ws://127.0.0.1:41816/controller/<token>` → `ws://127.0.0.1:41816/controller/…`.
/// What an error line shows of a relay URL, so the token — the one
/// guard on the relay's port (`format/ipc.md` §Transport "Browser") —
/// is printed once, by the relay at startup, and never echoed into a
/// diagnostic a user pastes into a bug report. The app's browser socket
/// redacts its own logs by the same rule (`ipc::browser::redacted`;
/// the page cannot link this crate), and a test in the app crate holds
/// the two equal.
pub fn redacted(url: &str) -> String {
    let authority = url.find("://").map_or(0, |at| at + 3);
    let Some(path) = url[authority..].find('/').map(|at| authority + at) else {
        return url.to_string();
    };
    match url[path + 1..].find('/') {
        Some(at) => format!("{}/…", &url[..path + 1 + at]),
        None => url.to_string(),
    }
}

/// Decode standard base64 (`A-Z a-z 0-9 + /`, `=` padding, no line
/// breaks): what a browser's `canvas.toDataURL` carries after its
/// `data:image/png;base64,` prefix, and what `capture.screenshot`'s
/// `png_base64` is. `Err` names the first byte that is not base64.
/// Written here rather than linked: thirty lines, one caller.
pub fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let bytes = text.trim().as_bytes();
    let unpadded = bytes.iter().rposition(|b| *b != b'=').map_or(0, |last| last + 1);
    if bytes.len() - unpadded > 2 || unpadded % 4 == 1 {
        return Err("base64 has a length no encoding produces".to_string());
    }
    let mut out = Vec::with_capacity(unpadded / 4 * 3 + 2);
    let mut acc: u32 = 0;
    let mut held = 0;
    for (index, byte) in bytes[..unpadded].iter().enumerate() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(format!("byte {index} ({:?}) is not base64", char::from(*byte))),
        };
        acc = (acc << 6) | u32::from(value);
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((acc >> held) as u8);
            acc &= (1 << held) - 1;
        }
    }
    Ok(out)
}

/// The request frame for `cmd` with `params` flat beside `id` and
/// `cmd`.
///
/// Errors (a `String`, per CODE_CONVENTIONS §9) when a parameter is
/// named `id` or `cmd`, which would overwrite the envelope.
pub fn request_frame(id: &Value, cmd: &str, params: &Map<String, Value>) -> Result<Value, String> {
    let mut frame = Map::with_capacity(params.len() + 2);
    frame.insert("id".into(), id.clone());
    frame.insert("cmd".into(), Value::String(cmd.to_string()));
    for (name, value) in params {
        if name == "id" || name == "cmd" {
            return Err(format!(
                "`{name}` is an envelope key, not a parameter (format/ipc.md, the reserved-word rule)"
            ));
        }
        frame.insert(name.clone(), value.clone());
    }
    Ok(Value::Object(frame))
}

/// A whole frame as one wire line, refusing one the server would.
pub fn frame_line(frame: &Value) -> Result<String, String> {
    let line = frame.to_string();
    if line.len() > REQUEST_CAP {
        return Err(format!(
            "the request is {} bytes; the server reads lines of at most {REQUEST_CAP} (1 MiB). \
             Send large payloads by file where the command takes one (to_file=, path=)",
            line.len()
        ));
    }
    Ok(line)
}

/// A request ready to send: its line serialized once, with the `id`
/// and command it will be matched and described by.
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub id: Value,
    pub command: String,
    /// The frame as one wire line, no terminator.
    pub line: String,
}

/// Serialize `frame` (a whole request with an `id`) for sending,
/// checking the server's line cap on the way.
pub fn outgoing(frame: &Value) -> Result<Outgoing, String> {
    Ok(Outgoing {
        id: frame.get("id").cloned().unwrap_or(Value::Null),
        command: frame
            .get("cmd")
            .and_then(Value::as_str)
            .unwrap_or("the request")
            .to_string(),
        line: frame_line(frame)?,
    })
}

/// One server → client frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Reply(Reply),
    Event(Event),
}

/// A reply: exactly one of `result` / `error` is meaningful, by `ok`.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    /// Echo of the request id; `null` for an unparseable request.
    pub id: Value,
    pub ok: bool,
    /// The command's payload (`{}` when `ok` and the server sent none).
    pub result: Value,
    pub error: Option<ErrorBody>,
    pub warnings: Vec<Warning>,
}

/// An `ok:false` reply's error. Codes are API; messages are not.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub data: Option<Value>,
}

/// One entry of a reply's `warnings` array.
#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub code: String,
    pub message: String,
}

/// An unsolicited frame (`hello`, `shutdown`, `connection_rejected`).
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub name: String,
    pub data: Value,
}

/// Read one line as a frame. `Err` carries what was wrong with the
/// line — the server never sends such a frame, so it is a transport
/// fault, not a command error.
pub fn decode_frame(line: &str) -> Result<Frame, String> {
    let value: Value =
        serde_json::from_str(line).map_err(|e| format!("the server sent a line that is not JSON: {e}"))?;
    let Value::Object(mut frame) = value else {
        return Err("the server sent a JSON value that is not an object".to_string());
    };
    if let Some(name) = frame.get("event").and_then(Value::as_str) {
        let name = name.to_string();
        let data = frame.remove("data").unwrap_or_else(|| Value::Object(Map::new()));
        return Ok(Frame::Event(Event { name, data }));
    }
    let Some(ok) = frame.get("ok").and_then(Value::as_bool) else {
        return Err(
            "the server sent a frame that is neither an event nor a reply (no `event`, no boolean `ok`)"
                .to_string(),
        );
    };
    let id = frame.remove("id").unwrap_or(Value::Null);
    let warnings = frame
        .get("warnings")
        .and_then(Value::as_array)
        .map(|list| list.iter().map(warning_of).collect())
        .unwrap_or_default();
    if ok {
        let result = frame
            .remove("result")
            .unwrap_or_else(|| Value::Object(Map::new()));
        return Ok(Frame::Reply(Reply {
            id,
            ok,
            result,
            error: None,
            warnings,
        }));
    }
    let error = frame.get("error");
    let text = |key: &str| {
        error
            .and_then(|e| e.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let error = ErrorBody {
        code: text("code"),
        message: text("message"),
        data: frame.get("error").and_then(|e| e.get("data")).cloned(),
    };
    Ok(Frame::Reply(Reply {
        id,
        ok,
        result: Value::Object(Map::new()),
        error: Some(error),
        warnings,
    }))
}

fn warning_of(entry: &Value) -> Warning {
    let text = |key: &str| {
        entry
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Warning {
        code: text("code"),
        message: text("message"),
    }
}

/// The greeting a connection opens with.
#[derive(Debug, Clone, PartialEq)]
pub struct Hello {
    pub protocol: Option<u64>,
    pub app: String,
    pub version: String,
    pub pid: Option<u64>,
    /// The loaded map's path; `None` before the first load.
    pub map: Option<String>,
}

impl Hello {
    /// Read a `hello` event's data.
    pub fn of(data: &Value) -> Hello {
        let text = |key: &str| {
            data.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        Hello {
            protocol: data.get("protocol").and_then(Value::as_u64),
            app: text("app"),
            version: text("version"),
            pid: data.get("pid").and_then(Value::as_u64),
            map: data.get("map").and_then(Value::as_str).map(str::to_string),
        }
    }

    /// Whether this is a server the client understands: Mandala,
    /// speaking [`PROTOCOL`]. `Err` names what differs and the way
    /// out. `Ok(Some(note))` is a mismatch `force` overrode; the note
    /// is for stderr.
    pub fn accept(&self, force: bool) -> Result<Option<String>, String> {
        let mismatch = match (self.protocol, self.app.as_str()) {
            (Some(PROTOCOL), "mandala") => return Ok(None),
            (Some(other), "mandala") => format!(
                "the app speaks protocol {other}, this mandalactl speaks {PROTOCOL} (app version {})",
                self.version
            ),
            (None, _) => "the greeting names no protocol".to_string(),
            (Some(_), app) => format!("the peer calls itself `{app}`, not `mandala`"),
        };
        if force {
            Ok(Some(format!("{mismatch}; continuing because of --force")))
        } else {
            Err(format!(
                "{mismatch}. Use a mandalactl built from the same checkout as the app, or pass --force to try anyway"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn params(value: Value) -> Map<String, Value> {
        value.as_object().cloned().expect("an object")
    }

    /// The request line as the client sends it.
    fn encode_request(id: &Value, cmd: &str, params: &Map<String, Value>) -> Result<String, String> {
        frame_line(&request_frame(id, cmd, params)?)
    }

    #[test]
    fn test_base64_decodes_every_padding_shape_and_refuses_what_is_not_base64() {
        assert_eq!(base64_decode("").expect("empty"), b"");
        assert_eq!(base64_decode("Zg==").expect("one byte"), b"f");
        assert_eq!(base64_decode("Zm8=").expect("two bytes"), b"fo");
        assert_eq!(base64_decode("Zm9v").expect("three bytes"), b"foo");
        assert_eq!(base64_decode("Zm9vYmFy").expect("six bytes"), b"foobar");
        assert_eq!(base64_decode("Zm9vYg").expect("unpadded is tolerated"), b"foob");
        // A PNG signature, as a data URL carries it.
        assert_eq!(
            base64_decode("iVBORw0KGgo=").expect("a signature"),
            [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']
        );
        assert!(base64_decode("Zm9v*mFy")
            .expect_err("not base64")
            .contains("byte 4"));
        assert!(base64_decode("Z").is_err(), "one stray sextet encodes nothing");
        assert!(
            base64_decode("Zm9v*").is_err(),
            "a length no encoding produces, before any byte is looked at"
        );
        assert!(base64_decode("Zg===").is_err(), "three pads");
    }

    #[test]
    fn test_bridge_frames_are_recognized_and_nothing_else_is() {
        assert!(is_bridge_frame(BRIDGE_ATTACHED) && is_bridge_frame(BRIDGE_DETACHED));
        assert!(
            is_bridge_frame(r#"{"bridge":"anything"}"#),
            "any value under the key is the relay's shape"
        );
        for text in [
            r#"{"id":1,"cmd":"ping"}"#,
            r#"{"bridge":"attached","id":1}"#,
            "x",
            "[]",
        ] {
            assert!(!is_bridge_frame(text), "{text}");
        }
    }

    #[test]
    fn test_request_puts_params_flat_beside_id_and_cmd() {
        let line = encode_request(&json!(7), "clock.step", &params(json!({"ms": 100}))).expect("encodes");
        let back: Value = serde_json::from_str(&line).expect("one JSON object");
        assert_eq!(back, json!({"id": 7, "cmd": "clock.step", "ms": 100}));
        assert!(!line.contains('\n'), "a frame is one line");
    }

    #[test]
    fn test_request_round_trips_a_string_that_needs_escaping() {
        let text = "line one\nline \"two\"\t\u{1F44B}";
        let line =
            encode_request(&json!(1), "input.text", &params(json!({ "text": text }))).expect("encodes");
        assert!(!line.contains('\n'), "a raw newline never reaches the wire");
        let back: Value = serde_json::from_str(&line).expect("one JSON object");
        assert_eq!(back["text"], json!(text));
    }

    #[test]
    fn test_request_refuses_a_parameter_named_like_the_envelope() {
        for name in ["id", "cmd"] {
            let err = encode_request(&json!(1), "ping", &params(json!({ name: 1 }))).expect_err("reserved");
            assert!(err.contains(name) && err.contains("envelope"), "{err}");
        }
    }

    #[test]
    fn test_request_over_the_server_line_cap_is_refused_before_sending() {
        let big = "x".repeat(REQUEST_CAP);
        let err =
            encode_request(&json!(1), "input.text", &params(json!({ "text": big }))).expect_err("too big");
        assert!(err.contains("1 MiB"), "{err}");
    }

    #[test]
    fn test_decode_ok_reply_keeps_id_result_and_warnings() {
        let line = r#"{"id":3,"ok":true,"result":{"ran":true},"warnings":[{"code":"macro_gate_rejected","message":"m"}]}"#;
        let Frame::Reply(reply) = decode_frame(line).expect("a reply") else {
            panic!("not a reply");
        };
        assert_eq!(reply.id, json!(3));
        assert!(reply.ok && reply.error.is_none());
        assert_eq!(reply.result, json!({"ran": true}));
        assert_eq!(
            reply.warnings,
            vec![Warning {
                code: "macro_gate_rejected".into(),
                message: "m".into()
            }]
        );
    }

    #[test]
    fn test_decode_error_reply_carries_code_message_and_data() {
        let line = r#"{"id":"a","ok":false,"error":{"code":"timeout","message":"gave up","data":{"elapsed_ms":10000}}}"#;
        let Frame::Reply(reply) = decode_frame(line).expect("a reply") else {
            panic!("not a reply");
        };
        assert!(!reply.ok);
        assert_eq!(reply.id, json!("a"));
        let error = reply.error.expect("an error");
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("timeout", "gave up")
        );
        assert_eq!(error.data, Some(json!({"elapsed_ms": 10000})));
    }

    #[test]
    fn test_decode_parse_error_reply_has_a_null_id() {
        let line = r#"{"id":null,"ok":false,"error":{"code":"parse_error","message":"m","data":{"line":0}}}"#;
        let Frame::Reply(reply) = decode_frame(line).expect("a reply") else {
            panic!("not a reply");
        };
        assert_eq!(reply.id, Value::Null);
    }

    #[test]
    fn test_decode_event_frames() {
        let Frame::Event(event) = decode_frame(r#"{"event":"shutdown","data":{}}"#).expect("an event") else {
            panic!("not an event");
        };
        assert_eq!(event.name, "shutdown");
        let Frame::Event(bare) = decode_frame(r#"{"event":"x"}"#).expect("an event") else {
            panic!("not an event");
        };
        assert_eq!(bare.data, json!({}), "a missing data is an empty object");
    }

    #[test]
    fn test_decode_ignores_keys_it_does_not_know() {
        let line = r#"{"id":1,"ok":true,"result":{},"revision":42,"future":[1,2]}"#;
        assert!(matches!(decode_frame(line), Ok(Frame::Reply(r)) if r.ok));
    }

    #[test]
    fn test_decode_refuses_what_is_not_a_frame() {
        for line in ["", "nope", "[1]", "3", r#"{"id":1}"#, r#"{"ok":"yes"}"#] {
            assert!(decode_frame(line).is_err(), "{line:?} is not a frame");
        }
    }

    #[test]
    fn test_hello_is_read_and_a_matching_protocol_is_accepted() {
        let hello =
            Hello::of(&json!({"protocol": 1, "app": "mandala", "version": "0.1.0", "pid": 9, "map": null}));
        assert_eq!(hello.pid, Some(9));
        assert_eq!(hello.map, None);
        assert_eq!(hello.accept(false), Ok(None));
    }

    #[test]
    fn test_hello_with_another_protocol_is_refused_unless_forced() {
        let hello = Hello::of(&json!({"protocol": 2, "app": "mandala", "version": "9.9.9"}));
        let refused = hello
            .accept(false)
            .expect_err("a protocol this client does not speak");
        assert!(
            refused.contains("protocol 2") && refused.contains("--force"),
            "{refused}"
        );
        let forced = hello.accept(true).expect("forced").expect("with a note");
        assert!(forced.contains("--force"), "{forced}");
    }

    #[test]
    fn test_hello_from_something_else_is_refused() {
        let other = Hello::of(&json!({"protocol": 1, "app": "nginx"}));
        assert!(other.accept(false).is_err());
        let silent = Hello::of(&json!({}));
        assert!(silent
            .accept(false)
            .expect_err("no protocol")
            .contains("no protocol"));
    }
}
