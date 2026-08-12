# Palettes

Color schemas are defined **once** at the map level and **referenced** by
name from each themed node.

```json
{
  "palettes": {
    "coral": {
      "groups": [
        { "background": "#a9decb", "frame": "#30b082", "text": "#000000", "title": "#000000" },
        { "background": "#f3b1c4", "frame": "#e24271", "text": "#000000", "title": "#000000" }
      ]
    }
  },
  "nodes": {
    "0": {
      "color_schema": {
        "palette": "coral",
        "level": 0,
        "starts_at_root": true,
        "connections_colored": true
      }
    }
  }
}
```

## Why hoist palettes to the map level?

The legacy format stored the full palette definition — every color group,
for every depth level — **on every themed node**. In the testament map,
243 of 243 nodes carried a `color_schema` object; 225 of those duplicated
the palette name, variant code, theme_id, and an empty `groups` array;
only 18 (the schema roots) actually contained the palette data.

Every child node copied fields that only made sense on the root, because
the legacy renderer walked up the parent chain to find the definition.
The file was twenty-three megabytes of redundancy.

Hoisting palettes solves this:

- Each palette is defined once, in one place
- Editing a palette is a single-point change; every node using it
  updates on the next render
- Per-node `color_schema` becomes tiny: a palette name, a depth level,
  two flags
- User-defined palettes and palette-switching mutations become natural
  (palette is a named thing you can reference, not a blob buried in a
  node's data)

## How a node resolves its colors

`MindMap::resolve_theme_colors(node)` in
`lib/baumhard/src/mindmap/model/theme.rs`:

1. Read `node.color_schema` — if absent, the node uses the colors in its
   `style` (background_color, frame_color, text_color). A channel the
   schema names in `overrides` short-circuits every step below and is
   used verbatim, even when the palette lookup would fail
2. Look up `schema.palette` in `map.palettes`
3. Pick the group index: `schema.level` when `starts_at_root` is true,
   `schema.level - 1` when it is false (see below)
4. Index into `palette.groups`. If the index exceeds the group count,
   clamp to the last group.

If the palette name doesn't exist in the map — or exists with an empty
`groups` array, or the `starts_at_root` shift leaves no index at all —
`resolve_theme_colors` returns `None` and the renderer falls back to the
node's plain `style` colors. `maptool verify` flags missing palette
references and empty palettes as errors.

The resolved group is what the projection passes read, through four
sibling helpers on `MindMap` that each name one role, each check this
node's own override first, and each fall back to the same `style`
field:

| Role | Override | Palette channel | Fallback | Read by |
|---|---|---|---|---|
| Node fill | `overrides.background` | `group.background` | `style.background_color` | `node_background_color` |
| Node frame | `overrides.frame` | `group.frame` | `style.frame_color` | `node_frame_color` |
| Node text | `overrides.text` | `group.text` | `style.text_color` | `node_text_color` |
| Node title | `overrides.title` | `group.title` | the text role above | `node_title_color` |

## Overriding one channel on one node

```json
"color_schema": {
  "palette": "coral",
  "level": 2,
  "starts_at_root": true,
  "connections_colored": true,
  "overrides": { "background": "#00ff00" }
}
```

`overrides` is where a per-node color edit lands — `color bg=#00ff00`
on a themed node, the glyph-wheel picker, `set_node_bg_color`. One
optional string per `ColorGroup` channel; absent channels take the
group's. The whole key is omitted from the JSON when nothing is
overridden, which is every node until somebody recolors one.

**Why not `style`?** Because `style` is the tier the palette
*shadows*. Every node the legacy converter produced carries baked
`style` colors that are stale copies of its own theme, so a reader
that let `style` win would un-theme the entire corpus, and a writer
that wrote `style` would report success and change nothing on
screen. The override is a third tier precisely so `style` can go on
meaning "what this node would be if it had no theme" while a direct
edit still beats an inherited one — the same
inline-style-beats-inherited rule the sub-node channels below
already follow.

The three interactive setters cover `background`, `frame` and
`text`, the channels `NodeStyle` also has. `title` is honored on
load and has no setter, because `node_title_color` falls through to
the text role and overriding *that* already moves the title.

Undo restores the whole `color_schema`, so undoing a per-node
recolor puts the node back on its palette rather than on the stale
`style` value.

An **unthemed** node has no `color_schema` and therefore no
override tier; its setters write `style` directly, which is the
only tier it has.

Everything below the node level is *more* specific and wins over the
theme:

- A **text run** naming its own `color` keeps it. The theme reaches text
  through runs that leave `color` empty and through sections that carry
  no runs at all. This is the inline-style-beats-inherited rule: a
  per-word color the author picked must survive a retheme.
- A **`border.color`** override on the node (or on the canvas default
  border) keeps painting the border glyphs; the theme supplies only the
  cascade base that override sits on.

The **title** role applies to the first hard-newline-delimited line of
the node's *first* section, and only when that section has no text runs.
A palette that leaves `title` empty, or sets it equal to `text`, leaves
the section undivided.

## What `level` means

Depth from the schema root, as a non-negative integer. The root of a
themed subtree has `level: 0` and indexes into `groups[0]`. Its children
have `level: 1` (groups[1]), grandchildren `level: 2`, and so on. A
palette with 7 groups themes 7 levels of hierarchy; the eighth and every
level below it **clamp to the last group** rather than cycling back to
`groups[0]`, so a subtree deeper than its palette degrades to one color
instead of repeating the root's.

> The miMind corpus this format was migrated from cycled instead: in
> `maps/testament.mindmap.json`, the `sandy` palette has 7 groups and
> nodes at levels 7–15 carry baked frame colors matching
> `groups[level % 7]`. Mandala clamps. The two rules agree for every
> level inside the palette and differ only past its end.

`level` is stored explicitly rather than computed from parent chain depth
because subtrees may be themed independently — a deep subtree can restart
at level 0 with a different palette.

## The `starts_at_root` and `connections_colored` flags

Inherited from miMind.

**`starts_at_root`** answers "is the schema root itself themed?"

- `true` — level 0 *is* the root. Group index equals `level`.
- `false` — the root is transparent: it keeps its own `style` colors and
  resolves to no group at all. Its children, at `level: 1`, take
  `groups[0]`. Group index is `level - 1`.

**`connections_colored`** controls whether edges inherit the palette
stroke. When the flag is set on a node's schema, every edge *leaving*
that node (its `from_id`) takes the resolved group's `frame` as its
stroke color, ahead of the edge's own `color`. The source node governs,
not the target: an edge is drawn in its parent's branch color. A cross
link follows the same rule, so the direction it was authored in is the
direction it takes its color from.

The edge color cascade, highest priority first, is therefore:

1. `edge.glyph_connection.color`, or `canvas.default_connection.color`
   when the edge has not forked its own connection config — an explicitly
   named stroke beats the theme
2. the source node's palette `frame`, when `connections_colored` is set
   and the schema resolves
3. `edge.color`

The label and portal-marker channels hang off the same cascade: each
takes its own override when it has one, and otherwise follows the edge
body, theme tier included.

## What's no longer in the format

The legacy per-node color schema carried three fields that the new format
drops:

- `groups`: now lives on the palette, not the node
- `theme_id` (e.g. `"Pastel:#BFFFFFFE01"`): an opaque miMind-internal
  identifier; Mandala never read it
- `variant`: an integer variant code; if a map had multiple variants of
  the same palette name, the converter folds the variant into the palette
  name (`"coral"` vs `"coral-v3"`)

`maptool convert --legacy` performs the hoisting automatically.
