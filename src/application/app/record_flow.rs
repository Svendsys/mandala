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
//! that follows is deferred — to loop time that would otherwise be slept
//! through ([`idle`]), to the moment `PENDING_BYTES` of frames wait, or
//! to the end of the recording — and ticks the watchdog per chunk; a frame that
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
    dir_bytes, make_dir, MadeDir, RecordConfig, RecordDestination, Recorder, Recording, Started, Step,
    Stopped, Summary,
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
            "a recording is already running into `{}`; stop it first (capture.record_stop, or `record stop` in the console)",
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
    let (t_ms, due_ms) = match recording.step(now) {
        Step::Capture { t_ms, due_ms } => (t_ms, due_ms),
        // Not reachable: a new timeline owes its first frame at once.
        Step::Wait { .. } | Step::Full => (0.0, 0.0),
    };
    file(&mut recording, t_ms, due_ms, revision, drawn);
    let started = Started {
        dir: recording.dir().to_path_buf(),
        manifest_path: recording.manifest_path(),
        width,
        height,
        fps,
        max_frames,
        // The report of a recording that ended on its own and was never
        // collected is replaced by this one; it is handed back instead.
        previous: recorder.finished.take(),
    };
    recorder.active = Some(recording);
    // A first frame can already be the last of a one-frame recording.
    settle(recorder, artifacts, watchdog, now);
    Ok(started)
}

