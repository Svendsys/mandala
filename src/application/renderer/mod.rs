// SPDX-License-Identifier: MPL-2.0

//! GPU-side presentation: every wgpu device, every cosmic-text
//! rasterization, every text buffer Mandala paints lives under
//! [`Renderer`]. A frame is its clear — the map's one background — and
//! glyphs, and nothing else (V1-B12). `Renderer` reads the intermediate
//! representations it is handed — the projection's glyph layers and the
//! chrome layer, with the journals of what changed in them, and the
//! overlay trees of the screen-space modals;
//! it never reaches into the document directly, and the document never
//! holds GPU resources (CODE_CONVENTIONS §3 "Model / view
//! separation").
//!
//! The submodule split corresponds to wgpu pipeline boundaries:
//!
//! - [`adapter`] — which GPU adapter a bootstrap runs on, and the
//!   order it tries them in.
//! - [`pipeline`] — the window's surface configuration.
//! - [`render`] — one frame as a prepare / encode pair
//!   parameterized over camera and target, and the window's
//!   per-frame driver over it (`Renderer::process`); composes the
//!   buffer layers in draw order.
//! - [`tree_walker`] — `GfxElement` tree → text buffers: how the
//!   console's and the color picker's overlay trees are shaped.
//! - [`glyph_runs`] — the positioned-glyph pass: every glyph of a
//!   `GlyphRun` at its own canvas position, from the run's one
//!   shaped buffer, through an atlas and a pipeline of its own, in
//!   one draw call per frame (design §13.2).
//! - [`layers`] — the layer renderer: the projection's glyph layers
//!   kept per glyph element, written only where a journal says they
//!   changed, and handed to the positioned-glyph pass every frame in
//!   their draw order (design §13.1).
//! - [`target_encoding`] — how an authored color is written to a
//!   target of a given format: the one answer the clear, glyphon's
//!   text and the glyph-run pass share, so a glyph colored like the
//!   background matches it on every target.
//! - [`backdrop`] — the opaque backdrops behind the console and the
//!   color picker, drawn as a grid of full-block glyphs (`█`) in the
//!   console's pass, under the modal's text.
//! - [`selection_overlay`] — the rubber-band selection
//!   rectangle, the one canvas visual with no model behind it.
//! - [`chrome_text`] — the screen-space chrome's text buffers and
//!   the glyph-advance measurement modal frames are laid out with.
//! - [`console_pass`] / [`console_geometry`] — the console
//!   overlay's glyph-tree pass + pure-function layout math.
//! - [`color_picker`] — the glyph-wheel picker overlay.
//! - [`camera`] — camera framing and the screen-space ↔
//!   canvas-space mapping (`screen_to_canvas`,
//!   `fit_camera_to_content`).
//! - `capture` (native-only) — the offscreen frame a screenshot
//!   draws through the same prepare / encode pair as the window,
//!   and its readback.
//! - [`decree`] — the `RenderDecree` queue the event loop
//!   feeds the renderer (resize, zoom, camera-pan, etc.).
//! - [`overlay_dispatch`] — overlay-vs-canvas slot routing
//!   for the [`crate::application::scene_host::AppScene`]
//!   tree handles.

// Adapter choice — the attempt plan and the environment it honors —
// shared by every bootstrap.
mod adapter;
// The console's and the color picker's backdrops, as full-block glyphs.
mod backdrop;
// The overlay-render modules below carry a wasm32-only
// `allow(dead_code)`: each renders a modal whose *shell* is
// native-gated (CLAUDE.md "Dual-target status"), so on wasm32 the
// builders compile with no caller. Scoped to wasm32 so the host
// lint, which can see the shell, stays armed.
mod camera;
mod chrome_text;
// Offscreen capture and readback. Native-only at the module boundary:
// a browser cannot block on a buffer map (CLAUDE.md "Dual-target
// status", "Screenshot capture").
#[cfg(not(target_arch = "wasm32"))]
mod capture;
// Color-picker overlay tree + mutators. Shell: the native-gated
// `app::color_picker_flow`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod color_picker;
// Console overlay geometry. Shell: the native-gated
// `app::console_input`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod console_geometry;
// Console overlay glyph areas + mutators. Same shell as
// `console_geometry`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod console_pass;
mod decree;
mod glyph_runs;
mod layers;
// Renderer entry points for the two overlay rebuilds. Same shells
// as `console_pass` and `color_picker`.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod overlay_dispatch;
mod pipeline;
mod render;
// How an authored color is written to a target of a given format: the
// one answer the clear, glyphon and the glyph-run pass share.
mod target_encoding;
// Rubber-band selection rectangle. Driven by the native-gated
// drag state machine.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod selection_overlay;
mod tree_walker;

#[cfg(test)]
pub(crate) use adapter::probe_adapter;
pub use adapter::AdapterPolicy;
pub use chrome_text::measure_max_glyph_advance;
// Both targets': the scene inspection measures an element by it
// (`view::inspect`).
pub(crate) use camera::content_bounds;
// The screenshot engine's half of the renderer's surface, native-only
// with the engine (`application::capture`).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use camera::FIT_PADDING_FRACTION;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use capture::{CaptureFailure, CapturedPixels};
// For the watchdog's test that holds the two bounds together.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) use capture::GPU_WAIT as CAPTURE_GPU_WAIT;
// Both shells read a skipped frame's reason (`Renderer::process`).
pub(crate) use render::FrameSkip;
// The view's pipeline hands the layer renderer the projector's journals
// through it (`view::MapView::sync`).
pub(crate) use layers::LayerRenderer;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use render::ModalLayer;
// The console overlay's geometry vocabulary, re-exported for the
// app-side console shell (`app::console_input`) that fills it in.
// That shell is native-gated, so on wasm32 nothing consumes these —
// the same reason the overlay modules above carry a wasm32-only
// `allow`, scoped the same way so the host lint stays armed.
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
pub use console_geometry::{
    ConsoleOverlayCompletion, ConsoleOverlayGeometry, ConsoleOverlayLine, ConsoleOverlayLineKind,
    MAX_CONSOLE_SCROLLBACK_ROWS,
};
// Layout math exercised only by this module's test block; the
// production callers inside `renderer/` reach it through
// `super::console_geometry::` directly.
#[cfg(test)]
use console_geometry::{
    build_console_border_strings, compute_console_frame_layout, MAX_CONSOLE_COMPLETION_ROWS,
};
#[cfg(test)]
use console_pass::{
    build_console_overlay_mutator, build_console_overlay_tree, completion_row_regions, console_overlay_areas,
    console_overlay_signature, CompletionRowSpans, CONSOLE_CHANNEL_BOTTOM_BORDER,
    CONSOLE_CHANNEL_COMPLETION_BASE, CONSOLE_CHANNEL_LEFT_COL, CONSOLE_CHANNEL_PROMPT,
    CONSOLE_CHANNEL_RIGHT_COL, CONSOLE_CHANNEL_SCROLLBACK_GUTTER_BASE, CONSOLE_CHANNEL_SCROLLBACK_TEXT_BASE,
    CONSOLE_CHANNEL_TOP_BORDER,
};

