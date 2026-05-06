// SPDX-License-Identifier: MPL-2.0

//! Per-node and per-section-geometry setters and node-style
//! helpers. Section text / colour / font / runs / payload
//! setters live in `section_text.rs`. Each setter captures prior
//! state into an `UndoAction`, mutates, sets `dirty`, and
//! returns whether anything changed.

use baumhard::mindmap::model::{NodeStyle, TextRun};

use super::compute_one_node_text_floor;
use super::grow_one_node_to_fit_border;
use super::undo_action::UndoAction;
use super::MindMapDocument;

mod border;
mod option_edit;
mod section_text;

pub use border::{BorderConfigEdits, BorderEditOutcome, BorderSide};
pub use option_edit::OptionEdit;
pub(in crate::application::document) use section_text::clamp_runs_to_text;

/// Snapshot of a `MindSection`'s user-facing fields, used by the
/// structured-clipboard path (`ClipboardContent::Section` carries
/// it, the in-process buffer in `application/clipboard.rs` stashes
/// it, `apply_section_payload` writes it back). Decoupled from the
/// trait layer so callers can build payloads without depending on
/// `console::traits::outcome`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SectionPayload {
    pub text_runs: Vec<TextRun>,
    pub offset: baumhard::mindmap::model::Position,
    pub size: Option<baumhard::mindmap::model::Size>,
    pub channel: Option<usize>,
    pub trigger_bindings: Vec<baumhard::mindmap::custom_mutation::TriggerBinding>,
}

impl SectionPayload {
    /// Snapshot a `MindSection` into a payload (deep-clone each
    /// field). Cheap — every contained type is `Clone`.
    pub fn from_section(section: &baumhard::mindmap::model::MindSection) -> Self {
        Self {
            text_runs: section.text_runs.clone(),
            offset: section.offset,
            size: section.size,
            channel: section.channel,
            trigger_bindings: section.trigger_bindings.clone(),
        }
    }
}

