// SPDX-License-Identifier: MPL-2.0

//! The writer's transactions (V1-B4, #238; design §9, D28, ruling R7):
//! stages that replace each other and commit as one entry, cancel,
//! the base that keeps what a session created, saving with a stage in
//! place, and who yields when a second change arrives — the interactive
//! transaction refuses it, the animated one is fast-forwarded and
//! committed.
//!
//! Every state a test reaches is held to `writer_support::assert_consistent`.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2): every `do_*` body
//! has a bench entry in `benches/test_bench.rs`.

use crate::document::element::DocKind;
use crate::document::mutation::compose::{create_child, create_root, insert_text};
use crate::document::tests::writer_support::{
    assert_consistent, built, children_of, fade, ident, live_slots, place, position, saved,
};
use crate::document::writer::error::ApplyError;
use crate::document::writer::transaction::TransactionKind::{Animated, Interactive};
use crate::document::writer::{ChangeSource, EditRequest, Revision};
use crate::engine::interpolate::Snap;
use crate::engine::journal::Touch;
use glam::Vec2;

#[test]
fn test_writer_thirty_stages_commit_as_one_apply_of_the_last() {
    do_writer_thirty_stages_commit_as_one_apply_of_the_last();
}

/// Thirty stages of a drag, each the whole offset so far, then a
/// commit: one history entry, the document as one apply of the last
/// stage makes it — nothing accumulated across stages — and between
/// stages the document is the stage. One undo gives back the original.
pub fn do_writer_thirty_stages_commit_as_one_apply_of_the_last() {
    let (mut dragged, ids) = built();
    let original = saved(&dragged);
    let tx = dragged.begin(ChangeSource::Input, Interactive).unwrap();
    for i in 1..=30 {
        let at = Vec2::new(i as f32, 2.0 * i as f32);
        dragged.stage(&tx, place(vec![ids.a, ids.d], at)).unwrap();
        assert_eq!(position(&dragged, ids.a), Some(at), "the document is the stage");
        assert_eq!(dragged.revision(), Revision(0), "a stage is not a revision");
        assert_consistent(&dragged, "a stage");
    }
    assert_eq!(dragged.commit(tx), Some(Revision(1)));
    assert_eq!(dragged.history().count(), 1, "one entry");

    let (mut applied, _) = built();
    applied
        .apply(
            place(vec![ids.a, ids.d], Vec2::new(30.0, 60.0)),
            ChangeSource::Input,
        )
        .unwrap();
    assert_eq!(
        saved(&dragged),
        saved(&applied),
        "the same as one apply of the last stage"
    );
    assert_consistent(&dragged, "committed");
    dragged.undo().unwrap();
    assert_eq!(saved(&dragged), original, "one undo for the whole drag");
}

#[test]
fn test_writer_cancel_restores_the_document() {
    do_writer_cancel_restores_the_document();
}

/// A transaction with a base call that inserts a node and a stage that
/// moves it, cancelled: the document is as it was before `begin`, no
/// history entry is made, and every slot the transaction allocated is
/// freed. A stage replaced by another frees what the first inserted.
pub fn do_writer_cancel_restores_the_document() {
    let (mut doc, ids) = built();
    let original = saved(&doc);
    let slots = live_slots(&doc);
    let tx = doc.begin(ChangeSource::Input, Interactive).unwrap();
    let steps = create_child(&doc, ids.d, Vec2::ZERO).unwrap();
    doc.apply_in(&tx, EditRequest::new(steps, "create")).unwrap();
    let new = children_of(&doc, ids.d, DocKind::Node)[0];
    doc.stage(&tx, place(vec![new], Vec2::new(4.0, 4.0))).unwrap();
    let steps = create_root(&doc, Vec2::ZERO).unwrap();
    doc.stage(&tx, EditRequest::new(steps, "a staged node")).unwrap();
    let staged = *children_of(&doc, doc.root(), DocKind::Node).last().unwrap();
    let steps = create_root(&doc, Vec2::ONE).unwrap();
    doc.stage(&tx, EditRequest::new(steps, "another")).unwrap();
    assert!(
        doc.tree().arena().get(staged).is_none(),
        "the replaced stage's node is freed, not leaked"
    );
    assert_consistent(&doc, "staged");
    doc.cancel(tx);
    assert_eq!(saved(&doc), original, "as before begin");
    assert_eq!(doc.history().count(), 0);
    assert_eq!(doc.revision(), Revision(0));
    assert_eq!(
        live_slots(&doc),
        slots,
        "nothing the transaction made is left allocated"
    );
    assert!(doc.tree().arena().get(new).is_none(), "the base's node too");
    assert_consistent(&doc, "cancelled");
}

