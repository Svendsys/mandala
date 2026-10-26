// SPDX-License-Identifier: MPL-2.0

//! Runtime-value interface the [`build`](crate::engine::builder::build())
//! function consults while expanding an AST. Consumers implement only
//! the methods their AST actually exercises; the runtime look-ups
//! default to `unreachable!()` so misuse is loud.

use crate::engine::element::TreeElement;
use crate::engine::mutation::Mutation;

/// Runtime look-up surface for the mutator builder. Each method
/// corresponds to one runtime-sourced AST variant; consumers implement
/// only those their [`MutatorNode`](super::MutatorNode) exercises.
///
/// The defaults of [`count`](Self::count),
/// [`mutation`](Self::mutation) and
/// [`mutation_list`](Self::mutation_list) panic loudly via
/// `unreachable!()` so a misused AST fails immediately rather than
/// silently producing a wrong mutator tree.
/// [`resolve`](Self::resolve) has no default: what a template means is
/// the payload's to say, so every context states it — a context for
/// ASTs that carry no templates says so in its own `resolve`.
pub trait BuildContext<E: TreeElement, M: Mutation<E>> {
    /// Resolve a runtime cell count for `Repeat { count: Runtime(_) }`.
    /// `name` is the `Runtime` label the AST carries; the consumer
    /// disambiguates among multiple `Repeat` sections by this label.
    /// Called once per `Repeat` node during a build.
    ///
    /// Cost: consumer-defined, typically O(1) via a small HashMap.
    fn count(&self, _name: &str) -> usize {
        unreachable!("build context does not supply runtime counts")
    }

    /// Resolve a runtime single mutation for
    /// [`MutationSrc::Runtime`](super::MutationSrc::Runtime). `label`
    /// is the label that variant carries. Called once per materialized
    /// `Single`/`Instruction` carrying a `Runtime` mutation.
    ///
    /// Cost: consumer-defined. The implementation owns the mutation it
    /// returns (cloning is implicit in the return-by-value contract).
    fn mutation(&self, _label: &str) -> M {
        unreachable!("build context does not supply runtime mutations")
    }

    /// Resolve a runtime `Vec<M>` for
    /// [`MutationListSrc::Runtime`](super::MutationListSrc::Runtime).
    /// `label` is the free-form key the AST carries; the consumer
    /// disambiguates among multiple runtime-sourced macros by this
    /// label. Called once per matching `Macro` node during a build.
    /// Returning an empty `Vec` is a valid no-op choice; the built
    /// `Macro` then walks over its target without applying anything.
    ///
    /// Cost: consumer-defined. The returned `Vec` is moved into the
    /// mutator tree, so no double allocation.
    fn mutation_list(&self, _label: &str) -> Vec<M> {
        unreachable!("build context does not supply runtime mutation lists")
    }

    /// Resolve a [`MutationSrc::Template`](super::MutationSrc::Template)
    /// into the mutation for iteration `index` of the `Repeat` section
    /// named `section`. Called once per materialized node carrying a
    /// template, inside a `Repeat`.
    ///
    /// Cost: consumer-defined; on per-frame paths, build only what the
    /// template names.
    fn resolve(&self, section: &str, index: usize, template: &M::Template) -> M;
}

/// The context for an AST with no runtime sources at all — literal
/// channels, counts and mutation lists only. Every look-up is
/// `unreachable!()`, like the trait's defaults: handing it an AST that
/// does consult the context is a bug at the call site, surfaced at
/// once. Zero-sized; one value serves every instantiation.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoRuntime;

impl<E: TreeElement, M: Mutation<E>> BuildContext<E, M> for NoRuntime {
    fn resolve(&self, _section: &str, _index: usize, _template: &M::Template) -> M {
        unreachable!("build context does not resolve templates")
    }
}
