# P1-30: Single-line editor duplication (~230 mirrored lines ×2 + tripled steal/commit blocks) and ThrottledInteraction covering only half its lifecycle

**Severity:** P1 (mechanical duplication in the modal/drag machinery; every new variant re-pays it) · **Area:** mandala/app

## Problem A — edge-label and portal-text editors are structural mirrors

`src/application/app/label_edit.rs:77-297` vs `312-553`: `LabelEditState` vs `PortalTextEditState` are field-for-field identical modulo the identity key; the `open_/handle_/close_` trios differ only in preview slot (`label_edit_preview` vs `portal_text_edit_preview`), refreshed tree (`update_connection_label_tree` vs `update_portal_tree`), commit setter, plus portal's `edge_still_valid` guard. Cursor/char primitives are already shared — the residue is pure lifecycle scaffolding (~230 mirrored lines). On top: three near-identical modal-steal blocks in `event_keyboard.rs:84-195` and three near-identical click-outside-commit blocks in `event_mouse_click.rs:403-518`. CONCEPTS already concedes portal-text "mirrors the label editor shape", and the dispatch arms treat them as one (`LabelEditCommit/Cancel` pick whichever is open).

**Fix:** a `SingleLineEditTarget` trait or enum (find, stage_preview, clear_preview, refresh, commit, hit_test_release) + one generic `SingleLineEditor<T>` holding `{buffer, cursor_grapheme_pos, original}` with one open/handle_key/close trio; collapse the steal and click-outside blocks to one each. The node text editor stays separate (multi-line + regions + tree preview is genuinely different). Coordinate with P1-17 (insertion primitive) — same files.

## Problem B — ThrottledInteraction trait covers only the drain half

Trait = `has_pending/throttle/drain/reset` (`throttled_interaction/mod.rs:126-193`). The other two lifecycle phases are open-coded per variant:

- **Accumulate**: five identical `total_delta += delta; pending_delta += delta;` arms + two cursor-overwrite arms in `event_cursor_moved.rs:144-189`.
- **Commit-on-release**: seven arms in `event_mouse_click.rs:598-846,1049-1140` (flush pending → commit model → undo → cache clear → rebuild); only NodeResize/SectionResize were extracted (`finalize_*_release`, whose own doc cites §5 for exactly this reason); MovingNode, MovingSection, EdgeHandle, PortalLabel, EdgeLabel remain inline.
- The 6-line `has_pending`/`throttle` accessor boilerplate repeats 8×.

CONCEPTS §5 sells the trait as "new throttled drags attach … without growing the dispatch" — but a new variant must grow two match ladders in two event files, and the planned WASM drag port would duplicate the release ladder again.

**Fix:** add `accumulate(&mut self, DragInput)` (`Delta(Vec2) | Cursor(Vec2)`) and `commit_on_release(&mut self, DrainContext)` to the trait; collapse the cursor-move Throttled arm and the left-release match to single `as_dyn_mut()` calls; the existing finalize helpers become two of the impls.

## Acceptance criteria

- One single-line editor implementation; grep shows one steal block, one click-outside-commit block.
- Adding a hypothetical eighth ThrottledDrag variant touches: one struct + one trait impl + one enum variant (as CONCEPTS promises) — no event-file match growth.
- All existing drag/editor tests pass; `./test.sh` green.

## Pointers

Files cited inline; CONCEPTS §5 (ThrottledInteraction), §6 (editors); CODE_CONVENTIONS §5, §6.
