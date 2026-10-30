// SPDX-License-Identifier: MPL-2.0

//! The loader's fuzz properties held to account (TEST_CONVENTIONS
//! §T14): each property shown to go red on a planted violation, the
//! structure-aware generator swept under a seeded source until it has
//! reached every case, and every fuzz finding replayed.

use crate::document::element::DocElement;
use crate::document::format::error::{LoadError, LoadErrorKind, NodeContext, TextPosition};
use crate::document::format::{load, save};
use crate::document::ids::{get, Document, IdentIndex};
use crate::document::tests::format_fuzz::{
    bounds_hold, check_generated, check_load_bytes, check_refusal, excerpt_length, node_lines, oracle,
    path_resolves, reference, saved_text_is_clean, NodeSpec,
};
use crate::document::tests::format_support::fixture;
use crate::document::values::{FitMode, FontSize};
use crate::util::tests::choices::Choices;
use serde_json::json;
use std::collections::BTreeMap;

/// Every minimized input a fuzz target found, by the symptom it had.
/// Each is replayed through every property here; its own test below
/// pins the symptom. The fuzz targets read these files as seeds.
const FINDINGS: [(&str, &[u8]); 2] = [
    (
        "c1_control_saved_raw",
        include_bytes!("fuzz_findings/c1_control_saved_raw.mandala.json"),
    ),
    (
        "syntax_error_in_a_node_has_no_node_context",
        include_bytes!("fuzz_findings/syntax_error_in_a_node_has_no_node_context.mandala.json"),
    ),
];

