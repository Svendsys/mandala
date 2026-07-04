# Mandala Console Deep Review — Findings

Scope: src/application/console/ (all), src/application/app/console_input/ (all), cross-referenced src/application/app/dispatch/ (action_core.rs, cross_dispatch/{style,edges,camera,mod}.rs) and baumhard color/hex/grapheme primitives. All files read end to end, including tests. Key claims verified by `cargo check -p mandala` (clean) and targeted `cargo test` runs (1611 tests in crate).

## Architecture assessment

The console is well-architected in its bones: one `const` command registry with per-verb `applicable`/`complete`/`execute` function pointers, one shell-style tokenizer, a capability-trait dispatcher (`TargetView` + `apply_kvs`) for the cross-cutting style verbs, `pub(crate)` mutation cores genuinely shared between console verbs and parametric `Action` dispatch arms (verified for ~15 of ~18 arms — the CONCEPTS §6 claim substantially holds), disciplined grapheme-indexed cursor math, zero panics on malformed user input, and an unusually strong regression-test culture (border/section/clipboard suites assert model state, undo round-trips, and exact error wording). The debt is concentrated in four places: (a) per-verb scaffolding — each of the 20 verbs hand-rolls subverb matching, kv parse/reject loops, usage strings, hint tables, and completion matches, so the same grammar lives in 2-4 places per verb; (b) the section/border family, where three target resolvers and a second copy of the border positional machinery (canvas.rs) are held in sync only by comments; (c) one real correctness bug on an untested path (`color text=accent section=K` double-wraps the var reference); and (d) a crate-wide `#![allow(dead_code)]` that quietly defeats CODE_CONVENTIONS §5's no-dead-code covenant and has already hidden a dead function and a test whose `#[test]` attribute was swallowed into a doc comment.

---

### 1. `color <axis>=<named-var> section=K` writes a double-wrapped var reference (`var(--var(--accent))`)
Severity: P1 | Category: correctness | Confidence: high
Files: src/application/console/commands/color.rs:370-374; src/application/console/traits/color_value.rs:44-51; src/application/console/constants.rs:18-20
Evidence:
    // color.rs, apply_section_colours (the `section=K` path):
    let resolved = match color_value {
        ColorValue::Hex(h) => h,
        ColorValue::Var(name) => format!("var(--{})", name),   // BUG
        ColorValue::Reset => "#ffffff".to_string(),
    };
`ColorValue::Var` carries the FULL model string: `ColorValue::parse("accent")` -> `Var(VAR_ACCENT)` where `VAR_ACCENT = "var(--accent)"`. So the format produces `"var(--var(--accent))"`, written into the section's `TextRun.color`. Every other call site correctly uses `as_model_string()` (color_value.rs:58-64), which returns the constant verbatim. `resolve_var` cannot resolve the malformed string, so the theme reference silently degrades to the renderer fallback.
Why it matters: `color text=accent section=1` (also fg/edge, with or without `range=`) persists a corrupted model value on save. The sibling trait-dispatch path (`color text=accent` on a Section selection) is correct and tested (color.rs:705), so the bug hides in exactly the untested cell — the `section=` kv tests use only hex values.
Fix: `ColorValue::Var(name) => name.to_string()` (or route through `as_model_string()`); add regression test `color text=accent section=1` asserting `run.color == "var(--accent)"`.
Effort: S

