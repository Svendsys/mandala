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
use indextree::NodeId;

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
/// record; one undo takes all three back.
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
    doc.undo().unwrap();
    assert_eq!(saved(&doc), original, "one undo restores every anchor");
    assert_eq!(doc.redo_len(), 1);
}

#[test]
fn test_writer_dirty_is_the_saved_state_not_a_count() {
    do_writer_dirty_is_the_saved_state_not_a_count();
}

/// `mark_saved`, an edit, its undo: clean. An edit, its undo, a
/// different edit: dirty, though the history is as long as it was when
/// saved. A redo away from the saved state is dirty; the undo back is
/// clean. An entry dropped off the end of the history takes the saved
/// state with it when it was the saved one.
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
}

#[test]
fn test_writer_history_keeps_its_limit_and_frees_what_leaves_it() {
    do_writer_history_keeps_its_limit_and_frees_what_leaves_it();
}

/// With a limit of 3 and five deletes, the oldest two entries are
/// dropped and the nodes they removed are freed — their ids stale —
/// while the three kept still hold theirs, detached, for an undo. Two
/// undos and a new edit clear the redo branch, and the nodes the
/// undone entries had created are freed with it.
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
/// record names what it removed with the address each had — the link it
/// took is the second of `a`'s links — and the undo's record, as
/// [`ChangeSource::Undo`], names them back in. The undo of a create
/// reports the created node removed, with its own ident.
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
            index: 1,
        }),
        "d, the second root node"
    );
    assert_eq!(
        address(ids.portal),
        Some(RemovedAddress {
            node: a_ident,
            kind: DocKind::Link,
            channel: 0,
            index: 1,
        }),
        "the portal, a's second link"
    );
    assert_eq!(
        address(ids.portal_target),
        Some(RemovedAddress {
            node: a_ident,
            kind: DocKind::PortalEnd,
            channel: 1,
            index: 0,
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
            index: 2,
        }),
        "the undone node, third of the root nodes"
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
/// base and of each applied entry, the undone ones' for a redo, under
/// the same limit the document keeps.
struct Model {
    base: String,
    applied: Vec<String>,
    undone: Vec<String>,
    limit: usize,
}

impl Model {
    fn current(&self) -> &str {
        self.applied.last().unwrap_or(&self.base)
    }

    fn push(&mut self, text: String) {
        self.undone.clear();
        self.applied.push(text);
        while self.applied.len() + self.undone.len() > self.limit {
            self.base = self.applied.remove(0);
        }
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

/// A seeded session of four hundred moves on the example under a
/// history limit of eight — applies of every kind of edit (refused ones
/// included), undos, redos, interactive transactions committed or
/// cancelled, animations written part-way and committed, saves with a
/// stage open — never loses its place: every undo and redo the model
/// expects succeeds and lands on the saved text the model holds for it,
/// so no journal is ever refused as out of order (A5's version stamps),
/// and every state is consistent. The session must have done each kind
/// of move, or it proves nothing about the one it skipped.
pub fn do_writer_session_of_edits_undos_and_transactions_stays_exact() {
    use crate::document::writer::transaction::TransactionKind;
    use crate::engine::interpolate::Snap;
    let (mut doc, _) = built();
    let limit = 8;
    doc.set_history_limit(limit);
    let mut model = Model {
        base: saved(&doc),
        applied: Vec::new(),
        undone: Vec::new(),
        limit,
    };
    let mut rng = Lcg(0x5eed);
    let mut done = [0usize; 7];
    for step in 0..400 {
        match rng.below(7) {
            0 => {
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
            1 => {
                let expected = model.applied.pop();
                assert_eq!(doc.undo().is_some(), expected.is_some(), "step {step}: undo");
                if let Some(text) = expected {
                    model.undone.push(text);
                    done[1] += 1;
                }
            }
            2 => {
                let expected = model.undone.pop();
                assert_eq!(doc.redo().is_some(), expected.is_some(), "step {step}: redo");
                if let Some(text) = expected {
                    model.applied.push(text);
                    done[2] += 1;
                }
            }
            3 | 4 => {
                let tx = doc
                    .begin(ChangeSource::Input, TransactionKind::Interactive)
                    .unwrap();
                let base = saved(&doc);
                for _ in 0..=rng.below(3) {
                    if let Some(request) = random_request(&doc, &mut rng) {
                        let _ = doc.stage(&tx, request);
                    }
                }
                if rng.below(3) == 0 {
                    assert_eq!(
                        doc.save_json().unwrap(),
                        base,
                        "step {step}: a save writes the committed state"
                    );
                    done[6] += 1;
                }
                if rng.below(2) == 0 {
                    let staged = saved(&doc);
                    if doc.commit(tx).is_some() {
                        assert_eq!(saved(&doc), staged, "step {step}: the commit keeps the stage");
                        model.push(staged);
                        done[3] += 1;
                    }
                } else {
                    doc.cancel(tx);
                    assert_eq!(saved(&doc), base, "step {step}: the cancel restores");
                    done[4] += 1;
                }
            }
            _ => {
                let tx = doc.begin(ChangeSource::Edit, TransactionKind::Animated).unwrap();
                let staged =
                    random_request(&doc, &mut rng).is_some_and(|request| doc.stage(&tx, request).is_ok());
                if staged {
                    let full = saved(&doc);
                    let snap = if rng.below(2) == 0 { Snap::Start } else { Snap::End };
                    doc.interpolate(&tx, rng.below(10) as f32 / 10.0, snap).unwrap();
                    assert_consistent(&doc, "part-way");
                    if doc.commit(tx).is_some() {
                        assert_eq!(saved(&doc), full, "step {step}: committed at its end");
                        model.push(full);
                        done[5] += 1;
                    }
                } else {
                    doc.cancel(tx);
                }
            }
        }
        assert_eq!(
            saved(&doc),
            model.current(),
            "step {step}: the document is where the model is"
        );
        assert_eq!(doc.redo_len(), model.undone.len(), "step {step}");
        assert_consistent(&doc, "a session step");
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
