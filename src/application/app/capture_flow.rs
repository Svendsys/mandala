// SPDX-License-Identifier: MPL-2.0

//! The screenshot engine's app half: the one function both the IPC
//! `capture.screenshot` command and the `screenshot` console verb call
//! (`crate::application::capture` holds the rest).
//!
//! **A capture shows the current state, and never writes it.** It first
//! brings the view up to date with the document (`MapView::sync`, the
//! frame's own first step, which does nothing when nothing changed), so
//! an edit the agent sent a moment ago is drawn. It then draws one frame
//! of the glyph layers through a camera of its own
//! (`Renderer::capture_frame`, which encodes the layers at that camera —
//! `LayerRenderer::encode`), at any zoom and any size: the projection is
//! the document's and does not depend on the camera, so a capture framed
//! anywhere needs nothing projected for it. The renderer's camera is
//! never touched. So the camera, the layers and every hit test afterwards
//! are what they were before — not restored, but never written.
//!
//! **And the one-shot launch's capture.** `--screenshot <path>` is the
//! same engine, called by the loop rather than by a command
//! ([`one_shot`]): once the app has settled — the condition
//! `clock.wait until=settled` waits for — it captures the window's view
//! and the loop ends with the code that says whether it was written.
//!
//! Native-only with the engine (`CLAUDE.md` "Dual-target status").

use baumhard::gfx_structs::camera::Camera2D;

use crate::application::capture::artifacts::Artifacts;
use crate::application::capture::plan::{plan, CaptureRequest, Framing, PlanContext};
use crate::application::capture::record::Recorder;
use crate::application::capture::{geometry, resolve, write, CaptureError, CaptureOutcome, Destination};
use crate::application::document::MindMapDocument;
use crate::application::renderer::{CaptureFailure, CapturedPixels, FrameSkip, ModalLayer, Renderer};
use crate::application::view::MapView;

use super::freeze_watchdog::FreezeWatchdog;
use super::input_context::InputHandlerContext;
use super::run_native_init::InitState;
use super::OneShot;
use crate::application::ipc::Until;

/// The app state a capture reads. Borrowed from `InitState` by the IPC
/// target and from the `InputHandlerContext` by the console, so the two
/// surfaces hand the engine the same fields.
///
/// The document, the view and the renderer are `&mut` for the one write
/// a capture makes to the window's state: the view's sync the next frame
/// owes anyway, run first so the capture shows it too. Nothing after that
/// writes them.
pub(super) struct CaptureScene<'a> {
    pub(super) document: &'a mut Option<MindMapDocument>,
    pub(super) view: &'a mut Option<MapView>,
    pub(super) renderer: &'a mut Renderer,
    /// Ticked after every readback band and every encoded chunk: a
    /// large capture is long main-thread work that keeps progressing.
    pub(super) watchdog: &'a FreezeWatchdog,
}

impl<'a> CaptureScene<'a> {
    /// The scene a capture reads out of the event loop's state, with the
    /// two other things a capture or a recording needs from it — the
    /// artifacts dir and the recorder — split off beside it: they are
    /// different fields, so all three are borrowed at once. The one
    /// place the loop's scene is assembled, so a field added to
    /// [`CaptureScene`] is added once.
    pub(super) fn of_init(init: &'a mut InitState) -> (Self, &'a mut Artifacts, &'a mut Recorder) {
        let scene = CaptureScene {
            document: &mut init.document,
            view: &mut init.view,
            renderer: &mut init.renderer,
            watchdog: &init.watchdog,
        };
        (scene, &mut init.artifacts, &mut init.recorder)
    }

    /// The same out of an input handler's context, which the console's
    /// `screenshot` and `record` verbs run in.
    pub(super) fn of_context(
        ctx: &'a mut InputHandlerContext<'_>,
    ) -> (Self, &'a mut Artifacts, &'a mut Recorder) {
        let scene = CaptureScene {
            document: &mut *ctx.document,
            view: &mut *ctx.view,
            renderer: &mut *ctx.renderer,
            watchdog: ctx.watchdog,
        };
        (scene, &mut *ctx.artifacts, &mut *ctx.recorder)
    }
}

