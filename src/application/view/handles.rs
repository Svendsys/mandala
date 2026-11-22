// SPDX-License-Identifier: MPL-2.0

//! **The handle chrome** (design §12.1; V1-C7 #248): the grab handles
//! the live interactions drag — a node's and a section's eight resize
//! handles, and a selected path's anchors, control points and midpoint —
//! as view-only glyphs in the chrome layer, built from app state: the
//! selection, the mode, and where the projector drew things (its
//! [`Projector`] geometry reads; no glyph layer is read).
//!
//! **Three handle sets, each in its own group, each on its own channel
//! range.** The sets are planted once, when the view is built, hidden
//! (opacity 0): a group per set under the chrome's root
//! ([`SET_GROUP_CHANNELS`]), and in it one run per handle on the set's
//! own channels ([`HandleSet::channel`]: node `0..8`, section `16..24`,
//! path `32..43`). A set is updated in place by one mutator tree
//! anchored at its group, with one child per handle that moved, keyed by
//! that handle's channel. The walker's merge gives one mutator to every
//! consecutive target with its key, and the first mutator with a key
//! consumes those targets (V1-A2 §3), so two sets that shared a parent
//! and their channels would have the second's updates reach nothing; a
//! group per set, and channels no other set uses, are what keeps every
//! set's keys its own (the note on #248 from #274's review).
//!
//! **Updated only when what it is built from changes**: the selection
//! or the mode (`MapView::sync` compares the targets it wants against
//! the ones it shows), or a sync pass whose projection may have moved a
//! target's geometry ([`baumhard::projection::Projector::geometry_moved`]
//! — a node's drawn rectangle, a section's box or its node's, a path's
//! curve). On geometry, not on the runs a target owns, which a node that
//! draws no frame does not write when it grows (#395's review). A pan or
//! a zoom changes none of them and writes no chrome: the renderer applies
//! the camera to the chrome at draw time, as to every layer (§12.4).
//!
//! **Hit testing is the chrome's own** ([`Handles::at`]): the centers
//! the sets show, kept here, against the pointer, before anything asks
//! the projector what a press landed on.
//!
//! Both targets build it, as both drew `main`'s handles; only native
//! has the drags that grab them (CLAUDE.md "Dual-target status").

use baumhard::arena::NodeId;
use baumhard::document::element::DocElement;
use baumhard::document::ids::Document;
use baumhard::document::resolve::ResolvedText;
use baumhard::document::values::{Align, FontSize, Weight};
use baumhard::engine::anchor::Anchors;
use baumhard::engine::tree::Placement;
use baumhard::flow::{shape_box, FlowSpan, Rect};
use baumhard::font::fonts::FontSystemLease;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::{GfxMutation, GfxMutator};
use baumhard::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunShape};
use baumhard::gfx_structs::tree::{GfxMutatorTree, GfxTree};
use baumhard::projection::Projector;
use glam::Vec2;
use std::sync::Arc;

use super::overlay::SELECTION_HIGHLIGHT;
use crate::application::document::Selection;

/// The glyph a resize handle draws: a small open square, as on `main`.
pub(crate) const RESIZE_HANDLE_GLYPH: &str = "\u{25A1}"; // □

/// The glyph a path's anchor or control point draws: a diamond, as on
/// `main`.
pub(crate) const PATH_HANDLE_GLYPH: &str = "\u{25C6}"; // ◆

/// The glyph a path's midpoint handle draws — the one that adds a
/// control point: a hook, as on `main`.
pub(crate) const MIDPOINT_HANDLE_GLYPH: &str = "\u{21BA}"; // ↺

/// The handles' glyph size, in points: `main`'s.
const HANDLE_SIZE_PT: f32 = 14.0;

/// How many control points a path's set has a handle for: as many as a
/// path holds ([`baumhard::document::bounds::MAX_CONTROL_POINTS`]).
const CONTROL_SLOTS: usize = baumhard::document::bounds::MAX_CONTROL_POINTS;

/// The channels of the three sets' groups under the chrome's root, in
/// [`HandleSet::ALL`]'s order: above the rubber band's runs, on
/// `RUN_CHANNEL`.
pub(crate) const SET_GROUP_CHANNELS: [usize; 3] = [1, 2, 3];

