// SPDX-License-Identifier: MPL-2.0

//! Spatial bookkeeping used by the gfx tree: a grid-bucket region
//! index for O(1) screen-region lookup, the grid-parameter helper
//! that picks non-prime subdivisions, and the hit-test rectangle
//! bag carried by each `GlyphModel`.

pub mod region_indexer;
pub mod regions;
pub mod hitbox;