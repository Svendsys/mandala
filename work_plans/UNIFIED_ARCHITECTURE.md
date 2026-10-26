# Unified Architecture — one mutation engine for data and view

**Status:** design, under review. Nothing here is implemented yet.
**Epics:** V1 (this design) and V2 (deferred features) — drafted in
[`UNIFIED_ARCHITECTURE_ISSUES.md`](./UNIFIED_ARCHITECTURE_ISSUES.md).
**Delivery:** an integration branch, `epic/unified-mutations`; `main`
receives one switch-over merge when V1 is complete (§17).
**Compatibility:** none (CODE_CONVENTIONS §10). The app has no users. The file format, the
mutation format, the in-memory model and every internal API are
redesigned to fit one architecture; nothing is kept to cater to the
current shape.

This document is the single reference every V1 sub-issue points to.
Where a sub-issue and this document disagree, this document wins
until it is amended — and it is amended in the same PR that changes
the decision, never after.

---

## §0 Decision log

The decisions below were made with the project owner and are not
reopened by implementation sub-issues. Each names the section that
develops it.

| # | Decision | § |
|---|---|---|
| D1 | The document itself becomes a baumhard `Tree`, mutated by the same engine as the glyph trees. No reverse sync from view to data, ever. | §3, §5 |
| D2 | Text is a sequence of **span** elements (text + style), not ranges over a string. | §5, §6 |
| D3 | Structure is nesting. Nodes carry a compact integer `id`, used only for references and external addressing. In memory, identity is the arena `NodeId`. | §5, §7 |
| D4 | The file format is strict, nested, typed JSON with a single `format` integer. Unknown fields are a load error. | §7 |
| D5 | A saved node `size` is the user's explicit size and acts as a **minimum**; the node grows to fit its content. Fitted sizes are derived, never saved. | §5, §12 |
| D6 | Two kinds of map behavior, separate types in code and separate lists in the file: **Edits** (change the map, saved, undoable) and **Presentations** (perform the map, never saved). | §10 |
| D7 | A map exposes **verbs**: named step sequences that triggers, links, the console and IPC invoke. Map verbs replace the map- and node-level macro tiers. | §11 |
| D8 | Frames, connections, sections and labels are all **TextFlows**: spans laid out in a box or along a path. A frame is text along the node's outline; a connection is text along its curve. | §6 |
| D9 | Glyphs are never rotated. Curves are made by **positioning** upright glyphs, the way ASCII art makes curves. | §6 |
| D10 | Frame fit mode, start point and direction are per-frame settings. Frames hold rings (stanzas). Frame font size is relative to the node's size. | §6 |
| D11 | Corner radius is a percentage of half the node's shorter side: 0% sharp, 100% fully round (circle on a square, pill on a rectangle). Capped at 100. | §6 |
| D12 | The node background stays a shader fill, shaped to match the frame (rounded rectangle, ellipse), with a configurable soft fade at its edge. | §13 |
| D13 | Deleted, not deferred: node `notes`, underline, node `layout`, `show_shadow` (shadow is deferred to V2), border `padding`, edge `width`, edge `line_style`, the `OnKey` trigger, `corner_radius_percent` / `frame_thickness` as they exist today (replaced by D10/D11). Edge width and line style are expressed through connection content and relative size. | §5 |
| D14 | Kept and made real: bold, italic, per-span font size, hyperlinks (links target a URL or a verb), node-level hover. | §5, §11 |
| D15 | Every stage after load does work proportional to what changed. No full-scene rebuild, scan or signature pass on an edit. | §2, §12, §13 |
| D16 | Delivery on an integration branch; `main` gets one merge. Features may be temporarily absent on the branch between the cut-over and their sub-issue; `main` never sees that. | §17 |

---

## §1 Why

Research for this design (October 2026) found that baumhard's
mutation system — the part of the project with the most thought in
it — is used only for the view, while the data model is edited by
hand-written Rust. The two have drifted since the second day of the
project. The specific findings:

1. **The engine is generic in name only.** `Tree<T, M>` has type
   parameters, but the walker (`walk_tree_from`), `MutatorTree::apply_to`,
   `Predicate`, `GfxMutator`, `Mutation`, `MutatorNode` and
   `mutator_builder::build` are all written against `GfxElement`
   and its `GlyphArea` / `GlyphModel` fields. Baumhard *cannot*
   mutate anything but glyphs. An agent that wanted to change data
   through a mutation had nothing to call — the technical root of the
   drift.
2. **The model is not a tree.** `MindMap.nodes` is a flat
   `HashMap<String, MindNode>` plus `edges: Vec<MindEdge>`. Structure
   is stored three times — `parent_id`, `parent_child` edges and the
   Dewey id — and reparent already lets them disagree
   (`format/ids.md` §"ID stability across reparent").
3. **Writes have no funnel.** About 70 non-test sites write the model
   directly, under 13 `UndoAction` variants and four undo-envelope
   helpers. Animation ticks, edge-handle drag drains and the load-time
   grow passes write with neither undo nor `mark_dirty`. A
   multi-selection edit pushes one undo entry per target.
4. **The node tree is rebuilt on nearly every commit** — including
   every advancing animation tick and every hover-target change in
   Reparent/Connect mode — and the renderer re-shapes every node
   buffer each time. The eight canvas roles re-run their data pass
   over the whole model and diff by signature; that is the one place
   "mutate, don't rebuild" is practiced.
5. **Custom mutations never run on the walker.** Only "flat" mutator
   shapes are accepted (`flat_mutations`); channels, filtering
   predicates, `Repeat`, `MapChildren` and runtime holes are skipped
   with a log line. Persistent results reach the model through
   `document/custom/sync.rs`, 1,059 lines of heuristics that reverse
   a lossy projection. Animations interpolate position only, and the
   nudge vocabulary is defined three times. CONCEPTS §4,
   `format/mutations.md` and `mutation inspect` all describe a walker
   path that does not exist.
6. **The model carries debt beyond the split.** Dead fields (`layout`,
   `notes`, `corner_radius_percent`, `frame_thickness`, `show_shadow`,
   edge `width` / `line_style`, border `padding`); dropped fields (run
   bold/italic/underline/hyperlink, per-run size collapsed to a max,
   border and connection `font`, control points past the second);
   every enum, color and font family kept as a `String` and parsed
   inside projection passes, colors theme-resolved four to six times
   per node per build; derived data persisted (grown `size`,
   `color_schema.level`, baked colors, fork-on-edit copies of canvas
   defaults); var refs that never resolve in border colors and palette
   groups; whole-struct rather than per-field default inheritance.
7. **Every border and connection glyph is its own `GlyphArea`,
   shaped as its own cosmic-text buffer.** That shape cannot carry
   text-bearing frames or connections at any useful density.

How it happened: the serde model existed before the tree bridge; the
tree bridge was built *from* it; the drag milestone on 2026-04-10
settled on "preview on the tree, commit to the model, rebuild", and
every later feature copied it. The written rules said "where
possible", §B2 was read as an arena-reuse rule, and the walker-based
apply path was deferred on 2026-04-17 and never scheduled.