/// Which of the eight resize handles round a box: clockwise from the
/// top-left corner, as on `main`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ResizeSide {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

impl ResizeSide {
    /// Every side, in slot order.
    pub(crate) const ALL: [ResizeSide; 8] = [
        ResizeSide::NW,
        ResizeSide::N,
        ResizeSide::NE,
        ResizeSide::E,
        ResizeSide::SE,
        ResizeSide::S,
        ResizeSide::SW,
        ResizeSide::W,
    ];

    /// How the handle moves each axis: `1` grows the size, `-1` moves
    /// the top-left and shrinks the size by as much (the opposite edge
    /// stays put), `0` leaves the axis alone. O(1).
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "the browser draws the handles; only native's drags grab them"
        )
    )]
    pub(crate) fn axis_factors(self) -> (i8, i8) {
        match self {
            ResizeSide::NW => (-1, -1),
            ResizeSide::N => (0, -1),
            ResizeSide::NE => (1, -1),
            ResizeSide::E => (1, 0),
            ResizeSide::SE => (1, 1),
            ResizeSide::S => (0, 1),
            ResizeSide::SW => (-1, 1),
            ResizeSide::W => (-1, 0),
        }
    }

    /// The box `start` resized by this handle dragged `delta`, its size
    /// held to at least `floor` on each axis: what a resize stages, from
    /// the press each time (never accumulated). A handle that moves the
    /// top-left moves it only as far as the size shrinks, so a box that
    /// cannot shrink below its content does not slide. O(1).
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "the browser draws the handles; only native's drags grab them"
        )
    )]
    pub(crate) fn resolve(self, start: Rect, delta: Vec2, floor: Vec2) -> Rect {
        let (fx, fy) = self.axis_factors();
        // One axis: its new origin and size. A handle that moves the
        // origin keeps the far edge where it was.
        let axis = |factor: i8, origin: f32, size: f32, d: f32, floor: f32| -> (f32, f32) {
            let floor = floor.max(0.0);
            match factor {
                -1 => {
                    let resized = (size - d).max(floor);
                    (origin + size - resized, resized)
                }
                1 => (origin, (size + d).max(floor)),
                _ => (origin, size),
            }
        };
        let (x, w) = axis(fx, start.origin.x, start.size.x, delta.x, floor.x);
        let (y, h) = axis(fy, start.origin.y, start.size.y, delta.y, floor.y);
        Rect::new(Vec2::new(x, y), Vec2::new(w, h))
    }

    /// The corner a fast resize grabs from `point` within `rect`: the
    /// one in the quadrant `point` is in, a point on a center line
    /// rounding south and east, as on `main`. Edge handles are never
    /// picked: a one-axis resize is the resize mode's. O(1).
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "the browser draws the handles; only native's drags grab them"
        )
    )]
    pub(crate) fn corner_for(point: Vec2, rect: Rect) -> ResizeSide {
        let center = rect.center();
        match (point.x >= center.x, point.y >= center.y) {
            (false, false) => ResizeSide::NW,
            (true, false) => ResizeSide::NE,
            (false, true) => ResizeSide::SW,
            (true, true) => ResizeSide::SE,
        }
    }

    /// Where this handle sits on `rect`. O(1).
    fn on(self, rect: Rect) -> Vec2 {
        let (min, max, c) = (rect.origin, rect.max(), rect.center());
        match self {
            ResizeSide::NW => min,
            ResizeSide::N => Vec2::new(c.x, min.y),
            ResizeSide::NE => Vec2::new(max.x, min.y),
            ResizeSide::E => Vec2::new(max.x, c.y),
            ResizeSide::SE => max,
            ResizeSide::S => Vec2::new(c.x, max.y),
            ResizeSide::SW => Vec2::new(min.x, max.y),
            ResizeSide::W => Vec2::new(min.x, c.y),
        }
    }

    /// The side's name, as `query.mode` reports it: `main`'s. O(1).
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "the browser draws the handles; only native's drags grab them"
        )
    )]
    pub(crate) fn word(self) -> &'static str {
        match self {
            ResizeSide::NW => "nw",
            ResizeSide::N => "n",
            ResizeSide::NE => "ne",
            ResizeSide::E => "e",
            ResizeSide::SE => "se",
            ResizeSide::S => "s",
            ResizeSide::SW => "sw",
            ResizeSide::W => "w",
        }
    }
}

