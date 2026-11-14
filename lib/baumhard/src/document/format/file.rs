// SPDX-License-Identifier: MPL-2.0

//! The file a map lives in: its extension, and the one way every
//! surface that has a path reads a map from it and writes one to it.
//!
//! `format::load` and `format::save` are pure: they turn text into a
//! document and back, and touch no file. Every surface that holds a path
//! — the app's `open` and `save`, and `maptool` — wraps them the same
//! way: refuse a path that is not a `.mandala.json` (D19), read the
//! bytes, and write through
//! [`guarded_file::publish`](crate::util::guarded_file::publish), the
//! workspace's one atomic writer. That wrapping is here, once, so the
//! surfaces cannot disagree about it.
//!
//! **Errors are `String`s, ready to show.** CODE_CONVENTIONS §9 admits
//! no new error type without a project-scale decision; every caller of
//! this module shows the message and does nothing else with it; and what
//! a caller could branch on is already typed one level down, in the
//! `LoadError` and the
//! [`PublishRefusal`](crate::util::guarded_file::PublishRefusal) whose
//! words the messages carry. Each message names the path through
//! [`tilde_path`](crate::util::log::tilde_path) or
//! [`collapse_home_in`](crate::util::log::collapse_home_in), because
//! it is shown to the user and pasted into bug reports, and most of the
//! loader's own messages do not name the file.

use std::path::Path;

use crate::document::format::load_from_bytes;
use crate::document::ids::Document;
use crate::util::log::tilde_path;

/// The one file extension a map is read from and written to (D19), so a
/// file in an older format is never mistaken for one in this one.
pub const MAP_EXTENSION: &str = ".mandala.json";

/// Whether `path` names a map: it ends in [`MAP_EXTENSION`]. A path that
/// does not is refused by every surface that opens or saves one, with
/// [`extension_refusal`]'s words. O(1).
pub fn has_map_extension(path: &str) -> bool {
    path.ends_with(MAP_EXTENSION)
}

/// The message a path without [`MAP_EXTENSION`] is refused with: it
/// names the path and the extension it wanted. Cost: one `String`.
pub fn extension_refusal(path: &str) -> String {
    format!(
        "{}: not a map; a map's file name ends in `{MAP_EXTENSION}`",
        tilde_path(Path::new(path))
    )
}

/// The bytes of the map at `path`: refused unless it ends in
/// [`MAP_EXTENSION`], then read whole. For a caller that needs the file
/// as it is on disk as well as the document [`load`] makes of it.
///
/// Errors, each prefixed by the path: the extension refusal, or the
/// read's own error. In the browser there are no files, and the read
/// fails and says so.
///
/// Cost: the file's bytes, held once.
pub fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
    if !has_map_extension(path) {
        return Err(extension_refusal(path));
    }
    std::fs::read(path).map_err(|e| format!("{}: could not read: {e}", tilde_path(Path::new(path))))
}

/// The document `bytes`, read from `path`, make: [`load_from_bytes`],
/// with the loader's [`LoadError`](crate::document::format::LoadError)
/// — its JSON path, its node and its line — behind the file's path.
///
/// Cost: [`load_from_bytes`]'s.
pub fn load(path: &str, bytes: &[u8]) -> Result<Document, String> {
    load_from_bytes(bytes).map_err(|e| format!("{}: {e}", tilde_path(Path::new(path))))
}

/// Read the map at `path`: [`read_bytes`], then [`load`].
///
/// Cost: theirs.
pub fn read(path: &str) -> Result<Document, String> {
    load(path, &read_bytes(path)?)
}

/// Write `text` to the map at `path`: refused unless it ends in
/// [`MAP_EXTENSION`], then published by
/// [`guarded_file::publish`](crate::util::guarded_file::publish), with
/// the file it replaces. So the bytes go to a temp file unique to this
/// writer — two processes saving one map never share one — created
/// exclusively and without following a symlink, are flushed and synced,
/// and only then are renamed over `path`; a write that fails part of the
/// way leaves the file that was there whole, and removes its temp file.
/// A `path` that is itself a symlink is refused, never written through
/// or replaced.
///
/// `text` is what [`Document::save_json`] returned; this function does
/// not judge it. In the browser there are no files, and the write fails
/// and says so.
///
/// Cost: [`guarded_file::publish`](crate::util::guarded_file::publish)'s:
/// the text written once, an `fsync` and a rename.
pub fn write(path: &str, text: &str) -> Result<(), String> {
    if !has_map_extension(path) {
        return Err(extension_refusal(path));
    }
    publish(path, |out| {
        out.write_all(text.as_bytes()).map_err(|e| e.to_string())
    })
}

