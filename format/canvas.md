# `MindMap.canvas`

The map-wide defaults that every node and section falls back to
when its own field is unset. Authoring is through the `canvas …`
console verb; the data model lives at
`baumhard::mindmap::model::Canvas`.

## Field reference

Every field of `Canvas` is listed here, plus `palettes`, which is
map-wide but lives beside `canvas` rather than inside it. The
struct is the authority; [schema.md](./schema.md#canvas) carries
the same table in brief.

| Field | Type | Default | Notes |
|---|---|---|---|
| `background_color` | string | **required** | Whole-canvas fill, `#RRGGBB` or `var(--name)`, resolved against `theme_variables` at render time. The one `Canvas` key with no serde default: a `canvas` object without it fails the typed load. No console verb sets it; author it in the file. |
| `default_connection` | object\|null | `null` | The map-wide `GlyphConnectionConfig` every edge falls back to when it carries no `glyph_connection` of its own. Same shape as an edge's `glyph_connection`. |
| `theme_variables` | object | `{}` | The **live** CSS-variable-style color table. Every `var(--name)` reference in node / edge / section styles resolves through it. No console verb edits it today; author it in the file, or switch it wholesale from `theme_variants` (next row). |
| `theme_variants` | object | `{}` | Named presets, each a whole variable map. Inert until activated: a `CustomMutation` carrying the `SetThemeVariant` document action copies the named preset into `theme_variables` (see [mutations.md](./mutations.md)). Editing a variant changes nothing on screen until then. |
| `palettes` | object | `{}` | Named color palettes, serving two consumers: a node's `color_schema` resolves its fill / frame / text / title through one (see [palettes.md](./palettes.md)), and every `color_palette = "…"` reference (on a node / section / border) cycles one per glyph. No console verb defines or edits a palette today; `border palette …` only *references* one by name. Note this key lives on the **map**, not inside `canvas` — it is listed here because it is map-wide shared state. |
| `default_border` | object\|null | `null` | The map-wide border every framed node falls back to. Same shape as `MindNode.style.border`. Authoring via `canvas border …`. |
| `default_section_frame_border` | object\|null | `null` | The map-wide default for the cyan rectangle drawn around an unfocused section in NodeEdit mode. Same shape as `MindNode.style.border`. Authoring via `canvas section-frame …`. |
| `default_focused_section_frame_border` | object\|null | `null` | The map-wide default for the focused section's frame (the section currently being text-edited). Same shape; falls back to `default_section_frame_border` then to a hardcoded heavy floor. Authoring via `canvas section-frame focused …`. |

## Cascade resolution

A border or section frame resolves by **slot**, not field by field:
the most specific config that is present is chosen whole.

```
per-node / per-section override   ← authored via `border …` / `section frame …`
  ↓ (if absent)
canvas default                    ← authored via `canvas border …` / `canvas section-frame …`
  ↓ (if absent)
hardcoded floor                   ← `light` preset for borders, `heavy` for focused section frames
```

So a node with a `border` object of its own draws that object alone:
a field it leaves unset (`font`, `glyphs`, `color_palette`) takes the
hardcoded default, not the canvas default's value. `preset`,
`font_size_pt` and `padding` always carry a value, so the resolver
could not tell an unset one from a chosen default even if it wanted
to. The one field-level exception is a node border's `color`: the
node's own `border.color`, then its theme's frame color, then the
canvas default's `color`, then `style.frame_color`. The first console
edit to a node's border creates its `border` object from the
hardcoded defaults rather than from the canvas default (issue #223).

The same cascade discipline applies to `theme_variables` /
`palettes` lookups: a `var(--accent)` reference fails through to a
hardcoded warn-and-fall-back if the canvas table doesn't define
the name.

## Console verbs

See [`border-patterns.md`](./border-patterns.md)'s "Console verb"
section for the full `canvas border …` and
`canvas section-frame [focused] …` grammar (positional + kv
forms, preview lifecycle, cycle / toggle subverbs).

`canvas border preset cycle` and the per-section
`section frame preset=…` parity work landed in Batch 6 / Batch 8;
canvas-side `show side=` filter and `verbose` flag are open
follow-ups (the canvas show formatter has a different shape
without per-node `size` to draw a side-rendered preview from).

## Schema migration note

The three `default_*` border fields landed in the
section-borders-resize PR (Batches 2 / 5 / 6). All default to
`null`, so legacy `.mindmap.json` files load unchanged (serde
defaults absorb absent fields). No schema-version bump.

The `MindSection.frame_border` per-section override landed in the
same PR — see [`sections.md`](./sections.md#field-reference) for
its row in the section field table.
