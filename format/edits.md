# Edits

An **edit** is a named change to a map: saved with the map when the
map defines it, undoable as one history entry, and optionally animated
into place (design §10.1, D6; V1-D1). Edits are the "change the map"
kind of behavior. Presentations, which perform a map without changing
it, are another list and another page.

An edit runs at the selection. From each selected element's node it
takes a set of nodes (`reach`), the elements of one kind within them
(`target`), keeps the ones a `filter` passes, and changes those
elements with its body. Running it is one `Document::apply` and
one history entry; an edit that changes nothing makes no entry.

```json
{ "id": "grow-font-2pt", "name": "Grow font 2pt",
  "description": "Every span of the selected subtrees, 2 pt larger.",
  "reach": "subtree", "target": "spans",
  "mutations": [ { "span": { "text": { "grow_size": 2.0 } } } ],
  "animate": { "ms": 200, "easing": "ease_out" } }
```

## Where edits come from

| Tier | Where | Read |
|---|---|---|
| App | `assets/edits.json`, compiled into the app | at startup |
| User | `edits.json` in the user config directory on native (`$XDG_CONFIG_HOME/mandala/`), `localStorage["mandala_edits"]` in the browser | at startup |
| Map | the map's top-level `edits` list ([`schema.md`](./schema.md#top-level)) | with the map |

