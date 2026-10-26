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
//! wasm32 gate and `lib/baumhard/tests/wasm_font_lock.rs`. The note
//! goes through `baumhard::util::test_skip`, past libtest's per-test
//! capture — a passing test's `eprintln!` is shown only under
//! `--nocapture`, which `./test.sh` does not pass — so it reaches the
//! terminal, and `test.sh` counts the `SKIPPED:` lines into its
//! summary. Set [`REQUIRE_ENV`] (CI does, after installing
//! `mesa-vulkan-drivers`) and a missing adapter fails the test
//! instead, so a runner that lost its driver goes red rather than
//! quietly shrinking the suite (TEST_CONVENTIONS §T13.4).
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
//! as the real app does. All three loads are reads — nothing a test
//! does through the app writes them back, and a `save` goes to the
//! path the test names — so the user's files are safe; what is not
//! hermetic is the app's view of them: a user macro bound to a key a
//! test presses would run. A test that depends on a keybind sets it
//! through [`Options::keybind_config`]; the rest is the app's own
//! startup, which is what the harness exists to run. A config-root
//! override for the harness is the follow-up if a test ever needs an
//! empty macro table, and is not in this PR.

use std::sync::{Mutex, MutexGuard, OnceLock};

use baumhard::util::test_skip;
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
    // Past libtest's capture, so the note shows under `./test.sh`.
    test_skip::note_skipped(&format!(
        "no GPU adapter could open a device ({why}). This GPU-backed test did not run, \
         so a green result here says nothing about the code behind `&mut Renderer`. Install \
         `mesa-vulkan-drivers` for Mesa's lavapipe (CLAUDE.md \"Run tests\"), or set \
         {REQUIRE_ENV} to make this a failure."
    ));
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use baumhard::util::doc_fixtures::repo_path;
    use baumhard::util::rust_source::{blank_string_literals, strip_comments};

    /// The spellings that open a wgpu instance or device, or build the
    /// whole app on one. A path route counts when it does not continue
    /// an identifier, so `GlyphTreeEventInstance::new(` is not
    /// `Instance::new(`; a method route (leading `.`) counts wherever
    /// it appears, since its receiver always precedes it.
    const DEVICE_ROUTES: &[&str] = &[
        "Instance::new(",
        ".request_adapter(",
        ".request_device(",
        "bootstrap_native(",
        "bootstrap_wasm(",
        "bootstrap_headless(",
        "run_native_init::build(",
    ];

    /// The files that may spell them: the production bootstrap path,
    /// and this harness. Relative to the repository root.
    const MAY_OPEN_A_DEVICE: &[&str] = &[
        "src/application/renderer/adapter.rs",
        "src/application/renderer/mod.rs",
        "src/application/app/run_native_init.rs",
        "src/application/app/run_native.rs",
        "src/application/app/run_wasm/mod.rs",
        "src/application/app/run_headless.rs",
        "src/application/app/gpu_harness.rs",
    ];

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    /// The routes `code` (comments and string literals blanked) spells,
    /// each with the line it is on.
    fn routes_in(code: &str) -> Vec<(usize, &'static str)> {
        let mut found = Vec::new();
        for (i, line) in code.lines().enumerate() {
            for route in DEVICE_ROUTES {
                let mut from = 0;
                while let Some(at) = line[from..].find(route) {
                    let at = from + at;
                    // A method route (`.request_device(`) always follows its
                    // receiver; a path route must not continue an identifier.
                    let preceded_by_ident = !route.starts_with('.')
                        && line[..at]
                            .chars()
                            .next_back()
                            .is_some_and(|c| c.is_alphanumeric() || c == '_');
                    if !preceded_by_ident {
                        found.push((i + 1, *route));
                    }
                    from = at + route.len();
                }
            }
        }
        found
    }

    /// §T8's requirement, held against the sources: a GPU-backed test
    /// gets its app from [`headless_app`](super::headless_app), and no
    /// test — nor anything else — under `src/`, `lib/` or `crates/`
    /// opens a wgpu instance or device, or builds the app on one, by
    /// any other route. A test that did would have no skip note, no
    /// `MANDALA_REQUIRE_GPU`, and no place in the one-at-a-time line.
    /// Comments and string literals are blanked first, so a doc
    /// comment naming `request_adapter` or a test fixture quoting it
    /// (`rust_source_tests.rs` does) is not a call.
    ///
    /// The precondition is that the scan is looking: the production
    /// bootstrap and this harness are found spelling the routes they
    /// own, so an empty offender list cannot come from a scan that
    /// matched nothing.
    #[test]
    fn test_only_the_bootstrap_path_and_the_harness_open_a_device() {
        let repo = repo_path("");
        let mut files = Vec::new();
        for root in ["src", "lib", "crates"] {
            rust_files(&repo_path(root), &mut files);
        }
        assert!(
            files.len() > 100,
            "precondition: the walk found the workspace ({} files)",
            files.len()
        );

        let mut seen_in_allowed = Vec::new();
        let mut offenders = Vec::new();
        for path in &files {
            let relative = path
                .strip_prefix(&repo)
                .expect("under the repo")
                .to_string_lossy()
                .replace('\\', "/");
            let source = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{relative}: {e}"));
            let code = blank_string_literals(&strip_comments(&source));
            for (line, route) in routes_in(&code) {
                if MAY_OPEN_A_DEVICE.contains(&relative.as_str()) {
                    seen_in_allowed.push((relative.clone(), route));
                } else {
                    offenders.push(format!("{relative}:{line}: {route}"));
                }
            }
        }

        for (file, route) in [
            ("src/application/renderer/adapter.rs", ".request_device("),
            ("src/application/renderer/mod.rs", "Instance::new("),
            ("src/application/app/run_native_init.rs", "bootstrap_headless("),
            ("src/application/app/gpu_harness.rs", "run_native_init::build("),
        ] {
            assert!(
                seen_in_allowed.contains(&(file.to_string(), route)),
                "precondition: the scan sees {file} spell `{route}`; seen: {seen_in_allowed:?}"
            );
        }
        assert!(
            offenders.is_empty(),
            "a wgpu instance, device or whole app is opened outside the bootstrap path and \
             `gpu_harness` — a test gets its app from `gpu_harness::headless_app` (TEST_CONVENTIONS \
             §T8):\n{}",
            offenders.join("\n")
        );
    }

    /// The route matcher's own control: an identifier that merely
    /// ends in a route's name is not that route, and a route inside a
    /// comment or a string is blanked before the matcher sees it.
    #[test]
    fn test_route_matcher_ignores_longer_identifiers_comments_and_strings() {
        let code = "let e = GlyphTreeEventInstance::new(1);\n\
                    // Instance::new( in a comment\n\
                    let s = \"adapter.request_device(x)\";\n\
                    let i = Instance::new(d);\n\
                    let d = adapter.request_device(&desc);\n";
        let blanked = blank_string_literals(&strip_comments(code));
        assert_eq!(
            routes_in(&blanked),
            vec![(4, "Instance::new("), (5, ".request_device(")],
            "the path route after `let i =`, the method route after its receiver, nothing else"
        );
        assert_eq!(
            routes_in(code).len(),
            4,
            "control: without blanking, the comment and the string would count too"
        );
    }
}
