// SPDX-License-Identifier: MPL-2.0

//! Pure-data tests for the [builder](crate::engine::builder) over the
//! glyph instantiation. These exercise the builder against a stub
//! `BuildContext<GfxElement, GfxMutation>` so we don't need any picker
//! / widget / GPU state. The `BuildError` paths an element type with
//! no geometry takes live in `engine::tests::builder_tests`.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2): every `do_*` body
//! has a bench entry in `benches/test_bench.rs`.

use crate::core::primitives::{ApplyOperation, ColorFontRegions, Discriminated};
use crate::engine::builder::{
    build, iter_section_channels, BuildContext, BuildError, ChannelSrc, CountSrc, InstructionSpec,
    MutationListSrc, MutationSrc, MutatorNode, NoRuntime,
};
use crate::engine::element::TreeElement;
use crate::engine::mutator::{Instruction, MutatorType};
use crate::engine::predicate::Predicate;
use crate::gfx_structs::area::{GlyphArea, GlyphAreaCommand};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutation, GfxMutator};
use crate::gfx_structs::template::{resolve_from_area, CellField};
use glam::Vec2;
use std::collections::HashMap;

/// The AST over the glyph instantiation.
type Node = MutatorNode<GfxElement, GfxMutation>;

/// Stub context: yields one pre-built `GlyphArea` per index, and
/// honors runtime-count queries out of a `HashMap`. Per-section
/// routing is deliberately minimal — the tests below don't need
/// distinct areas per section, only per index.
struct StubCtx {
    areas: Vec<GlyphArea>,
    runtime_counts: HashMap<String, usize>,
}

impl StubCtx {
    fn with_areas(n: usize) -> Self {
        let mut areas = Vec::with_capacity(n);
        for i in 0..n {
            let text = format!("cell_{}", i);
            let mut a =
                GlyphArea::new_with_str(&text, 10.0, 12.0, Vec2::new(i as f32, 0.0), Vec2::new(20.0, 30.0));
            a.regions = ColorFontRegions::single_span(
                crate::util::grapheme_chad::count_grapheme_clusters(&text),
                None,
                None,
            );
            areas.push(a);
        }
        Self {
            areas,
            runtime_counts: HashMap::new(),
        }
    }
}

impl BuildContext<GfxElement, GfxMutation> for StubCtx {
    fn count(&self, name: &str) -> usize {
        *self.runtime_counts.get(name).unwrap_or(&0)
    }
    fn resolve(&self, _section: &str, index: usize, template: &Vec<CellField>) -> GfxMutation {
        resolve_from_area(&self.areas[index], template)
    }
}

fn single_with_text() -> Box<Node> {
    Box::new(MutatorNode::Single {
        channel: ChannelSrc::SectionIndex,
        mutation: MutationSrc::Template(vec![
            CellField::Text,
            CellField::Operation(ApplyOperation::Assign),
        ]),
    })
}

#[test]
fn test_repeat_literal_expands_to_consecutive_channels() {
    do_repeat_literal_expands_to_consecutive_channels();
}

/// Repeat with literal count expands to the right number of children
/// at consecutive channels in declaration order.
pub fn do_repeat_literal_expands_to_consecutive_channels() {
    let node = MutatorNode::Void {
        channel: 0,
        children: vec![MutatorNode::Repeat {
            section: "cells".into(),
            channel_base: 100,
            count: CountSrc::Literal(5),
            skip_indices: vec![],
            template: single_with_text(),
        }],
    };
    let ctx = StubCtx::with_areas(5);
    let mt = build(&node, &ctx).unwrap();
    let channels: Vec<usize> = mt
        .root
        .children(&mt.arena)
        .map(|id| mt.arena.get(id).unwrap().get().channel())
        .collect();
    assert_eq!(channels, vec![100, 101, 102, 103, 104]);
}

#[test]
fn test_repeat_skip_indices_stride_channels() {
    do_repeat_skip_indices_stride_channels();
}

/// `skip_indices` skips the named iteration indices; channels are
/// strided (skipped channels are absent, not renumbered).
pub fn do_repeat_skip_indices_stride_channels() {
    let node = MutatorNode::Void {
        channel: 0,
        children: vec![MutatorNode::Repeat {
            section: "cells".into(),
            channel_base: 300,
            count: CountSrc::Literal(5),
            skip_indices: vec![2],
            template: single_with_text(),
        }],
    };
    let ctx = StubCtx::with_areas(5);
    let mt = build(&node, &ctx).unwrap();
    let channels: Vec<usize> = mt
        .root
        .children(&mt.arena)
        .map(|id| mt.arena.get(id).unwrap().get().channel())
        .collect();
    assert_eq!(channels, vec![300, 301, 303, 304]);
}

#[test]
fn test_multiple_sections_concatenate_in_declaration_order() {
    do_multiple_sections_concatenate_in_declaration_order();
}

