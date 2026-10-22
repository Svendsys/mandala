// SPDX-License-Identifier: MPL-2.0

//! The `scene` family: what is *actually on screen* (`format/ipc.md`
//! §`scene`). Where `query` reads what the application knows, this
//! reads what it draws — positions after layout and drag, sizes after
//! text fitting, the renderer's own cull verdict, and the text a user
//! reads after run coverage — so an agent can answer "is node X on
//! screen, where, overlapping what, saying what?" without pixels.
//!
//! **One source of truth: the drawn trees.** Every command reads the
//! `Tree<GfxElement, GfxMutator>`s the renderer shapes, through
//! Baumhard's serializer ([`baumhard::mindmap::scene_dump`]), with the
//! camera the renderer culls by. Nothing here re-derives geometry from
//! the model or re-implements a visibility rule. `scene.hit_test`
//! routes through the canonical click ladder on the app side
//! ([`CommandTarget::hit_test`](crate::application::ipc::CommandTarget::hit_test))
//! rather than any copy of it.
//!
//! **Read-only.** Four commands reach the app through
//! [`Call::observe`]; `scene.hit_test` needs `drive()` only because a
//! BVH descent memoizes subtree AABBs, and it changes nothing a frame
//! would show. None asks for a frame.

use glam::Vec2;
use serde::Serialize;
use serde_json::{json, Value};

use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use baumhard::mindmap::scene_dump::{
    dump_edges, dump_labels, dump_node, dump_nodes, dump_portals, node_boxes, DumpFilter, EdgeDump,
    LabelDump, NodeDump, Placement, PortalDump,
};
use baumhard::mindmap::tree_builder::PortalPart;

use crate::application::ipc::envelope::Reply;
use crate::application::ipc::registry::{Call, CommandSpec, Family, ParamSpec, ParamType, ResultSpec};
use crate::application::ipc::space::camera_json;
use crate::application::ipc::SceneView;
use crate::application::scene_host::CanvasRole;

use super::act::parse_pointer;
use super::refusal::{no_loaded_document, refuse, Refusal};
use super::sidecar::{parse_to_file, write_sidecar};

/// The element kinds `scene.dump` reports, in reply order.
const KINDS: [&str; 4] = ["nodes", "edges", "labels", "portals"];

const SPACE_DOC: &str = "The coordinate space of x and y: \"canvas\" (default) or \"surface\".";

