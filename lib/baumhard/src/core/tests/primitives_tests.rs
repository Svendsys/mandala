// SPDX-License-Identifier: MPL-2.0

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::font::fonts::AppFont;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref OVERLAPS_TEST: Vec<(Range, Range, bool)> = vec![
        (Range::new(0, 10), Range::new(10, 20), false),
        (Range::new(0, 10), Range::new(9, 20), true),
        (Range::new(0, 10), Range::new(0, 20), true),
        (Range::new(5, 10), Range::new(0, 20), true),
        (Range::new(5, 10), Range::new(0, 5), false),
        (Range::new(5, 10), Range::new(0, 6), true),
        (Range::new(5, 10), Range::new(8, 9), true),
    ];

    /// Truth table for [`ColorFontRegions::split_and_separate`]:
    /// `(case name, starting regions, inserted range, expected regions)`.
    /// Expected lists are in `BTreeSet` order (`start` then `end`)
    /// because that is the order the set iterates in.
    ///
    /// The table exists because the primitive's three arms are decided
    /// by where a region sits relative to the *insertion point*
    /// (`range.start`) — not relative to the inserted span as a whole,
    /// which is the confusion that produced the inverted ranges this
    /// table now pins down.
    pub static ref SPLIT_AND_SEPARATE_TABLE: Vec<(&'static str, Vec<Range>, Range, Vec<Range>)> = vec![
        (
            "region ends exactly at the insertion point — left-adjacent, untouched",
            vec![Range::new(0, 4)],
            Range::new(4, 8),
            vec![Range::new(0, 4)],
        ),
        (
            "region fully left with a gap — untouched",
            vec![Range::new(0, 3)],
            Range::new(4, 8),
            vec![Range::new(0, 3)],
        ),
        (
            "region straddles the insertion point — head keeps the prefix, tail is pushed past the span",
            vec![Range::new(0, 16)],
            Range::new(4, 8),
            vec![Range::new(0, 4), Range::new(8, 20)],
        ),
        (
            "straddler with a one-cluster tail — the tail survives as a one-cluster region",
            vec![Range::new(0, 5)],
            Range::new(4, 8),
            vec![Range::new(0, 4), Range::new(8, 9)],
        ),
        (
            "region begins exactly at the insertion point — pure shift, never split",
            vec![Range::new(4, 10)],
            Range::new(4, 8),
            vec![Range::new(8, 14)],
        ),
        (
            "region exactly equal to the inserted range — pure shift, no empty husk",
            vec![Range::new(4, 8)],
            Range::new(4, 8),
            vec![Range::new(8, 12)],
        ),
        (
            "region right of the insertion point but overlapping the inserted span — pure shift, no inverted half",
            vec![Range::new(5, 10)],
            Range::new(3, 7),
            vec![Range::new(9, 14)],
        ),
        (
            "region fully right and disjoint — pure shift",
            vec![Range::new(10, 15)],
            Range::new(4, 8),
            vec![Range::new(14, 19)],
        ),
        (
            "insertion at index 0 pushes the whole set right",
            vec![Range::new(0, 10)],
            Range::new(0, 3),
            vec![Range::new(3, 13)],
        ),
        (
            "straddler plus a follower — one splits, the other shifts",
            vec![Range::new(0, 16), Range::new(16, 32)],
            Range::new(4, 8),
            vec![Range::new(0, 4), Range::new(8, 20), Range::new(20, 36)],
        ),
        (
            "zero-magnitude insertion is a no-op, not a split-with-no-gap",
            vec![Range::new(0, 10)],
            Range::new(4, 4),
            vec![Range::new(0, 10)],
        ),
        (
            "inverted insertion range is dropped, not underflowed",
            vec![Range::new(0, 10)],
            Range::new(8, 4),
            vec![Range::new(0, 10)],
        ),
    ];
}

#[test]
fn test_overlaps() {
    do_overlaps();
}

pub fn do_overlaps() {
    for (a, b, expected) in OVERLAPS_TEST.clone() {
        let result = a.overlaps(&b);
        assert_eq!(result, expected);
        assert_eq!(result, b.overlaps(&a))
    }
}

