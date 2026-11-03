// SPDX-License-Identifier: MPL-2.0

//! Per-element re-shape cache shared by both scene passes — the
//! screen-space overlay sub-scene and the camera-transformed canvas
//! sub-scene.
//!
//! Both sub-scenes of
//! [`AppScene`](crate::application::scene_host::AppScene) are
//! re-walked after every change to any of their trees. Walking them
//! is cheap; *shaping* them is not — every walked `GlyphArea` used to
//! get a fresh `cosmic_text::Buffer`, a `set_rich_text` and a
//! `shape_until_scroll`, times `halos + 1`, under one `FONT_SYSTEM`
//! write guard, at mouse-move cadence. A picker hover changes one
//! cell's color; a console keystroke changes one line's text; an
//! edge-label drag moves one label. None of those is a reason to
//! re-shape the other fifty-odd elements, or the other seven canvas
//! roles.
//!
//! ## The reuse rule, and why it needs no notifications
//!
//! This cache does **not** subscribe to a dirty signal. It keeps,
//! per shaped element, a verbatim copy of every input the shaper
//! read, and re-validates against the live tree on each pass. An
//! element's output is reused as it stands only when all of the
//! following still hold at the same position in the walk — and,
//! when only the placement ones fail, moved rather than re-shaped
//! (see "Moved is not stale" below):
//!
//! - the same tree ([`SceneTreeId`]),
//! - the same `GfxElement::unique_id`,
//! - the same registered tree offset (the walker adds it to every
//!   emitted buffer's `pos`),
//! - a `GlyphArea` equal to the stored one — or, for an element that
//!   carries no area, still no area.
//!
//! That list is exactly what [`shape_one_element_into_buffers`]
//! reads: `element.glyph_area()`, `element.unique_id()`, and its
//! `offset` argument. Nothing else reaches the emitted buffers. (An
//! element emits no fill: a map draws no rect, V1-B12.)
//!
//! The consequence worth naming: because the check compares against
//! live state rather than trusting a producer to announce itself,
//! **a new writer of scene-tree state cannot make this cache go
//! stale by forgetting to notify it**. A mutator apply, a
//! `Scene::tree_mut` escape-hatch write, a full re-register — all of
//! them are seen the same way, as a `GlyphArea` that no longer
//! compares equal. That is what answers the canvas pass's ten
//! `flush_canvas_scene_buffers` call sites as a class rather than
//! one at a time: each of them mutates registered canvas trees and
//! then flushes, and whatever it wrote is what the next walk reads.
//! A caller that touched no tree at all re-shapes nothing; a caller
//! that re-registered one role re-shapes the elements of that role
//! whose shaping inputs actually moved.
//!
//! ## Moved is not stale
//!
//! Two of those inputs never reach cosmic-text: `GlyphArea::position`
//! and the registered tree offset. The walker adds them to each
//! emitted buffer's `pos` *after* shaping, and nothing else reads
//! them. So the key splits in two,
//! and [`ShapedSceneElement::classify`] answers in three:
//!
//! - **must re-shape** — the tree, the `unique_id`, and every area
//!   field but `position` (text, scale, line height, bounds, regions,
//!   outline, shape, alignment, zoom window). Any
//!   difference here is [`Verdict::Stale`], and the element goes back
//!   through [`shape_one_element_into_buffers`];
//! - **may be patched** — `position` and the tree offset. When these
//!   are the *only* difference the verdict is [`Verdict::Moved`], and
//!   [`ShapedSceneElement::patch_placement`] rewrites the held
//!   buffers' positions in place, with no shaping
//!   and no `FONT_SYSTEM` lock;
//! - **neither differs** — [`Verdict::Current`], nothing to do.
//!
//! The must-re-shape side is not a field list. It is the derived
//! equality with the recorded position aligned to the live one for
//! the length of the comparison, so a field added to `GlyphArea`
//! lands on the must-re-shape side by default — the conservative
//! one — and only `position` is singled out by name: once in
//! `classify`, to mask it, and once in `patch_placement`, to apply it.
//!
//! A picker wheel-move is what this is for. It re-runs the
//! layout-phase mutator, which writes every cell's position and
//! leaves every other field as it was; before the split each of the
//! wheel's outlined cells went back through the shaper once per halo
//! stamp plus once for its glyph, to come back with identical glyphs.
//! An edge-label drag on the canvas pass is the same shape of change.
//!
//! The patch has one hazard, and it is the overlay pass's common case
//! rather than an edge: an outlined element's buffers do not all sit
//! at its anchor. Its eight halo stamps sit at
//! [`OutlineStyle::offsets`](baumhard::gfx_structs::area_fields::OutlineStyle::offsets)
//! around it, so a patch that wrote the anchor flat would collapse
//! the halo under the glyph. The walker records each stamp's offset
//! in [`MindMapTextBuffer::emission_offset`] and the patch re-derives
//! `pos` from it — the same fix `Renderer::patch_drag_positions`
//! carries on the mindmap path.
//!
//! `GlyphArea`'s `PartialEq` is written out over an exhaustive
//! destructuring of the area, so a field added to it does not
//! compile until it is either compared or ignored by name — nobody
//! has to remember to wire it in. `hitbox` is the one field ignored
//! today. Ignoring a field is still one edit away, so the ignore-list
//! is pinned rather than trusted:
//! `test_glyph_area_equality_ignores_only_the_hitbox` destructures
//! `GlyphArea` exhaustively and asserts every other field breaks
//! `==`, which fails both on a new field nobody classified and on
//! an existing one dropped from the comparison. `hitbox` itself is on
//! the ignored side because the shaper never reads it.
//!
//! ## The one place `==` is not enough
//!
//! `GlyphArea`'s equality bottoms out, for the `regions`
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
//! ## Glyph runs carry their shaping
//!
//! A `GlyphRun` element is not shaped here at all: it holds its shaped
//! run and its placement (`Arc`s, shaped once by `baumhard::flow`), and
//! the positioned-glyph pass ([`super::glyph_runs`]) draws it from
//! them. Its entry holds a clone of the run — two `Arc`s and its
//! override vectors — and is compared by the run's own
//! [`GlyphRunData::same_state`]: the shaping and placement by identity,
//! everything else by value. A run whose state changed is held again
//! without the `FONT_SYSTEM` lock ([`ScenePassCounts::recaptured`]); a
//! run whose tree moved is [`Verdict::Moved`] like any element, because
//! the tree offset is added to it at draw time ([`glyph_runs`]).
//!
//! ## Draw order across a partial re-shape
//!
//! Each entry owns its own output and entries sit in walk order, so
//! re-shaping one element writes back into the position that element
//! already occupied. That is what keeps the order-sensitive draw
//! streams stable while only part of the scene re-shapes: text areas
//! are handed to glyphon in walk order, so a later one paints over an
//! earlier one. Grouping by walk position is the structural answer to
//! a hazard an earlier flat list keyed by element had — re-inserting
//! an edited element's output at the end floated it above every
//! other's — rather than its bookkeeping one: there is no removal for
//! an insertion point to be lost across.
//!
//! One function carries that order to the render pass, and it decides
//! it for **all three** order-sensitive streams: [`text_buffers`], for
//! each of the two glyphon passes, beside [`glyph_runs`] for the
//! positioned-glyph pass. `text_buffers` carries a
//! second ordering besides the one between elements — the walker
//! emits an element's halo stamps before the glyph they ring, so
//! that the glyph draws on top — which makes its inner order
//! load-bearing too.
//!
//! The order is pinned **absolutely**, by
//! `test_the_draw_streams_come_out_in_walk_order` and
//! `test_the_text_stream_puts_every_halo_stamp_before_the_glyph_it_rings`,
//! and the reason they have to be — rather than the comparison a
//! reuse cache invites — is worth stating instead of rediscovering.
//! That comparison is "a partial pass produces what a full one
//! does", and
//! `test_canvas_partial_reshape_matches_a_full_one_stream_for_stream`
//! makes it; it catches a per-entry difference the in-place path
//! introduces. What it cannot catch is a reordering the reader
//! applies *uniformly*, because it reads both of its sides through it
//! — a comparison between two runs of one reader can only see what
//! the reader is not doing to both. Before the absolute assertions
//! existed, reversing the reader left that
//! test and every other test in the workspace green; they are what
//! closed that, and the boundary is recorded here because it is a
//! property of the comparison's *shape* and will hold again for the
//! next assertion built that way.
//!
//! The reuse rule is pinned by
//! `test_scene_shape_cache_invalidates_on_every_writable_area_field`
//! and its neighbors in `renderer::tests`; the split by
//! `test_a_position_only_change_patches_and_shapes_nothing`,
//! `test_a_patch_places_halo_stamps_from_their_recorded_offsets`,
//! `test_a_shaping_change_re_shapes_even_alongside_a_move` and
//! `test_picker_drag_move_patches_every_cell_and_shapes_none`.

