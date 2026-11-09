// SPDX-License-Identifier: MPL-2.0

//! The writer (V1-B4, #238): `Document::apply` all or nothing, undo and
//! redo exact for every composite builder, one entry per request, the
//! dirty flag by identity, the bounded history and what leaving it
//! frees, the links a removal takes with it, the idents and their bound,
//! the change feed, and the wasm32 budget. Transactions are in
//! `writer_transaction_tests`.
//!
//! Every state a test reaches is held to `writer_support::assert_consistent`:
//! the writer's indexes computed again from scratch, and the document
//! still savable. States are compared by their saved text.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2): every `do_*` body
//! has a bench entry in `benches/test_bench.rs`.

use crate::arena::NodeId;
use crate::document::bounds::MAX_NODE_IDENT;
use crate::document::element::{DocElement, DocKind, LinkData};
use crate::document::format::{load, Limits};
use crate::document::ids::{Document, NodeIdent};
use crate::document::mutation::compose::{
    add_section, create_child, create_root, delete, delete_text, insert_text, remove_links, reparent,
    set_link_kind, split_section, style_range,
};
use crate::document::mutation::error::MutationError;
use crate::document::mutation::ops::{DocMutation, DocMutatorTree, NodeOp, TextOp};
use crate::document::mutation::step::{DocStep, DocWalk};
use crate::document::tests::test_support::{node_data, path_data, DocumentBuilder};
use crate::document::tests::writer_support::{
    assert_consistent, built, children_of, fade, ident, place, position, request, saved, single,
};
use crate::document::values::LinkKind;
use crate::document::writer::error::ApplyError;
use crate::document::writer::feed::RemovedAddress;
use crate::document::writer::{ChangeSource, Derived, EditRequest, Revision};
use crate::engine::anchor::Anchors;
use crate::engine::journal::Touch;
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tree::{Placement, TreeError};
use glam::Vec2;

/// The span of the example's line link.
fn line_span(doc: &Document, line: NodeId) -> NodeId {
    children_of(doc, line, DocKind::Span)[0]
}

#[test]
fn test_writer_refuses_a_request_whole() {
    do_writer_refuses_a_request_whole();
}

/// A request whose second step fails validation, one whose walk fails on
/// its second anchor, and one whose result breaks a shape rule that only
/// the walk decides (`validate_applied`) are each refused, and leave the
/// document byte-identical — no history entry, no revision, nothing in
/// the feed, the ident counter where it was — and as savable as it was.
/// The same first step on its own applies, so it is the later failure
/// that undoes it.
pub fn do_writer_refuses_a_request_whole() {
    let (mut doc, ids) = built();
    let original = saved(&doc);
    let counter = doc.next_ident();

    // Two steps: the first moves `a`; the second sets an opacity out of
    // its bound, and validation refuses it once the first has applied.
    let moved = DocStep::fixed(DocWalk::new(
        single(
            DocKind::Node,
            DocMutation::Node(NodeOp::SetPosition(Vec2::new(7.0, 7.0))),
        ),
        Anchors::Nodes(vec![ids.a]),
    ));
    let faded = DocStep::fixed(DocWalk::new(
        single(DocKind::Node, DocMutation::Node(NodeOp::SetOpacity(2.0))),
        Anchors::Nodes(vec![ids.a]),
    ));
    let refused = doc.apply(
        EditRequest::new(vec![moved, faded], "two steps"),
        ChangeSource::Action,
    );
    assert!(
        matches!(refused, Err(ApplyError::Invalid(MutationError::Bound { .. }))),
        "the second step's bound: {refused:?}"
    );
    assert_eq!(saved(&doc), original, "the first step is undone with the second");

    // One walk at two spans: the split of the first applies, the split
    // of the second (one cluster long) cannot.
    let one_cluster = line_span(&doc, ids.line);
    let splitting = DocWalk::new(
        DocMutatorTree::new_with(Mutator::new_structure(
            StructOp::Split { at: 2 },
            BranchKey::new(DocKind::Span, 0),
        )),
        Anchors::Nodes(vec![ids.a_title_span, one_cluster]),
    );
    let refused = doc.apply(
        EditRequest::new(vec![DocStep::fixed(splitting)], "split"),
        ChangeSource::Action,
    );
    assert_eq!(
        refused,
        Err(ApplyError::Invalid(MutationError::Tree(TreeError::Unsplittable(
            one_cluster
        )))),
        "the second anchor's split"
    );
    assert_eq!(saved(&doc), original, "the first anchor's split is undone");

    // A second frame on `a`: every payload is valid, the insert is not —
    // a node holds one frame — which only the result shows.
    let frame = request(
        DocMutatorTree::new_with(Mutator::new_structure(
            StructOp::Insert {
                subtree: Subtree::leaf(DocElement::Frame(Default::default())),
                at: Placement::Last,
            },
            BranchKey::new(DocKind::Node, 0),
        )),
        vec![ids.a],
        "frame",
    );
    let refused = doc.apply(frame, ChangeSource::Action);
    assert!(
        matches!(refused, Err(ApplyError::Invalid(MutationError::Shape(_)))),
        "the shape rule after the walk: {refused:?}"
    );
    assert_eq!(saved(&doc), original, "the inserted frame is gone");

    assert_eq!(doc.revision(), Revision(0));
    assert_eq!(doc.history().count(), 0, "no entry");
    assert!(doc.take_changes().is_empty(), "nothing in the feed");
    assert_eq!(doc.next_ident(), counter, "no ident spent");
    assert!(!doc.is_dirty());
    assert_consistent(&doc, "after three refusals");

    // The control: the first step alone applies.
    doc.apply(place(vec![ids.a], Vec2::new(7.0, 7.0)), ChangeSource::Action)
        .unwrap();
    assert_ne!(saved(&doc), original, "the first step on its own changes the map");
}

/// A composite builder's request on a fresh example, named.
type Builder = fn(&Document, &crate::document::tests::test_support::Example) -> Vec<DocStep>;

