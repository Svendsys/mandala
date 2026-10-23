// SPDX-License-Identifier: MPL-2.0

//! The recorder's app half: starting a recording, taking its frames as
//! the loop's heartbeat comes due, and ending it. The pure half —
//! cadence, gaps, the manifest, the files — is
//! `crate::application::capture::record`; each frame is drawn by the
//! same `capture_flow::draw` a screenshot is, so a recording shows what
//! a screenshot at the same parameters would.
//!
//! **A recording is app state, not session state.** It lives on
//! `InitState`, keeps taking frames across a client's disconnect, and is
//! ended by `capture.record_stop` from whoever connects next, by
//! reaching its `max_frames` or its disk budget, or by the app closing
//! ([`shutdown`]). A recording that ended on its own is kept as
//! `finished` until a stop collects it, so a client that looked away
//! still learns where its frames are.
//!
//! **The loop is told when to wake.** On an otherwise static scene the
//! loop parks and no heartbeat would come; [`deadline`] is the instant
//! `about_to_wait` parks until instead, the same idle-timer shape the
//! FPS grace uses, so frames land at the requested cadence with the
//! canvas idle.
//!
//! **Nothing here can wedge the loop.** A frame is one render and one
//! readback, ticked into the watchdog as a screenshot's are; the encode
//! that follows is deferred until `PENDING_BYTES` of frames wait (or the
//! recording ends) and ticks the watchdog per chunk; a frame that
//! cannot be taken is a gap in the manifest, not an error, and a
//! recording that can no longer be written ends itself with the reason.
//!
//! Native-only with the engine (`CLAUDE.md` "Dual-target status",
//! "Screenshot capture").

use std::time::Duration;

use web_time::Instant;

use crate::application::capture::artifacts::{Artifacts, BUDGET_BYTES};
use crate::application::capture::geometry::CameraInfo;
use crate::application::capture::plan::CaptureRequest;
use crate::application::capture::record::{
    make_dir, MadeDir, RecordConfig, RecordDestination, Recorder, Recording, Started, Step, Stopped, Summary,
};
use crate::application::capture::CaptureError;
use crate::application::common::now_ms;

use super::capture_flow::{self, CaptureScene, Drawn};
use super::freeze_watchdog::FreezeWatchdog;

/// The refusal for a state the recorder is not in.
fn state_refusal(message: String) -> CaptureError {
    CaptureError::Invalid {
        param: "cmd",
        message,
    }
}

/// Begin a recording of `request` at `fps`, into `destination`.
///
/// The request is planned and the directory checked and made before the
/// first frame is drawn, so a refused one costs no frame and leaves no
/// directory behind. The first frame is taken here, at once, so it
/// shows the state the command that follows this one acts on — not
/// whatever the next heartbeat finds. A first frame the renderer cannot
/// draw is a gap like any other, and the recording begins regardless.
///
/// `request.size` of `None` is the window's size now: the frames of one
/// recording are all the same size, which a video needs and a window
/// that is resized mid-recording must not change.
pub(super) fn start(
    scene: CaptureScene<'_>,
    recorder: &mut Recorder,
    artifacts: &mut Artifacts,
    request: &CaptureRequest,
    fps: u32,
    max_frames: u32,
    destination: RecordDestination,
) -> Result<Started, CaptureError> {
    if let Some(active) = recorder.active.as_ref() {
        return Err(state_refusal(format!(
            "a recording is already running into `{}`; stop it with capture.record_stop first",
            active.dir().display()
        )));
    }
    let mut request = *request;
    let live_size = scene.renderer.camera().viewport_size;
    request.size.get_or_insert(live_size);
    let watchdog = scene.watchdog;
    let revision = scene.document.as_ref().map_or(0, |doc| doc.revision());
    let now = now_ms();

    let mut made: Option<MadeDir> = None;
    let drawn = capture_flow::draw(scene, &request, |_| {
        let dir = make_dir(destination, artifacts)?;
        made = Some(dir.clone());
        Ok(dir)
    });
    let (dir, drawn) = match drawn {
        Ok(drawn) => drawn,
        Err(error) => {
            // `remove_dir` takes a directory only while it is empty, so
            // this cannot take anything that was not made for this.
            if let Some(dir) = made.filter(|dir| dir.created) {
                let _ = std::fs::remove_dir(&dir.path);
            }
            return Err(error);
        }
    };

    let camera = match &drawn {
        Drawn::Frame { camera, .. } | Drawn::Skipped { camera, .. } => *camera,
    };
    let (width, height) = camera.viewport_size;
    let config = RecordConfig {
        fps,
        max_frames,
        width,
        height,
        scale: camera.pixel_ratio,
        request,
    };
    let disk_limit = dir.in_artifacts.then_some(BUDGET_BYTES);
    let mut recording = Recording::begin(config, dir.path, now, disk_limit);
    // The timeline's first boundary is at 0 and so is owed now.
    let t_ms = match recording.step(now) {
        Step::Capture { t_ms } => t_ms,
        // Not reachable: a new timeline owes its first frame at once.
        Step::Wait { .. } | Step::Full => 0.0,
    };
    file(&mut recording, t_ms, revision, drawn);
    let started = Started {
        dir: recording.dir().to_path_buf(),
        manifest_path: recording.manifest_path(),
        width,
        height,
        fps,
        max_frames,
    };
    recorder.finished = None;
    recorder.active = Some(recording);
    // A first frame can already be the last of a one-frame recording.
    settle(recorder, artifacts, watchdog, now);
    Ok(started)
}

