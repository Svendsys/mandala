// SPDX-License-Identifier: MPL-2.0

//! Render-side graph structures — the glyph instantiation of the
//! [mutation engine](crate::engine): the `GfxElement` element type, its
//! `GfxMutation` payload, how it answers predicates, its builder
//! template, the spatial queries only glyph trees answer, and the
//! `Scene` that composes multiple trees into one frame. Mindmap nodes,
//! borders, connections, and overlay UI all compile down to
//! `GfxElement`s; mutations flow through the engine's walker to
//! reshape them without rebuilding from scratch.

/// `GlyphArea` — text-region element variant: text, scale,
/// position, color-font regions, hit-box.
pub mod area;
/// Field-level delta types for `GlyphArea` — the mutation
/// vocabulary that targets a single facet.
pub mod area_fields;
/// `Applicable` impls for `GlyphArea` commands and deltas,
/// dispatched by the tree walker.
pub mod area_mutators;
/// 2D pan/zoom camera — canvas ↔ screen-space projection.
pub mod camera;
/// `Delta` — the field-set container and `DeltaField` vocabulary
/// trait shared by the area-side and model-side mutation surfaces.
pub mod delta;
/// `GfxElement` — tree-node variant (`GlyphArea` / `GlyphModel` /
/// `GlyphRun` / `Fill` / `Void`) plus its field enum, flags, and AABB
/// caching.
pub mod element;
/// `Fill` — the node background element: a rectangle, a shape, a
/// color and a soft edge fade.
pub mod fill;
/// How a `GfxElement` answers a predicate's field tests — the glyph
/// `Inspect` implementation over `GfxElementField`.
pub mod inspect;
/// `GlyphModel` wrapping a `GlyphMatrix` of `GlyphLine`s of
/// `GlyphComponent`s.
pub mod model;
/// `GfxMutation` — the glyph payload — its `GfxMutator` node alias,
/// and the glyph event side channel.
pub mod mutator;
/// `GlyphRun` — the element for one whole shaped TextFlow, box- or
/// path-placed, with per-glyph presentation overrides.
pub mod run;
/// `Scene` — composes multiple `Tree`s at per-layer offsets into a
/// single rendered frame.
pub mod scene;
/// Per-node background / hit-test shape enum shared by the
/// renderer (SDF) and the BVH hit test.
pub mod shape;
/// `CellField` — the glyph payload's builder template — and the
/// helpers that resolve one into an `AreaDelta`.
pub mod template;
/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8).
/// `#[allow(clippy::unwrap_used)]` because this is a tests tree
/// (§T2.2): it carries no `cfg(test)` gate — the criterion harness
/// imports its bodies — so the crate-root `warn(clippy::unwrap_used)`
/// sees it as shipped code. `unwrap()` in a test is the correct
/// spelling; the gate that matters for shipped code is
/// `util::unwrap_posture`, which excludes these trees by path.
#[allow(clippy::unwrap_used)]
pub mod tests;
/// `GfxTree` / `GfxMutatorTree`, the glyph tree's cache, and the
/// spatial queries (AABB memo, BVH hit-test) only glyph trees answer.
pub mod tree;
/// Spatial bookkeeping: grid-bucket region index, grid parameters,
/// per-model hit-box bags.
pub mod util;
/// `ZoomVisibility` — per-`GlyphArea` lower/upper bound on
/// `camera.zoom` controlling whether the element renders. Orthogonal
/// to the connection / portal font-size clamps that reshape *size*
/// with zoom.
pub mod zoom_visibility;
