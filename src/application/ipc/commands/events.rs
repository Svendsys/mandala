// SPDX-License-Identifier: MPL-2.0

//! The `events` family: opting in to the event stream and reading the
//! revision counters (`format/ipc.md` §`events`).
//!
//! Nothing here emits an event. Observation and delivery are
//! `ipc::events` and the transport; these commands only edit the
//! connection's [`Subscriptions`](crate::application::ipc::events::Subscriptions)
//! and report the counters. A connection starts with none, so an agent
//! that polls never receives a frame it did not ask for.

use serde_json::{json, Value};

use crate::application::ipc::envelope::Reply;
use crate::application::ipc::events::{is_transport_event, Class, Classes, Driver};
use crate::application::ipc::registry::{Call, CommandSpec, Family, ParamSpec, ParamType, ResultSpec};

const SUBSCRIBED: ResultSpec = ResultSpec::new(
    "subscribed",
    ParamType::Array,
    "Every class now subscribed, by name, in name order.",
);
const REVISION: ResultSpec = ResultSpec::new(
    "revision",
    ParamType::Integer,
    "The revision counter: advances once per observation in which any event class had something to report.",
);
const DOCUMENT_REVISION: ResultSpec = ResultSpec::new(
    "document_revision",
    ParamType::Integer,
    "The document's revision: committed model changes, monotonic across open and new.",
);

pub(super) const FAMILY: Family = Family {
    name: "events",
    commands: &[
        CommandSpec {
            name: "events.subscribe",
            summary: "Opt in to event classes. Changes made before this command are not reported.",
            params: &[
                ParamSpec::new(
                    "classes",
                    ParamType::Array,
                    true,
                    "Class names to add (`describe` lists them under `events`). Already-subscribed classes stay.",
                ),
                ParamSpec::new(
                    "verbose",
                    ParamType::Boolean,
                    false,
                    "Report every observation separately, a macro's steps included, instead of one coalesced \
                     event per class per drain. Default false. Applies to the whole connection.",
                ),
            ],
            result: &[
                SUBSCRIBED,
                ResultSpec::new("verbose", ParamType::Boolean, "Whether the verbose level is on."),
                REVISION,
                DOCUMENT_REVISION,
            ],
            run: subscribe,
        },
        CommandSpec {
            name: "events.unsubscribe",
            summary: "Opt out of event classes, and drop any of them still waiting to be delivered.",
            params: &[ParamSpec::new(
                "classes",
                ParamType::Array,
                false,
                "Class names to drop. Omit to drop them all.",
            )],
            result: &[SUBSCRIBED],
            run: unsubscribe,
        },
        CommandSpec {
            name: "events.revision",
            summary: "Read the revision counters without subscribing to anything.",
            params: &[],
            result: &[REVISION, DOCUMENT_REVISION],
            run: revision,
        },
    ],
};

/// The classes `names` names, or the message for the first thing
/// wrong with the list. The three transport-level classes are named
/// as such, because they are the ones an agent will try.
fn parse_classes(names: &Value) -> Result<Classes, String> {
    let names = names.as_array().map(Vec::as_slice).unwrap_or_default();
    let mut classes = Classes::default();
    for name in names {
        let Some(word) = name.as_str() else {
            return Err(format!("`classes` holds strings, got {name}"));
        };
        match Class::parse(word) {
            Some(class) => classes.insert(class),
            None if is_transport_event(word) => {
                return Err(format!(
                    "`{word}` is always delivered and cannot be subscribed to or dropped"
                ))
            }
            None => {
                let known: Vec<&str> = Class::ALL.iter().map(|class| class.name()).collect();
                return Err(format!(
                    "no event class '{word}'; the classes are {}",
                    known.join(", ")
                ));
            }
        }
    }
    Ok(classes)
}

fn subscribed(call: &mut Call<'_>) -> Value {
    let classes = call.session().subscriptions().classes;
    json!(classes.iter().map(Class::name).collect::<Vec<_>>())
}

fn revision(call: &mut Call<'_>) -> Reply {
    match call.observe_events(Driver::App) {
        Some(revisions) => call.ok(json!({
            "revision": revisions.revision,
            "document_revision": revisions.document_revision,
        })),
        None => call.no_document(),
    }
}

