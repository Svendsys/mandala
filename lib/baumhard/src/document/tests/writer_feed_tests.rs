// SPDX-License-Identifier: MPL-2.0

//! What a change record carries for the projector beside its touches
//! (ruling B7.2 on #241): the resolution input kinds each element the
//! change left in the tree with another value touched
//! ([`ChangeRecord::inputs`](crate::document::writer::feed::ChangeRecord::inputs)),
//! and every parent whose children row it rearranged
//! ([`ChangeRecord::rows`](crate::document::writer::feed::ChangeRecord::rows)).
//! Both are read from the journals the change wrote, in the direction
//! it went.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2): every `do_*` body
//! has a bench entry in `benches/test_bench.rs`.

use crate::arena::NodeId;
use crate::document::deps::InputKind;
use crate::document::element::DocKind;
use crate::document::mutation::compose::reparent;
use crate::document::mutation::ops::{DocMutation, NodeOp, SpanOp, TextOp};
use crate::document::tests::writer_support::{built, request, single};
use crate::document::values::{ColorRef, Rgba};
use crate::document::writer::feed::ChangeRecord;
use crate::document::writer::transaction::TransactionKind;
use crate::document::writer::{ChangeSource, EditRequest};
use crate::engine::journal::Touch;
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use crate::engine::structure::StructOp;
use glam::Vec2;
use rustc_hash::FxHashSet;

/// The request that colors every span of `spans` `rgba`.
fn color_spans(spans: Vec<NodeId>, rgba: Rgba) -> EditRequest {
    request(
        single(
            DocKind::Span,
            DocMutation::Span(SpanOp::Text(TextOp::SetColor(Some(ColorRef::Rgba(rgba))))),
        ),
        spans,
        "color",
    )
}

/// The `(element, kind)` pairs a record's inputs name, as a set.
fn kinds(record: &ChangeRecord) -> FxHashSet<(NodeId, InputKind)> {
    record.inputs.iter().copied().collect()
}

#[test]
fn test_writer_feed_reports_input_kinds_and_undo_reverses_them() {
    do_writer_feed_reports_input_kinds_and_undo_reverses_them();
}

/// An edit's record names, for each element it changed, the input kinds
/// it touched: a span's color is its style, a node's palette its binding,
/// and a node's position no input at all, so a drag dirties no
/// resolution. The undo's record names the same kinds again, read from
/// the edit's journal with its two images swapped, and so does the redo's.
pub fn do_writer_feed_reports_input_kinds_and_undo_reverses_them() {
    let (mut doc, ids) = built();
    let edit = EditRequest::new(
        [
            color_spans(vec![ids.a_title_span], Rgba::opaque(1, 2, 3)),
            request(
                single(DocKind::Node, DocMutation::Node(NodeOp::SetPalette(None))),
                vec![ids.a],
                "unbind",
            ),
            request(
                single(
                    DocKind::Node,
                    DocMutation::Node(NodeOp::Nudge(Vec2::new(5.0, 0.0))),
                ),
                vec![ids.d],
                "nudge",
            ),
        ]
        .into_iter()
        .flat_map(|req| req.steps)
        .collect(),
        "three",
    );
    doc.apply(edit, ChangeSource::Action).unwrap();
    let expected: FxHashSet<_> = [
        (ids.a_title_span, InputKind::Style),
        (ids.a, InputKind::PaletteBinding),
    ]
    .into_iter()
    .collect();
    let records = doc.take_changes();
    assert_eq!(records.len(), 1);
    assert!(
        records[0].touched.contains(&(ids.d, Touch::Field)),
        "the nudge is a touch: the fixture moved d"
    );
    assert_eq!(
        kinds(&records[0]),
        expected,
        "a color is style, a palette a binding, a position no input"
    );
    assert!(records[0].rows.is_empty(), "no row was rearranged");

    doc.undo().unwrap();
    let records = doc.take_changes();
    assert_eq!(records[0].source, ChangeSource::Undo);
    assert_eq!(
        kinds(&records[0]),
        expected,
        "the undo touches what the edit touched, the other way"
    );
    doc.redo().unwrap();
    let records = doc.take_changes();
    assert_eq!(kinds(&records[0]), expected, "and so does the redo");
}

