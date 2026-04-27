// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::font::attrs`] — the `ColorFontRegions` →
//! cosmic-text bridges (`attrs_list_from_regions`,
//! [`RegionFamilies`], [`rich_text_spans_from_regions`]).
//!
//! Follows the `do_*()` / `test_*()` split from §B8 — every `do_*`
//! body is benchmarkable from `benches/test_bench.rs`.

use cosmic_text::{Color, Family, FontSystem};

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::font::attrs::{
    attrs_list_from_regions, rich_text_spans_from_regions, RegionFamilies,
};

// ---------------------------------------------------------------------------
// attrs_list_from_regions — `Editor::insert_string` shape
// ---------------------------------------------------------------------------

#[test]
fn test_attrs_list_from_empty_regions_yields_no_spans() {
    do_attrs_list_from_empty_regions_yields_no_spans();
}

/// Empty regions produce an empty span list. The defaults stored
/// inside `AttrsList` are not exposed via `spans()`, so an empty
/// input gives a length-0 span list.
pub fn do_attrs_list_from_empty_regions_yields_no_spans() {
    let regions = ColorFontRegions::new_empty();
    let mut fs = FontSystem::new();
    let list = attrs_list_from_regions("hello world", &regions, &mut fs);
    assert_eq!(list.spans().len(), 0);
}

#[test]
fn test_attrs_list_from_single_color_region_emits_one_span() {
    do_attrs_list_from_single_color_region_emits_one_span();
}

/// A single region with a color and no font produces one span,
/// with the color converted from f32 to u8 internally.
pub fn do_attrs_list_from_single_color_region_emits_one_span() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(
        Range::new(0, 5),
        None,
        Some([1.0, 0.0, 0.0, 1.0]),
    ));
    let mut fs = FontSystem::new();
    let list = attrs_list_from_regions("hello world", &regions, &mut fs);
    assert_eq!(list.spans().len(), 1);
}

#[test]
fn test_attrs_list_from_two_regions_emits_two_spans() {
    do_attrs_list_from_two_regions_emits_two_spans();
}

/// Two adjacent regions emit two spans. Guards against the
/// inherited region pipeline collapsing distinct ranges into one.
pub fn do_attrs_list_from_two_regions_emits_two_spans() {
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
    let list = attrs_list_from_regions("hello world", &regions, &mut fs);
    assert_eq!(list.spans().len(), 2);
}

#[test]
fn test_attrs_list_pins_family_name_when_region_carries_app_font() {
    do_attrs_list_pins_family_name_when_region_carries_app_font();
}

/// A region pinned to a real loaded `AppFont` emits a span whose
/// `family_owned` is `Name(<family-name>)` — pinning the data-model
/// → renderer end-to-end resolution path that the `font set`
/// feature relies on.
pub fn do_attrs_list_pins_family_name_when_region_carries_app_font() {
    use cosmic_text::FamilyOwned;

    crate::font::fonts::init();
    let family = crate::font::fonts::loaded_families_iter()
        .next()
        .expect("at least one loaded family");
    let app_font = crate::font::fonts::app_font_by_family(family)
        .expect("first family must round-trip");
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(
        Range::new(0, 4),
        Some(app_font),
        Some([1.0, 1.0, 1.0, 1.0]),
    ));
    let mut fs = crate::font::fonts::acquire_font_system_write(
        "attrs_tests::do_attrs_list_pins_family_name_when_region_carries_app_font",
    );
    let list = attrs_list_from_regions("hello world", &regions, &mut fs);
    let spans = list.spans();
    assert_eq!(spans.len(), 1, "one region → one span");
    match &spans[0].1.family_owned {
        FamilyOwned::Name(name) => assert_eq!(name.as_str(), family),
        other => panic!("expected Family::Name({:?}), got {:?}", family, other),
    }
}

#[test]
fn test_attrs_list_falls_back_to_monospace_when_region_has_no_font() {
    do_attrs_list_falls_back_to_monospace_when_region_has_no_font();
}

