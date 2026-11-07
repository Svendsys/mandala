// SPDX-License-Identifier: MPL-2.0

//! The geometry sidecar: the pixels ↔ ids map written next to a
//! screenshot (`<path>.geometry.json`, `format/ipc.md`
//! §`capture.screenshot`), so an agent can point at what it sees by
//! arithmetic instead of by guessing.
//!
//! **It describes the image, not the document.** Each node and section
//! is measured by the glyph runs the projector made for it
//! (`view::inspect::element_bounds`), at the camera the image was drawn
//! through, so its rects are in the image's own pixels and its membership
//! is the draw verdict at that camera (`view::inspect::element_visibility`):
//! a node a fold hides, a zoom window suppresses, or the framing leaves
//! outside is absent, exactly as it is absent from the pixels. Everything
//! else about the scene is `scene.dump`'s.

use baumhard::document::element::DocKind;
use baumhard::document::ids::Document;
use baumhard::flow::Rect;
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::projection::{GlyphLayers, Projector};
use baumhard::util::shortest_float::serialize_shortest;
use serde::Serialize;

use crate::application::document::selection::every_node;
use crate::application::view::inspect::{
    children_of_kind, element_bounds, element_visibility, ident_of, surface_rect,
};

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
    /// The node's integer id (`format/ipc.md` §"Naming elements").
    pub(crate) id: u32,
    /// The node's drawn extent in image pixels, before clipping: a node
    /// half outside the image has a rect that runs past its edge.
    pub(crate) rect: SceneRect,
    /// Whether `rect` runs past an edge of the image.
    pub(crate) clipped: bool,
    /// Its sections the image shows, in document order.
    pub(crate) sections: Vec<SectionGeometry>,
}

/// One section in the image.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SectionGeometry {
    /// The section's index among its node's sections.
    pub(crate) index: usize,
    pub(crate) rect: SceneRect,
}

/// Whether `rect` runs past the edge of a `width × height` image.
fn clipped(rect: &SceneRect, width: u32, height: u32) -> bool {
    rect.x < 0.0 || rect.y < 0.0 || rect.x + rect.w > width as f32 || rect.y + rect.h > height as f32
}

/// A rect in image pixels, as the sidecar carries it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct SceneRect {
    #[serde(serialize_with = "serialize_shortest")]
    pub(crate) x: f32,
    #[serde(serialize_with = "serialize_shortest")]
    pub(crate) y: f32,
    #[serde(serialize_with = "serialize_shortest")]
    pub(crate) w: f32,
    #[serde(serialize_with = "serialize_shortest")]
    pub(crate) h: f32,
}

impl SceneRect {
    fn of(rect: Rect) -> Self {
        SceneRect {
            x: rect.origin.x,
            y: rect.origin.y,
            w: rect.size.x,
            h: rect.size.y,
        }
    }
}

/// The sidecar for an image drawn through `camera`: every node and
/// section the camera shows, in document order, measured off `layers`.
///
/// # Costs
///
/// One walk of the document's nodes, and per node shown its runs and its
/// sections' runs.
pub(crate) fn sidecar(
    doc: &Document,
    projector: &Projector,
    layers: &GlyphLayers,
    camera: &Camera2D,
    protocol: u32,
    document_revision: u64,
) -> Sidecar {
    let (width, height) = camera.viewport_size;
    let shown = |element| element_visibility(doc, projector, layers, element, camera) == Some("visible");
    let nodes = every_node(doc)
        .filter(|&node| shown(node))
        .filter_map(|node| {
            let rect = SceneRect::of(surface_rect(
                element_bounds(doc, projector, layers, node)?,
                camera,
            ));
            let sections = children_of_kind(doc, node, DocKind::Section)
                .enumerate()
                .filter(|&(_, section)| shown(section))
                .filter_map(|(index, section)| {
                    let rect = element_bounds(doc, projector, layers, section)?;
                    Some(SectionGeometry {
                        index,
                        rect: SceneRect::of(surface_rect(rect, camera)),
                    })
                })
                .collect();
            Some(NodeGeometry {
                id: ident_of(doc, node)?.0,
                clipped: clipped(&rect, width, height),
                rect,
                sections,
            })
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

    /// The converted testament map, projected.
    fn testament() -> (MindMapDocument, Projector, GlyphLayers) {
        let path = baumhard::util::doc_fixtures::repo_path("maps/testament.mandala.json");
        let doc = MindMapDocument::load(path.to_str().expect("a UTF-8 repo path"), 50)
            .expect("the testament map loads");
        baumhard::font::fonts::init();
        let mut layers = GlyphLayers::new();
        let mut fonts = baumhard::font::fonts::FontSystemLease::new("geometry tests");
        let projector = Projector::project_full(doc.doc(), &mut layers, &mut fonts);
        (doc, projector, layers)
    }

    /// The node the camera centers on is in the sidecar, at the image's
    /// middle; its rect scales with `scale` while the set of nodes does
    /// not; and a node the camera does not show is absent.
    #[test]
    fn test_the_sidecar_maps_the_image_and_only_what_it_shows() {
        let (doc, projector, layers) = testament();
        let doc = doc.doc();
        let first = every_node(doc)
            .find(|&n| element_bounds(doc, &projector, &layers, n).is_some())
            .expect("a drawn node");
        let first_id = ident_of(doc, first).expect("a node").0;
        let canvas = element_bounds(doc, &projector, &layers, first).expect("drawn");
        let plain = Camera2D {
            position: canvas.origin + canvas.size / 2.0,
            zoom: 1.0,
            viewport_size: (1600, 1200),
            pixel_ratio: 1.0,
        };
        let one = sidecar(doc, &projector, &layers, &plain, 1, 0);
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
            .find(|n| n.id == first_id)
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
        let two = sidecar(doc, &projector, &layers, &doubled, 1, 0);
        let ids = |s: &Sidecar| s.nodes.iter().map(|n| n.id).collect::<Vec<_>>();
        assert_eq!(ids(&one), ids(&two), "a pixel ratio changes no membership");
        let twice = two.nodes.iter().find(|n| n.id == first_id).expect("shown");
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
            sidecar(doc, &projector, &layers, &far, 1, 0).nodes.is_empty(),
            "nothing out there"
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
                id: 7,
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
                    {"id": 7,
                     "rect": {"x": 401.0, "y": 92.5, "w": 218.0, "h": 64.0},
                     "clipped": false,
                     "sections": [{"index": 0, "rect": {"x": 401.0, "y": 92.5, "w": 218.0, "h": 30.0}}]}
                ]
            })
        );
    }
}
