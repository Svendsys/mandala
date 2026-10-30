# The `.mandala.json` format

A map is one JSON text with the extension `.mandala.json`. It is
**strict, typed and flat** (design D4, D30, §7): every key is defined
here, a file is either valid or it does not load, and the nodes of the
map are one flat list in which each node names its parent. A node's
own content (its sections, frame, connector and links) is nested
inside it, at most 9 levels of JSON deep; the tree of nodes is not, so
the depth of a map's tree costs nothing in the file and has no bound
of its own. The reference implementation is
`lib/baumhard/src/document/format/` (`load`, `load_from_bytes`,
`save`, `save_to`, `check_savable`); the fixture corpus in
`lib/baumhard/src/document/format/fixtures/` is a set of maps that save
back byte for byte.

The app does not read this format yet; V1-C3 switches it over. The
older `.mindmap.json` is described in
[`legacy-schema.md`](./legacy-schema.md) until that sub-issue deletes
it.

## Principles

- **One document, one text.** A given map saves to exactly one text,
  byte for byte, and a loader accepts a text if and only if it spells a
  valid map. A map can be spelled in more than one way only at the JSON
  level — whitespace, string escapes, number forms (`24`, `24.0`,
  `2.4e1`) and the order of keys within an object — never in the
  format's own values, except where a value below lists more than one
  spelling.
