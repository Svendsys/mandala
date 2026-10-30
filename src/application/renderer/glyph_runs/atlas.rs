// SPDX-License-Identifier: MPL-2.0

//! The positioned-glyph atlas: every glyph a [`GlyphRun`] draws,
//! rasterized once by cosmic-text and held on the GPU, keyed by the
//! cosmic-text [`CacheKey`] that names the rasterization.
//!
//! **Why the renderer holds a second atlas.** glyphon keeps its own
//! (`glyphon::TextAtlas`) for the `TextArea`s it draws, but exposes no
//! way to look a glyph up in it or insert one: in glyphon 0.12.0 the
//! atlas pages, their glyph cache and their packer are all
//! `pub(crate)`, and its one public method besides the constructors is
//! `trim`. A pipeline that draws glyphs glyphon did not lay out must
//! therefore hold its own. Until the remaining `TextArea` users move to
//! runs (V1-C2, C3), a glyph drawn by both paths is rasterized into
//! both atlases; that is the cost of the transition, and it ends with
//! it.
//!
//! **Two pages**, as glyphon has: an `R8Unorm` page for coverage masks
//! (ordinary glyphs, tinted by their color) and an `Rgba8UnormSrgb`
//! page for color glyphs (emoji), which keep their own colors. Each
//! page is packed by an [`etagere`] bucketed allocator, starts at
//! [`INITIAL_SIDE`] and doubles up to the device's
//! `max_texture_dimension_2d`.
//!
//! **Lifetime of an entry: evict before growing.** An entry is made the
//! first time a frame asks for its key. When a bitmap does not fit its
//! page, every entry of that page **stale for two frames** — drawn
//! neither by the current frame nor by the one before it
//! ([`PROTECTED_FRAMES`]) — is evicted; the page grows only when that
//! frees too little, which is when the last two frames' own glyphs do
//! not fit together. This is glyphon's order
//! (`InnerAtlas::try_allocate` evicts before `prepare` grows), though
//! glyphon evicts least recently used first, one at a time; this atlas
//! evicts everything stale at once (`GlyphAtlas::evict_for` says why).
//!
//! **Why two frames, not one.** "Not drawn by the current frame" is
//! everything the frame has not reached *yet*: a miss early in a frame
//! would evict the glyphs its later runs are about to ask for, and a
//! steady view would be rasterized again, whole, every few frames.
//! Protecting the previous frame too keeps a view that is drawn every
//! frame resident, keeps both of two alternating views (the window and
//! a capture at another scale) once the page has grown to hold both,
//! and still lets a continuous zoom drop the sizes it has left behind:
//! the page holds about two frames of sizes, not every size seen.
//!
//! **Remembered without a bitmap.** A glyph with nothing to draw (a
//! space), and one whose bitmap is larger than the largest page, are
//! remembered as such, so neither is rasterized again every frame.
//! Such entries are forgotten by any eviction pass once they are stale
//! for two frames, so they cannot accumulate across sizes. A glyph
//! refused for want of room *now* is not remembered: a later frame,
//! with other glyphs evictable, may have room for it.
//!
//! **Growth re-rasterizes.** etagere keeps every allocation where it
//! was when a page grows, so a grown page is filled by rasterizing its
//! entries again into the larger texture — glyphon's own approach,
//! chosen over a GPU texture copy because it is the one already proven
//! on every backend the app ships on (WebGL2 included).
//!
//! [`GlyphRun`]: baumhard::gfx_structs::element::GfxElement::GlyphRun

use etagere::{size2, AllocId, BucketedAtlasAllocator};
use glyphon::{CacheKey, FontSystem, SwashCache, SwashContent};
use rustc_hash::FxHashMap;

/// The side, in texels, a page starts at: glyphon's own starting size.
const INITIAL_SIDE: u32 = 256;

/// How many of the most recent frames' entries an eviction never
/// touches: the current frame's and the one before it. See the module
/// docs, "Why two frames, not one".
const PROTECTED_FRAMES: u64 = 2;

