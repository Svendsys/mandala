// SPDX-License-Identifier: MPL-2.0

//! `set_section_text_preserving_runs` — the `section text "<text>"
//! runs=preserve` path — read through what the canvas draws, not
//! only through the run table (#197). A section with non-empty
//! `text_runs` draws only the graphemes its runs cover, so a run
//! table that looks plausible can still hide text; every test here
//! asserts on [`drawn_text`], the section area the tree builder
//! actually produces, run through `displayed_text`.
//!
//! Part of the tests split for `document`. Helpers live in
//! `tests_common`; only the tests for this theme live here.
use super::defaults::default_text_run;
use super::tests_common::pinned_two_section_node;
use super::*;

use baumhard::font::attrs::displayed_text;
use baumhard::mindmap::model::TextRun;
use baumhard::mindmap::tree_builder::build_mindmap_tree;
use baumhard::util::grapheme_chad::count_grapheme_clusters;

/// What section `idx` of `node_id` draws: the tree builder's
/// section area, through the same coverage rule the renderer's
/// span bridge applies.
fn drawn_text(doc: &MindMapDocument, node_id: &str, idx: usize) -> String {
    let tree = build_mindmap_tree(&doc.mindmap);
    let arena_id = tree
        .section_arena_id(node_id, idx)
        .expect("section is in the tree");
    let area = tree
        .tree
        .arena()
        .get(arena_id)
        .expect("arena node")
        .get()
        .glyph_area()
        .expect("a section is a glyph area");
    displayed_text(&area.text, &area.regions)
}

/// A run over `[start, end)` that differs from the authoring
/// defaults in `color` and `bold`, so a test can tell which run a
/// grapheme took its style from.
fn run(start: usize, end: usize, color: &str, bold: bool) -> TextRun {
    TextRun {
        start,
        end,
        bold,
        color: color.into(),
        ..default_text_run(0)
    }
}

/// Section 1 of the pinned fixture, rewritten to `text` / `runs`
/// with no undo entry, so the edit under test is the only one.
fn seeded(text: &str, runs: Vec<TextRun>) -> (MindMapDocument, String) {
    let (mut doc, id) = pinned_two_section_node();
    let section = &mut doc.mindmap.nodes.get_mut(&id).unwrap().sections[1];
    section.text = text.into();
    section.text_runs = runs;
    (doc, id)
}

fn runs_of(doc: &MindMapDocument, id: &str) -> Vec<TextRun> {
    doc.mindmap.nodes.get(id).unwrap().sections[1].text_runs.clone()
}

/// The issue's reproduction. A run over the whole of "Hello"
/// reaches the old end, so it is extended over " world" and the
/// whole new text draws, bold. The control is the table the code
/// used to leave — the same text under the unextended run — which
/// draws only "Hello" through the same reader, so `drawn_text` can
/// tell the two apart.
#[test]
fn test_preserve_extends_a_run_reaching_the_old_end_over_appended_text() {
    let (mut doc, id) = seeded("Hello", vec![run(0, 5, "#ff0000", true)]);
    assert!(doc.set_section_text_preserving_runs(&id, 1, "Hello world".into()));
    assert_eq!(drawn_text(&doc, &id, 1), "Hello world");
    assert_eq!(runs_of(&doc, &id), vec![run(0, 11, "#ff0000", true)]);

    let (control, cid) = seeded("Hello world", vec![run(0, 5, "#ff0000", true)]);
    assert_eq!(
        drawn_text(&control, &cid, 1),
        "Hello",
        "control: the clipped-only table hides the appended graphemes"
    );
}

/// With several runs, only the last one carries on; the earlier
/// runs keep their ranges and their styles.
#[test]
fn test_preserve_appended_text_takes_the_last_runs_style() {
    let (mut doc, id) = seeded(
        "abcde",
        vec![run(0, 3, "#ff0000", true), run(3, 5, "#00ff00", false)],
    );
    assert!(doc.set_section_text_preserving_runs(&id, 1, "abcdefg".into()));
    assert_eq!(drawn_text(&doc, &id, 1), "abcdefg");
    assert_eq!(
        runs_of(&doc, &id),
        vec![run(0, 3, "#ff0000", true), run(3, 7, "#00ff00", false)],
        "the last run is the one extended; the first is untouched"
    );
}

