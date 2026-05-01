// SPDX-License-Identifier: MPL-2.0

//! Node data model: `MindNode` and the small structs that travel with
//! it — position, size, text runs, node style, layout, colour schema,
//! and the glyph-border config. Borders belong here because they are
//! always per-node (no edge-level borders exist).

use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::gfx_structs::zoom_visibility::ZoomVisibility;
use crate::mindmap::custom_mutation::{CustomMutation, TriggerBinding};

/// A single node in the mindmap: a styled rectangle that hosts one
/// or more [`MindSection`]s carrying the text content. The node
/// itself owns the visual chrome (background, frame, shape, border,
/// shadow) and the structural pieces (`parent_id`, `channel`,
/// trigger bindings, palette binding); the *user-facing strata of
/// data* live on its sections.
///
/// Attached to a tree position via [`Self::parent_id`] and a
/// Dewey-decimal [`Self::id`]. The loader materializes one of these
/// per `.mindmap.json` entry; the scene builder and tree builder
/// both project from this shape.
///
/// Plain data; no runtime cost beyond the `String` allocations serde
/// performs on deserialize.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MindNode {
    /// Dewey-decimal node id (e.g. `"0"`, `"0.1"`, `"0.1.3"`); the
    /// parent prefix establishes the tree structure redundantly with
    /// [`Self::parent_id`]. Must be unique across the map.
    pub id: String,
    /// Parent node id, or `None` for the root. Source of truth for
    /// tree structure; [`Self::id`]'s Dewey prefix must agree.
    pub parent_id: Option<String>,
    /// Canvas-space top-left corner of the node's AABB.
    pub position: Position,
    /// Canvas-space width and height of the node's AABB.
    pub size: Size,
    /// The user data strata of this node, in render order. Each
    /// section is a positioned text-bearing surface inside the node
    /// AABB. Validation invariants:
    ///
    /// - `sections` is non-empty for every renderable node — the
    ///   loader rejects maps where any node ships zero sections,
    ///   pointing at `maptool convert --sections` for migration.
    /// - Section `offset` + `size` should stay within the node's
    ///   AABB; `maptool verify` flags out-of-bounds sections.
    /// - Section `channel` collisions with sibling sections are
    ///   warned (not failed) by `verify`; collisions broadcast a
    ///   single mutation across both, which is occasionally the
    ///   intent.
    ///
    /// `#[serde(default)]` lets the typed shape parse a node
    /// without `sections` (deserialising as empty) so unit tests
    /// that synthesise `MindNode` from raw JSON skipping
    /// section authoring still parse — the *loader* is the layer
    /// that rejects zero-section maps with a migration pointer.
    #[serde(default)]
    pub sections: Vec<MindSection>,
    /// Background / frame / text colours, border, shape, and the
    /// visible-frame toggle. The text colour here acts as the
    /// node-level default — sections without their own per-run
    /// colour override fall through to it.
    pub style: NodeStyle,
    /// Layout descriptor carried through from miMind-format source
    /// maps. Mandala drives layout through custom mutations instead
    /// (see `format/mutations.md`); this is round-trip fidelity only.
    pub layout: NodeLayout,
    /// `true` when the node's subtree is collapsed — children stay in
    /// the model but the scene builder treats them as hidden.
    pub folded: bool,
    /// Long-form text attached to the node; rendered separately from
    /// the node's [`MindSection`] text by the notes overlay path.
    /// Carried at the node level rather than per-section because
    /// notes annotate the whole node, not any one stratum of data
    /// inside it.
    pub notes: String,
    /// Optional palette binding that colours this node and its
    /// descendants at a given depth level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_schema: Option<ColorSchema>,
    /// Channel index for mutation targeting in the baumhard tree.
    /// Multiple siblings can share a channel to form broadcast groups.
    #[serde(default)]
    pub channel: usize,
    /// Trigger bindings attached to this specific node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trigger_bindings: Vec<TriggerBinding>,
    /// Inline custom mutations defined on this node (not shared with other nodes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_mutations: Vec<CustomMutation>,
    /// Per-node macro definitions, opaque to baumhard. Loaded into
    /// the application-side `MacroRegistry` at the `Inline` tier
    /// (highest precedence — overrides Map / User / App on
    /// id collision). Stored as untyped JSON values for the same
    /// reason as `MindMap.macros`: the typed `Macro` lives in the
    /// application crate.
    ///
    /// Privilege model: Inline-tier macros — same as Map-tier —
    /// cannot run `ConsoleLine` or destructive `Action` variants
    /// (`SaveDocument`, `DeleteSelection`, `Cut`, `Paste`, `Copy`,
    /// `OrphanSelection`, `CreateOrphanNode`,
    /// `CreateOrphanNodeAndEdit`, `DoubleClickActivate`,
    /// `EditSelection`, `EditSelectionClean`, `NewDocument`).
    /// The privilege gate is enforced at dispatch time in the
    /// application's `dispatch_macro`; see `format/macros.md`
    /// for the full threat model.
    ///
    /// Cross-node id collisions inside the Inline tier are
    /// non-deterministic — namespace your ids
    /// (e.g. `<node-id>.action`) to avoid them. The loader emits
    /// a `warn!` when a collision is detected at load time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_macros: Vec<serde_json::Value>,
    /// Lower bound on `camera.zoom` at which this node (and its
    /// glyph border, which inherits from the node) renders.
    /// `None` = unbounded below. Mirrors the
    /// `min_font_size_pt` / `max_font_size_pt` pair on
    /// [`crate::mindmap::model::edge::GlyphConnectionConfig`] —
    /// same flat-optional posture, orthogonal concept (presence
    /// vs. size). Inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_zoom_to_render: Option<f32>,
    /// Upper bound on `camera.zoom` at which this node renders.
    /// `None` = unbounded above. Inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_zoom_to_render: Option<f32>,
}