fn subscribe(call: &mut Call<'_>) -> Reply {
    let classes = match call.param("classes").map(parse_classes) {
        Some(Ok(classes)) if !classes.is_empty() => classes,
        Some(Ok(_)) => return call.invalid_params("classes", "name at least one class to subscribe to"),
        Some(Err(message)) => return call.invalid_params("classes", message),
        None => return call.invalid_params("classes", "`events.subscribe` requires `classes`"),
    };
    let verbose = call.param("verbose").and_then(Value::as_bool).unwrap_or(false);
    // What changed before this command belongs to the subscriptions
    // that existed then: it is queued for them first, and only then
    // does the new subscription take effect. So a class added now
    // reports from here on, and never the history.
    let Some(revisions) = call.observe_events(Driver::App) else {
        return call.no_document();
    };
    call.session().subscribe(classes, verbose);
    let subscribed = subscribed(call);
    call.ok(json!({
        "subscribed": subscribed,
        "verbose": verbose,
        "revision": revisions.revision,
        "document_revision": revisions.document_revision,
    }))
}

fn unsubscribe(call: &mut Call<'_>) -> Reply {
    let classes = match call.param("classes").map(parse_classes) {
        Some(Ok(classes)) => Some(classes),
        Some(Err(message)) => return call.invalid_params("classes", message),
        None => None,
    };
    call.session().unsubscribe(classes);
    let subscribed = subscribed(call);
    call.ok(json!({ "subscribed": subscribed }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ipc::events::fixture::EventsApp;
    use crate::application::ipc::events::Session;
    use crate::application::ipc::registry::tests::{run_detached, run_in};
    use serde_json::json;

    fn run(frame: &str, app: &mut EventsApp, session: &mut Session) -> (Value, bool) {
        run_in(frame, Some(app), session)
    }

    /// The events a session holds, as `{event, data}` wire values, in
    /// delivery order.
    fn queued(session: &mut Session) -> Vec<Value> {
        let mut out = Vec::new();
        while let Some(queued) = session.pop_pending() {
            out.push(serde_json::to_value(&queued.event).expect("serializes"));
        }
        out
    }

    fn subscribe(app: &mut EventsApp, session: &mut Session, classes: &str) -> Value {
        run(
            &format!(r#"{{"id":1,"cmd":"events.subscribe","classes":{classes}}}"#),
            app,
            session,
        )
        .0
    }

    #[test]
    fn test_subscribe_names_what_is_wrong_with_the_class_list() {
        let mut app = EventsApp::new();
        let mut session = Session::default();
        for (classes, needle) in [
            ("[]", "at least one"),
            (r#"["nope"]"#, "no event class 'nope'"),
            (r#"["hello"]"#, "always delivered"),
            (r#"["shutdown"]"#, "always delivered"),
            (r#"["connection_rejected"]"#, "always delivered"),
            ("[7]", "holds strings"),
        ] {
            let reply = subscribe(&mut app, &mut session, classes);
            assert_eq!(reply["error"]["code"], json!("invalid_params"), "{classes}");
            assert_eq!(reply["error"]["data"]["param"], json!("classes"), "{classes}");
            let message = reply["error"]["message"].as_str().expect("message");
            assert!(message.contains(needle), "{classes}: {message}");
        }
        let reply = subscribe(&mut app, &mut session, r#"["nope"]"#);
        assert!(
            reply["error"]["message"]
                .as_str()
                .expect("message")
                .contains("document_changed"),
            "the refusal lists the classes that do exist"
        );
        assert!(
            !session.is_subscribed(),
            "a refused request subscribes to nothing"
        );
        let (reply, _) = run(r#"{"id":1,"cmd":"events.subscribe"}"#, &mut app, &mut session);
        assert_eq!(reply["error"]["data"]["param"], json!("classes"));
    }

    #[test]
    fn test_the_commands_that_look_at_the_app_wait_for_it() {
        for frame in [
            r#"{"id":1,"cmd":"events.subscribe","classes":["document_changed"]}"#,
            r#"{"id":1,"cmd":"events.revision"}"#,
        ] {
            let (reply, _) = run_detached(frame);
            assert_eq!(reply["error"]["code"], json!("no_document"), "{frame}");
        }
    }

    #[test]
    fn test_subscribe_reports_what_is_subscribed_and_the_counters() {
        let mut app = EventsApp::new();
        let mut session = Session::default();
        let reply = subscribe(
            &mut app,
            &mut session,
            r#"["selection_changed","document_changed"]"#,
        );
        assert_eq!(
            reply["result"],
            json!({
                "subscribed": ["document_changed", "selection_changed"],
                "verbose": false,
                "revision": 0,
                "document_revision": 0,
            })
        );
        let reply = subscribe(&mut app, &mut session, r#"["camera_changed"]"#);
        assert_eq!(
            reply["result"]["subscribed"],
            json!(["camera_changed", "document_changed", "selection_changed"]),
            "subscribing adds"
        );
    }

    #[test]
    fn test_a_class_subscribed_now_reports_from_here_on_not_the_history() {
        let mut app = EventsApp::new();
        let mut session = Session::default();
        // The first command's baseline.
        run(r#"{"id":1,"cmd":"ping"}"#, &mut app, &mut session);
        app.doc.mark_dirty();
        app.camera.zoom = 2.0;
        subscribe(&mut app, &mut session, r#"["document_changed"]"#);
        assert!(
            queued(&mut session).is_empty(),
            "the change before the command is history"
        );
        app.doc.mark_dirty();
        run(r#"{"id":2,"cmd":"ping"}"#, &mut app, &mut session);
        let events = queued(&mut session);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], json!("document_changed"));
    }

    #[test]
    fn test_a_change_before_subscribing_still_reaches_a_class_already_subscribed() {
        let mut app = EventsApp::new();
        let mut session = Session::default();
        subscribe(&mut app, &mut session, r#"["document_changed"]"#);
        app.doc.mark_dirty();
        app.camera.zoom = 2.0;
        subscribe(&mut app, &mut session, r#"["camera_changed"]"#);
        let events = queued(&mut session);
        let names: Vec<&Value> = events.iter().map(|e| &e["event"]).collect();
        assert_eq!(
            names,
            [&json!("document_changed")],
            "document_changed was already wanted; camera_changed was not"
        );
    }

    #[test]
    fn test_an_agents_command_is_reported_as_ipc_behind_the_counters_its_reply_carries() {
        let mut app = EventsApp::new();
        let mut session = Session::default();
        subscribe(&mut app, &mut session, r#"["document_changed"]"#);
        let (reply, repaint) = run(
            r#"{"id":5,"cmd":"act.console","line":"anything"}"#,
            &mut app,
            &mut session,
        );
        assert!(repaint);
        assert_eq!(
            reply["revision"],
            json!(1),
            "the reply includes the command's own effect"
        );
        assert_eq!(reply["document_revision"], json!(1));
        let events = queued(&mut session);
        assert_eq!(
            events,
            [json!({
                "event": "document_changed",
                "data": { "revision": 1, "document_revision": 1, "source": "ipc" }
            })]
        );
    }

    #[test]
    fn test_a_read_costs_no_revision_and_every_reply_still_carries_one() {
        let mut app = EventsApp::new();
        let mut session = Session::default();
        subscribe(
            &mut app,
            &mut session,
            r#"["document_changed","selection_changed","camera_changed"]"#,
        );
        for frame in [
            r#"{"id":1,"cmd":"ping"}"#,
            r#"{"id":2,"cmd":"describe"}"#,
            r#"{"id":3,"cmd":"events.revision"}"#,
        ] {
            let (reply, repaint) = run(frame, &mut app, &mut session);
            assert!(!repaint, "{frame}");
            assert_eq!(reply["revision"], json!(0), "{frame}");
            assert_eq!(reply["document_revision"], json!(0), "{frame}");
        }
        assert!(
            queued(&mut session).is_empty(),
            "nothing changed, so nothing is announced"
        );
    }

    #[test]
    fn test_events_revision_reads_both_counters_without_subscribing() {
        let mut app = EventsApp::new();
        let mut session = Session::default();
        run(r#"{"id":1,"cmd":"ping"}"#, &mut app, &mut session);
        app.doc.mark_dirty();
        app.doc.mark_dirty();
        let (reply, _) = run(r#"{"id":2,"cmd":"events.revision"}"#, &mut app, &mut session);
        assert_eq!(reply["result"], json!({ "revision": 1, "document_revision": 2 }));
        assert!(!session.is_subscribed());
    }

    #[test]
    fn test_a_macro_is_one_event_unless_the_subscription_is_verbose() {
        let steps = r#"[{"kind":"ConsoleLine","line":"a"},{"kind":"ConsoleLine","line":"b"},{"kind":"ConsoleLine","line":"c"}]"#;
        let run_macro = |verbose: bool| {
            let mut app = EventsApp::new();
            let mut session = Session::default();
            run(
                &format!(
                    r#"{{"id":1,"cmd":"events.subscribe","classes":["document_changed"],"verbose":{verbose}}}"#
                ),
                &mut app,
                &mut session,
            );
            let (reply, _) = run(
                &format!(r#"{{"id":2,"cmd":"act.macro","steps":{steps}}}"#),
                &mut app,
                &mut session,
            );
            (reply, queued(&mut session))
        };

        let (reply, events) = run_macro(false);
        assert_eq!(reply["document_revision"], json!(3));
        assert_eq!(events.len(), 1, "200 steps or 3, one drain is one event");
        assert_eq!(
            events[0]["data"]["document_revision"],
            json!(3),
            "carrying the final revision"
        );

        let (_, events) = run_macro(true);
        let revisions: Vec<&Value> = events.iter().map(|e| &e["data"]["document_revision"]).collect();
        assert_eq!(revisions, [&json!(1), &json!(2), &json!(3)], "an event per step");
    }

    #[test]
    fn test_unsubscribe_drops_classes_and_what_was_waiting_for_them() {
        let mut app = EventsApp::new();
        let mut session = Session::default();
        subscribe(&mut app, &mut session, r#"["document_changed","camera_changed"]"#);
        app.doc.mark_dirty();
        app.camera.zoom = 2.0;
        run(r#"{"id":2,"cmd":"ping"}"#, &mut app, &mut session);
        let (reply, _) = run(
            r#"{"id":3,"cmd":"events.unsubscribe","classes":["camera_changed"]}"#,
            &mut app,
            &mut session,
        );
        assert_eq!(reply["result"], json!({ "subscribed": ["document_changed"] }));
        let events = queued(&mut session);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], json!("document_changed"));

        let (reply, _) = run(r#"{"id":4,"cmd":"events.unsubscribe"}"#, &mut app, &mut session);
        assert_eq!(reply["result"], json!({ "subscribed": [] }));

        let (reply, _) = run(
            r#"{"id":5,"cmd":"events.unsubscribe","classes":["hello"]}"#,
            &mut app,
            &mut session,
        );
        assert_eq!(reply["error"]["code"], json!("invalid_params"));
    }

    #[test]
    fn test_describe_publishes_every_class_and_the_events_commands() {
        let (reply, _) = run_detached(r#"{"id":1,"cmd":"describe"}"#);
        let events: Vec<&str> = reply["result"]["events"]
            .as_array()
            .expect("events")
            .iter()
            .map(|e| e["name"].as_str().expect("name"))
            .collect();
        assert_eq!(
            events,
            [
                "animation_completed",
                "animation_started",
                "camera_changed",
                "connection_rejected",
                "document_changed",
                "document_replaced",
                "hello",
                "mode_changed",
                "selection_changed",
                "shutdown",
            ]
        );
        let commands: Vec<&str> = reply["result"]["commands"]
            .as_array()
            .expect("commands")
            .iter()
            .filter_map(|c| c["name"].as_str())
            .filter(|name| name.starts_with("events."))
            .collect();
        assert_eq!(
            commands,
            ["events.revision", "events.subscribe", "events.unsubscribe"]
        );
    }
}
