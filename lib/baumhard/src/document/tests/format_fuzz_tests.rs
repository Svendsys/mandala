// SPDX-License-Identifier: MPL-2.0

//! The loader's fuzz properties held to account (TEST_CONVENTIONS
//! §T14): each property shown to go red on a planted violation, the
//! structure-aware generator swept under a seeded source until it has
//! reached every case, and every fuzz finding replayed.

use crate::document::element::DocElement;
use crate::document::format::error::{LoadError, LoadErrorKind, NodeContext, TextPosition};
use crate::document::format::{load, save};
use crate::document::ids::{get, Document, IdentIndex};
use crate::document::tests::format_fuzz::{
    begun_nodes, bounded, bounds_hold, check_generated, check_load_bytes, check_loaded, check_refusal,
    derived_holds, excerpt_length, kind_name, node_lines, oracle, path_resolves, reference, render,
    saved_text_is_clean, BoundedField, Fault, MapSpec, NodeSpec, Spelled, BOUNDED_FIELDS,
};
use crate::document::tests::format_support::fixture;
use crate::document::values::{FitMode, FontSize};
use crate::document::writer::Derived;
use crate::util::tests::choices::Choices;

use std::collections::BTreeMap;

/// Every minimized input a fuzz target found, by the symptom it had.
/// Each is replayed through every property here; its own test below
/// pins the symptom. The fuzz targets read these files as seeds.
const FINDINGS: [(&str, &[u8]); 10] = [
    (
        "c1_control_saved_raw",
        include_bytes!("fuzz_findings/c1_control_saved_raw.mandala.json"),
    ),
    (
        "syntax_error_in_a_node_has_no_node_context",
        include_bytes!("fuzz_findings/syntax_error_in_a_node_has_no_node_context.mandala.json"),
    ),
    // A harness finding, not a loader one: the refusal names a place in
    // the first of two `sections` keys, and the path property followed
    // only the last.
    (
        "a_fault_inside_the_first_of_two_keys",
        include_bytes!("fuzz_findings/a_fault_inside_the_first_of_two_keys.mandala.json"),
    ),
    (
        "a_handler_name_shown_raw",
        include_bytes!("fuzz_findings/a_handler_name_shown_raw.mandala.json"),
    ),
    // A projection finding, written by hand from #364's: a map that loads
    // and made the projection panic.
    (
        "a_ring_past_the_body_index_cells",
        include_bytes!("fuzz_findings/a_ring_past_the_body_index_cells.mandala.json"),
    ),
    // A second, written by hand from #364's edges: a map that loaded and
    // made the projection panic at `f32::MAX`, now refused at load.
    (
        "a_path_spaced_past_what_a_box_holds",
        include_bytes!("fuzz_findings/a_path_spaced_past_what_a_box_holds.mandala.json"),
    ),
    // A harness finding (CI's load_raw job on #372, a timeout; #374): a
    // ring the projection's work bound let in, far out past what the cap
    // of a placement bounds the work of.
    (
        "a_ring_too_far_out_to_project_in_time",
        include_bytes!("fuzz_findings/a_ring_too_far_out_to_project_in_time.mandala.json"),
    ),
    // A harness finding of the same kind (CI's load_raw job on #375, a
    // timeout): `a_ring_past_the_body_index_cells` with its dots spaced a
    // tenth as far apart.
    (
        "a_ring_spaced_too_far_out_to_project_in_time",
        include_bytes!("fuzz_findings/a_ring_spaced_too_far_out_to_project_in_time.mandala.json"),
    ),
    // A harness finding (the review of #378, a timeout under ASan): a map
    // the bound admits that the check's own edit takes out of it.
    (
        "a_ring_its_check_makes_repeat",
        include_bytes!("fuzz_findings/a_ring_its_check_makes_repeat.mandala.json"),
    ),
    // A harness finding of the same kind as #375's (CI's load_raw job on
    // #382, a timeout): the dots of `a_ring_past_the_body_index_cells`
    // unspaced, 315 `gap`s out.
    (
        "a_ring_unspaced_too_far_out_to_project_in_time",
        include_bytes!("fuzz_findings/a_ring_unspaced_too_far_out_to_project_in_time.mandala.json"),
    ),
];

/// The fourth finding (CI's load_raw job on #365, minimized): an edit
/// whose handler is `flo` and U+202E, raw in the file, which this build
/// does not have. The refusal named it as the file wrote it, the bidi
/// control raw in a message a terminal or a dialog prints — `InvalidEdit`
/// alone of the refusals that quote the file showed its message as
/// written (since #331). It is shown as file text is now, escaped.
///
/// Control: `InvalidEdit`'s message shown as written, red.
#[test]
fn test_a_handler_name_from_the_file_is_escaped_in_its_refusal() {
    let text = std::str::from_utf8(FINDINGS[3].1).unwrap();
    assert!(
        text.contains("flo\u{202e}"),
        "precondition: the finding names a handler with U+202E raw"
    );
    let error = load(text).unwrap_err();
    assert!(
        matches!(error.kind, LoadErrorKind::InvalidEdit { .. }),
        "{error:?}"
    );
    assert_eq!(error.path, "edits[1].handler");
    let shown = error.to_string();
    assert!(!shown.contains('\u{202e}'), "shown raw: {shown:?}");
    assert!(shown.contains("`flo\\u202e`"), "{shown}");
}

