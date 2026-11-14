# Mandala & Baumhard — Conceptual Building Blocks

*A reference for the named concepts that make up this project.*

---

## On this document

Mandala is a mindmap application; Baumhard is the glyph-animation
library it is built on. They are one project
([`CODE_CONVENTIONS.md §1`](./CODE_CONVENTIONS.md)). Together they
have accumulated a vocabulary — `GlyphArea`, `MutatorTree`, `Channel`,
`Portal`, `ZoomVisibility`, `Projector`, `GlyphLayers`,
and so on — that sits deliberately across the thin line between *user*
and *developer*. The project aims to expose as much power to end
users as the architecture will carry, so even a curious non-programmer
benefits from knowing what the pieces are and how they fit.

This document names every load-bearing concept, says what problem it
solves, and shows where to reach for it. It is **not** a tutorial,
**not** a schema spec (see [`format/`](./format/) for that), and
**not** a set of prescriptions (see
[`CODE_CONVENTIONS.md`](./CODE_CONVENTIONS.md) and
[`lib/baumhard/CONVENTIONS.md`](./lib/baumhard/CONVENTIONS.md) for
those). It is a *reference*: one place to ctrl-F when a term is
unfamiliar, one place to browse when getting oriented, one place to
point a new contributor at.

The codebase is young and its ambitions are wide. Much of what is
here is a foundation for more. Where a concept has a seam that is
wider than strictly needed today, that is usually because a *named
trajectory* is expected to attach there later — plugins, a Baumhard
script API, richer animations, complex file exports. The "extra
ceiling height" is the point, not the accident
([`CODE_CONVENTIONS.md §7`](./CODE_CONVENTIONS.md)). Entries flag
these seams explicitly.

Each entry uses bold labels in this order: **Summary** (one
sentence), **What it's for** (the problem it solves), **Under
the hood** (file references as a path plus the item to search for —
`path/to/file.rs` and a `fn` or `struct` name, never a line number,
which rots silently while still reading as authority), and where
useful **Vision** (named trajectory) and
**Caveat** (gotchas).

## Table of contents

