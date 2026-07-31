// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::gfx_structs::model::GlyphModel`] — line/matrix
//! construction and component layout (§T1).

use crate::core::primitives::{Applicable, ApplyOperation, ColorFontRegions, Range};
use crate::font::fonts::AppFont;
use crate::gfx_structs::model::{
    DeltaGlyphModel, GlyphComponent, GlyphLine, GlyphMatrix, GlyphModel, GlyphModelField,
};
use crate::util::color::Color;

/// The tests are written in a non-test-annotated function and then wrapped by an annotated test function
/// So that they can be reused for benchmarking

#[test]
pub fn test_matrix_place_in_1() {
    matrix_place_in_1();
}

pub fn matrix_place_in_1() {
    let mut matrix = GlyphMatrix::new();
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));

    let mut regions = ColorFontRegions::new_empty();
    let mut my_string = String::new();

    matrix.place_in(&mut my_string, &mut regions, (10, 10));
    assert_matrix_place_in(&my_string, &regions);
    // Just asserting that the operation is idempotent
    matrix.place_in(&mut my_string, &mut regions, (10, 10));
    assert_matrix_place_in(&my_string, &regions);
    matrix.place_in(&mut my_string, &mut regions, (10, 10));
    assert_matrix_place_in(&my_string, &regions);
    matrix.place_in(&mut my_string, &mut regions, (10, 10));
    assert_matrix_place_in(&my_string, &regions);
    matrix.place_in(&mut my_string, &mut regions, (10, 10));
    assert_matrix_place_in(&my_string, &regions);
}

fn assert_matrix_place_in(my_string: &String, regions: &ColorFontRegions) {
    assert_eq!(
        my_string,
        "\n\n\n\n\n\n\n\n\n\n          ##########\n          ##########\n          ##########"
    );
    assert_eq!(regions.num_regions(), 3);
    let first_region = regions.regions.first();
    assert_eq!(first_region.is_some(), true);
    let unwrapped_first_region = first_region.unwrap();
    assert_eq!(unwrapped_first_region.range, Range::new(20, 30));
    assert_eq!(unwrapped_first_region.color.unwrap(), Color::black().to_float());
    assert_eq!(unwrapped_first_region.font.unwrap(), AppFont::Evilz);
    let second_region = regions.get(Range::new(41, 51));
    assert_eq!(second_region.is_some(), true);
    let unwrapped_second_region = second_region.unwrap();
    assert_eq!(unwrapped_second_region.range, Range::new(41, 51));
    assert_eq!(unwrapped_second_region.color.unwrap(), Color::black().to_float());
    assert_eq!(unwrapped_second_region.font.unwrap(), AppFont::Evilz);
    let third_region = regions.get(Range::new(62, 72));
    assert_eq!(third_region.is_some(), true);
    let unwrapped_third_region = third_region.unwrap();
    assert_eq!(unwrapped_third_region.range, Range::new(62, 72));
    assert_eq!(unwrapped_third_region.color.unwrap(), Color::black().to_float());
    assert_eq!(unwrapped_third_region.font.unwrap(), AppFont::Evilz);
}

#[test]
pub fn test_matrix_place_in_2() {
    matrix_place_in_2();
}

pub fn matrix_place_in_2() {
    let mut matrix_a = GlyphMatrix::new();
    matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));

    let mut matrix_b = GlyphMatrix::new();
    matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::white(),
    )));
    matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::white(),
    )));
    matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::white(),
    )));

    let mut regions = ColorFontRegions::new_empty();
    let mut my_string = String::new();
    matrix_a.place_in(&mut my_string, &mut regions, (0, 0));
    assert_eq!(my_string, "##########\n##########\n##########");
    assert_eq!(regions.num_regions(), 3);
    {
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(11, 21)).unwrap();
        let _region_3 = regions.get(Range::new(22, 32)).unwrap();
    }
    matrix_b.place_in(&mut my_string, &mut regions, (10, 0));
    assert_eq!(
        my_string,
        "##########@@@@@@@@@@\n##########@@@@@@@@@@\n##########@@@@@@@@@@"
    );
    assert_eq!(regions.num_regions(), 6);
    {
        // Corresponds to previous
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(21, 31)).unwrap();
        let _region_3 = regions.get(Range::new(42, 52)).unwrap();
        // New regions
        let _region_4 = regions.get(Range::new(10, 20)).unwrap();
        let _region_5 = regions.get(Range::new(31, 41)).unwrap();
        let _region_6 = regions.get(Range::new(52, 62)).unwrap();
    }
    matrix_a.place_in(&mut my_string, &mut regions, (10, 3));
    assert_eq!(my_string, "##########@@@@@@@@@@\n##########@@@@@@@@@@\n##########@@@@@@@@@@\n          ##########\n          ##########\n          ##########");
    assert_eq!(regions.num_regions(), 9);
    {
        // Corresponds to previous
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(21, 31)).unwrap();
        let _region_3 = regions.get(Range::new(42, 52)).unwrap();
        let _region_4 = regions.get(Range::new(10, 20)).unwrap();
        let _region_5 = regions.get(Range::new(31, 41)).unwrap();
        let _region_6 = regions.get(Range::new(52, 62)).unwrap();
        // new regions
        let _region_7 = regions.get(Range::new(73, 83)).unwrap();
        let _region_8 = regions.get(Range::new(94, 104)).unwrap();
        let _region_9 = regions.get(Range::new(115, 125)).unwrap();
    }
    matrix_b.place_in(&mut my_string, &mut regions, (0, 3));
    assert_eq!(my_string, "##########@@@@@@@@@@\n##########@@@@@@@@@@\n##########@@@@@@@@@@\n@@@@@@@@@@##########\n@@@@@@@@@@##########\n@@@@@@@@@@##########");
    assert_eq!(regions.num_regions(), 12);
    {
        // Corresponds to previous
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(21, 31)).unwrap();
        let _region_3 = regions.get(Range::new(42, 52)).unwrap();
        let _region_4 = regions.get(Range::new(10, 20)).unwrap();
        let _region_5 = regions.get(Range::new(31, 41)).unwrap();
        let _region_6 = regions.get(Range::new(52, 62)).unwrap();
        let _region_7 = regions.get(Range::new(73, 83)).unwrap();
        let _region_8 = regions.get(Range::new(94, 104)).unwrap();
        let _region_9 = regions.get(Range::new(115, 125)).unwrap();
        // new regions
        let _region_10 = regions.get(Range::new(63, 73)).unwrap();
        let _region_11 = regions.get(Range::new(84, 94)).unwrap();
        let _region_12 = regions.get(Range::new(105, 115)).unwrap();
    }
}

#[test]
pub fn test_matrix_place_in_3() {
    matrix_place_in_3();
}

