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
| `channel` | integer\|null | `null` (falls through to section's index) | Mutation channel inside the parent node-area. `null` lets the tree builder substitute the section's index — a three-section node with no authored channels gets channels `[0, 1, 2]`. `Some(0)` is honoured even at idx > 0, so an author can deliberately collide with a sibling mind-node on channel 0 to broadcast. |
| `trigger_bindings` | array | `[]` | Per-section [`TriggerBinding`s](./mutations.md). The click dispatcher fires section-level bindings *before* the whole-node bindings on `MindNode.trigger_bindings` — a section-targeted override (e.g. a different `OnClick` mutation per stratum of a multi-section node) takes precedence over catch-all node bindings. |

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

## Custom mutations

Mutations authored as `CustomMutation` reach a node's section
text + runs at apply time. The flat-apply path
(`apply_custom_mutation` in
`src/application/document/custom.rs`) iterates every
`Flag::SectionRoot` child of each affected node container and
applies the mutation list to each — same primitive as
`apply_tree_highlights` walks. Mutations that target chrome
fields (`area.scale`, `area.position`) also land on the
container, so position-affecting mutations move the whole node
in lockstep with its sections.

Section content sync from the tree back to the model on
persistent mutations is wired: `sync_node_from_tree` walks every
section, writes back `text` / `text_runs` / `offset` / `size`
through the merge-with-prior reverse converter
(`region_to_text_run`). A **selective gate** keyed by per-region
`(range, color, font)` skips untouched sections so a
`SectionsOnly` text mutation doesn't silently strip
`bold` / `italic` / `underline` / `size_pt` / `hyperlink` from
sections the mutation didn't touch. Range-mutating mutations
(`ChangeRegionRange`, `SetRegionFont`/`SetRegionColor` over a
new range) inherit prior styling via dominant-overlap fallback
when no exact-range prior matches.

Documented round-trip limit: `var(--name)` colour references
collapse to their resolved hex on the round trip — the tree-side
`FloatRgba` carries no record of the variable. Authors who edit
section colours through custom mutations and then save the model
will see the variable replaced with a hex literal. The
`set_section_text` / `set_section_text_color` /
`set_section_font_size` / `set_section_font_family` document
setters bypass the round trip and preserve `var(--name)`
references verbatim.

### Console axis applicability on a section selection

Sections only have a **text** colour axis (`color text=…`). The
`bg=` and `border=` axes are node-level chrome and have no
section-level field — running them against a `SelectionState::Section`
returns `Outcome::NotApplicable` rather than collapsing to the
owning node's `background_color` / `frame_color`. This applies
both to the kv form (`color bg=#fff section=K`) and to the
trait-dispatch form (`color bg=#fff` with the section already
selected). Same routing behind the colour-picker modal: opening
the picker on `color bg` / `color border` against a section
selection surfaces the NotApplicable message instead of opening
on the owning node.

## Channel space

Sections live in the same Baumhard tree as child mind-nodes. The
section channels and the child mind-node channels share one
sibling-channel space inside the container area. A custom
mutation that targets "channel 0 children" therefore hits both
the first section and any child mind-node tagged channel 0.

The seam closing this is the combination of
`TargetScope::SectionsOnly` and the
`GfxElementField::Flag(Flag::SectionRoot)` predicate variant —
both shipped. `SectionsOnly` walks every section directly via
`MindMapTree::section_arena_id` (bypassing the channel-keyed
sibling scan), and the predicate gate filters by element flag
within any other scope. See [mutations.md](./mutations.md) for
both authoring shapes.

The `MindSection.channel` field is `Option<usize>` (post Tier-E):
`None` falls through to the section's index, `Some(0)` is
honoured even at idx > 0. Pre-`Option`, the bare `usize`
silently overrode an author's explicit `channel: 0` on sections
beyond the first.

## Validation

`maptool verify` enforces:

- Per-section text-run invariants — non-overlapping, ascending,
  `end <= grapheme cluster count of section.text`. Same rules
  as the pre-section text_runs surface, just keyed by section.
  (Implemented in `crates/maptool/src/verify/text_runs.rs`.)
- Section offset / size shape: `offset.{x,y}` finite + non-
  negative; `size.{width,height}` (when set) finite + strictly
  positive; `offset + size <= node.size` (AABB containment).
- Node-level size sanity: `node.size.{width,height}` finite +
  strictly positive — a NaN at this level poisons every
  downstream AABB / hit-test / shaping computation.
- Astronomical section sizes: section `size > 100 × node.size`
  flags as a likely typo (e.g. accidental extra zero).
- Section channel collisions inside one node — surfaced because
  broadcasting one mutation across two sections sharing a
  channel is *occasionally* the intent but more often a typo.

The empty-`sections[]` invariant is enforced by the loader (not
verify): a zero-section map is rejected at parse time with a
`maptool convert --sections` migration pointer.

## See also

- [`schema.md`](./schema.md#mindsection) — the per-field type table.
- [`text-runs.md`](./text-runs.md) — per-grapheme styling, now
  anchored on a section instead of on the node.
- [`CONCEPTS.md` §3 "Sections"](../CONCEPTS.md) — conceptual
  treatment, including the named-trajectory seams.

