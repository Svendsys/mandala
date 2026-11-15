# Macros

Macros are named, reusable sequences of user-driven actions that the
keybind layer can fire as a single unit. A macro is a list of
[steps](#step-kinds), executed in order; bind a key combo to a macro
id in `keybinds.json` and pressing the key runs the whole sequence.

Macros are a *user-authoring layer* on top of the existing dispatch
funnel — they don't introduce new behaviors, just orchestrate
existing ones (`Action`, edits, console verbs).

This document is the on-disk format reference. For the Rust-side
types see `src/application/macros/`; the dispatcher lives in
`src/application/app/dispatch/macro_core.rs::dispatch_macro`
(cross-platform, abstracted over the `MacroDispatchTarget` trait —
the native shim that wraps `InputHandlerContext` is at
`src/application/app/dispatch/native.rs::dispatch_macro`).

## Where macros come from

Two loader tiers contribute to the active registry, in ascending
precedence (a later writer overrides an earlier one with the same
`id`):

<!-- SOURCE-OF-TRUTH: the tier order below is also encoded in
     four other places that must move together when the order or
     set of tiers changes:
       1. src/application/source_tier.rs — the SourceTier enum
          variant order, pinned by that module's tests.
       2. src/application/macros/loader/ — build_macro_registry and
          the parse_* helpers, which both targets' init call
          (app/run_native_init.rs::build, app/run_wasm/mod.rs::run).
       3. src/application/macros/mod.rs — MacroRegistry, whose
          lookup walks the tiers in this order.
       4. format/ipc.md §"Trust model" + work_plans/LLM_IPC.md §D4
          — the IPC surface is pinned to the User tier (it is
          deliberately NOT a tier of its own); adding or reordering
          tiers must re-evaluate that mapping.
     Update all five in the same commit. -->

1. **Application bundle** — `assets/macros/application.json`,
   compiled into the binary via `include_str!`. Lowest precedence so
   users can customize anything shipped by the app. Tier:
   `SourceTier::App`. Cross-platform (native + WASM).
2. **User file** — `$XDG_CONFIG_HOME/mandala/macros.json` on native
   (falls back to `$HOME/.config/mandala/macros.json`). On WASM,
   `localStorage` under the `mandala_macros` key > empty. There is
   no `?macros=` URL parameter: a link is composed by whoever sent
   it, and this tier may run console verbs. Both targets parse through
   the shared `loader::parse_user_macros_json`. Tier:
   `SourceTier::User`.

**A map carries no macros.** The old format's map and node-inline
tiers were deleted with it (V1-C3, #244); a map's own behavior is its
verbs (V1-D3, #252), which replace them. Opening another map therefore
leaves the registry as it was.

The `SourceTier` of a macro is **loader-pinned** — assigned at the
loader call site, never read from the on-disk content. A user
editing `~/.config/mandala/macros.json` cannot smuggle `App` tier
into their entries.

### Within-tier and cross-tier collision semantics

Two entries with the same `id` in the same tier: the second wins. Cross-tier: a User entry
**shadows** an App entry with the same id — both coexist in their own
tier slots, and lookup returns the User one. A User entry that fails
to parse is skipped (see [Loader resilience](#loader-resilience)), so
the App entry it would have shadowed stays in effect.

## Privilege model — read this before shipping a non-User loader

The dispatcher gates two surfaces on the macro's `SourceTier`:

- **`MacroStep::ConsoleLine`** runs an arbitrary console verb
  (`save`, `open`, `new`, the clock and capture verbs, etc.).
  User-tier-only via `SourceTier::allows_console_line`.
- **Destructive / I/O / clipboard `Action` variants**
  (`SaveDocument`, `DeleteSelection`, `Cut`, `Paste`, `Copy`,
  `OrphanSelection`, `CreateOrphanNode`, `CreateOrphanNodeAndEdit`,
  `NewDocument`, `Quit`) are User-tier-only via
  `SourceTier::allows_action`. Navigation / view-state Actions
  (zoom, pan, selection traversal, undo, etc.) pass freely.

Privilege rejections **fail-closed** — the dispatcher aborts the
rest of the macro on a rejected step so a hostile pattern like
`[DeleteSelection, ConsoleLine(rejected), SaveDocument]` cannot
sneak its outer steps past the gate. Honest mistakes (unbound
action, no document loaded) keep the existing best-effort
`continue` semantic.

**IPC is not a tier.** Commands arriving over the `--ipc` socket
([`format/ipc.md`](./ipc.md)) execute at `User` posture — the user
owns the flag exactly as they own this file. Macros fired *via* IPC
(`act.macro` with a `macro_id`) keep their own loader-pinned tier:
IPC initiates, it never escalates, and the fail-closed gates above
run unchanged. An anonymous step list sent as `act.macro`'s `steps`
was authored by the IPC client, so it runs at `User`. It runs
through the same gated step loop, not a second one. A gate
rejection is logged under the code `macro_gate_rejected`, which an
IPC reply carries as a warning. Rationale: `work_plans/LLM_IPC.md`
§D4.

### The load-time trust boundary

The privilege gates above answer "what may a macro *do*". A map is the
other input a user opens from wherever they got it, and since V1-C3 it
is read by the `.mandala.json` loader, which is strict: every key is
defined, every value is typed and bounded, and a file is either valid or
does not load ([`schema.md`](./schema.md) §"Bounds"). A number the
renderer would trust — a zero font size, an inverted clamp, absurd
geometry — is refused at the door with a message naming the part to
fix, not repaired. `MAX_MAP_BYTES` bounds the read itself before it
commits to the allocation.

### Threat model

**A map carries no macros and binds no keys** since V1-C3: the map and
node-inline tiers went with the old format. What a third-party map can
do is be drawn. Its own behavior returns as verbs and triggers (V1-D3,
#252), which bring their own gate; until then the capability surface of
this document is the user's file and the application bundle, both of
which the user owns.

**What is not yet closed**, so nobody reads the section above as a
finished job:

- **The browser's `?map=` takes a whole URL, not a path.** The value
  goes to `fetch` verbatim, so an absolute one loads a third-party map
  cross-origin (the attacker's own server decides its CORS). The
  content is screened like any other — the browser runs the same
  loader — but the *provenance* step is gone: there is no file the user
  chose to open, only a link they clicked. Restricting the parameter to
  a same-origin path is a product decision, not a bug fix, which is why
  it is named here rather than changed.

## On-disk format

Macros are loaded as a top-level JSON array of macro objects. The
application bundle and the user file use this same shape; tier is
assigned by the loader, not the file.

```json
[
  {
    "id": "save-and-zoom-out",
    "name": "Save and Zoom Out",
    "description": "Persist the current map and back off the camera.",
    "steps": [
      { "kind": "Action", "action": "SaveDocument" },
      { "kind": "Action", "action": "ZoomOut" }
    ]
  },
  {
    "id": "grow-and-save",
    "name": "Grow and save",
    "steps": [
      { "kind": "Edit", "id": "grow-font-2pt" },
      { "kind": "ConsoleLine", "line": "save" }
    ]
  }
]
```

### `Macro` fields

| field | type | notes |
|---|---|---|
| `id` | `string` | Required. Lookup key. Higher-tier macros override lower-tier ones with the same id. |
| `name` | `string` | Optional, defaults to `""`. Display label for future macro pickers / inspection. |
| `description` | `string` | Optional, defaults to `""`. Human-readable explanation for the same. |
| `steps` | `[MacroStep]` | Required. The ordered sequence executed when the macro fires. |

### Step kinds

Each step is an object with a `"kind"` discriminator (internally
tagged) plus the kind-specific fields.

#### `Action`

Run a built-in `Action` against the current `InputHandlerContext`.
Routes through the same `dispatch_action` that keyboard / mouse
input use.

```json
{ "kind": "Action", "action": "SelectAll" }
```

The `action` value is the variant name from
`src/application/keybinds/action/mod.rs`. Examples: `"Undo"`,
`"SaveDocument"`, `"ZoomReset"`, `"SelectAll"`,
`"DoubleClickActivate"`. Modal-context actions (`TextEditCursorLeft`,
`PickerCommit`, `ConsoleSubmit` etc.) are accepted but only fire
when the matching modal is open.

Some Actions are gated for non-User tiers — see
[Privilege model](#privilege-model--read-this-before-shipping-a-non-user-loader).

#### `Edit`

Run an edit ([`edits.md`](./edits.md)) by id at a target: the funnel a
key bound in `edit_bindings` and the console's `edit apply` take
(`dispatch::families::edits::run_edit`). The edit is found across its
tiers (App < User < Map), resolved against the open map, and applied
as one history entry — or, when it animates, staged and committed as
one entry at its end. Every tier may run one: an edit is an undoable
document change with no I/O. A step whose edit is unknown, or names
something the map lacks, is logged and does not count as run.

```json
{ "kind": "Edit", "id": "nudge-right-50" }
```

```json
{
  "kind": "Edit",
  "id": "flower",
  "target": { "node_id": 12 }
}
```

| field | type | notes |
|---|---|---|
| `id` | `string` | Required. The edit's id. |
| `target` | `MacroTarget` | Optional, defaults to `"current_selection"`. |

`MacroTarget` is one of:
- `"current_selection"` — the current selection.
- `{ "node_id": <integer> }` — the node with that id; a map without
  it skips the step.

#### `ConsoleLine`

**User-tier-only.** Re-parse the given line through the console
parser and run it as if typed. Lets macros leverage every
parameterized verb (`fps on`, `clock virtual`, `screenshot`, etc.)
without needing a bespoke step kind.

```json
{ "kind": "ConsoleLine", "line": "save" }
```

ConsoleLine requires a loaded document. Macros fired before any
document loads silently skip the step with a `warn!` — bind paths
through CLI args / `?map=` query rather than a startup-hotkey
macro.

A non-User-tier macro that contains `ConsoleLine` steps will be
**rejected entirely** (fail-closed); the dispatcher logs a `warn!`
and aborts the macro after the first rejected step.

##### ConsoleLine on WASM

WASM has no console runtime — the `console_input` module is
`#[cfg(not(target_arch = "wasm32"))]`-gated, so there's no
`execute_console_line` to call. **User-tier ConsoleLine steps on
WASM log `warn!` and skip; the macro continues with the next
step** (fail-soft, NOT abort). This matches the User-tier "step
failed" posture used elsewhere (e.g. an unhandled Action),
and prevents a copy-pasted-from-desktop macro shaped
`[Action::ZoomIn, ConsoleLine("save"), Action::ZoomIn]` from
fail-closed-aborting at step 2 and never running step 3.

The privilege gate above is unchanged — the non-User tier
(`App`) still fail-closed-aborts the macro on
the FIRST ConsoleLine encounter, identical to native. Only the
User-tier branch differs in *what* execution looks like
(log-skip on WASM, real exec on native).

## Resolution order at dispatch

When a key combo fires, the keyboard handler walks three resolution
tiers in order:

1. `keybinds.action_for_context(...)` — built-in `Action` variants.
2. `keybinds.macro_for(...)` — user-defined macros from
   `macro_bindings`. Unknown macro ids fall through to tier 3.
3. `keybinds.edit_for(...)` — edits from `edit_bindings`, run at the
   selection ([`edits.md`](./edits.md)).

A built-in `Action` wins over a colliding macro binding. To
override a built-in Action with a macro, first unbind the Action's
keybind:

```json
{
  "copy": [],
  "macro_bindings": { "Ctrl+C": "my-extended-copy" }
}
```

## Loader resilience

Each loader is best-effort, and a broken part costs only that part.
In the user file the unit is the **macro**: an entry that does not parse (unknown step kind, invalid
action name, missing `steps`) logs a `warn!` naming its index and
`id` and is skipped, and every other macro in the same list loads.
A macro is never loaded with a step missing — that would run a
different program under the same id. Only a user file that is not a
JSON array at all (or is not JSON) yields an empty user tier, again
with a `warn!`; an empty file is an empty tier. A skipped user macro
that overrode an App-tier macro of the same id leaves the App macro
in effect. A field written twice inside a macro in the user file uses
its last occurrence and logs a `warn!` naming its path (`[2].id`). The
application boots even if the macros file is broken.
The application bundle is a build-time invariant and is parsed with
`expect()` — a malformed bundle is a startup-time bug, not a user
input error.
