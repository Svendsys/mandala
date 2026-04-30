// SPDX-License-Identifier: MPL-2.0

//! Edge visual styling — body glyph, caps, color, font sizing/family, spacing.


use baumhard::mindmap::model::{
    portal_endpoint_state_mut, EdgeLabelConfig, GlyphConnectionConfig, PortalEndpointState,
};

use super::super::types::EdgeRef;
use super::super::undo_action::UndoAction;
use super::super::MindMapDocument;
use super::inline::{
    ensure_glyph_connection_inline, ensure_label_config_inline,
};

impl MindMapDocument {
    /// Set the body glyph string for a connection. Empty strings are
    /// rejected (an empty body would produce no glyphs). Returns
    /// `true` if the edge existed and the body actually changed.
    pub fn set_edge_body_glyph(&mut self, edge_ref: &EdgeRef, body: &str) -> bool {
        if body.is_empty() {
            return false;
        }
        self.mutate_edge(edge_ref, |edge, canvas| {
            // Peek at the effective body before forking to detect no-ops.
            let default_body = GlyphConnectionConfig::default().body;
            let current_body = edge
                .glyph_connection
                .as_ref()
                .map(|c| c.body.as_str())
                .or_else(|| canvas.default_connection.as_ref().map(|c| c.body.as_str()))
                .unwrap_or(&default_body);
            if current_body == body {
                return false;
            }
            ensure_glyph_connection_inline(edge, canvas).body = body.to_string();
            true
        })
    }

