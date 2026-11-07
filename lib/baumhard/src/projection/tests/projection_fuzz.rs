// SPDX-License-Identifier: MPL-2.0

//! The projection's fuzz properties (design D29, §20 rule 10; rulings
//! B7.3 on #241): a loaded map projects, and an edit sequence keeps the
//! projection equal to a full one.
//!
//! - [`check_projects`](crate::projection::tests::projection_fuzz::check_projects)
//!   is what the loader targets (`load_raw`, `load_structured`) hold every
//!   map they load to: inside the projection's work bound
//!   ([`within_bound`](crate::projection::tests::projection_fuzz::within_bound)),
//!   it projects without a panic, one group per visible node in every
//!   layer, every run's box finite, the same layers when projected again,
//!   hits that do not panic, and one generated edit that the update
//!   brings the layers through to a full projection's. A map outside the
//!   bound is skipped for this property and counted
//!   ([`projection_counts`](crate::projection::tests::projection_fuzz::projection_counts)),
//!   which the targets report.
//! - [`check_projection_edits`](crate::projection::tests::projection_fuzz::check_projection_edits)
//!   is the projection_edits target's body: a generated map and a
//!   sequence of edits drawn from the input — field edits, structural
//!   edits, a rebinding near the root, theme and palette changes,
//!   transaction stages, undos and redos — the oracle checked after each.
//!
//! **The bound** (ruling B7.3). The engine bounds each placement (R41)
//! and each frame's work (its budget), never a whole map, so a hostile map
//! can ask for more work than a fuzz input's time (#316). Where the
//! engine's bound lies past the timeout the harness bounds what it feeds
//! (TEST_CONVENTIONS §T14): a map is projected here when it holds at most
//! one ring with text, of at most
//! [`MAX_FRAME_GLYPHS`](crate::flow::tests::flow_fuzz::MAX_FRAME_GLYPHS) characters — the
//! flow harnesses' one frame an input — and every coordinate a path or an
//! outline is drawn from is within
//! [`EXTENT`](crate::projection::tests::projection_fuzz::EXTENT), so no path is long enough
//! for a placement near the cap.

use std::sync::atomic::{AtomicUsize, Ordering};

use glam::Vec2;
use rustc_hash::FxHashMap;

use crate::document::element::DocElement;
use crate::document::ids::Document;
use crate::document::writer::transaction::TransactionKind;
use crate::document::writer::ChangeSource;
use crate::engine::interpolate::Snap;
use crate::flow::tests::flow_fuzz::MAX_FRAME_GLYPHS;
use crate::projection::tests::corpus::{edit, edit_named, generated, EDITS};
use crate::projection::tests::support::Projected;
use crate::projection::Layer;
use crate::util::tests::choices::Choices;

/// How far from the origin, per axis, a coordinate of a map the loader
/// targets project may be: positions, sizes, offsets and control points.
pub const EXTENT: f32 = 8192.0;

static PROJECTED: AtomicUsize = AtomicUsize::new(0);
static SKIPPED: AtomicUsize = AtomicUsize::new(0);

/// How many loaded maps [`check_projects`] has projected and how many it
/// skipped, in this process: what a fuzz run reports (ruling B7.3).
pub fn projection_counts() -> (usize, usize) {
    (PROJECTED.load(Ordering::Relaxed), SKIPPED.load(Ordering::Relaxed))
}

/// Whether `v` is within [`EXTENT`] on both axes.
fn near(v: Vec2) -> bool {
    v.is_finite() && v.x.abs() <= EXTENT && v.y.abs() <= EXTENT
}