impl MindNode {
    /// This node's authored zoom window, as a
    /// [`ZoomVisibility`]. O(1).
    ///
    /// # Border inheritance
    ///
    /// Borders inherit this window verbatim via
    /// [`BorderNodeData::zoom_visibility`] in
    /// `tree_builder/border.rs` (stamped onto all four runs in
    /// `border_node_data`) and via the same field on
    /// `BorderElement` in `scene_builder/node_pass.rs` — both
    /// paths call this method directly. No separate
    /// per-border override exists today; the floating-frame-
    /// fragment case a non-inheriting border would produce is
    /// prevented by construction. A future
    /// `GlyphBorderConfig.min_zoom_to_render` field would need
    /// to revisit those two call sites together.
    ///
    /// [`BorderNodeData::zoom_visibility`]: crate::mindmap::tree_builder::BorderNodeData
    pub fn zoom_window(&self) -> ZoomVisibility {
        ZoomVisibility::from_pair(self.min_zoom_to_render, self.max_zoom_to_render)
    }

    /// Top-left corner of the node's AABB in canvas space, as a
    /// `glam::Vec2`. The model stores position as `f64` for serde
    /// fidelity; downstream geometry (camera transforms, hit tests,
    /// connection sampling) runs at `f32`. This helper performs the
    /// conversion at the model boundary so call sites can drop the
    /// `Vec2::new(node.position.x as f32, node.position.y as f32)`
    /// boilerplate.
    ///
    /// **Costs.** Two f64→f32 narrowing casts. No allocation
    /// (`glam::Vec2` is `#[repr(C)] [f32; 2]`). Lossy past ~16M
    /// canvas pixels — beyond f32's 24-bit integer-precise range —
    /// but well clear of any realistic mindmap canvas size.
    /// Inlinable; the optimiser folds construction-then-`.x`/`.y`
    /// access into bare register loads.
    pub fn pos_vec2(&self) -> Vec2 {
        Vec2::new(self.position.x as f32, self.position.y as f32)
    }

    /// Width/height of the node's AABB in canvas space, as a
    /// `glam::Vec2`. Sibling of [`Self::pos_vec2`].
    ///
    /// **Costs.** Same as [`Self::pos_vec2`] — two f64→f32 casts,
    /// no allocation, optimiser-foldable.
    pub fn size_vec2(&self) -> Vec2 {
        Vec2::new(self.size.width as f32, self.size.height as f32)
    }

    /// All section text concatenated with `'\n'` between sections —
    /// for legacy consumers (markdown export, plain-text dump) that
    /// want one rendered string per node. Single-section nodes (the
    /// legacy-migration default) round-trip with no surprises:
    /// `display_text()` is just `sections[0].text`.
    ///
    /// **Costs.** O(total section text bytes); one fresh `String`
    /// allocation sized to the joined output. Hot paths that need
    /// per-section rendering should walk `sections` directly
    /// instead of reaching for this helper.
    pub fn display_text(&self) -> String {
        let mut out = String::new();
        for (i, section) in self.sections.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&section.text);
        }
        out
    }

    /// Center of the node's AABB in canvas space — `pos + size / 2`.
    /// Used by edge-anchor and connection-routing math that needs
    /// the node's geometric centre rather than its top-left corner.
    ///
    /// **Costs.** Four f64→f32 casts (two from each helper) plus a
    /// componentwise `Vec2 * f32` and `Vec2 + Vec2`. With glam's
    /// SIMD path enabled the multiply-and-add is two SSE/NEON ops.
    /// Per-frame call sites typically hit this once per visible
    /// edge endpoint; if a profile implicates this hot, add a
    /// `do_*()` benchmark in `lib/baumhard/benches/test_bench.rs`
    /// alongside the existing geometry benches.
    pub fn center_vec2(&self) -> Vec2 {
        self.pos_vec2() + self.size_vec2() * 0.5
    }
}

