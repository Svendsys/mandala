---
name: benchmark-skill
description: >-
  Measure whether a skill actually pays off — design and run a controlled
  with/without benchmark of a target skill across models, then write the result
  up under `skill-analysis/<skill>/` as a study of one frozen skill revision. Use
  when asked to "benchmark the X skill", "is this skill worth it", "measure /
  evaluate a skill". On a repeat invocation it re-runs the *same* benchmark
  already on disk rather than redesigning one — so cohorts stay comparable. Not
  for the improve-a-skill loop that edits and re-measures (that is
  `/optimize-skill`), and not for benchmarking a website or generic code
  performance; this measures a *skill's* effect on agent behaviour.
---

# Benchmark a Skill

A benchmark answers one question about a skill: **does it pay off, and where?**
The deliverable is a self-contained study saved under
`skill-analysis/<skill-name>/` — a report grounded in per-agent data, not vibes.
**Saving the study is part of running the benchmark, not an afterthought:** every
run persists its report, data, evidence, and harness under
`skill-analysis/<skill-name>/`, so the result is durable and the next run can build on
it. **If the repo you're working in has no `skill-analysis/` directory yet, create it**
— it is the home for these studies, so set it up before saving the first one. The shape
to match is
[`skill-analysis/TEMPLATE/`](../../skill-analysis/TEMPLATE/) — read it before
designing anything, and reason by archetype (section 1): a permission/license
skill, a capability/environment skill, a QA/review skill, and a routine/workflow
skill each lead with a different metric, so pick the one closest to your target
and let it shape what you measure.

A single benchmark studies **one frozen revision of the skill** (its `skill_rev`) — it
tells you what that revision is worth, not how to make it better. Improving a skill by
editing it and re-measuring is a distinct operation, `/optimize-skill`, which drives this
one as its instrument.

The engine is **a controlled matrix**: the same task run by isolated agents that
differ in exactly one variable — skill access — measured across model strength.
Everything below serves making that comparison clean and reproducible.

What follows is guidance to reason with, not a checklist to execute. Every point
serves one end — a comparison you can trust — so when a case isn't covered, reason
from that end rather than hunting for a rule. Judgement governs; the structure is
here to inform it, not replace it.

## 0. Reuse an existing ruler before you redesign

**Always look at `skill-analysis/<skill>/` first.**

- **A study and harness already exist** → this is a re-run. **Do not redesign.** Re-run
  the *exact same* harness against the current state of the skill, append the new numbers
  as a fresh cohort, and regenerate the report. Comparable cohorts require an identical
  ruler; changing scenarios or prompts between runs voids the comparison.
