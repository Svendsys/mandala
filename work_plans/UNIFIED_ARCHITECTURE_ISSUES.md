# Unified Architecture — issue drafts (not yet filed)

Drafts for review. Nothing here exists on GitHub yet. Once approved,
each block becomes one issue; the two epics link their sub-issues as
GitHub sub-issues. Every V1 sub-issue cites
[`UNIFIED_ARCHITECTURE.md`](./UNIFIED_ARCHITECTURE.md) as its design
reference and targets the `epic/unified-mutations` branch.

---

# EPIC V1 — Unified architecture: one mutation engine for data and view

**Goal.** Every state change in Mandala — to the map's data or to what
is drawn — is a baumhard mutator tree applied by one walker to one
kind of tree. The document becomes a baumhard tree; the view is a
forward-only, change-proportional projection of it; Edits change the
map, Presentations perform it, and map verbs expose it.

**Design:** `work_plans/UNIFIED_ARCHITECTURE.md` (decisions D1–D16 in
§0). **Branch:** `epic/unified-mutations`; one merge into `main` at
the end (§17). **Compatibility:** none — file format, mutation format
and internal APIs are redesigned.

**Phases and sub-issues** (§17.2):

- **A — Engine:** A1, A2, A3, A4
- **B — Document and flows:** B1, B2, B3, B4, B5, B6
- **C — Cut-over:** C1, C2
- **D — Behavior:** D1, D2, D3
- **E — Tools, docs, merge:** E1, E2, E3

**Done when** §17.3 holds.

---

## A1 — Generic engine: tree, mutator, walker, predicate, AST

**Phase A · depends on: —**

Make baumhard's engine generic over the element type and the mutation
payload (§4.1–§4.3). `GfxElement` / `GfxMutation` become the first
instantiation with no behavior change.

- `TreeElement` trait; `Mutator<E, M>` replaces `GfxMutator`;
  `Mutation<E>` trait; walker, `MutatorTree::apply_*`, `Predicate<E>`,
  `MutatorNode` AST, `Repeat`, `BuildContext` and `build` generic.
- `SpatialDescend` available only for `E: Spatial`.
- `RotateWhile` removed (documented in V2).
- A minimal second element type in tests proves the engine is not
  glyph-bound.

**Acceptance:** every existing walker / mutator / builder test passes
against the glyph instantiation; the same walker tests pass against
the test element type; bench entries compile
(`cargo check --workspace --benches`).

## A2 — Routing keys, sorted children, anchored walks

**Phase A · depends on: A1**

- `BranchKey { kind, channel }` routing (§4.1).
- The sorted-children invariant, with the walker's merge rewritten to
  walk sibling lists in step — no per-node allocation or sort.
- `apply_at(tree, anchors, journal)` with `Parent` / `Siblings`
  relative anchors and the `IsOrigin` predicate field (§4.4).

**Acceptance:** an invariant checker in the engine tests; walker tests
for mixed-kind siblings proving a mutator for one kind never lands on
another; anchored walks reach exactly the anchored subtrees (with a
negative control on a sibling subtree).

## A3 — Structural commands

**Phase A · depends on: A2**

`Mutator::Structure` with `Insert`, `Remove`, `Move`, `Split`,
`Merge` (§4.4), applied in a second phase after the walk; removal
detaches and the journal owns the detached subtree.

**Acceptance:** structural tests on both instantiations; the
sorted-children invariant holds after every command; stale-`NodeId`
detection through indextree stamps is exercised.

## A4 — Journals and interpolation

**Phase A · depends on: A2 (A3 for structural entries)**

`Journal<E>` with field, insert, remove and move entries, exact
`revert` / `replay`, `touched()`; `Interpolate` for scalars, vectors,
sizes and RGBA, per-element declaration of interpolating fields and
`snap` (§4.5–§4.6).

**Acceptance:** generated-mutator property tests (§16): apply→revert
is identity; revert→replay reproduces the applied tree; one entry per
element per journal regardless of touch count.

---

## B1 — Document types, typed values, inheritance

**Phase B · depends on: A1**

`DocElement` kinds and fields (§5.1, §5.6); `Document` in
`lib/baumhard/src/document/`; identity (§5.2); typed values —
`ColorRef`, `FontId`, `FontSize`, `Shape`, `Anchor`, `LinkKind`,
`Weight`, `LinkTarget` (§5.3); per-field inheritance resolution
(§5.4); size semantics (§5.5). Not wired to the app.

**Acceptance:** resolution tests for every inheritance tier with
negative controls; depth-derived palette resolution; no `String`
field on a document element other than span text, names and URLs.

## B2 — Format: loader, saver, schema

**Phase B · depends on: B1**

