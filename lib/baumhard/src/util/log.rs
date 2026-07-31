// SPDX-License-Identifier: MPL-2.0

//! Logger initialization — single entry point for both targets.
//!
//! The `log` crate's macros (`log::info!` / `warn!` / ...) are
//! the universal Rust idiom: every alternative (`tracing`,
//! `defmt`, structured collectors) implements `log::Log` or
//! provides the same names. Wrapping the macros themselves
//! gains nothing portable, so callsites continue to use
//! `log::warn!(...)` etc. directly.
//!
//! What WAS scattered across `src/main.rs` was the per-target
//! init — `env_logger::init()` on native, `console_log::init_with_level`
//! on WASM, plus the panic hook wiring on WASM. This module
//! collapses both onto one [`crate::util::log::init`] call.

/// Initialize the global logger for whichever target this binary
/// is built for. Native uses `env_logger` (reads `RUST_LOG`);
/// WASM uses `console_log` at `Info` level and installs the
/// `console_error_panic_hook` so a panic surfaces a JS-side
/// stack trace.
///
/// The native default filter is `warn`, not `env_logger`'s own
/// `error` default. CODE_CONVENTIONS §9 designates `warn!` as the
/// channel a degraded frame reports on, and the `log` crate is
/// built with `release_max_level_warn` so those calls survive into
/// release; defaulting the runtime filter to `error` would delete
/// them again one layer down. `RUST_LOG` still overrides, so
/// `RUST_LOG=debug` in a debug build behaves as before.
///
/// Idempotent in the sense that calling twice is a programming
/// error — both backends will panic on a second init. Should
/// fire once at program start.
pub fn init() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    }

    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(console_error_panic_hook::hook));
        console_log::init_with_level(log::Level::Info).expect("failed to init logger");
    }
}

/// The `log` feature both workspace manifests must declare. Changing
/// this constant without changing both manifests fails
/// [`tests::both_manifests_keep_warn_and_error_in_release`].
#[cfg(all(test, not(target_arch = "wasm32")))]
const REQUIRED_LOG_LEVEL_FEATURE: &str = "release_max_level_warn";

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::REQUIRED_LOG_LEVEL_FEATURE;
    use std::path::{Path, PathBuf};

    /// Absolute path to a manifest under the repo root, resolved from
    /// baumhard's own `CARGO_MANIFEST_DIR` the same way
    /// [`crate::util::doc_fixtures::format_doc_path`] does.
    fn repo_path(relative: &str) -> PathBuf {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join(relative)
    }

    /// Return the `log = { ... }` dependency line from a manifest.
    fn log_dependency_line(manifest: &str) -> String {
        let text = std::fs::read_to_string(repo_path(manifest))
            .unwrap_or_else(|e| panic!("{manifest} must be readable: {e}"));
        text.lines()
            .find(|line| line.trim_start().starts_with("log = "))
            .unwrap_or_else(|| panic!("{manifest} no longer declares a `log = ` dependency"))
            .to_string()
    }

    /// The whole failure mode this issue was about: a `log` max-level
    /// feature that quietly deletes CODE_CONVENTIONS §9's failure
    /// channel from the binaries users run. Cargo unions features
    /// across the workspace and the *most restrictive*
    /// `release_max_level_*` wins, so one manifest drifting back to
    /// `off` silences both crates while every test still passes.
    /// Pin both manifests against the policy the conventions state.
    #[test]
    fn both_manifests_keep_warn_and_error_in_release() {
        // Every level cap that would drop `warn!` below the §9
        // contract, plus the two looser ones — naming them all means
        // a swap to any other variant fails here rather than sliding
        // through on a substring match.
        const REJECTED: &[&str] = &[
            "release_max_level_off",
            "release_max_level_error",
            "release_max_level_info",
            "release_max_level_debug",
            "release_max_level_trace",
        ];

        for manifest in ["Cargo.toml", "lib/baumhard/Cargo.toml"] {
            let line = log_dependency_line(manifest);
            assert!(
                line.contains(REQUIRED_LOG_LEVEL_FEATURE),
                "{manifest} must declare `{REQUIRED_LOG_LEVEL_FEATURE}` on the `log` \
                 dependency so CODE_CONVENTIONS §9's warn!/error! channel survives \
                 release builds; found: {line}"
            );
            for rejected in REJECTED {
                assert!(
                    !line.contains(rejected),
                    "{manifest} declares `{rejected}`, which contradicts \
                     `{REQUIRED_LOG_LEVEL_FEATURE}`; found: {line}"
                );
            }
        }
    }

    /// The conventions document is the policy's only prose home. If
    /// the level boundary moves, this catches the doc going stale
    /// alongside the manifests.
    #[test]
    fn code_conventions_documents_the_release_log_level() {
        let text = std::fs::read_to_string(repo_path("CODE_CONVENTIONS.md"))
            .expect("CODE_CONVENTIONS.md must be readable");
        assert!(
            text.contains(REQUIRED_LOG_LEVEL_FEATURE),
            "CODE_CONVENTIONS.md §9 must name `{REQUIRED_LOG_LEVEL_FEATURE}` so the \
             manifests and the stated policy cannot drift apart"
        );
    }
}