/// A last run ending short of the old end leaves the graphemes
/// after it hidden, and the edit keeps them hidden: it preserves
/// the table and covers only what it appended, with a copy of the
/// last run. The drawn text is therefore "Hello!" — " world" was
/// not on the canvas before the edit and is not after it.
#[test]
fn test_preserve_appends_a_copy_when_the_last_run_ends_short_of_the_old_end() {
    let (mut doc, id) = seeded("Hello world", vec![run(0, 5, "#ff0000", true)]);
    assert_eq!(
        drawn_text(&doc, &id, 1),
        "Hello",
        "before: \" world\" is uncovered"
    );
    assert!(doc.set_section_text_preserving_runs(&id, 1, "Hello world!".into()));
    assert_eq!(drawn_text(&doc, &id, 1), "Hello!");
    assert_eq!(
        runs_of(&doc, &id),
        vec![run(0, 5, "#ff0000", true), run(11, 12, "#ff0000", true)]
    );
}

/// The tail is counted in grapheme clusters: a ZWJ family, a
/// combining mark and a flag are one cluster each, and every byte
/// of them draws. A count in chars or bytes would end the run
/// mid-cluster or past the text.
#[test]
fn test_preserve_covers_an_appended_tail_of_multi_codepoint_clusters() {
    let (mut doc, id) = seeded("Hi", vec![run(0, 2, "#ff0000", true)]);
    let grown = "Hi \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} e\u{0301} \u{1F1EF}\u{1F1F5}";
    assert!(doc.set_section_text_preserving_runs(&id, 1, grown.into()));
    assert_eq!(drawn_text(&doc, &id, 1), grown);
    assert_eq!(count_grapheme_clusters(grown), 8);
    assert_eq!(runs_of(&doc, &id), vec![run(0, 8, "#ff0000", true)]);
}

/// Same length: the table is left exactly as it was, including a
/// trailing grapheme no run covers. Nothing was appended, so
/// nothing is covered.
#[test]
fn test_preserve_same_length_leaves_the_table_untouched() {
    let runs = vec![run(0, 2, "#ff0000", true), run(2, 4, "#00ff00", false)];
    let (mut doc, id) = seeded("abcde", runs.clone());
    assert!(doc.set_section_text_preserving_runs(&id, 1, "ABCDE".into()));
    assert_eq!(runs_of(&doc, &id), runs);
    assert_eq!(drawn_text(&doc, &id, 1), "ABCD");
}

/// Shorter text: the clip is unchanged — a straddling run ends at
/// the new end, a run past it drops — and the clipped text draws.
#[test]
fn test_preserve_shorter_text_clips_and_draws_what_remains() {
    let (mut doc, id) = seeded(
        "abcdef",
        vec![run(0, 3, "#ff0000", true), run(3, 6, "#00ff00", false)],
    );
    assert!(doc.set_section_text_preserving_runs(&id, 1, "ABCD".into()));
    assert_eq!(
        runs_of(&doc, &id),
        vec![run(0, 3, "#ff0000", true), run(3, 4, "#00ff00", false)]
    );
    assert_eq!(drawn_text(&doc, &id, 1), "ABCD");

    assert!(doc.set_section_text_preserving_runs(&id, 1, "AB".into()));
    assert_eq!(runs_of(&doc, &id), vec![run(0, 2, "#ff0000", true)]);
    assert_eq!(drawn_text(&doc, &id, 1), "AB");
}

/// A run-less section draws its whole text at the cascade
/// defaults, so growing it needs no run and gets none.
#[test]
fn test_preserve_leaves_a_run_less_section_run_less() {
    let (mut doc, id) = seeded("Hello", Vec::new());
    assert!(doc.set_section_text_preserving_runs(&id, 1, "Hello world".into()));
    assert!(runs_of(&doc, &id).is_empty());
    assert_eq!(drawn_text(&doc, &id, 1), "Hello world");
}