/// Multiple sections concatenate in declaration order; bands stay
/// disjoint.
pub fn do_multiple_sections_concatenate_in_declaration_order() {
    let node = MutatorNode::Void {
        channel: 0,
        children: vec![
            MutatorNode::Repeat {
                section: "a".into(),
                channel_base: 1,
                count: CountSrc::Literal(1),
                skip_indices: vec![],
                template: single_with_text(),
            },
            MutatorNode::Repeat {
                section: "b".into(),
                channel_base: 100,
                count: CountSrc::Literal(3),
                skip_indices: vec![],
                template: single_with_text(),
            },
            MutatorNode::Repeat {
                section: "c".into(),
                channel_base: 200,
                count: CountSrc::Literal(1),
                skip_indices: vec![],
                template: single_with_text(),
            },
        ],
    };
    let ctx = StubCtx::with_areas(3);
    let mt = build(&node, &ctx).unwrap();
    let channels: Vec<usize> = mt
        .root
        .children(&mt.arena)
        .map(|id| mt.arena.get(id).unwrap().get().channel())
        .collect();
    assert_eq!(channels, vec![1, 100, 101, 102, 200]);
}

#[test]
fn test_template_resolves_against_each_iterations_area() {
    do_template_resolves_against_each_iterations_area();
}

/// A glyph template resolves, per iteration, into the `AreaDelta`
/// today's picker relies on: every bare slot pulled out of *that
/// iteration's* area, every tagged slot baked in. The expected deltas
/// are written from the fixture's literal values, not computed by the
/// resolver under test.
pub fn do_template_resolves_against_each_iterations_area() {
    use crate::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    let node = MutatorNode::Void {
        channel: 0,
        children: vec![MutatorNode::Repeat {
            section: "cells".into(),
            channel_base: 10,
            count: CountSrc::Literal(2),
            skip_indices: vec![],
            template: Box::new(MutatorNode::Single {
                channel: ChannelSrc::SectionIndex,
                mutation: MutationSrc::Template(vec![
                    CellField::Text,
                    CellField::position,
                    CellField::Operation(ApplyOperation::Assign),
                ]),
            }),
        }],
    };
    let mt = build(&node, &StubCtx::with_areas(2)).unwrap();
    let built: Vec<GfxMutation> = mt
        .root
        .children(&mt.arena)
        .map(|id| match mt.arena.get(id).unwrap().get() {
            crate::gfx_structs::mutator::GfxMutator::Single { mutation, .. } => mutation.clone(),
            other => panic!("expected Single, got {other:?}"),
        })
        .collect();
    let expected = |text: &str, x: f32| {
        GfxMutation::area_delta(DeltaGlyphArea::new(vec![
            GlyphAreaField::Text(text.to_string()),
            GlyphAreaField::position(x, 0.0),
            GlyphAreaField::Operation(ApplyOperation::Assign),
        ]))
    };
    assert_eq!(built, vec![expected("cell_0", 0.0), expected("cell_1", 1.0)]);
}

#[test]
fn test_iter_section_channels_walks_in_order() {
    do_iter_section_channels_walks_in_order();
}

/// `iter_section_channels` emits every `(section, index, channel)`
/// tuple in tree-insertion order; used by the initial-build path so
/// the channel set stays aligned with the mutator path.
pub fn do_iter_section_channels_walks_in_order() {
    let node = MutatorNode::Void {
        channel: 0,
        children: vec![
            MutatorNode::Repeat {
                section: "a".into(),
                channel_base: 1,
                count: CountSrc::Literal(1),
                skip_indices: vec![],
                template: single_with_text(),
            },
            MutatorNode::Repeat {
                section: "b".into(),
                channel_base: 300,
                count: CountSrc::Literal(3),
                skip_indices: vec![1],
                template: single_with_text(),
            },
        ],
    };
    let ctx = StubCtx::with_areas(0);
    let mut out = Vec::new();
    iter_section_channels(&node, &ctx, &mut out);
    assert_eq!(
        out,
        vec![
            ("a".to_string(), 0, 1),
            ("b".to_string(), 0, 300),
            ("b".to_string(), 2, 302),
        ]
    );
}

#[test]
fn test_repeat_runtime_count_consults_context() {
    do_repeat_runtime_count_consults_context();
}

/// Runtime counts: builder asks the context how many cells the
/// section has at build time. Proves the design absorbs the
/// console overlay's `scrollback_rows`-style use case.
pub fn do_repeat_runtime_count_consults_context() {
    let node = MutatorNode::Void {
        channel: 0,
        children: vec![MutatorNode::Repeat {
            section: "rows".into(),
            channel_base: 1000,
            count: CountSrc::Runtime("row_count".into()),
            skip_indices: vec![],
            template: single_with_text(),
        }],
    };
    let mut ctx = StubCtx::with_areas(7);
    ctx.runtime_counts.insert("row_count".into(), 3);
    let mt = build(&node, &ctx).unwrap();
    let channels: Vec<usize> = mt
        .root
        .children(&mt.arena)
        .map(|id| mt.arena.get(id).unwrap().get().channel())
        .collect();
    assert_eq!(channels, vec![1000, 1001, 1002]);
}

