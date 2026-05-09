// SPDX-License-Identifier: MPL-2.0

//! Section text / colour / font / runs / payload setters. Every
//! setter in this file routes through `mutate_section_with_style_undo`
//! (single `EditNodeStyle` undo envelope) and most call the
//! `mutate_section_runs_in_range<F>` helper for range-targeted
//! mutations.

use baumhard::mindmap::model::TextRun;

use super::super::undo_action::UndoAction;
use super::super::MindMapDocument;
use super::SectionPayload;

impl MindMapDocument {
    /// Snapshot + mutate + undo plumbing shared by every section
    /// setter that uses the `EditNodeStyle` undo envelope. The
    /// caller verifies the section exists, then hands the actual
    /// field write here as a closure that returns `true` if the
    /// mutation actually changed anything (or `false` to declare a
    /// no-op). On `true`, this fn snapshots `node.style` +
    /// `node.sections` into a single undo entry and flips `dirty`;
    /// on `false`, neither happens — the section's pre-mutation
    /// state is restored from the snapshot taken before the
    /// closure ran. This shape lets a caller tell the helper
    /// "mutate, but back it out if it ends up a no-op" without the
    /// caller having to itself snapshot + post-hoc `undo_stack.pop`
    /// (which doesn't restore `dirty` and breaks the undo-LIFO
    /// invariant if any other entry slips between push and pop).
    /// Callers that need post-write auto-fit
    /// (`grow_one_node_to_fit_text` / `_border`) re-acquire the
    /// node and run them; helper deliberately stays out of that
    /// decision so colour-only setters (`set_section_text_color`)
    /// skip the cost.
    ///
    /// Returns the closure's verdict so the caller can chain
    /// post-write fix-ups conditionally.
    pub(super) fn mutate_section_with_style_undo<F>(
        &mut self,
        node_id: &str,
        section_idx: usize,
        mutate: F,
    ) -> bool
    where
        F: FnOnce(&mut baumhard::mindmap::model::MindSection) -> bool,
    {
        let node = self
            .mindmap
            .nodes
            .get(node_id)
            .expect("caller verified node exists");
        let before_style = node.style.clone();
        let before_sections = node.sections.clone();
        let before_position = node.position;
        let before_size = node.size;
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        let changed = mutate(&mut node.sections[section_idx]);
        if !changed {
            // Closure declared a no-op — restore the pre-mutation
            // section state from the snapshot we already cloned and
            // skip the undo-entry / dirty bookkeeping. This is
            // cheaper than a second clone-and-compare and avoids
            // the `undo_stack.pop()` anti-pattern callers used to
            // reach for.
            let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
            node.sections = before_sections;
            return false;
        }
        self.undo_stack.push(UndoAction::EditNodeStyle {
            node_id: node_id.to_string(),
            before_style,
            before_sections,
            before_position,
            before_size,
        });
        self.dirty = true;
        true
    }

    /// Replace one section's `text` and collapse its `text_runs`
    /// to a single run inheriting the first original run's
    /// Write both `text` and `text_runs` atomically, merging the
    /// editor's `ColorFontRegions` back to `Vec<TextRun>` via
    /// `region_to_text_run` so per-run attributes the regions
    /// don't carry (bold / italic / underline / hyperlink) survive
    /// the round trip.
    pub fn set_section_text_and_runs(
        &mut self,
        node_id: &str,
        section_idx: usize,
        new_text: String,
        new_regions: &baumhard::core::primitives::ColorFontRegions,
    ) -> bool {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let Some(section) = node.sections.get(section_idx) else {
            return false;
        };
        // Empty regions: fall back to `set_section_text` so a
        // plaintext-only edit doesn't wipe template-inherited
        // runs the editor never touched.
        if new_regions.all_regions().is_empty() {
            return self.set_section_text(node_id, section_idx, new_text);
        }
        let prior_runs: Vec<&TextRun> = section.text_runs.iter().collect();
        let new_runs: Vec<TextRun> = new_regions
            .all_regions()
            .iter()
            .map(|region| {
                let prior = super::super::custom::sync::exact_or_dominant_overlap(
                    &prior_runs,
                    region.range.start,
                    region.range.end,
                );
                super::super::custom::sync::region_to_text_run(region, prior)
            })
            .collect();
        if section.text == new_text && section.text_runs == new_runs {
            return false;
        }
        let before_sections = node.sections.clone();
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        if let Some(section) = node.sections.get_mut(section_idx) {
            section.text = new_text;
            section.text_runs = new_runs;
            clamp_runs_to_text(section);
        }
        super::super::grow_one_node_to_fit_text(node);
        super::super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        self.undo_stack.push(UndoAction::EditNodeText {
            node_id: node_id.to_string(),
            before_sections,
        });
        self.dirty = true;
        true
    }