/// [`write()`]'s publication: what `produce` writes, published over
/// `path` through the guarded writer (which, in the browser, refuses).
fn publish(
    path: &str,
    produce: impl FnOnce(&mut dyn std::io::Write) -> Result<(), String>,
) -> Result<(), String> {
    use crate::util::guarded_file::{publish, PublishRefusal};
    publish(Path::new(path), true, produce)
        .map(|_| ())
        .map_err(|refusal| {
            let (PublishRefusal::Invalid(message) | PublishRefusal::Internal(message)) = refusal;
            // The caller names its own area (`save:`, `apply:`); the
            // message leads with the path, as a read's does.
            format!(
                "{}: could not write: {}",
                tilde_path(Path::new(path)),
                crate::util::log::collapse_home_in(&message)
            )
        })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::util::test_temp::{names_in, scratch_dir};

    /// A path that is not a `.mandala.json` is refused by `read` and
    /// `write` alike, naming the extension, and `write` creates nothing.
    ///
    /// Control: dropping `write`'s extension check writes `map.json`, and
    /// the directory is no longer empty.
    #[test]
    fn test_file_refuses_a_path_without_the_map_extension() {
        let dir = scratch_dir("file-extension");
        let path = dir.join("map.json");
        let path = path.to_str().unwrap();
        let refused = write(path, "{}").unwrap_err();
        assert!(
            refused.ends_with("map.json: not a map; a map's file name ends in `.mandala.json`"),
            "{refused}"
        );
        assert_eq!(names_in(&dir), Vec::<String>::new());
        std::fs::write(path, r#"{"format": 1, "name": "m"}"#).unwrap();
        assert!(read(path).unwrap_err().contains("not a map"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A write replaces the map whole and leaves nothing beside it.
    ///
    /// Control: publishing by a copy that never removes the temp file
    /// leaves `.mandala-write-*.tmp` beside the map, and the second
    /// assertion goes red.
    #[test]
    fn test_file_write_replaces_the_map_and_leaves_no_temp_file() {
        let dir = scratch_dir("file-write");
        let path = dir.join("m.mandala.json");
        std::fs::write(&path, "old").unwrap();
        write(path.to_str().unwrap(), "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(names_in(&dir), ["m.mandala.json"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A write that fails part of the way — its producer has written
    /// some bytes, then the disk is full, in miniature — leaves the old
    /// bytes intact and no temp file behind, and says why.
    ///
    /// Control: publishing by writing the target directly
    /// (`File::create(path)`, then the producer) truncates it first, and
    /// the second assertion goes red.
    #[test]
    fn test_file_a_failed_write_leaves_the_old_bytes_and_no_temp_file() {
        let dir = scratch_dir("file-failed");
        let path = dir.join("m.mandala.json");
        std::fs::write(&path, "old").unwrap();
        let failed = publish(path.to_str().unwrap(), |out| {
            out.write_all(b"half of a ma").map_err(|e| e.to_string())?;
            Err("the disk is full".to_string())
        })
        .unwrap_err();
        assert!(failed.contains("the disk is full"), "{failed}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names_in(&dir), ["m.mandala.json"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A symlink at the map's path is refused and what it points at is
    /// untouched: the map is never written through a link, nor the link
    /// replaced by a file.
    ///
    /// Control: the earlier writer (`File::create` on `<name>.saving`,
    /// then a rename) replaced the link with a regular file, and the
    /// first assertion goes red.
    #[cfg(unix)]
    #[test]
    fn test_file_write_refuses_a_symlinked_map() {
        let dir = scratch_dir("file-symlink");
        let target = dir.join("real.mandala.json");
        std::fs::write(&target, "precious").unwrap();
        let link = dir.join("m.mandala.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let refused = write(link.to_str().unwrap(), "new").unwrap_err();
        assert!(refused.contains("is a symlink"), "{refused}");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "precious");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two writers never share a temp file: a file already standing at
    /// the name the earlier writer used (`<name>.saving`), as another
    /// process's half-written save would, is neither written into nor
    /// published, and a write still succeeds beside it.
    ///
    /// Control: the earlier writer `File::create`s `<name>.saving`,
    /// truncating the other writer's bytes, and the second assertion goes
    /// red.
    #[test]
    fn test_file_two_writers_do_not_share_a_temp_file() {
        let dir = scratch_dir("file-two-writers");
        let path = dir.join("m.mandala.json");
        let other = dir.join("m.mandala.json.saving");
        std::fs::write(&other, "another writer's half").unwrap();
        write(path.to_str().unwrap(), "mine").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "another writer's half");
        assert_eq!(names_in(&dir), ["m.mandala.json", "m.mandala.json.saving"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
