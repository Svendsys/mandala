// SPDX-License-Identifier: MPL-2.0

//! The §B8 bench-surface contract, held against the tree instead of
//! written down and trusted: **every `pub fn do_*()` in a
//! `pub mod tests;` tree has a `bench_function` entry in
//! `benches/test_bench.rs`.**
//!
//! Nothing else enforces this direction. The bench file imports the
//! test modules by glob, so a body that never gets an entry — or
//! silently loses one — upsets no compiler; `cargo check
//! --workspace --benches` only catches the *reverse* drift, an
//! entry whose body is gone. Issue #44 measured what discipline
//! alone had produced: 173 of 425 bodies unbenched, seven whole
//! modules never imported, and headers still claiming otherwise.
//!
//! The contract makes the `do_` prefix mean something checkable:
//! writing the `do_*()` / `test_*()` pair *is* putting the body on
//! the bench surface. A test with no benchmark value opts out by
//! being a plain `#[test]` fn instead — `#[test]` strips it from
//! non-test builds, which is exactly what removes it from the
//! surface this module scans. See `CONVENTIONS.md` §B8 for which
//! test shapes belong in each class.
//!
//! Built on [`crate::util::source_scan`]'s walkers so "which files
//! are a test-gated tree" is answered once, there — a
//! `#[cfg(test)] mod tests;` tree like `mindmap/tree_builder`'s
//! carries no bench obligation and is excluded by the same resolver
//! every other repo-as-subject scan uses. Test-only and
//! native-only, for the same reasons `source_scan` is.

use crate::util::doc_fixtures::repo_path;
use crate::util::rust_source::strip_comments;
use crate::util::source_scan::{
    is_test_gated, relative_to_repo, test_gated_module_files, workspace_rust_sources,
};
use std::path::PathBuf;
use syn::Item;

/// Every benchmark-reusable body the baumhard tests trees export,
/// as `(function name, repo-relative file)`: the `pub fn do_*()`
/// functions declared in files under a `tests/` directory of
/// `lib/baumhard/src` that are **not** reached through a
/// `#[cfg(test)] mod` declaration.
///
/// Cost: one read and one `syn` parse of every `.rs` file in the
/// workspace (the test-gate resolution), plus one parse per
/// tests-tree file.
pub(crate) fn benchmark_reusable_bodies() -> Vec<(String, String)> {
    let sources = workspace_rust_sources();
    let test_only = test_gated_module_files(&sources);
    let tree_files: Vec<PathBuf> = sources
        .into_iter()
        .filter(|file| {
            let rel = relative_to_repo(file);
            rel.starts_with("lib/baumhard/src/") && rel.contains("/tests/") && !test_only.contains(file)
        })
        .collect();
    collect_do_bodies(&tree_files)
}

/// The `pub fn do_*()` declarations in `files`, as `(name,
/// repo-relative file)`. Descends into ungated inline `mod` blocks;
/// skips any item behind a `cfg(test)` gate, because the bench
/// binary is not compiled under `cfg(test)` and cannot reach it.
pub(crate) fn collect_do_bodies(files: &[PathBuf]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        let parsed =
            syn::parse_file(&text).unwrap_or_else(|e| panic!("{} must parse as Rust: {e}", file.display()));
        collect_from_items(&parsed.items, &relative_to_repo(file), &mut out);
    }
    out.sort();
    out
}

fn collect_from_items(items: &[Item], rel: &str, out: &mut Vec<(String, String)>) {
    for item in items {
        match item {
            Item::Fn(fun) => {
                if is_test_gated(&fun.attrs) {
                    continue;
                }
                let name = fun.sig.ident.to_string();
                if matches!(fun.vis, syn::Visibility::Public(_)) && name.starts_with("do_") {
                    out.push((name, rel.to_string()));
                }
            }
            Item::Mod(module) => {
                if is_test_gated(&module.attrs) {
                    continue;
                }
                if let Some((_, inner)) = &module.content {
                    collect_from_items(inner, rel, out);
                }
            }
            _ => {}
        }
    }
}

/// The bodies in `bodies` that `bench_source` never references, as
/// rendered `"<file>: <name>"` lines.
///
/// Comments are stripped from the bench source first, so a
/// commented-out entry does not satisfy the contract, and a name
/// only counts when it appears as a whole identifier — an entry for
/// `do_x_and_more` says nothing about `do_x`.
pub(crate) fn missing_bench_entries(bodies: &[(String, String)], bench_source: &str) -> Vec<String> {
    let live = strip_comments(bench_source);
    bodies
        .iter()
        .filter(|(name, _)| !ident_is_referenced(&live, name))
        .map(|(name, file)| format!("{file}: {name}"))
        .collect()
}

