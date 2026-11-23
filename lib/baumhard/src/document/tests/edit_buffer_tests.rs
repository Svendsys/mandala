// SPDX-License-Identifier: MPL-2.0

//! [`EditBuffer`]: insert, delete and style across span boundaries,
//! multi-codepoint grapheme clusters, normalization, and `main`'s IME
//! sequences ported (the editor's literal-key path inserts a filtered
//! payload as one counted splice). Each operation is held to the span
//! builders it mirrors: the same change made to a document by
//! `insert_text`, `delete_text` or `style_range` leaves the owner with
//! exactly the buffer's spans.
//!
//! The bodies are `do_*` functions so the benches reuse them
//! (TEST_CONVENTIONS §T2.1, baumhard §B8).

use crate::arena::NodeId;
use crate::document::edit_buffer::EditBuffer;
use crate::document::element::DocKind;
use crate::document::element::{DocElement, SpanData, TextStyle};
use crate::document::ids::Document;
use crate::document::mutation::compose::{delete_text, insert_text, replace_spans, style_range};
use crate::document::mutation::error::MutationError;
use crate::document::mutation::ops::{DocMutation, DocMutatorTree, SpanOp, TextOp};
use crate::document::mutation::step::{DocStep, DocWalk};
use crate::document::tests::test_support::{example, DocumentBuilder};
use crate::document::values::{FontSize, Weight};
use crate::document::writer::{ChangeSource, EditRequest};
use crate::engine::anchor::Anchors;
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;

/// The family emoji: one grapheme cluster of five code points.
const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
/// An `e` with two combining marks: one cluster of three code points.
const MARKED: &str = "e\u{0301}\u{0302}";

/// A span of `text` in `style`.
fn styled(text: &str, style: TextStyle) -> SpanData {
    SpanData {
        text: text.to_string(),
        style,
        link: None,
    }
}

/// A bold style.
fn bold() -> TextStyle {
    TextStyle {
        weight: Some(Weight::BOLD),
        ..TextStyle::default()
    }
}

/// An italic style.
fn italic() -> TextStyle {
    TextStyle {
        italic: Some(true),
        ..TextStyle::default()
    }
}

/// A document with one node whose one section holds `spans`, as given
/// (the builder does not normalize), and that section.
fn section_of(spans: &[SpanData]) -> (Document, NodeId) {
    section_of_sized(spans, None)
}

/// [`section_of`], with the section's own text size set to `size`.
fn section_of_sized(spans: &[SpanData], size: Option<FontSize>) -> (Document, NodeId) {
    let mut builder = DocumentBuilder::new();
    let node = builder.node(builder.root());
    let section = builder.section(node, 0);
    builder.edit(section, |element| {
        if let DocElement::Section(data) = element {
            data.text.size = size;
        }
    });
    for span in spans {
        builder.add(section, DocElement::Span(span.clone()));
    }
    (builder.build().unwrap(), section)
}

/// The span children of `owner`, as the document holds them.
fn spans_of(doc: &Document, owner: NodeId) -> Vec<SpanData> {
    doc.children(owner)
        .filter_map(|id| match doc.get(id) {
            Some(DocElement::Span(span)) => Some(span.clone()),
            _ => None,
        })
        .collect()
}

/// Apply `steps` to `doc` as one request.
fn apply(doc: &mut Document, steps: Vec<DocStep>) {
    doc.apply(EditRequest::new(steps, "test"), ChangeSource::Input)
        .unwrap();
}

#[test]
fn test_edit_buffer_reads_every_text_owner() {
    do_edit_buffer_reads_every_text_owner();
}

