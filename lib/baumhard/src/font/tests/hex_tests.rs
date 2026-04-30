// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::font::hex::hex_to_cosmic_color`]. The
//! underlying `[f32; 4]` parser lives in
//! [`crate::util::color_conversion`] and is covered by
//! `util/tests/color_tests.rs`; the tests here pin the
//! cosmic-text colour boundary specifically — `f32 → u8` rounding
//! at channel edges (00 / 80 / ff) and the alpha pass-through
//! through the eight-digit form.

use crate::font::hex::hex_to_cosmic_color;

#[test]
fn test_hex_to_cosmic_color_round_trip() {
    do_hex_to_cosmic_color_round_trip();
}

/// `#ff0000` lands as `cosmic_text::Color::rgba(255, 0, 0, 255)`.
/// Round-tripping every channel boundary (00, 80, ff) catches
/// off-by-one errors in the f32→u8 rounding step.
pub fn do_hex_to_cosmic_color_round_trip() {
    assert_eq!(
        hex_to_cosmic_color("#ff0000").unwrap(),
        cosmic_text::Color::rgba(255, 0, 0, 255)
    );
    assert_eq!(
        hex_to_cosmic_color("#000000").unwrap(),
        cosmic_text::Color::rgba(0, 0, 0, 255)
    );
    assert_eq!(
        hex_to_cosmic_color("#ffffff").unwrap(),
        cosmic_text::Color::rgba(255, 255, 255, 255)
    );
    // Shorthand path: `#f00` must produce the same colour as `#ff0000`.
    assert_eq!(
        hex_to_cosmic_color("#f00").unwrap(),
        hex_to_cosmic_color("#ff0000").unwrap()
    );
    // Eight-digit form carries alpha through. `0x80` = 128 catches
    // the rounding-trap point where naive `as u8` from `0.5019…` could
    // truncate to 127.
    assert_eq!(
        hex_to_cosmic_color("#05638f80").unwrap(),
        cosmic_text::Color::rgba(5, 99, 143, 128)
    );
    // Garbage rejects rather than substituting.
    assert!(hex_to_cosmic_color("not-a-color").is_none());
    assert!(hex_to_cosmic_color("#ggg").is_none());
}
