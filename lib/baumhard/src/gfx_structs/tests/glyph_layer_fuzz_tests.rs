// SPDX-License-Identifier: MPL-2.0

//! The glyph-layer mutation fuzz properties held to account
//! (TEST_CONVENTIONS §T14): the structure-aware generator swept under a
//! seeded source, the JSON half on the documented wire spellings and on
//! hostile ones, and the box property shown to go red on a planted box
//! at infinity.

use glam::Vec2;

use crate::engine::element::TreeElement;
use crate::engine::tree::Placement;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::tests::fill_tests::a_fill;
use crate::gfx_structs::tests::glyph_layer_fuzz::{
    boxes_are_finite, check_glyph_mutations_generated, check_glyph_mutations_json,
};
use crate::gfx_structs::tests::glyph_layer_tests::authored_glyph_layer_mutations;
use crate::gfx_structs::tree::GfxTree;
use crate::util::tests::choices::Choices;

#[test]
fn test_generated_glyph_mutations_hold_every_property() {
    for seed in 0..2000 {
        if let Err(why) = check_glyph_mutations_generated(&mut Choices::seeded(seed)) {
            panic!("seed {seed}: {why}");
        }
    }
}

/// The wire spellings `format/mutations.md` publishes, one by one and as
/// one list, and hostile ones: values a JSON literal reads as infinity,
/// ranges to `usize::MAX`, backward and empty.
#[test]
fn test_authored_and_hostile_json_mutations_hold_every_property() {
    let authored: Vec<&str> = authored_glyph_layer_mutations()
        .into_iter()
        .map(|(_, wire)| wire)
        .collect();
    assert!(
        authored.len() >= 10,
        "precondition: the documented spellings are read"
    );
    for wire in &authored {
        check_glyph_mutations_json(wire.as_bytes()).unwrap_or_else(|why| panic!("{wire}: {why}"));
    }
    let list = format!("[{}]", authored.join(","));
    check_glyph_mutations_json(list.as_bytes()).unwrap_or_else(|why| panic!("{list}: {why}"));
    for wire in [
        r#"{"RunOrigin":[1e39,-1e39]}"#,
        r#"{"RunOpacity":-1e39}"#,
        r#"{"RunOverrides":{"Scale":{"range":{"start":0,"end":18446744073709551615},"value":1e39}}}"#,
        r#"{"RunOverrides":{"Offset":{"range":{"start":18446744073709551615,"end":0},"value":[1e39,1e39]}}}"#,
        r#"{"RunOverrides":{"Color":{"range":{"start":3,"end":3},"value":[0,0,0,0]}}}"#,
        r#"{"FillRect":{"origin":[1e39,0.0],"size":[10.0,10.0]}}"#,
        r#"{"FillRect":{"origin":[3.0e38,0.0],"size":[3.0e38,10.0]}}"#,
        r#"{"FillOpacity":1e39}"#,
        r#"[{"RunOverrides":"Clear"},{"RunOverrides":{"Scale":{"range":{"start":0,"end":1},"value":0.0}}}]"#,
        r#"{"RunSet":{}}"#,
        r#"{"AreaCommand":{"MoveTo":[1e39,0.0]}}"#,
        r#"[[[[["#,
        "",
    ] {
        check_glyph_mutations_json(wire.as_bytes()).unwrap_or_else(|why| panic!("{wire}: {why}"));
    }
}

/// The glyph-mutation target's finding: an authored `MoveTo` to `1e39`
/// (infinity, as a JSON literal reads it) put an area at infinity, and
/// an area answered its box unchecked, so every ancestor's subtree box
/// stretched to infinity. A run's and a fill's box were already held to
/// be finite; an area's now is too.
#[test]
fn test_an_area_moved_to_infinity_has_no_box_and_stretches_no_ancestor() {
    let finding = include_bytes!("fuzz_findings/area_moved_to_infinity_stretches_its_ancestors.json");
    check_glyph_mutations_json(finding).unwrap();

    let mut tree = GfxTree::new_non_indexed();
    let root = tree.root;
    let void = tree
        .insert_child(root, GfxElement::new_void(0), Placement::Last)
        .unwrap();
    tree.insert_child(void, GfxElement::new_fill(a_fill(), 0, 1), Placement::Last)
        .unwrap();
    let area = |x: f32| {
        GfxElement::new_area_non_indexed(
            GlyphArea::new(12.0, 14.0, Vec2::new(x, 0.0), Vec2::new(10.0, 10.0)),
            1,
        )
    };
    // Control: a finite area has its box, and the ancestors take it in.
    let finite = tree.insert_child(void, area(500.0), Placement::Last).unwrap();
    assert!(tree.arena()[finite].get().spatial_bounds().is_some());
    assert_eq!(boxes_are_finite(&mut tree), Ok(()));
    assert_eq!(tree.subtree_aabb(void).map(|(_, max)| max.x), Some(510.0));
    tree.remove_subtree(finite).unwrap();

    let at_infinity = tree
        .insert_child(void, area(f32::INFINITY), Placement::Last)
        .unwrap();
    assert_eq!(tree.arena()[at_infinity].get().spatial_bounds(), None);
    assert_eq!(boxes_are_finite(&mut tree), Ok(()));
    assert_eq!(
        tree.subtree_aabb(void),
        Some((Vec2::new(10.0, 20.0), Vec2::new(110.0, 70.0))),
        "the void's box is its fill's alone"
    );
}
