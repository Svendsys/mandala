// SPDX-License-Identifier: MPL-2.0

//! Writing a file someone else named: the one writer in the workspace
//! that puts bytes at a path the program did not choose — the app's IPC
//! file sidecar (`to_file` on `query.document` and `scene.dump`), the
//! screenshot's PNG and geometry sidecar and a recording's frames, from
//! IPC and the console alike, every map the app or `maptool` saves
//! (baumhard's `format::file::write`), and `mandalactl`'s screenshot
//! over `--ws` and `record assemble`'s GIF and MP4.
//!
//! **Why one writer.** A path typed by a person, or composed by an
//! agent, is a path the program did not choose, and three guarantees
//! are owed to whoever owns what is already there: a symlink is never
//! followed, an existing file is never replaced unless replacing it
//! was asked for, and a failed write never leaves a truncated file
//! where a good one was. Two more are owed to the writer's own temp
//! files: each has a name no other writer uses, and only what this
//! write created is ever removed. Each is easy to state and easy to
//! lose in a second copy, so they are stated and tested here once.
//!
//! **Why a crate of its own.** The app and `maptool` are two processes
//! that can save one map at the same moment, and `mandalactl` writes
//! beside them; one implementation serves the three. It is not a module
//! of baumhard only because `mandalactl` takes no baumhard dependency;
//! baumhard re-exports it as `baumhard::util::guarded_file`.
//!
//! **Two ways to name the file.** [`publish`] itself never writes
//! through a symlink: a link at the path is refused. A path the user
//! names on a command line — a map's save, `maptool`'s outputs,
//! `mandalactl`'s `record assemble out=` and screenshot path — is
//! instead resolved to its real file first ([`real_file`]), so a file
//! kept behind a symlink stays writable and the link stays a link; the
//! no-clobber rule then applies to that real file. A map's save writes
//! only into a `.mandala.json`, `maptool export` never into one: the
//! map's save is reachable over IPC, so that rule on the real file, not
//! a refusal of links, keeps a remote caller from writing map bytes
//! into a file that is not a map (amendment E1.1 on #253).
//!
//! **A writer that is another program.** `ffmpeg` opens its output
//! itself. [`PrivateDir`] gives it a directory this write created
//! exclusively beside the destination; what it writes there is
//! published the guarded way, and the directory, only ever this
//! write's, is removed after.
//!
//! **Windows.** The same guarantees, by the same calls: `create_new`
//! is `CREATE_NEW` with `FILE_FLAG_OPEN_REPARSE_POINT` (std sets the
//! flag for `create_new`), so a symlink at the temp name is opened as
//! itself, exists, and is refused rather than followed; a new directory
//! is `CreateDirectoryW`, which fails on any existing name; `rename` is
//! `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`, which replaces a
//! link at the destination rather than writing through it; a hard link
//! is `CreateHardLinkW`, which fails on an existing name. Two
//! differences: a file another process holds open cannot be replaced
//! (the write is refused and the temp file removed), and the private
//! directory takes its parent's permissions rather than mode `0700`.
//! Planting a symlink there needs a privilege a test cannot count on,
//! so the symlink tests are Unix-only; the rest run on both.
//!
//! The browser has no files, and no process id for the temp file's
//! name: there, [`publish`] and [`PrivateDir::beside`] refuse every
//! write and say so.
#![forbid(unsafe_code)]
#![warn(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicU32, Ordering};

/// Why [`publish`] wrote nothing.
#[derive(Debug, PartialEq)]
pub enum PublishRefusal {
    /// The path is the caller's mistake — a symlink, a directory, an
    /// existing file without `overwrite`, a parent that does not exist.
    /// The message names it.
    Invalid(String),
    /// The write itself failed: the disk, permissions, the producer.
    Internal(String),
}

#[cfg(not(target_arch = "wasm32"))]
/// A `Write` that counts what passes through it.
struct CountingWriter<W> {
    inner: W,
    bytes: u64,
}

#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
/// Distinguishes this process's temp names from each other.
static TEMP_SEQUENCE: AtomicU32 = AtomicU32::new(0);

#[cfg(not(target_arch = "wasm32"))]
/// How many temp names a write tries before it gives up. A name that is
/// taken — a crashed process's leftover under a reused pid, or a name
/// someone planted — is passed over for the next; only this many taken
/// in a row is a refusal.
const TEMP_ATTEMPTS: u32 = 64;

