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
use crate::util::color::FloatRgba;
use crate::mindmap::border_pattern::SidePattern;
use crate::mindmap::model::{ColorGroup, CustomBorderGlyphs, GlyphBorderConfig};

/// Fraction of `font_size` by which a border's top/bottom runs
/// are pulled inward so their glyph visible extents overlap with
/// the vertical columns. Empirically chosen for LiberationSans at
/// typical border font sizes; larger values visibly encroach on
/// the node content, smaller values leave gaps. Shared by the
/// renderer and `tree_builder::build_border_tree` so the two
/// paths can't drift.
pub const BORDER_CORNER_OVERLAP_FRAC: f32 = 0.35;

/// Multiplier estimating the advance of one border glyph as a
/// fraction of `font_size`. `0.6` matches LiberationSans box-
/// drawing characters at typical border sizes; both the renderer's
/// keyed border-buffer rebuild and the border-tree builder rely on
/// this to position corner glyphs consistently.
pub const BORDER_APPROX_CHAR_WIDTH_FRAC: f32 = 0.6;

/// Defines which glyphs to use for rendering a node's border.
/// Each field is a single character (glyph) from the selected font.
/// The border is rendered as positioned text elements around the node content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BorderGlyphSet {
    pub top: char,
    pub bottom: char,
    pub left: char,
    pub right: char,
    pub top_left: char,
    pub top_right: char,
    pub bottom_left: char,
    pub bottom_right: char,
}

impl BorderGlyphSet {
    /// Standard Unicode box-drawing characters (light lines)
    pub fn box_drawing_light() -> Self {
        BorderGlyphSet {
            top: '\u{2500}',        // ─
            bottom: '\u{2500}',     // ─
            left: '\u{2502}',       // │
            right: '\u{2502}',      // │
            top_left: '\u{250C}',   // ┌
            top_right: '\u{2510}',  // ┐
            bottom_left: '\u{2514}',// └
            bottom_right: '\u{2518}',// ┘
        }
    }

    /// Heavy box-drawing characters
    pub fn box_drawing_heavy() -> Self {
        BorderGlyphSet {
            top: '\u{2501}',        // ━
            bottom: '\u{2501}',     // ━
            left: '\u{2503}',       // ┃
            right: '\u{2503}',      // ┃
            top_left: '\u{250F}',   // ┏
            top_right: '\u{2513}',  // ┓
            bottom_left: '\u{2517}',// ┗
            bottom_right: '\u{251B}',// ┛
        }
    }

    /// Double-line box-drawing characters
    pub fn box_drawing_double() -> Self {
        BorderGlyphSet {
            top: '\u{2550}',        // ═
            bottom: '\u{2550}',     // ═
            left: '\u{2551}',       // ║
            right: '\u{2551}',      // ║
            top_left: '\u{2554}',   // ╔
            top_right: '\u{2557}',  // ╗
            bottom_left: '\u{255A}',// ╚
            bottom_right: '\u{255D}',// ╝
        }
    }

    /// Rounded box-drawing characters
    pub fn box_drawing_rounded() -> Self {
        BorderGlyphSet {
            top: '\u{2500}',        // ─
            bottom: '\u{2500}',     // ─
            left: '\u{2502}',       // │
            right: '\u{2502}',      // │
            top_left: '\u{256D}',   // ╭
            top_right: '\u{256E}',  // ╮
            bottom_left: '\u{2570}',// ╰
            bottom_right: '\u{256F}',// ╯
        }
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
/// after preset / config / fallback resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BorderCorners {
    pub top_left: String,
    pub top_right: String,
    pub bottom_left: String,
    pub bottom_right: String,
}

/// Parsed [`SidePattern`] for each of the four sides — what the
/// renderer fits between the corners. Populated by
/// [`resolve_border_style`] from the per-node config or the
/// preset's defaults.
#[derive(Clone, Debug)]
pub struct SidePatternQuad {
    pub top: SidePattern,
    pub bottom: SidePattern,
    pub left: SidePattern,
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
    Frame,
    Background,
    Text,
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
    /// [`resolve_border_style`]; defaults to the rounded preset's
    /// corners as single-cluster strings.
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
    /// (`#RRGGBB` hex or resolved theme variable). Uses the rounded
    /// box-drawing preset + 14 pt default font size — matches the
    /// scene builder's per-framed-node default so node borders keep
    /// the same look when a caller asks for "a default border in this
    /// color" instead of building one field-by-field.
    pub fn default_with_color(color: &str) -> Self {
        let glyph_set = BorderGlyphSet::box_drawing_rounded();
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

/// Per-corner cluster counts, used by the auto-resize pass to
/// reason about minimum widths in the same cluster units the
/// pattern fitter speaks in.
#[derive(Clone, Copy, Debug)]
pub struct CornerClusterCounts {
    pub top_left: usize,
    pub top_right: usize,
    pub bottom_left: usize,
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
/// 3. Hardcoded preset / font / size defaults (rounded, system,
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
    // `default_border_preset` literal "rounded"), the cascade
    // can't tell the difference between "author chose rounded"
    // and "author left the field unset". We accept that: the
    // canvas default only contributes when the per-node cfg is
    // absent entirely, mirroring the existing `style.frame_color`
    // cascade.
    let chosen = cfg.or(canvas_default);

    let preset_name = chosen
        .map(|c| c.preset.as_str())
        .unwrap_or("rounded");
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
/// Unknown preset names fall back to `rounded` and log a warning;
/// the `"custom"` preset returns `rounded` here too — its corners /
/// sides are overridden downstream by the `glyphs` payload.
pub fn preset_glyph_set(preset: &str) -> BorderGlyphSet {
    match preset.to_ascii_lowercase().as_str() {
        "light" => BorderGlyphSet::box_drawing_light(),
        "heavy" => BorderGlyphSet::box_drawing_heavy(),
        "double" => BorderGlyphSet::box_drawing_double(),
        "rounded" | "custom" => BorderGlyphSet::box_drawing_rounded(),
        other => {
            log::warn!(
                "border preset '{}' unknown; using 'rounded'",
                other
            );
            BorderGlyphSet::box_drawing_rounded()
        }
    }
}

/// The five preset names accepted by the schema's
/// `GlyphBorderConfig.preset` field. Surfaced for the console's
/// `border preset=` completion.
pub const BORDER_PRESETS: &[&str] =
    &["light", "heavy", "double", "rounded", "custom"];

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
    CustomBorderGlyphs {
        top: "\u{2500}".to_string(),
        bottom: "\u{2500}".to_string(),
        left: "\u{2502}".to_string(),
        right: "\u{2502}".to_string(),
        top_left: "\u{256D}".to_string(),
        top_right: "\u{256E}".to_string(),
        bottom_left: "\u{2570}".to_string(),
        bottom_right: "\u{256F}".to_string(),
    }
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
        // Default preset is rounded.
        assert_eq!(
            style.glyph_set.top_left,
            BorderGlyphSet::box_drawing_rounded().top_left
        );
        assert_eq!(style.font_name, None);
    }
}
