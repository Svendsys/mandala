// SPDX-License-Identifier: MPL-2.0

//! Scene introspection: what the canvas trees **draw**, where, and
//! whether the camera shows it — as plain serializable data, read
//! straight off the `GfxTree`s the renderer
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
//! - its **canvas** rect: the extent the renderer draws for it — the
//!   union of its background fill
//!   ([`GlyphArea::background_rect`](crate::gfx_structs::area::GlyphArea::background_rect),
//!   the box grown by its padding) and every text stamp
//!   ([`GlyphArea::text_stamp_offsets`](crate::gfx_structs::area::GlyphArea::text_stamp_offsets):
//!   the main glyph and each outline-halo stamp), offset by its tree's
//!   registered scene offset. An area that draws nothing itself, an
//!   unfilled node container, is placed by its layout box;
//! - its **surface** rect, that rect through
//!   [`Camera2D::canvas_to_screen`](crate::gfx_structs::camera::Camera2D::canvas_to_screen),
//!   in physical pixels;
//! - its [`Visibility`](crate::gfx_structs::camera::Visibility) verdict
//!   from
//!   [`Camera2D::visibility`](crate::gfx_structs::camera::Camera2D::visibility),
//!   the predicate the renderer's per-frame cull calls, asked of each
//!   drawn rect as the renderer asks it — so "on screen" here and
//!   "drawn" there cannot disagree.
//!
//! And for text, what the user actually reads:
//! [`displayed_text`](crate::font::attrs::displayed_text), the
//! coverage rule the shaper slices spans by.
//!
//! **Names come from the role's own identity index**, the one its
//! hit-test resolves through —
//! [`MindMapTree`](crate::mindmap::tree_builder::MindMapTree)'s id maps
//! for nodes and sections,
//! [`EdgeChannelIndex`](crate::mindmap::tree_builder::EdgeChannelIndex)
//! for connections and labels, and
//! [`PortalHitIndex`](crate::mindmap::tree_builder::PortalHitIndex) for
//! portal markers — so an element is called here what a click on it
//! is called.
//!
//! Every function is pure over `&` references and allocates only its
//! output. A [`DumpFilter`](crate::mindmap::scene_dump::DumpFilter) decides, before any text is resolved,
//! which elements are kept and whether their text is: an element the
//! filter drops, or a read that asks for no text, costs no
//! [`displayed_text`](crate::font::attrs::displayed_text) walk.

use glam::Vec2;
use indextree::NodeId;
use serde::Serialize;

use crate::font::attrs::displayed_text;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::camera::{Camera2D, Visibility};
use crate::gfx_structs::tree::GfxTree;
use crate::gfx_structs::zoom_visibility::ZoomVisibility;
use crate::mindmap::model::MindMap;
use crate::mindmap::scene_cache::EdgeKey;
use crate::mindmap::tree_builder::{EdgeChannelIndex, MindMapTree, PortalHitIndex, PortalPart};
use crate::util::shortest_float::serialize_shortest;

/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reach the `do_*()` bodies (`TEST_CONVENTIONS` §T2.2). The
/// `clippy::unwrap_used` allow is the `gfx_structs::tests` one: these
/// are test code that clippy, without `cfg(test)`, sees as shipped.
#[allow(clippy::unwrap_used)]
pub mod tests;

/// An axis-aligned rect: top-left corner and extent, y-down. In
/// canvas units or surface pixels depending on which side of a
/// [`Placement`] it is.
///
/// Each coordinate serializes as its shortest decimal
/// ([`serialize_shortest`]), so a dump built into a `serde_json::Value`
/// and one streamed into a file carry the same digits.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SceneRect {
    #[serde(serialize_with = "serialize_shortest")]
    pub x: f32,
    #[serde(serialize_with = "serialize_shortest")]
    pub y: f32,
    #[serde(serialize_with = "serialize_shortest")]
    pub w: f32,
    #[serde(serialize_with = "serialize_shortest")]
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

    /// This canvas rect in surface pixels at `camera`: the corner
    /// through [`Camera2D::canvas_to_screen`], the extent scaled by the
    /// camera's [`Camera2D::pixels_per_unit`]. The one place a scene
    /// rect changes space. O(1).
    pub fn to_surface(&self, camera: &Camera2D) -> SceneRect {
        SceneRect::new(
            camera.canvas_to_screen(Vec2::new(self.x, self.y)),
            Vec2::new(self.w, self.h) * camera.pixels_per_unit(),
        )
    }
}