// =============================================================
// Non-Repeat tree shapes — exercise the variants the picker spec
// doesn't use but that the AST has to carry for the named
// extensibility trajectory (console overlay, scope topologies,
// future script-API mutators).
// =============================================================

#[test]
fn test_void_root_with_no_children() {
    do_void_root_with_no_children();
}

/// Tree rooted at a `Void` with no children produces an empty root.
pub fn do_void_root_with_no_children() {
    let node: Node = MutatorNode::Void {
        channel: 42,
        children: vec![],
    };
    let ctx = StubCtx::with_areas(0);
    let mt = build(&node, &ctx).unwrap();
    let root = mt.arena.get(mt.root).unwrap().get();
    assert!(matches!(root, GfxMutator::Void { channel: 42 }));
    assert_eq!(mt.root.children(&mt.arena).count(), 0);
}

#[test]
fn test_single_as_tree_root_with_literal_channel() {
    do_single_as_tree_root_with_literal_channel();
}

/// `ChannelSrc::Literal` resolves without needing a `Repeat`
/// iteration context — proves a `Single` can sit at the tree root.
/// With `MutationSrc::None` there is no payload, and a mutator's "no
/// payload" is structural: the node builds as a `Void` on the same
/// channel.
pub fn do_single_as_tree_root_with_literal_channel() {
    let node: Node = MutatorNode::Single {
        channel: ChannelSrc::Literal(7),
        mutation: MutationSrc::None,
    };
    // No-op context — Single with MutationSrc::None needs nothing.
    let mt = build(&node, &NoRuntime).unwrap();
    let root = mt.arena.get(mt.root).unwrap().get();
    assert!(
        matches!(root, GfxMutator::Void { channel: 7 }),
        "expected Void on channel 7, got {root:?}"
    );
}

#[test]
fn test_mutation_src_runtime_consults_context() {
    do_mutation_src_runtime_consults_context();
}

/// `MutationSrc::Runtime` consults the context via
/// [`BuildContext::mutation`] with the label the variant carries —
/// inside a `Repeat` too, where the label is still the variant's own
/// and not the section's name — and the node builds as a `Single`
/// carrying what the context returned.
pub fn do_mutation_src_runtime_consults_context() {
    struct RuntimeCtx {
        asked_with: std::cell::RefCell<Vec<String>>,
    }
    impl BuildContext<GfxElement, GfxMutation> for RuntimeCtx {
        fn mutation(&self, label: &str) -> GfxMutation {
            self.asked_with.borrow_mut().push(label.to_string());
            GfxMutation::area_command(GlyphAreaCommand::NudgeRight(4.0))
        }
        fn resolve(&self, _: &str, _: usize, _: &Vec<CellField>) -> GfxMutation {
            unreachable!("no template in this AST")
        }
    }
    let node: Node = MutatorNode::Void {
        channel: 0,
        children: vec![
            MutatorNode::Single {
                channel: ChannelSrc::Literal(5),
                mutation: MutationSrc::Runtime("nudge".into()),
            },
            MutatorNode::Repeat {
                section: "cells".into(),
                channel_base: 10,
                count: CountSrc::Literal(1),
                skip_indices: vec![],
                template: Box::new(MutatorNode::Single {
                    channel: ChannelSrc::SectionIndex,
                    mutation: MutationSrc::Runtime("per_cell".into()),
                }),
            },
        ],
    };
    let ctx = RuntimeCtx {
        asked_with: std::cell::RefCell::new(Vec::new()),
    };
    let mt = build(&node, &ctx).unwrap();
    assert_eq!(
        ctx.asked_with.borrow().as_slice(),
        &["nudge".to_string(), "per_cell".to_string()]
    );
    let first = mt.root.children(&mt.arena).next().unwrap();
    match mt.arena.get(first).unwrap().get() {
        GfxMutator::Single { channel, mutation } => {
            assert_eq!(*channel, 5);
            assert_eq!(
                *mutation,
                GfxMutation::area_command(GlyphAreaCommand::NudgeRight(4.0))
            );
        }
        other => panic!("expected Single, got {other:?}"),
    }
}

#[test]
fn test_macro_pulls_mutation_list_from_runtime_context() {
    do_macro_pulls_mutation_list_from_runtime_context();
}

