// SPDX-License-Identifier: MPL-2.0

//! Field editing II, through both of its surfaces, on one fixture
//! (`../fixtures/paths.mandala.json`): every Action through the
//! family's body ([`super::run`], what `dispatch` runs before it syncs
//! the view) and every console form through its verb's `execute` (what
//! `execute_console_line` and IPC `act.console` run before they sync).
//!
//! The fixture: node 1 links to node 3 (a bare line), to node 4 (a line
//! drawn `─`, with a label) and to node 4 again (a portal); node 2,
//! node 1's child, has a connector drawn `─` with an end cap and a
//! label; node 3's line link is straight, colored and hidden; node 4's
//! passes through one control point, draws in a zoom window and holds a
//! span. The built-in content of a connector and a link is `·`.
//!
//! **The expectations are written by hand**, as the lines of the
//! fixture each edit changes: a case lists `(old, new)` replacements of
//! the fixture's text, and the document the edit makes must save to
//! exactly that text. They are not the output of a run (TEST_CONVENTIONS
//! §T13 rule 2). Each case is also exactly one history entry, and one
//! undo gives back the fixture's bytes.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::format;
use baumhard::document::ids::NodeIdent;
use baumhard::document::writer::DEFAULT_HISTORY_LIMIT;
use baumhard::projection::Layer;

use super::run;
use crate::application::app::dispatch::cross_dispatch::DispatchOutcome;
use crate::application::app::dispatch::families::projected::Projected;
use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;
use crate::application::fields::path::{LABEL_ON_A_PORTAL, LINK_KIND_ON_A_CONNECTOR};
use crate::application::keybinds::Action;
use crate::application::view::inspect::children_of_kind;

/// The fixture every case starts from.
const FIXTURE: &str = include_str!("../fixtures/paths.mandala.json");

/// The fixture, loaded, with nothing selected.
fn fixture() -> MindMapDocument {
    MindMapDocument::from_json_str(FIXTURE, None, DEFAULT_HISTORY_LIMIT).expect("the fixture loads")
}

/// The map as it saves.
fn bytes(doc: &MindMapDocument) -> String {
    format::save(doc.doc()).expect("saves")
}

/// The node with file id `ident`.
fn node(doc: &MindMapDocument, ident: u32) -> NodeId {
    doc.doc().node_by_ident(NodeIdent(ident)).expect("the node")
}

/// One path of the fixture.
#[derive(Clone, Copy, Debug)]
enum PathRef {
    /// Node `ident`'s connector.
    Connector(u32),
    /// Node `ident`'s link `nth`, in document order.
    Link(u32, usize),
}

/// Node 2's connector.
const C2: PathRef = PathRef::Connector(2);
/// Node 1's bare line link to node 3.
const L13: PathRef = PathRef::Link(1, 0);
/// Node 1's labeled line link to node 4.
const L14: PathRef = PathRef::Link(1, 1);
/// Node 1's portal link to node 4.
const P14: PathRef = PathRef::Link(1, 2);
/// Node 3's straight, hidden line link.
const L32: PathRef = PathRef::Link(3, 0);
/// Node 4's line link through one control point.
const L41: PathRef = PathRef::Link(4, 0);

/// The element `path` names.
fn path(doc: &MindMapDocument, path: PathRef) -> NodeId {
    let (ident, kind, nth) = match path {
        PathRef::Connector(ident) => (ident, DocKind::Connector, 0),
        PathRef::Link(ident, nth) => (ident, DocKind::Link, nth),
    };
    children_of_kind(doc.doc(), node(doc, ident), kind)
        .nth(nth)
        .expect("the path")
}

/// Select exactly `paths`, in order.
fn select(doc: &mut MindMapDocument, paths: &[PathRef]) {
    let ids: Vec<NodeId> = paths.iter().map(|&p| path(doc, p)).collect();
    doc.selection.set(ids);
}

