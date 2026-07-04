# P3-52: In-code doc corruption sweep — merge-damaged doc blocks attached to wrong items, glued `///` fragments, systematic §-number mis-citations, cost-claim lies

**Severity:** P3 (docs; one instance already broke a test) · **Area:** both crates

## Problem — four recurring damage patterns, all sites verified

**A. Doc blocks fused onto the wrong item (rustdoc renders them on the wrong function):**
- `event_mouse_click.rs:1014-1049` — the "Outside-click NodeEdit-exit helper" block sits above `finalize_node_resize_release`; the real fn (line 1143) is undocumented.
- `nodes/mod.rs:720-746` — three unrelated doc blocks (zoom-pair-guard, clamp-runs helper, verify-parity note) all attached to `validate_node_size`; `validate_zoom_pair` (:886) undocumented.
- `section_structure.rs:708-720` — an orphaned doc + stray `#[test]` above a second doc + second `#[test]` on `add_section_rejects_at_cap` (duplicate attribute; trips `duplicate_macro_attributes`).
- `lifecycle.rs:463-483` — `split_paste_for_targets`' paragraph fused onto `is_broadcast_paste`.
- `border.rs:1244-1272` — `resolve_palette_cycle`'s doc spliced mid-sentence into `apply_view_to_slot`'s; the real fn keeps only an orphaned cost line; `default_custom_glyphs` (:1226) has `//` not `///`.
- `fonts.rs:394-395` — stale "Opaque black…" fragment fused onto `InkBounds`' doc.

**B. Glued `///` mid-sentence / redaction artifacts** (plan references stripped mid-sentence): `dispatch/native.rs:1016` ("Inline helper for the empty-canvas orphan-and-edit gesture so" prefixing an unrelated fn); `style.rs:342,381,415,445,467`; `keybinds/action/mod.rs:303,672-678,833-855`; `section/mod.rs:341,348,619,682,700,743-744,945,1046,1058,1292,1354` (the last physically swallowed a `#[test]` — the proof this class isn't cosmetic); `canvas.rs:129,227,321,450-451`; `border/mod.rs:66`; `border/show.rs:21,31`.

**C. Systematic §-number mis-citations after CODE_CONVENTIONS renumbering:** no-panic rules cited as "§4" (`core/primitives.rs:185`, `tree_walker.rs:252`) and "§7" (`primitives.rs:403`, `regions.rs:22`, `console_pass.rs:211`, `dynamic_context.rs:319-321`, `renderer/tests.rs:424`) — the error-handling section is **§9**; seam-preservation cited as "§6" (`tree.rs:111,156`, `line.rs:61`, `area_fields.rs:112`, `model/mutator.rs:35,199`) — seams are **§7**.

**D. Docs that state falsehoods about behavior** (beyond those tracked in other issues): `color_at_region` "O(1) hash map" (BTreeSet, O(log n)); `GlyphArea.background_color` "Mutations can modify this directly through the tree walker" (no field/command variant exists — also note 3 renderable fields sit outside the §B4 mutation surface: background_color, background_padding, align_center — add variants or state rebuild-only); renderer `shape_id` doc describes an `f32::from_bits` encoding the code doesn't use (`as f32` + `u32(round(id))` — the described encoding would render nothing); picker arm docs say "10 glyphs" (arrays are 8); `FpsDisplayMode::Debug` promises a per-stage breakdown (it's a rolling average); tree_walker module doc claims "everything else … kept pub" (everything else is private); `completions` "populated lazily on Tab" (recomputed per input change) + first-vs-bottom-row default contradiction; `node_scale.rs` guard test claims the builder "walks iteratively" (it's literally `build_children_recursive`); `layout_portal_text`'s "diagonal normal" bug attribution (function only returns cardinals); `resolve_border_style` "field-by-field cascade" (it's slot-level — `color: None` does NOT inherit the canvas default's color; either fix wording or implement field-level as a deliberate change); metric_cache size/latency claims ("~12 bytes", "~100ns" — ignore the heap key); `metrics` module "Deprecated in favour of metric_cache" with five live call sites and no `#[deprecated]` (§10: delete rather than deprecate — finish the migration or drop the claim).

## Fix plan

One mechanical sweep per pattern (A: re-home blocks; B: repair/delete sentences; C: fix § numbers; D: correct each claim), split into 2-3 commits by crate. Behavior-affecting decisions flagged inline (background_color mutability, field-level cascade, metrics deprecation) get one-line decisions in the PR description.

## Acceptance criteria

- `cargo doc -p baumhard --no-deps` + `cargo doc -p mandala --no-deps` spot-check: every listed item renders the right doc on the right fn.
- Grep for "§4"/"§6"/"§7" citations in the listed files returns only correct references.
- The duplicate `#[test]` warning is gone.

## Pointers

CODE_CONVENTIONS §8; CONVENTIONS §B9 ("a doc comment that lies is worse than no doc comment"); per-agent findings files for any additional context.
