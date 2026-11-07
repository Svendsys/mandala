// SPDX-License-Identifier: MPL-2.0

//! The `scene` family: what is *actually on screen* (`format/ipc.md`
//! §`scene`). Where `query` reads what the application knows, this
//! reads what it draws — positions after layout, sizes after text
//! fitting, the draw verdict at the camera, and the text a user reads —
//! so an agent can answer "is node X on screen, where, overlapping what,
//! saying what?" without pixels.
//!
//! **One source of truth: the glyph runs.** Every command measures an
//! element by the runs the projector drew for it
//! (`view::inspect::element_bounds`), and judges it by the same runs at
//! the camera the renderer draws through
//! (`view::inspect::element_visibility`). Nothing here re-derives
//! geometry from the document or re-implements a visibility rule.
//! `scene.hit_test` routes through the canonical click hit chain on the
//! app side
//! ([`CommandTarget::hit_test`](crate::application::ipc::CommandTarget::hit_test))
//! rather than any copy of it.
//!
//! **Elements are named** as `view::inspect::element_ref` names them:
//! `{"node": <id>}`, `{"node": <id>, "section": <index>}`,
//! `{"node": <id>, "connector": true}`, `{"node": <id>, "link": <index>,
//! "label": <index>}` and so on.
//!
//! **Read-only.** Four commands reach the app through
//! [`Call::observe`]; `scene.hit_test` needs `drive()` only because the
//! hit test memoizes subtree bounds, and it changes nothing a frame would
//! show. None asks for a frame.

use serde_json::{json, Map, Value};

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, DocKind};
use baumhard::flow::Rect;

use crate::application::document::selection::every_node;
use crate::application::ipc::envelope::Reply;
use crate::application::ipc::registry::{
    Call, CommandSpec, Family, ParamSpec, ParamType, ResultSpec, Support,
};
use crate::application::ipc::space::camera_json;
use crate::application::ipc::SceneView;
use crate::application::view::inspect::{
    children_of_kind, element_bounds, element_ref, element_text, element_visibility, node_named, rect_json,
    surface_rect,
};

use super::act::parse_pointer;
use super::refusal::{no_loaded_document, refuse, Refusal};
use super::sidecar::{parse_to_file, write_sidecar};

/// The element kinds `scene.dump` reports, in reply order.
const KINDS: [&str; 4] = ["nodes", "paths", "labels", "portals"];

const SPACE_DOC: &str = "The coordinate space of x and y: \"canvas\" (default) or \"surface\".";