use glam::Vec2;

use baumhard::font::fonts;
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::run::GlyphRunData;
use baumhard::gfx_structs::scene::{Scene, SceneTreeId};

use super::tree_walker::shape_one_element_into_buffers;
use super::MindMapTextBuffer;

/// Which of the two sub-scenes a [`refresh`] pass is walking.
///
/// The passes differ in exactly one thing, and it lives on this enum:
/// the site name the `FONT_SYSTEM` write guard is acquired under.
/// Everything else — the walk, the reuse rule, the
/// truncation — is one body serving both.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(crate) enum ScenePassKind {
    /// The camera-transformed sub-scene: borders, connections,
    /// portals, edge handles, connection labels, section frames and
    /// the two resize-handle roles.
    Canvas,
    /// The screen-space sub-scene: the console and the color picker.
    Overlay,
}

impl ScenePassKind {
    /// The site name passed to
    /// [`fonts::acquire_font_system_write`], which reports it when a
    /// re-entrant same-thread acquire is caught. Distinct per pass
    /// so the report names which of the two was walking.
    fn font_system_site(self) -> &'static str {
        match self {
            ScenePassKind::Canvas => "scene_shape_cache::refresh (canvas)",
            ScenePassKind::Overlay => "scene_shape_cache::refresh (overlay)",
        }
    }
}

