// SPDX-License-Identifier: MPL-2.0

//! Whole invocations against a server on a real socket: the exit-code
//! contract, the argv → request mapping for every command, the
//! deadlines, and what the client does when the server misbehaves.
//! Nothing is mocked — see `testing`.

use std::io::Write;
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::platform::native;
use crate::testing::{against, hello, invoke, no, ok, split, FakeServer, Outcome};
use crate::verbs::VERBS;
use crate::wire::FRAME_CAP;
use crate::Exit;

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

/// A server that answers `clock.wait` and everything else with
/// `ok:true` and an empty result.
fn lenient() -> FakeServer {
    FakeServer::answering(|request| {
        let result = if request["cmd"] == "clock.wait" {
            json!({"elapsed_ms": 3})
        } else {
            json!({})
        };
        let mut frames = vec![ok(&request["id"], result)];
        // A subscription is only worth a connection that stays up, so
        // the one command that holds it open is given something to
        // print.
        if request["cmd"] == "events.subscribe" {
            frames.push(change("document_changed", 1));
        }
        frames
    })
}

/// A change event the way the server shapes one.
fn change(class: &str, revision: u64) -> Value {
    json!({"event": class, "data": {"revision": revision, "document_revision": revision, "source": "ipc"}})
}

// ---- success, failure, and what is printed -------------------------------

#[test]
fn test_ping_prints_the_result_fields_and_exits_zero() {
    let server = FakeServer::always(json!({"now_ms": 4100.5}));
    let outcome = against(&server, "ping");
    assert_eq!(outcome.exit, Exit::Success);
    assert_eq!(outcome.out, "now_ms: 4100.5\n");
    assert_eq!(outcome.err, "");
    assert_eq!(server.requests(), vec![json!({"id": 1, "cmd": "ping"})]);
}

#[test]
fn test_json_prints_the_reply_exactly_as_the_server_wrote_it() {
    let line = r#"{"result":{"now_ms":1.0},   "ok":true,"id":1,"revision":9}"#;
    let server = FakeServer::start(move |_, peer, seen| {
        peer.send(&hello());
        while let Some(request) = peer.read_request() {
            seen.lock().expect("log").push(request);
            peer.send_raw(&format!("{line}\n"));
        }
    });
    let outcome = against(&server, "--json ping");
    assert_eq!(outcome.exit, Exit::Success);
    assert_eq!(
        outcome.out,
        format!("{line}\n"),
        "the envelope is not re-serialized"
    );
}

#[test]
fn test_a_refusal_exits_one_with_the_code_and_message_on_stderr() {
    let server = FakeServer::answering(|r| vec![no(&r["id"], "not_found", "no node '9'")]);
    let outcome = against(&server, "scene.node_geometry 9");
    assert_eq!(outcome.exit, Exit::CommandError);
    assert_eq!(outcome.out, "");
    assert_eq!(outcome.err, "error: not_found: no node '9'\n");
}

#[test]
fn test_a_refusal_with_json_still_prints_the_envelope_and_the_error_line() {
    let server = FakeServer::answering(|r| vec![no(&r["id"], "no_document", "load one")]);
    let outcome = against(&server, "query.document --json");
    assert_eq!(outcome.exit, Exit::CommandError);
    let envelope: Value = serde_json::from_str(outcome.out.trim()).expect("one JSON line");
    assert_eq!(envelope["error"]["code"], json!("no_document"));
    assert!(
        outcome.err.starts_with("error: no_document: load one"),
        "{}",
        outcome.err
    );
}

#[test]
fn test_error_data_is_appended_compactly() {
    let server = FakeServer::answering(|r| {
        vec![json!({"id": r["id"], "ok": false,
                    "error": {"code": "busy", "message": "pending", "data": {"pending": 4}}})]
    });
    let outcome = against(&server, "input.click_node 1");
    assert_eq!(outcome.err, "error: busy: pending {\"pending\":4}\n");
}

#[test]
fn test_warnings_go_to_stderr_and_do_not_change_the_exit_code() {
    let server = FakeServer::answering(|r| {
        vec![json!({"id": r["id"], "ok": true, "result": {"ran": true},
                    "warnings": [{"code": "macro_gate_rejected", "message": "step refused"}]})]
    });
    let outcome = against(&server, "act.macro macro_id=tidy");
    assert_eq!(outcome.exit, Exit::Success);
    assert_eq!(outcome.out, "ran: true\n");
    assert_eq!(outcome.err, "warning: macro_gate_rejected: step refused\n");
}

#[test]
fn test_console_output_prints_as_bare_lines() {
    let server = FakeServer::always(json!({"output": ["section 0 of 3", "visible"]}));
    let outcome = against(&server, "act.console 'section show'");
    assert_eq!(outcome.out, "section 0 of 3\nvisible\n");
    assert_eq!(server.requests()[0]["line"], json!("section show"));
}

#[test]
fn test_describe_prints_the_command_table() {
    let server = FakeServer::always(json!({
        "protocol": 1, "app": "mandala", "version": "0.1.0",
        "commands": [{"name": "ping", "family": "bootstrap", "summary": "Liveness.", "params": [],
                      "result": [{"name": "now_ms", "type": "number", "doc": "The clock."}]}],
        "events": []
    }));
    let outcome = against(&server, "describe ping");
    assert_eq!(outcome.exit, Exit::Success);
    assert!(outcome.out.contains("ping - Liveness."), "{}", outcome.out);
    assert!(
        outcome.out.contains("  returns now_ms number - The clock."),
        "{}",
        outcome.out
    );
    assert_eq!(
        server.requests()[0],
        json!({"id": 1, "cmd": "describe", "command": "ping"})
    );
}

#[test]
fn test_a_reply_larger_than_a_megabyte_is_read_whole() {
    let big = "y".repeat(3 << 20);
    let expected = big.clone();
    let server = FakeServer::answering(move |r| vec![ok(&r["id"], json!({ "blob": big }))]);
    let outcome = against(&server, "query.document fields=mindmap");
    assert_eq!(outcome.exit, Exit::Success);
    assert!(outcome.out.contains(&expected));
}

