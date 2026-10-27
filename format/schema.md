# The `.mandala.json` format

A map is one JSON text with the extension `.mandala.json`. It is
**strict, nested and typed** (design D4, §7): the structure of the
file is the structure of the document, a node's children are nested
inside it, every key is defined here, and a file is either valid or it
does not load. The reference implementation is
`lib/baumhard/src/document/format/` (`load`, `load_from_bytes`,
`save`); the fixture corpus in
`lib/baumhard/src/document/format/fixtures/` is a set of maps that save
back byte for byte.

The app does not read this format yet; V1-C3 switches it over. The
older `.mindmap.json` is described in
[`legacy-schema.md`](./legacy-schema.md) until that sub-issue deletes
it.

## How a file is read

- **Unknown keys are an error.** Every object below denies keys it does
  not list. Nothing is preserved and nothing is skipped.
- **A value of the wrong type is an error.** `null` is not a way to
  write an absent key: leave the key out. An object cannot be written
  as an array of its values, and a name cannot be written as an object.
- **Only the spellings listed here are accepted.** There are no
  shorthands (`#fff`, a bare `"rectangle"`, `"24pt"`). Where this page
  lists more than one spelling of a value (a weight as `"bold"` or
  `700`, a start point by name or by side and fraction, a color with or
  without alpha digits), they read to the same value and the saver
  writes the canonical one.
- **A reference that names nothing is an error:** a link's `to`, a
  verb, a palette, a theme variable, a font.
- **Names become indices when the file is read and never again.** A
  `var(accent)` is a variable of the theme, resolved by name. The
  document holds indices; the file never does, because an index is
  assigned by sorted name and would name another entry once a name was
  added. The order of the keys in the file never matters: variables,
  variants and palettes get their index from the ascending byte order
  of their names.
