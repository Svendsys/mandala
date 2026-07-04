# Deep Code-Quality Review — src/application/document/, clipboard.rs, user_config/

## Architecture assessment

The document layer is a well-disciplined, heavily documented mutation surface: nearly every setter follows the snapshot → no-op-gate → mutate → undo-push → dirty idiom, the undo variant set is complete with round-trip tests for all 13 variants, fixtures are centralized per §T4, and the edge/portal/zoom channels route through genuinely shared cores (`mutate_edge`, `write_endpoint_field`, `OptionEdit`). The two structural weaknesses are (1) the model⇄tree bridge for custom mutations: `sync_node_from_tree` writes back only {text, runs, position, section offset/size}, so every other field the mutator language can touch (scale/font-size, bounds, outline, background, shape, zoom-visibility) — including both bundled font mutations and the entire Toggle behavior — is silently reverted by the `rebuild_all` that follows every dispatch; and (2) the node/section setter fan-out, where the EditNodeStyle/EditNodeText snapshot envelope and the grow-pass pair are copy-pasted across ~12 open-coded sites despite three helpers already existing for exactly that envelope. On top of that sit one hard data-corruption bug (delete-highest-root + undo), an animation-completion double-apply, and a selection variant (`SectionRange`) whose `range` field means two different things to two halves of the codebase.

## Findings

### F1. DeleteNode undo corrupts the map when the deleted node was the highest-numbered root with children
Severity: P0 | Category: correctness | Confidence: high
Files: src/application/document/topology.rs:21-63 (delete order), topology.rs:243-263 (fresh_child_id), src/application/document/undo.rs:123-143 (DeleteNode arm)
Evidence: `delete_node` removes the node **first** (`let node = self.mindmap.nodes.remove(node_id)?;`), then mints orphan ids via `fresh_child_id(None)` = max remaining root segment + 1. With roots `"0".."3"` and `"3"` (has child `"3.0"`) deleted, remaining max root = 2 → first orphan is cascade-renamed to `"3"` — the just-deleted node's own id. On undo, `self.mindmap.nodes.insert(restored_id.clone(), node)` (undo.rs:129) **overwrites** the renamed child (HashMap::insert replaces silently → child data destroyed), then `cascade_rename("3", "3.0")` renames the *restored parent* to `"3.0"` and sets its `parent_id = Some("3")` — dangling. Net after delete + Ctrl-Z: child node lost, parent mangled, dangling parent_id, self-edge possible via the edge-rename pass.
Why it matters: §3 "every user-facing mutation gets a matching UndoAction + undo() branch" — this branch destroys data on a routine gesture. CONCEPTS claims fresh ids are minted "without reusing deleted gaps", but remove-before-mint reuses exactly the freed max slot. tests_delete.rs deletes root "0" but never undoes a root delete, and `find_node_with_children_and_parent` always picks a non-root — zero coverage of the collision case.
Fix: Mint orphan ids against an id-space that still contains the deleted node (compute fresh ids before `nodes.remove`, or treat the deleted id as reserved in `fresh_child_id`). Defense in depth: `cascade_rename` and the undo arm refuse to overwrite an existing key (log + re-mint). Regression test: delete max-numbered root with a child and a grandchild, undo, assert full restoration.
Effort: S

### F2. Tree-only mutation effects don't survive the post-dispatch rebuild: bundled grow/shrink-font are no-ops; Toggle behavior is visually broken
Severity: P0 | Category: correctness | Confidence: high
Files: src/application/document/custom/sync.rs:148-370 (sync set = position/text/runs/offset/size only), src/application/document/custom/mod.rs:120-217 (Persistent + Toggle paths; undo push at 211-216), assets/mutations/application.json (grow-font-2pt / shrink-font-2pt use GrowFont/ShrinkFont), lib/baumhard/src/gfx_structs/area_mutators.rs:163-168 (GrowFont = `scale` delta), src/application/app/scene_rebuild.rs:367-383 (rebuild_all = `doc.build_tree()` from model), src/application/app/console_input/exec.rs:98 + src/application/app/click.rs:109 (rebuild_all after every dispatch)
Evidence: `sync_node_from_tree` doc: "pull node.position … and every section's (text, text_runs, offset, size)". Nothing reads back `area.scale`, line-height, container `bounds`, `outline`, `background`, `shape`, or `zoom_visibility`. So `grow-font-2pt` (Persistent, SelfAndDescendants) grows the tree scale, the selective gate sees unchanged regions and skips, the model stays byte-identical — yet `apply_custom_mutation` still pushes `UndoAction::CustomMutation` and sets `dirty = true`. The `rebuild_all` that both dispatch paths run immediately afterwards rebuilds the tree from the unchanged model: **the flagship bundled mutation has no lasting effect, never saves, and leaves a dead undo entry per invocation.** Same wipe kills Toggle: `active_toggles` has zero consumers outside `apply_custom_mutation` itself (grep confirms: only custom/mod.rs:129-145 + tests) — no re-application after rebuild exists, so a toggle-on's tree edit is destroyed by the rebuild at the end of the same dispatch, while CONCEPTS §4 promises "visual change without model commit, second trigger reverses".
Why it matters: violates the documented Persistent contract ("snapshot, apply, sync back, push undo") for every mutator field outside the sync set; two of five bundled mutations dead; the Toggle variant is dead weight; undo stack accumulates entries whose undo() is a visual no-op. tests_mutations.rs asserts sync-back only for position/regions/offset — scale and rebuild-survival are untested.
Fix: Read `scale` back into per-run `size_pt` in `sync_node_from_tree` (and either give line-height/outline/bounds model homes or warn-and-reject those mutations at apply time); re-apply `active_toggles` after every tree rebuild (hook in rebuild_all walking the set through `apply_to_tree`) or model-persist toggles; gate undo-push/dirty on "sync-back changed the model". Add rebuild-survival tests for grow-font and Toggle.
Effort: L

