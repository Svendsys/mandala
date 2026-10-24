// SPDX-License-Identifier: MPL-2.0

//! The server-chosen artifacts dir (`format/ipc.md` §`capture`,
//! "Artifact lifecycle"): where a capture lands when the client names
//! no path.
//!
//! **Bounded, because nobody else will bound it.** A client that names
//! its own path owns what it writes there. A client that names none
//! hands the disk to the app, and an agent in a loop takes thousands of
//! screenshots. So the dir is created lazily under `$XDG_RUNTIME_DIR`
//! (or the system temp dir) on the first capture that needs it, scoped
//! to this process, capped at [`BUDGET_BYTES`] with the oldest captures
//! evicted first, and removed on graceful exit.
//!
//! **Owned, because the parent may be shared.** The dir is made with a
//! single `mkdir` at mode `0700`, which fails rather than adopting a
//! name someone else already took — in a world-writable temp dir a
//! pre-planted name would otherwise become ours — and a taken name is
//! retried under the next suffix.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// The most bytes the artifacts dir holds before it evicts its oldest
/// captures: 256 MiB, a small share of the tmpfs `$XDG_RUNTIME_DIR`
/// usually is. One capture larger than the whole budget is kept on its
/// own — the newest capture is never evicted, since the reply that
/// names it has not been read yet.
pub(crate) const BUDGET_BYTES: u64 = 256 << 20;

/// How many taken names [`Artifacts::dir`] steps past before it gives
/// up. Only a parent full of planted names reaches it.
const NAME_ATTEMPTS: u32 = 64;

/// What one capture wrote, evicted together: the files of a
/// screenshot, `(path, bytes)`, or the directory of a recording.
#[derive(Debug)]
struct Capture {
    files: Vec<(PathBuf, u64)>,
    /// A recording's directory, removed whole, with `bytes` counted for
    /// it. `None` for a screenshot.
    tree: Option<(PathBuf, u64)>,
}

/// The per-process artifacts dir and the captures in it, oldest first.
#[derive(Debug)]
pub(crate) struct Artifacts {
    /// Where the dir is made.
    root: PathBuf,
    /// The dir, once the first capture has made it.
    dir: Option<PathBuf>,
    captures: VecDeque<Capture>,
    total: u64,
    budget: u64,
    /// The sequence number the next server-chosen file gets.
    next: u32,
}

impl Artifacts {
    /// The artifacts of this process, to be made under
    /// `$XDG_RUNTIME_DIR`, or the system temp dir when that is unset or
    /// empty. Touches nothing on disk until a capture needs the dir.
    pub(crate) fn for_this_process() -> Self {
        let root = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        Self::under(root, BUDGET_BYTES)
    }

    /// Artifacts made under `root`, capped at `budget` bytes.
    pub(crate) fn under(root: PathBuf, budget: u64) -> Self {
        Artifacts {
            root,
            dir: None,
            captures: VecDeque::new(),
            total: 0,
            budget,
            next: 1,
        }
    }

    /// The dir, made on first use.
    pub(crate) fn dir(&mut self) -> Result<&Path, String> {
        if self.dir.is_none() {
            self.dir = Some(self.make_dir()?);
        }
        Ok(self.dir.as_deref().expect("the dir was set just above"))
    }

