# P0-02: Custom-mutation sync-back gap — grow/shrink-font are silent no-ops, Toggle behavior is dead, undo stack accumulates junk entries

**Severity:** P0 (shipped feature does not work) · **Area:** mandala/document/custom + baumhard bridge · **Verified:** yes (sync.rs field set + application.json traced)

## Context

Custom mutations apply a `MutatorTree` to the runtime `Tree<GfxElement, GfxMutator>`, then `sync_node_from_tree` writes changed fields back to the `MindMap` model, then the dispatcher pushes `UndoAction::CustomMutation` (CONCEPTS.md §4 "Persistent: snapshot, apply, sync back, push undo"). Both dispatch paths (console `mutation apply`, click triggers/keybinds) run a full `rebuild_all` immediately afterwards, which rebuilds the tree **from the model** (`src/application/app/console_input/exec.rs:98`, `src/application/app/click.rs:109`).

## Problem

`sync_node_from_tree` (`src/application/document/custom/sync.rs:148-370`) syncs back only: node `position`, and per-section `(text, text_runs, offset, size)`. Critically, run `size_pt` is taken from the **prior model run**, never from the tree:

```rust
// sync.rs:92
let size_pt = prior.map(|p| p.size_pt).unwrap_or(DEFAULT_TEXT_RUN_SIZE_PT);
```

The bundled mutations `grow-font-2pt` / `shrink-font-2pt` (`assets/mutations/application.json`) are `AreaCommand::GrowFont/ShrinkFont`, which mutate tree-side `area.scale`. Consequences, in order:

1. Mutation applies to the tree (visible for at most one frame).
2. Sync-back sees no changes in its synced field set — model stays byte-identical.
3. `apply_custom_mutation` **still pushes** `UndoAction::CustomMutation` and sets `dirty = true` (`custom/mod.rs:179-217`).
4. `rebuild_all` rebuilds the tree from the unchanged model — **the change is gone**.

So two of five bundled mutations have no lasting effect, never save, and each invocation adds a dead undo entry (the next Ctrl-Z "undoes" nothing — silently eating a real undo step from the user's perspective).

Same wipe kills **Toggle** behavior: `active_toggles` has zero consumers outside `apply_custom_mutation` itself — nothing re-applies active toggles after a rebuild, so a toggle-on's tree edit dies at the end of the same dispatch. CONCEPTS §4 promises "visual change … second trigger reverses"; in reality there is no visual change to reverse.

Every other mutable tree field is equally unsynced: line-height, container `bounds`, `outline`, `background`, `shape`, `zoom_visibility`.

Related no-op pollution (`custom/mod.rs:292-315`, `76-85`, `341-349`): when `flat_mutations` fails (non-flat AST → warn-and-skip) or the predicate filters everything, snapshots are still pushed and `dirty` still set.

## Fix plan

1. **Sync scale back**: in `sync_node_from_tree`, read tree-side `area.scale` and write it to run `size_pt` (all runs of the section, or introduce a section-level size if runs diverge). Decide + document the mapping for line-height.
2. For tree fields with **no model home** (outline, background, shape, zoom_visibility via mutator): either give them model homes or **reject at apply time** with `log::warn!` naming the unsupported field — silent partial application is the worst option (§5 no half-features).
3. **Toggle**: either re-apply `active_toggles` after every tree rebuild (hook in `rebuild_all` / `build_tree`), or persist toggle state in the model. Re-application is closer to the documented design.
4. **Gate undo/dirty on actual change**: have `apply_to_tree`/sync-back report whether anything changed; skip `undo_stack.push` and `dirty = true` otherwise (also covers the flat-mutations-failed and predicate-filtered-all paths).
5. Tests (same commit): (a) `mutation apply grow-font-2pt` → `rebuild_all` → assert the visual size survives AND the model changed AND exactly one undo entry that actually reverses it; (b) toggle-on → rebuild → assert still visually applied; toggle-off → reverted; (c) no-op apply pushes no undo entry.

## Acceptance criteria

- `grow-font-2pt` visibly grows text, survives rebuild + save/reload, and Ctrl-Z reverses it.
- Toggle mutations survive rebuilds while active.
- No undo entries for no-op applies.
- `./test.sh` green.

## Pointers

`src/application/document/custom/{mod.rs,sync.rs}`; `assets/mutations/application.json`; `lib/baumhard/src/gfx_structs/area_mutators.rs:163-168` (GrowFont = scale delta); `src/application/app/scene_rebuild.rs:367-383`; CONCEPTS.md §4 (Behaviors); TEST_CONVENTIONS §T1.
