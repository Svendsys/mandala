// SPDX-License-Identifier: MPL-2.0

//! The work bound counts the shaper's `.notdef` passes (#387): CI's
//! load_structured finding on #380, a frame of a tab beside code points no
//! face draws, replayed on the compiled faces alone, where it lays past the
//! bound as it stood; each other shape of the same blind spot beside it;
//! and the two facts the count rests on, which faces may lay a `.notdef`
//! for a text and which code points the shaper hides.
//!
//! Each check runs in a child process whose font system holds the compiled
//! faces alone (`FONTCONFIG_FILE` naming a config of one empty directory),
//! so it fails alike on every host: a host face that draws what the
//! compiled ones do not — Unifont maps U+0001 and U+20E3 — would shape the
//! finding with no `.notdef` at all. A target whose fontdb does not read
//! `FONTCONFIG_FILE` cannot make such a child, and skips with a note; CI
//! runs on Linux and sets `MANDALA_REQUIRE_FONTCONFIG_HOSTS`, under which
//! it fails instead.

use std::process::Command;

use serde_json::json;

use crate::document::element::DocElement;
use crate::document::format::load;
use crate::document::ids::Document;
use crate::projection::tests::projection_fuzz::{
    check_projects, face_metrics, may_lay_notdef, ring_notdef_steps, ring_steps, within_bound, SHAPER_HIDDEN,
};
use crate::projection::tests::support::Projected;

/// The variable a child reads: which check it makes.
const CHILD: &str = "BAUMHARD_NOTDEF_BOUND_CHILD";

/// CI's finding (job 136906833586 on #380): the map its input made, and the
/// input itself, the load_structured target's choices.
const FINDING: &[u8] =
    include_bytes!("../../document/tests/fuzz_findings/a_tabbed_ring_of_notdefs.mandala.json");
const FINDING_CHOICES: &[u8] =
    include_bytes!("../../document/tests/fuzz_findings/structured_a_tabbed_ring_of_notdefs");