/// Every element that holds spans gives its text — a section, a ring, a
/// cap, a label, a portal end, a connector, a line link — and an element
/// that holds none is refused. Control: the refusal is not a blanket one,
/// since the section on the same map is read.
pub fn do_edit_buffer_reads_every_text_owner() {
    let (builder, ids) = example();
    let doc = builder.build().unwrap();
    for (owner, text) in [
        (ids.a_title, "title"),
        (ids.ring, "and the earth was without form"),
        (ids.b_cap, ""),
        (ids.b_label, "label"),
        (ids.portal_source, "\u{25C6}"),
        (ids.portal_label, "to d"),
        (ids.b_connector, "\u{2500}"),
        (ids.line, "~"),
        (ids.c_connector, ""),
    ] {
        let buffer = EditBuffer::from_owner(&doc, owner).unwrap();
        assert_eq!(buffer.text(), text, "{owner} reads as its spans");
        assert_eq!(buffer.spans(), spans_of(&doc, owner).as_slice());
    }
    for not_an_owner in [ids.a, ids.frame, ids.portal] {
        assert!(
            matches!(
                EditBuffer::from_owner(&doc, not_an_owner),
                Err(MutationError::NotA {
                    expected: "an element that holds spans",
                    ..
                })
            ),
            "{not_an_owner} holds no spans"
        );
    }
}

#[test]
fn test_edit_buffer_inserts_across_span_boundaries() {
    do_edit_buffer_inserts_across_span_boundaries();
}

/// Text lands in the span that holds the cluster it is inserted before;
/// at a span edge, at the end of the span before, so it takes that span's
/// style; past the end, at the end of the last span; and into a new span
/// with nothing set when there are none. Each placement is the one
/// `insert_text` makes in the document.
pub fn do_edit_buffer_inserts_across_span_boundaries() {
    let spans = [styled("ab", bold()), styled("cd", italic())];
    for (at, want) in [
        (0, vec![styled("Xab", bold()), styled("cd", italic())]),
        (1, vec![styled("aXb", bold()), styled("cd", italic())]),
        (2, vec![styled("abX", bold()), styled("cd", italic())]),
        (3, vec![styled("ab", bold()), styled("cXd", italic())]),
        (4, vec![styled("ab", bold()), styled("cdX", italic())]),
        (99, vec![styled("ab", bold()), styled("cdX", italic())]),
    ] {
        let (mut doc, section) = section_of(&spans);
        let mut buffer = EditBuffer::from_owner(&doc, section).unwrap();
        assert_eq!(buffer.insert(at, "X"), 1);
        assert_eq!(buffer.spans(), want.as_slice(), "insert at {at}");
        let steps = insert_text(&doc, section, at, "X".to_string()).unwrap();
        apply(&mut doc, steps);
        assert_eq!(spans_of(&doc, section), want, "insert_text at {at} agrees");
    }

    let (doc, section) = section_of(&[]);
    let mut buffer = EditBuffer::from_owner(&doc, section).unwrap();
    assert_eq!(buffer.grapheme_len(), 0);
    assert_eq!(buffer.insert(0, "hi"), 2);
    assert_eq!(buffer.spans(), [styled("hi", TextStyle::default())].as_slice());
    assert_eq!(buffer.insert(0, ""), 0, "empty text inserts nothing");
    assert_eq!(buffer.text(), "hi");
}

#[test]
fn test_edit_buffer_deletes_across_span_boundaries() {
    do_edit_buffer_deletes_across_span_boundaries();
}

/// A delete that crosses a span edge trims both spans; one that empties
/// a span removes it; one that leaves two equal spans side by side joins
/// them — and each leaves what `delete_text` leaves in the document.
pub fn do_edit_buffer_deletes_across_span_boundaries() {
    let spans = [styled("ab", bold()), styled("X", italic()), styled("cd", bold())];
    for (start, end, want) in [
        (1, 4, vec![styled("ad", bold())]),
        (2, 3, vec![styled("abcd", bold())]),
        (0, 2, vec![styled("X", italic()), styled("cd", bold())]),
        (
            1,
            2,
            vec![styled("a", bold()), styled("X", italic()), styled("cd", bold())],
        ),
        (3, 99, vec![styled("ab", bold()), styled("X", italic())]),
        (0, 99, vec![]),
    ] {
        let (mut doc, section) = section_of(&spans);
        let mut buffer = EditBuffer::from_owner(&doc, section).unwrap();
        buffer.delete(start, end);
        assert_eq!(buffer.spans(), want.as_slice(), "delete {start}..{end}");
        let steps = delete_text(&doc, section, start, end).unwrap();
        apply(&mut doc, steps);
        assert_eq!(spans_of(&doc, section), want, "delete_text {start}..{end} agrees");
    }
    let (doc, section) = section_of(&spans);
    let mut buffer = EditBuffer::from_owner(&doc, section).unwrap();
    let before = buffer.clone();
    buffer.delete(2, 2);
    buffer.delete(3, 1);
    assert_eq!(buffer, before, "an empty or reversed range deletes nothing");
}

