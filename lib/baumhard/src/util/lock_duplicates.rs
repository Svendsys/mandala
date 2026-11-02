// SPDX-License-Identifier: MPL-2.0

//! Every crate `Cargo.lock` holds at more than one semver-incompatible
//! version, each with the reason it stays (#284), held against the
//! lockfile instead of trusted to a pull request's description.
//!
//! A second version of a crate is a second build of it — of it and of
//! nothing else, since the two are different packages — so a duplicate
//! costs disk and compile time in every worktree that builds the graph.
//! #284 removed the ones a compatible release, a lockfile pin or a
//! dropped dependency could remove; [`DUPLICATES`] is what is left, and
//! why. Two ways that table can stop being true, both failures here:
//!
//! - **A duplicate comes back.** `cargo update` resolves each
//!   requirement to the newest release it allows, and two of the
//!   removals are lockfile pins against exactly that: swash accepts
//!   `skrifa >=0.31.1, <=0.44` and cosmic-text `^0.40`, so a bare
//!   `cargo update` puts swash back on 0.44 and the font stack back on
//!   two copies of skrifa, read-fonts and font-types; `flate2 1.1.10`
//!   moves to `miniz_oxide 0.9` while `png 0.18.1` asks for 0.8. Each
//!   is a crate on the lockfile at two versions that this table does
//!   not list.
//!
//!   The `flate2` pin is a downgrade, and it gives something up: 1.1.10
//!   also fixed an infinite loop writing gzip headers and footers
//!   (flate2 #547), rejects an incomplete deflate stream at EOF instead
//!   of reading it as a clean end (flate2 #556), and rejects oversized
//!   gzip extra fields (flate2 #553). None is on a path this workspace
//!   runs today — png's `ZlibEncoder` only writes, and the one
//!   `ZlibDecoder` (tiff, under arboard on macOS) decodes clipboard
//!   images, which `src/` never reads — so one duplicate build less is
//!   the better trade for now.
//!   **If a flate2 advisory lands, or a path starts decoding, drop the
//!   pin and list `miniz_oxide` 0.8/0.9 here with that reason; do not
//!   re-pin.** The skrifa pin gives swash the skrifa cosmic-text
//!   already uses instead of the newest its range allows; the demo
//!   maps render to the same pixels on either (#284).
//! - **A duplicate goes away.** A bump that removes one leaves a row
//!   here stating a reason for something that no longer happens; the
//!   row goes with it, so the table stays the list of what is.
//!
//! A version is compared by its semver-compatible line — `0.12` for
//! `0.12.5`, `1` for `1.0.69` — because that is what makes two
//! versions two packages: a patch release of either copy is not a new
//! duplicate and does not fail this.
//!
//! The lockfile, not `cargo tree`: it is a file, read in microseconds,
//! and it holds every target's graph at once — the macOS, Windows,
//! Android and Redox halves no Linux `cargo tree` prints without
//! `--target all`. It also holds a package no build compiles, which
//! [`DUPLICATES`] names as such (`rand 0.8`).
//!
//! Test-only and native-only for the same reasons
//! [`crate::util::manifests`] is: it reads a file off the filesystem.

use crate::util::doc_fixtures::repo_path;
use std::collections::{BTreeMap, BTreeSet};

