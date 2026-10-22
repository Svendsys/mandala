// SPDX-License-Identifier: MPL-2.0

//! Scene introspection: what the canvas trees **draw**, where, and
//! whether the camera shows it — as plain serializable data, read
//! straight off the `Tree<GfxElement, GfxMutator>`s the renderer
//! shapes.
//!
//! **The trees are the source of truth, not the model.** Since the
//! dual-pipeline consolidation (#22) every visual that comes from the
//! document reaches the GPU through one tree per canvas role
//! (CODE_CONVENTIONS §3), so the laid-out scene *is* those trees:
//! positions after layout and drag offsets, sizes after text fitting,
//! zoom windows after cascade, and text after run coverage. This
//! module walks them; it re-derives nothing from the `MindMap`, which
//! is consulted only for what a tree does not carry (a node's own
//! `folded` flag, the fold that removed a node from the tree).
//!
//! **Three facts per element, each from the renderer's own seam:**
//!
//! - its **canvas** rect, the `GlyphArea`'s `position` plus its
//!   tree's registered offset, sized by `render_bounds` — the rect the
//!   renderer shapes a buffer into;
//! - its **surface** rect, that rect through
//!   [`Camera2D::canvas_to_screen`], in physical pixels;
//! - its [`Visibility`] verdict from [`Camera2D::visibility`], the
//!   predicate the renderer's per-frame cull calls — so "on screen"
//!   here and "drawn" there cannot disagree.
//!
//! And for text, what the user actually reads:
//! [`displayed_text`](crate::font::attrs::displayed_text), the
//! coverage rule the shaper slices spans by.
//!
//! **Names come from the role's own identity index**, the one its
//! hit-test resolves through — [`MindMapTree`]'s id maps for nodes and
//! sections, [`EdgeChannelIndex`] for connections and labels,
//! [`PortalHitIndex`] for portal markers — so an element is called
//! here what a click on it is called.
//!
//! Every function is pure over `&` references and allocates only its
//! output. It is meant to be cheap enough to run per screenshot (the
//! IPC capture sidecar), which is why it is benched.

use glam::Vec2;
use indextree::NodeId;
use serde::Serialize;

use crate::font::attrs::displayed_text;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::camera::{Camera2D, Visibility};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::gfx_structs::zoom_visibility::ZoomVisibility;
use crate::mindmap::model::MindMap;
use crate::mindmap::scene_cache::EdgeKey;
use crate::mindmap::tree_builder::{EdgeChannelIndex, MindMapTree, PortalHitIndex, PortalPart};

/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reach the `do_*()` bodies (`TEST_CONVENTIONS` §T2.2). The
/// `clippy::unwrap_used` allow is the `gfx_structs::tests` one: these
/// are test code that clippy, without `cfg(test)`, sees as shipped.
#[allow(clippy::unwrap_used)]
pub mod tests;

/// An axis-aligned rect: top-left corner and extent, y-down. In
/// canvas units or surface pixels depending on which side of a
/// [`Placement`] it is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SceneRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl SceneRect {
    /// The rect at `pos` with extent `size`. O(1).
    pub fn new(pos: Vec2, size: Vec2) -> Self {
        SceneRect {
            x: pos.x,
            y: pos.y,
            w: size.x,
            h: size.y,
        }
    }

    /// Whether the two rects share interior area. Rects that only
    /// touch along an edge do not overlap. O(1).
    pub fn overlaps(&self, other: &SceneRect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }

    /// The smallest rect containing both. O(1).
    pub fn union(&self, other: &SceneRect) -> SceneRect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        SceneRect {
            x,
            y,
            w: (self.x + self.w).max(other.x + other.w) - x,
            h: (self.y + self.h).max(other.y + other.h) - y,
        }
    }

    /// Whether `point` lies inside, edges inclusive. O(1).
    pub fn contains(&self, point: Vec2) -> bool {
        point.x >= self.x && point.x <= self.x + self.w && point.y >= self.y && point.y <= self.y + self.h
    }
}