#[test]
fn test_split_and_separate_1() {
    do_split_and_separate_1();
}

pub fn do_split_and_separate_1() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 16)));
    regions.split_and_separate(Range::new(4, 8));
    assert_eq!(regions.num_regions(), 2);
    let _region_1 = regions.get(Range::new(0, 4)).unwrap();
    let _region_2 = regions.get(Range::new(8, 20)).unwrap();
}
#[test]
fn test_split_and_separate_2() {
    do_split_and_separate_2();
}

pub fn do_split_and_separate_2() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 16)));
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(16, 32)));
    regions.split_and_separate(Range::new(4, 8));
    assert_eq!(regions.num_regions(), 3);
    let _region_1 = regions.get(Range::new(0, 4)).unwrap();
    let _region_2 = regions.get(Range::new(8, 20)).unwrap();
    let _region_3 = regions.get(Range::new(20, 36)).unwrap();
}

#[test]
fn test_split_and_separate_truth_table() {
    do_split_and_separate_truth_table();
}

/// Walks [`SPLIT_AND_SEPARATE_TABLE`], asserting the exact resulting
/// region set for each case plus two invariants the primitive must
/// never break: no range is inverted or empty, and the total number of
/// covered grapheme clusters is conserved (splitting moves coverage,
/// it never creates or destroys any).
pub fn do_split_and_separate_truth_table() {
    for (name, initial, inserted, expected) in SPLIT_AND_SEPARATE_TABLE.iter() {
        let mut regions = ColorFontRegions::new_empty();
        for range in initial {
            regions.submit_region(ColorFontRegion::new_key_only(*range));
        }
        let covered_before: usize = initial.iter().map(|r| r.magnitude()).sum();

        regions.split_and_separate(*inserted);

        let got: Vec<Range> = regions.all_regions().iter().map(|r| r.range).collect();
        assert_eq!(&got, expected, "case `{}`", name);
        let mut covered_after = 0usize;
        for range in &got {
            assert!(
                range.start < range.end,
                "case `{}` produced the degenerate range {}..{}",
                name,
                range.start,
                range.end
            );
            covered_after += range.magnitude();
        }
        assert_eq!(
            covered_before, covered_after,
            "case `{}` did not conserve covered cluster count",
            name
        );
    }
}

#[test]
fn test_split_and_separate_preserves_payload_on_both_halves() {
    do_split_and_separate_preserves_payload_on_both_halves();
}

/// Splitting a straddler must carry the color and font pin onto
/// *both* halves — the two halves are the same styled run, just with
/// the newly inserted (and deliberately unstyled) span between them.
pub fn do_split_and_separate_preserves_payload_on_both_halves() {
    let gold = [1.0, 0.84, 0.0, 1.0];
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new(
        Range::new(0, 16),
        Some(AppFont::NotoSerifTibetanRegular),
        Some(gold),
    ));

    regions.split_and_separate(Range::new(4, 8));

    let head = regions.get(Range::new(0, 4)).unwrap();
    let tail = regions.get(Range::new(8, 20)).unwrap();
    assert_eq!(head.color, Some(gold));
    assert_eq!(tail.color, Some(gold));
    assert_eq!(head.font, Some(AppFont::NotoSerifTibetanRegular));
    assert_eq!(tail.font, Some(AppFont::NotoSerifTibetanRegular));
    // The inserted span itself is left uncovered on purpose — the
    // caller styles it with a follow-up `submit_region`.
    assert!(regions.get(Range::new(4, 8)).is_none());
}

#[test]
fn test_submit_region_drops_inverted_range() {
    do_submit_region_drops_inverted_range();
}

/// Regression for the `panic!` removed from `submit_region` in chunk
/// 2: an inverted (`start > end`) range used to abort the editor.
/// It now logs and is silently dropped, so a malformed mutation
/// degrades the frame instead.
pub fn do_submit_region_drops_inverted_range() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 16)));
    // Intentionally inverted — start > end. Pre-fix this would panic.
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(20, 5)));
    assert_eq!(regions.num_regions(), 1);
    let _kept = regions.get(Range::new(0, 16)).unwrap();
}