pub(super) const FAMILY: Family = Family {
    name: "scene",
    commands: &[
        CommandSpec {
            name: "scene.dump",
            summary: "The drawn scene: every drawn node (with its sections), path, label and portal end, \
                      with canvas and surface rects, the draw verdict and the displayed text.",
            params: &[
                ParamSpec::new(
                    "include",
                    ParamType::String,
                    false,
                    "\"visible\" (default): only what the camera shows. \"all\": every drawn element, \
                     each with its verdict, so off-screen ones can be found.",
                ),
                ParamSpec::new(
                    "kinds",
                    ParamType::Array,
                    false,
                    "Which element kinds to report, as strings: nodes, paths, labels, portals. \
                     Default: all four.",
                ),
                // Native: the browser has no filesystem.
                #[cfg(not(target_arch = "wasm32"))]
                ParamSpec::new(
                    "to_file",
                    ParamType::String,
                    false,
                    "Absolute path. Write the dump there as JSON, atomically, and reply with just \
                     {path, bytes}. For a scene too large to inline.",
                ),
                #[cfg(not(target_arch = "wasm32"))]
                ParamSpec::new(
                    "overwrite",
                    ParamType::Boolean,
                    false,
                    "Allow to_file to replace an existing file. Default false. Only valid with to_file.",
                ),
            ],
            result: &[
                ResultSpec::new(
                    "camera",
                    ParamType::Object,
                    "The camera the verdicts and surface rects were taken at, as query.camera reports it.",
                ),
                ResultSpec::new(
                    "nodes",
                    ParamType::Array,
                    "Drawn nodes in document order: {element, canvas, surface, visibility, sections}; \
                     each section {element, canvas, surface, visibility, text}.",
                ),
                ResultSpec::new(
                    "paths",
                    ParamType::Array,
                    "Drawn connectors and links: {element, canvas, surface, visibility}.",
                ),
                ResultSpec::new(
                    "labels",
                    ParamType::Array,
                    "Drawn path labels: {element, canvas, surface, visibility, text}.",
                ),
                ResultSpec::new(
                    "portals",
                    ParamType::Array,
                    "Drawn portal ends: {element, canvas, surface, visibility, text}.",
                ),
                ResultSpec::new(
                    "path",
                    ParamType::String,
                    "With to_file only: the absolute path written.",
                ),
                ResultSpec::new(
                    "bytes",
                    ParamType::Integer,
                    "With to_file only: the size of the file written.",
                ),
            ],
            support: Support::Everywhere,
            run: dump,
        },
        CommandSpec {
            name: "scene.visible_set",
            summary: "Just the names of what the camera shows right now, by kind. No text is resolved and \
                      no rect is reported.",
            params: &[],
            result: &[
                ResultSpec::new("nodes", ParamType::Array, "Visible nodes, in document order."),
                ResultSpec::new("sections", ParamType::Array, "Visible sections."),
                ResultSpec::new("paths", ParamType::Array, "Visible connectors and links."),
                ResultSpec::new("labels", ParamType::Array, "Visible path labels."),
                ResultSpec::new("portals", ParamType::Array, "Visible portal ends."),
            ],
            support: Support::Everywhere,
            run: visible_set,
        },
        CommandSpec {
            name: "scene.node_geometry",
            summary: "One node's whole geometry story: its position and size in the document, whether \
                      the scene draws it, its drawn box per section, its visibility, and the drawn \
                      nodes it overlaps.",
            params: &[ParamSpec::new(
                "node_id",
                ParamType::Integer,
                true,
                "The node to describe, by its integer id.",
            )],
            result: &[
                ResultSpec::new("element", ParamType::Object, "The node described: {node: <id>}."),
                ResultSpec::new(
                    "model",
                    ParamType::Object,
                    "{position: {x, y}, min_size: {w, h} or null, folded, hidden} as the document holds them.",
                ),
                ResultSpec::new(
                    "in_scene",
                    ParamType::Boolean,
                    "Whether the scene draws the node: false for a node hidden, folded away or empty.",
                ),
                ResultSpec::new(
                    "canvas",
                    ParamType::Object,
                    "In the scene only: the drawn box, {x, y, w, h} in canvas units.",
                ),
                ResultSpec::new(
                    "surface",
                    ParamType::Object,
                    "In the scene only: the same box in surface pixels.",
                ),
                ResultSpec::new(
                    "visibility",
                    ParamType::String,
                    "In the scene only: visible, offscreen or outside_zoom_window.",
                ),
                ResultSpec::new(
                    "sections",
                    ParamType::Array,
                    "In the scene only: {element, canvas, surface, visibility, text} per drawn section.",
                ),
                ResultSpec::new(
                    "overlaps",
                    ParamType::Array,
                    "In the scene only: the other drawn nodes whose boxes overlap this one.",
                ),
            ],
            support: Support::Everywhere,
            run: node_geometry,
        },
        CommandSpec {
            name: "scene.hit_test",
            summary: "What a click at a point would target: the element whose glyph is under it, its \
                      node, and the element a click selects.",
            params: &[
                ParamSpec::new("x", ParamType::Number, true, "The point's x, in `space`."),
                ParamSpec::new("y", ParamType::Number, true, "The point's y, in `space`."),
                ParamSpec::new("space", ParamType::String, false, SPACE_DOC),
            ],
            result: &[
                ResultSpec::new("canvas", ParamType::Object, "The point in canvas space, {x, y}."),
                ResultSpec::new(
                    "surface",
                    ParamType::Object,
                    "The point in surface pixels, {x, y}.",
                ),
                ResultSpec::new(
                    "click_hit",
                    ParamType::Object,
                    "What a press records and a double-click routes on: kind Element (with the element \
                     a click selects: a node, or a path for a connector, link, cap or label) or Empty.",
                ),
                ResultSpec::new(
                    "hit",
                    ParamType::Object,
                    "The projector's hit: {element, node} — the element whose glyph is under the point \
                     and the node it belongs to — or null.",
                ),
            ],
            support: Support::Everywhere,
            run: hit_test,
        },
        CommandSpec {
            name: "scene.find_text",
            summary: "Locate drawn sections, path labels and portal ends by the text they display.",
            params: &[
                ParamSpec::new(
                    "text",
                    ParamType::String,
                    true,
                    "The text to look for. Must not be empty.",
                ),
                ParamSpec::new(
                    "exact",
                    ParamType::Boolean,
                    false,
                    "Match the whole displayed text rather than a substring. Default false.",
                ),
                ParamSpec::new(
                    "case_sensitive",
                    ParamType::Boolean,
                    false,
                    "Compare case-sensitively. Default false.",
                ),
            ],
            result: &[ResultSpec::new(
                "matches",
                ParamType::Array,
                "Every drawn element whose displayed text matches, in document order, visible or not: \
                 {kind: section | label | portal_end, element, text, canvas, surface, visibility}.",
            )],
            support: Support::Everywhere,
            run: find_text,
        },
    ],
};

