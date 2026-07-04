# P1-28: Console duplication cluster — section target resolution ×3, canvas re-implements border's positional machinery, font verb/Action dual dispatch, three verb↔Action single-core exceptions

**Severity:** P1 (SSOT; every listed copy has already drifted at least once) · **Area:** mandala/console + dispatch

## Problem A — section target resolution exists three times

`section/mod.rs:352-403` (`resolve_section_idx` + `resolve_node_id` + `parse_section_kv`); `section/frame.rs:456-508` (**byte-identical** `parse_section_kv` + `resolve_section_idx_for` — its doc admits "kept in sync; review-fix CRIT-2 closed the divergence where this resolver lacked the rule-3 single-section auto-resolve"); `dispatch/cross_dispatch/style.rs:249-273` (`target_section`, intentionally different rules, documented). The rule table is load-bearing UX; CRIT-2 proves the drift mode is real.

**Fix:** one `resolve_section_target(selection, doc, kv_idx, policy)` in a shared module; `policy` encodes the two legitimate divergences (doc-less Action path; MultiSection acceptance). Delete the duplicate `parse_section_kv`.

## Problem B — canvas.rs re-implements border's positional machinery

`canvas.rs:303-467` duplicates from `border/execute.rs:254-595`: side-selector parsing, corner-selector, non-custom preset gate, reset-restores-preset-glyph logic, `preset cycle` resolution — plus a fourth copy of the kv staging loop. The Batch-6 "blocker" tests exist precisely because these copies drifted (canvas reset hardcoded "light"; canvas silently auto-promoted while border errored) — the fix landed as more parallel code.

**Fix:** parameterize border's positional appliers over `enum BorderSurface { Nodes(..), CanvasSlot(CanvasSlot) }` supplying `current_preset()` + `apply(edits)`; canvas.rs calls them; delete `positional_subverb_to_edits`.

## Problem C — font: verb path and Action core are two parallel selection dispatchers

`font.rs:278-349` (`apply_font_args`) vs `font.rs:617-687` (`apply_font_kv_to_selection`) both match all 10 SelectionState variants and route to the same setters — neither calls the other. Behavioral drift already: Multi/MultiSection min/max surfaces an explanatory message on the verb path, silently returns false on the Action path. `zoom` is the exemplar (verb routes through `apply_zoom_to_selection`).

**Fix:** rebuild `apply_font_args` on the core (or extend the core to take the (size,min,max) triple, verb wraps for messaging). Also dedupe the byte-identical key→trait closure in color.rs (:308-319 vs :450-461).

## Problem D — three verb↔Action single-core exceptions

The CONCEPTS §6 claim ("dispatch arms call pub(crate) mutation cores extracted from each console verb") holds for ~15 of ~18 arms; the exceptions:
- `apply_cycle_border_preset` (`style.rs:30-59`) re-derives `first_selection_preset` inline (byte-equivalent to `border/execute.rs:190-208`).
- `apply_toggle_border_visible` (`style.rs:63-89`) re-implements the read-flip-write loop (`border/execute.rs:210-249`).
- `apply_split_section` (`style.rs:464-490`): `at_grapheme=None` defaults to end-of-text, while the verb **requires** `at=` with a dedicated error because that default was ruled a footgun (`section/mod.rs:708-731`) — a macro `SplitSection { at_grapheme: "" }` silently does the thing the verb was hardened against.

**Fix:** export cycle + toggle as `pub(crate)` cores in border/execute.rs, call from style.rs; make the Action reject empty `at_grapheme` like the verb (or document the divergence in both places + warn-log — rejection preferred).

## Acceptance criteria

- One resolver, one positional-machinery implementation, one font selection dispatcher, zero verb↔Action body divergences (macro and verb behave identically for the same operation).
- Existing border/canvas/section tests pass; add a macro-path `SplitSection` rejection test.
- `./test.sh` green.

## Pointers

Files cited inline; CODE_CONVENTIONS §3 ("No second copy of Action body logic"), §5; CONCEPTS §5 Action dispatch, §6 Console.
