// SPDX-License-Identifier: MPL-2.0

//! Per-node frame configuration: visibility flag plus the
//! `GlyphBorderConfig` overrides ([`BorderConfigEdits`]) that the
//! console's `border` verb stages and applies atomically. This file
//! owns the edit-bundle struct, the side selector, the
//! [`MindMapDocument`] setters that consume them, and the private
//! plumbing that folds a bundle into the `MindNode.style.border`
//! slot — including the auto-promotion of `preset` to `"custom"`
//! whenever a side- or corner-glyph edit is staged against a
//! built-in preset.

use baumhard::mindmap::border::PaletteField;
use baumhard::mindmap::border_pattern::SidePattern;
use baumhard::mindmap::model::{CustomBorderGlyphs, GlyphBorderConfig};

use super::option_edit::{apply_option_edit, apply_string_set, apply_value_set, OptionEdit};
use super::{grow_one_node_to_fit_border, MindMapDocument, UndoAction};

/// Bundle of optional edits applied atomically by
/// [`MindMapDocument::set_node_border_config`]. Every field
/// defaults to "no edit"; the console verb composes one struct
/// per invocation and hands it to the setter.
///
/// Side-pattern fields carry pre-parsed [`SidePattern`]s plus the
/// raw input strings the parser produced from — the strings live
/// in the data model (so save / round-trip preserves the
/// original text) and the parsed payload validates the input
/// before the document is mutated. Construct with
/// [`BorderConfigEdits::with_side_pattern`] so a console caller
/// can't ship a parse-error string.
#[derive(Clone, Debug, Default)]
pub struct BorderConfigEdits {
    pub preset: OptionEdit<String>,
    pub font: OptionEdit<String>,
    pub font_size_pt: OptionEdit<f32>,
    pub color: OptionEdit<String>,
    pub padding: OptionEdit<f32>,
    pub color_palette: OptionEdit<String>,
    pub color_palette_field: OptionEdit<PaletteField>,
    pub side_top: OptionEdit<String>,
    pub side_bottom: OptionEdit<String>,
    pub side_left: OptionEdit<String>,
    pub side_right: OptionEdit<String>,
    pub corner_top_left: OptionEdit<String>,
    pub corner_top_right: OptionEdit<String>,
    pub corner_bottom_left: OptionEdit<String>,
    pub corner_bottom_right: OptionEdit<String>,
    /// `Some(true)` switches `style.show_frame` on, `Some(false)`
    /// off, `None` leaves it untouched. Kept on this struct (vs.
    /// a separate setter) so a single command can both flip the
    /// frame *and* configure it in one undoable step.
    pub visible: Option<bool>,
    /// `true` clears the per-node `style.border` override entirely
    /// (handles the `border reset` verb). When set, every other
    /// field on this struct is ignored.
    pub clear: bool,
}

impl BorderConfigEdits {
    /// Validate a side pattern string and stage it as a `Set`
    /// edit. Returns the parse error verbatim — the console verb
    /// surfaces it with a side prefix.
    pub fn with_side_pattern(&mut self, side: BorderSide, pattern: &str) -> Result<(), String> {
        SidePattern::parse(pattern).map_err(|e| format!("{}: {}", side.label(), e))?;
        let slot = match side {
            BorderSide::Top => &mut self.side_top,
            BorderSide::Bottom => &mut self.side_bottom,
            BorderSide::Left => &mut self.side_left,
            BorderSide::Right => &mut self.side_right,
        };
        *slot = OptionEdit::Set(pattern.to_string());
        Ok(())
    }
}

/// Side selector used by [`BorderConfigEdits::with_side_pattern`]
/// and the `border show` console output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderSide {
    Top,
    Bottom,
    Left,
    Right,
}

impl BorderSide {
    pub fn label(self) -> &'static str {
        match self {
            BorderSide::Top => "top",
            BorderSide::Bottom => "bottom",
            BorderSide::Left => "left",
            BorderSide::Right => "right",
        }
    }
}

