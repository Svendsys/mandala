// SPDX-License-Identifier: MPL-2.0

//! The composite builders, each run through the engine's journaled walk
//! on a document from `DocumentBuilder` — all its steps on one journal
//! builder, as the writer will run them.
//!
//! For every builder: the resulting document keeps the shape rules at
//! every element and passes `validate_applied`; reverting the journal
//! gives back the original tree exactly — the same ids, parents, order
//! and elements — and replaying it gives the edited tree exactly. The
//! text builders count grapheme clusters, and their fixtures put an
//! emoji of five code points and a letter with combining marks where a
//! `char` count would cut them.

use crate::document::element::{DocElement, DocKind, SectionData, SpanData, TextStyle};
use crate::document::ids::{Document, NodeIdent};
use crate::document::mutation::compose::{
    add_section, create_child, create_root, delete, delete_text, insert_text, merge_steps, remove_links,
    reparent, set_link_kind, split_section, style_range, tops_counting,
};
use crate::document::mutation::error::MutationError;
use crate::document::mutation::membership::Membership;
use crate::document::mutation::ops::TextOp;
use crate::document::mutation::step::DocStep;
use crate::document::mutation::validate::validate_applied;
use crate::document::shape::validate_shape;
use crate::document::tests::test_support::{example, DocumentBuilder, Example};
use crate::document::values::{LinkKind, LinkTarget};
use crate::engine::journal::{Journal, JournalBuilder};
use crate::engine::tree::{Tree, TreeError};
use glam::Vec2;
use indextree::NodeId;

/// The family emoji: one grapheme cluster of five code points.
const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
/// An `e` with two combining marks: one cluster of three code points.
const MARKED: &str = "e\u{0301}\u{0302}";

/// The tree as a list: every element in pre-order from the root, with
/// its id and parent.
fn snapshot(tree: &Tree<DocElement>) -> Vec<(NodeId, Option<NodeId>, DocElement)> {
    tree.descendants()
        .map(|id| (id, tree.arena()[id].parent(), tree.arena()[id].get().clone()))
        .collect()
}

/// Run `steps` on `doc` as one request, close the journal and judge it.
fn run(doc: &mut Document, steps: Vec<DocStep>) -> Result<Journal<DocElement>, MutationError> {
    let builder = doc.run_steps(steps, JournalBuilder::new())?;
    let journal = builder.close(doc.tree());
    validate_applied(doc.tree(), &journal)?;
    Ok(journal)
}

/// Run `steps` on `doc` and hold the result to the builders' contract:
/// shape rules everywhere, an exact revert and an exact replay.
fn apply(doc: &mut Document, steps: Vec<DocStep>) -> Journal<DocElement> {
    let before = snapshot(doc.tree());
    let journal = run(doc, steps).unwrap();
    let after = snapshot(doc.tree());
    for id in doc.tree().descendants() {
        validate_shape(doc.tree(), id).unwrap();
    }
    let mut tree = doc.tree().clone();
    journal.revert(&mut tree).unwrap();
    assert_eq!(snapshot(&tree), before, "revert restores the original exactly");
    journal.replay(&mut tree).unwrap();
    assert_eq!(snapshot(&tree), after, "replay restores the edit exactly");
    journal
}

/// A span element.
fn span_of(text: &str, style: TextStyle, link: Option<LinkTarget>) -> DocElement {
    DocElement::Span(SpanData {
        text: text.to_string(),
        style,
        link,
    })
}

/// An italic style; `None` is upright by inheritance.
fn italic(on: Option<bool>) -> TextStyle {
    TextStyle {
        italic: on,
        ..TextStyle::default()
    }
}

/// A document with one root node holding one section of `spans`.
fn section_of(spans: Vec<DocElement>) -> (Document, NodeId, NodeId) {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let node = builder.node(root);
    let section = builder.section(node, 0);
    for span in spans {
        builder.add(section, span);
    }
    (builder.build().unwrap(), node, section)
}

/// The spans of `section`: text and italic flag.
fn spans(doc: &Document, section: NodeId) -> Vec<(String, Option<bool>)> {
    doc.children(section)
        .filter_map(|id| match doc.get(id) {
            Some(DocElement::Span(span)) => Some((span.text.clone(), span.style.italic)),
            _ => None,
        })
        .collect()
}