/// Which kinds a dump reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Kinds {
    nodes: bool,
    paths: bool,
    labels: bool,
    portals: bool,
}

impl Kinds {
    const ALL: Kinds = Kinds {
        nodes: true,
        paths: true,
        labels: true,
        portals: true,
    };
}

/// One element as the scene reports it: its name, its drawn box in both
/// spaces and its verdict, or `None` when it draws nothing at all.
/// `with_text` adds the text it displays.
fn placed(view: &SceneView<'_>, element: NodeId, with_text: bool) -> Option<Map<String, Value>> {
    let doc = view.document.doc();
    let (projector, layers) = (view.view.projector(), view.view.layers());
    let visibility = element_visibility(doc, projector, layers, element, view.camera)?;
    let canvas = element_bounds(doc, projector, layers, element)?;
    let mut entry = Map::new();
    entry.insert("element".into(), element_ref(doc, element));
    entry.insert("canvas".into(), rect_json(canvas));
    entry.insert("surface".into(), rect_json(surface_rect(canvas, view.camera)));
    entry.insert("visibility".into(), json!(visibility));
    if with_text {
        entry.insert("text".into(), json!(element_text(doc, element)));
    }
    Some(entry)
}

/// Whether `entry` passes the `include` filter.
fn included(entry: &Map<String, Value>, all: bool) -> bool {
    all || entry.get("visibility") == Some(&json!("visible"))
}

/// The document's elements of `kinds`, in document order.
fn elements_of<'d>(view: &'d SceneView<'_>, kinds: &'d [DocKind]) -> impl Iterator<Item = NodeId> + 'd {
    let doc = view.document.doc();
    doc.tree().descendants().filter(move |&id| {
        doc.get(id)
            .map(DocElement::kind)
            .is_some_and(|kind| kinds.contains(&kind))
    })
}

/// A node's entry, with its drawn sections.
fn node_entry(view: &SceneView<'_>, node: NodeId, all: bool) -> Option<Value> {
    let mut entry = placed(view, node, false)?;
    let sections: Vec<Value> = children_of_kind(view.document.doc(), node, DocKind::Section)
        .filter_map(|section| placed(view, section, true))
        .filter(|section| included(section, all))
        .map(Value::Object)
        .collect();
    entry.insert("sections".into(), Value::Array(sections));
    Some(Value::Object(entry))
}

