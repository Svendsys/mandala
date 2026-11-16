# Presentations

A **presentation** performs a map without changing it (design §10.2,
D6, D17; V1-D2). It is never saved, never in the undo history and never
marks the map changed: it animates on top of the map as it stands,
whatever is edited underneath. A hover lift, a click pulse, a shimmer
and a sequential reveal are presentations. Edits, which change a map,
are another list and another page ([`edits.md`](./edits.md)).

A presentation is **started on nodes**. On each node it takes the
elements of one kind (`target`), keeps the ones a `filter` passes, and
writes its body there, at a progress `t` that moves from 0 to 1 and
back as its `play` says. Started on N nodes it is N **instances**, one
per node, each with its own `t`: stopping it on one node stops that
node's instance alone.

```json
{ "id": "glow", "name": "Glow", "play": "while", "target": "spans",
  "mutations": [ { "span": { "text": { "set_color": "#ffe9a8" } } } ],
  "animate": { "ms": 150 }, "return": { "ms": 300 } }
```

## Where presentations come from

| Tier | Where | Read |
|---|---|---|
| App | `assets/presentations.json`, compiled into the app | at startup |
| User | `presentations.json` in the user config directory on native (`$XDG_CONFIG_HOME/mandala/`), `localStorage["mandala_presentations"]` in the browser | at startup |
| Map | the map's top-level `presentations` list ([`schema.md`](./schema.md#top-level)) | with the map |

