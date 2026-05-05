// SPDX-License-Identifier: MPL-2.0

//! Pure manipulation primitives for `Vec<TextRun>` — the foundation
//! N4 (per-grapheme range targeting) builds on. Each helper preserves
//! the format invariants documented in `format/text-runs.md`: runs
//! are sorted ascending, half-open `[start, end)` grapheme indices,
//! no overlaps, gaps allowed (uncovered ranges inherit section /
//! node defaults).
//!
//! All operations are pure functions over `&[TextRun]` or
//! `&mut Vec<TextRun>`; no I/O, no allocation beyond the result
//! vector's growth, and no `unsafe`. With N typically under 20
//! per section, every helper runs in linear time over the run
//! vector.

use super::node::TextRun;

/// Index in `runs` of the run whose `[start, end)` contains
/// `grapheme_idx`, or `None` when `grapheme_idx` falls in a gap
/// or past the end. The half-open convention means a run
/// boundary at `idx == run.end` does **not** count — that
/// position belongs to the next run (or to a gap, if there is
/// one).
///
/// Linear scan with an early-out when a run's `start` exceeds
/// the index — runs are sorted ascending so anything past that
/// point can't contain `grapheme_idx`.
pub fn find_run_containing(runs: &[TextRun], grapheme_idx: usize) -> Option<usize> {
    for (i, run) in runs.iter().enumerate() {
        if run.start > grapheme_idx {
            return None;
        }
        if grapheme_idx < run.end {
            return Some(i);
        }
    }
    None
}

/// Index in `runs` of the run whose `start` equals `grapheme_idx`,
/// or `None` when no run begins exactly there. Pairs with
/// [`split_at`]: after `split_at(runs, idx)` succeeds, the
/// right-hand half lives at the index this returns.
pub fn find_run_starting_at(runs: &[TextRun], grapheme_idx: usize) -> Option<usize> {
    runs.iter().position(|r| r.start == grapheme_idx)
}

/// Ensure a run boundary at `grapheme_idx` by splitting the run
/// that straddles it (if any) into two adjacent runs sharing
/// every style attribute. Returns `true` when a split was
/// performed; `false` when `grapheme_idx` already sits on a
/// boundary, falls in a gap, or lies past the end of every run
/// (all of those are no-ops because the boundary either already
/// exists or is meaningless to introduce).
///
/// Pairs with [`merge_adjacent_equal`]: a range-targeted setter
/// calls `split_at(start)` + `split_at(end)` to carve out the
/// exact run set covering `[start, end)`, mutates each in place,
/// then `merge_adjacent_equal` to coalesce neighbours that
/// became identical.
pub fn split_at(runs: &mut Vec<TextRun>, grapheme_idx: usize) -> bool {
    let target_idx = match find_run_containing(runs, grapheme_idx) {
        Some(i) => i,
        None => return false,
    };
    // Boundary already at start of the run — no split needed.
    if runs[target_idx].start == grapheme_idx {
        return false;
    }
    let mut right = runs[target_idx].clone();
    runs[target_idx].end = grapheme_idx;
    right.start = grapheme_idx;
    runs.insert(target_idx + 1, right);
    true
}

/// Clone every run that intersects `[start, end)`, with each
/// returned run's `start` and `end` clamped to the slice
/// bounds. Original-coordinate output (not re-based to
/// `slice_start`) — the caller scanning attributes
/// (`current_color_at` over a range, "is every run in this
/// range bold?") doesn't need re-basing.
///
/// Empty result when `start >= end`, when the range falls in a
/// gap entirely, or when no runs overlap.
pub fn slice(runs: &[TextRun], slice_start: usize, slice_end: usize) -> Vec<TextRun> {
    if slice_start >= slice_end {
        return Vec::new();
    }
    let mut out = Vec::new();
    for run in runs {
        if run.end <= slice_start {
            continue;
        }
        if run.start >= slice_end {
            break;
        }
        let mut clipped = run.clone();
        if clipped.start < slice_start {
            clipped.start = slice_start;
        }
        if clipped.end > slice_end {
            clipped.end = slice_end;
        }
        out.push(clipped);
    }
    out
}

/// Coalesce neighbouring runs that share every style attribute
/// AND meet at a common boundary (`runs[i].end == runs[i+1].start`).
/// Runs separated by a gap stay separate even when their
/// attributes match — the gap carries semantic information
/// (uncovered grapheme ranges fall through to section / node
/// defaults, which may differ from the runs' attributes).
///
/// Single forward pass; cost is O(N) over `runs.len()`.
pub fn merge_adjacent_equal(runs: &mut Vec<TextRun>) {
    if runs.len() < 2 {
        return;
    }
    let mut write = 0usize;
    for read in 1..runs.len() {
        let prev_end = runs[write].end;
        if runs[read].start == prev_end && style_eq(&runs[write], &runs[read]) {
            runs[write].end = runs[read].end;
        } else {
            write += 1;
            if write != read {
                runs.swap(write, read);
            }
        }
    }
    runs.truncate(write + 1);
}