/// Whether `doc` is inside the projection's work bound (see the module
/// docs). O(the document).
pub fn within_bound(doc: &Document) -> bool {
    let tree = doc.tree();
    let arena = tree.arena();
    let mut rings_with_text = 0;
    for id in tree.descendants() {
        let ok = match arena[id].get() {
            DocElement::Node(node) => near(node.position) && node.min_size.is_none_or(near),
            DocElement::Section(section) => near(section.offset) && section.min_size.is_none_or(near),
            DocElement::Connector(c) => c.path.control_points.iter().all(|&p| near(p)),
            DocElement::Link(l) => l.path.control_points.iter().all(|&p| near(p)),
            DocElement::Label(label) => label.style.offset.is_none_or(|o| o.abs() <= EXTENT),
            DocElement::PortalEnd(end) => end.style.offset.is_none_or(|o| o.abs() <= EXTENT),
            DocElement::Ring(_) => {
                let chars: usize = id
                    .children(arena)
                    .filter_map(|s| match arena[s].get() {
                        DocElement::Span(span) => Some(span.text.chars().count()),
                        _ => None,
                    })
                    .sum();
                if chars > 0 {
                    rings_with_text += 1;
                }
                chars <= MAX_FRAME_GLYPHS
            }
            _ => true,
        };
        if !ok {
            return false;
        }
    }
    rings_with_text <= 1
}

/// The projection properties of a map the loader accepted (see the module
/// docs): `Ok` when they hold, or when the map is outside the bound —
/// skipped, and counted.
pub fn check_projects(doc: &Document) -> Result<(), String> {
    if !within_bound(doc) {
        SKIPPED.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }
    PROJECTED.fetch_add(1, Ordering::Relaxed);
    // In the test build the loader suites' maps, up to a hundred thousand
    // nodes, are projected sharing the shapings of equal keys as the
    // oracle's full projections do (`stages::memo`): every property here
    // compares by value. A fuzz build shapes every flow.
    #[cfg(test)]
    let mut p = Projected::new_shared(doc.clone());
    #[cfg(not(test))]
    let mut p = Projected::new(doc.clone());
    for layer in Layer::ALL {
        let tree = p.layers.tree(layer);
        let mut groups = 0;
        for glyph in tree.root().descendants(tree.arena()) {
            match tree.arena()[glyph].get().glyph_run() {
                None => groups += 1,
                Some(run) => {
                    let (corner, size) = run.bounds();
                    if !(corner.is_finite() && size.is_finite()) {
                        return Err(format!("a {layer:?} run's box is not finite: {corner} {size}"));
                    }
                }
            }
        }
        if groups != p.projector.nodes.len() + 1 {
            return Err(format!(
                "the {layer:?} layer holds {groups} groups for {} visible nodes and its root",
                p.projector.nodes.len()
            ));
        }
    }
    let mut points = vec![Vec2::ZERO];
    points.extend(p.projector.nodes.values().take(8).map(|n| n.rect.center()));
    for point in points {
        p.projector.hit(&mut p.layers, point);
    }
    // The same layers when projected again (the sweep's first check: a
    // full projection compared with these), then one edit through the
    // oracle.
    let mut c = Choices::seeded(doc.tree().arena().len() as u64);
    let mut covered = FxHashMap::default();
    let mut turn = 0;
    sweep("a loaded map", &mut p, &mut c, 1, &mut turn, &mut covered)
}

/// The projection_edits target's body: a generated map, and up to sixteen
/// steps drawn from `bytes`, each through the oracle (see the module
/// docs).
pub fn check_projection_edits(bytes: &[u8]) -> Result<(), String> {
    let mut c = Choices::new(bytes);
    let doc = generated(&mut c);
    let steps = 1 + c.below(16) as usize;
    let mut p = Projected::new(doc);
    let mut covered = FxHashMap::default();
    let mut turn = c.index(EDITS.len());
    sweep("projection_edits", &mut p, &mut c, steps, &mut turn, &mut covered)
}

/// Take `p` through `steps` edits — the edit [`EDITS`] names at `turn`,
/// advancing it, its targets and values drawn from `c`, applied and, one
/// in three, undone and redone — and transaction stages, the oracle
/// checked after every change; each edit applied is counted in
/// `covered`. `Err` names the case, the step and the edit.
pub fn sweep(
    case: &str,
    p: &mut Projected,
    c: &mut Choices<'_>,
    steps: usize,
    turn: &mut usize,
    covered: &mut FxHashMap<&'static str, usize>,
) -> Result<(), String> {
    p.check()
        .map_err(|e| format!("{case}, the full projection: {e}"))?;
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