pub fn matrix_place_in_3() {
    let mut matrix_a = GlyphMatrix::new();
    matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
        "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
        AppFont::AlphaMusicMan,
        Color::black(),
    )));
    matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
        "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
        "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
        AppFont::AliceInWonderland,
        Color::black(),
    )));

    let mut matrix_b = GlyphMatrix::new();
    matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::white(),
    )));
    matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::white(),
    )));
    matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::white(),
    )));

    let mut regions = ColorFontRegions::new_empty();
    let mut my_string = String::new();
    matrix_a.place_in(&mut my_string, &mut regions, (0, 0));
    assert_eq!(
        my_string,
        "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"
    );
    assert_eq!(regions.num_regions(), 3);
    {
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(11, 21)).unwrap();
        let _region_3 = regions.get(Range::new(22, 32)).unwrap();
    }
    matrix_b.place_in(&mut my_string, &mut regions, (10, 0));
    assert_eq!(
        my_string,
        "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@"
    );
    assert_eq!(regions.num_regions(), 6);
    {
        // Corresponds to previous
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(21, 31)).unwrap();
        let _region_3 = regions.get(Range::new(42, 52)).unwrap();
        // New regions
        let _region_4 = regions.get(Range::new(10, 20)).unwrap();
        let _region_5 = regions.get(Range::new(31, 41)).unwrap();
        let _region_6 = regions.get(Range::new(52, 62)).unwrap();
    }
    matrix_a.place_in(&mut my_string, &mut regions, (10, 3));
    assert_eq!(my_string,
              "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻");
    assert_eq!(regions.num_regions(), 9);
    {
        // Corresponds to previous
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(21, 31)).unwrap();
        let _region_3 = regions.get(Range::new(42, 52)).unwrap();
        let _region_4 = regions.get(Range::new(10, 20)).unwrap();
        let _region_5 = regions.get(Range::new(31, 41)).unwrap();
        let _region_6 = regions.get(Range::new(52, 62)).unwrap();
        // new regions
        let _region_7 = regions.get(Range::new(73, 83)).unwrap();
        let _region_8 = regions.get(Range::new(94, 104)).unwrap();
        let _region_9 = regions.get(Range::new(115, 125)).unwrap();
    }
    matrix_b.place_in(&mut my_string, &mut regions, (0, 3));
    assert_eq!(my_string, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n@@@@@@@@@@🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n@@@@@@@@@@🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n@@@@@@@@@@🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻");
    assert_eq!(regions.num_regions(), 12);
    {
        // Corresponds to previous
        let _region_1 = regions.get(Range::new(0, 10)).unwrap();
        let _region_2 = regions.get(Range::new(21, 31)).unwrap();
        let _region_3 = regions.get(Range::new(42, 52)).unwrap();
        let _region_4 = regions.get(Range::new(10, 20)).unwrap();
        let _region_5 = regions.get(Range::new(31, 41)).unwrap();
        let _region_6 = regions.get(Range::new(52, 62)).unwrap();
        let _region_7 = regions.get(Range::new(73, 83)).unwrap();
        let _region_8 = regions.get(Range::new(94, 104)).unwrap();
        let _region_9 = regions.get(Range::new(115, 125)).unwrap();
        // new regions
        let _region_10 = regions.get(Range::new(63, 73)).unwrap();
        let _region_11 = regions.get(Range::new(84, 94)).unwrap();
        let _region_12 = regions.get(Range::new(105, 115)).unwrap();
    }
}

#[test]
pub fn test_matrix_add_assign_2() {
    matrix_add_assign_2();
}

pub fn matrix_add_assign_2() {
    let mut matrix = GlyphMatrix::new();

    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::black(),
    )));

    let mut modifier_matrix = GlyphMatrix::new();
    modifier_matrix.push(GlyphLine::new());
    modifier_matrix.push(GlyphLine::new());
    modifier_matrix.push(GlyphLine::new());

    modifier_matrix.push(GlyphLine::new_with_vec(
        vec![
            GlyphComponent::space(3),
            GlyphComponent::text("HELP", AppFont::Evilz, Color::black()),
        ],
        true,
    ));

    modifier_matrix.push(GlyphLine::new_with_vec(
        vec![
            GlyphComponent::space(3),
            GlyphComponent::text("HELP", AppFont::Evilz, Color::black()),
        ],
        true,
    ));

    modifier_matrix.push(GlyphLine::new_with_vec(
        vec![
            GlyphComponent::space(3),
            GlyphComponent::text("HELP", AppFont::Evilz, Color::black()),
        ],
        true,
    ));

    modifier_matrix.push(GlyphLine::new_with_vec(
        vec![
            GlyphComponent::space(3),
            GlyphComponent::text("HELP", AppFont::Evilz, Color::black()),
        ],
        true,
    ));

    matrix += modifier_matrix;

    assert_eq!(matrix.get(0).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(1).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(2).unwrap().get(0).unwrap().text, "##########");

    assert_eq!(matrix.get(3).unwrap().get(0).unwrap().text, "###");
    assert_eq!(matrix.get(3).unwrap().get(1).unwrap().text, "HELP");
    assert_eq!(matrix.get(3).unwrap().get(2).unwrap().text, "###");

    assert_eq!(matrix.get(4).unwrap().get(0).unwrap().text, "###");
    assert_eq!(matrix.get(4).unwrap().get(1).unwrap().text, "HELP");
    assert_eq!(matrix.get(4).unwrap().get(2).unwrap().text, "###");

    assert_eq!(matrix.get(5).unwrap().get(0).unwrap().text, "###");
    assert_eq!(matrix.get(5).unwrap().get(1).unwrap().text, "HELP");
    assert_eq!(matrix.get(5).unwrap().get(2).unwrap().text, "###");

    assert_eq!(matrix.get(6).unwrap().get(0).unwrap().text, "###");
    assert_eq!(matrix.get(6).unwrap().get(1).unwrap().text, "HELP");
    assert_eq!(matrix.get(6).unwrap().get(2).unwrap().text, "###");

    assert_eq!(matrix.get(7).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(8).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(9).unwrap().get(0).unwrap().text, "##########");
}

#[test]
pub fn test_matrix_add_assign_1() {
    matrix_add_assign_1();
}

pub fn matrix_add_assign_1() {
    let mut matrix = create_default_matrix();

    let mut modifier_matrix = GlyphMatrix::new();
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));

    matrix += modifier_matrix;

    assert_eq!(matrix.get(0).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(0).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(1).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(1).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(2).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(2).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(3).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(3).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(4).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(5).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(6).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(7).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(8).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(9).unwrap().get(0).unwrap().text, "##########");
}

#[test]
pub fn test_matrix_mul_assign_1() {
    matrix_mul_assign_1();
}

pub fn matrix_mul_assign_1() {
    let mut matrix = create_default_matrix();

    let mut modifier_matrix = GlyphMatrix::new();
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));
    modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "HELP",
        AppFont::Evilz,
        Color::black(),
    )));

    matrix *= modifier_matrix;

    assert_eq!(matrix.get(0).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(0).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(1).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(1).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(2).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(2).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(3).unwrap().get(0).unwrap().text, "HELP");
    assert_eq!(matrix.get(3).unwrap().get(1).unwrap().text, "######");
    assert_eq!(matrix.get(4).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(5).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(6).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(7).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(8).unwrap().get(0).unwrap().text, "##########");
    assert_eq!(matrix.get(9).unwrap().get(0).unwrap().text, "##########");
}

