// SPDX-License-Identifier: MPL-2.0

//! Well-known context tags for
//! [`CustomMutation::contexts`](crate::mindmap::custom_mutation::CustomMutation::contexts).
//!
//! Contexts describe *where* and *on what* a mutation is meant to be
//! used. Dotted namespaces group related tags (`map.node`, `map.tree`);
//! a mutation may carry several. Format follows `format/enums.md`:
//! snake_case strings, with unknown tags round-tripped but matching
//! no well-known predicate. Plugins use the
//! `plugin.<name>.<kind>` sub-namespace for third-party tags.

/// Internal application use only. Not exposed to the user; invoked
/// programmatically by app code.
pub const INTERNAL: &str = "internal";

/// Mutates a mindmap. Root namespace for any `map.*` context.
pub const MAP: &str = "map";

/// Mutates the content of a single node (text, style, color, regions).
pub const MAP_NODE: &str = "map.node";

/// Mutates tree structure / layout descending from a node (positions,
/// children arrangement, reparenting).
pub const MAP_TREE: &str = "map.tree";
