// SPDX-License-Identifier: MPL-2.0

//! Offscreen capture: one frame drawn into a texture of the renderer's
//! own instead of the swapchain, and read back to the CPU as tightly
//! packed RGBA8 rows.
//!
//! **The window's frame, not a copy of it.** The frame goes through
//! [`Renderer::prepare_frame`] and [`Renderer::encode_frame`], the pair
//! `render()` draws the window with, at whatever camera the caller
//! frames. A capture at the live camera and the surface size therefore
//! draws the pixels the window shows; one at another size, framing or
//! pixel ratio draws the same scene projected differently. Nothing the
//! renderer holds is changed by a capture except the GPU staging it
//! re-uploads every frame anyway (both rect batches, the text prepares
//! and the glyphon viewport), so the next window frame is exactly the
//! frame it would have been.
//!
//! **The target is the frames' own format.** Both pipelines are built
//! for the renderer's `format` — the sRGB one the window's surface
//! offers, or `HEADLESS_FORMAT` with no window — so the offscreen
//! texture is created in it too: the format has one source and the
//! capture reads it from there rather than naming a constant beside it.
//! The window's texture cannot be copied from (it is `RENDER_ATTACHMENT`
//! only); this one is `COPY_SRC` as well, and with no window it is the
//! only texture a frame is ever drawn into.
//!
//! **Blocking, and bounded.** The readback waits for the GPU on the main
//! thread, as a synchronous screenshot must, each wait with a timeout
//! ([`GPU_WAIT`]) well inside the `FreezeWatchdog`'s threshold, and
//! reports progress after each, so a wedged driver fails the capture
//! rather than the process while a large one runs to completion.
//!
//! Native-only at this module boundary: a browser cannot block on a
//! buffer map (`CLAUDE.md` "Dual-target status", "Screenshot capture").

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use baumhard::gfx_structs::camera::Camera2D;
use web_time::Instant;

use super::render::{FrameSkip, ModalLayer};
use super::Renderer;

/// How long a capture waits for the GPU to finish one band of its frame
/// and copy before it gives up. Half the `FreezeWatchdog`'s abort
/// threshold (a test beside the watchdog holds the two together), and
/// the engine ticks the watchdog after every band and every encoded
/// chunk: a GPU that never answers costs the agent a failed capture,
/// and a capture that is still progressing never reads as a hang.
pub(crate) const GPU_WAIT: Duration = Duration::from_secs(5);

/// The order a 4-byte-per-pixel surface format stores its channels in.
/// The readback swizzles to RGBA from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelOrder {
    Rgba,
    Bgra,
}

impl ChannelOrder {
    /// The order of `format`'s bytes, or `None` for a format that is not
    /// 8 bits per channel in one of the two orders: a capture refuses
    /// those rather than guessing at their layout. sRGB and linear
    /// variants store the same bytes — the encoding is the value's, not
    /// the layout's — and the bytes are what the display shows, so both
    /// are read as they are.
    pub(crate) fn of(format: wgpu::TextureFormat) -> Option<ChannelOrder> {
        use wgpu::TextureFormat::*;
        match format {
            Rgba8Unorm | Rgba8UnormSrgb => Some(ChannelOrder::Rgba),
            Bgra8Unorm | Bgra8UnormSrgb => Some(ChannelOrder::Bgra),
            _ => None,
        }
    }
}

/// Why a capture produced no pixels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CaptureFailure {
    /// The frame was not drawn, for the same reason a window frame
    /// would not have been. Not an error: what the reply reports as
    /// `rendered: false`.
    Skipped(FrameSkip),
    /// The surface format is not one [`ChannelOrder::of`] can read.
    UnsupportedFormat(wgpu::TextureFormat),
    /// The GPU failed the readback: the wait timed out, or the buffer
    /// would not map. The message says which.
    Gpu(String),
}

