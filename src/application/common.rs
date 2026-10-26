// SPDX-License-Identifier: MPL-2.0

//! Small cross-cutting types shared between the event loop, the
//! renderer, and console verbs. Each type below carries its own
//! invariant; together they form the "configuration" surface the
//! event loop reads on every frame.

/// Renderer-side command queue entry. Event loop pushes one
/// for each per-frame intent the renderer should react to; the
/// renderer drains them at frame start. Everything that
/// changes GPU state without changing document state goes
/// through here so the model/view boundary (§3) stays clean.
///
/// Variants:
/// - `Noop` — default sentinel; never actually queued by the
///   event loop, but `RenderDecree::default() = Noop` lets
///   builders compile.
/// - `SetFpsDisplay(mode)` — flip the on-screen FPS readout
///   between off / snapshot / debug. See [`FpsDisplayMode`].
/// - `StartRender` — open the per-frame draw gate. Both targets
///   send it once, from their init path, after the first scene
///   is built; the renderer draws nothing before it.
/// - `SetSurfaceSize(w, h)` — propagate a winit `Resized` to
///   the wgpu surface configuration.
/// - `Terminate` — stop drawing: the event loop is about to exit,
///   and a frame after this would present into a surface on its way
///   out.
/// - `CameraPan(dx, dy)` — translate the camera origin by a
///   per-cursor-move delta in canvas pixels (the §3 carve-out
///   for per-frame continuous-gesture state).
/// - `CameraZoom { screen_x, screen_y, factor }` — multiply
///   the camera zoom by `factor`, anchored at the given screen
///   coordinates so the point under the cursor stays put.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum RenderDecree {
    #[default]
    Noop,
    SetFpsDisplay(FpsDisplayMode),
    StartRender,
    SetSurfaceSize(u32, u32),
    // Native-driver-only: `run_native` sends it on window-close;
    // a browser tab has no equivalent teardown event.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Terminate,
    CameraPan(f32, f32),
    CameraZoom {
        screen_x: f32,
        screen_y: f32,
        factor: f32,
    },
}

/// Which FPS readout the renderer should display, if any.
///
/// - `Off` — no overlay; the default.
/// - `Snapshot` — single per-frame FPS number rendered in the
///   corner. Useful for casual monitoring.
/// - `Debug` — the FPS number as a rolling average over recent
///   frames rather than a single frame's, steadier to read while
///   profiling; gated behind `Action::ToggleFpsDebug`. It is not a
///   per-stage breakdown: no stage timings are collected.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FpsDisplayMode {
    Off,
    Snapshot,
    Debug,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_decree_default_is_noop() {
        let decree: RenderDecree = RenderDecree::default();
        assert_eq!(decree, RenderDecree::Noop);
    }
}