/// `Macro` with `MutationListSrc::Runtime` pulls its list from the
/// context — proves the AST carries the scope-topology shape (which
/// uses Macro with runtime payloads).
pub fn do_macro_pulls_mutation_list_from_runtime_context() {
    struct MacroCtx;
    impl BuildContext<GfxElement, GfxMutation> for MacroCtx {
        fn mutation_list(&self, label: &str) -> Vec<GfxMutation> {
            assert_eq!(label, "scope_mutations");
            vec![
                GfxMutation::area_command(GlyphAreaCommand::NudgeRight(1.0)),
                GfxMutation::area_command(GlyphAreaCommand::NudgeUp(2.0)),
            ]
        }
        fn resolve(&self, _: &str, _: usize, _: &Vec<CellField>) -> GfxMutation {
            unreachable!("no template in this AST")
        }
    }
    let node: Node = MutatorNode::Macro {
        channel: 0,
        mutations: MutationListSrc::Runtime("scope_mutations".into()),
        children: vec![],
    };
    let mt = build(&node, &MacroCtx).unwrap();
    let root = mt.arena.get(mt.root).unwrap().get();
    match root {
        GfxMutator::Macro { channel, mutations } => {
            assert_eq!(*channel, 0);
            assert_eq!(mutations.len(), 2);
        }
        other => panic!("expected Macro root, got {other:?}"),
    }
}

#[test]
fn test_instruction_with_children_mirrors_scope_topology() {
    do_instruction_with_children_mirrors_scope_topology();
}

/// `Instruction` with children mirrors the `SelfAndDescendants`
/// scope-topology shape: the root Instruction wraps one Macro child
/// whose mutations come from runtime. The walker descends into
/// Instruction children the same way it descends into Void children.
pub fn do_instruction_with_children_mirrors_scope_topology() {
    struct ScopeCtx;
    impl BuildContext<GfxElement, GfxMutation> for ScopeCtx {
        fn mutation_list(&self, _label: &str) -> Vec<GfxMutation> {
            vec![GfxMutation::area_command(GlyphAreaCommand::NudgeRight(1.0))]
        }
        fn resolve(&self, _: &str, _: usize, _: &Vec<CellField>) -> GfxMutation {
            unreachable!("no template in this AST")
        }
    }
    let node: Node = MutatorNode::Instruction {
        channel: 0,
        instruction: InstructionSpec::RepeatWhileAlwaysTrue,
        mutation: MutationSrc::None,
        children: vec![MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Runtime("leaf".into()),
            children: vec![],
        }],
    };
    let mt = build(&node, &ScopeCtx).unwrap();
    let root = mt.arena.get(mt.root).unwrap().get();
    assert!(matches!(root.variant(), MutatorType::Instruction));
    assert!(
        matches!(root, GfxMutator::Instruction { mutation: None, .. }),
        "MutationSrc::None on an Instruction builds with no payload"
    );
    let child_ids: Vec<_> = mt.root.children(&mt.arena).collect();
    assert_eq!(child_ids.len(), 1);
    let child = mt.arena.get(child_ids[0]).unwrap().get();
    assert!(matches!(child.variant(), MutatorType::Macro));
}

#[test]
fn test_instruction_spec_repeat_while_always_true_materializes_predicate() {
    do_instruction_spec_repeat_while_always_true_materializes_predicate();
}

/// `InstructionSpec::RepeatWhileAlwaysTrue` materializes to a real
/// `Instruction::RepeatWhile(always_true())` without needing the
/// consumer to spell the predicate out.
pub fn do_instruction_spec_repeat_while_always_true_materializes_predicate() {
    let node: Node = MutatorNode::Instruction {
        channel: 5,
        instruction: InstructionSpec::RepeatWhileAlwaysTrue,
        mutation: MutationSrc::None,
        children: vec![],
    };
    let mt = build(&node, &NoRuntime).unwrap();
    match mt.arena.get(mt.root).unwrap().get() {
        GfxMutator::Instruction {
            instruction: Instruction::RepeatWhile(predicate),
            channel,
            ..
        } => {
            assert_eq!(*channel, 5);
            assert!(predicate.always_match);
        }
        other => panic!("expected Instruction::RepeatWhile, got {other:?}"),
    }
}

#[test]
fn test_instruction_spec_spatial_descend_carries_its_point() {
    do_instruction_spec_spatial_descend_carries_its_point();
}

/// `InstructionSpec::SpatialDescend` carries its point through to
/// `Instruction::SpatialDescend` for the glyph instantiation, which is
/// spatial. (Its refusal for a non-spatial element type, from the same
/// JSON, is `test_spatial_descend_builds_only_for_a_spatial_element`.)
pub fn do_instruction_spec_spatial_descend_carries_its_point() {
    use crate::util::ordered_vec2::OrderedVec2;
    let node: Node = MutatorNode::Instruction {
        channel: 0,
        instruction: InstructionSpec::SpatialDescend(OrderedVec2::new_f32(3.0, 4.0)),
        mutation: MutationSrc::None,
        children: vec![],
    };
    let mt = build(&node, &NoRuntime).unwrap();
    match mt.arena.get(mt.root).unwrap().get() {
        GfxMutator::Instruction {
            instruction: Instruction::SpatialDescend(point),
            ..
        } => assert_eq!(*point, OrderedVec2::new_f32(3.0, 4.0)),
        other => panic!("expected Instruction::SpatialDescend, got {other:?}"),
    }
}

