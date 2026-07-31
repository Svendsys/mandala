// SPDX-License-Identifier: MPL-2.0

//! Tests for the `CustomMutation` carrier — serde roundtrip,
//! backward-compat with pre-unification JSON, and the
//! `contexts` / `is_internal` / `targets_map` helpers.

use super::*;
use crate::gfx_structs::area::GlyphAreaCommand;
use crate::gfx_structs::mutator::Mutation;

fn sample(id: &str) -> CustomMutation {
    CustomMutation {
        id: id.to_string(),
        name: id.to_string(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![Mutation::area_command(
            GlyphAreaCommand::NudgeRight(10.0),
        )])),
        target_scope: TargetScope::SelfOnly,
        behavior: MutationBehavior::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: None,
    }
}

#[test]
fn roundtrip_preserves_all_fields() {
    let mut cm = sample("nudge");
    cm.description = "Nudge every selected node one pixel right.".into();
    cm.contexts = vec![contexts::MAP_NODE.into()];
    let json = serde_json::to_string(&cm).unwrap();
    let back: CustomMutation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, "nudge");
    assert_eq!(back.description, cm.description);
    assert_eq!(back.contexts, cm.contexts);
    assert_eq!(back.target_scope, TargetScope::SelfOnly);
}

#[test]
fn empty_description_and_contexts_omitted_from_json() {
    let cm = sample("bare");
    let json = serde_json::to_string(&cm).unwrap();
    assert!(!json.contains("\"description\""));
    assert!(!json.contains("\"contexts\""));
}

#[test]
fn legacy_json_with_mutations_and_scope_loads_into_new_shape() {
    // Pre-unification shape: `mutations` + `target_scope`, no
    // `mutator` field. The backward-compat deserializer
    // synthesizes the MutatorNode via `scope` helpers so old maps
    // load unchanged.
    let legacy = r#"{
        "id": "old",
        "name": "Old",
        "mutations": [{"AreaCommand": {"NudgeRight": 5.0}}],
        "target_scope": "Descendants"
    }"#;
    let cm: CustomMutation = serde_json::from_str(legacy).unwrap();
    assert_eq!(cm.id, "old");
    assert_eq!(cm.target_scope, TargetScope::Descendants);
    // Synthesized MutatorNode: Instruction(RepeatWhile) wrapping Macro.
    match &cm.mutator {
        Some(crate::mutator_builder::MutatorNode::Instruction { .. }) => {}
        other => panic!("expected Instruction mutator, got {:?}", other),
    }
}

#[test]
fn legacy_json_with_only_document_actions_loads_with_no_mutator() {
    // `theme_demo.mindmap.json` shape: document_actions only.
    // `mutations` absent (serde-default) + `target_scope` present.
    // The synthesized mutator is None — no tree payload.
    let legacy = r#"{
        "id": "switch-dark",
        "name": "Switch to dark theme",
        "target_scope": "SelfOnly",
        "document_actions": [{"SetThemeVariant": "dark"}]
    }"#;
    let cm: CustomMutation = serde_json::from_str(legacy).unwrap();
    assert_eq!(cm.id, "switch-dark");
    assert!(cm.mutator.is_none());
    assert_eq!(cm.document_actions.len(), 1);
}

