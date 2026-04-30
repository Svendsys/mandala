# CLAUDE.md
§1 When launching sub-agents for investigation or reviews, always use the most powerful agent you have available, 
not whatever is the default. Opus or if available Mythos

§2 NEVER skip changes because they are "merely cosmetic". 

§3 When proposing multiple options, if any of those options strays from the original task then make that absolutely clear

§4 NEVER use "Not introduced by me" as excuse. No one cares, just address it.

"API error: Stream idle timeout - partial response received" is an error that occurs regularly these days. 
To avoid it, please make sure that any large files such as (but not limited to) plan files are written in 
smaller pieces first, and then finally combined into the full file.

## What this is

Mandala is a Rust mindmap application built on wgpu and cosmic-text, using
the Baumhard glyph-animation library under `lib/baumhard`. It runs on both
native desktop and as a WebAssembly build. `.mindmap.json` files are loaded
and rendered as interactive canvases where every visual element — text,
borders, connection paths — is laid out as positioned font glyphs.

## Important references

- **`CONCEPTS.md`** — the conceptual building-blocks reference: what
  each named concept (`GlyphArea`, `MutatorTree`, `Channel`, `Portal`,
  `ZoomVisibility`, `CustomMutation`, ...) is, what problem it solves,
  and where it lives. Start here when a term is unfamiliar or for a
  top-down orientation across both crates.
- **`CODE_CONVENTIONS.md`** — the workspace-wide coding conventions and
  philosophy. Mandatory read.
- **`lib/baumhard/CONVENTIONS.md`** — crate-local rules for baumhard:
  mutation-not-rebuild, grapheme-aware text, arena discipline,
  benchmark-reuse, no-unsafe policy, and performance rules. Read this
  before touching anything under `lib/baumhard/`.
- **`TEST_CONVENTIONS.md`** — testing philosophy, where to put tests, the
  `do_*()` benchmark-reuse pattern, and what we deliberately don't do
  (no mocks, no snapshots, no GPU tests).
- **`format/`** — the `.mindmap.json` format specification.
  `format/schema.md` is the primary reference; per-concept docs cover
  Dewey-decimal IDs, named enums, palettes, channels, text runs,
  validation invariants, portal labels, mutations, and migration from
  legacy. Read this before changing the data model.
- **`crates/maptool/`** — CLI tool for working with `.mindmap.json`
  files: `show`, `grep`, `apply`, `export`, `convert --legacy`
  (migration from miMind-derived format), and `verify` (structural
  validation).
- **`lib/baumhard/src/mindmap/`** — the data model, loaders, scene
  builders, and the tree bridge. Most interesting logic lives here.
- **`src/application/`** — the app shell: event loop, document state,
  rendering pipeline, and input handling.

## Common tasks

- **Run tests**: `./test.sh` runs the full suite across both crates,
  prints a test count, then type-checks `wasm32-unknown-unknown` so
  cross-platform drift fails the run. Flags: `--coverage` (runs under
  `cargo-llvm-cov`, outputs `target/llvm-cov/html/index.html`),
  `--lint` (advisory `cargo fmt --check` + `cargo clippy`), `--bench`
  (runs the criterion benches after tests).
- **Build releases**: `./build.sh` cleans prior output and builds both
  the native binary (`target/release/mandala`) and the WASM bundle
  (`dist/` via `trunk build --release`). `--debug` builds dev profile
  on both sides; `--fat` switches native to `release-lto`. Requires
  `trunk` on `PATH` and the `wasm32-unknown-unknown` target installed.
- **Run the app**: `./run.sh [map.mindmap.json]` launches the release
  binary and `trunk serve --release` in parallel; Ctrl+C stops both.
  For one-off iteration use `cargo run -- maps/testament.mindmap.json`
  (native) or `trunk serve` (WASM) directly.
- **Target a specific test**: `cargo test -p baumhard --lib <pattern>` or
  `cargo test -p mandala --lib <pattern>`.
- **Load a different mindmap**: the first positional CLI arg is the path
  to a `.mindmap.json` file; WASM reads it from the `?map=` query param.
