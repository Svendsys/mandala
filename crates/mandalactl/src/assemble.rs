// SPDX-License-Identifier: MPL-2.0

//! `mandalactl record assemble <dir>`: a recording's frames and
//! manifest (`format/ipc.md` §`capture.record_start`) turned into
//! something a person can watch — an animated GIF by default, which is
//! what GitHub shows inline in a PR, or an MP4 through the host's
//! `ffmpeg`.
//!
//! **Client-side on purpose.** Nothing here talks to the app, and the
//! app links no encoder: assembly is a job for a tool that reads files,
//! and keeping it out of the app keeps the interactive path lean. The
//! one input is the recording's directory; the manifest is the whole
//! contract, so a recording made by a build that is no longer running
//! assembles the same.
//!
//! **Pacing is the manifest's, so the GIF runs at the speed it was
//! recorded.** Each frame is shown from its `t_ms` until the next
//! frame's (a gap in the recording holds the frame before it, which is
//! what the screen did), and the last until the recording ended. A GIF
//! counts in hundredths of a second, so delays are made from the
//! *rounded* start times rather than rounded one by one: the errors
//! cancel instead of piling up, and a recording's length survives to
//! within a hundredth of a second however many frames it had. Browsers
//! show any GIF frame shorter than 20 ms as 100 ms, which would make a
//! 60 fps recording play six times slow, so frames closer than that are
//! dropped ([`GIF_MIN_DELAY_CS`]) and counted, not left to play wrong.
//!
//! **Bounded.** Frames are decoded and written one at a time, so memory
//! is one frame whatever the recording's length; the GIF keeps at most
//! `max_frames` of them (default [`DEFAULT_MAX_FRAMES`]), thinning
//! evenly — at the same real-time speed — rather than cutting the end
//! off; a result past what GitHub shows inline is said so, with the
//! knobs that shrink it (`width=`, `fps=`).
//!
//! Video is the host's `ffmpeg`, run as a child process and never
//! linked: when it is missing the command says so and names the GIF
//! that needs nothing.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use crate::cli::Invocation;
use crate::params::{tokenize, Token};
use crate::render;
use crate::run::{Exit, Io};
use crate::wire::PROTOCOL;

/// The command word this module serves.
pub const COMMAND: &str = "record";

/// Its one subcommand.
pub const SUBCOMMAND: &str = "assemble";

/// The shortest time a GIF frame is shown, in hundredths of a second.
/// Browsers treat a shorter delay as 100 ms, so a faster frame would
/// play slow; the planner drops frames to keep every delay at or above
/// this rather than emit one.
pub const GIF_MIN_DELAY_CS: u64 = 2;

/// How many frames a GIF keeps when `max_frames=` is not given.
pub const DEFAULT_MAX_FRAMES: usize = 600;

/// The most `max_frames=` accepts: the recorder's own cap.
pub const MAX_FRAMES: usize = 3600;

/// The size of a GIF past which GitHub no longer shows it inline in a
/// comment, in bytes. The result says so rather than leaving a person
/// to find out in the PR.
pub const GITHUB_INLINE_BYTES: u64 = 10 << 20;

/// The most bytes of manifest read: a recording's is a few hundred
/// bytes a frame, so this is past any recording the app makes.
const MANIFEST_BYTES_CAP: u64 = 32 << 20;

/// The most frames a manifest may name.
const MANIFEST_FRAMES_CAP: usize = 100_000;

/// The most bytes one decoded frame may hold: the app's 64 Mpx cap,
/// four bytes each.
const FRAME_BYTES_CAP: usize = 256 << 20;

/// GIF's largest side, in pixels: dimensions are 16 bits.
const GIF_MAX_SIDE: u32 = 65_535;

/// The `gif` crate's quantizer speed, `1` (best, slowest) to `30`. `10`
/// is its own default; a UI full of flat color and fine text is not a
/// photograph and does not repay a slower search.
const GIF_QUANTIZER_SPEED: i32 = 10;

/// The manifest's name in a recording's directory.
const MANIFEST_FILE: &str = "manifest.json";

/// One frame the manifest names.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameRef {
    pub t_ms: f64,
    pub file: String,
}

/// What assembly reads of a recording's `manifest.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub frames: Vec<FrameRef>,
    /// How long the recording ran, in ms (absent from a manifest the
    /// app never finished).
    pub duration_ms: Option<f64>,
    /// Whether the app finished writing it.
    pub complete: bool,
    pub dropped_frames: u64,
}

/// Read a manifest's text, checking everything assembly relies on: its
/// protocol, a frame rate and size that mean something, frames that
/// exist in time order, and file names that stay inside the recording's
/// directory — a manifest is data from a directory, and its `file`
/// is never a path to follow out of it.
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("manifest.json is not JSON: {e}"))?;
    let object = value.as_object().ok_or("manifest.json is not a JSON object")?;
    let protocol = object.get("protocol").and_then(Value::as_u64);
    if protocol != Some(PROTOCOL) {
        return Err(format!(
            "manifest.json is protocol {}, and this mandalactl reads protocol {PROTOCOL}",
            protocol.map_or("missing".to_string(), |p| p.to_string())
        ));
    }
    let whole = |key: &str, max: u64| -> Result<u32, String> {
        object
            .get(key)
            .and_then(Value::as_u64)
            .filter(|n| (1..=max).contains(n))
            .map(|n| n as u32)
            .ok_or_else(|| format!("manifest.json's `{key}` must be a whole number in 1..={max}"))
    };
    let fps = whole("fps", 1000)?;
    let width = whole("width", u64::from(u32::MAX))?;
    let height = whole("height", u64::from(u32::MAX))?;
    let listed = object
        .get("frames")
        .and_then(Value::as_array)
        .ok_or("manifest.json has no `frames` list")?;
    if listed.len() > MANIFEST_FRAMES_CAP {
        return Err(format!(
            "manifest.json names {} frames; this reads at most {MANIFEST_FRAMES_CAP}",
            listed.len()
        ));
    }
    let mut frames: Vec<FrameRef> = Vec::with_capacity(listed.len());
    for (n, frame) in listed.iter().enumerate() {
        let t_ms = frame
            .get("t_ms")
            .and_then(Value::as_f64)
            .filter(|t| t.is_finite() && *t >= 0.0)
            .ok_or_else(|| format!("manifest.json frame {n} has no `t_ms` that is a time"))?;
        if frames.last().is_some_and(|last| t_ms < last.t_ms) {
            return Err(format!(
                "manifest.json frame {n} is earlier than the frame before it"
            ));
        }
        let file = frame
            .get("file")
            .and_then(Value::as_str)
            .filter(|file| is_plain_file_name(file))
            .ok_or_else(|| format!("manifest.json frame {n} has no `file` that is a plain file name"))?;
        frames.push(FrameRef {
            t_ms,
            file: file.to_string(),
        });
    }
    Ok(Manifest {
        fps,
        width,
        height,
        frames,
        duration_ms: object
            .get("duration_ms")
            .and_then(Value::as_f64)
            .filter(|d| d.is_finite() && *d >= 0.0),
        complete: object.get("complete").and_then(Value::as_bool).unwrap_or(false),
        dropped_frames: object.get("dropped_frames").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// A file name and nothing else: no directory part, no `..`, not
/// hidden.
fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
        && Path::new(name).file_name().is_some_and(|f| f == name)
}

/// When the last frame's display ends, in ms from the start: the
/// recording's length, but never less than one frame period after the
/// last frame, so a recording stopped the instant its last frame landed
/// still shows that frame.
fn end_ms(manifest: &Manifest) -> f64 {
    let last = manifest.frames.last().map_or(0.0, |f| f.t_ms);
    let period = 1000.0 / f64::from(manifest.fps);
    manifest.duration_ms.unwrap_or(0.0).max(last + period)
}

/// One frame of the GIF: which manifest frame, and for how long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Planned {
    /// An index into the manifest's frames.
    pub index: usize,
    /// How long it is shown, in hundredths of a second.
    pub delay_cs: u16,
}

/// What [`plan_gif`] decided.
#[derive(Debug, Clone, PartialEq)]
pub struct GifPlan {
    pub frames: Vec<Planned>,
    /// The manifest frames the GIF leaves out: too close to the one
    /// before (the 20 ms floor, `fps=`) or thinned for `max_frames=`.
    pub dropped: usize,
    /// The spacing between shown frames the plan settled on, in
    /// hundredths of a second: [`GIF_MIN_DELAY_CS`], or more when `fps=`
    /// or `max_frames=` asked for fewer.
    pub spacing_cs: f64,
}

