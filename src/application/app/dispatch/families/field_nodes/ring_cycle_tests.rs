// SPDX-License-Identifier: MPL-2.0

//! The frame cycle sets whole boxes (#338): `CycleFrameContent` gives a
//! frame's first ring its spans, `vertical` and `corners` together —
//! light `─ │ ┌ ┐ ┘ └`, heavy `━ ┃ ┏ ┓ ┛ ┗`, double `═ ║ ╔ ╗ ╝ ╚` — and
//! `frame ring <i> text=` takes a box's `vertical` and `corners` along
//! with its text.
//!
//! **The expectations are written by hand**, as the ring each step must
//! save to, never read back from the cycle's own table (TEST_CONVENTIONS
//! §T13 rule 2). Each step is checked three ways: the saved document is
//! exactly the expected text, that text loads and saves back to itself
//! byte for byte, and one undo gives back the document before the step,
//! byte for byte.

use baumhard::document::format;
use baumhard::document::ids::NodeIdent;
use baumhard::document::writer::DEFAULT_HISTORY_LIMIT;

use super::run;
use super::tests::run_line;
use crate::application::app::dispatch::cross_dispatch::DispatchOutcome;
use crate::application::document::MindMapDocument;
use crate::application::keybinds::Action;

/// Five nodes: one with no frame; one with the light ring the cycle made
/// before #338 (spans alone); one with an author's heavy ring whose
/// corners are diamonds, sized and colored; one with a frame of no ring;
/// and one with a frame of two rings, a heavy ring of spans alone inside
/// the light box.
const FIXTURE: &str = r##"{
  "format": 1,
  "name": "Ring cycle",
  "nodes": [
    {
      "id": 1,
      "position": [0.0, 0.0],
      "sections": [{"spans": [{"text": "bare"}]}]
    },
    {
      "id": 2,
      "position": [300.0, 0.0],
      "sections": [{"spans": [{"text": "light"}]}],
      "frame": {"fit": "repeat", "rings": [{"spans": [{"text": "─"}]}]}
    },
    {
      "id": 3,
      "position": [0.0, 300.0],
      "sections": [{"spans": [{"text": "author"}]}],
      "frame": {"inset": 9.0, "rings": [{"fit": "repeat", "size": 0.16, "color": "#ff0000", "spans": [{"text": "━"}], "vertical": "┃", "corners": ["◆", "◆", "◆", "◆"]}]}
    },
    {
      "id": 4,
      "position": [300.0, 300.0],
      "sections": [{"spans": [{"text": "ringless"}]}],
      "frame": {"fit": "repeat"}
    },
    {
      "id": 5,
      "position": [600.0, 0.0],
      "sections": [{"spans": [{"text": "two rings"}]}],
      "frame": {"fit": "repeat", "rings": [{"spans": [{"text": "━"}]}, {"spans": [{"text": "─"}], "vertical": "│", "corners": ["┌", "┐", "┘", "└"]}]}
    }
  ]
}
"##;

/// Node 5's two rings, as the fixture spells them: a heavy ring of
/// spans alone, then the light box.
const N5_RINGS: &str = r#""rings": [{"spans": [{"text": "━"}]}, {"spans": [{"text": "─"}], "vertical": "│", "corners": ["┌", "┐", "┘", "└"]}]"#;

/// Node 2's frame, as the fixture spells it.
const N2_FRAME: &str = r#""frame": {"fit": "repeat", "rings": [{"spans": [{"text": "─"}]}]}"#;
/// Node 3's ring, as the fixture spells it.
const N3_RING: &str = r##"{"fit": "repeat", "size": 0.16, "color": "#ff0000", "spans": [{"text": "━"}], "vertical": "┃", "corners": ["◆", "◆", "◆", "◆"]}"##;

/// Node 2's frame holding a ring of exactly these three keys, spelled by
/// hand as the saver writes them.
fn n2_frame(level: &str, vertical: &str, corners: [&str; 4]) -> String {
    let [a, b, c, d] = corners;
    format!(
        "\"frame\": {{\"fit\": \"repeat\", \"rings\": [{{\"spans\": [{{\"text\": \"{level}\"}}], \"vertical\": \"{vertical}\", \"corners\": [\"{a}\", \"{b}\", \"{c}\", \"{d}\"]}}]}}"
    )
}