/// One of a path's handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PathHandle {
    /// The path's start: dragging it attaches the start to the side of
    /// its node nearest the pointer.
    Start,
    /// The path's end, likewise.
    End,
    /// The control point at this index.
    Control(usize),
    /// The middle of a path with no control points and not straight:
    /// dragging it adds the path's one control point there (#248's
    /// ruling on its C1).
    Midpoint,
}

impl PathHandle {
    /// The handle's slot in the path set: start, end, the control points
    /// in order, the midpoint. O(1).
    fn slot(self) -> usize {
        match self {
            PathHandle::Start => 0,
            PathHandle::End => 1,
            PathHandle::Control(k) => 2 + k,
            PathHandle::Midpoint => 2 + CONTROL_SLOTS,
        }
    }

    /// The handle in path-set slot `slot`. O(1).
    fn of_slot(slot: usize) -> PathHandle {
        match slot {
            0 => PathHandle::Start,
            1 => PathHandle::End,
            s if s < 2 + CONTROL_SLOTS => PathHandle::Control(s - 2),
            _ => PathHandle::Midpoint,
        }
    }

    /// The handle's name, as `query.mode` reports it. Allocates for a
    /// control point.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "the browser draws the handles; only native's drags grab them"
        )
    )]
    pub(crate) fn word(self) -> String {
        match self {
            PathHandle::Start => "start".to_string(),
            PathHandle::End => "end".to_string(),
            PathHandle::Control(k) => format!("control:{k}"),
            PathHandle::Midpoint => "midpoint".to_string(),
        }
    }
}

/// A handle the chrome shows, as a press grabs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        dead_code,
        reason = "the browser draws the handles; only native's drags grab them"
    )
)]
pub(crate) enum Handle {
    /// A resize handle of the node the resize mode targets.
    Node { node: NodeId, side: ResizeSide },
    /// A resize handle of the section the resize mode targets.
    Section { section: NodeId, side: ResizeSide },
    /// A handle of the one selected path.
    Path { path: NodeId, handle: PathHandle },
}

/// One of the three handle sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HandleSet {
    /// The resize mode's node: eight handles.
    Node,
    /// The resize mode's section: eight handles.
    Section,
    /// The one selected path: start, end, a control point each, midpoint.
    Path,
}

impl HandleSet {
    /// The sets, in hit-testing precedence: a resize handle over a path's
    /// handle is the resize's, as on `main`.
    pub(crate) const ALL: [HandleSet; 3] = [HandleSet::Node, HandleSet::Section, HandleSet::Path];

    /// How many handles the set has. O(1).
    fn len(self) -> usize {
        match self {
            HandleSet::Node | HandleSet::Section => ResizeSide::ALL.len(),
            HandleSet::Path => 3 + CONTROL_SLOTS,
        }
    }

    /// The channel of the set's handle in `slot`: each set on its own
    /// range, so no two sets' keys meet whatever parent they were under.
    /// O(1).
    pub(crate) fn channel(self, slot: usize) -> usize {
        let base = match self {
            HandleSet::Node => 0,
            HandleSet::Section => 16,
            HandleSet::Path => 32,
        };
        base + slot
    }

    /// The set's index in [`Self::ALL`]. O(1).
    pub(crate) fn index(self) -> usize {
        match self {
            HandleSet::Node => 0,
            HandleSet::Section => 1,
            HandleSet::Path => 2,
        }
    }

    /// The glyph the handle in `slot` draws. O(1).
    fn glyph(self, slot: usize) -> &'static str {
        match self {
            HandleSet::Node | HandleSet::Section => RESIZE_HANDLE_GLYPH,
            HandleSet::Path if PathHandle::of_slot(slot) == PathHandle::Midpoint => MIDPOINT_HANDLE_GLYPH,
            HandleSet::Path => PATH_HANDLE_GLYPH,
        }
    }

    /// The handle of this set on `target` in `slot`. O(1).
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "the browser draws the handles; only native's drags grab them"
        )
    )]
    fn handle(self, target: NodeId, slot: usize) -> Handle {
        match self {
            HandleSet::Node => Handle::Node {
                node: target,
                side: ResizeSide::ALL[slot],
            },
            HandleSet::Section => Handle::Section {
                section: target,
                side: ResizeSide::ALL[slot],
            },
            HandleSet::Path => Handle::Path {
                path: target,
                handle: PathHandle::of_slot(slot),
            },
        }
    }
}

