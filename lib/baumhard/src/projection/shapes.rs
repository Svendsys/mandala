// SPDX-License-Identifier: MPL-2.0

//! The projector's shaping cache (V1-B14, #304; ruling B7.4 on #241).
//!
//! The projector shapes per unit, and each unit keeps its shaping while
//! its key is unchanged (#241 §3). Before this cache, N connectors or
//! labels or sections with one key made N shapings of it. Shaping is a
//! pure function of the key — the spans' texts, their resolved styles with
//! every size in points, and for a box its minimum size and alignment —
//! over the font system's faces, so every unit whose key is equal to a
//! live unit's takes that unit's shaping, the same `Arc`, and shapes
//! nothing.
//!
//! **What it holds.** Each entry is a key and a weak reference to the
//! shaping some unit holds: the cache keeps no shaping alive. A shaping
//! no unit holds any longer is gone, and its entry is dropped at the next
//! sweep, which runs when the entries have doubled since the last one, so
//! the cache holds at most twice as many entries as there are live
//! shapings, plus a floor. The key is the unit's own, shared (`Arc`), so
//! an entry copies no text.
//!
//! **The face set.** A shaping is also a function of the faces installed.
//! The font system registers them once, at `fonts::init`, before anything
//! is shaped (the precondition of every shaping call), and never changes
//! them after; the cache records the faces' generation
//! ([`face_set_generation`](crate::font::fonts::face_set_generation))
//! and lets everything go if it ever moves.
//!
//! **The font lock.** A hit reads no font, so an update whose every key
//! is cached takes no lock; a miss shapes through the lease the caller
//! holds, as before.

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Weak};

use rustc_hash::{FxHashMap, FxHasher};

use crate::document::resolve::ResolvedText;
use crate::document::values::FontSize;
use crate::flow::{ShapedBox, ShapedRun};
use crate::font::fonts::face_set_generation;
use crate::projection::state::{BoxKey, RunKey};

/// The fewest entries a sweep waits for: below it, sweeping would cost
/// more than the dead entries it frees.
const SWEEP_FLOOR: usize = 64;

/// A key's hash: of every field equality compares, sizes by their bits.
/// Two equal keys hash equal; a `NaN` size, which equals nothing, only
/// ever misses.
trait KeyHash {
    fn key_hash(&self) -> u64;
}

/// Feed one span to `state`.
fn hash_span(text: &str, style: &ResolvedText, state: &mut FxHasher) {
    text.hash(state);
    style.font.hash(state);
    match style.size {
        FontSize::Pt(points) => (0u8, points.to_bits()).hash(state),
        FontSize::Rel(fraction) => (1u8, fraction.to_bits()).hash(state),
    }
    style.color.hash(state);
    style.weight.hash(state);
    style.italic.hash(state);
}

impl KeyHash for RunKey {
    fn key_hash(&self) -> u64 {
        let mut state = FxHasher::default();
        for (text, style) in &self.spans {
            hash_span(text, style, &mut state);
        }
        state.finish()
    }
}

impl KeyHash for BoxKey {
    fn key_hash(&self) -> u64 {
        let mut state = FxHasher::default();
        for (text, style) in &self.spans {
            hash_span(text, style, &mut state);
        }
        self.min
            .map(|min| (min.x.to_bits(), min.y.to_bits()))
            .hash(&mut state);
        self.align.hash(&mut state);
        state.finish()
    }
}

/// The keys of one hash, and the shapings some unit holds for them.
type Bucket<K, V> = Vec<(Arc<K>, Weak<V>)>;

/// The shapings of one kind of flow, by key.
struct Shared<K, V> {
    /// By the key's hash: the keys with that hash and their shapings.
    entries: FxHashMap<u64, Bucket<K, V>>,
    /// Entries in all.
    len: usize,
    /// The entry count the next sweep waits for.
    sweep_at: usize,
    /// The face set's generation the entries were shaped over.
    faces: u64,
}

impl<K, V> Default for Shared<K, V> {
    fn default() -> Self {
        Shared {
            entries: FxHashMap::default(),
            len: 0,
            sweep_at: SWEEP_FLOOR,
            faces: face_set_generation(),
        }
    }
}

impl<K: KeyHash + PartialEq, V> Shared<K, V> {
    /// The live shaping of a key equal to `key`, or `shape()`'s, kept.
    ///
    /// Cost: one hash of the key (O(its text)), and one comparison per
    /// key of that hash; on a miss, `shape`; and now and then a sweep,
    /// O(entries), amortized O(1) a call.
    fn get_or_shape(&mut self, key: &Arc<K>, shape: impl FnOnce() -> Arc<V>) -> (Arc<V>, bool) {
        let faces = face_set_generation();
        if faces != self.faces {
            *self = Shared {
                faces,
                ..Shared::default()
            };
        }
        let hash = key.key_hash();
        let hit = self
            .entries
            .get(&hash)
            .and_then(|bucket| bucket.iter().find(|(kept, _)| **kept == **key))
            .and_then(|(_, shaped)| shaped.upgrade());
        if let Some(shaped) = hit {
            return (shaped, true);
        }
        let shaped = shape();
        let bucket = self.entries.entry(hash).or_default();
        let before = bucket.len();
        bucket.retain(|(kept, shaped)| shaped.strong_count() > 0 && **kept != **key);
        bucket.push((key.clone(), Arc::downgrade(&shaped)));
        self.len = self.len + bucket.len() - before;
        if self.len >= self.sweep_at {
            self.sweep();
        }
        (shaped, false)
    }

    /// Drop every entry whose shaping no unit holds.
    fn sweep(&mut self) {
        self.entries.retain(|_, bucket| {
            bucket.retain(|(_, shaped)| shaped.strong_count() > 0);
            !bucket.is_empty()
        });
        self.len = self.entries.values().map(Vec::len).sum();
        self.sweep_at = (2 * self.len).max(SWEEP_FLOOR);
    }
}

/// The projector's shaping cache: path flows' runs and box flows' boxes,
/// each by its full key. See the [module docs](self).
#[derive(Default)]
pub(crate) struct ShapeCache {
    runs: Shared<RunKey, ShapedRun>,
    boxes: Shared<BoxKey, ShapedBox>,
    /// Shapings a key equal to a live one's took instead of shaping:
    /// what the tests of the cache read. Test builds only.
    #[cfg(test)]
    pub(crate) hits: usize,
}

impl ShapeCache {
    /// The shaping of a run key equal to `key` some unit holds, or
    /// `shape()`'s.
    pub(crate) fn run(
        &mut self,
        key: &Arc<RunKey>,
        shape: impl FnOnce() -> Arc<ShapedRun>,
    ) -> Arc<ShapedRun> {
        let (shaped, _hit) = self.runs.get_or_shape(key, shape);
        #[cfg(test)]
        {
            self.hits += usize::from(_hit);
        }
        shaped
    }

    /// [`Self::run`] for a box key.
    pub(crate) fn boxed(
        &mut self,
        key: &Arc<BoxKey>,
        shape: impl FnOnce() -> Arc<ShapedBox>,
    ) -> Arc<ShapedBox> {
        let (shaped, _hit) = self.boxes.get_or_shape(key, shape);
        #[cfg(test)]
        {
            self.hits += usize::from(_hit);
        }
        shaped
    }

    /// Entries held, runs and boxes, live or not yet swept: what a test
    /// of the cache's bound reads. Test builds only.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.runs.len + self.boxes.len
    }
}
