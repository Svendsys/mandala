// SPDX-License-Identifier: MPL-2.0

//! Presentations (V1-D2, #251): that nothing of one reaches a save, the
//! history or the document's indexes; how an instance lives on the
//! clock; how instances stack; what the layer costs; and what a load and
//! a start refuse. Each assertion has a control beside it, or named in
//! its test's doc, that would make it fail.

use crate::arena::NodeId;
use crate::document::bounds::{BoundError, MAX_ANIMATE_MS, MAX_DEFINITION_PARTS};
use crate::document::element::{DocElement, DocKind};
use crate::document::format::presentations::GlyphOpDto;
use crate::document::format::{load, Limits, LoadErrorKind};
use crate::document::ids::Document;
use crate::document::mutation::compose::{delete, insert_text};
use crate::document::mutation::ops::{DocMutation, NodeOp};
use crate::document::presentation::layer::work;
use crate::document::presentation::{
    parse_presentations, Direction, InstanceId, Play, PresentError, PresentationBody, PresentationDef,
    PresentationLayer,
};
use crate::document::tests::format_support::{kid, kids, node, same_document, written};
use crate::document::tests::writer_support::{assert_consistent, fade, place, request, single};
use crate::document::values::{ColorRef, Rgba};
use crate::document::writer::error::ApplyError;
use crate::document::writer::feed::ChangeRecord;
use crate::document::writer::{ChangeSource, EditRequest, Revision};
use crate::engine::journal::Touch;
use glam::Vec2;

/// One presentation, from the JSON of its definition.
fn presentation(json: &str) -> PresentationDef {
    let file = format!(r#"{{"presentations":[{json}]}}"#);
    let mut read = parse_presentations(&file).unwrap_or_else(|error| panic!("{json} reads: {error}"));
    read.remove(0)
}

/// The refusal of the presentations file holding `json`.
fn refusal(json: &str) -> (String, LoadErrorKind) {
    match parse_presentations(&format!(r#"{{"presentations":[{json}]}}"#)) {
        Ok(_) => panic!("{json} read, and should not have"),
        Err(error) => (error.path, error.kind),
    }
}

/// A map to play on. Node 1 (top level) has two sections — spans "a"
/// (30 pt) and "b", and "c" — a frame with two rings, a line link to
/// node 5, and children 2 and 3; node 2 has child 4; nodes 3 and 5 are
/// on channel 1.
const MAP: &str = r##"{"format":1,"name":"t","theme":{"active":"d","variants":{"d":{"accent":"#ff0000","bg":"#000000"}}},
"nodes":[
{"id":1,"position":[0,0],"size":20.0,"sections":[{"spans":[{"text":"a","size":30.0},{"text":"b"}]},{"channel":1,"spans":[{"text":"c"}]}],
 "frame":{"rings":[{"spans":[{"text":"r0"}]},{"channel":1,"spans":[{"text":"r1"}]}]},
 "links":[{"to":5,"kind":"line"}]},
{"id":2,"parent":1,"position":[300,0],"sections":[{"spans":[{"text":"two"}]}]},
{"id":4,"parent":2,"position":[600,0],"sections":[{"spans":[{"text":"four"}]}]},
{"id":3,"parent":1,"channel":1,"position":[300,200],"sections":[{"spans":[{"text":"three"}]}]},
{"id":5,"channel":1,"position":[900,0],"sections":[{"spans":[{"text":"five"}]}]}
]}"##;

fn map() -> Document {
    load(MAP).unwrap_or_else(|error| panic!("the test map loads: {error}"))
}

/// The node with `ident`.
fn n(doc: &Document, ident: u32) -> NodeId {
    node(doc, ident).0
}

/// Conceals a node: hidden and at opacity 0 at once, back over 400 ms.
const CONCEALED: &str = r#"{"id":"concealed","name":"Concealed","play":"hold","target":"node",
"mutations":[{"node":{"set_hidden":true}},{"node":{"set_opacity":0.0}}],"return":{"ms":400}}"#;

/// Lifts a node ten units over 200 ms, back over 200.
const LIFT: &str = r#"{"id":"lift","name":"Lift","play":"while","target":"node",
"mutations":[{"node":{"nudge":[0.0,-10.0]}}],"animate":{"ms":200}}"#;

/// Colors a node's spans, at once.
const GLOW: &str = r##"{"id":"glow","name":"Glow","play":"while","target":"spans",
"mutations":[{"span":{"text":{"set_color":"#ffe9a8"}}}]}"##;

/// A presentation `id` that sets a node's opacity to `opacity`, in over
/// `in_ms` and out over `out_ms`.
fn fading(id: &str, play: &str, opacity: f32, in_ms: u32, out_ms: u32) -> PresentationDef {
    presentation(&format!(
        r#"{{"id":"{id}","name":"F","play":"{play}","target":"node","mutations":[{{"node":{{"set_opacity":{opacity:?}}}}}],"animate":{{"ms":{in_ms}}},"return":{{"ms":{out_ms}}}}}"#
    ))
}

/// One entry of a journal: the element, what happened to it, and its
/// image before and after.
type Image = (NodeId, Touch, Option<DocElement>, Option<DocElement>);

/// What the journals of the applied history hold, image for image.
fn history_images(doc: &Document) -> Vec<Vec<Image>> {
    doc.applied_journals()
        .map(|journal| {
            journal
                .touched()
                .map(|(id, touch)| (id, touch, journal.before(id).cloned(), journal.after(id).cloned()))
                .collect()
        })
        .collect()
}

/// The records of `records` the presentation layer did not make.
fn of_the_map(records: Vec<ChangeRecord>) -> Vec<ChangeRecord> {
    records
        .into_iter()
        .filter(|record| record.source != ChangeSource::Presentation)
        .collect()
}

/// The one instance playing on `doc`: its progress and its direction.
fn only(doc: &Document) -> (f32, Direction) {
    let mut playing = doc.presentations();
    let instance = playing.next().expect("an instance is playing");
    assert!(playing.next().is_none(), "one instance is playing");
    (instance.t, instance.direction)
}

/// The work the layer did while `f` ran: `[lifted, rewritten, walked]`.
fn work_of(f: impl FnOnce()) -> [usize; 3] {
    let before = work::done();
    f();
    let after = work::done();
    std::array::from_fn(|i| after[i] - before[i])
}

/// The opacity of the node `ident`.
fn opacity(doc: &Document, ident: u32) -> f32 {
    node(doc, ident).1.opacity
}

/// The map of nodes 3, 2 and 1 under a concealment, a lift at half its
/// way and a glow: three presentations, each showing.
fn performed() -> Document {
    let mut doc = map();
    let (one, two, three) = (n(&doc, 1), n(&doc, 2), n(&doc, 3));
    doc.start_presentation(&presentation(CONCEALED), &[three], 0.0)
        .unwrap();
    doc.start_presentation(&presentation(LIFT), &[two], 0.0).unwrap();
    doc.start_presentation(&presentation(GLOW), &[one], 0.0).unwrap();
    doc.tick_presentations(100.0);
    doc.take_changes();
    // The tree shows all three, so a test that then finds none of it in
    // a save found a save that left it out, not a map nothing was on.
    assert!(node(&doc, 3).1.hidden && opacity(&doc, 3) == 0.0);
    assert_eq!(node(&doc, 2).1.position, Vec2::new(300.0, -5.0));
    let section = kid(&doc, one, DocKind::Section, 0).0;
    let span = kid(&doc, section, DocKind::Span, 0).1;
    assert!(
        matches!(span, DocElement::Span(span) if span.style.color == Some(ColorRef::Rgba(Rgba::opaque(0xff, 0xe9, 0xa8))))
    );
    assert_eq!(doc.presentations().count(), 3);
    doc
}

/// With three presentations showing, every save — the writer's, and the
/// one a reader holding the document shared makes — is the save of the
/// map with none, the map is not dirty, its revision has not moved and
/// its history is empty; and the three still show afterwards.
///
/// Controls: with the writer's funnel not taking the layer out,
/// `save_json` writes node 3 hidden; with `format::save` reading the
/// tree as it stands, so does it.
#[test]
fn test_three_presentations_never_reach_a_save() {
    let pristine = written(&map());
    let mut doc = performed();
    assert_eq!(doc.save_json().unwrap(), pristine);
    assert_eq!(written(&doc), pristine);
    assert!(!doc.is_dirty());
    assert_eq!(doc.revision(), Revision(0));
    assert_eq!(doc.history().count(), 0);
    assert_eq!(doc.take_changes(), [], "a save records nothing");
    assert!(node(&doc, 3).1.hidden, "the layer is back on top");
    assert_eq!(node(&doc, 2).1.position, Vec2::new(300.0, -5.0));
}

/// An edit made under three presentations is the edit made without
/// them: the same journal, image for image, the same records of the
/// map, the same save; undo and redo too; and the presentations stay on
/// top of each state. The edits write the very fields the presentations
/// write — node 2's position, node 3's opacity — and one beside them.
///
/// Control: with the funnel not taking the layer out, the nudge's
/// before-image holds the lifted position `(300, -5)`.
#[test]
fn test_an_edit_under_presentations_is_the_edit_without_them() {
    let mut with = performed();
    let mut without = map();
    let edits = |doc: &Document| {
        let nudge = request(
            single(
                DocKind::Node,
                DocMutation::Node(NodeOp::Nudge(Vec2::new(50.0, 7.0))),
            ),
            vec![n(doc, 2)],
            "nudge",
        );
        [
            nudge,
            fade(vec![n(doc, 3)], 0.8),
            place(vec![n(doc, 4)], Vec2::new(1.0, 2.0)),
        ]
    };
    for (under, plain) in edits(&with).into_iter().zip(edits(&without)) {
        let label = under.label.clone();
        with.apply(under, ChangeSource::Action).unwrap();
        without.apply(plain, ChangeSource::Action).unwrap();
        assert_eq!(history_images(&with), history_images(&without), "{label}");
        assert_eq!(
            of_the_map(with.take_changes()),
            without.take_changes(),
            "{label}: the records"
        );
        assert_eq!(with.save_json().unwrap(), written(&without), "{label}: the save");
    }
    assert_eq!(with.revision(), without.revision());
    assert!(with.is_dirty());
    // On top of the edited map: the lift from where the nudge left node
    // 2, the concealment over the faded node 3.
    assert_eq!(node(&with, 2).1.position, Vec2::new(350.0, 2.0));
    assert_eq!(node(&without, 2).1.position, Vec2::new(350.0, 7.0));
    assert!(node(&with, 3).1.hidden && opacity(&with, 3) == 0.0);
    assert_eq!(opacity(&without, 3), 0.8);

    // Undo leaves them on top, and what is undone is the map's.
    for step in 0..3 {
        assert_eq!(with.undo(), without.undo(), "undo {step}");
        assert_eq!(
            with.save_json().unwrap(),
            written(&without),
            "undo {step}: the save"
        );
        assert_eq!(history_images(&with), history_images(&without), "undo {step}");
        assert_eq!(of_the_map(with.take_changes()), without.take_changes());
    }
    assert_eq!(with.save_json().unwrap(), written(&map()));
    assert_eq!(node(&with, 2).1.position, Vec2::new(300.0, -5.0), "still lifted");
    assert!(node(&with, 3).1.hidden, "still concealed");
    assert_eq!(with.presentations().count(), 3);
    assert!(!with.is_dirty(), "undone back to the saved state");
    for step in 0..3 {
        assert_eq!(with.redo(), without.redo(), "redo {step}");
        assert_eq!(
            with.save_json().unwrap(),
            written(&without),
            "redo {step}: the save"
        );
    }
    assert_eq!(node(&with, 2).1.position, Vec2::new(350.0, 2.0));
}

