# P2-37: Interaction rebuild-tier misuse — every native click runs the coarsest rebuild; rect-select rebuilds the whole tree per drain; every console command rebuilds unconditionally

**Severity:** P2 (per-interaction latency on large maps; the right-sizing helpers already exist) · **Area:** mandala/app

## Problems

1. **Native `handle_click` always runs `rebuild_all`** (`click.rs:106-110`) — full `doc.build_tree()` + cosmic-text buffer rebuild for every click outcome, even edge-label↔edge-label selection changes. `rebuild_after_selection_change` exists precisely for this (its doc says so verbatim) and is already used by the **WASM** release path and one native branch — the browser is better optimized than the desktop here. **Fix:** snapshot `prev = doc.selection.clone()`, finish with `rebuild_after_selection_change(&prev, ...)`; keep forcing `rebuild_all` when an OnClick trigger actually fired.
2. **Rubber-band selection** (`drain_frame.rs:25-56`): `drain_selecting_rect` runs `doc.build_tree()` + `apply_tree_highlights` + `rebuild_buffers_from_tree` on every drain during Shift+drag — the most expensive operation in the app, per cursor-move batch, deliberately outside the throttle with a comment calling it a "lightweight overlay redraw" (`app/mod.rs:495-502` — the comment is wrong). **Fix:** promote rect-select to a `ThrottledDrag` variant (fits P1-30's completed trait), or keep the tree and apply region-color deltas only for nodes entering/leaving the rect; correct the comment either way.
3. **Console executes every verb with `scene_cache.clear()` + `rebuild_all`** (`console_input/exec.rs:96-98`) — including `help`, `fps`, `mutation list`, and failed parses. **Fix:** add `effects.document_mutated` (the command cores already return mutation-ness bools) and gate the clear+rebuild on it.
4. **Sibling promotions pick different tiers**: EdgeLabel threshold-cross promotion uses `rebuild_after_selection_change` (with an explanatory comment); the structurally identical PortalLabel promotion calls unconditional `rebuild_all` (`event_cursor_moved.rs:226-310`). **Fix:** mirror the EdgeLabel arm.
5. **Middle-press mid-drag destroys an in-flight throttled drag** (`event_mouse_click.rs:95-119` vs :943-954): MiddleClick dispatch overwrites `DragState::Throttled(...)`; release forces `DragState::None`; the abandoned drag's release-commit never runs — tree keeps dragged offsets until the next model rebuild snaps them back (silent position loss, no undo). The right-button handler already refuses to clobber and names middle-click's overwrite as the posture it rejects. **Fix:** mirror the right-button guard on middle-press.

## Acceptance criteria

- Click latency on a large map no longer includes a full tree rebuild for selection-only changes (verify by tier-choice reading + optional timing).
- Rect-select no longer rebuilds the arena per drain.
- `help` in the console does not clear the scene cache.
- Mid-drag middle-press leaves the drag intact (test at the DragState level).
- `./test.sh` green.

## Pointers

`src/application/app/{click.rs,drain_frame.rs,mod.rs,event_cursor_moved.rs,event_mouse_click.rs}`; `src/application/app/console_input/exec.rs`; `src/application/app/scene_rebuild.rs:48-75` (the right-sizing helper); CODE_CONVENTIONS §4/§B1.