/// The children of `id` of `kind`.
fn children_of(doc: &Document, id: NodeId, kind: DocKind) -> Vec<NodeId> {
    doc.children(id)
        .filter(|&c| doc.get(c).map(DocElement::kind) == Some(kind))
        .collect()
}

fn built() -> (Document, Example) {
    let (builder, ids) = example();
    (builder.build().unwrap(), ids)
}

#[test]
fn test_document_compose_creates_nodes() {
    do_document_compose_creates_nodes();
}

/// `create_child` gives a node an unassigned child with one empty
/// section and a connector, after its other child nodes; `create_root`
/// a root node with a section and no connector. A parent that is not a
/// node is refused.
pub fn do_document_compose_creates_nodes() {
    let (mut doc, ids) = built();
    let at = Vec2::new(5.0, 6.0);
    let steps = create_child(&doc, ids.b, at).unwrap();
    apply(&mut doc, steps);
    let nodes = children_of(&doc, ids.b, DocKind::Node);
    assert_eq!(nodes.len(), 2, "c and the new node");
    let new = nodes[1];
    assert_ne!(new, ids.c, "the new node comes after c");
    let Some(DocElement::Node(data)) = doc.get(new) else { panic!("a node") };
    assert_eq!((data.ident, data.position), (NodeIdent::UNASSIGNED, at));
    assert_eq!(children_of(&doc, new, DocKind::Section).len(), 1);
    assert_eq!(children_of(&doc, new, DocKind::Connector).len(), 1);
    assert!(
        doc.children(children_of(&doc, new, DocKind::Section)[0])
            .next()
            .is_none(),
        "empty section"
    );

    let roots_before = children_of(&doc, doc.root(), DocKind::Node).len();
    let steps = create_root(&doc, at).unwrap();
    apply(&mut doc, steps);
    let roots = children_of(&doc, doc.root(), DocKind::Node);
    assert_eq!(roots.len(), roots_before + 1);
    let new = *roots.last().unwrap();
    assert_eq!(children_of(&doc, new, DocKind::Section).len(), 1);
    assert!(
        children_of(&doc, new, DocKind::Connector).is_empty(),
        "a root has no connector"
    );

    assert_eq!(
        create_child(&doc, ids.a_title, at).unwrap_err(),
        MutationError::NotA {
            element: ids.a_title,
            expected: "a node",
        }
    );
}

#[test]
fn test_document_compose_deletes_nodes_and_links() {
    do_document_compose_deletes_nodes_and_links();
}

/// `delete` trims a nested selection to its tops — naming `c` beside its
/// ancestor `b` is not `TargetRemovedEarlier` — and the links into the
/// removed subtrees go in a further step of the same request
/// (`remove_links`), which skips a link already removed with its node.
pub fn do_document_compose_deletes_nodes_and_links() {
    let (mut doc, ids) = built();
    let mut steps = delete(&doc, &[ids.c, ids.b, ids.d, ids.b]).unwrap();
    assert_eq!(steps.len(), 1);
    steps.push(remove_links(vec![ids.portal, ids.line]));
    apply(&mut doc, steps);
    for gone in [ids.b, ids.c, ids.d, ids.portal, ids.line] {
        assert!(!doc.tree().contains(gone), "{gone} is removed");
    }
    assert!(doc.tree().contains(ids.a));

    // Removing a with its links: they went with it, and the link step
    // builds nothing.
    let (mut doc, ids) = built();
    let mut steps = delete(&doc, &[ids.a]).unwrap();
    steps.push(remove_links(vec![ids.line, ids.portal]));
    apply(&mut doc, steps);
    assert!(!doc.tree().contains(ids.line));

    let (doc, ids) = built();
    assert!(matches!(
        delete(&doc, &[ids.a_title]),
        Err(MutationError::NotA {
            expected: "a node",
            ..
        })
    ));
    assert!(delete(&doc, &[]).unwrap().is_empty());
}

#[test]
fn test_document_compose_reparents() {
    do_document_compose_reparents();
}

