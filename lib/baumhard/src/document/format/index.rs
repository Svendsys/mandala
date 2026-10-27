// SPDX-License-Identifier: MPL-2.0

//! Phase 2 of a load (design B.14 steps 4–9): one parse of the whole
//! text that finds each top-level value, and each node, without
//! building anything.
//!
//! **Each value is captured as an extent**: a `&RawValue` borrowed from
//! the text, which is zero-copy. Capturing one checks its JSON syntax,
//! and serde_json skips a nested value iteratively, so a hostile nesting
//! costs memory proportional to the text and never stack. `nodes` is
//! captured as one extent per element. What this phase holds beside the
//! text is one fat pointer per node and nine per map.
//!
//! **Why index rather than stream.** A loader that built each node as
//! the reader reached it would need the theme, palettes and verbs to
//! come before `nodes` in the text, so the order of the keys would
//! matter: a file run through `jq -S` puts `nodes` first and would no
//! longer load. Indexing reads the node text twice, once here and once
//! to parse it, which is still O(text); in return the order of keys
//! never matters, every node has an exact place in the text, each node
//! is parsed on its own (which is what bounds the reader's nesting
//! count per node, design B.7), and a syntax error anywhere is reported
//! before any semantic one, so a truncated file says it is truncated.
//!
//! Then `format` is read and checked before any other key, wherever it
//! sits in the object, and a key the format does not define is refused
//! only once `format` has said this build reads the file: a newer
//! file's keys may legitimately be unknown here.

use crate::document::format::context::{offset_in, position_at, within};
use crate::document::format::error::{
    push_parse_path, render_parse_path, LoadError, LoadErrorKind, Seg, TextPosition,
};
use crate::document::format::rows;
use crate::document::format::strict;
use serde::de::{self, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::fmt;

/// The newest `format` this build reads, and the one it writes.
pub const FORMAT: u64 = 1;

/// The top-level keys of a file, in the order the saver writes them
/// (design B.3).
pub(crate) const TOP_LEVEL_KEYS: [&str; 9] = [
    "format",
    "name",
    "theme",
    "palettes",
    "defaults",
    "edits",
    "presentations",
    "verbs",
    "nodes",
];

/// A text, indexed: the extent of each top-level value, and of each
/// node.
pub(crate) struct MapIndex<'t> {
    /// The whole text, which every extent is a slice of.
    pub text: &'t str,
    /// The extent of each top-level key's value but `nodes`, in
    /// [`TOP_LEVEL_KEYS`] order; `None` for a key the text leaves out.
    pub values: [Option<&'t RawValue>; 8],
    /// The extent of each node, in list order.
    pub nodes: Vec<&'t RawValue>,
}

impl<'t> MapIndex<'t> {
    /// The extent of the top-level `key`'s value, if the text has one.
    pub fn value(&self, key: &str) -> Option<&'t RawValue> {
        TOP_LEVEL_KEYS
            .iter()
            .position(|known| *known == key)
            .and_then(|slot| self.values.get(slot).copied().flatten())
    }
}

/// What the top-level visitor has found so far, and what stopped it.
#[derive(Default)]
struct Found<'t> {
    values: [Option<&'t RawValue>; 8],
    nodes: Option<Vec<&'t RawValue>>,
    /// The first key the format does not define.
    unknown: Option<String>,
    /// Every key the format does not define, so that one written twice
    /// is found.
    unknown_seen: crate::document::keyed::KeyedSet<String>,
    /// The key whose value the reader is inside, for the path of an
    /// error it raises there.
    at: Option<String>,
    /// The node the reader is inside, when `at` is `nodes`.
    node: Option<usize>,
    /// A key written twice: what stopped the visitor, if anything did.
    duplicate: Option<String>,
}

/// The top-level object, read key by key into [`Found`].
struct TopLevel<'a, 't>(&'a mut Found<'t>);

impl<'de: 't, 't> DeserializeSeed<'de> for TopLevel<'_, 't> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de: 't, 't> Visitor<'de> for TopLevel<'_, 't> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an object, the map")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let found = self.0;
        while let Some(key) = map.next_key::<String>()? {
            let slot = TOP_LEVEL_KEYS.iter().position(|known| *known == key);
            let repeated = match slot {
                Some(8) => found.nodes.is_some(),
                Some(slot) => found.values.get(slot).is_some_and(Option::is_some),
                None => found.unknown_seen.contains(&key),
            };
            if repeated {
                found.duplicate = Some(key);
                return Err(de::Error::custom("a key appears twice"));
            }
            match slot {
                Some(8) => map.next_value_seed(Nodes(&mut *found))?,
                Some(slot) => map.next_value_seed(ValueExtent {
                    found: &mut *found,
                    slot,
                    key,
                })?,
                None => map.next_value_seed(Skip {
                    found: &mut *found,
                    key,
                })?,
            }
        }
        Ok(())
    }
}

