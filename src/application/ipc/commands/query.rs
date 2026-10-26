// SPDX-License-Identifier: MPL-2.0

//! The `query` family: read-only views of what the application knows
//! (`format/ipc.md` §`query`): the document, the selection, the camera,
//! the interaction mode, the status line, the running animations, the
//! modal editors and the undo stack. What a person reads off the screen
//! chrome and the app's mental state, as JSON.
//!
//! **Read-only by construction.** A handler reaches the app through
//! [`Call::observe`] (`&`), never [`Call::drive`], and the two reads the
//! target offers are `&MindMapDocument` and a `&self` projection of the
//! shell state ([`CommandTarget::document`], [`CommandTarget::shell_state`]).
//! So a query cannot flip `dirty`, push an undo entry, rebuild a scene
//! or ask for a frame. There is no mutation route to defend because
//! none is reachable.
//!
//! **No mirror structs.** The wire form of each piece of state is a
//! projection that lives beside the state (`document/ipc_state.rs`,
//! `app/ipc_state.rs`), each an exhaustive `match`. This module only
//! validates the request, asks for the projection and frames the
//! reply. Where it reads anything it checks the request first, so a
//! refused query (`invalid_params`) has touched nothing.

use serde_json::Value;

use crate::application::document::{DocumentField, MindMapDocument, PointerMiss};
use crate::application::ipc::envelope::Reply;
use crate::application::ipc::registry::{
    Call, CommandSpec, Family, ParamSpec, ParamType, ResultSpec, Support,
};
use crate::application::ipc::{CommandTarget, ShellState};

use super::refusal::{no_loaded_document, refuse, Refusal};
use super::sidecar::{parse_to_file, write_sidecar, ToFile};

/// `query.undo`'s default `limit`: enough to see the recent edits
/// without paying for a long session's whole stack.
const UNDO_DEFAULT_LIMIT: usize = 20;
/// `query.undo`'s ceiling on `limit`; a larger request is refused
/// rather than clamped, so the agent learns the cap.
const UNDO_MAX_LIMIT: usize = 1000;

