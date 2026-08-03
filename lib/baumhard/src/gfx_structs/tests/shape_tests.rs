// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::gfx_structs::shape`] — the `NodeShape` enum
//! and its point-in-shape / shape-vs-AABB primitives.
//!
//! Follows the `do_*()` / `test_*()` split from
//! [`TEST_CONVENTIONS.md §T2.2`] so the criterion bench harness at
//! `lib/baumhard/benches/test_bench.rs` can reuse each body as a
//! micro-benchmark. The shape math sits on the BVH hot path
//! (`bvh_descend`), so the rect-pipeline SDF in the renderer and
//! the lasso / editor hit-tests all pay for it every interaction
//! frame — benching is §B8-mandatory, not optional.

use glam::Vec2;
use strum::IntoEnumIterator;

use crate::gfx_structs::shape::{
    NodeShape, ShapeSpelling, KNOWN_SHAPES, SHAPE_ID_ELLIPSE, SHAPE_ID_RECTANGLE,
};

/// Parse every documented shape spelling, plus the `"circle"`
/// alias and the empty-string fallback. Pins the author-facing
/// vocabulary: changing a spelling here would silently change
/// which JSON maps still render as the intended shape.
#[test]
pub fn test_shape_from_style_string_known_names() {
    do_shape_from_style_string_known_names();
}

pub fn do_shape_from_style_string_known_names() {
    assert_eq!(NodeShape::from_style_string("rectangle"), NodeShape::Rectangle);
    assert_eq!(NodeShape::from_style_string("Rectangle"), NodeShape::Rectangle);
    assert_eq!(NodeShape::from_style_string("ellipse"), NodeShape::Ellipse);
    assert_eq!(NodeShape::from_style_string("ELLIPSE"), NodeShape::Ellipse);
    // "circle" is accepted as a convenience alias for the
    // Ellipse variant — a `width == height` ellipse *is* a
    // circle, and authors will often type this.
    assert_eq!(NodeShape::from_style_string("circle"), NodeShape::Ellipse);
}

/// An empty or unknown string falls back to the default
/// `Rectangle` variant. Mirrors how `tree_builder/node.rs` treats
/// malformed background hex: survive a typo rather than crash the
/// render. Which of these is *reported* is
/// `test_shape_classification_partitions_by_warning`'s subject —
/// the resolved shape is the same either way.
#[test]
pub fn test_shape_from_style_string_empty_and_unknown_fall_back_to_rectangle() {
    do_shape_from_style_string_empty_and_unknown_fall_back_to_rectangle();
}

pub fn do_shape_from_style_string_empty_and_unknown_fall_back_to_rectangle() {
    assert_eq!(NodeShape::from_style_string(""), NodeShape::Rectangle);
    assert_eq!(NodeShape::from_style_string("diamond"), NodeShape::Rectangle);
    assert_eq!(NodeShape::from_style_string("zigzag"), NodeShape::Rectangle);
}

/// Does this classification make `from_style_string` emit a
/// `log::warn!`? The one question issue #118 is about, asked of the
/// classifier rather than of a log sink — there is no logger to
/// install here, and asserting on the returned value is a stronger
/// test than scraping records anyway.
fn warns(spelling: ShapeSpelling) -> bool {
    match spelling {
        ShapeSpelling::Unrecognized => true,
        // Spelled out rather than `_ =>` so a fourth non-warning
        // classification cannot join the quiet set unnoticed.
        ShapeSpelling::Unspecified | ShapeSpelling::Rendered(_) | ShapeSpelling::KnownNotYetRendered => false,
    }
}

/// **Issue #118.** Every canonical spelling — iterated from
/// `KNOWN_SHAPES`, never re-spelled here — is a shape the format
/// publishes, `maptool convert --legacy` emits and `maptool verify`
/// accepts. None of them may be reported as unknown, whether or not
/// a `NodeShape` variant draws it yet. `"hexagon"` is the one that
/// produced 242 warnings per load of `maps/testament.mindmap.json`.
///
/// Iterating is the point: a spelling added to `KNOWN_SHAPES`
/// tomorrow is covered by this test the moment it lands, with
/// nobody editing the parser or this file.
#[test]
pub fn test_shape_every_known_spelling_is_non_warning() {
    do_shape_every_known_spelling_is_non_warning();
}