#[test]
fn test_writer_feed_a_stage_replacing_a_stage_reports_both_journals() {
    do_writer_feed_a_stage_replacing_a_stage_reports_both_journals();
}

/// A stage replaces the one before it: its record names what the earlier
/// stage's journal took back and what its own journal wrote, so a reader
/// re-resolves the span the first stage colored as well as the span the
/// second one does.
pub fn do_writer_feed_a_stage_replacing_a_stage_reports_both_journals() {
    let (mut doc, ids) = built();
    let tx = doc
        .begin(ChangeSource::Input, TransactionKind::Interactive)
        .unwrap();
    doc.stage(&tx, color_spans(vec![ids.a_title_span], Rgba::opaque(1, 2, 3)))
        .unwrap();
    let first = doc.take_changes();
    assert_eq!(
        kinds(&first[0]),
        [(ids.a_title_span, InputKind::Style)].into_iter().collect(),
        "the first stage colors the title span"
    );
    doc.stage(&tx, color_spans(vec![ids.b_span], Rgba::opaque(4, 5, 6)))
        .unwrap();
    let second = doc.take_changes();
    assert_eq!(second.len(), 1, "one record for the second stage");
    assert_eq!(
        kinds(&second[0]),
        [
            (ids.a_title_span, InputKind::Style),
            (ids.b_span, InputKind::Style)
        ]
        .into_iter()
        .collect(),
        "the span taken back and the span colored"
    );
    doc.cancel(tx);
}

#[test]
fn test_writer_feed_removing_a_first_section_reports_its_nodes_row() {
    do_writer_feed_removing_a_first_section_reports_its_nodes_row();
}

/// Removing a node's first section rearranges the node's row — its
/// second section is now its first, and takes the palette's title color —
/// so the record names the node among its rows, though the node's own
/// value did not change. The undo, which puts the section back, names it
/// too.
pub fn do_writer_feed_removing_a_first_section_reports_its_nodes_row() {
    let (mut doc, ids) = built();
    let remove = request(
        crate::document::mutation::ops::DocMutatorTree::new_with(Mutator::new_structure(
            StructOp::Remove,
            BranchKey::new(DocKind::Section, 0),
        )),
        vec![ids.a_title],
        "remove the title",
    );
    doc.apply(remove, ChangeSource::Action).unwrap();
    let records = doc.take_changes();
    assert!(
        records[0].touched.contains(&(ids.a_title, Touch::Removed)),
        "the fixture removed the title"
    );
    assert!(
        !records[0].touched.iter().any(|&(id, _)| id == ids.a),
        "the node itself is not touched: only its row says what changed"
    );
    assert!(records[0].rows.contains(&ids.a), "a's row: {:?}", records[0].rows);
    doc.undo().unwrap();
    let records = doc.take_changes();
    assert!(
        records[0].rows.contains(&ids.a),
        "the undo puts the title back into a's row"
    );
}

#[test]
fn test_writer_feed_a_move_reports_both_parents() {
    do_writer_feed_a_move_reports_both_parents();
}

/// Re-parenting `c` from `b` to `d` rearranges both rows, and the record
/// names both parents; the undo moves it back and names them again.
pub fn do_writer_feed_a_move_reports_both_parents() {
    let (mut doc, ids) = built();
    let steps = reparent(&doc, &[ids.c], ids.d).unwrap();
    doc.apply(EditRequest::new(steps, "move c"), ChangeSource::Action)
        .unwrap();
    let records = doc.take_changes();
    assert!(
        records[0].touched.contains(&(ids.c, Touch::Moved)),
        "the fixture re-parented c"
    );
    let rows: FxHashSet<NodeId> = records[0].rows.iter().copied().collect();
    assert!(
        rows.contains(&ids.b) && rows.contains(&ids.d),
        "the old parent and the new: {rows:?}"
    );
    doc.undo().unwrap();
    let records = doc.take_changes();
    let rows: FxHashSet<NodeId> = records[0].rows.iter().copied().collect();
    assert!(
        rows.contains(&ids.b) && rows.contains(&ids.d),
        "both again on the way back: {rows:?}"
    );
}
