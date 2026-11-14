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

/// The real file of the map at `path`: refused unless `path` ends in
/// [`MAP_EXTENSION`]; then, when it is a symlink, resolved to the file at
/// the end of its chain
/// ([`real_file`](crate::util::guarded_file::real_file)), which must be
/// named like a map too (amendment E1.1). The one rule both a read and a
/// write hold to: the extension is checked on the file used, not only on
/// the name typed. Errors name the path, and for a link both paths.
///
/// Cost: a `symlink_metadata`, and for a link `real_file`'s walk.
fn real_map(path: &str) -> Result<std::path::PathBuf, String> {
    use crate::util::guarded_file::{real_file, shown_path, PublishRefusal};
    if !has_map_extension(path) {
        return Err(extension_refusal(path));
    }
    let shown = || tilde_path(Path::new(path));
    let real = real_file(Path::new(path)).map_err(|refusal| {
        let (PublishRefusal::Invalid(message) | PublishRefusal::Internal(message)) = refusal;
        format!("{}: {}", shown(), crate::util::log::collapse_home_in(&message))
    })?;
    if !has_map_extension(&real.to_string_lossy()) {
        return Err(format!(
            "{}: links to `{}`, which is not a map; a map's file name ends in `{MAP_EXTENSION}`",
            shown(),
            crate::util::log::collapse_home_in(&shown_path(&real))
        ));
    }
    Ok(real)
}

