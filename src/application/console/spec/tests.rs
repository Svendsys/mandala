// SPDX-License-Identifier: MPL-2.0

//! Registry-wide invariants over every declared [`Grammar`].
//!
//! These are the checks that replace the per-verb assertions #128
//! had to write twice and called "a per-verb copy of a check, i.e.
//! more of the same disease". They walk the whole registry, every
//! nesting level, and hold the declaration against itself in both
//! directions — a form naming a key its level does not declare, and
//! a key no form prints. The second is the one that would have
//! caught `font range=`, `color range=` and `color section=` on the
//! day each was written: each was parseable, named in the verb's own
//! rejection, and absent from the list `help` printed.

use super::{Grammar, Subverb};
use crate::application::console::commands::COMMANDS;

/// Every level reachable from `root`, outermost first. Depth is
/// bounded by the declaration (four, at
/// `canvas section-frame focused preview`), so the recursion cannot
/// run away — a cycle would need a `Grammar` to name itself as its
/// own descendant, which `&'static` initialization order forbids.
fn levels(root: &'static Grammar, out: &mut Vec<&'static Grammar>) {
    out.push(root);
    for subverb in root.subverbs() {
        if let Some(child) = subverb.child {
            levels(child, out);
        }
    }
}

/// Every grammar level in the registry, paired with the verb that
/// roots it so a failure names something a reader can find.
fn all_levels() -> Vec<(&'static str, &'static Grammar)> {
    let mut out = Vec::new();
    for cmd in COMMANDS {
        let mut mine = Vec::new();
        levels(cmd.grammar, &mut mine);
        out.extend(mine.into_iter().map(|g| (cmd.name, g)));
    }
    out
}

/// Every key a form names is a key its level declares.
///
/// The failing input is a `Form` whose `required` or `optional`
/// carries a name absent from the level's `key_sets` — the shape a
/// rename produces, and the one that makes `help` print
/// `bogus=<value>` for a key the parse loop then rejects.
#[test]
fn test_every_form_key_is_declared_by_its_level() {
    for (verb, grammar) in all_levels() {
        let forms = grammar
            .subverbs()
            .flat_map(|s| s.forms.iter())
            .chain(grammar.bare.iter().flat_map(|b| b.forms.iter()));
        for form in forms {
            for name in form.names() {
                assert!(
                    grammar.key(name).is_some(),
                    "{verb}: level `{}` has a form naming key '{name}', which the level does not declare",
                    grammar.label
                );
            }
        }
    }
}

/// Every key a level declares is printed by at least one of its
/// forms.
///
/// This is the direction that catches the defect this epic exists
/// for. A key reachable by the parse loop and named by no form is a
/// key `help <verb>` never mentions and the popup offers only by
/// accident — `font range=` shipped that way, and so did
/// `color range=` after it.
#[test]
fn test_every_declared_key_is_printed_by_some_form() {
    for (verb, grammar) in all_levels() {
        for key in grammar.keys() {
            let printed = grammar
                .subverbs()
                .flat_map(|s| s.forms.iter())
                .chain(grammar.bare.iter().flat_map(|b| b.forms.iter()))
                .any(|form| form.names().any(|n| n == key.name));
            assert!(
                printed,
                "{verb}: level `{}` declares key '{}' that no form prints — \
                 it would be parseable and undocumented",
                grammar.label, key.name
            );
        }
    }
}

/// A subverb either descends into a child level or declares shapes
/// of its own, never both.
///
/// [`super::descent`] relies on it: a subverb with a child is a step
/// deeper and a subverb with forms is the end of the walk, so one
/// carrying both would have its slots and keys silently unreachable.
#[test]
fn test_no_subverb_both_descends_and_declares_forms() {
    for (verb, grammar) in all_levels() {
        for subverb in grammar.subverbs() {
            assert!(
                subverb.child.is_none() || subverb.forms.is_empty(),
                "{verb}: `{} {}` declares both a child level and shapes of its own",
                grammar.label,
                subverb.name
            );
        }
    }
}

/// A child level's label extends its parent's, so every message a
/// nested level prints is copy-pasteable at the depth it printed
/// from.
#[test]
fn test_every_child_label_extends_its_parent() {
    for (verb, grammar) in all_levels() {
        for subverb in grammar.subverbs() {
            let Some(child) = subverb.child else { continue };
            let want = format!("{} {}", grammar.label, subverb.name);
            assert_eq!(
                child.label, want,
                "{verb}: child level of `{}` labels itself '{}' rather than '{want}'",
                grammar.label, child.label
            );
        }
    }
}