fn fixture() -> MindMapDocument {
    MindMapDocument::from_json_str(FIXTURE, None, DEFAULT_HISTORY_LIMIT).expect("the fixture loads")
}

fn bytes(doc: &MindMapDocument) -> String {
    format::save(doc.doc()).expect("saves")
}

fn select(doc: &mut MindMapDocument, ident: u32) {
    let node = doc.doc().node_by_ident(NodeIdent(ident)).expect("the node");
    doc.selection.select_only(node);
}

/// `FIXTURE` with each `(old, new)` replaced, each `old` occurring once.
fn edited(edits: &[(&str, &str)]) -> String {
    let mut text = FIXTURE.to_string();
    for (old, new) in edits {
        assert_eq!(
            text.matches(old).count(),
            1,
            "an expectation's {old:?} is not unique"
        );
        text = text.replacen(old, new, 1);
    }
    text
}

/// Run `step` on `doc` and hold what it made: exactly `want`, as one
/// history entry, a text that loads and saves back to itself, and one
/// undo back to the bytes before it (after which the step is redone, so
/// the caller goes on from `want`).
fn holds(doc: &mut MindMapDocument, what: &str, want: &str, step: impl Fn(&mut MindMapDocument)) {
    let before = bytes(doc);
    let entries = doc.doc().history().count();
    step(doc);
    assert_eq!(bytes(doc), want, "{what}: the saved document");
    assert_eq!(doc.doc().history().count(), entries + 1, "{what}: one entry");
    let reloaded = MindMapDocument::from_json_str(want, None, DEFAULT_HISTORY_LIMIT)
        .unwrap_or_else(|e| panic!("{what}: the saved document loads: {e:?}"));
    assert_eq!(bytes(&reloaded), want, "{what}: it round-trips byte for byte");
    doc.undo();
    assert_eq!(
        bytes(doc),
        before,
        "{what}: one undo gives back the ring before it, whole"
    );
    doc.redo();
    assert_eq!(bytes(doc), want, "{what}: and redo the step");
}

fn cycle(doc: &mut MindMapDocument) {
    assert_eq!(run(&Action::CycleFrameContent, doc), DispatchOutcome::Handled);
}

/// **Each step of the cycle sets the whole box, and leaves nothing of the
/// box before it.** A ring of spans alone goes to the heavy box, then the
/// double, then the light, then the heavy again: each time the spans,
/// `vertical` and all four `corners` are the new box's and nothing else,
/// the result round-trips, and undo gives back the box before it whole.
///
/// Controls (each seen red, then reverted): the cycle setting spans
/// alone, as before #338 — red at "heavy: the saved document"; setting
/// `vertical` and not `corners` — red at the same; the double box's
/// `vertical` set to the heavy `┃` — red at "double"; a journal's revert
/// skipping its newest entry — red at "heavy: one undo gives back the
/// ring before it, whole". The round trip's control is on the next test.
#[test]
fn test_the_frame_cycle_sets_each_box_whole_and_leaves_none_behind() {
    let mut doc = fixture();
    assert_eq!(
        bytes(&doc),
        FIXTURE,
        "precondition: the fixture is in the saver's spelling"
    );
    select(&mut doc, 2);
    let heavy = edited(&[(N2_FRAME, &n2_frame("━", "┃", ["┏", "┓", "┛", "┗"]))]);
    let double = edited(&[(N2_FRAME, &n2_frame("═", "║", ["╔", "╗", "╝", "╚"]))]);
    let light = edited(&[(N2_FRAME, &n2_frame("─", "│", ["┌", "┐", "┘", "└"]))]);
    holds(&mut doc, "heavy", &heavy, cycle);
    holds(&mut doc, "double", &double, cycle);
    holds(&mut doc, "light", &light, cycle);
    holds(&mut doc, "heavy again", &heavy, cycle);
}

