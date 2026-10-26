---
name: mandala-drive
description: >-
  Launch Mandala with its --ipc socket, then inspect, drive and screenshot the
  running app from the shell with mandalactl — the canonical loop for seeing
  what the app actually draws. Use when asked to open a map and look at it,
  screenshot a node or region, check a layout or rendering change before and
  after, reproduce a UI bug, or confirm a change works in the real app rather
  than only in tests. Covers the launch recipes — mandala --headless for a
  container or CI with no display (Mesa lavapipe, no X server), a desktop with
  a display, and Xvfb when it must be a real window — the one-shot
  --screenshot, the text-first inspection commands, actions and synthetic input
  with --wait-settled, screenshots with the geometry sidecar, teardown, and a
  troubleshooting table.
---
# mandala-drive

Mandala serves a local control socket when launched with `--ipc <path>`.
`mandalactl` (`crates/mandalactl`) speaks it from the shell: one invocation is
one command, `path: value` lines (or `--json`) on stdout, and an exit code that
says whether the app answered. This skill is the loop: **launch → inspect →
act → see → tear down**. Every command below was run, and worked, in this
repo's cloud container — headless, through the Xvfb recipe, and against an X
display that was already running.

**Just need one picture?** No socket, no teardown:

```sh
target/release/mandala --headless --screenshot /tmp/map.png maps/testament.mindmap.json
# exit 0: /tmp/map.png and /tmp/map.png.geometry.json, the map as loaded, 800×600
# (--size 1920x1080 for more pixels; --overwrite to replace an existing file)
```

## When to reach for this

- You changed layout, rendering, borders, text, zoom behavior or input
  handling: screenshot the affected area **before and after**, and look.
- A user reports "X looks wrong" or "clicking Y does Z": reproduce it in the
  live app instead of reasoning from source.
- You need to know where a node is drawn, what text is on screen, or what a
  click at a point would hit: `scene.*` answers it as text, with no pixels.
- Tests passed and you want proof the real app does the thing.

You do not need to read the server's source to use any of this.
`mandalactl --help` lists every command with an example, `mandalactl help
<command>` gives one command's parameters, and `mandalactl describe <command>`
asks the running app for every parameter and result field. `format/ipc.md` is
the full protocol.

## Quick start: "open testament and screenshot the area around node 3.2"

Run from the repo root. Shell state may not survive between your tool calls,
so each block below sets `MANDALA_IPC` and `PATH` again. `BIN` asks cargo
where the binaries are: `CARGO_TARGET_DIR` (or `build.target-dir`) moves them
out of `target/`, and a stale `target/release/mandala` would otherwise be
run in their place.

```sh
# 1. Build (cargo skips what is up to date).
cargo build --release -p mandala -p mandalactl

# 2. Launch, detached, with no window (a container or CI; on a desktop
#    where you want to watch, drop `--headless`).
BIN="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')/release"
export MANDALA_IPC="${XDG_RUNTIME_DIR:-/tmp/mandala-$(id -u)}/mandala.sock"
setsid nohup "$BIN/mandala" --headless maps/testament.mindmap.json --ipc "$MANDALA_IPC" \
  > /tmp/mandala-app.log 2>&1 < /dev/null &

# 3. Wait until it answers.
export PATH="$BIN:$PATH"
for i in $(seq 1 30); do mandalactl --timeout 2 ping >/dev/null 2>&1 && break; sleep 0.5; done
mandalactl ping                      # now_ms: 171.69…   (exit 0)
```