/// Full round-trip on real-world legacy shape: load
/// `theme_demo.mindmap.json` (three document-actions-only
/// mutations in the pre-unification shape), save it to a temp
/// file through the canonical on-save path, reload, and assert
/// every `custom_mutation` still reflects the source file's
/// semantic content. Guards the silent upgrade-on-save contract
/// the review flagged — a user opening + saving a legacy map
/// should never lose information.
#[test]
fn legacy_theme_demo_map_round_trips_through_canonical_save() {
    use crate::mindmap::loader::{load_from_file, save_to_file};
    use std::path::PathBuf;

    let repo_root: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", ".."].iter().collect();
    let src_path = repo_root.join("maps").join("theme_demo.mindmap.json");
    // Some test runners may not have access to the full repo tree
    // (e.g. crate-scoped cargo test); skip gracefully rather than
    // fail a CI environment that doesn't ship the fixture.
    if !src_path.exists() {
        return;
    }

    let original = load_from_file(&src_path).expect("legacy theme_demo loads");
    assert!(
        !original.custom_mutations.is_empty(),
        "fixture expected to have custom_mutations"
    );

    // Save to a temp file using the canonical on-save path.
    let scratch = crate::util::test_temp::TempDir::new("theme-demo-roundtrip");
    let tmp_path = scratch.join("map.mindmap.json");
    save_to_file(&tmp_path, &original).expect("save_to_file succeeds");

    // Reload and compare field-by-field on every custom_mutation.
    let reloaded = load_from_file(&tmp_path).expect("canonical reloads");
    assert_eq!(reloaded.custom_mutations.len(), original.custom_mutations.len());
    for (before, after) in original
        .custom_mutations
        .iter()
        .zip(reloaded.custom_mutations.iter())
    {
        assert_eq!(before.id, after.id);
        assert_eq!(before.name, after.name);
        assert_eq!(before.target_scope, after.target_scope);
        assert_eq!(before.document_actions, after.document_actions);
        // The synthesized `mutator` is stable across the round-trip:
        // legacy had no `mutations` to translate, so `None` persists.
        assert_eq!(before.mutator.is_none(), after.mutator.is_none());
    }

    let _ = std::fs::remove_file(&tmp_path);
}

#[test]
fn matches_context_hits_exact_and_dotted_descendants() {
    let mut cm = sample("x");
    cm.contexts = vec![contexts::MAP_NODE.into(), contexts::MAP_TREE.into()];
    assert!(cm.matches_context(contexts::MAP));
    assert!(cm.matches_context(contexts::MAP_NODE));
    assert!(cm.matches_context(contexts::MAP_TREE));
    assert!(!cm.matches_context(contexts::INTERNAL));
}

#[test]
fn is_internal_is_true_on_empty_contexts() {
    let cm = sample("empty");
    assert!(cm.is_internal());
}

#[test]
fn is_internal_is_true_when_tag_present() {
    let mut cm = sample("tagged");
    cm.contexts = vec![contexts::INTERNAL.into(), contexts::MAP_NODE.into()];
    assert!(cm.is_internal());
    // targets_map also true because MAP_NODE is under MAP.
    assert!(cm.targets_map());
}

#[test]
fn targets_map_hits_any_map_sub_namespace() {
    let mut cm = sample("m");
    cm.contexts = vec![contexts::MAP_TREE.into()];
    assert!(cm.targets_map());
    assert!(!cm.is_internal());
}

#[test]
fn trigger_binding_roundtrip() {
    let binding = TriggerBinding {
        trigger: Trigger::OnKey("r".to_string()),
        mutation_id: "highlight-red".to_string(),
        contexts: vec![PlatformContext::Desktop, PlatformContext::Web],
    };
    let json = serde_json::to_string(&binding).unwrap();
    let back: TriggerBinding = serde_json::from_str(&json).unwrap();
    assert_eq!(back.trigger, Trigger::OnKey("r".to_string()));
    assert_eq!(back.contexts.len(), 2);
}

#[test]
fn document_action_set_theme_variant_roundtrip() {
    let action = DocumentAction::SetThemeVariant("dark".to_string());
    let json = serde_json::to_string(&action).unwrap();
    let back: DocumentAction = serde_json::from_str(&json).unwrap();
    assert_eq!(back, action);
}

#[test]
fn mutation_behavior_default_is_persistent() {
    // Absent `behavior` key in source JSON deserializes to
    // `Persistent` via the struct-level serde default, so maps
    // written before the field existed still load.
    let src = serde_json::json!({
        "id": "b",
        "name": "b",
        "mutator": { "Macro": { "channel": 0, "mutations": { "Literal": [] } } },
        "target_scope": "SelfOnly"
    })
    .to_string();
    let cm: CustomMutation = serde_json::from_str(&src).unwrap();
    assert_eq!(cm.behavior, MutationBehavior::Persistent);
}

