// SPDX-License-Identifier: MPL-2.0

//! Per-node background / hit-test shapes.
//!
//! [`crate::gfx_structs::shape::NodeShape`] is the single source of
//! truth for "what shape does this node occupy?". Both the renderer
//! (SDF fragment path) and the BVH hit test (point-in-shape check)
//! consult the same enum, so adding a new shape never drifts between
//! visuals and input.
//!
//! Extending the set is deliberately local:
//!
//! 1. Add a variant to the enum below.
//! 2. Give it its canonical `NodeStyle.shape` spelling(s) in
//!    [`crate::gfx_structs::shape::NodeShape::style_spellings`], and
//!    add that spelling to
//!    [`crate::gfx_structs::shape::KNOWN_SHAPES`] if it is not
//!    already listed. The `match` in `style_spellings` is
//!    exhaustive, so step 1 does not compile until this one is
//!    done — a new variant cannot silently stay on the
//!    quiet-fallback path.
//! 3. Add a `SHAPE_*` constant + a `case` arm to the rect pipeline's
//!    fragment shader (`src/application/renderer/mod.rs`,
//!    `RECT_SHADER_WGSL`).
//! 4. Add a branch in
//!    [`crate::gfx_structs::shape::NodeShape::contains_local`] and
//!    [`crate::gfx_structs::shape::NodeShape::intersects_local_aabb`].
//!
//! No new structs, no new mutation surfaces, no new mesh builders.
//!
//! The format's shape vocabulary is deliberately wider than the set
//! this module can draw:
//! [`crate::gfx_structs::shape::KNOWN_SHAPES`] lists every canonical
//! spelling `format/enums.md` publishes and `maptool verify` accepts,
//! while `NodeShape` names only the ones with a shader case behind
//! them. [`crate::gfx_structs::shape::ShapeSpelling`] is the pure
//! classifier that tells those two populations apart, so a canonical
//! spelling awaiting its shader case degrades quietly to a rectangle
//! and only a genuine typo is reported to the author.

use glam::Vec2;
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

/// The background / hit shape of a node. Stored on
/// [`crate::gfx_structs::area::GlyphArea`] next to `background_color`
/// and read by both the renderer and the BVH hit test.
///
/// The variant is copied out of the area in the hot paths, so it is
/// intentionally `Copy` and allocation-free.
///
/// # Costs
/// O(1) to copy, hash, compare. No heap allocation.
///
/// `EnumIter` is derived rather than hand-maintained so that
/// [`ShapeSpelling::classify`] enumerates exactly the variants that
/// exist — the same reason `GlyphAreaFieldType` derives it. A
/// hand-written `ALL` array would let a new variant be added without
/// ever becoming parseable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default, EnumIter)]
pub enum NodeShape {
    /// Fills the bounding box exactly — the legacy behaviour and the
    /// default for any node that doesn't opt in to a different shape.
    #[default]
    Rectangle,
    /// Axis-aligned ellipse inscribed in the bounding box. A perfect
    /// circle is expressed as an `Ellipse` with `width == height`;
    /// the same variant handles stretched / "conical" cases where
    /// the box is wider than it is tall (or vice versa) without any
    /// extra parameters.
    Ellipse,
}

/// Shader-side id for [`NodeShape::Rectangle`]. Must match the
/// `SHAPE_RECT` constant in the rect pipeline's WGSL fragment shader.
pub const SHAPE_ID_RECTANGLE: u32 = 0;
/// Shader-side id for [`NodeShape::Ellipse`]. Must match the
/// `SHAPE_ELLIPSE` constant in the rect pipeline's WGSL fragment
/// shader.
pub const SHAPE_ID_ELLIPSE: u32 = 1;

/// Canonical named-enum spellings for `NodeStyle.shape`, as used by
/// `format/enums.md` and by `maptool verify`. The runtime accepts
/// these case-insensitively (and treats `"circle"` as an alias for
/// `"ellipse"`); verify normalizes to lowercase before matching.
///
/// This is the **only** list of canonical spellings in the tree, and
/// deliberately so: [`ShapeSpelling::classify`] consults it rather
/// than repeating a set of literals, so adding a spelling here makes
/// it a documented, non-warning value everywhere at once —
/// `maptool verify` accepts it and the runtime stops calling it
/// unknown — without anyone editing the parser. Entries are written
/// lowercase; `test_shape_known_shapes_are_lowercase` pins that,
/// because the case-insensitive compare here and verify's
/// lowercase normalization only agree while it holds.
///
/// Membership here is a claim about the *format*, not about the
/// renderer: most of these have no [`NodeShape`] variant yet and
/// draw as a rectangle until one lands.
pub const KNOWN_SHAPES: &[&str] = &[
    "rectangle",
    "rounded_rectangle",
    "ellipse",
    "circle",
    "diamond",
    "parallelogram",
    "hexagon",
];