/// File what a draw produced: a frame, or the gap the renderer's skip
/// leaves.
fn file(recording: &mut Recording, t_ms: f64, revision: u64, drawn: Drawn) {
    match drawn {
        Drawn::Frame { camera, pixels } => {
            recording.push_frame(t_ms, revision, CameraInfo::of(&camera), pixels)
        }
        Drawn::Skipped { skip, .. } => recording.push_unrendered(t_ms, skip.word()),
    }
}

/// The heartbeat: if a frame is due, take it. Called once per loop
/// iteration while a recording runs; a no-op otherwise.
pub(super) fn tick(scene: CaptureScene<'_>, recorder: &mut Recorder, artifacts: &mut Artifacts) {
    let Some(active) = recorder.active.as_mut() else {
        return;
    };
    let watchdog = scene.watchdog;
    let revision = scene.document.as_ref().map_or(0, |doc| doc.revision());
    let now = now_ms();
    let request = active.config().request;
    match active.step(now) {
        Step::Wait { .. } => return,
        Step::Full => {}
        Step::Capture { t_ms } => match capture_flow::draw(scene, &request, |_| Ok(())) {
            Ok(((), drawn)) => file(active, t_ms, revision, drawn),
            // A drag in flight makes a frame at another zoom
            // undrawable (`capture_flow::draw`); the drag ends.
            Err(CaptureError::Busy(_)) => active.push_unrendered(t_ms, "busy"),
            Err(CaptureError::Invalid { message, .. } | CaptureError::Internal(message)) => {
                log::warn!("record: a frame could not be taken: {message}");
                end(recorder, artifacts, watchdog, Stopped::Error, Some(message));
                return;
            }
        },
    }
    settle(recorder, artifacts, watchdog, now);
}

/// The loop is about to sleep until the next frame is due: spend the
/// time on encoding the frames that wait, as far as it reaches
/// (`Recording::encode_in_idle`), so `capture.record_stop` has less left
/// to do. Called only when the loop has nothing else to do — an
/// animating window needs every millisecond of its heartbeat — and
/// never encodes past the next boundary.
pub(super) fn idle(recorder: &mut Recorder, artifacts: &mut Artifacts, watchdog: &FreezeWatchdog) {
    let Some(active) = recorder.active.as_mut() else {
        return;
    };
    let now = now_ms();
    if let Err(message) = active.encode_in_idle(active.wait_ms(now), now, &mut || watchdog.tick()) {
        log::warn!("record: the frames could not be written: {message}");
        end(recorder, artifacts, watchdog, Stopped::Error, Some(message));
        return;
    }
    settle(recorder, artifacts, watchdog, now);
}