/// Result of [`MindMapDocument::set_node_border_config`] —
/// distinguishes "no change" from "applied" and surfaces the
/// preset auto-promotion side effect so the console verb can
/// tell the user when their `preset=heavy top=…` request landed
/// as `preset=custom` (because setting any side or corner glyph
/// requires the custom preset for the data model to honour the
/// override at render time).
#[derive(Clone, Debug, Default)]
pub struct BorderEditOutcome {
    /// `true` when any field on the node actually changed.
    /// Console callers surface "no change" when this is false.
    pub changed: bool,
    /// `true` when the preset was auto-flipped from a non-custom
    /// value to `"custom"` because the same call also set a side
    /// or corner glyph. The verb's success message includes a
    /// note in that case.
    pub preset_auto_promoted: bool,
    /// The preset the user explicitly asked for in this edit, or
    /// `None` if no `preset=` kv was provided. Used together with
    /// `preset_auto_promoted` to phrase the auto-promotion note
    /// (`"preset=heavy was auto-promoted to 'custom'…"`).
    pub requested_preset: Option<String>,
}

/// Active live-preview substitution captured on
/// [`MindMapDocument::border_preview`]. The scene builder reads
/// this through a borrowed view (`scene_builder::BorderPreview<'a>`)
/// and substitutes the previewed `edits` for the resolved border
/// at the matching target. The model is never mutated; commit
/// dispatches to the matching committing setter and clears the
/// preview slot, cancel just clears.
///
/// `selection_snapshot` is the live selection at preview-set time —
/// the scene builder treats the preview as inactive when the
/// current `MindMapDocument.selection` no longer covers the
/// snapshot's targets (drift). The actual clear happens at the
/// next `set_*` / `commit_*` / `cancel_*` call; the steady-state
/// "orphaned by drift" preview is harmless until then.
///
/// One preview at a time: a fresh `set_border_preview` replaces
/// any active preview atomically.
#[derive(Clone, Debug)]
pub struct BorderPreview {
    pub target: BorderPreviewTarget,
    pub edits: BorderConfigEdits,
    pub selection_snapshot: super::super::SelectionState,
}

/// Which border slot the preview substitutes for at scene-build
/// time. Mirrors the four committing setters' shapes —
/// `Nodes(ids)` → [`MindMapDocument::set_node_border_config`],
/// `Sections((id, idx))` →
/// [`MindMapDocument::set_section_frame_border_config`],
/// `CanvasDefault` → [`MindMapDocument::set_canvas_default_border_config`],
/// `CanvasSectionFrame` / `CanvasSectionFrameFocused` →
/// [`MindMapDocument::set_canvas_default_section_frame_border_config`]
/// (with `focused = false / true`).
///
/// Single preview, single target variant — multi-target previews
/// (e.g. nodes *and* canvas-default at the same time) are
/// deliberately out of scope: setting a new preview replaces the
/// prior one.
#[derive(Clone, Debug)]
pub enum BorderPreviewTarget {
    Nodes(Vec<String>),
    Sections(Vec<(String, usize)>),
    CanvasDefault,
    CanvasSectionFrame,
    CanvasSectionFrameFocused,
}

impl MindMapDocument {
    /// Toggle the node's frame visibility. Returns `true` if the
    /// flag actually changed. No-op + no undo on no change, like
    /// every other style setter.
    pub fn set_node_border_visible(&mut self, node_id: &str, on: bool) -> bool {
        super::set_node_style_field(self, node_id, |s| {
            if s.show_frame == on {
                return false;
            }
            s.show_frame = on;
            true
        })
    }

