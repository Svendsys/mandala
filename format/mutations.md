# Glyph mutations

The glyph mutation vocabulary: `GfxMutation`, its command, delta and
glyph-run forms, and the `MutatorNode` AST that carries them, which
baumhard's `GlyphModel` family keeps (design decision D24). Their
published examples are read from this file by the tests, so an example
here is one the code parses.

The custom-mutation *carrier* that used to wrap this vocabulary — the
`CustomMutation` object, its loader and console verb, and the triggers
that fired it — was deleted with the old model in V1-C3 (#244). Edits
replace it (V1-D1, #250), and V1-E4 (#256) documents them. Nothing in
`.mandala.json` holds a glyph mutation today.

## Vocabulary

### Mutation vocabulary

`GfxMutation` is an enum from `baumhard::gfx_structs::mutator` — the
glyph payload of the mutation engine. The JSON tag is the variant
name; the payload is the variant's inner value. It has no no-op
variant: a mutator with nothing to apply says so structurally.
The most common variants for authoring:

- `{"AreaCommand": { "GrowFont": 2.0 }}` — grow font size by 2pt.
- `{"AreaCommand": { "ShrinkFont": 2.0 }}` — inverse.
- `{"AreaCommand": { "NudgeRight": 50.0 }}` — shift x by 50px.
  Sibling variants: `NudgeLeft`, `NudgeUp`, `NudgeDown`.
- `{"AreaCommand": { "SetFontSize": 18.0 }}` — absolute font size.
- `{"AreaCommand": { "MoveTo": [100.0, 200.0] }}` — absolute x,y.
- `{"AreaCommand": { "Rotate": { "pivot": [0.0, 0.0], "degrees": 90.0 } }}`
  — rotate the area's position clockwise around `pivot`. Degrees,
  not radians; clockwise in screen space, where `+y` points down.
  Mirrors its model-side twin `{"ModelCommand": { "Rotate": … }}`.

  **`pivot` is a two-element `[x, y]` array, not an
  `{"x": …, "y": …}` object.** The *variant* is struct-shaped (named
  members `pivot` and `degrees`), but the `Vec2` inside it is
  serialized by `glam` as a sequence, and its deserializer accepts
  nothing else. The object form fails with
  `invalid type: map, expected a sequence of 2 f32 values`, and
  whatever file carries the mutation fails to read it. The same
  applies to every future `Vec2` payload on either command enum.
  `do_area_rotate_command_json_wire_shape` **reads the example on the
  line above out of this file** and parses it, so editing it back to
  the object form fails the suite rather than shipping a doc whose
  example cannot be read.

The full vocabulary lives in
`lib/baumhard/src/gfx_structs/area_mutators.rs` under
`GlyphAreaCommand`. Same enum-variant-as-JSON-tag convention.

> **Added (pre-V1, per `CODE_CONVENTIONS.md` §10).** `Rotate` is
> new on the area side; the model side already had it. The addition
> is purely additive — no existing key changed meaning, so no
> in-repo fixture, bundled asset, or `maptool` path needed
> migrating. Previously `GlyphArea::rotate` existed in Rust but no
> command could reach it, which is why its missing translate-back
> (it rotated the pivot-relative vector and never added the pivot
> back, teleporting the area toward the origin) went unnoticed.

### Delta mutations and their field-map keys

`{"AreaDelta": …}` and `{"ModelDelta": …}` carry a *set* of field
payloads rather than one named command. On the wire the payload is
an object with a single `fields` member, and **the keys of that map
are field-type tag names** — one per touched field, plus a sibling
`Operation` entry naming the arithmetic that governs all of them:

```json
{
  "AreaDelta": {
    "fields": {
      "Position": { "Position": { "x": 100.0, "y": 200.0 } },
      "Operation": { "Operation": "Assign" }
    }
  }
}
```

The valid keys are exactly the variants of `GlyphAreaField`
(`Text`, `Scale`, `LineHeight`, `Position`, `Bounds`,
`ColorFontRegions`, `Outline`, `Shape`, `ZoomVisibility`,
`Operation`) and of `GlyphModelField` (`GlyphMatrix`, `GlyphLine`,
`GlyphLines`, `Layer`, `Position`, `Operation`). Both tag sets are
*derived* from the field enums, so the key list can never drift from
the fields that actually exist.

A `GlyphLine` payload (used directly by `GlyphLine`/`GlyphLines`
and nested inside `GlyphMatrix`) carries a required
`ignore_initial_space` boolean alongside its `line` array of runs.
When it is `true` the rhs's leading whitespace is *transparent*:
the all-whitespace runs in front are skipped, and the first run
carrying content paints at its own grapheme offset — its indent
counted into the offset, not written over the target. An rhs that
is entirely whitespace therefore paints nothing at all.

Every run after that first one paints at **its own** grapheme
offset within the rhs, in order. Offsets are columns, not run
ordinals: the rhs's run boundaries need not line up with the
target's, and after the first paint they generally do not. Two
rhs payloads that spell the same text with different run
boundaries produce the same result; only the column each run
starts at matters. Runs that reach past the end of the target
extend it, padding any gap with whitespace.

> **Behavior change (pre-V1, per `CODE_CONVENTIONS.md` §10).** All
> the statements above are new. The path previously mixed byte,
> `char`, and grapheme offsets, so an indent containing a
> multi-byte space (U+3000) panicked outright and a multi-char
> cluster (CRLF) painted one column off; runs after the first were
> placed by run ordinal against the target rather than by their own
> column, which reordered and mislaid them; an rhs with more runs
> than the target also panicked; and an all-whitespace rhs blanked the
> target instead of showing through. No in-repo fixture or bundled
> asset set the flag, so there was nothing to migrate — but a
> hand-authored file that relied on the old blanking behavior needs
> `ignore_initial_space: false` to keep it.

The area-side operation key is `Operation`, matching the field
variant it tags and the model-side key of the same meaning; there is
no `ApplyOperation` spelling and no `Flags` key, and either is refused
as an unknown variant.

### Glyph run mutations

The glyph layers (design §12.1) are made of one element type besides
areas and models: a **`GlyphRun`**, one whole TextFlow — a shaped box
or a shaped run placed along a path. Three run mutations are
*authored*: a presentation (V1-D2) or an edit (V1-D1) will name them. The one
that replaces a whole payload, `RunSet`, belongs to projection and is
never authored; it has **no JSON spelling**, and `{"RunSet": …}` in a
file is refused as an unknown variant.

There is no fill element (V1-B12): a map has one background, the color
the canvas is cleared to, and every other color comes from a glyph. A
mutation naming a fill — `FillColor`, `FillOpacity`, `FillRect` or
`FillSet` — is refused as an unknown variant, and a list holding one
does not read.

```json
{
  "mutations": [
    {"RunOpacity": 0.5},
    {"RunOrigin": [12.0, -4.0]},
    {"RunOverrides": {"Offset": {"range": {"start": 0, "end": 3}, "value": [0.0, -2.0]}}},
    {"RunOverrides": {"Color": {"range": {"start": 0, "end": 3}, "value": [255, 0, 0, 255]}}},
    {"RunOverrides": {"Color": {"range": {"start": 4, "end": 6}, "value": null}}},
    {"RunOverrides": {"Scale": {"range": {"start": 3, "end": 6}, "value": 1.5}}},
    {"RunOverrides": "Clear"}
  ]
}
```

- `RunOpacity` — a number, multiplied into the alpha of every glyph of
  the run.
- `RunOrigin` — a two-element `[x, y]` array (not an `{"x", "y"}`
  object, as for every `Vec2`): the canvas position of a box run's
  top-left, or the offset applied to every placed glyph of a path run.
  The run's hit-test box moves with it.
- `RunOverrides` — per-glyph presentation effects, one of four ops.
  `Offset` adds `value` to the position of each glyph in `range`;
  `Color` replaces their color with `value` (`null` returns them to
  their own); `Scale` multiplies their size by `value`; the bare string
  `"Clear"` drops every override. `range` is `{"start": s, "end": e}`,
  half-open, and indexes the run's **placed glyphs** — the glyphs as
  drawn, so a `Repeat`ed run counts every copy and a combining mark
  counts as a glyph. They are counted in text order: a stretch placed
  last to first so it reads forward ([readable path
  text](./schema.md#readable-path-text)) does not reorder them, and a
  frame's text set in two lines counts the first line's glyphs (the
  upper-right half), then the second's, without the whitespace they
  part at. A range is clamped to the glyph count: one that
  runs past the end writes up to it, and one that starts past the end,
  runs backward or is empty writes nothing. A `Once` filler's glyphs
  and a frame's [sides](./schema.md#sides) ornament are skipped by
  every range, since they belong to no span. A
  run's hit-test box does not move with its overrides.

On a predicate, two element-level fields select these elements:
`GfxElementField::ElementType` names an element's variant, and
`GfxElementField::Opacity` compares the opacity of a run.

```json
[
  {"ElementType": "GlyphRun"},
  {"Opacity": 0.5}
]
```

`ElementType` is tested with `Equals` (`Equals(false)` is `==`,
`Equals(true)` is `!=`); the variants are `GlyphArea`, `GlyphModel`,
`GlyphRun` and `Void`. `Opacity` accepts the ordering comparators and
matches only a run.

A run mutation that reaches an element of another kind changes
nothing and is logged, in every build. A scope such as
`SelfAndDescendants` sends one list to elements of every kind, so
`{"RunOpacity": 0.5}` meets areas, models and voids and leaves them as
they were.

## `mutator` — the MutatorNode AST

The mutator AST mirrors the variants of the engine's `Mutator` plus a
`Repeat` wrapper for "N children at consecutive channels" — all but
one: `Mutator::Structure`, a structural command (insert, remove, move,
split, merge), has no AST node and no JSON spelling. Structure is never
authored as JSON; the engine's serde skips the variant, so a
`"Structure"` object is refused as an unknown variant. See
`lib/baumhard/src/engine/builder/ast.rs` for the type definitions. The
AST is generic over the element type and the mutation payload; for the
glyph trees it is the glyph instantiation,
`MutatorNode<GfxElement, GfxMutation>`, so its predicates name
`GfxElementField`s and its literal mutations and templates are glyph
mutations.

### Routing keys: `kind` and `channel`

Every node names the **key** its mutator routes on in two fields,
`"kind"` and `"channel"`, and **both are required**: a node missing
either is refused at load. A mutator reaches a target element only
when the two keys are equal — kind and channel alike — so one parent
can hold children of several kinds whose channel spaces are
independent. The kind is the element type's: the glyph trees a custom
mutation targets have a single kind, `()`, written **`"kind": null`**,
so for them routing is by channel alone. (A key on its own — an
engine `Mutator`'s `"key"` — is the same pair:
`{"kind": null, "channel": 0}`.)

Kind routing holds wherever a mutator meets an element:

- **Children** are paired by equal key, kind and channel.
- **The root** meets the element the walk starts at — the tree's root,
  or each anchor of an anchored walk such as the selection — by
  **kind** alone; its channel is not compared. At an element of
  another kind the mutator does not apply at all, and nothing below
  that element is reached: a mutator tree's shape is relative to its
  root. Of the selected elements, only those of the root's kind are
  anchors, and an anchor inside another anchor's subtree is dropped
  only for an anchor the root applies at.
- **`RepeatWhile`'s descent** visits only the descendants of the
  repeated mutator's kind, on every channel; a descendant of another
  kind is not tested, not mutated and not descended into.
- `MapChildren` and `SpatialDescend` are the two exemptions: they pair
  by position and by point, and compare no key.

```json
{ "Void": { "kind": null, "channel": 0, "children": [] } }
```

Five variants:

- **`Void { kind, channel, children }`** — structural grouping, no
  mutation. Walks children.
- **`Single { kind, channel, mutation }`** — one mutation at one channel.
  With `"mutation": "None"` it builds as a `Void` on that channel:
  a mutator's "no payload" is structural, and a mutation payload has
  no no-op variant of its own.
- **`Macro { kind, channel, mutations, children }`** — a flat list of
  mutations applied to the matched target. `mutations` is
  `{ "Literal": [...] }` for baked-in values or
  `{ "Runtime": "<label>" }` for runtime-supplied values. `children`
  default to empty; used by the `SelfAndDescendants` scope topology
  (a Macro on the anchor whose child is an `Instruction` walking
  descendants).
- **`Instruction { kind, channel, instruction, children }`** — control
  flow wrapping inner children. Instructions:
  - `RepeatWhileAlwaysTrue` — apply children to every descendant of
    their kind.
  - `RepeatWhile(<Predicate>)` — apply children to every descendant
    of their kind for which the predicate holds, short-circuit once
    it fails. A descendant of another kind is skipped together with
    its subtree, before the predicate is asked.
    Note the predicate genuinely filters: a `RepeatWhile` whose
    predicate is the bare `{ "fields": [] }` shape (no fields,
    `always_match` absent/false) matches **nothing**. Today's
    flat-apply path has no predicate evaluator, so it declines any
    `RepeatWhile` that isn't `always_match` and warns instead of
    applying — the alternative would be landing the payload on the
    whole scope set, the inverse of what the AST says.
    Extraction is **all-or-nothing**: one such `RepeatWhile`
    *anywhere* in the AST declines the whole mutator, nested just as
    much as at the root. Honoring a root while dropping a nested
    branch would be the worse failure of the two — the root's payload
    would blanket every target, the nested payload would land
    nowhere, and nothing would warn.

    The same argument decides shapes with **no** unevaluatable node
    in them. `Macro{[L1], children: [Macro{[L2]}]}` is extractable
    top to bottom and still declines, because one flat list cannot be
    both `L1` and `L2` — picking `L1` lands `L2` nowhere, which is the
    identical failure by a different road. So the rule is: every
    payload anywhere in the AST must **agree with** the one
    extracted. `scope::self_and_descendants` satisfies it exactly
    (its root and nested `Macro` carry two clones of the same list);
    nesting *differing* payloads is declined, not merged.
    Concatenating them instead would turn that helper into a
    double-apply.

    "Agree" is *would write the same thing*, which is deliberately
    not `==` on the payload: the numeric fields are `f32`, so `==`
    is not reflexive and a `NaN` would make the duplicated payload
    of `scope::self_and_descendants` disagree with **itself** and
    decline, while the same payload under `scope::self_only` — with
    nothing to compare — applied. Two payloads agree when they are
    `==` **or** structurally identical, so `NaN` agrees with itself
    and `0.0` still agrees with `-0.0`. `NaN` does not agree with
    `inf`.

    Two corollaries of "every payload must agree":

    - An `Instruction` wrapper carrying its own `mutation` — a
      `Runtime` hole or a per-cell `Template` — is
      declined. Both are payloads the flat path provably cannot
      evaluate. The scope helpers all set `"mutation": "None"`.
    - A `Void` **on channel 0** is transparent, since it carries no
      mutation of its own: an empty one cannot lose anything and does
      not decline, while one wrapping a disagreeing payload still
      surfaces the disagreement. Off channel 0 it declines — a
      `Void`'s channel is branch routing (the walker aligns its
      children only against the target child on that channel), and
      the flat path produces one list applied to whole elements with
      nothing to route on. All the scope helpers build on channel 0.
      `Single` declines in every form, including
      `"mutation": "None"`: it is a leaf whose `channel` selects the
      target it writes, so admitting it would widen the flat path
      into precisely the routing it cannot honor.
  - `SpatialDescend(<OrderedVec2>)` — find the smallest descendant
    whose own box holds the point (the first in pre-order on a tie)
    and deliver the instruction's attached mutation to it. Only for
    an element type with geometry: glyph trees have it, and the
    builder refuses it for any other (see "Build errors" below).
  - `RotateWhile` is not part of V1; its design is V2 sub-issue #265.
  - `MapChildren` — **zip-by-sibling-position**. Pairs the
    mutator's direct children with the target's direct children
    by index, **bypassing key alignment**. See "MapChildren"
    below.
- **`Repeat { kind, section, channel_base, count, skip_indices, template }`**
  — expands at build time into N children keyed
  `(kind, channel_base + i)`, each derived from `template`, whose own
  `kind` must be the `Repeat`'s. `count` is `{ "Literal": N }` or
  `{ "Runtime": "<label>" }`. Used by widgets (the picker's hue
  ring) and any runtime-shaped section.

### Mutation sources

A `Single`'s or an `Instruction`'s `mutation` is one of three
spellings:

- `{ "Template": [<CellField>, …] }` — a per-iteration template,
  resolved for each `Repeat` iteration by the build context into one
  `AreaDelta`: a bare slot (`"Text"`, `"position"`, `"bounds"`,
  `"scale"`, `"line_height"`, `"ColorFontRegions"`, `"Outline"`) is
  pulled from that iteration's cell, a tagged one
  (`{ "Operation": "Assign" }`) is baked in. Only valid inside a
  `Repeat` template.
- `{ "Runtime": "<label>" }` — the build context supplies the
  mutation under that label, the same keying a `Macro`'s
  `{ "Runtime": "<label>" }` list uses.
- `"None"` — no mutation (the default on an `Instruction`).

### Build errors

Building an AST into a mutator tree refuses, rather than panics on,
every shape no mutator tree can express — the AST is untrusted map
content and the build runs on an interactive path:

- a `Repeat` at the root (it describes a parent's children);
- `"channel": "SectionIndex"` outside a `Repeat` template;
- a `Template` mutation outside a `Repeat` template;
- a `Repeat` whose `template` is itself a `Repeat`;
- a `Repeat` whose `template` names another `kind`;
- `SpatialDescend` for an element type with no geometry.

### Key alignment vs. `MapChildren`

By default, the walker pairs a mutator's direct children with a
target's direct children **by matching keys** — the `kind` and
`channel` of each — see [Routing keys](#routing-keys-kind-and-channel). This is broadcast
semantics: one mutator on channel N hits every target child that
happens to share its key. It's the right default for groups. A
mutator's children are kept in key order whatever order the JSON
declares them in, children with an equal key in declaration order;
when two of them share a key, the first reaches that key's targets
and the second reaches none.

`Instruction::MapChildren` is the opt-in alternative: it pairs
strictly by **sibling position** (zip), ignoring keys entirely.
This is the shape size-aware layouts want — the `i`-th target child
gets the `i`-th mutator child, regardless of how channels are
assigned. A typical declarative layout pairs `MapChildren` with a
`Repeat` expansion fed by a `BuildContext`:

```json
{
  "Instruction": {
    "kind": null,
    "channel": 0,
    "instruction": "MapChildren",
    "children": [{
      "Repeat": {
        "kind": null,
        "section": "children",
        "channel_base": 0,
        "count": { "Runtime": "children" },
        "template": {
          "Single": {
            "kind": null,
            "channel": "SectionIndex",
            "mutation": { "Template": ["position"] }
          }
        }
      }
    }]
  }
}
```

At build time the registered `BuildContext` supplies
`count("children") = N` and resolves the template once per index —
`resolve("children", i, ["position"])` — into that child's
`AreaDelta`; the builder expands the `Repeat` into N `Single`
mutators and, at apply time, `MapChildren` zips them against the N
target children.

### Runtime holes

A MutatorNode may embed runtime variants (`Runtime` mutations and
lists, runtime counts, and `Template`s resolved per iteration). At
build time the builder consults a `BuildContext` registered by the
host application for the mutation's `id`. Pure-data mutations (no
runtime holes) use a context that supplies nothing.

## The document's kinds and predicate fields

The document (`lib/baumhard/src/document/`, V1-B1) is the engine's
second element type. Nothing authors mutators against it yet: its
mutation vocabulary (`document::mutation`, V1-B3) is built by code and
has no JSON spelling in V1, and its edits and presentations are
V1-D1's and V1-D2's. The two spellings a document mutator will carry
are fixed here, because they are serialized by the types themselves.

**Kinds.** A document mutator's `"kind"` is one of, in sibling order:
`"map"`, `"theme"`, `"palette"`, `"defaults"`, `"section"`, `"span"`,
`"frame"`, `"ring"`, `"connector"`, `"cap_start"`, `"cap_end"`,
`"label"`, `"link"`, `"portal_end"`, `"node"`. Unlike the glyph trees'
`null`, the kind routes: a mutator for `"section"` channel 0 never
lands on `"node"` channel 0. A node, a section and a ring store their
channel; a portal end's channel is its end (0 the source, 1 the
target); every other kind is on channel 0. The root of a document is
its `"map"`, so a mutator applied from the root has a `"map"` root.

**Predicate fields** (`DocField`) are written `{"<field>": <value>}`
in `snake_case`, beside the same comparators as above:

| Field | Value | On |
|---|---|---|
| `kind` | a kind | every element |
| `background` | a color, never a palette color | map |
| `channel` | integer | every element |
| `ident` | integer | node |
| `position_x`, `position_y` | number | node |
| `min_width`, `min_height` | number | node, section |
| `shape` | `"ellipse"` or `{"rectangle": {"radius": 0–100}}` | node |
| `opacity` | number | node |
| `palette`, `on_click`, `on_hover` | integer (an index) | node |
| `folded`, `hidden` | boolean | node |
| `zoom_min`, `zoom_max` | number | node, connector, link |
| `offset_x`, `offset_y` | number | section |
| `align` | `"left"`, `"center"`, `"right"` | section |
| `font` | a compiled-in face, as `AppFont` spells it | node, section, span, frame, ring, cap, label |
| `font_size` | `{"pt": n}` or `{"rel": n}` | those, plus connector, link and portal end (points) |
| `color` | a color | those, plus connector, link and portal end |
| `weight` | 100–900 | node, section, span, frame, ring, cap, label |
| `italic` | boolean | node, section, span, frame, ring, cap, label |
| `visible` | boolean | frame, connector, link |
| `start_edge` | `"top"`, `"right"`, `"bottom"`, `"left"` | frame |
| `start_at`, `gap`, `inset` | number | frame |
| `direction` | `"clockwise"`, `"counter_clockwise"` | frame |
| `fit` | `"repeat"`, `"spread"`, `{"once": {"filler": …}}`, `{"fit": {"min": n, "max": n}}` | frame, ring, connector, link |
| `spacing` | number | frame, ring, connector, link |
| `anchor_from`, `anchor_to` | `"auto"`, `"top"`, `"right"`, `"bottom"`, `"left"` | connector, link |
| `link_kind` | `"line"`, `"portal"` | link |
| `label_t` | number | label |
| `offset` | number | label, portal end |
| `border_t` | number | portal end |

A color is `{"rgba": [r, g, b, a]}`, `{"var": <variable index>}` or
`{"palette": "frame" | "text" | "title"}`.

**This is the spelling of the in-memory `DocField`, and it is not what a
`.mandala.json` holds.** The values here are indices (`{"var": 1}`,
`"palette"`, `"on_click"`), and an index is assigned by sorted name, so
it would name another entry the moment a name was added. A predicate
saved in a map, in an edit (V1-D1) or a presentation, is spelled with the
file's own values — colors as `"#rrggbb"`, `"var(name)"` or
`"palette(frame)"`, a font by family name, a verb by id, a palette by
name, a fit, shape or start point as `format/schema.md` writes them —
and is resolved against the open document when it runs. The file format's
types hold no index (`document::tests::format_strict_tests` keeps
`DocField` and every index type out of the types the loader reads).

A predicate tests what an element **stores**, not what it resolves
to: a node with no `color` of its own has none here, whatever its
palette or the map's defaults paint it. A field the element's kind
does not carry answers nothing and the predicate moves on to its next
pair. On an unset optional field `Exists` fails, `Equals(false)` fails
and `Equals(true)` passes; `GreaterThan` and `LessThan` order numbers
— and font sizes of the same unit — and fail on everything else,
negated or not, as they do on an unset field.

## Related

- `lib/baumhard/src/gfx_structs/area_mutators.rs` (`GlyphAreaCommand`),
  `lib/baumhard/src/gfx_structs/mutator.rs` (`GfxMutation`) and
  `lib/baumhard/src/engine/builder/ast.rs` (the AST) — the types.
- `CODE_CONVENTIONS.md §1` and `§7` — why the mutation framework lives
  in Baumhard and why its seams are preserved.
