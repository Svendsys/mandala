// SPDX-License-Identifier: MPL-2.0

//! Presentations through the real app (V1-D2, #251; TEST_CONVENTIONS
//! §T8): that nothing a presentation shows is read into a change by any
//! `Action`, console verb or edit; the console's `present` verb, IPC's
//! `query.animations` and `clock.wait until=animations_complete` end to
//! end; and which answer a query gives while a presentation plays.
//!
//! **The differential** (ruling D2.1 §2). Every `Action` there is, every
//! console verb there is and every bundled edit is run twice on the same
//! map from the same state: once with nothing playing, once with a
//! presentation on every field of every element a presentation can
//! write. What each leaves behind — the reply, the save, the file on
//! disk, the history, the map after an undo, the clipboard, the
//! selection, the open interaction — must be the same. An `Action` or a
//! verb that reads the presented document to build its change leaves
//! something else, and a new `Action` or verb with no case here fails
//! the test by name.

use baumhard::arena::NodeId;
use baumhard::document::edit::parse_edits;
use baumhard::document::element::DocKind;
use baumhard::document::ids::NodeIdent;
use baumhard::document::presentation::{parse_presentations, PresentationDef};
use baumhard::util::test_temp::TempDir;
use serde_json::{json, Value};
use strum::IntoEnumIterator;

use super::gpu_harness::headless_app;
use super::ipc_target::IpcCommandTarget;
use super::run_native_init::InitState;
use super::step::Beat;
use crate::application::console::commands::COMMANDS;
use crate::application::document::selection::every_node;
use crate::application::document::MindMapDocument;
use crate::application::edits::EditRegistry;
use crate::application::ipc::{run_attached, Until};
use crate::application::keybinds::{Action, ActionKind};
use crate::application::view::inspect::children_of_kind;

/// The fixture: three nodes with sections, a frame, a connector and a
/// link, a theme and a palette.
fn fixture() -> String {
    format!(
        "{}/src/application/app/dispatch/families/fixtures/field.mandala.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn document(init: &InitState) -> &MindMapDocument {
    init.document
        .as_ref()
        .expect("the harness always builds a document")
}

fn document_mut(init: &mut InitState) -> &mut MindMapDocument {
    init.document
        .as_mut()
        .expect("the harness always builds a document")
}

/// Run one IPC request, through the real decoder and executor.
fn ipc(init: &mut InitState, request: Value) -> Value {
    run_attached(&request.to_string(), &mut IpcCommandTarget::new(init)).0
}

/// `act.console` with `line`: the whole reply.
fn console(init: &mut InitState, line: &str) -> Value {
    ipc(init, json!({"id": 1, "cmd": "act.console", "line": line}))
}

/// The presentations the differential plays: between them, a write of
/// every kind of field a presentation can write, on every kind of
/// element the fixture holds. Each holds at once but the last, which is
/// half way in when the step under test runs.
const PERFORMANCE: &str = r##"{"presentations":[
{"id":"p-node","name":"N","play":"hold","target":"node","mutations":[
  {"node":{"nudge":[13.0,-7.0]}},{"node":{"set_min_size":[333.0,222.0]}},{"node":{"set_opacity":0.4}},
  {"node":{"set_shape":"ellipse"}},{"node":"toggle_folded"},{"node":{"set_palette":"warm"}},
  {"node":{"set_zoom":{"min":0.5}}},
  {"node":{"text":{"set_color":"#123456"}}},{"node":{"text":{"set_size":31.0}}},
  {"node":{"text":{"set_weight":"bold"}}},{"node":{"text":{"set_italic":true}}}]},
{"id":"p-section","name":"S","play":"hold","target":"sections","mutations":[
  {"section":{"nudge":[5.0,9.0]}},{"section":{"set_min_size":[150.0,44.0]}},{"section":{"set_align":"center"}},
  {"section":{"text":{"set_color":"#654321"}}},{"section":{"text":{"set_size":27.0}}}]},
{"id":"p-span","name":"T","play":"hold","target":"spans","mutations":[
  {"span":{"set_text":"PRESENTED, NOT WRITTEN"}},{"span":{"text":{"set_size":23.0}}},
  {"span":{"text":{"set_color":"#abcdef"}}},{"span":{"text":{"set_weight":"bold"}}},
  {"span":{"text":{"set_italic":true}}},{"span":{"set_link":{"url":"https://example.org/"}}}]},
{"id":"p-frame","name":"F","play":"hold","target":"frame","mutations":[
  {"frame":{"set_visible":false}},{"frame":{"set_gap":7.0}},{"frame":{"set_inset":3.0}},
  {"frame":{"set_fit":"spread"}},{"frame":{"text":{"set_color":"#00ffff"}}}]},
{"id":"p-ring","name":"R","play":"hold","target":"rings","mutations":[
  {"ring":{"text":{"set_color":"#00ff00"}}},{"ring":{"set_fit":"spread"}}]},
{"id":"p-connector","name":"C","play":"hold","target":"connector","mutations":[
  {"path":{"set_color":"#ff00ff"}},{"path":{"set_size":19.0}},{"path":{"set_spacing":4.0}},
  {"path":{"set_content":"="}},{"path":{"set_visible":false}}]},
{"id":"p-links","name":"L","play":"hold","target":"links","mutations":[
  {"path":{"set_color":"#ffff00"}},{"path":{"set_size":17.0}},{"path":{"set_content":"+"}}]},
{"id":"p-moving","name":"M","play":"while","target":"node","mutations":[
  {"node":{"nudge":[40.0,40.0]}}],"animate":{"ms":1000}}
]}"##;

fn performance() -> Vec<PresentationDef> {
    parse_presentations(PERFORMANCE).unwrap_or_else(|error| panic!("the performance reads: {error}"))
}

/// Start the whole performance on every node of the open map, half a
/// second ago by the app clock, and bring the app to now: every
/// presentation shows, the last half way.
fn perform(init: &mut InitState) {
    let now = init.clock.now_ms();
    let doc = document_mut(init);
    let nodes: Vec<NodeId> = every_node(doc.doc()).collect();
    for def in performance() {
        doc.start_presentation(&def, &nodes, now)
            .unwrap_or_else(|error| panic!("`{}` starts: {error}", def.id));
    }
    init.clock.advance_to(now + 500.0);
    init.input_context().drain_frame(Beat::Loop);
    let doc = document(init);
    assert!(
        doc.doc().presentations().count() >= 8,
        "precondition: the performance plays"
    );
}

