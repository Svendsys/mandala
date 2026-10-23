// SPDX-License-Identifier: MPL-2.0

//! Writing a file someone else named: the one writer behind every
//! command that puts bytes at a caller-chosen path — the IPC file
//! sidecar (`to_file` on `query.document` and `scene.dump`) and the
//! screenshot's PNG and geometry sidecar, from IPC and the console
//! alike.
//!
//! **Why one writer.** A path typed by a person, or composed by an
//! agent, is a path the app did not choose, and three guarantees are
//! owed to whoever owns what is already there: a symlink is never
//! followed, an existing file is never replaced unless replacing it
//! was asked for, and a failed write never leaves a truncated file
//! where a good one was. Each is easy to state and easy to lose in a
//! second copy, so they are stated and tested here once.
//!
//! Native-only: its consumers are (`CLAUDE.md` "Dual-target status").

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

/// Why [`publish`] wrote nothing.
#[derive(Debug, PartialEq)]
pub(crate) enum PublishRefusal {
    /// The path is the caller's mistake — a symlink, a directory, an
    /// existing file without `overwrite`, a parent that does not exist.
    /// The message names it.
    Invalid(String),
    /// The write itself failed: the disk, permissions, the producer.
    Internal(String),
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

/// Run `produce` into `file`, flush it and `fsync` it, and return how
/// many bytes it wrote.
fn produce_into(
    file: std::fs::File,
    produce: impl FnOnce(&mut dyn Write) -> Result<(), String>,
) -> Result<u64, String> {
    let mut writer = CountingWriter {
        inner: std::io::BufWriter::new(file),
        bytes: 0,
    };
    produce(&mut writer)?;
    writer.flush().map_err(|e| e.to_string())?;
    writer.inner.get_ref().sync_all().map_err(|e| e.to_string())?;
    Ok(writer.bytes)
}

/// Write what `produce` writes to `path`, and return how many bytes it
/// was.
///
/// **Streamed.** `produce` writes straight into the file through a
/// buffer, so a large export or image is never held twice.
///
/// **Atomic, and it never follows a symlink.** The bytes go to a
/// sibling temp file created exclusively with `O_NOFOLLOW`
/// ([`create_exclusive`]), are flushed and `fsync`ed, and only then
/// published under the destination name. A failure at any point (the
/// producer, a full disk) removes the temp file and leaves the
/// destination exactly as it was, so an earlier good file is never
/// replaced by a truncated one and a retry never trips over a
/// half-written file. Publishing is `rename(2)` with `overwrite`,
/// which replaces the directory entry itself and cannot write through
/// a symlink; without it, `link(2)`, which fails if the name exists,
/// so a file created in the window since the check is not clobbered.
/// (A filesystem without hard links falls back to a check and a
/// rename.) The up-front `symlink_metadata` look is for the error
/// message: it names a symlink, a directory or an existing file before
/// any bytes are written, but the guarantees above do not rest on it.
///
/// `path` should be absolute; the callers check that, each in its own
/// words.
pub(crate) fn publish(
    path: &Path,
    overwrite: bool,
    produce: impl FnOnce(&mut dyn Write) -> Result<(), String>,
) -> Result<u64, PublishRefusal> {
    let shown = path.display();
    let invalid = |message: String| PublishRefusal::Invalid(message);
    let (Some(parent), Some(_)) = (path.parent(), path.file_name()) else {
        return Err(invalid(format!("`{shown}` names no file")));
    };
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(invalid(format!(
                "`{shown}` is a symlink, which is never followed"
            )))
        }
        Ok(meta) if meta.is_dir() => return Err(invalid(format!("`{shown}` is a directory"))),
        Ok(_) if !overwrite => return Err(exists(path)),
        _ => {}
    }

    let temp = parent.join(format!(
        ".mandala-write-{}-{}.tmp",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let file = create_exclusive(&temp).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => invalid(format!("the parent directory of `{shown}` does not exist")),
        _ => PublishRefusal::Internal(format!("could not create a temp file beside `{shown}`: {e}")),
    })?;
    let discard_temp = || {
        // Best effort: the refusal is what matters, and a temp file
        // that outlives it is named `.mandala-write-*.tmp`.
        let _ = std::fs::remove_file(&temp);
    };
    let bytes = match produce_into(file, produce) {
        Ok(bytes) => bytes,
        Err(e) => {
            discard_temp();
            return Err(PublishRefusal::Internal(format!(
                "could not write `{shown}`: {e}"
            )));
        }
    };

    let published = if overwrite {
        std::fs::rename(&temp, path)
    } else {
        match std::fs::hard_link(&temp, path) {
            Ok(()) => {
                discard_temp();
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e),
            // No hard links here (some network and FAT filesystems):
            // check, then rename. The window only risks replacing a
            // file that appeared a moment ago, never a write through
            // a link.
            Err(_) if std::fs::symlink_metadata(path).is_err() => std::fs::rename(&temp, path),
            Err(_) => Err(std::io::ErrorKind::AlreadyExists.into()),
        }
    };
    published.map_err(|e| {
        discard_temp();
        match e.kind() {
            std::io::ErrorKind::AlreadyExists => exists(path),
            _ => PublishRefusal::Internal(format!("could not move the file into `{shown}`: {e}")),
        }
    })?;
    Ok(bytes)
}

