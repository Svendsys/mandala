// SPDX-License-Identifier: MPL-2.0

//! Frame-sequence recording: the engine behind the IPC
//! `capture.record_start` / `capture.record_stop` commands and the
//! `record` console verb (`format/ipc.md`
//! §`capture.record_start` / `capture.record_stop`; CONCEPTS §5
//! "Frame-sequence recording").
//!
//! **A recording is a sequence of screenshots and a manifest.** Each
//! frame is drawn by the same engine a screenshot is
//! (`app::capture_flow::draw`), then held in memory as raw pixels and
//! encoded to `frame_%05d.png` in the recording's directory only when
//! [`PENDING_BYTES`] of them have piled up, and when the recording
//! ends. A frame costs the loop one render and one readback, never an
//! encode: "encoding after `stop`, not during capture, keeps per-frame
//! cost to a texture copy." `manifest.json` is the contract with
//! whatever assembles the frames (`mandalactl record assemble`, or any
//! tool that reads JSON).
//!
//! **Pacing is a pure function of the clock.** [`Timeline`] decides,
//! from the elapsed time alone, when the next frame is due and how
//! many boundaries a late heartbeat stepped over. It never reads a
//! clock itself — the caller passes the time — so the realtime pacing
//! tested here is the pacing a virtual clock (IPC-09) will reuse by
//! passing virtual time instead, and a manifest written under it is
//! the same shape ([`PACING_REALTIME`] is the one value its `pacing`
//! key has today).
//!
//! **Honest about what it missed.** A boundary the loop was too late
//! for, a frame the renderer did not draw and a frame a node drag made
//! undrawable are *gaps*: counted in `dropped_frames` and listed in
//! `gaps` with their reason, never silently absent. Frame timestamps
//! are the clock's reading when the frame was taken, so a consumer
//! that paces by them replays what happened.
//!
//! **Bounded.** `max_frames` (hard cap [`MAX_FRAMES`]) ends a recording
//! that nobody stopped; raw frames waiting for their encode are held to
//! [`PENDING_BYTES`]; and a recording in the server-chosen artifacts
//! dir ends when its PNGs pass that dir's budget. Each end is reported
//! as `stopped`, in the reply and the manifest.
//!
//! Native-only with the engine (`CLAUDE.md` "Dual-target status",
//! "Screenshot capture").

use std::collections::VecDeque;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use crate::application::guarded_file::{self, PublishRefusal};
use crate::application::renderer::CapturedPixels;

use super::artifacts::Artifacts;
use super::geometry::CameraInfo;
use super::plan::CaptureRequest;
use super::{png, CaptureError};

/// The frame rate a recording runs at when it names none.
pub(crate) const DEFAULT_FPS: u32 = 30;

/// The frame rates a recording accepts.
pub(crate) const FPS_RANGE: RangeInclusive<u32> = 1..=60;

/// How many frames a recording takes before it ends itself, when it
/// names no limit.
pub(crate) const DEFAULT_MAX_FRAMES: u32 = 600;

/// The most frames one recording may take: the largest `max_frames`
/// accepted. Five digits name every frame.
pub(crate) const MAX_FRAMES: u32 = 3600;

/// How many bytes of raw frames wait in memory for their encode before
/// the recording writes them out and carries on. An encode is a stall
/// the loop pays whenever it comes; this bounds what one stall
/// holds, and the memory a long recording holds, at the same number.
pub(crate) const PENDING_BYTES: u64 = 256 << 20;

/// How many frames in a row the renderer may fail to draw before the
/// recording gives up. A skip is usually a moment's contention; a run of
/// them this long is not going to clear, and a recording that can
/// never reach `max_frames` would otherwise never end.
pub(crate) const MAX_UNRENDERED_STREAK: u32 = 100;

/// How much headroom an idle-time encode wants: it runs only when the
/// time before the next frame is due is at least this many times the
/// measured cost of encoding one frame, so a misjudged encode does not
/// make the next boundary late.
const IDLE_ENCODE_MARGIN: f64 = 1.5;

/// The manifest's file name, in the recording's directory.
pub(crate) const MANIFEST_FILE: &str = "manifest.json";

/// The manifest's `pacing` when frame times are the wall clock's.
pub(crate) const PACING_REALTIME: &str = "realtime";

/// The file name of frame `index`.
pub(crate) fn frame_file(index: u32) -> String {
    format!("frame_{index:05}.png")
}

/// Why a recording ended. The words are the wire's `stopped`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stopped {
    /// `capture.record_stop`, or the console's `record stop`.
    Client,
    /// It took `max_frames` frames.
    MaxFrames,
    /// Its PNGs passed the artifacts dir's budget.
    DiskBudget,
    /// The app was closing. Only the manifest says so: the app is gone
    /// before any command could ask for the report.
    Shutdown,
    /// A write failed; the message went to the stop that collected it.
    Error,
}

impl Stopped {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Stopped::Client => "client",
            Stopped::MaxFrames => "max_frames",
            Stopped::DiskBudget => "disk_budget",
            Stopped::Shutdown => "shutdown",
            Stopped::Error => "error",
        }
    }
}

/// When the next frame is due, from the elapsed time alone.
///
/// Boundaries are the multiples of `1000 / fps` ms from the start,
/// counted as whole numbers so that sixty seconds of thirty-frames-a-
/// second is exactly 1800 of them whatever the float rounding does.
/// One frame is taken per poll that finds a boundary crossed, stamped
/// by the caller with the time it was taken; boundaries a poll finds
/// stepped over entirely are `missed`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Timeline {
    fps: u32,
    /// The index of the next boundary a frame is owed for.
    next: u64,
}

/// What [`Timeline::poll`] found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Tick {
    /// No boundary has been crossed; the next is at `until_ms`.
    Wait { until_ms: f64 },
    /// Take a frame now, for the latest boundary crossed, at
    /// `boundary_ms`. `missed` boundaries before it were stepped over
    /// without one, the first at `first_missed_ms`.
    Due {
        missed: u32,
        first_missed_ms: f64,
        boundary_ms: f64,
    },
}

impl Timeline {
    /// A timeline whose first boundary, at 0, is already due.
    pub(crate) fn new(fps: u32) -> Self {
        Timeline {
            fps: fps.max(1),
            next: 0,
        }
    }