/// Where one drawn element is, in both coordinate spaces, and whether
/// the camera shows it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Placement {
    /// The extent the renderer draws for the element, in canvas space.
    pub canvas: SceneRect,
    /// The same rect in surface pixels at the current camera. Not
    /// clipped to the surface: an element half off-screen has a rect
    /// that runs past its edge.
    pub surface: SceneRect,
    /// The renderer's cull verdict for it.
    pub visibility: Visibility,
}

impl Placement {
    /// A canvas extent and the verdict already reached for it, with
    /// its surface rect derived at `camera`. O(1).
    fn spanning(canvas: SceneRect, visibility: Visibility, camera: &Camera2D) -> Self {
        Placement {
            canvas,
            surface: canvas.to_surface(camera),
            visibility,
        }
    }

    /// Place one canvas rect with its zoom window at `camera`. O(1).
    pub fn of_rect(pos: Vec2, size: Vec2, window: ZoomVisibility, camera: &Camera2D) -> Self {
        Self::spanning(
            SceneRect::new(pos, size),
            camera.visibility(pos, size, window),
            camera,
        )
    }

    /// Place a `GlyphArea` of a tree registered at `offset` the way the
    /// renderer draws it: the union of its background fill
    /// ([`GlyphArea::background_rect`]) and its text stamps
    /// ([`GlyphArea::text_stamp_offsets`], each a `render_bounds` box),
    /// each culled under the area's zoom window as the renderer culls
    /// it, the verdict being the most visible of theirs. An area that
    /// draws neither is placed by its `position` / `render_bounds` box.
    ///
    /// O(1): at most one fill and nine stamps, no allocation.
    pub fn of_area(area: &GlyphArea, offset: Vec2, camera: &Camera2D) -> Self {
        let window = area.zoom_visibility;
        let origin = area.position() + offset;
        let bounds = Vec2::new(area.render_bounds.x.0, area.render_bounds.y.0);
        let mut extent = Extent::default();
        if let Some((pos, size)) = area.background_rect() {
            extent.add(&Self::of_rect(pos + offset, size, window, camera));
        }
        for (dx, dy) in area.text_stamp_offsets() {
            extent.add(&Self::of_rect(origin + Vec2::new(dx, dy), bounds, window, camera));
        }
        extent
            .placement(camera)
            .unwrap_or_else(|| Self::of_rect(origin, bounds, window, camera))
    }
}

/// The running union of an element's drawn parts and the most visible
/// of their verdicts — visible when any part is, off-screen when some
/// part only needs a pan, outside its zoom window only when every part
/// is.
#[derive(Default)]
struct Extent {
    whole: Option<(SceneRect, Visibility)>,
}

impl Extent {
    fn add(&mut self, part: &Placement) {
        let rank = |v: Visibility| match v {
            Visibility::Visible => 2,
            Visibility::Offscreen => 1,
            Visibility::OutsideZoomWindow => 0,
        };
        self.whole = Some(match self.whole {
            None => (part.canvas, part.visibility),
            Some((rect, verdict)) => (
                rect.union(&part.canvas),
                if rank(part.visibility) > rank(verdict) {
                    part.visibility
                } else {
                    verdict
                },
            ),
        });
    }

    /// The element's placement, or `None` when no part was added.
    fn placement(&self, camera: &Camera2D) -> Option<Placement> {
        self.whole
            .map(|(canvas, visibility)| Placement::spanning(canvas, visibility, camera))
    }
}

/// Which drawn elements a dump keeps, and whether it resolves their
/// text. Applied per element before its text is resolved, so a dropped
/// element and a text-free read cost no
/// [`displayed_text`] walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DumpFilter {
    /// Keep only elements whose verdict is [`Visibility::Visible`]. A
    /// node is kept when it or any of its sections is, carrying only
    /// its visible sections.
    pub visible_only: bool,
    /// Resolve each element's displayed text. Without it every `text`
    /// is `None` and is left out of the serialized form.
    pub text: bool,
}

impl DumpFilter {
    /// Every drawn element, with its text.
    pub const ALL: DumpFilter = DumpFilter {
        visible_only: false,
        text: true,
    };
    /// What the camera shows, with its text.
    pub const VISIBLE: DumpFilter = DumpFilter {
        visible_only: true,
        text: true,
    };
    /// What the camera shows, identities and placements only.
    pub const VISIBLE_IDS: DumpFilter = DumpFilter {
        visible_only: true,
        text: false,
    };

