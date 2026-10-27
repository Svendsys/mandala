// SPDX-License-Identifier: MPL-2.0

//! What an error says beside its path: the excerpt that names a node to
//! a person, the escaping that keeps text from the file from rewriting
//! the line it is printed on, and the line and column of a place in the
//! text.
//!
//! **Text from the file is attacker-controlled** and ends up in a
//! terminal, a dialog and the IPC stream. So nothing here prints it
//! raw: a C0 or C1 control, DEL, a line or paragraph separator, and a
//! bidi embedding, override or isolate control are written as escapes.
//! The bidi controls are the "Trojan Source" class (CVE-2021-42574):
//! they make text display in another order than its bytes.
//!
//! **Everything here runs on the error path only.** Counting the lines
//! before an offset is O(offset), which a load that succeeds never pays.

use crate::document::element::{DocElement, DocKind};
use crate::document::format::dto::NodeDto;
use crate::document::format::error::TextPosition;
use crate::engine::tree::Tree;
use indextree::NodeId;
use serde_json::value::RawValue;
use std::fmt::{self, Write};
use unicode_segmentation::UnicodeSegmentation;

/// How many grapheme clusters an [`excerpt`] keeps.
pub const EXCERPT_GRAPHEMES: usize = 32;

/// Whether the saver writes `c` as a `\uXXXX` escape although JSON lets
/// it stand raw (design B.9): U+0085, U+2028 and U+2029, which some
/// editors and diff viewers break a line at, and the bidi embedding,
/// override and isolate controls U+202A–U+202E and U+2066–U+2069, which
/// can make a line display in an order other than its bytes. With them
/// escaped, a node's line is one physical line in every editor and a
/// diff of it cannot be visually reordered.
pub(crate) fn saver_escapes(c: char) -> bool {
    matches!(c, '\u{85}' | '\u{2028}' | '\u{2029}') || is_bidi_control(c)
}

/// Whether `c` is a bidi embedding, override or isolate control:
/// U+202A–U+202E, U+2066–U+2069.
pub(crate) fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Whether `c` is escaped wherever text from the file is shown: every
/// control (C0, DEL, C1) and everything [`saver_escapes`].
fn escaped_when_shown(c: char) -> bool {
    c.is_control() || saver_escapes(c)
}

/// Write `c` as JSON writes an escaped character: the short forms
/// `\b`, `\f`, `\n`, `\r`, `\t`, else `\uXXXX` with lowercase hex.
fn write_escape(out: &mut impl Write, c: char) -> fmt::Result {
    match c {
        '\u{8}' => out.write_str("\\b"),
        '\u{c}' => out.write_str("\\f"),
        '\n' => out.write_str("\\n"),
        '\r' => out.write_str("\\r"),
        '\t' => out.write_str("\\t"),
        // Every character escaped here is in the Basic Multilingual
        // Plane, so one `\uXXXX` holds it.
        _ => write!(out, "\\u{:04x}", u32::from(c)),
    }
}

/// `text` for a message: shown as written, except that every control,
/// line separator and bidi control is escaped as JSON would escape it.
/// Quotes and backslashes stay as they are, so a message reads
/// naturally. Allocates nothing; formats in place.
pub struct Shown<'a>(&'a str);

impl fmt::Display for Shown<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut rest = self.0;
        while let Some((at, c)) = rest.char_indices().find(|(_, c)| escaped_when_shown(*c)) {
            f.write_str(&rest[..at])?;
            write_escape(f, c)?;
            rest = &rest[at + c.len_utf8()..];
        }
        f.write_str(rest)
    }
}

/// [`Shown`] for `text`.
pub fn shown(text: &str) -> Shown<'_> {
    Shown(text)
}

/// The excerpt an error shows for a node whose first span reads `text`:
/// at most [`EXCERPT_GRAPHEMES`] grapheme clusters, then `…` if any
/// were cut, escaped as a JSON string's content is (`"` and `\` too) and
/// with every control, line separator and bidi control escaped as well.
/// Cut by grapheme cluster, so a base letter is never parted from its
/// combining marks; cut before it is escaped, so an escape is never cut
/// in half.
pub fn excerpt(text: &str) -> String {
    let mut out = String::new();
    let mut graphemes = text.graphemes(true);
    for grapheme in graphemes.by_ref().take(EXCERPT_GRAPHEMES) {
        for c in grapheme.chars() {
            // Writing to a `String` cannot fail.
            let _ = match c {
                '"' => out.write_str("\\\""),
                '\\' => out.write_str("\\\\"),
                c if escaped_when_shown(c) => write_escape(&mut out, c),
                c => out.write_char(c),
            };
        }
    }
    if graphemes.next().is_some() {
        out.push('…');
    }
    out
}

/// The line and column of byte `offset` in `text`, both 1-based; the
/// column counts bytes from the start of the line. An offset past the
/// end, or inside a character, is taken at the nearest earlier one.
/// O(offset): the error path only.
pub fn position_at(text: &str, offset: usize) -> TextPosition {
    let before = &text.as_bytes()[..offset.min(text.len())];
    let line = before.iter().filter(|&&b| b == b'\n').count() + 1;
    let line_start = before.iter().rposition(|&b| b == b'\n').map_or(0, |n| n + 1);
    TextPosition {
        line: saturate(line),
        column: saturate(before.len() - line_start + 1),
    }
}

