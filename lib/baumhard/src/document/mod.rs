// SPDX-License-Identifier: MPL-2.0

//! The document: a map as a baumhard `Tree<DocElement>`, the second
//! instantiation of [the engine](crate::engine) beside the glyph trees
//! (design §5).
//!
//! A map is a tree whose shape is its structure: the
//! [`Map`](crate::document::element::DocKind::Map) root holds the theme, the palettes,
//! the map's defaults and the root nodes; a node holds its sections,
//! its frame, its connector to its parent, its links and its child
//! nodes; text is always a run of [spans](crate::document::element::DocKind::Span)
//! under whatever draws it. There is no parent pointer, no
//! path-encoding id and no edge list — structure is stored once, as
//! nesting (D3).
//!
//! Every value is typed at rest ([`values`](crate::document::values)): a color is a
//! [`ColorRef`](crate::document::values::ColorRef), a font an
//! [`AppFont`](crate::font::fonts::AppFont), a shape a
//! [`Shape`](crate::document::values::Shape), each parsed once at load. Every style
//! field is optional, and [`resolve`](crate::document::resolve) computes what an element draws
//! with field by field — through its owners, its palette, the map's
//! defaults and [`BUILT_IN`](crate::document::defaults::BUILT_IN) — recording every
//! input it reads so the projection can track what depends on what.
//!
//! Every change a document can undergo is a [`mutation`](crate::document::mutation)
//! (V1-B3); the writer that applies them, with history, is V1-B4.
//! Nothing here reads or writes a file: the format is V1-B2.

/// Every numeric bound the loader and the mutations enforce.
pub mod bounds;
/// [`BUILT_IN`](defaults::BUILT_IN): the last tier of every
/// resolution, reproducing today's defaults.
pub mod defaults;
/// [`inputs_changed`](deps::inputs_changed): which resolution inputs a
/// change of one element touches.
pub mod deps;
/// [`DocKind`](element::DocKind), [`DocElement`](element::DocElement)
/// and the per-kind data, and the engine instantiation.
pub mod element;
/// [`NodeIdent`](ids::NodeIdent), [`Document`](ids::Document) and its
/// read API.
pub mod ids;
/// [`DocField`](inspect::DocField): what a predicate can test on an
/// element.
pub mod inspect;
/// The engine's `Interpolable` for the document: numbers, positions,
/// sizes and literal colors blend; everything else snaps.
pub mod interpolation;
/// The mutation vocabulary: every change a document can undergo.
pub mod mutation;
/// Per-field style resolution with read recording.
pub mod resolve;
/// The shape rules: which kinds may hold which.
pub mod shape;
/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8), plus
/// `DocumentBuilder` (`tests::test_support`, crate-private), which
/// builds documents for them. `#[allow(clippy::unwrap_used)]` because
/// this is a tests tree (§T2.2): it carries no `cfg(test)` gate, so the
/// crate-root lint sees it as shipped code; `util::unwrap_posture`
/// excludes these trees by path.
#[allow(clippy::unwrap_used)]
pub mod tests;
/// Typed values: colors, sizes, shapes, anchors, modes.
pub mod values;
