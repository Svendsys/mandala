// SPDX-License-Identifier: MPL-2.0

//! The authored-edit fuzz properties (design D29, amendment on #250;
//! TEST_CONVENTIONS §T14): edits a map, a user or the app defines are an
//! untrusted-input surface, so any bytes read as an edits file, and edits
//! built from choices, hold to:
//!
//! - **no panic**, in the reader, the resolution, the request or the
//!   writer;
//! - **bounded time**: the definition bounds (32 levels, 65,536 parts)
//!   cap what an edit can ask for, and no step repeats past them — the
//!   harness's per-input timeout is the check;
//! - **a refusal names its JSON path**: an edit that does not read is
//!   refused under `edits[i]`;
//! - **all or nothing**: an edit that reads, run on a map, either
//!   applies whole — and undo takes the map back to the byte — or is
//!   refused and leaves the map as it was.
//!
//! The fuzz targets (`fuzz/fuzz_targets/edits_*.rs`) are thin wrappers
//! over [`check_edits_json`] and [`check_edits_structured`]; the sweep in
//! `edit_fuzz_tests` runs the generator on stable, in `./test.sh`.

use crate::arena::NodeId;
use crate::document::edit::parse_edits;
use crate::document::element::{DocElement, DocKind};
use crate::document::format::{load, save, LoadErrorKind};
use crate::document::ids::Document;
use crate::document::writer::ChangeSource;
use crate::edit_handlers::registry;
use crate::flow::{Curve, Rect, ShapedBox};
use crate::projection::Geometry;
use crate::util::tests::choices::Choices;
use glam::Vec2;
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// Where nodes are drawn, for a test or a fuzz run: each node a 100 × 40
/// box at its position.
pub struct Boxes(pub FxHashMap<NodeId, Rect>);

impl Boxes {
    /// Every node of `doc` drawn at its position. Iterative.
    pub fn of(doc: &Document) -> Self {
        let mut rects = FxHashMap::default();
        let mut pending = vec![doc.root()];
        while let Some(id) = pending.pop() {
            if let Some(DocElement::Node(data)) = doc.get(id) {
                rects.insert(id, Rect::new(data.position, Vec2::new(100.0, 40.0)));
            }
            pending.extend(doc.children(id));
        }
        Boxes(rects)
    }
}

impl Geometry for Boxes {
    fn node_rect(&self, node: NodeId) -> Option<Rect> {
        self.0.get(&node).copied()
    }
    fn section_box(&self, _: NodeId) -> Option<(Vec2, &Arc<ShapedBox>)> {
        None
    }
    fn curve(&self, _: NodeId) -> Option<Curve> {
        None
    }
    fn label_pos(&self, _: NodeId) -> Option<Vec2> {
        None
    }
}

/// The map edits are run on: the format's fullest fixture.
const MAP: &str = include_str!("../format/fixtures/full.mandala.json");

/// Every property of `bytes` read as an edits file.
pub fn check_edits_json(bytes: &[u8]) -> Result<(), String> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(());
    };
    check_text(text, false)
}

/// Every property of the edits file `bytes` choose.
pub fn check_edits_structured(bytes: &[u8]) -> Result<(), String> {
    check_edits_generated(&mut Choices::new(bytes))
}

/// [`check_edits_structured`] for choices from any source.
pub fn check_edits_generated(choices: &mut Choices<'_>) -> Result<(), String> {
    check_text(&generate(choices), true)
}