/// The finding of #364: a map that loads made the projection panic,
/// counting the cells of a region too large for the product to fit an
/// `i64` (attempt to multiply with overflow); a release build wrapped the
/// count, took the region for a small one and filed it under every cell.
/// The map is one node whose last ring is 600 `gap`s of 10⁹ out, the most
/// a length is held to wherever the loader bounds one, its dots spaced to
/// run all the way round: a region 1.2·10¹² across. It loads, and holds
/// every property. Since ruling B47.1 its ring is outside the projection's
/// work bound, which holds a frame's outline within its extent, so the
/// loader targets skip its projection; this test projects it itself.
///
/// Controls: with the count the product of two spans in `i64`, as it was,
/// a debug build panics in the first projection here (attempt to multiply
/// with overflow), and a release build's ends in a failed allocation.
#[test]
fn test_a_ring_past_the_body_index_cells_is_projected() {
    use crate::projection::tests::support::Projected;
    let finding = FINDINGS[4].1;
    let doc = load(std::str::from_utf8(finding).unwrap()).expect("the finding loads");
    let node = doc.node_by_ident(crate::document::ids::NodeIdent(1)).unwrap();
    let region = Projected::new(doc).projector.regions.get(node).1;
    let across = region.map(|(rect, _)| rect.size.min_element());
    assert!(
        across.is_some_and(|across| across > 1.0e12),
        "precondition: the ring's box is 600 gaps out each way, {region:?}"
    );
    check_load_bytes(finding).unwrap();
}

/// The finding of #374 (CI's load_raw job on #372, a timeout, minimized
/// by hand: the found map had 434 empty rings, and these 9 lay its ring
/// in counted work within half a percent of it): a map the projection's work
/// bound let in took the loader target past its timeout. One node, a dot
/// ring 9 `gap`s of 10⁹ out with no spacing, laid to the cap of a
/// placement (R41), 65,536 glyphs; that far out an `f32` point is a
/// thousand units from the next, and each glyph read its path a thousand
/// times setting its ink apart from the last, in every projection the
/// check made. The cap bounds a ring's glyphs, not their work, which its
/// budget does, 196,608,000 steps at the cap (ruling B17.2c), past the
/// timeout. The map is outside the bound on both counts: its ring lies
/// past the extent (ruling B47.1: the bound holds a frame's outline within
/// it, as every coordinate), and lays past `MOST_RING_GLYPHS` (ruling
/// B47.3) — so the check skips its projection and counts it skipped. What
/// the densest ring inside the bound takes is held in
/// `projection_fuzz_tests`.
///
/// Control, run and seen red: with `outline_reach` reading no reach,
/// which both counts read, and no `Repeat` ring's work bounded, the map is
/// admitted and its check fails, its ring laying 65,536 glyphs of 123
/// bounded (`rings_within_their_work`). With either count alone taken
/// out, it stays skipped.
#[test]
fn test_a_ring_too_far_out_to_project_in_time_is_skipped() {
    use crate::projection::tests::projection_fuzz::projection_counts;
    let finding = FINDINGS[6].1;
    // The counts are this thread's: exactly what this replay adds.
    let (projected, skipped, cut) = projection_counts();
    check_load_bytes(finding).unwrap();
    let (projected_now, skipped_now, cut_now) = projection_counts();
    assert_eq!(
        (projected_now - projected, skipped_now - skipped, cut_now - cut),
        (0, 1, 0),
        "the finding is skipped, not projected"
    );
}

/// The finding of CI's load_raw job on #375 (a timeout; #374), as found:
/// `a_ring_past_the_body_index_cells` with its `spacing` of 10⁹ mutated to
/// 10⁸, one byte (SHA-1 `7e0ac234`). Its ring, 600 `gap`s of 10⁹ out, lays
/// 37,663 glyphs at 1,499,631 glyph steps a projection, 39.8 a glyph, in
/// every projection the check made. It is not minimized further: only 600
/// `gap`s, the most the loader holds one to, put a ring that far out, and
/// fewer rings lay it nearer, where its glyphs read their path another
/// number of times (9 rings, spaced to lay as many glyphs, take 68.7 a
/// glyph). It was found on a branch with #370's carve-out. The map is
/// outside the bound on both counts, as the first finding's is: its ring
/// lies past the extent (ruling B47.1) and lays past `MOST_RING_GLYPHS`
/// (ruling B47.3), so the check skips its projection and counts it
/// skipped.
///
/// Control, run and seen red: with `outline_reach` reading no reach,
/// which both counts read, the map is admitted and its check fails, its
/// ring laying 37,663 glyphs of 2 bounded (`rings_within_their_work`).
/// With either count alone taken out, it stays skipped.
#[test]
fn test_a_ring_spaced_too_far_out_to_project_in_time_is_skipped() {
    use crate::projection::tests::projection_fuzz::projection_counts;
    let finding = FINDINGS[7].1;
    // The counts are this thread's: exactly what this replay adds.
    let (projected, skipped, cut) = projection_counts();
    check_load_bytes(finding).unwrap();
    let (projected_now, skipped_now, cut_now) = projection_counts();
    assert_eq!(
        (projected_now - projected, skipped_now - skipped, cut_now - cut),
        (0, 1, 0),
        "the finding is skipped, not projected"
    );
}