The fix is not a patch to any one of these. It is one architecture in
which they cannot occur.

---

## §2 Principles

These are the tests every V1 sub-issue's review applies.

1. **One engine.** Every state change in the app — to the map's data
   or to what is drawn — is a baumhard mutator tree applied by the
   walker to a baumhard tree. There is no second way to change state.
2. **One home per fact.** A fact is stored once. Anything that can be
   computed from stored facts (tree structure, theme depth, fitted
   sizes, resolved colors, sibling order) is computed and never
   saved.
3. **Typed at rest.** Strings are parsed once, at load, into enums,
   RGBA values, interned font ids and theme-variable indices. No
   projection, drag drain or frame parses a string.
4. **Work proportional to change.** After load, every stage handles
   exactly what changed: a document journal names the touched
   elements, the projector re-projects those and their dependents,
   the glyph journal names the touched glyph elements, and the
   renderer reshapes or re-places exactly those. Panning does
   nothing to any tree.
5. **The data and the view agree exactly.** Every document field has
   exactly one projection. Nothing is stored that is not drawn or
   used; nothing is drawn that is not stored or derived from stored
   facts.
6. **Changing the map and performing the map are different things,
   and the code makes it impossible to confuse them** (§10).
7. **Both targets, always.** Everything in this design compiles and
   runs on native and wasm32. The CLAUDE.md "Dual-target status"
   carve-outs are unchanged by it, and it adds none.
8. **Interactive paths never panic** (CODE_CONVENTIONS §9 is
   unchanged). A malformed mutator degrades to a no-op with a
   diagnostic; it never aborts an apply half-way, because the journal
   guarantees an apply is all-or-nothing (§8).

---

## §3 Overview

```
 Edit mutators ─┐   (actions · console · IPC · drags · edit handlers · verbs)
                │   walker: kind+channel routing, anchored → Journal
                ▼
 Document  =  Tree<DocElement, DocMutation>  ◀── load / save ──▶  .mandala.json (§7)
                │
 Presentation layer (§10): presentation mutators re-applied on top; never saved
                │
                │  Journal ⊕ dependency index (§12) → dirty set
                ▼  projector — forward only
 Glyph mutators ── walker ──▶ glyph layers  =  Tree<GfxElement, GfxMutation> per layer
                │                       │
                │                       └── glyph Journal → reshape / re-place set (§13)
 View-only glyph mutators                ▼
 (selection, dimming, hover chrome,   renderer: TextFlow runs, backgrounds → GPU
  glyph-layer presentations,
  console, color picker)
```

The same engine — one tree type, one walker, one mutator language,
one journal — runs in two instantiations: the **document**, which is
saved, and the **glyph layers**, which are drawn. Data only ever
flows downward. Nothing reads the glyph layers to learn what the
document is.

Four flows, end to end:

- **An edit** (recolor the selection): the action builds one
  `DocMutation` mutator anchored at every selected node →
  `Document::apply` walks it, journals the touched fields → one undo
  entry → the projector maps the journal through the dependency
  index (the nodes, their frames, the connections whose color
  inherits from them) → glyph mutators update exactly those glyph
  elements → the renderer reshapes exactly those runs.
- **A drag**: a transaction (§9). Each drain frame reverts the
  previous frame's journal and applies `position += total_delta` —
  two small journals, the same projection path. Release commits one
  undo entry; Escape reverts.
- **A hover presentation**: the trigger invokes a verb (§11); the
  verb starts a presentation (§10) on the hovered node; the
  presentation layer applies it, animated, on top of the document;
  projection is the same as for an edit; the document is never
  marked changed. Leaving the node plays it back out.
- **Load**: parse (§7) → build the document tree → one full
  projection into the glyph layers → shape every run once. This is
  the only full pass in the life of a document.

---

## §4 The engine (baumhard)

Everything in this section lives in `lib/baumhard` and is generic.
The glyph instantiation is the first client; the document is the
second. Neither is special-cased in the engine.

### §4.1 Elements and routing keys

```rust
pub trait TreeElement: Clone {
    type Kind: Copy + Ord + Eq + Hash + Debug;
    fn key(&self) -> BranchKey<Self::Kind>;
    fn void() -> Self;
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct BranchKey<K> { pub kind: K, pub channel: u32 }
```

The walker routes on `(kind, channel)`, not on channel alone. A
mutator addressed to `(Section, 0)` cannot land on `(Node, 0)` or
`(Link, 0)` under the same parent. This removes the class of problem
the current section fan-out hack in `document/custom/mod.rs` exists
for, and lets one parent hold children of several kinds whose channel
spaces are independent.

**Sorted-children invariant.** A parent's children are always kept in
`BranchKey` order (stable among equal keys). Structural commands
(§4.4) insert at the right place. The walker therefore merges a
mutator row against a target row by walking both sibling lists in
step, with no `Vec` allocation and no sort — today's walker collects
and sorts both rows at every visited node (`collect_sorted_children`).

### §4.2 Mutations and mutators

```rust
pub trait Mutation<E: TreeElement>: Clone {
    /// Apply to one element. The journal sink records the element's
    /// before-image the first time it is touched within a journal.
    fn apply(&self, target: &mut E, sink: &mut JournalSink<'_, E>);
}

pub enum Mutator<E: TreeElement, M: Mutation<E>> {
    Void        { key: BranchKey<E::Kind> },
    Single      { key: BranchKey<E::Kind>, mutation: M },
    Macro       { key: BranchKey<E::Kind>, mutations: Vec<M> },
    Instruction { key: BranchKey<E::Kind>, instruction: Instruction<E>, mutation: Option<M> },
    Structure   { key: BranchKey<E::Kind>, op: StructOp<E> },
}
pub type MutatorTree<E, M> = /* arena of Mutator<E, M> + root */;
```

`GfxMutator` becomes `Mutator<GfxElement, GfxMutation>`; the document
uses `Mutator<DocElement, DocMutation>` (§5). Instructions keep their
meaning: `RepeatWhile(Predicate)`, `MapChildren`, and
`SpatialDescend(point)` — the last available only for element types
that implement `Spatial` (glyph elements do; document elements do
not). `RotateWhile`, a reserved and never-implemented stub today, is
removed from V1 and documented in the V2 epic.

### §4.3 Predicates, the AST and the builder

`Predicate` becomes `Predicate<E>` over a per-element field selector
(`E::Field`), with the existing `Comparator` set. The `MutatorNode`
AST, `Repeat`, runtime holes (`SectionContext`, renamed
`BuildContext`) and `build` become generic over `(E, M)`. One JSON
authoring surface therefore serves document edits, presentations in
document vocabulary, and glyph-layer presentations; the element type
is fixed by where the mutator is used, never guessed.

### §4.4 Anchored walks and structural commands

```rust
impl<E, M> MutatorTree<E, M> {
    pub fn apply_at(&self, tree: &mut Tree<E>, anchors: &[NodeId], journal: &mut Journal<E>);
}
```

A walk starts at its anchors — typically the selection — and the
mutator's root matches the anchor element. Work is proportional to
the reached subtree, never to the whole tree. Relative anchors
(`Parent`, `Siblings`) resolve to concrete anchors before the walk;
`Siblings` anchors at the parent with an `IsOrigin` predicate field
excluding the origin.

