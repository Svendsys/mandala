# P2-43: Baumhard seam hygiene — EventSubscriber's Send+Sync/Mutex shape fights the single-threaded design; Tree's phantom fields force a crossbeam channel on every constructor; half-exposed terminator seam; unrecoverable SceneEntry

**Severity:** P2 (API shapes that block the named plugin/script trajectory; cheapest to fix now with zero consumers) · **Area:** baumhard/gfx_structs

## Problem A — EventSubscriber shape

`tree.rs:36-40`: `pub type EventSubscriber = Arc<Mutex<dyn FnMut(&mut GfxElement, GlyphTreeEventInstance) + Send + Sync>>`. The event seam is sanctioned (reserved for the script API), but the shape is wrong for it:
- `Send + Sync` forces every future plugin closure to be thread-safe in a §3 single-threaded app — excluding closures capturing `Rc`/`RefCell` app state, the very state a plugin wants.
- `accept_event` does `sub.lock().expect(...)` (`element.rs:576-583`) — a panic on an interactive dispatch path if a prior subscriber panicked while locked, and a **guaranteed self-deadlock** if a subscriber's reaction re-delivers an event to itself (std Mutex is not reentrant; single-threaded means deadlock, not race).
- Every delivery clones the whole subscriber Vec — an allocation per event per element.
- `Flag::MutationEvents`' field doc reads as implemented behavior ("all mutations should also create a corresponding event"); no walker code checks the flag.

**Fix:** `Rc<RefCell<dyn FnMut(...)>>`; `try_borrow` + log-skip (turns re-entrancy into a warn); index-iterate or scratch-buffer instead of the Vec clone; reword the MutationEvents doc to "reserved; not yet emitted".

## Problem B — Tree's phantom fields and the forced channel

`tree.rs:108-152, 179`: `position`, `pending_mutations`, `region_params`, `region_index` are private, `#[allow(dead_code)]`, written at construction, with **no accessor anywhere** — not attachable seams, and CONCEPTS describes them as "used narrowly today" (false). `Tree::new` demands a `crossbeam_channel::Sender<RegionElementKeyPair>` that is documented "not currently wired"; its only caller in the workspace is one test that builds an `unbounded()` channel solely to satisfy the signature — in a codebase whose architectural invariant is "no channels" (§3). `pending_mutations` uses `Arc` beside `region_index: Rc` (mixed threading postures). `Tree::import` has zero callers and mutates the arena **without** `invalidate_caches()` — the one in-crate violation of the invalidation discipline the same file documents.

**Fix:** delete `position` (Scene's per-entry `offset` is the live implementation of the same concept) and `pending_mutations`, or expose them as a real pub surface; remove the `_scene_index_sender` parameter (and with it the crossbeam-channel dependency — coordinate with P2-40); `import`: add invalidation + a caller/test, or delete. Align `Arc`→`Rc`.

## Problem C — related §B6 doc-vs-reality decision

CONVENTIONS §B6 and CONCEPTS describe the region index as "maintained as a side effect of MutatorTree::apply_to" — no such maintenance code exists anywhere; production trees are all `new_non_indexed*`. Either wire the seam minimally (apply_to updates `region_index` when present) or rewrite §B6/CONCEPTS/`RegionElementKeyPair` docs to "tested-but-unwired subsystem". A convention document asserting a nonexistent invariant is the most dangerous drift in a foundation crate.

## Problem D — small API traps

- `DEFAULT_TERMINATOR` is pub "so mutator authors can substitute custom terminators", but every function accepting a terminator is private and the call site hardcodes it — unattachable. Make `repeat_while` pub with the documented parameter, or make the constant private and fix the module doc.
- `Scene::remove` returns `SceneEntry` "returning ownership", but its fields are private and accessors borrow — a removed tree can only be cloned. Add `SceneEntry::into_tree(self)`.
- `ColorFontRegions::hard_get` — a panicking "test-only" helper living as an ordinary pub method beside `get`. Move into the tests tree (still bench-reachable per §T2.2) or `#[doc(hidden)]`.
- `InstructionSpec` shadows the already-serde-able `Instruction` 1:1 plus one sugar variant — collapse via serde alias/from-shim, or document the sugar rationale at the type and align the lone `#[non_exhaustive]` on `MutationListSrc` with its siblings.

## Acceptance criteria

- A plugin-shaped closure capturing `Rc<RefCell<...>>` state can subscribe (compile test).
- `Tree::new` takes no channel; crossbeam-channel gone from baumhard's manifest.
- §B6 text matches code (either direction).
- `./test.sh` green; doc build (`cargo doc -p baumhard --no-deps`) clean.

## Pointers

CODE_CONVENTIONS §3 (single-threaded, no channels), §7 (seam ≠ shape; seams must be attachable); CONVENTIONS §B6, §B10; files cited inline.