### F3. Animation completion double-applies relative mutations and snapshots undo mid-lerp
Severity: P0 | Category: correctness | Confidence: high (structural trace; with-tree path untested)
Files: src/application/document/animations.rs:343-401 (tick completion), src/application/app/drain_frame.rs:121-145 (rebuild_all per advancing tick), src/application/document/custom/mod.rs:179-217 (snapshots current model, applies to current tree)
Evidence: Each advancing frame lerps `model.position` toward `to`, then `drain_animation_tick` runs `rebuild_all` → the live tree holds the lerped position. On the completing frame (`elapsed >= total`, no final lerp write) the instance drains via `self.apply_custom_mutation(&anim.cm, &anim.target_id, Some(tree))`, which applies the FULL `NudgeRight(50)` on top of the lerped tree (`x ≈ from + 50·t_prev`) and syncs back: final `x ≈ from + 50·(1 + t_prev)` — approaching double the delta for any animation longer than one frame. The undo snapshot inside that call captures the *mid-lerp* model, so Ctrl-Z restores `from + 50·t_prev`, not `from`. Every animation test passes `tree = None` (tests_mutations.rs:1242-1458; tests_hit_move.rs:319-512), exercising only the `node.position = to_node.position` fallback — the production `Some(tree)` completion path has zero coverage.
Why it matters: §T1 ranks mutation/undo round-trips as the #1 fundamental; natural completion corrupts both the final state and the undo baseline for every animated relative mutation.
Fix: At completion, reset target state to `anim.from_node` (model + tree) before routing through `apply_custom_mutation`, and snapshot undo from `from_node`; add a completion test that passes a lerped tree.
Effort: M

