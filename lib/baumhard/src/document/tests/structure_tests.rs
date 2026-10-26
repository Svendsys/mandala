// SPDX-License-Identifier: MPL-2.0

//! The document under the engine's structural commands: a span splits
//! and merges at grapheme boundaries (`TreeElement::split` / `merge`,
//! baumhard §B3), journaled like any command, and
//! `Document::validate_subtree` judges every row of a subtree before it
//! is inserted.

use crate::document::element::{
    ConnectorData, DocElement, DocKind, FrameData, LinkData, MapData, PortalEndData, SectionData, SpanData,
    TextStyle,
};
use crate::document::ids::Document;
use crate::document::shape::{Cardinality, RowError, ShapeError};
use crate::document::tests::test_support::{node_data, path_data, span, DocumentBuilder};
use crate::document::values::{LinkKind, LinkTarget};
use crate::engine::anchor::Anchors;
use crate::engine::element::TreeElement;
use crate::engine::journal::JournalBuilder;
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tree::{MutatorTree, TreeError};
use crate::util::grapheme_chad::count_grapheme_clusters;
use indextree::NodeId;

/// Four grapheme clusters: `a`, a family emoji of five code points
/// joined by zero-width joiners, an `e` carrying two combining marks,
/// and `b`.
const TEXT: &str = "a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}e\u{0301}\u{0302}b";

/// A span of [`TEXT`] in italics, linking to a web address.
fn styled_span() -> DocElement {
    DocElement::Span(SpanData {
        text: TEXT.to_string(),
        style: TextStyle {
            italic: Some(true),
            ..TextStyle::default()
        },
        link: Some(LinkTarget::Url("https://example.org".to_string())),
    })
}

/// The text of a span element.
fn text(element: &DocElement) -> &str {
    match element {
        DocElement::Span(span) => &span.text,
        other => panic!("not a span: {other:?}"),
    }
}

#[test]
fn test_document_span_splits_and_merges_at_grapheme_boundaries() {
    do_document_span_splits_and_merges_at_grapheme_boundaries();
}

/// A span of four clusters, among them a five-code-point emoji and a
/// letter with two combining marks, splits at cluster 2 into `a` plus
/// the emoji, and the marked `e` plus `b` — where a split at the second
/// `char` would have cut the emoji in two — and both parts keep its
/// style and link. Merging them gives the span back. A split at 0, at
/// the cluster count or past it would leave a part empty and is
/// refused, at every cluster in between it succeeds, and a section
/// never splits.
pub fn do_document_span_splits_and_merges_at_grapheme_boundaries() {
    let whole = styled_span();
    assert_eq!(count_grapheme_clusters(TEXT), 4, "the fixture is four clusters");
    let char_cut: String = TEXT.chars().take(2).collect();
    assert_ne!(
        char_cut, "a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        "a char split would cut the emoji"
    );

    let (first, second) = whole.split(&2).unwrap();
    assert_eq!(text(&first), "a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}");
    assert_eq!(text(&second), "e\u{0301}\u{0302}b");
    for part in [&first, &second] {
        let (DocElement::Span(part), DocElement::Span(original)) = (part, &whole) else {
            panic!("a span splits into spans");
        };
        assert_eq!(part.style, original.style, "each part keeps the style");
        assert_eq!(part.link, original.link, "and the link");
        assert_eq!(part_key(part), whole.key(), "and the key");
    }
    assert_eq!(
        first.merge(&second),
        Some(whole.clone()),
        "merging gives the span back"
    );

    for at in [0, 4, 9] {
        assert_eq!(whole.split(&at), None, "a split at {at} leaves a part empty");
    }
    for at in 1..4 {
        assert!(
            whole.split(&at).is_some(),
            "a split at {at} cuts between clusters"
        );
    }
    let section = DocElement::Section(SectionData::default());
    assert_eq!(section.split(&1), None, "a section never splits");
}

/// The key a span part routes on.
fn part_key(span: &SpanData) -> BranchKey<DocKind> {
    DocElement::Span(span.clone()).key()
}

#[test]
fn test_document_span_merge_is_refused_for_different_styles_or_links() {
    do_document_span_merge_is_refused_for_different_styles_or_links();
}

