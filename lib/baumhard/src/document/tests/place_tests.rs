// SPDX-License-Identifier: MPL-2.0

//! The rules that depend on where a value sits (`document::place`,
//! R14.16), and that both judges of a document hold to them: the file
//! format's `check_savable`, which decides what a map may be saved as,
//! and the mutation verifier's `validate`, which decides what an edit
//! may make. Each test pins the rule itself with literals, then shows
//! the two judges giving the same answer case by case, so that one of
//! them drifting to a copy of its own goes red here.

use crate::document::bounds::{BoundError, MAX_CHANNEL};
use crate::document::element::{
    DocElement, DocKind, FrameData, FrameStyle, RingData, RingStyle, SectionData, SpanData, TextStyle,
};
use crate::document::format::{check_savable, LoadErrorKind, SaveErrorKind};
use crate::document::ids::Document;
use crate::document::mutation::error::MutationError;
use crate::document::mutation::ops::{
    DocMutation, DocMutatorTree, FrameOp, NodeOp, RingOp, SectionOp, TextOp,
};
use crate::document::mutation::validate::validate;
use crate::document::place::{nameable_font, sizes_its_own_text, Unit};
use crate::document::tests::test_support::{node_data, DocumentBuilder};
use crate::document::values::FontSize;
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tree::Placement;
use crate::font::fonts::AppFont;
use indextree::NodeId;

/// A mutator of one payload on `kind`, at the root.
fn at_root(kind: DocKind, mutation: DocMutation) -> DocMutatorTree {
    DocMutatorTree::new_with(Mutator::new(mutation, BranchKey::new(kind, 0)))
}

/// A one-node map, its node's id, and the id of the frame and ring the
/// node holds, each element edited by `edit` before the map is made.
fn map_with(edit: impl FnOnce(&mut DocumentBuilder, NodeId, NodeId, NodeId)) -> Document {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let node = builder.node_with(root, node_data(1));
    let section = builder.add(node, DocElement::Section(SectionData::default()));
    let frame = builder.add(node, DocElement::Frame(FrameData::default()));
    let ring = builder.add(
        frame,
        DocElement::Ring(RingData {
            channel: 0,
            style: RingStyle::default(),
        }),
    );
    edit(&mut builder, node, section, ring);
    builder.build_unchecked().expect("the map row is fine")
}

/// A text style holding only `size`.
fn sized(size: FontSize) -> TextStyle {
    TextStyle {
        size: Some(size),
        ..TextStyle::default()
    }
}

#[test]
fn test_the_unit_a_place_takes_is_one_rule_for_files_and_edits() {
    // The rule: relative in a frame and a ring, points everywhere else; a
    // span takes its place's unit once it has one.
    for (kind, unit) in [
        (DocKind::Node, Unit::Points),
        (DocKind::Section, Unit::Points),
        (DocKind::Span, Unit::Points),
        (DocKind::Frame, Unit::Relative),
        (DocKind::Ring, Unit::Relative),
        (DocKind::CapStart, Unit::Points),
        (DocKind::CapEnd, Unit::Points),
        (DocKind::Label, Unit::Points),
    ] {
        assert_eq!(Unit::of_tier(kind), unit, "{kind:?}");
    }
    assert_eq!(Unit::of_place(DocKind::Span, None), Unit::Either);
    assert_eq!(
        Unit::of_place(DocKind::Span, Some(DocKind::Section)),
        Unit::Points
    );
    assert_eq!(
        Unit::of_place(DocKind::Ring, Some(DocKind::Frame)),
        Unit::Relative
    );
    let (pt, rel) = (FontSize::Pt(12.0), FontSize::Rel(0.5));
    assert!(Unit::Points.admits(pt) && !Unit::Points.admits(rel));
    assert!(Unit::Relative.admits(rel) && !Unit::Relative.admits(pt));
    assert!(Unit::Either.admits(pt) && Unit::Either.admits(rel));

    // Both judges: a size of each unit on a node, a section, a frame and
    // a ring is accepted by the file exactly when an edit may set it.
    for size in [pt, rel] {
        for kind in [DocKind::Node, DocKind::Section, DocKind::Frame, DocKind::Ring] {
            let doc = map_with(|builder, node, section, ring| match kind {
                DocKind::Node => {
                    builder.edit_node(node, |data| data.style.text = sized(size));
                }
                DocKind::Section => {
                    builder.edit(section, |element| {
                        if let DocElement::Section(data) = element {
                            data.text = sized(size);
                        }
                    });
                }
                DocKind::Frame => {
                    let frame = builder.tree().arena()[ring].parent().expect("the ring's frame");
                    builder.set(
                        frame,
                        DocElement::Frame(FrameData {
                            style: FrameStyle {
                                text: sized(size),
                                ..FrameStyle::default()
                            },
                        }),
                    );
                }
                _ => {
                    builder.edit(ring, |element| {
                        if let DocElement::Ring(data) = element {
                            data.style.text = sized(size);
                        }
                    });
                }
            });
            let set = TextOp::SetSize(Some(size));
            let edit = match kind {
                DocKind::Node => DocMutation::Node(NodeOp::Text(set)),
                DocKind::Section => DocMutation::Section(SectionOp::Text(set)),
                DocKind::Frame => DocMutation::Frame(FrameOp::Text(set)),
                _ => DocMutation::Ring(RingOp::Text(set)),
            };
            let saves = check_savable(&doc);
            let edits = validate(&at_root(kind, edit), &doc);
            let expected = Unit::of_tier(kind).admits(size);
            assert_eq!(
                saves.is_ok(),
                expected,
                "{kind:?} {size:?}: the file says {saves:?}"
            );
            assert_eq!(
                edits.is_ok(),
                expected,
                "{kind:?} {size:?}: the edit says {edits:?}"
            );
            if !expected {
                assert!(
                    matches!(saves, Err(ref error) if matches!(error.kind, SaveErrorKind::WrongUnit { .. })),
                    "{saves:?}"
                );
                assert!(matches!(edits, Err(MutationError::Unit { .. })), "{edits:?}");
            }
        }
    }
}

