# P1-20: `color <axis>=<named-var> section=K` writes `var(--var(--accent))` into the model; three hex validators disagree; var() recognition tripled

**Severity:** P1 (persisted model corruption from a console verb) · **Area:** mandala/console + baumhard color

## Problem A — double-wrapped var reference (persisted corruption)

`src/application/console/commands/color.rs:370-374` (`apply_section_colours`, the `section=K` path):

```rust
let resolved = match color_value {
    ColorValue::Hex(h) => h,
    ColorValue::Var(name) => format!("var(--{})", name),   // BUG
    ColorValue::Reset => "#ffffff".to_string(),
};
```

`ColorValue::Var` already carries the FULL model string: `ColorValue::parse("accent")` → `Var(VAR_ACCENT)` where `VAR_ACCENT = "var(--accent)"` (`traits/color_value.rs:44-51`, `constants.rs:18-20`). The format produces `"var(--var(--accent))"`, written into the section's `TextRun.color` and **saved**. `resolve_var` can't resolve it, so the theme reference silently degrades to the renderer fallback. Every other call site uses `as_model_string()` correctly; the sibling trait-dispatch path (Section selection without `section=`) is correct and tested — the bug hides in exactly the untested cell (`section=` tests use only hex values).

**Fix:** `ColorValue::Var(name) => name.to_string()` (or route through `as_model_string()`); regression test `color text=accent section=1` asserting `run.color == "var(--accent)"`. Consider a small model-load warning for `var(--var(` patterns to surface already-corrupted saves.

## Problem B — three hex-validity grammars, mutually inconsistent

- `ColorValue::parse` accepts `#rgb/#rgba/#rrggbb/#rrggbbaa` (`color_value.rs:37-43`).
- `is_valid_color_literal` (paste path) accepts ONLY 6|8 digits + `var(--name)` (`traits/view.rs:598-606`).
- Baumhard's canonical `hex_to_rgba` accepts 3/4/6/8 with optional `#` (`util/color_conversion.rs`, `font/hex.rs:28-30`).

Live inconsistency: `color bg=#abc` succeeds and writes `"#abc"`; copying that color and pasting it onto an edge → `Outcome::Invalid("not a color: #abc")`. §1: color is Baumhard's — two app-side validators re-implement (and disagree with) what baumhard already defines.

**Fix:** expose `pub fn is_valid_hex_color(&str) -> bool` in `baumhard::util::color_conversion` (delegating to `hex_to_rgba(...).is_some()`); use it from both `ColorValue::parse` and `is_valid_color_literal`; one accepted-length policy everywhere.

## Problem C — `var(--name)` recognition exists in three grammars

Canonical `resolve_var` trims and accepts `var( --bg )` (`color_conversion.rs:50-60`, pinned by test); `view.rs:602` does `strip_prefix("var(--")` (no whitespace tolerance — rejects values the renderer resolves); `custom/sync.rs:72,307` do `starts_with("var(")`.

**Fix:** add `is_var_ref(&str) -> bool` + `parse_var_name(&str) -> Option<&str>` beside `resolve_var`; route view.rs and sync.rs through them.

## Acceptance criteria

- `color text=accent section=1` (and `fg`/`edge`, with and without `range=`) persists `var(--accent)` exactly; regression tests included.
- Copy→paste round-trips for `#abc`-form colors succeed.
- One hex-validity and one var-recognition implementation repo-wide (grep-clean).
- `./test.sh` green.

## Pointers

`src/application/console/commands/color.rs:345-400`; `src/application/console/traits/{color_value.rs,view.rs}`; `src/application/document/custom/sync.rs:72,307`; `lib/baumhard/src/util/color_conversion.rs`; CODE_CONVENTIONS §1, §5.
