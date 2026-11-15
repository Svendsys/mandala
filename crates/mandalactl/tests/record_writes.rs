// SPDX-License-Identifier: MPL-2.0

//! `record assemble`'s file against the guarded writer's guarantees
//! (#357): a temp name of this run's alone, an exclusive create, no
//! symlink followed, no file replaced unasked, and cleanup of only what
//! this run created — on the GIF path and on the MP4 path, where
//! `ffmpeg` writes only inside a directory this run created for it.
//! Each test drives the real binary.
//!
//! The MP4 path runs a stand-in `ffmpeg` put first on the child's
//! `PATH`: a shell script that writes its last argument the way
//! `ffmpeg -y` does, which is the worst case. It ignores the `-n` it is
//! passed and opens its output `O_WRONLY|O_CREAT|O_TRUNC`, which
//! follows a symlink and replaces what is there. So the tests need no
//! real `ffmpeg` and run the same with or without one on the host. The
//! stand-in never reads the frame list: the list's place inside the
//! private directory, and the video itself, are covered only by
//! `assemble`'s `test_an_mp4_is_made_when_ffmpeg_is_on_the_host`, which
//! runs the real program. It records the output path it was given in
//! `ARGS_LOG`, on every run. `STANDIN_MODE=fail` makes it exit 1
//! without touching its output; `STANDIN_MODE=race` makes it write
//! `RACE_PATH` first, a file appearing between the destination's check
//! and its publication.
//!
//! Unix-only: the stand-in is a shell script (on Windows `ffmpeg` is
//! looked for as `ffmpeg.exe`), the GIF's race is staged with a FIFO,
//! and planting a symlink on Windows takes a privilege. What Windows
//! can test of the same guarantees runs there in `guarded_file`'s own
//! tests and in `assemble`'s unit tests.
#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

const STANDIN: &str = r#"#!/bin/sh
for a in "$@"; do out=$a; done
if [ -n "$ARGS_LOG" ]; then printf '%s' "$out" > "$ARGS_LOG"; fi
case "$STANDIN_MODE" in
  fail) echo "stand-in ffmpeg: failed as asked" >&2; exit 1 ;;
  race) printf 'appeared meanwhile' > "$RACE_PATH" ;;
esac
printf 'stand-in ftyp video' > "$out"
"#;

/// A one-frame 8x6 recording in `dir`.
fn recording(dir: &Path) {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 8, 6);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("header");
    writer.write_image_data(&[200u8; 8 * 6 * 4]).expect("pixels");
    writer.finish().expect("finish");
    std::fs::write(dir.join("frame_00000.png"), bytes).expect("frame");
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"protocol":1,"fps":30,"pacing":"realtime","width":8,"height":6,"scale":1.0,
            "complete":true,"stopped":"client","duration_ms":100.0,
            "frames":[{"index":0,"t_ms":0.0,"file":"frame_00000.png","document_revision":1,"camera":{}}],
            "dropped_frames":0,"gaps":[]}"#,
    )
    .expect("manifest");
}

/// A directory holding only the stand-in `ffmpeg`.
fn standin() -> tempfile::TempDir {
    let bin = tempfile::tempdir().expect("a bin dir");
    let path = bin.path().join("ffmpeg");
    std::fs::write(&path, STANDIN).expect("stand-in");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    bin
}

/// `mandalactl record assemble <dir> <extra>`, with `bin` first on
/// `PATH` when given.
fn assemble(dir: &Path, extra: &[&str], env: &[(&str, &str)], bin: Option<&Path>) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mandalactl"));
    command.args(["record", "assemble"]).arg(dir).args(extra);
    command.env_remove("MANDALA_IPC");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(bin) = bin {
        let mut joined = std::ffi::OsString::from(bin);
        joined.push(":");
        joined.push(std::env::var_os("PATH").unwrap_or_default());
        command.env("PATH", joined);
    }
    for (k, v) in env {
        command.env(k, v);
    }
    command
}

fn run(mut command: Command) -> Output {
    command.output().expect("the binary runs")
}

