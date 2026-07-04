# P2-39: Loader — "cheap" legacy screen false-positives on every styled map (full second JSON parse per load); unknown keys silently dropped (author data loss on resave)

**Severity:** P2 (startup cost on every real map + silent data loss for hand-authors) · **Area:** baumhard/mindmap/loader · **Found independently by two reviewers**

## Problem A — double parse on virtually every load

`loader.rs:83-85`: `has_legacy_marker(json)` substring-matches `"text_runs":` — but **post-section files legitimately contain that key** inside every styled section (`maps/testament.mindmap.json` has 254 occurrences), so `detect_legacy_shape` re-parses the entire document into `serde_json::Value` on essentially every non-trivial load. The cost doc claims the Value walk "runs only when … the cheap substring screen flags a dropped field" — the screen flags ~always. Load path is self-described "felt every map load"; doubles peak memory for large maps on mobile (§B1/§4).

**Fix:** drop `"text_runs":` from the marker — the zero-sections symptom already catches real legacy nodes (they lack `sections`), or scope the check to node-level keys via the already-typed map. Fix the cost doc.

## Problem B — unknown keys vanish silently

serde ignores unknown keys (no `deny_unknown_fields`, no warning). A typo'd `"min_zoom_to_rendr": 2.0` or `"portal_form": {...}` is dropped without trace at load and **vanishes at save** — silent data loss for hand-authored files, the exact failure mode `detect_legacy_shape` exists to catch, but it screens only three historical keys. (The loader/verify division of labor for semantic checks is documented and fine; the unknown-key hole is the part that destroys data, and the app — not verify — is what resaves files.)

**Fix:** after a successful typed parse, one Value-walk comparing object keys against known field sets, `log::warn!` per unknown key with its JSON path ("node '1.2': unknown field 'portal_form' will be dropped on save"). Given Problem A's fix, this walk should be opt-in-cheap (only when a marker suggests it) or accepted as the one-time load cost with an honest doc — decide and document. A shared known-keys table can be derived per struct via a small macro or maintained beside the serde derives with a drift test.

## Acceptance criteria

- Loading testament does exactly one full parse (add a counter/probe in a test or assert via the legacy-detector's unit tests on marker behavior).
- Loading a file with an unknown node key logs a warning naming node + key; round-trip test demonstrates the warning fires before the data would be lost.
- `./test.sh` green.

## Pointers

`lib/baumhard/src/mindmap/loader.rs:32-93`; TEST_CONVENTIONS §T1 ("loader edges" are a fundamental — also add the missing malformed-JSON / missing-field / unknown-edge_type load tests identified in review); CODE_CONVENTIONS §9 (error messages point at the offending node/field).