/// Two spans merge only when their styles and links are equal: one
/// style field different, or one link different, refuses the merge. The
/// control: the same two spans with equal styles and links merge. A
/// span never merges with another kind, nor one section with another.
pub fn do_document_span_merge_is_refused_for_different_styles_or_links() {
    let (left, right) = styled_span().split(&1).unwrap();
    assert!(
        left.merge(&right).is_some(),
        "the control: equal styles and links merge"
    );

    let mut bold = right.clone();
    if let DocElement::Span(span) = &mut bold {
        span.style.italic = Some(false);
    }
    assert_eq!(left.merge(&bold), None, "a different style refuses the merge");
    let mut unlinked = right.clone();
    if let DocElement::Span(span) = &mut unlinked {
        span.link = None;
    }
    assert_eq!(left.merge(&unlinked), None, "a different link refuses the merge");
    let section = DocElement::Section(SectionData::default());
    assert_eq!(left.merge(&section), None);
    assert_eq!(section.merge(&section.clone()), None);
}

#[test]
fn test_document_span_split_and_merge_are_journaled_commands() {
    do_document_span_split_and_merge_are_journaled_commands();
}

/// In a document tree, a `Split` on a section's span leaves the span
/// and a new span after it, and the journal reverts it to one span and
/// replays it to two, with the same new id. A `Merge` of two spans whose
/// styles differ is `Unmergeable`, and a `Split` on the section itself
/// is `Unsplittable`; neither changes the tree.
pub fn do_document_span_split_and_merge_are_journaled_commands() {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let node = builder.node(root);
    let section = builder.section(node, 0);
    let first = builder.add(section, styled_span());
    let plain = builder.add(section, span("plain"));
    let mut tree = builder.tree().clone();
    let children =
        |tree: &crate::engine::tree::Tree<DocElement>| section.children(tree.arena()).collect::<Vec<_>>();
    let command = |op, kind| {
        MutatorTree::new_with(Mutator::<DocElement, NoMutation>::new_structure(
            op,
            BranchKey::new(kind, 0),
        ))
    };
    let at = |id: NodeId| Anchors::Nodes(vec![id]);

    let mut journal = JournalBuilder::new();
    command(StructOp::Split { at: 3 }, DocKind::Span)
        .apply_at_journaled(&mut tree, &at(first), |_, _| (), &mut journal)
        .unwrap();
    let after = children(&tree);
    assert_eq!(after.len(), 3);
    assert_eq!((after[0], after[2]), (first, plain));
    assert_eq!(text(tree.arena()[after[1]].get()), "b");
    let journal = journal.close(&tree);
    journal.revert(&mut tree).unwrap();
    assert_eq!(children(&tree), vec![first, plain]);
    assert_eq!(text(tree.arena()[first].get()), TEXT);
    journal.replay(&mut tree).unwrap();
    assert_eq!(children(&tree), after, "the same new span");

    let held = children(&tree);
    let result = command(StructOp::Merge, DocKind::Span).apply_at_journaled(
        &mut tree,
        &at(after[1]),
        |_, _| (),
        &mut JournalBuilder::new(),
    );
    assert_eq!(result, Err(TreeError::Unmergeable(after[1])));
    let result = command(StructOp::Split { at: 1 }, DocKind::Section).apply_at_journaled(
        &mut tree,
        &at(section),
        |_, _| (),
        &mut JournalBuilder::new(),
    );
    assert_eq!(result, Err(TreeError::Unsplittable(section)));
    assert_eq!(children(&tree), held);
}

/// A mutation type with no mutation: these walks carry structural
/// commands only.
#[derive(Clone, Debug)]
pub enum NoMutation {}

impl crate::engine::mutation::Mutation<DocElement> for NoMutation {
    type Template = ();

    fn apply_to(&self, _target: &mut DocElement, _inherited: &()) {
        match *self {}
    }
}

// ── validate_subtree ──────────────────────────────────────────────

/// A node with a section holding a span, a frame, and a child node with
/// its connector and a section — every kind of row a node subtree has.
fn node_subtree(with_connector: bool) -> Subtree<DocElement> {
    let mut children = vec![
        Subtree {
            root: DocElement::Section(SectionData::default()),
            children: vec![Subtree::leaf(span("title"))],
        },
        Subtree::leaf(DocElement::Frame(FrameData::default())),
        Subtree {
            root: DocElement::Node(node_data(0)),
            children: vec![
                Subtree::leaf(connector()),
                Subtree::leaf(DocElement::Section(SectionData::default())),
            ],
        },
    ];
    if with_connector {
        children.push(Subtree::leaf(connector()));
    }
    Subtree {
        root: DocElement::Node(node_data(0)),
        children,
    }
}

/// A connector with no style of its own.
fn connector() -> DocElement {
    DocElement::Connector(ConnectorData { path: path_data() })
}

