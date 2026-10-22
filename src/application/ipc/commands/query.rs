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

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::Serialize;
use serde_json::Value;

use crate::application::document::{DocumentField, MindMapDocument, PointerMiss};
use crate::application::ipc::envelope::{code, Reply};
use crate::application::ipc::registry::{Call, CommandSpec, Family, ParamSpec, ParamType, ResultSpec};
use crate::application::ipc::{CommandTarget, ShellState};

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
                    "Absolute path. Write the reply's fields there as JSON, streamed, and reply with \
                     just {path, bytes}. For a model too large to inline (an inline reply over 8 MiB \
                     is reply_too_large). The write is atomic: a temp file in the same directory, \
                     renamed into place.",
                ),
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

/// Distinguishes this process's temp files from each other.
static TEMP_SEQUENCE: AtomicU32 = AtomicU32::new(0);

/// Create `path` as a new file for writing: exclusively (it must not
/// exist), and **without following a symlink** at the final component.
/// The kernel makes both atomic with the create, so there is no
/// check-then-use window to race. A path that is already a symlink,
/// dangling or not, fails (`O_EXCL` and `O_NOFOLLOW` agree).
#[cfg(unix)]
fn create_exclusive(path: &Path) -> std::io::Result<std::fs::File> {
    use rustix::fs::{open, Mode, OFlags};
    let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    // The usual new-file bits; the process umask trims them.
    let mode = Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::WGRP | Mode::ROTH | Mode::WOTH;
    open(path, flags, mode)
        .map(std::fs::File::from)
        .map_err(std::io::Error::from)
}

/// Off Unix there is no `O_NOFOLLOW` to ask for. `create_new` still
/// refuses any existing path, which is the same refusal for a symlink.
#[cfg(not(unix))]
fn create_exclusive(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// Stream `value` into `file` as JSON, flush it and `fsync` it, and
/// return how many bytes it was.
fn stream_into(file: std::fs::File, value: &impl Serialize) -> Result<u64, String> {
    let mut writer = CountingWriter {
        inner: std::io::BufWriter::new(file),
        bytes: 0,
    };
    serde_json::to_writer(&mut writer, value).map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())?;
    writer.inner.get_ref().sync_all().map_err(|e| e.to_string())?;
    Ok(writer.bytes)
}

