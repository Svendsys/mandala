# Migrating Legacy Maps

Earlier development iterations used a miMind-derived format with:

- Opaque numeric IDs (heap pointers like `"348068464"`)
- Integer enum codes (`"shape_type": 0`, `"anchor_from": 3`)
- Per-node color schemas duplicating the palette on every descendant
- `index: i32` for sibling ordering
- No `channel` field

Mandala no longer reads that format. A one-shot migration tool converts
legacy files to the current format.

## Also: migrating portals into edges

An earlier revision stored portals in a separate top-level
`portals[]` array. That parallel hierarchy has been folded into the
`edges[]` array — portals are now edges with
`display_mode = "portal"`. The current loader refuses to read a
file that still carries a non-empty `portals[]`; migrate with:

```
maptool convert --portals <input.json> <output.json>
```

Input and output may be the same path (the read completes before
the write begins). Each legacy `PortalPair` becomes a `MindEdge`
with `edge_type = "cross_link"`, `display_mode = "portal"`, and the
original glyph / color / font carried into `glyph_connection`.

## The legacy-format command

```
maptool convert --legacy <input.json> <output.json>
```

Reads `<input.json>` as a legacy-format file and writes `<output.json>` in
the current format. The input is never modified.

## What it does

1. **Assigns Dewey-decimal IDs** by walking the tree (using `parent_id` +
   the old `index` field for sibling order). Rewrites every reference —
   edge `from_id`/`to_id` (covers both line-mode and portal-mode edges;
   post-refactor portals live in the edges array) and the HashMap keys.
2. **Converts integer enums to named strings** for `shape_type` →
   `shape`, `layout.type`, `layout.direction`, `line_style`,
   `anchor_from`, `anchor_to`. Unknown integer values fall back to
   sensible defaults (documented in each enum's value list —
   see [enums.md](./enums.md)).
3. **Hoists color schemas to top-level palettes**. Each unique palette is
   defined once; per-node `color_schema` becomes a lightweight reference.
   The `theme_id` and `variant` fields are dropped; `variant` != 2 gets
   folded into the palette name (`"coral"` + `variant: 3` becomes
   `"coral-v3"`).
4. **Removes `index`** from each node (sibling order derives from the new
   Dewey ID).
5. **Adds `channel: 0`** to each node (the default).

## Known limitations

- **Orphaned nodes** (nodes whose `parent_id` references a non-existent
  node) keep their original ID — they can't be placed in the Dewey tree
  without a parent. The output is internally consistent but has mixed ID
  styles. Fix the input or edit the output.
- **Unknown enum values** fall silently to defaults. If you had a custom
  shape code that meant something specific, it becomes `"rectangle"`.
- **Palette collisions** (two level-0 nodes with the same palette name +
  variant but different `groups`): first-writer-wins. Rare in practice
  because miMind produces consistent palettes across nodes in the same
  theme.

## After conversion

Run `maptool verify <output.json>` to confirm the converted file is
well-formed. It should exit 0 with no violations. If it doesn't,
the input had structural problems the converter couldn't resolve (cycles,
orphaned nodes, etc.).

## TextRun ranges: code points → grapheme clusters

`TextRun.start` and `TextRun.end` were originally measured in
**Unicode code points** (Rust's `char` count). The current spec
measures them in **grapheme clusters** — what users see as one
character — to match `ColorFontRegions::Range`, the cosmic-text
bridges in `baumhard::font::attrs`, and the rest of baumhard's
text primitives (see `lib/baumhard/CONVENTIONS.md §B1` and the
[`Range`](../CONCEPTS.md#range) entry).

ASCII / single-codepoint BMP text is unaffected — for that text
`chars().count()` and grapheme-cluster count agree, so existing
ranges remain valid. The migration matters only for text that
contains:

- combining marks (Hebrew niqqud, Arabic shadda, Devanagari matra,
  combining accents over Latin letters)
- ZWJ-joined emoji families (`👨‍👩‍👧`)
- regional-indicator pairs (flag emoji, `🇸🇪`)

For files with ranges authored against the old code-point spec,
`maptool verify` flags them with `"end N exceeds text length M
(grapheme clusters)"` because the grapheme count is lower than
the codepoint count for combining-mark text. The existing
`maps/testament.mindmap.json` (243 nodes, ~15 with Hebrew niqqud)
was migrated as part of the spec-flip commit. If you maintain a
custom map predating the flip and `maptool verify` reports
text-run violations on a node whose text has the kinds of
characters listed above, the ranges need to be recomputed
against grapheme clusters.

A grapheme-segmentation pass (`unicode-segmentation` in Rust;
`regex.findall(r'\X', text)` in Python; equivalents in most
languages) produces the right mapping: walk `node.text` once,
build a `char_index → grapheme_index` table, then rewrite each
run's `start` and `end` through the table. The spec-flip commit
message in this PR sketches a Python one-liner that uses
`unicodedata.combining` — sufficient for Hebrew niqqud but not
ZWJ emoji.

## Why a separate tool?

Mandala rejects legacy files at load time rather than silently migrating
them. The format drift is too large to patch over with `#[serde(alias)]`
and backward-compat struct fields — that approach bakes the legacy format
into the runtime indefinitely. A dedicated migration tool keeps the
runtime clean: it only ever reads the current format.

The conversion is idempotent-safe for files that already look current
(already-Dewey IDs survive unchanged, already-string enums pass through,
already-hoisted palettes don't double-hoist). But the converter is
intended as a one-shot migration, not an always-on pipeline.