#[test]
fn test_writer_one_transaction_at_a_time() {
    do_writer_one_transaction_at_a_time();
}

/// While an interactive transaction is open, a second `begin` and an
/// `apply` are refused, and the document keeps its stage; once it is
/// committed, both are accepted. Another document's handle is refused.
pub fn do_writer_one_transaction_at_a_time() {
    let (mut doc, ids) = built();
    let tx = doc.begin(ChangeSource::Input, Interactive).unwrap();
    doc.stage(&tx, place(vec![ids.a], Vec2::new(3.0, 3.0))).unwrap();
    let staged = saved(&doc);
    assert_eq!(
        doc.begin(ChangeSource::Action, Interactive),
        Err(ApplyError::TransactionOpen)
    );
    assert_eq!(
        doc.begin(ChangeSource::Action, Animated),
        Err(ApplyError::TransactionOpen)
    );
    assert_eq!(
        doc.apply(fade(vec![ids.b], 0.5), ChangeSource::Action),
        Err(ApplyError::TransactionOpen)
    );
    assert_eq!(saved(&doc), staged, "the stage stays");
    assert_eq!(doc.history().count(), 0);

    assert!(doc.commit(tx).is_some());
    doc.apply(fade(vec![ids.b], 0.5), ChangeSource::Action).unwrap();
    let second = doc.begin(ChangeSource::Action, Interactive).unwrap();
    // Another document's handle names no transaction of this one.
    let (mut other, _) = built();
    let foreign = other.begin(ChangeSource::Action, Interactive).unwrap();
    assert_eq!(
        doc.stage(&foreign, fade(vec![ids.a], 0.25)),
        Err(ApplyError::TransactionClosed)
    );
    assert_eq!(
        other.stage(&foreign, fade(vec![ids.a], 0.25)),
        Ok(()),
        "but its own"
    );
    doc.cancel(second);
}

#[test]
fn test_writer_undo_and_redo_cancel_an_open_transaction() {
    do_writer_undo_and_redo_cancel_an_open_transaction();
}

