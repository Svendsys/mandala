# IPC

Mandala's local inter-process control protocol: a line-delimited JSON
surface over a local socket that lets an external process — an LLM
agent, `mandalactl`, a test harness — drive, inspect, screenshot, and
record the running application with the same fidelity as a human at
the keyboard.

This document is the **wire-protocol source of truth**. The design
rationale (why each decision went the way it did, what was rejected)
lives in [`work_plans/LLM_IPC.md`](../work_plans/LLM_IPC.md); read
that first if you are changing this document rather than consuming
it. The Rust-side implementation lives under `src/application/ipc/`
(module layout in [Command registry](#command-registry-and-describe)).

> **Status.** The protocol below was pinned by issue #61 (EPIC #60,
> IPC-01) and is implemented incrementally: transport by IPC-02, the
> registry and first commands by IPC-03, then one command family per
> issue (owners named per section). A family section in this document
> is the contract its owning issue implements against; the `describe`
> command must stay byte-honest against this document at every merge.
> Sections whose owning issue has not merged yet are contract, not
> yet running code.

## Transport

### Socket, flag, lifecycle

IPC is **opt-in per launch** via a CLI flag, following the existing
`--keybinds <path>` pattern in `src/main.rs`:

```sh
mandala maps/testament.mindmap.json --ipc /run/user/1000/mandala.sock
```

- `--ipc <path>` / `--ipc=<path>` — listen on a Unix domain socket
  (`SOCK_STREAM`) at `<path>`. No flag, no socket, no IPC threads —
  the interactive app pays zero cost when IPC is off.
- The listener starts before the event loop takes over (alongside
  `FreezeWatchdog::spawn()` in `run_native.rs::run`) and lives for
  the process lifetime.
- **Startup, not runtime.** Bind failures are startup errors per
  CODE_CONVENTIONS §9: fail with `expect("<reason>")`, never limp
  along with IPC silently missing. A stale socket file (bind fails,
  a probe `connect` is refused) is unlinked and re-bound; a *live*
  socket (probe connect succeeds) means another instance owns the
  path and startup fails with a message saying so.
- The socket file is unlinked on graceful exit. After an unclean
  exit the stale-socket probe above makes the next launch self-heal.

### Ownership and permissions

The socket is as security-sensitive as `~/.config/mandala/macros.json`
(see [Trust model](#trust-model)). The server creates it with
owner-only permissions (`0600`, in a directory the user owns) and
refuses to bind a path whose parent directory is world-writable
(e.g. bare `/tmp`) unless it is sticky-bit-protected and the final
path component is unpredictable enough for the user's threat model —
the recommended location is `$XDG_RUNTIME_DIR` (already `0700` on
every mainstream distro). Never a TCP port: filesystem permissions
are the access-control mechanism.

### Single controller

Exactly **one client** is served at a time.

- The first connection becomes the controller.
- While a controller is connected, additional connections receive a
  single frame `{"event":"connection_rejected","data":{"reason":"busy"}}`
  and are closed. No queueing, no arbitration — two agents dragging
  the same node concurrently is not a coherent workflow, and an
  arbitration layer would be complexity with no named consumer
  (CODE_CONVENTIONS §7).
- When the controller disconnects (EOF, error, teardown), the
  listener accepts the next connection. **No session state survives
  a disconnect**: event subscriptions reset, pending `clock.wait`s
  are canceled, undelivered frames are discarded. A reconnecting
  client starts from the `hello` greeting like any fresh client.

Attach-after-launch and re-attach are first-class: the intended
workflow is a human launching (or already running) the app and an
agent attaching later while the human watches.

### Windows

The Windows transport is a **named pipe** at
`\\.\pipe\mandala-<name>` (the `--ipc` value names the pipe), with
the pipe's security descriptor restricted to the current user —
the closest Windows analog to a `0600` socket in `$XDG_RUNTIME_DIR`.
Localhost TCP was rejected: it is reachable by every local user on
a multi-user machine (weaker than file permissions), it collides
with other services on port allocation, and it triggers firewall
consent dialogs. Everything above and below the byte stream is
identical — framing, envelope, and commands are transport-agnostic.

IPC-02 ships the Unix transport; the named-pipe adapter is a
self-contained follow-up inside the same transport module. Until it
lands, `--ipc` on a Windows build fails at startup with a message
pointing at this section (honest fail-fast per §9/§10 — no silent
no-op of a user-requested feature).

### WASM

There is no WASM transport; the `src/application/ipc/` module is
`#[cfg(not(target_arch = "wasm32"))]`-gated at the module boundary,
mirroring `console_input`. The carve-out entry lives in `CLAUDE.md`
"Dual-target status"; the browser trajectory (WebSocket transport +
browser-side capture, same envelope and commands) is parked in
IPC-15. Browsers cannot open local sockets, so this is a transport
gap by nature, not a design gap.

### Framing

- **NDJSON**: one JSON object per line, `\n`-terminated, UTF-8. A
  frame must not contain a raw newline (JSON string escapes cover
  every payload). No pretty-printing on the wire.
- Empty lines are ignored.
- **Inbound line cap: 1 MiB.** A longer line is consumed and
  discarded up to its terminating newline and answered with a
  `parse_error` reply (`"id": null`). The stream stays usable.
- **Outbound frame cap: 8 MiB.** A reply that would serialize larger
  is replaced by a `reply_too_large` error naming the file-sidecar
  alternative for that command (see
  [Large payloads](#large-payloads)).
- Malformed JSON, non-object frames, or missing/invalid envelope
  keys are answered with `parse_error` / `invalid_params` and never
  terminate the process — hostile input degrades, it does not crash
  (same posture as the loader's cycle rejection, CODE_CONVENTIONS §9).

Why NDJSON and not JSON-RPC 2.0 or length-prefixed binary: the
consumers are LLM agents and shell tools — one `socat` away from a
debuggable session, one `jq` away from parsed output. JSON-RPC's
extra ceremony (envelope-in-envelope, batch semantics, notification
rules) buys nothing the envelope below doesn't already provide, and
binary framing optimizes exactly the payloads this protocol routes
to files instead (see [Large payloads](#large-payloads)).

## Envelope

Three frame shapes exist on the wire: **requests** (client → server),
**replies** (server → client, correlated by `id`), and **events**
(server → client, unsolicited).

### Request

```jsonc
{"id": 7, "cmd": "clock.step", "ms": 100}
```

| key | type | notes |
|---|---|---|
| `id` | string or integer | Required. Client-chosen, echoed verbatim on the reply, ≤ 256 bytes serialized. The server treats it as opaque and does not deduplicate — reusing an in-flight id is the client's own confusion. |
| `cmd` | string | Required. A registered command name: `family.verb`, lowercase snake_case on both sides of the dot. Exactly two bootstrap commands are unnamespaced: `describe` and `ping`. |
| *params* | — | Command parameters sit **flat at the top level** beside `id` and `cmd`. `id` and `cmd` are reserved words no command may use as a parameter name. Unknown parameters are rejected with `invalid_params` (strictness catches agent typos early; a tolerant server would silently ignore `regoin=`). |

Flat parameters (rather than a nested `"params"` object) are a
deliberate ergonomic choice for the primary consumer — an LLM
composing frames by hand: one less nesting level to get right, and
the common commands read as a sentence.

### Reply

```jsonc
{"id": 7, "ok": true, "result": {"now_ms": 4100.0, "frames_run": 6}}
```

```jsonc
{"id": 8, "ok": false,
 "error": {"code": "unknown_cmd", "message": "no command 'clock.stepp' — closest is 'clock.step'"},
 "warnings": [{"code": "deprecated_alias", "message": "…"}]}
```

| key | type | notes |
|---|---|---|
| `id` | — | Echo of the request `id`; `null` when the request was unparseable. |
| `ok` | boolean | Required. `true` ⇒ `result` present; `false` ⇒ `error` present. |
| `result` | object | Command-specific payload. Present iff `ok:true`; `{}` when a command has nothing to say. |
| `error` | object | `{"code", "message", "data"?}`. Present iff `ok:false`. See [Errors](#errors). |
| `warnings` | array | Optional; omitted when empty. See [Warnings](#warnings--per-command-diagnostics). |
| `revision` | integer | Reserved. Once IPC-14 lands, every reply carries the document revision observed after the command executed, so agents can order replies against `document_changed` events without an extra round-trip. Absent until then. |

Replies are **not guaranteed to arrive in request order**. Most
commands reply immediately and in order, but deferred-reply commands
(`clock.wait`, and any future long-running command) reply when their
condition resolves while later requests keep executing. Correlate by
`id`, always — a client that assumes ordering will break the moment
it pipelines `clock.wait` + `capture.screenshot`, which is the
canonical agent sequence.

### Event

```jsonc
{"event": "document_changed", "data": {"revision": 42}}
```

| key | type | notes |
|---|---|---|
| `event` | string | Event class name, snake_case. |
| `data` | object | Class-specific payload; always present, `{}` allowed. |

Events are opt-in via `events.subscribe` (IPC-14), with two
exceptions that are always delivered: `hello` and `shutdown`.
Reserved classes are listed under
[`events` — event stream + revision](#events--event-stream--revision-ipc-14).

### Handshake and versioning

On accept, before reading anything, the server greets:

```jsonc
{"event": "hello", "data": {
  "protocol": 1,
  "app": "mandala",
  "version": "0.1.0",        // Cargo package version
  "pid": 31337,
  "map": "maps/testament.mindmap.json"   // null before first load
}}
```

- `protocol` is a single integer, currently **1**. Pre-V1 there is
  no compatibility machinery beyond it (CODE_CONVENTIONS §10: break
  freely, delete rather than deprecate): **any** breaking change to
  framing, envelope, or a shipped command bumps the integer, in the
  same commit that changes this document. Additive changes (new
  commands, new optional params, new result keys, new warning
  codes) do not bump it — clients must tolerate unknown keys in
  replies and events.
- Clients pin against this document, check `protocol` in `hello`,
  and refuse to drive a server they don't understand (`mandalactl`
  refuses unless `--force`). There are no version-negotiation
  frames; there is exactly one live protocol per binary.
- `describe` (below) is the runtime mirror of this document, kept
  honest by unit tests asserting the registry matches the documented
  surface; the IPC-16 sweep re-verifies before the epic closes.

On graceful shutdown (window close, `quit`), the server best-effort
sends `{"event":"shutdown","data":{}}` before closing the socket, so
agents can distinguish "app exited" from "connection died".

### Errors

Errors are **structured JSON values, never Rust error types** — the
protocol's error posture is CODE_CONVENTIONS §9 verbatim: no
`anyhow`/`thiserror`/custom enums in the implementation; interactive
paths never panic on IPC input; a malformed or hostile frame
degrades to an error reply and the app keeps running. There is no
IPC-visible distinction between "bad request" and "command ran and
failed" beyond the code itself.

- `code` — stable snake_case identifier. **Codes are API**: agents
  and `mandalactl` may branch on them; renaming one is a breaking
  change (protocol bump).
- `message` — human/LLM-readable explanation. Messages are *not*
  API: wording may change freely, and good messages say what to do
  next ("no document loaded — send `act.console` with `open <path>`").
- `data` — optional object with structured context (e.g.
  `{"elapsed_ms": 10000}` on `timeout`).

Initial code registry (families add codes in their own sections;
additions are non-breaking):

| code | meaning |
|---|---|
| `parse_error` | Frame was not a JSON object / exceeded the 1 MiB line cap. `id` is `null`. |
| `unknown_cmd` | `cmd` names nothing in the registry. |
| `invalid_params` | Missing/mistyped/unknown parameter. `data.param` names the offender. |
| `unsupported` | Command exists but not on this platform/build (e.g. future Windows gaps). |
| `busy` | Reserved for request-level contention (the connection-level form is the `connection_rejected` event). |
| `overloaded` | Request queue full (1024 pending); resend after draining replies. |
| `reply_too_large` | Serialized reply exceeded 8 MiB; `data.hint` names the file-sidecar parameter. |
| `no_document` | Command needs a loaded document and none is. |
| `not_found` | A referenced entity (node id, macro id, mutation id) does not exist. |
| `console_error` | `act.console` line parsed but the verb reported an error; `message` is the verb's own text. |
| `timeout` | Deferred command hit its deadline (`clock.wait`). |
| `internal` | The degrade-don't-crash catch-all: the command hit a state it could not honor; details in `message`. |

### Warnings — per-command diagnostics

Release builds compile out all `log` output
(`release_max_level_off`; decision gate #45), so **replies are the
diagnostic channel**: anything a command path would `log::warn!`
about, and any §9 degrade the command can observe, is *also* pushed
into the reply's `warnings` array. This is the one warning-carrying
convention for the whole surface:

```jsonc
{"id": 3, "ok": true,
 "result": {"ran": true},
 "warnings": [
   {"code": "macro_gate_rejected",
    "message": "macro 'inline-x' (source Inline): ConsoleLine step rejected; remaining steps aborted"}
 ]}
```

- `warnings` is an array of `{"code", "message"}` (same stability
  rules as error codes), omitted when empty.
- Every command handler receives a warning sink in its execution
  context; the dual-emit rule is *log and push, same site, same
  text*. Where a degrade happens below a seam that cannot report it
  upward, extending that seam to return or sink the diagnostic is
  sanctioned §2 integration work, not scope creep — implementing
  issues own the completeness of their own commands' warnings.
- Degrades *outside* any command's execution window (a font drop
  during an ordinary interactive frame) are not retroactively
  attached to replies. Streaming those is the `log_line` event
  class, which exists **only if** #45 resolves to Option A
  (`release_max_level_warn`) — parked there, deliberately not
  designed here.

### Large payloads

Two disciplines keep frames within the caps:

1. **Pixels never travel inline.** `capture.*` commands write PNG /
   frame-sequence artifacts to disk and reply with absolute paths
   (plus geometry sidecars). No base64 screenshots in JSON — an
   agent reading a 4 MB base64 blob through its context window is
   the failure mode this epic exists to avoid.
2. **Structured dumps offer a file sidecar.** Any command whose
   reply can plausibly exceed the 8 MiB cap (`scene.dump` on a
   pathological map) takes an optional `to_file: "<path>"` param;
   with it, the reply inlines only `{"path", "bytes"}`. Without it,
   an oversized reply degrades to `reply_too_large` with
   `data.hint: "to_file"`.

### Coordinate spaces

Two named spaces appear throughout the surface; commands taking or
returning geometry say which one they use, and commands accepting
either take a `"space"` parameter:

- **`surface`** — physical pixels on the rendered surface, origin
  top-left, y-down. The space of screenshots, sidecar rects, and
  synthetic mouse coordinates (default for `input.*`).
- **`canvas`** — the document's world space (the space of
  `MindNode` offsets), as used by the camera. Default for
  `capture.screenshot region=`.

Conversions go through the same camera math the renderer uses
(`Camera2D`); `query.camera` exposes the current transform so
clients can convert without a round-trip per point.

## Trust model

**IPC executes at the `User` tier. It is not a new tier.**

The reasoning, in full, is `work_plans/LLM_IPC.md` §D4; the operative
rules:

- Attaching requires the `--ipc` flag at launch: the user owns the
  flag exactly as they own `keybinds.json` and `macros.json`
  (`format/macros.md` "Privilege model"). A process that can reach
  the socket is running as the same OS user and could already edit
  those files; the socket's `0600` permissions (or the pipe's
  per-user descriptor) are the enforcement of that equivalence.
- Direct IPC dispatch — `act.action`, `act.console`, `input.*` —
  carries User-tier privileges: destructive Actions, console verbs,
  file I/O verbs all pass, same as a human at the keyboard. That is
  the epic's charter ("as well as a human"), and any weaker tier
  would contradict it.
- **Macros keep their own tier.** `act.macro` fires
  `dispatch_macro`, and the macro's loader-pinned `MacroSource`
  gates it exactly as if a keybind had fired it: a Map- or
  Inline-tier macro still cannot run `ConsoleLine` or destructive
  Actions just because IPC pulled the trigger. IPC is a User-tier
  *initiator*, never a privilege *escalator* — the fail-closed gate
  in `dispatch_macro` (`src/application/app/dispatch/macro_core.rs`)
  is untouched by this protocol.
- Provenance is a logging concern, not a privilege concern: IPC
  dispatch sites attribute their origin in diagnostics (and, once
  IPC-14 lands, in events), which covers auditability without a
  `MacroSource::Ipc` variant that would either duplicate `User` or
  cripple the feature.
- The threat model of opening a hostile `.mindmap.json` is
  **unchanged** by IPC (tiers and gates as in `format/macros.md`).
  IPC adds no remote surface: no TCP, no discovery, no
  auto-connect. The new asset to protect is the socket path itself,
  handled under [Ownership and permissions](#ownership-and-permissions).

`format/macros.md`'s SOURCE-OF-TRUTH tier list records this mapping;
anyone adding or reordering a macro tier must re-evaluate it there
and here in the same commit.

## Threading contract (wire-visible consequences)

The full architecture is CODE_CONVENTIONS §3 (amended by IPC-01) and
`work_plans/LLM_IPC.md` §D2. What a client can observe:

- Commands execute **on the main thread, between winit events**, with
  the same access an input handler has. IPC never races user input;
  a command observes a consistent document and its effects are
  ordered with respect to the user's own actions.
- The socket is serviced by two dedicated I/O threads that never
  touch app state, so the app never blocks on a slow client. The
  cost of that protection: if a client stops reading long enough
  for **256 outbound frames** to queue, the server declares the
  client dead and drops the connection (reconnect and resubscribe;
  see [Single controller](#single-controller)).
- More than **1024 queued unexecuted requests** answers further
  requests with `overloaded` until the queue drains.
- A wedged main loop (the `FreezeWatchdog` scenario) also wedges
  IPC replies — by design, the watchdog then produces its
  diagnostic abort and the client sees EOF. IPC deliberately has no
  side-channel into app state that could observe or mutate a hung
  loop.

## Command registry and `describe`

Commands live in **one module per family** under
`src/application/ipc/commands/<family>.rs`, assembled into a single
registry table in `src/application/ipc/registry.rs` — one line per
family, mirroring the console's `COMMANDS` slice
(`src/application/console/commands/mod.rs`). Family PRs (IPC-04
through IPC-14) therefore land in parallel without conflicts
(CODE_CONVENTIONS §6): each adds its own module plus one registrar
line.

Registration is **self-describing by construction**: a command's
registry entry carries its name, summary, parameter descriptors
(name / type / required / doc) and result descriptors, and the
constructor requires them — a command without documentation does not
compile. `describe` renders the table:

### `describe`

Params: `cmd` (string, optional — describe one command instead of
everything).

```jsonc
{"id": 1, "cmd": "describe"}
```

```jsonc
{"id": 1, "ok": true, "result": {
  "protocol": 1,
  "app": "mandala",
  "version": "0.1.0",
  "commands": [
    {"name": "clock.step",
     "family": "clock",
     "summary": "Advance the virtual clock and run frames until caught up.",
     "params": [
       {"name": "ms", "type": "integer", "required": true,
        "doc": "Milliseconds of virtual time to advance."},
       {"name": "frames", "type": "integer", "required": false,
        "doc": "Exact heartbeat count to run instead of deriving from ms."}
     ],
     "result": [
       {"name": "now_ms", "type": "number", "doc": "Virtual clock after the step."},
       {"name": "frames_run", "type": "integer", "doc": "Heartbeats executed."}
     ]}
  ],
  "events": [
    {"name": "document_changed", "doc": "Fires after any committed document mutation; data carries the revision."}
  ]
}}
```

The type vocabulary is deliberately small: `string`, `integer`,
`number`, `boolean`, `object`, `array`. Enum-valued strings document
their values in `doc`. This is agent-discovery metadata, not a JSON
Schema dialect — `format/ipc.md` remains the authoritative reference,
and the two are kept byte-honest by tests as described under
[Handshake and versioning](#handshake-and-versioning).

### `ping`

Params: none. Result: `{"now_ms": <number>}` (the app's `now_ms()`
clock — virtual time when `clock.set_mode virtual` is active).
Liveness probe; also the cheapest way to read the clock.

## Command families

The family map, each family's owning issue, and its module. Every
command is listed here from birth; field-level reference sections
below are filled to the depth this design pins — capture and clock
shapes are pinned in full (their stability is what `mandalactl` and
the repo skills build against), query/scene/input/events field lists
are owned by their issues and appended to this document in the same
PR that implements them.

| family | module (`src/application/ipc/commands/`) | owner | commands |
|---|---|---|---|
| *(bootstrap)* | `meta.rs` | IPC-03 | `describe`, `ping` |
| `act` | `act.rs` | IPC-03 | `act.action`, `act.console`, `act.macro` |
| `query` | `query.rs` | IPC-04 | `query.document`, `query.selection`, `query.camera`, `query.mode`, `query.editors`, `query.animations`, `query.undo` |
| `scene` | `scene.rs` | IPC-05 | `scene.dump`, `scene.hit_test`, `scene.find_text` |
| `input` | `input.rs` | IPC-06 | `input.mouse_move`, `input.mouse_button`, `input.wheel`, `input.key`, `input.text`, `input.touch` |
| `capture` | `capture.rs` | IPC-07 (+ IPC-08) | `capture.screenshot`, `capture.record_start`, `capture.record_stop` |
| `clock` | `clock.rs` | IPC-09 | `clock.status`, `clock.set_mode`, `clock.step`, `clock.wait` |
| `events` | `events.rs` | IPC-14 | `events.subscribe`, `events.unsubscribe`, `events.revision` |

## `act` — dispatch bridge (IPC-03)

The §2 rule made wire-visible: these commands **reach the existing
funnels**, never a parallel control path. Everything they do is
undoable, gated, and behaviorally identical to the same operation
performed by hand.

- **`act.action`** — params: `action` (the serde JSON of an `Action`
  variant, exactly as `format/macros.md` step objects carry it: a
  string for unit variants — `"Undo"` — or an object for payload
  variants). Routes through `dispatch_action`. Result:
  `{"outcome": "handled" | "unhandled"}` (`unhandled` = the action
  didn't apply in the current context, e.g. a `Console*` action with
  the console closed — same semantics as `DispatchOutcome`).
- **`act.console`** — params: `line` (string). Routes through
  `execute_console_line` — the same parse → execute → drain-effects
  path the console modal uses, **without** opening the modal.
  Result: `{"output": ["<line>", …]}` mirroring the verb's
  scrollback output (`ExecResult::Ok`/`Lines`); a verb-reported
  failure (`ExecResult::Err`) is `ok:false` with code
  `console_error` and the verb's message.
- **`act.macro`** — params: `id` (string). Routes through
  `dispatch_macro` under the macro's own loader-pinned tier (see
  [Trust model](#trust-model)). Unknown id is `not_found`. Result:
  `{"ran": <bool>}` (the dispatcher's any-step-executed flag);
  privilege-gate aborts surface as `macro_gate_rejected` warnings.

## `query` — application state (IPC-04)

Read-only, allocation-light JSON views of what the application
knows: `query.document` (path, dirty flag, node/edge/section
counts, map metadata), `query.selection` (the `SelectionState`
variant and its ids), `query.camera` (center, zoom, surface size —
the `surface`↔`canvas` transform), `query.mode`
(`InteractionMode`, open modal if any), `query.editors` (in-flight
text/label/portal edit state and previews), `query.animations`
(active animation instances and their clocks), `query.undo`
(stack depth and top entries' kinds). Field-level reference lands
with IPC-04, appended here.

## `scene` — laid-out scene introspection (IPC-05)

What is *actually on screen*: `scene.dump` (visible elements with
resolved geometry and displayed text, honoring `ZoomVisibility` and
fold state; takes `to_file` per [Large payloads](#large-payloads)),
`scene.hit_test` (`x`, `y`, `space` → the same answer the click
pipeline would produce, via the canonical hit-test path, never a
fork of it), `scene.find_text` (locate nodes/sections/labels by
displayed text). Field-level reference lands with IPC-05, appended
here.

## `input` — synthetic raw input (IPC-06)

Below-the-funnel input with human fidelity, for the gestures that
are not discrete Actions: drags, hovers, modal typing, IME-ish text.
`input.mouse_move` (`x`, `y`, `space?`), `input.mouse_button`
(`button`, `state`: `press`/`release`/`click`, optional position),
`input.wheel` (`dy`, optional position), `input.key` (logical key +
modifiers, press/release), `input.text` (string, delivered to the
open modal editor as typed characters), `input.touch` (`phase`,
`id`, `x`, `y` — the recognizer's vocabulary). The fidelity contract:
these enter **the same handler entry points** winit events enter, so
selection state machines, drag thresholds, double-click timing, and
modal steals behave byte-for-byte as with real input. Field-level
reference lands with IPC-06, appended here.

## `capture` — screenshots and recordings (IPC-07, IPC-08)

Shapes pinned here in full; `mandalactl` and the `mandala-drive` /
`mandala-record` skills build against them.

### `capture.screenshot` (IPC-07)

Offscreen render of the current document state — never a window
grab, so it works identically under a compositor, Xvfb, or (post
IPC-10) no display at all.

| param | type | default | meaning |
|---|---|---|---|
| `path` | string | server-chosen file in a per-session artifacts dir | Absolute path for the PNG. Parent dir must exist. |
| `width`, `height` | integer | current surface size | Offscreen target size in physical pixels; camera center/zoom preserved, aspect follows the target. |
| `region` | object | — | `{"x","y","w","h"}` in `canvas` space (override with `space`): frame exactly this rect instead of the current camera view. `width`/`height` still set the target resolution; `region` only decides what the camera frames. |
| `scale` | number | `1.0` | Supersampling factor, clamped to `0.1..=4.0`. |
| `format` | string | `"png"` | `"png"` is the only value at pin time; the param exists so adding one is non-breaking. |
| `sidecar` | boolean | `true` | Write the geometry sidecar next to the PNG (`<path>.geometry.json`). |

Result: `{"path", "sidecar_path"?, "width", "height"}` (plus
`revision` at the envelope level once IPC-14 lands).

**Geometry sidecar** — the pixels↔ids map that lets an agent point
at what it sees:

```jsonc
{
  "protocol": 1,
  "revision": 42,                          // once IPC-14 lands
  "surface": {"width": 1920, "height": 1080, "scale": 1.0},
  "camera": {"center_x": 12.0, "center_y": -340.5, "zoom": 0.75},
  "nodes": [
    {"id": "1.2.3",
     "rect": {"x": 401.0, "y": 92.5, "w": 218.0, "h": 64.0},  // surface px, pre-clip
     "clipped": false,                     // true when partially outside the target
     "sections": [{"index": 0, "rect": {"x": 401.0, "y": 92.5, "w": 218.0, "h": 30.0}}]}
  ]
}
```

Elements suppressed by `ZoomVisibility` or folding at capture zoom
are absent — the sidecar describes the rendered image, not the
document (that is `scene.dump`'s job).

### `capture.record_start` / `capture.record_stop` (IPC-08)

Frame-sequence recording; assembly into GIF/video is deliberately
**outside the app** (the `mandala-record` skill, IPC-13, drives
`ffmpeg`/`gifski` over the manifest). One recording at a time;
starting while recording is `invalid_params`, stopping while idle is
`invalid_params`.

`capture.record_start` params: `dir` (string, default server-chosen),
`fps` (integer, default `30`, clamped `1..=60`), `max_frames`
(integer, default `600`, hard cap `3600` — the recorder stops itself
and warns rather than filling a disk), plus `width`/`height`/
`region`/`scale` exactly as `capture.screenshot`. Result: `{"dir"}`.

`capture.record_stop` params: none. Result: `{"dir", "frames",
"manifest_path", "dropped_frames"}`.

Frames are `frame_%05d.png` plus `manifest.json`:

```jsonc
{"protocol": 1, "fps": 30, "width": 1280, "height": 720,
 "frames": [{"index": 0, "t_ms": 0.0, "file": "frame_00000.png", "revision": 42}],
 "dropped_frames": 0}
```

`t_ms` is capture-clock time (virtual when the virtual clock is
active — recording under `clock.step` yields perfectly paced
sequences; that interplay is IPC-08/09's implementation seam, and
`manifest.json` is its contract).

## `clock` — determinism (IPC-09)

Time virtualization over the one clock bridge (`now_ms()`), plus the
settling primitive every capture workflow needs.

- **`clock.status`** — result: `{"mode": "real" | "virtual",
  "now_ms": <number>}`.
- **`clock.set_mode`** — params: `mode` (`"real"` / `"virtual"`).
  In virtual mode the app-observed clock advances **only** via
  `clock.step`; animation and timing behavior becomes a pure
  function of the step sequence. Result: `{"mode", "now_ms"}`.
- **`clock.step`** — params: `ms` (integer, required) or `frames`
  (integer): advance virtual time and run heartbeats until caught
  up. Error `invalid_params` in real mode. Result: `{"now_ms",
  "frames_run"}`.
- **`clock.wait`** — params: `until` (`"settled"` /
  `"animations_complete"`), `timeout_ms` (integer, default
  `10_000`, clamped `1..=60_000`). **Deferred reply**: evaluated at
  the end of every heartbeat; other commands keep executing while a
  wait is pending; a disconnect cancels it. On success:
  `{"elapsed_ms"}`. On deadline: `ok:false`, code `timeout`,
  `data.elapsed_ms`.

Wait conditions, defined against the six-step `drain_frame`
heartbeat (CONCEPTS §5 "Event loop and `drain_frame`"):

- **`animations_complete`** — `MindMapDocument` reports no active
  animations.
- **`settled`** — at a heartbeat boundary: `needs_continuation()`
  is false (no throttled drag pending, no picker-hover pending, no
  active animations, no dirty connection geometry), the document
  `dirty` flag is clear (the scene rebuild ran), no IPC-injected
  input remains queued, and the renderer has presented a frame at
  the current document revision (the IPC-14 revision counter is the
  arbiter; `clock.wait` consumes it, and if the epic's wave order
  ever runs IPC-09 first, IPC-09 carries the trivial counter field
  forward itself). In plain terms: **the pixels on screen are the
  final consequence of everything sent so far**, and screenshotting
  now is race-free.

There is deliberately **no** third `"idle"` condition: once settled,
the loop parks by construction (`ControlFlow::Wait`), and the only
residual timer is the FPS overlay's idle-grace flip — a diagnostic
that observes behavior and must not become observable behavior.

## `events` — event stream + revision (IPC-14)

Push notifications, opt-in per class: `events.subscribe`
(`classes`: array of class names), `events.unsubscribe` (`classes`
optional — omit for all), `events.revision` (result:
`{"revision"}` — the monotonic document revision counter IPC-14
introduces; also surfaces as the reply-envelope `revision` key).

Reserved event classes at pin time: `hello`, `shutdown`,
`connection_rejected` (the three transport-level classes, never
subscription-gated), `document_changed`, `selection_changed`,
`camera_changed`, `mode_changed`, `animation_started`,
`animation_completed`. IPC-14 owns the class list and payloads and
appends them here; a `log_line` class is possible **only** under
issue #45's Option A and is parked pending that decision. Tree-level
(Baumhard `EventSubscriber`) events are out of scope until the #43
reshape lands; app-funnel events above do not depend on it.

## Change discipline

- This document is the SSOT. A PR that changes wire behavior
  changes this file in the same commit, and bumps `protocol` if the
  change is breaking (see
  [Handshake and versioning](#handshake-and-versioning)).
- `describe` must stay byte-honest against this document; the
  registry's self-description tests enforce it, and IPC-16's
  convergence sweep audits it before the epic closes.
- Command families extend their own section here in the PR that
  implements them; the family map table above is the index and
  gains no rows without a design-level decision recorded in
  `work_plans/LLM_IPC.md`.