    /// Apply a bundle of border edits to one node atomically.
    ///
    /// `edits.clear == true` drops the per-node `style.border`
    /// override entirely (the node falls back to the canvas
    /// default), ignoring every other field.
    ///
    /// Otherwise: every field with `OptionEdit::Set(v)` is
    /// written; `Clear` removes an existing override; `Keep`
    /// leaves the field untouched. Side-pattern strings are
    /// trusted to have been validated upstream
    /// (via [`BorderConfigEdits::with_side_pattern`]).
    ///
    /// After mutation, runs [`grow_one_node_to_fit_border`] so
    /// the node grows to fit the new static parts; the same
    /// `EditNodeStyle` undo envelope captures both the style
    /// change and the size change.
    ///
    /// Returns a [`BorderEditOutcome`] describing whether
    /// anything changed and whether the preset was auto-promoted
    /// to `"custom"` (which happens whenever any side or corner
    /// glyph is set against a non-custom preset). The console
    /// verb surfaces the auto-promotion so the user knows their
    /// `preset=heavy top=…` request resulted in a `custom` border,
    /// not a `heavy` one with a side override.
    pub fn set_node_border_config(&mut self, node_id: &str, edits: BorderConfigEdits) -> BorderEditOutcome {
        // Any committing border edit clears an active preview —
        // otherwise a previewed config would render on top of the
        // just-committed value until the user manually cancelled.
        // See the plan's "Esc + console interaction" section.
        self.cancel_border_preview();
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = match self.mindmap.nodes.get_mut(node_id) {
            Some(n) => n,
            None => return BorderEditOutcome::default(),
        };
        let before_style = node.style.clone();
        let before_sections = node.sections.clone();
        let preset_before = before_style.border.as_ref().map(|c| c.preset.clone());

        let mut outcome = BorderEditOutcome::default();
        let any_change = if edits.clear {
            if node.style.border.is_none() && edits.visible.is_none() {
                false
            } else {
                node.style.border = None;
                if let Some(v) = edits.visible {
                    node.style.show_frame = v;
                }
                true
            }
        } else {
            apply_border_edits(node, &edits, &mut outcome)
        };

        if !any_change {
            return outcome;
        }

        // Detect a preset auto-promotion to "custom" so the
        // verb's success message can tell the user their
        // explicit `preset=…` choice was overridden by a side
        // / corner edit. The user-asked-for preset (if any) is
        // captured up-front in `outcome.requested_preset` by
        // `apply_border_edits`; here we compare against what
        // landed in the model.
        detect_preset_auto_promote(node.style.border.as_ref(), preset_before.as_deref(), &mut outcome);

        // The size grow is monotonic by design (mirrors
        // `grow_node_sizes_to_fit_text`), so undoing a border edit
        // restores the style override but leaves the node at its
        // grown size — same posture as text edits that grew the
        // box. The user can shrink manually if desired.
        grow_one_node_to_fit_border(node, canvas_default.as_ref());
        self.undo_stack.push(UndoAction::EditNodeStyle {
            node_id: node_id.to_string(),
            before_style,
            before_sections,
        });
        self.dirty = true;
        outcome.changed = true;
        outcome
    }

    /// Apply a bundle of border edits to one section's
    /// `frame_border` atomically — the per-section equivalent of
    /// [`Self::set_node_border_config`]. Drives the
    /// `section frame …` console verb.
    ///
    /// `edits.clear == true` drops the per-section
    /// `frame_border` override (the section falls back to
    /// `Canvas.default_section_frame_border` and then to a
    /// hardcoded floor — same cascade as
    /// [`baumhard::mindmap::border::resolve_section_frame_border`]).
    /// `edits.visible` is ignored: section frames don't carry a
    /// per-frame visibility flag (NodeEdit drives their lifecycle).
    ///
    /// Returns the same [`BorderEditOutcome`] shape — the verb
    /// surfaces auto-promotion identically whether the edit
    /// landed on a node or a section.
    pub fn set_section_frame_border_config(
        &mut self,
        node_id: &str,
        section_idx: usize,
        edits: BorderConfigEdits,
    ) -> BorderEditOutcome {
        // Implicit-cancel of any active preview — same rule the
        // node-level setter applies. See `set_node_border_config`.
        self.cancel_border_preview();
        // Validate node + section exist before we touch anything.
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return BorderEditOutcome::default(),
        };
        let Some(section) = node.sections.get(section_idx) else {
            return BorderEditOutcome::default();
        };
        let preset_before = section.frame_border.as_ref().map(|c| c.preset.clone());

