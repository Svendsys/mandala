// SPDX-License-Identifier: MPL-2.0

//! `layers_equivalent`, the oracle's comparison, held to what it must
//! tell apart and what it must not (#241, Tests): each difference it
//! must catch is planted into one of two projections of the same
//! document, which it otherwise finds equal.

use glam::Vec2;

use crate::arena::NodeId;
use crate::document::tests::writer_support::built;
use crate::engine::anchor::Anchors;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutation, GfxMutator};
use crate::gfx_structs::tree::GfxMutatorTree;
use crate::projection::equivalent::{layers_equivalent, TOLERANCE};
use crate::projection::tests::support::fonts;
use crate::projection::{GlyphLayers, Layer, Projector};

/// The example map projected twice, into two independent sets of layers.
fn twice() -> (GlyphLayers, GlyphLayers, Projector) {
    let (doc, _) = built();
    let mut fonts = fonts();
    let mut a = GlyphLayers::new();
    let mut b = GlyphLayers::new();
    let projector = Projector::project_full(&doc, &mut a, &mut fonts);
    Projector::project_full(&doc, &mut b, &mut fonts);
    (a, b, projector)
}

/// The first run of `layer`.
fn first_run(layers: &GlyphLayers, layer: Layer) -> NodeId {
    let tree = layers.tree(layer);
    tree.root()
        .descendants(tree.arena())
        .find(|&g| tree.arena()[g].get().glyph_run().is_some())
        .expect("the layer has a run")
}

/// Apply `mutation` to `glyph` of `layer`.
fn mutate(layers: &mut GlyphLayers, layer: Layer, glyph: NodeId, mutation: GfxMutation) {
    let mutator = GfxMutatorTree::new_with(GfxMutator::new(mutation, 0));
    mutator
        .apply_at(layers.tree_mut(layer), &Anchors::Nodes(vec![glyph]), |_, _| ())
        .unwrap();
}

#[test]
fn test_layers_equivalent_finds_two_projections_of_one_document_equal() {
    let (a, b, _) = twice();
    assert!(layers_equivalent(&a, &b).is_ok(), "{:?}", layers_equivalent(&a, &b));
    // Precondition: the two shapings are distinct `Arc`s, so equality is
    // by value — what the comparison is for.
    let ra = a.sections.arena()[first_run(&a, Layer::Sections)].get().glyph_run().unwrap().clone();
    let rb = b.sections.arena()[first_run(&b, Layer::Sections)].get().glyph_run().unwrap().clone();
    assert!(!ra.same_state(&rb), "the shapings are not the same Arcs");
}

#[test]
fn test_layers_equivalent_tells_a_moved_run_within_and_past_the_tolerance() {
    let (a, mut b, _) = twice();
    let run = first_run(&b, Layer::Sections);
    let origin = b.sections.arena()[run].get().glyph_run().unwrap().origin();
    mutate(&mut b, Layer::Sections, run, GfxMutation::RunOrigin(origin + Vec2::splat(TOLERANCE * 0.1)));
    assert!(layers_equivalent(&a, &b).is_ok(), "a shift well inside the tolerance is the same placement");
    mutate(&mut b, Layer::Sections, run, GfxMutation::RunOrigin(origin + Vec2::new(0.01, 0.0)));
    let difference = layers_equivalent(&a, &b).unwrap_err();
    assert_eq!(difference.layer, Layer::Sections);
    assert!(difference.what.starts_with("origin"), "{difference}");
}

#[test]
fn test_layers_equivalent_tells_opacity_and_overrides_apart() {
    let (a, mut b, _) = twice();
    let run = first_run(&b, Layer::Connections);
    mutate(&mut b, Layer::Connections, run, GfxMutation::RunOpacity(0.25));
    let difference = layers_equivalent(&a, &b).unwrap_err();
    assert!(difference.what.starts_with("opacity"), "{difference}");
    let (a, mut b, _) = twice();
    let run = first_run(&b, Layer::Frames);
    mutate(
        &mut b,
        Layer::Frames,
        run,
        GfxMutation::RunOverrides(crate::gfx_structs::run::RunOverrideOp::Scale { range: 0..1, value: 2.0 }),
    );
    assert_eq!(layers_equivalent(&a, &b).unwrap_err().what, "overrides");
}

#[test]
fn test_layers_equivalent_tells_another_text_apart() {
    let (a, _, _) = twice();
    let (mut doc, ids) = built();
    doc.apply(
        crate::document::tests::writer_support::request(
            crate::document::tests::writer_support::single(
                crate::document::element::DocKind::Span,
                crate::document::mutation::ops::DocMutation::Span(crate::document::mutation::ops::SpanOp::SetText(
                    "titles".into(),
                )),
            ),
            vec![ids.a_title_span],
            "text",
        ),
        crate::document::writer::ChangeSource::Action,
    )
    .unwrap();
    let mut b = GlyphLayers::new();
    Projector::project_full(&doc, &mut b, &mut fonts());
    let difference = layers_equivalent(&a, &b).unwrap_err();
    assert_eq!(difference.layer, Layer::Sections, "{difference}");
}

#[test]
fn test_layers_equivalent_tells_another_tree_shape_apart() {
    let (a, mut b, _) = twice();
    let root = b.labels.root();
    b.labels
        .insert_child(root, GfxElement::new_void(crate::projection::CHILD_GROUP_CHANNEL), crate::engine::tree::Placement::Last)
        .unwrap();
    let difference = layers_equivalent(&a, &b).unwrap_err();
    assert_eq!(difference.layer, Layer::Labels, "{difference}");
    let (a, mut b, _) = twice();
    let run = first_run(&b, Layer::Portals);
    b.portals
        .update(run, |e| {
            if let GfxElement::GlyphRun { channel, .. } = e {
                *channel = 7;
            }
        })
        .unwrap();
    // A run moved to another channel sorts among its siblings by it, so
    // the walk meets the difference where the rows part.
    assert_eq!(layers_equivalent(&a, &b).unwrap_err().layer, Layer::Portals);
}

#[test]
fn test_layers_equivalent_ignores_unique_ids() {
    let (a, mut b, _) = twice();
    let run = first_run(&b, Layer::Sections);
    b.sections.update(run, |e| e.set_unique_id(99)).unwrap();
    assert_eq!(b.sections.arena()[run].get().unique_id(), 99, "the id changed");
    assert!(layers_equivalent(&a, &b).is_ok(), "a unique id is not compared");
}
