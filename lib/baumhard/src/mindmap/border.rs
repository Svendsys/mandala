// SPDX-License-Identifier: MPL-2.0

//! Per-node border rendering vocabulary — the `GlyphBorder*` config
//! structs the loader deserializes and the geometry constants the
//! renderer and `tree_builder::build_border_tree` share to keep
//! border layout consistent across the two paths. Borders are the
//! glyph-drawn rectangles around framed nodes; portal labels, edge
//! handles, and drag previews all attach to these geometry hints.

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::mindmap::border_pattern::SidePattern;
use crate::mindmap::model::{ColorGroup, CustomBorderGlyphs, GlyphBorderConfig};
use crate::util::color::FloatRgba;

/// Fraction of `font_size` by which a border's top/bottom runs
/// are pulled inward so their glyph visible extents overlap with
/// the vertical columns. Empirically chosen for LiberationSans at
/// typical border font sizes. Shared by the renderer and
/// `tree_builder::build_border_tree` so the two paths can't drift.
/// Per-face calibration uses
/// [`crate::font::fonts::measure_glyph_ink_bounds`] when the chosen
/// border face differs from LiberationSans.
pub const BORDER_CORNER_OVERLAP_FRAC: f32 = 0.35;

/// Multiplier estimating one border-glyph advance as a fraction of
/// `font_size`. `0.6` matches LiberationSans box-drawing
/// characters; both the renderer's keyed border-buffer rebuild and
/// the border-tree builder consult this for corner positioning.
/// Per-face calibration measures the actual `─` advance via
/// cosmic-text shaping when the border face differs.
pub const BORDER_APPROX_CHAR_WIDTH_FRAC: f32 = 0.6;

/// Defines which glyphs to use for rendering a node's border.
/// Each field is a single character (glyph) from the selected font.
/// The border is rendered as positioned text elements around the node content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BorderGlyphSet {
    /// Horizontal fill glyph used along the top edge between corners.
    pub top: char,
    /// Horizontal fill glyph used along the bottom edge between corners.
    pub bottom: char,
    /// Vertical fill glyph used along the left edge between corners.
    pub left: char,
    /// Vertical fill glyph used along the right edge between corners.
    pub right: char,
    /// Single-character glyph at the top-left corner.
    pub top_left: char,
    /// Single-character glyph at the top-right corner.
    pub top_right: char,
    /// Single-character glyph at the bottom-left corner.
    pub bottom_left: char,
    /// Single-character glyph at the bottom-right corner.
    pub bottom_right: char,
}

/// Single source of truth for the four canonical Unicode
/// box-drawing presets. Each row is `(name, [top, bottom, left,
/// right, tl, tr, bl, br])`. Adding a fifth preset is a one-row
/// extension here, plus an entry in [`BORDER_PRESETS`] (which the
/// console's `border preset=` completion surfaces).
const PRESET_TABLE: &[(&str, [char; 8])] = &[
    ("light",   ['─', '─', '│', '│', '┌', '┐', '└', '┘']),
    ("heavy",   ['━', '━', '┃', '┃', '┏', '┓', '┗', '┛']),
    ("double", ['═', '═', '║', '║', '╔', '╗', '╚', '╝']),
    ("rounded", ['─', '─', '│', '│', '╭', '╮', '╰', '╯']),
];

impl BorderGlyphSet {
    /// Build a glyph set from the 8-char layout `[top, bottom,
    /// left, right, tl, tr, bl, br]` used by [`PRESET_TABLE`].
    fn from_glyphs(g: [char; 8]) -> Self {
        BorderGlyphSet {
            top: g[0],
            bottom: g[1],
            left: g[2],
            right: g[3],
            top_left: g[4],
            top_right: g[5],
            bottom_left: g[6],
            bottom_right: g[7],
        }
    }

    /// Standard Unicode box-drawing characters (light lines).
    pub fn box_drawing_light() -> Self {
        Self::from_glyphs(PRESET_TABLE[0].1)
    }

    /// Heavy box-drawing characters.
    pub fn box_drawing_heavy() -> Self {
        Self::from_glyphs(PRESET_TABLE[1].1)
    }

    /// Double-line box-drawing characters.
    pub fn box_drawing_double() -> Self {
        Self::from_glyphs(PRESET_TABLE[2].1)
    }

    /// Rounded box-drawing characters.
    pub fn box_drawing_rounded() -> Self {
        Self::from_glyphs(PRESET_TABLE[3].1)
    }

    /// Generates the top border string for a given width in characters.
    pub fn top_border(&self, char_width: usize) -> String {
        if char_width < 2 {
            return String::new();
        }
        let mut s = String::with_capacity(char_width);
        s.push(self.top_left);
        for _ in 0..char_width.saturating_sub(2) {
            s.push(self.top);
        }
        s.push(self.top_right);
        s
    }

    /// Generates the bottom border string for a given width in characters.
    pub fn bottom_border(&self, char_width: usize) -> String {
        if char_width < 2 {
            return String::new();
        }
        let mut s = String::with_capacity(char_width);
        s.push(self.bottom_left);
        for _ in 0..char_width.saturating_sub(2) {
            s.push(self.bottom);
        }
        s.push(self.bottom_right);
        s
    }

    /// Generates a left side character (repeated for each row).
    pub fn left_char(&self) -> char {
        self.left
    }

    /// Generates a right side character (repeated for each row).
    pub fn right_char(&self) -> char {
        self.right
    }