        let mut outcome = BorderEditOutcome::default();
        if edits.clear {
            if section.frame_border.is_none() {
                return outcome;
            }
            self.mutate_section_with_style_undo(node_id, section_idx, |s| {
                s.frame_border = None;
                true
            });
            outcome.changed = true;
            return outcome;
        }

        // Apply the staged edits to the section's frame_border slot
        // through the helper. The closure returns its change verdict
        // so the helper itself decides whether to push the undo
        // entry + flip `dirty` — no post-hoc `undo_stack.pop()` and
        // no leaked `dirty=true` on no-ops.
        let changed = self.mutate_section_with_style_undo(node_id, section_idx, |s| {
            apply_glyph_border_edits_to_slot(&mut s.frame_border, &edits, &mut outcome)
        });
        if !changed {
            return outcome;
        }

        // Detect preset auto-promotion (light / heavy / etc. → custom)
        // identically to the node-level setter.
        let landed = self
            .mindmap
            .nodes
            .get(node_id)
            .and_then(|n| n.sections.get(section_idx))
            .and_then(|s| s.frame_border.as_ref());
        detect_preset_auto_promote(landed, preset_before.as_deref(), &mut outcome);

        outcome.changed = true;
        outcome
    }

    /// Apply a bundle of border edits to
    /// [`baumhard::mindmap::model::Canvas::default_border`] —
    /// the map-wide fallback every framed node falls back to when
    /// it has no per-node `style.border` override. Drives the
    /// `canvas border …` console verb.
    ///
    /// `edits.clear == true` drops the canvas default (every
    /// unframed node falls back to the hardcoded floor). `visible`
    /// is ignored: canvas-level defaults don't carry a visibility
    /// flag — the per-node `show_frame` toggle is the
    /// authoritative on/off.
    ///
    /// Captures the entire `Canvas` in a `CanvasSnapshot` undo
    /// entry so undo restores every theme / palette / default
    /// field in one step. Same posture as the
    /// `theme switch` verb.
    pub fn set_canvas_default_border_config(&mut self, edits: BorderConfigEdits) -> BorderEditOutcome {
        // Implicit-cancel of any active preview — see
        // `set_node_border_config` for the rationale.
        self.cancel_border_preview();
        let preset_before = self
            .mindmap
            .canvas
            .default_border
            .as_ref()
            .map(|c| c.preset.clone());
        let canvas_snapshot = self.mindmap.canvas.clone();
        let mut outcome = BorderEditOutcome::default();

        let any_change = if edits.clear {
            if self.mindmap.canvas.default_border.is_none() {
                false
            } else {
                self.mindmap.canvas.default_border = None;
                true
            }
        } else {
            apply_glyph_border_edits_to_slot(&mut self.mindmap.canvas.default_border, &edits, &mut outcome)
        };

        if !any_change {
            return outcome;
        }

        detect_preset_auto_promote(
            self.mindmap.canvas.default_border.as_ref(),
            preset_before.as_deref(),
            &mut outcome,
        );

        self.undo_stack.push(UndoAction::CanvasSnapshot {
            canvas: canvas_snapshot,
        });
        self.dirty = true;
        outcome.changed = true;
        outcome
    }

    /// Apply a bundle of border edits to either
    /// [`baumhard::mindmap::model::Canvas::default_section_frame_border`]
    /// (when `focused == false`) or
    /// [`baumhard::mindmap::model::Canvas::default_focused_section_frame_border`]
    /// (when `focused == true`). Drives the
    /// `canvas section-frame …` and `canvas section-frame focused …`
    /// console subverbs.
    ///
    /// Same `edits.clear` / `visible`-ignored / `CanvasSnapshot`
    /// undo / auto-promotion-detection contract as
    /// [`Self::set_canvas_default_border_config`].
    pub fn set_canvas_default_section_frame_border_config(
        &mut self,
        focused: bool,
        edits: BorderConfigEdits,
    ) -> BorderEditOutcome {
        // Implicit-cancel of any active preview — see
        // `set_node_border_config` for the rationale.
        self.cancel_border_preview();
        let canvas_snapshot = self.mindmap.canvas.clone();
        let mut outcome = BorderEditOutcome::default();

        let preset_before = if focused {
            self.mindmap
                .canvas
                .default_focused_section_frame_border
                .as_ref()
                .map(|c| c.preset.clone())
        } else {
            self.mindmap
                .canvas
                .default_section_frame_border
                .as_ref()
                .map(|c| c.preset.clone())
        };

        let any_change = if edits.clear {
            let slot = if focused {
                &mut self.mindmap.canvas.default_focused_section_frame_border
            } else {
                &mut self.mindmap.canvas.default_section_frame_border
            };
            if slot.is_none() {
                false
            } else {
                *slot = None;
                true
            }
        } else {
            let slot = if focused {
                &mut self.mindmap.canvas.default_focused_section_frame_border
            } else {
                &mut self.mindmap.canvas.default_section_frame_border
            };
            apply_glyph_border_edits_to_slot(slot, &edits, &mut outcome)
        };

        if !any_change {
            return outcome;
        }

        let landed = if focused {
            self.mindmap.canvas.default_focused_section_frame_border.as_ref()
        } else {
            self.mindmap.canvas.default_section_frame_border.as_ref()
        };
        detect_preset_auto_promote(landed, preset_before.as_deref(), &mut outcome);

        self.undo_stack.push(UndoAction::CanvasSnapshot {
            canvas: canvas_snapshot,
        });
        self.dirty = true;
        outcome.changed = true;
        outcome
    }

    /// Set or replace the active border preview. Returns the
    /// outcome a *commit* would produce (`requested_preset`,
    /// `preset_auto_promoted`) computed by simulating
    /// `apply_glyph_border_edits_to_slot` against a clone of the
    /// affected slot — never re-implements the apply path. The
    /// console verb surfaces the simulated outcome so the user
    /// sees auto-promotion notes up-front, identical to what
    /// commit will report.
    ///
    /// No undo, no dirty, no model write. A prior preview is
    /// replaced atomically; orphan-by-drift previews are cleared.
    pub fn set_border_preview(
        &mut self,
        target: BorderPreviewTarget,
        edits: BorderConfigEdits,
    ) -> BorderEditOutcome {
        // Simulate the apply against a clone of the affected slot
        // so the outcome reflects what commit will produce. Pick
        // the slot per target variant; for multi-target
        // (`Nodes(ids)` / `Sections(...)`), simulate against the
        // first target — auto-promotion is a property of `edits`,
        // not of the slot's pre-state, so any one slot is
        // representative. Empty target lists fall through to a
        // canvas-default-shaped slot just to drive the helper.
        let mut outcome = BorderEditOutcome::default();
        let mut slot_clone: Option<GlyphBorderConfig> = match &target {
            BorderPreviewTarget::Nodes(ids) => ids
                .first()
                .and_then(|id| self.mindmap.nodes.get(id))
                .and_then(|n| n.style.border.clone()),
            BorderPreviewTarget::Sections(pairs) => pairs
                .first()
                .and_then(|(id, idx)| self.mindmap.nodes.get(id).and_then(|n| n.sections.get(*idx)))
                .and_then(|s| s.frame_border.clone()),
            BorderPreviewTarget::CanvasDefault => self.mindmap.canvas.default_border.clone(),
            BorderPreviewTarget::CanvasSectionFrame => {
                self.mindmap.canvas.default_section_frame_border.clone()
            }
            BorderPreviewTarget::CanvasSectionFrameFocused => self
                .mindmap
                .canvas
                .default_focused_section_frame_border
                .clone(),
        };
        apply_glyph_border_edits_to_slot(&mut slot_clone, &edits, &mut outcome);
        // The post-state preset on the cloned slot drives auto-
        // promotion detection; the same helper the four committing
        // setters use.
        let preset_before = match &target {
            BorderPreviewTarget::Nodes(ids) => ids
                .first()
                .and_then(|id| self.mindmap.nodes.get(id))
                .and_then(|n| n.style.border.as_ref())
                .map(|c| c.preset.clone()),
            BorderPreviewTarget::Sections(pairs) => pairs
                .first()
                .and_then(|(id, idx)| self.mindmap.nodes.get(id).and_then(|n| n.sections.get(*idx)))
                .and_then(|s| s.frame_border.as_ref())
                .map(|c| c.preset.clone()),
            BorderPreviewTarget::CanvasDefault => self
                .mindmap
                .canvas
                .default_border
                .as_ref()
                .map(|c| c.preset.clone()),
            BorderPreviewTarget::CanvasSectionFrame => self
                .mindmap
                .canvas
                .default_section_frame_border
                .as_ref()
                .map(|c| c.preset.clone()),
            BorderPreviewTarget::CanvasSectionFrameFocused => self
                .mindmap
                .canvas
                .default_focused_section_frame_border
                .as_ref()
                .map(|c| c.preset.clone()),
        };
        detect_preset_auto_promote(slot_clone.as_ref(), preset_before.as_deref(), &mut outcome);
        // The outcome's `changed` field is meaningful for commit
        // (where it gates the undo push); for preview-set we
        // surface it so the verb can say "no visible change" if
        // the staged edits are a no-op against the current slot.
        // The simulation already populated it via the helper.

        self.border_preview = Some(BorderPreview {
            target,
            edits,
            selection_snapshot: self.selection.clone(),
        });
        outcome
    }

    /// Discard any active preview. Returns `true` if a preview
    /// was active. O(1), no undo, no dirty — preview state is
    /// runtime-only.
    pub fn cancel_border_preview(&mut self) -> bool {
        self.border_preview.take().is_some()
    }

    /// Commit the active preview through the matching committing
    /// setter (`set_node_border_config` etc.) and clear the
    /// preview slot. Returns `Some(outcome)` when a preview was
    /// active (the outcome merges per-target results: first
    /// auto-promotion wins; `changed` is `true` if any underlying
    /// setter reported a change), `None` otherwise.
    ///
    /// Multi-node / multi-section commits push one undo entry per
    /// affected node — same posture as the verb-level
    /// `apply_edits` in `border/execute.rs`. Undoing a 5-node
    /// commit takes 5 Ctrl-Z's; intentional, not a regression.
    pub fn commit_border_preview(&mut self) -> Option<BorderEditOutcome> {
        let preview = self.border_preview.take()?;
        let mut merged = BorderEditOutcome::default();
        match preview.target {
            BorderPreviewTarget::Nodes(ids) => {
                for id in &ids {
                    let outcome = self.set_node_border_config(id, preview.edits.clone());
                    merge_outcome(&mut merged, outcome);
                }
            }
            BorderPreviewTarget::Sections(pairs) => {
                for (id, idx) in &pairs {
                    let outcome = self.set_section_frame_border_config(id, *idx, preview.edits.clone());
                    merge_outcome(&mut merged, outcome);
                }
            }
            BorderPreviewTarget::CanvasDefault => {
                merged = self.set_canvas_default_border_config(preview.edits);
            }
            BorderPreviewTarget::CanvasSectionFrame => {
                merged = self.set_canvas_default_section_frame_border_config(false, preview.edits);
            }
            BorderPreviewTarget::CanvasSectionFrameFocused => {
                merged = self.set_canvas_default_section_frame_border_config(true, preview.edits);
            }
        }
        Some(merged)
    }
}