pub(super) const FAMILY: Family = Family {
    name: "query",
    commands: &[
        CommandSpec {
            name: "query.document",
            summary: "The open document: path, dirty flag, undo depth, map metadata, counts, and \
                      on request the whole model.",
            params: &[
                ParamSpec::new(
                    "fields",
                    ParamType::Array,
                    false,
                    "Which parts to return, as strings: file_path, dirty, undo_depth, map, counts, \
                     mindmap. Default: all but mindmap, the one part whose size is unbounded.",
                ),
                ParamSpec::new(
                    "pointer",
                    ParamType::String,
                    false,
                    "An RFC 6901 JSON Pointer into the serialized model (\"/nodes/1.2\", \"/edges/0\"). \
                     The reply's mindmap is then the value found there. Implies fields=[\"mindmap\"] \
                     when fields is omitted; refused when fields lacks mindmap.",
                ),
                // Native: the browser has no filesystem (`format/ipc.md`
                // §Transport "Browser").
                #[cfg(not(target_arch = "wasm32"))]
                ParamSpec::new(
                    "to_file",
                    ParamType::String,
                    false,
                    "Absolute path. Write the reply's fields there as JSON, streamed, and reply with \
                     just {path, bytes}. For a model too large to inline (an inline reply over 8 MiB \
                     is reply_too_large). The write is atomic: a temp file in the same directory, \
                     renamed into place.",
                ),
                // Native: the browser has no filesystem (`format/ipc.md`
                // §Transport "Browser").
                #[cfg(not(target_arch = "wasm32"))]
                ParamSpec::new(
                    "overwrite",
                    ParamType::Boolean,
                    false,
                    "Allow to_file to replace an existing file. Default false: an existing file is \
                     invalid_params. A symlink or directory at the path is always refused. Only valid \
                     with to_file.",
                ),
            ],
            result: &[
                ResultSpec::new(
                    "file_path",
                    ParamType::String,
                    "The path the document is bound to, or null for one never saved.",
                ),
                ResultSpec::new(
                    "dirty",
                    ParamType::Boolean,
                    "Whether the document has unsaved changes.",
                ),
                ResultSpec::new(
                    "undo_depth",
                    ParamType::Integer,
                    "How many entries the undo stack holds.",
                ),
                ResultSpec::new(
                    "map",
                    ParamType::Object,
                    "{name, version} of the loaded map.",
                ),
                ResultSpec::new(
                    "counts",
                    ParamType::Object,
                    "{nodes, edges, sections, palettes, custom_mutations, macros}.",
                ),
                ResultSpec::new(
                    "mindmap",
                    ParamType::Object,
                    "The model through its own serde, in the .mindmap.json shape; with pointer, the \
                     value found there.",
                ),
                ResultSpec::new(
                    "path",
                    ParamType::String,
                    "With to_file only: the absolute path written.",
                ),
                ResultSpec::new(
                    "bytes",
                    ParamType::Integer,
                    "With to_file only: the size of the file written.",
                ),
            ],
            support: Support::Everywhere,
            run: document,
        },
        CommandSpec {
            name: "query.selection",
            summary: "The current selection: the SelectionState variant and its ids.",
            params: &[],
            result: &[ResultSpec::new(
                "kind",
                ParamType::String,
                "The variant: None, Single, Multi, Section, MultiSection, SectionRange, Edge, \
                 EdgeLabel, PortalLabel or PortalText. The variant's own ids sit beside it.",
            )],
            support: Support::Everywhere,
            run: selection,
        },
        CommandSpec {
            name: "query.camera",
            summary: "The camera: canvas-space center, zoom and surface size, the surface/canvas transform.",
            params: &[],
            result: &[
                ResultSpec::new(
                    "center",
                    ParamType::Object,
                    "{x, y}: the canvas point at the middle of the viewport.",
                ),
                ResultSpec::new(
                    "zoom",
                    ParamType::Number,
                    "Zoom factor; above 1 is zoomed in.",
                ),
                ResultSpec::new("zoom_min", ParamType::Number, "The lowest zoom the camera clamps to."),
                ResultSpec::new("zoom_max", ParamType::Number, "The highest zoom the camera clamps to."),
                ResultSpec::new(
                    "surface",
                    ParamType::Object,
                    "{width, height} of the surface, in physical pixels.",
                ),
            ],
            support: Support::Everywhere,
            run: camera,
        },
        CommandSpec {
            name: "query.mode",
            summary: "The interaction mode, the drag state machine, the open modal, the hovered node and the pointer.",
            params: &[],
            result: &[
                ResultSpec::new(
                    "interaction_mode",
                    ParamType::Object,
                    "{kind: Default|Reparent|Connect|NodeEdit|Resize, …}: the mode and its ids.",
                ),
                ResultSpec::new(
                    "drag_state",
                    ParamType::Object,
                    "{kind: None|Pending|PendingRight|Panning|SelectingRect|Throttled, …}; a Throttled \
                     drag names its gesture.",
                ),
                ResultSpec::new(
                    "modals",
                    ParamType::Array,
                    "Every open modal, in the order the keyboard handler consults them: console, \
                     color_picker, single_line_edit, text_edit. The picker claims only its own keys.",
                ),
                ResultSpec::new(
                    "key_owner",
                    ParamType::String,
                    "Who receives an ordinary key: console, single_line_edit, text_edit, or none \
                     (normal dispatch). Never color_picker, which claims only its own keys.",
                ),
                ResultSpec::new(
                    "hovered_node",
                    ParamType::String,
                    "The node id under the pointer, or null.",
                ),
                ResultSpec::new(
                    "cursor_pos",
                    ParamType::Object,
                    "{x, y}: the last known pointer position, in surface pixels.",
                ),
            ],
            support: Support::Everywhere,
            run: mode,
        },
        CommandSpec {
            name: "query.status",
            summary: "The mode-status line the renderer shows, such as \"editing: <id> — section [1 of 3]\".",
            params: &[],
            result: &[ResultSpec::new(
                "text",
                ParamType::String,
                "The status line, or null when none is showing (Default mode).",
            )],
            support: Support::Everywhere,
            run: status,
        },
        CommandSpec {
            name: "query.animations",
            summary: "The running animations and their clocks.",
            params: &[],
            result: &[
                ResultSpec::new(
                    "active",
                    ParamType::Boolean,
                    "Whether any animation instance is running (it is ticking unless paused).",
                ),
                ResultSpec::new(
                    "paused",
                    ParamType::Boolean,
                    "Whether a tree-mutating drag has suspended the tick; the animations are then \
                     frozen and each reports phase paused.",
                ),
                ResultSpec::new("count", ParamType::Integer, "How many animations are running."),
                ResultSpec::new(
                    "animations",
                    ParamType::Array,
                    "One entry per instance: target_id, section_idx, mutation_id, start_ms, \
                     elapsed_ms, phase (delay|running|due|paused), progress (0 to 1, before easing), \
                     timing.",
                ),
            ],
            support: Support::Everywhere,
            run: animations,
        },
        CommandSpec {
            name: "query.editors",
            summary: "The modal editors' live state and the document's active previews.",
            params: &[],
            result: &[
                ResultSpec::new(
                    "console",
                    ParamType::Object,
                    "The open console: input, cursor, history, scrollback tail, completions; null when closed.",
                ),
                ResultSpec::new(
                    "color_picker",
                    ParamType::Object,
                    "The open color picker: mode, hue_deg, sat, val, gesture; null when closed.",
                ),
                ResultSpec::new(
                    "single_line_edit",
                    ParamType::Object,
                    "The edge-label / portal-caption editor: target, buffer, cursor_grapheme_pos, \
                     original; null when closed.",
                ),
                ResultSpec::new(
                    "text_edit",
                    ParamType::Object,
                    "The node text editor: node_id, section_idx, buffer, cursor_grapheme_pos, \
                     selection_anchor; null when closed.",
                ),
                ResultSpec::new(
                    "previews",
                    ParamType::Object,
                    "The scene-only substitutions the document is showing: label_edit, portal_text_edit, \
                     color_picker, border, rect_select; each null when none is live.",
                ),
            ],
            support: Support::Everywhere,
            run: editors,
        },
        CommandSpec {
            name: "query.undo",
            summary: "The undo stack: its depth and its newest entries, newest first.",
            params: &[ParamSpec::new(
                "limit",
                ParamType::Integer,
                false,
                "How many entries to return, newest first. Default 20, at most 1000.",
            )],
            result: &[
                ResultSpec::new("depth", ParamType::Integer, "How many entries the stack holds."),
                ResultSpec::new(
                    "entries",
                    ParamType::Array,
                    "Newest first: {kind, node_ids?, index?}, the order Undo takes them.",
                ),
            ],
            support: Support::Everywhere,
            run: undo,
        },
    ],
};

