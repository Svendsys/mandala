// SPDX-License-Identifier: MPL-2.0

//! The palette cascade — the one place that answers "what color is
//! this node, and what color is the edge leaving it?"
//!
//! A node binds itself to a map-level [`Palette`](super::Palette) by
//! carrying a [`ColorSchema`](super::ColorSchema): a palette name, a
//! depth `level`, and two miMind-inherited flags. Resolution walks
//! *palette first, `style` second* — the whole reason palettes were
//! hoisted out of the nodes during migration is that the palette, not
//! the node, is where a theme edit lands. `format/palettes.md` is the
//! normative spec; this module is its implementation, and the
//! projection passes under [`crate::mindmap::tree_builder`] read
//! colors only through the four `node_*` / `edge_*` readers below.
//!
//! ## Why the readers return `&str` and not a parsed color
//!
//! Both tiers store authored strings — `#rrggbb`, `#rrggbbaa`, or a
//! `var(--name)` reference the canvas theme map expands later. The
//! cascade's job is to pick *which string*, and
//! [`crate::util::color::resolve_var`] is a separate, later step that
//! every caller already performs. Returning a borrow keeps the whole
//! cascade allocation-free on a per-node, per-frame path (§4's mobile
//! budget).

use super::{ColorGroup, MindEdge, MindMap, MindNode};
use crate::util::color::{hex_to_rgba_safe, resolve_var, FloatRgba};

impl MindMap {
    /// Resolve the [`ColorGroup`] a themed node draws from, or
    /// `None` when the node is unthemed and its `style` colors
    /// stand.
    ///
    /// Four ways to get `None`, each meaning "fall back to
    /// `style`":
    ///
    /// 1. The node carries no `color_schema` at all.
    /// 2. `schema.palette` names a palette this map does not have.
    ///    `maptool verify` reports that as an error; the renderer
    ///    degrades instead of failing the frame
    ///    (`CODE_CONVENTIONS.md` §9).
    /// 3. The palette exists but has no groups — likewise a
    ///    `verify` error.
    /// 4. `starts_at_root` is `false` and the node *is* the schema
    ///    root (`level == 0`). The flag's whole meaning is that
    ///    level 0 belongs to the root's children, so the root
    ///    itself is left transparent and keeps its own `style`.
    ///
    /// Group index is `level` when `starts_at_root`, else
    /// `level - 1`. An index past the last group **clamps to the
    /// last group** rather than wrapping, so a subtree deeper than
    /// its palette degrades to a single color instead of cycling
    /// back to the root's.
    ///
    /// Cost: one `HashMap` lookup and one slice index. No
    /// allocation.
    pub fn resolve_theme_colors<'a>(&'a self, node: &'a MindNode) -> Option<&'a ColorGroup> {
        let schema = node.color_schema.as_ref()?;
        let palette = self.palettes.get(&schema.palette)?;
        let index = if schema.starts_at_root {
            schema.level
        } else {
            schema.level.checked_sub(1)?
        };
        palette.groups.get(index).or_else(|| palette.groups.last())
    }

    /// The authored **fill** color for a node: the resolved
    /// palette group's `background`, else `node.style.background_color`.
    ///
    /// The empty string is a meaningful value downstream — the node
    /// pass reads it as "no fill, let the canvas show through" — so
    /// it is passed along rather than treated as a miss. Cost: one
    /// [`Self::resolve_theme_colors`].
    pub fn node_background_color<'a>(&'a self, node: &'a MindNode) -> &'a str {
        match self.resolve_theme_colors(node) {
            Some(group) => group.background.as_str(),
            None => node.style.background_color.as_str(),
        }
    }

    /// The authored **frame** color for a node: the resolved
    /// palette group's `frame`, else `node.style.frame_color`.
    ///
    /// This is the cascade base the border resolver
    /// (`mindmap::border::resolve_border_style`) sits on top of, so
    /// a per-node `border.color` override still wins over the
    /// theme — an explicit choice beats an inherited one. Cost: one
    /// [`Self::resolve_theme_colors`].
    pub fn node_frame_color<'a>(&'a self, node: &'a MindNode) -> &'a str {
        match self.resolve_theme_colors(node) {
            Some(group) => group.frame.as_str(),
            None => node.style.frame_color.as_str(),
        }
    }

