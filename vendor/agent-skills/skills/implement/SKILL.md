---
name: implement
description: >-
  Implement a ready-for-agent GitHub issue end to end — confirm its blockers,
  build it in the project's own conventions, verify, open a PR, and stay on it.
  Use when handed a ready-for-agent issue or asked to "implement this issue" or
  "/implement"; the ready-for-agent branch of /issue runs it. Not for shaping or
  splitting work — that's vision-align and to-issues.
---
# Implement

You've been handed a single ready-for-agent issue: build it, verify it, ship it,
and stay on the PR. The issue is the contract for *what*; this codebase is the
contract for *how*.

## 1. Ground yourself

If you weren't handed an issue — a link, a number, or one `/issue` routed here —
list the open `ready-for-agent` issues (`gh issue list --label ready-for-agent`)
and ask which to take; don't pick one yourself. Otherwise fetch it (`gh issue
view`, or a GitHub MCP server), and confirm every blocker is closed; if one is
still open, stop and tell the human — don't build on an unmerged foundation.

Then read `AGENTS.md` / `CLAUDE.md`, any nested convention docs, and the code
around what you'll change. **The project's conventions, philosophies, idioms, and the
direction it's moving are the bar** — hold to them over generic best practice. A change
that is clean in the abstract but breaks how this project does things — or quietly works
against where it's headed — is wrong here.

## 2. Build it

Implement to the issue's acceptance criteria, at the seams it defines, in the
codebase's own patterns — its naming, structure, error handling, and stated
philosophy. Reuse what's there; don't add a second way to do something the project
already does one way. Keep the gates (typecheck, lint, tests) green as you go, not
only at the end.

If at any point — first read or mid-build — you hit a decision the issue doesn't
resolve, do not guess: post the open question as a comment on the issue, swap
`ready-for-agent` for `needs-vision-align`, tell the human in-session in plain text,
then stop working on this issue. If that swap fails because `needs-vision-align`
doesn't exist on the repo, say so and ask the human to run `/setup-skills`.

## 3. Verify

Run the project's full gates and suite. Confirm the change does what the issue
asked — exercise it, don't assume (your tool's verify, e.g. `/verify`). Review the
diff for correctness before shipping (e.g. `/code-review`).

## 4. Ship and stay

Open a PR that closes the issue, describing what changed against the acceptance
criteria. Then run `/address-pr-comments` to stay subscribed through merge.