/// `reparent` under a node gives a moved root a connector; to the map it
/// takes every moved node's away; a nested selection moves as its top;
/// and a node moved under its own descendant fails the whole request,
/// leaving the document as it was.
pub fn do_document_compose_reparents() {
    let (mut doc, ids) = built();
    let steps = reparent(&doc, &[ids.d], ids.b).unwrap();
    apply(&mut doc, steps);
    assert_eq!(doc.parent(ids.d), Some(ids.b));
    assert_eq!(children_of(&doc, ids.d, DocKind::Connector).len(), 1);

    let (mut doc, ids) = built();
    let steps = reparent(&doc, &[ids.b, ids.c], doc.root()).unwrap();
    apply(&mut doc, steps);
    assert_eq!(doc.parent(ids.b), Some(doc.root()));
    assert_eq!(doc.parent(ids.c), Some(ids.b), "c moved with b, its top");
    assert!(children_of(&doc, ids.b, DocKind::Connector).is_empty());
    assert_eq!(
        children_of(&doc, ids.c, DocKind::Connector).len(),
        1,
        "c keeps its own"
    );

    let (mut doc, ids) = built();
    let before = snapshot(doc.tree());
    let steps = reparent(&doc, &[ids.a], ids.c).unwrap();
    assert!(matches!(
        run(&mut doc, steps),
        Err(MutationError::Tree(TreeError::Cycle { .. }))
    ));
    assert_eq!(snapshot(doc.tree()), before, "the connector step is undone too");
}

#[test]
fn test_document_compose_switches_link_kind() {
    do_document_compose_switches_link_kind();
}

/// A line link becomes a portal with its two ends and nothing of a
/// line's; a portal becomes a line without its ends; a link already of
/// the kind takes no step.
pub fn do_document_compose_switches_link_kind() {
    let (mut doc, ids) = built();
    let steps = set_link_kind(&doc, ids.line, LinkKind::Portal).unwrap();
    assert_eq!(steps.len(), 3, "two ends, then the switch");
    apply(&mut doc, steps);
    let kinds: Vec<DocKind> = doc
        .children(ids.line)
        .filter_map(|c| doc.get(c).map(DocElement::kind))
        .collect();
    assert_eq!(kinds, vec![DocKind::PortalEnd, DocKind::PortalEnd]);
    assert!(matches!(doc.get(ids.line), Some(DocElement::Link(l)) if l.kind == LinkKind::Portal));

    let steps = set_link_kind(&doc, ids.portal, LinkKind::Line).unwrap();
    apply(&mut doc, steps);
    assert!(doc.children(ids.portal).next().is_none());
    assert!(matches!(doc.get(ids.portal), Some(DocElement::Link(l)) if l.kind == LinkKind::Line));

    assert!(set_link_kind(&doc, ids.portal, LinkKind::Line)
        .unwrap()
        .is_empty());
}

#[test]
fn test_document_compose_styles_a_range() {
    do_document_compose_styles_a_range();
}

/// `style_range` splits at both cluster boundaries, styles what is
/// inside, and merges the result with equal neighbors: a range across an
/// emoji and a span edge leaves three spans, the middle one joining the
/// two styled parts.
pub fn do_document_compose_styles_a_range() {
    let plain = TextStyle::default();
    let (mut doc, _, section) = section_of(vec![
        span_of(&format!("ab{FAMILY}"), plain.clone(), None),
        span_of("cd", plain.clone(), None),
    ]);
    let steps = style_range(&doc, section, 1, 4, TextOp::SetItalic(Some(true))).unwrap();
    apply(&mut doc, steps);
    assert_eq!(
        spans(&doc, section),
        vec![
            ("a".to_string(), None),
            (format!("b{FAMILY}c"), Some(true)),
            ("d".to_string(), None),
        ]
    );

    // On span edges nothing is split.
    let (mut doc, _, section) = section_of(vec![
        span_of("ab", plain.clone(), None),
        span_of("cd", italic(Some(true)), None),
    ]);
    let ids_before: Vec<NodeId> = doc.children(section).collect();
    let steps = style_range(&doc, section, 0, 2, TextOp::SetItalic(Some(true))).unwrap();
    apply(&mut doc, steps);
    assert_eq!(spans(&doc, section), vec![("abcd".to_string(), Some(true))]);
    assert_eq!(
        doc.children(section).next(),
        Some(ids_before[0]),
        "the first span absorbed the second"
    );

    assert!(style_range(&doc, section, 3, 3, TextOp::SetItalic(None))
        .unwrap()
        .is_empty());
}

