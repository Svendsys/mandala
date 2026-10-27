// SPDX-License-Identifier: MPL-2.0

//! The flat node list (design B.4, B.14): where a node's `parent` may
//! be and what the loader says when it is elsewhere; the context and
//! the place every error carries; which of several errors is reported;
//! and the text as a whole — its byte-order mark, its line endings, and
//! every proper prefix of a map.

use crate::document::bounds::BoundError;
use crate::document::format::error::CHAIN_SHOWN;
use crate::document::format::{load, load_from_bytes, LoadError, LoadErrorKind};
use crate::document::tests::format_support::{fixture, node, written, FIXTURES};
use crate::document::tests::format_text::{map_with, node_of, rejection};

/// A map of `nodes`, each `(id, body)` where `body` is a run of
/// `,"key":value` pairs.
fn list(nodes: &[(u32, &str)]) -> String {
    let nodes: Vec<String> = nodes.iter().map(|(id, body)| node_of(*id, body)).collect();
    map_with("", &nodes.join(","))
}

/// `error`'s path and kind, for one comparison.
fn at(error: &LoadError) -> (&str, &LoadErrorKind) {
    (error.path.as_str(), &error.kind)
}

// ---- where a parent may be ------------------------------------------

#[test]
fn test_a_node_names_itself_as_its_parent_and_is_refused() {
    let error = rejection(&list(&[(1, ""), (2, r#","parent":2"#)]));
    assert_eq!(
        at(&error),
        ("nodes[1].parent", &LoadErrorKind::ParentIsSelf { id: 2 })
    );
    assert_eq!(
        error.node.as_ref().map(|node| (node.index, node.id)),
        Some((1, Some(2)))
    );
    // Control: the same line naming the node before it loads.
    assert!(load(&list(&[(1, ""), (2, r#","parent":1"#)])).is_ok());
}

#[test]
fn test_a_parent_that_does_not_come_earlier_is_refused() {
    // One that never appears.
    let error = rejection(&list(&[(1, ""), (2, r#","parent":9"#)]));
    assert_eq!(
        at(&error),
        ("nodes[1].parent", &LoadErrorKind::ParentNotEarlier { parent: 9 })
    );
    // One that appears, but later: a parent comes before its children.
    let error = rejection(&list(&[(1, ""), (2, r#","parent":3"#), (3, r#","parent":1"#)]));
    assert_eq!(
        at(&error),
        ("nodes[1].parent", &LoadErrorKind::ParentNotEarlier { parent: 3 })
    );
    // Control: the parent moved before the child loads.
    assert!(load(&list(&[(1, ""), (3, r#","parent":1"#), (2, r#","parent":3"#)])).is_ok());
}

#[test]
fn test_a_parent_whose_subtree_has_ended_is_refused_and_the_chain_is_named() {
    // 2's subtree ended when 4 came under 1; 5 cannot hang from 2 now.
    let nodes = [
        (1, ""),
        (2, r#","parent":1"#),
        (3, r#","parent":2"#),
        (4, r#","parent":1"#),
        (5, r#","parent":2"#),
    ];
    let error = rejection(&list(&nodes));
    assert_eq!(
        at(&error),
        (
            "nodes[4].parent",
            &LoadErrorKind::ParentNotOnChain {
                parent: 2,
                parent_index: 1,
                expected: vec![4, 1],
                truncated: false,
            }
        )
    );
    assert!(
        error
            .to_string()
            .starts_with("nodes[4].parent: node 2 (nodes[1]) is earlier, but its subtree has ended; a node here can name 4, 1, or none for a new root"),
        "{error}"
    );
    // Control: 5 under 4, which is on the chain, loads; so does 5 under 1.
    let mut fixed = nodes;
    fixed[4] = (5, r#","parent":4"#);
    assert!(load(&list(&fixed)).is_ok());
    fixed[4] = (5, r#","parent":1"#);
    assert!(load(&list(&fixed)).is_ok());
}

#[test]
fn test_the_ids_a_refusal_offers_are_cut_at_eight_and_say_so() {
    // 100 and its child 101, then a chain 1 → 2 → … → 12; then a node
    // that names 101, whose subtree ended when 1 began.
    let mut nodes: Vec<String> = vec![node_of(100, ""), node_of(101, r#","parent":100"#), node_of(1, "")];
    for id in 2..=12 {
        nodes.push(node_of(id, &format!(r#","parent":{}"#, id - 1)));
    }
    nodes.push(node_of(50, r#","parent":101"#));
    let error = rejection(&map_with("", &nodes.join(",")));
    let expected: Vec<u32> = (5..=12).rev().collect();
    assert_eq!(expected.len(), CHAIN_SHOWN);
    assert_eq!(
        at(&error),
        (
            "nodes[14].parent",
            &LoadErrorKind::ParentNotOnChain {
                parent: 101,
                parent_index: 1,
                expected,
                truncated: true,
            }
        )
    );
    assert!(
        error
            .to_string()
            .contains("can name 12, 11, 10, 9, 8, 7, 6, 5, …, or none"),
        "{error}"
    );
    // Control: a chain of exactly eight is not cut.
    nodes.truncate(10);
    nodes.push(node_of(50, r#","parent":101"#));
    let error = rejection(&map_with("", &nodes.join(",")));
    assert!(
        matches!(&error.kind, LoadErrorKind::ParentNotOnChain { expected, truncated: false, .. } if expected.len() == 8),
        "{error:?}"
    );
}

#[test]
fn test_an_id_or_parent_out_of_range_is_a_bound_and_a_fraction_is_invalid() {
    let error = rejection(&list(&[(1, ""), (2, r#","parent":0"#)]));
    assert_eq!(
        at(&error),
        (
            "nodes[1].parent",
            &LoadErrorKind::Bound {
                error: BoundError::NotAnIdent(0)
            }
        )
    );
    let error = rejection(&list(&[(1, ""), (2, r#","parent":4294967296"#)]));
    assert_eq!(
        at(&error),
        (
            "nodes[1].parent",
            &LoadErrorKind::Bound {
                error: BoundError::NotAnIdent(1 << 32)
            }
        )
    );
    let error = rejection(&map_with("", r#"{"id":4294967296,"position":[0,0]}"#));
    assert_eq!(
        at(&error),
        (
            "nodes[0].id",
            &LoadErrorKind::Bound {
                error: BoundError::NotAnIdent(1 << 32)
            }
        )
    );
    // The id is what is wrong, so the error cannot name the node by it.
    assert_eq!(error.node.as_ref().map(|node| node.id), Some(None));
    // An integer written as a fraction or with an exponent is not one.
    for id in ["1.0", "1e0"] {
        let error = rejection(&map_with("", &format!(r#"{{"id":{id},"position":[0,0]}}"#)));
        assert!(
            matches!(error.kind, LoadErrorKind::Invalid { .. }),
            "{id}: {error:?}"
        );
        assert_eq!(error.path, "nodes[0].id", "{id}");
    }
    let error = rejection(&list(&[(1, ""), (2, r#","parent":1.0"#)]));
    assert!(matches!(error.kind, LoadErrorKind::Invalid { .. }), "{error:?}");
    // A top-level node has no connector.
    let error = rejection(&list(&[(1, r#","connector":{}"#)]));
    assert_eq!(
        at(&error),
        ("nodes[0].connector", &LoadErrorKind::ConnectorOnRoot)
    );
    // Controls: the same lines with the value corrected load.
    assert!(load(&list(&[(1, ""), (2, r#","parent":1"#)])).is_ok());
    assert!(load(&map_with("", r#"{"id":4294967294,"position":[0,0]}"#)).is_ok());
    assert!(load(&list(&[(1, ""), (2, r#","parent":1,"connector":{}"#)])).is_ok());
}

#[test]
fn test_a_merge_that_gave_two_nodes_one_id_is_refused_naming_both() {
    // Two branches each added a node under 1, and each took the next id,
    // 253; git merged the two lines cleanly.
    let mut nodes: Vec<String> = vec![node_of(1, "")];
    for id in 2..=252 {
        nodes.push(node_of(id, r#","parent":1"#));
    }
    nodes.insert(100, node_of(253, r#","parent":1"#));
    nodes.push(node_of(253, r#","parent":1"#));
    let error = rejection(&map_with("", &nodes.join(",")));
    assert_eq!(
        at(&error),
        (
            "nodes[253].id",
            &LoadErrorKind::DuplicateId {
                id: 253,
                first_index: 100
            }
        )
    );
    assert!(
        error
            .to_string()
            .contains("another node, nodes[100], already has the id 253"),
        "{error}"
    );
    // Control: renumbering one of them loads.
    nodes.pop();
    nodes.push(node_of(254, r#","parent":1"#));
    assert!(load(&map_with("", &nodes.join(","))).is_ok());
}

// ---- context and place ----------------------------------------------

#[test]
fn test_an_error_names_its_node_by_id_and_excerpt() {
    // A first span longer than the excerpt, whose 32nd grapheme is a
    // letter with a combining accent, and a bidi override in it.
    let text = format!("{}\u{202E}e\u{301}tail", "a".repeat(30));
    let json = map_with(
        "",
        &format!(
            r#"{},{}"#,
            node_of(3, r#","sections":[{"spans":[{"text":"plain"}]}]"#),
            node_of(
                7,
                &format!(r#","fill":"var(nope)","sections":[{{"spans":[{{"text":"{text}"}}]}}]"#)
            ),
        ),
    );
    let error = rejection(&json);
    assert_eq!(error.path, "nodes[1].fill");
    let context = error.node.clone().expect("an error in a node names it");
    assert_eq!((context.index, context.id), (1, Some(7)));
    assert_eq!(
        context.excerpt.as_deref(),
        Some(format!("{}\\u202ee\u{301}…", "a".repeat(30)).as_str())
    );
    // The lowest-channel section's first span names a node, whatever
    // the order of the keys inside it.
    let json = map_with(
        "",
        &node_of(
            4,
            r#","fill":"var(nope)","sections":[{"spans":[{"text":"first"}]},{"channel":1,"spans":[{"text":"second"}]}]"#,
        ),
    );
    assert_eq!(
        rejection(&json).node.and_then(|node| node.excerpt).as_deref(),
        Some("first")
    );
    // A ring's text, when there is no section.
    let json = map_with(
        "",
        &node_of(
            4,
            r#","fill":"var(nope)","frame":{"rings":[{"spans":[{"text":"around"}]}]}"#,
        ),
    );
    assert_eq!(
        rejection(&json).node.and_then(|node| node.excerpt).as_deref(),
        Some("around")
    );
    // Control: a node with no text has no excerpt.
    let json = map_with("", &node_of(4, r#","fill":"var(nope)""#));
    assert_eq!(rejection(&json).node.map(|node| node.excerpt), Some(None));
}

#[test]
fn test_a_node_that_does_not_read_is_still_named_by_its_id() {
    // An unknown key deep in a frame: the node's DTO never reads, so its
    // id and text are read on their own.
    let json = map_with(
        "",
        &node_of(
            70211,
            r#","sections":[{"spans":[{"text":"In the beginning"}]}],"frame":{"rings":[{"bogus":1}]}"#,
        ),
    );
    let error = rejection(&json);
    assert_eq!(
        at(&error),
        (
            "nodes[0].frame.rings[0].bogus",
            &LoadErrorKind::UnknownKey {
                key: "bogus".to_string()
            }
        )
    );
    let context = error.node.expect("the node is named");
    assert_eq!(
        (context.id, context.excerpt.as_deref()),
        (Some(70211), Some("In the beginning"))
    );
}

#[test]
fn test_an_error_carries_its_line_exact_or_the_line_of_its_node() {
    let tree = fixture("tree");
    // The nodes start on line 47; node 3 is on line 50.
    let line_of = |id: u32| {
        1 + tree
            .lines()
            .position(|line| line.starts_with(&format!("    {{\"id\": {id},")))
            .unwrap()
    };
    assert_eq!((line_of(1), line_of(3)), (47, 50));
    // A reader's error, an unknown key in node 3's line: its line, and
    // the column of the key.
    let bad = tree.replace(
        r#"{"id": 3, "parent": 1, "#,
        r#"{"id": 3, "parent": 1, "bogus": 0, "#,
    );
    let error = rejection(&bad);
    assert_eq!(error.path, "nodes[3].bogus");
    let position = error.position.expect("a reader's error has a place");
    assert_eq!(position.line, 50);
    let key_at = bad.lines().nth(49).unwrap().find("\"bogus\"").unwrap() as u32 + 1;
    // The reader stops at the key's closing quote, the seventh byte of
    // `"bogus"`: the column in the whole line, not in the node's extent,
    // which starts four bytes in.
    assert_eq!(position.column, key_at + 6, "the column of the key's end");
    assert!(
        error.to_string().ends_with(r#"(node 3 "the earth", line 50)"#),
        "{error}"
    );
    // Control: the same error in the first node reports line 47.
    let bad = tree.replace(r#"{"id": 1, "#, r#"{"id": 1, "bogus": 0, "#);
    assert_eq!(rejection(&bad).position.map(|p| p.line), Some(47));
    // An error found after the node was read: the line of its node.
    let bad = tree.replace(
        r#""text": "the earth"}"#,
        r#""text": "the earth", "color": "var(nope)"}"#,
    );
    let error = rejection(&bad);
    assert_eq!(error.path, "nodes[3].sections[0].spans[0].color");
    assert_eq!(error.position.map(|p| (p.line, p.column)), Some((50, 5)));
    // In a top-level value: the start of the value.
    let bad = tree.replace(r##""fill": "#2a1b3d""##, r#""fill": "var(nope)""#);
    let error = rejection(&bad);
    assert_eq!(error.path, "palettes.rainbow[0].fill");
    assert_eq!(error.position.map(|p| (p.line, p.column)), Some((19, 15)));
}

// ---- which error is first -------------------------------------------

#[test]
fn test_which_of_several_errors_is_reported_is_fixed() {
    let bad_var = r#","fill":"var(nope)""#;
    // Syntax anywhere beats meaning anywhere: a broken end, a bad first
    // node.
    let json = list(&[(1, bad_var)]);
    let truncated = &json[..json.len() - 1];
    assert!(matches!(rejection(truncated).kind, LoadErrorKind::Syntax { .. }));
    // Control: whole, the node's error is the one.
    assert_eq!(rejection(&json).path, "nodes[0].fill");

    // A newer format is reported before the keys it may define.
    let newer = r#"{"format":2,"name":"t","buttons":[]}"#;
    assert_eq!(
        at(&rejection(newer)),
        (
            "format",
            &LoadErrorKind::UnsupportedFormat { found: 2, newest: 1 }
        )
    );
    // Control: the same keys under format 1 are refused by name.
    let current = r#"{"format":1,"name":"t","buttons":[]}"#;
    assert_eq!(
        at(&rejection(current)),
        (
            "buttons",
            &LoadErrorKind::UnknownKey {
                key: "buttons".to_string()
            }
        )
    );

    // A top-level value beats a node, wherever it sits in the text.
    let json = format!(
        r#"{{"nodes":[{}],"format":1,"name":"t","palettes":{{"p":[{{"fill":"var(x)","frame":"var(x)","text":"var(x)"}}]}}}}"#,
        node_of(1, bad_var)
    );
    assert_eq!(rejection(&json).path, "palettes.p[0].fill");

    // Within a node, its structure beats its content.
    let json = list(&[(1, ""), (2, &format!(r#","parent":9{bad_var}"#))]);
    assert_eq!(rejection(&json).path, "nodes[1].parent");
    // Control: with the parent fixed, the content's error is next.
    let json = list(&[(1, ""), (2, &format!(r#","parent":1{bad_var}"#))]);
    assert_eq!(rejection(&json).path, "nodes[1].fill");

    // Links come last: a dangling link in the first node waits for the
    // second node's error.
    let json = list(&[(1, r#","links":[{"to":9,"kind":"line"}]"#), (2, bad_var)]);
    assert_eq!(rejection(&json).path, "nodes[1].fill");
    // Control: with the second node fixed, the link's error is next,
    // named by its node and placed at its line.
    let error = rejection(&list(&[(1, r#","links":[{"to":9,"kind":"line"}]"#), (2, "")]));
    assert_eq!(
        at(&error),
        ("nodes[0].links[0].to", &LoadErrorKind::DanglingLink { to: 9 })
    );
    assert_eq!(error.node.as_ref().and_then(|node| node.id), Some(1));
    assert!(error.position.is_some());
}

// ---- the text ---------------------------------------------------------

#[test]
fn test_a_byte_order_mark_is_refused_and_crlf_is_read_and_written_as_lf() {
    let text = fixture("tree");
    let with_bom = format!("\u{FEFF}{text}");
    for error in [
        rejection(&with_bom),
        load_from_bytes(with_bom.as_bytes()).unwrap_err(),
    ] {
        assert_eq!(at(&error), ("", &LoadErrorKind::ByteOrderMark));
        assert_eq!(error.position, None);
    }
    // Control: without it, the same text loads.
    assert!(load_from_bytes(text.as_bytes()).is_ok());
    // CR LF is whitespace: it reads, and the saver writes LF.
    let crlf = text.replace('\n', "\r\n");
    assert!(crlf.contains("\r\n"));
    let doc = load(&crlf).expect("CR LF is whitespace");
    let saved = written(&doc);
    assert!(!saved.contains('\r'));
    assert_eq!(saved, text);
}

#[test]
fn test_every_proper_prefix_of_a_map_is_refused() {
    // A map cut anywhere is refused; there is no partial document. A cut
    // that removes only trailing whitespace is the same text.
    let mut cuts = 0;
    for (name, text) in FIXTURES {
        let end = text.trim_end().len();
        for cut in (0..end).filter(|cut| text.is_char_boundary(*cut)) {
            assert!(load(&text[..cut]).is_err(), "{name} cut at byte {cut} loaded");
            cuts += 1;
        }
        // Control: the whole text loads, and so does it without its
        // trailing newline.
        assert!(load(text).is_ok(), "{name}");
        assert!(load(&text[..end]).is_ok(), "{name}");
    }
    assert!(cuts > 10_000, "precondition: {cuts} prefixes");
}

#[test]
fn test_a_node_has_the_same_line_in_a_saved_map_as_its_error_says() {
    // The saver writes one node per line, so the line an error names for
    // a node is where `sed -n Np` finds it in a saved map.
    let doc = load(fixture("tree")).unwrap();
    let text = written(&doc);
    let (three, _) = node(&doc, 3);
    assert!(doc.get(three).is_some());
    let bad = text.replace(
        r#""text": "the earth"}"#,
        r#""text": "the earth", "color": "var(nope)"}"#,
    );
    let line = rejection(&bad).position.unwrap().line as usize;
    assert!(text
        .lines()
        .nth(line - 1)
        .unwrap()
        .starts_with(r#"    {"id": 3,"#));
}
