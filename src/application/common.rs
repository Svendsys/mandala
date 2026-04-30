// SPDX-License-Identifier: MPL-2.0

//! Small cross-cutting types shared between the event loop, the
//! renderer, and console verbs. Each type below carries its own
//! invariant; together they form the "configuration" surface the
//! event loop reads on every frame.

use std::time::Duration;
// `web_time` maps to `performance.now()` on wasm32; without this swap
// `Instant::now()` panics with "time not implemented on this platform".
use web_time::Instant;

/// How aggressively the event loop schedules redraws.
///
/// - `OnRequest` — only when an `Action` or input event explicitly
///   requests a redraw. Saves battery when the canvas is idle.
/// - `FpsLimit(n)` — at most `n` frames per second. The current
///   default; matches typical display refresh rates.
/// - `NoLimit` — render every loop iteration. Used for benchmark
///   captures and animation soak tests; not a user-facing default.
#[derive(Copy, Clone, Eq, Hash, PartialEq)]
pub enum RedrawMode {
    OnRequest,
    FpsLimit(usize),
    NoLimit,
}

/// How input events route to the dispatch funnel. Set at startup
/// from CLI / env detection; never mutates during a run.
///
/// - `Direct` — the canonical mode: every event drives `Action`
///   resolution and goes through the dispatch funnel.
/// - `MappedToInstruction` — reserved for future scriptable
///   input remapping (a layer above the keybind table). Today
///   no consumer reaches for it; preserved as an enum slot for
///   the named trajectory.
#[derive(Copy, Clone, Eq, Hash, PartialEq)]
pub enum InputMode {
    Direct,
    MappedToInstruction,
}

/// Renderer-side command queue entry. Event loop pushes one of
/// these per per-frame intent that the renderer should react to;
/// the renderer drains them at frame start. Everything that
/// changes GPU state without changing document state goes through
/// here so the model/view boundary (§3) stays clean.
///
/// Variants:
/// - `Noop` — default sentinel; never actually queued by the
///   event loop, but `RenderDecree::default() = Noop` lets
///   builders compile.
/// - `SetFpsDisplay(mode)` — flip the on-screen FPS readout
///   between off / snapshot / debug. See [`FpsDisplayMode`].
/// - `StartRender` / `StopRender` — gate the per-frame draw
///   loop. WASM uses these around the requestAnimationFrame
///   handshake.
/// - `ReinitAdapter` — discard the current `wgpu::Adapter` and
///   pick a fresh one. Used after a device-lost event.
/// - `SetSurfaceSize(w, h)` — propagate a winit `Resized` to
///   the wgpu surface configuration.
/// - `Terminate` — release GPU resources before the event loop
///   exits.
/// - `CameraPan(dx, dy)` — translate the camera origin by a
///   per-cursor-move delta in canvas pixels (the §3 carve-out
///   for per-frame continuous-gesture state).
/// - `CameraZoom { screen_x, screen_y, factor }` — multiply
///   the camera zoom by `factor`, anchored at the given screen
///   coordinates so the point under the cursor stays put.
#[derive(Clone, Debug, PartialEq)]
pub enum RenderDecree {
    Noop,
    SetFpsDisplay(FpsDisplayMode),
    StartRender,
    StopRender,
    ReinitAdapter,
    SetSurfaceSize(u32, u32),
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
/// - `Debug` — extra per-stage timing breakdown (event drain,
///   scene build, GPU submit). Heavier to render; gated behind
///   `Action::ToggleFpsDebug`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FpsDisplayMode {
    Off,
    Snapshot,
    Debug,
}

impl Default for RenderDecree {
    fn default() -> Self {
        RenderDecree::Noop
    }
}

/// Window startup mode picked by `Options` from CLI / env. The
/// event loop forwards the choice to winit at window creation.
///
/// - `Fullscreen` — exclusive-fullscreen on the primary monitor.
/// - `WindowedFullscreen` — borderless window sized to the
///   monitor; alt-tab still works.
/// - `Windowed { x, y }` — windowed mode with explicit pixel
///   dimensions.
#[derive(Copy, Clone)]
pub enum WindowMode {
    Fullscreen,
    WindowedFullscreen,
    Windowed { x: u32, y: u32 },
}

/// Wall-clock stopwatch, started at construction. Single-use:
/// `new_start` then one `stop` call returning the elapsed
/// `Duration`. Used by the freeze watchdog and the per-frame
/// drain to report degraded-frame durations to logs.
#[derive(Copy, Clone)]
pub struct StopWatch {
    start: Instant,
}

impl StopWatch {
    pub fn new_start() -> StopWatch {
        StopWatch {
            start: Instant::now(),
        }
    }

    pub fn stop(&self) -> Duration {
        Instant::now().duration_since(self.start)
    }
}

/// Re-armable countdown timer. `is_expired()` returns `true`
/// once `duration` has elapsed since the last `new` /
/// `expire_in` call. Used by the event loop to schedule periodic
/// background work (e.g. animation tick, scene-cache GC) without
/// pulling in a real scheduler.
#[derive(Copy, Clone)]
pub struct PollTimer {
    instant: Instant,
    duration: Duration,
}

impl PollTimer {
    #[inline]
    pub fn new(duration: Duration) -> PollTimer {
        PollTimer {
            instant: Instant::now(),
            duration,
        }
    }

    #[inline]
    pub fn immediately() -> PollTimer {
        Self::new(Duration::from_millis(0))
    }

    pub fn is_expired(&self) -> bool {
        Instant::now().duration_since(self.instant).ge(&self.duration)
    }
    pub fn expire_in(&mut self, duration: Duration) {
        self.instant = Instant::now();
        self.duration = duration;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_stopwatch_measures_elapsed() {
        let watch = StopWatch::new_start();
        thread::sleep(Duration::from_millis(10));
        let elapsed = watch.stop();
        assert!(
            elapsed >= Duration::from_millis(5),
            "StopWatch should measure at least 5ms after sleeping 10ms; got {:?}",
            elapsed,
        );
    }

    #[test]
    fn test_poll_timer_immediately_is_expired() {
        let timer = PollTimer::immediately();
        assert!(
            timer.is_expired(),
            "PollTimer::immediately() should be expired right away"
        );
    }

    #[test]
    fn test_poll_timer_far_future_not_expired() {
        let timer = PollTimer::new(Duration::from_secs(60));
        assert!(
            !timer.is_expired(),
            "PollTimer with 60s duration should not expire instantly"
        );
    }

    #[test]
    fn test_poll_timer_expire_in_resets() {
        let mut timer = PollTimer::immediately();
        assert!(timer.is_expired());
        timer.expire_in(Duration::from_secs(60));
        assert!(
            !timer.is_expired(),
            "expire_in should reset the timer with a new duration"
        );
    }

    #[test]
    fn test_render_decree_default_is_noop() {
        let decree: RenderDecree = RenderDecree::default();
        assert_eq!(decree, RenderDecree::Noop);
    }
}
