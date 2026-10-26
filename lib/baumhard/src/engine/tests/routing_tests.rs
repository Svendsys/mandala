// SPDX-License-Identifier: MPL-2.0

//! Routing keys over [`TestElement`]: a mutator reaches a target only
//! when their [`BranchKey`]s are equal, kind included, so one parent
//! holds children of two kinds whose channel spaces are independent.
//! Plus the key's JSON spelling — two required fields, `"kind"` and
//! `"channel"` — and the builder's keys: every AST node names its kind,
//! and a `Repeat` emits its own kind on consecutive channels.
//!
//! Every expected value is a literal worked out from the fixture.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2): every `do_*` body
//! has a bench entry in `benches/test_bench.rs`.

use crate::engine::builder::{build, BuildError, ChannelSrc, CountSrc, MutationSrc, MutatorNode, NoRuntime};
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use crate::engine::tests::builder_tests::IndexContext;
use crate::engine::tests::test_element::{
    add, add_mutator, marked, plain, row, value, TestElement, TestKind, TestMutation, TestMutator,
};
use crate::engine::tree::{MutatorTree, Tree};
use crate::engine::walker::walk_tree;
use indextree::NodeId;
use serde_json::json;

/// A root (Plain 0, value 0) with three children, added in this order:
/// `(Plain, 0) = 10`, `(Marked, 0) = 20`, `(Plain, 1) = 30`. Returns
/// their ids in the same order.
fn mixed() -> (Tree<TestElement>, [NodeId; 3]) {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let a0 = add(&mut tree, root, TestElement::new(0, 10));
    let b0 = add(&mut tree, root, TestElement::marked(0, 20));
    let a1 = add(&mut tree, root, TestElement::new(1, 30));
    (tree, [a0, b0, a1])
}

/// A mutator tree rooted at a `(Plain, 0)` `Void` with `children`.
fn under_void(children: Vec<TestMutator>) -> MutatorTree<TestElement, TestMutation> {
    let mut mt = MutatorTree::new();
    let root = mt.root;
    for child in children {
        add_mutator(&mut mt, root, child);
    }
    mt
}

// ── mixed-kind routing ────────────────────────────────────────────

#[test]
fn test_engine_walk_routes_by_kind_and_channel() {
    do_engine_walk_routes_by_kind_and_channel();
}

/// Under one parent, children `(Plain, 0)`, `(Marked, 0)` and
/// `(Plain, 1)`: a `(Marked, 0)` mutator touches only the `Marked`
/// child, though a `Plain` child shares its channel; the control, a
/// `(Plain, 0)` mutator, touches only the `Plain` child on channel 0.
pub fn do_engine_walk_routes_by_kind_and_channel() {
    let (mut tree, [a0, b0, a1]) = mixed();
    walk_tree(
        &mut tree,
        &under_void(vec![Mutator::new(TestMutation::Add(5), marked(0))]),
        &0,
    );
    assert_eq!(value(&tree, b0), 25, "the Marked child on channel 0");
    assert_eq!(
        value(&tree, a0),
        10,
        "a Plain child on the same channel is another key"
    );
    assert_eq!(value(&tree, a1), 30);

    let (mut tree, [a0, b0, a1]) = mixed();
    walk_tree(
        &mut tree,
        &under_void(vec![Mutator::new(TestMutation::Add(5), plain(0))]),
        &0,
    );
    assert_eq!(value(&tree, a0), 15, "the control: the Plain child on channel 0");
    assert_eq!(value(&tree, b0), 20);
    assert_eq!(value(&tree, a1), 30);
}

#[test]
fn test_engine_walk_merges_rows_of_two_kinds() {
    do_engine_walk_merges_rows_of_two_kinds();
}