/// Every composite builder of V1-B3, each with arguments that change
/// the example: both kinds of create, a delete that takes a link with
/// it and one that takes a portal, both kinds of reparent, both kind
/// switches, the four text builders, both forms of a new section, and
/// the link removal.
fn builders() -> Vec<(&'static str, Builder)> {
    vec![
        ("create_child", |doc, ids| {
            create_child(doc, ids.b, Vec2::new(5.0, 6.0)).unwrap()
        }),
        ("create_root", |doc, _| {
            create_root(doc, Vec2::new(-5.0, 6.0)).unwrap()
        }),
        ("delete b", |doc, ids| delete(doc, &[ids.b]).unwrap()),
        ("delete d", |doc, ids| delete(doc, &[ids.d]).unwrap()),
        ("reparent under d", |doc, ids| {
            reparent(doc, &[ids.c], ids.d).unwrap()
        }),
        ("reparent to the map", |doc, ids| {
            reparent(doc, &[ids.b], doc.root()).unwrap()
        }),
        ("line to portal", |doc, ids| {
            set_link_kind(doc, ids.line, LinkKind::Portal).unwrap()
        }),
        ("portal to line", |doc, ids| {
            set_link_kind(doc, ids.portal, LinkKind::Line).unwrap()
        }),
        ("style_range", |doc, ids| {
            style_range(doc, ids.a_title, 1, 3, TextOp::SetItalic(Some(true))).unwrap()
        }),
        ("insert_text", |doc, ids| {
            insert_text(doc, ids.a_title, 2, "xy".to_string()).unwrap()
        }),
        ("delete_text", |doc, ids| {
            delete_text(doc, ids.a_title, 1, 3).unwrap()
        }),
        ("split_section", |doc, ids| {
            split_section(doc, ids.a_title, 2).unwrap()
        }),
        ("add_section after", |doc, ids| {
            add_section(doc, ids.b, Some(ids.b_section)).unwrap()
        }),
        ("add_section first", |doc, ids| {
            add_section(doc, ids.b, None).unwrap()
        }),
        ("remove_links", |_, ids| vec![remove_links(vec![ids.line])]),
    ]
}

#[test]
fn test_writer_undo_and_redo_are_exact_for_every_composite_builder() {
    do_writer_undo_and_redo_are_exact_for_every_composite_builder();
}

/// For every composite builder: apply → undo → redo → undo gives back
/// the saved text of the original, and apply → undo → redo the saved
/// text of the applied document — the inserted nodes with the idents
/// the apply gave them. Each state is consistent, and each builder
/// changes the map, or there would be nothing for an undo to restore.
pub fn do_writer_undo_and_redo_are_exact_for_every_composite_builder() {
    for (name, build) in builders() {
        let (mut doc, ids) = built();
        let original = saved(&doc);
        let steps = build(&doc, &ids);
        let revision = doc
            .apply(EditRequest::new(steps, name), ChangeSource::Action)
            .unwrap();
        assert_eq!(revision, Revision(1), "{name}: one entry");
        let applied = saved(&doc);
        assert_ne!(
            applied, original,
            "{name}: precondition: the builder changes the map"
        );
        assert_consistent(&doc, name);

        assert_eq!(doc.undo(), Some(Revision(2)), "{name}");
        assert_eq!(saved(&doc), original, "{name}: undo gives back the original");
        assert_consistent(&doc, name);
        assert_eq!(doc.redo(), Some(Revision(3)), "{name}");
        assert_eq!(saved(&doc), applied, "{name}: redo gives back the edit");
        assert_consistent(&doc, name);
        assert_eq!(doc.undo(), Some(Revision(4)), "{name}");
        assert_eq!(saved(&doc), original, "{name}: and undo the original again");
        assert_consistent(&doc, name);
        assert_eq!(doc.undo(), None, "{name}: nothing older to undo");
    }
}

#[test]
fn test_writer_makes_one_entry_of_a_request_with_many_anchors() {
    do_writer_makes_one_entry_of_a_request_with_many_anchors();
}

/// A request at three nodes is one history entry, one revision, and one
/// record; the same request again changes nothing and makes neither;
/// one undo takes all three back.
pub fn do_writer_makes_one_entry_of_a_request_with_many_anchors() {
    let (mut doc, ids) = built();
    let original = saved(&doc);
    let revision = doc
        .apply(fade(vec![ids.a, ids.b, ids.d], 0.5), ChangeSource::Action)
        .unwrap();
    assert_eq!(revision, Revision(1));
    let entries: Vec<_> = doc
        .history()
        .map(|entry| (entry.label.to_string(), entry.source, entry.revision))
        .collect();
    assert_eq!(
        entries,
        vec![("fade".to_string(), ChangeSource::Action, Revision(1))]
    );
    assert_eq!(doc.take_changes().len(), 1, "one record");
    assert_eq!(
        doc.apply(fade(vec![ids.a, ids.b, ids.d], 0.5), ChangeSource::Action),
        Ok(Revision(1)),
        "the same change again changes nothing"
    );
    assert_eq!(doc.history().count(), 1, "and makes no entry");
    assert!(doc.take_changes().is_empty(), "and no record");
    doc.undo().unwrap();
    assert_eq!(saved(&doc), original, "one undo restores every anchor");
    assert_eq!(doc.redo_len(), 1);
}

#[test]
fn test_document_reads_nothing_a_change_removed() {
    do_document_reads_nothing_a_change_removed();
}

/// **What a change removed, the document does not read** (review
/// 5821011650): after `b`'s subtree is deleted, `get`, `children`,
/// `parent`, `depth` and `owning_node` answer for `b`, its section and
/// its child `c` as for an id the tree never held. Control: the arena
/// still holds them — the journal keeps them for its undo — so the
/// answers are the tree's, not the arena's; and after the undo every
/// read is back.
pub fn do_document_reads_nothing_a_change_removed() {
    let (mut doc, ids) = built();
    assert_eq!(doc.parent(ids.c), Some(ids.b), "precondition: c is under b");
    let steps = delete(&doc, &[ids.b]).unwrap();
    doc.apply(EditRequest::new(steps, "delete"), ChangeSource::Action)
        .unwrap();
    for gone in [ids.b, ids.b_section, ids.c] {
        assert!(
            doc.tree().get(gone).is_some(),
            "control: the arena still holds {gone}"
        );
        assert_eq!(doc.get(gone), None, "{gone} is not read");
        assert_eq!(doc.children(gone).count(), 0);
        assert_eq!(doc.parent(gone), None);
        assert_eq!(doc.depth(gone), 0);
        assert_eq!(doc.owning_node(gone), None);
    }
    doc.undo().unwrap();
    assert!(doc.get(ids.b).is_some_and(|e| e.kind() == DocKind::Node));
    assert_eq!(doc.parent(ids.c), Some(ids.b));
    assert_eq!(doc.owning_node(ids.b_section), Some(ids.b));
    assert_eq!(doc.depth(ids.c), 2);
    assert!(doc.children(ids.b).count() > 0);
}

#[test]
fn test_writer_dirty_is_the_saved_state_not_a_count() {
    do_writer_dirty_is_the_saved_state_not_a_count();
}

