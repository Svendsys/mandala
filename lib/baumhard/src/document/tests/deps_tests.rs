// SPDX-License-Identifier: MPL-2.0

//! The input contract: which inputs a change of an element touches
//! ([`inputs_changed`]), and that it agrees with what the resolvers
//! record — a change that alters a resolution touches an input that
//! resolution recorded.

use crate::document::deps::{inputs_changed, InputKind};
use crate::document::element::{DocElement, DocKind};
use crate::document::resolve::{
    resolve_frame, resolve_label, resolve_node, resolve_path, resolve_portal_end, resolve_ring, resolve_text,
    Input,
};
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
    // Placement, split from looks field by field.
    let label = |f: fn(&mut crate::document::element::LabelStyle)| {
        move |e: &mut DocElement| {
            if let DocElement::Label(l) = e {
                f(&mut l.style)
            }
        }
    };
    assert_eq!(
        touched(&mut builder, ids.b_label, label(|s| s.t = Some(0.2))),
        vec![InputKind::Placement]
    );
    assert_eq!(
        touched(&mut builder, ids.b_label, label(|s| s.offset = Some(3.0))),
        vec![InputKind::Placement]
    );
    assert_eq!(
        touched(&mut builder, ids.b_label, label(|s| s.text.italic = Some(true))),
        vec![InputKind::Style]
    );
    let end = |f: fn(&mut crate::document::element::PortalEndStyle)| {
        move |e: &mut DocElement| {
            if let DocElement::PortalEnd(end) = e {
                f(&mut end.style)
            }
        }
    };
    assert_eq!(
        touched(&mut builder, ids.portal_source, end(|s| s.border_t = Some(0.3))),
        vec![InputKind::Placement]
    );
    assert_eq!(
        touched(&mut builder, ids.portal_source, end(|s| s.offset = Some(1.0))),
        vec![InputKind::Placement]
    );
    assert_eq!(
        touched(&mut builder, ids.portal_source, end(|s| s.size = Some(40.0))),
        vec![InputKind::Style]
    );
    assert_eq!(
        touched(
            &mut builder,
            ids.portal_source,
            end(|s| s.content = Some("*".to_string()))
        ),
        vec![InputKind::Style]
    );
    let frame = |f: fn(&mut crate::document::element::FrameStyle)| {
        move |e: &mut DocElement| {
            if let DocElement::Frame(fr) = e {
                f(&mut fr.style)
            }
        }
    };
    assert_eq!(
        touched(&mut builder, ids.frame, frame(|s| s.gap = Some(2.0))),
        vec![InputKind::Placement]
    );
    assert_eq!(
        touched(
            &mut builder,
            ids.frame,
            frame(|s| s.direction = Some(crate::document::values::Direction::CounterClockwise))
        ),
        vec![InputKind::Placement]
    );
    assert_eq!(
        touched(&mut builder, ids.frame, frame(|s| s.visible = Some(false))),
        vec![InputKind::Style]
    );
    assert_eq!(
        touched(
            &mut builder,
            ids.frame,
            frame(|s| s.fit = Some(crate::document::values::FitMode::Repeat))
        ),
        vec![InputKind::Style]
    );

    let all: Vec<InputKind> = vec![
        InputKind::Style,
        InputKind::Placement,
        InputKind::PaletteBinding,
        InputKind::ParentRow,
    ];
    let before = builder.tree().arena()[ids.b_label].get().clone();
    assert_eq!(
        inputs_changed(&before, &crate::document::tests::test_support::span("x")).collect::<Vec<_>>(),
        all
    );
}

