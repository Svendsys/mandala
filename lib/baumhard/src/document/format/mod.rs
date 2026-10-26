// SPDX-License-Identifier: MPL-2.0

//! The file format: a [`Document`] as
//! strict, nested, typed JSON — `.mandala.json` (design §7).
//!
//! [`load`] reads a file into a document and
//! [`save`] writes one. The
//! shape, key by key, is `format/schema.md`; this module is its
//! reference implementation, and the two are held to each other by the
//! fixture corpus.
//!
//! **Strict.** Every object denies unknown keys, a wrong type is an
//! error, and a reference — a link's `to`, a verb, a palette, a theme
//! variable, a font — that names nothing is an error. There is no
//! preserved-unknown-keys mechanism and no skipped-construct report: a
//! file is a document or it does not load, and the error is the first
//! thing wrong, with the JSON path of the value
//! ([`LoadError`]).
//!
//! **Typed on read.** Names are resolved to their typed forms while the
//! file is read (`"var(accent)"` to a
//! [`VarId`](crate::document::values::VarId), a palette name to a
//! [`PaletteId`](crate::document::values::PaletteId), a family name to
//! an [`AppFont`](crate::font::fonts::AppFont)); the document never
//! holds a string form. Every number goes through
//! [`bounds`](crate::document::bounds).
//!
//! **Deterministic on write.** An unchanged document saves
//! byte-identically, and a one-field edit is a one-line diff: keys in
//! schema order, name-keyed objects in ascending byte order, nodes in
//! sibling order, numbers in serde_json's shortest form of the stored
//! `f32`, two-space indentation, a trailing newline.
//!
//! **One spelling per value, in the whole file.** The value spellings
//! (a color, a weight, a fit mode, a start point) are the types in
//! `document::format::spelling`, names throughout,
//! never an index: indices are assigned by sorted name, so a saved one
//! would name another entry the moment a name was added. A predicate
//! saved in an edit (V1-D1) is spelled with them too, and the strictness
//! gate keeps every index type out of the types this module reads.
//!
//! The old `.mindmap.json` format is not read here and has no knowledge
//! in this module.

#[macro_use]
mod macros;
pub(crate) mod dto;
pub mod error;
mod from_dto;
mod load_map;
mod load_node;
pub(crate) mod load_style;
pub(crate) mod spelling;
mod strict;
mod to_dto;

pub use error::{LoadError, LoadErrorKind};

use crate::document::format::dto::MapDto;
use crate::document::ids::Document;

/// Ceiling on the length of a map's text, in bytes, or `None` where the
/// platform imposes none.
///
/// **`None` on native, and that is the design rather than an
/// omission.** Mandala is meant to open maps as large as the machine
/// can hold. The user picked the file; a 10 GB map costing 10 GB is the
/// feature, and a failed allocation is an honest outcome of a choice
/// they made. A limit defended by the size of today's fixtures is a
/// limit that expires.
///
/// **`Some` on wasm32, because there the limit is physics.** `usize` is
/// 32 bits and the address space is 4 GiB, shared with the runtime, the
/// GPU buffers and the browser itself. A file that cannot fit is
/// refused with a message rather than trapping mid-parse. This is a
/// sanctioned target divergence, registered in `CLAUDE.md`'s dual-target
/// section.
///
/// What is *not* bounded by any of this is the ratio: peak memory is a
/// multiple of the text, the parsed DTOs and the document being alive
/// together. Predicting cost from file size is what a user needs, and a
/// ceiling never provided it.
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_MAP_BYTES: Option<u64> = None;

/// See the native definition above for the rationale. 3 GiB leaves
/// headroom inside wasm32's 4 GiB address space for the typed model the
/// text is parsed into, the glyph trees built from it, and the GPU-side
/// buffers — none of which the text's length counts.
#[cfg(target_arch = "wasm32")]
pub const MAX_MAP_BYTES: Option<u64> = Some(3 * 1024 * 1024 * 1024);