    /// The time of boundary `index`, in ms from the start.
    fn boundary_ms(&self, index: u64) -> f64 {
        index as f64 * 1000.0 / f64::from(self.fps)
    }

    /// The time the next frame is due, in ms from the start.
    pub(crate) fn due_ms(&self) -> f64 {
        self.boundary_ms(self.next)
    }

    /// `elapsed_ms` from the start: is a frame owed?
    pub(crate) fn poll(&mut self, elapsed_ms: f64) -> Tick {
        let due = self.due_ms();
        // A clock that reads before the start (or NaN) owes nothing.
        if elapsed_ms.is_nan() || elapsed_ms < due {
            return Tick::Wait { until_ms: due };
        }
        // The latest boundary at or before now. The product is exact for
        // the integer `fps` and the times a recording lives for, so a
        // reading that is exactly a boundary lands on it.
        let latest = ((elapsed_ms * f64::from(self.fps) / 1000.0).floor() as u64).max(self.next);
        let first_missed_ms = self.boundary_ms(self.next);
        let missed = u32::try_from(latest - self.next).unwrap_or(u32::MAX);
        self.next = latest + 1;
        Tick::Due {
            missed,
            first_missed_ms,
            boundary_ms: self.boundary_ms(latest),
        }
    }
}

/// What a recording is asked to be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RecordConfig {
    pub(crate) fps: u32,
    pub(crate) max_frames: u32,
    /// The frames' size in pixels, fixed by the first frame's plan.
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Image pixels per view pixel.
    pub(crate) scale: f32,
    /// What every frame frames, its size already pinned.
    pub(crate) request: CaptureRequest,
}

/// One frame in the manifest.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct FrameEntry {
    pub(crate) index: u32,
    pub(crate) t_ms: f64,
    pub(crate) file: String,
    pub(crate) document_revision: u64,
    pub(crate) camera: CameraInfo,
}

/// A stretch of the recording with no frame in it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Gap {
    /// When the first missing frame was due.
    pub(crate) t_ms: f64,
    /// How many frames are missing.
    pub(crate) frames: u32,
    /// `overrun` (the loop was late for the boundary), `busy` (a node
    /// drag made the frame undrawable), or a renderer skip word
    /// (`font_system_contended`, `not_started`, `prepare_failed`).
    pub(crate) reason: String,
}

/// The manifest, as written.
#[derive(Serialize)]
struct Manifest<'a> {
    protocol: u32,
    fps: u32,
    pacing: &'static str,
    width: u32,
    height: u32,
    #[serde(serialize_with = "baumhard::util::shortest_float::serialize_shortest")]
    scale: f32,
    /// False in the copy written mid-recording, so a recording the app
    /// never finished is still told from one it did.
    complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stopped: Option<Stopped>,
    duration_ms: f64,
    frames: &'a [FrameEntry],
    dropped_frames: u32,
    gaps: &'a [Gap],
}

/// What a recording that began is called back.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Started {
    pub(crate) dir: PathBuf,
    /// Where the manifest will be: written whenever frames are, and
    /// complete when the recording ends.
    pub(crate) manifest_path: PathBuf,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps: u32,
    pub(crate) max_frames: u32,
}

/// What a finished recording amounts to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Summary {
    pub(crate) dir: PathBuf,
    pub(crate) manifest_path: PathBuf,
    pub(crate) frames: u32,
    pub(crate) dropped_frames: u32,
    pub(crate) duration_ms: f64,
    pub(crate) stopped: Stopped,
    pub(crate) bytes: u64,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps: u32,
}

/// Milliseconds, to a microsecond: the manifest's times are read by
/// people and compared by tests, and a float's tail is neither.
fn round_ms(ms: f64) -> f64 {
    (ms * 1000.0).round() / 1000.0
}

/// A frame drawn and waiting for its encode.
struct Pending {
    index: u32,
    pixels: CapturedPixels,
}

/// A recording in progress.
pub(crate) struct Recording {
    config: RecordConfig,
    dir: PathBuf,
    /// The clock's reading when the recording began; frame times are
    /// measured from it.
    started_ms: f64,
    timeline: Timeline,
    frames: Vec<FrameEntry>,
    gaps: Vec<Gap>,
    pending: VecDeque<Pending>,
    pending_bytes: u64,
    /// What encoding one frame costs, in ms, smoothed over the ones
    /// measured; `None` before the first.
    encode_ms: Option<f64>,
    written_bytes: u64,
    /// The size of the manifest as last written.
    manifest_bytes: u64,
    /// Frames in a row the renderer did not draw ([`MAX_UNRENDERED_STREAK`]).
    unrendered_streak: u32,
    /// Where the PNGs may not pass, for a recording in the artifacts
    /// dir: that dir's budget.
    disk_limit: Option<u64>,
}

/// What the recorder wants of the loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Step {
    /// Nothing is due; the next frame is `in_ms` away.
    Wait { in_ms: f64 },
    /// Take a frame now, stamped `t_ms`, for the boundary due at
    /// `due_ms` (where a gap in its place is filed).
    Capture { t_ms: f64, due_ms: f64 },
    /// The recording has its `max_frames`.
    Full,
}

