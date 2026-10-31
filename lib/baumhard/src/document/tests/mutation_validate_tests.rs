// SPDX-License-Identifier: MPL-2.0

//! `validate`, `validate_for_presentation` and `validate_applied`.
//!
//! Every refusal is asserted by its variant and what it names — the
//! field, the reference, the rule, the mutator's path — never by a bare
//! `is_err`, and each sits beside the nearest input that passes, so a
//! check that refused everything would fail the controls.

use crate::document::bounds::{MAX_CANVAS_COORD, MAX_FONT_SIZE_PT};
use crate::document::element::{
    DefaultsData, DocElement, DocKind, LinkData, SectionData, SpanData, TextStyle,
};
use crate::document::ids::{Definitions, Document, NodeIdent, VerbDef};
use crate::document::mutation::error::{MutationError, StructureRule};
use crate::document::mutation::ops::{
    DefaultsOp, DocMutation, DocMutatorTree, FrameOp, LabelOp, LayoutOp, LinkOp, NodeOp, PaletteOp, PathOp,
    PortalEndOp, RingOp, SectionOp, SpanOp, TextOp, ThemeOp,
};
use crate::document::mutation::step::{DocStep, DocWalk};
use crate::document::mutation::validate::{validate, validate_applied, validate_for_presentation};
use crate::document::shape::ShapeError;
use crate::document::tests::test_support::{example, node_data, palette_group, path_data, Example};
use crate::document::values::{
    ColorRef, Edge, FitMode, FontSize, LinkKind, LinkTarget, PaletteId, PlainColor, Sides, StartPoint, VarId,
    VerbId, ZoomWindow,
};
use crate::engine::anchor::Anchors;
use crate::engine::journal::JournalBuilder;
use crate::engine::key::BranchKey;
use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::predicate::Predicate;
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tree::Placement;
use glam::Vec2;
use indextree::NodeId;

/// The example map with one verb defined.
fn document() -> (Document, Example) {
    let (mut builder, ids) = example();
    builder.definitions(Definitions {
        verbs: vec![VerbDef {
            id: "go".to_string(),
            name: "Go".to_string(),
        }],
        ..Definitions::default()
    });
    (builder.build().unwrap(), ids)
}

/// A mutator of one payload on `kind`, below a `Void` node root, so its
/// path is `[0]`.
fn below_node(kind: DocKind, mutation: DocMutation) -> DocMutatorTree {
    let mut tree = DocMutatorTree::new_with(Mutator::new_void(BranchKey::new(DocKind::Node, 0)));
    let root = tree.root;
    tree.insert_child(
        root,
        Mutator::new(mutation, BranchKey::new(kind, 0)),
        Placement::Last,
    )
    .unwrap();
    tree
}

/// A mutator of one payload on `kind`, at the root.
fn at_root(kind: DocKind, mutation: DocMutation) -> DocMutatorTree {
    DocMutatorTree::new_with(Mutator::new(mutation, BranchKey::new(kind, 0)))
}

/// A mutator of one structural command on `kind`, at the root.
fn command(kind: DocKind, op: StructOp<DocElement>) -> DocMutatorTree {
    DocMutatorTree::new_with(Mutator::new_structure(op, BranchKey::new(kind, 0)))
}

/// The field a `Bound` refusal names, and the mutator path.
fn bound_field(result: Result<(), MutationError>) -> (&'static str, Vec<usize>) {
    match result {
        Err(MutationError::Bound { field, at, .. }) => (field, at.mutator),
        other => panic!("expected a bound refusal, got {other:?}"),
    }
}

#[test]
fn test_document_validate_refuses_a_payload_of_another_kind() {
    do_document_validate_refuses_a_payload_of_another_kind();
}

/// A node payload keyed to a section is refused, naming the mutator's
/// path, its kind and the payload; keyed to a node it passes. A path
/// payload passes on a connector and a link, a cap payload on both caps.
pub fn do_document_validate_refuses_a_payload_of_another_kind() {
    let (doc, _) = document();
    let opacity = DocMutation::Node(NodeOp::SetOpacity(0.5));
    assert_eq!(
        validate(&below_node(DocKind::Section, opacity.clone()), &doc),
        Err(MutationError::PayloadKind {
            at: crate::document::mutation::error::MutatorPath {
                mutator: vec![0],
                subtree: None,
            },
            kind: DocKind::Section,
            payload: "Node",
        })
    );
    assert_eq!(validate(&below_node(DocKind::Node, opacity), &doc), Ok(()));
    for kind in [DocKind::Connector, DocKind::Link] {
        let mutation = DocMutation::Path(PathOp::SetVisible(false));
        assert_eq!(validate(&at_root(kind, mutation), &doc), Ok(()), "{kind:?}");
    }
    for kind in [DocKind::CapStart, DocKind::CapEnd] {
        let mutation = DocMutation::Cap(TextOp::SetItalic(Some(true)));
        assert_eq!(validate(&at_root(kind, mutation), &doc), Ok(()), "{kind:?}");
    }
    let mutation = DocMutation::Link(LinkOp::SetKind(LinkKind::Line));
    assert!(matches!(
        validate(&at_root(DocKind::Connector, mutation), &doc),
        Err(MutationError::PayloadKind { payload: "Link", .. })
    ));
}

#[test]
fn test_document_validate_refuses_every_literal_out_of_bounds() {
    do_document_validate_refuses_every_literal_out_of_bounds();
}