#[test]
fn test_repeat_template_wrapping_void_threads_iter_channel() {
    do_repeat_template_wrapping_void_threads_iter_channel();
}

/// Nested `Repeat`-inside-`Void` template: the builder threads the
/// per-iteration channel through intermediate wrapper nodes so an
/// inner `ChannelSrc::SectionIndex` still resolves to
/// `channel_base + i`. Exercises the iter-threading that fires
/// when a consumer wraps its cell template in a grouping Void (e.g.
/// for future per-cell decorations).
pub fn do_repeat_template_wrapping_void_threads_iter_channel() {
    let node: Node = MutatorNode::Void {
        channel: 0,
        children: vec![MutatorNode::Repeat {
            section: "wrapped".into(),
            channel_base: 50,
            count: CountSrc::Literal(3),
            skip_indices: vec![],
            template: Box::new(MutatorNode::Void {
                channel: 999, // Void's own channel — literal, not iterated.
                children: vec![MutatorNode::Single {
                    channel: ChannelSrc::SectionIndex,
                    mutation: MutationSrc::Runtime("inner".into()),
                }],
            }),
        }],
    };
    struct InnerCtx;
    impl BuildContext<GfxElement, GfxMutation> for InnerCtx {
        fn mutation(&self, _label: &str) -> GfxMutation {
            GfxMutation::area_command(GlyphAreaCommand::NudgeRight(1.0))
        }
        fn resolve(&self, _: &str, _: usize, _: &Vec<CellField>) -> GfxMutation {
            unreachable!("no template in this AST")
        }
    }
    let mt = build(&node, &InnerCtx).unwrap();
    // Root Void → 3 wrapper Voids → each wrapper Void has 1 Single child.
    let wrapper_ids: Vec<_> = mt.root.children(&mt.arena).collect();
    assert_eq!(wrapper_ids.len(), 3);
    for (expected_inner_ch, wrapper_id) in (50..).zip(wrapper_ids.iter()) {
        let wrapper = mt.arena.get(*wrapper_id).unwrap().get();
        assert!(matches!(wrapper, GfxMutator::Void { channel: 999 }));
        let inner_ids: Vec<_> = wrapper_id.children(&mt.arena).collect();
        assert_eq!(inner_ids.len(), 1);
        let inner = mt.arena.get(inner_ids[0]).unwrap().get();
        match inner {
            GfxMutator::Single { channel, .. } => {
                assert_eq!(*channel, expected_inner_ch);
            }
            other => panic!("expected Single inner, got {other:?}"),
        }
    }
}

// =============================================================
// Refused shapes — every AST no mutator tree can express is a
// `BuildError`, never a panic: these ASTs arrive from JSON (custom
// mutations in a map, widget specs), and the build runs on an
// interactive path. Each refusal is paired with its control — the
// nearest shape that does build — so the error is pinned to the
// shape and not to the fixture.
// =============================================================

#[test]
fn test_repeat_at_tree_root_is_a_build_error() {
    do_repeat_at_tree_root_is_a_build_error();
}

/// A `Repeat` at the tree root is a misuse — Repeats only describe
/// children expansion. The builder refuses it rather than producing a
/// malformed tree; the same `Repeat` one level down builds.
pub fn do_repeat_at_tree_root_is_a_build_error() {
    let repeat = MutatorNode::Repeat {
        section: "x".into(),
        channel_base: 0,
        count: CountSrc::Literal(1),
        skip_indices: vec![],
        template: single_with_text(),
    };
    let ctx = StubCtx::with_areas(1);
    assert_eq!(build(&repeat, &ctx).err(), Some(BuildError::RootRepeat));

    let wrapped: Node = MutatorNode::Void {
        channel: 0,
        children: vec![repeat],
    };
    assert!(build(&wrapped, &ctx).is_ok(), "the control: as a child it builds");
}

#[test]
fn test_channel_src_section_index_outside_repeat_is_a_build_error() {
    do_channel_src_section_index_outside_repeat_is_a_build_error();
}