impl Recording {
    /// Begin a recording into `dir` (made already) at the clock's
    /// reading `now_ms`. `disk_limit` bounds the PNGs written, for a
    /// recording in the artifacts dir.
    pub(crate) fn begin(config: RecordConfig, dir: PathBuf, now_ms: f64, disk_limit: Option<u64>) -> Self {
        Recording {
            timeline: Timeline::new(config.fps),
            config,
            dir,
            started_ms: now_ms,
            frames: Vec::new(),
            gaps: Vec::new(),
            pending: VecDeque::new(),
            pending_bytes: 0,
            encode_ms: None,
            written_bytes: 0,
            manifest_bytes: 0,
            unrendered_streak: 0,
            disk_limit,
        }
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn config(&self) -> &RecordConfig {
        &self.config
    }

    pub(crate) fn manifest_path(&self) -> PathBuf {
        self.dir.join(MANIFEST_FILE)
    }

    /// Whether a frame is owed at the clock's reading `now_ms`, and the
    /// boundaries it stepped over (recorded as an `overrun` gap here).
    pub(crate) fn step(&mut self, now_ms: f64) -> Step {
        if self.frames.len() >= self.config.max_frames as usize {
            return Step::Full;
        }
        let elapsed = (now_ms - self.started_ms).max(0.0);
        match self.timeline.poll(elapsed) {
            Tick::Wait { until_ms } => Step::Wait {
                in_ms: (until_ms - elapsed).max(0.0),
            },
            Tick::Due {
                missed,
                first_missed_ms,
                boundary_ms,
            } => {
                if missed > 0 {
                    self.push_gap(first_missed_ms, missed, "overrun".into());
                }
                Step::Capture {
                    t_ms: round_ms(elapsed),
                    due_ms: boundary_ms,
                }
            }
        }
    }

    /// How long until the next frame is due, from the clock's reading
    /// `now_ms`: what the loop sleeps for. Zero when one is owed.
    pub(crate) fn wait_ms(&self, now_ms: f64) -> f64 {
        let elapsed = (now_ms - self.started_ms).max(0.0);
        (self.timeline.due_ms() - elapsed).max(0.0)
    }

    /// Whether the recording has its `max_frames`.
    pub(crate) fn is_full(&self) -> bool {
        self.frames.len() >= self.config.max_frames as usize
    }

    /// File a frame taken at `t_ms`: its entry joins the manifest and
    /// its pixels wait for the encode.
    pub(crate) fn push_frame(
        &mut self,
        t_ms: f64,
        document_revision: u64,
        camera: CameraInfo,
        pixels: CapturedPixels,
    ) {
        let index = self.frames.len() as u32;
        self.frames.push(FrameEntry {
            index,
            t_ms,
            file: frame_file(index),
            document_revision,
            camera,
        });
        self.pending_bytes += pixels.rgba.len() as u64;
        self.pending.push_back(Pending { index, pixels });
        self.unrendered_streak = 0;
    }

    /// File a frame that was due at `due_ms` and could not be taken, for
    /// `reason`: a gap of one, counted toward [`MAX_UNRENDERED_STREAK`].
    pub(crate) fn push_unrendered(&mut self, due_ms: f64, reason: &str) {
        self.push_gap(due_ms, 1, reason.to_string());
        self.unrendered_streak += 1;
    }

    /// File a frame due at `due_ms` that a node drag made undrawable. A
    /// gap like any other, but not counted toward the stall: the drag
    /// ends by itself, and a recording of one is what recordings are for.
    pub(crate) fn push_busy(&mut self, due_ms: f64) {
        self.push_gap(due_ms, 1, "busy".to_string());
    }

    /// Account for the boundaries crossed since the last [`Self::step`],
    /// as the frames they are missing: called as the recording ends, so
    /// a stop that follows a long command does not leave `duration_ms`
    /// covering frames that are neither taken nor reported.
    fn account_boundaries(&mut self, now_ms: f64) {
        if self.is_full() {
            return;
        }
        let elapsed = (now_ms - self.started_ms).max(0.0);
        if let Tick::Due {
            missed,
            first_missed_ms,
            ..
        } = self.timeline.poll(elapsed)
        {
            self.push_gap(first_missed_ms, missed.saturating_add(1), "overrun".into());
        }
    }

    /// Whether the renderer has failed to draw [`MAX_UNRENDERED_STREAK`]
    /// frames in a row.
    pub(crate) fn is_stalled(&self) -> bool {
        self.unrendered_streak >= MAX_UNRENDERED_STREAK
    }

    /// Whether the PNGs go in the artifacts dir (and so are bounded by
    /// its budget and counted into it when the recording ends).
    pub(crate) fn in_artifacts(&self) -> bool {
        self.disk_limit.is_some()
    }

    /// File a missing frame: the boundary `t_ms` came and went with
    /// none, for `reason`.
    pub(crate) fn push_gap(&mut self, t_ms: f64, frames: u32, reason: String) {
        self.gaps.push(Gap {
            t_ms: round_ms(t_ms),
            frames,
            reason,
        });
    }

    /// How many frames are missing, in all.
    pub(crate) fn dropped_frames(&self) -> u32 {
        self.gaps.iter().map(|gap| gap.frames).sum()
    }

    /// Whether enough raw frames are waiting that they should be
    /// written out now.
    pub(crate) fn wants_flush(&self) -> bool {
        self.pending_bytes >= PENDING_BYTES
    }

    /// Whether the PNGs written so far pass the disk limit.
    pub(crate) fn over_disk_limit(&self) -> bool {
        self.disk_limit.is_some_and(|limit| self.written_bytes > limit)
    }

    /// Encode every waiting frame to its file, then rewrite the
    /// manifest as it stands (`complete: false`), so a recording the
    /// app never gets to finish is still one a reader can use. Calls
    /// `progress` as the PNGs encode ([`png::encode`]).
    ///
    /// A frame that cannot be written is taken back out of the
    /// manifest, with every frame after it, so the manifest names only
    /// files that exist.
    ///
    /// # Costs
    ///
    /// One DEFLATE pass over each waiting frame, on the calling thread:
    /// the stall a long recording pays, and why it is paid rarely.
    pub(crate) fn flush(&mut self, now_ms: f64, progress: &mut dyn FnMut()) -> Result<(), String> {
        while !self.pending.is_empty() {
            self.encode_oldest(now_ms, progress)?;
        }
        self.write_manifest(now_ms, None)
    }

    /// Encode waiting frames, oldest first, for as long as `budget_ms`
    /// leaves room for another ([`IDLE_ENCODE_MARGIN`] times what one
    /// has measured to cost): the encode the loop does in time it would
    /// otherwise sleep through, so that the rest of the recording's
    /// encode — which [`Self::flush`] pays all at once — is as little as
    /// it can be. The manifest is rewritten after, so what landed is
    /// named.
    pub(crate) fn encode_in_idle(
        &mut self,
        mut budget_ms: f64,
        now_ms: f64,
        progress: &mut dyn FnMut(),
    ) -> Result<(), String> {
        let mut encoded = false;
        // Until one encode has been measured nothing says what one costs,
        // so the first is a probe: it may be late by its own cost, once.
        while !self.pending.is_empty()
            && budget_ms >= self.encode_ms.map_or(0.0, |cost| cost * IDLE_ENCODE_MARGIN)
        {
            let started = Instant::now();
            self.encode_oldest(now_ms, progress)?;
            budget_ms -= started.elapsed().as_secs_f64() * 1000.0;
            encoded = true;
        }
        if encoded {
            // So a recording the app never finishes is still readable:
            // the manifest names the files that landed.
            self.write_manifest(now_ms, None)?;
        }
        Ok(())
    }

    /// Encode the oldest waiting frame to its file, and fold what it
    /// cost into the estimate. On failure the frame, and every frame
    /// after it, leave the manifest and the waiting queue.
    fn encode_oldest(&mut self, now_ms: f64, progress: &mut dyn FnMut()) -> Result<(), String> {
        let Some(frame) = self.pending.pop_front() else {
            return Ok(());
        };
        self.pending_bytes -= frame.pixels.rgba.len() as u64;
        let path = self.dir.join(frame_file(frame.index));
        let started = Instant::now();
        match guarded_file::publish(&path, false, |out| {
            png::encode(&frame.pixels, out, &mut *progress)
        }) {
            Ok(bytes) => {
                self.written_bytes += bytes;
                let cost = started.elapsed().as_secs_f64() * 1000.0;
                self.encode_ms = Some(self.encode_ms.map_or(cost, |was| 0.7 * was + 0.3 * cost));
                Ok(())
            }
            Err(refusal) => {
                self.frames.truncate(frame.index as usize);
                self.pending.clear();
                self.pending_bytes = 0;
                // Best effort: the failure being reported is the
                // frame's, and the manifest of what landed is a
                // courtesy to whoever reads the directory.
                let _ = self.write_manifest(now_ms, None);
                Err(message_of(refusal))
            }
        }
    }

    /// End the recording: flush what waits and write the final
    /// manifest.
    pub(crate) fn finish(
        mut self,
        stopped: Stopped,
        now_ms: f64,
        progress: &mut dyn FnMut(),
    ) -> Result<Summary, String> {
        self.account_boundaries(now_ms);
        if let Err(message) = self.flush(now_ms, progress) {
            // The manifest says how it ended, as far as it can.
            let _ = self.write_manifest(now_ms, Some(Stopped::Error));
            return Err(message);
        }
        self.write_manifest(now_ms, Some(stopped))?;
        Ok(Summary {
            manifest_path: self.manifest_path(),
            frames: self.frames.len() as u32,
            dropped_frames: self.dropped_frames(),
            duration_ms: self.duration_ms(now_ms),
            stopped,
            bytes: self.written_bytes + self.manifest_bytes,
            width: self.config.width,
            height: self.config.height,
            fps: self.config.fps,
            dir: self.dir,
        })
    }

    fn duration_ms(&self, now_ms: f64) -> f64 {
        round_ms((now_ms - self.started_ms).max(0.0))
    }

    /// Write the manifest atomically over the last one. `stopped` is
    /// `None` for the copy written while the recording runs.
    fn write_manifest(&mut self, now_ms: f64, stopped: Option<Stopped>) -> Result<(), String> {
        let manifest = Manifest {
            protocol: crate::application::ipc::PROTOCOL_VERSION,
            fps: self.config.fps,
            pacing: PACING_REALTIME,
            width: self.config.width,
            height: self.config.height,
            scale: self.config.scale,
            complete: stopped.is_some(),
            stopped,
            duration_ms: self.duration_ms(now_ms),
            frames: &self.frames,
            dropped_frames: self.dropped_frames(),
            gaps: &self.gaps,
        };
        let path = self.manifest_path();
        self.manifest_bytes = guarded_file::publish(&path, true, |out| {
            serde_json::to_writer(out, &manifest).map_err(|e| e.to_string())
        })
        .map_err(message_of)?;
        Ok(())
    }
}

/// The bytes in `dir`'s files, for counting a recording against the
/// artifacts budget however it ended. A directory that cannot be read
/// counts as empty: the count is a bound, not a record.
pub(crate) fn dir_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.metadata().ok())
            .filter(|meta| meta.is_file())
            .map(|meta| meta.len())
            .sum()
    })
}

