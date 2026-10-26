// SPDX-License-Identifier: MPL-2.0

//! The app-side half of `scene.hit_test` (`format/ipc.md` §`scene`):
//! what a click at a point would target, read through the canonical
//! click ladder rather than a copy of it.
//!
//! The ladder is private to `app` — `compute_click_hit` resolves node
//! → portal → edge label over the mindmap tree and the scene's role
//! trees, and `dispatch::edge_under_pointer` is its last rung, the
//! connection path under the pointer — so the report is assembled here
//! and handed to the `ipc` side as JSON, the shape
//! [`CommandTarget::shell_state`](crate::application::ipc::CommandTarget::shell_state)
//! already uses. Both callees are the very functions a mouse press, a
//! release and a touch tap run, which is the whole of the parity
//! guarantee: there is no second hit test to drift.
//!
//! Both targets' — each target's command target consumes it over the
//! same trees, which is what keeps `scene.hit_test` one answer. The
//! point resolver beside it ([`resolve_point`], [`surface_of`]) is
//! shared for the same reason: an IPC point lands where the camera
//! says, on either target.

use glam::Vec2;
use serde_json::{json, Value};

use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::tree_builder::{MindMapTree, PortalPart};

use crate::application::document::{f32_json, SelectionState};
use crate::application::scene_host::AppScene;

use crate::application::ipc::injected::Surface;
use crate::application::ipc::space::{Point, Space};
use crate::application::renderer::Renderer;

use super::dispatch::{edge_under_pointer, CanvasPerPixel};
use super::{compute_click_hit, ClickHit, ClickHitParts};

/// `point` in both spaces, `(surface, canvas)`, through `renderer`'s
/// camera as it stands now. One body for both targets' command targets
/// and both targets' injected-input landing: where an IPC point lands
/// is the camera's answer, and the camera is the same type on both.
pub(super) fn resolve_point(renderer: &Renderer, point: Point) -> (Vec2, Vec2) {
    match point.space {
        Space::Surface => (
            Vec2::new(point.x, point.y),
            renderer.screen_to_canvas(point.x, point.y),
        ),
        Space::Canvas => {
            let canvas = Vec2::new(point.x, point.y);
            (renderer.canvas_to_screen(canvas), canvas)
        }
    }
}

/// Where `point` lands on the surface, through `renderer`'s camera as
/// it stands now: how an injected event's position is resolved.
pub(super) fn surface_of(renderer: &Renderer, point: Point) -> Surface {
    let (surface, _) = resolve_point(renderer, point);
    (f64::from(surface.x), f64::from(surface.y))
}

impl ClickHit {
    /// The collapsed hit as `scene.hit_test` reports it: `kind` is the
    /// variant's own name, with its ids beside it. Exhaustive with no
    /// wildcard arm, so a new variant breaks the build here until its
    /// wire form is written into `format/ipc.md`.
    fn to_ipc_json(&self) -> Value {
        match self {
            ClickHit::Empty => json!({ "kind": "Empty" }),
            ClickHit::Node(node_id, section_idx) => {
                json!({ "kind": "Node", "node_id": node_id, "section_idx": section_idx })
            }
            ClickHit::PortalMarker { edge, endpoint } => {
                json!({ "kind": "PortalMarker", "edge": edge, "endpoint_node_id": endpoint })
            }
            ClickHit::PortalText { edge, endpoint } => {
                json!({ "kind": "PortalText", "edge": edge, "endpoint_node_id": endpoint })
            }
            ClickHit::EdgeLabel(edge) => json!({ "kind": "EdgeLabel", "edge": edge }),
        }
    }
}