/// Canvas-space top-left corner of a node's AABB, or — when used on
/// a [`MindSection`] — the node-local offset where the section
/// sits inside its owning node. Units are arbitrary canvas pixels
/// (the camera transforms to screen space at render time). Plain
/// data; no runtime cost.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Position {
    /// Canvas-space x coordinate (or node-local offset for sections).
    #[serde(default)]
    pub x: f64,
    /// Canvas-space y coordinate (canvas y-axis grows downward).
    #[serde(default)]
    pub y: f64,
}

/// Canvas-space extent of a node's AABB. Width and height are
/// strictly positive in practice but not checked at type level —
/// scene-builder code guards against zero-size nodes on its own.
/// Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Size {
    /// Width in canvas units.
    pub width: f64,
    /// Height in canvas units.
    pub height: f64,
}

/// One stratum of user data inside a [`MindNode`]. A section is a
/// positioned, text-bearing surface that lives inside the parent
/// node's AABB. Nodes can carry one or many sections; the
/// post-section-refactor data shape moves the old per-node
/// `text` + `text_runs` pair onto each section.
///
/// In the Baumhard tree (see [`crate::mindmap::tree_builder`])
/// each section materialises as a `GfxElement::GlyphArea` child
/// of the owning node's container area, with a single
/// `GfxElement::GlyphModel` grandchild that paints the section's
/// glyphs into the section-area's buffer. The section-area is the
/// hit-test target for click routing and the carrier for
/// per-section style mutations; the section-model carries the
/// actual glyph composition.
///
/// **Defaults / inheritance.** A section without `text_runs`
/// renders at cosmic-text's defaults clamped by `node.style`:
/// the section's effective colour falls through to
/// `node.style.text_color`, and the size to `14.0pt`. Per-grapheme
/// styling layers via `text_runs`.
///
/// Plain data; no runtime cost beyond the `String` allocations
/// serde performs on deserialize.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MindSection {
    /// Primary text content. Styled-slice overrides live in
    /// [`Self::text_runs`]; the empty-runs / partial-runs trade-off
    /// matches the pre-refactor [`TextRun`] contract — non-empty
    /// runs render *only* the covered ranges.
    pub text: String,
    /// Per-grapheme styled slices — see [`TextRun`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_runs: Vec<TextRun>,
    /// Top-left of the section's AABB *relative to the owning
    /// node's `position`*, in canvas units. `(0, 0)` puts the
    /// section flush against the node's top-left.
    #[serde(default, skip_serializing_if = "is_default_position")]
    pub offset: Position,
    /// Section AABB. `None` means "fill the parent node" — the
    /// tree and scene builders compute the absolute size from the
    /// node at projection time. Authors who want a section to
    /// occupy only part of a node's AABB set this explicitly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Size>,
    /// Channel index for mutation routing inside the parent
    /// node-area. `None` means "let the tree builder substitute
    /// the section's index" — which is the migration default and
    /// what most authored sections want. `Some(0)` means "the
    /// author explicitly chose channel 0", which the builder
    /// honours even when the section's index is `> 0` (so
    /// authored 0 can intentionally collide with a sibling
    /// mind-node on channel 0 to broadcast).
    ///
    /// Sibling section channels still collide with any child
    /// mind-node sharing the same channel inside the same node —
    /// by design today. The
    /// `TargetScope::SectionsOnly` + `GfxElementField::Flag`
    /// predicate seam disambiguates at the mutation layer; this
    /// field's job is just to carry the routing intent without
    /// the `0 == default` ambiguity that the bare `usize` carried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<usize>,
    /// Per-section trigger bindings — closes the
    /// `TriggerBindings` seam at zero data-shape cost. Today no
    /// dispatcher consults this field (whole-node bindings still
    /// flow through `MindNode.trigger_bindings`); the field is
    /// reserved so authoring tools can stamp section-targeted
    /// `OnClick` / `OnKey` / `OnLink` bindings without a follow-up
    /// data migration when the dispatcher wires up.
    ///
    /// `#[serde(default)]` + `skip_serializing_if = "Vec::is_empty"`
    /// keeps the on-disk format byte-identical for sections that
    /// don't carry bindings — every existing `.mindmap.json`
    /// file remains valid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trigger_bindings: Vec<TriggerBinding>,
}