/// `resolve_font_family` returns `None` for a region without a
/// font id; `attrs_list_from_regions` then forces
/// `Family::Monospace` per its documented fallback. The matching
/// contract for the spans helper (no pin instead of forced
/// monospace) is in `do_rich_text_spans_no_family_pin_when_region_has_no_font`.
pub fn do_attrs_list_falls_back_to_monospace_when_region_has_no_font() {
    use cosmic_text::FamilyOwned;

    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(
        Range::new(0, 4),
        None,
        Some([0.0, 0.0, 0.0, 1.0]),
    ));
    let mut fs = FontSystem::new();
    let list = attrs_list_from_regions("hello world", &regions, &mut fs);
    let spans = list.spans();
    assert_eq!(spans.len(), 1);
    match &spans[0].1.family_owned {
        FamilyOwned::Monospace => {}
        other => panic!("expected Family::Monospace, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// rich_text_spans_from_regions — `Buffer::set_rich_text` shape
// ---------------------------------------------------------------------------

#[test]
fn test_rich_text_spans_empty_regions_yield_single_whole_text_span() {
    do_rich_text_spans_empty_regions_yield_single_whole_text_span();
}

/// Empty regions on the rich-text path produce a single span
/// covering the whole text — the data-model contract that "no
/// regions" means "uniform default styling".
pub fn do_rich_text_spans_empty_regions_yield_single_whole_text_span() {
    let regions = ColorFontRegions::new_empty();
    let mut fs = FontSystem::new();
    let families = RegionFamilies::resolve(&regions, &mut fs);
    let spans = rich_text_spans_from_regions("hello", &families, 16.0, 18.0, None);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].0, "hello");
}

#[test]
fn test_rich_text_spans_two_regions_slice_text_per_range() {
    do_rich_text_spans_two_regions_slice_text_per_range();
}

/// Two adjacent regions emit two `(slice, attrs)` pairs whose text
/// slices match the per-region byte ranges.
pub fn do_rich_text_spans_two_regions_slice_text_per_range() {
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
    let spans = rich_text_spans_from_regions("hello world", &families, 16.0, 18.0, None);
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].0, "hello");
    assert_eq!(spans[1].0, " world");
}

#[test]
fn test_rich_text_spans_drop_zero_width_regions() {
    do_rich_text_spans_drop_zero_width_regions();
}

/// A degenerate region (start >= end after byte mapping) is dropped
/// silently — the renderer must not hand cosmic-text zero-width
/// spans.
pub fn do_rich_text_spans_drop_zero_width_regions() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(Range::new(3, 3), None, None));
    let mut fs = FontSystem::new();
    let families = RegionFamilies::resolve(&regions, &mut fs);
    let spans = rich_text_spans_from_regions("hello", &families, 16.0, 18.0, None);
    assert!(spans.is_empty());
}

#[test]
fn test_rich_text_spans_color_override_recolors_every_span() {
    do_rich_text_spans_color_override_recolors_every_span();
}

/// `color_override = Some(c)` recolors every span — the halo path's
/// contract. Per-region colors are ignored when the override is set.
pub fn do_rich_text_spans_color_override_recolors_every_span() {
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
    let spans = rich_text_spans_from_regions("abcdef", &families, 16.0, 18.0, Some(halo));
    assert_eq!(spans.len(), 2);
    for (_slice, attrs) in &spans {
        assert_eq!(attrs.color_opt, Some(halo));
    }
}

#[test]
fn test_rich_text_spans_pin_family_name_when_region_has_app_font() {
    do_rich_text_spans_pin_family_name_when_region_has_app_font();
}

/// A region pinned to a real `AppFont` produces a span whose
/// `family` is `Name(<family-name>)` — the same data-model →
/// renderer pin path `attrs_list_from_regions` covers, on the
/// `set_rich_text` API shape.
pub fn do_rich_text_spans_pin_family_name_when_region_has_app_font() {
    crate::font::fonts::init();
    let family = crate::font::fonts::loaded_families_iter()
        .next()
        .expect("at least one loaded family");
    let app_font = crate::font::fonts::app_font_by_family(family)
        .expect("first family must round-trip");
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(Range::new(0, 3), Some(app_font), None));
    let mut fs = crate::font::fonts::acquire_font_system_write(
        "attrs_tests::do_rich_text_spans_pin_family_name_when_region_has_app_font",
    );
    let families = RegionFamilies::resolve(&regions, &mut fs);
    let spans = rich_text_spans_from_regions("abc", &families, 16.0, 18.0, None);
    assert_eq!(spans.len(), 1);
    match spans[0].1.family {
        Family::Name(name) => assert_eq!(name, family),
        other => panic!("expected Family::Name({:?}), got {:?}", family, other),
    }
}

