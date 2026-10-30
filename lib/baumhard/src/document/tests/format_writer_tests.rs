// SPDX-License-Identifier: MPL-2.0

//! How a saved map looks (design B.9, the layout decided in
//! `format/writer.rs`), and `save_to`, which streams it.

use crate::document::format::to_dto::header_of;
use crate::document::format::{load, save, save_to, SaveErrorKind};
use crate::document::tests::format_support::{fixture, same_document, written, FIXTURES};
use serde_json::Value;
use std::io::{self, Write};

/// The node list of a saved `text`, as lines: everything between
/// `"nodes": [` and its `]`.
fn node_lines(text: &str) -> Vec<&str> {
    let Some(start) = text.find("\n  \"nodes\": [\n") else {
        return Vec::new();
    };
    let body = &text[start + "\n  \"nodes\": [\n".len()..];
    let end = body.find("\n  ]\n}\n").expect("the list closes");
    body[..end].split('\n').collect()
}

/// The node blocks of a saved `text`: each node's lines, from its
/// `    {` to its `    }` (with the `,` that follows it but the last).
fn node_blocks(text: &str) -> Vec<Vec<&str>> {
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    for line in node_lines(text) {
        if line == "    {" {
            blocks.push(Vec::new());
        }
        if let Some(block) = blocks.last_mut() {
            block.push(line);
        }
    }
    blocks
}

#[test]
fn test_a_saved_map_is_a_pretty_header_then_a_block_per_node_and_a_line_per_key() {
    let mut keys = 0;
    for (name, text) in FIXTURES {
        assert!(!text.contains('\r'), "{name}: LF only");
        assert!(
            text.ends_with("}\n") && !text.ends_with("\n\n"),
            "{name}: one trailing newline"
        );
        let doc = load(text).unwrap();
        let blocks = node_blocks(text);
        let count = doc
            .tree()
            .descendants()
            .filter(|id| doc.get(*id).is_some_and(|element| element.as_node().is_some()))
            .count();
        assert_eq!(blocks.len(), count, "{name}: a block per node");
        // Every line of the list belongs to a block.
        assert_eq!(
            blocks.iter().map(Vec::len).sum::<usize>(),
            node_lines(text).len(),
            "{name}: a line outside a node's block"
        );
        for (n, block) in blocks.iter().enumerate() {
            let last_node = n + 1 == blocks.len();
            // `    {`, a line per key, then `    }` with a `,` but the last.
            assert_eq!(block[0], "    {", "{name}");
            let close = block[block.len() - 1];
            assert_eq!(close, if last_node { "    }" } else { "    }," }, "{name}");
            let key_lines = &block[1..block.len() - 1];
            assert!(!key_lines.is_empty(), "{name}: a node with no key");
            for (k, line) in key_lines.iter().enumerate() {
                let last_key = k + 1 == key_lines.len();
                assert_eq!(line.ends_with(','), !last_key, "{name}: {line}");
                // Each key line, without its comma, is one JSON member:
                // a value never spans two lines.
                let member = line
                    .strip_prefix("      ")
                    .expect("six spaces in")
                    .strip_suffix(',')
                    .unwrap_or_else(|| line.trim_start());
                assert!(!member.starts_with(' '), "{name}: {line}");
                let value: Value = serde_json::from_str(&format!("{{{member}}}"))
                    .unwrap_or_else(|e| panic!("{name}: {e}: {line}"));
                assert_eq!(
                    value.as_object().map(|object| object.len()),
                    Some(1),
                    "{name}: {line}"
                );
                keys += 1;
            }
            assert!(
                key_lines[0].starts_with("      \"id\": "),
                "{name}: {}",
                key_lines[0]
            );
            // The block, without the `,` after it, is one JSON object.
            let joined = block.join("\n");
            let object = joined.strip_suffix(',').unwrap_or(&joined);
            let value: Value = serde_json::from_str(object).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(value.is_object(), "{name}");
        }
        // The header is serde_json's own two-space pretty form of the
        // header, which this test asks for without the writer.
        let header = serde_json::to_string_pretty(&header_of(&doc).unwrap()).unwrap();
        let expected_head = match blocks.is_empty() {
            true => format!("{header}\n"),
            false => format!("{},\n  \"nodes\": [\n", header.strip_suffix("\n}").unwrap()),
        };
        assert!(
            text.starts_with(&expected_head),
            "{name}: the header is not the pretty form:\n{text}"
        );
    }
    assert!(keys > 40, "precondition: {keys} key lines");
}