pub(super) const FAMILY: Family = Family {
    name: "scene",
    commands: &[
        CommandSpec {
            name: "scene.dump",
            summary: "The laid-out scene: every drawn node (with its sections), connection, edge label \
                      and portal marker, with canvas and surface rects, the renderer's visibility \
                      verdict and the displayed text.",
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
                    "Which element kinds to report, as strings: nodes, edges, labels, portals. \
                     Default: all four.",
                ),
                ParamSpec::new(
                    "to_file",
                    ParamType::String,
                    false,
                    "Absolute path. Write the dump there as JSON, atomically, and reply with just \
                     {path, bytes}. For a scene too large to inline.",
                ),
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
                    "Drawn nodes in draw order: {node_id, folded, canvas, surface, visibility, sections}; \
                     each section {section_idx, canvas, surface, visibility, text}.",
                ),
                ResultSpec::new(
                    "edges",
                    ParamType::Array,
                    "Drawn connections: {edge, canvas, surface, visibility, glyphs, visible_glyphs}.",
                ),
                ResultSpec::new(
                    "labels",
                    ParamType::Array,
                    "Drawn edge labels: {edge, canvas, surface, visibility, text}.",
                ),
                ResultSpec::new(
                    "portals",
                    ParamType::Array,
                    "Drawn portal icons and captions: {edge, endpoint_node_id, part, canvas, surface, \
                     visibility, text}.",
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
            run: dump,
        },
        CommandSpec {
            name: "scene.visible_set",
            summary: "Just the ids of what the camera shows right now, by kind. No text is resolved and \
                      no rect is reported.",
            params: &[],
            result: &[
                ResultSpec::new("nodes", ParamType::Array, "Visible node ids, in draw order."),
                ResultSpec::new(
                    "sections",
                    ParamType::Array,
                    "Visible sections, as {node_id, section_idx}.",
                ),
                ResultSpec::new(
                    "edges",
                    ParamType::Array,
                    "Visible connections, as edge identities.",
                ),
                ResultSpec::new(
                    "labels",
                    ParamType::Array,
                    "Visible edge labels, as edge identities.",
                ),
                ResultSpec::new(
                    "portals",
                    ParamType::Array,
                    "Visible portal parts, as {edge, endpoint_node_id, part}.",
                ),
            ],
            run: visible_set,
        },
        CommandSpec {
            name: "scene.node_geometry",
            summary: "One node's whole geometry story: its model position and size, whether the \
                      scene draws it (and which fold hides it when not), its laid-out rects per \
                      section, its visibility, and the drawn nodes it overlaps.",
            params: &[ParamSpec::new(
                "node_id",
                ParamType::String,
                true,
                "The node to describe.",
            )],
            result: &[
                ResultSpec::new("node_id", ParamType::String, "The node described."),
                ResultSpec::new(
                    "model",
                    ParamType::Object,
                    "{position: {x, y}, size: {width, height}, folded} as the model holds them.",
                ),
                ResultSpec::new(
                    "in_scene",
                    ParamType::Boolean,
                    "Whether the scene draws the node. When false, absent_reason says why.",
                ),
                ResultSpec::new(
                    "hidden_by",
                    ParamType::String,
                    "The folded ancestor that hides the node, or null.",
                ),
                ResultSpec::new(
                    "absent_reason",
                    ParamType::String,
                    "Why the scene does not draw the node: \"folded\" (hidden_by names the fold) or \
                     \"not_built\" (no scene built yet, or the builder placed no container for it); \
                     null when in_scene.",
                ),
                ResultSpec::new(
                    "canvas",
                    ParamType::Object,
                    "In the scene only: the laid-out box, {x, y, w, h} in canvas units.",
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
                    "In the scene only: {section_idx, canvas, surface, visibility, text} per drawn section.",
                ),
                ResultSpec::new(
                    "overlaps",
                    ParamType::Array,
                    "In the scene only: the ids of the other drawn nodes whose boxes overlap this one.",
                ),
            ],
            run: node_geometry,
        },
        CommandSpec {
            name: "scene.hit_test",
            summary: "What a click at a point would target: the canonical click ladder (node, portal, \
                      edge label) and its last rung (the connection path), rung by rung.",
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
                    "The collapsed hit a press records and DoubleClickActivate routes on: kind Node \
                     (node_id, section_idx), PortalMarker or PortalText (edge, endpoint_node_id), \
                     EdgeLabel (edge) or Empty.",
                ),
                ResultSpec::new(
                    "node",
                    ParamType::Object,
                    "The node rung: {node_id, section_idx}, or null.",
                ),
                ResultSpec::new(
                    "portal",
                    ParamType::Object,
                    "The portal rung, tried only when no node was hit: {edge, endpoint_node_id, part}, \
                     or null.",
                ),
                ResultSpec::new(
                    "edge_label",
                    ParamType::Object,
                    "The edge-label rung, tried only when no node or portal was hit: the edge, or null.",
                ),
                ResultSpec::new(
                    "edge",
                    ParamType::Object,
                    "The last rung, tried only when the ladder came up empty: the connection path \
                     under the point, or null.",
                ),
            ],
            run: hit_test,
        },
        CommandSpec {
            name: "scene.find_text",
            summary: "Locate drawn sections, edge labels and portal captions (never icons) by the text \
                      they display.",
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
                "Every drawn element whose displayed text matches, in draw order, visible or not: \
                 {kind: section, node_id, section_idx | kind: label, edge | kind: portal, edge, \
                 endpoint_node_id, part}, each with text, canvas, surface and visibility.",
            )],
            run: find_text,
        },
    ],
};

/// Which element kinds a read covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Kinds {
    nodes: bool,
    edges: bool,
    labels: bool,
    portals: bool,
}

impl Kinds {
    const ALL: Kinds = Kinds {
        nodes: true,
        edges: true,
        labels: true,
        portals: true,
    };
}

/// The drawn elements of the asked-for kinds that a filter keeps,
/// read off the trees at the view's camera. A kind not asked for is
/// `None`, and its trees are not walked.
#[derive(Debug, Serialize)]
struct Drawn {
    #[serde(skip_serializing_if = "Option::is_none")]
    nodes: Option<Vec<NodeDump>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    edges: Option<Vec<EdgeDump>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    labels: Option<Vec<LabelDump>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    portals: Option<Vec<PortalDump>>,
}

/// A role's registered tree and its scene offset, or `None` when the
/// role has no tree or its tree is registered hidden — in which case
/// the renderer draws none of it, and the scene reports none of it.
fn drawn_role<'v>(
    view: &SceneView<'v>,
    role: CanvasRole,
) -> Option<(&'v Tree<GfxElement, GfxMutator>, Vec2)> {
    let entry = view.app_scene.canvas_entry(role)?;
    entry.visible().then(|| (entry.tree(), entry.offset()))
}