/// Names are unique within a level, in both vocabularies.
///
/// A duplicate subverb name makes the second declaration
/// unreachable (`Grammar::subverb` takes the first match) while
/// still printing a usage line and a popup row for it; a duplicate
/// key name does the same to the value vocabulary.
#[test]
fn test_names_are_unique_within_a_level() {
    for (verb, grammar) in all_levels() {
        let mut seen: Vec<&'static str> = Vec::new();
        for subverb in grammar.subverbs() {
            assert!(
                !seen.contains(&subverb.name),
                "{verb}: level `{}` declares subverb '{}' twice",
                grammar.label,
                subverb.name
            );
            seen.push(subverb.name);
        }
        let mut seen: Vec<&'static str> = Vec::new();
        for key in grammar.keys() {
            assert!(
                !seen.contains(&key.name),
                "{verb}: level `{}` declares key '{}' twice",
                grammar.label,
                key.name
            );
            seen.push(key.name);
        }
    }
}

/// Every usage line a level publishes leads with that level's own
/// label, so a form lifted out of `help` and pasted back into the
/// console reaches the verb it documents.
#[test]
fn test_every_usage_form_leads_with_its_verb() {
    for cmd in COMMANDS {
        for form in super::usage::forms(cmd.grammar) {
            assert!(
                form.starts_with(cmd.name),
                "{}: usage form '{form}' does not lead with the verb name",
                cmd.name
            );
        }
    }
}

/// Every key a level's *bare* form names is offered by the popup at
/// that level's first slot, and published by `help <verb>`.
///
/// This is the acceptance criterion of the whole engine stated as a
/// test: adding a kv key is one table row, and parse, complete, help
/// and hint follow. The `help` half is a mirror — `key_lines`
/// derives from the same declaration — so what it can catch is
/// narrow. The *popup* half is not: it runs the real completion
/// engine over the real line, through the descent, the
/// positional-vs-kv gate and the readable-key resolution, and a
/// failure anywhere along that path shows up here.
#[test]
fn test_every_bare_form_key_reaches_the_popup_and_help() {
    let doc = crate::application::document::tests_common::load_test_doc();
    let ctx = crate::application::console::ConsoleContext::from_document(&doc);
    for cmd in COMMANDS {
        let Some(bare) = &cmd.grammar.bare else { continue };
        let line = format!("{} ", cmd.name);
        let offered: Vec<String> = crate::application::console::completion::complete(&line, line.len(), &ctx)
            .into_iter()
            .map(|c| c.text)
            .collect();
        let published = cmd.key_lines();
        for name in bare.readable_keys() {
            assert!(
                offered.iter().any(|t| t == &format!("{}=", name)),
                "{}: `{}<TAB>` must offer '{}='; got {offered:?}",
                cmd.name,
                line,
                name
            );
            assert!(
                published.iter().any(|l| l.starts_with(&format!("{}=", name))),
                "{}: `help {}` must publish '{}='; got {published:?}",
                cmd.name,
                cmd.name,
                name
            );
        }
    }
}

/// A subverb that reads no keys refuses one by name rather than
/// dropping it.
///
/// The failing input is any subverb whose form list grows a key it
/// does not mean to read, or a `kvs::read` that stops asking per
/// *form*. Before the engine, four border surfaces staged
/// `border preset heavy color=#fff`'s preset and discarded the
/// color without a word.
#[test]
fn test_a_subverb_that_reads_no_keys_refuses_one_by_name() {
    use crate::application::console::parser::Args;
    use crate::application::console::spec::descent::descend_at;
    for (verb, grammar) in all_levels() {
        let Some(key) = grammar.keys().next() else { continue };
        for subverb in grammar.subverbs() {
            if subverb.child.is_some() || !subverb.readable_keys().is_empty() {
                continue;
            }
            // Enter this level directly so the probe is one
            // subverb plus one kv, whatever depth the level sits
            // at on a real line.
            let tokens = vec![subverb.name.to_string(), format!("{}=x", key.name)];
            let args = Args::new(&tokens);
            let descent = descend_at(grammar, &tokens, 0);
            let err = super::kvs::read(&descent, &args).err().unwrap_or_else(|| {
                panic!(
                    "{verb}: `{} {} {}=x` must be refused",
                    grammar.label, subverb.name, key.name
                )
            });
            assert!(
                err.contains(key.name),
                "{verb}: the refusal must name the key: {err}"
            );
            assert!(
                err.contains(subverb.name),
                "{verb}: the refusal must name the form: {err}"
            );
        }
    }
}

/// The gate is only ever declared on subverbs that have something
/// to be confused with — a level with no kv keys at all can never
/// put its subverb slot in kv form, so a gate there would be inert
/// and misleading.
#[test]
fn test_a_gated_subverb_sits_at_a_level_that_has_keys() {
    for (verb, grammar) in all_levels() {
        let has_keys = grammar.keys().next().is_some();
        for subverb in grammar.subverbs().filter(|s: &&Subverb| s.gated) {
            assert!(
                has_keys,
                "{verb}: `{} {}` is gated on the positional-vs-kv discriminator \
                 at a level that declares no keys, so the gate can never fire",
                grammar.label, subverb.name
            );
        }
    }
}

