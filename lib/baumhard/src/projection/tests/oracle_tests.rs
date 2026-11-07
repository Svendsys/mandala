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

use crate::document::writer::transaction::TransactionKind;
use crate::document::writer::ChangeSource;
use crate::engine::interpolate::Snap;
use crate::projection::tests::corpus::{edit, edit_named, generated, of_kind, EDITS};
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
        single(DocKind::Node, DocMutation::Node(NodeOp::Nudge(Vec2::new(30.0, 5.0)))),
        vec![ids.a],
        "nudge",
    ))
    .unwrap();
    p.check().unwrap();
    p.apply(request(
        single(DocKind::Span, DocMutation::Span(SpanOp::SetText("a longer title".into()))),
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


/// Take `p` through `steps` generated edits drawn from `c` — each applied
/// and, one in three, undone and redone — and transaction stages, the
/// oracle checked after every change; the names of the edits that
/// applied are counted in `covered`. `Err` names the case, the step and
/// the edit.
pub(crate) fn sweep(
    case: &str,
    p: &mut Projected,
    c: &mut Choices<'_>,
    steps: usize,
    turn: &mut usize,
    covered: &mut FxHashMap<&'static str, usize>,
) -> Result<(), String> {
    p.check().map_err(|e| format!("{case}, the full projection: {e}"))?;
    for step in 0..steps {
        let at = |what: &str, e: String| format!("{case}, step {step}, {what}: {e}");
        if c.one_in(6) {
            // A transaction: two stages, then a commit or a cancel; an
            // animated one is interpolated half-way first.
            let kind = if c.one_in(2) {
                TransactionKind::Interactive
            } else {
                TransactionKind::Animated
            };
            let Ok(tx) = p.doc.begin(ChangeSource::Input, kind) else {
                continue;
            };
            for stage in 0..2 {
                let Some((name, req)) = edit(c, &p.doc) else {
                    continue;
                };
                if p.doc.stage(&tx, req).is_ok() {
                    *covered.entry("transaction::stage").or_default() += 1;
                    p.sync_and_discard();
                    p.check().map_err(|e| at(&format!("stage {stage} ({name})"), e))?;
                }
            }
            if kind == TransactionKind::Animated && p.doc.interpolate(&tx, 0.5, Snap::End).is_ok() {
                *covered.entry("transaction::interpolate").or_default() += 1;
                p.sync_and_discard();
                p.check().map_err(|e| at("interpolate", e))?;
            }
            if c.one_in(2) {
                p.doc.commit(tx);
            } else {
                p.doc.cancel(tx);
            }
            p.sync_and_discard();
            p.check().map_err(|e| at("commit or cancel", e))?;
            continue;
        }
        let name = EDITS[*turn % EDITS.len()];
        *turn += 1;
        let Some((name, req)) = edit_named(c, &p.doc, name) else {
            continue;
        };
        if p.apply(req).is_err() {
            continue;
        }
        *covered.entry(name).or_default() += 1;
        p.check().map_err(|e| at(name, e))?;
        if c.one_in(3) {
            p.doc.undo();
            p.sync_and_discard();
            p.check().map_err(|e| at(&format!("undo of {name}"), e))?;
            p.doc.redo();
            p.sync_and_discard();
            p.check().map_err(|e| at(&format!("redo of {name}"), e))?;
            *covered.entry("history::undo_and_redo").or_default() += 1;
        }
    }
    Ok(())
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
/// never saw applied.
fn sweep_group(group: u64) {
    let mut covered = FxHashMap::default();
    let mut turn = (group as usize) * 7;
    let per = DOCUMENTS / GROUPS;
    for seed in group * per..(group + 1) * per {
        let mut c = Choices::seeded(seed);
        let doc = generated(&mut c);
        let nodes = of_kind(&doc, DocKind::Node).len();
        assert!(nodes >= 50, "seed {seed}: a corpus document has fifty nodes or more ({nodes})");
        let mut p = Projected::new_shared(doc);
        sweep(&format!("seed {seed}"), &mut p, &mut c, STEPS, &mut turn, &mut covered).unwrap();
    }
    let missing: Vec<&str> = EDITS
        .iter()
        .chain(["transaction::stage", "transaction::interpolate", "history::undo_and_redo"].iter())
        .filter(|name| !covered.contains_key(*name))
        .copied()
        .collect();
    assert!(missing.is_empty(), "group {group} never applied {missing:?}");
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
    assert!(files.len() >= 9, "the V1-B2 fixtures and the demo maps: {files:?}");
    let mut covered = FxHashMap::default();
    let mut turn = 0;
    for (n, file) in files.iter().enumerate() {
        let text = std::fs::read_to_string(file).unwrap();
        let doc = crate::document::format::load(&text).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let mut p = Projected::new_shared(doc);
        let mut c = Choices::seeded(0x0B7_F1C5 + n as u64);
        sweep(&file.display().to_string(), &mut p, &mut c, 12, &mut turn, &mut covered).unwrap();
    }
}

#[test]
fn test_projection_oracle_a_rebinding_near_the_root_reprojects_exactly_the_rebound_subtree() {
    use crate::document::element::PaletteData;
    use crate::document::mutation::ops::{DocMutation, NodeOp};
    use crate::document::tests::test_support::{palette_group, DocumentBuilder};
    use crate::document::tests::writer_support::{request, single};
    use crate::document::values::PaletteId;
    use crate::projection::Stage;
    // A root bound to a palette, and below it two subtrees of eight
    // nodes each, every node with text the palette colors.
    let mut b = DocumentBuilder::new();
    b.palette(PaletteData {
        name: "a".into(),
        groups: vec![palette_group(10), palette_group(11), palette_group(12)],
    });
    b.palette(PaletteData {
        name: "b".into(),
        groups: vec![palette_group(20), palette_group(21)],
    });
    let root = b.root();
    let top = b.node(root);
    b.edit_node(top, |n| n.palette = Some(PaletteId(0)));
    let section = b.section(top, 0);
    b.span(section, "top");
    let subtree = |b: &mut DocumentBuilder| {
        let head = b.node(top);
        let mut nodes = vec![head];
        let mut sections = Vec::new();
        for i in 0..8 {
            let at = if i == 0 { head } else { b.node(nodes[(i - 1) / 2]) };
            if i > 0 {
                nodes.push(at);
            }
            let section = b.section(at, 0);
            b.span(section, "text");
            sections.push(section);
        }
        (head, nodes, sections)
    };
    let (rebound, rebound_nodes, rebound_sections) = subtree(&mut b);
    let (_, kept_nodes, kept_sections) = subtree(&mut b);
    let mut p = Projected::new(b.build().unwrap());
    p.apply(request(
        single(DocKind::Node, DocMutation::Node(NodeOp::SetPalette(Some(PaletteId(1))))),
        vec![rebound],
        "rebind",
    ))
    .unwrap();
    let ran = |stage: Stage| -> rustc_hash::FxHashSet<crate::arena::NodeId> {
        p.projector.ran.iter().filter(|d| d.stage == stage).map(|d| d.unit).collect()
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
        assert!(!p.projector.ran.iter().any(|d| d.unit == *unit), "{unit:?} beside it was not reached");
    }
    p.check().unwrap();
}
