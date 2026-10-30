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
> yet running code. **Running today**: the Unix transport, framing,
> envelope, `hello` / `connection_rejected` / `shutdown`, and the
> bootstrap commands `describe` and `ping` (IPC-02, #62); the command
> registry, reply `warnings`, and the `act` family — `act.action`,
> `act.console`, `act.macro` (IPC-03, #63); the `query` family — `query.document`,
> `query.selection`, `query.camera`, `query.mode`, `query.status`,
> `query.animations`, `query.editors`, `query.undo` (IPC-04, #64); the
> `scene` family — `scene.dump`, `scene.visible_set`,
> `scene.node_geometry`, `scene.hit_test`, `scene.find_text` (IPC-05,
> #65); the `input` family (IPC-06, #66); `capture.screenshot` (IPC-07,
> #67); `capture.record_start` / `capture.record_stop` (IPC-08, #68);
> the `clock` family (IPC-09, #69); the `events` family (IPC-14, #74);
> launches with no display
> ([Headless and one-shot launches](#headless-and-one-shot-launches),
> IPC-10, #70); and the client, `mandalactl`
> ([Client](#client-mandalactl), IPC-11, #71).

## Transport

### Socket, flag, lifecycle

IPC is **opt-in per launch** via a CLI flag, following the existing
`--keybinds <path>` pattern in `src/main.rs`:

```sh
mandala maps/testament.mindmap.json --ipc /run/user/1000/mandala.sock
```

- `--ipc <path>` / `--ipc=<path>` — listen on a Unix domain socket
  (`SOCK_STREAM`) at `<path>`. No flag, no socket, no IPC threads —
  the interactive app pays zero cost when IPC is off. A flag with no
  path (`--ipc` last on the line, or `--ipc=`) is a startup error,
  not an IPC-less launch.
- The listener starts before the event loop takes over (alongside
  `FreezeWatchdog::spawn()` in `run_native.rs::run`, and in
  `run_headless.rs::run` with no window) and lives for the process
  lifetime.
- **Startup, not runtime.** Bind failures are startup errors per
  CODE_CONVENTIONS §9: fail with `expect("<reason>")`, never limp
  along with IPC silently missing.
- **Stale-socket self-heal is type-gated — never a blind unlink.**
  When bind fails because the path already exists, the server
  unlinks and re-binds **only** after confirming, by `stat`, that
  the existing path is a Unix socket (`S_ISSOCK`) owned by the
  current user *and* that a probe `connect` to it is refused (no
  live listener). Any other existing path — a regular file, a FIFO,
  a directory, a socket owned by someone else, or a socket with a
  live listener — is left untouched and startup fails with a message
  naming the path. A typo'd `--ipc ~/thesis.txt` must fail loudly,
  never delete the file. The parent-directory permission check (next
  section) runs **before** this stat→unlink→bind sequence — an
  untrusted parent dir aborts startup outright, closing the
  unlink-then-someone-swaps-the-inode TOCTOU window.
- The socket file is unlinked on graceful exit. After an unclean
  exit the type-gated probe above makes the next launch self-heal.

### Ownership and permissions

The socket is as security-sensitive as `~/.config/mandala/macros.json`
(see [Trust model](#trust-model)). The server creates it with
owner-only permissions (`0600`, in a directory the user owns). It
is bound inside a fresh private (`0700`) staging directory, made
`0600` there, and only then hard-linked to its real name — so there
is no instant at which another user can reach it, and the link
refuses an existing path, so the final step can never replace
anything. A parent directory that does not exist yet is created,
owner-only (`0700`). The `0600` mode protects the socket's *bytes*,
but the socket's *name* is only as trustworthy as the directories
above it — whoever can rename or unlink an entry in any of them can
swap the path for one of their own while Mandala runs. So the server
refuses to bind unless **every directory on the path** is owned by
the user or by root, and is writable by nobody but its owner (group- **or**
world-writable fails) unless it has the sticky bit — one
mechanically checkable predicate per directory. The owner clause is
not redundant with the sticky bit: a sticky directory's *owner* can
still unlink or rename what is in it. "Every directory on the path"
is judged the way the kernel resolves the path the client will
connect through — component by component from `/`, following each
symbolic link where it stands — so it covers each directory that
*holds* a symlink on the path as well as every directory the link's
target leads through, chains included (a symlink in a directory
another user can write is a name they can re-point). Checking only
the canonical path would miss exactly the link holders. A path
longer than the platform's socket-path limit (107 bytes on Linux,
103 elsewhere) is refused up front, naming it. A root-owned sticky directory
(e.g. `/tmp`) is accepted but second-best; the recommended location is
`$XDG_RUNTIME_DIR` (already `0700` on every mainstream distro).
Never a TCP port: filesystem permissions are the access-control
mechanism.

### Single controller

Exactly **one client** is served at a time.

- The first connection becomes the controller.
- While a controller is connected, additional connections receive a
  single frame `{"event":"connection_rejected","data":{"reason":"busy"}}`
  and are closed **immediately** — a dedicated acceptor keeps calling
  `accept()` regardless of controller state, so a second agent learns
  it's busy at once rather than blocking in the OS backlog until the
  controller disconnects (mechanism: `work_plans/LLM_IPC.md` §D2).
  The reject write is non-blocking (best-effort; on `EWOULDBLOCK` the
  rejectee is simply closed) so a rejectee that never reads can't
  stall the acceptor and starve the real controller's reconnect. No
  queueing, no arbitration — two agents dragging the same node
  concurrently is not a coherent workflow, and an arbitration layer
  would be complexity with no named consumer (CODE_CONVENTIONS §7).
- When the controller disconnects (EOF, error, teardown), the
  listener accepts the next connection. **No session state survives
  a disconnect**: event subscriptions reset, pending `clock.wait`s
  are canceled, undelivered frames are discarded. (An in-flight
  recording is app state, not session state — see
  `capture.record_start`.) A reconnecting client starts from the
  `hello` greeting like any fresh client. This is enforced by a
  **controller generation** (a counter incremented on each accept):
  every request, reply, event, and pending wait is stamped with the
  generation it belongs to, the boundary drops any frame whose
  generation isn't current, and the reader's disconnect wakes the
  main thread to cancel that generation's pending waits and reset its
  subscriptions — so a departed controller's late `timeout` reply can
  never land on its successor (design: `work_plans/LLM_IPC.md` §D2).
  The departure is noticed by the controller's reader a moment after
  the socket closes; a client that connects inside that moment is
  refused `busy` like any other, and retries.
- **A half-close ends the requests, not the replies.** A client that
  closes its write half after sending (`printf … | nc -N -U`) still
  receives a reply to every request it sent before the close: the
  disconnect is ordered behind them. The server closes the
  connection once those replies are written.

Attach-after-launch and re-attach are first-class: the intended
workflow is a human launching (or already running) the app and an
agent attaching later while the human watches.

### Windows

The Windows transport is a **named pipe** at
`\\.\pipe\mandala-<name>` (the `--ipc` value names the pipe:
`mandala map.mindmap.json --ipc mandala` serves
`\\.\pipe\mandala-mandala`, and `mandalactl --ipc mandala` finds it),
with the pipe's security descriptor restricted to its owner — the
closest Windows analog to a `0600` socket in `$XDG_RUNTIME_DIR`. The
descriptor is a protected DACL granting generic-all to OWNER RIGHTS
and nothing else (`D:P(A;;GA;;;OW)`), and every instance refuses
remote clients. The owner is the launching user, except that an
elevated (run-as-administrator) launch makes the Administrators group
the owner, so any other elevated process of a member of that group
can connect too; an unelevated launch is the launching user's alone.
Localhost TCP was rejected: it is reachable by every local user on
a multi-user machine (weaker than file permissions), it collides
with other services on port allocation, and it triggers firewall
consent dialogs. The **envelope and command semantics** are
transport-agnostic and carry over unchanged; the *framing* (NDJSON
line-delimiting) and the byte caps below are byte-stream constructs,
re-expressed per transport — a named pipe keeps them, and the
browser's WebSocket transport ([Browser](#browser), IPC-15) replaces
`\n`-delimited lines with WebSocket message boundaries and keeps the
size caps as the app's own per-message limits. "Same protocol" means
the same envelope and commands, not the same framing bytes.

The lifecycle maps onto the pipe as follows (#169, after IPC-02
shipped the Unix transport):

- **One server per name.** The first instance is created with
  `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a second app launched with the
  same `--ipc` value fails at startup naming the pipe — the analog of
  the live-socket refusal. There is no stale state to heal and nothing
  to remove at exit: a pipe vanishes with its server's last handle.
- **Single controller.** The acceptor keeps one instance waiting; a
  client that connects while a controller is attached gets
  `connection_rejected` at once and its connection is closed, as on
  Unix. The acceptor never waits on it: the frame is written on the
  pipe's I/O runtime, whatever the refused client sends meanwhile is
  read and dropped, and its instance is disconnected when the client
  leaves or after a second, whichever comes first — so a refused
  client that has not read the frame within that second loses it, and
  one still writing gets an error rather than blocking (on Unix the
  frame is one non-blocking write, then the socket is closed).
- **Half-close** does not exist for a pipe. A client that closes its
  end still has a final unterminated line executed, but it cannot read
  the reply, so send a terminated line and read before closing.
- **Closing.** A connection that ended in order (after `shutdown`, a
  `connection_rejected`, or a client that left) is closed without
  discarding anything: what the client has not read yet stays
  readable, as a socket's buffer drains after `shutdown`. A connection
  the server gives up on — a write stalled past the stalled-write
  timeout, or a hang-up — is *disconnected*, which discards its unread
  output and fails the client's own pending writes, so a client that
  stopped reading is never left blocked on a pipe the server has let
  go of.
- **Connecting** waits for a free instance (`WaitNamedPipeW`). The
  server keeps one waiting except for the instant between handing one
  to a client and creating the next, so a client should connect with a
  short deadline rather than fail on the first `ERROR_PIPE_BUSY`;
  `mandalactl` does.

### Browser

The browser build speaks this protocol too (IPC-15): the same
envelope, the same commands, the same replies and events — the
registry is one table on both targets — over a transport of its own,
because a page can neither serve a local socket nor listen at all.

**Topology: the page connects out, through a relay.** `mandalactl
bridge` listens on loopback (`127.0.0.1:41816` by default; `listen=`
moves it, to another loopback address only) and serves two roles,
each behind the same token in its URL path:

- `ws://127.0.0.1:41816/app/<token>` — the page. Open the page with
  `?ipc=<this URL>` beside `?map=` (percent-encoded or not) and it
  connects, reconnects when the relay goes away (250 ms doubling to
  5 s), and otherwise does nothing: without `?ipc=` no socket is
  opened and nothing in the transport runs, as native runs nothing
  without `--ipc`. A page is **opt-in per load**, never a default.
- `ws://127.0.0.1:41816/controller/<token>` — the client: `mandalactl
  --ws <this URL>` (or `MANDALA_WS`), or anything that speaks
  WebSocket text frames.

The relay forwards text frames verbatim in both directions and owns
the [single-controller](#single-controller) rule. One page at a time
(a second is closed at the handshake); one controller at a time (a
second receives `{"event":"connection_rejected","data":{"reason":"busy"}}`
and is closed); a controller that arrives before any page receives
`{"event":"connection_rejected","data":{"reason":"no_app"}}` and is
closed, since nothing could greet it — `mandalactl` reports that one
with its remedy and does not retry it, as it does not retry any
reason but `busy`. A path naming another token, or no role, is refused
at the WebSocket handshake with HTTP `403` and nothing is relayed.

**Framing, re-expressed.** One frame is one WebSocket **text message**,
without the newline the socket transport ends a line with; a binary
message is nothing the protocol spells and is ignored. The caps are
the app's and unchanged — a text message over the 1 MiB inbound cap is
answered `parse_error` with `data.line` (a per-controller count of
messages, blank ones included), a reply over 8 MiB becomes
`reply_too_large` — and the relay only bounds its own memory above
them. Events are offered while the page's send buffer is under 1 MiB
and otherwise wait, coalesced behind newer ones of their class, as the
socket transport holds them behind a full queue.

**Lifecycle, mapped onto the relay.** The relay tells the page about
its other role with two control frames of its own, the one addition
to the wire — `{"bridge":"attached"}` and `{"bridge":"detached"}`, a
JSON object with that one key and no other, which no request is:

- *attached*: a controller connected. The page starts a fresh session
  — a new controller generation; subscriptions cleared, no wait
  pending — and greets it with `hello`, which the relay forwards: the
  first frame a controller reads is the greeting, exactly as on the
  socket. `hello.pid` is `null`: a page has no process id. Every other
  `hello` field is as documented.
- *detached*: the controller left. The page resets its session the way
  the native acceptor's `Gone` does; a request still in flight from
  the departed controller is dropped by generation, never executed
  for the next one. The page receives on callbacks and executes later,
  in order, so its queue can hold a departed controller's request
  ahead of the `detached` that retires it (a throttled background tab
  pumps seconds late): it stamps each queued message with the
  controller attached when it arrived, and executes only those whose
  controller is still current once every queued arrival has happened —
  native's generation check, which compares a request's stamp against
  the live controller when the request runs. A dropped message is not
  answered, a `parse_error` included, so nothing the departed
  controller sent reaches the next one ahead of its `hello`.
- The page leaving (tab closed, reload, the relay's socket lost): the
  controller receives `{"event":"shutdown","data":{}}` and is closed,
  the shape an exiting native app gives. A new page that connects
  waits for a new controller: no controller outlives the page it was
  attached to, so there is never one already there to tell it about.
- A controller's frame that has the relay's shape is dropped by the
  relay, never forwarded, so the page cannot be told about a
  controller by anyone but the relay.

Text from the page while no controller is attached goes nowhere, as
an event nobody subscribed to does.

**Where commands run.** On the browser's main thread, between event
handlers and never inside one: the socket's callbacks queue what they
received, and the app drains the queue — executing each command with
the same access an input handler has, observing the app for the event
stream, evaluating a pending `clock.wait` — from the callback's own
task, from the end of every frame, and from a timer armed at a wait's
deadline. That is this target's spelling of the
[threading contract](#threading-contract-wire-visible-consequences)
on a target that has no threads; a command's effects are ordered with
the user's exactly as on native.

**What the browser serves, and what it says it does not.** Every
command is registered on both targets, and `describe` reports per
command whether this build serves it (`supported`, and
`unsupported_reason` when not); a request for one it does not is
answered `unsupported` before its parameters are looked at. On the
browser that is:

| command | why |
|---|---|
| `act.console` | no console runtime: the shell that executes a line is native until console parity lands (CONCEPTS §6 "Console"). `act.action` and `act.macro` run on both. |
| `capture.record_start`, `capture.record_stop` | no files and no frame recorder. `capture.screenshot` is served, inline (below). |

Beside those, `act.action` with an `Action` whose body is
`WasmCompatibility::NativeOnly` is answered `unsupported` naming the
action — the cross-platform dispatcher would answer `unhandled`,
which also means "nothing to do", so the honest answer is the
refusal. A `NativeOnly` `Action` reached as an `act.macro` step is
not refused: a macro step is a keystroke-level instruction, and it is
answered as a keystroke bound to that `Action` is on the browser — it
does not run, and does not count toward `ran` — while the macro's other
steps run. The refusal belongs to `act.action`, where the `Action` is
the whole request. `query.*`, `scene.*`, `input.*`, `act.macro`, `clock.*` and
`events.*` are at parity, with these per-target shapes: `query.mode`
reports `drag_state.kind: "None"` (the browser has no drag-state
machine) and `modals` from the one modal it has, the node text editor;
`query.editors` reports `null` for the console, the color picker and
the single-line editor; `query.animations` reports no pause (the pause
is keyed to native's drags); `query.document` and `scene.dump` declare
no `to_file`/`overwrite` (no filesystem — a reply over the cap is
`reply_too_large` with no file alternative, so keep `include` narrow).
`capture.screenshot` on the browser is the canvas as drawn, at its own
size, returned inline: see [`capture.screenshot`](#capturescreenshot-ipc-07)
for the per-target table. `clock.set_mode virtual` and `clock.step`
work: a step runs the browser's frame — one injected sample, then the
animation tick — at each virtual instant through the same schedule
native's heartbeat runs (`app/step.rs`), and the page's own frames
stand still until the clock is real again.

**Trust: the delta from the socket.** The socket's guard is the
filesystem: `0600`, same-user, a path nothing else can open. A
loopback port has no such owner: **any page in any browser on this
machine can open a WebSocket to `127.0.0.1:<port>`**, and a browser's
same-origin policy does not apply to WebSocket connects. The relay's
answer is the token: 128 bits from the OS (`getrandom`), or `token=`
if given (at least 16 characters of `A-Z a-z 0-9 - _`, so a
convenience token cannot quietly be no guard), required on both roles, printed once at startup and
otherwise unguessable — so a page that does not hold it cannot attach
as a controller and cannot impersonate the app. The token is a
**same-user secret, the tier the socket path is**, not a network one:
it travels in a `ws://` URL that the page's own `location.search`
carries in the clear, and the relay listens on loopback only (another
`listen=` address is refused at startup). The privilege tier is
unchanged — commands execute at `User`, the page's own tier — and so
is what a hostile map can do (nothing more than on native). Nothing is
exposed beyond this machine, and nothing without a user who started a
relay and loaded a page with its URL.

**Where the token is printed.** The relay prints both URLs once, at
startup. Every line the app and `mandalactl` write shows a relay URL
redacted past its role (`ws://127.0.0.1:41816/app/…`) — the page's
logs, a browser message the page logs, and every `--ws` error. One
line is outside the page's control: the browser itself writes
`WebSocket connection to 'ws://…/app/<token>' failed` to its own
console on every failed connect and reconnect. It is not captured into
a reply's `warnings`, but it is in the console, so treat a console
capture from an IPC-enabled page as holding the token.

### Headless and one-shot launches

`--ipc` is one of four launch flags this protocol's users reach for;
the other three exist so an agent, a CI job or a container with **no
display and no GPU** can run the app at all (IPC-10, #70). Like
`--ipc` they are native only.

```sh
mandala --headless --ipc "$XDG_RUNTIME_DIR/mandala.sock" maps/testament.mindmap.json
mandala --headless --screenshot out.png maps/testament.mindmap.json     # load, capture, exit 0
```

- `--headless` — run with **no window, no display server and no
  event loop**. No X11 or Wayland is opened and no surface is made;
  the renderer is bootstrapped from an adapter chosen with no surface
  to be compatible with, and draws pixels only when a capture asks for
  them. Everything in this document works as it does under a window,
  with the differences listed below. A `--headless` launch with
  neither `--ipc` nor `--screenshot` is refused at startup: nothing
  could drive it or see it, and it would run until killed.
- `--size <width>x<height>` — the size, in pixels, of what frames are
  drawn for: the window's inner size (a window manager may still
  resize it), or the headless target's, which nothing resizes. Each
  side in `1..=16384`; a side the GPU cannot hold is clamped to what
  it can, as a resized window's is. Without it a window opens at the
  size it opens at, and a headless app is **800×600** — the size a
  window that asks for none was observed to open at under Xvfb, so the
  two frame the same view.
- `--adapter auto|software` — which GPU adapters the renderer may run
  on. `auto` (the default) asks wgpu for the adapter the power
  preference favors — software adapters included, ranked last — and,
  if that adapter cannot open a device, for a software one.
  `software` asks for a software (CPU) adapter only — Mesa's lavapipe
  or llvmpipe, WARP — so every runner rasterizes alike whatever GPU it
  carries. wgpu's own environment variables are honored with wgpu's
  meaning, in both drivers: `WGPU_BACKEND` narrows the backends,
  `WGPU_POWER_PREF` (`low`, `high`, `none`; `high` when unset) sets the
  preference, and `WGPU_ADAPTER_NAME` picks the first adapter whose
  name contains it, ignoring case. A launch that can use no adapter
  fails at startup with every attempt's reason.
- `--screenshot <path> [--overwrite]` — a **one-shot** launch: once the
  app has settled (the `clock.wait until=settled` condition), it
  captures the window's view at the surface size, with its geometry
  sidecar, to `<path>` — read against the working directory — once a
  first frame has been rendered, and exits: `0` when the PNG was
  written, `1` when it could not be, with the reason on stderr. It is
  `1`, with nothing written, when the map did not load (the load-failure
  placard is not the map asked for; the `startup:` line says why), when
  the frame cannot be drawn for a reason retrying would not cure, and
  when the app ends first — its window closed, or `Action::Quit`
  dispatched. The capture is `capture.screenshot` with no parameters but
  `path`, and its write guards: no symlink at `path` is followed, and an
  existing file there (or its sidecar) is refused unless `--overwrite`
  is given. A one-shot runs on a **virtual clock** (below), so it
  captures the map at its first instant and an animation cannot keep it
  from settling. It works with or without `--headless` and `--ipc`; with
  a window the window opens, and closes once the capture is written. A
  window's capture is at the size the window has when it is taken; a
  window manager that resizes a new window after its first frame (tiling
  ones do) is not waited for, since nothing announces such a resize. For
  an exact size, use `--headless --size`. `--overwrite` without
  `--screenshot` is refused at startup.
- A startup failure — a refused command line, no usable adapter, a
  socket that cannot be bound — ends the process with Rust's panic
  status (`101`) and a message naming what failed.

**What differs with no window.**

- **The clock starts virtual** (§[`clock`](#clock--determinism-ipc-09)):
  with no one watching, time moves only when `clock.step` moves it, so
  a headless run is a pure function of its commands. Under it, an
  animation stands still between steps, and an `input.drag`'s paced
  samples land on the step's heartbeats (`clock.step frames=<n>`), not
  on their own. `clock.set_mode real` gives the headless app real
  time, and with it frames paced at one per 16 ms of wall time — a
  60 Hz display's rate, the rate a window's present paces it at.
- **Input is the `input` family's alone.** It enters at the same
  entry points a window's events do, so nothing below them can tell
  the difference; there is no cursor to draw, so a cursor-icon change
  is no call at all. With no human there, `settled` is exact.
- **A frame draws nothing.** There is nothing to draw into and no one to
  see it, so a headless frame is bookkeeping: it counts as drawn for
  `settled`, and `capture.*` draws the pixels when asked, through the
  very prepare / encode pair a window's frame uses — so pixels are
  produced exactly when they are wanted. A headless capture and a
  windowed one of the same state, on the same machine and adapter, hold
  the same pixels: the target format is the sRGB format a window's
  surface offers under Xvfb on Mesa's lavapipe, and the two have been
  verified there byte-identical, PNG and sidecar.
- **There is no window to close: `act.action Quit` ends it.** The
  `Quit` action is the graceful exit — the one a window's close
  button takes — for either driver: the reply to it is sent, the
  client hears `shutdown`, a recording in progress is finished, and
  the socket and the artifacts dir are removed. It is User-tier, as
  IPC is, and unbound by default (it does not ask about unsaved
  changes, any more than closing a window does). A one-shot launch
  ends on its own. A signal, as under a window, is not a graceful
  exit: the socket and the artifacts dir are left behind, and the
  next launch on the same path self-heals the socket.

### Framing

- **NDJSON**: one JSON object per line, `\n`-terminated, UTF-8. A
  frame must not contain a raw newline (JSON string escapes cover
  every payload). No pretty-printing on the wire. A trailing `\r`
  is tolerated (it is JSON whitespace), and a final line the client
  closes the connection without terminating is still read as a line.
- Empty (or whitespace-only) lines are ignored — but still counted
  by `data.line` below, which indexes every line the client sent.
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
- A `parse_error` reply carries `data.line` — a monotonic
  0-based index of the inbound line that failed. Because a
  `parse_error` has `id: null` (there was no parseable id to echo)
  **and** is written ahead of any in-flight command replies (the
  reader emits it directly, without a main-thread round-trip), it is
  the one reply a pipelining client can neither correlate by `id`
  nor locate by stream position; `data.line` is how the client
  identifies which sent line was rejected.

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
| `id` | string or integer | Required. Client-chosen, echoed verbatim on the reply, ≤ 256 bytes serialized (a longer id is answered `invalid_params` with `"id": null` — the oversized value is not echoed). The server treats it as opaque and does not deduplicate — reusing an in-flight id is the client's own confusion. |
| `cmd` | string | Required. A registered command name: `family.verb`, lowercase snake_case on both sides of the dot. Exactly two bootstrap commands are unnamespaced: `describe` and `ping`. |
| *params* | — | Command parameters sit **flat at the top level** beside `id` and `cmd`. `id` and `cmd` are reserved words no command may use as a parameter name (see the rule below). Unknown parameters are rejected with `invalid_params` (strictness catches agent typos early; a tolerant server would silently ignore `regoin=`). |

Flat parameters (rather than a nested `"params"` object) are a
deliberate ergonomic choice for the primary consumer — an LLM
composing frames by hand: one less nesting level to get right, and
the common commands read as a sentence.

**Reserved-word rule (load-bearing for the flat layout).** Because
params share the top level with the envelope, no command may name a
parameter `id` or `cmd`. Commands that genuinely need to *reference*
a request id or a command name give the parameter a distinct name —
`describe` filters on `command` (not `cmd`), `act.macro` targets a
`macro_id` (not `id`), `input.touch` carries a `pointer_id` (not
`id`). This is what keeps flat params unambiguous; the registry's
compile-time parameter descriptors (see
[Command registry](#command-registry-and-describe)) enforce it — a
command declaring an `id` or `cmd` param fails to compile, so the
collision can't reach the wire. (A nested `params` object would
dodge the rule but cost every frame the nesting level flat params
exist to save; the rule is the cheaper trade.)

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
| `revision` | integer | The event revision observed after the command executed (see [`events`](#events--event-stream--revision-ipc-14)), so an agent can order a reply against change events without an extra round-trip. Present on every reply from a command that ran with the app up; absent on `parse_error`, `overloaded`, `reply_too_large` and the like, which the reader answers itself, and on a command that ran before the window existed. |
| `document_revision` | integer | The document's revision after the command executed: committed model changes, monotonic for the process. Present exactly when `revision` is. A polling agent compares it between replies to learn whether the document changed, without diffing it. |

Replies are **not guaranteed to arrive in request order**. Most
commands reply immediately and in order, but deferred-reply commands
(`clock.wait`, and any future long-running command) reply when their
condition resolves while later requests keep executing. Correlate by
`id`, always. The corollary: a pending deferred reply is **not a
barrier** — a request sent while a `clock.wait` is pending executes
immediately, against whatever state it finds. A command that must
observe the settled state (the canonical mutate → `clock.wait` →
`capture.screenshot` sequence) is sent **after** the wait's reply
arrives; that round-trip is the synchronization point. Pipelining
the screenshot behind a still-pending wait reintroduces exactly the
race the wait exists to prevent.

### Event

```jsonc
{"event": "document_changed", "data": {"revision": 42, "document_revision": 17, "source": "console"}}
```

| key | type | notes |
|---|---|---|
| `event` | string | Event class name, snake_case. |
| `data` | object | Class-specific payload; always present, `{}` allowed. |

Events are opt-in via `events.subscribe`. Three classes
are transport-level and never subscription-gated: `hello` and
`shutdown` always go to the controller; `connection_rejected` goes
to a refused connection instead of any reply. Reserved classes are
listed under
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

All `hello` fields except `map` are process-static
(`protocol`/`app`/`version`/`pid`; on the browser `pid` is `null`, a
page having no process — clients read it as optional); `map` is live
app state
(`MindMapDocument.file_path` — the file the document is bound to, so
it follows a load, a `new` and a save-as, and is `null` while no file
is bound). The state-free boundary threads do
not read it directly — the main thread publishes the current path
into a shared snapshot (an `Arc` swapped whenever the bound path
changes), which
the acceptor reads when composing the greeting, preserving the "I/O
threads never touch app state" invariant (§D2).

- `protocol` is a single integer, currently **1**. Pre-V1 there is
  no compatibility machinery beyond it (CODE_CONVENTIONS §10: break
  freely, delete rather than deprecate): **any** breaking change to
  framing, envelope, or a shipped command bumps the integer, in the
  same commit that changes this document. Additive changes (new
  commands, new optional params, new result keys, new warning
  codes) do not bump it — clients must tolerate unknown keys in
  replies and events. **Widening an existing enum-valued field
  (a new `outcome`, `mode`, `state`, or event-class value a client
  branches on) is breaking, not additive** — it bumps the integer,
  because a client's exhaustive branch would silently misclassify
  the new value. Clients should still treat an unrecognized enum
  value as the safe default for that field (e.g. an unknown
  `act.action` `outcome` → treat as `"unhandled"`).
- Clients pin against this document, check `protocol` in `hello`,
  and refuse to drive a server they don't understand (`mandalactl`
  refuses unless `--force`). There are no version-negotiation
  frames; there is exactly one live protocol per binary.
- `describe` (below) is the runtime mirror of this document for
  every *merged* command, kept honest by unit tests asserting the
  registry matches the documented surface; the IPC-16 sweep
  re-verifies before the epic closes. (This document's family map
  also lists not-yet-merged commands as forward contract; `describe`
  reflects only what the running binary actually registers.)

On graceful shutdown (window close — the only quit path today), the
server best-effort sends `{"event":"shutdown","data":{}}` before
closing the socket, so agents can distinguish "app exited" from
"connection died".

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
| `parse_error` | Frame was not a JSON object / exceeded the 1 MiB line cap. `id` is `null`; `data.line` locates the failed inbound line. |
| `unknown_cmd` | `cmd` names nothing in the registry. |
| `invalid_params` | Missing/mistyped/unknown parameter. `data.param` names the offender. |
| `unsupported` | Command exists but not on this platform/build (e.g. future Windows gaps). |
| `busy` | A single-instance command was already pending when another was issued — a second `clock.wait` while one is in flight (waits are exclusive per controller), a `clock.step` or `clock.set_mode` asked for from inside a running step — or `input.click_node` while injected samples are still queued (`data.pending`). Distinct from the connection-level `connection_rejected` event. |
| `overloaded` | Request queue full (1024 pending); resend after draining replies. Also an `input` command whose samples would overflow the injected-input queue (cap 1024, `data`: `{"pending", "cap"}`); nothing was queued, resend once `input.pending` has dropped. |
| `reply_too_large` | Serialized reply exceeded 8 MiB; `data.bytes` is its size, and `data.hint` names the file-sidecar parameter when the command has one. |
| `no_document` | Command needs a loaded document and none is. |
| `not_found` | A referenced entity (node id, macro id, mutation id) does not exist. |
| `console_error` | The console rejected the `act.console` line (unknown verb) or the verb reported an error; `message` is the console's own text. |
| `timeout` | Deferred command hit its deadline (`clock.wait`). |
| `internal` | The degrade-don't-crash catch-all: the command hit a state it could not honor; details in `message`. |

### Warnings — per-command diagnostics

Release builds keep `warn!` and `error!` (`release_max_level_warn`;
#45 chose that), but they reach the app's stderr, and an agent
attached over the socket does not read that stream. **Replies are the
diagnostic channel the agent does see.** Every `warn!` / `error!` a
command's execution emits is *also* carried in the reply's `warnings`
array, in the log line's own words. This is the one warning-carrying
convention for the whole surface:

```jsonc
{"id": 3, "ok": true,
 "result": {"ran": true},
 "warnings": [
   {"code": "macro_gate_rejected",
    "message": "macros: macro 'tidy' (source Map): ConsoleLine step rejected — only User-tier macros may run console verbs; aborting remaining steps"}
 ]}
```

- `warnings` is an array of `{"code", "message"}` (same stability
  rules as error codes), omitted when empty. A failed command carries
  its warnings too: the diagnostics that explain a failure are the
  ones an agent most needs.
- **How a warning gets there: the execution window.** The registry
  runs every command handler inside a capture window
  (`baumhard::util::log::capture`). The logger the app installs
  offers each record to the window open on the main thread, so
  every `warn!` / `error!` emitted while the command executes lands
  in its reply. That covers the dispatch arms, document setters,
  console verbs and Baumhard primitives it reaches. *Log and push,
  same site, same text* therefore holds by construction: the log
  line is the warning, emitted once, and a `warn!` added anywhere
  later is reported with no further work. `RUST_LOG` decides what is
  printed to stderr, not what a reply carries. A narrow filter does
  not hide a degrade from the agent.
- **Codes.** A log site that wants a machine-branchable identity
  attaches it as the record's `code` key-value
  (`log::warn!(code = GATE_REJECTED_CODE; "macros: …")`), and the
  warning carries that code. Every other record is classified by its
  level. A degrade that happens below a seam *without* logging is
  not seen by the window. Making it log, or giving its site a code,
  is sanctioned §2 integration work, and implementing issues own the
  completeness of their own commands' warnings.
- One reply carries at most 64 captured records (the first ones,
  since the first warning is usually the cause), each message at most
  4096 bytes (a longer one is cut on a grapheme boundary and says how
  much was elided). Any records beyond that are counted in a closing
  `warnings_truncated` entry. The bound is on bytes, not just records,
  because a message can echo arbitrary input, such as a failed console
  line. Without it, an oversized reply would be replaced whole by
  `reply_too_large`, and the command's result would be lost with it.
- Degrades *outside* any command's execution window (a font drop
  during an ordinary interactive frame) are not retroactively
  attached to replies. Streaming those would be a `log_line` event
  class. #45 settled on `release_max_level_warn`, so such a class is
  possible. IPC-14 does not ship one (see
  [`events`](#events--event-stream--revision-ipc-14)); it stays
  undesigned here.

Warning code registry (additions are non-breaking, like error codes):

| warning code | meaning |
|---|---|
| `log_warn` | A `warn!` the command's execution emitted with no code of its own. `message` is the log line. |
| `log_error` | An `error!` the command's execution emitted with no code of its own. `message` is the log line. |
| `warnings_truncated` | The command emitted more diagnostics than one reply carries (64); `message` says how many were left out. |
| `macro_gate_rejected` | A macro step was refused by its tier's privilege gate, and the rest of the macro aborted (`format/macros.md` "Privilege model"). |
| `save_unbound` | `SaveDocument` found no file path bound to the document, so nothing was written. `SaveDocumentAs` (or `act.console` with `save <path>`) binds one. |
| `save_failed` | `SaveDocument` could not write the bound file; `message` carries the I/O error. |
| `console_error` | A console line run by something other than `act.console` failed: a file-path `Action`'s line, or a macro `ConsoleLine` step. Matches the `act.console` error code, so a console failure is branchable however the line was reached. `message` is the log line's words: the failed line and then the console's own error, each with the home directory folded to `~` and excerpted — the line at `EXCERPT_WIDTH`, the error at the wider `FAILURE_WIDTH` (CODE_CONVENTIONS §9). |

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
  `dispatch_macro`, and the macro's loader-pinned `SourceTier`
  gates it exactly as if a keybind had fired it: a Map- or
  Inline-tier macro still cannot run `ConsoleLine` or destructive
  Actions just because IPC pulled the trigger. IPC is a User-tier
  *initiator*, never a privilege *escalator* — the fail-closed gate
  in `dispatch_macro` (`src/application/app/dispatch/macro_core.rs`)
  is untouched by this protocol. An anonymous step list sent as
  `act.macro`'s `steps` runs through that same gated loop
  (`run_macro_steps`) at the `User` tier. Privilege attaches to
  where instructions were authored, and the IPC client authored
  those steps, exactly as it authors an `act.console` line.
- Provenance is a logging concern, not a privilege concern: IPC
  dispatch sites attribute their origin in diagnostics (and in
  events: a `document_changed` an agent's command caused has
  `source: "ipc"`), which covers auditability without a
  `SourceTier::Ipc` variant that would either duplicate `User` or
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
- The socket is serviced by dedicated boundary threads that never
  touch app state, so the app never blocks on a slow client. An
  **acceptor** always waits on `accept()` — that is what lets a
  second client get its `connection_rejected` *immediately* rather
  than sitting in the OS backlog until the controller leaves (see
  [Single controller](#single-controller)); a per-controller
  **reader** parses inbound lines; a per-controller **outbound**
  writer serializes replies and events (per-controller so its queue
  is discarded by construction when the controller leaves — nothing
  crosses to the successor).
- **Backpressure is asymmetric by frame kind.** *Replies* are
  contractual, and are never dropped while the connection lives:
  when **256** replies wait in the outbound queue (which holds them
  plus a separate lane of 64 change events that never takes a reply's
  slot), the server **pauses executing that controller's requests** until the writer
  has made room, then resumes where it stopped. A pipelining client
  that reads its replies therefore never loses one, however far
  ahead it sends (up to the request-queue cap below). A client that
  has stopped reading is detected at the socket instead: once a
  write makes **no progress for 10 seconds** — the kernel's socket
  buffer full of output the client has not read — the client is
  declared dead and the connection dropped (reconnect and
  resubscribe). The reader's own replies (`parse_error`,
  `overloaded`, …) wait for room too, which stops it reading:
  pressure lands on the client that is not draining, never on the
  app. *Unsolicited events* the client never paced itself against
  (`camera_changed`, `animation_*`, …) are instead
  **coalesced/dropped-oldest by revision** under pressure rather
  than pausing anything — an agent that pauses to think for a few
  seconds while an animation emits a flood of events must not have
  its session stalled or torn down. So a slow-reading client loses
  *stale events*, not its session; only a client that stops reading
  *replies* altogether is dropped. The rule is `events`' delivery
  discipline: an event the controller's queue has no room for stays
  queued, a newer event of its class replaces it, and so at most one
  per class waits (see
  [`events`](#events--event-stream--revision-ipc-14)).
- More than **1024 queued unexecuted requests** answers further
  requests with `overloaded` until the queue drains.
- A wedged main loop (the `FreezeWatchdog` scenario) also wedges
  IPC replies — by design, the watchdog then produces its
  diagnostic abort and the client sees EOF. IPC deliberately has no
  side-channel into app state that could observe or mutate a hung
  loop.

## Command registry and `describe`

Commands live in **one module per family** under
`src/application/ipc/commands/<family>.rs`. Each module exports one
`Family` value, and `commands/mod.rs`'s `FAMILIES` table lists them,
one line per family. That table mirrors the console's `COMMANDS`
slice (`src/application/console/commands/mod.rs`). The registry in
`src/application/ipc/registry.rs` dispatches against it and renders
`describe` from it. Family PRs (IPC-04 through IPC-14) therefore land
in parallel without conflicts (CODE_CONVENTIONS §6): each adds its
own module plus one registrar line.

**Reaching the funnels across a module boundary.** The seams a
command invokes are `dispatch_action`, `dispatch_macro`,
`execute_console_line`, and `InitState` / `input_context()`. They are
`pub(in crate::application::app)`, but `crate::application::ipc` is a
*sibling* of `app`, so a command handler cannot see them directly.
Command handlers therefore run against an **app-side execution
context**: the `CommandTarget` trait (`ipc/target.rs`). Its native
implementation, `IpcCommandTarget` (`app/ipc_target.rs`), holds the
event loop's own state and builds the same `InputHandlerContext` a
winit input handler receives, per call. Each method is one funnel and
nothing else; the `input` family's is the `on_*` entry
points `handle_event`'s winit arms call. This is the indirection
`dispatch_macro` already uses through `MacroDispatchTarget`
(`app/dispatch/macro_core.rs`). It keeps the funnels private and the
`ipc` modules free of `app` internals. A handler reaches the app
through `drive()` (`&mut`) or `observe()` (`&`), so a read-only
command cannot mutate it. Whether a command changed pixels is
answered by the target from each funnel's own outcome
(`CommandTarget::changed_view`): a handled Action, a ran macro step,
a console line that rebuilt the scene, had a side effect or closed
the console. The event loop requests a redraw only then, as a winit
handler does after a mutating input. A refused action, a failed or
read-only console line, or a macro whose steps all skipped costs no
frame.

Before the window exists, which on native is also before the
document does, there is no app to reach, and a command that needs
one answers `no_document`.

Registration is **self-describing by construction**: a command's
registry entry carries its name, summary, parameter descriptors
(name / type / required / doc) and result descriptors, and the
constructor requires them. A command without documentation does not
compile. Parameter descriptors are built by a `const fn` that rejects
a parameter named `id` or `cmd` (the [reserved-word rule](#request)).
Every descriptor lives in a `const`, so an envelope collision is a
compile error, never a wire ambiguity.

The registry validates a request against its descriptors before the
handler runs. An unknown parameter, a missing required one, or one
of the wrong JSON type is `invalid_params` naming it in `data.param`.
`null` counts as mistyped, not omitted. Cross-parameter rules
("exactly one of") and value rules (an enum word, a range) are the
handler's to check. Every handler validates its whole request before
it touches the app, so a refused request has no side effects.
`describe` renders the table:

### `describe`

Params: `command` (string, optional). It describes just this one
command instead of the whole table, and is named `command`, not
`cmd`, per the reserved-word rule. A `command` that names no
registered command is `invalid_params` with `data.param: "command"`.

Result: `protocol` (the integer `hello` carries), `app` (always
`"mandala"`), `version` (the Cargo package version), `commands` (one
entry per registered command, in name order: `name`, `family`,
`summary`, `params`, `result`, `supported` — whether the target this
build runs on serves the command — and, when it does not,
`unsupported_reason`, the words a request for it is refused with), and
`events` (the event catalog, one `name` and `doc` per class). The
registry is the same table on every target; what differs per target is
`supported`, and sending an unsupported command is answered
`unsupported` with that reason before its parameters are looked at
([Browser](#browser) lists them).

The two unnamespaced bootstrap commands report `"family":
"bootstrap"`. The `events` array lists the three transport-level
classes and every subscribable class, in name order.

```jsonc
{"id": 1, "cmd": "describe", "command": "clock.step"}
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
       {"name": "ms", "type": "integer", "required": false,
        "doc": "Virtual milliseconds to advance, 1..=1600000. Exactly one of ms / frames."},
       {"name": "frames", "type": "integer", "required": false,
        "doc": "Exact heartbeat count to run, 16 virtual ms apart, 1..=100000. Exactly one of ms / frames."}
     ],
     "result": [
       {"name": "now_ms", "type": "number", "doc": "Virtual clock after the step."},
       {"name": "frames_run", "type": "integer", "doc": "Heartbeats executed: frames, or ceil(ms / 16)."}
     ]}
  ],
  "events": [
    {"name": "document_changed", "doc": "A committed document change: data carries revision, document_revision and the source (action, console, mutation, undo, input or ipc)."}
    // … the full event catalog; abbreviated here
  ]
}}
```

The `events` array is always the complete registered event catalog
— it is not narrowed by the `command` filter,
which scopes only the `commands` array. Above it is abbreviated to
one entry for space.

The type vocabulary is deliberately small: `string`, `integer`,
`number`, `boolean`, `object`, `array`. Enum-valued strings document
their values in `doc`. A parameter whose wire form is an existing
serde representation with more than one shape reports its words
joined with `|`. `act.action`'s `action` is `string|object`: a string
for a unit `Action` variant, an object for a payload variant. This is
agent-discovery metadata, not a JSON
Schema dialect — `format/ipc.md` remains the authoritative reference,
and the two are kept byte-honest by tests as described under
[Handshake and versioning](#handshake-and-versioning).

### `ping`

Params: none. Result: `now_ms` (number), the app clock
([`clock`](#clock--determinism-ipc-09)) — virtual time when
`clock.set_mode virtual` is active, and before the window exists the
wall clock a new clock reads. `ping` is the liveness probe, and also
the cheapest way to read the clock.

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
| *(bootstrap)* | `meta.rs` | IPC-02; registry by IPC-03 | `describe`, `ping` |
| `act` | `act.rs` | IPC-03 | `act.action`, `act.console`, `act.macro` |
| `query` | `query.rs` | IPC-04 | `query.document`, `query.selection`, `query.camera`, `query.mode`, `query.status`, `query.animations`, `query.editors`, `query.undo` |
| `scene` | `scene.rs` | IPC-05 | `scene.dump`, `scene.visible_set`, `scene.node_geometry`, `scene.hit_test`, `scene.find_text` |
| `input` | `input.rs` | IPC-06 | `input.mouse_move`, `input.mouse_button`, `input.wheel`, `input.key`, `input.text`, `input.touch`, `input.drag`, `input.click_node`, `input.pending` |
| `capture` | `capture.rs` | IPC-07, IPC-08 | `capture.screenshot`, `capture.record_start`, `capture.record_stop` |
| `clock` | `clock.rs` | IPC-09 | `clock.status`, `clock.set_mode`, `clock.step`, `clock.wait` |
| `events` | `events.rs` | IPC-14 | `events.subscribe`, `events.unsubscribe`, `events.revision` |

## `act` — dispatch bridge (IPC-03)

The §2 rule made wire-visible: these commands **reach the existing
funnels**, never a parallel control path. Each one drives the app
through a single funnel, the same one a key binding, the console
prompt or a bound macro reaches. So everything they do is undoable,
gated, and behaviorally identical to the same operation performed by
hand. **Undo fidelity follows from that, and nothing else adds to
it.** `act.action` with `"DeleteSelection"` pushes the undo entry the
Delete key pushes, and a following `"Undo"` (or Ctrl+Z) restores
exactly what it would after a keyboard delete.

All three need the app. Before its window, and with it its document,
exists, they answer `no_document`. Each validates its whole request
first, so a refused request (`invalid_params`) has no side effects.
The event loop requests a redraw after one only when it reports a
change of view (see [Command registry](#command-registry-and-describe)).
A failed console line run by an Action or a macro step is reported as
a `console_error` warning.

### `act.action`

Dispatch one `Action` through `dispatch_action`, as a key binding
fires it — with one difference, which only `PanCanvas` shows. A key
is a held input and this command is not, so a `PanCanvas` from a key
is momentary and ends on that key's release, while one from here is
modal: `query.mode` reports its `drag_state.owner` as `"toggled"`, no
button or key release ends it, and the next `PanCanvas` (from here, a
key, a macro or the console) does.

| param | type | required | meaning |
|---|---|---|---|
| `action` | string\|object | yes | The `Action`, in the externally tagged form macro `Action` steps deserialize through: a string for a unit variant (`"Undo"`), and a `{"Variant": payload}` object for a payload variant (`{"OpenDocument": "/path/m.mindmap.json"}`). An unknown variant or a mistyped payload is `invalid_params`, naming what serde expected. |
| `x`, `y` | number | no, together | A pointer position. With it, the cursor stands there for this dispatch only (afterwards it is back where the user's real pointer is, so their next wheel or key acts where they point, not where the agent did), and the dispatch carries what the canonical click hit test (`compute_click_hit`, the one mouse presses use) finds under it, as a mouse handler's `DispatchHit` does. Pointer-driven arms then behave as under a real pointer: `DoubleClickActivate` routes on the hit, and `ZoomIn` / `ZoomOut` / `CreateOrphanNode` use the position. Without it they use the current cursor, and `DoubleClickActivate` reports `unhandled`. One without the other is `invalid_params`. |
| `space` | string | no | The coordinate space of `x`/`y`: `"canvas"` (default) or `"surface"` ([Coordinate spaces](#coordinate-spaces)). Only valid with `x`/`y`. |

Result: `outcome` (string), `"handled"` or `"unhandled"`: the
funnel's `DispatchOutcome`. `unhandled` means the action did not
apply in the current context. A `Console*` action with the console
closed is one: every `Console*` body edits the open modal, so with
none open it reports that nothing ran. So is a file Action whose
line failed (`OpenDocument` of a missing file, `SaveDocumentAs` to an
unwritable path) and a `SaveDocument` that wrote nothing. Each of
those also carries a warning saying why: `console_error` for the
file-path Actions, `save_unbound` or `save_failed` for `SaveDocument`.
Treat an unrecognized value as `"unhandled"`.

`reason` (string or `null`): why an `unhandled` Action did not run,
when its arm declined for a stated reason; `null` otherwise, and
always `null` with `"handled"`. `FastResizeStart` is one: the resize
it starts is finalized by the release of the mouse button holding the
gesture, and a synthetic pointer holds none, so with `x`/`y` it is
refused with a reason saying nothing holds the gesture rather than
start a drag no release could end (without `x`/`y` there is no
gesture to anchor at, which is a reason too). Drive it as a hand
does instead, with the button it is bound to: `input.drag` with
`button: "right"` and `mods: ["ctrl"]` for the default
`Ctrl+RightDrag`.
`PanCanvas` is another, while a drag owed a commit or a held pan
runs.

```jsonc
{"id": 1, "cmd": "act.action", "action": "ZoomIn"}
{"id": 1, "ok": true, "result": {"outcome": "handled", "reason": null}}
{"id": 2, "cmd": "act.action", "action": "DoubleClickActivate", "x": 412, "y": 96, "space": "surface"}
```

`act.action` dispatches at the `User` tier (see
[Trust model](#trust-model)), so destructive, clipboard and file
Actions pass, as they do for a human at the keyboard. It applies the
Action and nothing around it. The keyboard handler's modal steals,
its context resolution and its macro / custom-mutation fall-through
are the *key's* machinery, not the Action's. Driving those is
`input.key`'s job (IPC-06).

### `act.console`

Run one console line through the console's own parse → execute →
side-effect drain, **without** opening the console.

| param | type | required | meaning |
|---|---|---|---|
| `line` | string | yes | The line, as typed at the prompt (`"section show"`, `"open maps/a.mindmap.json"`). |

Result: `output` (array of strings), the verb's output lines, one
element per line. It is empty for a blank line and for a verb that
succeeds silently. A line whose head names no verb, and a verb that
reports a failure, is `ok:false` with code `console_error` and the
console's own message (`"unknown command: frobnicate"`).

**Result path.** `execute_console_line` returns what the verb
produced (`LineOutcome`: blank, unknown, or the verb's
`ExecResult`) instead of writing it into the scrollback. Each caller
renders it. **The scrollback is the prompt's own transcript**: only
the console prompt prints an outcome there. Every other caller
`warn!`s a failure: a key-bound file Action, a macro `ConsoleLine`
step, `SaveDocument`, and every IPC command. Under IPC that warning is
also the reply's warning, and `act.console` returns the outcome
itself. So nothing an agent sends writes lines into a console the
user has open, or leaves its overlay stale. No fake open modal is
built to scrape. **The full
side-effect drain still runs**: `ConsoleSideEffect::ReplaceDocument`
(the document swap for `open` / `new`), `rebuild_all`, and any modal
handoff, because "behaviorally identical to a human typing it" (§2)
requires those effects, not just the return value. What IPC skips is
the human's half: an open console's scrollback and the command
history are left alone, by `act.console` and by the `act.action` /
`act.macro` paths that run console lines or save.

### `act.macro`

Run a macro through the macro dispatcher's gated step loop, the one
body that runs macro steps.

| param | type | required | meaning |
|---|---|---|---|
| `macro_id` | string | one of | Run the registered macro with this id (named `macro_id`, not `id`, per the [reserved-word rule](#request)). It runs under the tier its loader pinned (see [Trust model](#trust-model)). An id registered in no tier is `not_found`. |
| `steps` | array | one of | Run these steps, an anonymous macro in the on-disk step form of `format/macros.md`: `{"kind": "Action", "action": …}`, `{"kind": "CustomMutation", "id": …, "target"?: …}`, `{"kind": "ConsoleLine", "line": …}`. They run at the `User` tier: the IPC client authored them, and IPC executes as `User`. A list that does not parse is `invalid_params` naming `steps`. |

Exactly one of `macro_id` / `steps`: neither, or both, is
`invalid_params`.

Result: `ran` (boolean), the dispatcher's any-step-executed flag. A
step that did not apply does not count: an unhandled Action, a
skipped custom mutation, or a `ConsoleLine` that failed (also
reported as a `console_error` warning). A privilege-gate abort is not
an error. The steps before it ran, so
the reply is `ok:true` and carries a `macro_gate_rejected` warning
naming the refused step.

```jsonc
{"id": 3, "cmd": "act.macro", "steps": [
  {"kind": "Action", "action": "ZoomFit"},
  {"kind": "ConsoleLine", "line": "fps on"}]}
{"id": 3, "ok": true, "result": {"ran": true}}
```

## `query` — application state (IPC-04)

Read-only views of what the application knows: what a person reads off
the screen chrome, and the app's mental state. Eight commands:
`query.document`, `query.selection`, `query.camera`, `query.mode`,
`query.status`, `query.animations`, `query.editors`, `query.undo`.

**Zero mutation, by construction.** A query reaches the app through
`observe()` (`&`), and the only reads the execution context offers are
`&MindMapDocument` and a `&self` projection of the shell state. So a
query cannot flip `dirty`, push an undo entry, rebuild a scene or ask
for a frame (the reply's repaint flag is never set, and a query polled
in a loop costs no frame). A test holds each of those to it.

**No mirror structs.** Each wire form is a projection that sits beside
the state it exposes (`document/ipc_state.rs`, `app/ipc_state.rs`).
Where a type is pure serde (`MindMap`, `AnimationTiming`) the
projection is its own serializer; elsewhere it is an exhaustive `match`
with no wildcard arm. A new `SelectionState`, `InteractionMode`,
`DragState`, `ThrottledDrag` or `UndoAction` variant therefore fails
the build until its wire form is decided and written here. Every
variant-shaped value is an object whose `kind` is **the variant's own
name**, with that variant's ids beside it. Where a projection leaves
state out, this section names the omission.

**Errors.** Before the app exists every query answers `no_document`.
After it, the four queries that read the document (`query.document`,
`query.selection`, `query.animations`, `query.undo`) answer
`no_document` when its initial load failed. `query.camera`,
`query.mode`, `query.status` and `query.editors` need no document and
answer regardless. A query's own parameter failures are
`invalid_params`, naming the parameter.

### `query.document`

The open document: `file_path`, `dirty`, `undo_depth`, `map`, `counts`
and, on request, the whole model as `mindmap`.

| param | type | required | meaning |
|---|---|---|---|
| `fields` | array | no | Which parts to return, as strings from the list below. Default: every part but `mindmap`, the one whose size is unbounded. Empty, mistyped or unknown words are `invalid_params`. Replies carry the parts in the order below, whatever order they were asked for. |
| `pointer` | string | no | An RFC 6901 JSON Pointer into the serialized model (`"/nodes/1.2"`, `"/edges/0"`, `"/canvas/background_color"`). The reply's `mindmap` is then the value found there. Implies `fields: ["mindmap"]` when `fields` is omitted, and is `invalid_params` when `fields` lacks `mindmap`. A pointer that resolves to nothing is `not_found`, and is reported before anything is written to `to_file`. Only the selected subtree is serialized: the first one or two tokens (`/nodes/<id>`, `/edges/<index>`, `/palettes/<name>`, …) are resolved against the model's own structures, so polling `/nodes/0/position` costs one node, not the map. The result is exactly what a pointer into the whole serialized model would give. A field the serialized form omits while empty (`palettes`, `custom_mutations`, `macros`) is absent to a pointer too. |
| `to_file` | string | no | An absolute path. The file sidecar of [Large payloads](#large-payloads): the fields are written there as JSON and the reply is only `path` and `bytes`. Use it when `mindmap` may exceed the 8 MiB reply cap. The model is **streamed** from the app's own structures into the file, so no copy of it is built beside it. The write is **atomic**: the bytes go to a temp file in the same directory (created exclusively and with `O_NOFOLLOW`), are flushed and `fsync`ed, and are then renamed into place. A failure partway (a full disk) removes the temp file and leaves the destination exactly as it was, so an earlier good export is never replaced by truncated JSON. A replaced file takes the default permissions of a new file. |
| `overwrite` | boolean | no | Only with `to_file`. Allow replacing an existing file; default `false`, where an existing file is `invalid_params`. Without it the file is published with `link(2)`, which fails if the name exists, so a file created in the meantime is not clobbered. A symlink or a directory at the path is `invalid_params` either way, and a symlink is never written through: the temp file is created with `O_NOFOLLOW` and published by `rename(2)`, which replaces the directory entry itself. A missing parent directory is `invalid_params`. |

| result field | type | meaning |
|---|---|---|
| `file_path` | string | The path the document is bound to; `null` for a document never saved. |
| `dirty` | boolean | Unsaved changes. |
| `undo_depth` | integer | Entries on the undo stack (`query.undo` lists them). |
| `map` | object | `{"name", "version"}` of the loaded map. |
| `counts` | object | `{"nodes", "edges", "sections", "palettes", "custom_mutations", "macros"}`. `sections` is summed over every node. |
| `mindmap` | object | The model through its own serde, the `.mindmap.json` shape (`legacy-schema.md`). A save adds the loader's preserved unknown keys; a query does not, because they are the loader's bookkeeping and not part of the model. With `pointer`, the value found there. |
| `path`, `bytes` | string, integer | With `to_file` only, and then the *only* fields: the absolute path written and its size. |

The `mindmap` field round-trips: parsed back as a `MindMap` and
serialized again it is the value the app serialized, so what an agent
reads is what the app holds.

```jsonc
{"id": 1, "cmd": "query.document"}
{"id": 1, "ok": true, "result": {
  "file_path": "maps/testament.mindmap.json", "dirty": false, "undo_depth": 0,
  "map": {"name": "testament", "version": "1.0"},
  "counts": {"nodes": 252, "edges": 258, "sections": 254, "palettes": 12, "custom_mutations": 0, "macros": 0}}}
{"id": 2, "cmd": "query.document", "pointer": "/nodes/0/position"}
{"id": 2, "ok": true, "result": {"mindmap": {"x": -179.432388, "y": -2413.803223}}}
{"id": 3, "cmd": "query.document", "fields": ["mindmap"], "to_file": "/tmp/model.json"}
```

### `query.selection`

Params: none. Result: the `SelectionState`, an object whose `kind` is
`None`, `Single`, `Multi`, `Section`, `MultiSection`, `SectionRange`,
`Edge`, `EdgeLabel`, `PortalLabel` or `PortalText`, with the variant's
ids beside it. Lossless: every field of every variant is carried.

| `kind` | keys beside it |
|---|---|
| `None` | — |
| `Single` | `node_id` |
| `Multi` | `node_ids` (array) |
| `Section` | `node_id`, `section_idx` |
| `MultiSection` | `sections`: array of `{"node_id", "section_idx"}` |
| `SectionRange` | `node_id`, `section_idx`, `section_span` `{"lo", "hi"}` (inclusive **section indices**), `grapheme_range` `{"start", "end"}` (half-open **grapheme-cluster** offsets into that section's text; the two ranges are different units, see `SelectionState`) |
| `Edge`, `EdgeLabel` | `edge`: `{"from_id", "to_id", "edge_type"}` |
| `PortalLabel`, `PortalText` | `edge` (as above), `endpoint_node_id` |

```jsonc
{"id": 1, "cmd": "query.selection"}
{"id": 1, "ok": true, "result": {"kind": "Single", "node_id": "1.2"}}
```

### `query.camera`

Params: none. The `surface` ↔ `canvas` transform ([Coordinate
spaces](#coordinate-spaces)). A canvas point `p` lands at surface
`(p - center) * zoom + surface / 2`, so a client converts without a
round trip per point.

| result field | type | meaning |
|---|---|---|
| `center` | object | `{"x", "y"}`: the canvas point at the middle of the viewport. |
| `zoom` | number | Zoom factor; above 1 is zoomed in. |
| `zoom_min`, `zoom_max` | number | The clamp the camera enforces (`0.05` and `5.0`). |
| `surface` | object | `{"width", "height"}` in physical pixels. |

```jsonc
{"id": 1, "ok": true, "result": {"center": {"x": 12.0, "y": -340.5}, "zoom": 0.75,
  "zoom_min": 0.05, "zoom_max": 5.0, "surface": {"width": 1920, "height": 1080}}}
```

### `query.mode`

Params: none.

| result field | type | meaning |
|---|---|---|
| `interaction_mode` | object | `kind`: `Default`, `Reparent` (`sources`), `Connect` (`source`), `NodeEdit` (`node_id`) or `Resize` (`target`: `{"kind": "Node", "node_id"}` or `{"kind": "Section", "node_id", "section_idx"}`). |
| `drag_state` | object | `kind`: `None`, `Pending`, `PendingRight`, `Panning`, `SelectingRect` or `Throttled`. `Pending` carries `start_pos` (surface pixels), `hit_node`, `hit_section_idx` and `grabs`, the names of every grab the press recorded, in the order a drag past the threshold resolves them, so the first is the gesture it becomes (`node_resize_handle`, `section_resize_handle`, `edge_label`, `portal_label`, `edge_handle`). `PendingRight` carries `start_pos`, `start_canvas`, `hit_node`, `hit_section_idx`. `Panning` carries its `owner`, which is what ends it: `"button"` with the `button` that holds it (`"left"`, `"right"`, `"middle"`, as `input.mouse_button` spells them) or `"key"` with the `key` name that holds it — each ends on that input's release and no other — or `"toggled"`, a pan armed by a dispatch with no release behind it (`act.action`, `act.console`, `act.macro`), which the next `PanCanvas` ends. `SelectingRect` carries `start_canvas` and `current_canvas`. `Throttled` names its `gesture` (`MovingNode`, `MovingSection`, `SectionResize`, `NodeResize`, `EdgeHandle`, `PortalLabel`, `EdgeLabel`) with what it is acting on (`node_ids`, `node_id`, `section_idx`, `side`, `edge`, `handle`, `endpoint_node_id`). A gesture's pre-drag snapshots, pending delta and throttle are omitted: bookkeeping, not identity. |
| `modals` | array | Every open modal, in the order the keyboard handler consults them: `console`, `color_picker`, `single_line_edit`, `text_edit`. Keys do not have one owner. The console takes **every** key. The color picker claims **only its own** keys (its commit, cancel and nudge verbs, and its clipboard keys); every other key falls through to the editors. The single-line editor, then the node text editor, take every key that reaches them. `query.editors` has each one's state. |
| `key_owner` | string | Who receives an *ordinary* key right now: `console`, `single_line_edit`, `text_edit`, or `none` (normal dispatch). Never `color_picker`: with a picker and a text editor both open, a key the picker does not claim goes to `text_edit`, and that is what this field says. Type against `key_owner`, not against the head of `modals`. |
| `hovered_node` | string | The node id under the pointer; `null` over none. |
| `cursor_pos` | object | `{"x", "y"}`: the last known pointer position, in surface pixels. |

### `query.status`

Params: none. Result: `text` (string), the cyan mode-status line the
renderer shows (`"editing: 1.2 — section [1 of 3]"`), or `null` when none
is showing (Default mode). The app already writes this line for a human,
so an agent reads it instead of re-deriving it.

### `query.animations`

Params: none. Result: `active` (boolean, whether any animation instance is
running), `paused` (boolean, whether a tree-mutating drag has suspended
the tick), `count` (integer) and `animations` (array, oldest-started
first). Each entry:

| key | meaning |
|---|---|
| `id` | The running animation's identity: the `id` the `animation_started` and `animation_completed` events carry for it. It never changes while the animation runs (a drag release shifts `start_ms`, not this), and a re-trigger after the animation ended is a new `id` under the same `(target_id, section_idx, mutation_id)`. An agent that subscribed late, or that recovers from missed events, matches a later `animation_completed` to the entry it holds by this. |
| `target_id`, `section_idx` | The node animated, and the section when the trigger resolved to one (`null` otherwise). |
| `mutation_id` | The `CustomMutation` id being animated. |
| `start_ms`, `elapsed_ms` | The app-clock start and the time since, in whole milliseconds: the clock `ping` and `clock.status` report and the per-frame drain ticks against. In real mode it runs at the wall clock's rate; under `clock.set_mode virtual` it advances only by `clock.step`, so `elapsed_ms`, `phase` and `progress` are a pure function of the step sequence ([`clock`](#clock--determinism-ipc-09)). |
| `phase` | `delay` before the delay elapses, `running` while it interpolates, `due` once the full envelope has elapsed and the next tick commits it, and `paused` while a tree-mutating drag (a node or section move or resize) has suspended the tick. While paused, `elapsed_ms` and `progress` are frozen at the value the animation resumes from (on release the drain shifts each `start_ms` forward by the pause), and nothing commits until the drag ends. Poll `paused` to tell a stalled animation from a finished one. |
| `progress` | The linear position through the duration, `0` to `1`, before easing. |
| `timing` | The mutation's `AnimationTiming` through its own serializer (`duration_ms`, `delay_ms` when set, `easing`). |

The from/to node snapshots and the mutation list are omitted. They are
the document's own business, and large. `active: false` with an empty
list is how an agent sees that animations have settled; `active: true`
with `paused: true` is a drag holding them.

### `query.editors`

Params: none. Each modal editor's live state, `null` when it is closed,
and the document's active previews.

| result field | type | meaning |
|---|---|---|
| `console` | object | `input`, `cursor` (grapheme index), `history_len`, `history_idx`, `scrollback_len`, `scrollback` (the last 50 lines, each `{"kind": "Input"\|"Output"\|"Error", "text"}`), `completion_count`, `completion_idx`, `scroll_offset`. Omitted: each line's pinned font family, the completion rows, the wheel accumulator. |
| `color_picker` | object | `mode` (`{"kind": "Standalone"}`, or `{"kind": "Contextual", "handle", "seed_var_ref", "seed_hsv"}` where `handle` is `Edge` (`edge_index`), `Node` (`node_id`, `axis`) or `Section` (`node_id`, `section_idx`, `axis`, `grapheme_range`)), `hue_deg` (`[0, 360)`), `sat` and `val` (`[0, 1]`), `size_scale`, `gesture` (`Move`, `Resize` or `null`), `hover_preview`. Omitted: the cached layout, glyph-measurement caches and wheel placement, which are geometry (`scene.dump` is where what it looks like lives). |
| `single_line_edit` | object | The edge-label / portal-caption editor: `target` (`{"kind": "EdgeLabel", "edge"}` or `{"kind": "PortalText", "edge", "endpoint_node_id"}`), `buffer`, `cursor_grapheme_pos`, `original` (the target's string when the editor opened; `null` if none). |
| `text_edit` | object | The node text editor: `node_id`, `section_idx`, `buffer`, `cursor_grapheme_pos`, `selection_anchor` (`null` without a sub-range selection), `exit_to_default_on_close`. Omitted: the buffer's styling regions and the pre-edit snapshot cancel restores from. |
| `previews` | object | What the scene shows in place of committed model state, never serialized, never undoable, never dirtying: `label_edit` (`edge`, `buffer`), `portal_text_edit` (`edge`, `endpoint_node_id`, `buffer`), `color_picker` (`edge`, `color`), `border` (`kind`: `Nodes`, `Sections`, `CanvasDefault`, `CanvasSectionFrame`, `CanvasSectionFrameFocused`, with its targets; the staged values are omitted) and `rect_select` (the node ids a Shift+drag currently covers). Each is `null` when none is live. An agent whose pixels disagree with `query.document` looks here first. |

Cursors are **grapheme-cluster** indices into their buffer, never bytes:
the unit the editors use, and the one a user counts as characters.

### `query.undo`

| param | type | required | meaning |
|---|---|---|---|
| `limit` | integer | no | How many entries to return, newest first. Default 20, at most 1000 (a larger or negative value is `invalid_params`, so the cap is learned rather than silently applied). |

Result: `depth` (integer, the whole stack) and `entries` (array, newest
first, the order `Undo` takes them). Each entry is `{"kind", "node_ids"?,
"index"?}`: `kind` is the `UndoAction` variant's name (`MoveNodes`,
`CustomMutation`, `ReparentNodes`, `DeleteEdge`, `CreateEdge`,
`EditEdge`, `CreateNode`, `EditNodeText`, `EditNodeStyle`,
`EditNodeZoom`, `CanvasSnapshot`, `EditNodeAabb`, `DeleteNodes`), with the
nodes it touches or, for the edge entries, the edge `index`. The
before-images an entry carries are what undo restores from and are not
reported.

## `scene` — laid-out scene introspection (IPC-05)

What is *actually on screen*, as text: where each node, section,
connection, edge label and portal marker is drawn, whether the camera
shows it, and what text the user reads in it. Five commands:
`scene.dump`, `scene.visible_set`, `scene.node_geometry`,
`scene.hit_test`, `scene.find_text`. An agent answers "is node X on
screen? where? overlapping what? saying what?" from these, without
pixels.

**The source of truth is the drawn trees.** Since the dual-pipeline
consolidation (#22) every model-derived visual reaches the GPU through
one `GfxTree` per canvas role, so those trees *are*
the laid-out scene: positions after layout and in-flight drag offsets,
sizes after text fitting, zoom windows after cascade, text after run
coverage. The family reads them through Baumhard's serializer
(`baumhard::mindmap::scene_dump`), never by re-deriving geometry from
the model. The model is consulted only for what a tree does not carry:
a node's own `folded` flag, and the fold that removed a node from the
scene. Each element is named through the identity index its own hit
test resolves through (the node tree's id maps, the connection and
label trees' `EdgeChannelIndex`, the portal tree's `PortalHitIndex`), so
an element is called here what a click on it is called.

**Every element carries one placement**, three keys beside its own:

| key | meaning |
|---|---|
| `canvas` | `{"x", "y", "w", "h"}`: the extent the renderer draws for the element, in canvas units, offset by its tree's scene offset: the union of its background fill (its box grown by its padding, per edge, the rect `GlyphArea::background_rect` states for the renderer's fill pass too) and every text stamp (the main glyph and each outline-halo stamp, each a box of the element's render bounds). An area that draws neither, an unfilled node container, is placed by its layout box. For a connection, the union of its glyphs. The hit test resolves against the layout box, which can be smaller than a padded fill: `scene.hit_test` answers what a click hits. |
| `surface` | The same rect through the camera, in surface pixels ([Coordinate spaces](#coordinate-spaces)). Not clipped: a rect half off-screen runs past the surface edge. |
| `visibility` | The renderer's own cull verdict, from the one predicate its per-frame cull calls (`Camera2D::visibility`), asked of each drawn part as the renderer asks it, so "visible" here and "drawn" there cannot disagree: `visible` when any part is drawn (a fill whose padding crosses the viewport edge, one halo stamp on screen); `outside_zoom_window`, when the camera's zoom falls outside the element's zoom window (no pan brings it back; checked first, so a part both outside its window and off-screen reports this); or `offscreen`, when it is inside its window but misses the viewport (a pan would show it). |

**Displayed text, not model text.** A section with `text_runs` renders
only the graphemes its runs cover (`format/text-runs.md`); its `text`
here is that covered text, concatenated in run order, through the same
coverage rule the shaper slices spans by
(`baumhard::font::attrs::displayed_text`). A label being edited reports
its live buffer with the caret, which is what is drawn.

**What is not reported.** A node hidden by a folded ancestor is not in
the scene (`scene.node_geometry` names the fold). A connection whose
whole path is clipped inside its endpoint nodes draws no glyph and is
absent. A portal endpoint with no caption draws only its icon. A role
tree registered hidden draws nothing and reports nothing. Borders,
section frames and handles are chrome around the elements above and are
not reported; `query.mode` and `query.editors` describe the interaction
state they reflect.

**Read-only.** Four commands reach the app through `observe()` (`&`).
`scene.hit_test` needs `drive()` only because a BVH descent memoizes
subtree AABBs in the trees it walks; it writes nothing else — not the
document, the selection, the cursor or the camera. No scene command
asks for a frame. Before the app exists every one answers
`no_document`; with the app up but its initial load failed, they answer
`no_document` saying how to load one. Parameter failures are
`invalid_params`, naming the parameter.

Elements are listed in draw order: nodes in tree pre-order (parents
before children, siblings in model order), every other kind in its
tree's order. Edge identities are `{"from_id", "to_id", "edge_type"}`,
as everywhere on the wire. Every coordinate is the shortest decimal
that names its `f32` (`0.1`, not `0.10000000149011612`), inline and in
a `to_file` sidecar alike. The visibility filter is applied before any
text is resolved: an element a command drops costs no text walk, and
`scene.visible_set` resolves none.

### `scene.dump`

The laid-out scene, by kind.

| param | type | required | meaning |
|---|---|---|---|
| `include` | string | no | `"visible"` (default): only what the camera shows. `"all"`: every drawn element with its verdict, so an off-screen one can be found and panned to. With `"visible"`, a node is kept when it or any of its sections is visible, carrying only its visible sections. |
| `kinds` | array | no | Which kinds to report, as strings: `nodes`, `edges`, `labels`, `portals`. Default: all four. A kind not asked for is absent from the reply and its tree is not walked. Empty or unknown words are `invalid_params`. |
| `to_file` | string | no | An absolute path: the file sidecar of [Large payloads](#large-payloads), with the same atomic, no-symlink, no-clobber write `query.document` documents. The reply is then only `path` and `bytes`. |
| `overwrite` | boolean | no | Only with `to_file`, as for `query.document`. |

| result field | type | meaning |
|---|---|---|
| `camera` | object | The camera the verdicts and surface rects were taken at, exactly as [`query.camera`](#querycamera) reports it. |
| `nodes` | array | `{"node_id", "folded", canvas, surface, visibility, "sections"}` per drawn node. `folded` is the model flag: this node's descendants are hidden; the node itself is drawn. Each section is `{"section_idx", canvas, surface, visibility, "text"}`, in model order; a section the tree builder skips (an empty or non-finite authored size) is absent. |
| `edges` | array | `{"edge", canvas, surface, visibility, "glyphs", "visible_glyphs"}` per drawn connection: how many glyphs it draws along its path (caps included) and how many of them the camera shows. A portal-mode edge draws no path and appears under `portals`. |
| `labels` | array | `{"edge", canvas, surface, visibility, "text"}` per drawn edge label. |
| `portals` | array | `{"edge", "endpoint_node_id", "part", canvas, surface, visibility, "text"}` per drawn portal part: `part` is `icon` (the marker glyph) or `text` (its caption), and `endpoint_node_id` the node it floats above. |
| `path`, `bytes` | string, integer | With `to_file` only, and then the *only* fields. |

```jsonc
{"id": 1, "cmd": "scene.dump", "kinds": ["nodes"]}
{"id": 1, "ok": true, "result": {
  "camera": {"center": {"x": 0.0, "y": 0.0}, "zoom": 1.0, "zoom_min": 0.05, "zoom_max": 5.0,
             "surface": {"width": 1280, "height": 720}},
  "nodes": [{"node_id": "1", "folded": false,
             "canvas": {"x": -60.0, "y": -20.0, "w": 120.0, "h": 40.0},
             "surface": {"x": 580.0, "y": 340.0, "w": 120.0, "h": 40.0},
             "visibility": "visible",
             "sections": [{"section_idx": 0, "canvas": {…}, "surface": {…},
                           "visibility": "visible", "text": "…"}]}]}}
```

### `scene.visible_set`

Params: none. Just the identities of what the camera shows, by kind —
the default `scene.dump`'s ids, with no text resolved and no rect
reported.

| result field | type | meaning |
|---|---|---|
| `nodes` | array | Node ids, in draw order. A node is here when it or any of its sections is visible. |
| `sections` | array | `{"node_id", "section_idx"}` per visible section. |
| `edges` | array | Edge identities of the visible connections. |
| `labels` | array | Edge identities of the visible labels. |
| `portals` | array | `{"edge", "endpoint_node_id", "part"}` per visible portal part. |

### `scene.node_geometry`

One node's whole geometry story: what the model says, whether the scene
draws it, and where.

| param | type | required | meaning |
|---|---|---|---|
| `node_id` | string | yes | The node. One the document does not have is `not_found`. |

| result field | type | meaning |
|---|---|---|
| `node_id` | string | The node described. |
| `model` | object | `{"position": {"x", "y"}, "size": {"width", "height"}, "folded"}` as the model holds them: the authored values, before drag offsets and text fitting. |
| `in_scene` | boolean | Whether the scene draws the node. |
| `hidden_by` | string | The nearest folded ancestor, whose fold keeps the node out of the scene; `null` when none is folded. |
| `absent_reason` | string | Why the scene does not draw the node: `folded` (`hidden_by` names the fold) or `not_built` (no scene has been built yet, or the tree builder placed no container for the node); `null` when `in_scene`. |
| `canvas`, `surface`, `visibility` | object, object, string | Only when `in_scene`: the node's placement, as in `scene.dump`. |
| `sections` | array | Only when `in_scene`: the drawn sections, as in `scene.dump`, visible or not. |
| `overlaps` | array | Only when `in_scene`: the ids of the other drawn nodes whose canvas boxes share area with this one (boxes that only touch along an edge do not overlap), in draw order. |

### `scene.hit_test`

What a click at a point would target, through **the canonical click
ladder** — `compute_click_hit`, the function a mouse press, a release
and a touch tap run, never a fork of it — and its last rung,
`edge_under_pointer`, the connection path under the pointer. A test
holds it to a real click: over every node, label, portal part and
connection of the fixture, the selection a production click leaves is
the one this report's ladder names.

| param | type | required | meaning |
|---|---|---|---|
| `x`, `y` | number | yes | The point. |
| `space` | string | no | Its coordinate space: `"canvas"` (default) or `"surface"`. |

| result field | type | meaning |
|---|---|---|
| `canvas`, `surface` | object | The point in both spaces, `{"x", "y"}`. |
| `click_hit` | object | The collapsed hit a press records, which double-click detection compares and `DoubleClickActivate` routes on. `kind` is the variant's name: `Node` (`node_id`, `section_idx` — `null` unless the hit resolved to one section of a multi-section node), `PortalMarker` or `PortalText` (`edge`, `endpoint_node_id`), `EdgeLabel` (`edge`), or `Empty`. |
| `node` | object | The node rung: `{"node_id", "section_idx"}`, or `null`. |
| `portal` | object | The portal rung, tried only when no node was hit: `{"edge", "endpoint_node_id", "part"}` (`icon` or `text`), or `null`. |
| `edge_label` | object | The edge-label rung, tried only when no node or portal was hit: the edge, or `null`. |
| `edge` | object | The last rung, tried only when the ladder came up empty (`click_hit` is `Empty`), which is the only case a click consults it: the connection path within the click tolerance (`EDGE_HIT_TOLERANCE_PX` surface pixels, scaled to canvas units at the current zoom) of the point, or `null`. |

A plain click selects the highest rung that is not `null`: a node
(`Single`, or a section in `NodeEdit` on a multi-section node), a portal
part (`PortalText` / `PortalLabel`), an edge label (`EdgeLabel`), the
connection (`Edge`), or nothing. What the ladder does not see is what
runs *before* it in a real click: an open modal editor's claim on the
pointer and the `Reparent` / `Connect` target modes, which consult only
the node rung. `query.mode` reports both.

```jsonc
{"id": 4, "cmd": "scene.hit_test", "x": 412, "y": 96, "space": "surface"}
{"id": 4, "ok": true, "result": {
  "canvas": {"x": -228.0, "y": -264.0}, "surface": {"x": 412.0, "y": 96.0},
  "click_hit": {"kind": "Node", "node_id": "1.2", "section_idx": null},
  "node": {"node_id": "1.2", "section_idx": null},
  "portal": null, "edge_label": null, "edge": null}}
```

### `scene.find_text`

Locate drawn text: the sections, edge labels and portal captions whose
*displayed* text matches, visible or not, so an agent can find
something off-screen and pan to it. A portal's icon is a marker glyph,
not text, and is never matched. A node hidden by a fold displays
nothing and is not found.

| param | type | required | meaning |
|---|---|---|---|
| `text` | string | yes | What to look for. Empty is `invalid_params`. |
| `exact` | boolean | no | Match the whole displayed text rather than a substring. Default `false`. |
| `case_sensitive` | boolean | no | Default `false`: both sides are compared Unicode-lowercased, so `É` finds `é`. |

Result: `matches` (array), in draw order: sections, then labels, then
portal parts. Each is `{"kind", …ids, "text", canvas, surface,
visibility}` where `kind` is `section` (`node_id`, `section_idx`),
`label` (`edge`) or `portal` (`edge`, `endpoint_node_id`, `part`,
always `text`).

## `input` — synthetic raw input (IPC-06)

Below-the-funnel input with human fidelity, for the gestures that
are not discrete Actions: drags, hovers, modal typing, touch. Where
`act.action` applies an Action, these are the hand on the mouse and
the keyboard. The fidelity contract: each command becomes the
payload a winit event carries, and the app delivers it to **the same
entry point** that winit event reaches — the `on_*`
methods `handle_event`'s arms call, and below them
`handle_mouse_input`, `handle_cursor_moved`, `handle_keyboard_input`
and the touch recognizer. From there down nothing can tell the two
apart, and no handler has a branch for injected input. So the click
ladder, drag thresholds and state machines, double-click timing (on
the real clock), the modal steals, keybind resolution and the macro /
custom-mutation fall-through behave byte-for-byte as with real
input. The finger identifier is `pointer_id`, not `id`, per the
[reserved-word rule](#request).

**Positions** default to `surface` pixels, the space of a real
pointer ([Coordinate spaces](#coordinate-spaces)). A `canvas`
position is converted once per command, through the camera as it
stands when the command's first event is applied: a hand points at
the screen, not at the world behind it, so a drag keeps one camera
for all its frames and a pan-drag given in canvas space does not
chase its own camera. A command that waited in the queue behind a
camera-moving gesture therefore aims through the camera that gesture
left, not the one in force when it was sent. The pointer stays where an `input` command put
it — unlike `act.action`'s pointer, which is put back — because
moving the pointer is what these commands are for: it is the hover
an agent wants to observe.

**Modifiers.** `mods` (array of `"ctrl"`, `"shift"`, `"alt"`,
`"super"`) holds those modifiers for each of the command's events and
puts back the state the OS last reported after each one, so a
keystroke of the user's own between two frames of a paced drag still
sees their own modifiers. Omitted, the reported state stays in
force; `[]` holds none. A drag carries them on every sample, which is
what makes a `"shift"` drag on empty canvas a rubber-band select: the
rubber band reads Shift at the threshold-crossing move.

**Pacing model (pinned here because `settled` and virtual time
depend on it).** A gesture whose fidelity depends on events landing
in *different* frames cannot fire its sub-events in a single drain.
A drag is the case: `drain_inputs` drives the throttled drag
(`ThrottledInteraction`) between its moves, and a press, a dozen
moves and a release run back to back would skip that drain entirely
and test a path no human hits. So `input.drag` enqueues its samples
into a bounded **injected-input queue** (cap **1024**; a command that
would overflow it is refused whole with `overloaded`, and enqueues
nothing) that the heartbeat drains **one sample per frame**, applied
ahead of `drain_inputs` — where a winit event lands — at the app
clock's current reading. A non-empty queue is a continuation source, so the loop
keeps iterating and redrawing until it is empty. Every other command
is one ordered burst, applied before its reply as winit delivers
several events in one iteration: a move, a click, a tap, a typed
string. A double-click needs no pacing — the detector compares the
clock and the hit, and two clicks sent as two `input.mouse_button`
commands are as far apart as the agent sent them. **A burst sent while
samples are queued waits behind them**, so injected input always
lands in the order it was sent; its reply then says `applied: false`.
Under `clock.set_mode virtual`, the loop's own iterations land no
sample: `clock.step` advances this queue one sample per virtual
heartbeat, so injected drags are deterministic too (a step's
`frames=n` lands `n` samples). In real time `clock.wait
until=settled` requires this queue empty (condition (b) under
[`clock`](#clock--determinism-ipc-09)), so "wait until my synthesized
drag finished" is expressible; under a virtual clock the samples are
owed to future virtual instants, so step until `input.pending` reads
0, then wait.

Every command needs the app: before its window exists each answers
`no_document`. Raw input needs no document — a hand can click an
empty window — and only `input.click_node` answers `no_document`
when none is loaded. Each validates its whole request first, so a
refused one (`invalid_params`, naming the parameter) injects nothing.
A command whose events reached a handler asks for a redraw exactly
when the winit arm it stands in for would have.

**Result**, for every command but `input.pending`: `applied`
(boolean) — `true` when the events reached the handlers before the
reply, `false` when they joined the queue — and `pending` (integer),
the samples still queued afterwards. `pending: 0` means everything
sent so far has landed.

```jsonc
{"id": 1, "cmd": "input.drag", "from_x": 400, "from_y": 300, "to_x": 640, "to_y": 300}
{"id": 1, "ok": true, "result": {"applied": false, "pending": 13}}
{"id": 2, "cmd": "input.pending"}
{"id": 2, "ok": true, "result": {"pending": 0}}
{"id": 3, "cmd": "input.text", "text": "hello 👋"}
{"id": 3, "ok": true, "result": {"applied": true, "pending": 0}}
```

### `input.mouse_move`

A `CursorMoved` event: the pointer is now at the position. Hover
behavior, the drag threshold and an armed drag's update follow from
it; in the Reparent and Connect modes, the hovered target
(`query.mode`'s `hovered_node`) updates.

| param | type | required | meaning |
|---|---|---|---|
| `x`, `y` | number | yes | The position. |
| `space` | string | no | `"surface"` (default) or `"canvas"`. |
| `mods` | array | no | Modifiers held for the event. |

### `input.mouse_button`

`MouseInput` events: the click ladder on a left press, the drag
commits and click resolution on a release, the right- and
middle-button gestures and the bound `MouseGesture`s.

| param | type | required | meaning |
|---|---|---|---|
| `button` | string | yes | `"left"`, `"right"` or `"middle"`. |
| `state` | string | yes | `"press"`, `"release"`, or `"click"` (a press, then a release). |
| `x`, `y` | number | no, together | Move the pointer here first; without them the event lands where the pointer already is. |
| `space` | string | no | `"surface"` (default) or `"canvas"`; only with `x`/`y`. |
| `mods` | array | no | Modifiers held for the events. |

A second `click` on the same target within the double-click window
reaches the same `DoubleClickActivate` path a physical double-click
does.

### `input.wheel`

A `MouseWheel` event of `dy` lines (`LineDelta(0, dy)`): the Action
bound to `WheelUp` (positive `dy`) or `WheelDown` (negative), zooming
at the pointer by default — or, while the console is open, its
scrollback.

| param | type | required | meaning |
|---|---|---|---|
| `dy` | number | yes | Lines; positive scrolls up, negative down. `0` has no direction and is `invalid_params` (the wheel arm would read it as down). |
| `x`, `y` | number | no, together | Move the pointer here first, so a zoom focuses there. |
| `space` | string | no | `"surface"` (default) or `"canvas"`; only with `x`/`y`. |
| `mods` | array | no | Modifiers held for the events. |

### `input.key`

`KeyboardInput` for one logical key, through the whole keystroke
ladder: the console, color picker and text editors' steals first,
then the keybind table in the current context, type-to-edit on a
selected label, and the macro and custom-mutation fall-through.
`act.action` skips all of that; this is the command that drives it.

| param | type | required | meaning |
|---|---|---|---|
| `key` | string | yes | One typed character (`"a"`, `"/"`, `"é"`, `"👋"`), or a named key spelled as winit's `NamedKey` spells it, case-exact: `"Enter"`, `"Tab"`, `"Escape"`, `"Backspace"`, `"Delete"`, `"ArrowLeft"`, `"F2"`. These are the names a keybind matches, but a keybind compares them lowercased and the wire does not: `"enter"` is `invalid_params`, `"Enter"` presses the key a `"enter"` binding names. A space is the `Space` key, as winit delivers the space bar. A control character, and a modifier key (`"Shift"`, `"Control"`, `"Alt"`, `"AltGraph"`, `"Super"`, `"Meta"`, `"Hyper"`), are `invalid_params`: winit reports a modifier through a separate modifier-state event rather than as this key, so a modifier press would change nothing a later event reads. Hold modifiers with `mods`. |
| `state` | string | no | `"tap"` (default: a press, then a release), `"press"` or `"release"`. The app acts on presses; a release reaches the same arm a real one does, which does one thing: it ends a pan the released key holds (`query.mode`'s `drag_state.owner` `"key"`, with this `key`). With no such pan it does nothing. |
| `mods` | array | no | Modifiers held for the events: `"key": "z", "mods": ["ctrl"]` is Ctrl+Z. |

### `input.text`

Type a string: one key tap per grapheme cluster, so `"hello 👋"` is
seven taps and a ZWJ sequence or a base with its combining marks is
one. Each reaches whatever has the keyboard — an open text editor or
the console inserts it at the cursor — and with none open, the
keybind table, as typing does. A space, a newline (`\n`, `\r\n` or
`\r`) and a tab arrive as the `Space`, `Enter` and `Tab` keys, as winit
delivers them, so a newline commits the console line and inserts a
line break in the node text editor. Modifiers are those the OS
reports.

| param | type | required | meaning |
|---|---|---|---|
| `text` | string | yes | What to type: 1 to 512 graphemes (two samples each, so one command never outgrows the queue). Any other control character is `invalid_params`. |

### `input.touch`

One finger event, as `WindowEvent::Touch`, through the native touch
path: the touch-gesture recognizer (tap, long-press, one-finger pan,
pinch), with one exception. A finger that *starts a gesture* on a
resize handle in Resize mode drives the handle drag instead, as the
left button would, until it lifts (#160): it resizes, commits with
one undo entry on `ended` or `canceled`, and never reaches the
recognizer, so it does not pan. Another finger landing meanwhile is
ignored. The recognizer measures time on the real clock, so a
long-press is a `started`, a wait, and the finger's next event.

| param | type | required | meaning |
|---|---|---|---|
| `phase` | string | yes | `"started"`, `"moved"`, `"ended"` or `"canceled"` (winit's `TouchPhase::Cancelled`). |
| `pointer_id` | integer | yes | The finger, ≥ 0; one value for one finger's whole stroke. |
| `x`, `y` | number | yes | The position. |
| `space` | string | no | `"surface"` (default) or `"canvas"`. |

### `input.drag`

Sugar for the drag a hand makes, paced one sample per frame: a move
to `from`, a press, `steps` moves evenly spaced and ending exactly on
`to`, and a release — `steps + 3` samples. What it drags is what a
press at `from` arms: a node (moved, with its subtree unless `"alt"`
is held), a section, a resize handle in Resize mode, a label, the
canvas (a pan), or with `"shift"` on empty canvas a rubber-band
select. Where a moved node lands is decided by the throttled drag's
drains, so it lands where the same gesture by hand would.

| param | type | required | meaning |
|---|---|---|---|
| `from_x`, `from_y` | number | yes | The press position. |
| `to_x`, `to_y` | number | yes | The release position. |
| `space` | string | no | Space of both positions: `"surface"` (default) or `"canvas"`. |
| `button` | string | no | `"left"` (default), `"right"` or `"middle"`. |
| `steps` | integer | no | Moves after the press, 1 to 1021; default 10. |
| `mods` | array | no | Modifiers held for every sample. |

Always `applied: false`: the reply arrives before the first sample
lands.

### `input.click_node`

Sugar for a move to the center of a node's model AABB
(`position + size / 2`) — or, with `section`, of that section's AABB
by the tree builder's own rule (`offset` inside the node, filling the
node when it has no `size`) — and a click there. It adds no semantics
of its own: the click is the primitives', so it lands on whatever
the click ladder finds drawn at that point, which is the node unless
something is drawn over it, and does what a click there does (a
second one within the double-click window is a double-click).

| param | type | required | meaning |
|---|---|---|---|
| `node_id` | string | yes | The node. |
| `section` | integer | no | Click this section's center instead. One the node does not have is `invalid_params`. |
| `button` | string | no | `"left"` (default), `"right"` or `"middle"`. |
| `mods` | array | no | Modifiers held for the click: `["shift"]` adds the node to the selection, as a Shift+click does. |

Result: `applied`, `pending`, and `x`, `y` (number), the surface point
clicked. A node the document does not have, one the scene does not
draw — hidden by a folded ancestor, or outside its
`min/max_zoom_to_render` window at the current zoom — and one whose
center is off the surface (a hand cannot click there; bring it into
view first) are `not_found`. The aim is checked against what is drawn
when the command executes, so while injected samples are still
queued, `input.click_node` is refused `busy` (`data.pending`) rather
than clicking later through a camera they may have moved: poll
`input.pending` to 0 and resend. It is never queued, and its
`applied` is always `true`.

### `input.pending`

Params: none. Result: `pending` (integer), the samples still queued.
Poll it to 0 before reading what a paced gesture did.

## `capture` — screenshots and recordings (IPC-07, IPC-08)

Shapes pinned here in full; `mandalactl` and the `mandala-drive` /
`mandala-record` skills build against them. `capture.screenshot` is
implemented (IPC-07), and so is the recording pair below (IPC-08).

**Artifact lifecycle.** The per-item caps (64 Mpx per image, 3600
frames per recording) bound each artifact but not their number. The
**server-chosen** artifacts dir is bounded: it is created on first
use under `$XDG_RUNTIME_DIR` (or the system temp dir when that is
unset) as `mandala-<pid>-artifacts`, owner-only (`0700`) and never
adopted if the name already exists (the next free suffix is taken
instead), scoped to the process, capped at 256 MiB
(`capture::artifacts::BUDGET_BYTES`) with the oldest captures evicted
first — a capture's PNG and sidecar together, and never the newest
one, whose reply has not been read yet — and removed on graceful
exit. A long session cannot silently fill the disk. Artifacts written
to a **client-supplied** `path` / `dir` are the client's to manage;
the server neither caps nor reclaims them (the client asked for that
location explicitly).

### `capture.screenshot` (IPC-07)

Offscreen render of the current document state — never a window grab, so
it works identically under a compositor, Xvfb, or with no display at all
([`--headless`](#headless-and-one-shot-launches)). The frame is drawn by
the very prepare / encode pair the window is drawn with, into a texture
of the surface's own format, and read back; a capture at the default
parameters therefore holds the pixels the window shows. Before drawing,
the camera-geometry rebuild the next frame owes (a zoom the client sent
a moment ago) is run, so the capture shows the current state rather than
the last frame's.

**On the browser** ([Browser](#browser)) the command is served with a
different engine and a narrower shape, and `describe` declares exactly
the per-target parameters and result fields: the page draws the live
view into its canvas — the same prepare / encode pair, at the canvas's
own size and through the live camera — and reads it back in the same
task (`toDataURL`), before that task yields to the browser's rendering
update. What that read-back holds is **established for WebGL's
presentation model only, and unverified for WebGPU** — and WebGPU is
the backend the bundle takes whenever the browser's WebGPU offers an
adapter. A browser that does not define `navigator.gpu`, or whose
WebGPU offers no adapter, runs the bundle on WebGL2 (the latter since
#224, fixing #220), the model where the read is established — though
not yet observed through `capture.screenshot` on this bundle. On WebGL
without `preserveDrawingBuffer`, a same-task `toDataURL` reads the
frame just drawn and a read one frame later reads a cleared buffer
(observed in headless Chromium's WebGL2 on a plain canvas, not on this
bundle), which is why the read-back runs in the draw's task. A WebGPU canvas
presents its current texture at the rendering update instead, and
whether a same-task `toDataURL` returns the frame just submitted or
the previously presented one has not been observed: headless
Chromium's SwiftShader WebGPU reads transparent at every moment, which
is consistent with either answer, and no GPU-backed browser has run
it. Until one has, treat the browser's PNG as unverified, and its
`sidecar` — computed from the scene, not read from pixels — as the
reliable half. There is no file: the PNG comes back
inline as `png_base64`, and the geometry sidecar inline as `sidecar`,
and `mandalactl --ws capture.screenshot <path>` writes both
([Client](#client-mandalactl)). The framing parameters — `path`,
`overwrite`, `width`, `height`, `region`, `space`, `fit`, `scale` —
are not declared there and are refused as stray; `format` and
`sidecar` are. `rendered`/`skipped`, `width`/`height`/`scale`/`camera`
and `settled` mean what they mean below, `settled` reading the
browser's two sources (an animation ticking, injected input queued,
each in real time). `busy` does not arise; the write guards are the
client's, which writes on the page's behalf (below).

| param | type | default | meaning |
|---|---|---|---|
| `path` | string | server-chosen file in the per-session artifacts dir | Absolute path for the PNG. Parent dir must exist. Write guards (same-user trust tier, but an LLM composing a path clobbers by accident far more readily than a human): the server does **not** follow a symlink at `path` (`O_NOFOLLOW`), and refuses to overwrite an existing file **outside** the per-session artifacts dir unless `overwrite: true` is set — an unset `overwrite` hitting an existing external file (or an existing sidecar beside it) is `invalid_params`, and nothing is written. The write is atomic: a failure leaves whatever was at `path` as it was. |
| `overwrite` | boolean | `false` | Allow overwriting an existing file outside the artifacts dir. Inside the artifacts dir, overwrite is always allowed. Only valid with `path`. |
| `width`, `height` | integer | current surface size | The view's size, in window pixels; sent together. Camera center and zoom are preserved, so a larger view shows more canvas. Each side must lie in `1..=16384`, and the image (`width × height × scale²`) must not exceed **67,108,864** pixels (64 Mpx, ~8K square) nor the GPU's largest texture side; a request over any bound is `invalid_params`, never a best-effort giant allocation (degrade-don't-crash — an offscreen texture is a real GPU allocation). |
| `region` | object | — | `{"x","y","w","h"}` in the `space` coordinate space, `w` and `h` positive: frame this rect instead of the current camera view. `width`/`height` still set the view's size; the aspect rule below governs mismatches. A region so large it would need a zoom below the minimum (`0.05`) is `invalid_params` (it could not be shown whole); one so small it needs more than the maximum (`5.0`) is framed at the maximum, still whole. |
| `space` | string | `"canvas"` | Coordinate space of `region`: `"canvas"` (default) or `"surface"` (converted through the camera as of capture time). For `"surface"`, the pixel grid is the **live window surface** (the camera's current surface size), *not* the requested `width`/`height` offscreen target — the region is resolved against the window the human sees, then the camera it derives is rendered into the offscreen target. Only valid with `region`. |
| `fit` | boolean | `false` | Frame the whole drawn document as the load-time fit does: its content box, with the same margin (`FIT_PADDING_FRACTION`, 5% of the view on each side). `invalid_params` with `region`, when the document draws nothing, and when it would need a zoom below the minimum (`0.05`) to fit — framed at the minimum it would be cut off. |
| `scale` | number | `1.0` | Image pixels per view pixel, in `0.1..=4.0`. It changes the resolution and nothing else: the image is `round(width × scale) × round(height × scale)` pixels of the same view, with the same elements in it — zoom windows and connection glyph sizes are decided at the view's zoom, never at `zoom × scale`. Outside the range is `invalid_params`. |
| `format` | string | `"png"` | `"png"` is the only value at pin time; the param exists so adding one is non-breaking. |
| `sidecar` | boolean | `true` | Write the geometry sidecar next to the PNG (`<path>.geometry.json`). |

**Result** (plus `revision` and `document_revision` at the envelope level, as on every reply):

| field | type | meaning |
|---|---|---|
| `rendered` | boolean | Whether the frame was drawn. |
| `skipped` | string | Present exactly when not `rendered`: why (table below). |
| `path` | string | Present exactly when `rendered`: the absolute path of the PNG. |
| `sidecar_path` | string | Present exactly when `rendered` with `sidecar`: the absolute path of the geometry sidecar. |
| `width`, `height` | integer | The image's size, in pixels. |
| `scale` | number | Image pixels per view pixel, as asked. |
| `camera` | object | `{"center_x", "center_y", "zoom"}`: the camera the image was drawn through. A canvas point `p` is at image pixel `(p - center) × zoom × scale + (width, height) / 2`. |
| `settled` | boolean | Whether the app needed no further frame once the capture was taken (below). |

A frame the renderer did **not** draw is not an error and not a
picture: the reply is `ok:true` with `rendered: false` and `skipped`,
and no file is written, because a PNG of a skipped frame would be
indistinguishable from a frame that rendered black. `skipped` names
why:

| `skipped` | meaning |
|---|---|
| `not_started` | The renderer has not been started yet (no scene to draw). |
| `font_system_contended` | The font system was held elsewhere when the frame asked for it. Transient; retry. |
| `prepare_failed` | A glyph prepare failed — the glyph atlas has reached the GPU's texture limit, which can be permanent. The same failure the window logs. |

`settled` is whether the app needed no further frame once the capture
was taken: no animation running, no throttled drag or picker hover
pending, no injected input queued, no camera geometry waiting to
rebuild — `needs_continuation()` false, the first two conditions of
`clock.wait until=settled`, read as that command reads them: under a
virtual clock an animation standing still between steps and samples
waiting for one are owed to later instants, not to this capture. A capture taken unsettled may hold a
frame the next one moves on from. Waiting for the app to settle is
`clock.wait`'s job (IPC-09), not this command's: `capture.screenshot`
never defers its reply.

**Errors.** Beyond `invalid_params`: `internal` when the GPU does not
finish a band of the readback within five seconds
(`renderer::capture::GPU_WAIT`, half the `FreezeWatchdog`'s threshold)
or the disk refuses the write; `message` says which. A node drag in
flight refuses nothing: a capture at any zoom mid-drag shows the
dragged nodes and their connections where the window does (below).
A large capture is long main-thread work — the readback
runs in bands that fit the GPU's buffer limit, and the PNG encodes in
4 MiB slices — so it ticks the watchdog after every band and every
slice, the pattern `clock.step` follows per heartbeat: a capture that
is progressing never reads as a hang, and one that stops does.

**Screen-space overlays.** The FPS readout and the mode-status line
are drawn at their fixed offsets from the top-left corner, scaled by
`scale`. An open modal — the console or the color picker — is laid
out against the window's size, so it is drawn only when the capture
is the window's size (`width`/`height` unset, or equal to it); a
capture of any other size leaves it out rather than drawing it where
it would not be in any window.

**State after a capture is state before it, because the capture never
writes it.** The renderer's camera is never moved: the capture draws
through a camera of its own. A capture framed at another zoom needs
the zoom-dependent roles (connections, their labels, portals — their
glyph sizes are clamped in screen points) projected at that zoom, and
projects them into a scratch scene of the capture's own, drawing one
frame of that scratch over the live scene's zoom-independent roles;
the live scene, its connection cache and the window's shaped glyphs
are read, never written, so `query.camera`, `scene.*` and every hit
test afterwards answer exactly as before, and nothing is projected
back. During a node drag the scratch is projected with the drag's
in-flight offsets — the same ones the window's own frames use — so
the dragged nodes' connections land in the picture where they are on
screen. Nothing is redrawn on the window's behalf.

**Aspect rule.** When `region`'s aspect ratio differs from the
target's, the capture is **extended, never distorted or
letterboxed**: the camera frames the smallest aspect-matching rect
that contains `region`, center-preserved. Everything inside
`region` is guaranteed visible, extra canvas may appear along one
axis, and the pixel↔canvas mapping stays a single uniform zoom —
so the sidecar's rect math remains trivially affine.

**Geometry sidecar** — the pixels↔ids map that lets an agent point
at what it sees:

```jsonc
{
  "protocol": 1,
  "document_revision": 17,                 // the model state these rects were derived from
  "surface": {"width": 1920, "height": 1080, "scale": 1.0},
  "camera": {"center_x": 12.0, "center_y": -340.5, "zoom": 0.75},
  "nodes": [
    {"id": "1.2.3",
     "rect": {"x": 401.0, "y": 92.5, "w": 218.0, "h": 64.0},  // image px, pre-clip
     "clipped": false,                     // true when partially outside the target
     "sections": [{"index": 0, "rect": {"x": 401.0, "y": 92.5, "w": 218.0, "h": 30.0}}]}
  ]
}
```

`document_revision` is the document's revision when the image was
drawn, the same number `document_changed` events and replies carry, so
a pixel map can be tied to the model state it shows. `surface` is the
image (its pixel size and the `scale` it was drawn
at) and `camera` the same object the reply carries; every `rect` is in
the image's pixels, through the formula above. Elements suppressed by
`ZoomVisibility` or folding at capture zoom, and nodes the framing
leaves outside the image, are absent — the sidecar describes the
rendered image, not the document (that is `scene.dump`'s job). A node
or section is present exactly when the renderer drew some part of it.

**Also a console verb.** `screenshot [<path>] [width= height=]
[scale=] [frame=view|fit|x,y,w,h] [space=] [overwrite=on|off]
[sidecar=on|off]` calls the same engine with the same bounds and write
guards, and differs where a person is typing: a relative path is read
against the working directory (as `save` reads its own), no path means
`mandala-<unix ms>.png` there rather than the artifacts dir, and an
open console is left out of the picture it was asked from.
`capture.screenshot` keeps it at the window's size — over IPC the
capture shows what the window shows.

### `capture.record_start` / `capture.record_stop` (IPC-08)

Frame-sequence recording: PNG frames and a `manifest.json` that says
when each was taken. A screenshot proves a state; a recording proves a
*behavior* — an animation's timing, a drag, a hover transition — and the
manifest is what lets a tool replay it at the speed it happened.
**Assembly is not the app's.** The app writes frames; `mandalactl record
assemble <dir>` (a client-side command, no socket) turns a recording
into an animated GIF — the format GitHub shows inline in a PR — or, with
`format=mp4`, into a video through the host's `ffmpeg`. Any other tool
that reads JSON and PNGs can do the same from the manifest. One
recording at a time: `capture.record_start` while one is running is
`invalid_params`, and so is `capture.record_stop` when nothing has
recorded since the last stop.

`capture.record_start` params: `dir` (string, absolute path), `fps`
(integer `1..=60`, default `30`), `max_frames` (integer `1..=3600`,
default `600`), plus `width`/`height`/`region`/`space`/`fit`/`scale`
exactly as `capture.screenshot` (the same bounds, the same refusals;
`scale` multiplies the frame's pixels, not what it shows). Every frame
of a recording is the same size: `width`/`height` unset means the
window's size *when the recording starts*, and a window resized
afterwards does not change it. The bounds are refusals, as for a
screenshot: `fps` or `max_frames` outside its range is `invalid_params`
naming it, never a clamp.

`dir` is where the frames go. It must be an absolute path, not a
symlink, and either not there yet (its parent must exist; one level is
made, never a tree, so a typo in the middle of a path is a refusal) or
an **empty** directory: a recording never writes among files it did not
make, so it needs no `overwrite`. Unset, it is a fresh `recording-<n>`
directory in the per-session artifacts dir, counted into that dir's
budget when the recording ends.

Result: `{"dir", "manifest_path", "width", "height", "fps",
"max_frames"}`, plus `previous` when an earlier recording had ended on its
own and no `capture.record_stop` had collected it: starting a recording
discards that report, so it is handed back here — the same object
`capture.record_stop` would have returned, or `{"error"}` for a
recording that failed — and never dropped without a word. `manifest_path` is where `manifest.json` will be — the
file is written whenever frames are, and complete when the recording
ends. **The first frame is taken before the reply**, so it shows the
state the next command acts on; a first frame the renderer cannot draw
is a gap, like any other, and the recording begins regardless.

An in-flight recording is **app state, not session state**: it keeps
capturing across a client disconnect, and a reconnecting controller
stops it with `capture.record_stop`. It ends by itself at `max_frames`,
when its PNGs pass the artifacts dir's budget (256 MiB, a recording in
that dir only), when the renderer has drawn nothing for 100 frames in
a row, or when the app exits (finished first, its frames and manifest
on disk — except for a recording in the artifacts dir, which is dropped
unfinished because the dir goes with the exit) — and a recording that ended on its own is
kept until `capture.record_stop` collects its report, so an agent that
looked away still finds where its frames went. A recording started
after that discards the report; the files stay.

**Cadence.** An active recording is a **continuation/timer source**:
on an otherwise static scene the loop would park in `ControlFlow::Wait`
and no heartbeat would fire, so nothing would be captured until an
unrelated event woke it. While a recording runs, the loop instead
parks in `WaitUntil(next frame deadline)` — the same idle-timer shape
the FPS grace already uses — so frames land at the requested cadence
with the canvas idle. The **frame-emission rule** is pinned so the
manifest is deterministic: a frame is due each time the capture clock
crosses a `1000/fps` boundary (the boundaries are the multiples of
`1000/fps` ms from the start, counted as whole numbers), **not** once
per heartbeat. A heartbeat that finds several boundaries crossed — it
overran — takes **one** frame, for the latest, and counts the others as
missed. So a second at `fps=30` is exactly 30 boundaries however many
heartbeats ran. A recording started under `clock.set_mode virtual`
runs the same rule against virtual time: the loop sets no wake for it,
and `clock.step` drives the frames, stopping the clock on every
boundary, so `clock.step ms=1000` at `fps=30` emits exactly 30 and none
is ever late. A stepped boundary is rounded **up to a whole
millisecond** (at `fps=30`: 0, 34, 67, 100, … ms), each from its own
index so nothing accumulates: the virtual clock reads whole
milliseconds, as every animation stamp does, so a frame taken there
shows exactly the instant its `t_ms` names. The manifest's `pacing` says which: `"realtime"` or
`"stepped"`, fixed when the recording starts — the clock's mode cannot
change while one runs. While the loop idles under a virtual clock there
is no next-frame deadline to fit an idle encode before, so it spends at
most one heartbeat's worth of wall time (16 ms) per idle iteration on
it.

**Costs, and what is never done in the loop.** Taking a frame is one
render and one readback, ticking the `FreezeWatchdog` as a screenshot
does. The PNG encode is *not* in the frame: raw frames wait in memory
and are encoded (a) in time the loop would otherwise sleep through —
only when no animation frame, redraw or queued command is owed, and
only as far as the time before the next boundary reaches, judged from
the measured cost of an encode (the first, unmeasured, is a probe that
may be late by its own cost, once), the manifest rewritten after, at most every two seconds (it grows
with the recording, and a rewrite is an `fsync`), so what landed is
named — (b) when 256 MiB of them have piled up (a stall the loop pays
rarely, ticking the watchdog per 4 MiB chunk, and the boundaries it
costs are `dropped_frames`), and (c) when the recording ends, which is
what `capture.record_stop` pays for whatever (a) did not reach. A
heavy drag therefore records at whatever rate the machine sustains,
reports the frames it could not take, and never wedges the loop.

`capture.record_stop` params: none. Result: `{"dir", "manifest_path",
"frames", "dropped_frames", "duration_ms", "stopped", "bytes", "width",
"height", "fps"}` — `dir` and `manifest_path` as above, `frames` how
many frames were taken, `dropped_frames` how many were due and not,
`duration_ms` how long the recording ran, `bytes` what the frames and
the manifest take on disk, `width`, `height` and `fps` as `record_start`
reported them — and `stopped` says why it ended: `"client"` (this
command), `"max_frames"` or `"disk_budget"`. (The manifest's `stopped`
has two more values this command cannot return: `"shutdown"`, written
when the app exits mid-recording, after which no command can ask, and
`"error"`.) A recording that could not be written, or that the renderer
could not draw for 100 frames in a row, ends with `stopped: "error"` in
its manifest and this command answers `internal`, naming the directory
whose frames are usable. A frame that cannot be *framed* this time — a
`fit` whose document has since lost what it framed, a `surface` region
that now needs a zoom below the minimum — is a gap with reason
`invalid`, counted toward those 100, so a framing that works again
resumes the recording and one that never does ends it. It does the encode, so it is
long main-thread work for a long recording, ticking the watchdog the
way a large screenshot does.

Frames are `frame_%05d.png`, numbered by frame *taken* (a gap leaves no
hole in the numbering), plus `manifest.json`:

```jsonc
{"protocol": 1, "fps": 30, "pacing": "realtime",
 "width": 1280, "height": 720, "scale": 1.0,
 "complete": true, "stopped": "client", "duration_ms": 3012.5,
 "frames": [{"index": 0, "t_ms": 0.0, "file": "frame_00000.png",
             "document_revision": 17,
             "camera": {"center_x": 12.0, "center_y": -340.5, "zoom": 0.75}}],
 "dropped_frames": 2,
 "gaps": [{"t_ms": 1033.333, "frames": 1, "reason": "overrun"}]}
```

`t_ms` is capture-clock time, measured from the start, **when the frame
was taken**: the app clock's reading in `realtime` pacing (so a
consumer that paces by it replays what happened, stalls included) and,
in `stepped` pacing, the frame's `1000/fps` boundary rounded up to a
whole millisecond, exactly — the step stopped the clock there to take
it.
`duration_ms` is how long the recording ran, and with the last frame's
`t_ms` is the last frame's display time. `complete` is `false` in the
copy written while recording (a recording the app never finished — one
that died after its first idle encode or flush — is still readable, and
tells itself from one it did) and `true` once
finished; `stopped` is absent until then. `document_revision` and
`camera` are as in the geometry sidecar, so a frame ties to the model
state and view it shows.

`gaps` is every stretch with no frame, **never a silent drop**:
`t_ms` is when the first missing frame was due, `frames` how many,
`reason` why — `overrun` (the loop was late for the boundary, or the
recording ended with boundaries crossed since the last heartbeat),
`invalid` (above), or the renderer's skip word
(`font_system_contended`, `not_started`, `prepare_failed`). A node
drag is no gap: a recording framed at another zoom draws every frame
of it, dragged connections included, as a screenshot does. `t_ms` is
the boundary's time, not the moment it was noticed.
`dropped_frames` is their sum, and the manifest's `frames` names only
files that exist.

**Also a console verb.** `record start [dir=] [fps=] [max_frames=]
[width= height=] [scale=] [frame=view|fit|x,y,w,h] [space=]` and `record stop`
call the same engine with the same bounds and differ where a person is
typing: a relative `dir` is read against the working directory, no
`dir` means `mandala-recording-<unix ms>/` there rather than the
artifacts dir, and an open console is left out of the frames.

## `clock` — determinism (IPC-09)

Time virtualization over the one app clock, plus the settling
primitive every capture workflow needs. Design and the per-reader
disposition: [`work_plans/LLM_IPC.md`](../work_plans/LLM_IPC.md) §D9.

**One clock for what behaves, the wall clock for what measures.**
Everything the app *does* by time reads the app clock: the animation
tick, the drag pause on it, double-click and long-press timing, the
start of an animated `OnClick` trigger or keybound mutation, a
recording's frame times, `query.animations`, `ping`. Everything that
*measures* — the FPS overlay, the `FreezeWatchdog`, the drag
throttle's frame budget, an encode's cost, a `clock.wait` deadline —
reads the wall clock in both modes, because a stepped or frozen clock
would hide a stall rather than time it.

**One continuous axis.** Switching modes never moves the reading:
virtual time starts where real time stood, rounded up to the next
whole millisecond (so whole-millisecond steps keep every reading
whole), and real time resumes from where virtual time was left, at the
wall clock's rate. Nothing is rebased on a switch — an animation's
start, the last click, a touch, a recording all stay on the one axis
— so engaging virtual time mid-animation freezes the animation where
it stands, and the next step resumes it.

**The mode is app state.** It outlives the controller that set it,
like a recording, so `mandalactl`'s one command per connection can
drive it. An app left in virtual mode stays there: its animations stand
still and, with time frozen, two clicks any distance apart in wall time
are a double-click. `clock.set_mode real`, or the console's `clock
real`, gives it back. A launch starts real, except a `--headless` or
`--screenshot` one, which starts virtual
([Headless and one-shot launches](#headless-and-one-shot-launches)).

**What a frame does in each mode.** A frame has an input-driven half —
the throttled drag and picker hover drains, the rubber band, the
camera-geometry rebuild — and a time-driven half: one sample of the
injected-input queue (§`input`), the animation tick, a recording's
frame. In real mode every event-loop iteration runs both, exactly as
before this family existed. In virtual mode the loop's own iterations
run the input-driven half alone (a human's drag still lands; the
animations stand still), and the time-driven half runs inside
`clock.step`, at the virtual instants the step names. Under a virtual
clock a throttled drag drains on every frame it has something pending:
the throttle trades drain frequency for a real frame budget, and a
virtual heartbeat has none to protect.

- **`clock.status`** — no params. Result: `mode` (`"real"` /
  `"virtual"`) and `now_ms` (number), the app clock. Needs no app:
  before the window exists the clock is real and reads the wall clock,
  as a new clock does.
- **`clock.set_mode`** — params: `mode` (`"real"` / `"virtual"`,
  case-exact; anything else is `invalid_params` naming `mode`).
  Result: `mode`, `now_ms` — the reading, which the switch did not
  move. Setting the mode the clock is already in is a success that
  changes nothing. **Refused while a recording runs** (`invalid_params`
  naming `mode`): a recording's `pacing` is fixed for its whole length,
  so a switch would make its frame times mean two things. **Refused
  inside a running step** (`busy`, as a nested step is): a `clock real`
  console line typed by an injected key the step itself landed would
  otherwise leave the step's remaining heartbeats on a clock the wall
  moves. Changes no pixels.
- **`clock.step`** — params: exactly one of `ms` (integer,
  `1..=1_600_000`) or `frames` (integer, `1..=100_000`) — neither, or
  both, is `invalid_params` (naming `ms` for neither, `frames` for
  both), and so is a value outside its range: the client chunks.
  **Needs a virtual clock**: on a real one the step is `invalid_params`
  naming `cmd`, the recorder's state-refusal precedent. Virtual time
  advances at a pinned cadence of **one heartbeat per 16 ms** (≈60 fps,
  the rate the app targets), so the two spellings are the same work:
  `frames = n` runs `n` heartbeats 16 ms apart and advances `16 × n`
  ms; `ms = m` runs `ceil(m / 16)` heartbeats, 16 ms apart with the last
  landing on `m` **exactly**, so a step never overshoots. Each
  heartbeat is the whole frame — one injected sample, the drains, the
  animation tick, a recording's frame — run synchronously (deterministic
  catch-up is the point), and the loop **ticks the `FreezeWatchdog`
  once per heartbeat inside the step**, so the run is watchdog-safe
  *by construction* (a genuinely stuck single heartbeat still trips the
  10 s threshold; the bound is a responsiveness limit, not the safety
  mechanism). Each heartbeat costs the wall time of the frame it runs —
  an animation's tick rebuilds the scene, as it does in real time — so a
  long step over a large animated scene is long main-thread work, and
  the client's patience must match it (`mandalactl --timeout`); other
  commands queue behind it. **A running recording's frame boundaries are stopped at
  too**: one that falls between two heartbeats gets a tick of its own,
  on the boundary exactly — the drains, the tick and the frame, but no
  injected sample, so recording a gesture never changes when its
  samples land — and one that falls on a heartbeat is that heartbeat's.
  Result: `now_ms` (the virtual clock after the step) and `frames_run`
  (the cadence heartbeats: `frames`, or `ceil(ms / 16)`; a boundary
  tick is the recorder's to count, in its frames). A step that ran
  requests a redraw, so the window shows where it left the app. A step
  asked for from inside a running one — a `clock step` console line
  typed by an injected key the step itself landed — is `busy`.

- **`clock.wait`** — params: `until` (`"settled"` /
  `"animations_complete"`), `timeout_ms` (integer, default `10_000`,
  `1..=60_000`; a value outside it is `invalid_params`, as IPC-07 and
  IPC-08 made their bounds; always **wall-clock** milliseconds — the
  deadline is a client-liveness guard, so a frozen virtual clock must
  not disable it). **Exclusive per controller**: only one `clock.wait`
  may be pending at a time; a second while one is in flight is
  rejected `busy` (there is no named use for concurrent waits, and
  unbounded pending waits would be a memory/scan-cost vector). Its `id`
  stays in flight until the wait resolves or is canceled — a client
  must not reuse that `id` before then. **Deferred reply**: the wait is
  held in the controller's session and evaluated at the end of every
  event-loop iteration — after the redraw that iteration owes has been
  asked for, so a pixels-affecting command of the same iteration counts
  as a frame still owed — and after every window event, the frame's
  render among them; other commands keep executing while it is pending;
  a disconnect cancels it; a pending wait is **not a barrier** (see
  [Reply](#reply)). A `clock.step` runs as one command: the wait sees
  the state it left, not its intermediate heartbeats. While a wait is
  pending and the loop would otherwise park in `ControlFlow::Wait`, it
  parks in `WaitUntil(deadline)` instead — the FPS idle-grace precedent
  in `run_native.rs` — so a deadline fires even when no heartbeat would
  otherwise run, without polling. A controller paused on a full reply
  queue ([Threading contract](#threading-contract-wire-visible-consequences))
  is evaluated once it resumes, so a wait's reply never overtakes the
  one held for it. On success: `elapsed_ms` (integer), the wall-clock milliseconds the
  wait took.
  On deadline: `ok:false`, code `timeout`, `data.elapsed_ms`, and a
  message naming what was still owed. Like every reply from a command
  that ran with the app up, it carries `revision` and
  `document_revision`, as they stand when it is answered. Needs the
  app: `no_document` before the window exists.

Wait conditions, defined against the frame (above) and CONCEPTS §5
"Event loop and `drain_frame`":

- **`animations_complete`** — `MindMapDocument` reports no active
  animations. Under a virtual clock animations advance only by
  `clock.step`, so a wait for this with no step to come times out.
- **`settled`** — at an evaluation point, all of: (a)
  `needs_continuation()` is false (no throttled drag pending, no
  picker-hover pending, no dirty connection geometry — and, **in real
  time**, no active animation); (b) **in real time**, no IPC-injected
  input remains queued; (c) **no redraw is owed**: every redraw the
  app has asked for — every pixels-affecting command's, and every
  input handler's — has been drawn by a render that ran after it; and
  so (d) the renderer has **rendered** the frame reflecting the most
  recent pixels-affecting command. In plain terms: **the rendered scene
  is the final consequence of every command the agent has sent so
  far**, and capturing now is race-free *with respect to the agent's
  own commands*. (It is not fenced against a concurrent human — see
  the scope note below.)

  **Under a virtual clock, "final" means "at this instant".** An
  animation standing still between steps, and injected samples waiting
  for the heartbeats that land them, are owed to *future* virtual
  instants only `clock.step` reaches; with time frozen nothing more
  will happen until it does, so neither keeps the app from settling.
  That is what makes `clock.step ms=500` → `clock.wait until=settled` →
  `capture.screenshot` capture an animation exactly 500 ms in.

  **Rendered, not on-screen-presented.** Condition (d) is deliberately
  about the *render* completing, not a swapchain *present*.
  `capture.screenshot` is offscreen and must work under an
  occluded/minimized window and with no display at all
  ([`--headless`](#headless-and-one-shot-launches)) — where nothing ever
  presents. Gating `settled` on an on-screen present would make every
  capture workflow time out in exactly the containerized/Xvfb
  environment the epic targets. So a render counts as drawn once its CPU
  half has run — every buffer current, every glyph prepared — whether or
  not the surface then had a texture to present into. With no window a
  frame draws nothing and counts as drawn: a capture prepares and draws
  its own. A frame the renderer *skipped* (the font system contended, a
  failed prepare) has not drawn, and the redraw stays owed. One limit no
  protocol can lift: the render runs when winit delivers the
  `RedrawRequested` the app asked for, and a platform that withholds it
  from a hidden window withholds the render too; the wait then times out
  saying the latest frame was not drawn. A headless app has no such
  dependency: there is no platform between the frame it asks for and the
  frame it draws.

  **The arbiter is a redraw ledger, not the document revision.** The
  naive "rendered at the current *document* revision" is insufficient:
  view-only commands — a wheel zoom, an `act.action` pan, a
  selection-highlight change — repaint without mutating the document,
  so a stale pre-command frame would already match the document
  revision and `settled` would resolve before the repaint renders. The
  app therefore counts the redraws it **asks for** — every
  pixels-affecting command requests one (see
  [`work_plans/LLM_IPC.md`](../work_plans/LLM_IPC.md) §D2 — the IPC
  drain mirrors the existing winit handlers' `request_redraw`
  exactly), and so does every input handler — and the renders that
  **drew** them; `settled` holds only once the two agree. The
  envelope's `revision`
  ([`events`](#events--event-stream--revision-ipc-14)) orders replies
  against events; what `settled` needs beyond it is *drawn*, which
  the ledger records where the asks are made.

  **Scope: `settled` fences the agent, not the human.** The
  "race-free" guarantee is only against the agent's own prior
  commands. In the "human watches while agent drives" workflow a
  human can move the mouse, type, or pan the canvas between the
  `clock.wait` reply and the following `capture.screenshot`, and that
  input is *not* fenced (nor could it be). A human's continuous
  camera-pan drag is an inline per-frame gesture (CODE_CONVENTIONS §3
  carve-out) that can sit between two of its cursor events with
  `needs_continuation()` false and every redraw drawn, so `settled`
  can resolve while the human is mid-gesture. For byte-reproducible
  captures the agent should drive an instance with no concurrent
  human (the container/Xvfb case); the guarantee is exact there.

  `MindMapDocument.dirty` deliberately plays no role here: despite
  the name it is the *unsaved-changes* flag — set by every document
  setter, cleared only at construction and on a successful `save`,
  read by the `open` / `new` verbs' guards — not a rebuild-pending
  signal. (A design-time correction: issue #61's sketch assumed
  otherwise, as did CONCEPTS' "Dirty flag" entry, fixed alongside
  this document. A settled condition gated on it would never resolve
  after any unsaved mutation.)

  One residual non-determinism to know about: with the FPS overlay
  on (a native `fps` diagnostic, off by default), its idle-grace flip
  repaints the overlay region *after* `settled` resolves, so two
  "settled" screenshots can differ in that corner. A diagnostic must
  not become observable behavior (the reason it isn't a continuation
  source), so `settled` does not wait for it — capture with the
  overlay off for byte-identical frames.

There is deliberately **no** third `"idle"` condition: once settled,
the loop parks by construction (`ControlFlow::Wait`), and the only
residual timer is the FPS overlay's idle-grace flip — a diagnostic
that observes behavior and must not become observable behavior.

**The determinism contract.** With a virtual clock engaged before the
script starts, the same document and the same command sequence leave
the same laid-out scene: `scene.dump` is byte-identical across runs,
whatever the wall clock did meanwhile and whenever virtual time began
(`test_the_same_stepped_script_twice_dumps_byte_identical_scenes`
holds two independently built documents — every `HashMap` in each
seeded apart — to it). The one `HashMap`-order dependence the hunt for
this contract found was the inline-macro collision winner, which is
now the lowest node id in every process (`format/macros.md`). What the
contract does not cover: `now_ms` itself, whose virtual origin is the
real reading at engagement, and anything a concurrent human does.

**Also a console verb.** `clock status`, `clock real`, `clock
virtual`, and `clock step ms=<n>` / `clock step frames=<n>` reach the
same switch and the same step, with the same bounds and refusals in
the console's words. The console shell is native; the step's schedule
is not (`app/step.rs`, `CLAUDE.md` "Dual-target status"): the browser
runs its own frame at each heartbeat — one injected sample, then the
animation tick — and the `clock` family is served there over the
[browser transport](#browser), with the page's own frames standing
still while the clock is virtual.

## `events` — event stream + revision (IPC-14)

Push notifications, opt-in per class, and the two counters that order
them against replies. A connection starts with **no subscriptions**:
an agent that polls never receives a frame it did not ask for, and a
new controller starts clean (subscriptions and queued events reset
with the connection; the counters do not, because they are the
process's).

- `events.subscribe` — params: `classes` (array of class names,
  required, at least one), `verbose` (boolean, default `false`).
  Adds the classes. Result: `subscribed` (every class now
  subscribed, name order), `verbose`, `revision`,
  `document_revision`. **Changes made before the command are not
  reported**: a class added now speaks from here on, never the
  history. `verbose` applies to the whole connection and is cleared
  by an `events.unsubscribe` that leaves nothing subscribed. An
  unknown class is `invalid_params` naming `classes` and listing the
  valid ones; so is `hello`, `shutdown` or `connection_rejected`,
  which are always delivered. Needs the app up, like `act.*`:
  `no_document` before the window exists.
- `events.unsubscribe` — params: `classes` (array, optional; omit for
  all). Drops the classes and any event of theirs still waiting to
  be delivered. Result: `subscribed`, the classes that remain.
- `events.revision` — no params. Result: `revision`,
  `document_revision`. Reads the counters; subscribes to nothing.

`mandalactl events.subscribe <classes> [--count n]` is the shell's way
in: a subscription lives as long as its connection, so that one verb
stays connected after the subscribing reply and prints each event as it
arrives (one line, the class and its `data`; `--json` prints the frame
as received) until `--count` events have been printed, the app sends
`shutdown`, or stdout closes. `shutdown` ends it with exit `0`; a
connection that dies without it is exit `2`.

### The two counters

| counter | where | advances on |
|---|---|---|
| `document_revision` | `MindMapDocument::revision()`; every reply, every `document_*` event, the geometry sidecar | every committed model change: each document setter (the one place that sets the unsaved flag also advances it), each undo, each custom mutation, each animation that commits, each `open` / `new`. Never on a query, a screenshot, a preview, a selection or the camera. |
| `revision` | the event layer; every reply, every event | once per *observation* in which any event class had something to report — so a zoom or a selection moves it while `document_revision` stays put. |

Both are **monotonic per process**, not per document: `open` and
`new` carry the document counter on (the replacement is itself one
more change) and bump a `generation`, so neither number ever reads
lower than before. Both start at zero, and the first observation of
a process only records a baseline.

An *observation* is the unit: the app is looked at after each
command (so a reply's counters include that command's own effect)
and, while anything is subscribed, once per event-loop iteration
between the user's input and the first command of the drain (so a
change the user made is not reported as the agent's). Two changes
between two observations advance `revision` once. With
nothing subscribed, a change the user makes is not observed until the
next command looks, so `revision` is a lower bound on how many
things happened, never a count — `document_revision` is the exact
one.

### Classes

Every event's `data` carries `revision`: the counter as of the
observation that produced it.

| class | `data` besides `revision` | fires when |
|---|---|---|
| `document_changed` | `document_revision`, `source` | the document's revision advanced |
| `document_replaced` | `document_revision`, `generation`, `path` | an `open` or `new` replaced the document; `path` is the new file path, `null` when unbound |
| `selection_changed` | `selection` (as `query.selection` reports it) | the selection differs |
| `camera_changed` | `camera` (as `query.camera` reports it) | the camera's center, zoom or surface size differs |
| `mode_changed` | `mode` (the interaction mode, as `query.mode` reports it under `interaction_mode`) | the interaction mode differs |
| `animation_started` | `animations`: `[{id, target_id, section_idx, mutation_id}]` | animations began; `id` names the running animation and never changes while it runs, so an animation re-triggered after it ended is a new `id` under the same `(target_id, section_idx, mutation_id)` |
| `animation_completed` | `animations`, as above | animations committed or ended, by `id`. When one observation holds both the end of an animation and its re-trigger, `animation_completed` is sent first |

`source` says which funnel was driving when the document's revision
last advanced: `action` (`dispatch_action`: a key binding, a gesture,
a macro `Action` step), `console` (a console verb), `mutation` (a
custom mutation, or an animated one committing), `undo`, `input`
(direct manipulation that reaches the model with no funnel to name:
a drag, a text-edit commit), or `ipc` — any change made while an
agent's command was executing. The innermost declaration wins, so an
undo reached through `Action::Undo` reports `undo`; an agent's
`act.action` reports `ipc`. An `open` or `new` typed at the console is
`console`.

`document_changed` and `document_replaced` both fire for a swap, and
the swap's `document_revision` is the same in both.

### Delivery discipline

- Events are produced on the main thread, at observation points, and
  never during a mutation: nothing re-enters the code that is
  changing the document.
- **Coalesced per drain.** All an observation window's changes of one
  class are one event, with the final state. A 200-step macro is one
  `document_changed` carrying the final `document_revision`. The two
  `animation_*` classes carry *deltas* (what began or ended since the
  last look), so coalescing them concatenates their `animations`
  rather than keeping the newest: no start or completion is lost to
  a later one. A delta is folded into the newest queued event of its
  class only when no event of the other delta class was queued after
  it, so a re-trigger never reaches the client ahead of the
  completion it follows. With `verbose`, every observation is its own event and
  a macro **an agent runs** (`act.macro`) is observed **between its
  steps** as well, so the same macro yields an event per step that
  changed something. A macro a person triggers from the keyboard is
  one observation like any other input: the stream's session is the
  agent's, and the keyboard path has no way to reach it.
- Events leave **after the replies** of the drain that caused them,
  never before: an agent reads `{"id": 7, "ok": true, "revision": 12}`
  and then `{"event": "document_changed", "data": {"revision": 12, …}}`.
- Events never pause execution. They have a **lane of their own** in
  the controller's outbound queue: at most **64** change events wait
  there at once, and the queue is sized for 256 replies on top of
  them, so an event can never take the slot a reply needs and only
  replies can make the server pause. When the lane is full or the
  queue has no room, the event stays queued on the main thread and a
  newer event of its class replaces it (deltas fold instead, see
  above), so a client that is slow to read loses *stale* events,
  never its session; the outbound thread wakes the loop when there is
  room. (A queue is capped at 4096 events and
  drops the oldest, which only a `verbose` subscription, or a client
  that stops reading while animations alternate between starting and
  ending, can reach.) Events already in the queue are not recalled.
- No subscription, no observation between commands: the event loop's
  tap is a branch on the empty subscription set.

Reserved event classes: `hello`, `shutdown`, `connection_rejected`
(the three transport-level classes, never subscription-gated;
`connection_rejected` is delivered to the refused connection, not the
controller), `document_changed`, `document_replaced`,
`selection_changed`, `camera_changed`, `mode_changed`,
`animation_started`, `animation_completed`. `shutdown` is the
"app is closing" notice. Widening this list is breaking, like any
enum-valued field (see
[Handshake and versioning](#handshake-and-versioning)).

**Not here.** Tree-level (Baumhard `EventSubscriber`) events wait for
the #43 reshape; nothing above depends on it, because the tap points
are the app's funnels and the observation, not the walkers. A
`log_line` class for degrades outside any command's window is
possible now that #45 kept `warn!` and `error!` in release builds,
and is not shipped: it is a follow-up with its own design (what a
flood of per-frame degrades does to the queue is the first question).

## Client: `mandalactl`

`mandalactl` (`crates/mandalactl`) is the command-line client for this
protocol: one invocation is one connection, one command and its reply.
It adds no wire vocabulary, so what it does is what this document says
a client does:

- **Command names and parameters are the wire's.** `mandalactl
  scene.hit_test 412 96 space=surface` sends `{"id":1,"cmd":"scene.hit_test",
  "x":412,"y":96,"space":"surface"}`. A parameter is typed from the type
  word `describe` publishes for it (`x=412` the number, `text=412` the
  string); a command the client has no verb for is typed from the
  connected app's own `describe`, so a new command is callable before a
  client release. `mandalactl raw '<json>'` (or `raw -` on stdin) sends one
  frame as written, supplying an `id` when it has none.
- **Socket discovery:** `--ipc <path>`, else `$MANDALA_IPC`, else a usage
  error that says how to launch the app. **A browser app** is named with
  `--ws <url>` (the relay's controller URL), else `$MANDALA_WS`; the two
  flags are exclusive, and a shell with both variables set is a usage
  error naming them, since it names two apps. The client is the relay
  too: `mandalactl bridge [listen=] [token=]` prints `listen`, `app_url`
  and `controller_url` (`path: value` lines, or `--json`) and relays
  until stopped ([Browser](#browser)). Over `--ws` every command is the
  same, with one client-side difference: `capture.screenshot <path>`
  requires the path, takes `path` and `overwrite` off the request, and
  writes the PNG the page returns inline — and its sidecar, as
  `<path>.geometry.json` — with the native writer's guards: a symlink
  at either path is never followed, `overwrite=true` included; an
  existing file is refused unless `overwrite=true`; and the write is
  atomic (a sibling file, synced, then moved or linked into place), so
  a failure leaves what was there. A refusal is exit `1`, and the
  capture may be repeated; its output,
  `--json` included, then names `path` and `sidecar_path` as a native
  reply does, in place of `png_base64` and `sidecar`. A `rendered: false`
  reply writes nothing. The framing parameters are sent as given, for
  the page to refuse as the stray parameters they are there.
- **Paths:** a relative path argument (`path=`, `to_file=`) is made
  absolute against the client's working directory before it is sent,
  because the server requires absolute paths and has no working directory
  of the client's to resolve against. Which parameters are paths comes
  from the verb table, and for a command typed from `describe` from the
  registry's own convention that a path parameter's `doc` opens with
  "Absolute path". An empty value is sent as it is, for the app to
  refuse. `name:=json` and `raw` send what was written.
- **The handshake is checked before anything is sent.** The first frame
  must be `hello` with `"app": "mandala"` and `"protocol"` equal to the
  client's own (currently 1); anything else is refused unless `--force`
  ([Handshake and versioning](#handshake-and-versioning)). A
  `connection_rejected` first frame with `"reason": "busy"` is retried for
  two seconds, because the departure of the previous controller is noticed
  "a moment after the socket closes" ([Single controller](#single-controller))
  and two invocations in a row would otherwise race it; a slot still held
  after that is reported as another client holding it. Any other `reason`
  is reported as it is and not retried: nothing says it will pass —
  the relay's `no_app` is reported with its remedy (open the page).
- **Every wait has a deadline** (`--timeout`, default 15 s, longer than the
  10 s `FreezeWatchdog` threshold): the connect, the greeting, each write
  and each reply. The deadline is the whole line's, re-armed before every
  read and write, so a peer that trickles bytes cannot stretch it. The
  one exception is a `clock.wait` reply — named on the command line,
  sent through `raw`, or `--wait-settled`'s — which is waited on at
  least to the wait's own `timeout_ms` (the server's default `10000`
  when it names none, its bound `60000` at most) plus 2 s: the app
  answers a wait that runs out with a structured `timeout`, and a client
  that gave up first would report exit `4` instead of the app's answer.
  A reply is matched by `id`, with the `null`-id `parse_error` taken
  as the answer to the one request in flight; events the client did not
  subscribe to are skipped, and `shutdown` ends the invocation.
- **`events.subscribe` is the one verb that stays connected.** A
  subscription lives as long as its connection, so after the subscribing
  reply the invocation keeps reading and prints each event as it arrives
  (see [`events`](#events--event-stream--revision-ipc-14)); `--count n`
  ends it after `n` events, `shutdown` ends it with exit `0`, and a
  connection that dies without `shutdown` is exit `2`. `--count` is
  refused (exit 3) on any other command.
- **`--wait-settled`** first asks `describe command=clock.wait`, so an
  app without the clock family is reported (exit 1) *before* the command
  is sent: a wait found missing afterwards would fail a command that had
  succeeded, and a caller branching on the code would repeat a mutation.
  After a command that succeeds it sends `clock.wait` with `until:
  "settled"` and `timeout_ms` equal to the client's timeout (clamped to
  the server's `1..=60000`), on the same connection, and reports its
  result too. If that wait fails once the command has succeeded (a
  `timeout` reply, the connection lost), the invocation exits `4` with the
  command's result already printed and a message that the command ran and
  must not be repeated. The flag is refused up front (exit 3), through `raw`
  as well, for a command the client knows only reads.
- **Exit codes:** `0` the reply was `ok:true` (warnings do not change it);
  `1` it was `ok:false`; `2` the app could not be reached or the request
  did not go out whole (not running, stale socket, busy or refused,
  wrong protocol), so nothing ran; `3` the command line was wrong and
  nothing was sent; `4` the request went out whole and no reply was read
  (the deadline passed, the connection closed, `shutdown` arrived, or the
  reply was unreadable), so it may have run. The split exists for the
  caller's retry: `2` is safe to repeat, `4` calls for a read of the app's
  state first. The boundary is the end of the request's JSON body, not
  its newline: a body cut short cannot decode, so nothing runs, but the
  server also runs a final line its peer closes without terminating
  (`read_capped_line`), so a send that fails on the one-byte terminator
  after the whole body went out is a `4`. Over `--ws` the boundary is
  the whole WebSocket frame, header and mask included: a frame cut
  short names a length it never reaches and is never delivered, so a
  send that stopped inside it is a `2`, and one that failed after it a
  `4`.
  Errors go to stderr as `error: <code>: <message>`, warnings as
  `warning: <code>: <message>`. `--json` prints each reply line exactly as
  received, one per request.

The client's table of commands (`verbs::VERBS`) is held to the registry in
both directions by tests in the app crate, so it is as honest as `describe`
is against this document.

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
