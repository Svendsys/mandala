// SPDX-License-Identifier: MPL-2.0

//! The root manifest's debug-info profiles (#283), held against the
//! file instead of trusted to a comment.
//!
//! `[profile.dev]` gives the workspace's own crates `line-tables-only`
//! and `[profile.dev.package."*"]` gives every dependency no debug
//! info, which is what keeps one `./test.sh --lint` near 3 GB instead
//! of 6.2 GB. `[profile.dev-debug]` is the opt-in that restores all of
//! it. Three of the facts that make that arrangement work are easy to
//! lose in an edit and invisible when lost, so each is a failure here:
//!
//! - **The two lean settings stay.** A dependency that quietly gets
//!   debug info back, or our crates losing the line tables a
//!   `RUST_BACKTRACE=1` backtrace reads its `file:line` from, changes
//!   nothing a test notices; only the disk and the backtraces do.
//! - **`dev-debug` repeats the package table.** A profile that
//!   `inherits` another inherits its `package."*"` table too, and a
//!   package table beats the profile's own `debug`, so without
//!   `[profile.dev-debug.package."*"]` the "full debug info" profile
//!   builds every dependency with none. Cargo does not warn.
//! - **No profile sets `split-debuginfo`.** `"unpacked"` is a hard
//!   error on Windows, MSVC and GNU (`-Csplit-debuginfo=unpacked is
//!   unstable on this platform`), cargo profiles cannot be conditional
//!   on the target, and the `windows-ipc` CI job compiles the whole
//!   app there. The rule is held for every profile table in the
//!   manifest rather than for `dev` alone, because that is where the
//!   next person would reach for it.
//!
//! The reader is line-oriented, like [`crate::util::manifests`], and
//! has the same stance: [`profile_table`] panics on a header it cannot
//! match exactly once rather than reading an empty table, so a
//! renamed or deleted section fails the test that names it.
//!
//! Test-only and native-only for the same reasons `manifests` is: it
//! reads a file off the filesystem.

use crate::util::doc_fixtures::repo_path;
use std::collections::BTreeMap;

/// The `key = value` lines directly under `[header]`, trailing
/// comments stripped. Panics unless the header appears exactly once.
fn profile_table(manifest: &str, header: &str) -> BTreeMap<String, String> {
    let wanted = format!("[{header}]");
    let mut seen = 0usize;
    let mut inside = false;
    let mut table = BTreeMap::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == wanted;
            seen += usize::from(inside);
            continue;
        }
        if !inside || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = value.split('#').next().unwrap_or("").trim();
            table.insert(key.trim().to_string(), value.to_string());
        }
    }
    assert_eq!(seen, 1, "the manifest must declare `{wanted}` exactly once");
    table
}

/// Every `[profile...]` header in the manifest, brackets removed.
fn profile_headers(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("[profile"))
        .map(|l| l.trim_matches(|c| c == '[' || c == ']').to_string())
        .collect()
}

/// Whether `table` holds `key` at exactly `value`.
fn has(table: &BTreeMap<String, String>, key: &str, value: &str) -> bool {
    table.get(key).is_some_and(|v| v == value)
}

/// Why `manifest`'s profiles are not the arrangement #283 describes,
/// or `Ok` when they are.
fn check_profiles(manifest: &str) -> Result<(), String> {
    let dev = profile_table(manifest, "profile.dev");
    if !has(&dev, "debug", "\"line-tables-only\"") {
        return Err("`[profile.dev]` must set `debug = \"line-tables-only\"`".into());
    }
    let dev_deps = profile_table(manifest, "profile.dev.package.\"*\"");
    if !has(&dev_deps, "debug", "false") {
        return Err("`[profile.dev.package.\"*\"]` must set `debug = false`".into());
    }
    let full = profile_table(manifest, "profile.dev-debug");
    if !has(&full, "inherits", "\"dev\"") || !has(&full, "debug", "true") {
        return Err("`[profile.dev-debug]` must inherit `dev` with `debug = true`".into());
    }
    let full_deps = profile_table(manifest, "profile.dev-debug.package.\"*\"");
    if !has(&full_deps, "debug", "true") {
        return Err(
            "`[profile.dev-debug.package.\"*\"]` must set `debug = true`: the table inherited from `dev` \
             would otherwise leave every dependency with no debug info"
                .into(),
        );
    }
    for header in profile_headers(manifest) {
        if profile_table(manifest, &header).contains_key("split-debuginfo") {
            return Err(format!(
                "`[{header}]` sets `split-debuginfo`: `unpacked` is a compile error on Windows, where the \
                 `windows-ipc` job builds the app, and a profile cannot be conditional on the target"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arrangement as it should read, for the controls below to
    /// break one line at a time.
    const GOOD: &str = r#"
[profile.dev]
debug = "line-tables-only"  # our crates

[profile.dev.package."*"]
debug = false

[profile.dev-debug]
inherits = "dev"
debug = true

[profile.dev-debug.package."*"]
debug = true

[profile.release-lto]
inherits = "release"
"#;

    fn broken(from: &str, to: &str) -> String {
        assert!(GOOD.contains(from), "the control's fixture lost `{from}`");
        GOOD.replacen(from, to, 1)
    }

    #[test]
    fn test_build_profiles_fixture_passes_its_own_check() {
        assert_eq!(check_profiles(GOOD), Ok(()));
    }

    #[test]
    fn test_build_profiles_check_rejects_each_way_the_arrangement_breaks() {
        let cases = [
            (
                broken("\"line-tables-only\"  # our crates", "true"),
                "line-tables-only",
            ),
            (
                broken(
                    "[profile.dev.package.\"*\"]\ndebug = false",
                    "[profile.dev.package.\"*\"]\ndebug = true",
                ),
                "debug = false",
            ),
            (
                broken(
                    "inherits = \"dev\"\ndebug = true",
                    "inherits = \"dev\"\ndebug = false",
                ),
                "inherit `dev`",
            ),
            (
                broken(
                    "[profile.dev-debug.package.\"*\"]\ndebug = true",
                    "[profile.dev-debug.package.\"*\"]\ndebug = false",
                ),
                "no debug info",
            ),
            (
                broken(
                    "[profile.release-lto]",
                    "[profile.release-lto]\nsplit-debuginfo = \"unpacked\"",
                ),
                "windows-ipc",
            ),
        ];
        for (manifest, expect) in cases {
            let err = check_profiles(&manifest).expect_err("a broken arrangement must be rejected");
            assert!(err.contains(expect), "wrong reason for {expect:?}: {err}");
        }
    }

    #[test]
    #[should_panic(expected = "exactly once")]
    fn test_build_profiles_reader_refuses_a_missing_section() {
        let _ = check_profiles(&GOOD.replace(
            "[profile.dev-debug.package.\"*\"]",
            "[profile.dev-debug.package.x]",
        ));
    }

    #[test]
    fn test_the_workspace_profiles_keep_debug_info_lean() {
        let manifest =
            std::fs::read_to_string(repo_path("Cargo.toml")).expect("the root Cargo.toml must be readable");
        if let Err(why) = check_profiles(&manifest) {
            panic!("{why} (#283)");
        }
    }
}
