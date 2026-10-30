// SPDX-License-Identifier: MPL-2.0

//! How a saved map looks (design B.9): the one place the layout of the
//! text is decided. The saver hands this module the header's DTO and
//! then each node's, in list order, and this module decides where the
//! whitespace goes.
//!
//! **The layout** (Q2, ruling R20 on #236). The header — every
//! top-level key but `nodes` — is serde_json's two-space pretty
//! printing. Then `"nodes": [` and the nodes in pre-order, each a block:
//! `    {` alone on a line; one line per key the node has, in schema
//! order, six spaces and `"key": value`, with `,` ending every key's
//! line but the node's last; then `    }`, with `,` after every node but
//! the last. A key's value is in compact form, with `", "` between
//! members and elements, `": "` after each key and no space inside a
//! bracket. Then `  ]` and `}`. With no nodes, `nodes` is left out. LF
//! line endings, one trailing newline, no byte-order mark.
//!
//! **One key, one line, in every editor.** A value never spans two
//! lines. A newline inside a string
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

/// A node's block: the node object's own keys one per line, six
/// spaces in, and every value inside them in serde_json's compact form
/// with a space after each `,` and `:`.
struct NodeBlock {
    /// How many objects and arrays are open: 1 inside the node's own
    /// object.
    depth: usize,
}

impl Formatter for NodeBlock {
    fn begin_array<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth += 1;
        writer.write_all(b"[")
    }

    fn end_array<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth = self.depth.saturating_sub(1);
        writer.write_all(b"]")
    }

    fn begin_array_value<W: ?Sized + Write>(&mut self, writer: &mut W, first: bool) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth += 1;
        writer.write_all(b"{")
    }

    fn end_object<W: ?Sized + Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth = self.depth.saturating_sub(1);
        if self.depth == 0 {
            // The node's own closing brace, on its own line.
            writer.write_all(b"\n    }")
        } else {
            writer.write_all(b"}")
        }
    }

    fn begin_object_key<W: ?Sized + Write>(&mut self, writer: &mut W, first: bool) -> io::Result<()> {
        match (self.depth, first) {
            // One of the node's own keys: a line of its own.
            (1, true) => writer.write_all(b"\n      "),
            (1, false) => writer.write_all(b",\n      "),
            (_, true) => Ok(()),
            (_, false) => writer.write_all(b", "),
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

/// Write one node's block: the `,` and newline that end the block before
/// it unless it is the first, the indent, and the node, a key per line.
pub(super) fn node<W: Write>(out: &mut W, first: bool, node: &NodeDto) -> io::Result<()> {
    if !first {
        out.write_all(b",\n")?;
    }
    out.write_all(b"    ")?;
    write_with(out, NodeBlock { depth: 0 }, node)
}

/// Close the node list and the file, if the map has nodes; the header
/// closed the file already if it has none.
pub(super) fn end<W: Write>(out: &mut W, has_nodes: bool) -> io::Result<()> {
    if has_nodes {
        out.write_all(b"\n  ]\n}\n")?;
    }
    Ok(())
}