```sh
export MANDALA_IPC="${XDG_RUNTIME_DIR:-/tmp/mandala-$(id -u)}/mandala.sock"
BIN="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')/release"
export PATH="$BIN:$PATH"
# 4. Where is 3.2? (canvas units; the scene's own laid-out rect)
mandalactl scene.node_geometry 3.2 | grep -E '^(in_scene|canvas)'
#   canvas.h: 115.399994   canvas.w: 378.0   canvas.x: -25.4   canvas.y: 11697.3   in_scene: true

# 5. Frame a region around it — the rect grown by one node on each side:
#    {x: x−w, y: y−h, w: 3w, h: 3h}, rounded — and capture at
#    a matching aspect (width/height ∝ w/h) so nothing extra is added.
mandalactl capture.screenshot /tmp/around-3.2.png \
  region:='{"x":-404,"y":11581,"w":1134,"h":347}' width=1134 height=347
#   rendered: true   path: "/tmp/around-3.2.png"   sidecar_path: "/tmp/around-3.2.png.geometry.json"
```

6. Look at `/tmp/around-3.2.png` (your file-reading tool shows images). The
   sidecar says where 3.2 landed in that image:
   `{"id":"3.2","rect":{"x":378.6,"y":116.3,"w":378.0,"h":115.4},"clipped":false,…}`.

The live camera is untouched by a capture: `query.camera` reads the same
before and after.

## Launch

```sh
"$BIN/mandala" --headless <map.mindmap.json> --ipc <socket-path>   # no display; BIN as in the quick start
"$BIN/mandala" <map.mindmap.json> --ipc <socket-path>              # a window, on a desktop
cargo run --release -- maps/testament.mindmap.json --ipc "$MANDALA_IPC"   # a window, building first
```

- **Headless or a window.** `--headless` opens no window and needs no X
  server: use it in a container, in CI, in a cloud session — anywhere nobody
  watches. Drop it on a desktop when you (or the user) want to see the app
  move. An agent drives both the same way; what differs is below.
- **Size.** `--size 1280x800` sets the frame size (headless: exactly; a
  window: as asked of the window manager). Headless is 800×600 without it.
  A capture can always ask for its own `width=`/`height=`.
- **Socket path.** `$XDG_RUNTIME_DIR/mandala.sock` when that is set (it is
  `0700` on desktop distros). Containers often leave it unset; then use a
  fixed per-user path such as `/tmp/mandala-$(id -u)/mandala.sock` — the app
  creates a missing parent directory `0700`. Keep it **fixed**, not
  `mktemp`-random, so later tool calls can find it again. The server refuses
  a path whose directories another user can write, and paths over 107 bytes.
- **Tell `mandalactl` once**: `export MANDALA_IPC=<path>`, or pass `--ipc
  <path>` on each call.
- **Detach the launch** with `setsid nohup … > log 2>&1 < /dev/null &`, so
  your shell call returns and the app outlives it. The log is where startup
  panics and `warn!`/`error!` lines land.
- **Liveness**: poll `mandalactl --timeout 2 ping` until exit `0` (a release
  build answers within a second or two on llvmpipe). Exit `2` means not up
  yet, or dead — check the log.
- **Release or debug.** Release (`--release`, `target/release/…`) keeps
  `warn!`/`error!` and compiles `info!`/`debug!`/`trace!` out
  (`release_max_level_warn` in the root `Cargo.toml`); it is the fast one, use
  it by default. A dev build (`cargo build -p mandala -p mandalactl`,
  `target/debug/…`) is slower to render but honors `RUST_LOG=debug` (or a
  module filter, `RUST_LOG=mandala::application::ipc=debug`) — reach for it
  when you need the app to tell you what it is doing. Without `RUST_LOG`
  both print `warn` and above.
- **One controller at a time.** A second client is refused; `mandalactl`
  retries a refusal for 2 s, so back-to-back calls are fine.

## Environment matrix

| environment | recipe | status |
|---|---|---|
| (a) desktop with a display (X11, Wayland, macOS) | launch the binary directly; nothing else | the windowed path is validated against an already-running X server (`DISPLAY` set, `scripts/ipc_smoke.sh` passing in its window mode); not yet run on a physical desktop — run the smoke script there and update this cell |
| (b) container or CI, no display, no GPU | `target/release/mandala --headless <map> --ipc <sock>`, with the packages below | validated in this repo's cloud container (Ubuntu 24.04, Mesa 25.2): no X server, no `DISPLAY`; its captures are byte-identical to recipe (c)'s on the same machine, PNG and sidecar |
| (c) a real window with no display (Xvfb) | `xvfb-run -a -s "-screen 0 1280x800x24" target/release/mandala <map> --ipc <sock>`, with recipe (b)'s packages plus `xvfb libxkbcommon-x11-0` | validated in the same container, from a state with the packages removed. Reach for it only when the question is about the window itself — a window manager's resize, a real `RedrawRequested` — since (b) draws the same pixels |