/// The finding of CI's load_raw job on #382 (a timeout, artifact
/// 14663467799), as found, 1,491 bytes (SHA-1 `293215b2`): one node, 100 by
/// 40, its frame 315 empty rings at a `gap` of 10⁹ and outside them a ring
/// of `·` laid `Repeat` with a `spacing` of `0e9`, none. The empty rings
/// lay nothing and take no line height, so the dots lie 315 `gap`s out,
/// 3.15·10¹¹: laid, they place 65,536 glyphs, the cap, at 4,098,212 glyph
/// steps a projection, 62.5 a glyph, in every projection a check makes.
/// #382's branch, off the epic, has #370's carve-out and not rulings B47.1
/// and B47.3. Here the map is outside the bound on both counts: its ring
/// lies past the extent (ruling B47.1, `outline_reach` reads 3.15·10¹¹) and
/// may lay past `MOST_RING_GLYPHS` (ruling B47.3, `repeat_glyphs` reads the
/// cap), so the check skips its projection and counts it skipped.
///
/// Control, run and seen red: with `outline_reach` reading no reach,
/// which both counts read, and no `Repeat` ring's work bounded, the map is
/// admitted and its check fails, its ring laying 65,536 glyphs of 123
/// bounded (`rings_within_their_work`). With either count alone taken
/// out, it stays skipped.
#[test]
fn test_a_ring_unspaced_too_far_out_to_project_in_time_is_skipped() {
    use crate::projection::tests::projection_fuzz::projection_counts;
    let finding = FINDINGS[9].1;
    // The counts are this thread's: exactly what this replay adds.
    let (projected, skipped, cut) = projection_counts();
    check_load_bytes(finding).unwrap();
    let (projected_now, skipped_now, cut_now) = projection_counts();
    assert_eq!(
        (projected_now - projected, skipped_now - skipped, cut_now - cut),
        (0, 1, 0),
        "the finding is skipped, not projected"
    );
}

/// The finding of the review of #378 (verification review 5997246699, a
/// timeout under ASan, 26.6 s): a map the bound admits — a ring of `·`
/// laid `fit`, by default, 1,300 out round a node 1 across, its frame
/// laid by halves within `FRAME_STEPS_A_LAY` — and 1,095 bare nodes, which
/// set the arena's length the check's choices are seeded by. Those draw a
/// transaction whose first stage is `Ring::SetFit(Repeat)`: the ring laid
/// so is the review's "dots 1,300 out", 52,210 glyphs at 1,907,788 steps a
/// lay, in every projection after it, which the bound never read. The
/// check reads the bound again after each change (`left_the_bound`), so it
/// abandons the map at that stage and counts it apart: none projected,
/// none skipped, none cut short, one left, 1.3 s under ASan. With 1,094
/// bare nodes the seed draws a harmless edit, and the map is projected,
/// 1.9 s under ASan.
///
/// Control, run and seen red: with the bound not read again after an
/// edit, red on the counts, one projected and none left.
#[test]
fn test_a_ring_its_check_makes_repeat_is_abandoned() {
    use crate::projection::tests::projection_fuzz::{projection_counts, projection_left};
    let finding = FINDINGS[8].1;
    // The counts are this thread's: exactly what this replay adds.
    let ((projected, skipped, cut), left) = (projection_counts(), projection_left());
    check_load_bytes(finding).unwrap();
    let ((projected_now, skipped_now, cut_now), left_now) = (projection_counts(), projection_left());
    assert_eq!(
        (
            projected_now - projected,
            skipped_now - skipped,
            cut_now - cut,
            left_now - left
        ),
        (0, 0, 0, 1),
        "the finding is abandoned when its edit takes it out of the bound"
    );
}

