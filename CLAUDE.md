# CLAUDE.md
§1 NEVER skip changes because they are "merely cosmetic". 

§2 When proposing multiple options, if any of those options strays from the original task then make that absolutely clear

§3 NEVER use "Not introduced by me" as excuse. No one cares, just address it.

§4 NEVER defer the hard parts until later, and then proceed to ship a "good enough" or "approximate" now unless specifically instructed to. The hard parts are the work, there is nothing else.

§5 Use American English for consistency, not British English

§6 NEVER run benchmarks — no `cargo bench`, no `./bench.sh`, no `./test.sh --bench`. Run the tests.
`./test.sh` is the gate. Changing benchmark code is fine (§B3 requires a bench entry alongside a new
primitive); executing it is not — `cargo check --workspace --benches` proves the target still builds.
Say `--workspace`, and say it every time: without it cargo selects only the root package, which
declares `autobenches = false` and owns no bench target, so the bare form exits 0 on a bench file that
does not compile. `test_no_bench_check_selects_a_package_that_owns_no_bench` holds the flag to that
spelling in every Markdown file, shell script, CI workflow and Rust comment here — `test.sh`, where
the command is run rather than described, included. Make no performance claims: §B7 wants a
main-against-main control row you will not have, and control runs on identical code swing ±10–25% at
p=0.00 on this hardware, so an uncontrolled number is indistinguishable from noise. State work
removed as a structural fact visible in the diff, never as a measured win. See `AGENTS.md`, which
carries this same rule for agents on other harnesses.

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
  `do_*()` benchmark-reuse pattern, what we deliberately don't do
  (no mocks, no snapshots, no pixel assertions, no test that
  *requires* a hardware GPU), and §T13's checklist of assertions
  that cannot fail on their subject, with the negative control
  every passing assertion owes.
- **`format/`** — the file format specifications. `format/schema.md` is
  the `.mandala.json` format (V1) and its primary reference;
  `format/legacy-schema.md` is the `.mindmap.json` format the app still
  reads until V1-C3 deletes it. Per-concept docs cover
  Dewey-decimal IDs, named enums, palettes, channels, text runs,
  validation invariants, portal labels, mutations, and migration from
  legacy. Read this before changing the data model.
- **`crates/maptool/`** — CLI tool for working with `.mindmap.json`
  files: `show`, `grep`, `apply`, `export`, `convert` (`--legacy`
  migrates the miMind-derived format; `--portals` and `--sections`
  migrate pre-refactor maps that are otherwise current), and `verify`
  (structural and numeric validation).
