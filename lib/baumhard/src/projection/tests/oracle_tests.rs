// SPDX-License-Identifier: MPL-2.0

//! The oracle (#241): after every change, the layers kept up to date by
//! `Projector::update` equal a full projection of the document.
//! `cfg(test)`: sweeps, not bench bodies (TEST_CONVENTIONS §T14's rule
//! for property sweeps).

use crate::document::element::DocKind;
use crate::document::mutation::compose::delete;
use crate::document::mutation::ops::{DocMutation, NodeOp, SpanOp};
use crate::document::tests::writer_support::{built, request, single};
use crate::document::writer::EditRequest;
use crate::projection::tests::support::Projected;
use glam::Vec2;
use rustc_hash::FxHashMap;

use crate::projection::tests::corpus::{generated, of_kind, EDITS};
use crate::projection::tests::projection_fuzz::sweep;
use crate::util::tests::choices::Choices;

#[test]
fn test_projection_oracle_smoke_on_the_example_map() {
    let (doc, ids) = built();
    let mut p = Projected::new(doc);
    for layer in crate::projection::Layer::ALL {
        let tree = p.layers.tree(layer);
        let runs = tree
            .root()
            .descendants(tree.arena())
            .filter(|&g| tree.arena()[g].get().glyph_run().is_some())
            .count();
        eprintln!("{layer:?}: {runs} runs");
    }
    let ring = p.projector.glyphs_of(ids.ring);
    assert_eq!(ring.len(), 1, "the ring has its run");
    let tree = p.layers.tree(ring[0].0);
    let placed = match tree.arena()[ring[0].1].get().glyph_run().unwrap().shaped() {
        crate::gfx_structs::run::RunShape::Path { placed, .. } => placed.glyphs.len(),
        _ => 0,
    };
    assert!(placed > 10, "the ring placed its text: {placed} glyphs");
    p.check().unwrap();
    p.apply(request(
        single(
            DocKind::Node,
            DocMutation::Node(NodeOp::Nudge(Vec2::new(30.0, 5.0))),
        ),
        vec![ids.a],
        "nudge",
    ))
    .unwrap();
    p.check().unwrap();
    p.apply(request(
        single(
            DocKind::Span,
            DocMutation::Span(SpanOp::SetText("a longer title".into())),
        ),
        vec![ids.a_title_span],
        "text",
    ))
    .unwrap();
    p.check().unwrap();
    let steps = delete(&p.doc, &[ids.b]).unwrap();
    p.apply(EditRequest::new(steps, "delete b")).unwrap();
    p.check().unwrap();
    p.doc.undo().unwrap();
    p.sync_and_discard();
    p.check().unwrap();
    p.doc.redo().unwrap();
    p.sync_and_discard();
    p.check().unwrap();
}

/// How many generated documents the corpus holds: #241 asks for a
/// hundred or more, of fifty nodes or more each (`corpus::generated`).
const DOCUMENTS: u64 = 104;

/// How many generated edits each document is taken through.
const STEPS: usize = 12;

/// The corpus is swept in this many tests, each a slice of the seeds,
/// so the suite runs them side by side.
const GROUPS: u64 = 4;

/// Sweep slice `group` of the generated corpus, and require that it
/// applied every edit the generator draws, transaction stages and
/// undo and redo among them, so no slice holds a vocabulary the oracle
/// never saw applied — and that some of its paths run behind a node, so
/// the oracle holds the glyphs nodes hide (#332) as well.
fn sweep_group(group: u64) {
    let mut covered = FxHashMap::default();
    let mut behind = 0;
    let mut turn = (group as usize) * 7;
    let per = DOCUMENTS / GROUPS;
    for seed in group * per..(group + 1) * per {
        let mut c = Choices::seeded(seed);
        let doc = generated(&mut c);
        let nodes = of_kind(&doc, DocKind::Node).len();
        assert!(
            nodes >= 50,
            "seed {seed}: a corpus document has fifty nodes or more ({nodes})"
        );
        let mut p = Projected::new_shared(doc);
        behind += hiding(&p.projector);
        sweep(
            &format!("seed {seed}"),
            &mut p,
            &mut c,
            STEPS,
            &mut turn,
            &mut covered,
        )
        .unwrap();
    }
    let missing: Vec<&str> = EDITS
        .iter()
        .chain(
            [
                "transaction::stage",
                "transaction::interpolate",
                "history::undo_and_redo",
                "batch",
            ]
            .iter(),
        )
        .filter(|name| !covered.contains_key(*name))
        .copied()
        .collect();
    assert!(missing.is_empty(), "group {group} never applied {missing:?}");
    assert!(
        behind > 0,
        "precondition: group {group}'s documents run some path behind a node"
    );
}

