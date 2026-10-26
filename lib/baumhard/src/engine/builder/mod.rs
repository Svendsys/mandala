// SPDX-License-Identifier: MPL-2.0

//! Declarative mutator-tree DSL.
//!
//! A [`MutatorNode`](crate::engine::builder::MutatorNode) is a serde-friendly AST mirroring the four
//! [`Mutator`](crate::engine::mutator::Mutator) variants plus a
//! `Repeat` wrapper for "N consecutive children with the same
//! template" (24 hue cells, runtime-count rows, descendants-of).
//! [`build`](crate::engine::builder::build()) walks it recursively and consults a [`BuildContext`](crate::engine::builder::BuildContext) for
//! runtime values — counts, mutation lists, and the resolution of a
//! payload's templates per iteration — returning a [`BuildError`](crate::engine::builder::BuildError) for
//! an AST no mutator tree can express. Custom mutations carry their
//! payload as a `MutatorNode`; the color picker, console overlay, and
//! user widgets all reach for the same builder.

mod ast;
mod build;
mod context;

pub use ast::{ChannelSrc, CountSrc, InstructionSpec, MutationListSrc, MutationSrc, MutatorNode};
pub use build::{build, iter_section_channels, BuildError};
pub use context::{BuildContext, NoRuntime};