    /// The authored **text** color for a node: the resolved palette
    /// group's `text`, else `node.style.text_color`.
    ///
    /// This is the section-level default, not a per-grapheme one. A
    /// [`TextRun`](super::TextRun) carrying a non-empty `color`
    /// keeps it: a run is a deliberate per-slice override and the
    /// theme must not repaint it, exactly as an inline style beats
    /// an inherited one. The theme reaches text through the runs
    /// that *declined* to name a color and through sections that
    /// carry no runs at all. Cost: one
    /// [`Self::resolve_theme_colors`].
    pub fn node_text_color<'a>(&'a self, node: &'a MindNode) -> &'a str {
        match self.resolve_theme_colors(node) {
            Some(group) => group.text.as_str(),
            None => node.style.text_color.as_str(),
        }
    }

    /// The authored **title** color for a node — the first-line
    /// stand-in for [`Self::node_text_color`].
    ///
    /// Only the palette carries this channel; [`NodeStyle`] has no
    /// title field, so an unthemed node — or a themed one whose
    /// group leaves `title` empty — returns the node's text color
    /// and the first line is not distinguished.
    ///
    /// [`NodeStyle`]: super::NodeStyle
    ///
    /// Cost: one [`Self::resolve_theme_colors`].
    pub fn node_title_color<'a>(&'a self, node: &'a MindNode) -> &'a str {
        match self.resolve_theme_colors(node) {
            Some(group) if !group.title.is_empty() => group.title.as_str(),
            _ => self.node_text_color(node),
        }
    }

    /// [`Self::node_text_color`] carried the whole way to pixels —
    /// through the canvas theme variables and through
    /// [`crate::util::color::hex_to_rgba_safe`], with opaque black
    /// as the floor for an unparseable string.
    ///
    /// This is the value a section's glyphs are actually painted
    /// in when nothing more specific claims them, so it is what
    /// both the forward projection and any reverse comparison must
    /// use. Keeping it here rather than at the three call sites is
    /// what stops the projection and the comparison from drifting:
    /// a reverse gate that resolved the string differently from
    /// the forward pass would read every themed section as
    /// divergent.
    ///
    /// Cost: one [`Self::node_text_color`] plus one hex parse. No
    /// allocation — `resolve_var` borrows when there is nothing to
    /// substitute.
    pub fn node_text_rgba(&self, node: &MindNode) -> FloatRgba {
        let resolved = resolve_var(self.node_text_color(node), &self.canvas.theme_variables);
        hex_to_rgba_safe(resolved, [0.0, 0.0, 0.0, 1.0])
    }

    /// The palette stroke color an edge inherits from its **source**
    /// node, or `None` when no theme tier applies.
    ///
    /// This is the tier [`MindEdge::body_color`] takes as its
    /// `themed` argument; resolve it here rather than at the call
    /// site so every consumer of the edge color cascade agrees on
    /// what "the theme" means for an edge.
    ///
    /// Two conditions gate it: the source node must resolve to a
    /// palette group at all, and that node's
    /// `color_schema.connections_colored` must be `true` — the flag
    /// is per-schema precisely so a themed subtree can keep its
    /// node fills while leaving its connections alone.
    ///
    /// **The source node, not the target.** An edge is drawn in its
    /// parent's branch color, which is what the miMind-derived
    /// corpus shows: of the 248 `parent_child` edges in
    /// `maps/testament.mindmap.json`, 229 carry their `from_id`
    /// node's frame color and 5 carry their `to_id` node's. A
    /// cross-link is colored by the same rule, so the direction the
    /// author drew it in is the direction it takes its color from.
    ///
    /// The group's `frame` is the channel used, not `background`: a
    /// connection is a stroke, and the frame is the palette's
    /// stroke color for that depth.
    ///
    /// Cost: one node lookup plus one
    /// [`Self::resolve_theme_colors`]. No allocation.
    pub fn edge_theme_stroke_color<'a>(&'a self, edge: &'a MindEdge) -> Option<&'a str> {
        let from = self.nodes.get(&edge.from_id)?;
        if !from
            .color_schema
            .as_ref()
            .is_some_and(|schema| schema.connections_colored)
        {
            return None;
        }
        Some(self.resolve_theme_colors(from)?.frame.as_str())
    }

