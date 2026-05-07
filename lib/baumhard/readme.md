# Baumhard

Glyph-animation library powering Mandala's mindmap renderer. Every
visual element — text, borders, connection paths — lives in a tree
of `GfxElement`s carrying `GlyphArea` (positioned text) or
`GlyphModel` (structured glyph matrices), mutated through a
declarative `MutatorTree` rather than rebuilt each frame.

## Features

- Tree-shaped scene with channel-keyed sibling alignment
- Mutator DSL (deltas, commands, instructions, predicates) for
  declarative model edits
- Animation envelopes (timing + easing) layered over mutators
- Cross-platform `.mindmap.json` loader / saver with legacy-shape
  detection
- Cosmic-text font system wrapped behind a thin façade
  (`baumhard::font`)
- Spatial region indexer for hit-testing and rect-intersection
  queries

## Where to read next

| Document                                  | What it covers                                                  |
| ----------------------------------------- | --------------------------------------------------------------- |
| [`CONVENTIONS.md`](CONVENTIONS.md)        | Crate-local rules — mutation-not-rebuild, arena discipline, no-unsafe, perf invariants. **Mandatory before editing anything under `src/`.** |
| [`../../CONCEPTS.md`](../../CONCEPTS.md)  | Workspace-wide conceptual building-blocks (`GlyphArea`, `Channel`, `MutatorTree`, ...) |
| [`../../format/`](../../format/)          | `.mindmap.json` format spec — the on-disk shape of every scene  |

## Module layout

- [`src/mindmap/`](src/mindmap/) — the data model, loader, scene
  builders, and the tree bridge. Most interesting logic lives here.
- [`src/gfx_structs/`](src/gfx_structs/) — `GfxElement` /
  `GlyphArea` / `GlyphModel`, the mutator vocabulary, walker, and
  predicate language.
- [`src/font/`](src/font/) — single-source wrapper around
  `cosmic_text` (no other module imports `cosmic_text` directly).
- [`src/format/`](src/format/) — JSON façade over `serde_json` so
  loaders / savers go through one seam.
- [`src/util/`](src/util/) — `Color`, geometry helpers, primes,
  logging façade.
- [`src/core/`](src/core/) — shared primitives (regions, ranges,
  flags).