#[test]
fn test_single_span_empty_is_empty() {
    do_single_span_empty_is_empty();
}

/// `single_span(0, ...)` returns a region set with zero regions —
/// matches the `cluster_count > 0` guard every former open-coded
/// call site wrote by hand.
pub fn do_single_span_empty_is_empty() {
    let regions = ColorFontRegions::single_span(0, Some([1.0, 0.0, 0.0, 1.0]), None);
    assert_eq!(regions.num_regions(), 0);
}

#[test]
fn test_single_span_non_empty_covers_range() {
    do_single_span_non_empty_covers_range();
}

/// `single_span(N, color, font)` produces one region covering
/// `[0, N)` with the given color + font pin.
pub fn do_single_span_non_empty_covers_range() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let regions = ColorFontRegions::single_span(7, Some(red), Some(AppFont::NotoSerifTibetanRegular));
    assert_eq!(regions.num_regions(), 1);
    let r = regions.get(Range::new(0, 7)).unwrap();
    assert_eq!(r.range.start, 0);
    assert_eq!(r.range.end, 7);
    assert_eq!(r.color, Some(red));
    assert_eq!(r.font, Some(AppFont::NotoSerifTibetanRegular));
}

#[test]
fn test_single_span_none_color_none_font() {
    do_single_span_none_color_none_font();
}

/// Both `color` and `font` may be `None` — matches the renderer's
/// border-text areas where the renderer default color wins.
pub fn do_single_span_none_color_none_font() {
    let regions = ColorFontRegions::single_span(3, None, None);
    assert_eq!(regions.num_regions(), 1);
    let r = regions.get(Range::new(0, 3)).unwrap();
    assert_eq!(r.color, None);
    assert_eq!(r.font, None);
}

#[test]
fn test_shrink_regions_after_fully_right_shifts_left() {
    do_shrink_regions_after_fully_right_shifts_left();
}

/// Regions that sit fully right of the deletion window shift left
/// by `magnitude`.
pub fn do_shrink_regions_after_fully_right_shifts_left() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 3)));
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(10, 15)));
    regions.shrink_regions_after(5, 2);
    assert_eq!(regions.num_regions(), 2);
    assert!(regions.get(Range::new(0, 3)).is_some());
    assert!(regions.get(Range::new(8, 13)).is_some());
}

#[test]
fn test_shrink_regions_after_spanning_region_absorbs() {
    do_shrink_regions_after_spanning_region_absorbs();
}

/// A region that straddles the deletion window with strict room on
/// both sides absorbs the deletion: its `end` shrinks by the cut's
/// magnitude.
pub fn do_shrink_regions_after_spanning_region_absorbs() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 10)));
    regions.shrink_regions_after(3, 4);
    assert_eq!(regions.num_regions(), 1);
    assert!(regions.get(Range::new(0, 6)).is_some());
}

#[test]
fn test_shrink_regions_after_fully_inside_collapses() {
    do_shrink_regions_after_fully_inside_collapses();
}

/// A region that lies fully inside the deletion window is dropped
/// from the set — the text it covered is gone.
pub fn do_shrink_regions_after_fully_inside_collapses() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 3)));
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(4, 7)));
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(10, 15)));
    regions.shrink_regions_after(4, 3);
    assert_eq!(regions.num_regions(), 2);
    assert!(regions.get(Range::new(0, 3)).is_some());
    assert!(regions.get(Range::new(7, 12)).is_some());
}

#[test]
fn test_shrink_regions_after_left_partial_clamps() {
    do_shrink_regions_after_left_partial_clamps();
}

/// A region whose right edge falls inside the deletion window
/// clamps its `end` to the cut's start.
pub fn do_shrink_regions_after_left_partial_clamps() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 5)));
    regions.shrink_regions_after(3, 4);
    assert_eq!(regions.num_regions(), 1);
    assert!(regions.get(Range::new(0, 3)).is_some());
}

#[test]
fn test_shrink_regions_after_right_partial_clamps() {
    do_shrink_regions_after_right_partial_clamps();
}

