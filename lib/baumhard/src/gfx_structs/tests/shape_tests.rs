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
            !spelling.is_author_error(),
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

/// `KNOWN_SHAPES` entries are written lowercase, and nothing in the
/// type system says so.
///
/// **Not** because the runtime and `maptool verify` would otherwise
/// disagree — both compare with `eq_ignore_ascii_case` against this
/// same list (`ShapeSpelling::classify` here, `check_value` in
/// `crates/maptool/src/verify/enums.rs`), so they agree on case by
/// construction and `"HEXAGON"` in the list still verifies clean.
///
/// What the invariant protects is the two case-*sensitive*
/// `slice::contains` calls in the sibling tests below:
/// `do_shape_classification_partitions_by_warning` asks whether a
/// variant's `style_spellings` contains the entry, and
/// `do_shape_variant_spellings_are_all_known` asks the mirror
/// question of `KNOWN_SHAPES`. Uppercase a variant-claimed spelling
/// and it silently changes sides of the rendered / not-yet-rendered
/// partition; both go red, and this test is what names the cause
/// rather than leaving two derived failures neither of which says
/// "the list is mixed case".
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
///
/// Also pins `is_author_error` one classification at a time, since
/// it is the predicate every other test in this file asks. A blanket
/// `true` or `false` there would otherwise make several of them pass
/// for the wrong reason.
#[test]
pub fn test_shape_unrecognized_spelling_still_warns() {
    do_shape_unrecognized_spelling_still_warns();
}

pub fn do_shape_unrecognized_spelling_still_warns() {
    let spelling = ShapeSpelling::classify("pentagram");
    assert_eq!(spelling, ShapeSpelling::Unrecognized);
    assert!(spelling.is_author_error());
    assert_eq!(spelling.resolve(), NodeShape::Rectangle);
    assert_eq!(NodeShape::from_style_string("pentagram"), NodeShape::Rectangle);

    // Unset is silent, and has been since before #118.
    assert_eq!(ShapeSpelling::classify(""), ShapeSpelling::Unspecified);
    assert!(!ShapeSpelling::classify("").is_author_error());

    assert!(ShapeSpelling::Unrecognized.is_author_error());
    assert!(!ShapeSpelling::Unspecified.is_author_error());
    assert!(!ShapeSpelling::KnownNotYetRendered.is_author_error());
    for shape in NodeShape::iter() {
        assert!(
            !ShapeSpelling::Rendered(shape).is_author_error(),
            "{shape:?} renders as asked, so there is nothing to report"
        );
    }
}

/// The two library-owned reporting predicates, pinned **as a pair**
/// for every classification. `from_style_string` calls
/// `is_author_error` to reach `log::warn!` and `is_quiet_fallback` to
/// reach `log::trace!`, so between them these two booleans are the
/// entire reporting contract of the shipped loader — not a test-only
/// restatement of it.
///
/// Exhaustive by construction: the expectation comes out of a `match`
/// with no `_` arm, so a fifth `ShapeSpelling` variant fails to
/// compile here rather than defaulting into silence at the log site.
///
/// Disjointness is asserted too. A classification answering `true` to
/// both would make what a load actually logs depend on the order
/// `from_style_string` happens to test them in, which is exactly the
/// kind of coupling the split was meant to remove.
#[test]
pub fn test_shape_reporting_predicates_partition() {
    do_shape_reporting_predicates_partition();
}