/// Which frames the GIF shows, and for how long each, from the frames'
/// times. `end_ms` is when the last one stops being shown; `spacing_cs`
/// the least time (in hundredths of a second, at least
/// [`GIF_MIN_DELAY_CS`]) between two shown frames; `cap` the most
/// frames to keep.
///
/// Frames are chosen against a clock that ticks every `spacing_cs`
/// ([`select_frames`]), counting from start times rounded to hundredths;
/// each shown frame lasts until the next shown one starts, so the
/// delays add up to the recording's length within a hundredth, and no
/// frame is shown for less than the floor. Past `cap` the spacing is
/// widened — by bisection, to the narrowest that fits — which thins the
/// frames evenly, keeps as many as `cap` allows, and keeps the speed.
///
/// Pure: times in, delays out, which is why the pacing — the part that
/// makes the GIF true to what happened — is tested without a file.
pub fn plan_gif(times_ms: &[f64], end_ms: f64, spacing_cs: f64, cap: usize) -> GifPlan {
    let cap = cap.max(1);
    let floor = GIF_MIN_DELAY_CS as f64;
    let requested = if spacing_cs.is_finite() {
        spacing_cs.max(floor)
    } else {
        floor
    };
    let starts: Vec<u64> = times_ms.iter().map(|t| (t / 10.0).round() as u64).collect();
    let recorded_end_cs = (end_ms / 10.0).round() as u64;

    // The widest spacing that keeps `cap` frames: the requested one when
    // it already does, else the narrowest that does, found by bisection
    // (the frames kept only ever shrink as the spacing grows).
    let mut spacing = requested;
    if select_frames(&starts, spacing).len() > cap {
        let (mut lo, mut hi) = (spacing, starts.last().copied().unwrap_or(0) as f64 + floor);
        for _ in 0..60 {
            let mid = (lo + hi) / 2.0;
            if select_frames(&starts, mid).len() > cap {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        spacing = hi;
    }
    let shown = select_frames(&starts, spacing);
    // The last *shown* frame lasts to the recording's end, and never
    // less than the browser floor: a frame dropped after it must not cut
    // it short, and widening the spacing for `cap` is about the frames
    // between, not how long the last one is held.
    let end_cs = recorded_end_cs.max(shown.last().map_or(0, |&last| starts[last]) + GIF_MIN_DELAY_CS);
    let frames = shown
        .iter()
        .enumerate()
        .map(|(n, &index)| {
            let until = shown.get(n + 1).map_or(end_cs, |&next| starts[next]);
            // At least the floor for every frame, by `select_frames`.
            let delay = until.saturating_sub(starts[index]);
            Planned {
                index,
                delay_cs: u16::try_from(delay).unwrap_or(u16::MAX),
            }
        })
        .collect();
    GifPlan {
        dropped: times_ms.len() - shown.len(),
        frames,
        spacing_cs: spacing,
    }
}

/// Which frames (by index) are shown at a `spacing` of hundredths of a
/// second: those that land on or after a target clock that ticks every
/// `spacing` — fractionally, so a spacing of 3.33 keeps thirty frames a
/// second and not twenty-five — within half a hundredth (starts are
/// rounded), and never closer than the browser floor to the frame shown
/// before. A target that a gap in the recording left behind restarts
/// from the next frame shown.
fn select_frames(starts: &[u64], spacing: f64) -> Vec<usize> {
    let mut shown: Vec<usize> = Vec::new();
    let mut target = 0.0;
    for (i, &start) in starts.iter().enumerate() {
        let far_enough = shown
            .last()
            .is_none_or(|&last| start - starts[last] >= GIF_MIN_DELAY_CS);
        if far_enough && (shown.is_empty() || start as f64 + 0.5 >= target) {
            shown.push(i);
            target += spacing;
            if shown.len() == 1 || target <= start as f64 {
                target = start as f64 + spacing;
            }
        }
    }
    shown
}

/// Decode one frame's PNG to tightly packed RGBA8, checking it is the
/// size the manifest says every frame is.
fn decode_frame(path: &Path, want: (u32, u32)) -> Result<Vec<u8>, String> {
    let shown = path.display();
    let file = File::open(path).map_err(|e| format!("could not open frame `{shown}`: {e}"))?;
    let mut decoder = png::Decoder::new_with_limits(
        BufReader::new(file),
        png::Limits {
            bytes: FRAME_BYTES_CAP,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("frame `{shown}` is not a PNG: {e}"))?;
    let (width, height) = (reader.info().width, reader.info().height);
    if (width, height) != want {
        return Err(format!(
            "frame `{shown}` is {width}×{height}, and the manifest says every frame is {}×{}",
            want.0, want.1
        ));
    }
    let mut pixels = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| format!("frame `{shown}` is too large to decode"))?
    ];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|e| format!("could not decode frame `{shown}`: {e}"))?;
    pixels.truncate(info.buffer_size());
    let rgba = to_rgba(pixels, info.color_type).ok_or_else(|| {
        format!(
            "frame `{shown}` is {:?}, and this reads only RGB, RGBA and grayscale PNGs",
            info.color_type
        )
    })?;
    // What the scaler and the encoder index by: a PNG that decodes short
    // of its own header is refused, not a panic.
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(format!(
            "frame `{shown}` did not decode to {width}×{height} pixels"
        ));
    }
    Ok(rgba)
}

/// `pixels`, 8 bits a channel in `color`'s layout, as RGBA.
fn to_rgba(pixels: Vec<u8>, color: png::ColorType) -> Option<Vec<u8>> {
    match color {
        png::ColorType::Rgba => Some(pixels),
        png::ColorType::Rgb => Some(
            pixels
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
        ),
        png::ColorType::Grayscale => Some(pixels.iter().flat_map(|&g| [g, g, g, 255]).collect()),
        png::ColorType::GrayscaleAlpha => Some(
            pixels
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
        ),
        // `EXPAND` has turned a palette into RGB(A) before it gets here.
        png::ColorType::Indexed => None,
    }
}

/// The height an image `width`×`height` has when scaled to `to_width`,
/// at least 1.
fn scaled_height(width: u32, height: u32, to_width: u32) -> u32 {
    ((f64::from(height) * f64::from(to_width) / f64::from(width)).round() as u32).max(1)
}

/// `rgba` (`width`×`height`) scaled down to `to_width`×`to_height` by
/// averaging the source pixels each target pixel covers. The one pass
/// is O(source pixels); a target as large as the source is the source.
pub fn shrink(rgba: &[u8], width: u32, height: u32, to_width: u32, to_height: u32) -> Vec<u8> {
    if (to_width, to_height) == (width, height) {
        return rgba.to_vec();
    }
    let (width, height) = (width as usize, height as usize);
    let (to_w, to_h) = (to_width as usize, to_height as usize);
    let mut out = Vec::with_capacity(to_w * to_h * 4);
    for ty in 0..to_h {
        let y0 = ty * height / to_h;
        let y1 = ((ty + 1) * height).div_ceil(to_h).clamp(y0 + 1, height);
        for tx in 0..to_w {
            let x0 = tx * width / to_w;
            let x1 = ((tx + 1) * width).div_ceil(to_w).clamp(x0 + 1, width);
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                for pixel in rgba[(y * width + x0) * 4..(y * width + x1) * 4]
                    .as_chunks::<4>()
                    .0
                {
                    for (channel, total) in pixel.iter().zip(&mut sum) {
                        *total += u32::from(*channel);
                    }
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as u32;
            out.extend(sum.iter().map(|total| ((total + count / 2) / count) as u8));
        }
    }
    out
}

/// What was made.
#[derive(Debug, Clone, PartialEq)]
pub struct Assembled {
    pub out: PathBuf,
    pub format: &'static str,
    /// Frames in the recording's manifest, and frames in the result.
    pub frames_in: usize,
    pub frames_out: usize,
    pub width: u32,
    pub height: u32,
    /// How long the result plays, in ms.
    pub duration_ms: f64,
    pub bytes: u64,
    /// The GIF kept fewer frames than the `fps=` asked for because
    /// `max_frames=` would not hold more.
    pub cap_hit: bool,
}

/// What to make, as asked.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub out: Option<PathBuf>,
    pub format: Format,
    /// Scale the frames to this width, keeping their shape.
    pub width: Option<u32>,
    /// Show at most this many frames a second.
    pub fps: Option<u32>,
    pub max_frames: usize,
    pub overwrite: bool,
}