/// `mark_saved`, an edit, its undo: clean. An edit, its undo, a
/// different edit: dirty, though the history is as long as it was when
/// saved. A redo away from the saved state is dirty; the undo back is
/// clean. A saved state the history has dropped is out of reach: undoing
/// as far as the history goes is still dirty. A saved entry that is
/// dropped becomes the history's base, and undoing back to it is clean.
pub fn do_writer_dirty_is_the_saved_state_not_a_count() {
    let (mut doc, ids) = built();
    assert!(!doc.is_dirty(), "a loaded document is clean");
    doc.apply(fade(vec![ids.a], 0.5), ChangeSource::Action).unwrap();
    assert!(doc.is_dirty());
    doc.mark_saved();
    assert!(!doc.is_dirty(), "saved");

    doc.apply(fade(vec![ids.b], 0.5), ChangeSource::Action).unwrap();
    assert!(doc.is_dirty(), "an edit after the save");
    doc.undo().unwrap();
    assert!(!doc.is_dirty(), "undone back to the saved state");
    doc.redo().unwrap();
    assert!(doc.is_dirty(), "redone away from it");
    doc.undo().unwrap();
    assert!(!doc.is_dirty(), "and back");

    doc.undo().unwrap();
    assert!(doc.is_dirty(), "undone past the saved state");
    doc.apply(fade(vec![ids.d], 0.25), ChangeSource::Action).unwrap();
    assert_eq!(doc.history().count(), 1, "as many entries as when saved");
    assert!(
        doc.is_dirty(),
        "a different edit with as many entries is not the saved state"
    );

    // A trimmed history: the loaded state was saved and has left the
    // history, so undoing as far as the history reaches is still dirty.
    let (mut doc, ids) = built();
    doc.set_history_limit(2);
    for (i, node) in [ids.a, ids.b, ids.c, ids.d].into_iter().enumerate() {
        doc.apply(fade(vec![node], 0.1 * (i + 1) as f32), ChangeSource::Action)
            .unwrap();
    }
    assert!(doc.undo().is_some() && doc.undo().is_some());
    assert_eq!(doc.undo(), None, "the history reaches two entries back");
    assert!(doc.is_dirty(), "the base is the trimmed state, not the saved one");

    // The saved state is an entry that is trimmed: it becomes the base,
    // and undoing back to it is clean.
    let (mut doc, ids) = built();
    doc.set_history_limit(2);
    doc.apply(fade(vec![ids.a], 0.5), ChangeSource::Action).unwrap();
    doc.mark_saved();
    doc.apply(fade(vec![ids.b], 0.5), ChangeSource::Action).unwrap();
    doc.apply(fade(vec![ids.c], 0.5), ChangeSource::Action).unwrap();
    assert_eq!(
        doc.history().count(),
        2,
        "precondition: the saved entry left the history"
    );
    assert!(doc.is_dirty());
    assert!(doc.undo().is_some() && doc.undo().is_some());
    assert!(!doc.is_dirty(), "back at the saved state, now the base");
}

#[test]
fn test_writer_history_keeps_its_limit_and_frees_what_leaves_it() {
    do_writer_history_keeps_its_limit_and_frees_what_leaves_it();
}

/// With a limit of 3 and five deletes, the oldest two entries are
/// dropped and the nodes they removed are freed — their ids stale —
/// while the three kept still hold theirs, detached, for an undo. Two
/// undos and a new edit clear the redo branch, and the nodes the
/// undone entries had created are freed with it. A limit lowered with
/// nothing applied keeps the undone entry nearest a redo.
pub fn do_writer_history_keeps_its_limit_and_frees_what_leaves_it() {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let nodes: Vec<NodeId> = (0..5).map(|_| builder.node(root)).collect();
    let mut doc = builder.build().unwrap();
    doc.set_history_limit(3);
    for &node in &nodes {
        let steps = delete(&doc, &[node]).unwrap();
        doc.apply(EditRequest::new(steps, "delete"), ChangeSource::Action)
            .unwrap();
        assert_consistent(&doc, "a delete");
    }
    assert_eq!(doc.history().count(), 3, "the limit");
    for (i, &node) in nodes.iter().enumerate() {
        let held = doc.tree().arena().get(node).is_some();
        assert_eq!(
            held,
            i >= 2,
            "node {i}: freed exactly when its entry left the history"
        );
    }
    assert!(doc.undo().is_some() && doc.undo().is_some() && doc.undo().is_some());
    assert!(doc.undo().is_none(), "the dropped entries cannot be undone");
    assert_eq!(
        doc.children(doc.root()).filter(|&c| nodes.contains(&c)).count(),
        3
    );

    // Two creates, both undone, then a new edit.
    while doc.redo().is_some() {}
    let mut created = Vec::new();
    for _ in 0..2 {
        let steps = create_root(&doc, Vec2::ZERO).unwrap();
        doc.apply(EditRequest::new(steps, "create"), ChangeSource::Action)
            .unwrap();
        created.push(*children_of(&doc, doc.root(), DocKind::Node).last().unwrap());
    }
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.redo_len(), 2);
    assert!(
        created.iter().all(|&id| doc.tree().arena().get(id).is_some()),
        "held for a redo"
    );
    doc.apply(
        EditRequest::new(create_root(&doc, Vec2::ONE).unwrap(), "create"),
        ChangeSource::Action,
    )
    .unwrap();
    assert_eq!(doc.redo_len(), 0, "a new edit clears the redo branch");
    for id in created {
        assert!(
            doc.tree().arena().get(id).is_none(),
            "and frees what it would have redone"
        );
    }
    assert_consistent(&doc, "after the new edit");

    // Everything undone, then a lower limit: with nothing applied left,
    // the undone entries furthest from a redo go, and the nearest stays
    // redoable.
    while doc.undo().is_some() {}
    let undone = doc.redo_len();
    assert!(undone >= 2, "precondition: a redo branch to trim: {undone}");
    doc.redo().unwrap();
    let nearest = saved(&doc);
    doc.undo().unwrap();
    doc.set_history_limit(1);
    assert_eq!(doc.redo_len(), 1, "the redo branch kept to the limit");
    doc.redo().unwrap();
    assert_eq!(saved(&doc), nearest, "the entry kept is the next to redo");
    assert_eq!(doc.redo(), None);
    assert_consistent(&doc, "after the lowered limit");
}

#[test]
fn test_writer_removes_the_links_into_what_a_request_removes() {
    do_writer_removes_the_links_into_what_a_request_removes();
}