/// Whether `name` occurs in `text` as a whole identifier — not as a
/// substring of a longer one.
fn ident_is_referenced(text: &str, name: &str) -> bool {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(at) = text[from..].find(name) {
        let start = from + at;
        let end = start + name.len();
        let free_before = start == 0 || !is_ident_byte(bytes[start - 1]);
        let free_after = end >= bytes.len() || !is_ident_byte(bytes[end]);
        if free_before && free_after {
            return true;
        }
        from = start + 1;
    }
    false
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_temp::TempDir;

    /// The tests-tree prefixes the scan must show work from. Same
    /// posture as `source_scan`'s citation witnesses: "nothing is
    /// missing" is exactly as loud when the walk has stopped
    /// looking, so each tree that carries bodies today is pinned by
    /// name and a root that moves goes red naming itself.
    const BODY_WITNESSES: &[&str] = &[
        "lib/baumhard/src/core/tests/",
        "lib/baumhard/src/font/tests/",
        "lib/baumhard/src/gfx_structs/tests/",
        "lib/baumhard/src/util/tests/",
    ];

    /// **Every `do_*()` body has a bench entry** — §B8's checked
    /// direction. The reverse (an entry whose body is gone) fails
    /// `cargo check --workspace --benches`, which `./test.sh` runs.
    ///
    /// The failure lists every body missing its entry; the fix is
    /// one `c.bench_function("<name minus do_>", |b| b.iter(<name>))`
    /// per line, or — for a body that should not be a benchmark —
    /// folding it into its plain `#[test]` wrapper (§B8's opt-out).
    #[test]
    fn test_every_do_body_has_a_bench_entry() {
        let bodies = benchmark_reusable_bodies();
        for witness in BODY_WITNESSES {
            assert!(
                bodies.iter().any(|(_, file)| file.starts_with(witness)),
                "the scan found no do_* bodies under `{witness}`, so it is no longer \
                 looking there — a body missing its entry would pass in silence"
            );
        }
        // Floor well under the ~520 bodies the trees hold today:
        // a syn-walk defect that dropped whole files would land
        // here before it could hollow out the check. Lower it
        // consciously if the trees genuinely shrink.
        assert!(
            bodies.len() >= 400,
            "the scan found only {} do_* bodies where roughly 520 exist; the walk \
             has lost most of the surface it checks",
            bodies.len()
        );
        let bench_path = repo_path("lib/baumhard/benches/test_bench.rs");
        let bench_source = std::fs::read_to_string(&bench_path)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", bench_path.display()));
        let missing = missing_bench_entries(&bodies, &bench_source);
        assert!(
            missing.is_empty(),
            "{} do_* body/bodies have no entry in benches/test_bench.rs. Add one \
             `c.bench_function` per body (id = name minus `do_`), or fold a body \
             that should not be benched into its `#[test]` wrapper — §B8 names the \
             opt-out classes:\n  {}",
            missing.len(),
            missing.join("\n  ")
        );
    }

    /// Positive control: a body the bench text never mentions is
    /// reported, and a body it mentions is not. Proves the repo
    /// test *can* fail before trusting its silence.
    #[test]
    fn test_a_body_without_an_entry_is_reported() {
        let bodies = vec![
            ("do_present".to_string(), "a.rs".to_string()),
            ("do_absent".to_string(), "b.rs".to_string()),
        ];
        let bench = r#"c.bench_function("present", |b| b.iter(do_present));"#;
        assert_eq!(missing_bench_entries(&bodies, bench), vec!["b.rs: do_absent"]);
    }

    /// The control that disables the mechanism on the checked path:
    /// comment stripping is what stands between "the entry exists"
    /// and "the entry is written somewhere in the file's bytes". A
    /// reference that survives only inside a comment must not
    /// satisfy the contract.
    #[test]
    fn test_a_commented_out_entry_does_not_satisfy_the_contract() {
        let bodies = vec![("do_present".to_string(), "a.rs".to_string())];
        let live = r#"c.bench_function("present", |b| b.iter(do_present));"#;
        let commented = r#"// c.bench_function("present", |b| b.iter(do_present));"#;
        assert!(missing_bench_entries(&bodies, live).is_empty());
        assert_eq!(
            missing_bench_entries(&bodies, commented),
            vec!["a.rs: do_present"]
        );
    }

    /// An entry for `do_x_and_more` says nothing about `do_x`: the
    /// reference has to be the whole identifier, or the commonest
    /// drift — renaming a body by growing its name and updating the
    /// bench file — would leave the old body's contract satisfied
    /// by the new body's entry.
    #[test]
    fn test_a_longer_identifier_does_not_satisfy_a_shorter_body() {
        let bodies = vec![("do_x".to_string(), "a.rs".to_string())];
        let bench = r#"c.bench_function("x_and_more", |b| b.iter(do_x_and_more));"#;
        assert_eq!(missing_bench_entries(&bodies, bench), vec!["a.rs: do_x"]);
    }

    /// The collector reads only what the bench binary can reach: a
    /// `pub fn do_*` counts, a private one does not, and anything
    /// behind a `cfg(test)` gate — including a whole inline module —
    /// is invisible to a binary compiled without `cfg(test)`.
    #[test]
    fn test_the_collector_sees_only_pub_ungated_do_fns() {
        let dir = TempDir::new("bench-surface-collect");
        std::fs::write(
            dir.join("planted.rs"),
            "pub fn do_reachable() {}\n\
             fn do_private() {}\n\
             #[cfg(test)]\n\
             pub fn do_gated() {}\n\
             #[cfg(test)]\n\
             mod inner {\n    pub fn do_inside_gate() {}\n}\n\
             mod ungated_inner {\n    pub fn do_nested_reachable() {}\n}\n\
             #[test]\n\
             fn test_do_reachable() {}\n",
        )
        .expect("planted file must be writable");
        let found = collect_do_bodies(&[dir.join("planted.rs")]);
        let names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"do_reachable"), "{names:?}");
        assert!(names.contains(&"do_nested_reachable"), "{names:?}");
        assert!(!names.contains(&"do_private"), "{names:?}");
        assert!(!names.contains(&"do_gated"), "{names:?}");
        assert!(!names.contains(&"do_inside_gate"), "{names:?}");
    }
}