    /// No-op (returns `false`, no undo push) when the section
    /// doesn't exist or its text already matches.
    /// Replace the section's `text` while preserving as much of
    /// the existing `text_runs` as the new text supports. Runs
    /// wholly inside the new text length carry through unchanged;
    /// runs that straddle the new end get clipped at the new
    /// `grapheme_count`; runs entirely past the new end are
    /// dropped. Uncovered ranges (anything past the last surviving
    /// run's `end`) fall through to section / node defaults per
    /// `format/text-runs.md`.
    ///
    /// Distinct from [`Self::set_section_text`] which collapses
    /// every prior run to a single run cloned from
    /// `text_runs.first()` — that path is the right shape for
    /// "I want one uniform style on the new text"; this path is
    /// the right shape for "I want my multi-run styling to
    /// survive a text rewrite to the extent the new text covers
    /// the same graphemes".
    ///
    /// Backs the `section text "<text>" runs=preserve` console
    /// path. Pre-fix the verb claimed preserve mode but called
    /// `set_section_text` (which collapses), making the kv a
    /// phantom. Plan §4.5 §9.8.
    pub fn set_section_text_preserving_runs(
        &mut self,
        node_id: &str,
        section_idx: usize,
        new_text: String,
    ) -> bool {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let Some(section) = node.sections.get(section_idx) else {
            return false;
        };
        if section.text == new_text {
            return false;
        }
        let new_grapheme_count = baumhard::util::grapheme_chad::count_grapheme_clusters(&new_text);
        // Clip runs to the new text length: keep runs whose
        // `start < new_grapheme_count`; clamp `end` down to
        // `new_grapheme_count`. Runs entirely past the new end
        // (start >= new_grapheme_count) drop out. The
        // text_run_ops invariants (sorted, no-overlap, half-open)
        // are preserved by clamping in-place.
        let new_runs: Vec<TextRun> = section
            .text_runs
            .iter()
            .filter(|r| r.start < new_grapheme_count)
            .map(|r| {
                let mut clipped = r.clone();
                if clipped.end > new_grapheme_count {
                    clipped.end = new_grapheme_count;
                }
                clipped
            })
            // After clamping, a run with start == end is degenerate;
            // filter it out (the clamp can collapse a run when the
            // new text ends exactly at the run's start).
            .filter(|r| r.start < r.end)
            .collect();

        let before_sections = node.sections.clone();
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        if let Some(section) = node.sections.get_mut(section_idx) {
            section.text = new_text;
            section.text_runs = new_runs;
        }
        super::super::grow_one_node_to_fit_text(node);
        super::super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        self.undo_stack.push(UndoAction::EditNodeText {
            node_id: node_id.to_string(),
            before_sections,
        });
        self.dirty = true;
        true
    }