#[test]
fn test_document_compose_normalizes_spans() {
    do_document_compose_normalizes_spans();
}

/// A run of three spans made equal merges into one, which one walk
/// cannot do; neighbors whose links differ stay apart.
pub fn do_document_compose_normalizes_spans() {
    let (mut doc, _, section) = section_of(vec![
        span_of("a", italic(Some(false)), None),
        span_of("b", italic(None), None),
        span_of("c", italic(Some(true)), None),
        span_of(
            MARKED,
            italic(Some(false)),
            Some(LinkTarget::Url("x".to_string())),
        ),
    ]);
    let steps = style_range(&doc, section, 0, 3, TextOp::SetItalic(Some(true))).unwrap();
    apply(&mut doc, steps);
    assert_eq!(
        spans(&doc, section),
        vec![("abc".to_string(), Some(true)), (MARKED.to_string(), Some(false))]
    );

    let url = |u: &str| Some(LinkTarget::Url(u.to_string()));
    let (mut doc, _, section) = section_of(vec![
        span_of("a", TextStyle::default(), url("one")),
        span_of("b", TextStyle::default(), url("two")),
    ]);
    let steps = style_range(&doc, section, 0, 2, TextOp::SetItalic(Some(true))).unwrap();
    apply(&mut doc, steps);
    assert_eq!(
        spans(&doc, section),
        vec![("a".to_string(), Some(true)), ("b".to_string(), Some(true))],
        "different links keep the spans apart"
    );
}

#[test]
fn test_document_compose_inserts_and_deletes_text() {
    do_document_compose_inserts_and_deletes_text();
}

/// `insert_text` at a span edge extends the span before it, inside an
/// emoji's neighbor at a cluster boundary, past the end the last span,
/// and in an empty section a new span; `delete_text` across an emoji and
/// a span edge removes exactly those clusters and joins what is left when
/// it is equal, and keeps it apart when it is not.
pub fn do_document_compose_inserts_and_deletes_text() {
    let plain = TextStyle::default();
    let (mut doc, _, section) = section_of(vec![
        span_of(&format!("a{FAMILY}b"), plain.clone(), None),
        span_of("cd", italic(Some(true)), None),
    ]);
    let steps = insert_text(&doc, section, 2, "X".to_string()).unwrap();
    apply(&mut doc, steps);
    assert_eq!(spans(&doc, section)[0].0, format!("a{FAMILY}Xb"));
    let steps = insert_text(&doc, section, 4, "Y".to_string()).unwrap();
    apply(&mut doc, steps);
    assert_eq!(
        spans(&doc, section)[0].0,
        format!("a{FAMILY}XbY"),
        "a span edge extends the span before"
    );
    let steps = insert_text(&doc, section, 99, "Z".to_string()).unwrap();
    apply(&mut doc, steps);
    assert_eq!(spans(&doc, section)[1], ("cdZ".to_string(), Some(true)));

    let (mut doc, _, section) = section_of(vec![]);
    let steps = insert_text(&doc, section, 0, "new".to_string()).unwrap();
    apply(&mut doc, steps);
    assert_eq!(spans(&doc, section), vec![("new".to_string(), None)]);

    let (mut doc, _, section) = section_of(vec![
        span_of(&format!("a{FAMILY}{MARKED}b"), plain.clone(), None),
        span_of("cd", plain.clone(), None),
    ]);
    let steps = delete_text(&doc, section, 1, 3).unwrap();
    apply(&mut doc, steps);
    assert_eq!(
        spans(&doc, section),
        vec![("abcd".to_string(), None)],
        "joined back into one"
    );

    let (mut doc, _, section) = section_of(vec![
        span_of(&format!("a{FAMILY}"), plain.clone(), None),
        span_of(&format!("{MARKED}d"), italic(Some(true)), None),
    ]);
    let steps = delete_text(&doc, section, 1, 3).unwrap();
    apply(&mut doc, steps);
    assert_eq!(
        spans(&doc, section),
        vec![("a".to_string(), None), ("d".to_string(), Some(true))],
        "unequal neighbors stay apart"
    );

    let steps = delete_text(&doc, section, 0, 50).unwrap();
    apply(&mut doc, steps);
    assert!(
        spans(&doc, section).is_empty(),
        "an end past the text deletes to the end"
    );
}

