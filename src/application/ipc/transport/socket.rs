// SPDX-License-Identifier: MPL-2.0

//! The socket file's lifecycle: where it may be created, when an
//! existing path may be replaced, and how it is removed again
//! (`format/ipc.md` §Socket, flag, lifecycle and §Ownership and
//! permissions).
//!
//! The order is the security argument, so it is fixed:
//!
//! 1. **The directory chain is vetted first** — the parent and every
//!    ancestor above it must be owned by this user or by root, and
//!    writable by nobody else unless sticky. Whoever can rename any
//!    directory on the path can swap the socket's name for one of
//!    theirs, so checking the parent alone is not enough.
//! 2. **An existing path is replaced only when it is provably a stale
//!    socket of ours.**
//! 3. **The socket is bound where nobody else can reach it** — inside
//!    a fresh `0700` staging directory — made `0600`, and only then
//!    hard-linked to its real name. A link refuses an existing path,
//!    so the final step can never replace anything, and the socket is
//!    never reachable by another user with wider permissions.
//!
//! No step touches the process umask: it is process-wide, and
//! narrowing it for a moment would leak into whatever else the process
//! — or a parallel test — creates in that moment.
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
use std::sync::atomic::{AtomicU64, Ordering};

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
/// sequence: directory chain, existing path, staged owner-only bind.
///
/// Cost: a handful of syscalls per ancestor directory, once per
/// launch, before the event loop runs.
pub(super) fn bind(path: &Path) -> Result<(UnixListener, BoundSocket), String> {
    if path.as_os_str().is_empty() {
        return Err("`--ipc` needs a socket path, e.g. `--ipc $XDG_RUNTIME_DIR/mandala.sock`".to_string());
    }
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let me = rustix::process::geteuid().as_raw();
    prepare_parent(parent, me)?;
    clear_stale(path, me)?;
    bind_staged(path, parent)
}

/// Whether a directory with owner `uid` and permission bits `mode` is
/// a safe place on the socket's path, for a process running as `me`.
///
/// Owned by `me` or by root, because a directory's owner can always
/// rename or unlink what is in it, sticky bit or not. And writable by
/// nobody but its owner, or carrying the sticky bit, under which
/// others may create entries but not unlink or rename ours — one
/// predicate for group- and world-writable alike.
fn directory_is_trusted(uid: u32, mode: u32, me: u32) -> bool {
    const WRITABLE_BY_OTHERS: u32 = 0o022;
    const STICKY: u32 = 0o1000;
    let owner_is_trusted = uid == me || uid == 0;
    owner_is_trusted && (mode & WRITABLE_BY_OTHERS == 0 || mode & STICKY != 0)
}

/// Refuse `dir` unless it, and every directory above it, is
/// [`directory_is_trusted`]. The chain is read from the canonical
/// path, so a symlink anywhere on it is judged by where it leads.
fn vet_chain(dir: &Path, me: u32) -> Result<(), String> {
    let canonical = fs::canonicalize(dir)
        .map_err(|e| format!("cannot resolve the socket's directory {}: {e}", dir.display()))?;
    let leaf =
        fs::metadata(&canonical).map_err(|e| format!("cannot inspect {}: {e}", canonical.display()))?;
    if !leaf.is_dir() {
        return Err(format!(
            "the socket's parent {} is not a directory",
            dir.display()
        ));
    }
    for ancestor in canonical.ancestors() {
        let metadata =
            fs::metadata(ancestor).map_err(|e| format!("cannot inspect {}: {e}", ancestor.display()))?;
        let mode = metadata.permissions().mode();
        if !directory_is_trusted(metadata.uid(), mode, me) {
            return Err(format!(
                "refusing to bind under {} (owner uid {}, mode {:o}): another user could rename or \
                 replace the socket path there. Every directory on the path must be owned by you or \
                 root and writable only by its owner (or sticky); $XDG_RUNTIME_DIR is the usual \
                 choice",
                ancestor.display(),
                metadata.uid(),
                mode & 0o7777
            ));
        }
    }
    Ok(())
}

/// Vet the directory chain, create `parent` (owner-only, `0700`) if it
/// is missing, and vet again. The part that exists is vetted *before*
/// anything is created under it, and the whole chain after.
fn prepare_parent(parent: &Path, me: u32) -> Result<(), String> {
    let existing = parent
        .ancestors()
        .map(|a| {
            if a.as_os_str().is_empty() {
                Path::new(".")
            } else {
                a
            }
        })
        .find(|a| a.exists())
        .unwrap_or(Path::new("/"));
    vet_chain(existing, me)?;
    if !parent.exists() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|e| format!("cannot create the socket's directory {}: {e}", parent.display()))?;
    }
    vet_chain(parent, me)
}

/// Clear the way for a bind at `path`. A missing path is the common
/// case. An existing path is removed **only** when it is provably a
/// stale socket of ours: a Unix socket, owned by this user, that
/// refuses a probe connection. Anything else is left exactly as found
/// and the launch fails naming it — a typo'd `--ipc ~/thesis.txt`
/// must never delete the file.
fn clear_stale(path: &Path, me: u32) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("cannot inspect {}: {e}", path.display())),
    };
    let file_type = metadata.file_type();
    if !file_type.is_socket() {
        return Err(format!(
            "{} already exists and is {}; refusing to replace it — pick another `--ipc` path",
            path.display(),
            describe_kind(file_type)
        ));
    }
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

fn describe_kind(file_type: fs::FileType) -> &'static str {
    if file_type.is_symlink() {
        "a symbolic link"
    } else if file_type.is_dir() {
        "a directory"
    } else if file_type.is_file() {
        "a regular file"
    } else if file_type.is_fifo() {
        "a FIFO"
    } else {
        "not a socket"
    }
}