/// Run `read` against the loaded document (and the app it lives in, for
/// the state that is not the document's), or answer `no_document`.
fn with_document(
    call: &mut Call<'_>,
    read: impl FnOnce(&MindMapDocument, &dyn CommandTarget) -> Value,
) -> Reply {
    let Some(app) = call.observe() else {
        return call.no_document();
    };
    let Some(doc) = app.document() else {
        return no_loaded_document(call);
    };
    let result = read(doc, app);
    call.ok(result)
}

/// Reply with one part of the shell state, or `no_document` while the
/// app has not started.
fn with_shell(call: &mut Call<'_>, which: ShellState) -> Reply {
    let Some(app) = call.observe() else {
        return call.no_document();
    };
    let result = app.shell_state(which);
    call.ok(result)
}

/// `query.document`'s request, parsed.
#[derive(Debug, PartialEq)]
struct DocumentRequest {
    fields: Vec<DocumentField>,
    pointer: Option<String>,
    to_file: Option<ToFile>,
}

/// Parse and check `query.document`'s parameters, and the rules that
/// cross them. Pure over the raw values, so the whole rule set is
/// testable without an app.
fn parse_document_request(
    fields: Option<&Value>,
    pointer: Option<&Value>,
    to_file: Option<&Value>,
    overwrite: Option<&Value>,
) -> Result<DocumentRequest, Refusal> {
    let explicit = match fields {
        None => None,
        Some(value) => {
            let words = value.as_array().map(Vec::as_slice).unwrap_or_default();
            if words.is_empty() {
                return Err(Refusal::Invalid(
                    "fields",
                    "`fields` must name at least one of file_path, dirty, undo_depth, map, counts, mindmap"
                        .into(),
                ));
            }
            let mut parsed = Vec::new();
            for word in words {
                let field = word.as_str().and_then(DocumentField::parse).ok_or_else(|| {
                    Refusal::Invalid(
                        "fields",
                        format!(
                            "`{word}` is not a document field; the fields are {}",
                            DocumentField::ALL.map(DocumentField::name).join(", ")
                        ),
                    )
                })?;
                if !parsed.contains(&field) {
                    parsed.push(field);
                }
            }
            // Reply order is the document's, not the request's.
            parsed.sort_by_key(|f| DocumentField::ALL.iter().position(|a| a == f));
            Some(parsed)
        }
    };
    let pointer = match pointer {
        None => None,
        Some(value) => {
            let pointer = value.as_str().unwrap_or_default();
            if !pointer.is_empty() && !pointer.starts_with('/') {
                return Err(Refusal::Invalid(
                    "pointer",
                    format!(
                        "`{pointer}` is not a JSON Pointer: a pointer must be empty or begin with `/` \
                         (e.g. `/{pointer}`)"
                    ),
                ));
            }
            Some(pointer.to_string())
        }
    };
    let fields = match (explicit, &pointer) {
        (Some(fields), Some(_)) if !fields.contains(&DocumentField::Mindmap) => {
            return Err(Refusal::Invalid(
                "pointer",
                "`pointer` narrows `mindmap`, which `fields` does not select".into(),
            ))
        }
        (Some(fields), _) => fields,
        (None, Some(_)) => vec![DocumentField::Mindmap],
        (None, None) => DocumentField::ALL
            .into_iter()
            .filter(|f| f.is_default())
            .collect(),
    };
    let to_file = parse_to_file(to_file, overwrite)?;
    Ok(DocumentRequest {
        fields,
        pointer,
        to_file,
    })
}

