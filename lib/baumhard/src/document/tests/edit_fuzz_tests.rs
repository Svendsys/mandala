// SPDX-License-Identifier: MPL-2.0

//! The authored-edit fuzz properties held to account (TEST_CONVENTIONS
//! §T14): the generator swept under a seeded source, the seed corpus and
//! hostile texts read as files, and each property shown to go red.

use crate::document::bounds::{MAX_DEFINITION_DEPTH, MAX_DEFINITION_PARTS};
use crate::document::tests::edit_fuzz::{check_edits_generated, check_edits_json, generate};
use crate::util::tests::choices::Choices;

#[test]
fn test_generated_edits_hold_every_property() {
    let mut ran = 0;
    for seed in 0..400 {
        let mut choices = Choices::seeded(seed);
        check_edits_generated(&mut choices).unwrap_or_else(|why| panic!("seed {seed}: {why}"));
        ran += 1;
    }
    assert_eq!(ran, 400);
    // Precondition: the generator reaches both sides of the bounds and
    // every body, or the sweep proves less than it says.
    let texts: Vec<String> = (0..400)
        .map(|seed| generate(&mut Choices::seeded(seed)))
        .collect();
    for needle in [
        r#""mutations""#,
        r#""mutator""#,
        r#""handler""#,
        r#""filter""#,
        "spiral",
    ] {
        assert!(
            texts.iter().any(|t| t.contains(needle)),
            "the sweep never writes {needle}"
        );
    }
    let deepest = texts
        .iter()
        .map(|t| t.matches(r#""children""#).count())
        .max()
        .unwrap_or(0);
    assert!(
        deepest > MAX_DEFINITION_DEPTH,
        "the sweep never passes the depth bound"
    );
}

/// The bounds' edges and hostile texts: each read as a file and run on
/// the map.
#[test]
fn test_seed_and_hostile_edit_files_hold_every_property() {
    // The committed seeds are run by `util::tests::fuzz_seed_tests`.
    let parts = |count: usize| {
        format!(
            r#"{{"edits":[{{"id":"p","name":"P","reach":"self","target":"node","mutations":[{}]}}]}}"#,
            vec![r#"{"node":"toggle_folded"}"#; count].join(",")
        )
    };
    for text in [
        parts(MAX_DEFINITION_PARTS),
        parts(MAX_DEFINITION_PARTS + 1),
        "[[[[[".to_string(),
        String::new(),
        "\u{feff}{}".to_string(),
        format!(
            r#"{{"edits":[{{"id":"d","x":{}{}}}]}}"#,
            "[".repeat(500),
            "]".repeat(500)
        ),
        r#"{"edits":[{"id":"","name":"E","reach":"self","target":"node","handler":"flower"}]}"#.to_string(),
        r#"{"edits":[],"extra":1}"#.to_string(),
    ] {
        check_edits_json(text.as_bytes()).unwrap_or_else(|why| panic!("{why}"));
    }
}

/// The path property goes red on a generated text the reader refuses
/// outside an edit — what a refusal that named no path would look like.
#[test]
fn test_the_path_property_reports_a_refusal_outside_an_edit() {
    // A planted generated-shape text whose refusal is at the top level.
    let refused = crate::document::edit::parse_edits(r#"{"edits":[],"extra":1}"#).unwrap_err();
    assert!(!refused.path.starts_with("edits["), "precondition: {refused}");
    // And the generated texts' refusals are all inside an edit.
    for seed in 0..50 {
        let text = generate(&mut Choices::seeded(seed));
        if let Err(error) = crate::document::edit::parse_edits(&text) {
            assert!(error.path.starts_with("edits["), "seed {seed}: {error}");
        }
    }
}