/// A transaction under presentations stages, saves, commits and cancels
/// as without them: the staged change is drawn under the layer, a save
/// in the middle writes neither, and the committed entry is the plain
/// one.
#[test]
fn test_a_transaction_under_presentations_is_the_transaction_without_them() {
    use crate::document::writer::transaction::TransactionKind;
    let mut with = performed();
    let mut without = map();
    let two = n(&with, 2);
    let tx_with = with
        .begin(ChangeSource::Input, TransactionKind::Interactive)
        .unwrap();
    let tx_without = without
        .begin(ChangeSource::Input, TransactionKind::Interactive)
        .unwrap();
    for x in [310.0, 320.0, 330.0] {
        with.stage(&tx_with, place(vec![two], Vec2::new(x, 40.0)))
            .unwrap();
        without
            .stage(&tx_without, place(vec![two], Vec2::new(x, 40.0)))
            .unwrap();
        assert_eq!(
            node(&with, 2).1.position,
            Vec2::new(x, 35.0),
            "the lift, on the stage"
        );
        assert_eq!(
            with.save_json().unwrap(),
            written(&map()),
            "a save writes neither"
        );
    }
    assert_eq!(with.commit(tx_with), without.commit(tx_without));
    assert_eq!(history_images(&with), history_images(&without));
    assert_eq!(with.save_json().unwrap(), written(&without));
    // Canceled: the map as before, the layer on it.
    let tx = with
        .begin(ChangeSource::Input, TransactionKind::Interactive)
        .unwrap();
    with.stage(&tx, place(vec![two], Vec2::new(0.0, 0.0))).unwrap();
    with.cancel(tx);
    assert_eq!(node(&with, 2).1.position, Vec2::new(330.0, 35.0));
    assert_eq!(with.save_json().unwrap(), written(&without));
}

/// An animated transaction's stage, written part-way at each frame, is
/// drawn under the layer at every frame: the node is where the stage
/// has it, lifted. A part-way write changes elements and leaves the
/// tree's stamp where it was, so the stamp alone would have the layer
/// write its old application back over the frame.
///
/// Control: with a restore that goes by the tree's stamp alone, the
/// node stays at 300 through every frame and jumps at the commit.
#[test]
fn test_a_stage_written_part_way_is_drawn_under_the_layer() {
    use crate::document::writer::transaction::TransactionKind;
    use crate::engine::interpolate::Snap;
    let mut doc = performed();
    let two = n(&doc, 2);
    assert_eq!(
        node(&doc, 2).1.position,
        Vec2::new(300.0, -5.0),
        "precondition: lifted from (300, 0)"
    );
    let tx = doc.begin(ChangeSource::Edit, TransactionKind::Animated).unwrap();
    doc.stage(&tx, place(vec![two], Vec2::new(340.0, 40.0))).unwrap();
    doc.take_changes();
    for (t, x, y) in [(0.25, 310.0, 5.0), (0.5, 320.0, 15.0), (0.75, 330.0, 25.0)] {
        doc.stage_progress(&tx, t, Snap::End).unwrap();
        assert_eq!(
            node(&doc, 2).1.position,
            Vec2::new(x, y),
            "at {t}: the stage, lifted"
        );
        let records = doc.take_changes();
        assert!(
            records
                .iter()
                .any(|record| record.touched.iter().any(|(id, _)| *id == two)),
            "at {t}: the frame is on the feed"
        );
        assert_eq!(doc.save_json().unwrap(), written(&map()), "a save writes neither");
    }
    doc.commit(tx);
    assert_eq!(node(&doc, 2).1.position, Vec2::new(340.0, 35.0));
    assert_eq!(doc.history().count(), 1);
}

/// The derived state — the ident index, the incoming links, the budget —
/// describes the map, never the layer: under a presentation that
/// rewrites a span's text (more bytes than the map holds there), an
/// edit and its undo leave them exactly as they leave them without it.
///
/// Controls, each seen red: with the writer's funnel not taking the
/// layer out, the edit is accounted on what the presentation shows; with
/// a resume that leaves the layer set aside, it stays out.
#[test]
fn test_the_derived_state_is_the_maps_under_presentations() {
    let wordy = presentation(
        r#"{"id":"wordy","name":"W","play":"hold","target":"spans","mutations":[{"span":{"set_text":"a much longer text than the map holds"}}]}"#,
    );
    let mut with = map();
    let mut without = map();
    let one = n(&with, 1);
    with.start_presentation(&wordy, &[one], 0.0).unwrap();
    let section = kid(&with, one, DocKind::Section, 0).0;
    let span = kid(&with, section, DocKind::Span, 0);
    assert!(
        matches!(span.1, DocElement::Span(span) if span.text.len() > 30),
        "precondition: the tree shows the long text"
    );
    assert_ne!(
        with.derived().budget,
        crate::document::writer::Derived::scan(with.tree(), with.definitions()).budget,
        "precondition: counted from the tree as it stands, the budget would be another"
    );
    assert_eq!(with.derived(), without.derived(), "started");
    let span = span.0;
    // The builder reads the section's text to place the insert, so it
    // reads the map: the layer is aside while it does.
    let type_into = |doc: &mut Document| {
        doc.suspend_presentations();
        let steps = insert_text(doc, section, 0, "xyz".to_string()).unwrap();
        doc.resume_presentations();
        doc.apply(EditRequest::new(steps, "type"), ChangeSource::Input)
            .unwrap();
    };
    type_into(&mut with);
    type_into(&mut without);
    assert_eq!(with.derived(), without.derived(), "after the edit");
    assert_consistent(&with.without_presentations(), "the map under the layer");
    assert!(
        matches!(with.get(span), Some(DocElement::Span(span)) if span.text.len() > 30),
        "the presentation is on top of the edited span"
    );
    assert_eq!(with.save_json().unwrap(), written(&without));
    assert_eq!(with.undo(), without.undo());
    assert_eq!(with.derived(), without.derived(), "after its undo");
    assert_consistent(&with.without_presentations(), "the map, undone");
    assert_eq!(with.save_json().unwrap(), written(&map()));
}

/// A presentation of `play` that lifts a node ten units: in over
/// `animate`, out over `ret` (both as JSON objects).
fn lifting(play: &str, animate: &str, ret: &str) -> PresentationDef {
    presentation(&format!(
        r#"{{"id":"p","name":"P","play":"{play}","target":"node","mutations":[{{"node":{{"nudge":[0.0,-10.0]}}}}],"animate":{animate},"return":{ret}}}"#
    ))
}

/// Start `def` on node 2 at 0 and read where its instance is at each
/// instant of `at`, given in order: its progress, or `None` once it has
/// ended. `then` runs between the starts and the readings.
fn timeline(def: &PresentationDef, at: &[f64], then: impl FnOnce(&mut Document, NodeId)) -> Vec<Option<f32>> {
    let mut doc = map();
    let two = n(&doc, 2);
    doc.start_presentation(def, &[two], 0.0).unwrap();
    then(&mut doc, two);
    at.iter()
        .map(|&now| {
            doc.tick_presentations(now);
            let t = doc.presentations().next().map(|instance| instance.t);
            // What the tree shows is that progress, written: the lift is
            // ten units at 1.
            let shown = node(&doc, 2).1.position.y;
            assert_eq!(shown, -10.0 * t.unwrap_or(0.0), "at {now}");
            t
        })
        .collect()
}

