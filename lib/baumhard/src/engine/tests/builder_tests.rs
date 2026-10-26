// SPDX-License-Identifier: MPL-2.0

//! The builder over [`TestElement`]: an AST with templates, runtime
//! payloads and a `Repeat` builds into a mutator tree the walker
//! applies — the builder is as generic as the walker — and the
//! [`BuildError`]s an AST can earn are returned, never panicked. The
//! glyph instantiation's builder tests, including the
//! `SpatialDescend` refusal paired with its glyph control, live in
//! `gfx_structs::tests::builder_tests`.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2).

use crate::engine::builder::{
    build, BuildContext, BuildError, ChannelSrc, CountSrc, MutationSrc, MutatorNode, NoRuntime,
};
use crate::engine::tests::test_element::{add, value, TestElement, TestMutation};
use crate::engine::tree::Tree;

/// The AST over the test instantiation.
type Node = MutatorNode<TestElement, TestMutation>;

/// Resolves a template by offsetting its operand by the iteration
/// index, and answers `Runtime("double")` with `Add(2)`.
pub struct IndexContext;

impl BuildContext<TestElement, TestMutation> for IndexContext {
    fn mutation(&self, label: &str) -> TestMutation {
        assert_eq!(label, "double", "the AST carries its own label");
        TestMutation::Add(2)
    }

    fn resolve(&self, section: &str, index: usize, template: &TestMutation) -> TestMutation {
        assert_eq!(section, "cells");
        match template {
            TestMutation::Add(n) => TestMutation::Add(n + index as i64),
            TestMutation::Set(n) => TestMutation::Set(n + index as i64),
            TestMutation::SetToInherited => TestMutation::SetToInherited,
        }
    }
}

#[test]
fn test_engine_build_resolves_templates_and_runtime_payloads() {
    do_engine_build_resolves_templates_and_runtime_payloads();
}

/// A `Repeat` over three channels with an `Add(10)` template, beside a
/// literal-channel `Single` with a runtime payload, builds into a tree
/// whose walk adds `10 + index` to each repeated channel and `2` to
/// the runtime one — and nothing to the channel the `Repeat` skipped.
pub fn do_engine_build_resolves_templates_and_runtime_payloads() {
    let node: Node = MutatorNode::Void {
        channel: 0,
        children: vec![
            MutatorNode::Repeat {
                section: "cells".into(),
                channel_base: 1,
                count: CountSrc::Literal(4),
                skip_indices: vec![2],
                template: Box::new(MutatorNode::Single {
                    channel: ChannelSrc::SectionIndex,
                    mutation: MutationSrc::Template(TestMutation::Add(10)),
                }),
            },
            MutatorNode::Single {
                channel: ChannelSrc::Literal(9),
                mutation: MutationSrc::Runtime("double".into()),
            },
        ],
    };
    let mt = build(&node, &IndexContext).expect("the AST builds");

    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let targets: Vec<_> = [1, 2, 3, 4, 9]
        .iter()
        .map(|&channel| add(&mut tree, root, TestElement::new(channel, 100)))
        .collect();
    mt.apply_to(&mut tree, &0);
    let values: Vec<i64> = targets.iter().map(|id| value(&tree, *id)).collect();
    assert_eq!(values, vec![110, 111, 100, 113, 102]);
}

#[test]
fn test_engine_build_refuses_a_root_repeat() {
    do_engine_build_refuses_a_root_repeat();
}

/// A `Repeat` at the root is [`BuildError::RootRepeat`], returned and
/// not panicked; the same `Repeat` under a `Void` builds.
pub fn do_engine_build_refuses_a_root_repeat() {
    let repeat: Node = MutatorNode::Repeat {
        section: "cells".into(),
        channel_base: 0,
        count: CountSrc::Literal(2),
        skip_indices: vec![],
        template: Box::new(MutatorNode::Single {
            channel: ChannelSrc::SectionIndex,
            mutation: MutationSrc::Template(TestMutation::Add(1)),
        }),
    };
    assert_eq!(build(&repeat, &IndexContext).err(), Some(BuildError::RootRepeat));

    let wrapped: Node = MutatorNode::Void {
        channel: 0,
        children: vec![repeat],
    };
    assert!(
        build(&wrapped, &IndexContext).is_ok(),
        "the control: as a child it builds"
    );
}

#[test]
fn test_engine_build_refuses_spatial_descend_without_geometry() {
    do_engine_build_refuses_spatial_descend_without_geometry();
}

/// `TestElement` is not spatial, so a `SpatialDescend` anywhere in the
/// AST — here one level down, under a `Void` — is
/// [`BuildError::SpatialOnNonSpatial`], and no partial tree comes back.
/// The same AST with a `MapChildren` in its place builds.
pub fn do_engine_build_refuses_spatial_descend_without_geometry() {
    use crate::engine::builder::InstructionSpec;
    use crate::util::ordered_vec2::OrderedVec2;
    let with = |instruction: InstructionSpec<TestElement>| -> Node {
        MutatorNode::Void {
            channel: 0,
            children: vec![MutatorNode::Instruction {
                channel: 0,
                instruction,
                mutation: MutationSrc::None,
                children: vec![],
            }],
        }
    };
    let spatial = with(InstructionSpec::SpatialDescend(OrderedVec2::new_f32(0.0, 0.0)));
    assert_eq!(
        build(&spatial, &NoRuntime).err(),
        Some(BuildError::SpatialOnNonSpatial)
    );
    assert!(
        build(&with(InstructionSpec::MapChildren), &NoRuntime).is_ok(),
        "the control: another instruction in the same place builds"
    );
}