/// Where one drawn element is, in both coordinate spaces, and whether
/// the camera shows it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Placement {
    /// The element's rect in canvas space: what the renderer shapes
    /// into and culls against.
    pub canvas: SceneRect,
    /// The same rect in surface pixels at the current camera. Not
    /// clipped to the surface: an element half off-screen has a rect
    /// that runs past its edge.
    pub surface: SceneRect,
    /// The renderer's cull verdict for it.
    pub visibility: Visibility,
}

impl Placement {
    /// Place a canvas rect with its zoom window at `camera`. O(1).
    pub fn of_rect(pos: Vec2, size: Vec2, window: ZoomVisibility, camera: &Camera2D) -> Self {
        Placement {
            canvas: SceneRect::new(pos, size),
            surface: SceneRect::new(camera.canvas_to_screen(pos), size * camera.zoom),
            visibility: camera.visibility(pos, size, window),
        }
    }

    /// Place a `GlyphArea` of a tree registered at `offset`, exactly
    /// as the renderer culls its main buffer: `position + offset`,
    /// sized by `render_bounds`, under the area's own zoom window.
    /// O(1).
    pub fn of_area(area: &GlyphArea, offset: Vec2, camera: &Camera2D) -> Self {
        let pos = Vec2::new(area.position.x.0, area.position.y.0) + offset;
        let size = Vec2::new(area.render_bounds.x.0, area.render_bounds.y.0);
        Self::of_rect(pos, size, area.zoom_visibility, camera)
    }
}

/// Fold the verdicts of an element's parts into the element's: it is
/// visible when any part is, off-screen when some part only needs a
/// pan, and outside its zoom window only when every part is. O(1).
fn most_visible(a: Visibility, b: Visibility) -> Visibility {
    let rank = |v: Visibility| match v {
        Visibility::Visible => 2,
        Visibility::Offscreen => 1,
        Visibility::OutsideZoomWindow => 0,
    };
    if rank(b) > rank(a) {
        b
    } else {
        a
    }
}

// --- Nodes and sections ------------------------------------------------

/// One mind node as the scene draws it: its laid-out box and every
/// section it renders.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeDump {
    pub node_id: String,
    /// The model's `folded` flag: this node's descendants are hidden.
    /// The node itself is drawn — it is in the scene.
    pub folded: bool,
    /// The node's container box — the rect its background fills and
    /// its hit test resolves against.
    #[serde(flatten)]
    pub placement: Placement,
    /// Each rendered section, in model order. A section the tree
    /// builder skips (a non-finite or empty authored size) is absent.
    pub sections: Vec<SectionDump>,
}

/// One section of a node as the scene draws it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SectionDump {
    /// The section's index in `MindNode.sections`.
    pub section_idx: usize,
    #[serde(flatten)]
    pub placement: Placement,
    /// What the user reads in it: the section's text after its
    /// `text_runs` coverage ([`displayed_text`]), which drops every
    /// grapheme no run covers.
    pub text: String,
}

/// Every node the mindmap tree draws, in tree pre-order (parents
/// before children, siblings in model order — the order the renderer
/// walks them). A node hidden by a folded ancestor is not in the tree
/// and so not here; [`MindMap::folding_ancestor`] names the fold.
///
/// # Costs
///
/// O(tree elements): one arena walk, two hash lookups per element,
/// one [`displayed_text`] per section (a grapheme walk of its text
/// when it carries runs). Allocates the output: one `String` id per
/// node, one text `String` per section.
pub fn dump_nodes(tree: &MindMapTree, map: &MindMap, camera: &Camera2D) -> Vec<NodeDump> {
    containers(tree)
        .filter_map(|(mind_id, arena_id)| node_dump(tree, map, camera, mind_id, arena_id))
        .collect()
}

