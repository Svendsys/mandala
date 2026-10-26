// SPDX-License-Identifier: MPL-2.0

//! The input contract: which inputs a change of an element touches
//! ([`inputs_changed`]), and that it agrees with what the resolvers
//! record — a change that alters a resolution touches an input that
//! resolution recorded.

use crate::document::deps::{inputs_changed, InputKind};
use crate::document::element::{DocElement, DocKind};
use crate::document::resolve::{resolve_node, resolve_text, Input, ResolvedNode, ResolvedText};
use crate::document::tests::test_support::{example, DocumentBuilder, Example};
use crate::document::values::{ColorRef, FontSize, PaletteId, Rgba};
use crate::engine::tree::Tree;
use glam::Vec2;
use indextree::NodeId;

/// The kinds `edit` touches on the element at `id` of `builder`.
fn touched(builder: &mut DocumentBuilder, id: NodeId, edit: impl FnOnce(&mut DocElement)) -> Vec<InputKind> {
    let before = builder.tree().arena()[id].get().clone();
    builder.edit(id, edit);
    let after = builder.tree().arena()[id].get();
    inputs_changed(&before, after).collect()
}

/// Change the node at `id` with `f`.
fn on_node(f: impl FnOnce(&mut crate::document::element::NodeData)) -> impl FnOnce(&mut DocElement) {
    move |e| {
        if let DocElement::Node(n) = e {
            f(n)
        }
    }
}

#[test]
fn test_document_geometry_touches_no_input() {
    do_document_geometry_touches_no_input();
}

/// A node's position, minimum size, zoom window, folding, visibility
/// and opacity are read by no resolver, so changing them touches no
/// input — a drag dirties no resolution. Controls: its text size touches
/// `Style`, its palette `PaletteBinding`, its channel its parent's row.
pub fn do_document_geometry_touches_no_input() {
    let (mut builder, ids) = example();
    let none: Vec<InputKind> = Vec::new();
    assert_eq!(
        touched(
            &mut builder,
            ids.b,
            on_node(|n| n.position = Vec2::new(300.0, 40.0))
        ),
        none
    );
    assert_eq!(
        touched(
            &mut builder,
            ids.b,
            on_node(|n| n.min_size = Some(Vec2::new(200.0, 80.0)))
        ),
        none
    );
    assert_eq!(
        touched(
            &mut builder,
            ids.b,
            on_node(|n| {
                n.folded = true;
                n.hidden = true;
                n.opacity = 0.5;
            })
        ),
        none
    );
    assert_eq!(
        touched(
            &mut builder,
            ids.b,
            on_node(|n| n.style.text.size = Some(FontSize::Pt(30.0)))
        ),
        vec![InputKind::Style]
    );
    assert_eq!(
        touched(&mut builder, ids.b, on_node(|n| n.palette = Some(PaletteId(0)))),
        vec![InputKind::PaletteBinding]
    );
    assert_eq!(
        touched(&mut builder, ids.b, on_node(|n| n.channel = 2)),
        vec![InputKind::ParentRow]
    );
}

#[test]
fn test_document_inputs_changed_by_kind() {
    do_document_inputs_changed_by_kind();
}

