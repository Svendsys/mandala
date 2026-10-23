// SPDX-License-Identifier: MPL-2.0

//! The file sidecar of `format/ipc.md` §Large payloads: a command
//! whose reply can outgrow the 8 MiB frame cap takes `to_file` (and
//! `overwrite`), streams its result into that file atomically, and
//! replies with only `{"path", "bytes"}`. One writer for every such
//! command — `query.document` and `scene.dump` today — so the
//! no-symlink, no-clobber, no-torn-file guarantees are stated and
//! tested once.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::Serialize;
use serde_json::Value;

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
pub(super) fn write_sidecar(target: &ToFile, value: &impl Serialize) -> Result<Value, Refusal> {
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
        ".mandala-sidecar-{}-{}.tmp",
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
        // that outlives it is named `.mandala-sidecar-*.tmp`.
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

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use serde_json::json;

    /// A fresh directory under the system temp dir, unique to this
    /// process and call.
    pub(in crate::application::ipc::commands) fn scratch_dir(label: &str) -> std::path::PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mandala-sidecar-{}-{}-{label}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    pub(in crate::application::ipc::commands) fn to_file(path: &Path, overwrite: bool) -> ToFile {
        ToFile {
            path: path.to_str().expect("utf-8").into(),
            overwrite,
        }
    }

    /// What is left in `dir`, by name.
    pub(in crate::application::ipc::commands) fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("readable")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
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
}
