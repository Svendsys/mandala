// SPDX-License-Identifier: MPL-2.0

//! Identity and the read API: idents and their counter, and every read
//! accessor of [`Document`](crate::document::ids::Document).

use crate::document::element::{DocElement, DocKind};
use crate::document::ids::{DocumentError, NodeIdent};
use crate::document::tests::test_support::{example, node_data, DocumentBuilder};
use crate::document::values::PaletteId;

#[test]
fn test_document_idents_index_every_node() {
    do_document_idents_index_every_node();
}

/// Every node is found by its ident, an ident nobody carries finds
/// nothing, and the counter starts one past the highest ident.
pub fn do_document_idents_index_every_node() {
    let (builder, ids) = example();
    let doc = builder.build().unwrap();
    for (ident, node) in [(1, ids.a), (2, ids.b), (3, ids.c), (4, ids.d)] {
        assert_eq!(doc.node_by_ident(NodeIdent(ident)), Some(node));
    }
    assert_eq!(doc.node_by_ident(NodeIdent(5)), None);
    assert_eq!(doc.next_ident(), Some(NodeIdent(5)));
}

#[test]
fn test_document_ident_counter_starts_past_the_highest() {
    do_document_ident_counter_starts_past_the_highest();
}

/// The counter starts at `max + 1`, not at `count + 1` and not after
/// the last node built: idents 3 and 40, built in that order or the
/// other, give 41. An empty map starts at 1, past the reserved 0.
pub fn do_document_ident_counter_starts_past_the_highest() {
    for order in [[40, 3], [3, 40]] {
        let mut builder = DocumentBuilder::new();
        let root = builder.root();
        for ident in order {
            builder.node_with(root, node_data(ident));
        }
        assert_eq!(builder.build().unwrap().next_ident(), Some(NodeIdent(41)));
    }
    assert_eq!(
        DocumentBuilder::new().build().unwrap().next_ident(),
        Some(NodeIdent(1))
    );
}

#[test]
fn test_document_refuses_a_duplicate_or_unassigned_ident() {
    do_document_refuses_a_duplicate_or_unassigned_ident();
}

/// Two nodes with one ident, or a node with the reserved ident 0, are
/// not a document.
pub fn do_document_refuses_a_duplicate_or_unassigned_ident() {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    builder.node_with(root, node_data(7));
    builder.node_with(root, node_data(8));
    assert!(
        builder.build_unchecked().is_ok(),
        "distinct idents are a document"
    );

    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    builder.node_with(root, node_data(7));
    builder.node_with(root, node_data(7));
    assert_eq!(
        builder.build_unchecked().unwrap_err(),
        DocumentError::DuplicateIdent(NodeIdent(7))
    );

    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let unassigned = builder.node_with(root, node_data(0));
    assert_eq!(
        builder.build_unchecked().unwrap_err(),
        DocumentError::UnassignedIdent(unassigned)
    );
}

#[test]
fn test_document_tree_reads() {
    do_document_tree_reads();
}

/// `root`, `get`, `children`, `parent`: the tree as built, and nothing
/// for an id that is not in it.
pub fn do_document_tree_reads() {
    let (builder, ids) = example();
    let stale = {
        let (mut other, _) = example();
        let root = other.root();
        other.node(root)
    };
    let doc = builder.build().unwrap();
    assert_eq!(doc.get(doc.root()).map(DocElement::kind), Some(DocKind::Map));
    assert_eq!(doc.get(ids.b).map(DocElement::kind), Some(DocKind::Node));
    assert_eq!(doc.parent(ids.b), Some(ids.a));
    assert_eq!(doc.parent(doc.root()), None);
    let children: Vec<_> = doc.children(ids.b).collect();
    assert_eq!(children, vec![ids.b_section, ids.b_connector, ids.c]);
    assert!(
        doc.get(stale).is_none(),
        "an id from another tree is not in this one"
    );
    assert_eq!(doc.children(stale).count(), 0);
    assert_eq!(doc.parent(stale), None);
}

#[test]
fn test_document_depth_counts_node_ancestors() {
    do_document_depth_counts_node_ancestors();
}