/// Whether a `size` is points or a fraction of the node's shorter side.
/// Which one it is depends on where it sits, not on how it is written:
/// inside a frame (the frame, its rings, `defaults.frame`,
/// `defaults.ring`) it is a fraction, everywhere else points. The
/// loader stores the unit it reads a number as, and the saver writes a
/// size only where its unit is the one the position takes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum SizeUnit {
    /// Points.
    Pt,
    /// A fraction of the node's shorter side.
    Rel,
}

/// [`load`] for bytes: the entry point for input that is not known to
/// be text (a file read, a socket, a fuzzer's corpus). The text is
/// checked against [`MAX_MAP_BYTES`] before it is looked at, and then
/// must be UTF-8; bytes that are not are a
/// [`Syntax`](LoadErrorKind::Syntax) error naming the offset.
///
/// Pure and in-memory: it reads no file, so a caller that reads one
/// wraps this. It never panics, on any input: every refusal is a
/// [`LoadError`], and nothing is allocated in proportion to the input
/// before the size ceiling and the reader's depth limit have been
/// applied to it.
pub fn load_from_bytes(bytes: &[u8]) -> Result<Document, LoadError> {
    load_bytes_under(bytes, MAX_MAP_BYTES)
}

/// [`load_from_bytes`] against a caller-supplied ceiling.
pub(crate) fn load_bytes_under(bytes: &[u8], ceiling: Option<u64>) -> Result<Document, LoadError> {
    let length = bytes.len() as u64;
    if let Some(max) = ceiling.filter(|max| length > *max) {
        return Err(LoadError {
            path: String::new(),
            kind: LoadErrorKind::TooLarge { bytes: length, max },
        });
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => load_under(text, ceiling),
        Err(error) => Err(LoadError {
            path: String::new(),
            kind: LoadErrorKind::Syntax {
                message: format!("the text is not UTF-8: {error}"),
            },
        }),
    }
}

/// Read a `.mandala.json` into a document.
///
/// Refuses the text outright if it is longer than [`MAX_MAP_BYTES`]
/// (checked before anything is parsed, on every transport: the browser
/// receives its map as a string and never touches the filesystem path).
/// Otherwise one pass parses the text into the DTOs, one pass builds the
/// tree — resolving names, checking every number against
/// [`bounds`](crate::document::bounds) and every element against the
/// shape rules as it goes — and links are resolved last. The first
/// error is returned and nothing else: no partial document.
///
/// Cost: O(text) for the parse, then O(elements) with one hash insert
/// per node. The tree is built in key order, so no insert scans a row.
pub fn load(json: &str) -> Result<Document, LoadError> {
    load_under(json, MAX_MAP_BYTES)
}

/// [`load`] against a caller-supplied ceiling.
///
/// Split out so the refusal stays testable. [`MAX_MAP_BYTES`] is `None`
/// on every target the test suite runs on, so a test written against
/// the constant would assert nothing and pass — and the arm it stopped
/// covering is the one the browser depends on. This takes the ceiling
/// as a parameter, so a native test can hand it a small one and drive
/// the same code the wasm32 build reaches with a real one.
pub(crate) fn load_under(json: &str, ceiling: Option<u64>) -> Result<Document, LoadError> {
    check_text_cap_against(json, ceiling)?;
    let dto = parse(json)?;
    from_dto::build(&dto).map(|(document, _)| document)
}

/// [`load`], and how many elements of the document had their row
/// checked against the shape rules: every element, which a test asserts.
#[cfg(test)]
pub(crate) fn load_counting_checks(json: &str) -> Result<(Document, usize), LoadError> {
    check_text_cap_against(json, MAX_MAP_BYTES)?;
    from_dto::build(&parse(json)?)
}

/// Refuse `json` if it is longer than `ceiling`. O(1).
pub(crate) fn check_text_cap_against(json: &str, ceiling: Option<u64>) -> Result<(), LoadError> {
    let bytes = json.len() as u64;
    match ceiling.filter(|max| bytes > *max) {
        Some(max) => Err(LoadError {
            path: String::new(),
            kind: LoadErrorKind::TooLarge { bytes, max },
        }),
        None => Ok(()),
    }
}