/// Fold one per-target outcome into the running merged outcome.
/// `changed` aggregates with OR; the first auto-promotion's
/// `requested_preset` wins (same posture as
/// `border/execute.rs::apply_edits`'s tally).
fn merge_outcome(merged: &mut BorderEditOutcome, one: BorderEditOutcome) {
    if one.changed {
        merged.changed = true;
    }
    if one.preset_auto_promoted && !merged.preset_auto_promoted {
        merged.preset_auto_promoted = true;
        merged.requested_preset = one.requested_preset;
    }
}

/// Set `outcome.preset_auto_promoted = true` when `landed`'s preset
/// is `"custom"` (case-insensitive), the pre-edit preset wasn't
/// already custom, and the user explicitly asked for some preset
/// (i.e. their kv mentioned `preset=`). Shared between the four
/// border-style setters (`set_node_border_config`,
/// `set_section_frame_border_config`,
/// `set_canvas_default_border_config`,
/// `set_canvas_default_section_frame_border_config`) so the
/// detection logic lives in exactly one place. When the auto-
/// promote path is removed (per `SECTIONS_BORDERS_RESIZE_PLAN.md`
/// §5.4), this function and the `BorderEditOutcome.preset_auto_promoted`
/// field go together.
pub(super) fn detect_preset_auto_promote(
    landed: Option<&GlyphBorderConfig>,
    preset_before: Option<&str>,
    outcome: &mut BorderEditOutcome,
) {
    let Some(cfg) = landed else { return };
    if !cfg.preset.eq_ignore_ascii_case("custom") {
        return;
    }
    let was_already_custom = preset_before
        .map(|p| p.eq_ignore_ascii_case("custom"))
        .unwrap_or(false);
    if !was_already_custom && outcome.requested_preset.is_some() {
        outcome.preset_auto_promoted = true;
    }
}