/// One keypress, one visible step (ruling R36): an undo with an
/// interactive transaction open cancels it and stops there — the entry
/// before stays — returning the revision as it is; the handle is then
/// stale. A redo does the same. With an empty history, the undo that
/// cancels still answers `Some`, since the document changed; with
/// nothing open and nothing to undo, `None`; and one that cancels a
/// transaction that had changed nothing answers `None` and stops there
/// (ruling R37).
pub fn do_writer_undo_and_redo_cancel_an_open_transaction() {
    let (mut doc, ids) = built();
    let original = saved(&doc);
    let tx = doc.begin(ChangeSource::Input, Interactive).unwrap();
    doc.stage(&tx, place(vec![ids.a], Vec2::new(9.0, 9.0))).unwrap();
    assert_eq!(
        doc.undo(),
        Some(Revision(0)),
        "an empty history: the cancel is the change"
    );
    assert_eq!(saved(&doc), original, "the stage is gone");
    assert_eq!(doc.undo(), None, "nothing open, nothing to undo");

    // A transaction that changed nothing — no stage, or a stage that is
    // a no-op — cancels to nothing: the undo or redo answers `None`, and
    // stops there all the same (ruling R37).
    let (mut quiet, quiet_ids) = built();
    quiet
        .apply(fade(vec![quiet_ids.b], 0.5), ChangeSource::Action)
        .unwrap();
    let faded_b = saved(&quiet);
    let tx = quiet.begin(ChangeSource::Input, Interactive).unwrap();
    assert_eq!(quiet.undo(), None, "a transaction with no stage");
    assert_eq!(saved(&quiet), faded_b, "and the entry before is not undone");
    assert_eq!(quiet.commit(tx), None, "the transaction has ended");
    let tx = quiet.begin(ChangeSource::Input, Interactive).unwrap();
    quiet.stage(&tx, fade(vec![quiet_ids.b], 0.5)).unwrap();
    assert_eq!(quiet.redo(), None, "a no-op stage");
    assert_eq!(quiet.commit(tx), None);
    assert_eq!(quiet.history().count(), 1, "the entry before stays");

    doc.apply(fade(vec![ids.a], 0.5), ChangeSource::Action).unwrap();
    let faded = saved(&doc);
    let tx = doc.begin(ChangeSource::Input, Interactive).unwrap();
    doc.stage(&tx, place(vec![ids.a], Vec2::new(9.0, 9.0))).unwrap();
    assert_eq!(doc.undo(), Some(Revision(1)), "the revision as it is");
    assert_eq!(saved(&doc), faded, "the stage cancelled, the fade kept");
    assert_eq!(doc.history().count(), 1);
    assert_eq!(
        doc.stage(&tx, place(vec![ids.a], Vec2::new(1.0, 1.0))),
        Err(ApplyError::TransactionClosed),
        "the transaction has ended"
    );
    assert_eq!(doc.commit(tx), None);
    assert_consistent(&doc, "after the undo");

    doc.undo().unwrap();
    assert_eq!(saved(&doc), original, "the next undo undoes the fade");
    let tx = doc.begin(ChangeSource::Input, Interactive).unwrap();
    doc.stage(&tx, place(vec![ids.d], Vec2::new(9.0, 9.0))).unwrap();
    assert_eq!(doc.redo(), Some(Revision(2)), "the redo that cancels");
    assert_eq!(saved(&doc), original, "the stage cancelled, nothing redone");
    assert_eq!(doc.redo_len(), 1);
    doc.cancel(tx);
    assert_eq!(saved(&doc), original, "cancelling a stale handle does nothing");
    assert_eq!(doc.redo(), Some(Revision(3)));
    assert_eq!(saved(&doc), faded, "the next redo redoes the fade");
}

#[test]
fn test_writer_base_keeps_a_created_node_across_stages() {
    do_writer_base_keeps_a_created_node_across_stages();
}

/// The editor's shape (ruling R7): a node inserted into the base keeps
/// its `NodeId` and its ident across three stages of text into it,
/// through the commit, the undo and the redo. The commit is one entry;
/// the undo takes the node and its text out, the redo puts both back.
pub fn do_writer_base_keeps_a_created_node_across_stages() {
    let (mut doc, ids) = built();
    let original = saved(&doc);
    let tx = doc.begin(ChangeSource::Input, Interactive).unwrap();
    let steps = create_child(&doc, ids.d, Vec2::ZERO).unwrap();
    doc.apply_in(&tx, EditRequest::new(steps, "create")).unwrap();
    let new = children_of(&doc, ids.d, DocKind::Node)[0];
    let section = children_of(&doc, new, DocKind::Section)[0];
    let given = ident(&doc, new).unwrap();
    for text in ["h", "hi", "hi!"] {
        let steps = insert_text(&doc, section, 0, text.to_string()).unwrap();
        doc.stage(&tx, EditRequest::new(steps, "type")).unwrap();
        assert_eq!(
            children_of(&doc, ids.d, DocKind::Node),
            vec![new],
            "the same node"
        );
        assert_eq!(ident(&doc, new), Some(given), "with the same ident");
        assert_consistent(&doc, text);
    }
    assert_eq!(doc.commit(tx), Some(Revision(1)));
    assert_eq!(doc.history().count(), 1);
    let typed = saved(&doc);
    assert!(typed.contains("hi!"), "the last stage is committed");
    assert!(!typed.contains("hhi"), "and no earlier one");
    doc.undo().unwrap();
    assert_eq!(saved(&doc), original);
    assert_eq!(doc.node_by_ident(given), None);
    doc.redo().unwrap();
    assert_eq!(saved(&doc), typed);
    assert_eq!(
        doc.node_by_ident(given),
        Some(new),
        "the same node, the same ident"
    );
    assert_consistent(&doc, "redone");
}

