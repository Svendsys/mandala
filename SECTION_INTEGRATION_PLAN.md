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

## Context

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
  `TargetView::Section` for the `text` field
  (`view.rs:312-599`); pinned by `console/tests/clipboard.rs:131-161`.
  *Note: per-run / offset / size / channel fidelity is Tier 2B.*
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

### Tier 2B (deferred)

- Section drag — `DragState::MovingSection` /
  `ThrottledDrag::MovingSection`; threshold-cross promotion at
  `event_cursor_moved.rs:160-173` keeps `hit_section_idx`.
- Section resize handles for `section.size`.
- `set_section_offset` / `set_section_size` document setters with
  AABB validation.
- Console verbs `section move <dx> <dy>` / `section resize <w> <h>`.
- Structured `ClipboardContent::Section { text, text_runs, offset,
  size, channel, trigger_bindings }` payload with `String` fallback.
- Auto-fit covers `Some`-sized sections (`document/mod.rs:215`).

### Tier 2C (deferred — larger product changes)

- `SelectionState::MultiSection`.
- Manual node-resize gesture + `set_node_size` setter.
- Auto-fit shrink path / `node fit-to-content` verb.
- Per-grapheme range targeting via picker / font / color commands.
- "Insert section" paste verb.

### Surfaced by post-Tier-2A review, deferred for separate iteration

- **Action-path NotApplicable visibility (X2 from review).** When
  `Action::SetColor { axis: Bg|Border }` fires from a keybind /
  macro / palette against a `SelectionState::Section`, the trait
  dispatcher returns `Outcome::NotApplicable` per Items 2-3 and
  `apply_color_axis_to_selection` returns `false`. The verb path
  surfaces a NotApplicable scrollback message; the Action path is
  silent because Action arms have no scrollback. A `log::info!`
  hook in the helper for "any_applied=false && all NotApplicable"
  would close the visibility gap, but the same shape applies to
  every other NotApplicable case across all targets, not just
  Section + bg/border — out of Tier 2A scope.
- **Selection-identity HUD surface (X4 from review).** A user with
  a `SelectionState::Section` and the standalone color picker
  open has no visual confirmation that the wheel commit will land
  per-section vs whole-node. The Contextual picker title bar
  shows "section" via `PickerHandle::label()`; Standalone mode
  has a fixed title template. A "selection: section K of node X"
  hint in the Standalone footer or a one-line scrollback echo on
  selection change would close it. Cross-cutting UX, broader than
  Tier 2A's collapse-hole closure.
- **`var(--name)` collapse through picker (X5 from review).** The
  picker reads section colour through `current_color_at` →
  `current_hsv_at` → `resolve_var`, so it opens at the right hue
  even when the section's runs reference `var(--accent)`. Commit
  writes raw HSV hex via `set_section_text_color`, erasing the
  `var(--name)` reference. This is the same lossy round-trip
  documented for custom mutations in `format/sections.md` lines
  100-103, just newly reachable through the picker too. Closing
  it requires either a "preserve var ref if HSV unchanged"
  short-circuit on commit or a "commit as var" toggle in the
  picker — both new product surface, not Tier 2A scope.

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