**Structural commands** are a first-class mutator variant:

```rust
pub enum StructOp<E> {
    Insert { subtree: Subtree<E>, at: Placement },   // child of the target
    Remove,                                           // the target and its subtree
    Move   { to: AnchorRef, at: Placement },          // re-parent the target
    Split  { at: E::SplitPoint },                     // e.g. a span at a grapheme
    Merge  { with: Sibling },                         // e.g. two adjacent equal-style spans
}
```

They apply in a **second phase**: the walk collects `(target, op)` in
walk order and applies them after it ends, so the walk never iterates
a sibling list it is changing. Removal **detaches** rather than
frees: the subtree stays in the arena, owned by the journal entry,
so undo re-attaches the same `NodeId`s and every reference to them
stays valid. The arena slot is freed when the journal entry leaves
the history. indextree 4.9's per-slot stamps make any stale `NodeId`
detectable.

### §4.5 Journals

```rust
pub struct Journal<E: TreeElement> { entries: Vec<Entry<E>> }
enum Entry<E> {
    Field  { node: NodeId, before: E, after: E },
    Insert { node: NodeId, parent: NodeId, prev: Option<NodeId> },
    Remove { node: NodeId, parent: NodeId, prev: Option<NodeId> },  // subtree kept detached
    Move   { node: NodeId, from: (NodeId, Option<NodeId>), to: (NodeId, Option<NodeId>) },
}
impl<E> Journal<E> {
    pub fn revert(&self, tree: &mut Tree<E>);   // exact inverse, newest first
    pub fn replay(&self, tree: &mut Tree<E>);   // exact forward, oldest first
    pub fn touched(&self) -> impl Iterator<Item = (NodeId, Touch)>;
}
```

- A field entry is written once per element per journal (the first
  touch records `before`; `after` is filled when the journal closes),
  so a mutator that touches a node many times costs one entry.
- `revert` and `replay` are exact. Undo and redo are `revert` and
  `replay`; nothing is re-evaluated, so an undo can never disagree
  with what the edit did.
- `touched()` is what the projector (§12), the IPC event stream
  (§14) and the renderer (§13, for the glyph journal) consume.
- Elements are kept small enough that a before/after clone is cheap:
  text lives in span children (§5), not in nodes.

### §4.6 Interpolation

```rust
pub trait Interpolate: Sized { fn lerp(from: &Self, to: &Self, t: f32) -> Self; }
```

Implemented for scalars, `Vec2`, sizes and RGBA. Each element type
declares which of its fields interpolate; the rest switch at a
declared point (`snap: start | end`, default `end`). Given a journal
(`before`, `after` per touched element), an animation at progress `t`
writes `lerp(before, after, t)` field-wise. This is what makes any
numeric, position or color change animate (§10) without per-field
animation code.

### §4.7 Cost rules the engine keeps

- No allocation per visited node on the walk (sorted-children
  invariant).
- No string compares or hashing on the walk; keys are `Copy` integers.
- Journals allocate per touched element, not per visited element.
- Every new primitive in this section ships a bench entry beside it
  (baumhard CONVENTIONS §B8); `cargo check --workspace --benches`
  proves they build. Benchmarks are not run by agents (CLAUDE.md §6),
  and no performance claim is made — work removed is stated as a fact
  visible in the diff.

---

## §5 The document

The document is `Tree<DocElement, DocMutation>` (§4), owned by a
`Document` type in `lib/baumhard/src/document/`, which replaces
`MindMap` and the free-standing model under
`lib/baumhard/src/mindmap/model/`. `MindMapDocument` in the app keeps
what is genuinely app state — selection, camera, mode, the open
transaction, the presentation runtime — and holds a `Document`.

### §5.1 Shape

```
Map                       (root; exactly one)
├─ Theme                  variables (name → RGBA), variants (name → variable set)
├─ Palette*               ordered color groups (fill / frame / text / title per depth)
├─ Defaults               per-field defaults for frames, rings, connectors, links, labels, spans
└─ Node*                  roots, in sibling order
    ├─ Section*           a box TextFlow
    │   └─ Span*
    ├─ Frame?             the node's outline TextFlows
    │   └─ Ring*          one stanza per ring, innermost first
    │       └─ Span*
    ├─ Connector?         the line to the parent (absent on roots)
    │   ├─ Span*          the line's content ("─", "·", a line of poetry)
    │   ├─ CapStart? / CapEnd?  → Span*
    │   └─ Label?         → Span*
    ├─ Link*              a cross-link or portal to another node
    │   ├─ Span* / CapStart? / CapEnd? / Label?        (line links)
    │   └─ PortalEnd ×2   (portal links) → Span* (marker), Label?
    └─ Node*              children, in sibling order
```

Each element kind has its own channel space (§4.1). Sibling order is
the arena order, which the sorted-children invariant keeps within a
`(kind, channel)` key in insertion order; reordering siblings is a
`Move` with a `Placement`.

**What this removes:** `parent_id`, `parent_child` edges, Dewey ids,
`edge_type`, the `MindMap.nodes` map and its key-equals-`id` check,
`color_schema.level` (depth is the tree), `theme_variables` as a
persisted copy of a variant (the live variant is a field naming it),
fork-on-edit copies of defaults, and the section-level trigger
bindings (triggers are node-level, D14).

### §5.2 Identity

- **In memory:** the arena `NodeId`. Selection, transactions,
  journals, the projection map, presentations and link targets hold
  `NodeId`s. No string is hashed to find an element.
- **On disk and from outside:** a node carries `id: u32`, unique within
  the map and never reused within it. It exists so links and verbs can
  reference nodes in the file, and so the console, IPC and
  `mandalactl` can name a node. One `id → NodeId` index serves those
  external callers and the loader; no internal path consults it.
- New nodes take `max_id + 1`. Ids do not encode structure, so moving
  a node never renames anything.

### §5.3 Typed values

| Concept | Type | Replaces |
|---|---|---|
| Color | `ColorRef = Rgba([u8;4]) \| Var(VarId) \| Palette(PaletteField)` | hex / `var(--x)` strings resolved per build |
| Font | `FontId` (interned at load) | family-name strings scanned per run per build |
| Font size | `FontSize = Pt(f32) \| Rel(f32)` — `Rel` is a fraction of the node's shorter side (frames only) | `size_pt` collapsed to a max |
| Shape | `Shape::{Rectangle { radius: Percent }, Ellipse}` | `shape: String` parsed twice per node per build |
| Anchors | `Anchor::{Auto, Top, Right, Bottom, Left}` | `anchor_from` / `anchor_to` strings matched per frame |
| Link kind | `LinkKind::{Line, Portal}` | `display_mode` string compared per pass |
| Weight, style | `Weight`, `italic: bool` | dropped run flags |
| Link target | `LinkTarget::{Url(String), Verb(VerbId)}` | `hyperlink` string with no renderer |

`Palette(field)` resolves against the node's palette binding at the
node's **depth**, computed from the tree. A theme-variant switch is
one mutation of `Theme.active`; the dependency index (§12) knows
which elements read which variables.

