// SPDX-License-Identifier: MPL-2.0

//! Frame rendering: one frame as a CPU half
//! ([`Renderer::prepare_frame`]) and a GPU half
//! ([`Renderer::encode_frame`]), parameterized over the camera that
//! projects the canvas and the texture it lands in, with their
//! geometry / vertex helpers. The window (`Renderer::render`) is one
//! caller of the pair and the offscreen capture is the other, so a
//! screenshot draws through the very code the window does.

use glam::Vec2;
use glyphon::{TextArea, TextBounds};
use log::debug;
use wgpu::StoreOp;

use baumhard::font::{fonts, COLOR_WHITE};

use super::backdrop::{self, PixelRect};
use super::scene_shape_cache::CanvasSource;
use super::Renderer;

use baumhard::gfx_structs::camera::Camera2D;
use glyphon::Resolution;

/// Proof that [`Renderer::prepare_frame`] ran for the frame
/// [`Renderer::encode_frame`] is about to draw: everything the frame
/// draws — both glyphon prepares and the glyph-run instances — was
/// staged by that call, and lives in the passes themselves, so there is
/// nothing else to hand over.
pub(super) struct PreparedFrame {
    _prepared: (),
}

/// Whether a frame draws the screen-space modal — the console or the
/// glyph-wheel color picker, whichever is open (they are mutually
/// exclusive). The window always does. A capture omits it in two cases:
/// taken *from* the console, because the prompt that asked for the
/// picture is not what the picture is of; and drawn at a size other than
/// the window's, because a modal is laid out against the window's size
/// and has no coherent place in another one. The FPS readout and the
/// mode-status line are never omitted: they sit at fixed offsets from
/// the top-left corner, which every size has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModalLayer {
    Drawn,
    // Native-only producer: the capture engine; the window always draws.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Omitted,
}

/// Why a frame was not drawn. The window asks again for a transient one
/// ([`FrameSkip::is_transient`]) — nothing else would, with no input to
/// follow — and otherwise leaves the redraw owed; a capture reports it,
/// because "nothing was drawn" and "black was drawn" look alike in a PNG
/// and must not to an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameSkip {
    /// The renderer has not been told to start (`RenderDecree::StartRender`
    /// has not arrived), or has been told to stop, so there is no scene
    /// to draw.
    NotStarted,
    /// The font system was held elsewhere when the text prepare asked
    /// for it. Transient: the next frame retries.
    FontSystemContended,
    /// A glyphon prepare failed — `AtlasFull` once the glyph atlas has
    /// reached the GPU's texture limit, which can be permanent.
    PrepareFailed,
}

impl FrameSkip {
    /// The wire word a capture reports it by (`format/ipc.md`
    /// §`capture.screenshot`, `skipped`).
    // Native-only consumer: the capture engine; the browser renders
    // through `render` alone, which asks only whether a skip is
    // transient.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn word(self) -> &'static str {
        match self {
            FrameSkip::NotStarted => "not_started",
            FrameSkip::FontSystemContended => "font_system_contended",
            FrameSkip::PrepareFailed => "prepare_failed",
        }
    }

    /// Whether the same frame asked for again would likely draw: only a
    /// contended font system, which its holder releases. Retrying a
    /// renderer that has not started, or a prepare that can fail for
    /// good, would spin the loop.
    pub(crate) fn is_transient(self) -> bool {
        matches!(self, FrameSkip::FontSystemContended)
    }
}

