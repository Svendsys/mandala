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
//! shell state ([`CommandTarget::document`](crate::application::ipc::CommandTarget::document),
//! [`CommandTarget::shell_state`](crate::application::ipc::CommandTarget::shell_state)).
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

use std::io::Write;
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::application::document::{DocumentField, MindMapDocument};
use crate::application::ipc::envelope::{code, Reply};
use crate::application::ipc::registry::{Call, CommandSpec, Family, ParamSpec, ParamType, ResultSpec};
use crate::application::ipc::ShellState;

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
                ParamSpec::new(
                    "to_file",
                    ParamType::String,
                    false,
                    "Absolute path. Write the reply's fields there as JSON and reply with just \
                     {path, bytes}. For a model too large to inline (an inline reply over 8 MiB is \
                     reply_too_large).",
                ),
                ParamSpec::new(
                    "overwrite",
                    ParamType::Boolean,
                    false,
                    "Allow to_file to replace an existing file. Default false: an existing file is \
                     invalid_params. A symlink at the path is always refused. Only valid with to_file.",
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
                    "modal",
                    ParamType::String,
                    "Which modal takes the keyboard first: console, color_picker, single_line_edit, \
                     text_edit or none.",
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
                    "Whether the per-frame drain is ticking animations.",
                ),
                ResultSpec::new("count", ParamType::Integer, "How many animations are running."),
                ResultSpec::new(
                    "animations",
                    ParamType::Array,
                    "One entry per instance: target_id, section_idx, mutation_id, start_ms, \
                     elapsed_ms, phase (delay|running|due), progress (0 to 1, before easing), timing.",
                ),
            ],
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
            run: undo,
        },
    ],
};