### §5.4 Style inheritance

Every style field on every element is an `Option`. Resolution is
**per field**, never per struct:

```
element field → owning node's field (for spans: section → node) → node's palette binding
             → Defaults → built-in default
```

A node that sets only a frame's `fit` still inherits everything else
from `Defaults`; editing one field never copies a default into the
element. Resolved values are computed by the projector, cached in
projection state, and invalidated through the dependency index (§12)
— never recomputed on every pass.

### §5.5 Size (D5)

`Node.min_size: Option<Vec2>` is the size the user set by resizing.
The drawn size is `max(min_size, fitted content size)`, where the
fitted size comes from measuring sections (§6.4). Fitted sizes live
in projection state, never in the document, and are recomputed only
when a section's spans, fonts or sizes change. `Section` has the same
`min_size` semantics within its node.

### §5.6 Element fields (summary)

The authoritative field list is `format/schema.md` (rewritten in
V1); this is the shape.

- **Node:** `id`, `position`, `min_size?`, `shape`, `fill?`,
  `fill_fade?` (edge fade width, D12), `palette?`, `zoom?` (min/max),
  `folded`, `on { click?, hover? }` (verb refs, §11), and text
  defaults (`font?`, `size?`, `color?`) inherited by its spans.
- **Section:** `offset`, `min_size?`, `align?`, text defaults.
- **Frame:** `visible`, `start`, `direction`, `fit`, `gap` (between
  rings), `inset` (distance from the outline), text defaults.
