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
//! Until #228 the default family was a name no compiled face bore, so
//! text that pinned no family was drawn in whatever the machine had:
//! DejaVu Sans on a Linux desktop, a decorative face or `.notdef` boxes
//! on a host with none and in the browser. The host's fonts are still
//! loaded on native, after the compiled faces, for what no compiled face
//! holds. The host comparison runs every map in child processes whose
//! fontconfig names an empty font directory, or one planted with copies
//! of compiled faces, against this process.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use baumhard::document::format::load;
use baumhard::font::fonts::FontSystemLease;
use baumhard::gfx_structs::run::RunShape;
use baumhard::projection::{GlyphLayers, Layer, Projector};

/// Set in a child process: the map it projects, fresh, and reports.
const CHILD: &str = "BAUMHARD_FONT_STATE_CHILD";

/// Set where the host test's planted hosts must be built (CI's Linux
/// runner): a target whose fontdb does not read `FONTCONFIG_FILE` fails
/// there, where it would skip with a note elsewhere.
const REQUIRE_HOSTS: &str = "MANDALA_REQUIRE_FONTCONFIG_HOSTS";

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

/// The faces a host's own font index holds where this process runs, by
/// PostScript name, sorted: what `fontdb::Database::load_system_fonts`
/// finds, the call cosmic-text's `FontSystem::new` makes. The host test's
/// precondition: it reads the planted fonts, and none for the empty
/// host, or the hosts are no adversary.
fn host_faces() -> Vec<String> {
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_system_fonts();
    let mut names: Vec<String> = db.faces().map(|face| face.post_script_name.clone()).collect();
    names.sort();
    names
}

/// A run of each compiled script face's script, with the face that must
/// draw it: what `HOSTS` shapes at every weight and slant, against the
/// planted host's bold face of each script ([`renamed_bold`]).
const SCRIPTS: [(&str, baumhard::font::fonts::AppFont); 4] = [
    ("שלום", baumhard::font::fonts::AppFont::NotoSansHebrewRegular),
    ("नमस्ते", baumhard::font::fonts::AppFont::NotoSansDevanagariRegular),
    ("བོད", baumhard::font::fonts::AppFont::NotoSerifTibetanRegular),
    (
        "𓀀𓁐",
        baumhard::font::fonts::AppFont::NotoSansEgyptianHieroglyphsRegular,
    ),
];

/// The compiled script faces [`SCRIPTS`] names, by file under the font
/// directory, as the planted host's bold faces are made from them.
const SCRIPT_FILES: [&str; 4] = [
    "noto-sans-hebrew-font/NotoSansHebrew-Regular.ttf",
    "noto-sans-devanagari-font/NotoSansDevanagari-Regular.ttf",
    "noto-serif-tibetan-font/NotoSerifTibetan-Regular.ttf",
    "noto-sans-egyptian-hieroglyphs-font/NotoSansEgyptianHieroglyphs-Regular.ttf",
];

/// A copy of the font `data` a host could have installed beside the
/// compiled one: every `Noto` in its `name` table spelled `Host`, so it
/// bears no compiled family's name and is not removed as a shadow, and
/// its `OS/2` weight class 700, so a bold span meets it at its own weight
/// in cosmic-text's last pass. The outlines are the compiled face's. Made
/// at test time, byte for byte the same every run; the table checksums
/// are left stale, which nothing that loads a face checks.
fn renamed_bold(data: &[u8]) -> Vec<u8> {
    let u16_at = |at: usize| u16::from_be_bytes([data[at], data[at + 1]]) as usize;
    let u32_at =
        |at: usize| u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize;
    let mut out = data.to_vec();
    let (mut name, mut os2) = (None, None);
    for record in (0..u16_at(4)).map(|i| 12 + 16 * i) {
        let (offset, length) = (u32_at(record + 8), u32_at(record + 12));
        match &data[record..record + 4] {
            b"name" => name = Some(offset..offset + length),
            b"OS/2" => os2 = Some(offset),
            _ => {}
        }
    }
    let name = name.expect("precondition: the face has a name table");
    let os2 = os2.expect("precondition: the face has an OS/2 table");
    let table = &mut out[name];
    let spellings: [(&[u8], &[u8]); 2] = [(b"\0N\0o\0t\0o", b"\0H\0o\0s\0t"), (b"Noto", b"Host")];
    for (from, to) in spellings {
        let mut at = 0;
        while at + from.len() <= table.len() {
            if &table[at..at + from.len()] == from {
                table[at..at + from.len()].copy_from_slice(to);
                at += from.len();
            } else {
                at += 1;
            }
        }
    }
    out[os2 + 4..os2 + 6].copy_from_slice(&700u16.to_be_bytes());
    out
}