/// A crate locked at more than one semver-compatible line: its name,
/// the lines, and why the older one stays.
type Duplicate = (&'static str, &'static [&'static str], &'static str);

/// Every crate `Cargo.lock` holds at more than one line, and why.
///
/// "Newest" below means the newest release on crates.io when #284 was
/// written, against which a compatible bump had nothing to offer. Most
/// rows lead to `winit 0.30`: its newest stable release, 0.30.13, still
/// pins the older line of each, and 0.31 is in beta.
const DUPLICATES: &[Duplicate] = &[
    (
        "bitflags",
        &["1", "2"],
        "1 is core-graphics 0.23's and redox_syscall 0.4's, both winit 0.30's, on macOS and \
         Redox only.",
    ),
    (
        "block2",
        &["0.5", "0.6"],
        "Apple only: 0.5 is winit 0.30's objc2 0.5 family, 0.6 wgpu-hal 30's and arboard \
         3.7's objc2 0.6 family.",
    ),
    (
        "getrandom",
        &["0.3", "0.4"],
        "0.3 is ahash 0.8.12's (the newest), which winit 0.30 uses on Linux; rand 0.10 and \
         mandalactl are on 0.4.",
    ),
    (
        "hashbrown",
        &["0.16", "0.17"],
        "0.16 is gpu-allocator 0.28.0's (the newest), through wgpu-hal 30; naga, wgpu and \
         indexmap are on 0.17.",
    ),
    (
        "jni-sys",
        &["0.3", "0.4"],
        "Android only: ndk 0.9 asks for 0.3 and jni 0.22 for 0.4, both under winit 0.30.",
    ),
    (
        "linux-raw-sys",
        &["0.4", "0.12"],
        "0.4 is rustix 0.38's; see `rustix`.",
    ),
    (
        "lock_api",
        &["0.4", "0.5"],
        "0.4 is parking_lot 0.12's, 0.5 parking_lot 0.13's; see `parking_lot`.",
    ),
    (
        "objc2",
        &["0.5", "0.6"],
        "Apple only: 0.5 is winit 0.30's, 0.6 wgpu-hal 30's and arboard 3.7's.",
    ),
    (
        "objc2-app-kit",
        &["0.2", "0.3"],
        "Apple only: the objc2 0.5 and 0.6 families; see `objc2`.",
    ),
    (
        "objc2-foundation",
        &["0.2", "0.3"],
        "Apple only: the objc2 0.5 and 0.6 families; see `objc2`.",
    ),
    (
        "objc2-metal",
        &["0.2", "0.3"],
        "Apple only: the objc2 0.5 and 0.6 families; see `objc2`.",
    ),
    (
        "objc2-quartz-core",
        &["0.2", "0.3"],
        "Apple only: the objc2 0.5 and 0.6 families; see `objc2`.",
    ),
    (
        "parking_lot",
        &["0.12", "0.13"],
        "baumhard needs 0.13's `RecursiveRwLock` for `FONT_SYSTEM`; wgpu 30.0.1 and \
         wgpu-core 30.0.1 (the newest) and arboard 3.7.0 (the newest) ask for ^0.12.",
    ),
    (
        "parking_lot_core",
        &["0.9", "0.10"],
        "0.9 is parking_lot 0.12's, 0.10 parking_lot 0.13's; see `parking_lot`.",
    ),
    (
        "r-efi",
        &["5", "6"],
        "UEFI only: 5 is getrandom 0.3's, 6 getrandom 0.4's; see `getrandom`.",
    ),
    (
        "rand",
        &["0.8", "0.10"],
        "0.8 is locked and never built: it is ordered-float 5.5's optional dependency, \
         which only its `randtest` feature turns on, and nothing here does. Cargo locks \
         an optional dependency whether or not a feature enables it.",
    ),
    (
        "rand_core",
        &["0.6", "0.10"],
        "0.6 is rand 0.8's, locked and never built; see `rand`.",
    ),
    (
        "redox_syscall",
        &["0.4", "0.5", "0.9"],
        "Redox only: 0.4 is winit 0.30's, 0.5 parking_lot_core 0.9's, 0.9 parking_lot_core \
         0.10's and orbclient's.",
    ),
    (
        "rustc-hash",
        &["1", "2"],
        "1 is naga 30.0.1's and wgpu-core 30.0.1's (the newest), which ask for ^1.1; \
         cosmic-text, glyphon and this workspace are on 2.",
    ),
    (
        "rustix",
        &["0.38", "1"],
        "0.38 is winit 0.30's, calloop 0.13's and smithay-client-toolkit 0.19's, all under \
         winit 0.30; the app's own `cfg(unix)` calls are on 1.",
    ),
    (
        "smol_str",
        &["0.2", "0.3"],
        "0.2 is winit 0.30's, 0.3 cosmic-text 0.19's.",
    ),
    (
        "syn",
        &["2", "3"],
        "2 is strum_macros 0.28.0's and zerocopy-derive 0.8.63's (each the newest), and \
         thiserror-impl 1's among others; serde_derive, bytemuck_derive and mandala_derive \
         are on 3.",
    ),
    (
        "thiserror",
        &["1", "2"],
        "1 is calloop 0.13's and smithay-client-toolkit 0.19's, under winit 0.30.",
    ),
    (
        "thiserror-impl",
        &["1", "2"],
        "thiserror 1's and 2's; see `thiserror`.",
    ),
    (
        "windows-sys",
        &["0.52", "0.59", "0.61"],
        "Windows only: 0.52 is winit 0.30's, 0.59 rustix 0.38's, and the rest of the graph \
         is on 0.61.",
    ),
];

/// The semver-compatible line a version belongs to: the major for
/// `1.0.0` and later, `0.minor` below it, and `0.0.patch` below that.
fn compatible_line(version: &str) -> String {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let parts: Vec<&str> = core.split('.').collect();
    match parts.as_slice() {
        ["0", "0", patch, ..] => format!("0.0.{patch}"),
        ["0", minor, ..] => format!("0.{minor}"),
        [major, ..] => (*major).to_string(),
        [] => version.to_string(),
    }
}

/// Every registry package of a lockfile's text with each
/// semver-compatible line it is locked at. A path package (a workspace
/// member, which has no `source`) is left out.
fn locked_lines(text: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for table in text.split("[[package]]").skip(1) {
        let field = |key: &str| {
            table.lines().find_map(|line| {
                line.strip_prefix(key)
                    .and_then(|rest| rest.strip_prefix(" = \""))
                    .and_then(|rest| rest.strip_suffix('"'))
            })
        };
        let name = field("name").expect("every locked package has a name");
        let version = field("version").expect("every locked package has a version");
        if field("source").is_some() {
            out.entry(name.to_string())
                .or_default()
                .insert(compatible_line(version));
        }
    }
    out
}

/// How `text` disagrees with `table`, each line naming a crate: one
/// locked at several lines the table does not list, or at lines other
/// than the table's, and one the table lists that is no longer locked
/// at more than one.
fn disagreements(text: &str, table: &[Duplicate]) -> Vec<String> {
    let found: BTreeMap<String, BTreeSet<String>> = locked_lines(text)
        .into_iter()
        .filter(|(_, lines)| lines.len() > 1)
        .collect();
    let listed: BTreeMap<String, BTreeSet<String>> = table
        .iter()
        .map(|(name, lines, _)| {
            (
                (*name).to_string(),
                lines.iter().map(|line| (*line).to_string()).collect(),
            )
        })
        .collect();
    let mut out = Vec::new();
    for (name, lines) in &found {
        match listed.get(name) {
            None => out.push(format!(
                "{name} is locked at {lines:?} and `DUPLICATES` gives no reason for it"
            )),
            Some(expected) if expected != lines => out.push(format!(
                "{name} is locked at {lines:?}; `DUPLICATES` lists {expected:?}"
            )),
            Some(_) => {}
        }
    }
    for name in listed.keys().filter(|name| !found.contains_key(*name)) {
        out.push(format!(
            "{name} is listed in `DUPLICATES` and is no longer locked at more than one line"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two lockfile tables, as `Cargo.lock` writes them.
    fn lockfile(packages: &[(&str, &str)]) -> String {
        let mut text = String::from("version = 4\n");
        for (name, version) in packages {
            text.push_str(&format!(
                "\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n\
                 source = \"registry+https://github.com/rust-lang/crates.io-index\"\n"
            ));
        }
        text
    }

    #[test]
    fn test_a_version_is_compared_by_its_compatible_line() {
        assert_eq!(compatible_line("0.12.5"), "0.12");
        assert_eq!(compatible_line("1.0.69"), "1");
        assert_eq!(compatible_line("2.13.2"), "2");
        assert_eq!(compatible_line("0.0.3"), "0.0.3");
        assert_eq!(compatible_line("0.11.0+11769913"), "0.11");
        assert_eq!(compatible_line("1.0.0-rc.1"), "1");
    }

    /// The reader against a synthetic lockfile, controls included: a
    /// duplicate the table does not list, one it lists at other lines,
    /// one it lists that is gone, and a path package (a workspace
    /// member, no `source`) that is not compared.
    #[test]
    fn test_the_reader_names_every_disagreement() {
        let table: &[Duplicate] = &[("syn", &["2", "3"], "a reason")];
        let agreeing = lockfile(&[("syn", "2.0.119"), ("syn", "3.0.9"), ("serde", "1.0.229")]);
        assert_eq!(disagreements(&agreeing, table), Vec::<String>::new());

        // A patch release of either copy is not a new duplicate.
        let patched = lockfile(&[("syn", "2.0.120"), ("syn", "3.0.9"), ("serde", "1.0.230")]);
        assert_eq!(disagreements(&patched, table), Vec::<String>::new());

        let unlisted = lockfile(&[
            ("syn", "2.0.119"),
            ("syn", "3.0.9"),
            ("skrifa", "0.40.0"),
            ("skrifa", "0.44.0"),
        ]);
        let found = disagreements(&unlisted, table);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("skrifa is locked at"), "{found:?}");

        let moved = lockfile(&[("syn", "2.0.119"), ("syn", "4.0.0")]);
        let found = disagreements(&moved, table);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("`DUPLICATES` lists"), "{found:?}");

        let gone = lockfile(&[("syn", "3.0.9")]);
        let found = disagreements(&gone, table);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("no longer locked"), "{found:?}");

        let member = format!(
            "{}\n[[package]]\nname = \"syn\"\nversion = \"9.0.0\"\n",
            lockfile(&[("syn", "2.0.119"), ("syn", "3.0.9")])
        );
        assert_eq!(disagreements(&member, table), Vec::<String>::new());
    }

    /// Every crate `Cargo.lock` holds at more than one line is listed,
    /// with its reason, and every one listed is still held at those
    /// lines. A failure names the crate: a duplicate that came back
    /// (most likely a `cargo update` undoing one of #284's pins; see
    /// the module docs), or one that went away and leaves its row
    /// stating a reason for nothing.
    #[test]
    fn test_every_duplicate_in_the_lockfile_has_a_stated_reason() {
        let text = std::fs::read_to_string(repo_path("Cargo.lock")).unwrap();
        assert!(
            locked_lines(&text).len() > 300,
            "precondition: the lockfile's packages are read"
        );
        let found = disagreements(&text, DUPLICATES);
        assert!(
            found.is_empty(),
            "`Cargo.lock` and `util::lock_duplicates::DUPLICATES` disagree:\n{}\n\
             A duplicate that came back costs a second build of the crate in every \
             `target/`; undo the `cargo update` that brought it (`cargo update -p \
             <crate>@<version> --precise <the other version>`) or, if it cannot be \
             undone, list it with the reason. A duplicate that went away takes its row \
             with it.",
            found.join("\n")
        );
        for (name, lines, reason) in DUPLICATES {
            assert!(lines.len() > 1, "{name}: a duplicate has two lines or more");
            assert!(!reason.trim().is_empty(), "{name}: every duplicate has a reason");
        }
    }
}
