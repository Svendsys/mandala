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
/// under node 2, and node 6 under node 5, those two drawn well below the
/// family.
const FAMILY: &str = r#"{"format":1,"name":"t","nodes":[
{"id":1,"position":[0,0]},
{"id":2,"parent":1,"position":[10,10]},
{"id":5,"parent":2,"position":[20,1000]},
{"id":6,"parent":5,"position":[30,1100]},
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
    // Clockwise in sibling order: each next son 120° on, a positive step
    // on the canvas, whose y points down.
    assert_eq!(
        steps_between(center, &centers),
        [120.0, 120.0],
        "clockwise, evenly"
    );
    // Node 5 is not node 1's child: left where it was.
    assert_eq!(node(&doc, 5).1.position, Vec2::new(20.0, 1000.0));
    assert_eq!(doc.history().count(), 1);
}

/// The angle of each of `points` round `center`, in degrees (0 is right,
/// 90 down), and the signed step from each to the next, wrapped to
/// (−180, 180] and rounded to a hundredth: positive is clockwise on the
/// canvas.
fn steps_between(center: Vec2, points: &[Vec2]) -> Vec<f32> {
    points
        .windows(2)
        .map(|w| {
            let step = ((w[1] - center).to_angle() - (w[0] - center).to_angle()).to_degrees();
            let wrapped = (step + 180.0).rem_euclid(360.0) - 180.0;
            let wrapped = if wrapped == -180.0 { 180.0 } else { wrapped };
            (wrapped * 100.0).round() / 100.0
        })
        .collect()
}

/// The angle of `point` round `center`, in degrees, rounded to a
/// hundredth.
fn angle_of(center: Vec2, point: Vec2) -> f32 {
    ((point - center).to_angle().to_degrees() * 100.0).round() / 100.0
}

/// Whether the straight line from `a` to `b` passes through the inside
/// of `rect` where it is outside `skip`: sampled in f64 at a hundredth of
/// a unit, an oracle independent of the handlers' f32 clipping.
fn sampled_crossing(a: Vec2, b: Vec2, rect: &crate::flow::Rect, skip: &[crate::flow::Rect]) -> bool {
    let wide = |v: Vec2| (f64::from(v.x), f64::from(v.y));
    let inside = |r: &crate::flow::Rect, (x, y): (f64, f64)| {
        let (lo, hi) = (wide(r.origin), wide(r.max()));
        lo.0 < x && x < hi.0 && lo.1 < y && y < hi.1
    };
    let ((ax, ay), (bx, by)) = (wide(a), wide(b));
    let samples = ((bx - ax).hypot(by - ay) * 100.0).ceil() as usize;
    (0..=samples).any(|k| {
        let t = k as f64 / samples as f64;
        let p = (ax + (bx - ax) * t, ay + (by - ay) * t);
        inside(rect, p) && !skip.iter().any(|s| inside(s, p))
    })
}

/// FAMILY with its grandchildren on node 1's lower right: node 5 at
/// `[20,22]` and node 6 at `[30,26]`, each edge whole units from every
/// petal and line it decides, so no rounding of an angle changes the
/// answer.
fn family_on_its_lines() -> String {
    FAMILY
        .replace(
            r#"{"id":5,"parent":2,"position":[20,1000]}"#,
            r#"{"id":5,"parent":2,"position":[20,22]}"#,
        )
        .replace(
            r#"{"id":6,"parent":5,"position":[30,1100]}"#,
            r#"{"id":6,"parent":5,"position":[30,26]}"#,
        )
}

