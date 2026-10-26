// SPDX-License-Identifier: MPL-2.0

//! The walker that turns a [`MutatorNode`] + [`BuildContext`] into a
//! concrete [`MutatorTree<E, M>`], refusing ASTs that cannot mean
//! anything with a [`BuildError`] rather than a panic.

use crate::engine::element::TreeElement;
use crate::engine::key::BranchKey;
use crate::engine::mutation::Mutation;
use crate::engine::mutator::Mutator;
use crate::engine::tree::{MutatorTree, Placement, TreeError};
use indextree::NodeId;
use std::fmt;

use super::ast::{ChannelSrc, CountSrc, InstructionSpec, MutationListSrc, MutationSrc, MutatorNode};
use super::context::BuildContext;

/// Why [`build`] refused an AST. Each variant but [`BuildError::Tree`]
/// is a shape no mutator tree can express; none depends on the target
/// tree, so an AST that builds once builds every time.
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
    /// A `Repeat` whose template names a different `kind` from the
    /// `Repeat`'s own. Every key a `Repeat` emits carries the
    /// `Repeat`'s kind, so a template of another kind has no one key
    /// it could mean.
    TemplateKindDiffers,
    /// The mutator tree refused an insertion. The builder inserts only
    /// under nodes it has just created, so this names a defect in the
    /// builder rather than in the AST.
    Tree(TreeError),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::RootRepeat => f.write_str("a Repeat can only appear as a child, not as a tree root"),
            BuildError::SpatialOnNonSpatial => {
                f.write_str("SpatialDescend on an element type without spatial bounds")
            }
            BuildError::SectionIndexOutsideRepeat => {
                f.write_str("ChannelSrc::SectionIndex used outside a Repeat template")
            }
            BuildError::TemplateOutsideRepeat => {
                f.write_str("MutationSrc::Template used outside a Repeat template")
            }
            BuildError::RepeatAsTemplate => f.write_str("a Repeat cannot be the template of another Repeat"),
            BuildError::TemplateKindDiffers => {
                f.write_str("a Repeat template must have the same kind as its Repeat")
            }
            BuildError::Tree(e) => write!(f, "the mutator tree refused an insertion: {e}"),
        }
    }
}

/// Build a [`MutatorTree<E, M>`] from `node` + `ctx`. The tree's root
/// is `node` itself — a `Void` / `Single` / `Macro` / `Instruction`.
/// Every child is inserted in key order
/// ([`MutatorTree::insert_child`] with [`Placement::Last`]), so the
/// tree holds the sorted-children invariant whatever order the AST
/// declares its children in; children with equal keys keep their
/// declaration order.
///
/// Errors: [`BuildError::RootRepeat`] when `node` is a `Repeat`, and
/// any other [`BuildError`] the expansion meets anywhere in the AST; no
/// partial tree is returned.
///
/// Cost: O(nodes) in the expanded tree, plus one [`BuildContext`] call
/// per runtime-sourced count / mutation / template, plus the sibling
/// scan of each sorted insert — none for children declared in
/// ascending key order.
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

/// Iterate every `(section, index, key)` tuple the spec will emit at
/// apply time, in declaration order. Used by initial-build paths that
/// create the target tree from the same spec, so its key set matches
/// what the mutator path will target.
///
/// Cost: O(expanded nodes). No allocation beyond the caller-provided
/// `out` buffer.
pub fn iter_section_keys<E, M, C>(
    node: &MutatorNode<E, M>,
    ctx: &C,
    out: &mut Vec<(String, usize, BranchKey<E::Kind>)>,
) where
    E: TreeElement,
    M: Mutation<E>,
    C: BuildContext<E, M> + ?Sized,
{
    match node {
        MutatorNode::Repeat {
            kind,
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
                out.push((section.clone(), i, BranchKey::new(*kind, channel_base + i)));
            }
        }
        MutatorNode::Void { children, .. }
        | MutatorNode::Instruction { children, .. }
        | MutatorNode::Macro { children, .. } => {
            for child in children {
                iter_section_keys(child, ctx, out);
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

/// The key a `Single` routes on: its own kind, and the channel its
/// [`ChannelSrc`] names — inside a `Repeat`, `SectionIndex` is the
/// iteration's channel.
fn resolve_key<K>(kind: K, src: &ChannelSrc, iter: Option<IterCtx<'_>>) -> Result<BranchKey<K>, BuildError> {
    let channel = match src {
        ChannelSrc::Literal(c) => *c,
        ChannelSrc::SectionIndex => iter
            .map(|it| it.channel)
            .ok_or(BuildError::SectionIndexOutsideRepeat)?,
    };
    Ok(BranchKey::new(kind, channel))
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
        MutatorNode::Void { kind, channel, .. } => Mutator::new_void(BranchKey::new(*kind, *channel)),
        MutatorNode::Single {
            kind,
            channel,
            mutation,
        } => {
            let key = resolve_key(*kind, channel, iter)?;
            match materialize_mutation(mutation, ctx, iter)? {
                Some(m) => Mutator::new(m, key),
                None => Mutator::new_void(key),
            }
        }
        MutatorNode::Macro {
            kind,
            channel,
            mutations,
            ..
        } => {
            let key = BranchKey::new(*kind, *channel);
            match mutations {
                MutationListSrc::Literal(list) => Mutator::new_macro(list.clone(), key),
                MutationListSrc::Runtime(label) => Mutator::new_macro(ctx.mutation_list(label), key),
            }
        }
        MutatorNode::Instruction {
            kind,
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
                key: BranchKey::new(*kind, *channel),
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

/// Insert `mutator` under `parent` in key order, after any sibling
/// with an equal key — so equal keys keep their declaration order.
fn insert<E, M>(
    mt: &mut MutatorTree<E, M>,
    parent: NodeId,
    mutator: Mutator<E, M>,
) -> Result<NodeId, BuildError>
where
    E: TreeElement,
    M: Mutation<E>,
{
    mt.insert_child(parent, mutator, Placement::Last)
        .map_err(BuildError::Tree)
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
            kind,
            section,
            channel_base,
            count,
            skip_indices,
            template,
        } => {
            if !matches!(template.as_ref(), MutatorNode::Repeat { .. }) && template.kind() != *kind {
                return Err(BuildError::TemplateKindDiffers);
            }
            let n = resolve_count(count, ctx);
            for i in 0..n {
                if skip_indices.contains(&i) {
                    continue;
                }
                let sub_iter = Some(IterCtx {
                    section: section.as_str(),
                    index: i,
                    channel: channel_base + i,
                });
                let mutator = materialize_node(template, ctx, sub_iter)?;
                let id = insert(mt, parent, mutator)?;
                for grandchild in node_children(template) {
                    append(grandchild, id, mt, ctx, sub_iter)?;
                }
            }
        }
        _ => {
            let mutator = materialize_node(node, ctx, iter)?;
            let id = insert(mt, parent, mutator)?;
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