impl MindMapDocument {
    /// Set one section's `offset` (relative to its owning node's
    /// `position`) under a single `EditNodeStyle` undo entry.
    /// Drag callers must NOT invoke this per-frame; gather delta
    /// in a gesture-state shape and call once on release.
    pub fn set_section_offset(
        &mut self,
        node_id: &str,
        section_idx: usize,
        x: f64,
        y: f64,
    ) -> Result<bool, String> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Ok(false),
        };
        let Some(section) = node.sections.get(section_idx) else {
            return Ok(false);
        };
        let new_offset = baumhard::mindmap::model::Position { x, y };
        validate_section_aabb(node.size, section_idx, new_offset, section.size)?;
        if section.offset == new_offset {
            return Ok(false);
        }
        let canvas_default = self.mindmap.canvas.default_border.clone();
        self.mutate_section_with_style_undo(node_id, section_idx, |s| {
            s.offset.x = x;
            s.offset.y = y;
        });
        // Re-acquire node and run the floor passes — moving a
        // `None`-sized section can shift its measured-text floor
        // contribution beyond the current node.size, leaving the
        // node under its floor for the next unrelated edit. Every
        // other section setter (text, font, payload) calls these
        // for the same invariant.
        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("just confirmed exists");
        super::grow_one_node_to_fit_text(node);
        super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        Ok(true)
    }

    /// Set one section's `size`. `None` means fill-parent;
    /// `Some(Size)` pins an explicit AABB. Same verify-mirroring
    /// validation discipline as [`Self::set_section_offset`]; same
    /// no-per-frame contract for drag callers.
    pub fn set_section_size(
        &mut self,
        node_id: &str,
        section_idx: usize,
        size: Option<baumhard::mindmap::model::Size>,
    ) -> Result<bool, String> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Ok(false),
        };
        let Some(section) = node.sections.get(section_idx) else {
            return Ok(false);
        };
        validate_section_aabb(node.size, section_idx, section.offset, size)?;
        if section.size == size {
            return Ok(false);
        }
        let canvas_default = self.mindmap.canvas.default_border.clone();
        self.mutate_section_with_style_undo(node_id, section_idx, |s| {
            s.size = size;
        });
        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("just confirmed exists");
        super::grow_one_node_to_fit_text(node);
        super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        Ok(true)
    }

    /// Atomically set one section's `(offset, size)` under a
    /// single `EditNodeStyle` undo entry. Validates the
    /// **post-mutation** AABB so a gesture that shifts offset and
    /// grows size in the same frame doesn't fail on the
    /// intermediate state.
    pub fn set_section_aabb(
        &mut self,
        node_id: &str,
        section_idx: usize,
        new_offset: baumhard::mindmap::model::Position,
        new_size: baumhard::mindmap::model::Size,
    ) -> Result<bool, String> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Ok(false),
        };
        let Some(section) = node.sections.get(section_idx) else {
            return Ok(false);
        };
        validate_section_aabb(node.size, section_idx, new_offset, Some(new_size))?;
        if section.offset == new_offset && section.size == Some(new_size) {
            return Ok(false);
        }
        let canvas_default = self.mindmap.canvas.default_border.clone();
        self.mutate_section_with_style_undo(node_id, section_idx, |s| {
            s.offset = new_offset;
            s.size = Some(new_size);
        });
        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("just confirmed exists");
        super::grow_one_node_to_fit_text(node);
        super::grow_one_node_to_fit_border(node, canvas_default.as_ref());
        Ok(true)
    }

    /// Set a node's `size` under a single `EditNodeAabb` undo
    /// entry. Validates finite + strictly positive components and
    /// rejects astronomical typos against `MAX_NODE_AXIS`. Position
    /// stays unchanged. Used by the `node resize <w> <h>` console
    /// verb.
    ///
    /// Idempotent: the no-op gate runs against the *post-grow*
    /// `n.size` (so a framed node whose border-grow inflates
    /// past `new_size` still no-ops on repeated calls; pre-fix
    /// the gate compared the pre-mutation size against
    /// `new_size`, missing on every framed-node call after the
    /// first and stacking undo entries).
    ///
    /// Drag callers must NOT invoke this per-frame; gather delta
    /// in a gesture-state shape and call once on release via
    /// [`Self::set_node_aabb`] which atomically writes both
    /// position and size.
    pub fn set_node_size(
        &mut self,
        node_id: &str,
        new_size: baumhard::mindmap::model::Size,
    ) -> Result<bool, String> {
        validate_node_size(new_size)?;
        check_node_size_typo(new_size)?;
        if !self.mindmap.nodes.contains_key(node_id) {
            return Ok(false);
        }
        let before_position = self.mindmap.nodes[node_id].position;
        let before_size = self.mindmap.nodes[node_id].size;
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let n = self.mindmap.nodes.get_mut(node_id).expect("just confirmed exists");
        n.size = new_size;
        // Floor-respect pass.
        super::grow_one_node_to_fit_text(n);
        super::grow_one_node_to_fit_border(n, canvas_default.as_ref());
        // Idempotent gate AFTER the grow passes — a framed
        // node's post-grow size can exceed the bare `new_size`,
        // so comparing pre-mutation against `new_size` would
        // miss on every call after the first. Comparing the
        // post-mutation `n.size` against `before_size` catches
        // the no-op case for both bare and framed nodes.
        if n.size == before_size {
            return Ok(false);
        }
        self.undo_stack.push(UndoAction::EditNodeAabb {
            node_id: node_id.to_string(),
            before_position,
            before_size,
        });
        self.dirty = true;
        Ok(true)
    }

    /// Set a node's `(position, size)` atomically under a single
    /// `EditNodeAabb` undo entry. Used by the node-resize gesture's
    /// release-commit — corner / edge handles whose `axis_factors`
    /// shrink size by the same delta they shift offset by need
    /// the AABB written in lockstep so the undo stack carries one
    /// pre-edit pair, not two interleaved entries.
    ///
    /// Same post-grow no-op-gate discipline as
    /// [`Self::set_node_size`] — see there for the framed-node
    /// idempotency rationale.
    pub fn set_node_aabb(
        &mut self,
        node_id: &str,
        new_position: baumhard::mindmap::model::Position,
        new_size: baumhard::mindmap::model::Size,
    ) -> Result<bool, String> {
        validate_node_position(new_position)?;
        validate_node_size(new_size)?;
        check_node_size_typo(new_size)?;
        if !self.mindmap.nodes.contains_key(node_id) {
            return Ok(false);
        }
        let before_position = self.mindmap.nodes[node_id].position;
        let before_size = self.mindmap.nodes[node_id].size;
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let n = self.mindmap.nodes.get_mut(node_id).expect("just confirmed exists");
        n.position = new_position;
        n.size = new_size;
        // Same floor-respect pass as `set_node_size`.
        super::grow_one_node_to_fit_text(n);
        super::grow_one_node_to_fit_border(n, canvas_default.as_ref());
        // Post-grow no-op gate — see `set_node_size` for the
        // framed-node idempotency rationale. Position is
        // unaffected by the grow passes, so the comparison
        // against `before_position` is exact.
        let same_position = n.position.x == before_position.x && n.position.y == before_position.y;
        if same_position && n.size == before_size {
            return Ok(false);
        }
        self.undo_stack.push(UndoAction::EditNodeAabb {
            node_id: node_id.to_string(),
            before_position,
            before_size,
        });
        self.dirty = true;
        Ok(true)
    }

    /// Shrink (or grow) a node's `size` to its measured-text
    /// floor — the explicit-shrink path the ambient `grow_*`
    /// passes can't take. Border-bearing nodes are rounded up
    /// from the text floor by `grow_one_node_to_fit_border` so
    /// the rendered frame has room. Pushes one `EditNodeAabb`
    /// undo entry; idempotent (no entry pushed when already at
    /// the post-border-grow target). See `set_node_size` for
    /// the floor-rejection counterpart.
    ///
    /// Re-measures every section under per-section
    /// `FONT_SYSTEM` write-guard acquires — same cost shape as
    /// `grow_one_node_to_fit_text`. Drag callers must NOT
    /// invoke this per-frame.
    pub fn fit_node_to_content(&mut self, node_id: &str) -> Result<bool, String> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Ok(false),
        };
        let (floor_w, floor_h) = compute_one_node_text_floor(node);
        // `f64::NAN <= 0.0` is false, so the finite-check is
        // load-bearing — NaN from a bad-font-measure path would
        // otherwise slip through the simple `<= 0.0` gate.
        if !floor_w.is_finite() || !floor_h.is_finite() || floor_w <= 0.0 || floor_h <= 0.0 {
            return Err(format!(
                "node '{}' has no measurable text; fit-to-content has no target floor",
                node_id
            ));
        }
        let candidate = baumhard::mindmap::model::Size {
            width: floor_w,
            height: floor_h,
        };
        // Route the candidate through the same validation +
        // typo guard the sibling node-size setters use, so a
        // pinned `section.size` of e.g. 5_000_000 (which
        // propagates through the floor) can't bypass the
        // absolute ceiling. Cheap arithmetic; defends future
        // regressions in `compute_one_node_text_floor`.
        validate_node_size(candidate)?;
        check_node_size_typo(candidate)?;
        let before_position = node.position;
        let before_size = node.size;
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let n = self.mindmap.nodes.get_mut(node_id).expect("just confirmed exists");
        n.size = candidate;
        // Border-grow runs after the text-floor write — the
        // rendered border needs room. The text-grow pass is
        // *deliberately* not invoked here (this is the shrink
        // path; running grow would max-wins back up to the same
        // floor we just wrote).
        super::grow_one_node_to_fit_border(n, canvas_default.as_ref());
        // Idempotent gate AFTER the border-grow: a framed
        // node's post-grow size can exceed the bare text floor,
        // so checking the gate before the grow would let
        // repeated calls stack undo entries on framed nodes.
        // Comparing the post-mutation `n.size` against
        // `before_size` is the post-border-grow signature.
        if n.size == before_size {
            return Ok(false);
        }
        self.undo_stack.push(UndoAction::EditNodeAabb {
            node_id: node_id.to_string(),
            before_position,
            before_size,
        });
        self.dirty = true;
        Ok(true)
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
        if !size_pt.is_finite() {
            return false;
        }
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
/// Verify-parity guard for the section-position/size setters: a
/// corrupt save with `node.size.{width,height}` non-finite or
/// non-positive is caught upstream by `verify::sections::check`,
/// but if the loader hands such a node to a setter and the AABB
/// compares silently NaN-skip, the setter would write into a node
/// that shouldn't accept any size at all. Return the same
/// rejection messages verify produces.
/// Finite + strictly-positive guard on a candidate node `Size`.
/// Same rejection messages `verify::sections` emits.
fn validate_node_size(size: baumhard::mindmap::model::Size) -> Result<(), String> {
    if !size.width.is_finite() || !size.height.is_finite() {
        return Err(format!(
            "node.size has non-finite component (width={}, height={})",
            size.width, size.height
        ));
    }
    if size.width <= 0.0 {
        return Err(format!("node.size.width is not positive ({})", size.width));
    }
    if size.height <= 0.0 {
        return Err(format!("node.size.height is not positive ({})", size.height));
    }
    Ok(())
}