The App and User files are `{ "edits": [ … ] }` ([Edits file](#edits-file)).
Ids are unique within each tier. Across tiers the later one wins by
`id`, whole definition: App < User < Map. A User or Map edit with the
id of a bundled handler edit replaces it outright. `edit help` names the
tier the running definition came from.

A User file that does not read is logged and ignored as a whole, as the
other user config files are; a Map edit that does not read refuses the
map, with its path, as every other key of the map does.

### Edits file

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `edits` | array of [edits](#edit) | yes | | The edits, in order. Ids are unique within the file. |

## The definition

### Edit

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `id` | string | yes | | The id keybinds, the console, macros and verbs name it by. Not empty, no control or bidi characters. |
| `name` | string | yes | | The name shown to the user, and the label of its history entry. |
| `description` | string | no | `""` | What it does, for `edit help`. |
| `reach` | [reach](#reach) | yes | | Which nodes, relative to each selected element's node. |
| `target` | [target](#target) | yes | | Which elements within those nodes. |
| `filter` | [filter](#filter) | no | every element | Which of those elements it keeps. |
| `mutations` | array of [mutations](#mutations) | one of three | | Applied once to each element. |
| `mutator` | [mutator](#mutator) | one of three | | Anchored at each element; it reaches below as written. |
| `handler` | string | one of three | | A layout in Rust ([handlers](#handlers)). |
| `animate` | [animate](#animate) | no | instant | How it animates into place. |

Exactly one of `mutations`, `mutator` and `handler` is given; none, or
two, is refused. Every object of an edit refuses a key it does not
define.

### Reach

| Spelling | The nodes |
|---|---|
| `"self"` | each selected element's node |
| `"children"` | their child nodes |
| `"descendants"` | every node below them |
| `"subtree"` | them and every node below them |
| `"parent"` | their parent nodes; a top-level node has none |
| `"siblings"` | the other nodes that share their parent; top-level nodes are each other's siblings |

### Target

| Spelling | The elements, within each reached node |
|---|---|
| `"node"` | the node |
| `"sections"` | its sections |
| `"spans"` | every span of its sections |
| `"frame"` | its frame |
| `"rings"` | every ring of its frame |
| `"connector"` | its connector |
| `"links"` | its outgoing links |
| `"labels"` | the labels of its connector and its links |

The elements are taken in tree order, each once, however the selection
or the reach overlaps.

## Bodies

### Mutations

A mutation names the element it changes, then its op:
`{ "<element>": { "<op>": <value> } }`, in `snake_case`. Unit ops are
bare strings (`{ "node": "toggle_folded" }`), tuple ops arrays
(`{ "path": { "set_anchors": ["right", "left"] } }`), struct ops objects
(`{ "span": { "insert_text": { "at": 0, "text": "» " } } }`). An op that
sets an optional field takes `null` to unset it, so the element inherits
again.

Under `mutations`, every mutation must apply to the target's elements;
one aimed at another kind is refused at load, at its path.

| Element | Applies to | Ops |
|---|---|---|
| `node` | a node | `set_position` `[x, y]`, `nudge` `[dx, dy]`, `set_min_size` `[w, h]`, `set_shape`, `set_palette` (a palette name), `set_zoom` `{min, max}`, `set_folded`, `toggle_folded`, `set_hidden`, `set_opacity`, `set_click` / `set_hover` (a verb id), `set_channel`, `text` |
| `section` | a section | `set_offset`, `nudge`, `set_min_size`, `set_align`, `set_channel`, `text` |
| `span` | a span | `set_text`, `insert_text` `{at, text}`, `delete_text` `{start, end}` (grapheme clusters), `set_link` (`{"url": …}` or `{"verb": …}`), `text` |
| `frame` | a frame | `set_visible`, `set_start`, `set_direction`, `set_fit`, `set_gap`, `set_inset`, `layout`, `text` |
| `ring` | a ring | `set_fit`, `layout`, `set_channel`, `text` |
| `path` | a connector or a link | `set_fit`, `set_spacing`, `set_size` (points), `set_color`, `set_content`, `set_anchors`, `set_control_points`, `move_control_point` `{index, to}`, `set_visible`, `set_zoom` |
| `cap` | a cap | the `text` ops |
| `label` | a label | `set_t`, `set_offset`, `text` |
| `portal_end` | a portal end | `set_border_t`, `set_offset`, `set_color`, `set_size` (points), `set_content` |

`text` is a text style op: `set_font` (a family name), `set_size`,
`grow_size`, `scale_size`, `set_color`, `set_weight`, `set_italic`. A
size is a number in the unit the element's place stores: points, or a
fraction of the node's shorter side for a frame and a ring.
`grow_size` adds to the size the element *displays* — its own, or the one
it inherits — and stores the result as its own, so growing a subtree
grows every span by the same amount whether or not it set a size.
`layout` is `set_span`, `set_sides` or `set_spacing`, as a frame's or
ring's [layout keys](./schema.md#frame) are spelled.

Every value is spelled as the map spells it (`schema.md`): colors
`"#rrggbb"`, `"#rrggbbaa"`, `"var(name)"`, `"palette(frame)"`; shapes,
fits, start points, spans, sides and weights as their keys are. The
map's own rows — the theme, the palettes, the defaults — are out of an
edit's reach, and a link's kind changes only with the structure an
authored edit cannot hold, so neither has an op here. An authored edit
holds no structural command (design §10.1): moving nodes and making
links needs a handler.

### Names are resolved when the edit runs

An App or User edit is read before any map is open, so it cannot hold a
map's index, and an index would name another entry the moment a name
was added. An edit names theme variables (`"var(accent)"`), palettes,
fonts and verbs by **name**, and they are resolved against the open map
each time the edit runs. The same App edit therefore means the same
thing on two maps whose themes list their variables in other orders.

A name the open map does not define refuses that run with a message that
names the edit, the field and the name —
``edit `tint`: `color` names theme variable `accent`, which this map does not define`` —
and nothing is applied. A font family this build has no face for is
refused when the edit is read, since no map could answer it.

### Mutator

The advanced body: the mutator AST of [`mutations.md`](./mutations.md#mutator--the-mutatornode-ast)
in the document's kinds, anchored at each element. Its root's `kind`
is the target's (it is anchored nowhere else, and a root of another
kind is refused). What it reaches below its root is the author's to
say.

A mutator that reaches below its root — a child that is not a `void`,
or an `instruction` root — is walked once from each anchor that has no
other anchor above it. An anchor inside another's subtree is **not**
walked from: it is changed only where the walk from the outer one
reaches it, so nothing is changed twice. Only nodes nest, so this is a
`node` target's rule:

- With `reach` `descendants` or `subtree` the reached nodes always
  nest, and such a mutator is refused when the edit is read: it would
  start only from the outermost. Reach `self` and let the mutator walk
  down (`repeat_while` reaches any depth), or use `mutations`, which
  change every element they reach.
- With any other reach the selection itself can nest — a node and its
  child both selected — and the inner one is then walked from the
  outer one only. `edit inspect` lists the anchors each walk starts
  from, and the ones it does not, apart.

A mutator that touches only its root is walked from every anchor.

| Node | Keys |
|---|---|
| `{"void": …}` | `kind`, `channel`, `children` (optional) |
| `{"macro": …}` | `kind`, `channel`, `mutations`, `children` (optional) |
| `{"instruction": …}` | `kind`, `channel`, `instruction`, `children` (optional) |

`instruction` is `"repeat_while_always_true"` or
`{"repeat_while": <filter>}`. `map_children` is not spelled: it pairs
children by position, which a document walk refuses. A macro's mutations must apply to its
`kind`. There are no runtime holes: an edit has no build context, so
`single`, `repeat`, templates and runtime counts and lists are not
spelled.

### Filter

A predicate: the generic shape (`fields`, `always_match`) with the
map's own values in the fields (`format/mutations.md`, "The document's
kinds and predicate fields", spelled as this page spells values: a
color as `"var(name)"`, a palette, a font and a verb by name). The
first field test an element answers decides; with no test it answers,
the element is dropped.

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `fields` | array of `[field, comparator]` | no | `[]` | The tests, in order. A comparator is `{"Equals": false}` (`==`), `{"Equals": true}` (`!=`), `{"GreaterThan": …}`, `{"LessThan": …}` or `{"Exists": …}`. |
| `always_match` | boolean | no | `false` | Keep every element. |

```json
{ "fields": [ [ { "font_size": { "pt": 30.0 } }, { "LessThan": false } ] ] }
```

### Handlers

`handler` names a layout in Rust: a function that reads the map and
where it is drawn and returns its walks — a mutator and its anchors for
each write it needs — never writing the map itself
(`lib/baumhard/src/edit_handlers.rs`). The walks are the steps
of the edit's one request, so it is still one history entry (amendment
on #250: one walk applies one mutator per sibling key, and sibling nodes
commonly share one, so a layout that places each node apart needs a walk
per node). Its `target` is `node`, and it is given the nodes `reach`
names. A handler that declines says why, and the refusal names the edit.

| Handler | Lays out |
|---|---|
| `flower` | each node's child nodes round it at one radius, evenly spaced, clockwise: on the full circle, the first at the top, or on a fan where the circle is blocked |
| `cascade` | each node's descendants as a top-down tree, rows below their parents |

A layout places no node over another drawn node — a sibling, the
parent, or anything else on the map. The distances it tries, from
where a node belongs, are 64 steps of 40 units, then doubling steps out
to twice the canvas bound (about 19 more). Past the drawn nodes a probe
meets nothing, so a clear place is found unless a drawn node reaches
across all of them; then the node stays where it belongs.

- A flower keeps its shape: every petal at one radius, evenly spaced.
  It tries each radius from the base out, 32 of them 20 units apart.
  At each radius it tries the full circle, turned by an eighth of the
  petals' spacing at a time; then fans 30°, 60°, … narrower than the
  full circle, each centered in turn on twelve directions from the top.
  The first that is clear is taken: no petal over a drawn node, a placed
  petal or another petal, and no petal's line through any of them — the
  straight line from the parent's center to the petal's, where it runs
  outside the two. (A connector is drawn as a curve from the parent's
  outline; the straight line stands for it.) A petal's own line longer
  than 16,384 units outside the parent and the petal is not checked,
  and an arrangement with one is not taken, so a flower round a very
  large parent falls to the per-petal search below. A flower whose
  parent sits close above another node so becomes a fan that leaves a
  gap where that node is.

  Nor may a petal sit on what the run keeps. Once a family is laid
  out, the run keeps its lines and its parent's connector from its own
  parent — the straight line between their centers, where it runs
  outside the two, each where the run placed it or else where it is
  drawn — and every family laid out after it keeps clear of them all,
  not only of its own parent's connector. Only these are kept: a
  connector neither of whose ends the run flowers is not avoided. A
  kept line or connector longer than 32,768 units is kept only in its
  two end stretches of 16,384 units, beside the nodes it joins, so a
  later family's petal may sit on its middle; a family's own parent's
  connector is kept whole while its petals are laid, however long.

  Only when no such arrangement is clear does each petal move outward
  along its own ray, by those distances, until it is clear of drawn
  nodes, placed petals, its parent's connector and what the run keeps.
- A cascade row moves as a whole and keeps its spacing. It tries each
  of those depths below where it belongs. At each depth it tries
  straight below first, then 40 units right, 40 left, 80 right, 80
  left, and so on to 16 steps each way. A single node never leaves its
  row.

What is drawn is asked of the projector's index of drawn rectangles, a
query at a time. A run keeps only the nodes it moves, the rectangles
it has placed and the lines and connectors above, each kept line in
pieces of about 256 units — at most 128 of them: the whole line up to
32,768 units, else 64 in each of its two end stretches — filed in
cells of 512 units, so a petal reads only what is kept near it, and the
run's work follows the families it lays out, not the size of the map.
Near it may be a crowd: kept lines, unlike placed rectangles, may
overlap, so every line and connector the run keeps that converges on
one node is filed in the cells beside it, and a petal there reads them
all. A petal larger than 64 of those cells reads, instead, along its
outline or through the cells that hold kept pieces, whichever is fewer.
A node moved by hand, or by a `set_position` or `nudge`, goes where it
is put, overlap or not.

A handler places what is drawn: a hidden node, or one under a fold, is
left where it is, and takes no place in the layout — a flower of
twelve children with one hidden has eleven petals, and a cascade row
leaves no gap for it. A position a layout computes past the canvas
bound (a magnitude of 1e9) is pinned at the bound; the edit is not
refused.

## Animate

### Animate

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `ms` | integer | yes | | How long the change takes, in milliseconds: at most 60,000. |
| `delay_ms` | integer | no | `0` | How long it holds at its start first, in milliseconds: at most 60,000. |
| `easing` | `"linear"`, `"ease_in"`, `"ease_out"`, `"ease_in_out"` | no | `"linear"` | How progress follows time: `t`, `t²`, `1 - (1 - t)²`, `t²(3 - 2t)`. |
| `snap` | `"start"`, `"end"` | no | `"end"` | When the fields that do not blend switch. |

An animated edit runs as a transaction: it is applied once to learn
every field's value before and after, then each frame writes the eased
progress — numbers, positions, sizes and literal colors blend; every
other field, and the structure, stands on the side `snap` picks — and
at the end it commits as **one** history entry, identical to the
instant edit's. The app's clock drives it, so under a virtual clock
(`--headless`, `clock.step`) a step lands it on exact instants.

- Another change while it runs — another edit, an action, a console
  line — first finishes it and commits it, then proceeds. An action or
  a console line does so before it reads the map, so a copy or a field
  value it reads is the edit's end, never a frame of it. Actions that
  only move the camera, change the selection or toggle the FPS readout
  leave it running. So do the verbs `clock` (a step is how a virtual
  clock moves it), `help`, `fps`, `mode`, `screenshot`, `record`,
  `save` (which writes the map as committed) and `edit` (whose
  `inspect` says when an edit is still animating).
- Undo while it runs finishes it, commits it, and undoes it: the map is
  as it was before the edit, and redo brings the edit back.

## Bounds

A map-borne definition nests at most **32 levels** (a mutator node is a
level) and has at most **65,536 parts** (mutator nodes, mutations and
filter tests). Each is refused at load past its bound, at the
definition's path, naming the bound. Reaching any depth of the map is
done by repeating down the tree (`repeat_while`), not by nesting, and
the map's own depth has no bound.

An animation's `ms` and its `delay_ms` are each at most **60,000** (one
minute), refused at load past it, at the key's path. While an edit
animates the app keeps drawing frames and the change is not yet
committed: a save then writes the map without it, a quit does not ask
about it, and a settled wait does not settle. A minute is far longer
than a person watches one change land (the bundled edits take 150 to
400 ms), and short enough that a number from a shared map, or a typo,
cannot hold the app there for hours.

Inside `edits` a text that nests
deeper than any definition within the bounds can is refused the same
way before the JSON reader's own limit, so the refusal is always this
one.

## On the surfaces

- Console: `edit list`, `edit apply <id> [node-id …]`, `edit help <id>`,
  `edit inspect <id>` (the resolved reach and target, the elements it
  reaches at the selection, its body and its tier, and its walks: where
  each starts, and for a handler what it returns, from running it).
- Keys: `edit_bindings` in `keybinds.json` maps a key combo to an edit
  id.
- Macros: the `Edit` step, `{"kind": "Edit", "id": "…"}`
  ([`macros.md`](./macros.md)), and so IPC's `act.macro`.