/// Deleting `b` removes `c` with it, and the line link from `a` that
/// leads to `c` goes in the same entry; the undo puts the link back with
/// its own id, and the redo takes it again. Deleting `d` takes the
/// portal to it. A request that inserts a link to `c` and then deletes
/// `c` leaves no link behind: the link it inserted is found too, though
/// the index has not seen it yet. The index is held to a count from
/// scratch at every step.
pub fn do_writer_removes_the_links_into_what_a_request_removes() {
    let (mut doc, ids) = built();
    assert_eq!(
        doc.incoming_links(ids.c),
        &[ids.line],
        "precondition: the line leads to c"
    );
    let steps = delete(&doc, &[ids.b]).unwrap();
    doc.apply(EditRequest::new(steps, "delete b"), ChangeSource::Action)
        .unwrap();
    assert!(
        !doc.tree().contains(ids.line),
        "the link into the removed subtree is gone"
    );
    assert!(doc.incoming_links(ids.c).is_empty());
    assert_eq!(doc.history().count(), 1, "in the same entry");
    assert_consistent(&doc, "after the delete");
    doc.undo().unwrap();
    assert!(doc.tree().contains(ids.line), "the undo puts it back");
    assert_eq!(doc.parent(ids.line), Some(ids.a), "where it was");
    assert_eq!(doc.incoming_links(ids.c), &[ids.line]);
    assert_consistent(&doc, "after the undo");
    doc.redo().unwrap();
    assert!(!doc.tree().contains(ids.line), "the redo takes it again");
    assert_consistent(&doc, "after the redo");
    doc.undo().unwrap();

    let steps = delete(&doc, &[ids.d]).unwrap();
    doc.apply(EditRequest::new(steps, "delete d"), ChangeSource::Action)
        .unwrap();
    assert!(
        !doc.tree().contains(ids.portal),
        "the portal to d is gone with it"
    );
    assert_consistent(&doc, "after deleting d");
    doc.undo().unwrap();

    // One request: a new line link from d to c, then c deleted.
    let link = DocElement::Link(LinkData {
        to: ids.c,
        kind: LinkKind::Line,
        path: path_data(),
    });
    let insert = DocStep::fixed(DocWalk::new(
        DocMutatorTree::new_with(Mutator::new_structure(
            StructOp::Insert {
                subtree: Subtree::leaf(link),
                at: Placement::Last,
            },
            BranchKey::new(DocKind::Node, 0),
        )),
        Anchors::Nodes(vec![ids.d]),
    ));
    let mut steps = vec![insert];
    steps.extend(delete(&doc, &[ids.c]).unwrap());
    doc.apply(EditRequest::new(steps, "link then delete"), ChangeSource::Action)
        .unwrap();
    assert!(
        children_of(&doc, ids.d, DocKind::Link).is_empty(),
        "the link the request inserted went with its target"
    );
    assert!(!doc.tree().contains(ids.line), "and so did the old one");
    assert_consistent(&doc, "after the link and the delete");
}

#[test]
fn test_writer_gives_inserted_nodes_fresh_idents() {
    do_writer_gives_inserted_nodes_fresh_idents();
}

/// An inserted node gets the next ident; the undo takes it out of the
/// index and the redo puts the same node back under the same ident; a
/// node inserted after the undo gets a new one — an ident is never
/// issued twice. Two nodes inserted by one request get two, in the
/// order the request created them.
pub fn do_writer_gives_inserted_nodes_fresh_idents() {
    let (mut doc, ids) = built();
    let first = doc.next_ident().unwrap();
    let steps = create_child(&doc, ids.d, Vec2::ZERO).unwrap();
    doc.apply(EditRequest::new(steps, "create"), ChangeSource::Action)
        .unwrap();
    let new = children_of(&doc, ids.d, DocKind::Node)[0];
    assert_eq!(ident(&doc, new), Some(first), "the next ident");
    assert_eq!(doc.node_by_ident(first), Some(new));
    doc.undo().unwrap();
    assert_eq!(doc.node_by_ident(first), None, "out of the index with the node");
    doc.redo().unwrap();
    assert_eq!(
        doc.node_by_ident(first),
        Some(new),
        "the same node, the same ident"
    );
    assert_eq!(ident(&doc, new), Some(first));
    doc.undo().unwrap();
    let steps = create_child(&doc, ids.d, Vec2::ZERO).unwrap();
    doc.apply(EditRequest::new(steps, "create"), ChangeSource::Action)
        .unwrap();
    let again = children_of(&doc, ids.d, DocKind::Node)[0];
    assert_eq!(
        ident(&doc, again),
        Some(NodeIdent(first.0 + 1)),
        "never the undone node's"
    );
    assert_consistent(&doc, "after the second create");

    let mut steps = create_child(&doc, ids.b, Vec2::ZERO).unwrap();
    steps.extend(create_root(&doc, Vec2::ONE).unwrap());
    doc.apply(EditRequest::new(steps, "two"), ChangeSource::Action)
        .unwrap();
    let under_b = *children_of(&doc, ids.b, DocKind::Node).last().unwrap();
    let root = *children_of(&doc, doc.root(), DocKind::Node).last().unwrap();
    assert_eq!(
        ident(&doc, under_b),
        Some(NodeIdent(first.0 + 2)),
        "the first created first"
    );
    assert_eq!(ident(&doc, root), Some(NodeIdent(first.0 + 3)));
    assert_consistent(&doc, "after two in one request");
}

#[test]
fn test_writer_stops_at_the_last_ident() {
    do_writer_stops_at_the_last_ident();
}

/// A document whose highest ident is one below
/// [`MAX_NODE_IDENT`]: an insert takes the last ident, and the next is
/// refused with the bound named — nothing applied, the counter where it
/// was, never wrapped or reissued. A request inserting two nodes when
/// one ident is left is refused whole.
pub fn do_writer_stops_at_the_last_ident() {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    builder.node_with(root, node_data(MAX_NODE_IDENT - 1));
    let mut doc = builder.build().unwrap();
    assert_eq!(doc.next_ident(), Some(NodeIdent(MAX_NODE_IDENT)), "precondition");
    let pristine = saved(&doc);

    let mut two = create_root(&doc, Vec2::ZERO).unwrap();
    two.extend(create_root(&doc, Vec2::ONE).unwrap());
    let refused = doc.apply(EditRequest::new(two, "two"), ChangeSource::Action);
    assert_eq!(
        refused,
        Err(ApplyError::IdentSpaceExhausted { max: MAX_NODE_IDENT })
    );
    assert_eq!(saved(&doc), pristine, "neither node is applied");
    assert_eq!(
        doc.next_ident(),
        Some(NodeIdent(MAX_NODE_IDENT)),
        "the counter is put back"
    );

    let steps = create_root(&doc, Vec2::ZERO).unwrap();
    doc.apply(EditRequest::new(steps, "last"), ChangeSource::Action)
        .unwrap();
    let last = *children_of(&doc, doc.root(), DocKind::Node).last().unwrap();
    assert_eq!(
        ident(&doc, last),
        Some(NodeIdent(MAX_NODE_IDENT)),
        "the bound itself is issued"
    );
    assert_eq!(doc.next_ident(), None, "and nothing after it");
    let after_last = saved(&doc);

    let steps = create_root(&doc, Vec2::ZERO).unwrap();
    let refused = doc.apply(EditRequest::new(steps, "past"), ChangeSource::Action);
    assert_eq!(
        refused,
        Err(ApplyError::IdentSpaceExhausted { max: MAX_NODE_IDENT })
    );
    assert_eq!(saved(&doc), after_last, "nothing applied");
    assert!(
        refused
            .unwrap_err()
            .to_string()
            .contains(&MAX_NODE_IDENT.to_string()),
        "the bound is named"
    );
    assert_consistent(&doc, "at the bound");
}