/// Two runs share every style attribute (the seven fields a
/// `TextRun` carries beyond `start` / `end`). Used by
/// [`merge_adjacent_equal`] to decide whether two adjacent runs
/// should coalesce.
fn style_eq(a: &TextRun, b: &TextRun) -> bool {
    a.bold == b.bold
        && a.italic == b.italic
        && a.underline == b.underline
        && a.font == b.font
        && a.size_pt == b.size_pt
        && a.color == b.color
        && a.hyperlink == b.hyperlink
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(start: usize, end: usize, color: &str) -> TextRun {
        TextRun {
            start,
            end,
            bold: false,
            italic: false,
            underline: false,
            font: "Sans".into(),
            size_pt: 12,
            color: color.into(),
            hyperlink: None,
        }
    }

    // ── find_run_containing ──────────────────────────────────────

    #[test]
    fn test_find_run_containing_hits_inside() {
        let runs = vec![run(0, 5, "red"), run(5, 10, "blue")];
        assert_eq!(find_run_containing(&runs, 0), Some(0));
        assert_eq!(find_run_containing(&runs, 4), Some(0));
        assert_eq!(find_run_containing(&runs, 5), Some(1));
        assert_eq!(find_run_containing(&runs, 9), Some(1));
    }

    #[test]
    fn test_find_run_containing_excludes_end() {
        // `[start, end)` half-open — `idx == last.end` is past
        // the last run and falls in no run.
        let runs = vec![run(0, 10, "red")];
        assert_eq!(find_run_containing(&runs, 10), None);
    }

    #[test]
    fn test_find_run_containing_gap_is_none() {
        let runs = vec![run(0, 5, "red"), run(7, 10, "blue")];
        assert_eq!(find_run_containing(&runs, 5), None);
        assert_eq!(find_run_containing(&runs, 6), None);
    }

    #[test]
    fn test_find_run_containing_empty_runs_is_none() {
        let runs: Vec<TextRun> = Vec::new();
        assert_eq!(find_run_containing(&runs, 0), None);
        assert_eq!(find_run_containing(&runs, 100), None);
    }

    // ── find_run_starting_at ─────────────────────────────────────

    #[test]
    fn test_find_run_starting_at_finds_exact_boundary() {
        let runs = vec![run(0, 5, "red"), run(5, 10, "blue"), run(10, 15, "green")];
        assert_eq!(find_run_starting_at(&runs, 0), Some(0));
        assert_eq!(find_run_starting_at(&runs, 5), Some(1));
        assert_eq!(find_run_starting_at(&runs, 10), Some(2));
    }

    #[test]
    fn test_find_run_starting_at_returns_none_for_non_boundary() {
        let runs = vec![run(0, 5, "red"), run(5, 10, "blue")];
        assert_eq!(find_run_starting_at(&runs, 3), None);
        assert_eq!(find_run_starting_at(&runs, 7), None);
        assert_eq!(find_run_starting_at(&runs, 10), None);
    }

    // ── split_at ─────────────────────────────────────────────────

    #[test]
    fn test_split_at_inside_run_inserts_right_half() {
        let mut runs = vec![run(0, 10, "red")];
        let split = split_at(&mut runs, 5);
        assert!(split, "split inside a run must report true");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 5);
        assert_eq!(runs[1].start, 5);
        assert_eq!(runs[1].end, 10);
        // Style attributes inherit on both halves.
        assert_eq!(runs[0].color, "red");
        assert_eq!(runs[1].color, "red");
    }

    #[test]
    fn test_split_at_on_left_boundary_is_noop() {
        let mut runs = vec![run(0, 10, "red")];
        let split = split_at(&mut runs, 0);
        assert!(!split);
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn test_split_at_on_right_boundary_is_noop() {
        // `idx == run.end` falls past the run (half-open). No
        // split because the boundary at `run.end` already
        // exists implicitly — there's no run-half on the right
        // to carve out.
        let mut runs = vec![run(0, 10, "red")];
        let split = split_at(&mut runs, 10);
        assert!(!split);
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn test_split_at_in_gap_is_noop() {
        let mut runs = vec![run(0, 5, "red"), run(7, 10, "blue")];
        let split = split_at(&mut runs, 6);
        assert!(!split);
        assert_eq!(runs.len(), 2);
    }

    #[test]
    fn test_split_at_past_all_runs_is_noop() {
        let mut runs = vec![run(0, 10, "red")];
        let split = split_at(&mut runs, 100);
        assert!(!split);
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn test_split_at_followed_by_find_run_starting_at_locates_right_half() {
        // The intended call pattern for range-targeted setters:
        // split, then locate the right half by `start == idx`.
        let mut runs = vec![run(0, 20, "red")];
        split_at(&mut runs, 7);
        assert_eq!(find_run_starting_at(&runs, 7), Some(1));
    }

    // ── slice ────────────────────────────────────────────────────

    #[test]
    fn test_slice_clips_runs_to_bounds() {
        let runs = vec![run(0, 5, "red"), run(5, 10, "blue")];
        let out = slice(&runs, 2, 8);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start, 2);
        assert_eq!(out[0].end, 5);
        assert_eq!(out[0].color, "red");
        assert_eq!(out[1].start, 5);
        assert_eq!(out[1].end, 8);
        assert_eq!(out[1].color, "blue");
    }

    #[test]
    fn test_slice_preserves_internal_gaps() {
        let runs = vec![run(0, 5, "red"), run(7, 10, "blue")];
        let out = slice(&runs, 3, 9);
        // Gap `[5, 7)` survives — clipped output drops nothing
        // beyond runs that don't intersect.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start, 3);
        assert_eq!(out[0].end, 5);
        assert_eq!(out[1].start, 7);
        assert_eq!(out[1].end, 9);
    }

    #[test]
    fn test_slice_empty_when_start_ge_end() {
        let runs = vec![run(0, 10, "red")];
        assert!(slice(&runs, 5, 5).is_empty());
        assert!(slice(&runs, 7, 3).is_empty());
    }

    #[test]
    fn test_slice_empty_when_range_in_gap() {
        let runs = vec![run(0, 5, "red"), run(10, 15, "blue")];
        assert!(slice(&runs, 6, 9).is_empty());
    }

    #[test]
    fn test_slice_full_range_returns_clones() {
        let runs = vec![run(0, 5, "red"), run(5, 10, "blue")];
        let out = slice(&runs, 0, 10);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], runs[0]);
        assert_eq!(out[1], runs[1]);
    }

    // ── merge_adjacent_equal ─────────────────────────────────────

    #[test]
    fn test_merge_adjacent_equal_coalesces_matching_neighbours() {
        let mut runs = vec![run(0, 5, "red"), run(5, 10, "red")];
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 10);
    }

    #[test]
    fn test_merge_adjacent_equal_preserves_mismatched_style() {
        let mut runs = vec![run(0, 5, "red"), run(5, 10, "blue")];
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 2);
    }

    #[test]
    fn test_merge_adjacent_equal_preserves_gap_boundary() {
        // Gap means `runs[i].end != runs[i+1].start` — the gap
        // carries semantic information (uncovered range falls
        // through to defaults), so neighbours separated by a
        // gap MUST stay separate even when their attributes
        // match.
        let mut runs = vec![run(0, 5, "red"), run(7, 10, "red")];
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].end, 5);
        assert_eq!(runs[1].start, 7);
    }

    #[test]
    fn test_merge_adjacent_equal_chains_three_runs() {
        let mut runs = vec![
            run(0, 5, "red"),
            run(5, 10, "red"),
            run(10, 15, "red"),
        ];
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 15);
    }

    #[test]
    fn test_merge_adjacent_equal_partial_chain() {
        // First two runs match, third differs — only the first
        // pair coalesces.
        let mut runs = vec![
            run(0, 5, "red"),
            run(5, 10, "red"),
            run(10, 15, "blue"),
        ];
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 10);
        assert_eq!(runs[1].start, 10);
        assert_eq!(runs[1].end, 15);
    }

    #[test]
    fn test_merge_adjacent_equal_empty_and_single_are_noops() {
        let mut empty: Vec<TextRun> = Vec::new();
        merge_adjacent_equal(&mut empty);
        assert!(empty.is_empty());

        let mut single = vec![run(0, 5, "red")];
        merge_adjacent_equal(&mut single);
        assert_eq!(single.len(), 1);
    }

    // ── Round-trip integration: split + mutate + merge ───────────

    /// The intended call shape for a range-targeted setter:
    /// split at both ends → mutate runs in `[range.start,
    /// range.end)` → merge adjacent equals. Pins the
    /// composition contract — mutating the carved-out slice
    /// followed by a merge produces exactly the runs the user
    /// would expect from "set [3, 7) to blue".
    #[test]
    fn test_split_mutate_merge_round_trip() {
        let mut runs = vec![run(0, 10, "red")];
        // Carve out [3, 7).
        split_at(&mut runs, 3);
        split_at(&mut runs, 7);
        assert_eq!(runs.len(), 3);

        // Mutate the carved-out middle run's colour.
        for r in runs.iter_mut() {
            if r.start >= 3 && r.end <= 7 {
                r.color = "blue".into();
            }
        }

        // Merge — neighbours don't match the new blue run, so
        // the three-run shape survives.
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].color, "red");
        assert_eq!(runs[1].color, "blue");
        assert_eq!(runs[2].color, "red");
    }

    /// When the user "sets [3, 7) to red" on an already-red
    /// section, the carved-out middle run matches its
    /// neighbours and merge collapses back to a single run —
    /// the no-op-write should not leave the section with split
    /// runs.
    #[test]
    fn test_split_mutate_merge_no_op_recoalesces() {
        let mut runs = vec![run(0, 10, "red")];
        split_at(&mut runs, 3);
        split_at(&mut runs, 7);
        assert_eq!(runs.len(), 3);

        // No actual mutation — every run stays red.
        merge_adjacent_equal(&mut runs);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 10);
        assert_eq!(runs[0].color, "red");
    }
}
