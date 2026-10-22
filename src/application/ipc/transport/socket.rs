// SPDX-License-Identifier: MPL-2.0

//! The socket file's lifecycle: where it may be created, when an
//! existing path may be replaced, and how it is removed again
//! (`format/ipc.md` §Socket, flag, lifecycle and §Ownership and
//! permissions).
//!
//! The order is the security argument, so it is fixed: the parent
//! directory is vetted **first**, because the type-gated stale-socket
//! check that follows does a stat → unlink → bind sequence, and only a
//! parent no other user can write to closes the window in which
//! someone could swap the inode between the stat and the bind. The
//! bind then creates the socket owner-only from its first instant, so
//! there is no moment where another user could connect to it.
//!
//! Every failure is a `String` naming the path and the remedy. They
//! surface through the one startup `expect` in `run_native` — a bad
//! `--ipc` aborts the launch loudly rather than leaving IPC silently
//! missing (CODE_CONVENTIONS §9).

use std::fs::{self, DirBuilder, Permissions};
use std::io::ErrorKind;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// The socket file this process bound, identified by inode as well
/// as by name. Unlinking checks the identity first, so a graceful exit
/// never removes a socket some later process put at the same path.
///
/// The inode number is a sound identity only while it cannot be
/// recycled, and it cannot while the listener is open: a bound socket
/// holds a reference to its inode. [`BoundSocket::unlink`] therefore
/// runs while the acceptor thread still owns the listener.
#[derive(Debug)]
pub(super) struct BoundSocket {
    path: PathBuf,
    dev: u64,
    ino: u64,
}

impl BoundSocket {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Remove the socket file if it is still the one this process
    /// bound. Best-effort: exit proceeds either way, and the next
    /// launch's stale-socket check heals anything left behind.
    pub(super) fn unlink(&self) {
        let ours = fs::symlink_metadata(&self.path).is_ok_and(|m| m.dev() == self.dev && m.ino() == self.ino);
        if ours {
            if let Err(e) = fs::remove_file(&self.path) {
                log::warn!("ipc: could not remove socket {}: {e}", self.path.display());
            }
        }
    }
}

/// Bind the listening socket at `path`, running the whole vetted
/// sequence: parent directory, existing path, owner-only bind.
///
/// Cost: a handful of syscalls, once per launch, before the event loop
/// runs.
pub(super) fn bind(path: &Path) -> Result<(UnixListener, BoundSocket), String> {
    if path.as_os_str().is_empty() {
        return Err("`--ipc` needs a socket path, e.g. `--ipc $XDG_RUNTIME_DIR/mandala.sock`".to_string());
    }
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    prepare_parent(parent)?;
    clear_stale(path)?;
    bind_owner_only(path)
}

/// Whether a directory with permission bits `mode` is a safe home for
/// the socket's *name*: writable by nobody but its owner, or carrying
/// the sticky bit, under which others may create entries but not
/// unlink or rename ours. One predicate covers group- and
/// world-writable alike.
fn parent_mode_is_trusted(mode: u32) -> bool {
    const WRITABLE_BY_OTHERS: u32 = 0o022;
    const STICKY: u32 = 0o1000;
    mode & WRITABLE_BY_OTHERS == 0 || mode & STICKY != 0
}

/// Create `parent` (owner-only, `0700`) when it does not exist, then
/// refuse it unless [`parent_mode_is_trusted`].
fn prepare_parent(parent: &Path) -> Result<(), String> {
    let metadata = match fs::metadata(parent) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(|e| format!("cannot create the socket's directory {}: {e}", parent.display()))?;
            fs::metadata(parent)
                .map_err(|e| format!("cannot inspect the socket's directory {}: {e}", parent.display()))?
        }
        Err(e) => {
            return Err(format!(
                "cannot inspect the socket's directory {}: {e}",
                parent.display()
            ))
        }
    };
    if !metadata.is_dir() {
        return Err(format!(
            "the socket's parent {} is not a directory",
            parent.display()
        ));
    }
    let mode = metadata.permissions().mode();
    if !parent_mode_is_trusted(mode) {
        return Err(format!(
            "refusing to bind in {} (mode {:o}): it is writable by other users and has no sticky \
             bit, so they could replace the socket path; use a directory only you can write to, \
             such as $XDG_RUNTIME_DIR",
            parent.display(),
            mode & 0o7777
        ));
    }
    Ok(())
}

