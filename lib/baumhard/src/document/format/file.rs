// SPDX-License-Identifier: MPL-2.0

//! The file a map lives in: its extension, and the one way every
//! surface that has a path reads a map from it and writes one to it.
//!
//! `format::load` and `format::save` are pure: they turn text into
//! a document and back, and touch no file. Every surface that holds a
//! path — the app's `open` and `save`, and `maptool` — wraps them the
//! same way: refuse a path that is not a `.mandala.json` (D19), read the
//! bytes, and on a write go through a temporary file and a rename so a
//! failed write never leaves half a map where a whole one was. That
//! wrapping is here, once, so the surfaces cannot disagree about it.
//!
//! The messages name the path through
//! [`tilde_path`](crate::util::log::tilde_path), because they are shown
//! to the user and pasted into bug reports, and most of the loader's own
//! messages do not name the file.

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

/// Read the map at `path`: refused unless it ends in [`MAP_EXTENSION`],
/// then read whole and loaded by [`load_from_bytes`].
///
/// Errors, each prefixed by the path: the extension refusal, the read's
/// own error, or the loader's [`LoadError`](crate::document::format::LoadError)
/// rendered with its JSON path, its node and its line. In the browser
/// there are no files, and the read fails and says so.
///
/// Cost: the file's bytes, held once, and [`load_from_bytes`]'s.
pub fn read(path: &str) -> Result<Document, String> {
    if !has_map_extension(path) {
        return Err(extension_refusal(path));
    }
    let bytes =
        std::fs::read(path).map_err(|e| format!("{}: could not read: {e}", tilde_path(Path::new(path))))?;
    load_from_bytes(&bytes).map_err(|e| format!("{}: {e}", tilde_path(Path::new(path))))
}

/// Write `text` to the map at `path`: refused unless it ends in
/// [`MAP_EXTENSION`], then written to a temporary file beside it
/// (`<name>.saving`), flushed to the disk, and renamed over `path`, so a
/// write that fails part of the way leaves the file that was there
/// whole. On a failure the temporary file is removed.
///
/// `text` is what [`Document::save_json`] returned; this function does
/// not judge it.
///
/// Cost: one file written and synced, and one rename.
pub fn write(path: &str, text: &str) -> Result<(), String> {
    use std::io::Write;
    if !has_map_extension(path) {
        return Err(extension_refusal(path));
    }
    let target = Path::new(path);
    let shown = || tilde_path(target);
    let dir = target
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = target
        .file_name()
        .ok_or_else(|| format!("{}: not a file name", shown()))?;
    let mut temp = dir.join(name);
    temp.as_mut_os_string().push(".saving");
    let written = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, target)
    })();
    written.map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("{}: could not write: {e}", shown())
    })
}
