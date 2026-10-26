// SPDX-License-Identifier: MPL-2.0

//! The `capture` family: what the app draws, as pixels
//! (`format/ipc.md` §`capture`).
//!
//! **A thin surface over one engine.** `capture.screenshot` checks the
//! shape of its request, hands it to the screenshot engine through
//! `CommandTarget::capture` (native; `capture_canvas` on the browser)
//! — the function the `screenshot` console verb calls too — and words
//! what came back. Planning the camera, drawing offscreen, writing the
//! PNG and its geometry sidecar and keeping the artifacts dir within
//! its budget are all the engine's (`crate::application::capture`), so
//! the two surfaces cannot drift apart.
//!
//! **Pixels never travel inline.** The PNG goes to disk and the reply
//! names it (`format/ipc.md` §Large payloads).
//!
//! **Truthful.** A frame the renderer did not draw is `rendered:
//! false` with the reason in `skipped`, and no file — so "nothing was
//! drawn" never arrives looking like "black was drawn". And `settled`
//! says whether the app needed no further frame when the capture was
//! taken.

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
use serde_json::Map;
use serde_json::{json, Value};

use crate::application::capture::geometry::CameraInfo;
#[cfg(not(target_arch = "wasm32"))]
use crate::application::capture::plan::{CaptureRequest, Framing, Region};
#[cfg(not(target_arch = "wasm32"))]
use crate::application::capture::record::{fps_of, max_frames_of, RecordDestination, Summary};
#[cfg(not(target_arch = "wasm32"))]
use crate::application::capture::{CaptureError, CaptureOutcome, Destination};
use crate::application::ipc::envelope::Reply;
use crate::application::ipc::registry::{
    Call, CommandSpec, Family, ParamSpec, ParamType, ResultSpec, Support,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::application::ipc::space::Space;
#[cfg(not(target_arch = "wasm32"))]
use crate::application::ipc::Screenshot;
#[cfg(target_arch = "wasm32")]
use crate::application::ipc::{CanvasCapture, CanvasOutcome};
#[cfg(not(target_arch = "wasm32"))]
use crate::application::renderer::ModalLayer;

#[cfg(target_arch = "wasm32")]
use super::refusal::unsupported_on_browser;
use super::refusal::{no_loaded_document, refuse, Refusal};

/// The image formats `format` names. One at pin time; the parameter
/// exists so a second is not a breaking change.
const FORMATS: [&str; 1] = ["png"];

/// The keys a `region` object carries, all numbers.
#[cfg(not(target_arch = "wasm32"))]
const REGION_KEYS: [&str; 4] = ["x", "y", "w", "h"];

/// What `capture.screenshot` does, per target. The browser cannot draw
/// offscreen, so its capture is the canvas as drawn — the live view at
/// its own size — read back and returned inline (`format/ipc.md`
/// §Transport "Browser").
#[cfg(not(target_arch = "wasm32"))]
const SCREENSHOT_SUMMARY: &str = "Render the current document state offscreen — never a window grab — and \
                                  write it as a PNG, with a geometry sidecar mapping its pixels to node ids.";
#[cfg(target_arch = "wasm32")]
const SCREENSHOT_SUMMARY: &str = "Draw the live view into the canvas and read it back as a PNG, returned \
                                  inline as base64 with a geometry sidecar mapping its pixels to node ids.";

/// Why the two recording commands are refused on the browser: a
/// recording is frames on disk, taken by a recorder the browser's loop
/// does not run. The screenshot is served there, inline.
const RECORD_NATIVE_ONLY: &str = "the browser has no filesystem and no frame recorder; recording is \
                                  native-only. `capture.screenshot` on the browser returns the canvas \
                                  inline (`png_base64`), which `mandalactl --ws` writes to a file";

pub(super) const FAMILY: Family = Family {
    name: "capture",
    commands: &[CommandSpec {
        name: "capture.screenshot",
        summary: SCREENSHOT_SUMMARY,
        // The framing parameters and the file destination are native's:
        // the browser draws the live view into its canvas and returns
        // it inline, so it declares neither (`format/ipc.md`
        // §Transport "Browser"), and a request naming one there is a
        // stray parameter.
        params: &[
            #[cfg(not(target_arch = "wasm32"))]
            ParamSpec::new(
                "path",
                ParamType::String,
                false,
                "Absolute path for the PNG; its parent must exist, and a symlink there is never followed. \
                 Default: a fresh file in the per-session artifacts dir.",
            ),
            #[cfg(not(target_arch = "wasm32"))]
            ParamSpec::new(
                "overwrite",
                ParamType::Boolean,
                false,
                "Allow `path` to replace an existing file. Default false; only valid with `path`. Inside \
                 the artifacts dir, overwriting is always allowed.",
            ),
            #[cfg(not(target_arch = "wasm32"))]
            ParamSpec::new(
                "width",
                ParamType::Integer,
                false,
                "Target width in view pixels, 1..=16384; sent with `height`. Default: the window's.",
            ),
            #[cfg(not(target_arch = "wasm32"))]
            ParamSpec::new(
                "height",
                ParamType::Integer,
                false,
                "Target height in view pixels, 1..=16384; sent with `width`. Default: the window's.",
            ),
            #[cfg(not(target_arch = "wasm32"))]
            ParamSpec::new(
                "region",
                ParamType::Object,
                false,
                "{x, y, w, h} in `space`: frame the smallest aspect-matching view that contains this \
                 rect instead of the current camera's.",
            ),
            #[cfg(not(target_arch = "wasm32"))]
            ParamSpec::new(
                "space",
                ParamType::String,
                false,
                "Coordinate space of `region`: \"canvas\" (default) or \"surface\" (the live window). \
                 Only valid with `region`.",
            ),
            #[cfg(not(target_arch = "wasm32"))]
            ParamSpec::new(
                "fit",
                ParamType::Boolean,
                false,
                "Frame the whole drawn document, as the load-time fit does. Not with `region`.",
            ),
            #[cfg(not(target_arch = "wasm32"))]
            ParamSpec::new(
                "scale",
                ParamType::Number,
                false,
                "Image pixels per view pixel, 0.1..=4.0 (default 1.0). Changes the resolution, never what \
                 is shown. width × height × scale² must not exceed 67,108,864.",
            ),
            ParamSpec::new(
                "format",
                ParamType::String,
                false,
                "The image format: \"png\", the only one (default).",
            ),
            ParamSpec::new(
                "sidecar",
                ParamType::Boolean,
                false,
                "Write the geometry sidecar, `<path>.geometry.json`. Default true.",
            ),
        ],
        result: &[
            ResultSpec::new(
                "rendered",
                ParamType::Boolean,
                "Whether the frame was drawn. When false, `skipped` says why and no file was written.",
            ),
            ResultSpec::new(
                "skipped",
                ParamType::String,
                "When not rendered: \"not_started\", \"font_system_contended\" or \"prepare_failed\".",
            ),
            #[cfg(not(target_arch = "wasm32"))]
            ResultSpec::new(
                "path",
                ParamType::String,
                "When rendered: the absolute path of the PNG.",
            ),
            #[cfg(not(target_arch = "wasm32"))]
            ResultSpec::new(
                "sidecar_path",
                ParamType::String,
                "When rendered with `sidecar`: the absolute path of the geometry sidecar.",
            ),
            #[cfg(target_arch = "wasm32")]
            ResultSpec::new(
                "png_base64",
                ParamType::String,
                "When rendered: the PNG, base64-encoded. `mandalactl --ws capture.screenshot <path>` \
                 writes it to the file.",
            ),
            #[cfg(target_arch = "wasm32")]
            ResultSpec::new(
                "sidecar",
                ParamType::Object,
                "When rendered with `sidecar`: the geometry sidecar, inline (the object native writes \
                 to `<path>.geometry.json`).",
            ),
            ResultSpec::new("width", ParamType::Integer, "The image's width in pixels."),
            ResultSpec::new("height", ParamType::Integer, "The image's height in pixels."),
            ResultSpec::new("scale", ParamType::Number, "Image pixels per view pixel."),
            ResultSpec::new(
                "camera",
                ParamType::Object,
                "{center_x, center_y, zoom}: a canvas point p is at image pixel (p - center) × zoom × \
                 scale + (width, height) / 2.",
            ),
            ResultSpec::new(
                "settled",
                ParamType::Boolean,
                "Whether the app needed no further frame when the capture was taken: no animation, \
                 pending drag or queued input.",
            ),
        ],
        support: Support::Everywhere,
        run: screenshot,
    },
    CommandSpec {
        name: "capture.record_start",
        summary: "Start recording a frame sequence: PNG frames plus a manifest.json with each frame's time, \
                  for `mandalactl record assemble` (a GIF) or any tool that reads JSON.",
        params: &[
            ParamSpec::new(
                "dir",
                ParamType::String,
                false,
                "Absolute path of the directory for the frames: not there yet (its parent must exist) or \
                 empty, and never a symlink. Default: a fresh directory in the per-session artifacts dir.",
            ),
            ParamSpec::new(
                "fps",
                ParamType::Integer,
                false,
                "Frames per second, 1..=60 (default 30).",
            ),
            ParamSpec::new(
                "max_frames",
                ParamType::Integer,
                false,
                "The recording stops itself after this many frames, 1..=3600 (default 600).",
            ),
            ParamSpec::new(
                "width",
                ParamType::Integer,
                false,
                "Frame width in view pixels, 1..=16384; sent with `height`. Default: the window's.",
            ),
            ParamSpec::new(
                "height",
                ParamType::Integer,
                false,
                "Frame height in view pixels, 1..=16384; sent with `width`. Default: the window's.",
            ),
            ParamSpec::new(
                "region",
                ParamType::Object,
                false,
                "{x, y, w, h} in `space`: frame the smallest aspect-matching view that contains this \
                 rect instead of the current camera's.",
            ),
            ParamSpec::new(
                "space",
                ParamType::String,
                false,
                "Coordinate space of `region`: \"canvas\" (default) or \"surface\" (the live window). \
                 Only valid with `region`.",
            ),
            ParamSpec::new(
                "fit",
                ParamType::Boolean,
                false,
                "Frame the whole drawn document, as the load-time fit does. Not with `region`.",
            ),
            ParamSpec::new(
                "scale",
                ParamType::Number,
                false,
                "Image pixels per view pixel, 0.1..=4.0 (default 1.0). width × height × scale² must not \
                 exceed 67,108,864.",
            ),
        ],
        result: &[
            ResultSpec::new("dir", ParamType::String, "The absolute path of the directory the frames go in."),
            ResultSpec::new(
                "manifest_path",
                ParamType::String,
                "Where manifest.json will be: written as frames are, complete when the recording ends.",
            ),
            ResultSpec::new("width", ParamType::Integer, "Every frame's width in pixels."),
            ResultSpec::new("height", ParamType::Integer, "Every frame's height in pixels."),
            ResultSpec::new("fps", ParamType::Integer, "Frames per second, as asked."),
            ResultSpec::new(
                "max_frames",
                ParamType::Integer,
                "The frame count at which the recording stops itself.",
            ),
            ResultSpec::new(
                "previous",
                ParamType::Object,
                "Present when an earlier recording ended on its own and no `capture.record_stop` had \
                 collected it: that report (what `capture.record_stop` returns), or {error} for one that \
                 failed. Starting a recording discards it, so it is handed back here.",
            ),
        ],
        support: Support::NativeOnly(RECORD_NATIVE_ONLY),
        #[cfg(not(target_arch = "wasm32"))]
        run: record_start,
        #[cfg(target_arch = "wasm32")]
        run: unsupported_on_browser,
    },
    CommandSpec {
        name: "capture.record_stop",
        summary: "Stop the recording and finish its manifest; or collect the report of one that stopped itself.",
        params: &[],
        result: &[
            ResultSpec::new("dir", ParamType::String, "The absolute path of the directory with the frames."),
            ResultSpec::new("manifest_path", ParamType::String, "The absolute path of manifest.json."),
            ResultSpec::new("frames", ParamType::Integer, "How many frames were taken."),
            ResultSpec::new(
                "dropped_frames",
                ParamType::Integer,
                "How many frames were due and not taken (late, undrawn, undrawable); the manifest's \
                 `gaps` says where and why.",
            ),
            ResultSpec::new("duration_ms", ParamType::Number, "How long the recording ran, in ms."),
            ResultSpec::new(
                "stopped",
                ParamType::String,
                "Why it ended: \"client\", \"max_frames\" or \"disk_budget\".",
            ),
            ResultSpec::new("bytes", ParamType::Integer, "The size of the frames and manifest on disk."),
            ResultSpec::new("width", ParamType::Integer, "Every frame's width in pixels."),
            ResultSpec::new("height", ParamType::Integer, "Every frame's height in pixels."),
            ResultSpec::new("fps", ParamType::Integer, "Frames per second, as asked."),
        ],
        support: Support::NativeOnly(RECORD_NATIVE_ONLY),
        #[cfg(not(target_arch = "wasm32"))]
        run: record_stop,
        #[cfg(target_arch = "wasm32")]
        run: unsupported_on_browser,
    }],
};

/// The two parameters both targets' screenshots take: `format`, which
/// must be the one format, and `sidecar`, which defaults on. Returns
/// whether a sidecar was asked for.
fn parse_format_and_sidecar(call: &Call<'_>) -> Result<bool, Refusal> {
    let format = call.str_param("format").unwrap_or(FORMATS[0]);
    if !FORMATS.contains(&format) {
        return Err(Refusal::Invalid(
            "format",
            format!("`format` must be \"png\", got \"{format}\""),
        ));
    }
    Ok(call.param("sidecar").and_then(Value::as_bool).unwrap_or(true))
}

/// `capture.screenshot` on the browser: the canvas, read back and
/// returned inline (`format/ipc.md` §Transport "Browser").
#[cfg(target_arch = "wasm32")]
fn screenshot(call: &mut Call<'_>) -> Reply {
    let sidecar = match parse_format_and_sidecar(call) {
        Ok(sidecar) => sidecar,
        Err(refusal) => return refuse(call, refusal),
    };
    let Some(app) = call.drive() else {
        return call.no_document();
    };
    match app.capture_canvas(sidecar) {
        None => no_loaded_document(call),
        Some(Ok(capture)) => call.ok(canvas_result_of(&capture)),
        Some(Err(message)) => refuse(call, Refusal::Internal(message)),
    }
}

/// The reply's `result` for what the browser's capture did.
#[cfg(target_arch = "wasm32")]
fn canvas_result_of(capture: &CanvasCapture) -> Value {
    let camera = match &capture.outcome {
        CanvasOutcome::Drawn { camera, .. } | CanvasOutcome::Skipped { camera, .. } => camera,
    };
    let mut result = json!({
        "rendered": matches!(capture.outcome, CanvasOutcome::Drawn { .. }),
        "width": camera.viewport_size.0,
        "height": camera.viewport_size.1,
        "scale": crate::application::document::f32_json(camera.pixel_ratio),
        "camera": CameraInfo::of(camera),
        "settled": capture.settled,
    });
    let object = result.as_object_mut().expect("built as an object just above");
    match &capture.outcome {
        CanvasOutcome::Drawn {
            png_base64, sidecar, ..
        } => {
            object.insert("png_base64".into(), json!(png_base64));
            if let Some(sidecar) = sidecar {
                object.insert("sidecar".into(), json!(sidecar));
            }
        }
        CanvasOutcome::Skipped { skip, .. } => {
            object.insert("skipped".into(), json!(skip.word()));
        }
    }
    result
}

/// The request, as the engine takes it.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, PartialEq)]
struct Parsed {
    request: CaptureRequest,
    destination: Destination,
    sidecar: bool,
}

#[cfg(not(target_arch = "wasm32"))]
fn screenshot(call: &mut Call<'_>) -> Reply {
    let parsed = match parse(call) {
        Ok(parsed) => parsed,
        Err(refusal) => return refuse(call, refusal),
    };
    let Some(app) = call.drive() else {
        return call.no_document();
    };
    match app.capture(&parsed.request, parsed.destination, parsed.sidecar) {
        None => no_loaded_document(call),
        Some(Ok(screenshot)) => call.ok(result_of(&screenshot)),
        Some(Err(error)) => refuse(call, refusal_of(error)),
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// How the engine's refusal is worded on the wire.
fn refusal_of(error: CaptureError) -> Refusal {
    match error {
        CaptureError::Invalid { param, message } => Refusal::Invalid(param, message),
        CaptureError::Internal(message) => Refusal::Internal(message),
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// `capture.record_start`.
fn record_start(call: &mut Call<'_>) -> Reply {
    let parsed = match parse_recording(call) {
        Ok(parsed) => parsed,
        Err(refusal) => return refuse(call, refusal),
    };
    let Some(app) = call.drive() else {
        return call.no_document();
    };
    match app.record_start(&parsed.request, parsed.fps, parsed.max_frames, parsed.destination) {
        None => no_loaded_document(call),
        Some(Ok(started)) => {
            let mut result = json!({
                "dir": started.dir.to_string_lossy(),
                "manifest_path": started.manifest_path.to_string_lossy(),
                "width": started.width,
                "height": started.height,
                "fps": started.fps,
                "max_frames": started.max_frames,
            });
            if let (Some(previous), Some(object)) = (&started.previous, result.as_object_mut()) {
                object.insert(
                    "previous".into(),
                    match previous {
                        Ok(summary) => summary_json(summary),
                        Err(error) => json!({ "error": error }),
                    },
                );
            }
            call.ok(result)
        }
        Some(Err(error)) => refuse(call, refusal_of(error)),
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// `capture.record_stop`.
fn record_stop(call: &mut Call<'_>) -> Reply {
    let Some(app) = call.drive() else {
        return call.no_document();
    };
    match app.record_stop() {
        Ok(summary) => call.ok(summary_json(&summary)),
        Err(error) => refuse(call, refusal_of(error)),
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// A finished recording, as `capture.record_stop` reports it.
fn summary_json(summary: &Summary) -> Value {
    json!({
        "dir": summary.dir.to_string_lossy(),
        "manifest_path": summary.manifest_path.to_string_lossy(),
        "frames": summary.frames,
        "dropped_frames": summary.dropped_frames,
        "duration_ms": summary.duration_ms,
        "stopped": summary.stopped.word(),
        "bytes": summary.bytes,
        "width": summary.width,
        "height": summary.height,
        "fps": summary.fps,
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// A recording's request, as the engine takes it.
#[derive(Debug, PartialEq)]
struct ParsedRecording {
    request: CaptureRequest,
    destination: RecordDestination,
    fps: u32,
    max_frames: u32,
}

#[cfg(not(target_arch = "wasm32"))]
/// Check a recording's shape: the view it frames is a screenshot's, its
/// cadence and length are bounded here, and its directory is vetted by
/// the engine, which has the disk.
fn parse_recording(call: &Call<'_>) -> Result<ParsedRecording, Refusal> {
    // The bounds are `record`'s own (`record::{fps_of, max_frames_of}`),
    // the same two checks the console runs; only the wording is this
    // surface's. A value that is no whole number reads as one out of
    // every range.
    let bounded = |name: &'static str,
                   check: fn(Option<u64>) -> Result<u32, std::ops::RangeInclusive<u32>>| {
        let typed = call.param(name);
        check(typed.map(|value| value.as_u64().unwrap_or(u64::MAX))).map_err(|range| {
            Refusal::Invalid(
                name,
                format!(
                    "`{name}` must be a whole number in {}..={}, got {}",
                    range.start(),
                    range.end(),
                    typed.map_or(String::new(), |value| value.to_string())
                ),
            )
        })
    };
    Ok(ParsedRecording {
        request: request_of(call)?,
        destination: call.str_param("dir").map_or(RecordDestination::Artifacts, |dir| {
            RecordDestination::Path(PathBuf::from(dir))
        }),
        fps: bounded("fps", fps_of)?,
        max_frames: bounded("max_frames", max_frames_of)?,
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// The reply's `result` for what the engine did.
fn result_of(screenshot: &Screenshot) -> Value {
    let (camera, files) = match &screenshot.outcome {
        CaptureOutcome::Written {
            camera,
            path,
            sidecar_path,
        } => (camera, Ok((path, sidecar_path))),
        CaptureOutcome::Skipped { camera, skip } => (camera, Err(skip.word())),
    };
    let mut result = json!({
        "rendered": files.is_ok(),
        "width": camera.viewport_size.0,
        "height": camera.viewport_size.1,
        "scale": crate::application::document::f32_json(camera.pixel_ratio),
        "camera": CameraInfo::of(camera),
        "settled": screenshot.settled,
    });
    let object = result.as_object_mut().expect("built as an object just above");
    match files {
        Ok((path, sidecar_path)) => {
            object.insert("path".into(), json!(path.to_string_lossy()));
            if let Some(sidecar_path) = sidecar_path {
                object.insert("sidecar_path".into(), json!(sidecar_path.to_string_lossy()));
            }
        }
        Err(skipped) => {
            object.insert("skipped".into(), json!(skipped));
        }
    }
    result
}

#[cfg(not(target_arch = "wasm32"))]
/// Check the request's shape: everything about it that does not need
/// the app. Bounds that do — the window's size, the GPU's texture
/// limit, what `fit` has to frame — are the engine's plan.
fn parse(call: &Call<'_>) -> Result<Parsed, Refusal> {
    let invalid = |param: &'static str, message: String| Refusal::Invalid(param, message);
    let sidecar = parse_format_and_sidecar(call)?;

    let destination = match (
        call.str_param("path"),
        call.param("overwrite").and_then(Value::as_bool),
    ) {
        (None, Some(_)) => {
            return Err(invalid(
                "overwrite",
                "`overwrite` only applies with `path`; the artifacts dir is always overwritable".into(),
            ))
        }
        (None, None) => Destination::Artifacts,
        (Some(path), overwrite) => Destination::Path {
            path: PathBuf::from(path),
            overwrite: overwrite.unwrap_or(false),
        },
    };

    Ok(Parsed {
        request: request_of(call)?,
        destination,
        sidecar,
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// The view a capture or a recording frames: its size, its framing and
/// its scale. The same parameters, bounds and refusals for both, so a
/// recording's frames are the screenshots the same words would take.
fn request_of(call: &Call<'_>) -> Result<CaptureRequest, Refusal> {
    let invalid = |param: &'static str, message: String| Refusal::Invalid(param, message);

    let side = |name: &'static str| -> Result<Option<u32>, Refusal> {
        call.param(name)
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or_else(|| invalid(name, format!("`{name}` must lie in 1..=16384, got {value}")))
            })
            .transpose()
    };
    let size = match (side("width")?, side("height")?) {
        (Some(width), Some(height)) => Some((width, height)),
        (None, None) => None,
        (Some(_), None) => return Err(invalid("height", "`width` and `height` are sent together".into())),
        (None, Some(_)) => return Err(invalid("width", "`width` and `height` are sent together".into())),
    };

    let space = call
        .str_param("space")
        .map(|word| {
            Space::parse(word).ok_or_else(|| invalid("space", format!("`space` must be {}", Space::WORDS)))
        })
        .transpose()?;
    let fit = call.param("fit").and_then(Value::as_bool).unwrap_or(false);
    let framing = match (call.param("region").and_then(Value::as_object), fit) {
        (Some(_), true) => {
            return Err(invalid(
                "fit",
                "`fit` and `region` each frame the capture; send one".into(),
            ))
        }
        (Some(region), false) => Framing::Region(parse_region(region, space.unwrap_or(Space::Canvas))?),
        (None, _) if space.is_some() => {
            return Err(invalid(
                "space",
                "`space` qualifies a `region`; send it with one".into(),
            ))
        }
        (None, true) => Framing::Fit,
        (None, false) => Framing::View,
    };

    let scale = call.param("scale").and_then(Value::as_f64).unwrap_or(1.0) as f32;
    Ok(CaptureRequest {
        size,
        framing,
        scale,
        // Over IPC the capture shows what the window shows, an
        // open modal included (at the window's size).
        modal: ModalLayer::Drawn,
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// A `region` object: exactly `x`, `y`, `w`, `h`, each a number. Whether
/// the rect can be framed is the plan's to say.
fn parse_region(region: &Map<String, Value>, space: Space) -> Result<Region, Refusal> {
    let shape = || {
        Refusal::Invalid(
            "region",
            format!(
                "`region` must be {{\"x\", \"y\", \"w\", \"h\"}}, each a number, got {}",
                Value::Object(region.clone())
            ),
        )
    };
    if region.len() != REGION_KEYS.len() || !REGION_KEYS.iter().all(|key| region.contains_key(*key)) {
        return Err(shape());
    }
    let number = |key: &str| region[key].as_f64().map(|v| v as f32).ok_or_else(shape);
    Ok(Region {
        x: number("x")?,
        y: number("y")?,
        w: number("w")?,
        h: number("h")?,
        space,
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use baumhard::gfx_structs::camera::Camera2D;
    use glam::Vec2;

    use crate::application::capture::record::{Started, Stopped, Summary};
    use crate::application::console::LineOutcome;
    use crate::application::document::MindMapDocument;
    use crate::application::ipc::events::Observed;
    use crate::application::ipc::registry::tests::{run_attached, run_detached};
    use crate::application::ipc::space::Point;
    use crate::application::ipc::{CommandTarget, SceneView, ShellState};
    use crate::application::keybinds::Action;
    use crate::application::macros::MacroStep;
    use crate::application::renderer::FrameSkip;

    /// The engine's seam, standing in for the engine: it records what
    /// it was asked for and answers what the test told it to. The engine
    /// itself is tested where it lives (`capture::{plan, save}`,
    /// `renderer::capture`); this pins the wire — what a request turns
    /// into, and what an outcome is worded as. Every other method fails
    /// the test that reaches it.
    struct CaptureApp {
        loaded: bool,
        answer: Result<Screenshot, CaptureError>,
        asked: Vec<Parsed>,
        /// What `record_start` answers, and what it was asked.
        started: Result<Started, CaptureError>,
        recordings: Vec<ParsedRecording>,
        /// What `record_stop` answers, and how often it was asked.
        stopped: Result<Summary, CaptureError>,
        stops: u32,
    }

    fn camera() -> Camera2D {
        let mut camera = Camera2D::new(2560, 1440);
        camera.position = Vec2::new(12.0, -340.5);
        camera.zoom = 0.75;
        camera.pixel_ratio = 2.0;
        camera
    }

    impl CaptureApp {
        fn answering(answer: Result<Screenshot, CaptureError>) -> Self {
            CaptureApp {
                loaded: true,
                answer,
                asked: Vec::new(),
                started: Ok(Started {
                    dir: PathBuf::from("/tmp/rec"),
                    manifest_path: PathBuf::from("/tmp/rec/manifest.json"),
                    width: 1280,
                    height: 720,
                    fps: 30,
                    max_frames: 600,
                    previous: None,
                }),
                recordings: Vec::new(),
                stopped: Ok(Summary {
                    dir: PathBuf::from("/tmp/rec"),
                    manifest_path: PathBuf::from("/tmp/rec/manifest.json"),
                    frames: 88,
                    dropped_frames: 2,
                    duration_ms: 3012.5,
                    stopped: Stopped::Client,
                    bytes: 123_456,
                    width: 1280,
                    height: 720,
                    fps: 30,
                }),
                stops: 0,
            }
        }

        fn written() -> Self {
            Self::answering(Ok(Screenshot {
                outcome: CaptureOutcome::Written {
                    camera: camera(),
                    path: PathBuf::from("/tmp/a/shot.png"),
                    sidecar_path: Some(PathBuf::from("/tmp/a/shot.png.geometry.json")),
                },
                settled: true,
            }))
        }
    }

    impl CommandTarget for CaptureApp {
        fn dispatch_action(&mut self, _: Action, _: Option<Point>) -> bool {
            panic!("a capture must not dispatch an Action");
        }
        fn run_console_line(&mut self, _: &str) -> Option<LineOutcome> {
            panic!("a capture must not run a console line");
        }
        fn has_macro(&self, _: &str) -> bool {
            panic!("a capture must not look up a macro");
        }
        fn run_macro(&mut self, _: &str, _: &mut dyn FnMut(Observed<'_>)) -> bool {
            panic!("a capture must not run a macro");
        }
        fn run_macro_steps(&mut self, _: &[MacroStep], _: &mut dyn FnMut(Observed<'_>)) -> bool {
            panic!("a capture must not run macro steps");
        }
        fn changed_view(&self) -> bool {
            false
        }
        fn document(&self) -> Option<&MindMapDocument> {
            panic!("a capture reaches the document through the engine");
        }
        fn animation_pause(&self) -> Option<u64> {
            panic!("a capture must not read the animation pause");
        }
        fn shell_state(&self, _: ShellState) -> Value {
            panic!("a capture must not read the shell state");
        }
        fn scene(&self) -> Option<SceneView<'_>> {
            panic!("a capture reaches the scene through the engine");
        }
        fn hit_test(&mut self, _: Point) -> Option<Value> {
            panic!("a capture must not hit-test");
        }
        fn surface_point(&self, _: Point) -> (f64, f64) {
            panic!("a capture must not place a pointer");
        }
        fn inject(
            &mut self,
            _: Vec<crate::application::ipc::injected::Sample>,
            _: crate::application::ipc::injected::Pacing,
        ) -> Result<crate::application::ipc::Injected, crate::application::ipc::injected::Overloaded>
        {
            panic!("a capture must not inject input");
        }
        fn injected_pending(&self) -> usize {
            panic!("a capture must not read the injected-input queue");
        }
        fn capture(
            &mut self,
            request: &CaptureRequest,
            destination: Destination,
            sidecar: bool,
        ) -> Option<Result<Screenshot, CaptureError>> {
            self.asked.push(Parsed {
                request: *request,
                destination,
                sidecar,
            });
            self.loaded.then(|| self.answer.clone())
        }
        fn record_start(
            &mut self,
            request: &CaptureRequest,
            fps: u32,
            max_frames: u32,
            destination: RecordDestination,
        ) -> Option<Result<Started, CaptureError>> {
            self.recordings.push(ParsedRecording {
                request: *request,
                destination,
                fps,
                max_frames,
            });
            self.loaded.then(|| self.started.clone())
        }
        fn record_stop(&mut self) -> Result<Summary, CaptureError> {
            self.stops += 1;
            self.stopped.clone()
        }
        fn clock(&self) -> &crate::application::clock::AppClock {
            panic!("a capture command must not read the clock");
        }
        fn set_clock_mode(
            &mut self,
            _: crate::application::clock::ClockMode,
        ) -> Result<bool, crate::application::clock::ClockRefusal> {
            panic!("a capture command must not switch the clock");
        }
        fn step_clock(
            &mut self,
            _: crate::application::clock::StepRequest,
        ) -> Result<crate::application::clock::Stepped, crate::application::clock::ClockRefusal> {
            panic!("a capture command must not step the clock");
        }
    }

    fn asked(frame: &str) -> Parsed {
        let mut app = CaptureApp::written();
        let (reply, repaint) = run_attached(frame, &mut app);
        assert_eq!(reply["ok"], json!(true), "{frame}: {reply}");
        assert!(!repaint, "{frame}: a capture changes nothing the window draws");
        app.asked.pop().expect("the engine was asked")
    }

    fn refused(frame: &str) -> (String, String) {
        let mut app = CaptureApp::written();
        let (reply, _) = run_attached(frame, &mut app);
        assert_eq!(reply["ok"], json!(false), "{frame}: {reply}");
        assert!(
            app.asked.is_empty(),
            "{frame}: a malformed request never reaches the engine"
        );
        (
            reply["error"]["code"].as_str().expect("code").to_string(),
            reply["error"]["data"]["param"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        )
    }

    /// No parameters is the window as it stands, into the artifacts
    /// dir, with a sidecar.
    #[test]
    fn test_a_bare_screenshot_is_the_window_into_the_artifacts_dir() {
        assert_eq!(
            asked(r#"{"id":1,"cmd":"capture.screenshot"}"#),
            Parsed {
                request: CaptureRequest::default(),
                destination: Destination::Artifacts,
                sidecar: true,
            }
        );
    }

    /// Every parameter reaches the engine as what it says.
    #[test]
    fn test_every_parameter_reaches_the_engine() {
        let parsed = asked(
            r#"{"id":1,"cmd":"capture.screenshot","path":"/tmp/x.png","overwrite":true,"width":1920,
                "height":1080,"region":{"x":-10,"y":20.5,"w":300,"h":200},"space":"surface",
                "scale":1.5,"format":"png","sidecar":false}"#,
        );
        assert_eq!(
            parsed,
            Parsed {
                request: CaptureRequest {
                    size: Some((1920, 1080)),
                    framing: Framing::Region(Region {
                        x: -10.0,
                        y: 20.5,
                        w: 300.0,
                        h: 200.0,
                        space: Space::Surface,
                    }),
                    scale: 1.5,
                    modal: ModalLayer::Drawn,
                },
                destination: Destination::Path {
                    path: PathBuf::from("/tmp/x.png"),
                    overwrite: true,
                },
                sidecar: false,
            }
        );
        let fit = asked(r#"{"id":1,"cmd":"capture.screenshot","fit":true,"path":"/tmp/y.png"}"#);
        assert_eq!(fit.request.framing, Framing::Fit);
        assert_eq!(
            fit.destination,
            Destination::Path {
                path: PathBuf::from("/tmp/y.png"),
                overwrite: false
            }
        );
        let region = asked(r#"{"id":1,"cmd":"capture.screenshot","region":{"x":0,"y":0,"w":1,"h":1}}"#);
        assert!(
            matches!(
                region.request.framing,
                Framing::Region(Region {
                    space: Space::Canvas,
                    ..
                })
            ),
            "canvas is the default space"
        );
    }

    /// A malformed request is refused naming its parameter, before the
    /// engine sees it.
    #[test]
    fn test_a_malformed_request_is_refused_naming_its_parameter() {
        for (frame, param) in [
            (r#"{"id":1,"cmd":"capture.screenshot","format":"jpeg"}"#, "format"),
            (
                r#"{"id":1,"cmd":"capture.screenshot","overwrite":true}"#,
                "overwrite",
            ),
            (r#"{"id":1,"cmd":"capture.screenshot","width":10}"#, "height"),
            (r#"{"id":1,"cmd":"capture.screenshot","height":10}"#, "width"),
            (
                r#"{"id":1,"cmd":"capture.screenshot","width":-1,"height":10}"#,
                "width",
            ),
            (
                r#"{"id":1,"cmd":"capture.screenshot","width":10,"height":99999999999}"#,
                "height",
            ),
            (
                r#"{"id":1,"cmd":"capture.screenshot","space":"surface"}"#,
                "space",
            ),
            (
                r#"{"id":1,"cmd":"capture.screenshot","region":{"x":0,"y":0,"w":1,"h":1},"space":"world"}"#,
                "space",
            ),
            (
                r#"{"id":1,"cmd":"capture.screenshot","region":{"x":0,"y":0,"w":1,"h":1},"fit":true}"#,
                "fit",
            ),
            (
                r#"{"id":1,"cmd":"capture.screenshot","region":{"x":0,"y":0,"w":1}}"#,
                "region",
            ),
            (
                r#"{"id":1,"cmd":"capture.screenshot","region":{"x":0,"y":0,"w":1,"h":1,"z":2}}"#,
                "region",
            ),
            (
                r#"{"id":1,"cmd":"capture.screenshot","region":{"x":"0","y":0,"w":1,"h":1}}"#,
                "region",
            ),
        ] {
            assert_eq!(
                refused(frame),
                ("invalid_params".to_string(), param.to_string()),
                "{frame}"
            );
        }
    }

    /// A written capture names its files, its image and its camera; the
    /// camera is the view's, and the scale is the surface's.
    #[test]
    fn test_a_written_capture_names_its_files_image_and_camera() {
        let mut app = CaptureApp::written();
        let (reply, _) = run_attached(r#"{"id":1,"cmd":"capture.screenshot"}"#, &mut app);
        assert_eq!(
            reply["result"],
            json!({
                "rendered": true,
                "path": "/tmp/a/shot.png",
                "sidecar_path": "/tmp/a/shot.png.geometry.json",
                "width": 2560,
                "height": 1440,
                "scale": 2.0,
                "camera": {"center_x": 12.0, "center_y": -340.5, "zoom": 0.75},
                "settled": true,
            })
        );
    }

    /// A skipped frame says so and why, and names no file.
    #[test]
    fn test_a_skipped_frame_is_reported_as_not_rendered() {
        let mut app = CaptureApp::answering(Ok(Screenshot {
            outcome: CaptureOutcome::Skipped {
                camera: camera(),
                skip: FrameSkip::FontSystemContended,
            },
            settled: false,
        }));
        let (reply, _) = run_attached(r#"{"id":1,"cmd":"capture.screenshot"}"#, &mut app);
        let result = &reply["result"];
        assert_eq!(result["rendered"], json!(false));
        assert_eq!(result["skipped"], json!("font_system_contended"));
        assert_eq!(result["settled"], json!(false));
        assert!(
            result.get("path").is_none() && result.get("sidecar_path").is_none(),
            "{result}"
        );
    }

    /// The engine's refusals reach the wire under their codes, and the
    /// two ways of having no document answer `no_document`.
    #[test]
    fn test_engine_refusals_and_a_missing_document_reach_the_wire() {
        for (error, code) in [
            (
                CaptureError::Invalid {
                    param: "region",
                    message: "too large".into(),
                },
                "invalid_params",
            ),
            (
                CaptureError::Internal("the GPU did not finish".into()),
                "internal",
            ),
        ] {
            let mut app = CaptureApp::answering(Err(error));
            let (reply, _) = run_attached(r#"{"id":1,"cmd":"capture.screenshot"}"#, &mut app);
            assert_eq!(reply["error"]["code"], json!(code), "{reply}");
        }

        let (reply, _) = run_detached(r#"{"id":1,"cmd":"capture.screenshot"}"#);
        assert_eq!(reply["error"]["code"], json!("no_document"));
        let mut app = CaptureApp::written();
        app.loaded = false;
        let (reply, _) = run_attached(r#"{"id":1,"cmd":"capture.screenshot"}"#, &mut app);
        assert_eq!(reply["error"]["code"], json!("no_document"));
        assert!(reply["error"]["message"]
            .as_str()
            .expect("message")
            .contains("open <path>"));
    }

    fn recorded(frame: &str) -> ParsedRecording {
        let mut app = CaptureApp::written();
        let (reply, repaint) = run_attached(frame, &mut app);
        assert_eq!(reply["ok"], json!(true), "{frame}: {reply}");
        assert!(
            !repaint,
            "{frame}: a recording starts without touching what the window draws"
        );
        app.recordings.pop().expect("the engine was asked")
    }

    /// No parameters is thirty frames a second of the window as it
    /// stands, 600 at most, into a fresh dir in the artifacts dir.
    #[test]
    fn test_a_bare_record_start_is_the_documented_default() {
        assert_eq!(
            recorded(r#"{"id":1,"cmd":"capture.record_start"}"#),
            ParsedRecording {
                request: CaptureRequest::default(),
                destination: RecordDestination::Artifacts,
                fps: 30,
                max_frames: 600,
            }
        );
    }

    /// Every parameter reaches the engine as what it says — the view
    /// ones as a screenshot's do.
    #[test]
    fn test_every_record_start_parameter_reaches_the_engine() {
        let parsed = recorded(
            r#"{"id":1,"cmd":"capture.record_start","dir":"/tmp/rec","fps":12,"max_frames":3600,
                "width":640,"height":360,"fit":true,"scale":0.5}"#,
        );
        assert_eq!(
            parsed,
            ParsedRecording {
                request: CaptureRequest {
                    size: Some((640, 360)),
                    framing: Framing::Fit,
                    scale: 0.5,
                    modal: ModalLayer::Drawn,
                },
                destination: RecordDestination::Path(PathBuf::from("/tmp/rec")),
                fps: 12,
                max_frames: 3600,
            }
        );
        let region = recorded(
            r#"{"id":1,"cmd":"capture.record_start","region":{"x":0,"y":0,"w":10,"h":5},"space":"surface"}"#,
        );
        assert!(matches!(
            region.request.framing,
            Framing::Region(Region {
                space: Space::Surface,
                ..
            })
        ));
    }

    /// The bounds are refusals naming the parameter, never clamps, and
    /// the engine is not asked.
    #[test]
    fn test_record_start_bounds_are_refusals() {
        for (frame, param) in [
            (r#"{"id":1,"cmd":"capture.record_start","fps":0}"#, "fps"),
            (r#"{"id":1,"cmd":"capture.record_start","fps":61}"#, "fps"),
            (r#"{"id":1,"cmd":"capture.record_start","fps":-3}"#, "fps"),
            (
                r#"{"id":1,"cmd":"capture.record_start","max_frames":0}"#,
                "max_frames",
            ),
            (
                r#"{"id":1,"cmd":"capture.record_start","max_frames":3601}"#,
                "max_frames",
            ),
            (r#"{"id":1,"cmd":"capture.record_start","width":10}"#, "height"),
            (
                r#"{"id":1,"cmd":"capture.record_start","fit":true,"region":{"x":0,"y":0,"w":1,"h":1}}"#,
                "fit",
            ),
            (
                r#"{"id":1,"cmd":"capture.record_start","space":"surface"}"#,
                "space",
            ),
            // A screenshot's parameters are not a recording's.
            (
                r#"{"id":1,"cmd":"capture.record_start","path":"/tmp/x.png"}"#,
                "path",
            ),
            (
                r#"{"id":1,"cmd":"capture.record_start","sidecar":false}"#,
                "sidecar",
            ),
        ] {
            let mut app = CaptureApp::written();
            let (reply, _) = run_attached(frame, &mut app);
            assert_eq!(reply["ok"], json!(false), "{frame}: {reply}");
            assert_eq!(
                reply["error"]["code"],
                json!("invalid_params"),
                "{frame}: {reply}"
            );
            assert_eq!(reply["error"]["data"]["param"], json!(param), "{frame}: {reply}");
            assert!(app.recordings.is_empty(), "{frame}: refused before the engine");
        }
    }

    /// The start reply names the directory, the manifest and the frame
    /// size, which the first frame fixed.
    #[test]
    fn test_record_start_replies_with_where_and_what() {
        let mut app = CaptureApp::written();
        let (reply, _) = run_attached(r#"{"id":1,"cmd":"capture.record_start"}"#, &mut app);
        assert_eq!(
            reply["result"],
            json!({"dir": "/tmp/rec", "manifest_path": "/tmp/rec/manifest.json",
                   "width": 1280, "height": 720, "fps": 30, "max_frames": 600})
        );
    }

    /// The engine's refusals keep their codes: a recording already
    /// running is the caller's mistake, a failed disk is `internal`; no
    /// document is its own error.
    #[test]
    fn test_record_refusals_keep_the_engines_codes() {
        for (error, code) in [
            (
                CaptureError::Invalid {
                    param: "cmd",
                    message: "a recording is already running".into(),
                },
                "invalid_params",
            ),
            (CaptureError::Internal("disk".into()), "internal"),
        ] {
            let mut app = CaptureApp::written();
            app.started = Err(error.clone());
            app.stopped = Err(error);
            for frame in [
                r#"{"id":1,"cmd":"capture.record_start"}"#,
                r#"{"id":1,"cmd":"capture.record_stop"}"#,
            ] {
                let (reply, _) = run_attached(frame, &mut app);
                assert_eq!(reply["ok"], json!(false), "{frame}: {reply}");
                assert_eq!(reply["error"]["code"], json!(code), "{frame}: {reply}");
            }
        }
        let mut unloaded = CaptureApp::written();
        unloaded.loaded = false;
        let (reply, _) = run_attached(r#"{"id":1,"cmd":"capture.record_start"}"#, &mut unloaded);
        assert_eq!(reply["error"]["code"], json!("no_document"), "{reply}");
        let (reply, _) = run_detached(r#"{"id":1,"cmd":"capture.record_start"}"#);
        assert_eq!(reply["error"]["code"], json!("no_document"), "{reply}");
        let (reply, _) = run_detached(r#"{"id":1,"cmd":"capture.record_stop"}"#);
        assert_eq!(reply["error"]["code"], json!("no_document"), "{reply}");
    }

    /// The stop reply is the summary: where the frames are, how many
    /// were taken and how many were not, and why it ended.
    #[test]
    fn test_record_stop_replies_with_the_summary() {
        let mut app = CaptureApp::written();
        let (reply, repaint) = run_attached(r#"{"id":1,"cmd":"capture.record_stop"}"#, &mut app);
        assert!(!repaint);
        assert_eq!(app.stops, 1);
        assert_eq!(
            reply["result"],
            json!({"dir": "/tmp/rec", "manifest_path": "/tmp/rec/manifest.json", "frames": 88,
                   "dropped_frames": 2, "duration_ms": 3012.5, "stopped": "client", "bytes": 123456,
                   "width": 1280, "height": 720, "fps": 30})
        );
        let (reply, _) = run_attached(r#"{"id":2,"cmd":"capture.record_stop","extra":1}"#, &mut app);
        assert_eq!(reply["error"]["code"], json!("invalid_params"), "{reply}");
        assert_eq!(app.stops, 1, "an unknown parameter never reaches the engine");
    }

    /// A recording that ended on its own and was never collected is
    /// handed back when the next one starts, instead of being dropped
    /// without a word: its report, or its failure.
    #[test]
    fn test_starting_a_recording_hands_back_the_report_it_discards() {
        let summary = Summary {
            dir: PathBuf::from("/tmp/old"),
            manifest_path: PathBuf::from("/tmp/old/manifest.json"),
            frames: 600,
            dropped_frames: 0,
            duration_ms: 20_000.0,
            stopped: Stopped::MaxFrames,
            bytes: 9,
            width: 800,
            height: 600,
            fps: 30,
        };
        let mut app = CaptureApp::written();
        if let Ok(started) = &mut app.started {
            started.previous = Some(Ok(summary));
        }
        let (reply, _) = run_attached(r#"{"id":1,"cmd":"capture.record_start"}"#, &mut app);
        assert_eq!(reply["result"]["previous"]["dir"], "/tmp/old");
        assert_eq!(reply["result"]["previous"]["stopped"], "max_frames");
        assert_eq!(reply["result"]["previous"]["frames"], 600);

        let mut failed = CaptureApp::written();
        if let Ok(started) = &mut failed.started {
            started.previous = Some(Err("the recording ended early: disk full".into()));
        }
        let (reply, _) = run_attached(r#"{"id":1,"cmd":"capture.record_start"}"#, &mut failed);
        assert_eq!(
            reply["result"]["previous"],
            json!({"error": "the recording ended early: disk full"})
        );

        let (reply, _) = run_attached(
            r#"{"id":1,"cmd":"capture.record_start"}"#,
            &mut CaptureApp::written(),
        );
        assert!(
            reply["result"].get("previous").is_none(),
            "absent when there was nothing"
        );
    }
}
