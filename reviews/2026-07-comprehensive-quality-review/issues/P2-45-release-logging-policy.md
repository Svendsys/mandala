# P2-45: `release_max_level_off` silently deletes the §9 "log and degrade" posture from every build users run — decide and document

**Severity:** P2 (policy contradiction; production undiagnosability) · **Area:** workspace policy · **Found independently by two reviewers**

## Problem

Both crates set `log = { ..., features = ["release_max_level_off"] }` (root Cargo.toml:32; baumhard Cargo.toml:16), which compiles out **all** log levels — including `error!` and `warn!` — from release builds. `./build.sh`, `./run.sh`, and the WASM bundle (`trunk build --release`) all ship release binaries.

CODE_CONVENTIONS §9's contract is: "Degrade the frame, log via `log::warn!`/`log::error!`, keep running." The second half is compiled out exactly where the first half matters. Every degrade path the codebase carefully built becomes a *silent* degrade in production: unknown-font drops, failed UTF-8 conversions, clipboard failures, macro privilege rejections, mutation no-op warnings, corrupt-region drops. User reports of "my font silently changed" or "paste did nothing" become undiagnosable. The freeze watchdog already routes around the policy with raw `eprintln!` — evidence the policy bites.

This may well be a deliberate mobile-perf choice — but it is documented nowhere, and it materially weakens §9's observable intent.

## Fix plan (decision issue — two acceptable outcomes)

**Option A (recommended):** switch both crates to `release_max_level_warn` — `error!`/`warn!` survive release; the chatty `debug!`/`trace!` walker instrumentation still vanishes. Cost: the format-args branches for warn/error sites (negligible; they're off the hot path by §9's own design).

**Option B:** keep `off`, and document it: a paragraph in CODE_CONVENTIONS §9 stating that release builds are silent by design, that warns are debug-build tooling, and what the supported diagnosis story for production issues is (e.g. "run the debug build" / a future `--verbose` flag). Update the §9 wording so the contract matches shipped reality.

Either way: normalize the three coexisting log-prefix idioms ("area: message" vs bare vs fn-name) on the way past — one `<area>: message` convention.

## Acceptance criteria

- One documented logging policy that matches the shipped binaries.
- If Option A: verify release binary size/perf delta is acceptable (quote numbers); warns visible in a release run's stderr / browser console.
- `./test.sh` green.

## Pointers

`Cargo.toml:32`, `lib/baumhard/Cargo.toml:16`; CODE_CONVENTIONS §9; `src/application/app/freeze_watchdog.rs:142-158` (the existing workaround).