    /// Generates a vertical side column of `rows` rows, using
    /// `self.left` as the glyph. Rows are separated by `'\n'`, and
    /// the returned string ends without a trailing newline — one
    /// glyph per line cell, `rows` lines total.
    ///
    /// Callers that want the right side can either use this same
    /// string (since the rounded/light presets have `left == right`)
    /// or call `right_side_border` below for an explicit right column.
    ///
    /// Cost: O(rows) push operations, one allocation sized to
    /// `rows * (left.len_utf8() + 1)`.
    pub fn side_border(&self, rows: usize) -> String {
        build_side_column(self.left, rows)
    }

    /// Like [`Self::side_border`] but uses `self.right`. Presets where
    /// `left == right` can call either — this exists so callers
    /// never need to know which preset they have.
    pub fn right_side_border(&self, rows: usize) -> String {
        build_side_column(self.right, rows)
    }
}

fn build_side_column(glyph: char, rows: usize) -> String {
    if rows == 0 {
        return String::new();
    }
    let glyph_len = glyph.len_utf8();
    let mut s = String::with_capacity(rows * (glyph_len + 1) - 1);
    for i in 0..rows {
        s.push(glyph);
        if i + 1 < rows {
            s.push('\n');
        }
    }
    s
}

/// Runtime form of the four border corners as
/// grapheme-cluster-ready strings. The data model
/// ([`CustomBorderGlyphs`]) already stores corners as `String`, so
/// a user-supplied multi-cluster corner like `tl = "<<"` survives
/// serialization round-trip; this type is its render-time shape,
/// populated by [`resolve_border_style`] from either a
/// `CustomBorderGlyphs` payload (when the preset is `"custom"`)
/// or the chosen preset's single-char defaults.
///
/// Each field is one or more grapheme clusters. Empty strings are
/// allowed by the type but actively normalised to the preset
/// fallback during resolution, so the renderer never receives a
/// corner with zero glyphs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BorderCorners {
    /// Glyphs at the top-left corner of the rectangle.
    pub top_left: String,
    /// Glyphs at the top-right corner of the rectangle.
    pub top_right: String,
    /// Glyphs at the bottom-left corner of the rectangle.
    pub bottom_left: String,
    /// Glyphs at the bottom-right corner of the rectangle.
    pub bottom_right: String,
}

/// Parsed [`SidePattern`] for each of the four sides — what the
/// renderer fits between the corners. Populated by
/// [`resolve_border_style`] from the per-node config or the
/// preset's defaults.
#[derive(Clone, Debug)]
pub struct SidePatternQuad {
    /// Pattern fitted between the top corners.
    pub top: SidePattern,
    /// Pattern fitted between the bottom corners.
    pub bottom: SidePattern,
    /// Pattern repeated down the left column.
    pub left: SidePattern,
    /// Pattern repeated down the right column.
    pub right: SidePattern,
}

/// Which `ColorGroup` channel the border cycles through when a
/// `color_palette` is bound. Defaults to [`PaletteField::Frame`]
/// because frame is the channel whose meaning matches the border
/// today (`resolve_theme_colors` writes the same field into the
/// resolved colours). Open seam — adding a new variant here is a
/// localised change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteField {
    /// Cycle the border across each `ColorGroup`'s `frame`
    /// channel — the historical default and the channel whose
    /// meaning matches the border itself.
    Frame,
    /// Cycle across each group's `background` channel — useful
    /// when the border should track the node fill rather than the
    /// frame stroke.
    Background,
    /// Cycle across each group's `text` channel — for borders
    /// drawn in the same hue as the node label.
    Text,
    /// Cycle across each group's `title` channel — for borders
    /// drawn in the same hue as the node title.
    Title,
}

impl PaletteField {
    /// Parse the `color_palette_field` string from the data model.
    /// Unknown values warn and fall back to `Frame` so a typo
    /// degrades to "single colour" instead of dropping the whole
    /// border per `CODE_CONVENTIONS.md` §9.
    pub fn from_str_or_default(s: Option<&str>) -> Self {
        match s.map(str::to_ascii_lowercase).as_deref() {
            Some("frame") | None => PaletteField::Frame,
            Some("background") => PaletteField::Background,
            Some("text") => PaletteField::Text,
            Some("title") => PaletteField::Title,
            Some(other) => {
                log::warn!(
                    "border color_palette_field '{}' unknown; using 'frame'",
                    other
                );
                PaletteField::Frame
            }
        }
    }

    /// One-word readable label (`"frame"`, `"background"`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            PaletteField::Frame => "frame",
            PaletteField::Background => "background",
            PaletteField::Text => "text",
            PaletteField::Title => "title",
        }
    }

    /// Read the channel value out of one [`ColorGroup`].
    pub fn read<'a>(self, group: &'a ColorGroup) -> &'a str {
        match self {
            PaletteField::Frame => &group.frame,
            PaletteField::Background => &group.background,
            PaletteField::Text => &group.text,
            PaletteField::Title => &group.title,
        }
    }

    /// Static list of recognised values (used by the console
    /// command's completion).
    pub const ALL: &'static [&'static str] =
        &["frame", "background", "text", "title"];
}

/// Configuration for how a node's border should be rendered.
/// This struct is intended to be attached per-node or as a global default,
/// and is the key extensibility point for the editing experience.
#[derive(Debug, Clone)]
pub struct BorderStyle {
    /// Legacy single-character glyph set — kept on the type so
    /// callers that only need the simple box-drawing presets
    /// (e.g. the console overlay frame in
    /// `src/application/renderer/console_geometry.rs`) can keep
    /// using `top_border` / `side_border` etc. unchanged.
    pub glyph_set: BorderGlyphSet,
    /// Multi-cluster runtime corners. Populated by
    /// [`resolve_border_style`]; defaults to the light preset's
    /// corners (`┌┐└┘`) as single-cluster strings.
    pub corners: BorderCorners,
    /// Parsed [`SidePattern`] for each side. Populated by
    /// [`resolve_border_style`] from the user's
    /// `CustomBorderGlyphs` input or from preset defaults.
    pub side_patterns: SidePatternQuad,
    /// Optional palette name to cycle per-glyph colours from.
    /// Renderer-time lookup happens in [`build_border_regions`].
    pub color_palette: Option<String>,
    /// Which channel of each `ColorGroup` is cycled when
    /// `color_palette` is bound.
    pub palette_field: PaletteField,
    /// The font to use for border glyphs. None means use default system font.
    pub font_name: Option<String>,
    /// Font size for border glyphs in points.
    pub font_size_pt: f32,
    /// Border color as #RRGGBB hex string. Used as the fallback
    /// when `color_palette` is unset or fails to resolve.
    pub color: String,
    /// Whether to render this border at all.
    pub visible: bool,
}

