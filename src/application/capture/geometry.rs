// SPDX-License-Identifier: MPL-2.0

//! The geometry sidecar: the pixels ↔ ids map written next to a
//! screenshot (`<path>.geometry.json`, `format/ipc.md`
//! §`capture.screenshot`), so an agent can point at what it sees by
//! arithmetic instead of by guessing.
//!
//! **It describes the image, not the document.** Each node and section
//! is read off the drawn node tree through Baumhard's scene serializer
//! ([`dump_nodes`]) at the camera the image was drawn through, so its
//! rects are in the image's own pixels and its membership is the
//! renderer's cull verdict at that camera: a node a fold hides, a zoom
//! window suppresses, or the framing leaves outside is absent, exactly
//! as it is absent from the pixels. Everything else about the scene is
//! `scene.dump`'s.

use baumhard::gfx_structs::camera::Camera2D;
use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::scene_dump::{dump_nodes, DumpFilter, SceneRect};
use baumhard::mindmap::tree_builder::MindMapTree;
use baumhard::util::shortest_float::serialize_shortest;
use serde::Serialize;

/// The whole sidecar.
#[derive(Debug, Serialize)]
pub(crate) struct Sidecar {
    pub(crate) protocol: u32,
    /// The document's revision when the image was drawn
    /// (`MindMapDocument::revision`): what ties these rects to the
    /// model state they were derived from, and to a `document_changed`
    /// event's `document_revision`.
    pub(crate) document_revision: u64,
    pub(crate) surface: SurfaceInfo,
    pub(crate) camera: CameraInfo,
    pub(crate) nodes: Vec<NodeGeometry>,
}

/// The image: its size in pixels and the scale it was drawn at.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct SurfaceInfo {
    pub(crate) width: u32,
    pub(crate) height: u32,
    #[serde(serialize_with = "serialize_shortest")]
    pub(crate) scale: f32,
}

/// The camera the image was drawn through. A canvas point `p` is at
/// image pixel `(p - center) × zoom × scale + (width, height) / 2`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct CameraInfo {
    #[serde(serialize_with = "serialize_shortest")]
    pub(crate) center_x: f32,
    #[serde(serialize_with = "serialize_shortest")]
    pub(crate) center_y: f32,
    #[serde(serialize_with = "serialize_shortest")]
    pub(crate) zoom: f32,
}

impl CameraInfo {
    pub(crate) fn of(camera: &Camera2D) -> Self {
        CameraInfo {
            center_x: camera.position.x,
            center_y: camera.position.y,
            zoom: camera.zoom,
        }
    }
}

impl SurfaceInfo {
    pub(crate) fn of(camera: &Camera2D) -> Self {
        SurfaceInfo {
            width: camera.viewport_size.0,
            height: camera.viewport_size.1,
            scale: camera.pixel_ratio,
        }
    }
}

/// One node in the image.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct NodeGeometry {
    pub(crate) id: String,
    /// The node's drawn extent in image pixels, before clipping: a node
    /// half outside the image has a rect that runs past its edge.
    pub(crate) rect: SceneRect,
    /// Whether `rect` runs past an edge of the image.
    pub(crate) clipped: bool,
    /// Its sections the image shows, in model order.
    pub(crate) sections: Vec<SectionGeometry>,
}

/// One section in the image.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SectionGeometry {
    /// The section's index in `MindNode.sections`.
    pub(crate) index: usize,
    pub(crate) rect: SceneRect,
}

/// Whether `rect` runs past the edge of a `width × height` image.
fn clipped(rect: &SceneRect, width: u32, height: u32) -> bool {
    rect.x < 0.0 || rect.y < 0.0 || rect.x + rect.w > width as f32 || rect.y + rect.h > height as f32
}