    fn keeps(&self, placement: &Placement) -> bool {
        !self.visible_only || placement.visibility.is_visible()
    }

    fn text_of(&self, area: &GlyphArea) -> Option<String> {
        self.text.then(|| displayed_text(&area.text, &area.regions))
    }
}

// --- Nodes and sections ------------------------------------------------

/// One mind node as the scene draws it: its drawn extent and the
/// sections it renders.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeDump {
    pub node_id: String,
    /// The model's `folded` flag: this node's descendants are hidden.
    /// The node itself is drawn — it is in the scene.
    pub folded: bool,
    /// The node container's placement: its background fill grown by
    /// its padding, or its layout box when it draws no fill.
    #[serde(flatten)]
    pub placement: Placement,
    /// Each rendered section the filter keeps, in model order. A
    /// section the tree builder skips (a non-finite or empty authored
    /// size) is never here.
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
    /// grapheme no run covers. `None` when the filter asks for no text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Every node the mindmap tree draws that `filter` keeps, in tree
/// pre-order (parents before children, siblings in model order — the
/// order the renderer walks them). A node hidden by a folded ancestor
/// is not in the tree and so not here; [`MindMap::folding_ancestor`]
/// names the fold.
///
/// # Costs
///
/// O(tree elements): one arena walk, two hash lookups per element, and
/// one [`displayed_text`] per *kept* section when the filter asks for
/// text (a grapheme walk of its text when it carries runs). Allocates
/// the output: one `String` id per kept node, one text `String` per
/// kept section.
pub fn dump_nodes(tree: &MindMapTree, map: &MindMap, camera: &Camera2D, filter: DumpFilter) -> Vec<NodeDump> {
    containers(tree)
        .filter_map(|(mind_id, arena_id)| node_dump(tree, map, camera, mind_id, arena_id, filter))
        .collect()
}

/// Every drawn node's id and placement, in the order [`dump_nodes`]
/// reports them — the node half of a dump without the sections or
/// their text, for a caller that only asks where nodes are (an overlap
/// test).
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

/// One node, by id, with every section and its text, visible or not —
/// [`dump_nodes`] under [`DumpFilter::ALL`] for one node. `None` when
/// the tree does not draw it: an unknown id, or a node hidden by a
/// fold.
///
/// O(children of the node) plus one [`displayed_text`] per section.
pub fn dump_node(tree: &MindMapTree, map: &MindMap, camera: &Camera2D, node_id: &str) -> Option<NodeDump> {
    let arena_id = tree.arena_id_for(node_id)?;
    node_dump(tree, map, camera, node_id, arena_id, DumpFilter::ALL)
}

fn node_dump(
    tree: &MindMapTree,
    map: &MindMap,
    camera: &Camera2D,
    mind_id: &str,
    arena_id: NodeId,
    filter: DumpFilter,
) -> Option<NodeDump> {
    let arena = &tree.tree.arena;
    let container = arena.get(arena_id)?.get().glyph_area()?;
    let placement = Placement::of_area(container, Vec2::ZERO, camera);
    let sections: Vec<SectionDump> = arena_id
        .children(arena)
        .filter_map(|child| {
            let (_, section_idx) = tree.section_for_node(child)?;
            let area = arena.get(child)?.get().glyph_area()?;
            let placement = Placement::of_area(area, Vec2::ZERO, camera);
            filter.keeps(&placement).then(|| SectionDump {
                section_idx,
                placement,
                text: filter.text_of(area),
            })
        })
        .collect();
    if !filter.keeps(&placement) && sections.is_empty() {
        return None;
    }
    Some(NodeDump {
        node_id: mind_id.to_string(),
        folded: map.nodes.get(mind_id).is_some_and(|node| node.folded),
        placement,
        sections,
    })
}

// --- Edge-shaped roles ---------------------------------------------------

/// One connection as the scene draws it: the glyphs sampled along its
/// path, as a whole.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EdgeDump {
    pub edge: EdgeKey,
    /// The union of its glyphs' placements, and the most visible of
    /// their verdicts: visible when any glyph is drawn.
    #[serde(flatten)]
    pub placement: Placement,
    /// Glyphs drawn along the path, caps included.
    pub glyphs: usize,
    /// How many of them the camera shows.
    pub visible_glyphs: usize,
}