impl BorderStyle {
    /// Construct a default visible border with the given color
    /// (`#RRGGBB` hex or resolved theme variable). Uses the light
    /// box-drawing preset + 14 pt default font size — matches the
    /// scene builder's per-framed-node default so node borders keep
    /// the same look when a caller asks for "a default border in this
    /// color" instead of building one field-by-field.
    ///
    /// `light` (`┌─│┘`) is the chosen default rather than `rounded`
    /// (`╭─│╯`) because the rounded corners curve inward away from
    /// the cell edges, leaving a visible gap where corner meets
    /// side in any monospace face. The light preset's corners
    /// extend to the cell edges, so corner and side connect cleanly.
    pub fn default_with_color(color: &str) -> Self {
        let glyph_set = BorderGlyphSet::box_drawing_light();
        BorderStyle {
            corners: glyph_set.corners(),
            side_patterns: glyph_set.side_patterns(),
            glyph_set,
            color_palette: None,
            palette_field: PaletteField::Frame,
            font_name: None,
            font_size_pt: 14.0,
            color: color.to_string(),
            visible: true,
        }
    }

    /// Concatenated full top-edge text for `cluster_width` cluster
    /// columns: `top_left + top_pattern + top_right`. Cluster math
    /// trims the side fill so the corners fit.
    pub fn top_text(&self, cluster_width: usize) -> String {
        build_horizontal_text(
            &self.corners.top_left,
            &self.corners.top_right,
            &self.side_patterns.top,
            cluster_width,
        )
    }

    /// Concatenated full bottom-edge text for `cluster_width`
    /// cluster columns.
    pub fn bottom_text(&self, cluster_width: usize) -> String {
        build_horizontal_text(
            &self.corners.bottom_left,
            &self.corners.bottom_right,
            &self.side_patterns.bottom,
            cluster_width,
        )
    }

    /// Vertical column for the left side at `rows` rows. Each
    /// rendered cluster occupies one line; clusters are separated
    /// by `'\n'` and the last cluster has no trailing newline,
    /// mirroring the legacy `BorderGlyphSet::side_border` shape.
    pub fn left_column_text(&self, rows: usize) -> String {
        build_vertical_text(&self.side_patterns.left, rows)
    }

    /// Vertical column for the right side at `rows` rows.
    pub fn right_column_text(&self, rows: usize) -> String {
        build_vertical_text(&self.side_patterns.right, rows)
    }

    /// Cluster count of each corner — handed to the fitter and
    /// the auto-resize pass so they speak in the same units.
    pub fn corner_clusters(&self) -> CornerClusterCounts {
        CornerClusterCounts {
            top_left: count_clusters(&self.corners.top_left),
            top_right: count_clusters(&self.corners.top_right),
            bottom_left: count_clusters(&self.corners.bottom_left),
            bottom_right: count_clusters(&self.corners.bottom_right),
        }
    }
}

/// Per-side run geometry the three border-emit pipelines (the
/// in-place mutator path, the initial-build tree path, and the
/// flat-pipeline `rebuild_border_buffers_keyed` in the renderer)
/// each previously open-coded with byte-identical math.
///
/// One spec describes one side (top / bottom / left / right):
/// where the run sits in canvas space, how big its text bounds
/// are, what glyph string it carries, what palette offset to
/// hand to [`build_border_regions`], and the pre-counted
/// grapheme cluster count so consumers don't re-walk the string.
///
/// Pure data — no allocation beyond the `String` text. Consumers
/// translate the spec into their pipeline-specific output:
/// the tree path wraps it into a [`crate::gfx_structs::area::GlyphArea`];
/// the renderer's flat path shapes it into a `cosmic_text::Buffer`.
/// Color, palette cycle, and zoom-visibility belong with the
/// consumer (those are policy, not geometry).
#[derive(Clone, Debug, PartialEq)]
pub struct BorderRunSpec {
    /// 1=top, 2=bottom, 3=left, 4=right. Stable across rebuilds —
    /// the in-place mutator path keys leaves on this channel.
    pub channel: usize,
    /// Concatenated glyph string for this run (corners + side fill
    /// for horizontals, vertical column for verticals).
    pub text: String,
    /// Font size in pt; identical for all 4 sides (sourced from
    /// the [`BorderStyle::font_size_pt`]).
    pub font_size_pt: f32,
    /// Top-left position of the run's text bounds in canvas space.
    pub position: (f32, f32),
    /// Width / height of the run's text bounds.
    pub bounds: (f32, f32),
    /// Glyph-index offset into the per-cycle palette so a palette-
    /// cycling border sweeps continuously around the rectangle in
    /// top → right → bottom → left order. Zero when the upstream
    /// palette is empty (single-colour border).
    pub palette_offset: usize,
    /// Pre-computed `count_grapheme_clusters(text)`. Carried on
    /// the spec so consumers handing it to [`build_border_regions`]
    /// don't re-walk the string.
    pub cluster_count: usize,
}

