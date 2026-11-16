// SPDX-License-Identifier: MPL-2.0

//! Map verbs in the app (V1-D3, #252): the hover held on a node and
//! released, the click on a node and on a linked span, an address
//! opened through the opener's seam, the frame resuming a run that
//! waits, a verb that never settles an animated edit to start a
//! presentation, an edit step that yields to an interactive
//! transaction, and the macro `Verb` step — each through the runner
//! every route takes (`dispatch::families::verbs`), on an app clock the
//! test moves.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::ids::NodeIdent;
use baumhard::document::presentation::Direction;
use baumhard::document::writer::ChangeSource;
use baumhard::projection::Hit;

use crate::application::clock::{AppClock, ClockMode};
use crate::application::document::{Interaction, MindMapDocument};
use crate::application::edits::EditRegistry;
use crate::application::macros::MacroTarget;
use crate::application::platform::opener::take_opened;

use super::animation;
use super::dispatch::families::edits::Undrawn;
use super::dispatch::families::verbs::{self, Host};

/// A map whose node 1 holds `go` on its click and `lift` on its hover,
/// whose node 2 has a span linking to `go` and one linking to an
/// address, and whose presentations are `p` (`while`, in over 100 ms,
/// out over 200) and `h` (`hold`). Node 5 is another top-level node.
const MAP: &str = r##"{"format":1,"name":"verbs",
"edits":[{"id":"e","name":"E","reach":"self","target":"node","mutations":[{"node":{"nudge":[10.0,0.0]}}]},
 {"id":"slow","name":"Slow","reach":"self","target":"node","mutations":[{"node":{"nudge":[0.0,100.0]}}],"animate":{"ms":1000}}],
"presentations":[
 {"id":"p","name":"P","play":"while","target":"node","mutations":[{"node":{"set_opacity":0.5}}],"animate":{"ms":100},"return":{"ms":200}},
 {"id":"h","name":"H","play":"hold","target":"node","mutations":[{"node":{"set_hidden":true}}]}
],
"verbs":[
 {"id":"go","name":"Go","steps":[{"present":"h","on":{"nodes":[5]}}]},
 {"id":"lift","name":"Lift","steps":[{"present":"p","on":"self"},{"wait":300},{"present":"h","on":"children"}]},
 {"id":"edits","name":"Edits","steps":[{"edit":"e","on":"self"},{"present":"h","on":{"nodes":[5]}}]}
],
"nodes":[
 {"id":1,"position":[0,0],"on":{"click":"go","hover":"lift"},"sections":[{"spans":[{"text":"one"}]}]},
 {"id":2,"parent":1,"position":[300,0],"sections":[{"spans":[{"text":"go","link":{"verb":"go"}},{"text":"away","link":{"url":"https://example.org/page"}},{"text":"plain"}]}]},
 {"id":5,"position":[900,0],"sections":[{"spans":[{"text":"five"}]}]}
]}"##;

fn map() -> MindMapDocument {
    MindMapDocument::from_json_str(MAP, None, 50).expect("the test map loads")
}

fn node(doc: &MindMapDocument, ident: u32) -> NodeId {
    doc.doc().node_by_ident(NodeIdent(ident)).expect("a node")
}

/// The spans of node 2's section, in order.
fn spans(doc: &MindMapDocument) -> Vec<NodeId> {
    let d = doc.doc();
    let section = d
        .children(node(doc, 2))
        .find(|&c| d.get(c).is_some_and(|e| e.kind() == DocKind::Section))
        .expect("a section");
    d.children(section).collect()
}

/// What a pointer on `element` of `node` — on `span`, if any — hits.
fn hit(node: NodeId, element: NodeId, span: Option<NodeId>) -> Hit {
    Hit {
        node,
        element,
        span,
        glyph: span.map(|_| 0),
    }
}

/// The nodes, by ident, each instance of `id` plays on, and which way.
fn playing(doc: &MindMapDocument, id: &str) -> Vec<(u32, Direction)> {
    let d = doc.doc();
    d.presentations()
        .filter(|instance| instance.presentation == id)
        .map(|instance| {
            let ident = d
                .get(instance.node)
                .and_then(|e| e.as_node())
                .expect("a node")
                .ident
                .0;
            (ident, instance.direction)
        })
        .collect()
}