/// A mutator row that mixes kinds, added out of key order, merged
/// against a target row that mixes them too: each mutator reaches the
/// targets with its key and no other, and a mutator whose key no
/// target has reaches nothing. Keys order kind first, so every `Plain`
/// key comes before every `Marked` one in both rows.
pub fn do_engine_walk_merges_rows_of_two_kinds() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let m1 = add(&mut tree, root, TestElement::marked(1, 1));
    let p0 = add(&mut tree, root, TestElement::new(0, 2));
    let m0 = add(&mut tree, root, TestElement::marked(0, 3));
    let p1 = add(&mut tree, root, TestElement::new(1, 4));
    assert_eq!(
        row(&tree, root),
        vec![2, 4, 3, 1],
        "precondition: the tree keeps the row in key order — Plain 0, Plain 1, Marked 0, Marked 1"
    );

    let mt = under_void(vec![
        Mutator::new(TestMutation::Add(1000), marked(3)),
        Mutator::new(TestMutation::Add(100), marked(0)),
        Mutator::new(TestMutation::Add(10), plain(1)),
    ]);
    walk_tree(&mut tree, &mt, &0);
    assert_eq!(value(&tree, p1), 14);
    assert_eq!(value(&tree, m0), 103);
    assert_eq!(value(&tree, p0), 2, "no (Plain, 0) mutator");
    assert_eq!(
        value(&tree, m1),
        1,
        "no (Marked, 1) mutator; (Marked, 3) has no target"
    );
}

// ── the key's JSON ────────────────────────────────────────────────

#[test]
fn test_branch_key_json_has_two_required_fields() {
    do_branch_key_json_has_two_required_fields();
}

/// A key is written as `"kind"` and `"channel"`; a single-kind element
/// type's `()` kind is written `null`. Both fields are required — a
/// key missing either is refused, naming the missing field — and a key
/// with both reads back as the key it was.
pub fn do_branch_key_json_has_two_required_fields() {
    assert_eq!(
        serde_json::to_value(marked(3)).unwrap(),
        json!({"kind": "Marked", "channel": 3})
    );
    let unit: BranchKey<()> = 7.into();
    assert_eq!(
        serde_json::to_value(unit).unwrap(),
        json!({"kind": null, "channel": 7})
    );

    let no_kind = serde_json::from_value::<BranchKey<()>>(json!({"channel": 7})).unwrap_err();
    assert!(no_kind.to_string().contains("missing field `kind`"), "{no_kind}");
    let no_channel = serde_json::from_value::<BranchKey<()>>(json!({"kind": null})).unwrap_err();
    assert!(
        no_channel.to_string().contains("missing field `channel`"),
        "{no_channel}"
    );
    assert_eq!(
        serde_json::from_value::<BranchKey<()>>(json!({"kind": null, "channel": 7})).unwrap(),
        unit,
        "the control: both fields read back"
    );
}

#[test]
fn test_mutator_and_ast_json_carry_the_kind() {
    do_mutator_and_ast_json_carry_the_kind();
}

/// A [`Mutator`] writes its key as a `"key"` object; a
/// [`MutatorNode`] writes `"kind"` beside `"channel"`, and refuses JSON
/// without it. The AST's kind reaches the built mutator's key.
pub fn do_mutator_and_ast_json_carry_the_kind() {
    let single: TestMutator = Mutator::new(TestMutation::Add(1), marked(2));
    assert_eq!(
        serde_json::to_value(&single).unwrap(),
        json!({"Single": {"key": {"kind": "Marked", "channel": 2}, "mutation": {"Add": 1}}})
    );

    type Node = MutatorNode<TestElement, TestMutation>;
    let missing = serde_json::from_value::<Node>(json!({"Void": {"channel": 2}})).unwrap_err();
    assert!(missing.to_string().contains("missing field `kind`"), "{missing}");
    let node: Node = serde_json::from_value(json!({"Void": {"kind": "Marked", "channel": 2}})).unwrap();
    let mt = build(&node, &NoRuntime).unwrap();
    assert_eq!(mt.arena()[mt.root].get().key(), marked(2));
}

// ── the builder's keys ────────────────────────────────────────────

#[test]
fn test_engine_build_repeat_emits_its_kind_on_consecutive_channels() {
    do_engine_build_repeat_emits_its_kind_on_consecutive_channels();
}