/// Depth is the number of nodes above: 0 for a root node, 1 for its
/// child and for the child's own section, 2 below that; elements above
/// the nodes are at 0.
pub fn do_document_depth_counts_node_ancestors() {
    let (builder, ids) = example();
    let theme = builder.theme_id();
    let doc = builder.build().unwrap();
    assert_eq!(doc.depth(ids.a), 0);
    assert_eq!(doc.depth(ids.d), 0);
    assert_eq!(doc.depth(ids.a_title_span), 1);
    assert_eq!(doc.depth(ids.b), 1);
    assert_eq!(doc.depth(ids.b_span), 2);
    assert_eq!(doc.depth(ids.c), 2);
    assert_eq!(doc.depth(ids.c_connector), 3);
    assert_eq!(doc.depth(theme), 0);
    assert_eq!(doc.depth(doc.root()), 0);
}

#[test]
fn test_document_owning_node() {
    do_document_owning_node();
}

/// The owning node is the nearest node at or above: the node itself,
/// the node a span, ring, label or portal end belongs to; none for the
/// map's own children.
pub fn do_document_owning_node() {
    let (builder, ids) = example();
    let theme = builder.theme_id();
    let doc = builder.build().unwrap();
    assert_eq!(doc.owning_node(ids.b), Some(ids.b));
    assert_eq!(doc.owning_node(ids.b_span), Some(ids.b));
    assert_eq!(doc.owning_node(ids.ring_span), Some(ids.a));
    assert_eq!(doc.owning_node(ids.b_label), Some(ids.b));
    assert_eq!(doc.owning_node(ids.portal_source), Some(ids.a));
    assert_eq!(doc.owning_node(theme), None);
    assert_eq!(doc.owning_node(doc.root()), None);
}

#[test]
fn test_document_map_reads() {
    do_document_map_reads();
}

/// The theme, the defaults and the palettes, numbered in tree order.
pub fn do_document_map_reads() {
    let (mut builder, _) = example();
    builder.palette(crate::document::element::PaletteData {
        name: "second".to_string(),
        groups: Vec::new(),
    });
    let mut defaults = crate::document::element::DefaultsData::default();
    defaults.section.italic = Some(true);
    builder.defaults(defaults);
    let doc = builder.build().unwrap();
    assert_eq!(
        doc.theme().variables,
        vec!["accent".to_string(), "fg".to_string()]
    );
    assert_eq!(doc.defaults().section.italic, Some(true));
    let palettes: Vec<(PaletteId, &str)> = doc.palettes().map(|(id, p)| (id, p.name.as_str())).collect();
    assert_eq!(
        palettes,
        vec![(PaletteId(0), "rainbow"), (PaletteId(1), "second")]
    );
    assert!(doc.definitions().verbs.is_empty());
}

#[test]
fn test_document_never_reissues_an_ident() {
    do_document_never_reissues_an_ident();
}

/// The bound and the refusal agree: a map whose highest ident is
/// `MAX_NODE_IDENT` loads — the bound admits it — and has no ident left
/// to issue, rather than a counter that would issue one no load accepts;
/// an ident past the bound is refused outright. Control: one below the
/// bound leaves the counter at `MAX_NODE_IDENT`.
pub fn do_document_never_reissues_an_ident() {
    use crate::document::bounds::{check_node_ident, BoundError, MAX_NODE_IDENT};
    let with_ident = |ident| {
        let mut builder = DocumentBuilder::new();
        let root = builder.root();
        builder.node_with(root, node_data(ident));
        builder.build_unchecked()
    };
    assert_eq!(
        with_ident(MAX_NODE_IDENT - 1).unwrap().next_ident(),
        Some(NodeIdent(MAX_NODE_IDENT))
    );
    assert_eq!(check_node_ident(u64::from(MAX_NODE_IDENT)), Ok(MAX_NODE_IDENT));
    let at_bound = with_ident(MAX_NODE_IDENT).expect("an ident the bound admits loads");
    assert_eq!(at_bound.next_ident(), None, "no ident is left to issue");
    assert_eq!(
        with_ident(u32::MAX).unwrap_err(),
        DocumentError::Bound {
            what: "node id",
            error: BoundError::NotAnIdent(u64::from(u32::MAX)),
        }
    );
}

#[test]
fn test_document_refuses_more_than_an_index_addresses() {
    do_document_refuses_more_than_an_index_addresses();
}