    /// Set the `cap_start` glyph (or clear it with `None`). Returns
    /// `true` if the edge existed and the value changed.
    pub fn set_edge_cap_start(&mut self, edge_ref: &EdgeRef, cap: Option<&str>) -> bool {
        let new_val = cap.map(|s| s.to_string());
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            if cfg.cap_start == new_val {
                return false;
            }
            cfg.cap_start = new_val;
            true
        })
    }

    /// Set the `cap_end` glyph (or clear it with `None`). Returns
    /// `true` if the edge existed and the value changed.
    pub fn set_edge_cap_end(&mut self, edge_ref: &EdgeRef, cap: Option<&str>) -> bool {
        let new_val = cap.map(|s| s.to_string());
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            if cfg.cap_end == new_val {
                return false;
            }
            cfg.cap_end = new_val;
            true
        })
    }

    /// Set (or clear, with `color = None`) the `label_config.color`
    /// override on a line-mode edge's label. Sibling of
    /// [`Self::set_edge_color`], which targets the edge body cascade;
    /// this setter writes only the label channel so a coloured edge
    /// can carry a differently-coloured label. Forks a fresh
    /// `EdgeLabelConfig` on the edge if one isn't already present.
    /// Rolls back an all-default `EdgeLabelConfig` when clearing the
    /// color would leave the struct entirely empty, matching the
    /// rollback discipline on `set_portal_label_color` so unchanged
    /// selections don't leave undo droppings.
    pub fn set_edge_label_color(&mut self, edge_ref: &EdgeRef, color: Option<&str>) -> bool {
        let new_val = color.map(|s| s.to_string());
        self.mutate_edge(edge_ref, |edge, _canvas| {
            let current = edge.label_config.as_ref().and_then(|c| c.color.clone());
            if current == new_val {
                return false;
            }
            match new_val {
                Some(c) => {
                    ensure_label_config_inline(edge).color = Some(c);
                }
                None => {
                    if let Some(cfg) = edge.label_config.as_mut() {
                        cfg.color = None;
                        if cfg == &EdgeLabelConfig::default() {
                            edge.label_config = None;
                        }
                    }
                }
            }
            true
        })
    }

    /// Read the resolved **edge body** color for copy-to-clipboard.
    /// Walks the body cascade: `glyph_connection.color` →
    /// `edge.color`, with `var(--name)` references expanded
    /// through the theme variable map. Returns `None` only when
    /// the edge itself is missing; a no-override edge still
    /// produces a concrete hex (`edge.color` is always present
    /// in the model) so the user gets something pasteable. The
    /// clipboard copy on an `Edge` selection routes through this
    /// helper rather than duplicating the cascade inline, so a
    /// future change to the body cascade (e.g. a third tier) only
    /// touches one site.
    pub fn resolve_edge_color(&self, edge_ref: &EdgeRef) -> Option<String> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        let cfg =
            baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(edge, &self.mindmap.canvas);
        let raw = cfg.color.as_deref().unwrap_or(edge.color.as_str());
        Some(
            baumhard::util::color::resolve_var(raw, &self.mindmap.canvas.theme_variables)
                .to_string(),
        )
    }

    /// Read the resolved edge-label color for copy-to-clipboard.
    /// Walks the label color cascade: `label_config.color` →
    /// edge body cascade ([`Self::resolve_edge_color`]). The
    /// label channel's own override wins; absent override falls
    /// back to whatever the body cascade produces so the label
    /// visually matches the edge unless explicitly detached.
    pub fn resolve_edge_label_color(&self, edge_ref: &EdgeRef) -> Option<String> {
        let label_override = self
            .mindmap
            .edges
            .iter()
            .find(|e| edge_ref.matches(e))?
            .label_config
            .as_ref()
            .and_then(|c| c.color.clone());
        if let Some(hex) = label_override {
            return Some(
                baumhard::util::color::resolve_var(&hex, &self.mindmap.canvas.theme_variables)
                    .to_string(),
            );
        }
        self.resolve_edge_color(edge_ref)
    }

    /// Read the resolved portal-text color for copy-to-clipboard.
    /// Sibling of [`Self::resolve_portal_label_color`] targeting
    /// the text channel: cascade is `text_color` → icon color
    /// cascade (per-endpoint `color` → `glyph_connection.color` →
    /// `edge.color`). Returns `None` only when the edge is
    /// missing.
    pub fn resolve_portal_text_color(
        &self,
        edge_ref: &EdgeRef,
        endpoint_node_id: &str,
    ) -> Option<String> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        let state = baumhard::mindmap::model::portal_endpoint_state(edge, endpoint_node_id);
        // Text's own override wins; fall back to the icon's
        // already-resolved cascade via `resolve_portal_label_color`.
        if let Some(hex) = state.and_then(|s| s.text_color.as_deref()) {
            return Some(
                baumhard::util::color::resolve_var(hex, &self.mindmap.canvas.theme_variables)
                    .to_string(),
            );
        }
        self.resolve_portal_label_color(edge_ref, endpoint_node_id)
    }

    /// Set the color override on a connection's glyph_connection config.
    /// Passing `None` clears the override so the edge inherits from
    /// `edge.color` (or the canvas default). Returns `true` if the edge
    /// existed and the value changed.
    pub fn set_edge_color(&mut self, edge_ref: &EdgeRef, color: Option<&str>) -> bool {
        let new_val = color.map(|s| s.to_string());
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            if cfg.color == new_val {
                return false;
            }
            cfg.color = new_val;
            true
        })
    }

    /// Step the connection's base `font_size_pt` by `delta_pt`,
    /// clamped into `[min_font_size_pt, max_font_size_pt]`. Returns
    /// `true` if the clamp yielded a different value from the current
    /// (i.e. we're not already pinned at the relevant bound).
    pub fn set_edge_font_size_step(&mut self, edge_ref: &EdgeRef, delta_pt: f32) -> bool {
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            let new_val = (cfg.font_size_pt + delta_pt)
                .clamp(cfg.min_font_size_pt, cfg.max_font_size_pt);
            if (cfg.font_size_pt - new_val).abs() < f32::EPSILON {
                return false;
            }
            cfg.font_size_pt = new_val;
            true
        })
    }

    /// Set the connection's `font_size_pt` to an absolute value,
    /// clamped into `[min_font_size_pt, max_font_size_pt]`. Returns
    /// `true` if the clamped value differs from the current.
    ///
    /// Counterpart to [`set_edge_font_size_step`] for the console's
    /// `font size=<pt>` kv form, where callers have an absolute
    /// target rather than a delta.
    pub fn set_edge_font_size(&mut self, edge_ref: &EdgeRef, pt: f32) -> bool {
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            let new_val = pt.clamp(cfg.min_font_size_pt, cfg.max_font_size_pt);
            if (cfg.font_size_pt - new_val).abs() < f32::EPSILON {
                return false;
            }
            cfg.font_size_pt = new_val;
            true
        })
    }

    /// Reset the connection's `font_size_pt` to the hardcoded default
    /// (12.0). Returns `true` if the value actually changed.
    pub fn reset_edge_font_size(&mut self, edge_ref: &EdgeRef) -> bool {
        let default_size = GlyphConnectionConfig::default().font_size_pt;
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            if (cfg.font_size_pt - default_size).abs() < f32::EPSILON {
                return false;
            }
            cfg.font_size_pt = default_size;
            true
        })
    }

    /// Atomic `font size / min / max` setter for the edge body's
    /// `glyph_connection` channel. Applies `min` and `max` first,
    /// then clamps `size` against the **new** bounds, so the user-
    /// level command `font size=14 max=10` lands as `size=10, max=10`
    /// instead of the wrong `size=14, max=10` a naive one-at-a-time
    /// dispatch would produce. Each argument is optional; `None`
    /// leaves that field untouched. Returns `true` if any field
    /// changed. Rejects non-finite or non-positive values by
    /// leaving the field untouched.
    ///
    /// **Inverted bounds guard.** The resolved `(min, max)` pair
    /// (after applying overrides on top of the existing struct)
    /// must satisfy `min ≤ max`. Inverted input returns `false`
    /// without mutating — landing an inverted pair would panic
    /// the next renderer frame via
    /// [`baumhard::mindmap::model::GlyphConnectionConfig::effective_font_size_pt`]'s
    /// `clamp` call (interactive-path invariant per §9). The
    /// console `font` command re-checks up-front so the user gets
    /// a clear error message; this boundary check is defence in
    /// depth for any other caller.
    ///
    /// A single `EditEdge` undo entry covers the whole triple, so
    /// Ctrl+Z reverses the atomic edit in one step.
    pub fn set_edge_font(
        &mut self,
        edge_ref: &EdgeRef,
        size: Option<f32>,
        min: Option<f32>,
        max: Option<f32>,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        // Resolve the (min, max) pair that will land on the struct
        // if this call succeeds. Reject inverted pairs before any
        // mutation — Self::clamp panics on `min > max`, and a
        // later renderer frame hits the same panic via
        // `effective_font_size_pt`.
        let final_min = min
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(cfg.min_font_size_pt);
        let final_max = max
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(cfg.max_font_size_pt);
        if final_min > final_max {
            self.mindmap.edges[idx] = before;
            return false;
        }
        use baumhard::util::geometry::pretty_inequal;
        let mut changed = false;
        if let Some(m) = min.filter(|v| v.is_finite() && *v > 0.0) {
            if pretty_inequal(cfg.min_font_size_pt, m) {
                cfg.min_font_size_pt = m;
                changed = true;
            }
        }
        if let Some(m) = max.filter(|v| v.is_finite() && *v > 0.0) {
            if pretty_inequal(cfg.max_font_size_pt, m) {
                cfg.max_font_size_pt = m;
                changed = true;
            }
        }
        if let Some(s) = size.filter(|v| v.is_finite() && *v > 0.0) {
            // Bounds resolved above, known-ordered, safe for clamp.
            let clamped = s.clamp(cfg.min_font_size_pt, cfg.max_font_size_pt);
            if pretty_inequal(cfg.font_size_pt, clamped) {
                cfg.font_size_pt = clamped;
                changed = true;
            }
        }
        if !changed {
            self.mindmap.edges[idx] = before;
            return false;
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the edge body's `glyph_connection.font` family override.
    /// `Some("Norse")` pins the edge glyphs to that family; `None`
    /// clears the override (edge falls back to the canvas default
    /// font).
    ///
    /// Forks a fresh `GlyphConnectionConfig` on first edit via
    /// `ensure_glyph_connection`. A single `UndoAction::EditEdge`
    /// entry covers the change so Ctrl+Z reverses cleanly.
    /// Family-name validation is the caller's job — the data model
    /// stores the string verbatim and the tree builder resolves
    /// it through `baumhard::font::fonts::app_font_by_family` at
    /// render time, falling back to monospace with a warning if
    /// the family is unknown.
    pub fn set_edge_font_family(
        &mut self,
        edge_ref: &EdgeRef,
        family: Option<&str>,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        // Peek the effective family before forking so a no-op
        // clear (`None` on an edge that already has no override)
        // doesn't mint an undo entry.
        let current = self.mindmap.edges[idx]
            .glyph_connection
            .as_ref()
            .and_then(|c| c.font.as_deref());
        let target = family.filter(|s| !s.is_empty());
        if current == target {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        cfg.font = target.map(|s| s.to_string());
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Sibling of [`Self::set_edge_font`] targeting the edge
    /// **label** channel (`label_config.font_size_pt` / `min` /
    /// `max`). Same atomic ordering — min/max write before the
    /// clamped size — so label-level clamps can be tightened
    /// without dropping a concurrent size write. Forks a fresh
    /// `EdgeLabelConfig` on first edit; rolls back an all-default
    /// struct when clearing to None leaves nothing interesting.
    ///
    /// Resolver fallbacks: a label with no own override inherits
    /// the edge's `glyph_connection` clamps (see
    /// `EdgeLabelConfig::effective_font_size_pt`). Clamping the
    /// user-facing `size` value here happens against the
    /// **resolved** clamps — own min/max when set, edge min/max
    /// otherwise — so a label that only overrides `size` clamps
    /// into the edge's bounds without needing a full triple.
    pub fn set_edge_label_font(
        &mut self,
        edge_ref: &EdgeRef,
        size: Option<f32>,
        min: Option<f32>,
        max: Option<f32>,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        // Compute the resolved body clamps once for fallback
        // when the label config doesn't carry its own.
        let body_min;
        let body_max;
        {
            let edge = &self.mindmap.edges[idx];
            let cfg = GlyphConnectionConfig::resolved_for(edge, &self.mindmap.canvas);
            body_min = cfg.min_font_size_pt;
            body_max = cfg.max_font_size_pt;
        }
        // Resolve the (min, max) pair that will land after this
        // call. Either side falls back to the existing label
        // override or the body's clamp when the call leaves it
        // untouched. Inverted pairs bail before any mutation —
        // `f32::clamp` panics on `min > max`, and the renderer's
        // `effective_font_size_pt` would hit the same panic.
        let existing_label_min = self.mindmap.edges[idx]
            .label_config
            .as_ref()
            .and_then(|c| c.min_font_size_pt);
        let existing_label_max = self.mindmap.edges[idx]
            .label_config
            .as_ref()
            .and_then(|c| c.max_font_size_pt);
        let final_min = min
            .filter(|v| v.is_finite() && *v > 0.0)
            .or(existing_label_min)
            .unwrap_or(body_min);
        let final_max = max
            .filter(|v| v.is_finite() && *v > 0.0)
            .or(existing_label_max)
            .unwrap_or(body_max);
        if final_min > final_max {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        let label_cfg = Self::ensure_label_config(&mut self.mindmap.edges[idx]);
        let mut changed = false;
        if let Some(m) = min.filter(|v| v.is_finite() && *v > 0.0) {
            if label_cfg.min_font_size_pt != Some(m) {
                label_cfg.min_font_size_pt = Some(m);
                changed = true;
            }
        }
        if let Some(m) = max.filter(|v| v.is_finite() && *v > 0.0) {
            if label_cfg.max_font_size_pt != Some(m) {
                label_cfg.max_font_size_pt = Some(m);
                changed = true;
            }
        }
        if let Some(s) = size.filter(|v| v.is_finite() && *v > 0.0) {
            let effective_min = label_cfg.min_font_size_pt.unwrap_or(body_min);
            let effective_max = label_cfg.max_font_size_pt.unwrap_or(body_max);
            // `effective_{min,max}` are guaranteed ordered by the
            // `final_min > final_max` guard above — `effective_*`
            // resolve through the same `user-override → label
            // override → body clamp` cascade.
            let clamped = s.clamp(effective_min, effective_max);
            if label_cfg.font_size_pt != Some(clamped) {
                label_cfg.font_size_pt = Some(clamped);
                changed = true;
            }
        }
        // Rollback-on-noop + rollback-if-label-config-empty so
        // an unchanged triple doesn't leave an empty
        // `EdgeLabelConfig` behind.
        if !changed {
            self.mindmap.edges[idx] = before;
            return false;
        }
        if self.mindmap.edges[idx]
            .label_config
            .as_ref()
            .map_or(false, |c| c == &EdgeLabelConfig::default())
        {
            self.mindmap.edges[idx].label_config = None;
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Sibling of [`Self::set_edge_font`] targeting a portal
    /// endpoint's **text** channel
    /// (`PortalEndpointState.text_font_size_pt` / `text_min_font_size_pt`
    /// / `text_max_font_size_pt`). Same atomic ordering. Forks
    /// `PortalEndpointState` on first edit; rolls back an all-default
    /// endpoint state on clear. Fallback clamps come from the
    /// resolved `glyph_connection` when the endpoint's own clamps
    /// aren't set, matching the label resolver.
    pub fn set_portal_text_font(
        &mut self,
        edge_ref: &EdgeRef,
        endpoint_node_id: &str,
        size: Option<f32>,
        min: Option<f32>,
        max: Option<f32>,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let body_min;
        let body_max;
        {
            let edge = &self.mindmap.edges[idx];
            let cfg = GlyphConnectionConfig::resolved_for(edge, &self.mindmap.canvas);
            body_min = cfg.min_font_size_pt;
            body_max = cfg.max_font_size_pt;
        }
        // Check that the endpoint id resolves to a portal slot
        // before we clone the edge — cloning unnecessarily for a
        // bogus endpoint id would be wasteful.
        {
            let edge = &self.mindmap.edges[idx];
            if !(endpoint_node_id == edge.from_id || endpoint_node_id == edge.to_id) {
                return false;
            }
        }
        // Resolve the (min, max) pair that will land after this
        // call, using the same user-override → endpoint-override
        // → body-clamp cascade as `effective_font_size_pt` on the
        // render side. Reject inverted bounds before any mutation
        // to keep `clamp` panic-safe here and downstream.
        let (existing_text_min, existing_text_max) = {
            let edge = &self.mindmap.edges[idx];
            let state =
                baumhard::mindmap::model::portal_endpoint_state(edge, endpoint_node_id);
            (
                state.and_then(|s| s.text_min_font_size_pt),
                state.and_then(|s| s.text_max_font_size_pt),
            )
        };
        let final_min = min
            .filter(|v| v.is_finite() && *v > 0.0)
            .or(existing_text_min)
            .unwrap_or(body_min);
        let final_max = max
            .filter(|v| v.is_finite() && *v > 0.0)
            .or(existing_text_max)
            .unwrap_or(body_max);
        if final_min > final_max {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        let slot = match portal_endpoint_state_mut(
            &mut self.mindmap.edges[idx],
            endpoint_node_id,
        ) {
            Some(s) => s,
            None => return false,
        };
        // Track whether this call forked a fresh `PortalEndpointState`
        // so the default-scrub below only touches the slot this
        // call actually installed (a pre-existing default state on
        // the *other* endpoint must survive untouched).
        let forked_default = slot.is_none();
        let state = slot.get_or_insert_with(PortalEndpointState::default);
        let mut changed = false;
        if let Some(m) = min.filter(|v| v.is_finite() && *v > 0.0) {
            if state.text_min_font_size_pt != Some(m) {
                state.text_min_font_size_pt = Some(m);
                changed = true;
            }
        }
        if let Some(m) = max.filter(|v| v.is_finite() && *v > 0.0) {
            if state.text_max_font_size_pt != Some(m) {
                state.text_max_font_size_pt = Some(m);
                changed = true;
            }
        }
        if let Some(s) = size.filter(|v| v.is_finite() && *v > 0.0) {
            let effective_min = state.text_min_font_size_pt.unwrap_or(body_min);
            let effective_max = state.text_max_font_size_pt.unwrap_or(body_max);
            // Guaranteed ordered by the `final_min > final_max`
            // guard above.
            let clamped = s.clamp(effective_min, effective_max);
            if state.text_font_size_pt != Some(clamped) {
                state.text_font_size_pt = Some(clamped);
                changed = true;
            }
        }
        if !changed {
            self.mindmap.edges[idx] = before;
            return false;
        }
        // If this call forked a fresh default state and still
        // wrote nothing interesting (all writes would be
        // redundant), roll the slot back to `None` — matches the
        // label-config scrub discipline. Only the slot this call
        // wrote is touched; the *other* endpoint's state (even if
        // it happens to hold a pre-existing default) is left
        // alone, because the scrub is conditional on
        // `forked_default`.
        if forked_default {
            let edge = &mut self.mindmap.edges[idx];
            let post_state = baumhard::mindmap::model::portal_endpoint_state(
                edge,
                endpoint_node_id,
            );
            if post_state.map_or(false, |s| s == &PortalEndpointState::default()) {
                if endpoint_node_id == edge.from_id {
                    edge.portal_from = None;
                } else if endpoint_node_id == edge.to_id {
                    edge.portal_to = None;
                }
            }
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the connection's glyph `spacing` (canvas units between
    /// adjacent body glyphs). Returns `true` if the value actually
    /// changed.
    pub fn set_edge_spacing(&mut self, edge_ref: &EdgeRef, spacing: f32) -> bool {
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            if (cfg.spacing - spacing).abs() < f32::EPSILON {
                return false;
            }
            cfg.spacing = spacing;
            true
        })
    }
}