/// Clear the way for a bind at `path`. A missing path is the common
/// case. An existing path is removed **only** when it is provably a
/// stale socket of ours: a Unix socket, owned by this user, that
/// refuses a probe connection. Anything else is left exactly as found
/// and the launch fails naming it — a typo'd `--ipc ~/thesis.txt`
/// must never delete the file.
fn clear_stale(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("cannot inspect {}: {e}", path.display())),
    };
    let file_type = metadata.file_type();
    if !file_type.is_socket() {
        let kind = if file_type.is_symlink() {
            "a symbolic link"
        } else if file_type.is_dir() {
            "a directory"
        } else if file_type.is_file() {
            "a regular file"
        } else if file_type.is_fifo() {
            "a FIFO"
        } else {
            "not a socket"
        };
        return Err(format!(
            "{} already exists and is {kind}; refusing to replace it — pick another `--ipc` path",
            path.display()
        ));
    }
    let me = rustix::process::geteuid().as_raw();
    if metadata.uid() != me {
        return Err(format!(
            "{} is a socket owned by uid {}, not by you (uid {me}); refusing to replace it",
            path.display(),
            metadata.uid()
        ));
    }
    match UnixStream::connect(path) {
        Ok(_) => Err(format!(
            "another process is already listening on {}; is Mandala already running with this `--ipc` path?",
            path.display()
        )),
        Err(e) if e.kind() == ErrorKind::ConnectionRefused => fs::remove_file(path)
            .map_err(|e| format!("cannot remove the stale socket {}: {e}", path.display())),
        Err(e) => Err(format!(
            "cannot probe the existing socket {} for a live listener: {e}",
            path.display()
        )),
    }
}