/// The second finding: malformed text inside `nodes[1]` was refused at
/// the path `nodes[1]` with no node context, where every error inside a
/// node carries one (§7.2) — the index pass, which finds every syntax
/// error, never attached it. The node's text is not JSON, so the context
/// holds its index alone.
#[test]
fn test_a_syntax_error_inside_a_node_carries_the_nodes_context() {
    let text = std::str::from_utf8(FINDINGS[1].1).unwrap();
    let error = load(text).unwrap_err();
    assert!(matches!(error.kind, LoadErrorKind::Syntax { .. }), "{error:?}");
    assert_eq!(error.path, "nodes[1]");
    assert_eq!(
        error.node,
        Some(NodeContext {
            index: 1,
            id: None,
            excerpt: None,
        })
    );
    // Control: a syntax error outside the nodes is inside no node.
    let error = load(r#"{"format":1,"name" "m","nodes":[]}"#).unwrap_err();
    assert!(matches!(error.kind, LoadErrorKind::Syntax { .. }), "{error:?}");
    assert_eq!(error.node, None, "{error:?}");
}

#[test]
fn test_every_loader_fuzz_finding_holds_every_property() {
    assert!(!FINDINGS.is_empty(), "precondition: the findings are read");
    for (name, bytes) in FINDINGS {
        check_load_bytes(bytes).unwrap_or_else(|why| panic!("{name}: {why}"));
    }
}

/// The finding of #278's first run (R23): a C1 control in a map was
/// written raw by the saver, which escaped U+0085 alone of the C1
/// controls. U+009B is CSI in a UTF-8 terminal, so `cat` or `git diff`
/// of such a map could act on the terminal.
#[test]
fn test_a_c1_control_in_a_map_is_saved_escaped() {
    let finding = FINDINGS[0].1;
    let text = std::str::from_utf8(finding).unwrap();
    assert!(
        text.contains('\u{95}'),
        "precondition: the finding holds U+0095 raw"
    );
    let saved = save(&load(text).unwrap()).unwrap();
    assert!(!saved.contains('\u{95}'), "written raw:\n{saved}");
    assert!(saved.contains(r#""content": "\u0095""#), "{saved}");
    // DEL and every C1 control, at both ends of the range and CSI; and,
    // as the control, the first character past them, which stays raw.
    for (c, escape) in [
        ('\u{7f}', "\\u007f"),
        ('\u{80}', "\\u0080"),
        ('\u{9b}', "\\u009b"),
        ('\u{9f}', "\\u009f"),
    ] {
        let map = format!(r#"{{"format":1,"name":"a{c}b"}}"#);
        let saved = save(&load(&map).unwrap()).unwrap();
        assert!(!saved.contains(c) && saved.contains(escape), "{escape}:\n{saved}");
        let reloaded = load(&saved).unwrap();
        assert!(
            matches!(reloaded.get(reloaded.root()), Some(DocElement::Map(map)) if map.name == format!("a{c}b")),
            "{escape} reads back as the character"
        );
    }
    let saved = save(&load("{\"format\":1,\"name\":\"a\u{a0}b\"}").unwrap()).unwrap();
    assert!(
        saved.contains('\u{a0}'),
        "U+00A0 is no control and is written raw:\n{saved}"
    );
}

#[test]
fn test_generated_maps_hold_every_loader_property() {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for seed in 0..1000 {
        let mut choices = Choices::seeded(seed);
        match check_generated(&mut choices, 64) {
            Ok(outcome) => *seen.entry(outcome).or_default() += 1,
            Err(why) => panic!("seed {seed}: {why}"),
        }
    }
    // The sweep proves something only if it reached every case: a valid
    // map of each origin, each injected fault, and each refusal the
    // reference model names.
    for outcome in [
        "none: loaded",
        "structure: loaded",
        "content: Bound",
        "content: Invalid",
        "content: SiblingsOutOfOrder",
        "content: UnknownKey",
        "structure: Bound",
        "structure: ConnectorOnRoot",
        "structure: DanglingLink",
        "structure: DuplicateId",
        "structure: ParentIsSelf",
        "structure: ParentNotEarlier",
        "structure: ParentNotOnChain",
        "structure: SelfLink",
        "structure: SiblingsOutOfOrder",
    ] {
        assert!(
            seen.contains_key(outcome),
            "precondition: the sweep never reached `{outcome}`: {seen:?}"
        );
    }
}

#[test]
fn test_the_saved_text_property_refuses_every_character_that_may_not_stand_raw() {
    for c in [
        '\u{1}', '\u{1b}', '\r', '\u{7f}', '\u{80}', '\u{85}', '\u{9b}', '\u{9f}', '\u{2028}', '\u{2029}',
        '\u{202a}', '\u{202e}', '\u{2066}', '\u{2069}',
    ] {
        let text = format!("{{\"name\": \"a{c}b\"}}\n");
        assert!(
            saved_text_is_clean(&text).is_err(),
            "U+{:04X} passed",
            u32::from(c)
        );
    }
    // Controls: what may stand raw does, and the ending is checked.
    assert_eq!(
        saved_text_is_clean("{\n  \"name\": \"\u{a0}é\u{5d0}\u{1F469}\"\n}\n"),
        Ok(())
    );
    assert!(saved_text_is_clean("{}").is_err(), "no line feed at the end");
    assert!(
        saved_text_is_clean("{}\n\n").is_err(),
        "two line feeds at the end"
    );
}

#[test]
fn test_the_path_property_resolves_exactly_what_the_input_holds() {
    let value = json!({
        "nodes": [{"id": 1}, {"id": 2, "sections": [{"spans": []}]}],
        "palettes": {"a.b": [{"fill": 1}], "\u{202e}": []},
    });
    for path in [
        "",
        "nodes",
        "nodes[1].sections[0].spans",
        r#"palettes["a.b"][0].fill"#,
        "palettes[\"\\u202e\"]",
    ] {
        assert!(path_resolves(&value, path), "`{path}` resolves");
    }
    for path in [
        "nodes[2]",
        "nodes[1].sections[1]",
        "nodes.id",
        "palettes.a.b",
        r#"palettes["a.c"]"#,
        "nodes[",
        ".nodes",
        "nodes[1]..id",
    ] {
        assert!(!path_resolves(&value, path), "`{path}` does not resolve");
    }
}

#[test]
fn test_the_excerpt_property_measures_the_text_not_its_escapes() {
    assert_eq!(
        excerpt_length("a\\u0001\\\"b"),
        Some(4),
        "four characters, escaped"
    );
    let cut = format!("{}…", "a".repeat(256));
    assert_eq!(excerpt_length(&cut), Some(256), "the cut mark is not text");
    assert_eq!(excerpt_length(&"a".repeat(257)), Some(257));
    assert_eq!(
        excerpt_length("a\"b"),
        None,
        "an unescaped quote is not string content"
    );
}

/// A refusal of the fixture `tree` with `path`, `node` and `position`.
fn refusal(path: &str, node: Option<NodeContext>, position: Option<TextPosition>) -> LoadError {
    LoadError {
        path: path.to_string(),
        node,
        position,
        kind: LoadErrorKind::SelfLink,
    }
}

fn context(index: usize, excerpt: Option<String>) -> Option<NodeContext> {
    Some(NodeContext {
        index,
        id: Some(1),
        excerpt,
    })
}

#[test]
fn test_the_refusal_property_finds_each_way_a_refusal_can_be_wrong() {
    let text = fixture("tree");
    let bytes = text.as_bytes();
    let first_line = node_lines(text).expect("the fixture's nodes read")[0].0;
    let line = |line| Some(TextPosition { line, column: 1 });
    // The control: a well-formed refusal at node 0's opening line.
    let good = refusal(
        "nodes[0].links",
        context(0, Some("text".into())),
        line(first_line),
    );
    assert_eq!(
        check_refusal(&good, bytes),
        Ok(()),
        "precondition: the control passes"
    );
    let cases = [
        (refusal("nodes[0].links\u{1b}[2J", context(0, None), None), "raw"),
        (
            refusal("nodes[0].nope", context(0, None), None),
            "does not resolve",
        ),
        (refusal("nodes[1].id", context(0, None), None), "does not agree"),
        (refusal("nodes[0].id", None, None), "does not agree"),
        (refusal("format", context(0, None), None), "does not agree"),
        (
            refusal("nodes[0]", context(0, Some("a".repeat(257))), None),
            "257 bytes",
        ),
        (
            refusal("nodes[0]", context(0, None), line(1)),
            "outside its node's lines",
        ),
    ];
    for (error, why) in cases {
        let found = check_refusal(&error, bytes).expect_err(why);
        assert!(found.contains(why), "{why}: {found}");
    }
}

/// `doc` with `change` made to its first element `pick` accepts, as a
/// document whose index is rebuilt from its tree.
fn changed(
    doc: &Document,
    pick: impl Fn(&DocElement) -> bool,
    change: impl FnOnce(&mut DocElement),
) -> Document {
    let mut tree = doc.tree().clone();
    let id = tree
        .descendants()
        .find(|id| get(&tree, *id).is_some_and(&pick))
        .expect("precondition: the fixture holds the element");
    tree.update(id, change).unwrap();
    let mut highest = 0;
    let index: IdentIndex = tree
        .descendants()
        .filter_map(|id| match get(&tree, id) {
            Some(DocElement::Node(node)) => {
                highest = highest.max(node.ident.0);
                Some((node.ident, id))
            }
            _ => None,
        })
        .collect();
    Document::from_loaded(tree, doc.definitions().clone(), index, highest).unwrap()
}

#[test]
fn test_the_bounds_property_finds_a_value_past_its_bound_in_each_kind_of_field() {
    let doc = load(fixture("full")).unwrap();
    assert_eq!(
        bounds_hold(&doc),
        Ok(()),
        "precondition: the fixture is in bounds"
    );
    let node = |e: &DocElement| matches!(e, DocElement::Node(_));
    let span = |e: &DocElement| matches!(e, DocElement::Span(_));
    let link = |e: &DocElement| matches!(e, DocElement::Link(_));
    let defaults = |e: &DocElement| matches!(e, DocElement::Defaults(_));
    let cases: Vec<(&str, Document)> = vec![
        (
            "opacity",
            changed(&doc, node, |e| {
                if let DocElement::Node(n) = e {
                    n.opacity = 1.5;
                }
            }),
        ),
        (
            "position",
            changed(&doc, node, |e| {
                if let DocElement::Node(n) = e {
                    n.position = glam::Vec2::new(0.0, f32::INFINITY);
                }
            }),
        ),
        (
            "size",
            changed(&doc, span, |e| {
                if let DocElement::Span(s) = e {
                    s.style.size = Some(FontSize::Pt(0.1));
                }
            }),
        ),
        (
            "control_points",
            changed(&doc, link, |e| {
                if let DocElement::Link(l) = e {
                    l.path.control_points = vec![glam::Vec2::new(2.0e9, 0.0)];
                }
            }),
        ),
        (
            "gap",
            changed(&doc, defaults, |e| {
                if let DocElement::Defaults(d) = e {
                    d.frame.gap = Some(-1.0);
                }
            }),
        ),
        (
            "fit",
            changed(&doc, defaults, |e| {
                if let DocElement::Defaults(d) = e {
                    d.link.fit = Some(FitMode::Fit { min: 0.0, max: 1.0 });
                }
            }),
        ),
    ];
    for (field, changed) in cases {
        let found = bounds_hold(&changed).expect_err(field);
        assert!(found.contains(&format!("`{field}`")), "{field}: {found}");
    }
}

/// A node of the model: `id` under `parent`.
fn spec(id: i64, parent: Option<i64>, channel: i64) -> NodeSpec {
    NodeSpec {
        id,
        parent,
        channel,
        links: Vec::new(),
        content: Vec::new(),
        link_tails: Vec::new(),
    }
}

/// `chain_ref.py`'s named cases, against both models: the oracle's
/// verdict and the reference's refusal.
#[test]
fn test_the_structure_models_judge_the_named_cases_of_chain_ref() {
    let cases: [(&str, Vec<NodeSpec>, Option<(&str, &str)>); 8] = [
        (
            "valid",
            vec![spec(1, None, 0), spec(2, Some(1), 0), spec(3, Some(1), 1)],
            None,
        ),
        (
            "self parent",
            vec![spec(1, None, 0), spec(2, Some(2), 0)],
            Some(("nodes[1].parent", "ParentIsSelf")),
        ),
        (
            "forward parent",
            vec![spec(1, None, 0), spec(2, Some(3), 0), spec(3, Some(1), 0)],
            Some(("nodes[1].parent", "ParentNotEarlier")),
        ),
        (
            "closed subtree",
            vec![
                spec(1, None, 0),
                spec(2, Some(1), 0),
                spec(3, Some(1), 0),
                spec(4, Some(2), 0),
            ],
            Some(("nodes[3].parent", "ParentNotOnChain")),
        ),
        (
            "channel order",
            vec![spec(1, None, 0), spec(2, Some(1), 1), spec(3, Some(1), 0)],
            Some(("nodes[2]", "SiblingsOutOfOrder")),
        ),
        (
            "root channel order",
            vec![spec(1, None, 2), spec(2, None, 1)],
            Some(("nodes[1].channel", "SiblingsOutOfOrder")),
        ),
        (
            "duplicate",
            vec![spec(1, None, 0), spec(2, Some(1), 0), spec(2, Some(1), 0)],
            Some(("nodes[2].id", "DuplicateId")),
        ),
        (
            "cycle attempt",
            vec![spec(1, Some(2), 0), spec(2, Some(1), 0)],
            Some(("nodes[0].parent", "ParentNotEarlier")),
        ),
    ];
    for (name, nodes, expected) in cases {
        assert_eq!(oracle(&nodes), expected.is_none(), "{name}: the oracle");
        let got = reference(&nodes).map(|(path, kind, _)| (path, kind));
        assert_eq!(
            got,
            expected.map(|(path, kind)| (path.to_string(), kind)),
            "{name}: the reference"
        );
    }
}
