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
/// WASM's `console_log` filter stays at `Info` — looser than
/// native's `warn`, and harmless: the release cap compiles
/// `info!` out before the runtime filter ever sees it, so both
/// targets ship the same warn/error channel (§4).
///
/// Idempotent in the sense that calling twice is a programming
/// error — both backends will panic on a second init. Should
/// fire once at program start.
pub fn init() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut builder = env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or(NATIVE_DEFAULT_FILTER),
        );
        // `RUST_LOG=` — set but empty — is a shell accident, not a
        // request for silence. `default_filter_or` applies only when
        // the variable is *absent*, so an empty value would slide past
        // the `warn` default onto env_logger's own `error` and delete
        // the §9 channel again one layer down. Force the default back.
        if !rust_log_is_effective(std::env::var("RUST_LOG").ok().as_deref()) {
            builder.parse_filters(NATIVE_DEFAULT_FILTER);
        }
        builder.init();
    }

    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(console_error_panic_hook::hook));
        console_log::init_with_level(log::Level::Info).expect("failed to init logger");
    }
}

/// The native runtime filter applied when `RUST_LOG` carries nothing
/// usable. Matches the `release_max_level_warn` compile-time cap, so
/// the runtime filter never becomes the narrower of the two.
#[cfg(not(target_arch = "wasm32"))]
const NATIVE_DEFAULT_FILTER: &str = "warn";

/// Whether `RUST_LOG`'s raw value carries directives worth honoring.
/// `None` (unset) and `Some("")` / `Some("   ")` both mean "no", which
/// is the point: env_logger distinguishes them and we must not.
#[cfg(not(target_arch = "wasm32"))]
fn rust_log_is_effective(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.trim().is_empty())
}

/// The `log` feature both workspace manifests must declare. Changing
/// this constant without changing both manifests fails
/// [`tests::test_both_manifests_keep_warn_and_error_in_release`].
#[cfg(all(test, not(target_arch = "wasm32")))]
const REQUIRED_LOG_LEVEL_FEATURE: &str = "release_max_level_warn";

/// The CODE_CONVENTIONS heading whose section owns the release
/// log-level policy. Named once so the doc test pins the *section*
/// rather than scanning the whole file.
#[cfg(all(test, not(target_arch = "wasm32")))]
const POLICY_HEADING: &str = "## §9 Error handling";

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{POLICY_HEADING, REQUIRED_LOG_LEVEL_FEATURE, rust_log_is_effective};
    use crate::util::doc_fixtures::section_text;
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
    /// channel from the binaries users run.
    ///
    /// The two manifests cannot silently *disagree* — cargo unions
    /// `log`'s features across the workspace and `log` itself
    /// `compile_error!`s on "multiple release_max_level_* features
    /// set", so a mixed workspace fails to build and nothing gets as
    /// far as running. What has no compiler behind it is the two
    /// manifests silently *agreeing on the wrong level*: a joint drift
    /// back to `off`, a swap to `release_max_level_error`, or the
    /// feature dropped from both (which uncaps release to `Trace` and
    /// re-admits the chatty walker instrumentation). Those are what
    /// this pins.
    #[test]
    fn test_both_manifests_keep_warn_and_error_in_release() {
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

    /// The conventions document is the policy's only prose home, and
    /// §9 is the section that owns it. Scoped to that section on
    /// purpose: a whole-file `contains()` stays green when the
    /// paragraph is moved into another section or §9 is renamed out
    /// from under it, which is precisely the silent drift
    /// [`crate::util::doc_fixtures`] exists to refuse.
    ///
    /// Asserts the *boundary*, not just the feature name — §9 could
    /// otherwise state the opposite of what ships and still pass.
    #[test]
    fn test_code_conventions_section_9_documents_the_release_log_boundary() {
        let path = repo_path("CODE_CONVENTIONS.md");
        let section = section_text(&path, POLICY_HEADING);

        assert!(
            section.contains(REQUIRED_LOG_LEVEL_FEATURE),
            "CODE_CONVENTIONS.md {POLICY_HEADING} must name \
             `{REQUIRED_LOG_LEVEL_FEATURE}` so the manifests and the stated \
             policy cannot drift apart"
        );

        // The levels that survive release must be named as surviving,
        // and the ones that do not must be named as not surviving.
        // Checking both halves is what makes this a boundary pin:
        // `release_max_level_warn` alone does not say where the line
        // falls to a reader deciding which macro to reach for.
        for surviving in ["`warn!`", "`error!`"] {
            assert!(
                section.contains(surviving),
                "CODE_CONVENTIONS.md {POLICY_HEADING} must name {surviving} as a \
                 level that survives release builds"
            );
        }
        for compiled_out in ["`info!`", "`debug!`", "`trace!`"] {
            assert!(
                section.contains(compiled_out),
                "CODE_CONVENTIONS.md {POLICY_HEADING} must name {compiled_out} as a \
                 level that is compiled out of release builds"
            );
        }

        // Direction, not just vocabulary. Compared with runs of
        // whitespace collapsed so a re-wrap of the paragraph does not
        // fail the test, but an inverted claim does.
        let flat = section.split_whitespace().collect::<Vec<_>>().join(" ");
        let boundary =
            "`warn!` and `error!` survive into release; `info!`, `debug!` and `trace!` do not.";
        assert!(
            flat.contains(boundary),
            "CODE_CONVENTIONS.md {POLICY_HEADING} must state the boundary in the \
             direction the manifests actually implement it — expected the sentence \
             {boundary:?}. A §9 that named the same five macros while claiming the \
             opposite would otherwise pass."
        );
    }

    /// `RUST_LOG=` set-but-empty must behave as unset. env_logger's
    /// `default_filter_or` fires only on an *absent* variable, so
    /// without this distinction an empty value lands on env_logger's
    /// own `error` default and deletes §9's warn channel at runtime.
    #[test]
    fn test_empty_rust_log_is_treated_as_unset() {
        assert!(rust_log_is_effective(Some("debug")));
        assert!(rust_log_is_effective(Some("mandala=trace,warn")));
        assert!(!rust_log_is_effective(None), "unset must fall back to the default");
        assert!(!rust_log_is_effective(Some("")), "`RUST_LOG=` must fall back too");
        assert!(!rust_log_is_effective(Some("   ")), "whitespace-only is not a directive");
    }
}