/// The sidecar for an image drawn through `camera`: every node and
/// section the camera shows, from `tree` (`None` before the first scene
/// build, which shows nothing).
///
/// # Costs
///
/// One walk of the node tree ([`dump_nodes`] without text) and one id
/// `String` per node shown.
pub(crate) fn sidecar(
    tree: Option<&MindMapTree>,
    map: &MindMap,
    camera: &Camera2D,
    protocol: u32,
    document_revision: u64,
) -> Sidecar {
    let (width, height) = camera.viewport_size;
    let nodes = tree
        .map(|tree| dump_nodes(tree, map, camera, DumpFilter::VISIBLE_IDS))
        .unwrap_or_default()
        .into_iter()
        .map(|node| NodeGeometry {
            id: node.node_id,
            rect: node.placement.surface,
            clipped: clipped(&node.placement.surface, width, height),
            sections: node
                .sections
                .into_iter()
                .map(|section| SectionGeometry {
                    index: section.section_idx,
                    rect: section.placement.surface,
                })
                .collect(),
        })
        .collect();
    Sidecar {
        protocol,
        document_revision,
        surface: SurfaceInfo::of(camera),
        camera: CameraInfo::of(camera),
        nodes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::MindMapDocument;
    use glam::Vec2;

    fn testament() -> MindMapDocument {
        let path = baumhard::util::doc_fixtures::repo_path("maps/testament.mindmap.json");
        MindMapDocument::load(path.to_str().expect("a UTF-8 repo path")).expect("the testament map loads")
    }

    /// A camera centered on `node_id`'s laid-out box at `zoom`, drawing
    /// a `width × height` image at `scale`.
    fn camera_on(tree: &MindMapTree, map: &MindMap, node_id: &str, size: (u32, u32), scale: f32) -> Camera2D {
        let probe = Camera2D::new(1, 1);
        let node = baumhard::mindmap::scene_dump::dump_node(tree, map, &probe, node_id).expect("drawn");
        let canvas = node.placement.canvas;
        Camera2D {
            position: Vec2::new(canvas.x + canvas.w / 2.0, canvas.y + canvas.h / 2.0),
            zoom: 1.0,
            viewport_size: size,
            pixel_ratio: scale,
        }
    }

    /// The node the camera centers on is in the sidecar, at the image's
    /// middle, unclipped; its rect scales with `scale` while the set of
    /// nodes does not; and a node the camera does not show is absent.
    #[test]
    fn test_the_sidecar_maps_the_image_and_only_what_it_shows() {
        let doc = testament();
        let tree = doc.build_tree();
        let map = &doc.mindmap;
        let first = map.nodes.keys().min().expect("a node").clone();

        let plain = camera_on(&tree, map, &first, (1600, 1200), 1.0);
        let one = sidecar(Some(&tree), map, &plain, 1, 0);
        assert_eq!(
            (one.protocol, one.surface),
            (
                1,
                SurfaceInfo {
                    width: 1600,
                    height: 1200,
                    scale: 1.0
                }
            )
        );
        let centered = one
            .nodes
            .iter()
            .find(|n| n.id == first)
            .expect("the centered node is shown");
        let middle = Vec2::new(
            centered.rect.x + centered.rect.w / 2.0,
            centered.rect.y + centered.rect.h / 2.0,
        );
        assert!((middle - Vec2::new(800.0, 600.0)).length() < 0.5, "{middle}");

        let doubled = Camera2D {
            viewport_size: (3200, 2400),
            pixel_ratio: 2.0,
            ..plain
        };
        let two = sidecar(Some(&tree), map, &doubled, 1, 0);
        let ids = |s: &Sidecar| s.nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&one), ids(&two), "a pixel ratio changes no membership");
        let twice = two.nodes.iter().find(|n| n.id == first).expect("shown");
        assert!((twice.rect.w - 2.0 * centered.rect.w).abs() < 1e-3);
        assert_eq!(
            two.camera, one.camera,
            "the camera is the view's; the scale is the surface's"
        );

        for node in &one.nodes {
            let r = node.rect;
            assert!(
                r.x + r.w > 0.0 && r.y + r.h > 0.0 && r.x < 1600.0 && r.y < 1200.0,
                "{node:?}"
            );
            assert_eq!(node.clipped, clipped(&r, 1600, 1200));
        }
        let far = Camera2D {
            position: Vec2::new(1.0e7, 1.0e7),
            ..plain
        };
        assert!(
            sidecar(Some(&tree), map, &far, 1, 0).nodes.is_empty(),
            "nothing out there"
        );
        assert!(
            sidecar(None, map, &plain, 1, 0).nodes.is_empty(),
            "no tree, nothing drawn"
        );
    }

    #[test]
    fn test_clipped_is_any_edge_past_the_image() {
        let rect = |x, y, w, h| SceneRect { x, y, w, h };
        assert!(
            !clipped(&rect(0.0, 0.0, 100.0, 50.0), 100, 50),
            "flush with every edge"
        );
        for outside in [
            rect(-0.5, 0.0, 10.0, 10.0),
            rect(0.0, -1.0, 10.0, 10.0),
            rect(95.0, 0.0, 10.0, 10.0),
            rect(0.0, 45.0, 10.0, 10.0),
        ] {
            assert!(clipped(&outside, 100, 50), "{outside:?}");
        }
    }

    /// The wire shape `format/ipc.md` pins: these keys, shortest floats.
    #[test]
    fn test_the_sidecar_serializes_as_the_documented_shape() {
        let value = serde_json::to_value(Sidecar {
            protocol: 1,
            document_revision: 42,
            surface: SurfaceInfo {
                width: 1920,
                height: 1080,
                scale: 1.0,
            },
            camera: CameraInfo {
                center_x: 12.0,
                center_y: -340.5,
                zoom: 0.75,
            },
            nodes: vec![NodeGeometry {
                id: "1.2.3".into(),
                rect: SceneRect {
                    x: 401.0,
                    y: 92.5,
                    w: 218.0,
                    h: 64.0,
                },
                clipped: false,
                sections: vec![SectionGeometry {
                    index: 0,
                    rect: SceneRect {
                        x: 401.0,
                        y: 92.5,
                        w: 218.0,
                        h: 30.0,
                    },
                }],
            }],
        })
        .expect("serializes");
        assert_eq!(
            value,
            serde_json::json!({
                "protocol": 1,
                "document_revision": 42,
                "surface": {"width": 1920, "height": 1080, "scale": 1.0},
                "camera": {"center_x": 12.0, "center_y": -340.5, "zoom": 0.75},
                "nodes": [
                    {"id": "1.2.3",
                     "rect": {"x": 401.0, "y": 92.5, "w": 218.0, "h": 64.0},
                     "clipped": false,
                     "sections": [{"index": 0, "rect": {"x": 401.0, "y": 92.5, "w": 218.0, "h": 30.0}}]}
                ]
            })
        );
    }
}
