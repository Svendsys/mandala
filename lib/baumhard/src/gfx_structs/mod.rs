// SPDX-License-Identifier: MPL-2.0

//! Render-side graph structures — the `GfxElement` tree, its
//! mutators, the predicate language that steers walker traversal,
//! and the `Scene` that composes multiple trees into one frame.
//! Mindmap nodes, borders, connections, and overlay UI all compile
//! down to `GfxElement`s; mutations flow through the walker to
//! reshape them without rebuilding from scratch.

pub mod tree;
pub mod predicate;
pub mod model;
pub mod area_fields;
pub mod shape;
pub mod area_mutators;
pub mod area;
pub mod tree_walker;
pub mod element;
pub mod mutator;
pub mod tests;
pub mod scene;
pub mod util;
pub mod camera;
pub mod zoom_visibility;