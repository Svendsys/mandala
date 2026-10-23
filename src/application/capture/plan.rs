// SPDX-License-Identifier: MPL-2.0

//! What a capture frames, decided before anything is allocated: a
//! request — size, framing, scale — checked against the protocol's
//! bounds and the GPU's, and turned into the one [`Camera2D`] the
//! offscreen frame is drawn through (`format/ipc.md`
//! §`capture.screenshot`).
//!
//! **One camera says everything.** The plan's camera has the image's
//! size as its viewport, the capture's `scale` as its pixel ratio, and
//! a center and zoom from the framing. The renderer draws through it
//! and the geometry sidecar maps through it, so the image and the map
//! of what is in it cannot disagree. Because zoom and pixel ratio are
//! separate numbers on the camera, a supersampled capture keeps the
//! window's zoom — and with it the elements the window shows and the
//! connection layout it samples — and only gains pixels.
//!
//! Pure over plain values, so every bound and every framing rule is
//! tested without a GPU (TEST_CONVENTIONS §T8).

use std::ops::RangeInclusive;

use baumhard::gfx_structs::camera::Camera2D;
use glam::Vec2;

use crate::application::console::commands::screenshot::{ConsoleView, ScreenshotFrame, ScreenshotLine};
use crate::application::ipc::space::Space;
use crate::application::renderer::{ModalLayer, FIT_PADDING_FRACTION};

/// The largest `width` or `height` a capture accepts, in pixels.
pub(crate) const MAX_SIDE: u32 = 16_384;

/// The most pixels one image may hold: `width × height × scale²`.
/// 64 Mpx, about an 8K square. An offscreen texture is a real GPU
/// allocation, so a request over it is refused rather than attempted.
pub(crate) const MAX_PIXELS: f64 = 67_108_864.0;

/// The `scale` a capture accepts.
pub(crate) const SCALE_RANGE: RangeInclusive<f32> = 0.1..=4.0;

/// A rect to frame, in the space it was given in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Region {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
    pub(crate) space: Space,
}

/// Where the capture camera looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Framing {
    /// The live camera's center and zoom.
    View,
    /// The smallest aspect-matching view that contains the rect,
    /// center-preserved.
    Region(Region),
    /// The document's drawn content, framed as the load-time fit
    /// frames it.
    Fit,
}

/// A capture, as asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CaptureRequest {
    /// `(width, height)` in view pixels, or `None` for the live
    /// surface's size.
    pub(crate) size: Option<(u32, u32)>,
    pub(crate) framing: Framing,
    /// Image pixels per view pixel.
    pub(crate) scale: f32,
    /// Whether an open modal (the console or the color picker) is in
    /// the picture. Over IPC it is: the capture shows what the window
    /// shows. From the console's own `screenshot` verb it is not,
    /// because the prompt that asked is not what the user wanted a
    /// picture of. Either way the engine also leaves it out of a capture
    /// sized unlike the window, where it has no coherent place
    /// ([`ModalLayer`]).
    pub(crate) modal: ModalLayer,
}

impl Default for CaptureRequest {
    /// The window as it stands: its size, its camera, one image pixel
    /// per surface pixel.
    fn default() -> Self {
        CaptureRequest {
            size: None,
            framing: Framing::View,
            scale: 1.0,
            modal: ModalLayer::Drawn,
        }
    }
}

impl CaptureRequest {
    /// The `screenshot` console verb's line, as the engine takes it.
    /// The console is always left out of the picture it was asked from.
    /// (The IPC request is built by the `capture` family, from its
    /// parameters; both arrive here as the same value.)
    pub(crate) fn from_console_line(line: &ScreenshotLine) -> Self {
        Self::from_console_view(&ConsoleView {
            size: line.size,
            scale: line.scale,
            frame: line.frame,
        })
    }

