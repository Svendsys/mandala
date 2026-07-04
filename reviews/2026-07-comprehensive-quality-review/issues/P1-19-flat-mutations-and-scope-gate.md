# P1-19: `flat_mutations` guard has a `||`/`&&` precedence bug (match-nothing RepeatWhile applies to everything); `covers_reach` encodes the wrong anchoring model for Children/Siblings

**Severity:** P1 (mutation-safety gates unsound) · **Area:** baumhard/mindmap/custom_mutation

## Problem A — precedence bug inverts a documented footgun

`lib/baumhard/src/mindmap/custom_mutation/mod.rs:297-311`:

```rust
p.always_match || p.fields.iter().all(|(_, c)| matches!(c, Comparator::Equals(_))) && p.fields.is_empty()
```

Parses as `a || (b && c)`. `b` (`.all(...)` over fields) is vacuously true exactly when `c` (`fields.is_empty()`) is, so the guard reduces to `always_match || fields.is_empty()` — the `.all(Equals)` clause is dead code and the intent is mangled. Consequence: `RepeatWhile(Predicate { fields: [], always_match: false })` — which `Predicate::test` treats as *matches nothing* and mod.rs:99-102's own doc calls "matches nothing — a footgun" — is nevertheless extracted by the flat-apply path, and its Macro's mutations land on the **whole scope set**. The comment above the guard says "Only honour `always_true` predicates … falling through to `None`" — the code does the opposite for the empty-fields case. Semantic inversion between the flat-apply path and the (future) walker path for the same JSON. (Undo remains safe — the scope snapshot covers it — but the applied result differs from the authored AST.)

**Fix:** tighten to `p.always_match` only (matching the comment), so match-nothing shapes hit the existing non-flat warn path; delete the dead clause; add a test with `always_match: false, fields: []`.

## Problem B — `covers_reach` approves pairings whose undo snapshot is narrower than the touch set

`mod.rs:384-401`: `TargetScope::Children | TargetScope::Siblings => reach <= MutatorReach::Children`. But the documented application model is **per-target anchoring** (`scope.rs:18-23`: "the application layer is responsible for iterating … and anchoring the mutator at each of them" — and flat-apply does exactly that for every scope). Under per-target anchoring, a `MapChildren`-reach mutator anchored at each sibling touches each sibling's **children** — nodes absent from the Siblings snapshot (`collect_affected_node_ids` returns siblings only). CONCEPTS §4 calls the undo-snapshot equivalence "the load-bearing detail"; the gate guarding it approves violations for two arms. Latent today (flat_mutations returns None for MapChildren roots) but `covers_reach` is wired as the apply-time safety check precisely for the walker path the code says is coming.

Also: `Siblings` on a root node silently resolves to the empty set (`custom/mod.rs:582-595`) — other roots are arguably siblings; undocumented either way.

**Fix:** decide the anchoring contract per scope and encode it: with per-target anchoring, `Children`/`Siblings` must require `reach == SelfOnly`. Document root-node Siblings semantics. Add gate tests for the MapChildren-under-Siblings pairing.

## Acceptance criteria

- Match-nothing RepeatWhile JSON no longer flat-applies (warn instead); test pins it.
- `covers_reach` rejects (or the snapshot covers) every reach the applied mutator can touch, per scope; tests enumerate scope × reach.
- `./test.sh` green.

## Pointers

`lib/baumhard/src/mindmap/custom_mutation/{mod.rs,scope.rs}`; `src/application/document/custom/mod.rs:396-597` (snapshot collection); `lib/baumhard/src/gfx_structs/predicate.rs:150-186`; CONCEPTS §4 (Target scopes).
