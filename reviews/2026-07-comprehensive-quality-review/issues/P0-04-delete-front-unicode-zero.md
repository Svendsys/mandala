# P0-04: `delete_front_unicode(s, 0)` deletes one grapheme instead of none

**Severity:** P0 (Unicode fundamental, silent text corruption) · **Area:** baumhard/util/grapheme_chad · **Verified:** yes (empirically, against the built crate)

## Problem

`lib/baumhard/src/util/grapheme_chad.rs:399-416`. Empirically confirmed:

```
delete_front_unicode("abcd", 0) => "bcd"   (expected "abcd")
delete_back_unicode("abcd", 0)  => "abcd"  (correct)
```

The loop increments `grapheme_count` and accumulates the grapheme's byte length **before** testing `if grapheme_count >= n { break; }` — with `n == 0` the first iteration passes `1 >= 0`, breaks with `char_count = len(first grapheme)`, and `s.drain(0..char_count)` removes it. `delete_back_unicode` tests `grapheme_count > n` *before* accumulating, so the pair is asymmetric.

## Reachability

- `GlyphLine`'s delete-range splitting calls `comp.discard_front(end - begin)` / `discard_front(end - e_begin_comp)` (`lib/baumhard/src/gfx_structs/model/line.rs:257,291,320` via `component.rs:178`) where boundary-aligned ranges naturally produce `0` — silently eats a rendered glyph.
- The console's `kill_to_start` guards `cursor == 0` before calling (`src/application/app/console_input/edit.rs:161-164`) — a workaround-shaped guard that hides the defect.
- `REMOVE_PREFIX_TESTS` has no `n = 0` case, which is why this survived (§T1: "Test the surprising inputs before the obvious ones").

## Fix plan

1. Early-return `if n == 0 { return; }` — or restructure the loop to test before accumulating, mirroring `delete_back_unicode` (preferred: makes the pair symmetric).
2. Add `("abcd", 0, "abcd")` to `REMOVE_PREFIX_TESTS` and a matching `n = 0` case to `TRUNCATE_TESTS` (`lib/baumhard/src/util/tests/grapheme_chad_tests.rs`) in the same commit.
3. Optionally remove the now-unnecessary caller guard in `console_input/edit.rs` (or keep it as a cheap short-circuit with a corrected comment).

## Acceptance criteria

- `delete_front_unicode(s, 0)` is a no-op for all inputs including emoji/ZWJ-leading strings.
- New fixture rows fail on current main, pass after fix.
- `./test.sh` green; `./test.sh --bench` still passes (both functions are bench-reachable through the tests tree).

## Pointers

`lib/baumhard/src/util/grapheme_chad.rs:376-416` (both functions — fix the asymmetry, keep the shapes parallel); CONVENTIONS §B3; TEST_CONVENTIONS §T1.