/// What a format-level `NodeStyle.shape` string turns out to be,
/// decided without side effects so the decision and the reporting of
/// it can be tested apart.
///
/// The distinction that matters is between a spelling the *format*
/// knows and a spelling nobody knows. [`KNOWN_SHAPES`] is wider than
/// the [`NodeShape`] variant set — `"hexagon"` is published in
/// `format/enums.md`, emitted by `maptool convert --legacy` and
/// accepted by `maptool verify`, yet has no shader case — so
/// collapsing it to `Rectangle` is the intended behavior and there is
/// nothing for an author to fix. Telling them otherwise is noise, and
/// on `maps/testament.mindmap.json` it was 242 lines of it per load
/// (issue #118). A spelling outside `KNOWN_SHAPES` is a different
/// thing entirely: a typo, or a value written by a newer build, and
/// worth a `log::warn!`.
///
/// [`NodeShape::from_style_string`] is the caller that maps these to
/// a log level; anything else that needs to know *why* a shape
/// resolved the way it did — a linter, a script API, an editor
/// surfacing "this shape isn't drawn yet" — reads the classification
/// directly instead of scraping the log.
///
/// # Costs
/// O(1) to copy, hash, compare. Borrows nothing, allocates nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShapeSpelling {
    /// The empty string — the node names no shape at all, which is
    /// not an error and never was. Resolves to the
    /// [`NodeShape::default`] rectangle silently.
    Unspecified,
    /// A canonical spelling that has a [`NodeShape`] variant behind
    /// it, so the renderer draws what the author asked for.
    Rendered(NodeShape),
    /// Listed in [`KNOWN_SHAPES`], but no [`NodeShape`] variant
    /// exists for it yet. Documented, tool-emitted, round-trips
    /// intact — and draws as a rectangle until a shader case lands.
    /// Not an author error.
    KnownNotYetRendered,
    /// Not in [`KNOWN_SHAPES`] at all: a typo, or a value from a
    /// build that knows more shapes than this one. The only case
    /// that warrants a warning.
    Unrecognized,
}

impl ShapeSpelling {
    /// Classify a `NodeStyle.shape` string. Pure — no logging, no
    /// allocation, no I/O.
    ///
    /// Matching is ASCII-case-insensitive throughout, against the
    /// per-variant spellings from [`NodeShape::style_spellings`]
    /// first and [`KNOWN_SHAPES`] second. The empty string is
    /// [`ShapeSpelling::Unspecified`] rather than unrecognized: it
    /// means "unset", and every `NodeStyle` that omits the field
    /// would otherwise be an error.
    ///
    /// # Costs
    /// At most one `eq_ignore_ascii_case` per canonical spelling —
    /// bounded by `KNOWN_SHAPES.len()` plus the (subset) spellings
    /// of the drawable variants, each O(`s.len()`). No allocation.
    /// Runs once per node per scene rebuild, not per frame.
    pub fn classify(s: &str) -> Self {
        if s.is_empty() {
            return ShapeSpelling::Unspecified;
        }
        for shape in NodeShape::iter() {
            if shape
                .style_spellings()
                .iter()
                .any(|spelling| s.eq_ignore_ascii_case(spelling))
            {
                return ShapeSpelling::Rendered(shape);
            }
        }
        if KNOWN_SHAPES.iter().any(|known| s.eq_ignore_ascii_case(known)) {
            ShapeSpelling::KnownNotYetRendered
        } else {
            ShapeSpelling::Unrecognized
        }
    }

