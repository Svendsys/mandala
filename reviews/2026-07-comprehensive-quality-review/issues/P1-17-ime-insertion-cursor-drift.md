# P1-17: Character insertion implemented three times — console and single-line editors corrupt the grapheme cursor on IME/dead-key multi-codepoint payloads

**Severity:** P1 (live text-corruption bug on two surfaces + tripled primitive) · **Area:** mandala editors + baumhard/grapheme_chad

## Problem

The node text editor fixed this bug and documents the exact failure (`src/application/app/text_edit/editor.rs:544-598`): an IME delivering `"한"` (three jamo codepoints, one cluster) or a dead-key `"e\u{0301}"` "would otherwise call `insert_at_cursor` once per char and increment `cursor` by `+1` per char — but `count_grapheme_clusters` of the resulting buffer collapses the codepoints into one cluster, leaving `cursor_grapheme_pos` past the buffer's grapheme count". Its fix: measure the pre/post cluster-count delta and advance by that.

The two other insertion sites still do the warned-against per-char loop:

- Console: `insert_text` (`src/application/app/console_input/edit.rs:261-288`) — `for ch in text.chars() { insert_str_at_grapheme(...); *cursor += 1; }`. `console/tests/grapheme.rs:25-40` locks the pattern in for ASCII only.
- Edge-label / portal-text editor: `route_label_edit_key` (`src/application/app/label_edit.rs:31-55`) — same per-char loop; its comment even says "payloads can carry IME / dead-key multi-char sequences, so iterate".

Result: on those two surfaces a composed-character commit leaves the cursor past the real cluster count — subsequent edits land in the wrong place or get clamped.

## Fix plan

1. Add the primitive to baumhard (§1 "missing primitives are added to Baumhard"): `grapheme_chad::insert_str_at_grapheme_counted(buffer: &mut String, cursor_clusters: usize, s: &str) -> usize` returning the **cluster delta** (implemented via the editor.rs pre/post-count technique, or smarter local counting). Ship `do_*` test + bench in the same commit (§B3): cases for jamo composition, combining mark, ZWJ family emoji, plain ASCII.
2. Rewrite all three call sites on top of it (the node editor's local implementation becomes a thin call).
3. Extend `console/tests/grapheme.rs` with the combining-mark case (currently ASCII-only lock-in).

## Acceptance criteria

- Inserting `"e" + "\u{0301}"` (two events) or a 3-jamo IME commit into console/label/portal-text leaves `cursor == count_grapheme_clusters(buffer_up_to_cursor)` — tested on all three surfaces.
- One insertion implementation repo-wide (grep: no per-char `*cursor += 1` insertion loops remain).
- `./test.sh` green.

## Pointers

`src/application/app/text_edit/editor.rs:544-598` (the correct reference implementation); `src/application/app/console_input/edit.rs:261-288`; `src/application/app/label_edit.rs:31-55`; `src/application/app/text_edit/mod.rs:140-143` (`insert_at_cursor` primitive); `lib/baumhard/src/util/grapheme_chad.rs`; CONVENTIONS §B3; CODE_CONVENTIONS §1, §2 ("unify the shapes").
