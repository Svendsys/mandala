// SPDX-License-Identifier: MPL-2.0

use crate::util::primes::{get_primes, is_prime, PRIME_CEILING};
use std::collections::HashSet;

#[test]
pub fn test_primes() {
    do_primes()
}

/// Cross-check `is_prime` against the cached sieve for every value
/// `0..=PRIME_CEILING`: every value that the sieve emits as prime
/// must report `true`, every other value must report `false`. Pins
/// the sieve / lookup contract on both sides (positive and
/// negative).
pub fn do_primes() {
    let primes_set: HashSet<usize> = get_primes().into_iter().collect();
    for n in 0..=PRIME_CEILING {
        assert_eq!(is_prime(n), primes_set.contains(&n), "is_prime mismatch for {n}");
    }
}
