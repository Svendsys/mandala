// SPDX-License-Identifier: MPL-2.0

//! Positioned glyph runs: the renderer's path for a `GlyphRun`
//! element, which draws every glyph of **one** shaped run at its own
//! canvas position (design §13.2).
//!
//! glyphon draws whole buffers, each at one position, from the layout
//! cosmic-text computed for it; it offers no way to put a buffer's
//! glyphs anywhere else. This path does: it reads the glyphs of the
//! run's one shaped buffer, puts each where the run's placement says
//! ([`emit`]), rasterizes each distinct glyph once into an atlas of its
//! own ([`atlas`]), and draws every glyph of every run in the frame as
//! one instanced quad each, in **one draw call** ([`GlyphRunPass`]).
//! Nothing is shaped here and nothing is rotated: the shaping happened
//! once, in `baumhard::flow`, and the placement only moved upright
//! glyphs.
//!
//! What it shares with glyphon's path: the `FontSystem` (under the
//! guard the frame already holds for glyphon's prepare), cosmic-text's
//! `SwashCache` (the rasterizer's scratch state), the render pass, the
//! surface format and the camera. What it does not share is the atlas,
//! because glyphon's is not reachable from outside it ([`atlas`]
//! explains). Instance data is plain floats, so the pipeline needs no
//! integer vertex attribute (a WebGL2 gap on some browsers, as the rect
//! pipeline notes) and no storage buffer (none on WebGL2).
//!
//! This is the V1-C1 prototype. It draws whatever `GlyphRun` elements
//! the canvas trees hold; no projection emits them yet (V1-B7, C3), so
//! the app draws exactly what it drew before.

pub(crate) mod atlas;
pub(crate) mod emit;
#[cfg(test)]
mod tests;

use std::borrow::Cow;

use baumhard::gfx_structs::camera::Camera2D;
use baumhard::gfx_structs::run::GlyphRunData;
use bytemuck::{Pod, Zeroable};
use glam::Vec2;
use glyphon::{FontSystem, LayoutGlyph, SwashCache};

use self::atlas::{AtlasCounts, GlyphAtlas, Page};
use self::emit::{emit_run, GlyphSprite};

/// The glyph pipeline's shader. One instance per glyph, drawn as a
/// four-vertex triangle strip; the instance carries the bitmap's
/// surface rectangle, its texel origin on its page, its color and
/// which page it is on.
///
/// Instance attributes are all `f32` — `page` included, rounded back
/// to an integer in the vertex stage — for the reason the rect
/// pipeline gives: integer vertex attributes are not available on
/// every WebGL2 browser. `vertex_index` is a builtin, not an attribute.
///
/// A mask glyph is its color with the mask as coverage. A color glyph
/// keeps its own colors and takes only the instance's alpha, which is
/// how a run's opacity reaches an emoji.
const GLYPH_SHADER_WGSL: &str = r#"
const PAGE_COLOR: u32 = 0u;

struct Params {
    surface: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var color_page: texture_2d<f32>;
@group(0) @binding(1) var mask_page: texture_2d<f32>;
@group(0) @binding(2) var page_sampler: sampler;
@group(1) @binding(0) var<uniform> params: Params;

struct GlyphIn {
    @builtin(vertex_index) corner: u32,
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) page: f32,
};

struct GlyphOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) page: u32,
};

@vertex
fn vs_main(in: GlyphIn) -> GlyphOut {
    let corner = vec2<f32>(f32(in.corner & 1u), f32((in.corner >> 1u) & 1u));
    let page = u32(round(in.page));
    var side: vec2<u32>;
    if (page == PAGE_COLOR) {
        side = textureDimensions(color_page);
    } else {
        side = textureDimensions(mask_page);
    }
    let pixel = in.pos + in.size * corner;
    var out: GlyphOut;
    out.position = vec4<f32>(
        pixel.x / params.surface.x * 2.0 - 1.0,
        1.0 - pixel.y / params.surface.y * 2.0,
        0.0,
        1.0,
    );
    out.uv = (in.uv + in.size * corner) / vec2<f32>(side);
    out.color = in.color;
    out.page = page;
    return out;
}

