// SPDX-License-Identifier: MPL-2.0

//! Reverse converter — pulls live tree-side `(text, regions,
//! position, size)` per section back into the model's
//! `MindSection` shape after a custom mutation lands. The forward
//! direction (model → tree) lives in
//! `lib/baumhard/src/mindmap/tree_builder/node.rs::append_node_sections`;
//! this file is the dedicated reverse counterpart for the
//! `Persistent` apply path.
//!
//! Why split from `mod.rs`: the apply pipeline (200+ LOC) and
//! the reverse converter (200+ LOC) share `MindMapDocument`'s
//! `&mut self` access but no other state. Splitting along that
//! conceptual seam keeps each file to one job, matching the
//! `nodes/{mod, …}` precedent already in this directory.

use baumhard::core::primitives::ColorFontRegion;
use baumhard::font::fonts::family_name_of;
use baumhard::mindmap::model::TextRun;
use baumhard::mindmap::tree_builder::MindMapTree;
use baumhard::util::color_conversion::rgba_to_hex;

use super::super::nodes::clamp_runs_to_text;
use super::super::MindMapDocument;

/// Default text-run colour when neither the tree-side region nor
/// a prior model run carries one. Matches the renderer's
/// fall-through-to-`#ffffff` floor on a node with no explicit
/// `style.text_color` override.
pub(super) const DEFAULT_TEXT_RUN_COLOR: &str = "#ffffff";

/// Default font-size used by the renderer when no run pins one.
/// Mirrors `cosmic_text`'s 14pt fallback used at scene-build time.
pub(super) const DEFAULT_TEXT_RUN_SIZE_PT: u32 = 14;

/// Roll a tree-side [`ColorFontRegion`] back into a model-side
/// [`TextRun`], merging fields the tree dropped during the
/// forward conversion against a `prior` run when the prior
/// covered the same `Range`. The forward path
/// (`tree_builder/node.rs::append_node_sections`) only carries
/// `range`, `color`, and `font` onto the tree-side region;
/// `bold` / `italic` / `underline` / `size_pt` / `hyperlink`
/// disappear into the cosmic-text default attribute set. The
/// reverse path can recover them only when a matching prior run
/// is available — which is true for round-trips through the
/// custom-mutation pipeline (the tree is rebuilt from the model
/// just before each apply, so every region's range was an
/// authored run before the mutation ran).
///
/// Limitations:
/// - `var(--name)` colour references collapse to their resolved
///   hex on the round trip *unless* the prior run shares the
///   region's range — see the `prior_var_color` short-circuit
///   below.
/// - Unknown `AppFont` (corrupt tree state) falls through to
///   the empty string, matching the loader's tolerance for
///   missing-font runs.
///
/// Visible to [`super::super::nodes`] so the editor commit path
/// can reuse the converter through `set_section_text_and_runs`.
pub(crate) fn region_to_text_run(region: &ColorFontRegion, prior: Option<&TextRun>) -> TextRun {
    // Preserve `var(--name)` references when the prior run
    // shares the region's range and carries one. Without theme-
    // variables resolution at sync time we can't tell whether a
    // mutation deliberately recoloured the run away from the
    // variable; trusting the prior keeps the variable reference
    // verbatim across mutations that didn't touch the colour.
    // Same documented trade-off as the selective gate: a
    // deliberate `SetRegionColor` on a `var()`-bearing run is
    // silently swallowed here — the run keeps the variable.
    let prior_var_color: Option<&str> = prior.and_then(|p| {
        if p.color.starts_with("var(")
            && p.start == region.range.start
            && p.end == region.range.end
        {
            Some(p.color.as_str())
        } else {
            None
        }
    });
    let color = match (prior_var_color, region.color) {
        (Some(var_color), _) => var_color.to_string(),
        (None, Some(rgba)) => rgba_to_hex(rgba),
        (None, None) => prior
            .map(|p| p.color.clone())
            .unwrap_or_else(|| DEFAULT_TEXT_RUN_COLOR.to_string()),
    };
    let font = match region.font.and_then(family_name_of) {
        Some(name) => name.to_string(),
        None => prior.map(|p| p.font.clone()).unwrap_or_default(),
    };
    let bold = prior.is_some_and(|p| p.bold);
    let italic = prior.is_some_and(|p| p.italic);
    let underline = prior.is_some_and(|p| p.underline);
    let size_pt = prior.map(|p| p.size_pt).unwrap_or(DEFAULT_TEXT_RUN_SIZE_PT);
    let hyperlink = prior.and_then(|p| p.hyperlink.clone());
    TextRun {
        start: region.range.start,
        end: region.range.end,
        bold,
        italic,
        underline,
        font,
        size_pt,
        color,
        hyperlink,
    }
}