The App and User files are `{ "presentations": [ … ] }`
([Presentations file](#presentations-file)). Ids are unique within each
tier. Across tiers the later one wins by `id`, whole definition:
App < User < Map. `present list` names the tier each definition came
from.

A User file that does not read is logged and ignored as a whole; a Map
presentation that does not read refuses the map, with its path, as
every other key of the map does.

### Presentations file

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `presentations` | array of [presentations](#presentation) | yes | | The presentations, in order. Ids are unique within the file. |

## The definition

### Presentation

| Key | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `id` | string | yes | | The id the console and verbs name it by. Not empty, no control or bidi characters. |
| `name` | string | yes | | The name shown to the user. |
| `description` | string | no | `""` | What it does. |
| `play` | `"while"`, `"once"`, `"loop"`, `"hold"` | yes | | Its [lifecycle](#lifecycle). |
| `layer` | `"document"`, `"glyph"` | no | `"document"` | The [vocabulary](#layers) its body is written in. |
| `target` | [target](./edits.md#target) | yes | | Which elements within each node it is started on. On the glyph layer, one of `"sections"`, `"rings"`, `"connector"`, `"links"`, `"labels"`. |
| `filter` | [filter](./edits.md#filter) | no | every element | Which of those elements it keeps. |
| `mutations` | array of [mutations](./edits.md#mutations) | on the document layer | | Written once on each element. |
| `glyph` | array of [run mutations](#the-glyph-layer) | on the glyph layer | | Written once on each element's run. |
| `animate` | [animate](./edits.md#animate) | no | instant | How it plays in. |
| `return` | [animate](./edits.md#animate) | no | as `animate` | How it plays back out. |

The body is the layer's: `mutations` and no `glyph` on the document
layer, `glyph` and no `mutations` on the glyph layer. Both, neither, or
the other layer's is refused at load. Every object of a presentation
refuses a key it does not define.

A presentation has no `reach`: it plays on the nodes it is started on.
Which nodes those are is the caller's to say — the console's
`present start`, and a verb's `present` step (V1-D3).

## Lifecycle

`t` is an instance's progress: 0 is the map as it is, 1 is the body
fully written.

| `play` | In | Holds | Out | Ends |
|---|---|---|---|---|
| `while` | 0 → 1 over `animate` | until it is stopped | current → 0 over `return` | at 0 |
| `once` | 0 → 1 | no | straight back, 1 → 0 | at 0 |
| `loop` | 0 → 1 | no | 1 → 0, then in again | once stopped, at the end of the cycle it is in |
| `hold` | 0 → 1 | until it is stopped, or the map closes | current → 0 | at 0 |

- **A direction follows its timing whole**, from the `t` the instance
  has when the direction begins: it stays there for `delay_ms`, then
  moves to its end over `ms`, eased by `easing`. So the way in is
  `animate`'s and the way out `return`'s, each with its own delay, and
  a `return` left out is `animate` again, delay included.
- **Stopping on the way in turns back from the current `t`.** Nothing
  jumps. A stopped `loop` instead plays its cycle to the end.
- **Starting a presentation where it is already playing restarts that
  instance**: it turns to its way in from the current `t`. It never
  stacks a second instance on the same node.
- **A `while` and a `hold` differ in who stops them.** Both hold at 1
  until stopped; a held trigger (a hover, V1-D3) stops the `while`
  presentations it started when it ends, and a `hold` stays until it is
  stopped by name.
- **Time is the app's clock** ([`ipc.md`](./ipc.md#clock--determinism-ipc-09)).
  Under a virtual clock (`--headless`, `clock.step`) an instance is at
  an exact `t` at every instant: a direction that ends between two
  frames hands over at its own end, not at the frame that noticed.
- `animate` and `return` are held to the edit format's bounds: `ms` and
  `delay_ms` at most 60,000 each. A `loop` whose `animate` and `return`
  both take no time is refused: it would be at no place at any instant.
- `snap` (default `"end"`) says where the fields that do not blend
  stand between 0 and 1: with `"end"` they take the body's value only
  at `t = 1`, with `"start"` for every `t > 0`. Numbers, positions,
  sizes and literal colors blend
  ([`edits.md`](./edits.md#animate)). On the way out the `snap` is
  `return`'s.

## Layers

### The document layer

`"layer": "document"` (the default). The body is `mutations`, in the
document's vocabulary and the edit format's spelling
([`edits.md`](./edits.md#mutations)): position, size, colors, text
style, frame and path fields, `hidden`, `opacity`. What it writes goes
through projection as an edit's does, so a node lifted on hover takes
its connections with it.

- **Each mutation is one its target takes.** A node's mutation on
  `"target": "sections"` is refused at load, at the mutation's path
  (``this mutation does not apply to the presentation's target``): a
  presentation has no mutator to reach below its target with.
- **Fields only, never structure** (D17). The format has no spelling
  for a structural command, so a file that writes one does not read;
  and when an instance starts the document judges its mutator once
  more and refuses a structural command by name
  (``a presentation changes fields only, never structure``). `hidden`
  and `opacity` are fields, so a presentation can conceal, reveal and
  fade.
- **Names are resolved when it starts**, against the open map, as an
  edit's are when it runs
  ([`edits.md`](./edits.md#names-are-resolved-when-the-edit-runs)): a
  theme variable, a palette, a font and a verb are written by name. A
  name the map lacks refuses the start —
  ``presentation `glow`: `color` names theme variable `accent`, which this map does not define`` —
  and nothing starts.
- **Its elements are resolved when it starts**, once: the target
  within the node, kept by the filter, read from the map itself. An
  element an edit later removes is skipped while it is gone; an
  element added later is not joined. A node where the target and the
  filter name nothing gets no instance.

**It is never part of the map.** The instances form a layer on top of
the document:

```text
committed map ⊕ open transaction ⊕ presentation layer   →  what is drawn
```

- Every change underneath — an edit, a drag's frame, an undo, a redo —
  is made with the layer written back out, and the layer is applied
  again on the map as it then stands. The history holds the change
  alone, and undoing leaves the presentation on top.
- A save writes the map alone. `dirty` and the document's revision
  never move for a presentation.
- **What reads the map to change it reads the map.** An action, a
  console line or an edit that builds its change from what it reads —
  a copy, a layout, a value to step from — reads with the layer set
  aside, so nothing a presentation shows can be committed: each step,
  given the same app state, builds the change it builds with nothing
  playing. Actions that only move the camera, change the selection or
  read what is drawn leave it applied, and a selection made by what is
  drawn carries into the next step — `SelectAll` under a presentation
  that folds a node does not select what is under the fold, so a change
  after it reaches less than it would with nothing playing
  ([`ipc.md`](./ipc.md), "The map itself, or what is drawn").
- Two instances that write one field combine in the order they
  started: the later one starts from what the earlier one shows.
- On wasm32 a presentation is held to the platform's bounds as an edit
  is ([Limits](./schema.md#limits)): what it shows may not take
  the map past them. The instances are held to them together, in the
  order they started: each is judged, at the most it will show (its
  change written whole), on top of the map and of every instance
  started before it as that one stands — an instance whose text has
  not yet switched (`snap`) still shows the text under it, and frees
  no room until it switches. So a start on N nodes — N instances —
  must fit as a whole, and so must several presentations playing at
  once; when an instance switches, every one after it is judged
  again.
  - **At a start**, nothing starts, and the refusal is the answer where
    the start was asked for — the console line's error, and so
    `act.console`'s — naming the presentation and then the writer's own
    words:
    ``presentation `wordy`: the map would hold N bytes of text; this platform holds at most M``.
    A start is judged where the layer puts it: its instances join the
    end of the layer, on top of the ones already playing as they
    stand, and if one does not fit the start is taken back whole.
    While the layer is set aside (a change being built from the map),
    a start is judged on the map, its new instances one over another,
    and the layer is judged whole when it comes back, as below.
  - **While it plays** — an edit under it grew the map, or the
    instances before it now leave no room, so writing the instance
    back on would pass the bounds — the edit stands and the instance
    shows nothing: it keeps playing, is tried again after
    every later change, and shows again as soon as it fits. The reason
    is on the instance, in the same words: `present list` ends its line
    with ``— not applied: the map would hold N bytes of text; this
    platform holds at most M``, `query.animations` carries it as
    `refusal`, and the log has one warning when it begins
    (``presentation: `wordy` is not applied on the map as it stands: …``).

### The glyph layer

`"layer": "glyph"`. The body is `glyph`: run mutations, written on the
glyph runs the target elements project to
([`projection.md`](./projection.md)) — effects with no meaning in the
data. They never touch the document.

| Run mutation | Value | Effect |
|---|---|---|
| `{"run_opacity": n}` | `0` to `1` | The run's opacity, multiplied into every glyph's alpha. |
| `{"run_origin": [x, y]}` | canvas units | The run's origin: a box run's top-left, or the offset of every glyph of a path run. |
| `{"run_overrides": {"offset": {"range": [s, e], "value": [dx, dy]}}}` | canvas units | Moves each glyph of the range. |
| `{"run_overrides": {"scale": {"range": [s, e], "value": n}}}` | `n ≥ 0`; `1` is as shaped | Scales each glyph of the range about its center. |
| `{"run_overrides": {"color": {"range": [s, e], "value": "#rrggbbaa"}}}` | a literal color, or `null` | Colors each glyph of the range; `null` returns it to its own color. |
| `{"run_overrides": "clear"}` | | Drops every override of the run. |

- **`range` is `[start, end]`, half-open, over the run's placed
  glyphs**, counted as [`mutations.md`](./mutations.md#glyph-run-mutations)
  counts them: in text order, a repeated run counting every copy, a
  frame's text set in two lines counting the first line then the
  second, fillers and a frame's sides ornament skipped. A range past
  the end writes up to it. A per-glyph effect — a wave, letters that
  lift one after another — is several ops over ranges.
- **A target draws runs of its own**: a section's box, a ring's run, a
  connector's or a link's body, a label. `"node"`, `"frame"` and
  `"spans"` draw none and are refused on the glyph layer; a span is a
  range of its section's run.
- **Offsets, scales and the run's opacity and origin blend with `t`.**
  A color override blends only where the glyph was already colored by
  an override; otherwise it appears whole where `snap` puts it
  (`"start"`: as soon as the instance moves; `"end"`: at `t = 1`).
  To fade a glyph from its own color, give it that color with a `hold`
  presentation first and start the fading one after it: instances on a
  run combine in the order they started, so the second blends from the
  first's color. `maps/demos/dickinson.mandala.json` does:
  `feathers-rest` gives the letters of "feathers" the line's color and
  the trail after them (which `feathers-trail` writes, below) the same
  color at no opacity, and `feathers-lift`, started after it, thins the
  letters and brings the trail up from there.
- **A glyph the map does not draw has nothing to write on.** A run
  mutation moves, sizes and colors glyphs the run already holds; it
  adds none. A trail, a spark or an underline that a presentation
  brings up is text of the element, which the presentation then
  colors and moves. Text a map saves is the map's — it is laid out,
  exported, searched and copied with the rest — so text that is there
  only to be performed is better written by a document-layer
  presentation into a span the map saves empty. Dickinson's first line
  does: its last span is empty, and `feathers-trail` writes the five
  dots into it (`set_text`) while it plays. The line is saved centered,
  as the rest of the map is; `feathers-line`, started before the trail,
  aligns it left at the offset centering gave it (`set_align`,
  `set_offset`), so the trail added at its end moves nothing before it.
- When projection rewrites a run an instance plays on — its text
  changed, its node moved — the instance is written on it again, and
  on no other run. The selection's tint and a presentation share a
  run: the presentation is written over the tint, and when the last
  instance on a run ends the run is exactly what projection and the
  tint make it.

## The sequential reveal

A map that expands for its viewer is one presentation and its starts
and stops (design §10.2, D17):

```json
{ "id": "concealed", "name": "Concealed", "play": "hold", "target": "node",
  "mutations": [ { "node": { "set_hidden": true } }, { "node": { "set_opacity": 0.0 } } ],
  "return": { "ms": 400 } }
```

Started on the nodes to conceal, it has no `animate`, so it is at
`t = 1` at once: hidden, and at opacity 0. Stopping it on one node
plays that node's `return`. **`snap` is `"end"`** (the default), so
`hidden` holds only at `t = 1`: the node is shown as soon as its return
begins, and its opacity rises from 0 to 1 over the 400 ms. With
`"start"` it would stay hidden until the return had finished, and pop
in. The saved map always shows every node; the concealment exists only
while it is performed.

## Bounds

A map-borne definition has at most **65,536 parts** (mutations, run
mutations and filter tests), refused at load past it, at the
definition's path. A presentation has no nested part, so the **32
levels** a definition may nest ([`edits.md`](./edits.md#bounds)) are
more than any presentation the format reads; inside `presentations` a
text that nests deeper than a definition within the bounds can is
refused as too deep a definition, before the JSON reader's own limit,
so the refusal is always this one.

## On the surfaces

- Console: `present start <id> [node-id …]`, `present stop <id>
  [node-id …]` (the selection when no node is named) and `present
  list` (the definitions that run on this map, then the instances
  playing: the node, `t`, and which way each is going). It is how a
  presentation is started today: nothing in a map starts one when the
  map opens or a node is clicked until verbs have steps (V1-D3). The
  demo maps that carry presentations name the node to start each on in
  its `description` — `present start an-ellipse 2` on
  `maps/demos/frames.mandala.json`.
- IPC: [`query.animations`](./ipc.md#queryanimations) lists the
  instances playing and an animated edit that is running, and
  `clock.wait until=animations_complete` waits for them to come to
  rest. `act.console` runs the console lines above.
- Verbs: a map verb's `present` and `stop` steps (V1-D3).
