// SPDX-License-Identifier: MPL-2.0

//! Spatial bookkeeping used by the gfx tree: a grid-bucket region
//! index for O(1) screen-region lookup, the grid-parameter helper
//! that picks non-prime subdivisions, and the hit-test rectangle
//! bag carried by each `GlyphModel`.

/// `HitBox` — hit-test bounding-rectangle bag carried on every
/// `GlyphModel` (one rect per visual line for wrapped nodes).
pub mod hitbox;
/// `RegionIndexer` — grid-bucket spatial index delivering O(1)
/// per-bucket lookup of "which elements occupy this screen region".
pub mod region_indexer;
/// `RegionParams` — picks non-prime grid subdivisions of a pixel
/// resolution for `RegionIndexer`; re-exports the indexer.
pub mod regions;
