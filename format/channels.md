# Channels

Every node carries a `channel` integer field (default 0). The channel is an
index that determines which mutators apply to the node when a mutation
tree walks the baumhard tree.

```json
"0": { "id": "0", "channel": 0, ... },
"1": { "id": "1", "channel": 1, ... },
"2": { "id": "2", "channel": 1, ... }
```

## What a channel is

A mutator tree walking the GfxElement tree matches children against target
children by key — an element's kind and channel. The glyph elements a
mindmap is drawn with have a single kind, so for them the key is the
channel: when a mutator on channel N reaches a tree level, it applies to
all children on channel N and skips the rest.

Channels are **not unique**. Multiple siblings can share a channel to form
a broadcast group — one mutator hits all of them. They are **not
hierarchical**. A parent and child can share a channel, or have different
channels; the walker only compares channels within the same sibling level.

## Why mirror this on MindNode?

The baumhard tree (the GfxElement layer that renders nodes) already has
a channel on every element. Until now, `tree_builder` hardcoded channel 0
for every mindmap-derived element, which meant every mutation broadcast
to every child — no way to target a subset.

Adding `channel` to `MindNode` surfaces the runtime concept in the format,
where authors can tag children as "emphasis" (channel 1) vs "normal"
(channel 0) and apply different animations to each group.

This is the foundation for:

- **Selective custom mutations** — `CustomMutation` with a `ChildrenOnChannel(n)`
  target scope (future work) applies only to children on a specific channel
- **Partial animations** — animate the channel-2 children out without
  touching the channel-0 children
- **In-place mutation updates** — the existing pattern used by portals,
  borders, and connections (where stable channels enable delta application
  instead of full tree rebuild) becomes available to node mutations

## Sibling order

Every baumhard tree keeps each parent's children in ascending key order —
for the glyph trees, ascending channel — and siblings that share a channel
stay in the order they were added, which for mind-nodes is id order
(`id_sort_key`). The walker relies on this to pair mutator children with
target children in one pass. The tree keeps the order itself: every child
is inserted at the place its channel gives it, and a child whose channel
changes is moved.

From the format perspective: you can set channels however you like.
The runtime handles ordering.

## Backward compatibility

Every node in an existing map defaults to `channel: 0`. All current maps
behave identically before and after the channel field was added. Users
opt in by setting non-zero channels on specific nodes.
