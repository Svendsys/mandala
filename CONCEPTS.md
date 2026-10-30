# Mandala & Baumhard — Conceptual Building Blocks

*A reference for the named concepts that make up this project.*

---

## On this document

Mandala is a mindmap application; Baumhard is the glyph-animation
library it is built on. They are one project
([`CODE_CONVENTIONS.md §1`](./CODE_CONVENTIONS.md)). Together they
have accumulated a vocabulary — `GlyphArea`, `MutatorTree`, `Channel`,
`Portal`, `ZoomVisibility`, `ThrottledInteraction`, `CustomMutation`,
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
- [§3 The mindmap domain](#3-the-mindmap-domain)
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

### `Tree<E>`

An arena-backed forest of typed nodes plus the element type's
per-tree derived state, representing one layer of content.

A `Tree` is how Baumhard stores anything
hierarchical that needs to render or be hit-tested: one tree for the
mindmap nodes, one for connection glyphs, one for borders, one for
the console overlay, and so on. Every node in a tree is reached
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
structural command detached and a journal still owns; `update` refuses
those.

Defined in `lib/baumhard/src/engine/tree.rs`. Wraps
`indextree::Arena<E>`, read through `arena()`; adds `root: NodeId`,
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
rest alone. This is the seam custom mutations
([§4](#4-the-mutation-framework)) ride on.

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
costs work in the anchors and their rows, never in the rest of the
tree. `apply_to(&mut target, &seed)` is `apply_at` anchored at the
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
in the same walk. The state lives on the walker's call stack — never
on an element, never in a tree, no allocation per visited node. The
glyph trees inherit nothing (`Inherited = ()`, seed `&()`).

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
— a `GlyphArea`, a `GlyphModel`, a `GlyphRun`, a `Fill`, or a `Void`.

This is *the* tree-node type in the codebase.
All visual things — every text region, every composed-glyph
shape, every shaped TextFlow, every node background, every
structural padding node — is one variant of this
enum. Shared metadata rides on every variant: a `channel` for
mutation routing, a `unique_id` assigned by the host app (Mandala
uses it for the mindmap node id), a `flags` set, and an
`event_subscribers` list. What a glyph tree derives from its
elements — the AABB memo and each node's subtree AABB for the BVH
hit-test — lives in the tree's `GlyphTreeCache`, keyed by `NodeId`,
never on an element.

`lib/baumhard/src/gfx_structs/element.rs`.
`GlyphArea`, `GlyphModel`, `GlyphRun` and `Fill` each box their
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
color/font overrides ([`ColorFontRegions`](#colorfontregions)), a
background fill, an optional outline halo, a hit shape, and a
zoom-visibility window.

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
a path. Overrides do not move them. `zoom` is evaluated at render
time, never by projection.

The run changes through `RunSet` (the whole payload, what projection
writes), `RunOrigin`, `RunOpacity` and `RunOverrides`; interpolation
blends `origin`, `opacity` and the override vectors and snaps the
shaping. State comparison (`same_state`) compares the shaping by
`Arc` identity. Introduced in V1-B6 (#240).

A run in a canvas tree is drawn by the renderer's positioned-glyph
pass (V1-C1, #242; design §13.2): every glyph of the run's one shaped
buffer at its own placed position — upright, with its shaper offsets
kept, its overrides applied and the run's opacity in its alpha — from
an atlas the pass holds, in one instanced draw call per frame for
every run there is. Nothing is shaped to draw it, and a pan or a zoom
touches no run. No projection emits runs yet (V1-B7, C3).

`lib/baumhard/src/gfx_structs/run.rs`; the mutations are
`GfxMutation::Run*` in `mutator.rs`; the pass is
`src/application/renderer/glyph_runs/`.

### `Fill`

A node's background as a glyph element: a rectangle, the shape it
takes in it (a rounded rectangle, or the inscribed ellipse), a color,
a soft `fade` at its edge, an opacity and an optional zoom window.

The rectangle is `flow::Rect`, the one the node's frame runs around,
so the fill and the frame share one outline (design §13.4). A fill is
hit by its rectangle, ellipses included. It changes through `FillSet`
(projection's), `FillColor`, `FillOpacity` and `FillRect`;
interpolation blends the rectangle, the color, `fade` and `opacity`
and snaps the `shape` and `zoom`. Introduced in V1-B6 (#240); the
renderer's shader fill that draws it is V1-C2.

`lib/baumhard/src/gfx_structs/fill.rs`; the mutations are
`GfxMutation::Fill*`.

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
(`()`), so for them the key is the channel. In the mindmap domain,
`MindNode.channel` is where this surfaces to end users; see
[§3: Channels](#channels-mindmap-level).

Every parent's children are stored in key order (the sorted-children
invariant of [`Tree<E>`](#treee)); siblings with an equal key keep the
order the builder added them in, which for mind-nodes is **Dewey-id
order** (`id_sort_key`). [`MapChildren`](#instruction) pairs strictly
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
`Flag` is `Serialize` / `Deserialize` and reachable from a
`.mindmap.json` through the `GfxElementField::Flag(Flag)`
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
how one custom mutation can sweep a whole subtree without
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
imperative `Command`), plus `Event` (subscriber dispatch), the `Run*`
mutations that change a `GlyphRun` and the `Fill*` mutations that
change a `Fill`. It
implements the engine's `Mutation<GfxElement>`; its builder template
is a list of `CellField` slots (`gfx_structs/template.rs`).

`lib/baumhard/src/gfx_structs/mutator.rs`.
Each variant boxes its payload to keep the enum compact. Type
mismatches (e.g. `ModelDelta` applied to a `GlyphArea`) are
silently ignored per the [`Applicable`](#applicablet) no-panic
rule; a run or fill mutation at an element of another kind is
ignored and logged the same way, in every build. `RunSet` and
`FillSet` are projection-only and never authored: they are
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

A pluggable hit-test shape — `Rectangle` or
`Ellipse` — shared between the renderer SDF and the BVH
descent.

A node's visual silhouette and its clickable
silhouette must agree. `NodeShape` names the two today
(rectangle, ellipse) and gives both pipelines one source of
truth for "is this point inside?". Adding a new shape is one
enum variant, the match arms the compiler then demands, and a
`SHAPE_*` constant plus a `case` arm in the WGSL shader;
`shape.rs`'s own module header is the recipe. What makes it safe
to follow is that the arms are not remembered: four `match`es over
`NodeShape` in `shape.rs` are exhaustive — `shader_id`,
`style_spellings`, `contains_local`, `intersects_local_aabb` — so a
planted variant is four E0004s, and once those are answered a
fifth lands in the mandala crate, `wgsl_shape_const_name` in
`renderer/tests.rs`. The shader half is not a `match` and is
forced by a test instead:
`test_every_node_shape_has_a_matching_wgsl_constant_and_case_arm`
reads the WGSL as text and fails on a variant with no constant, a
constant carrying the wrong id, and a missing `case` arm.

`lib/baumhard/src/gfx_structs/shape.rs`.
`contains_local` does point-in-AABB or point-in-ellipse
(normalized coordinates, `nx² + ny² ≤ 1`); degenerate bounds
always return `false`. `intersects_local_aabb` supports
rect-select with conservative approximation for ellipses.

The format's shape vocabulary (`KNOWN_SHAPES`, published in
`format/enums.md` and enforced by `maptool verify`) is wider
than the variant set: `"hexagon"`, `"diamond"`,
`"parallelogram"` and `"rounded_rectangle"` are canonical but
have no shader case yet. `ShapeSpelling` is the pure
classifier that separates those from a genuine typo —
`Rendered` / `KnownNotYetRendered` / `Unrecognized` /
`Unspecified` — and `is_author_error` / `is_quiet_fallback`
are the two predicates it exposes for the reporting
decision. Before it existed, every hexagon in the demo map
warned on every load (issue #118).

`ShapeReport` is the second half of the split: the *routing*
as a value. `ShapeSpelling::report` composes the two
predicates into `Some(UnknownSpelling)` /
`Some(RectangleSubstituted)` / `None`, `ShapeReport::level`
is the single definition of which `log::Level` each carries,
and `from_style_string` does nothing but pick the literal
macro per arm — literal because `log`'s release compile-out
folds on a level the compiler can see, which a
`log::log!(computed, …)` would defeat. The value being a
value is what makes the decision ordinary testable data;
what is left, arm-to-macro, is a fact about source text and
`shape_tests.rs`'s `log_routing` reads it as one, holding
`from_style_string`'s body to a whitelist rather than
searching it for `log::` calls.

`KNOWN_SHAPES` is restated five times across three files —
three lists in `format/enums.md`, `LEGACY_SHAPE_ORDINALS` in
`crates/maptool/src/convert/enums.rs`, and the `shape_type`
ordinal table in `format/migration.md` — and each is pinned
back by a test that reads the restatement. The first four
derive their expectation from the constant directly; the
ordinal table is pinned to `LEGACY_SHAPE_ORDINALS` instead,
which is itself pinned to the constant, so the chain closes
in two hops rather than one. Either way, widening the
vocabulary is still one edit plus the copies the suite names
for you. The table in `shape.rs`'s `KNOWN_SHAPES` doc is the
index.

Shape-aware borders (glyph-drawn frames that follow
the ellipse outline, not just the AABB) wait on the
[`GlyphBorderConfig`](#border-geometry) side; the primitive
surface here is ready.

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
honor it — no script, no custom mutation, just two fields.

`lib/baumhard/src/gfx_structs/zoom_visibility.rs`.
Two `Option<f32>` fields, a `contains(zoom) -> bool` predicate;
the cost is two float comparisons and nothing else — no
cosmic-text reshaping or buffer-cache
invalidation fires on zoom steps. At the mindmap layer the
surface is two flat fields (`min_zoom_to_render`,
`max_zoom_to_render`) on `MindNode`, `MindEdge`,
`EdgeLabelConfig`, and `PortalEndpointState`; see
[`format/zoom-bounds.md`](./format/zoom-bounds.md) and
[§3: Zoom bounds](#zoom-bounds).

`NaN` zoom is treated as "not visible" deliberately —
a `NaN` camera is a bug upstream, and culling the frame surfaces
it faster than carrying the `NaN` through the glyph pipeline.
Inverted windows (`min > max`) render as "always hidden" at
runtime; `maptool verify` flags these as authoring errors.

The seam waiting here is **zoom-triggered LOD
mutations**: a `CustomMutation` bound to a zoom threshold could
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

The mindmap canvas is one tree; connection
glyphs are another; the console overlay is another; the color
picker overlay is yet another. `Scene` collects them all,
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

The recursive dispatch engine (a set of functions, not a type) that walks a
`MutatorTree` against a `Tree` and applies matched mutations.

Every mutation that a mutator tree delivers goes through the
walker. The walker aligns children by key (or by position, depending
on instruction): both rows are sorted by key, so it walks the two
sibling lists in step — the lower key advances, equal keys walk — with
no `Vec` and no sort. It recurses, and dispatches each payload to
`Mutation::apply_to` with the target's parent's
[inherited state](#mutatortreee-m). A target whose key a mutation
changed is recorded and moved to its sorted place when the walk is
over, so no walk sees a half-sorted row. Cost is `O(sum of matching
pairs)` — pruned branches are free.

`lib/baumhard/src/engine/walker.rs`, generic over the element type
and the payload — no `dyn`, no `Any`, no runtime type checks.
Key functions: `walk_tree_from` (the pair entry), `align_child_walks`
and `merge_rows` (the key-based pairing), `process_instruction_node`
(the loop/spatial/map dispatch), `default_terminator` (the
continuation that resumes key alignment after a `RepeatWhile` exits),
and `Walk` (what a walk keeps beside the two trees: the children whose
key it changed, and the journal a journaled walk records into). Every
apply goes through one function, `apply_recorded`, which is where a
journaled walk records its before-images. Branchless enough that
matching-pair cost dominates.

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
authored as JSON: the variant is skipped by serde.

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
render bounds, scale, line height and background color, a model's
position, a run's origin, opacity and per-glyph overrides, and a
fill's rectangle, color, fade and opacity; a run's shaping and zoom
window and a fill's shape and zoom window snap.

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
[custom mutations](#4-the-mutation-framework) attach to.

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

### Animation primitives — `AnimationTiming`, `Easing`, `Followup`

The vocabulary for motion: a serializable timing
envelope on a mutation, an easing curve, and a reserved slot for
what happens after a run completes.

A "grow font" that snaps is fine; one that
animates reads better. Rather than a general scheduler, motion
here rides on the mutation that causes it — an authored
`CustomMutation` carries `timing`, and the runtime blends between
a pre-mutation and a post-mutation snapshot instead of executing
a separate program.

`lib/baumhard/src/mindmap/animation.rs` for the serializable
half (`AnimationTiming`, `Easing`, `Followup`, and the `lerp_*`
helpers); `src/application/document/types.rs` +
`animations.rs` for the app-side per-playback record and tick,
which live there because the snapshot type (`MindNode`) and the
completion commit do. See [§4](#animation-timing) for the
authoring surface and
[`format/animation-roadmap.md`](./format/animation-roadmap.md)
for what is wired versus dormant.

A second, generic vocabulary — `AnimationDef` / `AnimationInstance`
/ `Timeline` / `TimelineEvent` in `core/animation.rs` — was
deleted in #41. It had no implementers of its `Mutable` bound, no
callers, and an `update` signature that took its instance by value
and returned nothing, so it could not have driven a tick as
written. The long-form scheduling it stood for is the `Followup`
slot's job when that lands.

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
mutator trees from this vocabulary and nothing else; the writer that
applies them with history is V1-B4.

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
  projection's job.
- **Glyphs are never rotated.** The unit of placement is the grapheme
  cluster, so a mark never leaves its letter. A cluster of advance `w`
  and line height `h` takes a step `|cos θ|·w + |sin θ|·h` along the
  path, θ the direction of the chord the step spans: side by side along
  a horizontal stretch, a line apart along a vertical one, blended on a
  curve and turning smoothly through a sharp corner — the way ASCII art
  makes one. Around an outline each cluster is pushed outward by half
  its extent, so text sits on the outline rather than across it.
- **Outlines** (`outline`, `offset`, `ring_offset`): a node's
  rectangle shaped as a rounded rectangle (radius a percentage of half
  the shorter side) or the inscribed ellipse; ring *k* runs around the
  outline grown past the rings inside it. Lines and circular arcs are
  walked exactly by arc length, ellipses and Béziers through a table
  whose points are evaluated on the curve itself.
- **Fit modes** say what happens when text and path differ in length:
  `Once` (with an optional filler, appended to the run in the flow's
  style), `Repeat` (reusing the run's glyph indices), `Spread` and
  `Fit` (rescaling the run), the last two solved so the last cluster
  ends at the path's end. Text too long is cut to the longest prefix,
  in reading order, that fits — a right-to-left run keeps its
  beginning — and never overlaps.
- **Hit mapping.** Every glyph names its span and its bytes of that
  span's text (`GlyphMetrics`, `ShapedBox::glyph_spans`); a filler
  glyph names `FILLER_SPAN`. No paragraph separator but a box's `\n`
  is drawn or breaks a line.

`lib/baumhard/src/flow/`. It caches nothing, builds no glyph tree and
never takes the font lock: every shaping function takes the guard its
caller holds, which it takes after `fonts::init` has run.

---

## §3 The mindmap domain

The mindmap domain is the world of `.mindmap.json` — the on-disk
format and its in-memory mirror. It lives in
[`lib/baumhard/src/mindmap/`](./lib/baumhard/src/mindmap/) and is
documented schema-side under
[`format/`](./format/). The format references are authoritative
for field-by-field detail; this section is conceptual.

### `MindMap`

The document root: nodes, edges, canvas configuration,
palettes, custom mutations.

Everything a user can save and reload is here.
The `MindMap` is a plain serializable struct — no derived state, no
runtime caches. The loader deserializes it from JSON and carries any
key no field claims along untouched, so the save writes it back
([preserved unknown keys](#preserved-unknown-keys)); the
[canvas-role projection](#canvas-role-projection) and
[tree builder](#tree-builder) turn it into renderable form;
mutations transform it in place.
Helper methods (`children_of`, `all_descendants`,
`is_hidden_by_fold`, `is_ancestor_or_self`, `resolve_theme_colors`)
walk the data on demand rather than caching.

`lib/baumhard/src/mindmap/model/mod.rs`. The shape is `version` and
`name`, a flat `HashMap<String, MindNode>` (keyed by Dewey id), a
`Vec<MindEdge>`, a [`canvas: Canvas`](#canvas), a `palettes:
HashMap<String, Palette>`, `custom_mutations: Vec<CustomMutation>`,
an opaque `macros: Vec<serde_json::Value>`, and the two
`#[serde(skip)]` carriers a load fills and a save writes back —
`unknown_keys` and `skipped_constructs`. See
[`format/legacy-schema.md`](./format/legacy-schema.md) for the JSON surface and
[`format/README.md`](./format/README.md) for a minimum-viable
example.

### Preserved unknown keys

Every object in `.mindmap.json` is **open**: a key no field claims
neither fails the load nor disappears from the file. The loader warns
once, carries the key on the map, and writes it back at the next save.

The reason is version skew, in both directions at once. A map authored
by a newer build carries keys an older build has never heard of;
refusing the document leaves the reader with an empty window, and
Mandala is an editor — it loads the whole map, mutates it, and writes
the whole model back — so ignoring the keys deletes them at the next
save. Keeping them is what lets an older build open, edit and resave a
newer map without destroying the newer features. A typo
(`"min_zoom_to_rendr"`) is kept the same way, and named the same way,
because a key you still have is a key you can still fix;
[`maptool verify`](#maptool-cli) is where it becomes a nonzero exit.
The warning names the part of the document that carries the key —
`node "1.2"`, `edge[3]`, `palette "coral"` — rather than a byte
offset.

Mechanically it is **not** a per-type `#[serde(flatten)]` catch-all,
and could not be: `serde_json::Value` implements neither `Eq` nor
`Hash`, and the graph reachable from a load is full of types that
derive `Copy`, `Eq` or `Hash` (`Color`, `OrderedVec2`, `Position`,
`Anchor`, `Range`, `GlyphMatrix`). A catch-all on only the types that
can hold one would preserve keys in some places and drop them in
others. Instead `mindmap::unknown_keys` wraps the one parse a load
already pays for and records the route to every key serde handed to
`deserialize_ignored_any` — which covers the whole graph, including
types the source walk cannot see. What the model owes the mechanism is
only that no type absorbs a key first: no `deny_unknown_fields`, no
`flatten`, no `untagged` or `tag`. That set is not written down
anywhere — a hand-kept list is the twin surface
`lib/baumhard/CONVENTIONS.md` §B4 warns about — so
`lib/baumhard/src/util/serde_coverage.rs` walks baumhard's own sources
with `syn` and
`loader::tests::test_no_loadable_type_can_swallow_an_unknown_key`
fails the moment one appears.

**A derivation is a reading, and a reading can be wrong in a way the
document it feeds is wrong in too.** The same walk produces the list
of arrays `format/legacy-schema.md` publishes — the places a captured key's
route stops naming a node by its id and starts naming an element by
its index — and for a long time it produced that list by matching the
token `Vec` and one spelling of `#[serde(rename …)]`. Three live
arrays were missing as a result: `ColorFontRegions::regions` is a
`BTreeSet`, and `MutationListSrc::Literal` and
`GlyphModelField::GlyphLines` are newtype-variant payloads with no
field name. The document said the same three things, because it was
written by reading the walk, so nothing disagreed
([#122](https://github.com/Svendsys/mandala/issues/122)).

The walk now resolves a field's type through the index — a `type`
alias, a newtype struct, any of the eleven container spellings serde
writes as a growable array, and the bare slice `[T]` that `Cow<'a,
[T]>` bottoms out on. Four of those eleven do not keep the order the
file was written in at all: `HashSet` and `FxHashSet` iterate by hash,
`BTreeSet` by the element, `BinaryHeap` by its heap. That is a
*stronger* reason to publish them than a `Vec`, not a weaker one — a
save re-sorts them, so an index into one can move without anybody
having edited anything. The walk also reads every spelling serde
accepts for a member name, including the `deserialize` arm of a
list-form `rename`, a container `rename_all`, and the extra spellings
an `alias` admits. But the fix that matters is that the
walk is no longer the only witness.
`lib/baumhard/src/util/serde_probe.rs` asks the **generated**
`Deserialize` impls the same questions by handing them a
`Deserializer` that answers every request with the emptiest value
that satisfies it and records what was asked — no source text, no
`syn`, no list, and `serde_derive`'s own expansion as the authority
on both member names and array-ness.
`unknown_keys::tests::test_the_derived_positional_arrays_survive_an_independent_derivation`
and its member-name sibling fail when the two disagree. Neither is a
replacement for the other: the probe cannot see a serialize-only
proxy, and the walk cannot see through a generic instantiation. Their
disagreement is the signal. Both see `#[serde(alias = "…")]` and both
publish it — the derive writes a field's aliases into the same
`FIELDS` list as its name and a variant's into `VARIANTS` — because a
file may write any accepted spelling and a captured route is recorded
against whichever it wrote.

Openness is about **keys, not meanings**. An `edge_type` the renderer
does not know still loads — open vocabularies stay open — and semantic
violations (an edge pointing at no node, a `color_schema` naming a
palette that is not there) are [`maptool verify`](#maptool-cli)'s
business. The interiors of `macros` / `inline_macros` are deliberately
opaque and are never reported. Three legacy spellings are the
exception and still refuse the load: a top-level `portals`, per-node
`text` / `text_runs`, and a non-array `sections` are names the current
model means something else by, and each has a `maptool convert` verb.

`lib/baumhard/src/mindmap/unknown_keys.rs`,
`lib/baumhard/src/mindmap/loader.rs`,
`lib/baumhard/src/util/serde_coverage.rs`,
`lib/baumhard/src/util/serde_probe.rs`. See
[`format/legacy-schema.md`](./format/legacy-schema.md) §"Unknown keys are kept" for
the policy as authors read it, and
[`format/validation.md`](./format/validation.md) for the split between
what the loader reports and what `verify` fails on. Its severe
counterpart is [skipped constructs](#skippedconstruct--skippedconstructs),
below.

### `SkippedConstruct` / `SkippedConstructs`

One construct a load could not read at all, lifted out of the document
so the rest of the map opens, and written back untouched at the next
save. `SkippedConstructs` is the ordered collection of them, carried
on `MindMap` beside `unknown_keys` and with the same `#[serde(skip)]`:
they go back at their own routes, not into a side object.

The problem is the acute form of the one
[preserved unknown keys](#preserved-unknown-keys) solves. A key from a
newer build is **inert** — nothing reads it, so ignoring it changes
nothing about what the map does. A **variant** from a newer build is
the opposite: it *is* the instruction. `{"mutator": {"Glow": …}}` used
to make the whole document unloadable, so opening a newer map in an
older build gave an empty window and the newer feature was one
accidental save away from gone.

**The unit is the whole construct**, never the part inside it that
failed, and that is the load-bearing design decision. Dropping one
`Mutation` out of a macro would leave a custom mutation that still
appears in `mutation list`, still fires, and now does two of the three
things it says it does — a silent partial behavior with nothing to
see. So the unit is the nearest container whose absence reads *as*
absence: a whole `custom_mutations[i]`, a whole node
`inline_mutations[i]`, or a whole node-or-section
`trigger_bindings[i]`. Nothing else is skippable — a node, an edge,
the canvas or a palette this build cannot read still fails the load,
because a map missing part of itself with no sign of which part is
worse than no map.

**Refused for any reason, not only an unknown variant.** Sorting
serde's message into "from the future" and "a typo" would mean parsing
that message — the twin surface `lib/baumhard/CONVENTIONS.md` §B4 is
about — and it sorts on the wrong axis anyway: the load's question is
"can I carry this out?", and the answer is no in both cases. So the
load skips what it cannot read, warns saying what serde said and that
nothing the construct describes will run, and `maptool verify` reports
every one as an `unknown_variant` violation with a nonzero exit. The
two questions — *can I open this?* and *is this file right?* — stay
separate.

Ordering matters at save: captured keys are spliced back **before**
skipped constructs, because a key's positional route was resolved
against the array with the constructs already lifted out. See
`loader::to_json_value`.

`lib/baumhard/src/mindmap/unknown_keys.rs` (the types),
`lib/baumhard/src/mindmap/loader.rs`
(`load_skipping_unreadable_constructs`,
`excise_unreadable_constructs`),
`crates/maptool/src/verify/unknown_keys.rs`
(`check_skipped_constructs`). See
[`format/legacy-schema.md`](./format/legacy-schema.md) §"Unknown keys are kept" and
[`format/validation.md`](./format/validation.md) §"Unknown variants".

### `Canvas`

The per-map shared rendering context: background
color, default node and connection styles, live theme-variable
map, named theme presets.

Some things are per-map rather than per-node:
the canvas background color, the defaults nodes and edges fall
back to when their fields are absent, the `var(--name)` theme
variables colors reference, and the presets theme-switching
mutations copy into those live variables. `Canvas` is that
shared state. It sits on `MindMap` directly (`canvas: Canvas`)
and is consulted at scene-build time for defaults and theme
resolution.

`lib/baumhard/src/mindmap/model/canvas.rs`. Fields:
`background_color`; four optional default records —
`default_border` (nodes), `default_connection` (edges),
`default_section_frame_border` and
`default_focused_section_frame_border` (the section-frame chrome,
see [`SectionFrameElement`](#sectionframeelement-and-section-frame-chrome));
`theme_variables: HashMap<String, String>` (live
values), `theme_variants: HashMap<String, HashMap<String,
String>>` (named presets). The
[`SetThemeVariant`](#document-actions) document action copies
a preset into the live map;
[`SetThemeVariables`](#document-actions) patches individual
entries.

### `MindNode`

One node — position, size, style, layout hint,
palette binding, channel, trigger bindings, and one or more
[`MindSection`](#mindsection)s carrying the text content.

The unit of content. Each node renders as a
shape with one or more text-bearing **sections** inside,
optionally framed by a glyph border, and participates in the
parent-child tree through its `parent_id`. Post-section refactor
the node owns visual chrome (background, frame, shape, border,
shadow) and structural pieces (`channel`, `color_schema`,
`trigger_bindings`, `inline_mutations`, zoom-bounds); the
user-typed text lives on its sections.

`lib/baumhard/src/mindmap/model/node.rs`. Full
field reference in [`format/legacy-schema.md`](./format/legacy-schema.md). Author
owns non-overlap of node AABBs; the model does no collision
checking. The tree builder excludes folded subtrees from the
display tree; the underlying data persists either way. The node
container materializes as a chrome-only `GfxElement::GlyphArea`
in the runtime tree, with the section subtree appended as
children — see [tree builder](#tree-builder).

### `MindSection`

A positioned text-bearing surface inside a
`MindNode` — the post-section data shape's home for `text` and
`text_runs`. Every renderable node has at least one section.

The user-facing strata of data. A node is a
*container*; a section is *what the user typed*. Sections give
the architecture room to grow per-stratum styling, per-stratum
mutations, and per-stratum interaction without making the node
itself bigger. For migrated maps the typical shape is one default
section per node (offset `(0, 0)`, fills the parent); authors who
want multiple strata of data on one node opt in by appending
extra sections.

`lib/baumhard/src/mindmap/model/node.rs` (`MindSection`).
Plain data — `text`, `text_runs`, `offset`, optional `size`,
optional `channel`, its own `trigger_bindings`, and an optional
`frame_border` for the per-section frame chrome. In the runtime
tree each section becomes a
`GfxElement::GlyphArea` child of the owning node's container area,
plus a structural `GfxElement::GlyphModel` grandchild that exists
as a future per-component-mutation seam (the renderer skips it).
The renderer's tree walker shapes each section-area into its own
`cosmic_text::Buffer` keyed by `unique_id`, so multiplicity falls
out for free. Loader rejects pre-section maps with a concrete
pointer at `maptool convert --sections`. Full reference:
[`format/sections.md`](./format/sections.md).

### `MindEdge`

A directed connection between two nodes — line-mode
or portal-mode — with style, optional label, and optional
per-endpoint state.

Edges carry both hierarchical structure (when
their `type` is `parent_child`) and arbitrary cross-links (when
`type` is `cross_link`). They render as either a path of glyphs
along a Bézier curve (line mode) or a pair of small markers, one
at each endpoint (portal mode). A line-mode edge can have a
single text label sitting along the path; a portal-mode edge has
two endpoint records, each with its own text and styling.

`lib/baumhard/src/mindmap/model/edge.rs`. Edges
have **no stable id** — they are identified by the tuple
`(from_id, to_id, edge_type)`
([`CODE_CONVENTIONS.md §3`](./CODE_CONVENTIONS.md)). The
`display_mode` field switches rendering style without changing the
underlying edge identity; flipping a long edge from line to portal
is a one-field change. Field reference in
[`format/legacy-schema.md`](./format/legacy-schema.md).

Multiple edges between the same pair with different
`type` are allowed (rare but legitimate). Multiple edges with the
*same* tuple are a duplicate and a validation error.

### Dewey-decimal IDs

Dot-separated hierarchical node IDs (`"0"`, `"1.2"`,
`"1.2.3"`) that encode tree structure in the key itself.

Reading a `.mindmap.json` reveals the tree shape
in the keys. IDs sort as numbers segment-by-segment (`"1.10"` after
`"1.9"`, not before), and `derive_parent_id` recovers the parent
without pointer chasing. The format is human-friendly and
diff-friendly — exactly the sort of place where opaque UUIDs would
have ended in the same byte count and zero readability.

`lib/baumhard/src/mindmap/model/mod.rs`.
`id_sort_key` extracts the last segment for sibling sort;
`derive_parent_id` strips it. Fresh IDs are minted by
`fresh_child_id` in `src/application/document/topology.rs`
without reusing deleted gaps. Full reference:
[`format/ids.md`](./format/ids.md).

IDs do **not** cascade on runtime reparent — when
node `"1.2"` moves under `"0"`, it stays `"1.2"` and `parent_id`
becomes the truth. They *do* cascade on delete-with-orphan-promote.
This trade keeps reparent cheap; `maptool verify` flags drift.

### Channels (mindmap level)

The `MindNode.channel` field — the user-facing
surface of the Baumhard routing tag.

Authors tag siblings with channels to opt them
into selective mutations. A `CustomMutation` whose mutator targets
channel 1 hits only siblings tagged 1; siblings tagged 0 are
skipped. Multiple siblings can share a channel (broadcast group),
or each can be unique (per-sibling targeting). All existing maps
default to channel 0 and behave as if the field did not exist.

Stored as `usize` on `MindNode`; preserved
through tree builder onto the corresponding `GfxElement.channel`,
which orders a node's children in the glyph tree; consulted by the
walker's [key alignment](#channel) at walk time. Full reference:
[`format/channels.md`](./format/channels.md).

A `TargetScope::ChildrenOnChannel(n)` variant is the
named extension waiting on this field — it would let a mutation
declare "children whose channel is 1" without an inline predicate.

### Palettes

Map-level named color schemes; nodes reference them
through `color_schema { palette, level, … }` rather than carrying
colors inline.

The legacy miMind format stored full palette
data on every node; the testament map alone duplicated the same
~225 palettes across nodes. Hoisting palettes to the document
level is a 100× reduction in file size and turns "rethemes the
whole map" into a single edit. Each palette is an array of
`ColorGroup`s indexed by depth; a node's `level` is which group
it pulls from. Level-clamping (last group when out of range) makes
deep subtrees degrade gracefully.

`lib/baumhard/src/mindmap/model/palette.rs`, with the cascade
itself in `model/theme.rs`. A node's binding lives in its
optional `color_schema` field, a `ColorSchema` record with
`palette: String` (the key into `map.palettes`),
`level: usize` (which `ColorGroup` to pull from), two
flags — `starts_at_root` (does level 0 apply to the schema
root or to its children?) and `connections_colored` (do edges
leaving this node inherit the palette stroke color?) — and
`overrides`, a `ColorOverrides` record of four optional strings
naming this node's own exceptions to the group.
`resolve_theme_colors` on `MindMap` does the lookup;
out-of-range `level` clamps to the last group rather than
failing, and `starts_at_root: false` leaves the schema root
itself unresolved so its own `style` stands. Validation
requires every referenced palette to exist with at least one
group.

The cascade is **override first, palette second, `style`
third**, and the
projection passes read it through four sibling helpers —
`node_background_color`, `node_frame_color`,
`node_text_color`, `node_title_color` — plus
`edge_theme_stroke_color`, which supplies the themed tier of
[`MindEdge`](#mindedge)'s color cascade for an edge whose
source node sets `connections_colored`. That helper reads
`node_frame_theme_tier` — the same one the border ladder
reads — so an `overrides.frame` reaches an edge exactly as it
reaches the node's own frame, and an empty frame is a hole on
both. Anything more specific
than the node still wins: a `TextRun` naming its own color, a
`border.color` override, a per-edge `glyph_connection.color`.

The `overrides` tier is what makes the theme *editable per
node*. A direct "make this one green" is more specific than an
inherited theme and has to win, and it cannot land in `style`:
`style` is the tier the palette shadows, and every migrated
node carries baked `style` colors that are stale copies of its
own theme, so a `style` write would report success and change
nothing on screen. `set_node_bg_color` /
`set_node_border_color` / `set_node_text_color` therefore write
`color_schema.overrides` on a themed node and `style` on an
unthemed one, through one shared
`set_node_color_channel`; `UndoAction::EditNodeStyle` carries
`before_color_schema` so undo puts the node back on its
palette.

A tier you can write is only half a tier: each of the three
setters takes an `Option`, and `None` — what `color bg=reset`
and the empty string on a non-fill channel both resolve to —
*drops* the override so the group shows through again, rather
than storing the authoring literal that would pin the node off
its theme forever. On an unthemed node there is nothing below
`style`, so the same `None` names the authoring default
instead. Full reference:
[`format/palettes.md`](./format/palettes.md).

Animated palette transitions are the seam — the data
shape is already mutation-friendly; the runtime would need to
interpolate `ColorGroup` fields on a clock.

### Text runs

Non-overlapping styled character ranges within a
node's text — bold, italic, underline, font, size, color,
hyperlink.

A single node can have rich text without being
fragmented into multiple nodes. Text runs are the mindmap-side
surface that the renderer translates into `ColorFontRegions`
spans for shaping. The user-visible effect is a per-span
override: emphasis on the first word, a colored annotation in
the middle, a link at the end — all on one node.

`lib/baumhard/src/mindmap/model/node.rs` (`TextRun`).
Each run carries `start` and `end` — the only two keys a file must
write — plus `bold`, `italic`, `underline` (default off), `font`
and `color` (strings; the empty string means the section-level
default), `size_pt` (default `DEFAULT_TEXT_RUN_SIZE_PT`, declared
beside the struct) and `hyperlink: Option<String>`. The struct
fields are not `Option`s: every styling field defaults to the value
that renders like the section's own text, so `{"start": 0, "end":
5, "bold": true}` is a complete run. Indexed by **grapheme
clusters** — what users see
as one character — matching `ColorFontRegions::Range` and
baumhard's text primitives (see
`lib/baumhard/CONVENTIONS.md §B1` and the
[`Range`](#range) entry above). Cluster indexing keeps a run
that ends after a Hebrew niqqud combining mark or a ZWJ-emoji
family on the same boundary the cosmic-text bridges in
`baumhard::font::attrs` slice on, so per-region styling lands
on whole glyphs. Validation: non-overlapping, ascending,
`end <= text's grapheme-cluster count`. Full reference:
[`format/text-runs.md`](./format/text-runs.md).

If `text_runs` is non-empty, **only covered ranges
render** — uncovered graphemes drop silently; nothing inherits.
So authors must cover every grapheme they want visible, not just
the ones they want to restyle. This is by design (it simplifies the
renderer's region pass) but it is the single biggest trap in
the format; `maptool verify` does not catch partial-coverage
intent vs. accident. The rule lives in one place for both
cosmic-text bridges, `covered_slices` in
`lib/baumhard/src/font/attrs.rs`: `rich_text_spans_into` shapes
exactly those slices and `displayed_text` concatenates them, so
what a section *displays* is not its `text` —
`baumhard::font::attrs::displayed_text` answers it, and the IPC
`scene` family reports that string. The only inheritance in the
model is *within* a run: a styling field a run leaves at its default
renders like the section's own text would have.

### Theme variables

Document-level CSS-style named colors referenced as
`var(--name)` from any color field.

Avoids hex repetition across hundreds of nodes
and edges. A theme switch changes the variable; everything
referencing it updates. Theme variants (presets) can be stored
under `canvas.theme_variants` and applied through the
`SetThemeVariant` document action.

Resolved at scene-build time in the color
cascade — variable lookup, then fall through to a default if the
name is unknown. Document actions
[`SetThemeVariant`](#document-actions) and `SetThemeVariables`
mutate the live `canvas.theme_variables` map.

### Zoom bounds

The mindmap-level surface of
[`ZoomVisibility`](#zoomvisibility): two flat fields
(`min_zoom_to_render`, `max_zoom_to_render`) on every renderable
entity (`MindNode`, `MindEdge`, `EdgeLabelConfig`,
`PortalEndpointState`). Cascade rule (replace-not-intersect),
field semantics, and authoring shape:
[`format/zoom-bounds.md`](./format/zoom-bounds.md). Console-verb
authoring: `zoom min=1.5 max=3.0`, `zoom clear`, `zoom max=unset`
against the active selection.

### Border geometry

Glyph-drawn frames around nodes — Unicode box-drawing
characters laid out around the node's AABB.

Borders are the visual frame that gives a node
its "boxed" appearance. They are made of glyphs (light, heavy,
double, rounded, or fully custom box-drawing chars), not solid
strokes — consistent with the
[everything-is-glyphs](#everything-is-glyphs) invariant. Borders
also serve as anchor surfaces for portal endpoints, which sit at
parametric positions along the border perimeter.

`lib/baumhard/src/mindmap/border.rs`. The
`GlyphBorderConfig` per-node record (in
`lib/baumhard/src/mindmap/model/node.rs`) carries:

- `preset: String` — one of `"light"` (`─ │ ┌ ┐ └ ┘`, the
  default: `default_border_preset` in `model/node.rs` is the serde
  default, and `resolve_border_style` in `border.rs` floors an
  unnamed preset to the same word), `"heavy"` (`━ ┃ ┏ ┓ ┗ ┛`),
  `"double"` (`═ ║ ╔ ╗ ╚ ╝`), `"rounded"` (`─ │ ╭ ╮ ╰ ╯`), or
  `"custom"`.
- `font: Option<String>` — font family override; `None` =
  system default.
- `font_size_pt: f32` — glyph size.
- `color: Option<String>` — `#RRGGBB` override. `None` on a
  node border falls through the node's theme frame color, then
  `canvas.default_border.color`, then `style.frame_color` (the one
  field-level tier of an otherwise slot-level cascade); `None` on
  a section frame is the selection-highlight cyan, with no theme
  or canvas tier.
- `glyphs: Option<CustomBorderGlyphs>` — per-side glyph
  overrides (top / bottom / left / right / four corners); only
  consulted when `preset = "custom"`.
- `padding: f32` — border-to-content gap in pixels.
- `color_palette: Option<String>` / `color_palette_field:
  Option<String>` — cycle a named palette's field across the border
  glyphs instead of one flat color (`border palette=…` on the
  console).

Geometry constants (`BORDER_CORNER_OVERLAP_FRAC`,
`BORDER_APPROX_CHAR_WIDTH_FRAC`) are shared between the
renderer and tree builder; they must agree, or corner
alignment drifts.

Borders today only render on rectangular nodes
(`NodeShape::Rectangle` and `style.show_frame = true`). Ellipse
borders need shape-aware glyph layout — a named seam not yet
implemented.

### `GlyphConnectionConfig`

The per-edge rendering configuration: body glyph,
caps, font, font size, screen-space font clamps, color.

Every `MindEdge` carries one. `GlyphBorderConfig`
is to a node what `GlyphConnectionConfig` is to an edge: the
shape of the glyphs that draw the thing. The body glyph is
repeated along the connection path; `cap_start` and `cap_end`
override the terminal glyphs if present. Font size is
interpreted as the target *on-screen* size at zoom = 1.0;
`min_font_size_pt` and `max_font_size_pt` clamp the effective
screen-space size as the camera zooms, so a long edge stays
readable both zoomed in and zoomed out.

`lib/baumhard/src/mindmap/model/edge.rs` (`GlyphConnectionConfig`).
Fields: `body: String` (default mid-dot `·`), `cap_start` /
`cap_end: Option<String>`, `font: Option<String>`, `font_size_pt:
f32`, `min_font_size_pt` / `max_font_size_pt: f32` (plain floats
with serde defaults, not `Option`s), `color: Option<String>`, and
`spacing: f32` (the gap between body glyphs along the path). Color
cascade priority (highest
first): edge-label → `glyph_connection.color` → the source
node's frame tier (`overrides.frame`, else its palette group's
`frame`) → `canvas.default_connection.color` → `edge.color`.
The theme sits above the canvas default because it is per-node
and the default is map-wide, and the frame tier is the one
`MindMap::node_frame_theme_tier` also hands the node's border
resolver, so a per-node frame override moves both.
`effective_font_size_pt(zoom)` is the helper callers reach for
to derive the clamped screen-space size.

### `ControlPoint`

An author-set Bézier offset on a `MindEdge`,
expressed as an offset from a node center rather than an
absolute canvas coordinate.

Straight line-mode edges can become curved
when the author specifies control points. Zero control points
is a straight segment; one promotes to a cubic Bézier (via
quadratic-to-cubic lifting); two or more define a cubic
directly. Control points live as offsets from endpoint centers
so a node move drags the curve along without the author
having to re-tune the path.

`lib/baumhard/src/mindmap/model/edge.rs`. Consumed by
[connection path construction](#connection-paths), where
`build_connection_path` converts control points from offsets
into cubic control coordinates in canvas space.

### Portals

Edges with `display_mode = "portal"`: rendered as two
glyph markers, one at each endpoint, instead of a connecting
line.

When two endpoints are far apart on the canvas,
drawing a literal line between them is visually noisy and
expensive (hundreds of glyphs). Portals decouple the visual link
from the physical span: the user sees a small glyph at each end,
recognizes them as a pair (matching color, matching text), and
can double-click either to fly the camera to the partner.
Portals share the underlying edge with line-mode — the only
difference is `display_mode`.

`lib/baumhard/src/mindmap/model/edge.rs`. Per-endpoint state lives
in `PortalEndpointState`: `color`, `border_t` (parametric
position on the owning node's border), `perpendicular_offset`
(signed distance along the outward normal), `text`, `text_color`,
`text_font_size_pt`, `text_min/max_font_size_pt`,
`min/max_zoom_to_render`. The icon and the adjacent text are
sibling leaves of the portal tree, so a click resolves to exactly
one of them through the tree's BVH
(`AppScene::portal_at` → `PortalHitIndex::resolve`, yielding a
`PortalHit` that names the sub-part). A click on the icon selects
`SelectionState::PortalLabel` (and font/color ops target the icon
channel) while a click on the text selects
`SelectionState::PortalText` (and ops target the text channel).
An endpoint with no text lays its text slot out at zero extent, so
the reserved slot cannot answer a click.
Full reference: [`format/portal-labels.md`](./format/portal-labels.md).

### Edge labels

Optional text along a line-mode edge, positioned by
parametric `t` along the path with an optional perpendicular
offset.

Edge annotations — "depends on", "blocks",
"derived from". Line-mode only; portal edges use per-endpoint
text instead. Labels can be dragged to reposition (native today),
authored via the `label position_t=… perpendicular=…` console
verb (cross-platform), and given their own zoom-window override.

`EdgeLabelConfig` on `MindEdge`. Position
encoded as `(position_t, perpendicular_offset)`. Drag computes
both via `closest_point_on_path`. Replace-not-intersect zoom
cascade matches portals.

### Connection paths

The geometric backbone of edge rendering: straight
segments and cubic Bézier curves with anchor resolution at the
endpoints.

Given two node AABBs and optional control
points, compute the curve along which to lay out edge glyphs.
The same path math powers glyph placement (sample at uniform arc
length), label drag (project cursor onto path), and hit-testing
(distance from cursor to path).

`lib/baumhard/src/mindmap/connection/`. Key
functions: `build_connection_path` (from anchors + control
points), `resolve_anchor_point` (auto / top / right / bottom /
left), `point_at_t`, `tangent_at_t`, `closest_point_on_path`
(uniform-t sampling + Newton refinement for cubics, direct
projection for straight lines), `sample_path` (arc-length-uniform
glyph placement), `distance_to_path`, and the pair behind the edge hit-test —
`distance_to_path_within`, which `hit_test_edge` calls, and
`path_bounds`, which it calls in turn. `path_bounds` is the
control polygon's AABB, which contains the curve by the Bezier
convex-hull property *in real arithmetic*; the f32 escape from
that box is what `distance_to_path_within`'s slack allows for, and
it is the reason to reach for that function rather than to compare
against the box by hand. Quadratics get promoted to
cubic at build time, so the apply path is always one of two
shapes.

### Portal geometry

The conversion between `border_t` — a side-indexed parameter with
period `BORDER_T_PERIOD`, one unit per side — and a canvas point on
a rectangular node's border, plus directional defaults.

Portal endpoints must sit on their owning
node's border, parametrically — so when the node is resized, a
label at "the middle of the right edge" stays at the middle of
the right edge. The side-indexed encoding — each side owns one
unit of `t`, top, right, bottom, left in that order, so the period
is the side count — is the right abstraction: stable across resize,
deterministic across corners.

`lib/baumhard/src/mindmap/portal_geometry.rs`, where
`BORDER_T_PERIOD` is declared.
Functions: `wrap_border_t` (rem-Euclid into `[0, BORDER_T_PERIOD)`),
`border_point_at`, `border_outward_normal`,
`default_border_t` (the auto-orientation: cast a ray from owner
to partner center), `nearest_border_t` (project a canvas point to
the closest border parameter, used by drag-snap).

### Fold state

A boolean per node; folded subtrees are excluded
from the display tree but persist in the model.

Hide subtrees without losing data. The user
can collapse a region of the map; reopening restores it. Each
build computes the hidden set once with
`MindMap::fold_hidden_set` (an O(N) pass) and tests membership
per element; the tree builder passes a `parent_folded` flag down
the recursive walk so the same cascade is checked by construction.
One-off callers can still use `MindMap::is_hidden_by_fold`, which
walks the parent chain at O(depth) per call.

### Tree builder

Projects a `MindMap` into a Baumhard
`GfxTree` mirroring the parent-child
structure, with each `MindNode` materializing as a three-deep
subtree (container + section-areas + section-models).

Mutations need a `Tree` to walk against. The
tree builder constructs it from the model: each visible
`MindNode` becomes a chrome-only container `GfxElement::GlyphArea`
plus one `GfxElement::GlyphArea` per section (carrying the
section's text + theme-resolved regions) plus a structural
`GfxElement::GlyphModel` grandchild per section-area as a
future-mutation seam. Parent-child relationships become tree
edges; channels are preserved. Per-role sub-builders cover
borders, portals, connections, edge labels, and edge handles,
each producing its own tree (and matching mutator-tree) so
per-role mutations stay scoped.

`lib/baumhard/src/mindmap/tree_builder/mod.rs`. Returns a
`MindMapTree` with `node_map: HashMap<String, NodeId>` (mind id →
container arena id), `section_map: HashMap<(String, usize), NodeId>`
(mind id + section index → section-area arena id), reverse maps
for both, and an `owning_mind_id` helper that climbs up to three
arena edges (model → section → container) to find the owning
mind-node. Section-areas (and section-models) carry
`Flag::SectionRoot`. Folded nodes are excluded.

### Canvas-role projection

Projects a `MindMap` into one `GfxTree` per
canvas role (`CanvasRole`, eight of them) — borders, connections,
connection labels, portals, section frames, and the three handle
families — beside the node tree the [tree builder](#tree-builder)
produces, which the renderer takes through its own
`rebuild_buffers_from_tree` rather than as a role. There is no
flat intermediate scene: the walker in
`src/application/renderer/tree_walker.rs` is the only consumer,
and every role reaches it as a tree.

Each role file under `lib/baumhard/src/mindmap/tree_builder/`
follows the same **courier** shape:

1. a *data pass* that resolves the model plus this frame's
   [overrides](#frame-overrides) into plain per-element data —
   `border_node_data`, `build_connection_elements`,
   `build_label_elements`, `portal_pair_data`,
   `build_section_frames`, `build_selected_node_handles`,
   `build_selected_section_handles`;
2. a *full-rebuild projection* (`build_*_tree`) and an *in-place
   projection* (`build_*_mutator_tree`) over that data.

Which projection runs is decided by hashing the data pass's
output, because any interaction re-runs every role: clicking an
edge label reaches the border and portal passes too. Two
questions, and roles that can answer both ask both. The
*structural* signature (`border_structure_signature`,
`portal_structure_signature`, `connection_identity_sequence`,
`handle_identity_sequence`, …) covers only what fixes the tree's
channel layout, and answers *can a mutator align against the
registered arena at all?* — a color change must leave it alone,
or a color-picker hover would reallocate the arena every frame,
which is the cost the in-place path exists to avoid. The
*content* signature (`border_content_signature`,
`portal_content_signature`) covers every field either projection
reads, and answers *is there anything left for that mutator to
write?* On a content match the role does nothing: the registered
tree already holds what a rebuild would produce. Completeness
runs the opposite way for the two — a structural signature is a
deliberate subset, a content signature that misses a field
serves a stale frame, so each content hasher destructures its
element type exhaustively and a new field fails to compile until
it is accounted for. Section frames, which have no in-place
projection, carry only the content signature
(`section_frame_content_signature`); the connection, label and
handle roles carry only the structural one and always apply
their mutator.

Those three still spell theirs `*_identity_sequence` and return
a `Vec` the caller hashes through `hash_canvas_signature`, which
is the older shape of the same idea; borders and portals were
converted to stream into the hasher when their content
signatures landed. The two spellings mean the same thing and the
conversion of the remaining three is unclaimed work, not a
distinction.

Style is resolved exactly once, in the data pass; neither
projection re-resolves it. Two inputs cross role boundaries.
`node_clip::node_clip_aabbs` is the one the connection sampler
clips glyph samples against; it reads only the resolved border
`font_size_pt`. [`EdgePathCache`](#edge-path-cache) is the other:
the sampler, the selected edge's grab-handles and the label layout
all want the same edge's `ConnectionPath`, and it is built once per
edge per frame for whichever of them asks first.

The application layer drives the sequence through
`CanvasFrame` (`src/application/app/scene_rebuild.rs`), which owns
the shared per-frame inputs (the fold-hidden set, the assembled
overrides) and exposes one `update_*` per role so a caller can
refresh only the roles its interaction can change — a scroll-wheel
zoom touches connections, labels, portals, and edge handles but
never a border or a resize handle.

Selection highlight, drag-preview offsets, NodeEdit dimming, and
color-picker previews are all applied at projection time and never
committed back to the model. Caching at the
[`scene_cache`](#scene-cache) level reuses sampled connection
positions when endpoints don't move.

### Frame overrides

`tree_builder::overrides` holds the transient, frame-local
substitutions the projection folds on top of the committed model:
`SceneSelectionContext` (selection plus the inline label / portal
text editors' uncommitted buffers), `EdgeColorPreview` /
`PortalColorPreview` (color-picker hover), `BorderPreview` (staged
`border preview …` edits), and `InteractionModeOverrides` (the
mode-derived resize / NodeEdit targets). `FrameOverrides` bundles
all four; `MindMapDocument::frame_overrides` assembles one per
rebuild so no two roles can disagree about what the user is
pointing at.

### Scene cache

A per-edge cache of sampled glyph positions, keyed
on `(from_id, to_id, edge_type)`, reused across frames when
endpoints have not moved.

Sampling a cubic Bézier path at uniform arc
length is the most expensive per-edge work. The cache invalidates
on endpoint drag (via the drag `offsets` map) and on zoom or
structural change. Otherwise the previous frame's samples are reused with a
cheap `point_inside_any_node` clip filter.

`lib/baumhard/src/mindmap/scene_cache.rs`.

An entry holds the sampled geometry, the endpoints it was sampled at,
and the `SampleParams` — effective font size, glyph spacing, per-path
sample budget — it was sampled under. It holds nothing the frame
*draws* with, and there is no exception: body glyph, cap glyphs, font
family, font size and body color are all read from the live model on
every path, so no styling edit can be served stale. The two reuse
doors, `reusable` and `reusable_mut`, compare the whole
`SampleParams`, so a font-size or spacing edit resamples without the
mutating site having to remember `clear()`.

What the caller still owes is geometry the params cannot see, and only
that: an endpoint that moved in the model without appearing in the
drag `offsets` map, an anchor or control-point edit, and a node
resize. Nothing about color is on that list — neither a theme-variable
edit nor a direct `edge.color` edit.

`refill` is the cache's only writer, and it hands the sampler the
buffer the entry already holds rather than a new one, so an edge that
resamples while cached allocates nothing. A refill that fills nothing
evicts, so an entry with no points — which the reuse doors would
otherwise serve as an edge that draws nothing, forever — cannot
exist.

Liveness is a generation stamp rather than a per-frame set of keys:
`begin_pass` opens a pass, every route that hands out or writes an
entry stamps it, and `evict_unseen` drops what still carries an older
stamp. Reaching an entry *is* marking it, so a route cannot draw an
edge and then forget to record that it saw one.

### Edge path cache

A per-frame memo of `ConnectionPath`s, keyed by index into
`map.edges` and filled on first ask.

Three passes want the same edge's path in one rebuild: the
connection sampler on a scene-cache miss, the label layout for
every labeled edge, and the grab-handles for the selected edge.
Each used to build its own.

It is **lazy** rather than precomputed, which is the load-bearing
half: an unlabeled, unselected edge that hits the scene cache asks
for no path and none is built. It borrows the `MindMap` and the
drag-offset map it resolves against, so a memo cannot outlive the
inputs it is keyed on — nothing has to remember to invalidate it.
`CanvasFrame` owns one per rebuild and threads it through both
connection passes.

`lib/baumhard/src/mindmap/tree_builder/edge_path.rs`.

### Trigger bindings

Per-node and per-section bindings of input events to
custom-mutation ids: `Trigger::{OnClick, OnHover, OnKey(String),
OnLink(String)}`.

Authoring interactive map elements without
custom code: a button-like node fires a custom mutation when
clicked. Each `TriggerBinding` carries `trigger`, `mutation_id`
and `contexts: Vec<PlatformContext>` — the platform filter
(Desktop / Web / Touch); empty means "all platforms". There is no
separate gate type: the binding's own `contexts` field is the
carrier.

`MindNode.trigger_bindings` and `MindSection.trigger_bindings`
(`lib/baumhard/src/mindmap/custom_mutation/mod.rs` for the types);
dispatch lives in the application's input handlers
(`click_triggers::fire_onclick_triggers` for `OnClick`). Missing mutation IDs are
silent no-ops — runtime ignores rather than panicking.

---

## §4 The mutation framework

The mutation framework is the primary extensibility seam. It spans
both crates — the AST and walker live in Baumhard, the registry
and dispatch live in Mandala — and is the answer to "how do I add
behavior to a mindmap without recompiling?" Everything from
"grow font 2pt on the selected subtree" to size-aware layouts like
`flower-layout` and `tree-cascade` flows through it.

For the JSON authoring surface see
[`format/mutations.md`](./format/mutations.md). For the
prescriptive carrier shape see
[`lib/baumhard/src/mindmap/custom_mutation/`](./lib/baumhard/src/mindmap/custom_mutation/).

### `CustomMutation`

The carrier struct: an id, name, description,
contexts, optional mutator AST, target scope, behavior, optional
document actions, and optional animation timing.

A `CustomMutation` is one named, reusable
operation. Authored as JSON (declarative) or registered in Rust
(imperative); referenced by id from console verbs, trigger
bindings, or other custom mutations. The same shape covers tiny
deltas ("add 2pt to font") and structural algorithms
(`flower-layout`).

`lib/baumhard/src/mindmap/custom_mutation/mod.rs`. Fields: `id`
(unique key), `name` and `description` (human-readable),
`contexts` (taxonomy tags — see
[contexts](#contexts-taxonomy)), `mutator` (an optional
`MutatorNode` AST), `target_scope` (which nodes the change
covers), `behavior` (`Persistent` or `Toggle`), `predicate` (an
optional `Predicate` filtering the targets the scope resolved),
`document_actions` (canvas-level operations, empty when none),
`timing` (optional `AnimationTiming`).

### Four-source loader

Mutations merge from four sources at startup, with
ascending precedence: App < User < Map < Inline.

Authors at every layer can define mutations
without stepping on each other. A bundled "grow-font-2pt" can be
overridden by the user's personal version, which can in turn be
overridden by a map's local definition, which can in turn be
overridden by a single node's `inline_mutations`. The
`MindMapDocument::mutation_sources` map records which layer won
each id, so `mutation help <id>` can report it.

`src/application/document/mutations_loader/`. Native:
- App from `assets/mutations/application.json` via `include_str!`.
- User from `$XDG_CONFIG_HOME/mandala/mutations.json`. The desktop
  wrapper takes an explicit-path layer ahead of it, but the CLI
  never fills it — `run_native_init` calls
  `load_app_and_user(None)`, and `--keybinds` is the only config
  flag `main.rs` parses.

WASM:
- App from the same embedded JSON.
- User from `localStorage` (never a URL query param).

Map and inline are loaded from the document on every load. Each
layer is best-effort — a malformed entry in the user file logs a
warning and is skipped on its own (a file that is not the
`{"mutations": [...]}` envelope at all is skipped whole); app bundle
failures log an error (a build-time invariant violation).

The provenance of each merged mutation is tracked in
`MindMapDocument::mutation_sources` as a `SourceTier`
(`src/application/source_tier.rs` — the same four-rung
`App` / `User` / `Map` / `Inline` ladder the macro registry
resolves against), so
`mutation help <id>` on the console can report which layer
won a given id.

### Declarative path — `MutatorNode` AST

A pure-data AST that compiles to a
`GfxMutatorTree` and runs through the standard tree
walker.

Any mutation expressible as a tree of
field-level deltas with control-flow instructions belongs here.
This is the default: write JSON, the runtime walks it. The AST
shape mirrors `Mutator` — `Void`, `Single`, `Macro`,
`Instruction` — plus a `Repeat` wrapper for "N consecutive
children at consecutive channels with the same template" (the
flower-petal pattern, etc.).

`build(ast, context)` in `lib/baumhard/src/engine/builder/build.rs`
walks the AST recursively, expands `Repeat` to N children with
incrementing channels, resolves `Runtime("<label>")` holes and
per-iteration `Template`s via the `BuildContext`, and returns a
fully-inflated `MutatorTree` — or a `BuildError` for a shape no
mutator tree can express. The
walker then applies it via `apply_to`. After application, changed
elements are synced back to the model so the change persists across
the next scene build.

### Imperative path — `DynamicMutationHandler`

A registered Rust function pointer the dispatcher
calls directly when the AST is too narrow.

Some operations are inherently imperative —
arbitrary BFS layouts, multi-pass spatial algorithms, anything
that needs runtime control flow the walker doesn't provide. The
handler registry lets them live as Rust functions registered at
startup, with the same `id`/contexts/target-scope surface as a
declarative mutation.

`src/application/document/mutations/`. Built-in handlers:
- `flower_layout.rs` — radial child arrangement.
- `tree_cascade.rs` — hierarchical cascading layout.

`register_builtin_handlers()` wires them at startup. Adding a new
handler is: new module, new function, new registration call, new
matching id in `assets/mutations/application.json`.

When a higher-precedence layer (User / Map / Inline)
declares the same id as a registered handler, **the declarative
mutator wins**. The handler is bypassed. This prevents a subtle
hijack where a user's JSON would silently invoke imperative code
they did not author.

### Target scopes

Seven variants telling the dispatcher which nodes the
mutation covers — also used as the snapshot window for undo.

A mutation declares "I touch this node only" or
"I touch this node and all its descendants" and the dispatcher
both walks the right subtree and snapshots the right set for
undo. The undo-snapshot equivalence is the load-bearing detail:
if a mutation's `target_scope` is too narrow, undo will not
fully reverse it.

Variants: `SelfOnly`, `Children`,
`Descendants` (not the anchor), `SelfAndDescendants`, `Parent`,
`Siblings` (the anchor's siblings, excluding itself — a root has
none, since "sibling" means "shares my parent" and the other
roots of a multi-root map do not), `SectionsOnly` (the anchor's
section-areas; the anchor `MindNode` is still the snapshot
window). Scope helpers in `custom_mutation::scope` produce
matching `MutatorNode` shapes for the AST walker.

The scope resolves to a target set and the mutator is anchored at
**each target in turn**, so a pairing has complete undo coverage
only when that set is closed under the mutator's reach. Only the
two descendant scopes are; every other scope needs a mutator that
touches its anchor alone. `TargetScope::covers_reach` encodes that
table and `warn!`s on a mismatch — see `format/mutations.md`.

Closure is about undo coverage alone, not about how often the
payload lands. Per-target anchoring runs a wider-than-`SelfOnly`
mutator once per ancestor target, so a `SelfAndDescendants` scope
paired with a `Descendants` reach writes a node at depth *k*
*k + 1* times — covered by the snapshot, but compounding for a
non-idempotent payload. The flat-apply path applies once per
target and so avoids this today.

### Behaviors — `Persistent` vs. `Toggle`

Whether the mutation commits to the model and pushes
an undo entry (`Persistent`) or only modifies the display tree
and remembers itself in `active_toggles` (`Toggle`).

Some mutations are "apply and remember"
(persistent — visual change, undo coverage); others are
"reversible inspection" (toggle — visual change without model
commit, second trigger reverses). Toggles are the right shape
for "highlight this", "expand this preview", "show debug
overlay".

Persistent: snapshot affected nodes, apply,
sync back, push undo. The sync-back
(`document/custom/sync.rs::sync_node_from_tree`) persists node
position, section offset / size / text / color+font runs, and
**font size** (the tree-side `scale` is distributed back across a
section's run `size_pt` values as a delta, preserving relative
run sizing); line-height is derived (`scale * LINE_HEIGHT_FACTOR`,
`tree_builder/node.rs`) so it needs no separate home. Fields with no reverse converter (outline, shape,
zoom-visibility, line-height) are `warn!`-flagged at apply time
rather than silently applied-then-reverted. The undo entry and the
`dirty` flag are gated on the sync-back reporting an actual model
change, so a no-op apply (predicate filtered everything, non-flat
mutator skipped, or a mutation that changed nothing) leaves no
dead entry behind. Toggle: apply to tree only, insert
`(node_id, mutation_id)` into `MindMapDocument::active_toggles`;
on second trigger from the same anchor, remove the pair (undo
stack gets no entry — re-triggering is the reverse). Because a
rebuild projects the tree from the model and toggles never touch
the model, `build_tree` re-stamps every active toggle
(`reapply_active_toggles`) after each rebuild — without that
re-application a toggle-on's visual would die at the end of the
same dispatch and have nothing left to reverse.

### Contexts taxonomy

Dotted-namespace tags describing what a mutation
operates on: `"internal"`, `"map"`, `"map.node"`, `"map.tree"`,
plus the reserved `"plugin.<name>.<kind>"` namespace.

The console's `mutation list` filters by
context so users see only mutations relevant to their current
selection. `"internal"` hides a mutation from listing entirely
(used by handlers that compose into other mutations). The
plugin namespace is the home of future plugin-authored
mutations.

`matches_context(query)` returns true if the
mutation's `contexts` include `query` exactly or sit inside its
dotted prefix; `matches_context("map")` hits both `"map.node"`
and `"map.tree"`.

### `PlatformContext`

A three-variant enum — `Desktop`, `Web`, `Touch` —
threaded through mutation handlers and trigger-binding filters
so an authored mutation can branch on where it's running.

Some operations should behave differently per
target — a layout that reflows narrower on mobile, a trigger
binding that only fires on Desktop. `PlatformContext` is the
channel for that distinction, distinct from the dotted-namespace
"contexts" taxonomy above (which describes *what* the mutation
operates on).

Defined in
`lib/baumhard/src/mindmap/custom_mutation`. Today the variant is
chosen at compile time (`Desktop` on native, `Web` on WASM,
through `app::PLATFORM_CONTEXT`); the `Touch` variant exists but
no input path dispatches on it yet — including the tap path,
which reports the build's platform like every other input.
Relabeling a gesture would silently stop every `["Web"]`-gated
trigger from firing under a finger in the browser, so which
platform a build *is* stays a build fact until the authoring
format decides otherwise (`format/mutations.md`).
Carried as `TriggerBinding.contexts: Vec<PlatformContext>` (see
[trigger bindings](#trigger-bindings)) so a binding can declare
which platforms it fires on; empty means all of them.

### Document actions

Canvas-level operations a mutation can carry
alongside (or instead of) its tree mutations:
`SetThemeVariant(name)`, `SetThemeVariables(map)`.

"Switch the theme" is not a per-node delta —
it touches `canvas.theme_variables`. Document actions cover that
seam. They run alongside the tree mutation; a single mutation
can both restyle nodes and switch the theme in one apply.

`lib/baumhard/src/mindmap/custom_mutation/mod.rs` (`DocumentAction`,
`#[non_exhaustive]` so the macro dispatcher's privilege carve-outs
must grow with it — see [Action dispatch](#action-dispatch)).
`SetThemeVariant` copies a named preset from
`canvas.theme_variants` into the live `theme_variables`;
`SetThemeVariables` overwrites individual entries while
preserving unmentioned keys.

### Animation timing

Optional duration / delay / easing wrapper around
any mutation, turning instant application into a clock-driven
interpolation.

A "grow font" that snaps is fine; one that
animates over 300ms reads better. The timing wrapper lets a
declarative mutation carry that timing without authoring an
animation by hand. The dispatcher starts an `AnimationInstance`
that ticks each frame, blends the in-flight state, and commits
on completion.

`lib/baumhard/src/mindmap/animation.rs` — reached as the `timing`
field on `CustomMutation`. Fields:
`duration_ms`, `delay_ms`, `easing` (`Linear` / `EaseIn` /
`EaseOut` / `EaseInOut`), and a reserved `then` (`Followup`)
slot. The app-side `AnimationInstance` that ticks it lives in
`src/application/document/types.rs`.

`Followup::{Reverse, Chain, Loop}` is named but not
yet wired. When it lands, mutations will compose into chains
and oscillations without scripting.

### Runtime holes — `BuildContext`

A trait the host implements to feed runtime values
into a `MutatorNode` AST at build time.

Some mutations need values the AST can't
inline — the count of currently-visible children, the cursor
position when invoked, a field looked up from the selected
node. `MutationSrc::Runtime("<label>")`,
`MutationListSrc::Runtime("<label>")` and
`CountSrc::Runtime("<label>")` defer those holes to a
`BuildContext` registered per-mutation-id, and a
`MutationSrc::Template` is resolved by it per `Repeat` iteration;
the builder consults it as it walks. Pure-data mutations (no holes)
use `NoRuntime`, the context that supplies nothing.

`lib/baumhard/src/engine/builder/context.rs`. `BuildContext<E, M>`
has four methods: `count(&self, name) -> usize`,
`mutation(&self, label) -> M` and `mutation_list(&self, label) ->
Vec<M>` have default bodies that `unreachable!` rather than return a
placeholder, because a hole the AST names and the context does not
fill is an authoring error, not a runtime value; `resolve(&self,
section, index, template) -> M` has none, because what a template
means is the payload's to say. A glyph context resolves a
`Vec<CellField>` template into an `AreaDelta` with the helpers in
`gfx_structs/template.rs` — from a pre-built `GlyphArea` per cell
(`resolve_from_area`), or field by field on a hot path
(`area_delta_from_template`). Custom mutations register their
context at apply time so the build produces the right concrete
tree.

---

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
the renderer, a `document: Option<MindMapDocument>`, the
`mindmap_tree` the tree builder projects from it (an
`Option<MindMapTree>` — the tree is app state, not document state),
`app_scene` and `scene_cache`, `drag_state`, `interaction_mode`,
modal UI state (console, node text editor, single-line editor, color
picker), `picker_hover`, the resolved keybind table and macro
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

A placard is an ordinary one-node `MindMap`
(`lib/baumhard/src/mindmap/placard.rs`) whose text is the headline,
the path or URL that was asked for, the loader's message in full,
and a line confirming nothing was written. Because it is a map, it
reaches the screen through the same tree projection as any other
document (§1 "Everything is glyphs") — no second pipeline, no
renderer pass, and nothing that needs a GPU to test. The message is
wrapped with `grapheme_chad::wrap_to_display_width`, so a path
containing an emoji folds without shattering.

"In full" has one bound, and it is not stylistic: a loader message
is as long as the file makes it, because serde quotes an offending
JSON string back verbatim. So the source and the message are each
elided in the *middle* — both ends kept, a one-line notice between
— past `PLACARD_HEAD_CLUSTERS + PLACARD_TAIL_CLUSTERS`. Nothing the
loader realistically emits comes near that (the longest, a 400-node
parent-cycle report, is 56 wrapped lines), and the budget is what
keeps the placard a map the loader itself accepts: unbounded, a
4 MB message asks for a node twice as tall as `MAX_NODE_AXIS`.

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

The per-frame heartbeat: land one injected-input sample, drive the
throttled drag and the picker hover, re-derive the rubber band,
rebuild camera-driven geometry, tick animations (`animation_drain`,
the body the browser's frame runs too). It does **not** render — the
frame is drawn from winit's `RedrawRequested` (or the headless
driver's owed-frame check), never from the drain.

Inputs arriving between frames mutate the document; the
throttled-interaction shells and the renderer's
connection-geometry flag ensure the drain rebuilds only what
changed. This decouples mutation frequency (often per-input-sample)
from rebuild frequency (at most once per frame), so a flurry of
pointer events doesn't trigger a flurry of scene rebuilds. Scene
rebuilds after a *mutation* are not the drain's business at all:
they run at their call sites (`rebuild_all` and its tiers, see
[scene rebuild granularity](#scene-rebuild-granularity)), and the
[dirty flag](#dirty-flag) is the unsaved-changes marker, not a
rebuild trigger.

`src/application/app/heartbeat.rs` holds the frame:
`InputHandlerContext::drain_frame(beat)` lands the injected sample
and calls `drain_inputs`, whose body is the drain. Two of its steps
are the helpers in `src/application/app/drain_frame.rs` (native-only,
like the drag state they read); the animation tick is
`animation_drain::drain_animation_tick` in
`src/application/app/animation_drain.rs`, which is not native's alone
— the browser's frame (`run_wasm/event_redraw_requested.rs`) runs the
same body from `RedrawRequested` since #144; the rest are inline. It
is called once per iteration of the loop body both
native drivers run (`pump::iterate` in `app/pump.rs`, see
[Headless driver](#headless-driver)) — winit's `about_to_wait`, or
the headless pump's pass. Step order, as the code runs it:

1. Land one injected-input sample from the IPC queue
   (`apply_injected`), delivered to the same `on_*` arm the winit
   event would reach — in real time, or on a step's heartbeat.
2. Drive the active throttled drag, if any
   ([`ThrottledInteraction::drive`](#throttledinteraction-and-throttleddrag)):
   drain the pending delta if the throttle says so.
3. Drive the picker-hover interaction while the picker is open
   (`ColorPickerHoverInteraction::drive`); if the picker closed with
   a drain still queued, clear the pending flags directly rather
   than strand them behind the throttle window.
4. `drain_rect_select` — unconditional, every frame, so the rubber
   band's overlay and covered set are re-derived from whatever
   `DragState` is found (see [`DragState`](#dragstate)).
5. `drain_camera_geometry_rebuild` — the camera-driven re-projection
   of the zoom-dependent roles, under the renderer's
   `connection_geometry_dirty` flag, suppressed while a node is
   being moved.
6. Animation pause/resume bookkeeping around tree-mutating drags
   (native's alone: there is no drag to pause for without a drag
   state), then `animation_drain::drain_animation_tick` — advance
   running animations, commit on completion (an undo entry is pushed
   there) — when the frame moves time and no tree-mutating drag is
   live.

What the loop body does *around* the drain is `pump::iterate`'s
business and is pinned by its tests: the IPC drain runs after
`drain_frame`, the recorder's `record_tick` after that, then the
redraw request, the FPS overlay's idle flip, the `clock.wait`
evaluation and the exit check. On native, `Renderer::process` — the
GPU half — runs only through `InitState::draw_frame`, which the
`RedrawRequested` arm in `run_native.rs` calls, and which
`run_headless.rs` calls when the redraw ledger (`app/settle.rs`)
says a frame is owed; the FPS counter ticks there, not here.

**Two halves.** Steps 2 through 5 are *input-driven*: they flush
what input already did and need no clock. The *time-driven* half —
the injected sample of step 1, the animation tick of step 6, and an
active recording's frame after the IPC drain — is a function of when
the frame runs, read off the [app clock](#app-clock). In real time
every iteration runs both. Under a virtual clock the loop's own
iterations (`Beat::Loop`) run the input-driven half only, and
`clock.step` runs the whole frame at the virtual instants it names —
a heartbeat every `HEARTBEAT_MS` (`Beat::Cadence`) and every frame
boundary of a running recording (`Beat::Boundary`, which lands no
injected sample) — ticking the watchdog per heartbeat. The frame is
a method set on `InputHandlerContext` rather than on `InitState`,
with the input arms (`on_*`) beside it, so a `clock step` console
line runs the very frame the IPC command does.

### `MindMapDocument`

The data plane: owns the `MindMap`, the selection, the undo stack,
the running animations, the mutation registries, and the transient
previews the projection reads.

This is where every persistent piece of state
lives. It is the only owner of the model and the undo stack; the
renderer reads from it, never mutates. The dirty flag belongs to
it. Transient previews (live color picker, in-flight label edit,
in-flight portal-caption edit) belong to it too — read by the scene
builder, never committed back without an explicit step.

`src/application/document/mod.rs` (`struct MindMapDocument`). Its
seventeen fields, in declaration order: `mindmap: MindMap`,
`file_path`, `dirty: bool`, `load_failure: bool` (this document *is*
the [load-failure placard](#load-failure-placard), which a one-shot
`--screenshot` launch reads so it does not capture the placard as the
map), `revision`
(see [document revision](#document-revision)), `selection:
SelectionState`, `undo_stack: Vec<UndoAction>`,
`mutation_registry`, `mutation_sources`, `mutation_handlers`,
`active_toggles`, `active_animations`, `label_edit_preview`,
`portal_text_edit_preview`, `color_picker_preview`,
`border_preview`, and `rect_select_preview` (the node ids a
rubber-band rectangle currently covers — see
[`DragState`](#dragstate)). What it does **not** own: the projected
tree (`InitState::mindmap_tree` on native,
`WasmInputState::mindmap_tree` in the browser — a view the tree
builder derives from the model, rebuilt rather than kept in step),
the renderer, GPU resources, drag/mode state, modal editor state,
keybinds — those are all on `InitState`.

### `SelectionState`

A tagged union of what the user has selected:
nothing, a node, multiple nodes, one section of one node, an
edge body, an edge label, a portal icon, or a portal text.

Selection variants are mutually exclusive by
construction — at most one thing is selected at a time. The
variant tag is the routing key for everything operating on the
selection: which clipboard channel a copy goes through, which
color field a color command sets, which font field a font
command sets. The renderer uses it to apply the cyan highlight
to the right element.

`src/application/document/types.rs`. Variants:

- `None`
- `Single(node_id)` — one node
- `Multi(Vec<node_id>)` — multiple nodes
- `Section(SectionSel)` — one [`MindSection`](#mindsection) of
  one node, identified by `(node_id, section_idx)`. Surfaces
  when the user clicks on a section-area in a *multi-section*
  node (single-section migrated nodes still route through
  `Single` so today's whole-node verbs keep firing on the whole
  node target). Per-section setters cover text
  (`set_section_text`, `set_section_text_and_runs`,
  `set_section_text_preserving_runs`), color
  (`set_section_text_color`), font (`set_section_font_size`,
  `set_section_font_family`), position + size
  (`set_section_offset`, `set_section_size`), the
  structured-clipboard payload (`apply_section_payload`), and —
  added in Batch 5 — structural mutators that change the
  `sections` vector length: `add_section` (insert with AABB
  validation against the parent), `delete_section` (remove
  with the "≥1 section per node" invariant enforced), and
  `split_section` (split text at a grapheme boundary; runs
  partitioned via `text_run_ops::slice`). The trait dispatcher
  and the `section …` console verbs route here from a
  `SelectionState::Section`, or from `Single(id)` on a
  single-section node (which §4.5 rule 3 auto-resolves to
  `(id, 0)`).
- `MultiSection(Vec<SectionSel>)` — two or more sections,
  possibly across distinct nodes. Built by shift+click on a
  section while another section (or section-set) is selected;
  each shift+click toggles the targeted section in / out of the
  set. Per-section verbs (color text, font size / family) fan
  out via `selection_targets` and apply to every section in the
  set. Per-section gestures (drag-to-move, drag-to-resize) stay
  single-target — a `MultiSection` selection emits no resize
  handles, and a press on a section in the set **demotes** the
  selection down to `Section(node, idx)` at threshold-cross so
  mid-drag picker hints + per-section verbs reflect the
  in-flight gesture's actual target rather than the prior
  multi-set. Whole-node move and node-resize gestures demote
  the same way (to `Single(node)`). The `section
  move` / `section resize` verbs target the single-section
  selected (or take an explicit `section=K` kv); `MultiSection`
  is fan-out-only at the trait dispatch layer.
- `SectionRange { sel, section_span, grapheme_range }` — one
  anchor section plus the two range meanings a section-scoped
  selection can carry, each in its own newtype so they can never
  share a slot again (#47 part C): `section_span: SectionSpan` is
  an inclusive span of **section indices** on the owning node
  (what border / style verbs fan out over, and what
  `cleanup_after_structural_mutation` clamps), and
  `grapheme_range: GraphemeRange` is a half-open sub-range of the
  anchor section's **grapheme clusters** (what the range-aware
  setters `set_section_text_color_range`,
  `set_section_font_size_range`, `set_section_font_family_range`
  and the range-aware clipboard cut / paste consume). Produced by
  the inline text editor's shift-select anchor on close: the
  editor lifts the (anchor, cursor) pair into `grapheme_range`
  with `section_span` covering just the anchor section. Accessors
  that only care about the owning section (`selected_section`,
  `is_selected`, `selected_ids`) treat it identically to
  `Section`. **Clipboard contract:** `Cut` and `Paste` are
  range-aware — cut removes the in-range graphemes and returns
  them as text; paste splices into the range; `Copy` falls
  through to whole-section copy because the structured payload's
  geometry belongs to the whole section. **Picker contract:**
  `ColorTarget::Section` and `PickerHandle::Section` carry the
  grapheme sub-range, so commit calls
  `set_section_text_color_range` directly (bypassing the
  `MultiSection` fan-out — different sections' lengths make
  cross-section sub-range semantics incoherent).
- `Edge(EdgeRef)` — the whole edge body
- `EdgeLabel(EdgeLabelSel)` — the text label of a line-mode edge
- `PortalLabel(PortalLabelSel)` — a portal endpoint icon
- `PortalText(PortalLabelSel)` — a portal endpoint text

The four edge-adjacent variants (`Edge`, `EdgeLabel`,
`PortalLabel`, `PortalText`) each route to a different
clipboard / color / font channel: copy on a `PortalLabel` reads
the icon color; copy on a `PortalText` reads the text color;
font commands write to the corresponding field group.

### `EdgeRef`

The `(from_id, to_id, edge_type)` triple that
identifies an edge.

Edges have no stable id (§3:
[`MindEdge`](#mindedge)), so selection, undo entries, and
console arguments all carry this triple. Equality and lookup are
by triple match against the model's `Vec<MindEdge>`.

`src/application/document/types.rs` (`EdgeRef`). The `matches`
method walks the edge vector linearly; this is fine because
edges are sparse and the lookup happens at user-event frequency,
not in hot loops.

### `InteractionMode`

The single cross-platform interaction-mode enum that absorbed
the pre-redesign `AppMode` (Reparent / Connect) plus the new
Resize / NodeEdit modes the section-borders-resize PR added.
Five variants today: `Default` / `Reparent { sources }` /
`Connect { source }` / `NodeEdit { node_id }` / `Resize {
target }`.

Some user actions take two clicks (select a source, then click
a target); some put the canvas into a sub-context where chrome
and click-routing diverge (resize handles / per-section frames).
`InteractionMode` is the modal substrate for both shapes — what
the user is *doing right now*.

- `Default` — normal canvas navigation. Click selects, drag
  pans, edges snap.
- `Reparent { sources }` — the next left-click on a node
  attaches `sources` as its last children; left-click on empty
  canvas promotes them to root; Esc cancels. Triggered by
  `Ctrl+P` (`enter_reparent_mode`) on a selection.
- `Connect { source }` — the next left-click on a target node
  creates a `cross_link` edge from `source`; left-click on
  empty canvas cancels. Esc also cancels. Triggered by `Ctrl+D`
  (`enter_connect_mode`) on one node.
- `Resize { target }` — chrome shows resize handles on the
  target (a `ResizeTarget::Node(id)` or
  `ResizeTarget::Section { node_id, section_idx }`). Drag a
  handle to resize; Esc returns to `Default`. Triggered by `r`
  keybind on a selectable AABB or by `mode resize`. Touch peer:
  `LongPress` — deliberately native-only, see
  `TouchGestureRecognizer` below.
- `NodeEdit { node_id }` — chrome dims sibling nodes and frames
  the active node's sections in cyan. Click a section to lift
  it into a `Section` selection; Enter (or `section edit`)
  opens the inline text editor on the active section. Esc /
  outside-click returns to `Default`. `EnterNodeEdit` and
  `EnterNodeEditClean` ship **unbound** (`enter_node_edit: []`), so
  the mode is reached through `mode node-edit` / `node edit` on the
  console or a binding the user adds.

`src/application/app/interaction_mode.rs`. The enum is cross-
platform (compiles + the field plumbs through `InitState` /
`WasmInputState`); several entry-point Actions
(`EnterResizeMode`, `EnterNodeEdit{,Clean}`, `EnterSectionEdit`,
`FastResizeStart`) are NativeOnly today because they depend on
the cursor-driven modal-stealer + DragState machinery that's
native-gated — the `LongPress` touch default dispatches the same
NativeOnly `EnterResizeMode` and so reports `Unhandled` on WASM,
where a one-shot warn names it rather than the gesture vanishing.
That one is a deliberate posture, not a pending port; the rest of
the touch vocabulary (tap, one-finger pan, pinch) reaches no
`Action` at all and works on both targets. See
`TouchGestureRecognizer` below.
Modal-stealer cascades route keystrokes per active mode (the
keybind resolver keys on `(InputContext, key)` and the modal
stealer can intercept e.g. Esc before normal dispatch).

The console verbs `mode resize` / `mode node-edit` /
`mode default` ride the same surface; `section edit
[section=<idx>]` and `node edit` are sugar over the
mode-flip + side-effect handler.

See `work_plans/SECTIONS_BORDERS_RESIZE_PLAN.md` §2 for the design
problem this lifted, and §3-§4 for the resize / node-edit
mode UX. Plan §1 captured the three problems the
`InteractionMode` enum unified (the consolidation of the
pre-redesign `AppMode` into this enum is one of those).

### `DragState`

The drag state machine: `None` / `Pending` / `PendingRight` /
`Panning` / `SelectingRect` / `Throttled(ThrottledDrag)`.

Mouse-down does not commit to a drag yet —
the user might be clicking, or might be about to drag. `Pending`
captures everything the cursor was over at button-down
(`PendingRight` is its body-only right-button counterpart); once
movement crosses the drag threshold, the state transitions to
`Panning` (empty space), `SelectingRect` (Shift+drag on empty
space), or one of the seven `ThrottledDrag` variants depending
on what was hit.

`Pending` and `Throttled` carry boxed payloads, so `DragState`
itself is 64 bytes rather than the 912 the widest variant used to
impose on every state — including the `None` that is live for all
but a few seconds of a session. `PendingRight`, 64 bytes, is the
widest variant left and stays unboxed. Those three figures are the
ones the comment on `DragState::Pending` in `app/mod.rs` records
from the pass that boxed the two; no `size_of` assertion pins them,
so read them as that comment's measurement rather than as a checked
invariant.

**`SelectingRect` splits its per-frame work in two, and only one
half runs every frame.** The overlay rectangle tracks the pointer
and is redrawn unconditionally. The covered-node preview is
memoized: `drain_rect_select` hit-tests against the *installed*
tree (a rubber-band drag mutates nothing, so its geometry is
current) and repaints only when `set_rect_select_preview` reports
the covered set actually moved. The set itself lives on
`MindMapDocument` beside the other transient previews, so every
rebuild path paints it — including the animation tick's, which
runs after this drain in the same frame and used to wipe a preview
stamped onto a tree the drain had built for itself. Before #37 the
drain ran `doc.build_tree()` plus a full text-buffer rebuild on
every frame of the gesture, under a comment calling it a
"lightweight overlay redraw".

**The variant is the authority; both artifacts are its
projection.** Because the covered set is document state that every
node-tree rebuild reads, a gesture that ended without it being
dropped leaves the whole canvas painting a rectangle the user let
go of — and `SelectingRect` has more ways out than the left-release
arm that used to be the only one dropping it (a left press
overwrote it with `Pending`, `Action::PanCanvas` with `Panning`,
and a target-picker mode swallowed the release entirely). So the
drain runs on *every* frame, not only the gesture's, and
re-derives the overlay rectangle and the covered set from the drag
state it finds; the two eager calls that remain (the release arm,
the target-picker mode entries) exist only so the rebuild they run
in the same breath does not paint the set one last time. Nothing
has to enumerate the exits, so a new one cannot strand it.

One consequence of hit-testing the installed tree is worth naming:
the drain runs *before* the animation tick in the same frame, so a
node an animation is advancing is hit-tested one tick behind where
it is drawn. The window is a single frame and only opens while an
animation and a rubber band are live at once; closing it would
mean either ordering the animation tick first (which would make
the rectangle lag the pointer) or going back to a per-frame arena
build.

`src/application/app/mod.rs`. Native-only today.

**Hit priority, in the three parts it actually has.** The line
that used to sit here read "edge handle > portal label > edge
label > node, so small grab-areas always win over larger AABBs",
and it was wrong in both clauses. What is true:

- **Capture, at button-down.** One chain runs, `compute_click_hit`,
  and it gives the *node* priority: a portal hit is resolved only
  when no node was hit, an edge-label hit only when neither was.
  `portal_label_drag_capture` then arms the portal drag on the
  same condition. So a marker or a label sitting over a node
  loses the press to the node — on the drag path and the click
  path alike, because there is one chain and the drag reads its
  answer. A larger AABB in front wins.
- **Capture, outside that chain.** The press also runs three
  handle hit-tests that never consult `hit_node`, each gated on
  state instead: `hit_edge_handle` on the selection being
  `SelectionState::Edge`, `hit_node_resize_handle` on
  `InteractionMode::Resize { Node }`, and
  `hit_section_resize_handle` on `Resize { Section }`. Handles
  are what the mode or the selection *is for*, so a press near
  one is read as aimed at it whatever the chain says.
- **Promotion, at threshold-cross.** `event_cursor_moved.rs`
  consumes the captured hits in the order node-resize-handle →
  section-resize-handle → edge-label → portal-label → edge-handle
  → section-move → node-move → Shift-rect-select → pan
  (`Grab::PRECEDENCE`). Between the two label hits this order
  decides nothing, since the capture rule above leaves at most one
  of them. What it does decide is **all three handle families
  versus the node**: each is ranked ahead of `hit_node`, and each
  was captured without asking about it. It also decides **a
  resize handle versus a label lying over it**: in Resize mode the
  mode's own handles outrank the label grabs, since the mode
  exists for them, and a label stays grabbable outside it (#218).
  The resize handles are captured only in that mode, so outside it
  the order is the label-first one it always was.
- **Double-click, read off the same order.** What a press would
  drag is what it is: when `first_grab()` in `Grab::PRECEDENCE` is
  a handle (`NodeResizeHandle`, `SectionResizeHandle`,
  `EdgeHandle`), the press takes no part in double-click detection,
  neither completing one nor starting one
  (`PendingPress::is_handle_press`, #218). So a second press on a
  handle inside the double-click window grabs it rather than
  opening the editor on the node behind it. A press whose first
  grab is a label double-clicks as any press does, which is why a
  double-click on an edge's label keeps opening its editor even
  where the label sits on the selected edge's midpoint handle. The
  same rule reaches a node: with an edge selected, a press where
  one of its anchors or control points sits on a node is that
  handle's press, so a double-click there does not open the node's
  editor. The first release selects the node, which deselects the
  edge, and the next pair double-clicks as usual. That press does
  drag the anchor, so it is what it would drag.
- **The gesture starts at the press.** The threshold decides
  *whether* a press is a drag, not where the drag begins: the move
  that promotes it seeds the new gesture with the travel from the
  press position (`start_promoted_gesture`), so a dragged node, a
  resized box or a panned canvas tracks the pointer exactly rather
  than lagging it by the sub-threshold travel.

**Where a click and a drag on the same press name different
things** follows from that: the click ladder has no rung for
handles at all, so every hit in the second bullet is a target the
drag path can reach and the click path cannot. Three ways in, one
shape.

It is reachable, not theoretical. `resize_handle_positions` puts
the eight handles *on* the boundary of the box they resize and
`nearest_handle_within` accepts anything inside
`HANDLE_HIT_TOLERANCE_PX` of one, while `hit_test_target` returns
`None` for a point `point_in_node_aabb` refuses — and that
predicate is shape-aware. So a press a few pixels outside a
corner, or exactly on the corner handle of an ellipse-shaped node
(where the corner is outside the shape by construction), is empty
canvas to the click ladder and a resize to the drag: hold still
and the selection clears, move and the node resizes. Where the
handle sits *over* a node instead, the same press selects that
node on a click and drags the handle on a drag.

That is the intended shape — a handle is a drag affordance, and a
click through it falling back to selection is what makes the
handle non-modal — not a divergence to repair.

Issue #48 reported the *portal* case as such a divergence,
reading the promotion order without the capture rule.
`test_portal_label_drag_capture_is_gated_by_the_same_node_hit_the_click_ladder_uses`
now holds the two paths together, and names the gate that had
been an unpinnable inline `if` inside the press handler.

**A press never clobbers a gesture already in flight.** All three
press paths in `event_mouse_click.rs` check the drag state before
writing it, at two widths:

- **Right and middle** refuse whenever the state is anything but
  `None` (`handle_right_button`'s inline guard;
  `route_middle_button`'s `Keep`). Neither button has a re-arming
  role to preserve — a right press arms `PendingRight` and a
  middle press dispatches `MouseGesture::MiddleClick`, and both
  are meaningful only from rest.
- **Left** refuses on the narrower
  `DragState::would_abandon_gesture` class — `PendingRight` and
  `Throttled(..)` — because it *is* the arming press for
  `Pending`, and a click after a release the window never
  delivered has to re-arm rather than do nothing.

The class is the point: `Throttled(..)` owes the model a write
and an undo entry that only `commit_on_release_core` performs,
so replacing it leaves the tree holding dragged offsets that the
next model rebuild silently snaps back — position loss with
nothing to undo. `Panning`, `SelectingRect` and `Pending` owe the
model nothing and are re-derived from the next sample or press.

**The same class guards the *Action*, not only the presses.**
`Action::PanCanvas` is the one Action that writes a drag state
from rest, and it has three entry points, not one:
`MouseGesture::MiddleClick`, `MouseGesture::LeftDrag` (which only
ever runs from `Pending`), and any keyboard binding or macro step
naming `pan_canvas` — a first-class user-bindable entry that
`event_keyboard` dispatches with no drag-state check and that
`SourceTier::allows_action` does not gate. So the guard sits on
the arm (`dispatch::native::route_pan_canvas`, pure and pinned at
`DragState` level the way `route_middle_button` is), which closes
all three at once. `Pending` stays outside the refused class
precisely because the `LeftDrag` threshold cross dispatches
`PanCanvas` from it. The `RightDrag` cross does not need the same
exemption for `PendingRight`: it takes the press down to `None`
before it dispatches, with a hit built from the press, so a
`RightDrag`-bound `pan_canvas` arms from rest and the right release
ends it. `FastResizeStart` reads its target, its anchor and its
owning button off that hit rather than off `PendingRight` (#194),
which is what let the state come down first.

The release halves match, and the rule is the same one: **the
button that started a gesture is the one that finalizes it.**
`resolve_release` commits only on the owning button and answers
`PutBack` for every other, which both dispatchers honor by
restoring the drag state. A stray right-click cannot end a
left-button drag, and — since the left press started being
refused mid-gesture — a left release cannot end a right-started
fast-resize either; the right release, which the user still owes
because the button is down, commits it. Middle-click was the
exception on both halves until #37 — it overwrote any state on
press and forced `None` on release — and the right-button guard's
comment named that overwrite as the posture it was rejecting.

**A pan belongs to the input that armed it** (#153).
`Panning { owner: PanOwner }` records it, and the release rule
above extends to it unchanged: a pan ends on its owner's release
and on no other, so a middle release no longer ends a left-drag
pan, nor a left release a middle-started one. What `pan_canvas`
means on a key is the one decision this needed, and it is
**momentary**: a key holds the pan exactly as a button does, and
ends it on its own key-up (the Space-to-pan posture of most canvas
tools). The reason is that momentary is what a held input *is* —
the release is the "stop" every other pointer gesture here already
reads — and it makes a key's auto-repeat harmless, where a toggle
would flip the pan off and on while the key is down. An input
with no release — a macro step, a console line, IPC `act.action`
— cannot be momentary, so its pan is **modal**: `Toggled`, ended
by the next `pan_canvas` from a key, a macro, the console or IPC.
(Not from the mouse: a middle press arms only from `None`, so it
cannot reach the arm while a pan runs.) One rule covers both
halves: *held inputs hold, the rest toggle.* A touch `LongPress`
bound to `pan_canvas` is one of the rest — `HeldInput` has no
finger — so its pan toggles too. Whatever owns a pan, a left
*press* re-arms `Pending` over it, as it always has: `Panning` owes
the model nothing, and the release rule is about releases.

The Action cannot see which input reached it — the funnel hands it
only an `Action` — so the arm arms every pan `Toggled`, and a held
input claims the pan its own dispatch armed on the way out of
`dispatch_held_action` (the same read-the-decision-back shape the
`LeftDrag` first-delta gate uses). A key bound to something else,
pressed during a toggled pan, claims nothing: the claim is only for
a pan that did not exist before the dispatch.

**Focus loss resolves every state** (`DragState::end_on_focus_loss`,
`WindowEvent::Focused(false)`). Before #153 nothing ran on it, so a
`Throttled` gesture whose owning release never arrived — focus
lost, the OS eating the release — was unreachable. Now the state
ends at `None` and a throttled drag is **committed**, through the
same `resolve_release` gate with `Release::FocusLost`, which
answers `Commit` whichever button started it. Commit rather than
discard because commit is the reversible one: the model takes what
the user was looking at, with its undo entry, while a discard
snaps the canvas back and leaves nothing to redo. A pending press
is dropped without firing its click; a pan of either kind, and a
rubber band, are simply ended. Native-only, as `DragState` is.

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

Two consequences of that deletion, both since closed (#160).
**A touch-only user lost every resize, not only
`FastResizeStart`**: the route this paragraph used to offer —
long-press into `Resize`, then drag a handle — did not exist,
because a one-finger drag is always the recognizer's `Pan` and
moved the camera out from under the handle. Now a finger that
starts a gesture on a handle in `Resize` mode is the left button
until it lifts (`app/touch_handle.rs`): it drives the press,
cursor-move and release handlers a mouse drives, so the handle drag
is the one that already exists, and it is never fed to the
recognizer. Every other touch is unchanged, so one-finger pan and
pinch keep their meaning everywhere off a handle. That makes
`FastResizeStart` a mouse accelerator by design rather than by
accident. Native-only, as the drag state is. **And an existing
`keybinds.json` naming `"TwoFingerDrag"` was silently accepted and
never matched**, because `KeyBind::parse` takes any non-modifier
word; a retired name is now refused at parse time through the
`RETIRED_NAMES` table, with a diagnostic saying what replaced it.

### `ThrottledInteraction` and `ThrottledDrag`

A trait pair + seven-variant enum providing one uniform
shell for the whole lifecycle of a continuous, high-rate-input
drag.

Dragging a node, a section, a section's
resize handle, a node's resize handle, an edge handle, a portal
label, and an edge label all follow the same three-phase pattern:
fold each cursor sample into pending state, ask the throttle
whether to drain and apply if so, and commit to the model when
the button comes up. All three phases live here; new throttled
drags attach as one struct + one trait impl + one enum variant
without growing either event-file dispatcher.

`src/application/app/throttled_interaction/mod.rs`.

`ThrottledInteraction` is the drain shell: `pending()` /
`pending_mut()` are the only required state accessors, and
`has_pending`, `throttle`, `should_perform_drain`,
`needs_continuation` and the `drive` shell are all provided from
them. Implementors add a `drain(ctx)` body and, rarely, a `reset`.

`ThrottledDragInteraction` adds the two phases a drag has and the
picker-hover interaction does not: `accumulate(DragInput)`
(provided) and `commit_on_release_core(ReleaseCommit) ->
ReleaseRefresh` (required). The core is required so that a new
variant cannot compile with no release behavior at all, and it is
the *only* release entry point on the trait — `ReleaseCommit`
carries no renderer, so a commit body has nothing to reach one
with. Running the decree needs `&mut Renderer` and lives on
`ReleaseRefresh::execute`, off the gesture trait entirely. What
stays convention is the drain half: `drain` and its `drive` shell
take a `DrainContext` because a per-frame drain genuinely
repaints.

`ThrottledPending` (`throttled_interaction/pending.rs`) owns the
pending half. Three disciplines cover every implementor, and each
picks one at construction:

- **delta-accumulate** — the drain applies an incremental
  movement, so skipped samples sum;
- **cursor-latch** — the drain projects an absolute position, so
  only the last sample carries information;
- **dirty flags** — nothing accumulates; a flag says the next
  drain has work (the picker-hover interaction).

`DragInput` carries one cursor sample in *both* forms — the
canvas-space delta since the previous event and the absolute
canvas-space position now — so the dispatcher never has to know
which discipline the active gesture uses.

`ReleaseRefresh` (`throttled_interaction/release.rs`) is the
canvas work a commit owes once its model write has landed:
`None`, `SceneOnly`, or `All`. Named rather than performed, so
the commit body stays renderer-free.

**What ends a drag is a `Release`**: `Button(b)` commits only when
`b` started the gesture and hands the drag back otherwise;
`FocusLost` commits whichever button did, because the owning
release may never come and nothing else finalizes a throttled drag
(see [`DragState`](#dragstate)).

Variants:

- `MovingNode(MovingNodeInteraction)`
- `MovingSection(MovingSectionInteraction)` — drags one section's
  `offset` relative to its owning node; threshold-cross promotes
  here when the press lands on a section of a multi-section node.
- `SectionResize(SectionResizeInteraction)` — drags one resize
  handle of a `Some`-sized selected section. Threshold-cross
  promotes here when the press lands on one of the 8 handles
  (corners + edge midpoints); release commits a single
  `(offset, size)` write through `set_section_aabb`.
- `NodeResize(NodeResizeInteraction)` — drags one resize handle
  of a `Single`-selected node. Threshold-cross promotes here
  when the press lands on one of the node's 8 handles; release
  commits a single `(position, size)` write through
  `set_node_aabb`.
- `EdgeHandle(EdgeHandleInteraction)`
- `PortalLabel(PortalLabelInteraction)`
- `EdgeLabel(EdgeLabelInteraction)`

`as_dyn_mut()` / `as_dyn()` widen to
`&mut dyn ThrottledDragInteraction` — the drag trait, which has
`ThrottledInteraction` as a supertrait, so one ladder serves all
three phases. Those two are the only per-variant matches on the
*gesture* path: `event_cursor_moved`'s accumulate arm,
`event_mouse_click`'s left-release arm and its right-release arm
are each a single call through them. One more production-path
match exists by design — `ThrottledDrag::to_ipc_json` in
`app/ipc_state.rs`, the exhaustive projection `query.mode` reports
the gesture through (see [IPC server](#ipc-server-ipcserver)) — and
the test builds add three more — `variant_name` and
`latches_the_cursor` in this module's tests, and the enumeration in
`app/ipc_state.rs`'s own test module. Planting a variant yields
E0004 at exactly those three production sites, and at those three
test sites under `--all-targets`.

Touch does **not** ride this ladder. Pinch-zoom, one-finger pan
and tap-select ship as `TouchGestureRecognizer` emissions that
reach the camera and the selection directly, because the
recognizer already is the state machine `ThrottledDrag` would
have supplied and the camera work is `CODE_CONVENTIONS §3`'s
continuous-gesture carve-out. What would earn a variant here is
a touch gesture that *mutates the document* per frame — dragging
a node with a finger — and that wants the browser to have a
drag-state machine first.

### `MutationFrequencyThrottle` (and `frame_throttle`)

An adaptive frame-counter throttle that gates
*application* of mutations under load while leaving *acceptance*
of input untouched.

When per-frame work threatens the GPU
budget, the system must degrade gracefully. The non-negotiable
rule is **responsiveness is never traded for fidelity**: the
cursor must stay current with the hardware pointer at all
times, even if the dragged node updates only every fourth
frame. The throttle samples actual work duration into a moving
average; if the average exceeds budget, it raises `n` (the
"drain divisor"); if work is well under budget with hysteresis
margin, it lowers `n` toward 1.

`src/application/frame_throttle.rs` (`MutationFrequencyThrottle`).
`DEFAULT_BUDGET` is the per-frame budget (a 60 Hz frame minus a
safety margin), `WINDOW_SIZE` the moving-average window, `MAX_N` the
ceiling `n` clamps to above 1; the hysteresis band is a factor in
the arithmetic of the adjust step rather than a named constant, so
read it there. Each `ThrottledDrag` owns its own throttle, so
per-gesture profiles tune independently — a 500-node move
budget does not bias an edge-label drag's average.

### `UndoAction`

A 13-variant tagged union; one variant per
user-facing mutation, dispatched through `MindMapDocument::undo`
to reverse it.

Every persistent change pushes one
`UndoAction`; Ctrl+Z pops the back of the stack and dispatches.
The discipline is **one mutation, one variant** — adding a new
mutation means adding a new variant, snapshotting the right
"before" state, and writing the matching `undo()` arm in the
same commit.

`src/application/document/undo_action.rs`. The thirteen
variants, in declaration order (the count is read off that enum; the
IPC projection in `document/ipc_state.rs` and `undo()` itself are
the exhaustive matches that make a fourteenth a compile error, this
list is not): `MoveNodes`, `CustomMutation`, `ReparentNodes`,
`DeleteEdge`, `CreateEdge`, `EditEdge`, `CreateNode`,
`EditNodeText`, `EditNodeStyle`, `EditNodeZoom`,
`CanvasSnapshot`, `EditNodeAabb`, `DeleteNodes`. `CustomMutation`
is the general bucket — it snapshots the `target_scope`-defined
window so any declarative or imperative mutation replays
cleanly. Every arm is bounds-checked (e.g. `index <
edges.len()`) before mutating, so undo is always safe — never
panics, even on a partially-deleted state.

### The node-edit envelope and `NodeEditTail`

The one place the *push* side of an `UndoAction` is written for
node-scoped edits: snapshot → closure verdict → undo-push →
auto-fit.

`UndoAction` says what a reversal looks like; the envelope says
how a setter records one. Three of the variants describe a
per-node edit — `EditNodeStyle`, `EditNodeText`, `EditNodeAabb`
— and each used to be open-coded at every setter that produced
it, together with the `grow_one_node_to_fit_text` /
`grow_one_node_to_fit_border` tail. That fan-out drifted into
shipped bugs twice: a copy was corrected, its siblings were not.

`src/application/document/nodes/undo_envelope.rs` holds one
implementation. `mutate_node_with_style_undo` and
`mutate_node_with_text_undo` take a closure returning
`Some(value)` to commit or `None` to declare a no-op — on `None`
the envelope restores exactly the fields the undo entry would
have restored and pushes nothing, which is what lets a caller
mutate speculatively instead of reaching for the
`undo_stack.pop()` anti-pattern. `mutate_node_with_aabb_undo`
computes its own verdict by comparing `(position, size)`
**after** the auto-fit tail, which is what makes repeated writes
idempotent on a framed node whose border-grow overshoots the
requested size. Two section-scoped wrappers narrow the closure
to one `MindSection` and fold the index lookup in, so a stale
index is a no-op rather than a panic.

`NodeEditTail` is the fourth argument and the named policy for
what runs after a commit: `None` (color-only edits, which must
not re-measure), `Border` (the explicit-shrink and
border-config paths), `Grow` (anything that can change measured
text extent), `GrowAndCleanup` (the structural mutators — the
only edits that can strand a selection or a border preview on a
dead section index). Naming it is the point: an auto-fit pass
that is a copied suffix is a pass nobody chose.

The edge side has the same shape one layer over:
`MindMapDocument::mutate_edge` is the single `EditEdge`
envelope, and `edges/font_triple.rs` holds the one
`(size, min, max)` resolution — request ordering, inverted-bounds
guard, clamp — that the body, label, and portal-text font
channels share.

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
The dual pipeline lives here:
- **Rect / SDF pipeline** — node fills, ellipse SDF (shape-aware
  fills via `RECT_SHADER_WGSL`), background fills.
- **Glyph pipeline** — every visible character, via
  cosmic-text + glyphon atlas.
- **Positioned-glyph pipeline** (`glyph_runs/`) — every glyph of a
  [`GlyphRun`](#glyphrun) at its own canvas position, rasterized by
  cosmic-text into an atlas of its own (glyphon's cannot be reached
  from outside it) and drawn as one instanced quad per glyph, in one
  draw call per frame.

Sub-passes sit in `borders.rs`, `console_pass.rs`,
`color_picker.rs` and `selection_overlay.rs`; connection glyphs
have no pass of their own — they are a canvas-role tree walked by
`tree_walker.rs` like every other role. Visibility culling is
`Camera2D::visibility`, which answers the spatial cull and the
[`ZoomVisibility`](#zoomvisibility) window in one verdict.

### `AppScene` and scene host

A two-role scene container: a camera-transformed
canvas and a screen-space overlay, each composed of named
sub-trees.

Mindmap content (nodes, borders, connections,
portals, edge handles) belongs in the canvas role — pans and
zooms with the camera. The console and color picker belong in
the overlay role — fixed in screen space. The `AppScene`
abstracts that split; rebuild dispatch
(`InPlaceMutator` for small mutator-able changes,
`FullRebuild` for structural changes) flows through the same
seam for both roles. Canvas roles that also record a *content*
signature ask both questions through one entry point,
`canvas_dispatch_with_content`, which returns a third outcome —
do nothing, because the registered tree is already what a
rebuild would produce. One function rather than two calls per
role because the *order* is the design: structure first, content
second, and swapping them still renders correctly while undoing
the whole point (see
[canvas-role projection](#canvas-role-projection)).

`src/application/scene_host.rs` (`struct AppScene`). Each
role has named slots (`CanvasRole`, `OverlayRole`); each slot
has a corresponding `GfxTree` and a
mutator registry. The same idiom drives both canvas-role
rebuilds (in `scene_rebuild.rs`) and overlay-role rebuilds
(console text changes, color picker re-layout).

### Scene shape cache

The rule that decides, per walked element, whether
its cosmic-text buffers and background fill are re-shaped, moved,
or reused — one mechanism serving both of
[`AppScene`](#appscene-and-scene-host)'s sub-scenes.

Rebuild dispatch stops at the arena. A mutator
apply reaches every slot of a role by design, and every writer
above it assigns every field whether or not the value moved, so
"mutate, don't rebuild" bought a reused arena and then paid for a
full re-shape of it anyway: a picker hover shaped all 59 overlay
elements, a console keystroke all ~50 rows, and every one of the
ten `flush_canvas_scene_buffers` call sites shaped every buffer of
all eight canvas roles no matter which one it had re-projected.

`src/application/renderer/scene_shape_cache.rs`.
`refresh(scene, ids, shaped, kind)` walks the sub-scene in layer
order and keeps, per walk position, a `ShapedSceneElement` holding
the output *and a verbatim copy of every input it was shaped from*
— tree id, `unique_id`, registered offset, and the `GlyphArea`.
Nothing announces a change: the pass re-validates against the live
tree, which is why a new writer of scene-tree state cannot make it
stale by forgetting to notify anyone, and why the ten canvas call
sites are answered as a class rather than one at a time.

Two details are load-bearing. `GlyphArea`'s derived equality is
blind to a region *recolor* — `ColorFontRegion`'s `Eq` is set
identity by range — which is exactly what a picker hover changes,
so the check adds `ColorFontRegions::same_content` on top of `==`.
And each element's background fill lives inside that element's own
entry rather than in a flat list, so a partial re-shape cannot
disturb the index order the rect pipeline paints in; the mindmap's
own keyed re-shape, whose fills *are* flat, needs
`BackgroundRectSlot` to hold that line by bookkeeping instead
(`renderer/tree_buffers.rs`).

The key has two halves, and a mismatch is classified rather than
merely detected (`ShapedSceneElement::classify` → `Verdict`).
`GlyphArea::position` and the registered tree offset never reach
cosmic-text — the walker adds them to each buffer's `pos` after
shaping — so when they are the *only* difference the entry is
`Moved`, and `patch_placement` rewrites its buffers' positions and
its fill in place with no shaping and no `FONT_SYSTEM` lock. Any
other difference is `Stale` and re-shapes. The patch re-derives
each buffer from the stamp offset the walker recorded
(`MindMapTextBuffer::emission_offset`), the same fix the mindmap's
`patch_drag_positions` carries: an outlined element's eight halo
stamps do not sit at its anchor, and writing the anchor flat would
collapse them under the glyph. A picker drag-move — every cell
repositioned, nothing else changed — is the case this exists for.

`ScenePassKind` is the whole difference between the two passes:
the canvas keeps the walker's fills because they reach a draw
pass, the overlay discards them because no screen-space rect
pipeline exists yet. `refresh` takes a `&Scene` rather than a
`&mut Renderer` so its granularity can be asserted without a live
device (TEST_CONVENTIONS §T8) — the same reason
[`RebuildTier`](#scene-rebuild-granularity) is a value rather than
a statement.

### Scene rebuild granularity

Whole-canvas rebuild functions, a selection-change chooser, a
camera-driven re-projection, and one updater per canvas role — each
scoped to a specific change kind.

Different changes invalidate different
amounts of work. Editing a node's text might change its width
(full rebuild); dragging a node only moves connection paths
(connection-only rebuild); changing a portal endpoint color
only touches portal markers (portal-only rebuild). Each tier
is dispatched explicitly so the cheapest one runs.

`src/application/app/scene_rebuild.rs`.
Free functions: `rebuild_all` (node tree + every canvas role),
`rebuild_scene_only` (reuse the node tree, refresh every canvas
role), `rebuild_selection_highlight` (node text buffers only — no
canvas roles, no mode-status line: what a change to *which nodes
are highlighted* actually needs), `rebuild_after_selection_change`
(the chooser, below), and `rebuild_camera_geometry` (the
zoom-dependent roles only — connections, labels, portals, edge
handles — which both targets reach: native from
`drain_camera_geometry_rebuild` once per frame under the renderer's
flag, the browser from its wheel handler; `rebuild_camera_geometry_at`
is the screenshot engine's variant that projects at another camera
and back). Then the per-role methods on
[`CanvasFrame`](#canvas-role-projection) —
`update_connection_trees` (edges + their grab handles),
`update_portal_tree`, `update_border_tree`,
`update_connection_label_tree`, `update_section_frame_tree`,
`update_node_resize_handle_tree` and
`update_section_resize_handle_tree` — each callable on its own so a
caller refreshes only what its interaction can change, with
`update_all` as the every-role batch the whole-canvas tiers call.

**`RebuildTier` names the top two rather than running them.**
`RebuildTier::{All, SceneOnly}` is the choice between the first
two as a value, with `execute` as the only place either is
actually called. The reason is the same one
[`ReleaseRefresh`](#throttledinteraction-and-throttleddrag) has on
the drag-release path: both tiers need `&mut Renderer`, so an
interaction that *decides* a tier could not be tested without
a device while deciding and performing were one statement
(TEST_CONVENTIONS §T8). Handing the decision back as a value
is what lets a renderer-free test ask which tier a given
interaction picks.

Two constructors carry the rules:

- `for_selection_change(prev, new)` — `All` when either side is a
  node-ish selection (`Single` / `Multi` / `Section` /
  `MultiSection` / `SectionRange`), because section-area
  highlights are stamped into the node tree's
  `ColorFontRegions`; `SceneOnly` when both are edge-adjacent and
  only the scene-level cascade moves.
  `rebuild_after_selection_change` is this constructor plus
  `execute`, and stays the one-liner for callers with nothing
  else to weigh.
- `for_click(triggers_fired, prev, new)` — the same, except a
  fired `OnClick` trigger forces `All`, since a trigger's
  document actions are unbounded (a theme switch repaints every
  node). Both targets read this one function: native used to run
  `rebuild_all` for every click outcome, and the browser used to
  run the selection-delta tier even when a trigger had just
  mutated the document, so §4's peers were wrong in opposite
  directions.

`highlight_entries_for(doc)` is the one mapping every
node-tree rebuild site uses for *which* nodes to tint: the
rubber-band preview when one is live, `doc.selection` otherwise.
The preview replaces rather than adds to the selection's entries,
because the gesture's release writes `SelectionState::from_ids`
over the selection outright — painting both would show a set that
is about to be discarded.

Interactions that decide a tier are split into a renderer-free
core that returns one and a shell that runs it —
`click::handle_click_core` / `click::handle_click`,
`event_cursor_moved::arm_label_drag` /
`event_cursor_moved::start_label_drag`. Both label promotions
(edge-label and portal-label) go through the second pair, so they
cannot pick different tiers again.

### Dirty flag

A single `bool` on `MindMapDocument` marking
unsaved changes: set by every document setter, cleared at
construction and on a successful `save`.

The user must not silently lose work. The
flag is the "there are changes worth saving" bit guarding
destructive document swaps.

`src/application/document/mod.rs` (the `dirty`
field). Set by the setter families under
`document/{nodes,edges}/`; cleared at construction
(`document/mod.rs`), by the `save` console verb
(`console/commands/save.rs`), and by the Ctrl+S save
(`save_document_to_bound_path`, `app/console_input/exec.rs`);
read by the `open` and `new` verbs' guards, which refuse to
replace a dirty document ("unsaved changes; save before…").
Despite the name it is **not** a render or rebuild signal —
scene rebuilds are call-site-driven (`rebuild_all` after
mutations), and the per-frame drain consults the renderer's
separate `connection_geometry_dirty` flag instead
(`app/drain_frame.rs`). Nor is it a change counter: it is one bit,
reset by a save. "Did anything change" is the
[document revision](#document-revision), and the flag is turned on
through the same call that advances it (`mark_dirty`).

### Document revision

A monotonic `u64` on `MindMapDocument` counting committed model
changes, plus the funnel that made the latest one.

The flag above cannot answer "did anything change since I last
looked", and nothing else could cheaply either (the undo stack pops,
a re-serialization is O(document)). An IPC agent polling for change,
and the event stream that spares it the polling
([IPC server](#ipc-server-ipcserver)), need a number.

`src/application/document/revision.rs`. Advanced by exactly two
calls: `mark_dirty()`, which every setter ends in (a source scan
holds that nothing else writes `dirty = true`), and
`advance_revision()`, which undo calls because it mutates without
dirtying. A preview, a selection and the camera advance nothing.
`open` / `new` carry the counter across the swap (`succeed`) and
bump a `generation`, so it is monotonic per process. Funnels
declare themselves (`enter_source` / `leave_source` / `with_source`:
`dispatch_action`, the console line executor, undo, custom
mutations, animation commits) and the innermost wins, which is the
`source` a `document_changed` event reports.

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
`.status`, `.animations`, `.editors` and `.undo` return what a person
reads off the chrome. They go through `observe()` and `&self`
reads on `CommandTarget` (`document()`, `animation_pause()` and
`shell_state()`), so a query cannot dirty the document, push undo or
ask for a frame. Each wire form is an exhaustive-`match` projection beside the state it
exposes (`document/ipc_state.rs`, `app/ipc_state.rs`), not a mirror
struct, so a new `SelectionState`, `InteractionMode`, `DragState` or
`UndoAction` variant breaks the build until it is given one. The
`scene` family is what is drawn rather than what is known:
`scene.dump`, `.visible_set`, `.node_geometry`, `.hit_test` and
`.find_text` read the canvas trees through Baumhard's serializer
(`baumhard::mindmap::scene_dump`) — rects in both spaces, the
renderer's own cull verdict, and the text after run coverage — and
name each element through its role's hit-test index. `scene.hit_test`
runs the canonical click ladder on the app side
(`CommandTarget::hit_test`) rather than a copy of it. The `input`
family is the hand on the mouse and keyboard, below the Action
funnel: `input.mouse_move`, `.mouse_button`, `.wheel`, `.key`,
`.text` and `.touch` become the payloads winit events carry and are
delivered to the same `on_*` entry points those events
reach, so the drag state machines, double-click timing and modal
steals cannot tell them apart. `input.drag` paces its samples one per
frame through the injected-input queue (`ipc/injected.rs`), so the
throttled drag drains between its moves as under a real hand;
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
sidecar, read off the node tree through `scene_dump` at the capture's
camera; `png` encodes. `app/capture_flow.rs` is the app half both
surfaces call: it runs the camera-geometry rebuild the next frame
owes, and for a capture framed at another zoom projects the
zoom-dependent roles — connections with their edge handles, labels,
portals (`CanvasRole::zoom_dependent`) — into a scratch scene of the
capture's own (`app/capture_scene.rs`: its own `AppScene`, its own
`SceneConnectionCache`, its own hit indices), through the same
`CanvasFrame::update_zoom_dependent_roles` body the camera rebuild
runs on the live scene. The renderer walks the scratch into a second
shaped set and composes one frame of it with the live set's
zoom-independent roles by layer (`renderer::compose_capture_canvas`,
`CanvasSource::Composed`); the live scene, cache and shaped set are
read and never written, so there is nothing to project back. A node
drag's drained offsets are passed into the scratch projection, so a
capture at any zoom mid-drag shows the dragged node's connections
where the window does, and no `busy` refusal is needed. The scratch
cache stays warm across the frames of a recording at one zoom and
follows the live cache's `clear` / `invalidate_edge` statements
through `SceneConnectionCache::invalidations`, so it goes stale
exactly when the live one is told to. Every caller-chosen path is written through
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
move on command, while the FPS overlay, the freeze watchdog and the
drag throttle's budget must keep measuring real cost. So the old
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
same `animation_drain` body and, like native's loop frame, ticks on a
real clock only, so it would stand still under a virtual one — but the
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
   `dispatch_action`, `fire_custom_mutation`, or
   `execute_console_line` depending on `MacroStep` kind, so plugin
   authors and macro recorders share one runtime path. That path is
   `run_macro_steps`, the gated step loop `dispatch_macro` calls
   after its lookup. IPC's `act.macro` runs an anonymous step list
   through it too, so there is one privilege gate however a macro
   arrives. **Unknown
   macro id falls through** to the custom-mutation tier, so a
   typo'd or half-loaded macros file doesn't swallow the keystroke.
3. `keybinds.custom_mutation_for(...)` — per-node custom mutations.

**Built-in Actions win on collision.** A key combo bound to both
`Action::Copy` (in `copy: ["Ctrl+C"]`) and a macro on `"Ctrl+C"` in
`macro_bindings` runs the Action — the macro never gets a chance.
To override a built-in Action with a macro, first unbind the
Action's keybind (set `copy: []`) and then bind the macro. Same
applies for built-in vs. custom-mutation collision.

**Macro privilege model.** Macros are tagged by their loader tier
(`SourceTier { App | User | Map | Inline }`); the dispatcher
fail-closes on tier-restricted surfaces (`ConsoleLine` and
destructive / I/O `Action` variants). Authoritative threat model
+ surface enumeration + tier-by-tier permissions:
[`format/macros.md`](./format/macros.md). The `#[non_exhaustive]`
gate on `DocumentAction` ensures any new I/O variant must add a
matching dispatcher carve-out.

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

**Custom-mutation parity.** `dispatch_custom_mutation_for_key`
and the click-trigger loop (`click_triggers::fire_onclick_triggers`)
land through one body, `app::fire_custom_mutation`: animation-aware
(`start_animation_at` when `timing.duration_ms > 0`, ticked by both
targets' frames since #144), always invokes
`apply_document_actions`, and applies with or without an installed
tree (the no-tree decision on #144). Closes the silent feature gap
where keyboard-triggered custom mutations skipped both.

---

## §6 The authoring surface

Authoring surface concepts are the parts a user actually touches:
modal editors, the console, keybinds, the color picker, clipboard,
and (briefly) the `maptool` and `mandalactl` CLIs. Most are native-only today; the
parity story for each is honest.

### Inline node-text editor

Multi-line, grapheme-aware text editing on a
selected node; commit-on-click-outside, cancel on Esc.

Editing a node's text without leaving the
canvas. Double-click or Enter opens the editor; Backspace on a
selected node opens it pre-cleared; arrow keys move the cursor
in grapheme units. Live edits paint through a `DeltaGlyphArea`
mutation against the tree (not the model) so the user sees
in-flight characters; on commit, the model is updated and a
single `EditNodeText` undo entry is pushed.

`src/application/app/text_edit/mod.rs` (`TextEditState`; the
session body is `editor.rs`). Cross-platform — works on both native
and WASM. Cursor math
runs on grapheme-cluster indices throughout (via
[`grapheme_chad`](#utilities--grapheme_chad-color-geometry)),
so emoji and combining marks behave as single units. Original
text and regions are snapshotted on open; Esc restores them.

### Inline single-line editor

Single-line text editing for the two one-line strings the model
carries: a line-mode edge's label, and a portal endpoint's
caption.

Setting or changing either without leaving the canvas. Same
lifecycle as the node editor (commit on click outside, cancel on
Esc) but restricted to one line. Portal captions are
per-endpoint: selection and editing target one endpoint at a
time, and the other endpoint's caption is unaffected.

`src/application/app/single_line_edit/`. One `SingleLineEditor`
holds `{target, buffer, cursor_grapheme_pos, original}`; a
`SingleLineEditTarget` variant owns everything that differs
between the two — where the current value lives, which preview
slot on `MindMapDocument` feeds the renderer
(`label_edit_preview` vs `portal_text_edit_preview`), which
canvas role is re-projected per keystroke, which setter commits,
and what "the release landed back on the thing I am editing"
means. Adding a third single-line editable is a variant plus one
arm per method; the lifecycle, the modal steal, the
click-outside commit and the dispatch arms do not grow.

The lifecycle core is renderer-free and returns an `EditRefresh`
naming the canvas work it owes, so the whole open / type /
commit / cancel sequence is driven directly in renderer-free
tests (§T8).

One asymmetry survives on purpose: a portal caption stops being
editable once its edge is deleted or leaves portal mode, and a
mid-edit keystroke then closes the editor without committing.
The edge-label target has never had that guard and keeps typing
into a buffer whose edge is gone; its commit no-ops in
`set_edge_label`. `SingleLineEditTarget::still_editable` is where
the two answer differently, and the differential-oracle tests pin
both columns.

The `keybinds.json` vocabulary keeps its `label_edit_*` spelling
(`Action::LabelEdit*`, `InputContext::LabelEdit`) — those are
user-facing binding names, and both targets have always shared
them.

Native-only today. WASM users reach the same operations via the
`label` console verb, which has full cross-platform parity.

### Modal editor ladder

The steal / release shell both inline text editors sit in.

While a text editor is open it owns the keyboard: the key
resolves in that editor's input context, its commit / cancel pair
goes through the `dispatch_action` funnel, and everything else
reaches the editor's own handler as a literal `winit::Key`. On a
pointer release, a release inside the edited element keeps
editing and consumes the release; a release outside commits
through the funnel and lets the click route normally so the new
selection lands.

`src/application/app/modal_editor.rs`. `ModalEditor` is that
shell written once, over the node text editor and the single-line
editor; `event_keyboard.rs` has one steal block and
`event_mouse_click.rs` one click-outside-commit block rather than
three each. Steal order prefers the single-line editor; the
release ladder resolves the node text editor first. Both orders
are pinned by tests, because they are the kind of caller-level
contract a unit test on either editor cannot see.

### Glyph-wheel color picker

A modal HSV picker rendered as a hue ring of `HUE_SLOT_COUNT`
glyphs with sat/value crosshairs and a central preview glyph.

Picking a color for the current selection
without leaving the canvas. Hover live-previews through the
`color_picker_preview` transient on `MindMapDocument`; the
connection, label, and portal passes read it during projection
and substitute the preview color for the targeted element. Click commits, click
outside cancels. Keyboard: h/H nudges hue, s/S sat, v/V value,
Enter commits, Esc cancels. The theme-variable quick-pick chip row
and its Tab cycling are gone: the `ColorPicker` context's `Action`
set is cancel, commit and the six nudges, the layout carries no
chip positions, and `compute_sizing.rs` names the retired row where
its width reservation used to be.

`src/application/color_picker/mod.rs` (`ColorPickerState`,
`PickerMode`, `OpenPicker` in `state.rs`) and
`src/application/color_picker_overlay/`. Native-only today.
`compute_color_picker_layout()` is a pure function over
geometry + viewport, so layout can be unit-tested without GPU.
Two modes: contextual (modal, opened from edge context menu;
commits to the targeted edge and closes) and standalone
(persistent palette, opened via `color picker on`; commits to
the current selection and stays open).

Commit, cancel and the six HSV nudges are `Action`s that run in
`dispatch_action` like every other user-named effect
(CODE_CONVENTIONS §3), so a macro step can drive the picker.
`color_picker_flow::picker_op_for` is the single predicate the
funnel arm, the keyboard pre-filter and the click router share.
Only the picker's Copy / Paste / Cut stay modal-local — they
carry a hex payload no `Action` body can express.

### `BorderPreview`

A transient slot on `MindMapDocument` that stages border-config
edits without writing the model — the renderer substitutes the
preview style for the targeted node / section / canvas slot
while the slot is `Some(...)`, and the user terminates with
`commit` (writes through the matching committing setter) or
`cancel` (discards).

Authoring iteration on the four border surfaces (per-node,
per-section, two canvas defaults). Without preview, every kv
edit is a commit-then-undo cycle and the visual feedback comes
*after* the model write — the "creative toolkit" framing
depends on the user seeing changes before they land.

`src/application/document/nodes/border.rs` (the slot type +
setters) and `lib/baumhard/src/mindmap/tree_builder/overrides.rs`
(the borrowed view + injection in `border.rs` / `node_clip.rs`,
`section_frame.rs`). Same discipline as `ColorPickerPreview`:
never serialized, never push undo, never flip `dirty`. Cancel
or commit clears the slot; a fresh `set_border_preview` call
replaces the prior preview atomically. Selection drift causes
lazy defer-clear: the preview stops rendering when the live
selection no longer covers the target, and the actual slot
clear happens at the next `set_*` / `commit_*` / `cancel_*`
call. Implicit cancel: any of the four committing setters
clears the preview as their first line, so a non-preview edit
always wins. `Action::SetBorderPreview { target_kind:
BorderPreviewTargetKind, field, value }`,
`Action::CommitBorderPreview`, `Action::CancelBorderPreview`
expose the keybind / dispatch surface; `Esc` cancels through
`Action::ExitMode`'s body before mode-clear.

### `SectionFrameElement` and section-frame chrome

The cyan rectangles drawn around an active node's sections
while the user is in `InteractionMode::NodeEdit { node_id }`.
Each section gets one rectangle keyed on `(node_id,
section_idx, focused)`; the frame is heavier (or
canvas-default-overridden) on the section currently being
text-edited so the user sees which section their keystrokes
land in.

The chrome is a parallel canvas — it doesn't belong to the
node's own `GfxElement` tree, so a node move or text rebuild
doesn't re-emit the frames. The dedicated canvas role
`CanvasRole::SectionFrames` registers its own
`InPlaceMutator` slot; rebuild-or-skip dispatch keys on
`section_frame_content_signature(elements) -> u64` which
streams every field the projection reads directly into a
hasher (no intermediate Vec) so the signature comparison runs
allocation-free per `Plan §7.4`. This role needs only the one
signature because it has no in-place mutator body to protect —
see [canvas-role projection](#canvas-role-projection) for the
two-signature shape borders and portals use instead.

`lib/baumhard/src/mindmap/tree_builder/section_frame.rs` holds
all three halves — the `SectionFrameElement` shape, the
`build_section_frames` emission pass, and the tree builder +
content hasher. Three style cascades
feed the resolution: per-section `frame_border` →
`canvas.default_section_frame_border` (or
`default_focused_section_frame_border` for focused) →
hardcoded floor. Each cascade is editable through the
`section frame …` and `canvas section-frame [focused] …`
console verbs, plus the `BorderPreview` lifecycle.

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
Visuals borrow
`baumhard::mindmap::border::BorderGlyphSet::box_drawing_rounded`
for the frame; content is clipped via
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
  map's palettes, a node's sections, the host's font families)
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
  whichever key was typed first pick the form. That exclusion
  stays a handwritten guard in `execute_move`, with a message
  naming both shapes — one of the two bespoke semantics #27's
  fix plan names as staying behind the table.
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

**A command reaches the document through two different calls,
and which one it picks is the rebuild signal.** `ConsoleEffects`
hands out `document()` (shared) and `document_mut()`
(exclusive); the second raises `document_mutated`, which
`console_input/exec.rs` reads back in
`console_line_needs_rebuild` to decide whether the line owes a
`scene_cache.clear()` + `rebuild_all`. The other input is the
command's `ConsoleSideEffect`: every variant counts except
`SetFpsDisplay`, whose overlay is screen-space and shares no
state with the scene tree. So `help`, `fps`, `mutation list`
and any verb that fails after only reading no longer drop the
connection cache and re-project the whole scene for output that
never leaves the scrollback.

The signal is the *borrow*, not the write. It over-reports — a
`border reset` that turns out to be a no-op still counts — and
that is the safe direction, since the cost is one rebuild
against a canvas silently disagreeing with the model. It is
also the version a new command cannot forget: there is no way
to write to the document except through the call that raises
it.

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
`SetBorderField { field: "preset", value: "rounded" }` directly
in `keybinds.json` without authoring a macro. Bindings use a
sibling `ParametricBinding` shape:

```jsonc
{
  "set_border_field": [
    { "combo": "Ctrl+B", "args": ["preset", "rounded"] }
  ],
  "set_color": [
    { "combo": "Ctrl+1", "args": ["bg", "#fafafa"] },
    { "combo": "Ctrl+2", "args": ["text", "accent"] }
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

Color / font / zoom carry the axis as the first arg (`bg|text|border`,
`size|min|max`, `min|max` respectively) so a single binding-list
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

Tuple rows (`SetEdgeBodyGlyph(glyph)`) sit outside the const
check by construction rather than by omission: their names are
local bindings the table invents, and the pattern that binds
them and the constructor that consumes them are the same macro
repetition, so there is no second order to disagree with.

Three skips are possible at resolve time and all three are
logged, never panicked: an unparseable combo, the wrong number
of args, and an arg that is not a valid value for its field.
The dispatch arms call `pub(crate)` mutation cores extracted
from each console verb, so the same setter path runs whether
the user types the verb or fires the bound key — including
`CycleBorderPreset` / `ToggleBorderVisible` (cores in
`console/commands/border/execute.rs`) and the font-size slots
(one selection dispatcher in `console/commands/font.rs`).
Section-targeted Action variants resolve their `(node_id,
section_idx)` through the shared cascade in
`console/commands/section/target.rs`, whose
`SectionTargetPolicy` names the two places the Action path
legitimately differs from the verb path (no document to count
sections, so no single-section auto-resolve; a genuine
multi-section selection is rejected rather than collapsed). Filesystem
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

The `App < User < Map < Inline` ladder in
`src/application/source_tier.rs`, shared by the custom-mutation
registry and the macro registry — both are id-keyed and both take
definitions from the same four places. The ladder means two
things at once: **precedence** (later tiers override earlier ones
on an id collision, which the derived `Ord` encodes) and **trust**
(`App` ships with the binary and `User` is the user's own file,
while `Map` and `Inline` arrive inside a possibly-shared
`.mindmap.json`). The macro dispatcher's privilege gates —
`allows_console_line`, `allows_action`, both `impl SourceTier`
blocks in `src/application/macros/mod.rs` — key off exactly that
trust split. Tier assignment is loader-pinned: nothing in an
on-disk file can raise its own tier.

### Clipboard

Cross-platform copy / cut / paste, with native
backed by `arboard` and WASM stubbed pending async-clipboard
integration.

Selection-routed clipboard: each
[`SelectionState`](#selectionstate) variant has its own channel.
Copying a node copies its style and text; copying a section
copies a structured payload (text + per-run formatting + offset
/ size / channel / bindings); copying an edge copies the body
color; copying an edge label copies the label color; copying
a portal label copies the icon color; copying a portal text
copies the text color. The font channel mirrors this routing
for `font size= min= max=` writes.

`src/application/clipboard.rs`. The OS
clipboard layer (native `arboard`, WASM stub) carries plain
text. A thread-local in-process `SECTION_BUFFER` slot carries
the structured `SectionPayload` for within-app section→section
round-trip; on paste, the payload is consulted only when its
`text` snapshot matches the OS clipboard's current text exactly
(consistency check; falls through to plain text when the user
copied from another app between Mandala copy and paste).
Failures on native are silent by construction — `read_clipboard`
maps a failed `arboard` call to `None` through `.ok()` and
`write_clipboard` discards its `Result` — because interactive
paths must not panic, and a clipboard that is merely unavailable
is not worth a log line per keystroke. The WASM stubs return
`None` / do nothing and say so at `log::debug!`, pending the
browser's async clipboard API.

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
`.mindmap.json` files and never talks to a process. Its one client-side
command, `record assemble <dir>`, turns a recording's frames and manifest
(see [Frame-sequence recording](#frame-sequence-recording)) into a GIF, or an
MP4 through the host's `ffmpeg`, and never opens the socket.

`crates/mandalactl/`. Protocol: [`format/ipc.md`](./format/ipc.md); the full
agent-facing reference is `mandalactl --help`.

### `maptool` CLI

A separate binary in `crates/maptool/` for
scripted operations on `.mindmap.json` files: `show`, `grep`,
`apply`, `export`, `convert --legacy`, `convert --portals`,
`convert --sections`, `verify`.

Authoring and maintenance from outside the
app. `verify` is the structural-invariant checker
([`format/validation.md`](./format/validation.md)). `convert`
migrates legacy formats — `--legacy` runs the portal and section
folds inside itself, so a miMind import is one hop
([`format/migration.md`](./format/migration.md)); every verb writes
through an atomic staging file + rename, so input and output may be
the same path. `apply` pipes node text through an
external command for batch edits. `export` renders to Markdown.
`grep` and `show` are read-only inspectors.

`crates/maptool/`. Not the focus of this
document — see the crate directly for the verb-level reference.
The format docs under [`format/`](./format/) are the
authoritative reference for what `verify` enforces.

---