fn create_default_matrix() -> GlyphMatrix {
    let mut matrix = GlyphMatrix::new();

    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));
    matrix.push(GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::Evilz,
        Color::white(),
    )));

    return matrix;
}

#[test]
pub fn test_line_add_assign_1() {
    line_add_assign_1();
}

// Pretty straight forward, simple test case
pub fn line_add_assign_1() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::text(
        "##########",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "**********",
        AppFont::AppleTea,
        Color::black(),
    ));

    let mut modifier_line = GlyphLine::new();
    modifier_line.push(GlyphComponent::text(
        "!!!!!!!!!!!",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line += modifier_line;
    assert_eq!(glyph_line.get(0).unwrap().text, "!!!!!!!!!!!");
    assert_eq!(glyph_line.get(1).unwrap().text, "@@@@@@@@@");
    assert_eq!(glyph_line.get(2).unwrap().text, "**********");
}

#[test]
pub fn test_line_add_assign_2() {
    line_add_assign_2();
}

// Here we test the ability to ignore initial whitespace, while also respecting the indices
pub fn line_add_assign_2() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::text(
        "##########",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "@@@@@@@@@@",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "**********",
        AppFont::AppleTea,
        Color::black(),
    ));

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(20));
    modifier_line.push(GlyphComponent::text(
        "!!!!!!!!!!",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line += modifier_line;
    assert_eq!(glyph_line.get(0).unwrap().text, "##########");
    assert_eq!(glyph_line.get(1).unwrap().text, "@@@@@@@@@@");
    assert_eq!(glyph_line.get(2).unwrap().text, "!!!!!!!!!!");
}

#[test]
pub fn test_line_add_assign_3() {
    line_add_assign_3();
}

// Here we test the ability to handle emojis
pub fn line_add_assign_3() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::text(
        "##########",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "**********",
        AppFont::AppleTea,
        Color::black(),
    ));

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(20));
    modifier_line.push(GlyphComponent::text(
        "🍕🍕🍕🍕🍕🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line += modifier_line;
    assert_eq!(glyph_line.get(0).unwrap().text, "##########");
    assert_eq!(glyph_line.get(1).unwrap().text, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻");
    assert_eq!(glyph_line.get(2).unwrap().text, "🍕🍕🍕🍕🍕🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻");
}

#[test]
pub fn test_line_add_assign_4() {
    line_add_assign_4();
}

pub fn line_add_assign_4() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::text(
        "##############################",
        AppFont::AppleTea,
        Color::black(),
    ));

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(10));
    modifier_line.push(GlyphComponent::text(
        "!!!!!!!!!!",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line += modifier_line;
    assert_eq!(glyph_line.get(0).unwrap().text, "##########");
    assert_eq!(glyph_line.get(1).unwrap().text, "!!!!!!!!!!");
    assert_eq!(glyph_line.get(2).unwrap().text, "##########");
}
#[test]
pub fn test_component_of_index() {
    component_of_index();
}

pub fn component_of_index() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(10)); //0
    glyph_line.push(GlyphComponent::space(10)); //1
    glyph_line.push(GlyphComponent::space(10)); //2
    let a = glyph_line.component_of_index(0);
    assert_eq!(a, 0);
    let b = glyph_line.component_of_index(9);
    assert_eq!(b, 0);
    let c = glyph_line.component_of_index(10);
    assert_eq!(c, 1);
    let d = glyph_line.component_of_index(15);
    assert_eq!(d, 1);
    let e = glyph_line.component_of_index(20);
    assert_eq!(e, 2);
    let f = glyph_line.component_of_index(29);
    assert_eq!(f, 2);
}

#[test]
pub fn test_index_of_component() {
    index_of_component();
}

pub fn index_of_component() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(10)); //0
    glyph_line.push(GlyphComponent::space(10)); //1
    glyph_line.push(GlyphComponent::space(10)); //2
    let a = glyph_line.index_of_component(0);
    assert_eq!(a, 0);
    let b = glyph_line.index_of_component(1);
    assert_eq!(b, 10);
    let c = glyph_line.index_of_component(2);
    assert_eq!(c, 20);
}

#[test]
pub fn test_expanding_insert_1() {
    expanding_insert_1();
}

pub fn expanding_insert_1() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(10)); //0
    glyph_line.push(GlyphComponent::text("12", AppFont::AppleTea, Color::black())); //1
    glyph_line.push(GlyphComponent::space(10)); //2
    assert_eq!(glyph_line.line.len(), 3);
    // This should now insert itself between the 1 and the 2
    glyph_line.expanding_insert(
        11,
        &GlyphComponent::text("onetwo", AppFont::Evilz, Color::black()),
    );
    // Two space comps + "1", and "2", and "onetwo" = 5
    assert_eq!(glyph_line.line.len(), 5);
    assert_eq!(glyph_line.get(1).unwrap().text, "1");
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(2).unwrap().text, "onetwo");
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::Evilz);
    assert_eq!(glyph_line.get(3).unwrap().text, "2");
    assert_eq!(glyph_line.get(3).unwrap().font, AppFont::AppleTea);
}

#[test]
pub fn test_expanding_insert_2() {
    expanding_insert_2();
}

pub fn expanding_insert_2() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(10)); //0
    glyph_line.push(GlyphComponent::text("12", AppFont::AppleTea, Color::black())); //1
    glyph_line.push(GlyphComponent::space(10)); //2
    assert_eq!(glyph_line.line.len(), 3);

    glyph_line.expanding_insert(
        10,
        &GlyphComponent::text("onetwo", AppFont::Evilz, Color::black()),
    );

    assert_eq!(glyph_line.line.len(), 4);
    assert_eq!(glyph_line.get(1).unwrap().text, "onetwo");
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Evilz);
    assert_eq!(glyph_line.get(2).unwrap().text, "12");
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AppleTea);
}

#[test]
pub fn test_expanding_insert_3() {
    expanding_insert_3();
}

pub fn expanding_insert_3() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(10)); //0
    glyph_line.push(GlyphComponent::text("12", AppFont::AppleTea, Color::black())); //1
    glyph_line.push(GlyphComponent::space(10)); //2
    assert_eq!(glyph_line.line.len(), 3);

    glyph_line.expanding_insert(
        12,
        &GlyphComponent::text("onetwo", AppFont::Evilz, Color::black()),
    );

    assert_eq!(glyph_line.line.len(), 4);

    assert_eq!(glyph_line.get(1).unwrap().text, "12");
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(2).unwrap().text, "onetwo");
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::Evilz);
}

#[test]
pub fn test_expanding_insert_4() {
    expanding_insert_4();
}