/// The properties, on the edits file `text`; `generated` when it is one
/// of [`generate`]'s, which are always JSON of the file's shape, so a
/// refusal must be inside an edit.
fn check_text(text: &str, generated: bool) -> Result<(), String> {
    let edits = match parse_edits(text) {
        Ok(edits) => edits,
        Err(error) => {
            let inside = error.path.starts_with("edits[");
            if generated && !inside {
                return Err(format!(
                    "a generated edit was refused outside its path: {error}\n{text}"
                ));
            }
            // A refusal with no path is about the whole text: one that is
            // not JSON, or not an object, or an object that lacks `edits`.
            let about_the_whole = matches!(
                error.kind,
                LoadErrorKind::Syntax { .. }
                    | LoadErrorKind::Invalid { .. }
                    | LoadErrorKind::MissingKey { .. }
                    | LoadErrorKind::ByteOrderMark
            );
            if !generated && error.path.is_empty() && !about_the_whole {
                return Err(format!("a refusal names no path into the file: {error}"));
            }
            return Ok(());
        }
    };
    let mut doc = load(MAP).map_err(|error| format!("the map loads: {error}"))?;
    let handlers = registry();
    let anchors: Vec<NodeId> = doc
        .children(doc.root())
        .filter(|&id| doc.get(id).is_some_and(|e| e.kind() == DocKind::Node))
        .collect();
    let before = save(&doc).map_err(|error| format!("the map saves: {error}"))?;
    for edit in &edits {
        let geometry = Boxes::of(&doc);
        let revision = doc.revision();
        let Ok(request) = edit.request(&doc, &geometry, &handlers, &anchors) else {
            if doc.revision() != revision {
                return Err(format!("edit `{}` was refused, and the map changed", edit.id));
            }
            continue;
        };
        let applied = doc.apply(request, ChangeSource::Edit);
        let now =
            save(&doc).map_err(|error| format!("edit `{}`: the map no longer saves: {error}", edit.id))?;
        match applied {
            Ok(_) if doc.revision() != revision => {
                doc.undo();
                let undone = save(&doc).map_err(|error| format!("the map saves: {error}"))?;
                if undone != before {
                    return Err(format!("edit `{}`: undo did not take the map back", edit.id));
                }
            }
            // Applied and changed nothing, or refused: as it was.
            _ if now != before => {
                return Err(format!("edit `{}` changed the map without an entry", edit.id));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Reaches, as the file spells them.
const REACHES: [&str; 6] = ["self", "children", "descendants", "subtree", "parent", "siblings"];

/// Targets, as the file spells them.
const TARGETS: [&str; 8] = [
    "node",
    "sections",
    "spans",
    "frame",
    "rings",
    "connector",
    "links",
    "labels",
];

/// Kinds a mutator node can be keyed on.
const KINDS: [&str; 9] = [
    "node",
    "section",
    "span",
    "frame",
    "ring",
    "connector",
    "link",
    "label",
    "portal_end",
];

/// One mutation of each element, some at their bounds and past them.
const MUTATIONS: [&str; 24] = [
    r#"{"node":{"nudge":[50.0,0.0]}}"#,
    r#"{"node":{"set_position":[1e39,0.0]}}"#,
    r#"{"node":{"set_shape":{"rectangle":{"radius":250}}}}"#,
    r#"{"node":{"set_palette":"mono"}}"#,
    r#"{"node":{"set_click":"glow"}}"#,
    r#"{"node":{"set_opacity":-3.0}}"#,
    r#"{"node":"toggle_folded"}"#,
    r#"{"node":{"set_channel":4294967296}}"#,
    r#"{"section":{"set_offset":[4.0,4.0]}}"#,
    r#"{"section":{"text":{"set_color":"var(accent)"}}}"#,
    r#"{"span":{"text":{"grow_size":2.0}}}"#,
    r#"{"span":{"text":{"scale_size":1e30}}}"#,
    r#"{"span":{"insert_text":{"at":18446744073709551615,"text":"é"}}}"#,
    r#"{"span":{"delete_text":{"start":3,"end":1}}}"#,
    r#"{"span":{"set_link":{"verb":"nope"}}}"#,
    r#"{"frame":{"set_fit":{"fit":{"min":2.0,"max":1.0}}}}"#,
    r#"{"frame":{"layout":{"set_sides":"✦"}}}"#,
    r#"{"ring":{"text":{"set_size":0.5}}}"#,
    r#"{"path":{"set_control_points":[[1.0,2.0],[3.0,4.0],[5.0,6.0]]}}"#,
    r#"{"path":{"move_control_point":{"index":7,"to":[1.0,1.0]}}}"#,
    r#"{"path":{"set_color":"palette(frame)"}}"#,
    r#"{"cap":{"set_font":"Novem"}}"#,
    r#"{"label":{"set_t":2.0}}"#,
    r#"{"portal_end":{"set_border_t":0.5}}"#,
];

/// Filter fields.
const FIELDS: [&str; 8] = [
    r#"[{"kind":"span"},{"Equals":false}]"#,
    r#"[{"font_size":{"pt":12.0}},{"GreaterThan":false}]"#,
    r#"[{"color":"var(accent)"},{"Equals":true}]"#,
    r#"[{"palette":"rainbow"},{"Exists":false}]"#,
    r#"[{"on_click":"reveal"},{"Equals":false}]"#,
    r#"[{"font":"Nope"},{"Equals":false}]"#,
    r#"[{"fit":"repeat"},{"Equals":false}]"#,
    r#"[{"channel":1},{"LessThan":true}]"#,
];

/// An edits file of up to four edits, each built from `choices`.
pub fn generate(choices: &mut Choices<'_>) -> String {
    let count = 1 + choices.below(4);
    let edits: Vec<String> = (0..count).map(|i| edit(choices, i)).collect();
    format!(r#"{{"edits":[{}]}}"#, edits.join(","))
}

/// One edit.
fn edit(choices: &mut Choices<'_>, i: u32) -> String {
    let mut out = format!(
        r#"{{"id":"e{i}","name":"E","reach":"{}","target":"{}""#,
        choices.pick(&REACHES),
        choices.pick(&TARGETS)
    );
    if choices.one_in(3) {
        let fields: Vec<&str> = (0..choices.below(3)).map(|_| *choices.pick(&FIELDS)).collect();
        out.push_str(&format!(r#","filter":{{"fields":[{}]}}"#, fields.join(",")));
    }
    match choices.below(4) {
        0 => {}
        1 => {
            let mutations: Vec<&str> = (0..choices.below(4)).map(|_| *choices.pick(&MUTATIONS)).collect();
            out.push_str(&format!(r#","mutations":[{}]"#, mutations.join(",")));
        }
        2 => {
            let depth = 1 + choices.below(36);
            out.push_str(&format!(r#","mutator":{}"#, mutator(choices, depth)));
        }
        _ => out.push_str(&format!(
            r#","handler":"{}""#,
            choices.pick(&["flower", "cascade", "spiral"])
        )),
    }
    if choices.one_in(2) {
        out.push_str(&format!(
            r#","animate":{{"ms":{},"easing":"{}"}}"#,
            choices.below(1000),
            choices.pick(&["linear", "ease_in", "ease_out", "ease_in_out"])
        ));
    }
    out.push('}');
    out
}

/// A mutator `depth` levels deep along one branch, with a sibling or two
/// on the way. Iterative: the levels are built inside out.
fn mutator(choices: &mut Choices<'_>, depth: u32) -> String {
    let mut inner: Option<String> = None;
    for _ in 0..depth {
        let kind = choices.pick(&KINDS);
        let channel = choices.below(3);
        let mut children: Vec<String> = inner.take().into_iter().collect();
        if choices.one_in(4) {
            children.push(format!(
                r#"{{"void":{{"kind":"{}","channel":0}}}}"#,
                choices.pick(&KINDS)
            ));
        }
        let children = format!(r#","children":[{}]"#, children.join(","));
        inner = Some(match choices.below(3) {
            0 => format!(r#"{{"void":{{"kind":"{kind}","channel":{channel}{children}}}}}"#),
            1 => format!(
                r#"{{"macro":{{"kind":"{kind}","channel":{channel},"mutations":[{}]{children}}}}}"#,
                choices.pick(&MUTATIONS)
            ),
            _ => format!(
                r#"{{"instruction":{{"kind":"{kind}","channel":{channel},"instruction":{}{children}}}}}"#,
                choices.pick(&[
                    r#""repeat_while_always_true""#,
                    r#"{"repeat_while":{"fields":[[{"kind":"node"},{"Equals":false}]]}}"#,
                ])
            ),
        });
    }
    inner.unwrap_or_default()
}
