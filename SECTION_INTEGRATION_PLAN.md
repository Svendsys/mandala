# Section Integration — Tier 2A plan & tracker

> **Living document.** This file is a tracker for a multi-session
> initiative. Update the status table as items land. The audit findings
> at the bottom stay frozen so future sessions can see the original
> baseline. Created on branch `claude/audit-section-integration-7bZ7U`.

## Scope (decided)

- **Tier 2A** — close the silent-collapse holes where the trait
  dispatcher and color picker route a `Section` selection through
  whole-node setters. No new gestures, no new data fields.
- Tier 2B (drag/resize/structured-clipboard) and Tier 2C
  (multi-section selection / manual node resize / auto-fit shrink /
  per-grapheme range targeting / insert-section paste) are **deferred**
  — captured at the bottom for future iterations.
- For the open question on picker bg/border axes against a `Section`
  selection: **return `NotApplicable`**, consistent with the existing
  `color bg= section=K` verb arm that already returns NotApplicable
  (`commands/color.rs:275-280`).

## Status

Legend: ✅ shipped · 🔧 in progress · ⏳ to do · ❌ deferred (out of 2A)

| # | Item | Status |
|---|---|---|
| 0 | Commit this plan file to the repo at `SECTION_INTEGRATION_PLAN.md` | ✅ |
| 1 | `HasTextColor::set_text_color` honours `Section` → `set_section_text_color` | ✅ |
| 2 | `HasBgColor::set_bg_color` returns `NotApplicable` for `Section` | ✅ |
| 3 | `HasBorderColor::set_border_color` returns `NotApplicable` for `Section` | ✅ |
| 4 | `AcceptsWheelColor::apply_wheel_color` routes `Section` through `set_section_text_color` for `Text` axis, `NotApplicable` for `Bg`/`Border` | ✅ |
| 5 | `AcceptsFontFamily::set_font_family` honours `Section` → `set_section_font_family` (wires the dead setter) | ✅ |
| 6 | `ColorTarget::Section { node_id, section_idx, axis: SectionColorAxis }` variant added to `color_picker/targets.rs` | ✅ |
| 7 | `picker_target_for` in `commands/color.rs` emits `ColorTarget::Section` for `Section` selections | ✅ |
| 8 | `current_color_at` for `Section` reads the resolved per-section text colour (with cascade fallback to `node.style.text_color`) | ✅ |
| 9 | Standalone-mode wheel commit (`app/color_picker_flow/commit.rs`) honours `Section` target | ✅ |
| 10 | `apply_font_kv_to_selection` Section arm in `font.rs` routes through `set_section_font_size` (Action-path lag fix) | ✅ |
| 11 | Tests added mirroring existing pinned shapes (see Verification) | ✅ |
| 12 | `./test.sh` clean (2004 tests pass + WASM `wasm32-unknown-unknown` type-check clean) | ✅ |
| 13 | `./test.sh --lint` clean (clippy errors fixed; pre-existing fmt drift in `crates/maptool` and parts of `lib/baumhard` is advisory and untouched) | ✅ |
| — | Out-of-scope cleanup unblocked by Item 13: derive `PartialEq` on `OrderedVec2` (`lib/baumhard/src/util/ordered_vec2.rs`); replace `<= 0` with `== 0` on two `u32` guards in `src/application/renderer/mod.rs`. Both pre-existed on `main`; flagged here for the audit trail. | ✅ |
| R1 | **Review fix-up (post-Tier-2A):** read/write asymmetry in `set_section_text_color` — write predicate now matches the picker's read cascade so a section whose runs unanimously carry a non-default colour is rewritable from the picker / kv path (pre-fix the write looked only for runs matching `node.style.text_color` and silently no-op'd, leaving the picker to close with no visible change). Pinned by `color_text_section_rewrites_unanimous_non_default_runs` in `commands/color.rs::tests`. | ✅ |
| R2 | **Review fix-up:** stale doc comments on `TargetView::Section` and `selection_targets` (`console/traits/view.rs`) refreshed to reflect post-Tier-2A trait dispatch (color/font route per-section; bg/border/zoom return NotApplicable). | ✅ |
| R3 | **Review fix-up:** four near-identical inline copies of the multi-section node scaffold (commands/color, commands/font, console/tests/wheel_dispatch, color_picker/tests/targets) collapsed to a single shared helper `make_two_section_node_with_pinned_runs` in `document/tests_common.rs`. The pre-existing inline copies in `color_text_section_kv_targets_specific_section` and `font_size_section_kv_targets_specific_section` were folded in too. | ✅ |
| R4 | **Review fix-up:** `font_family_action_section_writes_through_section_setter` added — direct `apply_font_family_to_selection` Action-path pin on a Section selection, sister to the Item-10 font-size pin. Coverage was previously transitive through the verb path only. | ✅ |
| R5 | **Review fix-up:** `picker_target_for_section_text_emits_section_target` test now `assert_exec_ok`s on the dispatcher result; previously discarded the `ExecResult` so a regression where the picker opens AND surfaces an error (mixed signal) would have slipped past. | ✅ |
| R6 | **Tier 2A.5 — A1: picker preserves `var(--name)` when HSV unchanged.** `PickerMode::Contextual` now carries `seed_var_ref: Option<String>` and `seed_hsv: (f32, f32, f32)` captured at open. `commit_color_picker` writes the seed reference verbatim when bit-exact `(hue_deg, sat, val)` equality says the user never moved the wheel — closes the seam where the picker would seed at `var(--accent)` and silently rewrite to its resolved hex on commit. Pinned by `picker_commit_preserves_var_ref_when_unchanged`, `..._overwrites_var_ref_when_hue_moved`, `..._writes_hex_when_no_var_ref` (`commit.rs::tests`). | ✅ |
| R7 | **Tier 2A.5 — A4: var-preserve symmetry test pin** (`color_text_section_preserves_var_ref_round_trip` in `commands/color.rs::tests`). Verb-side `color text=accent` on a `SelectionState::Section` writes the literal `var(--accent)` string into the section's runs — defensive pin against any future regression that resolves the var early at the verb layer. | ✅ |
| R8 | **Tier 2A.5 — A2: Action-path NotApplicable visibility.** `apply_color_axis_to_selection` now emits a `log::info!` line (with the dispatcher's per-target messages) when every target reports `Outcome::NotApplicable`, so a keybind for `Action::SetColor { axis: Bg \| Border }` against a `Section` selection has *some* feedback in the log — Action arms have no scrollback. Pinned by `apply_color_axis_logs_when_all_targets_not_applicable`. Font's parametric Action helper (`apply_font_kv_to_selection`) was deliberately not changed: its NotApplicable cases (`min`/`max` on a node/section) match the verb-path's silent-false documented behaviour. | ✅ |
| R9 | **Tier 2A.5 — A3: Standalone picker selection-identity title hint.** `ColorPickerOverlayGeometry` carries `selection_hint: Option<String>`; `rebuild_color_picker_overlay` populates it from `doc.selection` when the picker is in Standalone mode (`"section K of <node>"` / `"node <id>"` / `"{count} nodes"` / `"edge"` / `"(no selection)"`). The Standalone title bar now reads `"࿕ color palette · <hint>"` so a wheel commit's target is visible at a glance. Contextual mode unchanged — `PickerHandle::label` already labels its bound target. | ✅ |
| R10 | **Tier 2A.5 — Audit gap 2 pin.** `section_paste_collapses_runs_inheriting_first_run_template` (`console/tests/clipboard.rs`) pins the documented lossy behaviour of `set_section_text` on the unstructured paste path: pasting plain text into a multi-run section collapses to one run that inherits the first original run's `font` / `size_pt` / `color` / `bold`. Reduces regression risk when Tier 2B's structured `ClipboardContent::Section` payload lands. | ✅ |
| R11 | **Tier 2A.5 — A5: `format/sections.md`** updated with the new picker var-preserve semantics (bit-exact HSV equality is the "did the user touch it?" signal). | ✅ |
| — | Tier 2A.5 — Audit gaps 1 & 3 closed without code: gap 1 (no `HasFontSize` trait — font size dispatch goes through `apply_font_kv_to_selection` directly, already covered by Item 10); gap 3 (picker-open path on Section + bg/border returns NotApplicable — already pinned by `picker_target_for_section_bg_returns_not_applicable_message`). | ✅ |
| B1 | **Tier 2B-clipboard — `ClipboardContent::Section { text, payload }` variant + `SectionPayload`** struct (text_runs, offset, size, channel, trigger_bindings) in `console/traits/outcome.rs`. The `text` rides the OS clipboard; the `payload` rides the in-process structured buffer for within-app section→section round-trip. | ✅ |
| B2 | **Tier 2B-clipboard — In-process structured buffer** in `application/clipboard.rs`: `static SECTION_BUFFER: Mutex<Option<SectionBufferEntry>>` + `write_section_clipboard` + `read_section_clipboard(probe_text)`. Read returns the buffered payload only when `probe_text` matches the buffer's text snapshot — guards against the user copying from another app between Mandala copy and paste (buffer self-invalidates). | ✅ |
| B3 | **Tier 2B-clipboard — `apply_section_payload`** atomic document setter in `document/nodes/mod.rs`: replaces text + runs + offset + size + channel + bindings under a single `EditNodeStyle` undo entry, so a single Ctrl+Z restores the full pre-paste shape. Triggers the same monotonic `grow_one_node_to_fit_text` / `_border` floor as the other section setters. | ✅ |
| B4 | **Tier 2B-clipboard — `HandlesCopy` for `Section`** emits `ClipboardContent::Section { ... }` via `SectionPayload::from_section`. Was `Text` / `Empty` only. | ✅ |
| B5 | **Tier 2B-clipboard — `HandlesPaste` for `Section`** consults `read_section_clipboard(content)` first; on hit calls `apply_section_payload` (per-run formatting + section chrome preserved); on miss falls back to today's `set_section_text` (template inheritance — pinned by `section_paste_collapses_runs_inheriting_first_run_template`). The stale-`section_idx` clamp survives both branches. | ✅ |
| B6 | **Tier 2B-clipboard — `HandlesCut` for `Section`** snapshots the structured payload, then clears text + runs only (offset / size / channel / bindings stay on the source section so the cut reads as "the text disappeared" rather than "the section dissolved"). Pairs with the structured paste so cut→paste round-trips full shape. | ✅ |
| B7 | **Tier 2B-clipboard — `apply_copy_or_cut`** in `cross_dispatch/lifecycle.rs` dual-writes for the `Section` variant: plain text to the OS clipboard via `write_clipboard`, structured payload to the in-process buffer via `write_section_clipboard`. Cross-app paste sees plain text; within-app paste sees the structured payload. | ✅ |
| B8 | **Tier 2B-clipboard — Tests:** `section_copy_emits_structured_payload`, `section_paste_with_matching_buffer_preserves_runs`, `section_paste_with_mismatched_buffer_falls_back_to_plain`, `section_cut_emits_structured_payload_and_clears_text_runs_only`, `apply_section_payload_round_trips_through_undo`. The two pre-existing Tier 2A.5 paste tests (`section_paste_collapses_runs_inheriting_first_run_template`, `section_paste_clamps_stale_idx_to_last_section`) gained a `section_clipboard_test_guard` so they don't race with the new structured tests on the shared `SECTION_BUFFER` global. | ✅ |
| B9 | **Tier 2B-clipboard — Lint debt unblocked:** added `PartialEq` to `Position`, `Size` (`lib/baumhard/src/mindmap/model/node.rs`), and `TriggerBinding` (`lib/baumhard/src/mindmap/custom_mutation/mod.rs`) so `SectionPayload` and `ClipboardContent` can derive `PartialEq` cleanly. All three are pure data structs; the derive matches what hand-written impls would produce. | ✅ |
| B10 | **Tier 2B-clipboard review fix-up — C1 (CRITICAL):** the paste path's `content.trim_end()` was probed against the buffer, but the buffer was written with the untrimmed `section.text`. Sections whose text ended in `\n` (trivially produced by the inline editor's Enter key) silently fell through to the lossy plain-text branch. Probe with the untrimmed `content` so the consistency check is honest. Pinned by `section_paste_buffer_match_survives_trailing_newline`. | ✅ |
| B11 | **Review fix-up — C2:** `apply_section_payload` calls `clamp_runs_to_text(section)` after the field assignment so a future caller passing mismatched `(text, runs)` doesn't leave out-of-bounds runs. The copy site never produces such input today; defensive only. | ✅ |
| B12 | **Review fix-up — C3+X1:** widened `apply_section_payload`'s change-detection predicate to compare every field (text + runs + offset + size + channel + bindings). The pre-fix predicate compared text + runs only, so a paste that changed only chrome silently no-op'd against the atomic-setter promise. Now-redundant comment about "PartialEq derives ... none of which carry it today" dropped (B9 derived them). | ✅ |
| B13 | **Review fix-up — V3:** `SectionPayload` moved from `console/traits/outcome.rs` to `document/nodes/mod.rs` (re-exported from `document::`). The trait layer now imports it via the document layer. `apply_section_payload`'s 8-parameter signature collapsed to `(node_id, section_idx, text, &SectionPayload)`. Cuts call sites to two lines. | ✅ |
| B14 | **Review fix-up — V4:** the in-process `SECTION_BUFFER` migrated from `static Mutex<Option<…>>` to `thread_local! { static … : RefCell<Option<…>> }`. The editor's event loop is single-threaded; the previous `Mutex` doc-comment conceded this but claimed concurrency safety the rest of the architecture doesn't honour. `thread_local` is honest and gives parallel `cargo test` workers separate slots automatically. | ✅ |
| B15 | **Review fix-up — V5+V6:** `section_clipboard_test_guard` removed (unnecessary now that `thread_local` provides cross-thread isolation). `clear_section_clipboard_for_tests` survives for within-thread isolation across `cargo test`'s reused worker threads; tests that need a clean buffer call it explicitly. | ✅ |
| B16 | **Review fix-up — V2:** the six near-identical `MindSection::new_default(...) + text_runs = vec![TextRun {...}]` test scaffolds in `console/tests/clipboard.rs` collapsed to use the shared `make_two_section_node_with_pinned_runs` helper (extracted in Tier 2A.5 row R3). | ✅ |
| B17 | **Review fix-up — V1:** aggressive comment trim in `view.rs`, `lifecycle.rs`, `outcome.rs`, `clipboard.rs`, and `clipboard.rs::tests`. Dropped inline "Tier 2B" / "pre-Tier-2B-clipboard" / "Pre-fix" plan-tracker references (CLAUDE.md rule); shrunk multi-paragraph "what" comments to single-sentence "why" comments. | ✅ |
| B18 | **Review fix-up — X2:** `CONCEPTS.md §6 Clipboard` extended with the Section variant + the `SECTION_BUFFER` mechanism and consistency-check semantics; the stale `clipboard.rs:1-40` line range citation refreshed to the current shape. | ✅ |
| B19 | **Review fix-up — V7+V8:** plan tracker line 111 ("per-run / offset / size / channel fidelity is Tier 2B") replaced with a forward reference to B1-B9. `format/sections.md`'s structured-clipboard subsection rewritten to drop the implementation reference (`apply_section_payload`) and to mention the no-trim-on-either-side semantics that close the trailing-newline round-trip case. | ✅ |
| B20 | **Review fix-up — cosmetic:** `apply_copy_or_cut` gains a one-line comment explaining why `break`-on-first is correct (selection_targets emits at most one clipboard-eligible target per shape today; `Multi` is node-only). | ✅ |
| G1 | **Tier 2B-setters — `set_section_offset(node_id, idx, x, y) -> Result<bool, String>`** (`document/nodes/mod.rs`). Validates against the same rules `crates/maptool/src/verify/sections.rs` enforces — finite, non-negative, AABB-contained when size is `Some`. Rejection messages byte-equal to verify's. Single `EditNodeStyle` undo entry. Pinned by 6 tests in `tests_nodes.rs` (writes + undo, idempotent no-op, NaN/inf reject, negative reject, AABB-overflow reject, unknown-section returns Ok(false)). | ✅ |
| G2 | **Tier 2B-setters — `set_section_size(node_id, idx, Option<Size>) -> Result<bool, String>`** (`document/nodes/mod.rs`). Same validation surface (finite, strictly positive, AABB-contained, not 100× node — typo guard); `None` always valid (revert to fill-parent). Pinned by 6 tests (writes + undo, None resets, zero/negative reject, overflow reject, astronomical reject, idempotent no-op). | ✅ |
| G3 | **Tier 2B-setters — verb-side `Outcome::Invalid` messaging** mirrors `verify::sections::check`. Implemented as `Result<bool, String>` returns from G1/G2 that the verb file translates to `ExecResult::err`. Unified message body so a verb-rejected edit and a `verify` violation read identically. Covered transitively in G4/G5 verb tests. | ✅ |
| G4 | **Tier 2B-setters — console verb `section move <dx> <dy> [section=<idx>]`** in new file `console/commands/section.rs`. `dx`/`dy` are positional `f64`s (deltas relative to current offset). The `section=<idx>` kv is required when the active selection is `Single` (no implicit default); a `Section` selection supplies the index. Pinned by 7 tests (writes via section selection, kv overrides selection, single-selection without kv rejected, overflow reject with verify-mirror message, negative-offset reject, dx parse error, no-change ok-msg). | ✅ |
| G5 | **Tier 2B-setters — console verb `section resize <w> <h> [section=<idx>]` and `section resize none`.** `none` flips back to `Option::None` (fill-parent) — the only console-side path to that state today, closing the spec gap. Pinned by 6 tests (writes, none clears, overflow reject, zero reject, astronomical reject, undo round-trip). | ✅ |
| G6 | **Tier 2B-setters — auto-fit covers `Some`-sized sections** (`document/mod.rs::grow_one_node_to_fit_text`). Predicate now contributes the **larger of** measured text bounds and (when set) user-pinned `size` to the node-floor calculation, so user intent ("at least this big") survives when text fits AND text overflow still grows the parent (nothing visually clips). Pinned by 3 tests in `tests_nodes.rs` (overflow grows parent, user-size survives when text fits, None-sized regression). | ✅ |
| G7 | **Tier 2B-setters — docs.** `format/sections.md` gains a "Position and size verbs" subsection describing `section move` / `section resize` semantics + selection-shape requirements + the verify-message mirroring. `format/validation.md` gains a "Section bounds" subsection capturing the rules `verify::sections` enforces, with cross-link to the verbs. | ✅ |
| — | **UX-consistency gap (closed):** Tier 2B-setters, Tier 2B-drag, and Tier 2B-resize close all three halves — verbs, move gesture, resize gesture. Sections are now fully first-class for selection, colour, font, clipboard, drag-to-move, and drag-to-resize. | ✅ |
| G8 | **Tier 2B-setters review fix-up — C1 (CRITICAL):** floor-invariant gap. Pre-fix `set_section_offset` / `set_section_size` skipped the post-write `grow_one_node_to_fit_text` / `_border` calls that every other section setter makes; a `None`-sized section moved past existing node bounds left the parent under its measured-text floor, triggering a confusing growth on the next unrelated edit. Both setters now run the floor passes. | ✅ |
| G9 | **Review fix-up — C2:** out-of-range `section=K` errors at the verb layer rather than silently returning "no change" (which was indistinguishable from a successful idempotent set). `execute_section` now bounds-checks against `node.sections.len()` before delegating. Pinned by `section_move_out_of_range_section_kv_errors`. | ✅ |
| G10 | **Review fix-up — C3+X5:** `set_section_offset` / `set_section_size` now reject when the parent `node.size` is non-finite or non-positive, mirroring `verify::sections::check`'s node-level guards. Closes the verify→setter parity asymmetry the cross-cutting reviewer surfaced. New free fn `check_node_size_finite_positive` in `nodes/mod.rs`. | ✅ |
| G11 | **Review fix-up — V2:** `mutate_section_with_style_undo` closure helper in `nodes/mod.rs` deduplicates the snapshot/mutate/undo plumbing across six section setters that use the `EditNodeStyle` undo envelope (`set_section_text_color`, `set_section_font_size`, `set_section_font_family`, `apply_section_payload`, plus the new `set_section_offset` / `set_section_size`). Per-setter savings ~6 lines each; consistent semantics. The two `EditNodeText`-using setters (`set_section_text`, `set_section_text_and_runs`) keep their inline shape — different undo envelope. | ✅ |
| G12 | **Review fix-up — V1:** the two near-identical pinned-two-section fixtures (one in `tests_nodes.rs`, one in `commands/section.rs::tests`) collapsed to a single `pinned_two_section_node` helper in `tests_common.rs`. The verb-side fixture was missing `undo_stack.clear()` — fixed in the consolidated helper. | ✅ |
| G13 | **Review fix-up — V3+V4+V7+V8:** dropped inline plan-tracker terminology ("Tier 2B-setters", "(G6)", "Pre-Tier-2B-setters") from `tests_nodes.rs`; trimmed the over-explained `set_section_offset` / `set_section_size` docstrings to "what" + "why" essentials; tightened loose `> 10.0` auto-fit assertions to concrete `>= 100.0` bounds; tightened the `"is negative"` substring match in `commands/section.rs::tests` to `"section[1].offset.x is negative"` (matches the verify-mirror format). | ✅ |
| G14 | **Review fix-up — V5 + missing undo test:** added `test_auto_fit_some_sized_section_text_dominates_when_larger` (pins the `max(text, user-size)` floor selection — a regression that always picks user-size and ignores text would pass the prior two tests but fail here) and `section_move_round_trips_through_undo` (paralleling the resize undo test). | ✅ |
| G15 | **Review fix-up — X1+X3+X4 docs:** `format/sections.md` "Position and size verbs" subsection extended with the auto-fit "size as floor" semantic and the custom-mutation AABB-bypass note; `format/validation.md` channel-collision rule clarified as a warning (not a hard rejection); `CONCEPTS.md §MindSection` shipped-list extended with a per-section move/resize bullet, and the `Section(SectionSel)` `SelectionState` entry now enumerates every per-section setter (text + colour + font + position + size + structured payload) instead of "set_section_text and friends". | ✅ |
| — | **Surfaced by review, deferred:** `Action::SetSectionOffset { dx, dy }` / `Action::SetSectionSize { w, h }` parametric Action arms (let keybinds / macros / palette nudge a section like `Action::SetColor` / `SetFont` already do for colour/font). Verbs work today; the parametric path is a separate feature. Queued with Tier 2B-drag / Tier 2B-resize. | ⏳ |
| — | **Verified by inspection:** the byte-equal verify-mirror message claim (correctness reviewer walked all 9 messages including the Unicode `×` character and confirmed equality vs `crates/maptool/src/verify/sections.rs`). Adding a cross-crate equality test was considered and skipped — the inspection plus the substring-matching tests at both ends pin the contract sufficiently. | ✅ |
| D1 | **Tier 2B-drag — capture `hit_section_idx`** at the threshold-cross promotion in `app/event_cursor_moved.rs`. Was destructured as `_`; now consumed by D2's branch. | ✅ |
| D2 | **Tier 2B-drag — branch promotion** to `MovingSection` when `hit_section_idx.is_some()` AND the node has > 1 section AND the user isn't shift-dragging (multi-select). Single-section nodes and shift+drag still promote to `MovingNode`, mirroring `hit_test_target`'s single-section fold to `NodeContainer`. | ✅ |
| D3 | **Tier 2B-drag — `MovingSection(MovingSectionInteraction)` variant** on `ThrottledDrag` (`app/throttled_interaction/mod.rs`). | ✅ |
| D4 | **Tier 2B-drag — `MovingSectionInteraction`** in new file `app/throttled_interaction/moving_section.rs`. Mirrors `MovingNodeInteraction`'s shape: `node_id`, `section_idx`, `start_offset`, `total_delta`, `pending_delta`, `throttle`. `ThrottledInteraction::drain` calls the new tree helper, patches buffer positions in place via `patch_drag_positions`, flushes canvas scene, clears `pending_delta`. **No model writes per-frame** — release-commit discipline mirrors `MovingNodeInteraction`. | ✅ |
| D5 | **Tier 2B-drag — `apply_section_drag_delta_and_collect_patches`** tree-mutation helper in `document/hit_test.rs` (sibling of `apply_drag_delta_and_collect_patches`). Walks the targeted section's subtree via `tree.section_arena_id(node_id, idx)` and calls `collect_patches_recursive` on it; invalidates caches like the existing helper. Container and sibling sections untouched. | ✅ |
| D6 | **Tier 2B-drag — cursor-move accumulation arm** for `ThrottledDrag::MovingSection` mirrors the existing `MovingNode` arm (`event_cursor_moved.rs:111-125`): convert screen delta to canvas, `total_delta += delta`, `pending_delta += delta`. | ✅ |
| D7 | **Tier 2B-drag — release-commit arm** in `event_mouse_click.rs` (sibling of the existing `MovingNode` release arm). Single call to `doc.set_section_offset(&i.node_id, i.section_idx, i.start_offset.0 + i.total_delta.x as f64, i.start_offset.1 + i.total_delta.y as f64)`. AABB-overflow rejection logs and falls through to `rebuild_all` from model — section snaps back to its pre-drag offset because the model never accepted the in-progress drag. Single `EditNodeStyle` undo entry pushed by the setter. | ✅ |
| D8 | **Tier 2B-drag — selection-state preservation.** The existing `MovingNode` branch unconditionally sets `SelectionState::Single(node_id)` at threshold-cross; the new `MovingSection` branch instead sets `SelectionState::Section { node_id, section_idx }` so the picker hint (R9), per-section verbs (G1-G7), and structured-clipboard pickup (B1-B9) stay coherent through the drag. The selected-section highlight is rebuilt at threshold-cross via `apply_tree_highlights`. | ✅ |
| D9 | **Tier 2B-drag — tests.** 4 new pinned tests in `tests_hit_move.rs`: `test_apply_section_drag_delta_moves_only_target_section` (target moves, container + sibling untouched), `test_apply_section_drag_delta_unknown_section_no_op` (out-of-range clean), `test_section_drag_release_writes_through_set_section_offset` (release-commit shape + undo), `test_section_drag_release_aabb_overflow_rejects_and_preserves_model` (snap-back path). Plus the trait-default test suite the new `MovingSectionInteraction` inherits via `trait_default_tests_for_throttled_interaction!`. | ✅ |
| D10 | **Tier 2B-drag — docs.** `format/sections.md`'s "Position and size verbs" paragraph rewritten — drag now wired, only resize handles still queued. Plan tracker rows D1-D10. | ✅ |
| D11 | **Tier 2B-drag review fix-up — C1 (perf):** the new section-promotion arm previously called `build_tree()` + `rebuild_buffers_from_tree` unconditionally on every drag start. Now guards with `matches!(doc.selection, SelectionState::Section(s) if s.node_id == node_id && s.section_idx == section_idx)` — common case (click set the Section selection at press, drag promotes within same gesture) skips the rebuild. | ✅ |
| D12 | **Review fix-up — V4: `resolve_section_drag_target` extraction.** The multi-section + non-shift gate is now a named pure function in `event_cursor_moved.rs`, unit-testable without an event-loop harness. 6 new tests pin: multi-section + non-shift returns Some, `section_idx=0` on multi-section also promotes (closes the test gap), single-section returns None, shift+drag returns None, out-of-range returns None, no-doc/no-idx return None. | ✅ |
| D13 | **Review fix-up — V3: `rebuild_selection_highlight` extraction.** The 5-line `build_tree` + `apply_tree_highlights` + `rebuild_buffers_from_tree` block previously appeared in both the `MovingSection` and `MovingNode` promotion arms. Single helper now. | ✅ |
| D14 | **Review fix-up — V2: `canvas_delta` extraction.** The 6-line `screen_to_canvas` + delta calc was duplicated across three accumulator arms (`MovingNode`, `MovingSection`, `EdgeHandle`). Single helper now. | ✅ |
| D15 | **Review fix-up — V5: trait-default test parity.** Added `test_has_pending_true_for_tiny_nonzero_delta` (subpixel accumulator contract — required for the skipped-frames sum to remain coherent) and `test_reset_resets_only_throttle` (default-reset contract) to `MovingSectionInteraction::tests`, matching `MovingNodeInteraction`'s coverage. | ✅ |
| D16 | **Review fix-up — C2/V6: tree-side snap-back assertion.** The AABB-overflow test now drags the tree past the parent's bounds, asserts the tree reflects the per-frame mutation, then asserts a fresh `doc.build_tree()` (which `rebuild_all` calls at release) snaps the section back to its pre-drag offset. | ✅ |
| D17 | **Review fix-up — V1: comment trim.** `moving_section.rs` module/struct/drain/per-field comments shortened. `event_mouse_click.rs` release-commit comment shortened, `Ok(false)` no-op now logs at debug. `event_cursor_moved.rs` accumulator-arm comments dropped (helper is self-explanatory). Promotion-arm comment trimmed to two lines. | ✅ |
| D18 | **Review fix-up — X1+X2+X3+X4: doc drift.** `CONCEPTS.md §ThrottledInteraction` updated from "four-variant" to "five-variant" with `MovingSection` entry. Drag-gestures list adds "move-section". `MindSection` shipped-list adds the per-section drag-to-move bullet. `app/mod.rs::DragState` doc-comment de-counts the variants. | ✅ |
| D19 | **Review fix-up — X5: plan tracker UX-gap row** flipped from ⏳ to 🔧 (partial) — verb half + move-gesture half closed, resize-gesture half remaining. | ✅ |
| D20 | **Review fix-up — X6: `run_native.rs` `is_moving_node` comment.** Explains why `MovingSection` deliberately doesn't qualify for the camera-driven geometry rebuild suppression — section drag never moves the parent node, so the camera rebuild is harmless. | ✅ |
| D21 | **Review fix-up — V7: stale line ranges dropped** from D2 + D7 plan rows and `format/sections.md`'s drag paragraph. Line-range refs rot the moment a sibling commit lands; semantic anchors (`hit_test_target`'s single-section fold) survive. | ✅ |
| — | **Surfaced by review, now closed:** C3 + C5 + X7 closed by rows below (effective-size AABB, Section→Single demote on whole-node-drag promotion, animation tick suppression during section drag). | ✅ |
| R1 | **Tier 2B-resize — `ResizeHandleSide` enum** with 8 variants (`NW, N, NE, E, SE, S, SW, W`) plus `axis_factors() -> (i8, i8)`, `channel() -> usize`, `Display`, and `all() -> [Self; 8]`. Lives at `lib/baumhard/src/mindmap/scene_builder/section_resize_handle.rs` (not the document layer the original plan suggested — baumhard is the right home since the scene builder is the consumer that emits handle elements). | ✅ |
| R2 | **`SectionResizeHandleElement` struct** in the same baumhard module, plus `build_section_resize_handles(node_id, section_idx, section_pos, section_size)` that emits 8 elements per `Some`-sized section (corners + edge midpoints) and `Vec::new()` for `None`-sized fill-parent sections. | ✅ |
| R3 | **`RenderScene.section_resize_handles: Vec<SectionResizeHandleElement>`** field added to `RenderScene` in `lib/baumhard/src/mindmap/scene_builder/mod.rs`. | ✅ |
| R4 | **Selection-gated emission** via `SceneSelectionContext.selected_section: Option<(&str, usize)>` threaded into `build_scene_with_cache`. The app layer's `assemble_scene_overrides` populates it when `doc.selection` is `SelectionState::Section`; the builder resolves the section's AABB in canvas space (offsets applied) and dispatches to `build_section_resize_handles`. | ✅ |
| R5 | **`CanvasRole::SectionResizeHandles`** + `layers::SECTION_RESIZE_HANDLES = 51` (one above edge handles so resize handles win on the rare pixel overlap). `AppScene` slot, role-slot helpers, and `register_canvas` layer match arm wired. | ✅ |
| R6 | **Tree builder** `build_section_resize_handle_tree` + `_mutator_tree` + `section_resize_handle_identity_sequence` in `lib/baumhard/src/mindmap/tree_builder/section_resize_handle.rs`. Same shape as `edge_handle.rs` — single-source-of-truth `section_resize_handle_layout` fn keeps the build + mutator paths from drifting. Identity is per-side channel sequence; selection-gated 0 ↔ 8 transitions take the full-rebuild arm, drag stays on the in-place mutator arm. | ✅ |
| R7 | **`update_section_resize_handle_tree`** in `app/scene_rebuild.rs` mirroring `update_edge_handle_tree`'s §B2 dispatch (signature → `InPlaceMutator` / `FullRebuild`). Called from `rebuild_scene_only` so every selection change / drag frame keeps the handle tree fresh. | ✅ |
| R8 | **`hit_test_section_resize_handle(map, canvas_pos, node_id, section_idx, tolerance)`** in `document/hit_test.rs` (sibling of `hit_test_edge_handle`). Reuses `build_section_resize_handles` to compute live positions; bounded-cost 8 distance comparisons; returns `Option<ResizeHandleSide>`. `None`-sized sections + missing nodes / sections all return `None` cleanly. | ✅ |
| R9 | **`DragState::Pending.hit_section_resize_handle: Option<(String, usize, ResizeHandleSide)>`** field. Press-time hit test in `event_mouse_click.rs` populates when `doc.selection` is `SelectionState::Section`, using `EDGE_HANDLE_HIT_TOLERANCE_PX * canvas_per_pixel()` (handles are point-like; same forgiving tolerance as edge handles). | ✅ |
| R10 | **`SectionResizeInteraction`** in new file `app/throttled_interaction/section_resize.rs`. Carries `node_id`, `section_idx`, `side`, `start_offset` (`Position`), `start_size` (`Size`), `total_delta`, `pending_delta`, `throttle`. `resolve(total_delta) -> (Position, Size)` is a pure function that folds the cursor delta through `axis_factors` — used by both the per-frame drain (for offset shifts on W/N/NW/NE/SW handles) and the release-commit arm. | ✅ |
| R11 | **`SectionResize(SectionResizeInteraction)` variant** on `ThrottledDrag` + threshold-cross promotion in `event_cursor_moved.rs` between `EdgeHandle` and `MovingSection`/`MovingNode` arms (handles win over the section / node behind them, mirroring edge-handle precedence). Snapshot section's pre-drag `(offset, size)` for the drain-side resolve math. | ✅ |
| R12 | **Release-commit arm** in `event_mouse_click.rs`. Order: `set_section_size` first, then `set_section_offset` — the offset's AABB check uses the section's current size, so writing size first lets a shrink-from-NW gesture pass arithmetic that would have failed under the old size. AABB / non-positive-size rejection logs and falls through to `rebuild_all` from model — section snaps back. | ✅ |
| R13 | **Tests.** 5 hit-test pins in `tests_hit_move.rs` (`None`-sized returns None, missing node/section returns None, SE corner hit, N edge-mid hit, center-of-section misses every handle). 12 unit tests inside `section_resize_handle.rs::tests` + `section_resize.rs::tests` covering axis-factor pinning per side, 8-handle position math, channel uniqueness, every variant's `resolve()` math (SE / NW / N / E / NE / SW), throttle defaults, has_pending, reset semantics, and the trait-default macro suite. | ✅ |
| R14 | **Tier 2B-resize — docs.** `format/sections.md`'s "Drag-to-move gesture" paragraph extended with a "Drag-to-resize gesture" paragraph. Plan tracker rows R1-R14 + UX-gap row flipped from 🔧 to ✅. | ✅ |
| R15 | **Review fix-up — correctness #2 (atomic AABB setter).** `set_section_aabb(node_id, idx, offset, size)` validates the **post-mutation** AABB atomically and writes both fields under one `EditNodeStyle` undo entry. Replaces the size-then-offset two-step in the resize release-commit, which silently rejected legal W/N grow gestures pinned against the right/bottom edge (intermediate state had new size at old offset, overflowing). 6 new tests pin: W-grow accepts, post-mutation overflow rejects with verify-mirror message, negative-offset rejects, non-positive-size rejects, idempotent no-op, single-undo-entry. | ✅ |
| R16 | **Review fix-up — correctness #1 + #3 (mid-drag visual feedback).** `SectionResizeInteraction::drain` now writes the in-progress `(canvas_pos, canvas_size)` to the section-area's `GlyphArea` directly via the new `apply_section_resize_to_tree` helper, rebuilds buffers from the tree (cosmic-text reflows the section content against the new bounds), and refreshes the 8 handle positions via the new `update_section_resize_handle_tree_from_slice`. The user now sees the section grow / shrink + handles track the cursor mid-drag for every side, including pure E/S/SE handles where the previous version showed zero feedback until release. 2 new tests pin the helper. | ✅ |
| R17 | **Review fix-up — correctness #4 (fold guard).** `hit_test_section_resize_handle` now returns `None` for nodes hidden by an ancestor fold, mirroring the scene-builder's `is_hidden_by_fold` gate. A stale `Section` selection that survived a fold mutation can't capture phantom handle presses. 1 new test pins the gate. | ✅ |
| R18 | **Review fix-up — conventions: `Position` / `Size` derive `Copy`.** Both are plain `f64 × 2` data structs whose docstrings describe them as "Plain data; no runtime cost", so deriving `Copy` is just a §B0 cleanup. Drops 2 `.clone()` calls at the threshold-cross promotion. | ✅ |
| R19 | **Review fix-up — conventions: `EDGE_HANDLE_HIT_TOLERANCE_PX` → `HANDLE_HIT_TOLERANCE_PX`.** The constant is shared by edge handles and section resize handles; the new name reflects the unified scope. Doc-comment updated to "applies uniformly to edge handles and section resize handles." | ✅ |
| R20 | **Review fix-up — conventions: doc comments on `SectionResizeHandleElement` pub fields.** Added `///` docs to `node_id`, `section_idx`, `side` per §B9 ("Every `pub` item carries a `///` doc comment"). | ✅ |
| R21 | **Review fix-up — cross-cutting: `is_moving_node` comment.** Extended in `run_native.rs` to mention `SectionResize` alongside `MovingSection` — both deliberately don't qualify for camera-rebuild suppression. | ✅ |
| C3 | **Deferred (now closed): `None`-sized AABB containment.** `set_section_offset` and `verify::sections::check_within_node_aabb` now both use the section's *effective size* (`Some(sz)` or `node.size` fallback) for the right / bottom-edge containment check. A fill-parent section at non-zero offset stretches past the parent and is now correctly rejected — pre-fix the `None` arm of the size-Option skipped the check entirely, leaving the gesture free to drag the section into visual overflow. New tests: 2 in `tests_nodes.rs` (rejects-nonzero / accepts-zero on None-sized) + 2 in `verify::sections::tests` (offset-zero clean / nonzero-offset overflows). The pre-existing `unset_size_skips_aabb_check` test was renamed and inverted to pin the new contract. `format/sections.md` "Effective size for AABB containment" paragraph documents the rule. | ✅ |
| C5 | **Deferred (now closed): Section selection demoted on whole-node drag.** When the threshold-cross promotion falls through to `MovingNode` from a Section selection, the selection now demotes to `Single(node_id)` rather than surviving as `Section(node_id, idx)`. Pre-fix the picker hint and per-section verbs read "Section[K]" while the user bodily moved the parent — coherent only after release rebuild. The new `needs_demote` guard checks for a same-node Section selection and triggers the rebuild_selection_highlight branch unconditionally, so mid-drag UX matches the gesture. | ✅ |
| X7 | **Deferred (now closed): animation interleaving.** `drain_animation_tick` is suppressed in `run_native::drain_frame` when a `MovingSection` or `SectionResize` drag is in flight. The animation tick routes through `apply_custom_mutation` → `sync_node_from_tree`, which observes the in-progress mid-drag tree position; without suppression a `target_scope: SectionsOnly` animation firing on the same frame would commit the in-progress AABB to the model and push an unintended undo entry. Animations resume on the next frame after release. Documented in `CONCEPTS.md §MindSection` "Animation interleaving" subsection. | ✅ |
| C3+ | **Review fix-up: shared `effective_size` helper + `set_section_size(None)` symmetric fix.** Three reviewers (correctness #1, conventions #1, cross-cutting #1) independently flagged the same gap: `set_section_size(None)` still skipped the AABB check, so a flatten-to-fill-parent on a section pinned at non-zero offset committed a state verify rejects. The setter now runs the same `offset + effective_size <= node.size` guard for both the `Some` and `None` arms. The conventions reviewer's §5 / §10 finding about cross-crate duplication is also closed: `MindSection::effective_size(node_size)` lives at `lib/baumhard/src/mindmap/model/node.rs`; both `set_section_offset` and `verify::sections::check` route through it (and the verify side drops a redundant `.clone()` since `Size` is already `Copy`). 3 new tests (`test_set_section_size_rejects_none_when_offset_nonzero`, `test_set_section_size_accepts_none_at_zero_offset`, `mindsection_effective_size_falls_back_to_node_size_when_none`). | ✅ |
| X7+ | **Review fix-up: widen X7 to also suppress during `MovingNode`.** Correctness reviewer #2 surfaced that `MovingNode` mutates the tree per-frame (`apply_drag_delta_and_collect_patches`); an animation tick on the same frame would corrupt it via the same `sync_node_from_tree` path the section-drag suppression was meant to close. The predicate is renamed `is_drag_with_tree_mutation` and now matches all three of `MovingNode` / `MovingSection` / `SectionResize`. Edge / portal-label drags don't mutate the tree per-frame so they stay unsuppressed. | ✅ |
| Sch | **Review fix-up: `format/schema.md` for the C3 contract change.** Cross-cutting reviewer #2 found that schema.md (the "primary reference" per CLAUDE.md) still described the pre-fix contract for the `size` row. The row now includes the offset-coupling constraint and links to the new "Effective size for AABB containment" subsection in `format/sections.md` (promoted from a bold paragraph to an H3 heading). | ✅ |

`MindNode` now owns `sections: Vec<MindSection>` (see
`lib/baumhard/src/mindmap/model/node.rs:61` and `:270`). Each section
has its own `text`, `text_runs`, `offset`, `size`, `channel`,
`trigger_bindings`. Spec: `format/sections.md`. The migration shipped
end-to-end through the loader, runtime tree, hit-tester, inline
editor, and custom-mutation persistence.

The audit found the foundation solid — but five of the trait
dispatcher's style impls and the color picker's target enum
**explicitly collapse `Section` → whole-node**, with in-source comments
already calling these out as "future verb" seams. Tier 2A closes those
seams.

## Already shipped (acknowledge — do not redo)

These pieces of section integration are already correct; Tier 2A
must not regress them.

- ✅ Hit-test returns `HitTarget::Section { node_id, section_idx }` for
  multi-section nodes (`document/hit_test.rs:91-138`); single-section
  nodes fold to `NodeContainer` so legacy maps preserve whole-node
  semantics.
- ✅ Click → `SelectionState::Section(SectionSel { … })` on both
  native (`app/click.rs:92-101`) and WASM
  (`event_mouse_click.rs:237-250, :386-390`).
- ✅ Double-click discrimination keys on `(node_id, section_idx)`
  (`app/mod.rs:178`); inline editor opens on the targeted section
  (`text_edit/editor.rs:65-69`); commit through
  `set_section_text_and_runs`.
- ✅ Per-section trigger bindings fire before whole-node bindings
  (`event_mouse_click.rs:349-376`).
- ✅ Custom-mutation `target_scope: SectionsOnly` walks
  `MindMapTree::section_arena_id`; persistence via
  `sync_node_from_tree` (`document/custom/sync.rs:238-272`) writes
  back `section.offset` / `section.size`.
- ✅ Console verb `color text=#xxx section=K` calls
  `set_section_text_color` (`commands/color.rs:271`); pinned by
  `color_text_section_kv_targets_specific_section`.
- ✅ Console verb `font size=N section=K` calls
  `set_section_font_size` (`commands/font.rs:333`); pinned by
  `font_size_section_kv_targets_specific_section`.
- ✅ Clipboard traits (`HandlesCopy/Paste/Cut`) honour
  `TargetView::Section` for the `text` field; per-run / offset /
  size / channel / bindings fidelity then shipped as Tier 2B-clipboard
  (rows B1-B9 below).
- ✅ `selection_targets` emits `TargetId::Section` for the dispatcher
  (`view.rs:669-672`).
- ✅ Five section-aware document setters exist
  (`document/nodes/mod.rs`):
  `set_section_text` (149), `set_section_text_and_runs` (75),
  `set_section_text_color` (204), `set_section_font_size` (246),
  `set_section_font_family` (285 — currently dead, item #5 wires it).
- ✅ Auto-fit considers `None`-sized sections
  (`document/mod.rs:192-269`). *Note: `Some`-sized section growth is
  Tier 2B.*

## Tier 2A — work items

### Item 1 — `HasTextColor::set_text_color` honours `Section`

**File:** `src/application/console/traits/view.rs:153-198`

**Today (line 162):**
```rust
TargetView::Node { doc, id } | TargetView::Section { doc, id, .. } => {
    Outcome::applied(doc.set_node_text_color(id, color_as_string(&c, "#ffffff")))
}
```

**After:** split the arm. `Node` keeps `set_node_text_color`. `Section`
calls `doc.set_section_text_color(id, *section_idx, color_as_string(&c, "#ffffff"))`.

**Effect:** `color text=#xyz` from a section selection (without an
explicit `section=K` kv) writes only the targeted section's runs.

### Item 2 — `HasBgColor::set_bg_color` returns `NotApplicable` for `Section`

**File:** `src/application/console/traits/view.rs:126-151`

**Today (line 135):** Node and Section share `set_node_bg_color`.

**After:** split the arm. `Node` keeps `set_node_bg_color`. `Section`
returns `Outcome::NotApplicable` with a comment pointing at
`format/sections.md` (sections have no bg-fill chrome by spec). This
matches `commands/color.rs:275-280` where `color bg= section=K`
already returns NotApplicable.

### Item 3 — `HasBorderColor::set_border_color` returns `NotApplicable` for `Section`

**File:** `src/application/console/traits/view.rs:200-240`

**Today (line 205):** Node and Section share `set_node_border_color`.

**After:** split the arm. `Node` keeps `set_node_border_color`.
`Section` returns `Outcome::NotApplicable`. Same reasoning as Item 2.

### Item 4 — `AcceptsWheelColor::apply_wheel_color` for `Section`

**File:** `src/application/console/traits/view.rs:242-259`

**Today (line 248):** `TargetView::Node { .. } | TargetView::Section { .. }`
both call `self.set_bg_color(c)`.

**After:** split. `Node` keeps `self.set_bg_color(c)`. `Section`
calls `self.set_text_color(c)` (because the only colour axis a
section has is text). Combined with Item 1, the wheel will write
through `set_section_text_color`. Items 2 / 3 already cover the
explicit bg / border axes returning NotApplicable when the picker is
forced into those modes — but `apply_wheel_color` is the
"undirected" entry point and `Text` is the only sensible default for
a section.

### Item 5 — `AcceptsFontFamily::set_font_family` for `Section` (wires dead setter)

**File:** `src/application/console/traits/view.rs:261-283`,
`src/application/document/nodes/mod.rs:285-321`

**Today (line 268):** Node and Section share `set_node_font_family`.

**After:** split the arm. `Node` keeps `set_node_font_family`.
`Section` calls `doc.set_section_font_family(id, *section_idx,
family)`. This is the call site `set_section_font_family` was
written for; it has been dead since the section refactor landed.

### Item 6 — `ColorTarget::Section` variant

**File:** `src/application/color_picker/targets.rs:19-43`

**Today:** `ColorTarget = Edge(EdgeRef) | Node { id, axis:
NodeColorAxis }` where `NodeColorAxis = Bg | Text | Border`.

**After:** add a third variant.

```rust
pub enum ColorTarget {
    Edge(EdgeRef),
    Node { id: String, axis: NodeColorAxis },
    Section { node_id: String, section_idx: usize, axis: SectionColorAxis },
}

pub enum SectionColorAxis {
    Text,  // only axis sections have today
}
```

`SectionColorAxis::Text` is intentionally a single-variant enum so
adding `Bg`/`Border` later (Tier 2C, only if the data shape changes)
is non-breaking.

`PickerHandle` mirrors with a `Section { node_id, section_idx, axis }`
variant.

### Item 7 — `picker_target_for` emits `ColorTarget::Section`

**File:** `src/application/console/commands/color.rs:99-111`

**Today:** Section selection silently collapses to
`ColorTarget::Node { id: section.node_id, axis: … }`.

**After:** when the selection is `SelectionState::Section(s)` and the
axis is `Text`, return `ColorTarget::Section { node_id: s.node_id,
section_idx: s.section_idx, axis: SectionColorAxis::Text }`. When the
axis is `Bg` / `Border`, return `Outcome::NotApplicable` (the call
site that uses this for the picker open path needs to learn to
display the NotApplicable signal — likely a console message).

### Item 8 — `current_color_at` reads section text colour

**File:** `src/application/color_picker/targets.rs:122-129`

**Today:** Node-only — reads `n.style.background_color | text_color |
frame_color`.

**After:** add a `Section` arm. Read the resolved colour for the
section's text — the cascade is: first `text_run.color` if all runs
agree, else `node.style.text_color`. Use the same resolution helper
that `set_section_text_color` uses on the read side
(`document/nodes/mod.rs:204-237` is the write side; find or add a
mirror reader if missing).

### Item 9 — Standalone-mode wheel commit honours `ColorTarget::Section`

**File:** `src/application/app/color_picker_flow/commit.rs:228-269`

**Today:** Fans out via `selection_targets` →
`TargetView::apply_wheel_color`, which (via the collapsed Section arm
in Item 4) wrote node-level. Once Item 4 lands, this path
automatically routes correctly. Verify it does and add a test.

### Item 10 — Parametric Action-path lag

**File:** `src/application/console/commands/font.rs:459-505`

**Today (lines 478-486):** `apply_font_kv_to_selection`'s `Section`
arm collapses to `set_node_font_size`.

**After:** split — `Section { node_id, section_idx }` calls
`doc.set_section_font_size(node_id, section_idx, pt)`. This brings
the parametric Action arm in line with the verb path
(`section_font_outcome`), so keybinds and palette entries that
trigger `Action::SetFontSize` from a section selection target the
correct section.

### Item 11 — Tests

Mirror the existing pinned shapes:

- `color_text_section_collapse_writes_only_section`
  (mirrors `color_text_section_kv_targets_specific_section`,
  `commands/color.rs:402-442`) — drives via the trait dispatch path
  (no explicit `section=K` kv) and asserts only the targeted section's
  runs change. Pins Item 1.
- `color_bg_section_returns_not_applicable` — pins Item 2.
- `color_border_section_returns_not_applicable` — pins Item 3.
- `wheel_color_section_writes_through_text_color` — drives the wheel
  commit on a section selection. Pins Item 4 + Item 9.
- `font_family_section_collapse_writes_only_section` — mirrors the
  font-size test. Pins Item 5.
- `picker_target_for_section_emits_section_target` — pins Items 6/7.
- `current_color_at_section_reads_section_text_color` — pins Item 8.
- `font_size_action_section_writes_through_section_setter` — pins
  Item 10 (Action path).

Test locations: `console/tests/color.rs`, `console/tests/font.rs`,
`color_picker/tests/`.

### Items 12-13 — Build hygiene

- `./test.sh` — full suite + WASM type-check.
- `./test.sh --lint` — `cargo fmt --check` + `cargo clippy`.

## Critical files to touch

| File | What changes |
|---|---|
| `src/application/console/traits/view.rs` | Items 1–5: split each style trait arm to give `Section` its own behaviour |
| `src/application/console/commands/color.rs` | Item 7: `picker_target_for` emits `ColorTarget::Section` |
| `src/application/console/commands/font.rs` | Item 10: `apply_font_kv_to_selection` Section arm |
| `src/application/color_picker/targets.rs` | Items 6, 8: `ColorTarget::Section` + `current_color_at` arm |
| `src/application/color_picker/state.rs` | Item 6 (likely): `PickerHandle::Section` variant |
| `src/application/color_picker/compute.rs` | Item 6 follow-on: any `match ColorTarget` exhaustiveness |
| `src/application/app/color_picker_flow/commit.rs` | Item 9: verify and pin |
| `src/application/document/nodes/mod.rs` | No new setters; ensure `set_section_font_family` (line 285) is reachable from Item 5 (mostly a verification step) |
| `src/application/console/tests/color.rs`, `tests/font.rs` | Item 11 tests |
| `src/application/color_picker/tests/` | Item 11 tests for picker target / current colour |
| `format/sections.md` | Add a one-line note that bg/border axes return NotApplicable on a section selection (consistent with the existing `color bg= section=K` doc) |

## Reusable utilities (do NOT duplicate)

- `MindMapTree::section_arena_id` — already used by
  `TargetScope::SectionsOnly`; reuse for any "walk a node's sections"
  helper.
- `selection_targets` (`view.rs:669-672`) — already emits
  `TargetId::Section`; the picker `commit.rs` fan-out already iterates
  these targets.
- `set_section_text_color`, `set_section_font_family` — preserve
  `var(--name)` references; never sidestep with raw field writes.
- `SectionSel { node_id, section_idx }`
  (`document/types.rs:189-204`) — the canonical section reference
  type; reuse in new variants.
- `ColorValue` + `color_as_string` (`view.rs:104-124`) — color
  encoding helpers; reuse rather than reimplementing the
  `var(--name)` / hex split.

## Verification plan (end-to-end)

1. **Unit tests** as listed in Item 11 — pin each behaviour change.
2. **`./test.sh`** — full workspace tests + WASM type-check. Cross-
   platform drift fails the run.
3. **`./test.sh --lint`** — `cargo fmt --check` + `cargo clippy`.
4. **Manual smoke (native).** `cargo run -- maps/testament.mindmap.json`
   (or any multi-section map). Steps:
   - Click into a single section of a multi-section node — confirm
     selection lands on `SelectionState::Section`.
   - Run `color text=#ff8800` (no `section=` kv) → only that section's
     runs change colour (Item 1).
   - Run `color bg=#ff8800` → console reports NotApplicable
     (Item 2).
   - Open the standalone color picker (verb / shortcut), commit a
     colour → only that section's text colour changes (Items 4, 6, 7,
     8, 9).
   - Run `font set "Source Code Pro"` → only that section's runs
     change family (Item 5).
   - Bind `Action::SetFontSize` to a key, trigger from a section
     selection → only that section grows (Item 10).
   - Confirm the same actions on a `SelectionState::Single` whole-node
     selection still write whole-node (regression check).
5. **Manual smoke (WASM).** `./run.sh` and repeat the click +
   `color text=` flow in the browser.
6. **Round-trip check.** Save the map (`save` console verb), reload,
   verify section colours / fonts persisted via `set_section_*`
   setters (which preserve `var(--name)`) and not via any silent
   round-trip through `FloatRgba`.

## Out of scope — captured for future iterations

### Tier 2B — partial: clipboard + setters + drag shipped, resize pending

**Tier 2B-clipboard ✅ shipped** — rows B1-B20. Structured
`ClipboardContent::Section { text, payload }` with `String`
fallback round-trips per-run formatting and section chrome
through verb and Action paste paths via the in-process
`SECTION_BUFFER` slot.

**Tier 2B-setters ✅ shipped** — rows G1-G15. `set_section_offset`
/ `set_section_size` document setters with full AABB validation
mirroring `maptool verify`. Console verbs `section move` /
`section resize` (including `section resize none` for the
fill-parent flip). Auto-fit predicate covers `Some`-sized
sections with size-as-floor semantics.

**Tier 2B-drag ✅ shipped** — rows D1-D10. Section drag is now a
first-class throttled interaction (`MovingSectionInteraction`),
threshold-cross consumes `hit_section_idx` (was discarded), and
release-commit writes through `set_section_offset` with
AABB-overflow snap-back. Selection state preserved through
drag so picker hint, per-section verbs, and structured-clipboard
pickup all stay coherent.

**Tier 2B-resize ✅ shipped** — rows R1-R14. 8 resize handles
emit on top of the currently-selected `Some`-sized section
(corners + edge midpoints), gated on `SelectionState::Section` +
`size.is_some()`. Threshold-cross promotes
`DragState::Pending.hit_section_resize_handle` to
`Throttled(SectionResize)`; per-frame drain shifts the section's
tree-side `offset` for handles whose `axis_factors.x = -1` /
`axis_factors.y = -1` (W / N / NW / NE / SW); release-commit
writes the final `(offset, size)` through `set_section_size` +
`set_section_offset` under a single `EditNodeStyle` undo entry.
AABB-overflow / non-positive-size rejection logs and snaps back
via full `rebuild_all`. New scene-builder element
(`SectionResizeHandleElement`), new tree-builder
(`build_section_resize_handle_tree` /
`_mutator_tree`), new canvas role
(`CanvasRole::SectionResizeHandles` at layer 51). Closes the UX-
consistency gap entirely — drag and resize gestures both ship
for sections.

### Tier 2C (deferred — larger product changes)

- `SelectionState::MultiSection`.
- Manual node-resize gesture + `set_node_size` setter.
- Auto-fit shrink path / `node fit-to-content` verb.
- Per-grapheme range targeting via picker / font / color commands.

### Dropped from the original Tier 2C scope

- ~~"Insert section" paste verb.~~ Obsoleted by Tier 2B-clipboard:
  the structured `ClipboardContent::Section` payload already
  round-trips section→section via `apply_section_payload`, and
  pasting plain text into a non-section spot is covered by
  `set_section_text`'s template-collapse. There is no UX gap a
  dedicated "insert section" verb would fill that the structured
  paste doesn't already cover.

### Surfaced by post-Tier-2A review, addressed in Tier 2A.5

All three items below shipped on top of Tier 2A. Status table
rows R6–R11 capture the work; the original framing is preserved
here so future iterations can read the chain of reasoning.

- **X2 — Action-path NotApplicable visibility.** Closed by R8.
  `apply_color_axis_to_selection` logs a `log::info!` with the
  dispatcher's per-target messages when every target reports
  NotApplicable, so a keybind for `Action::SetColor { axis: Bg
  | Border }` against a `Section` selection now leaves a trace
  in the log. Font's parametric Action helper was deliberately
  *not* changed — its NotApplicable cases (`min` / `max` on a
  node/section) match the verb-path's silent-false documented
  behaviour, so introducing log lines there would be noise.
- **X4 — Selection-identity HUD surface.** Closed by R9.
  Standalone-mode picker title now appends a selection hint
  (`"section K of <node>"` / `"node <id>"` / `"{count} nodes"` /
  `"edge"` / `"(no selection)"`) so a wheel commit's target is
  visible at a glance. Plumbed through
  `ColorPickerOverlayGeometry::selection_hint`; populated at
  rebuild time from `doc.selection`. Contextual mode unchanged
  (already labels its bound target via `PickerHandle::label`).
- **X5 — `var(--name)` collapse through picker.** Closed by R6.
  `PickerMode::Contextual` now carries `seed_var_ref` +
  `seed_hsv` captured at open; commit preserves the var
  reference when bit-exact HSV equality says the user never
  moved the wheel. Custom-mutation writes still collapse var
  refs to hex on round-trip — that constraint is unchanged
  (separate code path; `FloatRgba` doesn't carry variable
  records).

## Original audit findings (reference — do not edit after baseline)

The source-of-truth audit findings, with file:line citations, are
preserved here so future sessions can reconstruct the reasoning.

### Q1 — Console & actions ⚠ partial

Trait dispatcher has `TargetView::Section` (`view.rs:29-85`) and
`selection_targets` materialises `TargetId::Section` (`view.rs:669-672`).
Parser is section-unaware (`parser.rs:25-179`); section addressing is
the kv `section=<idx>` on `color` and `font` only.

Per-command audit:

- `color.rs` ✅ for `text=`, ❌ for `bg`/`border` (correct — sections
  have no chrome).
- `font.rs` ✅ for `size=`, ❌ for `set <family>` (collapses).
- `border/`, `zoom.rs` collapse to whole node (correct — node-level
  data).
- `mutation.rs` resolves Section to node id; `target_scope` machinery
  handles dispatch.
- `anchor.rs`, `body.rs`, `cap.rs`, `edge.rs`, `label.rs`,
  `spacing.rs` are edge-only (correct).
- `fps.rs`, `help.rs`, `new.rs`, `open.rs`, `save.rs` not selection-
  bound.

Five style trait impls collapse Section → whole-node:
`HasBgColor`(135), `HasTextColor`(162), `HasBorderColor`(205),
`AcceptsWheelColor`(248), `AcceptsFontFamily`(268). All have in-source
"future verb" comments. Clipboard trio honours Section.

Silent collapse on natural workflow: `color text=#xxx` *without*
`section=K` from a section selection writes whole-node via
`HasTextColor` collapse. Same for `font set <family>`.

Dead code: `set_section_font_family` (`nodes/mod.rs:285-321`).

Action-path lag: `apply_font_kv_to_selection`(`font.rs:478-486`)
collapses Section even though `execute_font` honours it.

### Q2 — Mouse targeting ⚠ partial

Hit-test (`hit_test.rs:91-138`) returns `HitTarget::Section` for
multi-section nodes; single-section folds to `NodeContainer`.
Click → `SelectionState::Section` on native (`click.rs:92-101`) and
WASM (`event_mouse_click.rs:237-250, :386-390`). Double-click
discriminates on `(node_id, section_idx)` (`mod.rs:178`); editor
opens on the targeted section (`text_edit/editor.rs:65-69`).

Drag is the gap: `DragState::Pending` carries `hit_section_idx`
(`mod.rs:457-475`) but `event_cursor_moved.rs:160-173` discards it
and always promotes to `MovingNode`. No rect-select for sections, no
reparent at section granularity, no resize handles.

### Q3 — Moving sections ❌ missing

No mouse path. No console verb. No
`set_section_offset`/`set_section_size` document setter. The only
working path is `CustomMutation { target_scope: SectionsOnly,
mutations: [AreaCommand::NudgeRight/MoveTo/SetBounds] }`; persistence
via `sync_node_from_tree` (`custom/sync.rs:238-272`); pinned by
`test_sync_node_from_tree_section_offset_persists_after_rebuild`.

No sibling reflow — sections positioned independently
(`tree_builder/node.rs:148-153`); may overlap and overflow; pinned by
`test_point_in_node_aabb_includes_overflowing_section`.

### Q4 — Parent resize ⚠ partial

`grow_one_node_to_fit_text` (`document/mod.rs:192-269`) walks
sections, folds `section.offset`, applies floor only if larger
(grow-only, line 263-268). Skips `Some`-sized sections (line 215).

Tree builder derives `bounds = node.size_vec2()` for `None`-sized
sections (`tree_builder/node.rs:149-153`); `sync_node_from_tree`
preserves `None` across mutation round-trips
(`custom/sync.rs:254-271`).

No manual resize gesture. Grow-only — `tests_edges_chain.rs:126`.
`SetBounds` shrink has no clamp/relayout for sections; overflow
caught only at `verify` time.

### Q5 — Clipboard ⚠ partial (text-only)

`ClipboardContent` is `Text(String) | Empty | NotApplicable`
(`outcome.rs:33-39`); platform layer is `String` only
(`clipboard.rs:7-22`). Sections ARE first-class targets — all three
trait impls honour `TargetView::Section` (`view.rs:312-599`); paste
clamps `section_idx` against current count (`view.rs:394-416`).

Lossy: `text_runs`, `offset`, `size`, `channel`, `trigger_bindings`
all drop because copy reads `section.text` only and paste writes via
`set_section_text` which collapses runs to a single template-
inherited run (`nodes/mod.rs:149-191`).

No "insert section" paste verb. No multi-section selection.

### Q6 — Font ⚠ partial

`font` command (`commands/font.rs`) handles `set <family>`, `list`,
`size=N [section=K]`. `KEYS` includes `"section"`. Section-targeted
size works via `section_font_outcome` (`font.rs:256-266`) →
`set_section_font_size`; pinned. Family via trait collapses to whole
node. `set_section_font_family` is dead.

`apply_font_kv_to_selection` (`font.rs:478-486`) collapses Section
for size — Action-path lag behind verb.

### Q7 — Color picker ⚠ partial

`ColorTarget = Edge | Node { id, axis }` (`targets.rs:30-33`); no
Section variant. `picker_target_for` (`commands/color.rs:99-111`)
collapses Section → Node. Standalone commit
(`color_picker_flow/commit.rs:228-269`) fans out via
`selection_targets` → `TargetView::apply_wheel_color` → collapsed
Section arm.

Console `color text= section=K` works (`apply_section_colours`,
`color.rs:244-294`); `bg=`/`border=` with `section=K` correctly
returns NotApplicable. Without `section=K`, trait dispatch collapses.

No per-grapheme range coloring from any user surface;
`text_runs` per-glyph colour only via custom mutations.