/// **A flower fans away from nodes on its lines** (review of #331). On
/// FAMILY with nodes 5 and 6 on node 1's lower right, every rotation of
/// the full circle has its petals clear, but its line into the lower
/// right (between 30° and 120°) crosses node 5 or 6 outside node 1; so
/// does every 210° fan's. The flower is the widest fan whose lines are
/// clear: a 180° fan centered on the top, its first son at the
/// counter-clockwise end (180°), each next 90° on clockwise, and no line
/// from node 1 to a son crosses node 5 or 6 where it runs outside node 1
/// and the son (an f64 sampled oracle). Every edge that decides it is at
/// least two units clear.
///
/// Controls: with the line check off, the full circle was taken; with
/// the fan reversed, the first son was at the clockwise end and the
/// steps were negative.
#[test]
fn test_flower_fans_away_from_nodes_on_its_lines() {
    let mut doc = load(&family_on_its_lines()).unwrap();
    let one = n(&doc, 1);
    let flower = edit(r#"{"id":"f","name":"F","reach":"self","target":"node","handler":"flower"}"#);
    run(&mut doc, &flower, &[one]).unwrap();
    let center = Vec2::new(50.0, 20.0);
    let size = Vec2::new(100.0, 40.0);
    let sons: Vec<Vec2> = [2, 3, 4]
        .iter()
        .map(|&i| node(&doc, i).1.position + size / 2.0)
        .collect();
    for son in &sons {
        assert!(
            ((*son - center).length() - 140.0).abs() < 1e-3,
            "{son} at the base radius"
        );
    }
    assert_eq!(
        angle_of(center, sons[0]),
        180.0,
        "the first at the counter-clockwise end"
    );
    assert_eq!(
        steps_between(center, &sons),
        [90.0, 90.0],
        "clockwise, evenly: a 180° fan"
    );
    let rect = |at: Vec2| crate::flow::Rect::new(at, size);
    let parent = rect(Vec2::ZERO);
    for son in &sons {
        for blocker in [5, 6] {
            let on = rect(node(&doc, blocker).1.position);
            assert!(
                !sampled_crossing(center, *son, &on, &[parent, rect(*son - size / 2.0)]),
                "the line to {son} crosses node {blocker}"
            );
        }
    }
}

/// **A petal's line counts only outside its parent and the petal**
/// (review of #331). A 10 × 10 node sits inside node 1, across the line
/// to the top petal by five units each side, and five units below node
/// 1's top edge, where that line leaves it. The line's part outside node
/// 1 is clear, so the full circle is taken unturned, its first son at the
/// top — though the whole line from node 1's center runs through the
/// small node (asserted as a precondition, by the f64 sampled oracle).
///
/// Control: reading each line whole from node 1's center refused the
/// unturned circle.
#[test]
fn test_a_line_counts_only_outside_its_parent_and_the_petal() {
    use crate::flow::Rect;
    let text = format!(
        r#"{},{{"id":7,"channel":1,"position":[45,5]}}]}}"#,
        FAMILY.strip_suffix("]}").expect("FAMILY ends its list")
    );
    let mut doc = load(&text).unwrap();
    let mut geometry = Boxes::of(&doc);
    let inner = Rect::new(Vec2::new(45.0, 5.0), Vec2::splat(10.0));
    geometry.0.insert(n(&doc, 7), inner);
    let center = Vec2::new(50.0, 20.0);
    let size = Vec2::new(100.0, 40.0);
    let parent = Rect::new(Vec2::ZERO, size);
    let top = center - Vec2::new(0.0, 140.0);
    assert!(
        sampled_crossing(center, top, &inner, &[]),
        "precondition: the whole line runs through it"
    );
    assert!(
        !sampled_crossing(center, top, &inner, &[parent, Rect::new(top - size / 2.0, size)]),
        "precondition: its part outside node 1 does not"
    );
    let request = edit(r#"{"id":"f","name":"F","reach":"self","target":"node","handler":"flower"}"#)
        .request(&doc, &geometry, &registry(), &[n(&doc, 1)])
        .unwrap();
    doc.apply(request, ChangeSource::Edit).unwrap();
    let sons: Vec<Vec2> = [2, 3, 4]
        .iter()
        .map(|&i| node(&doc, i).1.position + size / 2.0)
        .collect();
    assert_eq!(
        angle_of(center, sons[0]),
        -90.0,
        "the first at the top: the circle unturned"
    );
    assert_eq!(steps_between(center, &sons), [120.0, 120.0], "the full circle");
}

/// **A petal's line must not cross its own siblings** (review of #331).
/// Two sons 600 wide at radius 700, 20° apart from the top: they do not
/// overlap, but the line to the first, straight up, runs through the
/// second, so the arrangement is not taken. 90° apart, it is.
///
/// Control: with the check against the arrangement's own petals off,
/// the 20° arrangement was taken.
#[test]
fn test_a_petals_line_must_not_cross_its_own_siblings() {
    use crate::edit_handlers::{arrangement, Clear};
    use crate::flow::Rect;
    let doc = load(FAMILY).unwrap();
    let geometry = Boxes::of(&doc);
    let wide = |i| (n(&doc, i), Rect::new(Vec2::ZERO, Vec2::new(600.0, 40.0)));
    let sons = [wide(2), wide(3)];
    // Node 4, FAMILY's third son, sits on node 1; it moves with the others.
    let moved = [wide(2), wide(3), wide(4)];
    let clear = Clear::new(&geometry, moved.iter());
    let parent = (n(&doc, 1), Rect::new(Vec2::ZERO, Vec2::new(100.0, 40.0)));
    let center = Vec2::new(50.0, 20.0);
    let up = -std::f32::consts::FRAC_PI_2;
    let tight = arrangement(&clear, parent, center, 700.0, up, 20f32.to_radians(), &sons);
    assert!(tight.is_none(), "the line to the first runs through the second");
    let laid = arrangement(&clear, parent, center, 700.0, up, 90f32.to_radians(), &sons)
        .expect("control: 90° apart, nothing crosses");
    assert_eq!(laid.len(), 2);
}

/// **A flower tries the circle's rotations before any fan** (review of
/// #331). A node sits where FAMILY's first son would go at the top, and
/// the next two rotations still meet it; the fourth, 45° on, is clear,
/// so the flower is the full circle at the base radius, turned by 45°,
/// each next son 120° on clockwise.
///
/// Controls: with no rotations, a fan was taken; with the circle
/// reversed, the steps were negative.
#[test]
fn test_a_flower_turns_the_circle_before_it_fans() {
    let blocked = format!(
        r#"{},{{"id":7,"channel":1,"position":[0,-140]}}]}}"#,
        FAMILY.strip_suffix("]}").expect("FAMILY ends its list")
    );
    let mut doc = load(&blocked).unwrap();
    let one = n(&doc, 1);
    let flower = edit(r#"{"id":"f","name":"F","reach":"self","target":"node","handler":"flower"}"#);
    run(&mut doc, &flower, &[one]).unwrap();
    let center = Vec2::new(50.0, 20.0);
    let sons: Vec<Vec2> = [2, 3, 4]
        .iter()
        .map(|&i| node(&doc, i).1.position + Vec2::new(50.0, 20.0))
        .collect();
    for son in &sons {
        assert!(
            ((*son - center).length() - 140.0).abs() < 1e-3,
            "{son} at the base radius"
        );
    }
    assert_eq!(
        angle_of(center, sons[0]),
        -45.0,
        "turned by three eighths of 120°"
    );
    assert_eq!(
        steps_between(center, &sons),
        [120.0, 120.0],
        "the full circle, clockwise"
    );
}

/// **A flower takes the smallest radius first** (review of #331). Two
/// small nodes block every rotation of the full circle at the base
/// radius (140), while the full circle is clear one step out (160); a
/// 210° fan is clear at 140, and it is taken.
///
/// Control: with each arrangement tried over every radius before the
/// next arrangement, the circle at 160 was taken.
#[test]
fn test_a_flower_takes_the_smallest_radius_first() {
    use crate::edit_handlers::{arrangement, regular_flower, Clear, FLOWER_RADIUS_STEP};
    use crate::flow::Rect;
    let text = format!(
        r#"{},{{"id":7,"channel":1,"position":[0,0]}},{{"id":8,"channel":2,"position":[0,0]}}]}}"#,
        FAMILY.strip_suffix("]}").expect("FAMILY ends its list")
    );
    let doc = load(&text).unwrap();
    let mut geometry = Boxes::of(&doc);
    geometry.0.insert(
        n(&doc, 7),
        Rect::new(Vec2::new(85.0, -110.0), Vec2::new(15.0, 10.0)),
    );
    geometry.0.insert(
        n(&doc, 8),
        Rect::new(Vec2::new(0.0, 120.0), Vec2::new(60.0, 20.0)),
    );
    let sons: Vec<(NodeId, Rect)> = [2, 3, 4]
        .iter()
        .map(|&i| (n(&doc, i), Rect::new(Vec2::ZERO, Vec2::new(100.0, 40.0))))
        .collect();
    let clear = Clear::new(&geometry, sons.iter());
    let parent = (n(&doc, 1), Rect::new(Vec2::ZERO, Vec2::new(100.0, 40.0)));
    let center = Vec2::new(50.0, 20.0);
    let circle = |r: f32, turn: f32| {
        arrangement(
            &clear,
            parent,
            center,
            r,
            -std::f32::consts::FRAC_PI_2 + turn,
            std::f32::consts::TAU / 3.0,
            &sons,
        )
    };
    assert!(
        (0..8).all(|j| circle(140.0, j as f32 * std::f32::consts::TAU / 24.0).is_none()),
        "precondition: no rotation of the circle is clear at 140"
    );
    assert!(
        circle(140.0 + FLOWER_RADIUS_STEP, 0.0).is_some(),
        "precondition: the circle is clear at 160"
    );
    let laid = regular_flower(&clear, parent, center, 140.0, &sons).expect("a clear arrangement");
    let at: Vec<Vec2> = laid.iter().map(|petal| petal.center()).collect();
    for petal in &at {
        assert!(
            ((*petal - center).length() - 140.0).abs() < 1e-3,
            "{petal} at 140, not 160"
        );
    }
    assert_eq!(angle_of(center, at[0]), -45.0);
    assert_eq!(steps_between(center, &at), [105.0, 105.0], "a 210° fan");
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

/// **A layout at the edge of the canvas pins what it would place past
/// it** (review of #331), as `main`'s layouts did: a flower round a
/// node near `MAX_CANVAS_COORD` puts its right-hand petals at the bound,
/// and the edit runs.
///
/// Control: unclamped, the position past the bound refused the whole
/// edit.
#[test]
fn test_a_layout_pins_at_the_canvas_bound() {
    use crate::document::bounds::MAX_CANVAS_COORD;
    let edge = FAMILY.replacen("[0,0]", "[999999950,0]", 1);
    let mut doc = load(&edge).unwrap();
    let one = n(&doc, 1);
    let flower = edit(r#"{"id":"f","name":"F","reach":"self","target":"node","handler":"flower"}"#);
    run(&mut doc, &flower, &[one]).expect("the flower runs");
    let xs: Vec<f32> = [2, 3, 4].iter().map(|&i| node(&doc, i).1.position.x).collect();
    assert!(xs.iter().all(|x| *x <= MAX_CANVAS_COORD), "{xs:?}");
    assert!(
        xs.contains(&MAX_CANVAS_COORD),
        "a petal pinned at the bound: {xs:?}"
    );
}

/// Each node of `moved` whose box overlaps another node's in `doc`, with
/// that node: a shared edge is not an overlap.
fn overlaps(doc: &Document, moved: &[NodeId]) -> Vec<(NodeId, NodeId)> {
    let boxes = Boxes::of(doc);
    let meets = |a: &crate::flow::Rect, b: &crate::flow::Rect| {
        a.origin.x < b.max().x && b.origin.x < a.max().x && a.origin.y < b.max().y && b.origin.y < a.max().y
    };
    let mut found = Vec::new();
    for &m in moved {
        for (&other, rect) in &boxes.0 {
            if other != m && meets(&boxes.0[&m], rect) {
                found.push((m, other));
            }
        }
    }
    found
}

/// Node 1 at the origin with twelve children stacked on it, inside a
/// grid of `cols` × `rows` other nodes, each box touching its neighbors:
/// no place inside the grid is clear.
fn crowded(cols: i32, rows: i32) -> String {
    let mut nodes = vec![r#"{"id":1,"position":[0,0]}"#.to_string()];
    nodes.extend((2..=13).map(|id| format!(r#"{{"id":{id},"parent":1,"position":[0,0]}}"#)));
    let mut id = 100;
    for col in 0..cols {
        for row in 0..rows {
            let (x, y) = ((col - cols / 2) * 100, (row - rows / 2) * 40);
            nodes.push(format!(r#"{{"id":{id},"position":[{x},{y}]}}"#));
            id += 1;
        }
    }
    format!(r#"{{"format":1,"name":"t","nodes":[{}]}}"#, nodes.join(","))
}

/// **A layout places no node over another drawn node** (owner, on #250:
/// testament's Ishmael flower put Hadad and Massa on Hagar). A node
/// where `flower`'s first petal and `cascade`'s first row would go is
/// stepped round; and in a grid where nothing nearby is clear — larger
/// than the bounded search, so some petals go past it all — every petal
/// and every row is still clear of every other box.
///
/// Control: `main`'s placement put the petal and the row on that node,
/// and laid the crowded family over the grid.
#[test]
fn test_a_layout_places_no_node_over_another() {
    let blocker = format!(
        r#"{},{{"id":7,"channel":1,"position":[0,-140]}},{{"id":8,"channel":2,"position":[-140,120]}}]}}"#,
        FAMILY.strip_suffix("]}").expect("FAMILY ends its list")
    );
    for (text, family) in [(blocker, vec![2, 3, 4]), (crowded(60, 120), (2..=13).collect())] {
        for handler in ["flower", "cascade"] {
            let mut doc = load(&text).unwrap();
            let one = n(&doc, 1);
            let def = edit(&format!(
                r#"{{"id":"h","name":"H","reach":"self","target":"node","handler":"{handler}"}}"#
            ));
            run(&mut doc, &def, &[one]).unwrap();
            let moved: Vec<NodeId> = family.iter().map(|&i| n(&doc, i)).collect();
            let found = overlaps(&doc, &moved);
            assert!(
                found.is_empty(),
                "{handler}: {} overlaps, first {:?}",
                found.len(),
                found.first()
            );
        }
    }
}

/// **A blocked cascade row moves as a whole, sideways before down**
/// (owner, on #250). A node sits under the left end of node 1's row:
/// straight below is blocked, and one [`CLEAR_STEP`] right is clear, so
/// the row stays at its depth, shifted right, its spacing kept, and the
/// rows below follow their parents.
///
/// Control: a search that only stepped down put the row one step lower.
///
/// [`CLEAR_STEP`]: crate::edit_handlers::CLEAR_STEP
#[test]
fn test_a_blocked_cascade_row_moves_whole_sideways_before_down() {
    use crate::edit_handlers::{CLEAR_STEP, ROW_GAP};
    let blocked = format!(
        r#"{},{{"id":9,"channel":1,"position":[-200,120]}}]}}"#,
        FAMILY.strip_suffix("]}").expect("FAMILY ends its list")
    );
    let mut doc = load(&blocked).unwrap();
    let one = n(&doc, 1);
    let cascade = edit(r#"{"id":"c","name":"C","reach":"self","target":"node","handler":"cascade"}"#);
    run(&mut doc, &cascade, &[one]).unwrap();
    let at = |i| node(&doc, i).1.position;
    let row = 40.0 + ROW_GAP;
    let left = 50.0 - (3.0 * 100.0 + 2.0 * 40.0) / 2.0 + CLEAR_STEP;
    assert_eq!(
        [at(2), at(3), at(4)],
        [
            Vec2::new(left, row),
            Vec2::new(left + 140.0, row),
            Vec2::new(left + 280.0, row)
        ],
        "the whole row, one step right, at its own depth"
    );
    assert_eq!(at(5), Vec2::new(left, 2.0 * row), "node 2's row follows it");
    let moved: Vec<NodeId> = [2, 3, 4, 5, 6].iter().map(|&i| n(&doc, i)).collect();
    assert!(overlaps(&doc, &moved).is_empty());
}

/// Any geometry, counting every question a layout asks of it.
struct Counting<'g>(&'g dyn crate::projection::Geometry, std::cell::Cell<usize>);

impl crate::projection::Geometry for Counting<'_> {
    fn node_rect(&self, node: NodeId) -> Option<crate::flow::Rect> {
        self.1.set(self.1.get() + 1);
        self.0.node_rect(node)
    }
    fn section_box(&self, section: NodeId) -> Option<(Vec2, &std::sync::Arc<crate::flow::ShapedBox>)> {
        self.0.section_box(section)
    }
    fn curve(&self, path: NodeId) -> Option<&[crate::flow::Curve]> {
        self.0.curve(path)
    }
    fn label_pos(&self, label: NodeId) -> Option<Vec2> {
        self.0.label_pos(label)
    }
    fn nodes_meeting(&self, rect: crate::flow::Rect) -> Vec<NodeId> {
        self.1.set(self.1.get() + 1);
        self.0.nodes_meeting(rect)
    }
}

/// **A layout's work follows the families it moves, not the map**
/// (review of #331, §20 rules 4 and 5): `flower` and `cascade` at node
/// 1 ask the same number of questions of the geometry whether the map
/// holds FAMILY alone or 10,000 more nodes far away. What is drawn
/// comes from the projector's index, a query at a time.
///
/// Control: building the obstacles from a scan of every drawn node asked
/// 10,000 more.
#[test]
fn test_a_layouts_work_does_not_grow_with_the_map() {
    use crate::projection::Geometry;
    let far: Vec<String> = (0..10_000)
        .map(|i| format!(r#"{{"id":{},"position":[{},0]}}"#, 100 + i, 1_000_000 + i * 200))
        .collect();
    let big = format!(
        r#"{},{}]}}"#,
        FAMILY.strip_suffix("]}").expect("FAMILY ends its list"),
        far.join(",")
    );
    for handler in ["flower", "cascade"] {
        let def = edit(&format!(
            r#"{{"id":"h","name":"H","reach":"self","target":"node","handler":"{handler}"}}"#
        ));
        let asked = |text: &str| {
            let doc = load(text).unwrap();
            let boxes = Boxes::of(&doc);
            let geometry = Counting(&boxes, std::cell::Cell::new(0));
            assert!(geometry.node_rect(n(&doc, 1)).is_some());
            geometry.1.set(0);
            def.request(&doc, &geometry, &registry(), &[n(&doc, 1)]).unwrap();
            geometry.1.get()
        };
        let (small, large) = (asked(FAMILY), asked(&big));
        assert!(small > 0, "{handler}: precondition: the layout asks");
        assert_eq!(small, large, "{handler}: as many questions on the larger map");
    }
}

/// **A blocked flower's work does not grow with the map** (review of
/// #331). Node 1 has twelve sons and a thin frame round it that every
/// son's line crosses, so every regular arrangement is tried and fails;
/// the sons then step out on their own rays. Through the real projector,
/// the questions asked and the bodies its index scans to answer them are
/// the same with 2,000 more nodes far away as without them — for a
/// parent of its own size, and for one 400,000 wide, whose long lines
/// are never read a piece wider than the index's cells. (The cap on a
/// line's pieces is pinned where it acts, by
/// `test_a_line_is_read_in_at_most_64_pieces_and_a_longer_one_not_at_all`.)
///
/// Control: reading every line whole from the parent's center, its
/// pieces clamped to 64, made each piece of the wide parent's lines
/// cover more cells than the index lists a body under, so each scanned
/// every body, and the far nodes were scanned.
#[test]
fn test_a_blocked_flowers_work_does_not_grow_with_the_map() {
    use crate::projection::tests::maps::{load_map, node};
    use crate::projection::tests::support::Projected;
    use serde_json::json;
    let def = edit(r#"{"id":"f","name":"F","reach":"self","target":"node","handler":"flower"}"#);
    for width in [100.0_f64, 400_000.0] {
        let work = |far: u32| {
            let mut nodes = vec![node(1, None, json!({"min_size": [width, 40.0]}))];
            nodes.extend((2..=13).map(|i| node(i, Some(1), json!({"min_size": [100.0, 40.0]}))));
            // The frame: four bars 20 units outside node 1, which every
            // line from it to a son crosses.
            let bar = |id: u32, at: [f64; 2], size: [f64; 2]| {
                node(id, None, json!({"channel": id, "position": at, "min_size": size}))
            };
            nodes.push(bar(20, [-24.0, -24.0], [width + 48.0, 4.0]));
            nodes.push(bar(21, [-24.0, 60.0], [width + 48.0, 4.0]));
            nodes.push(bar(22, [-24.0, -24.0], [4.0, 88.0]));
            nodes.push(bar(23, [width + 20.0, -24.0], [4.0, 88.0]));
            nodes.extend((0..far).map(|k| {
                node(
                    100 + k,
                    None,
                    json!({"channel": 100 + k, "position": [3.0e6 + f64::from(k) * 300.0, 0.0]}),
                )
            }));
            let p = Projected::new(load_map(json!({ "nodes": nodes })));
            let counting = Counting(&p.projector, std::cell::Cell::new(0));
            let before = p.projector.bodies.within_tried.get();
            def.request(&p.doc, &counting, &registry(), &[n(&p.doc, 1)])
                .unwrap();
            (counting.1.get(), p.projector.bodies.within_tried.get() - before)
        };
        let (near, far) = (work(0), work(2_000));
        assert!(
            near.0 > 4_480,
            "{width}: precondition: every arrangement was tried ({} questions)",
            near.0
        );
        assert_eq!(
            near, far,
            "{width}: (questions, bodies scanned) with the far nodes and without"
        );
    }
}

/// **A line is read in at most 64 pieces, and a longer one not at all**
/// (review of #331). One son straight above node 1 on an otherwise clear
/// map, so its line runs from node 1's top edge to the son's bottom one:
/// a line of 64 pieces of `LINE_PIECE` is read in exactly 64 questions,
/// after the one about the petal, and the arrangement is taken; a line of
/// 65 pieces is not read — the petal's question is the only one — and
/// the arrangement is refused.
///
/// Controls: with the refusal removed, the 65-piece line was read in 65
/// pieces; clamped to 64 pieces instead of refused, it was read in 64.
#[test]
fn test_a_line_is_read_in_at_most_64_pieces_and_a_longer_one_not_at_all() {
    use crate::edit_handlers::{arrangement, Clear, LINE_PIECE, MAX_LINE_PIECES};
    use crate::flow::Rect;
    let doc = load(FAMILY).unwrap();
    let boxes = Boxes::of(&doc);
    let geometry = Counting(&boxes, std::cell::Cell::new(0));
    let size = Vec2::new(100.0, 40.0);
    let sons = [(n(&doc, 2), Rect::new(Vec2::ZERO, size))];
    // Node 1's other sons sit on it; they are not what is measured.
    let moved = [sons[0], (n(&doc, 3), sons[0].1), (n(&doc, 4), sons[0].1)];
    let clear = Clear::new(&geometry, moved.iter());
    let parent = (n(&doc, 1), Rect::new(Vec2::ZERO, size));
    let center = Vec2::new(50.0, 20.0);
    let up = -std::f32::consts::FRAC_PI_2;
    // The line runs from y = 0 to the petal's bottom, `r - 40` long.
    let pieces = MAX_LINE_PIECES as f32;
    for (length, questions, taken) in [
        ((pieces - 0.5) * LINE_PIECE, 1 + MAX_LINE_PIECES as usize, true),
        ((pieces + 0.5) * LINE_PIECE, 1, false),
    ] {
        geometry.1.set(0);
        let laid = arrangement(&clear, parent, center, length + 40.0, up, 0.0, &sons);
        assert_eq!(laid.is_some(), taken, "a line {length} long");
        assert_eq!(geometry.1.get(), questions, "questions for a line {length} long");
    }
}

/// **A rectangle too large to count, or not finite, is filed under no
/// cell and never overflows the count** (review of #331): a child drawn
/// across the whole float range, and one drawn with an infinite size,
/// are laid out without panicking.
///
/// Control: counting cells as a product of the two axes' spans
/// overflowed `i64` on the first child, and the layout panicked.
#[test]
fn test_a_layout_survives_a_rectangle_too_large_to_count() {
    let doc = load(FAMILY).unwrap();
    for rect in [
        crate::flow::Rect::new(Vec2::splat(-3e38), Vec2::splat(f32::MAX)),
        crate::flow::Rect::new(Vec2::splat(-1e38), Vec2::splat(f32::INFINITY)),
    ] {
        let mut geometry = Boxes::of(&doc);
        geometry.0.insert(n(&doc, 2), rect);
        for handler in ["flower", "cascade"] {
            let def = edit(&format!(
                r#"{{"id":"h","name":"H","reach":"self","target":"node","handler":"{handler}"}}"#
            ));
            def.request(&doc, &geometry, &registry(), &[n(&doc, 1)]).unwrap();
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