/// Create `path` as a new file for writing: exclusively (it must not
/// exist), and therefore **without following a symlink** there. On Unix
/// `create_new` is `O_CREAT|O_EXCL`, which fails on a link at the final
/// component wherever it points, dangling or not (POSIX); on Windows it
/// is `CREATE_NEW` with `FILE_FLAG_OPEN_REPARSE_POINT`, which opens a
/// link as itself, so the name exists and the create fails. The kernel
/// makes the refusal atomic with the create: there is no check-then-use
/// window to race.
#[cfg(not(target_arch = "wasm32"))]
fn create_exclusive(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(not(target_arch = "wasm32"))]
/// The first of [`TEMP_ATTEMPTS`] names `.mandala-write-<pid>-<n><suffix>`
/// in `parent` that `make` can create, and what `make` made of it. A
/// name `make` finds taken (`AlreadyExists`) is skipped and never
/// touched; any other error ends the search.
fn claim<T>(
    parent: &Path,
    suffix: &str,
    make: impl Fn(&Path) -> std::io::Result<T>,
) -> std::io::Result<(PathBuf, T)> {
    let mut last = None;
    for _ in 0..TEMP_ATTEMPTS {
        let name = parent.join(format!(
            ".mandala-write-{}-{}{suffix}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        match make(&name) {
            Ok(made) => return Ok((name, made)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::ErrorKind::AlreadyExists.into()))
}

#[cfg(not(target_arch = "wasm32"))]
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

/// The up-front look [`publish`] takes at `path` before it writes a
/// byte, for a caller that must refuse before it spends work producing
/// the file (an encoder, a second file beside the first): `path` names
/// no file, is a symlink, is a directory, or exists without
/// `overwrite`. It is for the message, not the guarantees — those are
/// the write's own, atomic with it — so a path that passes here can
/// still be refused by the write if something appears in between.
///
/// Cost: one `symlink_metadata`.
pub fn check(path: &Path, overwrite: bool) -> Result<(), PublishRefusal> {
    checked_parent(path, overwrite).map(|_| ())
}

/// [`check`], returning the directory the file goes in.
fn checked_parent(path: &Path, overwrite: bool) -> Result<&Path, PublishRefusal> {
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
    Ok(parent)
}

/// Write what `produce` writes to `path`, and return how many bytes it
/// was.
///
/// **Streamed.** `produce` writes straight into the file through a
/// buffer, so a large export or image is never held twice.
///
/// **Atomic, and it never follows a symlink.** The bytes go to a
/// sibling temp file created exclusively (`create_exclusive`), are
/// flushed and `fsync`ed, and only then published under the
/// destination name. The temp file's name is this process's and this
/// write's (`.mandala-write-<pid>-<n>.tmp`); a name already taken — a
/// crashed process's leftover under a reused pid, or one someone
/// planted — is passed over for the next and left as it is. A failure
/// at any point (the producer, a full disk) removes the temp file and
/// leaves the destination exactly as it was, so an earlier good file
/// is never replaced by a truncated one and a retry never trips over a
/// half-written file. Publishing is `rename(2)` with `overwrite`,
/// which replaces the directory entry itself and cannot write through
/// a symlink; without it, `link(2)`, which fails if the name exists,
/// so a file created in the window since the check is not clobbered.
/// (A filesystem without hard links falls back to a check and a
/// rename.) The up-front look ([`check`]) is for the error message: it
/// names a symlink, a directory or an existing file before any bytes
/// are written, but the guarantees above do not rest on it.
///
/// `path` should be absolute where it comes from someone else; the
/// app's callers check that, each in its own words.
///
/// Cost: the bytes written once, an `fsync`, and a rename (or a hard
/// link and an unlink); no copy of the output is held.
#[cfg(not(target_arch = "wasm32"))]
pub fn publish(
    path: &Path,
    overwrite: bool,
    produce: impl FnOnce(&mut dyn Write) -> Result<(), String>,
) -> Result<u64, PublishRefusal> {
    let shown = path.display();
    let parent = checked_parent(path, overwrite)?;
    let (temp, file) = claim(parent, ".tmp", create_exclusive).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            PublishRefusal::Invalid(format!("the parent directory of `{shown}` does not exist"))
        }
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
    commit(&temp, path, overwrite).inspect_err(|_| discard_temp())?;
    Ok(bytes)
}

/// Publish the finished file at `temp` (this write's own, in the
/// destination's directory) under `path`: `rename(2)` with `overwrite`,
/// `link(2)` and an unlink of `temp` without it. A failure leaves
/// `path` as it was and `temp` for the caller to remove.
#[cfg(not(target_arch = "wasm32"))]
fn commit(temp: &Path, path: &Path, overwrite: bool) -> Result<(), PublishRefusal> {
    let published = if overwrite {
        std::fs::rename(temp, path)
    } else {
        match std::fs::hard_link(temp, path) {
            Ok(()) => {
                let _ = std::fs::remove_file(temp);
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e),
            // No hard links here (some network and FAT filesystems):
            // check, then rename. The window only risks replacing a
            // file that appeared a moment ago, never a write through
            // a link.
            Err(_) if std::fs::symlink_metadata(path).is_err() => std::fs::rename(temp, path),
            Err(_) => Err(std::io::ErrorKind::AlreadyExists.into()),
        }
    };
    published.map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => exists(path),
        _ => PublishRefusal::Internal(format!("could not move the file into `{}`: {e}", path.display())),
    })
}

