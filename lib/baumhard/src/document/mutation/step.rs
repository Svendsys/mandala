// SPDX-License-Identifier: MPL-2.0

//! [`DocStep`](crate::document::mutation::step::DocStep): one engine
//! call of an edit, built from the document as the earlier steps of the
//! same request left it (ruling R8 on #238).
//!
//! One walk cannot express every edit. It cannot split a span twice and
//! then style the middle part, because the middle part does not exist
//! until the splits have run; nor merge a run of three equal spans,
//! because the first merge absorbs the second span, which the walk had
//! as the next target. So an edit is an ordered list of steps, each of
//! which builds its [`DocWalk`](crate::document::mutation::step::DocWalk)
//! — a mutator tree and its anchors — from the document as it stands
//! when its turn comes. The writer (V1-B4) runs every step of a request
//! on **one** journal builder, so the request is one journal, one history
//! entry, and all or nothing: a step that fails, or whose walk fails,
//! abandons the builder and the document is as it was.
//!
//! **A step sees an unvalidated document.** The checks that only the
//! walk can decide
//! ([`validate_applied`](crate::document::mutation::validate::validate_applied))
//! run once, after a request's last step, because a request's
//! intermediate states need not be valid: a line link switched to a
//! portal holds one portal end for a step before it holds two. So a
//! step's closure may be handed a document that breaks the shape rules
//! or sets a size its place does not take, and builds its walk from
//! what is there without assuming otherwise (ruling R14.7 on #237).
//!
//! A step that finds nothing to do builds `None`, which is how a bounded
//! repetition — the merges that normalize a section's spans — ends
//! early: the composite builder emits as many as the section could ever
//! need, and the ones past the last merge build nothing.

use crate::document::ids::Document;
use crate::document::mutation::error::MutationError;
use crate::document::mutation::ops::DocMutatorTree;
use crate::engine::anchor::Anchors;
use std::fmt;

/// A mutator tree and where it applies: one walk.
#[derive(Debug)]
pub struct DocWalk {
    /// The mutator tree.
    pub mutator: DocMutatorTree,
    /// Where it applies. Its root applies at each anchor of the root's
    /// kind (ruling R1 on #232).
    pub anchors: Anchors,
}

impl DocWalk {
    /// `mutator` at `anchors`. No allocation.
    pub fn new(mutator: DocMutatorTree, anchors: Anchors) -> Self {
        DocWalk { mutator, anchors }
    }
}

/// The function a [`DocStep`] runs: given the document as the earlier
/// steps left it, the walk to run next, or `None` when there is nothing
/// to do.
type Build = dyn FnOnce(&Document) -> Result<Option<DocWalk>, MutationError> + Send;

/// One step of an edit. See the module docs.
pub struct DocStep(Box<Build>);

impl DocStep {
    /// A step that builds its walk with `build` when its turn comes. One
    /// allocation, for the closure.
    pub fn new(
        build: impl FnOnce(&Document) -> Result<Option<DocWalk>, MutationError> + Send + 'static,
    ) -> Self {
        DocStep(Box::new(build))
    }

    /// A step whose walk is known in advance: one that needs nothing an
    /// earlier step creates.
    pub fn fixed(walk: DocWalk) -> Self {
        DocStep::new(move |_| Ok(Some(walk)))
    }

    /// Build the walk from `doc` as it stands: `None` when the step has
    /// nothing to do. Consumes the step.
    pub fn build(self, doc: &Document) -> Result<Option<DocWalk>, MutationError> {
        (self.0)(doc)
    }
}

impl fmt::Debug for DocStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DocStep(..)")
    }
}