/// Each [`SCRIPTS`] run, shaped unpinned at every CSS weight, upright
/// and italic, in the font system: every glyph that is not its script's
/// compiled face's, or is a `.notdef`, as `weight style face`.
fn script_faults() -> Vec<String> {
    use baumhard::font::fonts::COMPILED_FONT_ID_MAP;
    use cosmic_text::{Attrs, Buffer, Metrics, Shaping, Style, Weight};
    let mut fonts = FontSystemLease::new("tests/font_state::scripts");
    let fs = fonts.fonts();
    let mut faults = Vec::new();
    let mut glyphs = 0;
    for (text, face) in SCRIPTS {
        let ids = &COMPILED_FONT_ID_MAP[&face];
        for weight in [100, 200, 300, 400, 500, 600, 700, 800, 900] {
            for style in [Style::Normal, Style::Italic] {
                let mut buffer = Buffer::new(fs, Metrics::new(16.0, 20.0));
                let attrs = Attrs::new().weight(Weight(weight)).style(style);
                buffer.set_text(text, &attrs, Shaping::Advanced, None);
                buffer.shape_until_scroll(fs, false);
                let drawn: Vec<(cosmic_text::fontdb::ID, u16)> = buffer
                    .layout_runs()
                    .flat_map(|run| {
                        run.glyphs
                            .iter()
                            .map(|g| (g.font_id, g.glyph_id))
                            .collect::<Vec<_>>()
                    })
                    .collect();
                glyphs += drawn.len();
                for (id, glyph) in drawn {
                    if !ids.contains(&id) || glyph == 0 {
                        let name = fs.db().face(id).map(|f| f.post_script_name.clone());
                        faults.push(format!("{text} {weight} {style:?} {name:?} {glyph}"));
                    }
                }
            }
        }
    }
    assert!(
        glyphs >= 4 * 18,
        "precondition: every script run is shaped ({glyphs} glyphs)"
    );
    faults
}