pub fn expanding_insert_4() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.expanding_insert(0, &GlyphComponent::text("123", AppFont::African, Color::black()));
    assert_eq!(glyph_line.line.len(), 1);
    assert_eq!(glyph_line.line.get(0).unwrap().text, "123");
}

#[test]
pub fn test_expanding_insert_5() {
    expanding_insert_5();
}

pub fn expanding_insert_5() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.expanding_insert(10, &GlyphComponent::text("123", AppFont::African, Color::black()));
    assert_eq!(glyph_line.line.len(), 2);
    assert_eq!(
        glyph_line.line.get(0).unwrap().text,
        GlyphComponent::space(10).text
    );
    assert_eq!(glyph_line.line.get(1).unwrap().text, "123");
}

#[test]
pub fn test_expanding_insert_6() {
    expanding_insert_6();
}

pub fn expanding_insert_6() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(5));
    glyph_line.expanding_insert(10, &GlyphComponent::text("123", AppFont::African, Color::black()));
    assert_eq!(glyph_line.line.len(), 2);
    assert_eq!(
        glyph_line.line.get(0).unwrap().text,
        GlyphComponent::space(10).text
    );
    assert_eq!(glyph_line.line.get(1).unwrap().text, "123");
}

#[test]
pub fn test_expanding_insert_7() {
    expanding_insert_7();
}

pub fn expanding_insert_7() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(10));
    glyph_line.expanding_insert(10, &GlyphComponent::text("123", AppFont::African, Color::black()));
    assert_eq!(glyph_line.line.len(), 2);
    assert_eq!(
        glyph_line.line.get(0).unwrap().text,
        GlyphComponent::space(10).text
    );
    assert_eq!(glyph_line.line.get(1).unwrap().text, "123");
}

#[test]
pub fn test_overriding_insert_1() {
    overriding_insert_1();
}

pub fn overriding_insert_1() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(20)); //0
    glyph_line.push(GlyphComponent::space(20)); //1
    glyph_line.push(GlyphComponent::space(20)); //2
    assert_eq!(glyph_line.line.len(), 3);
    glyph_line.overriding_insert(10, &GlyphComponent::space(40));
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().length(), 10);
    assert_eq!(glyph_line.get(1).unwrap().length(), 40);
    assert_eq!(glyph_line.get(2).unwrap().length(), 10);
}
#[test]
pub fn test_overriding_insert_2() {
    overriding_insert_2();
}

pub fn overriding_insert_2() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(20)); //0
    glyph_line.push(GlyphComponent::space(20)); //1
    glyph_line.push(GlyphComponent::space(20)); //2
    glyph_line.push(GlyphComponent::space(20)); //3
    glyph_line.push(GlyphComponent::space(20)); //4
    glyph_line.push(GlyphComponent::space(20)); //5
    assert_eq!(glyph_line.line.len(), 6);
    glyph_line.overriding_insert(10, &GlyphComponent::space(100));
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().length(), 10);
    assert_eq!(glyph_line.get(1).unwrap().length(), 100);
    assert_eq!(glyph_line.get(2).unwrap().length(), 10);
}

#[test]
pub fn test_overriding_insert_3() {
    overriding_insert_3();
}

pub fn overriding_insert_3() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(20)); //0
    glyph_line.push(GlyphComponent::space(20)); //1
    glyph_line.push(GlyphComponent::space(20)); //2
    assert_eq!(glyph_line.line.len(), 3);
    glyph_line.overriding_insert(60, &GlyphComponent::space(40));
    assert_eq!(glyph_line.line.len(), 4);
    assert_eq!(glyph_line.get(0).unwrap().length(), 20);
    assert_eq!(glyph_line.get(1).unwrap().length(), 20);
    assert_eq!(glyph_line.get(2).unwrap().length(), 20);
    assert_eq!(glyph_line.get(3).unwrap().length(), 40);
}

#[test]
pub fn test_overriding_insert_4() {
    overriding_insert_4();
}

pub fn overriding_insert_4() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(20)); //0
    glyph_line.push(GlyphComponent::space(20)); //1
    glyph_line.push(GlyphComponent::space(20)); //2
    assert_eq!(glyph_line.line.len(), 3);
    glyph_line.overriding_insert(0, &GlyphComponent::space(40));
    assert_eq!(glyph_line.line.len(), 2);
    assert_eq!(glyph_line.get(0).unwrap().length(), 40);
    assert_eq!(glyph_line.get(1).unwrap().length(), 20);
}

#[test]
pub fn test_overriding_insert_5() {
    overriding_insert_5();
}

pub fn overriding_insert_5() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(20)); //0
    glyph_line.push(GlyphComponent::space(20)); //1
    glyph_line.push(GlyphComponent::space(20)); //2
    assert_eq!(glyph_line.line.len(), 3);
    glyph_line.overriding_insert(0, &GlyphComponent::space(20));
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().length(), 20);
    assert_eq!(glyph_line.get(1).unwrap().length(), 20);
    assert_eq!(glyph_line.get(2).unwrap().length(), 20);
}

#[test]
pub fn test_overriding_insert_6() {
    overriding_insert_6();
}

pub fn overriding_insert_6() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(20)); //0
    glyph_line.push(GlyphComponent::space(20)); //1
    glyph_line.push(GlyphComponent::space(20)); //2
    assert_eq!(glyph_line.line.len(), 3);
    for _i in 0..3639 {
        glyph_line.overriding_insert(0, &GlyphComponent::space(20));
    }
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().length(), 20);
    assert_eq!(glyph_line.get(1).unwrap().length(), 20);
    assert_eq!(glyph_line.get(2).unwrap().length(), 20);
}

#[test]
pub fn test_overriding_insert_7() {
    overriding_insert_7();
}

pub fn overriding_insert_7() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::space(1)); //0
    glyph_line.push(GlyphComponent::space(1)); //1
    glyph_line.push(GlyphComponent::space(1)); //2
    assert_eq!(glyph_line.line.len(), 3);
    glyph_line.overriding_insert(3, &GlyphComponent::space(1));
    assert_eq!(glyph_line.line.len(), 4);

    glyph_line.overriding_insert(4, &GlyphComponent::space(1));
    assert_eq!(glyph_line.line.len(), 5);

    glyph_line.overriding_insert(5, &GlyphComponent::space(1));
    assert_eq!(glyph_line.line.len(), 6);

    glyph_line.overriding_insert(6, &GlyphComponent::space(1));
    assert_eq!(glyph_line.line.len(), 7);

    glyph_line.overriding_insert(7, &GlyphComponent::space(1));
    assert_eq!(glyph_line.line.len(), 8);

    glyph_line.overriding_insert(8, &GlyphComponent::space(1));
    assert_eq!(glyph_line.line.len(), 9);

    assert_eq!(glyph_line.get(0).unwrap().length(), 1);
    assert_eq!(glyph_line.get(1).unwrap().length(), 1);
    assert_eq!(glyph_line.get(2).unwrap().length(), 1);
    assert_eq!(glyph_line.get(3).unwrap().length(), 1);
    assert_eq!(glyph_line.get(4).unwrap().length(), 1);
    assert_eq!(glyph_line.get(5).unwrap().length(), 1);
    assert_eq!(glyph_line.get(6).unwrap().length(), 1);
    assert_eq!(glyph_line.get(7).unwrap().length(), 1);
    assert_eq!(glyph_line.get(8).unwrap().length(), 1);
}