/// What one [`refresh`] pass did.
///
/// Returned rather than merely performed for the reason
/// `RebuildTier` on the app side is a value rather than a statement:
/// the pass runs inside a `Renderer`, and the renderer-free tier of
/// the suite (TEST_CONVENTIONS §T8) has no device, so a granularity
/// claim about it has to be observable from somewhere a test can
/// stand. `refresh` takes a `&Scene` and not a renderer precisely so
/// that place exists, and these counts are what the claim is made in.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(crate) struct ScenePassCounts {
    /// Walk positions visited across every visible tree of the
    /// sub-scene — the number of elements the pass had to consider.
    pub(crate) walked: usize,
    /// How many of those were re-shaped because an input that
    /// reaches the shaped glyphs no longer matched what the cached
    /// output was produced from — [`Verdict::Stale`]. Each one is
    /// exactly one [`shape_one_element_into_buffers`] call: this
    /// counter is incremented at that call's only site.
    pub(crate) reshaped: usize,
    /// Walk positions whose placement changed and nothing else —
    /// [`Verdict::Moved`] — so whatever output they hold (buffers,
    /// or for a `Void` / `GlyphModel` element nothing but the
    /// recorded offset) was re-placed in place and no shaping ran for
    /// them. Counted the way `reshaped` counts: per walk position, a
    /// `Void` whose tree moved included. The rest,
    /// `walked - reshaped - patched - recaptured`, kept their output
    /// untouched.
    pub(crate) patched: usize,
    /// Walk positions holding a `GlyphRun` whose state changed (or that
    /// newly hold one): the run is held again, which clones two `Arc`s
    /// and its override vectors. Nothing is shaped and the font lock is
    /// not taken — a run carries its shaping with it.
    pub(crate) recaptured: usize,
}