    /// The same for any console verb that frames a view: `screenshot`'s
    /// line, `record start`'s.
    pub(crate) fn from_console_view(view: &ConsoleView) -> Self {
        CaptureRequest {
            size: view.size,
            framing: match view.frame {
                ScreenshotFrame::View => Framing::View,
                ScreenshotFrame::Fit => Framing::Fit,
                ScreenshotFrame::Region { x, y, w, h, surface } => Framing::Region(Region {
                    x,
                    y,
                    w,
                    h,
                    space: if surface { Space::Surface } else { Space::Canvas },
                }),
            },
            scale: view.scale.unwrap_or(CaptureRequest::default().scale),
            modal: ModalLayer::Omitted,
        }
    }
}

/// Why a request cannot be planned, charged to the parameter that
/// asked for the impossible.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlanError {
    pub(crate) param: &'static str,
    pub(crate) message: String,
}

impl PlanError {
    fn new(param: &'static str, message: impl Into<String>) -> Self {
        PlanError {
            param,
            message: message.into(),
        }
    }
}

/// What the plan reads from the app.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlanContext {
    /// The renderer's camera as it stands.
    pub(crate) live: Camera2D,
    /// The GPU's largest 2D texture side.
    pub(crate) max_texture_side: u32,
    /// The canvas box around the drawn nodes, for `fit`; `None` when
    /// nothing is drawn. Only `fit` reads it, and the caller computes it
    /// only then: it is a walk of the whole node tree.
    pub(crate) content: Option<(Vec2, Vec2)>,
}

/// Turn `request` into the camera the capture draws through, or say
/// which parameter asked for something that cannot be drawn.
///
/// The checks, in the order an agent meets them: `scale` in
/// [`SCALE_RANGE`]; each side in `1..=`[`MAX_SIDE`]; the image within
/// [`MAX_PIXELS`] and within the GPU's texture side; then the framing —
/// a region with area, framed at a zoom no lower than
/// [`Camera2D::MIN_ZOOM`] (lower would cut it off), and something to
/// fit for `fit`.
///
/// A region that needs more zoom than [`Camera2D::MAX_ZOOM`] is framed
/// at the maximum: it is still wholly visible, with more canvas around
/// it, which the aspect rule already allows. The image is
/// `round(width × scale) × round(height × scale)` pixels.
///
/// O(1).
pub(crate) fn plan(request: &CaptureRequest, context: &PlanContext) -> Result<Camera2D, PlanError> {
    let scale = request.scale;
    if !SCALE_RANGE.contains(&scale) {
        return Err(PlanError::new(
            "scale",
            format!(
                "`scale` must lie in {}..={}, got {scale}",
                SCALE_RANGE.start(),
                SCALE_RANGE.end()
            ),
        ));
    }
    let (width, height) = request.size.unwrap_or(context.live.viewport_size);
    for (param, side) in [("width", width), ("height", height)] {
        if !(1..=MAX_SIDE).contains(&side) {
            return Err(PlanError::new(
                param,
                format!("`{param}` must lie in 1..={MAX_SIDE}, got {side}"),
            ));
        }
    }
    // The bounds below are on the image, which `scale` multiplies; say
    // so through whichever the client set.
    let budget_param = if request.size.is_some() { "width" } else { "scale" };
    let pixels = f64::from(width) * f64::from(height) * f64::from(scale) * f64::from(scale);
    if pixels > MAX_PIXELS {
        return Err(PlanError::new(
            budget_param,
            format!(
                "{width}×{height} at scale {scale} is {pixels:.0} pixels; one capture holds at most \
                 {MAX_PIXELS:.0} (width × height × scale²)"
            ),
        ));
    }
    let image = (image_side(width, scale), image_side(height, scale));
    let max_side = context.max_texture_side;
    if image.0 > max_side || image.1 > max_side {
        return Err(PlanError::new(
            budget_param,
            format!(
                "a {}×{} image exceeds this GPU's largest texture side, {max_side} pixels",
                image.0, image.1
            ),
        ));
    }

    let mut camera = Camera2D {
        viewport_size: image,
        pixel_ratio: scale,
        ..context.live
    };
    match request.framing {
        Framing::View => {}
        Framing::Region(region) => frame_region(&mut camera, &context.live, region)?,
        Framing::Fit => {
            let Some((min, max)) = context.content else {
                return Err(PlanError::new("fit", "the document draws nothing to fit"));
            };
            // The same refusal a region gets: below the minimum zoom the
            // clamp would cut the document off and still answer "fit".
            match camera.fit_zoom(min, max, FIT_PADDING_FRACTION) {
                Some(zoom) if zoom < Camera2D::MIN_ZOOM => {
                    return Err(PlanError::new(
                        "fit",
                        format!(
                            "the document is too large to fit a {} view: it needs zoom {zoom}, below the \
                             minimum {}; ask for a larger width and height",
                            view_size(&camera),
                            Camera2D::MIN_ZOOM
                        ),
                    ))
                }
                _ => camera.fit_to_bounds(min, max, FIT_PADDING_FRACTION),
            }
        }
    }
    Ok(camera)
}