/// Find the prior `TextRun` for a tree-side region by range.
/// Prefers exact `(start, end)` match; falls back to the prior
/// run whose intersection with `[start, end)` is largest. Used by
/// `sync_node_from_tree`'s reverse converter so a custom mutation
/// that resizes / splits a region (e.g. `ChangeRegionRange`)
/// still inherits authored styling instead of zeroing every
/// field. Ties broken in favour of earlier `start`.
///
/// Returns `None` only when no prior run overlaps the new range
/// at all (e.g. a fresh region inserted by the mutation).
///
/// Visible to [`super::super::nodes`] so the editor commit path
/// can reuse the same lookup through `set_section_text_and_runs`.
pub(crate) fn exact_or_dominant_overlap<'a>(
    priors: &[&'a TextRun],
    start: usize,
    end: usize,
) -> Option<&'a TextRun> {
    if let Some(exact) = priors.iter().find(|r| r.start == start && r.end == end) {
        return Some(exact);
    }
    let mut best: Option<(&'a TextRun, usize)> = None;
    for run in priors.iter() {
        if run.end <= start || run.start >= end {
            continue;
        }
        let lo = run.start.max(start);
        let hi = run.end.min(end);
        if hi <= lo {
            continue;
        }
        let overlap = hi - lo;
        match best {
            None => best = Some((run, overlap)),
            Some((_, prev)) if overlap > prev => best = Some((run, overlap)),
            _ => {}
        }
    }
    best.map(|(r, _)| r)
}

