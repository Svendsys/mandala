# Unified Architecture — issue index

The issues on GitHub are the specification of record for each piece
of work; [`UNIFIED_ARCHITECTURE.md`](./UNIFIED_ARCHITECTURE.md) is the
design they implement. This file only indexes them.

**Level** is difficulty and criticality combined, 1–5: 1 is mechanical,
5 is on the critical path, subtle, with a large blast radius.

## V1 — Unified architecture: [#230](https://github.com/Svendsys/mandala/issues/230)

Every V1 sub-issue targets `epic/unified-mutations`, except V1-E5,
which merges that branch into `main`.

| Id | Issue | Title | Model | Level | Blocked by |
|---|---|---|---|---|---|
| A1 | #231 | Generic engine: tree, mutator, walker, predicate, builder | Opus | 5 | — |
| A2 | #232 | Routing keys, sorted-children invariant, anchored walks | Opus | 4 | #231 |
| A3 | #233 | Journals (exact revert/replay) and interpolation | Opus | 5 | #232 |
| A4 | #234 | Structural commands, journaled | Opus | 5 | #233 |
| B1 | #235 | Document: kinds, typed values, identity, inheritance | Opus | 4 | #232 |
| B2 | #236 | Format: strict `.mandala.json`, loader, saver | Sonnet | 3 | #235 |
| B3 | #237 | Document mutation vocabulary | Opus | 4 | #234, #235 |
| B4 | #238 | Document writer: apply, history, transactions, change feed | Opus | 5 | #237 |
| B5 | #239 | TextFlow: shaping, outlines, placement, fit modes | Opus | 5 | #235 |
| B6 | #240 | Glyph instantiation: `GlyphRun` and `Fill` | Sonnet | 3 | #239 |
| B7 | #241 | Projection: incremental, captured dependencies, oracle | Opus | 5 | #238, #240 |
| C1 | #242 | Renderer spike: positioned glyphs from one run | Opus | 4 | #240 |
| C2 | #243 | Renderer: glyph layers, journal-driven updates | Opus | 5 | #242, #241 |
| C3 | #244 | App cut-over; the old model is deleted | Opus | 5 | #236, #241, #243 |
| C4 | #245 | Structure editing | Opus | 4 | #244 |
| C5 | #246 | Field editing I: nodes, sections, frames, theme, defaults | Sonnet | 3 | #244 |
| C6 | #247 | Field editing II: connectors, links, labels, portals | Sonnet | 3 | #244 |
| C7 | #248 | Transactions: drags, resizes, chrome, color picker | Opus | 4 | #244 |
| C8 | #249 | Text editing on spans | Opus | 5 | #244 |
| D1 | #250 | Edits | Opus | 4 | #244 |
| D2 | #251 | Presentations | Opus | 5 | #244 |
| D3 | #252 | Map verbs, triggers, links | Opus | 4 | #250, #251 |
| E1 | #253 | maptool on the Document | Sonnet | 2 | #244 |
| E2 | #254 | IPC: journal events, protocol 2 | Sonnet | 3 | #252 |
| E3 | #255 | Feature-parity audit and retired-concepts gate | Opus | 3 | #245–#249, #252, #253, #254 |
| E4 | #256 | Documentation rewrite | Opus | 4 | #255 |
| E5 | #257 | Switch-over merge into `main` | Sonnet | 2 | #256 and all others |

**Parallel lanes.** After A2, the engine lane (A3 → A4) and the
document lane (B1 → B2, B5 → B6) run side by side. After C3, the nine
issues C4–C8, D1, D2 and E1 run in parallel; they own disjoint files
(C3's ownership table).

## V2 — Deferred features: [#258](https://github.com/Svendsys/mandala/issues/258)

Nothing starts before V1 merges (#257). Each opens with design
questions the owner answers on the issue.

| Id | Issue | Title | Model | Level |
|---|---|---|---|---|
| V2-1 | #259 | Buttons on nodes that invoke map verbs | Opus | 4 |
| V2-2 | #260 | Triggers: keys, map load, timers, sequences | Opus | 4 |
| V2-3 | #261 | Verb parameters | Sonnet | 3 |
| V2-4 | #262 | In-app authoring of edits, presentations, verbs | Opus | 5 |
| V2-5 | #263 | Node shadow | Sonnet | 2 |
| V2-6 | #264 | Hexagon, diamond, parallelogram shapes | Opus | 3 |
| V2-7 | #265 | `RotateWhile` instruction | Opus | 3 |
| V2-8 | #266 | Create a child or sibling node | Sonnet | 2 |
| V2-9 | #267 | Fold and unfold | Sonnet | 2 |
| V2-10 | #268 | Section merge and reorder | Sonnet | 2 |
| V2-11 | #269 | Touch platform context for triggers | Sonnet | 3 |
| V2-12 | #270 | Per-glyph function effects | Opus | 4 |
| V2-13 | #271 | Screen-space text-size clamps | Sonnet | 3 |
