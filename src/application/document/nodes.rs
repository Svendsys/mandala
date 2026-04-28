// SPDX-License-Identifier: MPL-2.0

//! Node-style mutations — `set_node_text` /
//! `set_node_bg_color` / `set_node_border_color` /
//! `set_node_text_color` / `set_node_font_size`, plus the
//! `set_node_style_field` helper that shared bodies route
//! through so the undo push / no-op detection stays uniform.

use baumhard::mindmap::border::PaletteField;
use baumhard::mindmap::border_pattern::SidePattern;
use baumhard::mindmap::model::{
    CustomBorderGlyphs, GlyphBorderConfig, NodeStyle, TextRun,
};

use super::grow_one_node_to_fit_border;
use super::undo_action::UndoAction;
use super::MindMapDocument;

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
    pub fn set_node_text(&mut self, node_id: &str, new_text: String) -> bool {
        // Validate + capture under an immutable borrow so the mutable
        // re-acquisition below can coexist with the canvas-default
        // clone (which would otherwise overlap the borrow held by
        // an upfront `get_mut`).
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        if node.text == new_text {
            return false;
        }
        let before_text = node.text.clone();
        let before_runs = node.text_runs.clone();
        // Collapse to a single run that spans the new text. Inherit
        // formatting from the first original run, or fall back to the
        // default-orphan defaults if the node had no runs.
        let template = before_runs.first().cloned().unwrap_or_else(|| TextRun {
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
        node.text = new_text;
        node.text_runs = new_runs;
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
            before_text,
            before_runs,
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
            .text_runs
            .iter()
            .any(|r| r.color == old_default && r.color != color);
        if old_default == color && !any_run_changes {
            return false;
        }
        let before_style = node.style.clone();
        let before_runs = node.text_runs.clone();
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        node.style.text_color = color.clone();
        for run in node.text_runs.iter_mut() {
            if run.color == old_default {
                run.color = color.clone();
            }
        }
        self.undo_stack.push(UndoAction::EditNodeStyle {
            node_id: node_id.to_string(),
            before_style,
            before_runs,
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
        let already = node.text_runs.iter().all(|r| r.size_pt == size_u);
        if already {
            return false;
        }
        let before_style = node.style.clone();
        let before_runs = node.text_runs.clone();
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        for run in node.text_runs.iter_mut() {
            run.size_pt = size_u;
        }
        // Larger text needs a larger box. Same monotonic floor as
        // `set_node_font_family`: grow on demand, never shrink.
        super::grow_one_node_to_fit_text(node);
        super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        self.undo_stack.push(UndoAction::EditNodeStyle {
            node_id: node_id.to_string(),
            before_style,
            before_runs,
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
            .text_runs
            .iter()
            .all(|r| r.font.as_str() == target);
        if already {
            return false;
        }
        let before_style = node.style.clone();
        let before_runs = node.text_runs.clone();
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = self.mindmap.nodes.get_mut(node_id).expect("just checked");
        for run in node.text_runs.iter_mut() {
            run.font = target.to_string();
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
            before_runs,
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

/// Triple-state edit on an `Option<T>` field. The three variants
/// distinguish "leave alone" from "explicitly clear an existing
/// override" from "set to a concrete value". Used by the
/// `BorderConfigEdits` bundle (every per-field slot is one of these)
/// and by the zoom-visibility setters (where a console line like
/// `zoom min=1.5 max=unset` translates each kv into an
/// `OptionEdit<f32>` so a single setter call handles both sides
/// atomically). The shared shape is what makes the console verbs'
/// `palette=off` / `font=off` / `min=unset` syntax possible —
/// without `Clear`, callers couldn't distinguish "the user didn't
/// mention this field" from "the user wants this field cleared".
///
/// `Keep` is the default so [`BorderConfigEdits`]'s
/// `#[derive(Default)]` builds the no-op edit set, and the console
/// verb only fills in the keys the user actually typed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum OptionEdit<T> {
    /// No edit — leave the model field at its current value.
    #[default]
    Keep,
    /// Drop the per-node override; the resolver cascade falls
    /// through to the canvas-level default or hardcoded floor.
    Clear,
    /// Write this concrete value to the field.
    Set(T),
}

impl<T: Clone> OptionEdit<T> {
    /// Fold this edit against `current`, yielding the new
    /// `Option<T>` value. Pure, O(1). The single canonical
    /// implementation of the Keep/Clear/Set semantics — every
    /// consumer (`zoom_bounds` setters today, future
    /// border-config writes when the bespoke
    /// `apply_option_edit` / `apply_value_set` helpers fold in)
    /// goes through this method instead of re-matching the
    /// three variants.
    pub fn apply(self, current: Option<T>) -> Option<T> {
        match self {
            OptionEdit::Keep => current,
            OptionEdit::Clear => None,
            OptionEdit::Set(v) => Some(v),
        }
    }
}

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
    pub fn with_side_pattern(
        &mut self,
        side: BorderSide,
        pattern: &str,
    ) -> Result<(), String> {
        SidePattern::parse(pattern)
            .map_err(|e| format!("{}: {}", side.label(), e))?;
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

impl MindMapDocument {
    /// Toggle the node's frame visibility. Returns `true` if the
    /// flag actually changed. No-op + no undo on no change, like
    /// every other style setter.
    pub fn set_node_border_visible(&mut self, node_id: &str, on: bool) -> bool {
        set_node_style_field(self, node_id, |s| {
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
    pub fn set_node_border_config(
        &mut self,
        node_id: &str,
        edits: BorderConfigEdits,
    ) -> BorderEditOutcome {
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = match self.mindmap.nodes.get_mut(node_id) {
            Some(n) => n,
            None => return BorderEditOutcome::default(),
        };
        let before_style = node.style.clone();
        let before_runs = node.text_runs.clone();
        let preset_before = before_style
            .border
            .as_ref()
            .map(|c| c.preset.clone());

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
            before_runs,
        });
        self.dirty = true;
        outcome.changed = true;
        outcome
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

/// Apply non-clear edits to a node's style/border. Returns
/// `true` when at least one slot actually changed value (so the
/// caller can decide whether to push an undo entry).
///
/// Bookkeeping is one boolean we OR with each per-field check —
/// avoids an `EditEqOp` clone-and-compare on the whole NodeStyle
/// (which doesn't implement `PartialEq`) and also lets a no-op
/// kv pair like `border on` against an already-on border short-
/// circuit cleanly.
fn apply_border_edits(
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
    let cfg = node
        .style
        .border
        .get_or_insert_with(default_glyph_border_config);
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
    changed |= apply_option_edit(
        &edits.color_palette_field,
        &mut cfg.color_palette_field,
        |v| v.as_str().to_string(),
    );

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

fn apply_string_set(edit: &OptionEdit<String>, slot: &mut String) -> bool {
    match edit {
        OptionEdit::Set(v) => {
            if slot != v {
                *slot = v.clone();
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Apply a `OptionEdit<T>` to an `Option<U>` slot, with `to_target`
/// projecting `T → U` for the value-write path. Returns `true` when the
/// slot actually changed. The four `font / color / color_palette /
/// color_palette_field` arms in `apply_border_edits` were structurally
/// identical (Set→write-if-different, Clear→None-if-some, Keep→no-op);
/// they collapse to one call each through this helper. The `to_target`
/// closure exists because `color_palette_field` writes a `String`-typed
/// slot from a `PaletteField` enum, so the projection isn't always
/// `clone()`.
fn apply_option_edit<T, U>(
    edit: &OptionEdit<T>,
    slot: &mut Option<U>,
    to_target: impl FnOnce(&T) -> U,
) -> bool
where
    U: PartialEq,
{
    match edit {
        OptionEdit::Set(v) => {
            let new = to_target(v);
            if slot.as_ref() != Some(&new) {
                *slot = Some(new);
                return true;
            }
        }
        OptionEdit::Clear => {
            if slot.is_some() {
                *slot = None;
                return true;
            }
        }
        OptionEdit::Keep => {}
    }
    false
}

/// Apply a `OptionEdit<T>` to a non-optional `T` slot — the
/// `Set`-only path used for `font_size_pt` and `padding` (their
/// underlying type stores a hardcoded default rather than `Option`,
/// so `Clear` is a no-op for them).
fn apply_value_set<T>(edit: &OptionEdit<T>, slot: &mut T) -> bool
where
    T: PartialEq + Clone,
{
    if let OptionEdit::Set(v) = edit {
        if slot != v {
            *slot = v.clone();
            return true;
        }
    }
    false
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

/// Guard used by every `set_*_zoom_visibility` setter. Rejects a
/// pair whose bounds are non-finite or whose resolved
/// `(min, max)` inverts. Mirrors the contract the verifier
/// enforces at load time and `ZoomVisibility::try_new` enforces
/// for programmatic callers — no panic in interactive paths per
/// `CODE_CONVENTIONS.md` §9.
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
fn set_node_style_field(
    doc: &mut MindMapDocument,
    node_id: &str,
    mutate: impl FnOnce(&mut NodeStyle) -> bool,
) -> bool {
    let node = match doc.mindmap.nodes.get_mut(node_id) {
        Some(n) => n,
        None => return false,
    };
    let before_style = node.style.clone();
    let before_runs = node.text_runs.clone();
    if !mutate(&mut node.style) {
        return false;
    }
    doc.undo_stack.push(UndoAction::EditNodeStyle {
        node_id: node_id.to_string(),
        before_style,
        before_runs,
    });
    doc.dirty = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::{
        first_testament_node_id as first_node_id,
        load_test_doc as fixture_doc,
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
        edits.with_side_pattern(BorderSide::Top, "###(*)###")
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
        let before_border = doc
            .mindmap
            .nodes
            .get(&id)
            .unwrap()
            .style
            .border
            .clone();
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