use std::sync::Arc;
use std::time::Duration;
use web_time::Instant;

use baumhard::engine::journal::Journal;
use baumhard::font::{Attrs, Buffer};
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::tree::GfxTree;
use baumhard::projection::{GlyphLayers, Layer};
use glyphon::{Cache, SwashCache, TextAtlas, TextRenderer, Viewport};
use log::{error, info, warn};
// Also read by the GPU-backed test that checks what a renderer handed
// its passes.
pub(crate) use target_encoding::TargetEncoding;

use wgpu::{Color, Device, Instance, MultisampleState, Queue, Surface, SurfaceConfiguration, TextureFormat};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::application::common::{FpsDisplayMode, RenderDecree};
use baumhard::font::fonts;
#[cfg(test)]
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::camera::Camera2D;
use glam::Vec2;

/// How many frames `FpsDisplayMode::Snapshot` waits between readout
/// refreshes, and how many frames `FpsDisplayMode::Debug` averages
/// over. 200 at 60 fps ≈ 3.3 s — short enough to react to sustained
/// perf changes, long enough to smooth out per-frame jitter.
const FPS_WINDOW: usize = 200;

/// Mode-status overlay layout: font size, screen-space position
/// (top-left corner, in pixels), and shaping bounds (max width ×
/// height in pixels). The position sits below the FPS overlay's
/// row (`(8.0, 8.0)` + ~24 px line height). Width is generous so
/// long node ids + section counts fit on one line; height matches
/// the FPS overlay so vertical alignment is predictable when both
/// are visible. Promoted to constants so future overlays can
/// stack predictably below the mode-status row.
const MODE_STATUS_FONT_SIZE_PT: f32 = 14.0;
const MODE_STATUS_OVERLAY_POS: (f32, f32) = (8.0, 32.0);
const MODE_STATUS_OVERLAY_BOUNDS: (f32, f32) = (640.0, 24.0);

/// Mode-status text color. The status row and the selection tint
/// are the same "this is active" affordance, so they are the same
/// color — derived from the one definition of it rather than
/// re-quantized here, where a hand-written
/// `Color::rgba(0, 230, 255, 255)` would disagree with `#00E5FF`, whose
/// green channel is 229.
pub(crate) const MODE_STATUS_COLOR: baumhard::font::Color =
    baumhard::font::color::cosmic_color_from_color(baumhard::util::color::Color {
        rgba: crate::application::view::overlay::SELECTION_HIGHLIGHT.0,
    });

/// Fixed-size ring buffer of frame intervals (microseconds) with an
/// O(1) running sum. Backs `FpsDisplayMode::Debug`'s rolling-average
/// readout. Encapsulates the sum invariant — `sum` is always
/// consistent with `samples[..filled.min(FPS_WINDOW)]` — so the
/// four-field state can never drift out of sync via direct access.
/// Private to this module.
pub(super) struct FrameIntervalRing {
    samples: [u128; FPS_WINDOW],
    idx: usize,
    sum: u128,
    filled: usize,
}

impl FrameIntervalRing {
    pub(super) fn new() -> Self {
        Self {
            samples: [0u128; FPS_WINDOW],
            idx: 0,
            sum: 0,
            filled: 0,
        }
    }

    pub(super) fn clear(&mut self) {
        self.samples = [0u128; FPS_WINDOW];
        self.idx = 0;
        self.sum = 0;
        self.filled = 0;
    }

    pub(super) fn push(&mut self, micros: u128) {
        let old = self.samples[self.idx];
        self.sum = self.sum - old + micros;
        self.samples[self.idx] = micros;
        self.idx = (self.idx + 1) % FPS_WINDOW;
        if self.filled < FPS_WINDOW {
            self.filled += 1;
        }
    }

    pub(super) fn avg_micros(&self) -> Option<u128> {
        if self.filled == 0 {
            None
        } else {
            Some(self.sum / self.filled as u128)
        }
    }
}

