// SPDX-License-Identifier: MPL-2.0

//! `ColorFontRegions` ↔ cosmic-text styling bridges.
//!
//! Two shapes, one resolver — bridges baumhard's `ColorFontRegions`
//! (the model-level representation of styled text runs) into either
//! cosmic-text API shape:
//!
//! - [`attrs_list_from_regions`] returns an `AttrsList` for
//!   callers using `Editor::insert_string`.
//! - [`RegionFamilies`] + [`rich_text_spans_from_regions`] returns
//!   a `Vec<(&str, Attrs)>` for callers using `Buffer::set_rich_text`
//!   (the renderer's tree walker).
//!
//! Both honour the per-region color, font pin, and (for the spans
//! API) grapheme-aware byte slicing. The shared private
//! `resolve_font_family` keeps the lookup + fallback discipline in
//! one place — see `CODE_CONVENTIONS.md` §1 and
//! `lib/baumhard/CONVENTIONS.md` §B5.

use cosmic_text::{Attrs, AttrsList, Color, Family, FontSystem, Metrics, Style};
use log::warn;

use crate::core::primitives::ColorFontRegions;
use crate::font::fonts::COMPILED_FONT_ID_MAP;
use crate::util::color::{convert_f32_to_u8, FloatRgba};
use crate::util::grapheme_chad;

/// Build a cosmic-text `AttrsList` from a `ColorFontRegions` source.
///
/// One span is emitted per region. A region with `color = Some(rgba)`
/// gets that color; otherwise the span uses cosmic-text's default. A
/// region with `font = Some(id)` resolves to that font family; an
/// unknown or unresolvable font id falls back to `Family::Monospace`
/// with a warning — this function runs inside the renderer's frame
/// loop and a corrupt save must not abort it.
///
/// Cost: O(n_regions) iteration plus one `font_system.db().face()`
/// lookup per region with a font id. The caller is expected to hold
/// the `FONT_SYSTEM` write lock for the same scope it uses the
/// returned list — that's how the renderer wires it today.
pub fn attrs_list_from_regions(
    source: &ColorFontRegions,
    font_system: &mut FontSystem,
) -> AttrsList {
    let mut attr_list = AttrsList::new(&Attrs::new());
    for region in &source.regions {
        let mut attrs = Attrs::new().style(Style::Normal);

        if let Some(color) = region.color.as_ref() {
            let rgba = convert_f32_to_u8(color);
            attrs = attrs.color(Color::rgba(rgba[0], rgba[1], rgba[2], rgba[3]));
        }

        // Resolve the font family. Both miss paths (compiled-id map
        // miss, fontdb face miss) fall back to Monospace with a
        // warning — consistent with §4's "degrade the frame, not
        // abort the process" rule.
        let family = resolve_font_family(region.font.as_ref(), font_system);
        attrs = match family {
            Some(ref name) => attrs.family(Family::Name(name.as_str())),
            None => attrs.family(Family::Monospace),
        };
        attr_list.add_span(region.range.to_rust_range(), &attrs);
    }
    attr_list
}

/// Look up the font-family name for a compiled font id. Returns
/// `None` silently when `font_id` is `None` (the region asked for no
/// pin); returns `None` with a `log::warn!` when `font_id` is
/// `Some` but the lookup misses (corrupt save / build skew).
/// Callers decide what `None` means at the cosmic-text seam:
/// [`attrs_list_from_regions`] forces `Family::Monospace`,
/// [`rich_text_spans_from_regions`] omits the family pin entirely.
fn resolve_font_family(
    font_id: Option<&crate::font::fonts::AppFont>,
    font_system: &mut FontSystem,
) -> Option<String> {
    let font_id = font_id?;
    let face_ids = match COMPILED_FONT_ID_MAP.get(font_id) {
        Some(ids) if !ids.is_empty() => ids,
        _ => {
            warn!("font::attrs: unknown font id {font_id:?}, dropping family pin");
            return None;
        }
    };
    match font_system.db().face(face_ids[0]) {
        Some(face) => Some(face.families[0].0.clone()),
        None => {
            warn!("font::attrs: fontdb face miss for {font_id:?}, dropping family pin");
            None
        }
    }
}