- **`crates/mandalactl/`** — CLI client for a *running* app's `--ipc`
  socket: one invocation, one command, JSON or `path: value` lines on
  stdout, exit code `0` / `1` (the app said no) / `2` (could not reach
  it, nothing ran) / `3` (bad command line) / `4` (sent, not confirmed:
  it may have run). The sibling of `maptool`, not a mode of
  it: `maptool` reads files and never talks to a process, `mandalactl`
  talks to a process and never reads a map. (Its one client-side
  command, `record assemble <dir>`, reads a *recording's* frames and
  manifest, never a map and never the socket: it turns the frames of
  `capture.record_start` / `capture.record_stop` into a GIF, or an MP4
  through the host's `ffmpeg`.) `mandalactl --help` is the
  agent-facing reference; `verbs::VERBS` is its command table, and the
  app crate's tests hold it to the IPC registry in both directions, so
  **a new IPC command ships its `mandalactl` verb in the same PR**.
- **`lib/baumhard/src/mindmap/`** — the data model, loaders, scene
  builders, and the tree bridge. Most interesting logic lives here.
- **`src/application/`** — the app shell: event loop, document state,
  rendering pipeline, and input handling.

## Dual-target status

Native desktop and the browser build are first-class peers
(CODE_CONVENTIONS §4). This section is the live registry of
sanctioned native-only carve-outs; each entry names the reason and
the parity trajectory (or why none is owed):

- **IPC: the native transports** (`--ipc`,
  `src/application/ipc/transport/`; IPC-02/#62, #169) — native-only by
  nature: a browser cannot serve a local socket or pipe. Within native
  the transport is a Unix socket, or on Windows a named pipe
  (`\\.\pipe\mandala-<name>`, `ipc/transport/pipe.rs`; #169) behind the
  same boundary threads. The pipe is owner-only, except that an
  elevated (run-as-administrator) launch makes the Administrators
  group its owner, so any other elevated process of a member of that
  group can connect too. The Windows half runs on CI's `windows-ipc`
  job (`.github/workflows/test.yml`: clippy, the `ipc::` tests and
  `mandalactl`'s whole test target), not in `./test.sh`, which cannot
  run it from Linux; locally
  `cargo check --target x86_64-pc-windows-gnu -p mandala` type-checks
  it. **The protocol is not gated, since IPC-15 (#75)**: the
  envelope, the registry, every command family and the event stream
  (`src/application/ipc/`, `app/ipc_target.rs` on native,
  `app/run_wasm/ipc_target.rs` on the browser) compile and run on
  both targets — as does the document revision the event stream
  observes (`document/revision.rs`), a counter on the model — and the
  browser has a transport of its own: a WebSocket *toward* a loopback
  relay, `mandalactl bridge`, opened when the page is loaded with
  `?ipc=<the relay's app URL>` (`ipc/browser/`, `run_wasm/ipc.rs`;
  `format/ipc.md` §Transport "Browser"). What the browser does not
  serve, `describe` reports per command (`supported`,
  `unsupported_reason`) and a request for it is answered
  `unsupported`: `act.console` (the console shell is native, below)
  and the two `capture.record_*` commands (no recorder, no files);
  `act.action` refuses a `NativeOnly` `Action` the same way. The
  client, `crates/mandalactl`, compiles on every target (the
  workspace is type-checked for wasm32), speaks the socket, the pipe
  and the WebSocket (`--ipc` / `--ws`) with one command surface, and
  is the relay. Protocol: `format/ipc.md`; design:
  `work_plans/LLM_IPC.md` §D6 (as amended by IPC-15).
- **The screenshot engine, and recording** (`src/application/capture/`
  minus `geometry.rs`, `src/application/renderer/capture.rs`,
  `src/application/app/capture_flow.rs`,
  `src/application/app/capture_scene.rs`; IPC-07/#67, #178) — the offscreen
  engine behind native's `capture.screenshot` and the `screenshot`
  console verb (its own camera, its own size, a file on disk), and
  with it frame-sequence recording (`capture/record.rs`,
  `app/record_flow.rs`; IPC-08/#68: `capture.record_start` /
  `capture.record_stop` and the `record` console verb), is
  native-only: the readback blocks the main thread on a GPU buffer
  map, which a browser cannot do (wgpu's `poll` is a no-op on WebGPU,
  and the map resolves only when the event loop yields), and a
  recording is files. The frame it draws is not gated:
  `Renderer::prepare_frame` / `encode_frame` are the browser's render
  path too — including the composed canvas a capture at another zoom
  reads (`CanvasSource::Composed`, `renderer/scene_shape_cache.rs`),
  which compiles for both targets and is simply never asked for by
  the window. **`capture.screenshot` itself is served on both
  targets since IPC-15 (#75)**: the browser draws the live view into
  its canvas and reads it back in the same task (`to_data_url`,
  `run_wasm/ipc_target.rs::capture_canvas`) — whether that read holds
  the frame just drawn is established on WebGL and unverified on
  WebGPU, which the bundle takes whenever the browser's WebGPU offers
  an adapter (a browser whose WebGPU offers none falls back to WebGL2
  since #224; `format/ipc.md` §`capture.screenshot`) — returning the
  PNG inline
  (`png_base64`) with the geometry sidecar — `capture/geometry.rs`,
  one body for both — inline beside it, which `mandalactl --ws`
  writes to the path given. What the browser's capture lacks is the
  engine's framing (`width`/`height`/`region`/`fit`/`scale`): it is
  the canvas as drawn, at its own size, and `describe` declares
  exactly the parameters each target takes. Recording stays native;
  `describe` says so. What any browser frame shows is limited by #228
  (open): the browser has no Latin text face for the default family,
  so edge labels draw as `.notdef` boxes and connection dots vanish, on
  every backend.
- **Headless and one-shot launches** (`--headless`, `--size`,
  `--adapter`, `--screenshot`; `src/application/app/run_headless.rs`,
  `app/pump.rs`, `app/shell.rs`, `renderer/adapter.rs`; IPC-10/#70) —
  native-only, and no parity is owed: they are command-line launches
  of a process with no display, and a browser page always has its
  canvas — running a browser without a display is the browser's own
  job. What they are made of is not gated where it need not be: the
  adapter plan (`renderer/adapter.rs`) runs in every bootstrap, the
  browser's included, and the frame a headless app draws is the
  `prepare_frame` half every target draws. The headless driver runs
  the loop body winit's `about_to_wait` runs (`app/pump.rs`), so the
  native carve-outs below — the console shell, the per-frame drain,
  virtual-clock stepping, capture — reach it unchanged.
  Protocol: `format/ipc.md` §"Headless and one-shot launches"; design:
  `work_plans/LLM_IPC.md` §D10.
- **Console modal shell** (`src/application/app/console_input/`) —
  the modal shell and its exec path are native-gated; the
  verb/parser layer (`src/application/console/`) is
  cross-platform. Over IPC this is `act.console`, which the browser
  reports unsupported (`describe`) and refuses by name. Parity is
  the named next step in CONCEPTS §6 "Console", not yet scheduled.
- **FreezeWatchdog** (`src/application/app/freeze_watchdog.rs`) —
  native-only by design: browsers ship their own unresponsive-tab
  dialog; no parity owed.
- **`MAX_MAP_BYTES` and `MAX_ELEMENTS`**
  (`lib/baumhard/src/document/format/mod.rs`) — `None` on native;
  on wasm32 a two-axis memory budget, because memory is the only
  limit and a map's cost grows both with its text and with the
  elements (and definitions) it becomes, and a short text can
  become many elements. A map's peak while it loads, and while it
  is then saved, is modeled as `text × 7 + elements × 576` bytes
  (`WASM32_BYTES_PER_TEXT_BYTE`, `WASM32_BYTES_PER_ELEMENT`), and
  each limit takes half of a 3 GiB budget: **230,087,533 bytes of
  text** (~219 MiB, refused before parsing) and **2,796,202
  elements** (counted as the builder appends them, the one past it
  refused with `TooManyElements`). The two costs are measured, not
  chosen: the densest shape of each kind of content (empty
  sections, empty rings, portal and line links, empty spans,
  top-level nodes, one node holding every row, a long text, a large
  theme or palette, the most verbs), at its worst point of container
  growth, by the wasm32
  module's own memory — which counts `dlmalloc`'s headers and
  rounding, `realloc` copies and freed chunks it could not reuse —
  in the probe (`lib/baumhard/tests/wasm_font_lock.rs`), which
  fails if a pin is stale; `lib/baumhard/tests/format_memory.rs`
  holds the same maps to the model natively. The cost per element is
  now essentially three arena slots at the arena's doubling point
  (the old buffer and the new one while it is copied): a smaller
  `DocElement` or a non-contiguous arena are the only levers left,
  and both are outside the format. `save` holds one node's
  row at a time beside the document — a second copy only to name a
  refusal about ids, parents or links, which only a defect makes —
  and refuses on wasm32 what that target's load would refuse, the
  element limit included, without that copy. What is not bounded: an
  edit that grows a document past the element limit (its save is
  refused, the edit is not), and what the app builds from the
  document (the scene), which the 1 GiB left of the 4 GiB address
  space is for. The old `.mindmap.json` loader keeps
  its own 3 GiB `MAX_MAP_BYTES` (`mindmap/loader.rs`) until C3
  retires it. Divergent by platform physics rather than policy:
  wasm32's `usize` is 32 bits and its address space is 4 GiB,
  shared with the runtime, the GPU buffers and the browser. Native
  opens maps as large as the machine can hold, which is the product
  intent; the browser cannot, and says so with a message instead of
  trapping. No parity is owed — the trajectory is the reverse, a
  64-bit wasm target, which is not this project's to schedule.
- **`MAX_UNKNOWN_KEYS`**
  (`lib/baumhard/src/mindmap/unknown_keys.rs`) — `None` on
  native, `Some(2_000_000)` on wasm32, for the same reason and
  with the same trajectory as the old loader's `MAX_MAP_BYTES`
  (`mindmap/loader.rs`, 3 GiB on wasm32). It began as
  a resource ceiling standing in for a cost defect; once the
  capture and the write-back were both made linear there was
  nothing on native left for it to bound.
- **Clipboard OS layer** (`src/application/clipboard.rs`) — native
  is backed by `arboard`; WASM is stubbed pending async-clipboard
  integration (CONCEPTS §6 "Clipboard").
- **`fps` console verb** — native because the console shell is; the
  render-side FPS plumbing compiles on both targets and browsers
  expose frame timing via DevTools (CONCEPTS §5 "FPS overlay").
- **Single-line editor** (`src/application/app/single_line_edit/`)
  — edge-label and portal-caption inline editing is native-only;
  the browser has no single-line editor yet. Consequence for
  input: an edge-label double-click commits the selection on both
  targets and opens the editor only on native, surfaced as
  `DoubleClickResidual::OpenEdgeLabelEditor` rather than a `cfg`,
  and logged at `debug!` on the browser side so the stop is
  observable. Parity unblocks `Action::DoubleClickActivate` and
  `Action::EditSelection*` flipping to `Compatible`; tracked in
  `work_plans/WASM_CONVERGENCE.md`.
- **`LongPress` → `EnterResizeMode`, and a finger on a resize
  handle** (`src/application/keybinds/config.rs`,
  `src/application/app/run_wasm/event_touch.rs`,
  `src/application/app/touch_handle.rs`) — the touch route to a
  resize, and native-only end to end. **On native** a long-press
  enters `InteractionMode::Resize` on the selection, and a finger
  that then starts a gesture on one of its handles drives the
  handle drag as the left button would (`touch_handle`, #160) —
  captured, promoted to the `Throttled` resize and committed with
  its undo entry on lift — without reaching the gesture recognizer,
  so it cannot also pan. Before #160 this entry implied that route
  existed while every finger drag panned the camera; a touch-only
  user could not resize at all. `FastResizeStart`
  (`Ctrl+RightDrag`) is the mouse's accelerator for the same resize,
  by design: it has no touch binding, because this is its touch
  route. **On the browser** neither half exists. `LongPress` is the
  one touch gesture that resolves through the keybind table, and the
  one that is native-only. Its default `Action` is `NativeOnly`, so on the
  browser the dispatch returns `Unhandled` and a one-shot `warn!`
  names the remedy; the gesture is *recognized* identically on both
  targets, and only the `Action` it reaches is not. Deliberate rather
  than pending: long-press is the touch peer of the keyboard's `r`,
  so rebinding the browser's long-press to some unrelated
  `Compatible` Action would make one gesture mean two different
  things on two targets — a worse CODE_CONVENTIONS §4 outcome than
  a gesture that is honestly unavailable and says so. **The rest of the touch
  vocabulary is not affected**: tap, one-finger pan and pinch-zoom
  resolve to no `Action` at all and run on both targets through
  `dispatch::apply_touch_effect`, which is what makes the browser
  usable by finger without this entry being closed. Parity is the
  `InteractionMode::Resize` chrome plus its handle drag reaching the
  browser — the same drag-state machine **Rubber-band select** below
  is parked on, with `touch_handle`'s routing ported beside it so a
  finger can hold a handle there too — and flips this `Action` to
  `Compatible` when it lands.
- **Rubber-band select** (`DragState::SelectingRect`,
  `src/application/app/drain_frame.rs`,
  `scene_rebuild::rebuild_selection_highlight`,
  `MindMapDocument::{set,take}_rect_select_preview`) —
  native-only because `DragState` and the per-frame drain are:
  the browser has no drag-state machine and no `AboutToWait` to
  hang a per-frame hit-test off, so there is nothing for a
  Shift+drag to enter. **The read is deliberately not gated.**
  `MindMapDocument::rect_select_preview` (the method) and
  `scene_rebuild::highlight_entries_for` are cross-platform,
  because both targets' node-tree rebuilds go through the one
  mapping that answers "which nodes are tinted"; only the
  gesture that fills it is native. On wasm32 the field is
  therefore always `None`, and the tier
  `rebuild_selection_highlight` is not native *by nature* —
  whichever browser-side gesture first needs a highlight-only
  repaint takes the `cfg` off it. Parity for the gesture itself
  rides on the browser gaining a drag-state machine — a press
  needs somewhere to be pending before it can promote — and is
  not yet scheduled. What the browser lacks for it is the
  *input-driven* half of a frame — native's `heartbeat::drain_inputs`
  draining a drag state — not a frame: the browser's frame exists
  (`run_wasm/event_redraw_requested.rs`) and runs the time-driven
  half, the animation tick, since #144. (Until then a per-frame
  animation drain was listed beside this entry as a second
  per-frame mechanism the browser lacked, and this sentence
  claimed the two shared a prerequisite until #35's registry work
  read them side by side.)
- **Animation pause during a drag** (`heartbeat::drain_inputs`'s
  `anim_pause_start_ms`; `MindMapDocument::shift_active_animations_start_ms`)
  — native-only because the tree-mutating drags it is keyed to
  (`MovingNode`, `MovingSection`, `SectionResize`, `NodeResize`)
  are: the tick is skipped while one runs and every instance's
  `start_ms` is shifted past the pause on release, so an animation
  resumes where it froze instead of snapping to its end. **The
  tick itself is not gated, since #144.**
  `animation_drain::drain_animation_tick`
  (`src/application/app/animation_drain.rs`) is one body both
  targets' frames run — native's heartbeat from `about_to_wait`
  and from a clock step, the browser's from
  `WindowEvent::RedrawRequested`, which asks for the next frame
  while `animation_drain::keeps_ticking` says an animation owes
  one — so an animated `CustomMutation` (`timing.duration_ms > 0`)
  started on the browser, from the `custom_mutation_bindings`
  keystroke tier or a node's `OnClick` trigger, lerps, completes
  and pushes its undo entry as on native. The two routes land
  through one body, `app::fire_custom_mutation`, whose no-tree
  semantics are decided on #144. Parity for the pause rides with
  the browser's drag-state machine (**Rubber-band select** above):
  there is no drag to pause for until there is a drag.
- **Virtual-clock stepping: the native frame it steps**
  (`src/application/app/heartbeat.rs`; IPC-09/#69) — what a step
  runs at each heartbeat on native is native's whole frame: the
  throttled drag and picker drains, the recorder's frame, the freeze
  watchdog's tick. **The step itself is not gated, since IPC-15
  (#75)**: the schedule (`app/step.rs`: `run_step`, the `Steps`
  trait, `Beat`) is one body both targets run, the browser
  implements it over its own frame (`run_wasm/step.rs`: one injected
  sample, then the animation tick, at each virtual instant), and
  `clock.set_mode` / `clock.step` / `clock.wait` are served over the
  browser transport. Neither the clock nor the tick was ever gated:
  `AppClock` (`src/application/clock.rs`) compiles for both targets,
  the browser holds one (`WasmInputState.clock`), every browser
  reader of app time reads it, and since #144 the browser's frame
  stands still under a virtual clock the way native's loop frame
  does (`event_redraw_requested` reads `Beat::Loop`'s predicates).
  The `clock step` *console* verb is native because the console
  shell is.

## Common tasks

- **Run tests**: `./test.sh` runs `cargo test --workspace` — all five
  members, `mandala`, `baumhard`, `mandala_derive`, `maptool` and
  `mandalactl` — prints a test count, then type-checks the benchmark targets
  (`cargo check --workspace --benches`, which runs no benchmark) and
  `wasm32-unknown-unknown`, so neither a drifted `do_*()` nor
  cross-platform drift can pass a green run. **With one caveat about
  where**: the wasm32 leg is *skipped with a note* when the target is
  not installed, so a green local run on a machine that has never
  had `rustup target add wasm32-unknown-unknown` has proved nothing
  about the browser build. `rust-toolchain.toml` lists the target, so
  `rustup toolchain install` in the checkout supplies it, and CI runs
  exactly that (`.github/workflows/test.yml`), so the guarantee is
  unconditional there and conditional here. The same
  holds for `lib/baumhard/tests/wasm_font_lock.rs`, which *runs*
  baumhard's font-lock helpers on wasm32 under `node` (TEST_CONVENTIONS
  §T9): it skips with a note without the target or `node`, and CI sets
  `MANDALA_REQUIRE_WASM_PROBE` so it cannot skip there. **And for
  the GPU-backed tests** (`src/application/app/tests_gpu.rs`,
  TEST_CONVENTIONS §T8): they build the real app headless on whatever
  adapter the machine has — Mesa's lavapipe on a GPU-less box,
  `apt-get install mesa-vulkan-drivers`; an agent container needs the
  apt-over-HTTPS recipe in the `mandala-drive` skill — and skip with a
  `SKIPPED:` note without one, so a green run on a box with no adapter
  has proved nothing about the code behind `&mut Renderer`. The note
  is written past libtest's per-test capture
  (`baumhard::util::test_skip`), so it shows in a plain run and the
  summary counts the `SKIPPED:` lines beside the pass count. CI
  installs lavapipe and sets `MANDALA_REQUIRE_GPU`, so they cannot
  skip there.
  The test count itself is not a gate — a summary-format change makes it
  print "count unavailable" rather than ending the run, which is
  what it used to do, silently, under `set -euo pipefail`. Flags: `--coverage`
  (runs under `cargo-llvm-cov`, outputs
  `target/llvm-cov/html/index.html`), `--lint` (runs
  `cargo fmt --check`, `cargo clippy` and `cargo doc`. `fmt`, both
  `doc` legs and both `clippy` legs are **hard gates** — their
  baselines are zero (#130, #134 for the first two; `clippy` followed
  once its warnings were cleared), a regression prints a `FAILED:`
  line and the run exits non-zero at the end, after every other gate
  has reported. `clippy` runs with `-D warnings`, because `cargo
  clippy` exits 0 on warnings. `fmt` runs
  **once** — rustfmt parses rather than compiles, so it is
  target-independent and there is no wasm32 leg for it. `clippy` and
  `doc` run **twice**, on the host target and on
  `wasm32-unknown-unknown`: a host-target compile drops everything
  under `#[cfg(target_arch = "wasm32")]`, so without the second leg the
  browser half of the app — its lints and its intra-doc links alike —
  is invisible. The wasm32 clippy leg is `--workspace` so baumhard is
  covered too, minus `--all-targets` (criterion and rayon refuse to
  build for wasm32); the host doc leg is `--workspace` with private
  items, while the wasm32 doc leg narrows to `-p mandala`, because
  baumhard cannot be documented standalone for wasm32 — its `rand` →
  `getrandom` edge needs the `wasm_js` feature that only the root
  manifest declares — and maptool, mandalactl and mandala_derive have
  no wasm32 half to document. Both wasm32 legs are skipped with a note when the
  target isn't installed. CI runs `./test.sh --lint`, so these gates
  hold on every pull request and not only on the machine of whoever
  remembered the flag. They can be hard because the toolchain is
  pinned in `rust-toolchain.toml`: a new stable release cannot add a
  lint until the pin is bumped, and bumping it is a change of its own
  that fixes what the new lints report), `--bench` (runs the criterion
  benches after tests — **maintainers only**; §6 and `AGENTS.md`
  forbid it to agents, who need the type-check above and nothing
  else).
- **Build releases**: `./build.sh` builds both the native binary
  (`target/release/mandala`) and the WASM bundle (`dist/` via
  `trunk build --release`), replacing prior output for the chosen
  profile — each output directory is set aside and discarded only
  once its replacement exists, so a failed build leaves the artifacts
  you already had intact, at the cost of holding a second copy of the
  output tree for the duration. `--debug` builds dev profile
  on both sides; `--fat` switches native to `release-lto`;
  `--no-keep` deletes the prior output up front instead, for when the
  disk matters more than the safety net. Requires `trunk` on `PATH`
  and the `wasm32-unknown-unknown` target installed.
- **Run the app**: `./run.sh [map.mindmap.json]` launches the release
  binary and `trunk serve --release` in parallel; Ctrl+C stops both.
  It launches rather than builds, so it refuses up front — naming
  which one — when `target/release/mandala` is absent or not
  executable, when the map file does not exist, or when `trunk` is
  not on `PATH`; produce the binary with `./build.sh` first. For
  one-off iteration use `cargo run -- maps/testament.mindmap.json`
  (native) or `trunk serve` (WASM) directly; `cargo run` builds what
  it needs, which makes it the form that works from a fresh clone.
- **Dev-profile build**: `./debug_build.sh [flags]` is `./build.sh
  --debug` with every other flag forwarded — the shorthand for the
  build run most often while iterating.
- **Target a specific test**: `cargo test -p baumhard --lib <pattern>`,
  `cargo test -p mandala <pattern>`,
  `cargo test -p mandala_derive`, `cargo test -p maptool` or
  `cargo test -p mandalactl`. `--lib` narrows to a library target, so
  it belongs only to the three members that have one — baumhard,
  mandala_derive and mandalactl (a `src/lib.rs` beside its binary).
  `mandala` and `maptool` are binary crates, where adding it narrows nothing and
  exits 101 with `no library targets found in package 'mandala'`
  before a test runs (#148).
  `test_no_documented_command_targets_a_lib_the_package_lacks` holds
  this bullet and TEST_CONVENTIONS §T11's copy of it against the real
  manifests — every code span and every fenced line in those two
  sections — so the flag cannot come back to either of them on a
  member that has no library target. It reads nothing else: the rest
  of this file, and every other document, is on your own care.
- **Load a different mindmap**: the first positional CLI arg is the path
  to a `.mindmap.json` file; WASM reads it from the `?map=` query param.
- **Drive the running app over IPC** (native): launch with
  `--ipc <socket-path>` — `$XDG_RUNTIME_DIR/mandala.sock` is the
  recommended place; on Windows the value is a pipe name instead,
  `--ipc mandala` serving `\\.\pipe\mandala-mandala`, and `mandalactl`
  takes the same name — then speak newline-delimited JSON to it, e.g.
  `printf '{"id":1,"cmd":"ping"}\n' | nc -N -U $XDG_RUNTIME_DIR/mandala.sock`
  (`-N` closes the write half after the request; the server answers
  everything it received, then closes).
  `{"id":2,"cmd":"describe"}` lists the commands this build serves;
  `{"id":3,"cmd":"act.action","action":"ZoomIn"}` drives an Action,
  and `act.console` / `act.macro` run a console line or a macro;
  `format/ipc.md` is the full protocol.
  **From a shell, use `mandalactl` instead of `nc`**: `export
  MANDALA_IPC=$XDG_RUNTIME_DIR/mandala.sock`, then `mandalactl ping`,
  `mandalactl act.action ZoomIn`,
  `mandalactl scene.find_text Introduction` — no JSON to hand-quote,
  typed arguments, a timeout on every wait, and an exit code that says
  whether the app answered. `mandalactl --help` lists every command
  with an example; `mandalactl raw -` takes a request frame on stdin
  for anything its argument spelling cannot say. Exit `4` means the
  request went out and no answer was read: check the app's state
  before repeating a command that changes it. (`--wait-settled` needs
  an app that serves `clock.wait`; it is detected first, and nothing is
  sent when it is missing.)
  **With no display** — a container, CI, a cloud session — launch
  `mandala --headless --ipc <socket-path> <map>`: no X server, no
  window, an adapter chosen with no surface (Mesa's lavapipe on a
  GPU-less box), the same IPC surface, on a clock that starts virtual
  (`format/ipc.md` §"Headless and one-shot launches"). For a single
  picture, `mandala --headless --screenshot out.png <map>` loads,
  captures once settled, and exits `0`.
  **The whole loop is the `mandala-drive` skill**
  (`.claude/skills/mandala-drive/SKILL.md`): the launch recipes — a
  desktop, `--headless` in a container (Mesa's Vulkan lavapipe, and
  the packages it needs), and Xvfb for when it must be a real
  window — text-first inspection, acting with `--wait-settled`,
  screenshots read through their geometry sidecar, teardown, and a
  troubleshooting table. `scripts/ipc_smoke.sh` is its end-to-end
  check — deliberately outside `./test.sh`: it needs the built
  binaries, a socket and a process to drive, where the suite's
  GPU-backed tests build the app in-process (TEST_CONVENTIONS §T8).
  **Showing a behavior, not a state, is the `mandala-record` skill**
  (`.claude/skills/mandala-record/SKILL.md`): a captioned demo GIF
  from `scripts/record_demo.sh`, watched by its maker before it is
  posted, never committed — recordings are review artifacts.