- **A report exists but there is no committed `harness/`.** Reconstruct from the report
  rather than redesigning — but a report *summarises* its method; it is not an exact spec
  (the precise prompts, fixtures, and flags aren't there). So build the most faithful
  harness you can, commit it as the canonical ruler going forward, and treat the old
  report's numbers as reference rather than claiming the first reconstructed run reproduces
  them exactly. Later re-runs, sharing the committed harness, compare cleanly.
- **Nothing exists for this skill** → design a new study (sections 1–6) and commit
  the harness alongside the report so the *next* invocation can simply re-run it.

The harness is therefore a committed artifact, not scratch: persist the runner,
fixtures, and exact prompts under `skill-analysis/<skill>/harness/` so any later
run — by anyone, in any session — reproduces the same benchmark byte-for-byte.

## 1. Assess the skill — what is it actually trying to move?

Read the target `SKILL.md` end to end and decide what *failure* and *success* look
like for it. The metric is whatever the skill exists to change; there is no formula,
so reason about its nature first:

| Skill nature | What it exists to change | Lead metric(s) | Cost's role |
| --- | --- | --- | --- |
| **QA / review / safety** | catch what a bare agent misses | correctness, recall (issues caught), false-positive rate | secondary — a subtle bug found early saves more than tokens |
| **Routine / workflow** | do a frequent task right and cheap | correctness **and** efficiency (output tokens, turns/attempts) | co-equal — it runs often |
| **Permission / license** (e.g. a deploy skill) | act unattended where a bare agent stops to ask | autonomy (ran without human help), correctness | secondary |
| **Capability / environment** (e.g. a screenshot skill) | know the gotchas a DIY attempt rediscovers | correctness, then efficiency (output tokens, turns — weak models flail) | strong signal at the weak end |

**Efficiency means token cost and attempts, never dollars.** Report **output tokens**
(the volume the skill produces) and **turns/attempts** (agentic iterations, plus any
`needed_human_help`). Exact USD has no meaning here — prices drift and differ by model,
so a dollar figure is not a stable ruler and is never a headline number. (Spending tokens
generously *while running the benchmark* is fine — benchmarks run rarely, and thoroughness
now saves work later; that is separate from token cost as a *metric of the skill*.)

State the chosen lead metric(s) explicitly; everything downstream is built to measure
them.

## 2. Design the scenarios — address the essence

Build **three to five scenarios spanning a spectrum from easy/obvious to
subtle/complex.** The easy end shows the skill's floor (often "no help needed"); the
hard end is where a skill earns its keep. Each scenario must have a single,
mechanically-checkable **correct end-state** — the thing `data.csv` records as right or
wrong.

Aim at the *essence*: the one counter-intuitive thing the skill encodes that a bare
agent gets wrong (for a deploy-style permission skill, "answer a rejected push with
force, never a pull"). The hardest scenario should be the one that isolates exactly
that.

## 3. The three arms

Every scenario is run under three arms; the **only** variable is skill access. The
task text is identical across arms (a neutral request that never hints at internals);
the *told* arm adds one sentence naming the skill.

| Arm | Setup | Question it answers |
| --- | --- | --- |
| **No skill** | skill mechanism disabled — pure DIY | what a bare agent does |
| **Skill (discovered)** | skill present, prompt never names it | does the agent surface it from a plain request? |
| **Skill (told)** | skill present, prompt names it | the skill's ceiling when invoked |

The discovered arm doubles as a test of the skill's `description` — if agents don't
reach for it on a plain request, the trigger wording is the defect to fix.

## 4. The human-help lifeline

A bare agent often stalls and asks for input — especially in the no-skill arm. Mirror
a supervised run: when an agent stalls without completing, resume it **once** with a
single canned reply granting the *minimal* hint or permission needed to proceed (e.g.
"you have permission to overwrite dev"). Record `needed_human_help = 1` for that run —
**it is a headline metric, not an aside.** Driving that column to zero is usually a
core part of what the skill is for, so the report must call out every run that needed
the lifeline and whether the skill removed the need.

## Interactive skills — a simulated counterpart

Section 4's lifeline is one canned reply for a stall. Some skills differ in kind: their
whole value **is** a multi-turn exchange with a human — an interview, a negotiation, a
teaching loop. A headless run has no human, so you supply one: a **second agent playing
the counterpart, driven from a hidden ground truth.**

- **The ground truth is the answer key.** Write the full set of decisions or facts the
  counterpart holds; the task text hands the agent only the vague version. The gap between
  them is exactly what the skill must extract.
- **The counterpart answers only what's asked** — truthfully, tersely, volunteering
  nothing. If it dumps the whole spec unprompted, the skill's value (asking the right
  things) is bypassed and every arm scores alike.
- **The correct end-state (section 2) becomes recall against the key:** of the N
  load-bearing items, how many the exchange surfaced, plus any the agent invented or got
  wrong. That is mechanically checkable — grade the final artifact against the key with a
  separate grader run, not by eye.
- **The counterpart is fixture, not variable.** Same ground truth, same answering
  discipline across every arm; only skill access changes. A drifting counterpart voids the
  comparison as surely as a drifting prompt would.

## 5. Models and volume

Run across **three model tiers**, named by capability and cost rather than by vendor so
the benchmark survives model churn. Pick a concrete model for each tier from whatever
agent the harness drives, and pin its full ID there (section 6):

- **small** — the cheap, fast, weakest tier (a mini / economy model).
- **mid** — the mid-capability, mid-cost tier.
- **large** — the strongest, most expensive tier (a flagship / frontier model).

Unless the human specifies otherwise:

- **The small tier is the anchor: n = 10 runs per cell** (cell = model × scenario × arm).
  Weak models flail hardest, so they carry the clearest signal and the tightest sample.
- **mid and large are a cross-check: n = 3 per cell.** They show how the skill's
  usefulness scales as models get stronger — often correctness saturates while autonomy,
  output tokens, and turns still move.

**Indication is not verification.** The n above are *verification* numbers — how much
volume it takes to trust a cell. A 1–2 run result only *indicates* a direction; it takes
~10 to *verify* an effect and size it. So don't read a small sample as a settled result,
and don't pay for a full cell before a cheap indication says there's something there — the
same going up the tiers, where a couple of runs on a stronger model indicate before you
commit. The corollary saves the most waste: **if no small sample ever shows the skill
clearly winning, adding runs just measures noise more precisely** — the fix is the
*scenario* (too easy, no room to win — see § 2), not the sample size. A benchmark where
the bare agent already scores ~1.0 has nothing to measure.

## 6. The harness — isolated, hermetic, reproducible runs

Each agent is an isolated headless run in its own fresh working copy of the repo,
launched by **a headless agent runner**: a non-interactive CLI or SDK that drives the
model unattended. The runner is the experiment's substrate, not an incidental detail —
it is what actually delivers the controls below, and **none of them can be reproduced by
a written-down parameter alone**. Pick one runner, drive every cell of the matrix with
it, and **confirm its exact invocation against the installed tool** — flag and option
spellings differ between agents and drift between versions, so the committed harness
(section 0), not this prose, is the source of truth for them.

The harness earns the "one variable" claim (section 3) only if every run is also
**hermetic** and **bounded** — and that is a property of the runner, not the prompt.
Concretely, the runner must give every run, identically:

- **Hermetic — nothing leaks in from the host.** A fresh checkout is not enough: a runner
  left on its defaults discovers ambient instruction files (e.g. `AGENTS.md`),
  hooks, *other* skills, plugins, MCP servers, and memory from whatever machine runs the
  harness, so two operators get different context from the same script. Launch each agent
  from a **clean context** — the runner's no-config/isolated launch mode, or an equivalent
  isolated `HOME` and explicit settings path — then add back **only** what the arm under
  test needs. (This is also why an orchestrator that fans out *in-session* subagents
  cannot be the substrate: those subagents inherit its context and are not hermetic.)
- **Skill access is the only knob.** No-skill arm: the target skill is absent and the
  skill mechanism disabled. Skill arms: expose **only** the target skill, no others. The
  task text is byte-identical across arms; the *told* arm's one extra sentence is the
  sole textual difference.
- **Identical permission posture across all arms.** Every arm runs with the *same*
  non-interactive permission mode and allow-list inside its sandbox. Otherwise a
  no-skill run stalls on a tool-approval prompt while a skill arm — pre-approved by the
  skill or local settings — sails through, and `needed_human_help` ends up measuring
  approval friction instead of the skill's domain behaviour. Permission parity is what
  keeps section 4 honest.
- **Bounded — every run is capped, identically.** Give each run the same turn cap and
  external wall-clock timeout; a coarse spend cap is a fine runaway *backstop* but is not
  a metric. The runner must *enforce* these — a documented limit doesn't stop a looping
  prompt or hung skill from blocking the matrix forever. A run that hits any cap is
  recorded as a stall/failure (`capped` in the data), never silently dropped.
- **Liveness — tell "working" from "hung", don't assume silence is progress.** A long
  fan-out has three states that look alike from outside: *working*, *throttled-but-
  recovering*, and *hung*. Use the per-unit result you already persist to disk as a
  heartbeat. Healthy: new results keep landing and the failed/empty count stays near
  zero. Throttled: results slow but empties stay low — retries are absorbing it; lower
  concurrency or add a cooldown. Hung: no new result for longer than one unit's worst
  case (turn-cap × per-call timeout), or the empty count climbs (retries exhausted, so
  the matrix is now rotting — stop and fix before it fills with phantom zeros). Check
  freshness, the empty count, and that the runner process is alive — a quiet run is not
  a finished one.
- **Pin the models.** A tier alias — the friendly `small`/`mid`/`large`-style name a
  runner exposes for "the current model of this tier" — resolves to *the latest* such
  model and drifts over time, so a later rerun would silently compare a new model against
  old rows. Pass **full, pinned model IDs** in the committed harness, and record the
  resolved model id from each run's own output.
- **Contain side effects.** If the skill mutates git/remote/filesystem state, give each
  agent its own throwaway origin / sandbox so nothing reaches anything real.
- **Stub what isn't under test.** A slow external gate the skill merely *invokes* (a
  test suite, a deploy) is stubbed to pass instantly so the measurement isolates the
  behaviour being studied — and the report's caveats say so, since stubbing
  *undercounts* a skill whose value includes that step.