@fragment
fn fs_main(in: GlyphOut) -> @location(0) vec4<f32> {
    if (in.page == PAGE_COLOR) {
        let texel = textureSampleLevel(color_page, page_sampler, in.uv, 0.0);
        return vec4<f32>(texel.rgb, texel.a * in.color.a);
    }
    let coverage = textureSampleLevel(mask_page, page_sampler, in.uv, 0.0).r;
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
"#;

/// One glyph as the pipeline reads it. Every field is `f32`
/// ([`GLYPH_SHADER_WGSL`] says why); `page` is `0.0` for the color
/// page and `1.0` for the mask page.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub(crate) struct GlyphInstance {
    /// The bitmap's top-left, in surface pixels.
    pub(crate) pos: [f32; 2],
    /// The bitmap's size, in surface pixels and texels alike.
    pub(crate) size: [f32; 2],
    /// The bitmap's top-left texel on its page.
    pub(crate) uv: [f32; 2],
    /// Linear RGB and alpha: the glyph's sRGB color decoded, the run's
    /// opacity folded into the alpha.
    pub(crate) color: [f32; 4],
    /// The page: `0.0` color, `1.0` mask.
    pub(crate) page: f32,
}

/// The instance attributes, slot for slot with `GlyphIn`'s locations.
const GLYPH_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x2,
    2 => Float32x2,
    3 => Float32x4,
    4 => Float32,
];

/// The instance buffer's starting capacity, in instances. It doubles
/// on overflow and never shrinks.
const INITIAL_INSTANCE_CAPACITY: usize = 1024;

/// The uniform the vertex stage maps surface pixels to clip space
/// through: the surface size, padded to the 16 bytes a uniform binding
/// is sized in.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
struct Params {
    surface: [f32; 2],
    _pad: [f32; 2],
}

/// What the last prepared frame held, and running totals: the
/// structural facts the GPU-backed tests assert (TEST_CONVENTIONS §T8;
/// never pixels, never timings).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GlyphRunCounts {
    /// Frames prepared since the pass was made.
    pub(crate) prepares: u64,
    /// Runs the last frame was handed.
    pub(crate) runs: usize,
    /// Of those, runs inside their zoom window, on screen and not
    /// transparent.
    pub(crate) runs_drawn: usize,
    /// Glyph instances the last frame draws.
    pub(crate) instances: usize,
    /// Glyphs the last frame skipped: off screen, hostile values, or
    /// nothing to draw.
    pub(crate) skipped: usize,
    /// Draw calls the last frame issues for runs: `1` when there is
    /// any glyph, `0` otherwise — never one per run or per glyph.
    pub(crate) draws: u32,
    /// The atlas's running totals.
    pub(crate) atlas: AtlasCounts,
    /// The keys the atlas holds after the last frame.
    pub(crate) atlas_entries: usize,
}

/// The positioned-glyph pass: its atlas, its pipeline, and the
/// instance buffer one frame's runs are flattened into.
pub(crate) struct GlyphRunPass {
    atlas: GlyphAtlas,
    pipeline: wgpu::RenderPipeline,
    params: Params,
    params_buffer: wgpu::Buffer,
    params_bind_group: wgpu::BindGroup,
    instances: Vec<GlyphInstance>,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    counts: GlyphRunCounts,
}

