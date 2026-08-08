// SPDX-License-Identifier: MPL-2.0

use crate::util::color::{from_hex, hex_to_rgba_safe};
use crate::util::color_conversion::hex_to_rgba;
use lazy_static::lazy_static;

/// Byte-quad → float-quad, the conversion the deleted `rgba!`
/// literal macro used to spell. Written out here rather than as a
/// crate macro because the controls below are the only callers left
/// and a test control should be readable at its use site.
const fn bytes_to_rgba(rgba: [u8; 4]) -> [f32; 4] {
    [
        rgba[0] as f32 / 255.0,
        rgba[1] as f32 / 255.0,
        rgba[2] as f32 / 255.0,
        rgba[3] as f32 / 255.0,
    ]
}

#[test]
pub fn test_from_hex() {
    do_from_hex();
}

/// Controls are byte quads rather than a second hex parse: a control
/// derived from the same parser under test proves only that the
/// parser agrees with itself.
pub fn do_from_hex() {
    let rgba = from_hex(&["f7b267", "f79d65", "f4845f", "f27059", "f25c54"]);
    let controls = [
        bytes_to_rgba([0xf7, 0xb2, 0x67, 255]),
        bytes_to_rgba([0xf7, 0x9d, 0x65, 255]),
        bytes_to_rgba([0xf4, 0x84, 0x5f, 255]),
        bytes_to_rgba([0xf2, 0x70, 0x59, 255]),
        bytes_to_rgba([0xf2, 0x5c, 0x54, 255]),
    ];
    assert_eq!(rgba.len(), controls.len());
    for (i, control) in controls.iter().enumerate() {
        assert_eq!(rgba.get(i).unwrap(), control, "entry {i}");
    }
}

lazy_static! {
    pub static ref CONTROL_1: [f32; 4] = bytes_to_rgba([0x05, 0x63, 0x8f, 255]);
    pub static ref CONTROL_2: [f32; 4] = bytes_to_rgba([0xdd, 0xbf, 0xfd, 255]);
    pub static ref CONTROL_3: [f32; 4] = bytes_to_rgba([0xba, 0x08, 0x4f, 255]);
    pub static ref CONTROL_4: [f32; 4] = bytes_to_rgba([0xfb, 0xa2, 0xc6, 255]);
    pub static ref RGBA_COLORS: Vec<[f32; 4]> = from_hex(&["#05638f", "ddbffd", "#ba084f", "#fba2c6"]);
}

#[test]
fn test_from_hex_lazy_static() {
    do_from_hex_lazy_static();
}

pub fn do_from_hex_lazy_static() {
    assert_eq!(RGBA_COLORS.len(), 4);
    assert_eq!(RGBA_COLORS.get(0).unwrap(), &CONTROL_1.clone());
    assert_eq!(RGBA_COLORS.get(1).unwrap(), &CONTROL_2.clone());
    assert_eq!(RGBA_COLORS.get(2).unwrap(), &CONTROL_3.clone());
    assert_eq!(RGBA_COLORS.get(3).unwrap(), &CONTROL_4.clone());
}

#[test]
fn test_from_hex_garbage_falls_back_to_black() {
    do_from_hex_garbage_falls_back_to_black();
}