/// The lines a child of `test` (its path under the crate) reported in mode
/// `mode`, from a font system of the compiled faces alone; `None`, with a
/// note, where this target cannot make one. The child's own first report
/// is how many host faces its font system holds, which must be none.
fn on_compiled_faces(test: &str, mode: &str) -> Option<Vec<String>> {
    let scratch = std::env::temp_dir().join(format!("baumhard_notdef_bound_{}_{mode}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let (empty, home) = (scratch.join("empty"), scratch.join("home"));
    for dir in [&empty, &home] {
        std::fs::create_dir_all(dir).expect("a scratch directory");
    }
    let conf = scratch.join("fonts.conf");
    std::fs::write(
        &conf,
        format!(
            "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n<fontconfig><dir>{}</dir></fontconfig>\n",
            empty.display()
        ),
    )
    .expect("a fontconfig file");
    let out = Command::new(std::env::current_exe().expect("the test binary has a path"))
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD, mode)
        .env("FONTCONFIG_FILE", &conf)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .output()
        .expect("the test binary runs");
    let _ = std::fs::remove_dir_all(&scratch);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{mode}: {text}");
    // libtest prints the test's name on the line a report starts, so a
    // report is found within a line.
    let reports: Vec<String> = text
        .lines()
        .filter_map(|line| line.split_once("REPORT ").map(|(_, report)| report.to_string()))
        .collect();
    match reports.first().map(String::as_str) {
        Some("HOSTS 0") => Some(reports[1..].to_vec()),
        hosts => {
            assert!(
                std::env::var_os("MANDALA_REQUIRE_FONTCONFIG_HOSTS").is_none(),
                "precondition: the child's font system holds the compiled faces alone ({hosts:?})"
            );
            crate::util::test_skip::note_skipped(&format!(
                "{test}: this target does not read FONTCONFIG_FILE, so no child holds the compiled faces alone"
            ));
            None
        }
    }
}

/// In a child: report how many host faces its font system holds, the
/// first report every child makes. Whether a face is the host's is read
/// from its source, as `test_flow_shape_a_glyph_no_face_draws_has_no_ink`
/// reads it.
fn report_hosts() {
    crate::font::fonts::init();
    let mut lease = crate::font::fonts::FontSystemLease::new("the notdef bound's child");
    let hosts = lease
        .fonts()
        .db()
        .faces()
        .filter(|face| !matches!(face.source, cosmic_text::fontdb::Source::Binary(_)))
        .count();
    println!("REPORT HOSTS {hosts}");
}

/// The rings of the bound's blind spot (#387), each on a node of its own,
/// laid on the compiled faces: CI's map, and a ring of each other shape a
/// `.notdef` pass took past the bound as it stood in — a tab beside code
/// points no face draws, a `fit` ring with a tab, a side ornament and a
/// `once` filler beside such text, a cluster each of whose characters some
/// face maps and no one face maps together (an Egyptian hieroglyph, which
/// only its face maps, and U+0301, which that face does not), and a
/// `Repeat` ring that lays text by direction, whose one shaping a lay
/// makes holds such text in its spans or in its corners, round a node 10
/// across.
fn blind_spot_maps() -> Vec<(&'static str, String)> {
    let ring = |ring: serde_json::Value| {
        json!({"format": 1, "name": "l", "nodes": [{"id": 1, "position": [0, 0], "frame": {"rings": [ring]}}]})
            .to_string()
    };
    let small = |ring: serde_json::Value| {
        json!({"format": 1, "name": "l", "nodes": [{"id": 1, "position": [0, 0], "min_size": [10.0, 10.0], "frame": {"rings": [ring]}}]})
            .to_string()
    };
    vec![
        (
            "CI's finding",
            String::from_utf8(FINDING.to_vec()).expect("the finding is text"),
        ),
        (
            "a tab beside six code points no face draws",
            ring(json!({"spans": [{"text": "\t\u{1}\u{2}\u{3}\u{4}\u{5}\u{6}"}]})),
        ),
        (
            "`fit`, a tab and a keycap",
            ring(json!({"fit": "fit", "spans": [{"text": "\tab\u{20e3}"}]})),
        ),
        (
            "a side ornament beside text no face draws",
            ring(json!({"sides": "*", "spans": [{"text": "a\u{1}\u{20e3}b"}]})),
        ),
        (
            "a `once` filler beside text no face draws",
            ring(json!({"fit": {"once": {"filler": "*"}}, "spans": [{"text": "a\u{1}\u{20e3}b\u{2}\u{3}"}]})),
        ),
        (
            "a tab beside a cluster no one face draws",
            ring(json!({"fit": "fit", "spans": [{"text": "\t\u{13000}\u{301}abcdefgh"}]})),
        ),
        (
            "a directional ring of spans no face draws",
            small(json!({
                "fit": "repeat",
                "spacing": 8.0,
                "spans": [{"text": "\u{1}\u{20e3}\u{2}\u{3}\u{4}\u{5}\u{6}\u{7}\u{e}\u{f}\u{10}\u{11}"}],
                "vertical": "|",
            })),
        ),
        (
            "a directional ring of corners no face draws",
            small(json!({
                "fit": "repeat",
                "spacing": 8.0,
                "spans": [{"text": "-"}],
                "vertical": "|",
                "corners": ["\u{1}\u{20e3}", "\u{2}", "\u{3}", "\u{4}"],
            })),
        ),
    ]
}

/// A `Repeat` ring of spans alone of code points no face draws, round a
/// node 10 across: what a lay of a ring of no `vertical` text and no
/// corners does with text the shaper lays as `.notdef`.
fn plain_repeat_map() -> String {
    json!({"format": 1, "name": "l", "nodes": [{"id": 1, "position": [0, 0], "min_size": [10.0, 10.0], "frame": {"rings": [{
        "fit": "repeat",
        "spacing": 8.0,
        "spans": [{"text": "\u{1}\u{20e3}\u{2}\u{3}"}],
    }]}}]})
    .to_string()
}

/// `doc`'s one ring: its frame and itself.
fn the_ring(doc: &Document) -> (crate::arena::NodeId, crate::arena::NodeId) {
    let tree = doc.tree();
    let ring = tree
        .descendants()
        .find(|&id| matches!(tree.arena()[id].get(), DocElement::Ring(_)))
        .expect("a ring");
    (ring.parent(tree.arena()).expect("a frame"), ring)
}

/// **The bound counts the shaper's `.notdef` passes** (#387). CI's
/// load_structured job on #380 (job 136906833586) found a map the bound
/// admitted whose one ring, `\n\r\t\u{1}` and U+FE0F U+20E3, took 14,157
/// glyph steps a lay where the bound allowed 9,344: no compiled face draws
/// U+0001 or U+20E3, the shaper joins them and the variation selector into
/// one cluster, a `.notdef` of three code points, and a frame whose text
/// holds a tab shapes it again at each reshape, a pass through every face
/// a glyph of its line, 2,200 steps each (`halves::notdef_steps`), where
/// the bound counted 192 a character. Each ring of the blind spot
/// ([`blind_spot_maps`]), laid on the compiled faces alone, takes more
/// steps than the bound read for it less its `.notdef` count — so the pass
/// is laid, and the bound as it stood is short — and no more than the
/// bound reads for it ([`ring_steps`]). The finding is outside the bound
/// now, and its input's check (`check_structured`) and its map's
/// (`check_projects`) hold, there and on this host's faces. And a `Repeat`
/// ring of spans alone, of the same code points, shapes nothing as it lays
/// ([`plain_repeat_map`]): the bound counts it no pass, and it lays fewer
/// steps than one pass costs, inside its `RING_STEPS_A_GLYPH` a glyph —
/// the rate a `.notdef` pass could break where a lay shaped, and does not
/// here.
///
/// Controls, each run and seen red: the bound as it stood, no `.notdef`
/// count (`notdef_steps_of` nothing), every ring past its bound and the
/// finding's check failing as CI's did ("a ring's lay took 14157 glyph
/// steps, past the 9344 the bound allows its map"); with no count for a
/// `fit` frame's tabbed reshapes, CI's finding, the six code points, the
/// `fit` keycap and the cluster no one face draws; with none for a side
/// ornament's shapings, the ornament's ring, and with none for a `once`
/// filler's own shaping either, the filler's ring too; with none for a
/// directional ring's shaping, both directional rings; with "may lay a
/// `.notdef`" read as "holds a character no face maps", the cluster no one
/// face draws. A filler's own shaping, made only where a frame's halves
/// cannot be measured, alone (the filler's ring is held by its sides'
/// count without it), and the two a character — a `.notdef` of one code
/// point and a glyph of a line that holds one of several, each at most one
/// a character — are margins no ring here comes near, and have no red
/// control.
#[test]
fn test_the_bound_counts_the_shapers_notdef_passes_on_the_compiled_faces() {
    let name = "projection::tests::notdef_bound_tests::test_the_bound_counts_the_shapers_notdef_passes_on_the_compiled_faces";
    if std::env::var(CHILD).as_deref() == Ok("rings") {
        report_hosts();
        for (what, map) in blind_spot_maps() {
            let doc = load(&map).unwrap_or_else(|why| panic!("{what}: does not load: {why}"));
            let (frame, ring) = the_ring(&doc);
            let allowed = ring_steps(doc.tree(), frame, ring).expect("a ring of text");
            let notdef = ring_notdef_steps(doc.tree(), ring);
            crate::flow::MOST_RING_LAY.with(|most| most.set(0));
            drop(Projected::new(doc.clone()));
            let laid = crate::flow::MOST_RING_LAY.with(std::cell::Cell::get);
            println!(
                "REPORT RING {what}: laid {laid} allowed {allowed} without its notdef count {} inside {}",
                allowed - notdef,
                within_bound(&doc)
            );
        }
        let doc = load(&plain_repeat_map()).expect("the plain ring loads");
        let (frame, ring) = the_ring(&doc);
        let allowed = ring_steps(doc.tree(), frame, ring).expect("a ring of text");
        let notdef = ring_notdef_steps(doc.tree(), ring);
        crate::flow::MOST_RING_LAY.with(|most| most.set(0));
        drop(Projected::new(doc.clone()));
        let laid = crate::flow::MOST_RING_LAY.with(std::cell::Cell::get);
        println!(
            "REPORT PLAIN laid {laid} allowed {allowed} notdef {notdef} inside {}",
            within_bound(&doc)
        );
        let doc = load(std::str::from_utf8(FINDING).unwrap()).expect("the finding loads");
        println!(
            "REPORT CHECKS structured {:?} projects {:?}",
            crate::document::tests::format_fuzz::check_structured(FINDING_CHOICES),
            check_projects(&doc)
        );
        return;
    }
    let Some(reports) = on_compiled_faces(name, "rings") else {
        return;
    };
    let mut wrong = Vec::new();
    let mut rings = 0;
    for report in &reports {
        eprintln!("{report}");
        let Some(ring) = report.strip_prefix("RING ") else {
            continue;
        };
        rings += 1;
        let number = |key: &str| -> usize {
            ring.split(key)
                .nth(1)
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|n| n.parse().ok())
                .unwrap_or_else(|| panic!("{ring}: no {key}"))
        };
        let (laid, allowed, without) = (
            number(" laid "),
            number(" allowed "),
            number(" without its notdef count "),
        );
        if laid <= without {
            wrong.push(format!(
                "precondition, the pass laid past the bound as it stood: {ring}"
            ));
        }
        if laid > allowed {
            wrong.push(format!("laid past its bound: {ring}"));
        }
        if ring.starts_with("CI's finding") && !ring.ends_with("inside false") {
            wrong.push(format!("the finding is inside the bound: {ring}"));
        }
    }
    assert_eq!(
        rings,
        blind_spot_maps().len(),
        "precondition: every ring reported"
    );
    assert!(wrong.is_empty(), "{wrong:#?}");
    let plain = reports
        .iter()
        .find_map(|r| r.strip_prefix("PLAIN "))
        .unwrap_or_else(|| panic!("the plain ring reported: {reports:?}"));
    let field = |key: &str| -> usize {
        plain
            .split(key)
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("{plain}: no {key}"))
    };
    let (laid, allowed, notdef) = (field("laid "), field(" allowed "), field(" notdef "));
    assert!(
        plain.ends_with("inside true") && notdef == 0 && laid < crate::flow::SHAPE_NOTDEF_STEPS && laid <= allowed,
        "a plain `Repeat` ring of code points no face draws, inside the bound, counted no pass and paid none: {plain}"
    );
    assert!(
        reports
            .iter()
            .any(|r| r == "CHECKS structured Ok(()) projects Ok(())"),
        "the finding's checks on the compiled faces: {reports:?}"
    );
    // And on this host's faces, whatever it has.
    let doc = load(std::str::from_utf8(FINDING).unwrap()).expect("the finding loads");
    crate::document::tests::format_fuzz::check_structured(FINDING_CHOICES)
        .unwrap_or_else(|why| panic!("the finding's input on this host: {why}"));
    check_projects(&doc).unwrap_or_else(|why| panic!("the finding's map on this host: {why}"));
}