    /// The [`NodeShape`] the renderer and the hit test should use for
    /// this classification. Every non-[`ShapeSpelling::Rendered`]
    /// case degrades to [`NodeShape::Rectangle`] — the bounding box
    /// is the one silhouette that is always correct to fall back to,
    /// because it is what the shape's AABB already occupies.
    ///
    /// # Costs
    /// O(1), branch-only. No allocation. Deliberately not
    /// `#[inline]`: §B7 wants a benchmark that resolves the effect
    /// before the attribute goes on, and this is not a per-frame
    /// call.
    pub const fn resolve(self) -> NodeShape {
        match self {
            ShapeSpelling::Rendered(shape) => shape,
            ShapeSpelling::Unspecified
            | ShapeSpelling::KnownNotYetRendered
            | ShapeSpelling::Unrecognized => NodeShape::Rectangle,
        }
    }
}

impl NodeShape {
    /// Stable id fed to the fragment shader. Must stay in lock-step
    /// with the `SHAPE_*` constants in
    /// `src/application/renderer/mod.rs` — adding a variant without
    /// adding its shader case would render the new shape as a
    /// rectangle. O(1).
    #[inline]
    pub const fn shader_id(self) -> u32 {
        match self {
            NodeShape::Rectangle => SHAPE_ID_RECTANGLE,
            NodeShape::Ellipse => SHAPE_ID_ELLIPSE,
        }
    }

