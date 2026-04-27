# Border-side patterns

The four side fields under
[`GlyphBorderConfig.glyphs`](./schema.md#glyphborderconfig)
(`top`, `bottom`, `left`, `right`) are parsed as **side patterns**:
small strings that describe how to fill a border side between its
two corners. Two shapes:

## 1. Atomic-repeat (no fill region)

The whole pattern is one cluster sequence repeated as many whole
times as fits. No partial copy.

```
"+=##=+"
```

At width 18 → `+=##=++=##=++=##=+` (3 copies).
At width 8 → `+=##=+` (1 copy; 2 columns blank — the second copy
wouldn't fit).

## 2. Prefix + Fill + Suffix

Exactly one fill region delimited by unescaped `(` and `)`. The
prefix and suffix are placed once at the ends; the fill is repeated
atomically as many whole times as fits between them. A single fill
iteration is also atomic — never split.

```
"###(*)###"
```

`###` (prefix) + `*` × N + `###` (suffix). At width 12 →
`###******###` (6 copies of `*`). At width 6 → `######` (0 copies;
just the statics).

A multi-cluster fill works the same way:

```
"+=#(\(\))#=+"
```

`+=#` (prefix) + `()` × N + `#=+` (suffix). At width 12 →
`+=#()()()#=+` (3 copies of `()`).

## Escapes

Three escape sequences are recognised everywhere:

| Sequence | Meaning |
|---|---|
| `\(` | literal `(` |
| `\)` | literal `)` |
| `\\` | literal `\` |

Any other backslash is a parse error. Trailing backslashes likewise
error rather than silently slicing the input.

## Grapheme awareness

After parsing, each section's string is split into grapheme
clusters via Unicode TR29 (`unicode-segmentation`). Cluster counts,
not codepoint counts, drive the fitter — `é` (a single codepoint)
counts as one cluster, and `🇺🇸` (two regional-indicator codepoints
that compose into one flag) also counts as one. Match
[`lib/baumhard/CONVENTIONS.md`](../lib/baumhard/CONVENTIONS.md) §1
for the rationale.

## Auto-resize

Nodes grow at load time and after every console edit so their
width / height accommodate the **static** parts of every side
pattern, plus one full fill iteration when feasible. The grow is
monotonic — node sizes are author intent, the loader and per-edit
setters only enforce a floor (matches the existing
`grow_node_sizes_to_fit_text` posture).

When even one fill iteration doesn't fit, the fitter renders zero
fill iterations and prints just the prefix + suffix (or as many
clusters as fit if the user manually shrunk the node below the
static floor). Static parts are never split.

## Palette cycling

When `GlyphBorderConfig.color_palette` is set, every cluster on
the border picks its colour from the named palette's
`groups[i % len][color_palette_field]`. The four sides chain into
one continuous sweep around the rectangle in
top → right → bottom → left order, so a coloured stripe wraps
naturally across the corners.

`color_palette_field` selects which channel of each `ColorGroup`
is cycled — `"frame"` (default), `"background"`, `"text"`, or
`"title"`. Unknown values warn and fall back to `"frame"`.

## Console verb

Per-node configuration runs through the
[`border` console verb](../src/application/console/commands/border).
Verbs:

```
border on                 # show_frame = true
border off                # show_frame = false
border show               # multi-line readout of the resolved config
border reset              # drop the per-node override
```

Composable kv form (every key is optional; multiple kvs apply
atomically):

```
border preset=<light|heavy|double|rounded|custom>
border font=<family> size=<pt>
border color=<#hex|var(--name)|preset|reset>
border palette=<name|off> [field=<frame|background|text|title>]
border top="<pattern>" bottom="<pattern>" \
       left="<pattern>" right="<pattern>"
border tl="<glyph>" tr="<glyph>" bl="<glyph>" br="<glyph>"
border padding=<px>
```

Setting any side or corner glyph automatically promotes the
preset to `"custom"`. Quoted patterns survive the tokenizer
unchanged so `(` / `)` / spaces don't need shell escaping.