/// Pre-resolved family-name strings for every region in a
/// `ColorFontRegions` source. Built once per text area, then reused
/// across multiple shape passes — typically the renderer's main glyph
/// pass + the eight outline-halo stamps.
///
/// Resolution runs `font_system.db().face(...)` once per region with
/// a font id. Reusing the same `RegionFamilies` across halo passes
/// avoids re-doing those lookups for every stamp.
///
/// Indexing matches `ColorFontRegions::all_regions()` order: entry
/// `i` corresponds to the `i`-th region returned by
/// `all_regions()`. `None` entries mean the region had no pin or
/// resolution missed (logged via `log::warn!` at resolve time).
pub struct RegionFamilies {
    names: Vec<Option<String>>,
}

impl RegionFamilies {
    /// Resolve every region's family-name string. Empty input
    /// produces an empty `RegionFamilies` (no allocation beyond the
    /// outer `Vec` itself).
    ///
    /// Cost: `O(n_regions)` plus one `font_system.db().face()` lookup
    /// per region with a font id. The caller is expected to hold the
    /// `font_system` write guard for the duration of this call —
    /// downstream shape passes that consult the result need their
    /// own access to `font_system` via the same guard scope.
    pub fn resolve(source: &ColorFontRegions, font_system: &mut FontSystem) -> Self {
        let names = source
            .all_regions()
            .iter()
            .map(|region| resolve_font_family(region.font.as_ref(), font_system))
            .collect();
        Self { names }
    }

    /// Number of resolved entries. Matches `source.num_regions()` at
    /// resolve time.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// `true` when no regions were resolved.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Borrow the `i`-th resolved family name, if any. Out-of-range
    /// indices and unresolved entries both yield `None`.
    pub fn get(&self, i: usize) -> Option<&str> {
        self.names.get(i).and_then(|s| s.as_deref())
    }
}

/// Build a `(text_slice, Attrs)` span list ready for
/// `Buffer::set_rich_text` from `text` + `regions` + a pre-resolved
/// [`RegionFamilies`]. One span per region; spans whose grapheme
/// range slices to an empty byte range are dropped silently
/// (degenerate edits leave such regions briefly).
///
/// `color_override = Some(c)` recolors **every** span to `c` — used
/// by outline-halo passes to stamp the same glyphs in the halo
/// color while preserving per-region font pins. `None` keeps each
/// region's own `region.color` (cosmic-text default for `None`).
///
/// Each span carries `Metrics::new(scale, line_height)` so per-area
/// metrics survive cosmic-text shaping. Empty input
/// (`regions.num_regions() == 0`) produces a single span over the
/// whole `text` — the data-model contract that "no regions" means
/// "uniform default styling".
///
/// Cost: O(n_regions) iteration; allocates the outer `Vec`. The
/// returned `Attrs<'a>` borrow strings from `families`, so
/// `families` must outlive the returned vector.
pub fn rich_text_spans_from_regions<'a>(
    text: &'a str,
    regions: &ColorFontRegions,
    families: &'a RegionFamilies,
    scale: f32,
    line_height: f32,
    color_override: Option<Color>,
) -> Vec<(&'a str, Attrs<'a>)> {
    let metrics = Metrics::new(scale, line_height);
    if regions.num_regions() == 0 {
        let mut attrs = Attrs::new().metrics(metrics);
        if let Some(c) = color_override {
            attrs = attrs.color(c);
        }
        return vec![(text, attrs)];
    }
    regions
        .all_regions()
        .iter()
        .enumerate()
        .filter_map(|(i, region)| {
            let start = grapheme_chad::find_byte_index_of_char(text, region.range.start)
                .unwrap_or(text.len());
            let end = grapheme_chad::find_byte_index_of_char(text, region.range.end)
                .unwrap_or(text.len());
            if start >= end {
                return None;
            }
            let slice = &text[start..end];
            let mut attrs = Attrs::new().metrics(metrics);
            let color = color_override.or_else(|| region.color.map(rgba_to_color));
            if let Some(c) = color {
                attrs = attrs.color(c);
            }
            if let Some(family) = families.get(i) {
                attrs = attrs.family(Family::Name(family));
            }
            Some((slice, attrs))
        })
        .collect()
}

