---
name: optimize-skill
description: >-
  Improve a skill until it measurably pays off — the loop that benchmarks a
  skill, reads where it fails, edits it, and re-benchmarks on the same ruler
  until a stronger revision wins. Use when asked to "improve / optimize / develop
  a skill", "make this skill better", "search for a better version of the skill",
  or to run a skill-improvement loop. Drives `/benchmark-skill` as its
  measurement subroutine — reach for that one directly when you only need to
  measure a fixed skill, not change it.
---
# Optimize a skill

Benchmarking measures a skill; **optimizing is the goal that uses it.** A benchmark tells
you whether one frozen revision pays off and where it fails; this loop turns that signal
into a *better* skill — read the failures, form a hypothesis, edit the skill, measure
again on the same ruler, keep what wins. The instrument is `/benchmark-skill`; everything
here is about spending it well and knowing when you've converged.

The deliverable is a **stronger skill revision**, backed by a benchmark cohort that shows
it beats the one you started from. Keep the two operations distinct: you never change how
a benchmark runs to move a number — you change the skill, and let the unchanged ruler
report the result.

## The loop

Find a strong version of the skill by iterating:

1. **Benchmark it cheaply.** Run `/benchmark-skill` on the current skill for a baseline — a
   first, small-sample read of where it wins and where it stalls. If a study already exists
   on disk, that baseline is already there; reuse its ruler rather than designing a new one.
2. **Read where it fails, at the source.** Go to the run transcripts and `evidence.md`, not
   the summary numbers. The failing arm shows you *what the agent actually did* — the wrong
   turn, the move it never made, the trigger it never reached for. That behaviour is the
   thing to fix, and only the source shows it.
3. **Form a hypothesis and edit the skill.** Name *why* it failed — a vague trigger, a
   missing gotcha, an instruction the weak model reads the wrong way — and change the
   `SKILL.md` to address exactly that. One lever at a time, so the next measurement
   attributes cleanly.
4. **Re-benchmark as a fresh `skill_rev` on the same ruler.** Re-run the *committed harness*
   unchanged against the edited skill, append the result as a new cohort under its new
   `skill_rev`, and compare it to the last. Same scenarios, same prompts, same pinned models
   — a moved number only means something if the ruler didn't move.
5. **Keep wins, iterate the weak tier, then verify hard.** If the cohort beats the last, keep
   the edit; if not, revert and try another lever. Iterate on the weakest model until the
   result is satisfactory, *then* commission a hard, full-volume verification across tiers to
   confirm the gain is real and see how it scales upward.

## Two gears — search cheap, verify hard

Spend measurement in two distinct gears; `/benchmark-skill` owns the exact per-cell volumes,
but the loop decides which gear to be in:

- **Search — n ≈ 1–2, iterate fast.** Here you are hunting for leverage, so iterate the
  *scenarios* as much as the skill: you want a regime where the contrast is stark — the bare
  agent clearly struggles and the skill clearly wins — because that is where an edit has room
  to show. A cheap run only *indicates* a direction, and that is all you need to choose the
  next edit. Going up the tiers, a couple of runs on a stronger model indicate before you
  commit to it.
- **Verify — n ≈ 10, once a version looks promising.** Ramp the volume up (and add tiers) to
  confirm the gain is real and not noise. This is the gear that earns the claim "it's
  better."

The corollary is `/benchmark-skill`'s, and it saves the most waste: **if no small sample
ever shows the skill clearly winning, a bigger sample won't help** — the leverage isn't
there, and the fix is a starker scenario or a better edit, not more runs.

## Invariants

- **Reuse the ruler.** Once you leave search and start comparing revisions, the harness is
  frozen: every iteration re-runs the *same* committed scenarios, prompts, and pinned models.
  Comparing edits on a drifting ruler tells you nothing. (Iterating *scenarios* belongs to
  the search gear, before the ruler is fixed — never between the cohorts you compare.)
- **Weak model first.** Tune on the small tier through the loop — the weakest model flails
  hardest, so it has the most room to improve and the clearest signal. Spend mid and large
  only once the small tier has converged, to see how the tuned skill scales upward. A
  small-only pass is a partial tuning cohort, not a full refresh: leave an existing study's
  mid and large cells in place until the final full-tier run.
- **Cohorts stay separate.** Every iteration is a fresh `skill_rev`; the report compares
  cohorts and never averages across them. A gain is one cohort beating another on the same
  ruler, not a blended number.
- **Change the skill, never the ruler.** The only thing you edit to move a metric is the
  skill under test. Touching the harness, scenarios, or prompts to chase a number voids every
  comparison — how a benchmark runs is `/benchmark-skill`'s domain, not a lever in this loop.