fn is_default_position(p: &Position) -> bool {
    p.x == 0.0 && p.y == 0.0
}

impl MindSection {
    /// Build a section that owns the given text and runs and
    /// otherwise inherits everything from the parent node — fills
    /// the node's AABB, channel 0, no offset. The right shape for
    /// the legacy single-section migration (`maptool convert
    /// --sections`) and for fresh orphan nodes that ship with one
    /// default section so users can start typing immediately.
    pub fn new_default(text: String, text_runs: Vec<TextRun>) -> Self {
        MindSection {
            text,
            text_runs,
            offset: Position::default(),
            size: None,
            channel: None,
            trigger_bindings: Vec::new(),
        }
    }
}

/// A styled slice of a section's `text`, matching miMind's text-run
/// concept: `[start, end)` grapheme indices carry one font / size /
/// color / style combination, with optional hyperlink target.
/// Multiple runs describe a single multi-style string; gaps in
/// coverage render with section-level defaults (which themselves
/// fall through to the owning node's defaults).
///
/// Plain data; no runtime cost beyond the string allocations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextRun {
    /// Grapheme-cluster index where this run begins (inclusive).
    pub start: usize,
    /// Grapheme-cluster index where this run ends (exclusive).
    pub end: usize,
    /// Bold weight flag.
    pub bold: bool,
    /// Italic style flag.
    pub italic: bool,
    /// Underline decoration flag.
    pub underline: bool,
    /// Font-family name; matched against `AppFont` at layout time
    /// with a fallback for unrecognised families.
    pub font: String,
    /// Font size in points.
    pub size_pt: u32,
    /// `#RRGGBB` or `var(--name)` text colour.
    pub color: String,
    /// Optional hyperlink target URL; the renderer decorates the
    /// run's underline when set.
    pub hyperlink: Option<String>,
}

/// Visual style for one node's frame / background / text. Colors are
/// raw `#RRGGBB` or `var(--name)` strings — callers pass them through
/// `util::color::resolve_var` against the canvas theme map before
/// rasterizing. Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStyle {
    /// Fill colour (`#RRGGBB` or `var(--name)`).
    pub background_color: String,
    /// Border / frame colour (`#RRGGBB` or `var(--name)`).
    pub frame_color: String,
    /// Default text colour for the node's primary text (`#RRGGBB`
    /// or `var(--name)`).
    pub text_color: String,
    /// Background shape spelling — matched against
    /// [`crate::gfx_structs::shape::NodeShape::from_style_string`].
    /// Falls back to rectangle on unknown values.
    #[serde(default = "default_shape")]
    pub shape: String,
    /// Corner radius as a percentage of the smaller AABB dimension
    /// (0 = square corners).
    pub corner_radius_percent: f64,
    /// Frame stroke thickness in canvas units.
    pub frame_thickness: f64,
    /// When `true`, render the frame stroke at all.
    pub show_frame: bool,
    /// When `true`, render a drop shadow behind the node.
    pub show_shadow: bool,
    /// Glyph-based border configuration. Optional — if absent, the renderer
    /// applies a default border style based on the node's frame_color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<GlyphBorderConfig>,
}

/// Configures how a node's border is rendered using font glyphs.
/// All fields are optional with sensible defaults so the format stays forgiving.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlyphBorderConfig {
    /// Which glyph preset to use: "light", "heavy", "double", "rounded", or "custom"
    #[serde(default = "default_border_preset")]
    pub preset: String,
    /// Font family name for border glyphs. None = system default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Font size in points for border glyphs.
    #[serde(default = "default_border_font_size")]
    pub font_size_pt: f32,
    /// Border color override as #RRGGBB. None = inherit from frame_color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Custom glyph definitions. Only used when preset = "custom".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyphs: Option<CustomBorderGlyphs>,
    /// Padding between border and content (in pixels).
    #[serde(default = "default_border_padding")]
    pub padding: f32,
    /// Optional palette name (key in `MindMap.palettes`) whose
    /// colours cycle per glyph around the border. When absent, the
    /// resolved single colour (cascade `border.color` →
    /// `style.frame_color`) paints every glyph. When present but
    /// missing from the map, the renderer logs a warning and falls
    /// back to the single-colour path — interactive paths must not
    /// panic per `CODE_CONVENTIONS.md` §9.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_palette: Option<String>,
    /// Which `ColorGroup` field is cycled when `color_palette` is
    /// set: `"frame" | "background" | "text" | "title"`. Defaults
    /// to `"frame"` (the channel whose meaning matches the border
    /// today). Unknown values warn-and-fall-back to `"frame"`.
    /// Open seam for "cycle title colours" without redesigning the
    /// field later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_palette_field: Option<String>,
}