/// A directory this write created exclusively beside a destination, for
/// a producer that is another program and opens its output itself
/// (`mandalactl`'s `ffmpeg`): it writes inside, under a name of the
/// caller's choosing, and [`PrivateDir::publish`] moves the result into
/// place the way [`publish`] does. The producer writes nowhere else,
/// so a symlink planted at a guessable name has nothing to catch.
///
/// The directory is `.mandala-write-<pid>-<n>.d` in the destination's
/// own directory, so the publish is a rename or a link on one
/// filesystem. It is created exclusively — a name already taken, by a
/// file, a directory or a symlink, is passed over for the next and left
/// as it is — and on Unix with mode `0700`, so no other user can plant
/// anything inside it. Dropping it removes it and everything in it,
/// whether the producer succeeded or failed; it only ever holds a
/// directory this value created.
#[derive(Debug)]
pub struct PrivateDir {
    path: PathBuf,
}

impl PrivateDir {
    /// Create a private directory in `destination`'s directory. Refused
    /// as [`publish`] refuses: `destination` names no file, or its
    /// parent directory does not exist. `destination` itself is not
    /// looked at; [`check`] does that.
    ///
    /// Cost: one `mkdir`, and one more per name found taken.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn beside(destination: &Path) -> Result<Self, PublishRefusal> {
        let shown = destination.display();
        let (Some(parent), Some(_)) = (destination.parent(), destination.file_name()) else {
            return Err(PublishRefusal::Invalid(format!("`{shown}` names no file")));
        };
        let (path, ()) = claim(parent, ".d", create_private_dir).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                PublishRefusal::Invalid(format!("the parent directory of `{shown}` does not exist"))
            }
            _ => PublishRefusal::Internal(format!(
                "could not create a private directory beside `{shown}`: {e}"
            )),
        })?;
        Ok(Self { path })
    }

    /// [`PrivateDir::beside`] in the browser, which has no files: always
    /// refused. O(1).
    #[cfg(target_arch = "wasm32")]
    pub fn beside(destination: &Path) -> Result<Self, PublishRefusal> {
        Err(PublishRefusal::Internal(format!(
            "could not write `{}`: the browser has no files",
            destination.display()
        )))
    }

    /// Where the producer writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Publish the file the producer wrote at `name` inside this
    /// directory under `path`, and return its size: refused as
    /// [`check`] refuses; the file must be a regular file (not a
    /// symlink); it is `fsync`ed, then moved into place exactly as
    /// [`publish`] moves its temp file — `rename(2)` with `overwrite`,
    /// `link(2)` without it, the same fallback without hard links. A
    /// failure leaves `path` as it was.
    ///
    /// Cost: an `fsync`, and a rename (or a hard link and an unlink).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn publish(&self, name: &str, path: &Path, overwrite: bool) -> Result<u64, PublishRefusal> {
        let shown = path.display();
        checked_parent(path, overwrite)?;
        let made = self.path.join(name);
        let internal = |e: String| PublishRefusal::Internal(format!("could not write `{shown}`: {e}"));
        let meta = std::fs::symlink_metadata(&made).map_err(|e| internal(e.to_string()))?;
        if !meta.file_type().is_file() {
            return Err(internal(format!("`{}` is not a regular file", made.display())));
        }
        // Synced, and closed at the end of this statement before it is
        // published: Windows refuses to rename an open file.
        std::fs::OpenOptions::new()
            .write(true)
            .open(&made)
            .and_then(|file| file.sync_all())
            .map_err(|e| internal(e.to_string()))?;
        commit(&made, path, overwrite)?;
        Ok(meta.len())
    }

    /// [`PrivateDir::publish`] in the browser, where no `PrivateDir` is
    /// ever made ([`PrivateDir::beside`] refuses): refused. O(1).
    #[cfg(target_arch = "wasm32")]
    pub fn publish(&self, _name: &str, path: &Path, _overwrite: bool) -> Result<u64, PublishRefusal> {
        Err(PublishRefusal::Internal(format!(
            "could not write `{}`: the browser has no files",
            path.display()
        )))
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        // Best effort, as for `publish`'s temp file: a directory that
        // outlives a failure is named `.mandala-write-*.d`.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Create the directory `path` and nothing else: it must not exist, as
/// a file, a directory or a symlink (`mkdir(2)` and `CreateDirectoryW`
/// both fail on any existing name), and on Unix only its owner may
/// enter it.
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    let builder = {
        let mut builder = std::fs::DirBuilder::new();
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
    };
    #[cfg(not(unix))]
    let builder = std::fs::DirBuilder::new();
    builder.create(path)
}

