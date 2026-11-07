// SPDX-License-Identifier: MPL-2.0

//! The layer renderer's render state, pinned without a device
//! (TEST_CONVENTIONS §T8's renderer-free tier): what `load` keeps, what
//! `apply` visits and writes for the journals it is handed, and that the
//! state it keeps up to date is always the state a fresh `load` of the
//! same layers would make. The frame half — `encode` through the
//! positioned-glyph pass, zoom windows, the camera, the atlas — is pinned
//! by `app::tests_gpu_layers`.
//!
//! Documents are the format's fixtures, the demo maps and the
//! projection's generated corpus, projected by the real projector
//! (`baumhard::projection`); every edit goes through `Document::apply`.
//! What a test compares the render state with is read from the layers
//! themselves — their trees, before and after — and never from the
//! journals `apply` reads, except where the subject is the visit itself.

use std::path::PathBuf;
use std::sync::Arc;

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::format::load;
use baumhard::document::ids::{Document, NodeIdent};
use baumhard::document::mutation::ops::{DocMutation, DocMutatorTree, NodeOp, SpanOp};
use baumhard::document::mutation::step::{DocStep, DocWalk};
use baumhard::document::writer::{ChangeSource, EditRequest};
use baumhard::engine::anchor::Anchors;
use baumhard::engine::journal::JournalBuilder;
use baumhard::engine::key::BranchKey;
use baumhard::engine::mutation::Mutation;
use baumhard::engine::mutator::Mutator;
use baumhard::gfx_structs::mutator::GfxMutation;
use baumhard::gfx_structs::run::{GlyphRunData, RunShape};
use baumhard::projection::tests::corpus::{edit, generated, of_kind};
use baumhard::projection::tests::maps::kids;
use baumhard::projection::tests::support::Projected;
use baumhard::projection::{GlyphLayers, Layer, LayerChanges};
use baumhard::util::tests::choices::Choices;
use glam::Vec2;
use rustc_hash::{FxHashMap, FxHashSet};

use super::{ApplyLog, LayerRenderer};

/// The repository's root.
fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

/// The format's fixtures and the demo maps, by path, sorted.
fn maps() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in ["lib/baumhard/src/document/format/fixtures", "maps/demos"] {
        let mut found: Vec<PathBuf> = std::fs::read_dir(repo(dir))
            .unwrap_or_else(|e| panic!("{dir}: {e}"))
            .map(|entry| entry.expect("a directory entry").path())
            .filter(|path| path.to_string_lossy().ends_with(".mandala.json"))
            .collect();
        found.sort();
        out.extend(found);
    }
    assert!(
        out.len() >= 9,
        "the fixtures and the demo maps are found: {out:?}"
    );
    out
}

/// The map at `path`, loaded.
fn document(path: &std::path::Path) -> Document {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    load(&text).unwrap_or_else(|e| panic!("{} loads: {e}", path.display()))
}

/// The demo map `name`, projected: one whose nodes are all drawn, each
/// with sections, a frame and, below the root, a connector.
fn demo(name: &str) -> Projected {
    Projected::new(document(&repo(&format!("maps/demos/{name}.mandala.json"))))
}

/// A renderer that has loaded `layers`.
fn loaded(layers: &GlyphLayers) -> LayerRenderer {
    let mut renderer = LayerRenderer::default();
    renderer.load(layers);
    renderer
}

/// Every element the journals of `changes` name, each once, in the order
/// a layer-by-layer reading of them first names it: what an apply of
/// them alone must visit, read from the journals directly.
fn touched(changes: &LayerChanges) -> Vec<(Layer, NodeId)> {
    let mut seen = FxHashSet::default();
    let mut out = Vec::new();
    for layer in Layer::ALL {
        for (id, _) in changes.journal(layer).touched() {
            if seen.insert((layer, id)) {
                out.push((layer, id));
            }
        }
    }
    out
}

