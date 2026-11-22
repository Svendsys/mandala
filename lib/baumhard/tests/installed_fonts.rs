// SPDX-License-Identifier: MPL-2.0

//! Installed font families a map names (#329), in processes of their
//! own: what reading a map costs in host font scans, and what a family
//! the machine lacks is drawn in before it is installed, and after a
//! rescan finds it.
//!
//! Each test runs its body in a child process of this binary, so the
//! process-wide font state — the font system, built once; the count of
//! host scans — starts empty and nothing another test does reaches it.
//! The rescan test's child reads a fontconfig that names directories of
//! its own — a face that shadows a compiled family, a face that does
//! not, and one empty until the test plants a face in it — as #327's
//! `font_state` hosts do.

use std::path::{Path, PathBuf};

use baumhard::document::fonts::{installed_families, missing_families, STAND_IN};
use baumhard::document::format::{load, save};
use baumhard::font::fonts::{
    add_found, face_set_generation, host_scans, rescan_installed, rescan_installed_at_launch,
    scan_for_installed, FontSystemLease,
};
use baumhard::gfx_structs::run::RunShape;
use baumhard::projection::{GlyphLayers, Layer, Projector};
use cosmic_text::fontdb::ID;

/// Set in a child process: which body it runs.
const CHILD: &str = "BAUMHARD_INSTALLED_FONTS_CHILD";

/// Set in the rescan test's child: the directory its fontconfig names,
/// where the test plants the face it then rescans for.
const FONT_DIR: &str = "BAUMHARD_INSTALLED_FONTS_DIR";

/// Set where the rescan test's host must be built (CI's Linux runner), as
/// `font_state` reads it.
const REQUIRE_HOSTS: &str = "MANDALA_REQUIRE_FONTCONFIG_HOSTS";