**Packages** (Debian/Ubuntu names):

```sh
apt-get update && apt-get install -y --no-install-recommends libvulkan1 mesa-vulkan-drivers   # (b)
apt-get install -y --no-install-recommends xvfb libxkbcommon-x11-0                            # + (c)
```

- `libvulkan1` + `mesa-vulkan-drivers` — the adapter is **Vulkan on Mesa's
  lavapipe** (llvmpipe, CPU). Headless, Mesa's GL through EGL works too
  (`libegl1 libegl-mesa0`; verified with `WGPU_BACKEND=gl`, same bytes);
  under Xvfb it does not — the window's surface is refused with
  `incompatible_surface_backends: Backends(GL)` — so Vulkan is the one that
  serves both recipes.
- `libxkbcommon-x11-0` — winit's X11 backend loads it at startup and panics
  without it. Headless loads no winit backend at all, so (b) does not need it.
- **Choosing the adapter.** `--adapter software` restricts the app to a
  software (CPU) adapter — lavapipe, llvmpipe — so every runner rasterizes
  alike whatever GPU it has. wgpu's own variables are honored:
  `WGPU_BACKEND=vulkan` (or `gl`), `WGPU_POWER_PREF=low|high`,
  `WGPU_ADAPTER_NAME=<part of the name>`. A launch that can use no adapter
  exits at startup naming what each attempt hit.
- The Xvfb window is 800×600 whatever the Xvfb screen size; headless is too,
  unless `--size` says otherwise.
- `scripts/ipc_smoke.sh` checks for exactly these and prints the install
  line when one is missing.

**In an agent container: apt over HTTPS.** The cloud container's apt
sources are plain `http://`, which its outbound proxy does not carry, so
`apt-get update` fails before it finds a package. Rewrite the sources to
`https://` and hand apt the proxy and its CA bundle (the `_apt` sandbox
user cannot read a bundle under `/root`, hence the sandbox override):

```sh
sed -i 's#http://archive.ubuntu.com#https://archive.ubuntu.com#; s#http://security.ubuntu.com#https://security.ubuntu.com#' /etc/apt/sources.list.d/*.sources
APT_OPTS=(-o "Acquire::https::Proxy=$HTTPS_PROXY" -o Acquire::https::CaInfo=/root/.ccr/ca-bundle.crt -o APT::Sandbox::User=root)
apt-get "${APT_OPTS[@]}" update
apt-get "${APT_OPTS[@]}" install -y --no-install-recommends libvulkan1 mesa-vulkan-drivers   # (b); add xvfb libxkbcommon-x11-0 for (c)
```

Verified in this repo's cloud container (Ubuntu 24.04): the ICD lands at
`/usr/share/vulkan/icd.d/lvp_icd.json` and wgpu reports the adapter as
`llvmpipe (LLVM 20.1.2, 256 bits)`. The suite's GPU-backed tests
(`src/application/app/tests_gpu.rs`, TEST_CONVENTIONS §T8) run on the same
adapter and print a `SKIPPED:` note until it is there, so this recipe is
also what turns them on in a container.

**What is different headless.**

- **The clock starts virtual.** Time moves only when you step it, so a run
  is a pure function of your commands. Animations stand still until
  `mandalactl clock.step ms=<n>`; an `input.drag`'s samples are queued and
  land on the step's heartbeats — `input.drag … --wait-settled` returns
  `pending: 13`, then `mandalactl clock.step frames=20` lands them
  (`input.pending` reads `0`). `mandalactl clock.set_mode real` gives the app
  real time instead, and everything then behaves as it does in a window.
