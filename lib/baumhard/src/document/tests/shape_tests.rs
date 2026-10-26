// SPDX-License-Identifier: MPL-2.0

//! The shape rules: the full example passes, and each row of the table
//! is broken once. Each rejection test first checks the row it is about
//! to break is valid — the negative control — then breaks it and
//! expects the error that names the break.

use crate::document::element::{
    CapData, DocElement, DocKind, FrameData, LabelData, MapData, PortalEndData, RingData, ThemeData,
};
use crate::document::shape::{validate_shape, Cardinality, ShapeError};
use crate::document::tests::test_support::{example, span, DocumentBuilder};
use crate::document::values::LinkKind;
use crate::engine::tree::Tree;
use indextree::NodeId;

/// `validate_shape` at `id` in `builder`'s tree.
fn check(builder: &DocumentBuilder, id: NodeId) -> Result<(), ShapeError> {
    validate_shape(builder.tree(), id)
}

#[test]
fn test_document_shape_accepts_the_full_example() {
    do_document_shape_accepts_the_full_example();
}

pub fn do_document_shape_accepts_the_full_example() {
    let (builder, _) = example();
    let kinds: Vec<DocKind> = builder
        .tree()
        .descendants()
        .map(|id| builder.tree().arena()[id].get().kind())
        .collect();
    for kind in DocKind::ALL {
        assert!(
            kinds.contains(&kind),
            "the example must hold a {kind:?}, or it accepts less than it claims"
        );
    }
    for id in builder.tree().descendants() {
        assert_eq!(
            check(&builder, id),
            Ok(()),
            "{:?}",
            builder.tree().arena()[id].get().kind()
        );
    }
    assert!(builder.build().is_ok());
    // The rejection tests below lean on this: they build the example
    // again and use the ids of an earlier build.
    assert_eq!(example().1, example().1);
}

#[test]
fn test_document_shape_map_row() {
    do_document_shape_map_row();
}

/// Map: Theme exactly 1, Palette*, Defaults exactly 1, Node*; nothing
/// else; the root is a map and no other element is.
pub fn do_document_shape_map_row() {
    let (mut builder, ids) = example();
    let root = builder.root();
    assert_eq!(check(&builder, root), Ok(()));

    builder.add(root, DocElement::Theme(ThemeData::default()));
    assert_eq!(
        check(&builder, root),
        Err(ShapeError::Count {
            parent: root,
            kind: DocKind::Theme,
            found: 2,
            allowed: Cardinality::Exactly(1),
        })
    );

    let (mut builder, _) = example();
    let defaults = builder.defaults_id();
    let mut tree = builder.tree().clone();
    tree.remove_subtree(defaults).unwrap();
    assert_eq!(
        validate_shape(&tree, root),
        Err(ShapeError::Count {
            parent: root,
            kind: DocKind::Defaults,
            found: 0,
            allowed: Cardinality::Exactly(1),
        })
    );

    let section = builder.add(root, DocElement::Section(Default::default()));
    assert_eq!(
        check(&builder, root),
        Err(ShapeError::ChildNotAllowed {
            parent: root,
            parent_kind: DocKind::Map,
            child: section,
            child_kind: DocKind::Section,
        })
    );

    let (mut builder, _) = example();
    let inner = builder.add(ids.a, DocElement::Map(MapData::default()));
    assert_eq!(check(&builder, inner), Err(ShapeError::MapNotRoot(inner)));

    let not_a_map: Tree<DocElement> = Tree::new_non_indexed_with(span("x"));
    assert_eq!(
        validate_shape(&not_a_map, not_a_map.root()),
        Err(ShapeError::RootNotMap { kind: DocKind::Span })
    );
}

#[test]
fn test_document_shape_node_row() {
    do_document_shape_node_row();
}

/// Node: Section*, Frame ≤ 1, Connector exactly 1 under a node and none
/// on a root, Link*, Node*.
pub fn do_document_shape_node_row() {
    let (mut builder, ids) = example();
    assert_eq!(check(&builder, ids.a), Ok(()));
    builder.add(ids.a, DocElement::Frame(FrameData::default()));
    assert_eq!(
        check(&builder, ids.a),
        Err(ShapeError::Count {
            parent: ids.a,
            kind: DocKind::Frame,
            found: 2,
            allowed: Cardinality::AtMost(1),
        })
    );

    // A connector on a root node.
    let (mut builder, ids) = example();
    assert_eq!(check(&builder, ids.d), Ok(()));
    let connector = builder.connector(ids.d);
    assert_eq!(
        check(&builder, ids.d),
        Err(ShapeError::ChildNotAllowed {
            parent: ids.d,
            parent_kind: DocKind::Node,
            child: connector,
            child_kind: DocKind::Connector,
        })
    );

    // A child node with no connector, and one with two.
    let (builder, ids) = example();
    assert_eq!(check(&builder, ids.c), Ok(()));
    let mut tree = builder.tree().clone();
    tree.remove_subtree(ids.c_connector).unwrap();
    assert_eq!(
        validate_shape(&tree, ids.c),
        Err(ShapeError::Count {
            parent: ids.c,
            kind: DocKind::Connector,
            found: 0,
            allowed: Cardinality::Exactly(1),
        })
    );
    let (mut builder, ids) = example();
    builder.connector(ids.c);
    assert_eq!(
        check(&builder, ids.c),
        Err(ShapeError::Count {
            parent: ids.c,
            kind: DocKind::Connector,
            found: 2,
            allowed: Cardinality::Exactly(1),
        })
    );

    // A node holding a span.
    let (mut builder, ids) = example();
    let stray = builder.span(ids.d, "stray");
    assert!(matches!(
        check(&builder, ids.d),
        Err(ShapeError::ChildNotAllowed { child, child_kind: DocKind::Span, .. }) if child == stray
    ));
}