/// A portal link to `to` with ends on the given channels.
fn portal(to: NodeId, ends: [u8; 2]) -> Subtree<DocElement> {
    Subtree {
        root: DocElement::Link(LinkData {
            to,
            kind: LinkKind::Portal,
            path: path_data(),
        }),
        children: ends
            .iter()
            .map(|&end| {
                Subtree::leaf(DocElement::PortalEnd(PortalEndData {
                    end,
                    ..PortalEndData::default()
                }))
            })
            .collect(),
    }
}

/// The row error `validate_subtree` reports, with where it sits.
fn row_error(subtree: &Subtree<DocElement>) -> Result<(), (Vec<usize>, RowError)> {
    match Document::validate_subtree(subtree) {
        Ok(()) => Ok(()),
        Err(ShapeError::SubtreeRow { path, error }) => Err((path, error)),
        Err(other) => panic!("not a subtree row error: {other:?}"),
    }
}

#[test]
fn test_document_validate_subtree_judges_every_row() {
    do_document_validate_subtree_judges_every_row();
}

/// A valid node subtree passes, its root with no connector or with one —
/// where it lands decides which is right, and that is `validate_shape`'s
/// to judge once it is in place. Each kind of row violation is reported
/// with where it sits, and each has a control — the same subtree, fixed
/// — that passes:
/// - a map inside the subtree (`MapNotRoot`);
/// - a span directly under a node (`ChildNotAllowed`);
/// - two frames under a node (`Count`);
/// - a child node with no connector (`Count`), and a root with two;
/// - a portal link whose ends are both channel 0 (`PortalEnds`).
pub fn do_document_validate_subtree_judges_every_row() {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let target = builder.node(root);
    assert_eq!(row_error(&node_subtree(false)), Ok(()));
    assert_eq!(row_error(&node_subtree(true)), Ok(()));

    let mut with_map = node_subtree(false);
    with_map.children[0]
        .children
        .push(Subtree::leaf(DocElement::Map(MapData::default())));
    assert_eq!(
        row_error(&with_map),
        Err((
            vec![0],
            RowError::ChildNotAllowed {
                parent_kind: DocKind::Section,
                position: 1,
                child_kind: DocKind::Map
            }
        )),
        "a map is no section's child"
    );
    let map_root = Subtree::leaf(DocElement::Map(MapData::default()));
    assert_eq!(row_error(&map_root), Err((vec![], RowError::MapNotRoot)));

    let mut span_under_node = node_subtree(false);
    span_under_node.children[2]
        .children
        .push(Subtree::leaf(span("loose")));
    assert_eq!(
        row_error(&span_under_node),
        Err((
            vec![2],
            RowError::ChildNotAllowed {
                parent_kind: DocKind::Node,
                position: 2,
                child_kind: DocKind::Span
            }
        ))
    );
    span_under_node.children[2].children[2] = Subtree {
        root: DocElement::Section(SectionData::default()),
        children: vec![Subtree::leaf(span("loose"))],
    };
    assert_eq!(
        row_error(&span_under_node),
        Ok(()),
        "control: the span in a section"
    );

    let mut two_frames = node_subtree(false);
    two_frames
        .children
        .push(Subtree::leaf(DocElement::Frame(FrameData::default())));
    assert_eq!(
        row_error(&two_frames),
        Err((
            vec![],
            RowError::Count {
                kind: DocKind::Frame,
                found: 2,
                allowed: Cardinality::AtMost(1)
            }
        ))
    );
    two_frames.children.pop();
    assert_eq!(row_error(&two_frames), Ok(()), "control: one frame");

    let mut no_connector = node_subtree(false);
    no_connector.children[2].children.remove(0);
    assert_eq!(
        row_error(&no_connector),
        Err((
            vec![2],
            RowError::Count {
                kind: DocKind::Connector,
                found: 0,
                allowed: Cardinality::Exactly(1)
            }
        )),
        "a node under a node has exactly one connector"
    );
    let mut two_connectors = node_subtree(true);
    two_connectors.children.push(Subtree::leaf(connector()));
    assert_eq!(
        row_error(&two_connectors),
        Err((
            vec![],
            RowError::Count {
                kind: DocKind::Connector,
                found: 2,
                allowed: Cardinality::Exactly(1)
            }
        ))
    );

    let mut bad_portal = node_subtree(false);
    bad_portal.children.push(portal(target, [0, 0]));
    assert_eq!(row_error(&bad_portal), Err((vec![3], RowError::PortalEnds)));
    bad_portal.children[3] = portal(target, [1, 0]);
    assert_eq!(row_error(&bad_portal), Ok(()), "control: one end on each channel");
}