/// What [`ShapedSceneElement::classify`] found at one walk position.
///
/// Three answers rather than a yes/no because the reuse key has two
/// halves: the inputs that reach the *shaped glyphs* and the inputs
/// that only decide *where* those glyphs are placed. See the module
/// header's "Moved is not stale".
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(crate) enum Verdict {
    /// Every input matches; the cached output is exactly what
    /// shaping the live element would produce.
    Current,
    /// Only the placement inputs — `GlyphArea::position` and the
    /// registered tree offset — differ. The cached glyphs are still
    /// right; their positions are not, and
    /// [`ShapedSceneElement::patch_placement`] re-derives them.
    Moved,
    /// An input the shaped glyphs depend on differs, or the walk
    /// position now holds a different element. Must be re-shaped.
    Stale,
}

/// One scene element's shaped output plus the inputs it was shaped
/// from. Held in walk order by
/// [`Renderer::canvas_scene_elements`](super::Renderer#structfield.canvas_scene_elements)
/// and
/// [`Renderer::overlay_scene_elements`](super::Renderer#structfield.overlay_scene_elements);
/// see the module header for the reuse rule.
pub(crate) struct ShapedSceneElement {
    /// Which tree of the sub-scene the element came from.
    tree: SceneTreeId,
    /// The element's `unique_id`, one of the two things the shaper
    /// reads off the element.
    unique_id: usize,
    /// The registered tree offset in force when this was shaped.
    offset: Vec2,
    /// The area the output below was shaped from, cloned verbatim.
    /// `None` for a `Void` / `GlyphModel` element, which shapes to
    /// nothing but still occupies a walk position.
    area: Option<GlyphArea>,
    /// The shaped buffers, in emission order: halo stamps first,
    /// main glyph last.
    pub(super) buffers: Vec<MindMapTextBuffer>,
    /// The element's run, when it is a `GlyphRun`: what the
    /// positioned-glyph pass draws ([`super::glyph_runs`]). Held by
    /// clone, which shares the run's shaping and placement (`Arc`s).
    pub(super) run: Option<GlyphRunData>,
}

impl ShapedSceneElement {
    /// Capture `element`'s shaping inputs alongside the output it
    /// produced.
    pub(super) fn new(
        tree: SceneTreeId,
        element: &GfxElement,
        offset: Vec2,
        buffers: Vec<MindMapTextBuffer>,
    ) -> Self {
        ShapedSceneElement {
            tree,
            unique_id: element.unique_id(),
            offset,
            area: element.glyph_area().cloned(),
            buffers,
            run: element.glyph_run().cloned(),
        }
    }