#[test]
fn test_document_shape_section_row() {
    do_document_shape_section_row();
}

/// Section: Span* only.
pub fn do_document_shape_section_row() {
    let (mut builder, ids) = example();
    builder.span(ids.a_title, "another");
    assert_eq!(check(&builder, ids.a_title), Ok(()));
    let ring = builder.add(ids.a_title, DocElement::Ring(RingData::default()));
    assert_eq!(
        check(&builder, ids.a_title),
        Err(ShapeError::ChildNotAllowed {
            parent: ids.a_title,
            parent_kind: DocKind::Section,
            child: ring,
            child_kind: DocKind::Ring,
        })
    );
}

#[test]
fn test_document_shape_frame_row() {
    do_document_shape_frame_row();
}

/// Frame: Ring* only.
pub fn do_document_shape_frame_row() {
    let (mut builder, ids) = example();
    builder.add(
        ids.frame,
        DocElement::Ring(RingData {
            channel: 1,
            ..Default::default()
        }),
    );
    assert_eq!(check(&builder, ids.frame), Ok(()));
    let stray = builder.span(ids.frame, "stray");
    assert_eq!(
        check(&builder, ids.frame),
        Err(ShapeError::ChildNotAllowed {
            parent: ids.frame,
            parent_kind: DocKind::Frame,
            child: stray,
            child_kind: DocKind::Span,
        })
    );
}

#[test]
fn test_document_shape_span_holder_rows() {
    do_document_shape_span_holder_rows();
}

/// Ring, CapStart, CapEnd, Label: Span* only.
pub fn do_document_shape_span_holder_rows() {
    let (builder, ids) = example();
    let holders = [ids.ring, ids.b_cap, ids.b_label];
    let mut cap_end = None;
    for id in ids.b_connector.children(builder.tree().arena()) {
        if builder.tree().arena()[id].get().kind() == DocKind::CapEnd {
            cap_end = Some(id);
        }
    }
    for holder in holders.into_iter().chain(cap_end) {
        let (mut builder, _) = example();
        builder.span(holder, "fine");
        assert_eq!(check(&builder, holder), Ok(()));
        let label = builder.add(holder, DocElement::Label(LabelData::default()));
        let kind = builder.tree().arena()[holder].get().kind();
        assert_eq!(
            check(&builder, holder),
            Err(ShapeError::ChildNotAllowed {
                parent: holder,
                parent_kind: kind,
                child: label,
                child_kind: DocKind::Label,
            })
        );
    }
}

#[test]
fn test_document_shape_connector_row() {
    do_document_shape_connector_row();
}

/// Connector: Span*, CapStart ≤ 1, CapEnd ≤ 1, Label ≤ 1.
pub fn do_document_shape_connector_row() {
    let (mut builder, ids) = example();
    assert_eq!(check(&builder, ids.b_connector), Ok(()));
    builder.add(ids.b_connector, DocElement::CapStart(CapData::default()));
    assert_eq!(
        check(&builder, ids.b_connector),
        Err(ShapeError::Count {
            parent: ids.b_connector,
            kind: DocKind::CapStart,
            found: 2,
            allowed: Cardinality::AtMost(1),
        })
    );
    let (mut builder, ids) = example();
    let end = builder.add(ids.b_connector, DocElement::PortalEnd(PortalEndData::default()));
    assert_eq!(
        check(&builder, ids.b_connector),
        Err(ShapeError::ChildNotAllowed {
            parent: ids.b_connector,
            parent_kind: DocKind::Connector,
            child: end,
            child_kind: DocKind::PortalEnd,
        })
    );
}

#[test]
fn test_document_shape_line_link_row() {
    do_document_shape_line_link_row();
}