/// Every run of every layer, by layer and glyph id, read from the trees.
fn runs_of(layers: &GlyphLayers) -> FxHashMap<(Layer, NodeId), GlyphRunData> {
    let mut out = FxHashMap::default();
    for layer in Layer::ALL {
        let tree = layers.tree(layer);
        for id in tree.root().descendants(tree.arena()) {
            if let Some(run) = tree.get(id).and_then(|node| node.get().glyph_run()) {
                out.insert((layer, id), run.clone());
            }
        }
    }
    out
}

/// The identity of what `run` was shaped into: the shaped box, or the
/// shaped run a placement refers to.
fn shaping(run: &GlyphRunData) -> *const () {
    match run.shaped() {
        RunShape::Box(shaped) => Arc::as_ptr(shaped).cast(),
        RunShape::Path { run, .. } => Arc::as_ptr(run).cast(),
    }
}

/// A request of one `mutation` of the element `target`, of `kind`.
fn edit_of(kind: DocKind, mutation: DocMutation, target: NodeId) -> EditRequest {
    let mutator = DocMutatorTree::new_with(Mutator::new(mutation, BranchKey::new(kind, 0)));
    EditRequest::new(
        vec![DocStep::fixed(DocWalk::new(
            mutator,
            Anchors::Nodes(vec![target]),
        ))],
        "renderer::layers::tests",
    )
}

/// A request of one `op` of the node `node`.
fn node_edit(node: NodeId, op: NodeOp) -> EditRequest {
    edit_of(DocKind::Node, DocMutation::Node(op), node)
}

/// A request setting the text of `node`'s first section's first span to
/// `text`: an edit its section must be shaped again for.
fn retext(doc: &Document, node: NodeId, text: &str) -> EditRequest {
    let section = kids(doc, node, DocKind::Section)[0];
    let span = kids(doc, section, DocKind::Span)[0];
    edit_of(
        DocKind::Span,
        DocMutation::Span(SpanOp::SetText(text.into())),
        span,
    )
}

/// The node `ident` of `doc`.
fn node(doc: &Document, ident: u32) -> NodeId {
    doc.node_by_ident(NodeIdent(ident))
        .expect("the fixture has the node")
}

/// Apply the projector's update to `renderer` and check it: the render
/// state is a fresh load's, and the visit was exactly the journals'.
fn apply_and_check(
    renderer: &mut LayerRenderer,
    p: &Projected,
    changes: &LayerChanges,
) -> Result<(), String> {
    renderer.apply(&p.layers, LayerRenderer::journals_of(changes));
    renderer.matches(&p.layers)?;
    let (want, have) = (touched(changes), &renderer.log().visited);
    if &want != have {
        return Err(format!("the journals name {want:?}, the apply visited {have:?}"));
    }
    Ok(())
}

#[test]
fn test_layers_load_holds_every_layer_in_its_pre_order() {
    let mut per_layer = [0usize; 5];
    for path in maps() {
        let p = Projected::new(document(&path));
        let renderer = loaded(&p.layers);
        renderer
            .matches(&p.layers)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        // The draw order, read from the trees: each layer in turn, in
        // `Layer::ALL`'s order, each in its pre-order.
        let want: Vec<&GlyphRunData> = Layer::ALL
            .iter()
            .flat_map(|&layer| {
                let tree = p.layers.tree(layer);
                tree.root()
                    .descendants(tree.arena())
                    .filter_map(move |id| tree.get(id).and_then(|node| node.get().glyph_run()))
            })
            .collect();
        for (count, layer) in per_layer.iter_mut().zip(Layer::ALL) {
            *count += runs_of(&p.layers).keys().filter(|(l, _)| *l == layer).count();
        }
        let have: Vec<&GlyphRunData> = renderer.runs().collect();
        assert_eq!(
            have.len(),
            want.len(),
            "{}: every run is drawn once",
            path.display()
        );
        for (i, (a, b)) in have.iter().zip(&want).enumerate() {
            assert!(
                a.same_state(b),
                "{}: run {i} of the draw order is not the layers' run {i}",
                path.display()
            );
        }
    }
    assert!(
        per_layer.iter().all(|&count| count > 0),
        "precondition: the maps draw runs in every layer: {per_layer:?}"
    );
}

/// How many generated documents the sweep takes, and how many edits each.
const DOCUMENTS: u64 = 12;
const STEPS: usize = 24;