/// **The cycle replaces an author's own `vertical` and `corners` with
/// the box's, and keeps the ring's other keys; undo gives the author's
/// back.** Node 3's heavy ring with diamond corners steps to the double
/// box: its size, color and fit stay, its diamonds go.
///
/// Controls: the cycle keeping a ring's corners when it has its own —
/// red at "the author's ring: the saved document"; the loader dropping
/// `corners` — red at "the author's ring: it round-trips byte for byte".
#[test]
fn test_the_frame_cycle_replaces_an_authors_corners_and_keeps_the_rest() {
    let mut doc = fixture();
    select(&mut doc, 3);
    let want = edited(&[(
        N3_RING,
        r##"{"fit": "repeat", "size": 0.16, "color": "#ff0000", "spans": [{"text": "═"}], "vertical": "║", "corners": ["╔", "╗", "╝", "╚"]}"##,
    )]);
    holds(&mut doc, "the author's ring", &want, cycle);
}

/// **A node with no frame, and a frame with no ring, get the light box
/// whole**, as `frame on` and `ToggleFrameVisible` give a node with no
/// frame.
///
/// Control: the new ring built of spans alone — red at "cycle on no
/// frame: the saved document".
#[test]
fn test_a_new_ring_is_the_light_box() {
    let light_ring = r#"{"spans": [{"text": "─"}], "vertical": "│", "corners": ["┌", "┐", "┘", "└"]}"#;
    let bare = edited(&[(
        r#""sections": [{"spans": [{"text": "bare"}]}]"#,
        &format!("\"sections\": [{{\"spans\": [{{\"text\": \"bare\"}}]}}],\n      \"frame\": {{\"fit\": \"repeat\", \"rings\": [{light_ring}]}}"),
    )]);
    let ringless = edited(&[(
        r#""frame": {"fit": "repeat"}"#,
        &format!("\"frame\": {{\"fit\": \"repeat\", \"rings\": [{light_ring}]}}"),
    )]);
    let mut doc = fixture();
    select(&mut doc, 1);
    holds(&mut doc, "cycle on no frame", &bare, cycle);
    let mut doc = fixture();
    select(&mut doc, 1);
    holds(&mut doc, "frame on", &bare, |doc| {
        run_line(doc, "frame on").expect("applies");
    });
    let mut doc = fixture();
    select(&mut doc, 1);
    holds(&mut doc, "ToggleFrameVisible", &bare, |doc| {
        assert_eq!(run(&Action::ToggleFrameVisible, doc), DispatchOutcome::Handled);
    });
    let mut doc = fixture();
    select(&mut doc, 4);
    holds(&mut doc, "cycle on no ring", &ringless, cycle);
}

/// **A new text takes a box's `vertical` and `corners` along: to the box
/// of the new text, or off when it is no box's.** An author's ring that
/// is no box keeps its own.
///
/// Controls: `text=` leaving the keys as they are — red at "to another
/// box"; `text=` moving a box's keys to another box but never clearing
/// them — red at "off a box" (the light `│ ┌ ┐ ┘ └` left round `*`);
/// `text=` clearing every ring's keys — red at "an author's ring".
#[test]
fn test_a_new_text_takes_a_boxs_keys_along_and_leaves_an_authors_alone() {
    let mut doc = fixture();
    select(&mut doc, 2);
    cycle(&mut doc);
    cycle(&mut doc);
    cycle(&mut doc);
    let light = edited(&[(N2_FRAME, &n2_frame("─", "│", ["┌", "┐", "┘", "└"]))]);
    assert_eq!(bytes(&doc), light, "precondition: node 2 is the light box");
    let double = edited(&[(N2_FRAME, &n2_frame("═", "║", ["╔", "╗", "╝", "╚"]))]);
    holds(&mut doc, "to another box", &double, |doc| {
        run_line(doc, "frame ring 0 text=═").expect("applies");
    });
    let off = edited(&[(
        N2_FRAME,
        r#""frame": {"fit": "repeat", "rings": [{"spans": [{"text": "*"}]}]}"#,
    )]);
    holds(&mut doc, "off a box", &off, |doc| {
        run_line(doc, "frame ring 0 text=*").expect("applies");
    });

    let mut doc = fixture();
    select(&mut doc, 3);
    let want = edited(&[(
        N3_RING,
        r##"{"fit": "repeat", "size": 0.16, "color": "#ff0000", "spans": [{"text": "*"}], "vertical": "┃", "corners": ["◆", "◆", "◆", "◆"]}"##,
    )]);
    holds(&mut doc, "an author's ring", &want, |doc| {
        run_line(doc, "frame ring 0 text=*").expect("applies");
    });
}