/// `query.document`'s result for `request`, read from `doc`: the
/// inline object, or `{path, bytes}` when it went to a file. The
/// pointer is resolved before any byte is written anywhere.
fn document_result(doc: &MindMapDocument, request: &DocumentRequest) -> Result<Value, Refusal> {
    let reply = doc
        .ipc_document_reply(&request.fields, request.pointer.as_deref())
        .map_err(|PointerMiss| {
            Refusal::NotFound(format!(
                "the pointer `{}` resolves to nothing in the model",
                request.pointer.as_deref().unwrap_or_default()
            ))
        })?;
    match &request.to_file {
        None => serde_json::to_value(&reply)
            .map_err(|e| Refusal::Internal(format!("could not serialize the document: {e}"))),
        Some(target) => write_sidecar(target, &reply),
    }
}

fn document(call: &mut Call<'_>) -> Reply {
    let request = match parse_document_request(
        call.param("fields"),
        call.param("pointer"),
        call.param("to_file"),
        call.param("overwrite"),
    ) {
        Ok(request) => request,
        Err(refusal) => return refuse(call, refusal),
    };
    let Some(app) = call.observe() else {
        return call.no_document();
    };
    let Some(doc) = app.document() else {
        return no_loaded_document(call);
    };
    match document_result(doc, &request) {
        Ok(result) => call.ok(result),
        Err(refusal) => refuse(call, refusal),
    }
}

fn selection(call: &mut Call<'_>) -> Reply {
    with_document(call, |doc, _| doc.selection.to_ipc_json())
}

fn camera(call: &mut Call<'_>) -> Reply {
    with_shell(call, ShellState::Camera)
}

fn mode(call: &mut Call<'_>) -> Reply {
    with_shell(call, ShellState::Mode)
}

fn status(call: &mut Call<'_>) -> Reply {
    with_shell(call, ShellState::Status)
}

fn animations(call: &mut Call<'_>) -> Reply {
    // The app clock: the one the per-frame drain ticks animations
    // against, so `elapsed_ms` and `phase` agree with what the next
    // tick will do — virtual milliseconds under a virtual clock, which
    // makes them a pure function of the step sequence. A
    // drag-suspended tick is reported as a pause rather than as a
    // clock that keeps running.
    with_document(call, |doc, app| {
        doc.ipc_animations_json(app.clock().now_whole_ms(), app.animation_pause())
    })
}

fn editors(call: &mut Call<'_>) -> Reply {
    with_shell(call, ShellState::Editors)
}

/// `query.undo`'s `limit`, checked.
fn parse_undo_limit(limit: Option<&Value>) -> Result<usize, Refusal> {
    let Some(limit) = limit else {
        return Ok(UNDO_DEFAULT_LIMIT);
    };
    match limit.as_u64().map(usize::try_from) {
        Some(Ok(n)) if n <= UNDO_MAX_LIMIT => Ok(n),
        _ => Err(Refusal::Invalid(
            "limit",
            format!("`limit` must be an integer from 0 to {UNDO_MAX_LIMIT}, got {limit}"),
        )),
    }
}

