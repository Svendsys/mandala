// SPDX-License-Identifier: MPL-2.0

//! Screenshots: the engine behind the IPC `capture.screenshot` command
//! and the `screenshot` console verb (`format/ipc.md`
//! §`capture.screenshot`; CONCEPTS §5 "Screenshot capture").
//!
//! **One engine, two surfaces.** A capture is planned (`plan`), its
//! destination resolved and checked (`resolve`) — both before
//! anything is drawn, so a refused request costs no frame — then drawn
//! offscreen by the renderer through the very frame pair the window
//! draws with (`renderer::capture`), and written here (`write()`): the
//! PNG, its geometry sidecar, and — for a server-chosen path — the
//! bounded artifacts dir. The app half that sits between planning and
//! saving — the view's sync, then the frame drawn through the capture's
//! own camera, which the glyph layers being in canvas space makes the
//! whole of it — is `app::capture_flow`. IPC and the console both call
//! that one function; neither has a capture path of its own.
//!
//! **Truthful.** A frame the renderer did not draw is reported as not
//! drawn (`rendered: false`, with the reason), and no file is written:
//! a PNG of a skipped frame would be indistinguishable from a frame
//! that rendered black.
//!
//! **Two targets, one sidecar.** The geometry sidecar ([`geometry`]) is
//! what both targets attach to a capture: the browser's canvas
//! capture (`format/ipc.md` §Transport "Browser") carries it inline
//! in the reply, as it carries the PNG. Everything else here — the
//! offscreen plan, the PNG encoder, the artifacts dir, the recorder
//! and the file writes — is native, item by item (`CLAUDE.md`
//! "Dual-target status", "Screenshot capture").

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod artifacts;
pub(crate) mod geometry;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod plan;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod png;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod record;

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
use baumhard::gfx_structs::camera::Camera2D;

#[cfg(not(target_arch = "wasm32"))]
use crate::application::renderer::{CapturedPixels, FrameSkip};
#[cfg(not(target_arch = "wasm32"))]
use baumhard::util::guarded_file::{self, PublishRefusal};

#[cfg(not(target_arch = "wasm32"))]
use artifacts::Artifacts;
#[cfg(not(target_arch = "wasm32"))]
use geometry::Sidecar;

#[cfg(not(target_arch = "wasm32"))]
/// The suffix the geometry sidecar adds to the PNG's file name.
pub(crate) const SIDECAR_SUFFIX: &str = ".geometry.json";

#[cfg(not(target_arch = "wasm32"))]
/// Where a capture's PNG goes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Destination {
    /// A path the caller chose. It must be absolute, and an existing
    /// file there is replaced only with `overwrite` — or when the path
    /// is in the artifacts dir, which is the app's own.
    Path { path: PathBuf, overwrite: bool },
    /// A fresh file in the artifacts dir.
    Artifacts,
}

#[cfg(not(target_arch = "wasm32"))]
/// What a capture did.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CaptureOutcome {
    /// The frame was drawn and written.
    Written {
        /// The camera the image was drawn through: its viewport is the
        /// image's size and its pixel ratio the scale.
        camera: Camera2D,
        path: PathBuf,
        /// The geometry sidecar, when one was asked for.
        sidecar_path: Option<PathBuf>,
    },
    /// The renderer did not draw the frame, so nothing was written.
    Skipped { camera: Camera2D, skip: FrameSkip },
}

#[cfg(not(target_arch = "wasm32"))]
/// Why a capture was refused.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CaptureError {
    /// A parameter asked for something that cannot be drawn or written.
    Invalid { param: &'static str, message: String },
    /// The capture could not be honored: the GPU, the disk.
    Internal(String),
}