/// What drawing a frame produced.
pub(super) enum Drawn {
    /// The frame was drawn: the camera it was drawn through, and its
    /// pixels.
    Frame {
        camera: Camera2D,
        pixels: CapturedPixels,
    },
    /// The renderer did not draw the frame (`rendered: false`).
    Skipped { camera: Camera2D, skip: FrameSkip },
}

/// Take one screenshot: plan `request` and resolve `destination` —
/// both before anything is drawn, so a refused request costs no frame —
/// then draw it and write it, with its geometry sidecar when `sidecar`
/// is set.
///
/// `Err(CaptureError::Internal)` when there is no document is the
/// callers' to pre-empt; both answer it in their own words first.
pub(super) fn capture(
    scene: CaptureScene<'_>,
    request: &CaptureRequest,
    destination: Destination,
    sidecar: bool,
    artifacts: &mut Artifacts,
) -> Result<CaptureOutcome, CaptureError> {
    let CaptureScene {
        document,
        view,
        renderer,
        watchdog,
    } = scene;
    let (target, drawn) = draw(
        CaptureScene {
            document: &mut *document,
            view: &mut *view,
            renderer,
            watchdog,
        },
        request,
        |_| resolve(destination, sidecar, artifacts),
    )?;
    let (camera, pixels) = match drawn {
        Drawn::Frame { camera, pixels } => (camera, pixels),
        Drawn::Skipped { camera, skip } => return Ok(CaptureOutcome::Skipped { camera, skip }),
    };
    let (Some(doc), Some(view)) = (document.as_ref(), view.as_ref()) else {
        return Err(CaptureError::Internal("no document is loaded".into()));
    };
    let map = target.sidecar.is_some().then(|| {
        geometry::sidecar(
            doc.doc(),
            view.projector(),
            view.layers(),
            &camera,
            crate::application::ipc::PROTOCOL_VERSION,
            doc.revision().0,
        )
    });
    let mut progress = || watchdog.tick();
    write(&target, &pixels, map.as_ref(), artifacts, &mut progress)?;
    Ok(CaptureOutcome::Written {
        camera,
        path: target.path,
        sidecar_path: target.sidecar,
    })
}

/// Draw one frame of `request` and hand back its pixels: the half of a
/// capture that screenshots and recordings share. `before_draw` runs
/// once the request is planned and before anything is drawn, with the
/// camera the frame will be drawn through, and its `Err` ends the
/// capture there — a screenshot resolves its destination in it, so a
/// refused path costs no frame, and a recording makes its directory in
/// it, so a refused request leaves none behind.
///
/// `Err(CaptureError::Internal)` when there is no document is the
/// callers' to pre-empt; both answer it in their own words first.
pub(super) fn draw<T>(
    scene: CaptureScene<'_>,
    request: &CaptureRequest,
    before_draw: impl FnOnce(&Camera2D) -> Result<T, CaptureError>,
) -> Result<(T, Drawn), CaptureError> {
    let CaptureScene {
        document,
        view,
        renderer,
        watchdog,
    } = scene;
    let mut progress = || watchdog.tick();
    let (Some(doc), Some(view)) = (document.as_mut(), view.as_mut()) else {
        return Err(CaptureError::Internal("no document is loaded".into()));
    };

    // What the next frame would sync before drawing, synced now so the
    // capture draws it too. The one write to the live state a capture
    // makes, and the frame's own.
    view.sync(doc, renderer);

    let live = *renderer.camera();
    let camera = plan(
        request,
        &PlanContext {
            live,
            max_texture_side: renderer.max_texture_side(),
            // A walk of every run, which only `fit` reads.
            content: match request.framing {
                Framing::Fit => renderer.content_bounds(),
                Framing::View | Framing::Region(_) => None,
            },
        },
    )?;

    let prepared = before_draw(&camera)?;

    // A modal is laid out against the window's size; a capture sized
    // otherwise leaves it out rather than drawing it somewhere arbitrary.
    let modal = match request.size {
        Some(size) if size != live.viewport_size => ModalLayer::Omitted,
        _ => request.modal,
    };
    let drawn = renderer.capture_frame(&camera, modal, &mut progress);

    let drawn = match drawn {
        Ok(pixels) => Drawn::Frame { camera, pixels },
        Err(CaptureFailure::Skipped(skip)) => Drawn::Skipped { camera, skip },
        Err(CaptureFailure::UnsupportedFormat(format)) => {
            return Err(CaptureError::Internal(format!(
                "the window's surface format, {format:?}, is not one a capture can read back"
            )))
        }
        Err(CaptureFailure::Gpu(message)) => return Err(CaptureError::Internal(message)),
    };
    Ok((prepared, drawn))
}