/// File what a draw produced: a frame taken at `t_ms`, or the gap the
/// renderer's skip leaves where the frame due at `due_ms` should be.
fn file(recording: &mut Recording, t_ms: f64, due_ms: f64, revision: u64, drawn: Drawn) {
    match drawn {
        Drawn::Frame { camera, pixels } => {
            recording.push_frame(t_ms, revision, CameraInfo::of(&camera), pixels)
        }
        Drawn::Skipped { skip, .. } => recording.push_unrendered(due_ms, skip.word()),
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
        Step::Capture { t_ms, due_ms } => match capture_flow::draw(scene, &request, |_| Ok(())) {
            Ok(((), drawn)) => file(active, t_ms, due_ms, revision, drawn),
            // A drag in flight makes a frame at another zoom
            // undrawable (`capture_flow::draw`); the drag ends.
            Err(CaptureError::Busy(_)) => active.push_busy(due_ms),
            // The request could not be framed *this time* — the document
            // lost what `fit` frames, a surface region needs a zoom the
            // live camera since left. A gap, counted toward the stall, so
            // a framing that never works ends the recording and one that
            // works again resumes it.
            Err(CaptureError::Invalid { param, message }) => {
                log::warn!("record: a frame could not be framed ({param}): {message}");
                active.push_unrendered(due_ms, "invalid");
            }
            Err(CaptureError::Internal(message)) => {
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
    // However it ended, what it wrote counts against the budget.
    if in_artifacts {
        artifacts.record_tree(dir.clone(), dir_bytes(&dir));
    }
    recorder.finished = Some(match (finished, failure) {
        (Ok(summary), None) => Ok(summary),
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
/// and manifest are on disk. A recording in the artifacts dir is
/// dropped unfinished, since the dir goes with the exit.
pub(super) fn shutdown(recorder: &mut Recorder, artifacts: &mut Artifacts, watchdog: &FreezeWatchdog) {
    let Some(active) = recorder.active.as_ref() else {
        return;
    };
    // The artifacts dir is removed whole right after this, so finishing a
    // recording in it would encode the frames that wait and write a
    // manifest only to delete them: the exit would stall on work nobody
    // can read.
    if active.in_artifacts() {
        log::info!("record: dropping the recording in the artifacts dir on exit");
        recorder.active = None;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::capture::artifacts::BUDGET_BYTES;
    use crate::application::capture::geometry::CameraInfo;
    use crate::application::capture::record::MANIFEST_FILE;
    use crate::application::guarded_file::tests::scratch_dir;
    use crate::application::renderer::CapturedPixels;

    /// A watchdog that will not abort the test process: parked, so the
    /// silence between tests is idle and not a freeze.
    fn watchdog() -> FreezeWatchdog {
        let watchdog = FreezeWatchdog::spawn();
        watchdog.parked();
        watchdog
    }

    fn config(max_frames: u32) -> RecordConfig {
        RecordConfig {
            fps: 10,
            max_frames,
            width: 2,
            height: 1,
            scale: 1.0,
            request: CaptureRequest::default(),
        }
    }

    /// A recording in `dir` holding `frames` frames, as the loop leaves
    /// one mid-run.
    fn recording(
        dir: std::path::PathBuf,
        frames: u32,
        max_frames: u32,
        disk_limit: Option<u64>,
    ) -> Recording {
        let mut recording = Recording::begin(config(max_frames), dir, now_ms(), disk_limit);
        for n in 0..frames {
            recording.push_frame(
                f64::from(n) * 100.0,
                1,
                CameraInfo {
                    center_x: 0.0,
                    center_y: 0.0,
                    zoom: 1.0,
                },
                CapturedPixels {
                    width: 2,
                    height: 1,
                    rgba: vec![n as u8, 0, 0, 255, 0, n as u8, 0, 255],
                },
            );
        }
        recording
    }

    /// A recorder with `active` recording.
    fn recorder_with(active: Recording) -> Recorder {
        Recorder {
            active: Some(active),
            finished: None,
        }
    }

    fn manifest(dir: &std::path::Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(dir.join(MANIFEST_FILE)).expect("manifest")).expect("JSON")
    }

    /// Nothing recording is the caller's mistake, said as such; a stop
    /// ends the recording and reports it; a second stop has nothing to
    /// stop.
    #[test]
    fn test_stop_ends_the_recording_and_a_second_stop_has_nothing_to_end() {
        let dir = scratch_dir("flow-stop");
        let mut artifacts = Artifacts::under(dir.join("unused"), BUDGET_BYTES);
        let watchdog = watchdog();
        let mut recorder = Recorder::default();
        let idle = stop(&mut recorder, &mut artifacts, &watchdog);
        assert!(
            matches!(&idle, Err(CaptureError::Invalid { param: "cmd", message }) if message.contains("nothing is recording")),
            "{idle:?}"
        );

        recorder.active = Some(recording(dir.join("rec"), 2, 10, None));
        std::fs::create_dir(dir.join("rec")).expect("dir");
        let summary = stop(&mut recorder, &mut artifacts, &watchdog).expect("stopped");
        assert_eq!((summary.frames, summary.stopped), (2, Stopped::Client));
        assert!(recorder.active.is_none() && recorder.finished.is_none());
        assert_eq!(manifest(&dir.join("rec"))["stopped"], "client");
        assert!(stop(&mut recorder, &mut artifacts, &watchdog).is_err());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// A recording that reached its `max_frames` ends on its own, and the
    /// stop that comes after collects what happened: why, and where.
    #[test]
    fn test_a_recording_that_ended_on_its_own_is_collected_by_the_next_stop() {
        let dir = scratch_dir("flow-self");
        std::fs::create_dir(dir.join("rec")).expect("dir");
        let mut artifacts = Artifacts::under(dir.join("unused"), BUDGET_BYTES);
        let watchdog = watchdog();
        let mut recorder = recorder_with(recording(dir.join("rec"), 2, 2, None));
        settle(&mut recorder, &mut artifacts, &watchdog, now_ms());
        assert!(recorder.active.is_none(), "it had its frames");
        assert!(matches!(&recorder.finished, Some(Ok(s)) if s.stopped == Stopped::MaxFrames));
        let summary = stop(&mut recorder, &mut artifacts, &watchdog).expect("collected");
        assert_eq!((summary.frames, summary.stopped), (2, Stopped::MaxFrames));
        assert_eq!(manifest(&dir.join("rec"))["stopped"], "max_frames");
        assert!(recorder.finished.is_none(), "collected once");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// A recording that cannot be written ends with the failure, which
    /// the stop reports as `internal` naming the directory whose frames
    /// are usable; the slot is clear afterwards, and the manifest says
    /// how it ended.
    #[test]
    fn test_a_recording_that_cannot_be_written_reports_the_failure_once() {
        let dir = scratch_dir("flow-fail");
        std::fs::create_dir(dir.join("rec")).expect("dir");
        std::fs::write(dir.join("rec").join("frame_00001.png"), "in the way").expect("seed");
        let mut artifacts = Artifacts::under(dir.join("unused"), BUDGET_BYTES);
        let watchdog = watchdog();
        let mut recorder = recorder_with(recording(dir.join("rec"), 3, 10, None));
        let failed = stop(&mut recorder, &mut artifacts, &watchdog);
        assert!(
            matches!(&failed, Err(CaptureError::Internal(m)) if m.contains("could not be finished") && m.contains("rec")),
            "{failed:?}"
        );
        assert!(recorder.active.is_none() && recorder.finished.is_none());
        assert_eq!(manifest(&dir.join("rec"))["stopped"], "error");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// The app closing finishes the recording: its frames and manifest
    /// are on disk, saying so, and nothing is left for a stop to collect.
    #[test]
    fn test_shutdown_finishes_the_recording_in_progress() {
        let dir = scratch_dir("flow-shutdown");
        std::fs::create_dir(dir.join("rec")).expect("dir");
        let mut artifacts = Artifacts::under(dir.join("unused"), BUDGET_BYTES);
        let watchdog = watchdog();
        let mut recorder = Recorder::default();
        shutdown(&mut recorder, &mut artifacts, &watchdog);
        recorder.active = Some(recording(dir.join("rec"), 2, 10, None));
        shutdown(&mut recorder, &mut artifacts, &watchdog);
        let manifest = manifest(&dir.join("rec"));
        assert_eq!(
            (manifest["stopped"].as_str(), manifest["complete"].as_bool()),
            (Some("shutdown"), Some(true))
        );
        assert!(dir.join("rec").join("frame_00001.png").exists());
        assert!(recorder.active.is_none() && recorder.finished.is_none());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// However a recording in the artifacts dir ends — stopped, or
    /// failed — what it wrote is counted against the budget, so the next
    /// capture can evict it; and one past the budget ends itself.
    #[test]
    fn test_a_recording_in_the_artifacts_dir_is_counted_however_it_ends() {
        let root = scratch_dir("flow-artifacts");
        let mut artifacts = Artifacts::under(root.clone(), BUDGET_BYTES);
        let watchdog = watchdog();
        let mut recorder = Recorder::default();
        let rec = artifacts.next_dir("recording").expect("dir");
        recorder.active = Some(recording(rec.clone(), 2, 10, Some(BUDGET_BYTES)));
        stop(&mut recorder, &mut artifacts, &watchdog).expect("stopped");
        assert_eq!(
            artifacts.total(),
            crate::application::capture::record::dir_bytes(&rec)
        );
        assert!(artifacts.total() > 0);

        // Failed: the frames that landed still count.
        let bad = artifacts.next_dir("recording").expect("dir");
        std::fs::write(bad.join("frame_00001.png"), "in the way").expect("seed");
        let before = artifacts.total();
        recorder.active = Some(recording(bad.clone(), 3, 10, Some(BUDGET_BYTES)));
        assert!(stop(&mut recorder, &mut artifacts, &watchdog).is_err());
        assert!(artifacts.total() > before);

        // Past the budget: a recording whose PNGs exceed the limit ends
        // by itself on the next look.
        let big = artifacts.next_dir("recording").expect("dir");
        let mut over = recording(big, 1, 10, Some(1));
        over.flush(now_ms(), &mut || {}).expect("flushed");
        recorder.active = Some(over);
        settle(&mut recorder, &mut artifacts, &watchdog, now_ms());
        assert!(matches!(&recorder.finished, Some(Ok(s)) if s.stopped == Stopped::DiskBudget));
        artifacts.remove();
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    /// On exit a recording in the artifacts dir is dropped, not finished:
    /// the dir is removed right after, so encoding the waiting frames
    /// would be seconds of work for files nobody can read.
    #[test]
    fn test_shutdown_drops_a_recording_in_the_artifacts_dir_unfinished() {
        let root = scratch_dir("flow-shutdown-artifacts");
        let mut artifacts = Artifacts::under(root.clone(), BUDGET_BYTES);
        let watchdog = watchdog();
        let rec = artifacts.next_dir("recording").expect("dir");
        let mut recorder = recorder_with(recording(rec.clone(), 3, 10, Some(BUDGET_BYTES)));
        shutdown(&mut recorder, &mut artifacts, &watchdog);
        assert!(recorder.active.is_none() && recorder.finished.is_none());
        assert_eq!(
            crate::application::guarded_file::tests::names_in(&rec),
            Vec::<String>::new(),
            "nothing was encoded and no manifest written"
        );
        artifacts.remove();
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
