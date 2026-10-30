// SPDX-License-Identifier: MPL-2.0

//! The fuzz targets and their committed seeds, held to the properties on
//! the stable toolchain (TEST_CONVENTIONS §T14). `fuzz/` is outside the
//! workspace and is built only on nightly, so this is where `./test.sh`
//! sees it: every target in `fuzz/fuzz_targets/` is a row of [`TARGETS`]
//! and calls that row's property function, every row has a target, its
//! bin entry and its seeds, and every seed passes its target's
//! properties — and a seed named for what it holds holds it. Native
//! only: it reads the files.

use std::collections::BTreeSet;

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