/// `ChannelSrc::SectionIndex` only makes sense inside a `Repeat`
/// template. Using it at a tree root (or in any non-iterated
/// position) is refused; inside a `Repeat` the same node builds.
pub fn do_channel_src_section_index_outside_repeat_is_a_build_error() {
    let single = MutatorNode::Single {
        channel: ChannelSrc::SectionIndex,
        mutation: MutationSrc::None,
    };
    assert_eq!(
        build(&single, &NoRuntime).err(),
        Some(BuildError::SectionIndexOutsideRepeat)
    );

    let inside: Node = MutatorNode::Void {
        channel: 0,
        children: vec![MutatorNode::Repeat {
            section: "x".into(),
            channel_base: 0,
            count: CountSrc::Literal(1),
            skip_indices: vec![],
            template: Box::new(single),
        }],
    };
    assert!(
        build(&inside, &NoRuntime).is_ok(),
        "the control: in a template it builds"
    );
}

#[test]
fn test_template_outside_repeat_is_a_build_error() {
    do_template_outside_repeat_is_a_build_error();
}

/// `MutationSrc::Template` needs a `Repeat` iteration context to
/// resolve against — outside one it is refused; inside one the same
/// template builds.
pub fn do_template_outside_repeat_is_a_build_error() {
    let node: Node = MutatorNode::Single {
        channel: ChannelSrc::Literal(0),
        mutation: MutationSrc::Template(vec![CellField::Text]),
    };
    let ctx = StubCtx::with_areas(1);
    assert_eq!(build(&node, &ctx).err(), Some(BuildError::TemplateOutsideRepeat));

    let inside: Node = MutatorNode::Void {
        channel: 0,
        children: vec![MutatorNode::Repeat {
            section: "x".into(),
            channel_base: 0,
            count: CountSrc::Literal(1),
            skip_indices: vec![],
            template: single_with_text(),
        }],
    };
    assert!(
        build(&inside, &ctx).is_ok(),
        "the control: in a template it builds"
    );
}

#[test]
fn test_repeat_as_template_is_a_build_error() {
    do_repeat_as_template_is_a_build_error();
}

/// A `Repeat` whose template is itself a `Repeat` has nothing to
/// materialize per iteration and is refused. A `Repeat` *inside* a template
/// node (one level further down) is a child expansion and builds.
pub fn do_repeat_as_template_is_a_build_error() {
    let inner = MutatorNode::Repeat {
        section: "inner".into(),
        channel_base: 0,
        count: CountSrc::Literal(1),
        skip_indices: vec![],
        template: single_with_text(),
    };
    let outer = |template: Node| -> Node {
        MutatorNode::Void {
            channel: 0,
            children: vec![MutatorNode::Repeat {
                section: "outer".into(),
                channel_base: 0,
                count: CountSrc::Literal(1),
                skip_indices: vec![],
                template: Box::new(template),
            }],
        }
    };
    let ctx = StubCtx::with_areas(1);
    assert_eq!(
        build(&outer(inner.clone()), &ctx).err(),
        Some(BuildError::RepeatAsTemplate)
    );

    let grouped = MutatorNode::Void {
        channel: 0,
        children: vec![inner],
    };
    assert!(
        build(&outer(grouped), &ctx).is_ok(),
        "the control: one level down it builds"
    );
}

#[test]
fn test_spatial_descend_builds_only_for_a_spatial_element() {
    do_spatial_descend_builds_only_for_a_spatial_element();
}

/// The engine refuses `SpatialDescend` for an element type with no
/// geometry when the mutator tree is built, so its walker never meets
/// one. The **same JSON** builds for the glyph instantiation, which is
/// spatial — the negative control that pins the refusal to the
/// element type and not to the AST.
pub fn do_spatial_descend_builds_only_for_a_spatial_element() {
    use crate::engine::tests::test_element::{TestElement, TestMutation};
    const SRC: &str = r#"{
        "Instruction": {
            "channel": 0,
            "instruction": { "SpatialDescend": { "x": 1.0, "y": 2.0 } }
        }
    }"#;
    // Checked at compile time: the precondition is a property of the
    // two types, not of a value.
    const {
        assert!(
            !TestElement::SPATIAL && GfxElement::SPATIAL,
            "precondition: the two element types differ"
        )
    };

    let for_test: MutatorNode<TestElement, TestMutation> =
        serde_json::from_str(SRC).expect("parse for TestElement");
    assert_eq!(
        build(&for_test, &NoRuntime).err(),
        Some(BuildError::SpatialOnNonSpatial)
    );

    let for_glyph: Node = serde_json::from_str(SRC).expect("parse for GfxElement");
    let mt = build(&for_glyph, &NoRuntime).expect("the same AST builds for a spatial element");
    assert!(matches!(
        mt.arena.get(mt.root).unwrap().get(),
        GfxMutator::Instruction {
            instruction: Instruction::SpatialDescend(_),
            ..
        }
    ));
}

// =============================================================
// JSON roundtrip — the AST's load-bearing purpose is deserializing
// from JSON (widgets, custom mutations, future user scripts). Drift
// in serde renames / variant shapes must surface here rather than
// silently at picker-open time.
// =============================================================

#[test]
fn test_json_roundtrip_minimal_repeat_tree() {
    do_json_roundtrip_minimal_repeat_tree();
}

