// SPDX-License-Identifier: MPL-2.0

//! A projection is a function of the document, the font database and the
//! settings, never of what the process shaped before it (V1-B21, #323).
//!
//! Until #323 the compiled faces were registered lazily, on the first read
//! of the compiled font table: at `fonts::init`, or the first time a span
//! pinned a compiled family. Text in the default family shaped before that
//! was shaped with the host's faces alone, so a map laid out one way in a
//! fresh process and another after a map pinning a compiled family had
//! been projected: testament's bold "Lord God" measured 377.77 canvas
//! units, then 375.53. Now the font system is built complete before it is
//! put in its lock.
//!
//! **This file calls `fonts::init` nowhere**, on purpose: whether, and
//! when, it runs must not matter. It is its own test binary, so its
//! process starts with nothing shaped; the fresh-process comparison runs
//! each map again in a child process of its own.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use baumhard::document::format::load;
use baumhard::font::fonts::FontSystemLease;
use baumhard::gfx_structs::run::RunShape;
use baumhard::projection::{GlyphLayers, Layer, Projector};

/// Set in a child process: the map it projects, fresh, and reports.
const CHILD: &str = "BAUMHARD_FONT_STATE_CHILD";

/// A map in the default family only, the shape of testament's first node:
/// a bold heading at 74 points, and body text in regular and italic. It
/// pins no compiled family, so projecting it reads nothing of the compiled
/// font table: the text #323's lazy registration shaped with the host's
/// faces alone. It is the A of the A-then-B-then-A order, where a map
/// small enough to compare line by line keeps a failure's message short;
/// testament itself, the map #323 was reported on, is in `MAPS`.
const DEFAULT_FAMILY: &str = r#"{"format":1,"name":"default family","nodes":[
  {"id":1,"position":[0,0],"sections":[
    {"spans":[{"text":"Lord God","size":74,"weight":"bold"}]},
    {"spans":[{"text":"commanded the man, saying, Of every tree of the garden","size":29},
              {"text":" thou mayest freely eat","size":29,"italic":true}]}]},
  {"id":2,"position":[0,400],"sections":[
    {"spans":[{"text":"Genesis 2:16","size":20,"italic":true}]}]}]}"#;

/// The repository root.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The maps compared, by name: the default-family fixture, then every
/// shipped map (`maps/demos/*.mandala.json`, `maps/*.mandala.json`).
fn maps() -> Vec<(String, String)> {
    let mut maps = vec![("default family".to_string(), DEFAULT_FAMILY.to_string())];
    for dir in ["maps/demos", "maps"] {
        let mut found: Vec<PathBuf> = std::fs::read_dir(root().join(dir))
            .unwrap_or_else(|e| panic!("{dir}: {e}"))
            .map(|entry| entry.expect("a directory entry").path())
            .filter(|path| path.to_string_lossy().ends_with(".mandala.json"))
            .collect();
        found.sort();
        for path in found {
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            maps.push((
                format!("{dir}/{}", path.file_name().unwrap_or_default().to_string_lossy()),
                text,
            ));
        }
    }
    maps
}

/// What `maps` reads, by name: the fixture and every shipped map,
/// testament among them. A precondition holds the two to each other, so
/// a map added, removed or renamed is named in the failure rather than
/// counted past.
const MAPS: [&str; 9] = [
    "default family",
    "maps/demos/blake.mandala.json",
    "maps/demos/dickinson.mandala.json",
    "maps/demos/ecclesiastes.mandala.json",
    "maps/demos/frames.mandala.json",
    "maps/palette_cascade.mandala.json",
    "maps/stress_long_edges.mandala.json",
    "maps/testament.mandala.json",
    "maps/theme_demo.mandala.json",
];

/// `maps`, once its names are `MAPS`.
fn the_maps() -> Vec<(String, String)> {
    let maps = maps();
    let names: Vec<&str> = maps.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names, MAPS,
        "precondition: the maps compared are the fixture and every shipped map"
    );
    maps
}