/// The real file a path names: the path itself when it is not a symlink
/// (or does not exist yet, so it is created), else the file at the end
/// of its chain of links. A write published there with [`publish`] goes
/// *through* the link to its target, atomically, and leaves the link a
/// link.
///
/// For the paths a map's save, `maptool`'s outputs and `mandalactl`'s
/// files (`record assemble out=`, a screenshot over `--ws`) are written
/// to, where a file kept behind a symlink (a dotfile setup, a synced
/// folder) must stay writable. A map's save is reachable over IPC too
/// (`act.console` with `save <path>`, `SaveDocumentAs`), so a caller of
/// this function owes the rule that keeps a remote caller in bounds on
/// the *resolved* path: a map's save requires the real file to be a
/// `.mandala.json` as well (baumhard's `format::file::write`, amendment
/// E1.1), and `maptool export` refuses a real file that is a map.
/// `mandalactl`'s paths are named on its own command line, never by a
/// remote caller. [`publish`] alone, for every other caller-chosen
/// path, refuses a symlink outright.
///
/// Refused, naming the path and where its link leads: a chain that ends
/// nowhere ("does not exist"), one that cannot be resolved — a loop
/// says so, in the system's own words — and one that ends at something
/// that is not a regular file. In the browser, which has no files, the
/// path is returned as it is and [`publish`] refuses it.
///
/// Cost: one `symlink_metadata`, and for a link one `read_link`, one
/// `canonicalize` (a walk of its chain) and one `metadata`.
pub fn real_file(path: &Path) -> Result<std::path::PathBuf, PublishRefusal> {
    let is_link = std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink());
    if !is_link {
        return Ok(path.to_path_buf());
    }
    let shown = path.display();
    let leads = std::fs::read_link(path)
        .map(|to| to.display().to_string())
        .unwrap_or_else(|e| format!("somewhere unreadable ({e})"));
    let real = std::fs::canonicalize(path).map_err(|e| {
        PublishRefusal::Invalid(match e.kind() {
            std::io::ErrorKind::NotFound => {
                format!("`{shown}` is a symlink to `{leads}`, which does not exist")
            }
            _ => format!("`{shown}` is a symlink to `{leads}`, which cannot be resolved: {e}"),
        })
    })?;
    match std::fs::metadata(&real) {
        Ok(meta) if meta.is_file() => Ok(real),
        Ok(meta) => Err(PublishRefusal::Invalid(format!(
            "`{shown}` is a symlink to `{leads}`, which leads to `{}`, {}, not a regular file",
            shown_path(&real),
            if meta.is_dir() {
                "a directory"
            } else {
                "something else"
            }
        ))),
        Err(e) => Err(PublishRefusal::Invalid(format!(
            "`{shown}` is a symlink to `{leads}`, which cannot be read: {e}"
        ))),
    }
}

/// `path` as a message shows it: on Windows a resolved path is
/// verbatim (`\\?\C:\…`), and the prefix only the system needs is
/// dropped, so the user reads the path they know. Cost: one `String`.
pub fn shown_path(path: &Path) -> String {
    let text = path.display().to_string();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => rest.to_string(),
        None => text,
    }
}

/// [`publish`] in the browser,
/// which has no files: every write is refused, and `produce` is not
/// run. O(1).
#[cfg(target_arch = "wasm32")]
pub fn publish(
    path: &Path,
    _overwrite: bool,
    _produce: impl FnOnce(&mut dyn Write) -> Result<(), String>,
) -> Result<u64, PublishRefusal> {
    Err(PublishRefusal::Internal(format!(
        "could not write `{}`: the browser has no files",
        path.display()
    )))
}