pub fn do_shape_every_known_spelling_is_non_warning() {
    assert!(
        !KNOWN_SHAPES.is_empty(),
        "KNOWN_SHAPES is empty — this test would pass vacuously"
    );
    for known in KNOWN_SHAPES {
        let spelling = ShapeSpelling::classify(known);
        assert!(
            !warns(spelling),
            "canonical shape {known:?} classified as {spelling:?}, which warns"
        );
        assert_ne!(
            spelling,
            ShapeSpelling::Unspecified,
            "canonical shape {known:?} must not classify as unset"
        );
    }
}

/// The classification of every canonical spelling, derived from the
/// two sources of truth rather than restated: a spelling one of the
/// `NodeShape` variants claims is `Rendered`, and every other
/// `KNOWN_SHAPES` entry is `KnownNotYetRendered`. This is the pin
/// that fails when a `KNOWN_SHAPES` entry is added that the
/// classifier does not account for, and equally when a spelling
/// silently changes sides.
#[test]
pub fn test_shape_classification_partitions_by_warning() {
    do_shape_classification_partitions_by_warning();
}

pub fn do_shape_classification_partitions_by_warning() {
    for known in KNOWN_SHAPES {
        let claimed_by = NodeShape::iter().find(|shape| shape.style_spellings().contains(known));
        let expected = match claimed_by {
            Some(shape) => ShapeSpelling::Rendered(shape),
            None => ShapeSpelling::KnownNotYetRendered,
        };
        assert_eq!(
            ShapeSpelling::classify(known),
            expected,
            "canonical shape {known:?} is not classified the way the \
             NodeShape variant set says it should be"
        );
    }
    // Both halves must be non-empty, or the partition above is
    // asserting nothing about one of them. Today: three rendered
    // spellings, four awaiting a shader case.
    let rendered = KNOWN_SHAPES
        .iter()
        .filter(|known| matches!(ShapeSpelling::classify(known), ShapeSpelling::Rendered(_)))
        .count();
    assert!(rendered > 0, "no canonical spelling maps to a NodeShape variant");
    assert!(
        rendered < KNOWN_SHAPES.len(),
        "every canonical spelling is rendered — the KnownNotYetRendered \
         half of this test is now vacuous and needs rewriting"
    );
}

/// A spelling no `NodeShape` variant claims may still be canonical,
/// but a spelling that claims to be canonical while sitting outside
/// `KNOWN_SHAPES` is a bug in the other direction: `maptool verify`
/// would reject a map the runtime renders correctly. Exhaustive over
/// the variants via `EnumIter`, so a new variant is covered
/// automatically.
#[test]
pub fn test_shape_variant_spellings_are_all_known() {
    do_shape_variant_spellings_are_all_known();
}

pub fn do_shape_variant_spellings_are_all_known() {
    for shape in NodeShape::iter() {
        assert!(
            !shape.style_spellings().is_empty(),
            "{shape:?} declares no canonical spelling, so no map can ask for it"
        );
        for spelling in shape.style_spellings() {
            assert!(
                KNOWN_SHAPES.contains(spelling),
                "{shape:?} claims spelling {spelling:?}, which is not in \
                 KNOWN_SHAPES — maptool verify would reject it"
            );
            assert_eq!(
                ShapeSpelling::classify(spelling),
                ShapeSpelling::Rendered(shape),
                "{spelling:?} does not classify back to {shape:?}"
            );
        }
    }
}

/// `KNOWN_SHAPES` entries are written lowercase. The runtime's
/// `eq_ignore_ascii_case` compare and `maptool verify`'s
/// lowercase-normalize-then-match only agree while that holds, and
/// nothing in the type system says so.
#[test]
pub fn test_shape_known_shapes_are_lowercase() {
    do_shape_known_shapes_are_lowercase();
}