/// Regression: bad hex strings must degrade to the fallback instead
/// of crashing. The valid entry in the middle ensures surrounding
/// items still parse correctly. The sentinel fallback `[0.42, …]`
/// distinguishes "returned the fallback" from "hardcoded black".
pub fn do_from_hex_garbage_falls_back_to_black() {
    // from_hex uses opaque-black as fallback internally.
    let rgba = from_hex(&["zzzzzz", "ff0000", "not-a-color", ""]);
    assert_eq!(rgba.len(), 4);
    assert_eq!(rgba[0], [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(rgba[1], [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(rgba[2], [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(rgba[3], [0.0, 0.0, 0.0, 1.0]);
    // Use a sentinel fallback to prove hex_to_rgba_safe actually
    // returns the caller's fallback rather than a hardcoded value.
    let sentinel = [0.42, 0.42, 0.42, 0.42];
    assert_eq!(hex_to_rgba_safe("garbage", sentinel), sentinel);
    assert_eq!(hex_to_rgba_safe("", sentinel), sentinel);
}

#[test]
fn test_hex_to_rgba_three_digit() {
    do_hex_to_rgba_three_digit();
}

/// `#f0a` expands per-nibble to `#ff00aa` with alpha pinned to 1.0.
/// Locks the canonical CSS-style 3-digit shorthand expansion.
pub fn do_hex_to_rgba_three_digit() {
    let parsed = hex_to_rgba("#f0a").unwrap();
    assert_eq!(parsed, [1.0, 0.0, 170.0 / 255.0, 1.0]);
    // Same string, no leading `#`.
    assert_eq!(hex_to_rgba("f0a").unwrap(), parsed);
}

#[test]
fn test_hex_to_rgba_four_digit() {
    do_hex_to_rgba_four_digit();
}

/// `#f0a8` expands per-nibble to `#ff00aa88` — alpha lifted from
/// the fourth nibble rather than pinned to 1.0.
pub fn do_hex_to_rgba_four_digit() {
    let parsed = hex_to_rgba("#f0a8").unwrap();
    assert_eq!(parsed, [1.0, 0.0, 170.0 / 255.0, 136.0 / 255.0]);
}

#[test]
fn test_hex_to_rgba_six_digit() {
    do_hex_to_rgba_six_digit();
}

/// `#05638f` is the same fixture used elsewhere in this file
/// (matches `hex!("#05638f")` and `rgba!([5, 99, 143, 255])`).
/// Six-digit form pins alpha to 1.0.
pub fn do_hex_to_rgba_six_digit() {
    let parsed = hex_to_rgba("#05638f").unwrap();
    assert_eq!(parsed, [5.0 / 255.0, 99.0 / 255.0, 143.0 / 255.0, 1.0]);
}

#[test]
fn test_hex_to_rgba_eight_digit() {
    do_hex_to_rgba_eight_digit();
}

/// Eight-digit form carries an explicit alpha byte. Tests an
/// unambiguous half-alpha (`80` = 128) so the channel can't be
/// confused with a default 255.
pub fn do_hex_to_rgba_eight_digit() {
    let parsed = hex_to_rgba("#05638f80").unwrap();
    assert_eq!(parsed, [5.0 / 255.0, 99.0 / 255.0, 143.0 / 255.0, 128.0 / 255.0]);
}

#[test]
fn test_hex_to_rgba_rejects_invalid_length() {
    do_hex_to_rgba_rejects_invalid_length();
}

/// 1, 2, 5, 7, 9 digit and empty inputs are not accepted lengths.
/// Each must round-trip to `None` so the fallible API contract holds.
pub fn do_hex_to_rgba_rejects_invalid_length() {
    assert!(hex_to_rgba("").is_none());
    assert!(hex_to_rgba("#").is_none());
    assert!(hex_to_rgba("#f").is_none());
    assert!(hex_to_rgba("#ff").is_none());
    assert!(hex_to_rgba("#ffff5").is_none());
    assert!(hex_to_rgba("#ffff55a").is_none());
    assert!(hex_to_rgba("#ffff55aa1").is_none());
}

#[test]
fn test_hex_to_rgba_rejects_non_hex_char() {
    do_hex_to_rgba_rejects_non_hex_char();
}

/// A non-hex byte anywhere in the body fails the parse — both
/// in the short-form path (length 3/4) and the byte-pair path
/// (length 6/8).
pub fn do_hex_to_rgba_rejects_non_hex_char() {
    assert!(hex_to_rgba("#zzz").is_none());
    assert!(hex_to_rgba("#zzzz").is_none());
    assert!(hex_to_rgba("#gg0000").is_none());
    assert!(hex_to_rgba("#ff00ZZ").is_none());
    assert!(hex_to_rgba("#deadbeef!").is_none()); // length-mismatch wins
    assert!(hex_to_rgba("#ff00 0a").is_none()); // embedded space
}

#[test]
fn test_is_valid_hex_color_matches_hex_parser() {
    use crate::util::color_conversion::is_valid_hex_color;

    for valid in ["#abc", "abc", "#abcd", "abcd", "#aabbcc", "aabbcc", "#aabbccdd"] {
        assert!(is_valid_hex_color(valid), "{valid:?} should be accepted");
    }
    for invalid in ["", "#", "#ab", "#abcde", "#abcdefghi", "#zzzzzz", "var(--accent)"] {
        assert!(!is_valid_hex_color(invalid), "{invalid:?} should be rejected");
    }
}

#[test]
fn test_parse_var_name_tolerates_inner_whitespace() {
    use crate::util::color_conversion::{is_var_ref, parse_var_name};

    assert_eq!(parse_var_name("var(--accent)"), Some("--accent"));
    assert_eq!(parse_var_name(" var( --bg ) "), Some("--bg"));
    assert!(is_var_ref("var( --fg )"));
}

#[test]
fn test_parse_var_name_rejects_malformed_refs() {
    use crate::util::color_conversion::{is_var_ref, parse_var_name};

    for invalid in [
        "var(--)",
        "var(accent)",
        "var(--accent)extra",
        "var(--foo(bar))",
        "var(--accent",
        "#abcdef",
    ] {
        assert_eq!(parse_var_name(invalid), None, "{invalid:?} should be rejected");
        assert!(!is_var_ref(invalid), "{invalid:?} should not be a var ref");
    }
}