/// A compact MutatorNode literal round-trips through serde_json
/// with the same shape we'd hand-build in Rust. Catches accidental
/// renames of variant tags or field names that would break the
/// picker's `mutator_spec` block without warning.
pub fn do_json_roundtrip_minimal_repeat_tree() {
    let src = r#"{
        "Void": {
            "channel": 0,
            "children": [
                {
                    "Repeat": {
                        "section": "cells",
                        "channel_base": 100,
                        "count": { "Literal": 3 },
                        "template": {
                            "Single": {
                                "channel": "SectionIndex",
                                "mutation": { "Template": ["Text", { "Operation": "Assign" }] }
                            }
                        }
                    }
                }
            ]
        }
    }"#;
    let node: Node = serde_json::from_str(src).expect("parse MutatorNode");
    // Shape assertions — same structure as a hand-built AST.
    let MutatorNode::Void { channel, children } = &node else {
        panic!("root must be Void");
    };
    assert_eq!(*channel, 0);
    assert_eq!(children.len(), 1);
    let MutatorNode::Repeat {
        section,
        channel_base,
        count,
        skip_indices,
        template,
    } = &children[0]
    else {
        panic!("child must be Repeat");
    };
    assert_eq!(section, "cells");
    assert_eq!(*channel_base, 100);
    assert!(matches!(count, CountSrc::Literal(3)));
    assert!(skip_indices.is_empty());
    let MutatorNode::Single { channel, mutation } = template.as_ref() else {
        panic!("template must be Single");
    };
    assert!(matches!(channel, ChannelSrc::SectionIndex));
    let MutationSrc::Template(fields) = mutation else {
        panic!("mutation must be Template");
    };
    assert_eq!(fields.len(), 2);
    assert!(matches!(fields[0], CellField::Text));
    assert!(matches!(fields[1], CellField::Operation(ApplyOperation::Assign)));
}

#[test]
fn test_json_skip_indices_defaults_to_empty() {
    do_json_skip_indices_defaults_to_empty();
}

/// `skip_indices` default to an empty Vec when omitted in JSON —
/// the overwhelming case, so the picker JSON stays terse for
/// sections that don't skip anything.
pub fn do_json_skip_indices_defaults_to_empty() {
    let src = r#"{
        "Repeat": {
            "section": "cells",
            "channel_base": 0,
            "count": { "Literal": 1 },
            "template": { "Single": { "channel": "SectionIndex", "mutation": "None" } }
        }
    }"#;
    let node: Node = serde_json::from_str(src).expect("parse MutatorNode");
    let MutatorNode::Repeat { skip_indices, .. } = node else {
        panic!("expected Repeat");
    };
    assert!(skip_indices.is_empty());
}

#[test]
fn test_json_instruction_mutation_defaults_to_none() {
    do_json_instruction_mutation_defaults_to_none();
}

/// `Instruction.mutation` defaults to `MutationSrc::None` when
/// omitted — scope-topology shapes (RepeatWhile over descendants)
/// don't need an explicit mutation payload on the instruction node
/// itself, and we want the JSON to reflect that.
pub fn do_json_instruction_mutation_defaults_to_none() {
    let src = r#"{
        "Instruction": {
            "channel": 0,
            "instruction": "RepeatWhileAlwaysTrue"
        }
    }"#;
    let node: Node = serde_json::from_str(src).expect("parse MutatorNode");
    let MutatorNode::Instruction { mutation, .. } = node else {
        panic!("expected Instruction");
    };
    assert!(matches!(mutation, MutationSrc::None));
}

#[test]
fn test_json_mutation_src_runtime_carries_its_label() {
    do_json_mutation_src_runtime_carries_its_label();
}

/// `MutationSrc::Runtime` carries its label in JSON as
/// `{ "Runtime": "<label>" }`, the same spelling as
/// `MutationListSrc::Runtime`.
pub fn do_json_mutation_src_runtime_carries_its_label() {
    let src = r#"{ "Single": { "channel": { "Literal": 2 }, "mutation": { "Runtime": "nudge" } } }"#;
    let node: Node = serde_json::from_str(src).expect("parse MutatorNode");
    let MutatorNode::Single {
        mutation: MutationSrc::Runtime(label),
        ..
    } = node
    else {
        panic!("expected Single with a Runtime mutation");
    };
    assert_eq!(label, "nudge");
}

#[test]
fn test_json_instruction_spec_map_children_materializes_correctly() {
    do_json_instruction_spec_map_children_materializes_correctly();
}