/// Every drawn node's id and laid-out box, in the order
/// [`dump_nodes`] reports them — the node half of a dump without the
/// sections or their text, for a caller that only asks where nodes
/// are (an overlap test).
///
/// O(tree elements): one arena walk and one hash lookup per element.
/// Allocates nothing; the ids borrow from `tree`.
pub fn node_boxes<'t>(
    tree: &'t MindMapTree,
    camera: &'t Camera2D,
) -> impl Iterator<Item = (&'t str, Placement)> + 't {
    containers(tree).filter_map(move |(mind_id, arena_id)| {
        let container = tree.tree.arena.get(arena_id)?.get().glyph_area()?;
        Some((mind_id, Placement::of_area(container, Vec2::ZERO, camera)))
    })
}

/// The node containers of `tree`, `(mind id, arena id)`, in pre-order.
fn containers(tree: &MindMapTree) -> impl Iterator<Item = (&str, NodeId)> + '_ {
    tree.tree
        .root()
        .descendants(&tree.tree.arena)
        .filter_map(|arena_id| Some((tree.mind_id_for_node(arena_id)?, arena_id)))
}

/// One node, by id, as [`dump_nodes`] would report it. `None` when the
/// tree does not draw it: an unknown id, or a node hidden by a fold.
///
/// O(children of the node) plus one [`displayed_text`] per section.
pub fn dump_node(tree: &MindMapTree, map: &MindMap, camera: &Camera2D, node_id: &str) -> Option<NodeDump> {
    let arena_id = tree.arena_id_for(node_id)?;
    node_dump(tree, map, camera, node_id, arena_id)
}

fn node_dump(
    tree: &MindMapTree,
    map: &MindMap,
    camera: &Camera2D,
    mind_id: &str,
    arena_id: NodeId,
) -> Option<NodeDump> {
    let arena = &tree.tree.arena;
    let container = arena.get(arena_id)?.get().glyph_area()?;
    let sections = arena_id
        .children(arena)
        .filter_map(|child| {
            let (_, section_idx) = tree.section_for_node(child)?;
            let area = arena.get(child)?.get().glyph_area()?;
            Some(SectionDump {
                section_idx,
                placement: Placement::of_area(area, Vec2::ZERO, camera),
                text: displayed_text(&area.text, &area.regions),
            })
        })
        .collect();
    Some(NodeDump {
        node_id: mind_id.to_string(),
        folded: map.nodes.get(mind_id).is_some_and(|node| node.folded),
        placement: Placement::of_area(container, Vec2::ZERO, camera),
        sections,
    })
}

// --- Edge-shaped roles ---------------------------------------------------

/// One connection as the scene draws it: the glyphs sampled along its
/// path, as a whole.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EdgeDump {
    pub edge: EdgeKey,
    /// The union of its glyph rects, and the most visible of their
    /// verdicts: visible when any glyph is drawn.
    #[serde(flatten)]
    pub placement: Placement,
    /// Glyphs drawn along the path, caps included.
    pub glyphs: usize,
    /// How many of them the camera shows.
    pub visible_glyphs: usize,
}

/// Every connection a connection-role tree draws, in tree order,
/// named through `index` — the tree's own [`EdgeChannelIndex`]. A
/// connection with no glyph (its whole path clipped inside its
/// endpoint nodes) draws nothing and is not reported. `offset` is the
/// tree's registered scene offset.
///
/// # Costs
///
/// O(glyphs): one rect and verdict per glyph, one `EdgeKey` clone per
/// connection. A per-edge `Void` the index cannot name (a tree/index
/// mismatch, never produced by the dispatcher) is left out with a
/// `warn!`.
pub fn dump_edges(
    tree: &Tree<GfxElement, GfxMutator>,
    offset: Vec2,
    camera: &Camera2D,
    index: &EdgeChannelIndex,
) -> Vec<EdgeDump> {
    tree.root()
        .children(&tree.arena)
        .filter_map(|edge_root| {
            let mut glyphs = 0usize;
            let mut visible_glyphs = 0usize;
            let mut whole: Option<(SceneRect, Visibility)> = None;
            for glyph in edge_root.children(&tree.arena) {
                let Some(area) = tree.arena.get(glyph).and_then(|n| n.get().glyph_area()) else {
                    continue;
                };
                let placed = Placement::of_area(area, offset, camera);
                glyphs += 1;
                visible_glyphs += usize::from(placed.visibility.is_visible());
                whole = Some(match whole {
                    None => (placed.canvas, placed.visibility),
                    Some((rect, verdict)) => (
                        rect.union(&placed.canvas),
                        most_visible(verdict, placed.visibility),
                    ),
                });
            }
            let (canvas, visibility) = whole?;
            let Some(edge) = index.resolve(tree, edge_root) else {
                log::warn!("scene_dump: a connection in the scene has no name in its index; left out");
                return None;
            };
            let origin = Vec2::new(canvas.x, canvas.y);
            Some(EdgeDump {
                edge,
                placement: Placement {
                    canvas,
                    surface: SceneRect::new(
                        camera.canvas_to_screen(origin),
                        Vec2::new(canvas.w, canvas.h) * camera.zoom,
                    ),
                    visibility,
                },
                glyphs,
                visible_glyphs,
            })
        })
        .collect()
}

