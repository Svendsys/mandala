# Macros

Macros are named, reusable sequences of user-driven actions that the
keybind layer can fire as a single unit. A macro is a list of
[steps](#step-kinds), executed in order; bind a key combo to a macro
id in `keybinds.json` and pressing the key runs the whole sequence.

Macros are a *user-authoring layer* on top of the existing dispatch
funnel — they don't introduce new behaviours, just orchestrate
existing ones (`Action`, `CustomMutation`, console verbs).

This document is the on-disk format reference. For the Rust-side
types see `src/application/macros/`; the dispatcher lives in
`src/application/app/dispatch.rs::dispatch_macro`.

## Where macros come from

Four loader tiers contribute to the active registry, in ascending
precedence (later writers override earlier ones with the same `id`):

<!-- SOURCE-OF-TRUTH: the tier order below is also encoded in
     three other places that must move together when the order or
     set of tiers changes:
       1. src/application/macros/mod.rs — MacroSource enum variant
          order.
       2. src/application/app/run_native_init.rs::build — the load
          order of the calls into MacroRegistry.
       3. src/application/app/console_input/exec.rs — the
          rebuild_map_macros invocation in the document-replace
          path.
     Update all four in the same commit. -->

1. **Application bundle** — `assets/macros/application.json`,
   compiled into the binary via `include_str!`. Lowest precedence so
   users can customise anything shipped by the app. Tier:
   `MacroSource::App`.
2. **User file** — `$XDG_CONFIG_HOME/mandala/macros.json` on native
   (falls back to `$HOME/.config/mandala/macros.json`). On WASM the
   user tier is not yet wired (deferred — see `TODO.md`). Tier:
   `MacroSource::User`.
3. **Map-inline** — `MindMap.macros` on the loaded document
   (initial load + every `open` / `new` console verb). On WASM
   this tier is not yet wired — opening a `.mindmap.json`
   in the browser silently ignores its `macros` array. Tier:
   `MacroSource::Map`.
4. **Node-inline** — `MindNode.inline_macros` on individual nodes.
   Loaded alongside Map tier (same trigger sites). Highest
   precedence — overrides Map / User / App on id collisions.
   Authors should namespace ids (e.g. `"node-id.action"`) to
   avoid collisions across nodes since the registry is
   id-keyed flat. Tier: `MacroSource::Inline`. WASM also not
   yet wired.

The `MacroSource` tier is **loader-pinned** — assigned at the
loader call site, never read from the on-disk content. A user
editing `~/.config/mandala/macros.json` cannot smuggle `App` tier
into their entries.

### Within-tier and cross-tier collision semantics

Two entries with the same `id` in the same tier (e.g. a Map-tier
`macros` array containing two entries both `id: "x"`) follow
last-writer-wins — the second entry overwrites the first.
Authoring tip: keep ids unique within a single source.

Cross-tier: a higher-tier entry **displaces** a lower-tier entry
with the same id at registry-insert time — the lower-tier entry
is removed from the HashMap, not stacked under it. So:

- Open document A with `Map`-tier `id: "save-and-quit"` shadows the
  user's `User`-tier `id: "save-and-quit"`.
- Open document B with no `macros` → `clear_tier(Map)` runs,
  removing the Map entry — but the User entry is **not** restored.

The displacement is permanent within the session. To avoid
this, Map-tier authors should namespace their ids
(e.g. `"my-map.save"` rather than bare `"save"`). A future
shadow-stacked registry could fix this; tracked in `TODO.md`.

## Privilege model — read this before shipping a non-User loader

The dispatcher gates two surfaces on `MacroSource` tier:

- **`MacroStep::ConsoleLine`** runs an arbitrary console verb
  (`save`, `open`, `mutation apply`, kv-shaped style setters, etc.).
  User-tier-only via `MacroSource::allows_console_line`.
- **Destructive / I/O / clipboard `Action` variants**
  (`SaveDocument`, `DeleteSelection`, `Cut`, `Paste`, `Copy`,
  `OrphanSelection`, `CreateOrphanNode`, `CreateOrphanNodeAndEdit`,
  `NewDocument`) are User-tier-only via
  `MacroSource::allows_action`. Navigation / view-state Actions
  (zoom, pan, selection traversal, undo, etc.) pass freely.

Privilege rejections **fail-closed** — the dispatcher aborts the
rest of the macro on a rejected step so a hostile pattern like
`[DeleteSelection, ConsoleLine(rejected), SaveDocument]` cannot
sneak its outer steps past the gate. Honest mistakes (unbound
action, no document loaded) keep the existing best-effort
`continue` semantic.

### Threat model

Once Map and Inline tiers ship, **opening any `.mindmap.json` from
an untrusted source becomes a privilege event**. Treat third-party
mindmap files as code, not data:

- A hostile mindmap can bind any non-destructive Action to a hotkey
  the user is likely to press (Enter, Tab, etc.).
- A hostile mindmap can carry CustomMutation steps that mutate
  document state in surprising ways (the changes are in-memory and
  undoable; the worst case is an annoying user experience, not data
  exfiltration).
- A hostile mindmap CANNOT run console verbs, save the file, delete
  selection, or touch the clipboard — those are all gated above.

If a future contributor adds a `DocumentAction` variant that
performs file I/O, network access, or arbitrary content load, the
gate must extend to cover it. `DocumentAction` is `#[non_exhaustive]`
specifically to surface this when the variant is added — see
`lib/baumhard/src/mindmap/custom_mutation/mod.rs`.

## On-disk format

Macros are loaded as a top-level JSON array of macro objects.
Application-bundle, user-file, and map-inline tiers all use this
same shape; tier is assigned by the loader, not the file.

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
    "id": "tag-as-inbox",
    "name": "Tag as Inbox",
    "steps": [
      { "kind": "CustomMutation", "id": "set-tag-inbox" },
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
`src/application/keybinds/action.rs`. Examples: `"Undo"`,
`"SaveDocument"`, `"ZoomReset"`, `"SelectAll"`,
`"DoubleClickActivate"`. Modal-context actions (`TextEditCursorLeft`,
`PickerCommit`, `ConsoleSubmit` etc.) are accepted but only fire
when the matching modal is open.

Some Actions are gated for non-User tiers — see
[Privilege model](#privilege-model--read-this-before-shipping-a-non-user-loader).

#### `CustomMutation`

Apply a custom mutation by id, against a resolvable target node.
Routes through the same path keybind-triggered custom mutations
use (animation-aware via `cm.timing`, always invokes
`apply_document_actions`).

```json
{ "kind": "CustomMutation", "id": "nudge-right" }
```

```json
{
  "kind": "CustomMutation",
  "id": "highlight-trunk",
  "target": { "node_id": "1.2.3" }
}
```

| field | type | notes |
|---|---|---|
| `id` | `string` | Required. Must resolve in the document's `mutation_registry`; unknown ids log a `warn!` and the step is skipped. |
| `target` | `MacroTarget` | Optional, defaults to `"current_selection"`. |

`MacroTarget` is one of:
- `"current_selection"` — use the currently-selected single node.
  Multi / edge / portal selections cause the step to skip.
- `{ "node_id": "..." }` — always target the named node. The
  dispatcher checks the id exists in the document; missing ids
  log a `warn!` and the step is skipped.

#### `ConsoleLine`

**User-tier-only.** Re-parse the given line through the console
parser and run it as if typed. Lets macros leverage every
parameterised verb (`border preset=triple`, `color bg=#fafafa`,
etc.) without needing a bespoke step kind.

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

## Resolution order at dispatch

When a key combo fires, the keyboard handler walks three resolution
tiers in order:

1. `keybinds.action_for_context(...)` — built-in `Action` variants.
2. `keybinds.macro_for(...)` — user-defined macros from
   `macro_bindings`. Unknown macro ids fall through to tier 3.
3. `keybinds.custom_mutation_for(...)` — per-node custom mutations
   from `custom_mutation_bindings`.

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

Each loader is best-effort. Failures (missing file, malformed
JSON, unknown step kinds, invalid action names) log a `warn!` and
the loader returns an empty / partial slice. The application boots
even if the macros file is broken. The application bundle is a
build-time invariant and is parsed with `expect()` — a malformed
bundle is a startup-time bug, not a user input error.