/// A known top-level key's value, captured as its extent into `slot`.
/// The reader has read the `:` when it hands the value over, so a syntax
/// error from here on is inside the value, and its path is the key.
struct ValueExtent<'a, 't> {
    found: &'a mut Found<'t>,
    slot: usize,
    key: String,
}

impl<'de: 't, 't> DeserializeSeed<'de> for ValueExtent<'_, 't> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        self.found.at = Some(self.key);
        let value = <&'de RawValue>::deserialize(deserializer)?;
        if let Some(entry) = self.found.values.get_mut(self.slot) {
            *entry = Some(value);
        }
        self.found.at = None;
        Ok(())
    }
}

/// A top-level key the format does not define: its value is skipped,
/// and the key remembered for the refusal that follows `format`.
struct Skip<'a, 't> {
    found: &'a mut Found<'t>,
    key: String,
}

impl<'de: 't, 't> DeserializeSeed<'de> for Skip<'_, 't> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        self.found.at = Some(self.key.clone());
        IgnoredAny::deserialize(deserializer)?;
        self.found.at = None;
        if self.found.unknown.is_none() {
            self.found.unknown = Some(self.key.clone());
        }
        self.found.unknown_seen.insert(self.key);
        Ok(())
    }
}

/// The `nodes` array, read element by element as extents.
struct Nodes<'a, 't>(&'a mut Found<'t>);

impl<'de: 't, 't> DeserializeSeed<'de> for Nodes<'_, 't> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        self.0.at = Some("nodes".to_string());
        self.0.nodes = Some(Vec::new());
        deserializer.deserialize_seq(self)
    }
}

impl<'de: 't, 't> Visitor<'de> for Nodes<'_, 't> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of nodes")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let found = self.0;
        let mut nodes = found.nodes.take().unwrap_or_default();
        loop {
            found.node = Some(nodes.len());
            match seq.next_element::<&'de RawValue>() {
                Ok(Some(node)) => nodes.push(node),
                Ok(None) => break,
                Err(error) => {
                    found.nodes = Some(nodes);
                    return Err(error);
                }
            }
        }
        found.node = None;
        found.at = None;
        found.nodes = Some(nodes);
        Ok(())
    }
}

/// Phase 2: index `text`, then read `format` and refuse a file this
/// build does not read, a key the format does not define, and a map
/// with no `name`. B.14 steps 4–9, in that order.
pub(crate) fn index(text: &str) -> Result<MapIndex<'_>, LoadError> {
    let mut found = Found::default();
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let read = TopLevel(&mut found)
        .deserialize(&mut deserializer)
        .and_then(|()| deserializer.end());
    if let Err(error) = read {
        return Err(index_refusal(&found, &error));
    }

    let format = found.values[0];
    let Some(format) = format else {
        return Err(LoadError::new(
            String::new(),
            LoadErrorKind::MissingKey {
                key: "format".to_string(),
            },
        ));
    };
    let at_format = |kind| LoadError {
        position: Some(position_at(text, offset_in(text, format.get()).unwrap_or(0))),
        ..LoadError::new("format".to_string(), kind)
    };
    match serde_json::from_str::<u64>(format.get()) {
        Ok(found) if found == FORMAT => {}
        Ok(found) if found > 0 => {
            return Err(at_format(LoadErrorKind::UnsupportedFormat {
                found,
                newest: FORMAT,
            }))
        }
        _ => {
            return Err(at_format(LoadErrorKind::Invalid {
                message: format!(
                    "`format` is a positive integer, and this build reads {FORMAT}; found `{}`",
                    format.get()
                ),
            }))
        }
    }
    if let Some(key) = found.unknown {
        let path = crate::document::format::error::render_path(&[Seg::Name(key.clone())]);
        return Err(LoadError::new(path, LoadErrorKind::UnknownKey { key }));
    }
    if found.values[1].is_none() {
        return Err(LoadError::new(
            String::new(),
            LoadErrorKind::MissingKey {
                key: "name".to_string(),
            },
        ));
    }
    Ok(MapIndex {
        text,
        values: found.values,
        nodes: found.nodes.unwrap_or_default(),
    })
}

/// The [`LoadError`] for an index that the reader stopped: a key
/// written twice, malformed text anywhere, a top level that is not an
/// object, or a `nodes` that is not an array. The position is the
/// reader's, which is already in the whole text, and is named once: the
/// message leaves out the reader's own ` at line L column C`.
fn index_refusal(found: &Found<'_>, error: &serde_json::Error) -> LoadError {
    use serde_json::error::Category;
    let position = reader_position(error.line(), error.column());
    let mut path = String::new();
    if let Some(key) = &found.at {
        let mut segs = vec![Seg::Name(key.clone())];
        segs.extend(found.node.map(Seg::Index));
        path = crate::document::format::error::render_path(&segs);
    }
    let kind = if let Some(key) = &found.duplicate {
        path = String::new();
        LoadErrorKind::DuplicateKey { key: key.clone() }
    } else if matches!(error.classify(), Category::Data) {
        LoadErrorKind::Invalid {
            message: without_position(error),
        }
    } else {
        LoadErrorKind::Syntax {
            message: without_position(error),
        }
    };
    LoadError {
        position,
        ..LoadError::new(path, kind)
    }
}