/// Bind with the process umask narrowed to owner-only, so the socket
/// is `0600` from the instant it exists rather than after a `chmod`
/// another user could race. The umask is process-wide; this runs at
/// startup, before the window, the renderer or any thread that
/// creates files exists, and restores it immediately. The explicit
/// `chmod` after it covers platforms that ignore the umask for socket
/// files.
fn bind_owner_only(path: &Path) -> Result<(UnixListener, BoundSocket), String> {
    use rustix::fs::Mode;

    let previous = rustix::process::umask(Mode::from_raw_mode(0o177));
    let bound = UnixListener::bind(path);
    rustix::process::umask(previous);
    let listener = bound.map_err(|e| format!("cannot bind {}: {e}", path.display()))?;

    let identity = fs::set_permissions(path, Permissions::from_mode(0o600))
        .and_then(|()| fs::symlink_metadata(path))
        .map_err(|e| format!("cannot restrict {} to owner-only: {e}", path.display()));
    match identity {
        Ok(metadata) => Ok((
            listener,
            BoundSocket {
                path: path.to_path_buf(),
                dev: metadata.dev(),
                ino: metadata.ino(),
            },
        )),
        Err(message) => {
            let _ = fs::remove_file(path);
            Err(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::util::test_temp::TempDir;

    fn mode_of(path: &Path) -> u32 {
        fs::symlink_metadata(path)
            .expect("path exists")
            .permissions()
            .mode()
            & 0o7777
    }

    #[test]
    fn test_parent_mode_is_trusted_refuses_writable_by_others_without_sticky() {
        assert!(parent_mode_is_trusted(0o700));
        assert!(parent_mode_is_trusted(0o755));
        assert!(parent_mode_is_trusted(0o1777), "sticky /tmp is accepted");
        assert!(
            parent_mode_is_trusted(0o1770),
            "sticky group-writable is accepted"
        );
        assert!(!parent_mode_is_trusted(0o777));
        assert!(!parent_mode_is_trusted(0o775), "group-writable alone is refused");
        assert!(!parent_mode_is_trusted(0o757), "world-writable alone is refused");
    }

    #[test]
    fn test_bind_creates_an_owner_only_socket_and_unlink_removes_it() {
        let dir = TempDir::new("ipc-bind");
        let path = dir.join("mandala.sock");
        let (_listener, bound) = bind(&path).expect("bind in a private dir");
        assert!(fs::symlink_metadata(&path)
            .expect("exists")
            .file_type()
            .is_socket());
        assert_eq!(mode_of(&path), 0o600, "the socket is owner-only");
        bound.unlink();
        assert!(!path.exists(), "a graceful exit removes the socket");
    }

    #[test]
    fn test_bind_creates_missing_parents_owner_only() {
        let dir = TempDir::new("ipc-parents");
        let path = dir.join("a/b/mandala.sock");
        let (_listener, _bound) = bind(&path).expect("missing parents are created");
        assert_eq!(mode_of(&dir.join("a")), 0o700);
        assert_eq!(mode_of(&dir.join("a/b")), 0o700);
    }

    #[test]
    fn test_bind_refuses_a_parent_other_users_can_write() {
        let dir = TempDir::new("ipc-open-parent");
        let open = dir.join("open");
        fs::create_dir(&open).expect("mkdir");
        fs::set_permissions(&open, Permissions::from_mode(0o777)).expect("chmod");
        let err = bind(&open.join("mandala.sock")).expect_err("a world-writable parent is refused");
        assert!(err.contains("writable by other users"), "{err}");
        assert!(!open.join("mandala.sock").exists(), "nothing was created");

        fs::set_permissions(&open, Permissions::from_mode(0o1777)).expect("chmod");
        assert!(
            bind(&open.join("mandala.sock")).is_ok(),
            "the sticky bit makes it acceptable"
        );
    }

    #[test]
    fn test_bind_never_replaces_a_path_that_is_not_a_stale_socket() {
        let dir = TempDir::new("ipc-clobber");

        let file = dir.join("thesis.txt");
        fs::write(&file, "chapter one").expect("write");
        let err = bind(&file).expect_err("a regular file is never replaced");
        assert!(err.contains("a regular file"), "{err}");
        assert_eq!(fs::read_to_string(&file).expect("still there"), "chapter one");

        let sub = dir.join("sub");
        fs::create_dir(&sub).expect("mkdir");
        assert!(bind(&sub).expect_err("a directory").contains("a directory"));
        assert!(sub.is_dir());

        let link = dir.join("link.sock");
        std::os::unix::fs::symlink(&file, &link).expect("symlink");
        assert!(bind(&link).expect_err("a symlink").contains("a symbolic link"));
        assert_eq!(
            fs::read_to_string(&file).expect("target untouched"),
            "chapter one"
        );
    }

    #[test]
    fn test_bind_refuses_a_socket_with_a_live_listener() {
        let dir = TempDir::new("ipc-live");
        let path = dir.join("mandala.sock");
        let (_live, _bound) = bind(&path).expect("first instance binds");
        let err = bind(&path).expect_err("a second instance must not steal the path");
        assert!(err.contains("already listening"), "{err}");
        assert!(path.exists(), "the live socket is left in place");
    }

    #[test]
    fn test_bind_heals_a_stale_socket_of_ours() {
        let dir = TempDir::new("ipc-stale");
        let path = dir.join("mandala.sock");
        // An unclean exit: the listener is gone, the file is not.
        drop(UnixListener::bind(&path).expect("plant a socket"));
        assert!(path.exists());
        let (_listener, _bound) = bind(&path).expect("a stale socket of ours is replaced");
        assert_eq!(mode_of(&path), 0o600);
    }

    #[test]
    fn test_bind_requires_a_path() {
        assert!(bind(Path::new(""))
            .expect_err("empty")
            .contains("needs a socket path"));
    }

    #[test]
    fn test_unlink_leaves_a_successor_socket_alone() {
        let dir = TempDir::new("ipc-successor");
        let path = dir.join("mandala.sock");
        // The listener stays open, as it does in the app at exit; that
        // is what keeps its inode number from being handed to the
        // successor.
        let (_listener, bound) = bind(&path).expect("bind");
        fs::remove_file(&path).expect("someone removed ours");
        let _successor = UnixListener::bind(&path).expect("and bound their own");
        bound.unlink();
        assert!(
            path.exists(),
            "a different inode at the same path is not ours to remove"
        );
    }
}