#[test]
fn test_writer_feed_names_what_each_change_touched() {
    do_writer_feed_names_what_each_change_touched();
}

/// An edit's record names exactly the elements it touched, with its
/// source and the revision it made; a second drain is empty. A delete's
/// record names what it removed with the address each had — a node by
/// its ident, the link it took as the second of `a`'s links — and the
/// undo's record, as [`ChangeSource::Undo`], names them back in. The undo
/// of a create reports the created node removed, with its own ident. Two
/// links removed together are ranked where they stood before either
/// left.
pub fn do_writer_feed_names_what_each_change_touched() {
    let (mut doc, ids) = built();
    doc.apply(fade(vec![ids.a, ids.d], 0.5), ChangeSource::Console)
        .unwrap();
    let records = doc.take_changes();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].source, ChangeSource::Console);
    assert_eq!(records[0].revision, Revision(1));
    let touched: rustc_hash::FxHashSet<_> = records[0].touched.iter().copied().collect();
    let expected: rustc_hash::FxHashSet<_> = [(ids.a, Touch::Field), (ids.d, Touch::Field)]
        .into_iter()
        .collect();
    assert_eq!(records[0].touched.len(), 2, "each once");
    assert_eq!(touched, expected, "exactly the two nodes");
    assert!(records[0].removed.is_empty());
    assert!(doc.take_changes().is_empty(), "drained");

    let d_ident = ident(&doc, ids.d);
    let a_ident = ident(&doc, ids.a);
    let steps = delete(&doc, &[ids.d]).unwrap();
    doc.apply(EditRequest::new(steps, "delete d"), ChangeSource::Ipc)
        .unwrap();
    let records = doc.take_changes();
    assert_eq!(records.len(), 1);
    let removed = &records[0].removed;
    let address = |id: NodeId| {
        removed
            .iter()
            .find(|(r, _)| *r == id)
            .map(|(_, address)| *address)
    };
    assert_eq!(
        address(ids.d),
        Some(RemovedAddress {
            node: d_ident,
            kind: DocKind::Node,
            channel: 0,
            index: None,
        }),
        "d, named by its id"
    );
    assert_eq!(
        address(ids.portal),
        Some(RemovedAddress {
            node: a_ident,
            kind: DocKind::Link,
            channel: 0,
            index: Some(1),
        }),
        "the portal, a's second link"
    );
    assert_eq!(
        address(ids.portal_target),
        Some(RemovedAddress {
            node: a_ident,
            kind: DocKind::PortalEnd,
            channel: 1,
            index: Some(0),
        }),
        "inside it, the target end"
    );
    assert!(
        records[0].touched.contains(&(ids.portal, Touch::Removed)),
        "the removal is a touch too"
    );

    doc.undo().unwrap();
    let records = doc.take_changes();
    assert_eq!(records[0].source, ChangeSource::Undo);
    assert_eq!(records[0].revision, Revision(3));
    assert!(
        records[0].touched.contains(&(ids.portal, Touch::Inserted)),
        "back in"
    );
    assert!(
        records[0].removed.is_empty(),
        "the undo of a delete removes nothing"
    );

    let steps = create_root(&doc, Vec2::ZERO).unwrap();
    doc.apply(EditRequest::new(steps, "create"), ChangeSource::Action)
        .unwrap();
    let new = *children_of(&doc, doc.root(), DocKind::Node).last().unwrap();
    let new_ident = ident(&doc, new);
    doc.take_changes();
    doc.undo().unwrap();
    let records = doc.take_changes();
    assert!(records[0].touched.contains(&(new, Touch::Removed)));
    let gone = records[0]
        .removed
        .iter()
        .find(|(id, _)| *id == new)
        .map(|(_, a)| *a);
    assert_eq!(
        gone,
        Some(RemovedAddress {
            node: new_ident,
            kind: DocKind::Node,
            channel: 0,
            index: None,
        }),
        "the undone node, named by its id"
    );

    // Both of a's links in one request: each is ranked where it stood
    // before the request — the portal behind the line, which left first.
    doc.apply(
        EditRequest::new(vec![remove_links(vec![ids.line, ids.portal])], "unlink"),
        ChangeSource::Action,
    )
    .unwrap();
    let records = doc.take_changes();
    let rank = |id: NodeId| {
        records[0]
            .removed
            .iter()
            .find(|(r, _)| *r == id)
            .and_then(|(_, address)| address.index)
    };
    assert_eq!(
        (rank(ids.line), rank(ids.portal)),
        (Some(0), Some(1)),
        "the ranks the links had before either left"
    );
}

#[test]
fn test_writer_holds_edits_to_the_platform_budget() {
    do_writer_holds_edits_to_the_platform_budget();
}

