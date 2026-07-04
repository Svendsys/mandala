# P1-12: `BorderRunSpec.line_height_pt` is dropped by both live border tree paths — the "gappy rails" fix only exists on the dead flat pipeline

**Severity:** P1 (shipped visual feature renders with the defect its field was invented to fix) · **Area:** baumhard border pipeline

## Problem

`border_run_specs` computes vertical-rail `line_height_pt` from **measured ink height** — its comment: "without this override … empty gap between rows that reads as 'gappy diamonds'" (`lib/baumhard/src/mindmap/border.rs:436-458`) — and sizes rail bounds/row counts from it (`left_v_height = left_row_count as f32 * left_line_h`, border.rs:563).

The only consumer honoring the field is the **dead** flat pipeline (`src/application/renderer/scene_buffers.rs:66-74`, `rebuild_border_buffers` — zero call sites; see P1-22). Both live tree paths discard it:

- `append_border_run` builds `GlyphArea::new_with_str(text, font_size, font_size, ...)` — font_size passed twice, line-height = font size (`lib/baumhard/src/mindmap/tree_builder/border.rs:323-356`).
- The border mutator path passes `spec.font_size_pt` twice (`border.rs:246-258`).
- Section frames inherit the drop (`tree_builder/section_frame.rs:135-152`).

The renderer's walker shapes with `area.line_height` and culls to `render_bounds` (`renderer/tree_walker.rs:137-163`), so a rail whose *bounds* were sized at `rows × ink_height` is *laid out* at `font_size` stride — rows gap, or overflow the buffer and vanish before the bottom corner. Affects any custom side pattern whose glyph ink-height ≠ font size (filled-glyph patterns — the exact case revision-4 addressed).

The mutator-vs-rebuild parity test (`tree_builder/tests/border.rs:234`) compares tree-vs-tree, so both being identically wrong passes.

## Fix plan

1. Thread the spec through: pass `&BorderRunSpec` into `append_border_run` and the mutator-path `GlyphArea` construction; set `line_height = spec.line_height_pt`.
2. Same for section frames.
3. Regression test (§T7): for a filled-glyph side pattern, assert `area.line_height == spec.line_height_pt` on the built tree (spec-vs-tree, not tree-vs-tree), and that rail row count × line height equals the rail bounds height.
4. Coordinate with P1-22 (dead flat pipeline removal): port this behavior **before** deleting `rebuild_border_buffers`, which is currently the only executable reference for the correct handling.

## Acceptance criteria

- Custom border patterns with tall/short ink render gap-free on the live path (visual check via `./run.sh` with a heavy/filled preset).
- New spec-vs-tree test fails on current main.
- `./test.sh` green.

## Pointers

`lib/baumhard/src/mindmap/border.rs:404-600`; `lib/baumhard/src/mindmap/tree_builder/border.rs`; `lib/baumhard/src/mindmap/tree_builder/section_frame.rs`; `src/application/renderer/tree_walker.rs:137-163`; CONCEPTS §3 (border geometry constants "must agree, or corner alignment drifts").