fn undo(call: &mut Call<'_>) -> Reply {
    let limit = match parse_undo_limit(call.param("limit")) {
        Ok(limit) => limit,
        Err(refusal) => return refuse(call, refusal),
    };
    with_document(call, |doc, _| doc.ipc_undo_json(limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::LineOutcome;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::UndoAction;
    use crate::application::ipc::events::Observed;
    use crate::application::ipc::registry::tests::{run_attached, run_detached};
    use crate::application::ipc::space::Point;
    use crate::application::keybinds::Action;
    use crate::application::macros::MacroStep;
    use serde_json::json;

    use super::super::sidecar::tests::{names_in, scratch_dir};

    /// A real loaded document behind the `CommandTarget` seam, for
    /// driving a query through the registry. The act methods are the
    /// funnels a query must never reach, so each one fails the test
    /// that calls it (the `MacroDispatchTarget` test double in
    /// `macro_core.rs` is the precedent: a real `IpcCommandTarget`
    /// needs a GPU renderer, which only the GPU-backed tier of
    /// `TEST_CONVENTIONS` §T8 builds, one app at a time).
    /// What it reports for the shell state is a fixed, recognizable
    /// value, since the real projections are tested where they live.
    struct LoadedApp {
        doc: Option<MindMapDocument>,
        pause: Option<u64>,
        clock: crate::application::clock::AppClock,
    }

    impl LoadedApp {
        fn with_fixture() -> Self {
            LoadedApp {
                doc: Some(load_test_doc()),
                pause: None,
                clock: crate::application::clock::AppClock::default(),
            }
        }
    }

    impl CommandTarget for LoadedApp {
        fn dispatch_action(&mut self, _: Action, _: Option<Point>) -> bool {
            panic!("a query must not dispatch an Action");
        }
        fn run_console_line(&mut self, _: &str) -> Option<LineOutcome> {
            panic!("a query must not run a console line");
        }
        fn has_macro(&self, _: &str) -> bool {
            panic!("a query must not look up a macro");
        }
        fn run_macro(&mut self, _: &str, _: &mut dyn FnMut(Observed<'_>)) -> bool {
            panic!("a query must not run a macro");
        }
        fn run_macro_steps(&mut self, _: &[MacroStep], _: &mut dyn FnMut(Observed<'_>)) -> bool {
            panic!("a query must not run macro steps");
        }
        fn changed_view(&self) -> bool {
            false
        }
        fn document(&self) -> Option<&MindMapDocument> {
            self.doc.as_ref()
        }
        fn animation_pause(&self) -> Option<u64> {
            self.pause
        }
        fn shell_state(&self, which: ShellState) -> Value {
            json!({ "shell_state": format!("{which:?}") })
        }
        fn scene(&self) -> Option<crate::application::ipc::SceneView<'_>> {
            panic!("a query must not read the scene");
        }
        fn hit_test(&mut self, _: Point) -> Option<Value> {
            panic!("a query must not hit-test the scene");
        }
        fn surface_point(&self, _: Point) -> (f64, f64) {
            panic!("a query must not place a pointer");
        }
        fn inject(
            &mut self,
            _: Vec<crate::application::ipc::injected::Sample>,
            _: crate::application::ipc::injected::Pacing,
        ) -> Result<crate::application::ipc::Injected, crate::application::ipc::injected::Overloaded>
        {
            panic!("a query must not inject input");
        }
        fn injected_pending(&self) -> usize {
            panic!("a query must not read the injected-input queue");
        }
        fn capture(
            &mut self,
            _: &crate::application::capture::plan::CaptureRequest,
            _: crate::application::capture::Destination,
            _: bool,
        ) -> Option<Result<crate::application::ipc::Screenshot, crate::application::capture::CaptureError>>
        {
            panic!("a query must not take a screenshot");
        }
        fn record_start(
            &mut self,
            _: &crate::application::capture::plan::CaptureRequest,
            _: u32,
            _: u32,
            _: crate::application::capture::record::RecordDestination,
        ) -> Option<
            Result<crate::application::capture::record::Started, crate::application::capture::CaptureError>,
        > {
            panic!("a query must not start a recording");
        }
        fn record_stop(
            &mut self,
        ) -> Result<crate::application::capture::record::Summary, crate::application::capture::CaptureError>
        {
            panic!("a query must not stop a recording");
        }
        fn clock(&self) -> &crate::application::clock::AppClock {
            &self.clock
        }
        fn set_clock_mode(
            &mut self,
            _: crate::application::clock::ClockMode,
        ) -> Result<bool, crate::application::clock::ClockRefusal> {
            panic!("a query must not switch the clock");
        }
        fn step_clock(
            &mut self,
            _: crate::application::clock::StepRequest,
        ) -> Result<crate::application::clock::Stepped, crate::application::clock::ClockRefusal> {
            panic!("a query must not step the clock");
        }
    }

    const ALL_QUERIES: [&str; 8] = [
        r#"{"id":1,"cmd":"query.document"}"#,
        r#"{"id":1,"cmd":"query.selection"}"#,
        r#"{"id":1,"cmd":"query.camera"}"#,
        r#"{"id":1,"cmd":"query.mode"}"#,
        r#"{"id":1,"cmd":"query.status"}"#,
        r#"{"id":1,"cmd":"query.animations"}"#,
        r#"{"id":1,"cmd":"query.editors"}"#,
        r#"{"id":1,"cmd":"query.undo"}"#,
    ];

    /// Every query answers `no_document` with no app attached, and
    /// none of them asks for a frame.
    #[test]
    fn test_every_query_answers_no_document_before_the_app_exists() {
        for frame in ALL_QUERIES {
            let (reply, repaint) = run_detached(frame);
            assert_eq!(reply["error"]["code"], json!("no_document"), "{frame}");
            assert!(!repaint, "{frame}: a query never asks for a frame");
        }
    }

    /// With the app up but its initial load failed, the four queries
    /// that read the document say so, in words that tell the agent
    /// how to load one; the four shell queries need no document.
    #[test]
    fn test_the_document_queries_say_when_no_document_loaded() {
        let mut app = LoadedApp {
            doc: None,
            pause: None,
            clock: crate::application::clock::AppClock::default(),
        };
        for frame in [ALL_QUERIES[0], ALL_QUERIES[1], ALL_QUERIES[5], ALL_QUERIES[7]] {
            let (reply, repaint) = run_attached(frame, &mut app);
            assert_eq!(reply["error"]["code"], json!("no_document"), "{frame}");
            assert!(
                reply["error"]["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("open <path>")),
                "{frame}: {reply}"
            );
            assert!(!repaint);
        }
        for frame in [ALL_QUERIES[2], ALL_QUERIES[3], ALL_QUERIES[4], ALL_QUERIES[6]] {
            let (reply, _) = run_attached(frame, &mut app);
            assert_eq!(reply["ok"], json!(true), "{frame}: {reply}");
        }
    }

    /// **Zero mutation, through the request path** (issue #64's
    /// acceptance criterion). Every query is dispatched through the
    /// decoder and the registry with a real loaded document behind
    /// the target. None reaches an act funnel (each would panic), none
    /// raises the repaint flag, and the document is exactly as it
    /// was: `dirty`, the undo depth, the selection and the whole
    /// model.
    #[test]
    fn test_every_query_through_the_registry_leaves_the_document_untouched() {
        let mut app = LoadedApp::with_fixture();
        {
            let doc = app.doc.as_mut().expect("loaded");
            doc.dirty = true;
            doc.undo_stack.push(UndoAction::CreateEdge { index: 0 });
        }
        let model_before =
            serde_json::to_value(&app.doc.as_ref().expect("loaded").mindmap).expect("serializes");

        let mut frames: Vec<&str> = ALL_QUERIES.to_vec();
        frames.push(r#"{"id":2,"cmd":"query.document","fields":["mindmap","counts","dirty"]}"#);
        frames.push(r#"{"id":2,"cmd":"query.document","pointer":"/nodes/0/position"}"#);
        frames.push(r#"{"id":2,"cmd":"query.undo","limit":3}"#);
        for frame in frames {
            let (reply, repaint) = run_attached(frame, &mut app);
            assert_eq!(reply["ok"], json!(true), "{frame}: {reply}");
            assert!(!repaint, "{frame}: a query must not ask for a frame");
        }

        let doc = app.doc.as_ref().expect("loaded");
        assert!(doc.dirty, "dirty is untouched");
        assert_eq!(doc.undo_stack.len(), 1, "no undo entry is pushed or popped");
        assert_eq!(
            serde_json::to_value(&doc.mindmap).expect("serializes"),
            model_before,
            "the model is untouched"
        );
    }

    /// The queries answer with what the document holds, through the
    /// registry: the default summary, the filter, a pointer, the undo
    /// stack, the selection and the animations.
    #[test]
    fn test_the_document_queries_report_the_loaded_document() {
        let mut app = LoadedApp::with_fixture();
        {
            let doc = app.doc.as_mut().expect("loaded");
            doc.dirty = true;
            doc.undo_stack.push(UndoAction::CreateEdge { index: 7 });
            doc.selection = crate::application::document::SelectionState::Single("0".into());
        }
        let node_count = app.doc.as_ref().expect("loaded").mindmap.nodes.len();

        let (reply, _) = run_attached(r#"{"id":1,"cmd":"query.document"}"#, &mut app);
        assert_eq!(reply["result"]["dirty"], json!(true));
        assert_eq!(reply["result"]["undo_depth"], json!(1));
        assert_eq!(reply["result"]["counts"]["nodes"], json!(node_count));
        assert!(reply["result"].get("mindmap").is_none(), "the model is opt-in");

        let (reply, _) = run_attached(r#"{"id":1,"cmd":"query.document","fields":["dirty"]}"#, &mut app);
        assert_eq!(reply["result"], json!({ "dirty": true }));

        let (reply, _) = run_attached(
            r#"{"id":1,"cmd":"query.document","pointer":"/nodes/0/id"}"#,
            &mut app,
        );
        assert_eq!(reply["result"], json!({ "mindmap": "0" }));

        let (reply, _) = run_attached(
            r#"{"id":1,"cmd":"query.document","pointer":"/nodes/no-such-node"}"#,
            &mut app,
        );
        assert_eq!(reply["error"]["code"], json!("not_found"));

        let (reply, _) = run_attached(r#"{"id":1,"cmd":"query.undo"}"#, &mut app);
        assert_eq!(
            reply["result"],
            json!({ "depth": 1, "entries": [{ "kind": "CreateEdge", "index": 7 }] })
        );
        let (reply, _) = run_attached(r#"{"id":1,"cmd":"query.selection"}"#, &mut app);
        assert_eq!(reply["result"], json!({ "kind": "Single", "node_id": "0" }));
        let (reply, _) = run_attached(r#"{"id":1,"cmd":"query.camera"}"#, &mut app);
        assert_eq!(reply["result"], json!({ "shell_state": "Camera" }));
    }

    /// **`query.document` round-trips through the registry** (issue
    /// #64's acceptance criterion): asked for `mindmap`, the reply
    /// parses back to the model the document holds.
    #[test]
    fn test_the_model_round_trips_through_the_registry() {
        let mut app = LoadedApp::with_fixture();
        let (reply, _) = run_attached(
            r#"{"id":1,"cmd":"query.document","fields":["mindmap"]}"#,
            &mut app,
        );
        let reparsed: baumhard::mindmap::model::MindMap =
            serde_json::from_value(reply["result"]["mindmap"].clone()).expect("the reply is a MindMap");
        assert_eq!(
            serde_json::to_value(&reparsed).expect("serializes"),
            serde_json::to_value(&app.doc.as_ref().expect("loaded").mindmap).expect("serializes"),
            "what the agent reads is what the app holds"
        );
    }

    /// `query.animations` through the registry reports the pause a
    /// tree-mutating drag puts on the tick, which only the app knows.
    #[test]
    fn test_query_animations_carries_the_apps_pause_state() {
        let mut app = LoadedApp::with_fixture();
        let (reply, _) = run_attached(r#"{"id":1,"cmd":"query.animations"}"#, &mut app);
        assert_eq!(reply["result"]["paused"], json!(false));
        assert_eq!(reply["result"]["active"], json!(false));
        app.pause = Some(0);
        let (reply, _) = run_attached(r#"{"id":1,"cmd":"query.animations"}"#, &mut app);
        assert_eq!(reply["result"]["paused"], json!(true));
    }

    fn refused(frame: &str) -> (String, String) {
        let (reply, repaint) = run_detached(frame);
        assert!(!repaint, "{frame}");
        assert_eq!(reply["ok"], json!(false), "{frame}");
        (
            reply["error"]["code"].as_str().expect("code").to_owned(),
            reply["error"]["data"]["param"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        )
    }

    /// A query that takes no parameters rejects a stray one, naming it.
    #[test]
    fn test_a_parameterless_query_refuses_a_stray_parameter() {
        for cmd in ["selection", "camera", "mode", "status", "animations", "editors"] {
            assert_eq!(
                refused(&format!(r#"{{"id":1,"cmd":"query.{cmd}","limit":1}}"#)),
                ("invalid_params".into(), "limit".into()),
                "query.{cmd}"
            );
        }
    }

    #[test]
    fn test_document_refusals_name_the_offending_parameter() {
        for (frame, param) in [
            (r#"{"id":1,"cmd":"query.document","fields":[]}"#, "fields"),
            (r#"{"id":1,"cmd":"query.document","fields":["nope"]}"#, "fields"),
            (r#"{"id":1,"cmd":"query.document","fields":[7]}"#, "fields"),
            (r#"{"id":1,"cmd":"query.document","fields":"dirty"}"#, "fields"),
            (r#"{"id":1,"cmd":"query.document","pointer":"nodes"}"#, "pointer"),
            (
                r#"{"id":1,"cmd":"query.document","fields":["dirty"],"pointer":"/nodes"}"#,
                "pointer",
            ),
            (
                r#"{"id":1,"cmd":"query.document","to_file":"relative.json"}"#,
                "to_file",
            ),
            (r#"{"id":1,"cmd":"query.document","overwrite":true}"#, "overwrite"),
            (r#"{"id":1,"cmd":"query.undo","limit":-1}"#, "limit"),
            (r#"{"id":1,"cmd":"query.undo","limit":1001}"#, "limit"),
            (r#"{"id":1,"cmd":"query.undo","limit":"3"}"#, "limit"),
        ] {
            assert_eq!(refused(frame), ("invalid_params".into(), param.into()), "{frame}");
        }
    }

    /// The pointer error states the rule the right way round: a
    /// literal reader must not strip a leading `/` that was correct.
    #[test]
    fn test_the_pointer_error_says_a_pointer_must_begin_with_a_slash() {
        let (reply, _) = run_detached(r#"{"id":1,"cmd":"query.document","pointer":"nodes"}"#);
        let message = reply["error"]["message"].as_str().expect("message");
        assert!(message.contains("must be empty or begin with `/`"), "{message}");
        assert!(
            message.contains("/nodes"),
            "it shows the corrected form: {message}"
        );
    }

    fn request(
        fields: Option<Value>,
        pointer: Option<&str>,
        to_file: Option<&str>,
        overwrite: Option<bool>,
    ) -> Result<DocumentRequest, Refusal> {
        let (pointer, to_file, overwrite) = (
            pointer.map(Value::from),
            to_file.map(Value::from),
            overwrite.map(Value::from),
        );
        parse_document_request(
            fields.as_ref(),
            pointer.as_ref(),
            to_file.as_ref(),
            overwrite.as_ref(),
        )
    }

    #[test]
    fn test_the_default_request_is_everything_but_the_model() {
        let parsed = request(None, None, None, None).expect("parses");
        assert_eq!(parsed.fields.len(), DocumentField::ALL.len() - 1);
        assert!(!parsed.fields.contains(&DocumentField::Mindmap));
        assert_eq!(parsed.pointer, None);
        assert_eq!(parsed.to_file, None);
    }

    #[test]
    fn test_a_pointer_alone_implies_the_model_and_fields_reply_in_document_order() {
        let parsed = request(None, Some("/nodes"), None, None).expect("parses");
        assert_eq!(parsed.fields, vec![DocumentField::Mindmap]);
        let parsed = request(Some(json!(["counts", "dirty", "counts"])), None, None, None).expect("parses");
        assert_eq!(
            parsed.fields,
            vec![DocumentField::Dirty, DocumentField::Counts],
            "deduplicated, in the document's order"
        );
    }

    #[test]
    fn test_to_file_streams_the_fields_and_reports_path_and_size() {
        let doc = load_test_doc();
        let dir = scratch_dir("write");
        let path = dir.join("model.json");
        let request = request(
            Some(json!(["mindmap", "counts"])),
            None,
            Some(path.to_str().expect("utf-8")),
            None,
        )
        .expect("parses");
        let reply = document_result(&doc, &request).expect("writes");
        let on_disk = std::fs::read(&path).expect("the file exists");
        assert_eq!(reply["path"], json!(path.to_str().expect("utf-8")));
        assert_eq!(
            reply["bytes"],
            json!(on_disk.len()),
            "the size reported is the size written"
        );
        let written: Value = serde_json::from_slice(&on_disk).expect("JSON");
        let inline = document_result(
            &doc,
            &DocumentRequest {
                to_file: None,
                ..request
            },
        )
        .expect("inline");
        assert_eq!(
            written, inline,
            "the stream and the inline reply are the same document"
        );
        assert_eq!(names_in(&dir), vec!["model.json"], "no temp file is left behind");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn test_the_undo_limit_defaults_and_is_bounded() {
        assert_eq!(parse_undo_limit(None), Ok(UNDO_DEFAULT_LIMIT));
        assert_eq!(parse_undo_limit(Some(&json!(0))), Ok(0));
        assert_eq!(parse_undo_limit(Some(&json!(1000))), Ok(1000));
        assert!(parse_undo_limit(Some(&json!(1001))).is_err());
        assert!(parse_undo_limit(Some(&json!(-1))).is_err());
        assert!(parse_undo_limit(Some(&json!(1.5))).is_err());
    }
}