/// Link, kind Line: Span*, CapStart ≤ 1, CapEnd ≤ 1, Label ≤ 1; no
/// portal end.
pub fn do_document_shape_line_link_row() {
    let (mut builder, ids) = example();
    assert_eq!(check(&builder, ids.line), Ok(()));
    builder.add(ids.line, DocElement::Label(LabelData::default()));
    assert_eq!(
        check(&builder, ids.line),
        Err(ShapeError::Count {
            parent: ids.line,
            kind: DocKind::Label,
            found: 2,
            allowed: Cardinality::AtMost(1),
        })
    );
    let (mut builder, ids) = example();
    let end = builder.add(ids.line, DocElement::PortalEnd(PortalEndData::default()));
    assert_eq!(
        check(&builder, ids.line),
        Err(ShapeError::ChildNotAllowed {
            parent: ids.line,
            parent_kind: DocKind::Link,
            child: end,
            child_kind: DocKind::PortalEnd,
        })
    );
}

#[test]
fn test_document_shape_portal_link_row() {
    do_document_shape_portal_link_row();
}

/// Link, kind Portal: exactly two portal ends, channels 0 and 1, and
/// nothing else.
pub fn do_document_shape_portal_link_row() {
    let (mut builder, ids) = example();
    assert_eq!(check(&builder, ids.portal), Ok(()));
    builder.add(
        ids.portal,
        DocElement::PortalEnd(PortalEndData {
            end: 1,
            ..Default::default()
        }),
    );
    assert_eq!(
        check(&builder, ids.portal),
        Err(ShapeError::Count {
            parent: ids.portal,
            kind: DocKind::PortalEnd,
            found: 3,
            allowed: Cardinality::Exactly(2),
        })
    );

    let (mut builder, ids) = example();
    let stray = builder.span(ids.portal, "stray");
    assert_eq!(
        check(&builder, ids.portal),
        Err(ShapeError::ChildNotAllowed {
            parent: ids.portal,
            parent_kind: DocKind::Link,
            child: stray,
            child_kind: DocKind::Span,
        })
    );

    // Two ends, but both the source.
    let (mut builder, ids) = example();
    builder.edit(ids.portal_target, |e| {
        if let DocElement::PortalEnd(end) = e {
            end.end = 0;
        }
    });
    assert_eq!(
        check(&builder, ids.portal),
        Err(ShapeError::PortalEnds { link: ids.portal })
    );

    // The same row is a line link's when the kind says so.
    let (mut builder, ids) = example();
    builder.edit(ids.portal, |e| {
        if let DocElement::Link(link) = e {
            link.kind = LinkKind::Line;
        }
    });
    assert!(matches!(
        check(&builder, ids.portal),
        Err(ShapeError::ChildNotAllowed {
            child_kind: DocKind::PortalEnd,
            ..
        })
    ));
}

#[test]
fn test_document_shape_portal_end_row() {
    do_document_shape_portal_end_row();
}

/// PortalEnd: Span* (the marker), Label ≤ 1.
pub fn do_document_shape_portal_end_row() {
    let (mut builder, ids) = example();
    assert_eq!(check(&builder, ids.portal_source), Ok(()));
    builder.add(ids.portal_source, DocElement::Label(LabelData::default()));
    assert_eq!(
        check(&builder, ids.portal_source),
        Err(ShapeError::Count {
            parent: ids.portal_source,
            kind: DocKind::Label,
            found: 2,
            allowed: Cardinality::AtMost(1),
        })
    );
    let (mut builder, ids) = example();
    let cap = builder.add(ids.portal_source, DocElement::CapStart(CapData::default()));
    assert_eq!(
        check(&builder, ids.portal_source),
        Err(ShapeError::ChildNotAllowed {
            parent: ids.portal_source,
            parent_kind: DocKind::PortalEnd,
            child: cap,
            child_kind: DocKind::CapStart,
        })
    );
}

#[test]
fn test_document_shape_leaf_rows() {
    do_document_shape_leaf_rows();
}

/// Span, Theme, Palette, Defaults: no children at all.
pub fn do_document_shape_leaf_rows() {
    let (builder, ids) = example();
    let palette = builder
        .root()
        .children(builder.tree().arena())
        .find(|&c| builder.tree().arena()[c].get().kind() == DocKind::Palette)
        .unwrap();
    for leaf in [
        ids.a_title_span,
        builder.theme_id(),
        palette,
        builder.defaults_id(),
    ] {
        let (mut builder, _) = example();
        assert_eq!(check(&builder, leaf), Ok(()));
        let child = builder.span(leaf, "below");
        let kind = builder.tree().arena()[leaf].get().kind();
        assert_eq!(
            check(&builder, leaf),
            Err(ShapeError::ChildNotAllowed {
                parent: leaf,
                parent_kind: kind,
                child,
                child_kind: DocKind::Span,
            })
        );
    }
}

#[test]
fn test_document_shape_checks_only_its_own_row() {
    do_document_shape_checks_only_its_own_row();
}

/// A broken row below is the business of the element that holds it: the
/// node above a broken section passes, and the section does not.
pub fn do_document_shape_checks_only_its_own_row() {
    let (mut builder, ids) = example();
    builder.add(ids.a_title, DocElement::Ring(RingData::default()));
    assert_eq!(check(&builder, ids.a), Ok(()));
    assert!(check(&builder, ids.a_title).is_err());
    assert!(builder.build().is_err(), "build checks every row");
}