- **No human, no cursor.** Input is only what you send; `settled` is exact.
- **No window to close.** `mandalactl act.action Quit` ends it (see
  **Tear down**).

## Inspect — text first

Text queries are cheap, exact and diff-able; pixels are for "does it *look*
right". Ask in text first, screenshot when the question is visual (layout,
overlap, color, glyph rendering) or when you need evidence for a human.

| question | command |
|---|---|
| what is loaded, dirty?, counts | `mandalactl query.document` |
| what is selected | `mandalactl query.selection` |
| camera center / zoom / window size | `mandalactl query.camera` |
| interaction mode, open modals, who gets keys | `mandalactl query.mode` |
| the status line the user sees | `mandalactl query.status` |
| undo stack | `mandalactl query.undo` |
| where is node X drawn, is it on screen | `mandalactl scene.node_geometry X` |
| find drawn text (on or off screen) | `mandalactl scene.find_text "Border Toolkit"` |
| what would a click at a point hit | `mandalactl scene.hit_test X Y [space=surface]` |
| everything visible, with rects and text | `mandalactl scene.dump` (`include=all` for off-screen too) |

Real output from testament:

```text
$ mandalactl query.document
counts.edges: 258
counts.nodes: 252
dirty: false
file_path: "maps/testament.mindmap.json"
map.name: "testament"
undo_depth: 0
…
$ mandalactl scene.find_text "Border Toolkit"
matches[0].kind: "section"
matches[0].node_id: "3"
matches[0].canvas.x: 0.0
matches[0].canvas.y: 11500.0
matches[0].visibility: "offscreen"
…
$ mandalactl scene.hit_test 165 11755
click_hit.kind: "Node"
click_hit.node_id: "3.2"
…
```

**Two coordinate spaces.** `canvas` is the map's world space (node positions,
the default for `scene.*`, `act.action x/y` and capture `region`). `surface`
is window pixels, origin top-left (the default for `input.*`). Every `scene.*`
element reports both. `visibility` is `visible`, `offscreen` (a pan would
show it) or `outside_zoom_window` (its zoom gate hides it at this zoom).

## Act

Three families, from coarse to fine. Everything they do goes through the
same funnels a person's key or click reaches, so **every change is undoable
exactly like human input** (`act.action Undo`, or `input.key z mods=ctrl`),
and nothing an agent does bypasses a gate. The flip side: changes are real.
Don't `SaveDocument` (or `save`) unless asked — `dirty` stays `true` even
after an undo returns the model to where it was.

| family | use it for | example |
|---|---|---|
| `act.action <Action>` | one named `Action`, as a key binding fires it | `mandalactl act.action ZoomIn x=165 y=11755` (canvas point to zoom at) |
| `act.console "<line>"` | a console line, without opening the console | `mandalactl act.console "fps on"` |
| `input.*` | the hand on mouse and keyboard: clicks, drags, keys, typing | `mandalactl input.click_node 3.2` |

**`--wait-settled`** on a mutating command waits, on the same connection,
until the result is rendered (animations done, queued input landed, camera
rebuilt), so your next read or screenshot sees it. Use it on every mutating
call you are about to inspect. It is refused on read-only commands.

A real sequence — bring an off-screen node into view, select it, delete it,
undo. It was run headless, so time is virtual and the drag's 13 queued
samples land on a step; in a window (real time) the `--wait-settled` on the
drag waits for them itself and the `clock.step` line is not needed:

```text
$ mandalactl input.click_node 3.2
error: not_found: node '3.2' is not on screen: its center lands at surface (350.0, 645.6), outside the 800×600 surface; bring it into view first …
$ mandalactl scene.hit_test 780 580 space=surface | grep click_hit.kind
click_hit.kind: "Empty"                       # empty canvas: a drag here pans
$ mandalactl input.drag 780 580 780 240 --wait-settled
applied: false
pending: 13
wait.elapsed_ms: 0
$ mandalactl clock.step frames=20             # headless only: land the queued samples
frames_run: 20
now_ms: 321.0
$ mandalactl input.click_node 3.2 --wait-settled
applied: true
$ mandalactl query.selection
kind: "Single"
node_id: "3.2"
$ mandalactl act.action DeleteSelection --wait-settled
outcome: "handled"
$ mandalactl query.undo
entries[0].kind: "DeleteNode"
entries[0].node_ids[0]: "3.2"
$ mandalactl act.action Undo --wait-settled
outcome: "handled"
```