pub fn do_shape_known_shapes_are_lowercase() {
    for known in KNOWN_SHAPES {
        assert_eq!(
            *known,
            known.to_ascii_lowercase().as_str(),
            "KNOWN_SHAPES entry {known:?} is not lowercase"
        );
    }
}

/// The other half of issue #118: a value that really is unknown
/// stays reported. `"pentagram"` is nowhere in `KNOWN_SHAPES`, so it
/// is a typo or a value from a newer build — exactly the case the
/// warning was written for. The empty string is *not* that case;
/// it means the field was left unset.
#[test]
pub fn test_shape_unrecognized_spelling_still_warns() {
    do_shape_unrecognized_spelling_still_warns();
}

pub fn do_shape_unrecognized_spelling_still_warns() {
    let spelling = ShapeSpelling::classify("pentagram");
    assert_eq!(spelling, ShapeSpelling::Unrecognized);
    assert!(warns(spelling));
    assert_eq!(spelling.resolve(), NodeShape::Rectangle);
    assert_eq!(NodeShape::from_style_string("pentagram"), NodeShape::Rectangle);

    // Unset is silent, and has been since before #118.
    assert_eq!(ShapeSpelling::classify(""), ShapeSpelling::Unspecified);
    assert!(!warns(ShapeSpelling::classify("")));
}

/// The three spellings named in the issue, asserted one at a time
/// because each pins a different property: `"HEXAGON"` that the
/// canonical-but-unrendered path is case-insensitive (uppercase is
/// how the warning was first noticed to be spelling-sensitive),
/// `"Circle"` that the alias survives mixed case, `""` that unset
/// is still silent.
#[test]
pub fn test_shape_classify_case_and_alias() {
    do_shape_classify_case_and_alias();
}

pub fn do_shape_classify_case_and_alias() {
    assert_eq!(
        ShapeSpelling::classify("HEXAGON"),
        ShapeSpelling::KnownNotYetRendered
    );
    assert_eq!(
        ShapeSpelling::classify("hexagon"),
        ShapeSpelling::KnownNotYetRendered
    );
    assert_eq!(NodeShape::from_style_string("HEXAGON"), NodeShape::Rectangle);

    assert_eq!(
        ShapeSpelling::classify("Circle"),
        ShapeSpelling::Rendered(NodeShape::Ellipse)
    );
    assert_eq!(
        ShapeSpelling::classify("CIRCLE"),
        ShapeSpelling::Rendered(NodeShape::Ellipse)
    );
    assert_eq!(NodeShape::from_style_string("Circle"), NodeShape::Ellipse);

    assert_eq!(
        ShapeSpelling::classify("RECTANGLE"),
        ShapeSpelling::Rendered(NodeShape::Rectangle)
    );
    assert_eq!(
        ShapeSpelling::classify("Ellipse"),
        ShapeSpelling::Rendered(NodeShape::Ellipse)
    );

    assert_eq!(ShapeSpelling::classify(""), ShapeSpelling::Unspecified);
    assert_eq!(ShapeSpelling::classify("").resolve(), NodeShape::Rectangle);
}

/// **Issue #118, against the file that produced it.** Every
/// `style.shape` string in `maps/testament.mindmap.json` — the demo
/// map the app opens by default, 242 of whose nodes are hexagons —
/// must classify as non-warning. Asserted over the map's real shape
/// strings rather than over emitted log records: there is no logger
/// to install from here, and the classification is the thing the
/// warning is a function of.
///
/// The map is loaded through `mindmap::loader`, so a shape string
/// the loader would reject never reaches the assertion — which is
/// the point: this is the load path, not a synthetic list.
#[test]
pub fn test_shape_testament_map_has_no_unknown_shapes() {
    do_shape_testament_map_has_no_unknown_shapes();
}