fn message_of(refusal: PublishRefusal) -> String {
    let (PublishRefusal::Invalid(message) | PublishRefusal::Internal(message)) = refusal;
    message
}

/// The app's one recording: in progress, or how the last one ended.
#[derive(Default)]
pub(crate) struct Recorder {
    pub(crate) active: Option<Recording>,
    /// How a recording that ended on its own (`max_frames`, the disk
    /// budget, a failed write) ended, kept until `capture.record_stop`
    /// collects it. Without it the agent that asked for a recording and
    /// looked away would find "nothing is recording" and no way to
    /// learn where its frames went.
    pub(crate) finished: Option<Result<Summary, String>>,
}

/// Where a recording's frames go.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RecordDestination {
    /// A directory the caller chose: absolute, and either not there
    /// yet or empty.
    Path(PathBuf),
    /// A fresh subdirectory of the artifacts dir.
    Artifacts,
}

/// A recording's directory, as made.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MadeDir {
    pub(crate) path: PathBuf,
    /// In the artifacts dir: bounded by its budget, removed with it.
    pub(crate) in_artifacts: bool,
    /// This call made it, so a recording that never began may take it
    /// away again.
    pub(crate) created: bool,
}

/// Resolve `destination` into a directory that exists and holds
/// nothing: a recording never writes among files it did not make.
///
/// A caller's path is absolute and is not a symlink; a directory
/// already there must be empty, and one that is not there yet is made
/// if its parent exists (`mkdir`, never `mkdir -p`, so a typo in the
/// middle of a path is a refusal and not a new tree).
pub(crate) fn make_dir(
    destination: RecordDestination,
    artifacts: &mut Artifacts,
) -> Result<MadeDir, CaptureError> {
    let invalid = |message: String| CaptureError::Invalid {
        param: "dir",
        message,
    };
    let path = match destination {
        RecordDestination::Artifacts => {
            return Ok(MadeDir {
                path: artifacts.next_dir("recording").map_err(CaptureError::Internal)?,
                in_artifacts: true,
                created: true,
            })
        }
        RecordDestination::Path(path) => path,
    };
    if !path.is_absolute() {
        return Err(invalid(format!("`{}` is not an absolute path", path.display())));
    }
    let shown = path.display();
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(invalid(format!(
            "`{shown}` is a symlink, which is never followed"
        ))),
        Ok(meta) if !meta.is_dir() => Err(invalid(format!("`{shown}` exists and is not a directory"))),
        Ok(_) => {
            let mut entries = std::fs::read_dir(&path)
                .map_err(|e| CaptureError::Internal(format!("could not read `{shown}`: {e}")))?;
            if entries.next().is_some() {
                return Err(invalid(format!(
                    "`{shown}` is not empty; a recording writes into a directory of its own"
                )));
            }
            Ok(MadeDir {
                path: path.clone(),
                in_artifacts: false,
                created: false,
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match std::fs::create_dir(&path) {
            Ok(()) => Ok(MadeDir {
                path: path.clone(),
                in_artifacts: false,
                created: true,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(invalid(format!(
                "the parent directory of `{shown}` does not exist"
            ))),
            Err(e) => Err(CaptureError::Internal(format!("could not make `{shown}`: {e}"))),
        },
        Err(e) => Err(CaptureError::Internal(format!("could not read `{shown}`: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::capture::artifacts::BUDGET_BYTES;
    use crate::application::guarded_file::tests::{names_in, scratch_dir};

    fn config(fps: u32, max_frames: u32) -> RecordConfig {
        RecordConfig {
            fps,
            max_frames,
            width: 2,
            height: 1,
            scale: 1.0,
            request: CaptureRequest::default(),
        }
    }

    fn camera() -> CameraInfo {
        CameraInfo {
            center_x: 1.5,
            center_y: -2.0,
            zoom: 0.75,
        }
    }

    fn pixels(seed: u8) -> CapturedPixels {
        CapturedPixels {
            width: 2,
            height: 1,
            rgba: vec![seed, 0, 0, 255, 0, seed, 0, 255],
        }
    }

    /// Poll `timeline` at every whole millisecond up to and including
    /// `to_ms`, counting the frames it owes.
    fn frames_owed(timeline: &mut Timeline, to_ms: u32) -> u32 {
        (0..=to_ms)
            .filter(|ms| matches!(timeline.poll(f64::from(*ms)), Tick::Due { .. }))
            .count() as u32
    }

    /// The emission rule `format/ipc.md` pins: a frame each time the
    /// clock crosses a `1000/fps` boundary, so a second at 30 fps is
    /// 30 frames — the first at 0 and the thirtieth at 966.7 ms — and
    /// the thirty-first is the next second's.
    #[test]
    fn test_a_second_at_thirty_fps_is_thirty_frames() {
        assert_eq!(frames_owed(&mut Timeline::new(30), 999), 30);
        assert_eq!(frames_owed(&mut Timeline::new(30), 1000), 31);
        assert_eq!(frames_owed(&mut Timeline::new(60), 999), 60);
        assert_eq!(frames_owed(&mut Timeline::new(1), 999), 1);
        // Counted as whole boundaries: a minute of 30 fps is 1800, not
        // whatever a float sum drifts to.
        let mut timeline = Timeline::new(30);
        let mut frames = 0;
        for tick in 0..(60 * 30) {
            let at = f64::from(tick) * 1000.0 / 30.0;
            assert!(matches!(timeline.poll(at), Tick::Due { missed: 0, .. }), "{tick}");
            frames += 1;
        }
        assert_eq!(frames, 1800);
    }

    /// The first frame is owed at 0; until the next boundary the answer
    /// is when it is.
    #[test]
    fn test_nothing_is_owed_before_the_next_boundary() {
        let mut timeline = Timeline::new(10);
        assert_eq!(
            timeline.poll(0.0),
            Tick::Due {
                missed: 0,
                first_missed_ms: 0.0,
                boundary_ms: 0.0
            }
        );
        assert_eq!(timeline.poll(99.9), Tick::Wait { until_ms: 100.0 });
        assert_eq!(timeline.due_ms(), 100.0);
        assert!(matches!(timeline.poll(100.0), Tick::Due { missed: 0, .. }));
        // A clock reading before the start, or no reading at all, owes
        // nothing.
        let mut fresh = Timeline::new(10);
        fresh.poll(0.0);
        assert!(matches!(fresh.poll(-5.0), Tick::Wait { .. }));
        assert!(matches!(fresh.poll(f64::NAN), Tick::Wait { .. }));
    }

    /// A heartbeat that overran takes one frame, for the latest
    /// boundary, and reports the ones it stepped over — which are not
    /// owed afterwards.
    #[test]
    fn test_a_late_heartbeat_takes_one_frame_and_counts_the_boundaries_it_missed() {
        let mut timeline = Timeline::new(30);
        timeline.poll(0.0);
        // Boundaries 1, 2 and 3 are at 33.3, 66.7 and 100 ms.
        match timeline.poll(101.0) {
            Tick::Due {
                missed,
                first_missed_ms,
                boundary_ms,
            } => {
                assert_eq!(missed, 2, "boundary 3 gets the frame, 1 and 2 are missed");
                assert_eq!(boundary_ms, 100.0, "the frame is for the latest boundary");
                assert!(
                    (first_missed_ms - 1000.0 / 30.0).abs() < 1e-9,
                    "{first_missed_ms}"
                );
            }
            other => panic!("a frame was owed: {other:?}"),
        }
        assert_eq!(
            timeline.poll(120.0),
            Tick::Wait {
                until_ms: 4.0 * 1000.0 / 30.0
            }
        );
    }

    /// The recorder stamps frames with the clock's reading since its
    /// start, records an overrun as a gap, and is full at `max_frames`.
    #[test]
    fn test_steps_stamp_frames_record_overruns_and_end_at_max_frames() {
        let mut recording = Recording::begin(config(10, 3), PathBuf::from("/unused"), 1000.0, None);
        assert_eq!(
            recording.step(1000.0),
            Step::Capture {
                t_ms: 0.0,
                due_ms: 0.0
            }
        );
        recording.push_frame(0.0, 7, camera(), pixels(1));
        assert_eq!(recording.step(1050.0), Step::Wait { in_ms: 50.0 });
        assert_eq!(recording.wait_ms(1050.0), 50.0);
        // 250 ms in: boundaries 1 and 2 (100, 200) passed, the frame is
        // for boundary 2, so boundary 1 is the gap.
        assert_eq!(
            recording.step(1250.0),
            Step::Capture {
                t_ms: 250.0,
                due_ms: 200.0
            }
        );
        recording.push_frame(250.0, 8, camera(), pixels(2));
        assert_eq!(recording.dropped_frames(), 1);
        assert_eq!(recording.gaps[0].t_ms, 100.0);
        assert_eq!(recording.gaps[0].reason, "overrun");
        // Boundary 3 is at 300 ms; the third frame fills the recording.
        assert_eq!(
            recording.step(1300.0),
            Step::Capture {
                t_ms: 300.0,
                due_ms: 300.0
            }
        );
        recording.push_frame(300.0, 8, camera(), pixels(3));
        assert!(recording.is_full());
        assert_eq!(recording.step(1400.0), Step::Full);
        assert_eq!(
            recording
                .frames
                .iter()
                .map(|f| f.file.as_str())
                .collect::<Vec<_>>(),
            ["frame_00000.png", "frame_00001.png", "frame_00002.png"],
            "numbered by frame taken, with no hole where a gap is"
        );
    }

    /// A clock reading before the start does not make time run
    /// backwards: the frame is stamped 0.
    #[test]
    fn test_a_clock_before_the_start_stamps_zero() {
        let mut recording = Recording::begin(config(10, 3), PathBuf::from("/unused"), 500.0, None);
        assert_eq!(
            recording.step(400.0),
            Step::Capture {
                t_ms: 0.0,
                due_ms: 0.0
            }
        );
    }

    fn read_manifest(dir: &Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(dir.join(MANIFEST_FILE)).expect("manifest")).expect("JSON")
    }

    /// A recording flushes its frames to numbered PNGs that decode to
    /// the pixels it was given, with a manifest that says what it is:
    /// incomplete while running, complete once finished, the gaps and
    /// their sum honest, and the sizes the frames really are.
    #[test]
    fn test_frames_and_the_manifest_land_in_the_directory() {
        let dir = scratch_dir("record-land");
        let mut recording = Recording::begin(config(10, 10), dir.clone(), 0.0, None);
        recording.push_frame(0.0, 3, camera(), pixels(10));
        recording.push_gap(100.0, 2, "overrun".into());
        recording.push_gap(300.0, 1, "font_system_contended".into());
        recording.push_frame(350.0, 4, camera(), pixels(20));
        // The boundaries up to 500 ms have been stepped, as the loop does.
        recording.timeline.next = 6;
        let mut ticks = 0;
        recording.flush(400.0, &mut || ticks += 1).expect("flushed");
        assert!(ticks > 0, "the watchdog hears about the encode");
        assert_eq!(
            names_in(&dir),
            ["frame_00000.png", "frame_00001.png", "manifest.json"]
        );
        let midway = read_manifest(&dir);
        assert_eq!(midway["complete"], false);
        assert!(midway.get("stopped").is_none());
        assert_eq!(midway["frames"].as_array().expect("frames").len(), 2);

        let summary = recording
            .finish(Stopped::Client, 500.0, &mut || {})
            .expect("finished");
        assert_eq!(
            (
                summary.frames,
                summary.dropped_frames,
                summary.duration_ms,
                summary.stopped
            ),
            (2, 3, 500.0, Stopped::Client)
        );
        assert_eq!(summary.manifest_path, dir.join(MANIFEST_FILE));
        let on_disk: u64 = names_in(&dir)
            .iter()
            .map(|name| std::fs::metadata(dir.join(name)).expect("file").len())
            .sum();
        assert_eq!(summary.bytes, on_disk, "the bytes are the directory's");

        let manifest = read_manifest(&dir);
        assert_eq!(manifest["protocol"], 1);
        assert_eq!(manifest["fps"], 10);
        assert_eq!(manifest["pacing"], "realtime");
        assert_eq!(
            (manifest["width"].as_u64(), manifest["height"].as_u64()),
            (Some(2), Some(1))
        );
        assert_eq!(manifest["complete"], true);
        assert_eq!(manifest["stopped"], "client");
        assert_eq!(manifest["duration_ms"], 500.0);
        assert_eq!(manifest["dropped_frames"], 3);
        assert_eq!(manifest["frames"][1]["index"], 1);
        assert_eq!(manifest["frames"][1]["t_ms"], 350.0);
        assert_eq!(manifest["frames"][1]["file"], "frame_00001.png");
        assert_eq!(manifest["frames"][1]["document_revision"], 4);
        assert_eq!(manifest["frames"][1]["camera"]["zoom"], 0.75);
        assert_eq!(
            manifest["gaps"],
            serde_json::json!([
                {"t_ms": 100.0, "frames": 2, "reason": "overrun"},
                {"t_ms": 300.0, "frames": 1, "reason": "font_system_contended"},
            ])
        );
        let gap_sum: u64 = manifest["gaps"]
            .as_array()
            .expect("gaps")
            .iter()
            .map(|gap| gap["frames"].as_u64().expect("count"))
            .sum();
        assert_eq!(manifest["dropped_frames"], gap_sum);

        let (width, height, rgba) =
            png::tests::decode(&std::fs::read(dir.join("frame_00001.png")).expect("png"));
        assert_eq!((width, height, rgba), (2, 1, pixels(20).rgba));
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// A frame that cannot be written comes out of the manifest with
    /// every frame after it, so the manifest names only files that
    /// exist, and the failure is reported.
    #[test]
    fn test_a_failed_write_leaves_a_manifest_of_what_landed() {
        let dir = scratch_dir("record-fail");
        std::fs::write(dir.join("frame_00001.png"), "in the way").expect("seed");
        let mut recording = Recording::begin(config(10, 10), dir.clone(), 0.0, None);
        for seed in 0..3 {
            recording.push_frame(f64::from(seed) * 100.0, 1, camera(), pixels(seed as u8));
        }
        let failed = recording.finish(Stopped::Client, 300.0, &mut || {});
        assert!(
            matches!(&failed, Err(message) if message.contains("frame_00001.png")),
            "{failed:?}"
        );
        let manifest = read_manifest(&dir);
        assert_eq!(manifest["complete"], true, "ended, and says how");
        assert_eq!(manifest["stopped"], "error");
        assert_eq!(manifest["frames"].as_array().expect("frames").len(), 1);
        assert!(dir.join("frame_00000.png").exists());
        assert!(!dir.join("frame_00002.png").exists(), "nothing after the failure");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// The raw frames waiting are counted until they are written and
    /// not after, and a recording in the artifacts dir reports when its
    /// PNGs pass the dir's budget.
    #[test]
    fn test_waiting_frames_and_the_disk_limit_are_accounted() {
        let dir = scratch_dir("record-account");
        let mut recording = Recording::begin(config(10, 10), dir.clone(), 0.0, Some(10));
        assert!(!recording.wants_flush());
        recording.push_frame(0.0, 1, camera(), pixels(1));
        assert_eq!(recording.pending_bytes, 8);
        assert!(!recording.over_disk_limit(), "nothing written yet");
        recording.flush(0.0, &mut || {}).expect("flushed");
        assert_eq!(recording.pending_bytes, 0);
        assert!(
            recording.over_disk_limit(),
            "a PNG and a manifest are more than ten bytes"
        );
        let unlimited = Recording::begin(config(10, 10), dir.clone(), 0.0, None);
        assert!(!unlimited.over_disk_limit());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// A caller's directory is absolute, is not a symlink, and is empty
    /// or not there yet; only a directory the call made may be taken
    /// away again.
    #[test]
    fn test_the_callers_directory_is_vetted_before_anything_is_written() {
        let root = scratch_dir("record-dir");
        let mut artifacts = Artifacts::under(root.join("artifacts"), BUDGET_BYTES);
        let mut make = |path: PathBuf| make_dir(RecordDestination::Path(path), &mut artifacts);

        let fresh = make(root.join("fresh")).expect("made");
        assert_eq!(
            fresh,
            MadeDir {
                path: root.join("fresh"),
                in_artifacts: false,
                created: true
            }
        );
        assert!(root.join("fresh").is_dir());
        let again = make(root.join("fresh")).expect("an empty directory is fine");
        assert!(!again.created, "it was there already");

        std::fs::write(root.join("fresh").join("x"), "x").expect("seed");
        for (path, word) in [
            (root.join("fresh"), "not empty"),
            (root.join("fresh").join("x"), "not a directory"),
            (root.join("missing").join("rec"), "does not exist"),
            (PathBuf::from("rec"), "not an absolute path"),
        ] {
            let refused = make(path.clone());
            assert!(
                matches!(&refused, Err(CaptureError::Invalid { param: "dir", message }) if message.contains(word)),
                "{path:?}: {refused:?}"
            );
        }
        assert!(!root.join("missing").exists(), "no tree was made for a typo");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("fresh"), root.join("link")).expect("link");
            let refused = make(root.join("link"));
            assert!(
                matches!(&refused, Err(CaptureError::Invalid { message, .. }) if message.contains("symlink")),
                "{refused:?}"
            );
        }
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    /// Without a path the directory is a fresh one in the artifacts dir,
    /// each recording its own.
    #[test]
    fn test_the_default_directory_is_a_fresh_one_in_the_artifacts_dir() {
        let root = scratch_dir("record-artifacts");
        let mut artifacts = Artifacts::under(root.clone(), BUDGET_BYTES);
        let first = make_dir(RecordDestination::Artifacts, &mut artifacts).expect("made");
        let second = make_dir(RecordDestination::Artifacts, &mut artifacts).expect("made");
        assert!(first.in_artifacts && first.path.is_dir() && second.path.is_dir());
        assert_ne!(first.path, second.path);
        assert_eq!(first.path.file_name().expect("named"), "recording-00001");
        artifacts.remove();
        assert!(!first.path.exists());
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    /// `fps` and `max_frames` are bounded the way `format/ipc.md` says.
    #[test]
    fn test_the_documented_bounds_are_the_enforced_ones() {
        use baumhard::util::doc_fixtures::{repo_path, section_text};
        let section = section_text(
            &repo_path("format/ipc.md"),
            "### `capture.record_start` / `capture.record_stop` (IPC-08)",
        );
        for stated in [
            format!("default `{DEFAULT_FPS}`"),
            format!("`{}..={}`", FPS_RANGE.start(), FPS_RANGE.end()),
            format!("default `{DEFAULT_MAX_FRAMES}`"),
            format!("`1..={MAX_FRAMES}`"),
            format!("{} MiB", PENDING_BYTES >> 20),
        ] {
            assert!(section.contains(&stated), "format/ipc.md must state {stated}");
        }
    }

    /// A run of frames the renderer would not draw ends the recording,
    /// and one frame in between starts the count over: a recording that
    /// can never reach `max_frames` must not run forever, and a moment's
    /// contention must not end one.
    #[test]
    fn test_a_long_run_of_undrawn_frames_stalls_the_recording() {
        let mut recording = Recording::begin(config(10, 10), PathBuf::from("/unused"), 0.0, None);
        for n in 0..MAX_UNRENDERED_STREAK - 1 {
            recording.push_unrendered(f64::from(n), "font_system_contended");
        }
        assert!(!recording.is_stalled());
        recording.push_frame(0.0, 1, camera(), pixels(1));
        recording.push_unrendered(0.0, "font_system_contended");
        assert!(
            !recording.is_stalled(),
            "a frame in between starts the count over"
        );
        for n in 0..MAX_UNRENDERED_STREAK {
            recording.push_unrendered(f64::from(n), "prepare_failed");
        }
        assert!(recording.is_stalled());
        assert_eq!(recording.dropped_frames(), 2 * MAX_UNRENDERED_STREAK);
        assert!(recording.gaps.iter().all(|gap| gap.frames == 1));
    }

    /// Idle-time encoding spends only the time it is given: nothing
    /// under the margin over the measured cost, the oldest frames first
    /// when there is room, and what it writes is what a flush would —
    /// numbered files that decode to the pixels given.
    #[test]
    fn test_idle_encoding_stays_within_the_time_it_is_given() {
        let dir = scratch_dir("record-idle");
        let mut recording = Recording::begin(config(10, 10), dir.clone(), 0.0, None);
        for seed in 1..=3 {
            recording.push_frame(f64::from(seed) * 100.0, 1, camera(), pixels(seed as u8));
        }
        recording.encode_ms = Some(100.0);
        recording.encode_in_idle(149.0, 0.0, &mut || {}).expect("fine");
        assert_eq!(
            recording.pending.len(),
            3,
            "under the margin over the cost: nothing"
        );
        assert_eq!(names_in(&dir), Vec::<String>::new());

        recording.encode_in_idle(150.0, 0.0, &mut || {}).expect("fine");
        assert!(recording.pending.len() < 3, "room for one: the oldest went");
        assert!(dir.join("frame_00000.png").exists());
        // The estimate drops to what the first encode really cost, so
        // the budget may reach the rest too; whatever still waits is the
        // newest, in order.
        if let Some(front) = recording.pending.front() {
            assert_eq!(front.index, 3 - recording.pending.len() as u32, "oldest first");
        }
        let measured = recording.encode_ms.expect("measured");
        assert!(measured < 100.0, "the estimate follows what it cost: {measured}");

        // Nothing is lost by encoding early: a flush finishes the rest
        // and the recording is whole.
        let summary = recording
            .finish(Stopped::Client, 300.0, &mut || {})
            .expect("finished");
        assert_eq!(summary.frames, 3);
        assert_eq!(
            names_in(&dir),
            [
                "frame_00000.png",
                "frame_00001.png",
                "frame_00002.png",
                "manifest.json"
            ]
        );
        let (_, _, rgba) = png::tests::decode(&std::fs::read(dir.join("frame_00002.png")).expect("png"));
        assert_eq!(rgba, pixels(3).rgba);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// An idle encode that fails ends the way a flush does: the failed
    /// frame and the ones after it come out of the manifest, and
    /// nothing is left waiting.
    #[test]
    fn test_a_failed_idle_encode_takes_its_frame_and_the_rest_back_out() {
        let dir = scratch_dir("record-idle-fail");
        std::fs::write(dir.join("frame_00001.png"), "in the way").expect("seed");
        let mut recording = Recording::begin(config(10, 10), dir.clone(), 0.0, None);
        for seed in 0..3 {
            recording.push_frame(f64::from(seed) * 100.0, 1, camera(), pixels(seed as u8));
        }
        let failed = recording.encode_in_idle(1.0e9, 0.0, &mut || {});
        assert!(
            matches!(&failed, Err(m) if m.contains("frame_00001.png")),
            "{failed:?}"
        );
        assert_eq!((recording.pending.len(), recording.pending_bytes), (0, 0));
        assert_eq!(read_manifest(&dir)["frames"].as_array().expect("frames").len(), 1);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// A drag-made gap is a gap and is stamped where the frame was due,
    /// but it does not count toward the stall: the drag ends by itself,
    /// and a recording of one is the point.
    #[test]
    fn test_a_busy_frame_is_a_gap_that_never_stalls_the_recording() {
        let mut recording = Recording::begin(config(10, 10), PathBuf::from("/unused"), 0.0, None);
        for n in 0..3 * MAX_UNRENDERED_STREAK {
            recording.push_busy(f64::from(n) * 100.0);
        }
        assert!(!recording.is_stalled());
        assert_eq!(recording.dropped_frames(), 3 * MAX_UNRENDERED_STREAK);
        assert_eq!(
            recording.gaps[2].t_ms, 200.0,
            "stamped with the boundary it was due at"
        );
        assert!(recording.gaps.iter().all(|gap| gap.reason == "busy"));
    }

    /// Boundaries crossed since the last step are accounted for as the
    /// recording ends — a stop after a long command leaves no stretch
    /// that `duration_ms` covers and nothing explains — and a recording
    /// that is full owes nothing more.
    #[test]
    fn test_the_boundaries_before_a_stop_are_gaps_not_silence() {
        let dir = scratch_dir("record-last");
        let mut recording = Recording::begin(config(10, 10), dir.clone(), 0.0, None);
        assert!(matches!(recording.step(0.0), Step::Capture { .. }));
        recording.push_frame(0.0, 1, camera(), pixels(1));
        // Boundaries 1..=5 (100..500 ms) pass with no heartbeat.
        let summary = recording
            .finish(Stopped::Client, 520.0, &mut || {})
            .expect("finished");
        assert_eq!((summary.frames, summary.dropped_frames), (1, 5));
        let manifest = read_manifest(&dir);
        assert_eq!(
            manifest["gaps"],
            serde_json::json!([{"t_ms": 100.0, "frames": 5, "reason": "overrun"}])
        );
        std::fs::remove_dir_all(&dir).expect("cleanup");

        let dir = scratch_dir("record-last-full");
        let mut full = Recording::begin(config(10, 1), dir.clone(), 0.0, None);
        assert!(matches!(full.step(0.0), Step::Capture { .. }));
        full.push_frame(0.0, 1, camera(), pixels(1));
        let summary = full
            .finish(Stopped::MaxFrames, 900.0, &mut || {})
            .expect("finished");
        assert_eq!(
            summary.dropped_frames, 0,
            "a full recording took what it was asked for"
        );
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// Frames encoded in idle time are named by a manifest at once, so a
    /// recording that dies before it is finished is still readable — and
    /// the first idle encode, with nothing measured yet, is a probe that
    /// runs whatever time there is.
    #[test]
    fn test_idle_encoding_leaves_a_readable_manifest_behind() {
        let dir = scratch_dir("record-idle-manifest");
        let mut recording = Recording::begin(config(10, 10), dir.clone(), 0.0, None);
        recording.push_frame(0.0, 1, camera(), pixels(1));
        assert!(recording.encode_ms.is_none());
        recording.encode_in_idle(0.0, 50.0, &mut || {}).expect("probed");
        assert!(recording.encode_ms.is_some(), "the probe measured one");
        let manifest = read_manifest(&dir);
        assert_eq!(manifest["complete"], false);
        assert_eq!(manifest["frames"].as_array().expect("frames").len(), 1);
        assert!(dir.join("frame_00000.png").exists());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// The artifacts dir counts what a recording left, in bytes.
    #[test]
    fn test_dir_bytes_counts_the_files_in_the_directory() {
        let dir = scratch_dir("record-bytes");
        std::fs::write(dir.join("a"), [0u8; 10]).expect("a");
        std::fs::write(dir.join("b"), [0u8; 5]).expect("b");
        std::fs::create_dir(dir.join("sub")).expect("sub");
        assert_eq!(dir_bytes(&dir), 15);
        assert_eq!(dir_bytes(&dir.join("missing")), 0);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