/// The refusal for an existing file the caller did not ask to replace.
fn exists(path: &Path) -> PublishRefusal {
    PublishRefusal::Invalid(format!(
        "`{}` exists; set `overwrite` to replace it",
        path.display()
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fresh directory under the system temp dir, unique to this
    /// process and call.
    pub(crate) fn scratch_dir(label: &str) -> std::path::PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mandala-write-{}-{}-{label}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// What is left in `dir`, by name.
    pub(crate) fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("readable")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn bytes(content: &'static str) -> impl FnOnce(&mut dyn Write) -> Result<(), String> {
        move |out| out.write_all(content.as_bytes()).map_err(|e| e.to_string())
    }

    #[test]
    fn test_publish_refuses_an_existing_file_unless_overwrite_and_replaces_it_whole() {
        let dir = scratch_dir("overwrite");
        let path = dir.join("model.json");
        std::fs::write(&path, "precious").expect("seed");
        let refused = publish(&path, false, bytes("{}"));
        assert!(
            matches!(&refused, Err(PublishRefusal::Invalid(m)) if m.contains("overwrite")),
            "{refused:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "precious",
            "left untouched"
        );

        assert_eq!(publish(&path, true, bytes(r#"{"a":1}"#)), Ok(7));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), r#"{"a":1}"#);
        assert_eq!(names_in(&dir), vec!["model.json"], "no temp file is left behind");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// A producer that fails after it has already written part of its
    /// output: a disk filling up mid-write, in miniature.
    fn fails_midway(out: &mut dyn Write) -> Result<(), String> {
        out.write_all(&[b'x'; 1 << 16]).map_err(|e| e.to_string())?;
        Err("the disk is full".into())
    }

    /// **The write is atomic** (review of #174): a failure midway
    /// leaves the destination exactly as it was, with `overwrite` (the
    /// earlier good file survives) and without (no half-written file
    /// makes the retry fail), and no temp file behind.
    #[test]
    fn test_a_failed_write_leaves_the_destination_alone_and_no_temp_file() {
        let dir = scratch_dir("atomic");
        let path = dir.join("model.json");
        std::fs::write(&path, "previous good export").expect("seed");
        let failed = publish(&path, true, fails_midway);
        assert!(
            matches!(&failed, Err(PublishRefusal::Internal(m)) if m.contains("disk is full")),
            "{failed:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "previous good export",
            "the earlier export is not replaced by a truncated one"
        );
        assert_eq!(names_in(&dir), vec!["model.json"]);

        std::fs::remove_file(&path).expect("clear");
        let failed = publish(&path, false, fails_midway);
        assert!(matches!(failed, Err(PublishRefusal::Internal(_))), "{failed:?}");
        assert_eq!(
            names_in(&dir),
            Vec::<String>::new(),
            "no half-written file is left to make the retry fail"
        );
        publish(&path, false, bytes("{}")).expect("the retry succeeds");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// **Never follows a symlink** (review of #174): with or without
    /// `overwrite`, a link at the destination is refused and what it
    /// points at is untouched, and a link planted at the temp name
    /// cannot be written through either, because the temp file is
    /// created exclusively with `O_NOFOLLOW`.
    #[cfg(unix)]
    #[test]
    fn test_publish_never_writes_through_a_symlink() {
        let dir = scratch_dir("symlink");
        let victim = dir.join("victim");
        std::fs::write(&victim, "precious").expect("write victim");
        let link = dir.join("link");
        std::os::unix::fs::symlink(&victim, &link).expect("symlink");
        for overwrite in [false, true] {
            assert!(
                matches!(
                    publish(&link, overwrite, bytes("{}")),
                    Err(PublishRefusal::Invalid(m)) if m.contains("symlink")
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
    fn test_publish_refuses_a_directory_and_a_missing_parent() {
        let dir = scratch_dir("dirs");
        assert!(matches!(
            publish(&dir, true, bytes("{}")),
            Err(PublishRefusal::Invalid(m)) if m.contains("directory")
        ));
        let nested = dir.join("missing").join("x.json");
        assert!(matches!(
            publish(&nested, false, bytes("{}")),
            Err(PublishRefusal::Invalid(m)) if m.contains("parent directory")
        ));
        assert_eq!(names_in(&dir), Vec::<String>::new());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
