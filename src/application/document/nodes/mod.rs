// SPDX-License-Identifier: MPL-2.0

//! Per-node mutations and the shared edit-shape primitives they
//! route through. The setters in this directory cover everything
//! the user-facing console can write to a single `MindNode`:
//! text + per-run formatting, background / frame / text colour,
//! font face, font size, zoom-visibility window, and the bundled
//! frame-config edits in [`border`]. Every setter follows the
//! same pattern — capture the prior state into an [`UndoAction`]
//! envelope, mutate, set `dirty`, return whether anything actually
//! changed — so the console layer can phrase "no-op" vs. "applied"
//! uniformly without re-reading the model.
//!
//! The split into sub-modules tracks concept, not size:
//! [`option_edit`] owns the triple-state edit primitive
//! ([`OptionEdit`]) and the field-level fold helpers
//! (`apply_option_edit` / `apply_value_set` / `apply_string_set`)
//! that consume it; [`border`] owns the border-config bundle
//! ([`BorderConfigEdits`]) and the apply pipeline that lands it on
//! `MindNode.style.border`. What's left in this `mod.rs` is the
//! suite of single-field text/color/font/zoom setters plus the
//! private `set_node_style_field` helper they share.

use baumhard::mindmap::model::{NodeStyle, TextRun};

use super::grow_one_node_to_fit_border;
use super::undo_action::UndoAction;
use super::MindMapDocument;

mod border;
mod option_edit;

pub use border::{BorderConfigEdits, BorderEditOutcome, BorderSide};
pub use option_edit::OptionEdit;