fn registry() -> EditRegistry {
    EditRegistry::new(Vec::new(), Vec::new())
}

/// A virtual clock, and its reading.
fn clock() -> (AppClock, f64) {
    let mut clock = AppClock::default();
    clock.set_mode(ClockMode::Virtual);
    let now = clock.now_ms();
    (clock, now)
}

/// **A hover holds its verb, and leaving releases it**: entering node 1
/// starts `lift`'s `while` presentation on it, and the run waits; the
/// pointer leaving before the wait is over stops `p` — it plays its way
/// out — and the steps after the wait never run, however long the frames
/// go on. Entering again starts it again; moving to another node
/// releases the first before anything else.
///
/// Controls, each seen red: a `hover` that does not release the held run
/// leaves `p` going in; a release that lets the run go on starts `h` on
/// node 2 at 300 ms.
#[test]
fn test_a_hover_holds_its_verb_and_leaving_releases_it() {
    let mut doc = map();
    let (one, two) = (node(&doc, 1), node(&doc, 2));
    let (mut clock, start) = clock();
    let edits = registry();
    let tick = |doc: &mut MindMapDocument, clock: &AppClock| {
        animation::tick(Some(doc), &mut Undrawn, &registry(), clock);
    };
    // The hover's target is the node the pointer is on whose `on.hover`
    // names a verb, and nothing else.
    assert_eq!(verbs::hovered(doc.doc(), Some(&hit(one, one, None))), Some(one));
    assert_eq!(verbs::hovered(doc.doc(), Some(&hit(two, two, None))), None);
    assert_eq!(verbs::hovered(doc.doc(), None), None);

    let host = |doc: &mut MindMapDocument, on: Option<NodeId>, at: f64| {
        Host {
            doc,
            drawn: &mut Undrawn,
            edits: &edits,
        }
        .hover(on, at)
    };
    assert!(host(&mut doc, Some(one), start));
    assert!(!host(&mut doc, Some(one), start + 10.0), "staying is no change");
    assert_eq!(playing(&doc, "p"), [(1, Direction::In)]);
    assert!(doc.runs().waiting(), "the run waits");
    clock.advance_to(start + 150.0);
    tick(&mut doc, &clock);
    assert!(host(&mut doc, None, start + 150.0), "leaving");
    assert_eq!(playing(&doc, "p"), [(1, Direction::Out)], "it plays its way out");
    assert!(doc.hover().is_none() && doc.runs().runs().next().is_none());
    clock.advance_to(start + 1_000.0);
    tick(&mut doc, &clock);
    assert!(
        playing(&doc, "h").is_empty(),
        "the steps after the wait never ran"
    );
    assert!(playing(&doc, "p").is_empty(), "and `p` played out");

    // Entered again and held past the wait: the rest runs; moving to
    // node 5, which has no hover, releases it.
    assert!(host(&mut doc, Some(one), start + 1_000.0));
    clock.advance_to(start + 1_400.0);
    tick(&mut doc, &clock);
    assert_eq!(
        playing(&doc, "h"),
        [(2, Direction::Hold)],
        "after the wait, its children"
    );
    let five = node(&doc, 5);
    assert!(host(&mut doc, Some(five), start + 1_400.0));
    assert_eq!(doc.hover(), None, "node 5 holds no hover");
    assert_eq!(playing(&doc, "p"), [(1, Direction::Out)]);
    assert_eq!(playing(&doc, "h"), [(2, Direction::Hold)], "a `hold` plays on");
}