- **Ring:** `size?` (a `Rel` font size), `fit?` (overrides the
  frame's), text defaults.
- **Span:** `text`, `font?`, `size?`, `color?`, `weight?`, `italic?`,
  `link?`.
- **Connector / Link (line):** `anchors`, `control_points` (all of
  them drawn), `fit`, `spacing?`, `size?`, `color?`, `visible`,
  `zoom?`; a Link also has `to: NodeRef` and `kind`.
- **PortalEnd:** `border_t?`, `offset?`, `color?`, `size?`.
- **Label:** `t` (position along the path), `offset`, text defaults.

### §5.7 Definitions the document carries but does not mutate in V1

Edits, presentations and verbs (§10, §11) are definitions: loaded,
validated, saved, and invoked, but not changed by any V1 code path —
there is no in-app authoring for them in V1 (V2 epic). They are held
by `Document` beside the tree, not as tree elements.

---

## §6 TextFlow — all text, one primitive

### §6.1 The concept

> **A TextFlow is a sequence of spans plus a layout.**

| Where | Layout |
|---|---|
| Section, Label | **Box** — wraps within a width, grows the box (§5.5) |
| Frame ring | **Closed path** — the node's outline, offset outward per ring |
| Connector, line Link | **Open path** — the connection curve |
| PortalEnd marker | **Point** — a single placed run |

One content model (spans), one shaping cache, one placement
primitive, one renderer path (§13). A frame can hold a poem; a
connection can be a line of verse from parent to child. Box-drawing
glyphs (`─`, `━`, `═`) are ordinary content, not presets.

### §6.2 Upright glyphs, positioned along a path (D9)

Glyphs are never rotated. A path flow is shaped once as a single
line (cosmic-text gives each glyph's advance), then placed:

1. Walk the path by arc length from the flow's start point in its
   direction.
2. Each glyph occupies a step whose length depends on the direction
   of travel θ at that point:
   `step = |cos θ| · advance + |sin θ| · line_height`.
   Along a horizontal stretch glyphs sit side by side; along a
   vertical stretch they stack one line apart; on a curve the two
   blend — the way ASCII art builds a curve.
3. The glyph's center sits on the path, offset along the outward
   normal by half its extent, so text sits on the outline rather
   than across it. Ring *k* is placed on the outline offset outward
   by `inset + Σ(thickness of rings < k) + k · gap`.

### §6.3 Outlines and corner radius (D11)

The outline is derived from the node's drawn rectangle and its shape:

- `Rectangle { radius }`: a rounded rectangle whose corner radius is
  `radius% × (min(width, height) / 2)`. 0% gives sharp corners; 100%
  gives a circle on a square node and a pill (straight sides,
  semicircular ends) on a wide one. Values are capped at 100.
- `Ellipse`: the inscribed ellipse.

The node's background fill (§13.4) uses the same outline, so the fill
and the glyph frame always agree.

### §6.4 Fit modes (D10)

When a flow's text is shorter or longer than its path:

| Mode | Behavior | Frames | Connectors |
|---|---|---|---|
| `once { filler? }` | Text placed once from the start; the rest of the path gets the filler span repeated, or nothing | ✓ | ✓ |
| `repeat` | Text repeated until the path is full; the last repetition is cut at a grapheme boundary | ✓ | ✓ (default — a `─` pattern) |
| `spread` | Text placed once with extra spacing distributed so it ends exactly at the path's end | ✓ | ✓ |
| `fit` | The ring's font is scaled so the text fills the path exactly once | ✓ (default) | ✓ |

**There is no "grow the node" mode for frames.** Ring font size is
relative to the node's size (D10), so the text's length and the
outline's perimeter scale together; growing the node never makes the
text fit. `fit` is the mode that answers "the poem, exactly once,
all the way around." (Box flows — sections and labels — grow their
box, D5.)

`start` (a point on the outline: a corner or a side plus a fraction)
and `direction` (clockwise / counter-clockwise) are per frame;
connectors start at their source end.

### §6.5 What is shaped, and when

- A flow is shaped when its spans, fonts or sizes change — and only
  then. Moving a node re-places its flows without reshaping; resizing
  re-places box flows and re-shapes `Rel`-sized rings (their font
  size changed), for that node only.
- Box-flow measurement (the fitted size, §5.5) comes from the same
  shaping and is cached with it.
- Hit testing maps a placed glyph back to its span through the
  projection map, which is how a click on a linked span finds its
  link (§11).

---

## §7 The file format

### §7.1 Shape

Strict, nested, typed JSON (D4). The structure of the file *is* the
structure of the tree: a node's children are nested inside it.

```json
{
  "format": 1,
  "name": "Testament",
  "theme": {
    "active": "dark",
    "variants": {
      "dark":  { "bg": "#101014", "fg": "#e0e0e0", "accent": "#d4a24c" },
      "light": { "bg": "#f6f3ec", "fg": "#202024", "accent": "#8a5a00" }
    }
  },
  "palettes": {
    "rainbow": [ { "fill": "#2a1b3d", "frame": "#a239ca", "text": "#e0e0e0", "title": "#ffffff" } ]
  },
  "defaults": {
    "ring":      { "size": 0.06, "color": "var(accent)" },
    "connector": { "spans": [ { "text": "─" } ], "fit": "repeat", "size": 12 }
  },
  "edits":         [ ],
  "presentations": [ ],
  "verbs":         [ ],
  "nodes": [
    {
      "id": 1,
      "position": [0, 0],
      "shape": { "rectangle": { "radius": 25 } },
      "palette": "rainbow",
      "sections": [
        { "spans": [ { "text": "In the beginning", "size": 24, "weight": "bold" } ] }
      ],
      "frame": {
        "fit": "fit", "start": "top-left", "direction": "clockwise",
        "rings": [ { "spans": [ { "text": "And the earth was without form, and void; " } ] } ]
      },
      "on": { "hover": "glow", "click": "reveal-chapter-2" },
      "links": [ { "to": 9, "kind": "portal" } ],
      "children": [
        { "id": 2, "position": [320, 40], "connector": { "spans": [ { "text": "·" } ] },
          "sections": [ { "spans": [ { "text": "the heaven and the earth" } ] } ] }
      ]
    }
  ]
}
```

### §7.2 Rules

- **Strict.** Every type denies unknown fields; an unknown key, a
  wrong type, a dangling `to` / verb reference or a duplicate `id` is
  a load error naming the JSON path. There is no preserved-unknown-keys
  mechanism and no skipped-construct report — a map is either valid
  or it does not load.
- **Typed on read.** `"var(accent)"`, palette names, font families,
  shapes, anchors, fit modes, weights and verb references are
  resolved to their typed forms (§5.3) during parsing; the document
  never holds the string forms.
- **Nothing derived is written.** No fitted sizes, no depth, no
  parent pointers, no resolved colors, no copied defaults. Optional
  fields equal to their inherited value are omitted on save.
- **Deterministic output.** Keys in schema order, nodes in sibling
  order, numbers in a canonical form, so an unchanged map saves
  byte-identically and a one-field edit is a one-line diff.
- **One pass.** Load is parse → build the tree with links and verb
  references resolved through a load-time `id → NodeId` table →
  validate. Save is one walk.
- **Platform limits.** `MAX_MAP_BYTES` (wasm32 only, platform physics)
  stays as it is. `MAX_UNKNOWN_KEYS` is deleted with the mechanism it
  bounded.
- **Extension.** Proposed: `.mandala.json`, so a file in the old
  format is never mistaken for one in the new (open question in the
  issues draft).

### §7.3 What happens to existing maps and tools

- The repository's maps (`maps/*.mindmap.json`, test fixtures) are
  converted **once**, in the cut-over sub-issue, by a throwaway
  script that is not committed. The converted maps are checked by
  screenshot against the old app's rendering.
- `maps/testament.mind` (the miMind original) and every legacy path
  are deleted: the baumhard loader's legacy shapes, `maptool convert`
  (`--legacy`, `--portals`, `--sections`), `format/migration.md`,
  `crates/maptool/src/migration_advice.rs`, the legacy custom-mutation
  shape (`mutations` + `target_scope`), and `unknown_keys.rs`.
- `maptool` is rebuilt as a client of `Document` (§14): `show`,
  `grep`, `export` and `verify` read through it; `apply` runs its
  command and writes through `Document::apply`.

---

## §8 One writer

### §8.1 The API

```rust
impl Document {
    pub fn apply(&mut self, req: EditRequest, source: ChangeSource) -> Result<Revision, ApplyError>;
    pub fn undo(&mut self) -> Option<Revision>;
    pub fn redo(&mut self) -> Option<Revision>;
    // §9
    pub fn begin(&mut self, source: ChangeSource) -> Transaction;
    pub fn stage(&mut self, tx: &Transaction, req: EditRequest) -> Result<(), ApplyError>;
    pub fn commit(&mut self, tx: Transaction) -> Option<Revision>;
    pub fn cancel(&mut self, tx: Transaction);
}

pub struct EditRequest { pub mutator: DocMutatorTree, pub anchors: Anchors, pub label: Label }
```

`Document::apply` is the **only** way the document changes after
load. Actions, console verbs, IPC, drags, the text editor, the color
picker, frame previews, edit handlers, animated edits and verbs all
build an `EditRequest` and call it (or the transaction API, §9).

### §8.2 Guarantees

- **All or nothing.** If any mutation in a request fails validation,
  the request's journal is reverted and an `ApplyError` names the
  element and field. The document is never left half-applied.
- **Validation lives in the mutations.** Numeric bounds (canvas
  coordinates, font sizes, radius 0–100, `Rel` sizes), structural
  rules (no node moved under its own descendant; removing a node also
  removes links that target it, journaled) and reference integrity
  are checked where the value is written. A setter can no longer
  forget a bound the loader enforces (issue #208's class).
- **One history.** Each successful `apply` or `commit` pushes one
  history entry: its journal, its `ChangeSource` and its label. Undo
  reverts, redo replays (§4.5); a new apply clears the redo branch.
  A multi-selection edit is one request with several anchors — one
  entry. The history is bounded by a configured entry count; dropping
  the oldest entry frees the subtrees its journal detached.
- **Revision and dirty.** Every apply, commit, undo and redo advances
  the revision. `dirty` is `revision != saved_revision`, so undoing
  back to the saved state is clean again.
- **Sources.** `ChangeSource::{Action, Console, Ipc, Input, Edit,
  Verb, Undo, Redo}`. Every apply declares one; there is no fallback.

### §8.3 What it replaces

The 13 `UndoAction` variants, `undo.rs`, `undo_action.rs`, the
undo-envelope helpers (`mutate_node_with_*_undo`), `mutate_edge`,
`closure_helpers` fork-on-edit, per-setter undo pushes in app code
(`moving_node.rs`, `dispatch/native.rs`), `with_source`, and
`mark_dirty` as a call sites must remember.

### §8.4 The gate

A source-scan test (the pattern of
`test_no_production_code_sets_dirty_directly`) fails if any code
outside `lib/baumhard/src/document/` obtains a mutable reference to a
document element or the document tree. The loader constructs; apply
writes; nothing else does.

---

## §9 Transactions — live interaction

A transaction is a staged edit that is re-staged every frame and
becomes one history entry when it ends.

```
begin ─▶ stage(req₁) ─▶ stage(req₂) ─▶ … ─▶ commit   (one undo entry)
                                       └──▶ cancel   (document as before begin)
```

`stage` reverts the previous staged journal and applies the new
request — so a drag stages `position += total_delta` each frame,
never an accumulation of per-frame deltas, and the document is exact
at every frame. Staged changes flow through projection like any
other change (§12): the drawn result of a drag *is* the document,
not a preview painted over it.

| Interaction | Staged request |
|---|---|
| Node / subtree move | `position += Δ` at each moved node |
| Section move, node / section resize | `offset += Δ`, `min_size = s` |
| Connector / link handle drag | control point, anchor |
| Label / portal-label drag | `t`, `offset`, `border_t` |
| Text editing (sections, labels, single-line editors) | span text edits, splits, merges for the edit session — one undo entry per session, as today |
| Color picker hover, frame/border preview | the previewed field |
| Animated edit (§10) | the edit's journal interpolated at `t` |

Rules:

- At most one transaction is open at a time.
- `undo` while a transaction is open cancels it first.
- Saving while a transaction is open writes the committed state:
  revert the staged journal, serialize, replay — cost proportional to
  what is staged.
- This deletes every bespoke preview mechanism: drag offsets applied
  to the tree (`apply_drag_delta*`, `apply_*_resize_to_tree`), the
  text editor's direct `DeltaGlyphArea` writes, `label_edit_preview`,
  `portal_text_edit_preview`, `color_picker_preview`,
  `border_preview`, and the per-frame model writes of the edge-handle
  and label drags.

---

## §10 Edits and Presentations (D6)

A map has two kinds of behavior, and the design keeps them apart at
every level: different Rust types, different lists in the file,
different console commands, different rules.

| | **Edit** | **Presentation** |
|---|---|---|
| Purpose | Change the map | Perform the map |
| Saved | Yes | Never |
| Undo | One history entry | Never in history |
| Marks the map changed | Yes | No |
| Document revision | Advances | Unchanged (has its own presentation epoch for IPC events) |
| Animation | Optional — animates into place, then commits | The usual case |
| Structural commands | Allowed | Not allowed (load error) |
| Typical triggers | Key binding, console, command; a verb step | Hover, click, links, verbs |
| Examples | flower layout, tree cascade, grow font | hover lift, click pulse, shimmer, reveal |

The names are the API: `EditDef` / `PresentationDef` in code,
`"edits"` / `"presentations"` in the file, `edit …` / `present …`
on the console. "Persistent", "Toggle" and `target_scope` are gone.

### §10.1 Edits

```json
{ "id": "grow-font", "name": "Grow font", "mutator": { … }, "animate": { "ms": 200, "easing": "ease-out" } }
{ "id": "flower-layout", "name": "Flower arrangement", "handler": "flower", "animate": { "ms": 400 } }
```

- **Declarative edits** carry a document mutator AST (§4.3), run by
  the walker from their anchors. Where an edit reaches is its
  mutator's shape alone — standard shapes (`self`, `children`,
  `descendants`, `subtree`, `parent`, `siblings`) are provided as
  named AST shapes.
- **Handler edits** name a Rust handler for algorithms the AST cannot
  express (radial and cascade layouts). A handler is
  `fn(&Document, &Anchors) -> DocMutatorTree`: it **reads** the
  document and **returns a mutator**. It never writes. That is what
  gives every handler undo, animation and IPC events for free.
- **Animated edits** run as a transaction (§9): apply once to learn
  the journal's before/after values, then stage the interpolation at
  each frame (§4.6), then commit — one history entry, identical to
  the instant edit. `undo` during the animation cancels it.
- **Sources** keep the existing precedence, App < User < Map:
  `assets/edits.json` (bundled), the user's `edits.json`, and the
  map's `"edits"`. The node-inline tier is removed: definitions live
  at map level, and a node refers to them through verbs.

### §10.2 Presentations

```json
{ "id": "glow",    "play": "while", "mutator": { … }, "animate": { "ms": 150 }, "return": { "ms": 300 } }
{ "id": "pulse",   "play": "once",  "mutator": { … }, "animate": { "ms": 120 } }
{ "id": "shimmer", "play": "loop",  "layer": "glyph", "mutator": { … }, "animate": { "ms": 900 } }
```

**Lifecycle (`play`):**

| `play` | In | Holds | Out |
|---|---|---|---|
| `while` | animates in on start | while its trigger holds (hover) or until stopped | animates back out (`return`, defaulting to `animate`) |
| `once` | animates in | — | animates straight back out |
| `loop` | in, out, in, … | until stopped | finishes the current cycle back to rest |
| `hold` | animates in | until stopped or the map closes | animates back out when stopped |

**Vocabulary (`layer`):**

- `document` (default): the mutator is written in document terms —
  position, size, colors, span style, frame fields, `folded`. It goes
  through projection like an edit, so lifting a node on hover moves
  its connections with it.
- `glyph`: the mutator is written in glyph terms and applied to the
  target's projected glyph runs — per-glyph offsets, colors and scale
  within a TextFlow run (§13.2). These are effects with no meaning
  in the data: wave, shimmer, jitter.

**The presentation layer.** Active presentation instances form a
layer on top of the document:

```
committed document ⊕ open transaction ⊕ presentation layer   →  projection
```

- Each instance holds the journal of its current application.
- Any change underneath — `apply`, `stage`, `undo`, `redo` — reverts
  the layer (newest first), makes the change, and re-applies each
  instance at its current progress. The history journal therefore
  contains only the edit, and a presentation always sits on top of
  the current map. Cost is proportional to the active instances'
  reach.
- Save writes the document with the layer reverted. A presentation
  can never reach the file.
- Starting a presentation that is already active on the same target
  restarts it rather than stacking it.
- `glyph`-layer instances re-apply to any glyph run the projector
  rewrites (the glyph journal names them).

---

## §11 Verbs and triggers (D7)

### §11.1 Verbs

A **verb** is a named sequence of steps the map exposes as its public
interface — what makes a map interactive.

```json
{ "id": "reveal-chapter-2", "name": "Reveal chapter 2",
  "steps": [
    { "present": "fade-in", "on": { "nodes": [12, 13, 14] } },
    { "wait": 300 },
    { "present": "glow", "on": { "channel": 2, "within": "self" } },
    { "edit": "flower-layout", "on": "self" },
    { "stop": "glow", "on": "self" }
  ] }
```

| Step | Effect |
|---|---|
| `present` | start a presentation on the targets |
| `stop` | stop a presentation on the targets (it plays its way out) |
| `edit` | apply an edit to the targets — an undoable change to the map, recorded with `ChangeSource::Verb` |
| `wait` | wait on the app clock (virtual-clock aware, so headless and IPC runs are deterministic) |
| `verb` | run another verb (recursion is bounded and checked at load) |

**Targets** (`on`): `"self"` (the node the trigger fired on),
`"selection"`, `{ "nodes": [ids] }`, `{ "channel": n, "within": "self" | "map" }`,
`"children"`, `"descendants"`. Channels are how a verb addresses a
group of nodes ("animate the channel-2 children out") — the purpose
`format/channels.md` gave them from the start.

**Edits are visible in a verb.** A verb that contains `edit` steps is
listed as changing the map (`verb list`, `describe`), so "Flower
arrangement" on a button is visibly a save-able change, and "Reveal"
is visibly a performance.

Verbs take no parameters in V1; the step format leaves room for them
without a breaking change (V2 epic).

### §11.2 Triggers in V1

| Trigger | Invokes |
|---|---|
| Node click | `node.on.click` |
| Node hover | `node.on.hover` — **held** for the hover's duration: leaving the node stops every `while` presentation the verb started |
| Span link click | a span's `link: { "verb": id }` (or opens `{ "url": … }`) |
| Console | `verb <id> [node-id]` |
| IPC | `act.verb` (and the `mandalactl verb` command, shipped in the same PR per CLAUDE.md) |
| User and app macros | a `verb` step |

Buttons as a node feature, keyboard triggers, map-load and timer
triggers, and verb parameters are V2 (the trigger mechanics are a
separate design discussion).

### §11.3 Trust boundary

A map's verbs can only start and stop presentations, apply the map's
own edits (including built-in edit handlers), wait, and call verbs.
They cannot run console lines, Actions or anything that touches files
or the process. **Map verbs replace the map-level and node-level macro
tiers** (`MindMap.macros`, `MindNode.inline_macros`), which today can
run arbitrary console commands from a map someone sent you. App- and
user-level macros remain the user's own automation and can invoke
verbs.

"Verb" also names console commands and `mandalactl` commands. In
prose where it is ambiguous, this design says **map verb**.

---

## §12 Projection — forward only, proportional to change

The projector lives in `lib/baumhard/src/projection/`. Its input is
the document (with the open transaction and the presentation layer
applied) and journals; its output is glyph mutator trees for the
glyph layers, and the glyph journals they produce.

### §12.1 Glyph layers

| Layer | Content | Replaces |
|---|---|---|
| Fills | one fill element per visible node (shape, radius, color, fade) | `node_background_rects`, the background half of the node tree |
| Sections | one box-flow run per section | the node tree's container + section areas |
| Frames | one path-flow run per ring | `Borders` (one `GlyphArea` per glyph) |
| Connections | one path-flow run per connector / line link, plus cap runs | `Connections` (one `GlyphArea` per glyph) |
| Labels | one box-flow run per label | `ConnectionLabels` |
| Portals | marker and label runs per portal end | `Portals` |
| Chrome | selection, handles, NodeEdit section frames, hover chrome — **view-only**, built from app state, not the document | `EdgeHandles`, `NodeResizeHandles`, `SectionResizeHandles`, `SectionFrames` |

Each layer is a `Tree<GfxElement, GfxMutation>` in the baumhard
`Scene`. A TextFlow becomes **one** glyph element (§13.2), not one
element per glyph.

### §12.2 State and maps

- **Projection map:** document `NodeId` → the glyph elements it
  produced (a node produces a fill, section runs, ring runs, its
  connector run, …). Hit testing uses the reverse direction to go from
  a glyph back to a span or node.
- **Projection state** per document element: resolved style values,
  shaped runs, measured fitted size, outline path, placement. This is
  where every value the old projection recomputed per pass is now
  cached.

### §12.3 The dependency index — captured, not declared

When the projector resolves a value, the resolver **records what it
read**: "ring 7's color read `Theme.var(accent)`", "connector 3's
endpoints read node 2's geometry and node 1's geometry", "node 5's
palette group read its depth". Those records are the dependency
index. A change re-projects the touched elements plus everything
whose recorded reads include them. Nothing is listed by hand, so the
index cannot drift from what projection actually reads.

Typical chains:

- Span text or style → section measurement → node fitted size → node
  geometry → its fill, frame rings (placement; `Rel` rings
  re-shape), section placement, its connector, its children's
  connectors, links and portal ends touching it, labels on those.
- Theme variant switch → every element that read a variable of the
  theme — and nothing else.
- Palette or `Defaults` field → elements whose resolution read it.
- `folded` → the descendants' projection is removed or added.
- Structural insert / remove / move → the subtree is projected,
  unprojected or re-placed; depth-dependent palette reads in a moved
  subtree are re-resolved.

### §12.4 Zoom and pan

Pan changes only the camera transform; no tree is touched. Zoom
re-projects only elements that declared a zoom-dependent property
when they were projected (zoom-visibility windows, screen-space
minimum font sizes); the projector keeps them in a zoom index.

### §12.5 View-only glyph mutators

Selection tint, NodeEdit dimming, hover chrome and `glyph`-layer
presentations are glyph mutators applied over the projected layers.
When the projector rewrites glyph elements, overlays are re-stamped on
exactly those elements (glyph journal ∩ overlay targets) — not on a
fresh tree each frame, as `overlay_tree` does today.

### §12.6 Load, and the oracle

Load is the only full projection. A full projection function also
exists for one purpose: the **oracle test** (§16) — after every
mutation in the test corpus, the incrementally maintained glyph
layers must equal a full projection of the resulting document.

---

## §13 Renderer

### §13.1 Proportional updates

Renderer buffers are keyed by glyph element. The glyph journal from
each projection step names the elements whose runs must be reshaped,
re-placed, recolored or dropped; nothing else is visited. This
replaces `rebuild_buffers_from_tree` (reshape every node buffer) and
the comparison scan in `scene_shape_cache::refresh` (walk every
element and compare it to its cached inputs).

### §13.2 Positioned glyph runs

A TextFlow projects to one `GlyphRun` element: a reference to its
shaped run plus a placement — box layout for box flows, per-glyph
canvas positions for path flows — and optional per-glyph overrides
(offset, color, scale) that `glyph`-layer presentations write.
Shaping happens once per run (§6.5); re-placing a run moves positions
without reshaping.

Drawing glyphs at arbitrary positions from one shaped run is the one
renderer capability V1 needs that glyphon does not offer directly:
glyphon draws whole buffers at a single position. The renderer
sub-issue opens with a spike that chooses between (a) placing glyphs
through the shaped layout glyphon already consumes, (b) a quad
pipeline of our own over glyphon's glyph atlas, and (c) vendoring
glyphon into the workspace with per-glyph placement added. The choice
is recorded in this section before implementation starts. It is the
highest technical risk in V1, which is why it is scheduled early
(§17).

### §13.3 What is no longer done

Per-glyph `GlyphArea`s and their per-glyph cosmic-text buffers for
frames and connections; full node-buffer reshaping on every edit;
per-frame comparison scans; canvas structure and content signatures
(`canvas_dispatch_with_content` and friends); the
`SceneConnectionCache` / `EdgePathCache` pair as separate
invalidation-by-convention caches (their job moves into projection
state, invalidated by the dependency index).

### §13.4 Fills (D12)

Node backgrounds stay a shader fill (the existing SDF path,
`SHAPE_*` constants). V1 adds a rounded-rectangle case whose radius
follows §6.3 — so the fill and the frame share one outline — and a
`fill_fade` width: the fill's alpha falls off smoothly over that
distance at its edge, giving a soft transition into the glyph frame.

---

## §14 Touchpoints outside the core

- **IPC** (`src/application/ipc/`, `format/ipc.md`): nodes are
  addressed by integer `id` everywhere (was the Dewey string);
  `query.document` and `query.undo` reflect the new document and
  history; document events are emitted from journals (what changed,
  with which source), with separate presentation and verb events; a
  new `act.verb`. The protocol version is bumped. Every command change
  ships its `mandalactl` change in the same PR (CLAUDE.md).
- **Console**: `mutation …` becomes `edit list | apply | help |
  inspect`, plus `present start | stop | list` and `verb list | run`.
  Every console command that sets a field builds an `EditRequest`.
- **Keybinds**: `custom_mutation_bindings` becomes `edit_bindings`.
  Keys bound to map verbs are a V2 trigger.
- **Macros**: the map and node-inline tiers are removed (§11.3); App
  and User tiers gain a `verb` step and use `edit` in place of the
  custom-mutation step.
- **maptool**: rebuilt on `Document` — `show`, `grep`, `export`,
  `verify`, `apply`; `convert` is deleted.
- **CLAUDE.md "Dual-target status"**: the `MAX_UNKNOWN_KEYS` entry is
  deleted; the entries that describe drags, previews, the animation
  pause and the per-frame drain are rewritten for transactions and
  the presentation layer. No carve-out is added.

---

## §15 What is deleted

Grouped; each is removed by the sub-issue that replaces it.

- **Model:** `lib/baumhard/src/mindmap/model/` as it stands
  (`MindMap`, `MindNode`, `MindSection`, `MindEdge`, `NodeStyle`,
  `NodeLayout`, `ColorSchema`, `TextRun` and friends), Dewey-id
  machinery (`id_sort_key`, `fresh_child_id`, `cascade_rename`,
  `next_free_root_segment`), `text_run_ops.rs`, `unknown_keys.rs`,
  `SkippedConstruct(s)`, legacy loader shapes.
- **Custom mutations:** `document/custom/` (`sync.rs` included), the
  flat-apply path (`flat_mutations`, `mutator_reach`, `covers_reach`,
  `TargetScope`, `MutationBehavior`), `apply_position_mutations_to_node`,
  `active_toggles` / `reapply_active_toggles`, the
  `DynamicMutationHandler` signature that writes the model.
- **Undo and writes:** `UndoAction` and `undo.rs`, the undo-envelope
  helpers, `mutate_edge`, fork-on-edit helpers, every direct model
  setter in `document/nodes/`, `document/edges/`, `topology.rs`.
- **Previews:** `apply_drag_delta*`, `apply_*_resize_to_tree`, the
  text editor's direct glyph writes, the `*_preview` slots on
  `MindMapDocument`.
- **Projection and rendering:** `build_mindmap_tree` and the per-role
  full builders as edit-time paths, canvas signatures, `RebuildTier`
  and the `rebuild_*` tiers, `scene_shape_cache`'s scan,
  per-glyph border and connection `GlyphArea`s, the border preset
  table as a special case (presets become content).
- **Behavior:** map and node-inline macro tiers, `Trigger::{OnKey,
  OnLink}` as separate triggers (links are span-level), `RotateWhile`
  (to V2).
- **Tools and docs:** `maptool convert`, `migration_advice.rs`,
  `format/migration.md`, `maps/testament.mind`.
- **Dead fields** (D13).

---

## §16 Testing strategy

All of it runs under `./test.sh`; none of it needs a GPU except the
renderer tests, which follow TEST_CONVENTIONS §T8 (lavapipe, skip with
a note locally, required in CI). No mocks, snapshots or pixel
assertions (TEST_CONVENTIONS). Every passing assertion owes its
negative control (§T13).

| Property | Test |
|---|---|
| The engine is one engine | Walker tests run against both instantiations — the glyph tree and the document |
| Sorted-children invariant | A checker run after every structural command in the engine tests |
| Journals are exact | For generated mutator trees: `apply` then `revert` restores the tree exactly; `revert` then `replay` reproduces the applied tree exactly; structural commands included |
| All-or-nothing apply | A request with one failing mutation leaves the document unchanged |
| One writer | Source-scan gate (§8.4) |
| The format round-trips | Every map in the repository loads and saves byte-identically; every strict-loader error names its JSON path |
| Projection is exact | **Oracle:** after every mutation in the corpus (every `DocMutation` kind and structural command, on the testament map and generated maps), the incremental glyph layers equal a full projection of the resulting document |
| Dependencies are complete | Per dependency kind: change the source, assert the dependent was re-projected; negative control: change an unrelated source, assert it was not |
| TextFlow placement | Geometric assertions: glyph centers on the (offset) outline within tolerance, step lengths per §6.2, fit modes' end conditions, ring offsets |
| Presentations never persist | Edit applied while presentations are active: the history journal holds only the edit; save output equals the save without the presentation |
| Verbs are deterministic | Headless app, virtual clock: step timing and presentation states at stepped instants |
| Verbs are sandboxed | Load rejects any step outside §11.1's set |
| Dual target | `./test.sh` type-checks wasm32 as today; the wasm font-lock probe covers the new shaping paths |

Bench entries accompany every new engine primitive (journaled walk,
structural command, sorted merge, placement) and are type-checked by
`cargo check --workspace --benches`. Agents do not run them
(CLAUDE.md §6).

---

## §17 Delivery

### §17.1 The branch

`epic/unified-mutations` is cut from `main`. Every V1 sub-issue's PR
targets it and leaves it green under `./test.sh --lint`. `main` takes
only fixes during the epic; the branch merges `main` at each phase
boundary. When V1 is complete, the branch merges into `main` once.

Between the cut-over (phase C) and the behavior sub-issues (phase D),
custom edits, animations and triggers may be absent on the branch.
Tests for absent features are deleted with the code they test and
rewritten with the replacement — never skipped. `main` never sees the
gap.

### §17.2 Phases

```
A  Engine (baumhard only; the app is unchanged)
   A1 generic engine ── A2 routing, sorted children, anchored walks
                      ├─ A3 structural commands
                      └─ A4 journals + interpolation
B  Document and flows (baumhard only; built and tested, not yet wired)
   B1 document types, typed values, inheritance ── B2 format: loader, saver
   B3 document mutation vocabulary ── B4 one writer, history, transactions
   B5 TextFlow layout and path placement
   B6 projection, dependency index, oracle
C  Cut-over (the app moves to the document)
   C1 renderer: positioned glyph runs (spike first), fills, journal-driven buffers
   C2 app cut-over: reads, writers, transactions, hit testing, IPC addressing,
      map conversion, deletion of the old model
D  Behavior
   D1 edits on the walker; handlers return mutators; animated edits
   D2 presentation layer
   D3 verbs, triggers, links
E  Tools, docs, merge
   E1 maptool on Document
   E2 docs rewrite (CONCEPTS, conventions, format/, CLAUDE.md)
   E3 switch-over merge into main
```

A and B can proceed in parallel lanes after A1/B1; C1's spike can
start as soon as B5 defines a placed run. C2 is the one large PR;
it is done by one owner, in ordered commits, because the app cannot
hold two models at once.

### §17.3 Definition of done for V1

- Every item in §15 is gone and every decision in §0 is implemented.
- The four flows of §3 behave as described, demonstrated with a
  recording (`mandala-record`) on the testament map: an edit, a drag,
  a hover presentation, a verb fired by click.
- `./test.sh --lint` is green on native and wasm32, with GPU tests
  required in CI.
- The docs describe what the code does (E2), and nothing in them
  describes a path that does not exist.

---

## §18 Out of V1 (documented in the V2 epic)

Buttons as a node feature; trigger mechanisms beyond node click,
node hover and span links (keys bound to map verbs, map load, timers,
sequences); verb parameters; in-app authoring of edits, presentations
and verbs; node shadow; the shapes the format already names but cannot
draw (hexagon, diamond, parallelogram), with frames following their
outlines; `RotateWhile`; a create-child-node action (today only orphan
creation exists); fold / unfold actions (fold has no production
writer); section merge and reorder; the `Touch` platform context for
triggers. Each is drafted as a V2 sub-issue so it is ready to pick up.

---

## §19 Open questions

1. The file extension: `.mandala.json` (proposed) or keep
   `.mindmap.json`.
2. The default history bound (entries).
3. Whether `edit_bindings` should also accept map verbs in V1, or
   wait for the V2 trigger design (proposed: wait).