/// What each set shows its handles on, in [`HandleSet::ALL`]'s order:
/// the resize mode's node, its section, and the one selected path.
pub(crate) type Targets = [Option<NodeId>; 3];

/// The targets the selection and the resize target want: a node or a
/// section for the resize sets, from the mode's target; the path set's
/// for a selection of exactly one connector or line link, as on `main`
/// (one selected edge). A target `doc` no longer holds is none. O(1).
pub(crate) fn targets_for(
    doc: &Document,
    selection: &Selection,
    resize: Option<crate::application::app::ResizeTarget>,
) -> Targets {
    use crate::application::app::ResizeTarget;
    let held = |id: NodeId| doc.get(id).is_some().then_some(id);
    let (node, section) = match resize {
        Some(ResizeTarget::Node(node)) => (held(node), None),
        Some(ResizeTarget::Section(section)) => (None, held(section)),
        None => (None, None),
    };
    let mut selected = selection.iter();
    let path = match (selected.next(), selected.next()) {
        (Some(only), None) if is_line_path(doc, only) => Some(only),
        _ => None,
    };
    [node, section, path]
}

/// Whether `id` is a connector or a line link: an element that draws a
/// path. O(1).
pub(crate) fn is_line_path(doc: &Document, id: NodeId) -> bool {
    match doc.get(id) {
        Some(DocElement::Connector(_)) => true,
        Some(DocElement::Link(link)) => link.kind == baumhard::document::values::LinkKind::Line,
        _ => false,
    }
}

/// A path's data: a connector's or a link's. O(1).
pub(crate) fn path_data(doc: &Document, path: NodeId) -> Option<&baumhard::document::element::PathData> {
    match doc.get(path)? {
        DocElement::Connector(connector) => Some(&connector.path),
        DocElement::Link(link) => Some(&link.path),
        _ => None,
    }
}

/// Where the handles of `set` on `target` center, slot by slot — `None`
/// for a slot the target shows no handle in — read from the projector's
/// geometry and the document. Every slot `None` for a target that is not
/// drawn. Cost: O(the set's slots), one hash read each.
pub(crate) fn centers(
    set: HandleSet,
    target: Option<NodeId>,
    doc: &Document,
    projector: &Projector,
) -> Vec<Option<Vec2>> {
    let mut out = vec![None; set.len()];
    let Some(target) = target else {
        return out;
    };
    match set {
        HandleSet::Node | HandleSet::Section => {
            let rect = if set == HandleSet::Node {
                projector.node_rect(target)
            } else {
                projector.section_rect(target)
            };
            if let Some(rect) = rect.filter(|r| r.size.x > 0.0 && r.size.y > 0.0 && r.size.is_finite()) {
                for (slot, side) in ResizeSide::ALL.into_iter().enumerate() {
                    out[slot] = Some(side.on(rect));
                }
            }
        }
        HandleSet::Path => {
            let (Some(curves), Some(data)) = (projector.curve(target), path_data(doc, target)) else {
                return out;
            };
            let (Some(first), Some(last)) = (curves.first(), curves.last()) else {
                return out;
            };
            out[PathHandle::Start.slot()] = Some(first.point(0.0));
            out[PathHandle::End.slot()] = Some(last.point(1.0));
            let n = data.control_points.len();
            if n == 0 && !data.straight {
                out[PathHandle::Midpoint.slot()] = midpoint(curves);
            } else if let Some(body) = body(curves, n) {
                // The path passes through each control point: the body's
                // curve `k` ends at point `k` (D34).
                for (k, curve) in body.iter().take(n.min(CONTROL_SLOTS)).enumerate() {
                    out[PathHandle::Control(k).slot()] = Some(curve.point(1.0));
                }
            }
        }
    }
    out
}

