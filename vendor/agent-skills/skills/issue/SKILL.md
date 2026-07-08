---
name: issue
description: Address a github issue
disable-model-invocation: true
---
# Issue

If no issue was specified, list open issues labelled `needs-vision-align` or `ready-for-agent` (`gh issue list --search "label:needs-vision-align,label:ready-for-agent"`, or a GitHub MCP server) and ask the human which is yours.

## Process

**`needs-vision-align`** — run a `/vision-align` session with the human for what only they hold (the direction, the real goal, the Grain); derive the routine bar items — Change, Seams, Acceptance, Out of scope — yourself from the code and the request, not by interrogating the human. Write the result onto the issue until it clears the bar below, swap `needs-vision-align` for `ready-for-agent`, then stop — a fresh session implements.

**`ready-for-agent`** — the issue is the contract. Run `/implement`.

## The bar

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