pub fn do_shape_reporting_predicates_partition() {
    let mut cases = vec![
        ShapeSpelling::Unspecified,
        ShapeSpelling::KnownNotYetRendered,
        ShapeSpelling::Unrecognized,
    ];
    cases.extend(NodeShape::iter().map(ShapeSpelling::Rendered));
    assert!(
        cases.len() > 3,
        "no NodeShape variant produced a Rendered case — the loop below \
         would never exercise one"
    );

    for spelling in cases {
        let expected = match spelling {
            ShapeSpelling::Unrecognized => (true, false),
            ShapeSpelling::KnownNotYetRendered => (false, true),
            ShapeSpelling::Unspecified | ShapeSpelling::Rendered(_) => (false, false),
        };
        assert_eq!(
            (spelling.is_author_error(), spelling.is_quiet_fallback()),
            expected,
            "{spelling:?} answers the wrong (is_author_error, \
             is_quiet_fallback) pair, which is what from_style_string logs on"
        );
        assert!(
            !(spelling.is_author_error() && spelling.is_quiet_fallback()),
            "{spelling:?} is both an author error and a quiet fallback, so \
             what a load logs would depend on branch order"
        );
    }
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

/// **`format/enums.md` restates `KNOWN_SHAPES`, so the doc is read
/// rather than trusted.**
///
/// `shape.rs` calls its list the only one a parser consults, which is
/// true, and the PR that introduced the classifier went further and
/// called it the only list in the tree, which was not: the format doc
/// publishes the vocabulary three times over in its `style.shape`
/// section — once as the canonical fence, once as the "live shapes"
/// sentence, once as the "remaining values" parenthesis — and nothing
/// tied any of them to the constant. Drop or rename a `KNOWN_SHAPES`
/// entry and the doc keeps publishing the old spelling to authors who
/// will then write maps `maptool verify` rejects.
///
/// Each of the three is checked against a set *derived* from the code
/// rather than restated here: the fence against `KNOWN_SHAPES` in
/// order, the live sentence against the canonical spelling of each
/// `NodeShape` variant, the remaining parenthesis against the entries
/// that classify `KnownNotYetRendered`. The three are then required
/// to cover `KNOWN_SHAPES` between them, so an entry cannot be
/// dropped from the section as a whole either.
///
/// The converter's copy of the same vocabulary is pinned on maptool's
/// side, by `legacy_shape_ordinals_are_canonical_spellings` — it lives
/// there because baumhard does not depend on `maptool`.
#[test]
pub fn test_shape_format_doc_publishes_exactly_known_shapes() {
    do_shape_format_doc_publishes_exactly_known_shapes();
}

pub fn do_shape_format_doc_publishes_exactly_known_shapes() {
    let section = documented_shape_section();

    // 1. The canonical fenced list, in order.
    let fence = fenced_spellings(&section);
    assert_eq!(
        fence,
        KNOWN_SHAPES.to_vec(),
        "the `style.shape` code fence in format/enums.md no longer \
         publishes exactly KNOWN_SHAPES, in order"
    );

    // 2. The "live shapes" sentence: one canonical spelling per
    //    drawable variant, in variant order.
    let live = backticked_spellings(paragraph_before(&section, "are **live shapes**"));
    let expected_live: Vec<&str> = NodeShape::iter()
        .map(|shape| {
            *shape
                .style_spellings()
                .first()
                .expect("every variant declares a canonical spelling")
        })
        .collect();
    assert_eq!(
        live, expected_live,
        "format/enums.md's live-shapes sentence and the NodeShape \
         variant set disagree about what the renderer can draw"
    );

    // 3. The "remaining values" parenthesis: exactly the canonical
    //    spellings with no variant behind them yet.
    let remaining = backticked_spellings(parenthesized_after(&section, "The remaining values ("));
    let expected_remaining: Vec<&str> = KNOWN_SHAPES
        .iter()
        .copied()
        .filter(|known| ShapeSpelling::classify(known) == ShapeSpelling::KnownNotYetRendered)
        .collect();
    assert_eq!(
        remaining, expected_remaining,
        "format/enums.md's \"remaining values\" list and the classifier \
         disagree about which canonical spellings are not drawn yet"
    );

    // 4. Between them the two prose lists plus the aliases must cover
    //    the whole constant, so no entry can quietly leave the
    //    section.
    for known in KNOWN_SHAPES {
        // The alias case (`"circle"`) is named in prose rather than
        // in either list, so a bare code span counts as coverage.
        let code_span = format!("`{known:?}`");
        let covered = live.contains(known) || remaining.contains(known) || section.contains(&code_span);
        assert!(
            covered,
            "canonical spelling {known:?} is published by KNOWN_SHAPES but \
             appears nowhere in format/enums.md's `style.shape` section"
        );
    }
}

/// The body of `format/enums.md`'s `### \`style.shape\`` section:
/// everything between that heading and the next `###`.
///
/// Panics rather than returning empty if the heading is gone — a
/// silent fallback would let this whole test pass vacuously, which is
/// the failure mode it exists to prevent.
fn documented_shape_section() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../format/enums.md");
    let doc = std::fs::read_to_string(path).expect("format/enums.md must be readable");
    let after = doc
        .split_once("### `style.shape`")
        .expect("format/enums.md must still document `style.shape`")
        .1;
    match after.split_once("\n### ") {
        Some((section, _)) => section.to_string(),
        None => after.to_string(),
    }
}

/// The `"quoted"` spellings inside the section's first fenced code
/// block, in source order.
fn fenced_spellings(section: &str) -> Vec<&str> {
    let after = section
        .split_once("```")
        .expect("the `style.shape` section must open a code fence")
        .1;
    let fence = after
        .split_once("```")
        .expect("the `style.shape` code fence must be closed")
        .0;
    quoted_spans(fence)
}