/// What a case runs.
#[derive(Clone, Copy)]
enum Input {
    /// A console line, through its verb.
    Line(&'static str),
    /// An Action, through the family's body.
    Act(fn() -> Action),
}

/// Run `input` on `doc`: `Ok` with what a console line printed, or the
/// Action's outcome; `Err` with a console line's refusal, or an Action's
/// outcome other than `Handled`.
fn run_input(doc: &mut MindMapDocument, input: Input) -> Result<String, String> {
    match input {
        Input::Line(line) => run_line(doc, line),
        Input::Act(action) => match run(&action(), doc) {
            DispatchOutcome::Handled => Ok("handled".to_string()),
            other => Err(format!("{other:?}")),
        },
    }
}

/// Run console `line` through its verb, as `execute_console_line` does.
fn run_line(doc: &mut MindMapDocument, line: &str) -> Result<String, String> {
    let ParseResult::Ok { cmd, args } = parse(line) else {
        return Err(format!("{line:?} does not parse"));
    };
    let mut effects = ConsoleEffects::new(doc);
    match (cmd.execute)(&Args::new(&args), &mut effects) {
        ExecResult::Ok(message) => Ok(message),
        ExecResult::Lines(lines) => Ok(lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n")),
        ExecResult::Err(message) => Err(message),
    }
}

/// The fixture with `edits` made, in order, each `old` occurring exactly
/// once in the text as the edits before it left it.
fn expected(edits: &[(&str, &str)]) -> String {
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

/// One case: a name, a selection, an input and the fixture text it
/// changes.
struct Case {
    name: &'static str,
    select: &'static [PathRef],
    input: Input,
    edits: &'static [(&'static str, &'static str)],
}

/// The fixture's spellings of its paths.
const C2_TEXT: &str = r#""connector": {"content": "─", "cap_end": {"spans": [{"text": ">"}]}, "label": {"t": 0.5, "spans": [{"text": "child"}]}}"#;
const L13_TEXT: &str = r#"{"to": 3, "kind": "line"}"#;
const L14_TEXT: &str = r#"{"to": 4, "kind": "line", "content": "─", "label": {"spans": [{"text": "see"}]}}"#;
const P14_TEXT: &str = r#"{"to": 4, "kind": "portal"}"#;
const L32_TEXT: &str =
    r##"{"to": 2, "kind": "line", "color": "#00ff00", "straight": true, "visible": false}"##;
const L41_TEXT: &str = r#"{"to": 1, "kind": "line", "fit": "repeat", "spacing": 2.0, "size": 9.0, "anchors": ["top", "left"], "control_points": [[0.0, -30.0]], "zoom": {"min": 0.5}, "spans": [{"text": "~"}]}"#;
/// Node 4's link once reset: every field it inherits gone, the zoom
/// window and the span kept.
const L41_RESET: &str = r#"{"to": 1, "kind": "line", "zoom": {"min": 0.5}, "spans": [{"text": "~"}]}"#;

/// Every console form and every Action of the family.
const CASES: &[Case] = &[
    // ── path: fields ──
    Case {
        name: "path content= on a connector",
        select: &[C2],
        input: Input::Line("path content=·"),
        edits: &[(r#"{"content": "─","#, r#"{"content": "·","#)],
    },
    Case {
        name: "path content=none inherits",
        select: &[C2],
        input: Input::Line("path content=none"),
        edits: &[(r#"{"content": "─", "#, "{")],
    },
    Case {
        name: "path fields on a connector",
        select: &[C2],
        input: Input::Line(
            "path fit=repeat spacing=4 size=12 color=#ff0000 anchors=top,bottom visible=false",
        ),
        edits: &[(
            r#"{"content": "─", "#,
            r##"{"fit": "repeat", "spacing": 4.0, "size": 12.0, "color": "#ff0000", "content": "─", "anchors": ["top", "bottom"], "visible": false, "##,
        )],
    },
    Case {
        name: "path fit and color on a line link",
        select: &[L13],
        input: Input::Line("path fit=once:· color=palette(text)"),
        edits: &[(
            L13_TEXT,
            r#"{"to": 3, "kind": "line", "fit": {"once": {"filler": "·"}}, "color": "palette(text)"}"#,
        )],
    },
    Case {
        name: "path passes over a portal link beside a line link",
        select: &[P14, L13],
        input: Input::Line("path size=9"),
        edits: &[(L13_TEXT, r#"{"to": 3, "kind": "line", "size": 9.0}"#)],
    },
    Case {
        name: "SetPathContent on three links is one entry",
        select: &[L13, L32, L41],
        input: Input::Act(|| Action::SetPathContent("·".into())),
        edits: &[
            (L13_TEXT, r#"{"to": 3, "kind": "line", "content": "·"}"#),
            (
                r##""color": "#00ff00", "##,
                r##""color": "#00ff00", "content": "·", "##,
            ),
            (
                r#""size": 9.0, "anchors""#,
                r#""size": 9.0, "content": "·", "anchors""#,
            ),
        ],
    },
    // `none` is the keyword spelled exactly; any other capitalization is
    // text the path draws.
    Case {
        name: "path content=None is text",
        select: &[C2],
        input: Input::Line("path content=None"),
        edits: &[(r#"{"content": "─","#, r#"{"content": "None","#)],
    },
    Case {
        name: "SetPathContent none",
        select: &[C2],
        input: Input::Act(|| Action::SetPathContent("none".into())),
        edits: &[(r#"{"content": "─", "#, "{")],
    },
    Case {
        name: "SetPathAnchor",
        select: &[L13],
        input: Input::Act(|| Action::SetPathAnchor {
            from: "left".into(),
            to: "auto".into(),
        }),
        edits: &[(
            L13_TEXT,
            r#"{"to": 3, "kind": "line", "anchors": ["left", "auto"]}"#,
        )],
    },
    Case {
        name: "SetPathSpacing",
        select: &[C2],
        input: Input::Act(|| Action::SetPathSpacing("3".into())),
        edits: &[(r#"{"content": "─", "#, r#"{"spacing": 3.0, "content": "─", "#)],
    },
    // ── path: geometry ──
    Case {
        name: "path straight drops the control points",
        select: &[L41],
        input: Input::Line("path straight"),
        edits: &[(r#""control_points": [[0.0, -30.0]], "#, r#""straight": true, "#)],
    },
    Case {
        name: "path curve makes a straight path the default curve",
        select: &[L32],
        input: Input::Line("path curve"),
        edits: &[(r#", "straight": true"#, "")],
    },
    Case {
        name: "path curve clears the control points",
        select: &[L41],
        input: Input::Line("path curve"),
        edits: &[(r#""control_points": [[0.0, -30.0]], "#, "")],
    },
    Case {
        name: "path reset keeps visible and the zoom",
        select: &[L32, L41],
        input: Input::Line("path reset"),
        edits: &[
            (L32_TEXT, r#"{"to": 2, "kind": "line", "visible": false}"#),
            (L41_TEXT, L41_RESET),
        ],
    },
    Case {
        name: "ResetPath on a connector and two links is one entry",
        select: &[C2, L32, L41],
        input: Input::Act(|| Action::ResetPath),
        edits: &[
            (r#"{"content": "─", "#, "{"),
            (L32_TEXT, r#"{"to": 2, "kind": "line", "visible": false}"#),
            (L41_TEXT, L41_RESET),
        ],
    },
    // ── path cap ──
    Case {
        name: "path cap inserts one and removes the other",
        select: &[C2],
        input: Input::Line("path cap start=< end=none"),
        edits: &[(
            r#""cap_end": {"spans": [{"text": ">"}]}"#,
            r#""cap_start": {"spans": [{"text": "<"}]}"#,
        )],
    },
    Case {
        name: "SetPathCap replaces the end and leaves the start",
        select: &[C2],
        input: Input::Act(|| Action::SetPathCap {
            from: "".into(),
            to: "»".into(),
        }),
        edits: &[(r#"{"text": ">"}"#, r#"{"text": "»"}"#)],
    },
    Case {
        name: "SetPathCap on a link",
        select: &[L13],
        input: Input::Act(|| Action::SetPathCap {
            from: "o".into(),
            to: "".into(),
        }),
        edits: &[(
            L13_TEXT,
            r#"{"to": 3, "kind": "line", "cap_start": {"spans": [{"text": "o"}]}}"#,
        )],
    },
    // A connector that has caps beside a link that has none: one request
    // replaces the one and inserts the other (`give`'s two halves).
    Case {
        name: "path cap on a capped connector and a bare link",
        select: &[C2, L13],
        input: Input::Line("path cap start=x end=y"),
        edits: &[
            (
                r#""cap_end": {"spans": [{"text": ">"}]}"#,
                r#""cap_start": {"spans": [{"text": "x"}]}, "cap_end": {"spans": [{"text": "y"}]}"#,
            ),
            (
                L13_TEXT,
                r#"{"to": 3, "kind": "line", "cap_start": {"spans": [{"text": "x"}]}, "cap_end": {"spans": [{"text": "y"}]}}"#,
            ),
        ],
    },
    Case {
        name: "path cap end=none removes the one there is",
        select: &[C2, L13],
        input: Input::Line("path cap end=none"),
        edits: &[(r#", "cap_end": {"spans": [{"text": ">"}]}"#, "")],
    },
    // ── link ──
    Case {
        name: "link kind=portal",
        select: &[L13],
        input: Input::Line("link kind=portal"),
        edits: &[(L13_TEXT, r#"{"to": 3, "kind": "portal"}"#)],
    },
    Case {
        name: "link kind=line passes over a connector beside the link",
        select: &[C2, P14],
        input: Input::Line("link kind=line"),
        edits: &[(P14_TEXT, r#"{"to": 4, "kind": "line"}"#)],
    },
    Case {
        name: "SetLinkKind portal drops a line link's label",
        select: &[L14],
        input: Input::Act(|| Action::SetLinkKind("portal".into())),
        edits: &[(L14_TEXT, r#"{"to": 4, "kind": "portal"}"#)],
    },
    // ── label ──
    Case {
        name: "label text= replaces a label's text",
        select: &[C2],
        input: Input::Line("label text=kin"),
        edits: &[(r#"{"text": "child"}"#, r#"{"text": "kin"}"#)],
    },
    Case {
        name: "label text= gives a path a label",
        select: &[L13],
        input: Input::Line("label text=\"a b\" t=end offset=6"),
        edits: &[(
            L13_TEXT,
            r#"{"to": 3, "kind": "line", "label": {"t": 1.0, "offset": 6.0, "spans": [{"text": "a b"}]}}"#,
        )],
    },
    Case {
        name: "label t= offset=",
        select: &[C2],
        input: Input::Line("label t=0.25 offset=-4"),
        edits: &[(r#"{"t": 0.5, "#, r#"{"t": 0.25, "offset": -4.0, "#)],
    },
    Case {
        name: "label delete",
        select: &[L14],
        input: Input::Line("label delete"),
        edits: &[(r#", "label": {"spans": [{"text": "see"}]}"#, "")],
    },
    Case {
        name: "SetLabelText",
        select: &[L13],
        input: Input::Act(|| Action::SetLabelText("x".into())),
        edits: &[(
            L13_TEXT,
            r#"{"to": 3, "kind": "line", "label": {"spans": [{"text": "x"}]}}"#,
        )],
    },
    Case {
        name: "SetLabelText empty removes the label",
        select: &[C2],
        input: Input::Act(|| Action::SetLabelText("".into())),
        edits: &[(r#", "label": {"t": 0.5, "spans": [{"text": "child"}]}"#, "")],
    },
    Case {
        name: "SetLabelPosition names a place and an offset",
        select: &[C2],
        input: Input::Act(|| Action::SetLabelPosition {
            t: "start".into(),
            offset: "3".into(),
        }),
        edits: &[(r#"{"t": 0.5, "#, r#"{"t": 0.0, "offset": 3.0, "#)],
    },
    Case {
        name: "SetLabelPosition resets t and leaves the offset",
        select: &[C2],
        input: Input::Act(|| Action::SetLabelPosition {
            t: "reset".into(),
            offset: "".into(),
        }),
        edits: &[(r#"{"t": 0.5, "#, "{")],
    },
    // A connector with a label beside a link with none: one request
    // replaces the one, sets its place, and inserts the other with it.
    Case {
        name: "label text= on a labeled connector and a bare link",
        select: &[C2, L13],
        input: Input::Line("label text=hey t=0.2"),
        edits: &[
            (
                r#""label": {"t": 0.5, "spans": [{"text": "child"}]}"#,
                r#""label": {"t": 0.2, "spans": [{"text": "hey"}]}"#,
            ),
            (
                L13_TEXT,
                r#"{"to": 3, "kind": "line", "label": {"t": 0.2, "spans": [{"text": "hey"}]}}"#,
            ),
        ],
    },
    Case {
        name: "label delete removes the one there is",
        select: &[C2, L13],
        input: Input::Line("label delete"),
        edits: &[(r#", "label": {"t": 0.5, "spans": [{"text": "child"}]}"#, "")],
    },
    // ── portal ──
    Case {
        name: "portal end=0",
        select: &[P14],
        input: Input::Line("portal end=0 border_t=0.25 offset=8 content=◆ size=20 color=#0000ff"),
        edits: &[(
            P14_TEXT,
            r##"{"to": 4, "kind": "portal", "ends": [{"border_t": 0.25, "offset": 8.0, "color": "#0000ff", "size": 20.0, "content": "◆"}, {}]}"##,
        )],
    },
    Case {
        name: "portal end=1",
        select: &[P14],
        input: Input::Line("portal end=1 content=none size=11"),
        edits: &[(
            P14_TEXT,
            r#"{"to": 4, "kind": "portal", "ends": [{}, {"size": 11.0}]}"#,
        )],
    },
];

/// **Every Action and every console form of the path family makes the
/// document its case expects, as exactly one history entry, and one undo
/// gives back the fixture's bytes.** Precondition: the fixture saves
/// back to its own bytes, so a difference is the edit's.
#[test]
fn test_every_path_edit_makes_its_expected_document_in_one_entry() {
    assert_eq!(
        bytes(&fixture()),
        FIXTURE,
        "the fixture is in the saver's spelling"
    );
    for text in [C2_TEXT, L13_TEXT, L14_TEXT, P14_TEXT, L32_TEXT, L41_TEXT] {
        assert_eq!(
            FIXTURE.matches(text).count(),
            1,
            "precondition: the fixture spells {text}"
        );
    }
    for case in CASES {
        let mut doc = fixture();
        select(&mut doc, case.select);
        let entries = doc.doc().history().count();
        let ran = run_input(&mut doc, case.input);
        assert!(ran.is_ok(), "{}: ran, not {ran:?}", case.name);
        assert_eq!(
            bytes(&doc),
            expected(case.edits),
            "{}: the saved document",
            case.name
        );
        assert_eq!(
            doc.doc().history().count(),
            entries + 1,
            "{}: exactly one entry",
            case.name
        );
        doc.undo();
        assert_eq!(bytes(&doc), FIXTURE, "{}: undo gives back the fixture", case.name);
    }
}

/// **`link kind=portal` then `link kind=line` gives a bare line link
/// back exactly**, its path fields kept while it was a portal (they are
/// never saved for one); and a line link that held a label, made a
/// portal, has lost it — which the undo of that switch gives back
/// (`compose::set_link_kind`). Control: the portal's bytes do differ.
#[test]
fn test_a_link_made_a_portal_and_a_line_again_round_trips() {
    let mut doc = fixture();
    select(&mut doc, &[L13]);
    run_line(&mut doc, "path size=7").expect("a field to keep");
    let sized = bytes(&doc);
    run_line(&mut doc, "link kind=portal").expect("to a portal");
    assert_ne!(bytes(&doc), sized, "control: the portal saves otherwise");
    assert!(
        bytes(&doc).contains(r#"{"to": 3, "kind": "portal"}"#),
        "a portal saves no path keys"
    );
    run_line(&mut doc, "link kind=line").expect("back to a line");
    assert_eq!(bytes(&doc), sized, "the round trip is exact");

    let mut doc = fixture();
    select(&mut doc, &[L14]);
    run_line(&mut doc, "link kind=portal").expect("to a portal");
    run_line(&mut doc, "link kind=line").expect("back to a line");
    assert!(
        bytes(&doc).contains(r#"{"to": 4, "kind": "line", "content": "─"}"#),
        "the label is gone"
    );
    doc.undo();
    doc.undo();
    assert_eq!(bytes(&doc), FIXTURE, "the undo gives the label back");
}

/// **`SetLinkKind` on a connector is refused, with the reason on the
/// status line, and changes nothing.** Control: the same Action on a
/// link is accepted.
#[test]
fn test_set_link_kind_on_a_connector_is_refused() {
    let mut doc = fixture();
    select(&mut doc, &[C2]);
    let revision = doc.revision();
    assert_eq!(
        run(&Action::SetLinkKind("portal".into()), &mut doc),
        DispatchOutcome::Refused(LINK_KIND_ON_A_CONNECTOR)
    );
    assert_eq!(
        doc.status(),
        Some(LINK_KIND_ON_A_CONNECTOR),
        "the status line says why"
    );
    assert_eq!(doc.revision(), revision, "nothing changed");
    assert_eq!(bytes(&doc), FIXTURE);

    select(&mut doc, &[L13]);
    assert_eq!(
        run(&Action::SetLinkKind("portal".into()), &mut doc),
        DispatchOutcome::Handled,
        "control: a link takes it"
    );
}

/// **A label on a portal link is refused, naming the span editors
/// (V1-C8, #249).** Control: on a line link the same Action applies.
#[test]
fn test_a_label_on_a_portal_link_is_refused() {
    let mut doc = fixture();
    select(&mut doc, &[P14]);
    assert_eq!(
        run(&Action::SetLabelText("x".into()), &mut doc),
        DispatchOutcome::Refused(LABEL_ON_A_PORTAL)
    );
    assert!(LABEL_ON_A_PORTAL.contains("V1-C8 (#249)"));
    assert_eq!(
        run_line(&mut doc, "label delete"),
        Err(LABEL_ON_A_PORTAL.to_string()),
        "the console says the same"
    );
    assert_eq!(bytes(&doc), FIXTURE);
    select(&mut doc, &[L13]);
    assert_eq!(
        run(&Action::SetLabelText("x".into()), &mut doc),
        DispatchOutcome::Handled,
        "control: a line link takes it"
    );
}

/// **Each verb refuses with a clear message when nothing it edits is
/// selected**, and changes nothing: a node alone, or — for `path` — a
/// portal link alone, which draws no path. Control: the same lines on
/// what they edit apply (the cases above).
#[test]
fn test_each_verb_refuses_naming_what_to_select() {
    let refusals: &[(&[PathRef], &str, &str)] = &[
        (&[], "path content=x", "path: select a connector or a line link"),
        (&[P14], "path content=x", "path: a portal link draws no path"),
        (&[], "path straight", "path: select a connector or a line link"),
        (
            &[],
            "path cap start=x",
            "path cap: select a connector or a line link",
        ),
        (&[C2], "path cap start=\"\"", "path cap: give a cap's text"),
        (&[], "link kind=portal", "link: select a link"),
        (&[C2], "link kind=portal", LINK_KIND_ON_A_CONNECTOR),
        (&[L13], "link kind=cross_link", "'cross_link' is not a link kind"),
        (&[], "label text=x", "label: select a connector or a line link"),
        (&[L13], "label t=0.5", "label: no selected path has a label"),
        (&[], "portal end=0 size=3", "portal: select a portal link"),
        (&[L13], "portal end=0 size=3", "portal: select a portal link"),
        (&[P14], "portal size=3", "portal: give end=0"),
        // Empty text is refused on all four text keys, each in its own
        // words.
        (&[C2], "path content=\"\"", "path: give the text it draws, or `none` to inherit"),
        (&[P14], "portal end=0 content=\"\"", "portal: give the marker's text, or `none` to inherit"),
        (&[C2], "path cap start=\"\"", "path cap: give a cap's text, or `none` to remove it"),
        (&[C2], "label text=\"\"", "label: give the label's text, or `label delete` to remove it"),
    ];
    for (paths, line, message) in refusals {
        let mut doc = fixture();
        select(&mut doc, paths);
        let revision = doc.revision();
        match run_line(&mut doc, line) {
            Err(got) => assert!(got.contains(message), "{line}: {got:?} names {message:?}"),
            Ok(got) => panic!("{line} on {paths:?} must be refused, ran: {got}"),
        }
        assert_eq!(doc.revision(), revision, "{line}: nothing changed");
    }
    let mut doc = fixture();
    select(&mut doc, &[]);
    for action in [
        Action::SetPathContent("x".into()),
        Action::ResetPath,
        Action::SetLabelText("x".into()),
        Action::SetPathAnchor {
            from: "top".into(),
            to: "auto".into(),
        },
    ] {
        assert_eq!(
            run(&action, &mut doc),
            DispatchOutcome::Unhandled,
            "{action:?} with nothing selected"
        );
    }
    assert_eq!(bytes(&doc), FIXTURE);
}

/// **An empty content is refused from an Action too, and `none` is the
/// keyword only as spelled**: `SetPathContent("")` applies nothing, and
/// `node palette=None` names a palette, which this map does not have,
/// rather than unbinding one. Control: the lowercase `none` is accepted
/// as the keyword (node 1 has nothing to unbind).
#[test]
fn test_empty_content_is_refused_and_none_is_matched_exactly() {
    let mut doc = fixture();
    select(&mut doc, &[C2]);
    assert_eq!(
        run(&Action::SetPathContent(String::new()), &mut doc),
        DispatchOutcome::Unhandled,
        "an empty content applies nothing"
    );
    assert_eq!(bytes(&doc), FIXTURE);

    let n1 = node(&doc, 1);
    doc.selection.select_only(n1);
    assert_eq!(
        run_line(&mut doc, "node palette=None"),
        Err("'None' is not a palette of this map".to_string()),
        "`None` is a palette's name, not the keyword"
    );
    assert!(
        run_line(&mut doc, "node palette=none").is_ok(),
        "control: the keyword is read as the keyword"
    );
}

/// **`path content="·"` changes what is drawn on the next projection,
/// and only there**: the update's `LayerChanges` touch node 2's
/// connector in the connections layer, and the run they hold now reads
/// `·` where it read `─`. Control: before the edit the connector's run
/// reads `─`, so the change is the edit's; and no other path is written.
#[test]
fn test_path_content_changes_the_drawn_run_on_the_next_projection() {
    let mut doc = fixture();
    let mut p = Projected::new(&mut doc);
    let connector = path(&doc, C2);
    let drawn = |p: &Projected| -> String {
        let tree = p.layers.tree(Layer::Connections);
        tree.root()
            .descendants(tree.arena())
            .filter(|&glyph| p.projector.element_of(Layer::Connections, glyph) == Some(connector))
            .filter_map(|glyph| tree.arena()[glyph].get().glyph_run())
            .filter_map(|run| match run.shaped() {
                baumhard::gfx_structs::run::RunShape::Path { run, .. } => Some(
                    run.buffer
                        .lines
                        .iter()
                        .map(|line| line.text())
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect()
    };
    assert!(drawn(&p).contains('─'), "control: drawn `─` before the edit");

    select(&mut doc, &[C2]);
    run_line(&mut doc, "path content=·").expect("applies");
    let written = p.update(&mut doc);
    let connections: Vec<_> = written
        .iter()
        .filter(|(layer, _, _)| *layer == Layer::Connections)
        .collect();
    assert!(
        connections.iter().all(|(_, element, _)| *element == connector),
        "only the connector is written: {connections:?}"
    );
    let texts: Vec<&str> = connections
        .iter()
        .filter_map(|(_, _, text)| text.as_deref())
        .collect();
    assert!(
        texts.iter().any(|text| text.contains('·') && !text.contains('─')),
        "the update writes the connector's run as `·`: {texts:?}"
    );

    // Control: an edit to another path writes that path, and not the
    // connector, so the comparison above can see an element beside it.
    // (Node 1's link to node 3 draws the built-in `·` already, so it is
    // given `+`.)
    let link = path(&doc, L13);
    select(&mut doc, &[L13]);
    run_line(&mut doc, "path content=+").expect("applies");
    let written: Vec<_> = p
        .update(&mut doc)
        .into_iter()
        .filter(|(layer, _, _)| *layer == Layer::Connections)
        .map(|(_, element, text)| (element, text))
        .collect();
    assert!(
        written
            .iter()
            .any(|(element, text)| *element == link && text.as_deref() == Some("+")),
        "control: the link is written as `+`: {written:?}"
    );
    assert!(
        written.iter().all(|(element, _)| *element != connector),
        "control: the connector is not written again: {written:?}"
    );
}