/// Everything a click at `canvas` would resolve, rung by rung.
///
/// `compute_click_hit` answers the three AABB rungs, each evaluated
/// only when the rungs above it missed — exactly as a press evaluates
/// them — and folds them into the `click_hit` double-click detection
/// and `DoubleClickActivate` route on. The fourth rung, the connection
/// path under the pointer, is a curve test against the model with a
/// tolerance of `EDGE_HIT_TOLERANCE_PX` surface pixels (hence
/// `canvas_per_pixel`), and it is evaluated only when the ladder came
/// up empty — the one case a click consults it.
///
/// `surface` is reported beside `canvas` so the agent sees both
/// spellings of the point it asked about.
///
/// # Costs
///
/// One BVH descent per rung that runs, plus, on an empty ladder, one
/// distance test per edge. Mutates only the trees' memoized subtree
/// AABBs.
pub(super) fn hit_report(
    canvas: Vec2,
    surface: Vec2,
    mindmap_tree: Option<&mut MindMapTree>,
    app_scene: &mut AppScene,
    map: &MindMap,
    canvas_per_pixel: CanvasPerPixel,
) -> Value {
    let ClickHitParts {
        click_hit,
        hit_node,
        hit_section_idx,
        portal_text_hit,
        portal_icon_hit,
        edge_label_hit,
    } = compute_click_hit(canvas, mindmap_tree, app_scene);
    // `compute_click_hit` hands the portal rung back as two views of one
    // `PortalHit`; the part is named through `PortalPart`'s own wire
    // form, the one `scene.dump` reports.
    let portal = match (portal_text_hit, portal_icon_hit) {
        (Some((edge, endpoint)), _) => Some((edge, endpoint, PortalPart::Text)),
        (None, Some((edge, endpoint))) => Some((edge, endpoint, PortalPart::Icon)),
        (None, None) => None,
    }
    .map(|(edge, endpoint, part)| json!({ "edge": edge, "endpoint_node_id": endpoint, "part": part }));
    let edge = match click_hit {
        ClickHit::Empty => match edge_under_pointer(canvas, map, canvas_per_pixel) {
            SelectionState::Edge(edge) => edge.to_ipc_json(),
            _ => Value::Null,
        },
        ClickHit::Node(..)
        | ClickHit::PortalMarker { .. }
        | ClickHit::PortalText { .. }
        | ClickHit::EdgeLabel(_) => Value::Null,
    };
    json!({
        "canvas": { "x": f32_json(canvas.x), "y": f32_json(canvas.y) },
        "surface": { "x": f32_json(surface.x), "y": f32_json(surface.y) },
        "click_hit": click_hit.to_ipc_json(),
        "node": hit_node.map(|node_id| json!({ "node_id": node_id, "section_idx": hit_section_idx })),
        "portal": portal,
        "edge_label": edge_label_hit,
        "edge": edge,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::gfx_structs::camera::Camera2D;
    use baumhard::mindmap::model::DISPLAY_MODE_PORTAL;
    use baumhard::mindmap::scene_cache::SceneConnectionCache;
    use baumhard::mindmap::scene_dump::{
        dump_edges, dump_labels, dump_nodes, dump_portals, DumpFilter, SceneRect,
    };

    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::MindMapDocument;
    use crate::application::scene_host::CanvasRole;

    use super::super::click::{handle_click_core, ClickCore};
    use super::super::InteractionMode;

    fn center(rect: &SceneRect) -> Vec2 {
        Vec2::new(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
    }

    /// The fixture with one unlabeled edge turned into a portal pair,
    /// so every rung of the ladder has something to land on.
    fn fixture() -> MindMapDocument {
        let mut doc = load_test_doc();
        let edge = doc
            .mindmap
            .edges
            .iter_mut()
            .find(|e| e.label.is_none())
            .expect("an unlabeled edge");
        edge.display_mode = Some(DISPLAY_MODE_PORTAL.to_string());
        doc
    }

    /// Points that land on every rung: the center of each drawn
    /// node, label, portal part and connection glyph run, plus empty
    /// canvas far from everything. Read off the projected scene
    /// through the `scene` family's own serializer.
    fn probe_points(doc: &MindMapDocument) -> Vec<Vec2> {
        let (tree, scene) = crate::application::app::project_scene_for_tests(doc, 1.0);
        let camera = Camera2D::new(1280, 720);
        let mut points: Vec<Vec2> = dump_nodes(&tree, &doc.mindmap, &camera, DumpFilter::ALL)
            .iter()
            .map(|n| center(&n.placement.canvas))
            .collect();
        let entry = |role| scene.canvas_entry(role).expect("role registered");
        let labels = entry(CanvasRole::ConnectionLabels);
        points.extend(
            dump_labels(
                labels.tree(),
                labels.offset(),
                &camera,
                scene
                    .edge_channel_index(CanvasRole::ConnectionLabels)
                    .expect("edge role"),
                DumpFilter::ALL,
            )
            .iter()
            .map(|l| center(&l.placement.canvas)),
        );
        let portals = entry(CanvasRole::Portals);
        points.extend(
            dump_portals(
                portals.tree(),
                portals.offset(),
                &camera,
                scene.portal_index(),
                DumpFilter::ALL,
            )
            .iter()
            .map(|p| center(&p.placement.canvas)),
        );
        let edges = entry(CanvasRole::Connections);
        points.extend(
            dump_edges(
                edges.tree(),
                edges.offset(),
                &camera,
                scene
                    .edge_channel_index(CanvasRole::Connections)
                    .expect("edge role"),
                DumpFilter::ALL,
            )
            .iter()
            .map(|e| center(&e.placement.canvas)),
        );
        points.push(Vec2::new(1.0e6, 1.0e6));
        points
    }

    /// **`scene.hit_test` agrees with a real click** at the same
    /// point (#65's dispatch-parity criterion). The click side is the
    /// production path, step for step: the press records
    /// `compute_click_hit`; a release over an edge label selects that
    /// label; any other release runs `handle_click_core`, which
    /// re-runs the portal rung and the connection-path rung itself.
    /// Over every node, label, portal part and connection of the
    /// fixture, the selection the click leaves is the one the report's
    /// ladder names.
    #[test]
    fn test_hit_report_agrees_with_a_real_click_at_every_probe() {
        let doc = fixture();
        let points = probe_points(&doc);
        let unzoomed = || CanvasPerPixel::from_ratio(1.0);
        let mut rungs = std::collections::BTreeSet::new();
        // One projection for every probe: a click names a selection
        // and the tier it owes, and rebuilds nothing itself, so the
        // trees stay what they were for the next probe.
        let (tree, mut scene) = crate::application::app::project_scene_for_tests(&doc, 1.0);
        let mut mindmap_tree = Some(tree);
        let mut cache = SceneConnectionCache::default();
        for canvas in points {
            let report = hit_report(
                canvas,
                canvas,
                mindmap_tree.as_mut(),
                &mut scene,
                &doc.mindmap,
                unzoomed(),
            );

            let mut clicked = fixture();
            let press = compute_click_hit(canvas, mindmap_tree.as_mut(), &mut scene);
            let selection = match press.edge_label_hit {
                Some(edge) => json!({ "kind": "EdgeLabel", "edge": edge }),
                None => {
                    let _ = handle_click_core(
                        press.hit_node,
                        press.hit_section_idx,
                        canvas,
                        unzoomed(),
                        false,
                        ClickCore {
                            document: &mut clicked,
                            mindmap_tree: &mut mindmap_tree,
                            app_scene: &mut scene,
                            scene_cache: &mut cache,
                            interaction_mode: &InteractionMode::Default,
                            now_ms: 0,
                        },
                    );
                    clicked.selection.to_ipc_json()
                }
            };

            let expected = if !report["node"].is_null() {
                rungs.insert("node");
                json!({ "kind": "Single", "node_id": report["node"]["node_id"] })
            } else if !report["portal"].is_null() {
                rungs.insert("portal");
                let kind = match report["portal"]["part"].as_str() {
                    Some("text") => "PortalText",
                    _ => "PortalLabel",
                };
                json!({
                    "kind": kind,
                    "edge": report["portal"]["edge"],
                    "endpoint_node_id": report["portal"]["endpoint_node_id"],
                })
            } else if !report["edge_label"].is_null() {
                rungs.insert("edge_label");
                json!({ "kind": "EdgeLabel", "edge": report["edge_label"] })
            } else if !report["edge"].is_null() {
                rungs.insert("edge");
                json!({ "kind": "Edge", "edge": report["edge"] })
            } else {
                rungs.insert("empty");
                json!({ "kind": "None" })
            };
            assert_eq!(selection, expected, "at {canvas}: report {report}");
        }
        assert_eq!(
            rungs.into_iter().collect::<Vec<_>>(),
            ["edge", "edge_label", "empty", "node", "portal"],
            "the probes must reach every rung, or a rung's parity is untested"
        );
    }

    /// The report's shape: both spellings of the point, the collapsed
    /// hit, and one key per rung, `null` for a rung that missed or
    /// never ran.
    #[test]
    fn test_hit_report_names_every_rung() {
        let doc = load_test_doc();
        let (mut tree, mut scene) = crate::application::app::project_scene_for_tests(&doc, 1.0);
        let far = Vec2::new(1.0e6, 1.0e6);
        let report = hit_report(
            far,
            Vec2::new(3.0, 4.0),
            Some(&mut tree),
            &mut scene,
            &doc.mindmap,
            CanvasPerPixel::from_ratio(1.0),
        );
        assert_eq!(
            report,
            json!({
                "canvas": { "x": 1.0e6, "y": 1.0e6 },
                "surface": { "x": 3.0, "y": 4.0 },
                "click_hit": { "kind": "Empty" },
                "node": null,
                "portal": null,
                "edge_label": null,
                "edge": null,
            })
        );

        let node = dump_nodes(&tree, &doc.mindmap, &Camera2D::new(800, 600), DumpFilter::ALL)
            .into_iter()
            .next()
            .expect("a drawn node");
        let at = center(&node.placement.canvas);
        let report = hit_report(
            at,
            at,
            Some(&mut tree),
            &mut scene,
            &doc.mindmap,
            CanvasPerPixel::from_ratio(1.0),
        );
        assert_eq!(report["click_hit"]["kind"], "Node");
        assert_eq!(report["node"]["node_id"], json!(node.node_id));
        assert_eq!(
            report["edge"],
            Value::Null,
            "the last rung runs only on an empty ladder"
        );
    }
}