#[test]
fn test_writer_saves_the_committed_state_with_a_stage_open() {
    do_writer_saves_the_committed_state_with_a_stage_open();
}

/// With a base call and a stage open, the saved text is the committed
/// document's, and the document keeps both: the stage is still drawn
/// after the save, the save records nothing, and the transaction
/// commits as it would have.
pub fn do_writer_saves_the_committed_state_with_a_stage_open() {
    let (mut doc, ids) = built();
    let committed = saved(&doc);
    let tx = doc.begin(ChangeSource::Input, Interactive).unwrap();
    let steps = create_child(&doc, ids.d, Vec2::ZERO).unwrap();
    doc.apply_in(&tx, EditRequest::new(steps, "create")).unwrap();
    doc.stage(&tx, place(vec![ids.a], Vec2::new(8.0, 8.0))).unwrap();
    let live = doc.tree().descendants().count();
    assert!(
        !doc.take_changes().is_empty(),
        "precondition: the stage and the base recorded"
    );
    assert_eq!(doc.save_json().unwrap(), committed, "the committed state");
    assert_eq!(
        position(&doc, ids.a),
        Some(Vec2::new(8.0, 8.0)),
        "the stage is back"
    );
    assert_eq!(doc.tree().descendants().count(), live, "and the base");
    assert!(doc.take_changes().is_empty(), "the save records nothing");
    assert_eq!(doc.commit(tx), Some(Revision(1)));
    assert_ne!(saved(&doc), committed);
    assert_consistent(&doc, "committed after the save");
}

#[test]
fn test_writer_an_animated_transaction_yields_by_finishing() {
    do_writer_an_animated_transaction_yields_by_finishing();
}