impl MindMapDocument {
    /// Sync the document model from the live tree — pull
    /// `node.position` from the container's glyph area and every
    /// section's `(text, text_runs, offset, size)` from its
    /// section-area, with a per-section selective gate that skips
    /// the lossy text/regions round-trip when the tree side hasn't
    /// diverged from the model. Position / offset / size always
    /// write back; text + runs gate on the
    /// `(range, colour, font)` triple.
    ///
    /// Used by the `Persistent` apply path to commit a custom
    /// mutation's tree-side mutations to the model so the next
    /// `rebuild_all` doesn't revert them. The selective gate
    /// matters because the forward conversion drops
    /// `bold` / `italic` / `underline` / `size_pt` / `hyperlink`;
    /// an unconditional round-trip would silently strip those
    /// fields from sections the mutation didn't touch.
    pub(super) fn sync_node_from_tree(&mut self, node_id: &str, tree: &MindMapTree) {
        let Some(tree_nid) = tree.arena_id_for(node_id) else {
            return;
        };
        let Some(element) = tree.tree.arena.get(tree_nid).map(|n| n.get()) else {
            return;
        };
        let Some(area) = element.glyph_area() else {
            return;
        };
        let new_pos = (area.position.x.0 as f64, area.position.y.0 as f64);

        // Gather every section's tree-side `(text, regions, position,
        // size)` before we acquire `&mut` on the model. The arena
        // lookup needs `&tree`; the model write needs `&mut self`;
        // sequencing them avoids overlapping borrows on
        // `self.mindmap`. Capturing position + size lets us write
        // `section.offset` / `section.size` back from the tree, so a
        // `SectionsOnly` mutation that translates / resizes a
        // section persists past the next `rebuild_all`.
        let section_count = self
            .mindmap
            .nodes
            .get(node_id)
            .map(|n| n.sections.len())
            .unwrap_or(0);
        struct SectionSnapshot {
            text: String,
            regions: Vec<ColorFontRegion>,
            tree_position: (f32, f32),
            tree_size: (f32, f32),
        }
        let mut section_snapshots: Vec<Option<SectionSnapshot>> = Vec::with_capacity(section_count);
        for idx in 0..section_count {
            let snapshot = tree
                .section_arena_id(node_id, idx)
                .and_then(|sid| tree.tree.arena.get(sid))
                .and_then(|n| n.get().glyph_area())
                .map(|sec_area| SectionSnapshot {
                    text: sec_area.text.clone(),
                    regions: sec_area
                        .regions
                        .all_regions()
                        .into_iter()
                        .copied()
                        .collect::<Vec<ColorFontRegion>>(),
                    tree_position: (sec_area.position.x.0, sec_area.position.y.0),
                    tree_size: (sec_area.render_bounds.x.0, sec_area.render_bounds.y.0),
                });
            section_snapshots.push(snapshot);
        }

        let Some(model_node) = self.mindmap.nodes.get_mut(node_id) else {
            return;
        };
        model_node.position.x = new_pos.0;
        model_node.position.y = new_pos.1;
        let node_pos_x = new_pos.0 as f32;
        let node_pos_y = new_pos.1 as f32;
        let node_size_x = model_node.size.width as f32;
        let node_size_y = model_node.size.height as f32;

        for (idx, snapshot) in section_snapshots.into_iter().enumerate() {
            let Some(snapshot) = snapshot else {
                continue;
            };
            let Some(section) = model_node.sections.get_mut(idx) else {
                continue;
            };

            // Write `section.offset` back from the tree's section-
            // area position so a `SectionsOnly` translate mutation
            // persists. The forward path computes
            // `section_area.position = node.pos + section.offset`,
            // so the inverse is `section.offset = section_area.position
            // - node.pos`. Section-area position is canvas-space
            // float; model `Position` is canvas-space f64 — same
            // unit, just wider. Without this, a `Translate` /
            // `MoveTo` on a section-area lands on the live tree
            // and reverts on the next `rebuild_all`.
            let new_offset_x = (snapshot.tree_position.0 - node_pos_x) as f64;
            let new_offset_y = (snapshot.tree_position.1 - node_pos_y) as f64;
            if section.offset.x != new_offset_x || section.offset.y != new_offset_y {
                section.offset.x = new_offset_x;
                section.offset.y = new_offset_y;
            }
            // Write `section.size` back when the model carries an
            // explicit size. `None` size means "fill the parent
            // node", which the tree resolves to the node's full
            // render_bounds — *don't* eagerly materialise it as
            // `Some(node.size)`, that would surprise authors who
            // chose the inheriting shape. Materialise only when the
            // tree's render_bounds diverges from the node's full
            // size (i.e. the mutation explicitly resized the
            // section, or the model already carried a Some).
            let tree_size_diverges =
                (snapshot.tree_size.0 - node_size_x).abs() > f32::EPSILON
                    || (snapshot.tree_size.1 - node_size_y).abs() > f32::EPSILON;
            if section.size.is_some() || tree_size_diverges {
                section.size = Some(baumhard::mindmap::model::Size {
                    width: snapshot.tree_size.0 as f64,
                    height: snapshot.tree_size.1 as f64,
                });
            }

            // Selective gate: tree-side state matches the model
            // snapshot? Skip the text/regions round-trip so
            // untouched sections keep their bold / italic /
            // underline / size_pt / hyperlink. Range / colour /
            // font are everything the forward conversion
            // preserves.
            //
            // **Range-keyed comparison.** Tree-side
            // `all_regions()` returns runs in `Range` order
            // (`BTreeSet`-keyed); model `text_runs: Vec<TextRun>`
            // is load-order. A positional `zip` would mis-align
            // any model whose runs were authored out of range
            // order, trip a false mismatch, and run the lossy
            // round-trip — silently stripping the prior styling
            // from sections the mutation didn't touch. Build a
            // map keyed by `(start, end)` and compare each
            // tree-side region against the same-range prior.
            let model_runs_by_range: rustc_hash::FxHashMap<(usize, usize), &TextRun> = section
                .text_runs
                .iter()
                .map(|r| ((r.start, r.end), r))
                .collect();
            let model_regions_match = model_runs_by_range.len() == snapshot.regions.len()
                && snapshot.regions.iter().all(|region| {
                    let key = (region.range.start, region.range.end);
                    let Some(run) = model_runs_by_range.get(&key) else {
                        return false;
                    };
                    // Colour comparison is **case-insensitive on
                    // hex**: `rgba_to_hex` always emits lowercase,
                    // but model-side `run.color` may have been
                    // hand-authored as `#FFFFFF` or mixed case. A
                    // byte-equal `==` would always-mismatch those
                    // and trigger the lossy round-trip on every
                    // apply_to_tree call.
                    let region_color_hex = region.color.map(rgba_to_hex);
                    let model_color_hex = if run.color.starts_with('#') {
                        Some(run.color.clone())
                    } else {
                        None
                    };
                    let model_is_var = run.color.starts_with("var(");
                    let colors_equal = match (region_color_hex.as_deref(), model_color_hex.as_deref()) {
                        (Some(a), Some(b)) => str::eq_ignore_ascii_case(a, b),
                        (None, None) => true,
                        // `(Some(hex), None)` with the model carrying
                        // a `var(--…)` reference: presume the
                        // variable resolves to the tree-side hex
                        // and treat as equal. Documented limit: a
                        // custom mutation that *deliberately*
                        // recolours a `var()`-bearing run is
                        // silently swallowed; the run keeps the
                        // variable.
                        (Some(_), None) if model_is_var => true,
                        _ => false,
                    };
                    if !colors_equal {
                        return false;
                    }
                    // Forward path: model `font: String` → tree
                    // `region.font: Option<AppFont>`; the reverse
                    // path uses `family_name_of`. Empty model
                    // font and `None` AppFont collide on
                    // "no pin", so equate them here.
                    let region_font_name = region.font.and_then(family_name_of);
                    let model_font_name: Option<&str> = if run.font.is_empty() {
                        None
                    } else {
                        Some(run.font.as_str())
                    };
                    region_font_name == model_font_name
                });
            if section.text == snapshot.text && model_regions_match {
                continue;
            }

            // Build the new run list by merging each tree-side
            // region with the prior run sharing the **same range,
            // or the dominant overlapping range** when the
            // mutation resized / split / shifted the run boundary.
            // A range-strict lookup loses every prior styling
            // (bold / italic / underline / size_pt / hyperlink)
            // on `ChangeRegionRange`-style mutations because no
            // prior matches the new range exactly; the overlap
            // fallback inherits from the prior whose intersection
            // is largest, preserving authored styling across
            // range edits.
            let prior_runs: Vec<&TextRun> = section.text_runs.iter().collect();
            let new_runs: Vec<TextRun> = snapshot
                .regions
                .iter()
                .map(|region| {
                    let prior = exact_or_dominant_overlap(&prior_runs, region.range.start, region.range.end);
                    region_to_text_run(region, prior)
                })
                .collect();

            section.text = snapshot.text;
            section.text_runs = new_runs;
            // Ensure no run extends past the new grapheme count —
            // `clamp_runs_to_text` is already idempotent on
            // already-clean run lists.
            clamp_runs_to_text(section);
        }
    }
}