    /// Decide whether the output held here is still what shaping
    /// `element` — at walk position `tree` / `offset` — would
    /// produce, would produce *somewhere else*, or would not produce
    /// at all. See the module header for what the input set is, why
    /// it is complete, and how it splits.
    ///
    /// Takes `&mut self` for an allocation-free comparison, and
    /// leaves `self` exactly as it found it: the recorded position is
    /// aligned to the live one for the length of one `==`, so that
    /// `GlyphArea`'s equality — which a new `GlyphArea` field cannot
    /// compile without joining or being ignored by name — answers "is
    /// anything *but* the position different?" without a clone of the
    /// area or a field list here that a new field would silently miss.
    ///
    /// # Costs
    ///
    /// Two integer compares, a `Vec2` compare, one `GlyphArea`
    /// equality and one region-content walk — together
    /// O(text length + region count) and allocation-free. Cheap
    /// against the cosmic-text shaping it stands in for.
    pub(super) fn classify(&mut self, tree: SceneTreeId, element: &GfxElement, offset: Vec2) -> Verdict {
        if self.tree != tree || self.unique_id != element.unique_id() {
            return Verdict::Stale;
        }
        let offset_moved = self.offset != offset;
        // A run is compared by its own state (`same_state`: its shaping
        // and placement by identity, everything else by value); the tree
        // offset is added to it at draw time, so a run whose state
        // matches in a moved tree is only moved.
        match (&self.run, element.glyph_run()) {
            (None, None) => {}
            (Some(held), Some(live)) if held.same_state(live) => {
                return if offset_moved {
                    Verdict::Moved
                } else {
                    Verdict::Current
                };
            }
            _ => return Verdict::Stale,
        }
        match (self.area.as_mut(), element.glyph_area()) {
            (None, None) if offset_moved => Verdict::Moved,
            (None, None) => Verdict::Current,
            (Some(cached), Some(live)) => {
                let recorded = cached.position;
                cached.position = live.position;
                // `==` covers every area field; `same_content` covers
                // what `==` cannot see inside `regions` — see the
                // module header's "The one place `==` is not enough".
                let shaping_unchanged = *cached == *live && cached.regions.same_content(&live.regions);
                cached.position = recorded;
                if !shaping_unchanged {
                    Verdict::Stale
                } else if offset_moved || recorded != live.position {
                    Verdict::Moved
                } else {
                    Verdict::Current
                }
            }
            _ => Verdict::Stale,
        }
    }

    /// Move the output held here to where shaping `element` at
    /// `offset` would have placed it, without shaping anything. Only
    /// meaningful after [`classify`](Self::classify) answered
    /// [`Verdict::Moved`] for the same `element` and `offset`.
    ///
    /// **Halos.** An outlined element holds one buffer per halo stamp
    /// plus its main glyph, and every one of them sits at a different
    /// distance from the anchor. Each buffer's position is therefore
    /// re-derived through [`MindMapTextBuffer::place_at`] — the same
    /// [`MindMapTextBuffer::stamp_pos`] the walker placed it with —
    /// from the
    /// [`emission_offset`](MindMapTextBuffer#structfield.emission_offset)
    /// the walker recorded when it stamped it. Writing the anchor
    /// flat would stack all eight halos under the glyph — the hazard
    /// `Renderer::patch_drag_positions` was fixed for on the mindmap
    /// path, and the common case here: the picker's wheel cells are
    /// outlined.
    ///
    /// # Costs
    ///
    /// O(halos + 1) position writes. No shaping, no `FONT_SYSTEM`
    /// lock, no allocation.
    pub(super) fn patch_placement(&mut self, element: &GfxElement, offset: Vec2) {
        self.offset = offset;
        let (Some(cached), Some(live)) = (self.area.as_mut(), element.glyph_area()) else {
            return;
        };
        cached.position = live.position;
        let anchor = (live.position.x.0, live.position.y.0);
        for buffer in &mut self.buffers {
            buffer.place_at(anchor, offset);
        }
    }
}

