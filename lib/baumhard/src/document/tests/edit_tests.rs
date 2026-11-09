// SPDX-License-Identifier: MPL-2.0

//! Edits (V1-D1, #250): what an edit reaches, what it writes, what a
//! load refuses, how its names resolve, and that a handler only reads.
//! Each assertion has a control beside it that would make it fail.

use crate::arena::NodeId;
use crate::document::bounds::{MAX_ANIMATE_MS, MAX_DEFINITION_DEPTH, MAX_DEFINITION_PARTS};
use crate::document::edit::{parse_edits, EditDef, EditError};
use crate::document::element::{DocElement, DocKind};
use crate::document::format::{load, LoadErrorKind};
use crate::document::ids::Document;
use crate::document::mutation::validate::validate;
use crate::document::resolve::resolve_text;
use crate::document::tests::edit_fuzz::Boxes;
use crate::document::tests::format_support::{kid, kids, node};
use crate::document::values::{ColorRef, FontSize, VarId};
use crate::document::writer::{ChangeSource, EditRequest};
use crate::edit_handlers::{registry, Handlers};
use glam::Vec2;

/// One edit, from the JSON of its definition.
fn edit(json: &str) -> EditDef {
    let file = format!(r#"{{"edits":[{json}]}}"#);
    let mut edits = parse_edits(&file).unwrap_or_else(|error| panic!("{json} reads: {error}"));
    edits.remove(0)
}

/// The refusal of the edits file holding `json`.
fn refusal(json: &str) -> (String, LoadErrorKind) {
    match parse_edits(&format!(r#"{{"edits":[{json}]}}"#)) {
        Ok(_) => panic!("{json} read, and should not have"),
        Err(error) => (error.path, error.kind),
    }
}

/// A map to reach into. Node 1 (top level) has two sections — spans
/// "a" (30 pt) and "b" (inheriting), and "c" — a frame with two rings, a
/// line link to node 5 with a label, and children 2 and 3; node 2 has a
/// connector with a label, and child 4; node 5 is the other top-level
/// node.
const MAP: &str = r##"{"format":1,"name":"t","theme":{"active":"d","variants":{"d":{"accent":"#ff0000","bg":"#000000"}}},
"verbs":[{"id":"go","name":"Go"}],
"nodes":[
{"id":1,"position":[0,0],"size":20.0,"sections":[{"spans":[{"text":"a","size":30.0},{"text":"b"}]},{"channel":1,"spans":[{"text":"c"}]}],
 "frame":{"rings":[{"spans":[{"text":"r0"}]},{"channel":1,"spans":[{"text":"r1"}]}]},
 "links":[{"to":5,"kind":"line","label":{"spans":[{"text":"l"}]}}]},
{"id":2,"parent":1,"position":[300,0],"sections":[{"spans":[{"text":"two"}]}],"connector":{"label":{"spans":[{"text":"k"}]}}},
{"id":4,"parent":2,"position":[600,0],"sections":[{"spans":[{"text":"four"}]}]},
{"id":3,"parent":1,"channel":1,"position":[300,200],"sections":[{"spans":[{"text":"three"}]}]},
{"id":5,"channel":1,"position":[900,0],"sections":[{"spans":[{"text":"five"}]}]}
]}"##;

fn map() -> Document {
    load(MAP).unwrap_or_else(|error| panic!("the test map loads: {error}"))
}

/// The node with `ident`.
fn n(doc: &Document, ident: u32) -> NodeId {
    node(doc, ident).0
}

/// The elements an edit of `reach` and `target`, with `filter`, reaches
/// from the nodes `idents`.
fn reached(doc: &Document, reach: &str, target: &str, filter: &str, anchors: &[NodeId]) -> Vec<NodeId> {
    let def = edit(&format!(
        r#"{{"id":"e","name":"E","reach":"{reach}","target":"{target}"{filter},"mutations":[]}}"#
    ));
    def.elements(doc, anchors).unwrap()
}

#[test]
fn test_each_reach_names_its_nodes() {
    let doc = map();
    let ids = |idents: &[u32]| idents.iter().map(|&i| n(&doc, i)).collect::<Vec<_>>();
    let two = [n(&doc, 2)];
    for (reach, expected) in [
        ("self", ids(&[2])),
        ("children", ids(&[4])),
        ("descendants", ids(&[4])),
        ("subtree", ids(&[2, 4])),
        ("parent", ids(&[1])),
        ("siblings", ids(&[3])),
    ] {
        assert_eq!(
            reached(&doc, reach, "node", "", &two),
            expected,
            "{reach} from node 2"
        );
    }
    // Controls from another node: each answer changes, so none of the
    // rows above passes for a reach that ignores its anchor.
    let one = [n(&doc, 1)];
    assert_eq!(reached(&doc, "children", "node", "", &one), ids(&[2, 3]));
    assert_eq!(reached(&doc, "descendants", "node", "", &one), ids(&[2, 4, 3]));
    let five = [n(&doc, 5)];
    assert_eq!(
        reached(&doc, "parent", "node", "", &five),
        ids(&[]),
        "a top-level node has no parent"
    );
    assert_eq!(
        reached(&doc, "siblings", "node", "", &five),
        ids(&[1]),
        "top-level nodes are siblings"
    );
}

#[test]
fn test_each_target_names_its_elements() {
    let doc = map();
    let one = n(&doc, 1);
    let two = n(&doc, 2);
    let sections: Vec<NodeId> = kids(&doc, one, DocKind::Section)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let spans: Vec<NodeId> = sections
        .iter()
        .flat_map(|&s| kids(&doc, s, DocKind::Span).into_iter().map(|(id, _)| id))
        .collect();
    let frame = kid(&doc, one, DocKind::Frame, 0).0;
    let rings: Vec<NodeId> = kids(&doc, frame, DocKind::Ring)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let link = kid(&doc, one, DocKind::Link, 0).0;
    let link_label = kid(&doc, link, DocKind::Label, 0).0;
    let connector = kid(&doc, two, DocKind::Connector, 0).0;
    let connector_label = kid(&doc, connector, DocKind::Label, 0).0;
    for (target, anchor, expected) in [
        ("node", one, vec![one]),
        ("sections", one, sections.clone()),
        ("spans", one, spans.clone()),
        ("frame", one, vec![frame]),
        ("rings", one, rings.clone()),
        ("connector", two, vec![connector]),
        ("links", one, vec![link]),
        ("labels", one, vec![link_label]),
        ("labels", two, vec![connector_label]),
    ] {
        assert_eq!(reached(&doc, "self", target, "", &[anchor]), expected, "{target}");
    }
    assert_eq!(spans.len(), 3, "precondition: the two sections' spans");
    assert_eq!(rings.len(), 2, "precondition: the frame's rings");
    // Controls: a node without the element answers nothing, and an
    // anchor inside a node stands for that node.
    assert_eq!(
        reached(&doc, "self", "connector", "", &[one]),
        Vec::<NodeId>::new()
    );
    assert_eq!(reached(&doc, "self", "links", "", &[two]), Vec::<NodeId>::new());
    assert_eq!(reached(&doc, "self", "node", "", &[spans[2]]), vec![one]);
}

#[test]
fn test_the_elements_are_each_once_in_tree_order() {
    let doc = map();
    let (one, two, four) = (n(&doc, 1), n(&doc, 2), n(&doc, 4));
    let span_of_two = kid(&doc, kid(&doc, two, DocKind::Section, 0).0, DocKind::Span, 0).0;
    // Overlapping anchors, given out of order: node 4 inside node 2's
    // subtree, node 2 twice (itself and through its span).
    let got = reached(&doc, "subtree", "node", "", &[four, span_of_two, two, one]);
    let three = n(&doc, 3);
    assert_eq!(got, vec![one, two, four, three]);
}

#[test]
fn test_a_filter_keeps_only_what_it_passes() {
    let doc = map();
    let one = n(&doc, 1);
    let all = reached(&doc, "self", "spans", "", &[one]);
    let large = reached(
        &doc,
        "self",
        "spans",
        r#","filter":{"fields":[[{"font_size":{"pt":25.0}},{"GreaterThan":false}]]}"#,
        &[one],
    );
    assert_eq!(all.len(), 3, "control: without the filter, every span");
    assert_eq!(large, vec![all[0]], "only the 30 pt span stores a size above 25");
    // A filter naming a theme variable resolves it against the map.
    let red = reached(
        &doc,
        "self",
        "spans",
        r#","filter":{"fields":[[{"color":"var(accent)"},{"Equals":false}]]}"#,
        &[one],
    );
    assert!(red.is_empty(), "no span stores the accent color");
}

/// Run `def` at `anchors` on `doc`, as one history entry.
fn run(doc: &mut Document, def: &EditDef, anchors: &[NodeId]) -> Result<(), EditError> {
    let request = def.request(doc, &Boxes::of(doc), &registry(), anchors)?;
    doc.apply(request, ChangeSource::Edit).unwrap();
    Ok(())
}

/// The size `span` displays, in points.
fn shown_pt(doc: &Document, span: NodeId) -> f32 {
    match resolve_text(doc.tree(), span, &mut ()).size {
        FontSize::Pt(pt) => pt,
        other => panic!("a section span shows points, not {other:?}"),
    }
}

#[test]
fn test_grow_font_grows_every_span_by_two_points_set_or_inherited() {
    let mut doc = map();
    let one = n(&doc, 1);
    let mut spans: Vec<NodeId> = Vec::new();
    for ident in [1, 2, 4, 3] {
        let node = n(&doc, ident);
        for (section, _) in kids(&doc, node, DocKind::Section) {
            spans.extend(kids(&doc, section, DocKind::Span).into_iter().map(|(id, _)| id));
        }
    }
    let before: Vec<f32> = spans.iter().map(|&s| shown_pt(&doc, s)).collect();
    // The precondition the test is about: one span sets its own size,
    // the others inherit two different ones (the node's 20, and the
    // built-in under nodes 2 to 4).
    assert_eq!(before[0], 30.0);
    assert_eq!(before[1], 20.0);
    assert_ne!(before[1], before[3]);
    let grow = edit(
        r#"{"id":"grow","name":"Grow","reach":"subtree","target":"spans","mutations":[{"span":{"text":{"grow_size":2.0}}}]}"#,
    );
    run(&mut doc, &grow, &[one]).unwrap();
    for (i, &span) in spans.iter().enumerate() {
        assert_eq!(shown_pt(&doc, span), before[i] + 2.0, "span {i}");
        let Some(DocElement::Span(data)) = doc.get(span) else { panic!("a span") };
        assert_eq!(
            data.style.size,
            Some(FontSize::Pt(before[i] + 2.0)),
            "span {i} stores it"
        );
    }
    assert_eq!(doc.history().count(), 1, "one entry for the whole subtree");
    // The node and the sections above the spans are untouched: a grow
    // that also wrote their sizes would leave sizes the spans did not
    // ask for.
    assert_eq!(node(&doc, 1).1.style.text.size, Some(FontSize::Pt(20.0)));
    for ident in [1, 2, 4, 3] {
        for (_, section) in kids(&doc, n(&doc, ident), DocKind::Section) {
            let DocElement::Section(data) = section else { panic!("a section") };
            assert_eq!(data.text.size, None, "node {ident}'s section sets no size");
        }
    }
}

#[test]
fn test_a_name_resolves_against_the_open_map() {
    let tint = edit(
        r#"{"id":"tint","name":"Tint","reach":"self","target":"spans","mutations":[{"span":{"text":{"set_color":"var(accent)"}}}]}"#,
    );
    let color_of_first_span = |doc: &Document| {
        let section = kid(doc, n(doc, 1), DocKind::Section, 0).0;
        match kid(doc, section, DocKind::Span, 0).1 {
            DocElement::Span(data) => data.style.color,
            other => panic!("{other:?}"),
        }
    };
    // Two maps whose themes list `accent` at different indices.
    let first = r##""theme":{"active":"d","variants":{"d":{"accent":"#ff0000"}}}"##;
    let second =
        r##""theme":{"active":"d","variants":{"d":{"a":"#000000","accent":"#ff0000","b":"#ffffff"}}}"##;
    for (theme, index) in [(first, 0), (second, 1)] {
        let mut doc = load(&format!(
            r#"{{"format":1,"name":"t",{theme},"nodes":[{{"id":1,"position":[0,0],"sections":[{{"spans":[{{"text":"x"}}]}}]}}]}}"#
        ))
        .unwrap();
        let one = n(&doc, 1);
        run(&mut doc, &tint, &[one]).unwrap();
        assert_eq!(
            color_of_first_span(&doc),
            Some(ColorRef::Var(VarId(index))),
            "{theme}"
        );
    }
    // A map with no `accent`: refused, naming the edit, the field and
    // the name, and nothing applied.
    let mut doc = load(r#"{"format":1,"name":"t","nodes":[{"id":1,"position":[0,0],"sections":[{"spans":[{"text":"x"}]}]}]}"#).unwrap();
    let one = n(&doc, 1);
    let revision = doc.revision();
    let error = run(&mut doc, &tint, &[one]).unwrap_err();
    assert_eq!(
        error,
        EditError::UnknownName {
            edit: "tint".to_string(),
            field: "color",
            name: "theme variable `accent`".to_string(),
        }
    );
    assert_eq!(
        error.to_string(),
        "edit `tint`: `color` names theme variable `accent`, which this map does not define"
    );
    assert_eq!(doc.revision(), revision, "nothing applied");
    assert_eq!(color_of_first_span(&doc), None);
}

#[test]
fn test_a_verb_and_a_palette_resolve_by_name() {
    let mut doc = map();
    let one = n(&doc, 1);
    let click = edit(
        r#"{"id":"c","name":"C","reach":"self","target":"node","mutations":[{"node":{"set_click":"go"}}]}"#,
    );
    run(&mut doc, &click, &[one]).unwrap();
    assert_eq!(node(&doc, 1).1.on.click.map(|verb| verb.0), Some(0));
    let unknown = edit(
        r#"{"id":"p","name":"P","reach":"self","target":"node","mutations":[{"node":{"set_palette":"warm"}}]}"#,
    );
    assert!(matches!(
        run(&mut doc, &unknown, &[one]),
        Err(EditError::UnknownName {
            field: "set_palette",
            ..
        })
    ));
}

#[test]
fn test_a_load_refuses_what_no_map_could_run() {
    let base = r#""id":"e","name":"E","reach":"self""#;
    let invalid = |json: &str| match refusal(json) {
        (path, LoadErrorKind::InvalidEdit { .. }) => path,
        other => panic!("{json}: {other:?}"),
    };
    // Exactly one body: none, or two.
    assert_eq!(invalid(&format!(r#"{{{base},"target":"node"}}"#)), "edits[0]");
    assert_eq!(
        invalid(&format!(
            r#"{{{base},"target":"node","mutations":[],"handler":"flower"}}"#
        )),
        "edits[0]"
    );
    // A mutation aimed at another kind than the target's.
    assert_eq!(
        invalid(&format!(
            r#"{{{base},"target":"spans","mutations":[{{"node":{{"set_hidden":true}}}}]}}"#
        )),
        "edits[0].mutations[0]"
    );
    // A mutator whose root is another kind than the target's, and a
    // macro whose payload is another kind than its own.
    assert_eq!(
        invalid(&format!(
            r#"{{{base},"target":"node","mutator":{{"void":{{"kind":"span","channel":0}}}}}}"#
        )),
        "edits[0].mutator"
    );
    assert_eq!(
        invalid(&format!(
            r#"{{{base},"target":"node","mutator":{{"void":{{"kind":"node","channel":0,"children":[{{"macro":{{"kind":"section","channel":0,"mutations":[{{"span":{{"set_text":"x"}}}}]}}}}]}}}}}}"#
        )),
        "edits[0].mutator.void.children[0].macro.mutations[0]"
    );
    // A handler this build lacks, and one aimed below a node.
    assert_eq!(
        invalid(&format!(r#"{{{base},"target":"node","handler":"spiral"}}"#)),
        "edits[0].handler"
    );
    assert_eq!(
        invalid(&format!(r#"{{{base},"target":"spans","handler":"flower"}}"#)),
        "edits[0].target"
    );
    // A face no map could name, refused when it is read.
    assert_eq!(
        refusal(&format!(
            r#"{{{base},"target":"spans","mutations":[{{"span":{{"text":{{"set_font":"Nope Sans"}}}}}}]}}"#
        )),
        (
            "edits[0].mutations[0]".to_string(),
            LoadErrorKind::UnknownFont {
                family: "Nope Sans".to_string()
            }
        )
    );
    // An authored edit holds no structure: there is no spelling for a
    // move or an insert, so either is an unknown variant, at its path.
    for op in [
        r#"{"node":{"move":{"to":1}}}"#,
        r#"{"link":{"set_kind":"portal"}}"#,
    ] {
        let (path, kind) = refusal(&format!(r#"{{{base},"target":"node","mutations":[{op}]}}"#));
        assert!(path.starts_with("edits[0].mutations[0]"), "{op}: {path}");
        assert!(matches!(kind, LoadErrorKind::Invalid { .. }), "{op}: {kind:?}");
    }
    // Control: the same shapes, written right, read.
    edit(&format!(
        r#"{{{base},"target":"node","mutations":[{{"node":{{"set_hidden":true}}}}]}}"#
    ));
    edit(&format!(r#"{{{base},"target":"node","handler":"cascade"}}"#));
    edit(&format!(
        r#"{{{base},"target":"spans","mutations":[{{"span":{{"text":{{"set_font":"Novem"}}}}}}]}}"#
    ));
}

/// **An animation takes at most a minute, and waits at most one**
/// (review of #331): `ms` and `delay_ms` at [`MAX_ANIMATE_MS`] read, and
/// one past it is refused as a bound at its path.
///
/// Control: with no bound, `4294967295` read — 49.7 days.
#[test]
fn test_an_animation_and_its_delay_are_at_most_a_minute() {
    let with = |animate: String| {
        format!(
            r#"{{"id":"e","name":"E","reach":"self","target":"node","handler":"flower","animate":{{{animate}}}}}"#
        )
    };
    for key in ["ms", "delay_ms"] {
        let at = |ms: u32| {
            let other = if key == "ms" { "" } else { r#""ms":1,"# };
            with(format!(r#"{other}"{key}":{ms}"#))
        };
        edit(&at(MAX_ANIMATE_MS));
        let (path, kind) = refusal(&at(MAX_ANIMATE_MS + 1));
        assert_eq!(path, format!("edits[0].animate.{key}"));
        assert!(matches!(kind, LoadErrorKind::Bound { .. }), "{key}: {kind:?}");
    }
    let (path, _) = refusal(&with(r#""ms":4294967295"#.to_string()));
    assert_eq!(path, "edits[0].animate.ms");
}

/// **A reach, a target, an easing and a snap are named as the file
/// spells them** (review of #331): each `spelling` reads back as itself.
///
/// Control: spelled as `Debug` spells them (`SelfNode`), none read.
#[test]
fn test_each_spelling_reads_back_as_itself() {
    use crate::document::edit::{Easing, Reach, Target};
    use crate::engine::interpolate::Snap;
    use Reach::*;
    for reach in [SelfNode, Children, Descendants, Subtree, Parent, Siblings] {
        let json = format!(
            r#"{{"id":"e","name":"E","reach":"{}","target":"node","mutations":[]}}"#,
            reach.spelling()
        );
        assert_eq!(edit(&json).reach, reach);
    }
    use Target::*;
    for target in [Node, Sections, Spans, Frame, Rings, Connector, Links, Labels] {
        let json = format!(
            r#"{{"id":"e","name":"E","reach":"self","target":"{}","mutations":[]}}"#,
            target.spelling()
        );
        assert_eq!(edit(&json).target, target);
    }
    use Easing::*;
    for (easing, snap) in [
        (Linear, Snap::Start),
        (EaseIn, Snap::End),
        (EaseOut, Snap::Start),
        (EaseInOut, Snap::End),
    ] {
        let json = format!(
            r#"{{"id":"e","name":"E","reach":"self","target":"node","mutations":[],"animate":{{"ms":1,"easing":"{}","snap":"{}"}}}}"#,
            easing.spelling(),
            snap.spelling()
        );
        let animate = edit(&json).animate.expect("animated");
        assert_eq!((animate.easing, animate.snap), (easing, snap));
    }
}

#[test]
fn test_an_omitted_snap_reads_as_end() {
    use crate::engine::interpolate::Snap;
    let def =
        edit(r#"{"id":"e","name":"E","reach":"self","target":"node","mutations":[],"animate":{"ms":10}}"#);
    assert_eq!(def.animate.map(|a| a.snap), Some(Snap::End));
    let def = edit(
        r#"{"id":"e","name":"E","reach":"self","target":"node","mutations":[],"animate":{"ms":10,"snap":"start"}}"#,
    );
    assert_eq!(
        def.animate.map(|a| a.snap),
        Some(Snap::Start),
        "control: a written snap is kept"
    );
}

/// A mutator of `depth` levels: `void` nodes, each the only child of
/// the one above.
fn nested_mutator(depth: usize) -> String {
    let mut json = r#"{"void":{"kind":"node","channel":0}}"#.to_string();
    for _ in 1..depth {
        json = format!(r#"{{"void":{{"kind":"node","channel":0,"children":[{json}]}}}}"#);
    }
    json
}

#[test]
fn test_a_definition_nests_at_most_thirty_two_levels() {
    let at = |depth: usize| {
        format!(
            r#"{{"id":"e","name":"E","reach":"self","target":"node","mutator":{}}}"#,
            nested_mutator(depth)
        )
    };
    edit(&at(MAX_DEFINITION_DEPTH));
    let (path, kind) = refusal(&at(MAX_DEFINITION_DEPTH + 1));
    assert_eq!(kind, LoadErrorKind::DefinitionTooDeep { max: 32 });
    assert_eq!(
        path,
        format!(
            "edits[0].mutator{}",
            ".void.children[0]".repeat(MAX_DEFINITION_DEPTH)
        )
    );
    // The same, inside a map.
    let map = |depth: usize| format!(r#"{{"format":1,"name":"t","edits":[{}],"nodes":[]}}"#, at(depth));
    assert!(load(&map(MAX_DEFINITION_DEPTH)).is_ok());
    let error = load(&map(MAX_DEFINITION_DEPTH + 1)).unwrap_err();
    assert_eq!(error.kind, LoadErrorKind::DefinitionTooDeep { max: 32 });
    assert!(error.path.starts_with("edits[0].mutator"), "{}", error.path);
}

/// **A map's deep `edits` are refused as a definition whatever comes
/// before them, and after a syntax error** (review of #331). A node
/// that nests 118 levels — within the reader's 128 — comes first, then
/// `edits` with a mutator 45 levels deep: refused as too deep a
/// definition. And a map whose deep `edits` come first, cut off at its
/// end, is refused as cut off: the syntax error, anywhere, first.
///
/// Controls: when the scan ran over the whole text before the index, it
/// stopped at the node and the reader answered "recursion limit
/// exceeded"; and the cut-off map read "too deep".
#[test]
fn test_deep_edits_are_a_definition_refusal_after_any_syntax_error() {
    let edits = format!(
        r#"[{{"id":"e","name":"E","reach":"self","target":"node","mutator":{}}}]"#,
        nested_mutator(45)
    );
    let node = format!(
        r#"{{"id":1,"position":[0,0],"x":{}{}}}"#,
        "[".repeat(115),
        "]".repeat(115)
    );
    let error = load(&format!(
        r#"{{"format":1,"name":"t","nodes":[{node}],"edits":{edits}}}"#
    ))
    .unwrap_err();
    assert_eq!(error.kind, LoadErrorKind::DefinitionTooDeep { max: 32 });
    assert_eq!(error.path, "edits[0]");
    let edits_first = format!(r#"{{"format":1,"name":"t","edits":{edits},"nodes":[]}}"#);
    let error = load(&edits_first[..edits_first.len() - 1]).unwrap_err();
    assert!(
        matches!(error.kind, LoadErrorKind::Syntax { .. }),
        "{:?}",
        error.kind
    );
}

#[test]
fn test_nesting_at_the_readers_limit_is_refused_as_a_definition() {
    // 128 levels of JSON — where the reader would stop with its own
    // "recursion limit exceeded" — inside an edit, and far past it.
    for depth in [128, 129, 400] {
        let deep = format!("{}{}", "[".repeat(depth - 3), "]".repeat(depth - 3));
        let edits = format!(
            r#"[{{"id":"e","name":"E","reach":"self","target":"node","handler":"flower","x":{deep}}}]"#
        );
        let error = parse_edits(&format!(r#"{{"edits":{edits}}}"#)).unwrap_err();
        assert_eq!(
            error.kind,
            LoadErrorKind::DefinitionTooDeep { max: 32 },
            "{depth} in a file"
        );
        assert_eq!(error.path, "edits[0]");
        let error = load(&format!(
            r#"{{"format":1,"name":"t","edits":{edits},"nodes":[]}}"#
        ))
        .unwrap_err();
        assert_eq!(
            error.kind,
            LoadErrorKind::DefinitionTooDeep { max: 32 },
            "{depth} in a map"
        );
        assert!(error.position.is_some(), "it says where");
    }
    // A mutator nested past the reader's limit, the realistic case:
    // ours too.
    let deep_mutator = format!(
        r#"{{"edits":[{{"id":"e","name":"E","reach":"self","target":"node","mutator":{}}}]}}"#,
        nested_mutator(45)
    );
    let error = parse_edits(&deep_mutator).unwrap_err();
    assert_eq!(error.kind, LoadErrorKind::DefinitionTooDeep { max: 32 });
    // Control: read without the check, the reader stops at its own
    // limit on that same text, so it is the check that answers above.
    let unchecked =
        serde_json::from_str::<crate::document::format::edits::EditsFileDto>(&deep_mutator).unwrap_err();
    assert!(
        unchecked.to_string().contains("recursion limit exceeded"),
        "{unchecked}"
    );
}

#[test]
fn test_a_definition_has_at_most_65536_parts() {
    let with = |count: usize| {
        let mutations = vec![r#"{"node":"toggle_folded"}"#; count].join(",");
        format!(r#"{{"id":"e","name":"E","reach":"self","target":"node","mutations":[{mutations}]}}"#)
    };
    edit(&with(MAX_DEFINITION_PARTS));
    assert_eq!(
        refusal(&with(MAX_DEFINITION_PARTS + 1)),
        (
            "edits[0]".to_string(),
            LoadErrorKind::DefinitionTooLarge { max: 65_536 }
        )
    );
}

/// A mutator that aligns a node's sections: it reaches below its root.
const ALIGN_SECTIONS: &str = r#""mutator":{"void":{"kind":"node","channel":0,"children":[{"macro":{"kind":"section","channel":0,"mutations":[{"section":{"set_align":"right"}}]}}]}}"#;

/// **A mutator that reaches below its root is not aimed at nodes
/// through `descendants` or `subtree`** (review of #331): those nodes
/// nest, and the walker starts only from the outermost, so the rest
/// would be changed only where the mutator happens to reach them.
///
/// Controls: the same mutator at `self` reads, and so does one that
/// touches only its root at `subtree`, and one over sections — which do
/// not nest — at `subtree`.
#[test]
fn test_a_mutator_reaching_below_its_root_is_refused_over_nested_nodes() {
    for reach in ["descendants", "subtree"] {
        let (path, kind) = refusal(&format!(
            r#"{{"id":"e","name":"E","reach":"{reach}","target":"node",{ALIGN_SECTIONS}}}"#
        ));
        assert_eq!(path, "edits[0].mutator", "{reach}");
        assert!(
            matches!(&kind, LoadErrorKind::InvalidEdit { message } if message.contains("outermost")),
            "{reach}: {kind:?}"
        );
    }
    edit(&format!(
        r#"{{"id":"e","name":"E","reach":"self","target":"node",{ALIGN_SECTIONS}}}"#
    ));
    edit(
        r#"{"id":"e","name":"E","reach":"subtree","target":"node","mutator":{"macro":{"kind":"node","channel":0,"mutations":[{"node":{"nudge":[1.0,0.0]}}]}}}"#,
    );
    edit(
        r#"{"id":"e","name":"E","reach":"subtree","target":"sections","mutator":{"macro":{"kind":"section","channel":0,"mutations":[],"children":[{"macro":{"kind":"span","channel":0,"mutations":[{"span":{"text":{"grow_size":1.0}}}]}}]}}}"#,
    );
}

/// **At a nested selection, a mutator that reaches below its root
/// starts only from the outermost anchor** (review of #331): pinned.
/// Node 2 is node 1's child; at both, the walk starts at node 1, whose
/// channel-0 section it aligns, and changes node 2's only where it reaches
/// them — here nowhere, since it stops at node 1's own sections.
///
/// Control: a `mutations` body over `sections` at the same selection
/// aligns node 2's section too.
#[test]
fn test_a_nested_anchor_is_walked_only_from_the_outer_one() {
    let mut doc = map();
    let (one, two) = (n(&doc, 1), n(&doc, 2));
    let def = edit(&format!(
        r#"{{"id":"e","name":"E","reach":"self","target":"node",{ALIGN_SECTIONS}}}"#
    ));
    let walks = def
        .walks(&doc, &Boxes::of(&doc), &registry(), &[two, one])
        .unwrap();
    assert_eq!(walks.len(), 1);
    assert_eq!(
        walks[0].mutator.walk_starts(doc.tree(), &walks[0].anchors),
        Ok(vec![one])
    );
    run(&mut doc, &def, &[two, one]).unwrap();
    let aligned = |doc: &Document, ident: u32| {
        doc.children(n(doc, ident))
            .filter(|&c| matches!(doc.get(c), Some(DocElement::Section(s)) if s.align.is_some()))
            .count()
    };
    assert_eq!((aligned(&doc, 1), aligned(&doc, 2)), (1, 0));

    let mut control = map();
    let sections = edit(
        r#"{"id":"e","name":"E","reach":"self","target":"sections","mutations":[{"section":{"set_align":"right"}}]}"#,
    );
    run(&mut control, &sections, &[two, one]).unwrap();
    assert_eq!((aligned(&control, 1), aligned(&control, 2)), (2, 1));
}

/// A map of node 1 at the origin with children 2, 3 and 4, node 5
/// under node 2, and node 6 under node 5.
const FAMILY: &str = r#"{"format":1,"name":"t","nodes":[
{"id":1,"position":[0,0]},
{"id":2,"parent":1,"position":[10,10]},
{"id":5,"parent":2,"position":[20,20]},
{"id":6,"parent":5,"position":[30,30]},
{"id":3,"parent":1,"channel":1,"position":[10,10]},
{"id":4,"parent":1,"channel":2,"position":[10,10]}]}"#;

#[test]
fn test_flower_puts_the_children_on_a_circle_round_their_parent() {
    let mut doc = load(FAMILY).unwrap();
    let one = n(&doc, 1);
    let geometry = Boxes::of(&doc);
    let flower = edit(r#"{"id":"f","name":"F","reach":"self","target":"node","handler":"flower"}"#);
    // What the handler returns is an ordinary mutator, which the
    // validator passes, before anything applies it.
    let walks = crate::edit_handlers::flower(&doc, &geometry, &[one]).unwrap();
    assert_eq!(walks.len(), 3, "one walk per child placed");
    for (mutator, _) in &walks {
        assert_eq!(validate(mutator, &doc), Ok(()));
    }
    run(&mut doc, &flower, &[one]).unwrap();
    let center = Vec2::new(50.0, 20.0);
    // Half of each 100 × 40 box's longer side, twice, and the padding.
    let radius = 50.0 + 50.0 + crate::edit_handlers::RADIAL_PADDING;
    let centers: Vec<Vec2> = [2, 3, 4]
        .iter()
        .map(|&i| node(&doc, i).1.position + Vec2::new(50.0, 20.0))
        .collect();
    for c in &centers {
        assert!(
            ((*c - center).length() - radius).abs() < 1e-3,
            "{c} on the circle"
        );
    }
    assert!(
        (centers[0] - (center - Vec2::new(0.0, radius))).length() < 1e-3,
        "the first at the top"
    );
    assert!((centers[0] - centers[1]).length() > 1.0 && (centers[1] - centers[2]).length() > 1.0);
    // Node 5 is not node 1's child: left where it was.
    assert_eq!(node(&doc, 5).1.position, Vec2::new(20.0, 20.0));
    assert_eq!(doc.history().count(), 1);
}

#[test]
fn test_cascade_sets_each_row_below_its_parent() {
    let mut doc = load(FAMILY).unwrap();
    let one = n(&doc, 1);
    let cascade = edit(r#"{"id":"c","name":"C","reach":"self","target":"node","handler":"cascade"}"#);
    run(&mut doc, &cascade, &[one]).unwrap();
    let at = |i| node(&doc, i).1.position;
    let row = 40.0 + crate::edit_handlers::ROW_GAP;
    // Three 100-wide children, 40 apart, centered under node 1's 50.
    let left = 50.0 - (3.0 * 100.0 + 2.0 * 40.0) / 2.0;
    assert_eq!(at(2), Vec2::new(left, row));
    assert_eq!(at(3), Vec2::new(left + 140.0, row));
    assert_eq!(at(4), Vec2::new(left + 280.0, row));
    // Node 5 is placed from node 2's new place, not its old one.
    assert_eq!(at(5), Vec2::new(left, 2.0 * row));
    assert_eq!(at(1), Vec2::ZERO, "the anchor stays");
}

/// Where FAMILY's nodes 1 to 6 are, after `handler` runs at each of
/// `runs` in turn — each run one edit, read from the map as the last
/// one left it.
fn laid_out(handler: &str, runs: &[&[u32]]) -> Vec<Vec2> {
    let mut doc = load(FAMILY).unwrap();
    let def = edit(&format!(
        r#"{{"id":"h","name":"H","reach":"self","target":"node","handler":"{handler}"}}"#
    ));
    for idents in runs {
        let anchors: Vec<NodeId> = idents.iter().map(|&i| n(&doc, i)).collect();
        run(&mut doc, &def, &anchors).unwrap();
    }
    (1..=6).map(|i| node(&doc, i).1.position).collect()
}

/// **A node nested under another anchor is laid out from where the
/// outer one puts it** (review of #331). Run at node 1 and a node in
/// its subtree — its child 2, or its grandchild 5, which has a child of
/// its own — `flower` ends where running it at node 1 and then at the
/// inner node ends, the inner node's children round its new place, and
/// `cascade` where running it at node 1 alone does, since that already
/// lays out the whole subtree, each row from its parent's new place.
///
/// Controls: when `flower` read each anchor's drawn rectangle, node 5
/// was placed round node 2's old place; when `cascade` seeded every
/// anchor, node 6 was placed under node 5's.
#[test]
fn test_nested_anchors_compose_as_the_outer_then_the_inner() {
    for inner in [2, 5] {
        let flower = laid_out("flower", &[&[1], &[inner]]);
        assert_ne!(
            flower,
            laid_out("flower", &[&[1]]),
            "precondition: the flower at node {inner} moves its child"
        );
        let cascade = laid_out("cascade", &[&[1]]);
        for (handler, expected) in [("flower", flower), ("cascade", cascade)] {
            assert_eq!(
                laid_out(handler, &[&[1, inner]]),
                expected,
                "{handler} at 1 and {inner}"
            );
        }
    }
}

#[test]
fn test_a_node_that_is_not_drawn_is_left_in_place() {
    let mut doc = load(FAMILY).unwrap();
    let (one, three) = (n(&doc, 1), n(&doc, 3));
    let mut geometry = Boxes::of(&doc);
    geometry.0.remove(&three);
    let request = edit(r#"{"id":"f","name":"F","reach":"self","target":"node","handler":"flower"}"#)
        .request(&doc, &geometry, &registry(), &[one])
        .unwrap();
    doc.apply(request, ChangeSource::Edit).unwrap();
    assert_eq!(node(&doc, 3).1.position, Vec2::new(10.0, 10.0));
    assert_ne!(
        node(&doc, 2).1.position,
        Vec2::new(10.0, 10.0),
        "control: a drawn child moved"
    );
}

#[test]
fn test_an_edit_that_reaches_nothing_makes_no_entry() {
    let mut doc = map();
    let five = n(&doc, 5);
    let edit = edit(
        r#"{"id":"e","name":"E","reach":"children","target":"node","mutations":[{"node":{"nudge":[5.0,0.0]}}]}"#,
    );
    run(&mut doc, &edit, &[five]).unwrap();
    assert_eq!(doc.history().count(), 0);
    let one = n(&doc, 1);
    run(&mut doc, &edit, &[one]).unwrap();
    assert_eq!(
        doc.history().count(),
        1,
        "control: one that reaches something does"
    );
}

/// A handler's registry holds the two bundled layouts.
#[test]
fn test_the_registry_holds_flower_and_cascade() {
    let handlers: Handlers = registry();
    let mut names: Vec<&str> = handlers.keys().copied().collect();
    names.sort_unstable();
    assert_eq!(names, ["cascade", "flower"]);
    let _: EditRequest = edit(r#"{"id":"f","name":"F","reach":"self","target":"node","handler":"flower"}"#)
        .request(&map(), &Boxes::of(&map()), &handlers, &[])
        .unwrap();
}