#[test]
fn test_line_separators_and_bidi_controls_are_escaped_and_read_back() {
    // A span and the map's name carrying every character the saver
    // escapes beyond JSON's own.
    let hostile = "a\u{2028}b\u{2029}c\u{85}d\u{202E}e\u{2066}f";
    let raw = format!(
        r#"{{"format":1,"name":"n{hostile}","nodes":[{{"id":1,"position":[0,0],"sections":[{{"spans":[{{"text":"{hostile}"}}]}}]}}]}}"#
    );
    let doc = load(&raw).expect("raw, they read");
    let text = written(&doc);
    for (c, escape) in [
        ('\u{2028}', "\\u2028"),
        ('\u{2029}', "\\u2029"),
        ('\u{85}', "\\u0085"),
        ('\u{202E}', "\\u202e"),
        ('\u{2066}', "\\u2066"),
    ] {
        assert!(!text.contains(c), "{c:?} written raw");
        assert_eq!(
            text.matches(escape).count(),
            2,
            "{escape} in the name and the span"
        );
    }
    // So each key is still one line, in any editor's count of lines:
    // the node's block is `{`, its three keys, `}`.
    assert_eq!(node_lines(&text).len(), 5);
    // The escaped and the raw spelling are one document.
    same_document(&doc, &load(&text).unwrap()).unwrap();
    // Control: text with none of them is written as it is, right-to-left
    // and accented letters included.
    let plain = "שלום é ü";
    let doc = load(&format!(
        r#"{{"format":1,"name":"t","nodes":[{{"id":1,"position":[0,0],"sections":[{{"spans":[{{"text":"{plain}"}}]}}]}}]}}"#
    ))
    .unwrap();
    assert!(written(&doc).contains(plain));
}

/// A writer that takes `room` bytes and then fails.
struct Full {
    room: usize,
    taken: Vec<u8>,
}

impl Write for Full {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.room == 0 {
            return Err(io::Error::other("the disk is full"));
        }
        let n = buf.len().min(self.room);
        self.taken.extend_from_slice(&buf[..n]);
        self.room -= n;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn test_save_to_streams_the_same_text_and_names_the_node_it_could_not_write() {
    let text = fixture("tree");
    let doc = load(text).unwrap();
    let mut streamed = Vec::new();
    save_to(&doc, &mut streamed).unwrap();
    assert_eq!(streamed, save(&doc).unwrap().into_bytes());
    assert_eq!(streamed, text.as_bytes());

    // A writer that fails ten bytes into node 3's block, the fourth.
    let line = text.find("    {\n      \"id\": 3,").unwrap();
    let mut full = Full {
        room: line + 10,
        taken: Vec::new(),
    };
    let error = save_to(&doc, &mut full).unwrap_err();
    assert_eq!(error.path, "nodes[3]");
    assert!(
        matches!(&error.kind, SaveErrorKind::Io { message } if message.contains("the disk is full")),
        "{error:?}"
    );
    assert_eq!(full.taken, text.as_bytes()[..line + 10]);
    // Failing in the header names no node.
    let error = save_to(
        &doc,
        &mut Full {
            room: 20,
            taken: Vec::new(),
        },
    )
    .unwrap_err();
    assert_eq!(error.path, "");
    // Control: a writer with room for it all takes the whole text.
    let mut roomy = Full {
        room: text.len(),
        taken: Vec::new(),
    };
    save_to(&doc, &mut roomy).unwrap();
    assert_eq!(roomy.taken, text.as_bytes());
}
