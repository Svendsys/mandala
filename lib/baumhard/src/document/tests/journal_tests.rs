// SPDX-License-Identifier: MPL-2.0

//! The document under the engine's journals (amendment 1, V1-A3's
//! items): a journaled change of a document tree reverts and replays
//! exactly, and `write_interpolated` shows it part-way — numbers,
//! positions, sizes and literal colors blending, everything else
//! snapping, the key never interpolated.

use crate::document::element::{DocElement, NodeData};
use crate::document::tests::test_support::{example, Example};
use crate::document::values::{ColorRef, FontSize, Rgba, VarId};
use crate::engine::element::TreeElement;
use crate::engine::interpolate::{Interpolable, Snap};
use crate::engine::journal::{Journal, JournalBuilder};
use crate::engine::tree::Tree;
use glam::Vec2;
use indextree::NodeId;

/// The tree in pre-order: each element with its parent.
fn snapshot(tree: &Tree<DocElement>) -> Vec<(NodeId, Option<NodeId>, DocElement)> {
    tree.descendants()
        .map(|id| (id, tree.arena()[id].parent(), tree.arena()[id].get().clone()))
        .collect()
}

/// Change the node at `id` in `tree`, journaled.
fn node(
    tree: &mut Tree<DocElement>,
    journal: &mut JournalBuilder<DocElement>,
    id: NodeId,
    f: impl FnOnce(&mut NodeData),
) {
    tree.update_journaled(id, journal, |e| {
        if let DocElement::Node(n) = e {
            f(n)
        }
    })
    .unwrap();
}

/// The example with `b` black at 20 pt, and a journal of changes to it:
/// `b` moves and turns red at 10 pt, `b`'s span changes text, `a`'s
/// title section moves behind its body (a key change), and the theme
/// switches variant.
fn journaled() -> (Tree<DocElement>, Tree<DocElement>, Journal<DocElement>, Example) {
    let (mut builder, ids) = example();
    builder.edit_node(ids.b, |n| {
        n.style.fill = Some(ColorRef::Rgba(Rgba::opaque(0, 0, 0)));
        n.style.text.size = Some(FontSize::Pt(20.0));
    });
    let mut tree = builder.tree().clone();
    let original = tree.clone();
    let mut journal = JournalBuilder::new();
    node(&mut tree, &mut journal, ids.b, |n| {
        n.position = Vec2::new(100.0, 50.0);
        n.style.fill = Some(ColorRef::Rgba(Rgba::opaque(200, 0, 0)));
        n.style.text.size = Some(FontSize::Pt(10.0));
    });
    tree.update_journaled(ids.b_span, &mut journal, |e| {
        if let DocElement::Span(s) = e {
            s.text = "changed".to_string();
        }
    })
    .unwrap();
    tree.update_journaled(ids.a_title, &mut journal, |e| {
        if let DocElement::Section(s) = e {
            s.channel = 5;
        }
    })
    .unwrap();
    let theme = builder.theme_id();
    tree.update_journaled(theme, &mut journal, |e| {
        if let DocElement::Theme(t) = e {
            t.active = 1;
        }
    })
    .unwrap();
    let journal = journal.close(&tree);
    (original, tree, journal, ids)
}

#[test]
fn test_document_journal_reverts_and_replays_exactly() {
    do_document_journal_reverts_and_replays_exactly();
}

/// A journal over a document tree reverts to the original exactly —
/// every element's state and every row's order — and replays to the
/// changed tree exactly. Control: the changed tree differs from the
/// original, and the journal recorded each changed element.
pub fn do_document_journal_reverts_and_replays_exactly() {
    let (original, mut tree, journal, ids) = journaled();
    let changed = snapshot(&tree);
    assert_ne!(snapshot(&original), changed, "the journal must change something");
    for id in [ids.b, ids.b_span, ids.a_title] {
        assert!(journal.before(id).is_some(), "{id} is journaled");
    }
    journal.revert(&mut tree).unwrap();
    assert_eq!(snapshot(&tree), snapshot(&original));
    tree.check_sorted().unwrap();
    journal.replay(&mut tree).unwrap();
    assert_eq!(snapshot(&tree), changed);
    tree.check_sorted().unwrap();
}

#[test]
fn test_document_same_state_sees_every_field() {
    do_document_same_state_sees_every_field();
}

/// `same_state` is `==`: a change of any one field, a size or a
/// span's text included, is a change of state, so a journal never drops
/// it. Control: an unchanged clone is the same state.
pub fn do_document_same_state_sees_every_field() {
    let (builder, ids) = example();
    let node = builder.tree().arena()[ids.b].get().clone();
    assert!(node.same_state(&node.clone()));
    let mut moved = node.clone();
    if let DocElement::Node(n) = &mut moved {
        n.min_size = Some(Vec2::ONE);
    }
    assert!(!node.same_state(&moved));
    let span = builder.tree().arena()[ids.b_span].get().clone();
    let mut retexted = span.clone();
    if let DocElement::Span(s) = &mut retexted {
        s.text.push('!');
    }
    assert!(!span.same_state(&retexted));
}