#[test]
fn all_target_scopes_serialize() {
    for scope in [
        TargetScope::SelfOnly,
        TargetScope::Children,
        TargetScope::Descendants,
        TargetScope::SelfAndDescendants,
        TargetScope::Parent,
        TargetScope::Siblings,
    ] {
        let json = serde_json::to_string(&scope).unwrap();
        let back: TargetScope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, scope);
    }
}

#[test]
fn all_triggers_serialize() {
    for trigger in [
        Trigger::OnClick,
        Trigger::OnHover,
        Trigger::OnKey("space".to_string()),
        Trigger::OnLink("action:expand".to_string()),
    ] {
        let json = serde_json::to_string(&trigger).unwrap();
        let back: Trigger = serde_json::from_str(&json).unwrap();
        assert_eq!(back, trigger);
    }
}

#[cfg(test)]
mod reach_tests {
    use super::*;
    use crate::gfx_structs::area::GlyphAreaCommand;
    use crate::gfx_structs::mutator::Mutation;

    fn nudge() -> Mutation {
        Mutation::area_command(GlyphAreaCommand::NudgeRight(1.0))
    }

    #[test]
    fn scope_self_only_covers_only_self_only_reach() {
        assert!(TargetScope::SelfOnly.covers_reach(MutatorReach::SelfOnly));
        assert!(!TargetScope::SelfOnly.covers_reach(MutatorReach::Children));
        assert!(!TargetScope::SelfOnly.covers_reach(MutatorReach::Descendants));
    }

    /// Per-target anchoring: a `Children`-scoped mutation anchors
    /// the mutator at *each child*, so a `MutatorReach::Children`
    /// mutator reaches the anchor's **grandchildren** — nodes the
    /// `Children` snapshot (`collect_affected_node_ids`) never
    /// captured. Only `SelfOnly` reach is covered.
    #[test]
    fn scope_children_covers_only_self_only_reach() {
        assert!(TargetScope::Children.covers_reach(MutatorReach::SelfOnly));
        assert!(!TargetScope::Children.covers_reach(MutatorReach::Children));
        assert!(!TargetScope::Children.covers_reach(MutatorReach::Descendants));
    }

    /// Same argument as [`scope_children_covers_only_self_only_reach`]
    /// one level over: a sibling's children are not siblings.
    #[test]
    fn scope_siblings_covers_only_self_only_reach() {
        assert!(TargetScope::Siblings.covers_reach(MutatorReach::SelfOnly));
        assert!(!TargetScope::Siblings.covers_reach(MutatorReach::Children));
        assert!(!TargetScope::Siblings.covers_reach(MutatorReach::Descendants));
    }

    #[test]
    fn scope_parent_covers_only_self_only_reach() {
        assert!(TargetScope::Parent.covers_reach(MutatorReach::SelfOnly));
        assert!(!TargetScope::Parent.covers_reach(MutatorReach::Children));
        assert!(!TargetScope::Parent.covers_reach(MutatorReach::Descendants));
    }

    #[test]
    fn scope_sections_only_covers_only_self_only_reach() {
        assert!(TargetScope::SectionsOnly.covers_reach(MutatorReach::SelfOnly));
        assert!(!TargetScope::SectionsOnly.covers_reach(MutatorReach::Children));
        assert!(!TargetScope::SectionsOnly.covers_reach(MutatorReach::Descendants));
    }

    #[test]
    fn scope_self_and_descendants_covers_everything() {
        assert!(TargetScope::SelfAndDescendants.covers_reach(MutatorReach::SelfOnly));
        assert!(TargetScope::SelfAndDescendants.covers_reach(MutatorReach::Children));
        assert!(TargetScope::SelfAndDescendants.covers_reach(MutatorReach::Descendants));
    }