/// Compute the four-side run geometry for one node's border.
/// Single source of truth for the per-side `(text, position,
/// bounds, palette_offset)` arithmetic that the in-place mutator
/// path, the initial-build tree path, and the flat-pipeline
/// `rebuild_border_buffers_keyed` previously reproduced
/// independently.
///
/// Channels:
/// - `1` = top, `2` = bottom, `3` = left, `4` = right.
///
/// Palette offsets (for a continuous top→right→bottom→left
/// sweep) are `[0, top_clusters + right_clusters,
/// top_clusters + right_clusters + bottom_clusters,
/// top_clusters]`. Vertical text strings include `'\n'`
/// separators which the grapheme counter folds into one cluster
/// per visible glyph, so the indices line up with the per-cluster
/// regions [`build_border_regions`] emits.
///
/// Cost: 4 `String` allocations (one per side text), 4
/// `count_grapheme_clusters` walks. No font-system access, no
/// shaping. Pure: same inputs → same array.
pub fn border_run_specs(
    border_style: &BorderStyle,
    node_pos: (f32, f32),
    node_size: (f32, f32),
) -> [BorderRunSpec; 4] {
    let font_size = border_style.font_size_pt;
    let approx_char_width = font_size * BORDER_APPROX_CHAR_WIDTH_FRAC;
    let char_count = ((node_size.0 / approx_char_width) + 2.0).ceil().max(3.0) as usize;
    let right_corner_x =
        node_pos.0 - approx_char_width + (char_count - 1) as f32 * approx_char_width;
    let corner_overlap = font_size * BORDER_CORNER_OVERLAP_FRAC;
    let top_y = node_pos.1 - font_size + corner_overlap;
    let bottom_y = node_pos.1 + node_size.1 - corner_overlap;
    let h_width = (char_count as f32 + 1.0) * approx_char_width;
    let v_width = approx_char_width * 2.0;
    // `.ceil()` rather than `.round()` so the side columns always
    // extend at least as far down as the node bottom. With
    // `.round()`, a node whose `size_y / font_size` rounds down
    // (e.g. 100/14 = 7.14 → 7 rows = 98 px on a 100 px node)
    // leaves the last row 2 px short of the bottom row's corner
    // cell, which renders as a visible gap at BL/BR.
    let row_count = (node_size.1 / font_size).ceil().max(1.0) as usize;

    let top_text = border_style.top_text(char_count);
    let bottom_text = border_style.bottom_text(char_count);
    let left_text = border_style.left_column_text(row_count);
    let right_text = border_style.right_column_text(row_count);

    let top_clusters = count_clusters(&top_text);
    let right_clusters = count_clusters(&right_text);
    let bottom_clusters = count_clusters(&bottom_text);
    let left_clusters = count_clusters(&left_text);

    [
        BorderRunSpec {
            channel: 1,
            text: top_text,
            font_size_pt: font_size,
            position: (node_pos.0 - approx_char_width, top_y),
            bounds: (h_width, font_size * 1.5),
            palette_offset: 0,
            cluster_count: top_clusters,
        },
        BorderRunSpec {
            channel: 2,
            text: bottom_text,
            font_size_pt: font_size,
            position: (node_pos.0 - approx_char_width, bottom_y),
            bounds: (h_width, font_size * 1.5),
            palette_offset: top_clusters + right_clusters,
            cluster_count: bottom_clusters,
        },
        BorderRunSpec {
            channel: 3,
            text: left_text,
            font_size_pt: font_size,
            position: (node_pos.0 - approx_char_width, node_pos.1),
            bounds: (v_width, node_size.1),
            palette_offset: top_clusters + right_clusters + bottom_clusters,
            cluster_count: left_clusters,
        },
        BorderRunSpec {
            channel: 4,
            text: right_text,
            font_size_pt: font_size,
            position: (right_corner_x, node_pos.1),
            bounds: (v_width, node_size.1),
            palette_offset: top_clusters,
            cluster_count: right_clusters,
        },
    ]
}

/// Per-corner cluster counts, used by the auto-resize pass to
/// reason about minimum widths in the same cluster units the
/// pattern fitter speaks in.
#[derive(Clone, Copy, Debug)]
pub struct CornerClusterCounts {
    /// Grapheme-cluster count of the top-left corner string.
    pub top_left: usize,
    /// Grapheme-cluster count of the top-right corner string.
    pub top_right: usize,
    /// Grapheme-cluster count of the bottom-left corner string.
    pub bottom_left: usize,
    /// Grapheme-cluster count of the bottom-right corner string.
    pub bottom_right: usize,
}

impl CornerClusterCounts {
    /// Cluster width consumed by the top corners together.
    pub fn top_horizontal(self) -> usize {
        self.top_left + self.top_right
    }
    /// Cluster width consumed by the bottom corners together.
    pub fn bottom_horizontal(self) -> usize {
        self.bottom_left + self.bottom_right
    }
}

impl Default for BorderStyle {
    fn default() -> Self {
        Self::default_with_color("#ffffff")
    }
}

impl BorderGlyphSet {
    /// Multi-cluster runtime corners derived from this preset's
    /// single-char fields. Each corner is a one-cluster string.
    pub fn corners(&self) -> BorderCorners {
        BorderCorners {
            top_left: self.top_left.to_string(),
            top_right: self.top_right.to_string(),
            bottom_left: self.bottom_left.to_string(),
            bottom_right: self.bottom_right.to_string(),
        }
    }

    /// Side patterns for this preset — each side is the
    /// single-character glyph repeated atomically (one cluster
    /// per "iteration"). Always parses; the inputs are bare ASCII
    /// or single-codepoint scalars by construction.
    pub fn side_patterns(&self) -> SidePatternQuad {
        SidePatternQuad {
            top: parse_legacy_glyph(self.top),
            bottom: parse_legacy_glyph(self.bottom),
            left: parse_legacy_glyph(self.left),
            right: parse_legacy_glyph(self.right),
        }
    }
}

