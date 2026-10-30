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
use baumhard::util::color_conversion::convert_u8_to_f32;

use super::scene_shape_cache::CanvasSource;
use super::{Renderer, RECT_VBUF_INITIAL_CAPACITY, RECT_VERTEX_FLOATS};

use baumhard::gfx_structs::camera::Camera2D;
use baumhard::gfx_structs::shape::SHAPE_ID_RECTANGLE;
use glyphon::Resolution;

/// Where [`Renderer::prepare_frame`] left one frame's two rect batches in
/// the shared vertex buffer — `[main | palette]` — for
/// [`Renderer::encode_frame`] to draw.
pub(super) struct PreparedFrame {
    main_bytes: u64,
    palette_bytes: u64,
    main_vertices: u32,
    palette_vertices: u32,
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
    /// Push the six vertices (two triangles) of a filled axis-aligned
    /// rectangle into `out`. Coords are already in NDC — the caller is
    /// responsible for any camera or screen→NDC transform. `color` is
    /// the flat RGBA written to every vertex; `shape_id` selects the
    /// fragment-shader path (`0` = rectangle, `1` = ellipse, …). The
    /// per-vertex `uv` is hard-wired to the quad's local `[0, 1]²`
    /// frame so the fragment shader can evaluate any SDF in the shape
    /// table without extra uniforms.
    ///
    /// Layout per vertex: `[x, y, u, v, r, g, b, a, shape_id]`
    /// (9 × 4 bytes = 36 bytes; must match `RECT_VERTEX_SIZE`).
    /// `shape_id` rides the stream as a plain `f32` (`shape_id as f32`)
    /// because wgpu's WebGL2 backend doesn't support integer vertex
    /// attributes on every browser — the WGSL vertex stage rounds and
    /// casts to `u32` before flat-interpolating. The round-trip is
    /// lossless for the small integer range we use.
    fn push_rect_ndc(out: &mut Vec<f32>, ndc_min: Vec2, ndc_max: Vec2, color: [f32; 4], shape_id: u32) {
        // Triangle 1: TL, BL, BR
        // Triangle 2: TL, BR, TR
        //
        // NDC y is UP, so "top" is the larger y. The caller computes
        // ndc_min / ndc_max from the canonical top-left + size by
        // flipping y during the screen-to-NDC transform, so here
        // ndc_min is bottom-left and ndc_max is top-right. Unpack:
        let (lx, ly) = (ndc_min.x, ndc_min.y); // bottom-left
        let (rx, ry) = (ndc_max.x, ndc_max.y); // top-right
        let [r, g, b, a] = color;
        // `shape_id` is encoded as `Float32` in the vertex buffer;
        // WGSL rounds + casts back to `u32` before the switch. See
        // the type-level doc on this function for the rationale.
        let sid = shape_id as f32;
        // UVs match the quad's local frame: TL = (0, 0), TR = (1, 0),
        // BR = (1, 1), BL = (0, 1). The SDF cases in the fragment
        // shader assume exactly this parameterization.
        let push = |out: &mut Vec<f32>, x: f32, y: f32, u: f32, v: f32| {
            out.extend_from_slice(&[x, y, u, v, r, g, b, a, sid]);
        };
        // Triangle 1: TL, BL, BR
        push(out, lx, ry, 0.0, 0.0);
        push(out, lx, ly, 0.0, 1.0);
        push(out, rx, ly, 1.0, 1.0);
        // Triangle 2: TL, BR, TR
        push(out, lx, ry, 0.0, 0.0);
        push(out, rx, ly, 1.0, 1.0);
        push(out, rx, ry, 1.0, 0.0);
    }

