// SPDX-License-Identifier: MPL-2.0

//! `DocInherited`: what a walk carries down, held against the resolver.
//!
//! The oracle is [`resolve_text`]: the size an element whose own size is
//! unset displays is what `resolve_text` gives it, and
//! `DocInherited::text_size_for` must answer exactly that — through the
//! seed of an anchored walk, and through the walker's own `inherit` on a
//! walk from the map that reaches every element of the example map. The
//! fixtures set the `Defaults` tiers and the owners' sizes to values that
//! differ from each other and from `BUILT_IN`, so a tier read in the
//! wrong order, or read where it must not be, gives another number.

use crate::document::element::{DefaultsData, DocElement, DocKind};
use crate::document::ids::Document;
use crate::document::mutation::inherited::inherited_seed;
use crate::document::mutation::ops::{
    DocMutation, DocMutatorTree, FrameOp, LabelOp, NodeOp, RingOp, SectionOp, SpanOp, TextOp,
};
use crate::document::mutation::step::{DocStep, DocWalk};
use crate::document::resolve::resolve_text;
use crate::document::tests::test_support::{connector_of, example, DocumentBuilder, Example};
use crate::document::values::FontSize;
use crate::engine::anchor::Anchors;
use crate::engine::element::TreeElement;
use crate::engine::journal::JournalBuilder;
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use crate::engine::tree::{Placement, Tree};
use indextree::NodeId;

/// The kinds a `TextOp` reaches.
const TEXT_KINDS: [DocKind; 8] = [
    DocKind::Node,
    DocKind::Section,
    DocKind::Span,
    DocKind::Frame,
    DocKind::Ring,
    DocKind::CapStart,
    DocKind::CapEnd,
    DocKind::Label,
];

/// `op` wrapped in the payload of `kind`.
fn text_payload(kind: DocKind, op: TextOp) -> DocMutation {
    match kind {
        DocKind::Node => DocMutation::Node(NodeOp::Text(op)),
        DocKind::Section => DocMutation::Section(SectionOp::Text(op)),
        DocKind::Span => DocMutation::Span(SpanOp::Text(op)),
        DocKind::Frame => DocMutation::Frame(FrameOp::Text(op)),
        DocKind::Ring => DocMutation::Ring(RingOp::Text(op)),
        DocKind::CapStart | DocKind::CapEnd => DocMutation::Cap(op),
        DocKind::Label => DocMutation::Label(LabelOp::Text(op)),
        other => panic!("{other:?} holds no text style"),
    }
}

/// Every `Defaults` size set, each to its own value.
fn every_default() -> DefaultsData {
    let mut d = DefaultsData::default();
    d.node.text.size = Some(FontSize::Pt(31.0));
    d.section.size = Some(FontSize::Pt(32.0));
    d.span.size = Some(FontSize::Pt(33.0));
    d.frame.text.size = Some(FontSize::Rel(0.11));
    d.ring.text.size = Some(FontSize::Rel(0.12));
    d.connector.size = Some(34.0);
    d.link.size = Some(35.0);
    d.label.text.size = Some(FontSize::Pt(36.0));
    d.portal_end.size = Some(37.0);
    d
}

/// Some defaults set and some not, so a tail that skips an unset tier
/// is seen to fall through to the next.
fn some_defaults() -> DefaultsData {
    let mut d = DefaultsData::default();
    d.node.text.size = Some(FontSize::Pt(41.0));
    d.frame.text.size = Some(FontSize::Rel(0.13));
    d.label.text.size = Some(FontSize::Pt(42.0));
    d
}

/// Set the owners' sizes: the node's text, a section, the frame, the
/// ring, a path, a cap, labels and a portal end.
fn set_owners(builder: &mut DocumentBuilder, ids: &Example) {
    builder.edit(ids.a, |e| set_size(e, FontSize::Pt(30.0)));
    builder.edit(ids.a_title, |e| set_size(e, FontSize::Pt(21.0)));
    builder.edit(ids.frame, |e| set_size(e, FontSize::Rel(0.2)));
    builder.edit(ids.ring, |e| set_size(e, FontSize::Rel(0.15)));
    builder.edit(ids.line, |e| {
        if let DocElement::Link(link) = e {
            link.path.flow.size = Some(15.0);
        }
    });
    builder.edit(ids.b_connector, |e| {
        if let DocElement::Connector(c) = e {
            c.path.flow.size = Some(13.0);
        }
    });
    builder.edit(ids.b_cap, |e| set_size(e, FontSize::Pt(16.0)));
    builder.edit(ids.line_label, |e| set_size(e, FontSize::Pt(17.0)));
    builder.edit(ids.portal_source, |e| {
        if let DocElement::PortalEnd(end) = e {
            end.style.size = Some(45.0);
        }
    });
}

/// Set an element's own text size.
fn set_size(element: &mut DocElement, size: FontSize) {
    match element {
        DocElement::Node(n) => n.style.text.size = Some(size),
        DocElement::Section(s) => s.text.size = Some(size),
        DocElement::Span(s) => s.style.size = Some(size),
        DocElement::Frame(f) => f.style.text.size = Some(size),
        DocElement::Ring(r) => r.style.text.size = Some(size),
        DocElement::CapStart(c) | DocElement::CapEnd(c) => c.text.size = Some(size),
        DocElement::Label(l) => l.style.text.size = Some(size),
        _ => {}
    }
}

