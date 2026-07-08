---
name: vision-align
description: >-
  Before you turn a feature request into a spec, plan, or design — especially when
  you can still talk to the person who asked — interview them to surface the vision
  you can't infer from the code and the request alone: where the project is really
  headed, what the change is ultimately for, and the invariant that lives only in
  their head. Get it into the spec so an unattended build serves the intent, not
  just the letter of the ask. Reach for this whenever a colleague describes a
  capability they want added and you're about to spec or build it, or on cue: "grill
  me", "align on the vision", "is this ready to build", "what am I missing". Not for
  a request already fully settled by the code and the ask, not for routine decisions
  a capable agent makes itself, not a substitute for doing the work.
---
# Vision align

A capable agent already understands the request and reads the code fine — it will pin the
scope, the edges, and the data shapes on its own. Interrogating it about those is the
nonsense to cut. What it *cannot* see is what lives only in your head: where the project
is really headed, what this change is ultimately *for*, the line it must not cross. This
interview exists to get that onto the page — so a fresh, unattended session builds toward
the vision instead of a locally-correct thing that quietly works against it.

## What only you can answer

The model derives everything the request and the code already imply. Spend the interview
on what they don't:

- **Direction** — where the project is going that this change has to stay compatible with:
  a pattern you're migrating toward, a dependency you're shedding, a boundary you hold. The
  obvious build often fits today's code and fights tomorrow's.
- **The real goal** — what the request is *for*. The literal ask is a proxy for an outcome;
  name it. If the obvious build meets the letter but misses the point — or the ask is a
  footgun or the wrong solution to the goal — say so and propose the path that serves the
  goal. Never faithfully spec a mistake.
- **The line** — an invariant that matters to you and is written nowhere: "the core stays
  pure", "stays usable offline", "never block the render". A fresh reader can't infer it, so
  a fresh session will cross it.
- **The call** — where more than one build is defensible, which one is *right* here, and
  why. That judgment is yours; the code doesn't spell it out.

## How to run it

- **Open with the catch-all.** Before you drill anything, ask the one question that finds
  hidden depth without your having to guess where it lives: *is there anything you should
  know?* Cast it wide, but name the axes so it lands — where this is headed, who or what
  depends on it or consumes what it produces, and anything it must never do, even under
  failure. Assume you're talking to the person who holds the whole picture: a good catch-all
  pulls more out of the author than ten targeted questions. A real "no, it's as it looks" is
  a genuine answer — believe it and keep the interview short; anything else is the catch, and
  the catch is the interview.
- **Then drill one thing at a time, carrying your best guess.** Once the catch-all names a
  dimension, switch from open questions to stated assumptions: say what you'd build and why,
  and let the human confirm or correct. A wrong guess said out loud pulls the real answer out
  faster than another open question — the catch-all is the one place open wins, because there
  you don't yet know which dimension to guess.
- **Turn what you hear back on the plan.** When the obvious implementation fights the
  direction, the goal, or the line, say so, name the aligned alternative, and let the human
  choose with eyes open. That catch is the whole point.
- **Trust the model with the rest.** Derive whatever the code and the request already settle
  instead of asking. Raise a routine decision only when it's genuinely load-bearing *and*
  ambiguous, then move on. A long interview over settled ground is the failure this skill
  exists to end.

## Done

Stop when the spec carries what a fresh session couldn't have inferred: the direction it
serves, the goal behind the ask, the lines it can't cross. State them back, get
confirmation, and hand off — persisting them into the issue is the caller's job, not this
one's.
