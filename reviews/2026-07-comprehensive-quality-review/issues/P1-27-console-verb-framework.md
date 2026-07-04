# P1-27: Console verb framework — the same grammar is hand-rolled 2-4 times per verb across 20 verbs (parse, complete, usage, hints); replace with a declarative arg-spec

**Severity:** P1 (top-3 duplication surface; drift shipped repeatedly) · **Area:** mandala/console

## Problem

Every verb re-implements the same scaffolding by hand, so one grammar lives in 2–4 places per verb (≥5 verbs cited per element; full site lists in the console findings file):

- **Subverb match + unknown-subverb error**: `section/mod.rs:230-338`, `border/execute.rs:24-142`, `canvas.rs:198-262,469-519`, `font.rs:185-204`, `label.rs`, `mode.rs`, `node.rs`, `mutation.rs`, `zoom.rs`, `fps.rs`, `section/frame.rs`.
- **kv loop + unknown-key rejection**: `section/mod.rs:885-942, 977-1012, 410-422`; `font.rs:211-272`; `zoom.rs:126-149`; `border/execute.rs:92-104`; `section/frame.rs:141-160`; `canvas.rs:250-262,507-519`; `label.rs`, `anchor.rs`, `cap.rs`, `edge.rs`.
- **Usage strings duplicated between `Command.usage` and inline errors — already drifting**: `section/mod.rs` has four variants (:59 vs :235 vs :922 vs :1007); `font.rs:61` omits `section=`/`range=` that its own error at :502 documents; `color.rs:35` omits `picker on|off`/`section=`/`range=`.
- **Per-key hint tables**: `font.rs:116-124`, `color.rs:66-74`, `section/mod.rs:214-228`, `border/execute.rs:735-748` (+ delegation shims).
- **Per-verb `complete` matches re-encoding the execute grammar positionally**: `border/complete.rs:12-56`, `canvas.rs:87-168` (hand-maintained token-index arithmetic), `section/mod.rs:75-138`, `font.rs:70-106`, `color.rs:42-64`, `zoom.rs:63-93`, `mutation.rs:33-77`.
- **Six finalize-tail shapes** (`ApplyTally::finalize`, `finalize_report`, `applied_or_no_change`, `fanout_size_outcome`, `aggregate_single_op`, the border-family triple).
- **In-file vocabulary split**: `section` has `pub const VERBS` (completion) AND `const KNOWN_VERBS` (validation) — same 9 entries, different order, compiler can't catch drift (`section/mod.rs:50-52` vs `267-269`).
- **`section=`/`range=` extraction + validation triplicated** across color and font (three copies of the extraction loop, three copies of the same error message, two copies of the grapheme pre-flight) — `range_kv.rs` exists as the shared home but hosts only leaf parsers.

The border family proves the target shape: shared `KEYS`/`VERBS` consts + one `stage_kv` + one `kv_hint` + one `kv_value_completions` reused verbatim by `section frame` and `canvas` — but that sharing was hand-built for one family instead of being the framework.

## Fix plan

1. Design a declarative per-verb arg-spec alongside `Command`: subverbs (name, group, hint, handler), kv keys (name, hint, value-parser, value-vocabulary fn), positional slots.
2. One engine generates from it: the kv parse loop with unknown-key rejection, Token/KvValue completion (incl. value vocabularies), usage/help forms, and the hint surface. Bespoke semantics (e.g. `section move` mutual exclusion, `border side` custom-gate) stay as handwritten handlers behind the table.
3. Extend `commands/range_kv.rs` with shared `extract_section_range_kvs(args, verb)` + the grapheme pre-flight; migrate color + font.
4. One `finish_border_edit(label, changed, auto_promoted, bare_custom)` for the four border-family tails.
5. Migrate incrementally: border family first (already closest), then section, canvas, font, color, zoom, rest. Validate against `VERBS` (delete `KNOWN_VERBS`).

## Acceptance criteria

- Adding a kv key to a migrated verb = one table row (parse+complete+help+hint all follow).
- Usage strings and inline errors derived from one source; existing exact-message tests updated deliberately, not silently.
- Completion tests keep passing; `./test.sh` green.

## Pointers

`src/application/console/commands/*`; `src/application/console/traits/`; console findings file for the full site list; CODE_CONVENTIONS §5, §6 ("reach for a strategy pattern when the shape is plural" — 20 verbs is plural).