/// Read the scene: one serializer call per asked-for kind, each over
/// the tree that kind is drawn from, named through that tree's own
/// identity index, and filtered by `filter` before any text is
/// resolved.
fn read_drawn(view: &SceneView<'_>, kinds: Kinds, filter: DumpFilter) -> Drawn {
    let camera = view.camera;
    let map = &view.document.mindmap;
    let edge_role = |role: CanvasRole| {
        let (tree, offset) = drawn_role(view, role)?;
        Some((tree, offset, view.app_scene.edge_channel_index(role)?))
    };
    Drawn {
        nodes: kinds.nodes.then(|| {
            view.mindmap_tree
                .map(|tree| dump_nodes(tree, map, camera, filter))
                .unwrap_or_default()
        }),
        edges: kinds.edges.then(|| {
            edge_role(CanvasRole::Connections)
                .map(|(tree, offset, index)| dump_edges(tree, offset, camera, index, filter))
                .unwrap_or_default()
        }),
        labels: kinds.labels.then(|| {
            edge_role(CanvasRole::ConnectionLabels)
                .map(|(tree, offset, index)| dump_labels(tree, offset, camera, index, filter))
                .unwrap_or_default()
        }),
        portals: kinds.portals.then(|| {
            drawn_role(view, CanvasRole::Portals)
                .map(|(tree, offset)| {
                    dump_portals(tree, offset, camera, view.app_scene.portal_index(), filter)
                })
                .unwrap_or_default()
        }),
    }
}

/// `scene.dump`'s reply: the camera beside the drawn elements. A
/// `Serialize` view over both, so `to_file` streams it into the file
/// rather than building a value tree of the whole scene first.
#[derive(Serialize)]
struct DumpReply<'a> {
    camera: Value,
    #[serde(flatten)]
    drawn: &'a Drawn,
}

/// Run `read` against the drawn scene, or answer `no_document`: with
/// no app yet in the registry's words, with an app but no document in
/// the read families' shared words.
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

/// `scene.dump`'s `include`, checked, as the filter it asks for.
fn parse_include(include: Option<&str>) -> Result<DumpFilter, Refusal> {
    match include {
        None | Some("visible") => Ok(DumpFilter::VISIBLE),
        Some("all") => Ok(DumpFilter::ALL),
        Some(other) => Err(Refusal::Invalid(
            "include",
            format!("`include` must be \"visible\" or \"all\", got \"{other}\""),
        )),
    }
}

