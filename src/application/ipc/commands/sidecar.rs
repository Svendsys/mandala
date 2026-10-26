// SPDX-License-Identifier: MPL-2.0

//! The file sidecar of `format/ipc.md` §Large payloads: a command
//! whose reply can outgrow the 8 MiB frame cap takes `to_file` (and
//! `overwrite`), streams its result into that file atomically, and
//! replies with only `{"path", "bytes"}`. The parameter rules live
//! here, for every such command — `query.document` and `scene.dump`
//! today; the write itself is `guarded_file::publish` (native), the one
//! writer behind every caller-chosen path, so the no-symlink,
//! no-clobber, no-torn-file guarantees are stated and tested once.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

#[cfg(not(target_arch = "wasm32"))]
use crate::application::guarded_file::{self, PublishRefusal};

use super::refusal::Refusal;

/// Where a sidecar goes, as `to_file` / `overwrite` asked.
#[derive(Debug, PartialEq)]
pub(super) struct ToFile {
    pub(super) path: String,
    pub(super) overwrite: bool,
}

/// Parse and check the `to_file` / `overwrite` pair: `to_file` must be
/// absolute, and `overwrite` means nothing without it. Pure over the
/// raw values; both are already type-checked by the registry.
pub(super) fn parse_to_file(
    to_file: Option<&Value>,
    overwrite: Option<&Value>,
) -> Result<Option<ToFile>, Refusal> {
    match (to_file, overwrite.and_then(Value::as_bool)) {
        (None, Some(_)) => Err(Refusal::Invalid(
            "overwrite",
            "`overwrite` only applies with `to_file`".into(),
        )),
        (None, None) => Ok(None),
        (Some(path), overwrite) => {
            let path = path.as_str().unwrap_or_default();
            if !Path::new(path).is_absolute() {
                return Err(Refusal::Invalid(
                    "to_file",
                    format!("`{path}` is not an absolute path"),
                ));
            }
            Ok(Some(ToFile {
                path: path.to_string(),
                overwrite: overwrite.unwrap_or(false),
            }))
        }
    }
}

/// Write `value` to `target` as JSON and return what was written:
/// `{"path", "bytes"}`, the file sidecar of `format/ipc.md` §Large
/// payloads.
///
/// **Streamed.** `value` is serialized straight into the file; for the
/// document that means the model goes from `&MindMap` to bytes with no
/// value tree held beside it, which is the case the sidecar exists for.
/// Atomic and symlink-proof as [`guarded_file::publish`] is.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn write_sidecar(target: &ToFile, value: &impl Serialize) -> Result<Value, Refusal> {
    let bytes = guarded_file::publish(Path::new(&target.path), target.overwrite, |out| {
        serde_json::to_writer(out, value).map_err(|e| e.to_string())
    })
    .map_err(|refusal| match refusal {
        PublishRefusal::Invalid(message) => Refusal::Invalid("to_file", message),
        PublishRefusal::Internal(message) => Refusal::Internal(message),
    })?;
    Ok(serde_json::json!({ "path": target.path, "bytes": bytes }))
}

/// The browser has no filesystem, and its registry entries declare no
/// `to_file` (`format/ipc.md` §Transport "Browser"), so a request
/// naming one is refused as a stray parameter before any handler runs.
/// This is the handler's own refusal for the same fact, so the write
/// has one type on both targets.
#[cfg(target_arch = "wasm32")]
pub(super) fn write_sidecar(target: &ToFile, _value: &impl Serialize) -> Result<Value, Refusal> {
    Err(Refusal::Invalid(
        "to_file",
        format!(
            "`{}` cannot be written: the browser has no filesystem; read the reply inline",
            target.path
        ),
    ))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use serde_json::json;

    pub(in crate::application::ipc::commands) use crate::application::guarded_file::tests::{
        names_in, scratch_dir,
    };

    pub(in crate::application::ipc::commands) fn to_file(path: &Path, overwrite: bool) -> ToFile {
        ToFile {
            path: path.to_str().expect("utf-8").into(),
            overwrite,
        }
    }

    /// The JSON half of the sidecar: what lands in the file is the
    /// value as `serde_json` writes it, the reply names the path and
    /// counts the bytes, and a refusal of the path is charged to
    /// `to_file` — the parameter the client sent it in. The file
    /// guarantees themselves are `guarded_file`'s tests.
    #[test]
    fn test_write_sidecar_streams_the_json_and_names_to_file_in_a_refusal() {
        let dir = scratch_dir("sidecar");
        let path = dir.join("model.json");
        let written = write_sidecar(&to_file(&path, false), &json!({ "a": 1 })).expect("writes");
        assert_eq!(
            written,
            json!({ "path": path.to_str().expect("utf-8"), "bytes": 7 })
        );
        assert_eq!(std::fs::read_to_string(&path).expect("read"), r#"{"a":1}"#);

        let refused = write_sidecar(&to_file(&path, false), &json!({}));
        assert!(
            matches!(&refused, Err(Refusal::Invalid("to_file", m)) if m.contains("overwrite")),
            "{refused:?}"
        );
        assert_eq!(names_in(&dir), vec!["model.json"]);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