/// Per kind, a style field touches `Style` and geometry, text and links
/// touch nothing; a key change touches the parent's row; the theme, a
/// palette and the defaults touch their own input; a change of kind
/// touches everything an element can be read for.
pub fn do_document_inputs_changed_by_kind() {
    let (mut builder, ids) = example();
    let none: Vec<InputKind> = Vec::new();
    let path = |f: fn(&mut crate::document::element::PathData)| {
        move |e: &mut DocElement| {
            if let DocElement::Connector(c) = e {
                f(&mut c.path)
            }
        }
    };
    assert_eq!(
        touched(
            &mut builder,
            ids.b_connector,
            path(|p| p.control_points.push(Vec2::ONE))
        ),
        none
    );
    assert_eq!(
        touched(
            &mut builder,
            ids.b_connector,
            path(|p| p.flow.color = Some(ColorRef::Rgba(Rgba::opaque(1, 2, 3))))
        ),
        vec![InputKind::Style]
    );
    let span = |f: fn(&mut crate::document::element::SpanData)| {
        move |e: &mut DocElement| {
            if let DocElement::Span(s) = e {
                f(s)
            }
        }
    };
    assert_eq!(
        touched(&mut builder, ids.b_span, span(|s| s.text.push('!'))),
        none
    );
    assert_eq!(
        touched(&mut builder, ids.b_span, span(|s| s.style.italic = Some(true))),
        vec![InputKind::Style]
    );
    assert_eq!(
        touched(&mut builder, ids.b_section, |e| {
            if let DocElement::Section(s) = e {
                s.offset = Vec2::new(4.0, 4.0);
            }
        }),
        none
    );
    assert_eq!(
        touched(&mut builder, ids.portal_target, |e| {
            if let DocElement::PortalEnd(end) = e {
                end.end = 0;
            }
        }),
        vec![InputKind::ParentRow]
    );
    let theme = builder.theme_id();
    assert_eq!(
        touched(&mut builder, theme, |e| {
            if let DocElement::Theme(t) = e {
                t.active = 1;
            }
        }),
        vec![InputKind::Theme]
    );
    let defaults = builder.defaults_id();
    assert_eq!(
        touched(&mut builder, defaults, |e| {
            if let DocElement::Defaults(d) = e {
                d.span.italic = Some(true);
            }
        }),
        vec![InputKind::Defaults]
    );
    assert_eq!(
        touched(&mut builder, defaults, |_| {}),
        none,
        "no change, no input"
    );
    let all: Vec<InputKind> = vec![InputKind::Style, InputKind::PaletteBinding, InputKind::ParentRow];
    let before = builder.tree().arena()[ids.b_label].get().clone();
    assert_eq!(
        inputs_changed(&before, &crate::document::tests::test_support::span("x")).collect::<Vec<_>>(),
        all
    );
}

/// One element's resolution: its id, what it resolved to (its text,
/// and a node's own fields), and what it recorded.
type Resolution = (NodeId, (ResolvedText, Option<ResolvedNode>), Vec<Input>);

/// Every element of `tree` that resolves, with its resolution and what
/// it recorded: text for spans and their holders, the node's own fields
/// for nodes.
fn resolve_all(tree: &Tree<DocElement>) -> Vec<Resolution> {
    tree.descendants()
        .filter(|&id| {
            !matches!(
                tree.arena()[id].get().kind(),
                DocKind::Map | DocKind::Theme | DocKind::Palette | DocKind::Defaults
            )
        })
        .map(|id| {
            let mut reads = Vec::new();
            let text = resolve_text(tree, id, &mut reads);
            let node =
                (tree.arena()[id].get().kind() == DocKind::Node).then(|| resolve_node(tree, id, &mut reads));
            (id, (text, node), reads)
        })
        .collect()
}

/// The inputs a change of the element at `id` touches, as `Input`s.
fn inputs_of(tree: &Tree<DocElement>, id: NodeId, kinds: &[InputKind]) -> Vec<Input> {
    kinds
        .iter()
        .filter_map(|kind| match kind {
            InputKind::Style => Some(Input::Style(id)),
            InputKind::PaletteBinding => Some(Input::PaletteBinding(id)),
            InputKind::ParentRow => tree.arena()[id].parent().map(Input::Row),
            InputKind::Theme => Some(Input::Theme),
            InputKind::Defaults => Some(Input::Defaults),
            InputKind::Palette => {
                let n = tree
                    .root()
                    .children(tree.arena())
                    .filter(|&c| tree.arena()[c].get().kind() == DocKind::Palette)
                    .position(|c| c == id)?;
                Some(Input::Palette(PaletteId(u16::try_from(n).ok()?)))
            }
        })
        .collect()
}

#[test]
fn test_document_records_and_changes_agree() {
    do_document_records_and_changes_agree();
}

