# Validation

A file that serde can deserialize is **syntactically** valid but might
still be **semantically** broken: dangling edge references, parent_id
pointing to a nonexistent node, Dewey IDs that disagree with `parent_id`,
palette references that don't resolve. These are caught by:

```
maptool verify <map.json>
```

Exit code 0 if clean, or if only warnings are found. Nonzero with a
list of violations if any error is found. Warnings are still printed so
a CI recipe that captures stderr can see them.

The split is three-way, not two, and the middle column is the one
that moved.

1. **Spelling — the loader refuses.** Every object in the format is
   closed, so a key no field claims fails the load outright rather
   than being dropped and then erased at the next save. See
   [schema.md](./schema.md#unknown-keys-are-rejected).
2. **Render safety — the loader also refuses.** A correctly-spelled
   file can still carry numbers that take the editor down rather
   than merely looking wrong: a zero font size trips an assert
   inside the text shaper, a crossed size window used to reach an
   `f32::clamp`, and node geometry sizes real allocations. Those are
   rejected at load, because a map that would abort the process must
   not open. The rules are listed under
   [Numeric domain](#numeric-domain) below, and the reasoning is in
   [macros.md](./macros.md#the-load-time-trust-boundary).
   **Parent cycles and the key-equals-`node.id` rule are in this
   column too** — both are load-blocking, because the cycle check
   and the scene builder address nodes by different spellings, and a
   disagreement between them is a walk that never terminates.
3. **Coherence — `verify` reports.** Everything else: a file that
   loads and renders but still says something incoherent. Dangling
   edge references, a Dewey ID that disagrees with `parent_id`, an
   unresolved palette name, a section hanging past its node's edge.

`verify` sees all three. It loads through
`loader::parse_for_inspection` rather than the editor's strict door,
precisely so it can still open the files column 1 and 2 reject — a
tool whose job is saying *what is wrong with this map* would be
useless if it could only read maps that were already fine. So a
violation listed in column 2 appears in a `verify` report **and**
blocks the load; one in column 3 appears only in the report.

## What gets checked

### Tree structure

- Every non-null `parent_id` points to a node that exists in `nodes`
- No cycles in the `parent_id` chain

**Why**: a node whose parent doesn't exist is unreachable through tree
traversal. A cycle makes `all_descendants` loop forever.

### ID consistency

- The HashMap key equals `node.id` for every entry
- For every non-root node, `derive_parent_id(node.id)` agrees with
  `node.parent_id`
- Root nodes (`parent_id: null`) have no dot in their ID

**Why**: the Dewey ID encodes structure. If `"1.2"` claims its parent is
`"0"`, either the ID is lying or `parent_id` is — which one loads right
depends on which code path runs, a reliability nightmare.

### References

- Every edge's `from_id` and `to_id` exist in `nodes`
- No two edges share the same `(from_id, to_id, edge_type)` tuple

**Why**: dangling references silently disappear at render time — the
connection just doesn't draw, with no indication that something was
lost. Applies uniformly to line-mode and portal-mode edges.

Duplicate tuples break edge identity: `EdgeRef` lookups return the
first match, `SceneConnectionCache` overwrites with the second edge's
geometry, and the on-disk format no longer round-trips through the
runtime faithfully. Multiple edges between the same pair with
different `edge_type` values are allowed.

### Palettes

- Every node with `color_schema` references a palette that exists in
  `map.palettes`
- Every palette has at least one group

**Why**: a missing palette falls back to the node's base `style` colors,
which silently wipes the theme. An empty palette produces no colors at
any level.

### Named enums

- `style.shape` is one of `"rectangle"`, `"rounded_rectangle"`,
  `"ellipse"`, `"circle"`, `"diamond"`, `"parallelogram"`, `"hexagon"`
  (compared case-insensitively)
- `layout.type` is one of `"map"`, `"tree"`, `"outline"`
- `layout.direction` is one of `"auto"`, `"up"`, `"down"`, `"left"`,
  `"right"`, `"balanced"`
- `edge.line_style` is one of `"solid"`, `"dashed"`
- `edge.anchor_from` and `anchor_to` are one of `"auto"`, `"top"`,
  `"right"`, `"bottom"`, `"left"`
- `edge.type` is one of `"parent_child"`, `"cross_link"`
- `edge.display_mode` (if present) is one of `"line"`, `"portal"`

See [enums.md](./enums.md) for the complete lists.

**Why**: the renderer falls back to defaults on unknown values. An author
typo (`shape: "retcangle"`) silently becomes a plain rectangle. Verify
catches the typo.

### Text runs

- Runs do not overlap (which implies ascending `start` order for
  well-formed runs)
- Each run's `start < end`
- `end` is within the text's grapheme-cluster count

**Why**: overlapping or out-of-bounds runs produce undefined rendering —
the first run wins silently, the tail is clipped. Rich text bugs are
painful to diagnose after the fact.

### Section bounds

For every `MindNode.sections[i]`:

- `offset.{x,y}` finite and non-negative
- The section's effective AABB (`offset + effective_size`) is inside
  the parent node's `size`. The effective size is the explicit
  `section.size` when set, otherwise `node.size` (fill-parent).
- `size.{width,height}` (when set) finite and strictly positive, and
  not over 100× the parent's matching dimension (typo guard)
- The owning `node.size.{width,height}` itself finite, strictly
  positive, and not over `MAX_NODE_AXIS` (`1_000_000.0`) (sub-check,
  since a corrupt node-size cascades into every section's AABB math)
- `node.sections.len()` does not exceed `MAX_SECTIONS_PER_NODE`
  (`1024`)
- No two sections share the same effective channel under the same
  parent — the effective channel is `section.channel.unwrap_or(section_idx)`.
  Surfaced as a *warning*, not a hard rejection: the broadcast
  is well-defined (a mutation targeting the shared channel hits
  every listed section), and authors who deliberately want
  broadcast can ignore it. Most cases are typos.

**Why**: an out-of-bounds section silently overflows its parent
container at render time and breaks hit-testing; a NaN at the
node level poisons every downstream AABB comparison. The console
verbs `section move dx=<dx> dy=<dy>` and `section resize w=<w> h=<h>`
([sections.md](./sections.md)) enforce these same rules at edit
time and surface byte-equal rejection messages — a verb-rejected
edit and a `verify` violation read identically.

### Zoom bounds

- `min_zoom_to_render` and `max_zoom_to_render` (when set) are finite
- Whenever both are set on a `MindNode`, `MindEdge`,
  `EdgeLabelConfig`, or `PortalEndpointState`, `min <= max` holds

**Why**: an inverted pair is a well-defined but always-invisible
window — the render-time check still terminates cleanly, but an
element that never renders at any zoom is almost always a typo. See
[zoom-bounds.md](./zoom-bounds.md).

## What's not checked

- **Color format** (`#RRGGBB` vs `rgb(...)` vs named colors): the format
  says hex or `var(--name)`, but the renderer is lenient. We don't verify
  color syntax — authors who type `"red"` will see default colors, and
  that's easy to diagnose visually.
- **Positions and sizes** are only *partly* unchecked now. A negative
  position is still valid — the canvas is unbounded — but a zero,
  negative, non-finite, or astronomically large extent is a
  load-blocking violation rather than a curiosity, because those
  numbers size real allocations downstream. See
  [Numeric domain](#numeric-domain).
- **ID stability after reparent**: Dewey IDs can drift from parent_id
  after a runtime reparent (documented in [ids.md](./ids.md)). Verify
  **does** flag ID/parent_id mismatches — saving a reparented map and
  running verify will report the drift as a violation. This is
  intentional: the on-disk format should be consistent, even if the
  runtime allows transient drift.
- **Referential integrity of `trigger_bindings.mutation_id`**: if a
  binding references a mutation ID that doesn't exist, the binding is a
  no-op at runtime. Verify could be extended to flag this; currently it
  doesn't.

## Numeric domain

Load-blocking. Every number a `.mindmap.json` carries into the scene
build has to be one the renderer survives, because the code
downstream of it is not defensive — the text shaper asserts, and the
border and connection builders size a `String` and a `Vec` from
authored geometry. `verify` reports these under the `numeric`
category; the loader refuses them outright.

| Rule | Bound |
|---|---|
| Font metrics — `font_size_pt`, `size_pt`, every min / max on borders, connections, labels, portals and text runs | finite, `0.5`..=`4096` pt |
| Min / max font pairs | not inverted (the cascades clamp with them) |
| Node extent | finite, positive, ≤ `1_000_000` per axis |
| Positions, section offsets, Bezier control points | finite, `|v|` ≤ `1e9` |
| Text runs | sorted, non-overlapping, non-empty, `end` ≤ the section's grapheme count |
| Connection body / cap glyphs | ≤ 16 grapheme clusters **and** ≤ 512 bytes (each is re-emitted per sampled point, and one cluster can carry unlimited combining marks — the cluster count keeps it a motif, the byte count keeps `bytes × samples` finite) |
| Animation envelope — `duration_ms`, `delay_ms` | ≤ `60_000` ms each |
| Zoom windows | finite, non-negative, not inverted |
| Whole file | ≤ 256 MiB |

The constants live in `lib/baumhard/src/mindmap/model/validate.rs`
and `lib/baumhard/src/font/fonts.rs` (with `MAX_NODE_AXIS` in
`model/node.rs` and the font window re-exported from `font/fonts.rs`),
and the loader, the document setters, and `verify` all read the same
ones.

The setters clamp rather than pass through, so a value the editor
writes is a value the loader accepts. That is a property maintained
by hand at each setter, not one the types enforce: the console
parsers are deliberately permissive — `parse_finite_pt` takes any
positive finite `f32`, the `spacing` verb any finite one — so a new
bounded field is a lockout waiting to happen until its setter is
clamped too. `border padding=` and `spacing` were exactly that, and
`test_extreme_editor_writes_still_reload` is where the property is
pinned: it drives each setter past its bound and round-trips through
the real save and the real strict load.

## Running verify in CI

`maptool verify` exits 0 on success, nonzero on violations. A CI job that
verifies every `.mindmap.json` in the repo is a natural safety net:

```bash
for f in maps/*.mindmap.json; do
  maptool verify "$f" || exit 1
done
```

## Violation output format

Errors:

```
<category> @ <location>: <message>
```

Warnings are printed the same way but prefixed with `warning:`:

```
warning: sections @ 0: channel 0 shared by sections [0, 1]; ...
```

Examples:

```
tree @ 0.0: parent_id "ghost" references a node that does not exist
ids @ 0.0: parent_id "ghost" does not match derived parent "0"
ids @ 1: HashMap key "1" does not match node.id "DIFFERENT"
references @ edge[0]: to_id "nowhere" is not a node
palettes @ 0: palette "nonexistent" is not defined in map.palettes
enums @ 0: style.shape "oblong" is not one of ["rectangle", "rounded_rectangle", "ellipse", "circle", "diamond", "parallelogram", "hexagon"]
numeric @ 0: zoom: min (2) is above max (0.5) — no zoom satisfies both bounds, so this never renders at any zoom level
numeric @ 0: section[0].text_runs[1] overlaps previous run (start 3 < previous end 5) — overlapping runs re-emit the same graphemes once per covering run
```

Each violation names its category, the location inside the file, and what
went wrong. The location format varies by category (node ID, edge index,
etc.) but is always clickable / greppable.