/// One palette, theme variable or verb past its bound is refused, and
/// the palettes past the bound are never numbered with an id another
/// palette has. Controls: exactly the bound is a document.
pub fn do_document_refuses_more_than_an_index_addresses() {
    use crate::document::bounds::{BoundError, MAX_PALETTES, MAX_THEME_VARIABLES, MAX_VERBS};
    use crate::document::element::{PaletteData, ThemeData};
    use crate::document::ids::{palettes, Definitions, VerbDef};

    let with_palettes = |n: usize| {
        let mut builder = DocumentBuilder::new();
        for _ in 0..n {
            builder.palette(PaletteData::default());
        }
        builder
    };
    let at_bound = with_palettes(MAX_PALETTES);
    assert!(at_bound.build_unchecked().is_ok());
    let past = with_palettes(MAX_PALETTES + 1);
    let numbered: Vec<PaletteId> = palettes(past.tree()).map(|(id, _)| id).collect();
    assert_eq!(numbered.len(), MAX_PALETTES, "the one past the bound gets no id");
    assert_eq!(numbered.last(), Some(&PaletteId(u16::MAX)));
    assert_eq!(
        past.build_unchecked().unwrap_err(),
        DocumentError::Bound {
            what: "palettes",
            error: BoundError::TooMany {
                count: MAX_PALETTES + 1,
                max: MAX_PALETTES,
            },
        }
    );

    let with_variables = |n: usize| {
        let mut builder = DocumentBuilder::new();
        builder.theme(ThemeData {
            variables: vec![String::new(); n],
            variants: Vec::new(),
            active: 0,
        });
        builder.build_unchecked()
    };
    assert!(with_variables(MAX_THEME_VARIABLES).is_ok());
    assert!(matches!(
        with_variables(MAX_THEME_VARIABLES + 1),
        Err(DocumentError::Bound {
            what: "theme variables",
            ..
        })
    ));

    let with_verbs = |n: usize| {
        let mut builder = DocumentBuilder::new();
        let verb = VerbDef {
            id: String::new(),
            name: String::new(),
        };
        builder.definitions(Definitions {
            verbs: vec![verb; n],
            ..Definitions::default()
        });
        builder.build_unchecked()
    };
    assert!(with_verbs(MAX_VERBS).is_ok());
    assert!(matches!(
        with_verbs(MAX_VERBS + 1),
        Err(DocumentError::Bound { what: "verbs", .. })
    ));
}

#[test]
fn test_the_ident_counter_starts_above_every_ident_the_map_names() {
    use crate::document::bounds::MAX_NODE_IDENT;
    use crate::document::ids::first_free_ident;
    // Q3 (ruling R20 on #236). The node with ident 5 was deleted while a
    // definition still names it: after a save and a reload the highest
    // node is 4, and the next node must still not be 5, or the target
    // would silently retarget to it.
    let highest_left = 4;
    let deleted = 5;
    let next = first_free_ident(highest_left, [deleted, 2]);
    assert_eq!(next, 6);
    assert_ne!(next, deleted, "a new node took an ident a definition names");
    // Nothing named: one above the highest node.
    assert_eq!(first_free_ident(highest_left, []), 5);
    assert_eq!(first_free_ident(0, []), 1);
    // A named ident below the highest changes nothing.
    assert_eq!(first_free_ident(9, [3]), 10);
    // At the bound the counter passes it, and no ident is issued.
    assert_eq!(first_free_ident(MAX_NODE_IDENT, [3]), MAX_NODE_IDENT + 1);
    assert_eq!(first_free_ident(3, [u32::MAX]), u32::MAX);
}

#[test]
fn test_a_loaded_map_starts_its_counter_through_the_one_hook() {
    use crate::document::ids::{first_free_ident, idents_named_by};
    use crate::document::tests::format_text::{map_with, node_of};
    // A map whose verbs and presentations name no node yet (their
    // targets arrive with V1-D1–D3): the counter is one above the
    // highest node, through the same hook that will count them.
    let json = map_with(
        r#","verbs":[{"id":"v","name":"V"}],"presentations":[{"id":"p","name":"P"}]"#,
        &format!("{},{}", node_of(7, ""), node_of(3, "")),
    );
    let doc = crate::document::format::load(&json).expect("the map loads");
    assert_eq!(idents_named_by(doc.definitions()).count(), 0);
    assert_eq!(
        doc.next_ident(),
        Some(NodeIdent(first_free_ident(7, idents_named_by(doc.definitions()))))
    );
    assert_eq!(doc.next_ident(), Some(NodeIdent(8)));
}