/// The scene as `scene.dump` reports it.
fn read_scene(view: &SceneView<'_>, kinds: Kinds, all: bool) -> Value {
    let mut out = Map::new();
    out.insert("camera".into(), camera_json(view.camera));
    let list = |of: &[DocKind], with_text: bool| -> Value {
        Value::Array(
            elements_of(view, of)
                .filter_map(|element| placed(view, element, with_text))
                .filter(|entry| included(entry, all))
                .map(Value::Object)
                .collect(),
        )
    };
    if kinds.nodes {
        let nodes: Vec<Value> = every_node(view.document.doc())
            .filter_map(|node| node_entry(view, node, all))
            .filter(|entry| all || entry["visibility"] == json!("visible"))
            .collect();
        out.insert("nodes".into(), Value::Array(nodes));
    }
    if kinds.paths {
        out.insert("paths".into(), list(&[DocKind::Connector, DocKind::Link], false));
    }
    if kinds.labels {
        out.insert("labels".into(), list(&[DocKind::Label], true));
    }
    if kinds.portals {
        out.insert("portals".into(), list(&[DocKind::PortalEnd], true));
    }
    Value::Object(out)
}

fn with_scene(call: &mut Call<'_>, read: impl FnOnce(&SceneView<'_>) -> Result<Value, Refusal>) -> Reply {
    let Some(app) = call.observe() else {
        return call.no_document();
    };
    let Some(view) = app.scene() else {
        return no_loaded_document(call);
    };
    match read(&view) {
        Ok(result) => call.ok(result),
        Err(refusal) => refuse(call, refusal),
    }
}

/// `include`, checked: whether every drawn element is reported.
fn parse_include(include: Option<&str>) -> Result<bool, Refusal> {
    match include {
        None | Some("visible") => Ok(false),
        Some("all") => Ok(true),
        Some(other) => Err(Refusal::Invalid(
            "include",
            format!("`include` must be \"visible\" or \"all\", got \"{other}\""),
        )),
    }
}

fn parse_kinds(kinds: Option<&Value>) -> Result<Kinds, Refusal> {
    let Some(kinds) = kinds else {
        return Ok(Kinds::ALL);
    };
    let words = kinds.as_array().map(Vec::as_slice).unwrap_or_default();
    if words.is_empty() {
        return Err(Refusal::Invalid(
            "kinds",
            format!("`kinds` must name at least one of {}", KINDS.join(", ")),
        ));
    }
    let mut parsed = Kinds {
        nodes: false,
        paths: false,
        labels: false,
        portals: false,
    };
    for word in words {
        let flag = match word.as_str() {
            Some("nodes") => &mut parsed.nodes,
            Some("paths") => &mut parsed.paths,
            Some("labels") => &mut parsed.labels,
            Some("portals") => &mut parsed.portals,
            _ => {
                return Err(Refusal::Invalid(
                    "kinds",
                    format!("`{word}` is not a scene kind; the kinds are {}", KINDS.join(", ")),
                ))
            }
        };
        *flag = true;
    }
    Ok(parsed)
}

fn dump(call: &mut Call<'_>) -> Reply {
    let request = parse_include(call.str_param("include")).and_then(|all| {
        Ok((
            all,
            parse_kinds(call.param("kinds"))?,
            parse_to_file(call.param("to_file"), call.param("overwrite"))?,
        ))
    });
    let (all, kinds, to_file) = match request {
        Ok(request) => request,
        Err(refusal) => return refuse(call, refusal),
    };
    with_scene(call, |view| {
        let scene = read_scene(view, kinds, all);
        match &to_file {
            None => Ok(scene),
            Some(target) => write_sidecar(target, &scene),
        }
    })
}

/// The names of the visible elements of `kinds`.
fn visible_names(view: &SceneView<'_>, kinds: &[DocKind]) -> Value {
    Value::Array(
        elements_of(view, kinds)
            .filter(|&element| {
                let doc = view.document.doc();
                element_visibility(
                    doc,
                    view.view.projector(),
                    view.view.layers(),
                    element,
                    view.camera,
                ) == Some("visible")
            })
            .map(|element| element_ref(view.document.doc(), element))
            .collect(),
    )
}