/// One refusal per bounded literal, each naming its field and the
/// mutator's path, beside the value at the bound, which passes.
pub fn do_document_validate_refuses_every_literal_out_of_bounds() {
    let (doc, _) = document();
    let fit = |min, max| Some(FitMode::Fit { min, max });
    let zoom = |min, max| Some(ZoomWindow { min, max });
    let points = |n: usize| vec![Vec2::ZERO; n];
    let sides = |content: &str| {
        Some(Sides::On {
            content: content.to_string(),
            color: None,
        })
    };
    #[rustfmt::skip]
    let cases: Vec<(DocKind, DocMutation, DocMutation, &str)> = vec![
        (DocKind::Node, DocMutation::Node(NodeOp::SetPosition(Vec2::new(2.0e9, 0.0))), DocMutation::Node(NodeOp::SetPosition(Vec2::splat(MAX_CANVAS_COORD))), "node.position"),
        (DocKind::Node, DocMutation::Node(NodeOp::Nudge(Vec2::new(f32::NAN, 0.0))), DocMutation::Node(NodeOp::Nudge(Vec2::new(1.0e30, 0.0))), "node.nudge"),
        (DocKind::Node, DocMutation::Node(NodeOp::SetMinSize(Some(Vec2::new(-1.0, 1.0)))), DocMutation::Node(NodeOp::SetMinSize(Some(Vec2::ZERO))), "node.min_size"),
        (DocKind::Node, DocMutation::Node(NodeOp::SetFillFade(Some(-0.5))), DocMutation::Node(NodeOp::SetFillFade(Some(0.0))), "node.fill_fade"),
        (DocKind::Node, DocMutation::Node(NodeOp::SetZoom(zoom(Some(0.0), None))), DocMutation::Node(NodeOp::SetZoom(zoom(Some(0.1), Some(0.1)))), "node.zoom"),
        (DocKind::Node, DocMutation::Node(NodeOp::SetOpacity(1.5)), DocMutation::Node(NodeOp::SetOpacity(1.0)), "node.opacity"),
        (DocKind::Node, DocMutation::Node(NodeOp::Text(TextOp::SetSize(Some(FontSize::Pt(0.4))))), DocMutation::Node(NodeOp::Text(TextOp::SetSize(Some(FontSize::Pt(0.5))))), "text.size"),
        (DocKind::Node, DocMutation::Node(NodeOp::Text(TextOp::GrowSize(f32::INFINITY))), DocMutation::Node(NodeOp::Text(TextOp::GrowSize(-5.0))), "text.grow_size"),
        (DocKind::Node, DocMutation::Node(NodeOp::Text(TextOp::ScaleSize(0.0))), DocMutation::Node(NodeOp::Text(TextOp::ScaleSize(0.01))), "text.scale_size"),
        (DocKind::Section, DocMutation::Section(SectionOp::SetOffset(Vec2::new(0.0, -2.0e9))), DocMutation::Section(SectionOp::SetOffset(Vec2::splat(-MAX_CANVAS_COORD))), "section.offset"),
        (DocKind::Section, DocMutation::Section(SectionOp::Nudge(Vec2::new(0.0, f32::INFINITY))), DocMutation::Section(SectionOp::Nudge(Vec2::ONE)), "section.nudge"),
        (DocKind::Section, DocMutation::Section(SectionOp::SetMinSize(Some(Vec2::new(2.0e9, 1.0)))), DocMutation::Section(SectionOp::SetMinSize(Some(Vec2::splat(MAX_CANVAS_COORD)))), "section.min_size"),
        (DocKind::Frame, DocMutation::Frame(FrameOp::SetStart(Some(StartPoint { edge: Edge::Top, at: 1.1 }))), DocMutation::Frame(FrameOp::SetStart(Some(StartPoint { edge: Edge::Top, at: 1.0 }))), "frame.start"),
        (DocKind::Frame, DocMutation::Frame(FrameOp::SetFit(fit(2.0, 1.0))), DocMutation::Frame(FrameOp::SetFit(fit(1.0, 1.0))), "frame.fit"),
        (DocKind::Frame, DocMutation::Frame(FrameOp::SetGap(Some(-1.0))), DocMutation::Frame(FrameOp::SetGap(Some(0.0))), "frame.gap"),
        (DocKind::Frame, DocMutation::Frame(FrameOp::SetInset(Some(f32::NAN))), DocMutation::Frame(FrameOp::SetInset(Some(5.0))), "frame.inset"),
        (DocKind::Frame, DocMutation::Frame(FrameOp::Text(TextOp::SetSize(Some(FontSize::Rel(1.5))))), DocMutation::Frame(FrameOp::Text(TextOp::SetSize(Some(FontSize::Rel(1.0))))), "text.size"),
        (DocKind::Ring, DocMutation::Ring(RingOp::SetFit(fit(0.0, 1.0))), DocMutation::Ring(RingOp::SetFit(fit(0.1, 1.0))), "ring.fit"),
        (DocKind::Frame, DocMutation::Frame(FrameOp::Layout(LayoutOp::SetSpacing(Some(-1.0)))), DocMutation::Frame(FrameOp::Layout(LayoutOp::SetSpacing(Some(0.0)))), "frame.spacing"),
        (DocKind::Ring, DocMutation::Ring(RingOp::Layout(LayoutOp::SetSpacing(Some(f32::NAN)))), DocMutation::Ring(RingOp::Layout(LayoutOp::SetSpacing(Some(6.0)))), "ring.spacing"),
        (DocKind::Frame, DocMutation::Frame(FrameOp::Layout(LayoutOp::SetSides(sides("")))), DocMutation::Frame(FrameOp::Layout(LayoutOp::SetSides(sides("✦")))), "frame.sides"),
        (DocKind::Ring, DocMutation::Ring(RingOp::Layout(LayoutOp::SetSides(sides("")))), DocMutation::Ring(RingOp::Layout(LayoutOp::SetSides(sides("·")))), "ring.sides"),
        (DocKind::Link, DocMutation::Path(PathOp::SetSpacing(Some(-1.0))), DocMutation::Path(PathOp::SetSpacing(Some(0.0))), "path.spacing"),
        (DocKind::Link, DocMutation::Path(PathOp::SetSize(Some(5000.0))), DocMutation::Path(PathOp::SetSize(Some(MAX_FONT_SIZE_PT))), "path.size"),
        (DocKind::Link, DocMutation::Path(PathOp::SetControlPoints(points(3))), DocMutation::Path(PathOp::SetControlPoints(points(2))), "path.control_points"),
        (DocKind::Link, DocMutation::Path(PathOp::SetControlPoints(vec![Vec2::new(2.0e9, 0.0)])), DocMutation::Path(PathOp::SetControlPoints(vec![Vec2::new(1.0e9, 0.0)])), "path.control_points"),
        (DocKind::Link, DocMutation::Path(PathOp::MoveControlPoint { index: 2, to: Vec2::ZERO }), DocMutation::Path(PathOp::MoveControlPoint { index: 1, to: Vec2::ZERO }), "path.control_points"),
        (DocKind::Link, DocMutation::Path(PathOp::MoveControlPoint { index: 0, to: Vec2::new(f32::NAN, 0.0) }), DocMutation::Path(PathOp::MoveControlPoint { index: 0, to: Vec2::ONE }), "path.control_points"),
        (DocKind::Connector, DocMutation::Path(PathOp::SetZoom(zoom(Some(2.0), Some(1.0)))), DocMutation::Path(PathOp::SetZoom(zoom(Some(1.0), Some(2.0)))), "path.zoom"),
        (DocKind::Label, DocMutation::Label(LabelOp::SetT(Some(-0.1))), DocMutation::Label(LabelOp::SetT(Some(0.0))), "label.t"),
        (DocKind::Label, DocMutation::Label(LabelOp::SetOffset(Some(f32::INFINITY))), DocMutation::Label(LabelOp::SetOffset(Some(-3.0))), "label.offset"),
        (DocKind::PortalEnd, DocMutation::PortalEnd(PortalEndOp::SetBorderT(Some(2.0))), DocMutation::PortalEnd(PortalEndOp::SetBorderT(Some(1.0))), "portal_end.border_t"),
        (DocKind::PortalEnd, DocMutation::PortalEnd(PortalEndOp::SetOffset(Some(-2.0e9))), DocMutation::PortalEnd(PortalEndOp::SetOffset(Some(0.0))), "portal_end.offset"),
        (DocKind::PortalEnd, DocMutation::PortalEnd(PortalEndOp::SetSize(Some(0.1))), DocMutation::PortalEnd(PortalEndOp::SetSize(Some(50.0))), "portal_end.size"),
    ];
    for (kind, refused, accepted, field) in cases {
        assert_eq!(
            bound_field(validate(&below_node(kind, refused.clone()), &doc)),
            (field, vec![0]),
            "{refused:?}"
        );
        assert_eq!(
            validate(&below_node(kind, accepted.clone()), &doc),
            Ok(()),
            "{accepted:?}"
        );
    }

    // The defaults are checked whole.
    let mut defaults = DefaultsData::default();
    defaults.connector.size = Some(5000.0);
    let replace = |d: &DefaultsData| DocMutation::Defaults(DefaultsOp::Replace(Box::new(d.clone())));
    assert_eq!(
        bound_field(validate(&at_root(DocKind::Defaults, replace(&defaults)), &doc)),
        ("path.size", vec![])
    );
    defaults.connector.size = Some(12.0);
    assert_eq!(
        validate(&at_root(DocKind::Defaults, replace(&defaults)), &doc),
        Ok(())
    );

    // So is every element of an inserted subtree, named by its path in it.
    let mut child = node_data(0);
    child.position = Vec2::new(0.0, 3.0e9);
    let subtree = Subtree {
        root: DocElement::Node(node_data(0)),
        children: vec![
            Subtree::leaf(DocElement::Section(SectionData::default())),
            Subtree {
                root: DocElement::Node(child),
                children: vec![Subtree::leaf(connector())],
            },
            Subtree::leaf(connector()),
        ],
    };
    let insert = |subtree| {
        command(
            DocKind::Node,
            StructOp::Insert {
                subtree,
                at: Placement::Last,
            },
        )
    };
    match validate(&insert(subtree.clone()), &doc) {
        Err(MutationError::Bound { at, field, .. }) => {
            assert_eq!(field, "node.position");
            assert_eq!(at.subtree, Some(vec![1]), "the second child of the subtree");
        }
        other => panic!("expected a bound refusal, got {other:?}"),
    }
    let mut fixed = subtree;
    fixed.children[1].root = DocElement::Node(node_data(0));
    assert_eq!(validate(&insert(fixed), &doc), Ok(()));
}

