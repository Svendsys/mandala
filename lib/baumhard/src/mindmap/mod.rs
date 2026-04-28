// SPDX-License-Identifier: MPL-2.0

//! Mindmap data model, loader/saver, and the builders that project a
//! `MindMap` into the Baumhard render tree and flat scene. Borders,
//! connections, portal labels, and edge handles all descend from the
//! types declared under `model` and materialise through
//! `tree_builder` / `scene_builder`.

pub mod animation;
pub mod model;
pub mod loader;
pub mod border;
pub mod border_pattern;
pub mod connection;
pub mod custom_mutation;
pub mod portal_geometry;
pub mod scene_builder;
pub mod scene_cache;
pub mod tree_builder;

#[cfg(test)]
pub(crate) mod test_helpers;

/// Cyan selection highlight applied at scene / tree emission time
/// (selected edges, edge handles, portal markers, portal mutator
/// output). The app crate's `document::types::HIGHLIGHT_COLOR` is
/// the approximately-matching float-RGBA form used by the selection
/// machinery upstream.
pub(crate) const SELECTION_HIGHLIGHT_HEX: &str = "#00E5FF";
