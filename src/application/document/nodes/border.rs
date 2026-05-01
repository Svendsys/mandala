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
        if let Some(cfg) = node.style.border.as_ref() {
            if cfg.preset.eq_ignore_ascii_case("custom") {
                let was_already_custom = preset_before
                    .as_deref()
                    .map(|p| p.eq_ignore_ascii_case("custom"))
                    .unwrap_or(false);
                if !was_already_custom && outcome.requested_preset.is_some() {
                    outcome.preset_auto_promoted = true;
                }
            }
        }

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
    if let OptionEdit::Set(p) = &edits.preset {
        outcome.requested_preset = Some(p.clone());
    }
    if let Some(v) = edits.visible {
        if node.style.show_frame != v {
            node.style.show_frame = v;
            changed = true;
        }
    }

    // Bring the per-node config into existence on first edit so
    // every field has a slot to land in. Skip the slot allocation
    // entirely when the only edit is `visible`, which writes
    // `style.show_frame` and doesn't touch the GlyphBorderConfig.
    let needs_cfg = edits_touch_cfg_field(edits);
    if !needs_cfg {
        return changed;
    }

    let had_cfg = node.style.border.is_some();
    let cfg = node.style.border.get_or_insert_with(default_glyph_border_config);
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