fn connector() -> DocElement {
    DocElement::Connector(crate::document::element::ConnectorData { path: path_data() })
}

#[test]
fn test_document_validate_holds_sizes_to_their_unit() {
    do_document_validate_holds_sizes_to_their_unit();
}

/// A ring's, a frame's and their defaults' sizes are relative, and a
/// point size there is refused; every other place's are points, and a
/// relative size there is refused (ruling R10 on #237). A span's place
/// is not known before the walk, so it may carry either.
pub fn do_document_validate_holds_sizes_to_their_unit() {
    let (doc, _) = document();
    let pt = Some(FontSize::Pt(12.0));
    let rel = Some(FontSize::Rel(0.1));
    let set = |kind: DocKind, size| {
        let op = TextOp::SetSize(size);
        let mutation = match kind {
            DocKind::Node => DocMutation::Node(NodeOp::Text(op)),
            DocKind::Section => DocMutation::Section(SectionOp::Text(op)),
            DocKind::Span => DocMutation::Span(SpanOp::Text(op)),
            DocKind::Frame => DocMutation::Frame(FrameOp::Text(op)),
            DocKind::Ring => DocMutation::Ring(RingOp::Text(op)),
            DocKind::Label => DocMutation::Label(LabelOp::Text(op)),
            _ => DocMutation::Cap(op),
        };
        validate(&at_root(kind, mutation), &doc)
    };
    for kind in [DocKind::Frame, DocKind::Ring] {
        assert!(
            matches!(
                set(kind, pt),
                Err(MutationError::Unit {
                    size: FontSize::Pt(_),
                    ..
                })
            ),
            "{kind:?} refuses points"
        );
        assert_eq!(set(kind, rel), Ok(()), "{kind:?} takes a relative size");
    }
    for kind in [DocKind::Node, DocKind::Section, DocKind::CapStart, DocKind::Label] {
        assert!(
            matches!(
                set(kind, rel),
                Err(MutationError::Unit {
                    size: FontSize::Rel(_),
                    ..
                })
            ),
            "{kind:?} refuses a relative size"
        );
        assert_eq!(set(kind, pt), Ok(()), "{kind:?} takes points");
    }
    assert_eq!(set(DocKind::Span, pt), Ok(()));
    assert_eq!(set(DocKind::Span, rel), Ok(()));

    let replace = |d: DefaultsData| {
        at_root(
            DocKind::Defaults,
            DocMutation::Defaults(DefaultsOp::Replace(Box::new(d))),
        )
    };
    let mut ring_pt = DefaultsData::default();
    ring_pt.ring.text.size = pt;
    let mut frame_pt = DefaultsData::default();
    frame_pt.frame.text.size = pt;
    let mut section_rel = DefaultsData::default();
    section_rel.section.size = rel;
    for (name, d) in [("ring", ring_pt), ("frame", frame_pt), ("section", section_rel)] {
        assert!(
            matches!(validate(&replace(d), &doc), Err(MutationError::Unit { .. })),
            "Defaults.{name}"
        );
    }
    let mut fine = DefaultsData::default();
    fine.ring.text.size = rel;
    fine.frame.text.size = rel;
    fine.section.size = pt;
    fine.span.size = pt;
    assert_eq!(validate(&replace(fine), &doc), Ok(()));
}

#[test]
fn test_document_validate_refuses_a_missing_reference() {
    do_document_validate_refuses_a_missing_reference();
}