    pub fn set_section_text(&mut self, node_id: &str, section_idx: usize, new_text: String) -> bool {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let Some(section) = node.sections.get(section_idx) else {
            return false;
        };
        if section.text == new_text {
            return false;
        }
        let before_sections = node.sections.clone();
        let template = section.text_runs.first().cloned().unwrap_or_else(|| TextRun {
            start: 0,
            end: 0,
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".to_string(),
            size_pt: 24,
            color: "#ffffff".to_string(),
            hyperlink: None,
        });
        let new_runs = vec![TextRun {
            start: 0,
            end: baumhard::util::grapheme_chad::count_grapheme_clusters(&new_text),
            ..template
        }];
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        if let Some(section) = node.sections.get_mut(section_idx) {
            section.text = new_text;
            section.text_runs = new_runs;
        }
        super::super::grow_one_node_to_fit_text(node);
        super::super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        self.undo_stack.push(UndoAction::EditNodeText {
            node_id: node_id.to_string(),
            before_sections,
        });
        self.dirty = true;
        true
    }

    /// Rewrite every run on the section that matches the cascade
    /// predicate (unanimous run colour, or the node's
    /// `style.text_color` default) to `color`. Mixed-colour
    /// sections preserve their non-predicate runs. The node's own
    /// `style.text_color` is never touched.
    pub fn set_section_text_color(&mut self, node_id: &str, section_idx: usize, color: String) -> bool {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let Some(section) = node.sections.get(section_idx) else {
            return false;
        };
        let predicate_color = section
            .text_runs
            .first()
            .filter(|first| section.text_runs.iter().all(|r| r.color == first.color))
            .map(|r| r.color.clone())
            .unwrap_or_else(|| node.style.text_color.clone());
        let any_run_changes = section
            .text_runs
            .iter()
            .any(|r| r.color == predicate_color && r.color != color);
        if !any_run_changes {
            return false;
        }
        self.mutate_section_with_style_undo(node_id, section_idx, |s| {
            for run in s.text_runs.iter_mut() {
                if run.color == predicate_color {
                    run.color = color.clone();
                }
            }
            true
        });
        true
    }

    /// Set the font size on one section's runs (bounded sibling
    /// of the whole-node [`Self::set_node_font_size`]). Rewrites
    /// every run's `size_pt` on the targeted section; sibling
    /// sections stay untouched. Triggers the same monotonic
    /// `grow_one_node_to_fit_text` floor as the whole-node setter
    /// — sections share the node's AABB, so a larger run on one
    /// section can grow the node.
    pub fn set_section_font_size(&mut self, node_id: &str, section_idx: usize, size_pt: f32) -> bool {
        if !size_pt.is_finite() {
            return false;
        }
        let size_u = size_pt.round().max(1.0) as u32;
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let Some(section) = node.sections.get(section_idx) else {
            return false;
        };
        let already = section.text_runs.iter().all(|r| r.size_pt == size_u);
        if already {
            return false;
        }
        let canvas_default = self.mindmap.canvas.default_border.clone();
        self.mutate_section_with_style_undo(node_id, section_idx, |s| {
            for run in s.text_runs.iter_mut() {
                run.size_pt = size_u;
            }
            true
        });
        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("just confirmed exists");
        super::super::grow_one_node_to_fit_text(node);
        super::super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        true
    }

    /// Set the font family on one section's runs (bounded sibling
    /// of the whole-node [`Self::set_node_font_family`]).
    /// `Some(name)` pins each run to that family on the targeted
    /// section; `None` clears the pin. Triggers the same monotonic
    /// `grow_one_node_to_fit_text` re-measure as the whole-node
    /// setter — face changes can shift advance widths.
    pub fn set_section_font_family(
        &mut self,
        node_id: &str,
        section_idx: usize,
        family: Option<&str>,
    ) -> bool {
        let target = family.unwrap_or("");
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let Some(section) = node.sections.get(section_idx) else {
            return false;
        };
        let already = section.text_runs.iter().all(|r| r.font.as_str() == target);
        if already {
            return false;
        }
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let target_owned = target.to_string();
        self.mutate_section_with_style_undo(node_id, section_idx, |s| {
            for run in s.text_runs.iter_mut() {
                run.font = target_owned.clone();
            }
            true
        });
        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("just confirmed exists");
        super::super::grow_one_node_to_fit_text(node);
        super::super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        true
    }

