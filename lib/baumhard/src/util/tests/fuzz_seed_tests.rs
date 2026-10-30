// SPDX-License-Identifier: MPL-2.0

//! The fuzz targets and their committed seeds, held to the properties on
//! the stable toolchain (TEST_CONVENTIONS §T14). `fuzz/` is outside the
//! workspace and is built only on nightly, so this is where `./test.sh`
//! sees it: every target in `fuzz/fuzz_targets/` is a row of [`TARGETS`]
//! and calls that row's property function, every row has a target, its
//! bin entry and its seeds, and every seed passes its target's
//! properties — and a seed named for what it holds holds it. Beside
//! them: `fuzz/Cargo.lock` locks every package it shares with the
//! workspace at a version and source the workspace's lockfile does, and
//! every input `scripts/fuzz.sh` feeds a target fits that target's
//! `-max_len`. Native only: it reads the files.

use std::collections::{BTreeMap, BTreeSet};

use crate::document::tests::format_fuzz::{self, generate, MAX_GENERATED_ROWS};
use crate::flow::tests::flow_fuzz;
use crate::gfx_structs::mutator::GfxMutation;
use crate::gfx_structs::run::RunOverrideOp;
use crate::gfx_structs::tests::glyph_layer_fuzz;
use crate::util::doc_fixtures::repo_path;
use crate::util::tests::choices::Choices;

/// A target's property function.
type Check = fn(&[u8]) -> Result<(), String>;

/// Every fuzz target: its name, the property function its file calls (as
/// the file spells the call's last two segments) and the function.
const TARGETS: [(&str, &str, Check); 6] = [
    (
        "load_raw",
        "format_fuzz::check_load_bytes",
        format_fuzz::check_load_bytes,
    ),
    (
        "load_structured",
        "format_fuzz::check_structured",
        format_fuzz::check_structured,
    ),
    (
        "flow_raw",
        "flow_fuzz::check_flow_bytes",
        flow_fuzz::check_flow_bytes,
    ),
    (
        "flow_structured",
        "flow_fuzz::check_flow_structured",
        flow_fuzz::check_flow_structured,
    ),
    (
        "glyph_mutation_raw",
        "glyph_layer_fuzz::check_glyph_mutations_json",
        glyph_layer_fuzz::check_glyph_mutations_json,
    ),
    (
        "glyph_mutation_structured",
        "glyph_layer_fuzz::check_glyph_mutations_structured",
        glyph_layer_fuzz::check_glyph_mutations_structured,
    ),
];

/// The files directly in `dir`, by name, sorted.
fn files(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()))
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.is_file())
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(&path).unwrap())
        })
        .collect();
    out.sort();
    out
}

#[test]
fn test_every_fuzz_seed_holds_its_targets_properties() {
    let fuzz = repo_path("fuzz");
    let declared: BTreeSet<String> = files(&fuzz.join("fuzz_targets"))
        .into_iter()
        .filter_map(|(name, _)| name.strip_suffix(".rs").map(str::to_string))
        .collect();
    let rows: BTreeSet<String> = TARGETS.iter().map(|(name, _, _)| name.to_string()).collect();
    assert_eq!(
        declared, rows,
        "the targets in fuzz/fuzz_targets/ and this table name the same set"
    );
    let manifest = std::fs::read_to_string(fuzz.join("Cargo.toml")).unwrap();
    for (name, call, check) in TARGETS {
        let source = std::fs::read_to_string(fuzz.join("fuzz_targets").join(format!("{name}.rs"))).unwrap();
        assert!(source.contains(call), "{name}.rs calls {call}");
        assert!(
            manifest.contains(&format!("name = \"{name}\"")),
            "fuzz/Cargo.toml declares {name}"
        );
        let seeds = files(&fuzz.join("seeds").join(name));
        assert!(!seeds.is_empty(), "{name} has seeds");
        for (seed, bytes) in &seeds {
            check(bytes).unwrap_or_else(|why| panic!("{name}/{seed}: {why}"));
        }
    }
}

#[test]
fn test_the_structured_loader_seeds_hold_the_shapes_they_are_named_for() {
    let seeds = files(&repo_path("fuzz/seeds/load_structured"));
    let mut named = 0;
    for (name, bytes) in &seeds {
        if let Some(outcome) = name.strip_prefix("outcome_") {
            let got = format_fuzz::check_generated(&mut Choices::new(bytes), MAX_GENERATED_ROWS).unwrap();
            assert_eq!(got.replacen(": ", "_", 1), outcome, "{name}");
            named += 1;
            continue;
        }
        let spec = generate(&mut Choices::new(bytes), MAX_GENERATED_ROWS);
        let nodes = &spec.nodes;
        let holds = match name.as_str() {
            n if n.starts_with("deep_chain_links") => {
                nodes.len() >= 4000
                    && nodes.iter().enumerate().all(|(k, node)| {
                        node.parent == (k > 0).then(|| nodes[k - 1].id) && (k == 0 || node.links.len() == 1)
                    })
            }
            "wide_fan" => {
                nodes.len() >= 4000 && nodes[1..].iter().all(|node| node.parent == Some(nodes[0].id))
            }
            "dense_empty_sections_empty_rings_portal_links" => {
                nodes[0].links.len() >= 3000
                    && nodes[0]
                        .content
                        .iter()
                        .any(|(key, value)| key == "sections" && value.matches("{}").count() >= 3000)
                    && nodes[0]
                        .content
                        .iter()
                        .any(|(key, value)| key == "frame" && value.matches("{}").count() >= 3000)
            }
            "random_tree_1024" => nodes.len() == 1024,
            other => panic!("{other}: a seed whose name says nothing it holds"),
        };
        assert!(holds, "{name} holds what it is named for");
        named += 1;
    }
    assert_eq!(named, seeds.len());
    assert!(named >= 20, "precondition: the seeds are read ({named})");
}