/// **A click runs a node's `on.click`, and a linked span's link**: on
/// node 1's body it runs `go`; on node 2's span that links to `go` it
/// runs `go` with `self` the span's node; on the span that links to an
/// address it opens the address through the opener's seam, and runs
/// nothing; on a plain span of node 2, which has no click, it runs
/// nothing. What a click triggers something on is where the pointer
/// shows a hand.
///
/// Controls, each seen red: `click_of` leaving the span's link unread
/// runs nothing on the linked spans; `Click::Url` not handed to the
/// opener records no address.
#[test]
fn test_a_click_runs_the_node_verb_and_the_span_link() {
    let mut doc = map();
    let (one, two) = (node(&doc, 1), node(&doc, 2));
    let [linked, away, plain] = spans(&doc)[..] else { panic!("three spans") };
    let edits = registry();
    let _ = take_opened();
    let click = |doc: &mut MindMapDocument, hit: &Hit| {
        Host {
            doc,
            drawn: &mut Undrawn,
            edits: &edits,
        }
        .click(Some(hit), 0.0)
    };
    for (what, at, triggers) in [
        ("node 1", hit(one, one, None), true),
        ("the verb link", hit(two, two, Some(linked)), true),
        ("the address", hit(two, two, Some(away)), true),
        ("a plain span", hit(two, two, Some(plain)), false),
        ("node 2's body", hit(two, two, None), false),
    ] {
        assert_eq!(
            verbs::points_at(doc.doc(), Some(&at)),
            triggers,
            "{what}: the hand"
        );
    }
    assert!(click(&mut doc, &hit(one, one, None)));
    assert_eq!(playing(&doc, "h"), [(5, Direction::Hold)]);
    assert!(take_opened().is_empty());
    let mut doc = map();
    assert!(click(&mut doc, &hit(two, two, Some(linked))));
    assert_eq!(playing(&doc, "h"), [(5, Direction::Hold)]);
    assert!(click(&mut doc, &hit(two, two, Some(away))));
    assert_eq!(take_opened(), ["https://example.org/page"]);
    let mut doc = map();
    assert!(!click(&mut doc, &hit(two, two, Some(plain))));
    assert!(!click(&mut doc, &hit(two, two, None)));
    assert!(playing(&doc, "h").is_empty() && take_opened().is_empty());
}

/// **A verb never settles an animated edit to start a presentation**:
/// with `slow` animating, a hover starts `p` and the edit is still
/// animating, at the place its own clock puts it. An edit step does
/// settle it (D28): it is an apply.
///
/// Control, seen red: `Host::hover` calling `finish_animation` before
/// it invokes commits `slow` at once.
#[test]
fn test_a_hover_does_not_settle_an_animated_edit() {
    let mut doc = map();
    let one = node(&doc, 1);
    let edits = EditRegistry::new(doc.doc().definitions().edits.clone(), Vec::new());
    let five = node(&doc, 5);
    super::dispatch::families::edits::run_edit(
        &mut doc,
        &mut Undrawn,
        &edits,
        "slow",
        Some(&[five]),
        ChangeSource::Edit,
        0.0,
    )
    .expect("slow runs");
    assert!(doc.is_animating(), "precondition: the edit animates");
    let mut host = Host {
        doc: &mut doc,
        drawn: &mut Undrawn,
        edits: &edits,
    };
    assert!(host.hover(Some(one), 10.0));
    assert!(doc.is_animating(), "the hover left it running");
    assert_eq!(doc.doc().history().count(), 0, "and uncommitted");
    assert_eq!(playing(&doc, "p"), [(1, Direction::In)]);
    // An edit step is an apply, and settles it.
    Host {
        doc: &mut doc,
        drawn: &mut Undrawn,
        edits: &edits,
    }
    .invoke("edits", Some(one), false, 20.0)
    .expect("runs");
    assert!(!doc.is_animating());
    let sources: Vec<ChangeSource> = doc.doc().history().map(|entry| entry.source).collect();
    assert_eq!(sources, [ChangeSource::Edit, ChangeSource::Verb]);
}