/// Pack a `[f32; 4]` baumhard color into a cosmic-text `Color`.
/// Per-channel `[0.0, 1.0]` → `[0, 255]` via `convert_f32_to_u8`.
fn rgba_to_color(rgba: FloatRgba) -> Color {
    let u8c = convert_f32_to_u8(&rgba);
    Color::rgba(u8c[0], u8c[1], u8c[2], u8c[3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::primitives::{ColorFontRegion, Range};

    /// Empty regions produce an empty span list. The defaults stored
    /// inside `AttrsList` are not exposed via `spans()`, so an empty
    /// input gives a length-0 span list.
    #[test]
    fn test_attrs_list_from_empty_regions_yields_no_spans() {
        // We don't need to load fonts here because the function only
        // touches the FontSystem inside the per-region loop, which
        // never runs on an empty input.
        let regions = ColorFontRegions::new_empty();
        let mut fs = FontSystem::new();
        let list = attrs_list_from_regions(&regions, &mut fs);
        assert_eq!(list.spans().len(), 0);
    }

    /// A single region with a color and no font produces one span,
    /// with the color converted from f32 to u8 internally.
    #[test]
    fn test_attrs_list_from_single_color_region_emits_one_span() {
        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, 5),
            None,
            Some([1.0, 0.0, 0.0, 1.0]),
        ));
        let mut fs = FontSystem::new();
        let list = attrs_list_from_regions(&regions, &mut fs);
        assert_eq!(list.spans().len(), 1);
    }

    /// Two adjacent regions emit two spans. Guards against the
    /// inherited region pipeline collapsing distinct ranges into one.
    #[test]
    fn test_attrs_list_from_two_regions_emits_two_spans() {
        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, 5),
            None,
            Some([1.0, 0.0, 0.0, 1.0]),
        ));
        regions.submit_region(ColorFontRegion::new(
            Range::new(5, 10),
            None,
            Some([0.0, 1.0, 0.0, 1.0]),
        ));
        let mut fs = FontSystem::new();
        let list = attrs_list_from_regions(&regions, &mut fs);
        assert_eq!(list.spans().len(), 2);
    }

    /// A region pinned to a real loaded `AppFont` emits a span
    /// whose `family_owned` is `Name(<family-name>)` — pinning the
    /// data-model → renderer end-to-end resolution path that the
    /// `font set` feature relies on. Regression guard against the
    /// silent-no-op bug the tree-builder fix closed.
    #[test]
    fn test_attrs_list_pins_family_name_when_region_carries_app_font() {
        use cosmic_text::FamilyOwned;

        crate::font::fonts::init();
        // Pick a real loaded family + its AppFont.
        let family = crate::font::fonts::loaded_families_iter()
            .next()
            .expect("at least one loaded family");
        let app_font =
            crate::font::fonts::app_font_by_family(family).expect("first family must round-trip");
        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, 4),
            Some(app_font),
            Some([1.0, 1.0, 1.0, 1.0]),
        ));
        // Use the *global* FONT_SYSTEM so the lookup actually
        // finds the bundled fonts — `FontSystem::new()` would
        // start empty and fall back to monospace, missing the
        // contract we're trying to pin.
        let mut fs = crate::font::fonts::acquire_font_system_write(
            "attrs_tests::test_attrs_list_pins_family_name_when_region_carries_app_font",
        );
        let list = attrs_list_from_regions(&regions, &mut fs);
        let spans = list.spans();
        assert_eq!(spans.len(), 1, "one region → one span");
        match &spans[0].1.family_owned {
            FamilyOwned::Name(name) => {
                assert_eq!(name.as_str(), family);
            }
            other => panic!("expected Family::Name({:?}), got {:?}", family, other),
        }
    }

    /// `resolve_font_family` returns `None` for a region without a
    /// font id; the calling path then pins `Family::Monospace` per
    /// the §9 fallback policy. The test pins both halves: the
    /// helper's `None` return *and* the resulting span's
    /// `Family::Monospace`.
    #[test]
    fn test_attrs_list_falls_back_to_monospace_when_region_has_no_font() {
        use cosmic_text::FamilyOwned;

        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, 4),
            None,
            Some([0.0, 0.0, 0.0, 1.0]),
        ));
        let mut fs = FontSystem::new();
        let list = attrs_list_from_regions(&regions, &mut fs);
        let spans = list.spans();
        assert_eq!(spans.len(), 1);
        // Monospace is the documented fallback.
        match &spans[0].1.family_owned {
            FamilyOwned::Monospace => {}
            other => panic!("expected Family::Monospace, got {:?}", other),
        }
    }

    /// Empty regions on the rich-text path produce a single span
    /// covering the whole text — the data-model contract that "no
    /// regions" means "uniform default styling".
    #[test]
    fn test_rich_text_spans_empty_regions_yield_single_whole_text_span() {
        let regions = ColorFontRegions::new_empty();
        let mut fs = FontSystem::new();
        let families = RegionFamilies::resolve(&regions, &mut fs);
        let spans = rich_text_spans_from_regions("hello", &regions, &families, 16.0, 18.0, None);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].0, "hello");
    }

    /// Two adjacent regions emit two `(slice, attrs)` pairs whose
    /// text slices match the per-region byte ranges.
    #[test]
    fn test_rich_text_spans_two_regions_slice_text_per_range() {
        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, 5),
            None,
            Some([1.0, 0.0, 0.0, 1.0]),
        ));
        regions.submit_region(ColorFontRegion::new(
            Range::new(5, 11),
            None,
            Some([0.0, 1.0, 0.0, 1.0]),
        ));
        let mut fs = FontSystem::new();
        let families = RegionFamilies::resolve(&regions, &mut fs);
        let spans =
            rich_text_spans_from_regions("hello world", &regions, &families, 16.0, 18.0, None);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].0, "hello");
        assert_eq!(spans[1].0, " world");
    }

    /// A degenerate region (start >= end after byte mapping) is
    /// dropped silently — the renderer must not hand cosmic-text
    /// zero-width spans.
    #[test]
    fn test_rich_text_spans_drop_zero_width_regions() {
        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(Range::new(3, 3), None, None));
        let mut fs = FontSystem::new();
        let families = RegionFamilies::resolve(&regions, &mut fs);
        let spans = rich_text_spans_from_regions("hello", &regions, &families, 16.0, 18.0, None);
        assert!(spans.is_empty());
    }

    /// `color_override = Some(c)` recolors every span — the halo
    /// path's contract. Per-region colors are ignored when the
    /// override is set.
    #[test]
    fn test_rich_text_spans_color_override_recolors_every_span() {
        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, 3),
            None,
            Some([1.0, 0.0, 0.0, 1.0]),
        ));
        regions.submit_region(ColorFontRegion::new(
            Range::new(3, 6),
            None,
            Some([0.0, 1.0, 0.0, 1.0]),
        ));
        let mut fs = FontSystem::new();
        let families = RegionFamilies::resolve(&regions, &mut fs);
        let halo = Color::rgba(255, 255, 0, 255);
        let spans =
            rich_text_spans_from_regions("abcdef", &regions, &families, 16.0, 18.0, Some(halo));
        assert_eq!(spans.len(), 2);
        for (_slice, attrs) in &spans {
            assert_eq!(attrs.color_opt, Some(halo));
        }
    }

    /// A region pinned to a real `AppFont` produces a span whose
    /// `family` is `Name(<family-name>)` — pinning the same
    /// data-model → renderer end-to-end path that
    /// `attrs_list_from_regions` covers, but on the
    /// `set_rich_text` API shape.
    #[test]
    fn test_rich_text_spans_pin_family_name_when_region_has_app_font() {
        crate::font::fonts::init();
        let family = crate::font::fonts::loaded_families_iter()
            .next()
            .expect("at least one loaded family");
        let app_font =
            crate::font::fonts::app_font_by_family(family).expect("first family must round-trip");
        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(Range::new(0, 3), Some(app_font), None));
        let mut fs = crate::font::fonts::acquire_font_system_write(
            "attrs_tests::test_rich_text_spans_pin_family_name_when_region_has_app_font",
        );
        let families = RegionFamilies::resolve(&regions, &mut fs);
        let spans = rich_text_spans_from_regions("abc", &regions, &families, 16.0, 18.0, None);
        assert_eq!(spans.len(), 1);
        match spans[0].1.family {
            Family::Name(name) => assert_eq!(name, family),
            other => panic!("expected Family::Name({:?}), got {:?}", family, other),
        }
    }

    /// A region with `font: None` produces a span with no family
    /// pin (cosmic-text default) — the rich-text variant differs
    /// from `attrs_list_from_regions`, which forces
    /// `Family::Monospace`. The walker's pre-existing behaviour was
    /// the no-pin variant; preserving it keeps the renderer's
    /// fallback-font choice in cosmic-text's hands rather than
    /// forcing monospace on every unpinned region.
    #[test]
    fn test_rich_text_spans_no_family_pin_when_region_has_no_font() {
        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, 3),
            None,
            Some([1.0, 1.0, 1.0, 1.0]),
        ));
        let mut fs = FontSystem::new();
        let families = RegionFamilies::resolve(&regions, &mut fs);
        let spans = rich_text_spans_from_regions("abc", &regions, &families, 16.0, 18.0, None);
        assert_eq!(spans.len(), 1);
        // No `family()` call means cosmic-text's default — *not*
        // `Family::Name`. The contract under test is "we do not
        // pin a name on no-pin regions".
        assert!(
            !matches!(spans[0].1.family, Family::Name(_)),
            "expected no Family::Name pin, got {:?}",
            spans[0].1.family,
        );
    }
}