    /// Whole-table pin: every `(scope, reach)` cell in one place so a
    /// future edit to one arm can't quietly widen another. The
    /// closure argument is spelled out in
    /// [`TargetScope::covers_reach`]'s doc table; the differential
    /// harness in `src/application/document/custom/mod.rs` derives
    /// the same table from real tree structure rather than by hand.
    #[test]
    fn covers_reach_table_is_exhaustive_and_stable() {
        use MutatorReach::{Children, Descendants, SelfOnly};
        let expected: &[(TargetScope, MutatorReach, bool)] = &[
            (TargetScope::SelfOnly, SelfOnly, true),
            (TargetScope::SelfOnly, Children, false),
            (TargetScope::SelfOnly, Descendants, false),
            (TargetScope::Children, SelfOnly, true),
            (TargetScope::Children, Children, false),
            (TargetScope::Children, Descendants, false),
            (TargetScope::Descendants, SelfOnly, true),
            (TargetScope::Descendants, Children, true),
            (TargetScope::Descendants, Descendants, true),
            (TargetScope::SelfAndDescendants, SelfOnly, true),
            (TargetScope::SelfAndDescendants, Children, true),
            (TargetScope::SelfAndDescendants, Descendants, true),
            (TargetScope::Parent, SelfOnly, true),
            (TargetScope::Parent, Children, false),
            (TargetScope::Parent, Descendants, false),
            (TargetScope::Siblings, SelfOnly, true),
            (TargetScope::Siblings, Children, false),
            (TargetScope::Siblings, Descendants, false),
            (TargetScope::SectionsOnly, SelfOnly, true),
            (TargetScope::SectionsOnly, Children, false),
            (TargetScope::SectionsOnly, Descendants, false),
        ];
        for (scope, reach, want) in expected {
            assert_eq!(
                scope.covers_reach(*reach),
                *want,
                "covers_reach({:?}, {:?}) should be {}",
                scope,
                reach,
                want
            );
        }
    }

    #[test]
    fn scope_descendants_covers_everything() {
        assert!(TargetScope::Descendants.covers_reach(MutatorReach::SelfOnly));
        assert!(TargetScope::Descendants.covers_reach(MutatorReach::Children));
        assert!(TargetScope::Descendants.covers_reach(MutatorReach::Descendants));
    }

    #[test]
    fn reach_of_self_only_scope_helper_is_self_only() {
        let node = scope::self_only(vec![nudge()]);
        assert_eq!(mutator_reach(&node), MutatorReach::SelfOnly);
    }

    #[test]
    fn reach_of_descendants_scope_helper_is_descendants() {
        let node = scope::descendants(vec![nudge()]);
        assert_eq!(mutator_reach(&node), MutatorReach::Descendants);
    }

    #[test]
    fn reach_of_self_and_descendants_scope_helper_is_descendants() {
        let node = scope::self_and_descendants(vec![nudge()]);
        assert_eq!(mutator_reach(&node), MutatorReach::Descendants);
    }

    #[test]
    fn reach_of_mapchildren_is_children() {
        use crate::mutator_builder::{InstructionSpec, MutationSrc, MutatorNode};
        let node = MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::MapChildren,
            mutation: MutationSrc::None,
            children: vec![],
        };
        assert_eq!(mutator_reach(&node), MutatorReach::Children);
    }
}

/// [`flat_mutations`] guard coverage.
///
/// The guard decides which `Instruction(RepeatWhile(..))` wrappers
/// the flat-apply path is allowed to unwrap into a literal mutation
/// list. Unwrapping one whose predicate is *not* always-true makes
/// the applied result differ from the authored AST — the flat path
/// has no predicate evaluator, so the Macro's mutations land on
/// every target the scope collected instead of the filtered subset.
/// `always_match` is the only admissible test because it is the only
/// thing [`Predicate::test`] short-circuits on.
#[cfg(test)]
mod flat_mutations_tests {
    use super::*;
    use crate::gfx_structs::area::GlyphAreaCommand;
    use crate::gfx_structs::element::GfxElementField;
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::predicate::{Comparator, Predicate};
    use crate::mutator_builder::{InstructionSpec, MutationListSrc, MutationSrc, MutatorNode};

    fn nudge() -> Mutation {
        Mutation::area_command(GlyphAreaCommand::NudgeRight(1.0))
    }