    /// [`MindEdge::body_color`] with this map's theme tier already
    /// supplied — the spelling every projection pass should use.
    ///
    /// Cost: [`Self::edge_theme_stroke_color`] plus the edge
    /// cascade's own O(1).
    pub fn edge_body_color<'a>(&'a self, edge: &'a MindEdge) -> &'a str {
        edge.body_color(&self.canvas, self.edge_theme_stroke_color(edge))
    }

    /// [`MindEdge::label_color`] with this map's theme tier already
    /// supplied. Cost: as [`Self::edge_body_color`].
    pub fn edge_label_color<'a>(&'a self, edge: &'a MindEdge) -> &'a str {
        edge.label_color(&self.canvas, self.edge_theme_stroke_color(edge))
    }

    /// [`MindEdge::portal_endpoint_color`] with this map's theme
    /// tier already supplied. `endpoint` is the state for the side
    /// being drawn — resolve it with
    /// [`portal_endpoint_state`](super::portal_endpoint_state)
    /// first. Cost: as [`Self::edge_body_color`].
    pub fn edge_portal_endpoint_color<'a>(
        &'a self,
        edge: &'a MindEdge,
        endpoint: Option<&'a super::PortalEndpointState>,
    ) -> &'a str {
        edge.portal_endpoint_color(&self.canvas, endpoint, self.edge_theme_stroke_color(edge))
    }

    /// [`MindEdge::portal_endpoint_text_color`] with this map's
    /// theme tier already supplied. Cost: as
    /// [`Self::edge_body_color`].
    pub fn edge_portal_endpoint_text_color<'a>(
        &'a self,
        edge: &'a MindEdge,
        endpoint: Option<&'a super::PortalEndpointState>,
    ) -> &'a str {
        edge.portal_endpoint_text_color(&self.canvas, endpoint, self.edge_theme_stroke_color(edge))
    }
}

#[cfg(test)]
mod tests {
    use crate::mindmap::model::{ColorGroup, ColorSchema, MindMap, Palette};
    use crate::mindmap::test_helpers::{synthetic_edge, synthetic_map, synthetic_node_full};

    /// Three groups whose every channel is a distinct sentinel, so a
    /// reader that grabs the wrong one cannot accidentally pass.
    fn probe_palette() -> Palette {
        Palette {
            groups: vec![
                group("#a10000", "#a20000", "#a30000", "#a40000"),
                group("#b10000", "#b20000", "#b30000", "#b40000"),
                group("#c10000", "#c20000", "#c30000", ""),
            ],
        }
    }

    fn group(background: &str, frame: &str, text: &str, title: &str) -> ColorGroup {
        ColorGroup {
            background: background.into(),
            frame: frame.into(),
            text: text.into(),
            title: title.into(),
        }
    }

    fn schema(level: usize, starts_at_root: bool, connections_colored: bool) -> ColorSchema {
        ColorSchema {
            palette: "probe".into(),
            level,
            starts_at_root,
            connections_colored,
        }
    }

    /// Two nodes joined `a -> b`, both carrying the sentinel styles
    /// `synthetic_node_full` pins (`#000` fill, `#fff` frame and
    /// text) so any palette value is distinguishable from the
    /// fallback.
    fn probe_map() -> MindMap {
        let mut map = synthetic_map(
            vec![
                synthetic_node_full("a", None, 0.0, 0.0, 80.0, 40.0, true),
                synthetic_node_full("b", Some("a"), 200.0, 0.0, 80.0, 40.0, true),
            ],
            vec![synthetic_edge("a", "b", "right", "left")],
        );
        map.palettes.insert("probe".into(), probe_palette());
        map
    }