/// What to select before a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pick {
    Nothing,
    /// Node 1: two sections, a frame with a ring, a child.
    Node,
    /// Node 2: node 1's child, with a connector.
    Child,
    /// Node 1's first section.
    Section,
    /// Three graphemes of node 2's one section.
    Range,
    /// Node 2's connector.
    Connector,
    /// Node 3's link.
    Link,
    /// Nodes 1 and 3.
    Nodes,
}

const PICKS: [Pick; 8] = [
    Pick::Nothing,
    Pick::Node,
    Pick::Child,
    Pick::Section,
    Pick::Range,
    Pick::Connector,
    Pick::Link,
    Pick::Nodes,
];

fn pick(init: &mut InitState, what: Pick) {
    let doc = document_mut(init);
    let d = doc.doc();
    let node = |ident| d.node_by_ident(NodeIdent(ident)).expect("a fixture node");
    let child = |of: NodeId, kind| children_of_kind(d, of, kind).next().expect("a fixture element");
    let (one, two, three) = (node(1), node(2), node(3));
    let picked: Vec<NodeId> = match what {
        Pick::Nothing => Vec::new(),
        Pick::Node => vec![one],
        Pick::Child => vec![two],
        Pick::Section | Pick::Range => vec![child(
            if what == Pick::Range { two } else { one },
            DocKind::Section,
        )],
        Pick::Connector => vec![child(two, DocKind::Connector)],
        Pick::Link => vec![child(three, DocKind::Link)],
        Pick::Nodes => vec![one, three],
    };
    doc.selection.set(picked.iter().copied());
    if what == Pick::Range {
        doc.selection.select_range(picked[0], 1, 4);
    }
}

/// One step of a case: an `Action` as IPC spells it, or a console line.
#[derive(Clone, Debug)]
enum Step {
    Act(Value),
    Line(String),
}

impl Step {
    /// The step in a failure's words: the Action as IPC spells it, or
    /// the console line.
    fn name(&self) -> String {
        match self {
            Step::Act(action) => action.to_string(),
            Step::Line(line) => line.clone(),
        }
    }
}

/// What a case left behind.
#[derive(Debug, PartialEq)]
struct Left {
    replies: Vec<Value>,
    saved: String,
    on_disk: String,
    history: Vec<(String, &'static str)>,
    redo: usize,
    dirty: bool,
    undone: Option<String>,
    clipboard: String,
    selection: Value,
    interaction: String,
}

impl Left {
    /// The fields `other` differs in, each with both values cut short.
    fn differs_in(&self, other: &Left) -> Vec<(&'static str, String)> {
        fn cut(text: String) -> String {
            text.chars().take(400).collect()
        }
        let mut fields = Vec::new();
        let mut field = |name: &'static str, same: bool, plain: String, performed: String| {
            if !same {
                fields.push((name, format!("{} | performed: {}", cut(plain), cut(performed))));
            }
        };
        let text = |a: &str, b: &str| {
            let at = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
            let from = (0..=at.saturating_sub(60))
                .rev()
                .find(|&i| a.is_char_boundary(i) && b.is_char_boundary(i));
            let from = from.unwrap_or(0);
            (a[from..].to_string(), b[from..].to_string())
        };
        field(
            "replies",
            self.replies == other.replies,
            format!("{:?}", self.replies),
            format!("{:?}", other.replies),
        );
        let (a, b) = text(&self.saved, &other.saved);
        field("saved", self.saved == other.saved, a, b);
        let (a, b) = text(&self.on_disk, &other.on_disk);
        field("on_disk", self.on_disk == other.on_disk, a, b);
        field(
            "history",
            self.history == other.history,
            format!("{:?}", self.history),
            format!("{:?}", other.history),
        );
        field(
            "redo",
            (self.redo, self.dirty) == (other.redo, other.dirty),
            format!("{:?}", (self.redo, self.dirty)),
            format!("{:?}", (other.redo, other.dirty)),
        );
        let (a, b) = text(
            self.undone.as_deref().unwrap_or("-"),
            other.undone.as_deref().unwrap_or("-"),
        );
        field("undone", self.undone == other.undone, a, b);
        field(
            "clipboard",
            self.clipboard == other.clipboard,
            self.clipboard.clone(),
            other.clipboard.clone(),
        );
        field(
            "selection",
            self.selection == other.selection,
            self.selection.to_string(),
            other.selection.to_string(),
        );
        field(
            "interaction",
            self.interaction == other.interaction,
            self.interaction.clone(),
            other.interaction.clone(),
        );
        fields
    }
}

/// The parts of a reply that are the step's own: whether it ran, what it
/// answered, and the document revision it left.
fn own(reply: &Value) -> Value {
    json!({
        "ok": reply["ok"],
        "result": reply["result"],
        "error": { "code": reply["error"]["code"], "message": reply["error"]["message"] },
        "document_revision": reply["document_revision"],
    })
}