fn text(out: &Output) -> String {
    format!(
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn out_arg(path: &Path) -> String {
    format!("out={}", path.display())
}

/// The names in `dir`, sorted.
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("readable")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// What the guarded writer left in `dir`: its temp files and private
/// directories (`.mandala-write-*`).
fn leftovers(dir: &Path) -> Vec<String> {
    names_in(dir)
        .into_iter()
        .filter(|name| name.starts_with(".mandala-write-"))
        .collect()
}

fn scratch() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("scratch");
    let dir = temp.path().to_path_buf();
    recording(&dir);
    (temp, dir)
}

/// A run of the binary in the background, killed if the test ends
/// before it does, so a failed test leaves no child blocked on a FIFO.
struct Running(Option<Child>);

impl Running {
    fn start(mut command: Command) -> Self {
        Self(Some(command.spawn().expect("the binary starts")))
    }

    fn finish(mut self) -> Output {
        self.0
            .take()
            .expect("running")
            .wait_with_output()
            .expect("the binary finishes")
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Replace the recording's frame with a FIFO and return its PNG bytes:
/// the GIF writer then blocks opening the frame — after it has checked
/// the destination and created its temp file — until the test feeds it.
fn frame_as_fifo(dir: &Path) -> Vec<u8> {
    let frame = dir.join("frame_00000.png");
    let bytes = std::fs::read(&frame).expect("frame");
    std::fs::remove_file(&frame).expect("rm frame");
    let made = Command::new("mkfifo").arg(&frame).status().expect("mkfifo");
    assert!(made.success(), "mkfifo");
    bytes
}

fn feed(dir: &Path, bytes: &[u8]) {
    let mut fifo = std::fs::OpenOptions::new()
        .write(true)
        .open(dir.join("frame_00000.png"))
        .expect("open fifo");
    fifo.write_all(bytes).expect("feed");
}

/// Wait until the running assembly has created its temp file in `dir`,
/// at most ten seconds.
fn wait_for_a_temp_file(dir: &Path) {
    for _ in 0..1000 {
        if !leftovers(dir).is_empty() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("precondition: no temp file appeared in {}", dir.display());
}

/// **A leftover beside the destination neither blocks a run nor is
/// touched**, on both paths. The leftover sits at `<out>.partial`, the
/// fixed name the writer used before #357, where a crash left it and
/// every later GIF run was refused; the MP4 run overwrote it and moved
/// it into place as the video.
///
/// Control: the former writer (a fixed `<out>.partial`) refuses the GIF
/// and consumes the MP4's leftover.
#[test]
fn test_a_leftover_beside_the_destination_neither_blocks_nor_is_touched() {
    let (_keep, dir) = scratch();
    let bin = standin();
    for (name, format) in [("x.gif", "format=gif"), ("x.mp4", "format=mp4")] {
        let out = dir.join(name);
        let leftover = dir.join(format!("{name}.partial"));
        std::fs::write(&leftover, "stale").expect("seed");
        let done = run(assemble(&dir, &[&out_arg(&out), format], &[], Some(bin.path())));
        assert!(done.status.success(), "{format}: {}", text(&done));
        assert_eq!(
            std::fs::read_to_string(&leftover).expect("kept"),
            "stale",
            "{format}: the leftover is not this run's and is left as it is"
        );
        assert!(out.is_file(), "{format}: the file was made");
    }
    assert_eq!(leftovers(&dir), Vec::<String>::new());
}

/// **Two runs to one destination do not collide**: while the first
/// holds its temp file (blocked reading a frame), a second run to the
/// same file completes, and the first then completes too, each with a
/// temp name of its own.
///
/// Control: the former fixed `<out>.partial` refuses the second run,
/// blaming an "interrupted" run that is still going.
#[test]
fn test_two_runs_to_one_destination_do_not_collide() {
    let (_keep_a, dir_a) = scratch();
    let (_keep_b, dir_b) = scratch();
    let shared = tempfile::tempdir().expect("shared");
    let out = shared.path().join("same.gif");
    let png = frame_as_fifo(&dir_a);
    let first = Running::start(assemble(&dir_a, &[&out_arg(&out), "overwrite=true"], &[], None));
    wait_for_a_temp_file(shared.path());
    let second = run(assemble(&dir_b, &[&out_arg(&out), "overwrite=true"], &[], None));
    feed(&dir_a, &png);
    let first = first.finish();
    assert!(second.status.success(), "the second run: {}", text(&second));
    assert!(first.status.success(), "the first run: {}", text(&first));
    assert_eq!(names_in(shared.path()), vec!["same.gif"]);
}

/// **GIF: a file that appears after the check is not replaced**
/// without `overwrite`: the publish is `link(2)`, which fails on an
/// existing name, and the run is refused leaving that file and no temp
/// file behind.
///
/// Control: publishing with `rename` (the former writer) replaces the
/// file that appeared.
#[test]
fn test_gif_a_file_that_appears_after_the_check_is_not_replaced() {
    let (_keep, dir) = scratch();
    let out = dir.join("x.gif");
    let png = frame_as_fifo(&dir);
    let child = Running::start(assemble(&dir, &[&out_arg(&out)], &[], None));
    wait_for_a_temp_file(&dir);
    std::fs::write(&out, "precious").expect("appears meanwhile");
    feed(&dir, &png);
    let done = child.finish();
    assert!(!done.status.success(), "refused: {}", text(&done));
    assert!(
        String::from_utf8_lossy(&done.stderr).contains("exists; pass overwrite=true"),
        "refused because the file appeared: {}",
        text(&done)
    );
    assert_eq!(std::fs::read(&out).expect("out"), b"precious");
    assert_eq!(leftovers(&dir), Vec::<String>::new());
}

/// **MP4: the same**, the file appearing while `ffmpeg` runs; the
/// publish refuses it, saying it exists, and the directory this run
/// made for `ffmpeg` is removed.
///
/// Control: the former writer's `rename` replaces it; so does a
/// `PrivateDir::publish` that neither looks at the destination nor
/// links (either alone still refuses).
#[test]
fn test_mp4_a_file_that_appears_after_the_check_is_not_replaced() {
    let (_keep, dir) = scratch();
    let bin = standin();
    let out = dir.join("x.mp4");
    let race = out.display().to_string();
    let done = run(assemble(
        &dir,
        &[&out_arg(&out), "format=mp4"],
        &[("STANDIN_MODE", "race"), ("RACE_PATH", &race)],
        Some(bin.path()),
    ));
    assert!(!done.status.success(), "refused: {}", text(&done));
    assert!(
        String::from_utf8_lossy(&done.stderr).contains("exists; pass overwrite=true"),
        "refused because the file appeared: {}",
        text(&done)
    );
    assert_eq!(std::fs::read_to_string(&out).expect("out"), "appeared meanwhile");
    assert_eq!(leftovers(&dir), Vec::<String>::new());
}

/// **An existing destination is refused before `ffmpeg` runs** (ruling
/// B38.1, addendum §5): with `x.mp4` there and no `overwrite`, the run
/// is refused naming it, the stand-in never ran (it logs every run to
/// `ARGS_LOG`), the file is kept, and nothing of this run's is left.
/// The same run with `overwrite=true` then reaches the stand-in, which
/// shows the log would have been there.
///
/// Control: without the up-front `check` in `assemble`, `ffmpeg` runs
/// to completion and only the publish refuses.
#[test]
fn test_an_existing_mp4_is_refused_before_ffmpeg_runs() {
    let (_keep, dir) = scratch();
    let bin = standin();
    let log = bin.path().join("args.log");
    let out = dir.join("x.mp4");
    std::fs::write(&out, "precious").expect("seed");
    let log_arg = log.display().to_string();
    let done = run(assemble(
        &dir,
        &[&out_arg(&out), "format=mp4"],
        &[("ARGS_LOG", &log_arg)],
        Some(bin.path()),
    ));
    assert!(!done.status.success(), "refused: {}", text(&done));
    assert!(
        String::from_utf8_lossy(&done.stderr)
            .contains(&format!("`{}` exists; pass overwrite=true", out.display())),
        "{}",
        text(&done)
    );
    assert!(!log.exists(), "ffmpeg was never run");
    assert_eq!(std::fs::read_to_string(&out).expect("kept"), "precious");
    assert_eq!(leftovers(&dir), Vec::<String>::new());

    let done = run(assemble(
        &dir,
        &[&out_arg(&out), "format=mp4", "overwrite=true"],
        &[("ARGS_LOG", &log_arg)],
        Some(bin.path()),
    ));
    assert!(done.status.success(), "{}", text(&done));
    assert!(
        log.exists(),
        "precondition: a run that reaches ffmpeg leaves the log"
    );
}

/// **`ffmpeg` never opens the destination or a guessable name**: the
/// output it is handed is inside a `.mandala-write-*.d` directory this
/// run created beside the destination, so a symlink planted at the
/// former temp name (`<out>.partial`) catches nothing, and the
/// directory is gone afterwards.
///
/// Control: the former writer hands `ffmpeg` `<out>.partial`, and the
/// victim behind the planted link receives the video.
#[test]
fn test_ffmpeg_writes_only_inside_a_directory_of_this_runs() {
    let (_keep, dir) = scratch();
    let bin = standin();
    let log = bin.path().join("args.log");
    let out = dir.join("x.mp4");
    let victim = dir.join("victim.txt");
    std::fs::write(&victim, "precious").expect("victim");
    std::os::unix::fs::symlink(&victim, dir.join("x.mp4.partial")).expect("plant");
    let log_arg = log.display().to_string();
    let done = run(assemble(
        &dir,
        &[&out_arg(&out), "format=mp4"],
        &[("ARGS_LOG", &log_arg)],
        Some(bin.path()),
    ));
    assert!(done.status.success(), "{}", text(&done));
    assert_eq!(std::fs::read_to_string(&victim).expect("victim"), "precious");
    let given = PathBuf::from(std::fs::read_to_string(&log).expect("the stand-in ran"));
    let private = given.parent().expect("inside a directory");
    let private_name = private.file_name().expect("named").to_string_lossy().into_owned();
    assert_eq!(
        private.parent(),
        Some(dir.as_path()),
        "beside the destination: {}",
        given.display()
    );
    assert!(
        private_name.starts_with(".mandala-write-") && private_name.ends_with(".d"),
        "a private directory of this run's: {}",
        given.display()
    );
    assert_eq!(
        std::fs::read_to_string(&out).expect("published"),
        "stand-in ftyp video"
    );
    assert_eq!(leftovers(&dir), Vec::<String>::new(), "the directory is removed");
}

/// **A failed `ffmpeg` removes only what this run created**: its
/// private directory is gone, an `<out>.partial` it never created is
/// left as it is, and no file is published.
///
/// Control: the former writer's `discard` removes the `<out>.partial`.
#[test]
fn test_a_failed_ffmpeg_removes_only_what_this_run_created() {
    let (_keep, dir) = scratch();
    let bin = standin();
    let out = dir.join("x.mp4");
    let partial = dir.join("x.mp4.partial");
    std::fs::write(&partial, "not yours").expect("seed");
    let done = run(assemble(
        &dir,
        &[&out_arg(&out), "format=mp4"],
        &[("STANDIN_MODE", "fail")],
        Some(bin.path()),
    ));
    assert!(
        !done.status.success(),
        "precondition: the stand-in failed: {}",
        text(&done)
    );
    assert!(
        String::from_utf8_lossy(&done.stderr).contains("failed as asked"),
        "precondition: it is the stand-in's failure: {}",
        text(&done)
    );
    assert_eq!(std::fs::read_to_string(&partial).expect("kept"), "not yours");
    assert!(!out.exists(), "nothing was published");
    assert_eq!(
        leftovers(&dir),
        Vec::<String>::new(),
        "the private directory is removed"
    );
}

/// **A symlink at the destination leads to the file written, and the
/// no-clobber rule is held to that file** (ruling B38.1 §6, as for
/// `maptool export`), on both paths: without `overwrite` the run is
/// refused and the file the link leads to is untouched; with it that
/// file is replaced and the link stays a link to it. A dangling link is
/// refused either way, and nothing is created at its target.
///
/// Controls: without the `real_file` step the run with `overwrite` is
/// refused as a symlink; the former writer replaced the link itself
/// with a regular file, leaving the target as it was.
#[test]
fn test_a_symlinked_destination_is_written_through_only_with_overwrite() {
    let (_keep, dir) = scratch();
    let bin = standin();
    for (name, format) in [("x.gif", "format=gif"), ("x.mp4", "format=mp4")] {
        let out = dir.join(name);
        let target = dir.join(format!("{name}.target"));
        std::fs::write(&target, "precious").expect("target");
        std::os::unix::fs::symlink(&target, &out).expect("link");

        let refused = run(assemble(&dir, &[&out_arg(&out), format], &[], Some(bin.path())));
        assert!(!refused.status.success(), "{format}: {}", text(&refused));
        let said = String::from_utf8_lossy(&refused.stderr);
        assert!(
            said.contains(&format!("`{}` leads to `", out.display()))
                && said.contains("which exists; pass overwrite=true"),
            "the refusal names the link typed and where it leads: {}",
            text(&refused)
        );
        assert_eq!(std::fs::read_to_string(&target).expect("kept"), "precious");

        let done = run(assemble(
            &dir,
            &[&out_arg(&out), format, "overwrite=true"],
            &[],
            Some(bin.path()),
        ));
        assert!(done.status.success(), "{format}: {}", text(&done));
        assert_ne!(
            std::fs::read(&target).expect("replaced"),
            b"precious",
            "{format}: the file the link leads to is replaced"
        );
        assert_eq!(
            std::fs::read_link(&out).expect("still a link"),
            target,
            "{format}"
        );

        let dangling = dir.join(format!("dangling-{name}"));
        let nowhere = dir.join(format!("nowhere-{name}"));
        std::os::unix::fs::symlink(&nowhere, &dangling).expect("link");
        for extra in ["overwrite=false", "overwrite=true"] {
            let refused = run(assemble(
                &dir,
                &[&out_arg(&dangling), format, extra],
                &[],
                Some(bin.path()),
            ));
            assert!(!refused.status.success(), "{format}: {}", text(&refused));
            assert!(
                String::from_utf8_lossy(&refused.stderr).contains("does not exist"),
                "{}",
                text(&refused)
            );
            assert!(!nowhere.exists(), "{format}: nothing was created at the target");
        }
    }
    assert_eq!(leftovers(&dir), Vec::<String>::new());
}