/// Under limits just above the document's — as wasm32 holds a map to
/// `MAX_ELEMENTS` and `MAX_MAP_BYTES` — an edit that would add more
/// elements, or more text, than the limits allow is refused with the
/// reason and changes nothing; one that fits applies, and so does each
/// with no limits.
pub fn do_writer_holds_edits_to_the_platform_budget() {
    let (mut doc, ids) = built();
    let start = doc.derived().budget;
    let pristine = saved(&doc);
    // `create_child` adds three elements: the node, its section, its
    // connector.
    doc.set_limits(Limits {
        bytes: None,
        elements: Some(start.elements + 2),
    });
    let steps = create_child(&doc, ids.d, Vec2::ZERO).unwrap();
    let refused = doc.apply(EditRequest::new(steps, "create"), ChangeSource::Action);
    assert_eq!(
        refused,
        Err(ApplyError::TooManyElements {
            elements: start.elements + 3,
            max: start.elements + 2,
        })
    );
    assert_eq!(saved(&doc), pristine);
    assert_eq!(doc.derived().budget, start, "the budget is untouched");
    doc.set_limits(Limits {
        bytes: None,
        elements: Some(start.elements + 3),
    });
    let steps = create_child(&doc, ids.d, Vec2::ZERO).unwrap();
    doc.apply(EditRequest::new(steps, "create"), ChangeSource::Action)
        .unwrap();
    assert_eq!(doc.derived().budget.elements, start.elements + 3);
    doc.undo().unwrap();

    doc.set_limits(Limits {
        bytes: Some(start.text + 2),
        elements: None,
    });
    let steps = insert_text(&doc, ids.a_title, 0, "abc".to_string()).unwrap();
    let refused = doc.apply(EditRequest::new(steps, "type"), ChangeSource::Input);
    assert_eq!(
        refused,
        Err(ApplyError::TooMuchText {
            bytes: start.text + 3,
            max: start.text + 2,
        })
    );
    assert_eq!(saved(&doc), pristine);
    let steps = insert_text(&doc, ids.a_title, 0, "ab".to_string()).unwrap();
    doc.apply(EditRequest::new(steps, "type"), ChangeSource::Input)
        .unwrap();
    assert_eq!(doc.derived().budget.text, start.text + 2);

    doc.set_limits(Limits {
        bytes: None,
        elements: None,
    });
    let steps = insert_text(&doc, ids.a_title, 0, "a longer text".to_string()).unwrap();
    doc.apply(EditRequest::new(steps, "type"), ChangeSource::Input)
        .unwrap();
    assert_consistent(&doc, "with no limits");
}

#[test]
fn test_writer_starts_from_what_the_loader_derived() {
    do_writer_starts_from_what_the_loader_derived();
}

/// A loaded document's incoming links and budget, which the loader
/// gathers as it builds, are what a walk of its tree finds — on every
/// fixture of the format, and on a map with links into several nodes.
pub fn do_writer_starts_from_what_the_loader_derived() {
    let mut linked = 0;
    for (name, text) in crate::document::tests::format_support::FIXTURES {
        let doc = load(text).unwrap_or_else(|error| panic!("{name}: {error}"));
        let scan = Derived::scan(doc.tree(), doc.definitions());
        assert_eq!(doc.derived().budget, scan.budget, "{name}: the budget");
        assert_consistent(&doc, name);
        linked += scan.incoming.len();
    }
    assert!(
        linked >= 2,
        "the fixtures must hold links for the index to be read: {linked}"
    );
}

#[test]
fn test_writer_refuses_the_sources_a_change_cannot_declare() {
    do_writer_refuses_the_sources_a_change_cannot_declare();
}

/// An apply or a begin declaring the history's own sources or the
/// presentation layer's is refused, and changes nothing; every other
/// source is accepted.
pub fn do_writer_refuses_the_sources_a_change_cannot_declare() {
    let (mut doc, ids) = built();
    for source in [ChangeSource::Undo, ChangeSource::Redo, ChangeSource::Presentation] {
        assert_eq!(
            doc.apply(fade(vec![ids.a], 0.5), source),
            Err(ApplyError::Source(source))
        );
        assert_eq!(
            doc.begin(
                source,
                crate::document::writer::transaction::TransactionKind::Interactive
            ),
            Err(ApplyError::Source(source))
        );
    }
    assert_eq!(doc.revision(), Revision(0));
    for source in [
        ChangeSource::Action,
        ChangeSource::Console,
        ChangeSource::Ipc,
        ChangeSource::Input,
        ChangeSource::Edit,
        ChangeSource::Verb,
    ] {
        doc.apply(
            place(vec![ids.a], Vec2::splat(doc.revision().0 as f32 + 1.0)),
            source,
        )
        .unwrap();
    }
    assert_eq!(doc.history().count(), 6);
    assert_eq!(position(&doc, ids.a), Some(Vec2::splat(6.0)));
}

/// A small deterministic generator for the session: the same seed, the
/// same session.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

/// What the session expects the history to hold: the saved text of the
/// base and of each applied entry, the undone ones' for a redo, each
/// with an identity of its own, under the same limit the document keeps;
/// and which of them was saved last.
struct Model {
    base: (u64, String),
    applied: Vec<(u64, String)>,
    undone: Vec<(u64, String)>,
    limit: usize,
    next: u64,
    saved: u64,
}

impl Model {
    fn new(text: String, limit: usize) -> Self {
        Model {
            base: (0, text),
            applied: Vec::new(),
            undone: Vec::new(),
            limit,
            next: 1,
            saved: 0,
        }
    }

    fn position(&self) -> u64 {
        self.applied.last().unwrap_or(&self.base).0
    }

    fn current(&self) -> &str {
        &self.applied.last().unwrap_or(&self.base).1
    }

    fn push(&mut self, text: String) {
        self.undone.clear();
        self.applied.push((self.next, text));
        self.next += 1;
        self.trim();
    }

    fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
        self.trim();
    }

    fn trim(&mut self) {
        while self.applied.len() + self.undone.len() > self.limit {
            if self.applied.is_empty() {
                self.undone.remove(0);
            } else {
                self.base = self.applied.remove(0);
            }
        }
    }

    fn undo(&mut self) -> bool {
        match self.applied.pop() {
            Some(entry) => {
                self.undone.push(entry);
                true
            }
            None => false,
        }
    }

    fn redo(&mut self) -> bool {
        match self.undone.pop() {
            Some(entry) => {
                self.applied.push(entry);
                true
            }
            None => false,
        }
    }

    fn is_dirty(&self) -> bool {
        self.position() != self.saved
    }
}

/// One random edit of `doc`'s current nodes, or `None` when the pick has
/// nothing to act on.
fn random_request(doc: &Document, rng: &mut Lcg) -> Option<EditRequest> {
    let nodes: Vec<NodeId> = doc
        .tree()
        .descendants()
        .filter(|&id| doc.get(id).map(DocElement::kind) == Some(DocKind::Node))
        .collect();
    let node = *nodes.get(rng.below(nodes.len()))?;
    let at = Vec2::new(rng.below(100) as f32, rng.below(100) as f32);
    let steps = match rng.below(8) {
        0 => return Some(fade(vec![node], 0.1 * (1 + rng.below(9)) as f32)),
        1 => return Some(place(vec![node], at)),
        2 => create_child(doc, node, at).ok()?,
        3 => create_root(doc, at).ok()?,
        4 => delete(doc, &[node]).ok()?,
        5 => {
            let target = *nodes.get(rng.below(nodes.len()))?;
            let to = if rng.below(3) == 0 { doc.root() } else { target };
            reparent(doc, &[node], to).ok()?
        }
        6 => {
            let section = *children_of(doc, node, DocKind::Section).first()?;
            insert_text(
                doc,
                section,
                rng.below(4),
                "\u{1F468}\u{200D}\u{1F469}x".to_string(),
            )
            .ok()?
        }
        _ => {
            let link = *children_of(doc, node, DocKind::Link).first()?;
            let kind = if rng.below(2) == 0 {
                LinkKind::Line
            } else {
                LinkKind::Portal
            };
            set_link_kind(doc, link, kind).ok()?
        }
    };
    Some(EditRequest::new(steps, "random"))
}