/// One captured frame: `height` rows of `width` RGBA8 pixels, top row
/// first, with no padding between rows.
#[derive(Debug)]
pub(crate) struct CapturedPixels {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

/// Bytes per row of a `width`-pixel, 4-byte-per-pixel image in a
/// texture-to-buffer copy: the packed row, rounded up to
/// `COPY_BYTES_PER_ROW_ALIGNMENT` (256), which wgpu requires of every
/// copy's row stride. O(1).
pub(crate) fn padded_bytes_per_row(width: u32) -> u32 {
    let packed = width * 4;
    packed.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

/// The row bands a `height`-row readback is copied in, as
/// `(first_row, rows)`: as many rows per band as fit a staging buffer of
/// `max_buffer_size` bytes at `padded_row` bytes each, and never fewer
/// than one. A frame whose rows all fit is one band.
///
/// The bands exist because the device's buffer limit is smaller than the
/// largest image a capture admits: `Renderer::new` asks for
/// `downlevel_defaults()` limits, whose `max_buffer_size` is 256 MiB,
/// and a 64 Mpx frame is 256 MiB before its rows are padded.
pub(crate) fn row_bands(
    height: u32,
    padded_row: u32,
    max_buffer_size: u64,
) -> impl Iterator<Item = (u32, u32)> {
    let per_band = (max_buffer_size / u64::from(padded_row.max(1))).clamp(1, u64::from(height.max(1))) as u32;
    (0..height)
        .step_by(per_band as usize)
        .map(move |first| (first, per_band.min(height - first)))
}

/// Append `rows` rows of a mapped band to `out` as packed RGBA: each row
/// is the first `width * 4` bytes of its `padded_row`-byte stride, its
/// channels swizzled from `order`. O(bytes), one pass, no allocation
/// beyond `out`'s growth.
pub(crate) fn pack_rows(
    band: &[u8],
    width: u32,
    padded_row: u32,
    rows: u32,
    order: ChannelOrder,
    out: &mut Vec<u8>,
) {
    let packed = width as usize * 4;
    for row in band.chunks(padded_row as usize).take(rows as usize) {
        let pixels = &row[..packed];
        match order {
            ChannelOrder::Rgba => out.extend_from_slice(pixels),
            ChannelOrder::Bgra => {
                let (whole, _) = pixels.as_chunks::<4>();
                for [b, g, r, a] in whole {
                    out.extend_from_slice(&[*r, *g, *b, *a]);
                }
            }
        }
    }
}

impl Renderer {
    /// The GPU's largest 2D texture side, in pixels: the largest side a
    /// capture target can have.
    pub(crate) fn max_texture_side(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    /// Draw one frame through `camera` into an offscreen texture of
    /// `camera.viewport_size` pixels and read it back, with or without
    /// the open modal as `modal` says.
    ///
    /// Refreshes the screen-space overlays the way `process()` does
    /// before its render — the mode-status line and the FPS readout
    /// reshape only if their text changed — but does not tick the FPS
    /// clock: a capture is not a frame of the window's.
    ///
    /// # Costs
    ///
    /// One full frame prepare and encode, a texture and a staging
    /// buffer of the target's size (the buffer at most `max_buffer_size`,
    /// reused per row band), and one blocking GPU wait per band, each
    /// bounded by [`GPU_WAIT`], with `progress` called after every band
    /// that lands — so a large capture reports its progress band by band
    /// and a band the GPU never finishes fails the capture. The result
    /// holds the image once, packed.
    pub(crate) fn capture_frame(
        &mut self,
        camera: &Camera2D,
        modal: ModalLayer,
        progress: &mut dyn FnMut(),
    ) -> Result<CapturedPixels, CaptureFailure> {
        if !self.should_render {
            return Err(CaptureFailure::Skipped(FrameSkip::NotStarted));
        }
        let format = self.format;
        let order = ChannelOrder::of(format).ok_or(CaptureFailure::UnsupportedFormat(format))?;
        let (width, height) = camera.viewport_size;
        // `capture::plan` is the one way here, and it refuses any side the
        // GPU cannot hold and never plans an empty one.
        debug_assert!(
            (1..=self.max_texture_side()).contains(&width) && (1..=self.max_texture_side()).contains(&height),
            "an unplanned capture target: {width}×{height}"
        );
        let started = Instant::now();
        if self.fps_display_mode != crate::application::common::FpsDisplayMode::Off {
            self.rebuild_fps_overlay_if_needed();
        }
        self.rebuild_mode_status_overlay_if_needed();
        let prepared = self
            .prepare_frame(camera, modal)
            .map_err(CaptureFailure::Skipped)?;

        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture_target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let padded_row = padded_bytes_per_row(width);
        let mut bands = row_bands(height, padded_row, self.device.limits().max_buffer_size).peekable();
        let band_rows = bands.peek().map_or(height, |&(_, rows)| rows);
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture_staging"),
            size: u64::from(padded_row) * u64::from(band_rows),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // The first band's copy rides the frame's own encoder, so a
        // frame that fits one band is one submission.
        let mut encoder = Some(self.encode_frame(&prepared, &view));
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for (first_row, rows) in bands {
            let mut band_encoder = encoder.take().unwrap_or_else(|| {
                self.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None })
            });
            band_encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: first_row,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_row),
                        rows_per_image: Some(rows),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height: rows,
                    depth_or_array_layers: 1,
                },
            );
            let submission = self.queue.submit(Some(band_encoder.finish()));
            self.read_band(&staging, submission, |band| {
                pack_rows(band, width, padded_row, rows, order, &mut rgba)
            })?;
            progress();
        }
        self.atlas.trim();
        log::debug!(
            "renderer: captured {width}×{height} in {} ms",
            started.elapsed().as_millis()
        );
        Ok(CapturedPixels { width, height, rgba })
    }

    /// Map `staging` once `submission` has run, hand its bytes to `read`,
    /// and unmap it for the next band.
    fn read_band(
        &self,
        staging: &wgpu::Buffer,
        submission: wgpu::SubmissionIndex,
        read: impl FnOnce(&[u8]),
    ) -> Result<(), CaptureFailure> {
        let mapped: Arc<OnceLock<bool>> = Arc::new(OnceLock::new());
        let signal = Arc::clone(&mapped);
        staging.map_async(wgpu::MapMode::Read, .., move |result| {
            // A second delivery cannot happen; `set` would refuse it.
            let _ = signal.set(result.is_ok());
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(GPU_WAIT),
            })
            .map_err(|e| CaptureFailure::Gpu(format!("the GPU did not finish the capture: {e}")))?;
        match mapped.get() {
            Some(true) => {}
            Some(false) => {
                return Err(CaptureFailure::Gpu(
                    "the capture buffer could not be mapped".into(),
                ))
            }
            None => {
                return Err(CaptureFailure::Gpu(
                    "the capture buffer was not mapped after the GPU finished".into(),
                ))
            }
        }
        {
            let view = staging
                .get_mapped_range(..)
                .map_err(|e| CaptureFailure::Gpu(format!("the capture buffer could not be read: {e}")))?;
            read(&view);
        }
        staging.unmap();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows are padded up to the 256-byte copy alignment and no
    /// further: exactly-aligned widths keep their packed size.
    #[test]
    fn test_padded_bytes_per_row_rounds_up_to_the_copy_alignment() {
        assert_eq!(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 256);
        for (width, padded) in [
            (1, 256),
            (63, 256),
            (64, 256),
            (65, 512),
            (1920, 7680),
            (1366, 5632),
        ] {
            assert_eq!(padded_bytes_per_row(width), padded, "width {width}");
            assert!(padded_bytes_per_row(width) >= width * 4);
        }
    }

    /// The bands cover every row exactly once, in order, none larger
    /// than the buffer limit allows — and one band when everything fits.
    /// A limit below one row still makes progress, a row at a time.
    #[test]
    fn test_row_bands_cover_every_row_once_within_the_buffer_limit() {
        let one = row_bands(1080, 7680, 256 << 20).collect::<Vec<_>>();
        assert_eq!(one, vec![(0, 1080)]);

        for (height, padded, limit) in [(1000, 256, 256 * 300), (7, 512, 512 * 2), (5, 1024, 10)] {
            let bands: Vec<_> = row_bands(height, padded, limit).collect();
            let mut next = 0;
            for &(first, rows) in &bands {
                assert_eq!(first, next, "{bands:?}");
                assert!(rows >= 1);
                assert!(u64::from(rows) * u64::from(padded) <= limit.max(u64::from(padded)));
                next = first + rows;
            }
            assert_eq!(next, height, "{bands:?}");
        }
        assert_eq!(row_bands(1000, 256, 256 * 300).count(), 4);
    }

    /// Packing drops each row's padding and, for a BGRA surface, swaps
    /// red and blue while leaving green and alpha where they were.
    #[test]
    fn test_pack_rows_drops_the_padding_and_swizzles_bgra() {
        // Two rows of two pixels, each row padded to 12 bytes.
        let band = [
            1, 2, 3, 4, 5, 6, 7, 8, 0xEE, 0xEE, 0xEE, 0xEE, //
            9, 10, 11, 12, 13, 14, 15, 16, 0xEE, 0xEE, 0xEE, 0xEE,
        ];
        let mut rgba = Vec::new();
        pack_rows(&band, 2, 12, 2, ChannelOrder::Rgba, &mut rgba);
        assert_eq!(rgba, (1..=16).collect::<Vec<u8>>());

        let mut swizzled = vec![0xAA];
        pack_rows(&band, 2, 12, 2, ChannelOrder::Bgra, &mut swizzled);
        assert_eq!(
            swizzled,
            vec![0xAA, 3, 2, 1, 4, 7, 6, 5, 8, 11, 10, 9, 12, 15, 14, 13, 16],
            "appends after what is there"
        );

        let mut first_row_only = Vec::new();
        pack_rows(&band, 2, 12, 1, ChannelOrder::Rgba, &mut first_row_only);
        assert_eq!(first_row_only, (1..=8).collect::<Vec<u8>>());
    }

    #[test]
    fn test_channel_order_reads_exactly_the_eight_bit_four_channel_formats() {
        use wgpu::TextureFormat::*;
        assert_eq!(ChannelOrder::of(Bgra8UnormSrgb), Some(ChannelOrder::Bgra));
        assert_eq!(ChannelOrder::of(Bgra8Unorm), Some(ChannelOrder::Bgra));
        assert_eq!(ChannelOrder::of(Rgba8UnormSrgb), Some(ChannelOrder::Rgba));
        assert_eq!(ChannelOrder::of(Rgba8Unorm), Some(ChannelOrder::Rgba));
        for refused in [Rgba16Float, Rgb10a2Unorm, R8Unorm, Rgba8Snorm] {
            assert_eq!(ChannelOrder::of(refused), None, "{refused:?}");
        }
    }
}
