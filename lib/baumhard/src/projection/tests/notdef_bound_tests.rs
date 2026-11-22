// SPDX-License-Identifier: MPL-2.0

//! The work bound and the shaper's `.notdef` passes (#387): CI's
//! load_structured finding on #380, a frame of a tab beside code points no
//! face draws, replayed on the compiled faces alone, where it laid past
//! the bound as it stood; each other shape of the same blind spot beside
//! it — the frames the bound now excludes, the directional rings whose
//! pass it now counts; and the facts the bound rests on: which texts may
//! lay a `.notdef`, which frames may shape one again, and which code
//! points the shaper hides.
//!
//! Each check runs in a child process whose font system holds the compiled
//! faces alone (`FONTCONFIG_FILE` naming a config of one empty directory),
//! so it fails alike on every host: a host face that draws what the
//! compiled ones do not — Unifont maps U+0001 and U+20E3 — would shape the
//! finding with no `.notdef` at all. A child that reports no font system
//! fails the test on every host. A target whose fontdb does not read
//! `FONTCONFIG_FILE` makes a child whose font system holds host faces, and
//! skips with a note; CI runs on Linux and sets
//! `MANDALA_REQUIRE_FONTCONFIG_HOSTS`, under which it fails instead.

use serde_json::json;

use crate::document::element::DocElement;
use crate::document::format::load;
use crate::document::ids::Document;
use crate::projection::tests::projection_fuzz::{
    check_projects, directional_notdef_steps, face_metrics, frame_may_shape_notdef, may_lay_notdef,
    projection_counts, ring_steps, within_bound, SHAPER_HIDDEN,
};
use crate::projection::tests::support::Projected;
use crate::util::tests::compiled_faces::{on_compiled_faces, report_hosts, this_test, CHILD};

/// CI's finding (job 136906833586 on #380): the map its input made, and the
/// input itself, the load_structured target's choices.
const FINDING: &[u8] =
    include_bytes!("../../document/tests/fuzz_findings/a_tabbed_ring_of_notdefs.mandala.json");
const FINDING_CHOICES: &[u8] =
    include_bytes!("../../document/tests/fuzz_findings/structured_a_tabbed_ring_of_notdefs");

/// The number after `key` in `report`.
fn number(report: &str, key: &str) -> usize {
    report
        .split(key)
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{report}: no number after {key:?}"))
}

/// What the bound does with a ring of [`blind_spot_maps`].
#[derive(Clone, Copy, Debug, PartialEq)]
enum Held {
    /// A frame laid by halves that may shape a `.notdef` again: excluded
    /// ([`frame_may_shape_notdef`]).
    Excluded,
    /// A directional ring whose counted pass takes it past its share.
    Outside,
    /// A directional ring the bound admits with its pass counted.
    Inside,
    /// A directional ring of text one face draws: no pass counted, though
    /// its reshape is charged, the shaping the rates were measured with.
    Drawn,
}

/// A map of one node at the origin, `min_size` across, framed by `ring`.
fn framed(min_size: f32, ring: serde_json::Value) -> String {
    json!({"format": 1, "name": "l", "nodes": [{"id": 1, "position": [0, 0], "min_size": [min_size, min_size], "frame": {"rings": [ring]}}]})
        .to_string()
}

