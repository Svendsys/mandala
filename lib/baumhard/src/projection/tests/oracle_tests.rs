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

/// One generated document taken through `steps` generated edits — each
/// applied, undone and redone — and transaction stages, the oracle
/// checked after every change; the names of the edits that applied are
/// added to `covered`. `Err` names the seed, the step and the edit.
pub(crate) fn sweep_one(
    seed: u64,
    steps: usize,
    covered: &mut rustc_hash::FxHashMap<&'static str, usize>,
) -> Result<(), String> {
    use crate::document::writer::transaction::TransactionKind;
    use crate::document::writer::ChangeSource;
    use crate::engine::interpolate::Snap;
    use crate::projection::tests::corpus::{edit, generated};
    use crate::util::tests::choices::Choices;
    let mut c = Choices::seeded(seed);
    let mut p = Projected::new_shared(generated(&mut c));
    p.check().map_err(|e| format!("seed {seed}, the full projection: {e}"))?;
    for step in 0..steps {
        let at = |what: &str, e: String| format!("seed {seed}, step {step}, {what}: {e}");
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
                let Some((name, req)) = edit(&mut c, &p.doc) else {
                    continue;
                };
                if p.doc.stage(&tx, req).is_ok() {
                    p.sync_and_discard();
                    p.check().map_err(|e| at(&format!("stage {stage} ({name})"), e))?;
                }
            }
            if kind == TransactionKind::Animated && p.doc.interpolate(&tx, 0.5, Snap::End).is_ok() {
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
        let Some((name, req)) = edit(&mut c, &p.doc) else {
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
        }
    }
    Ok(())
}

#[test]
fn test_projection_oracle_one_generated_document() {
    let mut covered = Default::default();
    let start = std::time::Instant::now();
    for seed in 0..20 { sweep_one(seed, 40, &mut covered).unwrap(); }
    eprintln!("one document, 40 steps: {:?}; {} edits covered", start.elapsed(), covered.len());
}
