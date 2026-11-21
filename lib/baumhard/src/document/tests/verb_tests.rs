// SPDX-License-Identifier: MPL-2.0

//! Map verbs (V1-D3, #252): what a load accepts and refuses, how a run
//! goes on the clock — waits, inline calls, each target, the edit step's
//! one history entry, held runs and their release — and the id counter
//! a verb's target holds back (D32). Each assertion has its control
//! beside it, or named in its test's doc, that would make it fail.

use crate::arena::NodeId;
use crate::document::bounds::{BoundError, MAX_ANIMATE_MS, MAX_DEFINITION_PARTS};
use crate::document::element::DocKind;
use crate::document::format::{load, LoadErrorKind};
use crate::document::ids::{Document, NodeIdent};
use crate::document::mutation::compose;
use crate::document::presentation::Direction;
use crate::document::tests::verb_fuzz::{map_with_verbs, node, OnTheDocument};
use crate::document::values::VerbId;
use crate::document::verb::{Runs, VerbError, MAX_VERB_DEPTH};
use crate::document::writer::{ChangeSource, EditRequest};
use glam::Vec2;

/// The test map (`verb_fuzz::MAP_BEFORE_VERBS`) with these verbs.
fn with(verbs: &str) -> Document {
    load(&map_with_verbs(verbs)).unwrap_or_else(|error| panic!("{verbs} loads: {error}"))
}

/// The refusal of the test map with these verbs: its path and kind.
fn refused(verbs: &str) -> (String, LoadErrorKind) {
    match load(&map_with_verbs(verbs)) {
        Ok(_) => panic!("{verbs} loaded, and should not have"),
        Err(error) => (error.path, error.kind),
    }
}

/// The verb with index `n`.
fn verb(n: u16) -> VerbId {
    VerbId(n)
}

/// The nodes, by ident, each instance of `presentation` plays on, in
/// start order.
fn playing(doc: &Document, presentation: &str) -> Vec<u32> {
    doc.presentations()
        .filter(|instance| instance.presentation == presentation)
        .filter_map(|instance| {
            doc.get(instance.node)
                .and_then(|e| e.as_node())
                .map(|n| n.ident.0)
        })
        .collect()
}

