// SPDX-License-Identifier: MPL-2.0

//! The authored-edit fuzz properties held to account (TEST_CONVENTIONS
//! §T14): the generator swept under a seeded source, the seed corpus and
//! hostile texts read as files, and each property shown to go red.

use crate::document::bounds::{MAX_DEFINITION_DEPTH, MAX_DEFINITION_PARTS};
use crate::document::tests::edit_fuzz::{
    check_edits_built_from_what_plays, check_edits_generated, check_edits_json, generate,
};
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

/// The differential goes red on an edit built from the map as it is
/// drawn (ruling D2.1 §2 on #251): the app's own `flower` lays a node's
/// children out round it from where they are, and under a performance
/// that moves every node they are somewhere else; a filter that keeps
/// the large spans keeps other spans where a presentation grows them.
/// Built from the map itself, both pass.
#[test]
fn test_the_differential_reports_an_edit_built_from_what_plays() {
    let bundled = std::fs::read_to_string(crate::util::doc_fixtures::repo_path("assets/edits.json"))
        .expect("the bundled edits are in the repository");
    let bundled = bundled.as_str();
    check_edits_json(bundled.as_bytes()).unwrap_or_else(|why| panic!("{why}"));
    let why = check_edits_built_from_what_plays(bundled).expect_err("a layout read off what plays");
    assert!(why.contains("under a performance"), "{why}");
    let filtered = r##"{"edits":[{"id":"large","name":"L","reach":"subtree","target":"spans",
        "filter":{"fields":[[{"font_size":{"pt":20.0}},{"GreaterThan":false}]]},
        "mutations":[{"span":{"text":{"set_color":"#102030"}}}]}]}"##;
    check_edits_json(filtered.as_bytes()).unwrap_or_else(|why| panic!("{why}"));
    let why = check_edits_built_from_what_plays(filtered).expect_err("a filter read off what plays");
    assert!(why.contains("under a performance"), "{why}");
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