/// Run `steps` on a fresh copy of the fixture at `path`, with `what`
/// selected and — when `performed` — the performance playing, and
/// report what they left.
fn run_case(init: &mut InitState, path: &str, what: Pick, steps: &[Step], performed: bool) -> Left {
    // A clean slate: the last case's map saved aside, so that `open`
    // does not refuse to leave it; the files a case may have written
    // gone; the fixture, fresh.
    let dir = std::path::Path::new(path)
        .parent()
        .expect("the map is in the test's directory");
    let aside = dir.join("aside.mandala.json");
    let set_aside = console(init, &format!("save {}", aside.display()));
    assert_eq!(set_aside["ok"], true, "{set_aside}");
    for entry in std::fs::read_dir(dir).expect("the test's directory lists") {
        std::fs::remove_file(entry.expect("an entry").path()).expect("a file of the last case goes");
    }
    std::fs::write(
        path,
        std::fs::read_to_string(fixture()).expect("the fixture reads"),
    )
    .expect("writes");
    let opened = console(init, &format!("open {path}"));
    assert_eq!(opened["ok"], true, "{opened}");
    pick(init, what);
    if performed {
        perform(init);
    } else {
        // The same half second, and the same frame, with nothing
        // playing.
        let now = init.clock.now_ms();
        init.clock.advance_to(now + 500.0);
        init.input_context().drain_frame(Beat::Loop);
    }
    let replies = steps
        .iter()
        .map(|step| {
            let reply = match step {
                Step::Act(action) => ipc(init, json!({"id": 1, "cmd": "act.action", "action": action})),
                Step::Line(line) => console(init, line),
            };
            // The app clock only goes forward, so no two cases run at
            // one reading: a reply that names the reading (`clock
            // status`) names it as "now".
            let reading = format!("{:.0} ms", init.clock.now_ms());
            let own = own(&reply).to_string().replace(&reading, "now");
            serde_json::from_str(&own).expect("still a reply")
        })
        .collect();
    let on_disk = std::fs::read_to_string(path).unwrap_or_default();
    let doc = document_mut(init);
    if performed {
        assert!(
            !doc.doc().presentations_suspended(),
            "{steps:?}: the layer is back once the step returns"
        );
    }
    // An animated edit ends where it ends; compare the ends.
    doc.finish_animation();
    let saved = doc.save_json().expect("the map saves");
    let history: Vec<(String, &'static str)> = doc
        .doc()
        .history()
        .map(|entry| (entry.label.to_string(), entry.source.name()))
        .collect();
    let (redo, dirty) = (doc.doc().redo_len(), doc.is_dirty());
    let clipboard = format!("{:?}", doc.clipboard);
    let selection = doc.ipc_selection_json();
    let interaction = format!("{:?}", doc.open_interaction());
    let undone = (!history.is_empty()).then(|| {
        doc.undo();
        doc.save_json().expect("the map saves")
    });
    Left {
        replies,
        saved,
        on_disk,
        history,
        redo,
        dirty,
        undone,
        clipboard,
        selection,
        interaction,
    }
}

/// The payloads to run an `Action` that carries one with. Every kind
/// that is not a bare name has a row here.
fn payloads(kind: &str, dir: &TempDir) -> Vec<Value> {
    let path = |name: &str| dir.join(name).to_str().expect("a UTF-8 path").to_string();
    let one = |payload: Value| vec![json!({ kind: payload })];
    match kind {
        "ReparentToTarget" => vec![json!({ kind: 3 }), json!({ kind: null })],
        "ConnectToTarget" => vec![json!({ kind: 2 }), json!({ kind: null })],
        "SetPathAnchor" => one(json!({"from": "top", "to": "bottom"})),
        "SetPathContent" => one(json!("·")),
        "SetFrameField" => vec![
            json!({ kind: {"field": "gap", "value": "5"} }),
            json!({ kind: {"field": "fit", "value": "spread"} }),
        ],
        "SetFramePreview" => one(json!({"target_kind": "node", "field": "gap", "value": "5"})),
        "SetPathCap" => one(json!({"from": "<", "to": ">"})),
        "SetColor" => ["text", "border", "ring"]
            .into_iter()
            .map(|axis| json!({ kind: {"axis": axis, "value": "#ff0000"} }))
            .collect(),
        "SetLinkKind" => one(json!("portal")),
        "SetFontFamily" => one(json!("Galatia SIL")),
        "ReplaceFontFamily" => one(json!({"family": "Nope Sans", "with": "Galatia SIL"})),
        "SetFont" => [("size", "20"), ("weight", "bold"), ("italic", "true")]
            .into_iter()
            .map(|(slot, value)| json!({ kind: {"slot": slot, "value": value} }))
            .collect(),
        "SetLabelText" => one(json!("hello")),
        "SetLabelPosition" => one(json!({"t": "0.3", "offset": "4"})),
        "SetPathSpacing" => one(json!("3")),
        "SetZoom" => one(json!({"bound": "min", "value": "0.5"})),
        "SetSectionOffsetDelta" => one(json!({"dx": "4", "dy": "6"})),
        "SetSectionSizeAbs" => one(json!({"w": "120", "h": "50"})),
        "SetSectionOffsetAbs" => one(json!({"x": "10", "y": "20"})),
        "SetSectionText" => one(json!({"text": "new text"})),
        "AddSection" => one(json!({"at": "1", "text": "added"})),
        "SplitSection" => one(json!({"at_grapheme": "2"})),
        "OpenDocument" => one(json!(fixture())),
        "SaveDocumentAs" => one(json!(path("saved-as.mandala.json"))),
        "NewDocumentAt" => one(json!(path("new-at.mandala.json"))),
        _ => Vec::new(),
    }
}

/// Every `Action`, as the steps that run it: a bare name, or each of
/// its payloads. A kind that is neither is a kind this test does not
/// walk, and fails it.
fn every_action(dir: &TempDir) -> Vec<Step> {
    let mut steps = Vec::new();
    for kind in ActionKind::iter() {
        let name = format!("{kind:?}");
        let with_payload = payloads(&name, dir);
        if !with_payload.is_empty() {
            for payload in &with_payload {
                serde_json::from_value::<Action>(payload.clone())
                    .unwrap_or_else(|error| panic!("{payload}: not how `{name}` is spelled: {error}"));
            }
            steps.extend(with_payload.into_iter().map(Step::Act));
        } else if serde_json::from_value::<Action>(json!(name)).is_ok() {
            steps.push(Step::Act(json!(name)));
        } else {
            panic!(
                "the Action `{name}` carries a payload and has no row in `payloads`: add one, so it is \
                 run with and without a presentation"
            );
        }
    }
    steps
}

/// The console lines to run each verb with: forms that read the map to
/// change it or to answer. Every verb of the registry has a row, and
/// every line runs under some selection or is in [`REFUSED_LINES`].
/// `{map}` stands for the fixture's path.
const LINES: &[(&str, &[&str])] = &[
    ("help", &["help", "help font"]),
    (
        "color",
        &[
            "color text=#ff0000",
            "color ring=#00ff00",
            "color text=var(accent)",
        ],
    ),
    (
        "edit",
        &[
            "edit list",
            "edit apply grow-font-2pt",
            "edit apply shrink-font-2pt",
            "edit apply nudge-right-50",
            "edit apply flower",
            "edit apply cascade",
            "edit apply dim-opaque",
            "edit apply dim-opaque 1 2 3",
            "edit inspect dim-opaque",
            "edit help flower",
        ],
    ),
    (
        "present",
        &["present list", "present start lift", "present stop p-node"],
    ),
    // A map verb whose edit step reads an opacity a presentation writes
    // (`fade-opaque`'s filter): it must read the map itself.
    ("verb", &["verb list", "verb run fade 1"]),
    (
        "font",
        &[
            "font size=20",
            "font weight=bold italic=true",
            "font family=\"Galatia SIL\"",
        ],
    ),
    ("fps", &["fps on", "fps off"]),
    ("mode", &["mode default"]),
    ("save", &["save"]),
    ("screenshot", &["screenshot a.png b.png"]),
    ("record", &["record pause"]),
    ("clock", &["clock status", "clock step ms=32"]),
    ("open", &["open {map}"]),
    ("new", &["new"]),
    (
        "node",
        &[
            "node fit",
            "node shape=ellipse opacity=0.5",
            "node hidden=true",
            "node folded=true",
            "node channel=2",
            "node palette=warm",
        ],
    ),
    (
        "section",
        &[
            "section text hi",
            "section move dx=5 dy=3",
            "section move x=5 y=3",
            "section text hi section=1",
            "section add text=more",
            "section split at=2",
            "section delete",
            "section show",
            "section show section=1",
        ],
    ),
    (
        "frame",
        &[
            "frame on",
            "frame off",
            "frame start=top",
            "frame ring add text=hi",
            "frame ring 0 text=changed",
            "frame preview fit=spread",
        ],
    ),
    ("range", &["range 0 2 weight=bold", "range 1 3 color=#ff0000"]),
    (
        "path",
        &[
            "path content=+ size=12 spacing=3",
            "path anchors=top,bottom visible=false",
            "path straight",
            "path curve",
            "path reset",
            "path cap start=< end=>",
        ],
    ),
    ("link", &["link kind=portal"]),
    ("label", &["label text=hi t=0.3"]),
    ("portal", &[PORTAL_LINE]),
    ("theme", &["theme variant light"]),
    (
        "defaults",
        &["defaults frame.fit=repeat", "defaults section.size=18"],
    ),
    ("zoom", &["zoom min=0.5 max=2", "zoom clear"]),
];

/// The lines of [`LINES`] that are refused under every selection, and
/// why they are there all the same.
const REFUSED_LINES: &[(&str, &str)] = &[
    (
        "screenshot a.png b.png",
        "a screenshot is a file of what is drawn; this form is refused before it writes one",
    ),
    (
        "record pause",
        "a recording is files of what is drawn; this form is refused before it starts one",
    ),
];

/// A portal end's line: the fixture holds no portal link, so it runs
/// after `link kind=portal` makes one ([`sequences`]).
const PORTAL_LINE: &str = "portal end=0 content=x size=12";

/// The Actions and console lines that do nothing alone, each run after
/// the step it needs by one of [`sequences`].
const AFTER_ANOTHER: &[&str] = &[
    "Undo",
    "Redo",
    "Paste",
    "CommitFramePreview",
    "CancelFramePreview",
    "ReparentToTarget",
    "ConnectToTarget",
    "ReplaceFontFamily",
    // A section is edited once NodeEdit picks it (V1-C8).
    "EnterSectionEdit",
    // A label is placed once a path has one.
    "SetLabelPosition",
    PORTAL_LINE,
    // The picker's keys act on an open picker (V1-C7).
    "PickerCancel",
    "PickerCommit",
    "PickerNudgeHueDown",
    "PickerNudgeHueUp",
    "PickerNudgeSatDown",
    "PickerNudgeSatUp",
    "PickerNudgeValDown",
    "PickerNudgeValUp",
    "CloseColorPicker",
];

/// The Actions that act on a pointer gesture no step of this walk can
/// make: a fast resize starts from a held button's drag (`act.action`
/// has no button to hold it, and is refused for that, as on `main`), and
/// is walked through the pointer in `tests_gpu_live`.
const POINTER_GESTURES: &[&str] = &["FastResizeStart", "DoubleClickActivate"];

/// Every console verb, as the lines that run it. A verb of the registry
/// with no row in [`LINES`] is a verb this test does not walk, and
/// fails it.
fn every_line() -> Vec<Step> {
    let mut steps = Vec::new();
    for command in COMMANDS {
        let lines = LINES
            .iter()
            .find(|(name, _)| *name == command.name)
            .unwrap_or_else(|| {
                panic!(
                    "the console verb `{}` has no row in `LINES`: add one, so it is run with and \
                     without a presentation",
                    command.name
                )
            })
            .1;
        assert!(!lines.is_empty(), "`{}` has a line", command.name);
        steps.extend(
            lines
                .iter()
                .map(|line| Step::Line(line.replace("{map}", &fixture()))),
        );
    }
    assert_eq!(LINES.len(), COMMANDS.len(), "no row names a verb that is gone");
    steps
}

/// Steps that only mean something after another: a paste after a copy
/// or a cut, a preview committed or canceled, a target clicked in a
/// mode, an undo and a redo of a change made while the performance
/// plays, a presentation stopped that a line started.
fn sequences() -> Vec<Vec<Step>> {
    let act = |name: &str| Step::Act(json!(name));
    let line = |line: &str| Step::Line(line.to_string());
    let preview =
        || Step::Act(json!({"SetFramePreview": {"target_kind": "node", "field": "gap", "value": "9"}}));
    vec![
        vec![act("Copy"), act("Paste")],
        vec![act("Cut"), act("Paste")],
        vec![act("Copy"), act("Paste"), act("Undo"), act("Redo")],
        vec![act("Cut"), act("Undo"), act("Paste")],
        vec![preview(), act("CommitFramePreview")],
        vec![preview(), act("CancelFramePreview")],
        vec![line("frame preview fit=spread"), line("frame commit")],
        vec![line("frame preview fit=spread"), line("frame cancel")],
        vec![
            act("EnterReparentMode"),
            Step::Act(json!({"ReparentToTarget": 3})),
        ],
        vec![act("EnterConnectMode"), Step::Act(json!({"ConnectToTarget": 3}))],
        vec![line("color text=#ff0000"), act("Undo"), act("Redo")],
        vec![
            line("section text hi"),
            line("range 0 1 weight=bold"),
            act("Undo"),
        ],
        vec![act("CreateOrphanNode"), act("Undo"), act("Redo")],
        // The editors (V1-C8): a session typed into, then committed as
        // one entry or canceled away.
        vec![
            act("EnterNodeEdit"),
            act("EnterSectionEdit"),
            act("TextEditInsertSpace"),
            act("TextEditCommit"),
        ],
        vec![
            act("EnterNodeEdit"),
            act("EnterSectionEdit"),
            act("TextEditCancel"),
        ],
        vec![
            act("LabelEditOnSelection"),
            act("LabelEditInsertSpace"),
            act("LabelEditCommit"),
        ],
        // A replacement rewrites an installed family the map names, so
        // one is named first (#329).
        vec![
            Step::Act(json!({"SetFontFamily": "Nope Sans"})),
            Step::Act(json!({"ReplaceFontFamily": {"family": "Nope Sans", "with": "Galatia SIL"}})),
            act("Undo"),
        ],
        vec![
            line("edit apply nudge-right-50"),
            line("edit apply flower"),
            act("Undo"),
        ],
        vec![line("present start lift"), line("present stop lift")],
        vec![
            Step::Act(json!({"SetLabelText": "hello"})),
            Step::Act(json!({"SetLabelPosition": {"t": "end", "offset": "6"}})),
        ],
        vec![line("link kind=portal"), line(PORTAL_LINE), act("Undo")],
        // The contextual picker: a nudge staged on the selection, then
        // committed as one entry, or canceled; and the standalone
        // palette's commit, one `apply` (V1-C7).
        vec![
            line("color text"),
            act("PickerNudgeHueUp"),
            act("PickerNudgeSatDown"),
            act("PickerNudgeValDown"),
            act("PickerCommit"),
        ],
        vec![
            line("color border"),
            act("PickerNudgeHueDown"),
            act("PickerNudgeSatUp"),
            act("PickerNudgeValUp"),
            act("PickerCancel"),
        ],
        vec![
            act("OpenColorPicker"),
            act("PickerNudgeHueUp"),
            act("PickerCommit"),
            act("CloseColorPicker"),
        ],
    ]
}

/// The steps that answer about what is drawn, or about the performance
/// itself, and the one field each may so differ in. Everything else of
/// theirs — the save, the history, the file — is held like any step's.
/// Held exactly: a row whose step no longer differs fails the test.
const ANSWERS_WHAT_IS_DRAWN: &[(&str, &str)] = &[
    // The performance folds node 1, and what is selected is what can be
    // seen: node 2 is under the fold.
    ("\"SelectAll\"", "selection"),
    ("\"InvertSelection\"", "selection"),
    // The readout of what plays, and stopping what plays.
    ("present list", "replies"),
    ("present stop p-node", "replies"),
];

/// Whether a reply says its step ran: an Action handled, a line
/// answered without an error.
fn ran(reply: &Value) -> bool {
    reply["ok"] == true
        && reply["result"]["outcome"]
            .as_str()
            .is_none_or(|outcome| outcome == "handled")
}

/// Why `step` runs under no selection when it is the only step, if this
/// walk knows a reason: its Action is a key of an open text editor's or
/// of the console's own line, or one of
/// [`AFTER_ANOTHER`]; its line is one of [`AFTER_ANOTHER`] or of
/// [`REFUSED_LINES`].
fn excuse(step: &Step) -> Option<&'static str> {
    match step {
        Step::Act(action) => {
            let kind = match action {
                Value::String(name) => name.as_str(),
                Value::Object(payload) => payload.keys().next().map_or("", String::as_str),
                _ => "",
            };
            let action: Action = serde_json::from_value(action.clone()).expect("an Action");
            let context = crate::application::keybinds::ActionKind::from(&action).context();
            if matches!(
                context,
                crate::application::keybinds::InputContext::TextEdit
                    | crate::application::keybinds::InputContext::LabelEdit
            ) {
                Some("a key of an open text editor's, which no step here opens")
            } else if kind.starts_with("Console") {
                Some("a key of the console's own line, which is not open")
            } else if AFTER_ANOTHER.contains(&kind) {
                Some("it does nothing alone, and is run by a sequence")
            } else if POINTER_GESTURES.contains(&kind) {
                Some("it acts on a pointer gesture, which no step here makes")
            } else {
                None
            }
        }
        Step::Line(line) if AFTER_ANOTHER.contains(&line.as_str()) => {
            Some("it does nothing alone, and is run by a sequence")
        }
        Step::Line(line) => REFUSED_LINES
            .iter()
            .find(|(refused, _)| refused == line)
            .map(|(_, why)| *why),
    }
}

