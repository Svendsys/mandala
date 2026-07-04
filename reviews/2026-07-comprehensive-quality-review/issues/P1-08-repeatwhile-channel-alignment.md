# P1-08: RepeatWhile sibling alignment is order-dependent and drops mutator-without-target matches; DEFAULT_TERMINATOR shares the stale assumption

**Severity:** P1 (walker correctness — mutation language misapplies) · **Area:** baumhard/gfx_structs/tree_walker · **Verified:** yes (independent read + agent analysis agree)

## Context

`align_child_walks` was hardened to sort both sibling rows by channel before the merge walk ("The arena order of children is **not** assumed to be channel-ascending … This removes a long-standing fragility", `tree_walker.rs:283-292`) because mindmap children arrive in Dewey-id order while `MindNode.channel` is arbitrary user data (CONCEPTS §2 "Channel" warns exactly this). The RepeatWhile instruction path never got the same fix.

## Problem

`lib/baumhard/src/gfx_structs/tree_walker.rs:181-241` (`compare_apply_repeat_while`):

1. **Raw arena order**: it walks sibling order directly; its doc even claims "Channel-ascending invariant on both sides is the same one align_child_walks documents" — which now documents the *opposite* (it sorts).
2. **Match-dropping advance rule even for sorted input**: when `m_chan < t_chan` it advances **both** cursors:

```rust
if m_chan >= t_chan {
    if let Some(next_t) = maybe_next_target { target_id = next_t; continue; }
}
match (next_mutator, maybe_next_target) {
    (Some(next_m), Some(next_t)) => { target_id = next_t; mutator_id = next_m; }
    _ => return,
}
```

Targets `[2,3]` vs mutators `[1,2]`: pair (1,2) no match → advance both → (2,3) no match → return. The channel-2 mutator never reaches the channel-2 target. A correct sorted merge advances only the mutator when `m < t`.

3. `DEFAULT_TERMINATOR` (`tree_walker.rs:32-68`) carries the same ascending-order assumption (`else if next.channel() > t_chan { break; }`), so post-loop resumption also mis-pairs on non-ascending channel rows.

Every existing RepeatWhile test uses ascending, gap-free channels, so neither defect is caught. Any JSON custom mutation using RepeatWhile with channel gaps or Dewey-order-vs-channel-order disagreement silently misapplies today.

## Fix plan

1. Reuse `collect_sorted_children` (already in the file) inside `compare_apply_repeat_while` and in `DEFAULT_TERMINATOR`'s sibling scan.
2. Fix the advance rule to the standard sorted merge: `m < t` → advance mutator only; `m > t` → advance target only; `m == t` → apply + advance target (preserving broadcast semantics: one mutator hits every same-channel target).
3. Correct the stale doc comment (it should now say both paths sort, matching align_child_walks).
4. Walker tests (same commit, §T7): (a) RepeatWhile over targets with non-ascending channels `[2,0,1]`; (b) a mutator channel with no target counterpart followed by a matching pair (`targets [2,3]` vs `mutators [1,2]`); (c) DEFAULT_TERMINATOR resumption over a non-ascending row.

## Acceptance criteria

- New tests fail on current main, pass after fix.
- Existing walker tests (broadcast, skip-style RepeatWhile) unchanged.
- `./test.sh` green; walker benches show no regression (`./test.sh --bench` — align allocation posture with §B7: reuse the sorted-children vectors if the sort is added to a hot loop).

## Pointers

`lib/baumhard/src/gfx_structs/tree_walker.rs:32-68, 181-241, 272-358`; `lib/baumhard/src/gfx_structs/tests/tree_walker_tests.rs`; CONCEPTS §2 (Channel: Dewey order vs channel order); CONVENTIONS §B2/§B7.