    fn make_dir(&self) -> Result<PathBuf, String> {
        let base = format!("mandala-{}-artifacts", std::process::id());
        for attempt in 0..NAME_ATTEMPTS {
            let name = match attempt {
                0 => base.clone(),
                n => format!("{base}-{n}"),
            };
            let candidate = self.root.join(name);
            match create_private_dir(&candidate) {
                Ok(()) => return Ok(candidate),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(format!(
                        "could not make the artifacts dir `{}`: {e}",
                        candidate.display()
                    ))
                }
            }
        }
        Err(format!(
            "could not make the artifacts dir under `{}`: {NAME_ATTEMPTS} names were taken",
            self.root.display()
        ))
    }

    /// A fresh path in the dir for a capture: `<stem>-<n>.<extension>`,
    /// the number counting up from 1 and padded to five digits.
    pub(crate) fn next_path(&mut self, stem: &str, extension: &str) -> Result<PathBuf, String> {
        let number = self.next;
        let path = self.dir()?.join(format!("{stem}-{number:05}.{extension}"));
        self.next += 1;
        Ok(path)
    }

    /// A fresh subdirectory of the dir for a recording's frames:
    /// `<stem>-<n>`, numbered like [`Self::next_path`]'s files and made
    /// private to this user the way the dir is. Its bytes count against
    /// the budget once the recording is over ([`Self::record_tree`]).
    pub(crate) fn next_dir(&mut self, stem: &str) -> Result<PathBuf, String> {
        let number = self.next;
        let path = self.dir()?.join(format!("{stem}-{number:05}"));
        self.next += 1;
        create_private_dir(&path)
            .map_err(|e| format!("could not make the recording dir `{}`: {e}", path.display()))?;
        Ok(path)
    }

    /// Whether `path` is in the artifacts dir — where overwriting is
    /// always allowed, because everything there is the app's own. A
    /// path through `..` is not taken to be in it.
    pub(crate) fn holds(&self, path: &Path) -> bool {
        let Some(dir) = self.dir.as_deref() else {
            return false;
        };
        path.parent() == Some(dir) && path.file_name().is_some_and(|name| name != ".." && name != ".")
    }

    /// Record the files one capture wrote into the dir, `(path, bytes)`,
    /// then evict the oldest captures until the dir is back within its
    /// budget. Files outside the dir are not recorded: they are the
    /// client's. A file recorded before — a client overwriting one in
    /// the dir by name — is counted once, at its new size.
    pub(crate) fn record(&mut self, files: &[(PathBuf, u64)]) {
        let ours: Vec<(PathBuf, u64)> = files
            .iter()
            .filter(|(path, _)| self.holds(path))
            .cloned()
            .collect();
        if ours.is_empty() {
            return;
        }
        for (path, _) in &ours {
            self.forget(path);
        }
        self.total += ours.iter().map(|(_, bytes)| bytes).sum::<u64>();
        self.captures.push_back(Capture {
            files: ours,
            tree: None,
        });
        self.evict_to_budget();
    }

    /// Record a finished recording's directory, `bytes` in all, as one
    /// capture: the oldest captures are evicted to make room for it,
    /// and it is the one that goes, whole, when it is the oldest. `dir`
    /// must be one [`Self::next_dir`] made.
    pub(crate) fn record_tree(&mut self, dir: PathBuf, bytes: u64) {
        self.total += bytes;
        self.captures.push_back(Capture {
            files: Vec::new(),
            tree: Some((dir, bytes)),
        });
        self.evict_to_budget();
    }

    /// Evict the oldest captures until the dir is back within its
    /// budget, never the newest.
    fn evict_to_budget(&mut self) {
        while self.total > self.budget && self.captures.len() > 1 {
            let Some(oldest) = self.captures.pop_front() else {
                break;
            };
            for (file, bytes) in &oldest.files {
                self.total -= bytes;
                if let Err(e) = std::fs::remove_file(file) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        log::warn!("capture: could not evict `{}`: {e}", file.display());
                    }
                }
            }
            if let Some((dir, bytes)) = &oldest.tree {
                self.total -= bytes;
                // `remove_dir_all` does not follow a symlink inside it.
                if let Err(e) = std::fs::remove_dir_all(dir) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        log::warn!("capture: could not evict `{}`: {e}", dir.display());
                    }
                }
            }
        }
    }

    /// Drop `path`, and its bytes, from whichever recorded capture
    /// holds it.
    fn forget(&mut self, path: &Path) {
        for capture in &mut self.captures {
            if let Some(at) = capture.files.iter().position(|(file, _)| file == path) {
                let (_, bytes) = capture.files.remove(at);
                self.total -= bytes;
            }
        }
        self.captures
            .retain(|capture| !capture.files.is_empty() || capture.tree.is_some());
    }

    /// Bytes the recorded captures hold.
    #[cfg(test)]
    pub(crate) fn total(&self) -> u64 {
        self.total
    }

    /// Remove the dir and everything in it: the graceful-exit half of
    /// the lifecycle. A no-op when no capture ever made it.
    pub(crate) fn remove(&mut self) {
        if let Some(dir) = self.dir.take() {
            // `remove_dir_all` does not follow a symlink inside the dir.
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                log::warn!(
                    "capture: could not remove the artifacts dir `{}`: {e}",
                    dir.display()
                );
            }
        }
        self.captures.clear();
        self.total = 0;
    }
}