/// The repository root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A one-node map whose spans are `spans`, each `(text, font, weight)`;
/// a `None` font names none.
fn map(spans: &[(&str, Option<&str>, u16)]) -> String {
    let spans: Vec<String> = spans
        .iter()
        .map(|(text, font, weight)| {
            let font = font.map_or(String::new(), |font| {
                format!(r#","font":{}"#, serde_json::json!(font))
            });
            format!(
                r#"{{"text":{}{font},"weight":{weight}}}"#,
                serde_json::json!(text)
            )
        })
        .collect();
    format!(
        r#"{{"format":1,"name":"fonts","nodes":[{{"id":1,"position":[0.0,0.0],"sections":[{{"spans":[{}]}}]}}]}}"#,
        spans.join(",")
    )
}

/// Run this binary again for the test `test`, with the child variable set
/// to `what` and `envs` beside it; its standard output's `REPORT` lines.
/// The child must succeed.
fn in_a_child(test: &str, what: &str, envs: &[(&str, &Path)]) -> Vec<String> {
    let exe = std::env::current_exe().expect("the test binary");
    let mut command = std::process::Command::new(&exe);
    command
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD, what);
    for (key, value) in envs {
        command.env(key, value);
    }
    let out = command.output().expect("the test binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{what}: {stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
        .lines()
        .filter_map(|line| line.split_once("REPORT ").map(|(_, report)| report.to_string()))
        .collect()
}

#[test]
fn test_reading_a_map_scans_no_host_font_and_asking_what_is_missing_scans_once() {
    // #329, the finding routed from #353: loading a map that named a font
    // built the whole font system, the host's font scan included, though
    // a load can be judged on the compiled faces' names alone. Now the
    // loader, the saver and the name checks read only those, so reading
    // and writing maps — naming compiled families, installed ones, or
    // none — scans nothing; asking which installed families this machine
    // lacks scans once, and only for a map that names one.
    //
    // Controls: `format::font` resolving names through the loaded-family
    // index (`loaded_families`) instead of the compiled one turns the
    // first count red at 1; `missing_families` asking the font system for
    // a map that names only compiled families turns the second red; a
    // second scan per question (no cached index) turns the last red at 2.
    if std::env::var(CHILD).as_deref() == Ok("scan") {
        let compiled = map(&[("Lord", Some("Public Pixel"), 700), ("God", None, 400)]);
        let installed = map(&[
            ("Lord", Some("Nope Sans"), 400),
            ("God", Some("Galatia SIL"), 400),
        ]);
        for text in [&compiled, &installed] {
            let doc = load(text).expect("the map loads");
            let saved = save(&doc).expect("the map saves");
            let again = load(&saved).expect("the saved map loads");
            assert_eq!(installed_families(&doc), installed_families(&again));
        }
        let doc = load(&compiled).expect("the map loads");
        println!("REPORT read {}", host_scans());
        assert!(missing_families(&doc).is_empty());
        println!("REPORT compiled-only {}", host_scans());
        let doc = load(&installed).expect("the map loads");
        let missing: Vec<String> = missing_families(&doc)
            .iter()
            .map(|missing| missing.family.to_string())
            .collect();
        println!("REPORT missing {missing:?}");
        println!("REPORT asked {}", host_scans());
        let _ = missing_families(&doc);
        println!("REPORT asked again {}", host_scans());
        return;
    }
    let report = in_a_child(
        "test_reading_a_map_scans_no_host_font_and_asking_what_is_missing_scans_once",
        "scan",
        &[],
    );
    assert_eq!(
        report,
        [
            "read 0",
            "compiled-only 0",
            r#"missing ["Nope Sans"]"#,
            "asked 1",
            "asked again 1",
        ],
        "host font scans, in a fresh process"
    );
}

/// A fontconfig file that lists `dirs` as the host's font directories,
/// in that order.
fn fontconfig(dirs: &[&Path]) -> String {
    let dirs: String = dirs
        .iter()
        .map(|dir| format!("<dir>{}</dir>", dir.display()))
        .collect();
    format!(
        "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n<fontconfig>{dirs}</fontconfig>\n"
    )
}

/// A copy of the compiled face `data` a host could install as a family of
/// its own: every `Noto` in its `name` table spelled `Host`, so
/// Noto Sans Hebrew becomes the installed family `Host Sans Hebrew`,
/// which no compiled face bears. The outlines are the compiled face's.
/// Byte for byte the same every run; the table checksums are left stale,
/// which nothing that loads a face checks.
fn renamed(data: &[u8]) -> Vec<u8> {
    let u16_at = |at: usize| u16::from_be_bytes([data[at], data[at + 1]]) as usize;
    let u32_at =
        |at: usize| u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize;
    let mut out = data.to_vec();
    let name = (0..u16_at(4))
        .map(|i| 12 + 16 * i)
        .find(|&record| &data[record..record + 4] == b"name")
        .map(|record| u32_at(record + 8)..u32_at(record + 8) + u32_at(record + 12))
        .expect("precondition: the face has a name table");
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
    out
}

/// `data`, a copy of the compiled Noto Sans Hebrew, with the character
/// map's Hebrew-letter segment (U+05D0–U+05EA) moved up one code point,
/// in every format 4 subtable: so the face also maps U+05EB
/// ([`UNASSIGNED`]), which Unicode leaves unassigned and no compiled face
/// maps — a cluster only this face can draw, which is what a measurement
/// cached before it was installed would get wrong. The segment's glyphs
/// are found through its range offset, so each code point maps to its
/// neighbor's glyph, and the next segment starts at U+05F0, so the
/// segments stay sorted.
fn mapping_unassigned(data: &[u8]) -> Vec<u8> {
    let u16_at = |data: &[u8], at: usize| u16::from_be_bytes([data[at], data[at + 1]]) as usize;
    let u32_at =
        |at: usize| u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize;
    let mut out = data.to_vec();
    let cmap = (0..u16_at(data, 4))
        .map(|i| 12 + 16 * i)
        .find(|&record| &data[record..record + 4] == b"cmap")
        .map(|record| u32_at(record + 8))
        .expect("precondition: the face has a character map");
    let mut moved = 0;
    for k in 0..u16_at(data, cmap + 2) {
        let table = cmap + u32_at(cmap + 4 + 8 * k + 4);
        if u16_at(data, table) != 4 {
            continue;
        }
        let segments = u16_at(data, table + 6) / 2;
        let (ends, starts) = (table + 14, table + 16 + 2 * segments);
        for i in 0..segments {
            if u16_at(data, starts + 2 * i) == 0x05D0 && u16_at(data, ends + 2 * i) == 0x05EA {
                out[starts + 2 * i..starts + 2 * i + 2].copy_from_slice(&0x05D1u16.to_be_bytes());
                out[ends + 2 * i..ends + 2 * i + 2].copy_from_slice(&0x05EBu16.to_be_bytes());
                moved += 1;
            }
        }
    }
    assert!(
        moved > 0,
        "precondition: the face maps the Hebrew letters in one segment"
    );
    out
}

/// A code point Unicode leaves unassigned, inside the Hebrew block: no
/// compiled face maps it, and the planted face does
/// ([`mapping_unassigned`]).
const UNASSIGNED: &str = "\u{05EB}";

/// The installed family the rescan test plants.
const PLANTED: &str = "Host Sans Hebrew";

/// Every glyph `text` projects to, as `text face weight`: the face by
/// PostScript name, the weight the shaper drew it at.
fn drawn(text: &str) -> Vec<String> {
    let doc = load(text).expect("the map loads");
    let mut layers = GlyphLayers::new();
    let mut fonts = FontSystemLease::new("tests/installed_fonts");
    Projector::project_full(&doc, &mut layers, &mut fonts);
    glyphs(&layers, &mut fonts)
}

/// Every glyph `layers` hold, as [`drawn`] lists them.
fn glyphs(layers: &GlyphLayers, fonts: &mut FontSystemLease) -> Vec<String> {
    let fs = fonts.fonts();
    let mut out = Vec::new();
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
                    let face = fs
                        .db()
                        .face(glyph.font_id)
                        .map(|face| face.post_script_name.clone());
                    out.push(format!(
                        "{} {} {}",
                        line.text.get(glyph.start..glyph.end).unwrap_or("?"),
                        face.unwrap_or_default(),
                        glyph.font_weight.0
                    ));
                }
            }
        }
    }
    out
}

