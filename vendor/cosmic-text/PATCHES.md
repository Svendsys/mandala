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

## What differs from 0.19.0

All of it is in **`mandala.patch`**, a unified diff against the published
crate that applies with `patch -p1` in a copy of it, and is checked
against this directory by
`util::manifests::tests::test_the_vendored_patch_is_the_one_listed`.
Every changed place in `src/` is marked `MANDALA PATCH`, and
`src/shape.rs`'s module docs list the changes too.

| File | Change | Why |
|---|---|---|
| `Cargo.toml` | The test and bench targets, their dev-dependencies and `[profile.test]` are removed, and this header is added. | Their files are not vendored. A profile in a non-root package is ignored with a warning. |
| `src/shape.rs` | **Linear fallback merge** (`FallbackMerge`). It makes the published merge's choices in one walk of the fallback's glyphs and one of the run's. A fallback whose cluster starts are not monotone is merged by the published algorithm (`reference_round`). | The published merge scans the missing list for each fallback glyph and calls `Vec::remove`/`insert` for each cluster. That is O(run × missing) per fallback face, quadratic in a word the first face lacks. |
| `src/shape.rs` | **A code point no face draws is found once** (`only_undrawn`, `drawn_by_no_face`, the shape buffer's `undrawn`). The loop stops when every missing glyph is a one-code-point cluster that no face draws, asked once per code point, direction and weight. | The published loop shapes the whole run in every face of the font system at every shaping of a run holding such a code point (the 35 ms per `.notdef` of #304). |
| `src/shape.rs` | **`ShapeLine::with_sizes_of`**: a shaping with each glyph's metrics read from other attributes. | So that a rescale lays out without shaping (`BufferLine::adopt_shape`). |
| `src/shape.rs` | **Work counters, per thread** (`ShapeWork`, `shape_work`), and **`set_reference_merge`**, which selects the published merge on a thread. | Tests show cost by counting, never by timing, and hold the patch to the published merge glyph for glyph. |
| `src/buffer_line.rs` | **`BufferLine::adopt_shape`**: take another line's shaping, sizes rewritten, when the text and shaping strategy match. | `Fit` and R27 rescale whole runs at each probe size. Shaping is in units of the em, so only layout needs to run again. |
| `src/font/fallback/mod.rs` | `FontFallbackIter::font_system`. | `drawn_by_no_face` asks the font system between two steps of the iterator. |
| `src/font/system.rs` | `db_mut` also clears the shape buffer's `undrawn`. | Which faces draw a code point can change with the database. |

## Upgrading cosmic-text

Drop the patch if upstream releases an equivalent fix. Upstream `main`
has rewritten the merge, but that version is not released and not
identical in edge cases, so the oracle in
`baumhard::font::tests::shape_work_tests` would have to be run against it.

Otherwise:

1. Replace this directory with the new release, as published.
2. Apply `mandala.patch` (`patch -p1`) and resolve what does not apply.
3. Regenerate `mandala.patch` against the new release.
4. Run `./test.sh`. The oracle holds the patched merge to
   `set_reference_merge`'s published one.