/// Why a query was refused, before the reply that carries it is built.
/// Pure helpers return this so they can be tested against a real
/// document without the registry's request plumbing.
#[derive(Debug, PartialEq)]
enum Refusal {
    /// A parameter is wrong: `invalid_params`, naming it.
    Invalid(&'static str, String),
    /// What a pointer named is not there: `not_found`.
    NotFound(String),
    /// The query could not be honored: `internal`.
    Internal(String),
}

fn refuse(call: &Call<'_>, refusal: Refusal) -> Reply {
    match refusal {
        Refusal::Invalid(param, message) => call.invalid_params(param, message),
        Refusal::NotFound(message) => call.error(code::NOT_FOUND, message, None),
        Refusal::Internal(message) => call.error(code::INTERNAL, message, None),
    }
}

/// `no_document` for the app that has started but holds no document
/// (its initial load failed). The registry's own `no_document` says
/// the app is still starting, which would mislead here.
fn no_loaded_document(call: &Call<'_>) -> Reply {
    call.error(
        code::NO_DOCUMENT,
        "no document is loaded: send `act.console` with `open <path>` to load one",
        None,
    )
}

/// Run `read` against the loaded document, or answer `no_document`.
fn with_document(call: &mut Call<'_>, read: impl FnOnce(&MindMapDocument) -> Value) -> Reply {
    let Some(app) = call.observe() else {
        return call.no_document();
    };
    let Some(doc) = app.document() else {
        return no_loaded_document(call);
    };
    let result = read(doc);
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

#[derive(Debug, PartialEq)]
struct ToFile {
    path: String,
    overwrite: bool,
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
                        "`{pointer}` is not a JSON Pointer: it is empty or starts with `/` (\"/nodes/1.2\")"
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
    let overwrite_flag = overwrite.and_then(Value::as_bool);
    let to_file = match (to_file, overwrite_flag) {
        (None, Some(_)) => {
            return Err(Refusal::Invalid(
                "overwrite",
                "`overwrite` only applies with `to_file`".into(),
            ))
        }
        (None, None) => None,
        (Some(path), overwrite) => {
            let path = path.as_str().unwrap_or_default();
            if !Path::new(path).is_absolute() {
                return Err(Refusal::Invalid(
                    "to_file",
                    format!("`{path}` is not an absolute path"),
                ));
            }
            Some(ToFile {
                path: path.to_string(),
                overwrite: overwrite.unwrap_or(false),
            })
        }
    };
    Ok(DocumentRequest {
        fields,
        pointer,
        to_file,
    })
}

/// The fields of `request`, read from `doc`, as one JSON object.
fn document_fields(doc: &MindMapDocument, request: &DocumentRequest) -> Result<Map<String, Value>, Refusal> {
    let mut reply = Map::new();
    for &field in &request.fields {
        let value = doc.ipc_field_json(field);
        let value = match (field, request.pointer.as_deref()) {
            (DocumentField::Mindmap, Some(pointer)) => value.pointer(pointer).cloned().ok_or_else(|| {
                Refusal::NotFound(format!(
                    "the pointer `{pointer}` resolves to nothing in the model"
                ))
            })?,
            _ => value,
        };
        reply.insert(field.name().to_string(), value);
    }
    Ok(reply)
}

/// A `Write` that counts what passes through it.
struct CountingWriter<W> {
    inner: W,
    bytes: u64,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Write `value` to `target` as JSON and return what was written:
/// `{"path", "bytes"}`, the file sidecar of `format/ipc.md` §Large
/// payloads.
///
/// An LLM composing a path clobbers by accident far more readily than
/// a human, so the write is guarded the way `capture.screenshot`'s is:
/// an existing file is refused unless `overwrite` is set, and a
/// symlink at the path is never followed. Without `overwrite` the
/// file is created exclusively (`create_new`, which refuses an
/// existing path, a symlink included, atomically); with it, the path
/// is checked not to be a symlink before it is truncated.
fn write_sidecar(target: &ToFile, value: &Value) -> Result<Value, Refusal> {
    let path = Path::new(&target.path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if target.overwrite {
        let is_symlink = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
        if is_symlink {
            return Err(Refusal::Invalid(
                "to_file",
                format!("`{}` is a symlink, which is never followed", target.path),
            ));
        }
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let file = options.open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => Refusal::Invalid(
            "to_file",
            format!("`{}` exists; pass `overwrite: true` to replace it", target.path),
        ),
        std::io::ErrorKind::NotFound => Refusal::Invalid(
            "to_file",
            format!("the parent directory of `{}` does not exist", target.path),
        ),
        _ => Refusal::Internal(format!("could not open `{}`: {e}", target.path)),
    })?;
    let mut writer = CountingWriter {
        inner: std::io::BufWriter::new(file),
        bytes: 0,
    };
    serde_json::to_writer(&mut writer, value)
        .map_err(|e| e.to_string())
        .and_then(|()| writer.flush().map_err(|e| e.to_string()))
        .map_err(|e| Refusal::Internal(format!("could not write `{}`: {e}", target.path)))?;
    Ok(json!({ "path": target.path, "bytes": writer.bytes }))
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
    let outcome = document_fields(doc, &request).and_then(|fields| match &request.to_file {
        None => Ok(Value::Object(fields)),
        Some(target) => write_sidecar(target, &Value::Object(fields)),
    });
    match outcome {
        Ok(result) => call.ok(result),
        Err(refusal) => refuse(call, refusal),
    }
}

fn selection(call: &mut Call<'_>) -> Reply {
    with_document(call, |doc| doc.selection.to_ipc_json())
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
    // The app clock, virtual under `clock.set_mode virtual`: the one
    // the per-frame drain ticks animations against, so `elapsed_ms`
    // and `phase` agree with what the next tick will do.
    let now_ms = crate::application::common::now_ms() as u64;
    with_document(call, |doc| doc.ipc_animations_json(now_ms))
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
    with_document(call, |doc| doc.ipc_undo_json(limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::UndoAction;
    use crate::application::ipc::registry::tests::run_detached;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Every query answers `no_document` with no app attached, and
    /// none of them asks for a frame.
    #[test]
    fn test_every_query_answers_no_document_before_the_app_exists() {
        for frame in [
            r#"{"id":1,"cmd":"query.document"}"#,
            r#"{"id":1,"cmd":"query.selection"}"#,
            r#"{"id":1,"cmd":"query.camera"}"#,
            r#"{"id":1,"cmd":"query.mode"}"#,
            r#"{"id":1,"cmd":"query.status"}"#,
            r#"{"id":1,"cmd":"query.animations"}"#,
            r#"{"id":1,"cmd":"query.editors"}"#,
            r#"{"id":1,"cmd":"query.undo"}"#,
        ] {
            let (reply, repaint) = run_detached(frame);
            assert_eq!(reply["error"]["code"], json!("no_document"), "{frame}");
            assert!(!repaint, "{frame}: a query never asks for a frame");
        }
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

    /// The default reply, the `fields` filter, and the pointer, run
    /// against the testament fixture: the same reply the handler
    /// frames.
    #[test]
    fn test_document_fields_filter_and_pointer_against_the_fixture() {
        let mut doc = load_test_doc();
        doc.dirty = true;

        let default =
            document_fields(&doc, &request(None, None, None, None).expect("parses")).expect("reads");
        assert_eq!(default["dirty"], json!(true));
        assert_eq!(default["counts"]["nodes"], json!(doc.mindmap.nodes.len()));
        assert!(!default.contains_key("mindmap"), "the model is opt-in");

        let only = document_fields(
            &doc,
            &request(Some(json!(["undo_depth"])), None, None, None).expect("parses"),
        )
        .expect("reads");
        assert_eq!(only.keys().collect::<Vec<_>>(), vec!["undo_depth"]);

        let node_id = doc
            .mindmap
            .nodes
            .keys()
            .next()
            .expect("fixture has nodes")
            .clone();
        let pointer = format!("/nodes/{node_id}/id");
        let one = document_fields(&doc, &request(None, Some(&pointer), None, None).expect("parses"))
            .expect("reads");
        assert_eq!(
            one["mindmap"],
            json!(node_id),
            "the pointer narrows the model to one value"
        );

        assert_eq!(
            document_fields(
                &doc,
                &request(None, Some("/nodes/no-such-node"), None, None).expect("parses")
            ),
            Err(Refusal::NotFound(
                "the pointer `/nodes/no-such-node` resolves to nothing in the model".into()
            ))
        );
    }

    /// **`query.document` byte-round-trips against the loaded model**
    /// through the whole request path (issue #64's acceptance
    /// criterion): asked for `mindmap`, the reply parses back to the
    /// model the document holds.
    #[test]
    fn test_the_model_field_round_trips_against_the_loaded_document() {
        let doc = load_test_doc();
        let reply = document_fields(
            &doc,
            &request(Some(json!(["mindmap"])), None, None, None).expect("parses"),
        )
        .expect("reads");
        let reparsed: baumhard::mindmap::model::MindMap =
            serde_json::from_value(reply["mindmap"].clone()).expect("the reply is a MindMap");
        assert_eq!(
            serde_json::to_value(&reparsed).expect("serializes"),
            serde_json::to_value(&doc.mindmap).expect("serializes"),
            "what the agent reads is what the app holds"
        );
    }

    fn scratch_path(label: &str) -> std::path::PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "mandala-query-{}-{}-{label}.json",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn test_to_file_writes_the_fields_and_reports_path_and_size() {
        let doc = load_test_doc();
        let path = scratch_path("write");
        let target = ToFile {
            path: path.to_str().expect("utf-8").into(),
            overwrite: false,
        };
        let fields = document_fields(
            &doc,
            &request(Some(json!(["mindmap", "counts"])), None, None, None).expect("parses"),
        )
        .expect("reads");
        let reply = write_sidecar(&target, &Value::Object(fields.clone())).expect("writes");
        let on_disk = std::fs::read(&path).expect("the file exists");
        assert_eq!(reply["path"], json!(target.path));
        assert_eq!(
            reply["bytes"],
            json!(on_disk.len()),
            "the size reported is the size written"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&on_disk).expect("JSON"),
            Value::Object(fields)
        );

        let again = write_sidecar(&target, &json!({}));
        assert!(
            matches!(&again, Err(Refusal::Invalid("to_file", message)) if message.contains("overwrite")),
            "an existing file is refused without overwrite: {again:?}"
        );
        assert_eq!(
            std::fs::read(&path).expect("still there"),
            on_disk,
            "and left untouched"
        );

        let overwriting = ToFile {
            overwrite: true,
            ..target
        };
        let small = write_sidecar(&overwriting, &json!({"a": 1})).expect("overwrites");
        assert_eq!(small["bytes"], json!(7));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), r#"{"a":1}"#);
        std::fs::remove_file(&path).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn test_to_file_never_follows_a_symlink() {
        let victim = scratch_path("victim");
        std::fs::write(&victim, "precious").expect("write victim");
        let link = scratch_path("link");
        std::os::unix::fs::symlink(&victim, &link).expect("symlink");
        for overwrite in [false, true] {
            let target = ToFile {
                path: link.to_str().expect("utf-8").into(),
                overwrite,
            };
            assert!(
                matches!(
                    write_sidecar(&target, &json!({})),
                    Err(Refusal::Invalid("to_file", _))
                ),
                "overwrite={overwrite}: a symlink is refused"
            );
        }
        assert_eq!(std::fs::read_to_string(&victim).expect("read"), "precious");
        std::fs::remove_file(&link).expect("cleanup");
        std::fs::remove_file(&victim).expect("cleanup");
    }

    #[test]
    fn test_to_file_into_a_missing_directory_is_invalid_params() {
        let target = ToFile {
            path: scratch_path("dir")
                .join("nested")
                .join("x.json")
                .to_str()
                .expect("utf-8")
                .into(),
            overwrite: false,
        };
        assert!(matches!(
            write_sidecar(&target, &json!({})),
            Err(Refusal::Invalid("to_file", message)) if message.contains("parent directory")
        ));
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

    /// **Zero mutation, through the request path** (issue #64's
    /// acceptance criterion): every projection a query frames, run
    /// against a document with a live selection, an undo stack and
    /// unsaved changes, leaves `dirty`, the undo depth and the model
    /// as they were.
    #[test]
    fn test_running_every_document_query_leaves_the_document_untouched() {
        let mut doc = load_test_doc();
        doc.dirty = true;
        doc.undo_stack.push(UndoAction::CreateEdge { index: 0 });
        let model_before = serde_json::to_value(&doc.mindmap).expect("serializes");

        let all_fields = request(
            Some(json!([
                "file_path",
                "dirty",
                "undo_depth",
                "map",
                "counts",
                "mindmap"
            ])),
            None,
            None,
            None,
        )
        .expect("parses");
        let _ = document_fields(&doc, &all_fields).expect("reads");
        let _ = doc.selection.to_ipc_json();
        let _ = doc.ipc_animations_json(0);
        let _ = doc.ipc_undo_json(UNDO_DEFAULT_LIMIT);

        assert!(doc.dirty);
        assert_eq!(doc.undo_stack.len(), 1);
        assert_eq!(
            serde_json::to_value(&doc.mindmap).expect("serializes"),
            model_before
        );
    }
}
