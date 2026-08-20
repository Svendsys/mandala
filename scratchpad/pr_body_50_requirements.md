# #50 PR body — required content

Shape: follow PR #165's body. NO performance claims (CLAUDE.md §7).
Merge with merge_method "merge". subscribe_pr_activity immediately after opening.

The body MUST carry all of these:

1. **The claim-not-file boundary.** Sweeping by *claim* rather than by *file*
   found fifteen sites the filed issue's list missed. (#164 swept by file and
   missed nine; #165 replaced the boundary and found a tenth. This is the third
   series where the boundary itself was the finding.)

2. **The two wrong control instructions, and that all eight were run.**
   CONCEPTS carries eight control instructions. Two were wrong:
   - The `NodeShape` recipe claimed one exhaustive match; adding a variant
     yields multiple E0004s. It omitted `shader_id` and `intersects_local_aabb`,
     and the WGSL case it *did* list is the half no match covers.
   - The placard's "compile error in `adopt`" lands in `resolve` — `adopt` is
     three lines with no match.

3. **The citation-form change as a structural fix.** Nine of thirteen
   line-range citations pointed somewhere wrong (e.g. `color_picker/mod.rs:1-77`
   at a 71-line file). All thirteen were replaced with path + item name, and the
   preamble promising the rotting form was updated. This removes the form that
   rots, not just the instances that had rotted.

4. **`MindMapDocument` has no tree.** It was described as owning a
   `tree: Option<MindMapTree>` field. Verified mechanically: 15 fields, no tree.

5. **The console open keybind is `/`, not `Ctrl+;`** (`keybinds/config.rs:86`).

6. **The audit itself was wrong about item 3's shape.** Item 3 was
   three-quarters closed, not half — four whole-canvas functions over seven
   per-role updaters, with `rebuild_camera_geometry` missing from the audit's
   enumeration entirely.

7. **The §8 digit-restating rule, broken one series after it was written.**
   The touch-vocabulary text restates `LONG_PRESS_MS`'s digits two bullets after
   naming the constant. #165 added that rule; #161 wrote the content that breaks
   it. State whether it was fixed or only observed.

8. **Item 10's thirteen sites.** The issue said "#41 removes the remnants," but
   thirteen in-code comments across nine files still described the retired chip
   row as live. The issue text was a hypothesis, not a specification.

9. **The pinnable-claims list left for #163.**

## Open rulings the review must settle (verify, do not accept)

- **Two hand counts left standing with a note**: the Scene shape cache entry's
  "59 overlay elements" and "~50 rows", where the picker payload is pinned at
  58. Is a known-suspect figure with a note acceptable, or is 59 simply wrong?

## Standing caution

Hand counts are this epic's most reliable source of wrong statements
("seventeen fields" was sixteen; "21 names" was 16; "two String clones" was
five; "four border runs" was eight at ten sites). I nearly reported a false
discrepancy in this very issue by hand-counting 15 fields as 7 — caught only by
re-counting mechanically. Re-count with `awk`/`grep -c`, never by eye.