/// The rings of the bound's blind spot (#387), laid on the compiled faces,
/// each with what the bound does with it: CI's map and each other shape of
/// frame a `.notdef` pass took past the bound as it stood in — a tab
/// beside code points no face draws, a `fit` ring with a tab, a side
/// ornament and a `once` filler beside such text, and a cluster each of
/// whose characters some face maps and no one face maps together (an
/// Egyptian hieroglyph, which only its face maps, and U+0301, which that
/// face does not), all excluded; and `Repeat` rings that lay text by
/// direction, round a node 10 across: one of text one face draws, which
/// the bound counts no pass for though its reshape is charged — the gate
/// in front of the count ([`may_lay_notdef`]) — and, whose one shaping
/// holds such text, four the bound admits with their pass counted — one in Galatia SIL,
/// whose U+1EB0 it decomposes to three glyphs, and one round a node its
/// section draws wider than its `min_size`, whose text the lay shapes at
/// a larger size than the bound does — and two it holds outside by their
/// count, one of them Galatia SIL's twenty U+1EB0 beside the keycap's
/// cluster, 63 glyphs of 23 characters.
fn blind_spot_maps() -> Vec<(&'static str, String, Held)> {
    let frame = |ring: serde_json::Value| framed(0.0, ring);
    let directional = |font: Option<&str>, spans: &str, vertical: &str, corners: Option<[&str; 4]>| {
        let mut ring =
            json!({"fit": "repeat", "spacing": 8.0, "spans": [{"text": spans}], "vertical": vertical});
        if let Some(font) = font {
            ring["font"] = json!(font);
        }
        if let Some(corners) = corners {
            ring["corners"] = json!(corners);
        }
        framed(10.0, ring)
    };
    let twenty: String = "\u{1}\u{20e3}"
        .chars()
        .chain(std::iter::repeat_n('\u{1EB0}', 20))
        .collect();
    vec![
        (
            "CI's finding",
            String::from_utf8(FINDING.to_vec()).expect("the finding is text"),
            Held::Excluded,
        ),
        (
            "a tab beside six code points no face draws",
            frame(json!({"spans": [{"text": "\t\u{1}\u{2}\u{3}\u{4}\u{5}\u{6}"}]})),
            Held::Excluded,
        ),
        (
            "`fit`, a tab and a keycap",
            frame(json!({"fit": "fit", "spans": [{"text": "\tab\u{20e3}"}]})),
            Held::Excluded,
        ),
        (
            "a side ornament beside text no face draws",
            frame(json!({"sides": "*", "spans": [{"text": "a\u{1}\u{20e3}b"}]})),
            Held::Excluded,
        ),
        (
            "a `once` filler beside text no face draws",
            frame(
                json!({"fit": {"once": {"filler": "*"}}, "spans": [{"text": "a\u{1}\u{20e3}b\u{2}\u{3}"}]}),
            ),
            Held::Excluded,
        ),
        (
            "a tab beside a cluster no one face draws",
            frame(json!({"fit": "fit", "spans": [{"text": "\t\u{13000}\u{301}abcdefgh"}]})),
            Held::Excluded,
        ),
        (
            "a directional ring of text one face draws",
            directional(None, "-=-", "|", Some(["+", "+", "+", "+"])),
            Held::Drawn,
        ),
        (
            "a directional ring of a keycap's cluster and a letter",
            directional(None, "\u{1}\u{20e3}a", "|", None),
            Held::Inside,
        ),
        (
            "a directional ring in Galatia SIL of a keycap's cluster and U+1EB0",
            directional(Some("Galatia SIL"), "\u{1}\u{20e3}\u{1EB0}", "|", None),
            Held::Inside,
        ),
        (
            "a directional ring of spans no face draws",
            directional(
                None,
                "\u{1}\u{20e3}\u{2}\u{3}\u{4}\u{5}\u{6}\u{7}\u{e}\u{f}\u{10}\u{11}",
                "|",
                None,
            ),
            Held::Outside,
        ),
        (
            "a directional ring of corners no face draws",
            directional(None, "-", "|", Some(["\u{1}\u{20e3}", "\u{2}", "\u{3}", "\u{4}"])),
            Held::Inside,
        ),
        (
            "a directional ring of a keycap's cluster round a node its section draws wider",
            json!({"format": 1, "name": "l", "nodes": [{
                "id": 1,
                "position": [0, 0],
                "min_size": [10.0, 10.0],
                "sections": [{"spans": [{"text": "M"}]}],
                "frame": {"rings": [{"fit": "repeat", "spacing": 8.0, "spans": [{"text": "\u{1}\u{20e3}a"}], "vertical": "|"}]},
            }]})
            .to_string(),
            Held::Inside,
        ),
        (
            "a directional ring in Galatia SIL of a keycap's cluster and twenty U+1EB0",
            directional(Some("Galatia SIL"), &twenty, "|", None),
            Held::Outside,
        ),
    ]
}