/// Apply non-clear edits to a node's style/border. Returns
/// `true` when at least one slot actually changed value (so the
/// caller can decide whether to push an undo entry).
///
/// Bookkeeping is one boolean we OR with each per-field check —
/// avoids an `EditEqOp` clone-and-compare on the whole NodeStyle
/// (which doesn't implement `PartialEq`) and also lets a no-op
/// kv pair like `border on` against an already-on border short-
/// circuit cleanly.
pub(super) fn apply_border_edits(
    node: &mut baumhard::mindmap::model::MindNode,
    edits: &BorderConfigEdits,
    outcome: &mut BorderEditOutcome,
) -> bool {
    let mut changed = false;
    if let Some(v) = edits.visible {
        if node.style.show_frame != v {
            node.style.show_frame = v;
            changed = true;
        }
    }
    changed |= apply_glyph_border_edits_to_slot(&mut node.style.border, edits, outcome);
    changed
}

/// Slot-level helper that applies every config-side field on
/// `BorderConfigEdits` (preset / font / size / color / padding /
/// palette / field / sides / corners) directly to a
/// `&mut Option<GlyphBorderConfig>`. Skips the `visible` flag —
/// that's a node-only concept that the per-node wrapper layers on
/// top.
///
/// Shared between `apply_border_edits` (writes `node.style.border`)
/// and [`MindMapDocument::set_section_frame_border_config`] (writes
/// `section.frame_border`). The factoring is what lets the
/// `border …` and `section frame …` verbs feed the same kv
/// vocabulary into two different model slots.
///
/// `outcome.requested_preset` is set when the caller passed
/// `preset=…` so the upper layer can phrase the "auto-promoted to
/// custom" message after detecting the preset shift.
pub(crate) fn apply_glyph_border_edits_to_slot(
    slot: &mut Option<GlyphBorderConfig>,
    edits: &BorderConfigEdits,
    outcome: &mut BorderEditOutcome,
) -> bool {
    if let OptionEdit::Set(p) = &edits.preset {
        outcome.requested_preset = Some(p.clone());
    }

    // Skip the slot allocation entirely when no config-side field
    // was touched. The caller may still have written `visible`
    // before us; that change is its bookkeeping, not ours.
    if !edits_touch_cfg_field(edits) {
        return false;
    }

    let mut changed = false;
    let had_cfg = slot.is_some();
    let cfg = slot.get_or_insert_with(default_glyph_border_config);
    if !had_cfg {
        changed = true;
    }

    if let OptionEdit::Set(p) = &edits.preset {
        if cfg.preset != *p {
            cfg.preset = p.clone();
            changed = true;
        }
    }
    changed |= apply_option_edit(&edits.font, &mut cfg.font, |v| v.clone());
    changed |= apply_value_set(&edits.font_size_pt, &mut cfg.font_size_pt);
    changed |= apply_option_edit(&edits.color, &mut cfg.color, |v| v.clone());
    changed |= apply_value_set(&edits.padding, &mut cfg.padding);
    changed |= apply_option_edit(&edits.color_palette, &mut cfg.color_palette, |v| v.clone());
    changed |= apply_option_edit(&edits.color_palette_field, &mut cfg.color_palette_field, |v| {
        v.as_str().to_string()
    });

    // Sides + corners: ensure the `glyphs` slot exists if any of
    // the eight glyph-string fields is being edited. The schema
    // says they're consulted only when `preset == "custom"`, so
    // setting a side without flipping the preset is silently a
    // no-op visually — the console verb upgrades the preset for
    // the user when any side / corner is set.
    if edits_touch_glyphs(edits) {
        if cfg.glyphs.is_none() {
            cfg.glyphs = Some(default_custom_glyphs());
            changed = true;
        }
        if !preset_is_custom(&cfg.preset) {
            cfg.preset = "custom".to_string();
            changed = true;
        }
        let g = cfg.glyphs.as_mut().expect("just inserted");
        changed |= apply_string_set(&edits.side_top, &mut g.top);
        changed |= apply_string_set(&edits.side_bottom, &mut g.bottom);
        changed |= apply_string_set(&edits.side_left, &mut g.left);
        changed |= apply_string_set(&edits.side_right, &mut g.right);
        changed |= apply_string_set(&edits.corner_top_left, &mut g.top_left);
        changed |= apply_string_set(&edits.corner_top_right, &mut g.top_right);
        changed |= apply_string_set(&edits.corner_bottom_left, &mut g.bottom_left);
        changed |= apply_string_set(&edits.corner_bottom_right, &mut g.bottom_right);
    }
    changed
}