The strict nested JSON format (§7): `format/schema.md` rewritten as
its reference; loader (parse → build → resolve → validate, errors with
JSON paths) and deterministic saver.

**Acceptance:** byte-identical round-trip on a fixture corpus covering
every element kind and field; one rejection test per strictness rule
(unknown key, wrong type, dangling `to`, dangling verb ref, duplicate
`id`).

## B3 — Document mutation vocabulary

**Phase B · depends on: A3, A4, B1**

`DocMutation`: field deltas and commands for every element kind
(nodes, sections, frames, rings, spans, connectors, links, portal
ends, labels, theme, palettes, defaults), structural commands for
nodes, sections, rings, links and spans (span split / merge with
equal-style normalization), validation and clamps inside `apply`
(§8.2), interpolation declarations.

**Acceptance:** per-mutation tests; one validation test per bound;
span normalization tests (grapheme-aware, baumhard CONVENTIONS §B3).

## B4 — One writer: apply, history, transactions

**Phase B · depends on: B3**

`Document::apply` / `undo` / `redo` / `begin` / `stage` / `commit` /
`cancel` (§8–§9); revision and dirty; `ChangeSource`; bounded
history; the source-scan gate (§8.4); save with an open transaction.

**Acceptance:** all-or-nothing test; undo/redo exactness across
structural edits; multi-anchor request = one history entry; staging
N frames then commit = one entry equal to a single apply; dirty
clears when undo returns to the saved revision; the gate fails on a
planted direct write (negative control).

## B5 — TextFlow: layout and path placement

**Phase B · depends on: B1**

Box, closed-path, open-path and point layouts (§6); outlines from
shape and radius (§6.3); upright glyph placement by arc length
(§6.2); rings; fit modes `once` / `repeat` / `spread` / `fit` (§6.4);
start and direction; shaping cache rules (§6.5).

**Acceptance:** geometric tests per §16; `fit` lands the last glyph at
the path's end; a move re-places without reshaping; a resize reshapes
only `Rel` rings.

## B6 — Projection: glyph layers, dependency index, oracle

**Phase B · depends on: B4, B5**

The projector (§12): glyph layers, projection map and state,
captured dependencies, zoom index, overlay re-stamping, and the
full-projection oracle.

**Acceptance:** the oracle (§16) over every mutation kind on the
testament map (converted fixture) and generated maps; per-dependency
tests with negative controls; pan touches no tree; zoom touches only
indexed elements.

---

## C1 — Renderer: positioned glyph runs, fills, journal-driven buffers

**Phase C · depends on: B5 (spike), B6 (integration)**

Opens with the spike of §13.2 — (a) placement through glyphon's
layout, (b) own quad pipeline over glyphon's atlas, (c) vendoring
glyphon — and records the choice in the design doc before building.
Then: `GlyphRun` rendering with per-glyph overrides; fills with the
rounded-rectangle SDF case and `fill_fade` (§13.4); buffers keyed by
glyph element and driven by the glyph journal (§13.1).

**Acceptance:** GPU tests (TEST_CONVENTIONS §T8) for runs, overrides
and fills; journal-driven updates touch only journaled elements
(negative control); wasm32 builds.

## C2 — App cut-over to the document

**Phase C · depends on: B6, C1**

The one large PR (§17.2). The app holds a `Document`; every read
goes through it; every write becomes an `EditRequest` or a
transaction (§9); hit testing goes through the projection map; IPC and
console address nodes by integer `id`; the repository's maps are
converted once (throwaway script, screenshot-checked); and the old
model, undo machinery, previews, rebuild tiers and per-glyph roles of
§15 are deleted.

**Acceptance:** `./test.sh --lint` green; the edit and drag flows of
§3 recorded on the testament map; no reference to the deleted model
remains.

---

## D1 — Edits on the walker

**Phase D · depends on: C2**

`EditDef` (§10.1): declarative edits run by the walker; standard reach
shapes; handler edits that read the document and return mutators
(`flower`, `cascade`); animated edits as transactions; App / User / Map
sources; `edit_bindings`; console `edit …`. Deletes `document/custom/`
and the flat path.

**Acceptance:** each bundled edit applied instant and animated
produces identical history entries; undo mid-animation cancels; a
handler that tried to write would not compile (the signature takes
`&Document`).

## D2 — Presentation layer

**Phase D · depends on: C2, A4**

`PresentationDef` (§10.2): `play` lifecycles, `document` and `glyph`
layers, the layer's revert / re-apply around every change, save
exclusion, restart semantics, presentation epoch for IPC; console
`present …`.

**Acceptance:** §16's "presentations never persist" tests; lifecycle
tests on the virtual clock; an edit under an active presentation keeps
the presentation on top.