/// The formats assembly makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Gif,
    Mp4,
}

/// The size of the result: `options.width`, or the recording's own.
fn output_size(manifest: &Manifest, options: &Options) -> (u32, u32) {
    match options.width {
        Some(to) if to < manifest.width => (to, scaled_height(manifest.width, manifest.height, to)),
        _ => (manifest.width, manifest.height),
    }
}

/// The spacing `fps=` asks for between shown frames, in hundredths of
/// a second, fractional so the rate is met and not undershot; at least
/// the GIF floor.
fn spacing_for(fps: Option<u32>) -> f64 {
    let floor = GIF_MIN_DELAY_CS as f64;
    fps.map_or(floor, |fps| (100.0 / f64::from(fps.max(1))).max(floor))
}

/// Write `plan`'s frames of the recording in `dir` as an animated GIF
/// into `out`, looping forever; returns the bytes written.
///
/// One frame is in memory at a time: decoded, scaled, quantized and
/// written before the next is read.
fn write_gif(
    dir: &Path,
    manifest: &Manifest,
    plan: &GifPlan,
    size: (u32, u32),
    out: &mut dyn Write,
) -> Result<(), String> {
    let (width, height) = size;
    if width > GIF_MAX_SIDE || height > GIF_MAX_SIDE {
        return Err(format!(
            "a GIF is at most {GIF_MAX_SIDE} pixels on a side; this one would be {width}×{height}. Pass width="
        ));
    }
    let mut encoder = gif::Encoder::new(out, width as u16, height as u16, &[])
        .map_err(|e| format!("could not start the GIF: {e}"))?;
    encoder
        .set_repeat(gif::Repeat::Infinite)
        .map_err(|e| format!("could not start the GIF: {e}"))?;
    for planned in &plan.frames {
        let frame = &manifest.frames[planned.index];
        let rgba = decode_frame(&dir.join(&frame.file), (manifest.width, manifest.height))?;
        let mut rgba = shrink(&rgba, manifest.width, manifest.height, width, height);
        let mut image =
            gif::Frame::from_rgba_speed(width as u16, height as u16, &mut rgba, GIF_QUANTIZER_SPEED);
        image.delay = planned.delay_cs;
        encoder
            .write_frame(&image)
            .map_err(|e| format!("could not write frame `{}` to the GIF: {e}", frame.file))?;
    }
    encoder
        .into_inner()
        .map_err(|e| format!("could not finish the GIF: {e}"))?
        .flush()
        .map_err(|e| format!("could not finish the GIF: {e}"))
}

/// The `ffmpeg` concat-demuxer list for the recording: each frame with
/// how long it is shown, in seconds, from the manifest's times — the
/// last for as long as the recording ran — and the last file named once
/// more with no duration, because the demuxer ignores the final
/// entry's. `dir` is where the files are; a quote in a path is escaped
/// the way the demuxer reads it.
pub fn ffmpeg_list(dir: &Path, manifest: &Manifest) -> String {
    let quoted = |file: &str| {
        let path = dir.join(file).display().to_string().replace('\'', r"'\''");
        format!("file '{path}'\n")
    };
    // Whole milliseconds from the *cumulative* rounded times, so the
    // rounding of one frame is paid back by the next and a recording of
    // thousands of frames does not play short.
    let at = |ms: f64| ms.round() as i64;
    let end = end_ms(manifest);
    let mut list = String::new();
    for (n, frame) in manifest.frames.iter().enumerate() {
        let until = manifest.frames.get(n + 1).map_or(end, |next| next.t_ms);
        list.push_str(&quoted(&frame.file));
        let ms = (at(until) - at(frame.t_ms)).max(1);
        list.push_str(&format!("duration {}.{:03}\n", ms / 1000, ms % 1000));
    }
    if let Some(last) = manifest.frames.last() {
        list.push_str(&quoted(&last.file));
    }
    list
}

/// `ffmpeg`'s arguments: read `list`, write `out` as H.264 in an MP4
/// that any player opens (yuv420p, even sides, the index up front) at
/// `fps`, scaled to `scale_to`'s width when given. `out` is a file of
/// ours (the `.partial` beside the real one), so it is always replaced,
/// and its format is named because its name does not say.
pub fn ffmpeg_args(list: &Path, out: &Path, fps: u32, scale_to: Option<u32>) -> Vec<String> {
    let mut filters = Vec::new();
    if let Some(width) = scale_to {
        filters.push(format!("scale={width}:-2"));
    }
    filters.push("pad=ceil(iw/2)*2:ceil(ih/2)*2".to_string());
    filters.push("format=yuv420p".to_string());
    let mut args: Vec<String> = [
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push(list.display().to_string());
    args.extend(["-vf".to_string(), filters.join(",")]);
    args.extend(["-r".to_string(), fps.to_string()]);
    args.extend(
        ["-c:v", "libx264", "-movflags", "+faststart", "-f", "mp4"]
            .iter()
            .map(|s| s.to_string()),
    );
    args.push(out.display().to_string());
    args
}

/// A new file for `ffmpeg`'s frame list in the temp dir, made
/// exclusively — `O_EXCL` neither follows a symlink nor reuses a name
/// someone planted — under a name that is this process's and this
/// moment's, tried a few times if taken.
fn create_temp_list() -> Result<(PathBuf, File), String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    for attempt in 0..16u32 {
        let path = std::env::temp_dir().join(format!(
            "mandalactl-{}-{nanos}-{attempt}-frames.txt",
            std::process::id()
        ));
        match File::options().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(format!(
                    "could not make ffmpeg's frame list `{}`: {e}",
                    path.display()
                ))
            }
        }
    }
    Err("could not find a free name for ffmpeg's frame list in the temp dir".into())
}