#[test]
fn test_writer_session_of_edits_undos_and_transactions_stays_exact() {
    do_writer_session_of_edits_undos_and_transactions_stays_exact();
}

/// Whether the staged change just made touched anything, draining the
/// feed: a stage that touches nothing makes no entry when committed.
fn staged_something(doc: &mut Document) -> bool {
    !doc.take_changes().is_empty()
}

/// A seeded session of six hundred moves on the example, under a history
/// limit the session itself moves — applies of every kind of edit
/// (refused ones included), undos, redos, saves marked, interactive
/// transactions with base calls and stages that commit, cancel, or meet
/// an undo, a redo or an apply, animations written part-way that commit,
/// or yield to an apply, a begin, an undo or a redo, saves with a stage
/// open — never loses its place: every undo and redo the model expects
/// succeeds and lands on the saved text the model holds for it, so no
/// journal is ever refused as out of order (A5's version stamps), and
/// the dirty flag is the model's, after every move. The state is held to
/// `assert_consistent` after every undo, every redo and every yield —
/// where the indexes are likeliest to drift — and after every tenth move
/// otherwise. Each kind of move is counted only when it did what it is
/// named for — an undo that undid, a cancel that took something back, a
/// yield of an animation that changed something — and the session must
/// have made each several times, or it proves nothing about the one it
/// skipped (ruling R37).
pub fn do_writer_session_of_edits_undos_and_transactions_stays_exact() {
    use crate::document::writer::transaction::TransactionKind;
    use crate::engine::interpolate::Snap;
    let (mut doc, _) = built();
    doc.set_history_limit(8);
    let mut model = Model::new(saved(&doc), 8);
    let mut rng = Lcg(0x5eed);
    let mut done = [0usize; 16];
    for step in 0..600 {
        // Set by every move that undoes, redoes or yields.
        let mut check = step % 10 == 0;
        match rng.below(10) {
            0 | 1 => {
                if let Some(request) = random_request(&doc, &mut rng) {
                    let (before, revision) = (saved(&doc), doc.revision());
                    match doc.apply(request, ChangeSource::Action) {
                        Ok(now) if now != revision => {
                            model.push(saved(&doc));
                            done[0] += 1;
                        }
                        Ok(_) => assert_eq!(saved(&doc), before, "step {step}: no change, no entry"),
                        Err(_) => assert_eq!(saved(&doc), before, "step {step}: a refusal changes nothing"),
                    }
                }
            }
            2 => {
                let undone = model.undo();
                assert_eq!(doc.undo().is_some(), undone, "step {step}: undo");
                done[1] += usize::from(undone);
                check = true;
            }
            3 => {
                let redone = model.redo();
                assert_eq!(doc.redo().is_some(), redone, "step {step}: redo");
                done[2] += usize::from(redone);
                check = true;
            }
            4 => {
                let limit = 2 + rng.below(10);
                doc.set_history_limit(limit);
                model.set_limit(limit);
                if rng.below(2) == 0 {
                    doc.mark_saved();
                    model.saved = model.position();
                }
                done[3] += 1;
            }
            5 | 6 => {
                let tx = doc
                    .begin(ChangeSource::Input, TransactionKind::Interactive)
                    .unwrap();
                let base = saved(&doc);
                if rng.below(2) == 0 {
                    if let Some(request) = random_request(&doc, &mut rng) {
                        if doc.apply_in(&tx, request).is_ok() {
                            done[4] += 1;
                        }
                    }
                }
                for _ in 0..=rng.below(3) {
                    if let Some(request) = random_request(&doc, &mut rng) {
                        let _ = doc.stage(&tx, request);
                    }
                }
                // Whether the transaction holds anything a save, a cancel
                // or an undo has to take back.
                let holds = saved(&doc) != base;
                if rng.below(3) == 0 {
                    assert_eq!(
                        doc.save_json().unwrap(),
                        base,
                        "step {step}: a save writes the committed state"
                    );
                    done[5] += usize::from(holds);
                }
                match rng.below(5) {
                    0 | 1 => {
                        let staged = saved(&doc);
                        if doc.commit(tx).is_some() {
                            assert_eq!(saved(&doc), staged, "step {step}: the commit keeps the stage");
                            model.push(staged);
                            done[6] += 1;
                        } else {
                            assert_eq!(
                                staged, base,
                                "step {step}: only a transaction that changed nothing makes no entry"
                            );
                        }
                    }
                    2 => {
                        doc.cancel(tx);
                        assert_eq!(saved(&doc), base, "step {step}: the cancel restores");
                        done[7] += usize::from(holds);
                    }
                    3 => {
                        let revision = doc.revision();
                        let answer = if rng.below(2) == 0 { doc.undo() } else { doc.redo() };
                        // A change the saved text cannot show — a portal's
                        // kept path — still counts as one the cancel took
                        // back, so only a visible one decides the answer.
                        if holds {
                            assert_eq!(
                                answer,
                                Some(revision),
                                "step {step}: the undo or redo only cancels"
                            );
                        } else {
                            assert!(
                                answer.is_none() || answer == Some(revision),
                                "step {step}: and answers None for a transaction that changed nothing"
                            );
                        }
                        assert_eq!(saved(&doc), base, "step {step}: and stops there");
                        done[8] += usize::from(holds);
                        check = true;
                    }
                    _ => {
                        let request = random_request(&doc, &mut rng).unwrap_or_else(|| fade(Vec::new(), 0.5));
                        assert_eq!(
                            doc.apply(request, ChangeSource::Action),
                            Err(ApplyError::TransactionOpen),
                            "step {step}: an interactive transaction refuses"
                        );
                        doc.cancel(tx);
                        done[9] += 1;
                    }
                }
            }
            _ => {
                let tx = doc.begin(ChangeSource::Edit, TransactionKind::Animated).unwrap();
                doc.take_changes();
                let staged =
                    random_request(&doc, &mut rng).is_some_and(|request| doc.stage(&tx, request).is_ok());
                if !staged {
                    doc.cancel(tx);
                    continue;
                }
                let changed = staged_something(&mut doc);
                let full = saved(&doc);
                let snap = if rng.below(2) == 0 { Snap::Start } else { Snap::End };
                doc.interpolate(&tx, rng.below(10) as f32 / 10.0, snap).unwrap();
                assert_consistent(&doc, "part-way");
                // Every arm finishes the animation first: committed, an
                // entry of its own when it changed something.
                let finish = |model: &mut Model| {
                    if changed {
                        model.push(full.clone());
                    }
                };
                match rng.below(5) {
                    0 => {
                        let committed = doc.commit(tx).is_some();
                        assert_eq!(committed, changed, "step {step}: the animation commits");
                        assert_eq!(saved(&doc), full, "step {step}: at its end");
                        finish(&mut model);
                        done[10] += usize::from(changed);
                    }
                    1 => {
                        finish(&mut model);
                        let revision = doc.revision();
                        let committed = Revision(revision.0 + u64::from(changed));
                        if let Some(request) = random_request(&doc, &mut rng) {
                            match doc.apply(request, ChangeSource::Action) {
                                Ok(now) if now != committed => model.push(saved(&doc)),
                                _ => {}
                            }
                            done[11] += usize::from(changed);
                        } else {
                            doc.commit(tx);
                        }
                        check = true;
                    }
                    2 => {
                        finish(&mut model);
                        let next = doc
                            .begin(ChangeSource::Input, TransactionKind::Interactive)
                            .unwrap();
                        assert_eq!(saved(&doc), full, "step {step}: a begin finishes the animation");
                        doc.cancel(next);
                        done[12] += usize::from(changed);
                        check = true;
                    }
                    3 => {
                        finish(&mut model);
                        assert_eq!(
                            doc.undo().is_some(),
                            model.undo(),
                            "step {step}: an undo of the animation"
                        );
                        done[13] += usize::from(changed);
                        check = true;
                    }
                    _ => {
                        finish(&mut model);
                        let answer = doc.redo();
                        let expected = model.redo();
                        assert_eq!(
                            answer.is_some(),
                            expected || changed,
                            "step {step}: a redo after the animation"
                        );
                        done[14] += usize::from(changed);
                        check = true;
                    }
                }
            }
        }
        assert_eq!(
            saved(&doc),
            model.current(),
            "step {step}: the document is where the model is"
        );
        assert_eq!(doc.redo_len(), model.undone.len(), "step {step}");
        assert_eq!(
            doc.is_dirty(),
            model.is_dirty(),
            "step {step}: dirty is the model's"
        );
        if check {
            assert_consistent(&doc, "a session step");
            done[15] += 1;
        }
    }
    assert!(
        done.iter().all(|&n| n >= 3),
        "every kind of move, several times: {done:?}"
    );
}

