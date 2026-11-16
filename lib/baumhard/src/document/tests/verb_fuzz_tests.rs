// SPDX-License-Identifier: MPL-2.0

//! The verb fuzz properties held to account (TEST_CONVENTIONS §T14): the
//! generator swept under a seeded source, hostile texts read as a map's
//! verbs, and the properties shown to go red.

use crate::document::tests::verb_fuzz::{check_verbs_generated, check_verbs_json, check_verbs_text, Ran};
use crate::util::tests::choices::Choices;

/// The sweep: 300 generated sets of verbs hold every property, and the
/// sweep reaches what they are about — maps that load and maps that are
/// refused, invocations, steps reaching the performer, and waits.
///
/// Controls, each seen red: with `check_graph` not called, seed 4's
/// verbs load where the model refuses them; with the depth bound one
/// verb looser, seed 7's do; with `On::nodes` keeping an id the map
/// lacks as the map's root, the D27 property fails ("reached …, which is
/// not a node in the map").
#[test]
fn test_generated_verbs_hold_every_property() {
    let (mut total, mut loaded, mut refused) = (Ran::default(), 0, 0);
    for seed in 0..300 {
        let mut choices = Choices::seeded(seed);
        let ran = check_verbs_generated(&mut choices).unwrap_or_else(|why| panic!("seed {seed}: {why}"));
        if ran.loaded {
            loaded += 1;
        } else {
            refused += 1;
        }
        total.invoked += ran.invoked;
        total.calls += ran.calls;
        total.waited += ran.waited;
    }
    // Held from below: a sweep whose every map was refused, or whose
    // every run did nothing, would hold nothing.
    assert!(
        loaded >= 40 && refused >= 40,
        "{loaded} loaded, {refused} refused"
    );
    assert!(total.invoked >= 200, "{total:?}");
    assert!(total.calls >= 1_000, "{total:?}");
    assert!(total.waited >= 30, "{total:?}");
}

/// Hostile texts in the `verbs` place: nothing panics, and every one is
/// read or refused within `verbs` — not JSON, the wrong type, nesting
/// deeper than the reader follows, a step with a console line, a wait of
/// every width.
#[test]
fn test_hostile_verbs_hold_every_property() {
    let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
    for text in [
        "",
        "null",
        "{}",
        "7",
        r#"[{"id":"a"}]"#,
        r#"[{"id":"a","name":"A","steps":{}}]"#,
        r#"[{"id":"a","name":"A","steps":[{"console":"save /tmp/x"}]}]"#,
        r#"[{"id":"a","name":"A","steps":[{"wait":-1}]}]"#,
        r#"[{"id":"a","name":"A","steps":[{"wait":18446744073709551615}]}]"#,
        r#"[{"id":"a","name":"A","steps":[{"present":"p","on":{"nodes":[[[[[[1]]]]]]}}]}]"#,
        r#"[{"id":"a","name":"A","steps":[{"present":"p","on":{"nodes":[1,1,1,99999999]}}]}]"#,
        r#"[{"id":"a","name":"A","steps":[{"present":"p","on":{"channel":4294967296,"within":"map"}}]}]"#,
        r#"[{"id":"","name":"A","steps":[]}]"#,
        r#"[{"id":"a","name":"A","steps":[]},{"id":"a","name":"B","steps":[]}]"#,
        &deep,
    ] {
        check_verbs_json(text.as_bytes()).unwrap_or_else(|why| panic!("{text:.80}: {why}"));
    }
    check_verbs_json(&[0xff, 0xfe, 0x00]).expect("bytes that are no text are no input");
    // A map whose verbs run: held to every run property, and it ran.
    let ran = check_verbs_text(
        r#"[{"id":"a","name":"A","steps":[{"present":"p","on":"self"},{"wait":5},{"edit":"e","on":{"nodes":[1,77]}},{"verb":"b"}]},
           {"id":"b","name":"B","steps":[{"stop":"p","on":"descendants"}]}]"#,
    )
    .expect("holds");
    assert!(
        ran.loaded && ran.invoked >= 2 && ran.calls >= 4 && ran.waited >= 1,
        "{ran:?}"
    );
}