pub fn do_shape_testament_map_has_no_unknown_shapes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../maps/testament.mindmap.json");
    let map = crate::mindmap::loader::load_from_file(std::path::Path::new(path))
        .expect("maps/testament.mindmap.json must load");

    let mut inspected = 0usize;
    for (location, node) in map.node_locations() {
        let spelling = ShapeSpelling::classify(&node.style.shape);
        assert!(
            !warns(spelling),
            "{location}: style.shape {:?} classified as {spelling:?} — \
             loading the demo map would warn",
            node.style.shape
        );
        inspected += 1;
    }
    assert!(
        inspected > 0,
        "no nodes inspected — the fixture walk found nothing and the \
         assertion above never ran"
    );
}

/// Rectangle `contains_local` matches the classic inclusive-AABB
/// predicate: corners and edges hit, anything outside `[0, bounds]`
/// misses. Locks in the happy path for the legacy behavior that
/// every existing node still depends on.
#[test]
pub fn test_shape_rectangle_contains_local() {
    do_shape_rectangle_contains_local();
}

pub fn do_shape_rectangle_contains_local() {
    let b = Vec2::new(100.0, 50.0);
    assert!(NodeShape::Rectangle.contains_local(Vec2::new(0.0, 0.0), b));
    assert!(NodeShape::Rectangle.contains_local(Vec2::new(100.0, 50.0), b));
    assert!(NodeShape::Rectangle.contains_local(Vec2::new(50.0, 25.0), b));
    assert!(!NodeShape::Rectangle.contains_local(Vec2::new(-0.1, 25.0), b));
    assert!(!NodeShape::Rectangle.contains_local(Vec2::new(100.1, 25.0), b));
}

/// Perfect-circle case: bounds 100×100, radius 50, center
/// `(50, 50)`. Center and the four cardinal rim points all count
/// as inside. Pins the "rim is inclusive" edge-case the BVH hit
/// test relies on for click-on-border behavior.
#[test]
pub fn test_shape_ellipse_contains_center_and_rim() {
    do_shape_ellipse_contains_center_and_rim();
}

