// SPDX-License-Identifier: MPL-2.0

//! The walker that turns a [`MutatorNode`] + [`BuildContext`] into a
//! concrete [`MutatorTree<E, M>`], refusing ASTs that cannot mean
//! anything with a [`BuildError`] rather than a panic.

use crate::engine::element::TreeElement;
use crate::engine::mutation::Mutation;
use crate::engine::mutator::Mutator;
use crate::engine::tree::MutatorTree;
use indextree::NodeId;
use std::fmt;

use super::ast::{ChannelSrc, CountSrc, InstructionSpec, MutationListSrc, MutationSrc, MutatorNode};
use super::context::BuildContext;

/// Why [`build`] refused an AST. Each variant is a shape no mutator
/// tree can express; none depends on the target tree, so an AST that
/// builds once builds every time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// A [`MutatorNode::Repeat`] at the root. A `Repeat` describes the
    /// expansion of a parent's children; at the root there is no
    /// parent to expand under.
    RootRepeat,
    /// An [`InstructionSpec::SpatialDescend`] for an element type whose
    /// [`SPATIAL`](TreeElement::SPATIAL) is `false`. Refused here so
    /// the walker never meets one.
    SpatialOnNonSpatial,
    /// A [`ChannelSrc::SectionIndex`] outside a `Repeat` template,
    /// where there is no iteration to take a channel from.
    SectionIndexOutsideRepeat,
    /// A [`MutationSrc::Template`] outside a `Repeat` template, where
    /// there is no section and index to resolve it for.
    TemplateOutsideRepeat,
    /// A `Repeat` whose template is itself a `Repeat`. A template is
    /// the one node each iteration materializes; a `Repeat` is not a
    /// node but an expansion, so it has nothing to materialize.
    RepeatAsTemplate,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BuildError::RootRepeat => "a Repeat can only appear as a child, not as a tree root",
            BuildError::SpatialOnNonSpatial => "SpatialDescend on an element type without spatial bounds",
            BuildError::SectionIndexOutsideRepeat => {
                "ChannelSrc::SectionIndex used outside a Repeat template"
            }
            BuildError::TemplateOutsideRepeat => "MutationSrc::Template used outside a Repeat template",
            BuildError::RepeatAsTemplate => "a Repeat cannot be the template of another Repeat",
        })
    }
}

/// Build a [`MutatorTree<E, M>`] from `node` + `ctx`. The tree's root
/// is `node` itself — a `Void` / `Single` / `Macro` / `Instruction`.
///
/// Errors: [`BuildError::RootRepeat`] when `node` is a `Repeat`, and
/// any other [`BuildError`] the expansion meets anywhere in the AST; no
/// partial tree is returned.
///
/// Cost: O(nodes) in the expanded tree, plus one [`BuildContext`] call
/// per runtime-sourced count / mutation / template.
pub fn build<E, M, C>(node: &MutatorNode<E, M>, ctx: &C) -> Result<MutatorTree<E, M>, BuildError>
where
    E: TreeElement,
    M: Mutation<E>,
    C: BuildContext<E, M> + ?Sized,
{
    if let MutatorNode::Repeat { .. } = node {
        return Err(BuildError::RootRepeat);
    }
    let root_mutator = materialize_node(node, ctx, None)?;
    let mut mt = MutatorTree::new_with(root_mutator);
    let root = mt.root;
    for child in node_children(node) {
        append(child, root, &mut mt, ctx, None)?;
    }
    Ok(mt)
}

/// Iterate every `(section, index, channel)` tuple the spec will emit
/// at apply time, in tree-insertion (channel-ascending) order. Used by
/// initial-build paths that create the target tree from the same spec,
/// so its channel set matches what the mutator path will target.
///
/// Cost: O(expanded nodes). No allocation beyond the caller-provided
/// `out` buffer.
pub fn iter_section_channels<E, M, C>(
    node: &MutatorNode<E, M>,
    ctx: &C,
    out: &mut Vec<(String, usize, usize)>,
) where
    E: TreeElement,
    M: Mutation<E>,
    C: BuildContext<E, M> + ?Sized,
{
    match node {
        MutatorNode::Repeat {
            section,
            channel_base,
            count,
            skip_indices,
            ..
        } => {
            let n = resolve_count(count, ctx);
            for i in 0..n {
                if skip_indices.contains(&i) {
                    continue;
                }
                out.push((section.clone(), i, channel_base + i));
            }
        }
        MutatorNode::Void { children, .. }
        | MutatorNode::Instruction { children, .. }
        | MutatorNode::Macro { children, .. } => {
            for child in children {
                iter_section_channels(child, ctx, out);
            }
        }
        MutatorNode::Single { .. } => {}
    }
}

/// Per-iteration state the builder threads into a Repeat template.
#[derive(Clone, Copy)]
struct IterCtx<'a> {
    section: &'a str,
    index: usize,
    channel: usize,
}

