// SPDX-License-Identifier: MPL-2.0

//! A projection is a function of the document, the font database and the
//! settings, never of what the process shaped before it (V1-B21, #323),
//! nor of the fonts the host has installed (#228).
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
//!
//! Until #228 the native font system began from the host's installed
//! fonts, and the default family was a name no compiled face bore, so
//! text that pinned no family was drawn in whatever the machine had:
//! DejaVu Sans on a Linux desktop, a decorative face or `.notdef` boxes
//! on a host with none and in the browser. The host comparison runs every
//! map in child processes whose fontconfig names an empty font directory,
//! or one planted with copies of compiled faces, against this process.

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

/// What a child runs `test_every_shipped_map_lays_out_alike_whatever_the_host_has_installed`
/// for: report, for the host its environment describes, how many faces
/// its font system holds that are not compiled in, whether every glyph of
/// the default-family fixture is the default face's, and each map's
/// digest.
const HOSTS: &str = "hosts";

/// A fontconfig file that lists `dir` as the host's one font directory,
/// as `FONTCONFIG_FILE` names a configuration to read.
fn fontconfig(dir: &std::path::Path) -> String {
    format!(
        "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n<fontconfig><dir>{}</dir></fontconfig>\n",
        dir.display()
    )
}

/// What `HOSTS` reports in this process: `STRAYS n`, `DEFAULT faults`
/// (the fixture's glyphs drawn in another face than the default face, or
/// as a `.notdef`), then `MAP name digest` for every shipped map.
fn host_report() -> Vec<String> {
    use baumhard::font::fallback::DEFAULT_FACE;
    use baumhard::font::fonts::COMPILED_FONT_ID_MAP;
    let compiled: Vec<_> = COMPILED_FONT_ID_MAP
        .values()
        .flat_map(|ids| ids.iter().copied())
        .collect();
    let default = &COMPILED_FONT_ID_MAP[&DEFAULT_FACE.0];
    let strays = {
        let mut fonts = FontSystemLease::new("tests/font_state::hosts");
        let names: Vec<String> = fonts
            .fonts()
            .db()
            .faces()
            .filter(|face| !compiled.contains(&face.id))
            .map(|face| face.post_script_name.clone())
            .collect();
        names
    };
    let doc = load(DEFAULT_FAMILY).expect("precondition: the fixture loads");
    let mut layers = GlyphLayers::new();
    let mut fonts = FontSystemLease::new("tests/font_state::hosts");
    Projector::project_full(&doc, &mut layers, &mut fonts);
    drop(fonts);
    let (mut glyphs, mut faults) = (0, Vec::new());
    for layer in Layer::ALL {
        let tree = layers.tree(layer);
        for id in tree.root().descendants(tree.arena()) {
            let Some(run) = tree.arena()[id].get().glyph_run() else {
                continue;
            };
            let RunShape::Box(shaped) = run.shaped() else {
                continue;
            };
            for line in shaped.buffer.layout_runs() {
                for glyph in line.glyphs {
                    glyphs += 1;
                    if !default.contains(&glyph.font_id) || glyph.glyph_id == 0 {
                        faults.push(format!("{:?}", line.text.get(glyph.start..glyph.end)));
                    }
                }
            }
        }
    }
    assert!(
        glyphs > 50,
        "precondition: the fixture's text is laid out ({glyphs} glyphs)"
    );
    let mut report = vec![format!("STRAYS {strays:?}"), format!("DEFAULT {faults:?}")];
    for (name, text) in the_maps() {
        report.push(format!("MAP {name} {}", digest(&layout(&text))));
    }
    report
}

#[test]
fn test_every_shipped_map_lays_out_alike_whatever_the_host_has_installed() {
    // #228: the font system holds the compiled faces and nothing of the
    // host, so a map lays out alike on a host with no fonts at all, on one
    // that has installed copies of compiled faces under their own names
    // (Inter, the default face, among them: what would shadow the default
    // face were the host's fonts indexed), and on this machine, whatever
    // it has. Each host is a child process whose fontconfig names one
    // directory, empty or planted, and whose home is empty; every glyph of
    // the default-family fixture is the default face's in each. Control:
    // with the host's fonts indexed again (`db.load_system_fonts()` in
    // `compiled_font_system`), red on the planted host's strays and on its
    // fixture drawn in the planted copy of Inter.
    if let Ok(what) = std::env::var(CHILD) {
        if what == HOSTS {
            for line in host_report() {
                println!("REPORT {line}");
            }
        }
        return;
    }
    let scratch = std::env::temp_dir().join(format!("baumhard_font_hosts_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let (empty, planted, home) = (
        scratch.join("empty"),
        scratch.join("planted"),
        scratch.join("home"),
    );
    for dir in [&empty, &planted, &home] {
        std::fs::create_dir_all(dir).expect("a scratch directory");
    }
    let fonts = root().join("lib/baumhard/src/font/fonts");
    for (from, to) in [
        ("inter-font/InterVariable.ttf", "InterVariable.ttf"),
        ("galatia-sil-font/GalatiaSil-Xdxg.ttf", "GalatiaSIL.ttf"),
        (
            "noto-sans-hebrew-font/NotoSansHebrew-Regular.ttf",
            "NotoSansHebrew.ttf",
        ),
    ] {
        std::fs::copy(fonts.join(from), planted.join(to)).unwrap_or_else(|e| panic!("{from}: {e}"));
    }
    let report = |name: &str, dir: &std::path::Path| -> Vec<String> {
        let conf = scratch.join(format!("{name}.conf"));
        std::fs::write(&conf, fontconfig(dir)).expect("a fontconfig file");
        let exe = std::env::current_exe().expect("the test binary");
        let out = std::process::Command::new(&exe)
            .args([
                "--exact",
                "test_every_shipped_map_lays_out_alike_whatever_the_host_has_installed",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, HOSTS)
            .env("FONTCONFIG_FILE", &conf)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .output()
            .expect("the test binary runs");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{name}: {stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        stdout
            .lines()
            .filter_map(|line| line.split_once("REPORT ").map(|(_, report)| report.to_string()))
            .collect()
    };
    let here = host_report();
    let hosts = [
        ("no fonts", report("empty", &empty)),
        ("planted", report("planted", &planted)),
    ];
    let _ = std::fs::remove_dir_all(&scratch);
    assert_eq!(
        here.len(),
        MAPS.len() + 2,
        "precondition: this process reports every map"
    );
    assert_eq!(here[0], "STRAYS []", "this process: faces not compiled in");
    assert_eq!(
        here[1], "DEFAULT []",
        "this process: default text not in the default face"
    );
    for (host, report) in hosts {
        assert_eq!(
            report.len(),
            here.len(),
            "precondition: the {host} host reports every map"
        );
        assert_eq!(report[0], "STRAYS []", "the {host} host: faces not compiled in");
        assert_eq!(
            report[1], "DEFAULT []",
            "the {host} host: default text not in the default face"
        );
        for (there, here) in report.iter().zip(&here).skip(2) {
            assert_eq!(
                there, here,
                "the {host} host lays a map out otherwise than this machine"
            );
        }
    }
}