/// Every connection a connection-role tree draws that `filter` keeps,
/// in tree order, named through `index` — the tree's own
/// [`EdgeChannelIndex`]. A connection with no glyph (its whole path
/// clipped inside its endpoint nodes) draws nothing and is not
/// reported. `offset` is the tree's registered scene offset. A
/// connection carries no text, so the filter's `text` is moot.
///
/// # Costs
///
/// O(glyphs): one placement per glyph, one `EdgeKey` clone per kept
/// connection. A per-edge `Void` the index cannot name (a tree/index
/// mismatch, never produced by the dispatcher) is left out with a
/// `warn!`.
pub fn dump_edges(
    tree: &GfxTree,
    offset: Vec2,
    camera: &Camera2D,
    index: &EdgeChannelIndex,
    filter: DumpFilter,
) -> Vec<EdgeDump> {
    tree.root()
        .children(&tree.arena)
        .filter_map(|edge_root| {
            let mut glyphs = 0usize;
            let mut visible_glyphs = 0usize;
            let mut extent = Extent::default();
            for glyph in edge_root.children(&tree.arena) {
                let Some(area) = tree.arena.get(glyph).and_then(|n| n.get().glyph_area()) else {
                    continue;
                };
                let placed = Placement::of_area(area, offset, camera);
                glyphs += 1;
                visible_glyphs += usize::from(placed.visibility.is_visible());
                extent.add(&placed);
            }
            let placement = extent.placement(camera)?;
            if !filter.keeps(&placement) {
                return None;
            }
            let Some(edge) = index.resolve(tree, edge_root) else {
                log::warn!("scene_dump: a connection in the scene has no name in its index; left out");
                return None;
            };
            Some(EdgeDump {
                edge,
                placement,
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
    /// buffer with its caret, which is what the tree holds. `None`
    /// when the filter asks for no text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Every label a connection-label tree draws that `filter` keeps, in
/// tree order, named through the tree's [`EdgeChannelIndex`]. `offset`
/// is the tree's registered scene offset.
///
/// O(labels), one `EdgeKey` clone per kept label and one
/// [`displayed_text`] per kept label when the filter asks for text. A
/// label the index cannot name is left out with a `warn!`.
pub fn dump_labels(
    tree: &GfxTree,
    offset: Vec2,
    camera: &Camera2D,
    index: &EdgeChannelIndex,
    filter: DumpFilter,
) -> Vec<LabelDump> {
    tree.root()
        .children(&tree.arena)
        .filter_map(|leaf| {
            let area = tree.arena.get(leaf)?.get().glyph_area()?;
            let placement = Placement::of_area(area, offset, camera);
            if !filter.keeps(&placement) {
                return None;
            }
            let Some(edge) = index.resolve(tree, leaf) else {
                log::warn!("scene_dump: an edge label in the scene has no name in its index; left out");
                return None;
            };
            Some(LabelDump {
                edge,
                placement,
                text: filter.text_of(area),
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
    /// The glyph or caption as displayed. `None` when the filter asks
    /// for no text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Every portal icon and caption a portal tree draws that `filter`
/// keeps, in tree order, named through the tree's [`PortalHitIndex`]
/// — the resolution a click on the marker takes. An endpoint with no
/// caption reserves an empty text slot at zero extent; it draws
/// nothing and is not reported. `offset` is the tree's registered
/// scene offset.
///
/// O(tree elements), one [`PortalHitIndex::resolve`] per kept leaf and
/// one [`displayed_text`] per kept leaf when the filter asks for text.
/// A leaf the index cannot name is left out with a `warn!`.
pub fn dump_portals(
    tree: &GfxTree,
    offset: Vec2,
    camera: &Camera2D,
    index: &PortalHitIndex,
    filter: DumpFilter,
) -> Vec<PortalDump> {
    tree.root()
        .descendants(&tree.arena)
        .filter_map(|leaf| {
            let area = tree.arena.get(leaf)?.get().glyph_area()?;
            if area.text.is_empty() {
                return None;
            }
            let placement = Placement::of_area(area, offset, camera);
            if !filter.keeps(&placement) {
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
                placement,
                text: filter.text_of(area),
            })
        })
        .collect()
}