#[test]
fn test_document_compose_splits_and_adds_sections() {
    do_document_compose_splits_and_adds_sections();
}

/// `split_section` before a cluster inside a span moves the rest to a
/// new section after it, on its channel and with its offset but no
/// minimum size; `add_section` adds an empty one after a given section
/// or first.
pub fn do_document_compose_splits_and_adds_sections() {
    let plain = TextStyle::default();
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let node2 = builder.node(root);
    let section2 = builder.add(
        node2,
        DocElement::Section(SectionData {
            offset: Vec2::new(3.0, 4.0),
            min_size: Some(Vec2::new(10.0, 10.0)),
            ..SectionData::default()
        }),
    );
    builder.add(section2, span_of(&format!("ab{FAMILY}"), plain.clone(), None));
    builder.add(section2, span_of("cd", italic(Some(true)), None));
    let mut doc = builder.build().unwrap();

    let steps = split_section(&doc, section2, 2).unwrap();
    apply(&mut doc, steps);
    let sections = children_of(&doc, node2, DocKind::Section);
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0], section2);
    assert_eq!(spans(&doc, sections[0]), vec![("ab".to_string(), None)]);
    assert_eq!(
        spans(&doc, sections[1]),
        vec![(FAMILY.to_string(), None), ("cd".to_string(), Some(true))]
    );
    let Some(DocElement::Section(new)) = doc.get(sections[1]) else { panic!("a section") };
    assert_eq!(
        (new.channel, new.offset, new.min_size),
        (0, Vec2::new(3.0, 4.0), None)
    );

    let steps = add_section(&doc, node2, Some(section2)).unwrap();
    apply(&mut doc, steps);
    let sections_after = children_of(&doc, node2, DocKind::Section);
    assert_eq!(sections_after.len(), 3);
    assert_eq!((sections_after[0], sections_after[2]), (sections[0], sections[1]));
    assert!(doc.children(sections_after[1]).next().is_none());

    let steps = add_section(&doc, node2, None).unwrap();
    apply(&mut doc, steps);
    let first = children_of(&doc, node2, DocKind::Section)[0];
    assert!(!sections_after.contains(&first), "the new section is first");
    assert!(matches!(
        doc.get(first),
        Some(DocElement::Section(SectionData { channel: 0, .. }))
    ));

    assert!(matches!(
        add_section(&doc, node2, Some(node2)),
        Err(MutationError::NotA { .. })
    ));
}

#[test]
fn test_document_compose_normalizes_in_log_steps() {
    do_document_compose_normalizes_in_log_steps();
}

