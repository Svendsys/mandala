// SPDX-License-Identifier: MPL-2.0

//! The presentation fuzz properties held to account (TEST_CONVENTIONS
//! §T14): the generator swept under a seeded source, the bounds' edges
//! and hostile texts read as files, and each property shown to go red.

use crate::document::bounds::{MAX_ANIMATE_MS, MAX_DEFINITION_PARTS};
use crate::document::presentation::parse_presentations;
use crate::document::tests::presentation_fuzz::{
    check_presentations_generated, check_presentations_json, check_presentations_text, generate, Played,
    BAD_GLYPH_OPS, GLYPH_OPS, LIFECYCLE_MS, PLAYABLE,
};
use crate::util::tests::choices::Choices;

/// The sweep: 200 generated files hold every property, and the sweep is
/// one — it starts instances, changes the map under them, and writes
/// every play, both layers, both bodies and both sides of the bounds.
///
/// Controls, each seen red at seed 20: with `format::save` reading the
/// tree as it stands, a frame "saves to other bytes under what plays";
/// with the writer's funnel leaving the layer applied across a change,
/// the map does after the edit `cascade`.
#[test]
fn test_generated_presentations_hold_every_property() {
    let mut played = Played::default();
    for seed in 0..200 {
        let mut choices = Choices::seeded(seed);
        let one =
            check_presentations_generated(&mut choices).unwrap_or_else(|why| panic!("seed {seed}: {why}"));
        played.started += one.started;
        played.edited += one.edited;
    }
    // Held from below: a sweep whose every start was refused, or whose
    // every edit changed nothing, would hold nothing.
    assert!(played.started >= 55, "{played:?}");
    assert!(played.edited >= 190, "{played:?}");
    // Precondition: the generator reaches what the properties are about.
    let texts: Vec<String> = (0..400)
        .map(|seed| generate(&mut Choices::seeded(seed)))
        .collect();
    for needle in [
        r#""play":"while""#,
        r#""play":"once""#,
        r#""play":"loop""#,
        r#""play":"hold""#,
        r#""play":"toggle""#,
        r#""layer":"glyph""#,
        r#""layer":"document""#,
        r#""glyph":["#,
        r#""mutations":["#,
        r#""filter""#,
        r#""return""#,
        r#""run_overrides":"clear""#,
        &format!(r#""ms":{MAX_ANIMATE_MS},"#),
        &format!(r#""ms":{},"#, MAX_ANIMATE_MS + 1),
        r#""ms":0,"#,
    ] {
        assert!(
            texts.iter().any(|t| t.contains(needle)),
            "the sweep never writes {needle}"
        );
    }
    let (read, refused): (Vec<&String>, Vec<&String>) =
        texts.iter().partition(|text| parse_presentations(text).is_ok());
    assert!(
        read.len() >= 40 && refused.len() >= 40,
        "{} read, {} refused",
        read.len(),
        refused.len()
    );
}

/// The generator's tables are what it says they are: every playable
/// mutation plays on the fixture at its target, every glyph op reads on
/// a target that draws runs, and every bad one is refused — so "most
/// are ones the fixture plays" is a fact about the sweep, not a hope.
#[test]
fn test_the_generators_tables_are_what_they_are_called() {
    for (target, mutations) in PLAYABLE {
        for mutation in mutations {
            let file = format!(
                r#"{{"presentations":[{{"id":"p","name":"P","play":"hold","target":"{target}","mutations":[{mutation}]}}]}}"#
            );
            // Top-level nodes have no connector to play on.
            let plays = usize::from(target != "connector");
            assert_eq!(
                check_presentations_text(&file, true).map(|played| played.started),
                Ok(plays),
                "{mutation} on {target}"
            );
        }
    }
    let glyph = |op: &str| {
        parse_presentations(&format!(
            r#"{{"presentations":[{{"id":"g","name":"G","play":"hold","layer":"glyph","target":"sections","glyph":[{op}]}}]}}"#
        ))
    };
    for op in GLYPH_OPS {
        assert!(glyph(op).is_ok(), "{op}: {:?}", glyph(op).err());
    }
    for op in BAD_GLYPH_OPS {
        let refused = glyph(op).expect_err(op);
        assert!(
            refused.path.starts_with("presentations[0].glyph[0]"),
            "{op}: {refused}"
        );
    }
}

/// The bounds' edges and hostile texts: each read as a file and, where
/// it reads, played on the map.
#[test]
fn test_seed_and_hostile_presentation_files_hold_every_property() {
    // The committed seeds are run by `util::tests::fuzz_seed_tests`.
    let parts = |count: usize| {
        format!(
            r#"{{"presentations":[{{"id":"p","name":"P","play":"hold","target":"node","mutations":[{}]}}]}}"#,
            vec![r#"{"node":"toggle_folded"}"#; count].join(",")
        )
    };
    let timed = |ms: u32| {
        format!(
            r#"{{"presentations":[{{"id":"t","name":"T","play":"loop","target":"node","mutations":[{{"node":{{"set_opacity":0.5}}}}],"animate":{{"ms":{ms},"delay_ms":{ms}}},"return":{{"ms":{ms},"delay_ms":{ms}}}}}]}}"#
        )
    };
    for text in [
        parts(MAX_DEFINITION_PARTS),
        parts(MAX_DEFINITION_PARTS + 1),
        // The longest lifecycle the format reads, and one past it.
        timed(MAX_ANIMATE_MS),
        timed(MAX_ANIMATE_MS + 1),
        "[[[[[".to_string(),
        String::new(),
        "\u{feff}{}".to_string(),
        format!(
            r#"{{"presentations":[{{"id":"d","x":{}{}}}]}}"#,
            "[".repeat(500),
            "]".repeat(500)
        ),
        r#"{"presentations":[{"id":"","name":"P","play":"hold","target":"node","mutations":[]}]}"#
            .to_string(),
        r#"{"presentations":[],"extra":1}"#.to_string(),
    ] {
        check_presentations_json(text.as_bytes()).unwrap_or_else(|why| panic!("{why}"));
    }
    // Precondition: the longest lifecycle reads, plays, and is the one
    // the lifecycle bound is drawn from.
    let longest = timed(MAX_ANIMATE_MS);
    assert_eq!(
        check_presentations_text(&longest, true).map(|played| played.started),
        Ok(1)
    );
    assert_eq!(LIFECYCLE_MS, 4.0 * f64::from(MAX_ANIMATE_MS) + 1.0);
}

/// The differential property goes red when a change is built from what
/// plays (ruling D2.1 §2): under a presentation that fades every node,
/// an edit that changes the opaque nodes, resolved against the map as
/// it is drawn, reaches none — and the same input passes when the edit
/// is resolved against the map itself.
#[test]
fn test_the_differential_reports_a_change_built_from_what_plays() {
    let faded = r#"{"presentations":[{"id":"fade","name":"F","play":"hold","target":"node","mutations":[{"node":{"set_opacity":0.25}}]}]}"#;
    let played = check_presentations_text(faded, true).unwrap_or_else(|why| panic!("{why}"));
    assert_eq!(played.started, 1);
    assert!(played.edited >= 3, "{played:?}");
    let why = check_presentations_text(faded, false).expect_err("built from what plays, the edits differ");
    assert!(
        why.contains("under what plays"),
        "the refusal is the differential's: {why}"
    );
    // And a layout, which reads where the nodes are, under a lift.
    let lifted = r#"{"presentations":[{"id":"lift","name":"L","play":"hold","target":"node","mutations":[{"node":{"nudge":[0.0,-600.0]}}]}]}"#;
    check_presentations_text(lifted, true).unwrap_or_else(|why| panic!("{why}"));
    let why =
        check_presentations_text(lifted, false).expect_err("laid out from what plays, the edits differ");
    assert!(why.contains("under what plays"), "{why}");
}

/// The path property goes red on a generated text the reader refuses
/// outside a presentation — what a refusal that named no path would
/// look like — and the generated texts' refusals are all inside one.
#[test]
fn test_the_path_property_reports_a_refusal_outside_a_presentation() {
    let refused = parse_presentations(r#"{"presentations":[],"extra":1}"#).unwrap_err();
    assert!(
        !refused.path.starts_with("presentations["),
        "precondition: {refused}"
    );
    let mut inside = 0;
    for seed in 0..100 {
        let text = generate(&mut Choices::seeded(seed));
        if let Err(error) = parse_presentations(&text) {
            assert!(error.path.starts_with("presentations["), "seed {seed}: {error}");
            inside += 1;
        }
    }
    assert!(inside >= 10, "{inside} of 100 generated texts were refused");
}