/// A verb, a palette, a theme variable, a theme variant and a palette
/// group each must exist; the last one that does passes.
pub fn do_document_validate_refuses_a_missing_reference() {
    let (doc, _) = document();
    assert_eq!(doc.definitions().verbs.len(), 1);
    assert_eq!(doc.theme().variables.len(), 2);
    assert_eq!(doc.theme().variants.len(), 2);
    let missing = |mutator: DocMutatorTree| match validate(&mutator, &doc) {
        Err(MutationError::Missing { what, index, .. }) => (what, index),
        other => panic!("expected a missing reference, got {other:?}"),
    };
    let node = |op| at_root(DocKind::Node, DocMutation::Node(op));
    assert_eq!(missing(node(NodeOp::SetClick(Some(VerbId(1))))), ("verb", 1));
    assert_eq!(validate(&node(NodeOp::SetClick(Some(VerbId(0)))), &doc), Ok(()));
    assert_eq!(
        missing(node(NodeOp::SetPalette(Some(PaletteId(1))))),
        ("palette", 1)
    );
    assert_eq!(
        validate(&node(NodeOp::SetPalette(Some(PaletteId(0)))), &doc),
        Ok(())
    );
    assert_eq!(
        missing(node(NodeOp::SetFill(Some(ColorRef::Var(VarId(2)))))),
        ("theme variable", 2)
    );
    assert_eq!(
        validate(&node(NodeOp::SetFill(Some(ColorRef::Var(VarId(1))))), &doc),
        Ok(())
    );
    let link = DocMutation::Span(SpanOp::SetLink(Some(LinkTarget::Verb(VerbId(4)))));
    assert_eq!(missing(at_root(DocKind::Span, link)), ("verb", 4));

    let theme = |op| at_root(DocKind::Theme, DocMutation::Theme(op));
    assert_eq!(missing(theme(ThemeOp::SetActive(2))), ("theme variant", 2));
    assert_eq!(validate(&theme(ThemeOp::SetActive(1)), &doc), Ok(()));
    let value = |variant, var| ThemeOp::SetValue {
        variant,
        var: VarId(var),
        value: crate::document::values::Rgba::opaque(1, 2, 3),
    };
    assert_eq!(missing(theme(value(1, 2))), ("theme variable", 2));
    assert_eq!(missing(theme(value(3, 0))), ("theme variant", 3));
    assert_eq!(validate(&theme(value(1, 1)), &doc), Ok(()));

    let palette = |index, group| {
        at_root(
            DocKind::Palette,
            DocMutation::Palette(PaletteOp::SetGroup { index, group }),
        )
    };
    assert_eq!(missing(palette(3, palette_group(1))), ("palette group", 3));
    assert_eq!(validate(&palette(2, palette_group(1)), &doc), Ok(()));
    let mut group = palette_group(1);
    group.title = Some(PlainColor::Var(VarId(7)));
    assert_eq!(missing(palette(0, group)), ("theme variable", 7));
}

#[test]
fn test_document_validate_holds_structure_to_the_document_s_rules() {
    do_document_validate_holds_structure_to_the_document_s_rules();
}

/// Only nodes move, under a node or the map; only spans split and
/// merge; the map row's theme, palettes and defaults are neither
/// inserted, removed nor moved, and the map is never a command's target;
/// an inserted subtree keeps the shape rules, its nodes carry no ident
/// and its links lead to nodes. Each has a passing counterpart.
pub fn do_document_validate_holds_structure_to_the_document_s_rules() {
    let (doc, ids) = document();
    let rule = |mutator: DocMutatorTree| match validate(&mutator, &doc) {
        Err(MutationError::Structure { rule, .. }) => rule,
        other => panic!("expected a structure refusal, got {other:?}"),
    };
    let to = |to: NodeId| StructOp::Move {
        to,
        at: Placement::Last,
    };
    assert_eq!(
        rule(command(DocKind::Section, to(ids.a))),
        StructureRule::MoveNotANode
    );
    assert_eq!(
        rule(command(DocKind::Node, to(ids.a_title))),
        StructureRule::MoveDestination
    );
    assert_eq!(validate(&command(DocKind::Node, to(ids.a)), &doc), Ok(()));
    assert_eq!(validate(&command(DocKind::Node, to(doc.root())), &doc), Ok(()));
    assert_eq!(
        rule(command(DocKind::Section, StructOp::Split { at: 1 })),
        StructureRule::SplitMergeNotASpan
    );
    assert_eq!(
        rule(command(DocKind::Node, StructOp::Merge)),
        StructureRule::SplitMergeNotASpan
    );
    assert_eq!(validate(&command(DocKind::Span, StructOp::Merge), &doc), Ok(()));
    assert_eq!(
        rule(command(DocKind::Palette, StructOp::Remove)),
        StructureRule::MapRow
    );
    assert_eq!(rule(command(DocKind::Map, StructOp::Remove)), StructureRule::Map);
    assert_eq!(validate(&command(DocKind::Label, StructOp::Remove), &doc), Ok(()));

    let insert = |kind, root: DocElement, children| {
        command(
            kind,
            StructOp::Insert {
                subtree: Subtree { root, children },
                at: Placement::Last,
            },
        )
    };
    let section = || DocElement::Section(SectionData::default());
    assert_eq!(
        rule(insert(DocKind::Map, section(), vec![])),
        StructureRule::MapRow
    );
    assert_eq!(
        rule(insert(DocKind::Node, DocElement::Map(Default::default()), vec![])),
        StructureRule::Map
    );
    assert_eq!(validate(&insert(DocKind::Node, section(), vec![]), &doc), Ok(()));
    assert!(matches!(
        validate(
            &insert(DocKind::Node, section(), vec![Subtree::leaf(section())]),
            &doc
        ),
        Err(MutationError::Subtree {
            error: ShapeError::SubtreeRow { .. },
            ..
        })
    ));
    let assigned = node_data(7);
    assert_eq!(
        validate(&insert(DocKind::Map, DocElement::Node(assigned), vec![]), &doc),
        Err(MutationError::AssignedIdent {
            at: crate::document::mutation::error::MutatorPath {
                mutator: vec![],
                subtree: Some(vec![]),
            },
            ident: NodeIdent(7),
        })
    );
    assert_eq!(
        validate(
            &insert(DocKind::Map, DocElement::Node(node_data(0)), vec![]),
            &doc
        ),
        Ok(())
    );
    let link = |to| {
        DocElement::Link(LinkData {
            to,
            kind: LinkKind::Line,
            path: path_data(),
        })
    };
    assert!(matches!(
        validate(&insert(DocKind::Node, link(ids.a_title), vec![]), &doc),
        Err(MutationError::LinkTarget { to, .. }) if to == ids.a_title
    ));
    assert_eq!(
        validate(&insert(DocKind::Node, link(ids.d), vec![]), &doc),
        Ok(())
    );
}

#[test]
fn test_document_validate_refuses_positional_and_spatial_instructions() {
    do_document_validate_refuses_positional_and_spatial_instructions();
}

/// `MapChildren` would pair a payload with a child of any kind, and
/// `SpatialDescend` needs a spatial element: both are refused. A
/// `RepeatWhile`, routed by kind, passes.
pub fn do_document_validate_refuses_positional_and_spatial_instructions() {
    let (doc, _) = document();
    let key = BranchKey::new(DocKind::Node, 0);
    let refused = DocMutatorTree::new_with(Mutator::new_instruction(Instruction::MapChildren, key));
    assert!(matches!(
        validate(&refused, &doc),
        Err(MutationError::Instruction { .. })
    ));
    let repeat = DocMutatorTree::new_with(Mutator::new_instruction(
        Instruction::RepeatWhile(Predicate::always_true()),
        key,
    ));
    assert_eq!(validate(&repeat, &doc), Ok(()));
}