/// A `Marked` `Repeat` over channels 1 and 2 builds two mutators keyed
/// `(Marked, 1)` and `(Marked, 2)`: they reach the two `Marked` targets
/// on those channels and not the `Plain` target on channel 1.
pub fn do_engine_build_repeat_emits_its_kind_on_consecutive_channels() {
    let node: MutatorNode<TestElement, TestMutation> = MutatorNode::Void {
        kind: TestKind::Plain,
        channel: 0,
        children: vec![MutatorNode::Repeat {
            kind: TestKind::Marked,
            section: "cells".into(),
            channel_base: 1,
            count: CountSrc::Literal(2),
            skip_indices: vec![],
            template: Box::new(MutatorNode::Single {
                kind: TestKind::Marked,
                channel: ChannelSrc::SectionIndex,
                mutation: MutationSrc::Template(TestMutation::Add(10)),
            }),
        }],
    };
    let mt = build(&node, &IndexContext).unwrap();
    let keys: Vec<BranchKey<TestKind>> = mt
        .root
        .children(mt.arena())
        .map(|id| mt.arena()[id].get().key())
        .collect();
    assert_eq!(keys, vec![marked(1), marked(2)]);

    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let p1 = add(&mut tree, root, TestElement::new(1, 100));
    let m1 = add(&mut tree, root, TestElement::marked(1, 200));
    let m2 = add(&mut tree, root, TestElement::marked(2, 300));
    mt.apply_to(&mut tree, &0);
    assert_eq!(value(&tree, m1), 210, "iteration 0: Add(10 + 0)");
    assert_eq!(value(&tree, m2), 311, "iteration 1: Add(10 + 1)");
    assert_eq!(
        value(&tree, p1),
        100,
        "a Plain target on channel 1 is another key"
    );
}

#[test]
fn test_engine_build_refuses_a_template_of_another_kind() {
    do_engine_build_refuses_a_template_of_another_kind();
}

/// Every key a `Repeat` emits carries its kind, so a template naming
/// another kind is [`BuildError::TemplateKindDiffers`]; the same AST
/// with the kinds agreeing builds.
pub fn do_engine_build_refuses_a_template_of_another_kind() {
    let with = |template_kind: TestKind| -> MutatorNode<TestElement, TestMutation> {
        MutatorNode::Void {
            kind: TestKind::Plain,
            channel: 0,
            children: vec![MutatorNode::Repeat {
                kind: TestKind::Marked,
                section: "cells".into(),
                channel_base: 1,
                count: CountSrc::Literal(2),
                skip_indices: vec![],
                template: Box::new(MutatorNode::Single {
                    kind: template_kind,
                    channel: ChannelSrc::SectionIndex,
                    mutation: MutationSrc::Template(TestMutation::Add(10)),
                }),
            }],
        }
    };
    assert_eq!(
        build(&with(TestKind::Plain), &IndexContext).err(),
        Some(BuildError::TemplateKindDiffers)
    );
    assert!(
        build(&with(TestKind::Marked), &IndexContext).is_ok(),
        "the control: the same kind builds"
    );
}

#[test]
fn test_engine_build_inserts_children_in_key_order() {
    do_engine_build_inserts_children_in_key_order();
}

/// Children declared out of key order — `(Marked, 0)`, `(Plain, 5)`,
/// `(Plain, 1)`, `(Plain, 5)` — build into a sorted mutator tree, the
/// two `(Plain, 5)`s in declaration order.
pub fn do_engine_build_inserts_children_in_key_order() {
    let single = |kind: TestKind, channel: usize, n: i64| MutatorNode::Single {
        kind,
        channel: ChannelSrc::Literal(channel),
        mutation: MutationSrc::Runtime(format!("{n}")),
    };
    struct Numbered;
    impl crate::engine::builder::BuildContext<TestElement, TestMutation> for Numbered {
        fn mutation(&self, label: &str) -> TestMutation {
            TestMutation::Set(label.parse().unwrap())
        }
        fn resolve(&self, _: &str, _: usize, _: &TestMutation) -> TestMutation {
            unreachable!("no templates")
        }
    }
    let node: MutatorNode<TestElement, TestMutation> = MutatorNode::Void {
        kind: TestKind::Plain,
        channel: 0,
        children: vec![
            single(TestKind::Marked, 0, 1),
            single(TestKind::Plain, 5, 2),
            single(TestKind::Plain, 1, 3),
            single(TestKind::Plain, 5, 4),
        ],
    };
    let mt = build(&node, &Numbered).unwrap();
    assert_eq!(mt.check_sorted(), Ok(()));
    let order: Vec<(BranchKey<TestKind>, TestMutation)> = mt
        .root
        .children(mt.arena())
        .map(|id| match mt.arena()[id].get() {
            Mutator::Single { key, mutation } => (*key, mutation.clone()),
            other => panic!("expected Single, got {other:?}"),
        })
        .collect();
    assert_eq!(
        order,
        vec![
            (plain(1), TestMutation::Set(3)),
            (plain(5), TestMutation::Set(2)),
            (plain(5), TestMutation::Set(4)),
            (marked(0), TestMutation::Set(1)),
        ]
    );
}