/// Bring `shaped` back in step with `scene`, re-shaping only the
/// walk positions whose inputs moved, and report what that took.
///
/// `ids` is the sub-scene's tree handles in layer order; the walk
/// visits every descendant of every *visible* one, in that order, so
/// entry `i` of `shaped` describes walk position `i`. Each position
/// is [classified](ShapedSceneElement::classify) against the entry
/// sitting there: a current one keeps its buffers untouched, a moved one has their positions patched in place, and
/// only a stale one is re-shaped — also in place. Positions past the
/// end of the walk belong to a tree that has since been unregistered
/// or shortened and are dropped.
///
/// # Costs
///
/// O(sum of descendants) across every tree named in `ids` for the
/// walk and the equality checks, plus one `cosmic_text::Buffer`
/// allocation and shaping per *stale* non-empty `GlyphArea`
/// (times `halos + 1`); a *moved* one costs `halos + 1` position
/// writes and no shaping. The `FONT_SYSTEM` write guard is acquired
/// lazily on the first element that actually needs shaping, so a
/// pass in which nothing went stale takes no lock at all — which is the
/// steady state for every sub-scene tree a given caller did not
/// touch. Empty sub-scenes short-circuit cheaply.
pub(crate) fn refresh(
    scene: &Scene,
    ids: &[SceneTreeId],
    shaped: &mut Vec<ShapedSceneElement>,
    kind: ScenePassKind,
) -> ScenePassCounts {
    // Lazily acquired: a pass in which every element still matches
    // its cached inputs never touches the lock.
    let mut font_system: Option<fonts::FontSystemWriteGuard> = None;
    let mut counts = ScenePassCounts {
        walked: 0,
        reshaped: 0,
        patched: 0,
        recaptured: 0,
    };
    for &id in ids {
        let Some(entry) = scene.get(id) else {
            continue;
        };
        if !entry.visible() {
            continue;
        }
        let tree = entry.tree();
        let offset = entry.offset();
        for descendant_id in tree.root().descendants(tree.arena()) {
            let Some(element) = tree.arena().get(descendant_id).map(|n| n.get()) else {
                continue;
            };
            let slot = counts.walked;
            counts.walked += 1;
            if let Some(cached) = shaped.get_mut(slot) {
                match cached.classify(id, element, offset) {
                    Verdict::Current => continue,
                    Verdict::Moved => {
                        cached.patch_placement(element, offset);
                        counts.patched += 1;
                        continue;
                    }
                    Verdict::Stale => {}
                }
            }
            if element.glyph_run().is_some() {
                // A run is drawn from the shaping it carries: hold it
                // again, without the font lock.
                let fresh = ShapedSceneElement::new(id, element, offset, Vec::new());
                match shaped.get_mut(slot) {
                    Some(existing) => *existing = fresh,
                    None => shaped.push(fresh),
                }
                counts.recaptured += 1;
                continue;
            }
            let font_system =
                font_system.get_or_insert_with(|| fonts::acquire_font_system_write(kind.font_system_site()));
            let mut buffers = Vec::new();
            shape_one_element_into_buffers(
                element,
                offset,
                font_system,
                &mut |_unique_id, buffer| buffers.push(buffer),
            );
            let fresh = ShapedSceneElement::new(id, element, offset, buffers);
            match shaped.get_mut(slot) {
                Some(existing) => *existing = fresh,
                None => shaped.push(fresh),
            }
            counts.reshaped += 1;
        }
    }
    // Anything past the last walked position belongs to a tree that
    // has since been unregistered or shortened.
    shaped.truncate(counts.walked);
    counts
}

/// Every shaped text buffer of a pass's output, in draw order.
///
/// The single reader of the grouping for the render path, so the
/// main pass and the palette pass flatten it the same way and a
/// test can assert on the same sequence the GPU is handed. Takes the
/// elements as an iterator rather than a slice because the canvas
/// pass draws from two shapes of source ([`canvas_elements`]).
pub(super) fn text_buffers<'a>(
    shaped: impl IntoIterator<Item = &'a ShapedSceneElement>,
) -> impl Iterator<Item = &'a MindMapTextBuffer> {
    shaped.into_iter().flat_map(|element| element.buffers.iter())
}

/// How many buffers [`text_buffers`] will yield — the capacity the
/// render pass reserves before flattening, counted without walking
/// into the groups.
pub(super) fn buffer_count<'a>(shaped: impl IntoIterator<Item = &'a ShapedSceneElement>) -> usize {
    shaped.into_iter().map(|element| element.buffers.len()).sum()
}