#[test]
fn test_document_validate_refuses_a_bare_kind_switch() {
    do_document_validate_refuses_a_bare_kind_switch();
}

/// `SetKind(Portal)` needs a `Remove` of the spans, caps and label as
/// children of its mutator; `SetKind(Line)` one of both portal ends. A
/// switch with none, or missing one, is refused; with all, it passes.
pub fn do_document_validate_refuses_a_bare_kind_switch() {
    let (doc, _) = document();
    let switch = |kind, removes: &[BranchKey<DocKind>]| {
        let mut tree = at_root(DocKind::Link, DocMutation::Link(LinkOp::SetKind(kind)));
        let root = tree.root;
        for &key in removes {
            tree.insert_child(
                root,
                Mutator::new_structure(StructOp::Remove, key),
                Placement::Last,
            )
            .unwrap();
        }
        validate(&tree, &doc)
    };
    let line_children = [
        BranchKey::new(DocKind::Span, 0),
        BranchKey::new(DocKind::CapStart, 0),
        BranchKey::new(DocKind::CapEnd, 0),
        BranchKey::new(DocKind::Label, 0),
    ];
    let ends = [
        BranchKey::new(DocKind::PortalEnd, 0),
        BranchKey::new(DocKind::PortalEnd, 1),
    ];
    let bare = |kind| {
        Err(MutationError::BareKindSwitch {
            at: Default::default(),
            kind,
        })
    };
    assert_eq!(switch(LinkKind::Portal, &[]), bare(LinkKind::Portal));
    assert_eq!(
        switch(LinkKind::Portal, &line_children[..3]),
        bare(LinkKind::Portal)
    );
    assert_eq!(switch(LinkKind::Portal, &line_children), Ok(()));
    assert_eq!(switch(LinkKind::Line, &ends[..1]), bare(LinkKind::Line));
    assert_eq!(switch(LinkKind::Line, &ends), Ok(()));
}

#[test]
fn test_document_presentations_change_fields_only() {
    do_document_presentations_change_fields_only();
}

/// A presentation may hide a node and fade it, but a structural command
/// is refused — which `validate` alone accepts (D17).
pub fn do_document_presentations_change_fields_only() {
    let (doc, _) = document();
    for op in [NodeOp::SetHidden(true), NodeOp::SetOpacity(0.0)] {
        assert_eq!(
            validate_for_presentation(&at_root(DocKind::Node, DocMutation::Node(op)), &doc),
            Ok(())
        );
    }
    let mut remove = below_node(DocKind::Node, DocMutation::Node(NodeOp::SetHidden(true)));
    let root = remove.root;
    remove
        .insert_child(
            root,
            Mutator::new_structure(StructOp::Remove, BranchKey::new(DocKind::Section, 0)),
            Placement::Last,
        )
        .unwrap();
    assert_eq!(validate(&remove, &doc), Ok(()));
    match validate_for_presentation(&remove, &doc) {
        Err(MutationError::PresentationStructure { at }) => assert_eq!(at.mutator, vec![0]),
        other => panic!("expected a presentation refusal, got {other:?}"),
    }
}

/// Run `walks` as steps of one request on `doc`, close the journal and
/// judge it.
fn applied(doc: &mut Document, walks: Vec<(DocMutatorTree, Vec<NodeId>)>) -> Result<(), MutationError> {
    let steps = walks
        .into_iter()
        .map(|(mutator, anchors)| DocStep::fixed(DocWalk::new(mutator, Anchors::Nodes(anchors))))
        .collect();
    let builder = doc.run_steps(steps, JournalBuilder::new())?;
    let journal = builder.close(doc.tree());
    validate_applied(doc.tree(), &journal)
}

#[test]
fn test_document_validate_applied_judges_what_the_steps_did() {
    do_document_validate_applied_judges_what_the_steps_did();
}