/// The byte offset of `part` in `text`, when `part` is a slice of it:
/// how a value the reader borrowed from the text (an extent) finds its
/// place. `None` for a string that is not inside `text`.
pub fn offset_in(text: &str, part: &str) -> Option<usize> {
    let start = text.as_ptr() as usize;
    let at = part.as_ptr() as usize;
    (at >= start && at + part.len() <= start + text.len()).then(|| at - start)
}

/// The place in the whole text of a position the JSON reader reported
/// inside an extent that starts at `base`: `line` and `column` are the
/// reader's, 1-based within the extent. The reader reports line `0`
/// when an error has no position, which gives `base`.
pub fn within(base: TextPosition, line: usize, column: usize) -> TextPosition {
    match line {
        0 => base,
        1 => TextPosition {
            line: base.line,
            column: base.column.saturating_add(saturate(column)).saturating_sub(1),
        },
        _ => TextPosition {
            line: base.line.saturating_add(saturate(line - 1)),
            column: saturate(column),
        },
    }
}

/// `value` as a `u32`, or `u32::MAX` past it: a text of more than four
/// billion lines is beyond any memory the app runs in, and a position
/// is information, never a reason to fail.
fn saturate(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// The excerpt of a node as the loader read it: the text of the first
/// span of its lowest-channel section, else of its first ring. The
/// lowest channel, the first among equals, so that it names the same
/// text the built node would show first even in a node refused for
/// listing its sections out of order.
pub(crate) fn dto_excerpt(node: &NodeDto) -> Option<String> {
    let section = lowest_first_text(
        node.sections
            .iter()
            .map(|section| (section.channel, section.spans.first().map(|span| &*span.text))),
    );
    let ring = || {
        node.frame.as_ref().and_then(|frame| {
            lowest_first_text(
                frame
                    .rings
                    .iter()
                    .map(|ring| (ring.channel, ring.spans.first().map(|span| &*span.text))),
            )
        })
    };
    section.or_else(ring).map(excerpt)
}

/// Of rows given as (channel, the text of the first span), the text of
/// the row with the lowest channel, the first among equals.
fn lowest_first_text<'a>(rows: impl Iterator<Item = (u64, Option<&'a str>)>) -> Option<&'a str> {
    let mut best: Option<(u64, Option<&'a str>)> = None;
    for (channel, text) in rows {
        if best.is_none_or(|(low, _)| channel < low) {
            best = Some((channel, text));
        }
    }
    best.and_then(|(_, text)| text)
}

/// The excerpt of the node at `node` in `tree`: the text of the first
/// span of its first section, else of its frame's first ring. Rows are
/// kept in channel order, so the first is the lowest. O(the node's
/// row).
pub(crate) fn tree_excerpt(tree: &Tree<DocElement>, node: NodeId) -> Option<String> {
    let arena = tree.arena();
    let first_of = |parent: NodeId, kind: DocKind| {
        parent
            .children(arena)
            .find(|&child| arena.get(child).is_some_and(|n| n.get().kind() == kind))
    };
    let first_text = |parent: NodeId| {
        first_of(parent, DocKind::Span).and_then(|span| match arena.get(span).map(|n| n.get()) {
            Some(DocElement::Span(span)) => Some(span.text.as_str()),
            _ => None,
        })
    };
    let section = first_of(node, DocKind::Section).and_then(first_text);
    let ring = || {
        first_of(node, DocKind::Frame)
            .and_then(|frame| first_of(frame, DocKind::Ring))
            .and_then(first_text)
    };
    section.or_else(ring).map(excerpt)
}

/// The id of the node whose extent is `extent`, read leniently — only
/// that key, everything else skipped — for an error from a node whose
/// DTO did not parse. `None` when the id itself is missing, not a
/// number, or out of range: when the id is what is wrong.
pub(crate) fn peek_id(extent: &RawValue) -> Option<u32> {
    #[derive(serde::Deserialize)]
    struct IdPeek {
        id: Option<u64>,
    }
    serde_json::from_str::<IdPeek>(extent.get())
        .ok()
        .and_then(|peek| peek.id)
        .and_then(|id| crate::document::bounds::check_node_ident(id).ok())
}

/// The excerpt of the node whose extent is `extent`, read leniently:
/// only the spans' text of its sections and rings, everything else
/// skipped. `None` when those do not read, or hold no text.
pub(crate) fn peek_excerpt(extent: &RawValue) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct SpanPeek {
        text: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct RowPeek {
        #[serde(default)]
        channel: u64,
        #[serde(default)]
        spans: Vec<SpanPeek>,
    }
    #[derive(serde::Deserialize)]
    struct FramePeek {
        #[serde(default)]
        rings: Vec<RowPeek>,
    }
    #[derive(serde::Deserialize)]
    struct TextPeek {
        #[serde(default)]
        sections: Vec<RowPeek>,
        frame: Option<FramePeek>,
    }
    let peek = serde_json::from_str::<TextPeek>(extent.get()).ok()?;
    let first = |rows: &[RowPeek]| {
        rows.iter()
            .enumerate()
            .min_by_key(|(n, row)| (row.channel, *n))
            .and_then(|(_, row)| row.spans.first())
            .and_then(|span| span.text.clone())
    };
    first(&peek.sections)
        .or_else(|| peek.frame.as_ref().and_then(|frame| first(&frame.rings)))
        .map(|text| excerpt(&text))
}