/// An edit whose filter reads a field the performance writes: it dims
/// only the nodes that are opaque on the map.
const DIM_OPAQUE: &str = r#"{"id":"dim-opaque","name":"Dim what is opaque","reach":"self","target":"node",
"filter":{"fields":[[{"opacity":0.5},{"GreaterThan":false}]]},
"mutations":[{"node":{"set_opacity":0.9}}]}"#;

/// **Every `Action`, console verb and edit leaves the same map, history
/// and files with a performance playing as with none** (ruling D2.1 §2
/// on #251).
///
/// The walk is held to being one: every Action and every verb has a
/// step, every step runs under some selection or has a reason this
/// test knows, and a reason that no longer holds fails it.
///
/// Controls, each seen red with one funnel left reading the performed
/// map: `dispatch_compatible` — `Copy` and `Cut` take the presented
/// text and colors; the console's execution — `section show` reads the
/// presented offset, size and text; `run_edit` — `flower` and `cascade`
/// lay out from the lifted positions, and `dim-opaque` reaches no node;
/// `edit inspect` — it lists what the faded map would reach.
#[test]
fn test_every_action_verb_and_edit_leaves_what_it_leaves_with_nothing_playing() {
    let Some(mut app) = headless_app(&fixture()) else {
        return;
    };
    let init = &mut app.init;
    init.input_context().drain_frame(Beat::Loop);
    let dir = TempDir::new("presentation-differential");
    let path = dir.join("map.mandala.json");
    let path = path.to_str().expect("a UTF-8 path");
    // The app's edits, and one whose filter reads what the performance
    // writes.
    let mut edits = parse_edits(include_str!("../../../assets/edits.json")).expect("the bundled edits read");
    edits.extend(parse_edits(&format!(r#"{{"edits":[{DIM_OPAQUE}]}}"#)).expect("the test edit reads"));
    init.edits = EditRegistry::new(edits, Vec::new());

    let singles: Vec<Step> = every_action(&dir).into_iter().chain(every_line()).collect();
    let sequences = sequences();
    let mut cases: Vec<(Pick, Vec<Step>)> = Vec::new();
    for step in &singles {
        for what in PICKS {
            cases.push((what, vec![step.clone()]));
        }
    }
    for steps in &sequences {
        for what in [
            Pick::Node,
            Pick::Child,
            Pick::Section,
            Pick::Nodes,
            Pick::Connector,
            Pick::Link,
        ] {
            cases.push((what, steps.clone()));
        }
    }

    let mut changed = 0;
    let mut copied = 0;
    let mut differing = Vec::new();
    let mut drawn = vec![0usize; ANSWERS_WHAT_IS_DRAWN.len()];
    // Per case list (a single step, or a sequence): whether every step
    // of it ran under some selection, and its last step's reply under
    // each.
    let mut ran_once: Vec<(Vec<Step>, bool, Vec<Value>)> = Vec::new();
    for (what, steps) in &cases {
        let plain = run_case(init, path, *what, steps, false);
        let performed = run_case(init, path, *what, steps, true);
        let mut fields = plain.differs_in(&performed);
        let row = match &steps[..] {
            [step] => ANSWERS_WHAT_IS_DRAWN
                .iter()
                .position(|(name, _)| *name == step.name()),
            _ => None,
        };
        if let Some(row) = row {
            let before = fields.len();
            fields.retain(|(field, _)| *field != ANSWERS_WHAT_IS_DRAWN[row].1);
            drawn[row] += before - fields.len();
        }
        if !fields.is_empty() {
            let fields: Vec<String> = fields
                .iter()
                .map(|(field, values)| format!("{field}: {values}"))
                .collect();
            differing.push(format!("{steps:?} with {what:?} selected: {}", fields.join("; ")));
        }
        changed += usize::from(!plain.history.is_empty());
        copied += usize::from(plain.clipboard != "None");
        let all_ran = plain.replies.iter().all(ran) || performed.replies.iter().all(ran);
        let last = plain.replies.last().expect("a case has a step").clone();
        match ran_once.last_mut() {
            Some((same, any, replies)) if format!("{same:?}") == format!("{steps:?}") => {
                *any |= all_ran;
                replies.push(last);
            }
            _ => ran_once.push((steps.clone(), all_ran, vec![last])),
        }
    }
    assert!(
        differing.is_empty(),
        "{} of {} cases left something else under a performance:\n{}",
        differing.len(),
        cases.len(),
        differing.join("\n")
    );
    for (row, differed) in ANSWERS_WHAT_IS_DRAWN.iter().zip(&drawn) {
        assert!(
            *differed > 0,
            "`{}` is listed as answering what is drawn in its {}, and answered the same: the row is stale",
            row.0,
            row.1
        );
    }
    // The walk is one: a step that runs nowhere compares nothing.
    assert_eq!(ran_once.len(), singles.len() + sequences.len());
    let mut idle = Vec::new();
    for (steps, any, replies) in &ran_once {
        match &steps[..] {
            [step] => match (any, excuse(step)) {
                (true, None) | (false, Some(_)) => {}
                (false, None) => idle.push(format!(
                    "`{}` ran under no selection ({}): a mistyped line or payload compares nothing",
                    step.name(),
                    replies[0]
                )),
                (true, Some(why)) => idle.push(format!(
                    "`{}` ran, and this test excuses it ({why}): the excuse is stale",
                    step.name()
                )),
            },
            _ if *any => {}
            _ => idle.push(format!("the sequence {steps:?} ran whole under no selection")),
        }
    }
    assert!(idle.is_empty(), "the walk is not one:\n{}", idle.join("\n"));
    for kind in AFTER_ANOTHER {
        assert!(
            sequences.iter().flatten().any(|step| step.name().contains(kind)),
            "`{kind}` does nothing alone, and no sequence runs it"
        );
    }
    // And it ran the changes it is about: held from below, so that a
    // funnel refusing everything cannot pass it.
    assert!(
        changed >= CHANGED,
        "{changed} of {} cases changed the map",
        cases.len()
    );
    assert!(
        copied >= COPIED,
        "{copied} of {} cases copied something",
        cases.len()
    );
}

/// How many cases of the walk change the map, and how many copy
/// something, at least.
const CHANGED: usize = 290;
const COPIED: usize = 28;

/// One IPC command's result, which must be a success.
fn result(init: &mut InitState, request: Value) -> Value {
    let reply = ipc(init, request.clone());
    assert_eq!(reply["ok"], true, "{request}: {reply}");
    reply["result"].clone()
}

/// Where node `id` is drawn, and where the document has it:
/// `scene.node_geometry`'s `canvas` corner, and `query.document`'s
/// position of the node — which `scene.node_geometry`'s `model` must
/// agree with, being the one part of that reply that is the document's.
fn drawn_and_saved(init: &mut InitState, id: u64) -> ((f64, f64), Value) {
    let geometry = result(
        init,
        json!({"id": 1, "cmd": "scene.node_geometry", "node_id": id}),
    );
    let canvas = &geometry["canvas"];
    let drawn = (
        canvas["x"].as_f64().expect("drawn"),
        canvas["y"].as_f64().expect("drawn"),
    );
    let pointer = format!("/nodes/{}/position", id - 1);
    let saved = result(
        init,
        json!({"id": 1, "cmd": "query.document", "pointer": pointer}),
    );
    let saved = saved["document"].clone();
    let model = &geometry["model"]["position"];
    assert_eq!(
        json!([model["x"], model["y"]]),
        saved,
        "`model` is the map's own node, whatever is drawn"
    );
    (drawn, saved)
}

/// Whether `clock.wait until=animations_complete` would hold now, as
/// the app reads it at a wait's evaluation point. (What a wait that
/// runs out then says is `ipc::wait`'s, and tested there.)
fn at_rest(init: &InitState) -> bool {
    init.settling().holds(Until::AnimationsComplete)
}

/// **A presentation played from the console is drawn, reported by
/// `query.animations`, waited for by `animations_complete`, and never
/// in the document**: `scene.node_geometry` answers where the node is
/// drawn, `query.document` where the map has it (`format/ipc.md` "The
/// map itself, or what is drawn").
///
/// Controls, each seen red: with the frame's tick not bringing the
/// presentations to the clock, `t` stays `0` after the first step; with
/// `format::save` reading the tree as it stands, `query.document`
/// answers the lifted position; with `scene.node_geometry` reading the
/// node off the tree, its `model` does.
#[test]
fn test_a_presentation_from_the_console_is_drawn_reported_and_never_in_the_document() {
    let Some(mut app) = headless_app(&fixture()) else {
        return;
    };
    let init = &mut app.init;
    init.input_context().drain_frame(Beat::Loop);
    assert!(
        init.clock.is_virtual(),
        "precondition: a headless app's clock is stepped"
    );
    let (home, saved_home) = drawn_and_saved(init, 2);
    assert_eq!(saved_home, json!([300.0, 0.0]));
    let idle = result(init, json!({"id": 1, "cmd": "query.animations"}));
    assert_eq!(
        idle,
        json!({"moving": false, "epoch": idle["epoch"], "edit": null, "count": 0, "presentations": [], "verbs": []})
    );
    assert!(at_rest(init), "nothing moves: the wait holds at once");

    // `lift`: 6 units up over 150 ms on 1 - (1 - f)², back over 300 ms.
    let started = console(init, "present start lift 2");
    assert_eq!(
        started["result"]["output"],
        json!(["present: `lift` is playing on 1 node"]),
        "{started}"
    );
    let instance = |t: f64, direction: &str, moving: bool| {
        json!({"id": 0, "presentation": "lift", "node": 2, "layer": "document", "play": "while",
               "t": t, "direction": direction, "moving": moving, "elements": 1, "refusal": null})
    };
    let playing = result(init, json!({"id": 1, "cmd": "query.animations"}));
    assert_eq!(playing["presentations"], json!([instance(0.0, "in", true)]));
    assert_eq!((&playing["moving"], &playing["count"]), (&json!(true), &json!(1)));
    assert!(
        playing["epoch"].as_u64() > idle["epoch"].as_u64(),
        "a start moves the epoch"
    );
    let settling = init.settling();
    assert!(
        !at_rest(init) && settling.animating && !settling.looping,
        "{settling:?}"
    );
    assert!(
        settling.holds(Until::Settled),
        "and a stepped clock is settled all the same: at this instant nothing more happens"
    );

    // 80 ms: five frames, f = 8/15, t = 1 - (7/15)².
    result(init, json!({"id": 1, "cmd": "clock.step", "ms": 80}));
    let t = 1.0 - (7.0_f64 / 15.0).powi(2);
    let mid = result(init, json!({"id": 1, "cmd": "query.animations"}));
    let reported = mid["presentations"][0]["t"].as_f64().expect("t");
    assert!(
        (reported - t).abs() < 1e-6,
        "{reported} at 80 ms, {t} by the curve"
    );
    let (drawn, saved) = drawn_and_saved(init, 2);
    assert!(
        (drawn.1 - (home.1 - 6.0 * t)).abs() < 1e-3,
        "drawn at {drawn:?}, home {home:?}"
    );
    assert_eq!(drawn.0, home.0);
    assert_eq!(saved, saved_home, "the map has the node where it was");

    // 150 ms: it holds, and nothing moves.
    result(init, json!({"id": 1, "cmd": "clock.step", "ms": 70}));
    let held = result(init, json!({"id": 1, "cmd": "query.animations"}));
    assert_eq!(held["presentations"], json!([instance(1.0, "hold", false)]));
    assert_eq!(held["moving"], false);
    assert!(at_rest(init), "held, the animations are complete");
    let (drawn, saved) = drawn_and_saved(init, 2);
    assert_eq!((drawn, saved), ((home.0, home.1 - 6.0), saved_home.clone()));

    // Nothing of it is the document's.
    let document = result(init, json!({"id": 1, "cmd": "query.document"}));
    assert_eq!(
        (&document["dirty"], &document["revision"], &document["history"]),
        (&json!(false), &json!(0), &json!({"undo": 0, "redo": 0}))
    );

    // Stopped, it returns over 300 ms and is over.
    let stopped = console(init, "present stop lift 2");
    assert_eq!(
        stopped["result"]["output"],
        json!(["present: stopped `lift` on 1 node"]),
        "{stopped}"
    );
    result(init, json!({"id": 1, "cmd": "clock.step", "ms": 150}));
    let returning = result(init, json!({"id": 1, "cmd": "query.animations"}));
    assert_eq!(returning["presentations"], json!([instance(0.5, "out", true)]));
    let (drawn, _) = drawn_and_saved(init, 2);
    assert!((drawn.1 - (home.1 - 3.0)).abs() < 1e-3, "{drawn:?}");
    result(init, json!({"id": 1, "cmd": "clock.step", "ms": 150}));
    let over = result(init, json!({"id": 1, "cmd": "query.animations"}));
    assert_eq!((&over["count"], &over["moving"]), (&json!(0), &json!(false)));
    assert_eq!(drawn_and_saved(init, 2), (home, saved_home));
}

/// **`animations_complete` waits for an animated edit, and never for a
/// loop that nobody stops**, and the app says which keeps it;
/// `query.animations` reports the edit's timing while it runs. A loop
/// stopped half way in plays the rest of its way in and all of its way
/// out, and is then over.
///
/// Controls, each seen red: with the app reading only the presentations
/// into `Settling::animating`, the wait holds while the edit is mid-way;
/// a loop stopped on its way in that does not finish its cycle is not on
/// its way out 100 ms later.
#[test]
fn test_animations_complete_waits_for_an_edit_and_names_a_loop() {
    let Some(mut app) = headless_app(&fixture()) else {
        return;
    };
    let init = &mut app.init;
    init.input_context().drain_frame(Beat::Loop);
    pick(init, Pick::Child);
    // `nudge-right-50`: 150 ms on f²(3 - 2f).
    let ran = console(init, "edit apply nudge-right-50");
    assert_eq!(ran["ok"], true, "{ran}");
    result(init, json!({"id": 1, "cmd": "clock.step", "ms": 48}));
    let running = result(init, json!({"id": 1, "cmd": "query.animations"}));
    let f = 48.0_f64 / 150.0;
    assert_eq!(running["moving"], true);
    assert_eq!(
        (
            &running["edit"]["elapsed_ms"],
            &running["edit"]["ms"],
            &running["edit"]["delay_ms"]
        ),
        (&json!(48.0), &json!(150), &json!(0))
    );
    assert_eq!(running["edit"]["easing"], "ease_in_out");
    let t = running["edit"]["t"].as_f64().expect("t");
    assert!((t - f * f * (3.0 - 2.0 * f)).abs() < 1e-6, "{t}");
    let settling = init.settling();
    assert!(
        !at_rest(init) && settling.animating && settling.virtual_clock && !settling.looping,
        "{settling:?}"
    );
    result(init, json!({"id": 1, "cmd": "clock.step", "ms": 112}));
    let done = result(init, json!({"id": 1, "cmd": "query.animations"}));
    assert_eq!((&done["moving"], &done["edit"]), (&json!(false), &Value::Null));
    assert!(at_rest(init));

    // A loop: stepping never brings it to rest, and the wait says what
    // would.
    let breathe = parse_presentations(
        r#"{"presentations":[{"id":"breathe","name":"Breathe","play":"loop","target":"node",
        "mutations":[{"node":{"set_opacity":0.5}}],"animate":{"ms":100},"return":{"ms":100}}]}"#,
    )
    .expect("reads")
    .remove(0);
    let now = init.clock.now_ms();
    let doc = document_mut(init);
    let nodes: Vec<NodeId> = every_node(doc.doc()).collect();
    doc.start_presentation(&breathe, &nodes[..1], now)
        .expect("starts");
    result(init, json!({"id": 1, "cmd": "clock.step", "ms": 1_050}));
    let settling = init.settling();
    assert!(
        !at_rest(init) && settling.looping,
        "a loop is what keeps it: {settling:?}"
    );
    // 1050 ms is five cycles of 200 and half of the sixth's way in.
    let half_in = result(init, json!({"id": 1, "cmd": "query.animations"}));
    assert_eq!(
        (
            &half_in["presentations"][0]["direction"],
            &half_in["presentations"][0]["t"]
        ),
        (&json!("in"), &json!(0.5))
    );
    // Stopped there, it plays the rest of its way in (50 ms), then all
    // of its way out (100 ms), and is over.
    assert_eq!(
        document_mut(init).stop_presentation("breathe", &nodes[..1], now + 1_050.0),
        1
    );
    result(init, json!({"id": 1, "cmd": "clock.step", "ms": 100}));
    let returning = result(init, json!({"id": 1, "cmd": "query.animations"}));
    assert_eq!(
        (
            &returning["count"],
            &returning["presentations"][0]["direction"],
            &returning["moving"]
        ),
        (&json!(1), &json!("out"), &json!(true)),
        "50 ms into its way out: {returning}"
    );
    assert!(
        !at_rest(init) && !init.settling().looping,
        "a stopped loop will end"
    );
    result(init, json!({"id": 1, "cmd": "clock.step", "ms": 50}));
    assert!(
        at_rest(init) && !init.settling().looping,
        "stopped, it finished its cycle"
    );
    let over = result(init, json!({"id": 1, "cmd": "query.animations"}));
    assert_eq!(over["count"], 0);
}

