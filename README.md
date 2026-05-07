# Mandala

Mandala is a Rust mindmap application built on
[wgpu](https://wgpu.rs/) and
[cosmic-text](https://github.com/pop-os/cosmic-text), using the
**Baumhard** glyph-animation library under
[`lib/baumhard/`](lib/baumhard/). It runs on both native desktop and
as a WebAssembly build. `.mindmap.json` files load and render as
interactive canvases where every visual element — text, borders,
connection paths — is laid out as positioned font glyphs.

## Quickstart

```sh
./test.sh                              # full test suite + wasm32 type-check
./build.sh                             # release build (native + wasm)
./run.sh maps/testament.mindmap.json   # native + trunk serve in parallel
```

For one-off iteration:

```sh
cargo run -- maps/testament.mindmap.json   # native only
trunk serve                                 # WASM only (loads via ?map=…)
```

## Where to read next

| Document                                        | What it covers                                                            |
| ----------------------------------------------- | ------------------------------------------------------------------------- |
| [`CLAUDE.md`](CLAUDE.md)                        | Project orientation; the canonical entry point for new contributors       |
| [`CONCEPTS.md`](CONCEPTS.md)                    | Conceptual building-blocks (`GlyphArea`, `MutatorTree`, `Channel`, ...)   |
| [`CODE_CONVENTIONS.md`](CODE_CONVENTIONS.md)    | Workspace-wide coding conventions and philosophy (mandatory)              |
| [`lib/baumhard/CONVENTIONS.md`](lib/baumhard/CONVENTIONS.md) | Crate-local rules for Baumhard (mutation-not-rebuild, arena, ...)        |
| [`TEST_CONVENTIONS.md`](TEST_CONVENTIONS.md)    | Testing philosophy + the `do_*()` benchmark-reuse pattern                 |
| [`format/`](format/)                            | The `.mindmap.json` format spec; start with `format/schema.md`            |

## Repository layout

- [`src/application/`](src/application/) — the app shell (event loop,
  document state, rendering pipeline, input handling).
- [`lib/baumhard/`](lib/baumhard/) — data model, loaders, scene
  builders, and the tree bridge. Most interesting logic lives here.
- [`crates/maptool/`](crates/maptool/) — CLI for working with
  `.mindmap.json` files: `show`, `grep`, `apply`, `export`,
  `convert --legacy`, `verify`.
- [`lib/mandala_derive/`](lib/mandala_derive/) — proc-macro support.

## License

MPL-2.0 — see per-file SPDX identifiers.
