// SPDX-License-Identifier: MPL-2.0

//! Per-`(face, font_size_pt, grapheme)` measured glyph metrics.
//!
//! Replaces the static `MONOSPACE_ADVANCE_RATIO = 0.6` /
//! `BORDER_APPROX_CHAR_WIDTH_FRAC = 0.6` approximations that the
//! border-rail math used for "how wide is one cluster". Those
//! approximations were calibrated against LiberationSans light
//! box-drawing chars; on every other glyph (`◆`, `━`, `┃`, `=`,
//! `#`, etc.) and on every other face the approximation diverged
//! from what cosmic-text actually shaped, producing the
//! alignment + tiling defects users see in the Border Toolkit
//! demo on `maps/testament.mindmap.json`.
//!
//! The fix is structural: every callsite that asks "how wide
//! will this glyph end up?" or "how tall will this row of glyphs
//! be?" routes through this cache. The cache returns the value
//! cosmic-text actually uses when shaping, so the math + the
//! layout agree at sub-pixel precision.
//!
//! ## Cache discipline
//!
//! - Key: `(Option<AppFont>, OrderedFloat<f32>, String)` — the
//!   `Option<AppFont>` carries the face pin (None = cosmic-text's
//!   default fallback face); the `String` is the grapheme cluster
//!   ("│", "◆·", etc.) — multi-grapheme clusters shape together
//!   so the cache key has to preserve them as a unit.
//! - Hit: read-locked `RwLock`, O(1); no `FONT_SYSTEM` access.
//! - Miss (unlocked caller): acquires the `FONT_SYSTEM` write
//!   guard through `acquire_font_system_write` — the timeout-
//!   guarded helper, never a raw `.write()` (CONVENTIONS §B5) —
//!   shapes the cluster through cosmic-text, stores the result.
//!   Subsequent calls for the same key hit the cache.
//! - Miss (caller already holding the guard): use the
//!   `*_with(&mut FontSystem, ...)` variants, which shape through
//!   the passed guard instead of acquiring a second one. The
//!   renderer's border-rebuild loop holds the write guard across
//!   the whole loop and MUST use these — a nested same-thread
//!   acquire is a guaranteed deadlock. This mirrors the
//!   `measure_glyph_ink_bounds` / `measure_text_block_unbounded`
//!   composable design in `font/fonts.rs`.
//! - Invalidation: implicit. When the user swaps the active
//!   font, the new `AppFont` discriminator produces a different
//!   cache key; old entries become dead memory until process
//!   exit. Acceptable — every entry is ~12 bytes.
//!
//! ## Why not just measure inline at every call site?
//!
//! `border_run_specs` runs per visible node per scene rebuild.
//! Shaping a single cluster through cosmic-text takes ~100µs
//! (allocate scratch buffer, set_text, shape_until_scroll). With
//! ~12 unique clusters per node and N visible nodes, an
//! uncached pass would cost N × 12 × 100µs = 12 ms / 10 nodes
//! per rebuild. The cache reduces hot-path lookups to a
//! `HashMap` read (~100 ns each), giving a ~1000× speedup on
//! re-renders.
//!
//! ## Public API
//!
//! - [`glyph_advance`](crate::font::metric_cache::glyph_advance) —
//!   horizontal advance of a single grapheme cluster (used by
//!   horizontal-rail char-count math).
//! - [`glyph_ink`](crate::font::metric_cache::glyph_ink) — the full
//!   ink extent (height plus top offset) of a single cluster, which
//!   is what the vertical-rail line-height math actually reads.

use cosmic_text::SwashCache;
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use lazy_static::lazy_static;
use ordered_float::OrderedFloat;
use rustc_hash::FxHashMap;
use std::sync::{Mutex, RwLock};
use std::time::Duration;

use crate::font::fonts::{
    acquire_font_system_write, acquire_font_system_write_with_timeout, ensure_warm, face_family_name_for_pin,
    measure_glyph_ink_bounds, AppFont,
};

type CacheKey = (Option<AppFont>, OrderedFloat<f32>, String);

/// Ink extent of one grapheme cluster at a given face + size.
///
/// `advance` is the horizontal advance (same value the
/// `glyph_advance` cache returns; included here for the
/// `glyph_ink` callers who want both together without two
/// cache lookups).
///
/// `ink_height` is the vertical pixel span the rasterized
/// glyph occupies (`y_max − y_min` from
/// `measure_glyph_ink_bounds`). For a corner glyph this is
/// the value the renderer uses as the corner buffer's height
/// AND as the side-rail's vertical offset from the node's
/// top/bottom edges. For a fill grapheme this is the value
/// the vertical rail uses as its `line_height` — using this
/// makes consecutive cluster rows TOUCH (no inter-row gap
/// from the font's larger em-height).
///
/// `ink_top` is the y_min from `measure_glyph_ink_bounds` —
/// signed offset from the glyph's baseline to the topmost
/// ink pixel. Negative for ink above baseline. The renderer
/// uses this to compute the buffer's `position.y` so the
/// ink's top edge lands at the target pixel.
#[derive(Copy, Clone, Debug)]
pub struct InkExtent {
    pub advance: f32,
    pub ink_height: f32,
    pub ink_top: f32,
}

