# Projection — where a map is drawn

How a `.mandala.json` map's elements become glyphs: which are drawn, in
which layer, where and how. The fields are in [`schema.md`](./schema.md);
this is what the projector (`lib/baumhard/src/projection/`, V1-B7 #241,
design §12) makes of them. Everything here is derived: nothing on this
page is saved.

## Layers

A map is drawn in five glyph layers, bottom to top:

| Layer | What it holds, per node |
|---|---|
| sections | one box run per section, in channel order |
| frames | one path run per ring of a drawn frame, innermost first |
| connections | the node's connector — its body, then its caps — then each line link's, in order |
| labels | the connector's label, then each line link's |
| portals | for each portal link: end 0's marker and label, then end 1's |

There is no fill layer: the map's `background` is the color every frame
is cleared to, and every other pixel is a glyph's (D12).

Every drawn node has a group in every layer, groups nesting as the
nodes do, so a node's glyphs are under its parent's in each layer.

## What is drawn

- A node is drawn unless it is `hidden`, or an ancestor is `hidden` or
  `folded` (a folded node itself is drawn; its descendants are not).
- A connector is drawn when its `visible` is true and its node is drawn.
- A link is drawn when both its node and the node it leads to are
  drawn; a line link also needs its `visible`. A link to a node that is
  not drawn draws nothing until that node is drawn again.
- A frame's rings are drawn when the frame's `visible` resolves to true.
- A cap, a label or a portal end's label is drawn with its path or its
  end.

## Where

**A node's drawn rectangle** starts at its `position`. Its size is the
larger, per axis, of its `min_size` and the far corner of its sections'
boxes measured from its position (each section's `offset` plus its box's
size); at least zero. A `min_size` axis that is not finite or is negative
reads as zero.

**A section's box** sits at the node's position plus its `offset`, as
large as its `min_size` and its text, whichever is larger per axis. Its
lines are aligned by `align`; unset is `left`.