/// Normalization takes `ceil(log2(spans)) + 1` steps (R14.6), counted
/// from the table below, not from `merge_steps` itself: `style_range`
/// over a section of four spans emits its two splits, the styling and
/// four merges (it can hold six spans). Eight equal spans — the longest
/// run three steps can merge — become one, which a step fewer cannot
/// do: run alone, two merge steps leave two spans.
pub fn do_document_compose_normalizes_in_log_steps() {
    for (spans, steps) in [
        (0, 1),
        (1, 1),
        (2, 2),
        (3, 3),
        (4, 3),
        (5, 4),
        (8, 4),
        (9, 5),
        (1024, 11),
    ] {
        assert_eq!(merge_steps(spans), steps, "{spans} spans");
    }

    let plain = TextStyle::default();
    let four: Vec<DocElement> = ["a", "b", "c", "d"]
        .iter()
        .map(|t| span_of(t, plain.clone(), None))
        .collect();
    let (doc, _, section) = section_of(four);
    let steps = style_range(&doc, section, 1, 3, TextOp::SetItalic(Some(true))).unwrap();
    assert_eq!(steps.len(), 3 + 4, "two splits, the styling, four merges");

    // Eight spans that differ only in slant, all made italic: one run of
    // eight, merged in three steps.
    let eight: Vec<DocElement> = (0..8)
        .map(|n| span_of(&n.to_string(), italic((n % 2 == 0).then_some(false)), None))
        .collect();
    let (mut doc, _, section) = section_of(eight);
    let steps = style_range(&doc, section, 0, 8, TextOp::SetItalic(Some(true))).unwrap();
    apply(&mut doc, steps);
    assert_eq!(spans(&doc, section), vec![("01234567".to_string(), Some(true))]);

    // Control: the same run given two merge steps is not done.
    let eight: Vec<DocElement> = (0..8)
        .map(|n| span_of(&n.to_string(), italic(Some(true)), None))
        .collect();
    let (mut doc, _, section) = section_of(eight);
    let mut steps = insert_text(&doc, section, 0, "x".to_string()).unwrap();
    steps.truncate(1 + 2);
    apply(&mut doc, steps);
    assert_eq!(spans(&doc, section).len(), 2, "two halvings of eight leave two");
}

/// How deep the chain of the climb-count test goes: deep enough that a
/// climb per node would be hundreds of millions of steps, shallow enough
/// that the control which replants one still finishes.
const DEEP: usize = 20_000;

/// A document holding one chain of `DEEP` nodes below a root node, and
/// the chain from the top down.
fn chain() -> (Document, Vec<NodeId>) {
    let mut builder = DocumentBuilder::new();
    let mut at = builder.root();
    let mut nodes = Vec::with_capacity(DEEP);
    for _ in 0..DEEP {
        at = builder.node(at);
        nodes.push(at);
    }
    (builder.build().unwrap(), nodes)
}

#[test]
fn test_document_compose_trims_a_deep_selection_once_per_ancestor() {
    do_document_compose_trims_a_deep_selection_once_per_ancestor();
}

/// The climbs `tops` makes go through each ancestor once over the whole
/// selection (§20 rule 11, R14.5), counted in steps, not timed.
///
/// - Every node of a `DEEP` chain, deepest first: the top is the one
///   top, in at most two climbs' worth of the chain (membership and
///   nesting).
/// - A chain of 2,000 with 2,000 leaves at its bottom, the chain's top
///   and every leaf selected: each leaf is nested, found by a climb that
///   stops at the bottom of the chain after the first, so the steps are
///   about the chain plus the leaves, where a climb per leaf would be
///   their product, four million.
pub fn do_document_compose_trims_a_deep_selection_once_per_ancestor() {
    let (doc, nodes) = chain();
    let deepest_first: Vec<NodeId> = nodes.iter().rev().copied().collect();
    let (tops, climbed) = tops_counting(&doc, &deepest_first);
    assert_eq!(tops.unwrap(), vec![nodes[0]]);
    assert!(climbed <= 2 * DEEP + 2, "{climbed} steps for {DEEP} nodes");

    const SIDE: usize = 2_000;
    let mut builder = DocumentBuilder::new();
    let mut at = builder.root();
    let mut spine = Vec::with_capacity(SIDE);
    for _ in 0..SIDE {
        at = builder.node(at);
        spine.push(at);
    }
    let leaves: Vec<NodeId> = (0..SIDE).map(|_| builder.node(at)).collect();
    let doc = builder.build().unwrap();
    let mut selection = vec![spine[0]];
    selection.extend(&leaves);
    let (tops, climbed) = tops_counting(&doc, &selection);
    assert_eq!(tops.unwrap(), vec![spine[0]]);
    assert!(
        climbed <= 4 * SIDE,
        "{climbed} steps for {SIDE} ancestors and {SIDE} leaves"
    );

    let mut members = Membership::default();
    for &leaf in &leaves {
        assert!(members.contains(doc.tree(), leaf));
    }
    assert!(
        members.climbed() <= 3 * SIDE,
        "{} steps for {SIDE} ancestors and {SIDE} leaves",
        members.climbed()
    );
}
