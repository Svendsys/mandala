# Borders: a creative toolkit

Mandala borders are not a "pick from five presets" feature.
They're a typographic surface authors can author freely: any
glyph, any pattern, any color, any palette, any font. The
machine-readable schema lives below; the goal of this section is
to set expectations.

If you want a border made of the letter `A`, you can do that.
If you want `-##---AAAA---##-` repeating around a node, you can
do that. If you want one node's left side to read "TODO ⇒"
forever and the right side to be palette-cycled emoji, you can
do that. The presets (`light`, `heavy`, `double`, `rounded`)
exist as conveniences — they're starting points authors can
walk away from any time.

The same machinery powers **section frames** (the cyan rectangles
around per-section subdivisions while NodeEdit is active) — see
[Section frames](#section-frames) below — so the customization
applies to both surfaces with one shared vocabulary.

Animation of border content (a scrolling marquee, a per-tick
glyph swap) is on the roadmap but not implemented. See
[`./animation-roadmap.md`](./animation-roadmap.md) for the gap
analysis and what's blocking it.

## Examples

Each example shows the resolved string at one specific rendered
width — narrower sides emit fewer fill iterations, wider sides
emit more. The auto-resize pass (§5) grows the node so the static
parts (prefix + suffix + corners) always fit.

```
top="+=##=+"             →  +=##=++=##=++=##=+   (width 18, 3 atomic copies)
top="###(*)###"          →  ###******###         (width 12, 6 fill iters)
top="-##---(AAAA)---##-" →  -##---AAAAAAAA---##- (width 20, 2 fill iters)
top="─" tl="◆" tr="◇"    →  ◆────────────◇       (width 14, custom corners)
top="+=#(\(\))#=+"       →  +=#()()()#=+         (width 12, escaped parens in fill)
```

Per-side strings live under `GlyphBorderConfig.glyphs` as
`top`, `bottom`, `left`, `right`. Per-corner glyphs live under
`top_left`, `top_right`, `bottom_left`, `bottom_right`. The console
verbs auto-promote `preset` to `"custom"` whenever a side or corner
glyph is set; hand-edited JSON should set `preset: "custom"`
explicitly so the resolver reads the `glyphs` payload.

## Reference: Border-side patterns

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

## Section frames

Section frames — the cyan rectangles drawn around each section
of a multi-section node while NodeEdit is active — flow through
the same machinery node borders do. Authors can write
`MindSection.frame_border: GlyphBorderConfig` on a per-section
basis; the configuration accepts every key the node-border
config does (`preset`, `font`, `font_size_pt`, `color`,
`color_palette`, `color_palette_field`, `glyphs.{top, bottom,
left, right, top_left, top_right, bottom_left, bottom_right}`,
`padding`).

Map-wide defaults live on the canvas:

- `Canvas.default_section_frame_border` — the unfocused frame
  shape (sections of the active NodeEdit node that are *not*
  currently inside the inline text editor).
- `Canvas.default_focused_section_frame_border` — the focused
  frame shape (the section whose text is being edited).

Resolver cascade (per-section, on each scene rebuild):
1. `MindSection.frame_border` if `Some` — per-section author
   override wins outright.
2. Otherwise the canvas default for the section's focus state:
   - **Focused section** (currently inside the inline text editor):
     `Canvas.default_focused_section_frame_border` if `Some`, else
     `Canvas.default_section_frame_border` if `Some` (focused
     frames inherit unfocused defaults so authors who only set the
     unfocused variant get one consistent shape).
   - **Unfocused section**: `Canvas.default_section_frame_border`
     if `Some`.
3. Otherwise a hardcoded floor: `light` preset for unfocused
   frames, `heavy` preset for the focused one. The floor is just
   another `GlyphBorderConfig` flowing through the same resolver —
   there are no inline glyph constants in the section-frame path.

The resolver call site is
`lib/baumhard/src/mindmap/border.rs::resolve_section_frame_border`.

### Console verbs (section frames)

Per-section frame style is authored through the `section frame …`
subverb of the [`section`
command](../src/application/console/commands/section/frame.rs):

```
section frame show               # readout of the resolved config (cascade source labelled)
section frame reset              # drop the per-section override
section frame preset=heavy color=#ff8800
section frame top="###(*)###" tl="◆" tr="◆" bl="◆" br="◆"
section frame palette=rainbow field=frame
section frame preset=heavy section=2     # explicit section index when selection is a single node
```

Map-wide defaults are authored through the `canvas` command:

```
canvas border preset=heavy color=#ff00cc        # Canvas.default_border
canvas section-frame preset=double              # Canvas.default_section_frame_border
canvas section-frame focused preset=heavy       # Canvas.default_focused_section_frame_border
canvas section-frame show
canvas border reset
```

The `section frame` and `canvas` verbs share the per-node `border`
verb's kv vocabulary verbatim — `preset`, `font`, `size`, `color`,
`palette`, `field`, `padding`, `top`, `bottom`, `left`, `right`,
`tl`, `tr`, `bl`, `br`, `show`, `reset` — so muscle memory carries
across surfaces.

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

`light` is the default preset. Its corner glyphs (`┌┐└┘`) extend
to the cell edges, so corners and sides connect cleanly in any
monospace face. `rounded` (`╭╮╰╯`) curves inward and leaves a
small visible gap at every corner — pick it deliberately if that's
the look you want.

Setting any side or corner glyph automatically promotes the
preset to `"custom"`. Quoted patterns survive the tokenizer
unchanged so `(` / `)` / spaces don't need shell escaping.

### Live preview

Every border verb has a `preview` sub-mode that stages edits
without writing the model. The preview renders on the targeted
node / section / canvas slot until the user terminates with
`commit` (writes through the matching committing setter) or
`cancel` (discards). Auto-promotion notes ride alongside the
preview's success message so the user sees the same outcome
they'll get on commit.

```
border preview preset=heavy color=#ff8800
border preview commit                       # write to MindNode.style.border
border preview cancel                       # discard

section frame preview top="###(*)###"
section frame preview commit                # write to MindSection.frame_border
section frame preview cancel

canvas border preview palette=rainbow
canvas border preview commit                # write to Canvas.default_border
canvas border preview cancel

canvas section-frame preview preset=double
canvas section-frame focused preview preset=heavy
canvas section-frame focused preview commit # write to Canvas.default_focused_section_frame_border
```

Selection drift cancels: setting a preview on node A then
selecting node B causes the preview to stop rendering (the
slot is cleared at the next `set_*` / `commit_*` / `cancel_*`
call). Implicit cancel: a non-preview committing edit
(`border preset=double` after `border preview preset=heavy`)
clears the preview before applying its own write — the
committing edit always wins.

Programmatic surface: `Action::SetBorderPreview { target_kind:
BorderPreviewTargetKind, field, value }`, `Action::CommitBorderPreview`,
`Action::CancelBorderPreview`. `BorderPreviewTargetKind` is a
typed enum (`node` | `section` | `canvas-border` | `canvas-sf` |
`canvas-sf-focused`) registered in `KeybindConfig` as
`set_border_preview` parametric bindings. Default `Esc` already
cancels an active preview through `Action::ExitMode`'s body —
the chain lives there because the keybind resolver maps
`(context, key) → Action` deterministically and can't fall
through. `cancel_border_preview` and `commit_border_preview`
ship unbound by default; users opt in for muscle-memory variants
(e.g. `Ctrl+Enter` for commit) via the JSON config.