#[test]
fn test_edit_buffer_styles_across_span_boundaries() {
    do_edit_buffer_styles_across_span_boundaries();
}

/// Styling a range splits the spans at both of its ends and styles what
/// is inside, across a span edge; a relative size grows the size the
/// span displays, inherited when it sets none — what `style_range` writes
/// in the document, field for field.
pub fn do_edit_buffer_styles_across_span_boundaries() {
    let spans = [styled("ab", TextStyle::default()), styled("cd", bold())];
    let bold_italic = TextStyle {
        weight: Some(Weight::BOLD),
        italic: Some(true),
        ..TextStyle::default()
    };
    let cases: [(usize, usize, TextOp, Vec<SpanData>); 3] = [
        (
            1,
            3,
            TextOp::SetItalic(Some(true)),
            vec![
                styled("a", TextStyle::default()),
                styled("b", italic()),
                styled("c", bold_italic.clone()),
                styled("d", bold()),
            ],
        ),
        (
            0,
            4,
            TextOp::SetWeight(Some(Weight::BOLD)),
            vec![styled("abcd", bold())],
        ),
        (
            2,
            4,
            TextOp::SetWeight(None),
            vec![styled("abcd", TextStyle::default())],
        ),
    ];
    for (start, end, op, want) in cases {
        let (mut doc, section) = section_of(&spans);
        let mut buffer = EditBuffer::from_owner(&doc, section).unwrap();
        buffer.style(start, end, &op);
        assert_eq!(buffer.spans(), want.as_slice(), "{op:?} on {start}..{end}");
        let steps = style_range(&doc, section, start, end, vec![op.clone()]).unwrap();
        apply(&mut doc, steps);
        assert_eq!(spans_of(&doc, section), want, "style_range {op:?} agrees");
    }

    // A relative size reads the size the owner hands down, as the walk
    // does — here the section's own 20 pt, not the default a span would
    // take from the node: the buffer and the document grow it alike.
    let (mut doc, section) = section_of_sized(&spans, Some(FontSize::Pt(20.0)));
    let mut buffer = EditBuffer::from_owner(&doc, section).unwrap();
    buffer.style(0, 1, &TextOp::GrowSize(3.0));
    assert_eq!(buffer.spans()[0].style.size, Some(FontSize::Pt(23.0)));
    let steps = style_range(&doc, section, 0, 1, vec![TextOp::GrowSize(3.0)]).unwrap();
    apply(&mut doc, steps);
    assert_eq!(buffer.spans(), spans_of(&doc, section).as_slice());

    let (mut doc, section) = section_of(&spans);
    let mut buffer = EditBuffer::from_owner(&doc, section).unwrap();
    buffer.style(0, 1, &TextOp::GrowSize(3.0));
    let steps = style_range(&doc, section, 0, 1, vec![TextOp::GrowSize(3.0)]).unwrap();
    apply(&mut doc, steps);
    assert_eq!(buffer.spans(), spans_of(&doc, section).as_slice());
    let grown = buffer.spans()[0].style.size;
    assert!(
        matches!(grown, Some(FontSize::Pt(size)) if size > 3.0),
        "grown from the inherited size, not from nothing: {grown:?}"
    );

    // A scale reads the inherited size the same way: 20 pt by 1.5.
    let (mut doc, section) = section_of_sized(&spans, Some(FontSize::Pt(20.0)));
    let mut buffer = EditBuffer::from_owner(&doc, section).unwrap();
    buffer.style(0, 1, &TextOp::ScaleSize(1.5));
    assert_eq!(buffer.spans()[0].style.size, Some(FontSize::Pt(30.0)));
    let steps = style_range(&doc, section, 0, 1, vec![TextOp::ScaleSize(1.5)]).unwrap();
    apply(&mut doc, steps);
    assert_eq!(
        buffer.spans(),
        spans_of(&doc, section).as_slice(),
        "style_range agrees"
    );

    // A ring's text is sized relative to its node (`places_in_frame`): a
    // grown or scaled span of it computes a `Rel` size, read from the size
    // the ring hands down, as the engine's span op does. A placed ring's
    // spans carry no size of their own (`PlacedRingSpanSize`), so the
    // writer refuses that span op and the buffer's spans staged whole —
    // as the single-line editor stages them — alike, and the ring is
    // left as it was.
    for op in [TextOp::GrowSize(0.01), TextOp::ScaleSize(1.5)] {
        let (builder, ids) = example();
        let mut doc = builder.build().unwrap();
        let before = spans_of(&doc, ids.ring);
        let mut buffer = EditBuffer::from_owner(&doc, ids.ring).unwrap();
        buffer.style(0, buffer.grapheme_len(), &op);
        assert!(
            matches!(buffer.spans()[0].style.size, Some(FontSize::Rel(_))),
            "{op:?} on a ring computes a relative size: {:?}",
            buffer.spans()[0].style.size
        );
        let spans: Vec<NodeId> = doc
            .children(ids.ring)
            .filter(|&id| matches!(doc.get(id), Some(DocElement::Span(_))))
            .collect();
        let mutator = DocMutatorTree::new_with(Mutator::new(
            DocMutation::Span(SpanOp::Text(op.clone())),
            BranchKey::new(DocKind::Span, 0),
        ));
        let by_op = doc.apply(
            EditRequest::new(
                vec![DocStep::fixed(DocWalk::new(mutator, Anchors::Nodes(spans)))],
                "op",
            ),
            ChangeSource::Input,
        );
        let steps = replace_spans(&doc, &[ids.ring], buffer.spans()).unwrap();
        let by_buffer = doc.apply(EditRequest::new(steps, "buffer"), ChangeSource::Input);
        let refusal =
            |result: &Result<_, _>| result.is_err() && format!("{result:?}").contains("PlacedRingSpanSize");
        assert!(
            refusal(&by_op) && refusal(&by_buffer),
            "{op:?}: refused alike: {by_op:?} {by_buffer:?}"
        );
        assert_eq!(spans_of(&doc, ids.ring), before, "{op:?}: the ring is as it was");
    }

    let (doc, section) = section_of(&spans);
    let mut buffer = EditBuffer::from_owner(&doc, section).unwrap();
    let before = buffer.clone();
    buffer.style(2, 2, &TextOp::SetItalic(Some(true)));
    assert_eq!(buffer, before, "an empty range styles nothing");
}

