// SPDX-License-Identifier: MPL-2.0

//! How a saved map looks (design B.9): the one place the layout of the
//! text is decided. The saver hands this module the header's DTO and
//! then each node's, in list order, and this module decides where the
//! whitespace goes.
//!
//! **The layout.** The header — every top-level key but `nodes` — is
//! serde_json's two-space pretty printing. Then `"nodes": [`, one line
//! per node in pre-order, each four spaces and the node object in
//! compact form with `", "` between members and elements and `": "`
//! after each key, a `,` ending every line but the last; then `  ]` and
//! `}`. With no nodes, `nodes` is left out. LF line endings, one
//! trailing newline, no byte-order mark.
//!
//! **One node, one line, in every editor.** A newline inside a string
//! is `\n` in JSON, and on top of serde_json's escaping this writes
//! U+0085, U+2028 and U+2029 — where some editors and diff viewers break
//! a line — and the bidi controls as `\uXXXX`
//! ([`saver_escapes`]),
//! so a line count agrees with a reported line number and a diff cannot
//! be shown in an order other than its bytes. This applies to every
//! string in the file, the header's too.

use crate::document::format::context::saver_escapes;
use crate::document::format::dto::{MapHeaderDto, NodeDto};
use serde::Serialize;
use serde_json::ser::{Formatter, PrettyFormatter};
use std::io::{self, Write};

/// A formatter that writes what `F` writes, except that the characters
/// [`saver_escapes`] names are written as `\uXXXX` inside strings.
struct Escaping<F>(F);

impl<F: Formatter> Formatter for Escaping<F> {
    fn begin_array<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.begin_array(writer)
    }

    fn end_array<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_array(writer)
    }

    fn begin_array_value<W: ?Sized + Write>(&mut self, writer: &mut W, first: bool) -> io::Result<()> {
        self.0.begin_array_value(writer, first)
    }

    fn end_array_value<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_array_value(writer)
    }

    fn begin_object<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.begin_object(writer)
    }

    fn end_object<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_object(writer)
    }

    fn begin_object_key<W: ?Sized + Write>(&mut self, writer: &mut W, first: bool) -> io::Result<()> {
        self.0.begin_object_key(writer, first)
    }

    fn end_object_key<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_object_key(writer)
    }

    fn begin_object_value<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.begin_object_value(writer)
    }

    fn end_object_value<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_object_value(writer)
    }

    fn write_string_fragment<W: ?Sized + Write>(&mut self, writer: &mut W, fragment: &str) -> io::Result<()> {
        let mut rest = fragment;
        while let Some((at, c)) = rest.char_indices().find(|(_, c)| saver_escapes(*c)) {
            writer.write_all(&rest.as_bytes()[..at])?;
            write!(writer, "\\u{:04x}", u32::from(c))?;
            rest = &rest[at + c.len_utf8()..];
        }
        writer.write_all(rest.as_bytes())
    }
}

/// serde_json's compact form with a space after each `,` and `:`: the
/// layout of a node's line.
struct Spaced;

impl Formatter for Spaced {
    fn begin_array_value<W: ?Sized + Write>(&mut self, writer: &mut W, first: bool) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_key<W: ?Sized + Write>(&mut self, writer: &mut W, first: bool) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_value<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        writer.write_all(b": ")
    }
}

/// `value` written through `formatter` to `out`.
fn write_with<W: Write, F: Formatter, T: Serialize + ?Sized>(
    out: &mut W,
    formatter: F,
    value: &T,
) -> io::Result<()> {
    let mut serializer = serde_json::Serializer::with_formatter(out, Escaping(formatter));
    value.serialize(&mut serializer).map_err(io::Error::other)
}

/// Write the header, and open the node list if the map has nodes. The
/// header's pretty form ends in `\n}`; with nodes, that brace is the
/// file's last and comes after the list instead. O(header).
pub(super) fn header<W: Write>(out: &mut W, header: &MapHeaderDto, has_nodes: bool) -> io::Result<()> {
    let mut text = Vec::new();
    write_with(&mut text, PrettyFormatter::with_indent(b"  "), header)?;
    if !has_nodes {
        out.write_all(&text)?;
        return out.write_all(b"\n");
    }
    // The header always has `format` and `name`, so it is never `{}`.
    let open = text.strip_suffix(b"\n}").unwrap_or(&text);
    out.write_all(open)?;
    out.write_all(b",\n  \"nodes\": [\n")
}

/// Write one node's line: the `,` and newline that end the line before
/// it unless it is the first, the indent, and the node.
pub(super) fn node<W: Write>(out: &mut W, first: bool, node: &NodeDto) -> io::Result<()> {
    if !first {
        out.write_all(b",\n")?;
    }
    out.write_all(b"    ")?;
    write_with(out, Spaced, node)
}

/// Close the node list and the file, if the map has nodes; the header
/// closed the file already if it has none.
pub(super) fn end<W: Write>(out: &mut W, has_nodes: bool) -> io::Result<()> {
    if has_nodes {
        out.write_all(b"\n  ]\n}\n")?;
    }
    Ok(())
}
