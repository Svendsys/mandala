// SPDX-License-Identifier: MPL-2.0

//! Hash maps keyed by what a stranger's file chose — node ids, verb
//! ids, names — hashed with a key the file cannot know (design §C.8).
//!
//! **Why not `FxHashMap`.** FxHash has no key: whoever writes a map can
//! pick ids or names whose hashes share their low bits, and a hash table
//! places entries by those bits, so every insert and lookup degrades to
//! a scan of the colliding cluster — a load quadratic in the file, from
//! the file alone (hash flooding, D29). A keyed hash makes the placement
//! unpredictable to anyone who does not know the key.
//!
//! **The hash** is SipHash-2-4 (`std::hash::SipHasher`), the keyed
//! function std's own `RandomState` is built on. std deprecates the
//! *type* in favor of `DefaultHasher`, whose algorithm it leaves
//! unspecified and which takes no key; this module needs a key it
//! chooses itself, which only the deprecated type takes.
//!
//! **The key** is drawn once per process from the operating system's
//! randomness through `getrandom` (`rand::rngs::SysRng`), on every
//! target. That matters on `wasm32-unknown-unknown`, where std's
//! `RandomState` has no source of randomness and uses fixed keys: there
//! `getrandom` reads the browser's `crypto.getRandomValues` (the
//! `wasm_js` backend the root manifest enables). Each map then takes its
//! own key from the process key and a counter, as std's does. If the
//! system has no randomness to give — which no supported target lacks —
//! the key falls back to std's `RandomState`, and a warning says so: a
//! load never fails for want of a hash key.
//!
//! **Output stays deterministic.** Nothing is ever written by iterating
//! one of these maps; the saver writes in tree order and sorted names.

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// The process's key, drawn once.
static PROCESS_KEY: OnceLock<(u64, u64)> = OnceLock::new();

/// How many [`KeyedState`]s this process has made: each takes its own
/// key from the process key and its number.
static MADE: AtomicU64 = AtomicU64::new(0);

/// The process key: 16 bytes from the system's randomness, or, should it
/// have none, from std's `RandomState`.
fn process_key() -> (u64, u64) {
    *PROCESS_KEY.get_or_init(|| {
        use rand::rngs::SysRng;
        use rand::TryRng;
        let mut bytes = [0u8; 16];
        match SysRng.try_fill_bytes(&mut bytes) {
            Ok(()) => {
                let (low, high) = bytes.split_at(8);
                let word = |half: &[u8]| {
                    let mut out = [0u8; 8];
                    out.copy_from_slice(half);
                    u64::from_le_bytes(out)
                };
                (word(low), word(high))
            }
            Err(error) => {
                log::warn!(
                    "document: no system randomness for the map index's hash key ({error}); \
                     falling back to std's RandomState"
                );
                let state = std::collections::hash_map::RandomState::new();
                (
                    state.hash_one(0x006d_616e_6461_6c61_u64),
                    state.hash_one(0x006b_6579_6564_u64),
                )
            }
        }
    })
}

/// A [`BuildHasher`] whose SipHash key the file cannot know. See the
/// module docs.
#[derive(Clone, Debug)]
pub struct KeyedState {
    k0: u64,
    k1: u64,
}

impl KeyedState {
    /// A state with a key of its own: the process key, varied by how
    /// many states came before. O(1); the first in a process draws the
    /// process key.
    pub fn new() -> Self {
        let (k0, k1) = process_key();
        let made = MADE.fetch_add(1, Ordering::Relaxed);
        KeyedState {
            k0: k0.wrapping_add(made),
            k1,
        }
    }
}

impl Default for KeyedState {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildHasher for KeyedState {
    // The only std hasher that takes a key; see the module docs.
    #[allow(deprecated)]
    type Hasher = std::hash::SipHasher;

    #[allow(deprecated)]
    fn build_hasher(&self) -> Self::Hasher {
        std::hash::SipHasher::new_with_keys(self.k0, self.k1)
    }
}

/// A hash map keyed by something a file chose.
pub type KeyedMap<K, V> = HashMap<K, V, KeyedState>;

/// A hash set of things a file chose.
pub type KeyedSet<T> = HashSet<T, KeyedState>;