/// Each `play` at stepped instants of the clock: where it is on its way
/// in, that it holds or does not, where it is on its way out, and when
/// it ends. Every row is a different lifecycle, so a table that read the
/// same for two of them would fail one.
#[test]
fn test_each_play_is_where_its_lifecycle_puts_it_at_each_instant() {
    let (fast, slow) = (r#"{"ms":100}"#, r#"{"ms":200}"#);
    let untouched = |_: &mut Document, _: NodeId| {};
    // `while`: in, then held for as long as nobody stops it.
    assert_eq!(
        timeline(
            &lifting("while", fast, slow),
            &[0.0, 25.0, 50.0, 100.0, 5000.0],
            untouched
        ),
        [Some(0.0), Some(0.25), Some(0.5), Some(1.0), Some(1.0)]
    );
    // `hold`: the same life; who stops it differs, not how it plays.
    assert_eq!(
        timeline(&lifting("hold", fast, slow), &[50.0, 100.0, 5000.0], untouched),
        [Some(0.5), Some(1.0), Some(1.0)]
    );
    // `once`: in, straight back out over `return`, ended at 0.
    assert_eq!(
        timeline(
            &lifting("once", fast, slow),
            &[50.0, 100.0, 150.0, 200.0, 250.0, 300.0],
            untouched
        ),
        [Some(0.5), Some(1.0), Some(0.75), Some(0.5), Some(0.25), None]
    );
    // `loop`: in, out, in again, with no end.
    assert_eq!(
        timeline(
            &lifting("loop", fast, slow),
            &[50.0, 100.0, 200.0, 300.0, 350.0, 450.0, 600.0],
            untouched
        ),
        [
            Some(0.5),
            Some(1.0),
            Some(0.5),
            Some(0.0),
            Some(0.5),
            Some(0.75),
            Some(0.0)
        ]
    );
}

/// A direction that ends between two readings of the clock hands over
/// at its own end: where an instance is at an instant does not depend
/// on how often it was asked. A loop asked once after a long time is
/// where the arithmetic puts it.
///
/// Control: handing over at the reading that noticed puts the `once`
/// below at `1.0` at 150, and the loop anywhere but `0.5`.
#[test]
fn test_an_instant_is_the_same_however_often_the_clock_was_read() {
    let (fast, slow) = (r#"{"ms":100}"#, r#"{"ms":200}"#);
    let untouched = |_: &mut Document, _: NodeId| {};
    assert_eq!(
        timeline(&lifting("once", fast, slow), &[200.0], untouched),
        [Some(0.5)],
        "one reading, past the way in"
    );
    assert_eq!(
        timeline(&lifting("once", fast, slow), &[1000.0], untouched),
        [None]
    );
    // 300 ms a cycle: 3,000,000,050 is 50 ms into a cycle's way in.
    assert_eq!(
        timeline(&lifting("loop", fast, slow), &[3_000_000_050.0], untouched),
        [Some(0.5)]
    );
    assert_eq!(
        timeline(&lifting("loop", fast, slow), &[3_000_000_200.0], untouched),
        [Some(0.5)],
        "and 200 ms in is half way out"
    );
}

/// A delay holds a direction at its start, and an easing bends its way:
/// each on the way in and on the way out, by that direction's own
/// timing.
#[test]
fn test_a_direction_follows_its_own_delay_and_easing() {
    let def = lifting(
        "once",
        r#"{"ms":100,"delay_ms":50,"easing":"ease_in"}"#,
        r#"{"ms":100,"delay_ms":20,"easing":"ease_out"}"#,
    );
    assert_eq!(
        timeline(
            &def,
            &[25.0, 50.0, 100.0, 150.0, 160.0, 170.0, 220.0, 270.0],
            |_, _| {}
        ),
        [
            Some(0.0),  // in its delay
            Some(0.0),  // the delay ends
            Some(0.25), // ease_in: (50 / 100)²
            Some(1.0),
            Some(1.0), // out's delay
            Some(1.0),
            Some(0.25), // ease_out: half way in time is 0.75 of the way out
            None,
        ]
    );
    // Control: linear, and with no delay, the same instants read
    // otherwise.
    assert_eq!(
        timeline(
            &lifting("once", r#"{"ms":100}"#, r#"{"ms":100}"#),
            &[50.0, 100.0],
            |_, _| {}
        ),
        [Some(0.5), Some(1.0)]
    );
}

/// Stopping on the way in turns back from where the instance is, with
/// no jump, over the whole of `return`; a stopped loop finishes its
/// cycle; stopping twice, or one on its way out, changes nothing.
#[test]
fn test_stop_turns_back_from_the_current_progress() {
    let (fast, slow) = (r#"{"ms":100}"#, r#"{"ms":200}"#);
    let stop_at = |when: f64| {
        move |doc: &mut Document, node: NodeId| {
            doc.tick_presentations(when);
            assert_eq!(doc.stop_presentation("p", &[node], when), 1);
        }
    };
    // `while`, stopped at 0.5: 0.5 at that instant, then 0.5 → 0 over
    // the 200 ms of `return`.
    assert_eq!(
        timeline(
            &lifting("while", fast, slow),
            &[50.0, 100.0, 150.0, 200.0, 250.0],
            stop_at(50.0)
        ),
        [Some(0.5), Some(0.375), Some(0.25), Some(0.125), None]
    );
    // Stopped while it holds: from 1.
    assert_eq!(
        timeline(
            &lifting("hold", fast, slow),
            &[500.0, 600.0, 700.0],
            stop_at(500.0)
        ),
        [Some(1.0), Some(0.5), None]
    );
    // `once` on its way in turns back too.
    assert_eq!(
        timeline(&lifting("once", fast, slow), &[50.0, 150.0, 250.0], stop_at(50.0)),
        [Some(0.5), Some(0.25), None]
    );
    // A loop finishes the cycle it is in: stopped at 0.5 on its way in,
    // it goes on to 1, back to 0, and ends there.
    assert_eq!(
        timeline(
            &lifting("loop", fast, slow),
            &[50.0, 100.0, 200.0, 250.0, 300.0, 400.0],
            stop_at(50.0)
        ),
        [Some(0.5), Some(1.0), Some(0.5), Some(0.25), None, None]
    );
    // Control: the same loop, not stopped, is back on its way in at 400.
    assert_eq!(
        timeline(&lifting("loop", fast, slow), &[400.0], |_, _| {}),
        [Some(1.0)]
    );
    // A second stop, on the way out, starts nothing over.
    let mut doc = map();
    let two = n(&doc, 2);
    doc.start_presentation(&lifting("while", fast, slow), &[two], 0.0)
        .unwrap();
    doc.tick_presentations(100.0);
    doc.stop_presentation("p", &[two], 100.0);
    doc.tick_presentations(200.0);
    doc.stop_presentation("p", &[two], 200.0);
    doc.tick_presentations(250.0);
    assert_eq!(only(&doc), (0.25, Direction::Out));
    // Another id, or another node, stops nothing.
    assert_eq!(doc.stop_presentation("q", &[two], 250.0), 0);
    assert_eq!(doc.stop_presentation("p", &[n(&doc, 5)], 250.0), 0);
}

/// Starting a presentation where it is already playing restarts that
/// instance — the same instance, turned to its way in from where it is —
/// and never stacks a second one.
///
/// Control: a start that always adds leaves two instances and the node
/// lifted twice as far.
#[test]
fn test_a_restart_does_not_stack() {
    let def = lifting("while", r#"{"ms":100}"#, r#"{"ms":200}"#);
    let mut doc = map();
    let two = n(&doc, 2);
    let first = doc.start_presentation(&def, &[two], 0.0).unwrap();
    // On its way in: it goes on as it was.
    doc.tick_presentations(50.0);
    assert_eq!(doc.start_presentation(&def, &[two], 50.0).unwrap(), first);
    assert_eq!(only(&doc), (0.5, Direction::In));
    doc.tick_presentations(100.0);
    // Holding: it holds.
    assert_eq!(doc.start_presentation(&def, &[two], 100.0).unwrap(), first);
    assert_eq!(only(&doc), (1.0, Direction::Hold));
    assert_eq!(node(&doc, 2).1.position.y, -10.0, "lifted once");
    // On its way out, at 0.5: back in from 0.5, over the whole of
    // `animate`.
    doc.stop_presentation("p", &[two], 100.0);
    doc.tick_presentations(200.0);
    assert_eq!(only(&doc), (0.5, Direction::Out));
    assert_eq!(doc.start_presentation(&def, &[two], 200.0).unwrap(), first);
    assert_eq!(only(&doc), (0.5, Direction::In));
    doc.tick_presentations(250.0);
    assert_eq!(only(&doc), (0.75, Direction::In));
    doc.tick_presentations(300.0);
    assert_eq!(only(&doc), (1.0, Direction::Hold));
    // On another node it is another instance.
    let other = doc.start_presentation(&def, &[n(&doc, 4)], 300.0).unwrap();
    assert_ne!(other, first);
    assert_eq!(doc.presentations().count(), 2);
    // Started on two nodes at once, one of them playing: one restart,
    // one new instance, in the order given.
    let five = n(&doc, 5);
    let both = doc.start_presentation(&def, &[five, two], 300.0).unwrap();
    assert_eq!(both.len(), 2);
    assert_eq!(both[1], first[0]);
    assert_eq!(doc.presentations().count(), 3);
}

/// One instance per node (amendment 2): started on three nodes, three
/// instances sharing a start, and a stop on one stops that node's alone.
/// An element stands for its node, a node named twice is one, and a node
/// where the target names nothing gets none.
#[test]
fn test_a_start_on_nodes_is_an_instance_a_node() {
    let mut doc = map();
    let (one, two, three) = (n(&doc, 1), n(&doc, 2), n(&doc, 3));
    let section = kid(&doc, two, DocKind::Section, 0).0;
    let ids = doc
        .start_presentation(&presentation(LIFT), &[one, section, three, two], 0.0)
        .unwrap();
    assert_eq!(ids.len(), 3);
    let nodes: Vec<NodeId> = doc.presentations().map(|instance| instance.node).collect();
    assert_eq!(nodes, [one, two, three]);
    doc.tick_presentations(200.0);
    assert_eq!(doc.stop_presentation("lift", &[two], 200.0), 1);
    doc.tick_presentations(300.0);
    let progress: Vec<f32> = doc.presentations().map(|instance| instance.t).collect();
    assert_eq!(progress, [1.0, 0.5, 1.0]);
    // Rings: node 1 has a frame, node 2 has none.
    let ringed = presentation(
        r##"{"id":"r","name":"R","play":"hold","target":"rings","mutations":[{"ring":{"text":{"set_color":"#00ff00"}}}]}"##,
    );
    let started = doc.start_presentation(&ringed, &[one, two], 300.0).unwrap();
    assert_eq!(started.len(), 1, "node 2 has no ring to play on");
    let on_rings = doc.presentations().last().unwrap();
    assert_eq!(on_rings.node, one);
    assert_eq!(on_rings.elements.len(), 2);
}

/// The sequential reveal (design §10.2, D17): `concealed` on the
/// channel-1 nodes hides them at once; stopping it on one node shows
/// that node as soon as its return begins — `snap: end`, so `hidden`
/// holds only at `t = 1` — with its opacity rising from 0 to 1, and the
/// other stays concealed. The saved map shows every node throughout.
///
/// Control: with `snap: start` on the return the node stays hidden
/// until the return has finished, which the second half asserts as the
/// other behavior.
#[test]
fn test_a_sequential_reveal_shows_a_node_as_its_return_begins() {
    let pristine = written(&map());
    let reveal = |snap: &str| {
        let def = presentation(&CONCEALED.replace(r#"{"ms":400}"#, &format!(r#"{{"ms":400{snap}}}"#)));
        let mut doc = map();
        let (three, five) = (n(&doc, 3), n(&doc, 5));
        // `begin`: the channel-1 nodes.
        let channel_one: Vec<NodeId> = [1, 2, 3, 4, 5]
            .into_iter()
            .map(|ident| n(&doc, ident))
            .filter(|&id| doc.get(id).is_some_and(|element| element.channel() == 1))
            .collect();
        assert_eq!(channel_one, [three, five]);
        doc.start_presentation(&def, &channel_one, 0.0).unwrap();
        assert!(node(&doc, 3).1.hidden && node(&doc, 5).1.hidden);
        assert_eq!((opacity(&doc, 3), opacity(&doc, 5)), (0.0, 0.0));
        assert!(!node(&doc, 1).1.hidden, "a channel-0 node is not concealed");
        // `next`: stop it on node 3, and step the clock.
        doc.stop_presentation("concealed", &[three], 1000.0);
        let mut seen = Vec::new();
        for now in [1000.0, 1025.0, 1100.0, 1200.0, 1300.0, 1400.0] {
            doc.tick_presentations(now);
            seen.push((node(&doc, 3).1.hidden, opacity(&doc, 3)));
            assert!(node(&doc, 5).1.hidden && opacity(&doc, 5) == 0.0, "node 5 stays");
            assert_eq!(doc.save_json().unwrap(), pristine, "at {now}");
        }
        assert_eq!(doc.presentations().count(), 1, "node 3's has ended");
        seen
    };
    assert_eq!(
        reveal(""),
        [
            (true, 0.0), // stopped, and not yet a moment on
            (false, 0.0625),
            (false, 0.25),
            (false, 0.5),
            (false, 0.75),
            (false, 1.0),
        ]
    );
    let late = reveal(r#","snap":"start""#);
    assert_eq!(
        late.iter().map(|(hidden, _)| *hidden).collect::<Vec<_>>(),
        [true, true, true, true, true, false],
        "`snap: start`: hidden for as long as the return runs"
    );
}

/// Two instances on one node's opacity combine in start order: the
/// later one starts from what the earlier one shows. And a frame in
/// which the older of the two moved writes back out from it through the
/// newest and applies those again, leaving an instance started before
/// them alone: counted on the applications themselves.
///
/// Control: writing the whole layer out on every frame lifts three
/// applications where this lifts two.
#[test]
fn test_instances_combine_in_start_order_and_a_frame_rewrites_from_the_oldest_that_moved() {
    let mut doc = map();
    let (one, five) = (n(&doc, 1), n(&doc, 5));
    // X on node 5, then A and B on node 1.
    doc.start_presentation(&fading("x", "hold", 0.5, 0, 0), &[five], 0.0)
        .unwrap();
    doc.start_presentation(&fading("a", "hold", 0.5, 0, 200), &[one], 0.0)
        .unwrap();
    doc.start_presentation(&fading("b", "hold", 0.25, 1000, 1000), &[one], 0.0)
        .unwrap();
    // A shows 0.5; B, half way, is half way from A's 0.5 to its 0.25.
    doc.tick_presentations(500.0);
    assert_eq!(opacity(&doc, 1), 0.375);
    assert_eq!(opacity(&doc, 5), 0.5);
    // Control for the order: B alone, half way, is half way from 1.
    let mut alone = map();
    alone
        .start_presentation(&fading("b", "hold", 0.25, 1000, 1000), &[n(&alone, 1)], 0.0)
        .unwrap();
    alone.tick_presentations(500.0);
    assert_eq!(opacity(&alone, 1), 0.625);

    // Stop the older, A, and step: A is half way back to the map's 1
    // (0.75), and B, at 0.625 of its way, starts from that.
    doc.stop_presentation("a", &[one], 500.0);
    doc.take_changes();
    let work = work_of(|| {
        doc.tick_presentations(625.0);
    });
    // 125 of A's 200 ms out: A at 0.375, showing 1 + (0.5 - 1) * 0.375.
    let a_shows = 0.8125;
    assert_eq!(opacity(&doc, 1), a_shows + (0.25 - a_shows) * 0.625);
    assert_eq!(opacity(&doc, 5), 0.5);
    // A and B came out; A's journal was written at its new progress; B,
    // which stands on what A shows, was walked afresh. X: nothing.
    assert_eq!(work, [2, 1, 1]);
    let records = doc.take_changes();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].source, ChangeSource::Presentation);
    assert!(
        records[0]
            .touched
            .iter()
            .all(|&(id, touch)| id == one && touch == Touch::Field),
        "node 1 alone: {:?}",
        records[0].touched
    );
    // Only B moved: nothing under it changed, so its own journal is
    // written at the new progress, and neither A nor X is touched.
    doc.tick_presentations(700.0);
    doc.take_changes();
    assert_eq!(doc.presentations().count(), 2, "A has ended");
    let work = work_of(|| {
        doc.tick_presentations(750.0);
    });
    assert_eq!(work, [1, 1, 0]);
    assert_eq!(opacity(&doc, 1), 1.0 + (0.25 - 1.0) * 0.75);
    // A reading at which nothing moved does nothing at all.
    doc.take_changes();
    let epoch = doc.presentation_epoch();
    let work = work_of(|| {
        assert!(!doc.tick_presentations(750.0));
    });
    assert_eq!(work, [0, 0, 0]);
    assert_eq!(doc.take_changes(), []);
    assert_eq!(doc.presentation_epoch(), epoch);
}

/// What the funnel costs, and what it tells the projector: an edit
/// elsewhere on the map takes each application out and walks each
/// afresh, and records nothing for the layer, since every instance
/// shows what it showed; an edit of a field an instance stands on is
/// recorded for that element alone; a call that changes nothing — a
/// save, a refused request — writes the journals forward again as they
/// are, with no walk.
///
/// Control: a restore that walks whatever the stamp says turns the
/// save's `[3, 3, 0]` into `[3, 0, 3]`.
#[test]
fn test_the_funnel_walks_only_when_the_map_changed_and_records_only_what_shows_differently() {
    let mut doc = performed();
    // Three instances: the concealment, the lift, and the glow.
    let work = work_of(|| {
        doc.apply(place(vec![n(&doc, 5)], Vec2::new(7.0, 7.0)), ChangeSource::Action)
            .unwrap();
    });
    assert_eq!(work, [3, 0, 3]);
    let records = doc.take_changes();
    assert_eq!(
        records.iter().map(|record| record.source).collect::<Vec<_>>(),
        [ChangeSource::Action],
        "the layer shows what it showed"
    );
    // The lift stands on node 2's position: moving node 2 moves what it
    // shows, and that is recorded for node 2 alone.
    let two = n(&doc, 2);
    doc.apply(place(vec![two], Vec2::new(0.0, 100.0)), ChangeSource::Action)
        .unwrap();
    let records = doc.take_changes();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].source, ChangeSource::Presentation);
    assert_eq!(records[1].touched, [(two, Touch::Field)]);
    assert_eq!(
        records[1].revision,
        doc.revision(),
        "at the revision the edit made"
    );
    assert_eq!(node(&doc, 2).1.position, Vec2::new(0.0, 95.0));
    // A save changes nothing in the tree.
    let work = work_of(|| {
        doc.save_json().unwrap();
    });
    assert_eq!(work, [3, 3, 0]);
    // A refused request: an opacity past its bound.
    let work = work_of(|| {
        assert!(doc.apply(fade(vec![two], 7.0), ChangeSource::Action).is_err());
    });
    assert_eq!(work, [3, 3, 0]);
    assert_eq!(doc.take_changes(), []);
}

/// What a start, a stop and a frame record, and what they count: a
/// record of `ChangeSource::Presentation` at the revision the document
/// has, naming the elements that now show differently; the epoch moved
/// by each, the revision by none.
#[test]
fn test_the_layer_records_under_its_own_source_and_counts_its_own_epoch() {
    let mut doc = map();
    let two = n(&doc, 2);
    let def = lifting("while", r#"{"ms":100}"#, r#"{"ms":100}"#);
    assert_eq!(doc.presentation_epoch(), 0);
    doc.start_presentation(&def, &[two], 0.0).unwrap();
    assert_eq!(doc.presentation_epoch(), 1);
    assert_eq!(doc.take_changes(), [], "at 0 it shows the map");
    assert!(doc.presentations_moving());
    assert!(doc.tick_presentations(50.0), "a frame that wrote owes a draw");
    assert_eq!(doc.presentation_epoch(), 2);
    let records = doc.take_changes();
    assert_eq!(records.len(), 1);
    assert_eq!(
        (records[0].source, records[0].revision),
        (ChangeSource::Presentation, Revision(0))
    );
    assert_eq!(records[0].touched, [(two, Touch::Field)]);
    assert!(
        records[0].inputs.is_empty(),
        "a node's position is no resolution input"
    );
    assert!(records[0].removed.is_empty() && records[0].rows.is_empty());
    // A style it writes is named as the input it is, for the readers of
    // that element's style.
    let mut styled = map();
    let one = n(&styled, 1);
    styled
        .start_presentation(&presentation(GLOW), &[one], 0.0)
        .unwrap();
    let records = styled.take_changes();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].touched.len(), 3, "node 1's three spans");
    let spans: Vec<NodeId> = records[0].touched.iter().map(|&(id, _)| id).collect();
    assert_eq!(
        records[0].inputs,
        spans
            .iter()
            .map(|&span| (span, crate::document::deps::InputKind::Style))
            .collect::<Vec<_>>()
    );
    doc.tick_presentations(100.0);
    assert!(!doc.presentations_moving(), "holding");
    let epoch = doc.presentation_epoch();
    // An edit underneath is not the layer's doing.
    doc.apply(place(vec![n(&doc, 5)], Vec2::ZERO), ChangeSource::Action)
        .unwrap();
    assert_eq!(doc.presentation_epoch(), epoch);
    assert_eq!(doc.revision(), Revision(1));
    doc.stop_presentation("lift", &[two], 100.0);
    assert_eq!(doc.presentation_epoch(), epoch, "another id stops nothing");
    doc.stop_presentation("p", &[two], 100.0);
    assert_eq!(doc.presentation_epoch(), epoch + 1);
    doc.tick_presentations(200.0);
    assert_eq!(doc.presentation_epoch(), epoch + 2, "ended");
    assert_eq!(doc.presentations().count(), 0);
    assert_eq!(doc.revision(), Revision(1));
    assert_eq!(doc.history().count(), 1);
}

/// Setting the layer aside shows the map itself; resuming puts every
/// instance back at the progress it has by then. With nobody reading the
/// feed in between, the pair records nothing and walks nothing; with a
/// reader in between, the reader is told each way. Suspensions nest.
///
/// Control: a resume that does not apply leaves node 3 shown.
#[test]
fn test_a_suspension_shows_the_map_and_comes_back() {
    let pristine = written(&map());
    let mut doc = performed();
    let work = work_of(|| {
        doc.suspend_presentations();
        assert!(doc.presentations_suspended());
        assert!(!node(&doc, 3).1.hidden, "the map itself");
        assert_eq!(node(&doc, 2).1.position, Vec2::new(300.0, 0.0));
        assert!(same_document(&doc, &map()).is_ok());
        assert_eq!(doc.presentations().count(), 3, "the instances live on");
        doc.resume_presentations();
    });
    assert_eq!(work, [3, 3, 0]);
    assert!(!doc.presentations_suspended());
    assert!(node(&doc, 3).1.hidden);
    assert_eq!(node(&doc, 2).1.position, Vec2::new(300.0, -5.0));
    assert_eq!(doc.take_changes(), [], "nobody read in between: nothing to tell");

    // A reader in between is told the layer came out, then that it is
    // back.
    doc.suspend_presentations();
    let out = doc.take_changes();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].source, ChangeSource::Presentation);
    let touched = |records: &[ChangeRecord]| {
        let mut ids: Vec<NodeId> = records
            .iter()
            .flat_map(|record| record.touched.iter().map(|&(id, _)| id))
            .collect();
        ids.sort();
        ids.dedup();
        ids
    };
    let shown = touched(&out);
    assert!(shown.contains(&n(&doc, 3)) && shown.contains(&n(&doc, 2)));
    assert_eq!(shown.len(), 2 + 3, "nodes 3 and 2, and node 1's three spans");
    doc.resume_presentations();
    assert_eq!(touched(&doc.take_changes()), shown);

    // An edit made in between is made on the map, and the layer comes
    // back on top of it. The clock moves on meanwhile.
    doc.suspend_presentations();
    doc.suspend_presentations();
    doc.apply(
        place(vec![n(&doc, 2)], Vec2::new(10.0, 10.0)),
        ChangeSource::Action,
    )
    .unwrap();
    doc.tick_presentations(200.0);
    doc.resume_presentations();
    assert!(doc.presentations_suspended(), "the outer one is still open");
    assert_eq!(node(&doc, 2).1.position, Vec2::new(10.0, 10.0));
    doc.resume_presentations();
    assert_eq!(
        node(&doc, 2).1.position,
        Vec2::new(10.0, 0.0),
        "lifted whole by now"
    );
    let mut plain = map();
    plain
        .apply(
            place(vec![n(&plain, 2)], Vec2::new(10.0, 10.0)),
            ChangeSource::Action,
        )
        .unwrap();
    assert_eq!(history_images(&doc), history_images(&plain));
    assert_eq!(doc.save_json().unwrap(), written(&plain));
    assert_ne!(written(&plain), pristine);
    // A resume nothing matches does nothing.
    doc.resume_presentations();
    assert!(!doc.presentations_suspended());
    // A presentation started while the layer is aside shows when it is
    // back.
    doc.suspend_presentations();
    doc.start_presentation(&fading("f", "hold", 0.5, 0, 0), &[n(&doc, 5)], 200.0)
        .unwrap();
    assert_eq!(opacity(&doc, 5), 1.0);
    doc.resume_presentations();
    assert_eq!(opacity(&doc, 5), 0.5);
}

/// `own` is the map's element under whatever plays on it — one
/// instance or two, part-way or whole — and the tree's where nothing
/// does; it follows the map when an edit changes it.
///
/// Control: reading the newest application's before-image gives node 2
/// as the dimming left it, under the lift.
#[test]
fn test_own_is_the_maps_element_under_whatever_plays() {
    let mut doc = performed();
    let (two, three, four) = (n(&doc, 2), n(&doc, 3), n(&doc, 4));
    // A second instance on node 2, over the lift.
    doc.start_presentation(&fading("dim", "hold", 0.25, 0, 0), &[two], 100.0)
        .unwrap();
    let shown = node(&doc, 2).1;
    assert_eq!(
        (shown.position, shown.opacity),
        (Vec2::new(300.0, -5.0), 0.25),
        "precondition: both show"
    );
    let own = |doc: &Document, id: NodeId| match doc.own(id) {
        Some(DocElement::Node(data)) => (data.position, data.opacity, data.hidden),
        other => panic!("{other:?}"),
    };
    assert_eq!(own(&doc, two), (Vec2::new(300.0, 0.0), 1.0, false));
    assert_eq!(
        own(&doc, three),
        (Vec2::new(300.0, 200.0), 1.0, false),
        "concealed"
    );
    assert_eq!(
        doc.own(four),
        doc.get(four),
        "nothing plays on node 4: the tree's own element"
    );
    // The map moves under them.
    doc.apply(place(vec![two], Vec2::new(340.0, 40.0)), ChangeSource::Action)
        .unwrap();
    assert_eq!(own(&doc, two), (Vec2::new(340.0, 40.0), 1.0, false));
    assert_eq!(node(&doc, 2).1.position, Vec2::new(340.0, 35.0));
    // Set aside, the tree is the map.
    doc.suspend_presentations();
    assert_eq!(own(&doc, two), (Vec2::new(340.0, 40.0), 1.0, false));
    doc.resume_presentations();
    // An element the tree no longer holds has no element of the map's.
    let steps = delete(&doc, &[four]).unwrap();
    doc.apply(EditRequest::new(steps, "delete"), ChangeSource::Action)
        .unwrap();
    assert!(doc.own(four).is_none());
}

/// What a presentation says of the rest of the map: the verbs it names,
/// in its filter and in its mutations, and that it can show a node when
/// it writes `set_hidden: false` — which only one that plays on nodes
/// can write, and nothing gives a portal end a marker: a load refuses a
/// mutation of another kind than the target's.
///
/// Controls: with the filter not walked, `peek` is not named; with the
/// mutations not walked, `go` is not, and nothing is revealed.
#[test]
fn test_a_presentation_mentions_the_verbs_it_names_and_what_it_can_show() {
    use crate::document::edit::{Named, Shows};
    let said = |json: &str| {
        let def = presentation(json);
        let mut verbs: Vec<String> = Vec::new();
        let shows = def.mentions(|mention| {
            if mention.kind == Named::Verb {
                verbs.push(mention.name.to_string());
            }
        });
        (verbs, shows)
    };
    let nothing = Shows::default();
    let reveals = Shows {
        reveals: true,
        ..Shows::default()
    };
    assert_eq!(
        said(
            r#"{"id":"p","name":"P","play":"hold","target":"node",
            "filter":{"fields":[[{"on_hover":"peek"},{"Equals":false}]]},
            "mutations":[{"node":{"set_click":"go"}},{"node":{"set_hidden":false}}]}"#
        ),
        (vec!["peek".to_string(), "go".to_string()], reveals)
    );
    assert_eq!(
        said(
            r#"{"id":"p","name":"P","play":"hold","target":"node","mutations":[{"node":{"set_hidden":true}}]}"#
        ),
        (Vec::new(), nothing)
    );
    assert_eq!(
        said(
            r#"{"id":"p","name":"P","play":"hold","target":"spans","mutations":[{"span":{"set_link":{"verb":"go"}}}]}"#
        ),
        (vec!["go".to_string()], nothing)
    );
    assert_eq!(
        said(
            r#"{"id":"g","name":"G","play":"hold","layer":"glyph","target":"sections","glyph":[{"run_opacity":0.5}]}"#
        ),
        (Vec::new(), nothing)
    );
    // A node's `set_hidden` on another target, and a portal end's marker
    // on any, are no presentation: neither reads.
    for (target, mutation) in [
        ("sections", r#"{"node":{"set_hidden":false}}"#),
        ("links", r#"{"portal_end":{"set_content":"◆"}}"#),
    ] {
        let (path, _) = refusal(&format!(
            r#"{{"id":"p","name":"P","play":"hold","target":"{target}","mutations":[{mutation}]}}"#
        ));
        assert_eq!(path, "presentations[0].mutations[0]", "{mutation} on {target}");
    }
}

/// A node an edit removes takes its instance's application with it, and
/// an undo brings both back; once the history can no longer bring the
/// node back, the instance is dropped. An edit that pushes the removal
/// out of the history frees the node inside its own apply, and so ends
/// the instance there: the presentation epoch advances inside that
/// apply, though the edit wrote nothing of an instance's.
///
/// Controls, each seen red: the funnel not dropping the instances of
/// freed nodes leaves the instance listed after that apply; dropping
/// them without advancing the epoch leaves the epoch where it was.
#[test]
fn test_an_instance_follows_its_node_out_of_the_tree_and_back() {
    let mut doc = map();
    let four = n(&doc, 4);
    doc.start_presentation(&fading("f", "hold", 0.5, 0, 0), &[four], 0.0)
        .unwrap();
    assert_eq!(opacity(&doc, 4), 0.5);
    let steps = delete(&doc, &[four]).unwrap();
    doc.apply(EditRequest::new(steps, "delete"), ChangeSource::Action)
        .unwrap();
    assert!(doc.get(four).is_none());
    assert_eq!(doc.presentations().count(), 1, "it waits for its node");
    assert_eq!(doc.save_json().unwrap().matches(r#""id": 4"#).count(), 0);
    doc.undo().unwrap();
    assert_eq!(opacity(&doc, 4), 0.5, "back, and performed again");
    let restored = doc.save_json().unwrap();
    assert_eq!(restored, written(&map()), "the map's own opacity is saved");
    doc.redo().unwrap();
    // The entry leaves the history: nothing can restore the node.
    doc.set_history_limit(0);
    doc.tick_presentations(10.0);
    assert_eq!(doc.presentations().count(), 0);

    // Freed inside an edit's apply.
    let mut doc = map();
    doc.set_history_limit(1);
    let (two, four) = (n(&doc, 2), n(&doc, 4));
    doc.start_presentation(&fading("f", "hold", 0.5, 0, 0), &[four], 0.0)
        .unwrap();
    let steps = delete(&doc, &[four]).unwrap();
    doc.apply(EditRequest::new(steps, "delete"), ChangeSource::Action)
        .unwrap();
    let before = doc.presentation_epoch();
    assert_eq!(doc.presentations().count(), 1, "precondition: it waits");
    doc.apply(fade(vec![two], 0.8), ChangeSource::Action).unwrap();
    assert_eq!(doc.presentations().count(), 0, "the edit freed its node");
    assert_eq!(doc.presentation_epoch(), before + 1);
}

/// On a target that holds maps to a budget (wasm32's bounds), a start
/// that would pass it is refused in the writer's words, and starts
/// nothing.
#[test]
fn test_a_start_past_the_platform_budget_is_refused_in_the_writers_words() {
    let mut doc = map();
    let text = doc.derived().budget.text;
    doc.set_limits(Limits {
        bytes: Some(text + 4),
        elements: None,
    });
    let wordy = presentation(
        r#"{"id":"wordy","name":"W","play":"hold","target":"spans","mutations":[{"span":{"set_text":"0123456789"}}]}"#,
    );
    // Node 2's one span holds "two": ten bytes for three is seven more.
    let refused = doc.start_presentation(&wordy, &[n(&doc, 2)], 0.0).unwrap_err();
    assert_eq!(
        refused,
        PresentError::Refused {
            presentation: "wordy".to_string(),
            error: Box::new(ApplyError::TooMuchText {
                bytes: text + 7,
                max: text + 4
            }),
        }
    );
    assert_eq!(
        refused.to_string(),
        format!(
            "presentation `wordy`: the map would hold {} bytes of text; this platform holds at most {}",
            text + 7,
            text + 4
        )
    );
    assert_eq!(doc.presentations().count(), 0);
    assert!(same_document(&doc, &map()).is_ok());
    // Control: within the budget it starts.
    doc.set_limits(Limits {
        bytes: Some(text + 7),
        elements: None,
    });
    assert_eq!(
        doc.start_presentation(&wordy, &[n(&doc, 2)], 0.0).unwrap().len(),
        1
    );
}

/// On a target that holds maps to a budget, the instances are held to
/// it together, in start order — each on top of the map and every
/// instance before it — never one by one against the map alone: two
/// that each fit and together do not are not both shown.
///
/// - A start on two nodes is two instances: it is refused, in the
///   writer's words for the two together, and starts neither.
/// - Started one after the other, the second start is refused the same
///   way, and the first plays on.
/// - Two that fit together until an edit grows the map: the older shows,
///   the newer shows nothing and says why, and both show again once an
///   undo makes room.
///
/// Controls, each seen red: each application judged against the map
/// alone (the writer's `within_budget`), and a restore that does not
/// carry forward the cost of the instances before each, both start both
/// instances of the first start.
#[test]
fn test_instances_that_fit_alone_and_not_together_are_held_to_the_budget_together() {
    let wordy = presentation(
        r#"{"id":"wordy","name":"W","play":"hold","target":"spans","mutations":[{"span":{"set_text":"0123456789"}}]}"#,
    );
    let mut doc = map();
    let text = doc.derived().budget.text;
    let (two, four) = (n(&doc, 2), n(&doc, 4));
    // "two" → ten bytes is seven more; "four" → ten is six more. Each
    // fits within eight; together they are thirteen.
    doc.set_limits(Limits {
        bytes: Some(text + 8),
        elements: None,
    });
    let together = Box::new(ApplyError::TooMuchText {
        bytes: text + 13,
        max: text + 8,
    });
    assert_eq!(
        doc.start_presentation(&wordy, &[two, four], 0.0),
        Err(PresentError::Refused {
            presentation: "wordy".to_string(),
            error: together.clone(),
        }),
        "one start, two instances"
    );
    assert_eq!(doc.presentations().count(), 0);
    assert!(same_document(&doc, &map()).is_ok());
    // One after the other.
    doc.start_presentation(&wordy, &[two], 0.0).unwrap();
    assert_eq!(
        doc.start_presentation(&wordy, &[four], 0.0),
        Err(PresentError::Refused {
            presentation: "wordy".to_string(),
            error: together,
        }),
        "a second start, on top of the first"
    );
    assert_eq!(doc.presentations().count(), 1);
    assert_eq!(first_span(&doc, 2), "0123456789");
    assert_eq!(first_span(&doc, 4), "four");

    // Room for both, until an edit grows the map by one byte.
    let mut doc = map();
    doc.set_limits(Limits {
        bytes: Some(text + 13),
        elements: None,
    });
    doc.start_presentation(&wordy, &[two, four], 0.0).unwrap();
    assert_eq!(
        (first_span(&doc, 2).as_str(), first_span(&doc, 4).as_str()),
        ("0123456789", "0123456789")
    );
    let section = kid(&doc, n(&doc, 1), DocKind::Section, 0).0;
    let steps = insert_text(&doc, section, 0, "x".to_string()).unwrap();
    doc.apply(EditRequest::new(steps, "type"), ChangeSource::Input)
        .unwrap();
    assert_eq!(first_span(&doc, 2), "0123456789", "the older one shows");
    assert_eq!(first_span(&doc, 4), "four", "the newer one shows nothing");
    let refusals: Vec<Option<ApplyError>> = doc
        .presentations()
        .map(|instance| instance.refusal.cloned())
        .collect();
    assert_eq!(
        refusals,
        vec![
            None,
            Some(ApplyError::TooMuchText {
                bytes: text + 14,
                max: text + 13
            })
        ]
    );
    doc.undo().unwrap();
    assert_eq!(
        (first_span(&doc, 2).as_str(), first_span(&doc, 4).as_str()),
        ("0123456789", "0123456789")
    );
    assert!(doc.presentations().all(|instance| instance.refusal.is_none()));
}

/// The text of node `ident`'s first span, as the tree holds it now.
fn first_span(doc: &Document, ident: u32) -> String {
    let section = kid(doc, n(doc, ident), DocKind::Section, 0).0;
    match kid(doc, section, DocKind::Span, 0).1 {
        DocElement::Span(span) => span.text.clone(),
        other => panic!("{other:?}"),
    }
}

/// The bytes of text the tree holds now, counted from scratch: what the
/// platform's bound is about, whatever the layer shows.
fn tree_text(doc: &Document) -> u64 {
    crate::document::writer::Derived::scan(doc.tree(), doc.definitions())
        .budget
        .text
}

/// **An instance costs what it shows.** A text field snaps (`snap: end`
/// by default): until it switches, the element shows its before-image,
/// so an instance that shrinks a text frees no room on its way in, and
/// takes the room back as soon as it starts its way out. The instances
/// after it are judged on top of it as it stands, and judged again when
/// it switches; the tree never holds more text than the bound.
///
/// - `shrink` empties node 4's span ("four", 4 bytes) over 400 ms.
///   Started first, on its way in, it frees nothing: `wordy` (+7 on
///   node 2) does not fit within +3, and the start is refused.
/// - Once `shrink` has switched (at 400 ms), `wordy` fits: −4 + 7.
/// - Stopped, `shrink` shows "four" again from its first step out:
///   `wordy` on top of it no longer fits, and shows nothing and says why.
///
/// Control, seen red: each instance credited with its after-image,
/// whatever its progress, admits `wordy` at 0 ms, the tree then holding
/// four bytes more than the bound.
#[test]
fn test_an_instance_is_costed_as_it_stands_at_its_snap() {
    let shrink = presentation(
        r#"{"id":"shrink","name":"S","play":"hold","target":"spans","mutations":[{"span":{"set_text":""}}],"animate":{"ms":400},"return":{"ms":400}}"#,
    );
    let wordy = presentation(
        r#"{"id":"wordy","name":"W","play":"hold","target":"spans","mutations":[{"span":{"set_text":"0123456789"}}]}"#,
    );
    let mut doc = map();
    let text = doc.derived().budget.text;
    let (two, four) = (n(&doc, 2), n(&doc, 4));
    doc.set_limits(Limits {
        bytes: Some(text + 3),
        elements: None,
    });
    doc.start_presentation(&shrink, &[four], 0.0).unwrap();
    assert_eq!(first_span(&doc, 4), "four", "precondition: not switched yet");
    assert_eq!(
        doc.start_presentation(&wordy, &[two], 0.0),
        Err(PresentError::Refused {
            presentation: "wordy".to_string(),
            error: Box::new(ApplyError::TooMuchText {
                bytes: text + 7,
                max: text + 3
            }),
        }),
        "no room is freed before the switch"
    );
    assert_eq!(doc.presentations().count(), 1);
    assert_eq!(tree_text(&doc), text);

    doc.tick_presentations(400.0);
    assert_eq!(first_span(&doc, 4), "", "switched");
    doc.start_presentation(&wordy, &[two], 400.0).unwrap();
    assert_eq!(first_span(&doc, 2), "0123456789");
    assert_eq!(tree_text(&doc), text + 3);

    doc.stop_presentation("shrink", &[four], 500.0);
    doc.tick_presentations(600.0);
    assert_eq!(
        first_span(&doc, 4),
        "four",
        "on its way out, before the switch back"
    );
    assert_eq!(first_span(&doc, 2), "two", "wordy no longer fits on top of it");
    let refusals: Vec<Option<ApplyError>> = doc
        .presentations()
        .map(|instance| instance.refusal.cloned())
        .collect();
    assert_eq!(
        refusals,
        vec![
            None,
            Some(ApplyError::TooMuchText {
                bytes: text + 7,
                max: text + 3
            })
        ]
    );
    assert_eq!(tree_text(&doc), text);
}

/// **A start is judged where the layer puts it.** On a span an older
/// instance already writes, the new one stands on the older one's text,
/// not the map's, and is costed so: `twelve` (+9 over the map's "two")
/// on top of `ten` (+7) costs +9 in all, and fits within +10, though
/// the two deltas over the map add up to +16.
///
/// Control, seen red: the new instance judged on the map, over the cost
/// of the instances already playing, counts `ten`'s delta and its own
/// over the map both, and refuses `twelve` (`bytes: text + 16`).
#[test]
fn test_a_start_is_judged_on_the_instances_it_stands_on() {
    let setting = |id: &str, text: &str| {
        presentation(&format!(
            r#"{{"id":"{id}","name":"T","play":"hold","target":"spans","mutations":[{{"span":{{"set_text":"{text}"}}}}]}}"#
        ))
    };
    let (ten, twelve) = (setting("ten", "0123456789"), setting("twelve", "0123456789ab"));
    let mut doc = map();
    let text = doc.derived().budget.text;
    let two = n(&doc, 2);
    doc.set_limits(Limits {
        bytes: Some(text + 10),
        elements: None,
    });
    doc.start_presentation(&ten, &[two], 0.0).unwrap();
    assert_eq!(tree_text(&doc), text + 7);
    assert_eq!(
        doc.start_presentation(&twelve, &[two], 0.0).map(|ids| ids.len()),
        Ok(1)
    );
    assert_eq!(first_span(&doc, 2), "0123456789ab");
    assert_eq!(tree_text(&doc), text + 9);
    assert!(doc.presentations().all(|instance| instance.refusal.is_none()));
}

/// **A refused start restarts nothing, in what is drawn as in the
/// lifecycle.** A start that restarts an instance on one node and is
/// refused on another is taken back whole: the restarted instance is
/// drawn again at the progress and snap it had — not as the restart drew
/// it — at once and after a frame at the same instant, the tree holds the
/// text it held, the feed holds none of the start's records, and the
/// epoch has not moved.
///
/// `p` writes ten bytes on a span, in on `snap: start`, out on
/// `snap: end`. On node 2 it is on its way out (`t = 0.75`), showing
/// "two"; a start on nodes 2 and 4 restarts node 2's (in, `snap: start`:
/// "0123456789") and is refused on node 4 (+6 on top of +7, within +7).
///
/// Controls, each seen red: the layer taken back from the first new
/// instance only leaves node 2 showing "0123456789" after the refusal;
/// with the start's records left in the feed, they are there to be
/// taken.
#[test]
fn test_a_refused_start_leaves_a_restarted_instance_drawn_as_it_was() {
    let p = presentation(
        r#"{"id":"p","name":"P","play":"hold","target":"spans","mutations":[{"span":{"set_text":"0123456789"}}],
"animate":{"ms":400,"snap":"start"},"return":{"ms":400,"snap":"end"}}"#,
    );
    let mut doc = map();
    let text = doc.derived().budget.text;
    let (two, four) = (n(&doc, 2), n(&doc, 4));
    doc.set_limits(Limits {
        bytes: Some(text + 7),
        elements: None,
    });
    doc.start_presentation(&p, &[two], 0.0).unwrap();
    doc.tick_presentations(400.0);
    assert_eq!(doc.stop_presentation("p", &[two], 400.0), 1);
    doc.tick_presentations(500.0);
    let was = |doc: &Document| {
        let instance = doc.presentations().next().expect("one instance");
        (instance.t, instance.direction, instance.snap)
    };
    let before = was(&doc);
    assert_eq!(
        before,
        (0.75, Direction::Out, crate::engine::interpolate::Snap::End)
    );
    assert_eq!(first_span(&doc, 2), "two", "precondition: on its way out");
    assert_eq!(tree_text(&doc), text);
    doc.take_changes();
    let epoch = doc.presentation_epoch();

    assert_eq!(
        doc.start_presentation(&p, &[two, four], 500.0),
        Err(PresentError::Refused {
            presentation: "p".to_string(),
            error: Box::new(ApplyError::TooMuchText {
                bytes: text + 13,
                max: text + 7
            }),
        })
    );
    assert_eq!(doc.presentations().count(), 1, "node 4's is not kept");
    assert_eq!(was(&doc), before, "the restart is taken back");
    assert_eq!(first_span(&doc, 2), "two", "and so is what it drew");
    assert_eq!(tree_text(&doc), text);
    assert_eq!(doc.take_changes(), [], "the feed holds none of it");
    assert_eq!(doc.presentation_epoch(), epoch);
    assert!(
        !doc.tick_presentations(500.0),
        "nothing moved at the same instant"
    );
    assert_eq!(first_span(&doc, 2), "two");
}

/// A glyph-layer instance lives in the layer and writes nothing to the
/// document: it is listed with its elements and its run mutations, its
/// progress moves with the clock and a frame in which it moved owes a
/// draw, and the document, its feed and its save are untouched.
#[test]
fn test_a_glyph_layer_instance_never_touches_the_document() {
    let shimmer = presentation(
        r##"{"id":"shimmer","name":"S","play":"once","layer":"glyph","target":"sections",
"glyph":[{"run_opacity":0.5},{"run_overrides":{"scale":{"range":[0,2],"value":1.5}}},{"run_overrides":{"color":{"range":[1,2],"value":"#ff000080"}}}],
"animate":{"ms":100,"snap":"start"}}"##,
    );
    let mut doc = map();
    let one = n(&doc, 1);
    let ids = doc.start_presentation(&shimmer, &[one], 0.0).unwrap();
    assert_eq!(ids.len(), 1);
    let sections: Vec<NodeId> = kids(&doc, one, DocKind::Section)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    {
        let instance = doc.presentations().next().unwrap();
        assert_eq!(instance.layer, PresentationLayer::Glyph);
        assert_eq!(instance.play, Play::Once);
        assert_eq!(instance.elements, sections);
        let ops = instance
            .glyph
            .expect("a glyph instance carries its run mutations");
        assert_eq!(ops.0.len(), 3);
        assert_eq!(ops.0[0], GlyphOpDto::Opacity(0.5));
        assert_eq!(instance.snap, crate::engine::interpolate::Snap::Start);
    }
    assert!(
        doc.tick_presentations(50.0),
        "a glyph instance that moved owes a draw"
    );
    assert_eq!(doc.presentations().next().unwrap().t, 0.5);
    assert!(same_document(&doc, &map()).is_ok());
    assert_eq!(doc.take_changes(), []);
    assert_eq!(doc.save_json().unwrap(), written(&map()));
    let work = work_of(|| {
        doc.apply(place(vec![one], Vec2::ONE), ChangeSource::Action)
            .unwrap();
    });
    assert_eq!(work, [0, 0, 0], "an edit has no layer to take out");
    assert!(doc.tick_presentations(200.0), "its end owes a draw too");
    assert_eq!(doc.presentations().count(), 0);
    // A document-layer instance is listed with no run mutations.
    doc.start_presentation(&presentation(LIFT), &[one], 0.0).unwrap();
    assert!(doc.presentations().next().unwrap().glyph.is_none());
    let _: InstanceId = ids[0];
}

/// A load accepts `hidden` and `opacity` — a presentation conceals and
/// fades (D17) — and refuses a structural command: the format has no
/// spelling for one, so each way of writing one is refused by the
/// reader at its path. The same holds whichever file it comes from: a
/// map's list, or a presentations file (the app's, the user's).
#[test]
fn test_a_load_accepts_hidden_and_opacity_and_refuses_structure() {
    let concealed = presentation(CONCEALED);
    assert_eq!(concealed.play, Play::Hold);
    assert!(matches!(concealed.body, PresentationBody::Mutations(_)));
    let with = |mutation: &str| {
        format!(r#"{{"id":"p","name":"P","play":"hold","target":"node","mutations":[{mutation}]}}"#)
    };
    // Control: the same definition with a field op reads.
    presentation(&with(r#"{"node":{"set_folded":true}}"#));
    for structural in [
        r#"{"node":{"remove":true}}"#,
        r#"{"node":{"insert_child":{"id":9}}}"#,
        r#"{"node":{"move":{"to":1}}}"#,
        r#"{"link":{"set_kind":"portal"}}"#,
        r#"{"structure":{"remove":true}}"#,
        r#"{"section":{"split":{"at":1}}}"#,
    ] {
        let (path, kind) = refusal(&with(structural));
        assert!(
            path.starts_with("presentations[0].mutations[0]"),
            "{structural}: refused at its path, not at {path}"
        );
        assert!(
            matches!(&kind, LoadErrorKind::Invalid { message } if message.contains("unknown variant")),
            "{structural}: {kind:?}"
        );
        // In a map's own list: the same refusal, at the same path.
        let map = format!(
            r#"{{"format":1,"name":"t","presentations":[{}],"nodes":[]}}"#,
            with(structural)
        );
        let error = load(&map).unwrap_err();
        assert!(error.path.starts_with("presentations[0].mutations[0]"), "{error}");
    }
    // A mutator AST, an edit's other body, is no body of a presentation.
    let (path, _) = refusal(
        r#"{"id":"p","name":"P","play":"hold","target":"node","mutator":{"void":{"kind":"node","channel":0}}}"#,
    );
    assert_eq!(path, "presentations[0].mutator");
}

/// What a load refuses besides: a body that is not its layer's, a
/// glyph-layer target that draws no run, a mutation aimed at another
/// kind, a loop that takes no time, a timing past its bound, a run
/// mutation's number out of bounds, an unknown key, a face this build
/// lacks. Each beside the spelling that reads.
#[test]
fn test_a_load_refuses_what_no_start_could_play() {
    let invalid = |json: &str| match refusal(json) {
        (path, LoadErrorKind::InvalidPresentation { message }) => (path, message),
        (path, kind) => panic!("{json}: {kind:?} at {path}"),
    };
    let glyph = |target: &str, ops: &str| {
        format!(
            r#"{{"id":"g","name":"G","play":"hold","layer":"glyph","target":"{target}","glyph":[{ops}]}}"#
        )
    };
    // The body is the layer's.
    let (path, message) =
        invalid(r#"{"id":"p","name":"P","play":"hold","target":"node","glyph":[{"run_opacity":0.5}]}"#);
    assert_eq!(path, "presentations[0]");
    assert!(message.contains("this one has `glyph`"), "{message}");
    let (_, message) = invalid(r#"{"id":"p","name":"P","play":"hold","target":"node"}"#);
    assert!(message.contains("this one has neither"), "{message}");
    let (_, message) =
        invalid(r#"{"id":"p","name":"P","play":"hold","target":"sections","mutations":[],"glyph":[]}"#);
    assert!(message.contains("this one has both"), "{message}");
    let (_, message) =
        invalid(r#"{"id":"p","name":"P","play":"hold","layer":"glyph","target":"sections","mutations":[]}"#);
    assert!(
        message.contains("a glyph-layer presentation has `glyph`"),
        "{message}"
    );

    // A glyph-layer target draws runs of its own.
    for target in ["sections", "rings", "connector", "links", "labels"] {
        presentation(&glyph(target, r#"{"run_opacity":0.5}"#));
    }
    for target in ["node", "frame", "spans"] {
        let (path, message) = invalid(&glyph(target, r#"{"run_opacity":0.5}"#));
        assert_eq!(path, "presentations[0].target");
        assert!(
            message.ends_with("its target is one of `sections`, `rings`, `connector`, `links`, `labels`"),
            "{target}: {message}"
        );
    }
    // On the document layer every target reads.
    for target in ["node", "frame", "spans"] {
        presentation(&format!(
            r#"{{"id":"p","name":"P","play":"hold","target":"{target}","mutations":[]}}"#
        ));
    }

    // A mutation applies to the target's kind.
    let (path, message) = invalid(
        r#"{"id":"p","name":"P","play":"hold","target":"sections","mutations":[{"node":{"set_opacity":0.5}}]}"#,
    );
    assert_eq!(path, "presentations[0].mutations[0]");
    assert!(message.contains("does not apply"), "{message}");

    // A loop takes time, in or out.
    let looped = |timing: &str| {
        format!(r#"{{"id":"p","name":"P","play":"loop","target":"node","mutations":[]{timing}}}"#)
    };
    let (path, message) = invalid(&looped(""));
    assert_eq!(path, "presentations[0]");
    assert!(message.contains("at no place at any instant"), "{message}");
    invalid(&looped(r#","animate":{"ms":0},"return":{"ms":0}"#));
    presentation(&looped(r#","animate":{"ms":1}"#));
    presentation(&looped(r#","return":{"ms":1}"#));
    presentation(&looped(r#","animate":{"ms":0,"delay_ms":1}"#));
    // Every other lifecycle may be instant.
    presentation(r#"{"id":"p","name":"P","play":"once","target":"node","mutations":[]}"#);

    // A timing is at most a minute, each way, delay or duration.
    for (key, field) in [
        ("animate", "ms"),
        ("animate", "delay_ms"),
        ("return", "ms"),
        ("return", "delay_ms"),
    ] {
        let timed = |ms: u32| {
            let timing = if field == "ms" {
                format!(r#"{{"ms":{ms}}}"#)
            } else {
                format!(r#"{{"ms":1,"delay_ms":{ms}}}"#)
            };
            format!(
                r#"{{"id":"p","name":"P","play":"hold","target":"node","mutations":[],"{key}":{timing}}}"#
            )
        };
        presentation(&timed(MAX_ANIMATE_MS));
        let (path, kind) = refusal(&timed(MAX_ANIMATE_MS + 1));
        assert_eq!(path, format!("presentations[0].{key}.{field}"));
        assert!(matches!(kind, LoadErrorKind::Bound { .. }), "{kind:?}");
    }

    // A run mutation's numbers.
    let bound = |ops: &str| match refusal(&glyph("sections", ops)) {
        (path, LoadErrorKind::Bound { error }) => (path, error),
        (path, kind) => panic!("{ops}: {kind:?} at {path}"),
    };
    assert_eq!(
        bound(r#"{"run_opacity":1.5}"#),
        (
            "presentations[0].glyph[0].run_opacity".to_string(),
            BoundError::Above { value: 1.5, max: 1.0 }
        )
    );
    let (path, error) = bound(r#"{"run_opacity":0.5},{"run_origin":[0.0,1e39]}"#);
    assert_eq!(path, "presentations[0].glyph[1].run_origin[1]");
    assert!(matches!(error, BoundError::NotFinite(_)), "{error:?}");
    let (path, _) = bound(r#"{"run_overrides":{"offset":{"range":[0,1],"value":[1e39,0.0]}}}"#);
    assert_eq!(path, "presentations[0].glyph[0].run_overrides.offset.value[0]");
    let (path, _) = bound(r#"{"run_overrides":{"scale":{"range":[0,1],"value":-1.0}}}"#);
    assert_eq!(path, "presentations[0].glyph[0].run_overrides.scale.value");
    // A color is written, `null` included; a variable is not a literal.
    presentation(&glyph(
        "sections",
        r#"{"run_overrides":{"color":{"range":[0,1],"value":null}}}"#,
    ));
    for unread in [
        r#"{"run_overrides":{"color":{"range":[0,1]}}}"#,
        r#"{"run_overrides":{"color":{"range":[0,1],"value":"var(accent)"}}}"#,
        r#"{"run_overrides":{"scale":{"range":[0,1],"value":1.0,"extra":1}}}"#,
        r#"{"run_overrides":{"scale":{"range":[0],"value":1.0}}}"#,
        r#"{"run_set":{}}"#,
        r#"{"fill_opacity":0.5}"#,
    ] {
        let (path, kind) = refusal(&glyph("sections", unread));
        assert!(path.starts_with("presentations[0].glyph[0]"), "{unread}: {path}");
        assert!(
            matches!(
                kind,
                LoadErrorKind::Invalid { .. }
                    | LoadErrorKind::MissingKey { .. }
                    | LoadErrorKind::UnknownKey { .. }
            ),
            "{unread}: refused by the reader, as {kind:?}"
        );
    }
    assert_eq!(
        refusal(&glyph(
            "sections",
            r#"{"run_overrides":{"color":{"range":[0,1]}}}"#
        ))
        .1,
        LoadErrorKind::MissingKey {
            key: "value".to_string()
        },
        "a color is written, `null` included"
    );

    // An unknown key, an unknown play, a font family name that cannot be
    // one; a family no compiled face bears reads, as an installed one
    // (#329).
    let (path, _) =
        refusal(r#"{"id":"p","name":"P","play":"hold","target":"node","mutations":[],"reach":"self"}"#);
    assert_eq!(path, "presentations[0].reach");
    let (path, _) = refusal(r#"{"id":"p","name":"P","play":"toggle","target":"node","mutations":[]}"#);
    assert_eq!(path, "presentations[0].play");
    let (path, kind) = refusal(
        r#"{"id":"p","name":"P","play":"hold","target":"spans","mutations":[{"span":{"text":{"set_font":"Bell\u0007"}}}]}"#,
    );
    assert_eq!(path, "presentations[0].mutations[0]");
    assert_eq!(kind, LoadErrorKind::NameNotPrintable { code_point: 7 });
    assert!(
        parse_presentations(
            r#"{"presentations":[{"id":"p","name":"P","play":"hold","target":"spans","mutations":[{"span":{"text":{"set_font":"No Such Face"}}}]}]}"#
        )
        .is_ok(),
        "an installed family, present or not, reads"
    );
    // Ids are unique within a file.
    let twice = r#"{"presentations":[{"id":"p","name":"P","play":"hold","target":"node","mutations":[]},{"id":"p","name":"Q","play":"hold","target":"node","mutations":[]}]}"#;
    let error = parse_presentations(twice).unwrap_err();
    assert_eq!(error.path, "presentations[1].id");
}

/// A definition has at most 65,536 parts — mutations, run mutations and
/// filter tests — and a text nested past what a definition can nest is
/// refused as too deep a definition, never as the JSON reader's
/// "recursion limit exceeded".
///
/// Controls: one part fewer reads; the same deep text read by the
/// reader alone gives the reader's own error.
#[test]
fn test_a_definition_is_held_to_the_definition_bounds() {
    let sized = |mutations: usize, tests: usize| {
        let mutations = vec![r#"{"node":{"set_folded":true}}"#; mutations].join(",");
        let tests = vec![r#"[{"folded":true},{"Equals":false}]"#; tests].join(",");
        format!(
            r#"{{"id":"p","name":"P","play":"hold","target":"node","filter":{{"fields":[{tests}]}},"mutations":[{mutations}]}}"#
        )
    };
    presentation(&sized(MAX_DEFINITION_PARTS - 1, 1));
    let (path, kind) = refusal(&sized(MAX_DEFINITION_PARTS, 1));
    assert_eq!(path, "presentations[0]");
    assert_eq!(
        kind,
        LoadErrorKind::DefinitionTooLarge {
            max: MAX_DEFINITION_PARTS
        }
    );
    let glyph = |ops: usize| {
        let ops = vec![r#"{"run_opacity":0.5}"#; ops].join(",");
        format!(
            r#"{{"id":"g","name":"G","play":"hold","layer":"glyph","target":"sections","glyph":[{ops}]}}"#
        )
    };
    presentation(&glyph(MAX_DEFINITION_PARTS));
    assert_eq!(
        refusal(&glyph(MAX_DEFINITION_PARTS + 1)).1,
        LoadErrorKind::DefinitionTooLarge {
            max: MAX_DEFINITION_PARTS
        }
    );

    // Nesting: 129 levels and 400 levels inside `presentations`, in a
    // file and in a map.
    for depth in [129, 400] {
        let deep = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        let json = format!(
            r#"{{"id":"p","name":"P","play":"hold","target":"node","mutations":[],"filter":{{"fields":{deep}}}}}"#
        );
        let (path, kind) = refusal(&json);
        assert_eq!(path, "presentations[0]");
        assert!(
            matches!(kind, LoadErrorKind::DefinitionTooDeep { .. }),
            "{kind:?}"
        );
        let map = format!(r#"{{"format":1,"name":"t","presentations":[{json}],"nodes":[]}}"#);
        let error = load(&map).unwrap_err();
        assert!(
            matches!(error.kind, LoadErrorKind::DefinitionTooDeep { .. }),
            "{error}"
        );
        assert_eq!(error.path, "presentations[0]");
        let raw: Result<serde_json::Value, _> = serde_json::from_str(&json);
        assert!(
            raw.unwrap_err().to_string().contains("recursion limit exceeded"),
            "the control: the reader alone stops at its own limit"
        );
    }
}

/// A start resolves a definition's names against the open map: the same
/// definition means the same on two maps whose themes order their
/// variables differently, and a name the map lacks refuses the start,
/// naming the presentation, the field and the name, with nothing
/// started. A definition built in code is held to the layer's rules at
/// the start, as a file's is at load.
#[test]
fn test_a_start_resolves_names_and_refuses_what_the_map_lacks() {
    let tint = presentation(
        r#"{"id":"tint","name":"T","play":"hold","target":"spans","mutations":[{"span":{"text":{"set_color":"var(accent)"}}}]}"#,
    );
    let themed = |variables: &str| {
        load(&format!(
            r##"{{"format":1,"name":"t","theme":{{"active":"d","variants":{{"d":{{{variables}}}}}}},"nodes":[{{"id":1,"position":[0,0],"sections":[{{"spans":[{{"text":"a"}}]}}]}}]}}"##
        ))
        .unwrap()
    };
    for (variables, index) in [
        (r##""accent":"#ff0000","zeta":"#000000""##, 0),
        (r##""aaa":"#000000","accent":"#ff0000""##, 1),
    ] {
        let mut doc = themed(variables);
        let one = n(&doc, 1);
        doc.start_presentation(&tint, &[one], 0.0).unwrap();
        let section = kid(&doc, one, DocKind::Section, 0).0;
        let span = kid(&doc, section, DocKind::Span, 0).1;
        assert!(
            matches!(span, DocElement::Span(span) if span.style.color == Some(ColorRef::Var(crate::document::values::VarId(index)))),
            "{variables}: {span:?}"
        );
    }
    let mut lacking = themed(r##""bg":"#000000""##);
    let one = n(&lacking, 1);
    let refused = lacking.start_presentation(&tint, &[one], 0.0).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "presentation `tint`: `set_color` names theme variable `accent`, which this map does not define"
    );
    assert_eq!(lacking.presentations().count(), 0);
    assert_eq!(lacking.presentation_epoch(), 0);
    assert_eq!(lacking.take_changes(), []);

    // Built in code, past what a load lets through: a body of the other
    // layer, and a loop that takes no time.
    let mut doc = map();
    let two = n(&doc, 2);
    let mut crossed = presentation(LIFT);
    crossed.layer = PresentationLayer::Glyph;
    let refused = doc.start_presentation(&crossed, &[two], 0.0).unwrap_err();
    assert!(
        matches!(&refused, PresentError::Value { field: "layer", .. }),
        "{refused}"
    );
    let mut timeless = presentation(GLOW);
    timeless.play = Play::Loop;
    let refused = doc.start_presentation(&timeless, &[two], 0.0).unwrap_err();
    assert!(
        matches!(&refused, PresentError::Value { field: "play", .. }),
        "{refused}"
    );
    assert_eq!(doc.presentations().count(), 0);
    // A write the document refuses starts nothing, and leaves nothing
    // of itself applied: a size below zero.
    let sized = presentation(
        r#"{"id":"sized","name":"S","play":"hold","target":"rings","mutations":[{"ring":{"text":{"set_size":-1.0}}}]}"#,
    );
    let refused = doc.start_presentation(&sized, &[n(&doc, 1)], 0.0).unwrap_err();
    assert!(matches!(refused, PresentError::Refused { .. }), "{refused}");
    assert_eq!(doc.presentations().count(), 0);
    assert!(
        same_document(&doc, &map()).is_ok(),
        "nothing of it is left applied"
    );
}

/// A saved map carries its presentations as it read them, and a
/// direction written as its default is saved by leaving it out.
#[test]
fn test_a_maps_presentations_are_saved_as_they_were_read() {
    let map = format!(
        r#"{{"format":1,"name":"t","presentations":[{CONCEALED},{LIFT},{{"id":"s","name":"S","play":"loop","layer":"glyph","target":"rings","glyph":[{{"run_overrides":"clear"}}],"animate":{{"ms":0}},"return":{{"ms":50,"easing":"linear","snap":"end"}}}}],"nodes":[]}}"#
    );
    let doc = load(&map).unwrap();
    let saved = written(&doc);
    let again = load(&saved).unwrap();
    assert_eq!(doc.definitions(), again.definitions());
    assert_eq!(written(&again), saved);
    let value: serde_json::Value = serde_json::from_str(&saved).unwrap();
    let shimmer = &value["presentations"][2];
    assert_eq!(shimmer["layer"], "glyph");
    assert!(
        shimmer.get("animate").is_none(),
        "an instant way in is the default"
    );
    assert_eq!(shimmer["return"], serde_json::json!({"ms": 50}));
    assert!(
        value["presentations"][0].get("layer").is_none(),
        "the document layer is the default"
    );
    assert_eq!(
        value["presentations"][1]["animate"],
        serde_json::json!({"ms": 200})
    );
    assert!(value["presentations"][1].get("return").is_none());
}

/// Every part of a refused start that may hold what a file wrote — the
/// presentation's id, the name it gives a field, a message that quotes
/// one — is shown as file text is, every control, line separator and
/// bidi control escaped, as an edit's refusal shows it (#365).
///
/// Control, seen red: `PresentError`'s parts shown as written.
#[test]
fn test_a_refused_starts_text_from_the_file_is_escaped() {
    let raw = "a\u{202e}b\u{2028}c\u{85}d\u{1b}e";
    let errors = [
        PresentError::UnknownName {
            presentation: raw.to_string(),
            field: "color",
            name: format!("theme variable `{raw}`"),
        },
        PresentError::Value {
            presentation: raw.to_string(),
            field: "span",
            message: raw.to_string(),
        },
        PresentError::Refused {
            presentation: raw.to_string(),
            error: Box::new(ApplyError::TooMuchText { bytes: 2, max: 1 }),
        },
    ];
    let dangerous = |c: char| {
        c.is_control()
            || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
    };
    for error in errors {
        let shown = error.to_string();
        assert!(!shown.chars().any(dangerous), "{error:?} shows {shown:?}");
        assert!(
            shown.contains("a\\u202eb"),
            "{error:?} shows its text escaped: {shown}"
        );
    }
}

/// What a presentation writes that its map must define is what `mentions`
/// reports, and `Mention::is_defined_in` answers as a start does: for
/// each place a presentation can write a verb, a theme variable or a
/// palette, the one name it writes is reported with the key the file
/// writes it at as its field and its JSON path, a map that defines it
/// starts the presentation, and a map that does not refuses the start
/// with exactly the refusal `Mention::presentation_refusal` words.
///
/// Controls, each run red: `is_defined_in` answering `true` for a palette
/// fails the palette cases on the bare map; answering `false` fails them
/// on the full one; dropping `Walk::color_in`'s `Var` arm fails the variable
/// case, and the filter `on_click` arm its case.
#[test]
fn test_the_names_a_presentation_mentions_are_the_ones_a_start_refuses() {
    use crate::document::edit::{Mention, Named};

    const NODE: &str = r#"{"id":1,"position":[0,0],"sections":[{"spans":[{"text":"x"}]}]}"#;
    let load_full = || {
        load(&format!(
            r##"{{"format":1,"name":"t","theme":{{"active":"d","variants":{{"d":{{"accent":"#ff0000"}}}}}},
            "verbs":[{{"id":"go","name":"Go","steps":[]}}],
            "palettes":{{"warm":[{{"frame":"#ff0000","text":"#ff0000"}}]}},"nodes":[{NODE}]}}"##
        ))
        .unwrap()
    };
    let load_bare = || load(&format!(r#"{{"format":1,"name":"t","nodes":[{NODE}]}}"#)).unwrap();
    let filtered = |field: &str| {
        format!(
            r#""filter":{{"fields":[[{field},{{"Equals":false}}]]}},"mutations":[{{"node":{{"set_folded":true}}}}]"#
        )
    };
    let cases: Vec<(&str, Named, &str, &str, String)> = vec![
        (
            "set_click",
            Named::Verb,
            "set_click",
            "mutations[0].node.set_click",
            r#""mutations":[{"node":{"set_click":"go"}}]"#.to_string(),
        ),
        (
            "set_palette",
            Named::Palette,
            "set_palette",
            "mutations[0].node.set_palette",
            r#""mutations":[{"node":{"set_palette":"warm"}}]"#.to_string(),
        ),
        (
            "text color",
            Named::Variable,
            "set_color",
            "mutations[0].node.text.set_color",
            r#""mutations":[{"node":{"text":{"set_color":"var(accent)"}}}]"#.to_string(),
        ),
        (
            "on_click",
            Named::Verb,
            "on_click",
            "filter.fields[0][0].on_click",
            filtered(r#"{"on_click":"go"}"#),
        ),
        (
            "palette",
            Named::Palette,
            "palette",
            "filter.fields[0][0].palette",
            filtered(r#"{"palette":"warm"}"#),
        ),
    ];
    let mut wrong = Vec::new();
    for (way, kind, field, path, body) in &cases {
        let def = presentation(&format!(
            r#"{{"id":"p","name":"P","play":"hold","target":"node",{body}}}"#
        ));
        let mut said: Vec<Mention<'_>> = Vec::new();
        def.mentions(|mention| said.push(mention));
        let [mention] = said.as_slice() else {
            wrong.push(format!("{way}: mentions {said:?}, not one name"));
            continue;
        };
        if (mention.kind, mention.field, mention.path.as_str()) != (*kind, *field, *path) {
            wrong.push(format!("{way}: mentions {mention:?}"));
        }
        let start = |mut doc: Document| {
            let one = n(&doc, 1);
            doc.start_presentation(&def, &[one], 0.0).err()
        };
        if !mention.is_defined_in(&load_full()) || start(load_full()).is_some() {
            wrong.push(format!("{way}: not defined, or refused, on the full map"));
        }
        if mention.is_defined_in(&load_bare())
            || start(load_bare()) != Some(mention.presentation_refusal("p"))
        {
            wrong.push(format!(
                "{way}: defined, or not refused as worded, on the bare map"
            ));
        }
    }
    assert_eq!(wrong, Vec::<String>::new());
}