#[test]
fn test_document_write_interpolated() {
    do_document_write_interpolated();
}

/// `write_interpolated` at 0 is the original and at 1 the changed tree,
/// exactly, under either snap. At 0.5 the position, the literal fill and
/// the point size are halfway; the span's text, the theme's active
/// variant and the section's key snap — before under `Snap::End`, after
/// under `Snap::Start` — and every row stays sorted.
pub fn do_document_write_interpolated() {
    let (original, mut tree, journal, ids) = journaled();
    let changed = snapshot(&tree);
    for snap in [Snap::End, Snap::Start] {
        journal.write_interpolated(&mut tree, 0.0, snap).unwrap();
        assert_eq!(snapshot(&tree), snapshot(&original), "{snap:?} at 0");
        journal.write_interpolated(&mut tree, 1.0, snap).unwrap();
        assert_eq!(snapshot(&tree), changed, "{snap:?} at 1");

        journal.write_interpolated(&mut tree, 0.5, snap).unwrap();
        tree.check_sorted().unwrap();
        let DocElement::Node(b) = tree.arena()[ids.b].get() else { panic!("b is a node") };
        assert_eq!(b.position, Vec2::new(50.0, 25.0));
        assert_eq!(b.style.fill, Some(ColorRef::Rgba(Rgba::opaque(100, 0, 0))));
        assert_eq!(b.style.text.size, Some(FontSize::Pt(15.0)));
        let theme = tree.root().children(tree.arena()).next().unwrap();
        let DocElement::Theme(theme) = tree.arena()[theme].get() else { panic!("the theme") };
        assert_eq!(theme.active, if snap == Snap::Start { 1 } else { 0 });
        let DocElement::Span(span) = tree.arena()[ids.b_span].get() else { panic!("a span") };
        assert_eq!(span.text, if snap == Snap::Start { "changed" } else { "child" });
        let title_channel = tree.arena()[ids.a_title].get().channel();
        assert_eq!(title_channel, if snap == Snap::Start { 5 } else { 0 });
    }
}

#[test]
fn test_document_interpolate_blends_numbers_and_colors() {
    do_document_interpolate_blends_numbers_and_colors();
}

/// Between two set values of the same form, a number, a size of one
/// unit and a literal color blend; across forms — a literal and a theme
/// variable, points and a fraction — the field snaps. Controls: at 0
/// and 1 the endpoints are exact.
pub fn do_document_interpolate_blends_numbers_and_colors() {
    let (builder, ids) = example();
    let before = builder.tree().arena()[ids.b].get().clone();
    let with = |fill: ColorRef, size: FontSize, opacity: f32| {
        let mut e = before.clone();
        if let DocElement::Node(n) = &mut e {
            n.style.fill = Some(fill);
            n.style.text.size = Some(size);
            n.opacity = opacity;
        }
        e
    };
    let from = with(ColorRef::Rgba(Rgba::opaque(0, 0, 0)), FontSize::Pt(10.0), 0.0);
    let to = with(
        ColorRef::Rgba(Rgba::opaque(200, 100, 50)),
        FontSize::Pt(20.0),
        1.0,
    );
    let half = DocElement::interpolate(&from, &to, 0.5, Snap::End);
    let DocElement::Node(n) = &half else { panic!("a node") };
    assert_eq!(n.style.fill, Some(ColorRef::Rgba(Rgba::opaque(100, 50, 25))));
    assert_eq!(n.style.text.size, Some(FontSize::Pt(15.0)));
    assert_eq!(n.opacity, 0.5);
    assert_eq!(DocElement::interpolate(&from, &to, 0.0, Snap::Start), from);
    assert_eq!(DocElement::interpolate(&from, &to, 1.0, Snap::End), to);

    let var = with(ColorRef::Var(VarId(0)), FontSize::Rel(0.1), 1.0);
    let DocElement::Node(n) = DocElement::interpolate(&from, &var, 0.5, Snap::End) else {
        panic!("a node")
    };
    assert_eq!(
        n.style.fill,
        Some(ColorRef::Rgba(Rgba::opaque(0, 0, 0))),
        "a variable snaps"
    );
    assert_eq!(
        n.style.text.size,
        Some(FontSize::Pt(10.0)),
        "across units it snaps"
    );
    assert_eq!(n.opacity, 0.5, "the number still blends");
}