/// The body of a drawn path with `n` control points: its `n + 1` curves
/// between the straight leads its ends run out along first
/// (`projection::geometry::path_through`), each from a point it passes
/// through to the next — the ends, then the control points in order.
/// `None` for a chain that is not such a path's. O(1).
pub(crate) fn body(curves: &[baumhard::flow::Curve], n: usize) -> Option<&[baumhard::flow::Curve]> {
    let lead =
        usize::from(curves.len() > n + 1 && matches!(curves.first(), Some(baumhard::flow::Curve::Line(..))));
    curves.get(lead..lead + n + 1)
}

/// Where a path with no control points draws its midpoint handle: the
/// middle of its body's one curve, by parameter — where the control point
/// a drag of it adds starts, so the path does not move until the pointer
/// does. `None` for a chain that is not such a path's. O(1).
pub(crate) fn midpoint(curves: &[baumhard::flow::Curve]) -> Option<Vec2> {
    Some(body(curves, 0)?.first()?.point(0.5))
}

/// One handle set as it is planted and shown.
struct Planted {
    /// The set's group under the chrome's root, which holds one run per
    /// slot, on the set's channels.
    group: NodeId,
    /// Half each slot's run, to center it on its handle.
    half: Vec<Vec2>,
    /// What the set is shown on.
    target: Option<NodeId>,
    /// Where each slot's handle centers while shown.
    shown: Vec<Option<Vec2>>,
}

/// The three handle sets in the chrome. See the [module docs](self).
pub(crate) struct Handles {
    sets: [Planted; 3],
    /// How many times a set's centers were read again: what the tests
    /// that hold a pan to rebuilding nothing count. Test builds only.
    #[cfg(test)]
    rebuilt: u64,
}

impl Handles {
    /// Put the three sets in `chrome`, hidden, each in its group under
    /// the root on its channels. Called once, when the chrome is built.
    pub(crate) fn plant(chrome: &mut GfxTree) -> Handles {
        let mut fonts = FontSystemLease::new("view::handles::plant");
        let sets = HandleSet::ALL.map(|set| {
            let group = chrome
                .insert_child(
                    chrome.root(),
                    GfxElement::new_void(SET_GROUP_CHANNELS[set.index()]),
                    Placement::Last,
                )
                .expect("the chrome's root takes a group");
            let mut half = Vec::with_capacity(set.len());
            for slot in 0..set.len() {
                let (run, size) = handle_run(set.glyph(slot), &mut fonts);
                half.push(size * 0.5);
                chrome
                    .insert_child(
                        group,
                        GfxElement::new_run(run, set.channel(slot), 0),
                        Placement::Last,
                    )
                    .expect("a handle group takes its handles");
            }
            Planted {
                group,
                half,
                target: None,
                shown: vec![None; set.len()],
            }
        });
        Handles {
            sets,
            #[cfg(test)]
            rebuilt: 0,
        }
    }

    /// The chrome mutators that show each set on `targets`, where the
    /// targets changed or `follow` says a set's target was touched: one
    /// mutator tree per set that changed, anchored at its group, with a
    /// child per handle that moved, appeared or went, keyed by its
    /// channel — every other handle and every other set untouched.
    /// Nothing when nothing changed.
    ///
    /// Cost: O(the slots of the sets that are rebuilt).
    pub(crate) fn update(
        &mut self,
        targets: Targets,
        follow: [bool; 3],
        doc: &Document,
        projector: &Projector,
    ) -> Vec<(GfxMutatorTree, Anchors)> {
        let mut out = Vec::new();
        for set in HandleSet::ALL {
            let i = set.index();
            if targets[i] == self.sets[i].target && !follow[i] {
                continue;
            }
            let want = centers(set, targets[i], doc, projector);
            #[cfg(test)]
            {
                self.rebuilt += 1;
            }
            let planted = &mut self.sets[i];
            planted.target = targets[i];
            if let Some(tree) = planted.diff(set, &want) {
                out.push((tree, Anchors::Nodes(vec![planted.group])));
            }
            planted.shown = want;
        }
        out
    }