fn parse_legacy_glyph(c: char) -> SidePattern {
    // `SidePattern::parse` would treat '(' / ')' / '\\' specially.
    // The four legacy presets contain none of those, but a future
    // preset could; emit `AtomicRepeat { cluster: vec![c] }`
    // directly so the legacy path stays parse-free.
    SidePattern::AtomicRepeat {
        cluster: vec![c.to_string()],
    }
}

/// Resolve a node's effective `BorderStyle` from its optional
/// `GlyphBorderConfig`, the canvas-level default, and the resolved
/// frame colour. Single source of truth — every border-build path
/// (scene_builder, tree_builder, renderer) goes through this so
/// preset / font / size / color / pattern resolution can't drift
/// between pipelines.
///
/// Cascade for each field, most-specific wins:
/// 1. Per-node `GlyphBorderConfig` (the `cfg` arg).
/// 2. Canvas-level default (the `canvas_default` arg).
/// 3. Hardcoded preset / font / size defaults (light, system,
///    14 pt, the resolved `frame_color`).
///
/// Pattern parse errors on a configured side fall back to the
/// preset's side glyph for that side and emit a `log::warn!` —
/// per `CODE_CONVENTIONS.md` §9, interactive paths must not panic
/// and a corrupt model value should degrade to a sensible visual.
pub fn resolve_border_style(
    cfg: Option<&GlyphBorderConfig>,
    canvas_default: Option<&GlyphBorderConfig>,
    frame_color_resolved: &str,
) -> BorderStyle {
    // Field-by-field cascade. `cfg` takes precedence; if a key
    // sits at a meaningful default in `cfg` (e.g. the
    // `default_border_preset` literal "light"), the cascade
    // can't tell the difference between "author chose light"
    // and "author left the field unset". We accept that: the
    // canvas default only contributes when the per-node cfg is
    // absent entirely, mirroring the existing `style.frame_color`
    // cascade.
    let chosen = cfg.or(canvas_default);

    let preset_name = chosen
        .map(|c| c.preset.as_str())
        .unwrap_or("light");
    let glyph_set = preset_glyph_set(preset_name);

    // Multi-cluster corners + side patterns: prefer the cfg's
    // `glyphs` payload when the preset is `"custom"`; otherwise
    // fall back to the preset's single-glyph defaults.
    let (corners, side_patterns) =
        if preset_name.eq_ignore_ascii_case("custom") {
            let custom = chosen
                .and_then(|c| c.glyphs.as_ref())
                .cloned()
                .unwrap_or_else(default_custom_glyphs);
            corners_and_patterns_from_custom(&custom, &glyph_set)
        } else {
            (glyph_set.corners(), glyph_set.side_patterns())
        };

    let font_name = chosen.and_then(|c| c.font.clone());
    let font_size_pt = chosen
        .map(|c| c.font_size_pt)
        .unwrap_or(14.0);
    let color = chosen
        .and_then(|c| c.color.clone())
        .unwrap_or_else(|| frame_color_resolved.to_string());
    let color_palette = chosen.and_then(|c| c.color_palette.clone());
    let palette_field = PaletteField::from_str_or_default(
        chosen.and_then(|c| c.color_palette_field.as_deref()),
    );

    BorderStyle {
        glyph_set,
        corners,
        side_patterns,
        color_palette,
        palette_field,
        font_name,
        font_size_pt,
        color,
        visible: true,
    }
}

/// Pick the [`BorderGlyphSet`] for a preset name, case-insensitively.
/// Unknown preset names fall back to `light` and log a warning;
/// the `"custom"` preset returns `light` here too — its corners /
/// sides are overridden downstream by the `glyphs` payload, so the
/// fallback only shows through when the user picked custom but
/// supplied no glyph fields.
///
/// Reachable per tree rebuild (`tree_builder/border.rs:111`),
/// which §9 puts in interactive territory — no panic site. The
/// fallback is statically known to exist because [`PRESET_TABLE`]
/// is `const &[…; 4]` (non-empty by construction).
pub fn preset_glyph_set(preset: &str) -> BorderGlyphSet {
    let name = preset.to_ascii_lowercase();
    let row = PRESET_TABLE
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| {
            // "custom" is in `BORDER_PRESETS` but absent from the
            // glyph table — it signals "user-supplied glyphs
            // override these defaults," with the per-side fallback
            // to `light`. Anything else gets a warn-log.
            if name != CUSTOM_PRESET_NAME {
                log::warn!("border preset '{}' unknown; using 'light'", preset);
            }
            &PRESET_TABLE[0]
        });
    BorderGlyphSet::from_glyphs(row.1)
}

/// Sentinel preset name for "user-supplied glyphs override these
/// defaults." Has no row in [`PRESET_TABLE`]; the per-side
/// fallback in [`preset_glyph_set`] is `light`. Repeated literal
/// `"custom"` checks reach for this to keep the meaning single-
/// sourced.
pub const CUSTOM_PRESET_NAME: &str = "custom";

/// Every preset name accepted by the schema's
/// `GlyphBorderConfig.preset` field — the four typed glyph rows
/// in [`PRESET_TABLE`] plus the [`CUSTOM_PRESET_NAME`] sentinel.
/// Surfaced for the console's `border preset=` completion.
///
/// Derived from `PRESET_TABLE`'s row-index → name extraction at
/// const-fn time so adding a fifth glyph row to `PRESET_TABLE`
/// only requires extending the table — `BORDER_PRESETS` rebuilds
/// automatically.
pub const BORDER_PRESETS: &[&str] = &{
    const N: usize = PRESET_TABLE.len();
    let mut out: [&str; N + 1] = [""; N + 1];
    let mut i = 0;
    while i < N {
        out[i] = PRESET_TABLE[i].0;
        i += 1;
    }
    out[N] = CUSTOM_PRESET_NAME;
    out
};