#[test]
fn test_layers_apply_keeps_the_render_state_a_fresh_load_through_the_corpus() {
    let mut total = ApplyLog::default();
    for seed in 0..DOCUMENTS {
        let mut c = Choices::seeded(0xC2_0000 + seed);
        let doc = generated(&mut c);
        assert!(
            of_kind(&doc, DocKind::Node).len() >= 50,
            "seed {seed}: a corpus document"
        );
        let mut p = Projected::new(doc);
        let mut renderer = loaded(&p.layers);
        for step in 0..STEPS {
            let what = if step % 7 == 6 {
                if step % 2 == 0 {
                    p.doc.undo().map(|_| "undo")
                } else {
                    p.doc.redo().map(|_| "redo")
                }
            } else {
                edit(&mut c, &p.doc)
                    .and_then(|(name, req)| p.doc.apply(req, ChangeSource::Action).ok().map(|_| name))
            };
            let Some(what) = what else {
                continue;
            };
            let changes = p.sync();
            apply_and_check(&mut renderer, &p, &changes)
                .unwrap_or_else(|e| panic!("seed {seed} step {step} ({what}): {e}"));
            let log = renderer.log();
            total.inserted += log.inserted;
            total.freed += log.freed;
            total.relinked += log.relinked;
            total.reshaped += log.reshaped;
            total.replaced += log.replaced;
            total.moved += log.moved;
            total.restyled += log.restyled;
            changes.discard(&mut p.layers);
        }
    }
    // Every way an apply writes the state was taken, so no path of it
    // went unchecked against a fresh load.
    for (what, count) in [
        ("inserted", total.inserted),
        ("freed", total.freed),
        ("relinked", total.relinked),
        ("reshaped", total.reshaped),
        ("replaced", total.replaced),
        ("moved", total.moved),
        ("restyled", total.restyled),
    ] {
        assert!(count > 0, "the sweep never {what} a slot or a run: {total:?}");
    }
}

#[test]
fn test_layers_one_edit_rewrites_exactly_the_runs_it_changed() {
    let mut p = demo("ecclesiastes");
    let mut renderer = loaded(&p.layers);
    let before = runs_of(&p.layers);
    let stamps: FxHashMap<(Layer, NodeId), u64> = before
        .keys()
        .map(|&(layer, id)| {
            (
                (layer, id),
                renderer.state(layer, id).expect("a loaded run").stamp(),
            )
        })
        .collect();
    let last = stamps.values().copied().max().expect("the fixture draws runs");
    // One node's text, rewritten: its section reshapes, its frame and the
    // paths at it move with its size, and nothing else changes.
    let target = node(&p.doc, 2);
    p.doc
        .apply(
            retext(&p.doc, target, "a longer line than the one it replaces"),
            ChangeSource::Action,
        )
        .expect("the edit applies");
    let changes = p.sync();
    apply_and_check(&mut renderer, &p, &changes).unwrap();
    let after = runs_of(&p.layers);
    let (mut kept, mut written) = (0, 0);
    for (&(layer, id), run) in &after {
        let stamp = renderer.state(layer, id).expect("every run is kept").stamp();
        match before.get(&(layer, id)) {
            Some(old) if old.same_state(run) => {
                assert_eq!(
                    stamp,
                    stamps[&(layer, id)],
                    "{layer:?} {id:?} did not change, and its state was written"
                );
                kept += 1;
            }
            Some(_) => {
                assert_ne!(
                    stamp,
                    stamps[&(layer, id)],
                    "{layer:?} {id:?} changed, and its state was not"
                );
                written += 1;
            }
            None => assert!(stamp > last, "{layer:?} {id:?} entered, and holds an old state"),
        }
    }
    assert!(
        kept > 0 && written > 0,
        "precondition: the edit changes some runs and leaves others ({written} changed, {kept} kept)"
    );
    changes.discard(&mut p.layers);
}

