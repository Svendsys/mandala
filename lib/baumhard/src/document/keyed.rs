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

use std::borrow::Borrow;
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

/// An index from the names a file chose — verb ids, presentation and
/// edit ids, theme variables, palettes — to what they name: a lookup is
/// one keyed probe, never a scan of a list whose length the file chose
/// (D29; ruling D3.3 item 2 on #252). Where two entries share a name,
/// the first inserted keeps it, as a scan in order would find it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NameIndex<V> {
    map: KeyedMap<NameKey, V>,
}

impl<V> Default for NameIndex<V> {
    fn default() -> Self {
        NameIndex {
            map: KeyedMap::default(),
        }
    }
}

impl<V: Copy> NameIndex<V> {
    /// Index `name` as `value`, unless an earlier entry has the name.
    /// O(the name's length).
    pub fn insert_first(&mut self, name: &str, value: V) {
        self.map.entry(key(name)).or_insert(value);
    }

    /// What `name` names, if anything. One probe: O(the name's length).
    /// One body in every build: the probe compares through
    /// `NameKey`'s [`Borrow`], which counts in a test build.
    pub fn get(&self, name: &str) -> Option<V> {
        self.map.get(name).copied()
    }

    /// Whether no name is indexed. O(1).
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Room for `more` names besides those indexed.
    pub fn reserve(&mut self, more: usize) {
        self.map.reserve(more);
    }
}

/// A key of a [`NameIndex`]: the name, owned. It hashes and compares as
/// the `str` it holds, and is looked up by one ([`Borrow`]). In a test
/// build every compare is counted (`NAMES_COMPARED`): a probe compares a
/// candidate through `borrow` (the hash map's `Equivalent`), and an
/// insert through `PartialEq` — about one for a hash probe, as many as
/// there are names for a scan of the keys, whoever wrote the scan
/// (rulings D3.3 item 3 and D3.4 item 4 on #252).
#[derive(Clone, Debug, Eq)]
pub(crate) struct NameKey(Box<str>);

/// `name` as a [`NameIndex`] key.
fn key(name: &str) -> NameKey {
    NameKey(Box::from(name))
}

impl std::hash::Hash for NameKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl Borrow<str> for NameKey {
    fn borrow(&self) -> &str {
        // The hash map calls this once a candidate it compares.
        #[cfg(test)]
        NAMES_COMPARED.with(|count| count.set(count.get() + 1));
        &self.0
    }
}

impl PartialEq for NameKey {
    fn eq(&self, other: &Self) -> bool {
        #[cfg(test)]
        NAMES_COMPARED.with(|count| count.set(count.get() + 1));
        self.0 == other.0
    }
}

#[cfg(test)]
thread_local! {
    /// Test counter (rulings D3.3 item 3 and D3.4 item 4 on #252): how
    /// many times a [`NameIndex`]'s keys, and the ids and names a lookup
    /// could scan ([`Name`]), have been compared on this thread.
    pub(crate) static NAMES_COMPARED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// An id or a name a lookup by name could scan instead of probing an
/// index: a verb's, a presentation's and an edit's id, a theme
/// variable's and a palette's name. A `String` — and in a test build a
/// string whose every compare is counted (`counted::CountedName`), so a
/// test holding a lookup to its compares sees a scan of the list as well
/// as one of the index (ruling D3.4 item 4 on #252).
#[cfg(not(test))]
pub type Name = String;

#[cfg(test)]
pub use counted::CountedName as Name;

/// `name` as a [`Name`]: how every place that makes one makes it, the
/// same in a test build, where it is counted, as in any other.
#[allow(
    clippy::useless_conversion,
    reason = "a `String` to a `String` outside a test build, where `Name` is one"
)]
pub fn name(name: impl Into<String>) -> Name {
    Name::from(name.into())
}

/// The test build's [`Name`].
#[cfg(test)]
mod counted {
    use super::NAMES_COMPARED;
    use std::borrow::Borrow;
    use std::fmt;
    use std::ops::Deref;

    /// A `String` that counts every compare made through it
    /// (`NAMES_COMPARED`): what a test build stores an id or a name as.
    #[derive(Clone, Default, PartialOrd, Ord)]
    pub struct CountedName(String);

    impl std::hash::Hash for CountedName {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.0.hash(state);
        }
    }

    fn counted() {
        NAMES_COMPARED.with(|count| count.set(count.get() + 1));
    }

    impl Deref for CountedName {
        type Target = String;
        fn deref(&self) -> &String {
            &self.0
        }
    }

    impl Borrow<str> for CountedName {
        fn borrow(&self) -> &str {
            &self.0
        }
    }

    impl AsRef<str> for CountedName {
        fn as_ref(&self) -> &str {
            &self.0
        }
    }

    impl fmt::Debug for CountedName {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Debug::fmt(&self.0, f)
        }
    }

    impl fmt::Display for CountedName {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Display::fmt(&self.0, f)
        }
    }

    impl From<String> for CountedName {
        fn from(name: String) -> Self {
            CountedName(name)
        }
    }

    impl From<&str> for CountedName {
        fn from(name: &str) -> Self {
            CountedName(name.to_string())
        }
    }

    impl From<&String> for CountedName {
        fn from(name: &String) -> Self {
            CountedName(name.clone())
        }
    }

    impl From<CountedName> for String {
        fn from(name: CountedName) -> Self {
            name.0
        }
    }

    impl Eq for CountedName {}

    impl PartialEq for CountedName {
        fn eq(&self, other: &Self) -> bool {
            counted();
            self.0 == other.0
        }
    }

    impl PartialEq<str> for CountedName {
        fn eq(&self, other: &str) -> bool {
            counted();
            self.0 == other
        }
    }

    impl PartialEq<&str> for CountedName {
        fn eq(&self, other: &&str) -> bool {
            counted();
            self.0 == *other
        }
    }

    impl PartialEq<String> for CountedName {
        fn eq(&self, other: &String) -> bool {
            counted();
            self.0 == *other
        }
    }

    impl PartialEq<CountedName> for str {
        fn eq(&self, other: &CountedName) -> bool {
            counted();
            self == other.0
        }
    }

    impl PartialEq<CountedName> for &str {
        fn eq(&self, other: &CountedName) -> bool {
            counted();
            *self == other.0
        }
    }

    impl PartialEq<CountedName> for String {
        fn eq(&self, other: &CountedName) -> bool {
            counted();
            *self == other.0
        }
    }
}