/// The stray-positional rejection opens a second sentence only when
/// it has one to open, and that sentence stands on its own words.
///
/// Four levels, four shapes: keys and `preview`, keys alone,
/// `preview` alone, neither. Fails when: the two suggestion clauses
/// are appended one after the other rather than composed — the
/// no-suggestion row then ends `'x'..` (which is what `open` really
/// printed) and the preview-only row opens on `or`.
///
/// Three of the four arms are reachable from the real registry, and
/// `EXEC_CORPUS` pins one line through each: `border padding 12 50`
/// and `canvas border padding 12 50` through `(composed, staged)`,
/// `anchor sideways` through `(composed, bare)`, and
/// `open a.mindmap.json b.mindmap.json` through `(neither)`. The
/// levels differ, so the bytes do — what those rows hold is the arm,
/// which is why a reword that satisfied this test by editing one
/// `match` arm would still move a pinned row.
///
/// The fourth arm — `preview` beside a bare form that reads no keys
/// — is declared by no level today, so the synthetic `PREVIEW_ONLY`
/// grammar below is the only thing that pins its wording. That is
/// the arm this test exists for; the other three it merely seconds.
#[test]
fn test_the_extra_positional_sentence_is_built_from_what_the_level_has() {
    use super::usage::extra_positional_message;
    use super::{Bare, Form, Key, Vocabulary};

    static KEYS: &[Key] = &[Key::new("pad", "padding", Vocabulary::Free { placeholder: "n" })];
    static COMPOSED: &[Form] = &[Form::opt(&["pad"])];
    static SLOT_ONLY: &[Form] = &[Form::slots(&[])];
    static PREVIEW: &[Subverb] = &[Subverb::bare("preview", "staged", "stage the edit")];

    static BOTH: Grammar = Grammar {
        label: "both",
        subverb_sets: &[PREVIEW],
        key_sets: &[KEYS],
        bare: Some(Bare::new("composed", COMPOSED)),
    };
    static KEYS_ONLY: Grammar = Grammar {
        label: "keys-only",
        subverb_sets: &[],
        key_sets: &[KEYS],
        bare: Some(Bare::new("composed", COMPOSED)),
    };
    static PREVIEW_ONLY: Grammar = Grammar {
        label: "preview-only",
        subverb_sets: &[PREVIEW],
        key_sets: &[],
        bare: Some(Bare::new("slots", SLOT_ONLY)),
    };
    static NEITHER: Grammar = Grammar {
        label: "neither",
        subverb_sets: &[],
        key_sets: &[],
        bare: Some(Bare::new("slots", SLOT_ONLY)),
    };

    assert_eq!(
        extra_positional_message(&BOTH, "both pad", "50"),
        "both pad: unexpected extra positional '50'. Compose multiple edits via \
         the kv form (`both <key>=<value> …`) or stage with `both preview …`."
    );
    assert_eq!(
        extra_positional_message(&KEYS_ONLY, "keys-only", "50"),
        "keys-only: unexpected extra positional '50'. Compose multiple edits via \
         the kv form (`keys-only <key>=<value> …`)."
    );
    assert_eq!(
        extra_positional_message(&PREVIEW_ONLY, "preview-only", "50"),
        "preview-only: unexpected extra positional '50'. Stage with `preview-only preview …`."
    );
    assert_eq!(
        extra_positional_message(&NEITHER, "neither", "50"),
        "neither: unexpected extra positional '50'."
    );

    // Hold the doc comment's division of labor against the real
    // registry rather than trusting it. Three arms are seconded by a
    // pinned corpus row *because* a level reaches them; the fourth
    // is pinned here alone *because* none does. A level that grows a
    // `preview` subverb beside a bare form reading no keys flips
    // that, and this is what says so — at which point the arm wants
    // a corpus row and this comment wants a rewrite.
    let mut reached = [false; 4];
    for (_, grammar) in all_levels() {
        let composed = grammar
            .bare
            .as_ref()
            .is_some_and(|b| !b.readable_keys().is_empty());
        let staged = grammar.subverb("preview").is_some();
        reached[usize::from(composed) * 2 + usize::from(staged)] = true;
    }
    assert!(
        reached[3] && reached[2] && reached[0],
        "the three arms `EXEC_CORPUS` seconds must each be reachable: {reached:?}"
    );
    assert!(
        !reached[1],
        "a level now declares `preview` beside a bare form that reads no keys, so the \
         `(false, true)` arm is reachable and owes a pinned corpus row of its own"
    );
}