pub fn do_shape_ellipse_contains_center_and_rim() {
    let b = Vec2::new(100.0, 100.0);
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(50.0, 50.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(0.0, 50.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(100.0, 50.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(50.0, 0.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(50.0, 100.0), b));
}

/// Bounding-box corners sit at distance `√2 · r` from the center
/// of an inscribed circle — comfortably outside. This is the
/// exact case the whole refactor exists to reject: under the
/// pre-change AABB-only hit test, a corner click on an ellipse
/// node would select it; post-change it must miss.
#[test]
pub fn test_shape_ellipse_rejects_aabb_corners() {
    do_shape_ellipse_rejects_aabb_corners();
}

pub fn do_shape_ellipse_rejects_aabb_corners() {
    let b = Vec2::new(100.0, 100.0);
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(0.0, 0.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(100.0, 0.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(0.0, 100.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(100.0, 100.0), b));
}

/// Stretched conic case: bounds `200 × 50`, radii `(100, 25)`.
/// Center and cardinal rim points still hit; bounding-box corners
/// still miss. Guards the "ellipse handles wider-than-tall without
/// extra parameters" claim from the shape doc comment.
#[test]
pub fn test_shape_ellipse_handles_stretched_conic() {
    do_shape_ellipse_handles_stretched_conic();
}

pub fn do_shape_ellipse_handles_stretched_conic() {
    let b = Vec2::new(200.0, 50.0);
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(100.0, 25.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(0.0, 25.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(200.0, 25.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(0.0, 0.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(200.0, 50.0), b));
}

/// Degenerate bounds (zero or negative extent on either axis)
/// never hit — guards the division by `bounds / 2` in the ellipse
/// math and mirrors how the BVH's AABB check skips zero-size areas.
/// Rendering a zero-size node is already a no-op upstream, so
/// counting a click as a miss is the internally consistent answer.
#[test]
pub fn test_shape_degenerate_bounds_never_hit() {
    do_shape_degenerate_bounds_never_hit();
}

pub fn do_shape_degenerate_bounds_never_hit() {
    assert!(!NodeShape::Rectangle.contains_local(Vec2::ZERO, Vec2::ZERO));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::ZERO, Vec2::new(0.0, 100.0)));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::ZERO, Vec2::new(100.0, -1.0)));
}

/// Selection rect tucked fully inside the ellipse: the closest
/// point on the rect to the ellipse center is the ellipse center
/// itself, so `distance == 0` and the test registers a hit.
/// Without this branch, the lasso would report "no nodes
/// selected" whenever the user drew a small rectangle inside a
/// circular node — the exact case a user would expect to match.
#[test]
pub fn test_shape_ellipse_intersects_aabb_fully_inside() {
    do_shape_ellipse_intersects_aabb_fully_inside();
}

pub fn do_shape_ellipse_intersects_aabb_fully_inside() {
    let b = Vec2::new(100.0, 100.0);
    let min = Vec2::new(40.0, 40.0);
    let max = Vec2::new(60.0, 60.0);
    assert!(NodeShape::Ellipse.intersects_local_aabb(min, max, b));
}

/// Selection rect tucked into the AABB corner, outside the
/// ellipse. The pre-change AABB-overlap test would have matched
/// this as "node selected"; the shape-aware test must reject it.
/// This is the case the rect-select refactor exists to fix.
#[test]
pub fn test_shape_ellipse_intersects_aabb_corner_only() {
    do_shape_ellipse_intersects_aabb_corner_only();
}

pub fn do_shape_ellipse_intersects_aabb_corner_only() {
    let b = Vec2::new(100.0, 100.0);
    let min = Vec2::new(0.0, 0.0);
    let max = Vec2::new(5.0, 5.0);
    assert!(!NodeShape::Ellipse.intersects_local_aabb(min, max, b));
}

/// Selection rect crossing the ellipse's left rim: the clamp
/// lands on the rect's inside edge (x ≈ 0), which is on the
/// ellipse boundary. Conservative (`<= 1.0`) counts this as a
/// hit — the spirit of a lasso is "any overlap selects".
#[test]
pub fn test_shape_ellipse_intersects_aabb_straddling_rim() {
    do_shape_ellipse_intersects_aabb_straddling_rim();
}

pub fn do_shape_ellipse_intersects_aabb_straddling_rim() {
    let b = Vec2::new(100.0, 100.0);
    let min = Vec2::new(-10.0, 40.0);
    let max = Vec2::new(10.0, 60.0);
    assert!(NodeShape::Ellipse.intersects_local_aabb(min, max, b));
}

/// Selection rect entirely outside the node's bounding box.
/// Early-bails on the AABB–AABB overlap so the shape math isn't
/// even reached. Guards the cheap path every lasso hit-test takes
/// when the user drags far away from any node.
#[test]
pub fn test_shape_ellipse_intersects_aabb_fully_outside() {
    do_shape_ellipse_intersects_aabb_fully_outside();
}

pub fn do_shape_ellipse_intersects_aabb_fully_outside() {
    let b = Vec2::new(100.0, 100.0);
    let min = Vec2::new(200.0, 200.0);
    let max = Vec2::new(300.0, 300.0);
    assert!(!NodeShape::Ellipse.intersects_local_aabb(min, max, b));
}

/// The `shader_id` values are wire-format: the fragment shader
/// matches on the same integers via `SHAPE_RECT` / `SHAPE_ELLIPSE`
/// WGSL constants. Pinning them here catches the silent-breakage
/// case where a future reorder of the enum variants reassigns the
/// ids and every ellipse in every map quietly renders as a
/// rectangle.
#[test]
pub fn test_shape_shader_ids_are_stable() {
    do_shape_shader_ids_are_stable();
}

pub fn do_shape_shader_ids_are_stable() {
    assert_eq!(NodeShape::Rectangle.shader_id(), SHAPE_ID_RECTANGLE);
    assert_eq!(NodeShape::Ellipse.shader_id(), SHAPE_ID_ELLIPSE);
    // The absolute values are also part of the wire format — the
    // WGSL fragment shader hard-codes `0u` / `1u` in its `switch`
    // arms. Keeping the numeric assertion here means a rename
    // alone can't drift them.
    assert_eq!(NodeShape::Rectangle.shader_id(), 0);
    assert_eq!(NodeShape::Ellipse.shader_id(), 1);
}