/// **An edit step yields to an interactive transaction (D28)**: with one
/// open, the step is skipped with a warning — no history entry, the map
/// unchanged, and no status line, which a change the user made and the
/// writer refused would show — and the run goes on to its next step.
/// With none open the same verb makes its entry.
///
/// Control, seen red: without the performer's check the step reaches
/// the writer, which refuses it and sets the status line.
#[test]
fn test_an_edit_step_is_skipped_while_an_interaction_is_open() {
    let mut doc = map();
    let one = node(&doc, 1);
    let edits = EditRegistry::new(doc.doc().definitions().edits.clone(), Vec::new());
    assert!(doc.begin(
        ChangeSource::Input,
        baumhard::document::writer::transaction::TransactionKind::Interactive,
        Interaction::Test
    ));
    let before = doc.save_json().expect("saves");
    Host {
        doc: &mut doc,
        drawn: &mut Undrawn,
        edits: &edits,
    }
    .invoke("edits", Some(one), false, 0.0)
    .expect("the run itself is not refused");
    assert_eq!(doc.save_json().expect("saves"), before, "the edit was skipped");
    assert_eq!(
        doc.status(),
        None,
        "skipped with a warning, not refused on the status line as a change the user made would be"
    );
    assert_eq!(playing(&doc, "h"), [(5, Direction::Hold)], "the run went on");
    assert!(doc.cancel(Interaction::Test));
    // Control: with nothing open, the same verb edits.
    let mut doc = map();
    Host {
        doc: &mut doc,
        drawn: &mut Undrawn,
        edits: &edits,
    }
    .invoke("edits", Some(one), false, 0.0)
    .expect("runs");
    assert_eq!(doc.doc().history().count(), 1);
}

/// **The frame resumes a run that waits, and owes the loop frames for
/// it**: on a real clock a waiting run keeps the loop ticking; on a
/// virtual one it keeps nothing awake, and the tick at the wait's end
/// runs the rest. `clock.wait until=animations_complete` reads the same:
/// a waiting run is an animation moving.
///
/// Control, seen red: `animations_moving` not reading the runs lets
/// `keeps_ticking` answer no while the run waits.
#[test]
fn test_the_frame_resumes_a_waiting_run() {
    let mut doc = map();
    let one = node(&doc, 1);
    let edits = registry();
    let (mut clock, start) = clock();
    Host {
        doc: &mut doc,
        drawn: &mut Undrawn,
        edits: &edits,
    }
    .invoke("lift", Some(one), false, start)
    .expect("runs");
    // `p` arrives at 100 ms; at 200 only the run is moving.
    clock.advance_to(start + 200.0);
    animation::tick(Some(&mut doc), &mut Undrawn, &edits, &clock);
    assert!(
        !doc.presentations_moving() && !doc.is_animating(),
        "precondition: only the run"
    );
    assert!(doc.animations_moving());
    assert!(animation::keeps_ticking(Some(&doc), &AppClock::default()));
    assert!(
        !animation::keeps_ticking(Some(&doc), &clock),
        "a virtual clock keeps nothing awake"
    );
    clock.advance_to(start + 300.0);
    assert!(animation::tick(Some(&mut doc), &mut Undrawn, &edits, &clock));
    assert_eq!(playing(&doc, "h"), [(2, Direction::Hold)]);
    assert!(!doc.runs().waiting());
}

/// **A macro's `Verb` step runs the map verb on its target**: the node
/// it names; with `CurrentSelection`, the one selected node, or none
/// with nothing selected — and a verb that acts on `self` is then
/// refused, so the step did not run; a node the map lacks skips it.
#[test]
fn test_a_macro_verb_step_runs_on_its_target() {
    let edits = registry();
    let run = |doc: &mut MindMapDocument, id: &str, at: &MacroTarget| {
        verbs::run_step(doc, &mut Undrawn, &edits, id, at, 0.0)
    };
    let mut doc = map();
    assert!(run(&mut doc, "lift", &MacroTarget::NodeId(1)));
    assert_eq!(playing(&doc, "p"), [(1, Direction::In)]);
    let mut doc = map();
    assert!(
        !run(&mut doc, "lift", &MacroTarget::CurrentSelection),
        "nothing selected: no self"
    );
    assert!(
        run(&mut doc, "go", &MacroTarget::CurrentSelection),
        "control: a verb that names its nodes"
    );
    assert!(!run(&mut doc, "lift", &MacroTarget::NodeId(99)));
    let one = node(&doc, 1);
    doc.selection.select_only(one);
    assert!(run(&mut doc, "lift", &MacroTarget::CurrentSelection));
    assert_eq!(playing(&doc, "p"), [(1, Direction::In)]);
}