/// Validate a candidate post-mutation section AABB against its
/// parent's size. Folds finite/positive guards on both node and
/// section, the 100× typo guard, and right/bottom edge
/// containment. `size = None` means fill-parent (effective size
/// = node_size for the containment check). Mirrors
/// `verify::sections` rejection messages so model invariants are
/// pinned at write time.
fn validate_section_aabb(
    node_size: baumhard::mindmap::model::Size,
    section_idx: usize,
    offset: baumhard::mindmap::model::Position,
    size: Option<baumhard::mindmap::model::Size>,
) -> Result<(), String> {
    validate_node_size(node_size)?;
    if !offset.x.is_finite() || !offset.y.is_finite() {
        return Err(format!(
            "section[{}].offset has non-finite component (x={}, y={})",
            section_idx, offset.x, offset.y
        ));
    }
    if offset.x < 0.0 {
        return Err(format!(
            "section[{}].offset.x is negative ({})",
            section_idx, offset.x
        ));
    }
    if offset.y < 0.0 {
        return Err(format!(
            "section[{}].offset.y is negative ({})",
            section_idx, offset.y
        ));
    }
    if let Some(s) = size {
        if !s.width.is_finite() || !s.height.is_finite() {
            return Err(format!(
                "section[{}].size has non-finite component (width={}, height={})",
                section_idx, s.width, s.height
            ));
        }
        if s.width <= 0.0 {
            return Err(format!(
                "section[{}].size.width is not positive ({})",
                section_idx, s.width
            ));
        }
        if s.height <= 0.0 {
            return Err(format!(
                "section[{}].size.height is not positive ({})",
                section_idx, s.height
            ));
        }
        if s.width > node_size.width * 100.0 {
            return Err(format!(
                "section[{}].size.width ({}) is over 100× the node's width ({}); \
                 likely a typo (e.g. an extra zero)",
                section_idx, s.width, node_size.width
            ));
        }
        if s.height > node_size.height * 100.0 {
            return Err(format!(
                "section[{}].size.height ({}) is over 100× the node's height ({}); \
                 likely a typo (e.g. an extra zero)",
                section_idx, s.height, node_size.height
            ));
        }
    }
    let effective = size.unwrap_or(node_size);
    let right = offset.x + effective.width;
    let bottom = offset.y + effective.height;
    if right > node_size.width {
        return Err(format!(
            "section[{}] extends past node right edge ({} > {})",
            section_idx, right, node_size.width
        ));
    }
    if bottom > node_size.height {
        return Err(format!(
            "section[{}] extends past node bottom edge ({} > {})",
            section_idx, bottom, node_size.height
        ));
    }
    Ok(())
}

