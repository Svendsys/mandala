// SPDX-License-Identifier: MPL-2.0

//! Tree-builder tests sharded by subject so each file stays small
//! and greppable. Shared fixtures (`test_map_path`, `synthetic_node`,
//! `synthetic_map`, `mk_chain_map`, `mk_star_map`, `synthetic_portal`,
//! `glyph_area_of`) live in [`fixtures`].
//!
//! - [`node_basic`] — `build_mindmap_tree` structure, root-node count,
//!   glyph_area properties, color-region projection, parent/child
//!   hierarchy, unique IDs, element type invariants.
//! - [`node_scale`] — large-N regression guards (1000-node chain,
//!   500-wide fan-out, deep-chain stack safety).
//! - [`node_background`] — `GlyphArea::background_color` resolution
//!   across hex, empty, transparent, theme-var, malformed, 3-digit.
//! - [`border`] — border tree: void-per-framed, filters, drag
//!   offset, theme var, channel stability, mutator round-trip.
//! - [`portal`] — portal tree: marker pairs, fold filter,
//!   selection highlight, ascending channels, mutator round-trip,
//!   identity sequence.
//! - [`connection`] — connection tree (edges + labels): per-edge
//!   voids, cap filters, identity drift, mutator round-trips.
//! - [`edge_handle`] — edge-handle tree: channel ordering, mutator
//!   round-trip, identity shift on midpoint→CP transitions.

mod fixtures;

mod border;
mod connection;
mod edge_handle;
mod node_background;
mod node_basic;
mod node_scale;
mod portal;