/// Run the host's `ffmpeg` over the recording. Missing is a refusal
/// that names the GIF, which needs nothing; a failure carries what
/// `ffmpeg` said.
fn run_ffmpeg(dir: &Path, manifest: &Manifest, out: &Path, options: &Options) -> Result<(), String> {
    let (list_path, list_file) = create_temp_list()?;
    let list = ffmpeg_list(dir, manifest);
    // Written and closed before ffmpeg reads it.
    std::io::Write::write_all(&mut { list_file }, list.as_bytes()).map_err(|e| {
        let _ = std::fs::remove_file(&list_path);
        format!(
            "could not write ffmpeg's frame list `{}`: {e}",
            list_path.display()
        )
    })?;
    let outcome = Command::new("ffmpeg")
        .args(ffmpeg_args(
            &list_path,
            out,
            options.fps.unwrap_or(manifest.fps),
            options.width.filter(|w| *w < manifest.width),
        ))
        .output();
    let _ = std::fs::remove_file(&list_path);
    match outcome {
        Ok(done) if done.status.success() => Ok(()),
        Ok(done) => Err(format!(
            "ffmpeg failed ({}): {}",
            done.status,
            String::from_utf8_lossy(&done.stderr).trim()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(
            "format=mp4 needs `ffmpeg` on PATH, and it is not there. The GIF (format=gif, the default) needs nothing \
             but this tool"
                .to_string(),
        ),
        Err(e) => Err(format!("could not run ffmpeg: {e}")),
    }
}

/// Assemble the recording in `dir` as `options` says, into `out`.
///
/// The file is made without clobbering: refused when it exists unless
/// `overwrite`; written beside its final name and moved into place, so
/// a failure leaves nothing half-written where a good file was.
pub fn assemble(dir: &Path, manifest: &Manifest, out: &Path, options: &Options) -> Result<Assembled, String> {
    if manifest.frames.is_empty() {
        return Err("the recording holds no frames; there is nothing to assemble".into());
    }
    if !options.overwrite && std::fs::symlink_metadata(out).is_ok() {
        return Err(format!(
            "`{}` exists; pass overwrite=true to replace it",
            out.display()
        ));
    }
    let size = output_size(manifest, options);
    let partial = {
        let mut name = out.as_os_str().to_owned();
        name.push(".partial");
        PathBuf::from(name)
    };
    let discard = |why: String| {
        let _ = std::fs::remove_file(&partial);
        why
    };
    match options.format {
        Format::Gif => {
            let times: Vec<f64> = manifest.frames.iter().map(|f| f.t_ms).collect();
            let plan = plan_gif(
                &times,
                end_ms(manifest),
                spacing_for(options.fps),
                options.max_frames,
            );
            let file = File::options()
                .write(true)
                .create_new(true)
                .open(&partial)
                .map_err(|e| match e.kind() {
                    std::io::ErrorKind::AlreadyExists => format!(
                        "`{}` is in the way: an earlier run was interrupted. Delete it and run again",
                        partial.display()
                    ),
                    _ => format!("could not create `{}`: {e}", partial.display()),
                })?;
            let mut writer = BufWriter::new(file);
            write_gif(dir, manifest, &plan, size, &mut writer).map_err(discard)?;
            let file = writer
                .into_inner()
                .map_err(|e| discard(format!("could not write `{}`: {e}", partial.display())))?;
            file.sync_all()
                .map_err(|e| discard(format!("could not write `{}`: {e}", partial.display())))?;
            std::fs::rename(&partial, out)
                .map_err(|e| discard(format!("could not move the GIF into `{}`: {e}", out.display())))?;
            let shown_cs: u64 = plan.frames.iter().map(|f| u64::from(f.delay_cs)).sum();
            let cap_hit = plan.spacing_cs > spacing_for(options.fps) + 1e-9;
            Ok(Assembled {
                out: out.to_path_buf(),
                format: "gif",
                frames_in: manifest.frames.len(),
                frames_out: plan.frames.len(),
                width: size.0,
                height: size.1,
                duration_ms: shown_cs as f64 * 10.0,
                bytes: std::fs::metadata(out).map_or(0, |m| m.len()),
                cap_hit,
            })
        }
        Format::Mp4 => {
            // `ffmpeg` writes beside the final name, and the file is
            // moved into place only once it has finished: a failure
            // never leaves a truncated video where a good one was.
            run_ffmpeg(dir, manifest, &partial, options).map_err(discard)?;
            std::fs::rename(&partial, out)
                .map_err(|e| discard(format!("could not move the video into `{}`: {e}", out.display())))?;
            Ok(Assembled {
                out: out.to_path_buf(),
                format: "mp4",
                frames_in: manifest.frames.len(),
                frames_out: manifest.frames.len(),
                width: size.0,
                height: size.1,
                duration_ms: end_ms(manifest),
                bytes: std::fs::metadata(out).map_or(0, |m| m.len()),
                cap_hit: false,
            })
        }
    }
}

/// Run `mandalactl record ...`. Exit `0` when the file was made, `1`
/// when the recording could not be assembled (a missing manifest, a
/// frame that will not decode, no `ffmpeg` for an MP4), `3` when the
/// command line was wrong; `2` and `4` are the socket's, and nothing
/// here opens one.
pub fn run(invocation: &Invocation, io: &mut Io<'_>) -> Exit {
    let args = match parse_args(&invocation.words, io.cwd) {
        Ok(args) => args,
        Err(why) => return crate::run::usage(io, &why),
    };
    let manifest_path = args.dir.join(MANIFEST_FILE);
    let manifest = match read_manifest(&manifest_path) {
        Ok(manifest) => manifest,
        Err(why) => return failed(io, &why),
    };
    let out = args.options.out.clone().unwrap_or_else(|| {
        args.dir.join(match args.options.format {
            Format::Gif => "recording.gif",
            Format::Mp4 => "recording.mp4",
        })
    });
    if !manifest.complete {
        crate::run::emit(
            io.err,
            "warning: manifest.json says the recording was not finished (the app exited, or is still recording): \
             assembling the frames that landed\n",
        );
    }
    let made = match assemble(&args.dir, &manifest, &out, &args.options) {
        Ok(made) => made,
        Err(why) => return failed(io, &why),
    };
    for note in notes(&manifest, &made, &args.options) {
        crate::run::emit(io.err, &format!("warning: {note}\n"));
    }
    let result = json!({
        "out": made.out.to_string_lossy(),
        "format": made.format,
        "frames_in": made.frames_in,
        "frames_out": made.frames_out,
        "width": made.width,
        "height": made.height,
        "duration_ms": made.duration_ms,
        "bytes": made.bytes,
    });
    let text = if invocation.options.json {
        format!("{result}\n")
    } else {
        render::prefixed("", &result)
    };
    crate::run::emit(io.out, &text);
    Exit::Success
}

fn failed(io: &mut Io<'_>, why: &str) -> Exit {
    crate::run::emit(io.err, &format!("mandalactl: {why}\n"));
    Exit::CommandError
}

/// What a person should know about what was made, one line each: the
/// recording's own gaps, frames left out, a result too big to show
/// inline.
fn notes(manifest: &Manifest, made: &Assembled, options: &Options) -> Vec<String> {
    let mut notes = Vec::new();
    if manifest.dropped_frames > 0 {
        notes.push(format!(
            "the app could not take {} frame(s) of this recording (see `gaps` in manifest.json); each gap shows \
             the frame before it, as the screen did",
            manifest.dropped_frames
        ));
    }
    if made.format == "gif" && made.frames_out < made.frames_in {
        let mut why = vec![format!(
            "a GIF cannot show frames closer than {} ms apart",
            GIF_MIN_DELAY_CS * 10
        )];
        if let Some(fps) = options.fps {
            why.push(format!("fps={fps} was asked for"));
        }
        if made.cap_hit {
            why.push(format!("max_frames={} would not hold more", options.max_frames));
        }
        notes.push(format!(
            "{} of {} frames were left out ({}); the speed is unchanged",
            made.frames_in - made.frames_out,
            made.frames_in,
            why.join(", ")
        ));
    }
    if made.format == "gif" && made.bytes > GITHUB_INLINE_BYTES {
        notes.push(format!(
            "the GIF is {:.1} MiB, and GitHub shows images inline only up to {} MiB. Make it smaller with width= (a \
             narrower picture), fps= (fewer frames), or record with a smaller width=/scale= in the first place",
            made.bytes as f64 / f64::from(1 << 20),
            GITHUB_INLINE_BYTES >> 20
        ));
    }
    notes
}

/// Read and check `path`, the recording's manifest.
fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let file = File::open(path).map_err(|e| {
        format!(
            "could not read `{}`: {e}. A recording's directory is the one `capture.record_stop` reported",
            path.display()
        )
    })?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(file, MANIFEST_BYTES_CAP + 1), &mut text)
        .map_err(|e| format!("could not read `{}`: {e}", path.display()))?;
    if text.len() as u64 > MANIFEST_BYTES_CAP {
        return Err(format!(
            "`{}` is larger than {} MiB, which is more than any recording's manifest",
            path.display(),
            MANIFEST_BYTES_CAP >> 20
        ));
    }
    parse_manifest(&text)
}

/// The command line after `record`, read.
#[derive(Debug, PartialEq)]
pub(crate) struct Args {
    pub(crate) dir: PathBuf,
    pub(crate) options: Options,
}