    /// Convert a screen-space rectangle (top-left + size in pixels)
    /// into a NDC bounding pair. Y is flipped so "top" (small y on
    /// screen) maps to "top" (large y in NDC).
    fn screen_rect_to_ndc_bounds(
        left: f32,
        top: f32,
        width: f32,
        height: f32,
        vp_w: f32,
        vp_h: f32,
    ) -> (Vec2, Vec2) {
        let x0 = left / vp_w * 2.0 - 1.0;
        let x1 = (left + width) / vp_w * 2.0 - 1.0;
        // Screen y grows down; NDC y grows up. Invert.
        let y_top = 1.0 - top / vp_h * 2.0;
        let y_bottom = 1.0 - (top + height) / vp_h * 2.0;
        // ndc_min = (x0, y_bottom), ndc_max = (x1, y_top)
        (Vec2::new(x0, y_bottom), Vec2::new(x1, y_top))
    }
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
    /// occluded window's. The prepare staged its uploads (both rect
    /// batches, the glyph atlas, the text vertices) on the queue, and wgpu
    /// holds staged writes until the next submission; with none to come
    /// until a capture asks, every such frame would add to them. An empty
    /// submission flushes them, as the frame's own would have.
    fn present_nowhere(&mut self) {
        self.queue.submit(std::iter::empty());
        self.atlas.trim();
    }

    /// The CPU half of a frame, for a target `camera.viewport_size`
    /// pixels large: rebuild both rect batches through `camera` and
    /// upload them, size the glyphon viewport to the target, and run
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
    /// are left out of the frame; nothing else changes.
    ///
    /// `canvas` names which shaped canvas set the main pass draws the
    /// tree-rendered roles from: the window's live set, or a capture's
    /// composition of it with a scratch projection
    /// ([`CanvasSource`]). The node text and backgrounds are the
    /// renderer's own and are drawn either way.
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
        let vp_w_px = width as f32;
        let vp_h_px = height as f32;
        self.viewport.update(&self.queue, Resolution { width, height });

        // Rebuild the "main" rect batch: canvas-space node
        // backgrounds transformed to NDC via `camera`.
        // Cheap: one push per visible node, no text shaping.
        // Visible-range test mirrors the text-area cull below so
        // clipped-offscreen nodes don't waste vertices either.
        self.main_rect_vertices.clear();
        for rect in self
            .node_background_rects
            .iter()
            .chain(super::scene_shape_cache::background_fills(
                super::scene_shape_cache::canvas_elements(
                    canvas,
                    &self.canvas_scene_elements,
                    &self.capture_canvas,
                ),
            ))
        {
            if !rect.visible_at(camera) {
                continue;
            }
            let screen_tl = camera.canvas_to_screen(rect.position);
            let screen_size = rect.size * camera.pixels_per_unit();
            let (ndc_min, ndc_max) = Self::screen_rect_to_ndc_bounds(
                screen_tl.x,
                screen_tl.y,
                screen_size.x,
                screen_size.y,
                vp_w_px,
                vp_h_px,
            );
            let color = convert_u8_to_f32(&rect.color);
            Self::push_rect_ndc(
                &mut self.main_rect_vertices,
                ndc_min,
                ndc_max,
                color,
                rect.shape_id,
            );
        }

        // Rebuild the "palette" rect batch: one opaque backdrop
        // behind the command palette and/or the glyph-wheel color
        // picker, in screen space at the camera's pixel ratio. The two
        // modals are mutually exclusive but a single batch handles
        // either case (or both, if a future variant ever overlaps
        // them). Both are pitch black: the palette's cyan frame and the
        // picker's saturated hue-ring glyphs pop against true black
        // with no tinting, over any canvas background.
        self.console_rect_vertices.clear();
        let ratio = camera.pixel_ratio;
        let backdrops = match modal {
            ModalLayer::Drawn => self
                .console_backdrop
                .into_iter()
                .chain(self.color_picker_backdrop),
            ModalLayer::Omitted => None.into_iter().chain(None),
        };
        for (left, top, w, h) in backdrops {
            let (ndc_min, ndc_max) = Self::screen_rect_to_ndc_bounds(
                left * ratio,
                top * ratio,
                w * ratio,
                h * ratio,
                vp_w_px,
                vp_h_px,
            );
            Self::push_rect_ndc(
                &mut self.console_rect_vertices,
                ndc_min,
                ndc_max,
                [0.0, 0.0, 0.0, 1.0],
                SHAPE_ID_RECTANGLE,
            );
        }