/// A one-shot launch's capture (`--screenshot`), if the app has settled:
/// the window's view at the surface size, with its sidecar, written to
/// the launch's path, and the code the process should end with — `0`
/// written, `1` refused or failed, with the reason on stderr. `None`
/// while the app has not settled, and after a transient skip, whose
/// retry is asked for here: the frame the loop draws for it brings the
/// next check.
///
/// Settled is the `clock.wait until=settled` condition, read at the
/// point that command reads it — and not before the first frame has
/// been rendered, the earliest the window can be said to be on screen.
/// That is all the first frame promises about the window's size: the
/// capture is the window at the size it has then, and a window manager
/// that resizes it after its first expose (a tiling one, typically) is
/// not waited for, since nothing says one is coming. A launch that needs
/// an exact size runs `--headless --size`, where the size is the
/// launch's own. A one-shot launch runs on a virtual clock
/// (`run_native_init::build`), so an animation never keeps it from
/// settling, and the capture is the map as loaded.
///
/// Two ways it ends without settling, both `1`: the startup document is
/// the load-failure placard — a picture of it would be the failure
/// passed off as the map, so nothing is written, and the loader's
/// message is the `startup:` line already on stderr — and a render that
/// stalled (`RedrawLedger::is_stalled`), which will never settle on its
/// own: the capture is tried anyway, and its own skip says why.
pub(super) fn one_shot(init: &mut InitState, shot: &OneShot) -> Option<std::process::ExitCode> {
    use std::process::ExitCode;

    if init
        .document
        .as_ref()
        .is_some_and(MindMapDocument::is_load_failure_placard)
    {
        log::error!(
            "screenshot: the map did not load (the `startup:` line says why); nothing was written to {}",
            baumhard::util::log::tilde_path(&shot.path)
        );
        return Some(ExitCode::FAILURE);
    }
    if !init.redraws.attempted() {
        return None;
    }
    if !init.redraws.is_stalled() && !init.settling().holds(Until::Settled) {
        return None;
    }
    let (scene, artifacts, _) = CaptureScene::of_init(init);
    let destination = Destination::Path {
        path: shot.path.clone(),
        overwrite: shot.overwrite,
    };
    match capture(scene, &CaptureRequest::default(), destination, true, artifacts) {
        Ok(CaptureOutcome::Written { .. }) => Some(ExitCode::SUCCESS),
        // "The next frame may draw it": ask for that frame, whose
        // iteration checks again.
        Ok(CaptureOutcome::Skipped { skip, .. }) if skip.is_transient() => {
            init.request_redraw();
            None
        }
        Ok(CaptureOutcome::Skipped { skip, .. }) => {
            log::error!(
                "screenshot: the frame was not drawn ({}); nothing was written to {}",
                skip.word(),
                baumhard::util::log::tilde_path(&shot.path)
            );
            Some(ExitCode::FAILURE)
        }
        Err(refused) => {
            log::error!(
                "screenshot: {}",
                baumhard::util::log::collapse_home_in(&refused.into_message())
            );
            Some(ExitCode::FAILURE)
        }
    }
}