    // ── Range-targeted section setters ─────────────────────────
    //
    // Range-aware mirrors of the uniform setters above; route
    // through `text_run_ops::mutate_in_range`.

    /// Set the text colour on a sub-range of one section's text.
    /// Bounded sibling of [`Self::set_section_text_color`] — that
    /// setter rewrites every run uniformly, this one targets
    /// `[range_start, range_end)` graphemes only. Ranges that
    /// partially or wholly cross uncovered gaps fill the gap
    /// with a fresh run inheriting the section / node cascade
    /// defaults plus the new colour, so the user's "make these
    /// graphemes red" intent is honoured even where no run
    /// exists today.
    ///
    /// `range_end` is clamped to the section's grapheme count;
    /// callers don't need to pre-clamp. No-op when the section
    /// is missing, the range is empty after clamping, or the
    /// post-mutation runs are unchanged from the pre-mutation
    /// runs.
    pub fn set_section_text_color_range(
        &mut self,
        node_id: &str,
        section_idx: usize,
        range_start: usize,
        range_end: usize,
        color: String,
    ) -> bool {
        // Text colour doesn't affect glyph advance — no grow.
        self.mutate_section_runs_in_range(node_id, section_idx, range_start, range_end, false, |r| {
            r.color = color.clone()
        })
    }

    /// Set the font size on a sub-range of one section's text.
    /// Triggers `grow_one_node_to_fit_text` — larger runs can
    /// grow the node.
    pub fn set_section_font_size_range(
        &mut self,
        node_id: &str,
        section_idx: usize,
        range_start: usize,
        range_end: usize,
        size_pt: f32,
    ) -> bool {
        if !size_pt.is_finite() {
            return false;
        }
        let size_u = size_pt.round().max(1.0) as u32;
        self.mutate_section_runs_in_range(node_id, section_idx, range_start, range_end, true, move |r| {
            r.size_pt = size_u
        })
    }

    /// Set the font family on a sub-range of one section's text.
    /// `Some(name)` pins each in-range run; `None` clears the pin
    /// (empty string = inherit cascade). Triggers grow — face
    /// changes shift advance widths.
    pub fn set_section_font_family_range(
        &mut self,
        node_id: &str,
        section_idx: usize,
        range_start: usize,
        range_end: usize,
        family: Option<&str>,
    ) -> bool {
        let target = family.unwrap_or("").to_string();
        self.mutate_section_runs_in_range(node_id, section_idx, range_start, range_end, true, move |r| {
            r.font = target.clone()
        })
    }

    /// Per-attribute range-aware setter shell. Clamps the range,
    /// snapshots pre-runs, applies `mutate_run` to every in-range
    /// run (and to the template that fills uncovered gaps), pops
    /// the undo entry on no-op-after-mutation, and optionally
    /// runs the text/border grow passes for attributes that can
    /// change advance widths.
    fn mutate_section_runs_in_range<F>(
        &mut self,
        node_id: &str,
        section_idx: usize,
        range_start: usize,
        range_end: usize,
        grow_after: bool,
        mut mutate_run: F,
    ) -> bool
    where
        F: FnMut(&mut baumhard::mindmap::model::TextRun),
    {
        let (clamped_end, mut template) =
            match self.clamp_range_and_build_template(node_id, section_idx, range_end) {
                Some(pair) => pair,
                None => return false,
            };
        if range_start >= clamped_end {
            return false;
        }
        mutate_run(&mut template);
        let pre = self
            .mindmap
            .nodes
            .get(node_id)
            .and_then(|n| n.sections.get(section_idx))
            .map(|s| s.text_runs.clone())
            .unwrap_or_default();
        let canvas_default = self.mindmap.canvas.default_border.clone();
        self.mutate_section_with_style_undo(node_id, section_idx, |s| {
            baumhard::mindmap::model::text_run_ops::mutate_in_range(
                &mut s.text_runs,
                range_start,
                clamped_end,
                &template,
                &mut mutate_run,
            );
            true
        });
        let post = self
            .mindmap
            .nodes
            .get(node_id)
            .and_then(|n| n.sections.get(section_idx))
            .map(|s| s.text_runs.clone())
            .unwrap_or_default();
        if pre == post {
            self.undo_stack.pop();
            return false;
        }
        if grow_after {
            let node = self
                .mindmap
                .nodes
                .get_mut(node_id)
                .expect("just confirmed exists");
            super::super::grow_one_node_to_fit_text(node);
            super::super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        }
        true
    }