/// The refusal for an existing file the caller did not ask to replace.
fn exists(path: &Path) -> PublishRefusal {
    PublishRefusal::Invalid(format!(
        "`{}` exists; set `overwrite` to replace it",
        path.display()
    ))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    /// A scratch directory of the test's own, removed on drop. Its own
    /// rather than `tempfile`'s or baumhard's `util::test_temp`: this
    /// crate depends on nothing, and these few lines are all its tests
    /// need, so a dev-dependency would be a second mechanism for one.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "guarded-file-test-{}-{}-{label}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a scratch directory");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The names of what is in `dir`, sorted: what a writer left behind.
    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("readable")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// Every test that takes a temp name holds this, so a test that
    /// reads [`TEMP_SEQUENCE`] knows the next name no other test takes
    /// first.
    fn serial() -> MutexGuard<'static, ()> {
        static WRITES: Mutex<()> = Mutex::new(());
        WRITES.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The `n`th temp name in `dir` with `suffix`, as [`claim`] spells it.
    fn temp_name(dir: &Path, n: u32, suffix: &str) -> PathBuf {
        dir.join(format!(".mandala-write-{}-{n}{suffix}", std::process::id()))
    }

    fn bytes(content: &'static str) -> impl FnOnce(&mut dyn Write) -> Result<(), String> {
        move |out| out.write_all(content.as_bytes()).map_err(|e| e.to_string())
    }

    #[test]
    fn test_publish_refuses_an_existing_file_unless_overwrite_and_replaces_it_whole() {
        let _serial = serial();
        let scratch = Scratch::new("overwrite");
        let dir = &scratch.0;
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
        assert_eq!(names_in(dir), vec!["model.json"], "no temp file is left behind");
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
        let _serial = serial();
        let scratch = Scratch::new("atomic");
        let dir = &scratch.0;
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
        assert_eq!(names_in(dir), vec!["model.json"]);

        std::fs::remove_file(&path).expect("clear");
        let failed = publish(&path, false, fails_midway);
        assert!(matches!(failed, Err(PublishRefusal::Internal(_))), "{failed:?}");
        assert_eq!(
            names_in(dir),
            Vec::<String>::new(),
            "no half-written file is left to make the retry fail"
        );
        publish(&path, false, bytes("{}")).expect("the retry succeeds");
    }

    #[test]
    fn test_publish_refuses_a_directory_and_a_missing_parent() {
        let _serial = serial();
        let scratch = Scratch::new("dirs");
        let dir = &scratch.0;
        assert!(matches!(
            publish(dir, true, bytes("{}")),
            Err(PublishRefusal::Invalid(m)) if m.contains("directory")
        ));
        let nested = dir.join("missing").join("x.json");
        assert!(matches!(
            publish(&nested, false, bytes("{}")),
            Err(PublishRefusal::Invalid(m)) if m.contains("parent directory")
        ));
        assert_eq!(names_in(dir), Vec::<String>::new());
    }

    /// **A leftover at the next temp name is passed over, not an error,
    /// and left as it is** (ruling B38.1 §4): a crashed process's temp
    /// file under a pid this process now has, at the very sequence
    /// number this write takes first. The producer looks at the
    /// directory while it writes, which pins both that the planted name
    /// is the one the write tried first and that it wrote under the
    /// next one.
    ///
    /// Control: a `claim` that returns its first `AlreadyExists` (no
    /// retry) refuses the write.
    #[test]
    fn test_a_leftover_at_the_next_temp_name_is_passed_over_and_left_alone() {
        let _serial = serial();
        let scratch = Scratch::new("leftover");
        let dir = scratch.0.clone();
        let next = TEMP_SEQUENCE.load(Ordering::Relaxed);
        let leftover = temp_name(&dir, next, ".tmp");
        std::fs::write(&leftover, "stale").expect("plant");
        let path = dir.join("model.json");
        let seen = std::cell::RefCell::new(Vec::new());
        let written = publish(&path, false, |out| {
            *seen.borrow_mut() = names_in(&dir);
            out.write_all(b"{}").map_err(|e| e.to_string())
        });
        assert_eq!(written, Ok(2), "the leftover does not block the write");
        let leftover_name = leftover
            .file_name()
            .expect("named")
            .to_string_lossy()
            .into_owned();
        let used = temp_name(&dir, next + 1, ".tmp");
        let used_name = used.file_name().expect("named").to_string_lossy().into_owned();
        assert_eq!(
            *seen.borrow(),
            {
                let mut both = vec![leftover_name.clone(), used_name];
                both.sort();
                both
            },
            "precondition: the write tried the planted name first and wrote under the next"
        );
        assert_eq!(std::fs::read_to_string(&leftover).expect("kept"), "stale");
        assert_eq!(std::fs::read_to_string(&path).expect("written"), "{}");
        assert_eq!(names_in(&dir), vec![leftover_name, "model.json".to_string()]);
    }

    /// **A hard link planted at the next temp name is never written
    /// through**: the planting anyone can do on both platforms, with no
    /// privilege (Windows needs one for a symlink). The write passes
    /// over it, and the file it shares is untouched.
    ///
    /// Control: `create_exclusive` with `.create(true).truncate(true)`
    /// in place of `.create_new(true)` truncates the victim through the
    /// link and writes the map into it.
    #[test]
    fn test_a_hard_link_at_the_next_temp_name_is_not_written_through() {
        let _serial = serial();
        let scratch = Scratch::new("hardlink");
        let dir = &scratch.0;
        let victim = dir.join("yours.txt");
        std::fs::write(&victim, "precious").expect("seed");
        let next = TEMP_SEQUENCE.load(Ordering::Relaxed);
        let planted = temp_name(dir, next, ".tmp");
        std::fs::hard_link(&victim, &planted).expect("plant");
        let path = dir.join("model.json");
        assert_eq!(publish(&path, false, bytes("{}")), Ok(2));
        assert_eq!(
            std::fs::read_to_string(&victim).expect("kept"),
            "precious",
            "the file the planted link shares is untouched"
        );
        assert_eq!(std::fs::read_to_string(&planted).expect("kept"), "precious");
        assert_eq!(std::fs::read_to_string(&path).expect("written"), "{}");
    }

    /// A verbatim Windows path loses the prefix only the system needs,
    /// a UNC one keeps its `\\server`, and any other path is shown as it
    /// is.
    ///
    /// Control: returning the display unchanged fails the first two.
    #[test]
    fn test_shown_path_drops_only_the_verbatim_prefix() {
        assert_eq!(
            shown_path(Path::new(r"\\?\C:\maps\m.mandala.json")),
            r"C:\maps\m.mandala.json"
        );
        assert_eq!(
            shown_path(Path::new(r"\\?\UNC\server\maps\m.mandala.json")),
            r"\\server\maps\m.mandala.json"
        );
        assert_eq!(
            shown_path(Path::new("/home/u/m.mandala.json")),
            "/home/u/m.mandala.json"
        );
    }

    /// **Never follows a symlink** (review of #174): with or without
    /// `overwrite`, a link at the destination is refused and what it
    /// points at is untouched; and a link planted at a temp name — to a
    /// file, to a directory, or dangling — is never created through,
    /// because the temp file's create is `create_new` (`O_CREAT|O_EXCL`,
    /// which fails on a link wherever it points). Through [`publish`]
    /// too: links planted at the next three temp names are passed over,
    /// and nothing behind them changes. This is what lets the crate do
    /// without `O_NOFOLLOW` (ruling B38.1 §2).
    ///
    /// Control: `create_exclusive` with `.create(true).truncate(true)`
    /// in place of `.create_new(true)` writes through the link to a
    /// file, creates the dangling link's target, and fails on the link
    /// to a directory with another kind of error.
    #[cfg(unix)]
    #[test]
    fn test_publish_never_writes_through_a_symlink() {
        let _serial = serial();
        let scratch = Scratch::new("symlink");
        let dir = &scratch.0;
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

        let victim_dir = dir.join("victim_dir");
        std::fs::create_dir(&victim_dir).expect("victim dir");
        let nowhere = dir.join("nowhere");
        let plant = |name: &Path| {
            let to_file = name.with_extension("to-file");
            let to_dir = name.with_extension("to-dir");
            let dangling = name.with_extension("dangling");
            std::os::unix::fs::symlink(&victim, &to_file).expect("symlink");
            std::os::unix::fs::symlink(&victim_dir, &to_dir).expect("symlink");
            std::os::unix::fs::symlink(&nowhere, &dangling).expect("symlink");
            [to_file, to_dir, dangling]
        };
        let untouched = || {
            assert_eq!(std::fs::read_to_string(&victim).expect("read"), "precious");
            assert_eq!(
                names_in(&victim_dir),
                Vec::<String>::new(),
                "nothing in the directory"
            );
            assert!(
                std::fs::symlink_metadata(&nowhere).is_err(),
                "the dangling target was not created"
            );
        };
        for planted in plant(&dir.join("planted")) {
            let refused = create_exclusive(&planted).map(|_| ());
            assert_eq!(
                refused.map_err(|e| e.kind()),
                Err(std::io::ErrorKind::AlreadyExists),
                "{}: a link at the name is refused as an existing name",
                planted.display()
            );
        }
        untouched();

        let next = TEMP_SEQUENCE.load(Ordering::Relaxed);
        let links = [
            (temp_name(dir, next, ".tmp"), victim.clone()),
            (temp_name(dir, next + 1, ".tmp"), victim_dir.clone()),
            (temp_name(dir, next + 2, ".tmp"), nowhere.clone()),
        ];
        for (name, to) in &links {
            std::os::unix::fs::symlink(to, name).expect("plant at a temp name");
        }
        let out = dir.join("out.json");
        assert_eq!(publish(&out, false, bytes("{}")), Ok(2));
        assert_eq!(std::fs::read_to_string(&out).expect("written"), "{}");
        untouched();
        for (name, to) in &links {
            assert_eq!(&std::fs::read_link(name).expect("still a link"), to);
        }
    }

    /// **The private directory is created only where nothing is**: an
    /// existing directory, a file, a symlink to a directory and a
    /// dangling one are each refused as existing names, and each is
    /// left as it was. On Unix it is its owner's alone (`0700`).
    ///
    /// Controls: a `create_private_dir` that clears the way first
    /// (`remove_file`, then `create`) removes the file and both links;
    /// `create_dir_all` in place of `create` accepts the directory and
    /// the link to one; without the `mode` call the directory is
    /// `0755`.
    #[test]
    fn test_a_private_dir_is_created_only_where_nothing_is() {
        let _serial = serial();
        let scratch = Scratch::new("private-create");
        let dir = &scratch.0;
        let existing = dir.join("existing.d");
        std::fs::create_dir(&existing).expect("dir");
        std::fs::write(existing.join("theirs"), "theirs").expect("theirs");
        let file = dir.join("file.d");
        std::fs::write(&file, "a file").expect("file");
        // Off Unix nothing is added: planting a symlink there takes a
        // privilege a test cannot count on.
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut taken = vec![existing.clone(), file.clone()];
        #[cfg(unix)]
        {
            let to_dir = dir.join("to-dir.d");
            std::os::unix::fs::symlink(&existing, &to_dir).expect("symlink");
            let dangling = dir.join("dangling.d");
            std::os::unix::fs::symlink(dir.join("nowhere"), &dangling).expect("symlink");
            taken.extend([to_dir, dangling]);
        }
        let before = names_in(dir);
        for name in &taken {
            assert_eq!(
                create_private_dir(name).map_err(|e| e.kind()),
                Err(std::io::ErrorKind::AlreadyExists),
                "{}: an existing name is refused",
                name.display()
            );
        }
        assert_eq!(names_in(dir), before, "nothing was removed or added");
        assert_eq!(std::fs::read_to_string(&file).expect("file"), "a file");
        assert_eq!(names_in(&existing), vec!["theirs"]);

        let fresh = dir.join("fresh.d");
        create_private_dir(&fresh).expect("a free name");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&fresh).expect("made").permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "owner only: {mode:o}");
        }
    }

    /// **A private directory passes over a taken name, publishes what
    /// was written inside, and leaves nothing of its own behind**: a
    /// directory already at the next name (another run's) is skipped
    /// and survives with what is in it; the file written inside is
    /// published to the destination; dropping the private directory
    /// removes it, and only it.
    ///
    /// Controls: a `claim` with no retry refuses `beside`; a `Drop`
    /// that removes nothing leaves the `.d` directory behind.
    #[test]
    fn test_a_private_dir_passes_over_a_taken_name_and_leaves_only_what_it_published() {
        let _serial = serial();
        let scratch = Scratch::new("private-publish");
        let dir = &scratch.0;
        let next = TEMP_SEQUENCE.load(Ordering::Relaxed);
        let theirs = temp_name(dir, next, ".d");
        std::fs::create_dir(&theirs).expect("another run's");
        std::fs::write(theirs.join("video.mp4"), "theirs").expect("theirs");
        let dest = dir.join("out.mp4");
        let private = PrivateDir::beside(&dest).expect("a private directory");
        assert_eq!(
            private.path(),
            temp_name(dir, next + 1, ".d"),
            "the taken name is passed over"
        );
        std::fs::write(private.path().join("video.mp4"), "ours").expect("the producer writes");
        assert_eq!(private.publish("video.mp4", &dest, false), Ok(4));
        drop(private);
        assert_eq!(std::fs::read_to_string(&dest).expect("published"), "ours");
        let theirs_name = theirs.file_name().expect("named").to_string_lossy().into_owned();
        assert_eq!(names_in(dir), vec![theirs_name, "out.mp4".to_string()]);
        assert_eq!(
            std::fs::read_to_string(theirs.join("video.mp4")).expect("kept"),
            "theirs"
        );
    }

    /// **A private directory refuses as [`publish`] does, and is removed
    /// when nothing is published**: an existing destination without
    /// `overwrite` is refused and kept, and a symlink at the destination
    /// is refused even with it; a producer that wrote nothing, or left a
    /// symlink in place of its file, is refused naming why; in each case
    /// the directory is gone once dropped. With `overwrite` the
    /// destination is replaced.
    ///
    /// Controls: a `publish` that skips `checked_parent` replaces the
    /// link at the destination (the existing file is still refused, by
    /// the link that publishes it); one that skips the regular-file
    /// check publishes the link; a `Drop` that removes nothing leaves the
    /// directories.
    #[test]
    fn test_a_private_dir_refuses_as_publish_does_and_is_removed_after_a_failure() {
        let _serial = serial();
        let scratch = Scratch::new("private-refuse");
        let dir = &scratch.0;
        let dest = dir.join("out.mp4");
        std::fs::write(&dest, "precious").expect("seed");

        let private = PrivateDir::beside(&dest).expect("a private directory");
        std::fs::write(private.path().join("video.mp4"), "ours").expect("the producer writes");
        let refused = private.publish("video.mp4", &dest, false);
        assert!(
            matches!(&refused, Err(PublishRefusal::Invalid(m)) if m.contains("overwrite")),
            "{refused:?}"
        );
        drop(private);
        assert_eq!(std::fs::read_to_string(&dest).expect("kept"), "precious");
        assert_eq!(names_in(dir), vec!["out.mp4"], "the private directory is gone");

        let private = PrivateDir::beside(&dest).expect("a private directory");
        let missing = private.publish("video.mp4", &dest, true);
        assert!(matches!(missing, Err(PublishRefusal::Internal(_))), "{missing:?}");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&dest, private.path().join("video.mp4")).expect("symlink");
            let linked = private.publish("video.mp4", &dest, true);
            assert!(
                matches!(&linked, Err(PublishRefusal::Internal(m)) if m.contains("not a regular file")),
                "{linked:?}"
            );
            // A link at the destination is refused even with
            // `overwrite`, where a rename would replace it.
            std::fs::remove_file(private.path().join("video.mp4")).expect("clear");
            std::fs::write(private.path().join("video.mp4"), "ours").expect("the producer writes");
            let at_link = dir.join("at-link.mp4");
            std::os::unix::fs::symlink(&dest, &at_link).expect("symlink");
            let refused = private.publish("video.mp4", &at_link, true);
            assert!(
                matches!(&refused, Err(PublishRefusal::Invalid(m)) if m.contains("symlink")),
                "{refused:?}"
            );
            assert!(std::fs::symlink_metadata(&at_link)
                .expect("still there")
                .file_type()
                .is_symlink());
            std::fs::remove_file(&at_link).expect("clear");
        }
        drop(private);
        assert_eq!(std::fs::read_to_string(&dest).expect("kept"), "precious");
        assert_eq!(names_in(dir), vec!["out.mp4"], "the private directory is gone");

        let private = PrivateDir::beside(&dest).expect("a private directory");
        std::fs::write(private.path().join("video.mp4"), "ours").expect("the producer writes");
        assert_eq!(private.publish("video.mp4", &dest, true), Ok(4));
        drop(private);
        assert_eq!(std::fs::read_to_string(&dest).expect("replaced"), "ours");
        assert_eq!(names_in(dir), vec!["out.mp4"]);
    }

    /// `SKIPPED: {what}` on the process's stderr, past libtest's capture
    /// (the `Write` impl on the handle, not `eprintln!`), where
    /// `./test.sh` counts it — the shape of baumhard's
    /// `util::test_skip::note_skipped`, which this crate cannot reach.
    #[cfg(target_os = "linux")]
    fn note_skipped(what: &str) {
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "SKIPPED: {what}");
    }

    /// A filesystem without hard links, in miniature: `link(2)` and
    /// `linkat(2)` fail with `EPERM`, as on FAT and some network
    /// filesystems. Loaded with `LD_PRELOAD` into a child run of this
    /// test binary; std's `hard_link` calls `linkat` through the C
    /// library, which the preload replaces.
    #[cfg(target_os = "linux")]
    const NO_LINK_SHIM: &str = r#"#include <errno.h>