- **Capture the full transcript, from the source.** The runner must emit a
  **machine-readable, turn-by-turn transcript**; persist each run's complete stream to a
  per-run log, since a final-result-only summary cannot substantiate the verbatim command
  and message excerpts `evidence.md` needs. Read `turns`, `input_tokens`, `output_tokens`,
  and the resolved model from each run's own structured usage fields — never estimate, and
  do not record dollars. Tag every run uniquely.

Commit the runner, fixtures, exact prompt files, and the pinned invocation under
`skill-analysis/<skill>/harness/` (section 0).

## 7. Write it up

Produce, under `skill-analysis/<skill-name>/`:

- **`README.md`** — the report. Match `skill-analysis/TEMPLATE/README.md`:
  1. **Bottom line first** — a bold one-paragraph verdict ("Yes, on every model and
     metric" / "Yes, on the case that matters").
  2. **Scenario table** — situation → correct end-state.
  3. **Results tables** — the small-tier grid (per arm, then split by scenario), then an
     *across tiers* table. Lead with the metric chosen in section 1.
  4. **What the data says** — a few tight, evidenced bullets, each citing numbers.
  5. **Why the no-skill agents struggle** — the mechanism, not just the gap.
  6. **Evidence** — link `evidence.md`; for visual skills embed cropped PNGs
     (with-skill-correct vs the no-skill failure modes).
  7. **Method** and **Caveats** (sample sizes, what was stubbed, what is undercounted),
     and a `_Generated <date>._` line. Use today's date from context; do not invent.
- **`data.csv`** — one row per agent. Columns:
  `skill_rev,model,model_id,scenario,arm,rep,<outcome…>,used_skill,needed_human_help,capped,method,turns,input_tokens,output_tokens,tag`
  where `<outcome…>` is **one or more** skill-specific result columns — a single
  correctness bit where a scenario has one correct end-state (e.g. `deployed_correct`,
  `correct_capture`), but **separate count columns when the metric needs them**: a
  QA/review skill with several seeded issues per scenario records e.g.
  `issues_found,issues_seeded,false_positives` so the report can compute recall and
  false-positive rate (section 1) — collapsing that to one bit would destroy those metrics.
  `skill_rev` is the target skill's commit SHA (or loop iteration) the row was measured
  against — what separates rerun cohorts; `model` is the tier (`small`/`mid`/`large`) and
  `model_id` is the resolved pinned model it ran as; `capped` flags a run that hit a
  turn/time/budget limit. **No dollar column** —
  efficiency is tokens and turns (section 1). Keep the leading and trailing columns
  consistent across studies so they stay comparable.
- **`evidence.md`** — verbatim transcript excerpts (the agents' own commands and final
  messages) contrasting how each arm handled the hardest scenario. This is the proof a
  reader clicks through to.

On a **re-run**, add the new cohort under a fresh `skill_rev` rather than blending it into
the old numbers — the report's job is to show how the cohorts differ, not to average
across skill revisions. If the existing `data.csv` predates these
columns, reconcile the headers before mixing rows (or keep the old file as reference and
start fresh); just don't leave one file in two shapes.

## Invariants

- **One variable, hermetically.** Across arms only skill access changes — same task
  text, fixtures, pinned models, volume, *and the same non-interactive permission
  posture*. A second changed variable, or host context (instruction files, other skills,
  MCP, memory) leaking in, voids the comparison.
- **Reproducible and bounded.** The committed harness — pinned model IDs, capped
  turns/budget/time, fixed flags — reproduces the study; a re-run uses it unchanged.
  Cohorts compare only on an identical ruler.
- **Cohorts stay separate.** Every row carries the `skill_rev` it was measured against;
  a re-run appends a new cohort and the report compares cohorts, never averages across
  them.
- **Metric fits the skill.** Lead with what the skill exists to move (section 1); do
  not default to token cost for a QA or permission skill.
- **`needed_human_help` is first-class.** Every lifeline use is recorded and reported —
  and it only means something because every arm shares one permission posture.
- **Results are saved.** The study (report, `data.csv`, `evidence.md`, `harness/`) is
  written under `skill-analysis/<skill>/` as part of the run — a benchmark whose numbers
  aren't persisted there hasn't finished.
- **Numbers come from the runs.** Token counts and turns are read from each run's own
  output, and outcomes from a mechanical check of the end-state — never estimated, and
  never reported in dollars.
