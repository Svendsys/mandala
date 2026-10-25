// SPDX-License-Identifier: MPL-2.0

//! The executor's half of the `record` verb: the line the verb read
//! (`console::commands::record`), handed to the recorder
//! (`app::record_flow`) — the one IPC's `capture.record_start` and
//! `capture.record_stop` call — and the recorder's report worded for
//! the scrollback.
//!
//! What differs from IPC is decided here and nowhere else, exactly as
//! for `screenshot` (`console_input::screenshot`): a relative `dir` is
//! read against the working directory, no `dir` means
//! `mandala-recording-<unix ms>` there rather than the IPC artifacts
//! dir, which is removed when the app exits, and the console is left
//! out of the frames. The bounds on `fps` and `max_frames` are IPC's,
//! worded for the console.

use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use crate::application::capture::plan::CaptureRequest;
use crate::application::capture::record::{fps_of, max_frames_of, RecordDestination, Started, Summary};
use crate::application::capture::CaptureError;
use crate::application::console::commands::record::{RecordLine, RecordStartLine};
use crate::application::console::ExecResult;

use super::super::capture_flow::CaptureScene;
use super::super::input_context::InputHandlerContext;
use super::super::record_flow;
use super::screenshot::unix_ms;

/// Do what `line` asks of the recorder, and say what happened.
pub(super) fn take(ctx: &mut InputHandlerContext<'_>, line: &RecordLine) -> ExecResult {
    match line {
        RecordLine::Stop => report_stop(record_flow::stop(
            ctx.recorder,
            ctx.artifacts,
            ctx.watchdog,
            ctx.clock,
        )),
        RecordLine::Start(start) => start_recording(ctx, start),
    }
}

fn start_recording(ctx: &mut InputHandlerContext<'_>, start: &RecordStartLine) -> ExecResult {
    let dir = match std::env::current_dir() {
        Ok(cwd) => console_dir(start.dir.as_deref(), &cwd, unix_ms()),
        Err(e) => return ExecResult::err(format!("record: the working directory cannot be read: {e}")),
    };
    // The bounds are `record`'s own, the same two checks IPC runs; only
    // the wording is the console's.
    let refusal = |name: &str, typed: Option<u32>, range: RangeInclusive<u32>| {
        format!(
            "record: {name}={} must be in {}..={}",
            typed.map_or(String::new(), |n| n.to_string()),
            range.start(),
            range.end()
        )
    };
    let fps = match fps_of(start.fps.map(u64::from)) {
        Ok(fps) => fps,
        Err(range) => return ExecResult::err(refusal("fps", start.fps, range)),
    };
    let max_frames = match max_frames_of(start.max_frames.map(u64::from)) {
        Ok(max_frames) => max_frames,
        Err(range) => return ExecResult::err(refusal("max_frames", start.max_frames, range)),
    };
    let clock = ctx.clock.clone();
    let (scene, artifacts, recorder) = CaptureScene::of_context(ctx);
    report_start(
        record_flow::start(
            scene,
            recorder,
            artifacts,
            &CaptureRequest::from_console_view(&start.view),
            fps,
            max_frames,
            RecordDestination::Path(dir),
            &clock,
        ),
        max_frames,
    )
}

/// Where the frames go: `typed` read against `cwd` the way the shell
/// reads it, or `mandala-recording-<now_ms>` in `cwd` when nothing was
/// typed.
fn console_dir(typed: Option<&str>, cwd: &Path, now_ms: u128) -> PathBuf {
    match typed {
        Some(typed) => cwd.join(typed),
        None => cwd.join(format!("mandala-recording-{now_ms}")),
    }
}

/// The recorder's report on starting, as the scrollback line. A refusal
/// names the console key that asked for the impossible — `frame=` for
/// what IPC calls `region` and `fit`.
fn report_start(started: Result<Started, CaptureError>, max_frames: u32) -> ExecResult {
    match started {
        Ok(started) => {
            let discarded = match &started.previous {
                None => String::new(),
                Some(Ok(earlier)) => format!(
                    "; an earlier recording had ended ({}) with {} frames in {}, and its report is discarded",
                    earlier.stopped.word(),
                    earlier.frames,
                    earlier.dir.display()
                ),
                Some(Err(why)) => {
                    format!("; an earlier recording had failed and its report is discarded: {why}")
                }
            };
            ExecResult::ok_msg(format!(
                "record: {}×{} at {} fps into {} (stops itself after {max_frames} frames; `record stop` ends it){discarded}",
                started.width,
                started.height,
                started.fps,
                started.dir.display()
            ))
        }
        Err(CaptureError::Invalid { param, message }) => {
            let key = match param {
                "region" | "fit" => "frame",
                "cmd" => "start",
                other => other,
            };
            ExecResult::err(format!("record: {key}: {message}"))
        }
        Err(CaptureError::Busy(message) | CaptureError::Internal(message)) => {
            ExecResult::err(format!("record: {message}"))
        }
    }
}

/// The recorder's report on stopping, as the scrollback line.
fn report_stop(stopped: Result<Summary, CaptureError>) -> ExecResult {
    match stopped {
        Ok(summary) => {
            let dropped = match summary.dropped_frames {
                0 => String::new(),
                n => format!(", {n} dropped"),
            };
            ExecResult::ok_msg(format!(
                "record: {} frames{dropped} over {:.1} s in {} (ended: {}); assemble with `mandalactl record assemble {}`",
                summary.frames,
                summary.duration_ms / 1000.0,
                summary.dir.display(),
                summary.stopped.word(),
                summary.dir.display()
            ))
        }
        Err(refused) => ExecResult::err(format!("record: {}", refused.into_message())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A typed directory is the shell's: absolute as it is, relative
    /// against the working directory. None is a timestamped directory
    /// there.
    #[test]
    fn test_the_dir_is_read_the_way_the_shell_reads_it() {
        let cwd = Path::new("/work");
        assert_eq!(console_dir(Some("frames"), cwd, 7), PathBuf::from("/work/frames"));
        assert_eq!(console_dir(Some("/tmp/f"), cwd, 7), PathBuf::from("/tmp/f"));
        assert_eq!(
            console_dir(None, cwd, 7),
            PathBuf::from("/work/mandala-recording-7")
        );
    }

    /// A refusal names the console's key, not IPC's parameter, and the
    /// state refusals read as the commands a person typed.
    #[test]
    fn test_refusals_are_worded_for_the_console() {
        let refused = |param| {
            report_start(
                Err(CaptureError::Invalid {
                    param,
                    message: "no".into(),
                }),
                1,
            )
        };
        for (param, worded) in [
            ("fit", "record: frame: no"),
            ("region", "record: frame: no"),
            ("dir", "record: dir: no"),
            ("cmd", "record: start: no"),
        ] {
            let result = refused(param);
            assert!(
                matches!(&result, ExecResult::Err(m) if m == worded),
                "{param}: {result:?}"
            );
        }
        let stopped = report_stop(Err(CaptureError::Invalid {
            param: "cmd",
            message: "nothing is recording".into(),
        }));
        assert!(
            matches!(&stopped, ExecResult::Err(m) if m == "record: nothing is recording"),
            "{stopped:?}"
        );
    }
}
