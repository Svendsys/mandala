# Map formats

This directory documents the on-disk representation of a Mandala map and
the app's other file and wire formats.

## Start here

- [`schema.md`](./schema.md) — the `.mandala.json` format (V1): strict,
  nested, typed JSON, with one table per object. The loader and saver are
  `lib/baumhard/src/document/format/`.
- [`projection.md`](./projection.md) — where a V1 map is drawn: its five
  glyph layers, what is drawn, and the geometry of rectangles, paths,
  labels and portal ends. The projector is
  `lib/baumhard/src/projection/`.

This is the only map format. The concept explainers for it are written
by V1-E4 (#256).

## Concept explainers

- [`fonts.md`](./fonts.md) — the bundled faces, how a map names a
  family, weight within a family, and ornament fallback
- [`edits.md`](./edits.md) — edits: named, saved, undoable changes a
  map, the user or the app defines, applied by the walker, optionally
  animated (V1-D1, #250)
- [`presentations.md`](./presentations.md) — presentations: named
  performances a map, the user or the app defines, played on nodes and
  never saved — a lift, a fade, a reveal, a word that swells — on the
  document or on the glyph runs (V1-D2, #251)
- [`verbs.md`](./verbs.md) — verbs: a map's public interface, named
  sequences of steps that start and stop its presentations, apply its
  edits and wait, invoked by a node's click and hover, a span's link,
  the console, IPC and macros; and links that open an address (V1-D3,
  #252)
- [`mutations.md`](./mutations.md) — the glyph mutation vocabulary
  (`GfxMutation`, the `MutatorNode` AST) that glyph trees are edited
  with, and the document's kinds and predicate fields
- [`macros.md`](./macros.md) — `Macro` carrier, the two-tier loader,
  privilege gates, and the threat model

## Wire protocol

- [`ipc.md`](./ipc.md) — the `--ipc` local-socket control protocol
  (framing, envelope, trust model, command families). Not part of
  the on-disk map format; it lives here beside `macros.md` because
  it is a privilege-bearing surface under the same source-of-truth
  discipline.

## Examples

`lib/baumhard/src/document/format/fixtures/` holds a corpus of maps that
load and save back byte for byte, and `maps/` the repository's converted
maps, which do too.