/// A `Repeat` ring of spans alone of code points no face draws, round a
/// node 10 across: what a lay of a ring of no `vertical` text and no
/// corners does with text the shaper lays as `.notdef`.
fn plain_repeat_map() -> String {
    framed(
        10.0,
        json!({"fit": "repeat", "spacing": 8.0, "spans": [{"text": "\u{1}\u{20e3}\u{2}\u{3}"}]}),
    )
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

/// In a child: lay `map`'s one ring, and report what the bound reads of it
/// and what its lay took: whether it is inside the bound and excluded,
/// the steps laid, the steps allowed ([`ring_steps`]), the `.notdef`
/// count ([`directional_notdef_steps`]), and what a directional ring's
/// reshape charged as it was laid (`flow::LAST_RESHAPE`, `none` where none
/// was).
fn report_ring(what: &str, map: &str) {
    let doc = load(map).unwrap_or_else(|why| panic!("{what}: does not load: {why}"));
    let (frame, ring) = the_ring(&doc);
    let allowed = ring_steps(doc.tree(), frame, ring).expect("a ring of text");
    let count = directional_notdef_steps(doc.tree(), frame, ring);
    let excluded = frame_may_shape_notdef(doc.tree(), ring);
    crate::flow::MOST_RING_LAY.with(|most| most.set(0));
    crate::flow::LAST_RESHAPE.with(|last| last.set(None));
    drop(Projected::new(doc.clone()));
    let laid = crate::flow::MOST_RING_LAY.with(std::cell::Cell::get);
    let charged = crate::flow::LAST_RESHAPE
        .with(std::cell::Cell::get)
        .map_or("none".to_string(), |steps| steps.to_string());
    println!(
        "REPORT RING {what}: inside {} excluded {excluded} laid {laid} allowed {allowed} count {count} charged {charged}",
        within_bound(&doc)
    );
}

/// **Every lay that may pay the shaper's `.notdef` passes is excluded or
/// counted** (#387). CI's load_structured job on #380 (job 136906833586)
/// found a map the bound admitted whose one ring, `\n\r\t\u{1}` and U+FE0F
/// U+20E3, took 14,157 glyph steps a lay where the bound allowed 9,344: no
/// compiled face draws U+0001 or U+20E3, the shaper joins them and the
/// variation selector into one cluster, a `.notdef` of three code points,
/// and a frame whose text holds a tab shapes it again at each reshape, a
/// pass through every face a glyph of its line, 2,200 steps each
/// (`halves::notdef_steps`), where the bound counted 192 a character. On
/// the compiled faces alone, each ring of the blind spot
/// ([`blind_spot_maps`]) is held as listed:
///
/// - each frame lays more steps than the bound's rates allow it — the
///   pass is laid, and the bound as it stood was short — and is excluded,
///   outside the bound;
/// - each directional ring's count is not nothing, and is exactly what its
///   reshape charged as it was laid (`flow::LAST_RESHAPE`), Galatia SIL's
///   decomposed U+1EB0 included; those it admits lay within what it
///   allows, and those it holds outside lay more than the rates allow
///   without the count.
///
/// The finding's input and its map are each checked once on the compiled
/// faces, and each is skipped there, outside the bound — `SKIPPED` rises
/// by one at each — and on this host's faces each is checked, projected
/// where the host's faces admit it and skipped, with a note, where they do
/// not. A `Repeat` ring of spans alone, of the same code points, shapes
/// nothing as it lays ([`plain_repeat_map`]): the bound counts it no pass,
/// no reshape is charged, and it lays fewer steps than one pass costs.
///
/// Controls, each run and seen red: no frame excluded (`text_frames_fit`
/// not reading `frame_may_shape_notdef`), each frame inside the bound, and
/// the finding's input failing as CI's did ("a ring's lay took 14157 glyph
/// steps, past the 9344 the bound allows its map"); no directional count,
/// every directional ring's count nothing where its reshape charged, and
/// the ring of spans no face draws admitted though it lays 49,256 steps of
/// the 29,312 it is then allowed; the count the bound read before the
/// review of #388, two `.notdef` passes a character of the lines, not the
/// reshape's charge, red on every directional ring, and on Galatia SIL's
/// twenty U+1EB0 — 63 glyphs of 23 characters — 101,200 where the
/// reshape charged 137,829; the gate in front of the count dropped, the
/// ring of text one face draws counted its whole charge, 562.
#[test]
fn test_every_lay_that_may_pay_a_notdef_pass_is_excluded_or_counted() {
    let name = this_test!();
    if std::env::var(CHILD).as_deref() == Ok("rings") {
        report_hosts();
        for (what, map, _) in blind_spot_maps() {
            report_ring(what, &map);
        }
        report_ring(
            "a plain `Repeat` ring of spans no face draws",
            &plain_repeat_map(),
        );
        let doc = load(std::str::from_utf8(FINDING).unwrap()).expect("the finding loads");
        let skipped = || projection_counts().1;
        let before = skipped();
        let structured = crate::document::tests::format_fuzz::check_structured(FINDING_CHOICES);
        let between = skipped();
        let projects = check_projects(&doc);
        let after = skipped();
        println!(
            "REPORT CHECKS structured {structured:?} skipped {} projects {projects:?} skipped {}",
            between - before,
            after - between
        );
        return;
    }
    let Some(reports) = on_compiled_faces(&name, "rings") else {
        return;
    };
    let mut wrong = Vec::new();
    let mut rings = 0;
    let held: Vec<(&str, Held)> = blind_spot_maps()
        .into_iter()
        .map(|(what, _, held)| (what, held))
        .collect();
    for report in &reports {
        eprintln!("{report}");
        let Some(ring) = report.strip_prefix("RING ") else {
            continue;
        };
        let Some(&(_, held)) = held
            .iter()
            .find(|(what, _)| ring.starts_with(&format!("{what}:")))
        else {
            continue;
        };
        rings += 1;
        let (laid, allowed, count) = (
            number(ring, " laid "),
            number(ring, " allowed "),
            number(ring, " count "),
        );
        let inside = ring.contains(" inside true ");
        let excluded = ring.contains(" excluded true ");
        let charged = ring.split(" charged ").nth(1).unwrap_or("none");
        let mut want = |holds: bool, what: &str| {
            if !holds {
                wrong.push(format!("{what}: {ring}"));
            }
        };
        match held {
            Held::Excluded => {
                want(
                    excluded && !inside,
                    "a frame that may shape a notdef again, excluded",
                );
                want(
                    laid > allowed,
                    "precondition, the pass laid past the bound's rates",
                );
                want(count == 0, "a frame counted no pass");
            }
            Held::Drawn => {
                want(!excluded && inside, "a directional ring one face draws, admitted");
                want(count == 0, "no pass counted where no line may lay one");
                want(
                    charged.parse::<usize>().is_ok_and(|charged| charged > 0),
                    "precondition, its reshape charged",
                );
            }
            Held::Outside | Held::Inside => {
                want(!excluded, "a directional ring is no frame to exclude");
                want(count > 0, "a pass counted");
                want(
                    charged == count.to_string(),
                    "the count, exactly what the reshape charged",
                );
                if held == Held::Inside {
                    want(inside && laid <= allowed, "admitted, and laid within its bound");
                } else {
                    want(!inside, "held outside the bound by its count");
                    want(
                        laid > allowed - count,
                        "precondition, the pass laid past the bound's rates",
                    );
                }
            }
        }
    }
    assert_eq!(
        rings,
        held.len(),
        "precondition: every ring reported: {reports:#?}"
    );
    assert!(wrong.is_empty(), "{wrong:#?}");
    let plain = reports
        .iter()
        .find_map(|r| r.strip_prefix("RING a plain `Repeat` ring of spans no face draws: "))
        .unwrap_or_else(|| panic!("the plain ring reported: {reports:?}"));
    assert!(
        plain.starts_with("inside true excluded false ")
            && plain.ends_with(" count 0 charged none")
            && number(plain, " laid ") < crate::flow::SHAPE_NOTDEF_STEPS,
        "a plain `Repeat` ring of code points no face draws, inside the bound, counted no pass and paid none: {plain}"
    );
    assert!(
        reports
            .iter()
            .any(|r| r == "CHECKS structured Ok(()) skipped 1 projects Ok(()) skipped 1"),
        "the finding's input and its map, each skipped once, outside the bound: {reports:#?}"
    );
    // And on this host's faces, whatever it has.
    let doc = load(std::str::from_utf8(FINDING).unwrap()).expect("the finding loads");
    crate::document::tests::format_fuzz::check_structured(FINDING_CHOICES)
        .unwrap_or_else(|why| panic!("the finding's input on this host: {why}"));
    check_projects(&doc).unwrap_or_else(|why| panic!("the finding's map on this host: {why}"));
    if within_bound(&doc) {
        eprintln!("the finding's map on this host's faces: inside the bound, projected and checked");
    } else {
        crate::util::test_skip::note_skipped(
            "the finding's map on this host's faces: outside the bound, so its check there is a skip",
        );
    }
}

/// What a frame of [`test_a_frame_that_may_shape_a_notdef_again_is_excluded`]
/// is to the bound, and what its lay pays the shaper's `.notdef` passes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Frame {
    /// Excluded, and its lay pays a pass.
    Pays,
    /// Excluded for a `once` filler's line alone, which only the branch
    /// whose halves cannot be measured shapes: its lay round a measurable
    /// outline pays none.
    Branch,
    /// Admitted, and its lay pays no pass.
    Clean,
}