/// Every face the font system holds, as its id and `id name`, in the
/// database's order — the order it iterates in.
fn faces() -> Vec<(ID, String)> {
    let mut fonts = FontSystemLease::new("tests/installed_fonts::faces");
    fonts
        .fonts()
        .db()
        .faces()
        .map(|face| (face.id, format!("{:?} {}", face.id, face.post_script_name)))
        .collect()
}

#[test]
fn test_a_missing_family_draws_in_the_stand_in_until_installed_and_rescanned() {
    // #329: a map naming a family this machine lacks loads, reports it
    // missing, and draws its text in the stand-in, the default face, at
    // the span's weight. Once the family is installed, a rescan adds its
    // faces after every face the font system holds — renumbering none,
    // every added id greater than every earlier one, iterating last —
    // and the map, projected again, draws in it — and so does a projection
    // kept from before, updated for the found family alone
    // (`Projector::update_faces`). A rescan that finds
    // nothing new changes nothing: the faces and the face-set generation
    // are exactly as they were. Two rescans that overlap — both looking
    // at the host before either adds — add the face once and move the
    // generation once.
    //
    // The child's fontconfig names three directories, in order: the
    // compiled Noto Sans Hebrew unrenamed, which shadows a compiled family
    // and so is kept out when the font system is built; a renamed copy of
    // Noto Sans Devanagari (`Host Sans Devanagari`), a host face that
    // joins it; and one empty until the child plants a renamed copy of
    // Noto Sans Hebrew in it (`Host Sans Hebrew`). The first is the
    // adversary of "after every face": fontdb's slot map gives a freed
    // slot to the next face pushed, so a face removed at the build would
    // hand the rescanned face an id among the host's (the review of #390
    // reproduced it). The host is an adversary only where fontdb reads
    // `FONTCONFIG_FILE` (Linux), so a target where the host index is not
    // what the fontconfig names skips with a note, and fails where
    // `MANDALA_REQUIRE_FONTCONFIG_HOSTS` is set, as CI sets it.
    //
    // Controls: `rescan_installed` adding no face turns the "after
    // install" arm red (still missing, still the stand-in);
    // `rescan_installed` bumping the generation unconditionally turns the
    // "nothing new" arms red; `font_pin` ignoring an installed family
    // turns the drawing after the rescan red (Noto Sans Hebrew, the
    // compiled fallback); `shadows_a_compiled_family` answering `true`
    // for every face turns the found list red; the build pushing every
    // host face and removing the shadowing ones after turns the order arm
    // red (`order false false`); the metric caches keyed without the
    // face-set generation turn the measured arm red (`true false true`,
    // the stale `.notdef` advance); `forget_shapings` dropping nothing turns
    // the kept projection's arm red (its Hebrew still the compiled
    // fallback's); `add_found` not checking again under the
    // write lock turns the overlap arm red (the face added again, the
    // generation moved again).
    if std::env::var(CHILD).as_deref() == Ok("rescan") {
        let dir = PathBuf::from(std::env::var_os(FONT_DIR).expect("the font directory"));
        let mut host = cosmic_text::fontdb::Database::new();
        host.load_system_fonts();
        let host_names: Vec<String> = host.faces().map(|face| face.post_script_name.clone()).collect();
        println!("REPORT host {host_names:?}");
        let text = map(&[("Shalom ", Some(PLANTED), 700), ("שלום", Some(PLANTED), 400)]);
        let doc = load(&text).expect("the map loads");
        let names = |doc: &baumhard::document::ids::Document| -> Vec<String> {
            missing_families(doc)
                .iter()
                .map(|missing| missing.family.to_string())
                .collect()
        };
        println!("REPORT missing {:?}", names(&doc));
        println!("REPORT before {:?}", drawn(&text));
        // A projection kept across the rescan, as an open map's view is.
        let mut kept_layers = GlyphLayers::new();
        let mut kept = {
            let mut fonts = FontSystemLease::new("tests/installed_fonts::kept");
            Projector::project_full(&doc, &mut kept_layers, &mut fonts)
        };
        let (faces_before, generation_before) = (faces(), face_set_generation());
        println!(
            "REPORT held {:?}",
            faces_before
                .iter()
                .filter(|(_, face)| face.contains(" HostSans"))
                .map(|(_, face)| face.split_once(' ').map_or(face.as_str(), |(_, name)| name))
                .collect::<Vec<_>>()
        );
        println!("REPORT rescan-1 {:?}", rescan_installed([PLANTED]));
        println!(
            "REPORT unchanged-1 {} {}",
            faces() == faces_before,
            face_set_generation() == generation_before
        );
        let hebrew =
            root().join("lib/baumhard/src/font/fonts/noto-sans-hebrew-font/NotoSansHebrew-Regular.ttf");
        let data = std::fs::read(&hebrew).expect("the compiled Hebrew face");
        // Measured before the family is installed: a cluster no face maps.
        let measure = || {
            (
                baumhard::font::metric_cache::glyph_advance(None, 16.0, UNASSIGNED),
                baumhard::font::metric_cache::glyph_ink(None, 16.0, UNASSIGNED).advance,
            )
        };
        let unmapped = measure();
        std::fs::write(
            dir.join("HostSansHebrew.ttf"),
            renamed(&mapping_unassigned(&data)),
        )
        .expect("a planted face");
        // Two rescans that overlap: both look at the host before either
        // adds what it found.
        let (first, second) = (scan_for_installed([PLANTED]), scan_for_installed([PLANTED]));
        let (first, second) = (
            first.expect("the planted face is found"),
            second.expect("found twice"),
        );
        println!("REPORT rescan-2 {:?}", add_found(first));
        // Measured again once it is: the planted face maps it to the glyph
        // of tav, whose advance the compiled Hebrew face measures.
        let tav = baumhard::font::metric_cache::glyph_advance(None, 16.0, "\u{05EA}");
        println!(
            "REPORT measured {} {} {}",
            unmapped != (tav, tav),
            measure() == (tav, tav),
            unmapped.0 != tav
        );
        let after = faces();
        let added: Vec<&(ID, String)> = after.iter().filter(|face| !faces_before.contains(face)).collect();
        println!(
            "REPORT kept {} added {:?} generation {}",
            faces_before.iter().all(|face| after.contains(face)),
            added
                .iter()
                .map(|(_, face)| face.split_once(' ').map_or(face.as_str(), |(_, name)| name))
                .collect::<Vec<_>>(),
            face_set_generation() - generation_before
        );
        println!(
            "REPORT order {} {}",
            added
                .iter()
                .all(|(id, _)| faces_before.iter().all(|(before, _)| id > before)),
            after.last().is_some_and(|last| added.contains(&last))
        );
        // The kept projection, told the family's faces came: what the view
        // of an open map does after a rescan that found it.
        {
            let mut fonts = FontSystemLease::new("tests/installed_fonts::update_faces");
            let _ = kept.update_faces(&doc, &[], &[PLANTED], &mut kept_layers, &mut fonts);
            println!("REPORT updated {:?}", glyphs(&kept_layers, &mut fonts));
        }
        let (faces_first, generation_first) = (faces(), face_set_generation());
        println!(
            "REPORT overlap {:?} {} {}",
            add_found(second),
            faces() == faces_first,
            face_set_generation() == generation_first
        );
        let (faces_found, generation_found) = (faces(), face_set_generation());
        println!("REPORT rescan-3 {:?}", rescan_installed([PLANTED]));
        println!(
            "REPORT unchanged-3 {} {}",
            faces() == faces_found,
            face_set_generation() == generation_found
        );
        let doc = load(&text).expect("the map loads again");
        println!("REPORT missing-after {:?}", names(&doc));
        println!("REPORT after {:?}", drawn(&text));
        return;
    }
    let scratch = std::env::temp_dir().join(format!("baumhard_installed_fonts_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let (shadowing, joining, fonts, home) = (
        scratch.join("shadowing"),
        scratch.join("joining"),
        scratch.join("fonts"),
        scratch.join("home"),
    );
    for dir in [&shadowing, &joining, &fonts, &home] {
        std::fs::create_dir_all(dir).expect("a scratch directory");
    }
    let compiled = root().join("lib/baumhard/src/font/fonts");
    std::fs::copy(
        compiled.join("noto-sans-hebrew-font/NotoSansHebrew-Regular.ttf"),
        shadowing.join("NotoSansHebrew-Regular.ttf"),
    )
    .expect("a shadowing face");
    let devanagari = std::fs::read(compiled.join("noto-sans-devanagari-font/NotoSansDevanagari-Regular.ttf"))
        .expect("the compiled Devanagari face");
    std::fs::write(joining.join("HostSansDevanagari.ttf"), renamed(&devanagari)).expect("a joining face");
    let conf = scratch.join("fonts.conf");
    std::fs::write(&conf, fontconfig(&[&shadowing, &joining, &fonts])).expect("a fontconfig file");
    let report = in_a_child(
        "test_a_missing_family_draws_in_the_stand_in_until_installed_and_rescanned",
        "rescan",
        &[
            ("FONTCONFIG_FILE", &conf),
            (FONT_DIR, &fonts),
            ("HOME", &home),
            ("XDG_CONFIG_HOME", &home.join(".config")),
            ("XDG_DATA_HOME", &home.join(".local/share")),
        ],
    );
    let _ = std::fs::remove_dir_all(&scratch);
    if report.first().map(String::as_str)
        != Some(r#"host ["NotoSansHebrew-Regular", "HostSansDevanagari-Regular"]"#)
    {
        assert!(
            std::env::var_os(REQUIRE_HOSTS).is_none(),
            "precondition: the child's host font index holds what its fontconfig names ({:?})",
            report.first()
        );
        baumhard::util::test_skip::note_skipped(
            "installed_fonts: this target does not read FONTCONFIG_FILE, so the rescan is unproven",
        );
        return;
    }
    let line = |prefix: &str| -> &str {
        report
            .iter()
            .find_map(|line| line.strip_prefix(prefix))
            .unwrap_or_else(|| panic!("the child reports {prefix:?}: {report:?}"))
    };
    let stand_in = "InterVariable";
    assert_eq!(
        STAND_IN, "Inter Variable",
        "precondition: the stand-in is the default face"
    );
    assert_eq!(
        line("missing "),
        r#"["Host Sans Hebrew"]"#,
        "before it is installed"
    );
    let before = parse_list(line("before "));
    let latin: Vec<&String> = before
        .iter()
        .filter(|glyph| glyph.starts_with(|c: char| c.is_ascii_alphabetic()))
        .collect();
    assert_eq!(
        latin.len(),
        6,
        "precondition: every Latin glyph is read ({before:?})"
    );
    for glyph in &latin {
        assert!(
            glyph.ends_with(&format!(" {stand_in} 700")),
            "a missing family's Latin text is the stand-in's, at its span's weight: {glyph}"
        );
    }
    assert!(
        before
            .iter()
            .any(|glyph| glyph.contains(" NotoSansHebrew-Regular ")),
        "the Hebrew is the compiled fallback's while the family is missing: {before:?}"
    );
    assert_eq!(
        line("held "),
        r#"["HostSansDevanagari-Regular"]"#,
        "the host face that shadows no compiled family joins the font system"
    );
    assert_eq!(line("rescan-1 "), "[]");
    assert_eq!(
        line("unchanged-1 "),
        "true true",
        "a rescan that finds nothing changes nothing"
    );
    assert_eq!(line("rescan-2 "), r#"["Host Sans Hebrew"]"#);
    assert_eq!(
        line("kept "),
        r#"true added ["HostSansHebrew-Regular"] generation 1"#,
        "the rescan adds the planted face, renumbering none, and moves the generation once"
    );
    assert_eq!(
        line("order "),
        "true true",
        "the added face's id is greater than every earlier face's, and it iterates last"
    );
    assert_eq!(
        line("measured "),
        "true true true",
        "a cluster measured before the rescan is measured again after it, in the face it added"
    );
    assert_eq!(
        line("overlap "),
        "[] true true",
        "an overlapping rescan adds nothing the first added, and moves no generation"
    );
    assert_eq!(line("rescan-3 "), "[]");
    assert_eq!(
        line("unchanged-3 "),
        "true true",
        "a second rescan changes nothing"
    );
    assert_eq!(
        line("missing-after "),
        "[]",
        "the installed family is no longer missing"
    );
    let after = parse_list(line("after "));
    assert_eq!(
        parse_list(line("updated ")),
        after,
        "the projection kept across the rescan, updated for the found family, draws what a full projection does"
    );
    let hebrew: Vec<&String> = after
        .iter()
        .filter(|glyph| !glyph.starts_with(|c: char| c.is_ascii()))
        .collect();
    assert_eq!(
        hebrew.len(),
        4,
        "precondition: every Hebrew glyph is read ({after:?})"
    );
    for glyph in &hebrew {
        assert!(
            glyph.contains(" HostSansHebrew-Regular "),
            "the installed family draws its text once found: {glyph}"
        );
    }
}

#[test]
fn test_the_first_open_at_launch_rescans_nothing_the_build_just_scanned() {
    // #329, the review of #390: at launch the font system is built with a
    // scan of the host, and the first map's missing families would be
    // looked up again milliseconds later — two scans of every installed
    // font where one suffices. `rescan_installed_at_launch` skips the
    // second while the build's scan is the latest; once anything has
    // scanned since, it rescans, as every later open does.
    //
    // Control: `rescan_installed_at_launch` calling `rescan_installed`
    // unconditionally turns the first two counts red (2 and 3).
    if std::env::var(CHILD).as_deref() == Ok("launch") {
        let text = map(&[("Lord", Some("Nope Sans"), 400)]);
        let doc = load(&text).expect("the map loads");
        let missing: Vec<String> = missing_families(&doc)
            .iter()
            .map(|missing| missing.family.to_string())
            .collect();
        println!("REPORT built {}", host_scans());
        let names = || missing.iter().map(String::as_str);
        println!(
            "REPORT at-launch {:?} {}",
            rescan_installed_at_launch(names()),
            host_scans()
        );
        println!("REPORT at-launch-again {}", {
            let _ = rescan_installed_at_launch(names());
            host_scans()
        });
        println!("REPORT rescan {:?} {}", rescan_installed(names()), host_scans());
        println!("REPORT at-launch-after {}", {
            let _ = rescan_installed_at_launch(names());
            host_scans()
        });
        return;
    }
    let report = in_a_child(
        "test_the_first_open_at_launch_rescans_nothing_the_build_just_scanned",
        "launch",
        &[],
    );
    assert_eq!(
        report,
        [
            "built 1",
            "at-launch [] 1",
            "at-launch-again 1",
            "rescan [] 2",
            "at-launch-after 3",
        ],
        "host font scans, in a fresh process"
    );
}

/// A list a child printed with `{:?}`, read back.
fn parse_list(text: &str) -> Vec<String> {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("a list: {error}: {text}"))
}