/// How many drawn paths' bodies have glyphs a node hides (#332).
fn hiding(projector: &crate::projection::Projector) -> usize {
    let count = |run: Option<&crate::gfx_structs::run::GlyphRunData>| run.map(|r| r.shaped().glyph_count());
    projector
        .units
        .values()
        .filter(|u| match &u.body {
            crate::projection::state::UnitBody::Line(line) => {
                count(line.laid[0].as_ref()) != count(u.slots[0].want.as_ref())
            }
            _ => false,
        })
        .count()
}

#[test]
fn test_projection_oracle_generated_corpus_0() {
    sweep_group(0);
}

#[test]
fn test_projection_oracle_generated_corpus_1() {
    sweep_group(1);
}

#[test]
fn test_projection_oracle_generated_corpus_2() {
    sweep_group(2);
}

#[test]
fn test_projection_oracle_generated_corpus_3() {
    sweep_group(3);
}

#[test]
fn test_projection_oracle_fixtures_and_demo_maps() {
    use crate::util::doc_fixtures::repo_path;
    let mut files = Vec::new();
    for dir in ["lib/baumhard/src/document/format/fixtures", "maps/demos"] {
        let mut found: Vec<_> = std::fs::read_dir(repo_path(dir))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.to_string_lossy().ends_with(".mandala.json"))
            .collect();
        found.sort();
        files.extend(found);
    }
    assert!(
        files.len() >= 9,
        "the V1-B2 fixtures and the demo maps: {files:?}"
    );
    let mut covered = FxHashMap::default();
    let mut turn = 0;
    for (n, file) in files.iter().enumerate() {
        let text = std::fs::read_to_string(file).unwrap();
        let doc = crate::document::format::load(&text).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let mut p = Projected::new_shared(doc);
        let mut c = Choices::seeded(0x0B7_F1C5 + n as u64);
        sweep(
            &file.display().to_string(),
            &mut p,
            &mut c,
            12,
            &mut turn,
            &mut covered,
        )
        .unwrap();
    }
}

#[test]
fn test_projection_oracle_a_rebinding_near_the_root_reprojects_exactly_the_rebound_subtree() {
    use crate::document::mutation::ops::{DocMutation, NodeOp};
    use crate::document::tests::writer_support::{request, single};
    use crate::document::values::PaletteId;
    use crate::projection::tests::maps::{ident, kid, load_map, node};
    use crate::projection::Stage;
    use serde_json::json;
    // A root bound to a palette, and below it two subtrees of eight
    // nodes each — node `i` of a subtree under node `(i - 1) / 2` — every
    // node with text the palette colors. Written in the flat list's
    // pre-order.
    let mut nodes = vec![node(
        1,
        None,
        json!({"palette": "a", "sections": [{"spans": [{"text": "top"}]}]}),
    )];
    for base in [10u32, 20] {
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let parent = if i == 0 { 1 } else { base + (i - 1) / 2 };
            nodes.push(node(
                base + i,
                Some(parent),
                json!({"connector": {}, "sections": [{"spans": [{"text": "text"}]}]}),
            ));
            for child in [2 * i + 2, 2 * i + 1] {
                if child < 8 {
                    stack.push(child);
                }
            }
        }
    }
    let doc = load_map(json!({
        "palettes": {
            "a": [
                {"frame": "#000a00", "text": "#00000a"},
                {"frame": "#000b00", "text": "#00000b"},
                {"frame": "#000c00", "text": "#00000c"}
            ],
            "b": [{"frame": "#001400", "text": "#000014"}, {"frame": "#001500", "text": "#000015"}]
        },
        "nodes": nodes
    }));
    let subtree = |base: u32| -> (Vec<crate::arena::NodeId>, Vec<crate::arena::NodeId>) {
        let nodes: Vec<_> = (0..8).map(|i| ident(&doc, base + i)).collect();
        let sections = nodes.iter().map(|&n| kid(&doc, n, DocKind::Section)).collect();
        (nodes, sections)
    };
    let (rebound_nodes, rebound_sections) = subtree(10);
    let (kept_nodes, kept_sections) = subtree(20);
    let rebound = rebound_nodes[0];
    let mut p = Projected::new(doc);
    p.apply(request(
        single(
            DocKind::Node,
            DocMutation::Node(NodeOp::SetPalette(Some(PaletteId(1)))),
        ),
        vec![rebound],
        "rebind",
    ))
    .unwrap();
    let ran = |stage: Stage| -> rustc_hash::FxHashSet<crate::arena::NodeId> {
        p.projector
            .ran
            .iter()
            .filter(|d| d.stage == stage)
            .map(|d| d.unit)
            .collect()
    };
    assert_eq!(
        ran(Stage::Carry),
        rebound_nodes.iter().copied().collect(),
        "every node whose carried binding changed carried again, and no other"
    );
    assert_eq!(
        ran(Stage::Resolve),
        rebound_sections.iter().copied().collect(),
        "every section that reads it resolved again, and no other"
    );
    for unit in kept_nodes.iter().chain(kept_sections.iter()) {
        assert!(
            !p.projector.ran.iter().any(|d| d.unit == *unit),
            "{unit:?} beside it was not reached"
        );
    }
    p.check().unwrap();
}