#[test]
fn test_rich_text_spans_no_family_pin_when_region_has_no_font() {
    do_rich_text_spans_no_family_pin_when_region_has_no_font();
}

/// A region with `font: None` produces a span with no family pin
/// (cosmic-text default) — the rich-text variant differs from
/// `attrs_list_from_regions`, which forces `Family::Monospace`. The
/// walker's pre-existing behaviour was the no-pin variant; preserving
/// it keeps the renderer's fallback-font choice in cosmic-text's
/// hands rather than forcing monospace on every unpinned region.
pub fn do_rich_text_spans_no_family_pin_when_region_has_no_font() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(
        Range::new(0, 3),
        None,
        Some([1.0, 1.0, 1.0, 1.0]),
    ));
    let mut fs = FontSystem::new();
    let families = RegionFamilies::resolve(&regions, &mut fs);
    let spans = rich_text_spans_from_regions("abc", &families, 16.0, 18.0, None);
    assert_eq!(spans.len(), 1);
    // No `family()` call means cosmic-text's default — *not*
    // `Family::Name`. The contract under test is "we do not pin a
    // name on no-pin regions".
    assert!(
        !matches!(spans[0].1.family, Family::Name(_)),
        "expected no Family::Name pin, got {:?}",
        spans[0].1.family,
    );
}

#[test]
fn test_rich_text_spans_clamps_out_of_range_region_end() {
    do_rich_text_spans_clamps_out_of_range_region_end();
}

/// A region whose `range.end` exceeds the text's char count clamps
/// to `text.len()` rather than panicking or producing a malformed
/// span. The text is consumed up to its actual end; any further
/// region clamps to the same byte index, dropping the now-empty
/// trailing remainder via the zero-width filter. Corrupt-save
/// resilience per §9.
pub fn do_rich_text_spans_clamps_out_of_range_region_end() {
    let mut regions = ColorFontRegions::new_empty();
    // text "hello" is 5 chars; region asks for [0, 100).
    regions.submit_region(ColorFontRegion::new(
        Range::new(0, 100),
        None,
        Some([1.0, 0.0, 0.0, 1.0]),
    ));
    let mut fs = FontSystem::new();
    let families = RegionFamilies::resolve(&regions, &mut fs);
    let spans = rich_text_spans_from_regions("hello", &families, 16.0, 18.0, None);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].0, "hello");
}

#[test]
fn test_rich_text_spans_clamps_fully_out_of_range_region() {
    do_rich_text_spans_clamps_fully_out_of_range_region();
}

/// A region whose `range.start` is also past the text length
/// collapses to `start = end = text.len()` — zero-width — and is
/// dropped by the filter_map.
pub fn do_rich_text_spans_clamps_fully_out_of_range_region() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(
        Range::new(50, 100),
        None,
        Some([1.0, 0.0, 0.0, 1.0]),
    ));
    let mut fs = FontSystem::new();
    let families = RegionFamilies::resolve(&regions, &mut fs);
    let spans = rich_text_spans_from_regions("hi", &families, 16.0, 18.0, None);
    assert!(spans.is_empty());
}

#[test]
fn test_rich_text_spans_color_override_applies_to_uncolored_region() {
    do_rich_text_spans_color_override_applies_to_uncolored_region();
}

/// `color_override = Some(c)` applies to every span, including
/// regions with `color = None` (which would otherwise carry no
/// `.color()` call). Pin the halo recoloring contract for the
/// `region.color = None` branch the previous tests didn't cover.
pub fn do_rich_text_spans_color_override_applies_to_uncolored_region() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(Range::new(0, 3), None, None));
    let mut fs = FontSystem::new();
    let families = RegionFamilies::resolve(&regions, &mut fs);
    let halo = Color::rgba(0, 255, 0, 255);
    let spans = rich_text_spans_from_regions("abc", &families, 16.0, 18.0, Some(halo));
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].1.color_opt, Some(halo));
}

#[test]
fn test_rich_text_spans_color_override_drops_zero_width_regions() {
    do_rich_text_spans_color_override_drops_zero_width_regions();
}

/// A zero-width region is dropped even when `color_override` is set
/// — the override applies to spans we keep, not spans we'd
/// resurrect. Pin the interaction between the halo recolor and the
/// zero-width filter.
pub fn do_rich_text_spans_color_override_drops_zero_width_regions() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(Range::new(2, 2), None, None));
    let mut fs = FontSystem::new();
    let families = RegionFamilies::resolve(&regions, &mut fs);
    let halo = Color::rgba(0, 0, 255, 255);
    let spans = rich_text_spans_from_regions("hello", &families, 16.0, 18.0, Some(halo));
    assert!(spans.is_empty());
}