impl Renderer {
    /// Draw one frame to the window: [`Self::prepare_frame`] at the live
    /// camera, then [`Self::encode_frame`] into the swapchain texture,
    /// then present. The window is one caller of the pair; an offscreen
    /// capture (`renderer::capture`) is the other, with its own camera
    /// and target. `Err` names why the frame was not drawn
    /// ([`Renderer::process`]).
    ///
    /// With no window ([`Sink::Headless`](super::Sink)) there is nothing
    /// to draw into, so the frame draws nothing: no prepare, no encode.
    /// It counts as drawn all the same — the frame is current, in the
    /// only sense a frame nobody can see has — and a capture prepares
    /// and draws its own (`renderer::capture`), so pixels are produced
    /// exactly when they are asked for.
    #[inline]
    pub(super) fn render(&mut self) -> Result<(), FrameSkip> {
        if !self.should_render {
            return Err(FrameSkip::NotStarted);
        }
        if !self.sink.presents() {
            return Ok(());
        }
        let camera = self.camera;
        // On lock contention or prepare failure, skip the rest of this
        // frame — before acquiring a surface texture, so a skipped
        // frame takes no vsync backpressure.
        let prepared = self.prepare_frame(&camera, ModalLayer::Drawn, CanvasSource::Live)?;
        let frame = match self.sink.current_texture() {
            Some(wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t)) => t,
            other => {
                // Drawn all the same: the buffers are current, and what
                // is missing is somewhere to present them, which an
                // occluded window does not have.
                debug!("Failed to get the surface texture ({other:?}), can't render.");
                self.present_nowhere();
                return Ok(());
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let encoder = self.encode_frame(&prepared, &view);
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        self.atlas.trim();
        Ok(())
    }

    /// End a frame that was prepared and has nowhere to be presented — an
    /// occluded window's. The prepare staged its uploads (the glyph
    /// atlases, the text vertices, the glyph-run instances) on the queue, and wgpu
    /// holds staged writes until the next submission; with none to come
    /// until a capture asks, every such frame would add to them. An empty
    /// submission flushes them, as the frame's own would have.
    fn present_nowhere(&mut self) {
        self.queue.submit(std::iter::empty());
        self.atlas.trim();
    }

    /// The CPU half of a frame, for a target `camera.viewport_size`
    /// pixels large: size the glyphon viewport to the target, and run
    /// both glyphon prepares and the positioned-glyph pass's
    /// ([`Self::prepare_text_for_pass`]).
    ///
    /// **Everything the frame draws is projected through `camera`, and
    /// nothing else.** Canvas content goes through its transform and
    /// its cull ([`Camera2D::visibility`], via `visible_at`); the
    /// screen-space layer — console, color picker, FPS readout, mode
    /// status — is laid out in window pixels and drawn at
    /// `camera.pixel_ratio` of them, so a supersampled capture scales
    /// it with the canvas. For the window, `camera` is the live one and
    /// the ratio is 1.
    ///
    /// With [`ModalLayer::Omitted`], an open modal's backdrop and glyphs
    /// are left out of the frame; nothing else changes. A backdrop is
    /// glyphs too — full blocks at the bottom of the chrome pass
    /// ([`super::backdrop`]) — so a frame draws its clear and glyphs and
    /// nothing else (V1-B12).
    ///
    /// `canvas` names which shaped canvas set the main pass draws the
    /// tree-rendered roles from: the window's live set, or a capture's
    /// composition of it with a scratch projection
    /// ([`CanvasSource`]). The node text is the renderer's own and is
    /// drawn either way.
    ///
    /// Leaves the glyphon viewport at the target's resolution; the next
    /// frame's prepare sets it to its own. `Viewport::update` writes
    /// only on a change, so the window's frames pay a compare.
    pub(super) fn prepare_frame(
        &mut self,
        camera: &Camera2D,
        modal: ModalLayer,
        canvas: CanvasSource,
    ) -> Result<PreparedFrame, FrameSkip> {
        let (width, height) = camera.viewport_size;
        self.viewport.update(&self.queue, Resolution { width, height });

        // Collect text areas + run both glyphon prepares against
        // the atlas. Shared with `prewarm()` (through `render`) so the
        // warm-up sees exactly the same glyph set that the next real
        // frame will draw — no risk of warming a different set than
        // gets rasterized.
        self.prepare_text_for_pass(camera, modal, canvas)?;
        Ok(PreparedFrame { _prepared: () })
    }

    /// The GPU half of a frame: one render pass into `view`, cleared to
    /// the map's background ([`Self::set_background`]), drawing what
    /// [`Self::prepare_frame`] uploaded.
    /// Returns the encoder unsubmitted, so a caller can record more
    /// after the pass — the capture records its texture-to-buffer copy
    /// there.
    ///
    /// `view` must be a texture of the surface format, the one every
    /// pipeline was built for, at the size the frame was prepared for.
    pub(super) fn encode_frame(
        &self,
        _prepared: &PreparedFrame,
        view: &wgpu::TextureView,
    ) -> wgpu::CommandEncoder {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // 1. Main text pass — node text, borders, connections,
            //    edge handles, camera-transformed and screen-space
            //    overlays, straight onto the cleared background: a map
            //    draws no fill (V1-B12).
            //    Interactive path: log and continue on render failure
            //    so a single bad atlas frame doesn't crash the editor.
            //    Unreachable as of glyphon 0.12.0 — `TextRenderer::
            //    render` returns `Ok(())` on every path and neither
            //    `RenderError` variant is ever constructed — so this
            //    is forward-compat cover, not a live warn, and it
            //    carries no frame-rate spam risk today. Kept because
            //    the signature is fallible and a future glyphon may
            //    start using it.
            if let Err(e) = self.text_renderer.render(&self.atlas, &self.viewport, &mut pass) {
                log::warn!("renderer: text_renderer.render failed: {e}");
            }

            // 2. Positioned glyph runs — every glyph of every
            //    `GlyphRun` at its own canvas position, one instanced
            //    draw. Canvas content, so under the chrome below.
            self.glyph_runs.render(&mut pass);

            // 3. Chrome pass — an open modal's backdrop (full-block
            //    cells, `backdrop`), then its glyphs: the console's
            //    cyan border, query line and rows, or the picker's
            //    wheel; then the FPS readout and the mode-status line.
            //    Glyphon draws a pass's glyphs in the order they were
            //    prepared, so the backdrop covers every canvas glyph
            //    and every chrome glyph sits on it.
            //    Interactive path: log and continue on render failure.
            //    Unreachable in glyphon 0.12.0 for the same reason as
            //    the main pass above; kept as forward-compat cover.
            if let Err(e) = self
                .console_text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
            {
                log::warn!("renderer: console_text_renderer.render failed: {e}");
            }
        }
        encoder
    }

