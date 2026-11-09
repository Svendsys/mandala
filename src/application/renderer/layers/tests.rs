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
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{DocMutation, DocMutatorTree, NodeOp, SpanOp};
use baumhard::document::mutation::step::{DocStep, DocWalk};
use baumhard::document::values::Rgba;
use baumhard::document::writer::{ChangeSource, EditRequest};
use baumhard::engine::anchor::Anchors;
use baumhard::engine::journal::JournalBuilder;
use baumhard::engine::key::BranchKey;
use baumhard::engine::mutation::Mutation;
use baumhard::engine::mutator::Mutator;
use baumhard::gfx_structs::mutator::GfxMutation;
use baumhard::gfx_structs::run::{GlyphRunData, RunOverrideOp, RunShape};
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

/// Every element the journals of `batch` — one or more projector
/// updates, oldest first — name, each once, in the order a
/// layer-by-layer reading of them first names it: what one apply of them
/// all must visit, read from the journals directly.
fn touched(batch: &[LayerChanges]) -> Vec<(Layer, NodeId)> {
    let mut seen = FxHashSet::default();
    let mut out = Vec::new();
    for layer in Layer::ALL {
        for changes in batch {
            for (id, _) in changes.journal(layer).touched() {
                if seen.insert((layer, id)) {
                    out.push((layer, id));
                }
            }
        }
    }
    out
}

/// The journals of every update of `batch`, oldest first, each with its
/// layer: what one [`LayerRenderer::apply`] takes for several projector
/// updates made since the last one.
fn merged(
    batch: &[LayerChanges],
) -> impl Iterator<
    Item = (
        Layer,
        &baumhard::engine::journal::Journal<baumhard::gfx_structs::element::GfxElement>,
    ),
> {
    batch.iter().flat_map(LayerRenderer::journals_of)
}