#[test]
fn test_a_reply_that_arrives_in_pieces_and_with_crlf_is_assembled() {
    let server = FakeServer::start(|_, peer, seen| {
        peer.send_raw(&format!("{}\r\n", hello()));
        while let Some(request) = peer.read_request() {
            seen.lock().expect("log").push(request);
            for piece in [r#"{"id":1,"ok":tr"#, r#"ue,"result":{"a":1}}"#, "\r\n"] {
                peer.send_raw(piece);
                thread::sleep(Duration::from_millis(30));
            }
        }
    });
    let outcome = against(&server, "ping");
    assert_eq!(outcome.exit, Exit::Success);
    assert_eq!(outcome.out, "a: 1\n");
}

#[test]
fn test_events_the_client_never_asked_for_are_skipped() {
    let server = FakeServer::answering(|r| {
        vec![
            json!({"event": "camera_changed", "data": {}}),
            ok(&r["id"], json!({"now_ms": 1})),
        ]
    });
    let outcome = against(&server, "ping");
    assert_eq!(outcome.exit, Exit::Success);
    assert_eq!(outcome.out, "now_ms: 1\n");
}

#[test]
fn test_a_parse_error_reply_with_a_null_id_answers_the_pending_request() {
    let server = FakeServer::answering(|_| {
        vec![json!({"id": null, "ok": false, "error": {"code": "parse_error", "message": "bad line"}})]
    });
    let outcome = against(&server, "ping");
    assert_eq!(outcome.exit, Exit::CommandError);
    assert!(outcome.err.contains("parse_error"), "{}", outcome.err);
}

// ---- argv -> request, every command --------------------------------------

#[test]
fn test_every_verb_example_reaches_the_wire_as_its_own_command() {
    for verb in VERBS {
        let server = lenient();
        let outcome = against(
            &server,
            verb.example
                .strip_prefix("mandalactl ")
                .expect("examples start with the program name"),
        );
        assert_eq!(outcome.exit, Exit::Success, "`{}`: {}", verb.example, outcome.err);
        let requests = server.requests();
        let first = &requests[0];
        assert_eq!(first["cmd"], json!(verb.name), "`{}`", verb.example);
        let sent = first.as_object().expect("a request is an object");
        for key in sent.keys().filter(|k| *k != "id" && *k != "cmd") {
            assert!(
                verb.params.iter().any(|p| p.name == key),
                "`{}` sent `{key}`, which {} does not declare",
                verb.example,
                verb.name
            );
        }
        assert_eq!(requests.len(), 1, "`{}` sends one request", verb.example);
    }
}

#[test]
fn test_typed_arguments_arrive_with_their_json_types() {
    let server = lenient();
    let outcome = against(&server, "scene.hit_test 412 96.5 space=surface");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_eq!(
        server.requests(),
        vec![json!({"id": 1, "cmd": "scene.hit_test", "x": 412, "y": 96.5, "space": "surface"})]
    );
    let server = lenient();
    let outcome = against(&server, "input.key z mods=ctrl,shift");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_eq!(
        server.requests(),
        vec![json!({"id": 1, "cmd": "input.key", "key": "z", "mods": ["ctrl", "shift"]})]
    );
}

#[test]
fn test_a_unicode_text_argument_survives_the_trip() {
    let server = lenient();
    let outcome = against(&server, "input.text 'hello \u{1F44B} caf\u{e9}'");
    assert_eq!(outcome.exit, Exit::Success);
    assert_eq!(server.requests()[0]["text"], json!("hello \u{1F44B} caf\u{e9}"));
}

#[test]
fn test_a_json_object_action_and_a_double_dash_value_arrive_intact() {
    let server = lenient();
    against(
        &server,
        r#"act.action '{"OpenDocument":"maps/a=b.mindmap.json"}'"#,
    );
    against(&server, "act.console -- fps=on");
    let requests = server.requests();
    assert_eq!(
        requests[0]["action"],
        json!({"OpenDocument": "maps/a=b.mindmap.json"})
    );
    assert_eq!(requests[1]["line"], json!("fps=on"));
}

// ---- commands the table does not know ------------------------------------

fn describing_future() -> FakeServer {
    FakeServer::answering(|request| match request["cmd"].as_str() {
        Some("describe") if request["command"] == "future.cmd" => vec![ok(
            &request["id"],
            json!({"protocol": 1, "app": "mandala", "version": "9", "events": [], "commands": [{
            "name": "future.cmd", "family": "future", "summary": "s", "result": [],
            "params": [
                {"name": "ms", "type": "integer", "required": false, "doc": "d"},
                {"name": "on", "type": "boolean", "required": false, "doc": "d"},
                {"name": "label", "type": "string", "required": false, "doc": "d"}
            ]}]}),
        )],
        Some("future.cmd") => vec![ok(&request["id"], json!({"echoed": true}))],
        _ => vec![no(&request["id"], "invalid_params", "no command to describe")],
    })
}

#[test]
fn test_a_command_the_table_lacks_is_typed_from_the_apps_describe() {
    let server = describing_future();
    let outcome = against(&server, "future.cmd ms=100 on=true label=7");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_eq!(outcome.out, "echoed: true\n");
    let requests = server.requests();
    assert_eq!(
        requests[0],
        json!({"id": 1, "cmd": "describe", "command": "future.cmd"})
    );
    assert_eq!(
        requests[1],
        json!({"id": 2, "cmd": "future.cmd", "ms": 100, "on": true, "label": "7"})
    );
}

#[test]
fn test_a_described_command_refuses_a_parameter_it_does_not_have() {
    let server = describing_future();
    let outcome = against(&server, "future.cmd frames=3");
    assert_eq!(outcome.exit, Exit::UsageError);
    assert!(outcome.err.contains("ms, on, label"), "{}", outcome.err);
    assert_eq!(server.requests().len(), 1, "only the describe went out");
}

#[test]
fn test_a_command_the_app_does_not_know_is_sent_for_the_server_to_judge() {
    let server = FakeServer::answering(|r| {
        if r["cmd"] == "describe" {
            vec![no(
                &r["id"],
                "invalid_params",
                "no command 'nope.cmd' to describe",
            )]
        } else {
            vec![no(
                &r["id"],
                "unknown_cmd",
                "no command 'nope.cmd' - closest is 'ping'",
            )]
        }
    });
    let outcome = against(&server, "nope.cmd n=3 s=hi");
    assert_eq!(outcome.exit, Exit::CommandError);
    assert!(outcome.err.contains("closest is 'ping'"), "{}", outcome.err);
    assert_eq!(
        server.requests()[1],
        json!({"id": 2, "cmd": "nope.cmd", "n": 3, "s": "hi"})
    );
}