#[test]
fn test_a_span_in_a_ring_sets_no_size_for_files_and_edits() {
    // The rule: only a span inside a ring may not size itself.
    assert!(!sizes_its_own_text(DocKind::Span, Some(DocKind::Ring)));
    for parent in [
        Some(DocKind::Section),
        Some(DocKind::Label),
        Some(DocKind::Connector),
        None,
    ] {
        assert!(sizes_its_own_text(DocKind::Span, parent), "{parent:?}");
    }
    assert!(sizes_its_own_text(DocKind::Ring, Some(DocKind::Frame)));

    let span = |size: Option<FontSize>| SpanData {
        text: "x".to_string(),
        style: TextStyle {
            size,
            ..TextStyle::default()
        },
        link: None,
    };
    for (size, allowed) in [(Some(FontSize::Pt(12.0)), false), (None, true)] {
        // The file: a document holding such a span cannot be saved.
        let doc = map_with(|builder, _, _, ring| {
            builder.add(ring, DocElement::Span(span(size)));
        });
        let saves = check_savable(&doc);
        // The edit: inserting a ring holding it is refused.
        let insert = DocMutatorTree::new_with(Mutator::new_structure(
            StructOp::Insert {
                subtree: Subtree {
                    root: DocElement::Ring(RingData {
                        channel: 1,
                        style: RingStyle::default(),
                    }),
                    children: vec![Subtree::leaf(DocElement::Span(span(size)))],
                },
                at: Placement::Last,
            },
            BranchKey::new(DocKind::Frame, 0),
        ));
        let edits = validate(&insert, &doc);
        assert_eq!(saves.is_ok(), allowed, "{saves:?}");
        assert_eq!(edits.is_ok(), allowed, "{edits:?}");
        if !allowed {
            assert!(
                matches!(saves, Err(ref error) if error.kind == SaveErrorKind::SizeInRing),
                "{saves:?}"
            );
            assert!(
                matches!(edits, Err(MutationError::RingSpanSize { .. })),
                "{edits:?}"
            );
        }
    }
}

#[test]
fn test_a_font_must_have_a_name_a_file_can_write_for_files_and_edits() {
    // The rule: the fallback face never; every compiled-in face, which
    // has a family name, always. (Every face this build compiles in has
    // one, so the "no family name" half of the rule is reached only by
    // `Any` here.)
    assert!(!nameable_font(AppFont::Any));
    assert!(nameable_font(AppFont::Evilz));
    let faces: Vec<AppFont> = std::iter::once(AppFont::Any)
        .chain(crate::font::fonts::FONT_DATA.iter().map(|(font, _)| *font))
        .collect();
    assert!(faces.len() > 10, "precondition: {} faces", faces.len());
    for font in faces {
        let doc = map_with(|builder, node, _, _| {
            builder.edit_node(node, |data| data.style.text.font = Some(font));
        });
        let saves = check_savable(&doc);
        let edits = validate(
            &at_root(
                DocKind::Node,
                DocMutation::Node(NodeOp::Text(TextOp::SetFont(Some(font)))),
            ),
            &doc,
        );
        let expected = nameable_font(font);
        assert_eq!(saves.is_ok(), expected, "{font:?}: the file says {saves:?}");
        assert_eq!(edits.is_ok(), expected, "{font:?}: the edit says {edits:?}");
        if !expected {
            assert!(matches!(saves, Err(ref error) if error.kind == SaveErrorKind::NoFamilyName));
            assert!(matches!(edits, Err(MutationError::AnyFont { .. })), "{edits:?}");
        }
    }
}

#[test]
fn test_a_channel_is_one_bound_for_files_and_edits() {
    // The bound itself, and one past it where a `usize` can hold that.
    let past = MAX_CHANNEL.checked_add(1);
    #[cfg(target_pointer_width = "64")]
    assert!(past.is_some(), "a 64-bit usize holds a channel past the bound");
    for channel in std::iter::once(MAX_CHANNEL).chain(past) {
        let doc = map_with(|builder, node, _, _| {
            builder.edit_node(node, |data| data.channel = channel);
        });
        let saves = check_savable(&doc);
        let edits = validate(
            &at_root(DocKind::Node, DocMutation::Node(NodeOp::SetChannel(channel))),
            &doc,
        );
        let allowed = channel <= MAX_CHANNEL;
        assert_eq!(saves.is_ok(), allowed, "{channel}: {saves:?}");
        assert_eq!(edits.is_ok(), allowed, "{channel}: {edits:?}");
        if !allowed {
            let shown = BoundError::NotAChannel(1 << 32);
            match saves {
                Err(error) => match error.kind {
                    SaveErrorKind::NotLoadable { error } => {
                        assert_eq!(error.path, "nodes[0].channel");
                        assert_eq!(error.kind, LoadErrorKind::Bound { error: shown });
                    }
                    other => panic!("expected the loader's bound, got {other:?}"),
                },
                Ok(()) => unreachable!(),
            }
            assert!(
                matches!(edits, Err(MutationError::Bound { field: "node.channel", error, .. }) if error == shown),
                "{edits:?}"
            );
        }
    }
}