pub struct Renderer {
    /// Where the window's frames land: a window's swapchain, or — with
    /// no window at all — nowhere ([`Sink`]).
    sink: Sink,
    /// The color format of every frame: the window's surface format, or
    /// the headless target's (`HEADLESS_FORMAT`). Every pipeline is
    /// built for it, and a capture's offscreen texture is created in it,
    /// so it has this one source.
    // Native-only reader: the capture engine, which is native-gated.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    format: TextureFormat,
    /// The size, in pixels, of what the window's frames are drawn for:
    /// the window's surface, or the headless target. Every screen-space
    /// layout and the live camera's viewport follow it.
    surface_size: (u32, u32),
    device: Device,
    queue: Queue,
    viewport: Viewport,
    swash_cache: SwashCache,
    /// glyphon's shader / bind-group cache. Held, not read: it is
    /// borrowed once each by `TextAtlas::new` and `Viewport::new` at
    /// construction, and both keep their own handle to it. Dropping
    /// the field would end the renderer's share of a resource its
    /// two consumers still use.
    #[allow(dead_code)]
    glyphon_cache: Cache,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    /// Second glyphon TextRenderer, for the screen-space chrome: the
    /// console or the color picker over its backdrop, the FPS readout
    /// and the mode-status line. Shares `self.atlas` with
    /// `text_renderer` so glyph caching is unified, but keeps its own
    /// vertex/index buffers — which is what lets the glyph-run pass
    /// draw BETWEEN the two text renders inside one render pass, so the
    /// chrome covers every canvas glyph (re-preparing the single text
    /// renderer would race with the pass's already-recorded draw
    /// commands).
    console_text_renderer: TextRenderer,
    /// The positioned-glyph pass: draws the `GlyphRun` elements the
    /// canvas trees hold, each glyph at its own position, between the
    /// main text pass and the chrome pass. Shares the font system,
    /// the swash cache and the render pass with glyphon; holds its own
    /// atlas, because glyphon's cannot be reached from outside it.
    glyph_runs: glyph_runs::GlyphRunPass,
    /// The render state of the projection's glyph layers (design §13.1):
    /// kept per glyph element, written by [`Self::load_view`] and
    /// [`Self::apply_layer_journals`] only, and drawn every frame through
    /// `glyph_runs`, after the canvas trees' runs.
    layer_renderer: layers::LayerRenderer,
    /// Whether the current glyphon-`prepare` fault episode has already
    /// been reported. Set when either `prepare()` fails, cleared on
    /// the next fully successful prepare, so a *persistent* fault —
    /// `PrepareError::AtlasFull` is permanent once the atlas hits
    /// `max_texture_dimension_2d` — logs one line instead of one per
    /// frame. A failed prepare skips `present()` too, so the frame
    /// takes no vsync backpressure and the loop can spin at CPU rate.
    prepare_fault_logged: bool,
    should_render: bool,
    fps: Option<usize>,
    /// Which FPS readout to display, if any. `Snapshot` samples one
    /// frame's interval every `FPS_WINDOW` frames; `Debug` averages the
    /// last `FPS_WINDOW` frame intervals and updates every frame.
    /// Toggled via `fps on` / `fps debug` / `fps off`.
    fps_display_mode: FpsDisplayMode,
    /// Screen-space text buffer(s) carrying the yellow FPS readout.
    /// Chained into `palette_text_areas` at render time so the readout
    /// draws at `scale: 1.0` with no camera transform. Empty whenever
    /// `fps_display_mode` is `Off`.
    fps_overlay_buffers: Vec<MindMapTextBuffer>,
    /// The `self.fps` value that was shaped into `fps_overlay_buffers`
    /// last. Used to skip re-shaping when the integer value hasn't
    /// changed since the last rebuild.
    last_fps_shaped: Option<usize>,
    /// Pending status-line text, set by the view's sync with the
    /// document's status (D28) and consumed by
    /// [`Self::rebuild_mode_status_overlay_if_needed`] at the next
    /// frame. `None` clears the line; `Some(text)` shows it. The string
    /// is the document's, so the renderer stays model-agnostic.
    mode_status_text: Option<String>,
    /// Screen-space text buffer(s) carrying the mode-status line
    /// (e.g. `editing: <node-id> — section [N of M]`). Sibling of
    /// `fps_overlay_buffers`; same render path. Empty when
    /// `mode_status_text` is `None`.
    mode_status_overlay_buffers: Vec<MindMapTextBuffer>,
    /// The `self.mode_status_text` value that was shaped into
    /// `mode_status_overlay_buffers` last. Used to skip re-shaping
    /// when the text hasn't changed.
    last_mode_status_shaped: Option<String>,
    /// Wall-clock timestamp of the previous rendered frame. The
    /// difference between consecutive values is the actual frame
    /// interval, which is what FPS is derived from. Measuring
    /// wall-clock here rather than the duration of the `render()`
    /// call is load-bearing: `render()` can early-return on
    /// font-system lock contention under heavy interaction, and
    /// timing the call would then shrink to near-zero and inflate FPS
    /// to a false huge value.
    last_frame_instant: Option<Instant>,
    /// Frame counter used by `FpsDisplayMode::Snapshot` to refresh the
    /// displayed value only every `FPS_WINDOW` frames. Increments
    /// every frame regardless of mode; meaningful only in Snapshot.
    fps_clock: usize,
    /// Rolling window of the last `FPS_WINDOW` frame intervals,
    /// consumed by `FpsDisplayMode::Debug` to compute a rolling
    /// average. The sum / divisor invariant is enforced by the
    /// `FrameIntervalRing` wrapper — no direct field access here.
    fps_ring: FrameIntervalRing,
    /// Set by [`Self::set_fps_idle`] when the event loop transitions
    /// from active rendering to `ControlFlow::Wait`. Consumed by
    /// the next [`Self::tick_fps`] call, which short-circuits to
    /// `fps = None` so the transitional render paints "FPS: -"
    /// regardless of what the rolling average or snapshot
    /// alignment would otherwise compute.
    fps_pending_idle_paint: bool,

    camera: Camera2D,
    /// Screen-space geometry of the color picker's opaque backdrop, in
    /// window pixels. Captured inside
    /// `rebuild_color_picker_overlay_buffers`; each frame draws it as
    /// full-block cells alongside the console's ([`backdrop`]). `None`
    /// whenever the picker is closed, and when its spec asks for a
    /// transparent backdrop.
    color_picker_backdrop: Option<(f32, f32, f32, f32)>,
    /// The rubber-band selection rectangle's four shaped edges, and
    /// nothing else. Camera-transformed. Written only by
    /// [`Self::rebuild_selection_rect_overlay`] and dropped only by
    /// [`Self::clear_selection_rect_overlay`]; the color picker, the FPS
    /// readout and the mode-status line each have their own store.
    ///
    /// The exclusivity is load-bearing rather than incidental:
    /// `app::drain_frame::drain_rect_select` — a plain code span
    /// because `app` is a private module and rustdoc cannot resolve
    /// a link into one — clears this on every frame no rubber band
    /// is live, so anything else parked here would be wiped. The
    /// doc used to read "temporary overlay buffers (e.g., selection
    /// rectangle)", which invited exactly that.
    overlay_buffers: Vec<MindMapTextBuffer>,
    /// `(char_count, row_count)` of the most recent selection-rect
    /// shape held in [`Self::overlay_buffers`]. Per-tick rebuilds
    /// reuse the existing shaped buffers (just update positions)
    /// when these counts match, avoiding 4 fresh `cosmic_text`
    /// shapings per drag tick. `None` whenever the overlay is
    /// cleared or holds a non-selection-rect shape.
    selection_rect_shape_cache: Option<(usize, usize)>,
    /// The console's and the color picker's shaped buffers, in draw
    /// order: made from the overlay trees of the app's
    /// [`AppScene`](crate::application::scene_host::AppScene) by
    /// [`Self::rebuild_overlay_scene_buffers`], drawn in the chrome pass.
    overlay_scene_elements: Vec<MindMapTextBuffer>,
    /// Screen-space geometry of the console's opaque backdrop, in
    /// window pixels: `(left, top, width, height)`. Captured inside
    /// `rebuild_console_overlay_buffers` so each frame can lay its
    /// cells out in its own target's pixels ([`backdrop`]) without
    /// re-running the layout. `None` whenever the console is closed.
    console_backdrop: Option<(f32, f32, f32, f32)>,
    /// The one shaped full block every backdrop's cells draw
    /// ([`backdrop::BackdropCell`]). Shaped the first time a backdrop
    /// is set ([`Self::ensure_backdrop_cell`]), and kept: it depends on
    /// nothing a backdrop or a target varies.
    backdrop_cell: Option<backdrop::BackdropCell>,
    /// The fill blocks the last prepared frame handed each pass
    /// ([`backdrop::FillCensus`]). Test build only, read by the
    /// GPU-backed tests (`Renderer::fill_census_of_last_frame`).
    #[cfg(test)]
    fill_census: backdrop::FillCensus,
    /// The color mode glyphon's atlas was built with, kept by the test
    /// build so the GPU-backed test can read what `Renderer::new` handed
    /// it (glyphon keeps its own copy private).
    #[cfg(test)]
    glyphon_color_mode: glyphon::ColorMode,
    /// Clear color for the render pass: the map's resolved
    /// background, the one color drawn behind the whole canvas
    /// ([`Self::set_background`]). Starts as opaque black — the
    /// format's default background — so the app looks right before a
    /// map loads; the event loop sets it right after load.
    clear_color: Color,
}