/// After the steps: a node left without its connector, a root moved
/// under a node without one, a link switched to a portal with no ends, a
/// link inserted to its own node or to a node a later step removed, and a
/// size in the unit its place does not take are each refused; the
/// nearest change that keeps the rules passes.
pub fn do_document_validate_applied_judges_what_the_steps_did() {
    let shape = |result: Result<(), MutationError>| match result {
        Err(MutationError::Shape(ShapeError::Row { element, .. })) => element,
        other => panic!("expected a shape refusal, got {other:?}"),
    };
    let remove = |kind| command(kind, StructOp::Remove);

    let (mut doc, ids) = document();
    assert_eq!(
        shape(applied(
            &mut doc,
            vec![(remove(DocKind::Connector), vec![ids.b_connector])]
        )),
        ids.b
    );
    let (mut doc, ids) = document();
    assert_eq!(
        applied(&mut doc, vec![(remove(DocKind::Label), vec![ids.b_label])]),
        Ok(())
    );

    let to_b = command(
        DocKind::Node,
        StructOp::Move {
            to: ids.b,
            at: Placement::Last,
        },
    );
    let (mut doc, _) = document();
    assert_eq!(shape(applied(&mut doc, vec![(to_b.clone(), vec![ids.d])])), ids.d);
    let (mut doc, _) = document();
    let connect = command(
        DocKind::Node,
        StructOp::Insert {
            subtree: Subtree::leaf(connector()),
            at: Placement::Last,
        },
    );
    assert_eq!(
        applied(&mut doc, vec![(connect, vec![ids.d]), (to_b, vec![ids.d])]),
        Ok(())
    );

    let (mut doc, _) = document();
    let mut switch = at_root(
        DocKind::Link,
        DocMutation::Link(LinkOp::SetKind(LinkKind::Portal)),
    );
    let root = switch.root;
    for kind in [DocKind::Span, DocKind::CapStart, DocKind::CapEnd, DocKind::Label] {
        switch
            .insert_child(
                root,
                Mutator::new_structure(StructOp::Remove, BranchKey::new(kind, 0)),
                Placement::Last,
            )
            .unwrap();
    }
    assert_eq!(shape(applied(&mut doc, vec![(switch, vec![ids.line])])), ids.line);

    let link = |to| {
        command(
            DocKind::Node,
            StructOp::Insert {
                subtree: Subtree::leaf(DocElement::Link(LinkData {
                    to,
                    kind: LinkKind::Line,
                    path: path_data(),
                })),
                at: Placement::Last,
            },
        )
    };
    let (mut doc, _) = document();
    assert!(matches!(
        applied(&mut doc, vec![(link(ids.b), vec![ids.b])]),
        Err(MutationError::SelfLink { .. })
    ));
    let (mut doc, _) = document();
    assert_eq!(applied(&mut doc, vec![(link(ids.d), vec![ids.b])]), Ok(()));
    let (mut doc, _) = document();
    assert!(matches!(
        applied(&mut doc, vec![(link(ids.d), vec![ids.b]), (remove(DocKind::Node), vec![ids.d])]),
        Err(MutationError::DanglingLink { to, .. }) if to == ids.d
    ));

    let size = |size| {
        at_root(
            DocKind::Span,
            DocMutation::Span(SpanOp::Text(TextOp::SetSize(Some(size)))),
        )
    };
    let placed = |result: Result<(), MutationError>| match result {
        Err(MutationError::PlacedUnit { element, .. }) => element,
        other => panic!("expected a placed-unit refusal, got {other:?}"),
    };
    let ring_span = |result: Result<(), MutationError>| match result {
        Err(MutationError::PlacedRingSpanSize { element }) => element,
        other => panic!("expected a ring-span refusal, got {other:?}"),
    };
    // A span inside a ring sets no size, in either unit (R14.1): the
    // ring's is the one size.
    for unit in [FontSize::Pt(12.0), FontSize::Rel(0.1)] {
        let (mut doc, _) = document();
        assert_eq!(
            ring_span(applied(&mut doc, vec![(size(unit), vec![ids.ring_span])])),
            ids.ring_span,
            "{unit:?}"
        );
    }
    // A computed size is refused the same way.
    let (mut doc, _) = document();
    let grow = at_root(
        DocKind::Span,
        DocMutation::Span(SpanOp::Text(TextOp::GrowSize(0.01))),
    );
    assert_eq!(
        ring_span(applied(&mut doc, vec![(grow, vec![ids.ring_span])])),
        ids.ring_span
    );
    // Control: a section's span takes points, and only points.
    let (mut doc, _) = document();
    assert_eq!(
        applied(&mut doc, vec![(size(FontSize::Pt(12.0)), vec![ids.b_span])]),
        Ok(())
    );
    let (mut doc, _) = document();
    assert_eq!(
        placed(applied(
            &mut doc,
            vec![(size(FontSize::Rel(0.1)), vec![ids.b_span])]
        )),
        ids.b_span
    );

    // An inserted span is judged where it lands: at the top of the
    // insert after the walk, inside it before.
    let span_sized = |size: Option<FontSize>| {
        DocElement::Span(SpanData {
            text: "x".to_string(),
            style: TextStyle {
                size,
                ..TextStyle::default()
            },
            link: None,
        })
    };
    let insert_span = |size| {
        command(
            DocKind::Ring,
            StructOp::Insert {
                subtree: Subtree::leaf(span_sized(Some(size))),
                at: Placement::Last,
            },
        )
    };
    let (mut doc, _) = document();
    let refused = ring_span(applied(
        &mut doc,
        vec![(insert_span(FontSize::Rel(0.2)), vec![ids.ring])],
    ));
    assert_eq!(Some(refused), doc.children(ids.ring).last());
    let insert_ring = |size| {
        command(
            DocKind::Frame,
            StructOp::Insert {
                subtree: Subtree {
                    root: DocElement::Ring(Default::default()),
                    children: vec![Subtree::leaf(span_sized(size))],
                },
                at: Placement::Last,
            },
        )
    };
    let (mut doc, _) = document();
    assert!(matches!(
        applied(&mut doc, vec![(insert_ring(Some(FontSize::Rel(0.2))), vec![ids.frame])]),
        Err(MutationError::RingSpanSize { at }) if at.subtree == Some(vec![0])
    ));
    let (mut doc, _) = document();
    assert_eq!(
        applied(&mut doc, vec![(insert_ring(None), vec![ids.frame])]),
        Ok(()),
        "a ring span with no size of its own is fine"
    );

    // A frame inserted with its relative size keeps it: its own kind
    // decides its unit.
    let insert_frame = command(
        DocKind::Node,
        StructOp::Insert {
            subtree: Subtree::leaf(DocElement::Frame(crate::document::element::FrameData {
                style: crate::document::element::FrameStyle {
                    text: TextStyle {
                        size: Some(FontSize::Rel(0.1)),
                        ..TextStyle::default()
                    },
                    ..Default::default()
                },
            })),
            at: Placement::Last,
        },
    );
    let (mut doc, _) = document();
    assert_eq!(applied(&mut doc, vec![(insert_frame, vec![ids.d])]), Ok(()));
}

#[test]
fn test_document_run_steps_validates_and_is_all_or_nothing() {
    do_document_run_steps_validates_and_is_all_or_nothing();
}

/// The steps of a request run on one builder: a step that fails
/// validation, or fails to build, undoes the steps before it, and the
/// document is exactly as it was; without the failing step the same
/// request applies.
pub fn do_document_run_steps_validates_and_is_all_or_nothing() {
    let (mut doc, ids) = document();
    let snapshot = |doc: &Document| -> Vec<(NodeId, Option<NodeId>, DocElement)> {
        doc.tree()
            .descendants()
            .map(|id| (id, doc.parent(id), doc.get(id).cloned().unwrap()))
            .collect()
    };
    let before = snapshot(&doc);
    let hide = || {
        DocStep::fixed(DocWalk::new(
            at_root(DocKind::Node, DocMutation::Node(NodeOp::SetHidden(true))),
            Anchors::Nodes(vec![ids.b]),
        ))
    };
    let invalid = DocStep::fixed(DocWalk::new(
        at_root(DocKind::Node, DocMutation::Node(NodeOp::SetOpacity(2.0))),
        Anchors::Nodes(vec![ids.d]),
    ));
    let result = doc.run_steps(vec![hide(), invalid], JournalBuilder::new());
    assert!(matches!(
        result,
        Err(MutationError::Bound {
            field: "node.opacity",
            ..
        })
    ));
    assert_eq!(snapshot(&doc), before, "the first step is undone");

    let element = ids.a;
    let failing = DocStep::new(move |_| {
        Err(MutationError::NotA {
            element,
            expected: "anything",
        })
    });
    assert!(doc
        .run_steps(vec![hide(), failing], JournalBuilder::new())
        .is_err());
    assert_eq!(snapshot(&doc), before, "a step that fails to build undoes too");

    let builder = doc.run_steps(vec![hide()], JournalBuilder::new()).unwrap();
    let _ = builder.close(doc.tree());
    assert!(matches!(doc.get(ids.b), Some(DocElement::Node(n)) if n.hidden));
}

#[test]
fn test_document_validate_refuses_the_fallback_font() {
    do_document_validate_refuses_the_fallback_font();
}