/// The bytes of the map at `path`: its real file (`real_map`), read whole. For
/// a caller that needs the file as it is on disk as well as the document
/// [`load`] makes of it.
///
/// Errors, each prefixed by the path: the extension refusal (of the
/// name typed, or of the file a link leads to), a link that cannot be
/// followed, or the read's own error. In the browser there are no
/// files, and the read fails and says so.
///
/// Cost: the file's bytes, held once.
pub fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
    let real = real_map(path)?;
    std::fs::read(real).map_err(|e| format!("{}: could not read: {e}", tilde_path(Path::new(path))))
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
/// A `path` that is a symlink is saved *through*: the write goes beside
/// the file at the end of its chain of links
/// ([`real_file`](crate::util::guarded_file::real_file)), which it
/// replaces, and the link stays a link. That real file must be a map
/// too — its name ends in [`MAP_EXTENSION`] — or nothing is written and
/// the refusal names both paths (amendment E1.1): the save is reachable
/// over IPC, and a link named like a map must not carry map bytes into
/// another file. A link that leads nowhere, loops, or ends at something
/// that is not a regular file is refused, naming both.
///
/// `text` is what [`Document::save_json`] returned; this function does
/// not judge it. In the browser there are no files, and the write fails
/// and says so.
///
/// Cost: [`guarded_file::publish`](crate::util::guarded_file::publish)'s:
/// the text written once, an `fsync` and a rename.
pub fn write(path: &str, text: &str) -> Result<(), String> {
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
    // The extension rule holds for the file written, not only for the
    // name typed (amendment E1.1): a link named like a map cannot carry
    // map bytes into a file that is not one.
    let real = real_map(path)?;
    publish(&real, true, produce).map(|_| ()).map_err(|refusal| {
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

    /// A map kept behind a symlink saves through it: the target's bytes
    /// are replaced, the link is still a link to the same target, and
    /// nothing is left beside either.
    ///
    /// Control: publishing at the path as given (no `real_file`) refuses
    /// the symlink, and the write's `unwrap` goes red; the earlier
    /// writer replaced the link with a regular file, and the link
    /// assertion goes red.
    #[cfg(unix)]
    #[test]
    fn test_file_write_saves_through_a_symlinked_map() {
        let dir = scratch_dir("file-symlink");
        let target = dir.join("real.mandala.json");
        std::fs::write(&target, "old").unwrap();
        let link = dir.join("m.mandala.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write(link.to_str().unwrap(), "new").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_link(&link).unwrap(), target);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(names_in(&dir), ["m.mandala.json", "real.mandala.json"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A chain of two links saves through to the file at its end, and
    /// both links stay links.
    ///
    /// Control: resolving one hop (`read_link`) in place of the chain
    /// publishes over the middle link, which is refused, and the write's
    /// `unwrap` goes red.
    #[cfg(unix)]
    #[test]
    fn test_file_write_saves_through_a_chain_of_links() {
        let dir = scratch_dir("file-chain");
        let target = dir.join("real.mandala.json");
        std::fs::write(&target, "old").unwrap();
        let middle = dir.join("middle.mandala.json");
        std::os::unix::fs::symlink(&target, &middle).unwrap();
        let link = dir.join("m.mandala.json");
        std::os::unix::fs::symlink(&middle, &link).unwrap();
        write(link.to_str().unwrap(), "new").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        for name in [&link, &middle] {
            assert!(std::fs::symlink_metadata(name).unwrap().file_type().is_symlink());
        }
        assert_eq!(
            names_in(&dir),
            ["m.mandala.json", "middle.mandala.json", "real.mandala.json"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A link that leads nowhere is refused, naming the link and where it
    /// leads, and nothing is created — neither its target nor a file in
    /// its place.
    ///
    /// Control: returning the canonical-or-given path for a dangling
    /// link publishes at the link, which the guarded writer refuses as a
    /// symlink, so the message assertion goes red.
    #[cfg(unix)]
    #[test]
    fn test_file_write_refuses_a_dangling_link() {
        let dir = scratch_dir("file-dangling");
        let link = dir.join("m.mandala.json");
        std::os::unix::fs::symlink(dir.join("gone.mandala.json"), &link).unwrap();
        let refused = write(link.to_str().unwrap(), "new").unwrap_err();
        assert!(refused.contains("is a symlink to"), "{refused}");
        assert!(
            refused.contains("gone.mandala.json`, which does not exist"),
            "{refused}"
        );
        assert_eq!(names_in(&dir), ["m.mandala.json"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two writes never use one temp file: each sees a temp file of its
    /// own beside the map while it writes, under another name.
    ///
    /// Control: fixing the temp name to a constant (no pid, no sequence)
    /// gives both writes the same name, and the inequality goes red.
    #[test]
    fn test_file_two_writes_use_two_temp_names() {
        let dir = scratch_dir("file-two-writes");
        let path = dir.join("m.mandala.json");
        let path_str = path.to_str().unwrap();
        let mut seen = Vec::new();
        for text in ["first", "second"] {
            publish(path_str, |out| {
                let temps: Vec<String> = names_in(&dir)
                    .into_iter()
                    .filter(|name| name.starts_with(".mandala-write-"))
                    .collect();
                assert_eq!(
                    temps.len(),
                    1,
                    "precondition: one temp file while writing: {temps:?}"
                );
                seen.extend(temps);
                out.write_all(text.as_bytes()).map_err(|e| e.to_string())
            })
            .unwrap();
        }
        assert_eq!(seen.len(), 2);
        assert_ne!(seen[0], seen[1], "each write has a temp file of its own");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A temp file a crashed writer left behind — at the name a constant
    /// would give, `.mandala-write-0-0.tmp` — neither blocks a later save
    /// nor is touched by it, and the save leaves no temp file of its own.
    ///
    /// Control: fixing the temp name to that constant makes the exclusive
    /// create fail on the leftover, and the save's `unwrap` goes red.
    #[test]
    fn test_file_a_leftover_temp_file_neither_blocks_nor_is_touched() {
        let dir = scratch_dir("file-leftover");
        let path = dir.join("m.mandala.json");
        let leftover = dir.join(".mandala-write-0-0.tmp");
        std::fs::write(&leftover, "a crashed writer's half").unwrap();
        write(path.to_str().unwrap(), "mine").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
        assert_eq!(
            std::fs::read_to_string(&leftover).unwrap(),
            "a crashed writer's half"
        );
        assert_eq!(names_in(&dir), [".mandala-write-0-0.tmp", "m.mandala.json"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A link named like a map that leads to a file that is not one is
    /// refused, naming both, and the file it leads to is not written
    /// (amendment E1.1: the extension rule holds for the file written).
    ///
    /// Control: checking only the name typed lets the write through, and
    /// `notamap.txt` holds the map's bytes.
    #[cfg(unix)]
    #[test]
    fn test_file_write_refuses_a_link_to_a_file_that_is_not_a_map() {
        let dir = scratch_dir("file-not-a-map");
        let other = dir.join("notamap.txt");
        std::fs::write(&other, "notes").unwrap();
        let link = dir.join("sneaky.mandala.json");
        std::os::unix::fs::symlink(&other, &link).unwrap();
        let refused = write(link.to_str().unwrap(), "map bytes").unwrap_err();
        assert!(refused.contains("sneaky.mandala.json: links to `"), "{refused}");
        assert!(refused.contains("notamap.txt`, which is not a map"), "{refused}");
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "notes");
        assert_eq!(names_in(&dir), ["notamap.txt", "sneaky.mandala.json"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A link to a directory is refused by `real_file` in its own words,
    /// naming the link, where it leads and what is there.
    ///
    /// Control: accepting any target leaves the refusal to `publish`,
    /// whose message ("is a directory") is not this one.
    #[cfg(unix)]
    #[test]
    fn test_file_write_refuses_a_link_to_a_directory_naming_both() {
        let dir = scratch_dir("file-link-dir");
        let target = dir.join("real.mandala.json");
        std::fs::create_dir(&target).unwrap();
        let link = dir.join("m.mandala.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let refused = write(link.to_str().unwrap(), "new").unwrap_err();
        assert!(refused.contains("m.mandala.json` is a symlink to `"), "{refused}");
        assert!(
            refused.contains("real.mandala.json`, a directory, not a regular file"),
            "{refused}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A loop of links says it is a loop, not that its target does not
    /// exist, and nothing is created.
    ///
    /// Control: the earlier message ("does not exist" whatever the error)
    /// fails the first assertion.
    #[cfg(unix)]
    #[test]
    fn test_file_write_names_a_loop_of_links() {
        let dir = scratch_dir("file-loop");
        let a = dir.join("a.mandala.json");
        let b = dir.join("b.mandala.json");
        std::os::unix::fs::symlink(&b, &a).unwrap();
        std::os::unix::fs::symlink(&a, &b).unwrap();
        let refused = write(a.to_str().unwrap(), "new").unwrap_err();
        assert!(refused.contains("which cannot be resolved: "), "{refused}");
        assert!(!refused.contains("does not exist"), "{refused}");
        assert_eq!(names_in(&dir), ["a.mandala.json", "b.mandala.json"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A read holds to the same rule as a write: through a link to a map
    /// it reads the map, and through a link named like a map to a file
    /// that is not one it is refused, naming both.
    ///
    /// Control: checking the name typed alone reads `notamap.txt`, whose
    /// text is a valid map, and the refusal assertion goes red.
    #[cfg(unix)]
    #[test]
    fn test_file_read_holds_the_extension_rule_to_the_file_read() {
        let dir = scratch_dir("file-read-link");
        let map = r#"{"format": 1, "name": "m"}"#;
        let real = dir.join("real.mandala.json");
        std::fs::write(&real, map).unwrap();
        let to_map = dir.join("link.mandala.json");
        std::os::unix::fs::symlink(&real, &to_map).unwrap();
        assert_eq!(read(to_map.to_str().unwrap()).unwrap().name(), "m");
        let other = dir.join("notamap.txt");
        std::fs::write(&other, map).unwrap();
        let sneaky = dir.join("sneaky.mandala.json");
        std::os::unix::fs::symlink(&other, &sneaky).unwrap();
        let refused = read(sneaky.to_str().unwrap()).unwrap_err();
        assert!(refused.contains("sneaky.mandala.json: links to `"), "{refused}");
        assert!(refused.contains("notamap.txt`, which is not a map"), "{refused}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