/// Astronomical-typo guard for a candidate node `Size` — fixed
/// absolute bound rather than a multiplier against the prior
/// size, so a gesture that legitimately enlarges a tiny node by
/// many factors at release isn't silently rejected. The bound
/// (`MAX_NODE_AXIS`) is large enough to swallow any sane canvas
/// extent and small enough to flag an extra zero or two as the
/// "extra zero" canonical typo.
const MAX_NODE_AXIS: f64 = 1_000_000.0;

fn check_node_size_typo(size: baumhard::mindmap::model::Size) -> Result<(), String> {
    if size.width > MAX_NODE_AXIS {
        return Err(format!(
            "node.size.width ({}) exceeds the {} ceiling; likely a typo (e.g. an extra zero)",
            size.width, MAX_NODE_AXIS
        ));
    }
    if size.height > MAX_NODE_AXIS {
        return Err(format!(
            "node.size.height ({}) exceeds the {} ceiling; likely a typo (e.g. an extra zero)",
            size.height, MAX_NODE_AXIS
        ));
    }
    Ok(())
}

/// Validate a candidate `(x, y)` for a node — finite components
/// only. Nodes float freely on the canvas (no parent AABB), so
/// negative coordinates are legal — a node can sit at a negative
/// canvas-x to the left of the origin.
fn validate_node_position(pos: baumhard::mindmap::model::Position) -> Result<(), String> {
    if !pos.x.is_finite() || !pos.y.is_finite() {
        return Err(format!(
            "node.position has non-finite component (x={}, y={})",
            pos.x, pos.y
        ));
    }
    Ok(())
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
