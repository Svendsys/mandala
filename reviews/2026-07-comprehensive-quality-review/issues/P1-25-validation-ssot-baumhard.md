# P1-25: Section/node validation rules and exact messages are hand-mirrored byte-for-byte between the app crate and maptool — move them into baumhard

**Severity:** P1 (SSOT; drift already exists) · **Area:** baumhard model + mandala document + maptool verify · **Found independently by two reviewers**

## Problem

`crates/maptool/src/verify/sections.rs:63-309` and `src/application/document/nodes/mod.rs:747-845+` implement the same validation rules with **identical format strings**, hand-maintained in two crates:

- `"section[{}].offset.x is negative ({})"`
- `"section[{}] extends past node right edge ({} > {})"`
- `"node.size has non-finite component (width={}, height={})"`
- the "over 100× the node's width … likely a typo" message
- …and the rest of the section-AABB family.

Byte-equality is held only by substring assertions in `tests_nodes.rs:151-289`; `format/sections.md:147-152` *celebrates* the byte-equality — sync-by-discipline, not by construction. Drift already exists:

- The app enforces `MAX_NODE_AXIS = 1_000_000.0` on every setter (`nodes/mod.rs:854-870`); verify has no node ceiling — `width: 1e30` passes verify while the app refuses to produce it.
- `MAX_SECTIONS_PER_NODE = 1024` is duplicated as two constants (`document/mod.rs:104-106` vs `verify/sections.rs:97-113`), and verify's docstring claims an impossible OOM-defense rationale (verify must fully parse before checking; the loader — the actual entry point — has no cap).
- `verify/sections.rs:135` re-derives the effective-channel rule `section.channel.unwrap_or(idx)` with a comment admitting it copies the tree builder.

The repo already proved the right pattern by hoisting `MindSection::effective_size` into the model "so the two cannot drift" (`node.rs:381-390`) — then left the other eight checks, the cap, the channel rule, and all messages duplicated.

## Fix plan

1. Add `lib/baumhard/src/mindmap/model/validate.rs`: `pub fn node_size(node) -> Result<(), String>`, `pub fn section_aabb(node, idx) -> Result<(), String>`, `pub fn section_count(node) -> Result<(), String>`, with the constants (`MAX_NODE_AXIS`, `MAX_SECTIONS_PER_NODE`) as `pub const` beside them, and `MindSection::effective_channel(idx)` beside `effective_size`.
2. maptool verify and the document setters both call these; delete the app copies and the local consts. Keep the exact current message strings (tests pin them).
3. Enforce the section cap in the **loader** (the honest entry point) or reword verify's rationale; document the cap in validation.md (overlaps P1-14 item 4-5 — coordinate).
4. Add verify's missing node-size ceiling via the now-shared constant.

## Acceptance criteria

- One implementation of each rule; app + maptool import it (grep: format strings exist exactly once).
- Existing message-pinning tests pass unchanged.
- `./test.sh` green.

## Pointers

`crates/maptool/src/verify/sections.rs`; `src/application/document/nodes/mod.rs:747-870`; `src/application/document/mod.rs:104-106`; `lib/baumhard/src/mindmap/model/node.rs:381-390` (the exemplar); CODE_CONVENTIONS §1 ("missing primitives are added to Baumhard"), §5; format/sections.md, format/validation.md.