/// Write a document as a `.mandala.json`: deterministic, two-space
/// indented, ending in a newline.
///
/// Nothing derived is written, and what the file cannot spell is left
/// out with a warning (see the module docs of the saver). Cost: one walk
/// of the tree to build the DTOs, then serde_json's writer; O(elements).
pub fn save(doc: &Document) -> String {
    let dto = to_dto::dtos_of(doc);
    let mut text = match serde_json::to_string_pretty(&dto) {
        Ok(text) => text,
        Err(error) => {
            // The DTOs hold strings, numbers and maps with string keys:
            // nothing here can fail to serialize.
            log::error!("save: the document could not be written: {error}");
            String::new()
        }
    };
    text.push('\n');
    text
}

/// Parse `json` into the DTOs, naming the path of whatever is wrong.
fn parse(json: &str) -> Result<MapDto, LoadError> {
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let parsed: Result<MapDto, serde_path_to_error::Error<serde_json::Error>> =
        serde_path_to_error::deserialize(strict::Objects(&mut deserializer));
    match parsed {
        Ok(dto) => match deserializer.end() {
            Ok(()) => Ok(dto),
            Err(error) => Err(LoadError {
                path: String::new(),
                kind: LoadErrorKind::Syntax {
                    message: error.to_string(),
                },
            }),
        },
        Err(error) => Err(refusal(json, error)),
    }
}

/// The [`LoadError`] for a parse that failed.
///
/// The reader's messages are the only place the kind of a data error is
/// written, so the kinds that name a key are read back out of them; the
/// tests pin every message shape this depends on, and a message that is
/// not recognized is an [`Invalid`](LoadErrorKind::Invalid) with the
/// reader's text, which still carries the path.
fn refusal(json: &str, error: serde_path_to_error::Error<serde_json::Error>) -> LoadError {
    use serde_json::error::Category;
    let path = error.path().to_string();
    let path = if path == "." { String::new() } else { path };
    let inner = error.into_inner();
    let message = inner.to_string();

    if matches!(inner.classify(), Category::Syntax | Category::Eof | Category::Io) {
        // Malformed text: the message says where, by line and column,
        // and the path is the last value the reader entered, which is
        // where it gave up. It has none to give when it died on a key.
        let path = if path.contains('?') { String::new() } else { path };
        return LoadError {
            path,
            kind: LoadErrorKind::Syntax { message },
        };
    }

    // A file with a `format` other than 1 is a file this build does not
    // read, whatever else is wrong with it: a later format will have
    // keys this one does not know, and "unknown key" would be the wrong
    // thing to say about it.
    if let Some(found) = format_number(json).filter(|found| *found != 1) {
        return LoadError {
            path: "format".to_string(),
            kind: LoadErrorKind::UnsupportedFormat { found },
        };
    }

    let quoted = |prefix: &str| {
        message
            .strip_prefix(prefix)
            .and_then(|rest| rest.split('`').next())
            .map(str::to_string)
    };
    if let Some(key) = quoted("unknown field `") {
        // The reader has already entered the key: the path ends in it.
        return LoadError {
            path,
            kind: LoadErrorKind::UnknownKey { key },
        };
    }
    if let Some(key) = quoted("missing field `") {
        return LoadError {
            path,
            kind: LoadErrorKind::MissingKey { key },
        };
    }
    if let Some(key) = quoted("duplicate field `") {
        return LoadError {
            path,
            kind: LoadErrorKind::DuplicateKey { key },
        };
    }
    LoadError {
        path,
        kind: LoadErrorKind::Invalid { message },
    }
}

/// The number a file gives as its top-level `format`, if it gives one.
/// Only read for a file that failed to parse as the current format, so
/// its cost is the failure's.
fn format_number(json: &str) -> Option<u64> {
    #[derive(serde::Deserialize)]
    struct Peek {
        format: Option<u64>,
    }
    // `Peek` has no `deny_unknown_fields`, so everything else in the
    // file is skipped; a file that is not an object at all has none.
    serde_json::from_str::<Peek>(json)
        .ok()
        .and_then(|peek| peek.format)
}