#[test]
fn test_the_structured_glyph_mutation_seeds_hold_the_values_they_are_named_for() {
    let seeds = files(&repo_path("fuzz/seeds/glyph_mutation_structured"));
    assert!(seeds.len() >= 10, "precondition: the seeds are read");
    for (name, bytes) in &seeds {
        let (_, _, assigned) = glyph_layer_fuzz::generate(&mut Choices::new(bytes));
        let scale = |pred: fn(f32) -> bool| {
            assigned
                .iter()
                .any(|(_, m)| matches!(m, GfxMutation::RunOverrides(RunOverrideOp::Scale { value, .. }) if pred(*value)))
        };
        let holds = match name.as_str() {
            "scale_nan" => scale(f32::is_nan),
            "scale_infinite" => scale(f32::is_infinite),
            "scale_zero" => scale(|v| v == 0.0),
            "scale_negative" => scale(|v| v < 0.0 && v.is_finite()),
            "scale_huge" => scale(|v| v.is_finite() && v >= 1e30),
            "offset_nan" => assigned
                .iter()
                .any(|(_, m)| matches!(m, GfxMutation::RunOverrides(RunOverrideOp::Offset { value, .. }) if value.is_nan())),
            "offset_infinite" => assigned.iter().any(|(_, m)| {
                matches!(m, GfxMutation::RunOverrides(RunOverrideOp::Offset { value, .. }) if !value.is_finite() && !value.is_nan())
            }),
            "origin_nan" => assigned.iter().any(|(_, m)| matches!(m, GfxMutation::RunOrigin(v) if v.is_nan())),
            "fill_rect_infinite" => assigned
                .iter()
                .any(|(_, m)| matches!(m, GfxMutation::FillRect(r) if !(r.origin + r.size).is_finite())),
            "range_to_usize_max" => assigned.iter().any(|(_, m)| {
                matches!(m, GfxMutation::RunOverrides(RunOverrideOp::Scale { range, .. } | RunOverrideOp::Offset { range, .. })
                    if range.start == 0 && range.end == usize::MAX)
            }),
            other => panic!("{other}: a seed whose name says nothing it holds"),
        };
        assert!(holds, "{name} holds what it is named for");
    }
}

/// A lockfile's packages: each name with every `(version, source)` it is
/// locked at (`source` is `None` for a path package). Reads the
/// `[[package]]` tables' `name`, `version` and `source` lines, which is
/// all of a `Cargo.lock` this needs.
fn locked(text: &str) -> BTreeMap<String, BTreeSet<(String, Option<String>)>> {
    let mut out: BTreeMap<String, BTreeSet<(String, Option<String>)>> = BTreeMap::new();
    for table in text.split("[[package]]").skip(1) {
        let field = |key: &str| {
            table.lines().find_map(|line| {
                line.strip_prefix(key)
                    .and_then(|rest| rest.strip_prefix(" = \""))
                    .and_then(|rest| rest.strip_suffix('"'))
                    .map(str::to_string)
            })
        };
        let name = field("name").expect("every locked package has a name");
        let version = field("version").expect("every locked package has a version");
        out.entry(name).or_default().insert((version, field("source")));
    }
    out
}

/// Each package `fuzz` locks at a version or source the workspace does
/// not lock it at, named. A package the workspace locks at two versions
/// may be locked at either; one only `fuzz/` uses (libfuzzer-sys, the
/// fuzz package itself) is not compared.
fn lock_drift(root: &str, fuzz: &str) -> Vec<String> {
    let root = locked(root);
    locked(fuzz)
        .into_iter()
        .filter_map(|(name, versions)| {
            let workspace = root.get(&name)?;
            (!versions.is_subset(workspace))
                .then(|| format!("{name}: fuzz/ locks {versions:?}, the workspace {workspace:?}"))
        })
        .collect()
}