#[test]
pub fn test_overriding_insert_8() {
    overriding_insert_8();
}

pub fn overriding_insert_8() {
    let mut glyph_line = GlyphLine::new();

    glyph_line.push(GlyphComponent::text(
        "0123456789",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "abcdefghij",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    assert_eq!(glyph_line.line.len(), 2);
    glyph_line.overriding_insert(10, &GlyphComponent::text("x🙏🏻🍕", AppFont::Any, Color::black()));
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456789");
    assert_eq!(glyph_line.get(1).unwrap().as_str(), "x🙏🏻🍕");
    assert_eq!(glyph_line.get(2).unwrap().as_str(), "defghij");
    assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AliceInWonderland);
}

#[test]
pub fn test_overriding_insert_9() {
    overriding_insert_9();
}

pub fn overriding_insert_9() {
    let mut glyph_line = GlyphLine::new();

    glyph_line.push(GlyphComponent::text(
        "0123456789",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "abcdefghij",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    assert_eq!(glyph_line.line.len(), 2);
    glyph_line.overriding_insert(7, &GlyphComponent::text("x🙏🏻🍕", AppFont::Any, Color::black()));
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456");
    assert_eq!(glyph_line.get(1).unwrap().as_str(), "x🙏🏻🍕");
    assert_eq!(glyph_line.get(2).unwrap().as_str(), "abcdefghij");
    assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AliceInWonderland);
    assert_eq!(glyph_line.get(0).unwrap().color, Color::black());
    assert_eq!(glyph_line.get(1).unwrap().color, Color::black());
    assert_eq!(glyph_line.get(2).unwrap().color, Color::white());
}

#[test]
pub fn test_overriding_insert_10() {
    overriding_insert_10();
}

pub fn overriding_insert_10() {
    let mut glyph_line = GlyphLine::new();

    glyph_line.push(GlyphComponent::text(
        "0123456789",
        AppFont::AppleTea,
        Color::black(),
    ));
    glyph_line.push(GlyphComponent::text(
        "abcdefghij",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line.push(GlyphComponent::text(
        "Ook? Ook! Ook? Ook.",
        AppFont::Evilz,
        Color::black(),
    ));
    assert_eq!(glyph_line.line.len(), 3);
    glyph_line.overriding_insert(
        7,
        &GlyphComponent::text("Nanananananananana", AppFont::Any, Color::black()),
    );

    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456");
    assert_eq!(glyph_line.get(1).unwrap().as_str(), "Nanananananananana");
    assert_eq!(glyph_line.get(2).unwrap().as_str(), "Ook! Ook? Ook.");
    assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::Evilz);
    assert_eq!(glyph_line.get(0).unwrap().color, Color::black());
    assert_eq!(glyph_line.get(1).unwrap().color, Color::black());
    assert_eq!(glyph_line.get(2).unwrap().color, Color::black());
}

#[test]
pub fn test_overriding_insert_11() {
    overriding_insert_11();
}

pub fn overriding_insert_11() {
    let mut glyph_line = GlyphLine::new();

    glyph_line.push(GlyphComponent::text(
        "0123456789",
        AppFont::AppleTea,
        Color::black(),
    ));
    assert_eq!(glyph_line.line.len(), 1);
    glyph_line.overriding_insert(10, &GlyphComponent::text("10", AppFont::Any, Color::black()));
    assert_eq!(glyph_line.line.len(), 2);
    assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456789");
    assert_eq!(glyph_line.get(1).unwrap().as_str(), "10");
}

#[test]
pub fn test_overriding_insert_12() {
    overriding_insert_12();
}

pub fn overriding_insert_12() {
    let mut glyph_line = GlyphLine::new();

    glyph_line.push(GlyphComponent::text(
        "0123456789",
        AppFont::AppleTea,
        Color::black(),
    ));
    assert_eq!(glyph_line.line.len(), 1);
    glyph_line.overriding_insert(
        11,
        &GlyphComponent::text("10", AppFont::AlphaMusicMan, Color::black()),
    );
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456789");
    assert_eq!(glyph_line.get(1).unwrap().as_str(), " ");
    assert_eq!(glyph_line.get(2).unwrap().as_str(), "10");
    assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
    assert_eq!(glyph_line.get(1).unwrap().color, Color::invisible());
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AlphaMusicMan);
    assert_eq!(glyph_line.get(2).unwrap().color, Color::black());
}

#[test]
pub fn test_overriding_insert_13() {
    overriding_insert_13();
}

pub fn overriding_insert_13() {
    let mut glyph_line = GlyphLine::new();

    glyph_line.push(GlyphComponent::text(
        "0123456789",
        AppFont::AppleTea,
        Color::black(),
    ));
    assert_eq!(glyph_line.line.len(), 1);
    glyph_line.overriding_insert(15, &GlyphComponent::text("10", AppFont::AppleTea, Color::black()));
    assert_eq!(glyph_line.line.len(), 3);
    assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456789");
    assert_eq!(glyph_line.get(1).unwrap().as_str(), "     ");
    assert_eq!(glyph_line.get(2).unwrap().as_str(), "10");
    assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
    assert_eq!(glyph_line.get(1).unwrap().color, Color::invisible());
    assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AppleTea);
    assert_eq!(glyph_line.get(2).unwrap().color, Color::black());
}
// ── Mutation-surface completeness: model deltas (P1-10) ─────────────

/// `GlyphModelField::GlyphLine` deltas must apply through the
/// `Applicable` path, not just when hand-applied. Regression for the
/// missing branch in `GlyphModel::apply_operation`.
#[test]
pub fn test_delta_glyph_line_assign_applies_through_apply_to() {
    do_delta_glyph_line_assign_applies_through_apply_to();
}

pub fn do_delta_glyph_line_assign_applies_through_apply_to() {
    let mut model = GlyphModel::new();
    model.add_line(GlyphLine::new_with(GlyphComponent::text(
        "first",
        AppFont::AppleTea,
        Color::black(),
    )));

    let replacement = GlyphLine::new_with(GlyphComponent::text("replaced", AppFont::Evilz, Color::white()));
    let delta = DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Assign),
        GlyphModelField::GlyphLine(1, replacement),
    ]);

    delta.apply_to(&mut model);

    assert_eq!(model.glyph_matrix.matrix.len(), 2);
    assert_eq!(
        model.glyph_matrix.get(0).unwrap().get(0).unwrap().as_str(),
        "first"
    );
    assert_eq!(
        model.glyph_matrix.get(1).unwrap().get(0).unwrap().as_str(),
        "replaced"
    );
}

