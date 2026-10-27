// SPDX-License-Identifier: MPL-2.0

//! What an error says beside its path: the excerpt that names a node,
//! the escaping that keeps text from the file out of the line an error
//! is printed on, the position, and how the three render.

use crate::document::format::context::{
    excerpt, position_at, shown, within, EXCERPT_BYTES, EXCERPT_GRAPHEMES,
};
use crate::document::format::{
    LoadError, LoadErrorKind, NodeContext, SaveError, SaveErrorKind, TextPosition,
};
use crate::document::tests::format_text::{map_of, map_with, node_of, rejection};

/// The characters an error must never print raw: C0, DEL, C1, the line
/// and paragraph separators, and the bidi controls.
fn is_dangerous(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

#[test]
fn test_an_excerpt_is_cut_at_a_grapheme_never_inside_one() {
    // 31 letters, then `e` with a combining acute accent: the 32nd
    // grapheme is two characters, so the 32nd *character* is the `e`
    // alone, and a cut by characters would part it from its accent.
    let text = format!("{}e\u{301}xyz", "a".repeat(31));
    let cut = excerpt(&text);
    assert_eq!(cut, format!("{}e\u{301}…", "a".repeat(31)));
    // Exactly the limit is not cut, and carries no ellipsis.
    let exact = format!("{}e\u{301}", "a".repeat(31));
    assert_eq!(excerpt(&exact), exact);
    assert_eq!(EXCERPT_GRAPHEMES, 32);
}

#[test]
fn test_an_excerpt_escapes_controls_quotes_separators_and_bidi() {
    // A right-to-left override would make the rest of the message read
    // backwards in a terminal; a newline would start a forged line.
    assert_eq!(
        excerpt("abc\u{202E}def\nx\"y\\z\u{2028}\u{85}\u{9b}\u{7f}"),
        "abc\\u202edef\\nx\\\"y\\\\z\\u2028\\u0085\\u009b\\u007f"
    );
    // Text with none of them is shown as it is, right-to-left included.
    assert_eq!(excerpt("שלום עולם"), "שלום עולם");
}

#[test]
fn test_text_shown_in_a_message_has_no_raw_control_but_keeps_its_quotes() {
    let hostile = "red\u{1b}[31m\u{2066}evil\u{2069}\r\n\"quoted\"";
    let rendered = shown(hostile).to_string();
    assert_eq!(rendered, "red\\u001b[31m\\u2066evil\\u2069\\r\\n\"quoted\"");
    assert!(!rendered.chars().any(is_dangerous), "{rendered:?}");
    // Every kind that carries text from the file renders it through the
    // same escape.
    let error = LoadError::new(
        "nodes[0].fill".to_string(),
        LoadErrorKind::UnknownVariable {
            name: hostile.to_string(),
        },
    );
    let rendered = error.to_string();
    assert!(!rendered.chars().any(is_dangerous), "{rendered:?}");
}

#[test]
fn test_an_error_renders_its_path_message_and_context() {
    let error = LoadError {
        path: "nodes[3].channel".to_string(),
        node: Some(NodeContext {
            index: 3,
            id: Some(15),
            excerpt: Some("Abel".to_string()),
        }),
        position: Some(TextPosition { line: 7, column: 5 }),
        kind: LoadErrorKind::SiblingsOutOfOrder {
            previous_channel: 2,
            previous_id: Some(12),
        },
    };
    assert_eq!(
        error.to_string(),
        "nodes[3].channel: siblings are listed in ascending channel, and the sibling before this one \
         (node 12) has channel 2 (node 15 \"Abel\", line 7)"
    );
    // Each part that is absent is left out; with none, so is the
    // parenthesis; with no path, so is the prefix.
    let no_id = LoadError {
        node: Some(NodeContext {
            index: 3,
            id: None,
            excerpt: None,
        }),
        position: Some(TextPosition { line: 7, column: 5 }),
        ..error.clone()
    };
    assert!(no_id.to_string().ends_with("has channel 2 (line 7)"), "{no_id}");
    let bare = LoadError::new(String::new(), LoadErrorKind::ByteOrderMark);
    assert_eq!(
        bare.to_string(),
        "the text starts with a byte-order mark, which a map does not have: save it as UTF-8 without a BOM"
    );
    let save = SaveError {
        path: "nodes[2].sections[0].size".to_string(),
        node: Some(NodeContext {
            index: 2,
            id: Some(9),
            excerpt: None,
        }),
        kind: SaveErrorKind::SizeInRing,
    };
    assert!(save.to_string().ends_with("its text (node 9)"), "{save}");
}

#[test]
fn test_a_position_is_one_based_and_translates_out_of_an_extent() {
    let text = "{\n  \"a\": [\n    {\"id\": 1}\n  ]\n}\n";
    let extent = text.find("{\"id\"").unwrap();
    let base = position_at(text, extent);
    assert_eq!(base, TextPosition { line: 3, column: 5 });
    assert_eq!(position_at(text, 0), TextPosition { line: 1, column: 1 });
    // The reader's line 1 is the extent's own line, offset by where the
    // extent starts on it; a later line keeps the reader's column.
    assert_eq!(within(base, 1, 3), TextPosition { line: 3, column: 7 });
    assert_eq!(within(base, 2, 4), TextPosition { line: 4, column: 4 });
    // Line 0 is the reader's "no position".
    assert_eq!(within(base, 0, 0), base);
}

#[test]
fn test_an_excerpt_is_bounded_in_bytes_as_well_as_in_clusters() {
    // One cluster of any length: a letter and 100,000 combining marks.
    // It is cut at a character boundary at the byte bound: `a` and 127
    // marks of two bytes each are 255 bytes, and one more is 257.
    let one_cluster = format!("a{}", "\u{301}".repeat(100_000));
    assert_eq!(excerpt(&one_cluster), format!("a{}…", "\u{301}".repeat(127)));
    // Clusters of 25 bytes each: ten fit in the bound, the eleventh does
    // not, and is left whole rather than parted.
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    assert_eq!(family.len(), 25);
    assert_eq!(excerpt(&family.repeat(40)), format!("{}…", family.repeat(10)));
    // Short clusters still stop at 32.
    assert_eq!(excerpt(&"€".repeat(100)), format!("{}…", "€".repeat(32)));
    assert_eq!(EXCERPT_BYTES, 256);
    // And an error on a node holding the long cluster stays short.
    let json = map_of(&format!(
        r#","sections":[{{"spans":[{{"text":"{one_cluster}"}}]}}],"fill":"var(nope)""#
    ));
    let error = rejection(&json);
    assert_eq!(
        (error.path.as_str(), &error.kind),
        (
            "nodes[0].fill",
            &LoadErrorKind::UnknownVariable {
                name: "nope".to_string()
            }
        )
    );
    let rendered = error.to_string();
    assert!(rendered.len() < 512, "{} bytes: {rendered}", rendered.len());
}

#[test]
fn test_a_key_from_the_file_is_escaped_in_a_path_wherever_it_is() {
    // Every character a message must not print raw, in a key, spelled
    // as JSON escapes in the file and read as the characters.
    let key = r"x\u202e\u2028\u0085\u007f\u009b\u2066\u001by";
    let in_path = format!(r#"["{key}"]"#);
    // An unknown key in a node.
    let error = rejection(&map_of(&format!(r#","{key}":1"#)));
    assert_eq!(error.path, format!("nodes[0]{in_path}"));
    assert!(
        matches!(error.kind, LoadErrorKind::UnknownKey { .. }),
        "{error:?}"
    );
    let rendered = error.to_string();
    assert!(!rendered.chars().any(is_dangerous), "{rendered:?}");
    // An unknown key at the top.
    let error = rejection(&map_with(&format!(r#","{key}":1"#), &node_of(1, "")));
    assert_eq!(error.path, in_path);
    assert!(
        matches!(error.kind, LoadErrorKind::UnknownKey { .. }),
        "{error:?}"
    );
    let rendered = error.to_string();
    assert!(!rendered.chars().any(is_dangerous), "{rendered:?}");
    // A palette name refused for what it holds, at its own path.
    let error = rejection(&map_with(
        &format!(r#","palettes":{{"{key}":[{{"fill":"var(bg)","frame":"var(bg)","text":"var(fg)"}}]}}"#),
        &node_of(1, ""),
    ));
    assert_eq!(
        (error.path.as_str(), &error.kind),
        (
            format!("palettes{in_path}").as_str(),
            &LoadErrorKind::NameNotPrintable { code_point: 0x202E }
        )
    );
    let rendered = error.to_string();
    assert!(!rendered.chars().any(is_dangerous), "{rendered:?}");
}

#[test]
fn test_a_content_error_inside_a_value_names_its_place_once() {
    // The reader's ` at line L column C` counts inside the value it read
    // (here node 4's own line), not the text: it is left out, and the
    // place is the error's position, shown once in its context.
    let nodes = format!(
        "{},\n{},\n{},\n{}",
        node_of(1, ""),
        node_of(2, ""),
        node_of(3, ""),
        r#"{"id":4,"position":"x"}"#
    );
    let error = rejection(&map_with("", &nodes));
    assert_eq!(error.path, "nodes[3].position");
    assert!(matches!(error.kind, LoadErrorKind::Invalid { .. }), "{error:?}");
    assert_eq!(error.position.map(|p| p.line), Some(4));
    let rendered = error.to_string();
    assert!(!rendered.contains("at line"), "{rendered}");
    assert_eq!(rendered.matches("line ").count(), 1, "{rendered}");
    // The same in a header value.
    let error = rejection(&map_with(r#","verbs":[{"id":1,"name":"v"}]"#, &node_of(1, "")));
    assert_eq!(error.path, "verbs[0].id");
    assert!(matches!(error.kind, LoadErrorKind::Invalid { .. }), "{error:?}");
    assert!(!error.to_string().contains("at line"), "{error}");
}