/// `fuzz/Cargo.lock` builds baumhard with the dependencies the
/// workspace's `Cargo.lock` does: a fuzz run on other versions would
/// hold other code to the properties (review of #293, blocking item 3).
/// `fuzz/` is outside the workspace, so nothing else keeps the two in
/// step; after the root lockfile changes, copy it over `fuzz/Cargo.lock`
/// and run `cargo update -p mandala-fuzz` in `fuzz/` on its nightly,
/// which adds the fuzz-only packages and keeps every shared version.
#[test]
fn test_the_fuzz_lockfile_locks_what_the_workspace_locks() {
    let root = std::fs::read_to_string(repo_path("Cargo.lock")).unwrap();
    let fuzz = std::fs::read_to_string(repo_path("fuzz/Cargo.lock")).unwrap();
    let workspace = locked(&root);
    let shared = locked(&fuzz)
        .keys()
        .filter(|name| workspace.contains_key(*name))
        .count();
    assert!(
        shared >= 50,
        "precondition: the two lockfiles share packages ({shared})"
    );
    let drift = lock_drift(&root, &fuzz);
    assert!(
        drift.is_empty(),
        "fuzz/Cargo.lock has drifted from Cargo.lock:\n{}",
        drift.join("\n")
    );
    // Negative control: one shared package moved to another version.
    let version = locked(&fuzz)["serde_json"]
        .iter()
        .next()
        .map(|(version, _)| version.clone())
        .expect("precondition: both lockfiles lock serde_json");
    let moved = fuzz.replacen(
        &format!("name = \"serde_json\"\nversion = \"{version}\""),
        &format!("name = \"serde_json\"\nversion = \"{version}9\""),
        1,
    );
    assert_ne!(moved, fuzz, "precondition: the control edits the text");
    assert_eq!(
        lock_drift(&root, &moved).len(),
        1,
        "a moved version is drift"
    );
}

/// `scripts/fuzz.sh`'s `-max_len` for each target, from its
/// `<target>) MAX_LEN=<n> ;;` arms.
fn max_lens(script: &str) -> BTreeMap<String, usize> {
    script
        .lines()
        .filter_map(|line| {
            let (target, rest) = line.trim().split_once(") MAX_LEN=")?;
            let length = rest.strip_suffix(" ;;")?.parse().ok()?;
            Some((target.to_string(), length))
        })
        .collect()
}

/// The directories `scripts/fuzz.sh` reads as `target`'s corpus besides
/// `fuzz/corpus/` (written by the run) and `fuzz/seeds/<target>`: the
/// quoted `lib/` paths of the `CORPORA+=(` lines under each `case` arm
/// whose pattern matches `target`, and whether such an arm links the
/// repository's `maps/*.mandala.json`.
fn script_corpora(script: &str, target: &str) -> (Vec<String>, bool) {
    let mut arm: Option<String> = None;
    let mut dirs = Vec::new();
    let mut maps = false;
    for line in script.lines() {
        let trimmed = line.trim();
        if let Some(pattern) = trimmed.strip_suffix(')') {
            let is_arm = !pattern.is_empty()
                && pattern
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c == '*');
            if is_arm {
                arm = Some(pattern.to_string());
                continue;
            }
        }
        let matches = arm
            .as_deref()
            .is_some_and(|pattern| match pattern.strip_suffix('*') {
                Some(prefix) => target.starts_with(prefix),
                None => pattern == target,
            });
        if !matches {
            continue;
        }
        if trimmed.starts_with("CORPORA+=(") {
            dirs.extend(
                trimmed
                    .split('"')
                    .filter(|part| part.starts_with("lib/"))
                    .map(str::to_string),
            );
        }
        maps |= trimmed.contains("maps/*.mandala.json");
    }
    (dirs, maps)
}

/// Every input `scripts/fuzz.sh` feeds a target fits that target's
/// `-max_len`: libFuzzer cuts a longer one to the ceiling without a
/// word, so a seed past it would be read as some other, truncated input
/// (review of #293, optional item i).
#[test]
fn test_every_fuzz_input_fits_its_targets_max_len() {
    let script = std::fs::read_to_string(repo_path("scripts/fuzz.sh")).unwrap();
    let lengths = max_lens(&script);
    let names: BTreeSet<String> = TARGETS
        .iter()
        .map(|(name, _, _)| name.to_string())
        .collect();
    assert_eq!(
        lengths.keys().cloned().collect::<BTreeSet<String>>(),
        names,
        "scripts/fuzz.sh sets a -max_len for exactly the targets"
    );
    let mut longest = 0;
    for (name, _, _) in TARGETS {
        let ceiling = lengths[name];
        let (dirs, maps) = script_corpora(&script, name);
        if name.starts_with("load_") {
            assert!(
                dirs.len() == 2 && maps,
                "precondition: {name} reads the fixtures, the findings and the maps ({dirs:?}, {maps})"
            );
        }
        let mut inputs = files(&repo_path("fuzz/seeds").join(name));
        for dir in &dirs {
            inputs.extend(files(&repo_path(dir)));
        }
        let map_dir = repo_path("maps");
        if maps && map_dir.is_dir() {
            inputs.extend(
                files(&map_dir)
                    .into_iter()
                    .filter(|(file, _)| file.ends_with(".mandala.json")),
            );
        }
        for (file, bytes) in &inputs {
            longest = longest.max(bytes.len());
            assert!(
                bytes.len() <= ceiling,
                "{name}: {file} is {} bytes, past its -max_len of {ceiling}",
                bytes.len()
            );
        }
    }
    assert!(
        longest > 32_768,
        "precondition: the deep-chain seeds are read ({longest} bytes the longest)"
    );
}