/// **Several frames cycled at once each take the set after their own,
/// in one edit that one undo takes back whole.** Nodes 1 to 4 selected
/// together, one `CycleFrameContent`: the bare node gets the light box,
/// node 2's light ring the heavy box, node 3's author's heavy ring the
/// double box, and node 4's ringless frame the light box.
///
/// Controls: every selected ring sent to the set after the *first*
/// ring's — red at "four frames: the saved document"; a journal's revert
/// writing back its newest entry alone — red at "four frames: one undo
/// gives back the ring before it, whole".
#[test]
fn test_frames_cycled_together_each_take_the_set_after_their_own() {
    let light_ring = r#"{"spans": [{"text": "─"}], "vertical": "│", "corners": ["┌", "┐", "┘", "└"]}"#;
    let want = edited(&[
        (
            r#""sections": [{"spans": [{"text": "bare"}]}]"#,
            &format!(
                "\"sections\": [{{\"spans\": [{{\"text\": \"bare\"}}]}}],\n      \"frame\": {{\"fit\": \"repeat\", \"rings\": [{light_ring}]}}"
            ),
        ),
        (N2_FRAME, &n2_frame("━", "┃", ["┏", "┓", "┛", "┗"])),
        (
            N3_RING,
            r##"{"fit": "repeat", "size": 0.16, "color": "#ff0000", "spans": [{"text": "═"}], "vertical": "║", "corners": ["╔", "╗", "╝", "╚"]}"##,
        ),
        (
            r#""frame": {"fit": "repeat"}"#,
            &format!("\"frame\": {{\"fit\": \"repeat\", \"rings\": [{light_ring}]}}"),
        ),
    ]);
    let mut doc = fixture();
    let nodes = [1, 2, 3, 4].map(|ident| doc.doc().node_by_ident(NodeIdent(ident)).expect("the node"));
    doc.selection.set(nodes.to_vec());
    holds(&mut doc, "four frames", &want, cycle);
}

/// **The cycle changes a frame's first ring alone; `frame ring 1 text=`
/// takes the box keys of the second ring along.** Node 5's heavy ring of
/// spans alone steps to the double box and its second ring, the light
/// box, stays as it is; then `frame ring 1 text=═` makes that second ring
/// the double box, and `text=*` takes its keys off.
///
/// Controls: the cycle changing every ring of a frame — red at "ring 0
/// only: the saved document"; `text=` reading whether ring 0, not the
/// ring it sets, is a box — red at "ring 1 to another box".
#[test]
fn test_the_cycle_changes_the_first_ring_alone_and_ring_one_takes_its_keys_along() {
    let mut doc = fixture();
    select(&mut doc, 5);
    let cycled = edited(&[(
        N5_RINGS,
        r#""rings": [{"spans": [{"text": "═"}], "vertical": "║", "corners": ["╔", "╗", "╝", "╚"]}, {"spans": [{"text": "─"}], "vertical": "│", "corners": ["┌", "┐", "┘", "└"]}]"#,
    )]);
    holds(&mut doc, "ring 0 only", &cycled, cycle);

    let mut doc = fixture();
    select(&mut doc, 5);
    let doubled = edited(&[(
        N5_RINGS,
        r#""rings": [{"spans": [{"text": "━"}]}, {"spans": [{"text": "═"}], "vertical": "║", "corners": ["╔", "╗", "╝", "╚"]}]"#,
    )]);
    holds(&mut doc, "ring 1 to another box", &doubled, |doc| {
        run_line(doc, "frame ring 1 text=═").expect("applies");
    });
    let off = edited(&[(
        N5_RINGS,
        r#""rings": [{"spans": [{"text": "━"}]}, {"spans": [{"text": "*"}]}]"#,
    )]);
    holds(&mut doc, "ring 1 off a box", &off, |doc| {
        run_line(doc, "frame ring 1 text=*").expect("applies");
    });
}
