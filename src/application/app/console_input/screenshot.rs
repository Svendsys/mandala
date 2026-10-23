// SPDX-License-Identifier: MPL-2.0

//! The executor's half of the `screenshot` verb: the line the verb read
//! (`console::commands::screenshot`), handed to the screenshot engine
//! (`app::capture_flow`) — the one IPC's `capture.screenshot` calls —
//! and the engine's report worded for the scrollback.
//!
//! What differs from IPC is decided here and nowhere else: a relative
//! path is read against the working directory, as `save` reads its
//! own; no path means `mandala-<unix ms>.png` there rather than the
//! IPC artifacts dir, which is removed when the app exits; and the
//! console is left out of the picture it was asked from (the request
//! itself is the engine's reading of the line,
//! `CaptureRequest::from_console_line`).

use std::path::{Path, PathBuf};

use crate::application::capture::plan::CaptureRequest;
use crate::application::capture::{CaptureError, CaptureOutcome, Destination};
use crate::application::console::commands::screenshot::ScreenshotLine;
use crate::application::console::ExecResult;

use super::super::capture_flow::{self, CaptureScene};
use super::super::input_context::InputHandlerContext;

/// Take the screenshot `line` asks for, and say what happened.
pub(super) fn take(ctx: &mut InputHandlerContext<'_>, line: &ScreenshotLine) -> ExecResult {
    let path = match std::env::current_dir() {
        Ok(cwd) => console_path(line.path.as_deref(), &cwd, unix_ms()),
        Err(e) => return ExecResult::err(format!("screenshot: the working directory cannot be read: {e}")),
    };
    let scene = CaptureScene {
        moving_node: ctx.drag_state.is_moving_node(),
        document: ctx.document,
        mindmap_tree: ctx.mindmap_tree,
        app_scene: ctx.app_scene,
        renderer: ctx.renderer,
        scene_cache: ctx.scene_cache,
        interaction_mode: ctx.interaction_mode,
        watchdog: ctx.watchdog,
    };
    let destination = Destination::Path {
        path,
        overwrite: line.overwrite,
    };
    report(capture_flow::capture(
        scene,
        &CaptureRequest::from_console_line(line),
        destination,
        line.sidecar,
        ctx.artifacts,
    ))
}

/// Where the PNG goes: `typed` read against `cwd` the way the shell
/// reads it, or `mandala-<now_ms>.png` in `cwd` when nothing was typed.
fn console_path(typed: Option<&str>, cwd: &Path, now_ms: u128) -> PathBuf {
    match typed {
        Some(typed) => cwd.join(typed),
        None => cwd.join(format!("mandala-{now_ms}.png")),
    }
}

/// Milliseconds since the Unix epoch, for the default file name. A
/// clock before 1970 names the file `mandala-0.png`, which the write
/// guard then keeps from replacing an earlier one.
pub(super) fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0)
}

/// The engine's report, as the scrollback line. A refusal names the
/// console key that asked for the impossible — `frame=` for what IPC
/// calls `region` and `fit`.
fn report(taken: Result<CaptureOutcome, CaptureError>) -> ExecResult {
    match taken {
        Ok(CaptureOutcome::Written {
            camera,
            path,
            sidecar_path,
        }) => {
            let (width, height) = camera.viewport_size;
            let geometry = sidecar_path
                .map(|side| format!(", geometry in {}", side.display()))
                .unwrap_or_default();
            ExecResult::ok_msg(format!(
                "screenshot: {width}×{height} written to {}{geometry}",
                path.display()
            ))
        }
        Ok(CaptureOutcome::Skipped { skip, .. }) => ExecResult::err(format!(
            "screenshot: the frame was not drawn ({}), so nothing was written; try again",
            skip.word()
        )),
        Err(CaptureError::Invalid { param, message }) => {
            let key = match param {
                "region" | "fit" => "frame",
                other => other,
            };
            ExecResult::err(format!("screenshot: {key}: {message}"))
        }
        Err(CaptureError::Busy(message) | CaptureError::Internal(message)) => {
            ExecResult::err(format!("screenshot: {message}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::gfx_structs::camera::Camera2D;

    use crate::application::renderer::FrameSkip;

    /// A typed path is the shell's: absolute as it is, relative against
    /// the working directory. No path is a timestamped file there.
    #[test]
    fn test_the_path_is_read_the_way_the_shell_reads_it() {
        let cwd = Path::new("/home/someone/maps");
        assert_eq!(
            console_path(Some("/tmp/a.png"), cwd, 7),
            PathBuf::from("/tmp/a.png")
        );
        assert_eq!(console_path(Some("shots/a.png"), cwd, 7), cwd.join("shots/a.png"));
        assert_eq!(
            console_path(None, cwd, 1_700_000_000_123),
            cwd.join("mandala-1700000000123.png")
        );
    }

    /// The report names the file and its size, says a skipped frame
    /// wrote nothing, and charges a framing refusal to `frame=`.
    #[test]
    fn test_the_report_names_what_happened() {
        let camera = Camera2D::new(640, 360);
        let written = report(Ok(CaptureOutcome::Written {
            camera,
            path: PathBuf::from("/tmp/a.png"),
            sidecar_path: Some(PathBuf::from("/tmp/a.png.geometry.json")),
        }));
        assert!(
            matches!(&written, ExecResult::Ok(m) if m == "screenshot: 640×360 written to /tmp/a.png, geometry in /tmp/a.png.geometry.json"),
            "{written:?}"
        );
        let skipped = report(Ok(CaptureOutcome::Skipped {
            camera,
            skip: FrameSkip::PrepareFailed,
        }));
        assert!(
            matches!(&skipped, ExecResult::Err(m) if m.contains("prepare_failed") && m.contains("nothing was written"))
        );
        let framing = report(Err(CaptureError::Invalid {
            param: "region",
            message: "too large".into(),
        }));
        assert!(
            matches!(&framing, ExecResult::Err(m) if m == "screenshot: frame: too large"),
            "{framing:?}"
        );
        let busy = report(Err(CaptureError::Busy("a node drag is in progress".into())));
        assert!(matches!(&busy, ExecResult::Err(m) if m.contains("node drag")));
    }
}