/// Where the window's frames land.
enum Sink {
    /// A window's swapchain.
    Window {
        surface: Surface<'static>,
        config: SurfaceConfiguration,
        /// The window the surface was created from. The
        /// `Surface<'static>` lifetime is bought by handing wgpu an
        /// `Arc<Window>` clone at `create_surface`, and this is the
        /// renderer's own share of that ownership — dropping it would
        /// make the renderer's lifetime depend on whoever else still
        /// holds the window. The browser's loop asks it to redraw,
        /// through [`Renderer::request_redraw`]; native's shell owns
        /// its redraw requests (`app::shell`).
        #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
        window: Arc<Window>,
    },
    /// No window (`--headless`). A frame for it draws nothing: there is
    /// nothing to draw into and nobody to see it, and a capture prepares
    /// and draws its own (`renderer::capture`) into a texture that is
    /// the only sink a headless app has — so pixels are produced exactly
    /// when they are asked for.
    #[cfg(not(target_arch = "wasm32"))]
    Headless,
}

impl Sink {
    /// Whether a frame drawn for this sink is shown anywhere.
    fn presents(&self) -> bool {
        match self {
            Sink::Window { .. } => true,
            #[cfg(not(target_arch = "wasm32"))]
            Sink::Headless => false,
        }
    }

    /// The window's next swapchain texture, or `None` with no window.
    fn current_texture(&self) -> Option<wgpu::CurrentSurfaceTexture> {
        match self {
            Sink::Window { surface, .. } => Some(surface.get_current_texture()),
            #[cfg(not(target_arch = "wasm32"))]
            Sink::Headless => None,
        }
    }
}

/// The color format of a headless renderer's frames. There is no
/// surface to offer one, so it is chosen: sRGB, as the window's
/// bootstrap prefers among what its surface offers, in the
/// blue-green-red byte order the windowed path was observed to be
/// offered under Xvfb on Mesa's lavapipe — so a headless capture and a
/// windowed one on the same machine draw through pipelines of the same
/// format. Every adapter wgpu exposes can render to it.
#[cfg(not(target_arch = "wasm32"))]
const HEADLESS_FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;

/// The render-pass clear value that leaves `background` in a target of
/// `format`, opaque.
///
/// A clear value is written to the target the way a shader's output
/// is, so it takes the target's [`TargetEncoding`]: on an sRGB target
/// each channel is the authored byte decoded to linear light, which the
/// GPU encodes back as it writes; on a linear (`Unorm`) target — the
/// browser's WebGPU surface, which is never sRGB — the byte goes in as
/// it is, which that target keeps. Glyphon's text and the glyph-run
/// pass make the same choice from the same type, so the frame holds
/// `#rrggbb` as written behind glyphs that hold their own colors as
/// written, on native and on both browser backends alike
/// (CODE_CONVENTIONS §4). The alpha is always 1.
///
/// Pure, so the renderer-free tier pins it (§T8). O(1).
pub(super) fn clear_color_for(background: [u8; 4], format: TextureFormat) -> Color {
    let [r, g, b, _] = background;
    let encoding = TargetEncoding::of(format);
    Color {
        r: encoding.channel(r),
        g: encoding.channel(g),
        b: encoding.channel(b),
        a: 1.0,
    }
}

/// Clamp a requested surface (width, height) to the GPU's
/// `max_texture_dimension_2d`. Pure function so the clamp logic is
/// testable without a live GPU device.
///
/// # Why this exists
///
/// `surface.configure` on dimensions beyond the GPU's 2D texture
/// limit can leave the surface in a bad state on some wgpu
/// backends — subsequent `get_current_texture()` calls may then
/// block indefinitely rather than returning an error. Clamping
/// proactively trades a letterboxed frame for a non-hung UI. The
/// scenario is realistic on ultra-wide displays or multi-monitor-
/// maxed windows.
pub(crate) fn clamp_surface_size_to_gpu_limit(width: u32, height: u32, max_dim: u32) -> (u32, u32) {
    let clamped_width = if width > max_dim {
        warn!(
            "renderer: requested surface width {} exceeds GPU max_texture_dimension_2d {}; clamping",
            width, max_dim
        );
        max_dim
    } else {
        width
    };
    let clamped_height = if height > max_dim {
        warn!(
            "renderer: requested surface height {} exceeds GPU max_texture_dimension_2d {}; clamping",
            height, max_dim
        );
        max_dim
    } else {
        height
    };
    (clamped_width, clamped_height)
}