fn corners_and_patterns_from_custom(
    g: &CustomBorderGlyphs,
    fallback: &BorderGlyphSet,
) -> (BorderCorners, SidePatternQuad) {
    let corners = BorderCorners {
        top_left: nonempty_or(&g.top_left, fallback.top_left),
        top_right: nonempty_or(&g.top_right, fallback.top_right),
        bottom_left: nonempty_or(&g.bottom_left, fallback.bottom_left),
        bottom_right: nonempty_or(&g.bottom_right, fallback.bottom_right),
    };
    let side_patterns = SidePatternQuad {
        top: parse_side_or_fallback(&g.top, fallback.top, "top"),
        bottom: parse_side_or_fallback(&g.bottom, fallback.bottom, "bottom"),
        left: parse_side_or_fallback(&g.left, fallback.left, "left"),
        right: parse_side_or_fallback(&g.right, fallback.right, "right"),
    };
    (corners, side_patterns)
}

fn nonempty_or(s: &str, fallback: char) -> String {
    if s.is_empty() {
        fallback.to_string()
    } else {
        s.to_string()
    }
}

fn parse_side_or_fallback(s: &str, fallback: char, side: &str) -> SidePattern {
    if s.is_empty() {
        return parse_legacy_glyph(fallback);
    }
    match SidePattern::parse(s) {
        Ok(p) => p,
        Err(e) => {
            log::warn!(
                "border {} pattern '{}' rejected ({}); falling back to preset",
                side, s, e
            );
            parse_legacy_glyph(fallback)
        }
    }
}

fn default_custom_glyphs() -> CustomBorderGlyphs {
    // Mirrors the `default_*_glyph` defaults on the data-model
    // type. Used when `preset = "custom"` but `glyphs` is absent.
    // Corners are the light preset's `┌┐└┘` rather than the
    // rounded `╭╮╰╯` so the fallback joins cleanly with the
    // side glyphs at cell boundaries.
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

/// Resolve `border_style.color_palette` (a name) to a list of
/// per-cycle-position RGBA colours, reading the configured
/// `palette_field` channel out of each `ColorGroup`. Returns an
/// empty `Vec` when the name is unset or the named palette is not
/// in the map (logs a warning in the latter case per
/// `CODE_CONVENTIONS.md` §9). Pre-resolution lets the renderer and
/// tree builder consume the colour list without re-walking the
/// palette every frame; the resolved cycle is stamped on
/// [`crate::mindmap::scene_builder::BorderElement::palette_cycle`]
/// and
/// [`crate::mindmap::tree_builder::BorderNodeData::palette_cycle`]
/// at scene-build time.
///
/// Cost: O(groups.len()) hex parses on names that resolve, O(1) on
/// the unset / missing fallback paths.
pub fn resolve_palette_cycle(
    palettes: &std::collections::HashMap<String, crate::mindmap::model::Palette>,
    border_style: &BorderStyle,
    fallback_rgba: FloatRgba,
) -> Vec<FloatRgba> {
    let Some(name) = border_style.color_palette.as_deref() else {
        return Vec::new();
    };
    let Some(palette) = palettes.get(name) else {
        log::warn!(
            "border color_palette '{}' not found in map; falling back to single colour",
            name
        );
        return Vec::new();
    };
    palette
        .groups
        .iter()
        .map(|g| {
            let hex = border_style.palette_field.read(g);
            crate::util::color::hex_to_rgba_safe(hex, fallback_rgba)
        })
        .collect()
}

/// Build a [`ColorFontRegions`] that paints `cluster_count` glyph
/// clusters. When `palette_cycle` is non-empty, each cluster
/// picks its colour from `palette_cycle[(offset + i) % len]`. When
/// it's empty, a single uniform region is emitted using
/// `fallback_rgba`.
///
/// `glyph_index_offset` lets callers chain side runs into one
/// continuous cycle around the rectangle (top → right → bottom →
/// left), so a colour sweep wraps cleanly across corners.
///
/// # Newlines in vertical sides
///
/// Vertical-side text is one cluster per line (`"|\n|\n|"` for a
/// 3-row column) — newline `'\n'` is its own grapheme cluster, so
/// `cluster_count` includes them and the palette index advances
/// across newlines too. The visible glyphs end up at palette
/// positions `[offset, offset+2, offset+4, …]` rather than
/// `[offset, offset+1, offset+2, …]`. This matches the tree
/// builder's per-side region emission, which means the flat-scene
/// renderer and the Baumhard-tree renderer paint identical colour
/// sequences. Callers that want a denser cycle on a column can
/// shorten the palette to compensate.
///
/// Cost: O(cluster_count) when palette-cycling, O(1) otherwise.
/// Per-cluster regions are only built when the caller opts in
/// (non-empty cycle) — the default single-region path matches
/// the existing renderer cost.
pub fn build_border_regions(
    cluster_count: usize,
    palette_cycle: &[FloatRgba],
    fallback_rgba: FloatRgba,
    glyph_index_offset: usize,
) -> ColorFontRegions {
    let mut regions = ColorFontRegions::new_empty();
    if cluster_count == 0 {
        return regions;
    }
    if palette_cycle.is_empty() {
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, cluster_count),
            None,
            Some(fallback_rgba),
        ));
        return regions;
    }
    for i in 0..cluster_count {
        let pos = (glyph_index_offset + i) % palette_cycle.len();
        regions.submit_region(ColorFontRegion::new(
            Range::new(i, i + 1),
            None,
            Some(palette_cycle[pos]),
        ));
    }
    regions
}

