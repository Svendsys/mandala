// SPDX-License-Identifier: MPL-2.0

//! The `capture` family: what the app draws, as pixels
//! (`format/ipc.md` §`capture`).
//!
//! **A thin surface over one engine.** `capture.screenshot` checks the
//! shape of its request, hands it to the screenshot engine through
//! [`CommandTarget::capture`](crate::application::ipc::CommandTarget::capture)
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

use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::application::capture::geometry::CameraInfo;
use crate::application::capture::plan::{CaptureRequest, Framing, Region};
use crate::application::capture::{CaptureError, CaptureOutcome, Destination};
use crate::application::ipc::envelope::Reply;
use crate::application::ipc::registry::{Call, CommandSpec, Family, ParamSpec, ParamType, ResultSpec};
use crate::application::ipc::space::Space;
use crate::application::ipc::Screenshot;
use crate::application::renderer::ModalLayer;

use super::refusal::{no_loaded_document, refuse, Refusal};

/// The image formats `format` names. One at pin time; the parameter
/// exists so a second is not a breaking change.
const FORMATS: [&str; 1] = ["png"];

/// The keys a `region` object carries, all numbers.
const REGION_KEYS: [&str; 4] = ["x", "y", "w", "h"];

pub(super) const FAMILY: Family = Family {
    name: "capture",
    commands: &[CommandSpec {
        name: "capture.screenshot",
        summary: "Render the current document state offscreen — never a window grab — and write it as a \
                  PNG, with a geometry sidecar mapping its pixels to node ids.",
        params: &[
            ParamSpec::new(
                "path",
                ParamType::String,
                false,
                "Absolute path for the PNG; its parent must exist, and a symlink there is never followed. \
                 Default: a fresh file in the per-session artifacts dir.",
            ),
            ParamSpec::new(
                "overwrite",
                ParamType::Boolean,
                false,
                "Allow `path` to replace an existing file. Default false; only valid with `path`. Inside \
                 the artifacts dir, overwriting is always allowed.",
            ),
            ParamSpec::new(
                "width",
                ParamType::Integer,
                false,
                "Target width in view pixels, 1..=16384; sent with `height`. Default: the window's.",
            ),
            ParamSpec::new(
                "height",
                ParamType::Integer,
                false,
                "Target height in view pixels, 1..=16384; sent with `width`. Default: the window's.",
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
            ResultSpec::new(
                "path",
                ParamType::String,
                "When rendered: the absolute path of the PNG.",
            ),
            ResultSpec::new(
                "sidecar_path",
                ParamType::String,
                "When rendered with `sidecar`: the absolute path of the geometry sidecar.",
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
        run: screenshot,
    }],
};

/// The request, as the engine takes it.
#[derive(Debug, PartialEq)]
struct Parsed {
    request: CaptureRequest,
    destination: Destination,
    sidecar: bool,
}

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
        Some(Err(error)) => refuse(
            call,
            match error {
                CaptureError::Invalid { param, message } => Refusal::Invalid(param, message),
                CaptureError::Busy(message) => Refusal::Busy(message),
                CaptureError::Internal(message) => Refusal::Internal(message),
            },
        ),
    }
}

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

/// Check the request's shape: everything about it that does not need
/// the app. Bounds that do — the window's size, the GPU's texture
/// limit, what `fit` has to frame — are the engine's plan.
fn parse(call: &Call<'_>) -> Result<Parsed, Refusal> {
    let invalid = |param: &'static str, message: String| Refusal::Invalid(param, message);

    let format = call.str_param("format").unwrap_or(FORMATS[0]);
    if !FORMATS.contains(&format) {
        return Err(invalid(
            "format",
            format!("`format` must be \"png\", got \"{format}\""),
        ));
    }

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
    Ok(Parsed {
        request: CaptureRequest {
            size,
            framing,
            scale,
            // Over IPC the capture shows what the window shows, an
            // open modal included (at the window's size).
            modal: ModalLayer::Drawn,
        },
        destination,
        sidecar: call.param("sidecar").and_then(Value::as_bool).unwrap_or(true),
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::gfx_structs::camera::Camera2D;
    use glam::Vec2;

    use crate::application::console::LineOutcome;
    use crate::application::ipc::events::Observed;
    use crate::application::document::MindMapDocument;
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
        fn observed(&self) -> Option<Observed<'_>> {
            None
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
            (CaptureError::Busy("a node drag is in progress".into()), "busy"),
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
}
