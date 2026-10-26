# LLM IPC — architecture design (IPC-01)

The keystone document for EPIC #60: an LLM-friendly IPC interface so
an agent can drive, inspect, screenshot, and record the running app
— "give Claude a better feedback loop" (README). This file pins the
decisions every downstream issue builds against and carries the
rationale and the rejected alternatives; the wire-level contract
those decisions produce is [`format/ipc.md`](../format/ipc.md).

**If you are implementing IPC-02 … IPC-16**: your issue body plus
`format/ipc.md` plus this file are intended to be sufficient. Read
in this order: your issue → `format/ipc.md` (the sections your
issue owns) → the decision sections here that name your issue →
CONCEPTS §5 "Action dispatch" → CODE_CONVENTIONS §3/§4/§7/§9.

**If you are changing a decision**: decisions here were pinned as a
set — transport, threading, envelope, and trust interlock. Change
the decision section here, ripple `format/ipc.md` (bump `protocol`
if breaking), and check the [amendment ledger](#amendment-ledger)
for the convention text that moves with it.

## What already exists (the seams this design builds on)

The codebase was shaped for this trajectory; IPC adds **no parallel
control path** (CODE_CONVENTIONS §2). The seams, verified at design
time:

- **`dispatch_action`** — the single funnel: "Mouse, keyboard, the
  future macro runtime, and any plugin host all reach the same
  arms" (CONCEPTS §5). `Action` is serde-JSON with a compiled-in
  classification taxonomy (`is_destructive` / `context` /
  `wasm_compatibility` via `mandala_derive::ActionClassify`) that
  new variants cannot dodge.
- **`dispatch_macro`** over the cross-platform `MacroDispatchTarget`
  trait (`src/application/app/dispatch/macro_core.rs`) — the
  privilege-gated step loop whose steps already fan out to
  `dispatch_action` / custom mutations / `execute_console_line`.
- **`execute_console_line`**
  (`src/application/app/console_input/exec.rs`) — a string-in /
  void-out interpreter over ~20 verbs, callable with the console UI
  closed. It runs each verb's typed `ExecResult` internally and
  folds it into the console scrollback rather than returning it — so
  the `act.console` command extracts that result (see D7 and
  `format/ipc.md` §act); the parse → execute → drain-effects path is
  the reusable seam.
- **`SourceTier { App | User | Map | Inline }`** — the
  loader-pinned, fail-closed privilege model (`format/macros.md`).
- **The winit event loop** (`src/application/app/run_native.rs`) —
  `NativeApp: ApplicationHandler`, `about_to_wait` drains and then
  parks in `ControlFlow::Wait` when `needs_continuation()` is
  false. The loop already has an unused wake seam: winit's
  `EventLoopProxy` user events.
- **`FreezeWatchdog`**
  (`src/application/app/freeze_watchdog.rs`) — the sanctioned
  "background thread that never touches app state" precedent §3
  already carries.
- **Pure, GPU-free introspection surfaces** — hit-testing and scene
  building are plain functions; the model layer is fully serde;
  `tick_animations` takes its clock as a parameter with a single
  production call site (the virtualization seam IPC-09 uses).
- **Dormant intent markers** — `Options.should_exit` ("smoke-tests /
  CI captures", never read; IPC-10 territory, which replaced it with
  the read `one_shot` — D10) and the
  `EventSubscriber` seam ("reserved for the script API", reshaped by
  #43; deliberately *not* consumed by this epic's app-funnel events).

## Non-goals (pinned so they stay out)

Inherited from EPIC #60 and binding for every sub-issue: no remote /
network control, no multi-user sessions, no plugin system (IPC
*consumes* funnels, it does not define a plugin API), no maptool
static rendering, no WASM console parity work, no CI wiring. If an
implementation issue seems to need one of these, the design is being
misread — come back here.

---

## D1 — Transport: NDJSON over a local socket, opt-in by flag

**Decision.** Newline-delimited JSON over a Unix domain socket
(`SOCK_STREAM`) at a user-owned path, enabled per launch by
`--ipc <path>` (mirroring `--keybinds`). Windows: the same byte
protocol over a named pipe restricted to its owner, `--ipc <name>`
naming the pipe; IPC-02 shipped the Unix adapter and #169 the pipe,
behind the same boundary threads. Single controller at a time; the
listener outlives client connections; no session state survives a
disconnect. Full wire detail: `format/ipc.md` §Transport.

**Why a socket and not stdio.** stdio dies on the epic's central
workflow — *attach after launch, user watches while agent drives*.
The app is launched standalone (`run.sh`, a desktop session, an
already-running instance the user wants help in); a stdio protocol
would exist only for app instances an agent itself spawned, would
allow exactly one controller ever, and would leave protocol framing
one stray `println!` away from corruption on a stream the app has
never treated as an API surface. A socket also gives
reconnect-after-drop for free, which stdio structurally cannot.

**Why not localhost TCP.** No filesystem permissions — on a
multi-user machine any local user could connect; ports collide;
desktop firewalls throw consent dialogs. The socket path *is* the
access-control story (same-user-only, `0600`), and that story is
load-bearing for D4.

**Why NDJSON and not JSON-RPC / length-prefixed frames.** The
consumers are LLM agents and shell tooling; a protocol you can debug
with `socat` and parse with `jq` beats one that needs a codec.
JSON-RPC 2.0's additions (nested envelope, batching, notification
semantics) duplicate what the D3 envelope already provides, worse
(its `error.code` is a number; agents want names). Length-prefixed
binary framing earns its complexity only for large binary payloads —
which D3 deliberately routes to files instead.

**Connection lifecycle.** Pinned in `format/ipc.md`: greeting
(`hello` event) on accept; `connection_rejected` + close for a
second concurrent client; listener re-accepts after disconnect;
stale-socket self-healing at bind; socket unlinked on graceful
exit. Single-controller is a *simplification with a named escape
hatch*: the protocol has no per-connection semantics beyond
subscriptions, so if a real multi-client consumer ever appears
(§7: it must be named, not hypothetical), the change is confined
to the transport module and D2's queues — the envelope does not
change.

## D2 — §3 integration: boundary threads, a queue boundary, and a proxy wake

**The problem.** CODE_CONVENTIONS §3: single-threaded event loop, no
channels, no worker threads in interactive paths. The loop parks in
`ControlFlow::Wait` when idle (`NativeApp::about_to_wait`,
`src/application/app/run_native.rs`), so any poll-per-frame IPC
design starves while the app idles — and idle is precisely when an
agent drives the app. Something must block on the socket, and it
cannot be the main thread.

**Decision.** When `--ipc` is active, dedicated named boundary
threads own the socket. The acceptor is persistent; a reader **and**
an outbound writer are spawned per controller connection and both
exit when that controller disconnects (per-controller outbound so a
departed controller's queued frames are dropped by construction —
see the generation stamp below). None touches app state.

```
            clients (agent / mandalactl)
                      │ unix socket, NDJSON
   ┌──────────────────┴───────────────────────────────┐
   │ mandala-ipc-acceptor  (persistent)                │
   │  loop { accept();                                 │
   │    controller busy? → nonblocking write           │
   │                        connection_rejected, close │
   │    else → gen+=1, install controller, spawn       │
   │           reader+outbound for this gen }          │
   └──────────────────┬────────────────────────────────┘
                      │ hands the stream to
                      ▼
   ┌───────────────────────────────┐   ┌───────────────────────────┐
   │ mandala-ipc-reader            │   │ mandala-ipc-outbound      │
   │  (per controller, gen G)      │   │  (per controller, gen G)  │
   │  blocking read → 1 MiB cap →  │   │  recv value → serialize   │
   │  parse → enqueue IpcRequest   │   │  (8 MiB cap / too_large)  │
   │  {gen:G} → send_event         │   │  → blocking write         │
   └───────┬───────────────────────┘   └──────────▲────────────────┘
           │ mpsc inbound queue (FIFO; 1024 reqs)  │ mpsc outbound queue
           ▼                                       │ (cap 256)
   ┌───────────────────────────────────────────────┴──┐
   │ main thread — winit event loop                   │
   │  user_event(IpcWake) unparks; about_to_wait,     │
   │  after drain_inputs: drain the inbound queue,    │
   │  execute each command against InitState (same    │
   │  access as any input handler), push replies,     │
   │  request_redraw if any command changed pixels    │
   └──────────────────────────────────────────────────┘
```

- **The acceptor thread** always waits on `accept()`, independent of
  controller state — this is what makes `connection_rejected`
  *immediate*. A single thread that both accepted and read the
  controller could not honor the single-controller promise: while
  blocked in the controller's `read()`, nothing calls `accept()`, so
  a second client would sit in the OS backlog until the controller
  left instead of being told it's busy. The acceptor rejects a
  second connection with one **non-blocking** write + close (no app
  state, no queue; on `EWOULDBLOCK` it just closes — a rejectee that
  never reads must not stall the acceptor and re-starve the real
  controller, which is the whole reason accept is its own thread) and
  returns to accepting; the first connection it installs as
  controller — bumping a monotonic **generation** counter G — and
  spawns a reader + outbound pair tagged G.
- **The reader thread** (one per controller, gen G) owns that
  connection's read half: reads lines, enforces the 1 MiB cap,
  parses the envelope, and either enqueues a well-formed `IpcRequest`
  stamped `{gen:G}` and wakes the loop via
  `EventLoopProxy::send_event(IpcWake)`, or pushes a pre-formed
  `parse_error` reply straight onto its outbound queue (parse errors
  never need app state, so they never cross into the loop). On
  EOF/error it enqueues `Gone{gen:G}` — on the same queue as its
  requests, behind the last of them — and wakes the loop, so the main
  thread retires this generation (canceling its pending
  `clock.wait` and resetting its subscriptions); then it and its
  outbound peer exit, and the acceptor's next accept installs a fresh
  controller at G+1.
- **The outbound thread** (per controller, gen G) owns the write
  half: receives reply and event *values* (from the main thread;
  plus the reader's pre-formed parse-error replies), serializes them
  — the 8 MiB cap check and the `reply_too_large` substitution live
  here, off the interactive path — and performs every blocking write.
  Its queue dies with the connection, so a reply the main thread
  computed for a departed controller is discarded rather than
  delivered to the successor. Backpressure is **asymmetric**: a full
  queue of 256 *replies* pauses that controller's execution until the
  writer makes room (the writer wakes the loop when it does), and a
  write that makes no progress for 10 s — a client that has stopped
  reading — tears the connection down; unsolicited *events* are
  coalesced/dropped-oldest-by-revision rather than pausing anything
  (a thinking agent that pauses while events flood must keep its
  session). **The app is never backpressured by a slow client**, and
  the `FreezeWatchdog` never sees IPC I/O.
- **The generation stamp** is what keeps a single persistent-looking
  reply/event stream honest across reconnects: every `IpcRequest`,
  reply, event, and pending wait carries the gen it belongs to; the
  main thread only executes and routes for the current gen and only
  honors a `Gone{gen}` for the live gen. This is the mechanism
  behind `format/ipc.md`'s "no session state survives a disconnect."
- **The main thread** is the only place commands execute. *Amended
  by IPC-10 (#70):* the body described here is `app::pump::iterate`,
  which winit's `about_to_wait` and the headless driver both run, and
  with no event loop the wake is `Thread::unpark` of the parked main
  thread (D10); what follows holds for both. The event
  loop is `EventLoop::<IpcWake>::with_user_event()` — `IpcWake` a
  payload-free wake — and `NativeApp` implements
  `ApplicationHandler::user_event` to tick the watchdog (`unparked`,
  like every other handler) and nothing more: the wake's job is to
  unpark the loop, and winit runs `about_to_wait` after every
  iteration that delivered events. There, **right after
  `drain_inputs`** — so a command observes every input the user sent
  before it, throttled drag moves and zoom steps included, and its
  effects are ordered after them — the main thread drains the inbound
  queue (an empty queue is a no-op: an earlier drain may already have
  consumed what a later wake announced), executes each current-gen
  command against app state (the same access `input_context()` gives
  an input handler; the registry that dispatches it is IPC-03's),
  enqueues replies, and — the seam that keeps the watched window and
  screenshots truthful — **requests a redraw whenever a command
  changed pixels**. One drain site per iteration, so a burst
  coalesces per frame. `hello`'s `map` field is live app
  state, so the main thread publishes the current file path into an
  `Arc`-swapped snapshot whenever it changes, and the acceptor reads
  *that* (never `MindMapDocument`) when greeting — the
  state-free-boundary invariant holds. `handle_event`, which only
  ever receives `WindowEvent`s, needs no signature change; user
  events land in the separate `user_event` method. (The
  `redraw_after → request_redraw()` pattern the drain reuses is
  already in `handle_event`.)

**Amended by IPC-02 (#62), in three places, all to make the pinned
behavior hold rather than to change it.** (1) *One ordered queue,
one payload-free wake.* This section first had the reader announce a
disconnect as its own user event, `IpcGone{gen}`, beside an mpsc
queue of requests. Two channels have no order between them: a
disconnect could overtake the requests sent just before it — so
`printf … | nc -N -U`, which half-closes right after its request,
could lose its reply — and a controller's first request could reach
the main thread before the notice that the controller exists. Every
inbound message — connected, request, gone — now travels on one FIFO
queue, with the request cap counted separately so a lifecycle notice
is never refused; the user event only says "drain". (2) *Pause, not
teardown, on a full reply queue.* "256 undelivered replies →
teardown" read the queue's length as the client's health, and it is
not: the main thread executes a pipelined burst faster than the
writer thread is scheduled to drain it, so the queue can fill while a
perfectly healthy client is reading — with one `write` per reply, the
loopback tests filled it hundreds of times in a 1000-ping burst. The
writer now sends everything queued in one write, which makes that
rare, and a full queue pauses that controller's execution
until the writer makes room, the reader waits for room for its own
replies (which stops it reading, so pressure lands on the client),
and death is judged where it is visible — a write that makes no
progress for 10 s. `format/ipc.md` §Threading contract carries the
wire-visible half. (3) *One drain site, after the input drain.* This
section first drained in `user_event` and again beside `drain_inputs`.
Any drain ahead of `drain_inputs` executes a command before throttled
input the user had already sent — a pending drag move, a zoom step —
reversing the ordering `format/ipc.md` promises; and since winit runs
`about_to_wait` after every iteration with events, the `user_event`
drain was never the only one anyway. The wake now only unparks.

**Pixels-affecting commands must request a redraw (and bump the
render revision).** The existing winit handlers set
`redraw_after = true` → `Window::request_redraw()` after any
mutating input, because `about_to_wait`'s continuation check alone
does *not* schedule a present for a one-shot change that leaves
`needs_continuation()` false (a selection, a style write, a single
zoom). The IPC drain mirrors that exactly: each command
reports whether it touched pixels (document mutation *or* view /
selection change), and the drain requests a redraw and advances the
render revision (IPC-14's counter — a *render*-affecting revision,
not merely a document one) if any did. Without this, an
`act.action` zoom would update buffers that never re-render, and a
following `clock.wait until=settled` + screenshot would capture the
stale frame — the exact race `settled` exists to prevent. This is
why `settled` (D7) keys on the *rendered* render revision (render
pass executed, not swapchain-presented — so headless/occluded
capture still terminates), not the document revision.

**Why this is a §3 amendment and not a violation.** The covenant's
purpose is that *app state has exactly one thread*: no interleaving,
no lock discipline, no cross-thread mutation. That holds unchanged —
the I/O threads see only protocol value types (`IpcRequest`,
serialized frames), never `InitState`, `MindMapDocument`, or
`Renderer`. The queues are a process-boundary adapter, not
inter-component plumbing; the `FreezeWatchdog` ("thread that never
touches app state") is the sanctioned shape this generalizes. The
amendment is recorded in CODE_CONVENTIONS §3 in this PR — same
commit as the design, exactly as §3 demands for project-scale
decisions — and scopes the exception narrowly: IPC boundary only,
`std::sync::mpsc` only (crossbeam is on its way *out* per #43/P2-40),
protocol values only, and never a blocking IPC operation on the
main thread.

**Rejected: `ControlFlow::WaitUntil` polling.** A poll interval is
a fork: short enough for snappy IPC (≤ 5 ms) and the app burns CPU
waking ~200×/s forever once IPC was ever active — §4's
mobile-budget ethos says idle apps sleep; long enough to be cheap
(≥ 100 ms) and every agent command eats up to the interval in
latency, hundreds of times per session. Polling also still needs
either a non-blocking accept/read on the main thread (I/O in the
interactive path — a worse §3 breach than threads that touch
nothing) or the boundary reader thread anyway. The proxy wake gives
zero idle cost *and* sub-millisecond dispatch latency; winit built
`EventLoopProxy` for precisely this. (`WaitUntil` still earns its
keep for the two *timer* cases — a pending `clock.wait` deadline and
an active recording's frame cadence — where the loop must wake at a
known instant, not on socket readiness.)

**Rejected: one thread with a poll(2)/mio readiness loop.** Folding
accept + read + write onto one multiplexed thread is fewer threads
on paper, but it trades dumb blocking loops for a hand-rolled
readiness state machine with partial-write buffering — more code,
more failure modes, identical covenant surface. Threads whose
entire body is "blocking call, forward, repeat" are the most
auditable shape this boundary can have; the single-controller
promise (D-transport) is exactly why accept and read are *separate*
blocking loops rather than one.

**Watchdog interplay (pinned invariants).** Commands execute inside
`about_to_wait`, which ticks `unparked()` on entry like every
handler (as does the `user_event` wake), so a command that wedges
the loop still gets the watchdog's diagnostic abort — IPC gains no
exemption from the 10-second contract. Command execution is
synchronous and bounded; anything that must outlive a heartbeat
(waits, recordings) is a *deferred reply* (D7), never a blocking
loop inside a command handler.

## D3 — Envelope: flat requests, correlated replies, structured errors

**Decision.** Pinned byte-for-byte in `format/ipc.md` §Envelope:
requests `{"id", "cmd", <flat params>}`; replies `{"id", "ok",
"result" | "error", "warnings"?}` with a reserved `revision` key
(activates with IPC-14); events `{"event", "data"}`; a `hello`
greeting carrying `protocol: 1`; stable snake_case error codes;
1 MiB inbound / 8 MiB outbound caps; pixels always to files,
oversized dumps steered to `to_file`.

**Error posture (§9, hostile-input precedent PR #59).** Errors are
structured JSON values — `{"code", "message", "data"?}` — never
Rust error types (no `anyhow`/`thiserror`/custom enums anywhere in
the implementation) and never panics: a malformed, oversized, or
hostile frame degrades to a `parse_error`/`invalid_params` reply
and the app keeps running, exactly as the loader rejects a cyclic
document without dying. Codes are API (agents branch on them);
messages are prose and free to improve.

**Correlation over ordering.** Replies may arrive out of request
order because deferred-reply commands (D7's `clock.wait`) resolve
at heartbeat boundaries while later commands keep executing.
Pinning id-correlation now — rather than FIFO-with-blocking-waits —
is what keeps a pending `wait` from starving a `capture.record_stop`
and keeps the main thread free of any "wait for X inside a command"
loop, which D2 forbids.

**Why flat params.** The primary author of these frames is a
language model. `{"id":1,"cmd":"clock.step","ms":100}` has one
nesting level and reads as a sentence; a `params` wrapper adds a
brace every message to protect against a collision that reserving
two words (`id`, `cmd`) prevents outright. Strict unknown-param
rejection converts agent typos into immediate `invalid_params`
with the offender named, instead of silently ignored options and a
confusing screenshot three commands later.

**Version stance (§10).** One live protocol integer, bumped on any
breaking change, no negotiation, no deprecation shims pre-V1.
Clients tolerate unknown keys (additive growth is free);
`mandalactl` refuses a protocol it doesn't know unless forced. The
SSOT chain — this doc pins semantics, `format/ipc.md` pins bytes,
`describe` mirrors it at runtime, tests keep the mirror honest — is
what lets ~10 issues build in parallel without drift.

## D4 — Trust tier: IPC executes as `User`; no new tier

**Decision.** IPC commands execute at the existing
`SourceTier::User` posture. No `SourceTier::Ipc` variant is
added. `act.macro` runs macros under their own loader-pinned tier —
IPC initiates, it never escalates. `format/macros.md`'s
SOURCE-OF-TRUTH tier list gains a pointer recording this mapping
(done in this PR), so a future tier addition or reorder must
re-evaluate it consciously.

**Rationale — "the user owns the flag."** The trust question is:
*who chose to expose this surface?* For `keybinds.json` and
`macros.json` the answer is "the user, by editing a file they own,"
and the tier is `User`. For IPC the answer is "the user, by passing
`--ipc` at launch" — the same authority exercising the same kind of
consent, enforced by the same mechanism (filesystem permissions on
a user-owned path, D1). The peer process on that socket runs as the
same OS user and could already write `macros.json`, edit
`keybinds.json`, or attach a debugger; pretending in-app tiers can
contain it would be security theater. Meanwhile the epic's charter
is human parity — an agent that cannot save, delete, cut, paste, or
run console verbs cannot give feedback on the app, so any tier
*below* User contradicts the feature's reason to exist.

**Why not a distinct `Ipc` tier anyway (auditability,
future-proofing)?** Everything a tier would buy is available
cheaper and safer:

- *Attribution* — a logging/eventing concern: IPC dispatch sites
  tag their origin in diagnostics and (post IPC-14) events carry
  it. No privilege machinery needed.
- *Future restriction* — would be a product decision to make IPC
  weaker than the human it serves; if that day comes it is a
  protocol-level redesign (this document), not a dormant enum
  variant waiting to confuse someone.
- Against those non-benefits: a fifth variant ripples through every
  entry in the SOURCE-OF-TRUTH list (`SourceTier` order, two
  loader call-site groups, the loader helpers) plus the registry's
  hand-written tier walk beside that list, forces an answer to
  "does `Ipc` allow
  ConsoleLine?" whose only sensible value duplicates `User`, and
  permanently widens the threat-model reference everyone must read.
  §7 over-engineers for *named* trajectories; "maybe IPC should be
  weaker someday" is not one.

**The composition rule (the part that must not drift).** Privilege
is attached to *where instructions were authored*, not *who pulled
the trigger*. A Map/Inline-tier macro fired over IPC is still
Map/Inline-tier — `dispatch_macro`'s fail-closed gates run
untouched. IPC-03's bridge therefore calls the existing dispatchers
and adds **no** gate-bypassing entry points; if a future command
needs a path around a gate, that is a redesign of this section, not
an implementation detail. The same rule places `act.macro`'s
anonymous `steps` (issue #63's inline variant): the IPC client
authored them, so they run at `User`, and they run through the one
gated step loop (`run_macro_steps`, which `dispatch_macro` now calls
after its lookup) rather than a second loop.

## D5 — Command families, registry shape, self-description

**Decision.** Eight families — bootstrap `describe`/`ping`, `act`,
`query`, `scene`, `input`, `capture`, `clock`, `events` — each owned
by exactly one issue and living in exactly one module under
`src/application/ipc/commands/`, assembled by one registry table
(one registrar line per family). The full family → issue → command
map, and the self-description (`describe`) contract, are pinned in
`format/ipc.md` §Command registry / §Command families.

**Why this shape.** It is the console's proven pattern (`COMMANDS`
slice, one module per verb) scaled one level up, and it is §6
modular design serving a scheduling need: Wave 2 lands IPC-04/05/06/
14 as parallel PRs, so the registry must make their diffs disjoint
by construction — new module + one line, no shared files edited.
Command names are namespaced `family.verb` so the registry stays
flat (no routing tree), `describe` output groups naturally, and a
family's docs section is greppable from any frame.

**Self-description as a compile-shape.** A registry entry cannot be
constructed without name, summary, and param/result descriptors —
the same forcing-function philosophy as `ActionClassify` (a variant
cannot exist unclassified). `describe` is rendered *from the
registry*, so the discovery surface cannot drift from the dispatch
surface; drift against `format/ipc.md` is caught by unit tests
asserting the registry matches the documented command list (and
IPC-16 audits at epic close).

**Module layout** (IPC-02 creates the spine, IPC-03 the registry):

```
src/application/ipc/            #[cfg(not(target_arch = "wasm32"))]
├── mod.rs         module doc: the D2 boundary contract
├── transport/     acceptor + per-controller reader + outbound threads (IPC-02)
├── envelope.rs    IpcRequest / reply / event / warning value types (IPC-02, warnings IPC-03)
├── registry.rs    CommandSpec + descriptors, execute, the capture window (IPC-03)
├── target.rs      CommandTarget, the app-side execution context (IPC-03)
├── space.rs       the surface / canvas coordinate spaces (IPC-03)
└── commands/
    ├── mod.rs     FAMILIES, the family registrar index (IPC-03)
    ├── meta.rs    describe, ping (IPC-03)
    ├── act.rs     … (IPC-03)
    ├── query.rs   … (IPC-04)
    ├── scene.rs   … (IPC-05)
    ├── refusal.rs the read families' shared refusals (IPC-05, from IPC-04's query.rs)
    ├── sidecar.rs the to_file writer (IPC-05, from IPC-04's query.rs)
    ├── input.rs   … (IPC-06)
    ├── capture.rs … (IPC-07, extended by IPC-08)
    ├── clock.rs   … (IPC-09)
    └── events.rs  … (IPC-14)
```

Known micro-overlaps (from EPIC #60's parallel map, unchanged):
the camera read accessor (IPC-04 ∩ IPC-05 — first to land adds it),
the `document/mod.rs` revision field (IPC-14; IPC-09 consumes).

**Crossing the `app` boundary.** `crate::application::ipc` is a
*sibling* of `crate::application::app`, but every funnel a command
reaches — `dispatch_action`, `dispatch_macro`,
`execute_console_line`, `InitState` / `input_context()` — is
`pub(in crate::application::app)`, so a command handler cannot call
them directly. IPC-03 therefore reaches them through an app-side
**execution-context trait** (holding `&mut InitState`, implemented
inside `app`, passed into the `ipc` handlers) — the exact
indirection `dispatch_macro` already uses via `MacroDispatchTarget`
(`app/dispatch/macro_core.rs`), which is why native and WASM share
one macro loop. This keeps the funnels private and the `ipc` modules
free of `app` internals. Widening the seams to
`pub(in crate::application)` is an acceptable equivalent, but the
trait shim is the §2/§7-aligned choice and reuses an existing
precedent; it is IPC-03's to wire and is called out here so the
"reaches the funnels" language in §D5/`format/ipc.md` isn't read as
"calls private items across a module boundary."

*As built by IPC-03 (#63):* the trait is `CommandTarget`
(`ipc/target.rs`), and its native implementation is
`IpcCommandTarget` (`app/ipc_target.rs`). It wraps the same
`InputHandlerContext` a winit handler receives rather than `&mut
InitState`, because that context is exactly "the access an input
handler has". Each method is one funnel. A handler reaches the target
through `drive()` (`&mut`) or `observe()` (`&`), so a read-only family
like IPC-04's cannot mutate by construction. The redraw decision is
not taken from the borrow. The target reports it
(`CommandTarget::changed_view`) from each funnel's own outcome, so a
refused action or a read-only console line polled in a loop costs no
frame. (The first IPC-03 cut keyed it on the `&mut` borrow, and review
caught every failed or read-only command forcing a render.)

*As built by IPC-04 (#64):* the read half needed no new funnel. The
trait gained three `&self` reads: `document()` (a `&MindMapDocument`),
`animation_pause()` (the drag pause on the animation tick, which lives in
`InitState` and not in the document) and `shell_state(ShellState)` (a
JSON projection of the app-private state: camera, mode, status,
editors), because `DragState` and the modal
editors are private to `app` and the `ipc` modules never name them.
The projections are inherent methods beside the state
(`document/ipc_state.rs`, `app/ipc_state.rs`), each an exhaustive
`match`, so a new variant breaks the build there. The family is eight
commands: the seven pinned in `format/ipc.md`'s family map plus
`query.status`, which the issue listed and the map did not. The
camera read accessor is `Renderer::camera()`; IPC-05 reuses it.

*As built by IPC-05 (#65):* the pipeline question the issue raised is
settled by #22 having closed: there is no flat `RenderScene` left, and
every model-derived visual is a `Tree<GfxElement, GfxMutator>`. So the
serializer is a Baumhard primitive over those trees,
`baumhard::mindmap::scene_dump`, and nothing in the family re-derives
geometry from the model. Three seams were extended rather than
re-implemented. The renderer's two duplicated cull conjunctions became
one classifier, `Camera2D::visibility`, which returns the *reason*
(`OutsideZoomWindow` before `Offscreen`) and which the renderer now
calls; the dump reports its verdict, so the two cannot disagree. The
span shaper's coverage rule moved into one private iterator shared with
`font::attrs::displayed_text`, so the text the family reports is the
text the renderer slices. And the label tree's channel → `EdgeKey`
index became `EdgeChannelIndex`, which now also names the connection
tree's per-edge `Void`s: the connection dispatcher stamps it on
`AppScene` from the identity sequence it already built for its
signature, moving the keys instead of dropping them. The trait gained
one `&self` read, `scene()` (a `SceneView`: the document, the node
tree, `AppScene`, the camera — plain data, so a test stands one up from
`CanvasFrame` with no renderer), and one `&mut` method, `hit_test`,
whose projection lives app-side (`app/ipc_scene.rs`) because the click
ladder (`compute_click_hit` and its last rung `edge_under_pointer`) is
private to `app`. A test holds it to a real click at every node, label,
portal part and connection of the fixture. The family is five
commands: the three pinned in `format/ipc.md`'s family map plus
`scene.visible_set` and `scene.node_geometry`, which the issue listed
and the map did not — the `query.status` precedent above. The file
sidecar writer and the read families' refusal type moved out of
`query.rs` into `commands/sidecar.rs` and `commands/refusal.rs`, so
`scene.dump`'s `to_file` is the same atomic write rather than a second
one, and the camera's wire projection moved to `ipc/space.rs` beside
the coordinate spaces it describes, shared by `query.camera` and every
dump.

*As built by IPC-06 (#66):* the fidelity contract made the seam
literal. `handle_event`'s mouse, wheel, cursor, touch and keyboard
arms became `InitState::on_*` methods, and the `input` family's
samples are delivered to exactly those — so the target now holds
`&mut InitState` (and the `ActiveEventLoop` the keyboard handler
takes) and builds the `InputHandlerContext` per call, where IPC-03's
cut wrapped one. *(IPC-09 since moved the `on_*` methods onto
`InputHandlerContext`, where the heartbeat can reach them, and dropped
the event-loop handle the keyboard handler never read: D9.)*
`dispatch_touch_event` takes the touch payload
rather than winit's `Touch`, whose `DeviceId` cannot be built outside
winit. The injected-input queue (`ipc/injected.rs`) lives on
`InitState`, is drained one sample per `about_to_wait` ahead of
`drain_inputs`, and is a `needs_continuation` source. Only
`input.drag` paces through it; every other command is one ordered
burst applied before its reply, because nothing between its events
is drained per frame — a double-click's detector reads the clock and
the hit, and two clicks in one iteration are still a double-click —
and a burst sent behind queued samples waits for them, so order
holds. The family is nine commands: the six in the family map plus
`input.drag` and `input.click_node`, the sugar the issue named, and
`input.pending`, the queue-depth read an agent polls until
IPC-09's `settled` exists. `input.key` spells named keys the way
winit's `NamedKey` deserializes (winit's `serde` feature): the names
`key_to_name` reads off a live event, case-exact on the wire where a
keybind compares them lowercased. Modifier keys are refused there and
held with `mods` instead, because winit reports them through
`ModifiersChanged`, not as the key. The queue is kept per command and
resolves a command's canvas positions when its first sample lands,
so a command queued behind a pan-drag aims through the camera the
drag left; `input.click_node`, whose reply reports where it clicked,
is refused `busy` instead of queueing.

Driving the real app through it surfaced two app defects, both fixed
in the same change. winit delivers the space bar as
`Key::Named(Space)`, and the node text editor and the single-line
label editor inserted only `Key::Character` payloads, so a space
typed by hand was dropped; the fix is the console's own shape, a
rebindable Action per editor (`TextEditInsertSpace`,
`LabelEditInsertSpace`, both bound to `Space`), not a hardcoded key.
And a promoted drag began at the move that crossed the drag
threshold rather than at the press, so the node, resized box or
panned canvas lagged the pointer by the sub-threshold travel for the
whole gesture; the promoting move now seeds the gesture with the
travel from the press (`start_promoted_gesture`).

## D6 — Cross-platform stance (§4): explicit native carve-out

**Decision.** IPC is native-only at the `src/application/ipc/`
module boundary (`#[cfg(not(target_arch = "wasm32"))]`, the
`console_input` precedent). The §4-mandated carve-out entry now
exists in `CLAUDE.md` "Dual-target status" — a section this PR
creates, since §4, CONCEPTS §1, and `freeze_watchdog.rs` all
referenced it and it had never been written; the already-documented
native-only surfaces (watchdog, console modal, clipboard backend,
`fps` verb) are backfilled there from their existing doc entries so
the section is born truthful.

**Why carve out rather than build dual-target now.** The transport
is *definitionally* native — browsers cannot serve local sockets,
and the epic's consumer (an agent in a desktop/Xvfb container)
is native. The §4 rule exists to stop "I'll add WASM later" drift
on features that *could* be shared; here the shared part — envelope,
command semantics, registry — is exactly what `format/ipc.md` pins
platform-independently, and the WASM path (WebSocket transport +
browser capture, reusing the envelope verbatim) is a *named,
parked* trajectory with an owner: IPC-15. `Action`'s
`wasm_compatibility` taxonomy already gives IPC-15 its per-command
compatibility answers. `./test.sh`'s wasm32 type-check gate keeps
the boundary honest at every merge.

## D7 — Capture and determinism surface: shapes pinned, engines owned downstream

**Decision.** The command shapes for `capture.screenshot`,
`capture.record_start/stop`, `clock.status/set_mode/step`, and
`clock.wait` — params, defaults, clamps, results, the geometry
sidecar schema, the recording manifest schema, and the settling
semantics — are pinned in full in `format/ipc.md` (§capture,
§clock). IPC-07/08/09 implement engines behind stable shapes;
`mandalactl` and the two skills (IPC-12/13) build against the
shapes without waiting.

**The shape-level calls that needed making here:**

- **Offscreen, never window-grab.** `capture.screenshot` renders to
  an offscreen target sized by the request. This makes captures
  compositor-independent (Xvfb container = laptop = future
  headless IPC-10), decouples capture resolution from window size,
  and sidesteps the swapchain-format bug tracked in #7 (offscreen
  target format is IPC-07's to pin, coordinated with #7's SSOT).
- **Sidecar over annotation.** The pixels↔ids map is a JSON sidecar
  (`<path>.geometry.json`), not baked into the image — agents
  hit-test by arithmetic, humans view clean PNGs, and the sidecar
  schema can grow keys additively.
- **Bounded before allocating / running, and write-guarded.** A
  malformed agent frame must degrade, not detonate: `capture.*`
  clamps each side to `1..=16384` and the pixel budget
  (`w × h × scale²`) to 64 Mpx — an offscreen texture is a real GPU
  allocation. `clock.step` clamps to **one** budget: since virtual
  time advances at a pinned 16 ms/heartbeat, `frames = ceil(ms/16)`
  and both spellings clamp to `frames ≤ 100_000` (⇒ `ms ≤ 1_600_000`,
  ~27 min) — the earlier "`ms ≤ 1 h`" let the two params bound
  different work, now reconciled. And the clamp is *not* the
  watchdog-safety mechanism: the step loop **ticks the watchdog per
  internal heartbeat**, so a bounded catch-up is safe by
  construction while a genuinely stuck single heartbeat still trips
  the 10 s threshold; the clamp is a responsiveness/DoS limit.
  Because an LLM clobbers a path by accident more readily than a
  human, `capture.*` also **write-guards** client-supplied `path` /
  `dir` (no symlink-follow; no overwrite outside the artifacts dir
  without `overwrite:true`) and bounds the **server-chosen**
  artifacts dir (ring-buffer, removed on exit) — client-supplied
  paths are the client's to reclaim. Over-bound/guarded requests are
  `invalid_params`. Exact numbers: `format/ipc.md` §capture / §clock.
- **Recording emits frames + manifest; assembly is not the app's.**
  GIF/video encoding in-app would drag an encoder dependency into
  the render path for a job a client does better; the
  `manifest.json` (with per-frame `t_ms` on the capture clock) is
  the contract between IPC-08's recorder and whatever assembles it.
  *Amended by IPC-08 (#68):* the assembler is `mandalactl record
  assemble <dir>` — a client-side command that reads the directory
  and never opens the socket, so the app still links no encoder and
  the interactive path stays lean — writing an animated GIF (the
  format GitHub shows inline in a PR, which is the review vehicle
  the epic names) with the pure-Rust `gif` crate, or an MP4 through
  the host's `ffmpeg`, shelled out to and never a dependency. The
  split this decision made — frames in the app, encoding outside it
  — is unchanged; what moved is that the encoder is not left to a
  skill's choice of tool. An **active recording is a
  continuation/timer source** —
  on a static scene the loop would park and capture nothing, so
  while recording it parks in `WaitUntil(next frame deadline)` at
  the requested `fps` (under the virtual clock, `clock.step` drives
  frames instead). The **frame-emission rule** is pinned so the
  manifest is deterministic: emit a frame each time the capture
  clock crosses a `1000/fps` boundary — **not** once per heartbeat —
  so `clock.step ms=1000` at `fps=30` yields exactly 30 frames no
  matter how many heartbeats ran. That one rule is the whole IPC-08 ↔
  IPC-09 interlock EPIC #60 warns about.
- **`act.console` reads a typed result, not a live scrollback.**
  `execute_console_line` today returns `()` and pushes verb output
  into a `ConsoleState` scrollback that only exists when the modal
  is `Open`. IPC must not fake an open modal to scrape it; IPC-03
  extracts the parse-and-execute core to return the `ExecResult`
  the command already produces (`Ok`/`Lines`/`Err`), which the
  modal path keeps rendering to scrollback and IPC reads directly.
  Threading the result out is sanctioned §2 seam work; pinned in
  `format/ipc.md` §act so IPC-03 doesn't improvise a UI-scraping
  path.
- **`clock.wait` is a deferred reply** evaluated at heartbeat
  boundaries and at its deadline (D2/D3), with two conditions only:
  `animations_complete` and `settled`, and **exclusive per
  controller** (a second pending wait → `busy`; unbounded pending
  waits would be a memory/scan-cost vector, and there's no named use
  for concurrent waits). **`settled`** means "the rendered scene is
  the final consequence of every command the agent sent" and keys on
  a **render revision**, not the document revision:
  `needs_continuation()` false, injected-input queue empty (the
  one-sample-per-frame queue from D5/§input — pinned so `settled`
  and IPC-06 share one model), no redraw pending, and the frame has
  been **rendered** (render pass ran, buffers current) at the
  current render revision. Two subtleties the review surfaced, both
  now pinned: (1) `settled` keys on *rendered*, **not**
  swapchain-*presented* — an offscreen/headless (IPC-10) or occluded
  capture never presents, so a present-gated `settled` would time
  out in exactly the target environment; and (2) "race-free" is
  scoped to the agent's own commands — a concurrent human's input
  between the wait reply and the screenshot, and especially an inline
  human pan-drag that changes pixels without bumping the revision, is
  not fenced. The counter advances on *every* pixels-affecting
  command, view-only ones (zoom, pan, selection) included; keying on
  the document revision alone would resolve `settled` before a
  view-only repaint rendered (this is why D2 has the IPC drain
  request a redraw and bump the render revision per pixels-affecting
  command). IPC-14 owns the counter (overlapping the
  `document_changed`/`camera_changed`/`selection_changed` emission it
  already builds — the many-site instrumentation is not net-new
  scope); if wave order runs IPC-09 first, IPC-09 carries the field
  itself. `MindMapDocument.dirty` deliberately plays no role: despite
  the name it is the *unsaved-changes* flag — set by document
  setters, cleared only at construction and on `save`, read by the
  `open` / `new` guards — not a rebuild signal. Issue #61's sketch
  assumed otherwise, as did CONCEPTS' "Dirty flag" entry; both
  corrections land with this design, because a settled condition
  gated on `dirty` would never resolve after any unsaved mutation.
  The `timeout_ms` deadline is wall-clock even under a virtual clock
  (it guards client liveness, not simulation time), and a pending
  wait converts an idle park from `ControlFlow::Wait` to
  `WaitUntil(nearest deadline)` — the FPS idle-grace precedent —
  so deadlines fire without polling and without a wake-starved
  hang. A wait is not a barrier: dependent commands are sent after
  its reply (D3). A third `idle` condition was considered and
  dropped: post-settled the loop parks by construction, and the
  only residual timer is the FPS overlay's idle-grace flip — a
  diagnostic that observes behavior and must not become observable
  behavior.
- **Virtual time is a mode, not a per-command flag.**
  `clock.set_mode virtual` freezes app-observed time except via
  `clock.step`, making animation state a pure function of the step
  sequence. How virtual time threads through `now_ms()` and the
  heartbeat is IPC-09's design problem (EPIC #60 rates it Fable
  for a reason); the *surface* above is what it must satisfy, and
  the single-call-site clock parameterization of `tick_animations`
  is the seam it starts from. *Resolved by IPC-09 (#69): D9.*

## D8 — Observability: replies carry their own diagnostics

**Decision.** Every reply may carry a `warnings` array of
`{"code", "message"}`; every command handler receives a warning
sink; the convention is **dual-emit** — the same site that
`log::warn!`s a degrade pushes the same text into the sink. Defined
once in `format/ipc.md` §Warnings; implementing issues own their
commands' warning completeness, and extending a seam to surface a
degrade upward is sanctioned §2 work.

**Amended by IPC-03 (#63): the sink is the log, tapped.** As written,
dual-emit meant threading a sink through every site a command can
reach. For `act.action` that is every dispatch arm, every document
setter, and the Baumhard primitives under them. That is a second
diagnostic channel beside the first, incomplete the day it lands,
and it cannot reach Baumhard without Baumhard learning about IPC.
IPC-03 keeps the decision and changes the mechanism. The registry
runs each handler inside a `baumhard::util::log::capture` window. The
installed logger offers every record to the window open on its
thread, so each `warn!` / `error!` the command emits lands in its
reply. The log line *is* the warning: one site, one emission, the
same text by construction, and a `warn!` added anywhere later is
reported with no further work, as `ActionClassify` gives a new
variant no way to go unclassified. A site that wants a
machine-branchable code attaches it as the record's `code` key-value
(`log`'s `kv` feature). The macro gate's `macro_gate_rejected` is the
first such code. Uncoded records are `log_warn` / `log_error`. The
window is bounded (64 records, then a `warnings_truncated` count), so
a per-node degrade on a large map cannot push a reply past the frame
cap and lose its result. Each captured message is also held to 4096
bytes, because a count alone does not bound size: a message can echo
arbitrary input. "Extending a seam" now means making a silent degrade
log, which §9 asks of it anyway.

**Why the envelope and not the log stream.** The design first
reasoned from `release_max_level_off`, under which an agent driving a
release binary had no log at all. #45 has since kept `warn!` /
`error!` in release (`release_max_level_warn`), and the reason
survives that change. The log reaches the app's stderr, which an
agent attached to an already-running instance over the socket never
sees. The reply envelope is the one channel that provably reaches the
agent in every build profile and every launch arrangement. Streaming
out-of-band degrades (a `log_line` event class), meaning those
outside any command's window, became possible with #45. Whether to
ship it was IPC-14's call, and IPC-14 did not (see its ledger entry
below); this design takes no dependency on it.

## D9 — Virtual time: one app clock, a frame split in two halves (IPC-09)

**Decision.** Time virtualization lives in one cross-platform type,
`AppClock` (`src/application/clock.rs`), owned by the app shell
(`InitState.clock` on native, `WasmInputState.clock` in the browser)
and threaded to every reader of app-observed time through the context
it already receives (`InputHandlerContext::clock`, and
`InputContextCore::clock` for the cross-platform dispatch bodies). The
old free function `now_ms()` is gone: its two meanings — "the time the
app behaves by" and "the time a measurement is taken on" — are two
names now, `AppClock::now_ms` and `clock::wall_ms`, and every reader
was dispositioned to one of them (table below). The step engine is the
heartbeat (`app/heartbeat.rs`), and it is native for the reason the
per-frame drain is.

**Why a value threaded through the context, not a global.** A global
virtual offset under the old `now_ms()` would have virtualized every
caller at once, the cost meters included — the FPS overlay would have
read a stepped clock and the freeze watchdog a frozen one — and the
per-reader decision this issue asked for would have been made by
default rather than by reading. Threading the clock makes each reader
name the clock it means, and makes a test's clock an ordinary value.

**One continuous axis.** Switching modes never moves the reading:
entering virtual time freezes it where it stood (rounded up to the
next whole millisecond, so integer steps keep the `u64` stamps
animations and triggers are kept in exact), and leaving resumes from
where virtual time was left, at the wall clock's rate (`offset =
virtual − wall`). This replaces the issue's suggestion — rebase the
in-flight animations' `start_ms` through the drag-pause shift primitive
— because the shift covers animations alone: the last click's time, a
touch's start, the drag pause's own stamp and a recording's start are
readings too, and a switch that rebased only some of them would leave
the rest on another axis. With no jump there is nothing to rebase.

**The frame has an input-driven half and a time-driven half.** The
input-driven half (throttled drag and picker-hover drains, rubber
band, camera-geometry rebuild) flushes what input already did and needs
no clock. The time-driven half (one injected-input sample, the
animation tick, a recording's frame) is a function of *when* the frame
runs. In real time every event-loop iteration runs both, as before. In
virtual time the loop's own iterations run the input-driven half only
(`Beat::Loop`), and the time-driven half runs inside `clock.step`.
Three consequences, each the only consistent one:

- **The loop does not spin.** An animation or queued injected input is
  a continuation source in real time only: in virtual time the
  heartbeat that advances it is a step's, and a loop that polled for it
  would run at 100 % CPU doing nothing.
- **`settled` means "nothing owed at this instant"** under a virtual
  clock, so an animation frozen between steps and samples waiting for
  a step do not hold it back. That is what makes `clock.step ms=500` →
  `clock.wait until=settled` → `capture.screenshot` capture an animation
  exactly 500 ms in — the issue's first acceptance criterion. This
  amends D7's settled conditions (a) and (b), which were written
  before there was a virtual clock to read them against.
- **The throttle does not throttle a virtual heartbeat.** It trades
  drain frequency for staying inside a real frame budget; a virtual
  heartbeat has no real deadline, so skipping a drain buys nothing and
  makes what a stepped frame shows depend on how long the machine took
  to draw the last one. Its measurements stay wall-clock and its state
  is untouched by an unthrottled frame, so real time resumes with the
  tuning real time left it.

**The step schedule.** A heartbeat every 16 virtual ms from the step's
start, the last on its end exactly (`clock::heartbeat_times`) — so
`ms` advances exactly `ms` and `frames` exactly `16 × frames`, and
neither overshoots. A running recording's frame boundaries are stopped
at too (`Beat::Boundary`): the realtime recorder is a timer source the
loop wakes for, and a virtual step honors the same timer on its own
axis. A boundary tick runs the drains, the tick and the recorder but
lands no injected sample, so recording a gesture never changes when the
gesture's samples land; the frame it takes is *at* its boundary, which
is what makes a stepped recording's frames fixed-interval and its GIF
smooth. A stepped boundary is rounded up to a whole millisecond
(`capture::record::Timeline`), each from its own index: the virtual
clock and every animation stamp are whole milliseconds, and a
fractional boundary would stop the clock where the tick reads a
truncated instant while `t_ms` names the exact one. The loop over these instants (`heartbeat::run_step`) is generic
over a `Steps` implementor — the handler context in the app, a real
recording or document on a real clock in the tests — so the schedule
the tests pin is the one the app runs. Every heartbeat and boundary
tick ticks the watchdog (D7's watchdog-per-heartbeat pin).

**Where a step can run from.** The heartbeat is a method set on
`InputHandlerContext`, not on `InitState`: the `on_*` input arms moved
there with it (`InitState` builds the context and calls them, exactly
as before), so `clock step` typed into the console — whose executor
holds a context, not `InitState` — runs the very frame `clock.step`
does, and a macro's `ConsoleLine` step that steps the clock does so in
order with the steps around it. A step or a mode switch asked for from
inside a running step (a `clock step` or `clock real` line typed by an
injected key the step itself landed) is refused rather than nested: a
switch to real time partway would leave the step running its remaining
heartbeats on a clock the wall now moves.

**`clock.wait` is the protocol's first deferred reply.** The handler
records a `PendingWait` in the controller's `Session` (which therefore
cancels it on disconnect, for free) and its `Executed` carries no reply;
the transport answers it when `IpcServer::evaluate_wait` finds it due.
The evaluation points are the end of every loop iteration — after its
redraw has been asked for, which is what makes a pixels-affecting
command of the same iteration count as owed — and after every window
event, the render among them. "Rendered" is a ledger (`app/settle.rs`)
of redraws asked for and redraws drawn, counted where every redraw is
asked for (`InitState::request_redraw`); a render whose CPU half ran
has drawn whether or not the surface could present (D7's
"rendered, not presented"). That is the render revision D2 and D7 call
for, kept at the asks rather than inferred from the document, so a
view-only change is fenced exactly like a document one.

**Determinism, verified rather than believed.** With virtual time
engaged before a script, the laid-out scene is a pure function of the
document and the command sequence:
`test_the_same_stepped_script_twice_dumps_byte_identical_scenes` holds
two independently built documents — every `HashMap` in each seeded
apart — to byte-identical `scene.dump`s mid-animation and after it,
with virtual time engaged at different readings. The hunt for the
`HashMap`-order class found one live instance, the inline-macro
collision: the walk had been sorted, but every duplicate was still
handed to the registry's last-writer-wins insert, so the *highest*
node id won while the warning named the lowest. It now keeps the
lowest, as the warning and `format/macros.md` say.

**Real time unchanged.** With the clock never switched, the reading is
the wall clock (zero offset), every iteration runs the whole frame in
the order it ran before (injected sample, drains, IPC drain, recorder),
the throttle gates as before, and the continuation sources are the
same set. What moved is where the code lives and which name a reader
calls, not what a real-time run does.

Two things do change, and neither is a real-time behavior of the
frame: a cross-node inline-macro collision now has the winner its
warning always named (above), and the browser's wall clock counts from
its first read through `web_time` — as native's always did — rather
than from page load, which moves its origin and nothing else (no reader
compares a reading with the page's own clock).

**Every reader, dispositioned.** The issue asked for each `now_ms()`
call site to be decided explicitly; this is the decision, and the
readers of `Instant` that measure are listed beside them, since they
are the other half of the same question.

| reader | site | clock | why |
|---|---|---|---|
| animation tick | `animation_drain::drain_animation_tick` (from `heartbeat::drain_inputs`; the browser's frame runs it too) | app | the behavior virtual time exists for |
| drag pause on the tick | `heartbeat::drain_inputs` (`anim_pause_start_ms`) | app | its stamp is subtracted from the tick's clock; two clocks would not cancel |
| double-click window | `event_mouse_click.rs` (native), `run_wasm/event_mouse_click.rs` | app | an IPC-06 click pair must be schedulable on the axis a step advances |
| `OnClick` trigger start | `click.rs` (via `ClickCore::now_ms`), `cross_dispatch/pointer.rs` (tap), `run_wasm/event_mouse_click.rs` | app | starts an animation, which the tick then measures |
| keybound / macro custom mutation start | `cross_dispatch/lifecycle.rs`, `dispatch/native.rs` (macro target), `run_wasm/mod.rs` (macro target) | app | as above |
| long-press and tap timing | `InputHandlerContext::dispatch_touch_event` (native), `run_wasm/event_touch.rs` | app (`AppClock::instant`) | input timing, the double-click's peer |
| recording frame times, pacing, deadline, end | `record_flow` (all of it) | app | a recording is paced on the clock its frames show; the wake it asks the loop for is a wall-clock instant only in real time, where the two run at one rate |
| `query.animations` `elapsed_ms` / `phase` | `ipc/commands/query.rs` | app | must agree with what the next tick does |
| `ping`, `clock.status` | `ipc/commands/meta.rs`, `ipc/commands/clock.rs` | app | they report the app clock |
| FPS overlay and its idle grace | `renderer::tick_fps`, `fps_idle_defer_deadline` | wall (`Instant`) | measures real frame cost; a diagnostic must not become observable behavior |
| freeze watchdog | `freeze_watchdog.rs` | wall (`Instant`) | guards real liveness; a frozen clock would disable it |
| drag throttle's budget | `ThrottledInteraction::drive` | wall (`Instant`) | measures real work; the *decision* is suspended under virtual time (above) |
| encode cost, idle-encode budget, manifest rewrite interval | `capture/record.rs` | wall (`Instant`) | cost measurements; under virtual time the idle budget is one heartbeat of wall time |
| GPU readback wait | `renderer/capture.rs` | wall (`Instant`) | a liveness bound on the GPU |
| `clock.wait` deadline and `elapsed_ms` | `ipc/wait.rs` | wall (`Instant`) | client liveness (D7) |
| transport timers | `ipc/transport` | wall (`Instant`) | I/O liveness |
| console default file names | `console_input::screenshot` / `record` | wall (`SystemTime`) | a name, not a time the app behaves by |

**Rejected: a virtual mode that resets on disconnect.** Session state
dies with its controller (D1), and the pending wait does. The mode does
not: `mandalactl` is one connection per command, so a mode that reset
on disconnect could never be stepped from a shell. The cost is named in
`format/ipc.md` §`clock` — an app left virtual stays frozen until
someone says `real`.

**Rejected: stepping at the recording's cadence instead of 16 ms.**
Running heartbeats only at frame boundaries while a recording runs
would have made the recording smooth with no extra ticks, but it would
have made *what the animation, the drag and the injected queue saw*
depend on whether a recording was running — an observer effect, and
`frames` would no longer have meant one count. Boundary ticks beside a
fixed cadence keep both.

---

## D10 — Headless: one loop body, two drivers, no window (IPC-10)

**Decision.** `--headless` runs the app with no event loop, no display
and no window, on a second native driver (`app/run_headless.rs`) beside
winit's (`app/run_native.rs`). The two share everything but how they
wait: the body winit's `about_to_wait` ran is extracted, step for step,
into one function both call (`app/pump.rs`, `pump::iterate`), which
returns *when the next iteration should run* as a value (`Next`) —
winit maps it to a `ControlFlow`, the headless driver to a park. The
app state (`InitState`) moves out of the winit file into
`run_native_init.rs` beside its builder, because it is no longer one
driver's; the input arms (`on_*`, `dispatch_touch_event`) move to
`input_context.rs` for the same reason.

**The seam is two calls wide.** The app reaches its window for exactly
two things — asking for a frame and setting the cursor icon — and both
now go through `app/shell.rs`'s `Shell` (`Window(Arc<Window>)` or
`Headless`). Everything else a frame needs reads the renderer, which
was sized from the window once and resized by decrees, never by
asking the window. With no window a frame asked for is the redraw
ledger's alone (`app/settle.rs`), which every ask already goes
through: the headless driver draws whenever the ledger says a frame is
owed — the first one unasked, as a new window's; then each ask no
render has drawn, unless the last render stalled on a skip retrying
would not cure — before it next parks, where winit delivers
`RedrawRequested`, and then runs another iteration, as winit runs
`about_to_wait` after every iteration that delivered an event. (A flag
on the shell was the first draft, and a second record of what the
ledger already knew; review removed it.) A cursor icon is state nobody
sees, so it is no call. The
renderer gets the same seam on its side: its frames land in a `Sink`
(`Window { surface, config, window }` or `Headless`), and its format
and size live beside the sink rather than inside a surface
configuration it may not have.

**A headless frame draws nothing.** There is nothing to draw into and
nobody to see it: the frame is bookkeeping — the ledger records it
drawn, the FPS and mode-status overlays stay current — and pixels are
drawn only when a capture asks, through the prepare / encode pair
IPC-07 parameterized, whose prepare brings every buffer current at the
capture's own camera. Two heavier frames were rejected: encoding every
headless frame into a texture nobody reads (a GPU pass for nothing an
agent can see), and — the first draft — the CPU half alone, an
occluded window's frame, which review found was the same waste one
stage earlier: the capture prepares again anyway, and staged uploads
with no submission accumulate in wgpu until one comes. The format is
`Bgra8UnormSrgb` — the sRGB format the windowed bootstrap picked under
Xvfb on Mesa's lavapipe (observed: offered `[Bgra8UnormSrgb,
Bgra8Unorm]`) — and the default size is 800×600, the size a window
asking for none opened at there; `--size` sets either driver's.

**Order is the body's, not the driver's.** The issue sketched the pump
as "drain IPC → drain inputs → tick → rebuild → render"; the order kept
is the one CONCEPTS §5 already pinned and winit ran — the frame's
injected sample and drains first, the IPC drain after them, then the
recorder, the redraw decision, the FPS flip, the clock wait, the map
publication — because the issue's own rule was to reuse the functions
and not fork a second heartbeat. The one-shot check is the eleventh
step, at the point the clock wait is evaluated. `pump::tests` pins the
order against a recording fake (renderer-free, §T8).

**Waiting and waking.** The headless driver parks its thread — until
the iteration's deadline, or indefinitely — and the IPC boundary
threads wake it with `Thread::unpark`, the role the winit
`EventLoopProxy` plays under a window. An unpark that lands while the
pump is busy is kept for its next park, so a request is never slept
through. The `wake` closure `IpcServer::start` takes was already
driver-neutral; only what it calls changed. Frames are paced to one
per `HEARTBEAT_MS` of wall time, by a sleep before the draw — the
role a window's vsync-blocked present plays — so real-time continuous
work advances at a display's rate rather than spinning a core; an
idle headless app parks and costs nothing.

**Time starts virtual, with no window.** No one is watching time pass,
so it moves only when `clock.step` moves it: a headless run is a pure
function of its commands (D9's determinism contract), and
`clock.set_mode real` is one command away. A one-shot launch starts
virtual under a window too, so an animation can never keep it from
settling.

**Adapter choice: a plan of attempts over `request_adapter`, not a
re-ranking.** The issue asked for an explicit software-fallback order,
env-honoring, with `force_fallback_adapter` as a real option, and a
pure, tested preference function. Ranking `enumerate_adapters`
ourselves was rejected: a GL adapter is discovered through the surface
it will present to, and a surface-less enumeration re-ranked could
lose the only adapter a GL-only desktop has — the windowed path's
"untouched" requirement rules that out. So `renderer/adapter.rs` keeps
`request_adapter` as the mechanism (compatible with the window's
surface when there is one) and makes the *plan* pure: under `auto`,
the preferred adapter (CPU adapters are already eligible, ranked last,
so a GPU-less box lands on lavapipe on the first attempt), then a
software-only attempt when the first adapter could not open a device —
skipped when it was itself a software adapter, which would come back
again; under `--adapter software`, the software attempt alone. The
environment is wgpu's own: `WGPU_BACKEND` through
`InstanceDescriptor::with_env` (the skill had verified
`Instance::default()` ignores it), `WGPU_POWER_PREF` for the
preference (`high` when unset — what the renderer always asked for),
and `WGPU_ADAPTER_NAME` by wgpu's substring rule, ahead of the plan.
With none set and no flag, both windowed bootstraps ask exactly what
they asked before. The browser runs the same plan; it has no
environment, so the variables are absent there by construction.

**`Options`: no dormant fields.** `should_exit` ("short-circuit after
the first frame") is replaced by what its doc said it was for:
`one_shot`, the `--screenshot` capture whose condition is "settled on
a rendered frame, then exit", read by the loop body in both drivers.
`window_mode`, `ui_scale` and `launch_gpu_prefer_low_power`, which the
issue also named, had been deleted before this issue was built; the
power preference they stood for is `WGPU_POWER_PREF` now. The fields
`Options` carries are all read: `headless`, `surface_size`,
`adapter`, `one_shot`.

**What headless is not.** It is not a browser feature and owes no
browser parity: a browser page always has its canvas, and running one
without a display is the browser's own job (`CLAUDE.md` "Dual-target
status"). It adds no IPC command: the wire protocol is unchanged, and
an agent drives a headless app exactly as it drives a windowed one.
**Ending it.** A headless app has no window to close, so its graceful
exit is an `Action`: `Quit`, the funnel's name for what a window's
close button does — dispatched over IPC as `act.action Quit`, so no
command is added and `mandalactl` already speaks it. The arm sets
`InitState::quit_requested`; the loop body's last step sees it and
returns `Next::Exit`, and both drivers then run the one
`pump::shut_down` a closed window runs. It is destructive-classed, so
only the User tier — the user's own keybinds and macros, the console,
IPC — can reach it, and unbound by default, since it no more asks
about unsaved changes than a closed window does. A signal handler was
the alternative and was not taken: it needs a third sanctioned
boundary thread or an `unsafe` handler (§3), for a teardown that an
agent can as easily send as a command. A loop that ends with a
one-shot capture still owed — `Quit`, a closed window — ends with `1`.

## What each downstream issue consumes from this design

| issue | builds | consumes from IPC-01 |
|---|---|---|
| IPC-02 | transport + loop integration | D1 (socket/flag/lifecycle), D2 (threads, queues, `IpcWake`, watchdog rules), D3 (framing/caps/`hello`), D6 (cfg boundary), §3 amendment text |
| IPC-03 | registry + `act` bridge + gates | D5 (registry/`describe`), D4 (User posture, composition rule), D3 (envelope, error codes), D8 (warning sink plumbing) |
| IPC-04 | `query` family | D5 (module/table), D3, D8; camera accessor micro-overlap note |
| IPC-05 | `scene` family | D5, D3 (`to_file` discipline), coordinate spaces; canonical hit-test rule (never fork) |
| IPC-06 | `input` family | D5, D3; fidelity contract (same handler entry points as winit events); the injected-input queue + one-sample-per-frame pacing (shared with `settled`) |
| IPC-07 | screenshot engine | D7 shapes (params/sidecar/`space` grid/write-guards), offscreen mandate, #7 coordination |
| IPC-08 | recorder + `mandalactl record assemble` | D7 shapes (manifest), the frames-in-the-app / encoding-outside split, capture-clock `t_ms`, the virtual-clock frame-emission rule |
| IPC-09 | virtual clock + wait | D7 (`clock.*` shapes; `settled` keys on *rendered* not presented; wait exclusivity; watchdog-per-heartbeat; injected-input queue), D2 (deferred replies); builds D9 |
| IPC-10 | headless | D7 (offscreen capture path; `settled`-on-rendered makes headless capture terminate), `Options.should_exit` seam; app-shell freeze per EPIC #60; builds D10 |
| IPC-11 | `mandalactl` | D3 (protocol pinning/refusal; enum-widening is breaking), `format/ipc.md` as its command source |
| IPC-12/13 | skills | pinned capture/wait shapes; artifacts + manifest layout |
| IPC-14 | events + revision | D3 (event frames, reserved classes, envelope `revision`); the render-affecting revision counter + event backpressure coalescing; D8 (#45 boundary), #43 boundary (app-funnel only) |
| IPC-15 | WASM parity | D6 (parked trajectory: WebSocket transport reusing the *envelope + commands*; framing/caps re-expressed per transport), `wasm_compatibility` taxonomy |
| IPC-16 | docs sweep | Change-discipline section of `format/ipc.md`; `describe` byte-honesty audit |

## Amendment ledger

Documents moved by this design, in the same PR that pinned it:

- **CODE_CONVENTIONS §3** — the sanctioned IPC boundary: a
  persistent acceptor plus a per-controller reader + outbound pair
  (generation-stamped, torn down together on disconnect),
  `std::sync::mpsc` carrying protocol values only, proxy wake,
  main-thread-only command execution, no blocking IPC I/O on the
  main thread. The covenant's scope line ("in interactive paths")
  now has its second sanctioned boundary case after the watchdog.
- **CLAUDE.md "Dual-target status"** — section created (it was
  referenced by §4, CONCEPTS §1, and `freeze_watchdog.rs` but never
  existed); IPC carve-out entry added; existing documented
  native-only surfaces backfilled.
- **CONCEPTS.md** — §1 "Single-threaded event loop" and §5
  "FreezeWatchdog" both stop claiming the watchdog is the *only*
  sanctioned thread (they now point at the §3 amendment). §5
  "Dirty flag" is rewritten to match code — it is the
  unsaved-changes flag, not a rebuild trigger; drift caught while
  pinning D7's `settled` — and the event-loop step list drops its
  stale dirty-gated-rebuild claim.
- **`format/macros.md`** — SOURCE-OF-TRUTH tier list gains the
  IPC-maps-to-User entry; privilege-model section states the
  composition rule (D4).
- **`format/README.md`** — indexes `ipc.md` under a "Wire protocol"
  heading (it documents a protocol, not the on-disk map format —
  placed per IPC-01's instruction to mirror `macros.md`'s home).

Moved later, by the issue that found the need:

- **IPC-02 (#62)** — §D2's inbound ordering and reply backpressure
  (the "Amended by IPC-02" note there); `format/ipc.md` §Single
  controller and §Threading contract carry the wire-visible half, and
  CODE_CONVENTIONS §3's IPC bullet describes the as-built shape. §9's
  startup roster gains `EventLoop::with_user_event` and
  `IpcServer::start`.
- **IPC-03 (#63)** — §D8's warning sink becomes a tap on the log
  (the "Amended by IPC-03" note there), with a `kv` feature on the
  workspace `log` declaration to carry codes. `format/ipc.md`
  §Warnings gains the mechanism and a warning-code registry, and its
  stale `release_max_level_off` premise is corrected to #45's
  outcome. §D4 places `act.macro`'s anonymous `steps` at `User`.
  §D5's module layout and boundary trait are recorded as built.

- **IPC-14 (#74)** — D3's reserved `revision` and the "render
  revision" of D2 and D7, built as **two counters** because the issue
  and this design asked for different things and both are needed. The
  document's (`document_revision`, `MindMapDocument::revision`) is
  what the issue's acceptance criteria describe: committed model
  changes, untouched by previews, selection and camera, monotonic
  across `open` / `new`. The envelope's `revision` is D2/D7's: it
  advances once per *observation* in which any event class had
  something to report, so a view-only change moves it, and every
  reply carries both. What `settled` still needs and `revision` does
  not record is *rendered*: that the renderer has drawn a given
  `revision` stays IPC-09's, which now has the number to key on.
  Emission is by **observation, not instrumentation** (D2's "tap at
  the app funnels"): after each command, and once per loop iteration
  while subscribed, a read-only view of the app is compared with the
  last look, so no setter and no walker reports anything and human
  input is seen the same as an agent's. The funnels declare
  themselves on the document only to name a `document_changed`'s
  `source`. `animation_*`, `camera_changed`, `mode_changed` and
  `document_replaced` join the reserved classes; D3's "app is
  closing" is the existing `shutdown`. `log_line` is not shipped —
  its own design (what a flood of per-frame degrades does to a
  coalescing queue) is the follow-up. `format/ipc.md` §`events` is
  the contract; the geometry sidecar gains `document_revision`.
- **IPC-07 (#67)** — §D7's `capture.screenshot` shape, refined where
  building it found the pinned text ambiguous or silent, each in
  `format/ipc.md` §`capture.screenshot` in the same PR. (1) `scale` is
  image pixels per view pixel and never a zoom: the image is
  `round(width × scale) × round(height × scale)` of the same view,
  with zoom windows and connection glyph sizes decided at the view's
  zoom — carried by a `pixel_ratio` on Baumhard's `Camera2D`, so the
  renderer and the sidecar keep using the one cull predicate. (2) The
  bounds are refusals, as D7's own "over-bound … is `invalid_params`"
  says; the table's "clamped" wording is gone. (3) `fit` frames the
  drawn document as the load-time fit does — the issue's third framing,
  which D7's table lacked — and is exclusive with `region`; `space`
  without `region` and `overwrite` without `path` are `invalid_params`,
  the house rule for a qualifier with nothing to qualify. (4) The
  result reports `rendered` (with `skipped` naming a frame the
  renderer did not draw, and no file written), `scale`, the `camera`
  the image was drawn through, and `settled`. Waiting stays
  `clock.wait`'s: a screenshot reports whether the app was settled and
  never defers its reply. (5) A capture framed at another zoom was at
  first `busy` during a node drag, because its re-projection of the
  live scene would have drawn from a model the drag has not written;
  #178 moved that projection into a scratch scene of the capture's
  own, fed with the drag's offsets, and the refusal is gone — see
  `format/ipc.md` §`capture.screenshot` "State after a capture".
  (6) The offscreen target is the
  surface's own format — #7's single source, `config.format` — not a
  constant beside it.

- **IPC-08 (#68)** — §D7's recording pair, built and refined where
  building it found the pinned text silent or short, each in
  `format/ipc.md` §`capture.record_start` / `capture.record_stop` in
  the same PR. (1) **Assembly lives in `mandalactl`** (see the D7
  bullet above): `record assemble` makes the GIF or, through the
  host's `ffmpeg`, an MP4. (2) **The bounds are refusals**, as IPC-07
  made them for a screenshot: `fps` outside `1..=60` and `max_frames`
  outside `1..=3600` are `invalid_params`, not clamps; `width`,
  `height`, `region`, `space`, `fit` and `scale` are a screenshot's, by
  the same parser. (3) **The first frame is taken before the reply**,
  so it is the state the next command acts on; and every frame of a
  recording is the size the first one fixed, which a video needs.
  (4) **Pacing.** `t_ms` is the capture clock's reading when the frame
  was taken — the wall clock's in `realtime`, the one pacing today,
  named by the manifest's `pacing` key so `stepped` is a new value and
  not a rework — and a heartbeat that overran takes one frame for the
  latest boundary and counts the rest in `dropped_frames` and `gaps`.
  The assembler paces a GIF by those times, rounded so the error does
  not accumulate, and drops frames closer than the 20 ms browsers
  honor. (5) **Encoding is deferred** past the frame — raw frames wait
  in memory until 256 MiB of them have, and for `record_stop` — so a
  frame costs the loop a render and a readback; a gap is a stretch
  with no frame, never a silent drop, with its reason. (6) **A
  recording that ends on its own** (its `max_frames`, the artifacts
  dir's budget, a renderer that draws nothing for 100 frames, the app
  exiting) is kept until `capture.record_stop` collects it, so
  "stopping while idle is `invalid_params`" does not strand an agent
  that looked away. The manifest gains `complete`, `stopped`,
  `duration_ms`, `gaps`, and per-frame `camera`.

- **IPC-09 (#69)** — the design is D9, added by this issue as §D7's
  last bullet asked. Refined where building it found the pinned text
  silent or short, each in `format/ipc.md` §`clock` in the same PR.
  (1) **`ms` advances exactly `ms`**: the pinned `frames = ceil(ms /
  16)` is kept, and the last heartbeat lands on the step's end instead
  of overshooting it. (2) **A step needs a virtual clock** — on a real
  one it is `invalid_params` naming `cmd` — and a nested step, or a
  mode switch from inside a step, is `busy`. (3) **`clock.set_mode` is refused while a recording runs**,
  so a manifest's `pacing` (`"stepped"`, the value IPC-08 reserved the
  key for; the assembler paces by `t_ms` and never branched on it, so
  the new value is additive) means one thing for its whole length.
  (4) **A step stops at a recording's boundaries**, so stepped frames
  are their `1000/fps` instants, as §D7's frame-emission rule and
  IPC-08's manifest text already promised — each rounded up to a whole
  millisecond, which keeps the virtual clock on the whole milliseconds
  the animation tick reads, so a frame shows exactly the instant its
  `t_ms` names. The count per second is §D7's unchanged. (5) **`timeout_ms`
  outside `1..=60_000` is a refusal**, not a clamp, as IPC-07 and
  IPC-08 made theirs; `mandalactl --wait-settled` already sent no more
  than 60 000. (6) **`settled` (a) and (b) are clock-aware**: under a
  virtual clock an animation and the injected queue are owed to future
  instants, not now (D9). (7) **The render revision is a ledger** of
  redraws asked for and drawn, not the envelope's `revision`, which
  IPC-14 built to order replies against events; `settled` needed
  *drawn*, which only the asks can count. (8) **The mode is app
  state**, kept across controllers like a recording, because
  `mandalactl` is one command per connection. `clock.status` and `ping`
  answer before the window exists, with the real clock a new one is.

- **IPC-10 (#70)** — the design is D10, added by this issue. Refined
  where building it found the issue's sketch short, each in
  `format/ipc.md` §"Headless and one-shot launches" in the same PR.
  (1) **The pump runs the order winit ran**, not the issue's sketched
  "IPC first" order (D10). (2) **A headless frame draws nothing**,
  pixels on demand (D10). (3) **`--size`** sets the surface of either
  driver, and a headless app is 800×600 without it. (4) **`--adapter
  auto|software`** and wgpu's three variables are honored by every
  bootstrap, through a plan of `request_adapter` attempts rather than a
  ranking of our own (D10). (5) **The one-shot is `--screenshot <path>
  [--overwrite]`**, under either driver: settled, captured with
  `capture.screenshot`'s defaults and write guards, exit `0` / `1`; it
  and a headless launch start on a virtual clock. (6) **A headless
  launch needs `--ipc` or `--screenshot`**, or it is refused at
  startup. (7) **No new IPC command**: the graceful exit is a new
  `Action`, `Quit`, which `act.action` reaches. (8) **A one-shot
  fails loudly**: exit `1`, nothing written, for a map that did not
  load, a frame that cannot be drawn, and an app that ends before the
  capture.

## Change governance

This document and `format/ipc.md` change **together or not at all**:
semantics here, bytes there, `protocol` bump when breaking (D3).
Decisions D1–D8 were reviewed as the epic's human gate (EPIC #60
"Human-decision gates"); a downstream issue that finds a decision
unworkable stops and amends here first — implementation-side
workarounds of a pinned decision are the drift class this document
exists to prevent. No decision in this file is deferred, optional,
or "TBD"; if a section reads ambiguous, the ambiguity is a bug in
this file — fix the file.