/// **The call graph is checked at load**: a cycle is refused at the step
/// that closes it, naming the verbs; a chain of nine verbs is refused at
/// its head, while one of eight loads; and a call to a verb the map does
/// not define is refused at that step.
///
/// Controls, each seen red: with `check_graph` not called, the cycle and
/// the chain of nine load; with the depth check one verb looser
/// (`depth > MAX_VERB_DEPTH + 1`), the chain of nine loads.
#[test]
fn test_a_cycle_a_chain_too_deep_and_an_unknown_verb_are_refused() {
    let (path, kind) = refused(
        r#"[{"id":"a","name":"A","steps":[{"wait":1},{"verb":"b"}]},{"id":"b","name":"B","steps":[{"verb":"a"}]}]"#,
    );
    assert_eq!(path, "verbs[1].steps[0].verb");
    assert_eq!(
        kind,
        LoadErrorKind::VerbCycle {
            cycle: vec!["a".into(), "b".into(), "a".into()]
        }
    );
    // A verb that calls itself is the shortest cycle.
    assert_eq!(
        refused(r#"[{"id":"a","name":"A","steps":[{"verb":"a"}]}]"#).0,
        "verbs[0].steps[0].verb"
    );
    let chain = |length: usize| {
        let verbs: Vec<String> = (0..length)
            .map(|n| {
                let step = if n + 1 < length {
                    format!(r#"{{"verb":"c{}"}}"#, n + 1)
                } else {
                    r#"{"wait":0}"#.to_string()
                };
                format!(r#"{{"id":"c{n}","name":"C","steps":[{step}]}}"#)
            })
            .collect();
        format!("[{}]", verbs.join(","))
    };
    assert_eq!(MAX_VERB_DEPTH, 8);
    with(&chain(8));
    assert_eq!(
        refused(&chain(9)),
        (
            "verbs[0]".to_string(),
            LoadErrorKind::VerbTooDeep { max: MAX_VERB_DEPTH }
        )
    );
    assert_eq!(
        refused(r#"[{"id":"a","name":"A","steps":[{"verb":"nobody"}]}]"#),
        (
            "verbs[0].steps[0].verb".to_string(),
            LoadErrorKind::UnknownVerb { id: "nobody".into() }
        )
    );
}

/// **A step names the map's own presentations and edits** (amendment 2
/// on #252): one the map does not define is refused at its key — the App
/// tier's `lift` too, which a console could start — while the map's own
/// load.
#[test]
fn test_a_presentation_or_an_edit_the_map_lacks_is_refused() {
    with(
        r#"[{"id":"a","name":"A","steps":[{"present":"p","on":"self"},{"edit":"e","on":"self"},{"stop":"h","on":"self"}]}]"#,
    );
    assert_eq!(
        refused(r#"[{"id":"a","name":"A","steps":[{"present":"lift","on":"self"}]}]"#),
        (
            "verbs[0].steps[0].present".to_string(),
            LoadErrorKind::UnknownPresentation { id: "lift".into() }
        )
    );
    assert_eq!(
        refused(r#"[{"id":"a","name":"A","steps":[{"wait":1},{"stop":"nope","on":"self"}]}]"#).0,
        "verbs[0].steps[1].stop"
    );
    assert_eq!(
        refused(r#"[{"id":"a","name":"A","steps":[{"edit":"nudge","on":"self"}]}]"#),
        (
            "verbs[0].steps[0].edit".to_string(),
            LoadErrorKind::UnknownEdit { id: "nudge".into() }
        )
    );
}

/// **A node a target names is not checked at load (D27)**: an id no node
/// has loads, and is skipped when the step runs; an id no node can carry
/// is refused as out of bounds.
#[test]
fn test_a_missing_node_loads_and_an_impossible_one_does_not() {
    with(r#"[{"id":"a","name":"A","steps":[{"present":"p","on":{"nodes":[99]}}]}]"#);
    let (path, kind) = refused(r#"[{"id":"a","name":"A","steps":[{"present":"p","on":{"nodes":[1,0]}}]}]"#);
    assert_eq!(path, "verbs[0].steps[0].on.nodes[1]");
    assert_eq!(
        kind,
        LoadErrorKind::Bound {
            error: BoundError::NotAnIdent(0)
        }
    );
}

/// **No step kind beyond the five** — the sandbox (§11.3): a step with a
/// key of its own, with none of the five or with two, with `on` where
/// it takes none or without `on` where it needs one, and a target of no
/// spelling the format has, are each refused at the step; a `wait` past
/// a minute is refused as a bound (ruling D3.1).
///
/// Control, seen red: the check narrowed to two kinds or more
/// (`kinds > 1`) lets `{}` through to no kind, and it is not refused as
/// the step it is.
#[test]
fn test_a_step_is_one_of_five_and_nothing_else() {
    for (step, at) in [
        (r#"{"console":"save"}"#, "verbs[0].steps[0]"),
        (r#"{"action":"Quit"}"#, "verbs[0].steps[0]"),
        (
            r#"{"present":"p","on":{"open":"https://x"}}"#,
            "verbs[0].steps[0]",
        ),
        (r#"{"present":"p","on":"everything"}"#, "verbs[0].steps[0]"),
        (r#"{"present":"p","on":{"channel":1}}"#, "verbs[0].steps[0]"),
    ] {
        let (path, kind) = refused(&format!(r#"[{{"id":"a","name":"A","steps":[{step}]}}]"#));
        assert!(path.starts_with(at), "{step}: {path}");
        assert!(
            matches!(
                kind,
                LoadErrorKind::UnknownKey { .. }
                    | LoadErrorKind::MissingKey { .. }
                    | LoadErrorKind::Invalid { .. }
            ),
            "{step}: {kind:?}"
        );
    }
    for step in [
        r#"{}"#,
        r#"{"present":"p","stop":"p","on":"self"}"#,
        r#"{"present":"p"}"#,
    ] {
        let (path, kind) = refused(&format!(r#"[{{"id":"a","name":"A","steps":[{step}]}}]"#));
        assert_eq!(path, "verbs[0].steps[0]", "{step}");
        assert!(
            matches!(kind, LoadErrorKind::InvalidVerb { .. }),
            "{step}: {kind:?}"
        );
    }
    for step in [r#"{"wait":1,"on":"self"}"#, r#"{"verb":"a","on":"self"}"#] {
        let (path, kind) = refused(&format!(r#"[{{"id":"a","name":"A","steps":[{step}]}}]"#));
        assert_eq!(path, "verbs[0].steps[0].on", "{step}");
        assert!(
            matches!(kind, LoadErrorKind::KeyNotAllowed { key: "on", .. }),
            "{kind:?}"
        );
    }
    with(&format!(
        r#"[{{"id":"a","name":"A","steps":[{{"wait":{MAX_ANIMATE_MS}}}]}}]"#
    ));
    let (path, kind) = refused(&format!(
        r#"[{{"id":"a","name":"A","steps":[{{"wait":{}}}]}}]"#,
        MAX_ANIMATE_MS + 1
    ));
    assert_eq!(path, "verbs[0].steps[0].wait");
    assert!(matches!(kind, LoadErrorKind::Bound { .. }), "{kind:?}");
}

/// **A verb's parts are counted after its calls are expanded** (ruling
/// D3.1; owner, format Q4): a fan of four levels, each calling the next
/// `width` times, is `width + width·(…)` parts though its text is a few
/// hundred bytes. At width 15 it is 54,240 parts and loads; at 16,
/// 69,904, past 65,536, refused at the verb that fans out. The verbs
/// below it stay within the bound and are not the ones refused.
///
/// Control, seen red: the expansion not added to a verb's own parts lets
/// width 16 load.
#[test]
fn test_a_verb_is_bounded_by_its_parts_after_expansion() {
    let fan = |width: usize| {
        let level = |n: usize| {
            let step = if n < 3 {
                format!(r#"{{"verb":"f{}"}}"#, n + 1)
            } else {
                r#"{"wait":0}"#.to_string()
            };
            format!(
                r#"{{"id":"f{n}","name":"F","steps":[{}]}}"#,
                vec![step; width].join(",")
            )
        };
        format!("[{}]", (0..4).map(level).collect::<Vec<_>>().join(","))
    };
    let parts = |w: usize| w * (1 + w + w * w + w * w * w);
    assert_eq!((parts(15), parts(16)), (54_240, 69_904));
    assert!(parts(15) <= MAX_DEFINITION_PARTS && parts(16) > MAX_DEFINITION_PARTS);
    with(&fan(15));
    assert_eq!(
        refused(&fan(16)),
        (
            "verbs[0]".to_string(),
            LoadErrorKind::DefinitionTooLarge {
                max: MAX_DEFINITION_PARTS
            }
        )
    );
}

/// The names [`NameIndex`](crate::document::keyed::NameIndex) keys have
/// been compared while `f` ran, and what it returned (ruling D3.3 item
/// 3): the counter is the keys' own `PartialEq`, so a scan of an index's
/// keys counts every key it compares, whoever wrote it.
fn names_compared<R>(f: impl FnOnce() -> R) -> (usize, R) {
    use crate::document::keyed::NAMES_COMPARED;
    NAMES_COMPARED.with(|count| count.set(0));
    let out = f();
    (NAMES_COMPARED.with(|count| count.get()), out)
}

/// **A step's name is one probe, however long the lists it names
/// into** (#380 review, item 1; D29; rulings D3.3 item 3 and D3.4 item
/// 4): a map of 2,048 presentations, 2,048 edits and 2,048 verbs, each
/// verb naming the far end of each list, loads having compared at least
/// one name per name its steps spell — 4 × 2,048 − 1 — and at most twice
/// that, the room a hash probe's rare false match and the indexes' own
/// building take; where a scan compares on the order of 2,048 for each.
/// What is counted is every compare of a name a lookup could scan: the
/// indexes' keys (through `Borrow`, the one body a probe compares
/// through in every build) and, in a test build, the ids and names the
/// lists hold (`keyed::Name`).
///
/// Controls, each seen red: `NameIndex::get` scanning its keys
/// (`iter().find(|(known, _)| Borrow::<str>::borrow(*known) == name)`,
/// nothing of it counted by hand) compares millions; `NameKey`'s
/// `Borrow` not counting leaves the count under the floor.
#[test]
fn test_a_step_resolves_each_name_in_one_probe() {
    const N: usize = 2_048;
    let presentations: Vec<String> = (0..N)
        .map(|n| {
            format!(
                r#"{{"id":"p{n}","name":"P","play":"while","target":"node","mutations":[{{"node":{{"set_opacity":0.5}}}}]}}"#
            )
        })
        .collect();
    let edits: Vec<String> = (0..N)
        .map(|n| {
            format!(
                r#"{{"id":"e{n}","name":"E","reach":"self","target":"node","mutations":[{{"node":{{"nudge":[1.0,0.0]}}}}]}}"#
            )
        })
        .collect();
    let verbs: Vec<String> = (0..N)
        .map(|n| {
            let far = N - 1 - n;
            let call = if n + 1 < N {
                format!(r#",{{"verb":"v{}"}}"#, N - 1)
            } else {
                String::new()
            };
            format!(
                r#"{{"id":"v{n}","name":"V","steps":[{{"present":"p{far}","on":"self"}},{{"stop":"p{far}","on":"self"}},{{"edit":"e{far}","on":"self"}}{call}]}}"#
            )
        })
        .collect();
    let map = format!(
        r#"{{"format":1,"name":"wide","edits":[{}],"presentations":[{}],"verbs":[{}],"nodes":[{{"id":1,"position":[0,0],"sections":[{{"spans":[{{"text":"one"}}]}}]}}]}}"#,
        edits.join(","),
        presentations.join(","),
        verbs.join(",")
    );
    let (compared, doc) = names_compared(|| load(&map));
    let doc = doc.unwrap_or_else(|error| panic!("the wide map loads: {error}"));
    assert_eq!(doc.definitions().verbs.len(), N);
    let names = 4 * N - 1;
    assert!(
        (names..=2 * names).contains(&compared),
        "{compared} keys compared for {names} names"
    );
}

/// **What a trigger starts names in one probe a name** (ruling D3.3
/// item 2): with 4,096 verbs, theme variables and palettes, a verb that
/// starts a presentation of 64 `set_link`s to the last verb and 64 colors
/// of the last variable, and runs an edit of 64 `set_palette`s to the last
/// palette and 64 `set_click`s to the last verb, compares at least one
/// key per name each resolution reads and at most twice that — where a
/// scan of any of the three lists compares on the order of 4,096 a name.
///
/// The ids and names the lists hold count as the index's keys do (in a
/// test build, `keyed::Name`; ruling D3.4 item 4), so a scan of a list
/// counts wherever it is written — in place of the probe or beside it,
/// and through `==` or `as_str` alike: a counted name has no `Deref`,
/// `AsRef` or `Borrow` to read it through uncounted (ruling D3.5 item 4).
///
/// Controls, each seen red: the review's probes, `verb_of` scanning the
/// verbs before its probe through `==` (`position(|verb| verb.id ==
/// id)`) and through `as_str` (`verb.id.as_str() == id`: 524,674); `verb_of`,
/// `var_of` and `palette_of` each back to its scan of the list;
/// `NameIndex::get` scanning its keys — each compares hundreds of
/// thousands of names for the 256.
#[test]
fn test_a_trigger_resolves_each_name_in_one_probe() {
    const N: usize = 4_096;
    const M: usize = 64;
    let verbs: Vec<String> = (0..N)
        .map(|n| format!(r#"{{"id":"v{n}","name":"V","steps":[]}}"#))
        .collect();
    let variables: Vec<String> = (0..N).map(|n| format!(r##""c{n}":"#ff0000""##)).collect();
    let palettes: Vec<String> = (0..N)
        .map(|n| format!(r##""p{n}":[{{"frame":"#a20000","text":"#a30000"}}]"##))
        .collect();
    let last = N - 1;
    let span_mutations: Vec<String> = (0..M)
        .flat_map(|_| {
            [
                format!(r#"{{"span":{{"set_link":{{"verb":"v{last}"}}}}}}"#),
                format!(r#"{{"span":{{"text":{{"set_color":"var(c{last})"}}}}}}"#),
            ]
        })
        .collect();
    let node_mutations: Vec<String> = (0..M)
        .flat_map(|_| {
            [
                format!(r#"{{"node":{{"set_palette":"p{last}"}}}}"#),
                format!(r#"{{"node":{{"set_click":"v{last}"}}}}"#),
            ]
        })
        .collect();
    let map = format!(
        r#"{{"format":1,"name":"wide","theme":{{"active":"d","variants":{{"d":{{{}}}}}}},"palettes":{{{}}},
        "presentations":[{{"id":"wide","name":"W","play":"hold","target":"spans","mutations":[{}]}}],
        "edits":[{{"id":"wide-edit","name":"W","reach":"self","target":"node","mutations":[{}]}}],
        "verbs":[{{"id":"run","name":"Run","steps":[{{"present":"wide","on":"self"}},{{"edit":"wide-edit","on":"self"}}]}},{}],
        "nodes":[{{"id":1,"position":[0,0],"sections":[{{"spans":[{{"text":"one"}}]}}]}}]}}"#,
        variables.join(","),
        palettes.join(","),
        span_mutations.join(","),
        node_mutations.join(","),
        verbs.join(",")
    );
    let mut doc = load(&map).unwrap_or_else(|error| panic!("the wide map loads: {error}"));
    let one = node(&doc, 1);
    let mut runs = Runs::default();
    let (compared, ran) = names_compared(|| {
        let mut on = OnTheDocument::new(&mut doc);
        let ran = runs.invoke(verb(0), Some(one), false, 0.0, &mut on);
        (ran, on.calls.len())
    });
    let (ran, calls) = ran;
    ran.expect("runs");
    assert_eq!(calls, 2, "the presentation started and the edit run");
    assert!(doc.presentations().next().is_some(), "the presentation plays");
    let names = 4 * M;
    assert!(
        (names..=2 * names).contains(&compared),
        "{compared} keys compared for {names} names"
    );
}

/// **A verb's index is the definitions' own, and a debug build says when
/// it is not** (ruling D3.4 item 5): a `Definitions` holding a verb that
/// no document indexed has no index to answer from, and `verb_by_id`
/// stops there rather than answer `None` as though the map lacked it.
///
/// Control, seen red: `verb_by_id` without its `debug_assert!`s answers
/// `None` and does not panic.
#[test]
#[should_panic(expected = "1 verbs and no index")]
fn test_verb_by_id_refuses_definitions_no_document_indexed() {
    use crate::document::ids::{Definitions, VerbDef};
    let definitions = Definitions {
        verbs: vec![VerbDef {
            id: crate::document::keyed::name("go"),
            name: "Go".to_string(),
            description: String::new(),
            steps: Vec::new(),
        }],
        ..Definitions::default()
    };
    let _ = definitions.verb_by_id("go");
}

/// **A stale index says so in a debug build** (ruling D3.5 item 7): an
/// index that puts `b` where `a` is — what a `Definitions` whose verbs
/// changed after it was indexed would hold — stops `verb_by_id("b")`
/// rather than answer with `a`'s definition.
///
/// Control, seen red: `verb_by_id` without its second `debug_assert!`
/// answers `a` for `b` and does not panic.
#[test]
#[should_panic(expected = "the index puts `b` at")]
fn test_verb_by_id_refuses_a_stale_index() {
    use crate::document::ids::{Definitions, VerbDef};
    use crate::document::keyed::name;
    let verb = |id: &str| VerbDef {
        id: name(id),
        name: id.to_uppercase(),
        description: String::new(),
        steps: Vec::new(),
    };
    let mut definitions = Definitions {
        verbs: vec![verb("a"), verb("b")],
        ..Definitions::default()
    };
    definitions.verb_ids.insert_first("b", VerbId(0));
    let _ = definitions.verb_by_id("b");
}

/// **A link opens `http`, `https` and `mailto`, and nothing else**, at
/// load and at apply (amendment 3 on #252): a `file://` span link is
/// refused at its `url`, naming the scheme; an `https` one loads; and a
/// `set_link` to a `file://` address is refused by the writer, the map
/// left as it was, while one to `mailto:` applies.
///
/// Controls, seen red: with the load check removed the `file://` map
/// loads; with the validator's check removed the `set_link` applies.
#[test]
fn test_a_link_to_a_file_is_refused_at_load_and_at_apply() {
    let map = |url: &str| {
        format!(
            r#"{{"format":1,"name":"l","nodes":[{{"id":1,"position":[0,0],"sections":[{{"spans":[{{"text":"x","link":{{"url":"{url}"}}}}]}}]}}]}}"#
        )
    };
    let error = load(&map("file:///etc/passwd")).expect_err("a file link is refused");
    assert_eq!(error.path, "nodes[0].sections[0].spans[0].link.url");
    assert_eq!(
        error.kind,
        LoadErrorKind::LinkScheme {
            scheme: "file".into()
        }
    );
    let mut doc = load(&map("https://example.org")).expect("an https link loads");
    let span = doc
        .tree()
        .descendants()
        .find(|&id| doc.get(id).is_some_and(|e| e.kind() == DocKind::Span))
        .expect("a span");
    let set = |doc: &Document, url: &str| {
        use crate::document::mutation::ops::{DocMutation, SpanOp};
        use crate::document::values::LinkTarget;
        let steps = compose::set(
            doc,
            DocKind::Span,
            vec![DocMutation::Span(SpanOp::SetLink(Some(LinkTarget::Url(
                url.into(),
            ))))],
            &[span],
        )
        .expect("composes");
        EditRequest::new(steps, "link")
    };
    let before = doc.save_json().unwrap();
    let refusal = doc
        .apply(set(&doc, "file:///etc/passwd"), ChangeSource::Edit)
        .expect_err("the writer refuses a file link");
    assert!(refusal.to_string().contains("`file`"), "{refusal}");
    assert_eq!(doc.save_json().unwrap(), before, "the map is as it was");
    doc.apply(set(&doc, "mailto:a@example.org"), ChangeSource::Edit)
        .expect("a mailto link applies");
    // An edit that writes one is refused when it is read.
    let edit = r#"{"edits":[{"id":"x","name":"X","reach":"self","target":"spans","mutations":[{"span":{"set_link":{"url":"javascript:alert(1)"}}}]}]}"#;
    let read = crate::document::edit::parse_edits(edit).expect_err("an edit that links a script is refused");
    assert_eq!(read.path, "edits[0].mutations[0]");
}

/// **A `mailto` link names only RFC 6068's headers** (ruling D3.3
/// item 1 on #252, which supersedes D3.2 item 2), wherever a link is
/// checked. Every spelling verification review 5995253466 found past the
/// attachment denylist — a `+`, a NUL, a vertical tab or an NBSP beside
/// the name, an `attach` behind an encoded `&` or a second `?`, an
/// address part hiding a `?` — is refused at load at the link's `url`,
/// naming the header as written (or as it reads decoded, or the address
/// part), and so are `attach` in three spellings, `X-Foo`, `from` and
/// `reply-to`, and `body=a%26b%3Dc`, over-refused because its decoded
/// query reads a header `b`; and (ruling D3.4 item 1) a header in the
/// address part after a `&`, `;`, `,` or `#`, raw or encoded —
/// verification review 5997452252's table — and `a@b;c=d`, over-refused.
/// Plain addresses after a `,` or a `;` load. The six headers load, in any case, with
/// encoded values and a `Q%26A` body, and an `https` address's `attach`
/// is no header. The writer refuses one at apply, the map left as it
/// was, and an edit's or a presentation's `set_link` to one is refused
/// when it is read.
///
/// Ruling D3.5: the query split on `,` and `#` as well (verification
/// review 6001048552's rows, and the over-refused `body=a,b=c` and
/// `subject=x#y=z`), the first part of the address read raw
/// (`mailto:attach=…`), a later address with an encoded `=`
/// (`x@y,a%3Db@c`), a decoded line break outside a `body` and a raw
/// control character anywhere (`LinkMailControl`) are refused; a
/// fragment, `mailto:to=a@b`, `mailto:a%3Db@c` and a `body` with line
/// breaks load.
///
/// Controls, each seen red: `check_link_url` as it was at 3e90f093 (the
/// attachment denylist) loads `attach+`, the first row; as it was at
/// fed68afd (D3.3, the address part checked for `?` alone) loads
/// `a@example.org&attach=`; as it was at 5f8bcfff (D3.4) loads every
/// D3.5 row; the raw address loop skipping its first part loads the four
/// `mailto:attach=…`; without the raw control check, `body=x\ry` and
/// `a\u{1}@…` load; without the query's line-break check, the two
/// `subject=x%0A…` rows load (each failure lists every row it gets
/// wrong); `load_node`
/// accepting `LinkRefusal::MailHeader` loads it too; the validator
/// accepting it applies the `set_link`; `resolve` accepting it reads the
/// edit (and so the presentation, which reads through it).
#[test]
fn test_a_mailto_names_only_rfc_6068_headers_wherever_a_link_is_checked() {
    let map = |url: &str| {
        let url = serde_json::to_string(url).expect("a string");
        format!(
            r#"{{"format":1,"name":"l","nodes":[{{"id":1,"position":[0,0],"sections":[{{"spans":[{{"text":"x","link":{{"url":{url}}}}}]}}]}}]}}"#
        )
    };
    // Every row is read before any is judged, so a failure lists each
    // address the code at hand gets wrong, not only the first.
    let mut wrong: Vec<String> = Vec::new();
    let mut refused = |url: &str, kind: LoadErrorKind| match load(&map(url)) {
        Ok(_) => wrong.push(format!("{url:?} loads")),
        Err(error) if error.path != "nodes[0].sections[0].spans[0].link.url" || error.kind != kind => {
            wrong.push(format!("{url:?}: {} {:?}", error.path, error.kind))
        }
        Err(_) => {}
    };
    for (url, name) in [
        ("mailto:a@example.org?attach+=/etc/passwd", "attach+"),
        ("mailto:a@example.org?+attach=/etc/passwd", "+attach"),
        ("mailto:a@example.org?attach%00=/etc/passwd", "attach%00"),
        ("mailto:a@example.org?attach%0B=/etc/passwd", "attach%0B"),
        ("mailto:a@example.org?attach%C2%A0=/etc/passwd", "attach%C2%A0"),
        ("mailto:a@example.org?subject=x%26attach=/etc/passwd", "attach"),
        ("mailto:a@example.org?subject=x?attach=/etc/passwd", "attach"),
        (
            "mailto:a@example.org%3Fattach=/etc/passwd",
            "a@example.org%3Fattach=/etc/passwd",
        ),
        ("mailto:a@example.org?attach=/etc/passwd", "attach"),
        ("MAILTO:a@example.org?subject=hi&ATTACH=x", "ATTACH"),
        ("mailto:a@example.org?%61ttach=x", "%61ttach"),
        ("mailto:a@example.org?X-Foo=1", "X-Foo"),
        ("mailto:a@example.org?from=b@example.org", "from"),
        ("mailto:a@example.org?reply-to=b@example.org", "reply-to"),
        ("mailto:a@example.org?body=a%26b%3Dc", "b"),
        // Ruling D3.4 item 1: a header in the address part, after a
        // `&`, `;`, `,` or `#`, raw or encoded — verification review
        // 5997452252's table, each of which loaded at fed68afd.
        ("mailto:a@example.org&attach=/etc/passwd", "attach"),
        ("mailto:a@example.org&attach=/etc/passwd?subject=hi", "attach"),
        ("mailto:a@example.org;attach=x", "attach"),
        ("mailto:a@example.org,attach=x", "attach"),
        ("mailto:a@example.org%26attach=x", "a@example.org%26attach"),
        ("mailto:a@example.org%3Battach=x", "a@example.org%3Battach"),
        ("mailto:a@example.org#&attach=x", "attach"),
        ("mailto:a@example.org#attach=x", "attach"),
        // Over-refused: an address part with an `=` that is no header.
        ("mailto:a@example.org;c=d", "c"),
        // Ruling D3.5 item 1: the query split on the address part's
        // separators, `,` and `#` too — verification review 6001048552's
        // rows, each of which loaded at 5f8bcfff — and the over-refusals
        // that come with it.
        ("mailto:a@example.org?subject=hi#attach=/etc/passwd", "attach"),
        ("mailto:a@example.org?body=x#attach=/etc/passwd", "attach"),
        ("mailto:a@example.org?subject=hi%23attach=/etc/passwd", "attach"),
        ("mailto:a@example.org?to=c,attach=/etc/passwd", "attach"),
        ("mailto:a@example.org?to=c%2Cattach=/etc/passwd", "attach"),
        ("mailto:a@example.org?body=a,b=c", "b"),
        ("mailto:a@example.org?subject=x#y=z", "y"),
        // Ruling D3.5 item 2: the first part of the address, read raw.
        ("mailto:attach=/etc/passwd", "attach"),
        ("mailto:%61ttach=/etc/passwd", "%61ttach"),
        ("mailto:attach=/etc/passwd?subject=x", "attach"),
        ("mailto:attach=/etc/passwd&to=c@d", "attach"),
        // Ruling D3.5 item 5: a later address with `=`, encoded or not.
        ("mailto:x@y,a%3Db@c", "a"),
    ] {
        refused(url, LoadErrorKind::LinkMailHeader { name: name.into() });
    }
    // Ruling D3.5 item 3: a decoded line break outside a `body`, and a
    // raw control character anywhere — which nothing else refuses: the
    // format reads a `url` as any JSON string.
    for (url, name) in [
        (
            "mailto:a@example.org?subject=x%0D%0AAttach:%20/etc/passwd",
            "subject",
        ),
        ("mailto:a@example.org?subject=x%0AAttach=/etc/passwd", "subject"),
        ("mailto:a%0D%0Ab@example.org", "a\r\nb@example.org"),
        ("mailto:a@example.org?subject=x\nAttach: /etc/passwd", "subject"),
        ("mailto:a@example.org?body=x\ry", "body"),
        ("mailto:a\u{1}@example.org", "a\u{1}@example.org"),
    ] {
        refused(url, LoadErrorKind::LinkMailControl { name: name.into() });
    }
    assert!(
        wrong.is_empty(),
        "addresses not refused as they should be:\n{}",
        wrong.join("\n")
    );
    for url in [
        "mailto:a@example.org?to=b@example.org&cc=c@example.org&bcc=d@example.org",
        "mailto:a@example.org?subject=attach&body=attachment",
        "mailto:a@example.org?in-reply-to=%3Cid@example.org%3E",
        "mailto:a@example.org?SUBJECT=Hi&Body=x&IN-REPLY-TO=y",
        "mailto:a@example.org?subject=%E2%9C%93+done",
        "mailto:a@example.org?body=Q%26A",
        "mailto:a@example.org,b@example.org",
        "mailto:a@example.org;b@example.org?subject=x",
        "mailto:a@example.org?to=c@example.org#frag",
        "mailto:to=a@example.org",
        "mailto:a%3Db@example.org",
        "mailto:a@example.org?body=line1%0D%0Aline2",
        "https://example.org/?attach=x",
    ] {
        load(&map(url)).unwrap_or_else(|error| panic!("{url} loads: {error}"));
    }
    let mut doc = load(&map("https://example.org")).expect("an https link loads");
    let span = doc
        .tree()
        .descendants()
        .find(|&id| doc.get(id).is_some_and(|e| e.kind() == DocKind::Span))
        .expect("a span");
    let set = |doc: &Document, url: &str| {
        use crate::document::mutation::ops::{DocMutation, SpanOp};
        use crate::document::values::LinkTarget;
        let steps = compose::set(
            doc,
            DocKind::Span,
            vec![DocMutation::Span(SpanOp::SetLink(Some(LinkTarget::Url(
                url.into(),
            ))))],
            &[span],
        )
        .expect("composes");
        EditRequest::new(steps, "link")
    };
    let before = doc.save_json().unwrap();
    let refusal = doc
        .apply(
            set(&doc, "mailto:a@example.org?attach+=/etc/passwd"),
            ChangeSource::Edit,
        )
        .expect_err("the writer refuses a mailto naming another header");
    assert!(
        refusal.to_string().contains("names only the headers") && refusal.to_string().contains("`attach+`"),
        "{refusal}"
    );
    assert_eq!(doc.save_json().unwrap(), before, "the map is as it was");
    doc.apply(
        set(&doc, "mailto:a@example.org?subject=attach"),
        ChangeSource::Edit,
    )
    .expect("a mailto naming only its headers applies");
    let edit = r#"{"edits":[{"id":"x","name":"X","reach":"self","target":"spans","mutations":[{"span":{"set_link":{"url":"mailto:a@example.org?attach+=x"}}}]}]}"#;
    let read =
        crate::document::edit::parse_edits(edit).expect_err("an edit naming another header is refused");
    assert_eq!(read.path, "edits[0].mutations[0]");
    assert!(read.to_string().contains("names only the headers"), "{read}");
    let presentation = r#"{"presentations":[{"id":"x","name":"X","play":"hold","target":"spans","mutations":[{"span":{"set_link":{"url":"mailto:a@example.org?subject=x%26attach=y"}}}]}]}"#;
    let read = crate::document::presentation::load::parse_presentations(presentation)
        .expect_err("a presentation naming another header is refused");
    assert_eq!(read.path, "presentations[0].mutations[0]");
    assert!(read.to_string().contains("names only the headers"), "{read}");
}

/// **Steps run at once, in order, until a `wait`**, which suspends the
/// run on the clock; a frame before the wait is over runs nothing, one at
/// or past it runs the rest, **at the time the wait ends** — a
/// presentation started then starts there, whatever the frame's reading.
/// An inline `verb` step runs its callee's steps in the same run, and
/// comes back.
///
/// Controls, seen red: `run.at_ms` advanced to the frame's reading
/// instead of by the wait starts `p` at 480; a `wait` that does not
/// suspend runs every step at once.
#[test]
fn test_a_run_waits_on_the_clock_and_runs_its_calls_inline() {
    let mut doc = with(
        r#"[
        {"id":"inner","name":"I","steps":[{"present":"h","on":{"nodes":[2]}}]},
        {"id":"outer","name":"O","steps":[{"present":"h","on":{"nodes":[1]}},{"verb":"inner"},{"wait":300},{"present":"p","on":{"nodes":[3]}},{"wait":0},{"stop":"h","on":{"nodes":[1]}}]}
        ]"#,
    );
    let mut runs = Runs::default();
    let mut on = OnTheDocument::new(&mut doc);
    runs.invoke(verb(1), None, false, 100.0, &mut on).expect("runs");
    let calls = |on: &OnTheDocument<'_>| {
        on.calls
            .iter()
            .map(|(what, id, _, at)| format!("{what} {id} @{at}"))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        calls(&on),
        ["start h @100", "start h @100"],
        "up to the wait, the call inline"
    );
    assert!(runs.waiting());
    assert_eq!(runs.next_due_ms(), Some(400.0));
    assert!(!runs.tick(399.0, &mut on), "nothing before the wait is over");
    assert_eq!(on.calls.len(), 2);
    assert!(runs.tick(480.0, &mut on));
    assert_eq!(
        calls(&on),
        ["start h @100", "start h @100", "start p @400", "stop h @400"],
        "the rest, at the time the wait ends"
    );
    assert!(
        !runs.waiting() && runs.runs().next().is_none(),
        "a run that is not held ends"
    );
    let doc = on.doc;
    assert_eq!(playing(doc, "h"), [1, 2]);
    assert_eq!(playing(doc, "p"), [3]);
}

/// **Each target resolves when its step runs**, with a control for each:
/// `self` is the node invoked on, not its children; `children` are the
/// child nodes and not the grandchild; `descendants` are every node
/// below and not `self`; a channel within `self` is the nodes below on
/// that channel and not the ones on another; a channel within the map
/// reaches a top-level node too; `selection` is what is selected when the
/// step runs; and `nodes` acts on each id the map has and skips one it
/// lacks (D27), the others still acted on.
#[test]
fn test_each_target_resolves_when_its_step_runs() {
    let cases: [(&str, Vec<u32>); 8] = [
        (r#""self""#, vec![1]),
        (r#""children""#, vec![2, 3]),
        (r#""descendants""#, vec![2, 4, 3]),
        (r#"{"channel":1,"within":"self"}"#, vec![4, 3]),
        (r#"{"channel":0,"within":"self"}"#, vec![2]),
        (r#"{"channel":1,"within":"map"}"#, vec![4, 3, 5]),
        (r#""selection""#, vec![5]),
        (r#"{"nodes":[5,99,2]}"#, vec![5, 2]),
    ];
    for (on, expected) in cases {
        let mut doc = with(&format!(
            r#"[{{"id":"v","name":"V","steps":[{{"present":"h","on":{on}}}]}}]"#
        ));
        let one = node(&doc, 1);
        let five = node(&doc, 5);
        let mut runs = Runs::default();
        let mut performer = OnTheDocument::new(&mut doc);
        performer.selection = vec![five];
        runs.invoke(verb(0), Some(one), false, 0.0, &mut performer)
            .unwrap_or_else(|error| panic!("{on}: {error}"));
        let reached: Vec<u32> = performer.calls[0]
            .2
            .iter()
            .map(|&n| {
                performer
                    .doc
                    .get(n)
                    .and_then(|e| e.as_node())
                    .expect("a node")
                    .ident
                    .0
            })
            .collect();
        assert_eq!(reached, expected, "{on}");
        assert_eq!(playing(performer.doc, "h"), expected, "{on}: what plays");
    }
}

/// **A verb that acts on `self` is refused without a node**, with
/// nothing run — its own step or a step of a verb it calls; one that
/// names its nodes runs without one.
#[test]
fn test_a_verb_that_needs_a_node_is_refused_without_one() {
    let mut doc = with(
        r#"[{"id":"mine","name":"M","steps":[{"present":"h","on":{"nodes":[1]}},{"present":"h","on":"children"}]},
           {"id":"calls","name":"C","steps":[{"verb":"mine"}]},
           {"id":"named","name":"N","steps":[{"present":"h","on":{"nodes":[1]}}]}]"#,
    );
    let mut runs = Runs::default();
    let mut on = OnTheDocument::new(&mut doc);
    for (n, id) in [(0, "mine"), (1, "calls")] {
        assert_eq!(
            runs.invoke(verb(n), None, false, 0.0, &mut on),
            Err(VerbError::NeedsNode { verb: id.into() })
        );
    }
    assert!(on.calls.is_empty(), "nothing ran");
    runs.invoke(verb(2), None, false, 0.0, &mut on)
        .expect("control: names its nodes");
    assert_eq!(on.calls.len(), 1);
}

/// **An `edit` step is one history entry, recorded as the verb's**
/// (`ChangeSource::Verb`): two edit steps make two entries, each undone
/// on its own; a `present` step makes none.
///
/// Control, seen red: the performer applying under `ChangeSource::Edit`
/// fails the source assertion.
#[test]
fn test_an_edit_step_is_one_entry_with_the_verbs_source() {
    let mut doc = with(
        r#"[{"id":"v","name":"V","steps":[{"present":"h","on":"self"},{"edit":"e","on":"self"},{"edit":"e","on":{"nodes":[5]}}]}]"#,
    );
    let one = node(&doc, 1);
    let at = |doc: &Document, ident: u32| match doc.get(node(doc, ident)) {
        Some(crate::document::element::DocElement::Node(data)) => data.position,
        _ => panic!("a node"),
    };
    let (one_was, five_was) = (at(&doc, 1), at(&doc, 5));
    assert_eq!(doc.history().count(), 0);
    let mut runs = Runs::default();
    runs.invoke(verb(0), Some(one), false, 0.0, &mut OnTheDocument::new(&mut doc))
        .expect("runs");
    let history: Vec<ChangeSource> = doc.history().map(|entry| entry.source).collect();
    assert_eq!(history, [ChangeSource::Verb, ChangeSource::Verb]);
    assert_eq!(at(&doc, 1), one_was + Vec2::new(10.0, 0.0));
    assert_eq!(at(&doc, 5), five_was + Vec2::new(10.0, 0.0));
    doc.undo();
    assert_eq!(
        (at(&doc, 1), at(&doc, 5)),
        (one_was + Vec2::new(10.0, 0.0), five_was)
    );
}

/// **A held run's release** stops every `while` instance it started — it
/// plays its way out — and leaves a `hold` it started playing; released
/// before its `wait` is over, it skips the steps after the wait. A run
/// that is not held cannot be released, and its `while` instance plays
/// on.
///
/// Controls, seen red: a release that does not stop the run's instances
/// leaves `p` going in; a release that lets the run go on starts `h` on
/// node 3 at the wait's end.
#[test]
fn test_a_release_stops_its_while_instances_and_skips_the_rest() {
    let mut doc = with(
        r#"[{"id":"v","name":"V","steps":[{"present":"p","on":"self"},{"present":"h","on":{"nodes":[5]}},{"wait":200},{"present":"h","on":{"nodes":[3]}}]}]"#,
    );
    let one = node(&doc, 1);
    let mut runs = Runs::default();
    let mut on = OnTheDocument::new(&mut doc);
    let held = runs.invoke(verb(0), Some(one), true, 0.0, &mut on).expect("runs");
    on.doc.tick_presentations(50.0);
    let direction = |doc: &Document, id: &str| {
        doc.presentations()
            .find(|instance| instance.presentation == id)
            .map(|instance| instance.direction)
    };
    assert_eq!(direction(on.doc, "p"), Some(Direction::In));
    assert!(runs.release(held, 50.0, &mut on), "the held run is released");
    on.doc.tick_presentations(60.0);
    assert_eq!(
        direction(on.doc, "p"),
        Some(Direction::Out),
        "`p` plays its way out"
    );
    assert_eq!(playing(on.doc, "h"), [5], "the `hold` it started plays on");
    runs.tick(1_000.0, &mut on);
    assert_eq!(playing(on.doc, "h"), [5], "the steps after the wait were skipped");
    assert!(runs.runs().next().is_none());
    // A run that is not held is not released, and its instance holds.
    let free = runs
        .invoke(verb(0), Some(one), false, 2_000.0, &mut on)
        .expect("runs");
    assert!(!runs.release(free, 2_050.0, &mut on));
    on.doc.tick_presentations(2_060.0);
    assert_eq!(direction(on.doc, "p"), Some(Direction::In));
}

/// **The id counter starts above every node id a verb names (D32)**,
/// #252 comment 6525119189: delete node 2, which a verb's target names
/// and which is the highest id, save and reload; a node added then is
/// given 3, never 2, so the verb is not silently retargeted to it.
///
/// Control, seen red: `from_loaded` starting the counter at
/// `first_free_ident(highest, [])` gives the new node id 2.
#[test]
fn test_a_deleted_node_a_verb_names_never_gives_its_id_to_a_new_node() {
    let json = r#"{"format":1,"name":"d32",
        "presentations":[{"id":"h","name":"H","play":"hold","target":"node","mutations":[{"node":{"set_hidden":true}}]}],
        "verbs":[{"id":"v","name":"V","steps":[{"present":"h","on":{"nodes":[2]}}]}],
        "nodes":[{"id":1,"position":[0,0]},{"id":2,"parent":1,"position":[100,0]}]}"#;
    let mut doc = load(json).expect("loads");
    let two = node(&doc, 2);
    let steps = compose::delete(&doc, &[two]).expect("composes");
    doc.apply(EditRequest::new(steps, "delete"), ChangeSource::Edit)
        .expect("an ordinary delete (D27)");
    let mut doc = load(&doc.save_json().expect("saves")).expect("reloads");
    assert!(
        doc.node_by_ident(NodeIdent(2)).is_none(),
        "precondition: node 2 is gone"
    );
    let steps = compose::create_root(&doc, Vec2::new(0.0, 300.0)).expect("composes");
    doc.apply(EditRequest::new(steps, "insert"), ChangeSource::Edit)
        .expect("inserts");
    let new_id = doc
        .tree()
        .descendants()
        .filter_map(|id| doc.get(id).and_then(|e| e.as_node()).map(|n| n.ident))
        .max()
        .expect("nodes");
    assert_ne!(
        new_id,
        NodeIdent(2),
        "the verb would silently retarget to the new node"
    );
    assert_eq!(new_id, NodeIdent(3));
}

/// The node ids `node` names: a helper's own precondition.
#[test]
fn test_the_test_map_is_as_its_docs_say() {
    let doc = with("[]");
    let child = |parent: u32| -> Vec<u32> {
        doc.children(node(&doc, parent))
            .filter_map(|id| doc.get(id).and_then(|e| e.as_node()).map(|n| n.ident.0))
            .collect()
    };
    assert_eq!(child(1), [2, 3]);
    assert_eq!(child(2), [4]);
    let _: NodeId = node(&doc, 5);
}