#[cfg(not(target_arch = "wasm32"))]
impl CaptureError {
    /// The refusal's words, without the parameter or the class it was
    /// charged to: for a caller that reports every refusal the same way.
    pub(crate) fn into_message(self) -> String {
        match self {
            CaptureError::Invalid { message, .. } | CaptureError::Internal(message) => message,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<plan::PlanError> for CaptureError {
    fn from(error: plan::PlanError) -> Self {
        CaptureError::Invalid {
            param: error.param,
            message: error.message,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// `path`'s sidecar: the same name with [`SIDECAR_SUFFIX`] after it.
pub(crate) fn sidecar_path_for(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(SIDECAR_SUFFIX);
    PathBuf::from(name)
}

#[cfg(not(target_arch = "wasm32"))]
/// Where a capture will be written, resolved and checked before
/// anything is drawn: a refused path costs no frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Target {
    pub(crate) path: PathBuf,
    /// The sidecar's path, when one was asked for.
    pub(crate) sidecar: Option<PathBuf>,
    overwrite: bool,
}

#[cfg(not(target_arch = "wasm32"))]
/// Resolve `destination` into a [`Target`]: a fresh name in the
/// artifacts dir, or the caller's path — absolute, and, unless
/// `overwrite` (or the artifacts dir, which is the app's own) allows
/// it, with nothing at it or at its sidecar's name yet.
///
/// The look at each file is the writer's own ([`guarded_file::check`]),
/// taken before a frame is drawn, so the refusal is the one the write
/// would give, in the same words: a symlink or a directory there, or an
/// existing file without `overwrite`. It is for the common case and the
/// message; the write itself refuses again atomically
/// ([`guarded_file::publish`]), so a file appearing in between is still
/// never clobbered.
pub(crate) fn resolve(
    destination: Destination,
    sidecar: bool,
    artifacts: &mut Artifacts,
) -> Result<Target, CaptureError> {
    let (path, overwrite) = match destination {
        Destination::Artifacts => (
            artifacts
                .next_path("screenshot", "png")
                .map_err(CaptureError::Internal)?,
            true,
        ),
        Destination::Path { path, overwrite } => {
            if !path.is_absolute() {
                return Err(CaptureError::Invalid {
                    param: "path",
                    message: format!("`{}` is not an absolute path", path.display()),
                });
            }
            let overwrite = overwrite || artifacts.holds(&path);
            (path, overwrite)
        }
    };
    let sidecar = sidecar.then(|| sidecar_path_for(&path));
    look(&path, sidecar.as_ref(), overwrite)?;
    Ok(Target {
        path,
        sidecar,
        overwrite,
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// The writer's look ([`guarded_file::check`]) at the PNG's file and,
/// when one will be written, the sidecar's: both, before either is
/// written, so a sidecar in the way leaves no PNG behind. Taken twice
/// (ruling B38.1, second addendum §8): by [`resolve`] before the frame
/// is drawn, and by [`write()`] right before its first write, for a file
/// that appeared in between.
fn look(path: &Path, sidecar: Option<&PathBuf>, overwrite: bool) -> Result<(), CaptureError> {
    for target in std::iter::once(path).chain(sidecar.map(PathBuf::as_path)) {
        guarded_file::check(target, overwrite).map_err(refused)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
/// The writer's refusal as this command's: a path that is the caller's
/// mistake is `invalid_params` on `path`, in the writer's words.
fn refused(refusal: PublishRefusal) -> CaptureError {
    match refusal {
        PublishRefusal::Invalid(message) => CaptureError::Invalid {
            param: "path",
            message,
        },
        PublishRefusal::Internal(message) => CaptureError::Internal(message),
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Write a drawn frame to `target`: the PNG, then — when the target has
/// one — the sidecar beside it, both through [`guarded_file::publish`],
/// and record whatever landed in the artifacts dir so its budget holds.
/// `sidecar` is the map to write, present exactly when the target names
/// a sidecar path.
///
/// Both files are looked at again first ([`look`]): one that appeared
/// since [`resolve`] is refused with nothing written. A sidecar that
/// still fails after its PNG was written says so: the PNG is good and
/// stays. `progress` is called as the PNG encodes ([`png::encode`]).
pub(crate) fn write(
    target: &Target,
    pixels: &CapturedPixels,
    sidecar: Option<&Sidecar>,
    artifacts: &mut Artifacts,
    progress: &mut dyn FnMut(),
) -> Result<(), CaptureError> {
    let sidecar = sidecar.zip(target.sidecar.as_ref());
    look(&target.path, sidecar.map(|(_, path)| path), target.overwrite)?;
    let png_bytes = guarded_file::publish(&target.path, target.overwrite, |out| {
        png::encode(pixels, out, &mut *progress)
    })
    .map_err(refused)?;
    let mut written = vec![(target.path.clone(), png_bytes)];
    if let Some((sidecar, sidecar_path)) = sidecar {
        let json_bytes = guarded_file::publish(sidecar_path, target.overwrite, |out| {
            serde_json::to_writer(out, sidecar).map_err(|e| e.to_string())
        })
        .map_err(|refusal| {
            artifacts.record(&written);
            CaptureError::Internal(format!(
                "the PNG was written to `{}`, but its sidecar was not: {}",
                target.path.display(),
                refusal.message()
            ))
        })?;
        written.push((sidecar_path.clone(), json_bytes));
    }
    artifacts.record(&written);
    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use baumhard::util::test_temp::{names_in, scratch_dir};
    use geometry::{CameraInfo, SurfaceInfo};

    fn pixels() -> CapturedPixels {
        CapturedPixels {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 128],
        }
    }

    fn sidecar() -> Sidecar {
        Sidecar {
            protocol: 1,
            document_revision: 0,
            surface: SurfaceInfo {
                width: 2,
                height: 1,
                scale: 1.0,
            },
            camera: CameraInfo {
                center_x: 0.0,
                center_y: 0.0,
                zoom: 1.0,
            },
            nodes: Vec::new(),
        }
    }

    /// [`resolve`] then [`write`], as the engine runs them around the
    /// frame: the PNG's path and the sidecar's.
    fn save(
        map: Option<&Sidecar>,
        destination: Destination,
        artifacts: &mut Artifacts,
    ) -> Result<(PathBuf, Option<PathBuf>), CaptureError> {
        let target = resolve(destination, map.is_some(), artifacts)?;
        write(&target, &pixels(), map, artifacts, &mut || {})?;
        Ok((target.path, target.sidecar))
    }

    #[test]
    fn test_the_sidecar_is_named_after_the_png() {
        assert_eq!(
            sidecar_path_for(Path::new("/tmp/shot.png")),
            PathBuf::from("/tmp/shot.png.geometry.json")
        );
    }

    /// A client path gets the PNG — decoding to the frame's pixels —
    /// and the sidecar beside it, and neither counts against the
    /// artifacts dir, which is never made.
    #[test]
    fn test_a_client_path_gets_the_png_and_its_sidecar() {
        let dir = scratch_dir("save-client");
        let root = scratch_dir("save-client-root");
        let mut artifacts = Artifacts::under(root.clone(), artifacts::BUDGET_BYTES);
        let path = dir.join("shot.png");
        let (png, side) = save(
            Some(&sidecar()),
            Destination::Path {
                path: path.clone(),
                overwrite: false,
            },
            &mut artifacts,
        )
        .expect("saved");
        assert_eq!(
            (png.clone(), side.clone()),
            (path.clone(), Some(dir.join("shot.png.geometry.json")))
        );
        let decoded = png::tests::decode(&std::fs::read(&png).expect("read"));
        assert_eq!(decoded, (2, 1, pixels().rgba));
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(side.expect("sidecar")).expect("read")).expect("JSON");
        assert_eq!(json["surface"]["width"], 2);
        assert_eq!(
            names_in(&root),
            Vec::<String>::new(),
            "the artifacts dir was not needed"
        );

        // Without the sidecar, only the PNG.
        let lone = dir.join("lone.png");
        save(
            None,
            Destination::Path {
                path: lone.clone(),
                overwrite: false,
            },
            &mut artifacts,
        )
        .expect("saved");
        assert_eq!(
            names_in(&dir),
            vec!["lone.png", "shot.png", "shot.png.geometry.json"]
        );
        std::fs::remove_dir_all(&dir).expect("cleanup");
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    /// An existing PNG — or an existing sidecar beside a new PNG — is
    /// refused by `resolve`, before a frame is drawn or anything is
    /// written, unless `overwrite`; a relative path is refused outright.
    #[test]
    fn test_existing_files_are_refused_before_anything_is_written() {
        let dir = scratch_dir("save-guard");
        let mut artifacts = Artifacts::under(dir.join("unused"), artifacts::BUDGET_BYTES);
        let path = dir.join("shot.png");
        std::fs::write(sidecar_path_for(&path), "precious").expect("seed");
        let refused = save(
            Some(&sidecar()),
            Destination::Path {
                path: path.clone(),
                overwrite: false,
            },
            &mut artifacts,
        );
        assert!(
            matches!(&refused, Err(CaptureError::Invalid { param: "path", message }) if message.contains("geometry.json")),
            "{refused:?}"
        );
        assert!(!path.exists(), "no PNG was left behind the refused sidecar");
        save(
            Some(&sidecar()),
            Destination::Path {
                path: path.clone(),
                overwrite: true,
            },
            &mut artifacts,
        )
        .expect("overwrite replaces both");
        assert_ne!(
            std::fs::read_to_string(sidecar_path_for(&path)).expect("read"),
            "precious"
        );

        let relative = save(
            None,
            Destination::Path {
                path: PathBuf::from("shot.png"),
                overwrite: true,
            },
            &mut artifacts,
        );
        assert!(
            matches!(relative, Err(CaptureError::Invalid { param: "path", .. })),
            "{relative:?}"
        );
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// **The look before the frame is the writer's own, in the writer's
    /// words**: an existing PNG, or an existing sidecar beside a new one,
    /// is refused with the message [`guarded_file::publish`] gives,
    /// ending in the remedy the writer publishes
    /// (`guarded_file::OVERWRITE_HINT`, which `mandalactl` rewrites into
    /// its own spelling); a directory or a symlink there, at the PNG's
    /// name or at the sidecar's, is named as what it is, with or without
    /// `overwrite`; a directory that is not there is refused, and so is
    /// a sidecar name the filesystem cannot hold (third addendum §14);
    /// and with `overwrite` an existing file passes.
    ///
    /// Controls: a refusal composed here in other words fails the first
    /// two; a look that only asks whether something exists, and only
    /// without `overwrite`, calls the directory and the links existing
    /// files, lets them pass with `overwrite`, and passes the missing
    /// directory; a look that reads "name too long" as "nothing there"
    /// passes the long name.
    #[test]
    fn test_resolve_refuses_in_the_writers_own_words() {
        let dir = scratch_dir("resolve-words");
        let mut artifacts = Artifacts::under(dir.join("unused"), artifacts::BUDGET_BYTES);
        let mut look = |path: &Path, overwrite: bool, sidecar: bool| {
            let destination = Destination::Path {
                path: path.to_path_buf(),
                overwrite,
            };
            resolve(destination, sidecar, &mut artifacts)
        };
        let invalid = |message: String| {
            Err(CaptureError::Invalid {
                param: "path",
                message,
            })
        };
        let exists = |at: &Path| format!("`{}` exists; {}", at.display(), guarded_file::OVERWRITE_HINT);

        let taken = dir.join("taken.png");
        std::fs::write(&taken, "precious").expect("seed");
        assert_eq!(look(&taken, false, false), invalid(exists(&taken)));
        assert_eq!(
            look(&taken, true, false).map(|target| target.path),
            Ok(taken.clone())
        );

        let free = dir.join("free.png");
        let beside = sidecar_path_for(&free);
        std::fs::write(&beside, "precious").expect("seed");
        assert_eq!(look(&free, false, true), invalid(exists(&beside)));
        assert_eq!(
            look(&free, false, false).map(|target| target.path),
            Ok(free.clone()),
            "a sidecar that will not be written is not in the way"
        );

        let folder = dir.join("folder.png");
        std::fs::create_dir(&folder).expect("a directory");
        for overwrite in [false, true] {
            assert_eq!(
                look(&folder, overwrite, false),
                invalid(format!("`{}` is a directory", folder.display())),
                "overwrite={overwrite}"
            );
        }
        let nowhere = dir.join("missing").join("shot.png");
        for overwrite in [false, true] {
            assert_eq!(
                look(&nowhere, overwrite, true),
                invalid(format!(
                    "the parent directory of `{}` does not exist",
                    nowhere.display()
                )),
                "overwrite={overwrite}"
            );
        }
        #[cfg(unix)]
        {
            let link = dir.join("link.png");
            std::os::unix::fs::symlink(&taken, &link).expect("symlink");
            let dangling = dir.join("dangling.png");
            std::os::unix::fs::symlink(dir.join("nowhere.png"), &dangling).expect("symlink");
            let linked_side = dir.join("side.png");
            let side_link = sidecar_path_for(&linked_side);
            std::os::unix::fs::symlink(&taken, &side_link).expect("symlink");
            for overwrite in [false, true] {
                for (path, refused) in [(&link, &link), (&dangling, &dangling), (&linked_side, &side_link)] {
                    assert_eq!(
                        look(path, overwrite, true),
                        invalid(format!(
                            "`{}` is a symlink, which is never followed",
                            refused.display()
                        )),
                        "overwrite={overwrite}"
                    );
                }
            }
            assert_eq!(std::fs::read_to_string(&taken).expect("kept"), "precious");

            // An image name at the directory's limit: the sidecar's is
            // fourteen bytes over it. The limit is found, not assumed.
            let limit = (1..=255)
                .rev()
                .find(|len| {
                    let probe = dir.join("p".repeat(*len));
                    let made = std::fs::write(&probe, b"").is_ok();
                    let _ = std::fs::remove_file(&probe);
                    made
                })
                .expect("a directory that takes a name");
            let at_the_limit = dir.join(format!("{}.png", "a".repeat(limit - 4)));
            assert_eq!(
                look(&at_the_limit, false, true),
                invalid(format!(
                    "a name in `{}` is too long for this filesystem, or is not one it allows",
                    sidecar_path_for(&at_the_limit).display()
                ))
            );
            assert_eq!(
                look(&at_the_limit, false, false).map(|target| target.path),
                Ok(at_the_limit.clone()),
                "precondition: the image's own name fits"
            );
        }
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// **A file that appears while the frame is drawn is refused before
    /// the first write, with nothing written** (ruling B38.1, second
    /// addendum §8): `resolve` passes, then a file appears at the PNG's
    /// name, or at the sidecar's, and `write` looks again and refuses it
    /// in the writer's words, leaving it and writing no PNG.
    ///
    /// Control: a `write` that does not look again writes the PNG and
    /// only then finds the sidecar in the way.
    #[test]
    fn test_a_file_that_appears_after_resolve_is_refused_before_the_first_write() {
        let dir = scratch_dir("second-look");
        let mut artifacts = Artifacts::under(dir.join("unused"), artifacts::BUDGET_BYTES);
        for appears_at_the_sidecar in [false, true] {
            let path = dir.join(format!("shot-{appears_at_the_sidecar}.png"));
            let destination = Destination::Path {
                path: path.clone(),
                overwrite: false,
            };
            let target =
                resolve(destination, true, &mut artifacts).expect("precondition: nothing is in the way yet");
            let appeared = match appears_at_the_sidecar {
                true => sidecar_path_for(&path),
                false => path.clone(),
            };
            std::fs::write(&appeared, "appeared").expect("appears");
            let refused = write(&target, &pixels(), Some(&sidecar()), &mut artifacts, &mut || {});
            assert_eq!(
                refused,
                Err(CaptureError::Invalid {
                    param: "path",
                    message: format!(
                        "`{}` exists; {}",
                        appeared.display(),
                        guarded_file::OVERWRITE_HINT
                    ),
                })
            );
            assert_eq!(std::fs::read_to_string(&appeared).expect("kept"), "appeared");
            let name = appeared
                .file_name()
                .expect("named")
                .to_string_lossy()
                .into_owned();
            assert_eq!(names_in(&dir), vec![name], "nothing else was written");
            std::fs::remove_file(&appeared).expect("clear");
        }
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// **A sidecar that fails after the PNG has landed says the PNG was
    /// written**, and where, and is an internal error: the PNG is good
    /// and stays. Staged with no seam, through the hook `write` already
    /// has: its `progress` callback runs while the PNG encodes, after
    /// the second look, and here it puts a file at the sidecar's name.
    /// The PNG is published, the sidecar's write finds the name taken.
    /// `mandalactl` says the same for a browser's screenshot.
    ///
    /// Control: a sidecar refusal mapped the way the PNG's is does not
    /// say the PNG was written.
    #[test]
    fn test_a_sidecar_that_fails_after_the_png_says_the_png_was_written() {
        let dir = scratch_dir("sidecar-after");
        let mut artifacts = Artifacts::under(dir.join("unused"), artifacts::BUDGET_BYTES);
        let path = dir.join("shot.png");
        let beside = sidecar_path_for(&path);
        let destination = Destination::Path {
            path: path.clone(),
            overwrite: false,
        };
        let target = resolve(destination, true, &mut artifacts).expect("precondition: nothing is in the way");
        let mut planted = 0;
        let failed = write(&target, &pixels(), Some(&sidecar()), &mut artifacts, &mut || {
            std::fs::write(&beside, "appeared").expect("appears");
            planted += 1;
        });
        assert!(planted > 0, "precondition: the PNG's encoding called `progress`");
        assert!(path.is_file(), "precondition: the PNG landed: {failed:?}");
        assert_eq!(
            failed,
            Err(CaptureError::Internal(format!(
                "the PNG was written to `{}`, but its sidecar was not: `{}` exists; {}",
                path.display(),
                beside.display(),
                guarded_file::OVERWRITE_HINT
            )))
        );
        assert_eq!(std::fs::read_to_string(&beside).expect("kept"), "appeared");
        assert_eq!(names_in(&dir), vec!["shot.png", "shot.png.geometry.json"]);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// A server-chosen capture lands in the artifacts dir under a fresh
    /// name each time, and a client path into that dir may replace what
    /// is there without `overwrite`, because the dir is the app's.
    #[test]
    fn test_artifacts_captures_are_fresh_and_the_dir_is_overwritable() {
        let root = scratch_dir("save-artifacts");
        let mut artifacts = Artifacts::under(root.clone(), artifacts::BUDGET_BYTES);
        let (first, first_side) =
            save(Some(&sidecar()), Destination::Artifacts, &mut artifacts).expect("saved");
        let (second, _) = save(None, Destination::Artifacts, &mut artifacts).expect("saved");
        assert_ne!(first, second);
        assert!(first.exists() && second.exists() && first_side.expect("sidecar").exists());

        save(
            None,
            Destination::Path {
                path: first.clone(),
                overwrite: false,
            },
            &mut artifacts,
        )
        .expect("inside the artifacts dir, overwriting is always allowed");
        artifacts.remove();
        assert!(!first.exists() && !second.exists());
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