/// **Which texts may lay a `.notdef`** (#387), on the compiled faces alone:
/// a text one face maps, the code points the shaper hides and a tab and a
/// line break, each handed over as a space, may not; a code point no face
/// maps may, and so may a cluster each of whose characters some face maps
/// and no one face maps together — U+13000, an Egyptian hieroglyph, and
/// U+0301, which faces of their own map, joined: why the bound asks for
/// one face and not for each character's.
///
/// Control, run and seen red: with "may lay a `.notdef`" read as "holds a
/// character no face maps", the hieroglyph and its accent.
#[test]
fn test_one_face_must_map_a_whole_text_for_it_to_lay_no_notdef() {
    let name =
        "projection::tests::notdef_bound_tests::test_one_face_must_map_a_whole_text_for_it_to_lay_no_notdef";
    let cases: [(&str, bool); 8] = [
        ("MANDALA mindmap", false),
        ("a\u{200d}b\u{fe0f}\u{e0041}", false),
        ("a\tb\nc\rd\u{2029}e", false),
        ("\u{13000}\u{13001}", false),
        ("\u{1}", true),
        ("a\u{20e3}", true),
        ("\u{3164}", true),
        ("\u{13000}\u{301}", true),
    ];
    if std::env::var(CHILD).as_deref() == Ok("faces") {
        report_hosts();
        let metrics = face_metrics();
        for (text, _) in cases {
            let mapped = text
                .chars()
                .all(|c| metrics.narrowest.contains_key(&u32::from(c)));
            println!(
                "REPORT TEXT {text:?} may {} every character mapped {mapped}",
                may_lay_notdef(metrics, text.chars())
            );
        }
        return;
    }
    let Some(reports) = on_compiled_faces(name, "faces") else {
        return;
    };
    for report in &reports {
        eprintln!("{report}");
    }
    assert_eq!(reports.len(), cases.len(), "{reports:#?}");
    for ((text, may), report) in cases.iter().zip(&reports) {
        assert!(
            report.starts_with(&format!("TEXT {text:?} may {may} ")),
            "{text:?}: {report}"
        );
    }
    let cluster = &reports[cases.len() - 1];
    assert!(
        cluster.ends_with("every character mapped true"),
        "precondition: some face maps each of the hieroglyph and its accent: {cluster}"
    );
}