    fn macro_child() -> MutatorNode {
        MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge()]),
            children: vec![],
        }
    }

    fn repeat_while(predicate: Predicate) -> MutatorNode {
        MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::RepeatWhile(predicate),
            mutation: MutationSrc::None,
            children: vec![macro_child()],
        }
    }

    /// A bare `Predicate::new()` (`fields: []`, `always_match: false`)
    /// matches **nothing** — the footgun documented on
    /// [`CustomMutation::predicate`]. A `RepeatWhile` wrapping it
    /// must therefore fall through to `None` so the apply site warns
    /// and skips, rather than blanket-applying the Macro's payload
    /// to the whole scope set.
    ///
    /// Red on `eddd53a`: the guard read
    /// `always_match || all(Equals) && fields.is_empty()`, which
    /// parses as `a || (b && c)`. `b` is vacuously true exactly when
    /// `c` is, so the guard collapsed to
    /// `always_match || fields.is_empty()` and admitted this shape.
    #[test]
    fn match_nothing_repeat_while_is_not_flat_extractable() {
        let node = repeat_while(Predicate::new());
        assert!(
            flat_mutations(&node).is_none(),
            "a match-nothing RepeatWhile must not flat-apply"
        );
    }

    /// The always-true wrapper `scope::descendants` builds is the
    /// shape the guard exists to admit.
    #[test]
    fn always_true_repeat_while_is_flat_extractable() {
        let node = repeat_while(Predicate::always_true());
        assert_eq!(flat_mutations(&node).map(|m| m.len()), Some(1));
    }

    /// `always_match` short-circuits [`Predicate::test`] before the
    /// field list is consulted, so a predicate carrying fields *and*
    /// the flag is still semantically always-true and still
    /// extractable. Pins that the guard keys on the flag, not on
    /// `fields.is_empty()`.
    #[test]
    fn always_match_with_fields_is_still_flat_extractable() {
        let predicate = Predicate {
            fields: vec![(GfxElementField::Channel(3), Comparator::equals())],
            always_match: true,
        };
        assert_eq!(flat_mutations(&repeat_while(predicate)).map(|m| m.len()), Some(1));
    }

    /// A genuinely filtering predicate needs walker-based
    /// evaluation the flat path can't provide.
    #[test]
    fn field_filtered_repeat_while_is_not_flat_extractable() {
        let predicate = Predicate {
            fields: vec![(GfxElementField::Channel(3), Comparator::equals())],
            always_match: false,
        };
        assert!(flat_mutations(&repeat_while(predicate)).is_none());
    }

    /// The `RepeatWhileAlwaysTrue` sugar variant takes its own arm
    /// and stays extractable.
    #[test]
    fn repeat_while_always_true_sugar_is_flat_extractable() {
        let node = MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::RepeatWhileAlwaysTrue,
            mutation: MutationSrc::None,
            children: vec![macro_child()],
        };
        assert_eq!(flat_mutations(&node).map(|m| m.len()), Some(1));
    }

    /// The scope helpers stay extractable — they are the shapes the
    /// flat-apply path is built around.
    #[test]
    fn scope_helpers_stay_flat_extractable() {
        assert_eq!(
            flat_mutations(&scope::self_only(vec![nudge()])).map(|m| m.len()),
            Some(1)
        );
        assert_eq!(
            flat_mutations(&scope::descendants(vec![nudge()])).map(|m| m.len()),
            Some(1)
        );
        assert_eq!(
            flat_mutations(&scope::self_and_descendants(vec![nudge()])).map(|m| m.len()),
            Some(1)
        );
    }

    /// `MapChildren` has no flat equivalent — the flat path iterates
    /// scope-collected model nodes, not the zip-by-position pairing
    /// the instruction means.
    #[test]
    fn map_children_is_not_flat_extractable() {
        let node = MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::MapChildren,
            mutation: MutationSrc::None,
            children: vec![macro_child()],
        };
        assert!(flat_mutations(&node).is_none());
    }
}
