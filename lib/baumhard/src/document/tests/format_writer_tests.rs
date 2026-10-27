// SPDX-License-Identifier: MPL-2.0

//! How a saved map looks (design B.9, the layout decided in
//! `format/writer.rs`), and `save_to`, which streams it.

use crate::document::format::to_dto::header_of;
use crate::document::format::{load, save, save_to, SaveErrorKind};
use crate::document::tests::format_support::{fixture, same_document, written, FIXTURES};
use serde_json::Value;
use std::io::{self, Write};

/// The node lines of a saved `text`: everything between `"nodes": [`
/// and its `]`.
fn node_lines(text: &str) -> Vec<&str> {
    let Some(start) = text.find("\n  \"nodes\": [\n") else {
        return Vec::new();
    };
    let body = &text[start + "\n  \"nodes\": [\n".len()..];
    let end = body.find("\n  ]\n}\n").expect("the list closes");
    body[..end].split('\n').collect()
}

#[test]
fn test_a_saved_map_is_a_pretty_header_then_one_node_per_line() {
    let mut lines = 0;
    for (name, text) in FIXTURES {
        assert!(!text.contains('\r'), "{name}: LF only");
        assert!(
            text.ends_with("}\n") && !text.ends_with("\n\n"),
            "{name}: one trailing newline"
        );
        let doc = load(text).unwrap();
        // Each node is one line, four spaces in, that is a JSON object
        // on its own, ending in `,` but the last.
        let nodes = node_lines(text);
        let count = doc
            .tree()
            .descendants()
            .filter(|id| doc.get(*id).is_some_and(|element| element.as_node().is_some()))
            .count();
        assert_eq!(nodes.len(), count, "{name}: a line per node");
        for (n, line) in nodes.iter().enumerate() {
            let last = n + 1 == nodes.len();
            assert_eq!(line.ends_with(','), !last, "{name}: {line}");
            let object = line
                .strip_prefix("    ")
                .expect("four spaces in")
                .trim_end_matches(',');
            let value: Value = serde_json::from_str(object).unwrap_or_else(|e| panic!("{name}: {e}: {line}"));
            assert!(value.is_object(), "{name}: {line}");
            assert!(object.starts_with("{\"id\": "), "{name}: {line}");
            lines += 1;
        }
        // The header is serde_json's own two-space pretty form of the
        // header, which this test asks for without the writer.
        let header = serde_json::to_string_pretty(&header_of(&doc).unwrap()).unwrap();
        let expected_head = match nodes.is_empty() {
            true => format!("{header}\n"),
            false => format!("{},\n  \"nodes\": [\n", header.strip_suffix("\n}").unwrap()),
        };
        assert!(
            text.starts_with(&expected_head),
            "{name}: the header is not the pretty form:\n{text}"
        );
    }
    assert!(lines > 15, "precondition: {lines} node lines");
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
    // So the node is still one line, in any editor's count of lines.
    assert_eq!(node_lines(&text).len(), 1);
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

    // A writer that fails ten bytes into node 3's line, the fourth.
    let line = text.find("    {\"id\": 3,").unwrap();
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