fn edits_touch_cfg_field(edits: &BorderConfigEdits) -> bool {
    !matches!(edits.preset, OptionEdit::Keep)
        || !matches!(edits.font, OptionEdit::Keep)
        || !matches!(edits.font_size_pt, OptionEdit::Keep)
        || !matches!(edits.color, OptionEdit::Keep)
        || !matches!(edits.padding, OptionEdit::Keep)
        || !matches!(edits.color_palette, OptionEdit::Keep)
        || !matches!(edits.color_palette_field, OptionEdit::Keep)
        || edits_touch_glyphs(edits)
}

fn edits_touch_glyphs(edits: &BorderConfigEdits) -> bool {
    matches!(edits.side_top, OptionEdit::Set(_))
        || matches!(edits.side_bottom, OptionEdit::Set(_))
        || matches!(edits.side_left, OptionEdit::Set(_))
        || matches!(edits.side_right, OptionEdit::Set(_))
        || matches!(edits.corner_top_left, OptionEdit::Set(_))
        || matches!(edits.corner_top_right, OptionEdit::Set(_))
        || matches!(edits.corner_bottom_left, OptionEdit::Set(_))
        || matches!(edits.corner_bottom_right, OptionEdit::Set(_))
}

fn preset_is_custom(s: &str) -> bool {
    s.eq_ignore_ascii_case("custom")
}

fn default_glyph_border_config() -> GlyphBorderConfig {
    // Mirrors the loader-time defaults from
    // `baumhard::mindmap::model::node`. Centralised here so the
    // setter doesn't need access to the private `default_*`
    // factory functions in the model module.
    GlyphBorderConfig {
        preset: "light".to_string(),
        font: None,
        font_size_pt: 14.0,
        color: None,
        glyphs: None,
        padding: 4.0,
        color_palette: None,
        color_palette_field: None,
    }
}

fn default_custom_glyphs() -> CustomBorderGlyphs {
    // Light-preset corners (`┌┐└┘`) match the new default border
    // preset, so a `custom` payload that omits a corner falls back
    // to the same join-cleanly shape the surrounding sides expect.
    CustomBorderGlyphs {
        top: "\u{2500}".to_string(),
        bottom: "\u{2500}".to_string(),
        left: "\u{2502}".to_string(),
        right: "\u{2502}".to_string(),
        top_left: "\u{250C}".to_string(),
        top_right: "\u{2510}".to_string(),
        bottom_left: "\u{2514}".to_string(),
        bottom_right: "\u{2518}".to_string(),
    }
}