/// One resolution of one element: its id, which resolver, what it
/// resolved to (as text, so every resolver's result compares alike), and
/// what it recorded.
type Resolution = (NodeId, &'static str, String, Vec<Input>);

/// Every resolution of every element of `tree` that resolves: text for
/// every element below the map's own children, and each element's own
/// resolver — node, frame, ring, path, label, portal end.
fn resolve_all(tree: &Tree<DocElement>) -> Vec<Resolution> {
    let mut out = Vec::new();
    for id in tree.descendants() {
        let kind = tree.arena()[id].get().kind();
        if matches!(
            kind,
            DocKind::Map | DocKind::Theme | DocKind::Palette | DocKind::Defaults
        ) {
            continue;
        }
        let mut reads = Vec::new();
        let text = format!("{:?}", resolve_text(tree, id, &mut reads));
        out.push((id, "text", text, reads));
        let mut reads = Vec::new();
        let own = match kind {
            DocKind::Node => Some(("node", format!("{:?}", resolve_node(tree, id, &mut reads)))),
            DocKind::Frame => Some(("frame", format!("{:?}", resolve_frame(tree, id, &mut reads)))),
            DocKind::Ring => Some(("ring", format!("{:?}", resolve_ring(tree, id, &mut reads)))),
            DocKind::Connector | DocKind::Link => {
                Some(("path", format!("{:?}", resolve_path(tree, id, &mut reads))))
            }
            DocKind::Label => Some(("label", format!("{:?}", resolve_label(tree, id, &mut reads)))),
            DocKind::PortalEnd => Some((
                "portal end",
                format!("{:?}", resolve_portal_end(tree, id, &mut reads)),
            )),
            _ => None,
        };
        if let Some((resolver, result)) = own {
            out.push((id, resolver, result, reads));
        }
    }
    out
}

/// The inputs a change of the element at `id` touches, as `Input`s.
fn inputs_of(tree: &Tree<DocElement>, id: NodeId, kinds: &[InputKind]) -> Vec<Input> {
    kinds
        .iter()
        .filter_map(|kind| match kind {
            InputKind::Style => Some(Input::Style(id)),
            InputKind::Placement => Some(Input::Placement(id)),
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

/// One single-field edit: its name, whether it only moves something
/// (geometry or placement), and the edit, which returns the element it
/// changed.
type Edit = (&'static str, bool, fn(&mut DocumentBuilder, &Example) -> NodeId);

/// Change the element at `id` of `b` with `f`, and return `id`.
fn change(b: &mut DocumentBuilder, id: NodeId, f: impl FnOnce(&mut DocElement)) -> NodeId {
    b.edit(id, f);
    id
}

/// The edits [`do_document_records_and_changes_agree`] runs.
fn edits() -> Vec<Edit> {
    vec![
        ("node position", true, |b, ids| {
            b.edit_node(ids.a, |n| n.position = Vec2::new(500.0, 500.0));
            ids.a
        }),
        ("label t", true, |b, ids| {
            change(b, ids.b_label, |e| {
                if let DocElement::Label(l) = e {
                    l.style.t = Some(0.25);
                }
            })
        }),
        ("portal border_t", true, |b, ids| {
            change(b, ids.portal_source, |e| {
                if let DocElement::PortalEnd(end) = e {
                    end.style.border_t = Some(0.3);
                }
            })
        }),
        ("frame gap", true, |b, ids| {
            change(b, ids.frame, |e| {
                if let DocElement::Frame(f) = e {
                    f.style.gap = Some(2.0);
                }
            })
        }),
        ("node text size", false, |b, ids| {
            b.edit_node(ids.b, |n| n.style.text.size = Some(FontSize::Pt(31.0)));
            ids.b
        }),
        ("palette binding removed", false, |b, ids| {
            b.edit_node(ids.a, |n| n.palette = None);
            ids.a
        }),
        ("section channel", false, |b, ids| {
            change(b, ids.a_title, |e| {
                if let DocElement::Section(s) = e {
                    s.channel = 9;
                }
            })
        }),
        ("theme variant", false, |b, _| {
            let theme = b.theme_id();
            change(b, theme, |e| {
                if let DocElement::Theme(t) = e {
                    t.variants[0].values[0] = Rgba::opaque(1, 1, 1);
                }
            })
        }),
        ("defaults", false, |b, _| {
            let defaults = b.defaults_id();
            change(b, defaults, |e| {
                if let DocElement::Defaults(d) = e {
                    d.connector.size = Some(17.0);
                }
            })
        }),
        ("palette group", false, |b, _| {
            let palette = b.root().children(b.tree().arena()).nth(1).unwrap();
            change(b, palette, |e| {
                if let DocElement::Palette(p) = e {
                    p.groups[1].fill = crate::document::values::PlainColor::Rgba(Rgba::opaque(9, 9, 9));
                }
            })
        }),
        ("connector color, reaching its cap", false, |b, ids| {
            change(b, ids.b_connector, |e| {
                if let DocElement::Connector(c) = e {
                    c.path.flow.color = Some(ColorRef::Rgba(Rgba::opaque(5, 5, 5)));
                }
            })
        }),
        ("connector fit", false, |b, ids| {
            change(b, ids.b_connector, |e| {
                if let DocElement::Connector(c) = e {
                    c.path.flow.fit = Some(crate::document::values::FitMode::Spread);
                }
            })
        }),
        ("ring size", false, |b, ids| {
            change(b, ids.ring, |e| {
                if let DocElement::Ring(r) = e {
                    r.style.text.size = Some(FontSize::Rel(0.2));
                }
            })
        }),
    ]
}

#[test]
fn test_document_records_and_changes_agree() {
    do_document_records_and_changes_agree();
}

/// The two halves of the contract agree, over every resolver — text,
/// node, frame, ring, path, label, portal end — and a set of
/// single-field edits:
///
/// - every resolution an edit changes recorded an input the edit
///   touched (no stale projection);
/// - an edit that only moves something — a node's position, a label's
///   `t`, a portal end's `border_t`, a frame's `gap` — touches no input
///   any **text** resolution recorded (no reshaping on a drag).
///
/// Controls: a node's position touches no input at all and changes no
/// resolution; every other edit changes at least one resolution, so
/// neither property is checked vacuously; and the cap under the
/// recolored connector is among the resolutions that change.
pub fn do_document_records_and_changes_agree() {
    for (what, moves_only, edit) in edits() {
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
        let mut changed = Vec::new();
        for (element, resolver, was, reads) in &before {
            // By id: an edit that re-keys an element reorders the walk.
            let now = &after
                .iter()
                .find(|(id, r, _, _)| id == element && r == resolver)
                .unwrap()
                .2;
            if was != now {
                changed.push((*element, *resolver));
                assert!(
                    reads.iter().any(|r| changed_inputs.contains(r)),
                    "{what}: the {resolver} resolution of a {:?} changed, but it recorded none of {changed_inputs:?}",
                    builder.tree().arena()[*element].get().kind()
                );
            }
            if moves_only && *resolver == "text" {
                assert!(
                    !reads.iter().any(|r| changed_inputs.contains(r)),
                    "{what}: a move touched {changed_inputs:?}, which the text of a {:?} reads",
                    builder.tree().arena()[*element].get().kind()
                );
            }
        }
        if what == "node position" {
            assert_eq!(
                (changed.len(), kinds.len()),
                (0, 0),
                "geometry changes no resolution and no input"
            );
        } else {
            assert!(
                !changed.is_empty(),
                "{what} must change some resolution, or this proves nothing"
            );
        }
        if what == "connector color, reaching its cap" {
            assert!(
                changed.contains(&(ids.b_cap, "text")),
                "the cap takes its connector's color"
            );
        }
    }
}