- **The order of the node list is part of the document.** It is never
  a choice of whoever wrote the file ([The node list](#the-node-list)).
- **Nothing is derived.** No depth, no child list, no fitted size, no
  resolved value, no index.
- **Depth costs nothing.** Neither the format nor the loader bounds the
  depth of the tree, the JSON nesting of a file does not grow with it,
  and every walk over a map is iterative and linear in its size.

## The text

| Rule | Detail |
|---|---|
| Encoding | UTF-8. Bytes that are not UTF-8 are a `Syntax` error naming the byte offset. |
| Byte-order mark | **Refused** (`ByteOrderMark`). Save as UTF-8 without a BOM: RFC 8259 §8.1 forbids a writer to add one, and accepting it would be a second spelling of the start of the file. |
| Line endings | Any JSON whitespace is accepted, CR LF included. The saver writes LF only. |
| Size | Checked against `MAX_MAP_BYTES` before anything else; the elements a map becomes are counted against `MAX_ELEMENTS` as they are built ([Limits](#limits)). |
| One value | The text is one JSON object. Anything after it but whitespace is a `Syntax` error. |
| Duplicate keys | Refused in every object, name-keyed ones included. |
| Key order | **Never significant to the loader.** A file run through `jq -S` loads as the same map. The saver writes the order the tables below list. |
| Strings | Any JSON escape is accepted. A lone surrogate escape is a `Syntax` error. |
| Numbers | Any JSON number form is accepted where a number is expected, and must be finite once read as the field's type. An integer field (`format`, `id`, `parent`, `channel`, `to`, `radius`, a numeric weight) refuses a number written with a fraction or an exponent. |
| Comments | None: JSON has none, and JSONC and JSON5 are not accepted. |

## How a file is read

A load is three linear passes over the text, none of them recursing on
the depth of the tree:

1. **Gates.** The size, then the byte-order mark, then UTF-8.
2. **Index.** One parse reads where each top-level value and each node
   of `nodes` is in the text, building nothing. A JSON syntax error
   anywhere is reported here, before anything is read for meaning, so
   a truncated file says it is truncated. Then `format` is checked,
   wherever it sits in the object, and only then is a top-level key the
   format does not define refused: a file of a newer format may have
   keys this build does not know, and `format` is the one useful thing
   to say about it.
3. **Build.** The top-level values are read in the order of the table
   below, then each node is read on its own from its place in the text
   and appended to the tree, so only one node is being read at a time.
   A node's rows (its sections and links, a frame's rings, the spans of
   a section, a ring, a path, a cap, a label or a portal end) are read
   one item at a time as each is appended, however many a node holds.
   Links are resolved last, because a link may name a node that comes
   later in the list.

What holds for every value:

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
- **A reference that names nothing is an error:** a `parent`, a link's
  `to`, a verb, a palette, a theme variable, a font.
- **Names become indices when the file is read and never again.** A
  `var(accent)` is a variable of the theme, resolved by name. The
  document holds indices; the file never does, because an index is
  assigned by sorted name and would name another entry once a name was
  added. Variables, variants and palettes get their index from the
  ascending byte order of their names, whatever order the file gives
  them in.
- **The first error ends the load,** and nothing else is returned (no
  partial document). Every error names the JSON path of the value
  (`nodes[3].frame.rings[0].spans[2].color`), the node it is in, and
  its line; see [Errors](#errors).
- **Every number is checked** against the bounds in
  [Bounds](#bounds) as it is read. A number outside its bound is an
  error, never clamped, saturated or aliased.
- **A row is listed in the order the map keeps it, or refused.**
  Sibling nodes, top-level nodes, a node's `sections` and a frame's
  `rings` appear in ascending `channel`, and equal channels appear in
  the order written. One whose `channel` is below the one listed before
  it is refused (`SiblingsOutOfOrder`), never sorted: a sorted file
  would be a second spelling of the same map, and the saver would
  rewrite it on the next save. `links` and `spans` have no channel; their
  order is the order written.

## The node list

`nodes` holds every node of the map exactly once, **flat, in
depth-first pre-order**:

- a node with no `parent` key is a **top-level node**, and a map has
  any number of them, none included;
- a node with `"parent": p` hangs from the node whose `id` is `p`;
- there is no `children` key, no depth key and no other key that says
  where a node is;
- a node comes right after its parent if it is the parent's first
  child, else right after the last node of its previous sibling's
  subtree; a top-level node follows the last node of the previous one's.

So every subtree is one contiguous block of the list — cutting the
block and pasting it elsewhere, then changing its first node's
`parent`, moves it — and a node's `parent` is always on the **chain**
of the node listed before it: that node, or one of its ancestors. The
loader checks exactly that, at the node's `parent`:

| Refusal | When |
|---|---|
| `ParentIsSelf` | the node names itself |
| `ParentNotEarlier` | no node before this one has that id: one that never appears, or appears later. A parent comes before its children. |
| `ParentNotOnChain` | the parent comes earlier, but its subtree has already ended. The message names where the parent is and lists the ids a node here could name — the node before it, then its ancestors, nearest first, at most 8 — or none, for a new top-level node. |

Because a parent must already be on the chain when its child is read,
no cycle, orphan or second parent can be written.

**Identity.** `id` is required on every node, an integer in
`1..=4294967294`, unique in the map, and the node's identity for the
life of the map: the saver writes it as the document holds it and
never renumbers. Moving a node changes only its `parent`. Ids say
nothing about structure, and the list is ordered by structure, not by
id. Two nodes with one `id` are refused (`DuplicateId`) at the second,
naming where the first is; the usual cause is a git merge of two
branches that each added a node.

| Who names a node by `id` | Checked at load |
|---|---|
| a node's `parent` | on the chain, as above |
| a link's `to` | names a node anywhere in the list but the link's own; a later node is fine |
| a verb or presentation target | not at load: a missing node is skipped with a warning when it runs (D27) |
| IPC, the console, `mandalactl` | through the document's ident index |

The counter that gives a new node its id is not saved; a loaded map
starts it one above the highest node id in the list **and** above every
node id a verb or a presentation names, so a node deleted while
something still names it never has its id given to a new node after a
reload. (Verbs and presentations carry no targets yet; a node's
`parent` and a link's `to` always name a node in the list.)

## How a file is written

- **Deterministic.** Keys are written in the order the tables below
  list them, `id` and then `parent` first on every node. Name-keyed
  objects (theme variants and variables, palettes) are written in
  ascending byte order of the name. Nodes are written in the tree's
  pre-order, siblings in the order the map keeps them, which is exactly
  the order [the node list](#the-node-list) requires. Numbers are
  written in serde_json's shortest round-trip form of the stored `f32`
  (`24.0`, `0.06`, `1e-7`), integers as integers. An unchanged map saves
  byte-identically.
- **The layout.** The top-level values other than `nodes` are written
  with two-space indentation. Then `"nodes": [` and each node as a
  block: `    {` alone on a line; then one line per key the node has, in
  the order of [the node table](#node), each six spaces and
  `"key": value`, with a `,` ending every key's line but the node's last;
  then `    }`, with a `,` after every node but the last. A key's value
  is in compact form, with `", "` between members and elements, `": "`
  after each key and no space inside a bracket or brace. Then `  ]` and
  `}`. A map with no nodes leaves `nodes` out. LF line endings, exactly
  one trailing newline, no byte-order mark. A key's value never spans
  two lines, so a changed key is one changed line in a diff, a moved
  subtree is a moved block of lines, `grep '"id": 7,'` finds a node and
  `grep '"parent": 7,'` its children.
- **Escapes.** Strings are escaped as serde_json escapes them (`"`, `\`,
  and the characters below U+0020), and on top of that U+0085, U+2028
  and U+2029 — where some editors and diff viewers break a line — and
  the bidi controls U+202A–U+202E and U+2066–U+2069 — which can make a
  line display in an order other than its bytes — are written as
  `\uXXXX`. All other text is written as raw UTF-8.
- **Streaming.** `save_to` writes the header, then each node's block as
  its walk reaches it, to any `io::Write`, so a large map is never held
  as one text; `save` is `save_to` into a string. A failing writer is a
  `SaveError` with the kind `Io`, at the node being written.
- **Omitted:** every optional key that is unset (inheriting); `channel`
  `0`, `folded` `false`, `hidden` `false`, `opacity` `1.0`; a section's
  `offset` `[0, 0]`; a path's `anchors` `["auto", "auto"]` and `visible`
  `true`; empty arrays; a child node's connector with every field
  inherited and nothing under it; a portal link's `ends` when both ends
  are all-inherited with nothing under them; and `defaults`, and any
  object inside it, that sets nothing.
- **Never written:** anything derived: fitted sizes, depth, child lists,
  resolved colors.
- **Nothing is dropped, and nothing is written that does not load.** A
  document that holds a value the file cannot spell is not saved: `save`
  returns a `SaveError` naming the path the value would have had and
  the node it is in, in the notation of [Errors](#errors), and writes
  nothing. `check_savable` gives the same answer without writing, for
  the mutations and the editor to call when they apply an edit. What it
  refuses:
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
    and so on. The saver hands what it would write, node by node, to the
    loader's own builder, so the two cannot disagree about what loads.
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
| `format` | integer | yes | | The format number: `1`. Read and checked before anything else in the file, wherever it sits; a newer one is refused as written by a newer version. |
| `name` | string | yes | | The map's name. |
| `theme` | [theme](#theme) | no | the empty theme | The theme. Absent is the empty theme (no variables, no variants), which has no other spelling. A theme that is present has at least one variant. |
| `palettes` | object: name to array of [palette groups](#palette-group) | no | none | The palettes. |
| `defaults` | [defaults](#defaults) | no | nothing set | Per-field defaults, consulted after the element and its owners and before the built-in value. |
| `edits` | array of [definitions](#definitions) | no | `[]` | The edits the map defines. V1-D1 extends this schema. |
| `presentations` | array of [definitions](#definitions) | no | `[]` | The presentations the map defines. V1-D2 extends this schema. |
| `verbs` | array of [definitions](#definitions) | no | `[]` | The verbs the map defines. V1-D3 extends this schema. |
| `nodes` | array of [nodes](#node) | no | `[]` | Every node of the map, flat, in depth-first pre-order ([The node list](#the-node-list)). |

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
| `start` | [start point](#start-point) | no | unset | Where a `repeat` ring starts; a ring in another mode ignores it. |
| `direction` | `"clockwise"` or `"counterclockwise"` | no | unset | Which way a `repeat` ring goes around; every stretch still [reads forward](#readable-path-text). A ring in another mode is [two lines](#readable-path-text) and ignores it. |
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
| `id` | integer, `1` to `4294967294` | yes | | The node's identity ([The node list](#the-node-list)): unique in the map, stable across saves. `0` is reserved. Ids do not encode structure; moving a node never renames anything. |
| `parent` | integer, `1` to `4294967294` | no | absent: a top-level node | The `id` of the node it hangs from, which is on the chain of the node listed before it. |
| `channel` | integer `0..=4294967295` | no | `0` | The node's place among its sibling nodes: siblings are listed in ascending channel. |
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
| `sections` | array of [sections](#section) | no | `[]` | In ascending `channel`. |
| `frame` | [frame](#frame) | no | none | The node's outline text. A node with no `frame` draws none; a `frame` of `{}` draws one with every field unset. |
| `connector` | [connector](#connector) | no | see meaning | The line to the node's parent. **Forbidden on a top-level node.** On a child node it is optional: absent means a connector with every field unset and nothing under it, and the saver omits it in exactly that case. |
| `links` | array of [links](#link) | no | `[]` | |

A node's child nodes are not inside it: they follow it in the list.

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
| `min` | number `> 0` | no | open | The lowest camera zoom it renders at. A camera zoom is always above 0, so a bound at or below it would never be reached. |
| `max` | number `> 0` | no | open | The highest camera zoom it renders at. Not below `min`. |

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
| `start` | [start point](#start-point) | no | unset | Where a `repeat` ring starts on the outline; a ring in another mode is [two lines](#readable-path-text) and ignores it. |
| `direction` | `"clockwise"` or `"counterclockwise"` | no | unset | Which way a `repeat` ring goes around; every stretch still [reads forward](#readable-path-text). A ring in another mode is [two lines](#readable-path-text) and ignores it. |
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
| `"repeat"` | The text repeated until the path is full. Around a frame, as many whole repetitions as fit, evenly spaced, so the ring closes without a seam; along a connector, the last repetition is cut. |
| `"spread"` | The text once, spaced out to end where the path ends. On a frame's ring, each of its two lines spaced out over its span. |
| `"once"` | The text once; the rest of the path gets nothing. On a frame's ring, each line centered on its span. |
| `{ "once": { "filler": "·" } }` | The text once, then the filler repeated. On a frame's ring, the filler fills each span on both sides of its line. |
| `"fit"` | The text scaled to fill the path exactly once, within the built-in bounds: `0.5` to `2.0` times the declared size. On a frame's ring, the largest size at which each line fits its span; text too long even at `min` has its bottom line's end cut. |
| `{ "fit": { "min": 0.5, "max": 2.0 } }` | The same, with bounds of your own: `0 < min <= max`. The built-in bounds are written as `"fit"`. |

### Readable path text

Text along a path — a frame's ring, a connector, a link — is placed so
that it reads left to right or top to bottom (D33, §6.2). There is
nothing to set: it is how path text is placed.

**A frame's text is two lines.** A ring whose [fit](#fit) is `once`,
`spread` or `fit` reads over the top of the outline and then along the
bottom, each line left to right on the level part of its half — where
the outline runs within 45° of level — and the steep sides stay clear.
The text is parted at a word break, punctuation staying on its word:

- at its first line break (`\n`, `\r\n`, or any other paragraph
  separator), if it holds one; any further line break reads as a
  space;
- otherwise where the two lines best balance against their halves;
- a text with no word break is one line, on top.

Both lines share one size; the whitespace they part at is not drawn.
`start` and `direction` do not apply: a frame's text always begins over
the top. They are kept and saved, and a ring set to `repeat` uses them.

**A pattern and a connector read stretch by stretch.** On a `repeat`
ring and along a connector or a link, glyphs are never rotated, so
where the path runs leftward (the bottom of a clockwise frame) or
upward (its left side), that stretch's glyphs are placed last to
first, and a word is never torn across two stretches. `start` and
`direction` decide where a `repeat` ring begins and which stretch comes
next; they never make a stretch read backward. A line break in such a
text reads as a space.

### Start point

Where a `repeat` ring starts on the outline (a ring in another mode is
[two lines](#readable-path-text)). Edges run clockwise: top
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
| `fill_fade`, `gap`, `inset`, `spacing` | `>= 0` |
| A `zoom` bound | above `0`, and `min` not above `max` (the bound the mutations hold an edit to) |
| `fit` bounds | `0 < min <= max` |
| Corner radius | `0` to `100` |
| Weight | 100, 200, ... 900 |
| `control_points` | at most 2 (`MAX_CONTROL_POINTS`) |
| A node's `id` and `parent`, a link's `to` | `1` to `4294967294` (`MAX_NODE_IDENT`); a larger integer is this bound, not another error |
| `channel` (node, section, ring) | `0` to `4294967295` (`MAX_CHANNEL`), the most a `usize` holds on wasm32, so every target accepts the same files |
| `palettes`, theme variables, theme variants, `verbs`, `edits`, `presentations` | at most 65536 each, the most an index addresses |

## Errors

A load returns a `LoadError`: a `path`, the `node` it is in, a
`position` in the text, and a `kind`. Rendered, it reads

```text
nodes[2].parent: no node before this one has the id 3; a parent comes before its children (node 5 "the firmament", line 64)
```

- **The `path`** writes keys after a dot and array positions in
  brackets: `nodes[3].frame.rings[0].spans[2].color`. A key that is not
  a plain identifier (a letter or `_`, then letters, digits and `_`),
  which only a name an author chose can be, is quoted in brackets as
  JSON quotes a string, with every control, line separator and bidi
  control escaped too, so that it cannot be read as more path:
  `palettes["My Palette"][0].fill`, `theme.variants[""].bg`. The path
  addresses a node by its place in the list, never by its id, so it is
  always a JSON path a tool can follow (`jq '.nodes[3].frame'`). It is
  empty for an error about the whole text (too large, a byte-order mark,
  not UTF-8), for the map itself (a missing top-level key), and when the
  reader died where no value had begun. An unknown key's path ends in
  the key; a missing key's path is the object that lacks it, and the
  key is in the `kind`. **A path always names something the text
  holds**: a row listed below the channel before it is refused at its
  `channel`, or at the node, section or ring itself when its channel
  is 0 — the default, which a file writes by leaving the key out.
- **The `node`**, on every error whose path starts with `nodes[i]`: its
  place in the list, its `id` — read on its own when the rest of the
  node did not read, so it is absent only when the id itself is what is
  wrong — and an excerpt of its text: the first span of its
  lowest-channel section, else of its first ring, cut to 32 grapheme
  clusters or 256 bytes, whichever comes first, with `…` when cut.
- **The `position`**, a line and a column (1-based; the column counts
  bytes): exact for an error the JSON reader found (a syntax error, a
  wrong type, an unknown or missing key); the start of the node for one
  found after the node was read (a bound, a reference, the chain), which
  is the line of the node's opening `{`; the start of the value for
  one in a top-level value; absent for an error about the whole text.
  It is the only place an error names: a message the reader wrote does
  not carry the reader's own `at line L column C`, which counts inside
  the node or value it read.
- **Nothing from the file is printed raw.** In a rendered error, the
  excerpt, a name, a key and a reader's message have every control
  character, line separator and bidi control escaped, so a map cannot
  rewrite the line its error is shown on.
- **Which error is first** is fixed: the size, the byte-order mark and
  the encoding; then JSON syntax anywhere in the text; then `format`;
  then the other top-level values, all of which are read first, in the
  order of the table above (a wrong type, an unknown or a missing key),
  before any of their content is checked: the counts an index
  addresses, the theme, the palettes, the edits, presentations and
  verbs, and the defaults last; then the nodes in list order; then the
  links, in list order. Within a node, an error the JSON reader finds —
  a wrong type, an unknown or a missing key, anywhere in the node —
  comes first, in the order of the text, since the node is read whole
  before anything is checked; then
  its `id`, `parent` and `channel`; then its other keys; and a node's
  row is checked once its subtree has ended.
- A file whose `format` is a positive integer other than `1` is
  reported as that (`UnsupportedFormat`, which says it was written by a
  newer version and names both numbers), whatever else is wrong with it.
  A `format` that is not a positive integer is invalid.

A `SaveError` has the same `path` and `node`, for the place the value
would have had in the file.

## Limits

| Limit | Value |
|---|---|
| The text | Unbounded on native. On wasm32 the text is refused above `MAX_MAP_BYTES`, 230,087,533 bytes, before it is parsed (`TooLarge`). |
| Elements | Unbounded on native. On wasm32 a map may become at most `MAX_ELEMENTS`, 2,796,202 elements, each verb, edit and presentation counting as one; the one past it is refused, before it is built, at its path (`TooManyElements`). |
| Why those two | On wasm32 memory is the only limit, and a map costs memory by its text and by its elements: its peak while it loads and is saved is at most `text × 7 + elements × 576` bytes, measured on the densest maps the format allows, by the browser's own allocator. Each limit takes half of a 3 GiB budget, so a map under both never traps the page. A document that would be refused there cannot be saved there. |
| The tree's depth | **None.** Every walk is iterative. |
| The number of nodes | None beyond the ids there are and, on wasm32, the element bound above. |
| A node's JSON nesting | At most 9 levels below the node object, by the schema: a portal end's label's span's link target is the deepest. The JSON reader allows 128, and reads each node on its own, so a valid file never comes near it; a text that does is refused, never a crash. |
| Counts an index addresses | Palettes, theme variables and variants, verbs, edits, presentations: at most 65,536 each. |
| Rows | No bound on sections, spans, rings or links per element. |

## Predicates

Values inside a predicate saved in a map (V1-D1) are spelled as they are
everywhere else in this file: colors as above; palettes, variables,
fonts and verbs by name, never by index. The strictness gate refuses any
type in this format that holds an index.

## Example

A small map, exactly as the saver writes it (it is the fixture
`tree.mandala.json`): two top-level nodes, three levels, two node
channels and two section channels, a line link and a portal link that
both name later nodes, a verb named from a node's `on` and from a span's
`link`, a palette, two theme variants and a default. The ids are
deliberately not in list order: the list is ordered by structure, and
node 5 was made after node 3 but sits under node 2.

```json
{
  "format": 1,
  "name": "Genesis",
  "theme": {
    "active": "dark",
    "variants": {
      "dark": {
        "accent": "#d4a24c",
        "bg": "#101014",
        "fg": "#e0e0e0"
      },
      "light": {
        "accent": "#8a5a00",
        "bg": "#f6f3ec",
        "fg": "#202024"
      }
    }
  },
  "palettes": {
    "rainbow": [
      {
        "fill": "#2a1b3d",
        "frame": "#a239ca",
        "text": "#e0e0e0",
        "title": "#ffffff"
      },
      {
        "fill": "var(bg)",
        "frame": "var(accent)",
        "text": "var(fg)"
      }
    ]
  },
  "defaults": {
    "connector": {
      "fit": "repeat",
      "content": "─"
    }
  },
  "verbs": [
    {
      "id": "reveal-days",
      "name": "Reveal the days"
    }
  ],
  "nodes": [
    {
      "id": 1,
      "position": [0.0, 0.0],
      "shape": {"rectangle": {"radius": 25}},
      "palette": "rainbow",
      "on": {"click": "reveal-days"},
      "sections": [{"spans": [{"text": "In the beginning", "size": 24.0, "weight": "bold"}]}, {"channel": 1, "offset": [0.0, 40.0], "spans": [{"text": "Genesis 1:1"}]}],
      "frame": {"fit": "fit", "rings": [{"spans": [{"text": "And the earth was without form, and void; "}]}]},
      "links": [{"to": 6, "kind": "portal"}]
    },
    {
      "id": 2,
      "parent": 1,
      "position": [320.0, -80.0],
      "sections": [{"spans": [{"text": "the heaven"}]}],
      "links": [{"to": 4, "kind": "line", "control_points": [[400.0, 0.0]]}]
    },
    {
      "id": 5,
      "parent": 2,
      "position": [640.0, -120.0],
      "sections": [{"spans": [{"text": "the firmament", "link": {"verb": "reveal-days"}}]}]
    },
    {
      "id": 3,
      "parent": 1,
      "position": [320.0, 80.0],
      "sections": [{"spans": [{"text": "the earth"}]}],
      "connector": {"spans": [{"text": "·"}]}
    },
    {
      "id": 4,
      "parent": 1,
      "channel": 1,
      "position": [320.0, 240.0],
      "sections": [{"spans": [{"text": "darkness upon the face of the deep"}]}]
    },
    {
      "id": 6,
      "position": [0.0, 600.0],
      "sections": [{"spans": [{"text": "Day one"}]}]
    }
  ]
}
```

How it reads, node by node (the nodes start on line 47; each line is
where the node's block opens):

| Line | Node | Parent | The chain before it | Accepted because |
|---|---|---|---|---|
| 47 | 1 | none | empty | a top-level node may always start |
| 57 | 2 | 1 | 1 | 1 is the node before it |
| 64 | 5 | 2 | 1, 2 | 2 is the node before it |
| 70 | 3 | 1 | 1, 2, 5 | 1 is on the chain, which closes 5 and 2; its previous sibling, 2, has channel 0, as 3 does |
| 77 | 4 | 1 | 1, 3 | 1 is on the chain, which closes 3; channel 1 is not below 3's channel 0 |
| 84 | 6 | none | 1, 4 | a top-level node, which closes 4 and 1; the previous one, 1, has channel 0, as 6 does |

Edits that make it invalid, and what the loader says:

- Move node 3's block below node 4's:
  `nodes[4]: siblings are listed in ascending channel, and the sibling before this one (node 4) has channel 1 (node 3 "the earth", line 77)`
- Give node 5 `"parent": 3`:
  `nodes[2].parent: no node before this one has the id 3; a parent comes before its children (node 5 "the firmament", line 64)`
- Move node 5's block to the end of the list:
  `nodes[5].parent: node 2 (nodes[1]) is earlier, but its subtree has ended; a node here can name 6, or none for a new root (node 5 "the firmament", line 83)`
