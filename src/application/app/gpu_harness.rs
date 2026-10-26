// SPDX-License-Identifier: MPL-2.0

//! The GPU-backed test harness: a whole native app, built the way
//! `--headless` builds one, on whatever adapter this machine has.
//!
//! **What it is for.** Everything behind `&mut Renderer` — every
//! `dispatch_action` arm, the console's side-effect drain, every scene
//! rebuild, the IPC `act.*` commands end to end — needs a wgpu device
//! to run, and until #171 the suite had none (TEST_CONVENTIONS §T8).
//! The reason was environmental, not architectural: agent containers
//! and CI runners have no hardware GPU. They do have a software one on
//! demand — Mesa's lavapipe, a Vulkan ICD that rasterizes on the CPU —
//! and the real app runs on it (the `mandala-drive` skill's recipe
//! (b)). So a test can build the real app, with no window, exactly as
//! `run_headless` does, and assert on the state the real arms leave
//! behind: the document, the undo stack, the tree, what a rebuild left
//! in the renderer, the redraw ledger. Never pixels (§T10): lavapipe's
//! rasterization is not a hardware GPU's, and no snapshot could be
//! compared across them.
//!
//! **Skipped with a note, never silently.** [`headless_app`] probes
//! once per process for an adapter that can open a device
//! ([`probe`]); with none, every GPU-backed test prints a `SKIPPED:`
//! note and passes without having proved anything — the same
//! conditional-locally, unconditional-in-CI arrangement as `test.sh`'s
//! wasm32 gate and `lib/baumhard/tests/wasm_font_lock.rs`. Set
//! [`REQUIRE_ENV`] (CI does, after installing `mesa-vulkan-drivers`)
//! and a missing adapter fails the test instead, so a runner that lost
//! its driver goes red rather than quietly shrinking the suite
//! (TEST_CONVENTIONS §T13.4).
//!
//! **Any adapter, not only a software one.** The probe asks under
//! `AdapterPolicy::Auto`, the plan every bootstrap uses: the preferred
//! adapter first, a software one second. A developer's machine with a
//! GPU runs these tests on it; a GPU-less box runs them on lavapipe;
//! CI runs them on lavapipe by construction. The assertions are on
//! state, so which adapter rasterized is immaterial, and asking for a
//! software adapter only would skip the tests on exactly the machines
//! that have the most to run them on (macOS has no software Vulkan).
//!
//! **One app at a time.** Cargo runs tests on parallel threads, and
//! each app here opens a device, shapes every node's text under the
//! `FONT_SYSTEM` write lock and allocates a glyph atlas. Two of those
//! at once contend for the font lock and double the memory for no
//! coverage, so [`headless_app`] serializes them through one mutex; a
//! test that needs two apps builds the second after dropping the
//! first. The watchdog the app is built with is
//! [`FreezeWatchdog::unarmed`]: the app runs on a test thread among
//! dozens, and the process has no main loop to guard.
//!
//! **Not hermetic about the user's configuration, by the same choice
//! the app makes.** `run_native_init::build` loads the user's
//! mutations, macros and console history from their XDG directories
//! as the real app does. A test that depends on a keybind sets it
//! through [`Options::keybind_config`]; the rest is the app's own
//! startup, which is what the harness exists to run.

use std::sync::{Mutex, MutexGuard, OnceLock};

use pollster::block_on;

use super::freeze_watchdog::FreezeWatchdog;
use super::run_native_init::{self, InitState};
use super::shell::Shell;
use super::Options;
use crate::application::keybinds::KeybindConfig;
use crate::application::renderer::{probe_adapter, AdapterPolicy};

/// Set to make a missing adapter a failure rather than a skip.
pub(super) const REQUIRE_ENV: &str = "MANDALA_REQUIRE_GPU";

/// The frame size the harness's app draws for: the headless default a
/// `--headless` launch takes with no `--size`.
pub(super) const HARNESS_SIZE: (u32, u32) = run_native_init::HEADLESS_SURFACE_SIZE;

/// The one adapter probe per process: the name of the adapter that
/// opened a device, or every attempt's failure. Cached because the
/// answer cannot change while the process runs, and a probe opens a
/// device of its own.
fn probe() -> &'static Result<String, String> {
    static PROBE: OnceLock<Result<String, String>> = OnceLock::new();
    PROBE.get_or_init(|| block_on(probe_adapter(AdapterPolicy::Auto)))
}

/// A missing adapter: skip with a note, or fail when CI demands that
/// the test really run.
fn unavailable(why: &str) {
    if std::env::var_os(REQUIRE_ENV).is_some() {
        panic!(
            "no GPU adapter could open a device ({why}), and {REQUIRE_ENV} is set, so this test may not skip"
        );
    }
    eprintln!(
        "SKIPPED: no GPU adapter could open a device ({why}). This GPU-backed test did not run, \
         so a green result here says nothing about the code behind `&mut Renderer`. Install \
         `mesa-vulkan-drivers` for Mesa's lavapipe (CLAUDE.md \"Run tests\"), or set \
         {REQUIRE_ENV} to make this a failure."
    );
}

/// The real app, headless, on this machine's adapter. Holds the
/// one-at-a-time guard for as long as it lives, so a test that needs a
/// second app drops the first before building it.
pub(super) struct GpuApp {
    pub(super) init: InitState,
    _one_at_a_time: MutexGuard<'static, ()>,
}

/// Build the app on `map`, as `mandala --headless <map>` would build
/// it, with the default keybinds: `None`, after a `SKIPPED:` note,
/// when no adapter can open a device and [`REQUIRE_ENV`] is unset. The
/// caller returns on `None`; the test then passes having run nothing,
/// which is what the note says.
pub(super) fn headless_app(map: &str) -> Option<GpuApp> {
    headless_app_with(map, KeybindConfig::default())
}

/// [`headless_app`] with the keybinds a test wants resolved.
pub(super) fn headless_app_with(map: &str, keybind_config: KeybindConfig) -> Option<GpuApp> {
    let adapter = match probe() {
        Ok(name) => name,
        Err(why) => {
            unavailable(why);
            return None;
        }
    };
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    // A poisoned lock means an earlier GPU-backed test panicked while
    // holding its app, which is that test's failure, not this one's.
    let guard = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    eprintln!("gpu_harness: building the app on adapter {adapter:?}");
    let options = Options {
        mindmap_path: map.to_string(),
        keybind_config,
        ipc_socket: None,
        headless: true,
        surface_size: Some(HARNESS_SIZE),
        adapter: AdapterPolicy::Auto,
        one_shot: None,
    };
    let init = run_native_init::build(&options, Shell::Headless, FreezeWatchdog::unarmed());
    Some(GpuApp {
        init,
        _one_at_a_time: guard,
    })
}