/// The part of the paragraph that runs up to `marker` — everything
/// after the last blank line preceding it. Scoping to one paragraph
/// is what keeps the fenced list and neighboring prose out of a
/// sentence-level assertion.
fn paragraph_before<'a>(section: &'a str, marker: &str) -> &'a str {
    let head = section
        .split_once(marker)
        .unwrap_or_else(|| panic!("format/enums.md must still contain {marker:?}"))
        .0;
    head.rsplit("\n\n")
        .next()
        .expect("rsplit always yields a first element")
}

/// The text between `marker` and the first following `)`.
fn parenthesized_after<'a>(section: &'a str, marker: &str) -> &'a str {
    let after = section
        .split_once(marker)
        .unwrap_or_else(|| panic!("format/enums.md must still contain {marker:?}"))
        .1;
    after
        .split_once(')')
        .expect("the parenthesis opened by that marker must be closed")
        .0
}

/// Spellings written as prose code spans — a backtick, a quoted
/// spelling, a backtick. Prose also contains bare quoted words
/// (`"conical"`), so the backticks are what tell a published spelling
/// from an ordinary one.
fn backticked_spellings(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for span in text.split('`') {
        if let Some(inner) = span.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            out.push(inner);
        }
    }
    out
}

/// Every `"quoted"` span in `text`, in order.
fn quoted_spans(text: &str) -> Vec<&str> {
    text.split('"').skip(1).step_by(2).collect()
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
            !spelling.is_author_error(),
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

/// **Which `log::` macro each reporting branch reaches, pinned by
/// parsing `shape.rs` itself.**
///
/// Every other test in this file asserts on a returned value, and a
/// returned value cannot see a log call. That left issue #118's
/// *second* requirement — "a value not in `KNOWN_SHAPES`: keep the
/// `warn!`" — untested in a way that mattered: deleting the entire
/// reporting block from `from_style_string` left the suite green, and
/// so did swapping the two macros. The gap was never "which macro is
/// written"; it was whether anything is written at all.
///
/// The closure everyone reached for first is a test logger, and
/// `util::test_logger` really is on an unmerged branch (#117). It is
/// not the only one. `syn` is already a baumhard dev-dependency for
/// exactly this class of question — `util::serde_coverage` parses the
/// crate's own sources so a contract can be *derived* from the code
/// instead of restated beside it — and the routing of a `log::` macro
/// is a fact about the source text. So this module reads it out of
/// the source.
///
/// §B8 asks for a `do_*()` body and a bench entry per test; these two
/// have neither, deliberately and with precedent. `syn` is a
/// dev-dependency, so a `pub` body in this `pub mod tests` tree would
/// not compile into the library, and `serde_coverage`'s own consumer
/// tests are plain `#[cfg(test)]` tests for the same reason. Native
/// only: it reads a file.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod log_routing {
    use syn::{Block, Expr, ImplItem, Item, Path, Stmt, Type};

    /// The `(guard predicate, `log::` macro)` pairs
    /// `NodeShape::from_style_string` must contain, in source order.
    ///
    /// This is the reporting contract in one place: `warn!` behind
    /// `is_author_error`, `trace!` behind `is_quiet_fallback`, and
    /// nothing else. Changing the shape of the routing — a `match` on
    /// some future `report()` enum, say — is meant to fail here, so
    /// that moving the decision is a decision somebody makes rather
    /// than one that happens.
    const EXPECTED_ROUTING: &[(&str, &str)] = &[("is_author_error", "warn"), ("is_quiet_fallback", "trace")];

    /// Stands in for the guard of a `log::` call that is not inside
    /// an `if` at all — an unconditional warning, for instance.
    const UNGUARDED: &str = "<unguarded>";

    /// Macro names that count as reporting. `log` itself is here so a
    /// switch to `log::log!(level, …)`, which moves the level into a
    /// runtime value and out of this test's sight, shows up as a
    /// changed pair rather than as silence.
    const LOG_MACROS: &[&str] = &["error", "warn", "info", "debug", "trace", "log"];

    /// The pinned routing, read out of the shipped source.
    #[test]
    fn test_from_style_string_log_routing_is_pinned() {
        let mut found = Vec::new();
        walk_block(&from_style_string_body(), None, &mut found);

        let expected: Vec<(String, String)> = EXPECTED_ROUTING
            .iter()
            .map(|(guard, level)| ((*guard).to_string(), (*level).to_string()))
            .collect();
        assert!(
            !expected.is_empty(),
            "EXPECTED_ROUTING is empty — this test would pass vacuously"
        );
        assert_eq!(
            found, expected,
            "NodeShape::from_style_string's log routing changed. An empty \
             list means the reporting was deleted outright, which is issue \
             #118's second requirement silently removed; a {UNGUARDED:?} \
             guard means a log call escaped both reporting predicates; a \
             changed level means warn! and trace! swapped places."
        );
    }

    /// Positive control for the walker, so a green run above cannot
    /// mean "the parse found nothing and said nothing". Feeds it a
    /// block with both levels transposed and one unguarded call, and
    /// requires it to report all three.
    #[test]
    fn test_log_routing_walker_reports_a_mis_routed_call() {
        let block: Block = syn::parse_str(
            r#"{
                if spelling.is_author_error() {
                    log::trace!("transposed");
                } else if spelling.is_quiet_fallback() {
                    log::warn!("transposed");
                }
                log::error!("no guard at all");
            }"#,
        )
        .expect("the control snippet must parse as a block");

        let mut found = Vec::new();
        walk_block(&block, None, &mut found);
        assert_eq!(
            found,
            vec![
                ("is_author_error".to_string(), "trace".to_string()),
                ("is_quiet_fallback".to_string(), "warn".to_string()),
                (UNGUARDED.to_string(), "error".to_string()),
            ],
            "the walker did not see a routing it was handed directly"
        );
    }

    /// The body of `NodeShape::from_style_string`, parsed out of
    /// `shape.rs`. Panics if the function is gone: a silent skip
    /// would turn the pin above into a no-op.
    fn from_style_string_body() -> Block {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/gfx_structs/shape.rs");
        let text = std::fs::read_to_string(path).expect("shape.rs must be readable");
        let file = syn::parse_file(&text).expect("shape.rs must parse as Rust");
        for item in &file.items {
            let Item::Impl(item) = item else { continue };
            if !is_impl_for(&item.self_ty, "NodeShape") {
                continue;
            }
            for member in &item.items {
                if let ImplItem::Fn(function) = member {
                    if function.sig.ident == "from_style_string" {
                        return function.block.clone();
                    }
                }
            }
        }
        panic!("shape.rs no longer defines NodeShape::from_style_string");
    }

    fn is_impl_for(ty: &Type, name: &str) -> bool {
        match ty {
            Type::Path(path) => path
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == name),
            _ => false,
        }
    }

    /// Record every reporting macro in `block`, tagged with the
    /// method name of the `if` condition guarding it.
    fn walk_block(block: &Block, guard: Option<&str>, out: &mut Vec<(String, String)>) {
        for stmt in &block.stmts {
            match stmt {
                Stmt::Expr(expr, _) => walk_expr(expr, guard, out),
                Stmt::Local(local) => {
                    if let Some(init) = &local.init {
                        walk_expr(&init.expr, guard, out);
                    }
                }
                Stmt::Macro(node) => record(&node.mac.path, guard, out),
                Stmt::Item(_) => {}
            }
        }
    }

    fn walk_expr(expr: &Expr, guard: Option<&str>, out: &mut Vec<(String, String)>) {
        match expr {
            Expr::If(node) => {
                let inner = guard_name(&node.cond);
                walk_block(&node.then_branch, inner.as_deref(), out);
                if let Some((_, otherwise)) = &node.else_branch {
                    walk_expr(otherwise, guard, out);
                }
            }
            Expr::Match(node) => {
                for arm in &node.arms {
                    walk_expr(&arm.body, guard, out);
                }
            }
            Expr::Block(node) => walk_block(&node.block, guard, out),
            Expr::Macro(node) => record(&node.mac.path, guard, out),
            _ => {}
        }
    }

    /// The method an `if` condition calls, which is how a guard is
    /// named here. A condition that is not a plain method call
    /// leaves its branch `UNGUARDED`, so replacing
    /// `spelling.is_author_error()` with an inlined comparison fails
    /// the pin rather than passing it under a new name.
    fn guard_name(cond: &Expr) -> Option<String> {
        match cond {
            Expr::MethodCall(call) => Some(call.method.to_string()),
            _ => None,
        }
    }

    fn record(path: &Path, guard: Option<&str>, out: &mut Vec<(String, String)>) {
        let Some(last) = path.segments.last() else { return };
        let name = last.ident.to_string();
        let from_log = path.segments.len() == 1
            || path
                .segments
                .first()
                .is_some_and(|segment| segment.ident == "log");
        if from_log && LOG_MACROS.contains(&name.as_str()) {
            out.push((guard.unwrap_or(UNGUARDED).to_string(), name));
        }
    }
}