#[test]
fn test_rich_text_spans_slice_at_grapheme_boundary() {
    do_rich_text_spans_slice_at_grapheme_boundary();
}

/// `find_byte_index_of_grapheme` operates on grapheme clusters,
/// matching the documented unit of `ColorFontRegion::Range` (see
/// `CONCEPTS.md`'s Range entry and `lib/baumhard/CONVENTIONS.md
/// §B1`). A region whose `[start, end)` covers one grapheme
/// produces a span whose byte slice covers that whole cluster —
/// even when the cluster is multiple Unicode scalars (a regional-
/// indicator pair flag, a ZWJ-joined emoji family, etc). This
/// pins the grapheme-not-scalar contract.
pub fn do_rich_text_spans_slice_at_grapheme_boundary() {
    let mut regions = ColorFontRegions::new_empty();
    // Text is "🇸🇪🇫🇮" — two flag emojis, each a regional-indicator
    // pair (2 scalars). One grapheme cluster per flag, two clusters
    // total. Region [0, 1) selects only the first flag.
    regions.submit_region(ColorFontRegion::new(
        Range::new(0, 1),
        None,
        Some([1.0, 0.0, 0.0, 1.0]),
    ));
    let mut fs = FontSystem::new();
    let families = RegionFamilies::resolve(&regions, &mut fs);
    let spans = rich_text_spans_from_regions(
        "\u{1F1F8}\u{1F1EA}\u{1F1EB}\u{1F1EE}",
        &families,
        16.0,
        18.0,
        None,
    );
    assert_eq!(spans.len(), 1);
    // First flag is one grapheme = two scalars = 8 UTF-8 bytes.
    assert_eq!(spans[0].0, "\u{1F1F8}\u{1F1EA}");
    assert_eq!(spans[0].0.len(), 8);
}

#[test]
fn test_rich_text_spans_slice_at_zwj_grapheme_boundary() {
    do_rich_text_spans_slice_at_zwj_grapheme_boundary();
}

/// Sibling guard: a ZWJ-joined emoji (`👨‍👩‍👧` — five scalars, one
/// grapheme) at a region boundary must be sliced as one cluster,
/// not split mid-ZWJ. Catches a regression to char-indexed
/// slicing more dramatically than the regional-indicator test
/// because mis-slicing a ZWJ sequence emits a partial codepoint
/// run that's a different visible glyph.
pub fn do_rich_text_spans_slice_at_zwj_grapheme_boundary() {
    let mut regions = ColorFontRegions::new_empty();
    // Text is "👨‍👩‍👧A" — one ZWJ-emoji grapheme (5 scalars, 18
    // bytes) followed by ASCII "A". Two grapheme clusters.
    let text = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}A";
    regions.submit_region(ColorFontRegion::new(
        Range::new(0, 1),
        None,
        Some([1.0, 0.0, 0.0, 1.0]),
    ));
    let mut fs = FontSystem::new();
    let families = RegionFamilies::resolve(&regions, &mut fs);
    let spans = rich_text_spans_from_regions(text, &families, 16.0, 18.0, None);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].0, "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}");
    assert_eq!(spans[0].0.len(), 18, "5 scalars × 4 + 2 (ZWJs) = 18 bytes");
}

#[test]
fn test_rich_text_spans_empty_text_with_region_yields_no_spans() {
    do_rich_text_spans_empty_text_with_region_yields_no_spans();
}

/// Empty text with a non-empty region: `find_byte_index_of_char`
/// returns `None` for any positive index on an empty string,
/// clamping start/end to `text.len() = 0`. The zero-width filter
/// drops the span. Defensive against the renderer calling with
/// `area.text = ""` while regions still carry stale ranges.
pub fn do_rich_text_spans_empty_text_with_region_yields_no_spans() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(
        Range::new(0, 5),
        None,
        Some([1.0, 0.0, 0.0, 1.0]),
    ));
    let mut fs = FontSystem::new();
    let families = RegionFamilies::resolve(&regions, &mut fs);
    let spans = rich_text_spans_from_regions("", &families, 16.0, 18.0, None);
    assert!(spans.is_empty());
}