impl GlyphRunPass {
    /// The pass for frames of `format` on `device`: an empty atlas, the
    /// pipeline, and the buffers at their starting sizes.
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let atlas = GlyphAtlas::new(device);
        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glyph_runs params layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64),
                },
                count: None,
            }],
        });
        let params = Params {
            surface: [1.0, 1.0],
            _pad: [0.0, 0.0],
        };
        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glyph_runs params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glyph_runs params bind group"),
            layout: &params_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glyph_runs shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(GLYPH_SHADER_WGSL)),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glyph_runs pipeline layout"),
            bind_group_layouts: &[Some(atlas.layout()), Some(&params_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glyph_runs pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GlyphInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &GLYPH_INSTANCE_ATTRIBUTES,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let instance_buffer = Self::instance_buffer(device, INITIAL_INSTANCE_CAPACITY);
        GlyphRunPass {
            atlas,
            pipeline,
            params,
            params_buffer,
            params_bind_group,
            instances: Vec::with_capacity(INITIAL_INSTANCE_CAPACITY),
            instance_buffer,
            instance_capacity: INITIAL_INSTANCE_CAPACITY,
            counts: GlyphRunCounts::default(),
        }
    }

    fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glyph_runs instances"),
            size: (capacity * std::mem::size_of::<GlyphInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Stop the atlas growing past its pages' present size
    /// ([`GlyphAtlas::freeze_growth`]).
    #[cfg(test)]
    pub(crate) fn freeze_atlas_growth(&mut self) {
        self.atlas.freeze_growth();
    }

    /// What the last prepared frame held, and the running totals.
    #[cfg(test)]
    pub(crate) fn counts(&self) -> GlyphRunCounts {
        self.counts
    }

    /// The CPU half of a frame: flatten every glyph of `runs` — each
    /// with its tree's registered offset — that `camera` shows into one
    /// instance list, rasterizing glyphs the atlas has not seen, and
    /// upload it.
    ///
    /// `font_system` is the guard the frame already holds for glyphon's
    /// prepare; nothing here takes the font lock or shapes text.
    ///
    /// Cost: O(glyphs of the runs handed in) — one placement read, one
    /// [`glyphon::CacheKey`] and one atlas lookup per glyph — plus one
    /// rasterization and texture write per glyph the atlas has not
    /// seen. One upload of the instance list. A run outside its zoom
    /// window or off screen costs one bounds test.
    pub(crate) fn prepare<'r>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_system: &mut FontSystem,
        swash: &mut SwashCache,
        camera: &Camera2D,
        runs: impl Iterator<Item = (&'r GlyphRunData, Vec2)>,
    ) {
        self.atlas.begin_frame();
        self.instances.clear();
        let mut frame = GlyphRunCounts {
            prepares: self.counts.prepares + 1,
            ..GlyphRunCounts::default()
        };
        // Scratch for each path run's laid-out glyphs, kept across the
        // frame's runs so it grows once rather than once per run.
        let mut layout: Vec<&'r LayoutGlyph> = Vec::new();
        let atlas = &mut self.atlas;
        let instances = &mut self.instances;
        for (run, offset) in runs {
            frame.runs += 1;
            let mut nothing_to_draw = 0;
            let emitted = emit_run(
                run,
                offset,
                camera,
                &mut layout,
                &mut |sprite: GlyphSprite| match atlas.slot(device, queue, font_system, swash, sprite.key) {
                    Some(slot) => instances.push(instance(&sprite, slot)),
                    None => nothing_to_draw += 1,
                },
            );
            if emitted.drawn {
                frame.runs_drawn += 1;
            }
            frame.skipped += emitted.skipped + nothing_to_draw;
        }

        let surface = [camera.viewport_size.0 as f32, camera.viewport_size.1 as f32];
        if self.params.surface != surface {
            self.params.surface = surface;
            queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&self.params));
        }
        if self.instances.len() > self.instance_capacity {
            self.instance_capacity = self.instances.len().next_power_of_two();
            self.instance_buffer = Self::instance_buffer(device, self.instance_capacity);
        }
        if !self.instances.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&self.instances));
        }
        frame.instances = self.instances.len();
        frame.draws = u32::from(!self.instances.is_empty());
        frame.atlas = self.atlas.counts();
        frame.atlas_entries = self.atlas.entry_count();
        self.counts = frame;
    }

    /// The GPU half: draw what [`Self::prepare`] uploaded, as one
    /// instanced draw, into `pass`. Nothing when it held no glyph.
    pub(crate) fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.instances.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, self.atlas.bind_group(), &[]);
        pass.set_bind_group(1, &self.params_bind_group, &[]);
        let bytes = (self.instances.len() * std::mem::size_of::<GlyphInstance>()) as u64;
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..bytes));
        pass.draw(0..4, 0..self.instances.len() as u32);
    }
}

/// The instance that draws `sprite` from `slot`: the bitmap's rectangle
/// placed by its bearing from the pen, and the color decoded to linear.
pub(crate) fn instance(sprite: &GlyphSprite, slot: atlas::Slot) -> GlyphInstance {
    let [r, g, b, a] = sprite.color;
    GlyphInstance {
        pos: [
            (sprite.pen.0 + slot.left) as f32,
            (sprite.pen.1 - slot.top) as f32,
        ],
        size: [slot.size.0 as f32, slot.size.1 as f32],
        uv: [slot.uv.0 as f32, slot.uv.1 as f32],
        color: [
            srgb_to_linear(r),
            srgb_to_linear(g),
            srgb_to_linear(b),
            f32::from(a) / 255.0,
        ],
        page: match slot.page {
            Page::Color => 0.0,
            Page::Mask => 1.0,
        },
    }
}

/// One sRGB-encoded channel as linear light, the decode glyphon's
/// shader applies to a mask glyph's color for an sRGB target.
fn srgb_to_linear(channel: u8) -> f32 {
    let c = f32::from(channel) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