/// Look at the active recording after a frame: flush what waits if
/// enough does, and end it if it is full, over its disk budget, or can
/// draw nothing.
fn settle(recorder: &mut Recorder, artifacts: &mut Artifacts, watchdog: &FreezeWatchdog, now: f64) {
    let Some(active) = recorder.active.as_mut() else {
        return;
    };
    if active.wants_flush() {
        if let Err(message) = active.flush(now, &mut || watchdog.tick()) {
            log::warn!("record: the frames could not be written: {message}");
            end(recorder, artifacts, watchdog, Stopped::Error, Some(message));
            return;
        }
    }
    let Some(active) = recorder.active.as_ref() else {
        return;
    };
    let ending = if active.over_disk_limit() {
        Some((
            Stopped::DiskBudget,
            "the recording's frames passed the artifacts dir's budget",
        ))
    } else if active.is_full() {
        Some((Stopped::MaxFrames, "the recording took its max_frames"))
    } else if active.is_stalled() {
        Some((
            Stopped::Error,
            "the renderer drew nothing for a hundred frames in a row",
        ))
    } else {
        None
    };
    if let Some((stopped, why)) = ending {
        log::warn!("record: {why}; stopping");
        let message = (stopped == Stopped::Error).then(|| why.to_string());
        end(recorder, artifacts, watchdog, stopped, message);
    }
}

/// End the active recording as `stopped`: flush, write the manifest,
/// count a recording in the artifacts dir into its budget, and keep the
/// outcome for the stop that collects it. `failure` is the message a
/// `Stopped::Error` ends with, when the cause is not a failed write.
fn end(
    recorder: &mut Recorder,
    artifacts: &mut Artifacts,
    watchdog: &FreezeWatchdog,
    stopped: Stopped,
    failure: Option<String>,
) {
    let Some(active) = recorder.active.take() else {
        return;
    };
    let in_artifacts = active.in_artifacts();
    let dir = active.dir().to_path_buf();
    let finished = active.finish(stopped, now_ms(), &mut || watchdog.tick());
    recorder.finished = Some(match (finished, failure) {
        (Ok(summary), None) => {
            if in_artifacts {
                artifacts.record_tree(dir, summary.bytes);
            }
            Ok(summary)
        }
        // The frames that landed are on disk with a manifest; the cause
        // is what the stop reports.
        (Ok(_), Some(message)) => Err(format!(
            "the recording ended early: {message}; the frames taken so far are in `{}`",
            dir.display()
        )),
        (Err(message), _) => Err(format!(
            "the recording could not be finished: {message}; what was written is in `{}`",
            dir.display()
        )),
    });
}

/// `capture.record_stop`: end the recording and report it — or, when it
/// ended on its own since the client last looked, report that.
pub(super) fn stop(
    recorder: &mut Recorder,
    artifacts: &mut Artifacts,
    watchdog: &FreezeWatchdog,
) -> Result<Summary, CaptureError> {
    end(recorder, artifacts, watchdog, Stopped::Client, None);
    match recorder.finished.take() {
        Some(Ok(summary)) => Ok(summary),
        Some(Err(message)) => Err(CaptureError::Internal(message)),
        None => Err(state_refusal("nothing is recording".into())),
    }
}

/// The app is closing: finish the recording in progress so its frames
/// and manifest are on disk. A recording in the artifacts dir goes with
/// the dir.
pub(super) fn shutdown(recorder: &mut Recorder, artifacts: &mut Artifacts, watchdog: &FreezeWatchdog) {
    if recorder.active.is_none() {
        return;
    }
    end(recorder, artifacts, watchdog, Stopped::Shutdown, None);
    match recorder.finished.take() {
        Some(Ok(summary)) => log::info!(
            "record: finished {} frames into `{}` on exit",
            summary.frames,
            summary.dir.display()
        ),
        Some(Err(message)) => log::warn!("record: {message}"),
        None => {}
    }
}

/// When the loop must wake to take the next frame, or `None` when
/// nothing is recording.
pub(super) fn deadline(recorder: &Recorder) -> Option<Instant> {
    let active = recorder.active.as_ref()?;
    let wait = Duration::from_secs_f64(active.wait_ms(now_ms()) / 1000.0);
    Some(Instant::now() + wait)
}
