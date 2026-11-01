// SPDX-License-Identifier: MPL-2.0

//! What the writer's change feed costs, counted rather than timed: a
//! removed element's address is found by walking back through the
//! siblings of its own key, so the work follows the journal and the run
//! the element stood in, never the length of a row (rule 5, ruling R36
//! on #238). Reads the `cfg(test)` step counter of
//! `document::writer::feed`, so these are plain tests (TEST_CONVENTIONS
//! §T2.2, a `cfg(test)` sibling carries no bench row).

use crate::document::element::{DocElement, DocKind, LinkData};
use crate::document::mutation::compose::{create_root, delete, remove_links};
use crate::document::tests::test_support::{path_data, DocumentBuilder};
use crate::document::tests::writer_support::{assert_consistent, children_of};
use crate::document::values::LinkKind;
use crate::document::writer::feed::steps;
use crate::document::writer::{ChangeSource, EditRequest};
use glam::Vec2;
use indextree::NodeId;

/// How long the rows are that a change must not read.
const WIDE: usize = 1_000;

/// The siblings `f` made the feed read.
fn steps_of(f: impl FnOnce()) -> usize {
    let before = steps::taken();
    f();
    steps::taken() - before
}

/// A map of [`WIDE`] root nodes, the first of which holds [`WIDE`]
/// sections and one link to the second.
fn wide() -> (crate::document::ids::Document, Vec<NodeId>, NodeId) {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let nodes: Vec<NodeId> = (0..WIDE).map(|_| builder.node(root)).collect();
    for channel in 0..WIDE {
        builder.section(nodes[0], channel);
    }
    let link = builder.add(
        nodes[0],
        DocElement::Link(LinkData {
            to: nodes[1],
            kind: LinkKind::Line,
            path: path_data(),
        }),
    );
    (builder.build().unwrap(), nodes, link)
}

/// Deleting one of a thousand root nodes, undoing the create of another,
/// and removing the one link behind a thousand sections each read a
/// sibling or two — a node is named by its id and ranks nothing, and a
/// link's run is one link long — where reading the rows would read a
/// thousand. The precondition is that the rows are that long.
#[test]
fn test_writer_feed_reads_the_run_not_the_row() {
    let (mut doc, nodes, link) = wide();
    assert_eq!(
        children_of(&doc, doc.root(), DocKind::Node).len(),
        WIDE,
        "precondition"
    );
    assert_eq!(
        children_of(&doc, nodes[0], DocKind::Section).len(),
        WIDE,
        "precondition"
    );

    let deleted = steps_of(|| {
        let steps = delete(&doc, &[nodes[WIDE / 2]]).unwrap();
        doc.apply(EditRequest::new(steps, "delete"), ChangeSource::Action)
            .unwrap();
    });
    assert_eq!(deleted, 0, "a removed node ranks nothing");

    let steps = create_root(&doc, Vec2::ZERO).unwrap();
    doc.apply(EditRequest::new(steps, "create"), ChangeSource::Action)
        .unwrap();
    let undone = steps_of(|| {
        doc.undo().unwrap();
    });
    assert!(
        undone <= 1,
        "the undone node's section is first in its row: {undone}"
    );

    let unlinked = steps_of(|| {
        doc.apply(
            EditRequest::new(vec![remove_links(vec![link])], "unlink"),
            ChangeSource::Action,
        )
        .unwrap();
    });
    assert!(unlinked <= 1, "the link is alone in its run: {unlinked}");
    let records = doc.take_changes();
    let address = records
        .last()
        .and_then(|record| record.removed.iter().find(|(id, _)| *id == link))
        .map(|(_, address)| address.index);
    assert_eq!(address, Some(Some(0)), "and its rank is still found");
    assert_consistent(&doc, "after the wide changes");
}