    /// The canonical `NodeStyle.shape` spellings that resolve to this
    /// variant, in the order [`ShapeSpelling::classify`] tries them.
    /// Every returned spelling must also appear in [`KNOWN_SHAPES`]
    /// — `test_shape_variant_spellings_are_all_known` pins that, so a
    /// variant cannot claim a spelling `maptool verify` would reject.
    ///
    /// The `match` is exhaustive on purpose. It is the guarantee that
    /// adding a `NodeShape` variant is a **build error** rather than
    /// a silent no-op: a new variant with no spelling here would
    /// otherwise be unreachable from the format, and its canonical
    /// name would keep resolving to a quiet `Rectangle` forever.
    ///
    /// `"circle"` is the one alias: a `width == height` ellipse *is*
    /// a circle, and it is the spelling authors reach for first. It
    /// costs nothing to accept and round-trips intact, because
    /// `NodeStyle.shape` stays a free-form `String` at the format
    /// layer and is never written back from here.
    ///
    /// # Costs
    /// O(1) — returns a `'static` slice. No allocation.
    pub const fn style_spellings(self) -> &'static [&'static str] {
        match self {
            NodeShape::Rectangle => &["rectangle"],
            NodeShape::Ellipse => &["ellipse", "circle"],
        }
    }

    /// Parse the format-level `NodeStyle.shape` string into the shape
    /// the renderer and hit test should use, reporting only the case
    /// an author can act on.
    ///
    /// The decision is [`ShapeSpelling::classify`]'s and is pure;
    /// this function is the one place that turns it into a log line,
    /// which is what keeps the classifier testable without a logger:
    ///
    /// - a canonical spelling with a variant renders as asked;
    /// - the empty string means "unset" and is silent;
    /// - a [`KNOWN_SHAPES`] spelling with no variant yet
    ///   (`"hexagon"`, `"diamond"`, `"parallelogram"`,
    ///   `"rounded_rectangle"`) falls back to
    ///   [`NodeShape::Rectangle`] at `log::trace!`. It is documented,
    ///   tool-emitted and correct; warning about it told authors a
    ///   valid value was unknown, 242 times per load of the demo map
    ///   (issue #118);
    /// - anything else keeps the `log::warn!`, because there it is
    ///   accurate.
    ///
    /// Unknown values stay on disk untouched either way, so a
    /// round-trip through `maptool convert` doesn't lose them.
    ///
    /// # Costs
    /// [`ShapeSpelling::classify`]'s compares plus one branch. No
    /// allocation on any path; the `trace!` arm compiles out
    /// entirely in release (`release_max_level_warn`).
    pub fn from_style_string(s: &str) -> Self {
        let spelling = ShapeSpelling::classify(s);
        match spelling {
            ShapeSpelling::Unspecified | ShapeSpelling::Rendered(_) => {}
            ShapeSpelling::KnownNotYetRendered => {
                log::trace!(
                    "shape: {s:?} is a canonical shape with no renderer yet, \
                     drawing it as Rectangle"
                );
            }
            ShapeSpelling::Unrecognized => {
                log::warn!(
                    "shape: unknown shape {s:?}, \
                     falling back to Rectangle"
                );
            }
        }
        spelling.resolve()
    }

    /// Point-in-shape test in the node's **local** coordinate space,
    /// where the bounding box runs from `(0, 0)` to `bounds`.
    ///
    /// Callers pre-translate into that frame, and **which two values
    /// they subtract matters**: derive `local` and `bounds` from the
    /// same `min` / `max` pair, as
    /// `local = point - min`, `bounds = max - min`. Recomputing
    /// either side independently (say `local = point - position`
    /// against a separately-stored extent) makes the boundary
    /// compare inexact — f32 addition is not associative, so
    /// `(position + extent) - position` can exceed `extent` by an
    /// ULP and a point exactly on the far edge then reports `false`.
    /// `bvh_find` in
    /// [`tree_walker`](crate::gfx_structs::tree_walker) is the
    /// reference caller.
    ///
    /// A degenerate `bounds` (either dimension `<= 0`) always
    /// reports `false`, matching how the BVH skips zero-size areas
    /// at the AABB stage.
    ///
    /// O(1). No allocation.
    #[inline]
    pub fn contains_local(self, local: Vec2, bounds: Vec2) -> bool {
        if bounds.x <= 0.0 || bounds.y <= 0.0 {
            return false;
        }
        match self {
            NodeShape::Rectangle => {
                local.x >= 0.0 && local.x <= bounds.x && local.y >= 0.0 && local.y <= bounds.y
            }
            NodeShape::Ellipse => {
                // Normalised coordinates in [-1, 1] relative to the
                // ellipse centre. A perfect circle is `bounds.x ==
                // bounds.y`; a stretched conic is anything else.
                let rx = bounds.x * 0.5;
                let ry = bounds.y * 0.5;
                let nx = (local.x - rx) / rx;
                let ny = (local.y - ry) / ry;
                nx * nx + ny * ny <= 1.0
            }
        }
    }

    /// Does the shape's filled area overlap the AABB
    /// `[min, max]` (in local coordinates, same frame as
    /// [`Self::contains_local`])? Conservative — false positives
    /// are tolerated, false negatives are not. Used by rect-select.
    ///
    /// For the ellipse variant this clamps the AABB to the
    /// ellipse's bounding box, then checks whether the closest
    /// clamped point sits inside the ellipse. That's conservative
    /// in the "selection rect fully inside the ellipse" corner
    /// (the closest-point test returns true when any corner of the
    /// rect is inside the ellipse, *or* when the ellipse-centre is
    /// inside the rect) — which is what we want for a lasso.
    ///
    /// Degenerate bounds (zero or negative extent on either axis)
    /// report `false` even if the AABBs would overlap numerically.
    /// This matches [`Self::contains_local`] and the BVH's
    /// `bounds.x > 0.0 && bounds.y > 0.0` guard in `bvh_descend`:
    /// a zero-size node renders nothing, so selecting nothing for
    /// it is the internally consistent answer. Small behaviour
    /// change from the pre-shape `rect_select` (which would have
    /// matched a point-sized node under the cursor) — considered
    /// an improvement and noted here so a future test author can
    /// find the rationale.
    ///
    /// O(1). No allocation.
    #[inline]
    pub fn intersects_local_aabb(self, min: Vec2, max: Vec2, bounds: Vec2) -> bool {
        if bounds.x <= 0.0 || bounds.y <= 0.0 {
            return false;
        }
        // First, AABB–AABB overlap. Bails on any shape whose bounds
        // don't touch the selection rectangle at all.
        if max.x < 0.0 || min.x > bounds.x || max.y < 0.0 || min.y > bounds.y {
            return false;
        }
        match self {
            NodeShape::Rectangle => true,
            NodeShape::Ellipse => {
                let rx = bounds.x * 0.5;
                let ry = bounds.y * 0.5;
                let cx = rx;
                let cy = ry;
                // Closest point on the AABB to the ellipse centre.
                let clamped_x = cx.clamp(min.x, max.x);
                let clamped_y = cy.clamp(min.y, max.y);
                let nx = (clamped_x - cx) / rx;
                let ny = (clamped_y - cy) / ry;
                nx * nx + ny * ny <= 1.0
            }
        }
    }
}

// Tests live out-of-line at
// `lib/baumhard/src/gfx_structs/tests/shape_tests.rs` so the
// criterion bench harness at `lib/baumhard/benches/test_bench.rs`
// can reuse each `do_*()` body as a micro-benchmark — see
// TEST_CONVENTIONS.md §T2.2.
