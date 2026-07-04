# P1-26: Document setter fan-out — the EditNodeStyle/EditNodeText undo envelope and grow-pass tail are copy-pasted across ~12 sites; four edge-font setters bypass `mutate_edge`; edge color cascade open-coded at 6+ sites

**Severity:** P1 (largest duplication surface in the document layer; the envelope has already drifted once) · **Area:** mandala/document + baumhard model

## Problem A — snapshot envelopes copy-pasted

Three envelope helpers already exist and prove the pattern: `set_node_style_field` (`nodes/mod.rs:909-936`), `mutate_section_with_style_undo` (`section_text.rs:38-80`), `mutate_node_with_style_undo` (`section_structure.rs:44-91`), `mutate_edge` (`edges/structural.rs:379-396`, doc: "Single source of truth for the find idx → clone before → mutate → push undo template").

Yet the same snapshot → no-op-gate → mutate → undo-push → dirty envelope is open-coded at: `nodes/mod.rs:513-552, 563-607, 627-673`; `nodes/border.rs:237-309` (EditNodeStyle); `nodes/mod.rs:397-467`; `section_text.rs:87-145, 165-226, 228-285` (EditNodeText). Six byte-similar snapshot blocks were identified (`nodes/mod.rs:527-531 = 580-585 = 641-646 = border.rs:257-261 = section_structure.rs:57-61 = section_text.rs:52-56`). The grow-pass tail (`canvas_default` clone + `grow_one_node_to_fit_text` + `grow_one_node_to_fit_border`) repeats at **12+ sites**. The envelope has drifted before: `undo_action.rs:49-101`'s "Pre-fix" narrations document real shipped bugs (missing position/size/selection legs) caused by copies not updated together.

## Problem B — edge font setters bypass `mutate_edge`

`set_edge_font`, `set_edge_label_font`, `set_portal_text_font`, `set_edge_font_family` (`edges/style.rs:259-316, 331-353, 370-461, 471-585`) each re-implement position-lookup → before-clone → mutate → rollback-on-noop → push(EditEdge) → dirty, including **three copies** of the `final_min`/`final_max` inversion guard around ~60-line near-clone bodies. The zoom-bounds channels prove the target shape: all four go through `mutate_edge` + `OptionEdit` with zero copy-paste (`edges/zoom_bounds.rs` — the exemplar).

## Problem C — edge/label/portal color cascade

The chain `override.color → glyph_connection.color → edge.color` is re-typed at 6+ sites across two crates (`scene_builder/connection.rs:336`; `label.rs:177-179` AND `234-236`; `portal.rs:168-170`; `document/edges/style.rs:113-160` re-implements all three tiers as read helpers) — while the zoom cascade got proper SSOT helpers on `MindEdge` (`label_zoom_window`/`portal_endpoint_zoom_window`, each documented "Single source of truth").

## Problem D — default TextRun template

`TextRun { font: "LiberationSans", size_pt: 24, color: "#ffffff" }` hardcoded at four sites (`defaults.rs:42-52`, `nodes/mod.rs:424-438`, `section_text.rs:244-254`, plus a 14pt variant at `section_text.rs:562-576` and constants in `custom/sync.rs:30-34`) — the 14-vs-24 split already looks like unintended drift. Also `traits/view.rs:762-777` vs `:858-868` duplicate it twice more in one file.

## Fix plan

1. One `mutate_node_with_style_undo(doc, id, grow: GrowPass, f)` for all EditNodeStyle producers (fold the three existing helpers; adopt the no-op-restore semantics of `mutate_section_with_style_undo`; opt-in grow tail). Sibling `mutate_node_with_text_undo`.
2. Fold the four edge-font setters into `mutate_edge` closures; extract one `apply_font_triple(slots, size, min, max)` core (a `FontTripleSlots` accessor view over the three channels).
3. Add `MindEdge::body_color(&Canvas)`, `::label_color(..)`, `::portal_endpoint_color(endpoint)` mirroring the zoom helpers; route scene_builder + document reads through them.
4. One `default_text_run(end) -> TextRun` (+ color/size overrides) in `defaults.rs`; reconcile 14-vs-24 deliberately; replace all sites.
5. Drive-by (same files): `mutate_section_runs_in_range` (`section_text.rs:484-544`) uses the post-hoc `undo_stack.pop()` anti-pattern its own file header condemns and leaks `dirty=true` on no-op — rebuild it on the envelope with an honest changed-verdict.

## Acceptance criteria

- The snapshot envelope exists once per (style/text) kind; grep shows no open-coded copies.
- All edge setters flow through `mutate_edge`.
- Cascade knowledge exists once (baumhard helpers); scene_builder + console reads agree by construction.
- All existing undo round-trip tests pass; `./test.sh` green.

## Pointers

CODE_CONVENTIONS §5; the in-repo exemplars named above (zoom_bounds.rs, mutate_edge, effective_size). Coordinate with P1-15 (palette wiring touches the same cascade sites).
