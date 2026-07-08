---
name: to-issues
description: >-
  Turn an aligned feature into a set of small, self-contained GitHub issues, each
  buildable cold by a fresh agent session. Use right after a `/vision-align` session, or
  when asked to "split this into issues", "break this plan into tickets", or "decompose
  this work". Not for shaping the feature itself — that's `/vision-align` — and not for
  building an issue, which is `/implement`.
disable-model-invocation: true
---
# To Issues

Run in the same window as the `/vision-align` session — the reasoning behind the
decisions is input. Ask the human when a split isn't forced by what's on the
table; questions here are about decomposition (boundaries, seam ownership,
ordering), not re-opening the feature.

## 1. Decide the split

Read the repo first. The unit is one agent session's worth of work — don't split
below it; coordination costs more than it saves. Two issues are parallel only if
the file sets they touch are disjoint (verify against the repo, don't assume);
everything else is sequential, ordered by dependency. Where two meet at an
interface, define the seam — types, signatures, contract — now, and write it into
both.

## 2. Write each issue

The assigned agent starts from the issue and a fresh clone, nothing else. Each
issue must clear the bar:

<!--slot:ready-for-agent-->
An issue is ready for an agent when a fresh clone plus this text settles all of it:

- **Context** — current behavior and the reason for the change, enough for someone
  who wasn't in the planning session.
- **Grain** — any project invariant or direction the change must honor that a fresh
  clone wouldn't make obvious: a principle held, a pattern being migrated toward, the
  reason the locally-obvious implementation would be wrong. Skip when nothing
  non-obvious is at stake.
- **Change** — the outcome to reach, described by behavior, not a procedure.
- **Seams** — the contracts it must meet or produce (types, signatures), named where
  they touch existing code by stable symbol, not line number.
- **Acceptance** — testable criteria that decide when it's correct and done.
- **Out of scope** — what this issue explicitly does not touch.
- **Self-contained** — no "as discussed", no leaning on another issue's body; a seam
  shared with a sibling is written here too.
<!--/slot-->

## 3. Publish

Create in dependency order, blockers first, so their numbers exist. For 3+ related
issues, create a parent tracking issue first for the summary and ordered list, and keep
each child's actionable spec in the child.

With `gh` (≥ 2.94 for native dependency links):

    gh issue create --title … --body … --label ready-for-agent \
      [--blocked-by N] [--parent E]

Dependencies and parent links are set natively — agents and humans read the same graph.

With a GitHub MCP server instead of `gh`: create each issue through the server's
issue-write operation with the label included — a missing label **auto-creates on first
apply** (see `setup-skills`), so there's no separate setup step and no reason to stall
waiting for one. Nest children under the parent with the server's add-sub-issue operation
(it identifies the child by its internal id, not its issue number). If the server exposes
a native issue-dependency operation, set the blocker with it; otherwise there is no
native blocked-by link over MCP, so **state each issue's blocker in its body** ("Blocked
by #N — don't start until it's merged") and let the parent's ordered list be the source
of truth for the chain.

Report the created issues with their order and blocking structure.