fn default_shape() -> String {
    "rectangle".to_string()
}
// `light` is the chosen default rather than `rounded` so corner
// glyphs (`┌┐└┘`) extend to the cell edges and join cleanly with
// the side glyphs in monospace fonts. The rounded preset's
// `╭╮╰╯` curve inward away from the cell edges, leaving a visible
// gap at every corner.
fn default_border_preset() -> String {
    "light".to_string()
}
fn default_border_font_size() -> f32 {
    14.0
}
fn default_border_padding() -> f32 {
    4.0
}

/// Custom glyphs for each part of the border.
/// Each field is a string (single char or multi-char glyph).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomBorderGlyphs {
    /// Glyph used for the horizontal top edge.
    #[serde(default = "default_h_glyph")]
    pub top: String,
    /// Glyph used for the horizontal bottom edge.
    #[serde(default = "default_h_glyph")]
    pub bottom: String,
    /// Glyph used for the vertical left edge.
    #[serde(default = "default_v_glyph")]
    pub left: String,
    /// Glyph used for the vertical right edge.
    #[serde(default = "default_v_glyph")]
    pub right: String,
    /// Glyph used for the top-left corner.
    #[serde(default = "default_tl_glyph")]
    pub top_left: String,
    /// Glyph used for the top-right corner.
    #[serde(default = "default_tr_glyph")]
    pub top_right: String,
    /// Glyph used for the bottom-left corner.
    #[serde(default = "default_bl_glyph")]
    pub bottom_left: String,
    /// Glyph used for the bottom-right corner.
    #[serde(default = "default_br_glyph")]
    pub bottom_right: String,
}

fn default_h_glyph() -> String {
    "\u{2500}".to_string()
}
fn default_v_glyph() -> String {
    "\u{2502}".to_string()
}
// Light-preset corners (┌┐└┘). Match the new `default_border_preset`
// so a `preset=custom` payload that omits a corner falls back to
// the same shape the surrounding sides connect with.
fn default_tl_glyph() -> String {
    "\u{250C}".to_string()
}
fn default_tr_glyph() -> String {
    "\u{2510}".to_string()
}
fn default_bl_glyph() -> String {
    "\u{2514}".to_string()
}
fn default_br_glyph() -> String {
    "\u{2518}".to_string()
}

/// Descriptor for how this node arranges its children — a
/// miMind-compat record carried through for round-trip fidelity.
/// Mandala does not currently drive layout from these fields;
/// custom mutations (see `format/mutations.md`) are the active
/// layout mechanism. Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeLayout {
    /// Layout-algorithm name from the miMind format — round-tripped
    /// but not currently honoured by the renderer.
    #[serde(rename = "type")]
    pub layout_type: String,
    /// Growth direction hint carried through from miMind.
    pub direction: String,
    /// Inter-child spacing hint carried through from miMind.
    pub spacing: f64,
}

/// Links a node to one entry in a named [`super::Palette`] keyed by
/// depth. `level` is the index into the palette's `groups`; clamped
/// at theme-resolve time (`resolve_theme_colors`) so a schema
/// referencing a level beyond the palette's length falls back to
/// the last group rather than erroring.
/// `starts_at_root` and `connections_colored` are round-tripped
/// miMind-compat flags that the renderer interprets when resolving
/// effective colors. Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorSchema {
    /// Named palette to bind this node's colours to — keys into
    /// [`super::MindMap::palettes`].
    pub palette: String,
    /// Index into the palette's `groups` for this node's depth.
    /// Clamped against the palette's length at resolve time so a
    /// level past the end falls back to the last group.
    pub level: i32,
    /// `true` when depth indexing begins at the root (level 0 is the
    /// root itself); `false` shifts the indexing so the root is
    /// transparent and children start at level 0.
    pub starts_at_root: bool,
    /// When `true`, outgoing connections inherit the palette's
    /// colour at this node's level instead of the edge's own colour.
    pub connections_colored: bool,
}

/// One palette entry — the four colors a themed node inherits at a
/// given depth level. Referenced from [`ColorSchema::level`] via
/// [`super::Palette::groups`]. Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorGroup {
    /// Background-fill colour for a node at this level.
    pub background: String,
    /// Frame-stroke colour for a node at this level.
    pub frame: String,
    /// Text colour for a node at this level.
    pub text: String,
    /// First-line / title colour — overrides `text` for the first
    /// line of a node's text when present.
    pub title: String,
}