/// A scratch directory, removed when dropped, so a failed child's panic
/// does not leave it behind.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What `HOSTS` reports in this process: `HOST faces` (what the host's
/// font index holds, [`host_faces`]), `SHADOWS faces` (the font
/// system's faces that are not compiled in and bear a compiled family's
/// name), `DEFAULT faults` (the fixture's glyphs drawn in another face
/// than the default face, or as a `.notdef`), `SCRIPTS faults`
/// ([`script_faults`]), then `MAP name digest` for every shipped map.
fn host_report() -> Vec<String> {
    use baumhard::font::fallback::DEFAULT_FACE;
    use baumhard::font::fonts::COMPILED_FONT_ID_MAP;
    let compiled: Vec<_> = COMPILED_FONT_ID_MAP
        .values()
        .flat_map(|ids| ids.iter().copied())
        .collect();
    let default = &COMPILED_FONT_ID_MAP[&DEFAULT_FACE.0];
    let shadows = {
        let mut fonts = FontSystemLease::new("tests/font_state::hosts");
        let db = fonts.fonts().db();
        let names: Vec<String> = compiled
            .iter()
            .filter_map(|id| db.face(*id))
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
            .collect();
        let shadows: Vec<String> = db
            .faces()
            .filter(|face| !compiled.contains(&face.id))
            .filter(|face| face.families.iter().any(|(name, _)| names.contains(name)))
            .map(|face| face.post_script_name.clone())
            .collect();
        shadows
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
    let mut report = vec![
        format!("HOST {:?}", host_faces()),
        format!("SHADOWS {shadows:?}"),
        format!("DEFAULT {faults:?}"),
        format!("SCRIPTS {:?}", script_faults()),
    ];
    for (name, text) in the_maps() {
        report.push(format!("MAP {name} {}", digest(&layout(&text))));
    }
    report
}

#[test]
fn test_every_shipped_map_lays_out_alike_whatever_the_host_has_installed() {
    // #228: the font system loads the host's fonts after the compiled
    // faces and drops any that bears a compiled family's name, and text
    // that names no family asks for the compiled default face by name, so
    // a map lays out alike on a host with no fonts at all, on one that has
    // installed copies of compiled faces under their own names (Inter,
    // the default face, and Galatia SIL among them: what would shadow the
    // default face and a pin were they kept), and on this machine,
    // whatever it has. Each host is a child process whose fontconfig
    // names one directory, empty or planted, and whose home is empty;
    // every glyph of the default-family fixture is the default face's in
    // each. The planted host also holds a bold face of each compiled
    // script under a name of its own ([`renamed_bold`]), which no run of
    // the script, at any weight or slant, may reach.
    //
    // The hosts are an adversary only where fontdb reads
    // `FONTCONFIG_FILE` (Linux, with cosmic-text's `fontconfig` feature),
    // so each child first reports what the host's own font index holds,
    // and the test requires the planted faces and nothing else, and
    // nothing for the empty host. A target where the fixture cannot be
    // built skips with a note, and fails where
    // `MANDALA_REQUIRE_FONTCONFIG_HOSTS` is set, as CI sets it.
    //
    // Controls: with a script's `SCRIPT_FALLBACK` entry removed, red on
    // that script at the heavier weights, drawn in the planted bold face;
    // with the shadowing faces kept (no `remove_shadowing_faces`), red on
    // the planted host's shadows (and, with Inter's italic face planted
    // beside it, on the fixture's italic text drawn in it); with the
    // host's fonts loaded before the compiled ones, red on the layouts,
    // this machine's ids no longer the hosts'.
    if let Ok(what) = std::env::var(CHILD) {
        if what == HOSTS {
            for line in host_report() {
                println!("REPORT {line}");
            }
        }
        return;
    }
    let scratch = Scratch(std::env::temp_dir().join(format!("baumhard_font_hosts_{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&scratch.0);
    let scratch = &scratch.0;
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
    // A bold face of each compiled script under a name of its own, which
    // a bold span meets at its own weight in the last pass, ahead of the
    // compiled face's 400, unless the fallback lists the compiled face.
    for (k, from) in SCRIPT_FILES.iter().enumerate() {
        let data = std::fs::read(fonts.join(from)).unwrap_or_else(|e| panic!("{from}: {e}"));
        std::fs::write(planted.join(format!("HostBold{k}.ttf")), renamed_bold(&data))
            .expect("a planted face");
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
    assert_eq!(
        here.len(),
        MAPS.len() + 4,
        "precondition: this process reports every map"
    );
    assert_eq!(
        here[1], "SHADOWS []",
        "this process: host faces bearing a compiled name"
    );
    assert_eq!(
        here[2], "DEFAULT []",
        "this process: default text not in the default face"
    );
    assert_eq!(
        here[3], "SCRIPTS []",
        "this process: a compiled script drawn in another face"
    );
    let planted_faces = concat!(
        r#"HOST ["GalatiaSIL", "HostSansDevanagari-Regular", "HostSansEgyptianHieroglyphs-Regular", "#,
        r#""HostSansHebrew-Regular", "HostSerifTibetan-Regular", "InterVariable", "NotoSansHebrew-Regular"]"#
    );
    let indexed = (
        hosts[0].1.first().map(String::as_str),
        hosts[1].1.first().map(String::as_str),
    );
    if indexed != (Some("HOST []"), Some(planted_faces)) {
        // fontdb reads `FONTCONFIG_FILE` only on Linux with fontconfig:
        // elsewhere the hosts are the machine's own, no adversary. CI
        // runs on Linux and requires them.
        assert!(
            std::env::var_os(REQUIRE_HOSTS).is_none(),
            "precondition: each child's host font index holds what its fontconfig names, \
             or the hosts are no adversary on this target ({indexed:?})"
        );
        baumhard::util::test_skip::note_skipped(
            "font_state: this target does not read FONTCONFIG_FILE, so the planted hosts are unproven",
        );
        return;
    }
    for (host, report) in hosts {
        assert_eq!(
            report.len(),
            here.len(),
            "precondition: the {host} host reports every map"
        );
        assert_eq!(
            report[1], "SHADOWS []",
            "the {host} host: host faces bearing a compiled name"
        );
        assert_eq!(
            report[2], "DEFAULT []",
            "the {host} host: default text not in the default face"
        );
        assert_eq!(
            report[3], "SCRIPTS []",
            "the {host} host: a compiled script drawn in another face"
        );
        for (there, here) in report.iter().zip(&here).skip(4) {
            assert_eq!(
                there, here,
                "the {host} host lays a map out otherwise than this machine"
            );
        }
    }
}

/// Every glyph `text` projects to that is not drawn in a face its span
/// names or a listed fallback face, or is a `.notdef`, as `layer text
/// face glyph`, and how many glyphs were read. A span's face is read from
/// the attributes its buffer shaped it with: a family name names the
/// compiled faces bearing it; a generic family, the default face.
fn stray_glyphs(text: &str) -> (Vec<String>, usize) {
    use baumhard::font::fallback::{fallback_families, DEFAULT_FACE};
    use baumhard::font::fonts::COMPILED_FONT_ID_MAP;
    use cosmic_text::Family;
    let doc = load(text).unwrap_or_else(|error| panic!("precondition: the map loads: {error}"));
    let mut layers = GlyphLayers::new();
    let mut fonts = FontSystemLease::new("tests/font_state::glyphs");
    Projector::project_full(&doc, &mut layers, &mut fonts);
    let fs = fonts.fonts();
    let compiled: Vec<_> = COMPILED_FONT_ID_MAP
        .values()
        .flat_map(|ids| ids.iter().copied())
        .collect();
    let listed: Vec<_> = fallback_families()
        .flat_map(|(font, _)| COMPILED_FONT_ID_MAP[font].iter().copied())
        .collect();
    let bearing = |name: &str| -> Vec<cosmic_text::fontdb::ID> {
        compiled
            .iter()
            .copied()
            .filter(|id| {
                fs.db()
                    .face(*id)
                    .is_some_and(|face| face.families.iter().any(|(family, _)| family == name))
            })
            .collect()
    };
    let (mut faults, mut read) = (Vec::new(), 0);
    for layer in Layer::ALL {
        let tree = layers.tree(layer);
        for id in tree.root().descendants(tree.arena()) {
            let Some(run) = tree.arena()[id].get().glyph_run() else {
                continue;
            };
            let buffer = match run.shaped() {
                RunShape::Box(shaped) => shaped.buffer.clone(),
                RunShape::Path { run, .. } => run.buffer.clone(),
            };
            for line in buffer.layout_runs() {
                let attrs_list = buffer.lines[line.line_i].attrs_list();
                for glyph in line.glyphs {
                    read += 1;
                    let named = match attrs_list.get_span(glyph.start).family {
                        Family::Name(name) => bearing(name),
                        _ => COMPILED_FONT_ID_MAP[&DEFAULT_FACE.0].to_vec(),
                    };
                    if (!named.contains(&glyph.font_id) && !listed.contains(&glyph.font_id))
                        || glyph.glyph_id == 0
                    {
                        let face = fs.db().face(glyph.font_id).map(|f| f.post_script_name.clone());
                        faults.push(format!(
                            "{layer:?} {:?} {face:?} glyph {}",
                            line.text.get(glyph.start..glyph.end),
                            glyph.glyph_id
                        ));
                    }
                }
            }
        }
    }
    (faults, read)
}

#[test]
fn test_every_shipped_map_draws_each_glyph_in_a_face_its_span_names_or_the_fallback() {
    // #228: whatever the host has installed, a shipped map draws every
    // glyph in a face its span names or a listed fallback face, never in
    // an unlisted compiled face, a host face or as a `.notdef` — the
    // property that makes the shipped maps independent of the host, on
    // any host, where the host comparison above samples only this
    // machine's faces and the planted ones. Cosmic-text's last pass, the
    // only way to another face, can put a host emoji face, a host face of
    // the span's weight or, for an italic span, a host italic face ahead
    // of a compiled one (`format/fonts.md` "Fallback"), so no glyph may
    // reach it. Control: a `┌`, which only decorative compiled faces
    // hold, added to the default-family fixture's italic text, red on it.
    let maps = the_maps();
    let mut faults = Vec::new();
    for (name, text) in &maps {
        let (strays, read) = stray_glyphs(text);
        assert!(read > 20, "precondition: {name}'s glyphs are read ({read})");
        faults.extend(strays.into_iter().map(|stray| format!("{name}: {stray}")));
    }
    assert!(
        faults.is_empty(),
        "{} glyphs not in a face their span names or the fallback:\n{}",
        faults.len(),
        faults.join("\n")
    );
}