/// **A frame that may shape a `.notdef` again is excluded** (#387): the
/// predicate the bound excludes a frame laid by halves by
/// ([`frame_may_shape_notdef`]), held to each of its terms both ways and to
/// what the engine's lay pays, on the compiled faces alone. A lay shapes a
/// line again where the frame has a side ornament — set, or a `once`
/// filler's — or a `once` filler, `sides: false` included, or is `Fit` with
/// a tab in its text, and the predicate is true exactly there, where a
/// line it shapes (the text, the ornament, the `x`, the filler) may lay a
/// `.notdef`. Each frame it is true of is outside the bound, and each it
/// is false of inside. Laid, each frame it is false of pays exactly no
/// pass, the passes every reshape of its lay that shaped paid
/// (`flow::RESHAPE_NOTDEF_STEPS`: a frame's charged shapings are its
/// reshapes and its ornament's `x` and ornament alone, the ornament shaped
/// again on a line of its own in the same build's reshape); each it is
/// true of for its text or its ornament pays one; and each it is true of
/// for a filler's line alone pays none round this outline, whose halves
/// can be measured — an exclusion for the branch that appends the filler.
///
/// Controls, each run and seen red: with a side ornament not read as
/// shaping again, the two ornaments' rings (the filler that is the sides
/// is held by the filler's term too); with a `Fit` frame shaping again whatever its text, the untabbed
/// `fit` ring; with any tabbed frame shaping again, `spread`'s; with a
/// `once` filler not read as shaping again, the fillers under `sides:
/// false`; with the filler's line not read, the two fillers no face draws
/// beside sides set or off; with "may lay a `.notdef`" read as "holds a
/// character no face maps", the cluster no one face draws; with no frame
/// excluded from the bound, each frame the predicate is true of inside it;
/// and with the predicate and its expectation wrong together — no side
/// ornament read as shaping again, and the ornaments' rows written as
/// admitted — the engine's side, the ornaments' lays paying a pass.
#[test]
fn test_a_frame_that_may_shape_a_notdef_again_is_excluded() {
    let name = this_test!();
    let frame = |ring: serde_json::Value| framed(0.0, ring);
    let cases: Vec<(&str, String, Frame)> = vec![
        (
            "an ornament beside text no face draws",
            frame(json!({"sides": "*", "spans": [{"text": "a\u{1}\u{20e3}b"}]})),
            Frame::Pays,
        ),
        (
            "an ornament no face draws",
            frame(json!({"sides": "\u{1}", "spans": [{"text": "abc"}]})),
            Frame::Pays,
        ),
        (
            "an ornament beside text one face draws",
            frame(json!({"sides": "*", "spans": [{"text": "abc"}]})),
            Frame::Clean,
        ),
        (
            "a `once` filler, its sides, no face draws",
            frame(json!({"fit": {"once": {"filler": "\u{1}"}}, "spans": [{"text": "abc"}]})),
            Frame::Pays,
        ),
        (
            "a `once` filler no face draws beside a set ornament",
            frame(json!({"fit": {"once": {"filler": "\u{1}"}}, "sides": "*", "spans": [{"text": "abc"}]})),
            Frame::Branch,
        ),
        (
            "a `once` filler one face draws beside a set ornament",
            frame(json!({"fit": {"once": {"filler": "-"}}, "sides": "*", "spans": [{"text": "abc"}]})),
            Frame::Clean,
        ),
        (
            "a `once` filler no face draws, `sides: false`",
            frame(json!({"fit": {"once": {"filler": "\u{1}"}}, "sides": false, "spans": [{"text": "abc"}]})),
            Frame::Branch,
        ),
        (
            "a `once` filler one face draws, `sides: false`, beside text no face draws",
            frame(json!({"fit": {"once": {"filler": "-"}}, "sides": false, "spans": [{"text": "ab\u{1}"}]})),
            Frame::Branch,
        ),
        (
            "a `once` filler one face draws, `sides: false`",
            frame(json!({"fit": {"once": {"filler": "-"}}, "sides": false, "spans": [{"text": "abc"}]})),
            Frame::Clean,
        ),
        (
            "`fit` with a tab beside a code point no face draws",
            frame(json!({"fit": "fit", "spans": [{"text": "\tab\u{1}"}]})),
            Frame::Pays,
        ),
        (
            "`fit` with a tab beside text one face draws",
            frame(json!({"fit": "fit", "spans": [{"text": "\tabc"}]})),
            Frame::Clean,
        ),
        (
            "`fit` with no tab beside a code point no face draws",
            frame(json!({"fit": "fit", "spans": [{"text": "ab\u{1}\u{20e3}"}]})),
            Frame::Clean,
        ),
        (
            "`spread` with a tab beside a code point no face draws",
            frame(json!({"fit": "spread", "spans": [{"text": "\tab\u{1}\u{20e3}"}]})),
            Frame::Clean,
        ),
        (
            "`fit` with a tab beside a cluster no one face draws",
            frame(json!({"fit": "fit", "spans": [{"text": "\t\u{13000}\u{301}"}]})),
            Frame::Pays,
        ),
    ];
    if std::env::var(CHILD).as_deref() == Ok("frames") {
        report_hosts();
        for (what, map, _) in &cases {
            let doc = load(map).unwrap_or_else(|why| panic!("{what}: does not load: {why}"));
            let (_, ring) = the_ring(&doc);
            let excluded = frame_may_shape_notdef(doc.tree(), ring);
            let inside = within_bound(&doc);
            crate::flow::RESHAPE_NOTDEF_STEPS.with(|paid| paid.set(0));
            drop(Projected::new(doc));
            let paid = crate::flow::RESHAPE_NOTDEF_STEPS.with(std::cell::Cell::get);
            println!("REPORT FRAME {what}: excluded {excluded} inside {inside} paid {paid}");
        }
        return;
    }
    let Some(reports) = on_compiled_faces(&name, "frames") else {
        return;
    };
    for report in &reports {
        eprintln!("{report}");
    }
    assert_eq!(
        reports.len(),
        cases.len(),
        "precondition: every frame reported: {reports:#?}"
    );
    let mut wrong = Vec::new();
    for (what, _, held) in &cases {
        let Some(report) = reports
            .iter()
            .find_map(|r| r.strip_prefix(&format!("FRAME {what}: ")))
        else {
            wrong.push(format!("{what}: not reported"));
            continue;
        };
        let excluded = *held != Frame::Clean;
        if !report.starts_with(&format!("excluded {excluded} inside {} ", !excluded)) {
            wrong.push(format!("{what}: excluded {excluded} wanted: {report}"));
        }
        let paid = number(report, " paid ");
        let pays = match held {
            Frame::Pays => paid > 0,
            Frame::Branch | Frame::Clean => paid == 0,
        };
        if !pays {
            wrong.push(format!("{what}: the engine's side, {held:?} wanted: {report}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}\n{reports:#?}");
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
    let name = this_test!();
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
    let Some(reports) = on_compiled_faces(&name, "faces") else {
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
    let name = this_test!();
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
            font: AppFont::Any.into(),
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
    let Some(reports) = on_compiled_faces(&name, "hidden") else {
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