/// Which page a glyph lives on.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    /// Full-color glyphs (emoji): RGBA, sRGB-encoded.
    Color,
    /// Coverage masks: one channel, tinted by the glyph's color.
    Mask,
}

impl Page {
    fn format(self) -> wgpu::TextureFormat {
        match self {
            Page::Color => wgpu::TextureFormat::Rgba8UnormSrgb,
            Page::Mask => wgpu::TextureFormat::R8Unorm,
        }
    }

    fn bytes_per_texel(self) -> u32 {
        match self {
            Page::Color => 4,
            Page::Mask => 1,
        }
    }

    /// The page swash's content goes on, or `None` for content this
    /// atlas does not draw: cosmic-text rasterizes in alpha or color,
    /// never in subpixel masks, so that arm is refused rather than
    /// guessed at.
    fn of(content: SwashContent) -> Option<Page> {
        match content {
            SwashContent::Color => Some(Page::Color),
            SwashContent::Mask => Some(Page::Mask),
            SwashContent::SubpixelMask => None,
        }
    }
}

/// Where a glyph's bitmap is, and how it sits relative to its pen.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    /// The page it is on.
    pub(crate) page: Page,
    /// Its top-left texel on that page.
    pub(crate) uv: (u32, u32),
    /// Its size in texels, which is its size on the surface.
    pub(crate) size: (u32, u32),
    /// The bitmap's left edge, right of the pen.
    pub(crate) left: i32,
    /// The bitmap's top edge, above the pen's baseline.
    pub(crate) top: i32,
}

/// An entry: a placed bitmap, or the memory that the key has none.
struct Entry {
    /// `None` for a glyph with nothing to draw.
    placed: Option<(Slot, AllocId)>,
    /// The frame that last asked for it.
    last_used: u64,
}

/// One texture page and its packer.
struct AtlasPage {
    page: Page,
    side: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    packer: BucketedAtlasAllocator,
    /// Bitmaps written to the current texture's predecessors and to it,
    /// counted by the write itself ([`AtlasPage::upload`]), so a count
    /// read from here cannot move without a write.
    written: u64,
}

impl AtlasPage {
    fn new(device: &wgpu::Device, page: Page, side: u32) -> Self {
        let (texture, view) = Self::texture(device, page, side);
        AtlasPage {
            page,
            side,
            texture,
            view,
            packer: BucketedAtlasAllocator::new(size2(side as i32, side as i32)),
            written: 0,
        }
    }