**The outline** is the node's rectangle shaped by its `shape`
([schema](./schema.md#shape)). A frame's ring *k* runs around the outline
grown by `inset`, the line heights the rings inside it were placed at,
and `k × gap`, its glyphs set outside it.

**A ring's size** is relative: `size × min(width, height)` of the node's
drawn rectangle, in points, held to the shapeable range (0.5 pt to
4096 pt). The stored size is never changed by that hold: a relative size
scaled very small still loads and saves as written.

**A path** (a connector, a line link) runs from its start node to its end
node: a connector from its node's parent to its node, a link from its
node to the node it leads to.

- **Where it attaches.** Each end sits at the midpoint of one side of
  its node's *reach*: the box round the inks of the node's drawn rings,
  or its drawn rectangle where it draws none. The side is the one its
  `anchors` entry names, or for `auto` the side the ray from the reach's
  center toward the other node's reach's center leaves through: left or
  right where the ray's `|dx|` over the reach's half width is the larger,
  top or bottom otherwise.
- **How it runs.** It leaves its start and arrives at its end square to
  their sides, and passes through its [control
  points](./schema.md#control-points): one cubic between each two points,
  joined without a kink. An end's handle reaches 0.5523 of how far the
  curve's other end lies along it, and at least a third of the curve's
  chord: between facing sides an S as deep as the sides are apart,
  between a side and one turned a quarter from it a round bend. A
  [`straight`](./schema.md#connector) path is one segment from its
  start's side midpoint to its end's instead; at a shared side it keeps
  the direction that segment leaves in, and its first glyphs are held
  out as any path's are.
- **Paths that share a side.** The paths that leave one side of a node
  leave its one midpoint, fanned in the order they turn toward their
  other ends, as seen from where their first two glyphs end. Each two
  neighbors are apart by the turn at which their first glyphs clear each
  other by their own gap — the widest any of them needs — and none turns
  more than 40° from the side's normal: past that the fan is narrowed to
  hold it, and their first glyphs are held out until they clear. A
  straight path is not turned: it takes its place in the order by the
  direction it leaves in, and the others fan round it, as near the even
  fan as keeps them that far from it (between it and the normal they
  may turn as far as it does), and the paths between two straight ones
  share the room between them. Each path is held out as far as the step
  and its own two neighbors need — further only for a neighbor nearer
  than the step — but never past a quarter of the straight line to its
  other end, so it keeps its glyphs whatever its neighbors do. Two
  straight paths that leave in exactly one direction cannot be told
  apart, and overlap. Along
  their whole length no two come nearer each other than their own gap
  and a glyph: two that would turn toward their other ends sooner, their
  curves' handles shortened until they keep clear. A path alone on its
  side leaves along the normal.
- **Its glyphs.** A `repeat` path whose text has ink to measure — an
  ornament, `·` or `~` — holds a whole number of glyphs: as many as
  bring every gap between two inks, box to box, nearest the gap its font
  sets them at on a line plus its `spacing`, the difference spread evenly
  over all of them. The gaps are the same whichever way the path runs,
  so a wide glyph steps further along a level stretch than down a steep
  one. The gaps at its two ends are among them, each measured from
  its end glyph's ink to the nearest ink of the node's rings, or to its
  rectangle: the end glyph moves back past the side's midpoint, through
  the rings and never into the rectangle, or on, to keep it. Any other
  path is walked by its fit mode from its start.
- **Behind a node.** A node's contents take priority over every path
  that does not end at it. A glyph of a path's body or caps is not drawn
  where its box — its ink, or its advance by its line height where it
  has none — grown by the path's own gap meets the node's *region*, in
  two parts: the box round its sections' glyphs (each its advance by its
  line's height, and no glyph of whitespace), a rectangle; and the box
  round its rings' inks, shaped by its `shape` (an ellipse's is the
  ellipse in that box). Not its drawn rectangle: a box wider than its
  text, or a `min_size`, hides nothing past the text, and a node that
  draws nothing hides nothing. So a path shown on either side of a node
  stops at least one gap short of it — a gap, and up to one step more
  where the next glyph falls inside — as it stops short of its own
  frames. A glyph is drawn whole or not at all, never clipped. The path's
  own two nodes never hide it. A node hides a path only if it is drawn
  wherever the path is: a node at opacity 0, or whose zoom window does
  not hold the path's, hides nothing of it. A path whose two nodes both
  lie inside another node's ringed region — a ringed container drawn
  round them — is hidden whole. Hiding changes nothing in the document:
  the glyphs are drawn again when the node moves off the path. A path's
  label is not hidden.

**Caps** are centered on the path's start (`cap_start`) and end
(`cap_end`).

**A label on a path** centers at `t`, a fraction of the path's length
(held to `0..1`), moved `offset` along the path's normal there — positive
to the left of the direction of travel, so above a path running right —
its lines centered.

**A portal end** sits on its own node's outline: at `border_t`, a
fraction of the outline's perimeter from the start of its top side,
clockwise; unset, where the ray from its node's center toward the other
node's center leaves the outline, so it faces the other end. The ends
with `border_t` unset at one node are held apart along the outline, no
two nearer, center to center, than half the larger sides of their
markers' and labels' boxes summed; a run of them that had to move is
centered where they face. It is moved
`offset` along the outline's outward normal there, and its marker is set
just outside that point. Its label sits beside the marker, further out
along the same normal by the label's `offset`; a portal end's label has
no `t`.

## How

- **Opacity** multiplies down the tree: a node's runs are drawn at its
  own `opacity` times every ancestor's. A connector is its node's; a
  link — its body, caps, label and both portal ends — is its source
  node's.
- **Zoom windows** are evaluated when drawing, never by projection:
  sections and rings render in their node's `zoom`; a connector, a line
  link and their caps and labels in the path's own `zoom`; a portal
  link's ends in its source node's.
- **Colors** are each glyph's own, resolved by the style chain
  ([schema](./schema.md)); nothing is filled.
