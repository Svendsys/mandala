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
//! saving — projecting the zoom-dependent roles into the capture's own
//! scratch scene for a capture that frames another zoom, so the live
//! scene is never written for a picture of it — is `app::capture_flow`
//! with `app::capture_scene`. IPC and the console both call that one
//! function; neither has a capture path of its own.
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
use crate::application::guarded_file::{self, PublishRefusal};
#[cfg(not(target_arch = "wasm32"))]
use crate::application::renderer::{CapturedPixels, FrameSkip};

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
/// The existence check is for the common case and the message; the
/// write itself refuses again atomically ([`guarded_file::publish`]),
/// so a file appearing in between is still never clobbered.
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
    if !overwrite {
        for target in std::iter::once(&path).chain(sidecar.as_ref()) {
            if std::fs::symlink_metadata(target).is_ok() {
                return Err(CaptureError::Invalid {
                    param: "path",
                    message: format!("`{}` exists; set `overwrite` to replace it", target.display()),
                });
            }
        }
    }
    Ok(Target {
        path,
        sidecar,
        overwrite,
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// Write a drawn frame to `target`: the PNG, then — when the target has
/// one — the sidecar beside it, both through [`guarded_file::publish`],
/// and record whatever landed in the artifacts dir so its budget holds.
/// `sidecar` is the map to write, present exactly when the target names
/// a sidecar path.
///
/// A sidecar that fails after its PNG was written says so: the PNG is
/// good and stays. `progress` is called as the PNG encodes
/// ([`png::encode`]).
pub(crate) fn write(
    target: &Target,
    pixels: &CapturedPixels,
    sidecar: Option<&Sidecar>,
    artifacts: &mut Artifacts,
    progress: &mut dyn FnMut(),
) -> Result<(), CaptureError> {
    let refused = |refusal: PublishRefusal| match refusal {
        PublishRefusal::Invalid(message) => CaptureError::Invalid {
            param: "path",
            message,
        },
        PublishRefusal::Internal(message) => CaptureError::Internal(message),
    };
    let png_bytes = guarded_file::publish(&target.path, target.overwrite, |out| {
        png::encode(pixels, out, &mut *progress)
    })
    .map_err(refused)?;
    let mut written = vec![(target.path.clone(), png_bytes)];
    if let (Some(sidecar), Some(sidecar_path)) = (sidecar, target.sidecar.as_ref()) {
        let json_bytes = guarded_file::publish(sidecar_path, target.overwrite, |out| {
            serde_json::to_writer(out, sidecar).map_err(|e| e.to_string())
        })
        .map_err(|refusal| {
            artifacts.record(&written);
            let (PublishRefusal::Invalid(message) | PublishRefusal::Internal(message)) = refusal;
            CaptureError::Internal(format!(
                "the PNG was written to `{}`, but its sidecar was not: {message}",
                target.path.display()
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
    use crate::application::guarded_file::tests::{names_in, scratch_dir};
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