/// Concatenate corner + side fill + corner into one horizontal
/// border row. `cluster_width` is the row's total cluster width
/// (corners included); the side pattern fills the gap between
/// the corners.
fn build_horizontal_text(
    corner_left: &str,
    corner_right: &str,
    pattern: &SidePattern,
    cluster_width: usize,
) -> String {
    let cl = count_clusters(corner_left);
    let cr = count_clusters(corner_right);
    let between = cluster_width.saturating_sub(cl + cr);
    let rendered = pattern.render(between);
    let mut s = String::with_capacity(
        corner_left.len() + rendered.text.len() + corner_right.len(),
    );
    s.push_str(corner_left);
    s.push_str(&rendered.text);
    s.push_str(corner_right);
    s
}

/// Render a side pattern as a vertical column of `rows` rows.
/// Each cluster sits on its own line; lines are separated by
/// `'\n'` with no trailing newline, matching the existing
/// `BorderGlyphSet::side_border` shape consumed by the renderer.
fn build_vertical_text(pattern: &SidePattern, rows: usize) -> String {
    if rows == 0 {
        return String::new();
    }
    let rendered = pattern.render(rows);
    let mut s = String::with_capacity(rendered.text.len() + rows);
    let mut first = true;
    for g in rendered.text.graphemes(true) {
        if !first {
            s.push('\n');
        }
        s.push_str(g);
        first = false;
    }
    s
}

fn count_clusters(s: &str) -> usize {
    s.graphemes(true).count()
}

