# P2-49: User-tier config loaders — size-capped read and layered-fallback drivers copy-pasted three times; MacroSource/MutationSource tier duplication unpinned

**Severity:** P2 (three identical shapes, a fourth coming with any new user-tier config) · **Area:** mandala config loading

## Problem

The primitives ARE shared (`user_config::{MAX_USER_PAYLOAD_BYTES, payload_within_cap, xdg_mandala_path, web_storage}`) — but the composed logic is tripled, each copy's header saying "Mirrors ..." (sync-by-comment):

- **Desktop capped-read** (stat → cap → read → parse, byte-identical error strings incl. `"{} exceeds size cap ({} bytes > {} max); refusing to load"`): `document/mutations_loader/platform_desktop.rs:50-70`, `keybinds/platform_desktop.rs:25-40`, `macros/loader/platform_desktop.rs:40-62`.
- **Web layered driver** (query-param → cap → parse → localStorage → cap → parse → default): `mutations_loader/platform_web.rs:26-50`, `keybinds/platform_web.rs:23-47`, `macros/loader/platform_web.rs:29-53`.
- **Desktop layered driver** (explicit > XDG > default): mutations_loader vs keybinds.

Adjacent: `MacroSource` (`macros/mod.rs:25-65`) duplicates `MutationSource` (`mutations_loader/mod.rs:38-58`) — same four variants, same ordering contract, "Mirrors" comment, and the loader header explicitly says "Changes to the set or order update all three sites in the same commit" (sync-by-discipline). And the border-preset completion hint table has a silent `_ => ""` fallback for future presets while the rest of the preset pipeline is exemplary single-table (`border/complete.rs:86-95` vs `border.rs:1155`).

## Fix plan

1. `user_config::read_capped(path, label) -> Result<String, String>` — one implementation of the stat/cap/read/error shape; three desktop loaders call it.
2. A generic web driver `load_web_layered<T>(param_name, storage_key, parse) -> Option<T>` (or a small builder) for the three web loaders; same for the desktop explicit>XDG>default chain if it generalizes cleanly (don't force it — two sites may stay).
3. Either a shared `SourceTier` enum consumed by both Macro/Mutation source types, or (minimum) a test pinning variant-set + order equality so the three-site sync-by-comment becomes compiler/test-enforced.
4. Border hints: fold hint strings into the baumhard preset table or add a non-empty-hint-per-preset test.

## Acceptance criteria

- The cap error string exists exactly once (grep).
- Adding a fourth user-tier config file requires no new read/cap/fallback code.
- Tier-order pin test in place.
- `./test.sh` green.

## Pointers

`src/application/user_config/`; the six platform loader files cited; CODE_CONVENTIONS §2 ("repetition of shape is a smell"), §5.
