# P1-18: build.rs font scan — nondeterministic AppFont enum order, new fonts don't trigger rebuilds, UTF-16 name tables panic the build, generated docs unprofessional

**Severity:** P1 (non-reproducible builds + broken documented workflow) · **Area:** baumhard/build.rs · **Verified:** empirically (two generated files from this machine differ in variant order)

## Problems

1. **Nondeterministic enum order**: `collect_fonts` accumulates into `std::collections::HashMap` and emits `fonts_map.into_iter()` order (`lib/baumhard/build.rs:74,118-134`) — randomized per process. Verified: two `generated_fonts_data.rs` files in target/ list completely different variant orders. Consequences: non-reproducible builds; `AppFont` variant order/discriminants differ per binary (serde uses names, so saves survive — but any future ordinal use, `FONT_DATA` layout, and generated-file diffs churn arbitrarily).

2. **New fonts don't appear**: the script emits `cargo:rerun-if-changed` only per font file found (:131). Once any rerun-if-changed is emitted, cargo tracks only those paths — dropping a **new** .ttf into `src/font/fonts/` does not rerun the script, contradicting CONCEPTS ("drop a font file in, recompile, and the variant appears").

3. **UTF-16BE name records panic the build**: `get_font_name` does `std::str::from_utf8(name.name).expect("Not UTF-8")` (:193-215). Windows-platform name records are UTF-16BE; ASCII names survive by accident (interleaved NULs), but any non-ASCII character (e.g. `é` = `0x00 0xE9`) yields invalid UTF-8 → build panic. The graceful `fallback_sanitize` path is unreachable for exactly the fonts that need it.

4. **Collision/robustness nits**: dedup keys on lowercased filename prefix — two files with different prefixes but identical camel-cased internal names would emit `enum AppFont { X, X }` (confusing generated-code build error); extension checks are case-sensitive (`Foo.TTF` skipped); `fallback_sanitize(&filename)` operates on the extension-bearing name (numbers-only font → invalid variant `.ttf`).

5. **Generated docs**: `pub enum AppFont` carries no `///` at all; the only variant doc emitted is for `Any`: *"Indicates that the defining party does not give two fucks about the font used"* (`build.rs:13-14` DOC_ANY_STR) — public rustdoc in a crate intended for standalone extraction (§B9 "every pub item carries a doc comment"; also just not shippable wording).

## Fix plan

1. `fonts.sort_by(|a, b| a.0.cmp(&b.0))` before generation (one line closes determinism).
2. Emit `cargo:rerun-if-changed` for the font **directories** (walk and emit per-directory lines) so additions/removals invalidate.
3. Use ttf-parser's UTF-16-aware `name.to_string()`; fall back to `fallback_sanitize` on `None`. Strip the extension before fallback sanitizing. Compare extensions case-insensitively.
4. Dedup on the final variant name: numeric-suffix or skip-with-warning on collision; equal-preference collisions pick lexicographically smallest path.
5. Emit a real `///` for the enum and per-variant docs (family name + source file); replace DOC_ANY_STR with professional wording.

## Acceptance criteria

- Two consecutive clean builds produce byte-identical `generated_fonts_data.rs`.
- Adding/removing a font file triggers regeneration (manual check documented in the PR).
- A font with a non-ASCII name table builds via the fallback path (add such a fixture or a unit test on the name-extraction fn).
- `cargo doc -p baumhard --no-deps` shows documented AppFont.

## Pointers

`lib/baumhard/build.rs`; CONCEPTS §2 (Font system); CONVENTIONS §B9, §B0.
