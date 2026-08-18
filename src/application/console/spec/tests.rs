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
        if let Some(grammar) = cmd.grammar {
            let mut mine = Vec::new();
            levels(grammar, &mut mine);
            out.extend(mine.into_iter().map(|g| (cmd.name, g)));
        }
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
        let Some(grammar) = cmd.grammar else { continue };
        for form in super::usage::forms(grammar) {
            assert!(
                form.starts_with(cmd.name),
                "{}: usage form '{form}' does not lead with the verb name",
                cmd.name
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