/// How many anchors the bench request has.
const FIFTY: usize = 50;

#[test]
fn test_writer_apply_of_a_fifty_anchor_request() {
    do_writer_apply_of_a_fifty_anchor_request();
}

/// The writer's bench row: one request that moves fifty root nodes, a
/// section each, applied as one entry, then undone and redone. Each node
/// lands where the request put it, and the undo puts every one back.
pub fn do_writer_apply_of_a_fifty_anchor_request() {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let nodes: Vec<NodeId> = (0..FIFTY)
        .map(|_| {
            let node = builder.node(root);
            let section = builder.section(node, 0);
            builder.span(section, "text");
            node
        })
        .collect();
    let mut doc = builder.build().unwrap();
    let at = Vec2::new(12.0, 34.0);
    assert_eq!(
        doc.apply(place(nodes.clone(), at), ChangeSource::Action),
        Ok(Revision(1))
    );
    assert!(
        nodes.iter().all(|&node| position(&doc, node) == Some(at)),
        "every anchor moved"
    );
    assert_eq!(doc.history().count(), 1, "one entry for fifty anchors");
    doc.undo().unwrap();
    assert!(
        nodes.iter().all(|&node| position(&doc, node) == Some(Vec2::ZERO)),
        "every one back"
    );
    doc.redo().unwrap();
    assert_eq!(
        doc.take_changes()
            .iter()
            .map(|record| record.touched.len())
            .sum::<usize>(),
        3 * FIFTY
    );
}

#[test]
fn test_writer_numbers_a_nested_insert_once_in_pre_order() {
    do_writer_numbers_a_nested_insert_once_in_pre_order();
}

/// One request inserts a root node and then, in a later step, a child
/// under the node the first step created, and a grandchild under that:
/// the three take consecutive idents in pre-order — top, child,
/// grandchild — and the counter moves by three, though the inner
/// subtrees are reached from more than one created root.
pub fn do_writer_numbers_a_nested_insert_once_in_pre_order() {
    let (mut doc, _) = built();
    let first = doc.next_ident().unwrap();
    // The last node of `parent`'s row once the earlier steps have run.
    let under_newest = |parent: fn(&Document) -> NodeId| {
        DocStep::new(move |doc: &Document| {
            let at = parent(doc);
            let Some(&newest) = children_of(doc, at, DocKind::Node).last() else {
                return Ok(None);
            };
            let mut steps = create_child(doc, newest, Vec2::ONE)?;
            match steps.pop() {
                Some(step) => step.build(doc),
                None => Ok(None),
            }
        })
    };
    let mut steps = create_root(&doc, Vec2::ZERO).unwrap();
    steps.push(under_newest(|doc| doc.root()));
    steps.push(under_newest(|doc| {
        *children_of(doc, doc.root(), DocKind::Node)
            .last()
            .expect("the new root")
    }));
    doc.apply(EditRequest::new(steps, "nested"), ChangeSource::Action)
        .unwrap();
    let top = *children_of(&doc, doc.root(), DocKind::Node).last().unwrap();
    let child = children_of(&doc, top, DocKind::Node)[0];
    let grandchild = children_of(&doc, child, DocKind::Node)[0];
    assert_eq!(
        [ident(&doc, top), ident(&doc, child), ident(&doc, grandchild)],
        [
            Some(first),
            Some(NodeIdent(first.0 + 1)),
            Some(NodeIdent(first.0 + 2))
        ],
        "consecutive, in pre-order"
    );
    assert_eq!(
        doc.next_ident(),
        Some(NodeIdent(first.0 + 3)),
        "no ident spent twice"
    );
    assert_consistent(&doc, "after the nested insert");
}
