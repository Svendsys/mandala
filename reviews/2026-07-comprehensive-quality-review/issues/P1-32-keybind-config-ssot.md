# P1-32: KeybindConfig — every bindable Action declared in 3+ hand-synced places; the drift class has already shipped; shipped template uses a renamed key that is silently ignored

**Severity:** P1 (user-facing config surface; proven drift mode) · **Area:** mandala/keybinds + config assets

## Problem A — field/Default/resolve triple with no completeness enforcement

Adding one bindable Action requires: a `KeybindConfig` struct field (`config.rs:41-288`), a `Default` entry (:290-505), and a `resolve()` table row or `push_parametric` call (:517-921). A field missing from `resolve()` **compiles** and the binding is silently dead. This has shipped: `config.rs:241-243` — "Pre-fix this Action variant existed but was *unregistered* — users could not bind a key to preview-set via JSON." The default-coverage test only catches actions with non-empty defaults; the ~28 parametric fields default to `vec![]` and are invisible to it. The classifier side is already compiler-enforced via `mandala_derive::ActionClassify` — the config side has no equivalent.

## Problem B — shipped template uses a dead key

`config/default_keybinds.json:8` uses `cancel_mode` — the schema renamed it to `exit_mode` (rename recorded in `work_plans/SECTIONS_BORDERS_RESIZE_PLAN.md:2384`). `KeybindConfig` has no `deny_unknown_fields`, so unknown keys vanish silently: a user editing the shipped template's entry gets no effect and no warning.

## Fix plan

1. **Structural fix (preferred):** extend the `ActionClassify` derive (or a sibling derive/macro) with `#[action(config_field = "undo")]`, generating the struct fields, Default entries, and resolve-table rows from the enum — one source of truth, compiler-enforced completeness. `lib/mandala_derive` already demonstrates the pattern for the three classifiers.
2. **Minimum fix:** an exhaustiveness test — serialize `KeybindConfig::default()`, stuff a sentinel binding into **every** field via JSON manipulation, deserialize, `resolve()`, and assert every `ActionKind` (via `ActionKind::iter()`) is covered by the pairs table ∪ an explicit parametric/gesture allowlist.
3. Fix `config/default_keybinds.json` (`cancel_mode` → `exit_mode`).
4. Warn on unrecognized top-level keys at keybind load (serde `unknown_fields` capture or a Value-pass) so future renames degrade loudly.

## Acceptance criteria

- A new Action variant without config wiring fails to compile (option 1) or fails a test (option 2).
- Editing the shipped template's exit binding works; unknown keys produce a `log::warn!`.
- `./test.sh` green (keybinds tests are extensive — extend, don't weaken).

## Pointers

`src/application/keybinds/{config.rs,action/mod.rs,resolved.rs,tests.rs}`; `lib/mandala_derive/src/lib.rs` (the compile-enforcement exemplar); `config/default_keybinds.json`; CODE_CONVENTIONS §5.