#[cfg(test)]
mod region_converter_tests {
    use super::{exact_or_dominant_overlap, region_to_text_run, DEFAULT_TEXT_RUN_COLOR, DEFAULT_TEXT_RUN_SIZE_PT};
    use baumhard::core::primitives::{ColorFontRegion, Range};
    use baumhard::mindmap::model::TextRun;

    fn run(start: usize, end: usize, color: &str, font: &str) -> TextRun {
        TextRun {
            start,
            end,
            bold: false,
            italic: false,
            underline: false,
            font: font.into(),
            size_pt: 14,
            color: color.into(),
            hyperlink: None,
        }
    }

    fn styled_run(start: usize, end: usize) -> TextRun {
        TextRun {
            start,
            end,
            bold: true,
            italic: true,
            underline: true,
            font: "LiberationSans".into(),
            size_pt: 21,
            color: "#aabbcc".into(),
            hyperlink: Some("https://example.org".into()),
        }
    }

    #[test]
    fn region_to_text_run_merges_with_prior() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, Some([1.0, 0.0, 0.0, 1.0]));
        let prior = styled_run(0, 5);
        let out = region_to_text_run(&region, Some(&prior));
        assert_eq!(out.start, 0);
        assert_eq!(out.end, 5);
        assert_eq!(out.color, "#ff0000");
        assert!(out.bold);
        assert!(out.italic);
        assert!(out.underline);
        assert_eq!(out.size_pt, 21);
        assert_eq!(out.hyperlink.as_deref(), Some("https://example.org"));
    }

    #[test]
    fn region_to_text_run_falls_back_to_defaults_without_prior() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, None);
        let out = region_to_text_run(&region, None);
        assert!(!out.bold);
        assert!(!out.italic);
        assert!(!out.underline);
        assert_eq!(out.size_pt, DEFAULT_TEXT_RUN_SIZE_PT);
        assert_eq!(out.hyperlink, None);
        assert_eq!(out.font, "");
        assert_eq!(out.color, DEFAULT_TEXT_RUN_COLOR);
    }

    #[test]
    fn region_to_text_run_uses_region_color_without_prior() {
        let region = ColorFontRegion::new(Range::new(0, 3), None, Some([0.0, 1.0, 0.0, 1.0]));
        let out = region_to_text_run(&region, None);
        assert_eq!(out.color, "#00ff00");
    }

    #[test]
    fn region_to_text_run_preserves_var_color_when_range_matches() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, Some([1.0, 0.0, 0.0, 1.0]));
        let prior_with_var = TextRun {
            color: "var(--accent)".into(),
            ..styled_run(0, 5)
        };
        let out = region_to_text_run(&region, Some(&prior_with_var));
        assert_eq!(out.color, "var(--accent)");
    }

    #[test]
    fn region_to_text_run_loses_var_color_on_range_change() {
        let region = ColorFontRegion::new(Range::new(0, 3), None, Some([1.0, 0.0, 0.0, 1.0]));
        let prior_with_var = TextRun {
            color: "var(--accent)".into(),
            ..styled_run(0, 5)
        };
        let out = region_to_text_run(&region, Some(&prior_with_var));
        assert_eq!(out.color, "#ff0000");
    }

    #[test]
    fn exact_overlap_match_wins_over_partial() {
        let r1 = run(0, 5, "#aabbcc", "");
        let r2 = run(2, 7, "#ddeeff", "");
        let priors = vec![&r1, &r2];
        let hit = exact_or_dominant_overlap(&priors, 0, 5).expect("exact match");
        assert_eq!(hit.color, "#aabbcc");
    }

    #[test]
    fn dominant_overlap_wins_when_no_exact_match() {
        let small = run(0, 1, "#000000", "");
        let large = run(0, 4, "#ffffff", "");
        let priors = vec![&small, &large];
        let hit = exact_or_dominant_overlap(&priors, 0, 5).expect("partial overlap");
        assert_eq!(hit.color, "#ffffff");
    }

    #[test]
    fn no_overlap_returns_none() {
        let r1 = run(0, 5, "#aabbcc", "");
        let priors = vec![&r1];
        assert!(exact_or_dominant_overlap(&priors, 10, 15).is_none());
    }
}
