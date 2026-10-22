// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::util::shortest_float`].

use crate::util::shortest_float::shortest_f64;

#[test]
fn test_shortest_f64_prints_and_round_trips_like_the_f32() {
    do_shortest_f64_prints_and_round_trips_like_the_f32();
}

/// The widened value prints the `f32`'s own shortest decimal and parses
/// back to the same `f32`, across ordinary, tiny, huge and negative
/// values; a non-finite value has none.
pub fn do_shortest_f64_prints_and_round_trips_like_the_f32() {
    for value in [
        0.1_f32,
        7.325_424_7,
        -340.5,
        190.483_23,
        1.0e-40,
        f32::MIN_POSITIVE,
        -f32::MAX,
        0.0,
    ] {
        let shortest = shortest_f64(value).expect("finite");
        assert_eq!(shortest.to_string(), value.to_string(), "{value}");
        assert_eq!(shortest as f32, value, "{value}");
    }
    assert_ne!(
        f64::from(0.1_f32).to_string(),
        "0.1",
        "the widening this exists to avoid"
    );
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(shortest_f64(value), None);
    }
}