/// `AppFont::Any` names no face: it is refused by `SetFont`, inside an
/// inserted subtree (named by its path there) and in replaced defaults
/// (R14.2). A real face, and no face (`None`, which inherits), pass.
pub fn do_document_validate_refuses_the_fallback_font() {
    use crate::font::fonts::AppFont;
    let (doc, _) = document();
    let set_font = |font| {
        at_root(
            DocKind::Span,
            DocMutation::Span(SpanOp::Text(TextOp::SetFont(font))),
        )
    };
    assert!(matches!(
        validate(&set_font(Some(AppFont::Any)), &doc),
        Err(MutationError::AnyFont { .. })
    ));
    assert_eq!(validate(&set_font(Some(AppFont::Evilz)), &doc), Ok(()));
    assert_eq!(validate(&set_font(None), &doc), Ok(()));

    let section_with = |font| {
        command(
            DocKind::Node,
            StructOp::Insert {
                subtree: Subtree {
                    root: DocElement::Section(SectionData::default()),
                    children: vec![Subtree::leaf(DocElement::Span(SpanData {
                        text: "x".to_string(),
                        style: TextStyle {
                            font: Some(font),
                            ..TextStyle::default()
                        },
                        link: None,
                    }))],
                },
                at: Placement::Last,
            },
        )
    };
    match validate(&section_with(AppFont::Any), &doc) {
        Err(MutationError::AnyFont { at }) => assert_eq!(at.subtree, Some(vec![0])),
        other => panic!("expected the fallback font refused, got {other:?}"),
    }
    assert_eq!(validate(&section_with(AppFont::Evilz), &doc), Ok(()));

    let replace = |font| {
        let mut defaults = DefaultsData::default();
        defaults.label.text.font = Some(font);
        at_root(
            DocKind::Defaults,
            DocMutation::Defaults(DefaultsOp::Replace(Box::new(defaults))),
        )
    };
    assert!(matches!(
        validate(&replace(AppFont::Any), &doc),
        Err(MutationError::AnyFont { .. })
    ));
    assert_eq!(validate(&replace(AppFont::Evilz), &doc), Ok(()));
}

#[test]
fn test_document_validate_bounds_channels() {
    do_document_validate_bounds_channels();
}

/// A node's, a section's and a ring's channel is at most `MAX_CHANNEL`,
/// set by `SetChannel` or carried by an inserted element (R14.3); the
/// bound itself passes. Only a target whose `usize` is wider than 32
/// bits can hold a channel past it.
pub fn do_document_validate_bounds_channels() {
    use crate::document::bounds::{BoundError, MAX_CHANNEL};
    let (doc, _) = document();
    let set = |kind, channel| {
        let mutation = match kind {
            DocKind::Node => DocMutation::Node(NodeOp::SetChannel(channel)),
            DocKind::Section => DocMutation::Section(SectionOp::SetChannel(channel)),
            _ => DocMutation::Ring(RingOp::SetChannel(channel)),
        };
        validate(&at_root(kind, mutation), &doc)
    };
    let insert_section = |channel| {
        command(
            DocKind::Node,
            StructOp::Insert {
                subtree: Subtree::leaf(DocElement::Section(SectionData {
                    channel,
                    ..SectionData::default()
                })),
                at: Placement::Last,
            },
        )
    };
    for (kind, field) in [
        (DocKind::Node, "node.channel"),
        (DocKind::Section, "section.channel"),
        (DocKind::Ring, "ring.channel"),
    ] {
        assert_eq!(set(kind, MAX_CHANNEL), Ok(()), "{kind:?}");
        if let Some(past) = MAX_CHANNEL.checked_add(1) {
            assert_eq!(bound_field(set(kind, past)), (field, vec![]), "{kind:?}");
        }
    }
    assert_eq!(validate(&insert_section(MAX_CHANNEL), &doc), Ok(()));
    #[cfg(target_pointer_width = "64")]
    assert!(
        MAX_CHANNEL.checked_add(1).is_some(),
        "a 64-bit usize holds a channel past the bound"
    );
    if let Some(past) = MAX_CHANNEL.checked_add(1) {
        match validate(&insert_section(past), &doc) {
            Err(MutationError::Bound { field, error, at }) => {
                assert_eq!(field, "section.channel");
                assert_eq!(error, BoundError::NotAChannel(1 << 32));
                assert_eq!(at.subtree, Some(vec![]));
            }
            other => panic!("expected the channel refused, got {other:?}"),
        }
    }
}

#[test]
fn test_document_validate_refuses_an_inverted_range() {
    do_document_validate_refuses_an_inverted_range();
}

/// `DeleteText` from 3 to 1 is a malformed range, refused with both
/// indices as they were given — integers, never rounded through a
/// float; an empty range passes.
pub fn do_document_validate_refuses_an_inverted_range() {
    let (doc, _) = document();
    let delete = |start, end| {
        at_root(
            DocKind::Span,
            DocMutation::Span(SpanOp::DeleteText { start, end }),
        )
    };
    let huge = (1usize << 24) + 1;
    assert_eq!(
        validate(&delete(huge, 1), &doc),
        Err(MutationError::InvertedRange {
            at: Default::default(),
            start: huge,
            end: 1,
        })
    );
    assert_eq!(validate(&delete(1, 1), &doc), Ok(()));
}

#[test]
fn test_document_validate_applied_judges_nested_links() {
    do_document_validate_applied_judges_nested_links();
}

/// A link nested inside an inserted node — not at the top of the insert
/// — whose target a later step of the same request removes is a
/// dangling link (R14.8); a nested link to a node that stays passes.
pub fn do_document_validate_applied_judges_nested_links() {
    let (_, ids) = document();
    let node_with_link = |to| {
        command(
            DocKind::Node,
            StructOp::Insert {
                subtree: Subtree {
                    root: DocElement::Node(node_data(0)),
                    children: vec![
                        Subtree::leaf(connector()),
                        Subtree::leaf(DocElement::Link(LinkData {
                            to,
                            kind: LinkKind::Line,
                            path: path_data(),
                        })),
                    ],
                },
                at: Placement::Last,
            },
        )
    };
    let (mut doc, _) = document();
    let remove_d = command(DocKind::Node, StructOp::Remove);
    assert!(matches!(
        applied(&mut doc, vec![(node_with_link(ids.d), vec![ids.b]), (remove_d, vec![ids.d])]),
        Err(MutationError::DanglingLink { to, .. }) if to == ids.d
    ));
    let (mut doc, _) = document();
    assert_eq!(
        applied(&mut doc, vec![(node_with_link(ids.d), vec![ids.b])]),
        Ok(())
    );
}

/// How deep the deep tests go: §20 rule 11's chain.
const DEEP: usize = 100_000;

#[test]
fn test_document_validate_walks_a_deep_mutator() {
    do_document_validate_walks_a_deep_mutator();
}