### 2. Regression test never runs — `#[test]` swallowed into the doc comment
Severity: P1 | Category: testing | Confidence: high (verified: `cargo test -p mandala section_resize_fill_literal` -> "running 0 tests")
Files: src/application/console/commands/section/mod.rs:1353-1355
Evidence:
    /// `section resize fill` (renamed from the prior `none`
    /// literal) clears `size` to fill-parent.    #[test]
    fn section_resize_fill_literal_clears_size() {
The `#[test]` is literal doc-comment text; the fn is unannotated and never executed. The crate-wide `#![allow(dead_code)]` (finding 3) suppresses the warning that would expose it.
Why it matters: TEST_CONVENTIONS §T7 — this is a rename-regression pin that pins nothing. `section_resize_none_clears_size` (section/mod.rs:1171) happens to cover the behavior, so nothing is currently unguarded, but the file carries a dead test that reads as coverage.
Fix: Put `#[test]` on its own line, or delete the fn as a duplicate (its name also references the removed `none` literal).
Effort: S

### 3. Crate-wide `#![allow(dead_code)]` defeats §5's no-dead-code covenant
Severity: P1 | Category: convention / dead-code | Confidence: high (verified: `cargo check -p mandala` emits zero warnings with confirmed-dead code present)
Files: src/main.rs:3; casualties found in this scope: src/application/console/traits/view.rs:608-611 (`read_edge_label`, zero call sites repo-wide), section/mod.rs:1355 (finding 2 produces no warning)
Evidence: `grep -rn read_edge_label src lib` -> only the definition; `cargo check` clean.
Why it matters: CODE_CONVENTIONS §5 ("no dead code"; "every merge commit is a state we would ship"). With the lint globally off in an ~83K-LOC AI-written crate, dead surface accumulates silently; two hits in this one module suggest more elsewhere.
Fix: Remove the crate-level allow; delete `read_edge_label`; fix finding 2; triage remaining warnings — deliberate seams get item-level `#[allow]` with a comment naming the future consumer (§7).
Effort: M
### 4. Per-verb scaffolding duplication — the same grammar hand-rolled 2-4 times per verb across 20 verbs
Severity: P1 | Category: duplication / architecture | Confidence: high
Files (recurring shape, >=5 verbs per element):
- Subverb match + unknown-subverb error: section/mod.rs:230-338 (with its own grouped-listing error at 270-281), border/execute.rs:24-104 + 132-142 (`unknown_subverb_message`), canvas.rs:198-262 + 469-519, font.rs:185-204, label.rs:66-123, mode.rs:66-99, node.rs:40-51, mutation.rs:79-91, zoom.rs:117-125, fps.rs:44-53, section/frame.rs:104-139.
- kv loop + unknown-key rejection: section/mod.rs:885-942 (`parse_move_kvs`), 977-1012 (`parse_resize_kvs`), 410-422 (`reject_unknown_kvs`); font.rs:211-272 (`parse_font_args`); zoom.rs:126-149; border/execute.rs:92-104; section/frame.rs:141-160; canvas.rs:250-262 + 507-519; label.rs:125-142; anchor.rs:107-127; cap.rs:102-121; edge.rs:107-156.
- Usage strings duplicated between `Command.usage` and inline error text, already drifting: section/mod.rs:59 vs :235 vs :922 vs :1007 (four variants); font.rs:61 vs :251-253 vs :502 (registry copy omits `section=`/`range=` that :502 documents); zoom.rs:44 vs :123 vs :145; color.rs:35 vs :293 (neither mentions `picker on|off` nor `section=`/`range=`); border/mod.rs:97-106 vs execute.rs:101.
- Per-key hint tables: font.rs:116-124, color.rs:66-74, section/mod.rs:214-228, border/execute.rs:735-748 (+ delegation shims frame.rs:95-97, canvas.rs:194-196), zoom.rs:73-79 inline.
- Per-verb `complete` matches re-encoding the execute grammar positionally: border/complete.rs:12-56, canvas.rs:87-168 (hand-maintained token-index arithmetic for the `focused` modifier at 122-156), section/mod.rs:75-138, font.rs:70-106, color.rs:42-64, zoom.rs:63-93, label.rs:43-64, mutation.rs:33-77.
- Finalize tails (six shapes): `ApplyTally::finalize` helpers.rs:118-127; `finalize_report` color.rs:493-508; `applied_or_no_change` commands/mod.rs:110-116; `fanout_size_outcome` font.rs:360-380; `aggregate_single_op` traits/dispatch.rs:169-192; border-family triple (finding 12).
Why it matters: §5 ("the answer is never to copy it") and §6 (the shape is plural — 20 verbs — and is hardcoded per verb). Every new verb re-pays the whole tax; every element is an independent drift channel, and drift has already happened (findings 13, 17, the usage variants above). The border family proves the target shape works: shared KEYS/VERBS consts + one `stage_kv` + one `kv_hint` + one `kv_value_completions` reused verbatim by `section frame` and `canvas` — but that sharing was hand-built for one family instead of being the framework.
Fix: A declarative per-verb arg-spec table alongside `Command` — subverbs (name, hint, handler), kv keys (name, hint, value-parser, value-vocabulary fn), positional slots — from which ONE engine generates: the kv parse loop with unknown-key rejection, Token/KvValue completion (incl. value vocabularies), the usage/help forms, and the hint surface. Bespoke semantics (`section move` mutual-exclusion, `border side` custom-gate) stay as hand-written handlers behind the table. Unifies parse+complete+help (quests 1, 4, 10) in one stroke.
Effort: L

### 5. `font`: verb path and Action core are two parallel selection-dispatch implementations
Severity: P1 | Category: duplication / ssot | Confidence: high
Files: src/application/console/commands/font.rs:278-349 (`apply_font_args`, verb path) vs 617-687 (`apply_font_kv_to_selection`, Action core)
Evidence: Both functions match `doc.selection.clone()` over all 10 SelectionState variants and route to the same setters (`set_node_font_size`, `set_section_font_size`, `set_edge_font`, `set_edge_label_font`, `set_portal_text_font`, `set_section_font_size_range`). `execute_font` never calls the core; the core never calls the verb helper. Behavioral drift already exists: on `Multi`/`MultiSection` with min/max the verb surfaces "min/max: <kind>s have no screen-space clamps" (font.rs:367-368) while the core silently returns false (font.rs:654-655).
Why it matters: CONCEPTS §6 claims "dispatch arms call pub(crate) mutation cores extracted from each console verb, so the same setter path runs" — the setters are shared, but the selection-routing layer (which is where the section/range/portal subtleties live) is duplicated. Contrast `zoom` (zoom.rs:204: verb routes through `apply_zoom_to_selection`, the exemplar) and `color` (both sides call `apply_kvs`, though the key->trait closure is byte-duplicated at color.rs:308-319 vs 450-461).
Fix: Rebuild `apply_font_args` on top of the core: apply min/max first then size via three core calls, or extend the core to accept the (size,min,max) triple and have the verb wrap it for messaging (mirror the zoom.rs shape). Deduplicate the color closure into a shared fn while there.
Effort: M

### 6. Section target resolution exists three times (plus a duplicated kv shim), kept in sync by comments
Severity: P1 | Category: ssot | Confidence: high
Files: src/application/console/commands/section/mod.rs:352-403 (`resolve_section_idx` + `resolve_node_id` + `parse_section_kv`); src/application/console/commands/section/frame.rs:456-508 (`parse_section_kv` byte-identical + `resolve_section_idx_for`); src/application/app/dispatch/cross_dispatch/style.rs:249-273 (`target_section`)
Evidence: frame.rs:467-472 doc: "Cascade matching `section/mod.rs::resolve_section_idx`'s rule table (kept in sync; review-fix CRIT-2 closed the divergence where this resolver lacked the rule-3 single-section auto-resolve...)" — i.e. the two verb-side resolvers already diverged once and were re-synced by hand. The Action-side `target_section` is a third implementation with intentionally different rules (rejects `Single` even on 1-section nodes; rejects MultiSection>1), documented at style.rs:224-248. `parse_section_kv` (the `section=K` extraction loop) is copy-pasted between section/mod.rs:396-403 and frame.rs:456-463.
Why it matters: §5. The rule table (CONCEPTS §5's SelectionState routing rules 1-5) is load-bearing UX; three hand-synced implementations means the next rule change is a three-site edit guarded only by comments. CRIT-2 proves the failure mode is real, not hypothetical.
Fix: One `resolve_section_target(selection, doc, kv_idx, policy)` in a shared module (e.g. commands/section/resolve.rs), where `policy` encodes the two legitimate divergences (doc-less Action path; multi-section acceptance). Delete the duplicate `parse_section_kv`.
Effort: M

### 7. canvas.rs re-implements the border positional machinery instead of sharing it
Severity: P1 | Category: duplication | Confidence: high
Files: src/application/console/commands/canvas.rs:303-467 (`positional_subverb_to_edits`) vs src/application/console/commands/border/execute.rs:254-595 (per-field positional appliers)
Evidence: Side-selector parsing duplicated (canvas.rs:350-360 vs border/execute.rs:501-515 `parse_side_selector`); corner-selector duplicated (canvas.rs:413-423 vs 586-595 `parse_corner_selector`); non-custom preset gate duplicated (canvas.rs:369-381 and 424-437 vs border 434-442 and 542-550); reset-restores-preset-glyph logic duplicated (canvas.rs:384-401 and 440-462 vs border 444-468 and 551-582); `preset cycle` resolution duplicated (canvas.rs:322-327 vs border 271-285). The kv-form staging loop appears a fourth time (canvas.rs:250-257, 507-514 vs border/execute.rs:92-99 vs frame.rs:141-154 — preview.rs:125-144 already extracted `stage_kv_for_preview` for the preview family, proving the extraction is easy).
Why it matters: §5; the Batch-6 "blocker" tests (canvas.rs:1158-1251) exist precisely because the copies drifted (canvas reset hardcoded "light", canvas silently auto-promoted while border errored). The fix landed as more parallel code rather than a shared core, so the same class of drift is still open for the next field.
Fix: Parameterize border's positional appliers over a target abstraction (`enum BorderSurface { Nodes(selection), CanvasSlot(CanvasSlot) }` supplying `current_preset()` and `apply(edits)`), and have canvas.rs call them; delete `positional_subverb_to_edits`.
Effort: M

### 8. Verb <-> Action single-core exceptions: two dispatch arms re-implement verb bodies; one documented divergence contradicts the verb's new contract
Severity: P1 | Category: ssot | Confidence: high
Files: src/application/app/dispatch/cross_dispatch/style.rs:30-59 (`apply_cycle_border_preset`) vs src/application/console/commands/border/execute.rs:190-208 (`first_selection_preset`) + 271-295; style.rs:63-89 (`apply_toggle_border_visible`) vs border/execute.rs:210-249 (`apply_toggle_visible`); style.rs:464-490 (`apply_split_section`, `at_grapheme = None` -> end-of-text) vs section/mod.rs:708-731 (verb REQUIRES `at=`, explicitly because the end-of-text default was ruled a foot-gun)
Evidence: `apply_cycle_border_preset` re-derives "first selected node's preset falling back to canvas default falling back to light" inline (style.rs:42-53) — byte-equivalent to `first_selection_preset`. `apply_toggle_border_visible` re-implements the read-flip-write loop of `apply_toggle_visible` (minus messaging). `apply_split_section`'s doc still says "Mirror of `section split [at=<grapheme>]` ... `None` defaults to end-of-text (empty suffix)" although the verb now rejects that default with a dedicated error (section/mod.rs:714-720) — so a macro `SplitSection { at_grapheme: "" }` silently does the exact thing the verb was hardened against.
Why it matters: These are the three spots where the CONCEPTS "one pub(crate) mutation core" claim does not hold. The remaining ~15 parametric arms verify clean (SetBorderField/SetColor/SetFontFamily/SetFont/SetZoom/ClearZoom/SetSpacing/SetEdgeAnchor/Cap/BodyGlyph/Type/DisplayMode/ResetEdge/SetEdgeLabelText/Position all route through the console cores — edges.rs:15-84, style.rs:16-24+175-222, camera.rs:172-194).
Fix: Export `first_selection_preset`-based cycle and the toggle loop as `pub(crate)` cores in border/execute.rs and call them from style.rs; either make the Action reject empty `at_grapheme` like the verb or update both docs to declare the divergence intentional (and add the missing warn-log).
Effort: S-M
### 9. File-lifecycle verbs have zero tests: `open`, `new`, `save`, `fps`
Severity: P2 | Category: testing | Confidence: high (verified: no `#[test]` in the four files; no `run("open`/`new`/`save`/`fps` anywhere in the suite)
Files: src/application/console/commands/open.rs (49 lines, 0 tests), new.rs (54, 0), save.rs (59, 0), fps.rs (54, 0)
Evidence: The dirty-guard ("unsaved changes; save before opening another map", open.rs:39-41; new.rs:39-41), the save-rebind semantics (save.rs:51-55: rebinding `file_path` + clearing `dirty`), and `new <path>`'s write-immediately behavior (new.rs:44-48) are all untested. Every other verb has between 4 and ~60 tests.
Why it matters: TEST_CONVENTIONS §T1/§T12 — the dirty-guard is the only thing standing between a typo'd `open` and silent loss of unsaved work; it is exactly "happy path plus each distinct error path" material. All four bodies are pure `ConsoleEffects` manipulation + `loader::save_to_file`, trivially testable with temp paths (no GPU).
Fix: Add tests: open-on-dirty rejects; open-unknown-path surfaces loader error; new-on-dirty rejects; `save <path>` rebinds and clears dirty; `save` with no bound path errors; `fps on|off|debug|garbage` side-effect emission.
Effort: S

### 10. Three hex-color validity grammars; the paste validator rejects values the rest of the app produces
Severity: P2 | Category: ssot / api-design | Confidence: high
Files: src/application/console/traits/color_value.rs:37-43 (accepts #rgb/#rgba/#rrggbb/#rrggbbaa); src/application/console/traits/view.rs:598-606 (`is_valid_color_literal`: accepts ONLY 6|8 digits + `var(--name)`); lib/baumhard/src/font/hex.rs:28-30 + `util::color_conversion::hex_to_rgba` (canonical: 3/4/6/8, optional '#')
Evidence: `color bg=#abc` is accepted and writes `"#abc"` into the model (ColorValue path). Copy that node's color and paste onto an edge -> `Outcome::Invalid("not a color: #abc")` (view.rs:600 requires len 6|8). Meanwhile baumhard's `hex_to_rgba` — the renderer's authority — accepts 3/4/6/8.
Why it matters: §1 ("color is Baumhard's — do not redefine in the app crate"): two app-side shape validators re-implement what a baumhard `is_valid_hex_color`/`hex_to_rgba(...).is_some()` check already defines, and they disagree with each other and with the renderer. Concrete UX: clipboard color round-trip fails for short-hex-styled documents.
Fix: Add/expose a validity helper in baumhard (`util::color`), and use it from both `ColorValue::parse` and `is_valid_color_literal`; widen the paste path to 3/4-digit forms (var(--x) handling stays app-side or also moves next to `resolve_var`).
Effort: S

### 11. `section=` / `range=` kv extraction + validation triplicated across color and font
Severity: P2 | Category: duplication | Confidence: high
Files: src/application/console/commands/color.rs:274-302 + 345-357; src/application/console/commands/font.rs:226-247 (parse_font_args) + 523-541 (execute_font_set) + 397-409 (pre-flight)
Evidence: Three copies of the extraction loop ("split out optional section=N and range=A..B"); three copies of the "range=A..B requires section=N — ranges target grapheme indices inside one section" error (color.rs:300, font.rs:241, font.rs:539-540); two copies of the `rs >= total` grapheme pre-flight with the same warn-comment (color.rs:345-357 vs font.rs:397-409, incl. near-identical log::warn wording at color.rs:390-396 vs font.rs:422-428). range_kv.rs already exists as the shared home for exactly this concern (its header cites §5) but only hosts the two leaf parsers.
Why it matters: §5; a semantic change to range targeting (e.g. permitting `range=` with a SectionRange selection) is a 3-site edit today.
Fix: Extend commands/range_kv.rs with `extract_section_range_kvs(args, verb) -> Result<(Option<usize>, Option<(usize,usize)>, Vec<(String,String)> or remaining), String>` plus the shared pre-flight; call from color + both font paths.
Effort: S

### 12. Border-family success/finalize tail copy-pasted three times
Severity: P2 | Category: duplication | Confidence: high
Files: src/application/console/commands/border/execute.rs:624-661 (apply_edits tail); src/application/console/commands/section/frame.rs:281-307 (apply_edits tail); src/application/console/commands/canvas.rs:604-633 (finish)
Evidence: All three implement: `changed == 0` -> bare_custom? two-line hint : "<label>: no change"; else "<label> applied/updated ..." + optional auto-promoted note (same 4-line string) + optional custom_preset_hint + `lines.len()==1 -> Ok else Lines`. preview.rs:194-217 (`finish_preview`) is a fourth sibling with the same skeleton. The team already unified `custom_preset_hint`/`edits_has_glyph_field` per §5 (border/mod.rs:51-57 says the prior 3 copies "violated CODE_CONVENTIONS.md §5") — the surrounding tail was left copied.
Why it matters: §5; the auto-promote note's wording already varies ("per-node" vs "per-section" vs "per-canvas" vs "per-target" glyph override) in a way that would be a parameter in a shared fn.
Fix: One `finish_border_edit(label, changed_count_or_flag, auto_promoted, bare_custom) -> ExecResult` in border/execute.rs, reused by all four.
Effort: S

### 13. `section` verb: subverb vocabulary duplicated as two consts in the same file
Severity: P2 | Category: ssot | Confidence: high
Files: src/application/console/commands/section/mod.rs:50-52 (`pub const VERBS`, drives completion) vs 267-269 (`const KNOWN_VERBS`, drives execute validation)
Evidence: Same 9 entries, different order ("add" last in one, mid-list in the other). A new subverb added to one list but not the other either becomes invisible to completion or is rejected at runtime with "unknown subverb" while completion offers it — the compiler cannot catch it.
Why it matters: §5; this is the in-file miniature of finding 4, and the grouped unknown-subverb error text at 271-281 is a third redundant encoding of the same vocabulary.
Fix: Validate against `VERBS` (they are identical sets); derive the grouped listing from a (verb, group, hint) table shared with `verb_hint`.
Effort: S

### 14. Parametric-Action failure feedback is inconsistent: some invalid payloads warn, `SetBorderField` is fully silent
Severity: P2 | Category: error-handling | Confidence: high
Files: src/application/console/commands/border/execute.rs:676-697 (`apply_border_field_to_selection`: `stage_kv` error -> `return false`, no log); contrast style.rs:109-111 (`apply_set_border_preview` logs stage_kv errors), action_core.rs:404-408 + 425-427 (SetFont/SetZoom warn on bad payload), color.rs:475-489 (`log_not_applicable_if_silent` — logs NotApplicable but not Invalid values)
Evidence: A `keybinds.json` entry `set_border_field: [{combo: "Ctrl+B", args: ["preset", "hevy"]}]` fires, stages fail, returns false — no scrollback (none exists on this path), no log line. CONCEPTS §6 states "wrong arg counts emit a warn-log and are skipped (never panic)" — arg-count errors do warn, but invalid values on this arm (and invalid color VALUES on SetColor) vanish.
Why it matters: §9 posture is "degrade + log"; a silently-dead keybinding is the kind of failure users cannot self-diagnose (the CONCEPTS X2 rationale for `log_not_applicable_if_silent` applies verbatim here).
Fix: In `apply_border_field_to_selection`, log the `stage_kv` error at warn level (mirroring apply_set_border_preview); extend `log_not_applicable_if_silent` (or a sibling) to also surface Invalid messages on the Action path.
Effort: S

### 15. Grapheme work bypasses/duplicates baumhard primitives in four places
Severity: P2 | Category: convention (§1) / duplication | Confidence: med
Files: src/application/console/commands/section/mod.rs:180-212 (preview <=20 graphemes; iterates `graphemes(true)` twice — take(20) then count()) and 464-480 (preview <=40, single-pass variant with a comment explaining why the 2-pass shape was bad); src/application/app/console_input/edit.rs:169-195 (`kill_word` collects `Vec<&str>` of all clusters before cursor); src/application/app/console_input/completion.rs:111-127 (`accept_console_completion` collects `Vec<&str>`)
Evidence: grapheme_chad has no "first N graphemes" / boundary-scan primitive, so call sites hand-roll it — and the two preview truncations in the same file solved the same problem twice, differently (the 20-cap version still does the double-walk the 40-cap version's comment condemns). `kill_word` re-implements a boundary walk with the exact `Vec<&str>`-over-prefix allocation that `grapheme_chad::word_left`'s doc (lib/baumhard/src/util/grapheme_chad.rs:430-443) says it was created to eliminate (semantics differ: whitespace-delimited vs alphanumeric-word — a `word_left_ws` sibling, not a drop-in).
Why it matters: §1: "Missing primitives are added to Baumhard, not to src/application/". Per-keystroke path (`accept_console_completion`) allocates a Vec of slices each Tab; previews run per `section show`/completion popup row.
Fix: Add `grapheme_chad::take_graphemes(&str, n) -> (&str, bool /*truncated*/)` and a whitespace-boundary `word_left_ws`; migrate the four sites (kills the intra-file preview duplication too).
Effort: M

### 16. Monolith audit: section/mod.rs (2101), canvas.rs (1377), font.rs (1258)
Severity: P2 | Category: architecture (§6) | Confidence: high
Files: as named
Evidence / concepts jammed together:
- section/mod.rs = registration+completion (~230 LOC: 49-228) + verb routing incl. CRIT-1 ordering rules (230-338) + target resolution (340-422) + geometry subverbs with MultiSection fan-out (742-1012) + structure subverbs add/delete/split (639-740) + text/edit subverbs (543-637) + readout (424-541) + ~1090 LOC of tests (1014-2101). Five named concepts, one file; the resolver that frame.rs must mirror (finding 6) is buried mid-file.
- canvas.rs = completion (87-196) + subject dispatch (198-262, 469-519) + the duplicated positional machinery (finding 7, 303-467) + previews (528-571) + finalize (573-633) + shows (635-747) + ~620 LOC tests in four modules. It is `border/` flattened into one file — border itself already demonstrates the right split (mod/complete/execute/preview/show/tests).
- font.rs = completion (70-166) + kv parse (168-272) + verb dispatch + outcome formatters (274-486) + `font set` (489-610 with its own section/range handling) + two Action cores (592-687) + `font list` (689-702) + ~550 LOC tests. The size is a symptom of finding 5's dual dispatch more than of missing file splits.
Why it matters: §6 "split by conceptual boundary... monolith files that have outgrown their concept". Roughly half of each file is tests (fine inline per §T2.1), but the non-test halves each mix 4-5 load-bearing concepts, and both cross-file duplications (findings 6, 7) trace back to these files not exposing their concepts as modules.
Fix: section/ -> mod.rs (COMMAND + routing), complete.rs, resolve.rs (shared with frame.rs), geometry.rs (move/resize), structure.rs (add/delete/split/text/edit/show), tests.rs. canvas.rs -> canvas/ mirroring border/'s layout, with execute delegating to the shared border cores (finding 7). font.rs: fold the verb onto the cores (finding 5); a file split is then optional.
Effort: L

### 17. Documentation drift cluster (user-facing + CONCEPTS)
Severity: P2 | Category: docs | Confidence: high
Files/Evidence:
- CONCEPTS.md §6 "Console" (line ~2455): "Verbs include ... `portal` ... `quit`" — neither exists in `COMMANDS` (commands/mod.rs:70-91; tests/commands.rs:612-614 confirms `portal` was folded into `edge`; `quit` appears only in a doc-comment example at console/mod.rs:153). CONCEPTS also omits `border`/`canvas`/`section`/`node`/`mode`/`help`, the largest verbs in the module.
- console/mod.rs:296-298: `completions` field doc says "Populated lazily on Tab" — actually recomputed on every input change (dispatch.rs:273-275, completion engine doc agrees); `completion_idx` doc "Some(idx) after Tab" — actually `Some(0)` on every recompute (console_input/completion.rs:46).
- console_input/completion.rs:12-14 fn doc says highlight defaults to "the bottom row" while the code + inline comment set/say first row (`Some(0)`, :42-46).
- Registry usage strings missing real surface: color.rs:35 omits `picker on|off`, `section=`, `range=`; font.rs:61 omits `section=`/`range=` (its own error at :502 documents them). `help color` / `help font` therefore under-document the verbs.
- color_value.rs:30 doc: accepts "accent/edge/fg/bg" — no `bg` arm exists (:45-51) and no `VAR_BG` constant; `color bg=bg` errors "unknown color".
Why it matters: §8 (docs are load-bearing here — help text IS the product surface); CONCEPTS is the mandated orientation doc and it names verbs that do not exist.
Fix: Regenerate the CONCEPTS verb list from `COMMANDS`; fix the three stale field/fn docs; extend the two usage strings (or derive usage from the arg-spec table per finding 4); either implement `bg` as a known var or fix the ColorValue doc.
Effort: S
### 18. Mangled doc comments and dangling plan references throughout the section/border/canvas files
Severity: P3 | Category: docs | Confidence: high
Files/Evidence (redaction artifacts where a plan-reference was stripped mid-sentence):
- section/mod.rs:341 "offset.§906-920 selection rules:", :348 "only one option.///    rule 3 (line 914): closes the §5.7 hostile error." (glued `///`), :619 "target.Routes through", :682 "delete_section.Errors", :700 "split_section.`at=` is now", :743-744 "kv form replaces the pre-Batch-5 positional `<dx> <dy>`/// — no compatibility shim", :945, :1046, :1058, :1292, :1354 (which caused finding 2).
- canvas.rs:129 "mirror the per-node `border` verb'swork.", :227 "keybinds, per).", :321 "(acknowledged the gap)", :450-451 "butinteractive paths".
- border/mod.rs:66 "completions.added the per-field positional subverbs"; border/show.rs:21 "—/ §5.3.", :31 "—calls this out as a UX bug bake-in".
- style.rs:342, :381, :415, :445, :467: "macro path.`Action::X`. Destructive." (glued sentences).
- Dangling file references: section/mod.rs:7 and mode.rs:30 cite `SECTIONS_BORDERS_RESIZE_PLAN.md`; action_core.rs:25 and cross_dispatch/mod.rs:18 cite `WASM_CONVERGENCE.md` — neither file exists in the repo (verified `ls *.md`).
Why it matters: §8 ("inline comments explain why") — these read as review-round shrapnel; one of them physically broke a test (finding 2), proving the class is not cosmetic. Dangling doc pointers send readers to nonexistent files.
Fix: Sweep the listed sites; repair or delete the sentences; replace dead plan references with self-contained rationale (the repo's own convention after plan files are removed).
Effort: S

### 19. British spellings in user-facing strings (project mandates American English)
Severity: P3 | Category: convention (CLAUDE.md §6) | Confidence: high
Files/Evidence (user-visible strings only; comments are a longer tail):
- "not recognised": section/mod.rs:589 (error), action_core.rs:474 (warn-log), section/mod.rs:1639 (test pins the British spelling).
- "internal: unrecognised corner": border/execute.rs:566, :574; canvas.rs:454 (errors).
- "cancelled": border/preview.rs:184 (success line "border preview cancelled").
- "colours"/"colour": border/execute.rs:741 (completion hint "cycle per-glyph colours"), color.rs:296 (error "requires at least one colour axis").
Why it matters: CLAUDE.md §6 "Use American English for consistency". These render in the console scrollback / completion popup.
Fix: recognized / unrecognized / canceled / colors / color; update the pinned test string at section/mod.rs:1639 in the same commit.
Effort: S

### 20. Identity re-map ceremony in completion recompute
Severity: P3 | Category: dead-code | Confidence: high
Files: src/application/app/console_input/completion.rs:32-41
Evidence: `complete_console` returns `Vec<console::completion::Completion>`; the code then `.into_iter().map(|c| Completion { text: c.text, display: c.display, hint: c.hint, font_family: c.font_family })` into the SAME type. `*completions = new;` suffices. Leftover from a since-unified duplicate struct.
Why it matters: §5 polish; per-keystroke path (harmless cost, misleading shape — implies a type boundary that no longer exists).
Fix: Replace with direct assignment.
Effort: S

### 21. Small duplication / polish nits
Severity: P3 | Category: duplication / polish | Confidence: high
Files/Evidence:
- zoom.rs:185-203 selection->noun label mapping duplicates `targets_kind_label` (traits/dispatch.rs:194-221) minus pluralization.
- traits/view.rs:762-777 and :858-868: identical hardcoded fallback `TextRun` template ("LiberationSans", 24pt) built twice in the same file (cut vs paste range paths).
- font.rs:439 (`section_font_outcome`): message reads "min/max: nodes have no screen-space clamps" even when the target is a section (doc admits "the surface message is the same"; the noun is wrong for the section path).
- commands/mod.rs:94-98 `command_by_name`: allocates `name.to_ascii_lowercase()` and then ALSO uses `eq_ignore_ascii_case` — one of the two suffices; runs on every keystroke via `complete()` (trivial cost, redundant shape).
- traits/view.rs:460: `paste_edge_adjacent_color(self, &content)` — `&content` on an already-`&str` binding (double reference, auto-derefed).
Why it matters: §5 drive-by-fix culture; each is a one-liner.
Fix: As listed; for the zoom label reuse `targets_kind_label` by exposing it `pub(super)`.
Effort: S

---

## Quest-by-quest summary

1. Copy-paste audit: finding 4 (master), 7, 11, 12, 13. The border family is the internal proof that a shared framework works.
2. Verb<->Action SSOT: CONCEPTS claim verified TRUE for ~15 parametric arms (SetBorderField, SetBorderPreview/Commit/Cancel, SetColor, SetFontFamily, SetFont, SetSpacing, SetZoom, ClearZoom, SetEdgeAnchor/BodyGlyph/Cap/Type/DisplayMode/ResetEdge, SetEdgeLabelText/Position — all via cross_dispatch calling console cores). Exceptions: finding 8 (CycleBorderPreset, ToggleBorderVisible re-implemented; SplitSection contract divergence) and finding 5 (font's routing layer duplicated even though the setters are shared). `section` verbs vs trait dispatch: styling axes (color text / font size/family / clipboard / wheel) go through ONE path (TargetView); geometry/structure section verbs are a SECOND path with THREE resolver implementations (finding 6). Section-Action arms call document setters directly with documented divergences (style.rs:224-248).
3. Monoliths: finding 16.
4. Completion vs parse duplication: yes — two sources of truth per verb, held together by shared consts in the best cases (border) and by nothing in the worst (finding 13); the arg-spec proposal in finding 4 unifies them.
5. kv parsing: tokenize/split_kv single-sourced (parser.rs — good). Number parsing: `parse_finite_pt` shared by font/border/zoom-adjacent paths (helpers.rs:137-146 — good), but f64 section geometry re-implements parse+finite checks twice (parse_move_kvs/parse_resize_kvs, folded into finding 4) and zoom keeps an `unset`-aware sibling (justified). Hex: THREE grammars (finding 10). var(): recognized in ColorValue (named vars only), is_valid_color_literal (full var(--x) syntax), resolve_var (baumhard, authoritative) — finding 10. section=/range=: findings 6, 11.
6. traits/view.rs: sound design — one enum, 8 capability traits, aliasing-safe per-iteration view_for, shared edge-adjacent helpers (write_edge_adjacent_color). Dead surface: `read_edge_label` (finding 3); the range cut/paste helpers carry the duplicated TextRun template (finding 21). Implementors: the single TargetView enum; consumers: color/font/label verbs, clipboard Action arms, wheel commit.
7. Error posture: consistent Ok/Err/Lines into scrollback; no panics on user input anywhere in the scope (defensive arms at section/mod.rs:330-335, zoom.rs:196-202, border corner arms all log::error + Err instead — §9 compliant). Gap: Action-path silent failures (finding 14).
8. Grapheme discipline: cursor math is grapheme-indexed end to end (edit.rs uses grapheme_chad exclusively for cursor ops; renderer clips via truncate_to_display_width at console_pass.rs:248,313,341). Completion converts grapheme cursor -> byte via find_byte_index_of_grapheme before byte-slicing (completion.rs:30, correct). Remaining direct unicode_segmentation sites: finding 15. History/scrollback: clamped offsets, saturating arithmetic, wheel accumulator NaN-guarded (edit.rs:109-122) — clean.
9. Test coverage: zero-test verbs = open/new/save/fps (finding 9); mutation completion untested; everything else covered, several verbs heavily. clipboard.rs (802) and border/tests.rs (1258) are HIGH signal: real model assertions, undo round-trips, exact-message pins, contract pins between verb-strict and macro-permissive paths (border/tests.rs:1236-1258), Unicode edge cases (traits/tests.rs:169-185). One dead test (finding 2).
10. Help drift: usage strings live next to the verb (good locality) but are free-form and already drifted (finding 17); `mutation help/inspect` are registry-driven (cannot drift — good); `help` splitter is depth-aware and well-tested (help.rs:64-90, 267-289).

## Checked and CLEAN

- Tokenizer (parser.rs): quote/escape semantics precise, documented, tested including unterminated-quote and `=raw` escape hatch; kv split single-sourced; completion reuses it.
- The §3 dispatch-funnel carve-out: console keystrokes route key->Action::Console* -> dispatch_action -> dispatch_console_action (one funnel body for keys AND macros); literal character insertion is the documented modal-steal carve-out (dispatch.rs:56-76).
- apply_kvs / apply_to_targets aggregation: unknown-key reported once per pair (short-circuit), Invalid > Applied > Unchanged > NotApplicable priorities, empty-selection report shared — all pinned by tests (tests/apply_kvs.rs, traits/tests.rs:292-384).
- Grapheme cursor invariants in the line editor (ZWJ family delete, cluster counting) — tested (tests/grapheme.rs, edit/tests.rs).
- Scrollback model: offset clamped at read time against MAX_CONSOLE_SCROLLBACK_ROWS, reset-to-bottom on input/output, wheel fractional accumulator with non-finite guard.
- Border preview lifecycle (stage/commit/cancel, four surfaces via one dispatch_border_preview with per-verb target resolvers + subverb_pos) — genuinely shared, tested per surface.
- No unwrap()/expect() on user input in the scope; every `expect` guards a just-checked invariant (e.g. `len==1`).
- selection_targets fan-out (Multi, MultiSection, SectionRange with range threading) — single-sourced and tested.
- Console history persistence: best-effort, error-swallowing with warn logs, MAX_HISTORY trimming on both in-memory copies and disk (dispatch.rs:330-341, history.rs).
- Completion performance posture: recompute is per-keystroke but allocation volume is bounded by candidate count at human typing rates; native-only surface today (documented deliberate) — acceptable per §4 budget. No per-frame console work.
- Case-insensitivity policy: subverbs/preset names/command lookup consistently case-insensitive (C14 fixes applied across border/canvas/frame paths).
- WASM gating: verb implementations cfg-free; only the modal shell (console_input/) is native-gated at the module boundary (mod.rs:8) — matches the documented deliberate decision.
