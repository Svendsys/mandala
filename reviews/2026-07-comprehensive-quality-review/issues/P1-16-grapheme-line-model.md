# P1-16: grapheme_chad line-model incoherence — CRLF invisible to the grapheme line finder; trailing empty line counted but unaddressable

**Severity:** P1 (fundamentals disagree with each other) · **Area:** baumhard/util/grapheme_chad · **Verified:** empirically

## Problem A — CRLF (empirically confirmed)

```
count_number_lines("a\r\nb")              = 2
find_nth_line_grapheme_range("a\r\nb", 0) = Some((0, 3))   // swallows the break
find_nth_line_grapheme_range("a\r\nb", 1) = None           // line 1 "does not exist"
find_nth_line_byte_range("a\r\nb", 1)     = Some((3, 4))   // line 1 exists
```

Under UAX #29, `"\r\n"` is **one grapheme cluster**, so the grapheme walk's `graph == "\n"` test (`grapheme_chad.rs:155`) never matches; the byte-level siblings split on the raw `\n`. Three primitives that must agree return incompatible line models for the same string. `slice_to_newline` / `replace_graphemes_until_newline` additionally split the `\r\n` cluster mid-grapheme. Reachability: Windows-origin paste or any loaded text containing CRLF; nothing normalizes it away. The 580-line test file contains no `\r\n` case.

## Problem B — trailing line

`count_number_lines("abc\n") == 2` (the empty trailing line counts, by documented design), but `find_nth_line_grapheme_range("abc\n", 1)` and the byte variant both return `None` — the final line is counted yet unaddressable. Any caller iterating `0..count_number_lines(s)` gets a guaranteed `None` on the last iteration of every newline-terminated string; a cursor on the empty final line of a text area is exactly this shape. The fixtures pin both behaviors separately without confronting the contradiction.

## Fix plan

1. Pick ONE line model and apply it to all five primitives (`count_number_lines`, `find_nth_line_grapheme_range`, `find_nth_line_byte_range`, `slice_to_newline`, `replace_graphemes_until_newline`):
   - **Recommended:** treat `"\r\n"` (the single cluster) and lone `"\r"` as line terminators in the grapheme walk, aligning with the byte variant; and make both finders return `Some((len, len))`-style empty ranges for the trailing empty line (there is already a `("\n", 0) → Some((0,0))` precedent in the fixtures).
   - Alternative: mandate LF-only buffers and normalize at every input boundary (paste, load) — more call sites, easier primitives. Document the choice in the module header either way.
2. Add CRLF rows to `NTH_LINE_*_TEST`, `COUNT_LINES_TEST`, `SLICE_TO_NEWLINE_TEST`, and trailing-line rows exercising the `count..find` composition, same commit (§B3: tests+bench with the primitive).
3. Audit the callers that iterate lines (text_edit cursor math) for assumptions broken by the chosen model.

## Acceptance criteria

- For every input, `(0..count_number_lines(s)).all(|i| find_nth_line_grapheme_range(s, i).is_some())` holds, and grapheme/byte finders agree on line boundaries.
- CRLF fixtures fail on current main, pass after.
- `./test.sh` green; benches updated if signatures change (§B8 two-file rule).

## Pointers

`lib/baumhard/src/util/grapheme_chad.rs:18-22, 129-202`; `lib/baumhard/src/util/tests/grapheme_chad_tests.rs`; CONVENTIONS §B3; TEST_CONVENTIONS §T1.