#[test]
fn test_edit_buffer_counts_multi_codepoint_clusters() {
    do_edit_buffer_counts_multi_codepoint_clusters();
}

/// A five-code-point family and an `e` with two marks are one cluster
/// each, wherever they sit against a span edge: inserting after one
/// steps past the whole cluster, and deleting one removes all of it —
/// and each step leaves what the builder it mirrors leaves in the
/// document.
pub fn do_edit_buffer_counts_multi_codepoint_clusters() {
    let spans = [styled(FAMILY, bold()), styled(MARKED, italic())];
    let (mut doc, section) = section_of(&spans);
    let mut buffer = EditBuffer::from_owner(&doc, section).unwrap();
    assert_eq!(buffer.grapheme_len(), 2);
    assert_eq!(buffer.insert(1, "x"), 1);
    assert_eq!(
        buffer.spans(),
        [styled(&format!("{FAMILY}x"), bold()), styled(MARKED, italic())].as_slice(),
        "after the family, never inside it"
    );
    let steps = insert_text(&doc, section, 1, "x".to_string()).unwrap();
    apply(&mut doc, steps);
    assert_eq!(spans_of(&doc, section), buffer.spans(), "insert_text agrees");

    buffer.delete(0, 1);
    assert_eq!(
        buffer.spans(),
        [styled("x", bold()), styled(MARKED, italic())].as_slice()
    );
    let steps = delete_text(&doc, section, 0, 1).unwrap();
    apply(&mut doc, steps);
    assert_eq!(spans_of(&doc, section), buffer.spans(), "delete_text agrees");

    buffer.delete(1, 2);
    assert_eq!(buffer.text(), "x", "the marked letter goes whole");
    let steps = delete_text(&doc, section, 1, 2).unwrap();
    apply(&mut doc, steps);
    assert_eq!(spans_of(&doc, section), buffer.spans(), "delete_text agrees");

    buffer.style(0, 1, &TextOp::SetItalic(Some(true)));
    assert_eq!(buffer.grapheme_len(), 1);
    let steps = style_range(&doc, section, 0, 1, vec![TextOp::SetItalic(Some(true))]).unwrap();
    apply(&mut doc, steps);
    assert_eq!(spans_of(&doc, section), buffer.spans(), "style_range agrees");
}

