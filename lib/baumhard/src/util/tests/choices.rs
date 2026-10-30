// SPDX-License-Identifier: MPL-2.0

//! The choice source behind every structure-aware fuzz target
//! (TEST_CONVENTIONS §T14, design D29).
//!
//! A structure-aware target does not hand its bytes to a parser: it
//! reads them as a **sequence of choices** — how many nodes, which
//! parent, which fault, which number — and builds a value from those
//! choices that is valid wherever it chose to be, so the fuzzer spends
//! its time past the JSON syntax, in the typed, bounds and shape logic.
//! libFuzzer mutates the bytes and so the choices; coverage tells it
//! which choices reached new code.
//!
//! The decoder lives here, in baumhard, rather than in `fuzz/`, for one
//! reason: a finding is a byte string, and its regression test in
//! `./test.sh` must replay it through **the same** decoder on the pinned
//! stable toolchain. The same generators also run under a deterministic
//! [`Choices::seeded`] source in the ordinary suite, so the stable
//! toolchain exercises every generator and every property on every run,
//! not only the nightly one that drives the fuzzer.
//!
//! **Encoding.** Each choice reads the next four bytes as a
//! little-endian `u32`, and a choice among `n` takes it modulo `n`. An
//! exhausted input reads as zeros, so every generator must treat choice
//! `0` as "stop" or "the plainest option": a short input is a small
//! value, and the empty input is the smallest. That makes a seed easy to
//! write by hand ([`encode`]): the choices, four bytes each.

/// A sequence of choices: the bytes of a fuzz input, or a deterministic
/// generator for the suite's own sweeps.
pub struct Choices<'a> {
    from: From<'a>,
}

enum From<'a> {
    /// The fuzzer's input, read four bytes per choice.
    Bytes { bytes: &'a [u8], at: usize },
    /// A xorshift generator (Marsaglia 2003), for the stable sweeps.
    Seeded(u64),
}

impl<'a> Choices<'a> {
    /// Choices read from `bytes`, zero once they run out.
    pub fn new(bytes: &'a [u8]) -> Self {
        Choices {
            from: From::Bytes { bytes, at: 0 },
        }
    }

    /// Choices drawn from a deterministic generator seeded with `seed`
    /// (a zero seed is replaced, since xorshift stays at zero), for a
    /// sweep that must be the same on every run.
    pub fn seeded(seed: u64) -> Choices<'static> {
        Choices {
            from: From::Seeded(seed.max(1)),
        }
    }

    /// Whether a byte input has been read to its end; a seeded source
    /// never is. A generator that loops on choices checks this so that
    /// an exhausted input stops it rather than repeating its zero
    /// choice forever.
    pub fn exhausted(&self) -> bool {
        match &self.from {
            From::Bytes { bytes, at } => *at >= bytes.len(),
            From::Seeded(_) => false,
        }
    }

    /// The next raw choice.
    pub fn next_u32(&mut self) -> u32 {
        match &mut self.from {
            From::Bytes { bytes, at } => {
                let mut word = [0u8; 4];
                for (slot, byte) in word.iter_mut().zip(bytes.iter().skip(*at)) {
                    *slot = *byte;
                }
                *at = at.saturating_add(4);
                u32::from_le_bytes(word)
            }
            From::Seeded(state) => {
                *state ^= *state << 13;
                *state ^= *state >> 7;
                *state ^= *state << 17;
                (*state >> 32) as u32
            }
        }
    }

    /// A choice in `0..n`; `0` when `n` is `0`.
    pub fn below(&mut self, n: u32) -> u32 {
        let word = self.next_u32();
        if n == 0 {
            0
        } else {
            word % n
        }
    }

    /// A choice in `0..n` as a `usize`.
    pub fn index(&mut self, n: usize) -> usize {
        self.below(u32::try_from(n).unwrap_or(u32::MAX)) as usize
    }

    /// `true` with odds `1 in n` — so a zero choice, and an exhausted
    /// input, is always `true` for `n == 1` and otherwise `false`
    /// unless the choice was `n - 1`.
    pub fn one_in(&mut self, n: u32) -> bool {
        n > 0 && self.below(n) == n - 1
    }

    /// One of `items`, the first on a zero choice.
    pub fn pick<'t, T>(&mut self, items: &'t [T]) -> &'t T {
        &items[self.index(items.len())]
    }

    /// A raw `f32` from the next four bytes: every bit pattern, `NaN`s,
    /// infinities, subnormals and `-0.0` included.
    pub fn any_f32(&mut self) -> f32 {
        f32::from_bits(self.next_u32())
    }
}

/// The bytes a [`Choices`] reads back as exactly `choices`, in order:
/// how a seed of a structure-aware target is written by hand.
pub fn encode(choices: &[u32]) -> Vec<u8> {
    choices.iter().flat_map(|choice| choice.to_le_bytes()).collect()
}

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
}
