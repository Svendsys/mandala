// SPDX-License-Identifier: MPL-2.0

//! The root manifest's debug-info profiles (#283), held against the
//! file instead of trusted to a comment.
//!
//! `[profile.dev]` gives the workspace's own crates `line-tables-only`
//! and `[profile.dev.package."*"]` gives every dependency no debug
//! info, which roughly halves what one `./test.sh --lint` leaves in
//! `target/`. `[profile.dev-debug]` is the opt-in that restores all of
//! it. The facts that make that arrangement work are easy to lose in
//! an edit and invisible when lost, so each is a failure here:
//!
//! - **The lean settings stay.** Our crates losing the line tables a
//!   `RUST_BACKTRACE=1` backtrace reads its `file:line` from, or a
//!   dependency getting debug info back, changes nothing a test
//!   notices; only the disk and the backtraces do.
//! - **Nothing else turns debug info on.** Any table under
//!   `profile.dev*` or `profile.test` may set `debug` only if it is
//!   one of the five tables checked here. So a
//!   `[profile.dev.package.naga]` left behind from a debugging
//!   session, a `[profile.test] debug = true` and a
//!   `[profile.dev.build-override] debug = true` all fail, though
//!   cargo would honor each of them.
//! - **`dev-debug` repeats the package table.** A profile that
//!   `inherits` another inherits its `package."*"` table too, and a
//!   package table beats the profile's own `debug`, so without
//!   `[profile.dev-debug.package."*"]` the "full debug info" profile
//!   builds every dependency with none. Cargo does not warn.
//! - **`dev-debug` sets `build-override`.** A workspace member's build
//!   script (`lib/baumhard/build.rs`) matches no package table and
//!   takes cargo's default, no debug info, so
//!   `[profile.dev-debug.build-override]` is what gives it symbols.
//! - **No profile sets `split-debuginfo`.** No single value is stable
//!   on both Windows ABIs: MSVC accepts only `packed` and GNU only
//!   `off` (`-Csplit-debuginfo=... is unstable on this platform`),
//!   cargo profiles cannot be conditional on the target, and the
//!   `windows-ipc` CI job compiles the whole app there. The rule is
//!   held for every profile table in the manifest rather than for
//!   `dev` alone, because that is where the next person would reach
//!   for it.
//!
//! The reader is line-oriented, like [`crate::util::manifests`], and
//! fails closed: [`profile_table`] panics on a header that appears
//! twice, and [`check_profiles`] reports a missing required table by
//! name and reason rather than reading it as empty.
//!
//! Test-only and native-only for the same reasons `manifests` is: it
//! reads a file off the filesystem.

use crate::util::doc_fixtures::repo_path;
use std::collections::BTreeMap;

/// The five tables under `profile.dev*` / `profile.test` that may set
/// `debug`. [`check_profiles`] holds each to its value.
const DEBUG_TABLES: [&str; 5] = [
    "profile.dev",
    "profile.dev.package.\"*\"",
    "profile.dev-debug",
    "profile.dev-debug.package.\"*\"",
    "profile.dev-debug.build-override",
];

/// The name inside the brackets of a `[header]` line, ignoring a
/// trailing comment; `None` for any other line.
fn header_of(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix('[')?;
    rest.split_once(']').map(|(name, _)| name.trim())
}

