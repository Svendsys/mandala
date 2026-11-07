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
node to the node it leads to. Each end attaches to the midpoint of one
side of its node's rectangle, which lies on the outline whatever its
shape: the side its `anchors` entry names, or for `auto` the side
midpoint nearest the other node's center (top, right, bottom, left, in
that order on a tie). With no control points the path is a line; with
one it is a quadratic Bézier whose control point is that offset from the
start node's center; with two a cubic, the first an offset from the start
node's center and the second from the end node's.

**Caps** are centered on the path's start (`cap_start`) and end
(`cap_end`).

**A label on a path** centers at `t`, a fraction of the path's length
(held to `0..1`), moved `offset` along the path's normal there — positive
to the left of the direction of travel, so above a path running right —
its lines centered.

**A portal end** sits on its own node's outline: at `border_t`, a
fraction of the outline's perimeter from the start of its top side,
clockwise; unset, where the ray from its node's center toward the other
node's center leaves the outline, so it faces the other end. It is moved
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