#[test]
fn test_edit_buffer_counts_a_cluster_across_a_span_edge_span_by_span() {
    do_edit_buffer_counts_a_cluster_across_a_span_edge_span_by_span();
}

/// A cluster split across a span edge — a bold `e`, then an italic
/// combining acute and an `x` — is counted span by span: three clusters,
/// the mark one of its own at its span's start, as the builders count it.
/// Styled so the two spans are equal, they merge, and the `e` and its mark
/// are one cluster: two. The buffer and the builders agree at each step.
pub fn do_edit_buffer_counts_a_cluster_across_a_span_edge_span_by_span() {
    let spans = [styled("e", bold()), styled("\u{301}x", italic())];
    let (mut doc, section) = section_of(&spans);
    let mut buffer = EditBuffer::from_owner(&doc, section).unwrap();
    assert_eq!(buffer.grapheme_len(), 3, "counted span by span");

    let ops = [TextOp::SetWeight(None), TextOp::SetItalic(Some(true))];
    for op in &ops {
        buffer.style(0, 3, op);
    }
    assert_eq!(
        buffer.spans(),
        [styled("e\u{301}x", italic())].as_slice(),
        "merged"
    );
    assert_eq!(buffer.grapheme_len(), 2, "one cluster once the spans are one");
    let steps = style_range(&doc, section, 0, 3, ops.to_vec()).unwrap();
    apply(&mut doc, steps);
    assert_eq!(spans_of(&doc, section), buffer.spans(), "style_range agrees");
    assert_eq!(
        EditBuffer::from_owner(&doc, section).unwrap().grapheme_len(),
        2,
        "and counts it as the buffer does"
    );
}

#[test]
fn test_edit_buffer_joins_a_crlf_as_main_did() {
    do_edit_buffer_joins_a_crlf_as_main_did();
}

/// `\n` typed after a `\r` joins it into one cluster, as `main`'s
/// grapheme tests had it: the insert advances the caret by none, though
/// the text changed, and deleting the cluster removes both. The builders
/// agree.
pub fn do_edit_buffer_joins_a_crlf_as_main_did() {
    let (mut doc, section) = section_of(&[styled("a\r", TextStyle::default())]);
    let mut buffer = EditBuffer::from_owner(&doc, section).unwrap();
    assert_eq!(buffer.grapheme_len(), 2);
    assert_eq!(buffer.insert(2, "\n"), 0, "the line feed joins the return");
    assert_eq!((buffer.text().as_str(), buffer.grapheme_len()), ("a\r\n", 2));
    let steps = insert_text(&doc, section, 2, "\n".to_string()).unwrap();
    apply(&mut doc, steps);
    assert_eq!(spans_of(&doc, section), buffer.spans(), "insert_text agrees");

    buffer.delete(1, 2);
    assert_eq!(buffer.text(), "a", "a delete takes both");
    let steps = delete_text(&doc, section, 1, 2).unwrap();
    apply(&mut doc, steps);
    assert_eq!(spans_of(&doc, section), buffer.spans(), "delete_text agrees");
}