/// `scene.dump`'s `kinds`, checked: at least one, each a known kind.
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
        edges: false,
        labels: false,
        portals: false,
    };
    for word in words {
        let flag = match word.as_str() {
            Some("nodes") => &mut parsed.nodes,
            Some("edges") => &mut parsed.edges,
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
    let request = parse_include(call.str_param("include")).and_then(|include| {
        Ok((
            include,
            parse_kinds(call.param("kinds"))?,
            parse_to_file(call.param("to_file"), call.param("overwrite"))?,
        ))
    });
    let (filter, kinds, to_file) = match request {
        Ok(request) => request,
        Err(refusal) => return refuse(call, refusal),
    };
    with_scene(call, |view| {
        let drawn = read_drawn(view, kinds, filter);
        let reply = DumpReply {
            camera: camera_json(view.camera),
            drawn: &drawn,
        };
        match &to_file {
            None => serde_json::to_value(&reply)
                .map_err(|e| Refusal::Internal(format!("could not serialize the scene: {e}"))),
            Some(target) => write_sidecar(target, &reply),
        }
    })
}

fn visible_set(call: &mut Call<'_>) -> Reply {
    with_scene(call, |view| {
        let drawn = read_drawn(view, Kinds::ALL, DumpFilter::VISIBLE_IDS);
        let nodes = drawn.nodes.unwrap_or_default();
        let sections: Vec<Value> = nodes
            .iter()
            .flat_map(|node| {
                node.sections
                    .iter()
                    .map(|s| json!({ "node_id": node.node_id, "section_idx": s.section_idx }))
            })
            .collect();
        Ok(json!({
            "nodes": nodes.iter().map(|n| &n.node_id).collect::<Vec<_>>(),
            "sections": sections,
            "edges": drawn.edges.unwrap_or_default().into_iter().map(|e| e.edge).collect::<Vec<_>>(),
            "labels": drawn.labels.unwrap_or_default().into_iter().map(|l| l.edge).collect::<Vec<_>>(),
            "portals": drawn
                .portals
                .unwrap_or_default()
                .into_iter()
                .map(|p| json!({ "edge": p.edge, "endpoint_node_id": p.endpoint_node_id, "part": p.part }))
                .collect::<Vec<_>>(),
        }))
    })
}

/// `{canvas, surface, visibility}` beside `entry`'s own keys: the flat
/// shape every scene element takes on the wire.
fn placed(mut entry: Value, placement: &Placement) -> Value {
    if let (Some(object), Ok(Value::Object(placed))) =
        (entry.as_object_mut(), serde_json::to_value(placement))
    {
        object.extend(placed);
    }
    entry
}

fn node_geometry(call: &mut Call<'_>) -> Reply {
    let Some(node_id) = call.str_param("node_id").map(str::to_owned) else {
        return call.invalid_params("node_id", "missing `node_id`");
    };
    with_scene(call, |view| {
        let map = &view.document.mindmap;
        let node = map
            .nodes
            .get(&node_id)
            .ok_or_else(|| Refusal::NotFound(format!("the document has no node `{node_id}`")))?;
        let mut result = json!({
            "node_id": node_id,
            "model": {
                "position": { "x": node.position.x, "y": node.position.y },
                "size": { "width": node.size.width, "height": node.size.height },
                "folded": node.folded,
            },
            "hidden_by": map.folding_ancestor(node).map(|ancestor| ancestor.id.as_str()),
        });
        let drawn = view
            .mindmap_tree
            .and_then(|tree| Some((tree, dump_node(tree, map, view.camera, &node_id)?)));
        let Some((tree, dumped)) = drawn else {
            // A node the model holds but the scene does not draw is
            // either behind a fold, or not built: no tree yet (before
            // the first scene build), or the builder placed no
            // container for it.
            let reason = if result["hidden_by"].is_null() {
                "not_built"
            } else {
                "folded"
            };
            result["in_scene"] = json!(false);
            result["absent_reason"] = json!(reason);
            return Ok(result);
        };
        result["absent_reason"] = Value::Null;
        let overlaps: Vec<&str> = node_boxes(tree, view.camera)
            .filter(|(other, placed)| *other != node_id && placed.canvas.overlaps(&dumped.placement.canvas))
            .map(|(other, _)| other)
            .collect();
        result["in_scene"] = json!(true);
        result["sections"] = serde_json::to_value(&dumped.sections)
            .map_err(|e| Refusal::Internal(format!("could not serialize the node: {e}")))?;
        result["overlaps"] = json!(overlaps);
        Ok(placed(result, &dumped.placement))
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

fn find_text(call: &mut Call<'_>) -> Reply {
    let text = call.str_param("text").unwrap_or_default();
    if text.is_empty() {
        return call.invalid_params("text", "`text` must not be empty");
    }
    let flag = |name: &str| call.param(name).and_then(Value::as_bool).unwrap_or(false);
    let matcher = TextMatch::new(text, flag("exact"), flag("case_sensitive"));
    with_scene(call, |view| {
        let drawn = read_drawn(
            view,
            Kinds {
                edges: false,
                ..Kinds::ALL
            },
            DumpFilter::ALL,
        );
        let shown = |text: &Option<String>| text.as_deref().is_some_and(|t| matcher.matches(t));
        let mut matches: Vec<Value> = Vec::new();
        for node in drawn.nodes.unwrap_or_default() {
            for section in node.sections.iter().filter(|s| shown(&s.text)) {
                matches.push(placed(
                    json!({
                        "kind": "section",
                        "node_id": node.node_id,
                        "section_idx": section.section_idx,
                        "text": section.text,
                    }),
                    &section.placement,
                ));
            }
        }
        for label in drawn
            .labels
            .unwrap_or_default()
            .into_iter()
            .filter(|l| shown(&l.text))
        {
            matches.push(placed(
                json!({ "kind": "label", "edge": label.edge, "text": label.text }),
                &label.placement,
            ));
        }
        for portal in drawn
            .portals
            .unwrap_or_default()
            .into_iter()
            // Captions only: an icon is a marker glyph, not text a
            // user reads.
            .filter(|p| p.part == PortalPart::Text && shown(&p.text))
        {
            matches.push(placed(
                json!({
                    "kind": "portal",
                    "edge": portal.edge,
                    "endpoint_node_id": portal.endpoint_node_id,
                    "part": portal.part,
                    "text": portal.text,
                }),
                &portal.placement,
            ));
        }
        Ok(json!({ "matches": matches }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::gfx_structs::camera::Camera2D;
    use baumhard::mindmap::tree_builder::MindMapTree;

    use crate::application::console::LineOutcome;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::MindMapDocument;
    use crate::application::ipc::registry::tests::{run_attached, run_detached};
    use crate::application::ipc::space::{Point, Space};
    use crate::application::ipc::{CommandTarget, ShellState};
    use crate::application::keybinds::Action;
    use crate::application::macros::MacroStep;
    use crate::application::scene_host::AppScene;

    use super::super::sidecar::tests::{names_in, scratch_dir};

    /// A real document and the real scene projected from it behind the
    /// `CommandTarget` seam: the trees `CanvasFrame` builds, with no
    /// renderer (TEST_CONVENTIONS §T8). Every act method fails the test
    /// that reaches it — the `scene` family reads. The hit test answers
    /// a recognizable stand-in and records the point it was asked
    /// about: the real ladder is pinned against a real click where it
    /// lives (`app/ipc_scene.rs`).
    struct SceneApp {
        doc: Option<MindMapDocument>,
        tree: Option<MindMapTree>,
        scene: AppScene,
        camera: Camera2D,
        probed: Option<Point>,
    }

    impl SceneApp {
        /// The fixture, with the camera centered on its first node.
        fn with_fixture() -> Self {
            Self::with_doc(load_test_doc())
        }

        /// `doc` projected, with the camera centered on its first node.
        fn with_doc(doc: MindMapDocument) -> Self {
            let (tree, scene) = crate::application::app::project_scene_for_tests(&doc, 1.0);
            let mut camera = Camera2D::new(1280, 720);
            let first = first_node(&doc);
            let node = &doc.mindmap.nodes[&first];
            camera.position = Vec2::new(
                (node.position.x + node.size.width / 2.0) as f32,
                (node.position.y + node.size.height / 2.0) as f32,
            );
            SceneApp {
                doc: Some(doc),
                tree: Some(tree),
                scene,
                camera,
                probed: None,
            }
        }
    }

    fn first_node(doc: &MindMapDocument) -> String {
        doc.mindmap.nodes.keys().min().cloned().expect("fixture nodes")
    }

    impl CommandTarget for SceneApp {
        fn dispatch_action(&mut self, _: Action, _: Option<Point>) -> bool {
            panic!("a scene read must not dispatch an Action");
        }
        fn run_console_line(&mut self, _: &str) -> Option<LineOutcome> {
            panic!("a scene read must not run a console line");
        }
        fn has_macro(&self, _: &str) -> bool {
            panic!("a scene read must not look up a macro");
        }
        fn run_macro(&mut self, _: &str) -> bool {
            panic!("a scene read must not run a macro");
        }
        fn run_macro_steps(&mut self, _: &[MacroStep]) -> bool {
            panic!("a scene read must not run macro steps");
        }
        fn changed_view(&self) -> bool {
            false
        }
        fn document(&self) -> Option<&MindMapDocument> {
            self.doc.as_ref()
        }
        fn animation_pause(&self) -> Option<u64> {
            None
        }
        fn shell_state(&self, _: ShellState) -> Value {
            panic!("a scene read must not read the shell state");
        }
        fn scene(&self) -> Option<SceneView<'_>> {
            Some(SceneView {
                document: self.doc.as_ref()?,
                mindmap_tree: self.tree.as_ref(),
                app_scene: &self.scene,
                camera: &self.camera,
            })
        }
        fn hit_test(&mut self, point: Point) -> Option<Value> {
            self.doc.as_ref()?;
            self.probed = Some(point);
            Some(json!({ "stand_in": true }))
        }
    }

    const ALL_SCENE_COMMANDS: [&str; 5] = [
        r#"{"id":1,"cmd":"scene.dump"}"#,
        r#"{"id":1,"cmd":"scene.visible_set"}"#,
        r#"{"id":1,"cmd":"scene.node_geometry","node_id":"1"}"#,
        r#"{"id":1,"cmd":"scene.hit_test","x":0,"y":0}"#,
        r#"{"id":1,"cmd":"scene.find_text","text":"a"}"#,
    ];

    fn result(frame: &str, app: &mut SceneApp) -> Value {
        let (reply, repaint) = run_attached(frame, app);
        assert!(!repaint, "{frame}: a scene read never asks for a frame");
        assert_eq!(reply["ok"], json!(true), "{frame}: {reply}");
        reply["result"].clone()
    }

    #[test]
    fn test_every_scene_command_answers_no_document_without_one() {
        for frame in ALL_SCENE_COMMANDS {
            let (reply, repaint) = run_detached(frame);
            assert_eq!(reply["error"]["code"], json!("no_document"), "{frame}");
            assert!(!repaint, "{frame}");
        }
        let mut app = SceneApp::with_fixture();
        app.doc = None;
        for frame in ALL_SCENE_COMMANDS {
            let (reply, _) = run_attached(frame, &mut app);
            assert_eq!(reply["error"]["code"], json!("no_document"), "{frame}");
            let message = reply["error"]["message"].as_str().expect("message");
            assert!(message.contains("open <path>"), "{frame}: {message}");
        }
    }

    /// Reading the scene leaves the document as it was: no dirty flag,
    /// no undo entry, no selection change, and no frame asked for.
    #[test]
    fn test_every_scene_command_leaves_the_document_untouched() {
        let mut app = SceneApp::with_fixture();
        let before = app
            .doc
            .as_ref()
            .map(|d| (d.dirty, d.undo_stack.len(), d.selection.to_ipc_json()));
        for frame in ALL_SCENE_COMMANDS {
            let (_, repaint) = run_attached(frame, &mut app);
            assert!(!repaint, "{frame}");
        }
        let after = app
            .doc
            .as_ref()
            .map(|d| (d.dirty, d.undo_stack.len(), d.selection.to_ipc_json()));
        assert_eq!(before, after);
    }

    /// The default dump is what the camera shows: every element in it
    /// is `visible`, the camera it was taken at rides along, and
    /// `include: "all"` is a superset that also carries the culled.
    #[test]
    fn test_dump_defaults_to_the_visible_scene_and_all_adds_the_culled() {
        let mut app = SceneApp::with_fixture();
        let visible = result(r#"{"id":1,"cmd":"scene.dump"}"#, &mut app);
        assert_eq!(visible["camera"]["zoom"], json!(1.0));
        let all = result(r#"{"id":1,"cmd":"scene.dump","include":"all"}"#, &mut app);
        for kind in KINDS {
            let shown = visible[kind].as_array().expect("an array per kind");
            let drawn = all[kind].as_array().expect("an array per kind");
            assert!(shown.len() <= drawn.len(), "{kind}");
            for element in shown {
                let section_shown = element["sections"].as_array().is_some_and(|s| !s.is_empty());
                assert!(
                    element["visibility"] == "visible" || section_shown,
                    "{kind}: {element}"
                );
            }
        }
        let nodes = visible["nodes"].as_array().expect("nodes");
        assert!(!nodes.is_empty(), "the camera is centered on a node");
        let drawn_nodes = all["nodes"].as_array().expect("nodes");
        assert_eq!(drawn_nodes.len(), 252, "every testament node is drawn");
        assert!(
            drawn_nodes.iter().any(|n| n["visibility"] == "offscreen"),
            "a 1280×720 window does not show the whole map"
        );
        assert!(!all["edges"].as_array().expect("edges").is_empty());
        assert!(!all["labels"].as_array().expect("labels").is_empty());
    }

    #[test]
    fn test_dump_kinds_narrow_the_reply_and_bad_words_are_refused() {
        let mut app = SceneApp::with_fixture();
        let only = result(r#"{"id":1,"cmd":"scene.dump","kinds":["labels"]}"#, &mut app);
        let keys: Vec<&String> = only.as_object().expect("object").keys().collect();
        assert_eq!(keys, ["camera", "labels"]);
        for (frame, param) in [
            (r#"{"id":1,"cmd":"scene.dump","kinds":[]}"#, "kinds"),
            (
                r#"{"id":1,"cmd":"scene.dump","kinds":["nodes","arrows"]}"#,
                "kinds",
            ),
            (r#"{"id":1,"cmd":"scene.dump","include":"some"}"#, "include"),
            (r#"{"id":1,"cmd":"scene.dump","overwrite":true}"#, "overwrite"),
            (
                r#"{"id":1,"cmd":"scene.dump","to_file":"relative.json"}"#,
                "to_file",
            ),
        ] {
            let (reply, _) = run_attached(frame, &mut app);
            assert_eq!(reply["error"]["code"], json!("invalid_params"), "{frame}");
            assert_eq!(reply["error"]["data"]["param"], json!(param), "{frame}");
        }
    }

    /// `to_file` writes exactly the dump the inline reply carries, and
    /// replies with only where and how much.
    #[test]
    fn test_dump_to_file_writes_the_inline_dump() {
        let mut app = SceneApp::with_fixture();
        let dir = scratch_dir("scene");
        let path = dir.join("scene.json");
        let frame = json!({ "id": 1, "cmd": "scene.dump", "include": "all", "to_file": path }).to_string();
        let reply = result(&frame, &mut app);
        let on_disk = std::fs::read(&path).expect("written");
        assert_eq!(reply, json!({ "path": path, "bytes": on_disk.len() }));
        let inline = result(r#"{"id":1,"cmd":"scene.dump","include":"all"}"#, &mut app);
        assert_eq!(serde_json::from_slice::<Value>(&on_disk).expect("JSON"), inline);
        assert_eq!(names_in(&dir), ["scene.json"]);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// The visible set is the default dump's ids, kind by kind.
    #[test]
    fn test_visible_set_is_the_ids_of_the_visible_dump() {
        let mut app = SceneApp::with_fixture();
        let set = result(r#"{"id":1,"cmd":"scene.visible_set"}"#, &mut app);
        let dump = result(r#"{"id":1,"cmd":"scene.dump"}"#, &mut app);
        let ids = |kind: &str, key: &str| -> Vec<Value> {
            dump[kind]
                .as_array()
                .expect("array")
                .iter()
                .map(|e| e[key].clone())
                .collect()
        };
        assert_eq!(set["nodes"].as_array().expect("nodes"), &ids("nodes", "node_id"));
        assert_eq!(set["edges"].as_array().expect("edges"), &ids("edges", "edge"));
        assert_eq!(set["labels"].as_array().expect("labels"), &ids("labels", "edge"));
        let sections: usize = dump["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .map(|n| n["sections"].as_array().expect("sections").len())
            .sum();
        assert_eq!(set["sections"].as_array().expect("sections").len(), sections);
        assert!(!set["nodes"].as_array().expect("nodes").is_empty());
    }

    #[test]
    fn test_node_geometry_tells_the_whole_story_of_a_drawn_node() {
        let mut app = SceneApp::with_fixture();
        let id = first_node(app.doc.as_ref().expect("doc"));
        let frame = json!({ "id": 1, "cmd": "scene.node_geometry", "node_id": id }).to_string();
        let story = result(&frame, &mut app);
        assert_eq!(story["node_id"], json!(id));
        assert_eq!(story["in_scene"], json!(true));
        assert_eq!(story["hidden_by"], Value::Null);
        assert_eq!(story["visibility"], "visible", "the camera is centered on it");
        let model = &app.doc.as_ref().expect("doc").mindmap.nodes[&id];
        assert_eq!(story["model"]["position"]["x"], json!(model.position.x));
        assert_eq!(story["model"]["folded"], json!(model.folded));
        assert!(!story["sections"].as_array().expect("sections").is_empty());
        let all = result(
            r#"{"id":1,"cmd":"scene.dump","include":"all","kinds":["nodes"]}"#,
            &mut app,
        );
        let me = all["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .find(|n| n["node_id"] == json!(id))
            .expect("in the dump")
            .clone();
        assert_eq!(story["canvas"], me["canvas"]);
        assert_eq!(story["surface"], me["surface"]);
        for other in story["overlaps"].as_array().expect("overlaps") {
            assert_ne!(other, &json!(id), "a node does not overlap itself");
        }

        let (reply, _) = run_attached(
            r#"{"id":1,"cmd":"scene.node_geometry","node_id":"no-such-node"}"#,
            &mut app,
        );
        assert_eq!(reply["error"]["code"], json!("not_found"));
    }

    /// A node a fold hides is in the model and not in the scene, and
    /// the story names the fold.
    #[test]
    fn test_node_geometry_names_the_fold_that_hides_a_node() {
        let mut app = SceneApp::with_fixture();
        let doc = app.doc.as_mut().expect("doc");
        let mut parents: Vec<String> = doc
            .mindmap
            .nodes
            .values()
            .filter_map(|n| n.parent_id.clone())
            .collect();
        parents.sort();
        let parent = parents[0].clone();
        doc.mindmap.nodes.get_mut(&parent).expect("parent").folded = true;
        let child = doc.mindmap.all_descendants(&parent)[0].clone();
        app.tree = Some(app.doc.as_ref().expect("doc").build_tree());
        let frame = json!({ "id": 1, "cmd": "scene.node_geometry", "node_id": child }).to_string();
        let story = result(&frame, &mut app);
        assert_eq!(story["in_scene"], json!(false));
        assert_eq!(story["hidden_by"], json!(parent));
        assert_eq!(story["absent_reason"], json!("folded"));
        assert!(story.get("canvas").is_none() && story.get("sections").is_none());
    }

    /// `hit_test` validates its point before it reaches the app, and
    /// hands the app the point in the space it was asked in.
    #[test]
    fn test_hit_test_checks_the_point_and_hands_it_over() {
        for (frame, param) in [
            (r#"{"id":1,"cmd":"scene.hit_test","x":1}"#, "y"),
            (
                r#"{"id":1,"cmd":"scene.hit_test","x":1,"y":2,"space":"screen"}"#,
                "space",
            ),
            (r#"{"id":1,"cmd":"scene.hit_test","x":1e300,"y":2}"#, "x"),
        ] {
            let (reply, _) = run_detached(frame);
            assert_eq!(reply["error"]["code"], json!("invalid_params"), "{frame}");
            assert_eq!(reply["error"]["data"]["param"], json!(param), "{frame}");
        }
        let mut app = SceneApp::with_fixture();
        let reply = result(
            r#"{"id":1,"cmd":"scene.hit_test","x":12.5,"y":-3,"space":"surface"}"#,
            &mut app,
        );
        assert_eq!(reply, json!({ "stand_in": true }));
        assert_eq!(
            app.probed,
            Some(Point {
                x: 12.5,
                y: -3.0,
                space: Space::Surface
            })
        );
    }

    #[test]
    fn test_find_text_locates_drawn_text_by_substring_case_and_whole() {
        let mut app = SceneApp::with_fixture();
        let all = result(
            r#"{"id":1,"cmd":"scene.dump","include":"all","kinds":["nodes"]}"#,
            &mut app,
        );
        let (node_id, section) = all["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .find_map(|n| {
                let s = n["sections"].as_array()?.iter().find(|s| {
                    s["text"]
                        .as_str()
                        .is_some_and(|t| t.chars().count() > 4 && t.to_lowercase() != t.to_uppercase())
                })?;
                Some((n["node_id"].clone(), s.clone()))
            })
            .expect("a section with cased text");
        let text = section["text"].as_str().expect("text");

        let find = |app: &mut SceneApp, request: Value| -> Vec<Value> {
            result(&request.to_string(), app)["matches"]
                .as_array()
                .expect("matches")
                .clone()
        };
        let hit = |m: &Value| {
            m["kind"] == "section" && m["node_id"] == node_id && m["section_idx"] == section["section_idx"]
        };

        let whole = find(
            &mut app,
            json!({ "id": 1, "cmd": "scene.find_text", "text": text, "exact": true }),
        );
        let found = whole.iter().find(|m| hit(m)).expect("found by its whole text");
        assert_eq!(found["canvas"], section["canvas"]);
        assert_eq!(found["visibility"], section["visibility"]);

        let upper = text.to_uppercase();
        let folded = find(
            &mut app,
            json!({ "id": 1, "cmd": "scene.find_text", "text": upper }),
        );
        assert!(folded.iter().any(hit), "case-insensitive by default");
        if upper != text {
            let strict = find(
                &mut app,
                json!({ "id": 1, "cmd": "scene.find_text", "text": upper, "case_sensitive": true, "exact": true }),
            );
            assert!(!strict.iter().any(hit), "case_sensitive honors case");
        }
        let fragment: String = text.chars().skip(1).take(3).collect();
        let partial = find(
            &mut app,
            json!({ "id": 1, "cmd": "scene.find_text", "text": fragment }),
        );
        assert!(partial.iter().any(hit), "a substring finds it");
        let partial_exact = find(
            &mut app,
            json!({ "id": 1, "cmd": "scene.find_text", "text": fragment, "exact": true }),
        );
        assert!(!partial_exact.iter().any(hit), "exact wants the whole text");

        let (reply, _) = run_attached(r#"{"id":1,"cmd":"scene.find_text","text":""}"#, &mut app);
        assert_eq!(reply["error"]["data"]["param"], json!("text"));
    }

    /// Before the first scene build there is no tree, and a node the
    /// model holds is absent for that reason, not for a fold.
    #[test]
    fn test_node_geometry_says_when_the_scene_is_not_built() {
        let mut app = SceneApp::with_fixture();
        app.tree = None;
        let id = first_node(app.doc.as_ref().expect("doc"));
        let frame = json!({ "id": 1, "cmd": "scene.node_geometry", "node_id": id }).to_string();
        let story = result(&frame, &mut app);
        assert_eq!(story["in_scene"], json!(false));
        assert_eq!(story["hidden_by"], Value::Null);
        assert_eq!(story["absent_reason"], json!("not_built"));
        let mut drawn = SceneApp::with_fixture();
        let story = result(&frame, &mut drawn);
        assert_eq!(
            story["absent_reason"],
            Value::Null,
            "a drawn node is absent for no reason"
        );
    }

    /// A portal's icon is a marker glyph, not text: searching for it
    /// finds no portal, though the dump reports the icon with that
    /// very glyph.
    #[test]
    fn test_find_text_never_matches_a_portal_icon() {
        use baumhard::mindmap::model::DISPLAY_MODE_PORTAL;

        let mut doc = load_test_doc();
        doc.mindmap
            .edges
            .iter_mut()
            .find(|e| e.label.is_none())
            .expect("an unlabeled edge")
            .display_mode = Some(DISPLAY_MODE_PORTAL.to_string());
        let mut app = SceneApp::with_doc(doc);
        let all = result(
            r#"{"id":1,"cmd":"scene.dump","include":"all","kinds":["portals"]}"#,
            &mut app,
        );
        let icon = all["portals"]
            .as_array()
            .expect("portals")
            .iter()
            .find(|p| p["part"] == "icon")
            .expect("the portal draws an icon")
            .clone();
        let glyph = icon["text"].as_str().expect("the icon's glyph");
        let frame = json!({ "id": 1, "cmd": "scene.find_text", "text": glyph }).to_string();
        let matches = result(&frame, &mut app)["matches"]
            .as_array()
            .expect("matches")
            .clone();
        assert!(
            matches.iter().all(|m| m["kind"] != "portal"),
            "an icon is not text: {matches:?}"
        );
    }

    /// Rects carry the shortest decimal of each `f32` on the inline
    /// path, the digits a streamed sidecar writes.
    #[test]
    fn test_scene_coordinates_are_shortest_decimals() {
        let mut app = SceneApp::with_fixture();
        let dump = result(r#"{"id":1,"cmd":"scene.dump","kinds":["nodes"]}"#, &mut app);
        let node = &dump["nodes"][0];
        for key in ["x", "y", "w", "h"] {
            let number = node["canvas"][key].as_f64().expect("a number");
            assert_eq!(
                number.to_string(),
                (number as f32).to_string(),
                "{key} carries the f32's own digits"
            );
        }
    }
}