lazy_static! {
    static ref ADVANCE_CACHE: RwLock<FxHashMap<CacheKey, f32>> =
        RwLock::new(FxHashMap::default());
    static ref INK_EXTENT_CACHE: RwLock<FxHashMap<CacheKey, InkExtent>> =
        RwLock::new(FxHashMap::default());
    /// Singleton `SwashCache` for the `glyph_ink` measurement
    /// path. `measure_glyph_ink_bounds` requires a mutable
    /// `SwashCache` to rasterise glyphs; we hold one process-
    /// lifetime and reuse it across all `glyph_ink` cache misses.
    /// Behind a `Mutex` because cosmic-text's `SwashCache` is
    /// `!Sync`; reads-only-on-hit paths consult `INK_EXTENT_CACHE`
    /// directly without acquiring this lock.
    static ref SWASH_CACHE: Mutex<SwashCache> = Mutex::new(SwashCache::new());
}

/// Width (in pt) of `grapheme` when shaped by cosmic-text
/// against `face` at `size_pt`. Returns the sum of `glyph.w`
/// across every layout glyph the cluster produces (multi-
/// grapheme clusters like `◆·` shape as a unit).
///
/// `face = None` uses cosmic-text's default fallback face —
/// same shape every other shaping site that doesn't pin a
/// family takes.
///
/// First call per `(face, size_pt, grapheme)` shapes through
/// cosmic-text and caches. Subsequent calls return the cached
/// value. The cache is process-lifetime.
pub fn glyph_advance(face: Option<AppFont>, size_pt: f32, grapheme: &str) -> f32 {
    glyph_advance_with_timeout(
        face,
        size_pt,
        grapheme,
        crate::font::fonts::FONT_SYSTEM_LOCK_TIMEOUT,
    )
}

/// [`glyph_advance`]'s shared body, parameterized by the acquire
/// timeout — the public wrapper calls this with the standard
/// `FONT_SYSTEM_LOCK_TIMEOUT` budget. The `acquire_timeout` parameter
/// exists so the re-entrancy regression test can drive the timeout
/// path on a short budget instead of waiting the full production one;
/// it mirrors the `acquire_font_system_write` /
/// `acquire_font_system_write_with_timeout` pair in `fonts.rs`.
/// `pub(crate)` — not public API surface.
pub(crate) fn glyph_advance_with_timeout(
    face: Option<AppFont>,
    size_pt: f32,
    grapheme: &str,
    acquire_timeout: Duration,
) -> f32 {
    // Fast path: a cache hit needs no `FONT_SYSTEM` access at all.
    let key = (face, OrderedFloat(size_pt), grapheme.to_string());
    if let Ok(cache) = ADVANCE_CACHE.read() {
        if let Some(&v) = cache.get(&key) {
            return v;
        }
    }
    // Miss: warm the font lazy-statics BEFORE acquiring so a
    // `Some(face)` pin lookup under the guard can't re-enter
    // `load_fonts` (see `ensure_warm`), then acquire through the
    // timeout-guarded helper (never a raw `.write()`) and shape.
    ensure_warm();
    let mut guard = acquire_font_system_write_with_timeout("metric_cache::glyph_advance", acquire_timeout);
    glyph_advance_with(&mut guard, face, size_pt, grapheme)
}

/// [`glyph_advance`] for callers that already hold the `FONT_SYSTEM`
/// write guard (the renderer's border-rebuild loop). Shapes any
/// cache miss through the passed `font_system` instead of acquiring
/// a second guard — the composable design §B5 and the
/// `measure_glyph_ink_bounds` primitive share.
pub fn glyph_advance_with(
    font_system: &mut FontSystem,
    face: Option<AppFont>,
    size_pt: f32,
    grapheme: &str,
) -> f32 {
    let key = (face, OrderedFloat(size_pt), grapheme.to_string());
    if let Ok(cache) = ADVANCE_CACHE.read() {
        if let Some(&v) = cache.get(&key) {
            return v;
        }
    }
    let measured = shape_advance_with(font_system, face, size_pt, grapheme);
    if let Ok(mut cache) = ADVANCE_CACHE.write() {
        cache.insert(key, measured);
    }
    measured
}