- [§1 Project foundations](#1-project-foundations)
- [§2 The Baumhard foundation](#2-the-baumhard-foundation)
- [§3 The document domain](#3-the-document-domain)
- [§4 The mutation framework](#4-the-mutation-framework)
- [§5 The application runtime](#5-the-application-runtime)
- [§6 The authoring surface](#6-the-authoring-surface)
---

## §1 Project foundations

Eight cross-cutting stances shape almost every concept below. None of
them are invented here — the canonical statements live in
[`CODE_CONVENTIONS.md`](./CODE_CONVENTIONS.md) and
[`lib/baumhard/CONVENTIONS.md`](./lib/baumhard/CONVENTIONS.md) — but
they are named here so the rest of this document makes sense without
detour.

### Mandala and Baumhard are one project

Baumhard is not a dependency we use; it is a foundation we build.
Both crates are ours. When a feature needs a primitive Baumhard does
not yet have, the primitive is added to Baumhard rather than worked
around in the app. See [`CODE_CONVENTIONS.md §1`](./CODE_CONVENTIONS.md).

### Mutation-first

Any data-model change is a **mutator** applied to a **tree**, never
clone-edit-reinsert. See
[`lib/baumhard/CONVENTIONS.md §B2`](./lib/baumhard/CONVENTIONS.md).

### Everything is glyphs

Text, borders, connection lines, portal markers, console chrome,
selection highlights — every visual element is a positioned font
glyph. There are no rectangle-shader UIs, no bitmap sprites, no
icon atlases. Introducing a new visual is a question of "what glyph
goes where", not "add a new pipeline".

### Single-threaded event loop

`Application` owns the `Renderer` directly. No channels, no worker
threads, no `tokio`, no `std::thread::spawn` in any interactive
path. The one sanctioned exception running today is the native
[`FreezeWatchdog`](#freezewatchdog) thread, which only *reads* an
`AtomicU64` ping; CODE_CONVENTIONS §3 additionally sanctions the
[IPC boundary threads](#ipc-server-ipcserver), which run only under
`--ipc`, move protocol values and never touch app state (on Windows,
with one tokio runtime worker driving the named pipe's I/O beneath
them). Lock
scopes stay trivial because of this.

### Model / view separation

The [`MindMapDocument`](#mindmapdocument) owns the data; the
[`Renderer`](#renderer) owns GPU resources. The renderer reads
intermediate representations from the document each frame; it
never reaches into the document. The document never holds GPU
handles.

### Cross-platform as first-class

Native desktop and the browser are equal deployments. Full
prescriptive rules: [`CODE_CONVENTIONS.md §4`](./CODE_CONVENTIONS.md).
Live parity status: [`CLAUDE.md`](./CLAUDE.md) "Dual-target status".

### Canonical or exemplary

The bar for every merged change is *canonical* or *exemplary*. See
[`CODE_CONVENTIONS.md §0`](./CODE_CONVENTIONS.md). "Not caused by
my changes" is not an excuse — if you notice a gap, you own the
close.

### Preserved seams

A **seam** is a point where a future extension can attach without
rewriting what surrounds it. Seams are named throughout this
document; "extra ceiling height" is deliberate. See
[`CODE_CONVENTIONS.md §7`](./CODE_CONVENTIONS.md).

---

## §2 The Baumhard foundation

Baumhard is the glyph-animation library under
[`lib/baumhard/`](./lib/baumhard/). It is where most of the
conceptual vocabulary of the project originates. The mindmap layer
(§3) and the application layer (§5) reach into it constantly; most
of their own concepts are compositions of the primitives below.

For the prescriptive rules of the crate — the mutation-first
discipline, the arena invariants, the unsafe policy, the benchmark
obligations — see
[`lib/baumhard/CONVENTIONS.md`](./lib/baumhard/CONVENTIONS.md).
This section is conceptual.

### The engine — `Tree<E>`, `MutatorTree<E, M>` and their element types

Baumhard's mutation engine, generic over **the element type** and
**the mutation payload**, so the same walker can mutate any tree of
any element type that implements its traits.

The engine lives in `lib/baumhard/src/engine/` and knows nothing
about glyphs. An element type implements `TreeElement` — the kind and
channel the walker routes on, which together are the element's
`BranchKey` (see [`Channel`](#channel)), a `void()` placeholder, the
state it hands its children (`inherit`, see "Inherited state" below),
and, for an element type with geometry, its own box
(`spatial_bounds`). A payload implements
`Mutation<E>`: one `apply_to(target, inherited)`, plus the shape of
its builder template. The glyph trees are the first instantiation —
`GfxElement` with the `GfxMutation` payload — the document is the
second ([below](#the-document--document-docelement-dockind)), and
`util::engine_isolation`'s scan holds the engine to importing nothing
from `gfx_structs`, `mindmap` or `font`.

### `Arena<T>` and `NodeId`

The storage every tree in Baumhard is made of: nodes in one `Vec`,
linked to their parent, first and last child and previous and next
sibling, and named by a `NodeId`.

A tree that is edited in place needs ids that survive the edit, links
that make every one-step move O(1), and a way to tell an id whose node
is gone. A `NodeId` is a slot and the slot's **generation**: removing
a node frees the slot and moves it to its next generation, so the old
id reads nothing (`Arena::get` answers `None`, indexing with it
panics naming it) even once another node takes the slot, and a slot at
its last generation is retired rather than reused — no id can ever
alias another node. Every walk iterates — `children` (both ways),
`ancestors`, `descendants`, `traverse` — and `Clone`, `PartialEq`,
`Debug` and `Drop` of an arena are those of its flat `Vec`, so depth
costs none of them call stack (§20 rule 11).

The links are compact (V1-A7, #301): a link is a bare 4-byte slot
number, which the arena wrote and which is live by construction, so it
carries no generation and is never checked against one. A generation
is checked where an id **enters** the arena — `get`, `node`, `at`,
indexing, `get_mut`, and each `NodeId` method's `self` and arguments —
once per id per call; a walk checks its start and then follows links.
For the same reason a node's links are read through its id
(`id.parent(arena)`) or through a `NodeRef` (`arena.node(id)`,
`arena.at(id)`), which hand each link back as a `NodeId` with its
slot's generation, never from a bare `Node`. A `NodeId` is 8 bytes on
every target, a slot of a document element 24 bytes beyond the
element, and an arena holds at most `MAX_SLOTS` (`u32::MAX - 1`)
slots; `arena::tests::layout_tests` pins the sizes on both targets.
No structural operation allocates (`tests/arena_alloc.rs`), and the
loader makes the arena's room for every node before it builds.

Structure changes through `NodeId`: `detach`, `remove_subtree`, the
O(1) value inserts that create a node (`append_value`,
`prepend_value`, `insert_after_value`, `insert_before_value`), and two
ways to move an existing one. Under **another parent** it goes through
a checked insert (`checked_append`, `checked_prepend`,
`checked_insert_after`, `checked_insert_before`), which refuses a cycle
by climbing from the destination to the top — O(depth), once per
moved subtree. Back into **the row it was detached from** it goes
through the crate-private `reattach_in_row`, which cannot close a cycle
and so climbs nothing: O(1) (ruling R16 on #282). The engine calls it
from one function, `engine::siblings::reattach_in_row`, which
`test_the_same_row_reattach_is_the_unchecked_insert` holds to it.

Baumhard's own (V1-A6, #290), replacing the `indextree` (gone) crate,
whose every insert of an existing node made the climb.
`lib/baumhard/src/arena/`.

### `Tree<E>`

An arena-backed forest of typed nodes plus the element type's
per-tree derived state, representing one layer of content.

A `Tree` is how Baumhard stores anything
hierarchical that needs to render or be hit-tested: the document,
each of the five glyph layers, the console overlay, and so on. Every node in a tree is reached
through an opaque `NodeId` — not a pointer, not an index — so the
tree can be rearranged, cached, or serialized without invalidating
references. Nodes are `Clone`, mutation is in-place (never
rebuild-the-arena).

Every parent's children are kept in ascending `BranchKey` order —
kind, then channel — with equal keys in the order they were placed
in: the **sorted-children invariant**, which is what lets the walker
merge sibling rows without sorting them. The arena is private to the
engine, so the invariant is kept by the only ways to change a tree:
`insert_child(parent, element, Placement)` places a new child in the
run of its key (`First`, `Last`, or `Before` / `After` an equal-key
sibling), `update(id, f)` — the one way to change an element outside
a walk — moves an element whose key `f` changed, `remove_subtree`
removes (freeing, for building trees), a walk moves the elements whose
key it changed once it is over, and its
[structural commands](#structural-commands--structop-subtree) place
every node they insert or move within the run of its key.
`check_sorted` verifies it. `update_journaled(id, journal, f)`
is `update` recorded in a [journal](#journals--journalbuilder-journal-touch).
`contains(id)` says whether a node is in the tree — under the root —
rather than merely in the arena, which also holds the subtrees a
structural command detached and a journal still owns; `update`,
`insert_child` and `remove_subtree` refuse those. It is O(1): the
tree keeps the set of its detached nodes, which only the engine writes
— where a subtree leaves or re-enters the tree, per element of it —
so a caller can ask it, or update, once per element at no cost that
grows with depth (§20 rule 11). The root is read through `root()`;
the field is private to `engine/tree.rs`, so the compiler holds every
other file to reading it — a root moved behind the engine's back would
leave `contains` and every journal describing another tree.

Defined in `lib/baumhard/src/engine/tree.rs`. Wraps an
[`Arena<E>`](#arenat-and-nodeid), read through `arena()`; adds the root,
`layer: usize`, and `cache: E::Cache` — the element type's
`TreeCache`, which the walker and every one of those methods
invalidate. The glyph tree,
`GfxTree = Tree<GfxElement>`, keeps its AABB memo
(`Cell<Option<...>>` because the values are `Copy`), its subtree-AABB
dirty flag, and an optional `RegionParams` + `RegionIndexer` seam
there (`GlyphTreeCache`, `lib/baumhard/src/gfx_structs/tree.rs`),
and only glyph trees answer the spatial queries (`descendant_at`,
`descendant_near`, `descendants_aabb`). The blessed iteration
primitives are `NodeId::children(tree.arena())` and
`descendants(tree.arena())`; collecting into a `Vec<NodeId>` is a
code smell. Every change leaves the cache invalidated, so no caller
invalidates it by hand.

### `MutatorTree<E, M>`

The mutation-side mirror of a `Tree`: same shape,
carrying `Mutator`s instead of elements.

If `Tree` is the *noun*, `MutatorTree` is the
*verb*. A `GfxMutatorTree` describes a change to apply to a
`GfxTree`: "mutate the third child's text, shrink the font on every
descendant of channel 2, repeat until the predicate fails." The
walker pairs the two up by channel (or sibling position, depending on
the instruction), applies matching mutations in place, and leaves the
rest alone. This is the seam the mutation framework
([§4](#4-the-mutation-framework)) rides on.

Also in `lib/baumhard/src/engine/tree.rs`. Minimal — an
`Arena<Mutator<E, M>>` and a `root: NodeId`, its children kept in key
order like a `Tree`'s and added the same way (`insert_child`). No
derived state: mutators are pure deltas, they do not render. The root
of an empty mutator tree is a channel-0 `Mutator::Void`, which pads
the mutator's shape to match the target's when keys do not line up.

Two entry points, both over the walker. `apply_at(&mut target,
&anchors, seed_for)` walks from **anchors** — typically the
selection: `Anchors::Nodes`, `ParentOf` (the distinct parents) or
`SiblingsOf` (each node's same-kind siblings). The anchors resolve to
a de-duplicated set, of which only the nodes of the mutator root's
**kind** stay anchors (channels not compared): a mutator's shape is
relative to its root, so at a node of another kind it applies nothing,
there or below. Among those, when the mutator reaches below its root,
an anchor inside another anchor's subtree is dropped, so a subtree
mutation reaches each element once; the rest are walked in tree order.
Every anchor's seed is taken before any anchor is walked, and a stale
anchor refuses the whole call with nothing applied. The resolution
costs work in the anchors, their rows and the paths above them, never
in the rest of the tree: checking and ordering climb each element
above an anchor once over all of them, through a memo, so a selection
of every element of a deep chain costs the chain once, not once per
element (§20 rule 11). `apply_to(&mut target, &seed)` is `apply_at` anchored at the
root. `apply_at_journaled(&mut target, &anchors, seed_for, &mut
journal)` is `apply_at` recorded in a
[journal](#journals--journalbuilder-journal-touch), for a tree that
keeps history; the plain entry points are for trees that need none
(glyph layers that are rebuilt). A `Mutator::Structure` node carries a
[structural command](#structural-commands--structop-subtree), which
either entry point runs after its walk.

**Inherited state.** Some mutations need a value their element
inherits rather than stores. The walker carries `E::Inherited` down
every walk: the seed is the state of the walk root's parent, a mutation
receives its target's *parent's* state, and each element hands its
children `inherit(parent's state)` computed after its own mutations
in the same walk. The state lives on a heap stack beside the walk's
frames — never on an element, never in a tree, never re-derived by
climbing to the root, no allocation per visited node. The glyph trees
inherit nothing (`Inherited = ()`, seed `&()`).

### `Applicable<T>`

A one-method dispatch trait: "apply this delta to that
value".

Every field-level mutation primitive in Baumhard implements
`Applicable` against its target type:
`DeltaGlyphArea: Applicable<GlyphArea>`,
`DeltaGlyphModel: Applicable<GlyphModel>`,
`GlyphAreaCommand: Applicable<GlyphArea>`, and so on. The shape is
always `fn apply_to(&self, target: &mut T)`. This keeps the
vocabulary uniform: to learn how a new delta works, look at its
`apply_to` and nothing else. The element-level counterpart is the
engine's `Mutation<E>` (above): `GfxMutation` routes each payload to
one of these.

Defined in `lib/baumhard/src/core/primitives.rs`.
The trait is deliberately minimal; no associated types, no `Result`.
Interactive paths cannot panic
([`CODE_CONVENTIONS.md §9`](./CODE_CONVENTIONS.md)), so type
mismatches (e.g. applying a `ModelDelta` to a `GlyphArea` target)
are silently ignored by design — the dispatch site is responsible
for well-typed pairing. This tradeoff is ugly but correct for a
real-time editor: the cost of a dropped mutation is a visual
glitch on one frame; the cost of a panic is a lost document.

### `ApplyOperation`

The operation selector a delta carries — `Add`,
`Assign`, `Subtract`, `Multiply`, `Delete`, or `Noop`.

A single `DeltaGlyphArea` does not hardcode
"text replace" vs. "text append"; it carries an `ApplyOperation`
that tells the generic `apply` helper which trait assignment to
use. That is how one `Text(String)` delta variant covers both
"concatenate this suffix" (`Add`) and "replace the whole text"
(`Assign`) without duplicating the variant.

`lib/baumhard/src/core/primitives.rs`. The
generic apply requires the target type implement `AddAssign`,
`SubAssign`, `MulAssign`, and `Default` — which is why every
mutable field type in the delta world carries all four.

### `GfxElement`

The tagged union every `Tree<GfxElement, _>` node is
— a `GlyphArea`, a `GlyphModel`, a `GlyphRun`, or a `Void`.

This is *the* tree-node type in the codebase.
All visual things — every text region, every composed-glyph
shape, every shaped TextFlow, every structural padding node — is one variant of this
enum. Shared metadata rides on every variant: a `channel` for
mutation routing, a `unique_id` assigned by the host app (Mandala
uses it for the mindmap node id), a `flags` set, and an
`event_subscribers` list. What a glyph tree derives from its
elements — the AABB memo and each node's subtree AABB for the BVH
hit-test — lives in the tree's `GlyphTreeCache`, keyed by `NodeId`,
never on an element.

`lib/baumhard/src/gfx_structs/element.rs`.
`GlyphArea`, `GlyphModel` and `GlyphRun` each box their
payload (one heap allocation per element of that kind); `Void` has no
payload.
There is a companion `GfxElementType` enum for cheap variant
checks without destructuring, and a `GfxElementField` enum used
by predicates and field-level mutations to name "which part of
which variant". `GfxElementType` — like every other `*Type` tag in
the pipeline (`MutatorType`, `MutationType`, `GlyphAreaFieldType`,
`GlyphModelFieldType`, `GlyphAreaCommandType`,
`GlyphModelCommandType`) — is *derived* from its enum via strum's
`EnumDiscriminants`, so a tag can never drift from the variants it
names. All of them are read through one trait,
`core::primitives::Discriminated`, which supplies `variant()` and
`same_type()`.

### `GlyphArea`

A text region — the only element that actually draws
glyphs to the screen.

When something visible has characters in it, a
`GlyphArea` represents it: mindmap node text, connection glyphs,
portal icons, console lines, FPS overlay digits. The struct
carries everything the renderer needs to shape and draw that text
— position, render bounds, font scale and line-height, per-span
color/font overrides ([`ColorFontRegions`](#colorfontregions)), an
optional outline halo, a hit shape, and a zoom-visibility window. It
draws its text and nothing behind it: a map's one background is the
frame's clear color, and every other color comes from a glyph
(V1-B12, design D12).

`lib/baumhard/src/gfx_structs/area.rs`. Uses
`OrderedFloat<f32>` and `OrderedVec2` for its numeric fields so
the struct is `Eq + Hash` despite holding floats — important for
caching and identity-based diffing. The `hitbox` field is the one
exception to the hash/eq contract: it is derived by the scene
builder from the rest of the fields, not part of identity. One
`GlyphArea` maps to one cosmic-text `TextArea` in the renderer.

The `text: String` is edited with grapheme-aware
helpers ([`grapheme_chad`](#utilities--grapheme_chad-color-geometry)); byte
offsets from user-facing counts will land mid-cluster on the
first emoji.

### `GlyphModel`, `GlyphMatrix`, `GlyphLine`, `GlyphComponent`

A four-level composition hierarchy for glyph shapes
built out of small typed cells.

Sometimes a visual element is more structured
than a plain string — a grid, a menu, a composed diagram built of
box-drawing pieces. `GlyphModel` is the answer: it is a child of a
`GlyphArea` that contributes a matrix of lines of components, each
component carrying its own text plus optional font and color
overrides. The model paints its contents *into* the owning
`GlyphArea`'s buffer at shape time, so the whole thing shapes and
renders as one cosmic-text pass while remaining structurally
addressable for mutation.

The hierarchy is: `GlyphModel` owns a
`GlyphMatrix`; `GlyphMatrix` owns a `Vec<GlyphLine>`; `GlyphLine`
owns a `Vec<GlyphComponent>`; `GlyphComponent` is
`{ text, font: AppFont, color: Color }`. All
files in `lib/baumhard/src/gfx_structs/model/`. Matrix/Line both
auto-expand on out-of-range index write, so callers can poke at
arbitrary coordinates without pre-sizing. The central
`GlyphMatrix::place_in` method paints the matrix into the owning
area's `String + ColorFontRegions`, padding with newlines and
spaces so every component lands on the intended grapheme cell.

### `GlyphRun`

One whole TextFlow as one glyph element: a reference to what was
shaped, the placement that positions it, and the per-glyph effects
layered on top. It never splits into one element per glyph.

A box flow (a section, a label) is a `RunShape::Box` — the shaped
box, drawn whole with its top-left at the run's `origin`. A path flow
(a frame ring, a connection, a portal marker) is a `RunShape::Path` —
the shaped run *the placement refers to* (the rescaled or filled run
when placement returned one) plus its `PlacedRun`, one canvas position
per placed glyph. Both hold `Arc`s, so cloning a run for a journal's
before-image copies pointers, and re-placing a moved flow swaps the
`placed` pointer without reshaping. `GlyphOverrides` — an offset, a
color and a scale per **placed** glyph — are what glyph-layer
presentations write, through `RunOverrideOp`; the placed glyph count
is not the shaped glyph count, because `Repeat` and fillers reuse
indices, and a range op skips filler glyphs. The run's `bounds` (its
hit-test box) are computed when the shape or origin is set: the
shaped size at the origin for a box, the union of the glyph boxes for
a path. Overrides do not move them. The bounds find the candidates; a
box run is hit anywhere in them, a path run only on one of its placed
glyphs' boxes (`GlyphRunData::placed_glyph_at`), so a click inside a
ring or in the empty corner of a diagonal connection's box misses it. `zoom` is evaluated at render
time, never by projection.

The run changes through `RunSet` (the whole payload, what projection
writes), `RunOrigin`, `RunOpacity` and `RunOverrides`; interpolation
blends `origin`, `opacity` and the override vectors and snaps the
shaping. State comparison (`same_state`) compares the shaping by
`Arc` identity. Introduced in V1-B6 (#240).

A run in a canvas tree or a glyph layer is drawn by the renderer's
positioned-glyph pass (V1-C1, #242; design §13.2): every glyph of the
run's one shaped buffer at its own placed position — upright, with its
shaper offsets kept, its overrides applied and the run's opacity in its
alpha — from an atlas the pass holds, in one instanced draw call per
frame for every run there is. Nothing is shaped to draw it, and a pan
or a zoom touches no run. The [projector](#projection--projector-glyphlayers-layerchanges-hit)
emits them into the glyph layers (V1-B7), which the
[layer renderer](#layer-renderer) keeps and draws (V1-C2); the app draws
them once C3 wires its projector to it.

`lib/baumhard/src/gfx_structs/run.rs`; the mutations are
`GfxMutation::Run*` in `mutator.rs`; the pass is
`src/application/renderer/glyph_runs/`.

### No fill element

There is none (V1-B12, design D12, replacing V1-B6's `Fill`): a map
has exactly one background, the color every frame is cleared to, and
every other color it draws comes from a glyph — text, frames, rings,
connectors, links, labels, portal ends, ornaments. The document holds
the background as the map's own field (`MapData::background`, a
literal or a theme variable, black when unset), resolved by
`document::resolve::resolve_background`. A mutation naming a fill is
refused as an unknown variant.

### `Void`

A no-op tree node: no payload, no render cost, just
structure.

Sometimes a mutator tree needs a child at index
*k* that does nothing, so subsequent children align against the
right target children. Sometimes a target tree needs a parent that
has no content of its own but holds other elements. `Void` is the
answer in both cases. It is never required — but used tastefully,
it keeps tree shapes regular and channel alignment clean.

`lib/baumhard/src/gfx_structs/element.rs` for
the target side; same enum on the mutator side in `mutator.rs`.
No heap allocation, just metadata (channel, id, flags).

### `ColorFontRegions`

A set of character-range spans, each with optional
color and font overrides, layered over a `GlyphArea`'s text.

A single node's text can have multiple styles —
a bold first word, a red annotation, a smaller footnote. Rather
than fragmenting text into per-style nodes, Baumhard carries
**span tables**: `[start, end)` ranges that say "between these
two positions, use this color and/or this font". What becomes of
text no span covers is the *bridge's* decision, not the primitive's:
`attrs_list_from_regions` leaves it at the `AttrsList` default,
while the renderer's `rich_text_spans_into` draws only the covered
slices (see [text runs](#text-runs)).
The same primitive drives rich-text on mindmap nodes
([`text runs`](#text-runs)), highlight on selected regions, and
transient live-edit previews.

`lib/baumhard/src/core/primitives.rs`. Backed
by `BTreeSet<ColorFontRegion>` keyed on the `Range`, so lookups
by range are `O(log n)` but two regions with the same range and
different payloads collide (last write wins) — this is
deliberate, not a bug. The `Range` indices are **grapheme-cluster
offsets** — the unit baumhard's text primitives speak in (see
`lib/baumhard/CONVENTIONS.md §B1` and the helpers in
`util/grapheme_chad.rs`). Every fresh producer counts via
`count_grapheme_clusters`; the cosmic-text bridges in
`font/attrs.rs` slice through `find_byte_index_of_grapheme`. The
primitive itself just holds `usize` pairs and does not enforce a
unit at the type level, so consumers that reach in from elsewhere
must agree on the grapheme convention. The mutation surface is nine
`&mut self` methods. Four re-index the set under a text edit —
`insert_regions_at`, `shrink_regions_after`, `shift_regions_after`,
`split_and_separate` — and five write regions directly:
`submit_region`, `set_or_insert`, `replace_regions`, `remove`,
`remove_range`. A spatial index
([`RegionIndexer`](#regionparams-regionindexer-regionerror)) can
be layered on top for hit-testing.

Never mutate `ColorFontRegions` outside the mutator
pipeline — direct writes skip the index update and selection
drifts silently. See [`lib/baumhard/CONVENTIONS.md §B6`](./lib/baumhard/CONVENTIONS.md).

### `Range`

A half-open `[start, end)` span of `usize` indices.

The canonical primitive for "some part of the
text" everywhere text appears. `ColorFontRegion` keys on it;
`GlyphAreaCommand::ChangeRegionRange` manipulates one; text-run
schema validation runs over them. Small but load-bearing: a
single shared `Range` type means span operations compose across
modules without glue.

`lib/baumhard/src/core/primitives.rs`. Totally
ordered for `BTreeSet` use; ships with exactly six methods — `new`,
`tup`, `magnitude`, `overlaps`, `to_rust_range` and
`checked_push_right`. The checked shift is the only shift: every
caller moves a range whose bounds came from a document, and the
unchecked predecessor wrapped silently in release, turning a
far-right region into a far-left one.

### `Channel`

An integer routing tag on every node. With the node's kind it forms
the node's `BranchKey`, and the tree walker matches mutator nodes to
target nodes by equal key — kind and channel — within a sibling
group.

Without channels, every mutation applied to a
parent would broadcast to *every* child; with channels, the author
can say "this mutation only hits siblings tagged channel 1". A
parent and its child can share a channel or differ; the matching
is within-sibling only. Siblings on the same channel form a
*broadcast group*: one mutator affects all of them. This is the
primitive that makes a single mutation selective without naming
child indices.

An element type answers `TreeElement::kind` and
`TreeElement::channel`, and so `TreeElement::key`
(`lib/baumhard/src/engine/element.rs`, `engine/key.rs`); every
`Mutator` carries its own `key`. The kind keeps one parent's children
of different kinds apart: a mutator for one kind's channel 0 cannot
land on another kind's channel 0. The glyph trees have a single kind
(`()`), so for them the key is the channel. In the document, a
node's channel is where this surfaces to end users (§3, "Elements").

Every parent's children are stored in key order (the sorted-children
invariant of [`Tree<E>`](#treee)); siblings with an equal key keep the
order they were added in, which for the document is the order its file
writes them. [`MapChildren`](#instruction) pairs strictly
by sibling position instead, for a layout where index matters more
than tag.

### `Flag` / `Flaggable` / `AnchorBox`

A small enum of state markers any node can carry, and
the trait that queries them.

Some per-node state is not *data* in the
rendering sense but *status* — "this node is focused", "this
node is in edit mode", "this node is anchored to a specific
screen corner". Flags provide a uniform place to store those,
queryable by [predicates](#predicate-and-comparator) without
extending the element's data fields.

`lib/baumhard/src/core/primitives.rs`. Current
variants: `Focused`, `Mutable`, `Anchored(AnchorBox)`,
`MutationEvents`, `SectionRoot`. `AnchorBox` holds up to four
`Anchor` entries for layout-solver pinning. `MutationEvents` is
reserved — it marks a node that should fire events on mutation (a
seam for future reactive handlers).

`Anchored` and its `AnchorBox` / `Anchor` / `AnchorPoint` /
`AnchorTarget` payload have no layout solver behind them yet, and
nothing in either crate constructs one. They survive the #41
dead-code sweep on format grounds rather than code grounds:
`Flag` is `Serialize` / `Deserialize` and reachable from authored
JSON through the `GfxElementField::Flag(Flag)`
predicate language, where
[`format/mutations.md`](./format/mutations.md) publishes
`Anchored(AnchorBox)` as an authorable variant. Removing the
variant would change what an authored map round-trips, which is a
format decision and not a cleanup.

### `Event`, `GlyphTreeEvent`, `GlyphTreeEventInstance`, `EventSubscriber`

A *non-state-mutating* kind of mutator: instead of
changing element data, it invokes callbacks subscribed to the
element.

Event-driven behavior (button-like nodes,
hover-response, keyboard dispatch to a focused node) does not
belong in the mutation-first data pipeline — a keystroke is not
a delta to a field. Events reuse the mutator infrastructure for
dispatch but invoke subscriber callbacks instead of editing
data. A subscriber *can* enqueue further mutations as a
reaction, which is how reactive chains are built.

`lib/baumhard/src/gfx_structs/mutator.rs`.
`GlyphTreeEvent` is the enum of event kinds, seven variants:
`KeyboardEvent`, `MouseEvent(MouseEventData)`, `AppEvent`,
`CloseEvent`, `KillEvent`, `MutationEvent`, `NoopEvent(usize)`.
None of them is constructed on a production path in either crate:
`MouseEvent`, `AppEvent`, `CloseEvent`, `KillEvent` and `NoopEvent`
are built only under `lib/baumhard/src/gfx_structs/tests/`, and
`KeyboardEvent` and `MutationEvent` are constructed nowhere at all —
the latter is the mutation-fired kind the `Flag::MutationEvents`
seam is reserved for. `GlyphTreeEventInstance` wraps the kind with a
timestamp;
`EventSubscriber` is
`Rc<RefCell<dyn FnMut(&mut GfxElement, GlyphTreeEventInstance)>>`
(`lib/baumhard/src/gfx_structs/tree.rs`). The `Rc<RefCell<…>>` shape
exists so that cloning an element (as the arena does) keeps a single
callback reachable from every clone rather than duplicating state;
the event loop is single-threaded, so nothing needs `Send`.

Today the mindmap app does not use subscribers
heavily — most interaction goes through the application's own
input handlers. The seam is preserved for the Baumhard script
API and plugin trajectory, where user-authored code will want
to subscribe to events without reaching into the app crate.

### `Predicate` and `Comparator`

A small expression language for "does this element
match?" tests, used by loop and dispatch instructions.

Some mutations only apply to certain nodes —
"every child whose font size is under 12pt", "every descendant
marked `Focused`". A `Predicate` names the fields to test and
the `Comparator` (equals, not-equals, greater-than, etc.) to use
against each; the walker evaluates it per candidate node and
decides whether to recurse.

`lib/baumhard/src/engine/predicate.rs`. `Predicate<E>` is generic:
its fields are the element type's own selector, `E::Field`, and the
element answers each `(field, comparator)` pair through the `Inspect`
trait. The glyph answer — `GfxElementField` over `GlyphArea`,
`GlyphModel`, regions, channel, id and flags — is
`lib/baumhard/src/gfx_structs/inspect.rs`.
Pure data (serializable); typical predicates carry one or two
fields, so evaluation is effectively `O(1)`. Float comparisons
use `almost_equal`, whose epsilon is `ERROR_TOLERANCE_ALMOST_EQUAL`,
declared beside it
([`util/geometry.rs`](#utilities--grapheme_chad-color-geometry)). The
`Comparator` uses a *negation flag* pattern: `Equals(false)` is
`==`, `Equals(true)` is `!=`, halving the variant count.

### `Instruction`

The three control-flow primitives a `Mutator` can
carry: `RepeatWhile`, `SpatialDescend`, and `MapChildren`.

Most mutations are direct: apply this delta to
this node. Some need to loop ("apply this to every descendant
matching predicate X"), some need spatial routing ("apply this
to whichever node contains this point"), some need
position-indexed pairing ("apply these N mutators to these N
siblings, zip-style"). `Instruction` is the vocabulary. This is
how one mutator can sweep a whole subtree without
hand-listing every target.

`lib/baumhard/src/engine/mutator.rs`.
- `RepeatWhile(Predicate)` — iterates children, applies mutator
  children while predicate holds, stops on failure. Aligns by
  key (broadcast semantics), and below that descends into the
  children of the repeated mutator's kind only, on every channel.
- `SpatialDescend(OrderedVec2)` — finds the smallest descendant
  whose own box (`TreeElement::spatial_bounds`) contains the given
  point, applies the mutation there. Bypasses key alignment.
  Only for an element type with geometry — the glyph trees; the
  builder refuses it for any other.
- `MapChildren` — zips mutator children to target children
  **strictly by sibling position**, ignoring keys. The
  right shape for size-aware layouts where index matters more
  than tag.

`RotateWhile` is not part of V1; its design is V2 sub-issue #265.

### `Mutator` and `GfxMutator`

The mutator-side node type: `Single`, `Macro`, `Void`, or
`Instruction` variants.

Every node of a `MutatorTree<E, M>` is a
`Mutator<E, M>`; the glyph trees' is
`GfxMutator = Mutator<GfxElement, GfxMutation>`. The four variants
cover "one field change here",
"a batch of changes on this target", "structural padding", and
"control flow with nested children". Together with
[`Instruction`](#instruction) and
[`Predicate`](#predicate-and-comparator) they form a small but
complete mutation language.

`lib/baumhard/src/engine/mutator.rs`. Every variant carries the
`key: BranchKey` it routes on. A `Macro` carries a `Vec<M>`
applied in order to the same target, plus optional `children` for
descendant instruction nodes. A mutator that carries no payload says
so structurally — a `Void`, or an `Instruction` whose `mutation` is
`None` — so a payload type has no no-op variant.

### `GfxMutation` enum

The glyph payload union: which kind of delta or command
a glyph mutator carries.

A mutation is not one uniform thing — a
`GlyphArea` and a `GlyphModel` accept different kinds of change.
The `GfxMutation` enum is the sum type covering all of them: two
flavors each for area and model (field-level `Delta` vs.
imperative `Command`), plus `Event` (subscriber dispatch) and the
`Run*` mutations that change a `GlyphRun`. It
implements the engine's `Mutation<GfxElement>`; its builder template
is a list of `CellField` slots (`gfx_structs/template.rs`).

`lib/baumhard/src/gfx_structs/mutator.rs`.
Each variant boxes its payload to keep the enum compact. Type
mismatches (e.g. `ModelDelta` applied to a `GlyphArea`) are
silently ignored per the [`Applicable`](#applicablet) no-panic
rule; a run mutation at an element of another kind is ignored and
logged the same way, in every build. `RunSet` is projection-only and
never authored: it is
`#[serde(skip)]` and have no JSON spelling, and since only a
projection bug can land one on the wrong element, that case also trips
a `debug_assert!`.

### `GlyphAreaField` and `DeltaGlyphArea`

The per-field delta surface for `GlyphArea`: text,
scale, position, bounds, regions, outline, shape, zoom
visibility.

This is the granular surface any field-level
mutation reaches into. A font-size change is a
`GlyphAreaField::Scale(…)` inside a `DeltaGlyphArea` with an
`ApplyOperation` — that one pattern scales across every field
without bespoke plumbing per field.

`lib/baumhard/src/gfx_structs/area_fields.rs`
for the field enum and `OutlineStyle`; `area_mutators.rs` for
`DeltaGlyphArea`. The wrapper carries one `ApplyOperation`
shared across all fields in the batch, so "move this node 10
units right" and "set this node's text" use the same delta type
with different field lists and a different operation.

### `GlyphModelField`, `DeltaGlyphModel`, `GlyphModelCommand`

The `GlyphModel` mutation surface — the parallel of
the area-side delta and command trio, applied to composed-glyph
structures rather than plain text.

Everything the area side offers
([`GlyphAreaField`](#glyphareafield-and-deltaglypharea),
[`GlyphAreaCommand`](#glyphareacommand)), the model side needs
too — position nudges, matrix inserts and replacements, color
and font edits on individual components. Same operation vocab
(`ApplyOperation`), same `Applicable` dispatch, same walker
path; different target type.

`lib/baumhard/src/gfx_structs/model/mutator.rs`. `GlyphModelField`
variants cover the structural bits (matrix inserts, component
edits, model position). `DeltaGlyphModel` wraps them with an
`ApplyOperation`. `GlyphModelCommand` is the named-operation
counterpart for things that don't fit arithmetic — row pops,
matrix-coordinate moves, rotations. All three ride in the
`GfxMutation::ModelDelta` / `GfxMutation::ModelCommand` variants of
the [glyph payload](#gfxmutation-enum).

### `GlyphAreaCommand`

The *named-operation* mutation surface, for actions
that are not arithmetic deltas.

Some operations have fixed semantics that
don't map to "add/subtract/assign": *pop the last three
graphemes*, *change the range of this region*, *delete a
specific region*. Commands are the vocabulary for those.
Imperatively named, grapheme-aware; nineteen variants today.

`lib/baumhard/src/gfx_structs/area_mutators.rs`.
All grapheme-touching commands use `grapheme_chad` helpers, so
emoji / ZWJ / combining-mark sequences survive intact.

### `OutlineStyle`

A colored halo behind text, rendered as eight stamp
copies (four cardinals + four diagonals) around the main glyph.

When glyphs sit on a busy background, legibility
drops. `OutlineStyle` draws an outline halo so the glyphs read.
It is a field on `GlyphArea`, optional; default is no outline.

`lib/baumhard/src/gfx_structs/area_fields.rs`.
Two fields: `color: [u8; 4]` and `px: f32`. Cost is **9×** the
cosmic-text shapings of the area (one main + eight stamps). Hot
path, so enable only when background legibility demands it.

### `NodeShape`

A pluggable hit-test shape — `Rectangle` or `Ellipse` — the
silhouette a node's body occupies for the projector's hit test.

A node's clickable silhouette and its document shape must agree. The
projection keeps one `NodeShape` per visible node, taken from the
node's typed `Shape` (`document::values::Shape`), and asks it whether
a point or a rubber-band rectangle lands on the node. Nothing draws
the shape itself: a node draws no fill (V1-B12), and its frame is text
along the outline (`flow`). Adding a shape is one enum variant and the
match arms the compiler then demands: `contains_local` and
`intersects_local_aabb` are exhaustive `match`es over `NodeShape`, so
a planted variant does not compile until it has both.

`lib/baumhard/src/gfx_structs/shape.rs`.
`contains_local` does point-in-AABB or point-in-ellipse
(normalized coordinates, `nx² + ny² ≤ 1`); degenerate bounds
always return `false`. `intersects_local_aabb` supports
rect-select with conservative approximation for ellipses.

### `ZoomVisibility`

An optional inclusive `[min, max]` camera-zoom
window that gates whether an element renders at the current
zoom.

Visual detail that makes sense at one zoom
rarely makes sense at another. A legend label is precious when
zoomed in on its region and noise when the whole map is on
screen; an overview landmark is a guide when zoomed out and
redundant up close. `ZoomVisibility` lets authors say "this
appears between 1.5× and 3× zoom" and have the renderer silently
honor it — no script, no mutator, just two fields.

`lib/baumhard/src/gfx_structs/zoom_visibility.rs`.
Two `Option<f32>` fields, a `contains(zoom) -> bool` predicate;
the cost is two float comparisons and nothing else — no
cosmic-text reshaping or buffer-cache
invalidation fires on zoom steps. In the document the surface is a
node's and a path's `zoom` window (`format/schema.md`).

`NaN` zoom is treated as "not visible" deliberately —
a `NaN` camera is a bug upstream, and culling the frame surfaces
it faster than carrying the `NaN` through the glyph pipeline.
Inverted windows (`min > max`) render as "always hidden" at
runtime; `maptool verify` flags these as authoring errors.

The seam waiting here is **zoom-triggered LOD
mutations**: an edit (V1-D1) bound to a zoom threshold could
swap a node's content entirely at the transition, so a cluster
summary becomes a detail view as you zoom in.
`GlyphAreaField::ZoomVisibility` already carries the mutator
target; what remains is the dispatcher that fires mutations on
zoom crossings.

### `Camera2D` and `CameraMutation`

A 2D canvas camera with pan/zoom and an intent-level
mutation vocabulary.

The renderer projects canvas coordinates to
screen pixels through a `Camera2D`; pan and zoom are represented
as `CameraMutation` variants so that one handler can accept
input, animation, and scripted values uniformly. When a gesture
says "pan by 10 pixels" and an animation says "fit-to-bounds with
5% margin", both go through the same apply site.

`lib/baumhard/src/gfx_structs/camera.rs`.
Position in canvas space (the point at the viewport center),
`zoom: f32` clamped between `Camera2D::MIN_ZOOM` and
`Camera2D::MAX_ZOOM`. `CameraMutation` variants: `Pan { screen_delta }`,
`ZoomAt { screen_focus, factor }`, `ZoomCenter { factor }`,
`SetPosition { canvas_pos }`, `SetZoom { factor }`,
`FitToBounds { min, max, padding_fraction }`. Projection
helpers `canvas_to_screen` / `screen_to_canvas` are the only
place coordinate-space conversion lives, and they scale by
`pixels_per_unit()` — `zoom × pixel_ratio`. `pixel_ratio` is 1 for
the live window and the supersampling `scale` for an offscreen
capture: zoom windows and zoom-dependent layout read `zoom` alone,
so a capture at any ratio shows the elements the window shows, at
more pixels. `Camera2D::visibility` is
the only place the cull lives: it answers whether an element with a
given canvas rect and zoom window is drawn, and if not why
(`Visibility::OutsideZoomWindow` before `Offscreen`). The renderer's
per-frame cull asks it, and the IPC `scene` family reports its verdict
verbatim, so "on screen" means one thing.

### `Scene`

A multi-layer compositor: owns many `Tree`s at
different draw-order layers and screen-space offsets.

The console overlay is one tree and the color picker overlay is
another. `Scene` collects them all,
orders them by layer, and provides a single `component_at(point)`
hit-test entry that walks top-to-bottom and returns the first
tree that owns the point. This is the structural seam where the
`AppScene` at the application layer
([§5: scene host](#appscene-and-scene-host)) attaches.

`lib/baumhard/src/gfx_structs/scene.rs`. Uses
`Slab<SceneEntry>` for stable ids across insert/remove; each
entry carries `layer: i32`, `offset: Vec2`, and `visible: bool`.
Hit-test is `O(trees)` at the scene level and `O(tree size)`
inside the matched tree.

### Tree walker — `walk_tree_from`

The dispatch engine that walks a `MutatorTree` against a `Tree` and
applies matched mutations — iteratively, so a tree of any depth walks
in a bounded call stack.

Every mutation that a mutator tree delivers goes through the
walker. The walker aligns children by key (or by position, depending
on instruction): both rows are sorted by key, so it walks the two
sibling lists in step — the lower key advances, equal keys walk — with
no `Vec` and no sort. It dispatches each payload to
`Mutation::apply_to` with the target's parent's
[inherited state](#mutatortreee-m). A target whose key a mutation
changed is recorded and moved to its sorted place when the walk is
over, so no walk sees a half-sorted row. Cost is `O(sum of matching
pairs)` — pruned branches are free.

**Depth costs no call stack** (design D30, §20 rule 11: a map's depth
has no bound). The walk does not recurse: each step of the descent —
a (target, mutator) pair, a row merged by key or zipped by position, a
`RepeatWhile` level and its terminator — is a `Frame` on a heap stack,
and the inherited states the frames hand down sit on a second one. A
frame that reaches a child pushes itself back with its cursors
advanced and the child on top, so the child's whole subtree runs
before the row moves on: the visit, apply and journal order is the
depth-first order a recursive walk has. The stacks are reused across a
walk's anchors and grow only to the depth reached.

`lib/baumhard/src/engine/walker.rs`, generic over the element type
and the payload — no `dyn`, no `Any`, no runtime type checks.
Key items: `walk_tree_from` (the pair entry), `Frame` (one step of
the descent: `Pair`, `Merge` — the key-based pairing — `Zip`
(`MapChildren`), `Repeat` and `RepeatChildren` (a `RepeatWhile` level)
and `Terminator` (the continuation that resumes key alignment after a
`RepeatWhile` exits)), `Driver` (the step of each frame;
`align_child_walks` starts a key merge), and `Walk` (what a walk keeps
beside the two trees: the children whose key it changed, the journal a
journaled walk records into, and its two stacks). Every apply goes
through one function, `apply_recorded`, which is where a journaled
walk records its before-images. Branchless enough that matching-pair
cost dominates.

A re-keyed child is put back into the row it never left through one
function, `siblings::reattach_in_row`; a child alone in its row is not
moved at all. Putting a node back under its own parent cannot close a
cycle, so ruling R16 (#282) requires that insert to cost O(1): it is
the arena's unchecked same-row re-attach (V1-A6, #290), which climbs
nothing, and `test_same_row_rearrangement_is_linear_in_depth` counts
its steps.

### Journals — `JournalBuilder`, `Journal`, `Touch`

An exact record of what a walk changed, which can undo it, redo it,
or show it part-way.

Undo, redo, transactions, animated edits and change notifications
all need to know exactly what an edit did. A journal recorded by the
walker cannot forget a field and cannot disagree with what the walk
did, because nothing but the walker writes it: the walker records an
element's before-image (a clone) and its position immediately before
its first apply within a `JournalBuilder`, and later applies to the
same element record nothing. `Mutation`s know nothing about journals
and cannot opt out. `JournalBuilder::close(&tree)` reads every
recorded element's after-state and drops the ones whose
`TreeElement::same_state` is unchanged and which did not move, so an
empty `Journal` means "nothing changed". `Journal::revert` (newest
entry first) and `Journal::replay` (oldest first) write recorded
values and positions and evaluate nothing — no `Mutation::apply_to`,
no `inherit` — and refuse a tree whose referenced nodes are stale
before writing anything. `touched()` names what changed; `before` /
`after` read one element's two states.

An element whose key changed is moved among its siblings, so a
journal restores positions too: every position it records is the
element's place in the arrangement the builder started from — an
earlier move in the same builder that shifted the element's
neighbors is accounted for when that move happens — and a restore
puts the moved elements back in that arrangement's order. Those
positions are exact only if the tree changes through nothing but
journaled calls on the builder until it closes. A builder is all or
nothing: a journaled call that fails reverts everything the builder
recorded before returning its error, and `JournalBuilder::abandon` is
the same step for a failure its caller detects.

Structural commands record into the same builder and restore the same
way: an inserted node has only a close position, a removed one only a
start position, a re-parented one a position in each parent, and a
restore puts back arrangements, not a history of moves — which is what
lets structure and key changes mix in one builder. `touched()` reads
each node's change from where it is before and after: `Field` (state,
or place in its row), `Moved` (another parent), `Inserted` / `Removed`
(in the tree at one end only — the whole subtree that moved with it,
from its top, in the pre-order of the arrangement it is in, however
many calls built it). `parents(node)` names the parent a node left or
took — the row a removal changed, which the arena no longer links to.
A journal is never cloned into a history: a clone shares the subtrees
it owns.
A journal **owns** the subtrees it holds out of the tree — what it
removed while applied, what it inserted while reverted — and
`Journal::discard` frees them when the journal leaves the history;
nothing else frees them, and a revert never does.

A restore also writes, per element that enters or leaves the tree,
the tree's set of detached nodes behind the O(1) `Tree::contains`,
from the touches it computed at close. Those touches describe the
change between the journal's own two arrangements, so journals
restore in their history's order — undo newest first, redo oldest
first — and every restore checks it, in every build, in O(1): the tree
carries a version stamp that a journaled call moves to a fresh value
and a restore moves to the stamp of the side it writes, and a journal
whose two stamps the tree is at neither of refuses with
`TreeError::OutOfOrder` before writing anything. A journal that closes
empty has one arrangement, so closing it puts the tree back at the
stamp it started from, and dropping it never strands the history. A
structural change made without a journal — `Tree::insert_child`,
`Tree::remove_subtree`, or an unjournaled walk that ran a structural
command — moves the tree to a fresh stamp no journal holds, so it
ends the undo history of the whole tree: every journal recorded before
it is refused afterwards, even one whose nodes it never reached. A
tree that keeps history makes every structural edit through a
journaled call. Changes that keep every node's parent and membership —
an unjournaled `update`, re-key moves included, or a walk without
structural commands — leave the stamp alone.

Depth costs a journal nothing per entry (§20 rule 11): `close` asks,
for each entry, the top of the subtree that entered or left with it,
and those climbs are memoized, so each element above an entry is
climbed through once over the whole close; a restore puts an element
that keeps its parent back through `siblings::reattach_in_row` (the
same-row insert above), and only one that changes parent pays the
O(depth) cycle check of a re-parent.

`lib/baumhard/src/engine/journal.rs`.

### Structural commands — `StructOp`, `Subtree`

A mutator that changes the shape of a tree rather than a field.

Creating, deleting and re-parenting nodes, adding sections and
splitting text spans are edits like any other, so they are mutators
like any other: a `Mutator::Structure { key, op }` reaches its target
by key from the walk's anchors and carries a `StructOp` —
`Insert { subtree, at }` (a `Subtree` of new elements as a child of the
target, at a `Placement` within its key's run), `Remove`, `Move { to,
at }` (re-parent under a concrete `NodeId`; resolving persistent ids to
`NodeId`s is the instantiation's), `Split { at }` and `Merge` (cut an
element in two at its `TreeElement::SplitPoint`, or absorb the next
equal-key sibling — the element says how through `TreeElement::split`
/ `merge`, and by default it cannot; the glyph elements cannot).

The walk applies them in a **second phase**: it collects `(target,
command)` pairs in walk order and changes no structure, so it never
iterates a row a command is changing; the commands then run in that
order, journaled. All or nothing: a failing command (`Cycle`,
`PlacementCrossesKey`, `StaleNode`, `Unsplittable`, `Unmergeable`,
`RemoveRoot`, `TargetRemovedEarlier`, …) undoes the whole call through
the journal, the walk's field changes included. A removed subtree is
**detached, not freed** — the journal owns it, so undo re-attaches the
same `NodeId`s and every id held elsewhere stays valid. A walk without
a journal journals into one of its own when it has a command, and
discards it after, which frees what it removed. Structure is never
authored as JSON: the variant is skipped by serde, and `Subtree` is
not serializable at all.

Depth costs the commands nothing per node (§20 rule 11). Every node a
command creates — an inserted subtree, the second part of a split —
goes in through the arena's value inserts, which climb nothing. A
command knows its target is in the tree because the walk reached it
there; whether an earlier command of the same walk removed it is the
O(1) `Tree::contains`, which reads the tree's set of detached nodes —
written per element where a subtree leaves or re-enters the tree (a
`Remove` or `Merge`, a journal's revert, replay or interpolated write,
from its touches) and cleared by `discard`. The one climb a command
owes is a `Move` to another parent, which checks for a cycle: O(depth),
once per node the user moves — the price of refusing a cycle, not a
cost per visited node (ruling R16 on #282). A `Move` within its own row
owes none, since under its own parent it cannot close a cycle; it goes
through `siblings::reattach_in_row`, the arena's O(1) same-row
re-attach (above). A `Subtree` is as deep as its builder made it, so its
`Drop`, `Clone`, `PartialEq` and `Debug` are written by hand and
iterate.

`lib/baumhard/src/engine/structure.rs`.

### Interpolation — `Interpolate`, `Snap`, `Interpolable`

The state between a journal's before and after, so any change can be
shown part-way without animation code of its own.

`Interpolate::lerp(from, to, t)` blends one value (scalars, `Vec2`,
`OrderedVec2`, `OrderedFloat<f32>`, RGBA as `[f32; 4]` or rounded
`[u8; 4]`), returning the endpoints themselves at `t = 0` and
`t = 1`. An `Interpolable` element type says which of its fields
interpolate; every other field takes the before or the after value by
`Snap` — `Start` switches as soon as `t > 0`, `End` only at `t = 1`.
`Journal::write_interpolated(&mut tree, t, snap)` writes every
entry's interpolated element, and stands moved elements in the
before or the after arrangement by the same snap, so the rows stay
sorted. The glyph instantiation interpolates an area's position,
render bounds, scale and line height, a model's position, and a
run's origin, opacity and per-glyph overrides; a run's shaping and
zoom window snap.

`lib/baumhard/src/engine/interpolate.rs`; `Interpolable for
GfxElement` in `lib/baumhard/src/gfx_structs/element.rs`.

### Mutator builder DSL — `MutatorNode`, `BuildContext`, `Repeat`, runtime holes

A serde-friendly AST (`MutatorNode<E, M>`) that compiles
to a `MutatorTree<E, M>`, with a `BuildContext` for runtime value
injection.

Declaring mutators by hand as a `GfxMutatorTree` is fine for Rust
code but hostile to JSON authoring. The builder DSL solves this:
authors write `MutatorNode` in JSON (the shape is nearly identical
to `Mutator` but serializable and with `Repeat` for "N consecutive
channels with the same template"), and the builder walks the AST
with a `BuildContext` to resolve runtime values (counts, templates,
dynamically-chosen mutations) into a concrete tree ready for
`walk_tree_from`. This is the seam
[the mutation framework](#4-the-mutation-framework) attaches to.

`lib/baumhard/src/engine/builder/`. The AST:
`MutatorNode::{Void, Single, Macro, Instruction, Repeat}`, each naming
the `kind` half of its key beside its channel. The
indirection enums `ChannelSrc` and `CountSrc` have a `Literal`
variant (inline) and a `SectionIndex` or `Runtime(String)` variant;
`MutationSrc` is a payload `Template` resolved per `Repeat`
iteration, a `Runtime(String)` mutation, or `None`; the runtime
ones consult the `BuildContext` trait at build time.
`build(ast, context)` returns a `MutatorTree<E, M>` — children inserted
in key order — with `Repeat` expanded to N children of its kind on
consecutive channels, or a `BuildError` for a shape no mutator tree
can express (a root `Repeat`, a `Repeat` template of another kind, a
`SpatialDescend` for an element type with no geometry, …).

### Font system — `FONT_SYSTEM`, `AppFont`, `attrs_list_from_regions`, `RegionFamilies`, `rich_text_spans_from_regions`

A single global cosmic-text `FontSystem`, a
compile-time enum of available fonts, and a small set of bridges
from `ColorFontRegions` to the two cosmic-text shaping API shapes.

Every piece of text shaping in the project
flows through these. `fonts::init()` is called once at startup;
the `FONT_SYSTEM` `RwLock` is private and is acquired through
`acquire_font_system_write("site-name")` (or its siblings
`try_acquire_font_system_write` / `acquire_font_system_read`). Re-acquiring on the same thread panics at once; a wait on
another thread's hold is bounded and ends in a panic naming the
holder, never a hang. `AppFont` is generated at build time by scanning
`lib/baumhard/src/font/fonts/` — drop a font file in, recompile,
and the variant appears.

**The default face and the fallback** (#228, #297,
`font/fallback.rs`). `FONT_SYSTEM` holds the compiled faces and, on
native, the host's installed faces after them, less any bearing a
compiled family's name (R30.1). Text that pins no family is drawn in the
default face, `DEFAULT_FACE`: Inter 4.1's variable face, which the font
system names for every generic family. A glyph a span's face lacks
falls back through one list, the same everywhere: the default face, the
two ornament faces of `ORNAMENT_FALLBACK` — `Mandala Ornaments`
(DejaVu Sans subset to the demo maps' ornaments, the box-drawing
characters and the app's chrome glyphs: the console's prompt, cursor,
gutter and marker, the rubber band, the `body` presets) and `Mandala
Emoji` (Noto Color Emoji subset to `🌸`), regenerated by
`scripts/ornament_fonts.sh` — and the compiled script faces of
`SCRIPT_FALLBACK` (Hebrew, Devanagari, Tibetan, Egyptian hieroglyphs);
a glyph no compiled face holds is drawn, on native, in a host face. The ornament faces hold no code point the default face
holds, so a letter, digit, space or punctuation mark is never drawn by
one. Each static fallback face is registered at every CSS weight, since
a fallback family is matched only at the exact weight a span asks for
or within a variable face's axis. A span whose own family holds the
glyph still draws it. See `format/fonts.md`.

**The shaper is patched** (#304, `vendor/cosmic-text`). cosmic-text
0.19.0 is vendored and every crate's `cosmic-text` is pointed at it by
the root manifest's `[patch.crates-io]` (and the fuzz crate's, and the
wasm32 probe's), so the app, glyphon and baumhard share one copy. The
patch, marked `MANDALA PATCH` in its `src/shape.rs`: the font
fallback's merge of a fallback face's glyphs is linear in the run (the
published one was quadratic in a run the first face lacks: one 262 KB
span took 596 s to project, #243); a code point no face draws is asked
of the faces once per font system, and a run holding it then tries no
fallback face for it; a line can take another's shaping at other sizes
(`BufferLine::adopt_shape`), so a rescale lays out without shaping; and
the shaper counts its work per thread (`cosmic_text::shape_work`:
shapings, bytes shaped, fallback rounds, merge steps, coverage probes),
which is how tests show cost without a clock. `set_reference_merge`
selects the published merge on a thread, for the tests that hold the
patch to it glyph for glyph. `util::manifests` holds the patch table to
its rule: the members' version, a path in the repository, the same in
every workspace that builds baumhard.

`lib/baumhard/src/font/`. Two bridges, one
shared private resolver, both live in `attrs.rs`:

- `attrs_list_from_regions` returns a single
  `cosmic_text::AttrsList` for callers using
  `Editor::insert_string`. `None` family resolution forces
  `Family::Monospace` per the `Editor` shape's existing fallback.
- `RegionFamilies` + `rich_text_spans_from_regions` return a
  `Vec<(&str, Attrs)>` for callers using `Buffer::set_rich_text`
  — the renderer's tree walker today. `RegionFamilies::resolve`
  caches the borrowed regions slice and pre-resolves family-name
  strings once per text area so the renderer's nine shape passes
  (one main + eight outline-halo stamps) reuse the same lookups.
  `None` family resolution omits the family pin (cosmic-text
  picks), preserving the walker's pre-existing fallback.

Unknown fonts log a `warn!` and drop to a monospace / no-pin
fallback rather than aborting — interactive paths must not panic
([`CODE_CONVENTIONS.md §9`](./CODE_CONVENTIONS.md)). The lock tells
its two failures apart rather than timing them: a thread
re-acquiring what it already holds is refused at once from a
thread-local record, and `FONT_SYSTEM_LOCK_TIMEOUT` bounds only a
wait on *another* thread's hold, which is a deadlock if it runs that
long.

### `RegionParams`, `RegionIndexer`, `RegionError`

A grid-bucketed spatial index over color/font
regions for cheap hit-testing.

Hit-testing "which region contains this point?"
against hundreds of spans over thousands of glyphs would be
linear per query. `RegionIndexer` divides the rendered surface
into a grid of buckets; queries consult the bucket containing
the point and scan only that bucket's regions. `RegionParams`
configures the grid, adapting to the resolution so dimensions
that don't factor cleanly (primes, near-primes) still get a
sensible subdivision.

`lib/baumhard/src/gfx_structs/util/`.
`RegionError::{InvalidParameters, Poisoned}` covers the failure
modes; callers match and decide rather than panicking. The indexer
and its parameters are a tested-but-unwired subsystem: they are
allocated by `Tree::new` but `MutatorTree::apply_to` does not
currently maintain them. Per-tree BVH descent (`Tree::descendant_at`)
handles hit-testing today; when the region index is wired, region
mutations must go through the mutator pipeline or the index will
drift silently.

### Utilities — `grapheme_chad`, `color`, `geometry`

The shared-primitive toolkit: grapheme-aware text
operations, color types and macros, and epsilon-aware 2D
geometry helpers.

Three small modules that the rest of the
codebase builds on, rather than each module re-implementing its
own take:

- **`grapheme_chad`** — the only legitimate way to manipulate
  `String`/`&str` when the offset comes from user input.
  Among its functions: `count_grapheme_clusters`,
  `find_byte_index_of_grapheme`,
  `replace_graphemes_until_newline`, `split_off_graphemes`,
  `delete_back_unicode`, `delete_front_unicode`,
  `find_nth_line_grapheme_range`, `count_number_lines`, and the
  display-width pair `truncate_to_display_width` (clip the
  overflow) / `wrap_to_display_width` (fold it onto the next
  line). Byte slicing from user-facing counts lands mid-cluster
  on the first emoji; always reach for these. See
  [`CODE_CONVENTIONS.md §1`](./CODE_CONVENTIONS.md) and
  [`lib/baumhard/CONVENTIONS.md §B3`](./lib/baumhard/CONVENTIONS.md).
- **`color`** — `FloatRgba = [f32; 4]` and `Rgba = [u8; 4]`
  color types, `Palette = Vec<FloatRgba>`, plus compile-time
  macros `rgb!`, `rgba!`, and (non-const) `hex!`. Channel-index
  constants for consistency.
- **`geometry`** — `almost_equal` (`|a - b| ≤
  ERROR_TOLERANCE_ALMOST_EQUAL`, the baumhard-wide epsilon, declared
  beside it), `clockwise_rotation_around_pivot`,
  y-dominant `pixel_greater_than` and siblings (cursor-reading
  order), `vec2_area`. `Comparator` float equality uses
  `almost_equal`.

`lib/baumhard/src/util/`. All pure functions,
no shared state, no allocations beyond what the return types
demand.

### The document — `Document`, `DocElement`, `DocKind`

The map as the engine's second instantiation: a `Tree<DocElement>`
whose shape *is* the map's structure. The root `Map` holds the
`Theme`, the `Palette`s, the map's `Defaults` and the root nodes; a
`Node` holds its `Section`s, its `Frame` and its `Ring`s, its
`Connector` to its parent, its `Link`s and its child nodes; text is
always a run of `Span`s under whatever draws it. There is no parent
pointer, no id that encodes a path and no edge list — structure is
stored once, as nesting. Built in V1-B1 (#235) beside the model it
replaces; nothing in the app uses it until the cut-over (V1-C3).

- **`DocKind`** is the element's kind, the first half of its routing
  key, and its declaration order is the order a parent keeps its
  children in: a node's sections, frame, connector, links, then child
  nodes. A mutator for one kind never lands on another.
- **Typed values** (`document::values`): a color is a `ColorRef`
  (a literal, a theme variable, or a palette field), a font an
  `AppFont`, a size a `FontSize` (`Pt`, or `Rel` — a fraction of the
  node's shorter side, for frames), a shape a `Shape`. Strings are
  parsed once, at load.
- **Identity** (`document::ids`): in memory an element is its arena
  `NodeId`; from outside, a node is its `NodeIdent`, which only
  `Document::node_by_ident` maps — for the loader, the console and IPC.
- **Per-field inheritance** (`document::resolve`): every style field
  is an `Option`, and a resolver computes each one from the element,
  its owners, the node's palette binding, the map's `Defaults` and
  `BUILT_IN` (`document::defaults`), recording every input it reads
  through a `ReadRecorder` so the projection can learn what depends on
  what. An input names as little as the read needed — an element's
  style, its placement, a node's palette binding, a children row, the
  theme, a palette, the defaults, a depth — and
  `document::deps::inputs_changed` maps a change of one element onto
  them, so moving or resizing a node dirties no resolution, and
  dragging a label or a portal marker dirties no text resolution. A connector's, portal marker's, label's or
  frame's size and color stay in their own flow and never come from
  the node's text.
- **Shape rules** (`document::shape::validate_shape`) say which kind
  may hold which — `Document::validate_subtree` judges a detached
  subtree's rows the same way before a structural insert puts it in;
  **bounds** (`document::bounds`) every number's range.
- **Structure** changes through the engine's
  [structural commands](#structural-commands--structop-subtree). Only a
  span splits and merges: at a grapheme-cluster index, and with a
  neighbor of the same style and link.

`lib/baumhard/src/document/`. `util::document_surface` holds it to
naming nothing of the model it replaces.

### The document's mutations — `DocMutation`, `DocInherited`, `DocStep`

Every change a document can undergo, as the engine's payload for the
document (V1-B3, #237). Every path that changes a map — actions, the
console, IPC, drags, text editing, edits, presentations, verbs — builds
mutator trees from this vocabulary and nothing else, and the
[writer](#the-documents-writer--documentapply-editrequest-transaction-changerecord)
applies them with history.

- **`DocMutation`** has one payload per kind (`Node(NodeOp)`,
  `Span(SpanOp)`, `Path(PathOp)` for connectors and links, `Cap(TextOp)`
  …). Each op writes exactly the field it names; there are no setters.
  The relative ops (`Nudge`, `GrowSize`, `ScaleSize`, `MoveControlPoint`)
  clamp their result with `document::bounds`, and an index the reached
  element lacks leaves it unchanged with a diagnostic.
- **`DocInherited`** is what a document walk carries down, so "grow the
  font" on text with no size of its own grows the size it displays:
  `text_size_for(kind)` answers what `resolve_text` would, from
  `Defaults` tails the seed reads once (`inherited_seed`) and the owners'
  sizes within the flow. A frame never takes the node's points, and a
  connector, cap or label never takes node text.
- **Validation** has two halves. `validate` judges a mutator tree before
  it walks: payload against key, every literal against its bound,
  every reference, and the structural commands the document allows
  (`validate_for_presentation` refuses all structure, D17).
  `validate_applied` judges a request's journal after its last step:
  the rows it changed, inserted links (nested ones included), links
  whose kind changed, and that every size it wrote is in the unit its
  place takes — none at all on a span inside a ring. A font is never
  the fallback face, and a channel never passes `MAX_CHANNEL`. Both
  halves iterate with explicit stacks and remember which elements are
  in the tree, so depth costs them no call stack and no climb per
  element (§20 rule 11).
- **`DocStep`** is one walk of a request, built from the document as the
  earlier steps left it — one walk cannot split a span twice and style
  the middle, or merge a run of three. The composite builders
  (`document::mutation::compose`: `create_child`, `delete`, `reparent`,
  `set_link_kind`, `style_range`, `insert_text`, `delete_text`,
  `split_section`, …) return the steps of each change whose shape is
  not obvious; the writer runs them on one journal builder, all or
  nothing.

`lib/baumhard/src/document/mutation/`.

### The document's writer — `Document::apply`, `EditRequest`, `Transaction`, `ChangeRecord`

**The only way a document changes after it is loaded** (design §8,
§9; V1-B4, #238). Every change is an `EditRequest` — the ordered
`DocStep`s of one change and a label — declared by a `ChangeSource`
(`Action`, `Console`, `Ipc`, `Input`, `Edit`, `Verb`; `Undo`, `Redo`
and `Presentation` are the history's and the presentation layer's own,
and a request cannot declare them). Nothing outside `document` holds a
`&mut` to the tree or an element; `util::document_surface` holds every
`pub` item there to handing none out.

- **`apply`** runs a request's steps on one journal builder, then adds
  the writer's own steps to the same builder: every inserted node is
  given the next `NodeIdent` (never past `MAX_NODE_IDENT`, never
  issued twice), and every link left leading into a removed subtree is
  removed — found through the document's index of incoming links,
  which every journal moves, never by a scan. The closed journal is
  judged once (`validate_applied`) and held to wasm32's element and
  text budget; any failure leaves the document byte-identical. The
  journal becomes one history entry and advances the `Revision`.
- **Undo and redo** revert and replay an entry's journals, newest first
  and oldest first; nothing is evaluated again, and there is no other
  undo representation. The history is bounded (`DEFAULT_HISTORY_LIMIT`,
  50, D18); a journal that leaves it — off the end, or with the redo
  branch a new change clears — is discarded, which frees the subtrees
  it held out of the tree. `is_dirty` compares the history position
  with the saved one by identity, so undoing back to the saved state is
  clean and a different change of the same length is not.
- **Transactions** (`begin`, `stage`, `apply_in`, `interpolate`,
  `commit`, `cancel`) are live interaction: a stage replaces the one
  before it, so a drag stages the whole offset every frame and the
  document is exact at each; a base call (`apply_in`) stays across
  stages, so the node an editor session creates keeps its id; the
  commit is one entry. One is open at a time (D28): an interactive one
  refuses every other `apply` and `begin`, an animated one is
  fast-forwarded to its end and committed first, and an undo or a redo
  is one visible step: it cancels an interactive transaction and stops,
  or makes an animated one its own entry and goes on. `save_json` writes the committed state.
- **The change feed** (`take_changes`) is one `ChangeRecord` per change
  — its source, the revision, every element it touched with how
  (`Touch`), and the address (`RemovedAddress`: the owning node's
  ident, the kind, the channel, the index among its siblings) of each
  element it took out of the tree. Beside them, read from the journals
  the change wrote and never from the document (ruling B7.2 on #241):
  the resolution input kinds each element it changed touched
  (`ChangeRecord::inputs`, `inputs_changed` of the element's two
  images in the change's direction), and every parent whose children
  row it rearranged (`ChangeRecord::rows`). The projector (V1-B7) and
  IPC's event stream (V1-E2) read it; neither diffs anything.
- **One funnel.** Every call that changes the tree runs through one
  private method, `mutate`, where the presentation layer (V1-D2) will
  revert and re-apply around each change.

`lib/baumhard/src/document/writer/`.

### TextFlow — `FlowSpan`, `ShapedRun`, `ShapedBox`, `PlacedRun`, `Outline`, `Curve`

The one text primitive: a sequence of spans plus a layout. A section
or a label is a **box** flow — its spans shaped as lines broken at
`\n` only, in a box that grows to fit them (`shape_box`). A frame
ring, a connector, a line link and a portal marker are **path**
flows: shaped once as a single line (`shape_run`), then placed around
the node's outline (`place_on_closed`), along the connection curve
(`place_on_open`) or at a point (`place_at_point`) — shape once, place
many. Built in V1-B5 (#239); nothing draws it until the renderer
sub-issues. A flow's result becomes one [`GlyphRun`](#glyphrun)
element.

- **Spans are real.** Each `FlowSpan` is text, a `ResolvedText` and
  an optional link. Its face, size (per span, through cosmic-text's
  per-span metrics), weight, slant and color all reach the shaper.
  Sizes arrive in points; converting a frame's relative size is the
  projection's job. A span pinned to a compiled family is drawn in that
  family's compiled faces on every machine: its weight is resolved to
  the nearest one the family has, as CSS Fonts 4 matches `font-weight`
  and with nothing synthesized, before cosmic-text sees it — the
  shaper keeps a pin only at an exact weight and would otherwise draw
  the span in another face of that weight, the variable default face
  among them (R30 on #278, `PinnedFamily`). Only a character the family
  does not hold falls back to another face, through the one fallback
  list (#228, #297, "The default face and the fallback" under "Font
  system" above).
- **Glyphs are never rotated.** The unit of placement is the grapheme
  cluster, so a mark never leaves its letter. A cluster of advance `w`
  and line height `h` takes a step `|cos θ|·w + |sin θ|·h` along the
  path, θ the direction of the chord the step spans: side by side along
  a horizontal stretch, a line apart along a vertical one, blended on a
  curve and turning smoothly through a sharp corner — the way ASCII art
  makes one. That is the box step, which `Repeat` rings take. A
  connector's or a line link's `Repeat` ornament is laid instead as
  whole glyphs evenly gapped from node to node (`place_pitched`, #296):
  as many inks as bring every gap, box to box, nearest the pattern's
  own, whichever way the path runs, its two end gaps among them, each
  measured to the boxes at its end (the frame's inks). A frame's text is sized with the level step (ruling R28,
  `PathWalk::level_len`: `w` horizontally), then laid by its letters'
  inks along its half (below). Around an outline each cluster is pushed
  outward by half its extent, so text sits on the outline rather than
  across it.
- **Readable path text** (D33, #291). Path text reads left to right or
  top to bottom.
  - **A frame's text is two lines** (ruling R27; `flow/place/lines.rs`).
    A ring in `Once`, `Spread` or `Fit` is parted at its first line
    break, or at the word break where the two lines best balance against
    the level parts of the outline — over the top and along the bottom,
    where it runs within 45° of level — and `Fit` takes the largest size
    at which both lines fit those parts, too long at `min` cutting only
    the bottom line's end. That is the parting and the size; the lines
    are laid along the halves below. `start` and `direction` apply to
    `Repeat` rings only.
  - **Each line follows its half** (rulings R32, R33, R33.1;
    `flow/place/halves.rs`). R27 parts and sizes the text; the lines are
    then laid along the outline's diagonal halves, split where it faces
    up-left and down-right (`Outline::support`): the first from the
    top-left junction clockwise over the top and down the right side,
    the second down the left side and along the bottom, glyphs upright.
    Every ink of the frame, both lines' and the sides' copies, is
    anchored on one band curve, the outline's offset at one distance
    all round (#308, ruling B15.1): a letter at its ink's center along
    the line and its span's band center across it, halfway between its
    baseline and its x-height, so a span's letters share a baseline and
    their ascenders and descenders hang free; the distance is solved once
    a frame, outward the least that clears every ink, inward centering
    the frame's ink on the outline.
    Each line is tracked evenly — one ink gap between every two letters,
    by the ornament's pairwise rule, and a space plus two gaps between
    words — over the part of its half its `span` gives it
    (`RingLayout`, `values::LineSpan`: `natural`, at its font's own gap
    and centered on its half's apex, to `whole`, the default). At each
    junction the two lines' end inks stay two word gaps apart, and
    `sides` (`values::Sides`) fills that break with whole, evenly
    spaced copies of an ornament no taller than the text's x-height —
    a `Once` filler, when `sides` is unset (ruling R34). A `Once` or
    `Spread` line is never cut to the level part; only a line its half
    cannot hold at its font's gap is cut, a `Fit` frame shrunk first.
  - **A pattern and a connector read stretch by stretch**. On a `Repeat`
    ring and every connector, the walk (the fit mode) decides which
    clusters are placed and which stretch — rightward, leftward,
    downward, upward, by each step's dominant direction — each lies in,
    a word a boundary would tear moved whole to one side; then the same
    clusters are laid in reading order, each leftward and upward
    stretch's last to first, each stretch in its own part of the path,
    borrowing room only from a neighbor read its way. A tight turn
    through vertical makes no stretch of its own (a hysteresis); a loop
    reads like a seal. An ornament, with no letter or digit, has no
    reading direction and is placed as walked, each glyph stepped by
    its ink, not a line-high box, and set as far from the last glyph's
    ink as a line would set them, so a ring's gaps are even where its
    outline runs steep and round its corners (ruling R31). The one
    exception to
    keeping the walk's glyphs:
    with no slack and a zero gap, the end is cut — filler copies or
    repeat clusters first.

  - **A directional ring** (#324; `flow/place/directional.rs`,
    `element::DirectionalGlyphs`). A `Repeat` ring with a `vertical`
    text or `corners` lays its spans where the outline runs level, the
    vertical text where it runs steep, and a corner where it turns as a
    box does, so `─ │ ┌ ┐ ┘ └` round a rectangle reads as a box. The
    outline is parted at its four diagonal support points
    (`Outline::support`), where it turns through 45°, into two level and
    two steep runs — a run shorter than three line heights that ends on
    a curve left to its neighbors (the readable rule's hysteresis), a
    side of a box kept however short — and each run is laid whole
    between the corners beside it as a connector's pattern is
    (`place_pitched`'s core, `pitch::pitch`), its gaps measured along the
    way it reads, the spans' own gap all round, never looser. A corner
    is set cell to cell with its neighbors, on the level text's baseline
    and centered on the vertical text's box, only where it would lie on
    the outline: where the box's corner, its sides run straight on, lies
    within half a line height of the outline's turn; round a wider curve
    the runs meet with none. A side of a box
    runs straight from corner to corner. Its path keeps a sharp rectangle
    sharp (`offset_keeping_corners`).

  Either way a placement lists its glyphs in text order; only positions
  move. One placement places at most `MAX_PLACED_GLYPHS`, 65,536
  glyphs (ruling R41), text, repeat copies, fillers and sides together;
  past it the flow stops and reports `PlacedRun::truncated`.
- **Outlines** (`outline`, `offset`, `ring_offset`): a node's
  rectangle shaped as a rounded rectangle (radius a percentage of half
  the shorter side) or the inscribed ellipse; ring *k* runs around the
  outline grown past the rings inside it. Lines and circular arcs are
  walked exactly by arc length, ellipses and Béziers through a table
  whose points are evaluated on the curve itself.
- **Fit modes** say what happens when text and path differ in length:
  `Once` (with an optional filler, appended to the run in the flow's
  style), `Repeat` (reusing the run's glyph indices; around a closed
  outline, whole copies evenly spaced so the ring closes without a
  seam, rulings R29 and R31, a ring's `spacing` more between every two
  steps, its work under a frame's budget, #310; along an open path every
  mode's work is under one too, and a connector's whole-glyph pattern's,
  #313), `Spread` and `Fit`
  (rescaling the run), the last two solved so the last cluster ends at
  the path's end. On a frame they part and size its two lines, which
  its `span` then lays along its halves.
  Text too long is cut to the longest prefix, in reading order, that
  fits — a right-to-left run keeps its beginning — and never
  overlaps. A rescale — `Fit`'s probes, R27's sizing of a frame's
  lines — shapes nothing (#304): the rescaled buffer takes the run's
  shaping and is only laid out at the new size, glyph for glyph what
  shaping it again lays out (`flow::shape::resized_buffer`); only a
  filler, or text holding a tab, is shaped again. An ornament's inks
  are read off its outlines once per glyph and size, and kept
  (`fonts::cached_outline_bounds`, per thread, used under the font
  lock's guard).
- **Hit mapping.** Every glyph names its span and its bytes of that
  span's text (`GlyphMetrics`, `ShapedBox::glyph_spans`); a filler
  glyph names `FILLER_SPAN`. No paragraph separator is drawn. A box
  breaks a line at `\n`; a path run lays each separator as a space
  (`GlyphMetrics::line_break`), and a frame's text parts its two lines
  at the first.
- **Tabs** (R24, R24.1, R24.2 on #278). A tab is a space widened to the next
  tab stop, eight of the face's spaces apart from the start of its
  line, as cosmic-text draws it. A face whose space has no width cannot
  measure a stop (cosmic-text would make the tab and every glyph after
  it on the line `NaN` wide or placed), so shaping decides **per span,
  from that span's face, before shaping**: a span whose face's space
  has no finite, positive advance draws its tabs as spaces, and every
  other span keeps its tab stops, beside it or on another line. That
  decision measures each span's primary face, and a tab shaped in a
  fallback face can still come back non-finite, so a last-resort check
  after shaping (R24.2) lays the text out again with its tabs as spaces
  when a width or position is not finite — only the failing lines of a
  box, so the others keep their stops, and the whole of a run — and
  logs it. No `NaN` from a tab reaches placement or the renderer.

`lib/baumhard/src/flow/`. It caches nothing, builds no glyph tree and
never takes the font lock: every shaping function takes the guard its
caller holds, which it takes after `fonts::init` has run.

### Projection — `Projector`, `GlyphLayers`, `LayerChanges`, `Hit`

The document turned into glyph layers, **forward only, with work
proportional to what changed** (design §12, V1-B7 #241). Nothing reads
the layers to learn what the document is, and nothing flows back.

- **`GlyphLayers`** are five glyph trees — sections, frames,
  connections, labels, portals; there is no fill layer (D12) — each
  mirroring the visible nodes as nested `Void` groups (child groups on
  `CHILD_GROUP_CHANNEL`), a group's own `GlyphRun`s under it in
  document order. `format/projection.md` says what is drawn where.
- **`Projector::project_full`** projects a whole document: what load
  does, and what the oracle compares against. **`Projector::update`**
  takes the document's `ChangeRecord`s since the last call and
  re-projects exactly what they touched and what read it, writing the
  layers through journaled glyph mutations — a `RunSet` only where a
  run's data differs from what the projector last wrote (never compared
  with the live element, which overlays may have written), and
  structural commands for groups and runs that appear, leave or move.
  Its **`LayerChanges`** are the five journals, which the renderer
  discards once it has read them.
- **Units and stages.** A unit is an element that draws runs of its own
  — a section, a ring, a connector or line link (body, caps, label), a
  portal link (each end's marker and label). A node carries its facts
  down (`Stage::Carry`: its palette binding, effective opacity and zoom
  window, from its parent's — never climbed for, ruling R16) and
  measures itself (`Stage::Geometry`: its drawn rectangle from its
  sections, D5, and its outline); a unit resolves (`Resolve`), shapes
  (`Shape`, only when its shape key — the texts and styles shaped from —
  changed) and places (`Place`). Phases order them: carry top-down,
  sections, measure, then sections' and rings' placement, then paths —
  and of the paths, every one's ends (`Geometry`) before any is laid
  (`Place`), because the paths that leave one side of a node are fanned
  together (#296), and every path laid before any node hides its glyphs
  (`Hide`, #332).
- **Connector geometry** (#296, `projection/geometry.rs`). A path
  attaches to its node's **reach** — the box round its drawn rings'
  inks, or its drawn rectangle — at the midpoint of the side its
  `anchors` name or that the ray toward the other node leaves through,
  leaves and arrives square to that side, and passes through its control
  points, one cubic between each two. The paths that leave one side of
  a node share its midpoint, fanned by the turn that clears their first
  glyphs, at most 40°, and held out past the point where the cap leaves
  them too close (`Projector::junctions`, keyed by node and side). Along
  their whole length they keep clear of each other: two that would come
  too near turn toward their other ends sooner (`geometry::untangle`).
  Each junction is fanned once an update, after every path's ends are
  found, and only the paths whose fan changed are laid again. Portal
  ends with no `border_t` that face one way at a node are held apart
  along its outline (`geometry::held_apart`).
- **Nodes hide the paths behind them** (#332, `projection/behind.rs`). A
  node's contents take priority over every path that does not end at
  it: once every path is laid (`Stage::Hide`, its own phase), a glyph of
  a path's body or caps whose box, grown by the path's gap, meets the
  node's **region** — what it draws, in two parts: the box round its
  text's glyph cells, and the box round its rings' inks by its outline — is
  left out of the run its layer holds, whole — and the path's label,
  whole, where the box round its text meets that region — when the node is drawn
  wherever the path is (opacity above 0, a zoom window holding the
  path's). It is a view
  decision: the document keeps the path, the projector keeps the run as
  laid (`Projector::laid`), and the glyph comes back when the node moves
  off. Indices on the hit grid keep it incremental: the regions, a body
  index for each part (`Projector::regions`), and where each path's glyphs are
  (`Projector::path_cells`). A path's hide records reading the `Region`,
  opacity and zoom window of each node it met, so a node whose region
  changes marks it; that node then finds, in the path cells, the paths its
  new region meets.
- **Captured dependencies.** Every read of another element's state goes
  through a recorder into the dependency index (`projection::Input`:
  the resolvers' `document::resolve::Input`s, and the projection's own
  `Geometry`, `Size`, `Visibility`, `Opacity`, `Parent`, `Inner`,
  `Reach` and `Region` of an element), keyed by the stage that read it. The change
  records' `inputs` and `rows` (ruling B7.2) seed it, and a touched
  element seeds only the stage its unclassified fields feed: a label, a
  portal end or a cap its unit's placement, a section's offset its box,
  a line's anchors, control points and zoom window its ends, which lay
  it — its resolution only when a field that resolution read changed.
  So a drag — of a node, a section, a label, a portal marker or a
  control point, geometry only — reaches geometry and placement and
  resolves and shapes nothing. Nothing is listed by hand; the oracle test
  holds the index to it, comparing the incremental layers with a full
  projection after every edit of a corpus of generated maps and the
  format's fixtures.
- **One shaping per key** (`projection::shapes`, #304, ruling B7.4).
  The projector's shaping cache holds, by full shape key (texts, styles
  with sizes in points, and a box's minimum size and alignment, over the
  font system's face set), weak references to the key and the shaping
  some unit holds: N units with one key make one shaping and share its
  `Arc`, and hold one key `Arc` (a unit whose key is found takes the
  entry's), so a share lasts as long as any sharer. The cache keeps nothing alive, neither a shaping nor a key and
  its text, and holds at most about twice as many entries as there are
  live shapings, a dead one two weak pointers. A hit reads no font, so
  it takes no lock.
- **`Hit`** (`Projector::hit`): the layers top-down through each one's
  glyph BVH — a path run only where one of its glyphs is drawn — to the
  element and, through the run's glyph-to-span mapping, the span; then
  the smallest visible node whose body holds the point by its shape
  (ruling B7.1); else nothing.
- **Read API** (`Projector::node_rect`, `section_box`, `curve`,
  `label_pos`, `laid`, and the `Geometry` trait): derived geometry for the
  consumers that act on where things are drawn, served from projection
  state.

`lib/baumhard/src/projection/`. The font lock is reached only through
the caller's `FontSystemLease` — the lazy guard the spec names
`LazyFontGuard` — and only when something is shaped or placed again.

---

## §3 The document domain

A map is a **document**: a tree of typed elements, read from and
written to `.mandala.json` ([`format/schema.md`](./format/schema.md)),
changed only through one writer. It lives in
[`lib/baumhard/src/document/`](./lib/baumhard/src/document/); the full
explainer for it is V1-E4's (#256).

### Elements

`DocElement` is the element type of the document's tree (a `Tree<E>`,
§1): the map at the root, its theme, palettes and defaults, and the
nodes — each holding its sections and their spans, its frame and its
rings, its connector to its parent, and its links with their labels,
caps and portal ends. Each kind is a `DocKind`. A node's channel is
its own, a portal end's is its end, every other kind's is 0: the key a
document mutator routes on ([`format/mutations.md`](./format/mutations.md)
§"The document's kinds and predicate fields"). Values are typed
(`document::values`): a color, a size or a shape is parsed once, at
load, and nothing after load parses a string.

### Ids

Inside the process an element is an arena `NodeId`. In the file and on
the IPC wire a node is its integer ident (`NodeIdent`), and every other
element is named relative to its node — `{node, section}`,
`{node, connector: true}`, `{node, link}` ([`format/ipc.md`](./format/ipc.md)).

### The format

`document::format` loads and saves `.mandala.json` strictly: every type
the loader reads denies unknown keys, none reads an index, and a map
written in the saver's spelling saves back to its own bytes. The
strictness is held against the sources through `util::serde_coverage`
(`document::tests::format_strict_tests`).

### The one writer

Every change is an `EditRequest` — steps of document mutations —
applied by the writer (`document::writer`). The writer journals each
change into the history, which is what undo and redo walk; advances
the document's `Revision`; and records the change on the change feed,
which the view drains once per sync and hands to the projector and to
the IPC event stream. An interactive or animated change is a
transaction (D28): while one is open every other change is refused,
and the app shows the refusal on its status line. Unsaved work is
derived — the history against the position `mark_saved` last recorded.

### Projection

What the document looks like is the projector's (§2, "Projection"):
the document into the five glyph layers once at load, then updated
from the change feed.

## §4 The mutation framework

The engine (`lib/baumhard/src/engine/`) applies a `MutatorTree` to a
`Tree<E>`: the walker pairs mutators with elements by key, under a
journal, so every application can be reverted and replayed exactly.
Two element types use it: the glyph trees (`GfxElement`, with
`GfxMutation`) and the document (`DocElement`, with `DocMutation`).

The glyph vocabulary — commands, deltas and glyph-run mutations — and
the `MutatorNode` AST that carries it, with runtime holes a
`BuildContext` fills, are kept (D24) and published in
[`format/mutations.md`](./format/mutations.md). Nothing in a
`.mandala.json` holds a glyph mutation today: the authored carrier is
the **edit** (V1-D1, #250), and presentations are V1-D2's.

## §5 The application runtime

The application runtime is the shell around the document. It owns
the event loop, the input state machines, the renderer, the
modal-UI state, and the keybind table. It does not own the data
model — that lives on [`MindMapDocument`](#mindmapdocument). The
split between "what changed" (document) and "what is on screen"
(renderer) is the model/view discipline at work.

Lives under [`src/application/`](./src/application/).

### `Application`, `InitState`, and the two native drivers

The native entry points. `Application` is the pre-window root;
`InitState` is the persistent app state; `NativeApp` (winit's
`ApplicationHandler`) and the [headless driver](#headless-driver) are
the two things that run it.

The platform separation is honest: pre-window work (parse args, init
fonts, load mutations) happens before any GPU resources exist; once
there is somewhere to draw — a window the OS gave us, or, under
`--headless`, nothing — `run_native_init::build` makes the
`InitState`, and the app stays there for the lifetime of the run.
Neither driver holds anything substantive: both run the one loop body
(`app/pump.rs`) over `InitState`, and differ only in how they wait
and where a frame comes from.

`src/application/app/run_native_init.rs` holds `InitState` and
`build`; `run_native.rs` and `run_headless.rs` the two drivers.
`InitState` carries the `shell` (the window or none, `app/shell.rs`),
the renderer, a `document: Option<MindMapDocument>`, its `view`
(an `Option<MapView>`: the projection and the glyph layers — view
state, not document state), `app_scene`, `drag_state`,
`interaction_mode`, modal UI state (console, color picker), the
resolved keybind table and macro
registry, the touch recognizer, and the clock, recorder, injected
queue, watchdog and redraw ledger the IPC surface reads. Its `input_context()` produces a borrowed view of these
fields per event so handlers can borrow disjoint subsets without
lifetime contortions.

The `Option` on `document` is the interactive shell's shape, not
startup's: init always produces a document, because a load that
fails produces the [load-failure placard](#load-failure-placard)
instead of nothing.

### Load-failure placard

The document a rejected map load is rendered as, so the loader's
message reaches the canvas instead of only `stderr`.

Startup is the surface most likely to be a user's first contact
with the format, and it used to be the only one that swallowed a
loader error — the console `open` verb puts the message in the
overlay, `maptool` puts it on `stderr` and exits nonzero, and
startup logged it and drew an empty canvas. Behind a double-click
there is no terminal, so that log line did not exist for the person
holding the file (#107).

A placard is an ordinary one-node document
(`MindMapDocument::load_failure_placard`) whose text is the headline,
the path or URL that was asked for — on a line of its own unless the
message already names it (`document/placard.rs`) — and the loader's
message. Because it is a document, it reaches the screen through the
same projection as any other document (§1 "Everything is glyphs") —
no second pipeline, no renderer pass, and nothing that needs a GPU to
test. It is bound to no file, so a reflexive save cannot overwrite
the file that failed to load.

`src/application/app/startup_load.rs` is the decision. Each init
path has one entry — `native_startup_document` and
`wasm_startup_document` — and both are
`adopt(startup_surface(...))` and nothing else. `StartupSurface` has
one arm per outcome and is private to the module; `resolve` is the
only place its arms are matched, and is pure, so which document is
installed and what is said about it are both assertable without a
logger; `adopt` is the three-line shell that runs `resolve` and
logs its report. That is what keeps the browser build and the
desktop build reporting a bad map identically (§1 "Cross-platform
as first-class"); the browser build's earlier DOM overlay is gone,
replaced by this. Adding a third outcome is a compile error in
`resolve` rather than a silent return to the empty window — planting
one yields a single E0004 there, and `adopt` is untouched.

The placard is bound to no file path, which is load-bearing rather
than incidental: `save` and `Ctrl+S` both refuse a document without
one, so a reflexive save cannot write the placard over the file
that failed to parse — a file that, being unparseable, is likely
the only copy of what its author was hand-editing.

The convention behind the choice is
[`CODE_CONVENTIONS.md`](./CODE_CONVENTIONS.md) §9: `expect` is for a
broken program precondition, and a user's malformed input is not
one.

### Event loop and `drain_frame`

The per-frame heartbeat: land one injected-input sample, re-derive the
rubber band, and sync the view with the document. It does **not**
render — the frame is drawn from winit's `RedrawRequested` (or the
headless driver's owed-frame check), never from the drain.

`InputHandlerContext::drain_frame(beat)` lands the injected sample and
runs the drain; `src/application/app/drain_frame.rs` holds the rubber
band (native-only, like the drag state it reads). The view's sync
(`MapView::sync`) is the one place the document reaches the screen: it
drains the change feed once, updates the projector, re-stamps the
selection tint and hands the journals to the layer renderer, with the
background and the status line. A pan or a zoom is a camera change and
applies nothing. The browser's frame
(`run_wasm/event_redraw_requested.rs`) runs the same sync.

### `MindMapDocument`

The app's handle on the open map: the document, and the app state that
is about it.

`src/application/document/mod.rs`. It holds the baumhard `Document`
(the elements, the writer, the history and the change feed), the file
it is bound to, the selection (a set of element ids), the open
transaction's handle and the status line a refused change left (D28),
and whether it is the [load-failure placard](#load-failure-placard).
What it does **not** own: the view of it (`MapView`,
`src/application/view/` — the projector and the glyph layers), the
renderer, drag and mode state, modal editors and keybinds; those are
on `InitState`.

### `InteractionMode`

What the user is doing at the canvas level. `Default` — a click
selects, a drag pans, Shift+drag draws a rubber band — and the two
target-picking modes, `Reparent` and `Connect` (native), which hold
their source nodes: the sources wear the view's source tint in place
of the selection's, the status line says what the mode does (as on
`main`), the pointer's valid target is tinted with the view's target
overlay, and the next click moves the sources under it or links them
to it. The resize and node-edit modes return with V1-C7
(#248) and V1-C8 (#249); until then their Actions answer `unhandled`,
naming their owner.

`src/application/app/interaction_mode.rs`.

### `DragState`

The drag state machine: `None` / `Pending` / `PendingRight` /
`Panning` / `SelectingRect`.

Mouse-down does not commit to a drag yet — the user might be clicking,
or might be about to drag. `Pending` holds the press's position and
what it hit (`ClickHit`: an element, or empty canvas, through
`Projector::hit`); `PendingRight` is its right-button counterpart.
Once movement crosses the drag threshold the state becomes `Panning`
or, with Shift on empty canvas, `SelectingRect` — the rubber band,
whose covered set is `Projector::nodes_in` of the rectangle and which
the selection highlight tints while it is out. Drags that move or
resize document elements return with V1-C7 (#248), on writer
transactions.

`src/application/app/mod.rs` (`enum DragState`), native-only like the
per-frame drain that drives it.

### `TouchGestureRecognizer`

The touch half of the pointer vocabulary, and `DragState`'s
peer rather than its consumer: a plain-value state machine
that turns raw `(phase, finger_id, position, now)` tuples into
a typed `RecognizedGesture`.

A finger is not a mouse, and winit's mouse synthesis
does not cover a hold, a second finger, or a drag that never
pressed a button. Without a recognizer the browser — the
surface `run_wasm/mod.rs` calls the *primary* one this
project targets — has no input path at all on a phone.
`Idle ↔ OneFinger ↔ TwoFingers` with four emit points:

- **`Tap`** on the lift of a finger that never left
  `POINTER_DRAG_THRESHOLD_PX` and was down for less than
  `LONG_PRESS_MS`.
- **`LongPress`** from `tick(now)` — "held for `LONG_PRESS_MS`" is
  a clock transition, not an event — once per episode.
- **`Pan { pos, delta }`** on every `Moved` from the threshold
  crossing onward. The first emission carries the travel since
  the finger *landed*, so summing an episode's deltas gives
  the finger's net displacement and the threshold's slop is not
  lost.
- **`PinchStep { center, pan, scale }`** while two fingers are
  down, whenever their midpoint or their separation has moved
  past the threshold since the last emission. Both halves ride
  every emission, because two fingers moving describe a
  translation and a scale at once; a parallel two-finger drag
  therefore reports steps whose `scale` multiplies to 1.0, and
  a symmetric spread steps whose `pan` sums to zero.

**No clock is read and no I/O happens inside it** — time is a
parameter — which is what makes every rule above provable on a
machine with no touchscreen (`TEST_CONVENTIONS §T9`). What
cannot be proved that way is the layer below: that a real
digitizer delivers the phases in the order the machine assumes.

**Two routes out, and the split is the point.** `LongPress`
resolves through `MouseGesture::LongPress` and the keybind
table, exactly as a mouse gesture does — it is the only touch
gesture that reaches the table. The other three carry no
`Action` at all: they take the two carve-outs CODE_CONVENTIONS
§3 already grants the mouse, the pre-funnel selection
bookkeeping a single click runs and the per-frame camera delta
a drag runs, through one cross-platform
`dispatch::apply_touch_effect`. That is why tap, pan and pinch
cannot be dead on one target: an `Action` can be `NativeOnly`,
and a `RenderDecree` cannot.

`Action::PanCanvas` is *not* on that path and could not have
been. It does not move the camera — it arms `DragState::Panning`
(`dispatch::native`'s `route_pan_canvas`), which is native-only,
so dispatching it from the browser returns `Unhandled` and warns.

`src/application/app/touch_gesture.rs` for the machine;
`dispatch/cross_dispatch/pointer.rs` for `drive_touch_event`
(recognize → route) and `apply_touch_effect` (run). Both
runtimes call both: `run_native.rs::dispatch_touch_event` and
`run_wasm/event_touch.rs`.

**What is native-only, and what that costs.** `LongPress`
ships bound to `EnterResizeMode`, which is `NativeOnly`, so on
the browser it dispatches, returns `Unhandled` and fires a
one-shot warn naming the remedy. That is deliberate rather than
pending: long-press is the touch peer of the keyboard's `r`,
and rebinding the browser's long-press to some unrelated
Compatible Action would make one gesture mean two different
things on two targets — a worse §4 outcome than a gesture that
is honestly unavailable and says so. Its parity rides on
`InteractionMode::Resize`'s chrome and handle-drag reaching the
browser. `TwoFingerDrag` used to sit beside it, bound to
`FastResizeStart` and equally dead there; two fingers now drive
the camera instead, and the variant is gone rather than
default-unbound, because a binding firing on the same event as
the camera step would be a conflict rather than a choice.

The resize mode and its handle drag went with the old model, and
V1-C7 (#248) brings them back as transactions — with them the route
a finger takes onto a handle, which keeps `FastResizeStart` a mouse
accelerator by design (#160). **An existing `keybinds.json` naming
`"TwoFingerDrag"` was silently accepted and never matched**, because
`KeyBind::parse` takes any non-modifier word; a retired name is now
refused at parse time through the `RETIRED_NAMES` table, with a
diagnostic saying what replaced it.

### `Renderer`

The GPU resource holder and command-buffer builder;
reads from the document, writes to the swapchain.

The `Renderer` is the view side of the
model/view split. It owns wgpu device, queue, surface,
pipelines, atlases, and the FPS ring buffer; every frame, its
`process()` reads document and scene state, builds command
buffers, and submits to the GPU. It never holds a reference to
the document.

`src/application/renderer/mod.rs` (`struct Renderer`).
A frame is its clear — the map's one background,
`Renderer::set_background` (V1-B12) — and glyphs, drawn by two
pipelines:
- **Glyph pipeline** — every visible character, via
  cosmic-text + glyphon atlas, in two passes: the canvas's, and the
  screen-space chrome's over it. The opaque backdrops behind the
  console and the color picker are glyphs too: a grid of full blocks
  (`█`, Public Pixel's, which is the whole em square) at the bottom of
  the chrome pass, laid out in the target's own pixels so the cells
  abut without a seam (`renderer/backdrop.rs`).
- **Positioned-glyph pipeline** (`glyph_runs/`) — every glyph of a
  [`GlyphRun`](#glyphrun) at its own canvas position, rasterized by
  cosmic-text into an atlas of its own (glyphon's cannot be reached
  from outside it) and drawn as one instanced quad per glyph, in one
  draw call per frame: the canvas trees' runs, then the glyph layers'
  ([layer renderer](#layer-renderer)).

Sub-passes sit in `console_pass.rs` and `color_picker.rs`; the
document's glyphs and the chrome layer's (the rubber band) are glyph
runs of the layer renderer. Visibility culling is
`Camera2D::visibility`, which answers the spatial cull and the
[`ZoomVisibility`](#zoomvisibility) window in one verdict.

### Layer renderer

The projection's [glyph layers](#projection--projector-glyphlayers-layerchanges-hit)
as the renderer keeps them: render state **per glyph element**, written
only where a journal says a layer changed (design §13.1, V1-C2 #243).

For each of the five layers it keeps a mirror of the layer's tree — a
slot per element, linked to its parent and its siblings — so the
layer's pre-order, which is the order its runs are drawn in, is a walk
of links, and an element entering, leaving or moving is a splice of a
few of them. A run's slot holds the run as the layer does: its shaping
and placement by `Arc`, so keeping it never shapes.

- **`load`** takes the layers whole, once per document: the only walk
  of a layer.
- **`apply`** takes the journals of what changed — the projector's
  `LayerChanges`, merged with those of the glyph mutators applied over
  them, from one or more projector updates — and visits exactly the
  elements they name, each once, the last touch deciding. One that left
  is dropped with its subtree, except what an earlier journal moved out
  of it; one that entered gets a slot; one that changed is compared with
  its slot and only a difference is written, into the buffers the slot
  already has (a drag re-places the same shaping; an unchanged run
  keeps its state); one whose place changed is spliced in after its
  previous sibling, and an element counts as in place only after a
  sibling no journal names. No other element is looked at.
- **`encode`** hands every run to the frame's
  [positioned-glyph pass](#glyphrun), in draw order: sections, frames,
  connections, labels, portals, each in tree order, after the canvas
  trees' runs and under the screen-space chrome. The pass applies the
  camera and each run's zoom window at draw time; `encode` takes
  `&self`, so a pan or a zoom applies nothing and writes no render
  state. The instances are built per frame from the kept runs, as the
  C1 decision record says: a glyph's rasterization depends on the zoom
  and on its pen's subpixel position.

`src/application/renderer/layers/`. The app's projector is wired to it
by V1-C3; until then the window and every capture draw it empty.

### `AppScene` and scene host

The screen-space overlay trees of the modals: the console and the
color picker, each a named slot (`OverlayRole`) holding a `GfxTree`.

The document's content is not here; it is the glyph layers the view
holds (§2, "Projection"). `src/application/scene_host.rs`
(`struct AppScene`) registers, replaces and removes an overlay's
tree, and answers which of the two update arms (§B2) a change takes:
`InPlaceMutator` when the recorded structural signature matches,
`FullRebuild` when it does not.

### Dirty flag

Whether the map has unsaved work: derived, not stored. The writer
compares its history with the position `Document::mark_saved` last
recorded, so an edit and its undo leave a map clean again.

`MindMapDocument::is_dirty`. `mark_saved` is called only by
`MindMapDocument::save_to`, after the file is written
(`test_no_production_code_sets_dirty_directly` holds that); the `open`
and `new` verbs refuse to replace a dirty document ("unsaved changes;
save before…"). It is not a render signal — the change feed is.

### Document revision

A counter on the document advanced by every change the writer accepts,
an undo and a redo included, plus the funnel that made the latest one
(`ChangeSource`).

An IPC agent polling for change, and the event stream that spares it
the polling ([IPC server](#ipc-server-ipcserver)), need a number. The
revision is the writer's (`Document::revision`). It restarts with each
loaded document, and the event stream reports a `generation` beside it
so a client can tell documents apart (`format/ipc.md`). A selection and
the camera advance nothing. The source of the last record on the
change feed is the `source` a `document_changed` event reports.

### FPS overlay

Two display modes for frame-time diagnostics:
snapshot (stable readout, re-sampled periodically) and debug
(live rolling average).

Performance-conscious development needs a
truthful FPS readout. The snapshot mode answers "what is the
steady-state frame rate?"; the debug mode answers "where are
the hitches?". The `fps` console verb (native) toggles between
them.

Embedded in
`src/application/renderer/mod.rs`. Both modes read
**wall-clock** deltas via `Instant::now()` stored in
`Renderer::last_frame_instant` — measuring render-body time
would lie under stress, because `render()` early-returns on
font-system lock contention and would collapse the reported
frame cost to near zero. The render-side plumbing
(`fps_display_mode`, `fps_overlay_buffers`, `set_fps_display`,
`tick_fps`, `RenderDecree::SetFpsDisplay`) compiles on both
targets; only the `fps` console verb is native-gated because
the console itself is. Browsers expose FPS via DevTools so the
WASM parity gap is cheap to leave.

### `FreezeWatchdog`

A native-only background thread that reads an
atomic timestamp pinged by the main loop and aborts the
process with a diagnostic banner if the main loop stalls past
threshold.

Mandala is single-threaded; an infinite
loop, a same-thread `RwLock` re-entry, or a blocking GPU call
would hang indefinitely with no actionable error. The watchdog
turns a hang into a fast, diagnostic crash. It is the only
background thread of a default launch — CODE_CONVENTIONS §3
additionally sanctions the [IPC boundary
threads](#ipc-server-ipcserver), which exist only under `--ipc` —
and the single-threaded invariant for the model/view pipeline is
preserved because the watchdog only *reads* a shared `AtomicU64`,
never touching app state.

`src/application/app/freeze_watchdog.rs` (`FreezeWatchdog`). Main
thread calls `tick()` at every event-loop boundary; the watchdog
reads the atomic every `WATCHDOG_POLL`; if the gap exceeds
`FREEZE_THRESHOLD`, prints diagnostics and aborts. Both constants
are declared at the top of that file. Not present on
WASM — browsers already provide an "unresponsive tab" dialog
for free.

### IPC server (`IpcServer`)

The opt-in local socket an external process — an LLM agent, a test
harness, `mandalactl` — drives the running app through, speaking
newline-delimited JSON (`format/ipc.md`).

The app's feedback loop for an agent: attach to an instance a human
launched (or one the agent launched itself), send commands, read
structured replies. Off unless `--ipc <path>` is passed; without the
flag no socket exists and no thread runs. The browser build speaks
the same protocol over a WebSocket toward `mandalactl bridge`, a
loopback relay, when the page is opened with `?ipc=<its app URL>`
(`ipc/browser/`, `app/run_wasm/ipc.rs`; `format/ipc.md` §Transport
"Browser"); `describe` says per command what each target serves.

`src/application/ipc/`. Boundary threads own the socket — a
persistent acceptor (one controller at a time; a second client is
refused immediately), plus a reader and an outbound writer per
controller — and exchange only protocol values with the main thread
over one ordered `mpsc` queue in and a bounded one out. They wake the
parked event loop through a winit user event (`IpcWake`) — or, with no
event loop under `--headless`, unpark the main thread — so an idle app
answers without polling; the loop body (`app::pump`) drains the queue
right after `drain_inputs` and executes every command on the main
thread, with the same access an input handler has. Each connection
carries a generation stamp, so nothing a departed controller sent can
act after it left. A controller that falls behind on its replies is
paused rather than dropped; one whose socket accepts nothing for
`STALLED_WRITE_TIMEOUT` (`ipc/transport/mod.rs`) is dropped. The
socket is created owner-only, refuses any
directory on its path that another user could rename it out of, and
replaces an existing path only when it is provably a stale socket of
ours.

Commands live in a registry (`ipc/registry.rs`), one module per family
under `ipc/commands/`, and `describe` renders that same table, so the
discovery surface cannot drift from the dispatch surface. Each command
runs inside a log-capture window (`baumhard::util::log::capture`), so
every `warn!` / `error!` it causes also reaches its reply's
`warnings`. The `act` family is the dispatch bridge: `act.action`,
`act.console` and `act.macro` reach `dispatch_action`, the console's
`execute_console_line` and the macro dispatcher's gated step loop
through `CommandTarget`, the app-side execution context. There is no
parallel control path, so undo, gates and side effects match the
keystroke each stands in for. The `query` family is the read half of
the loop: `query.document`, `.selection`, `.camera`, `.mode`,
`.status`, `.editors` and `.undo` return what a person reads off
the chrome. They go through `observe()` and `&self` reads on
`CommandTarget` (`document()` and `shell_state()`), so a query cannot
dirty the document, push undo or
ask for a frame. Each wire form is an exhaustive-`match` projection beside the state it
exposes (`document/ipc_state.rs`, `app/ipc_state.rs`), not a mirror
struct, so a new `InteractionMode` or `DragState` variant breaks the
build until it is given one. The `scene` family is what is drawn
rather than what is known: `scene.dump`, `.visible_set`,
`.node_geometry`, `.hit_test` and `.find_text` read the view
(`view::inspect`: the projector's geometry and the glyph runs) — rects
in both spaces, the cull verdict, and the displayed text — and name
each element by its integer ref. `scene.hit_test`
runs the canonical click ladder on the app side
(`CommandTarget::hit_test`) rather than a copy of it. The `input`
family is the hand on the mouse and keyboard, below the Action
funnel: `input.mouse_move`, `.mouse_button`, `.wheel`, `.key`,
`.text` and `.touch` become the payloads winit events carry and are
delivered to the same `on_*` entry points those events
reach, so the drag state machines, double-click timing and modal
steals cannot tell them apart. `input.drag` paces its samples one per
frame through the injected-input queue (`ipc/injected.rs`), so the
drag's frame runs between its moves as under a real hand;
`input.click_node` is a move and a click at a node's center, and
`input.pending` reports how many injected samples are still queued,
so a client polls it to zero before reading what a drag did. The
`capture` family is what is drawn as pixels: `capture.screenshot` is
a thin wire over the screenshot engine (see
[Screenshot capture](#screenshot-capture)), which the `screenshot`
console verb calls too.

The `events` family is the push half: `events.subscribe` opts in
per class, and a connection starts with none, so a polling agent is
never sent a frame it did not ask for. Nothing in the app emits an
event. An `ipc::events::Tracker` is handed a read-only `Observed`
view (document, camera, interaction mode) after every command and,
while anything is subscribed, once per loop iteration, and reports
what differs from the last look; the document's revision is what
makes "the model changed" an integer comparison. So a keystroke, a
drag, an animation commit and an agent's command are seen alike, and
a 200-step macro is one `document_changed` (`verbose` observes
between its steps through `MacroDispatchTarget::step_finished`).
Events queue on the main thread, leave behind the replies of their
drain, never pause execution, and under a slow reader coalesce to
the newest per class. Every reply carries `revision` (advances once
per observation with something to report, view changes included)
and `document_revision`.

**Vision:** the families after `capture` (clock) each add one
module and one line to the registry's family table; recording
(IPC-08) and headless mode (IPC-10) draw through the same
parameterized frame the screenshot engine does.
**Caveat:** native only — a Unix socket, or a named pipe on Windows
(#169) — and the browser has no socket at all (`CLAUDE.md`
"Dual-target status").

### Screenshot capture

One offscreen frame, drawn by the code that draws the window, read
back and written as a PNG with a geometry sidecar mapping its pixels
to node ids — the visual half of "inspect the app as well as a human
does", for agents (`capture.screenshot`) and people (`screenshot`).

A screenshot has to be the window's frame, not a lookalike of it, and
it has to be truthful about the frames that were not drawn. The
renderer's frame is therefore split into a CPU half
(`Renderer::prepare_frame`) and a GPU half (`Renderer::encode_frame`),
parameterized over the camera that projects the canvas and the
texture it lands in; the window is one caller and the capture
(`renderer::capture`) another, drawing into a texture of the
surface's own format and reading it back in row bands that fit the
device's buffer limit. At the default parameters the PNG is the
window, pixel for pixel. Resolution is the camera's
[`pixel_ratio`](#camera2d-and-cameramutation) rather than its zoom,
so a supersampled capture shows the same elements the window does.

`src/application/capture/` holds the engine: `plan` turns a request
(size, framing — the view, a region by the aspect rule, or `fit` —
and scale) into the one `Camera2D` the frame is drawn through,
refusing what cannot be drawn before anything is allocated;
`artifacts` is the bounded, owner-only, per-process dir a
server-chosen capture lands in, removed on exit; `geometry` is the
sidecar, read off the view (the projector's geometry) at the
capture's camera; `png` encodes. `app/capture_flow.rs` is the app
half both surfaces call: it syncs the view, then the layer renderer
draws the glyph layers through the capture's camera
(`LayerRenderer::encode`) — the same layers the window draws, at any
zoom, with nothing projected again. Every caller-chosen path is written through
`guarded_file::publish` — atomic, no symlink followed, no file
replaced unasked — the writer the IPC `to_file` sidecar uses too.

A frame the renderer skips (`FrameSkip`: not started, font system
contended, glyph prepare failed) is reported as not rendered and
writes no file. An open modal (console or color picker) is laid out
against the window, so a capture of another size leaves it out
(`ModalLayer::Omitted`), and so does the console verb, from its own
picture; the IPC command at the window's size keeps it.

**Caveat:** native-only — the readback blocks on a buffer map, which
a browser cannot do (`CLAUDE.md` "Dual-target status"; browser
canvas capture is IPC-15's). IPC-08's recorder and the
[headless driver](#headless-driver) draw through the same prepare /
encode pair; with no window, a capture is the only frame ever drawn
into a texture.


### Frame-sequence recording

A screenshot proves a state; a recording proves a *behavior* — an
animation's timing, a drag, a hover transition. `capture.record_start`
and `capture.record_stop` (and the `record start` / `record stop`
console verb) take a sequence of the very frames `screenshot` takes
and a `manifest.json` that says when each was taken; `mandalactl
record assemble <dir>` turns them into an animated GIF — the format
GitHub shows inline in a PR — or an MP4 through the host's `ffmpeg`.
The app links no encoder: the frames and the manifest are the whole
contract, and the assembler is a client that reads a directory.

`src/application/capture/record.rs` is the engine's pure half.
`Timeline` decides from the elapsed time alone when a frame is due and
how many boundaries a late heartbeat stepped over, never reading a
clock itself, so one timeline paces both a realtime recording and a
stepped one, which `clock.step` drives by stopping the
[app clock](#app-clock) on each boundary (a stepped boundary rounded
up to a whole millisecond, the unit the clock and the animation tick
read); `Recording` holds the
frames, the gaps and the manifest. `app/record_flow.rs` is the app
half: it starts a recording, takes a frame each time `about_to_wait`
finds one due (an active recording is a loop timer source, so an idle
canvas still wakes for its frames), and ends it. Each frame is drawn
by `capture_flow::draw`, the screenshot's own path, and held as raw
pixels until `PENDING_BYTES` of them wait or the recording ends, so
a frame costs the loop a render and a readback and never an
encode. A frame
that was due and not taken — the loop was late, the renderer skipped
it, a node drag made it undrawable — is a *gap* with its reason, never
a silent drop.

A recording is app state, not session state: it outlives the client
that started it, and one that ended on its own (its `max_frames`, the
artifacts dir's budget, the app exiting) is kept until
`capture.record_stop` collects it. In `mandalactl`, `assemble.rs`
plans the GIF from the manifest's times — rounding start times rather
than delays, so the error does not accumulate, and dropping frames
closer than `GIF_MIN_DELAY_CS` (the shortest delay browsers honor
rather than stretch) — and writes one decoded frame at a time.

### Headless driver

The app with no window, no display and no event loop (`--headless`):
the same app state, the same frame and the same IPC surface, run on a
pump of its own instead of winit's — so a CI job or a cloud agent with
no X server and no GPU can launch, drive and screenshot it.

What made it a second *driver* rather than a second app is that the
app's reach into its window was two calls wide — asking for a frame
and setting the cursor icon — and both now go through `Shell`
(`app/shell.rs`), which is a window or nothing. The body winit's
`about_to_wait` ran is one function, `pump::iterate` (`app/pump.rs`),
that both drivers call and that returns *when the next iteration
should run* as a value: winit turns it into a `ControlFlow`, the
headless driver (`app/run_headless.rs`) into a park the IPC threads
end with `Thread::unpark`. Where winit delivers `RedrawRequested`, the
headless driver draws the frame the redraw ledger says is owed
(`app/settle.rs`), paced to one per `HEARTBEAT_MS` of wall time
(`FRAME_INTERVAL` in `run_headless.rs`), and runs another iteration,
as winit does after any event. A headless frame
draws nothing (`renderer::Sink::Headless`): pixels are drawn when a
[capture](#screenshot-capture) asks. The clock starts virtual, the
input is the IPC `input` family's alone, and the renderer's adapter is
chosen with no surface by the same plan every bootstrap runs
(`renderer/adapter.rs`: preferred, then software; `--adapter software`
for software only; wgpu's `WGPU_*` variables). It ends gracefully on
`Action::Quit` (`act.action Quit` over IPC), which either driver honors
through the same shutdown a closed window runs. A one-shot launch
(`--screenshot <path>`) captures once the app has settled and exits,
under either driver — `1` with nothing written for a map that did not
load, a frame that cannot be drawn, or an app that ends first.

`format/ipc.md` §"Headless and one-shot launches" is the user-facing
contract; `work_plans/LLM_IPC.md` §D10 the design. **Caveat:**
native-only, with nothing owed to the browser, whose page always has
its canvas (`CLAUDE.md` "Dual-target status").

### App clock

The one time axis everything the app *does* by time reads —
animations, the drag pause on them, double-click and long-press timing,
an `OnClick` trigger's start, a recording's frames — with a virtual
mode in which it advances only when stepped. `AppClock` in
`src/application/clock.rs`; `wall_ms` beside it is the wall clock
what *measures* reads.

A reproducible run, a screenshot of an animation's middle, a test of a
double-click at exactly its window all need time to hold still and
move on command, while the FPS overlay and the freeze watchdog
must keep measuring real cost. So the old
single `now_ms()` bridge became two names, and every reader was
dispositioned to one of them (`work_plans/LLM_IPC.md` §D9 has the
table). The clock is threaded through `InputHandlerContext` and
`InputContextCore`, so a test's clock is an ordinary value. Switching
modes never moves the reading — virtual time starts where real time
stood and real time resumes where virtual time was left — so nothing
stored on the axis is ever rebased. Driven by the IPC `clock` family
and the `clock` console verb; `clock.wait` is its settling primitive
(`format/ipc.md` §`clock`).

Native and the browser each hold one; the browser's stays real. What
it lacks is not the tick — since #144 the browser's frame runs the
same view sync and, like native's loop frame, advances on a real
clock only, so it would stand still under a virtual one — but the
step driver (`heartbeat::step`, a method on native's
`InputHandlerContext`) and the IPC transport to command it
(`CLAUDE.md` "Dual-target status", "Virtual-clock stepping"). Both
read the wall clock through `web_time`, which is `performance.now()`
in the browser.

---

### Action dispatch

Every user-driven application-level effect is a variant
of `enum Action` (`src/application/keybinds/action/mod.rs`) and runs
through a single `dispatch_action(action, ctx, hit)` funnel
(`src/application/app/dispatch/native.rs`). Mouse, keyboard, the future
macro runtime, and any plugin host all reach the same arms.

Before this funnel existed, mouse gestures
(double-click create-orphan, double-click open-editor, middle-click
pan, wheel zoom) were hardcoded inside event handlers and bypassed
the keybind system entirely. Users couldn't disable, rebind, or
replace them without recompiling. The funnel reifies every gesture
as an Action so one vocabulary covers keys, mouse, macros, and
plugins.

`KeyBind` (`src/application/keybinds/bind.rs`)
accepts mouse-shaped binding strings —`DoubleClick`, `MiddleClick`,
`RightClick`, `LeftClick`, `LeftDrag`, `WheelUp`, `WheelDown` —
alongside keyboard names. Mouse handlers synthesize the gesture's
canonical name via `gesture_key_name(MouseGesture::*)` and feed it
through the same `ResolvedKeybinds::action_for_context` lookup as
keyboard input. Lookup → `Action` → `dispatch_action(action, ctx,
Some(&DispatchHit { click_hit, canvas_pos }))`.

**Resolution order** (any binding):
1. `keybinds.action_for_context(...)` — built-in `Action` variants.
2. `keybinds.macro_for(...)` — user-defined macros, loaded from
   `~/.config/mandala/macros.json` on native. See
   `crate::application::macros` for `Macro`, `MacroStep`,
   `MacroRegistry`, and `dispatch_macro`. Steps fan out to
   `dispatch_action`, the edit step (V1-D1), or
   `execute_console_line` depending on `MacroStep` kind, so plugin
   authors and macro recorders share one runtime path. That path is
   `run_macro_steps`, the gated step loop `dispatch_macro` calls
   after its lookup. IPC's `act.macro` runs an anonymous step list
   through it too, so there is one privilege gate however a macro
   arrives. **Unknown
   macro id falls through** to the edit tier, so a typo'd or
   half-loaded macros file doesn't swallow the keystroke.
3. `keybinds.edit_for(...)` — edit bindings, which fire nothing until
   V1-D1 (#250) rebuilds edits.

**Built-in Actions win on collision.** A key combo bound to both
`Action::Copy` (in `copy: ["Ctrl+C"]`) and a macro on `"Ctrl+C"` in
`macro_bindings` runs the Action — the macro never gets a chance.
To override a built-in Action with a macro, first unbind the
Action's keybind (set `copy: []`) and then bind the macro. Same
applies for a built-in vs. an edit binding.

**Macro privilege model.** Macros are tagged by their loader tier
(`SourceTier { App | User | Map | Inline }`); the dispatcher
fail-closes on tier-restricted surfaces (`ConsoleLine` and
destructive / I/O `Action` variants). Authoritative threat model
+ surface enumeration + tier-by-tier permissions:
[`format/macros.md`](./format/macros.md). An effect a map can
carry that does I/O is gated at every site that applies it
(CODE_CONVENTIONS §3).

**Dispatch status per gesture.** `DoubleClick`, `MiddleClick`,
`LeftDrag`, `WheelUp`, `WheelDown` are dispatched through
`dispatch_action` from their respective handlers. `LeftClick` and
`RightClick` are reserved tokens — the parser accepts them so user
configs don't fail validation, but no handler currently looks up
an Action for them. A single left-press is already consumed by the
selection state machine; wiring `LeftClick` would need a clear
post-selection dispatch point. `RightClick` has no non-color-picker
dispatch site at all.

**`LeftDrag`** is the continuous "press + movement past threshold
on empty canvas" gesture (default `PanCanvas`). The threshold
cross dispatches whatever Action the gesture resolves to — no
`PanCanvas` special-case in the handler — and the `PanCanvas` arm
sets `DragState::Panning` for the press duration, unless the state
it finds owes a commit (see the guard under
[`DragState`](#dragstate); a `pan_canvas` binding on a plain key
is why the guard is on the arm). The per-frame
pan delta stays inline in `event_cursor_moved.rs` because
per-cursor-move state is legitimately not a discrete-action
concern; the threshold frame's first delta — measured from the
press, like every promoted gesture's — is gated on the dispatch
having actually entered `Panning`, so an Action rebound onto
`LeftDrag` doesn't get a free camera nudge.

**Modifier-fallback for mouse gestures.** Mouse handlers resolve
through `ResolvedKeybinds::action_for_gesture`, which tries the
exact `(key, ctrl, shift, alt)` binding first and falls back to
the unmodified `(key, false, false, false)` binding if no exact
match exists. Modifiers on mouse gestures are typically decorations,
not distinct bindings — pre-branch `Ctrl+Wheel` zoomed exactly the
same as a bare `Wheel`, and the fallback preserves that. Users who
*do* want `Shift+DoubleClick` to mean something different just
bind it explicitly.

**Default-off `CreateOrphanNodeAndEdit`.** Empty-canvas double-click
ships unbound. Users opt back in via:

```json
{ "create_orphan_node_and_edit": ["DoubleClick"] }
```

---

## §6 The authoring surface

Authoring surface concepts are the parts a user actually touches:
modal editors, the console, keybinds, the color picker, clipboard,
and (briefly) the `maptool` and `mandalactl` CLIs. Most are native-only today; the
parity story for each is honest.

### Modal editor ladder

The steal / release shell the map's inline text editors will sit in.
The node text editor and the single-line editor were deleted with the
old model and return with V1-C8 (#249), on the writer; until then the
console is the one modal that steals the keyboard, and the color picker
claims only its own keys.

### Glyph-wheel color picker

A modal HSV picker rendered as a hue ring of `HUE_SLOT_COUNT` glyphs
with sat/value crosshairs and a central preview glyph.

`src/application/color_picker/mod.rs` (`ColorPickerState`,
`PickerMode`, `OpenPicker` in `state.rs`) and
`src/application/color_picker_overlay/`. Native-only today.
`compute_color_picker_layout()` is a pure function over geometry +
viewport, so layout can be unit-tested without GPU. What the picker
writes — its previews and its commit — is V1-C7's (#248), on writer
transactions: the widget is kept, and its commit and hover paths are
not wired until then.

### Console

A CLI-style command palette (`/` by default, the `open_console`
binding) for mutations, styling, settings, and document
operations.

Power-user operations that don't have a
keybind. The console covers the long tail: zoom-bound
authoring, font-size clamps, palette swaps, mutation listing
and application, FPS toggle. Tokenized shell-style
(whitespace-split, `"quoted"` preserves spaces, `key=value`
first-class). Tab-completion is contextual and prefix-matched;
scrollback shows command history with dimmed older lines.

`src/application/console/mod.rs` (`ConsoleState`, `ConsoleEffects`;
the line executor, `execute_console_line`, sits with the native
shell in `app/console_input/exec.rs`). Native-only today. The verbs
are the
`COMMANDS` table in `console/commands/mod.rs`, in its order: `help`,
`anchor`, `body`, `border`, `canvas`, `cap`, `color`, `edge`, `font`,
`fps`, `spacing`, `label`, `mode`, `mutation` (with `list`, `help`,
`apply`, `inspect` subverbs), `save`, `screenshot` (see
[Screenshot capture](#screenshot-capture)), `record` (see
[Frame-sequence recording](#frame-sequence-recording)), `clock` (see
[App clock](#app-clock)), `open`, `new`, `node`, `section`, `zoom`.
Visuals draw a rounded box-drawing frame; content is clipped via
`grapheme_chad::truncate_to_display_width` so wide CJK
characters never overflow.

### `Grammar` — the declarative console verb spec

**Every verb is one declaration, and one engine reads it.**
`console::spec` holds `Grammar` / `Subverb` / `Key` / `Slot` /
`Form` / `Vocabulary`, and `Command` carries a
`&'static Grammar` and nothing else: the usage forms `help`
prints, the `keys:` block under them, the search tags, the
completion popup, the kv parse loop and every hint all derive
from it. Adding a kv key is one table row — parse, complete,
help and hint follow.

A `Grammar` is a **level**, not a verb. `border` is one,
`border preview` is another, `canvas section-frame focused` a
third, each joined to its parent by a `&'static` reference on
`Subverb::child`. `subverb_sets` and `key_sets` are slices *of
slices*, so `canvas border` names border's fifteen keys and its
seven positional subverbs (five in the `per-field` group, `side`
and `corner` in `glyphs`) rather than transcribing them; `section
frame` names the same fifteen and adds its own `section=`.

Four things the shape is load-bearing for:

- **`Vocabulary` answers three questions from one
  declaration** — the rows a popup offers, the `<…>` a usage
  form prints, and the word list an error can quote back.
  `Rows` covers both the document-derived vocabularies (a
  map's palettes, a node's sections, the compiled font families)
  and the *suggestion* lists too open-ended to print: `zoom
  min=` offers the presets in `zoom.rs`'s `VALUE_PRESETS` and
  accepts any positive float, so the popup gets the presets and
  the usage line gets `<zoom|unset>`.
- **A `Form` is (slots, keys), and a subverb may declare
  several.** `section move` takes `dx=`/`dy=` *or* `x=`/`y=`;
  `section resize` takes the `fill` literal *or* `w=`/`h=`.
  `help` prints one line per form, and the popup offers the
  union of the forms the line's positionals still admit —
  which is what puts `fill` beside `w=` and `h=` at
  `section resize <TAB>` and takes both away again at
  `section resize fill <TAB>`.

  The engine enforces the exclusion as far as the *slots*
  express it, and no further. `fill` sits in a slot only one
  form declares, so typing it rules the other form out and
  `section resize fill w=99` is refused by name and pointed at
  the shape that reads `w=`. `section move`'s two forms differ
  only in their *keys*, so an empty positional list admits
  both equally; deciding between them would mean letting
  whichever key was typed first pick the form. That exclusion is
  the verb's own, a handwritten guard with a message naming both
  shapes — one of the two bespoke semantics #27's fix plan names as
  staying behind the table — which is `section move`'s body's
  (`console/commands/section`).
- **No verb sees a raw token index.** `spec::descent` is the
  only reader of token order. A handler asks
  `descent.subverb()`, `descent.parent_name(0)` and
  `descent.slot_value(args)`. Eight completion sites used to
  do their lookahead with `tokens.get(N)` while the arms they
  fed were keyed on the *positional* index, and the two
  disagree the moment a kv pair sits earlier on the line. The
  positional-vs-kv discriminator lives there too, declared on
  the subverb (`Subverb::gated`) rather than re-asked at each
  slot that emits the vocabulary.
- **A kv the matched form does not read is refused by name.**
  `kvs::read` asks per *form* — narrowed by the positionals
  already on the line, per the bullet above — rather than per
  level, and points the key at the form that does read it:
  the level's composed form for `border preset heavy
  color=#fff`, another shape of the same subverb for
  `section resize fill w=99`. Before the declaration both
  staged what they matched and discarded the rest without a
  word, the first identically on four surfaces.

What stays hand-written is value parsing and mutation: a `Key`
declares its name, its sentence and its vocabulary, and what
`padding=8` *means* is the verb's. Bespoke semantics —
`section move`'s mutual exclusion, the `border side`
non-custom-preset gate — are handwritten handlers *behind* the
table.

*Which* handler a matched subverb runs is declared too, through
`spec::route`: each dispatcher keeps a `Routes` table pairing
every declared name with a variant of its own route enum, and
matches on the variant. Before it, dispatchers matched
`subverb.name` and most let `_ =>` absorb the last name, so a new
`Subverb` row was right on every derived surface and ran the
catch-all's behavior: `fps snapshot` would have toggled the
rolling average (#155). The match is now exhaustive without a
catch-all, and because every route enum is private to its
dispatcher with one payload-free variant per row, a variant whose
row is deleted is dead code, which the clippy gate refuses. A
registry test holds every table against the subverbs its levels
declare, in both directions, so a row with no handler fails the
suite naming the subverb. A level composing a shared set — the
border family's per-field subverbs — hands it on through a
`Delegates` row, which is data the registry test reads too, and
the shared set's own table is held against the set.

`console::spec`'s own tests hold the declaration against itself
over the whole registry, in both directions: a form naming a
key its level does not declare, and a key no form prints. The
second is what would have caught `font range=`, `color range=`
and `color section=` on the day each was written — each was
parseable, named in its verb's own rejection, and documented
nowhere.

They also hold the *engine* against the declaration, over the
same registry rather than a sample of it: for every form of
every subverb at every level, the line that reaches it is built
from the level's `label` and the form's own required slots,
handed to the real completion engine, and checked for the keys
that form names. Two more read the sources rather than the
tables, because a verb can decline to consult them: no verb
spells out a `usage:` line of its own (`label` did, two keys
behind its grammar), and no hint names a value in a `|`
alternation that the vocabulary beside it rejects
(`border color`'s two hints said `preset`, which the slot
refuses, and omitted `accent|edge|fg`, which it accepts).

The differential oracle (`console::tests::oracle`) is what made
the migration reviewable: `EXEC_CORPUS`, `COMPLETION_CORPUS`
and `EXEC_PREFIX_CORPUS` pin execute outcomes and popup
contents byte for byte, so a change that alters no pinned row
is behavior-preserving by construction and a row that does move
is a decision with a reason attached.

**A console line owes no rebuild.** Whatever a line changes reaches
the document through its writer, and the view's next sync projects
exactly that from the change feed; a line that only reads changes
nothing, and nothing is projected for it.

Console parity on WASM is the obvious next step;
the verb implementations are already cross-platform, only the
modal shell is native-gated.

### Keybinds and `Action`

A three-layer pipeline: abstract `Action` enum →
parsed `KeyBind` → resolved table; with cross-platform
configuration via XDG (native) and `localStorage` (WASM).

Every keystroke that does *anything* maps to
an `Action` first; the `Action` is then dispatched in the right
input context (`InputContext`: Document, Console, ColorPicker,
LabelEdit, TextEdit, NodeEdit). This indirection means users can
rebind keys without
touching code, and the same `Action` works on both targets even
though the config-loading paths differ.

`src/application/keybinds/`. The three
layers:

- `Action` enum (`action/mod.rs`) — high-level intents:
  `Undo`, `CreateOrphanNode`, `EnterReparentMode`,
  `EnterConnectMode`, `DeleteSelection`, `EditSelection`,
  `OpenConsole`, `Copy`, `Paste`, `Cut`, `ExitMode`, etc.
- `KeyBind` parser (`bind.rs`) — string syntax like `"Ctrl+Z"`
  → modifier mask + key code.
- `ResolvedKeybinds` (`resolved.rs`) — fast `O(1)` lookup
  table built from `KeybindConfig` at startup.

`Action::context()` returns the input context the action
belongs to; the event loop uses it to filter eligible actions
based on which modal is open. Native config: hardcoded defaults
+ `$XDG_CONFIG_HOME/mandala/keybinds.json` + optional
`--keybinds <path>` CLI override. WASM: same defaults +
`localStorage["mandala_keybinds"]` — there is deliberately no
`?keybinds=` layer, because a query param is owned by whoever
composed the link rather than by the user (see
`keybinds/platform_web.rs`). Partial configs merge via serde
`default` attributes.

**The config surface is declared once.** A bindable Action used
to be written out three times — a `KeybindConfig` field, a
`Default` entry, a `resolve()` row — and a field missing from
`resolve()` compiled, leaving a binding that deserialized,
round-tripped and did nothing. `keybinds/surface.rs`'s
`keybind_surface!` collapses the three into one row per Action
in `keybinds/config.rs`, generates the struct itself, and emits
an `ActionKind → BindSurface` match that the loader's
recognized-key set is built by walking. Because that match is
exhaustive, an `Action` variant that appears in no section of
the table — neither bound nor listed under `unbindable` with a
reason — fails to compile. Unit variants take the string-list
shape (`"undo": ["Ctrl+Z"]`) and payload variants the `args`
shape; which one is not a choice, since each section only
accepts the variant shape it is for. Unrecognized top-level keys
in a user's file are warned about (`"keybinds: unrecognized key
…"`, naming the nearest recognized key when there is one) and
skipped rather than rejected — one stale *key* must not cost the
user the rest of their bindings — and keys starting with `_` are
treated as comments, which is how the shipped
`config/default_keybinds.json` carries its instructions. A
*malformed value* under a recognized key gets the same posture one
level down: the file is parsed field by field (a loop generated from
the same table), so that key is warned about and keeps its default
while every other binding loads; a key written twice uses its last
occurrence, also with a warning. Only text that is not a JSON object
costs the layer. `user_config::salvage` holds the primitives, and
`macros.json` and `mutations.json` use them per entry.

**Parametric Actions.** A subset of variants carries payload
(`String` paths, `(field, value)` tuples, etc.) — these wrap
parameterized console verbs so a user can bind e.g. `Ctrl+B` →
`SetFrameField { field: "fit", value: "repeat" }` directly
in `keybinds.json` without authoring a macro. Bindings use a
sibling `ParametricBinding` shape:

```jsonc
{
  "set_frame_field": [
    { "combo": "Ctrl+B", "args": ["fit", "repeat"] }
  ],
  "set_color": [
    { "combo": "Ctrl+1", "args": ["text", "#fafafa"] },
    { "combo": "Ctrl+2", "args": ["ring", "var(accent)"] }
  ],
  "set_font": [
    { "combo": "F8", "args": ["size", "14"] }
  ],
  "set_zoom": [
    { "combo": "F12", "args": ["min", "0.5"] }
  ],
  // `clear_zoom` carries no payload, so it takes the plain
  // string-list shape every unit Action takes.
  "clear_zoom": ["Shift+F12"]
}
```

Color / font / zoom carry the axis as the first arg (`text|border|ring`,
`size|weight|italic`, `min|max` respectively) so a single binding-list
covers the whole field group. The typed `ColorAxis` / `FontSlot` /
`ZoomBound` enums on the Action variant make the dispatcher's
match exhaustive without a fan-out guard, and they reach the
payload through `surface::ArgValue`: a `String` field takes the
argument verbatim, a typed field goes through its strum `FromStr`,
and a payload type with no `ArgValue` impl does not compile.

Each variant names its arg shape in the table row that declares
it — the payload field names there are both the arity and the
list quoted back at the user when a binding's `args` array is
the wrong length. **Their order is the positional `args`
contract**, and the language does not enforce it on its own: the
generated constructor is a struct expression, which does not
care what order its fields are written in, so a row whose names
are transposed would compile and hand the user's arguments to
the wrong fields.

Two mechanisms hold that line, pinning different halves of it.
The build holds the *names*:
`mandala_derive::PayloadFieldNames` publishes each variant's
field names in declaration order into `action_payload_fields`,
`surface::keybind_field_order_check!` compares the row's names
against them under `const _: () = assert!(…)`, and a
transposition is an `error[E0080]` naming the row. Two
independent sources — the declaration and the table — so it is
not a mirror agreeing with itself, and a brand-new row is
covered with nothing written by hand. The tests hold the
*values*: the sentinel table in `keybinds/tests.rs` carries the
`Action` each row's args must produce, cannot omit a row, and
drives the args end to end through the JSON, the `ArgValue`
conversion and `resolve()` — which is where "`set_color`'s first
arg has to be a real `ColorAxis`" lives, and which the const
check, reading two lists of identifiers, cannot see.

Tuple rows (`SetPathContent(glyph)`) sit outside the const
check by construction rather than by omission: their names are
local bindings the table invents, and the pattern that binds
them and the constructor that consumes them are the same macro
repetition, so there is no second order to disagree with.

Three skips are possible at resolve time and all three are
logged, never panicked: an unparseable combo, the wrong number
of args, and an arg that is not a valid value for its field.
The editing Actions among them answer `unhandled` until their
owners rebuild them on the writer (`app::dispatch::families`). Filesystem
variants (`OpenDocument`, `SaveDocumentAs`, `NewDocumentAt`) are
`NativeOnly` and denylisted from non-User macro tiers per the
privilege gate.

### User-tier config loading — `check_cap`, `read_capped`, `load_layered`

The three user-owned JSON files (`keybinds.json`,
`mutations.json`, `macros.json`) are all found the same way, so
the finding is written once in
`src/application/user_config/`.

- `MAX_USER_PAYLOAD_BYTES` and `check_cap` are the single wording
  and enforcement of the size cap. A real config is a few KB; one
  large enough to exceed the cap is accidental or hostile and is
  rejected before serde ever sees it.
- `read_capped(path)` is the native filesystem read: stat, cap,
  read. Native-only, like its neighbor `xdg_mandala_path` — the
  filesystem tier of a user config exists only on desktop.
- `load_layered(label, layers, parse)` is the fallback walk over
  an ordered list of `ConfigLayer`s. Each layer is a name plus a
  lazy fetch; the first layer whose payload fits the cap *and*
  parses wins. An absent layer is skipped silently; a broken one
  is logged and the walk continues; exhausting the list returns
  `None` and the caller substitutes its defaults. Nothing here is
  platform-specific, so the precedence logic is unit-tested on
  native even for the browser's layers.
- "Broken" means the parser found nothing to salvage: the parsers
  stand on `salvage` (`parse_object` / `parse_array`, `take_field`,
  `parse_entry`), which drop a malformed field or entry with a
  warning naming it and fail the layer only when its container is
  not usable JSON of the right outer shape.
- Each target names its own layers exactly once, in the
  composition wrapper that sits on the driver:
  `desktop::load_desktop_layered(label, filename, explicit, parse)`
  for the explicit CLI path before the XDG path, and
  `web_storage::load_web_storage_only(label, key, parse)` for
  `localStorage[key]` alone — deliberately no URL layer, since a
  `?<name>=<json>` param belongs to whoever composed the link. All
  six platform loaders (three configs × two targets) are now a
  filename or storage key and a parser.
- The one deliberate asymmetry lives in the desktop wrapper: only
  the XDG layer is filtered on `exists()`. An absent user config
  is the normal case and stays silent, whereas an explicit
  `--keybinds <path>` that does not resolve is a user error worth
  a warning. Changing that is a change to one function.

Adding a fourth user-tier config file is a matter of naming its
filename and storage key, then handing each
wrapper a parser — no new read, cap, layer, or fallback code.

### `SourceTier`

The `App < User` ladder in `src/application/source_tier.rs`, which
the macro registry is keyed by. The ladder means two things at once:
**precedence** (`User` overrides `App` on an id collision, which the
derived `Ord` encodes) and **trust** (`App` ships with the binary and
`User` is the user's own file; a map's own verbs are V1-D3's). The
macro dispatcher's privilege gates —
`allows_console_line`, `allows_action`, both `impl SourceTier`
blocks in `src/application/macros/mod.rs` — key off exactly that
trust split. Tier assignment is loader-pinned: nothing in an
on-disk file can raise its own tier.

### Clipboard

The OS clipboard layer: native backed by `arboard`, WASM stubbed
pending async-clipboard integration. It carries plain text. Beside it,
the document holds the app's own clipboard: the text the OS was given,
and, for a copied section, its spans and style, which a paste uses
while the OS still holds that text. On the browser, the app's
clipboard is the only one.

`src/application/clipboard.rs`: what a copy, a cut and a paste take
from and give to each kind of selected element — a section, part of
one, a node, a path, a label, a portal end — each cut and paste one
request on the document's writer. Failures on native are silent by
construction — an interactive path must not panic, and a clipboard
that is merely unavailable is not worth a log line per keystroke — and
the WASM stubs say so at `log::debug!`.

### `mandalactl` CLI

A separate binary in `crates/mandalactl/` that
drives a *running* app over its `--ipc` socket from the shell:
`mandalactl <family.verb> [args...]`, one connection and one command per
invocation, the reply printed as `path: value` lines (`--json` for the
envelope as received). The one verb that keeps the connection is
`events.subscribe`, since a subscription lives only as long as its
connection: it prints each pushed event until `--count` of them, or the
app exits.

The shell is an agent's native tool, and hand-built JSON over `nc` is the
fragile incantation it replaces. `name=value` arguments are typed from the
command's declared parameter types, so `x=412` is a number and `text=412` a
string; a command newer than the build is typed from the app's own
`describe`, so the client needs no release to call it. `--wait-settled`
issues the command and then `clock.wait until=settled`. Every wait has a
deadline and a message naming what to suspect, and the exit code is the
contract: `0` answered, `1` the app said no, `2` the app could not be
reached or the request did not go out whole (nothing ran), `3` a bad command
line, `4` the request was sent and no answer was read (it may have run: read
the app's state before repeating it). `--wait-settled` first checks that the
app serves `clock.wait`, so a missing wait is reported before the command
runs rather than after it succeeded; a wait that fails once the command
succeeded is a `4` that says so. A refusal for `busy` (the single-controller
slot) is retried briefly, because two invocations in a row can find the
previous controller not yet noticed as gone; a refusal for any other reason
is reported as it is.

`verbs::VERBS` is the table of commands the build knows, for `--help` and for
bare (positional) arguments. The app crate's tests hold it against the IPC
registry in both directions, so a server command without its verb fails the
PR that adds it. The sibling of `maptool`, not a mode of it: `maptool` reads
`.mandala.json` files and never talks to a process. Its one client-side
command, `record assemble <dir>`, turns a recording's frames and manifest
(see [Frame-sequence recording](#frame-sequence-recording)) into a GIF, or an
MP4 through the host's `ffmpeg`, and never opens the socket.

`crates/mandalactl/`. Protocol: [`format/ipc.md`](./format/ipc.md); the full
agent-facing reference is `mandalactl --help`.

### `maptool` CLI

A separate binary in `crates/maptool/` for scripted operations on
`.mandala.json` files. Its verbs (`show`, `grep`, `apply`, `export`,
`verify`) are rebuilt as clients of the document by V1-E1 (#253); until
then every invocation says so and exits `2`.

---
