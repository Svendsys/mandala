# Sections

A **section** is a positioned text-bearing surface inside a
`MindNode`. Every renderable node carries at least one section;
a node's "user data strata" — the actual text the user types
into a node — lives on its sections rather than on the node
itself.

## Where they sit

```
MindMap
└── nodes[id]: MindNode
    ├── style, position, size, layout, channel, …  (node chrome)
    └── sections: [MindSection, MindSection, …]     ← user-data strata
        ├── text
        ├── text_runs
        ├── offset (relative to node.position)
        ├── size (None = fill the node)
        └── channel
```

In the runtime [Baumhard tree](../CONCEPTS.md#tree-t-m), a `MindNode`
materialises as a three-deep subtree:

- one **container** `GlyphArea` (chrome only — background, frame
  padding, shape, zoom window),
- one **section-area** `GlyphArea` per section, carrying the
  section's text and its theme-resolved `ColorFontRegions`,
- one structural **section-model** `GlyphModel` child per
  section-area, present as a future per-component / per-grapheme
  mutation seam (the renderer skips it today).

The renderer's tree walker shapes each section-area into its own
`cosmic_text::Buffer`, keyed by `unique_id`. No special-case in
the renderer — sections are first-class `GlyphArea` elements; the
multiplicity is the only thing the renderer notices.

## Field reference

| Field | Type | Default | Notes |
|---|---|---|---|
| `text` | string | required | The section's plain text. May contain `\n`. |
| `text_runs` | array | `[]` | Per-grapheme run table — see [text-runs.md](./text-runs.md). Empty means "render in the section/node defaults". Non-empty means "only the covered ranges render", same coverage trap as the pre-section single-runs vector. |
| `offset.x`, `offset.y` | number | `0.0` | Top-left of the section's AABB *relative to the owning node's `position`*, in canvas units. `(0, 0)` puts the section flush against the node's top-left. |
| `size` | object\|null | `null` | Section AABB. `null` means "fill the parent node" — the typical migration-default shape, where every node has one section that occupies its whole AABB. An explicit `{width, height}` lets a section occupy only part of the parent node, leaving room for siblings. |
| `channel` | integer | section's index | Mutation channel inside the parent node-area. The default value `0` is replaced at tree-build time by the section's index when index > 0, so a three-section node with no authored channels gets channels `[0, 1, 2]` automatically. |

## Migration

Pre-section maps put `text` and `text_runs` directly on each
`MindNode`. The post-section data shape moves them into the
node's first section (and only section, in the default
migration). Per [`CODE_CONVENTIONS.md` §10](../CODE_CONVENTIONS.md)
"no dual shapes", the loader rejects pre-section files at parse
time with a concrete migration pointer:

```
legacy `text` / `text_runs` on node "0"; run
`maptool convert --sections <file>` to migrate node text into `sections[]`
```

`maptool convert --sections <in.json> <out.json>` walks every
node, lifts its legacy `text` + `text_runs` into a single default
`MindSection`, and writes the result back. The migration is
idempotent: re-running on an already-migrated map is a no-op.

The legacy `convert --legacy` pipeline (miMind import) folds the
section pass in automatically, so a single `convert --legacy`
hop produces a post-section file in one step.

## Channel space

Sections live in the same Baumhard tree as child mind-nodes. The
section channels and the child mind-node channels share one
sibling-channel space inside the container area. A custom
mutation that targets "channel 0 children" therefore hits both
the first section and any child mind-node tagged channel 0.

This is a known authoring caveat today, accepted in exchange for
a simpler tree shape. The named seam that closes it is a
predicate variant `Predicate::IsSection` plus a target-scope
variant `TargetScope::SectionsOnly` / `ChildrenOnly` — neither
field needs adjustment when those land.

## Validation

`maptool verify` enforces:

- Every node ships at least one section (zero-section maps are
  rejected by the typed loader; this rule guards against
  hand-edits that end up with empty `sections[]`).
- Per-section text-run invariants — non-overlapping, ascending,
  `end <= grapheme cluster count of section.text`. Same rules
  as the pre-section text_runs surface, just keyed by section.

Invariants the verifier deliberately doesn't enforce yet:

- Section AABB containment: an `offset + size` that pokes
  outside the parent node's AABB is allowed and renders as a
  section that overflows its parent. A future check could warn.
- Section `channel` collisions inside one node — broadcasting
  one mutation across two sections sharing a channel is
  occasionally the intent.

## See also

- [`schema.md`](./schema.md#mindsection) — the per-field type table.
- [`text-runs.md`](./text-runs.md) — per-grapheme styling, now
  anchored on a section instead of on the node.
- [`CONCEPTS.md` §3 "Sections"](../CONCEPTS.md) — conceptual
  treatment, including the named-trajectory seams.