/// §T1: the grow is one `EditNodeText` entry, and one undo restores
/// the text, the run table and what the section draws, byte for
/// byte. The control is the state between the edit and the undo,
/// which differs from the restored one on all three.
#[test]
fn test_preserve_grow_undo_restores_text_runs_and_drawing() {
    let before_runs = vec![run(0, 3, "#ff0000", true), run(3, 5, "#00ff00", false)];
    let (mut doc, id) = seeded("Hello", before_runs.clone());
    let depth = doc.undo_stack.len();
    assert!(doc.set_section_text_preserving_runs(&id, 1, "Hello world".into()));
    assert_eq!(doc.undo_stack.len(), depth + 1);
    assert!(matches!(
        doc.undo_stack.last(),
        Some(UndoAction::EditNodeText { .. })
    ));
    let section = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
    assert_eq!(section.text, "Hello world");
    assert_ne!(section.text_runs, before_runs);
    assert_eq!(drawn_text(&doc, &id, 1), "Hello world");

    assert!(doc.undo());
    assert_eq!(doc.undo_stack.len(), depth);
    let section = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
    assert_eq!(section.text, "Hello");
    assert_eq!(section.text_runs, before_runs);
    assert_eq!(drawn_text(&doc, &id, 1), "Hello");
}

/// A table that overshoots its text (a run ending past the last
/// grapheme, which a tree-walker shortening can leave behind) is
/// clamped to the old text and then extended, rather than given a
/// copy starting at the old end, which would overlap the run it was
/// copied from.
#[test]
fn test_preserve_extends_a_run_that_overshot_the_old_text() {
    let (mut doc, id) = seeded("abc", vec![run(0, 5, "#ff0000", true)]);
    assert!(doc.set_section_text_preserving_runs(&id, 1, "abcdefg".into()));
    assert_eq!(runs_of(&doc, &id), vec![run(0, 7, "#ff0000", true)]);
    assert_eq!(drawn_text(&doc, &id, 1), "abcdefg");
}

/// A last run whose *start* overshot the old text is clamped away
/// before the tail is covered, so every appended grapheme draws:
/// `[4,6)` on "abc" covers nothing the author saw, and the tail
/// takes the style of `[0,1)`, the last run that did. "b" and "c"
/// were uncovered before the edit and stay so.
#[test]
fn test_preserve_covers_the_tail_when_the_last_run_started_past_the_old_text() {
    let (mut doc, id) = seeded(
        "abc",
        vec![run(0, 1, "#ff0000", true), run(4, 6, "#00ff00", false)],
    );
    assert!(doc.set_section_text_preserving_runs(&id, 1, "abcdefg".into()));
    assert_eq!(drawn_text(&doc, &id, 1), "adefg");
    assert_eq!(
        runs_of(&doc, &id),
        vec![run(0, 1, "#ff0000", true), run(3, 7, "#ff0000", true)]
    );
}

/// The appended graphemes take the last run's style but not its
/// link: a linked run keeps its range, and the tail is a copy of it
/// with `hyperlink: None`. The control is the same edit on the
/// unlinked run, which is extended instead.
#[test]
fn test_preserve_does_not_carry_a_hyperlink_onto_appended_text() {
    let linked = TextRun {
        hyperlink: Some("https://example.com".into()),
        ..run(0, 5, "#ff0000", true)
    };
    let (mut doc, id) = seeded("Hello", vec![linked.clone()]);
    assert!(doc.set_section_text_preserving_runs(&id, 1, "Hello world".into()));
    assert_eq!(drawn_text(&doc, &id, 1), "Hello world");
    assert_eq!(runs_of(&doc, &id), vec![linked, run(5, 11, "#ff0000", true)]);

    let (mut control, cid) = seeded("Hello", vec![run(0, 5, "#ff0000", true)]);
    assert!(control.set_section_text_preserving_runs(&cid, 1, "Hello world".into()));
    assert_eq!(runs_of(&control, &cid), vec![run(0, 11, "#ff0000", true)]);
}

/// When every run sat past the old text, the clamp leaves none and
/// the section becomes run-less, drawing its whole text — as a
/// shorter edit that drops every run already does. The control is
/// the drawing before the edit, which shows nothing.
#[test]
fn test_preserve_grow_leaves_a_section_run_less_when_every_run_overshot() {
    let (mut doc, id) = seeded("abc", vec![run(4, 6, "#00ff00", false)]);
    assert_eq!(drawn_text(&doc, &id, 1), "", "before: no run covers \"abc\"");
    assert!(doc.set_section_text_preserving_runs(&id, 1, "abcdefg".into()));
    assert!(runs_of(&doc, &id).is_empty());
    assert_eq!(drawn_text(&doc, &id, 1), "abcdefg");
}