/// `GlyphModelField::GlyphLines` deltas must apply through the
/// `Applicable` path. Mirrors the single-line regression test above.
#[test]
pub fn test_delta_glyph_lines_assign_applies_through_apply_to() {
    do_delta_glyph_lines_assign_applies_through_apply_to();
}

pub fn do_delta_glyph_lines_assign_applies_through_apply_to() {
    let mut model = GlyphModel::new();

    let lines = vec![
        (
            0,
            GlyphLine::new_with(GlyphComponent::text(
                "line zero",
                AppFont::AppleTea,
                Color::black(),
            )),
        ),
        (
            2,
            GlyphLine::new_with(GlyphComponent::text("line two", AppFont::Evilz, Color::white())),
        ),
    ];
    let delta = DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Assign),
        GlyphModelField::GlyphLines(lines),
    ]);

    delta.apply_to(&mut model);

    assert_eq!(model.glyph_matrix.matrix.len(), 3);
    assert_eq!(
        model.glyph_matrix.get(0).unwrap().get(0).unwrap().as_str(),
        "line zero"
    );
    assert_eq!(model.glyph_matrix.get(1).unwrap().line.len(), 0);
    assert_eq!(
        model.glyph_matrix.get(2).unwrap().get(0).unwrap().as_str(),
        "line two"
    );
}

/// `ApplyOperation::Delete` on a `GlyphLine` delta clears the target
/// line. Part of the per-field operation-table work for P1-10.
#[test]
pub fn test_delta_glyph_line_delete_clears_line() {
    do_delta_glyph_line_delete_clears_line();
}

pub fn do_delta_glyph_line_delete_clears_line() {
    let mut model = GlyphModel::new();
    model.add_line(GlyphLine::new_with(GlyphComponent::text(
        "to clear",
        AppFont::AppleTea,
        Color::black(),
    )));

    let delta = DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Delete),
        GlyphModelField::GlyphLine(0, GlyphLine::new()),
    ]);

    delta.apply_to(&mut model);

    assert_eq!(model.glyph_matrix.get(0).unwrap().line.len(), 0);
}

/// Destructive operations on a single-line delta must not grow an
/// absent row. They mirror `GlyphMatrix::{sub,mul}_assign`, where
/// missing rows fall away.
#[test]
pub fn test_delta_glyph_line_destructive_ops_do_not_grow_missing_line() {
    do_delta_glyph_line_destructive_ops_do_not_grow_missing_line();
}

pub fn do_delta_glyph_line_destructive_ops_do_not_grow_missing_line() {
    for operation in [
        ApplyOperation::Subtract,
        ApplyOperation::Multiply,
        ApplyOperation::Delete,
    ] {
        let mut model = GlyphModel::new();
        model.add_line(GlyphLine::new_with(GlyphComponent::text(
            "kept",
            AppFont::AppleTea,
            Color::black(),
        )));

        let delta = DeltaGlyphModel::new(vec![
            GlyphModelField::Operation(operation),
            GlyphModelField::GlyphLine(
                3,
                GlyphLine::new_with(GlyphComponent::text("stale", AppFont::Any, Color::white())),
            ),
        ]);

        delta.apply_to(&mut model);

        assert_eq!(model.glyph_matrix.matrix.len(), 1);
        assert_eq!(
            model.glyph_matrix.get(0).unwrap().get(0).unwrap().as_str(),
            "kept"
        );
    }
}

/// Multi-line deltas use the same absent-row rule as single-line
/// deltas for destructive operations.
#[test]
pub fn test_delta_glyph_lines_destructive_ops_do_not_grow_missing_lines() {
    do_delta_glyph_lines_destructive_ops_do_not_grow_missing_lines();
}

pub fn do_delta_glyph_lines_destructive_ops_do_not_grow_missing_lines() {
    for operation in [
        ApplyOperation::Subtract,
        ApplyOperation::Multiply,
        ApplyOperation::Delete,
    ] {
        let mut model = GlyphModel::new();
        model.add_line(GlyphLine::new_with(GlyphComponent::text(
            "kept",
            AppFont::AppleTea,
            Color::black(),
        )));

        let delta = DeltaGlyphModel::new(vec![
            GlyphModelField::Operation(operation),
            GlyphModelField::GlyphLines(vec![(
                3,
                GlyphLine::new_with(GlyphComponent::text("stale", AppFont::Any, Color::white())),
            )]),
        ]);

        delta.apply_to(&mut model);

        assert_eq!(model.glyph_matrix.matrix.len(), 1);
        assert_eq!(
            model.glyph_matrix.get(0).unwrap().get(0).unwrap().as_str(),
            "kept"
        );
    }
}

/// `Subtract` on the layer field must saturate at zero instead of
/// underflowing `usize`. Regression for the raw `operation.apply`
/// path in `GlyphModel::apply_operation`.
#[test]
pub fn test_model_layer_subtract_saturates_at_zero() {
    do_model_layer_subtract_saturates_at_zero();
}

pub fn do_model_layer_subtract_saturates_at_zero() {
    let mut model = GlyphModel::new();
    model.layer = 2;

    let delta = DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Subtract),
        GlyphModelField::Layer(5),
    ]);

    delta.apply_to(&mut model);

    assert_eq!(model.layer, 0);
}

/// A `Noop` matrix delta must leave the target matrix untouched, and
/// a `Delete` matrix delta must clear it — both without reading the
/// payload. Regression for `DeltaGlyphModel::glyph_matrix` returning
/// a deep clone on every apply (P1-24): the borrow-plus-`apply_ref`
/// path only clones on the arms that consume the payload, so these
/// two operations must still produce exactly the documented result.
#[test]
pub fn test_delta_glyph_matrix_noop_and_delete_ignore_payload() {
    do_delta_glyph_matrix_noop_and_delete_ignore_payload();
}

pub fn do_delta_glyph_matrix_noop_and_delete_ignore_payload() {
    let payload = || {
        let mut matrix = GlyphMatrix::new();
        matrix.push(GlyphLine::new_with(GlyphComponent::text(
            "payload",
            AppFont::AppleTea,
            Color::white(),
        )));
        matrix
    };

    let mut noop_model = GlyphModel::new();
    noop_model.add_line(GlyphLine::new_with(GlyphComponent::text(
        "original",
        AppFont::AppleTea,
        Color::black(),
    )));
    DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Noop),
        GlyphModelField::GlyphMatrix(payload()),
    ])
    .apply_to(&mut noop_model);
    assert_eq!(noop_model.glyph_matrix.matrix.len(), 1);
    assert_eq!(
        noop_model.glyph_matrix.get(0).unwrap().get(0).unwrap().as_str(),
        "original"
    );

    let mut delete_model = GlyphModel::new();
    delete_model.add_line(GlyphLine::new_with(GlyphComponent::text(
        "original",
        AppFont::AppleTea,
        Color::black(),
    )));
    DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Delete),
        GlyphModelField::GlyphMatrix(payload()),
    ])
    .apply_to(&mut delete_model);
    assert!(delete_model.glyph_matrix.matrix.is_empty());
}