/// Every `GlyphRun` of a pass's output, in draw order, with the
/// registered offset of the tree it came from — what the
/// positioned-glyph pass ([`super::glyph_runs`]) draws, as
/// [`text_buffers`] is what glyphon draws.
pub(super) fn glyph_runs<'a>(
    shaped: impl IntoIterator<Item = &'a ShapedSceneElement>,
) -> impl Iterator<Item = (&'a GlyphRunData, Vec2)> {
    shaped
        .into_iter()
        .filter_map(|element| element.run.as_ref().map(|run| (run, element.offset)))
}

// ---------------------------------------------------------------------
// A capture's canvas: the live zoom-independent roles and a scratch
// projection of the zoom-dependent ones, drawn as one frame.
// ---------------------------------------------------------------------

/// Which shaped canvas output a frame's main pass draws.
///
/// The window draws [`Self::Live`]: `Renderer::canvas_scene_elements`,
/// whole, in the walk order [`refresh`] left it in. A capture framed
/// at a zoom other than the window's draws [`Self::Composed`]: the
/// [`CaptureCanvas`] the capture engine filled a moment before, which
/// interleaves the live set's zoom-independent trees with a scratch
/// set's zoom-dependent ones. The window never draws `Composed` and a
/// capture at the live zoom draws `Live`; what either reads is chosen
/// per frame by the caller of `Renderer::prepare_frame`, so a frame
/// cannot be left pointed at a set it did not ask for.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(crate) enum CanvasSource {
    /// The live canvas set, all of it.
    Live,
    /// The capture composition ([`CaptureCanvas::segments`]).
    // Native-only producer: the capture engine; the window always
    // draws the live set.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Composed,
}

/// Which of the two shaped canvas sets a [`CanvasSegment`] reads.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(crate) enum ShapedSet {
    /// `Renderer::canvas_scene_elements`: the window's set.
    Live,
    /// [`CaptureCanvas::shaped`]: the capture's scratch projection.
    Scratch,
}

/// One tree's worth of a composed frame: a contiguous run of walk
/// positions in one set, in the order [`refresh`] left them.
#[derive(Clone, Eq, PartialEq, Debug)]
pub(crate) struct CanvasSegment {
    pub(crate) set: ShapedSet,
    pub(crate) range: std::ops::Range<usize>,
}

/// A capture's canvas, held by the renderer beside the live set.
///
/// `shaped` is a second [`refresh`] output, walked from the capture
/// engine's scratch `AppScene` and reused across captures by the same
/// rule as the live set — its reuse key is the scratch scene's tree
/// ids, which never meet the live scene's, because each set is
/// refreshed from exactly one `Scene`. `segments` is the draw order of
/// the next composed frame: for each canvas role in layer order, the
/// live set's span of that role's tree when the role is
/// zoom-independent, and the scratch set's when it is not
/// ([`compose_capture_canvas`]).
#[derive(Default)]
pub(crate) struct CaptureCanvas {
    pub(super) shaped: Vec<ShapedSceneElement>,
    pub(super) segments: Vec<CanvasSegment>,
}

#[cfg(test)]
impl CaptureCanvas {
    /// The scratch set, for a test to fill through [`refresh`] as the
    /// renderer's flush does.
    pub(crate) fn shaped_mut(&mut self) -> &mut Vec<ShapedSceneElement> {
        &mut self.shaped
    }

    /// The scratch set, for a test to compute its own spans over.
    pub(crate) fn shaped(&self) -> &[ShapedSceneElement] {
        &self.shaped
    }

    /// The composition the next composed frame draws.
    pub(crate) fn segments(&self) -> &[CanvasSegment] {
        &self.segments
    }
}

impl ShapedSceneElement {
    /// The tree this element was walked from. Test-gated: production
    /// reads it through [`span_of`], and the composition test computes
    /// its expected spans without that function.
    #[cfg(test)]
    pub(crate) fn tree(&self) -> SceneTreeId {
        self.tree
    }
}