/// The Action funnel's guard (`dispatch::the_map::OnTheMap`) puts the
/// presentation layer back when it is dropped — at the end of its scope,
/// on an early return, and while a panic unwinds — and one made for an
/// Action that reads only what is drawn leaves the layer applied. It is
/// what every Action goes through; `MindMapDocument::suspended`, the
/// other funnels' guard, is held the same way in `tests_presentations`.
///
/// Control, seen red: a guard whose drop does nothing leaves the
/// document suspended after each.
#[test]
fn test_the_action_funnels_guard_resumes_however_its_scope_ends() {
    use super::dispatch::the_map::OnTheMap;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    /// Whether the open map's layer is set aside, read through `init`.
    fn aside(init: &InitState) -> bool {
        document(init).doc().presentations_suspended()
    }
    /// Through a guard for an Action that reads the map, whether the
    /// layer is set aside; `None` when it returns early.
    fn early(init: &mut InitState, leave: bool) -> Option<bool> {
        let mut ctx = init.input_context();
        let (mut core, _) = ctx.split_borrow();
        let guard = OnTheMap::when(&mut core, true);
        let set_aside = guard.document.as_ref()?.doc().presentations_suspended();
        if leave {
            return None;
        }
        Some(set_aside)
    }

    let Some(mut app) = headless_app(&fixture()) else {
        return;
    };
    let init = &mut app.init;
    perform(init);
    assert!(!aside(init), "precondition: the layer is applied");
    {
        let mut ctx = init.input_context();
        let (mut core, _) = ctx.split_borrow();
        let guard = OnTheMap::when(&mut core, true);
        let doc = guard.document.as_ref().expect("a document");
        assert!(doc.doc().presentations_suspended(), "through the guard: the map");
    }
    assert!(!aside(init), "scope end");
    assert_eq!(early(init, false), Some(true));
    assert!(!aside(init), "after a return with a value");
    assert_eq!(early(init, true), None);
    assert!(!aside(init), "early return");
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        let mut ctx = init.input_context();
        let (mut core, _) = ctx.split_borrow();
        let _guard = OnTheMap::when(&mut core, true);
        panic!("an Action failed half way");
    }));
    assert!(unwound.is_err());
    assert!(!aside(init), "panic");
    // An Action that reads only what is drawn: the guard sets nothing
    // aside.
    {
        let mut ctx = init.input_context();
        let (mut core, _) = ctx.split_borrow();
        let guard = OnTheMap::when(&mut core, false);
        let doc = guard.document.as_ref().expect("a document");
        assert!(!doc.doc().presentations_suspended(), "what is drawn");
    }
    assert!(!aside(init));
}