/// The two halves of the contract agree: for a set of single-field
/// edits over the example, every resolution the edit changes recorded
/// an input the edit touched. Controls: the geometry edit changes no
/// resolution and touches no input, and each other edit does change at
/// least one resolution, so the property is never checked vacuously.
pub fn do_document_records_and_changes_agree() {
    type Edit = fn(&mut DocumentBuilder, &Example) -> NodeId;
    let edits: [(&str, Edit); 9] = [
        ("node position", |b, ids| {
            b.edit_node(ids.a, |n| n.position = Vec2::new(500.0, 500.0));
            ids.a
        }),
        ("node text size", |b, ids| {
            b.edit_node(ids.b, |n| n.style.text.size = Some(FontSize::Pt(31.0)));
            ids.b
        }),
        ("palette binding removed", |b, ids| {
            b.edit_node(ids.a, |n| n.palette = None);
            ids.a
        }),
        ("section channel", |b, ids| {
            b.edit(ids.a_title, |e| {
                if let DocElement::Section(s) = e {
                    s.channel = 9;
                }
            });
            ids.a_title
        }),
        ("theme variant", |b, _| {
            let theme = b.theme_id();
            b.edit(theme, |e| {
                if let DocElement::Theme(t) = e {
                    t.variants[0].values[0] = Rgba::opaque(1, 1, 1);
                }
            });
            theme
        }),
        ("defaults", |b, _| {
            let defaults = b.defaults_id();
            b.edit(defaults, |e| {
                if let DocElement::Defaults(d) = e {
                    d.connector.size = Some(17.0);
                }
            });
            defaults
        }),
        ("palette group", |b, _| {
            let palette = b.root().children(b.tree().arena()).nth(1).unwrap();
            b.edit(palette, |e| {
                if let DocElement::Palette(p) = e {
                    p.groups[1].fill = crate::document::values::PlainColor::Rgba(Rgba::opaque(9, 9, 9));
                }
            });
            palette
        }),
        ("connector color", |b, ids| {
            b.edit(ids.b_connector, |e| {
                if let DocElement::Connector(c) = e {
                    c.path.flow.color = Some(ColorRef::Rgba(Rgba::opaque(5, 5, 5)));
                }
            });
            ids.b_connector
        }),
        ("ring size", |b, ids| {
            b.edit(ids.ring, |e| {
                if let DocElement::Ring(r) = e {
                    r.style.text.size = Some(FontSize::Rel(0.2));
                }
            });
            ids.ring
        }),
    ];
    for (what, edit) in edits {
        let (mut builder, ids) = example();
        // One theme reader, so a theme edit has a resolution to change.
        builder.edit(ids.b_span, |e| {
            if let DocElement::Span(s) = e {
                s.style.color = Some(ColorRef::Var(crate::document::values::VarId(0)));
            }
        });
        let before = resolve_all(builder.tree());
        let snapshot = builder.tree().clone();
        let id = edit(&mut builder, &ids);
        let kinds: Vec<InputKind> =
            inputs_changed(snapshot.arena()[id].get(), builder.tree().arena()[id].get()).collect();
        let changed_inputs = inputs_of(builder.tree(), id, &kinds);
        let after = resolve_all(builder.tree());
        let mut changed = 0;
        for (element, was, reads) in &before {
            // By id: an edit that re-keys an element reorders the walk.
            let now = &after.iter().find(|(id, _, _)| id == element).unwrap().1;
            if was != now {
                changed += 1;
                assert!(
                    reads.iter().any(|r| changed_inputs.contains(r)),
                    "{what}: {:?}'s resolution changed, but it recorded none of {changed_inputs:?}",
                    builder.tree().arena()[*element].get().kind()
                );
            }
        }
        if what == "node position" {
            assert_eq!(
                (changed, kinds.len()),
                (0, 0),
                "geometry changes no resolution and no input"
            );
        } else {
            assert!(
                changed > 0,
                "{what} must change some resolution, or this proves nothing"
            );
        }
    }
}