/// Every run `text` projects to, in every layer, as the facts its drawing
/// reads: its origin and box, and each glyph's face, glyph id, size and
/// position, laid out (a box run) or placed (a path run).
fn layout(text: &str) -> Vec<String> {
    let doc = load(text).unwrap_or_else(|error| panic!("precondition: the map loads: {error}"));
    let mut layers = GlyphLayers::new();
    let mut fonts = FontSystemLease::new("tests/font_state");
    Projector::project_full(&doc, &mut layers, &mut fonts);
    drop(fonts);
    let mut out = Vec::new();
    for layer in Layer::ALL {
        let tree = layers.tree(layer);
        for id in tree.root().descendants(tree.arena()) {
            let Some(run) = tree.arena()[id].get().glyph_run() else {
                continue;
            };
            out.push(format!("{layer:?} at {:?} box {:?}", run.origin(), run.bounds()));
            match run.shaped() {
                RunShape::Box(shaped) => {
                    for line in shaped.buffer.layout_runs() {
                        for glyph in line.glyphs {
                            out.push(format!(
                                "  {:?} {:?} face {:?} glyph {} size {} at {} {} w {}",
                                line.text.get(glyph.start..glyph.end),
                                glyph.font_weight,
                                glyph.font_id,
                                glyph.glyph_id,
                                glyph.font_size,
                                glyph.x,
                                line.line_y,
                                glyph.w
                            ));
                        }
                    }
                }
                RunShape::Path { run, placed } => {
                    for (glyph, laid) in run.glyphs.iter().zip(run.layout_glyphs()) {
                        out.push(format!(
                            "  {glyph:?} face {:?} glyph {}",
                            laid.font_id, laid.glyph_id
                        ));
                    }
                    out.push(format!("  placed {:?}", placed.glyphs));
                }
            }
        }
    }
    out
}

/// A layout's digest, to compare across processes.
fn digest(layout: &[String]) -> String {
    let mut hasher = DefaultHasher::new();
    layout.hash(&mut hasher);
    format!("{:016x} {}", hasher.finish(), layout.len())
}

/// The first line two layouts differ at, for a failure's message.
fn first_difference(a: &[String], b: &[String]) -> String {
    a.iter().zip(b).find(|(a, b)| a != b).map_or_else(
        || format!("lengths {} and {}", a.len(), b.len()),
        |(a, b)| format!("{a}\n  then\n{b}"),
    )
}

/// What a child runs `test_a_map_lays_out_alike_before_and_after_another`
/// for: the A-then-B-then-A order, in a process that has shaped nothing.
const ABA: &str = "a, b, a";

/// Run this binary again for the test `test`, with the child variable set
/// to `what`; its standard output, which must report success. The child
/// runs on one thread whatever `RUST_TEST_THREADS` says, so its report is
/// in the order it printed it.
fn in_a_child(test: &str, what: &str) -> String {
    let exe = std::env::current_exe().expect("the test binary");
    let out = std::process::Command::new(&exe)
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD, what)
        .output()
        .expect("the test binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{what}: {stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn test_a_map_lays_out_alike_before_and_after_another() {
    // In a child, so that its first projection is the first shaping its
    // process makes: this binary's tests run in parallel threads of one
    // process, and another's shaping first would hide the defect.
    match std::env::var(CHILD) {
        Err(_) => {
            in_a_child("test_a_map_lays_out_alike_before_and_after_another", ABA);
            return;
        }
        Ok(what) if what != ABA => return,
        Ok(_) => {}
    }
    // A, then B, then A again: A is the default-family map, which reads
    // nothing of the compiled font table; B is every demo map, each of
    // which pins compiled families.
    let first = layout(DEFAULT_FAMILY);
    assert!(
        first
            .iter()
            .any(|line| line.contains("\"Lord God\"") || line.contains("Some(\"L\")")),
        "precondition: the fixture's heading is laid out"
    );
    let demos: Vec<(String, String)> = the_maps()
        .into_iter()
        .filter(|(name, _)| name.starts_with("maps/demos/"))
        .collect();
    for (_, text) in &demos {
        layout(text);
    }
    let again = layout(DEFAULT_FAMILY);
    assert!(
        first == again,
        "the default-family map laid out otherwise after the demo maps: {}",
        first_difference(&first, &again)
    );
}

/// Run this binary again for `map`, alone, and read the digest it reports.
fn fresh_digest(name: &str) -> String {
    let stdout = in_a_child(
        "test_a_fresh_process_lays_out_each_shipped_map_as_a_primed_one",
        name,
    );
    // On one thread libtest prints the test's name on the line the
    // child's report starts, so the marker is found within a line.
    stdout
        .lines()
        .find_map(|line| line.split_once("DIGEST ").map(|(_, digest)| digest.to_string()))
        .unwrap_or_else(|| panic!("{name} reported nothing:\n{stdout}"))
}

#[test]
fn test_a_fresh_process_lays_out_each_shipped_map_as_a_primed_one() {
    let maps = the_maps();
    if let Ok(name) = std::env::var(CHILD) {
        // The child: this map, first thing in its process.
        let (_, text) = maps
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("no map named {name}"));
        println!("DIGEST {}", digest(&layout(text)));
        return;
    }
    // The parent: every map once, so every face any of them uses has
    // been used, then each again, primed, against the same map first in a
    // process of its own.
    for (_, text) in &maps {
        layout(text);
    }
    for (name, text) in &maps {
        let primed = digest(&layout(text));
        let fresh = fresh_digest(name);
        assert_eq!(
            fresh, primed,
            "{name}: laid out otherwise first in a fresh process than after every other map"
        );
    }
}