/// Discard the journals of `batch`, newest first: the order a history
/// lets go of the updates it holds.
fn discard(batch: Vec<LayerChanges>, layers: &mut GlyphLayers) {
    for changes in batch.into_iter().rev() {
        changes.discard(layers);
    }
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

/// Apply the projector's updates `batch`, oldest first, to `renderer` in
/// one call, and check it: the render state is a fresh load's, and the
/// visit was exactly the journals'.
fn apply_and_check(
    renderer: &mut LayerRenderer,
    p: &Projected,
    batch: &[LayerChanges],
) -> Result<(), String> {
    renderer.apply(&p.layers, merged(batch));
    renderer.matches(&p.layers)?;
    let (want, have) = (touched(batch), &renderer.log().visited);
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

/// How many generated documents the sweep takes, how many applies each,
/// and at most how many projector updates one apply takes.
const DOCUMENTS: u64 = 12;
const STEPS: usize = 24;
const MERGED: u32 = 4;

#[test]
fn test_layers_apply_keeps_the_render_state_a_fresh_load_through_the_corpus() {
    let mut total = ApplyLog::default();
    let mut most = 0;
    let mut composites = [0usize; 2];
    for seed in 0..DOCUMENTS {
        let mut c = Choices::seeded(0xC2_0000 + seed);
        let doc = generated(&mut c);
        assert!(
            of_kind(&doc, DocKind::Node).len() >= 50,
            "seed {seed}: a corpus document"
        );
        let mut p = Projected::new(doc);
        let mut renderer = loaded(&p.layers);
        let mut edits = 0usize;
        for step in 0..STEPS {
            // One to `MERGED` projector updates, each its own `sync`, all
            // handed to one apply: what a frame that saw several changes
            // gives it, and the only way journals of different updates
            // meet in one apply — an element moved in one and its old
            // parent removed in the next, a row reordered twice.
            // One step in four is one of the composites the corpus's
            // single edits never make (see `composite`).
            let planned = if c.one_in(4) {
                composite(&mut c, &p.doc)
            } else {
                None
            };
            let count = match &planned {
                Some((_, steps)) => steps.len(),
                None => c.below(MERGED) as usize + 1,
            };
            let mut batch = Vec::new();
            let mut names = Vec::new();
            for k in 0..count {
                edits += 1;
                let what = if let Some((name, steps)) = &planned {
                    let req = steps[k](&p.doc);
                    p.doc.apply(req, ChangeSource::Action).ok().map(|_| *name)
                } else if edits % 7 == 6 {
                    if edits.is_multiple_of(2) {
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
                names.push(what);
                batch.push(p.sync());
            }
            if batch.is_empty() {
                continue;
            }
            most = most.max(batch.len());
            if let Some((name, steps)) = &planned {
                if batch.len() == steps.len() {
                    composites[usize::from(*name == MOVED_OUT)] += 1;
                }
            }
            apply_and_check(&mut renderer, &p, &batch)
                .unwrap_or_else(|e| panic!("seed {seed} step {step} ({names:?}): {e}"));
            let log = renderer.log();
            total.inserted += log.inserted;
            total.freed += log.freed;
            total.relinked += log.relinked;
            total.reshaped += log.reshaped;
            total.replaced += log.replaced;
            total.moved += log.moved;
            total.restyled += log.restyled;
            discard(batch, &mut p.layers);
        }
    }
    assert_eq!(
        most, MERGED as usize,
        "precondition: some apply took {MERGED} updates at once"
    );
    assert!(
        composites.iter().all(|&n| n > 0),
        "precondition: the sweep made both composites: {composites:?}"
    );
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
    apply_and_check(&mut renderer, &p, std::slice::from_ref(&changes)).unwrap();
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
    apply_and_check(&mut renderer, &p, std::slice::from_ref(&changes)).unwrap();
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
    apply_and_check(&mut renderer, &p, std::slice::from_ref(&changes)).unwrap();
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
    let projected_touches: FxHashSet<(Layer, NodeId)> =
        touched(std::slice::from_ref(&changes)).into_iter().collect();
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

/// Run `edits` on `p`, one projector update each, and hand all the
/// updates to `renderer` in one apply; then the same for undoing them
/// all, newest first, and for redoing them all. Each apply is checked
/// against a fresh load and against the journals' visit.
///
/// What `edits` builds is read from the document as it stands when the
/// edit runs, so an edit may name what an earlier one moved.
fn merged_round_trip(renderer: &mut LayerRenderer, p: &mut Projected, edits: &[Step]) -> Result<(), String> {
    let mut batch = Vec::new();
    for (i, edit) in edits.iter().enumerate() {
        let req = edit(&p.doc);
        p.doc
            .apply(req, ChangeSource::Action)
            .map_err(|e| format!("edit {i} applies: {e:?}"))?;
        batch.push(p.sync());
    }
    apply_and_check(renderer, p, &batch).map_err(|e| format!("the edits, merged: {e}"))?;
    discard(batch, &mut p.layers);
    for (pass, undo) in [("undo", true), ("redo", false)] {
        let mut batch = Vec::new();
        for i in 0..edits.len() {
            let done = if undo { p.doc.undo() } else { p.doc.redo() };
            done.ok_or_else(|| format!("{pass} {i} has an entry"))?;
            batch.push(p.sync());
        }
        apply_and_check(renderer, p, &batch).map_err(|e| format!("every {pass}, merged: {e}"))?;
        discard(batch, &mut p.layers);
    }
    Ok(())
}

/// A request moving `node` under `to`, after its other child nodes.
fn reparent(doc: &Document, node: NodeId, to: NodeId) -> EditRequest {
    EditRequest::new(
        compose::reparent(doc, &[node], to).expect("a node, moved under a node"),
        "compose::reparent",
    )
}

/// One edit of a composite, built against the document as it stands
/// when it runs.
type Step = Box<dyn Fn(&Document) -> EditRequest>;

/// The names of the two composites.
const REORDERED: &str = "row reordered twice";
const MOVED_OUT: &str = "moved out, then its parent removed";

/// Three edits of the row of `parent`'s child nodes in which `p` is
/// right before `x`: `x` to the end of its own row, then `p`, then `x`
/// again. The row ends `[…, p, x]`: `x` follows `p` before and after,
/// while `p` itself moves — past the siblings after `x`, when it has
/// any.
fn row_reordered_twice(parent: NodeId, p: NodeId, x: NodeId) -> Vec<Step> {
    vec![
        Box::new(move |doc: &Document| reparent(doc, x, parent)),
        Box::new(move |doc: &Document| reparent(doc, p, parent)),
        Box::new(move |doc: &Document| reparent(doc, x, parent)),
    ]
}

/// Two edits of `child`, under `parent` under `grand`: `child` moved under
/// `grand`, then `parent` deleted. The second edit's journal names
/// `parent`'s subtree as it then stands, without `child`.
fn moved_out_then_removed(grand: NodeId, parent: NodeId, child: NodeId) -> Vec<Step> {
    vec![
        Box::new(move |doc: &Document| reparent(doc, child, grand)),
        Box::new(move |doc: &Document| {
            EditRequest::new(
                compose::delete(doc, &[parent]).expect("a node"),
                "compose::delete",
            )
        }),
    ]
}

/// Every node of `doc` with a parent node and a child node, as
/// `(grand, parent, child)`: what [`moved_out_then_removed`] takes.
fn three_deep(doc: &Document) -> Vec<(NodeId, NodeId, NodeId)> {
    of_kind(doc, DocKind::Node)
        .into_iter()
        .filter_map(|n| {
            let grand = doc.parent(n)?;
            let child = *kids(doc, n, DocKind::Node).first()?;
            (doc.get(grand)?.kind() == DocKind::Node).then_some((grand, n, child))
        })
        .collect()
}

/// One of the two composites, its nodes drawn from `c`, or `None` when
/// `doc` has nowhere to make it: the edit sequences whose journals, met
/// in one apply, the single edits of the corpus never produce — a single
/// update reprojects from its final state — and which a renderer must
/// hold to (#319's review).
fn composite(c: &mut Choices, doc: &Document) -> Option<(&'static str, Vec<Step>)> {
    if c.one_in(2) {
        let rows: Vec<NodeId> = of_kind(doc, DocKind::Node)
            .into_iter()
            .filter(|&n| kids(doc, n, DocKind::Node).len() >= 3)
            .collect();
        let parent = *rows.get(c.below(rows.len().max(1) as u32) as usize)?;
        let row = kids(doc, parent, DocKind::Node);
        // A sibling after `x`, so the row's order changes.
        let at = c.below(row.len() as u32 - 2) as usize;
        Some((REORDERED, row_reordered_twice(parent, row[at], row[at + 1])))
    } else {
        let found = three_deep(doc);
        let &(grand, parent, child) = found.get(c.below(found.len().max(1) as u32) as usize)?;
        Some((MOVED_OUT, moved_out_then_removed(grand, parent, child)))
    }
}

#[test]
fn test_layers_a_row_reordered_twice_in_one_apply_is_drawn_in_its_final_order() {
    // A row `[A, P, X, B, …]` of one parent's child nodes, and three
    // updates merged: X to the end of its own row, then P, then X again.
    // The row ends `[A, B, …, P, X]`: X's previous sibling is P before the
    // apply and after it, while P itself moves — a slot judged against a
    // neighbor that moves in the same apply.
    for seed in 0..DOCUMENTS {
        let mut c = Choices::seeded(0xC2_0000 + seed);
        let mut p = Projected::new(generated(&mut c));
        let parent = *of_kind(&p.doc, DocKind::Node)
            .iter()
            .find(|&&n| kids(&p.doc, n, DocKind::Node).len() >= 4)
            .unwrap_or_else(|| panic!("precondition: seed {seed} has a node with four child nodes"));
        let row = kids(&p.doc, parent, DocKind::Node);
        let (moved_p, moved_x) = (row[1], row[2]);
        let mut renderer = loaded(&p.layers);
        merged_round_trip(
            &mut renderer,
            &mut p,
            &row_reordered_twice(parent, moved_p, moved_x),
        )
        .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let row = kids(&p.doc, parent, DocKind::Node);
        assert_eq!(
            &row[row.len() - 2..],
            &[moved_p, moved_x],
            "precondition: seed {seed}'s row ends with P, then X"
        );
    }
}

#[test]
fn test_layers_a_node_moved_out_of_a_subtree_removed_in_the_same_apply_keeps_its_own() {
    // A node C under P under G, and two updates merged: C moved under G,
    // then P deleted. The second update's journal names P's subtree as it
    // then stood, without C; the renderer's state of P, from before both,
    // still holds C and everything under it.
    for seed in 0..DOCUMENTS {
        let mut c = Choices::seeded(0xC2_0000 + seed);
        let mut p = Projected::new(generated(&mut c));
        let (grand, parent, child) = *three_deep(&p.doc)
            .first()
            .unwrap_or_else(|| panic!("precondition: seed {seed} has a node three deep"));
        let mut renderer = loaded(&p.layers);
        merged_round_trip(
            &mut renderer,
            &mut p,
            &moved_out_then_removed(grand, parent, child),
        )
        .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert_eq!(
            p.doc.parent(child),
            Some(grand),
            "precondition: seed {seed}'s C ends under G"
        );
    }
}

#[test]
fn test_layers_apply_returns_on_an_incomplete_journal_set() {
    // One update the renderer never sees — a child node created — and
    // then one it does, creating two more after it: the second's journals
    // name a row whose first new member follows an element the renderer
    // holds no slot for. The apply must give those up, warn and return,
    // not wait on them. On a thread of its own, so a splice that never
    // ends fails this test with its own message instead of hanging the
    // suite (the layers are not `Send`, so the whole case runs there).
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut p = demo("ecclesiastes");
        let mut renderer = loaded(&p.layers);
        let parent = node(&p.doc, 2);
        let child =
            |doc: &Document| compose::create_child(doc, parent, Vec2::new(30.0, 40.0)).expect("a node");
        p.doc
            .apply(
                EditRequest::new(child(&p.doc), "renderer::layers::tests"),
                ChangeSource::Action,
            )
            .expect("the first child is created");
        let unseen = p.sync();
        unseen.discard(&mut p.layers);
        let mut steps = child(&p.doc);
        steps.extend(child(&p.doc));
        p.doc
            .apply(
                EditRequest::new(steps, "renderer::layers::tests"),
                ChangeSource::Action,
            )
            .expect("two more children are created");
        let seen = p.sync();
        renderer.apply(&p.layers, LayerRenderer::journals_of(&seen));
        let _ = done.send(renderer.log().clone());
        seen.discard(&mut p.layers);
    });
    let log = match wait.recv_timeout(std::time::Duration::from_secs(60)) {
        Ok(log) => log,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("apply did not return within 60 s on an incomplete journal set")
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            panic!("the case panicked before the apply returned (its message is above)")
        }
    };
    assert!(
        log.given_up > 0,
        "precondition: the apply met a row it could not complete: {log:?}"
    );
}

/// The addresses of `run`'s three override vectors.
fn override_buffers(run: &GlyphRunData) -> [usize; 3] {
    let o = run.overrides();
    [
        o.offset().as_ptr() as usize,
        o.color().as_ptr() as usize,
        o.scale().as_ptr() as usize,
    ]
}

#[test]
fn test_layers_a_run_restyled_again_keeps_its_override_buffers() {
    // A per-glyph overlay over one section run, twice: the first gives
    // the run overrides, and the state takes vectors of its own; the
    // second rewrites them at the same length, and the state's write is
    // into those vectors, not into new ones — an overlay or an animation
    // that touches a run every frame allocates nothing for it.
    let mut p = demo("ecclesiastes");
    let mut renderer = loaded(&p.layers);
    let tree = &p.layers.sections;
    let run = tree
        .root()
        .descendants(tree.arena())
        .find(|&id| {
            tree.get(id)
                .and_then(|node| node.get().glyph_run())
                .is_some_and(|run| run.shaped().glyph_count() >= 2 && run.overrides().is_empty())
        })
        .expect("precondition: a section run of two glyphs or more, with no overrides");
    let mut buffers = Vec::new();
    for op in [
        RunOverrideOp::Offset {
            range: 0..1,
            value: Vec2::new(0.0, -3.0),
        },
        RunOverrideOp::Color {
            range: 1..2,
            value: Some(Rgba::new(200, 40, 40, 255)),
        },
    ] {
        let mut overlay = JournalBuilder::new();
        p.layers
            .sections
            .update_journaled(run, &mut overlay, |element| {
                GfxMutation::RunOverrides(op.clone()).apply_to(element, &());
            })
            .expect("the run is in the layer");
        let overlay = overlay.close(&p.layers.sections);
        renderer.apply(&p.layers, [(Layer::Sections, &overlay)]);
        renderer.matches(&p.layers).unwrap();
        let kept = renderer.state(Layer::Sections, run).expect("a run").run();
        assert!(!kept.overrides().is_empty(), "precondition: {op:?} set overrides");
        buffers.push(override_buffers(kept));
        overlay.discard(&mut p.layers.sections);
    }
    assert_eq!(
        renderer.log().restyled,
        1,
        "precondition: the second overlay restyled the run: {:?}",
        renderer.log()
    );
    assert_eq!(
        buffers[1], buffers[0],
        "a second write of overrides as long reallocated the state's buffers"
    );
}