pub(crate) fn parse_args(words: &[String], cwd: Option<&Path>) -> Result<Args, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut out = None;
    let mut format = Format::Gif;
    let mut width = None;
    let mut fps = None;
    let mut max_frames = DEFAULT_MAX_FRAMES;
    let mut overwrite = false;
    let whole = |name: &str, value: &str, range: std::ops::RangeInclusive<u64>| -> Result<u64, String> {
        value
            .parse::<u64>()
            .ok()
            .filter(|n| range.contains(n))
            .ok_or_else(|| {
                format!(
                    "`{name}` must be a whole number in {}..={}, got `{value}`",
                    range.start(),
                    range.end()
                )
            })
    };
    for token in tokenize(words) {
        match token {
            Token::Positional(word) => positional.push(word),
            Token::Json { name, .. } => {
                return Err(format!(
                    "`{name}:=` sends JSON to the app, and `record {SUBCOMMAND}` does not talk to it; use `{name}=value`"
                ))
            }
            Token::Named { name, value } => match name.as_str() {
                "out" => out = Some(value),
                "format" => {
                    format = match value.as_str() {
                        "gif" => Format::Gif,
                        "mp4" => Format::Mp4,
                        other => return Err(format!("`format` is `gif` or `mp4`, got `{other}`")),
                    }
                }
                "width" => width = Some(whole("width", &value, 1..=u64::from(GIF_MAX_SIDE))? as u32),
                "fps" => fps = Some(whole("fps", &value, 1..=60)? as u32),
                "max_frames" => max_frames = whole("max_frames", &value, 1..=MAX_FRAMES as u64)? as usize,
                "overwrite" => {
                    overwrite = match value.as_str() {
                        "true" => true,
                        "false" => false,
                        other => return Err(format!("`overwrite` is `true` or `false`, got `{other}`")),
                    }
                }
                other => {
                    return Err(format!(
                        "`record {SUBCOMMAND}` has no parameter `{other}`; it takes out, format, width, fps, \
                         max_frames and overwrite"
                    ))
                }
            },
        }
    }
    let mut positional = positional.into_iter();
    match positional.next().as_deref() {
        Some(SUBCOMMAND) => {}
        Some(other) => {
            return Err(format!(
                "`record` has one subcommand, `{SUBCOMMAND}` (`mandalactl record {SUBCOMMAND} <dir>`); got `{other}`. \
                 Starting and stopping a recording are `capture.record_start` and `capture.record_stop`"
            ))
        }
        None => {
            return Err(format!(
                "`record` needs a subcommand: `mandalactl record {SUBCOMMAND} <dir>`"
            ))
        }
    }
    let dir = positional.next().ok_or_else(|| {
        format!("`record {SUBCOMMAND}` needs the recording's directory: `record {SUBCOMMAND} <dir>`")
    })?;
    if let Some(extra) = positional.next() {
        return Err(format!(
            "`record {SUBCOMMAND}` takes one directory; `{extra}` is a second"
        ));
    }
    let resolve = |path: &str| -> Result<PathBuf, String> {
        let path = Path::new(path);
        if path.is_absolute() {
            return Ok(path.to_path_buf());
        }
        cwd.map(|cwd| cwd.join(path)).ok_or_else(|| {
            format!(
                "`{}` is relative and the working directory could not be read; pass an absolute path",
                path.display()
            )
        })
    };
    Ok(Args {
        dir: resolve(&dir)?,
        options: Options {
            out: out.as_deref().map(resolve).transpose()?,
            format,
            width,
            fps,
            max_frames,
            overwrite,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifest `format/ipc.md` shows, with `frames` filled in.
    fn manifest_text(frames: &str) -> String {
        format!(
            r#"{{"protocol":1,"fps":30,"pacing":"realtime","width":8,"height":6,"scale":1.0,
                "complete":true,"stopped":"client","duration_ms":400.0,"frames":[{frames}],
                "dropped_frames":2,"gaps":[{{"t_ms":50.0,"frames":2,"reason":"overrun"}}]}}"#
        )
    }

    fn frame(t_ms: f64, file: &str) -> String {
        format!(r#"{{"index":0,"t_ms":{t_ms},"file":"{file}","document_revision":1,"camera":{{}}}}"#)
    }

    /// `n` frame times at `fps`, as a recording at that rate takes them.
    fn evenly(n: usize, fps: u32) -> Vec<f64> {
        (0..n).map(|i| i as f64 * 1000.0 / f64::from(fps)).collect()
    }

    /// The browser floor as a spacing.
    const FLOOR: f64 = GIF_MIN_DELAY_CS as f64;

    fn total_cs(plan: &GifPlan) -> u64 {
        plan.frames.iter().map(|f| u64::from(f.delay_cs)).sum()
    }

    #[test]
    fn test_a_manifest_is_read_for_what_assembly_relies_on() {
        let text = manifest_text(&format!(
            "{},{}",
            frame(0.0, "frame_00000.png"),
            frame(100.5, "frame_00001.png")
        ));
        let manifest = parse_manifest(&text).expect("a manifest");
        assert_eq!(
            manifest,
            Manifest {
                fps: 30,
                width: 8,
                height: 6,
                frames: vec![
                    FrameRef {
                        t_ms: 0.0,
                        file: "frame_00000.png".into()
                    },
                    FrameRef {
                        t_ms: 100.5,
                        file: "frame_00001.png".into()
                    },
                ],
                duration_ms: Some(400.0),
                complete: true,
                dropped_frames: 2,
            }
        );
        // One the app never finished has no `duration_ms` or `complete`.
        let unfinished = parse_manifest(
            r#"{"protocol":1,"fps":10,"width":2,"height":2,"frames":[{"t_ms":0,"file":"a.png"}]}"#,
        )
        .expect("readable");
        assert_eq!((unfinished.complete, unfinished.duration_ms), (false, None));
    }

    /// A manifest is data from a directory: whatever else is wrong with
    /// it is a refusal naming the thing, and a frame's `file` is never a
    /// path out of the directory.
    #[test]
    fn test_a_manifest_that_cannot_be_trusted_is_refused() {
        let ok = frame(0.0, "frame_00000.png");
        for (text, why) in [
            ("not json".to_string(), "not JSON"),
            ("[]".to_string(), "not a JSON object"),
            (
                manifest_text(&ok).replace("\"protocol\":1", "\"protocol\":2"),
                "protocol 2",
            ),
            (
                manifest_text(&ok).replace("\"protocol\":1,", ""),
                "protocol missing",
            ),
            (manifest_text(&ok).replace("\"fps\":30", "\"fps\":0"), "`fps`"),
            (
                manifest_text(&ok).replace("\"width\":8", "\"width\":-3"),
                "`width`",
            ),
            (
                manifest_text(&ok).replace("\"frames\"", "\"shots\""),
                "no `frames`",
            ),
            (manifest_text(&frame(-1.0, "a.png")), "`t_ms`"),
            (
                manifest_text(&format!("{},{}", frame(50.0, "a.png"), frame(10.0, "b.png"))),
                "earlier",
            ),
            (manifest_text(&frame(0.0, "../frame.png")), "plain file name"),
            (manifest_text(&frame(0.0, "/etc/passwd")), "plain file name"),
            (manifest_text(&frame(0.0, "sub/frame.png")), "plain file name"),
            (manifest_text(&frame(0.0, ".hidden.png")), "plain file name"),
            (manifest_text(&frame(0.0, "")), "plain file name"),
        ] {
            let refused = parse_manifest(&text);
            assert!(
                matches!(&refused, Err(message) if message.contains(why)),
                "{why}: {refused:?}"
            );
        }
    }

    /// Thirty frames a second are shown 3 or 4 hundredths each, and the
    /// delays add up to the recording's length, to a hundredth: the
    /// rounding does not accumulate into a GIF that runs fast or slow.
    #[test]
    fn test_thirty_fps_keeps_its_length_without_rounding_drift() {
        let times = evenly(90, 30);
        let plan = plan_gif(&times, 3000.0, FLOOR, 600);
        assert_eq!(plan.frames.len(), 90);
        assert_eq!(plan.dropped, 0);
        assert!(
            plan.frames.iter().all(|f| (3..=4).contains(&f.delay_cs)),
            "{plan:?}"
        );
        assert_eq!(total_cs(&plan), 300, "three seconds, exactly");
        // Rounded one by one, 33.3 ms is 3 cs ninety times: 270, nine
        // tenths of the time. That is the drift this avoids.
    }

    /// A browser shows any GIF frame under 20 ms as 100 ms, so sixty
    /// frames a second cannot be kept: every other one is dropped, none
    /// is shown for less than two hundredths, the length holds, and the
    /// plan says what it dropped.
    #[test]
    fn test_sixty_fps_is_thinned_to_what_a_browser_plays_at_speed() {
        let times = evenly(120, 60);
        let plan = plan_gif(&times, 2000.0, FLOOR, 600);
        assert!(plan.frames.iter().all(|f| f.delay_cs >= 2), "{plan:?}");
        assert_eq!(plan.frames.len() + plan.dropped, 120);
        // 16.7 ms apart is 1.67 hundredths: every frame whose start is
        // two or more past the last shown stays, about two in three.
        assert!((75..=85).contains(&plan.frames.len()), "{}", plan.frames.len());
        assert_eq!(total_cs(&plan), 200);
    }

    /// A stretch the recording missed holds the frame before it for as
    /// long as it lasted, and the last frame lasts until the recording
    /// ended.
    #[test]
    fn test_a_gap_holds_the_frame_before_it_and_the_last_frame_runs_to_the_end() {
        let plan = plan_gif(&[0.0, 50.0, 100.0, 500.0, 550.0], 600.0, FLOOR, 600);
        assert_eq!(
            plan.frames.iter().map(|f| f.delay_cs).collect::<Vec<_>>(),
            [5, 5, 40, 5, 5]
        );
        // A recording stopped the instant its last frame landed still
        // shows that frame, for one period.
        let plan = plan_gif(&[0.0, 100.0], 100.0, FLOOR, 600);
        assert_eq!(plan.frames[1].delay_cs, 2, "never less than the floor");
        let one = plan_gif(&[0.0], 0.0, FLOOR, 600);
        assert_eq!(
            one.frames,
            vec![Planned {
                index: 0,
                delay_cs: 2
            }]
        );
    }

    /// `fps=` is a ceiling on the rate that is *met*: ten a second is a
    /// frame every ten hundredths, and thirty a second keeps a thirty
    /// frames-a-second recording whole — not the twenty-five a spacing
    /// rounded up to whole hundredths would leave — at the recording's
    /// speed.
    #[test]
    fn test_an_fps_ceiling_thins_to_the_rate_asked_without_changing_the_speed() {
        assert_eq!(spacing_for(None), 2.0);
        assert_eq!(spacing_for(Some(10)), 10.0);
        assert!((spacing_for(Some(30)) - 100.0 / 30.0).abs() < 1e-12);
        assert_eq!(spacing_for(Some(60)), 2.0, "the browser floor still holds");
        let plan = plan_gif(&evenly(60, 30), 2000.0, spacing_for(Some(10)), 600);
        assert_eq!(plan.frames.len(), 20);
        assert_eq!(plan.spacing_cs, 10.0);
        assert!(plan.frames.iter().all(|f| f.delay_cs >= 10));
        assert_eq!(total_cs(&plan), 200);

        // A recording at the rate asked for loses nothing.
        let whole = plan_gif(&evenly(600, 30), 20_000.0, spacing_for(Some(30)), 600);
        assert_eq!(whole.frames.len(), 600, "{}", whole.dropped);
        let slower = plan_gif(&evenly(600, 30), 20_000.0, spacing_for(Some(25)), 600);
        assert!(
            (490..=510).contains(&slower.frames.len()),
            "25 of every 30: {}",
            slower.frames.len()
        );
        assert_eq!(total_cs(&slower), 2000);
    }

    /// Past `max_frames` the frames are thinned evenly, not cut off at
    /// the end: the GIF still covers the whole recording at the speed
    /// it happened.
    #[test]
    fn test_more_frames_than_the_cap_are_thinned_evenly_not_truncated() {
        let times = evenly(1000, 30);
        let plan = plan_gif(&times, 33_400.0, FLOOR, 100);
        assert!(plan.frames.len() <= 100, "{}", plan.frames.len());
        assert!(
            plan.frames.len() >= 95,
            "as many as the cap allows: {}",
            plan.frames.len()
        );
        assert_eq!(plan.frames.len() + plan.dropped, 1000);
        assert_eq!(total_cs(&plan), 3340, "the whole recording");
        let last = plan.frames.last().expect("frames").index;
        assert!(last > 900, "the end of the recording is in it: {last}");
        assert!(plan.spacing_cs > FLOOR);
        // One frame over the cap thins a few frames, not a third of them
        // (a clock cannot drop exactly one, but it drops about one in
        // fifty).
        let barely = plan_gif(&evenly(601, 30), 20_100.0, FLOOR, 600);
        assert!(
            (580..=600).contains(&barely.frames.len()),
            "{}",
            barely.frames.len()
        );
    }

    #[test]
    fn test_shrinking_averages_what_each_target_pixel_covers() {
        // 4×2 → 2×1: each target is the mean of a 2×2 block.
        let rgba = [
            0, 0, 0, 255, 100, 0, 0, 255, 200, 200, 0, 255, 0, 0, 0, 255, //
            0, 0, 0, 255, 100, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255,
        ];
        assert_eq!(shrink(&rgba, 4, 2, 2, 1), [50, 0, 0, 255, 50, 50, 0, 255]);
        assert_eq!(
            shrink(&rgba, 4, 2, 4, 2),
            rgba,
            "the same size is the same pixels"
        );
        // A ratio that does not divide still covers every target pixel
        // and reads only inside the source.
        let odd = vec![200u8; 7 * 5 * 4];
        let small = shrink(&odd, 7, 5, 3, 2);
        assert_eq!(small.len(), 3 * 2 * 4);
        assert!(small.iter().all(|&b| b == 200));
        assert_eq!(scaled_height(1280, 720, 640), 360);
        assert_eq!(scaled_height(1000, 1, 10), 1, "never zero");
    }

    // — the files —

    /// A recording in a fresh directory: `colors` is each frame's flat
    /// color, `times` its time; every frame is 8×6.
    fn recording(
        label: &str,
        colors: &[[u8; 3]],
        times: &[f64],
        duration_ms: f64,
    ) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::Builder::new()
            .prefix(label)
            .tempdir()
            .expect("a scratch directory");
        let dir = temp.path().to_path_buf();
        let mut listed = Vec::new();
        for (n, (color, t_ms)) in colors.iter().zip(times).enumerate() {
            let name = format!("frame_{n:05}.png");
            let mut bytes = Vec::new();
            let mut encoder = png::Encoder::new(&mut bytes, 8, 6);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            let pixels: Vec<u8> = (0..8 * 6)
                .flat_map(|_| [color[0], color[1], color[2], 255])
                .collect();
            writer.write_image_data(&pixels).expect("pixels");
            writer.finish().expect("finish");
            std::fs::write(dir.join(&name), bytes).expect("frame");
            listed.push(frame(*t_ms, &name));
        }
        let manifest = manifest_text(&listed.join(",")).replace("400.0", &format!("{duration_ms}"));
        std::fs::write(dir.join(MANIFEST_FILE), manifest).expect("manifest");
        (temp, dir)
    }

    fn options() -> Options {
        Options {
            out: None,
            format: Format::Gif,
            width: None,
            fps: None,
            max_frames: DEFAULT_MAX_FRAMES,
            overwrite: false,
        }
    }

    /// Every frame of `gif`: `(delay in cs, width, height, first RGBA pixel)`.
    fn read_gif(path: &Path) -> Vec<(u16, u16, u16, [u8; 4])> {
        let mut decode = gif::DecodeOptions::new();
        decode.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = decode
            .read_info(File::open(path).expect("the GIF"))
            .expect("a GIF");
        let mut frames = Vec::new();
        while let Some(frame) = decoder.read_next_frame().expect("a frame") {
            frames.push((
                frame.delay,
                frame.width,
                frame.height,
                [frame.buffer[0], frame.buffer[1], frame.buffer[2], frame.buffer[3]],
            ));
        }
        frames
    }

    fn near(got: [u8; 4], want: [u8; 3]) -> bool {
        got[3] == 255 && got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 8)
    }

    /// The pictures, in order, at the pacing the manifest gives: 0 ms,
    /// 100 ms and 250 ms into a 400 ms recording are shown 10, 15 and 15
    /// hundredths; the GIF loops, and its size is the recording's.
    #[test]
    fn test_a_recording_becomes_a_gif_at_the_pace_it_was_recorded() {
        let colors = [[255, 0, 0], [0, 255, 0], [0, 0, 255]];
        let (_keep, dir) = recording("assemble-gif", &colors, &[0.0, 100.0, 250.0], 400.0);
        let manifest = read_manifest(&dir.join(MANIFEST_FILE)).expect("manifest");
        let out = dir.join("out.gif");
        let made = assemble(&dir, &manifest, &out, &options()).expect("assembled");
        assert_eq!(
            made,
            Assembled {
                out: out.clone(),
                format: "gif",
                frames_in: 3,
                frames_out: 3,
                width: 8,
                height: 6,
                duration_ms: 400.0,
                bytes: std::fs::metadata(&out).expect("written").len(),
                cap_hit: false,
            }
        );
        let frames = read_gif(&out);
        assert_eq!(
            frames.iter().map(|f| (f.0, f.1, f.2)).collect::<Vec<_>>(),
            [(10, 8, 6), (15, 8, 6), (15, 8, 6)]
        );
        for (got, want) in frames.iter().zip(colors) {
            assert!(near(got.3, want), "{:?} is not {want:?}", got.3);
        }
        let bytes = std::fs::read(&out).expect("read");
        assert!(
            bytes.windows(11).any(|w| w == b"NETSCAPE2.0"),
            "it loops: the Netscape extension is there"
        );
        assert!(
            !dir.join("out.gif.partial").exists(),
            "nothing half-written is left"
        );
    }

    #[test]
    fn test_width_scales_the_frames_keeping_their_shape() {
        let (_keep, dir) = recording("assemble-width", &[[10, 20, 30]], &[0.0], 100.0);
        let manifest = read_manifest(&dir.join(MANIFEST_FILE)).expect("manifest");
        let made = assemble(
            &dir,
            &manifest,
            &dir.join("small.gif"),
            &Options {
                width: Some(4),
                ..options()
            },
        )
        .expect("assembled");
        assert_eq!((made.width, made.height), (4, 3));
        let frames = read_gif(&dir.join("small.gif"));
        assert_eq!((frames[0].1, frames[0].2), (4, 3));
        assert!(near(frames[0].3, [10, 20, 30]));
        // A width at or above the recording's is not an enlargement.
        let same = assemble(
            &dir,
            &manifest,
            &dir.join("same.gif"),
            &Options {
                width: Some(80),
                ..options()
            },
        )
        .expect("assembled");
        assert_eq!((same.width, same.height), (8, 6));
    }

    /// What goes wrong goes wrong whole and says what: nothing is
    /// clobbered, a bad frame is named, and no partial file is left.
    #[test]
    fn test_refusals_name_the_problem_and_leave_nothing_behind() {
        let (_keep, dir) = recording("assemble-refuse", &[[1, 2, 3], [4, 5, 6]], &[0.0, 100.0], 300.0);
        let manifest = read_manifest(&dir.join(MANIFEST_FILE)).expect("manifest");

        let precious = dir.join("precious.gif");
        std::fs::write(&precious, "precious").expect("seed");
        let refused = assemble(&dir, &manifest, &precious, &options());
        assert!(
            matches!(&refused, Err(m) if m.contains("overwrite=true")),
            "{refused:?}"
        );
        assert_eq!(std::fs::read_to_string(&precious).expect("read"), "precious");
        let replaced = assemble(
            &dir,
            &manifest,
            &precious,
            &Options {
                overwrite: true,
                ..options()
            },
        );
        assert!(replaced.is_ok(), "{replaced:?}");
        assert_eq!(read_gif(&precious).len(), 2);

        // A frame missing, and one of the wrong size, are named; neither
        // leaves a file.
        std::fs::remove_file(dir.join("frame_00001.png")).expect("remove");
        let missing = assemble(&dir, &manifest, &dir.join("a.gif"), &options());
        assert!(
            matches!(&missing, Err(m) if m.contains("frame_00001.png")),
            "{missing:?}"
        );
        let mut wrong = manifest.clone();
        wrong.width = 9;
        let sized = assemble(&dir, &wrong, &dir.join("b.gif"), &options());
        assert!(
            matches!(&sized, Err(m) if m.contains("8×6") && m.contains("9×6")),
            "{sized:?}"
        );
        assert!(!dir.join("a.gif").exists() && !dir.join("b.gif").exists());
        assert!(!dir.join("a.gif.partial").exists() && !dir.join("b.gif.partial").exists());

        let empty = Manifest {
            frames: Vec::new(),
            ..manifest
        };
        assert!(
            matches!(assemble(&dir, &empty, &dir.join("c.gif"), &options()), Err(m) if m.contains("no frames"))
        );
    }

    #[test]
    fn test_the_ffmpeg_list_carries_each_frames_time_and_repeats_the_last() {
        let manifest = parse_manifest(&manifest_text(&format!(
            "{},{}",
            frame(0.0, "frame_00000.png"),
            frame(250.0, "it's.png")
        )))
        .expect("manifest");
        assert_eq!(
            ffmpeg_list(Path::new("/rec"), &manifest),
            "file '/rec/frame_00000.png'\nduration 0.250\n\
             file '/rec/it'\\''s.png'\nduration 0.150\n\
             file '/rec/it'\\''s.png'\n"
        );
    }

    /// Durations are cut from cumulative rounded times, so 33.333 ms
    /// frames do not each lose a third of a millisecond: six hundred of
    /// them still add up to twenty seconds.
    #[test]
    fn test_the_ffmpeg_durations_do_not_drift() {
        let frames: Vec<String> = evenly(600, 30)
            .iter()
            .enumerate()
            .map(|(n, t)| frame(*t, &format!("f{n}.png")))
            .collect();
        let text = manifest_text(&frames.join(",")).replace("400.0", "20000.0");
        let manifest = parse_manifest(&text).expect("manifest");
        let total_ms: u64 = ffmpeg_list(Path::new("/r"), &manifest)
            .lines()
            .filter_map(|line| line.strip_prefix("duration "))
            .map(|d| (d.parse::<f64>().expect("seconds") * 1000.0).round() as u64)
            .sum();
        assert_eq!(total_ms, 20_000);
    }

    #[test]
    fn test_the_ffmpeg_arguments_make_an_mp4_any_player_opens() {
        let args = ffmpeg_args(Path::new("/l.txt"), Path::new("/o.mp4.partial"), 30, Some(640));
        assert_eq!(
            args,
            [
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                "/l.txt",
                "-vf",
                "scale=640:-2,pad=ceil(iw/2)*2:ceil(ih/2)*2,format=yuv420p",
                "-r",
                "30",
                "-c:v",
                "libx264",
                "-movflags",
                "+faststart",
                "-f",
                "mp4",
                "/o.mp4.partial"
            ]
        );
        let unscaled = ffmpeg_args(Path::new("/l.txt"), Path::new("/o"), 12, None);
        assert!(unscaled.contains(&"pad=ceil(iw/2)*2:ceil(ih/2)*2,format=yuv420p".to_string()));
    }

    /// The frame list is made exclusively: a name someone planted — as
    /// a symlink to a file of yours, say — is never written through.
    #[test]
    #[cfg(unix)]
    fn test_the_ffmpeg_list_is_never_written_through_a_planted_name() {
        let (path, mut file) = create_temp_list().expect("a list");
        file.write_all(b"x").expect("written");
        assert!(path.starts_with(std::env::temp_dir()));
        let again = create_temp_list().expect("another");
        assert_ne!(path, again.0, "each is a name of its own");
        let target = tempfile::NamedTempFile::new().expect("a victim");
        std::fs::write(target.path(), "precious").expect("seed");
        let planted = std::env::temp_dir().join("mandalactl-planted-frames.txt");
        let _ = std::fs::remove_file(&planted);
        std::os::unix::fs::symlink(target.path(), &planted).expect("plant");
        let refused = File::options().write(true).create_new(true).open(&planted);
        assert!(refused.is_err(), "O_EXCL refuses a symlink, dangling or not");
        assert_eq!(std::fs::read_to_string(target.path()).expect("read"), "precious");
        for p in [&path, &again.0, &planted] {
            let _ = std::fs::remove_file(p);
        }
    }

    /// With a real `ffmpeg` on the host the MP4 is made and is a video;
    /// without one the test says so and has nothing to check (the
    /// refusal path is tested below with a name that is certainly not
    /// there).
    #[test]
    fn test_an_mp4_is_made_when_ffmpeg_is_on_the_host() {
        let present = Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success());
        if !present {
            eprintln!("skipped: no `ffmpeg` on PATH (format=mp4 is the host's)");
            return;
        }
        let (_keep, dir) = recording("assemble-mp4", &[[255, 0, 0], [0, 255, 0]], &[0.0, 500.0], 1000.0);
        let manifest = read_manifest(&dir.join(MANIFEST_FILE)).expect("manifest");
        let out = dir.join("out.mp4");
        let made = assemble(
            &dir,
            &manifest,
            &out,
            &Options {
                format: Format::Mp4,
                ..options()
            },
        )
        .expect("assembled");
        assert_eq!((made.format, made.frames_out), ("mp4", 2));
        let bytes = std::fs::read(&out).expect("written");
        assert!(bytes.windows(4).any(|w| w == b"ftyp"), "an MP4 container");
        assert!(made.bytes > 0);
    }

    // — the command line —

    fn words(line: &[&str]) -> Vec<String> {
        line.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn test_the_command_line_is_read_and_relative_paths_meet_the_working_directory() {
        let cwd = Path::new("/work");
        let args = parse_args(
            &words(&[
                "assemble",
                "rec",
                "out=shots/a.gif",
                "width=640",
                "fps=12",
                "max_frames=300",
                "overwrite=true",
            ]),
            Some(cwd),
        )
        .expect("parsed");
        assert_eq!(
            args,
            Args {
                dir: PathBuf::from("/work/rec"),
                options: Options {
                    out: Some(PathBuf::from("/work/shots/a.gif")),
                    format: Format::Gif,
                    width: Some(640),
                    fps: Some(12),
                    max_frames: 300,
                    overwrite: true,
                }
            }
        );
        let video =
            parse_args(&words(&["assemble", "/abs/rec", "format=mp4"]), None).expect("absolute needs no cwd");
        assert_eq!(
            (video.dir, video.options.format),
            (PathBuf::from("/abs/rec"), Format::Mp4)
        );
        let defaults = parse_args(&words(&["assemble", "/abs/rec"]), None)
            .expect("parsed")
            .options;
        assert_eq!(
            (
                defaults.max_frames,
                defaults.overwrite,
                defaults.width,
                defaults.fps
            ),
            (DEFAULT_MAX_FRAMES, false, None, None)
        );
    }

    #[test]
    fn test_a_malformed_command_line_is_refused_naming_the_mistake() {
        for (line, why) in [
            (&[][..], "needs a subcommand"),
            (&["start", "/d"][..], "one subcommand"),
            (&["assemble"][..], "needs the recording's directory"),
            (&["assemble", "/a", "/b"][..], "second"),
            (&["assemble", "/a", "format=avi"][..], "gif` or `mp4"),
            (&["assemble", "/a", "width=0"][..], "`width`"),
            (&["assemble", "/a", "fps=61"][..], "`fps`"),
            (&["assemble", "/a", "fps=fast"][..], "`fps`"),
            (&["assemble", "/a", "max_frames=3601"][..], "`max_frames`"),
            (&["assemble", "/a", "overwrite=yes"][..], "`true` or `false`"),
            (&["assemble", "/a", "color=red"][..], "no parameter `color`"),
            (&["assemble", "/a", "out:=\"x\""][..], "does not talk to it"),
        ] {
            let refused = parse_args(&words(line), Some(Path::new("/w")));
            assert!(
                matches!(&refused, Err(m) if m.contains(why)),
                "{line:?}: {refused:?}"
            );
        }
        let no_cwd = parse_args(&words(&["assemble", "rec"]), None);
        assert!(
            matches!(&no_cwd, Err(m) if m.contains("working directory")),
            "{no_cwd:?}"
        );
    }

    fn run_line(line: &[&str], cwd: &Path, json: bool) -> (Exit, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut stdin = std::io::empty();
        let invocation = Invocation {
            options: crate::cli::Options {
                json,
                ..crate::cli::Options::default()
            },
            command: COMMAND.to_string(),
            words: words(line),
        };
        let exit = run(
            &invocation,
            &mut Io {
                out: &mut out,
                err: &mut err,
                cwd: Some(cwd),
                stdin: &mut stdin,
            },
        );
        (
            exit,
            String::from_utf8(out).expect("UTF-8"),
            String::from_utf8(err).expect("UTF-8"),
        )
    }

    /// The whole command, as a shell sees it: a relative directory, the
    /// result as `path: value` lines or as one JSON object, the default
    /// file beside the frames, and the exit codes — `0` made, `1` could
    /// not, `3` misspoken.
    #[test]
    fn test_the_command_reports_what_it_made_and_exits_by_what_happened() {
        let (keep, dir) = recording("assemble-run", &[[9, 9, 9], [200, 100, 50]], &[0.0, 100.0], 300.0);
        let cwd = keep.path().parent().expect("a parent").to_path_buf();
        let relative = dir.file_name().expect("named").to_string_lossy().into_owned();

        let (exit, out, err) = run_line(&["assemble", &relative], &cwd, false);
        assert_eq!(exit, Exit::Success, "{err}");
        assert!(
            out.contains(&format!("out: \"{}\"", dir.join("recording.gif").display())),
            "{out}"
        );
        assert!(
            out.contains("frames_out: 2") && out.contains("format: \"gif\""),
            "{out}"
        );
        assert!(
            err.contains("the app could not take 2 frame(s)"),
            "the recording's own gaps are said: {err}"
        );
        assert_eq!(read_gif(&dir.join("recording.gif")).len(), 2);

        let (exit, out, _) = run_line(&["assemble", &relative, "out=again.gif"], &cwd, true);
        assert_eq!(exit, Exit::Success);
        let json: Value = serde_json::from_str(out.trim()).expect("one JSON object");
        assert_eq!(json["frames_in"], 2);
        assert_eq!(json["out"], json!(cwd.join("again.gif").to_string_lossy()));
        std::fs::remove_file(cwd.join("again.gif")).expect("cleanup");

        let (exit, _, err) = run_line(&["assemble", &relative], &cwd, false);
        assert_eq!(exit, Exit::CommandError, "an existing file is not clobbered");
        assert!(err.contains("overwrite=true"), "{err}");

        let (exit, _, err) = run_line(&["assemble", "no-such-recording"], &cwd, false);
        assert_eq!(exit, Exit::CommandError);
        assert!(err.contains("manifest.json"), "{err}");

        let (exit, _, err) = run_line(&["assemble"], &cwd, false);
        assert_eq!(exit, Exit::UsageError);
        assert!(err.contains("Try `mandalactl --help`"), "{err}");
    }

    /// A recording the app never finished is assembled from the frames
    /// that landed, with a warning that says so.
    #[test]
    fn test_an_unfinished_recording_is_assembled_with_a_warning() {
        let (keep, dir) = recording("assemble-unfinished", &[[9, 9, 9]], &[0.0], 100.0);
        let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).expect("manifest");
        std::fs::write(
            dir.join(MANIFEST_FILE),
            text.replace("\"complete\":true", "\"complete\":false"),
        )
        .expect("rewrite");
        let (exit, _, err) = run_line(&["assemble", &dir.display().to_string()], keep.path(), false);
        assert_eq!(exit, Exit::Success, "{err}");
        assert!(err.contains("not finished"), "{err}");
    }

    /// The notes say what a person needs to hear and nothing else: a
    /// clean, small, complete result says nothing.
    #[test]
    fn test_notes_say_what_was_lost_and_what_would_help() {
        let manifest = parse_manifest(
            &manifest_text(&frame(0.0, "a.png")).replace("\"dropped_frames\":2", "\"dropped_frames\":0"),
        )
        .expect("manifest");
        let clean = Assembled {
            out: PathBuf::from("/o.gif"),
            format: "gif",
            frames_in: 10,
            frames_out: 10,
            width: 8,
            height: 6,
            duration_ms: 100.0,
            bytes: 1000,
            cap_hit: false,
        };
        assert!(notes(&manifest, &clean, &options()).is_empty());
        let big = Assembled {
            bytes: GITHUB_INLINE_BYTES + 1,
            frames_out: 6,
            ..clean
        };
        let notes = notes(
            &manifest,
            &big,
            &Options {
                fps: Some(5),
                ..options()
            },
        );
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(
            notes[0].contains("4 of 10 frames")
                && notes[0].contains("fps=5")
                && notes[0].contains("speed is unchanged")
        );
        assert!(notes[1].contains("10 MiB") && notes[1].contains("width="));
        // When the cap did the thinning, the note says the cap did.
        let capped = Assembled {
            frames_out: 6,
            cap_hit: true,
            ..clean_for_cap()
        };
        let said = super::notes(
            &manifest,
            &capped,
            &Options {
                max_frames: 6,
                ..options()
            },
        );
        assert!(said[0].contains("max_frames=6 would not hold more"), "{said:?}");
        assert!(!said[0].contains("fps="), "{said:?}");
    }

    fn clean_for_cap() -> Assembled {
        Assembled {
            out: PathBuf::from("/o.gif"),
            format: "gif",
            frames_in: 10,
            frames_out: 10,
            width: 8,
            height: 6,
            duration_ms: 100.0,
            bytes: 1000,
            cap_hit: false,
        }
    }

    /// A manifest's `file` that holds a control character is refused: a
    /// newline would split a line of ffmpeg's frame list.
    #[test]
    fn test_a_file_name_with_a_control_character_is_refused() {
        let refused = parse_manifest(&manifest_text(&frame(0.0, "a\\nfile 'b'.png")));
        assert!(
            matches!(&refused, Err(m) if m.contains("plain file name")),
            "{refused:?}"
        );
    }

    /// A stale `.partial` from an interrupted run is named in the
    /// refusal, so the way out is in the message.
    #[test]
    fn test_a_leftover_partial_file_is_named_in_the_refusal() {
        let (_keep, dir) = recording("assemble-partial", &[[1, 2, 3]], &[0.0], 100.0);
        let manifest = read_manifest(&dir.join(MANIFEST_FILE)).expect("manifest");
        std::fs::write(dir.join("x.gif.partial"), "stale").expect("seed");
        let refused = assemble(&dir, &manifest, &dir.join("x.gif"), &options());
        assert!(
            matches!(&refused, Err(m) if m.contains("x.gif.partial") && m.contains("interrupted")),
            "{refused:?}"
        );
    }
}
