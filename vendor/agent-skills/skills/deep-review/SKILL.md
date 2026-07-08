---
name: deep-review
description: >-
  Run a thorough, high-standard review of the EXISTING codebase and design (not
  a new feature). Use when asked to "review the whole solution", "audit the
  project before release", find "what can be done better", or assess technical
  debt, duplication, or convention drift in depth. Fans out parallel dimension
  audits, fixes what is unambiguously right, and surfaces the judgment calls.
---

# Deep Review

Repository-wide quality review: parallel dimension audits, a hard
safe-vs-needs-input split, re-verify before acting. Breadth comes from fanning
out; trust comes from the orchestrator validating every finding against source.

## 1. Ground the review

Read `AGENTS.md` / `CLAUDE.md` and any conventions docs — **these are the bar**;
measure against the project's own standards, not generic best practice. Capture
a baseline so findings aren't vibes: build/test time, output size and largest
artifacts, suite green. Note the numbers.

## 2. Fan out one audit per dimension (parallel, read-only)

Three dimensions apply to any project:

1. **Conventions / duplication / single source of truth** — duplicated shape,
   parallel paths, values defined twice, history-narrating comments, dead code.
2. **Build + test** — build-time hotspots, flakiness/ordering, coverage gaps,
   untested invariants, gates.
3. **Tidiness / complexity / debt** — oversized multi-concept files, needless
   indirection, magic numbers, lying names; navigability signposts.

Then add the domain dimensions — the ones this project's users would feel.
List this skill's directory for domain catalogs (`WEB.md`, `EMBEDDED.md`, …)
and use the one matching the project. No matching catalog: derive the
dimensions yourself, and write what you derived back as a new catalog file.

Spawn one subagent per dimension, in parallel.

## 3. Verify — do not trust blindly

Re-verify every finding against source before acting; for a visual or contrast
claim, open the actual file or image. A finding you haven't confirmed is a
hypothesis. Reconcile cross-audit disagreements by reading the code — a
"duplication" may be generated output — and demote anything that doesn't survive
a look.

## 4. Fix the safe set

Apply the SAFE-TO-FIX findings honoring the project's idioms and existing seams
(reuse, don't add a parallel path). Run lint, typecheck, and the suite after;
when a fix protects an untested invariant, add the test. Do **not** start the
refactor a NEEDS-INPUT finding implies. State plainly what changed.

## 5. Report and present

Write a committed report: fixed (with rationale), needs-input (evidence +
recommendation + why it's not unilateral, ordered by value), and verified
strengths. Quote copy findings exactly. Then present in chat as a discussion
opener, in plain text, no picker — the user decides the NEEDS-INPUT items.