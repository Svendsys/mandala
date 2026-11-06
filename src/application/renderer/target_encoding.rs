// SPDX-License-Identifier: MPL-2.0

//! How an authored color reaches a target of a given format: the one
//! answer the three passes that write a map's colors share — the clear
//! (the map's background), glyphon's text, and the glyph-run pass.
//!
//! **Why the passes must agree.** Under D12 every pixel a map draws is
//! the background or a glyph, so a glyph colored like the background is
//! the format's way of drawing nothing — and the app's way of keeping a
//! glyph legible is its distance from the background. If one pass
//! decodes `#rrggbb` to linear light and another writes the byte as it
//! is, the same map shows a different glyph-to-background relationship
//! on two targets. CODE_CONVENTIONS §4 makes the targets peers, so a
//! glyph colored like the background matches it exactly on every one
//! of them.
//!
//! **The two encodings.** A target either encodes what a pass writes
//! (an sRGB format: the GPU applies the sRGB transfer function on the
//! way in) or stores it as it is (a `Unorm` or float format):
//!
//! - native's window and the headless renderer pick an sRGB format;
//! - the browser on **WebGL2** is offered `Rgba8UnormSrgb` when the
//!   context supports sRGB, and takes it;
//! - the browser on **WebGPU** is never offered one: wgpu 30's
//!   `WebSurface::get_capabilities` lists `Bgra8Unorm`, `Rgba8Unorm`
//!   and possibly `Rgba16Float`, so `Renderer::for_window` falls back
//!   to the first, a linear format. That is the bundle's path whenever
//!   the browser's WebGPU offers an adapter.
//!
//! On an sRGB target every pass decodes the authored byte to linear
//! light, which the GPU encodes back; on a linear one every pass writes
//! the byte as it is, which the target keeps and the compositor shows
//! as sRGB. Either way the frame holds `#rrggbb` as written.
//!
//! Pure, so the renderer-free tier pins the passes against each other
//! for every format (§T8): `test_the_three_color_passes_agree_on_every_target`.

use glyphon::ColorMode;
use wgpu::TextureFormat;

/// Whether a target encodes what is written to it, and therefore
/// whether an authored sRGB byte is decoded before it is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetEncoding {
    /// An sRGB format: the GPU encodes a pass's linear output as it
    /// writes, so each authored byte is decoded to linear light first.
    Srgb,
    /// A `Unorm` or float format: the target stores what a pass
    /// writes, so each authored byte goes in as it is.
    Linear,
}

impl TargetEncoding {
    /// The encoding of a target of `format`. O(1).
    pub(crate) fn of(format: TextureFormat) -> Self {
        if format.is_srgb() {
            TargetEncoding::Srgb
        } else {
            TargetEncoding::Linear
        }
    }

    /// The value a pass writes for the authored sRGB byte `byte`: the
    /// byte decoded by the sRGB transfer function (IEC 61966-2-1) on an
    /// sRGB target, and the byte as a fraction of 255 on a linear one.
    /// The clear takes it as it is; the glyph-run pass rounds it to
    /// `f32`. O(1).
    pub(crate) fn channel(self, byte: u8) -> f64 {
        let value = f64::from(byte) / 255.0;
        match self {
            TargetEncoding::Srgb => {
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            }
            TargetEncoding::Linear => value,
        }
    }

    /// glyphon's color mode for this target. `Accurate` tags every
    /// glyph for its shader's decode and keeps color glyphs on an sRGB
    /// page; `Web` writes a glyph's color as it is and keeps color
    /// glyphs on a linear page — glyphon's own mode for "a linear RGB
    /// texture containing sRGB colors", which is what a linear target
    /// is here.
    pub(crate) fn glyphon_color_mode(self) -> ColorMode {
        match self {
            TargetEncoding::Srgb => ColorMode::Accurate,
            TargetEncoding::Linear => ColorMode::Web,
        }
    }

    /// The format of the glyph-run pass's color page, the page full-color
    /// glyphs (emoji) are sampled from. sRGB on an sRGB target, so a
    /// sample is decoded to linear light for the GPU to encode back;
    /// linear on a linear one, so a sample is the stored byte, which the
    /// target keeps — the same choice glyphon's two modes make for their
    /// own color page.
    pub(crate) fn color_page_format(self) -> TextureFormat {
        match self {
            TargetEncoding::Srgb => TextureFormat::Rgba8UnormSrgb,
            TargetEncoding::Linear => TextureFormat::Rgba8Unorm,
        }
    }
}