fn node_children<E: TreeElement, M: Mutation<E>>(node: &MutatorNode<E, M>) -> &[MutatorNode<E, M>] {
    match node {
        MutatorNode::Void { children, .. } => children,
        MutatorNode::Instruction { children, .. } => children,
        MutatorNode::Macro { children, .. } => children,
        MutatorNode::Single { .. } | MutatorNode::Repeat { .. } => &[],
    }
}

fn resolve_count<E, M, C>(src: &CountSrc, ctx: &C) -> usize
where
    E: TreeElement,
    M: Mutation<E>,
    C: BuildContext<E, M> + ?Sized,
{
    match src {
        CountSrc::Literal(n) => *n,
        CountSrc::Runtime(name) => ctx.count(name),
    }
}

fn resolve_channel(src: &ChannelSrc, iter: Option<IterCtx<'_>>) -> Result<usize, BuildError> {
    match src {
        ChannelSrc::Literal(c) => Ok(*c),
        ChannelSrc::SectionIndex => iter
            .map(|it| it.channel)
            .ok_or(BuildError::SectionIndexOutsideRepeat),
    }
}

fn materialize_node<E, M, C>(
    node: &MutatorNode<E, M>,
    ctx: &C,
    iter: Option<IterCtx<'_>>,
) -> Result<Mutator<E, M>, BuildError>
where
    E: TreeElement,
    M: Mutation<E>,
    C: BuildContext<E, M> + ?Sized,
{
    Ok(match node {
        MutatorNode::Void { channel, .. } => Mutator::new_void(*channel),
        MutatorNode::Single { channel, mutation } => {
            let ch = resolve_channel(channel, iter)?;
            match materialize_mutation(mutation, ctx, iter)? {
                Some(m) => Mutator::new(m, ch),
                None => Mutator::new_void(ch),
            }
        }
        MutatorNode::Macro {
            channel, mutations, ..
        } => match mutations {
            MutationListSrc::Literal(list) => Mutator::new_macro(list.clone(), *channel),
            MutationListSrc::Runtime(label) => Mutator::new_macro(ctx.mutation_list(label), *channel),
        },
        MutatorNode::Instruction {
            channel,
            instruction,
            mutation,
            ..
        } => {
            if let InstructionSpec::SpatialDescend(_) = instruction {
                if !E::SPATIAL {
                    return Err(BuildError::SpatialOnNonSpatial);
                }
            }
            Mutator::Instruction {
                channel: *channel,
                instruction: instruction.clone().into_instruction(),
                mutation: materialize_mutation(mutation, ctx, iter)?,
            }
        }
        // `build` refuses a root `Repeat` and `append` expands a child
        // one without coming here, so the only `Repeat` that reaches
        // this arm is another `Repeat`'s template.
        MutatorNode::Repeat { .. } => return Err(BuildError::RepeatAsTemplate),
    })
}

fn append<E, M, C>(
    node: &MutatorNode<E, M>,
    parent: NodeId,
    mt: &mut MutatorTree<E, M>,
    ctx: &C,
    iter: Option<IterCtx<'_>>,
) -> Result<(), BuildError>
where
    E: TreeElement,
    M: Mutation<E>,
    C: BuildContext<E, M> + ?Sized,
{
    match node {
        MutatorNode::Repeat {
            section,
            channel_base,
            count,
            skip_indices,
            template,
        } => {
            let n = resolve_count(count, ctx);
            for i in 0..n {
                if skip_indices.contains(&i) {
                    continue;
                }
                let ch = channel_base + i;
                let sub_iter = Some(IterCtx {
                    section: section.as_str(),
                    index: i,
                    channel: ch,
                });
                let mutator = materialize_node(template, ctx, sub_iter)?;
                let id = mt.arena.new_node(mutator);
                parent.append(id, &mut mt.arena);
                for grandchild in node_children(template) {
                    append(grandchild, id, mt, ctx, sub_iter)?;
                }
            }
        }
        _ => {
            let mutator = materialize_node(node, ctx, iter)?;
            let id = mt.arena.new_node(mutator);
            parent.append(id, &mut mt.arena);
            for child in node_children(node) {
                append(child, id, mt, ctx, iter)?;
            }
        }
    }
    Ok(())
}

/// The mutation `src` stands for, or `None` for
/// [`MutationSrc::None`].
fn materialize_mutation<E, M, C>(
    src: &MutationSrc<E, M>,
    ctx: &C,
    iter: Option<IterCtx<'_>>,
) -> Result<Option<M>, BuildError>
where
    E: TreeElement,
    M: Mutation<E>,
    C: BuildContext<E, M> + ?Sized,
{
    Ok(match src {
        MutationSrc::None => None,
        MutationSrc::Runtime(label) => Some(ctx.mutation(label)),
        MutationSrc::Template(template) => {
            let it = iter.ok_or(BuildError::TemplateOutsideRepeat)?;
            Some(ctx.resolve(it.section, it.index, template))
        }
    })
}
