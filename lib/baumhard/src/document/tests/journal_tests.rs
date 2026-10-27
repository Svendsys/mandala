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

#[test]
fn test_document_interpolation_follows_the_listed_fields() {
    do_document_interpolation_follows_the_listed_fields();
}

/// #237's list, field by field: each listed field is at its midpoint
/// halfway, and each field outside it — a flag, a shape's radius, a fit
/// range, a zoom window, a frame's start point, unequal control points —
/// holds the before value under `Snap::End` and the after value under
/// `Snap::Start`. The four that B1 blended and the list leaves out
/// (radius, fit range, zoom window, start point) are among the snapped
/// (ruling R10 on #237).
pub fn do_document_interpolation_follows_the_listed_fields() {
    use crate::document::element::{
        ConnectorData, FrameData, FrameStyle, LabelData, LabelStyle, PaletteData, PathData, PortalEndData,
        PortalEndStyle, SectionData, ThemeData, ThemeVariant,
    };
    use crate::document::tests::test_support::{palette_group, path_data};
    use crate::document::values::{Edge, FitMode, Percent, PlainColor, Shape, StartPoint, ZoomWindow};

    /// `before` and `after` at 0.5 under both snaps.
    fn halves(before: &DocElement, after: &DocElement) -> (DocElement, DocElement) {
        (
            DocElement::interpolate(before, after, 0.5, Snap::End),
            DocElement::interpolate(before, after, 0.5, Snap::Start),
        )
    }

    let (builder, ids) = example();
    let DocElement::Node(base) = builder.tree().arena()[ids.b].get().clone() else {
        panic!("b is a node")
    };
    let node = |f: &dyn Fn(&mut NodeData)| {
        let mut n = base.clone();
        f(&mut n);
        DocElement::Node(n)
    };
    let before = node(&|n| {
        n.position = Vec2::new(0.0, 0.0);
        n.min_size = Some(Vec2::new(10.0, 10.0));
        n.style.fill_fade = Some(0.0);
        n.opacity = 0.0;
        n.hidden = false;
        n.style.shape = Some(Shape::Rectangle {
            radius: Percent::new(0),
        });
        n.zoom = Some(ZoomWindow {
            min: Some(1.0),
            max: None,
        });
    });
    let after = node(&|n| {
        n.position = Vec2::new(10.0, -20.0);
        n.min_size = Some(Vec2::new(30.0, 50.0));
        n.style.fill_fade = Some(4.0);
        n.opacity = 1.0;
        n.hidden = true;
        n.style.shape = Some(Shape::Rectangle {
            radius: Percent::new(100),
        });
        n.zoom = Some(ZoomWindow {
            min: Some(3.0),
            max: None,
        });
    });
    let (end, start) = halves(&before, &after);
    let (DocElement::Node(end), DocElement::Node(start)) = (end, start) else { panic!("nodes") };
    assert_eq!(end.position, Vec2::new(5.0, -10.0));
    assert_eq!(end.min_size, Some(Vec2::new(20.0, 30.0)));
    assert_eq!(end.style.fill_fade, Some(2.0));
    assert_eq!(end.opacity, 0.5);
    assert!(!end.hidden && start.hidden, "hidden snaps");
    assert_eq!(
        end.style.shape,
        Some(Shape::Rectangle {
            radius: Percent::new(0)
        }),
        "a radius snaps"
    );
    assert_eq!(
        start.style.shape,
        Some(Shape::Rectangle {
            radius: Percent::new(100)
        })
    );
    assert_eq!(end.zoom.and_then(|z| z.min), Some(1.0), "a zoom window snaps");
    assert_eq!(start.zoom.and_then(|z| z.min), Some(3.0));

    let section = |offset: Vec2| {
        DocElement::Section(SectionData {
            offset,
            ..SectionData::default()
        })
    };
    let (end, _) = halves(&section(Vec2::ZERO), &section(Vec2::new(8.0, 2.0)));
    assert!(matches!(end, DocElement::Section(s) if s.offset == Vec2::new(4.0, 1.0)));

    let frame = |gap: f32, at: f32, max: f32| {
        DocElement::Frame(FrameData {
            style: FrameStyle {
                gap: Some(gap),
                inset: Some(gap * 2.0),
                start: Some(StartPoint { edge: Edge::Top, at }),
                fit: Some(FitMode::Fit { min: 0.5, max }),
                ..FrameStyle::default()
            },
        })
    };
    let (end, start) = halves(&frame(0.0, 0.0, 1.0), &frame(2.0, 1.0, 3.0));
    let (DocElement::Frame(end), DocElement::Frame(start)) = (end, start) else { panic!("frames") };
    assert_eq!((end.style.gap, end.style.inset), (Some(1.0), Some(2.0)));
    assert_eq!(end.style.start.map(|s| s.at), Some(0.0), "a start point snaps");
    assert_eq!(start.style.start.map(|s| s.at), Some(1.0));
    assert_eq!(
        end.style.fit,
        Some(FitMode::Fit { min: 0.5, max: 1.0 }),
        "a fit range snaps"
    );
    assert_eq!(start.style.fit, Some(FitMode::Fit { min: 0.5, max: 3.0 }));

    let connector = |spacing: f32, size: f32, points: Vec<Vec2>| {
        let mut path: PathData = path_data();
        path.flow.spacing = Some(spacing);
        path.flow.size = Some(size);
        path.control_points = points;
        DocElement::Connector(ConnectorData { path })
    };
    let (end, _) = halves(
        &connector(0.0, 10.0, vec![Vec2::ZERO]),
        &connector(2.0, 20.0, vec![Vec2::new(4.0, 4.0)]),
    );
    let DocElement::Connector(end) = end else { panic!("a connector") };
    assert_eq!(
        (end.path.flow.spacing, end.path.flow.size),
        (Some(1.0), Some(15.0))
    );
    assert_eq!(end.path.control_points, vec![Vec2::new(2.0, 2.0)]);
    let (end, _) = halves(
        &connector(0.0, 10.0, vec![Vec2::ZERO]),
        &connector(0.0, 10.0, vec![Vec2::ONE, Vec2::ONE]),
    );
    let DocElement::Connector(end) = end else { panic!("a connector") };
    assert_eq!(end.path.control_points, vec![Vec2::ZERO], "unequal counts snap");

    let label = |t: f32| {
        DocElement::Label(LabelData {
            style: LabelStyle {
                t: Some(t),
                offset: Some(t * 10.0),
                ..LabelStyle::default()
            },
        })
    };
    let (end, _) = halves(&label(0.0), &label(1.0));
    assert!(matches!(end, DocElement::Label(l) if l.style.t == Some(0.5) && l.style.offset == Some(5.0)));

    let portal = |t: f32| {
        DocElement::PortalEnd(PortalEndData {
            end: 0,
            style: PortalEndStyle {
                border_t: Some(t),
                offset: Some(t),
                size: Some(10.0 + t * 10.0),
                ..PortalEndStyle::default()
            },
        })
    };
    let (end, _) = halves(&portal(0.0), &portal(1.0));
    let DocElement::PortalEnd(end) = end else { panic!("a portal end") };
    assert_eq!(
        (end.style.border_t, end.style.offset, end.style.size),
        (Some(0.5), Some(0.5), Some(15.0))
    );

    let theme = |v: u8| {
        DocElement::Theme(ThemeData {
            variables: vec!["a".to_string()],
            variants: vec![ThemeVariant {
                name: "v".to_string(),
                values: vec![Rgba::opaque(v, v, v)],
            }],
            active: 0,
        })
    };
    let (end, _) = halves(&theme(0), &theme(100));
    assert!(matches!(end, DocElement::Theme(t) if t.variants[0].values[0] == Rgba::opaque(50, 50, 50)));

    let palette = |n: u8| {
        let mut group = palette_group(n);
        group.title = Some(PlainColor::Rgba(Rgba::opaque(n, n, n)));
        DocElement::Palette(PaletteData {
            name: "p".to_string(),
            groups: vec![group],
        })
    };
    let (end, _) = halves(&palette(0), &palette(100));
    let DocElement::Palette(end) = end else { panic!("a palette") };
    assert_eq!(
        end.groups[0].title,
        Some(PlainColor::Rgba(Rgba::opaque(50, 50, 50))),
        "a palette group's colors blend"
    );
}