#[test]
fn test_edit_buffer_normalizes_equal_neighbors_only() {
    do_edit_buffer_normalizes_equal_neighbors_only();
}

/// Read from an owner whose spans are not normalized, the buffer merges
/// neighbors with the same style and link and drops empty spans; the
/// negative control keeps neighbors whose style, or whose link alone,
/// differs.
pub fn do_edit_buffer_normalizes_equal_neighbors_only() {
    let (doc, section) = section_of(&[
        styled("a", bold()),
        styled("", italic()),
        styled("b", bold()),
        styled("c", bold()),
    ]);
    let buffer = EditBuffer::from_owner(&doc, section).unwrap();
    assert_eq!(buffer.spans(), [styled("abc", bold())].as_slice());

    let linked = SpanData {
        link: Some(crate::document::values::LinkTarget::Url(
            "https://example.com".into(),
        )),
        ..styled("c", bold())
    };
    let (doc, section) = section_of(&[
        styled("a", bold()),
        styled("b", italic()),
        linked.clone(),
        styled("d", bold()),
    ]);
    let buffer = EditBuffer::from_owner(&doc, section).unwrap();
    assert_eq!(
        buffer.spans(),
        [
            styled("a", bold()),
            styled("b", italic()),
            linked,
            styled("d", bold())
        ]
        .as_slice(),
        "control: neighbors that differ in style, or in their link alone, are kept"
    );
}

#[test]
fn test_edit_buffer_takes_ime_payloads_as_main_did() {
    do_edit_buffer_takes_ime_payloads_as_main_did();
}

/// `main`'s IME sequences, ported from its single-line and node editors:
/// a multi-character payload advances by its visible clusters (`né` is
/// two), a Hangul jamo sequence composing one syllable advances by one,
/// a combining mark typed after its base merges into it and advances by
/// none while still changing the text, and a base typed before a
/// standalone mark absorbs it and advances by one.
pub fn do_edit_buffer_takes_ime_payloads_as_main_did() {
    let (doc, section) = section_of(&[]);
    let empty = EditBuffer::from_owner(&doc, section).unwrap();
    // Each sequence's inserts made by the builder too, from the same
    // empty section, leave what the buffer holds.
    let agrees = |inserts: &[(usize, &str)], buffer: &EditBuffer| {
        let (mut doc, section) = section_of(&[]);
        for &(at, text) in inserts {
            let steps = insert_text(&doc, section, at, text.to_string()).unwrap();
            apply(&mut doc, steps);
        }
        assert_eq!(
            spans_of(&doc, section),
            buffer.spans(),
            "insert_text agrees: {inserts:?}"
        );
    };

    let mut buffer = empty.clone();
    assert_eq!(buffer.insert(0, "n\u{00E9}"), 2);
    assert_eq!(buffer.text(), "n\u{00E9}");
    agrees(&[(0, "n\u{00E9}")], &buffer);

    let jamo = "\u{1112}\u{1161}\u{11AB}";
    let mut buffer = empty.clone();
    assert_eq!(buffer.insert(0, jamo), 1);
    assert_eq!((buffer.text().as_str(), buffer.grapheme_len()), (jamo, 1));
    agrees(&[(0, jamo)], &buffer);

    let mut buffer = empty.clone();
    buffer.insert(0, "e");
    assert_eq!(buffer.insert(1, "\u{0301}"), 0, "the mark merges into its base");
    assert_eq!(buffer.text(), "e\u{0301}", "but the text did change");
    assert_eq!(buffer.grapheme_len(), 1);
    agrees(&[(0, "e"), (1, "\u{0301}")], &buffer);

    let mut buffer = empty;
    buffer.insert(0, "\u{0301}");
    assert_eq!(buffer.insert(0, "e"), 1, "a base absorbs the mark after it");
    assert_eq!((buffer.text().as_str(), buffer.grapheme_len()), ("e\u{0301}", 1));
    agrees(&[(0, "\u{0301}"), (0, "e")], &buffer);
}