impl MindMapDocument {
    /// Replace a node's `text` and collapse its `text_runs` to a single
    /// run inheriting the first original run's formatting (font,
    /// size_pt, color, bold, italic, underline). If the original had
    /// no runs, a white 24pt Liberation Sans run is synthesized —
    /// mirrors `default_orphan_node`.
    ///
    /// Returns `true` if the value actually changed. No-op / no undo
    /// push on unchanged text, matching `set_edge_label`'s contract.
    ///
    /// **Collapse caveat**: authored multi-run nodes lose their per-span
    /// formatting on any edit — a future per-run splitter would preserve
    /// it, but until then the editor path is single-run.
    /// Replace one section's `text` and collapse its `text_runs`
    /// to a single run inheriting the first original run's
    /// formatting. Returns `true` when the value actually changed.
    /// Section-aware sibling of [`Self::set_node_text`] — the
    /// latter's contract is preserved by routing through here
    /// with `section_idx = 0`.
    ///
    /// No-op (returns `false`, no undo push) when the section
    /// doesn't exist or its text already matches.
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
        super::grow_one_node_to_fit_text(node);
        super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        self.undo_stack.push(UndoAction::EditNodeText {
            node_id: node_id.to_string(),
            before_sections,
        });
        self.dirty = true;
        true
    }

    /// Set the text colour on one section's runs, mirroring the
    /// whole-node [`Self::set_node_text_color`] but bounded to a
    /// single section. Per-run colour overrides authored on
    /// matching colours (`run.color == old_default`) are rewritten;
    /// runs the user explicitly coloured by hand keep their
    /// override. The owning node's `style.text_color` is *not*
    /// touched — that's the node-level default and a per-section
    /// override doesn't change its meaning.
    ///
    /// No-op when the section is missing or every targeted run
    /// already carries the new colour.
    pub fn set_section_text_color(&mut self, node_id: &str, section_idx: usize, color: String) -> bool {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let Some(section) = node.sections.get(section_idx) else {
            return false;
        };
        let old_default = node.style.text_color.clone();
        let any_run_changes = section
            .text_runs
            .iter()
            .any(|r| r.color == old_default && r.color != color);
        if !any_run_changes {
            return false;
        }
        let before_style = node.style.clone();
        let before_sections = node.sections.clone();
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        if let Some(section) = node.sections.get_mut(section_idx) {
            for run in section.text_runs.iter_mut() {
                if run.color == old_default {
                    run.color = color.clone();
                }
            }
        }
        self.undo_stack.push(UndoAction::EditNodeStyle {
            node_id: node_id.to_string(),
            before_style,
            before_sections,
        });
        self.dirty = true;
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
        let before_style = node.style.clone();
        let before_sections = node.sections.clone();
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        if let Some(section) = node.sections.get_mut(section_idx) {
            for run in section.text_runs.iter_mut() {
                run.size_pt = size_u;
            }
        }
        super::grow_one_node_to_fit_text(node);
        super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        self.undo_stack.push(UndoAction::EditNodeStyle {
            node_id: node_id.to_string(),
            before_style,
            before_sections,
        });
        self.dirty = true;
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
        let before_style = node.style.clone();
        let before_sections = node.sections.clone();
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        if let Some(section) = node.sections.get_mut(section_idx) {
            for run in section.text_runs.iter_mut() {
                run.font = target.to_string();
            }
        }
        super::grow_one_node_to_fit_text(node);
        super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        self.undo_stack.push(UndoAction::EditNodeStyle {
            node_id: node_id.to_string(),
            before_style,
            before_sections,
        });
        self.dirty = true;
        true
    }

    pub fn set_node_text(&mut self, node_id: &str, new_text: String) -> bool {
        // Validate + capture under an immutable borrow so the mutable
        // re-acquisition below can coexist with the canvas-default
        // clone (which would otherwise overlap the borrow held by
        // an upfront `get_mut`).
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        // Pre-section-refactor this setter wrote `node.text`; post-
        // refactor it writes the *first* section's text. Multi-
        // section nodes only have their first section edited here —
        // the per-section UX surface lives in the follow-up commit;
        // the data model already supports addressing by index.
        let Some(first_section) = node.sections.first() else {
            return false;
        };
        if first_section.text == new_text {
            return false;
        }
        let before_sections = node.sections.clone();
        // Collapse the first section to a single run spanning the new
        // text. Inherit formatting from the first original run on that
        // section, or fall back to the default-orphan defaults.
        let template = first_section
            .text_runs
            .first()
            .cloned()
            .unwrap_or_else(|| TextRun {
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
        if let Some(section) = node.sections.first_mut() {
            section.text = new_text;
            section.text_runs = new_runs;
        }
        // Re-fit the box on text change for the same reason
        // `set_node_font_size` / `set_node_font_family` do: longer
        // text on the same face overflows the right edge, and the
        // monotonic floor only applies if we measure here. Border
        // floor runs after because a wider node may also need a
        // wider frame.
        super::grow_one_node_to_fit_text(node);
        super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        self.undo_stack.push(UndoAction::EditNodeText {
            node_id: node_id.to_string(),
            before_sections,
        });
        self.dirty = true;
        true
    }

    /// Set the background color on a node's `style.background_color`.
    /// Returns `true` if the value actually changed. Pushes one
    /// `UndoAction::EditNodeStyle` entry so undo restores both the
    /// `NodeStyle` *and* the `text_runs` (unchanged for this setter,
    /// but the variant always carries both so the undo arm has a
    /// single shape).
    ///
    /// No-op on missing node id, matching the `EditEdge` pattern.
    pub fn set_node_bg_color(&mut self, node_id: &str, color: String) -> bool {
        set_node_style_field(self, node_id, |s| {
            if s.background_color == color {
                return false;
            }
            s.background_color = color;
            true
        })
    }

    /// Set the frame (border) color on a node's `style.frame_color`.
    /// Returns `true` on change.
    pub fn set_node_border_color(&mut self, node_id: &str, color: String) -> bool {
        set_node_style_field(self, node_id, |s| {
            if s.frame_color == color {
                return false;
            }
            s.frame_color = color;
            true
        })
    }

    /// Set the *default* text color on a node. Writes
    /// `style.text_color` directly, and for every `TextRun` whose
    /// `color` matches the pre-edit default, rewrites that run's
    /// `color` to the new value — so a node whose runs all inherited
    /// the default gets visually recolored, while runs the user
    /// explicitly colored by hand keep their per-span override.
    ///
    /// The match is byte-exact on the pre-edit `style.text_color`
    /// string. This is deliberately strict: if the user wrote
    /// `"#FFFFFF"` (uppercase) as the default but an authored run
    /// carries `"#ffffff"`, the run is *not* considered
    /// default-following and keeps its lowercase override. Matches the
    /// convention in `baumhard::util::color::hex_to_rgba_safe` —
    /// colors are strings in the model and comparisons are literal.
    pub fn set_node_text_color(&mut self, node_id: &str, color: String) -> bool {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let old_default = node.style.text_color.clone();
        let any_run_changes = node
            .sections
            .iter()
            .flat_map(|s| s.text_runs.iter())
            .any(|r| r.color == old_default && r.color != color);
        if old_default == color && !any_run_changes {
            return false;
        }
        let before_style = node.style.clone();
        let before_sections = node.sections.clone();
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        node.style.text_color = color.clone();
        for section in node.sections.iter_mut() {
            clamp_runs_to_text(section);
            for run in section.text_runs.iter_mut() {
                if run.color == old_default {
                    run.color = color.clone();
                }
            }
        }
        self.undo_stack.push(UndoAction::EditNodeStyle {
            node_id: node_id.to_string(),
            before_style,
            before_sections,
        });
        self.dirty = true;
        true
    }

    /// Set the *default* font size on a node. Rewrites every
    /// `TextRun.size_pt` to `size_pt` — the node's runs all track
    /// the same size-in-points; unlike text color, there is no
    /// natural "keep per-run override" rule here (authored multi-
    /// size runs would already have been flattened by the text
    /// editor's collapse step in `set_node_text`).
    ///
    /// `size_pt` is rounded to the nearest positive integer; values
    /// below 1 clamp to 1.
    pub fn set_node_font_size(&mut self, node_id: &str, size_pt: f32) -> bool {
        let size_u = size_pt.round().max(1.0) as u32;
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let already = node
            .sections
            .iter()
            .flat_map(|s| s.text_runs.iter())
            .all(|r| r.size_pt == size_u);
        if already {
            return false;
        }
        let before_style = node.style.clone();
        let before_sections = node.sections.clone();
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        for section in node.sections.iter_mut() {
            clamp_runs_to_text(section);
            for run in section.text_runs.iter_mut() {
                run.size_pt = size_u;
            }
        }
        // Larger text needs a larger box. Same monotonic floor as
        // `set_node_font_family`: grow on demand, never shrink.
        super::grow_one_node_to_fit_text(node);
        super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        self.undo_stack.push(UndoAction::EditNodeStyle {
            node_id: node_id.to_string(),
            before_style,
            before_sections,
        });
        self.dirty = true;
        true
    }

    /// Set the font family on every `TextRun` of `node_id` to
    /// `family`. Returns `true` if any run actually changed.
    ///
    /// `Some(name)` pins each run to that family; `None` clears the
    /// pin by writing an empty string into each `TextRun.font` —
    /// which the tree builder treats as "fall back to the document
    /// default at render time" (`baumhard::mindmap::tree_builder::node`
    /// resolves empty-string font as `None` on the
    /// `ColorFontRegion`). Family-name validation is the caller's
    /// job; an unknown family lands in the data model and degrades
    /// at render time per CODE_CONVENTIONS §9.
    ///
    /// Capture / undo: piggybacks on the existing
    /// `UndoAction::EditNodeStyle` envelope (which already includes
    /// the full `text_runs` snapshot via `before_runs`), so a
    /// `font set` on a node is reversed by the same `undo()` arm
    /// that reverses every other node-style edit. No new
    /// `UndoAction` variant.
    pub fn set_node_font_family(&mut self, node_id: &str, family: Option<&str>) -> bool {
        let target = family.unwrap_or("");
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let already = node
            .sections
            .iter()
            .flat_map(|s| s.text_runs.iter())
            .all(|r| r.font.as_str() == target);
        if already {
            return false;
        }
        let before_style = node.style.clone();
        let before_sections = node.sections.clone();
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        for section in node.sections.iter_mut() {
            clamp_runs_to_text(section);
            for run in section.text_runs.iter_mut() {
                run.font = target.to_string();
            }
        }
        // Re-measure the node's text in the new face. Fonts vary
        // wildly in advance width — pinning a wide display face on
        // a node previously sized for a narrow monospace would clip
        // the text against the right edge. Same monotonic floor the
        // text loader enforces: grow if the new measurement exceeds
        // the current size; never shrink. The border floor runs
        // after because a wider node may also need a wider frame.
        super::grow_one_node_to_fit_text(node);
        super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        self.undo_stack.push(UndoAction::EditNodeStyle {
            node_id: node_id.to_string(),
            before_style,
            before_sections,
        });
        self.dirty = true;
        true
    }

    /// Write the node's zoom-visibility window. Each of `min` /
    /// `max` is an [`OptionEdit<f32>`]: `Keep` leaves the side
    /// untouched, `Clear` sets it to `None` (unbounded), `Set(v)`
    /// sets it to `Some(v)`. Returns `true` if either side
    /// actually changed.
    ///
    /// Inversion (`min > max` after the edit) is rejected as a
    /// no-op with `false`; the console surface catches this first,
    /// so this is a defensive guard for programmatic callers.
    /// Non-finite values are likewise rejected — the invariant
    /// mirrors
    /// [`ZoomVisibility::try_new`](baumhard::gfx_structs::zoom_visibility::ZoomVisibility::try_new).
    pub fn set_node_zoom_visibility(
        &mut self,
        node_id: &str,
        min: OptionEdit<f32>,
        max: OptionEdit<f32>,
    ) -> bool {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let before_min = node.min_zoom_to_render;
        let before_max = node.max_zoom_to_render;
        let new_min = min.apply(before_min);
        let new_max = max.apply(before_max);
        if !validate_zoom_pair(new_min, new_max) {
            return false;
        }
        if new_min == before_min && new_max == before_max {
            return false;
        }
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        node.min_zoom_to_render = new_min;
        node.max_zoom_to_render = new_max;
        self.undo_stack.push(UndoAction::EditNodeZoom {
            node_id: node_id.to_string(),
            before_min,
            before_max,
        });
        self.dirty = true;
        true
    }
}

/// Guard used by every `set_*_zoom_visibility` setter. Rejects a
/// pair whose bounds are non-finite or whose resolved
/// `(min, max)` inverts. Mirrors the contract the verifier
/// enforces at load time and `ZoomVisibility::try_new` enforces
/// for programmatic callers — no panic in interactive paths per
/// `CODE_CONVENTIONS.md` §9.
/// Clamp a section's `text_runs` against its current text length
/// in grapheme clusters, dropping runs that became degenerate
/// (`start >= end`) and shrinking trailing runs that overshoot the
/// text. Defensive guard the per-section style setters call before
/// rewriting `color` / `size_pt` / `font` on each run — a previous
/// tree-walker mutation that shortened `section.text` may have
/// left runs whose `end` exceeds the current grapheme count, which
/// `cosmic_text` either ignores or panics on depending on build.
///
/// Cost: O(runs.len() * text grapheme count) — one
/// `count_grapheme_clusters` call per section, plus a linear pass
/// over the runs. Trivial for typical single-run sections.
fn clamp_runs_to_text(section: &mut baumhard::mindmap::model::MindSection) {
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

pub(super) fn validate_zoom_pair(min: Option<f32>, max: Option<f32>) -> bool {
    if let Some(m) = min {
        if !m.is_finite() {
            return false;
        }
    }
    if let Some(m) = max {
        if !m.is_finite() {
            return false;
        }
    }
    if let (Some(lo), Some(hi)) = (min, max) {
        if lo > hi {
            return false;
        }
    }
    true
}

/// Shared body of the node-style setters that touch a single field on
/// `NodeStyle` and nothing else. `mutate` returns `true` when it
/// actually changed something; on `false` no undo is pushed and the
/// style is left untouched. Keeps the trait-facing setters terse.
pub(super) fn set_node_style_field(
    doc: &mut MindMapDocument,
    node_id: &str,
    mutate: impl FnOnce(&mut NodeStyle) -> bool,
) -> bool {
    let node = match doc.mindmap.nodes.get_mut(node_id) {
        Some(n) => n,
        None => return false,
    };
    let before_style = node.style.clone();
    let before_sections = node.sections.clone();
    if !mutate(&mut node.style) {
        return false;
    }
    doc.undo_stack.push(UndoAction::EditNodeStyle {
        node_id: node_id.to_string(),
        before_style,
        before_sections,
    });
    doc.dirty = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::{
        first_testament_node_id as first_node_id, load_test_doc as fixture_doc,
    };

    /// `BorderConfigEdits::with_side_pattern` validates the
    /// pattern *before* mutating the bundle — a parse error
    /// leaves the slot untouched so a half-applied edit can't
    /// leak into the document. Critical for the verb's atomic
    /// contract.
    #[test]
    fn with_side_pattern_rejects_bad_input_without_mutation() {
        let mut edits = BorderConfigEdits::default();
        let err = edits
            .with_side_pattern(BorderSide::Top, "a)b")
            .expect_err("unmatched ')' must error");
        assert!(err.contains("top:"), "missing prefix: {}", err);
        assert!(matches!(edits.side_top, OptionEdit::Keep));
    }

    /// Setting a side pattern auto-promotes the preset to
    /// `"custom"` and surfaces that through `BorderEditOutcome`.
    /// The console verb consumes the `preset_auto_promoted` flag
    /// to print a note; this test guards the document-layer
    /// signal independently.
    #[test]
    fn set_node_border_config_signals_preset_auto_promotion() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        let mut edits = BorderConfigEdits::default();
        edits.preset = OptionEdit::Set("heavy".into());
        edits
            .with_side_pattern(BorderSide::Top, "###(*)###")
            .expect("pattern parses");
        let outcome = doc.set_node_border_config(&id, edits);
        assert!(outcome.changed, "expected change applied");
        assert!(
            outcome.preset_auto_promoted,
            "side override against preset=heavy must auto-promote"
        );
        assert_eq!(outcome.requested_preset.as_deref(), Some("heavy"));
        let cfg = doc
            .mindmap
            .nodes
            .get(&id)
            .unwrap()
            .style
            .border
            .as_ref()
            .expect("config materialised");
        assert_eq!(cfg.preset, "custom");
    }

    /// `set_node_border_config` writes through the existing
    /// `EditNodeStyle` undo envelope so the next `undo()`
    /// restores the pre-edit `style.border`. Round-trip test:
    /// apply an edit, undo, confirm the override is gone (or
    /// matches its prior value).
    #[test]
    fn set_node_border_config_undo_round_trip_restores_style() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        let before_border = doc.mindmap.nodes.get(&id).unwrap().style.border.clone();
        let mut edits = BorderConfigEdits::default();
        edits.preset = OptionEdit::Set("double".into());
        let outcome = doc.set_node_border_config(&id, edits);
        assert!(outcome.changed);
        // Sanity: the edit landed.
        assert_eq!(
            doc.mindmap
                .nodes
                .get(&id)
                .unwrap()
                .style
                .border
                .as_ref()
                .map(|c| c.preset.clone()),
            Some("double".to_string()),
        );
        // Now reverse.
        assert!(doc.undo(), "undo must succeed");
        let after_border = doc.mindmap.nodes.get(&id).unwrap().style.border.clone();
        assert_eq!(
            before_border.as_ref().map(|c| c.preset.clone()),
            after_border.as_ref().map(|c| c.preset.clone()),
            "undo must restore the pre-edit preset"
        );
    }

    /// `set_node_border_config` with `clear=true` on a node that
    /// already has no border override is a no-op — no undo
    /// entry, no `dirty` flag flip, returns `changed=false`.
    /// Guards the early-return branch.
    #[test]
    fn set_node_border_config_clear_no_op_when_already_none() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        // Strip any pre-existing override.
        doc.mindmap.nodes.get_mut(&id).unwrap().style.border = None;
        doc.dirty = false;
        let undo_len_before = doc.undo_stack.len();
        let mut edits = BorderConfigEdits::default();
        edits.clear = true;
        let outcome = doc.set_node_border_config(&id, edits);
        assert!(!outcome.changed);
        assert!(!doc.dirty, "no-op clear must not mark the document dirty");
        assert_eq!(
            doc.undo_stack.len(),
            undo_len_before,
            "no-op clear must not push an undo entry"
        );
    }

    /// `set_node_border_visible` toggles `style.show_frame` and
    /// returns `true` iff the value changed. Sibling test of
    /// the `set_*` patterns elsewhere in this module.
    #[test]
    fn set_node_border_visible_returns_true_only_on_change() {
        let mut doc = fixture_doc();
        let id = first_node_id(&doc);
        // Force a known starting state.
        doc.mindmap.nodes.get_mut(&id).unwrap().style.show_frame = false;
        assert!(doc.set_node_border_visible(&id, true));
        assert!(doc.mindmap.nodes.get(&id).unwrap().style.show_frame);
        // Second call same value → no-op.
        assert!(!doc.set_node_border_visible(&id, true));
    }
}