    #[test]
    fn test_resolve_theme_colors_indexes_the_group_at_level() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(1, true, false));
        let node = map.nodes.get("a").unwrap();
        let resolved = map.resolve_theme_colors(node).expect("level 1 resolves");
        assert_eq!(resolved.frame, "#b20000");
    }

    #[test]
    fn test_resolve_theme_colors_none_without_a_schema() {
        let map = probe_map();
        let node = map.nodes.get("a").unwrap();
        assert!(map.resolve_theme_colors(node).is_none());
    }

    #[test]
    fn test_resolve_theme_colors_none_for_a_missing_palette() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(ColorSchema {
            palette: "no-such-palette".into(),
            level: 0,
            starts_at_root: true,
            connections_colored: false,
        });
        let node = map.nodes.get("a").unwrap();
        assert!(
            map.resolve_theme_colors(node).is_none(),
            "a dangling palette reference degrades to the style fallback"
        );
    }

    #[test]
    fn test_resolve_theme_colors_none_for_an_empty_palette() {
        let mut map = probe_map();
        map.palettes
            .insert("probe".into(), Palette { groups: Vec::new() });
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
        let node = map.nodes.get("a").unwrap();
        assert!(map.resolve_theme_colors(node).is_none());
    }

    /// The clamp branch: a level past the last group lands on the
    /// last group rather than wrapping to `groups[0]` or panicking.
    #[test]
    fn test_resolve_theme_colors_clamps_a_level_past_the_last_group() {
        let mut map = probe_map();
        for level in [3usize, 4, 9, usize::MAX] {
            map.nodes.get_mut("a").unwrap().color_schema = Some(schema(level, true, false));
            let node = map.nodes.get("a").unwrap();
            let resolved = map
                .resolve_theme_colors(node)
                .expect("an out-of-range level still resolves");
            assert_eq!(
                resolved.frame, "#c20000",
                "level {level} must clamp to the last group, not wrap"
            );
        }
    }

    /// `starts_at_root = false` leaves the schema root itself
    /// unthemed and shifts its children down one group.
    #[test]
    fn test_starts_at_root_false_leaves_the_root_transparent() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, false, false));
        let node = map.nodes.get("a").unwrap();
        assert!(
            map.resolve_theme_colors(node).is_none(),
            "level 0 under starts_at_root = false is the transparent root"
        );
        assert_eq!(
            map.node_background_color(node),
            "#000",
            "and it keeps its own style fill"
        );
    }

    #[test]
    fn test_starts_at_root_false_shifts_children_onto_group_zero() {
        let mut map = probe_map();
        map.nodes.get_mut("b").unwrap().color_schema = Some(schema(1, false, false));
        let node = map.nodes.get("b").unwrap();
        let resolved = map.resolve_theme_colors(node).expect("level 1 resolves");
        assert_eq!(resolved.frame, "#a20000", "level 1 - 1 = groups[0]");
    }

    #[test]
    fn test_starts_at_root_true_and_false_disagree_by_one_group() {
        let mut map = probe_map();
        map.nodes.get_mut("b").unwrap().color_schema = Some(schema(2, true, false));
        let with_root = map
            .resolve_theme_colors(map.nodes.get("b").unwrap())
            .unwrap()
            .frame
            .clone();
        map.nodes.get_mut("b").unwrap().color_schema = Some(schema(2, false, false));
        let without_root = map
            .resolve_theme_colors(map.nodes.get("b").unwrap())
            .unwrap()
            .frame
            .clone();
        assert_eq!(with_root, "#c20000");
        assert_eq!(without_root, "#b20000");
    }

    /// Each role reads its own channel, and each falls back to its
    /// own `style` field when the node is unthemed.
    #[test]
    fn test_node_color_roles_read_their_own_channel() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
        let node = map.nodes.get("a").unwrap();
        assert_eq!(map.node_background_color(node), "#a10000");
        assert_eq!(map.node_frame_color(node), "#a20000");
        assert_eq!(map.node_text_color(node), "#a30000");
        assert_eq!(map.node_title_color(node), "#a40000");
    }

    #[test]
    fn test_node_color_roles_fall_back_to_style_without_a_schema() {
        let map = probe_map();
        let node = map.nodes.get("a").unwrap();
        assert_eq!(map.node_background_color(node), "#000");
        assert_eq!(map.node_frame_color(node), "#fff");
        assert_eq!(map.node_text_color(node), "#fff");
        assert_eq!(
            map.node_title_color(node),
            "#fff",
            "no style field carries a title color, so it follows text"
        );
    }

    #[test]
    fn test_node_title_color_falls_back_to_text_when_the_group_leaves_it_empty() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(2, true, false));
        let node = map.nodes.get("a").unwrap();
        assert_eq!(map.node_title_color(node), "#c30000");
    }

    /// The headline claim in `format/palettes.md`: editing the
    /// palette re-themes every node bound to it, with no per-node
    /// edit.
    #[test]
    fn test_editing_the_palette_changes_every_bound_node() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
        map.nodes.get_mut("b").unwrap().color_schema = Some(schema(0, true, false));
        assert_eq!(map.node_background_color(map.nodes.get("a").unwrap()), "#a10000");
        assert_eq!(map.node_background_color(map.nodes.get("b").unwrap()), "#a10000");

        map.palettes.get_mut("probe").unwrap().groups[0].background = "#0f0f0f".into();

        assert_eq!(map.node_background_color(map.nodes.get("a").unwrap()), "#0f0f0f");
        assert_eq!(map.node_background_color(map.nodes.get("b").unwrap()), "#0f0f0f");
    }

    #[test]
    fn test_edge_theme_stroke_color_follows_the_source_node() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, true));
        map.nodes.get_mut("b").unwrap().color_schema = Some(schema(1, true, true));
        assert_eq!(
            map.edge_theme_stroke_color(&map.edges[0]),
            Some("#a20000"),
            "the edge takes its from_id node's group, not its to_id node's"
        );
    }

    #[test]
    fn test_edge_theme_stroke_color_none_when_connections_are_not_colored() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
        assert!(map.edge_theme_stroke_color(&map.edges[0]).is_none());
    }

    #[test]
    fn test_edge_theme_stroke_color_none_when_the_source_is_unthemed() {
        let map = probe_map();
        assert!(map.edge_theme_stroke_color(&map.edges[0]).is_none());
    }

    #[test]
    fn test_edge_body_color_prefers_the_theme_over_the_edges_own_color() {
        let mut map = probe_map();
        map.edges[0].color = "#edge00".into();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, true));
        assert_eq!(map.edge_body_color(&map.edges[0]), "#a20000");
        // …and gives it back the moment the flag comes off.
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, false));
        assert_eq!(map.edge_body_color(&map.edges[0]), "#edge00");
    }

    #[test]
    fn test_an_explicit_connection_color_outranks_the_theme() {
        use crate::mindmap::model::GlyphConnectionConfig;
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, true));
        map.edges[0].glyph_connection = Some(GlyphConnectionConfig {
            color: Some("#explicit".into()),
            ..GlyphConnectionConfig::default()
        });
        assert_eq!(
            map.edge_body_color(&map.edges[0]),
            "#explicit",
            "a color the author named on the edge is not a theme's to overrule"
        );
    }

    #[test]
    fn test_edge_label_and_portal_channels_inherit_the_theme_through_the_body() {
        let mut map = probe_map();
        map.nodes.get_mut("a").unwrap().color_schema = Some(schema(0, true, true));
        let edge = &map.edges[0];
        assert_eq!(map.edge_label_color(edge), "#a20000");
        assert_eq!(map.edge_portal_endpoint_color(edge, None), "#a20000");
        assert_eq!(map.edge_portal_endpoint_text_color(edge, None), "#a20000");
    }
}