/// Sum of `glyph_advance` for each grapheme cluster in
/// `cluster`. Multi-grapheme clusters that ARE single graphemes
/// in some scripts still get summed per-grapheme here; for
/// proper kerning callers should call `glyph_advance` directly
/// on the whole cluster as a single string.
///
/// Convenience for the border-rail math where the side pattern's
/// `cluster: Vec<String>` field is already split per grapheme.
pub fn cluster_width(face: Option<AppFont>, size_pt: f32, graphemes: &[String]) -> f32 {
    graphemes.iter().map(|g| glyph_advance(face, size_pt, g)).sum()
}

/// Full ink extent of `grapheme` at `face` × `size_pt`:
/// advance + ink_height + ink_top (signed baseline offset).
///
/// Cache: read-locked hit ≈ 100 ns; miss acquires the `FONT_SYSTEM`
/// write guard (via `acquire_font_system_write`) plus
/// `SWASH_CACHE.lock()` to rasterise the glyph through
/// `measure_glyph_ink_bounds`. Once-per-(face, size, grapheme) cost.
/// Callers already holding the guard must use [`glyph_ink_with`].
///
/// Returns a defensive fallback (`advance` from the cheaper
/// advance-only path, `ink_height = size_pt`, `ink_top =
/// -size_pt × 0.75`) if rasterisation produces no ink — this
/// happens for whitespace, control characters, or missing
/// glyphs. The fallback values match what the prior
/// approximation produced, so callers downstream don't see a
/// regression on degenerate glyphs.
pub fn glyph_ink(face: Option<AppFont>, size_pt: f32, grapheme: &str) -> InkExtent {
    // Fast path: a cache hit needs no `FONT_SYSTEM` access.
    let key = (face, OrderedFloat(size_pt), grapheme.to_string());
    if let Ok(cache) = INK_EXTENT_CACHE.read() {
        if let Some(&v) = cache.get(&key) {
            return v;
        }
    }
    // Warm before acquiring (see `ensure_warm`) so a `Some(face)` pin
    // lookup under the guard can't re-enter `load_fonts`.
    ensure_warm();
    let mut guard = acquire_font_system_write("metric_cache::glyph_ink");
    glyph_ink_with(&mut guard, face, size_pt, grapheme)
}

/// [`glyph_ink`] for callers that already hold the `FONT_SYSTEM`
/// write guard (the renderer's border-rebuild loop). Rasterises any
/// cache miss through the passed `font_system` — plus its own
/// `SWASH_CACHE.lock()`, which is a distinct lock and so re-entrancy-
/// safe — instead of acquiring a second `FONT_SYSTEM` guard.
pub fn glyph_ink_with(
    font_system: &mut FontSystem,
    face: Option<AppFont>,
    size_pt: f32,
    grapheme: &str,
) -> InkExtent {
    let key = (face, OrderedFloat(size_pt), grapheme.to_string());
    if let Ok(cache) = INK_EXTENT_CACHE.read() {
        if let Some(&v) = cache.get(&key) {
            return v;
        }
    }
    let measured = shape_ink_extent_with(font_system, face, size_pt, grapheme);
    if let Ok(mut cache) = INK_EXTENT_CACHE.write() {
        cache.insert(key, measured);
    }
    measured
}

fn shape_ink_extent_with(
    font_system: &mut FontSystem,
    face: Option<AppFont>,
    size_pt: f32,
    grapheme: &str,
) -> InkExtent {
    let mut swash_guard = SWASH_CACHE
        .lock()
        .expect("SWASH_CACHE poisoned in metric_cache::shape_ink_extent");
    let bounds = measure_glyph_ink_bounds(font_system, &mut swash_guard, face, grapheme, size_pt);
    let ink_height = (bounds.y_max - bounds.y_min).max(0.0);
    if ink_height > 0.0 && bounds.advance > 0.0 {
        InkExtent {
            advance: bounds.advance,
            ink_height,
            ink_top: bounds.y_min,
        }
    } else {
        // Defensive fallback for whitespace / tofu / missing
        // glyphs. Matches the prior approximation's defaults
        // so callers see no behavioural regression on
        // degenerate input.
        InkExtent {
            advance: if bounds.advance > 0.0 {
                bounds.advance
            } else {
                size_pt * 0.6
            },
            ink_height: size_pt,
            ink_top: -size_pt * 0.75,
        }
    }
}

fn shape_advance_with(
    font_system: &mut FontSystem,
    face: Option<AppFont>,
    size_pt: f32,
    grapheme: &str,
) -> f32 {
    let mut buffer = Buffer::new(font_system, Metrics::new(size_pt, size_pt));
    let family_name: Option<String> = face.and_then(|f| face_family_name_for_pin(font_system, f));
    let attrs = match family_name.as_deref() {
        Some(name) => Attrs::new().family(Family::Name(name)),
        None => Attrs::new(),
    };
    buffer.set_text(font_system, grapheme, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    let mut total = 0.0f32;
    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            total += glyph.w;
        }
    }
    total
}