- **Check before you drag.** A drag that starts on a node moves the node;
  one on empty canvas pans. `scene.hit_test X Y space=surface` tells you which.
- **`input.click_node` needs the node on screen** (it clicks where a hand
  would). Pan first, as above; `act.action CenterOnSelection` re-centers once
  something is selected.
- **Typing goes to whoever owns the keyboard** (`query.mode`'s `key_owner`):
  `act.action OpenConsole`, then `mandalactl input.text 'fps on'` fills the
  console (`query.editors` shows `console.input: "fps on"`), and
  `mandalactl input.key Enter` runs it. Outside a modal, `input.key Escape`
  exits the mode and clears the selection. In the console the first Escape
  only dismisses the completion list typing opened (`query.editors`
  `console.completion_count`), and the next one closes the console — check
  `query.mode`'s `key_owner` rather than assuming one key did it.
- Action names: `ZoomIn`, `ZoomOut`, `ZoomFit`, `ZoomReset`,
  `PanCameraNorth`/`South`/`East`/`West`, `CenterOnSelection`, `SelectAll`,
  `DeselectAll`, `Undo`, `DeleteSelection`, `OpenConsole`, … — the full list
  is the `Action` enum in `src/application/keybinds/action/mod.rs`, and
  `mandalactl describe act.action` says how payload variants are spelled.
- Exit `1` is the app saying no (`error: <code>: <message>` on stderr, e.g.
  `console_error: unknown command: frobnicate`). Exit `4` means the request
  went out and no reply came back: **read state before repeating** a
  mutating command.

## See — screenshots and the geometry sidecar

`capture.screenshot` renders offscreen through the window's own
prepare/encode path — not a window grab — and writes a PNG plus
`<png>.geometry.json`. It never moves the live camera.

```sh
mandalactl capture.screenshot /tmp/now.png                     # what the window shows, window size
mandalactl capture.screenshot /tmp/big.png width=1920 height=1080 fit=true   # whole map
mandalactl capture.screenshot /tmp/r.png region:='{"x":-404,"y":11581,"w":1134,"h":347}' width=1134 height=347
mandalactl capture.screenshot /tmp/hi.png scale=2               # same view, 2× the pixels
mandalactl capture.screenshot                                   # no path: a file in the app's artifacts dir
```

- **Read the reply.** `rendered: true` means a frame was drawn and `path`
  holds it. `rendered: false` comes with `skipped`
  (`not_started`, `font_system_contended` — retry, `prepare_failed`) and
  **no file**: a skipped frame is never passed off as a black picture.
  `settled: false` means something was still moving — `--wait-settled` your
  previous command, or `mandalactl clock.wait settled` before capturing.
- **Paths**: absolute or relative to your shell (mandalactl absolutizes).
  An existing file is refused unless `overwrite=true`:
  `error: invalid_params: … exists; set overwrite to replace it`.
- **`region`** is in canvas units by default (`space=surface` for window
  pixels). The image keeps the `width`×`height` aspect (default: the window's,
  800×600 under Xvfb) by **extending** the region, never distorting it — so
  give `width`/`height` the region's aspect for a tight crop.
- **`fit=true`** frames the whole map, but refuses when that needs a zoom
  below 0.05: testament does not fit 800×600 (`invalid_params: … needs zoom
  0.0372…, below the minimum 0.05; ask for a larger width and height`), and
  fits at `width=1920 height=1080`.
- Size bounds: each side `1..=16384`, `width × height × scale² ≤ 64 Mpx`,
  `scale` in `0.1..=4.0`.

**Use the sidecar instead of guessing coordinates.** It lists every node the
image actually shows, with rects **in image pixels**:

```json
{"protocol":1,"document_revision":0,"surface":{"width":800,"height":600,"scale":1.0},
 "camera":{"center_x":1160.6841,"center_y":4843.0986,"zoom":0.05},
 "nodes":[{"id":"1.0","rect":{"x":373.7439,"y":41.888306,"w":24.060888,"h":22.31579},
           "clipped":false,"sections":[{"index":0,"rect":{…}}]}, …]}
```

- Node id → where it is in the picture: `jq '.nodes[] | select(.id=="1.0")' /tmp/now.png.geometry.json`.
- Pixel → node: the rect that contains it (or `scene.hit_test` on the live app).
- `clipped: true` — the node runs past the image edge. A node absent from
  the sidecar is not drawn in this image (off-frame, folded away, or
  outside its zoom window).
- Canvas → image: `pixel = (p − camera.center) × camera.zoom × scale + (width, height) / 2`.
- For a capture of the live view (no `region`/`fit`, window size, `scale`
  1) image pixels **are** surface pixels, so a sidecar rect's center can be
  fed straight to `input.*` or `scene.hit_test … space=surface`. Verified:
  the center of `1.0`'s rect above, `(385.8, 53.0)`, hit-tests as node `1.0`.

Look at the PNG yourself before you describe it — your file-reading tool
renders images.

## Tear down

```sh
mandalactl act.action Quit      # the graceful exit, a closed window's: exits 0, removes the socket and the artifacts dir
```

`Quit` is how a headless app ends (it has no window to close), and it works
on a windowed one too. If the app no longer answers, kill it — SIGTERM is not
a graceful exit, so clean up after it:

```sh
# The app serving $MANDALA_IPC, and nothing else: not a desktop instance the
# user is running, nor its artifacts.
pid="$(pgrep -a -x mandala | awk -v s="$MANDALA_IPC" '{ for (i = 2; i < NF; i++) if ($i == "--ipc" && $(i + 1) == s) print $1 }')"
kill -TERM "$pid"
rm -f "$MANDALA_IPC"            # optional: the next launch on this path self-heals a stale socket anyway
rm -rf "${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/mandala-$pid-artifacts"*   # path-less captures; `*` takes the -N fallback names
```

`xvfb-run` stops its Xvfb when the app exits. A one-shot (`--screenshot`)
cleans up after itself: it exits on its own, through the graceful path.

## Troubleshooting

| symptom | meaning | fix |
|---|---|---|
| app log: `mandala: --headless needs --ipc <socket> or --screenshot <path>` (panic, exit 101) | a headless launch nothing could drive or see | add `--ipc <sock>` or `--screenshot <png>`; every refused flag panics the same way, naming itself |
| app log: `renderer: no GPU adapter could be used without a window: …` (exit 101) | headless, and no adapter at all: no Vulkan ICD, no EGL | install `libvulkan1 mesa-vulkan-drivers`; the message lists what each attempt hit |
| `--screenshot` exits `1`, log: `screenshot: … exists; set overwrite to replace it` | the one-shot's write guard: the PNG or its sidecar is already there | add `--overwrite`, or pick another path |
| app log: `Library libxkbcommon-x11.so could not be loaded` (panic, exit 101) | a window (recipe (c)) in a container missing winit's X11 keyboard library | install `libxkbcommon-x11-0`, or launch `--headless`, which loads no winit backend |
| app log: `failed to create wgpu surface for window: … FailedToCreateSurfaceForAnyBackend` | a window, and no usable graphics backend at all (no Vulkan ICD, no EGL) | install `libvulkan1 mesa-vulkan-drivers` |
| app log: `renderer: no GPU adapter could present to the window: … incompatible_surface_backends: Backends(GL)` | a window under Xvfb with only GL/EGL present; GL cannot present to it | install `mesa-vulkan-drivers` (lavapipe), or launch `--headless`, where GL works |
| app log: `Could not create an EventLoop: … neither WAYLAND_DISPLAY nor WAYLAND_SOCKET nor DISPLAY is set` (exit 101) | a window was asked for and there is no display | launch `--headless`, recipe (b) — or under `xvfb-run`, recipe (c) |
| `mandalactl: … exists but nothing is listening: a stale socket` (exit 2) | the app died or was killed; its socket file remains | read the app log for why; relaunch on the same path (it self-heals) |
| `mandalactl: no socket path. Pass --ipc <path> or set MANDALA_IPC.` (exit 3) | your shell lost the export | re-export `MANDALA_IPC`, or pass `--ipc` |
| `no greeting within N s. App busy or frozen?` (exit 2) | the process exists but its loop is not answering | wait and retry once; if it stays, it is wedged — see the next row |
| app log: `!!! MANDALA FREEZE WATCHDOG !!!` (the banner `src/application/app/freeze_watchdog.rs` prints), process aborted | the main loop made no progress for 10 s, and the watchdog aborted it on purpose | a real hang: report what you sent last. An **idle** app is parked, not frozen — it answers `ping` instantly and is never aborted |
| `query.document` shows `file_path: null`, `map.name: "load-failure"` | the map failed to load; the app is up with a placeholder | the app log's `startup: could not load …` line says why |
| screenshot is all background | the frame rendered, but nothing is in it: check the sidecar — `"nodes":[]` means the framing missed the map | frame by a node's `canvas` rect, or `fit=true` with a large `width`/`height` |
| reply `rendered: false` | the frame was not drawn; no file written | `skipped` says why; `font_system_contended` → just retry |
| `input.click_node … not_found: … not on screen` | the node is off the surface | pan (drag empty canvas, see **Act**) and retry |
| `error: busy: 202 injected samples are still queued …` from `input.click_node` | an earlier `input.drag` still has samples queued | in real time, `mandalactl clock.wait settled` (or `--wait-settled` the drag), then resend; on a virtual clock — headless, by default — `mandalactl clock.step frames=<n>` lands them first |
| `clock.wait animations_complete` times out headless | the clock is virtual: animations move only when stepped | `mandalactl clock.step ms=<n>` past the animation, or `clock.set_mode real` |

## Recording a behavior

A screenshot proves a state. To show an animation, a zoom, what a click
does — something a human should *watch* — use the
[`mandala-record`](../mandala-record/SKILL.md) skill: the same launch and
inspection loop, ending in a short captioned GIF and `scripts/record_demo.sh`.

## Smoke test

`scripts/ipc_smoke.sh` is this loop as a script: preflight the environment,
build, launch — with no window when there is no display, a window when there
is, or under its own Xvfb on request — ping, check the map loaded, `clock.wait
settled`, capture the window and a region around one node, check both PNGs
(signature, size, not a flat color) and sidecars, check the capture left the
camera alone, quit it with `act.action Quit` and check it exited 0 and
removed its socket and artifacts dir, check that a one-shot `--screenshot`
launch of the map writes the same picture byte for byte, and tear everything
down.

```sh
./scripts/ipc_smoke.sh                 # PASS / FAIL: …   exit 0 / 1; 2 = environment missing (prints the install line)
./scripts/ipc_smoke.sh --headless      # no window even when there is a display
./scripts/ipc_smoke.sh --xvfb          # a real window under its own Xvfb
./scripts/ipc_smoke.sh --compare-xvfb  # also: headless and Xvfb one-shots must be byte-identical
./scripts/ipc_smoke.sh --debug         # dev-profile build
./scripts/ipc_smoke.sh --map <file> --keep
```

Its launch, preflight and teardown live in `scripts/lib/mandala_app.sh`,
which `scripts/record_demo.sh` sources too — change the recipe there.

It is deliberately **not** part of `./test.sh` (TEST_CONVENTIONS §T8/§T10: no
live GPU in the suite). Run it after changing anything on the
launch/IPC/capture path, and whenever this skill is edited.
