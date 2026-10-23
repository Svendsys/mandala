// SPDX-License-Identifier: MPL-2.0

//! The captured frame as a PNG: 8-bit RGBA, sRGB-tagged.
//!
//! **The bytes are what the display shows.** The offscreen target is
//! the surface's own format, so its bytes are the bytes the window
//! hands the compositor; for the sRGB surface `Renderer::new` prefers,
//! they are sRGB-encoded, which is what the PNG's `sRGB` chunk says.
//!
//! **Fast compression.** The encode runs on the main thread inside the
//! frame budget the `FreezeWatchdog` polices, and a capture can be 64
//! Mpx. `Compression::Fast` keeps that bounded; a smaller file is the
//! client's to make with a tool built for it.

use std::io::Write;

use crate::application::renderer::CapturedPixels;

/// How much of the image is encoded between two `progress` calls: a
/// slice small enough that encoding it is never what a watchdog waits
/// on.
const ENCODE_CHUNK_BYTES: usize = 4 << 20;

/// Write `pixels` to `out` as a PNG, calling `progress` after every
/// [`ENCODE_CHUNK_BYTES`] encoded.
///
/// `progress` is how a long encode stays honest with the
/// `FreezeWatchdog`: the engine ticks it per chunk, the way the design
/// has `clock.step` tick it per heartbeat, so an encode that is making
/// progress never reads as a hang and one that stops does.
///
/// # Costs
///
/// One pass of DEFLATE over the image; nothing is allocated beyond the
/// encoder's own window, and the image is not copied.
pub(crate) fn encode(
    pixels: &CapturedPixels,
    out: &mut dyn Write,
    progress: &mut dyn FnMut(),
) -> Result<(), String> {
    let mut encoder = png::Encoder::new(out, pixels.width, pixels.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("could not start the PNG: {e}"))?;
    let mut stream = writer
        .stream_writer()
        .map_err(|e| format!("could not start the PNG: {e}"))?;
    for chunk in pixels.rgba.chunks(ENCODE_CHUNK_BYTES) {
        stream
            .write_all(chunk)
            .map_err(|e| format!("could not encode the PNG: {e}"))?;
        progress();
    }
    stream
        .finish()
        .and_then(|()| writer.finish())
        .map_err(|e| format!("could not finish the PNG: {e}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Decode a PNG back to `(width, height, rgba)`, for the tests here
    /// and the engine's.
    pub(crate) fn decode(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().expect("a PNG header");
        let mut rgba = vec![0; reader.output_buffer_size().expect("a bounded image")];
        let info = reader.next_frame(&mut rgba).expect("one frame");
        assert_eq!(
            (info.color_type, info.bit_depth),
            (png::ColorType::Rgba, png::BitDepth::Eight)
        );
        rgba.truncate(info.buffer_size());
        (info.width, info.height, rgba)
    }

    /// The encoded image decodes to the very pixels it was given, at
    /// its size, top row first, and says it is sRGB.
    #[test]
    fn test_encode_round_trips_the_pixels_and_tags_srgb() {
        let pixels = CapturedPixels {
            width: 3,
            height: 2,
            rgba: (0..24).map(|i| (i * 10) as u8).collect(),
        };
        let mut bytes = Vec::new();
        encode(&pixels, &mut bytes, &mut || {}).expect("encodes");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(decode(&bytes), (3, 2, pixels.rgba.clone()));

        let reader = png::Decoder::new(std::io::Cursor::new(&bytes))
            .read_info()
            .expect("a PNG header");
        assert!(reader.info().srgb.is_some(), "the sRGB chunk is written");
    }

    /// `progress` is called once per encoded chunk: a large image
    /// reports its progress several times, and the bytes are the same
    /// as one written whole.
    #[test]
    fn test_encode_reports_progress_per_chunk() {
        let (width, height) = (1024, 2050);
        let pixels = CapturedPixels {
            width,
            height,
            rgba: (0..width as usize * height as usize * 4)
                .map(|i| (i % 251) as u8)
                .collect(),
        };
        let mut calls = 0;
        let mut bytes = Vec::new();
        encode(&pixels, &mut bytes, &mut || calls += 1).expect("encodes");
        assert_eq!(calls, pixels.rgba.len().div_ceil(ENCODE_CHUNK_BYTES));
        assert!(
            calls > 1,
            "a {} MiB image is more than one chunk",
            pixels.rgba.len() >> 20
        );
        assert_eq!(decode(&bytes), (width, height, pixels.rgba));
    }

    /// The slice `format/ipc.md` states is the one the encoder ticks per.
    #[test]
    fn test_the_documented_slice_is_the_encoded_one() {
        use baumhard::util::doc_fixtures::{repo_path, section_text};
        let section = section_text(&repo_path("format/ipc.md"), "### `capture.screenshot` (IPC-07)");
        assert_eq!(ENCODE_CHUNK_BYTES % (1 << 20), 0, "stated in whole MiB");
        let stated = format!("{} MiB slices", ENCODE_CHUNK_BYTES >> 20);
        assert!(
            section.contains(&stated),
            "format/ipc.md §capture.screenshot must state {stated}"
        );
    }

    /// A pixel buffer that does not match its stated size is an error
    /// reply, never a panic.
    #[test]
    fn test_encode_refuses_a_buffer_of_the_wrong_size() {
        let pixels = CapturedPixels {
            width: 4,
            height: 4,
            rgba: vec![0; 10],
        };
        let failed = encode(&pixels, &mut Vec::new(), &mut || {});
        assert!(matches!(&failed, Err(m) if m.contains("PNG")), "{failed:?}");
    }
}