/// The same delta applies more than once with the same result — the
/// borrowing accessors hand out references, so nothing about the
/// delta is consumed by an apply.
#[test]
pub fn test_delta_glyph_matrix_applies_repeatedly() {
    do_delta_glyph_matrix_applies_repeatedly();
}

pub fn do_delta_glyph_matrix_applies_repeatedly() {
    let mut payload = GlyphMatrix::new();
    payload.push(GlyphLine::new_with(GlyphComponent::text(
        "assigned",
        AppFont::AppleTea,
        Color::white(),
    )));

    let delta = DeltaGlyphModel::new(vec![
        GlyphModelField::Operation(ApplyOperation::Assign),
        GlyphModelField::GlyphMatrix(payload.clone()),
    ]);

    // The accessors borrow, so the payload is still readable between
    // applies and identical afterwards.
    assert_eq!(delta.glyph_matrix(), Some(&payload));

    let mut first = GlyphModel::new();
    let mut second = GlyphModel::new();
    delta.apply_to(&mut first);
    delta.apply_to(&mut second);

    assert_eq!(first.glyph_matrix, payload);
    assert_eq!(second.glyph_matrix, payload);
    assert_eq!(delta.glyph_matrix(), Some(&payload));
}

/// `GlyphMatrix::add_assign` moves surplus rhs rows into self rather
/// than cloning them, and keeps the row order. Locks the
/// `into_iter()` rewrite (the previous `insert(i, ..)` would panic if
/// a caller ever grew self non-contiguously).
#[test]
pub fn test_matrix_add_assign_absorbs_taller_rhs() {
    do_matrix_add_assign_absorbs_taller_rhs();
}

pub fn do_matrix_add_assign_absorbs_taller_rhs() {
    let mut matrix = GlyphMatrix::new();
    let mut rhs = GlyphMatrix::new();
    for text in ["one", "two", "three"] {
        rhs.push(GlyphLine::new_with(GlyphComponent::text(
            text,
            AppFont::AppleTea,
            Color::black(),
        )));
    }

    matrix += rhs;

    assert_eq!(matrix.matrix.len(), 3);
    assert_eq!(matrix.get(0).unwrap().get(0).unwrap().as_str(), "one");
    assert_eq!(matrix.get(1).unwrap().get(0).unwrap().as_str(), "two");
    assert_eq!(matrix.get(2).unwrap().get(0).unwrap().as_str(), "three");
}

/// `GlyphMatrix::{sub,mul}_assign` drop rhs rows that self does not
/// have, and leave self's row count alone.
#[test]
pub fn test_matrix_destructive_assigns_drop_surplus_rhs_rows() {
    do_matrix_destructive_assigns_drop_surplus_rhs_rows();
}

pub fn do_matrix_destructive_assigns_drop_surplus_rhs_rows() {
    for subtract in [true, false] {
        let mut matrix = GlyphMatrix::new();
        matrix.push(GlyphLine::new_with(GlyphComponent::text(
            "keep",
            AppFont::AppleTea,
            Color::black(),
        )));

        let mut rhs = GlyphMatrix::new();
        for text in ["a", "b", "c"] {
            rhs.push(GlyphLine::new_with(GlyphComponent::text(
                text,
                AppFont::AppleTea,
                Color::black(),
            )));
        }

        if subtract {
            matrix -= rhs;
        } else {
            matrix *= rhs;
        }

        assert_eq!(matrix.matrix.len(), 1);
    }
}

/// `GlyphComponent::add_assign` appends the rhs run in place. Locks
/// the `push_str` rewrite that removed a whole-string clone per
/// concatenation.
#[test]
pub fn test_component_add_assign_appends_text() {
    do_component_add_assign_appends_text();
}

pub fn do_component_add_assign_appends_text() {
    let mut left = GlyphComponent::text("héllo ", AppFont::AppleTea, Color::black());
    let right = GlyphComponent::text("wörld🌍", AppFont::Evilz, Color::black());

    left += right.clone();
    assert_eq!(left.text, "héllo wörld🌍");

    // An empty rhs must not touch the lhs text.
    let mut untouched = GlyphComponent::text("stay", AppFont::AppleTea, Color::black());
    untouched += GlyphComponent::text("", AppFont::AppleTea, Color::black());
    assert_eq!(untouched.text, "stay");

    // `Add` reuses the same path without cloning the lhs first.
    let summed = GlyphComponent::text("a", AppFont::AppleTea, Color::black()) + right;
    assert_eq!(summed.text, "awörld🌍");
}

/// `GlyphModelCommand::Rotate` swings the model's position around a
/// displaced pivot. Uses the geometry epsilon rather than bit
/// equality — the trig is `f32`, so `model_block_commands` can only
/// assert the pivot-is-the-position fixed point exactly.
#[test]
pub fn test_model_rotate_moves_position_around_pivot() {
    do_model_rotate_moves_position_around_pivot();
}

pub fn do_model_rotate_moves_position_around_pivot() {
    use crate::gfx_structs::model::GlyphModelCommand;
    use crate::util::geometry::almost_equal_vec2;
    use glam::Vec2;

    let mut model = GlyphModel::new();
    model.position = crate::util::ordered_vec2::OrderedVec2::new_f32(10.0, 0.0);

    GlyphModelCommand::Rotate {
        pivot: Vec2::ZERO,
        degrees: 90.0,
    }
    .apply_to(&mut model);

    // Clockwise 90° takes `+x` onto `-y` (screen space has `+y` down).
    assert!(almost_equal_vec2(model.position.to_vec2(), Vec2::new(0.0, -10.0)));
}

/////////////////////////////////////////////////////////////////
// `GlyphLine::perform_op` — the `ignore_initial_space` path.
//
// Every case below reached `perform_op` through a real `*Assign`
// operator, which is how the mutation pipeline reaches it: a
// `ModelDelta` carrying a `GlyphLine`/`GlyphMatrix` payload runs
// `ApplyOperation::apply_ref` straight into these impls, and the
// payload is deserialized from user-authored JSON — so
// `ignore_initial_space` is reachable from a `.mindmap.json`
// without a single Rust caller setting it.
/////////////////////////////////////////////////////////////////

/// Per-run text of a line, in order — the shape the
/// `ignore_initial_space` assertions compare against.
fn line_component_texts(line: &GlyphLine) -> Vec<&str> {
    line.line.iter().map(|comp| comp.text.as_str()).collect()
}

/// A ten-cluster ASCII line to paint onto.
fn hash_line() -> GlyphLine {
    GlyphLine::new_with(GlyphComponent::text(
        "##########",
        AppFont::AppleTea,
        Color::black(),
    ))
}