// -----------------------------------------------------------------
// Tests
//
// Border string generation is on every scene-rebuild hot path: one
// call to `top_border` / `bottom_border` per framed node, per frame.
// The loops look trivial today but are easy to break in ways that
// either quietly misalign corners or accidentally go quadratic. These
// tests double as perf regression guards.
// -----------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// `border_run_specs` produces four runs in the contractually
    /// required channel order (top=1, bottom=2, left=3, right=4)
    /// and assigns palette offsets that sweep continuously
    /// top→right→bottom→left. The invariant the three border
    /// pipelines (initial-build tree, in-place mutator tree,
    /// flat-pipeline scene_buffers) all rely on.
    #[test]
    fn border_run_specs_channels_and_palette_offsets() {
        let style = BorderStyle::default_with_color("#ffffff");
        let specs = border_run_specs(&style, (10.0, 20.0), (100.0, 50.0));
        assert_eq!(specs[0].channel, 1, "top channel");
        assert_eq!(specs[1].channel, 2, "bottom channel");
        assert_eq!(specs[2].channel, 3, "left channel");
        assert_eq!(specs[3].channel, 4, "right channel");
        // top offset is 0 (sweep starts here).
        assert_eq!(specs[0].palette_offset, 0);
        // right offset = top_clusters.
        assert_eq!(specs[3].palette_offset, specs[0].cluster_count);
        // bottom offset = top + right clusters.
        assert_eq!(
            specs[1].palette_offset,
            specs[0].cluster_count + specs[3].cluster_count
        );
        // left offset = top + right + bottom clusters.
        assert_eq!(
            specs[2].palette_offset,
            specs[0].cluster_count + specs[3].cluster_count + specs[1].cluster_count
        );
    }

    /// Each spec's `cluster_count` is consistent with
    /// `count_grapheme_clusters(text)` — the field exists so
    /// consumers handing the spec to `build_border_regions`
    /// don't re-walk the string, but the contract is that the
    /// pre-counted value matches a fresh count.
    #[test]
    fn border_run_specs_cluster_count_matches_text() {
        let style = BorderStyle::default_with_color("#ffffff");
        let specs = border_run_specs(&style, (0.0, 0.0), (200.0, 80.0));
        for spec in &specs {
            assert_eq!(
                spec.cluster_count,
                count_clusters(&spec.text),
                "spec channel {} cluster_count mismatch",
                spec.channel
            );
        }
    }

    /// `row_count` uses `.ceil()` not `.round()` so the side
    /// columns always extend to the node bottom — even when
    /// `node_size.1 / font_size` rounds down. The 100/14 case
    /// in the existing comment block at the spec is the
    /// canonical regression case.
    #[test]
    fn border_run_specs_uses_ceil_for_row_count() {
        let style = BorderStyle::default_with_color("#ffffff");
        // 100 / 14 = 7.14 — .round() = 7, .ceil() = 8. Verify
        // the left column carries 8 newline-separated lines (7
        // newlines + the last cluster), matching .ceil().
        let specs = border_run_specs(&style, (0.0, 0.0), (100.0, 100.0));
        let left_lines = specs[2].text.matches('\n').count() + 1;
        assert!(
            left_lines >= 8,
            "left column must use .ceil() (>= 8 rows for 100/14 = 7.14); got {}",
            left_lines
        );
    }

    /// The light preset's top border at width 5 is corners + 3 fill
    /// characters. Structural invariant: first char is `top_left`, last
    /// is `top_right`, all middle chars equal `top`.
    #[test]
    fn test_top_border_light_basic_shape() {
        let glyphs = BorderGlyphSet::box_drawing_light();
        let border = glyphs.top_border(5);
        assert_eq!(border, "\u{250C}\u{2500}\u{2500}\u{2500}\u{2510}");
        let chars: Vec<char> = border.chars().collect();
        assert_eq!(chars.len(), 5);
        assert_eq!(chars[0], glyphs.top_left);
        assert_eq!(chars[4], glyphs.top_right);
        for c in &chars[1..4] {
            assert_eq!(*c, glyphs.top);
        }
    }

    /// Widths below 2 have no room for both corners, so the function
    /// returns an empty string. Guards the early-return branch.
    #[test]
    fn test_top_border_width_under_two_is_empty() {
        let glyphs = BorderGlyphSet::box_drawing_light();
        assert_eq!(glyphs.top_border(0), "");
        assert_eq!(glyphs.top_border(1), "");
        assert_eq!(glyphs.bottom_border(0), "");
        assert_eq!(glyphs.bottom_border(1), "");
    }

    /// The bottom border must use the `bottom_*` corners, not the
    /// `top_*` corners. Copy-paste slip guard.
    #[test]
    fn test_bottom_border_uses_bottom_corners() {
        let glyphs = BorderGlyphSet::box_drawing_light();
        let border = glyphs.bottom_border(4);
        let chars: Vec<char> = border.chars().collect();
        assert_eq!(chars.len(), 4);
        assert_eq!(chars[0], glyphs.bottom_left);
        assert_eq!(chars[3], glyphs.bottom_right);
        assert_ne!(chars[0], glyphs.top_left);
        assert_ne!(chars[3], glyphs.top_right);
    }

    /// Every preset must produce a length-N string for width N ≥ 2 on
    /// both top and bottom. Catches a preset accidentally missing a
    /// glyph field (serde would default it to `'\0'`, which would still
    /// produce a length-N string — so also spot-check the first char is
    /// non-null).
    #[test]
    fn test_all_four_presets_produce_non_empty_borders() {
        let presets = [
            BorderGlyphSet::box_drawing_light(),
            BorderGlyphSet::box_drawing_heavy(),
            BorderGlyphSet::box_drawing_double(),
            BorderGlyphSet::box_drawing_rounded(),
        ];
        for glyphs in &presets {
            let top = glyphs.top_border(6);
            let bottom = glyphs.bottom_border(6);
            assert_eq!(top.chars().count(), 6);
            assert_eq!(bottom.chars().count(), 6);
            assert_ne!(top.chars().next().unwrap(), '\0');
            assert_ne!(bottom.chars().next().unwrap(), '\0');
            assert_ne!(glyphs.left_char(), '\0');
            assert_ne!(glyphs.right_char(), '\0');
        }
    }

    /// `top_border(10_000)` must succeed without panic and produce
    /// exactly 10,000 characters. Guards against accidental integer
    /// overflow on `char_width.saturating_sub(2)` or a quadratic
    /// string-growth refactor.
    #[test]
    fn test_top_border_large_width_no_panic() {
        let glyphs = BorderGlyphSet::box_drawing_light();
        let border = glyphs.top_border(10_000);
        assert_eq!(border.chars().count(), 10_000);
        // First and last are still corners, not middle fill.
        let chars: Vec<char> = border.chars().collect();
        assert_eq!(chars[0], glyphs.top_left);
        assert_eq!(chars[9_999], glyphs.top_right);
    }

    /// `side_border(rows)` emits exactly `rows` glyphs separated by
    /// newlines — one glyph per logical row. Guards against an
    /// off-by-one on the trailing newline.
    #[test]
    fn test_side_border_exact_row_count() {
        let glyphs = BorderGlyphSet::box_drawing_rounded();
        assert_eq!(glyphs.side_border(0), "");
        assert_eq!(glyphs.side_border(1), "│");
        assert_eq!(glyphs.side_border(3), "│\n│\n│");
        // Each of the 3 rows is exactly the `left` char, no more.
        let border = glyphs.side_border(5);
        assert_eq!(border.lines().count(), 5);
        for line in border.lines() {
            assert_eq!(line.chars().count(), 1);
            assert_eq!(line.chars().next().unwrap(), glyphs.left);
        }
    }

    /// Right-side helper uses `self.right`; for the rounded preset
    /// that's the same as `left`, but the API keeps them distinct so
    /// callers don't have to know.
    #[test]
    fn test_right_side_border_uses_right_glyph() {
        let glyphs = BorderGlyphSet::box_drawing_rounded();
        let border = glyphs.right_side_border(4);
        for line in border.lines() {
            assert_eq!(line.chars().next().unwrap(), glyphs.right);
        }
    }

    /// `BorderStyle::default_with_color` is what the scene builder
    /// constructs for every framed node. Spot-check its fields.
    #[test]
    fn test_border_style_default_with_color() {
        let style = BorderStyle::default_with_color("#ff0000");
        assert_eq!(style.color, "#ff0000");
        assert!(style.visible);
        // Default preset is light — its corners extend to the cell
        // edges so they connect cleanly with the side glyphs.
        assert_eq!(
            style.glyph_set.top_left,
            BorderGlyphSet::box_drawing_light().top_left
        );
        assert_eq!(style.font_name, None);
    }

    /// `resolve_border_style(None, None, ...)` is the most common
    /// path: a framed node with no per-node `GlyphBorderConfig` and
    /// a canvas with no `default_border` falls all the way through
    /// the cascade to the hardcoded preset / font / size defaults.
    /// Pin that the corners and side patterns land on the light
    /// preset so a future flip of the default doesn't silently
    /// change the rendered look for every map that lacks an
    /// explicit border config.
    #[test]
    fn resolve_border_style_with_no_overrides_uses_light_preset() {
        let style = resolve_border_style(None, None, "#abcdef");
        let expected = BorderGlyphSet::box_drawing_light();
        assert_eq!(style.corners.top_left, expected.top_left.to_string());
        assert_eq!(style.corners.top_right, expected.top_right.to_string());
        assert_eq!(style.corners.bottom_left, expected.bottom_left.to_string());
        assert_eq!(style.corners.bottom_right, expected.bottom_right.to_string());
        assert_eq!(style.color, "#abcdef");
        assert_eq!(style.font_size_pt, 14.0);
        assert!(style.visible);
    }
}