/// `InstructionSpec::MapChildren` round-trips through JSON and
/// materializes to `Instruction::MapChildren` via
/// `into_instruction`. Guards the seam authors rely on when
/// declaring MapChildren-shaped mutators in a custom_mutations
/// bundle.
pub fn do_json_instruction_spec_map_children_materializes_correctly() {
    let src = r#"{
        "Instruction": {
            "channel": 3,
            "instruction": "MapChildren"
        }
    }"#;
    let node: Node = serde_json::from_str(src).expect("parse MutatorNode");
    let MutatorNode::Instruction {
        channel, instruction, ..
    } = node
    else {
        panic!("expected Instruction");
    };
    assert_eq!(channel, 3);
    // Materialize the InstructionSpec into a concrete Instruction and
    // verify the variant pairs with the walker primitive.
    let inst = instruction.clone().into_instruction();
    assert!(matches!(inst, Instruction::MapChildren));
}

// =============================================================
// Full serialize <-> deserialize round-trips — catches accidental
// serde field renames / shape drift that a one-way parse test
// would miss (parse-only tests can't see a serialize-side rename
// until a downstream consumer trips on it).
// =============================================================

/// Helper: round-trip a MutatorNode through serde_json and assert
/// the result parses back to a structurally identical value. Uses
/// `serde_json::Value` for comparison so field-order drift doesn't
/// spuriously fail.
fn assert_mutator_node_json_round_trips(node: &Node) {
    let first = serde_json::to_string(node).expect("first serialize");
    let reparsed: Node = serde_json::from_str(&first).expect("reparse");
    let second = serde_json::to_string(&reparsed).expect("second serialize");
    let v1: serde_json::Value = serde_json::from_str(&first).unwrap();
    let v2: serde_json::Value = serde_json::from_str(&second).unwrap();
    assert_eq!(v1, v2, "MutatorNode JSON drifted across round-trip");
}

#[test]
fn test_json_round_trip_macro_with_literal_mutations_preserves_shape() {
    do_json_round_trip_macro_with_literal_mutations_preserves_shape();
}

pub fn do_json_round_trip_macro_with_literal_mutations_preserves_shape() {
    // The pure-data shape that scope::self_only produces — a Macro
    // on channel 0 carrying a `MutationListSrc::Literal` list.
    let node = MutatorNode::Macro {
        channel: 0,
        mutations: MutationListSrc::Literal(vec![
            GfxMutation::area_command(GlyphAreaCommand::NudgeRight(3.0)),
            GfxMutation::area_command(GlyphAreaCommand::GrowFont(1.5)),
        ]),
        children: vec![],
    };
    assert_mutator_node_json_round_trips(&node);
}

#[test]
fn test_json_round_trip_instruction_map_children_preserves_shape() {
    do_json_round_trip_instruction_map_children_preserves_shape();
}

pub fn do_json_round_trip_instruction_map_children_preserves_shape() {
    let node = MutatorNode::Instruction {
        channel: 7,
        instruction: InstructionSpec::MapChildren,
        mutation: MutationSrc::None,
        children: vec![MutatorNode::Single {
            channel: ChannelSrc::Literal(3),
            mutation: MutationSrc::None,
        }],
    };
    assert_mutator_node_json_round_trips(&node);
}

#[test]
fn test_json_round_trip_repeat_inside_map_children_preserves_shape() {
    do_json_round_trip_repeat_inside_map_children_preserves_shape();
}

pub fn do_json_round_trip_repeat_inside_map_children_preserves_shape() {
    // The canonical declarative size-aware layout shape — Repeat
    // under MapChildren, with SectionIndex channels and a template.
    // The Repeat-composition test relies on this exact serde layout.
    let node = MutatorNode::Instruction {
        channel: 0,
        instruction: InstructionSpec::MapChildren,
        mutation: MutationSrc::None,
        children: vec![MutatorNode::Repeat {
            section: "children".into(),
            channel_base: 0,
            count: CountSrc::Runtime("children".into()),
            skip_indices: vec![],
            template: Box::new(MutatorNode::Single {
                channel: ChannelSrc::SectionIndex,
                mutation: MutationSrc::Template(vec![CellField::position]),
            }),
        }],
    };
    assert_mutator_node_json_round_trips(&node);
}

#[test]
fn test_json_round_trip_repeat_while_predicate_preserves_shape() {
    do_json_round_trip_repeat_while_predicate_preserves_shape();
}

/// `Predicate<GfxElement>` survives the AST's JSON round-trip with its
/// glyph field vocabulary — the element type fixes what a predicate's
/// fields mean, and the AST carries them verbatim.
pub fn do_json_round_trip_repeat_while_predicate_preserves_shape() {
    use crate::engine::predicate::Comparator;
    use crate::gfx_structs::element::GfxElementField;
    let node: Node = MutatorNode::Instruction {
        channel: 0,
        instruction: InstructionSpec::RepeatWhile(Predicate {
            fields: vec![(GfxElementField::Channel(4), Comparator::equals())],
            always_match: false,
        }),
        mutation: MutationSrc::None,
        children: vec![],
    };
    assert_mutator_node_json_round_trips(&node);
}