    /// Collect every text-area `camera` shows from the renderer's
    /// buffer maps (mindmap nodes, overlays, canvas-scene — which
    /// carries borders, connections, labels, and handles through the
    /// tree pipeline — console, color picker, FPS overlay) and run both
    /// `text_renderer.prepare()` and `console_text_renderer.prepare()`
    /// against the atlas; then, under the same font guard, prepare the
    /// positioned-glyph pass over the canvas set's `GlyphRun`s
    /// ([`super::glyph_runs`]). Returns the [`FrameSkip`] (and the
    /// caller skips the rest of its frame) on font-system lock
    /// contention or on either glyphon prepare failing; the glyph-run
    /// pass cannot fail the frame — a glyph it has no room for is left
    /// out and warned of once.
    ///
    /// Reached through [`Self::prepare_frame`], by `render()`
    /// (steady-state, and `prewarm()` through it) and by the offscreen
    /// capture, so every frame the app draws sees one glyph set rule.
    ///
    /// Note: this performs disjoint mutable borrows of
    /// `text_renderer`, `atlas`, `glyph_runs` and `swash_cache` while holding
    /// immutable borrows of the buffer-map fields; the borrow
    /// checker permits this within a single `&mut self` body but
    /// would reject it across a method boundary, which is why the
    /// prepare call lives in this method rather than splitting into
    /// a separate collection helper.
    fn prepare_text_for_pass(
        &mut self,
        camera: &Camera2D,
        modal_layer: ModalLayer,
        canvas: CanvasSource,
    ) -> Result<(), FrameSkip> {
        let vp_w = camera.viewport_size.0 as i32;
        let vp_h = camera.viewport_size.1 as i32;
        let ratio = camera.pixel_ratio;
        let vp_bounds = TextBounds {
            left: 0,
            top: 0,
            right: vp_w,
            bottom: vp_h,
        };
        let default_color = COLOR_WHITE;

        // Collect "main" text areas: the mindmap node buffers +
        // overlays + canvas-scene arena buffers (borders,
        // connections, labels, portals, handles all arrive here
        // through the tree walker).
        // Chrome buffers go into a separate list so they render in a
        // second glyphon pass, over the glyph-run pass that draws
        // between the two (hence the split).
        // Upper-bound capacity so the per-frame `Vec` doesn't grow
        // through several reallocs. Visibility-culling reduces the
        // realized count below this; allocating once at the ceiling
        // is still cheaper than `push` reallocations.
        let canvas_elements = || {
            super::scene_shape_cache::canvas_elements(
                canvas,
                &self.canvas_scene_elements,
                &self.capture_canvas,
            )
        };
        let main_capacity = self.mindmap_buffers.values().map(|v| v.len()).sum::<usize>()
            + self.overlay_buffers.len()
            + super::scene_shape_cache::buffer_count(canvas_elements());
        let mut main_text_areas: Vec<TextArea> = Vec::with_capacity(main_capacity);
        main_text_areas.extend(
            self.mindmap_buffers
                .values()
                .flat_map(|v| v.iter())
                .chain(self.overlay_buffers.iter())
                .chain(super::scene_shape_cache::text_buffers(canvas_elements()))
                .filter_map(|tb| {
                    if !tb.visible_at(camera) {
                        return None;
                    }
                    let canvas_pos = Vec2::new(tb.pos.0, tb.pos.1);
                    let screen_pos = camera.canvas_to_screen(canvas_pos);
                    Some(TextArea {
                        buffer: &tb.buffer,
                        left: screen_pos.x,
                        top: screen_pos.y,
                        scale: camera.pixels_per_unit(),
                        bounds: vp_bounds,
                        default_color,
                        custom_glyphs: &[],
                    })
                }),
        );

        // Chrome: screen-space text, drawn in its own glyphon pass over
        // the canvas's — first an open modal's backdrop cells, so they
        // are under everything else here. Both the
        // console and the glyph-wheel color picker put their glyph
        // buffers in `overlay_scene_elements` (populated by
        // `rebuild_overlay_scene_buffers` from the respective
        // overlay tree in `AppScene`) — they are mutually exclusive
        // screen-space modals sharing this one pass — which is also
        // why omitting the modal omits the whole sub-scene: it holds
        // nothing else.
        let modal: &[super::scene_shape_cache::ShapedSceneElement] = match modal_layer {
            ModalLayer::Drawn => &self.overlay_scene_elements,
            ModalLayer::Omitted => &[],
        };
        // The backdrops, in this target's pixels: the console's and the
        // picker's (mutually exclusive, but either or both is drawn the
        // same way), each the pixels its window rectangle covers at this
        // camera's ratio, cut to the target, as full-block cells clipped
        // to it.
        let backdrops: [Option<PixelRect>; 2] = match modal_layer {
            ModalLayer::Drawn => [self.console_backdrop, self.color_picker_backdrop]
                .map(|extent| extent.map(|extent| PixelRect::covered_by(extent, ratio).within(vp_w, vp_h))),
            ModalLayer::Omitted => [None, None],
        };
        let backdrop_cells: usize = backdrops.iter().flatten().map(PixelRect::cell_count).sum();
        let palette_capacity = backdrop_cells
            + super::scene_shape_cache::buffer_count(modal)
            + self.fps_overlay_buffers.len()
            + self.mode_status_overlay_buffers.len();
        let mut palette_text_areas: Vec<TextArea> = Vec::with_capacity(palette_capacity);
        if let Some(cell) = &self.backdrop_cell {
            for rect in backdrops.into_iter().flatten() {
                palette_text_areas.extend(backdrop::text_areas(cell, rect));
            }
        }
        palette_text_areas.extend(
            super::scene_shape_cache::text_buffers(modal)
                .chain(self.fps_overlay_buffers.iter())
                .chain(self.mode_status_overlay_buffers.iter())
                .map(|tb| TextArea {
                    buffer: &tb.buffer,
                    left: tb.pos.0 * ratio,
                    top: tb.pos.1 * ratio,
                    scale: ratio,
                    bounds: vp_bounds,
                    default_color,
                    custom_glyphs: &[],
                }),
        );

        // Interactive path: a contended font-system lock must skip
        // the frame, not abort the process. `debug!`, not `warn!`,
        // per CODE_CONVENTIONS §9: this runs once per frame, the
        // skip is a designed transient that the next frame retries,
        // and there is nothing a user could act on — a `warn!` here
        // would flood stderr at frame rate for no diagnostic gain.
        // `rebuild_mode_status_overlay_if_needed` takes the same
        // contention path silently for the same reason.
        let Some(mut font_system) = fonts::try_acquire_font_system_write("render::prepare_text_for_pass")
        else {
            log::debug!("renderer: font_system lock contended in prepare_text_for_pass, skipping");
            return Err(FrameSkip::FontSystemContended);
        };

        // The test build's census of the fill blocks each pass is handed
        // (`backdrop::FillCensus`), counted from the very lists the two
        // prepares below draw and the very runs the glyph-run pass does.
        // The fill glyph is read off the backdrop cell, shaped here if no
        // backdrop has needed it yet, so a frame with no modal counts too.
        #[cfg(test)]
        {
            let shaped_here;
            let cell = match &self.backdrop_cell {
                Some(cell) => cell,
                None => {
                    shaped_here = backdrop::BackdropCell::shape(&mut font_system);
                    &shaped_here
                }
            };
            let fill = backdrop::FillGlyph::of(cell).expect("the backdrop cell lays out its block");
            let areas = |areas: &[TextArea]| {
                areas
                    .iter()
                    .map(|area| fill.in_buffer(area.buffer))
                    .sum::<usize>()
            };
            let runs: usize = super::scene_shape_cache::glyph_runs(canvas_elements())
                .map(|(run, _)| fill.in_run(run))
                .sum();
            self.fill_census = backdrop::FillCensus {
                canvas: areas(&main_text_areas) + runs,
                chrome: areas(&palette_text_areas),
            };
        }

        // Interactive path: a glyphon prepare failure must degrade the
        // frame, not abort the process. Skip the whole render so we
        // don't run a half-prepared atlas through the GPU.
        //
        // Logged once per fault episode, not once per frame. Unlike
        // the `render()` failures above, `PrepareError::AtlasFull` is
        // reachable and can be *permanent*: glyphon only reports it
        // after `TextAtlas::grow()` returns `false`, which it does
        // unconditionally once the atlas has reached
        // `max_texture_dimension_2d` — commonly 4096 on WebGL2, a
        // first-class target under CODE_CONVENTIONS §4. Returning
        // `false` here makes the caller bail before
        // `get_current_texture()` / `present()`, so the skipped frame
        // takes no vsync backpressure either: the loop is uncapped
        // and runs under `ControlFlow::Poll`, so it spins at CPU rate
        // during a drag or animation and an
        // unguarded `warn!` would write a line per spin. The flag
        // clears on the next successful prepare, so a transient fault
        // logs again if it returns.
        if let Err(e) = self.text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut font_system,
            &mut self.atlas,
            &self.viewport,
            main_text_areas,
            &mut self.swash_cache,
        ) {
            if !self.prepare_fault_logged {
                self.prepare_fault_logged = true;
                log::warn!("renderer: text_renderer.prepare failed, skipping frame: {e}");
            }
            return Err(FrameSkip::PrepareFailed);
        }
        if let Err(e) = self.console_text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut font_system,
            &mut self.atlas,
            &self.viewport,
            palette_text_areas,
            &mut self.swash_cache,
        ) {
            if !self.prepare_fault_logged {
                self.prepare_fault_logged = true;
                log::warn!("renderer: console_text_renderer.prepare failed, skipping frame: {e}");
            }
            return Err(FrameSkip::PrepareFailed);
        }
        // The positioned glyph runs, under the same guard: they shape
        // nothing, but a glyph the atlas has not seen is rasterized.
        self.glyph_runs.prepare(
            &self.device,
            &self.queue,
            &mut font_system,
            &mut self.swash_cache,
            camera,
            super::scene_shape_cache::glyph_runs(canvas_elements()),
        );
        drop(font_system);
        self.prepare_fault_logged = false;
        Ok(())
    }

    /// Pre-warm the render pipeline during map load: run one full
    /// `render()` cycle (glyph atlas upload → text and glyph-run
    /// pipeline binds → `surface.get_current_texture()` →
    /// `frame.present()`). Forces
    /// the wgpu driver to compile pipeline shaders and the
    /// swapchain to allocate its first backing image — both are
    /// commonly 50-300ms costs (Vulkan/Metal/D3D12 lazily compile
    /// SPIR-V→GPU-ISA on first pipeline bind, Mesa especially) that
    /// would otherwise land on the user's first interaction.
    ///
    /// The original `prewarm_atlas` (gone) skipped the render pass to
    /// avoid `atlas.trim()` evicting freshly warmed glyphs. The
    /// concern was unfounded but for a reason worth stating
    /// precisely: glyphon's `TextAtlas::trim()` doesn't evict at
    /// all — it just clears the per-frame `InnerAtlas::glyphs_in_use` set
    /// (text_atlas.rs `mask_atlas.trim` / `color_atlas.trim`
    /// are `self.glyphs_in_use.clear()`). The glyph cache itself
    /// only evicts under packer pressure inside `InnerAtlas::try_allocate`,
    /// and the `InnerAtlas::glyphs_in_use` set is populated by
    /// `text_renderer.prepare()` — not `render()`. So warmed
    /// glyphs sit in the LRU cache regardless of whether a draw
    /// happened, and the prior atlas-only path was already safe;
    /// what the full render adds is shader compile and the
    /// swapchain allocation, which `prepare` alone doesn't reach.
    ///
    /// With no window the frame draws nothing ([`Self::render`]), so
    /// there is nothing to warm: the pipelines compile and the atlas
    /// fills on the first capture, which is the first frame a headless
    /// app draws pixels for at all.
    ///
    /// Caller must have already populated the canvas-scene buffers
    /// (`flush_canvas_scene_buffers`) and dispatched
    /// `RenderDecree::StartRender`. If buffers are empty (doc-load
    /// failure path) this is a cheap no-op (one empty draw + one
    /// blank present).
    ///
    /// **Failure handling.** The whole call is wrapped in
    /// `catch_unwind`: prewarm runs *before the window is
    /// visible*, so if the first render's pipeline compile or
    /// `frame.present()` panics on a flaky driver, we'd otherwise
    /// abort startup with no UI feedback. `render()`'s own surface-
    /// acquisition failures already early-return cleanly (see
    /// `render()`'s `get_current_texture` match), but anything
    /// deeper in wgpu/glyphon that panics would propagate. Catching
    /// the panic and continuing means a degraded first frame
    /// instead of a dead app — the next real `process()` cycle
    /// retries from a known state.
    ///
    /// **Except when the panic took the font system with it.** A panic
    /// raised while `prepare_text_for_pass` held the `FONT_SYSTEM`
    /// guard leaves the lock poisoned, and no state is known after
    /// that: every later acquire panics, so the first frame that
    /// rebuilds anything would end the process with a message about
    /// the poison instead of its cause. That panic is re-raised here
    /// with its own payload, so the report names what actually failed.
    pub fn prewarm(&mut self) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.render()));
        if let Err(payload) = result {
            if fonts::font_system_poisoned() {
                log::error!(
                    "renderer: prewarm: first render panicked while holding the font system, \
                     which leaves it unusable; re-raising the original panic"
                );
                std::panic::resume_unwind(payload);
            }
            log::warn!(
                "renderer: prewarm: first render panicked; continuing with cold pipelines. \
                 First user interaction may show a one-frame stutter."
            );
        }
    }
}