/// A one-component rhs whose own leading whitespace must be counted
/// into the paint offset but not written.
fn indented_rhs(text: &str) -> GlyphLine {
    let mut line = GlyphLine::new_with(GlyphComponent::text(
        text,
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    line.ignore_initial_space = true;
    line
}

#[test]
pub fn test_line_ignore_initial_space_multibyte_indent() {
    do_line_ignore_initial_space_multibyte_indent();
}

/// Four U+3000 IDEOGRAPHIC SPACEs: cluster 4, byte 12. Feeding the
/// old `char` ordinal to the byte-indexed `String::split_off` landed
/// mid-codepoint and panicked. The content must land at cluster 4.
pub fn do_line_ignore_initial_space_multibyte_indent() {
    let mut glyph_line = hash_line();
    glyph_line += indented_rhs("\u{3000}\u{3000}\u{3000}\u{3000}!!!");

    assert_eq!(line_component_texts(&glyph_line), vec!["####", "!!!", "###"]);
    assert_eq!(glyph_line.length(), 10);
}

#[test]
pub fn test_line_ignore_initial_space_crlf_indent() {
    do_line_ignore_initial_space_crlf_indent();
}

/// CRLF is one grapheme cluster made of two chars, so an indent of
/// `"\r\n  "` is cluster 3 but char ordinal 4. The old code paid the
/// char ordinal into a cluster-indexed insert and painted one column
/// too far right.
pub fn do_line_ignore_initial_space_crlf_indent() {
    let mut glyph_line = hash_line();
    glyph_line += indented_rhs("\r\n  🍕🍕🍕");

    assert_eq!(line_component_texts(&glyph_line), vec!["###", "🍕🍕🍕", "####"]);
    assert_eq!(glyph_line.length(), 10);
}

#[test]
pub fn test_line_ignore_initial_space_zwj_content() {
    do_line_ignore_initial_space_zwj_content();
}

/// A ZWJ family sequence is one cluster of eight chars. The offset
/// must stay in clusters on both sides of the split so the painted
/// run occupies two columns, not eight.
pub fn do_line_ignore_initial_space_zwj_content() {
    let mut glyph_line = hash_line();
    glyph_line += indented_rhs("  👨‍👩‍👧🍕");

    assert_eq!(line_component_texts(&glyph_line), vec!["##", "👨‍👩‍👧🍕", "######"]);
    assert_eq!(glyph_line.length(), 10);
}

#[test]
pub fn test_line_ignore_initial_space_sub_assign_rhs_longer_than_lhs() {
    do_line_ignore_initial_space_sub_assign_rhs_longer_than_lhs();
}

/// `self` and `rhs` need not agree on how their text is cut into
/// runs, so the rhs run index may name no lhs run at all. The
/// `SubAssign` color arm indexed `self.line[i]` unguarded and
/// panicked; it now falls back to the rhs color.
pub fn do_line_ignore_initial_space_sub_assign_rhs_longer_than_lhs() {
    let mut glyph_line = GlyphLine::new_with(GlyphComponent::text("##", AppFont::AppleTea, Color::black()));

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(2));
    modifier_line.push(GlyphComponent::text(
        "!!",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line -= modifier_line;

    assert_eq!(line_component_texts(&glyph_line), vec!["##", "!!"]);
    assert_eq!(glyph_line.get(1).unwrap().color, Color::white());
}

#[test]
pub fn test_line_ignore_initial_space_mul_assign_rhs_longer_than_lhs() {
    do_line_ignore_initial_space_mul_assign_rhs_longer_than_lhs();
}

/// Same shape as the `SubAssign` case for the `MulAssign` arm, which
/// carried the identical unguarded index.
pub fn do_line_ignore_initial_space_mul_assign_rhs_longer_than_lhs() {
    let mut glyph_line = GlyphLine::new_with(GlyphComponent::text("##", AppFont::AppleTea, Color::black()));

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(2));
    modifier_line.push(GlyphComponent::text(
        "!!",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line *= modifier_line;

    assert_eq!(line_component_texts(&glyph_line), vec!["##", "!!"]);
    assert_eq!(glyph_line.get(1).unwrap().color, Color::white());
}

#[test]
pub fn test_line_ignore_initial_space_sub_assign_uses_lhs_color_when_present() {
    do_line_ignore_initial_space_sub_assign_uses_lhs_color_when_present();
}

/// The guard must not swallow the arm it guards: when the lhs *does*
/// have a run at that index, `SubAssign` still subtracts the rhs
/// color from it rather than taking the fallback.
pub fn do_line_ignore_initial_space_sub_assign_uses_lhs_color_when_present() {
    let mut glyph_line = GlyphLine::new();
    glyph_line.push(GlyphComponent::text("ab", AppFont::AppleTea, Color::white()));
    glyph_line.push(GlyphComponent::text("cd", AppFont::AppleTea, Color::white()));

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(2));
    modifier_line.push(GlyphComponent::text(
        "!!",
        AppFont::AliceInWonderland,
        Color::black(),
    ));
    glyph_line -= modifier_line;

    let painted = glyph_line
        .line
        .iter()
        .find(|comp| comp.text == "!!")
        .expect("the rhs content run must have been painted");
    assert_eq!(painted.color, Color::white() - Color::black());
}

#[test]
pub fn test_line_ignore_initial_space_surplus_rhs_runs_append() {
    do_line_ignore_initial_space_surplus_rhs_runs_append();
}

/// With the leading-space runs skipped, the trailing loop starts at a
/// component index the lhs may be several slots short of.
/// `Vec::insert(i, …)` panics there; surplus runs now append, the
/// same rule `GlyphMatrix::add_assign` applies to surplus rows.
pub fn do_line_ignore_initial_space_surplus_rhs_runs_append() {
    let mut glyph_line = GlyphLine::new();

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(1));
    modifier_line.push(GlyphComponent::space(1));
    modifier_line.push(GlyphComponent::space(1));
    modifier_line.push(GlyphComponent::text(
        "A",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    modifier_line.push(GlyphComponent::text(
        "B",
        AppFont::AliceInWonderland,
        Color::white(),
    ));
    glyph_line += modifier_line;

    assert_eq!(line_component_texts(&glyph_line), vec!["   ", "A", "B"]);
}

#[test]
pub fn test_line_ignore_initial_space_all_whitespace_rhs_paints_nothing() {
    do_line_ignore_initial_space_all_whitespace_rhs_paints_nothing();
}

/// Every run of an all-whitespace rhs is leading whitespace, so under
/// the flag's contract the whole rhs is transparent. It used to be
/// opaque — the same run blanked the lhs when nothing followed it and
/// showed through when something did.
pub fn do_line_ignore_initial_space_all_whitespace_rhs_paints_nothing() {
    let mut glyph_line = hash_line();

    let mut modifier_line = GlyphLine::new();
    modifier_line.ignore_initial_space = true;
    modifier_line.push(GlyphComponent::space(2));
    modifier_line.push(GlyphComponent::space(2));
    glyph_line += modifier_line;

    assert_eq!(line_component_texts(&glyph_line), vec!["##########"]);
}