### F4. SelectionState::SectionRange's `range` field has two contradictory meanings, and the variant is unconstructable in production
Severity: P1 | Category: ssot / correctness | Confidence: high
Files: src/application/document/types.rs:193-200 (doc: "pair of **section indices**"), types.rs:393-402 (`selected_range` doc: "shift-selected a sub-range inside a section" = graphemes), src/application/document/nodes/border.rs:978-990 (`live_selection_section_pairs` expands range as section indices), src/application/document/nodes/section_structure.rs:149-168 (clamps as section indices), src/application/console/commands/font.rs:338-346 + 679 (consumes as grapheme range → `set_section_font_size_range`), src/application/console/commands/color.rs:176-186 (picker sub-range = graphemes), src/application/console/traits/view.rs:626-631 + 678-682 (TargetId carries range as "grapheme indices"), src/application/app/text_edit/editor.rs:257-294 (`lift_anchor_to_section_range` — the only intended producer — now returns `Section` and *discards* the range; its doc asserts "every SectionRange consumer … interprets the range field as section indices", which is false), CONCEPTS.md §5 SelectionState entry (documents grapheme semantics + Cut/Paste/picker contracts)
Evidence: `live_selection_section_pairs`: `SelectionState::SectionRange { sel, range } => (lo..=hi).map(|i| (sel.node_id.clone(), i))` — section indices. `font.rs:344`: `let effective_range = a.range_target.or(Some(range)); section_font_outcome(…, effective_range)` — grapheme bounds fed to `set_section_font_size_range`. If any future producer emits grapheme ranges (as CONCEPTS and half the consumers expect), border-preview drift checks and structural-mutation clamps will misinterpret them as section indices (e.g. grapheme range (3,17) reads as "sections 3..=17"), and vice versa. Today no production code constructs the variant at all (grep: constructors are the editor lift — which emits `Section` — plus `cleanup_after_structural_mutation`'s re-clamp of an existing value and tests), so the grapheme-consuming verb arms, the picker sub-range plumbing, and the documented "SectionRange Cut/Paste → NotApplicable" contract are all currently dead code documented as live.
Why it matters: §2 "reach for the existing seam, do not add a parallel path" / SSOT — one field, two units, three documentation sources disagreeing (types.rs vs its own accessor doc vs CONCEPTS). The next contributor wiring shift-select (per CONCEPTS) will re-introduce grapheme ranges and silently break the section-index consumers.
Fix: Decide one meaning. If grapheme: rename to `grapheme_range`, fix `live_selection_section_pairs`/`cleanup_after_structural_mutation` to treat SectionRange as a single-section selection, and re-wire the editor lift to actually emit it. If section-index: rename to `section_span`, delete the grapheme verb arms + `TargetId::Section.range` carry, and correct CONCEPTS.md + `selected_range` docs. Either way add a type-level distinction (newtype) so the units can't silently cross again.
Effort: M
### F5. Setter fan-out: the EditNodeStyle/EditNodeText snapshot envelope and grow-pass pair are copy-pasted across the node/section matrix
Severity: P1 | Category: duplication | Confidence: high
Files (the full matrix):
- Shared envelopes that already exist: `set_node_style_field` (src/application/document/nodes/mod.rs:909-936), `mutate_section_with_style_undo` (nodes/section_text.rs:38-80), `mutate_node_with_style_undo` (nodes/section_structure.rs:44-91), `mutate_edge` (edges/structural.rs:379-396).
- Open-coded copies of the same 5-field snapshot (before_style/before_sections/before_position/before_size/before_selection) + push EditNodeStyle + dirty: `set_node_text_color` (nodes/mod.rs:513-552), `set_node_font_size` (563-607), `set_node_font_family` (627-673), `set_node_border_config` (nodes/border.rs:237-309).
- Open-coded EditNodeText envelope (before_sections/position/size/selection + push + dirty): `set_node_text` (nodes/mod.rs:397-467), `set_section_text` (section_text.rs:228-285), `set_section_text_and_runs` (87-145), `set_section_text_preserving_runs` (165-226).
- The `canvas_default = canvas.default_border.clone(); grow_one_node_to_fit_text(node); grow_one_node_to_fit_border(node, canvas_default.as_ref())` tail is repeated at 12 sites: nodes/mod.rs:90-109, 161-173, 199-212, 244-268, 296-321, 366-394, 444-457, 585-596, 646-662; section_text.rs:127-135, 209-216, 268-275, 347-360, 388-402, 512-541, 608-630; section_structure.rs:62-87.
- Channel × property matrix for font size/min/max: `set_edge_font` (edges/style.rs:259-316), `set_edge_label_font` (370-461), `set_portal_text_font` (471-585) are three ~60-line near-clones of resolve-final-(min,max) → reject-inverted → write min → write max → clamp+write size → rollback-on-noop → scrub-empty-config → push EditEdge; only the target struct and clamp fallback differ.
- Those three (plus `set_edge_font_family`, style.rs:331-353) also bypass `mutate_edge`, re-open-coding its idx-lookup/before-clone/rollback/push template — the exact template the module banner (structural.rs:319-340) says is single-sourced in `mutate_edge`.
- Node-vs-section pairs with duplicated bodies differing only in section scope: `set_node_font_size` vs `set_section_font_size` (section_text.rs:331-362), `set_node_font_family` vs `set_section_font_family` (370-404), `set_node_text_color` vs `set_section_text_color` (292-322).
Evidence (representative): nodes/mod.rs:527-531 = nodes/mod.rs:580-585 = nodes/mod.rs:641-646 = border.rs:257-261 = section_structure.rs:57-61 = section_text.rs:52-56 — six byte-similar snapshot blocks; edges/style.rs:277-283 vs 405-413 vs 513-521 — three copies of the `final_min`/`final_max` inversion guard.
Why it matters: §5 "identical logic copy/pasted … the answer is never to copy it". The envelope has already drifted once (EditNodeText gained `before_position/size/selection` late — the "Pre-fix" comments in undo_action.rs:49-66 document bugs caused precisely by copies not being updated together). Every new field in the snapshot must currently be added at ~10 sites.
Fix: (1) Make every EditNodeStyle producer route through one `mutate_node_with_style_undo(doc, id, grow: bool, f: FnOnce(&mut MindNode) -> bool)` (fold `set_node_style_field` + section_structure's helper into it; give it the no-op-restore semantics of `mutate_section_with_style_undo` and an opt-in grow tail). (2) Same for a `mutate_node_with_text_undo`. (3) Extract a `FontTripleSlots { size, min, max, fallback_min, fallback_max }` view + one `apply_font_triple(slots, size, min, max) -> bool` core, and route all three channel setters through `mutate_edge`. (4) Collapse the node/section run-rewrite pairs onto one core taking a section filter.
Effort: L

### F6. `mutate_section_runs_in_range` uses the documented anti-pattern: post-hoc `undo_stack.pop()` and leaks `dirty = true` on no-op
Severity: P2 | Category: correctness / convention | Confidence: high
Files: src/application/document/nodes/section_text.rs:484-544 (esp. 530-533), contradicting the helper contract at section_text.rs:24-30
Evidence: `mutate_section_with_style_undo`'s doc sells itself as the fix for "the caller having to itself snapshot + post-hoc `undo_stack.pop` (which doesn't restore `dirty` and breaks the undo-LIFO invariant …)". Forty lines later `mutate_section_runs_in_range` does exactly that: it always returns `true` from the closure (so undo is pushed and `dirty` set), then compares pre/post run clones and `self.undo_stack.pop(); return false;` — leaving `dirty = true` after a no-op ranged set (one spurious full rebuild) and re-introducing the LIFO fragility the doc warns about. It also clones the full `text_runs` twice per call purely for the comparison.
Why it matters: §5 (the file's own stated discipline is violated in the same file); a no-op `color text=… range=…` costs a scene rebuild.
Fix: Compute the mutated run set on a scratch clone first (or have `text_run_ops::mutate_in_range` report changed), then call `mutate_section_with_style_undo` with an honest verdict.
Effort: S

### F7. `commit_border_preview` re-implements `edits_touch_cfg_field` inline — the exact one-field-drift risk the codebase already fixed once
Severity: P2 | Category: duplication | Confidence: high
Files: src/application/document/nodes/border.rs:734-746 (inline seven-field `touches_any_field` + `edits_touch_glyphs`) vs border.rs:925-934 (`edits_touch_cfg_field`); cautionary precedent at src/application/document/mod.rs:774-781 (`view_implies_visible` was deliberately rewired onto `touches_any_field` because "the previous parallel implementation drifted by one field")
Evidence: the commit-time force-show predicate lists `preset/font/font_size_pt/color/padding/color_palette/color_palette_field` + `edits_touch_glyphs` — byte-equivalent to `edits_touch_cfg_field`. Adding a ninth border field updates one and misses the other, silently breaking the "commit shows what preview showed" C8 contract.
Fix: `let touches_any_field = edits_touch_cfg_field(&commit_edits);`
Effort: S

### F8. App crate bypasses `grapheme_chad` with direct `unicode_segmentation` calls (§1 violation)
Severity: P2 | Category: convention (§1 extend-Baumhard) | Confidence: high
Files (in scope): src/application/document/nodes/section_structure.rs:311, 332 (`original_text.graphemes(true).count()` → should be `grapheme_chad::count_grapheme_clusters`), 349-353 (`grapheme_indices(true).nth(split_grapheme)` → should be `grapheme_chad::find_byte_index_of_grapheme`). Same pattern elsewhere in the crate (corroborating, outside strict scope): console/commands/section/mod.rs:180,196,199,464-468; app/console_input/completion.rs:88-123; app/console_input/edit.rs:170-179; app/text_edit/mod.rs:171-190.
Evidence: CONVENTIONS §B3: "All text primitives live in grapheme_chad… If you need to manipulate a String or &str, call a function from that file." CODE_CONVENTIONS §1: "Text through baumhard::util::grapheme_chad — grapheme-aware primitives for every String/&str manipulation."
Why it matters: the app crate grows a parallel Unicode path; grapheme_chad's tests/benches no longer cover the segmentation the app actually ships. Where an existing primitive is missing (e.g. "preview first N clusters"), §1 says add it to grapheme_chad, not import unicode_segmentation locally.
Fix: Replace section_structure.rs's two uses with `count_grapheme_clusters` / `find_byte_index_of_grapheme`; sweep the sibling sites, adding small grapheme_chad primitives (`take_clusters`, `nth_cluster_byte_offset`) where needed, with `do_*` tests + bench entries per §B3.
Effort: M

### F9. `tree_cascade` panics (assert!) on cycle detection inside an interactive path
Severity: P2 | Category: convention (§9) | Confidence: med (deliberate trade-off, but §9 is categorical)
Files: src/application/document/mutations/tree_cascade.rs:34-46
Evidence: `assert!(iterations <= iteration_budget, "tree-cascade BFS exceeded …")` — reachable from `apply_custom_mutation` → console `mutation apply` / click triggers, i.e. inside `Application::run`. §9: "Interactive paths must not panic … Degrade the frame, log via log::warn!/log::error!, keep running." The comment argues freeze→crash conversion, but §9 offers the sanctioned alternative: log::error! + return (the layout simply doesn't apply). `flower_layout` (sibling handler) degrades gracefully everywhere.
Fix: `if iterations > budget { log::error!(…); return; }` and keep the existing `#[should_panic]` test as a should-log/should-return test.
Effort: S

### F10. `set_edge_font`/`set_edge_label_font`/`set_portal_text_font`/`set_edge_font_family` bypass `mutate_edge`
Severity: P2 | Category: duplication / ssot | Confidence: high
Files: src/application/document/edges/style.rs:259-316, 331-353, 370-461, 471-585 vs the canonical template at edges/structural.rs:319-396
Evidence: each re-implements `position(|e| edge_ref.matches(e))` → `before = edges[idx].clone()` → mutate → `if !changed { edges[idx] = before; return false; }` → `push(EditEdge)` → `dirty = true`. `mutate_edge`'s doc calls itself the "Single source of truth for the find idx → clone before → mutate → push undo template". The closures can reach everything they need (`ensure_glyph_connection_inline(edge, canvas)`, `GlyphConnectionConfig::resolved_for(edge, canvas)`).
Why it matters: §5 duplication; a future change to the edge-undo discipline (e.g. dirty semantics, index invalidation) must touch 5 sites instead of 1. Overlaps F5(c) — fixing F5's triple-core naturally lands these on `mutate_edge`.
Fix: fold all four into `self.mutate_edge(edge_ref, |edge, canvas| { … })`.
Effort: M

### F11. `apply_custom_mutation` pushes undo + dirty even when the apply was a guaranteed no-op
Severity: P2 | Category: correctness | Confidence: high
Files: src/application/document/custom/mod.rs:179-217 (snapshots collected and pushed unconditionally when non-empty), 292-315 (non-flat mutator warn-and-skip inside `apply_to_tree`), 341-349 + 76-85 (predicate-filtered-everything warn)
Evidence: when `flat_mutations` fails (non-flat AST) or the predicate filters all candidates, `apply_to_tree` returns having changed nothing — but the caller has already collected `snapshots` and proceeds to `undo_stack.push(UndoAction::CustomMutation { … })` + `dirty = true`. The user's next Ctrl-Z then "undoes" nothing (and clobbers any interleaved legitimate expectation of what undo will hit). Same for the F2 grow-font case.
Why it matters: §3 undo discipline — entries should correspond to actual changes; the warn logs acknowledge the no-op but the undo stack doesn't.
Fix: have `apply_to_tree` (and sync-back) report whether anything changed; gate the push/dirty on it.
Effort: S
### F12. Default TextRun template hardcoded in four places (LiberationSans / 24pt / #ffffff)
Severity: P2 | Category: duplication | Confidence: high
Files: src/application/document/defaults.rs:42-52 (`default_orphan_node`), src/application/document/nodes/mod.rs:424-438 (`set_node_text` fallback), src/application/document/nodes/section_text.rs:244-254 (`set_section_text` fallback), section_text.rs:562-576 (`clamp_range_and_build_template` — same shape but `color: node.style.text_color`, `size_pt: 14`)
Evidence: three byte-identical `TextRun { …, font: "LiberationSans", size_pt: 24, color: "#ffffff", … }` literals plus one near-variant; the fourth's divergence (14pt vs 24pt) is exactly the kind of drift a single `default_text_run(template_source)` constructor in defaults.rs would surface or prevent. sync.rs adds its own default pair (`DEFAULT_TEXT_RUN_COLOR = "#ffffff"`, `DEFAULT_TEXT_RUN_SIZE_PT = 14`) — a fifth partial statement of the same defaults.
Why it matters: §5; changing the app default font/size requires a four-file hunt today, and the 24-vs-14 split already looks like unintended drift.
Fix: one `pub(crate) fn default_text_run(end: usize) -> TextRun` (+ a `with_color` variant) in defaults.rs; reference it everywhere; reconcile 14 vs 24 deliberately.
Effort: S

### F13. Orphaned/duplicated doc comments and a duplicate `#[test]` attribute
Severity: P3 | Category: docs | Confidence: high
Files: src/application/document/nodes/mod.rs:720-746 (four stacked `///` blocks — the zoom-pair-guard doc, the `clamp_runs_to_text` doc, and the verify-parity doc are all attached to `validate_node_size`, whose real doc is the fourth block; `validate_zoom_pair` at 886 is left undocumented), src/application/document/nodes/section_structure.rs:708-720 (the "Pin the node.size undo restoration" doc block is orphaned above a stray `#[test]` at 714, and `add_section_rejects_at_cap` at 721 carries TWO `#[test]` attributes — compiles, but rustc emits the `duplicate_macro_attributes` warning, verified: `warning: duplicated attribute`)
Why it matters: §8 "a doc comment that lies … is worse than no doc comment"; the misplaced blocks describe functions in other files; the doubled attribute is warning noise in every test build.
Fix: move each block to its owner (`validate_zoom_pair`, `clamp_runs_to_text`, `validate_section_aabb`), delete the stray `#[test]`, and re-home the orphaned undo-restoration doc onto `add_section_undo_restores_node_size_when_floor_pass_grew_it` (section_structure.rs:744).
Effort: S

### F14. British spellings throughout (project mandates American English)
Severity: P3 | Category: convention (CLAUDE.md §6) | Confidence: high
Files: 75 occurrences across 20 files in src/application/document/ alone (grep `colour|behaviour|serialis|materialis|honour|…`): e.g. mod.rs:180 "never serialised", types.rs:141/151 "colour", hit_test.rs:42/160 "behaviour", custom/sync.rs:26-70 "colour"/"recoloured", nodes/section_text.rs (8), tests_hit_move.rs (11), tests_nodes.rs (11), animations.rs, borders, edges. All in comments/docs (identifiers are clean).
Why it matters: CLAUDE.md §6 "Use American English for consistency"; §2 of the CLAUDE instructions explicitly forbids skipping "merely cosmetic" items.
Fix: mechanical sweep (colour→color, behaviour→behavior, serialised→serialized, materialise→materialize, honoured→honored, recognise→recognize, normalise→normalize, centre→center, artefact→artifact) across the module; keep quoted format-spec strings untouched.
Effort: S

### F15. Dead code: `build_mutation_registry_with_user` has no callers
Severity: P3 | Category: dead-code | Confidence: high
Files: src/application/document/animations.rs:66-70
Evidence: grep across src/ finds only the definition; the doc says "Variant retained for callers that already supply a user slice" but every caller uses `build_mutation_registry()` or `build_mutation_registry_with_app_and_user`.
Why it matters: §5 "no dead code"; §10 delete rather than deprecate.
Fix: delete it (callers that ever need it can pass `&[]` as app slice inline).
Effort: S

### F16. Stale `#[allow(unused_imports)]` + "until commit 5 lands" comment on a now-used re-export
Severity: P3 | Category: dead-code / docs | Confidence: high
Files: src/application/document/mod.rs:78-84
Evidence: "Triggers an unused-import warning until commit 5 lands; suppress." — `BorderPreviewTarget` is now imported through this path by console/commands/section/frame.rs:526, border verbs, and multiple test modules; the allow and its justification are stale.
Fix: drop the `#[allow(unused_imports)]` and rewrite the comment to a plain "re-exported for the border verbs" note.
Effort: S

### F17. Click-vs-drag hit-priority orders are opposite for node-vs-portal, and only one of them matches CONCEPTS
Severity: P3 | Category: api-design / docs | Confidence: med (each order is individually deliberate and documented in code)
Files: src/application/app/mod.rs:260-315 (`compute_click_hit`: node → portal-text → portal-icon → edge-label; "node hits beat portal hits"), CONCEPTS.md §5 DragState ("Hit priority on Pending is fixed: edge handle > portal label > edge label > node, so small grab-areas always win over larger AABBs"), src/application/app/click.rs:66-100 (fallback click path: portal-text → portal-icon → edge), src/application/renderer/hit.rs:119-147 (portal icon/text scan order notes)
Evidence: a portal marker floating over a node is *draggable* from that spot (drag priority: portal label > node) but not *click-selectable* (click priority: node > portal) — the same pixel routes to different targets depending on whether the press turns into a click or a drag. CONCEPTS documents only the drag order, presenting "small grab-areas always win" as the principle; the click funnel deliberately inverts it.
Why it matters: single-source-of-truth for interaction rules; the divergence is invisible in docs (CONCEPTS states one rule as if global) and will surprise both users and contributors.
Fix: either align click priority with drag (small targets win, with the node reachable by a second click), or document the split explicitly in CONCEPTS §5 next to the DragState entry.
Effort: S (doc) / M (behavior)

### F18. `cascade_rename` is O(renames²)+O(edges·renames) and silently overwrites on key collision
Severity: P3 | Category: performance / correctness-hardening | Confidence: high
Files: src/application/document/topology.rs:68-114
Evidence: parent-id fixup runs the inner `for (ro, rn) in &renames` per renamed node (quadratic in subtree size); edge rewrite is `for edge … for (old, new) in &renames` (edges × renames). Deleting a node whose orphaned child owns a 5k-node subtree does ~25M string compares at user-event time. `self.mindmap.nodes.insert(new.clone(), node)` (line 99) silently replaces any existing entry — the mechanism that turns F1 into data loss instead of an error.
Why it matters: §4 mobile budget (a large delete stalls the single-threaded loop); the silent-overwrite is the corruption amplifier for any future id-collision bug.
Fix: build a `HashMap<old, new>` for the parent fixup and edge rewrite (O(n)); `debug_assert!`/log on insert-overwrite.
Effort: S

### F19. `SelectionState::is_selected` allocates a String per query for `Multi`
Severity: P3 | Category: performance | Confidence: high
Files: src/application/document/types.rs:315 (`ids.contains(&node_id.to_string())`)
Evidence: called per candidate node in hover/highlight paths (e.g. event_cursor_moved.rs:759, selection-highlight assembly per rebuild); the allocation is avoidable: `ids.iter().any(|i| i == node_id)`.
Why it matters: §B1-adjacent hygiene; trigger frequency is per-node-per-rebuild/hover rather than per frame per glyph, so impact is small but the fix is free.
Effort: S

### F20. `set_*_font_size`/`_family` silently no-op on run-less sections (`.all()` on empty iterator)
Severity: P3 | Category: correctness | Confidence: med
Files: src/application/document/nodes/mod.rs:569-577 + 633-638, src/application/document/nodes/section_text.rs:343 + 384
Evidence: `already = section.text_runs.iter().all(|r| r.size_pt == size_u)` is vacuously true for a section whose `text_runs` is empty (legal state: `set_section_text("")` produces empty runs; loader accepts run-less sections whose text renders with defaults). `font size=…` on such a section returns false with no effect and no message — the user cannot set a size until a run exists.
Fix: treat empty-runs-with-nonempty-text as "create the default run at the new size" (reuse F12's template), or return a distinguishable "no runs" outcome the verb can surface.
Effort: S

### F21. CONCEPTS.md says UndoAction has "12 variants" — code has 13 (`EditNodeAabb` missing from the list)
Severity: P3 | Category: docs | Confidence: high
Files: CONCEPTS.md §5 `UndoAction` entry ("A 12-variant tagged union… The twelve variants: MoveNodes, …, DeleteNode") vs src/application/document/undo_action.rs:13-144 (13 variants incl. `EditNodeAabb`)
Also stale: CONCEPTS' SelectionState/SectionRange entry (see F4) and the Clipboard entry's "WASM stubs warn-and-noop" vs clipboard.rs:43-50 which logs at `debug` level.
Fix: refresh the CONCEPTS entries in the same commit as any F4 resolution.
Effort: S

### F22. Animation completion without a tree loses undo coverage (latent)
Severity: P3 | Category: correctness (latent) | Confidence: med
Files: src/application/document/animations.rs:388-395 (tick fallback), 441-458 (`fast_forward_animations` fallback)
Evidence: the `None`-tree fallback writes `node.position = anim.to_node.position` directly with no undo entry ("Undo path is then the caller's responsibility" — no caller takes it). Production native paths always pass `Some(tree)`, so this is latent; but any future WASM/console path that ticks without a tree silently produces an un-undoable position write.
Fix: push a `MoveNodes` entry in the fallback, or make the fallback route through the same snapshot discipline.
Effort: S
### F23. Test suite: high signal density, but the three P0 paths above are exactly the uncovered ones
Severity: P2 | Category: testing | Confidence: high
Files: src/application/document/tests_nodes.rs (2790), tests_hit_move.rs (1517), tests_mutations.rs (1458), tests_edges_style.rs (1274), tests_delete.rs (671), tests_edges_chain.rs (374), tests_reparent.rs (245), tests_selection.rs (433), tests_resize.rs (159), tests_common.rs (353)
Evidence: strengths — every one of the 13 UndoAction variants has a forward-and-back test (§T1 #1 satisfied nominally); fixtures are single-sourced (`load_test_doc` OnceLock cache, `pinned_two_section_node`, `TestNudgeMutation` builder replacing three drifted local factories); Unicode edges covered (ZWJ/combining/flags in tests_nodes.rs:76-105, family-emoji split in section_structure.rs:577); error paths mirrored against verify messages; cross-crate parity test (tests_nodes.rs:2352-2471) pins the preview/commit contract; determinism hardening against HashMap order is pervasive and documented. Gaps — (a) no test deletes the highest-numbered root with children and undoes (F1); (b) no test asserts a custom mutation's effect survives `rebuild_all` (F2 — grow-font/Toggle rebuild-survival); (c) every animation-completion test passes `tree = None`, so the production completion path is untested (F3); (d) the declarative-beats-handler security property is tested for the gate function (console/commands/mutation.rs:634-667) but there is no end-to-end test that a User-tier override's *mutator* actually runs instead of the Rust handler. Redundancy is low; only a handful of accessor tests (tests_selection.rs:194-221 from_ids trivia) edge toward §11 "locking in trivial stuff", and tests_common.rs:180 uses `text.chars().count()` where grapheme counting is the house rule.
Why it matters: §T1/§T7 — the fundamentals with the heaviest consequences are precisely the untested seams.
Fix: add the four scenario tests named above; switch tests_common.rs:180 to `count_grapheme_clusters`.
Effort: M

---

## Quest-by-quest summaries (evidence for questions that didn't each yield a standalone finding)

**Q1 Setter matrix** — covered by F5/F10/F12. Full channel×property map: color {node bg/border/text: nodes/mod.rs:477,489,513; section text (+range): section_text.rs:292,426; edge body: style.rs:166; edge label: style.rs:78; portal icon: portal.rs:60; portal text: portal.rs:177}; font size/family {node: nodes/mod.rs:563,627; section (+range): section_text.rs:331,370,443,464; edge body: style.rs:191,210,224,259,331; edge label: style.rs:370; portal text: style.rs:471}; zoom bounds {node: nodes/mod.rs:687; edge/edge-label/portal-endpoint: zoom_bounds.rs:34,65,102 — this file is the exemplar: all three route through `mutate_edge` + `OptionEdit` with zero copy-paste}; text {node: nodes/mod.rs:397; section: section_text.rs:87,165,228,585; edge label: label.rs:15; portal text: portal.rs:144}. The zoom file proves the unification pattern works; the font-triple trio and the node-side envelopes are the two surfaces that never adopted it.

**Q2 Undo completeness** — every public mutating path either pushes undo internally, returns undo data consumed by a `*_with_undo` wrapper (`apply_move_*`→MoveNodes at drag release, `apply_reparent`/`apply_orphan_selection`→ReparentNodes, `create_*_edge`→CreateEdge at the dispatch site, `remove_edge`→DeleteEdge via `apply_delete_selection`), or is documented-transient (previews). Inverse-semantics audit: DeleteNode restores edges at original indices AND children (✓, except F1); ReparentNodes restores parent_id + whole edges vec (✓ order included); EditNodeStyle/EditNodeText snapshot every field their producers can change including grow-pass side effects and selection (✓ — the undo_action.rs field docs narrate the historical holes that were closed); EditEdge is a full-edge snapshot (✓); CreateEdge clears a dangling matching selection (✓). Un-covered mutating writes: animation per-tick lerp (by design, completion covers it — but see F3/F22) and the F11 no-op pushes (inverse problem: entries with nothing to undo).

**Q3 Hit-testing SSOT** — four subsystems, each owning a distinct element class: nodes/sections via BVH `tree.descendant_at` + `NodeShape::contains_local` (document/hit_test.rs:34-190); edge bodies via `build_connection_path` + `distance_to_path` (hit_test.rs:199-240); edge/node resize handles + edge grab-handles via live-geometry scans (hit_test.rs:511-645, edges/structural.rs:37-67); edge labels/portal icons/portal texts via renderer-cached AABB maps (renderer/hit.rs:61-147); overlay UI via baumhard `Scene::component_at` (scene_host.rs:427). No duplicated *logic* across classes — shape math stays in baumhard (`contains_local`, `intersects_local_aabb`), path math in `connection`. Two small overlaps: the closed-interval AABB predicate exists three times (renderer/hit.rs:17-19 `aabb_contains`, document/hit_test.rs:178-189 inline, baumhard shape.rs) — a candidate for one `geometry::aabb_contains`; and priority order is gesture-dependent (F17).

**Q5 topology** — `fresh_child_id` max+1 semantics correct for creation (no gap reuse); the delete-first ordering breaks it (F1). Reparent cycle-prevention verified correct: `is_ancestor_or_self(source, target)` (baumhard model/mod.rs:167-179) returns true when source==target or source is an ancestor of target → skip, with tests (tests_reparent.rs:166-208). `dedup_subtree_roots` correct and tested. `cascade_rename` complexity + overwrite hardening: F18. No duplication with baumhard's `derive_parent_id`/`id_sort_key` found — topology.rs operates on prefix strings deliberately (rename must rewrite descendants wholesale, which `derive_parent_id` doesn't express).

**Q6 clipboard** — clipboard.rs itself is minimal and correct: thread-local SECTION_BUFFER + byte-equal probe + clear-before-copy (stale-payload and broadcast-paste hazards are both explicitly defended, lifecycle.rs:360 + 428-436). Channel routing is NOT duplicated: `selection_targets` (console/traits/view.rs:645-694) is the single fan-out and `TargetView` trait impls own per-channel copy/cut/paste. WASM stub is log::debug + None (CONCEPTS says "warn-and-noop" — doc drift folded into F21).

**Q9 mutations_loader** — precedence (App<User<Map<Inline) implemented as insertion order in `build_mutation_registry_with_app_and_user` (animations.rs:78-104), matching the enum order and format doc; provenance stamped for `mutation help`. The declarative-beats-handler security rule is implemented exactly as documented: `will_dispatch_to_handler` requires both handler presence AND `MutationSource::App` (custom/mod.rs:104-107), with gate-level tests. Platform loaders share path/query/size-cap plumbing through user_config (native 121 lines vs web 50, no meaningful duplication); the 1 MiB payload cap and best-effort warn-and-skip posture are uniform. Clean except the F15 dead variant.

**Q10 SelectionState** — helpers are consistent and well-tested (tests_selection.rs pins narrow-vs-wide accessor semantics). Match-site audit across the crate: every `_ =>` catch-all I inspected is deliberate (edge-adjacent exclusions in `live_selection_node_ids`, accessor `_ => None` arms); the newer variants (Section/MultiSection/SectionRange) are explicitly listed at all consequential sites (click.rs:256-264, topology.rs:174-204, scene_rebuild.rs:42, predicates.rs:41-64, selection/mod.rs:91). The one real inconsistency is F4's unit fork, plus `cross_dispatch/selection/mod.rs:206` documenting that sibling-walk deliberately drops SectionRange's range (consistent with F4's "demote" direction).

**Q11 test quality** — F23.

---

## Checked and CLEAN

- `mutate_edge` rollback-on-false contract (fork rollback, pre-fork None in undo) — implemented and directly tested (edges/structural.rs tests).
- Inverted-min/max font clamps rejected before mutation at all three channels (panic-avoidance for `f32::clamp`), each with tests.
- Zoom-bounds setters: single OptionEdit-based shape across node/edge/label/portal-endpoint; validation mirrors `ZoomVisibility::try_new`; no duplication.
- Reparent cycle/self rejection semantics (arg order of `is_ancestor_or_self` verified against baumhard impl).
- DeleteNode edge-restoration ordering (pre-removal indices; bystander order preserved) — correct and pinned by a dedicated regression test.
- Four-source mutation precedence + handler-override security property (implementation and gate tests).
- BorderPreview discipline: never serialized / never undo / never dirty verified in setters and pinned by tests; drift is subset-based (not equality) with documented rationale; commit/preview parity pinned cross-crate.
- `validate_section_aabb` / node-size validators mirror `maptool verify` messages; both width and height branches tested.
- Transient previews (`label_edit_preview`, `portal_text_edit_preview`, `color_picker_preview`, `border_preview`) — all read-only at scene build, cleared on commit/cancel, no model writes (documented contract honored).
- `apply_tree_highlights` goes through the mutator/walker vocabulary (mutation-first §3), replacing an older direct-arena trio — exemplary.
- user_config XDG fallback order + env-race-safe tests; web query-param/localStorage loaders; payload cap shared and logged uniformly.
- `flower_layout` — model-write posture is the documented handler contract; undo via Children-scope CustomMutation snapshot covers exactly the moved set; NaN-hardened; production-dispatch round-trip tested.
- Edge identity by `(from,to,type)` linear scans — documented decision, applied consistently (`EdgeRef::matches` everywhere, no shadow indices).
- No custom error types anywhere in scope (String errors only), per §9.
- `hit_test_target` single-walk climb (container + section identity in one ancestor pass) — the documented perf fix is real.
- `compute_click_hit` short-circuits lower-priority scans when a higher-priority hit exists (no wasted linear scans per click).