impl Renderer {
    /// Native bootstrap: own the `wgpu::Instance` + `Surface`
    /// construction so the caller doesn't need to import `wgpu`.
    /// Hand wgpu the owned `Arc<Window>` rather than pre-snapshotting
    /// raw handles via `SurfaceTargetUnsafe::from_window`: under
    /// wgpu 29 + winit 0.30 the latter blew up with
    /// `Hal(MissingDisplayHandle)` on EGL/GL Linux because the GL
    /// surface ctor re-queries the display handle and won't accept a
    /// captured raw struct.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn bootstrap_native(window: Arc<Window>, adapter: AdapterPolicy) -> Renderer {
        let instance = Instance::new(adapter::instance_descriptor());
        let surface = instance
            .create_surface(window.clone())
            .expect("failed to create wgpu surface for window");
        Self::for_window(&instance, surface, window, adapter).await
    }

    /// WASM bootstrap: same as `bootstrap_native` but binds the
    /// surface to the supplied `<canvas>` element. The browser's
    /// adapter/device init is Promise-backed so this stays async
    /// like the native form. The instance comes from
    /// [`adapter::browser_instance`], which falls back to WebGL when
    /// WebGPU offers no adapter, and it must come before the surface:
    /// creating a surface claims the canvas for one context type.
    #[cfg(target_arch = "wasm32")]
    pub async fn bootstrap_wasm(window: Arc<Window>, canvas: web_sys::HtmlCanvasElement) -> Renderer {
        let instance = adapter::browser_instance(adapter::instance_descriptor()).await;
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .expect("failed to create wgpu surface for canvas");
        Self::for_window(&instance, surface, window, AdapterPolicy::Auto).await
    }

    /// Headless bootstrap (`--headless`): no window, no surface — an
    /// adapter chosen with no surface to be compatible with, and frames
    /// `size` pixels large for a [`Sink::Headless`] that presents
    /// nothing. Everything else — device, pipelines, atlas — is what a
    /// window gets.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn bootstrap_headless(size: PhysicalSize<u32>, adapter: AdapterPolicy) -> Renderer {
        let instance = Instance::new(adapter::instance_descriptor());
        let (_adapter, device, queue) = adapter::acquire(&instance, None, adapter)
            .await
            .unwrap_or_else(|why| panic!("renderer: no GPU adapter could be used without a window: {why}"));
        Self::new(device, queue, Sink::Headless, HEADLESS_FORMAT, size)
    }

    /// The window half of both windowed bootstraps: an adapter that can
    /// present to `surface`, and the surface configured in the sRGB
    /// format it offers (its first format when it offers none).
    async fn for_window(
        instance: &Instance,
        surface: Surface<'static>,
        window: Arc<Window>,
        adapter: AdapterPolicy,
    ) -> Renderer {
        let (adapter, device, queue) = adapter::acquire(instance, Some(&surface), adapter)
            .await
            .unwrap_or_else(|why| panic!("renderer: no GPU adapter could present to the window: {why}"));
        let surface_capabilities = surface.get_capabilities(&adapter);
        let surface_format = surface_capabilities
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_capabilities.formats[0]);
        let size = window.inner_size();
        let config = Self::create_surface_config(
            surface_format,
            &surface_capabilities,
            PhysicalSize::new(size.width, size.height),
        );
        let sink = Sink::Window {
            surface,
            config,
            window,
        };
        Self::new(device, queue, sink, surface_format, size)
    }

    /// Everything a renderer holds beyond its sink: the glyphon atlas
    /// and text renderers and the glyph-run pass — all in `format` —
    /// and a camera over `size`.
    fn new(
        device: Device,
        queue: Queue,
        sink: Sink,
        surface_format: TextureFormat,
        size: PhysicalSize<u32>,
    ) -> Renderer {
        let glyphon_cache = Cache::new(&device);

        // How every pass writes a color to this target, derived once and
        // handed to both glyph passes: glyphon decodes a glyph's color for
        // an sRGB target and writes it as it is for a linear one, as the
        // clear and the glyph-run pass do (`target_encoding`).
        let encoding = TargetEncoding::of(surface_format);
        let glyphon_color_mode = encoding.glyphon_color_mode();
        let mut atlas = TextAtlas::with_color_mode(
            &device,
            &queue,
            &glyphon_cache,
            surface_format,
            glyphon_color_mode,
        );
        let text_renderer = TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        let console_text_renderer = TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        let viewport = Viewport::new(&device, &glyphon_cache);
        let glyph_runs = glyph_runs::GlyphRunPass::new(&device, surface_format, encoding);
        let camera = Camera2D::new(size.width, size.height);

        Renderer {
            sink,
            format: surface_format,
            surface_size: (size.width, size.height),
            device,
            queue,
            atlas,
            swash_cache: SwashCache::new(),
            text_renderer,
            console_text_renderer,
            glyph_runs,
            layer_renderer: layers::LayerRenderer::default(),
            prepare_fault_logged: false,
            should_render: false,
            fps: None,
            fps_display_mode: FpsDisplayMode::Off,
            fps_overlay_buffers: Vec::new(),
            last_fps_shaped: None,
            mode_status_text: None,
            mode_status_overlay_buffers: Vec::new(),
            last_mode_status_shaped: None,
            last_frame_instant: None,
            fps_clock: 0,
            fps_ring: FrameIntervalRing::new(),
            fps_pending_idle_paint: false,
            glyphon_cache,
            viewport,
            camera,
            color_picker_backdrop: None,
            overlay_buffers: Vec::new(),
            selection_rect_shape_cache: None,
            overlay_scene_elements: Vec::new(),
            console_backdrop: None,
            backdrop_cell: None,
            #[cfg(test)]
            fill_census: backdrop::FillCensus::default(),
            #[cfg(test)]
            glyphon_color_mode,
            clear_color: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        }
    }

    /// How many glyphon text buffers the renderer holds: the overlays'
    /// (FPS, mode status, rubber band, console and picker). The glyph
    /// layers add none — what the GPU-backed tests hold a run to.
    #[cfg(test)]
    pub(crate) fn glyphon_buffer_count(&self) -> usize {
        self.fps_overlay_buffers.len()
            + self.mode_status_overlay_buffers.len()
            + self.overlay_buffers.len()
            + self.overlay_scene_elements.len()
    }

    /// What the positioned-glyph pass held at the last prepared frame,
    /// and its running totals — for the GPU-backed tests, which assert
    /// on structure (runs, instances, draw calls, atlas insertions),
    /// never on pixels.
    #[cfg(test)]
    pub(crate) fn glyph_run_counts(&self) -> glyph_runs::GlyphRunCounts {
        self.glyph_runs.counts()
    }

    /// Stop the positioned-glyph atlas growing past its present size,
    /// so a GPU-backed test can fill a page.
    #[cfg(test)]
    pub(crate) fn freeze_glyph_atlas_growth(&mut self) {
        self.glyph_runs.freeze_atlas_growth();
    }

    /// The render state of the glyph layers, for the GPU-backed tests
    /// that hold what a frame drew to what was applied.
    #[cfg(test)]
    pub(crate) fn layer_renderer(&self) -> &layers::LayerRenderer {
        &self.layer_renderer
    }

    /// Current camera zoom level.
    pub fn camera_zoom(&self) -> f32 {
        self.camera.zoom
    }

    /// The camera, read-only: the `surface` ↔ `canvas` transform the
    /// renderer and every hit test derive from. Added for the IPC
    /// `query.camera` command (and shared with `scene.*`); nothing
    /// outside the renderer can move the camera through it.
    // Native-driver-only: the IPC `query.camera` reader; the browser
    // serves no socket (CLAUDE.md "Dual-target status").
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn camera(&self) -> &Camera2D {
        &self.camera
    }

    /// The text of the cyan mode-status line, or `None` when none is
    /// showing. Read-only: `set_mode_status_text` is the writer, called
    /// by the view's sync with the document's status, and by native's
    /// console when it switches the mode; `query.status` reads it on both
    /// targets.
    pub fn mode_status_text(&self) -> Option<&str> {
        self.mode_status_text.as_deref()
    }

    /// Width in pixels of what the window's frames are drawn for: the
    /// swapchain surface, or the headless target.
    pub fn surface_width(&self) -> u32 {
        self.surface_size.0
    }

    /// Height in pixels of what the window's frames are drawn for: the
    /// swapchain surface, or the headless target.
    pub fn surface_height(&self) -> u32 {
        self.surface_size.1
    }

    /// Clear every frame to `background`, the map's one background
    /// color (V1-B12), so the frame shows exactly that color behind
    /// everything the map draws. Called on both targets when a map
    /// loads and after every rebuild of the canvas, so the clear
    /// follows the document's background through a theme-variant
    /// switch, an undo or a document swap.
    ///
    /// Opaque whatever `background` says: nothing is behind the
    /// canvas for an alpha to blend with. See [`clear_color_for`] for
    /// how the color is written to a target of this renderer's format.
    pub fn set_background(&mut self, background: [u8; 4]) {
        self.clear_color = clear_color_for(background, self.format);
    }

    /// Take the projection's glyph layers and the chrome layer as a
    /// whole: what installing a document does
    /// ([`layers::LayerRenderer::load`], `view::MapView::load`). Every
    /// frame after draws them, on both targets, the window's and every
    /// capture's.
    ///
    /// Cost: one walk of each layer.
    pub(crate) fn load_view(&mut self, layers: &GlyphLayers, chrome: &GfxTree) {
        self.layer_renderer.load(layers);
        self.layer_renderer.load_chrome(chrome);
    }

    /// Bring the drawn layers up to date after a projector update, from
    /// the journals of what changed — the projector's, and those of the
    /// glyph mutators applied over them, in order
    /// ([`layers::LayerRenderer::apply`]). A pan or a zoom has nothing to
    /// apply: the camera reaches the layers at draw time.
    ///
    /// Cost: O(the elements the journals name).
    pub(crate) fn apply_layer_journals<'j>(
        &mut self,
        layers: &GlyphLayers,
        journals: impl IntoIterator<Item = (Layer, &'j Journal<GfxElement>)>,
    ) {
        self.layer_renderer.apply(layers, journals);
    }

    /// The color every frame is cleared to, as the render pass is
    /// given it. For the GPU-backed tests (TEST_CONVENTIONS §T8),
    /// which assert on state, never on pixels.
    #[cfg(test)]
    pub(crate) fn clear_color(&self) -> Color {
        self.clear_color
    }

    /// The fill blocks the last prepared frame handed the canvas
    /// passes and the chrome pass ([`backdrop::FillCensus`]): a map's
    /// frame hands the canvas none, and an open console's or picker's
    /// backdrop hands the chrome one per cell. For the GPU-backed tests,
    /// which assert on what a frame was given to draw, never on pixels.
    #[cfg(test)]
    pub(crate) fn fill_census_of_last_frame(&self) -> backdrop::FillCensus {
        self.fill_census
    }

    /// What `Renderer::new` handed the two glyph passes for this
    /// renderer's target: glyphon's color mode, and the glyph-run pass's
    /// encoding with the format of its color page. For the GPU-backed
    /// test that builds a renderer on a linear target.
    #[cfg(test)]
    pub(crate) fn glyph_pass_encodings(&self) -> (glyphon::ColorMode, (TargetEncoding, TextureFormat)) {
        (self.glyphon_color_mode, self.glyph_runs.encoding())
    }

    /// A headless renderer like `bootstrap_headless`'s, but drawing in
    /// `format`, so a GPU-backed test can stand on a target the app's
    /// own headless launch never picks — a linear one, the browser's
    /// WebGPU case. The caller has probed for an adapter first
    /// (`app::gpu_harness`), so one that then opens no device dies saying
    /// so, as the headless bootstrap does.
    #[cfg(test)]
    pub(crate) async fn headless_in(size: PhysicalSize<u32>, format: TextureFormat) -> Renderer {
        let instance = Instance::new(adapter::instance_descriptor());
        let (_adapter, device, queue) = adapter::acquire(&instance, None, AdapterPolicy::Auto)
            .await
            .unwrap_or_else(|why| {
                panic!("renderer: no GPU adapter could be used for a test's target: {why}")
            });
        Self::new(device, queue, Sink::Headless, format, size)
    }

    /// The console's backdrop rectangle, in window pixels, while the
    /// console is open. For the GPU-backed tests.
    #[cfg(test)]
    pub(crate) fn console_backdrop_extent(&self) -> Option<(f32, f32, f32, f32)> {
        self.console_backdrop
    }

    /// Shape the one backdrop cell if no backdrop has needed it yet.
    /// Called by the two overlay rebuilds whenever they set a backdrop,
    /// so a frame that draws one finds it shaped; a no-op after the
    /// first. Takes the font system for that first shaping only.
    fn ensure_backdrop_cell(&mut self) {
        if self.backdrop_cell.is_none() {
            let mut font_system = fonts::acquire_font_system_write("renderer::backdrop_cell");
            self.backdrop_cell = Some(backdrop::BackdropCell::shape(&mut font_system));
        }
    }

    /// Set the screen-space FPS readout mode. Routes through the
    /// decree bus so `should_render` / `StartRender` and the FPS
    /// toggle share a single in-renderer mutation point.
    pub fn set_fps_display(&mut self, mode: FpsDisplayMode) {
        self.process_decree(RenderDecree::SetFpsDisplay(mode));
    }

    /// Current FPS readout mode. Used by `ToggleFps` / `ToggleFpsDebug`
    /// dispatch arms to compute the next state.
    pub fn fps_display_mode(&self) -> FpsDisplayMode {
        self.fps_display_mode
    }

    /// Forward a redraw request to the underlying winit window. On
    /// web (winit-web) this schedules an internal
    /// `requestAnimationFrame`; multiple calls in one event chain
    /// coalesce to a single delivery, so it is safe to call from any
    /// handler that mutated visual state.
    ///
    /// Browser-only consumer: `run_wasm` drives its frames off winit
    /// redraw requests, while native's loop runs `ControlFlow::Poll`
    /// and never asks. Lint armed on wasm32 so the day that changes,
    /// wasm32 says so.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn request_redraw(&self) {
        match &self.sink {
            Sink::Window { window, .. } => window.request_redraw(),
            #[cfg(not(target_arch = "wasm32"))]
            Sink::Headless => {}
        }
    }

    /// Draw one frame, and report whether it was drawn: `Ok` when the
    /// frame's CPU half ran (every buffer current, every glyph
    /// prepared), whether or not the surface then had a texture to
    /// present into; `Err` naming why for a frame the renderer skipped.
    /// The native shell's redraw ledger reads it (`app::settle`), so an
    /// occluded window that never presents still counts as drawn, and
    /// both shells ask again after a transient skip. The
    /// loop paces itself — `ControlFlow::Poll` plus the
    /// surface's own vsync backpressure — so there is no rate gate
    /// here.
    ///
    /// There used to be one: a `RedrawMode` with `OnRequest` /
    /// `FpsLimit(n)` / `NoLimit` arms. The field was written exactly
    /// once, to `NoLimit`, so the other two were unreachable, and the
    /// `FpsLimit` arm subtracted one `Duration` from another with no
    /// saturation — with its seeded values (10 ms target, 16 ms last
    /// render) the very first tick would have underflowed and
    /// panicked. Wiring the mode would not have enabled a feature; it
    /// would have crashed the frame. Reinstating a frame cap means
    /// writing the arithmetic, not reviving this.
    #[inline]
    pub(crate) fn process(&mut self) -> Result<(), FrameSkip> {
        if self.fps_display_mode != FpsDisplayMode::Off {
            self.tick_fps();
            self.rebuild_fps_overlay_if_needed();
        }
        self.rebuild_mode_status_overlay_if_needed();
        self.render()
    }

    /// Set the status line's text. `None` clears it; `Some(text)` shows
    /// the line on the next frame. The writer is the view's sync
    /// (`MapView::sync`), which hands over the document's status — what
    /// a refused change says (D28) — whenever it changes, on both
    /// targets; native's console clears it when it switches the mode.
    pub fn set_mode_status_text(&mut self, text: Option<String>) {
        self.mode_status_text = text;
    }

    /// Re-shape the cyan mode-status line when `self.mode_status_text`
    /// has changed since the last shape. Sibling of
    /// [`Self::rebuild_fps_overlay_if_needed`]; same caching
    /// discipline (skip when nothing changed). Silent on font-system
    /// lock contention — the cache key advance is gated on a
    /// successful shaping, so a missed lock retries on the next
    /// process() cycle without permanently dropping the overlay.
    #[inline]
    fn rebuild_mode_status_overlay_if_needed(&mut self) {
        // Same-string short-circuit — no parse / no allocation when
        // the active mode hasn't changed (every drag-drain frame
        // hits this path).
        if self.mode_status_text.as_deref() == self.last_mode_status_shaped.as_deref() {
            return;
        }
        let Some(mut font_system) = fonts::try_acquire_font_system_write("renderer::mode_status_overlay")
        else {
            // Lock contention — leave the cache state alone so the
            // next process() cycle retries shaping. Pre-fix this
            // advanced `last_mode_status_shaped` before the lock
            // check, which permanently lost the overlay until the
            // text changed again.
            return;
        };
        self.mode_status_overlay_buffers.clear();
        if let Some(text) = self.mode_status_text.as_deref() {
            let attrs = Attrs::new().color(MODE_STATUS_COLOR);
            let buf = chrome_text::create_text_buffer(
                &mut font_system,
                text,
                &attrs,
                MODE_STATUS_FONT_SIZE_PT,
                MODE_STATUS_OVERLAY_POS,
                MODE_STATUS_OVERLAY_BOUNDS,
            );
            self.mode_status_overlay_buffers.push(buf);
        }
        // Cache key advances only after a successful shape (or an
        // empty-text path that explicitly cleared the buffers).
        self.last_mode_status_shaped = self.mode_status_text.clone();
    }

    /// Re-shape the yellow "FPS: N" screen-space overlay when the
    /// integer `self.fps` value has changed since the last shape.
    /// Called from `process()` after `tick_fps`. In Snapshot mode
    /// the value only changes every `FPS_WINDOW` frames, so most
    /// rebuilds early-return; in Debug mode the value can change
    /// every frame, but cosmic-text shaping a 6-glyph string is
    /// cheap and only fires when the rounded integer actually
    /// shifts. Silent on font-system lock contention — the next
    /// process() cycle retries.
    ///
    /// `self.fps == None` is rendered as `"FPS: -"` to signal idle:
    /// since the overlay no longer forces continuous rendering,
    /// the readout reflects the app's actual workload — when no
    /// frames are being drawn, the dash makes that explicit
    /// instead of leaving a stale numeric value frozen on screen.
    #[inline]
    fn rebuild_fps_overlay_if_needed(&mut self) {
        if matches!(self.fps_display_mode, FpsDisplayMode::Off) {
            return;
        }
        if self.fps == self.last_fps_shaped && !self.fps_overlay_buffers.is_empty() {
            return;
        }
        let Some(mut font_system) = fonts::try_acquire_font_system_write("renderer::fps_overlay") else {
            return;
        };
        let text = match self.fps {
            Some(n) => format!("FPS: {}", n),
            None => "FPS: -".to_string(),
        };
        let attrs = Attrs::new().color(baumhard::font::Color::rgba(255, 235, 0, 255));
        let buf =
            chrome_text::create_text_buffer(&mut font_system, &text, &attrs, 16.0, (8.0, 8.0), (200.0, 24.0));
        self.fps_overlay_buffers.clear();
        self.fps_overlay_buffers.push(buf);
        self.last_fps_shaped = self.fps;
    }

    /// Capture the wall-clock interval since the previous frame and
    /// update `self.fps` according to the active display mode.
    /// Wall-clock (rather than the duration of the `render()` call)
    /// is load-bearing: `render()` can early-return on a contended
    /// font-system lock under heavy drag / scene-rebuild load, which
    /// would otherwise shrink the measured cost to a near-zero
    /// early-return and inflate the reported FPS into the hundreds of
    /// thousands.
    ///
    /// A frame interval longer than [`Self::IDLE_FRAME_THRESHOLD_US`]
    /// indicates the previous "frame" was actually idle wall-clock,
    /// not a render — under event-driven rendering the loop parks
    /// between user actions. Resuming from such a gap discards the
    /// spurious huge interval (folding it into a real FPS reading
    /// would compute "FPS: 1") and resets the readout to idle so
    /// the next genuine frame interval lands fresh.
    #[inline]
    fn tick_fps(&mut self) {
        let now = Instant::now();
        let frame_micros = self
            .last_frame_instant
            .map(|prev| now.duration_since(prev).as_micros())
            .unwrap_or(0);
        self.last_frame_instant = Some(now);

        // Honor a pending idle paint queued by `set_fps_idle`: this
        // transition render must show "-" even if the rolling avg
        // would compute a value from prior active samples. Clear
        // the rolling window so the next active session starts
        // fresh instead of inheriting stale samples.
        if self.fps_pending_idle_paint {
            self.fps_pending_idle_paint = false;
            self.fps = None;
            self.fps_ring.clear();
            return;
        }

        if frame_micros > Self::IDLE_FRAME_THRESHOLD_US {
            // Resuming from idle. Don't fold the huge gap into a
            // FPS sample; just reset to the idle marker. The next
            // real frame's interval lands in a clean state.
            self.fps = None;
            return;
        }

        match self.fps_display_mode {
            FpsDisplayMode::Off => {}
            FpsDisplayMode::Snapshot => {
                if self.fps_clock.is_multiple_of(FPS_WINDOW) && frame_micros > 0 {
                    self.fps = Some((1_000_000u128 / frame_micros) as usize);
                }
                self.fps_clock = self.fps_clock.wrapping_add(1);
            }
            FpsDisplayMode::Debug => {
                if frame_micros > 0 {
                    self.fps_ring.push(frame_micros);
                }
                // `checked_div`: a zero average leaves `self.fps` as it was.
                if let Some(fps) = self
                    .fps_ring
                    .avg_micros()
                    .and_then(|avg| 1_000_000u128.checked_div(avg))
                {
                    self.fps = Some(fps as usize);
                }
            }
        }
    }

    /// A frame interval longer than this is treated as a wall-clock
    /// idle gap rather than a render — see [`Self::tick_fps`]. 500ms
    /// is comfortably longer than any genuine frame at refresh rates
    /// down to 4Hz and short enough that a brief lull during typing
    /// shows as idle in the overlay.
    const IDLE_FRAME_THRESHOLD_US: u128 = 500_000;

    /// True iff the FPS overlay currently displays a numeric reading
    /// (i.e., the renderer recently sampled a frame interval). False
    /// if the overlay is at the idle marker or has never been
    /// populated. Used by the event loop to decide whether the
    /// overlay needs one more redraw to flip to "-" before parking.
    // Native-driver-only, with the two below: the active-to-idle FPS
    // transition belongs to `ControlFlow::Wait`, which only native's
    // loop enters.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn has_live_fps(&self) -> bool {
        self.fps.is_some()
    }

    /// Decide whether the active→idle FPS transition should be
    /// deferred. Returns `Some(deadline)` if the last rendered
    /// frame is more recent than `grace`, meaning the user could
    /// still be reading the live reading and an immediate flip to
    /// "-" would flicker. The caller pairs this with
    /// `ControlFlow::WaitUntil(deadline)` so the loop wakes after
    /// the grace period to commit the transition. Returns `None`
    /// when the FPS is already idle, when no frame has been
    /// rendered yet, or when the grace period has already elapsed
    /// (transition can fire immediately).
    ///
    /// Without this, an active throttled drag whose `should_drain`
    /// gates produce momentary `needs_continuation == false` gaps
    /// between drain frames would flash "FPS: -" between every
    /// drain — making the readout unusable as a diagnostic.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn fps_idle_defer_deadline(&self, grace: Duration) -> Option<Instant> {
        if !self.has_live_fps() {
            return None;
        }
        let last = self.last_frame_instant?;
        let age = last.elapsed();
        if age >= grace {
            None
        } else {
            Some(last + grace)
        }
    }

    /// Force the FPS overlay into the idle state so the next render
    /// shows "-" instead of a stale numeric value. Called when the
    /// event loop transitions from active rendering to
    /// `ControlFlow::Wait`. Pairs with a `request_redraw` so the
    /// transition lands one final frame before the loop parks.
    /// The arm-and-consume flag protects the transitional frame
    /// from `tick_fps` re-computing a numeric reading from the
    /// pre-idle rolling-avg samples.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn set_fps_idle(&mut self) {
        self.fps = None;
        self.fps_pending_idle_paint = true;
    }

    #[inline]
    fn update_surface_size(&mut self, width: u32, height: u32) {
        if width == 0 {
            error!("renderer: width has to be higher than 0 but was {}", width);
            return;
        }
        if height == 0 {
            error!("renderer: height has to be higher than 0 but was {}", height);
            return;
        }
        let max_dim = self.device.limits().max_texture_dimension_2d;
        let (width, height) = clamp_surface_size_to_gpu_limit(width, height, max_dim);
        info!("Updating surface size");
        self.surface_size = (width, height);
        match &mut self.sink {
            Sink::Window { surface, config, .. } => {
                config.width = width;
                config.height = height;
                surface.configure(&self.device, config);
            }
            // No surface to configure: the next frame is drawn for the
            // new size because every frame reads it from here.
            #[cfg(not(target_arch = "wasm32"))]
            Sink::Headless => {}
        }
        // The glyphon viewport follows on the next frame: every frame's
        // `prepare_frame` sizes it to the target it draws.
        self.camera.set_viewport_size(width, height);
        // Canvas-space glyph positions and shaped buffers survive a
        // viewport resize; the per-frame `visible_at` cull handles
        // whether each buffer falls inside the new bounds.
    }
}

