// SPDX-License-Identifier: MPL-2.0

//! Field editing I, through both of its surfaces, on one fixture
//! (`../fixtures/field.mandala.json`): every Action through the
//! family's body ([`super::run`], what `dispatch` runs before it syncs
//! the view) and every console form through its verb's `execute` (what
//! `execute_console_line` and IPC `act.console` run before they sync).
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

use super::run;
use crate::application::app::dispatch::cross_dispatch::DispatchOutcome;
use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;
use crate::application::keybinds::{Action, ColorAxis, FontSlot, FramePreviewTarget, ZoomBound};
use crate::application::view::inspect::children_of_kind;

/// The fixture every case starts from.
const FIXTURE: &str = include_str!("../fixtures/field.mandala.json");

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

/// The first child of `kind` of node `ident`, or its `nth` section.
fn child(doc: &MindMapDocument, ident: u32, kind: DocKind, nth: usize) -> NodeId {
    children_of_kind(doc.doc(), node(doc, ident), kind)
        .nth(nth)
        .expect("the child")
}

/// What a case selects.
#[derive(Clone, Copy)]
enum Select {
    Node(u32),
    Nodes(&'static [u32]),
    Section(u32, usize),
    /// Grapheme clusters `start..end` of a node's first section.
    Range(u32, usize, usize),
    Connector(u32),
    Link(u32),
}

fn select(doc: &mut MindMapDocument, what: Select) {
    match what {
        Select::Node(id) => {
            let id = node(doc, id);
            doc.selection.select_only(id);
        }
        Select::Nodes(ids) => {
            let ids: Vec<NodeId> = ids.iter().map(|&id| node(doc, id)).collect();
            doc.selection.set(ids);
        }
        Select::Section(id, nth) => {
            let section = child(doc, id, DocKind::Section, nth);
            doc.selection.select_only(section);
        }
        Select::Range(id, start, end) => {
            let section = child(doc, id, DocKind::Section, 0);
            doc.selection.select_range(section, start, end);
        }
        Select::Connector(id) => {
            let connector = child(doc, id, DocKind::Connector, 0);
            doc.selection.select_only(connector);
        }
        Select::Link(id) => {
            let link = child(doc, id, DocKind::Link, 0);
            doc.selection.select_only(link);
        }
    }
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
/// Action's outcome; `Err` with a console line's refusal.
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
pub(super) fn run_line(doc: &mut MindMapDocument, line: &str) -> Result<String, String> {
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

/// One case: a name, a selection, an input and the fixture lines it
/// changes.
struct Case {
    name: &'static str,
    select: Select,
    input: Input,
    edits: &'static [(&'static str, &'static str)],
}

const N1: Select = Select::Node(1);
const N2: Select = Select::Node(2);
const N3: Select = Select::Node(3);

/// Node 1's lines, as the fixture spells them.
const N1_COLOR: &str = r#"      "color": "var(fg)",
"#;
const N1_FRAME: &str = r#""frame": {"fit": "repeat", "rings": [{"spans": [{"text": "─"}]}]}"#;
const N3_SECTIONS: &str = r#""sections": [{"spans": [{"text": "plain"}]}],"#;
const N3_ZOOM: &str = r#"      "zoom": {"min": 0.25},
"#;

/// Every console form and every Action of the family.
const CASES: &[Case] = &[
    // ── node ──
    Case {
        name: "node resize",
        select: N1,
        input: Input::Line("node resize 120 80"),
        edits: &[(N1_COLOR, "      \"min_size\": [120.0, 80.0],\n      \"color\": \"var(fg)\",\n")],
    },
    Case {
        name: "node fit",
        select: N2,
        input: Input::Line("node fit"),
        edits: &[("      \"min_size\": [200.0, 60.0],\n", "")],
    },
    Case {
        name: "node fields on two nodes",
        select: Select::Nodes(&[1, 3]),
        input: Input::Line("node shape=ellipse palette=warm folded=true hidden=true opacity=0.5"),
        edits: &[
            (
                N1_COLOR,
                "      \"shape\": \"ellipse\",\n      \"palette\": \"warm\",\n      \"folded\": true,\n      \"hidden\": true,\n      \"opacity\": 0.5,\n      \"color\": \"var(fg)\",\n",
            ),
            (
                N3_ZOOM,
                "      \"shape\": \"ellipse\",\n      \"palette\": \"warm\",\n      \"zoom\": {\"min\": 0.25},\n      \"folded\": true,\n      \"hidden\": true,\n      \"opacity\": 0.5,\n",
            ),
        ],
    },
    Case {
        name: "node radius",
        select: N1,
        input: Input::Line("node radius=25"),
        edits: &[(N1_COLOR, "      \"shape\": {\"rectangle\": {\"radius\": 25}},\n      \"color\": \"var(fg)\",\n")],
    },
    // ── section ──
    Case {
        name: "section move by a delta",
        select: Select::Section(1, 1),
        input: Input::Line("section move dx=5 dy=-5"),
        edits: &[("\"offset\": [0.0, 40.0]", "\"offset\": [5.0, 35.0]")],
    },
    Case {
        name: "section move to an offset",
        select: Select::Section(1, 1),
        input: Input::Line("section move x=1 y=2"),
        edits: &[("\"offset\": [0.0, 40.0]", "\"offset\": [1.0, 2.0]")],
    },
    Case {
        name: "section resize by index",
        select: N1,
        input: Input::Line("section resize w=100 h=30 section=0"),
        edits: &[("[{\"spans\": [{\"text\": \"Title\"}]}", "[{\"min_size\": [100.0, 30.0], \"spans\": [{\"text\": \"Title\"}]}")],
    },
    Case {
        name: "section resize fill",
        select: N2,
        input: Input::Line("section resize fill"),
        edits: &[("{\"min_size\": [180.0, 40.0], \"spans\"", "{\"spans\"")],
    },
    Case {
        name: "section align",
        select: N2,
        input: Input::Line("section align=center"),
        edits: &[("[180.0, 40.0], \"spans\"", "[180.0, 40.0], \"align\": \"center\", \"spans\"")],
    },
    Case {
        name: "section text",
        select: N3,
        input: Input::Line("section text \"two words\""),
        edits: &[("{\"text\": \"plain\"}", "{\"text\": \"two words\"}")],
    },
    Case {
        name: "section add first",
        select: N3,
        input: Input::Line("section add at=0 text=first"),
        edits: &[(N3_SECTIONS, "\"sections\": [{\"spans\": [{\"text\": \"first\"}]}, {\"spans\": [{\"text\": \"plain\"}]}],")],
    },
    Case {
        name: "section add last",
        select: N3,
        input: Input::Line("section add text=second"),
        edits: &[(N3_SECTIONS, "\"sections\": [{\"spans\": [{\"text\": \"plain\"}]}, {\"spans\": [{\"text\": \"second\"}]}],")],
    },
    Case {
        name: "section delete",
        select: Select::Section(1, 1),
        input: Input::Line("section delete"),
        edits: &[(", {\"channel\": 1, \"offset\": [0.0, 40.0], \"spans\": [{\"text\": \"body\"}]}", "")],
    },
    Case {
        name: "section split",
        select: N3,
        input: Input::Line("section split at=2"),
        edits: &[(N3_SECTIONS, "\"sections\": [{\"spans\": [{\"text\": \"pl\"}]}, {\"spans\": [{\"text\": \"ain\"}]}],")],
    },
    // ── frame ──
    Case {
        name: "frame fields",
        select: N1,
        input: Input::Line("frame start=top direction=ccw fit=once:· gap=2 inset=4"),
        edits: &[(
            N1_FRAME,
            "\"frame\": {\"start\": \"top\", \"direction\": \"counterclockwise\", \"fit\": {\"once\": {\"filler\": \"·\"}}, \"gap\": 2.0, \"inset\": 4.0, \"rings\": [{\"spans\": [{\"text\": \"─\"}]}]}",
        )],
    },
    Case {
        name: "frame start at an edge",
        select: N1,
        input: Input::Line("frame start=left:0.25 fit=fit:0.5,1.5"),
        edits: &[(
            N1_FRAME,
            "\"frame\": {\"start\": {\"edge\": \"left\", \"at\": 0.25}, \"fit\": {\"fit\": {\"min\": 0.5, \"max\": 1.5}}, \"rings\": [{\"spans\": [{\"text\": \"─\"}]}]}",
        )],
    },
    Case {
        name: "frame off",
        select: N1,
        input: Input::Line("frame off"),
        edits: &[("\"frame\": {\"fit\"", "\"frame\": {\"visible\": false, \"fit\"")],
    },
    Case {
        name: "frame on gives a frame",
        select: N3,
        input: Input::Line("frame on"),
        edits: &[(N3_SECTIONS, "\"sections\": [{\"spans\": [{\"text\": \"plain\"}]}],\n      \"frame\": {\"fit\": \"repeat\", \"rings\": [{\"spans\": [{\"text\": \"─\"}]}]},")],
    },
    Case {
        name: "frame ring add",
        select: N1,
        input: Input::Line("frame ring add text=two size=0.1"),
        edits: &[("{\"text\": \"─\"}]}]}", "{\"text\": \"─\"}]}, {\"size\": 0.1, \"spans\": [{\"text\": \"two\"}]}]}")],
    },
    Case {
        name: "frame ring add to a node with none",
        select: N3,
        input: Input::Line("frame ring add text=poem"),
        edits: &[(N3_SECTIONS, "\"sections\": [{\"spans\": [{\"text\": \"plain\"}]}],\n      \"frame\": {\"rings\": [{\"spans\": [{\"text\": \"poem\"}]}]},")],
    },
    Case {
        name: "frame ring set",
        select: N1,
        input: Input::Line("frame ring 0 text=═ size=0.2 fit=spread"),
        edits: &[("\"rings\": [{\"spans\": [{\"text\": \"─\"}]}]", "\"rings\": [{\"fit\": \"spread\", \"size\": 0.2, \"spans\": [{\"text\": \"═\"}]}]")],
    },
    Case {
        name: "frame ring delete",
        select: N1,
        input: Input::Line("frame ring 0 delete"),
        edits: &[(N1_FRAME, "\"frame\": {\"fit\": \"repeat\"}")],
    },
    // ── color, font, zoom ──
    Case {
        name: "color on two nodes",
        select: Select::Nodes(&[1, 3]),
        input: Input::Line("color text=#ff0000"),
        edits: &[
            ("\"color\": \"var(fg)\"", "\"color\": \"#ff0000\""),
            (N3_ZOOM, "      \"zoom\": {\"min\": 0.25},\n      \"color\": \"#ff0000\",\n"),
        ],
    },
    Case {
        name: "color of a frame and its rings",
        select: N1,
        input: Input::Line("color border=#00ff00 ring=var(accent)"),
        edits: &[(
            N1_FRAME,
            "\"frame\": {\"fit\": \"repeat\", \"color\": \"#00ff00\", \"rings\": [{\"color\": \"var(accent)\", \"spans\": [{\"text\": \"─\"}]}]}",
        )],
    },
    Case {
        name: "color reset on a connector",
        select: Select::Connector(2),
        input: Input::Line("color text=reset"),
        edits: &[(",\n      \"connector\": {\"color\": \"var(accent)\"}", "")],
    },
    Case {
        name: "color of a link",
        select: Select::Link(3),
        input: Input::Line("color border=palette(text)"),
        edits: &[("{\"to\": 2, \"kind\": \"line\"}", "{\"to\": 2, \"kind\": \"line\", \"color\": \"palette(text)\"}")],
    },
    Case {
        name: "color of a range across clusters",
        select: Select::Range(2, 1, 3),
        input: Input::Line("color text=#ff0000"),
        edits: &[(
            "\"spans\": [{\"text\": \"a👨‍👩‍👧é̂b\"}]",
            "\"spans\": [{\"text\": \"a\"}, {\"text\": \"👨‍👩‍👧é̂\", \"color\": \"#ff0000\"}, {\"text\": \"b\"}]",
        )],
    },
    Case {
        name: "font on a node",
        select: N1,
        input: Input::Line("font size=20 weight=bold italic=true"),
        edits: &[(N1_COLOR, "      \"size\": 20.0,\n      \"color\": \"var(fg)\",\n      \"weight\": \"bold\",\n      \"italic\": true,\n")],
    },
    Case {
        name: "font family",
        select: N3,
        input: Input::Line("font family=Novem"),
        edits: &[(N3_ZOOM, "      \"zoom\": {\"min\": 0.25},\n      \"font\": \"Novem\",\n")],
    },
    Case {
        name: "font size on a connector",
        select: Select::Connector(2),
        input: Input::Line("font size=9"),
        edits: &[("\"connector\": {\"color\"", "\"connector\": {\"size\": 9.0, \"color\"")],
    },
    Case {
        name: "zoom both bounds",
        select: N2,
        input: Input::Line("zoom min=0.5 max=2"),
        edits: &[("      \"min_size\": [200.0, 60.0],\n", "      \"min_size\": [200.0, 60.0],\n      \"zoom\": {\"min\": 0.5, \"max\": 2.0},\n")],
    },
    Case {
        name: "zoom keeps the other bound",
        select: N3,
        input: Input::Line("zoom max=4"),
        edits: &[(N3_ZOOM, "      \"zoom\": {\"min\": 0.25, \"max\": 4.0},\n")],
    },
    Case {
        name: "zoom clear",
        select: N3,
        input: Input::Line("zoom clear"),
        edits: &[(N3_ZOOM, "")],
    },
    // ── range, theme, defaults ──
    Case {
        name: "range across clusters",
        select: N2,
        input: Input::Line("range 1 3 weight=bold italic=true"),
        edits: &[(
            "\"spans\": [{\"text\": \"a👨‍👩‍👧é̂b\"}]",
            "\"spans\": [{\"text\": \"a\"}, {\"text\": \"👨‍👩‍👧é̂\", \"weight\": \"bold\", \"italic\": true}, {\"text\": \"b\"}]",
        )],
    },
    Case {
        name: "theme variant",
        select: N1,
        input: Input::Line("theme variant light"),
        edits: &[("\"active\": \"dark\"", "\"active\": \"light\"")],
    },
    Case {
        name: "theme set in the active variant",
        select: N1,
        input: Input::Line("theme set accent=#000000"),
        edits: &[("\"accent\": \"#30b082\"", "\"accent\": \"#000000\"")],
    },
    Case {
        name: "theme set in a named variant",
        select: N1,
        input: Input::Line("theme set fg=#ffffff variant=light"),
        edits: &[("\"fg\": \"#1b1b1b\"", "\"fg\": \"#ffffff\"")],
    },
    Case {
        name: "defaults",
        select: N1,
        input: Input::Line("defaults frame.fit=repeat ring.size=0.2 span.weight=bold section.size=reset"),
        edits: &[(
            "    \"section\": {\n      \"size\": 14.0\n    }\n",
            "    \"frame\": {\n      \"fit\": \"repeat\"\n    },\n    \"ring\": {\n      \"size\": 0.2\n    },\n    \"span\": {\n      \"weight\": \"bold\"\n    }\n",
        )],
    },
    // ── the Actions ──
    Case {
        name: "SetColor",
        select: N3,
        input: Input::Act(|| Action::SetColor {
            axis: ColorAxis::Text,
            value: "var(accent)".into(),
        }),
        edits: &[(N3_ZOOM, "      \"zoom\": {\"min\": 0.25},\n      \"color\": \"var(accent)\",\n")],
    },
    Case {
        name: "SetFont",
        select: N3,
        input: Input::Act(|| Action::SetFont {
            slot: FontSlot::Weight,
            value: "700".into(),
        }),
        edits: &[(N3_ZOOM, "      \"zoom\": {\"min\": 0.25},\n      \"weight\": \"bold\",\n")],
    },
    Case {
        name: "SetFontFamily",
        select: N3,
        input: Input::Act(|| Action::SetFontFamily("Galatia SIL".into())),
        edits: &[(N3_ZOOM, "      \"zoom\": {\"min\": 0.25},\n      \"font\": \"Galatia SIL\",\n")],
    },
    Case {
        name: "SetZoom",
        select: N3,
        input: Input::Act(|| Action::SetZoom {
            bound: ZoomBound::Min,
            value: "unset".into(),
        }),
        edits: &[(N3_ZOOM, "")],
    },
    Case {
        name: "ClearZoom",
        select: N3,
        input: Input::Act(|| Action::ClearZoom),
        edits: &[(N3_ZOOM, "")],
    },
    Case {
        name: "SetFrameField",
        select: N1,
        input: Input::Act(|| Action::SetFrameField {
            field: "inset".into(),
            value: "3".into(),
        }),
        edits: &[("{\"fit\": \"repeat\", \"rings\"", "{\"fit\": \"repeat\", \"inset\": 3.0, \"rings\"")],
    },
    Case {
        name: "CycleFrameContent",
        select: N1,
        input: Input::Act(|| Action::CycleFrameContent),
        edits: &[("{\"text\": \"─\"}", "{\"text\": \"━\"}")],
    },
    Case {
        name: "CycleFrameContent gives a frame",
        select: N3,
        input: Input::Act(|| Action::CycleFrameContent),
        edits: &[(N3_SECTIONS, "\"sections\": [{\"spans\": [{\"text\": \"plain\"}]}],\n      \"frame\": {\"fit\": \"repeat\", \"rings\": [{\"spans\": [{\"text\": \"─\"}]}]},")],
    },
    Case {
        name: "ToggleFrameVisible",
        select: N1,
        input: Input::Act(|| Action::ToggleFrameVisible),
        edits: &[("\"frame\": {\"fit\"", "\"frame\": {\"visible\": false, \"fit\"")],
    },
    Case {
        name: "ToggleFrameVisible gives a frame",
        select: N3,
        input: Input::Act(|| Action::ToggleFrameVisible),
        edits: &[(N3_SECTIONS, "\"sections\": [{\"spans\": [{\"text\": \"plain\"}]}],\n      \"frame\": {\"fit\": \"repeat\", \"rings\": [{\"spans\": [{\"text\": \"─\"}]}]},")],
    },
    Case {
        name: "SetSectionOffsetDelta",
        select: Select::Section(1, 1),
        input: Input::Act(|| Action::SetSectionOffsetDelta {
            dx: "-1".into(),
            dy: "2".into(),
        }),
        edits: &[("\"offset\": [0.0, 40.0]", "\"offset\": [-1.0, 42.0]")],
    },
    Case {
        name: "SetSectionOffsetAbs",
        select: Select::Section(1, 1),
        input: Input::Act(|| Action::SetSectionOffsetAbs {
            x: "7".into(),
            y: "8".into(),
        }),
        edits: &[("\"offset\": [0.0, 40.0]", "\"offset\": [7.0, 8.0]")],
    },
    Case {
        name: "SetSectionSizeAbs",
        select: N3,
        input: Input::Act(|| Action::SetSectionSizeAbs {
            w: "50".into(),
            h: "20".into(),
        }),
        edits: &[("{\"spans\": [{\"text\": \"plain\"}]}", "{\"min_size\": [50.0, 20.0], \"spans\": [{\"text\": \"plain\"}]}")],
    },
    Case {
        name: "SetSectionSizeFillParent",
        select: N2,
        input: Input::Act(|| Action::SetSectionSizeFillParent),
        edits: &[("{\"min_size\": [180.0, 40.0], \"spans\"", "{\"spans\"")],
    },
    Case {
        name: "SetSectionText",
        select: N2,
        input: Input::Act(|| Action::SetSectionText { text: "one".into() }),
        edits: &[("{\"text\": \"a👨‍👩‍👧é̂b\"}", "{\"text\": \"one\"}")],
    },
    Case {
        name: "AddSection",
        select: N3,
        input: Input::Act(|| Action::AddSection {
            at: "".into(),
            text: "".into(),
        }),
        edits: &[(N3_SECTIONS, "\"sections\": [{\"spans\": [{\"text\": \"plain\"}]}, {}],")],
    },
    Case {
        name: "DeleteSection",
        select: Select::Section(1, 0),
        input: Input::Act(|| Action::DeleteSection),
        edits: &[("[{\"spans\": [{\"text\": \"Title\"}]}, {", "[{")],
    },
    Case {
        name: "SplitSection",
        select: N3,
        input: Input::Act(|| Action::SplitSection {
            at_grapheme: "1".into(),
        }),
        edits: &[(N3_SECTIONS, "\"sections\": [{\"spans\": [{\"text\": \"p\"}]}, {\"spans\": [{\"text\": \"lain\"}]}],")],
    },
];

/// **Every Action and every console form of the field family makes the
/// document its case expects, as exactly one history entry, and one undo
/// gives back the fixture's bytes.** Precondition: the fixture saves
/// back to its own bytes, so a difference is the edit's.
#[test]
fn test_every_field_edit_makes_its_expected_document_in_one_entry() {
    assert_eq!(
        bytes(&fixture()),
        FIXTURE,
        "the fixture is in the saver's spelling"
    );
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

/// A projection of `doc`, kept up to date by [`Projected::sync`]: what
/// the app's view runs, with no renderer.
struct Projected {
    projector: baumhard::projection::Projector,
    layers: baumhard::projection::GlyphLayers,
}

impl Projected {
    fn new(doc: &mut MindMapDocument) -> Self {
        baumhard::font::fonts::init();
        let mut layers = baumhard::projection::GlyphLayers::new();
        let mut fonts = baumhard::font::fonts::FontSystemLease::new("field_nodes tests");
        let projector = baumhard::projection::Projector::project_full(doc.doc(), &mut layers, &mut fonts);
        doc.take_changes();
        Projected { projector, layers }
    }

    /// Bring the projection up to `doc`, returning the document elements
    /// whose glyphs the update wrote, in any layer.
    fn sync(&mut self, doc: &mut MindMapDocument) -> Vec<NodeId> {
        use baumhard::projection::Layer;
        let mut fonts = baumhard::font::fonts::FontSystemLease::new("field_nodes tests");
        let records = doc.take_changes();
        let changes = self
            .projector
            .update(doc.doc(), &records, &mut self.layers, &mut fonts);
        let mut touched: Vec<NodeId> = Vec::new();
        for layer in Layer::ALL {
            for (glyph, _) in changes.journal(layer).touched() {
                if let Some(element) = self.projector.element_of(layer, glyph) {
                    if !touched.contains(&element) {
                        touched.push(element);
                    }
                }
            }
        }
        changes.discard(&mut self.layers);
        touched.sort();
        touched
    }
}

/// **`node fit` clears the minimum size, and the node is then drawn at
/// the size of its text, following it** (D5). Node 2's minimum size is
/// larger than its short text: its drawn rectangle is that size; after
/// `node fit` it is smaller in both directions, and a longer text
/// widens it. Control: before the fit the same longer text does not
/// change the drawn rectangle, which the minimum size still holds.
#[test]
fn test_node_fit_draws_the_node_at_the_size_of_its_text() {
    let mut doc = fixture();
    let mut p = Projected::new(&mut doc);
    let n2 = node(&doc, 2);
    let rect = |p: &Projected| p.projector.node_rect(n2).expect("node 2 is drawn").size;
    assert_eq!(
        rect(&p),
        glam::Vec2::new(200.0, 60.0),
        "drawn at its minimum size"
    );

    doc.selection.select_only(n2);
    run_line(&mut doc, "section text abc").expect("the text applies");
    p.sync(&mut doc);
    assert_eq!(
        rect(&p),
        glam::Vec2::new(200.0, 60.0),
        "control: under the minimum size a longer text changes nothing"
    );

    run_line(&mut doc, "node fit").expect("node fit applies");
    p.sync(&mut doc);
    let fitted = rect(&p);
    assert!(
        fitted.x < 200.0 && fitted.y < 60.0,
        "fitted to its text, smaller than the minimum it had: {fitted}"
    );
    run_line(
        &mut doc,
        "section text \"a much longer line of text than before\"",
    )
    .expect("applies");
    p.sync(&mut doc);
    let grown = rect(&p);
    assert!(
        grown.x > fitted.x,
        "the drawn size follows the text: {grown} after {fitted}"
    );
    assert_eq!(grown.y, fitted.y, "one line still");
}

/// **`theme variant` re-projects exactly the elements that read a theme
/// variable.** In the fixture node 1's color is `var(fg)`, which both
/// its sections inherit, and node 2's connector is `var(accent)`; nothing
/// else reads a variable. Switching the variant writes the glyphs of
/// those three and of nothing else — node 1's ring, node 2's section,
/// node 3's section and the link are untouched. Control: an edit to node
/// 3's text writes node 3's section and the link that leaves its box, so
/// the comparison can see elements outside the variable's readers.
#[test]
fn test_theme_variant_reprojects_exactly_the_variable_readers() {
    let mut doc = fixture();
    let mut p = Projected::new(&mut doc);
    let mut readers = vec![
        child(&doc, 1, DocKind::Section, 0),
        child(&doc, 1, DocKind::Section, 1),
        child(&doc, 2, DocKind::Connector, 0),
    ];
    readers.sort();
    run_line(&mut doc, "theme variant light").expect("the switch applies");
    assert_eq!(p.sync(&mut doc), readers, "exactly the variable readers");

    let mut own = vec![
        child(&doc, 3, DocKind::Section, 0),
        child(&doc, 3, DocKind::Link, 0),
    ];
    own.sort();
    doc.selection.select_only(node(&doc, 3));
    run_line(&mut doc, "section text other").expect("applies");
    assert_eq!(p.sync(&mut doc), own, "control: another edit writes its own");
}

/// **`range` styles by grapheme cluster and leaves the spans
/// normalized.** Node 2's text is `a`, a five-code-point emoji, an `é`
/// with a combining circumflex, and `b`: four clusters. `range 1 3`
/// bolds the emoji and the marked `é` whole — three spans — and `range 0 4`
/// then bolds the rest, after which the normalization merges the four
/// equal spans into one. Control: a range counted in code points would
/// cut inside the emoji, which the first span list rules out.
#[test]
fn test_range_styles_by_grapheme_and_normalizes() {
    use baumhard::document::element::DocElement;
    let mut doc = fixture();
    doc.selection.select_only(node(&doc, 2));
    let section = child(&doc, 2, DocKind::Section, 0);
    let spans = |doc: &MindMapDocument| -> Vec<(String, bool)> {
        children_of_kind(doc.doc(), section, DocKind::Span)
            .filter_map(|id| match doc.doc().get(id) {
                Some(DocElement::Span(span)) => Some((span.text.clone(), span.style.weight.is_some())),
                _ => None,
            })
            .collect()
    };
    run_line(&mut doc, "range 1 3 weight=bold").expect("applies");
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let marked = "\u{e9}\u{302}";
    assert_eq!(
        spans(&doc),
        vec![
            ("a".to_string(), false),
            (format!("{family}{marked}"), true),
            ("b".to_string(), false),
        ]
    );
    run_line(&mut doc, "range 0 4 weight=bold").expect("applies");
    assert_eq!(
        spans(&doc),
        vec![(format!("a{family}{marked}b"), true)],
        "one span"
    );
    assert_eq!(doc.doc().history().count(), 2, "one entry each");
}

/// **A frame preview is a transaction: cancelled, it leaves the document
/// byte for byte as it was and the history as it was; committed, it is
/// one entry.** A second preview replaces the first rather than adding
/// to it, and while one is open the document refuses every other edit
/// (D28), on the console line and as the Action's reason. The Actions
/// and the console reach the same transaction.
///
/// Controls: the preview does change the document while open, so the
/// byte comparison after the cancel is one an unreverted preview fails;
/// and with the preview gone the refused edit applies.
#[test]
fn test_a_frame_preview_cancels_to_nothing_and_commits_as_one_entry() {
    use crate::application::document::{Interaction, TRANSACTION_OPEN_STATUS};
    let mut doc = fixture();
    doc.selection.select_only(node(&doc, 1));
    let entries = doc.doc().history().count();

    run_line(&mut doc, "frame preview fit=spread").expect("the preview stages");
    assert_eq!(doc.open_interaction(), Some(Interaction::FramePreview));
    assert_ne!(bytes(&doc), FIXTURE, "control: the preview shows");
    assert_eq!(
        run_line(&mut doc, "color text=#ff0000"),
        Err(TRANSACTION_OPEN_STATUS.to_string()),
        "another edit is refused while the preview is open"
    );
    assert_eq!(
        run(&Action::CycleFrameContent, &mut doc),
        DispatchOutcome::Refused(TRANSACTION_OPEN_STATUS),
        "an Action too, with the reason"
    );
    run_line(&mut doc, "frame cancel").expect("the cancel runs");
    assert_eq!(bytes(&doc), FIXTURE, "cancelled: byte for byte as before");
    assert_eq!(doc.doc().history().count(), entries, "and no entry");
    assert_eq!(doc.open_interaction(), None);
    run_line(&mut doc, "color text=#ff0000").expect("control: with no preview the edit applies");
    doc.undo();

    let set_preview = |field: &str, value: &str| Action::SetFramePreview {
        target_kind: FramePreviewTarget::Node,
        field: field.into(),
        value: value.into(),
    };
    assert_eq!(
        run(&set_preview("inset", "9"), &mut doc),
        DispatchOutcome::Handled
    );
    assert_eq!(run(&set_preview("gap", "2"), &mut doc), DispatchOutcome::Handled);
    assert_eq!(
        run(&Action::CommitFramePreview, &mut doc),
        DispatchOutcome::Handled
    );
    assert_eq!(
        bytes(&doc),
        expected(&[(
            "{\"fit\": \"repeat\", \"rings\"",
            "{\"fit\": \"repeat\", \"gap\": 2.0, \"rings\""
        )]),
        "the second preview replaced the first"
    );
    assert_eq!(doc.doc().history().count(), entries + 1, "committed: one entry");
    assert_eq!(
        run(&Action::CommitFramePreview, &mut doc),
        DispatchOutcome::Unhandled,
        "nothing left to commit"
    );
    assert_eq!(
        run(&Action::CancelFramePreview, &mut doc),
        DispatchOutcome::Unhandled
    );
    doc.undo();
    assert_eq!(bytes(&doc), FIXTURE, "and one undo takes it back");
}

/// **A frame preview never ends another interaction's transaction.**
/// With a transaction another interaction opened, `frame commit` and
/// `frame cancel` find no preview and leave it open, and a preview is
/// refused as busy. Control: the same commit ends a preview.
#[test]
fn test_a_frame_preview_leaves_another_interactions_transaction_alone() {
    use crate::application::document::Interaction;
    use baumhard::document::writer::transaction::TransactionKind;
    use baumhard::document::writer::ChangeSource;
    let mut doc = fixture();
    doc.selection.select_only(node(&doc, 1));
    assert!(doc.begin(
        ChangeSource::Input,
        TransactionKind::Interactive,
        Interaction::Test
    ));
    assert!(run_line(&mut doc, "frame commit").is_err());
    assert!(run_line(&mut doc, "frame cancel").is_err());
    assert!(run_line(&mut doc, "frame preview gap=1").is_err(), "busy");
    assert_eq!(doc.open_interaction(), Some(Interaction::Test), "still open");
    assert!(doc.cancel(Interaction::Test));
    run_line(&mut doc, "frame preview gap=1").expect("control: a preview opens");
    run_line(&mut doc, "frame commit").expect("control: and commits");
    assert_eq!(doc.open_interaction(), None);
}

/// **`font family=` takes exactly the faces a map's `font` key takes**
/// (`format::font`): a compiled family is set on the node, and a name the
/// loader refuses is refused with the reason, applying nothing. Control:
/// the accepted face is one the loader reads.
#[test]
fn test_font_family_takes_what_the_loader_takes() {
    let face = baumhard::font::fonts::loaded_families_iter()
        .find(|family| baumhard::document::format::font(family).is_some())
        .expect("a compiled face")
        .to_string();
    let mut doc = fixture();
    doc.selection.select_only(node(&doc, 3));
    let line = format!("font family=\"{face}\"");
    run_line(&mut doc, &line).expect("a compiled face is taken");
    assert!(
        bytes(&doc).contains(&format!("\"font\": \"{face}\"")),
        "{face} is saved"
    );
    let revision = doc.revision();
    assert!(run_line(&mut doc, "font family=\"Nope Sans\"").is_err());
    assert_eq!(doc.revision(), revision, "a refused face applies nothing");
    assert_eq!(
        run(&Action::SetFontFamily("Nope Sans".into()), &mut doc),
        DispatchOutcome::Unhandled
    );
}

/// **Elements that receive the same thing are one walk; elements that
/// receive different things are one walk each.** Two nodes given one
/// color are one step; two nodes whose zoom windows come out different
/// — node 3 keeps its own `min` — are two, and two whose windows come
/// out equal are one again. Control: a grouping that never merged would
/// give the color two steps.
#[test]
fn test_a_field_edit_is_one_walk_per_distinct_value() {
    use crate::application::fields::style;
    use baumhard::document::values::{ColorRef, Rgba};
    let mut doc = fixture();
    select(&mut doc, Select::Nodes(&[1, 3]));
    let red = Some(ColorRef::Rgba(Rgba::opaque(255, 0, 0)));
    let steps = style::color(&doc, ColorAxis::Text, red).expect("the color builds");
    assert_eq!(steps.len(), 1, "two nodes, one color: one walk");
    let steps = style::zoom(&doc, style::Zoom::bound(ZoomBound::Max, Some(4.0))).expect("the zoom builds");
    assert_eq!(
        steps.len(),
        2,
        "two windows: node 1's opens below, node 3's keeps 0.25"
    );
    let steps = style::zoom(&doc, style::Zoom::CLEAR).expect("the clear builds");
    assert_eq!(steps.len(), 1, "two equal windows: one walk");
}

/// **A first preview the writer refuses leaves no transaction open**:
/// `frame preview gap=-1` is refused by the gap's bound after its
/// transaction began, and the document is as it was — bytes, history,
/// nothing open — and takes the next edit. A refused later preview, from
/// the console or a keybind's Action, keeps the stage before it.
/// Control: the cancel of the empty transaction is what this holds;
/// without it the color line after the refusal is refused as busy.
#[test]
fn test_a_refused_preview_leaves_nothing_locked_and_keeps_the_stage_before() {
    use crate::application::document::Interaction;
    let mut doc = fixture();
    doc.selection.select_only(node(&doc, 1));
    let entries = doc.doc().history().count();
    assert!(
        run_line(&mut doc, "frame preview gap=-1").is_err(),
        "a negative gap is refused"
    );
    assert_eq!(
        doc.open_interaction(),
        None,
        "the transaction it began is cancelled"
    );
    assert_eq!(bytes(&doc), FIXTURE);
    assert_eq!(doc.doc().history().count(), entries);
    run_line(&mut doc, "color text=#ff0000").expect("the document is not left locked");
    doc.undo();

    run_line(&mut doc, "frame preview gap=2").expect("a preview stages");
    let staged = bytes(&doc);
    assert!(run_line(&mut doc, "frame preview gap=-1").is_err());
    let inset = Action::SetFramePreview {
        target_kind: FramePreviewTarget::Node,
        field: "inset".into(),
        value: "-3".into(),
    };
    assert_eq!(
        run(&inset, &mut doc),
        DispatchOutcome::Unhandled,
        "an Action's refusal too"
    );
    assert_eq!(bytes(&doc), staged, "the stage before them stands");
    assert_eq!(doc.open_interaction(), Some(Interaction::FramePreview));
    run_line(&mut doc, "frame cancel").expect("cancels");
    assert_eq!(bytes(&doc), FIXTURE);
}

/// **A preview of the map's default frame is the same transaction**:
/// `frame preview target=defaults` stages the defaults replaced with the
/// frame field changed, which cancels to the bytes before it with no
/// entry, and `SetFramePreview` with `defaults` commits as one entry
/// that one undo takes back. No selection is needed. Control: while
/// staged the bytes differ, so the cancel's comparison can fail.
#[test]
fn test_a_default_frame_preview_cancels_to_nothing_and_commits_as_one_entry() {
    let mut doc = fixture();
    let entries = doc.doc().history().count();
    run_line(&mut doc, "frame preview target=defaults fit=spread").expect("stages");
    assert!(
        bytes(&doc).contains("\"fit\": \"spread\""),
        "control: the default frame previews"
    );
    run_line(&mut doc, "frame cancel").expect("cancels");
    assert_eq!(bytes(&doc), FIXTURE, "cancelled: byte for byte");
    assert_eq!(doc.doc().history().count(), entries);

    let gap = Action::SetFramePreview {
        target_kind: FramePreviewTarget::Defaults,
        field: "gap".into(),
        value: "3".into(),
    };
    assert_eq!(run(&gap, &mut doc), DispatchOutcome::Handled);
    assert_eq!(
        run(&Action::CommitFramePreview, &mut doc),
        DispatchOutcome::Handled
    );
    assert_eq!(
        bytes(&doc),
        expected(&[(
            "      \"size\": 14.0\n    }\n",
            "      \"size\": 14.0\n    },\n    \"frame\": {\n      \"gap\": 3.0\n    }\n"
        )]),
        "the default frame's gap"
    );
    assert_eq!(doc.doc().history().count(), entries + 1, "committed: one entry");
    doc.undo();
    assert_eq!(bytes(&doc), FIXTURE);
}

/// **A ring added to several frames takes each frame's own last
/// channel**, so a frame saves alike whatever was selected beside it:
/// node 1's rings are on channel 0 and node 2's on 3, and `frame ring
/// add` on both gives node 1's new ring channel 0 (written as nothing)
/// and node 2's channel 3. Control: node 2's new ring does carry its
/// channel, so the comparison sees one.
#[test]
fn test_a_ring_added_to_several_frames_takes_each_frames_channel() {
    let map = r#"{"format": 1, "name": "c", "nodes": [
        {"id": 1, "position": [0.0, 0.0], "frame": {"rings": [{"spans": [{"text": "a"}]}]}},
        {"id": 2, "position": [0.0, 300.0], "frame": {"rings": [{"channel": 3, "spans": [{"text": "b"}]}]}}
    ]}"#;
    let mut doc = MindMapDocument::from_json_str(map, None, DEFAULT_HISTORY_LIMIT).expect("loads");
    select(&mut doc, Select::Nodes(&[1, 2]));
    run_line(&mut doc, "frame ring add text=new").expect("applies");
    let saved = bytes(&doc);
    assert!(
        saved.contains(r#""rings": [{"spans": [{"text": "a"}]}, {"spans": [{"text": "new"}]}]"#),
        "node 1's new ring is on its channel: {saved}"
    );
    assert!(
        saved.contains(
            r#"{"channel": 3, "spans": [{"text": "b"}]}, {"channel": 3, "spans": [{"text": "new"}]}"#
        ),
        "node 2's new ring is on its own: {saved}"
    );
}