    /// The handle a press at `point` grabs: the shown handle whose
    /// center is nearest, within `tolerance` (canvas units) of it, the
    /// resize sets before the path's. O(the shown handles).
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "the browser draws the handles; only native's drags grab them"
        )
    )]
    pub(crate) fn at(&self, point: Vec2, tolerance: f32) -> Option<Handle> {
        for set in HandleSet::ALL {
            let planted = &self.sets[set.index()];
            let Some(target) = planted.target else {
                continue;
            };
            let nearest = planted
                .shown
                .iter()
                .enumerate()
                .filter_map(|(slot, center)| center.map(|c| (slot, c.distance(point))))
                .filter(|&(_, d)| d <= tolerance)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((slot, _)) = nearest {
                return Some(set.handle(target, slot));
            }
        }
        None
    }

    /// Where `handle` centers while it is shown. O(1).
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "the browser draws the handles; only native's drags grab them"
        )
    )]
    pub(crate) fn center_of(&self, handle: Handle) -> Option<Vec2> {
        let (set, target, slot) = match handle {
            Handle::Node { node, side } => (HandleSet::Node, node, side_slot(side)),
            Handle::Section { section, side } => (HandleSet::Section, section, side_slot(side)),
            Handle::Path { path, handle } => (HandleSet::Path, path, handle.slot()),
        };
        let planted = &self.sets[set.index()];
        if planted.target != Some(target) {
            return None;
        }
        planted.shown.get(slot).copied().flatten()
    }

    /// How many times a set was rebuilt since the view was built. Test
    /// builds only.
    #[cfg(test)]
    pub(crate) fn rebuilt(&self) -> u64 {
        self.rebuilt
    }

    /// The group of `set`, which holds its runs slot by slot: for the
    /// tests that read what the chrome holds.
    #[cfg(test)]
    pub(crate) fn group(&self, set: HandleSet) -> NodeId {
        self.sets[set.index()].group
    }
}

/// The slot of `side` in a resize set. O(1).
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        dead_code,
        reason = "the browser draws the handles; only native's drags grab them"
    )
)]
fn side_slot(side: ResizeSide) -> usize {
    ResizeSide::ALL
        .iter()
        .position(|&s| s == side)
        .expect("every side is in ALL")
}

impl Planted {
    /// The mutator tree that takes the set from what it shows to `want`:
    /// a void root, anchored at the group, and a child per slot that
    /// changed — a slot shown moved to its center at full opacity, a
    /// slot no longer shown made transparent where it was. `None` when
    /// no slot changed.
    fn diff(&self, set: HandleSet, want: &[Option<Vec2>]) -> Option<GfxMutatorTree> {
        let mut tree = GfxMutatorTree::new_with(GfxMutator::new_void(SET_GROUP_CHANNELS[set.index()]));
        let root = tree.root();
        let mut any = false;
        for (slot, (now, next)) in self.shown.iter().zip(want).enumerate() {
            if now == next {
                continue;
            }
            let mutations = match next {
                Some(center) => vec![
                    GfxMutation::RunOrigin(*center - self.half[slot]),
                    GfxMutation::RunOpacity(1.0),
                ],
                None => vec![GfxMutation::RunOpacity(0.0)],
            };
            tree.insert_child(
                root,
                GfxMutator::new_macro(mutations, set.channel(slot)),
                Placement::Last,
            )
            .expect("a handle set's channels ascend");
            any = true;
        }
        any.then_some(tree)
    }
}

/// One hidden handle run of `glyph`, at the origin, in the selection's
/// cyan, and its size.
fn handle_run(glyph: &str, fonts: &mut FontSystemLease) -> (GlyphRunData, Vec2) {
    let [r, g, b, _] = SELECTION_HIGHLIGHT.0;
    let style = ResolvedText {
        font: baumhard::document::values::FontRef::DEFAULT,
        size: FontSize::Pt(HANDLE_SIZE_PT),
        color: baumhard::document::values::Rgba::new(r, g, b, 0xFF),
        weight: Weight::NORMAL,
        italic: false,
    };
    let spans = [FlowSpan {
        text: glyph,
        style: &style,
        link: None,
    }];
    let shaped = shape_box(&spans, None, Align::Left, fonts);
    let size = shaped.size;
    let run = GlyphRunData::new(
        RunShape::Box(Arc::new(shaped)),
        Vec2::ZERO,
        GlyphOverrides::none(),
        0.0,
        None,
    )
    .expect("a run with no overrides is always built");
    (run, size)
}