/// The reader's line and column as a [`TextPosition`], or `None` for
/// the reader's "no position" (line 0).
fn reader_position(line: usize, column: usize) -> Option<TextPosition> {
    (line > 0).then(|| TextPosition {
        line: u32::try_from(line).unwrap_or(u32::MAX),
        column: u32::try_from(column).unwrap_or(u32::MAX),
    })
}

/// Parse the value at `extent` as a `T`, strictly
/// ([`strict::Objects`]: a struct is an object and nothing else), with
/// `prefix` as the path of the value. A refusal names the path inside
/// the value after the prefix, and the reader's position translated to
/// the whole text.
pub(crate) fn parse_extent<'t, T: Deserialize<'t>>(
    text: &'t str,
    extent: &'t RawValue,
    prefix: &[Seg],
) -> Result<T, LoadError> {
    rows::clear_fault();
    let mut deserializer = serde_json::Deserializer::from_str(extent.get());
    let parsed: Result<T, serde_path_to_error::Error<serde_json::Error>> =
        serde_path_to_error::deserialize(strict::Objects(&mut deserializer));
    parsed.map_err(|error| refusal(error, prefix, text, extent.get()))
}

/// The reader's message for `error`, without the ` at line L column C`
/// its `Display` ends with. That place is counted inside the extent the
/// value was read from, not the text; the error's own
/// [`position`](LoadError::position) is the place in the text, and a
/// message names one place, once, in its context.
fn without_position(error: &serde_json::Error) -> String {
    let message = error.to_string();
    if error.line() == 0 {
        return message;
    }
    let suffix = format!(" at line {} column {}", error.line(), error.column());
    match message.strip_suffix(&suffix) {
        Some(stripped) => stripped.to_string(),
        None => message,
    }
}

/// The [`LoadError`] for a value that failed to parse, inside `extent`,
/// a slice of `text`. When what failed was a row inside it, which is
/// read by a reader of its own ([`rows`]), the row's fault says the rest
/// of the path and the place: the error is the innermost reader's, at
/// the whole path, at its place in the text.
///
/// The reader's messages are the only place the kind of a data error is
/// written, so the kinds that name a key are read back out of them; the
/// tests pin every message shape this depends on, and a message that is
/// not recognized is an [`Invalid`](LoadErrorKind::Invalid) with the
/// reader's text, which still carries the path.
fn refusal(
    error: serde_path_to_error::Error<serde_json::Error>,
    prefix: &[Seg],
    text: &str,
    extent: &str,
) -> LoadError {
    use serde_json::error::Category;
    let (mut path, mut has_unknown_step) = render_parse_path(prefix, error.path());
    let inner = error.into_inner();
    // Where the reader that stopped began, as an offset into `extent`:
    // its start, or the start of the row whose fault it is.
    let (inner, into_extent) = match rows::take_fault(&inner.to_string(), extent) {
        Some(fault) => {
            for step in &fault.steps {
                has_unknown_step |= push_parse_path(&mut path, step);
            }
            (fault.error, fault.base.0.checked_sub(extent.as_ptr() as usize))
        }
        None => (inner, Some(0)),
    };
    // Both offsets are of slices of `text`: the extent by
    // `parse_extent`'s contract (the index borrowed it from `text`), and
    // the row by `take_fault`, which takes a fault only from inside the
    // extent. An offset outside would be a defect in the loader, never a
    // property of the input, and saturating it would put the error at
    // the wrong place in silence, so it is asserted.
    let start = offset_in(text, extent)
        .zip(into_extent)
        .map(|(extent_at, into)| extent_at + into)
        .expect("a value's reader begins inside the text it was read from");
    let base = position_at(text, start);
    let position = Some(within(base, inner.line(), inner.column()));
    let message = without_position(&inner);
    let at = |path: String, kind| LoadError {
        position,
        ..LoadError::new(path, kind)
    };

    if matches!(inner.classify(), Category::Syntax | Category::Eof | Category::Io) {
        // Only the reader's own depth limit reaches here: the text was
        // already read whole by the index. The path is the last value the
        // reader entered, or the prefix when it died on a key.
        let path = if has_unknown_step {
            crate::document::format::error::render_path(prefix)
        } else {
            path
        };
        return at(path, LoadErrorKind::Syntax { message });
    }

    let quoted = |start: &str| {
        message
            .strip_prefix(start)
            .and_then(|rest| rest.split('`').next())
            .map(str::to_string)
    };
    if let Some(key) = quoted("unknown field `") {
        // The reader has already entered the key: the path ends in it.
        return at(path, LoadErrorKind::UnknownKey { key });
    }
    if let Some(key) = quoted("missing field `") {
        return at(path, LoadErrorKind::MissingKey { key });
    }
    if let Some(key) = quoted("duplicate field `") {
        return at(path, LoadErrorKind::DuplicateKey { key });
    }
    at(path, LoadErrorKind::Invalid { message })
}