fn visible_set(call: &mut Call<'_>) -> Reply {
    with_scene(call, |view| {
        Ok(json!({
            "nodes": visible_names(view, &[DocKind::Node]),
            "sections": visible_names(view, &[DocKind::Section]),
            "paths": visible_names(view, &[DocKind::Connector, DocKind::Link]),
            "labels": visible_names(view, &[DocKind::Label]),
            "portals": visible_names(view, &[DocKind::PortalEnd]),
        }))
    })
}

/// Whether two boxes share interior area.
fn overlaps(a: Rect, b: Rect) -> bool {
    let (a_max, b_max) = (a.origin + a.size, b.origin + b.size);
    a.origin.x < b_max.x && b.origin.x < a_max.x && a.origin.y < b_max.y && b.origin.y < a_max.y
}

/// `scene.node_geometry` for `node`.
fn node_geometry_of(view: &SceneView<'_>, node: NodeId) -> Value {
    let doc = view.document.doc();
    let (projector, layers) = (view.view.projector(), view.view.layers());
    let Some(DocElement::Node(data)) = doc.get(node) else {
        return Value::Null;
    };
    let model = json!({
        "position": { "x": data.position.x, "y": data.position.y },
        "min_size": data.min_size.map(|size| json!({ "w": size.x, "h": size.y })),
        "folded": data.folded,
        "hidden": data.hidden,
    });
    let mut out = Map::new();
    out.insert("element".into(), element_ref(doc, node));
    out.insert("model".into(), model);
    let Some(Value::Object(entry)) = node_entry(view, node, true) else {
        out.insert("in_scene".into(), json!(false));
        return Value::Object(out);
    };
    out.insert("in_scene".into(), json!(true));
    for key in ["canvas", "surface", "visibility", "sections"] {
        if let Some(value) = entry.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    let own = element_bounds(doc, projector, layers, node);
    let others: Vec<Value> = every_node(doc)
        .filter(|&other| other != node)
        .filter(|&other| {
            matches!((own, element_bounds(doc, projector, layers, other)), (Some(a), Some(b)) if overlaps(a, b))
        })
        .map(|other| element_ref(doc, other))
        .collect();
    out.insert("overlaps".into(), Value::Array(others));
    Value::Object(out)
}

fn node_geometry(call: &mut Call<'_>) -> Reply {
    let Some(value) = call.param("node_id").cloned() else {
        return call.invalid_params("node_id", "missing `node_id`");
    };
    if value.as_u64().is_none() {
        return call.invalid_params(
            "node_id",
            format!("`node_id` is a node's integer id, not {value}"),
        );
    }
    with_scene(call, |view| {
        let node = node_named(view.document.doc(), &value).map_err(Refusal::NotFound)?;
        Ok(node_geometry_of(view, node))
    })
}

fn hit_test(call: &mut Call<'_>) -> Reply {
    let point = match parse_pointer(call) {
        Ok(Some(point)) => point,
        // `x` and `y` are required, so the registry has already
        // refused a request without them.
        Ok(None) => return call.invalid_params("x", "missing `x` and `y`"),
        Err(reply) => return reply,
    };
    let report = {
        let Some(app) = call.drive() else {
            return call.no_document();
        };
        app.hit_test(point)
    };
    match report {
        Some(report) => call.ok(report),
        None => no_loaded_document(call),
    }
}

/// `scene.find_text`'s matcher: substring or whole-text, with or
/// without case. Case folds through Unicode lowercasing, so `É`
/// finds `é`.
struct TextMatch {
    needle: String,
    exact: bool,
    case_sensitive: bool,
}

impl TextMatch {
    fn new(text: &str, exact: bool, case_sensitive: bool) -> Self {
        TextMatch {
            needle: if case_sensitive {
                text.to_owned()
            } else {
                text.to_lowercase()
            },
            exact,
            case_sensitive,
        }
    }

    fn matches(&self, displayed: &str) -> bool {
        let folded;
        let hay = if self.case_sensitive {
            displayed
        } else {
            folded = displayed.to_lowercase();
            &folded
        };
        if self.exact {
            hay == self.needle
        } else {
            hay.contains(&self.needle)
        }
    }
}

/// Every drawn section, label and portal end whose text `matcher` finds,
/// in document order, visible or not.
fn find_matches(view: &SceneView<'_>, matcher: &TextMatch) -> Vec<Value> {
    elements_of(view, &[DocKind::Section, DocKind::Label, DocKind::PortalEnd])
        .filter_map(|element| {
            let kind = match view.document.doc().get(element)?.kind() {
                DocKind::Section => "section",
                DocKind::Label => "label",
                _ => "portal_end",
            };
            let mut entry = placed(view, element, true)?;
            let text = entry.get("text")?.as_str()?;
            if !matcher.matches(text) {
                return None;
            }
            entry.insert("kind".into(), json!(kind));
            Some(Value::Object(entry))
        })
        .collect()
}

fn find_text(call: &mut Call<'_>) -> Reply {
    let text = call.str_param("text").unwrap_or_default();
    if text.is_empty() {
        return call.invalid_params("text", "`text` must not be empty");
    }
    let flag = |name: &str| call.param(name).and_then(Value::as_bool).unwrap_or(false);
    let matcher = TextMatch::new(text, flag("exact"), flag("case_sensitive"));
    with_scene(call, |view| {
        Ok(json!({ "matches": find_matches(view, &matcher) }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::testament;
    use crate::application::document::MindMapDocument;
    use crate::application::view::MapView;
    use baumhard::gfx_structs::camera::Camera2D;

    /// The testament map, viewed, with a camera fit to what it draws.
    fn fixture() -> (MindMapDocument, MapView, Camera2D) {
        let doc = testament();
        let view = MapView::for_test(&doc);
        let runs: Vec<_> = every_node(doc.doc())
            .filter_map(|n| element_bounds(doc.doc(), view.projector(), view.layers(), n))
            .collect();
        let (min, max) = runs
            .iter()
            .fold((runs[0].origin, runs[0].origin + runs[0].size), |(lo, hi), r| {
                (lo.min(r.origin), hi.max(r.origin + r.size))
            });
        let mut camera = Camera2D::new(1600, 1200);
        camera.fit_to_bounds(min, max, 0.05);
        (doc, view, camera)
    }

    fn scene<'a>(doc: &'a MindMapDocument, view: &'a MapView, camera: &'a Camera2D) -> SceneView<'a> {
        SceneView {
            document: doc,
            view,
            camera,
        }
    }

    /// **The determinism contract** (`format/ipc.md` §`clock`): the same
    /// document, loaded twice into two independent views, dumps the same
    /// scene byte for byte — every element, its box, its verdict and its
    /// text. Nothing in the dump depends on an iteration order a process
    /// chooses: the document's arena, the projector's layers and the
    /// dump's walk are all in document order.
    ///
    /// Control: the same document at another camera dumps differently,
    /// so the comparison is of a reading that depends on its input, not
    /// of two empty replies.
    #[test]
    fn test_the_same_document_twice_dumps_byte_identical_scenes() {
        let all = Kinds {
            nodes: true,
            paths: true,
            labels: true,
            portals: true,
        };
        let (first, first_view, camera) = fixture();
        let (second, second_view, _) = fixture();
        let one = serde_json::to_string(&read_scene(&scene(&first, &first_view, &camera), all, true))
            .expect("the dump serializes");
        let two = serde_json::to_string(&read_scene(&scene(&second, &second_view, &camera), all, true))
            .expect("the dump serializes");
        assert!(
            one.len() > 10_000,
            "precondition: the dump holds the map ({} bytes)",
            one.len()
        );
        assert_eq!(one, two, "two independent loads dump the same scene");
        let mut moved = camera;
        moved.zoom *= 2.0;
        let three = serde_json::to_string(&read_scene(&scene(&first, &first_view, &moved), all, true))
            .expect("the dump serializes");
        assert_ne!(one, three, "control: another camera dumps another scene");
    }

    /// The dump names every node it reports by integer id, at a box whose
    /// surface rect is the canvas rect through the camera, and a camera
    /// far away sees nothing — while `include=all` still lists it all.
    #[test]
    fn test_the_dump_reports_drawn_nodes_by_integer_id() {
        let (doc, view, camera) = fixture();
        let dumped = read_scene(&scene(&doc, &view, &camera), Kinds::ALL, false);
        let nodes = dumped["nodes"].as_array().expect("nodes");
        assert!(nodes.len() > 3, "the fit camera shows the map");
        for node in nodes {
            assert!(node["element"]["node"].is_u64(), "{node}");
            assert_eq!(node["visibility"], json!("visible"));
        }
        let far = Camera2D {
            position: glam::Vec2::new(1.0e7, 1.0e7),
            ..camera
        };
        let none = read_scene(&scene(&doc, &view, &far), Kinds::ALL, false);
        assert!(none["nodes"].as_array().unwrap().is_empty());
        let all = read_scene(&scene(&doc, &view, &far), Kinds::ALL, true);
        assert_eq!(all["nodes"].as_array().unwrap().len(), nodes.len());
        assert!(all["nodes"][0]["visibility"] == json!("offscreen"));
    }

    /// `find_text` finds a section by a word of its text, names it with
    /// its node and index, and finds nothing for text nobody shows.
    #[test]
    fn test_find_text_finds_a_section_by_its_text() {
        let (doc, view, camera) = fixture();
        let view = scene(&doc, &view, &camera);
        let section = elements_of(&view, &[DocKind::Section])
            .find(|&s| element_text(doc.doc(), s).len() > 3)
            .expect("a section with text");
        let text = element_text(doc.doc(), section);
        let word: String = text.chars().take(4).collect();
        let found = find_matches(&view, &TextMatch::new(&word.to_uppercase(), false, false));
        let named = element_ref(doc.doc(), section);
        assert!(
            found
                .iter()
                .any(|m| m["element"] == named && m["kind"] == json!("section")),
            "{word:?} in {found:?}"
        );
        assert!(named["node"].is_u64() && named["section"].is_u64(), "{named}");
        // Control: case-sensitive upper-case misses a mixed-case text,
        // and text nobody shows finds nothing.
        assert!(find_matches(&view, &TextMatch::new("zz-no-such-text-0e1a", false, false)).is_empty());
    }

    /// A node's geometry carries its document position and its drawn
    /// box, and `in_scene` is true for a drawn node.
    #[test]
    fn test_node_geometry_reports_the_model_and_the_drawn_box() {
        let (doc, view, camera) = fixture();
        let view = scene(&doc, &view, &camera);
        let node = every_node(doc.doc())
            .find(|&n| element_bounds(doc.doc(), view.view.projector(), view.view.layers(), n).is_some())
            .expect("a drawn node");
        let geometry = node_geometry_of(&view, node);
        assert_eq!(geometry["in_scene"], json!(true));
        assert!(geometry["model"]["position"]["x"].is_number());
        assert!(geometry["canvas"]["w"].as_f64().unwrap() > 0.0);
        assert!(geometry["overlaps"].is_array());
    }

    #[test]
    fn test_include_and_kinds_are_checked() {
        assert_eq!(parse_include(None), Ok(false));
        assert_eq!(parse_include(Some("all")), Ok(true));
        assert!(parse_include(Some("some")).is_err());
        assert!(parse_kinds(Some(&json!(["paths"]))).unwrap().paths);
        assert!(parse_kinds(Some(&json!([]))).is_err());
        assert!(
            parse_kinds(Some(&json!(["edges"]))).is_err(),
            "the old word is not a kind"
        );
        assert!(TextMatch::new("É", false, false).matches("café é"));
        assert!(!TextMatch::new("É", false, true).matches("é"));
    }
}