/// Write `value` to `target` as JSON and return what was written:
/// `{"path", "bytes"}`, the file sidecar of `format/ipc.md` §Large
/// payloads.
///
/// **Streamed.** `value` is serialized straight into the file; for the
/// document that means the model goes from `&MindMap` to bytes with no
/// value tree held beside it, which is the case the sidecar exists for.
///
/// **Atomic, and it never follows a symlink.** The bytes go to a
/// sibling temp file created exclusively with `O_NOFOLLOW`
/// ([`create_exclusive`]), are flushed and `fsync`ed, and only then
/// published under the destination name. A failure at any point
/// (serialization, a full disk) removes the temp file and leaves the
/// destination exactly as it was, so an earlier good export is never
/// replaced by truncated JSON and a retry never trips over a
/// half-written file. Publishing is `rename(2)` with `overwrite`,
/// which replaces the directory entry itself and cannot write
/// through a symlink; without it, `link(2)`, which fails if the name
/// exists, so a file created in the window since the check is not
/// clobbered. (A filesystem without hard links falls back to a check
/// and a rename.) The up-front `symlink_metadata` look is for the
/// error message: it names a symlink, a directory or an existing file
/// before any bytes are written, but the guarantees above do not rest
/// on it.
fn write_sidecar(target: &ToFile, value: &impl Serialize) -> Result<Value, Refusal> {
    let invalid = |message: String| Refusal::Invalid("to_file", message);
    let destination = Path::new(&target.path);
    let (Some(parent), Some(_)) = (destination.parent(), destination.file_name()) else {
        return Err(invalid(format!("`{}` names no file", target.path)));
    };
    match std::fs::symlink_metadata(destination) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(invalid(format!(
                "`{}` is a symlink, which is never followed",
                target.path
            )))
        }
        Ok(meta) if meta.is_dir() => return Err(invalid(format!("`{}` is a directory", target.path))),
        Ok(_) if !target.overwrite => {
            return Err(invalid(format!(
                "`{}` exists; pass `overwrite: true` to replace it",
                target.path
            )))
        }
        _ => {}
    }

    let temp = parent.join(format!(
        ".mandala-query-{}-{}.tmp",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let file = create_exclusive(&temp).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => invalid(format!(
            "the parent directory of `{}` does not exist",
            target.path
        )),
        _ => Refusal::Internal(format!(
            "could not create a temp file beside `{}`: {e}",
            target.path
        )),
    })?;
    let discard_temp = || {
        // Best effort: the refusal is what matters, and a temp file
        // that outlives it is named `.mandala-query-*.tmp`.
        let _ = std::fs::remove_file(&temp);
    };
    let bytes = match stream_into(file, value) {
        Ok(bytes) => bytes,
        Err(e) => {
            discard_temp();
            return Err(Refusal::Internal(format!(
                "could not write `{}`: {e}",
                target.path
            )));
        }
    };

    let published = if target.overwrite {
        std::fs::rename(&temp, destination)
    } else {
        match std::fs::hard_link(&temp, destination) {
            Ok(()) => {
                discard_temp();
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e),
            // No hard links here (some network and FAT filesystems):
            // check, then rename. The window only risks replacing a
            // file that appeared a moment ago, never a write through
            // a link.
            Err(_) if std::fs::symlink_metadata(destination).is_err() => std::fs::rename(&temp, destination),
            Err(_) => Err(std::io::ErrorKind::AlreadyExists.into()),
        }
    };
    published.map_err(|e| {
        discard_temp();
        match e.kind() {
            std::io::ErrorKind::AlreadyExists => invalid(format!(
                "`{}` exists; pass `overwrite: true` to replace it",
                target.path
            )),
            _ => Refusal::Internal(format!("could not move the export into `{}`: {e}", target.path)),
        }
    })?;
    Ok(serde_json::json!({ "path": target.path, "bytes": bytes }))
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
    // The app's wall clock (`common::now_ms()`, ms since process
    // start): the one the per-frame drain ticks animations against,
    // so `elapsed_ms` and `phase` agree with what the next tick will
    // do. It is not virtual; the `clock` family that would make it so
    // (IPC-09) does not exist yet. A drag-suspended tick is reported
    // as a pause rather than as a clock that keeps running.
    let now_ms = crate::application::common::now_ms() as u64;
    with_document(call, |doc, app| {
        doc.ipc_animations_json(now_ms, app.animation_pause())
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
    use crate::application::ipc::registry::tests::{run_attached, run_detached};
    use crate::application::ipc::space::Point;
    use crate::application::keybinds::Action;
    use crate::application::macros::MacroStep;
    use serde_json::json;

    /// A real loaded document behind the `CommandTarget` seam, for
    /// driving a query through the registry. The act methods are the
    /// funnels a query must never reach, so each one fails the test
    /// that calls it (the `MacroDispatchTarget` test double in
    /// `macro_core.rs` is the precedent: a real `IpcCommandTarget`
    /// needs a GPU renderer, which `TEST_CONVENTIONS` §T8 rules out).
    /// What it reports for the shell state is a fixed, recognizable
    /// value, since the real projections are tested where they live.
    struct LoadedApp {
        doc: Option<MindMapDocument>,
        pause: Option<u64>,
    }

    impl LoadedApp {
        fn with_fixture() -> Self {
            LoadedApp {
                doc: Some(load_test_doc()),
                pause: None,
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
        fn run_macro(&mut self, _: &str) -> bool {
            panic!("a query must not run a macro");
        }
        fn run_macro_steps(&mut self, _: &[MacroStep]) -> bool {
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

    fn scratch_dir(label: &str) -> std::path::PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mandala-query-{}-{}-{label}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn to_file(path: &Path, overwrite: bool) -> ToFile {
        ToFile {
            path: path.to_str().expect("utf-8").into(),
            overwrite,
        }
    }

    /// What is left in `dir`, by name.
    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("readable")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
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
    fn test_to_file_refuses_an_existing_file_unless_overwrite_and_replaces_it_whole() {
        let dir = scratch_dir("overwrite");
        let path = dir.join("model.json");
        std::fs::write(&path, "precious").expect("seed");
        let refused = write_sidecar(&to_file(&path, false), &json!({ "a": 1 }));
        assert!(
            matches!(&refused, Err(Refusal::Invalid("to_file", m)) if m.contains("overwrite")),
            "{refused:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "precious",
            "left untouched"
        );

        let replaced = write_sidecar(&to_file(&path, true), &json!({ "a": 1 })).expect("overwrites");
        assert_eq!(replaced["bytes"], json!(7));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), r#"{"a":1}"#);
        assert_eq!(names_in(&dir), vec!["model.json"], "no temp file is left behind");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// A serializer that fails after it has already written part of
    /// its output: a disk filling up mid-dump, in miniature.
    struct FailsMidway;

    impl Serialize for FailsMidway {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::{Error, SerializeMap};
            let mut map = serializer.serialize_map(None)?;
            map.serialize_entry("started", &"x".repeat(1 << 16))?;
            Err(S::Error::custom("the disk is full"))
        }
    }

    /// **The write is atomic** (review of #174): a failure midway
    /// leaves the destination exactly as it was, with `overwrite` (the
    /// earlier good export survives) and without (no half-written file
    /// makes the retry fail), and no temp file behind.
    #[test]
    fn test_a_failed_write_leaves_the_destination_alone_and_no_temp_file() {
        let dir = scratch_dir("atomic");
        let path = dir.join("model.json");
        std::fs::write(&path, "previous good export").expect("seed");
        let failed = write_sidecar(&to_file(&path, true), &FailsMidway);
        assert!(matches!(failed, Err(Refusal::Internal(_))), "{failed:?}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "previous good export",
            "the earlier export is not replaced by truncated JSON"
        );
        assert_eq!(names_in(&dir), vec!["model.json"]);

        std::fs::remove_file(&path).expect("clear");
        let failed = write_sidecar(&to_file(&path, false), &FailsMidway);
        assert!(matches!(failed, Err(Refusal::Internal(_))), "{failed:?}");
        assert_eq!(
            names_in(&dir),
            Vec::<String>::new(),
            "no half-written file is left to make the retry fail"
        );
        write_sidecar(&to_file(&path, false), &json!({})).expect("the retry succeeds");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// **Never follows a symlink** (review of #174): with or without
    /// `overwrite`, a link at the destination is refused and what it
    /// points at is untouched, and a link planted at the temp name
    /// cannot be written through either, because the temp file is
    /// created exclusively with `O_NOFOLLOW`.
    #[cfg(unix)]
    #[test]
    fn test_to_file_never_writes_through_a_symlink() {
        let dir = scratch_dir("symlink");
        let victim = dir.join("victim");
        std::fs::write(&victim, "precious").expect("write victim");
        let link = dir.join("link");
        std::os::unix::fs::symlink(&victim, &link).expect("symlink");
        for overwrite in [false, true] {
            assert!(
                matches!(
                    write_sidecar(&to_file(&link, overwrite), &json!({})),
                    Err(Refusal::Invalid("to_file", m)) if m.contains("symlink")
                ),
                "overwrite={overwrite}: a symlink destination is refused"
            );
        }
        assert_eq!(std::fs::read_to_string(&victim).expect("read"), "precious");

        // The exclusive, no-follow create itself: a symlink at the
        // name it is asked to create, dangling or not, is refused.
        let planted = dir.join("planted");
        std::os::unix::fs::symlink(&victim, &planted).expect("symlink");
        assert!(
            create_exclusive(&planted).is_err(),
            "a link to a file is not created through"
        );
        let dangling = dir.join("dangling");
        std::os::unix::fs::symlink(dir.join("nowhere"), &dangling).expect("symlink");
        assert!(create_exclusive(&dangling).is_err(), "nor is a dangling one");
        assert!(
            !dir.join("nowhere").exists(),
            "the dangling target was not created"
        );
        assert_eq!(std::fs::read_to_string(&victim).expect("read"), "precious");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn test_to_file_refuses_a_directory_and_a_missing_parent() {
        let dir = scratch_dir("dirs");
        assert!(matches!(
            write_sidecar(&to_file(&dir, true), &json!({})),
            Err(Refusal::Invalid("to_file", m)) if m.contains("directory")
        ));
        let nested = dir.join("missing").join("x.json");
        assert!(matches!(
            write_sidecar(&to_file(&nested, false), &json!({})),
            Err(Refusal::Invalid("to_file", m)) if m.contains("parent directory")
        ));
        assert_eq!(names_in(&dir), Vec::<String>::new());
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