/// **The shaper hides every code point the bound reads as hidden**
/// ([`SHAPER_HIDDEN`]; #387), on the compiled faces alone, where none of
/// them is drawn: each, shaped alone and after a letter, lays no glyph 0;
/// and each code point of `Default_Ignorable_Code_Point` the table leaves
/// out — the Hangul fillers, the shorthand format controls and U+180F —
/// does, alone, where no compiled face maps it, so the table is not too
/// wide where it can be seen to be.
///
/// Control, run and seen red: with U+3164 added to the table, it laid as
/// glyph 0 alone (and `test_one_face_must_map_a_whole_text_for_it_to_lay_no_notdef`
/// read it as laying none).
#[test]
fn test_the_shaper_hides_every_code_point_the_bound_reads_as_hidden() {
    use crate::document::resolve::ResolvedText;
    use crate::document::values::{FontSize, Rgba, Weight};
    use crate::flow::{shape_run, FlowSpan};
    use crate::font::fonts::{AppFont, FontSystemLease};
    let name = "projection::tests::notdef_bound_tests::test_the_shaper_hides_every_code_point_the_bound_reads_as_hidden";
    let left_out = [
        '\u{115F}',
        '\u{1160}',
        '\u{180F}',
        '\u{3164}',
        '\u{FFA0}',
        '\u{1BCA0}',
        '\u{1BCA3}',
    ];
    if std::env::var(CHILD).as_deref() == Ok("hidden") {
        report_hosts();
        let style = ResolvedText {
            font: AppFont::Any,
            size: FontSize::Pt(16.0),
            color: Rgba::opaque(255, 255, 255),
            weight: Weight::NORMAL,
            italic: false,
        };
        // Read before the lease is taken: the metrics take the font
        // system's lock of their own.
        let metrics = face_metrics();
        let mut lease = FontSystemLease::new("the shaper's hidden code points");
        let mut notdefs = |text: &str| {
            let run = shape_run(
                &[FlowSpan {
                    text,
                    style: &style,
                    link: None,
                }],
                1.0,
                &mut lease,
            );
            run.layout_glyphs().filter(|glyph| glyph.glyph_id == 0).count()
        };
        let (mut read, mut laid) = (0, Vec::new());
        for &(first, last) in &SHAPER_HIDDEN {
            for c in (first..=last).filter_map(char::from_u32) {
                read += 1;
                if notdefs(&c.to_string()) + notdefs(&format!("a{c}")) > 0 {
                    laid.push(format!("U+{:04X}", u32::from(c)));
                }
            }
        }
        println!("REPORT HIDDEN read {read} laid as a notdef {laid:?}");
        for c in left_out {
            println!(
                "REPORT LEFT OUT U+{:04X} mapped {} laid as a notdef {}",
                u32::from(c),
                metrics.narrowest.contains_key(&u32::from(c)),
                notdefs(&c.to_string()) > 0
            );
        }
        return;
    }
    let Some(reports) = on_compiled_faces(name, "hidden") else {
        return;
    };
    for report in &reports {
        eprintln!("{report}");
    }
    assert_eq!(
        reports.first().map(String::as_str),
        Some("HIDDEN read 4165 laid as a notdef []"),
        "{reports:#?}"
    );
    let mut shown = 0;
    for report in &reports[1..] {
        if report.contains(" mapped false ") {
            shown += 1;
            assert!(report.ends_with("laid as a notdef true"), "{report}");
        }
    }
    assert_eq!(reports.len(), 1 + left_out.len(), "{reports:#?}");
    assert!(
        shown > 0,
        "precondition: some code point left out is mapped by no compiled face: {reports:#?}"
    );
}