    fn texture(device: &wgpu::Device, page: Page, side: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(match page {
                Page::Color => "glyph_runs color atlas",
                Page::Mask => "glyph_runs mask atlas",
            }),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: page.format(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    /// Copy `data`, a `size` bitmap in this page's format, to `uv`, and
    /// count the write.
    fn upload(&mut self, queue: &wgpu::Queue, uv: (u32, u32), size: (u32, u32), data: &[u8]) {
        self.written += 1;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: uv.0,
                    y: uv.1,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.0 * self.page.bytes_per_texel()),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// Running totals since the atlas was made, for the structural
/// assertions the GPU-backed tests make (TEST_CONVENTIONS §T8).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AtlasCounts {
    /// Keys asked for and found.
    pub(crate) hits: u64,
    /// Keys asked for and not found: each one rasterized once.
    pub(crate) rasterized: u64,
    /// Bitmaps written to a page, growth's re-uploads excluded.
    pub(crate) uploaded: u64,
    /// Times a page doubled.
    pub(crate) growths: u64,
    /// Bitmaps written again into a grown page: every entry it held.
    /// Read back from the page's own write count, so only a write moves
    /// it.
    pub(crate) reuploaded: u64,
    /// Bitmaps evicted from a full page to make room.
    pub(crate) evicted: u64,
    /// Entries without a bitmap forgotten by an eviction pass: read from
    /// the map's own length, so only a removal moves it.
    pub(crate) forgotten: u64,
    /// Glyphs refused for want of room this frame: a page at its
    /// largest holding only the last two frames' glyphs. Not remembered.
    pub(crate) refused: u64,
    /// Glyphs whose bitmap is larger than the largest page: remembered
    /// as drawing nothing.
    pub(crate) oversized: u64,
}

/// What [`GlyphAtlas::place`] made of a key.
enum Placed {
    /// A bitmap, on a page.
    Bitmap(Slot, AllocId),
    /// Nothing to draw, now or ever: no bitmap, an empty one, content
    /// this atlas does not hold, or a bitmap larger than any page.
    Nothing,
    /// No room this frame.
    NoRoom,
}

/// The atlas: two pages, the entries on them, and the bind group the
/// glyph pipeline samples them through.
pub(crate) struct GlyphAtlas {
    color: AtlasPage,
    mask: AtlasPage,
    entries: FxHashMap<CacheKey, Entry>,
    max_side: u32,
    frame: u64,
    sampler: wgpu::Sampler,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    /// Whether a refusal has been logged since the last frame that
    /// refused nothing, so a page full for good warns once.
    refusal_logged: bool,
    refused_this_frame: bool,
    /// Whether an oversized glyph has been logged. Each is remembered,
    /// so the warning is once for the atlas's life.
    oversized_logged: bool,
    counts: AtlasCounts,
}

impl GlyphAtlas {
    /// An empty atlas of two [`INITIAL_SIDE`] pages on `device`.
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let max_side = device.limits().max_texture_dimension_2d;
        let side = INITIAL_SIDE.min(max_side);
        let color = AtlasPage::new(device, Page::Color, side);
        let mask = AtlasPage::new(device, Page::Mask, side);
        // Nearest: a glyph quad covers exactly its bitmap's texels on
        // whole surface pixels, so there is nothing to filter.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glyph_runs atlas sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                view_dimension: wgpu::TextureViewDimension::D2,
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glyph_runs atlas layout"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = Self::bind(device, &layout, &color, &mask, &sampler);
        GlyphAtlas {
            color,
            mask,
            entries: FxHashMap::default(),
            max_side,
            frame: 0,
            sampler,
            layout,
            bind_group,
            refusal_logged: false,
            refused_this_frame: false,
            oversized_logged: false,
            counts: AtlasCounts::default(),
        }
    }

    fn bind(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        color: &AtlasPage,
        mask: &AtlasPage,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glyph_runs atlas bind group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&mask.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    /// The layout of [`Self::bind_group`], for the pipeline.
    pub(crate) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// Both pages and the sampler, as the pipeline binds them.
    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// Stop both pages growing past the size they have now, as if the
    /// device allowed no larger texture: how a test reaches a full page
    /// without allocating the device's largest one.
    #[cfg(test)]
    pub(crate) fn freeze_growth(&mut self) {
        self.max_side = self.color.side.max(self.mask.side);
    }

    /// Running totals.
    pub(crate) fn counts(&self) -> AtlasCounts {
        self.counts
    }

    /// How many keys the atlas remembers, bitmaps and empties alike.
    pub(crate) fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Start a frame: entries asked for from here on are the frame's,
    /// and are not evicted under it.
    pub(crate) fn begin_frame(&mut self) {
        self.frame += 1;
        if !self.refused_this_frame {
            self.refusal_logged = false;
        }
        self.refused_this_frame = false;
    }

    /// Where `key`'s bitmap is, rasterizing and uploading it the first
    /// time it is asked for. `None` when the glyph draws nothing (no
    /// bitmap, an empty one, content this atlas does not hold, or a
    /// bitmap larger than any page — each remembered) or cannot be
    /// placed this frame (its page holds only the last two frames' glyphs at its
    /// largest size; warned once per episode, not remembered).
    ///
    /// Cost: one hash lookup on a hit. On a miss, one rasterization
    /// under the font guard the caller holds and one texture write; a
    /// page that must make room walks the entries once, evicting every
    /// one stale for two frames, and tries the allocation again; one
    /// that must grow re-rasterizes and re-uploads every entry on it.
    pub(crate) fn slot(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_system: &mut FontSystem,
        swash: &mut SwashCache,
        key: CacheKey,
    ) -> Option<Slot> {
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = self.frame;
            self.counts.hits += 1;
            return entry.placed.map(|(slot, _)| slot);
        }
        self.counts.rasterized += 1;
        let placed = match self.place(device, queue, font_system, swash, key) {
            Placed::Bitmap(slot, id) => Some((slot, id)),
            Placed::Nothing => None,
            // Not remembered: a later frame may have room for it.
            Placed::NoRoom => return None,
        };
        self.entries.insert(
            key,
            Entry {
                placed,
                last_used: self.frame,
            },
        );
        placed.map(|(slot, _)| slot)
    }

    /// Rasterize `key` and find it room: evict what the frame has not
    /// used, and grow only when that is not enough.
    fn place(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_system: &mut FontSystem,
        swash: &mut SwashCache,
        key: CacheKey,
    ) -> Placed {
        let Some(image) = swash.get_image_uncached(font_system, key) else {
            return Placed::Nothing;
        };
        let Some(page) = Page::of(image.content) else {
            return Placed::Nothing;
        };
        let size = (image.placement.width, image.placement.height);
        if size.0 == 0 || size.1 == 0 {
            return Placed::Nothing;
        }
        // A bitmap whose bytes do not match its size would be read past
        // its end by the texture write.
        if image.data.len() as u64
            != u64::from(size.0) * u64::from(size.1) * u64::from(page.bytes_per_texel())
        {
            log::warn!(
                "renderer: glyph_runs: a rasterized glyph's {} bytes do not fill its {}×{} bitmap; not drawn",
                image.data.len(),
                size.0,
                size.1
            );
            return Placed::Nothing;
        }
        // A bitmap larger than the largest page can never be placed;
        // evicting and growing for it would only empty the atlas.
        if size.0 > self.max_side || size.1 > self.max_side {
            self.counts.oversized += 1;
            if !self.oversized_logged {
                self.oversized_logged = true;
                log::warn!(
                    "renderer: glyph_runs: a {}×{} glyph is larger than the largest atlas page, {2}×{2}; \
                     it is not drawn",
                    size.0,
                    size.1,
                    self.max_side
                );
            }
            return Placed::Nothing;
        }
        let (id, uv) = loop {
            if let Some(allocation) = allocate(self.page(page), size) {
                break allocation;
            }
            if let Some(allocation) = self.evict_for(page, size) {
                break allocation;
            }
            if self.grow(device, queue, font_system, swash, page) {
                continue;
            }
            self.refuse(page);
            return Placed::NoRoom;
        };
        let slot = Slot {
            page,
            uv,
            size,
            left: image.placement.left,
            top: image.placement.top,
        };
        self.page(page).upload(queue, slot.uv, size, &image.data);
        self.counts.uploaded += 1;
        Placed::Bitmap(slot, id)
    }

    /// Count a glyph there is no room for, and warn once per episode
    /// of refusals.
    fn refuse(&mut self, page: Page) {
        self.counts.refused += 1;
        self.refused_this_frame = true;
        if !self.refusal_logged {
            self.refusal_logged = true;
            log::warn!(
                "renderer: glyph_runs: a glyph does not fit the {page:?} atlas, at most {0}×{0} with \
                 the last two frames' glyphs kept; it is not drawn",
                self.max_side
            );
        }
    }

    fn page(&mut self, page: Page) -> &mut AtlasPage {
        match page {
            Page::Color => &mut self.color,
            Page::Mask => &mut self.mask,
        }
    }

    /// Double `page` (up to the device's limit), rasterize every entry
    /// on it into the new texture, and rebind. `false` when it is
    /// already at the limit.
    fn grow(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_system: &mut FontSystem,
        swash: &mut SwashCache,
        page: Page,
    ) -> bool {
        let max_side = self.max_side;
        let target = match page {
            Page::Color => &mut self.color,
            Page::Mask => &mut self.mask,
        };
        if target.side >= max_side {
            return false;
        }
        let side = (target.side * 2).min(max_side);
        target.packer.grow(size2(side as i32, side as i32));
        let (texture, view) = AtlasPage::texture(device, page, side);
        target.texture = texture;
        target.view = view;
        target.side = side;
        self.counts.growths += 1;
        log::debug!("renderer: glyph_runs: {page:?} atlas grew to {side}×{side}");
        // Every entry on this page is still at its texel; the new
        // texture is empty, so each is rasterized and written again.
        let written_before = target.written;
        for (key, entry) in &self.entries {
            let Some((slot, _)) = entry.placed else {
                continue;
            };
            if slot.page != page {
                continue;
            }
            if let Some(image) = swash.get_image_uncached(font_system, *key) {
                if image.data.len() as u64
                    == u64::from(slot.size.0) * u64::from(slot.size.1) * u64::from(page.bytes_per_texel())
                {
                    target.upload(queue, slot.uv, slot.size, &image.data);
                }
            }
        }
        self.counts.reuploaded += target.written - written_before;
        self.bind_group = Self::bind(device, &self.layout, &self.color, &self.mask, &self.sampler);
        true
    }

    /// Make room for a `size` bitmap on `page` by evicting every bitmap
    /// of the page stale for two frames — drawn by neither this frame
    /// nor the one before ([`PROTECTED_FRAMES`]) — and return the
    /// allocation if it now fits. `None` when there was nothing to evict
    /// or the room freed is not enough: every bitmap left belongs to the
    /// last two frames, and the page must grow.
    ///
    /// All of them at once rather than one at a time: etagere's
    /// bucketed packer frees a column only when every item in it is
    /// gone, so evicting a few old glyphs at a time leaves the page
    /// fragmented — partly free columns no new glyph fits — where
    /// clearing everything stale hands the frame's new glyphs whole
    /// columns. A page fills only when the view moved on (a zoom, a new
    /// region), and what it held is what that view no longer shows.
    ///
    /// The same pass forgets every bitmap-less entry stale for two
    /// frames, so remembered spaces and oversized glyphs do not pile up
    /// across sizes.
    ///
    /// Cost: one walk of the entries, one deallocation per eviction and
    /// one allocation attempt. Only when a page is full.
    fn evict_for(&mut self, page: Page, size: (u32, u32)) -> Option<(AllocId, (u32, u32))> {
        let frame = self.frame;
        let target = match page {
            Page::Color => &mut self.color,
            Page::Mask => &mut self.mask,
        };
        let held = self.entries.len();
        let mut evicted = 0;
        self.entries.retain(|_, entry| {
            if entry.last_used + PROTECTED_FRAMES > frame {
                return true;
            }
            match entry.placed {
                None => false,
                Some((slot, id)) if slot.page == page => {
                    target.packer.deallocate(id);
                    evicted += 1;
                    false
                }
                Some(_) => true,
            }
        });
        // Counted from what left the map, so the count cannot move
        // without the removal: every entry gone that was not a bitmap
        // evicted was a bitmap-less one forgotten.
        self.counts.evicted += evicted;
        self.counts.forgotten += (held - self.entries.len()) as u64 - evicted;
        if evicted == 0 {
            return None;
        }
        allocate(target, size)
    }
}

/// An allocation of `size` texels on `page`, as `(id, top-left)`.
fn allocate(page: &mut AtlasPage, size: (u32, u32)) -> Option<(AllocId, (u32, u32))> {
    let allocation = page.packer.allocate(size2(size.0 as i32, size.1 as i32))?;
    let min = allocation.rectangle.min;
    Some((allocation.id, (min.x as u32, min.y as u32)))
}