#[test]
fn test_layers_a_drag_re_places_every_run_and_reshapes_none() {
    let mut p = demo("ecclesiastes");
    let mut renderer = loaded(&p.layers);
    let shapings: FxHashMap<(Layer, NodeId), *const ()> = runs_of(&p.layers)
        .iter()
        .map(|(&key, run)| (key, shaping(run)))
        .collect();
    let target = node(&p.doc, 2);
    p.doc
        .apply(
            node_edit(target, NodeOp::Nudge(Vec2::new(40.0, -25.0))),
            ChangeSource::Action,
        )
        .expect("the drag applies");
    let changes = p.sync();
    apply_and_check(&mut renderer, &p, &changes).unwrap();
    let log = renderer.log().clone();
    assert!(
        log.moved + log.replaced > 0,
        "precondition: the drag moved runs: {log:?}"
    );
    assert_eq!(log.reshaped, 0, "a drag adopted a new shaping: {log:?}");
    for ((layer, id), run) in runs_of(&p.layers) {
        let kept = renderer.state(layer, id).expect("every run is kept").run();
        assert_eq!(
            shaping(kept),
            shapings[&(layer, id)],
            "{layer:?} {id:?}: the drag's state holds another shaping"
        );
        assert_eq!(
            shaping(kept),
            shaping(&run),
            "{layer:?} {id:?}: the state is the layer's"
        );
    }
    changes.discard(&mut p.layers);

    // The control: a text change does reshape, and the state adopts the
    // projector's new shaping.
    p.doc
        .apply(retext(&p.doc, target, "x"), ChangeSource::Action)
        .expect("the text change applies");
    let changes = p.sync();
    apply_and_check(&mut renderer, &p, &changes).unwrap();
    assert!(
        renderer.log().reshaped > 0,
        "a text change reshaped nothing: {:?}",
        renderer.log()
    );
    changes.discard(&mut p.layers);
}

#[test]
fn test_layers_a_merged_journal_set_visits_each_element_once() {
    let mut p = demo("ecclesiastes");
    let mut renderer = loaded(&p.layers);
    let target = node(&p.doc, 2);
    p.doc
        .apply(retext(&p.doc, target, "another line"), ChangeSource::Action)
        .expect("the edit applies");
    let changes = p.sync();
    let projected_touches: FxHashSet<(Layer, NodeId)> = touched(&changes).into_iter().collect();
    // A view-only glyph mutator over the update, as the selection tint
    // will be: one section run the projector rewrote, and one it did not.
    let tree = &p.layers.sections;
    let sections: Vec<NodeId> = tree
        .root()
        .descendants(tree.arena())
        .filter(|&id| tree.get(id).is_some_and(|node| node.get().glyph_run().is_some()))
        .collect();
    let both = *sections
        .iter()
        .find(|&&id| projected_touches.contains(&(Layer::Sections, id)))
        .expect("precondition: the edit rewrote a section run");
    let only = *sections
        .iter()
        .find(|&&id| !projected_touches.contains(&(Layer::Sections, id)))
        .expect("precondition: the edit left a section run alone");
    let mut overlay = JournalBuilder::new();
    for id in [both, only] {
        p.layers
            .sections
            .update_journaled(id, &mut overlay, |element| {
                GfxMutation::RunOpacity(0.25).apply_to(element, &());
            })
            .expect("the run is in the layer");
    }
    let overlay = overlay.close(&p.layers.sections);
    renderer.apply(
        &p.layers,
        LayerRenderer::journals_of(&changes).chain([(Layer::Sections, &overlay)]),
    );
    renderer.matches(&p.layers).unwrap();
    let visited = &renderer.log().visited;
    for id in [both, only] {
        assert_eq!(
            visited.iter().filter(|&&v| v == (Layer::Sections, id)).count(),
            1,
            "{id:?} is visited once however many journals name it"
        );
        assert_eq!(
            renderer
                .state(Layer::Sections, id)
                .expect("a run")
                .run()
                .opacity(),
            0.25,
            "{id:?}: the overlay's write is what the state holds"
        );
    }
    assert_eq!(
        visited.len(),
        projected_touches.len() + 1,
        "the visit is the projector's touches and the overlay's one other run"
    );
    overlay.discard(&mut p.layers.sections);
    changes.discard(&mut p.layers);
}
