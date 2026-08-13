// SPDX-License-Identifier: MPL-2.0

//! Per-element re-shape cache for the screen-space overlay pass.
//!
//! The overlay sub-scene ([`AppScene`](crate::application::scene_host::AppScene)'s
//! console and color-picker trees) is re-walked after every mutator
//! apply. Walking it is cheap; *shaping* it is not — every walked
//! `GlyphArea` used to get a fresh `cosmic_text::Buffer`, a
//! `set_rich_text` and a `shape_until_scroll`, times `halos + 1`,
//! under one `FONT_SYSTEM` write guard, at mouse-move cadence. A
//! picker hover changes one cell's color; a console keystroke
//! changes one line's text. Neither is a reason to re-shape the
//! other fifty-odd elements.
//!
//! ## The reuse rule, and why it needs no notifications
//!
//! This cache does **not** subscribe to a dirty signal. It keeps,
//! per shaped element, a verbatim copy of every input the shaper
//! read, and re-validates against the live tree on each pass. An
//! element's buffers are reused only when all of the following still
//! hold at the same position in the walk:
//!
//! - the same overlay tree ([`SceneTreeId`]),
//! - the same `GfxElement::unique_id`,
//! - the same registered tree offset (the walker adds it to every
//!   emitted buffer's `pos`),
//! - a `GlyphArea` equal to the stored one — or, for an element that
//!   carries no area, still no area.
//!
//! That list is exactly what
//! [`shape_one_element_into_buffers`](super::tree_walker::shape_one_element_into_buffers)
//! reads: `element.glyph_area()`, `element.unique_id()`, and its
//! `offset` argument. Nothing else reaches the emitted buffers.
//!
//! The consequence worth naming: because the check compares against
//! live state rather than trusting a producer to announce itself,
//! **a new writer of overlay-tree state cannot make this cache go
//! stale by forgetting to notify it**. A mutator apply, a
//! `Scene::tree_mut` escape-hatch write, a full re-register — all of
//! them are seen the same way, as a `GlyphArea` that no longer
//! compares equal. `GlyphArea`'s `PartialEq` is derived, so a field
//! added to the area joins the comparison by default rather than by
//! anyone remembering to wire it in. "By default" and not
//! "always": the derive is `derivative`'s, and a field carrying
//! `#[derivative(PartialEq = "ignore")]` opts back out — exactly
//! what `hitbox`, the one field skipped today, does. So the
//! ignore-list is pinned rather than trusted:
//! `test_glyph_area_equality_ignores_only_the_hitbox` destructures
//! `GlyphArea` exhaustively and asserts every other field breaks
//! `==`, which fails both on a new field nobody classified and on
//! an `ignore` added to an existing one. `hitbox` itself is on the
//! ignored side because the shaper never reads it.
//!
//! ## The one place `==` is not enough
//!
//! `GlyphArea`'s derived equality bottoms out, for the `regions`
//! field, in `BTreeSet` equality over `ColorFontRegion` — whose own
//! `Eq` is *set identity by range*, ignoring the font and color
//! pins. So `area_a == area_b` is true for two areas whose spans
//! are painted different colors, which is precisely what a
//! color-picker hover changes and nothing else. Comparing on `==`
//! alone would have reused every cell's buffers through a hover and
//! frozen the wheel's colors on screen. The reuse check therefore
//! adds
//! [`ColorFontRegions::same_content`](baumhard::core::primitives::ColorFontRegions::same_content),
//! the content-equality question that type now answers, on top of
//! `==`.
//!
//! The one input outside that set is the font database itself, which
//! `RegionFamilies::resolve` and cosmic-text's shaper both consult.
//! It is loaded once, by `fonts::load_fonts` behind a `lazy_static`,
//! and never mutated again — `db_mut` has exactly one call site in
//! the workspace — so it cannot invalidate a cached buffer at
//! runtime. A future runtime font load would have to clear this
//! cache, and this paragraph exists to tell whoever writes one.
//!
//! The reuse rule is pinned by
//! `test_overlay_shape_cache_invalidates_on_every_writable_area_field`
//! and its three neighbors in `renderer::tests`.

use glam::Vec2;

use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::scene::SceneTreeId;

use super::MindMapTextBuffer;

/// One overlay element's shaped output plus the inputs it was shaped
/// from. Held in walk order by
/// [`Renderer::overlay_scene_buffers`](super::Renderer#structfield.overlay_scene_buffers);
/// see the module header for the reuse rule.
pub(super) struct ShapedOverlayElement {
    /// Which overlay tree the element came from.
    tree: SceneTreeId,
    /// The element's `unique_id`, one of the two things the shaper
    /// reads off the element.
    unique_id: usize,
    /// The registered tree offset in force when this was shaped.
    offset: Vec2,
    /// The area the buffers below were shaped from, cloned
    /// verbatim. `None` for a `Void` / `GlyphModel` element, which
    /// shapes to nothing but still occupies a walk position.
    area: Option<GlyphArea>,
    /// The shaped buffers, in emission order: halo stamps first,
    /// main glyph last.
    pub(super) buffers: Vec<MindMapTextBuffer>,
}

impl ShapedOverlayElement {
    /// Capture `element`'s shaping inputs alongside `buffers`.
    pub(super) fn new(
        tree: SceneTreeId,
        element: &GfxElement,
        offset: Vec2,
        buffers: Vec<MindMapTextBuffer>,
    ) -> Self {
        ShapedOverlayElement {
            tree,
            unique_id: element.unique_id(),
            offset,
            area: element.glyph_area().cloned(),
            buffers,
        }
    }

    /// Whether the buffers held here are still what shaping
    /// `element` — at walk position `tree` / `offset` — would
    /// produce. See the module header for what this set is and why
    /// it is complete.
    ///
    /// # Costs
    ///
    /// Two integer compares, a `Vec2` compare, one `GlyphArea`
    /// equality and one region-content walk — together
    /// O(text length + region count) and allocation-free. Cheap
    /// against the cosmic-text shaping it stands in for.
    pub(super) fn still_matches(&self, tree: SceneTreeId, element: &GfxElement, offset: Vec2) -> bool {
        if self.tree != tree || self.unique_id != element.unique_id() || self.offset != offset {
            return false;
        }
        match (self.area.as_ref(), element.glyph_area()) {
            (None, None) => true,
            // `==` covers every area field; `same_content` covers
            // what `==` cannot see inside `regions` — see the module
            // header's "The one place `==` is not enough".
            (Some(cached), Some(live)) => cached == live && cached.regions.same_content(&live.regions),
            _ => false,
        }
    }
}