/// `mkdir` at mode `0700`, failing if the name exists.
#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}

/// Off Unix there is no mode to give; `create_dir` still fails on an
/// existing name.
#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::guarded_file::tests::{names_in, scratch_dir};

    /// The budget `format/ipc.md` states is the one the dir keeps.
    #[test]
    fn test_the_documented_budget_is_the_enforced_one() {
        use baumhard::util::doc_fixtures::{repo_path, section_text};
        let section = section_text(
            &repo_path("format/ipc.md"),
            "## `capture` — screenshots and recordings (IPC-07, IPC-08)",
        );
        assert_eq!(BUDGET_BYTES % (1 << 20), 0, "stated in whole MiB");
        let stated = format!("{} MiB", BUDGET_BYTES >> 20);
        assert!(
            section.contains(&stated),
            "format/ipc.md §capture must state {stated}"
        );
    }

    fn write(path: &Path, bytes: usize) -> (PathBuf, u64) {
        std::fs::write(path, vec![0u8; bytes]).expect("write");
        (path.to_path_buf(), bytes as u64)
    }

    /// Nothing is made until a capture asks; the dir is then private to
    /// this user, numbered paths count up inside it, and `remove` takes
    /// it and everything in it away.
    #[test]
    fn test_the_dir_is_made_on_first_use_private_and_removed_whole() {
        let root = scratch_dir("artifacts-life");
        let mut artifacts = Artifacts::under(root.clone(), BUDGET_BYTES);
        assert_eq!(names_in(&root), Vec::<String>::new(), "nothing on disk yet");

        let first = artifacts.next_path("screenshot", "png").expect("a path");
        let second = artifacts.next_path("screenshot", "png").expect("a path");
        let dir = first.parent().expect("in the dir").to_path_buf();
        assert_eq!(first.file_name().expect("named"), "screenshot-00001.png");
        assert_eq!(second.file_name().expect("named"), "screenshot-00002.png");
        assert_eq!(second.parent(), Some(dir.as_path()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).expect("made").permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "owner only");
        }

        let file = write(&first, 10);
        artifacts.record(&[file]);
        artifacts.remove();
        assert!(!dir.exists(), "removed with what was in it");
        artifacts.remove();
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    /// A name someone else already took is never adopted: the dir is
    /// made under the next one.
    #[test]
    fn test_a_taken_name_is_stepped_past_not_adopted() {
        let root = scratch_dir("artifacts-taken");
        let planted = root.join(format!("mandala-{}-artifacts", std::process::id()));
        std::fs::create_dir(&planted).expect("plant");
        std::fs::write(planted.join("bait"), "x").expect("bait");
        let mut artifacts = Artifacts::under(root.clone(), BUDGET_BYTES);
        let dir = artifacts.dir().expect("made").to_path_buf();
        assert_ne!(dir, planted);
        assert!(dir
            .file_name()
            .expect("named")
            .to_string_lossy()
            .ends_with("-artifacts-1"));
        artifacts.remove();
        assert!(
            planted.join("bait").exists(),
            "the planted dir is not ours to remove"
        );
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    /// Over budget, the oldest captures go first — a capture's files
    /// together — and the newest stays even when it alone is over.
    #[test]
    fn test_the_oldest_captures_are_evicted_to_hold_the_budget() {
        let root = scratch_dir("artifacts-budget");
        let mut artifacts = Artifacts::under(root.clone(), 100);
        let capture = |artifacts: &mut Artifacts, bytes: usize| {
            let png = artifacts.next_path("screenshot", "png").expect("a path");
            let json = png.with_extension("png.geometry.json");
            let files = [write(&png, bytes), write(&json, 1)];
            artifacts.record(&files);
            png
        };
        let a = capture(&mut artifacts, 40);
        let b = capture(&mut artifacts, 40);
        assert!(a.exists() && b.exists());
        assert_eq!(artifacts.total(), 82);
        let c = capture(&mut artifacts, 40);
        assert!(!a.exists(), "the oldest went");
        assert!(
            !a.with_extension("png.geometry.json").exists(),
            "with its sidecar"
        );
        assert!(b.exists() && c.exists());
        assert_eq!(artifacts.total(), 82);

        let huge = capture(&mut artifacts, 500);
        assert!(huge.exists(), "the newest is never evicted");
        assert!(!b.exists() && !c.exists());
        assert_eq!(artifacts.total(), 501);
        artifacts.remove();
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    /// Only files in the dir are the app's to count and evict; a file
    /// written there again by name is counted once.
    #[test]
    fn test_only_the_dirs_own_files_are_recorded_and_each_once() {
        let root = scratch_dir("artifacts-holds");
        let mut artifacts = Artifacts::under(root.clone(), 1000);
        let outside = write(&root.join("client.png"), 10);
        assert!(!artifacts.holds(&outside.0), "no dir yet, nothing held");
        let inside = artifacts.next_path("screenshot", "png").expect("a path");
        artifacts.record(std::slice::from_ref(&outside));
        assert_eq!(artifacts.total(), 0, "the client's file is the client's");

        let sidecar = inside.with_extension("png.geometry.json");
        artifacts.record(&[write(&inside, 30), write(&sidecar, 5)]);
        artifacts.record(&[write(&inside, 50)]);
        assert_eq!(artifacts.total(), 55, "rewritten, counted once at its new size");

        let dir = inside.parent().expect("in the dir");
        assert!(artifacts.holds(&dir.join("other.png")));
        assert!(!artifacts.holds(&dir.join("..")));
        assert!(!artifacts.holds(&dir.join("nested").join("x.png")));
        artifacts.remove();
        assert!(outside.0.exists(), "removing the dir leaves the client's file");
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    /// A recording's directory is numbered with the files, private like
    /// the dir, and evicted whole — with everything in it — when it is
    /// the oldest capture over the budget; the newest never is.
    #[test]
    fn test_a_recording_dir_is_one_capture_evicted_whole() {
        let root = scratch_dir("artifacts-tree");
        let mut artifacts = Artifacts::under(root.clone(), 100);
        let shot = artifacts.next_path("screenshot", "png").expect("a path");
        artifacts.record(&[write(&shot, 30)]);
        let recording = |artifacts: &mut Artifacts, frames: usize| {
            let dir = artifacts.next_dir("recording").expect("a dir");
            for n in 0..frames {
                write(&dir.join(format!("frame_{n:05}.png")), 10);
            }
            artifacts.record_tree(dir.clone(), frames as u64 * 10);
            dir
        };
        let first = recording(&mut artifacts, 3);
        assert_eq!(
            first.file_name().expect("named"),
            "recording-00002",
            "numbered with the files"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&first).expect("made").permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "owner only");
        }
        assert!(shot.exists() && first.exists());
        assert_eq!(artifacts.total(), 60);

        let second = recording(&mut artifacts, 5);
        assert!(!shot.exists(), "the oldest capture went first");
        assert!(first.exists() && second.exists());
        assert_eq!(artifacts.total(), 80);

        let third = recording(&mut artifacts, 4);
        assert!(!first.exists(), "a recording goes whole, its frames with it");
        assert!(second.exists() && third.exists());
        assert_eq!(artifacts.total(), 90);

        let huge = recording(&mut artifacts, 30);
        assert!(huge.exists(), "the newest is never evicted");
        assert!(!second.exists() && !third.exists());
        assert_eq!(artifacts.total(), 300);
        artifacts.remove();
        assert!(!huge.exists(), "removed with the dir");
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