- **The first error ends the load,** and nothing else is returned (no
  partial document). Every error names the JSON path of the value
  (`nodes[3].frame.rings[0].spans[2].color`); see
  [Errors](#errors).
- **Every number is checked** against the bounds in
  [Bounds](#bounds) as it is read. A number outside its bound is an
  error, never clamped, saturated or aliased.
- **A row is read in key order.** Sections, rings and sibling nodes are
  ordered by `channel`, then by their place in the file. A file that
  lists them in another order loads (the loader sorts each row, stably,
  before it builds it), and saves in the sorted order.

## How a file is written

- **Deterministic.** Keys are written in the order the tables below
  list them. Name-keyed objects (theme variants and variables,
  palettes) are written in ascending byte order of the name. Nodes are
  written in sibling order. Numbers are written in serde_json's
  shortest round-trip form of the stored `f32` (`24.0`, `0.06`),
  indentation is two spaces, and the text ends with a newline. An
  unchanged map saves byte-identically, and a one-field edit is a
  one-line diff.
- **Omitted:** every optional key that is unset (inheriting); `channel`
  `0`, `folded` `false`, `hidden` `false`, `opacity` `1.0`; a section's
  `offset` `[0, 0]`; a path's `anchors` `["auto", "auto"]` and `visible`
  `true`; empty arrays; a child node's connector with every field
  inherited and nothing under it; a portal link's `ends` when both ends
  are all-inherited with nothing under them; and `defaults`, and any
  object inside it, that sets nothing.
- **Never written:** anything derived: fitted sizes, depth, resolved
  colors, parent pointers.
- **Nothing is dropped, and nothing is written that does not load.** A
  document that holds a value the file cannot spell is not saved: `save`
  returns a `SaveError` naming the path the value would have had, in the
  notation of [Errors](#errors), and writes nothing. `check_savable`
  gives the same answer without writing, for the mutations and the
  editor to call when they apply an edit. What it refuses:
  - a size in the wrong unit for where it sits (points on a frame or a
    ring, a fraction elsewhere), and a span `size` inside a ring;
  - the fallback face as an explicit `font`;
  - a reference to a theme variable, palette or verb the document does
    not define (so a `var()` with nothing behind it is never written);
  - two theme variants, two theme variables or two palettes of one name,
    and two edits, presentations or verbs of one `id`;
  - a theme with variables or an active index but no variants, an active
    index that is not a variant, and a variant with a different number
    of values than the theme has variables;
  - a link that leads to something that is not a node, or to its own
    node, and a portal link without exactly two ends;
  - everything the loader would refuse in the file it would write: a
    number outside its [bound](#bounds), a `channel` past `MAX_CHANNEL`,
    and so on. The saver builds the file it wrote with the loader's own
    builder, so the two cannot disagree about what loads.
- **The empty theme has one spelling: no `theme` key.** A document with
  no variables and no variants saves without one, and a file without one
  loads as that theme. A `theme` that is present has at least one
  variant.
- **One exception to "a saved map loads back as the same document":** a
  portal link keeps the fields of a path within a session, so that a link
  can switch between line and portal without losing them, and the file
  does not save them (the path keys belong to line links). A portal link
  reloads with the default path.

## Objects

Each table lists the object's keys in the order the saver writes them.
*Required* keys must be present. An optional key that is absent is
unset, and inherits from the owners, the map's `defaults` and the
built-in value, unless the table gives a default of its own.

### Top level

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `format` | integer | yes | | The format number: `1`. Any other number is an error, reported before anything else in the file. |
| `name` | string | yes | | The map's name. |
| `theme` | [theme](#theme) | no | the empty theme | The theme. Absent is the empty theme (no variables, no variants), which has no other spelling. A theme that is present has at least one variant. |
| `palettes` | object: name to array of [palette groups](#palette-group) | no | none | The palettes. |
| `defaults` | [defaults](#defaults) | no | nothing set | Per-field defaults, consulted after the element and its owners and before the built-in value. |
| `edits` | array of [definitions](#definitions) | no | `[]` | The edits the map defines. V1-D1 extends this schema. |
| `presentations` | array of [definitions](#definitions) | no | `[]` | The presentations the map defines. V1-D2 extends this schema. |
| `verbs` | array of [definitions](#definitions) | no | `[]` | The verbs the map defines. V1-D3 extends this schema. |
| `nodes` | array of [nodes](#node) | no | `[]` | The top-level nodes, in sibling order. |

### Theme

The theme stores every variant and the *name* of the active one (D21);
the live values are the active variant's, never a second copy.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `active` | string | yes | | The name of the active variant. It must be one of `variants`; a theme with no variants has nothing to name, so it is an error. |
| `variants` | object: variant name to object: variable name to [literal color](#colors) | yes | | The variants. Every variant defines exactly the same variable names; an error names the variant and the variable that differs. A variable's value is `#rrggbb` or `#rrggbbaa`: not `var(...)`, not `palette(...)`. |

A variable's index is its position in ascending byte order of its name,
the same in every variant. A variant's index is likewise its position in
ascending byte order of its name. Neither depends on the order of the
keys in the file.

### Palette group

A palette is an array of groups, one per depth below the node bound to
it (the nearest ancestor-or-self with `palette` set); a depth past the
last group takes the last. A palette's index is its position in
ascending byte order of its name.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `fill` | [color](#colors) | yes | | A node's fill. |
| `frame` | [color](#colors) | yes | | A node's frame text. |
| `text` | [color](#colors) | yes | | Section text. |
| `title` | [color](#colors) | no | the group's `text` | The text of a node's first section (its lowest channel). With no `title`, the first section takes `text`. |

A palette's colors are `#rrggbb`, `#rrggbbaa` or `var(name)`. A
`palette(...)` inside `palettes` is an error: a palette cannot refer to a
palette.

### Definitions

An entry of `edits`, `presentations` or `verbs`. Ids are unique within
their list. A verb is named by its `id` from a node's `on` and from a
span's `link`; the same string in two different lists is two things.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `id` | string | yes | | The id the file names it by. |
| `name` | string | yes | | The name shown to the user. |

### Defaults

The map's defaults. Each key is an object with the keys of the element
it is the default for, all optional; an absent key sets nothing. Editing
one field of an element never copies a default into it.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `node` | [node defaults](#node-defaults) | no | nothing set | Node defaults. |
| `section` | [text style](#text-style) | no | nothing set | Section text defaults (points). |
| `frame` | [frame defaults](#frame-defaults) | no | nothing set | Frame defaults. |
| `ring` | [ring defaults](#ring-defaults) | no | nothing set | Ring defaults. |
| `connector` | [path defaults](#path-defaults) | no | nothing set | Connector defaults. |
| `link` | [path defaults](#path-defaults) | no | nothing set | Line-link defaults. |
| `label` | [label defaults](#label-defaults) | no | nothing set | Label defaults. |
| `portal_end` | [portal-end defaults](#portal-end-defaults) | no | nothing set | Portal-end defaults. |
| `span` | [text style](#text-style) | no | nothing set | Span defaults (points). |

### Node defaults

`defaults.node`.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `shape` | [shape](#shape) | no | unset | The outline. |
| `fill` | [color](#colors) | no | unset | The background fill. |
| `fill_fade` | number `>= 0` | no | unset | The width of the fill's soft edge. |
| `font` | string | no | unset | See [text style](#text-style). |
| `size` | number | no | unset | Points. |
| `color` | [color](#colors) | no | unset | The text color. |
| `weight` | [weight](#weight) | no | unset | The weight. |
| `italic` | boolean | no | unset | Whether the text is italic. |

### Text style

The five text-style keys, on everything that has text.
`defaults.section` and `defaults.span` are objects of just these.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `font` | string | no | unset | A font family name. It must be a face compiled into this build (see [Fonts](#fonts)). |
| `size` | number | no | unset | Points, `0.5` to `4096`; or, inside a frame, a [fraction](#size-units). |
| `color` | [color](#colors) | no | unset | The text color. |
| `weight` | [weight](#weight) | no | unset | The weight. |
| `italic` | boolean | no | unset | Whether the text is italic. |

### Frame defaults

`defaults.frame`: the keys of a [frame](#frame) except `rings`. The
text-style `size` is a [fraction](#size-units).

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `visible` | boolean | no | unset | Whether a frame is drawn. |
| `start` | [start point](#start-point) | no | unset | Where the text starts. |
| `direction` | `"clockwise"` or `"counterclockwise"` | no | unset | Which way the text runs. |
| `fit` | [fit](#fit) | no | unset | |
| `gap` | number `>= 0` | no | unset | |
| `inset` | number `>= 0` | no | unset | |
| `font` | string | no | unset | |
| `size` | number, `(0, 1]` | no | unset | A fraction of the node's shorter side. |
| `color` | [color](#colors) | no | unset | |
| `weight` | [weight](#weight) | no | unset | |
| `italic` | boolean | no | unset | |

### Ring defaults

`defaults.ring`. The text-style `size` is a [fraction](#size-units).

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `fit` | [fit](#fit) | no | unset | |
| `font` | string | no | unset | |
| `size` | number, `(0, 1]` | no | unset | A fraction of the node's shorter side. |
| `color` | [color](#colors) | no | unset | |
| `weight` | [weight](#weight) | no | unset | |
| `italic` | boolean | no | unset | |

### Path defaults

`defaults.connector` and `defaults.link`.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `fit` | [fit](#fit) | no | unset | |
| `spacing` | number `>= 0` | no | unset | Extra space between glyphs. |
| `size` | number | no | unset | Points. |
| `color` | [color](#colors) | no | unset | |
| `content` | string | no | unset | The text drawn when the path has no spans. |

### Label defaults

`defaults.label`.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `t` | number, `0` to `1` | no | unset | |
| `offset` | number | no | unset | |
| `font` | string | no | unset | |
| `size` | number | no | unset | Points. |
| `color` | [color](#colors) | no | unset | |
| `weight` | [weight](#weight) | no | unset | |
| `italic` | boolean | no | unset | |

### Portal-end defaults

`defaults.portal_end`.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `border_t` | number, `0` to `1` | no | unset | |
| `offset` | number | no | unset | |
| `color` | [color](#colors) | no | unset | |
| `size` | number | no | unset | Points. |
| `content` | string | no | unset | |

### Node

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `id` | integer, `1` to `4294967294` | yes | | The id links and verbs name the node by, unique in the map. `0` is reserved. Ids do not encode structure; moving a node never renames anything. |
| `channel` | integer `0..=4294967295` | no | `0` | The node's place among its sibling nodes. |
| `position` | `[x, y]` | yes | | The top-left corner, canvas units. |
| `min_size` | `[w, h]` | no | none | The size the user set, a *minimum*: the node grows to fit its content (D5). |
| `shape` | [shape](#shape) | no | unset | The outline its fill and frame share. |
| `fill` | [color](#colors) | no | unset | The background fill. |
| `fill_fade` | number `>= 0` | no | unset | The width of the fill's soft edge. |
| `palette` | string | no | none | The name of the palette that colors the node and its descendants by depth. |
| `zoom` | [zoom](#zoom) | no | none | The zoom range the node renders in. |
| `folded` | boolean | no | `false` | Whether its descendants are collapsed. |
| `hidden` | boolean | no | `false` | Whether the node, its subtree, its connector and the links touching them are left out. |
| `opacity` | number, `0` to `1` | no | `1.0` | Multiplies the alpha of everything the node and its subtree draw. |
| `on` | [triggers](#triggers) | no | none | The verbs its click and hover run. |
| `font` | string | no | unset | See [text style](#text-style). |
| `size` | number | no | unset | Points. |
| `color` | [color](#colors) | no | unset | |
| `weight` | [weight](#weight) | no | unset | |
| `italic` | boolean | no | unset | |
| `sections` | array of [sections](#section) | no | `[]` | |
| `frame` | [frame](#frame) | no | none | The node's outline text. A node with no `frame` draws none; a `frame` of `{}` draws one with every field unset. |
| `connector` | [connector](#connector) | no | see meaning | The line to the node's parent. **Forbidden on a top-level node.** On a child node it is optional: absent means a connector with every field unset and nothing under it, and the saver omits it in exactly that case. |
| `links` | array of [links](#link) | no | `[]` | |
| `children` | array of [nodes](#node) | no | `[]` | Child nodes, in sibling order. |

### Triggers

`on`.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `click` | string | no | none | The id of a verb in `verbs`. |
| `hover` | string | no | none | The id of a verb in `verbs`. |

### Zoom

`zoom`, on a node, connector or link. Either bound may be open.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `min` | number `>= 0` | no | open | The lowest camera zoom it renders at. |
| `max` | number `>= 0` | no | open | The highest camera zoom it renders at. Not below `min`. |

### Section

A box of spans inside a node. A box breaks lines at `\n` only (D26).

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `channel` | integer `0..=4294967295` | no | `0` | The place among the node's sections. |
| `offset` | `[x, y]` | no | `[0, 0]` | The top-left corner, relative to the node's. |
| `min_size` | `[w, h]` | no | none | The size the user set, a minimum. |
| `align` | `"left"`, `"center"` or `"right"` | no | unset | How the lines align in the box. |
| `font` | string | no | unset | See [text style](#text-style). |
| `size` | number | no | unset | Points. |
| `color` | [color](#colors) | no | unset | |
| `weight` | [weight](#weight) | no | unset | |
| `italic` | boolean | no | unset | |
| `spans` | array of [spans](#span) | no | `[]` | |

### Span

Text with one style.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `text` | string | yes | | The text. |
| `font` | string | no | unset | See [text style](#text-style). |
| `size` | number | no | unset | Points. **Not allowed on a span inside a ring** (the ring sizes its text). |
| `color` | [color](#colors) | no | unset | |
| `weight` | [weight](#weight) | no | unset | |
| `italic` | boolean | no | unset | |
| `link` | [link target](#link-target) | no | none | Where it leads when clicked. |

### Link target

Exactly one of the two keys.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `url` | string | one of the two | | A web address. |
| `verb` | string | one of the two | | The id of a verb in `verbs`. |

### Frame

A node's outline text, in rings. Inside a frame, `size` is a
[fraction](#size-units) of the node's shorter side, never points. Every
key is unset when absent; the built-in values that then apply are
`visible` `true` (D25: shown unless it says otherwise), `start`
`"top-left"`, `direction` `"clockwise"`, `fit` `"fit"`, `gap` `0` and
`inset` `0`.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `visible` | boolean | no | unset | Whether the frame is drawn. |
| `start` | [start point](#start-point) | no | unset | Where the text starts on the outline. |
| `direction` | `"clockwise"` or `"counterclockwise"` | no | unset | Which way the text runs. |
| `fit` | [fit](#fit) | no | unset | What the rings do when text and outline differ in length. |
| `gap` | number `>= 0` | no | unset | The space between rings, canvas units. |
| `inset` | number `>= 0` | no | unset | The first ring's distance from the outline, canvas units. |
| `font` | string | no | unset | |
| `size` | number, `(0, 1]` | no | unset | A fraction of the node's shorter side. |
| `color` | [color](#colors) | no | unset | |
| `weight` | [weight](#weight) | no | unset | |
| `italic` | boolean | no | unset | |
| `rings` | array of [rings](#ring) | no | `[]` | One stanza per ring, innermost first, ordered by `channel`. |

### Ring

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `channel` | integer `0..=4294967295` | no | `0` | The place among the frame's rings. |
| `fit` | [fit](#fit) | no | the frame's | Overrides the frame's fit. |
| `font` | string | no | unset | |
| `size` | number, `(0, 1]` | no | unset | The ring's size: a fraction of the node's shorter side, the one `size` a ring has. |
| `color` | [color](#colors) | no | unset | |
| `weight` | [weight](#weight) | no | unset | |
| `italic` | boolean | no | unset | |
| `spans` | array of [spans](#span) | no | `[]` | |

### Connector

The line to a node's parent, and the shape of a line link's path keys.
A connector is always a line (D22): a parent-child relation shown as a
portal is a connector with `visible: false` plus a portal link.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `fit` | [fit](#fit) | no | unset | |
| `spacing` | number `>= 0` | no | unset | Extra space between glyphs. |
| `size` | number | no | unset | Points. |
| `color` | [color](#colors) | no | unset | |
| `content` | string | no | unset | The text drawn when the path has no spans. |
| `anchors` | `[from, to]`, each `"auto"`, `"top"`, `"right"`, `"bottom"` or `"left"` | no | `["auto", "auto"]` | Where it attaches at its start and at its end. |
| `control_points` | array of `[x, y]` | no | `[]` | Bézier control points: at most 2. 0 points is a line, 1 a quadratic and 2 a cubic Bézier. |
| `visible` | boolean | no | `true` | Whether it is drawn. |
| `zoom` | [zoom](#zoom) | no | none | |
| `spans` | array of [spans](#span) | no | `[]` | The line's content (`"─"`, `"·"`, a line of poetry). |
| `cap_start` | [cap](#cap) | no | none | The glyphs at its start. |
| `cap_end` | [cap](#cap) | no | none | The glyphs at its end. |
| `label` | [label](#label) | no | none | Text placed along it. |

### Link

A cross-link or portal from a node to another node. `to` must name
another existing node: a link to the node that holds it is an error. It
may name a node that comes later in the file. The path keys belong to a
line link only; naming one on a portal link is an error, and so is
`ends` on a line link.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `to` | integer | yes | | The `id` of the node it leads to. |
| `kind` | `"line"` or `"portal"` | yes | | Whether it draws a path or two portal ends. |
| `fit` | [fit](#fit) | no | unset | Line only. The path keys, as for a [connector](#connector). |
| `spacing` | number `>= 0` | no | unset | Line only. |
| `size` | number | no | unset | Line only. Points. |
| `color` | [color](#colors) | no | unset | Line only. |
| `content` | string | no | unset | Line only. |
| `anchors` | `[from, to]` | no | `["auto", "auto"]` | Line only. |
| `control_points` | array of `[x, y]` | no | `[]` | Line only. At most 2. |
| `visible` | boolean | no | `true` | Line only. |
| `zoom` | [zoom](#zoom) | no | none | Line only. |
| `spans` | array of [spans](#span) | no | `[]` | Line only. |
| `cap_start` | [cap](#cap) | no | none | Line only. |
| `cap_end` | [cap](#cap) | no | none | Line only. |
| `label` | [label](#label) | no | none | Line only. |
| `ends` | array of exactly two [portal ends](#portal-end) | no | two ends with every field unset | Portal only. The source end (beside the link's node), then the target end. |

### Cap

The glyphs at a path's start or end: text-style keys and spans.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `font` | string | no | unset | |
| `size` | number | no | unset | Points. |
| `color` | [color](#colors) | no | unset | |
| `weight` | [weight](#weight) | no | unset | |
| `italic` | boolean | no | unset | |
| `spans` | array of [spans](#span) | no | `[]` | |

### Label

Text along a path, or beside a portal end.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `t` | number, `0` to `1` | no | unset | The position along the path. |
| `offset` | number | no | unset | The distance from the path along its normal. |
| `font` | string | no | unset | |
| `size` | number | no | unset | Points. |
| `color` | [color](#colors) | no | unset | |
| `weight` | [weight](#weight) | no | unset | |
| `italic` | boolean | no | unset | |
| `spans` | array of [spans](#span) | no | `[]` | |

### Portal end

One end of a portal link.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `border_t` | number, `0` to `1` | no | unset (faces the other end) | The position around the node's outline. |
| `offset` | number | no | unset | The distance from the outline. |
| `color` | [color](#colors) | no | unset | The marker color. |
| `size` | number | no | unset | The marker size, points. |
| `content` | string | no | unset | The marker drawn when the end has no spans. |
| `spans` | array of [spans](#span) | no | `[]` | The marker, as text. |
| `label` | [label](#label) | no | none | |

## Values

### Colors

| Spelling | Meaning |
|---|---|
| `"#rrggbb"`, `"#rrggbbaa"` | A literal color. Hex digits are lowercase, and an opaque color is written without alpha digits: `#ABCDEF` and `#abcdefff` are refused, with the canonical spelling in the message. |
| `"var(name)"` | The theme variable `name`, in the active variant. The name must be a variable of the theme. |
| `"palette(fill)"`, `"palette(frame)"`, `"palette(text)"`, `"palette(title)"` | That color of the palette group the element's owning node is bound to, at the node's depth. Not allowed inside `palettes`. |

### Weight

`"normal"` (400), `"bold"` (700), or a number from 100 to 900 in steps of
100. `400` and `700` read as `"normal"` and `"bold"` and are written that
way.

### Shape

`"ellipse"`, or `{ "rectangle": { "radius": 25 } }`. The radius is an
integer percentage of half the node's shorter side: `0` is sharp, `100`
fully round (D11). A bare `"rectangle"` is not a spelling: the radius is
required.

### Fit

What a text does when it and its path differ in length (D10, §6.4).

| Spelling | Meaning |
|---|---|
| `"repeat"` | The text repeated until the path is full. |
| `"spread"` | The text once, spaced out to end where the path ends. |
| `"once"` | The text once; the rest of the path gets nothing. |
| `{ "once": { "filler": "·" } }` | The text once, then the filler repeated. |
| `"fit"` | The text scaled to fill the path exactly once, within the built-in bounds: `0.5` to `2.0` times the declared size. |
| `{ "fit": { "min": 0.5, "max": 2.0 } }` | The same, with bounds of your own: `0 < min <= max`. The built-in bounds are written as `"fit"`. |

### Start point

Where a frame's text starts on the outline. Edges run clockwise: top
runs left to right, right runs top to bottom, bottom runs right to left,
left runs bottom to top.

| Name | Side | Fraction |
|---|---|---|
| `"top-left"` | top | 0 |
| `"top"` | top | 0.5 |
| `"top-right"` | right | 0 |
| `"right"` | right | 0.5 |
| `"bottom-right"` | bottom | 0 |
| `"bottom"` | bottom | 0.5 |
| `"bottom-left"` | left | 0 |
| `"left"` | left | 0.5 |

Any other point is `{ "edge": "top", "at": 0.25 }`: a side and a
fraction from `0` to `1` along it. The saver writes the name whenever
one matches exactly.

The keys of the objects that spell the values above:

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `radius` | integer, `0` to `100` | yes | | In `{ "rectangle": ... }`. |
| `filler` | string | yes | | In `{ "once": ... }`. |
| `min` | number | yes | | In `{ "fit": ... }`. |
| `max` | number | yes | | In `{ "fit": ... }`. |
| `edge` | `"top"`, `"right"`, `"bottom"` or `"left"` | yes | | In a start point. |
| `at` | number, `0` to `1` | yes | | In a start point. |

### Size units

`size` means points everywhere except inside a **frame**, where it is a
fraction of the node's shorter side in `(0, 1]`: the frame, its rings,
`defaults.frame` and `defaults.ring`. There is no way to write points
there: `24` on a ring is a fraction past all of the node, and an error. A
span's `size` inside a ring is refused, because a ring's text is sized
by the ring.

### Fonts

`font` is a font family name that resolves to a face compiled into this
build, for example `"Public Pixel"`. A family that is not compiled in is
an error, including one that happens to be installed as a system font on
this machine: such a family resolves to the fallback face, which has no
name a file can write back, and the same map would mean something else
on another machine. A face with several family names loads under any of
them and is written under the first.

### Numbers

Any JSON number is accepted where a number is expected (`24` and
`24.0` read the same) and every number must be finite. The saver writes
serde_json's shortest form of the stored `f32`, so a number reads back
to the same bits.

## Bounds

All are in `document::bounds`; an out-of-range value is an error naming
its path.

| Value | Bound |
|---|---|
| Canvas coordinates: `position`, `offset`, control points, a label's or portal end's `offset`, either part of a `min_size` | magnitude at most 1e9 (a `min_size` also `>= 0`) |
| Font size in points | `0.5` to `4096` |
| Font size as a fraction (inside a frame) | above `0`, at most `1` |
| `opacity`, a label's `t`, `border_t`, a start point's `at` | `0` to `1` |
| `fill_fade`, `gap`, `inset`, `spacing`, a `zoom` bound | `>= 0` |
| `fit` bounds | `0 < min <= max` |
| Corner radius | `0` to `100` |
| Weight | 100, 200, ... 900 |
| `control_points` | at most 2 (`MAX_CONTROL_POINTS`) |
| Node `id` | `1` to `4294967294` (`MAX_NODE_IDENT`) |
| `channel` (node, section, ring) | `0` to `4294967295` (`MAX_CHANNEL`), the most a `usize` holds on wasm32, so every target accepts the same files |
| `palettes`, theme variables, theme variants, `verbs`, `edits`, `presentations` | at most 65536 each, the most an index addresses |

## Errors

A load returns a `LoadError` with a `path` and a `kind`.

- The `path` writes keys after a dot and array positions in brackets:
  `nodes[3].frame.rings[0].spans[2].color`. A key that is not a plain
  identifier (a letter or `_`, then letters, digits and `_`), which only
  a name an author chose can be, is quoted in brackets as JSON quotes a
  string, so that it cannot be read as more path: `palettes["a.b"][0].fill`,
  `theme.variants[""].bg`, `nodes[0][""]`. A `SaveError`'s path is written
  the same way. It is empty for an error
  about the whole text (too large; not UTF-8), for the map itself (a
  missing top-level key) and when the reader died where it has no path
  to give (the end of the text).
- An unknown key's path ends in the key. A missing key's path is the
  object that lacks it, and the key is in the `kind`.
- Which error is "first" follows the order the loader reads the file in:
  the text, then the map's keys in schema order, then each node depth
  first. Links are resolved last, because a link may name a node that
  comes after it.
- A file whose `format` is not `1` is reported as that, whatever else is
  wrong with it.

## Limits

- **The size of the text.** Unbounded on native. On wasm32 the text
  is refused above 3 GiB (`MAX_MAP_BYTES`), before it is parsed.
- **Nesting.** The JSON reader follows 128 levels of array and object.
  A node nested in a node costs two (the node and its `children`), so a
  chain of about 60 nested nodes is the deepest a file can hold; deeper
  is an error, never a crash.

## Predicates

Values inside a predicate saved in a map (V1-D1) are spelled as they are
everywhere else in this file: colors as above; palettes, variables,
fonts and verbs by name, never by index. The strictness gate refuses any
type in this format that holds an index.

## Example

```json
{
  "format": 1,
  "name": "Testament",
  "theme": {
    "active": "dark",
    "variants": {
      "dark": { "accent": "#d4a24c", "bg": "#101014" }
    }
  },
  "nodes": [
    {
      "id": 1,
      "position": [0.0, 0.0],
      "shape": { "rectangle": { "radius": 25 } },
      "sections": [
        { "spans": [ { "text": "In the beginning", "size": 24.0, "weight": "bold" } ] }
      ],
      "frame": {
        "fit": "fit",
        "rings": [ { "spans": [ { "text": "And the earth was without form, and void; " } ] } ]
      },
      "links": [ { "to": 2, "kind": "portal" } ],
      "children": [
        { "id": 2, "position": [320.0, 40.0], "connector": { "spans": [ { "text": "·" } ] } }
      ]
    }
  ]
}
```

A saved file puts each array element on its own line; this one is
condensed to read.