/// A mutator tree `DEEP` levels deep — a chain of node mutators, as a
/// map-borne edit could carry (D29) — is validated on a default test
/// thread, so the walk over it costs no call stack per level (§20
/// rule 11, R14.4); a payload of another kind at the bottom is still
/// found, and named by its full path.
pub fn do_document_validate_walks_a_deep_mutator() {
    let (doc, _) = document();
    let mut deep = DocMutatorTree::new_with(Mutator::new_void(BranchKey::new(DocKind::Node, 0)));
    let mut at = deep.root;
    for _ in 1..DEEP {
        at = deep
            .insert_child(
                at,
                Mutator::new_void(BranchKey::new(DocKind::Node, 0)),
                Placement::Last,
            )
            .unwrap();
    }
    assert_eq!(validate(&deep, &doc), Ok(()));
    deep.insert_child(
        at,
        Mutator::new(
            DocMutation::Node(NodeOp::SetHidden(true)),
            BranchKey::new(DocKind::Section, 0),
        ),
        Placement::Last,
    )
    .unwrap();
    match validate(&deep, &doc) {
        Err(MutationError::PayloadKind { at, .. }) => {
            assert_eq!(
                at.mutator.len(),
                DEEP,
                "the path runs from the root to the bottom"
            );
            assert!(at.mutator.iter().all(|&p| p == 0));
        }
        other => panic!("expected the payload at the bottom refused, got {other:?}"),
    }
}

/// A chain of `DEEP` nodes, each with its connector, as an insert's
/// subtree: built from the bottom up, so building it recurses on
/// nothing.
fn deep_subtree() -> Subtree<DocElement> {
    let mut chain = Subtree {
        root: DocElement::Node(node_data(0)),
        children: vec![Subtree::leaf(connector())],
    };
    for _ in 1..DEEP {
        chain = Subtree {
            root: DocElement::Node(node_data(0)),
            children: vec![Subtree::leaf(connector()), chain],
        };
    }
    chain
}

#[test]
fn test_document_validate_walks_a_deep_insert() {
    do_document_validate_walks_a_deep_insert();
}

/// An inserted subtree `DEEP` levels deep is validated on a default
/// test thread: `walk_subtree` iterates (§20 rule 11, R14.4). A refusal
/// at the bottom — the fallback font on the deepest node — is found and
/// named by its full path in the subtree. What it builds is dropped
/// here too: `Subtree`'s drop iterates (V1-A5, #282).
pub fn do_document_validate_walks_a_deep_insert() {
    let (doc, _) = document();
    let insert = |subtree| {
        command(
            DocKind::Node,
            StructOp::Insert {
                subtree,
                at: Placement::Last,
            },
        )
    };
    let fine = insert(deep_subtree());
    assert_eq!(validate(&fine, &doc), Ok(()));
    drop(fine);

    let mut bad = deep_subtree();
    let mut bottom = &mut bad;
    while bottom.children.len() > 1 {
        bottom = &mut bottom.children[1];
    }
    if let DocElement::Node(node) = &mut bottom.root {
        node.style.text.font = Some(crate::font::fonts::AppFont::Any);
    }
    let bad = insert(bad);
    match validate(&bad, &doc) {
        Err(MutationError::AnyFont { at }) => {
            let path = at.subtree.expect("inside the insert");
            assert_eq!(path.len(), DEEP - 1, "the path runs to the bottom node");
            assert!(path.iter().all(|&p| p == 1));
        }
        other => panic!("expected the bottom's font refused, got {other:?}"),
    }
}

#[test]
fn test_document_validate_applied_skips_removed_parents() {
    do_document_validate_applied_skips_removed_parents();
}

/// A request that removes `c`, then its parent `b`: `c` left `b`'s row,
/// but `b` has left the tree too, so its row is not judged — the
/// journal's touches say so (R14.5). Judged, the detached `b` would hold
/// a connector with no node above it and be refused. Control: removing
/// `c` alone judges `b`, which keeps the rules.
pub fn do_document_validate_applied_skips_removed_parents() {
    let (_, ids) = document();
    let remove = || command(DocKind::Node, StructOp::Remove);
    let (mut doc, _) = document();
    assert_eq!(
        applied(&mut doc, vec![(remove(), vec![ids.c]), (remove(), vec![ids.b])]),
        Ok(())
    );
    let (mut doc, _) = document();
    assert_eq!(applied(&mut doc, vec![(remove(), vec![ids.c])]), Ok(()));
}

#[test]
fn test_document_validate_holds_an_inserted_name_to_the_files_rule() {
    use crate::document::element::{PaletteData, ThemeData, ThemeVariant};
    use crate::document::mutation::validate::judge_element;
    use crate::document::place::NameFault;
    use crate::document::values::Rgba;
    let (doc, _) = document();
    let palette = |name: &str| {
        DocElement::Palette(PaletteData {
            name: name.to_string(),
            groups: vec![palette_group(1)],
        })
    };
    let theme = |variant: &str, variable: &str| {
        DocElement::Theme(ThemeData {
            variables: vec![variable.to_string()],
            variants: vec![ThemeVariant {
                name: variant.to_string(),
                values: vec![Rgba::opaque(0, 0, 0)],
            }],
            active: 0,
        })
    };
    // No edit reaches a name today: a theme or a palette goes only in
    // the map row, and inserting into it is refused before any element
    // is judged; no op renames one.
    let insert = |element: DocElement| {
        command(
            DocKind::Map,
            StructOp::Insert {
                subtree: Subtree::leaf(element),
                at: Placement::Last,
            },
        )
    };
    assert!(matches!(
        validate(&insert(palette("")), &doc),
        Err(MutationError::Structure {
            rule: StructureRule::MapRow,
            ..
        })
    ));
    // Where an inserted theme or palette is judged, its names are held
    // to the rule a file's are.
    let name = |result: Result<(), MutationError>| match result {
        Err(MutationError::Name { what, fault, .. }) => (what, fault),
        other => panic!("expected a name refusal, got {other:?}"),
    };
    assert_eq!(
        name(judge_element(&doc, &palette(""))),
        ("palette", NameFault::Empty)
    );
    assert_eq!(
        name(judge_element(&doc, &palette("a\u{202E}b"))),
        ("palette", NameFault::NotPrintable('\u{202E}'))
    );
    assert_eq!(
        name(judge_element(&doc, &theme("d\u{7}", "bg"))),
        ("theme variant", NameFault::NotPrintable('\u{7}'))
    );
    assert_eq!(
        name(judge_element(&doc, &theme("d", ""))),
        ("theme variable", NameFault::Empty)
    );
    // Control: names a file can hold pass.
    assert_eq!(judge_element(&doc, &palette("My Palette")), Ok(()));
    assert_eq!(judge_element(&doc, &theme("d", "bg")), Ok(()));
}