// ---- raw -----------------------------------------------------------------

#[test]
fn test_raw_sends_the_frame_and_supplies_an_id_when_it_has_none() {
    let server = lenient();
    let outcome = against(&server, r#"raw '{"cmd":"ping"}'"#);
    assert_eq!(outcome.exit, Exit::Success);
    assert_eq!(server.requests(), vec![json!({"id": 1, "cmd": "ping"})]);
}

#[test]
fn test_raw_keeps_an_id_it_was_given_and_matches_the_reply_to_it() {
    let server = lenient();
    let outcome = against(&server, r#"raw '{"id":"mine","cmd":"ping"}' --json"#);
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_eq!(server.requests(), vec![json!({"id": "mine", "cmd": "ping"})]);
    assert!(outcome.out.contains(r#""id":"mine""#), "{}", outcome.out);
}

#[test]
fn test_raw_reads_the_request_from_stdin() {
    let server = lenient();
    let line = format!("--ipc {} raw -", server.path().display());
    let outcome = invoke(
        &line,
        None,
        "{\n  \"cmd\": \"act.macro\",\n  \"steps\": [{\"kind\": \"Action\", \"action\": \"ZoomFit\"}]\n}\n",
    );
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_eq!(
        server.requests()[0]["steps"],
        json!([{"kind": "Action", "action": "ZoomFit"}])
    );
}

#[test]
fn test_raw_prints_a_known_commands_result_the_way_the_verb_would() {
    let server = FakeServer::always(json!({"output": ["a", "b"]}));
    let outcome = against(&server, r#"raw '{"cmd":"act.console","line":"x"}'"#);
    assert_eq!(outcome.out, "a\nb\n");
}

#[test]
fn test_raw_from_stdin_over_the_server_line_cap_says_so() {
    let server = lenient();
    let frame = format!("{{\"cmd\":\"input.text\",\"text\":\"{}\"}}", "x".repeat(2 << 20));
    let outcome = invoke(&format!("--ipc {} raw -", server.path().display()), None, &frame);
    assert_eq!(outcome.exit, Exit::UsageError);
    assert!(
        outcome.err.contains("over 1048576 bytes (1 MiB)"),
        "{}",
        &outcome.err[..outcome.err.len().min(300)]
    );
    assert_eq!(server.connections(), 0);
}

#[test]
fn test_raw_rejects_a_request_that_is_not_a_frame_without_connecting() {
    let server = lenient();
    for (argument, stdin) in [
        ("raw 'not json'", ""),
        ("raw '[1]'", ""),
        ("raw '{\"id\":1}'", ""),
        ("raw '{\"cmd\":5}'", ""),
        ("raw", ""),
        ("raw one two", ""),
        ("raw -", ""),
        ("raw -", "   \n"),
    ] {
        let line = format!("--ipc {} {argument}", server.path().display());
        let outcome = invoke(&line, None, stdin);
        assert_eq!(outcome.exit, Exit::UsageError, "{argument:?}: {}", outcome.err);
    }
    assert_eq!(server.connections(), 0);
}

// ---- --wait-settled ------------------------------------------------------

#[test]
fn test_wait_settled_sends_the_wait_after_the_command_and_prints_both() {
    let server = lenient();
    let outcome = against(&server, "act.action ZoomIn --wait-settled");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_eq!(outcome.out, "ok\nwait.elapsed_ms: 3\n");
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["cmd"], json!("describe"));
    assert_eq!(requests[0]["command"], json!("clock.wait"));
    assert_eq!(requests[1]["cmd"], json!("act.action"));
    assert_eq!(requests[2]["cmd"], json!("clock.wait"));
    assert_eq!(requests[2]["until"], json!("settled"));
    assert_eq!(requests[2]["id"], json!(3));
    assert_eq!(server.connections(), 1, "one connection carries all three");
}

#[test]
fn test_wait_settled_asks_the_server_to_wait_as_long_as_the_client_will() {
    let server = lenient();
    against(&server, "act.action ZoomIn --wait-settled --timeout 7.5");
    assert_eq!(server.requests()[2]["timeout_ms"], json!(7500));
    let server = lenient();
    against(&server, "act.action ZoomIn --wait-settled --timeout 600");
    assert_eq!(
        server.requests()[2]["timeout_ms"],
        json!(60_000),
        "the server's own bound"
    );
}

#[test]
fn test_wait_settled_with_json_prints_both_envelopes_in_order() {
    let server = lenient();
    let outcome = against(&server, "act.action ZoomIn --wait-settled --json");
    let printed = lines(&outcome.out);
    assert_eq!(printed.len(), 2, "{}", outcome.out);
    assert!(
        printed[0].contains(r#""id":2"#) && printed[1].contains("elapsed_ms"),
        "{}",
        outcome.out
    );
}

#[test]
fn test_wait_settled_does_not_wait_for_a_command_that_failed() {
    let server = FakeServer::answering(|r| {
        if r["cmd"] == "describe" {
            vec![ok(&r["id"], json!({}))]
        } else {
            vec![no(&r["id"], "invalid_params", "bad")]
        }
    });
    let outcome = against(&server, "act.action Nope --wait-settled");
    assert_eq!(outcome.exit, Exit::CommandError);
    assert_eq!(
        server.requests().len(),
        2,
        "the capability question and the command, no wait"
    );
}

#[test]
fn test_a_wait_that_times_out_after_a_command_that_succeeded_is_not_a_failed_command() {
    let server = FakeServer::answering(|r| {
        if r["cmd"] == "clock.wait" {
            vec![json!({"id": r["id"], "ok": false,
                        "error": {"code": "timeout", "message": "never settled", "data": {"elapsed_ms": 10000}}})]
        } else {
            vec![ok(&r["id"], json!({}))]
        }
    });
    let outcome = against(&server, "input.drag 1 2 3 4 --wait-settled");
    // The drag ran and its result was printed; only the wait failed. An
    // exit of 1 ("the app said no") would invite a caller to drag again.
    assert_eq!(outcome.exit, Exit::Unconfirmed);
    assert_eq!(outcome.out, "ok\n", "the command's own result is still printed");
    assert!(
        outcome.err.contains("error: timeout: never settled"),
        "{}",
        outcome.err
    );
    assert!(outcome.err.contains("10000"), "{}", outcome.err);
    assert!(
        outcome.err.contains("The command itself succeeded") && outcome.err.contains("do not repeat it"),
        "{}",
        outcome.err
    );
}

#[test]
fn test_a_wait_the_client_gives_up_on_also_says_the_command_succeeded() {
    let server = FakeServer::start(|_, peer, seen| {
        peer.send(&hello());
        while let Some(request) = peer.read_request() {
            seen.lock().expect("log").push(request.clone());
            if request["cmd"] != "clock.wait" {
                peer.send(&ok(&request["id"], json!({})));
            }
        }
    });
    let outcome = against(&server, "act.action ZoomIn --timeout 0.3 --wait-settled");
    assert_eq!(outcome.exit, Exit::Unconfirmed);
    assert_eq!(outcome.out, "ok\n");
    assert!(outcome.err.contains("do not repeat it"), "{}", outcome.err);
}

/// A `clock.wait` asked for by name, whose own `timeout_ms` outlasts
/// `--timeout`, is waited on for its deadline: the app's structured
/// `timeout` is what gets printed (exit 1, the app said no), not an
/// exchange the client gave up on (exit 4).
#[test]
fn test_a_named_wait_is_waited_on_past_the_client_timeout_until_its_own_deadline() {
    let server = FakeServer::answering(|r| {
        if r["cmd"] == "clock.wait" {
            thread::sleep(Duration::from_millis(300));
            vec![json!({"id": r["id"], "ok": false,
                        "error": {"code": "timeout", "message": "never settled", "data": {"elapsed_ms": 300}}})]
        } else {
            vec![ok(&r["id"], json!({}))]
        }
    });
    let outcome = against(&server, "clock.wait settled timeout_ms=300 --timeout 0.1");
    assert_eq!(outcome.exit, Exit::CommandError, "{}", outcome.err);
    assert!(outcome.err.contains("timeout: never settled"), "{}", outcome.err);
    // Any other command keeps `--timeout` as it is.
    let silent = FakeServer::start(|_, peer, seen| {
        peer.send(&hello());
        while let Some(request) = peer.read_request() {
            seen.lock().expect("log").push(request);
        }
    });
    let started = Instant::now();
    assert_eq!(against(&silent, "ping --timeout 0.1").exit, Exit::Unconfirmed);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn test_an_app_without_clock_wait_is_told_so_before_the_command_runs() {
    let server = FakeServer::answering(|r| {
        if r["cmd"] == "describe" && r["command"] == "clock.wait" {
            vec![no(
                &r["id"],
                "invalid_params",
                "no command 'clock.wait' to describe",
            )]
        } else {
            vec![ok(&r["id"], json!({}))]
        }
    });
    let outcome = against(&server, "act.action DeleteSelection --wait-settled");
    assert_eq!(outcome.exit, Exit::CommandError);
    assert!(
        outcome.err.contains("needs `clock.wait`") && outcome.err.contains("nothing was sent"),
        "{}",
        outcome.err
    );
    assert_eq!(outcome.out, "");
    let sent = server.requests();
    assert_eq!(sent.len(), 1, "only the capability question went out: {sent:?}");
    assert_eq!(sent[0]["cmd"], json!("describe"));
}

#[test]
fn test_wait_settled_on_a_read_only_command_is_refused_before_connecting() {
    let server = lenient();
    let outcome = against(&server, "query.camera --wait-settled");
    assert_eq!(outcome.exit, Exit::UsageError);
    assert!(outcome.err.contains("does not change the app"), "{}", outcome.err);
    assert_eq!(server.connections(), 0);
}

#[test]
fn test_wait_settled_on_a_read_only_command_is_refused_through_raw_too() {
    let server = lenient();
    let outcome = against(&server, r#"raw '{"cmd":"query.camera"}' --wait-settled"#);
    assert_eq!(outcome.exit, Exit::UsageError);
    assert!(outcome.err.contains("`query.camera`"), "{}", outcome.err);
    let outcome = against(&server, r#"raw '{"cmd":"future.cmd"}' --wait-settled"#);
    assert_eq!(
        outcome.exit,
        Exit::Success,
        "a command the table lacks is let through: {}",
        outcome.err
    );
}

// ---- exit 3: the command line --------------------------------------------

#[test]
fn test_usage_errors_exit_three_and_send_nothing() {
    let server = lenient();
    for line in [
        "query.undo abc",
        "scene.hit_test 1 2 3",
        "ping --wait-settle",
        "scene.hit_test regoin=1",
        "capture.screenshot fit=maybe",
        "act.macro steps=[oops",
        "--timeout 0 ping",
        "--ipc",
    ] {
        let outcome = against(&server, line);
        assert_eq!(outcome.exit, Exit::UsageError, "`{line}`: {}", outcome.err);
        assert!(
            outcome.err.contains("--help"),
            "`{line}` does not point at the help: {}",
            outcome.err
        );
    }
    assert_eq!(server.connections(), 0);
}

#[test]
fn test_a_missing_required_parameter_is_the_servers_to_report() {
    let server = FakeServer::answering(|r| {
        vec![json!({"id": r["id"], "ok": false,
                    "error": {"code": "invalid_params", "message": "missing `y`", "data": {"param": "y"}}})]
    });
    let outcome = against(&server, "scene.hit_test 1");
    assert_eq!(outcome.exit, Exit::CommandError);
    assert_eq!(
        outcome.err,
        "error: invalid_params: missing `y` {\"param\":\"y\"}\n"
    );
    assert_eq!(
        server.requests(),
        vec![json!({"id": 1, "cmd": "scene.hit_test", "x": 1})]
    );
}

#[test]
fn test_no_command_and_no_socket_are_usage_errors_with_a_hint() {
    let outcome = invoke("", None, "");
    assert_eq!(outcome.exit, Exit::UsageError);
    assert!(
        outcome.err.contains("no command") && outcome.err.contains("--help"),
        "{}",
        outcome.err
    );
    let outcome = invoke("ping", None, "");
    assert_eq!(outcome.exit, Exit::UsageError);
    assert!(
        outcome.err.contains("MANDALA_IPC") && outcome.err.contains("--ipc"),
        "{}",
        outcome.err
    );
    // The hint is spelled for the platform it runs on: a Windows user
    // told to use `$XDG_RUNTIME_DIR` has been told nothing.
    let launch = native("--ipc \"$XDG_RUNTIME_DIR/mandala.sock\"", "--ipc mandala");
    assert!(outcome.err.contains(launch), "{}", outcome.err);
    let outcome = invoke("ping", Some(""), "");
    assert_eq!(outcome.exit, Exit::UsageError, "an empty variable is no variable");
}

// ---- socket discovery ----------------------------------------------------

#[test]
fn test_the_environment_variable_names_the_socket_when_no_flag_does() {
    let server = lenient();
    let outcome = invoke("ping", Some(&server.path().display().to_string()), "");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
}

#[test]
fn test_the_flag_beats_the_environment_variable() {
    let server = lenient();
    let line = format!("--ipc {} ping", server.path().display());
    let outcome = invoke(&line, Some("/nonexistent/mandala.sock"), "");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
}

// ---- exit 2: the app could not be talked to ------------------------------

fn transport_failure(outcome: &Outcome) -> &str {
    assert_eq!(outcome.exit, Exit::TransportError, "stderr: {}", outcome.err);
    assert_eq!(outcome.out, "");
    &outcome.err
}

/// The request went out whole and no answer was read: exit 4, with the
/// advice not to assume it did not run.
fn unconfirmed_failure(outcome: &Outcome) -> &str {
    assert_eq!(outcome.exit, Exit::Unconfirmed, "stderr: {}", outcome.err);
    assert_eq!(outcome.out, "");
    assert!(
        outcome.err.contains("The request was sent, so it may have run"),
        "{}",
        outcome.err
    );
    &outcome.err
}

// The endpoint's own failures. On Unix the `--ipc` value is a path, and
// a socket file can be missing, stale, not a socket, or too long a path;
// on Windows it is a pipe name, and only missing and too long carry over
// — a pipe has no file to go stale, and a name cannot be a file of some
// other kind (the nearest thing, a value spelled like a path, is refused
// by name). Each Unix-only test says so beside its `cfg`.

// The socket's spelling; the twin below is the pipe's.
#[cfg(unix)]
#[test]
fn test_no_socket_says_the_app_is_not_running_and_how_to_start_it() {
    let dir = tempfile::tempdir().expect("a directory");
    let line = format!("--ipc {} ping", dir.path().join("none.sock").display());
    let outcome = invoke(&line, None, "");
    let err = transport_failure(&outcome);
    assert!(
        err.contains("no socket at") && err.contains("App not running"),
        "{err}"
    );
    assert!(err.contains("--ipc <socket-path>"), "{err}");
}

#[cfg(windows)]
#[test]
fn test_no_pipe_says_the_app_is_not_running_and_how_to_start_it() {
    let line = format!("--ipc mandalactl-test-absent-{} ping", std::process::id());
    let outcome = invoke(&line, None, "");
    let err = transport_failure(&outcome);
    assert!(
        err.contains(r"no pipe \\.\pipe\mandala-mandalactl-test-absent-") && err.contains("App not running"),
        "{err}"
    );
    assert!(err.contains("--ipc <name>"), "{err}");
}

// Unix-only: a pipe vanishes with its server's last handle, so there is
// no stale endpoint to tell apart from a missing one.
#[cfg(unix)]
#[test]
fn test_a_stale_socket_file_is_told_apart_from_a_missing_one() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("stale.sock");
    drop(UnixListener::bind(&path).expect("binds"));
    assert!(path.exists(), "dropping a listener leaves the file behind");
    let outcome = invoke(&format!("--ipc {} ping", path.display()), None, "");
    let err = transport_failure(&outcome);
    assert!(
        err.contains("nothing is listening") && err.contains("stale"),
        "{err}"
    );
}

// Unix-only as written: on Windows the value names a pipe, never a file.
// The twin below is what a path given there gets.
#[cfg(unix)]
#[test]
fn test_a_path_that_is_not_a_socket_is_reported_by_name() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("thesis.txt");
    std::fs::write(&path, "words").expect("writes");
    let outcome = invoke(&format!("--ipc {} ping", path.display()), None, "");
    let err = transport_failure(&outcome);
    assert!(err.contains("thesis.txt"), "{err}");
}

#[cfg(windows)]
#[test]
fn test_a_path_where_a_pipe_name_belongs_is_reported_by_name() {
    let outcome = invoke(r"--ipc C:\work\thesis.txt ping", None, "");
    let err = transport_failure(&outcome);
    assert!(
        err.contains(r"C:\work\thesis.txt") && err.contains("names a pipe, not a path"),
        "{err}"
    );
}

// The socket's limit; the twin below is the pipe name's.
#[cfg(unix)]
#[test]
fn test_a_socket_path_over_the_platform_limit_is_reported_as_such() {
    let long = format!("/tmp/{}/m.sock", "d".repeat(200));
    let outcome = invoke(&format!("--ipc {long} ping"), None, "");
    let err = transport_failure(&outcome);
    assert!(
        err.contains("at most 107") || err.contains("no socket at"),
        "{err}"
    );
}

#[cfg(windows)]
#[test]
fn test_a_pipe_name_over_the_platform_limit_is_reported_as_such() {
    // `mandala-` is 8 units, so 249 more is one past Windows' 256.
    let outcome = invoke(&format!("--ipc {} ping", "x".repeat(249)), None, "");
    let err = transport_failure(&outcome);
    assert!(err.contains("Windows allows 256"), "{err}");
}

fn rejecting(times: usize) -> FakeServer {
    FakeServer::start(move |index, peer, seen| {
        if index < times {
            peer.send(&json!({"event": "connection_rejected", "data": {"reason": "busy"}}));
        } else {
            peer.serve(seen, &|r| vec![ok(&r["id"], json!({"now_ms": 1}))]);
        }
    })
}

#[test]
fn test_a_busy_slot_is_retried_until_the_previous_client_is_gone() {
    let server = rejecting(2);
    let outcome = against(&server, "ping");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_eq!(server.connections(), 3);
}

#[test]
fn test_a_slot_that_stays_busy_is_reported_as_another_client_holding_it() {
    let server = rejecting(usize::MAX);
    let started = Instant::now();
    let outcome = against(&server, "--timeout 0.6 ping");
    let err = transport_failure(&outcome);
    assert!(err.contains("another client holds the controller slot"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(5), "the retry is bounded");
}

#[test]
fn test_a_protocol_the_client_does_not_speak_is_refused_unless_forced() {
    let server = FakeServer::start(|_, peer, seen| {
        peer.send(&json!({"event": "hello", "data": {"protocol": 2, "app": "mandala", "version": "9.0.0"}}));
        while let Some(request) = peer.read_request() {
            seen.lock().expect("log").push(request.clone());
            peer.send(&ok(&request["id"], json!({"now_ms": 1})));
        }
    });
    let outcome = against(&server, "ping");
    let err = transport_failure(&outcome);
    assert!(err.contains("protocol 2") && err.contains("--force"), "{err}");
    assert!(
        server.requests().is_empty(),
        "nothing is sent to a server we do not understand"
    );
    let outcome = against(&server, "--force ping");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert!(
        outcome.err.starts_with("warning:") && outcome.err.contains("--force"),
        "{}",
        outcome.err
    );
}

#[test]
fn test_a_silent_app_times_out_with_a_hint_and_does_not_hang() {
    let server = FakeServer::start(|_, peer, _| {
        peer.send(&hello());
        while peer.read_request().is_some() {}
    });
    let started = Instant::now();
    let outcome = against(&server, "--timeout 0.3 ping");
    let err = unconfirmed_failure(&outcome);
    assert!(err.contains("no reply to `ping` within 0.3 s"), "{err}");
    assert!(err.contains("busy or frozen") && err.contains("10 s"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn test_a_silent_greeting_times_out_too() {
    let server = FakeServer::start(|_, peer, _| while peer.read_request().is_some() {});
    let outcome = against(&server, "--timeout 0.3 ping");
    let err = transport_failure(&outcome);
    assert!(err.contains("no greeting within 0.3 s"), "{err}");
}

#[test]
fn test_an_app_that_closes_mid_request_is_reported_as_gone() {
    let server = FakeServer::start(|_, peer, _| {
        peer.send(&hello());
        let _ = peer.read_request();
    });
    let outcome = against(&server, "ping");
    let err = unconfirmed_failure(&outcome);
    assert!(
        err.contains("closed the connection before replying to `ping`"),
        "{err}"
    );
    assert!(err.contains("freeze watchdog"), "{err}");
}

#[test]
fn test_a_connection_closed_before_the_greeting_is_reported() {
    let server = FakeServer::start(|_, _, _| {});
    let outcome = against(&server, "ping");
    let err = transport_failure(&outcome);
    assert!(err.contains("before the greeting"), "{err}");
}

#[test]
fn test_a_server_that_greets_with_something_else_is_refused() {
    let server = FakeServer::start(|_, peer, _| peer.send(&ok(&json!(1), json!({}))));
    let outcome = against(&server, "ping");
    let err = transport_failure(&outcome);
    assert!(err.contains("expected a `hello` greeting"), "{err}");
}

#[test]
fn test_a_shutdown_notice_is_reported_as_the_app_exiting() {
    let server = FakeServer::answering(|_| vec![json!({"event": "shutdown", "data": {}})]);
    let outcome = against(&server, "ping");
    let err = unconfirmed_failure(&outcome);
    assert!(err.contains("shutting down"), "{err}");
}

#[test]
fn test_a_line_that_is_not_json_leaves_the_request_unconfirmed() {
    let server = FakeServer::start(|_, peer, _| {
        peer.send(&hello());
        let _ = peer.read_request();
        peer.send_raw("this is not json\n");
    });
    let outcome = against(&server, "ping");
    let err = unconfirmed_failure(&outcome);
    assert!(err.contains("not JSON"), "{err}");
}

#[test]
fn test_a_reply_to_some_other_request_leaves_the_request_unconfirmed() {
    let server = FakeServer::answering(|_| vec![ok(&json!(99), json!({}))]);
    let outcome = against(&server, "ping");
    let err = unconfirmed_failure(&outcome);
    assert!(err.contains("request id 99"), "{err}");
}

#[test]
fn test_a_frame_of_exactly_the_servers_cap_is_read_and_one_byte_more_is_not() {
    for (extra, accepted) in [(0usize, true), (1, false)] {
        let server = FakeServer::start(move |_, peer, _| {
            peer.send(&hello());
            let _ = peer.read_request();
            let head = r#"{"id":1,"ok":true,"result":{"pad":""#;
            let tail = r#""}}"#;
            let pad = "p".repeat(FRAME_CAP + extra - head.len() - tail.len());
            peer.send_raw(&format!("{head}{pad}{tail}\n"));
        });
        let outcome = against(&server, "--json ping");
        if accepted {
            assert_eq!(
                outcome.exit,
                Exit::Success,
                "{}",
                &outcome.err[..outcome.err.len().min(200)]
            );
            assert_eq!(outcome.out.trim().len(), FRAME_CAP);
        } else {
            assert_eq!(outcome.exit, Exit::Unconfirmed);
            assert!(outcome.err.contains("over 8388608 bytes"), "{}", outcome.err);
        }
    }
}

#[test]
fn test_a_frame_beyond_the_servers_own_cap_is_refused_not_buffered() {
    let server = FakeServer::start(|_, peer, _| {
        peer.send(&hello());
        let _ = peer.read_request();
        peer.send_raw(&"x".repeat(FRAME_CAP + 64));
    });
    let outcome = against(&server, "ping");
    let err = unconfirmed_failure(&outcome);
    assert!(err.contains("over 8388608 bytes"), "{err}");
}

#[test]
fn test_a_request_over_the_servers_line_cap_is_a_usage_error_and_never_sent() {
    let server = lenient();
    let line = format!(
        "--ipc {} input.text {}",
        server.path().display(),
        "x".repeat(1 << 20)
    );
    let outcome = invoke(&line, None, "");
    assert_eq!(
        outcome.exit,
        Exit::UsageError,
        "{}",
        &outcome.err[..outcome.err.len().min(300)]
    );
    assert!(outcome.err.contains("1 MiB"));
    assert!(server.requests().is_empty());
}

// ---- help, version, and a reader that went away ---------------------------

#[test]
fn test_help_needs_no_socket_and_exits_zero() {
    for line in [
        "--help",
        "-h",
        "help",
        "help act.action",
        "act.action --help",
        "help raw",
        "--version",
    ] {
        let outcome = invoke(line, None, "");
        assert_eq!(outcome.exit, Exit::Success, "`{line}`: {}", outcome.err);
        assert!(!outcome.out.is_empty(), "`{line}`");
        assert_eq!(outcome.err, "", "`{line}`");
    }
    assert!(invoke("--help", None, "").out.contains("EXIT CODES"));
    assert!(invoke("--version", None, "").out.contains("protocol 1"));
}

#[test]
fn test_help_for_a_command_that_is_not_there_is_a_usage_error() {
    let outcome = invoke("help nope.cmd", None, "");
    assert_eq!(outcome.exit, Exit::UsageError);
    assert!(outcome.err.contains("describe nope.cmd"), "{}", outcome.err);
}

#[test]
fn test_a_reader_that_went_away_does_not_change_the_outcome() {
    struct Gone;
    impl Write for Gone {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let server = FakeServer::always(json!({"now_ms": 1}));
    let words = split(&format!("--ipc {} ping", server.path().display()));
    let mut input: &[u8] = b"";
    let exit = crate::run(
        &words,
        crate::run::Env::default(),
        &mut crate::Io {
            out: &mut Gone,
            err: &mut Gone,
            cwd: None,
            stdin: &mut input,
        },
    );
    assert_eq!(exit, Exit::Success);
}

#[test]
fn test_exit_codes_are_the_documented_numbers() {
    assert_eq!(
        [
            Exit::Success,
            Exit::CommandError,
            Exit::TransportError,
            Exit::UsageError,
            Exit::Unconfirmed
        ]
        .map(Exit::code),
        [0, 1, 2, 3, 4]
    );
}

// ---- deadlines inside a line, rejection reasons, paths, raw --------------

#[test]
fn test_a_reply_that_trickles_in_cannot_outlast_the_timeout() {
    // A byte every 100 ms and never a newline: each read returns well
    // inside its own timeout, so only a deadline re-armed per read stops
    // the client at --timeout rather than whenever the line ends.
    let server = FakeServer::start(|_, peer, _| {
        peer.send(&hello());
        let _ = peer.read_request();
        for _ in 0..60 {
            peer.send_raw("x");
            thread::sleep(Duration::from_millis(100));
        }
    });
    let started = Instant::now();
    let outcome = against(&server, "--timeout 0.5 ping");
    let err = unconfirmed_failure(&outcome);
    assert!(err.contains("no reply to `ping` within 0.5 s"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "took {:?}",
        started.elapsed()
    );
}

#[test]
fn test_a_send_the_app_never_drains_is_bounded_and_means_nothing_ran() {
    // The app greets and never reads: the buffer fills part way through
    // a request near the 1 MiB cap, and the write must give up at the
    // deadline. The request line is incomplete, so the server could not
    // have executed it — exit 2, not 4. On the pipe that rests on
    // `PipeWire::write` handing over at most `WRITE_CHUNK` at a time: a
    // write it cannot cancel may still deliver that chunk, never the
    // rest of the body, so the client still knows the line never
    // completed. Handing over the whole remainder made it exit 4.
    let server = FakeServer::start(|_, peer, _| {
        peer.send(&hello());
        thread::sleep(Duration::from_secs(5));
    });
    let frame = format!("{{\"cmd\":\"input.text\",\"text\":\"{}\"}}", "x".repeat(900_000));
    let started = Instant::now();
    let outcome = invoke(
        &format!("--ipc {} --timeout 0.5 raw -", server.path().display()),
        None,
        &frame,
    );
    let err = transport_failure(&outcome);
    assert!(err.contains("room to send `input.text`"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "took {:?}",
        started.elapsed()
    );
}

#[test]
fn test_a_rejection_for_any_reason_but_busy_is_reported_as_is_and_not_retried() {
    let server = FakeServer::start(|_, peer, _| {
        peer.send(&json!({"event": "connection_rejected", "data": {"reason": "unauthorized"}}));
    });
    let started = Instant::now();
    let outcome = against(&server, "ping");
    let err = transport_failure(&outcome);
    assert!(err.contains("reason: unauthorized"), "{err}");
    assert!(
        !err.contains("another client"),
        "it does not claim a second client: {err}"
    );
    assert_eq!(
        server.connections(),
        1,
        "a refusal that will not change is not retried"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn test_a_relative_path_argument_reaches_the_app_absolute() {
    let server = lenient();
    let outcome = against(&server, "capture.screenshot after.png fit=true");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_eq!(
        server.requests()[0]["path"],
        json!(native("/work/maps/after.png", r"C:\work\maps\after.png"))
    );
    let outcome = against(&server, "query.document to_file=./out/model.json");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_eq!(
        server.requests()[1]["to_file"],
        json!(native(
            "/work/maps/out/model.json",
            r"C:\work\maps\out\model.json"
        ))
    );
}

#[test]
fn test_a_relative_path_with_no_working_directory_is_a_usage_error() {
    let server = lenient();
    let words = split(&format!(
        "--ipc {} capture.screenshot rel.png",
        server.path().display()
    ));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut input: &[u8] = b"";
    let exit = crate::run(
        &words,
        crate::run::Env::default(),
        &mut crate::Io {
            out: &mut out,
            err: &mut err,
            cwd: None,
            stdin: &mut input,
        },
    );
    assert_eq!(exit, Exit::UsageError);
    assert!(String::from_utf8_lossy(&err).contains("working directory"));
    assert_eq!(server.connections(), 0);
}

#[test]
fn test_raw_accepts_a_double_dash_before_its_request() {
    let server = lenient();
    let outcome = against(&server, r#"raw -- '{"cmd":"ping"}'"#);
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_eq!(server.requests(), vec![json!({"id": 1, "cmd": "ping"})]);
    let line = format!("--ipc {} raw -- -", server.path().display());
    let outcome = invoke(&line, None, r#"{"cmd":"ping"}"#);
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
}

#[test]
fn test_a_timeout_too_small_to_be_a_deadline_is_a_usage_error_not_an_outage() {
    let server = lenient();
    for tiny in ["1e-12", "0.0005"] {
        let outcome = against(&server, &format!("--timeout {tiny} ping"));
        assert_eq!(outcome.exit, Exit::UsageError, "{tiny}: {}", outcome.err);
    }
    assert_eq!(server.connections(), 0);
}

#[test]
fn test_consecutive_invocations_each_get_the_controller_slot() {
    // The server serves one connection at a time, as the app does:
    // a second connection is refused until the first has closed.
    let held = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server = FakeServer::start(move |_, peer, seen| {
        if held.swap(true, std::sync::atomic::Ordering::SeqCst) {
            peer.send(&json!({"event": "connection_rejected", "data": {"reason": "busy"}}));
            return;
        }
        peer.serve(seen, &|r| vec![ok(&r["id"], json!({"now_ms": 1}))]);
        thread::sleep(Duration::from_millis(60));
        held.store(false, std::sync::atomic::Ordering::SeqCst);
    });
    for _ in 0..3 {
        let outcome = against(&server, "ping");
        assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    }
}

// ---- events.subscribe holds the connection and prints what is pushed ------

/// A server that acknowledges the subscription, then pushes `events`.
fn pushing(events: Vec<Value>) -> FakeServer {
    FakeServer::answering(move |request| {
        let mut frames = vec![ok(
            &request["id"],
            json!({"subscribed": ["document_changed"], "verbose": false, "revision": 0, "document_revision": 0}),
        )];
        frames.extend(events.clone());
        frames
    })
}

#[test]
fn test_subscribe_prints_the_reply_then_each_event_and_stops_at_count() {
    let server = pushing(vec![
        change("document_changed", 1),
        change("document_changed", 2),
        change("document_changed", 3),
    ]);
    let outcome = against(&server, "events.subscribe document_changed --count 2");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    assert_eq!(
        lines(&outcome.out),
        [
            "document_revision: 0",
            "revision: 0",
            "subscribed[0]: \"document_changed\"",
            "verbose: false",
            "document_changed {\"document_revision\":1,\"revision\":1,\"source\":\"ipc\"}",
            "document_changed {\"document_revision\":2,\"revision\":2,\"source\":\"ipc\"}",
        ],
        "two events, not the third the server also sent"
    );
    assert_eq!(
        server.requests(),
        vec![json!({"id": 1, "cmd": "events.subscribe", "classes": ["document_changed"]})]
    );
}

#[test]
fn test_subscribe_with_json_prints_every_frame_as_received() {
    let line = r#"{"event":"document_changed", "data":{"revision":7}}"#;
    let server = FakeServer::start(move |_, peer, seen| {
        peer.send(&hello());
        while let Some(request) = peer.read_request() {
            seen.lock().expect("log").push(request.clone());
            peer.send(&ok(&request["id"], json!({"subscribed": ["document_changed"]})));
            peer.send_raw(&format!("{line}\n"));
        }
    });
    let outcome = against(&server, "events.subscribe document_changed --json --count 1");
    assert_eq!(outcome.exit, Exit::Success, "{}", outcome.err);
    let printed = lines(&outcome.out);
    assert_eq!(printed.len(), 2, "the reply and one event: {printed:?}");
    assert_eq!(printed[1], line, "the event is not re-serialized");
}

#[test]
fn test_subscribe_ends_quietly_when_the_app_says_it_is_shutting_down() {
    let server = pushing(vec![
        change("document_changed", 1),
        json!({"event": "shutdown", "data": {}}),
    ]);
    let outcome = against(&server, "events.subscribe document_changed");
    assert_eq!(
        outcome.exit,
        Exit::Success,
        "an app that exits ends the watch: {}",
        outcome.err
    );
    assert_eq!(
        outcome
            .out
            .lines()
            .filter(|l| l.starts_with("document_changed "))
            .count(),
        1,
        "{}",
        outcome.out
    );
    assert!(outcome.err.contains("shutting down"), "{}", outcome.err);
}

#[test]
fn test_subscribe_is_a_transport_fault_when_the_connection_dies_unannounced() {
    let server = FakeServer::start(|_, peer, seen| {
        peer.send(&hello());
        if let Some(request) = peer.read_request() {
            seen.lock().expect("log").push(request.clone());
            peer.send(&ok(&request["id"], json!({"subscribed": ["document_changed"]})));
        }
        // Dropping the peer closes the socket with no `shutdown`.
    });
    let outcome = against(&server, "events.subscribe document_changed");
    assert_eq!(outcome.exit, Exit::TransportError, "{}", outcome.err);
    assert!(outcome.err.contains("aborted"), "{}", outcome.err);
}

#[test]
fn test_a_refused_subscription_exits_one_and_does_not_wait() {
    let server = FakeServer::answering(|r| vec![no(&r["id"], "invalid_params", "no event class 'nope'")]);
    let outcome = against(&server, "events.subscribe nope");
    assert_eq!(outcome.exit, Exit::CommandError);
    assert!(outcome.err.contains("no event class"), "{}", outcome.err);
}

#[test]
fn test_count_belongs_to_the_watch_and_nothing_is_sent_without_it() {
    let server = lenient();
    let outcome = against(&server, "ping --count 2");
    assert_eq!(outcome.exit, Exit::UsageError);
    assert!(outcome.err.contains("events.subscribe"), "{}", outcome.err);
    assert!(server.requests().is_empty(), "a usage error sends nothing");
}

#[test]
fn test_the_other_events_commands_are_plain_exchanges() {
    let server = FakeServer::always(json!({"revision": 4, "document_revision": 2}));
    let outcome = against(&server, "events.revision");
    assert_eq!(outcome.exit, Exit::Success);
    assert_eq!(outcome.out, "document_revision: 2\nrevision: 4\n");
    let outcome = against(&server, "events.unsubscribe");
    assert_eq!(outcome.exit, Exit::Success);
    assert_eq!(
        server.requests()[1],
        json!({"id": 1, "cmd": "events.unsubscribe"})
    );
}