/// `camera`'s view, `width×height` in view pixels, for a refusal.
fn view_size(camera: &Camera2D) -> String {
    format!(
        "{}×{}",
        (camera.viewport_size.0 as f32 / camera.pixel_ratio).round(),
        (camera.viewport_size.1 as f32 / camera.pixel_ratio).round()
    )
}

/// One side of the image: `side × scale`, rounded, and never empty.
fn image_side(side: u32, scale: f32) -> u32 {
    ((side as f32 * scale).round() as u32).max(1)
}

/// Center `camera` on `region` at the largest zoom that shows all of
/// it (`format/ipc.md` §`capture.screenshot`, "Aspect rule"). A
/// `surface` region is read against the live window the human sees,
/// through `live`.
fn frame_region(camera: &mut Camera2D, live: &Camera2D, region: Region) -> Result<(), PlanError> {
    let finite = [region.x, region.y, region.w, region.h]
        .iter()
        .all(|v| v.is_finite());
    if !finite || region.w <= 0.0 || region.h <= 0.0 {
        return Err(PlanError::new(
            "region",
            format!(
                "`region` needs finite x and y and a positive w and h, got {{x: {}, y: {}, w: {}, h: {}}}",
                region.x, region.y, region.w, region.h
            ),
        ));
    }
    let (corner, far) = (
        Vec2::new(region.x, region.y),
        Vec2::new(region.x + region.w, region.y + region.h),
    );
    let (min, max) = match region.space {
        Space::Canvas => (corner, far),
        Space::Surface => (live.screen_to_canvas(corner), live.screen_to_canvas(far)),
    };
    let zoom = camera
        .fit_zoom(min, max, 0.0)
        .ok_or_else(|| PlanError::new("region", "`region` has no area in canvas space"))?;
    if zoom < Camera2D::MIN_ZOOM {
        return Err(PlanError::new(
            "region",
            format!(
                "`region` is too large to frame at a {} view: it needs zoom {zoom}, below the \
                 minimum {}; ask for a larger width and height",
                view_size(camera),
                Camera2D::MIN_ZOOM
            ),
        ));
    }
    camera.position = (min + max) / 2.0;
    camera.zoom = zoom.min(Camera2D::MAX_ZOOM);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live() -> Camera2D {
        let mut camera = Camera2D::new(1280, 720);
        camera.position = Vec2::new(300.0, -40.0);
        camera.zoom = 0.8;
        camera
    }

    fn context() -> PlanContext {
        PlanContext {
            live: live(),
            max_texture_side: 16_384,
            content: Some((Vec2::new(-500.0, -200.0), Vec2::new(1500.0, 800.0))),
        }
    }

    fn request(size: Option<(u32, u32)>, framing: Framing, scale: f32) -> CaptureRequest {
        CaptureRequest {
            size,
            framing,
            scale,
            ..CaptureRequest::default()
        }
    }

    fn refusal(request: &CaptureRequest) -> PlanError {
        plan(request, &context()).expect_err("refused")
    }

    /// The default request is the window: the live camera exactly, so
    /// the capture draws what the window draws, pixel for pixel.
    #[test]
    fn test_the_default_request_is_the_live_camera_unchanged() {
        assert_eq!(plan(&CaptureRequest::default(), &context()), Ok(live()));
    }

    /// A size keeps the camera's center and zoom and shows more or less
    /// canvas; a scale multiplies the image and leaves the zoom alone,
    /// so a supersampled capture of a view frames the same canvas.
    #[test]
    fn test_size_and_scale_keep_the_view_and_multiply_the_pixels() {
        let sized = plan(&request(Some((1920, 1080)), Framing::View, 1.0), &context()).expect("plans");
        assert_eq!(sized.viewport_size, (1920, 1080));
        assert_eq!(
            (sized.position, sized.zoom, sized.pixel_ratio),
            (live().position, 0.8, 1.0)
        );

        let doubled = plan(&request(None, Framing::View, 2.0), &context()).expect("plans");
        assert_eq!(doubled.viewport_size, (2560, 1440));
        assert_eq!((doubled.zoom, doubled.pixel_ratio), (0.8, 2.0));
        assert_eq!(
            doubled.screen_to_canvas(Vec2::ZERO),
            live().screen_to_canvas(Vec2::ZERO),
            "the same canvas corner"
        );

        let fractional = plan(&request(Some((333, 101)), Framing::View, 1.5), &context()).expect("plans");
        assert_eq!(
            fractional.viewport_size,
            (500, 152),
            "rounded, never truncated to nothing"
        );
        let tiny = plan(&request(Some((1, 1)), Framing::View, 0.1), &context()).expect("plans");
        assert_eq!(tiny.viewport_size, (1, 1), "never an empty image");
    }

    /// Every bound is a refusal naming its parameter, never a clamp to
    /// something the client did not ask for.
    #[test]
    fn test_every_bound_is_refused_naming_its_parameter() {
        for scale in [0.09, 4.01, f32::NAN, f32::INFINITY, 0.0, -1.0] {
            assert_eq!(
                refusal(&request(None, Framing::View, scale)).param,
                "scale",
                "{scale}"
            );
        }
        for (size, param) in [
            ((0, 10), "width"),
            ((10, 0), "height"),
            ((16_385, 10), "width"),
            ((10, 16_385), "height"),
        ] {
            assert_eq!(
                refusal(&request(Some(size), Framing::View, 1.0)).param,
                param,
                "{size:?}"
            );
        }
        // 8192 × 8192 is exactly the budget; one more row is over it.
        assert!(plan(&request(Some((8192, 8192)), Framing::View, 1.0), &context()).is_ok());
        let over = refusal(&request(Some((8192, 8193)), Framing::View, 1.0));
        assert_eq!(over.param, "width");
        assert!(
            over.message.contains("width × height × scale²"),
            "{}",
            over.message
        );
        // Over the budget through `scale` alone: charged to `scale`.
        let window = PlanContext {
            live: Camera2D::new(4096, 4096),
            ..context()
        };
        assert_eq!(
            plan(&request(None, Framing::View, 2.01), &window)
                .expect_err("over")
                .param,
            "scale"
        );
        assert!(plan(&request(None, Framing::View, 2.0), &window).is_ok());

        let small_gpu = PlanContext {
            max_texture_side: 2048,
            ..context()
        };
        let gpu = plan(&request(Some((2049, 10)), Framing::View, 1.0), &small_gpu).expect_err("over");
        assert_eq!(gpu.param, "width");
        assert!(gpu.message.contains("2048"), "{}", gpu.message);
        assert!(plan(&request(Some((2048, 10)), Framing::View, 1.0), &small_gpu).is_ok());
    }

    /// The aspect rule: the region's center is the camera's, all of it
    /// is inside the image, and it touches the image's edges along one
    /// axis — the smallest aspect-matching view that contains it.
    #[test]
    fn test_a_region_is_wholly_visible_center_preserved_and_tight_on_one_axis() {
        let region = Region {
            x: 100.0,
            y: 50.0,
            w: 400.0,
            h: 100.0,
            space: Space::Canvas,
        };
        for (size, scale) in [
            ((800, 600), 1.0),
            ((600, 800), 1.0),
            ((1000, 250), 2.0),
            ((333, 777), 1.5),
        ] {
            let camera =
                plan(&request(Some(size), Framing::Region(region), scale), &context()).expect("plans");
            assert_eq!(camera.position, Vec2::new(300.0, 100.0), "{size:?}");
            let tl = camera.canvas_to_screen(Vec2::new(region.x, region.y));
            let br = camera.canvas_to_screen(Vec2::new(region.x + region.w, region.y + region.h));
            let (iw, ih) = (camera.viewport_size.0 as f32, camera.viewport_size.1 as f32);
            let eps = 1e-2;
            assert!(
                tl.x >= -eps && tl.y >= -eps && br.x <= iw + eps && br.y <= ih + eps,
                "{size:?}: {tl} {br}"
            );
            let tight_x = tl.x.abs() < eps && (br.x - iw).abs() < eps;
            let tight_y = tl.y.abs() < eps && (br.y - ih).abs() < eps;
            assert!(tight_x || tight_y, "{size:?}: tight along one axis, {tl} {br}");
        }
    }

    /// A `surface` region is read against the live window — the window
    /// the human sees — whatever size the capture is drawn at: framing
    /// the whole window at the window's size is the window's camera.
    #[test]
    fn test_a_surface_region_is_read_against_the_live_window() {
        let whole = Region {
            x: 0.0,
            y: 0.0,
            w: 1280.0,
            h: 720.0,
            space: Space::Surface,
        };
        let camera = plan(&request(None, Framing::Region(whole), 1.0), &context()).expect("plans");
        assert!(
            (camera.position - live().position).length() < 1e-3,
            "{}",
            camera.position
        );
        assert!((camera.zoom - live().zoom).abs() < 1e-5, "{}", camera.zoom);

        let doubled = plan(
            &request(Some((2560, 1440)), Framing::Region(whole), 1.0),
            &context(),
        )
        .expect("plans");
        assert!(
            (doubled.zoom - 2.0 * live().zoom).abs() < 1e-5,
            "the same window region at twice the pixels"
        );
    }

    /// A region too small for `MAX_ZOOM` is framed at it (still wholly
    /// visible); one too large for `MIN_ZOOM` would be cut off, so it
    /// is refused; one with no area or a non-finite corner is refused.
    #[test]
    fn test_a_region_beyond_the_zoom_range_is_framed_or_refused() {
        let tiny = Region {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            space: Space::Canvas,
        };
        let camera = plan(&request(None, Framing::Region(tiny), 1.0), &context()).expect("plans");
        assert_eq!(camera.zoom, Camera2D::MAX_ZOOM);
        assert_eq!(camera.position, Vec2::new(0.5, 0.5));

        let vast = Region { w: 1.0e7, ..tiny };
        let refused = refusal(&request(None, Framing::Region(vast), 1.0));
        assert_eq!(refused.param, "region");
        assert!(
            refused.message.contains("larger width and height"),
            "{}",
            refused.message
        );

        for bad in [
            Region { w: 0.0, ..tiny },
            Region { h: -3.0, ..tiny },
            Region { x: f32::NAN, ..tiny },
            Region {
                y: f32::INFINITY,
                ..tiny
            },
        ] {
            assert_eq!(
                refusal(&request(None, Framing::Region(bad), 1.0)).param,
                "region",
                "{bad:?}"
            );
        }
    }

    /// Every word of a console line becomes its part of the request,
    /// and the console is always left out of the picture.
    #[test]
    fn test_a_console_line_becomes_the_same_request() {
        let line = ScreenshotLine {
            path: None,
            size: Some((800, 600)),
            scale: Some(2.0),
            frame: ScreenshotFrame::Region {
                x: 1.0,
                y: 2.0,
                w: 3.0,
                h: 4.0,
                surface: true,
            },
            overwrite: false,
            sidecar: true,
        };
        assert_eq!(
            CaptureRequest::from_console_line(&line),
            CaptureRequest {
                size: Some((800, 600)),
                framing: Framing::Region(Region {
                    x: 1.0,
                    y: 2.0,
                    w: 3.0,
                    h: 4.0,
                    space: Space::Surface
                }),
                scale: 2.0,
                modal: ModalLayer::Omitted,
            }
        );
        let bare = ScreenshotLine {
            size: None,
            scale: None,
            frame: ScreenshotFrame::Fit,
            ..line.clone()
        };
        assert_eq!(
            CaptureRequest::from_console_line(&bare),
            CaptureRequest {
                framing: Framing::Fit,
                modal: ModalLayer::Omitted,
                ..CaptureRequest::default()
            }
        );
        let canvas = ScreenshotLine {
            frame: ScreenshotFrame::Region {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
                surface: false,
            },
            ..line
        };
        assert!(matches!(
            CaptureRequest::from_console_line(&canvas).framing,
            Framing::Region(Region {
                space: Space::Canvas,
                ..
            })
        ));
    }

    /// The bounds `format/ipc.md` states are the bounds the plan
    /// enforces: each figure the document prints is formatted from the
    /// constant here, so changing one without the other fails.
    #[test]
    fn test_the_documented_bounds_are_the_enforced_ones() {
        use baumhard::util::doc_fixtures::{repo_path, section_text};
        let section = section_text(&repo_path("format/ipc.md"), "### `capture.screenshot` (IPC-07)");
        let pixels = MAX_PIXELS as u64;
        let grouped = format!(
            "{},{:03},{:03}",
            pixels / 1_000_000,
            pixels / 1_000 % 1_000,
            pixels % 1_000
        );
        for figure in [
            format!("`1..={MAX_SIDE}`"),
            format!("**{grouped}** pixels"),
            format!("`{:.1}..={:.1}`", SCALE_RANGE.start(), SCALE_RANGE.end()),
            format!("(`{}`)", Camera2D::MIN_ZOOM),
            format!("(`{:.1}`)", Camera2D::MAX_ZOOM),
            format!("{}%", (FIT_PADDING_FRACTION * 100.0).round()),
        ] {
            assert!(
                section.contains(&figure),
                "format/ipc.md §capture.screenshot must state {figure}"
            );
        }
    }

    /// `fit` frames the content as the load-time fit does — padding
    /// included — and needs content to frame.
    #[test]
    fn test_fit_frames_the_content_with_the_load_time_padding() {
        let (min, max) = context().content.expect("content");
        let camera = plan(&request(Some((1000, 1000)), Framing::Fit, 1.0), &context()).expect("plans");
        let mut expected = Camera2D {
            viewport_size: (1000, 1000),
            ..live()
        };
        expected.fit_to_bounds(min, max, FIT_PADDING_FRACTION);
        assert_eq!(camera, expected);

        // Content that would need a zoom below the minimum is refused,
        // as a region is, rather than framed cut off.
        let vast = PlanContext {
            content: Some((Vec2::ZERO, Vec2::new(1.0e7, 10.0))),
            ..context()
        };
        let refused = plan(&request(None, Framing::Fit, 1.0), &vast).expect_err("cut off");
        assert_eq!(refused.param, "fit");
        assert!(
            refused.message.contains("larger width and height"),
            "{}",
            refused.message
        );

        let empty = PlanContext {
            content: None,
            ..context()
        };
        let refused = plan(&request(None, Framing::Fit, 1.0), &empty).expect_err("nothing to fit");
        assert_eq!(refused.param, "fit");
    }
}