/// D28's other arm: an animated edit half way through, then an apply —
/// the animation is fast-forwarded to its end and committed as its own
/// entry, and the apply follows as the next. A `begin` does the same.
/// Interpolation writes the document part-way, and is refused on an
/// interactive transaction. An undo during an animation commits it and
/// undoes it alone, so it can be redone; a redo commits it (ruling R36).
pub fn do_writer_an_animated_transaction_yields_by_finishing() {
    let (mut doc, ids) = built();
    let tx = doc.begin(ChangeSource::Edit, Animated).unwrap();
    doc.stage(&tx, place(vec![ids.a], Vec2::new(100.0, 0.0))).unwrap();
    doc.interpolate(&tx, 0.5, Snap::End).unwrap();
    assert_eq!(position(&doc, ids.a), Some(Vec2::new(50.0, 0.0)), "half way");
    assert_consistent(&doc, "half way");
    doc.apply(fade(vec![ids.b], 0.5), ChangeSource::Action).unwrap();
    assert_eq!(
        position(&doc, ids.a),
        Some(Vec2::new(100.0, 0.0)),
        "finished first"
    );
    let labels: Vec<_> = doc
        .history()
        .map(|entry| (entry.source, entry.revision))
        .collect();
    assert_eq!(
        labels,
        vec![
            (ChangeSource::Edit, Revision(1)),
            (ChangeSource::Action, Revision(2))
        ],
        "the animation, then the apply"
    );
    assert_eq!(
        doc.interpolate(&tx, 0.5, Snap::End),
        Err(ApplyError::TransactionClosed)
    );
    assert_consistent(&doc, "after the apply");

    let tx = doc.begin(ChangeSource::Edit, Animated).unwrap();
    doc.stage(&tx, place(vec![ids.d], Vec2::new(0.0, 10.0))).unwrap();
    doc.interpolate(&tx, 0.25, Snap::End).unwrap();
    let next = doc.begin(ChangeSource::Input, Interactive).unwrap();
    assert_eq!(
        position(&doc, ids.d),
        Some(Vec2::new(0.0, 10.0)),
        "a begin finishes it too"
    );
    assert_eq!(doc.history().count(), 3);
    assert_eq!(
        doc.interpolate(&next, 0.5, Snap::End),
        Err(ApplyError::NotAnimated)
    );
    doc.cancel(next);

    // An undo during an animation finishes and commits it, then undoes
    // it alone: the entry before stays, and a redo brings the animation
    // back whole. A redo during one finishes it too, which leaves nothing
    // to redo.
    let before = position(&doc, ids.d);
    let tx = doc.begin(ChangeSource::Edit, Animated).unwrap();
    doc.stage(&tx, place(vec![ids.d], Vec2::new(0.0, 20.0))).unwrap();
    doc.interpolate(&tx, 0.5, Snap::End).unwrap();
    let revision = doc.revision().0;
    assert_eq!(
        doc.undo(),
        Some(Revision(revision + 2)),
        "a commit, then its undo"
    );
    assert_eq!(
        position(&doc, ids.d),
        before,
        "the animation undone, the entry before kept"
    );
    assert_eq!(doc.history().count(), 3);
    assert_eq!(doc.redo_len(), 1, "the animation can be redone");
    doc.redo().unwrap();
    assert_eq!(position(&doc, ids.d), Some(Vec2::new(0.0, 20.0)), "whole");
    let tx = doc.begin(ChangeSource::Edit, Animated).unwrap();
    doc.stage(&tx, place(vec![ids.d], Vec2::new(0.0, 30.0))).unwrap();
    doc.interpolate(&tx, 0.5, Snap::End).unwrap();
    let revision = doc.revision().0;
    assert_eq!(
        doc.redo(),
        Some(Revision(revision + 1)),
        "the commit is the change"
    );
    assert_eq!(position(&doc, ids.d), Some(Vec2::new(0.0, 30.0)), "finished");
    assert_eq!(doc.redo_len(), 0);
    assert_consistent(&doc, "after the undo");
}

#[test]
fn test_writer_records_every_stage_commit_and_cancel() {
    do_writer_records_every_stage_commit_and_cancel();
}

/// A stage's record names what it touched at the unchanged revision; a
/// restage names the previous stage's elements and the new one's; the
/// commit's names the entry's at the new revision; a cancel's names what
/// it took back, and the node it took out of the tree as removed.
pub fn do_writer_records_every_stage_commit_and_cancel() {
    let (mut doc, ids) = built();
    let tx = doc.begin(ChangeSource::Input, Interactive).unwrap();
    doc.stage(&tx, place(vec![ids.a], Vec2::new(1.0, 1.0))).unwrap();
    doc.stage(&tx, place(vec![ids.d], Vec2::new(1.0, 1.0))).unwrap();
    let records = doc.take_changes();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].touched, vec![(ids.a, Touch::Field)]);
    assert_eq!(
        records[1].touched,
        vec![(ids.a, Touch::Field), (ids.d, Touch::Field)],
        "the stage taken back, and the new one"
    );
    assert!(records
        .iter()
        .all(|r| r.revision == Revision(0) && r.source == ChangeSource::Input));
    doc.commit(tx).unwrap();
    let records = doc.take_changes();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].touched, vec![(ids.d, Touch::Field)]);
    assert_eq!(records[0].revision, Revision(1));

    let tx = doc.begin(ChangeSource::Input, Interactive).unwrap();
    let steps = create_root(&doc, Vec2::ZERO).unwrap();
    doc.apply_in(&tx, EditRequest::new(steps, "create")).unwrap();
    let new = *children_of(&doc, doc.root(), DocKind::Node).last().unwrap();
    doc.take_changes();
    doc.cancel(tx);
    let records = doc.take_changes();
    assert_eq!(records.len(), 1);
    assert!(records[0].touched.contains(&(new, Touch::Removed)));
    assert!(
        records[0].removed.iter().any(|(id, _)| *id == new),
        "with its address"
    );
}