/// Staging directories created by this process, so concurrent binds —
/// parallel tests — never pick the same name.
static NEXT_STAGING: AtomicU64 = AtomicU64::new(0);

/// Bind inside a fresh `0700` staging directory in `parent`, make the
/// socket `0600`, then hard-link it to `path`. Nobody else can reach
/// the socket before the link exists, and the link itself refuses an
/// existing path. The staging name is kept short (`.mipc<n>/s`)
/// because a Unix socket path is length-limited and the staged path
/// is the one `bind` sees.
fn bind_staged(path: &Path, parent: &Path) -> Result<(UnixListener, BoundSocket), String> {
    let staging = loop {
        let candidate = parent.join(format!(".mipc{}", NEXT_STAGING.fetch_add(1, Ordering::Relaxed)));
        match DirBuilder::new().mode(0o700).create(&candidate) {
            Ok(()) => break candidate,
            // Left behind by a process that died mid-bind, or another
            // instance binding next to us; take the next name.
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(format!(
                    "cannot create a staging directory in {}: {e}",
                    parent.display()
                ))
            }
        }
    };
    let bound = bind_in(&staging, path);
    if let Err(e) = fs::remove_dir_all(&staging) {
        log::warn!(
            "ipc: could not remove staging directory {}: {e}",
            staging.display()
        );
    }
    bound
}

fn bind_in(staging: &Path, path: &Path) -> Result<(UnixListener, BoundSocket), String> {
    let staged = staging.join("s");
    let listener = UnixListener::bind(&staged).map_err(|e| {
        format!(
            "cannot bind the socket for {} (staged at {}): {e}",
            path.display(),
            staged.display()
        )
    })?;
    fs::set_permissions(&staged, Permissions::from_mode(0o600))
        .map_err(|e| format!("cannot restrict {} to owner-only: {e}", staged.display()))?;
    fs::hard_link(&staged, path).map_err(|e| match e.kind() {
        ErrorKind::AlreadyExists => format!(
            "{} appeared while the socket was being bound; refusing to replace it",
            path.display()
        ),
        _ => format!("cannot create the socket at {}: {e}", path.display()),
    })?;
    let metadata =
        fs::symlink_metadata(path).map_err(|e| format!("cannot inspect {}: {e}", path.display()))?;
    Ok((
        listener,
        BoundSocket {
            path: path.to_path_buf(),
            dev: metadata.dev(),
            ino: metadata.ino(),
        },
    ))
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
    fn test_directory_is_trusted_refuses_writable_by_others_without_sticky() {
        const ME: u32 = 1000;
        assert!(directory_is_trusted(ME, 0o700, ME));
        assert!(directory_is_trusted(ME, 0o755, ME));
        assert!(
            directory_is_trusted(0, 0o1777, ME),
            "root's sticky /tmp is accepted"
        );
        assert!(
            directory_is_trusted(ME, 0o1770, ME),
            "sticky group-writable is accepted"
        );
        assert!(!directory_is_trusted(ME, 0o777, ME));
        assert!(
            !directory_is_trusted(ME, 0o775, ME),
            "group-writable alone is refused"
        );
        assert!(
            !directory_is_trusted(ME, 0o757, ME),
            "world-writable alone is refused"
        );
    }

    #[test]
    fn test_directory_is_trusted_requires_the_owner_to_be_me_or_root() {
        const ME: u32 = 1000;
        const OTHER: u32 = 1001;
        // A sticky directory's owner can still unlink or rename what is
        // in it, so the sticky bit vouches only for root's directories.
        assert!(!directory_is_trusted(OTHER, 0o1777, ME));
        assert!(
            !directory_is_trusted(OTHER, 0o755, ME),
            "its owner can rename our entry"
        );
        assert!(directory_is_trusted(0, 0o755, ME));
        assert!(
            directory_is_trusted(0, 0o755, 0),
            "root trusts its own directories"
        );
        assert!(
            !directory_is_trusted(OTHER, 0o700, 0),
            "root does not trust a user's directory"
        );
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
        assert!(err.contains("another user could"), "{err}");
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

    #[test]
    fn test_bind_refuses_an_ancestor_other_users_can_write() {
        // `/shared(0777)/mine(0700)/m.sock`: `mine` is private, but any
        // user can rename it away from under `shared` and put their own
        // in its place.
        let dir = TempDir::new("ipc-open-ancestor");
        let shared = dir.join("shared");
        let mine = shared.join("mine");
        fs::create_dir_all(&mine).expect("mkdir");
        fs::set_permissions(&mine, Permissions::from_mode(0o700)).expect("chmod");
        fs::set_permissions(&shared, Permissions::from_mode(0o777)).expect("chmod");
        let err = bind(&mine.join("m.sock")).expect_err("an open ancestor is refused");
        assert!(
            err.contains(&shared.display().to_string()),
            "the refusal names the ancestor: {err}"
        );

        // The same check runs before anything is created under it.
        let err = bind(&shared.join("new/m.sock")).expect_err("refused before creating");
        assert!(err.contains("another user could"), "{err}");
        assert!(
            !shared.join("new").exists(),
            "nothing was created under the open ancestor"
        );
    }

    #[test]
    fn test_bind_leaves_only_the_socket_behind() {
        let dir = TempDir::new("ipc-staging");
        let path = dir.join("m.sock");
        let (_listener, _bound) = bind(&path).expect("bind");
        let entries: Vec<String> = fs::read_dir(dir.path())
            .expect("read_dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, ["m.sock"], "the staging directory is removed");
    }
}
