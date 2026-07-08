---
name: benchmark-runner
description: >-
  Drive the Claude Code CLI as the hermetic, headless runner for a skill
  benchmark — the concrete Claude implementation of benchmark-skill's abstract
  "headless agent runner". Use when building or running a benchmark harness (see
  the benchmark-skill) on Claude Code: the exact `claude -p` invocation, how to
  isolate a run, how to expose or withhold the skill under test, how to read
  tokens/turns, and the reliability traps that silently corrupt results.
---
# Benchmark Runner (Claude Code)

`benchmark-skill` specifies *what* the runner must deliver — hermetic, bounded,
one-variable, structured transcript. This is how to get all of it from the
`claude` CLI. Confirm flags against the installed version (`claude --help`); they
drift.

## The call

One hermetic, non-interactive run:

    claude -p "$PROMPT" \
      --model "$MODEL" \
      --output-format json \
      --setting-sources '' \
      --disallowedTools '*' \
      --append-system-prompt "$SYSTEM"

- **`-p` + `--output-format json`** — non-interactive; emits one JSON object with
  `result` (the text), `usage.output_tokens` / `usage.input_tokens`, `num_turns`,
  `total_cost_usd`, and `modelUsage`. **Trap:** `modelUsage` is a dict of *every* model
  the call touched, and a background utility model (haiku) is often listed **first** — so
  `next(iter(modelUsage))` records that background id, not the model you requested. Pick
  the entry whose id matches the model you asked for (or persist the whole dict). Recording
  the first key silently mislabels every row; it stamped haiku's id across this study's own
  tier data. Record the resolved id, not the alias — `haiku` maps to a dated id that drifts.
- **`--setting-sources ''`** — load no user/project/local settings. Combined with
  running in an **empty working directory**, this is what makes the run hermetic:
  no ambient `CLAUDE.md`, skills, hooks, or MCP leak in. Without it the run
  discovers the host's config and two operators get different results.
- **`--disallowedTools '*'`** — no tool use, for a pure text/dialogue task. Drop it
  when the skill genuinely needs tools, but then hold the tool set identical across
  arms.
- **Auth just works** — the CLI uses the session's own credentials; no
  `ANTHROPIC_API_KEY` needed. (Setting one forces API-key auth instead.)

## The one variable: skill access

- **No-skill arm** — a neutral `--append-system-prompt` describing the task, with
  no mention of the skill.
- **Skill (told)** — same system prompt **plus the skill's `SKILL.md` body**
  appended (strip its YAML frontmatter). The task text stays byte-identical to the
  no-skill arm; the skill body is the only difference.
- **Skill (discovered)** — drop the skill dir under `.claude/skills/<name>/` in the
  run's working directory and *don't* name it; this tests whether the description
  fires. (Discovery needs a real settings source, so relax `--setting-sources`
  accordingly and add back nothing else.)

## Interactive skills — multi-turn

`claude -p` is one-shot. For an interview/negotiation, drive turns from a harness:
each turn is a fresh `claude -p` call whose prompt carries the **whole transcript so
far** (the call is stateless — you re-send context every turn). Play the human with
a second `claude -p` call driven from a hidden ground truth (see benchmark-skill,
"Interactive skills"). Cap the turns and force a final answer at the cap.

## Reliability — these silently corrupt results

Learned the hard way; ignore them and your grid fills with phantom findings:

- **An empty `result` is a failure, not an answer.** Under rate pressure the CLI
  returns success with an empty `result`. Treat empty/whitespace as a retryable
  error with backoff — otherwise a rate-limited interview turns into an empty
  artifact scored as a real zero.
- **Grade in a separate pass, never inside the interview fan-out.** A grader model
  competing with concurrent interview calls for the same rate limit starves and
  returns empty scores — indistinguishable from a genuine zero. Run all interviews
  first, persist each transcript+output to disk, then grade the saved artifacts in
  a second low-concurrency pass with retries.
- **Keep concurrency modest** (a handful) and cool down between large cohorts.
  Sustained high concurrency is what triggers the empty-result failures above.
- **Persist every run's transcript as it completes**, namespaced by skill revision.
  It's the durable store a timeout or a re-grade builds on, and re-runs of the same
  revision must not overwrite each other.
- **Enforce a per-call wall-clock timeout, and never run the interview loop in the
  foreground.** A single wedged `claude -p` otherwise hangs a whole interview; wrap
  each call in a timeout (e.g. `subprocess.run(timeout=…)`) and treat a hit as a
  retryable failure. A foreground `timeout <shell>` around a *whole* multi-turn run
  kills it mid-interview and loses the transcript — run the matrix in the background
  and bound each call, not the batch.
- **A liveness check must not grep for its own command line.** `pgrep -f "skill-rev
  v2a"` (or `pkill -f …`) matches the waiting/killing shell itself, because that string
  is in its own argv — the waiter deadlocks or the kill suicides. Wait on a *result*
  condition (new files appeared, a count reached, a done-marker written), not on a
  process-name match of a string you also typed. And when hunting a wedged call, the
  agent's own long-lived session is itself a `claude` process — filter it out (it's the
  `--replay-user-messages` launcher, not a `-p` run) before concluding anything is stuck.