int link(const char *a, const char *b) { (void)a; (void)b; errno = EPERM; return -1; }
int linkat(int fa, const char *a, int fb, const char *b, int f) {
    (void)fa; (void)a; (void)fb; (void)b; (void)f; errno = EPERM; return -1;
}
"#;

    /// **Without hard links, one fallback: a check and a rename** (ruling
    /// B38.1 §5). The half that needs the filesystem to refuse links runs
    /// in a child run of this test binary with [`NO_LINK_SHIM`] preloaded,
    /// built here with the host's C compiler; with none (`cc` or `gcc`)
    /// the test says `SKIPPED:` and proves nothing. CI's Linux runners
    /// carry `gcc`. Linux only: `LD_PRELOAD` is the glibc loader's.
    ///
    /// Control: `commit` without the fallback arm (a failed link is the
    /// refusal) turns the child, and so this test, red.
    #[cfg(target_os = "linux")]
    #[test]
    fn test_without_hard_links_a_write_falls_back_to_a_check_and_a_rename() {
        let scratch = Scratch::new("nolink-shim");
        let source = scratch.0.join("nolink.c");
        std::fs::write(&source, NO_LINK_SHIM).expect("shim source");
        let shim = scratch.0.join("nolink.so");
        let built = ["cc", "gcc"].iter().any(|compiler| {
            std::process::Command::new(compiler)
                .args(["-shared", "-fPIC", "-o"])
                .arg(&shim)
                .arg(&source)
                .status()
                .is_ok_and(|status| status.success())
        });
        if !built {
            note_skipped(
                "guarded_file: no C compiler (cc, gcc) to build the no-hard-link shim; the fallback \
                 for a filesystem without hard links was not tested",
            );
            return;
        }
        let child = std::process::Command::new(std::env::current_exe().expect("this test binary"))
            .args([
                "--ignored",
                "--exact",
                "tests::test_without_hard_links_inner_half",
                "--nocapture",
            ])
            .env("LD_PRELOAD", &shim)
            .output()
            .expect("the child run");
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        assert!(
            said.contains("running 1 test"),
            "precondition: the inner half ran: {said}"
        );
        assert!(child.status.success(), "the inner half failed: {said}");
    }

    /// The inner half of the test above, run only by it, under the shim.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "run by test_without_hard_links_a_write_falls_back_to_a_check_and_a_rename, under LD_PRELOAD"]
    fn test_without_hard_links_inner_half() {
        let _serial = serial();
        let scratch = Scratch::new("nolink");
        let dir = &scratch.0;
        let probe = dir.join("probe");
        std::fs::write(&probe, "").expect("probe");
        assert!(
            std::fs::hard_link(&probe, dir.join("probe-link")).is_err(),
            "precondition: link(2) fails here (the shim is loaded)"
        );
        std::fs::remove_file(&probe).expect("clear");

        let path = dir.join("model.json");
        assert_eq!(
            publish(&path, false, bytes("{}")),
            Ok(2),
            "the fallback publishes"
        );
        assert_eq!(std::fs::read_to_string(&path).expect("written"), "{}");
        assert_eq!(names_in(dir), vec!["model.json"], "no temp file is left behind");
        let refused = publish(&path, false, bytes("[]"));
        assert!(
            matches!(&refused, Err(PublishRefusal::Invalid(m)) if m.contains("overwrite")),
            "{refused:?}"
        );
        assert_eq!(std::fs::read_to_string(&path).expect("kept"), "{}");

        let dest = dir.join("out.mp4");
        let private = PrivateDir::beside(&dest).expect("a private directory");
        std::fs::write(private.path().join("video.mp4"), "ours").expect("the producer writes");
        assert_eq!(private.publish("video.mp4", &dest, false), Ok(4));
        drop(private);
        assert_eq!(std::fs::read_to_string(&dest).expect("published"), "ours");
        assert_eq!(names_in(dir), vec!["model.json", "out.mp4"]);
    }
}