/// The `key = value` lines directly under `[header]`, trailing
/// comments stripped; `None` when the manifest has no such table.
/// Panics when it has more than one.
fn profile_table(manifest: &str, header: &str) -> Option<BTreeMap<String, String>> {
    let mut seen = 0usize;
    let mut inside = false;
    let mut table = BTreeMap::new();
    for line in manifest.lines().map(str::trim) {
        if let Some(name) = header_of(line) {
            inside = name == header;
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
    assert!(seen <= 1, "the manifest must declare `[{header}]` at most once");
    (seen == 1).then_some(table)
}

/// Every `[profile...]` header in the manifest, brackets removed.
fn profile_headers(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .filter_map(header_of)
        .filter(|name| name.starts_with("profile"))
        .map(str::to_string)
        .collect()
}

/// Whether `key` is a way of setting debug info: `debug` itself, or a
/// dotted spelling of a nested table's (`package.naga.debug`).
fn sets_debug(key: &str) -> bool {
    key == "debug" || key.ends_with(".debug")
}

/// Check that `header` exists and sets `debug` to `value`; `why` is
/// the reason the table must be there.
fn require(manifest: &str, header: &str, value: &str, why: &str) -> Result<(), String> {
    match profile_table(manifest, header) {
        None => Err(format!("`[{header}]` is missing: {why}")),
        Some(table) if table.get("debug").is_some_and(|v| v == value) => Ok(()),
        Some(_) => Err(format!("`[{header}]` must set `debug = {value}`: {why}")),
    }
}

/// Why `manifest`'s profiles are not the arrangement #283 describes,
/// or `Ok` when they are.
fn check_profiles(manifest: &str) -> Result<(), String> {
    require(
        manifest,
        "profile.dev",
        "\"line-tables-only\"",
        "our crates keep the line tables backtraces read `file:line` from",
    )?;
    require(
        manifest,
        "profile.dev.package.\"*\"",
        "false",
        "dependencies carry no debug info",
    )?;
    require(
        manifest,
        "profile.dev-debug",
        "true",
        "the opt-in profile restores full debug info",
    )?;
    let full = profile_table(manifest, "profile.dev-debug").unwrap_or_default();
    if full.get("inherits").is_none_or(|v| v != "\"dev\"") {
        return Err("`[profile.dev-debug]` must inherit `dev`".into());
    }
    require(
        manifest,
        "profile.dev-debug.package.\"*\"",
        "true",
        "the `package.\"*\"` table inherited from `dev` would otherwise leave every dependency with no \
         debug info, and it beats the profile's own `debug = true`",
    )?;
    require(
        manifest,
        "profile.dev-debug.build-override",
        "true",
        "our own build script matches no package table and would otherwise have no debug info",
    )?;

    for header in profile_headers(manifest) {
        let table = profile_table(manifest, &header).unwrap_or_default();
        if table.contains_key("split-debuginfo") {
            return Err(format!(
                "`[{header}]` sets `split-debuginfo`: no single value is stable on both Windows ABIs, where \
                 the `windows-ipc` job builds the app, and a profile cannot be conditional on the target"
            ));
        }
        let debug_scoped = header.starts_with("profile.dev") || header.starts_with("profile.test");
        if !debug_scoped {
            continue;
        }
        let checked = DEBUG_TABLES.contains(&header.as_str());
        for key in table.keys() {
            let inline_table = key == "package" || key == "build-override";
            let stray_debug = sets_debug(key) && !(checked && key == "debug");
            if inline_table || stray_debug {
                return Err(format!(
                    "`[{header}]` sets `{key}`, which can turn debug info on outside the five checked \
                     tables: only those may set `debug`, so a table left behind from a debugging session \
                     cannot quietly bring `target/` back to full size"
                ));
            }
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

[profile.dev-debug.build-override]
debug = true

[profile.release-lto]
inherits = "release"
"#;

    fn broken(from: &str, to: &str) -> String {
        assert!(GOOD.contains(from), "the control's fixture lost `{from}`");
        GOOD.replacen(from, to, 1)
    }

    /// `GOOD` with `extra` added as a table of its own.
    fn planted(extra: &str) -> String {
        format!("{GOOD}\n{extra}\n")
    }

    #[test]
    fn test_build_profiles_fixture_passes_its_own_check() {
        assert_eq!(check_profiles(GOOD), Ok(()));
    }

    #[test]
    fn test_build_profiles_check_rejects_each_way_the_arrangement_breaks() {
        let cases = [
            // The lean settings.
            (
                broken("\"line-tables-only\"  # our crates", "true"),
                "must set `debug = \"line-tables-only\"`",
            ),
            (
                broken(
                    "[profile.dev.package.\"*\"]\ndebug = false",
                    "[profile.dev.package.\"*\"]\ndebug = true",
                ),
                "must set `debug = false`",
            ),
            // The opt-in profile.
            (
                broken(
                    "inherits = \"dev\"\ndebug = true",
                    "inherits = \"dev\"\ndebug = false",
                ),
                "`[profile.dev-debug]` must set `debug = true`",
            ),
            (
                broken("inherits = \"dev\"\ndebug = true", "debug = true"),
                "must inherit `dev`",
            ),
            (
                broken("[profile.dev-debug.package.\"*\"]\ndebug = true\n", ""),
                "`[profile.dev-debug.package.\"*\"]` is missing: the `package.\"*\"` table inherited",
            ),
            (
                broken(
                    "[profile.dev-debug.package.\"*\"]\ndebug = true",
                    "[profile.dev-debug.package.\"*\"]\ndebug = false",
                ),
                "`[profile.dev-debug.package.\"*\"]` must set `debug = true`",
            ),
            (
                broken("[profile.dev-debug.build-override]\ndebug = true\n", ""),
                "`[profile.dev-debug.build-override]` is missing: our own build script",
            ),
            // Debug info turned back on from outside the checked tables.
            (
                planted("[profile.dev.package.naga]\ndebug = true"),
                "`[profile.dev.package.naga]` sets `debug`",
            ),
            (
                planted("[profile.test]\ndebug = true"),
                "`[profile.test]` sets `debug`",
            ),
            (
                planted("[profile.dev.build-override]\ndebug = true"),
                "`[profile.dev.build-override]` sets `debug`",
            ),
            (
                planted("[profile.dev-debug.package.naga]\ndebug = 1"),
                "`[profile.dev-debug.package.naga]` sets `debug`",
            ),
            (
                broken(
                    "debug = \"line-tables-only\"  # our crates",
                    "debug = \"line-tables-only\"\npackage.naga.debug = true",
                ),
                "`[profile.dev]` sets `package.naga.debug`",
            ),
            (
                broken(
                    "debug = \"line-tables-only\"  # our crates",
                    "debug = \"line-tables-only\"\npackage = { naga = { debug = true } }",
                ),
                "`[profile.dev]` sets `package`",
            ),
            // `split-debuginfo`, anywhere.
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
    fn test_build_profiles_check_leaves_other_profiles_alone() {
        // `release` and `bench` may say what they like about debug.
        let manifest = planted("[profile.release]\ndebug = 1\n\n[profile.bench]\ndebug = true");
        assert_eq!(check_profiles(&manifest), Ok(()));
    }

    #[test]
    #[should_panic(expected = "at most once")]
    fn test_build_profiles_reader_refuses_a_duplicated_section() {
        let _ = check_profiles(&planted("[profile.dev]\ndebug = false"));
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