        // Upload both batches to the shared rect vertex buffer,
        // growing if the combined size exceeds the current
        // capacity. Layout: `[main_bytes | palette_bytes]`.
        let main_bytes_len = self.main_rect_vertices.len() * std::mem::size_of::<f32>();
        let palette_bytes_len = self.console_rect_vertices.len() * std::mem::size_of::<f32>();
        let total_bytes = (main_bytes_len + palette_bytes_len) as u64;
        if total_bytes > self.rect_vertex_buffer_capacity {
            let mut new_cap = self.rect_vertex_buffer_capacity.max(RECT_VBUF_INITIAL_CAPACITY);
            while new_cap < total_bytes {
                new_cap *= 2;
            }
            self.rect_vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rect_vertex_buffer"),
                size: new_cap,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.rect_vertex_buffer_capacity = new_cap;
        }
        if main_bytes_len > 0 {
            self.queue.write_buffer(
                &self.rect_vertex_buffer,
                0,
                bytemuck::cast_slice(&self.main_rect_vertices),
            );
        }
        if palette_bytes_len > 0 {
            self.queue.write_buffer(
                &self.rect_vertex_buffer,
                main_bytes_len as u64,
                bytemuck::cast_slice(&self.console_rect_vertices),
            );
        }

        // Collect text areas + run both glyphon prepares against
        // the atlas. Shared with `prewarm()` (through `render`) so the
        // warm-up sees exactly the same glyph set that the next real
        // frame will draw — no risk of warming a different set than
        // gets rasterized.
        self.prepare_text_for_pass(camera, modal, canvas)?;
        Ok(PreparedFrame {
            main_bytes: main_bytes_len as u64,
            palette_bytes: palette_bytes_len as u64,
            main_vertices: (self.main_rect_vertices.len() / RECT_VERTEX_FLOATS) as u32,
            palette_vertices: (self.console_rect_vertices.len() / RECT_VERTEX_FLOATS) as u32,
        })
    }

    /// The GPU half of a frame: one render pass into `view`, cleared to
    /// the canvas color, drawing what [`Self::prepare_frame`] uploaded.
    /// Returns the encoder unsubmitted, so a caller can record more
    /// after the pass — the capture records its texture-to-buffer copy
    /// there.
    ///
    /// `view` must be a texture of the surface format, the one both
    /// pipelines were built for, at the size the frame was prepared for.
    pub(super) fn encode_frame(
        &self,
        prepared: &PreparedFrame,
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

            // 1. Node backgrounds (rect pipeline, camera-transformed).
            if prepared.main_vertices > 0 {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_vertex_buffer(0, self.rect_vertex_buffer.slice(0..prepared.main_bytes));
                pass.draw(0..prepared.main_vertices, 0..1);
            }

            // 2. Main text pass — node text, borders, connections,
            //    edge handles, camera-transformed and screen-space
            //    overlays, all drawn on top of the node backgrounds.
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

            // 2b. Positioned glyph runs — every glyph of every
            //     `GlyphRun` at its own canvas position, one instanced
            //     draw. Canvas content, so under the palette below.
            self.glyph_runs.render(&mut pass);

            // 3. Palette backdrop (rect pipeline, screen-space).
            //    Drawn AFTER the main text pass so node text
            //    sitting behind the palette is fully occluded.
            if prepared.palette_vertices > 0 {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_vertex_buffer(
                    0,
                    self.rect_vertex_buffer
                        .slice(prepared.main_bytes..prepared.main_bytes + prepared.palette_bytes),
                );
                pass.draw(0..prepared.palette_vertices, 0..1);
            }

            // 4. Palette text pass — cyan border, query line,
            //    filtered action rows. Drawn on top of the palette
            //    backdrop so every glyph sits cleanly on solid fill.
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
        // Palette buffers go into a separate list so they render
        // in a second glyphon pass (with the backdrop rect
        // between them, hence the split).
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

        // Palette overlay: screen-space text, drawn in its own
        // glyphon pass so the rect-pipeline backdrop can be
        // interleaved between the main text and this one. Both the
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
        let palette_capacity = super::scene_shape_cache::buffer_count(modal)
            + self.fps_overlay_buffers.len()
            + self.mode_status_overlay_buffers.len();
        let mut palette_text_areas: Vec<TextArea> = Vec::with_capacity(palette_capacity);
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
    /// `render()` cycle (rect-vertex prep → glyph atlas upload →
    /// `pass.set_pipeline(rect_pipeline)` → text pipeline bind →
    /// `surface.get_current_texture()` → `frame.present()`). Forces
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
