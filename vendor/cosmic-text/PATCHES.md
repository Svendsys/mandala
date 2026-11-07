# cosmic-text 0.19.0, as Mandala patches it

This directory is [cosmic-text](https://github.com/pop-os/cosmic-text)
0.19.0 as it is published on crates.io: its normalized `Cargo.toml`,
`src/`, `README.md` and its two license files. It is not Mandala's code.
It keeps its own license, **MIT OR Apache-2.0** (`LICENSE-MIT`,
`LICENSE-APACHE`). The repository's MPL-2.0 header script
(`scripts/add_license_headers.sh`) skips `vendor/` for that reason.

The root manifest's `[patch.crates-io]` points every
`cosmic-text = "0.19.0"` in the build here: baumhard's and glyphon's, so
there is one copy of its types. `fuzz/Cargo.toml` carries the same entry,
and the wasm32 probe (`lib/baumhard/tests/wasm_font_lock.rs`) writes its
entry from the root's. `baumhard::util::manifests` holds every patch to a
rule: the version the members ask for, from a path inside the repository,
in every workspace that builds baumhard.

It is patched because of V1-B14 (#304). One 262 KB span took 596 s to
project (#243). The cost was cosmic-text's font fallback, and 0.19.0 is
its latest release.

**Provenance.** The published crate is `cosmic-text-0.19.0.crate`, with
sha256 `be17b688510d934ce13f48a2beba700e11583e281e0fda99c22bb256a14eda73`:
the checksum the lockfile recorded for it from crates.io until this patch
replaced the registry source with a path, which has none.
`scripts/check_vendored_cosmic_text.sh` fetches the crate, checks that
sha256, applies `mandala.patch`, and compares the result with this
directory file for file. It then runs cosmic-text's own test suite, which
is not vendored, against the patched `src/`. CI runs it as the
`vendored-cosmic-text` job (`.github/workflows/test.yml`). It needs the
network, so `./test.sh` does not run it.

## What differs from 0.19.0

All of it is in **`mandala.patch`**, a unified diff against the published
crate that applies with `patch -p1` in a copy of it. That this directory
is exactly the published crate plus the patch, and nothing else, is held
by the provenance script above, in CI. Offline, in `./test.sh`,
`util::manifests::tests::test_the_vendored_patch_is_the_one_listed`
holds less, since the published crate is not in the repository. It
checks that every line the patch adds is in its file, in order; that
every file the patch changes is marked `MANDALA PATCH` and listed below;
that no marked file is left out of the patch; and that the license files
are here. It does not see a line the patch removes still being present,
or an unmarked edit to a file the patch does not name. Every changed
place in `src/` is marked `MANDALA PATCH`, and `src/shape.rs`'s module
docs list the changes too.

| File | Change | Why |
|---|---|---|
| `Cargo.toml` | The test and bench targets, their dev-dependencies and `[profile.test]` are removed, and this header is added. | Their files are not vendored. A profile in a non-root package is ignored with a warning. |
| `src/shape.rs` | **Linear fallback merge** (`FallbackMerge`, chosen per round by `merge_round`). It makes the published merge's choices in one walk of the fallback's glyphs and one of the run's. A round whose fallback glyphs are not monotone, or whose run glyphs are not monotone and forward (`well_formed`), is merged by the published algorithm (`reference_round`). | The published merge scans the missing list for each fallback glyph and calls `Vec::remove`/`insert` for each cluster. That is O(run × missing) per fallback face, quadratic in a word the first face lacks. |
| `src/shape.rs` | **A code point no face draws is found once** (`only_undrawn`, `drawn_by_no_face`, the shape buffer's `undrawn`). The loop stops when every missing glyph is a one-code-point cluster that no face draws, asked once per code point, direction and weight. | The published loop shapes the whole run in every face of the font system at every shaping of a run holding such a code point (the 35 ms per `.notdef` of #304). |
| `src/shape.rs` | **`ShapeLine::with_sizes_of`**: a shaping with each glyph's metrics read from other attributes. | So that a rescale lays out without shaping (`BufferLine::adopt_shape`). |
| `src/shape.rs` | **Work counters, per thread** (`ShapeWork`, `shape_work`), and **`set_reference_merge`**, which selects the published merge on a thread. | Tests show cost by counting, never by timing, and hold the patch to the published merge glyph for glyph. |
| `src/shape.rs` | **`mandala_test_hooks`** (`#[doc(hidden)]`): one merge round, and the early stop's question, on glyphs a test builds. | Some guards decide cases no shaping makes, so only a built fixture can show a test needs them. |
| `src/buffer_line.rs` | **`BufferLine::adopt_shape`**: take another line's shaping, sizes rewritten, when the text and shaping strategy match. | `Fit` and R27 rescale whole runs at each probe size. Shaping is in units of the em, so only layout needs to run again. |
| `src/font/fallback/mod.rs` | `FontFallbackIter::font_system`. | `drawn_by_no_face` asks the font system between two steps of the iterator. |
| `src/font/system.rs` | `db_mut` also clears the shape buffer's `undrawn`. | Which faces draw a code point can change with the database. |
| `src/font/system.rs` | **`FontSystem::instance`**: a number no other font system made in the process has, given with its database. | A face id means a face only within one database; baumhard's outline cache keys by this beside the id, so no entry answers another font system, or a value assigned in one's place. |

## What Mandala relies on

The patch adds public API to cosmic-text, and baumhard uses it. Dropping
the patch for an upstream release therefore means replacing these, not
only deleting this directory:

- **`shape_work()` and `ShapeWork`** (with `ShapeWork::since`): the
  per-thread work counters. Every counted-work test of the shaper, the
  flows and the projection reads them.
- **`set_reference_merge`**: selects the published merge on a thread.
  The oracle and the controls use it.
- **`BufferLine::adopt_shape`** (and the crate-internal
  `ShapeLine::with_sizes_of` behind it): `flow::shape::resized_buffer`,
  a rescale with no shaping.
- **`FontSystem::instance`**: the outline cache's key
  (`fonts::cached_outline_bounds`).
- **`mandala_test_hooks::{fallback_round, stops_early}`**
  (`#[doc(hidden)]`), and `ShapeGlyph`'s public fields that build their
  fixtures: the tests of the merge's and the early stop's guards.

## Upgrading cosmic-text

Drop the patch if upstream releases an equivalent fix. Upstream `main`
has rewritten the merge, but that version is not released and not
identical in edge cases, so the oracle in
`baumhard::font::tests::shape_work_tests` would have to be run against it.

Otherwise:

1. Replace this directory with the new release, as published.
2. Apply `mandala.patch` (`patch -p1`) and resolve what does not apply.
3. Regenerate `mandala.patch` against the new release.
4. Update the version and sha256 in
   `scripts/check_vendored_cosmic_text.sh` and above, and run it: it
   checks the new directory against the new release plus the patch, and
   runs upstream's own tests against the patched sources.
5. Run `./test.sh`. The oracle holds the patched merge to
   `set_reference_merge`'s published one.