/// Clear an element's own text size.
fn clear_size(element: &mut DocElement) {
    let style = match element {
        DocElement::Node(n) => &mut n.style.text,
        DocElement::Section(s) => &mut s.text,
        DocElement::Span(s) => &mut s.style,
        DocElement::Frame(f) => &mut f.style.text,
        DocElement::Ring(r) => &mut r.style.text,
        DocElement::CapStart(c) | DocElement::CapEnd(c) => &mut c.text,
        DocElement::Label(l) => &mut l.style.text,
        _ => return,
    };
    style.size = None;
}

/// The four fixtures: nothing set; every default; owners set; and some
/// defaults with the owners set.
fn fixtures() -> Vec<(&'static str, DocumentBuilder, Example)> {
    let (plain, ids) = example();
    let (mut defaults, _) = example();
    defaults.defaults(every_default());
    let (mut owners, _) = example();
    set_owners(&mut owners, &ids);
    let (mut mixed, _) = example();
    mixed.defaults(some_defaults());
    set_owners(&mut mixed, &ids);
    vec![
        ("nothing set", plain, ids),
        ("every default", defaults, ids),
        ("owners set", owners, ids),
        ("some defaults, owners set", mixed, ids),
    ]
}

/// The size `resolve_text` gives `id` once its own size is cleared.
fn displayed_unsized(tree: &Tree<DocElement>, id: NodeId) -> FontSize {
    let mut cleared = tree.clone();
    cleared.update(id, clear_size).unwrap();
    resolve_text(&cleared, id, &mut ()).size
}

#[test]
fn test_document_seed_matches_the_resolver_for_every_kind() {
    do_document_seed_matches_the_resolver_for_every_kind();
}

/// For every text-bearing element of the example map, in every fixture,
/// the seed of a walk anchored there answers, for the element's kind,
/// the size `resolve_text` gives the element with its own size cleared.
pub fn do_document_seed_matches_the_resolver_for_every_kind() {
    let mut checked = [0usize; TEXT_KINDS.len()];
    for (name, builder, _) in fixtures() {
        let tree = builder.tree();
        for id in tree.descendants() {
            let kind = tree.arena()[id].get().kind();
            let Some(slot) = TEXT_KINDS.iter().position(|&k| k == kind) else {
                continue;
            };
            checked[slot] += 1;
            assert_eq!(
                inherited_seed(tree, id).text_size_for(kind),
                displayed_unsized(tree, id),
                "{name}: {kind:?} {id}"
            );
        }
    }
    assert!(
        checked.iter().all(|&n| n > 0),
        "every text-bearing kind must be checked: {checked:?}"
    );
}

/// A mutator that mirrors `tree`'s shape from its root, carrying `op`
/// on every mutator of `kind` and nothing elsewhere, so one walk from the
/// map reaches every element of `kind` through every level above it.
/// Children with equal keys share one mutator (the walk broadcasts it),
/// whose own children mirror theirs together.
fn mirror(tree: &Tree<DocElement>, kind: DocKind, op: &TextOp) -> DocMutatorTree {
    let mut mutator = DocMutatorTree::new();
    let root = mutator.root;
    mirror_into(tree, tree.root(), &mut mutator, root, kind, op);
    mutator
}

fn mirror_into(
    tree: &Tree<DocElement>,
    target: NodeId,
    mutator: &mut DocMutatorTree,
    at: NodeId,
    kind: DocKind,
    op: &TextOp,
) {
    for child in target.children(tree.arena()) {
        let key = tree.arena()[child].get().key();
        let existing = at
            .children(mutator.arena())
            .find(|&m| mutator.arena()[m].get().key() == key);
        let m = match existing {
            Some(m) => m,
            None => {
                let node = if key.kind == kind {
                    Mutator::new(text_payload(kind, op.clone()), key)
                } else {
                    Mutator::new_void(key)
                };
                mutator.insert_child(at, node, Placement::Last).unwrap()
            }
        };
        mirror_into(tree, child, mutator, m, kind, op);
    }
}

#[test]
fn test_document_walk_from_the_map_matches_the_resolver_for_every_kind() {
    do_document_walk_from_the_map_matches_the_resolver_for_every_kind();
}