    /// Range-setter pre-flight: clamps `range_end` to the
    /// section's grapheme count and builds the gap-fill
    /// template from the section's first run (cascade source)
    /// or hardcoded defaults when the section has no runs.
    /// Caller overwrites the one attribute it's setting.
    fn clamp_range_and_build_template(
        &self,
        node_id: &str,
        section_idx: usize,
        range_end: usize,
    ) -> Option<(usize, baumhard::mindmap::model::TextRun)> {
        let node = self.mindmap.nodes.get(node_id)?;
        let section = node.sections.get(section_idx)?;
        let total = baumhard::util::grapheme_chad::count_grapheme_clusters(&section.text);
        let clamped_end = range_end.min(total);
        let template =
            section
                .text_runs
                .first()
                .cloned()
                .unwrap_or_else(|| baumhard::mindmap::model::TextRun {
                    start: 0,
                    end: 0,
                    bold: false,
                    italic: false,
                    underline: false,
                    font: "LiberationSans".to_string(),
                    size_pt: 24,
                    color: node.style.text_color.clone(),
                    hyperlink: None,
                });
        Some((clamped_end, template))
    }

    /// Atomically replace one section's full payload (text +
    /// runs + offset + size + channel + bindings) under a single
    /// `EditNodeStyle` undo entry — a single Ctrl+Z restores the
    /// pre-write shape. Returns `true` on a real change; no-op
    /// when the section is missing or every field matches.
    pub fn apply_section_payload(
        &mut self,
        node_id: &str,
        section_idx: usize,
        text: String,
        payload: &SectionPayload,
    ) -> bool {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let Some(section) = node.sections.get(section_idx) else {
            return false;
        };
        let unchanged = section.text == text
            && section.text_runs == payload.text_runs
            && section.offset == payload.offset
            && section.size == payload.size
            && section.channel == payload.channel
            && section.trigger_bindings == payload.trigger_bindings;
        if unchanged {
            return false;
        }
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let payload = payload.clone();
        self.mutate_section_with_style_undo(node_id, section_idx, |s| {
            s.text = text;
            s.text_runs = payload.text_runs;
            s.offset = payload.offset;
            s.size = payload.size;
            s.channel = payload.channel;
            s.trigger_bindings = payload.trigger_bindings;
            // Defensive: a future caller might pass mismatched
            // (text, runs) — the copy site never does, but the
            // public setter shouldn't trust its input enough to
            // leave runs whose ranges exceed the new text length.
            clamp_runs_to_text(s);
            true
        });
        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("just confirmed exists");
        super::super::grow_one_node_to_fit_text(node);
        super::super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        true
    }
}

pub(in crate::application::document) fn clamp_runs_to_text(
    section: &mut baumhard::mindmap::model::MindSection,
) {
    let max_end = baumhard::util::grapheme_chad::count_grapheme_clusters(&section.text);
    section.text_runs.retain_mut(|run| {
        if run.start >= max_end {
            return false;
        }
        if run.end > max_end {
            run.end = max_end;
        }
        run.start < run.end
    });
}