/// The walk positions [`refresh`] gave `tree` within `shaped`.
///
/// Contiguous by construction: the pass visits one tree's descendants
/// before the next tree's, and writes entry `i` at walk position `i`.
/// Empty for a tree that contributed nothing — unregistered,
/// invisible, or not among the pass's ids. O(len) in the worst case,
/// one scan.
pub(super) fn span_of(shaped: &[ShapedSceneElement], tree: SceneTreeId) -> std::ops::Range<usize> {
    let Some(first) = shaped.iter().position(|element| element.tree == tree) else {
        return 0..0;
    };
    let len = shaped[first..]
        .iter()
        .take_while(|element| element.tree == tree)
        .count();
    first..first + len
}

/// Fill `capture.segments` with the draw order of a frame that shows
/// `scratch`'s zoom-dependent roles over `live`'s zoom-independent
/// ones: every canvas role in layer order, each read from the set
/// that holds the right projection of it
/// ([`CanvasRole::zoom_dependent`](crate::application::scene_host::CanvasRole::zoom_dependent)).
/// A role with no tree in its scene, or whose tree walked to nothing,
/// contributes no segment.
///
/// `live_shaped` and `capture.shaped` must be the current [`refresh`]
/// outputs of the two scenes — the segments are ranges into them.
/// Pure over its inputs: the renderer calls it from
/// `Renderer::rebuild_capture_canvas_buffers`, and a test calls it
/// with two projected scenes and no device (TEST_CONVENTIONS §T8).
///
/// # Costs
///
/// One [`span_of`] scan per registered role — eight at most — and a
/// sort of eight roles by layer. No allocation past the segment list's
/// first fill.
// Native-only caller: the capture engine.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn compose_capture_canvas(
    live: &crate::application::scene_host::AppScene,
    scratch: &crate::application::scene_host::AppScene,
    live_shaped: &[ShapedSceneElement],
    capture: &mut CaptureCanvas,
) {
    use crate::application::scene_host::CanvasRole;

    let mut roles = CanvasRole::ALL;
    roles.sort_by_key(|role| role.layer());
    capture.segments.clear();
    for role in roles {
        let (set, scene, shaped): (ShapedSet, _, &[ShapedSceneElement]) = if role.zoom_dependent() {
            (ShapedSet::Scratch, scratch, &capture.shaped)
        } else {
            (ShapedSet::Live, live, live_shaped)
        };
        let Some(id) = scene.canvas_id(role) else {
            continue;
        };
        let range = span_of(shaped, id);
        if !range.is_empty() {
            capture.segments.push(CanvasSegment { set, range });
        }
    }
}

/// The elements a frame's main pass draws from `source`, in draw
/// order: the live set as it stands, or the composition's segments one
/// after another. A segment whose range the set no longer covers
/// yields nothing rather than panicking — an interactive path
/// (CODE_CONVENTIONS §9) — though none can: the composition is read
/// by the capture that filled it, with nothing between.
pub(super) fn canvas_elements<'a>(
    source: CanvasSource,
    live: &'a [ShapedSceneElement],
    capture: &'a CaptureCanvas,
) -> impl Iterator<Item = &'a ShapedSceneElement> + 'a {
    match source {
        CanvasSource::Live => Either::Left(live.iter()),
        CanvasSource::Composed => Either::Right(capture.segments.iter().flat_map(move |segment| {
            let set = match segment.set {
                ShapedSet::Live => live,
                ShapedSet::Scratch => &capture.shaped[..],
            };
            set.get(segment.range.clone())
                .into_iter()
                .flat_map(|slice| slice.iter())
        })),
    }
}

/// One of two iterators with the same item, for a function that
/// returns either shape. The private twenty lines that spare the
/// render path a `Box<dyn Iterator>` per frame.
enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<T, L: Iterator<Item = T>, R: Iterator<Item = T>> Iterator for Either<L, R> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        match self {
            Either::Left(left) => left.next(),
            Either::Right(right) => right.next(),
        }
    }
}