/// One edge label as the scene draws it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LabelDump {
    pub edge: EdgeKey,
    #[serde(flatten)]
    pub placement: Placement,
    /// The label as displayed — while it is being edited, the live
    /// buffer with its caret, which is what the tree holds.
    pub text: String,
}

/// Every label a connection-label tree draws, in tree order, named
/// through the tree's [`EdgeChannelIndex`]. `offset` is the tree's
/// registered scene offset.
///
/// O(labels), one `EdgeKey` clone and one [`displayed_text`] each. A
/// label the index cannot name is left out with a `warn!`.
pub fn dump_labels(
    tree: &Tree<GfxElement, GfxMutator>,
    offset: Vec2,
    camera: &Camera2D,
    index: &EdgeChannelIndex,
) -> Vec<LabelDump> {
    tree.root()
        .children(&tree.arena)
        .filter_map(|leaf| {
            let area = tree.arena.get(leaf)?.get().glyph_area()?;
            let Some(edge) = index.resolve(tree, leaf) else {
                log::warn!("scene_dump: an edge label in the scene has no name in its index; left out");
                return None;
            };
            Some(LabelDump {
                edge,
                placement: Placement::of_area(area, offset, camera),
                text: displayed_text(&area.text, &area.regions),
            })
        })
        .collect()
}

/// One portal marker part as the scene draws it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PortalDump {
    pub edge: EdgeKey,
    /// The node this marker floats above.
    pub endpoint_node_id: String,
    pub part: PortalPart,
    #[serde(flatten)]
    pub placement: Placement,
    /// The glyph or caption as displayed.
    pub text: String,
}

/// Every portal icon and caption a portal tree draws, in tree order,
/// named through the tree's [`PortalHitIndex`] — the resolution a
/// click on the marker takes. An endpoint with no caption reserves an
/// empty text slot at zero extent; it draws nothing and is not
/// reported. `offset` is the tree's registered scene offset.
///
/// O(tree elements), one [`PortalHitIndex::resolve`] and one
/// [`displayed_text`] per drawn leaf. A leaf the index cannot name is
/// left out with a `warn!`.
pub fn dump_portals(
    tree: &Tree<GfxElement, GfxMutator>,
    offset: Vec2,
    camera: &Camera2D,
    index: &PortalHitIndex,
) -> Vec<PortalDump> {
    tree.root()
        .descendants(&tree.arena)
        .filter_map(|leaf| {
            let area = tree.arena.get(leaf)?.get().glyph_area()?;
            if area.text.is_empty() {
                return None;
            }
            let Some(hit) = index.resolve(tree, leaf) else {
                log::warn!("scene_dump: a portal marker in the scene has no name in its index; left out");
                return None;
            };
            Some(PortalDump {
                edge: hit.edge_key,
                endpoint_node_id: hit.endpoint_node_id,
                part: hit.part,
                placement: Placement::of_area(area, offset, camera),
                text: displayed_text(&area.text, &area.regions),
            })
        })
        .collect()
}