pub struct MindMapTextBuffer {
    pub buffer: Buffer,
    pub pos: (f32, f32),
    pub bounds: (f32, f32),
    /// Per-`GlyphArea` zoom window copied in at buffer-build time.
    /// The main render loop skips this buffer whenever
    /// `camera.zoom` falls outside the window. Default (both
    /// bounds `None`) renders at every zoom — existing buffers pay
    /// nothing.
    pub zoom_visibility: baumhard::gfx_structs::zoom_visibility::ZoomVisibility,
}

impl MindMapTextBuffer {
    /// Where a buffer stamped at `emission_offset` from `anchor`, in
    /// a tree registered at `offset`, is drawn: `anchor +
    /// emission_offset + offset`, summed in that order.
    ///
    /// The one spelling of the stamp-placement sum: the walker places
    /// every buffer it emits through it, an outlined element's halo
    /// stamps each at their own offset.
    pub(super) fn stamp_pos(anchor: (f32, f32), emission_offset: (f32, f32), offset: Vec2) -> (f32, f32) {
        (
            anchor.0 + emission_offset.0 + offset.x,
            anchor.1 + emission_offset.1 + offset.y,
        )
    }

    /// Should this text buffer render at the current camera
    /// state? Asks the one cull predicate, `Camera2D::visibility`,
    /// which the IPC scene introspection reports verbatim. Pure,
    /// no allocation; the render loop calls this once per buffer
    /// per frame in the `main_text_areas` collector.
    pub(super) fn visible_at(&self, camera: &baumhard::gfx_structs::camera::Camera2D) -> bool {
        let pos = Vec2::new(self.pos.0, self.pos.1);
        let size = Vec2::new(self.bounds.0, self.bounds.1);
        camera.visibility(pos, size, self.zoom_visibility).is_visible()
    }
}

#[cfg(test)]
mod tests;