## D3 — Verbs, triggers and links

**Phase D · depends on: D1, D2**

`VerbDef` and steps (§11.1), targets (nodes, channels, self,
selection, children, descendants), node click and held hover
triggers, span links to verbs and URLs, console `verb …`, IPC
`act.verb` with its `mandalactl` command, macro `verb` step, removal
of the map and node-inline macro tiers, load-time sandbox and
recursion checks.

**Acceptance:** verb runs deterministic under the virtual clock; hover
leave stops `while` presentations; a verb with an `edit` step creates
one history entry per edit with `ChangeSource::Verb`; sandbox
rejection tests.

---

## E1 — maptool on the document

**Phase E · depends on: C2**

`show`, `grep`, `export`, `verify`, `apply` rebuilt on `Document`
(`apply` writes through `Document::apply`); `convert` and
`migration_advice.rs` deleted.

**Acceptance:** maptool tests rewritten against the new format.

## E2 — Documentation rewrite

**Phase E · depends on: D3, E1**

CONCEPTS (§1–§5 rewritten around the engine, the document, TextFlow,
Edits / Presentations / verbs), CODE_CONVENTIONS and baumhard
CONVENTIONS ("mutation-first" stated exactly), TEST_CONVENTIONS (oracle
and journal properties), every file under `format/` (schema, ids,
channels, spans, frames, edits, presentations, verbs, macros, theme,
palettes, validation, enums; `migration.md` deleted), CLAUDE.md
(dual-target registry, common tasks), `format/ipc.md`.

**Acceptance:** no document describes a path that does not exist; the
existing doc gates pass.

## E3 — Switch-over

**Phase E · depends on: all of the above**

Merge `main` into the branch, run the full gate including GPU tests,
record §3's four flows, merge into `main`.

---
---

# EPIC V2 — Deferred features

**Goal.** Hold every feature that was discussed during the V1 design
and deliberately left out of it, documented well enough to implement
without re-deriving the context. Nothing here starts before V1's
switch-over (E3), because each item is designed against the V1
architecture.

Each sub-issue cites the V1 design section it builds on.

---

## V2-1 — Buttons on nodes that invoke verbs

A node can present a button that invokes a map verb. Open design
questions to settle first: what a button is in the document (a node
property, a section kind, a span style), how it is drawn in glyphs,
its states (rest, hover, pressed, disabled), and how it is targeted by
presentations. Builds on §11 (verbs) and §6 (TextFlow).

## V2-2 — Trigger mechanisms beyond click, hover and links

The trigger design discussion deferred from V1: keys bound to map
verbs (user keybinds and map-declared), map load, timers, and
sequences / autoplay (a map that performs itself). Includes whether
`edit_bindings` should accept map verbs. Builds on §11.2.

## V2-3 — Verb parameters

Verbs that take arguments (a target, a color, a duration). The V1
step format leaves room for them without a breaking change. Builds on
§11.1.

## V2-4 — In-app authoring of edits, presentations and verbs

V1 loads, validates, saves and invokes these definitions but never
changes them (§5.7). V2 makes them editable in the app — and, by the
V1 principles, every such change is itself an `EditRequest` with undo.
Requires moving the definitions into the document tree or giving them
their own journaled store.

## V2-5 — Node shadow

Deleted from V1 as a dead field (`show_shadow`). Design how a shadow
is drawn (shader, alongside the fill's fade, or glyphs) and how it
interacts with frames.

## V2-6 — More node shapes: hexagon, diamond, parallelogram

The format has long named these shapes without being able to draw
them. Each needs an SDF fill case (§13.4) and an outline path for
frames (§6.3), including what corner radius means for it.

## V2-7 — `RotateWhile` instruction

The mutator language reserved `RotateWhile(f32, Predicate)` — sibling
rotation — but the walker never implemented it, and V1 removed the
stub (§4.2). Specify its semantics against the generic engine and
implement it on both instantiations.

## V2-8 — Create a child node

The app today can only create orphan (root) nodes. Add creating a
child (and sibling) of the selection, as an `Insert` structural edit
with its connector inherited from defaults.

## V2-9 — Fold and unfold

`folded` exists in the format and the projection honors it, but no
action writes it. Add fold / unfold / toggle actions (an edit), and
decide whether folding can also be a presentation (temporary reveal).

## V2-10 — Section merge and reorder

Sections can be added, deleted and split, but not merged or
reordered. Both are structural edits on the V1 engine.

## V2-11 — Touch platform context for triggers

`PlatformContext::Touch` exists but is never produced. Decide how
touch input maps onto triggers (there is no hover on touch) and which
verbs a map can declare touch-specific.