/// A region whose left edge falls inside the deletion window
/// clamps its `start` to the cut's start and shifts its `end` left
/// by the cut's magnitude, so the region sits flush against the
/// remaining-text boundary.
pub fn do_shrink_regions_after_right_partial_clamps() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(5, 15)));
    regions.shrink_regions_after(3, 4);
    assert_eq!(regions.num_regions(), 1);
    assert!(regions.get(Range::new(3, 11)).is_some());
}

#[test]
fn test_shrink_regions_after_zero_magnitude_is_noop() {
    do_shrink_regions_after_zero_magnitude_is_noop();
}

/// `magnitude == 0` means "nothing was deleted"; the region set
/// must be unchanged.
pub fn do_shrink_regions_after_zero_magnitude_is_noop() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 3)));
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(5, 10)));
    regions.shrink_regions_after(4, 0);
    assert_eq!(regions.num_regions(), 2);
    assert!(regions.get(Range::new(0, 3)).is_some());
    assert!(regions.get(Range::new(5, 10)).is_some());
}

#[test]
fn test_insert_regions_at_straddling_region_absorbs() {
    do_insert_regions_at_straddling_region_absorbs();
}

/// A region that straddles the insertion point absorbs the new
/// chars — its `end` grows by `magnitude`.
pub fn do_insert_regions_at_straddling_region_absorbs() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 6)));
    let absorbed = regions.insert_regions_at(3, 2);
    assert!(absorbed);
    assert_eq!(regions.num_regions(), 1);
    assert!(regions.get(Range::new(0, 8)).is_some());
}

#[test]
fn test_insert_regions_at_left_adjacent_region_absorbs() {
    do_insert_regions_at_left_adjacent_region_absorbs();
}

/// A region whose `end == idx` (left-adjacent to the insertion)
/// absorbs the new chars rather than leaving them uncovered.
pub fn do_insert_regions_at_left_adjacent_region_absorbs() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 3)));
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(3, 6)));
    let absorbed = regions.insert_regions_at(3, 2);
    assert!(absorbed);
    // The left region extends; the right region shifts.
    assert_eq!(regions.num_regions(), 2);
    assert!(regions.get(Range::new(0, 5)).is_some());
    assert!(regions.get(Range::new(5, 8)).is_some());
}

#[test]
fn test_insert_regions_at_shifts_right_regions() {
    do_insert_regions_at_shifts_right_regions();
}

/// Regions entirely right of the insertion shift right by `magnitude`.
pub fn do_insert_regions_at_shifts_right_regions() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(10, 15)));
    let absorbed = regions.insert_regions_at(5, 3);
    assert!(!absorbed);
    assert_eq!(regions.num_regions(), 1);
    assert!(regions.get(Range::new(13, 18)).is_some());
}

#[test]
fn test_insert_regions_at_zero_position_shifts_all() {
    do_insert_regions_at_zero_position_shifts_all();
}

/// `idx == 0` shifts every region right by `magnitude` (no absorber
/// can exist because no region can have `start < 0`).
pub fn do_insert_regions_at_zero_position_shifts_all() {
    let mut regions = ColorFontRegions::new_empty();
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(0, 5)));
    regions.submit_region(ColorFontRegion::new_key_only(Range::new(5, 10)));
    let absorbed = regions.insert_regions_at(0, 2);
    assert!(!absorbed);
    assert_eq!(regions.num_regions(), 2);
    assert!(regions.get(Range::new(2, 7)).is_some());
    assert!(regions.get(Range::new(7, 12)).is_some());
}

#[test]
fn test_insert_regions_at_empty_returns_false() {
    do_insert_regions_at_empty_returns_false();
}

/// Inserting into an empty region set returns `false` — the caller
/// (the text-editor caret path) uses this to insert a fresh region
/// for the caret glyph so it renders in an empty-buffer node.
pub fn do_insert_regions_at_empty_returns_false() {
    let mut regions = ColorFontRegions::new_empty();
    let absorbed = regions.insert_regions_at(0, 1);
    assert!(!absorbed);
    assert_eq!(regions.num_regions(), 0);
}
