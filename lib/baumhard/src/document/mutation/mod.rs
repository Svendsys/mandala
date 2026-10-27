// SPDX-License-Identifier: MPL-2.0

//! The document's mutation vocabulary: every change a map can undergo
//! (V1-B3, #237).
//!
//! Every later path that changes a map — actions, the console, IPC,
//! drags, text editing, edits, presentations, verbs — builds a mutator
//! tree from this vocabulary and nothing else:
//!
//! - [`ops`](crate::document::mutation::ops): [`DocMutation`](crate::document::mutation::ops::DocMutation), one payload per kind of
//!   element, each op writing exactly the field it names, the relative
//!   ones clamped;
//! - [`inherited`](crate::document::mutation::inherited): [`DocInherited`](crate::document::mutation::inherited::DocInherited), what a
//!   walk carries down so that growing an unsized text grows the size it
//!   displays, and [`inherited_seed`](crate::document::mutation::inherited::inherited_seed), the
//!   state an anchored walk starts from;
//! - [`validate`](mod@crate::document::mutation::validate): what is checked before a walk
//!   ([`validate()`](crate::document::mutation::validate::validate),
//!   [`validate_for_presentation`](crate::document::mutation::validate::validate_for_presentation))
//!   and after the steps of a request
//!   ([`validate_applied`](crate::document::mutation::validate::validate_applied));
//! - [`step`](crate::document::mutation::step): [`DocStep`](crate::document::mutation::step::DocStep), one walk of a request, built
//!   from the document as the earlier steps left it;
//! - [`compose`](crate::document::mutation::compose): the composite builders, the one sanctioned shape of
//!   each change that takes more than one field write.
//!
//! The vocabulary is built by code and never read from a file in V1 —
//! authored edits are V1-D1's — so nothing here parses a string. Nothing
//! here writes a document either: running the steps of a request is the
//! writer's, V1-B4.

/// The composite builders.
pub mod compose;
/// [`MutationError`](error::MutationError).
pub mod error;
/// [`DocInherited`](inherited::DocInherited) and
/// [`inherited_seed`](inherited::inherited_seed).
pub mod inherited;
/// Whether many elements are in the tree, at no per-element depth
/// cost.
pub(crate) mod membership;
/// [`DocMutation`](ops::DocMutation) and its ops.
pub mod ops;
/// [`DocStep`](step::DocStep) and [`DocWalk`](step::DocWalk).
pub mod step;
/// Validation before a walk and after a request.
pub mod validate;