/// The second finding of #364: a map that loaded made the projection
/// panic at the far edge of a `spacing`. The connector from node 1 to
/// node 2 is spaced `f32::MAX`, so each of its glyphs' boxes, grown by its
/// gap, had no far corner, and node 3, 1000 away, round and ringed, clamped
/// to no number trying it. A `spacing` is now held to the canvas bound at
/// load, so the map is refused, naming the connector's `spacing` and the
/// bound, and it holds every property of a refusal.
///
/// Control: with a path's `spacing` held only to `check_non_negative` at
/// load, as it was, red on the refusal.
#[test]
fn test_a_path_spaced_past_what_a_box_holds_is_refused() {
    let finding = FINDINGS[5].1;
    let error = load(std::str::from_utf8(finding).unwrap()).expect_err("the finding is refused");
    assert_eq!(
        (error.path.as_str(), &error.kind),
        (
            "nodes[1].connector.spacing",
            &crate::document::format::LoadErrorKind::Bound {
                error: crate::document::bounds::BoundError::Above {
                    value: f32::MAX,
                    max: crate::document::bounds::MAX_CANVAS_COORD,
                },
            }
        )
    );
    check_load_bytes(finding).unwrap();
}

/// The second finding: malformed text inside `nodes[1]` was refused at
/// the path `nodes[1]` with no node context, where every error inside a
/// node carries one (§7.2) — the index pass, which finds every syntax
/// error, never attached it. The node's text is not JSON, so the context
/// holds its index alone.
#[test]
fn test_a_syntax_error_inside_a_node_carries_the_nodes_context() {
    let text = std::str::from_utf8(FINDINGS[1].1).unwrap();
    let error = load(text).unwrap_err();
    assert!(matches!(error.kind, LoadErrorKind::Syntax { .. }), "{error:?}");
    assert_eq!(error.path, "nodes[1]");
    assert_eq!(
        error.node,
        Some(NodeContext {
            index: 1,
            id: None,
            excerpt: None,
        })
    );
    // Control: a syntax error outside the nodes is inside no node.
    let error = load(r#"{"format":1,"name" "m","nodes":[]}"#).unwrap_err();
    assert!(matches!(error.kind, LoadErrorKind::Syntax { .. }), "{error:?}");
    assert_eq!(error.node, None, "{error:?}");
}

/// The review of #293 found the second finding's fix one node too
/// eager: an error between two nodes, or after the last one, was given
/// the path and context of the node after it — `nodes[1]` of a list that
/// holds one. A file cut off after a node is the likeliest corruption a
/// map meets. Such an error is inside no node: its path is the list.
#[test]
fn test_an_error_between_nodes_or_after_the_last_is_inside_none() {
    let head = r#"{"format":1,"name":"m","nodes":[{"id":1,"position":[0,0]}"#;
    for (case, tail) in [
        ("a missing comma", " x]}"),
        ("a file cut off after a node", ""),
        ("a trailing comma", ",]}"),
        ("a file cut off after a comma", ","),
    ] {
        let text = format!("{head}{tail}");
        let error = load(&text).unwrap_err();
        assert!(
            matches!(error.kind, LoadErrorKind::Syntax { .. }),
            "{case}: {error:?}"
        );
        assert_eq!(
            (error.path.as_str(), &error.node),
            ("nodes", &None),
            "{case}: {error:?}"
        );
        check_load_bytes(text.as_bytes()).unwrap_or_else(|why| panic!("{case}: {why}"));
    }
    // Control: a fault inside the second node's text is inside it, from
    // its first byte on.
    for tail in [r#",{"id":2,"position" 0}]}"#, ",x]}"] {
        let error = load(&format!("{head}{tail}")).unwrap_err();
        assert_eq!(error.path, "nodes[1]", "{tail}: {error:?}");
        assert_eq!(error.node.map(|node| node.index), Some(1), "{tail}");
    }
}

#[test]
fn test_begun_nodes_counts_every_node_the_text_begins() {
    assert_eq!(begun_nodes(r#"{"format":1,"nodes":[]}"#), Some(0));
    assert_eq!(begun_nodes(r#"{"nodes":[{"id":1,"t":"]}"},{"id":2}]}"#), Some(2));
    assert_eq!(
        begun_nodes(r#"{"nodes":[{"id":1},{"id":2,"position" 0"#),
        Some(2),
        "a broken node is begun"
    );
    assert_eq!(
        begun_nodes(r#"{"nodes":[{"id":1},"#),
        Some(1),
        "a comma begins nothing"
    );
    assert_eq!(
        begun_nodes(r#"{"nodes":[{"id":1} x"#),
        Some(1),
        "a stray byte where a comma goes begins nothing"
    );
    assert_eq!(
        begun_nodes(r#"{"nodes":[{"id":1},}"#),
        Some(2),
        "where an element goes, a byte that cannot begin a value still begins it"
    );
    assert_eq!(
        begun_nodes(r#"{"name":"nodes","x":{"nodes":[1,2]}}"#),
        None,
        "only the top-level key counts"
    );
    assert_eq!(
        begun_nodes(r#"{"nodes":[1,2,3]}"#),
        Some(3),
        "a key is compared as it decodes"
    );
}

#[test]
fn test_every_loader_fuzz_finding_holds_every_property() {
    assert!(!FINDINGS.is_empty(), "precondition: the findings are read");
    for (name, bytes) in FINDINGS {
        check_load_bytes(bytes).unwrap_or_else(|why| panic!("{name}: {why}"));
    }
}

/// The finding of #278's first run (R23): a C1 control in a map was
/// written raw by the saver, which escaped U+0085 alone of the C1
/// controls. U+009B is CSI in a UTF-8 terminal, so `cat` or `git diff`
/// of such a map could act on the terminal.
#[test]
fn test_a_c1_control_in_a_map_is_saved_escaped() {
    let finding = FINDINGS[0].1;
    let text = std::str::from_utf8(finding).unwrap();
    assert!(
        text.contains('\u{95}'),
        "precondition: the finding holds U+0095 raw"
    );
    let saved = save(&load(text).unwrap()).unwrap();
    assert!(!saved.contains('\u{95}'), "written raw:\n{saved}");
    assert!(saved.contains(r#""content": "\u0095""#), "{saved}");
    // DEL and every C1 control, at both ends of the range and CSI; and,
    // as the control, the first character past them, which stays raw.
    for (c, escape) in [
        ('\u{7f}', "\\u007f"),
        ('\u{80}', "\\u0080"),
        ('\u{9b}', "\\u009b"),
        ('\u{9f}', "\\u009f"),
    ] {
        let map = format!(r#"{{"format":1,"name":"a{c}b"}}"#);
        let saved = save(&load(&map).unwrap()).unwrap();
        assert!(!saved.contains(c) && saved.contains(escape), "{escape}:\n{saved}");
        let reloaded = load(&saved).unwrap();
        assert!(
            matches!(reloaded.get(reloaded.root()), Some(DocElement::Map(map)) if map.name == format!("a{c}b")),
            "{escape} reads back as the character"
        );
    }
    let saved = save(&load("{\"format\":1,\"name\":\"a\u{a0}b\"}").unwrap()).unwrap();
    assert!(
        saved.contains('\u{a0}'),
        "U+00A0 is no control and is written raw:\n{saved}"
    );
}

#[test]
fn test_generated_maps_hold_every_loader_property() {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let (mut reordered, mut tampered, mut neither) = (0, 0, 0);
    for seed in 0..1000 {
        let mut choices = Choices::seeded(seed);
        match check_generated(&mut choices, 64) {
            Ok(generated) => {
                *seen.entry(generated.outcome).or_default() += 1;
                reordered += usize::from(generated.reordered);
                tampered += usize::from(generated.tampered);
                neither += usize::from(!generated.reordered && !generated.tampered);
            }
            Err(why) => panic!("seed {seed}: {why}"),
        }
    }
    // The second key order and the tampered document are a choice's to
    // take: the sweep must have taken each, and skipped both.
    assert!(
        reordered > 0 && tampered > 0 && neither > 0,
        "precondition: the sweep reordered {reordered}, tampered {tampered} and took neither step {neither} times"
    );
    // The sweep proves something only if it reached every case: a valid
    // map of each origin, each injected fault, and each refusal the
    // reference model names.
    for outcome in [
        "none: loaded",
        "structure: loaded",
        "content: Bound",
        "content: Invalid",
        "content: TranslucentBackground",
        "content: UnknownVariable",
        "content: SiblingsOutOfOrder",
        "content: UnknownKey",
        "structure: Bound",
        "structure: ConnectorOnRoot",
        "structure: DanglingLink",
        "structure: DuplicateId",
        "structure: ParentIsSelf",
        "structure: ParentNotEarlier",
        "structure: ParentNotOnChain",
        "structure: SelfLink",
        "structure: SiblingsOutOfOrder",
    ] {
        assert!(
            seen.contains_key(outcome),
            "precondition: the sweep never reached `{outcome}`: {seen:?}"
        );
    }
}

#[test]
fn test_the_saved_text_property_refuses_every_character_that_may_not_stand_raw() {
    for c in [
        '\u{1}', '\u{1b}', '\r', '\u{7f}', '\u{80}', '\u{85}', '\u{9b}', '\u{9f}', '\u{2028}', '\u{2029}',
        '\u{202a}', '\u{202e}', '\u{2066}', '\u{2069}',
    ] {
        let text = format!("{{\"name\": \"a{c}b\"}}\n");
        assert!(
            saved_text_is_clean(&text).is_err(),
            "U+{:04X} passed",
            u32::from(c)
        );
    }
    // Controls: what may stand raw does, and the ending is checked.
    assert_eq!(
        saved_text_is_clean("{\n  \"name\": \"\u{a0}é\u{5d0}\u{1F469}\"\n}\n"),
        Ok(())
    );
    assert!(saved_text_is_clean("{}").is_err(), "no line feed at the end");
    assert!(
        saved_text_is_clean("{}\n\n").is_err(),
        "two line feeds at the end"
    );
}

#[test]
fn test_the_path_property_resolves_exactly_what_the_input_holds() {
    // The text holds `sections` twice in node 2, the first longer: the
    // reader meets a fault in the first before it meets the second key,
    // and its path must resolve (the first load_raw run's harness
    // finding; a `serde_json::Value` keeps the last alone).
    let text = r#"{
        "nodes": [{"id": 1}, {"id": 2, "sections": [{}, 0], "sections": [{"spans": []}]}],
        "palettes": {"a.b": [{"fill": 1}], "\u202e": []}
    }"#;
    let value: Spelled = serde_json::from_str(text).unwrap();
    for path in [
        "",
        "nodes",
        "nodes[1].sections[0].spans",
        "nodes[1].sections[1]",
        r#"palettes["a.b"][0].fill"#,
        "palettes[\"\\u202e\"]",
    ] {
        assert!(path_resolves(&value, path), "`{path}` resolves");
    }
    for path in [
        "nodes[2]",
        "nodes[1].sections[2]",
        "nodes.id",
        "palettes.a.b",
        r#"palettes["a.c"]"#,
        "nodes[",
        ".nodes",
        "nodes[1]..id",
    ] {
        assert!(!path_resolves(&value, path), "`{path}` does not resolve");
    }
}

#[test]
fn test_the_excerpt_property_measures_the_text_not_its_escapes() {
    assert_eq!(
        excerpt_length("a\\u0001\\\"b"),
        Some(4),
        "four characters, escaped"
    );
    let cut = format!("{}…", "a".repeat(256));
    assert_eq!(excerpt_length(&cut), Some(256), "the cut mark is not text");
    assert_eq!(excerpt_length(&"a".repeat(257)), Some(257));
    assert_eq!(
        excerpt_length("a\"b"),
        None,
        "an unescaped quote is not string content"
    );
}

/// A refusal of the fixture `tree` with `path`, `node` and `position`.
fn refusal(path: &str, node: Option<NodeContext>, position: Option<TextPosition>) -> LoadError {
    LoadError {
        path: path.to_string(),
        node,
        position,
        kind: LoadErrorKind::SelfLink,
    }
}

fn context(index: usize, excerpt: Option<String>) -> Option<NodeContext> {
    Some(NodeContext {
        index,
        id: Some(1),
        excerpt,
    })
}

#[test]
fn test_the_refusal_property_finds_each_way_a_refusal_can_be_wrong() {
    let text = fixture("tree");
    let bytes = text.as_bytes();
    let first_line = node_lines(text).expect("the fixture's nodes read")[0].0;
    let line = |line| Some(TextPosition { line, column: 1 });
    // The control: a well-formed refusal at node 0's opening line.
    let good = refusal(
        "nodes[0].links",
        context(0, Some("text".into())),
        line(first_line),
    );
    assert_eq!(
        check_refusal(&good, bytes),
        Ok(()),
        "precondition: the control passes"
    );
    let cases = [
        (refusal("nodes[0].links\u{1b}[2J", context(0, None), None), "raw"),
        (
            refusal("nodes[0].nope", context(0, None), None),
            "does not resolve",
        ),
        (refusal("nodes[1].id", context(0, None), None), "does not agree"),
        (refusal("nodes[0].id", None, None), "does not agree"),
        (refusal("format", context(0, None), None), "does not agree"),
        (
            refusal("nodes[0]", context(0, Some("a".repeat(257))), None),
            "257 bytes",
        ),
        (
            refusal("nodes[0]", context(0, None), line(1)),
            "outside its node's lines",
        ),
    ];
    for (error, why) in cases {
        let found = check_refusal(&error, bytes).expect_err(why);
        assert!(found.contains(why), "{why}: {found}");
    }
    // A context for the node after the last, its path agreeing with it.
    let count = node_lines(text).expect("the fixture's nodes read").len();
    let past = refusal(&format!("nodes[{count}]"), context(count, None), None);
    let found = check_refusal(&past, bytes).expect_err("a node past the last");
    assert!(
        found.contains(&format!("a text that begins {count} nodes")),
        "{found}"
    );
}

/// `doc` with `change` made to its first element `pick` accepts, as a
/// document whose index is rebuilt from its tree.
fn changed(
    doc: &Document,
    pick: impl Fn(&DocElement) -> bool,
    change: impl FnOnce(&mut DocElement),
) -> Document {
    let mut tree = doc.tree().clone();
    let id = tree
        .descendants()
        .find(|id| get(&tree, *id).is_some_and(&pick))
        .expect("precondition: the fixture holds the element");
    tree.update(id, change).unwrap();
    let mut highest = 0;
    let index: IdentIndex = tree
        .descendants()
        .filter_map(|id| match get(&tree, id) {
            Some(DocElement::Node(node)) => {
                highest = highest.max(node.ident.0);
                Some((node.ident, id))
            }
            _ => None,
        })
        .collect();
    let derived = Derived::scan(&tree, doc.definitions());
    Document::from_loaded(tree, doc.definitions().clone(), index, highest, derived).unwrap()
}

/// The derived-state property holds on a loaded fixture with links,
/// and catches each way the loader's gathering could drift from the
/// tree: a link left out of the index, one element miscounted, one byte
/// of text miscounted.
#[test]
fn test_the_derived_property_finds_what_the_loader_miscounted() {
    let doc = load(fixture("full")).unwrap();
    assert_eq!(derived_holds(&doc), Ok(()));
    assert!(
        !doc.derived.incoming.is_empty(),
        "precondition: the fixture has links"
    );
    let mut unlinked = doc.clone();
    unlinked.derived.incoming.clear();
    let mut miscounted = doc.clone();
    miscounted.derived.budget.elements += 1;
    let mut misread = doc.clone();
    misread.derived.budget.text -= 1;
    for (name, drifted) in [("index", unlinked), ("elements", miscounted), ("text", misread)] {
        assert!(
            derived_holds(&drifted).is_err(),
            "{name}: the property sees the drift"
        );
    }
}

#[test]
fn test_the_bounds_property_finds_a_value_past_its_bound_in_each_kind_of_field() {
    let doc = load(fixture("full")).unwrap();
    assert_eq!(
        bounds_hold(&doc),
        Ok(()),
        "precondition: the fixture is in bounds"
    );
    let node = |e: &DocElement| matches!(e, DocElement::Node(_));
    let span = |e: &DocElement| matches!(e, DocElement::Span(_));
    let link = |e: &DocElement| matches!(e, DocElement::Link(_));
    let defaults = |e: &DocElement| matches!(e, DocElement::Defaults(_));
    let cases: Vec<(&str, Document)> = vec![
        (
            "opacity",
            changed(&doc, node, |e| {
                if let DocElement::Node(n) = e {
                    n.opacity = 1.5;
                }
            }),
        ),
        (
            "position",
            changed(&doc, node, |e| {
                if let DocElement::Node(n) = e {
                    n.position = glam::Vec2::new(0.0, f32::INFINITY);
                }
            }),
        ),
        (
            "size",
            changed(&doc, span, |e| {
                if let DocElement::Span(s) = e {
                    s.style.size = Some(FontSize::Pt(0.1));
                }
            }),
        ),
        (
            "control_points",
            changed(&doc, link, |e| {
                if let DocElement::Link(l) = e {
                    l.path.control_points = vec![glam::Vec2::new(2.0e9, 0.0)];
                }
            }),
        ),
        (
            "gap",
            changed(&doc, defaults, |e| {
                if let DocElement::Defaults(d) = e {
                    d.frame.gap = Some(-1.0);
                }
            }),
        ),
        (
            "fit",
            changed(&doc, defaults, |e| {
                if let DocElement::Defaults(d) = e {
                    d.link.fit = Some(FitMode::Fit { min: 0.0, max: 1.0 });
                }
            }),
        ),
    ];
    for (field, changed) in cases {
        let found = bounds_hold(&changed).expect_err(field);
        assert!(found.contains(&format!("`{field}`")), "{field}: {found}");
    }
}

/// A node of the model: `id` under `parent`.
fn spec(id: i64, parent: Option<i64>, channel: i64) -> NodeSpec {
    NodeSpec {
        id,
        parent,
        channel,
        links: Vec::new(),
        content: Vec::new(),
        link_tails: Vec::new(),
    }
}

/// `node` with one more key, as JSON text.
fn with(mut node: NodeSpec, key: &str, value: &str) -> NodeSpec {
    node.content.push((key.to_string(), value.to_string()));
    node
}

/// `node` with a line link to `to`.
fn linking(mut node: NodeSpec, to: i64) -> NodeSpec {
    node.links.push(to);
    node.link_tails.push(String::new());
    node
}

/// A named case of `chain_ref.py`: its name, its nodes, and the
/// refusal both models expect (`None`: it loads).
type NamedCase = (&'static str, Vec<NodeSpec>, Option<(&'static str, &'static str)>);

/// `chain_ref.py`'s named cases, and the three this port adds to its
/// model (a connector on a top-level node, a link to an absent id, a
/// link to its own node), against both models and the loader: the
/// oracle's verdict, the reference's refusal, and the loader's on the
/// rendered text.
#[test]
fn test_the_structure_models_judge_the_named_cases_of_chain_ref() {
    let connector = r#"{"spans": [{"text": "c"}]}"#;
    let cases: [NamedCase; 13] = [
        (
            "valid",
            vec![spec(1, None, 0), spec(2, Some(1), 0), spec(3, Some(1), 1)],
            None,
        ),
        (
            "self parent",
            vec![spec(1, None, 0), spec(2, Some(2), 0)],
            Some(("nodes[1].parent", "ParentIsSelf")),
        ),
        (
            "forward parent",
            vec![spec(1, None, 0), spec(2, Some(3), 0), spec(3, Some(1), 0)],
            Some(("nodes[1].parent", "ParentNotEarlier")),
        ),
        (
            "closed subtree",
            vec![
                spec(1, None, 0),
                spec(2, Some(1), 0),
                spec(3, Some(1), 0),
                spec(4, Some(2), 0),
            ],
            Some(("nodes[3].parent", "ParentNotOnChain")),
        ),
        (
            "channel order",
            vec![spec(1, None, 0), spec(2, Some(1), 1), spec(3, Some(1), 0)],
            Some(("nodes[2]", "SiblingsOutOfOrder")),
        ),
        (
            "root channel order",
            vec![spec(1, None, 2), spec(2, None, 1)],
            Some(("nodes[1].channel", "SiblingsOutOfOrder")),
        ),
        (
            "duplicate",
            vec![spec(1, None, 0), spec(2, Some(1), 0), spec(2, Some(1), 0)],
            Some(("nodes[2].id", "DuplicateId")),
        ),
        (
            "cycle attempt",
            vec![spec(1, Some(2), 0), spec(2, Some(1), 0)],
            Some(("nodes[0].parent", "ParentNotEarlier")),
        ),
        (
            "connector on a top-level node",
            vec![spec(1, None, 0), with(spec(2, None, 0), "connector", connector)],
            Some(("nodes[1].connector", "ConnectorOnRoot")),
        ),
        (
            "connector on a child",
            vec![
                spec(1, None, 0),
                with(spec(2, Some(1), 0), "connector", connector),
            ],
            None,
        ),
        (
            "dangling link",
            vec![spec(1, None, 0), linking(spec(2, Some(1), 0), 7)],
            Some(("nodes[1].links[0].to", "DanglingLink")),
        ),
        (
            "self link",
            vec![spec(1, None, 0), linking(spec(2, Some(1), 0), 2)],
            Some(("nodes[1].links[0].to", "SelfLink")),
        ),
        (
            "link forward and back",
            vec![linking(spec(1, None, 0), 2), linking(spec(2, Some(1), 0), 1)],
            None,
        ),
    ];
    for (name, nodes, expected) in cases {
        assert_eq!(oracle(&nodes), expected.is_none(), "{name}: the oracle");
        let got = reference(&nodes).map(|(path, kind, _)| (path, kind));
        assert_eq!(
            got,
            expected.map(|(path, kind)| (path.to_string(), kind)),
            "{name}: the reference"
        );
        let text = render(
            &MapSpec {
                nodes,
                fault: Fault::Structure,
                bounded: None,
                defaults: Vec::new(),
                background: None,
            },
            0,
        );
        let loaded = load(&text)
            .err()
            .map(|error| (error.path.clone(), kind_name(&error.kind)));
        assert_eq!(
            loaded,
            expected.map(|(path, kind)| (path.to_string(), kind.to_string())),
            "{name}: the loader, on\n{text}"
        );
    }
}

/// The map [`bounded`] writes one field into: a top-level node and a
/// child of it, the field in the child, which can hold every field.
fn bounded_map(choices: &mut Choices<'_>) -> (MapSpec, &'static BoundedField) {
    let mut nodes = vec![spec(1, None, 0), spec(2, Some(1), 0)];
    let mut defaults = Vec::new();
    let (fault, field) = bounded(choices, &mut nodes, &mut defaults, 1);
    (
        MapSpec {
            nodes,
            fault,
            bounded: Some(field.name),
            defaults,
            background: None,
        },
        field,
    )
}

/// Every bounded field of `format/schema.md`'s table below the header's
/// counts — of a node and what it holds, and of each style of the
/// header's `defaults` — at its bound,
/// just past it, just inside it and anywhere: the loader refuses a value
/// past the bound at the field's path as `Bound`, and reads one inside
/// it into a document that holds every property — the saver's float
/// writer round-tripping it included. The sweep proves something only
/// if it put every field both inside its bound and outside it.
#[test]
fn test_every_bounded_field_is_refused_past_its_bound_and_read_inside_it() {
    let mut reached: BTreeMap<(&'static str, bool), usize> = BTreeMap::new();
    for seed in 0..4000 {
        let mut choices = Choices::seeded(seed);
        let (map, field) = bounded_map(&mut choices);
        let text = render(&map, 0);
        let result = load(&text);
        match (&map.fault, &result) {
            (Fault::None, Ok(doc)) => {
                if let Err(why) = check_loaded(doc) {
                    panic!("seed {seed}, {}: {why}\n{text}", field.name);
                }
            }
            (Fault::Content { path, kind, .. }, Err(error))
                if error.path == *path && kind_name(&error.kind) == *kind =>
            {
                if let Err(why) = check_refusal(error, text.as_bytes()) {
                    panic!("seed {seed}, {}: {why}\n{text}", field.name);
                }
            }
            (fault, got) => panic!(
                "seed {seed}, {}: expected {fault:?}, the loader answered {:?}\n{text}",
                field.name,
                got.as_ref().err()
            ),
        }
        *reached.entry((field.name, map.fault == Fault::None)).or_default() += 1;
    }
    for field in &BOUNDED_FIELDS {
        for holds in [true, false] {
            assert!(
                reached.contains_key(&(field.name, holds)),
                "precondition: the sweep never put `{}` {} its bound",
                field.name,
                if holds { "inside" } else { "outside" }
            );
        }
    }
}
