// SPDX-License-Identifier: MPL-2.0

//! The choice source behind the structure-aware fuzz targets, held to
//! its encoding (TEST_CONVENTIONS §T14).

use crate::util::tests::choices::{encode, Choices};

#[test]
fn test_choices_read_back_what_was_encoded_and_zero_after() {
    let bytes = encode(&[7, 3, u32::MAX]);
    let mut choices = Choices::new(&bytes);
    assert_eq!(choices.below(10), 7);
    assert_eq!(choices.below(2), 1, "3 mod 2");
    assert!(!choices.exhausted(), "one choice is left");
    assert_eq!(choices.next_u32(), u32::MAX);
    assert!(choices.exhausted());
    assert_eq!(choices.below(10), 0, "an exhausted input reads as zero");
    assert!(!choices.one_in(4), "zero is not the one");
    assert!(choices.one_in(1));
    // A trailing partial word is zero-padded, not dropped.
    let mut partial = Choices::new(&[1, 2]);
    assert_eq!(partial.next_u32(), 0x0201);
}

#[test]
fn test_a_seeded_source_is_deterministic_and_never_exhausted() {
    let draw = |seed| {
        let mut choices = Choices::seeded(seed);
        (0..8).map(|_| choices.next_u32()).collect::<Vec<_>>()
    };
    assert_eq!(draw(5), draw(5));
    assert_ne!(draw(5), draw(6));
    assert!(!Choices::seeded(0).exhausted());
    assert_ne!(draw(0), vec![0; 8], "a zero seed does not stick at zero");
    // What a seeded source drew replays from bytes, choice for choice.
    let mut seeded = Choices::seeded(9);
    let drawn: Vec<u32> = (0..5).map(|_| seeded.below(1000)).collect();
    let bytes = seeded.replay();
    let mut replayed = Choices::new(&bytes);
    assert_eq!((0..5).map(|_| replayed.below(1000)).collect::<Vec<_>>(), drawn);
    assert!(replayed.exhausted());
}