/// The walker's own `inherit`, level by level from the map: for each
/// text-bearing kind, every element of that kind has its size cleared,
/// one walk from the map grows each by 0, and each stores the size
/// `resolve_text` gave it — so the state every level handed down was the
/// resolver's answer. Only that kind is changed by its walk, so the
/// levels above it hand down what they did before.
pub fn do_document_walk_from_the_map_matches_the_resolver_for_every_kind() {
    for (name, builder, _) in fixtures() {
        for kind in TEXT_KINDS {
            let mut tree = builder.tree().clone();
            let targets: Vec<NodeId> = tree
                .descendants()
                .filter(|&id| tree.arena()[id].get().kind() == kind)
                .collect();
            assert!(!targets.is_empty(), "{name}: the example holds a {kind:?}");
            for &id in &targets {
                tree.update(id, clear_size).unwrap();
            }
            let expected: Vec<FontSize> = targets
                .iter()
                .map(|&id| resolve_text(&tree, id, &mut ()).size)
                .collect();
            let mutator = mirror(&tree, kind, &TextOp::GrowSize(0.0));
            let seed = inherited_seed(&tree, tree.root());
            mutator.apply_to(&mut tree, &seed);
            for (&id, expected) in targets.iter().zip(&expected) {
                let stored = tree.arena()[id].get().text_style().and_then(|s| s.size);
                assert_eq!(stored, Some(*expected), "{name}: {kind:?} {id}");
            }
        }
    }
}

/// Run one step of `mutation` on `kind` at `anchor` of `doc`.
fn grow(doc: &mut Document, anchor: NodeId, kind: DocKind, op: TextOp) {
    let mutator = DocMutatorTree::new_with(Mutator::new(text_payload(kind, op), BranchKey::new(kind, 0)));
    let step = DocStep::fixed(DocWalk::new(mutator, Anchors::Nodes(vec![anchor])));
    let journal = doc.run_steps(vec![step], JournalBuilder::new()).unwrap();
    let _ = journal.close(doc.tree());
}

/// The text size the element at `id` of `doc` stores.
fn stored(doc: &Document, id: NodeId) -> Option<FontSize> {
    doc.get(id).and_then(DocElement::text_style).and_then(|s| s.size)
}

#[test]
fn test_document_grow_size_grows_the_inherited_size() {
    do_document_grow_size_grows_the_inherited_size();
}

/// The two cases #237 names. A span with no size under a section at
/// 18 pt grows by 2 to `Pt(20)`. Inside a frame, a ring with no size —
/// at the built-in `Rel(0.06)` — scaled by 2 is `Rel(0.12)`; the target
/// is the ring, since a span's own size inside a frame is never read
/// (#235 amendment 3).
pub fn do_document_grow_size_grows_the_inherited_size() {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let node = builder.node(root);
    let section = builder.section(node, 0);
    builder.edit(section, |e| set_size(e, FontSize::Pt(18.0)));
    let span = builder.span(section, "grow");
    let frame = builder.add(node, DocElement::Frame(Default::default()));
    let ring = builder.add(frame, DocElement::Ring(Default::default()));
    let mut doc = builder.build().unwrap();
    assert_eq!(stored(&doc, span), None, "the span sets no size of its own");
    assert_eq!(stored(&doc, ring), None, "the ring sets no size of its own");

    grow(&mut doc, span, DocKind::Span, TextOp::GrowSize(2.0));
    assert_eq!(stored(&doc, span), Some(FontSize::Pt(20.0)));

    grow(&mut doc, ring, DocKind::Ring, TextOp::ScaleSize(2.0));
    assert_eq!(stored(&doc, ring), Some(FontSize::Rel(0.12)));

    // Growing again starts from the size the element now sets.
    grow(&mut doc, span, DocKind::Span, TextOp::GrowSize(2.0));
    assert_eq!(stored(&doc, span), Some(FontSize::Pt(22.0)));
}

#[test]
fn test_document_grow_size_never_crosses_a_flow() {
    do_document_grow_size_never_crosses_a_flow();
}

/// The three cases the plain overlay got wrong (#237, comment
/// 6429648415), with the node's text at 30 pt: a child node with no
/// size grows from the 24 pt it displays, not from its parent's 30; an
/// unsized ring under `Defaults.frame` at `Rel(0.1)` scales from 0.1,
/// not from any point size; a connector span grows from the connector's
/// 12 pt, not from the node's text.
pub fn do_document_grow_size_never_crosses_a_flow() {
    let (mut builder, ids) = example();
    builder.edit(ids.a, |e| set_size(e, FontSize::Pt(30.0)));
    builder.edit(ids.b, |e| set_size(e, FontSize::Pt(30.0)));
    let mut defaults = DefaultsData::default();
    defaults.frame.text.size = Some(FontSize::Rel(0.1));
    builder.defaults(defaults);
    let b_connector = connector_of(&builder, ids.b);
    assert_eq!(b_connector, ids.b_connector);
    let mut doc = builder.build().unwrap();

    grow(&mut doc, ids.c, DocKind::Node, TextOp::GrowSize(2.0));
    assert_eq!(stored(&doc, ids.c), Some(FontSize::Pt(26.0)), "a child node");
    grow(&mut doc, ids.ring, DocKind::Ring, TextOp::ScaleSize(2.0));
    assert_eq!(stored(&doc, ids.ring), Some(FontSize::Rel(0.2)), "a ring");
    grow(
        &mut doc,
        ids.b_connector_span,
        DocKind::Span,
        TextOp::GrowSize(2.0),
    );
    assert_eq!(
        stored(&doc, ids.b_connector_span),
        Some(FontSize::Pt(14.0)),
        "a connector span"
    );
}
