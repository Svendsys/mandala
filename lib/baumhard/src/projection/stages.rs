// SPDX-License-Identifier: MPL-2.0

//! The stages of a projection (#241 §5.3–5.5): what each node and unit
//! computes, the inputs it records reading, and the phases that order
//! them so each piece of work runs at most once per update.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;

use glam::Vec2;
use rustc_hash::FxHashSet;

use crate::arena::NodeId;
use crate::document::element::{DocElement, DocKind};
use crate::document::ids::Document;
use crate::document::resolve::BorderPoint;
use crate::document::resolve::{
    carry_binding, resolve_frame, resolve_label, resolve_node, resolve_path, resolve_portal_end,
    resolve_ring_in, resolve_text_in, Binding, Content, Input as DocInput, ReadRecorder, ResolvedText,
};
use crate::document::values::{Align, FitMode, FontSize};
use crate::engine::tree::Tree;
use crate::flow::inked as inked_run;
use crate::flow::{
    chain_point, offset as offset_outline, outline, pattern_gap, place_at_point, place_on_closed,
    place_on_open, place_pitched, shape_box, shape_run, FlowSpan, PathEnd, PlacedRun, Rect, RingLayout,
    ShapedRun,
};
use crate::font::fonts::FontSystemLease;
use crate::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunShape};
use crate::projection::geometry::{self, drawn_rect, label_center, points, reach};
use crate::projection::input::{Dependent, ProjectionInput, Reads, Stage};
use crate::projection::shapes::ShapeCache;
use crate::projection::state::{
    Attach, BoxFlow, BoxKey, EndState, JunctionKey, LabelState, OwnFields, PathFlow, Placed, PortalAt,
    RingResolved, RunFlow, RunKey, UnitBody,
};
use crate::projection::structure::element;
use crate::projection::Projector;

/// The phases of an update, in order.
const CARRY: u8 = 0;
const SECTIONS: u8 = 1;
const MEASURE: u8 = 2;
const PLACE: u8 = 3;
const PATHS: u8 = 4;
const FANS: u8 = 5;
const LAY: u8 = 6;
const DONE: u8 = 7;

/// One update's worklists, and what its structural step left for the
/// write.
#[derive(Default)]
pub(crate) struct PhaseWork {
    /// Every piece of work marked and not yet run.
    queued: FxHashSet<Dependent>,
    /// Phase A0: nodes to carry, shallowest first.
    carry: BinaryHeap<Reverse<(usize, NodeId)>>,
    /// Phase A1: sections to resolve and shape.
    to_resolve: Vec<NodeId>,
    /// Phase A2: nodes to measure.
    measure: Vec<NodeId>,
    /// Phase B: sections to place.
    to_place: Vec<NodeId>,
    /// Phase B: frames with rings to project, each once.
    frames: Vec<NodeId>,
    frames_seen: FxHashSet<NodeId>,
    /// Phase C: connectors and links to resolve, and lines whose ends to
    /// find.
    paths: Vec<NodeId>,
    /// Phase C, once every line's ends are found: the junctions whose
    /// members came, went or moved, each fanned once.
    fans: FxHashSet<JunctionKey>,
    /// Phase C, once the junctions are fanned: paths to lay.
    lay: Vec<NodeId>,
    /// The phase running.
    phase: u8,
    /// The nodes this update created state for.
    pub(crate) created: FxHashSet<NodeId>,
    /// The top of each subtree this update made visible, to write whole.
    pub(crate) pending_roots: Vec<NodeId>,
    /// Sections whose box within their node is due: their offset applied
    /// to their shaped size, after every section of the update is shaped
    /// and before any node is measured.
    pub(crate) boxes: Vec<NodeId>,
    /// Kept nodes whose parent changed.
    pub(crate) moves: Vec<NodeId>,
    /// Kept nodes touched in place, whose place in their row may have
    /// changed.
    pub(crate) reorders: Vec<NodeId>,
    /// Kept units touched in place, likewise.
    pub(crate) unit_reorders: Vec<NodeId>,
    /// Every unit some stage of which ran: the write compares their slots.
    pub(crate) units_run: FxHashSet<NodeId>,
    /// Every piece of work run, in order: what the tests of the
    /// dependency index read. Test builds only.
    #[cfg(test)]
    pub(crate) ran: Vec<Dependent>,
}

impl PhaseWork {
    /// Take `dep` off the queue: whether it was on it. A piece of work
    /// taken is logged as run.
    fn take(&mut self, dep: Dependent) -> bool {
        if self.queued.remove(&dep) {
            #[cfg(test)]
            self.ran.push(dep);
            if dep.stage != Stage::Carry && dep.stage != Stage::Geometry {
                self.units_run.insert(dep.unit);
            }
            true
        } else {
            false
        }
    }
}

/// Records every read but those `drop` refuses: for a resolver whose one
/// call answers two stages, each stage keeping the reads of what it uses.
struct Filtered<'a> {
    reads: &'a mut Vec<ProjectionInput>,
    drop: fn(&DocInput) -> bool,
}

impl ReadRecorder for Filtered<'_> {
    fn read(&mut self, input: DocInput) {
        if !(self.drop)(&input) {
            self.reads.push(ProjectionInput::Doc(input));
        }
    }
}

/// `style` with its size in points, a relative one taken of `rect`
/// ([`points`]).
fn in_points(style: ResolvedText, rect: Rect) -> ResolvedText {
    ResolvedText {
        size: FontSize::Pt(points(style.size, rect)),
        ..style
    }
}

/// The flow spans of a key.
fn flow_spans(spans: &[(String, ResolvedText)]) -> Vec<FlowSpan<'_>> {
    spans
        .iter()
        .map(|(text, style)| FlowSpan {
            text,
            style,
            link: None,
        })
        .collect()
}

/// `cache`'s shaping when it was shaped from `key`; otherwise the shaping
/// of an equal key another unit holds (`shapes`); otherwise `key` shaped
/// now. Kept either way. Cost: nothing for an unchanged key; a hash of
/// the key for one another unit shaped; one shaping otherwise.
fn shaped_run(
    cache: &mut Option<RunFlow>,
    key: RunKey,
    shapes: &mut ShapeCache,
    fonts: &mut FontSystemLease,
) -> Arc<ShapedRun> {
    if let Some(flow) = cache {
        if *flow.key == key {
            return flow.shaped.clone();
        }
    }
    // On a hit the cache hands back the key the sharers hold, and this
    // unit keeps that one (`projection::shapes`).
    let mut key = Arc::new(key);
    let shaped = shapes.run(&mut key, |key| {
        memo::run(key, || Arc::new(shape_run(&flow_spans(&key.spans), 1.0, fonts)))
    });
    *cache = Some(RunFlow {
        key,
        shaped: shaped.clone(),
    });
    shaped
}

/// [`shaped_run`] for a box flow.
fn shaped_box(
    cache: &mut Option<BoxFlow>,
    key: BoxKey,
    shapes: &mut ShapeCache,
    fonts: &mut FontSystemLease,
) -> bool {
    if cache.as_ref().is_some_and(|flow| *flow.key == key) {
        return false;
    }
    let mut key = Arc::new(key);
    let shaped = shapes.boxed(&mut key, |key| {
        memo::boxed(key, || {
            Arc::new(shape_box(&flow_spans(&key.spans), key.min, key.align, fonts))
        })
    });
    *cache = Some(BoxFlow { key, shaped });
    true
}

/// Whether two runs placement returned for the same input hold the same
/// glyphs: their metrics, line height and every laid-out glyph.
fn same_run(a: &ShapedRun, b: &ShapedRun) -> bool {
    a.glyphs == b.glyphs && a.line_height == b.line_height && crate::projection::equivalent::same_layout(a, b)
}

/// A placement, its `Arc`s reused where nothing changed: the run placement
/// returned is the last one when it was returned for the same input and
/// holds the same glyphs, and the placement is the last one when its
/// positions are equal. So an unchanged flow writes nothing.
fn settle(
    prev: Option<&Placed>,
    input: &Arc<ShapedRun>,
    replaced: Option<ShapedRun>,
    placed: PlacedRun,
) -> Placed {
    let run = match replaced {
        None => input.clone(),
        Some(run) => match prev {
            Some(p)
                if Arc::ptr_eq(&p.input, input)
                    && !Arc::ptr_eq(&p.run, &p.input)
                    && same_run(&p.run, &run) =>
            {
                p.run.clone()
            }
            _ => Arc::new(run),
        },
    };
    let placed = match prev {
        Some(p) if *p.placed == placed => p.placed.clone(),
        _ => Arc::new(placed),
    };
    Placed {
        input: input.clone(),
        run,
        placed,
    }
}

/// A run of `shape` at `origin`, at `opacity`, in `zoom`.
fn run_data(
    shape: RunShape,
    origin: Vec2,
    opacity: f32,
    zoom: Option<crate::document::values::ZoomWindow>,
) -> Option<GlyphRunData> {
    GlyphRunData::new(shape, origin, GlyphOverrides::none(), opacity, zoom).ok()
}

/// The span children of `id`, in order.
fn spans_of(tree: &Tree<DocElement>, id: NodeId) -> Vec<NodeId> {
    let arena = tree.arena();
    id.children(arena)
        .filter(|&c| arena[c].get().kind() == DocKind::Span)
        .collect()
}

/// The text of a span.
fn span_text(tree: &Tree<DocElement>, span: NodeId) -> String {
    match element(tree, span) {
        Some(DocElement::Span(data)) => data.text.clone(),
        _ => String::new(),
    }
}

/// The spans of `owner` resolved for shaping, each in its own style with
/// `binding`, sizes converted with `rect`: their ids and the key's spans.
/// Records each span's reads and the owner's row.
fn resolve_spans(
    tree: &Tree<DocElement>,
    owner: NodeId,
    binding: Binding,
    rect: Rect,
    reads: &mut Vec<ProjectionInput>,
) -> (Vec<NodeId>, Vec<(String, ResolvedText)>) {
    reads.push(ProjectionInput::Doc(DocInput::Row(owner)));
    let ids = spans_of(tree, owner);
    let spans = ids
        .iter()
        .map(|&span| {
            let style = resolve_text_in(tree, span, binding, &mut Reads(reads));
            (span_text(tree, span), in_points(style, rect))
        })
        .collect();
    (ids, spans)
}

/// A rect of no size: what a flow outside a frame converts a relative
/// size with — none is ever loaded there — so it shapes at the minimum.
const NO_RECT: Rect = Rect::new(Vec2::ZERO, Vec2::ZERO);

impl Projector {
    /// Mark `dep` to run, in the phase its stage and unit belong to.
    /// O(1).
    pub(crate) fn queue(&self, work: &mut PhaseWork, dep: Dependent) {
        let body = self.units.get(&dep.unit).map(|u| &u.body);
        let path = matches!(body, Some(UnitBody::Line(_)) | Some(UnitBody::Portal(_)));
        let phase = match dep.stage {
            Stage::Carry => CARRY,
            // A section's geometry is its box: due in the sections phase,
            // before its node is measured from it.
            Stage::Geometry if matches!(body, Some(UnitBody::Section(_))) => SECTIONS,
            // A path's is where it leaves its nodes, found for every path
            // before any is laid (#296).
            Stage::Geometry if path => PATHS,
            Stage::Geometry => MEASURE,
            Stage::Place if path => LAY,
            _ => match body {
                Some(UnitBody::Section(_)) if dep.stage == Stage::Resolve => SECTIONS,
                Some(UnitBody::Section(_)) | Some(UnitBody::Ring(_)) => PLACE,
                Some(UnitBody::Line(_)) | Some(UnitBody::Portal(_)) => PATHS,
                None => return,
            },
        };
        if phase < work.phase {
            // A stage marked by work of a later phase: the phases are
            // ordered so that this cannot happen.
            log::error!("projection: {dep:?} marked after its phase");
            debug_assert!(false, "projection: {dep:?} marked after its phase");
            return;
        }
        if !work.queued.insert(dep) {
            return;
        }
        match phase {
            CARRY => {
                let depth = self.nodes.get(&dep.unit).map_or(0, |n| n.depth);
                work.carry.push(Reverse((depth, dep.unit)));
            }
            SECTIONS if dep.stage == Stage::Geometry => work.boxes.push(dep.unit),
            SECTIONS => work.to_resolve.push(dep.unit),
            MEASURE => work.measure.push(dep.unit),
            PLACE => match self.units.get(&dep.unit).map(|u| &u.body) {
                Some(UnitBody::Ring(ring)) => {
                    if let Some(frame) = ring.frame {
                        if work.frames_seen.insert(frame) {
                            work.frames.push(frame);
                        }
                    }
                }
                _ => work.to_place.push(dep.unit),
            },
            PATHS => work.paths.push(dep.unit),
            _ => work.lay.push(dep.unit),
        }
    }

    /// Mark every piece of work that read `input`. O(those).
    pub(crate) fn mark(&self, work: &mut PhaseWork, input: ProjectionInput) {
        for &dep in self.deps.dependents(&input) {
            self.queue(work, dep);
        }
    }

    /// Run every phase over what `work` holds marked.
    pub(crate) fn run_phases(&mut self, doc: &Document, work: &mut PhaseWork, fonts: &mut FontSystemLease) {
        let tree = doc.tree();
        work.phase = CARRY;
        while let Some(Reverse((_, id))) = work.carry.pop() {
            if work.take(Dependent::new(id, Stage::Carry)) {
                self.carry(tree, id, work);
            }
        }
        work.phase = SECTIONS;
        while let Some(id) = work.to_resolve.pop() {
            if work.take(Dependent::new(id, Stage::Resolve)) {
                self.section_resolve(tree, id, work, fonts);
            }
        }
        while let Some(id) = work.boxes.pop() {
            if work.take(Dependent::new(id, Stage::Geometry)) {
                self.section_geometry(tree, id, work);
            }
        }
        work.phase = MEASURE;
        while let Some(id) = work.measure.pop() {
            if work.take(Dependent::new(id, Stage::Geometry)) {
                self.measure(tree, id, work);
            }
        }
        work.phase = PLACE;
        while let Some(id) = work.to_place.pop() {
            if work.take(Dependent::new(id, Stage::Place)) {
                self.section_place(id, work);
            }
        }
        let mut at = 0;
        while at < work.frames.len() {
            let frame = work.frames[at];
            at += 1;
            self.frame_rings(tree, frame, work, fonts);
        }
        work.phase = PATHS;
        while let Some(id) = work.paths.pop() {
            if work.take(Dependent::new(id, Stage::Resolve)) {
                self.path_resolve(tree, id, work, fonts);
            }
            if work.take(Dependent::new(id, Stage::Geometry)) {
                self.line_ends(tree, id, work);
            }
        }
        work.phase = FANS;
        for key in std::mem::take(&mut work.fans) {
            self.refan(key, work);
        }
        work.phase = LAY;
        while let Some(id) = work.lay.pop() {
            if work.take(Dependent::new(id, Stage::Place)) {
                self.path_place(tree, id, fonts);
            }
        }
        work.phase = DONE;
    }

    /// Phase A0 for a node: its palette binding, effective opacity and
    /// zoom window, from its own fields and its parent's carried facts
    /// (ruling R16 on #282: carried down, never climbed for). Marks the
    /// readers of each that changed. O(1).
    fn carry(&mut self, tree: &Tree<DocElement>, id: NodeId, work: &mut PhaseWork) {
        let Some(DocElement::Node(data)) = element(tree, id) else {
            return;
        };
        let Some(state) = self.nodes.get(&id) else {
            return;
        };
        let parent = state.parent.and_then(|p| self.nodes.get(&p).map(|s| (p, s)));
        let mut reads = Vec::new();
        let binding = carry_binding(
            tree,
            id,
            parent.and_then(|(_, s)| s.binding),
            &mut Reads(&mut reads),
        );
        let own = own_opacity(data.opacity);
        let opacity = own * parent.map_or(1.0, |(_, s)| s.opacity);
        if let Some((p, _)) = parent {
            reads.push(ProjectionInput::Opacity(p));
        }
        let zoom = data.zoom;
        self.deps.record(Dependent::new(id, Stage::Carry), &reads);
        let Some(state) = self.nodes.get_mut(&id) else {
            return;
        };
        let changed_binding = state.binding != binding;
        let changed_opacity = state.opacity != opacity;
        let changed_zoom = state.zoom != zoom;
        state.carried_from = Some((own, zoom));
        state.binding = binding;
        state.opacity = opacity;
        state.zoom = zoom;
        if changed_binding {
            self.mark(work, ProjectionInput::Doc(DocInput::Binding(id)));
        }
        if changed_opacity {
            self.mark(work, ProjectionInput::Opacity(id));
        }
        if changed_zoom {
            self.mark(work, ProjectionInput::Visibility(id));
        }
    }

    /// Phase A1 for a section: its spans resolved with its node's carried
    /// binding, shaped again only when its shape key changed, and its box
    /// within its node. Marks what read its box when it changed, and its
    /// own placement.
    fn section_resolve(
        &mut self,
        tree: &Tree<DocElement>,
        id: NodeId,
        work: &mut PhaseWork,
        fonts: &mut FontSystemLease,
    ) {
        let Some(DocElement::Section(data)) = element(tree, id) else {
            return;
        };
        let Some(node) = self.units.get(&id).map(|u| u.node) else {
            return;
        };
        let binding = Binding::Carried(self.nodes.get(&node).and_then(|n| n.binding));
        let mut reads = Vec::new();
        let (ids, spans) = resolve_spans(tree, id, binding, NO_RECT, &mut reads);
        let key = BoxKey {
            spans,
            min: data.min_size,
            align: data.align.unwrap_or(Align::Left),
        };
        let own = OwnFields::of(&DocElement::Section(data.clone()));
        self.deps.record(Dependent::new(id, Stage::Resolve), &reads);
        let Some(unit) = self.units.get_mut(&id) else {
            return;
        };
        unit.own = own;
        let UnitBody::Section(state) = &mut unit.body else {
            return;
        };
        shaped_box(&mut state.flow, key, &mut self.shapes, fonts);
        unit.slots[0].element = Some(id);
        unit.slots[0].spans = ids;
        self.queue(work, Dependent::new(id, Stage::Geometry));
        self.queue(work, Dependent::new(id, Stage::Place));
    }

    /// A section's box within its node: its offset, its shaped size. What
    /// a nudge of the section runs, and nothing before it: an offset is
    /// read here, never by its resolution. Marks what read its box, its
    /// node's measure, and its own placement, when the box changed. O(1).
    fn section_geometry(&mut self, tree: &Tree<DocElement>, id: NodeId, work: &mut PhaseWork) {
        let Some(DocElement::Section(data)) = element(tree, id) else {
            return;
        };
        let offset = geometry::usable_point(data.offset);
        let Some(unit) = self.units.get_mut(&id) else {
            return;
        };
        let UnitBody::Section(state) = &mut unit.body else {
            return;
        };
        let size = state.flow.as_ref().map_or(Vec2::ZERO, |f| f.shaped.size);
        let local = Some(Rect::new(offset, size));
        if state.local == local {
            return;
        }
        state.local = local;
        self.mark(work, ProjectionInput::Geometry(id));
        self.queue(work, Dependent::new(id, Stage::Place));
    }

    /// Phase A2 for a node: its shape, and its drawn rectangle from its
    /// minimum size and its sections' boxes (D5), and its outline. Marks
    /// what read its geometry when either changed. O(its sections).
    fn measure(&mut self, tree: &Tree<DocElement>, id: NodeId, work: &mut PhaseWork) {
        let Some(DocElement::Node(data)) = element(tree, id) else {
            return;
        };
        if !self.nodes.contains_key(&id) {
            return;
        }
        let mut reads = vec![ProjectionInput::Doc(DocInput::Row(id))];
        let shape = resolve_node(tree, id, &mut Reads(&mut reads)).shape;
        let arena = tree.arena();
        let sections: Vec<NodeId> = id
            .children(arena)
            .take_while(|&c| arena[c].get().kind() == DocKind::Section)
            .collect();
        let extents: Vec<Vec2> = sections
            .iter()
            .filter_map(|&s| {
                reads.push(ProjectionInput::Geometry(s));
                match &self.units.get(&s)?.body {
                    UnitBody::Section(section) => section.local.map(|r| r.max()),
                    _ => None,
                }
            })
            .collect();
        let rect = drawn_rect(data.position, data.min_size, extents.into_iter());
        self.deps.record(Dependent::new(id, Stage::Geometry), &reads);
        let Some(state) = self.nodes.get_mut(&id) else {
            return;
        };
        if state.rect == rect && state.shape == shape {
            return;
        }
        let resized = state.rect.size != rect.size;
        state.rect = rect;
        state.shape = shape;
        state.outline = outline(rect, shape);
        self.bodies.set(id, rect, geometry::node_shape(shape));
        self.mark(work, ProjectionInput::Geometry(id));
        if resized {
            self.mark(work, ProjectionInput::Size(id));
        }
    }

    /// Phase B for a section: its run at its node's position plus its
    /// offset, in its node's opacity and zoom window.
    fn section_place(&mut self, id: NodeId, _work: &mut PhaseWork) {
        let Some(unit) = self.units.get(&id) else {
            return;
        };
        let node = unit.node;
        let reads = [
            ProjectionInput::Geometry(node),
            ProjectionInput::Opacity(node),
            ProjectionInput::Visibility(node),
        ];
        self.deps.record(Dependent::new(id, Stage::Place), &reads);
        let Some(n) = self.nodes.get(&node) else {
            return;
        };
        let (origin, opacity, zoom) = (n.rect.origin, n.opacity, n.zoom);
        let Some(unit) = self.units.get_mut(&id) else {
            return;
        };
        let UnitBody::Section(state) = &unit.body else {
            return;
        };
        let want = match (&state.flow, state.local) {
            (Some(flow), Some(local)) => run_data(
                RunShape::Box(flow.shaped.clone()),
                origin + local.origin,
                opacity,
                zoom,
            ),
            _ => None,
        };
        unit.slots[0].want = want;
    }

    /// Phase B for a frame: each of its rings that is marked, in order, so
    /// a ring is placed after every ring inside it.
    fn frame_rings(
        &mut self,
        tree: &Tree<DocElement>,
        frame: NodeId,
        work: &mut PhaseWork,
        fonts: &mut FontSystemLease,
    ) {
        if element(tree, frame).is_none() {
            return;
        }
        let rings: Vec<NodeId> = frame.children(tree.arena()).collect();
        for ring in rings {
            if work.take(Dependent::new(ring, Stage::Resolve)) {
                self.ring_resolve(tree, ring, work);
            }
            if work.take(Dependent::new(ring, Stage::Shape)) {
                self.ring_shape(ring, work, fonts);
            }
            if work.take(Dependent::new(ring, Stage::Place)) {
                self.ring_place(tree, ring, work, fonts);
            }
        }
    }

    /// A ring's resolution: its spans' styles and its own text style, with
    /// its node's carried binding; sizes stay relative.
    fn ring_resolve(&mut self, tree: &Tree<DocElement>, id: NodeId, work: &mut PhaseWork) {
        let Some(node) = self.units.get(&id).map(|u| u.node) else {
            return;
        };
        let binding = Binding::Carried(self.nodes.get(&node).and_then(|n| n.binding));
        let mut reads = vec![ProjectionInput::Doc(DocInput::Row(id))];
        let ids = spans_of(tree, id);
        let spans: Vec<(String, ResolvedText)> = ids
            .iter()
            .map(|&span| {
                let style = resolve_text_in(tree, span, binding, &mut Reads(&mut reads));
                (span_text(tree, span), style)
            })
            .collect();
        let text = resolve_text_in(tree, id, binding, &mut Reads(&mut reads));
        self.deps.record(Dependent::new(id, Stage::Resolve), &reads);
        let Some(unit) = self.units.get_mut(&id) else {
            return;
        };
        let UnitBody::Ring(state) = &mut unit.body else {
            return;
        };
        state.spans = spans;
        state.text = Some(text);
        unit.own = OwnFields::Ring;
        unit.slots[0].element = Some(id);
        unit.slots[0].spans = ids;
        self.queue(work, Dependent::new(id, Stage::Shape));
    }

    /// A ring's shaping: its relative sizes in points of its node's drawn
    /// rectangle (#241 amendment 1), shaped again only when that key
    /// changed — a node resized reshapes its rings, a node moved does not.
    fn ring_shape(&mut self, id: NodeId, work: &mut PhaseWork, fonts: &mut FontSystemLease) {
        let Some(unit) = self.units.get(&id) else {
            return;
        };
        let node = unit.node;
        let Some(rect) = self.nodes.get(&node).map(|n| n.rect) else {
            return;
        };
        let UnitBody::Ring(state) = &unit.body else {
            return;
        };
        let relative = state
            .spans
            .iter()
            .any(|(_, s)| matches!(s.size, FontSize::Rel(_)));
        // A relative size is taken of the node's drawn size, never of
        // where it is: a node moved, not resized, shapes nothing.
        let reads: Vec<ProjectionInput> = if relative {
            vec![ProjectionInput::Size(node)]
        } else {
            Vec::new()
        };
        // The key it was shaped from, when it holds these texts at these
        // sizes in points: a resize that keeps `min(w, h)` changes no
        // size, and is found so without cloning a span's text.
        let same_key = state.flow.flow.as_ref().is_some_and(|flow| {
            flow.key.spans.len() == state.spans.len()
                && flow
                    .key
                    .spans
                    .iter()
                    .zip(&state.spans)
                    .all(|((kt, ks), (t, s))| kt == t && *ks == in_points(*s, rect))
        });
        self.deps.record(Dependent::new(id, Stage::Shape), &reads);
        if !same_key {
            #[cfg(test)]
            self.key_builds.push(id);
            let key = RunKey {
                spans: state
                    .spans
                    .iter()
                    .map(|(t, s)| (t.clone(), in_points(*s, rect)))
                    .collect(),
            };
            let Some(unit) = self.units.get_mut(&id) else {
                return;
            };
            let UnitBody::Ring(state) = &mut unit.body else {
                return;
            };
            shaped_run(&mut state.flow.flow, key, &mut self.shapes, fonts);
        }
        // Its placement reads its own text style, which its resolution
        // may have changed with the key unchanged.
        self.queue(work, Dependent::new(id, Stage::Place));
    }

    /// A ring's placement: around its node's outline, grown past the rings
    /// inside it as they were placed, by its frame's and its own layout,
    /// in its node's opacity and zoom window. A ring of a frame that is
    /// not drawn draws nothing. Marks the ring just outside it when its
    /// extent (its line height and those inside it) changed. O(its run),
    /// reading one ring inside it, never all of them.
    fn ring_place(
        &mut self,
        tree: &Tree<DocElement>,
        id: NodeId,
        work: &mut PhaseWork,
        fonts: &mut FontSystemLease,
    ) {
        let Some(unit) = self.units.get(&id) else {
            return;
        };
        let node = unit.node;
        let UnitBody::Ring(state) = &unit.body else {
            return;
        };
        let Some(frame_id) = state.frame else {
            return;
        };
        let Some(n) = self.nodes.get(&node) else {
            return;
        };
        let (rect, node_outline, opacity, zoom, binding) = (n.rect, n.outline, n.opacity, n.zoom, n.binding);
        let mut reads = vec![
            ProjectionInput::Geometry(node),
            ProjectionInput::Opacity(node),
            ProjectionInput::Visibility(node),
            ProjectionInput::Inner(id),
        ];
        let frame = resolve_frame(tree, frame_id, &mut Reads(&mut reads));
        let ring = resolve_ring_in(tree, id, Binding::Carried(binding), &mut Reads(&mut reads));
        // What the rings inside it take: the extent of the nearest ring
        // before it, which carries every ring inside that one.
        let mut inner = (0.0f32, 0u32);
        let mut before = tree.node(id).and_then(|n| n.previous_sibling());
        while let Some(prev) = before {
            if let Some(UnitBody::Ring(r)) = self.units.get(&prev).map(|u| &u.body) {
                reads.push(ProjectionInput::Geometry(prev));
                inner = r.extent.unwrap_or(inner);
                break;
            }
            before = tree.node(prev).and_then(|n| n.previous_sibling());
        }
        let text = state.text.map(|t| in_points(t, rect));
        let resolved = RingResolved {
            frame: frame.clone(),
            ring: ring.clone(),
        };
        let input = state.flow.flow.as_ref().map(|f| f.shaped.clone());
        let prev = state.flow.placed.clone();
        let was_inked = state.inked.clone();
        self.deps.record(Dependent::new(id, Stage::Place), &reads);
        let placed = match (frame.visible, input, text) {
            (true, Some(input), Some(text)) => {
                let path = offset_outline(&node_outline, clearance(frame.inset, frame.gap, inner));
                let layout = RingLayout {
                    span: ring.span,
                    sides: ring.sides.as_ref().map(|s| (s.content.clone(), s.color)),
                    spacing: ring.spacing,
                };
                let params = (&path, frame.start, frame.direction, &ring.fit, &layout, &text);
                let (replaced, placed) = memo::placed(&input, &params, || {
                    place_on_closed(
                        &input,
                        &path,
                        frame.start,
                        frame.direction,
                        &ring.fit,
                        &layout,
                        true,
                        &text,
                        fonts,
                    )
                });
                Some(settle(prev.as_ref(), &input, replaced, placed))
            }
            _ => None,
        };
        let line_height = placed.as_ref().map_or(0.0, |p| p.run.line_height);
        let extent = (inner.0 + line_height, inner.1 + 1);
        // Its inks, what its node's paths stop at (#296): a text ring's
        // run, which placement returns with no ink, inked once a shaping.
        let inked = placed.as_ref().map(|p| {
            if p.run.glyphs.iter().all(|g| g.blank || g.ink.is_some()) {
                return p.run.clone();
            }
            match was_inked {
                Some((from, inked)) if Arc::ptr_eq(&from, &p.run) => inked,
                _ => Arc::new(inked_run(&p.run, fonts.fonts())),
            }
        });
        let inks: Vec<Rect> = match (&placed, &inked) {
            (Some(p), Some(run)) => placed_inks(run, &p.placed).collect(),
            _ => Vec::new(),
        };
        let ink = inks.iter().copied().reduce(|a, b| a.union(&b));
        let Some(unit) = self.units.get_mut(&id) else {
            return;
        };
        let UnitBody::Ring(state) = &mut unit.body else {
            return;
        };
        let changed = state.extent != Some(extent);
        // Every ink, not only their box: a connector's end gap is measured
        // to the nearest one, and a respaced ring moves its inks along a
        // straight side and keeps its box.
        let reach_changed = state.inks != inks;
        state.extent = Some(extent);
        state.ink = ink;
        state.inks = inks;
        state.inked = placed
            .as_ref()
            .zip(inked)
            .map(|(p, inked)| (p.run.clone(), inked));
        state.visible = frame.visible;
        state.resolved = Some(resolved);
        unit.slots[0].want = placed.as_ref().and_then(|p| {
            run_data(
                RunShape::Path {
                    run: p.run.clone(),
                    placed: p.placed.clone(),
                },
                Vec2::ZERO,
                opacity,
                zoom,
            )
        });
        state.flow.placed = placed;
        if changed {
            self.mark(work, ProjectionInput::Geometry(id));
        }
        if reach_changed {
            self.mark(work, ProjectionInput::Reach(node));
        }
    }

    /// Phase C, resolution, for a connector or a link: whether it is
    /// drawn — its `visible`, both its ends drawn — and its flows'
    /// texts, shaped again where their keys changed. A link that switched
    /// kind retires the slots of its old kind.
    fn path_resolve(
        &mut self,
        tree: &Tree<DocElement>,
        id: NodeId,
        work: &mut PhaseWork,
        fonts: &mut FontSystemLease,
    ) {
        let Some(owner) = self.units.get(&id).map(|u| u.node) else {
            return;
        };
        let Some(el) = element(tree, id) else {
            return;
        };
        let portal =
            matches!(el, DocElement::Link(link) if link.kind == crate::document::values::LinkKind::Portal);
        let ends = match el {
            DocElement::Connector(_) => self.nodes.get(&owner).and_then(|n| n.parent).map(|p| (p, owner)),
            DocElement::Link(link) => Some((owner, link.to)),
            _ => None,
        };
        let visible = el.as_path().is_some_and(|p| p.visible);
        let own = OwnFields::of(el);
        if let Some(unit) = self.units.get_mut(&id) {
            unit.own = own;
        }
        let mut reads = Vec::new();
        match (el, ends) {
            // A link reads whether the node it leads to is drawn.
            (DocElement::Link(_), Some((_, to))) => reads.push(ProjectionInput::Visibility(to)),
            // A connector reads which node its own node hangs from, root
            // or not: its own node's parent, never that parent's row.
            (DocElement::Connector(_), _) => reads.push(ProjectionInput::Parent(owner)),
            _ => {}
        }
        let drawn = ends
            .is_some_and(|(from, to)| self.nodes.contains_key(&from) && self.nodes.contains_key(&to))
            && (portal || visible);
        let binding = Binding::Carried(self.nodes.get(&owner).and_then(|n| n.binding));
        // A path not drawn, or of the other kind now, leaves no end at a
        // junction.
        let was_portal = matches!(self.units.get(&id).map(|u| &u.body), Some(UnitBody::Portal(_)));
        if !drawn || was_portal != portal {
            self.leave_junctions(id, work);
        }
        // A link that switched kind takes on the other kind's slots, and
        // its old kind's labels leave the label index.
        let mut old_labels: [Option<NodeId>; 2] = [None, None];
        if let Some(unit) = self.units.get_mut(&id) {
            let is_portal = matches!(unit.body, UnitBody::Portal(_));
            if is_portal != portal {
                old_labels = match &unit.body {
                    UnitBody::Line(line) => [line.label.id, None],
                    UnitBody::Portal(p) => [p.end[0].label.id, p.end[1].label.id],
                    _ => [None, None],
                };
                let body = if portal {
                    UnitBody::Portal(Box::default())
                } else {
                    UnitBody::Line(Box::default())
                };
                unit.rebody(body);
            }
        }
        for label in old_labels.into_iter().flatten() {
            if self.label_hosts.get(&label) == Some(&id) {
                self.label_hosts.remove(&label);
            }
        }
        if !drawn {
            self.deps.record(Dependent::new(id, Stage::Resolve), &reads);
            self.deps.clear(Dependent::new(id, Stage::Geometry));
            self.deps.clear(Dependent::new(id, Stage::Place));
            // Its labels are not drawn either, so they leave the label
            // index: a label of a path not drawn has no position. They
            // keep their ids, so drawing the path again indexes them anew.
            let mut labels: [Option<NodeId>; 2] = [None, None];
            if let Some(unit) = self.units.get_mut(&id) {
                for slot in &mut unit.slots {
                    slot.want = None;
                }
                match &mut unit.body {
                    UnitBody::Line(line) => {
                        line.drawn = false;
                        line.ends = ends;
                        labels = [line.label.id, None];
                    }
                    UnitBody::Portal(p) => {
                        p.drawn = false;
                        p.ends = ends;
                        labels = [p.end[0].label.id, p.end[1].label.id];
                    }
                    _ => {}
                }
            }
            for label in labels.into_iter().flatten() {
                if self.label_hosts.get(&label) == Some(&id) {
                    self.label_hosts.remove(&label);
                }
            }
            return;
        }
        if portal {
            self.portal_resolve(tree, id, ends, binding, reads, fonts);
        } else {
            self.line_resolve(tree, id, ends, binding, reads, fonts);
        }
        self.queue(work, Dependent::new(id, Stage::Geometry));
    }

    /// [`Self::path_resolve`] for a connector or a line link.
    fn line_resolve(
        &mut self,
        tree: &Tree<DocElement>,
        id: NodeId,
        ends: Option<(NodeId, NodeId)>,
        binding: Binding,
        mut reads: Vec<ProjectionInput>,
        fonts: &mut FontSystemLease,
    ) {
        let resolved = resolve_path(tree, id, &mut Reads(&mut reads));
        let text = resolve_text_in(tree, id, binding, &mut Reads(&mut reads));
        let arena = tree.arena();
        let mut caps = [None, None];
        let mut label = None;
        for child in id.children(arena) {
            match arena[child].get().kind() {
                DocKind::CapStart => caps[0] = Some(child),
                DocKind::CapEnd => caps[1] = Some(child),
                DocKind::Label => label = Some(child),
                _ => {}
            }
        }
        let (body_ids, body_spans) = match &resolved.content {
            Content::Spans => resolve_spans(tree, id, binding, NO_RECT, &mut reads),
            Content::Text(content) => (Vec::new(), vec![(content.clone(), in_points(text, NO_RECT))]),
        };
        let cap_flows: Vec<Option<CapSpans>> = caps
            .iter()
            .map(|cap| {
                cap.map(|cap| {
                    let (ids, spans) = resolve_spans(tree, cap, binding, NO_RECT, &mut reads);
                    (cap, ids, spans)
                })
            })
            .collect();
        let label_flow = label.map(|label| {
            let (ids, spans) = resolve_spans(tree, label, binding, NO_RECT, &mut reads);
            (label, ids, spans)
        });
        self.deps.record(Dependent::new(id, Stage::Resolve), &reads);
        let Some(unit) = self.units.get_mut(&id) else {
            return;
        };
        let UnitBody::Line(line) = &mut unit.body else {
            return;
        };
        line.ends = ends;
        line.drawn = true;
        line.resolved = Some((resolved, text));
        line.flows[0].id = Some(id);
        shaped_run(
            &mut line.flows[0].flow,
            RunKey { spans: body_spans },
            &mut self.shapes,
            fonts,
        );
        unit.slots[0].element = Some(id);
        unit.slots[0].spans = body_ids;
        for (k, cap) in cap_flows.into_iter().enumerate() {
            let flow = &mut line.flows[k + 1];
            match cap {
                Some((cap, ids, spans)) => {
                    flow.id = Some(cap);
                    shaped_run(&mut flow.flow, RunKey { spans }, &mut self.shapes, fonts);
                    unit.slots[k + 1].element = Some(cap);
                    unit.slots[k + 1].spans = ids;
                }
                None => {
                    *flow = PathFlow::default();
                    unit.slots[k + 1].element = None;
                    unit.slots[k + 1].spans.clear();
                }
            }
        }
        let old_label = line.label.id;
        match label_flow {
            Some((label, ids, spans)) => {
                line.label.id = Some(label);
                shaped_box(
                    &mut line.label.flow,
                    BoxKey {
                        spans,
                        min: None,
                        align: Align::Center,
                    },
                    &mut self.shapes,
                    fonts,
                );
                unit.slots[3].element = Some(label);
                unit.slots[3].spans = ids;
            }
            None => {
                line.label = LabelState::default();
                unit.slots[3].element = None;
                unit.slots[3].spans.clear();
            }
        }
        let new_label = line.label.id;
        self.relabel(id, old_label, new_label);
    }

    /// Keep the label index in step with a unit's label changing from
    /// `old` to `new`.
    fn relabel(&mut self, host: NodeId, old: Option<NodeId>, new: Option<NodeId>) {
        if old != new {
            if let Some(old) = old {
                self.label_hosts.remove(&old);
            }
        }
        if let Some(new) = new {
            self.label_hosts.insert(new, host);
        }
    }

    /// [`Self::path_resolve`] for a portal link: each end's marker text and
    /// label. A portal end's placement fields are read when it is placed,
    /// never here, and a touch of an end seeds its link's placement alone
    /// (`touch_stage`), so a marker drag resolves no text.
    fn portal_resolve(
        &mut self,
        tree: &Tree<DocElement>,
        id: NodeId,
        ends: Option<(NodeId, NodeId)>,
        binding: Binding,
        mut reads: Vec<ProjectionInput>,
        fonts: &mut FontSystemLease,
    ) {
        reads.push(ProjectionInput::Doc(DocInput::Row(id)));
        let arena = tree.arena();
        let end_ids: Vec<NodeId> = id
            .children(arena)
            .filter(|&c| arena[c].get().kind() == DocKind::PortalEnd)
            .collect();
        type Resolved = (
            NodeId,
            Vec<NodeId>,
            Vec<(String, ResolvedText)>,
            Option<(NodeId, Vec<NodeId>, Vec<(String, ResolvedText)>)>,
        );
        let mut resolved_ends: [Option<Resolved>; 2] = [None, None];
        for &end in &end_ids {
            let k = usize::from(arena[end].get().channel() == 1);
            let content = resolve_portal_end(
                tree,
                end,
                &mut Filtered {
                    reads: &mut reads,
                    drop: |i| matches!(i, DocInput::Placement(_)),
                },
            )
            .content;
            let text = resolve_text_in(tree, end, binding, &mut Reads(&mut reads));
            let (marker_ids, marker_spans) = match content {
                Content::Spans => resolve_spans(tree, end, binding, NO_RECT, &mut reads),
                Content::Text(content) => (Vec::new(), vec![(content, in_points(text, NO_RECT))]),
            };
            let label = end
                .children(arena)
                .find(|&c| arena[c].get().kind() == DocKind::Label)
                .map(|label| {
                    let (ids, spans) = resolve_spans(tree, label, binding, NO_RECT, &mut reads);
                    (label, ids, spans)
                });
            resolved_ends[k] = Some((end, marker_ids, marker_spans, label));
        }
        self.deps.record(Dependent::new(id, Stage::Resolve), &reads);
        let Some(unit) = self.units.get_mut(&id) else {
            return;
        };
        let UnitBody::Portal(portal) = &mut unit.body else {
            return;
        };
        portal.ends = ends;
        portal.drawn = true;
        let mut relabels = Vec::new();
        for (k, resolved) in resolved_ends.into_iter().enumerate() {
            let state: &mut EndState = &mut portal.end[k];
            let old_label = state.label.id;
            match resolved {
                Some((end, marker_ids, marker_spans, label)) => {
                    state.marker.id = Some(end);
                    shaped_run(
                        &mut state.marker.flow,
                        RunKey { spans: marker_spans },
                        &mut self.shapes,
                        fonts,
                    );
                    unit.slots[2 * k].element = Some(end);
                    unit.slots[2 * k].spans = marker_ids;
                    match label {
                        Some((label, ids, spans)) => {
                            state.label.id = Some(label);
                            shaped_box(
                                &mut state.label.flow,
                                BoxKey {
                                    spans,
                                    min: None,
                                    align: Align::Center,
                                },
                                &mut self.shapes,
                                fonts,
                            );
                            unit.slots[2 * k + 1].element = Some(label);
                            unit.slots[2 * k + 1].spans = ids;
                        }
                        None => {
                            state.label = LabelState::default();
                            unit.slots[2 * k + 1].element = None;
                            unit.slots[2 * k + 1].spans.clear();
                        }
                    }
                }
                None => {
                    *state = EndState::default();
                    unit.slots[2 * k].element = None;
                    unit.slots[2 * k + 1].element = None;
                }
            }
            relabels.push((old_label, portal.end[k].label.id));
        }
        for (old, new) in relabels {
            self.relabel(id, old, new);
        }
    }

    /// Phase C, placement, for a drawn connector or link.
    fn path_place(&mut self, tree: &Tree<DocElement>, id: NodeId, fonts: &mut FontSystemLease) {
        match self.units.get(&id).map(|u| &u.body) {
            Some(UnitBody::Line(line)) if line.drawn => self.line_place(tree, id, fonts),
            Some(UnitBody::Portal(portal)) if portal.drawn => self.portal_place(tree, id),
            _ => {}
        }
    }

    /// Phase C for a connector's or line link's geometry (#296): where it
    /// leaves each of its nodes — the side of the node's reach box its
    /// `anchors` name, or that faces the other end; the side's midpoint
    /// and normal — and how far it must turn from a neighbor leaving the
    /// same point. Marks the junctions an end moved at, so the paths
    /// that share them are laid again, and queues its own lay. O(1).
    fn line_ends(&mut self, tree: &Tree<DocElement>, id: NodeId, work: &mut PhaseWork) {
        let Some(anchors) = element(tree, id).and_then(DocElement::as_path).map(|p| p.anchors) else {
            return;
        };
        let Some(UnitBody::Line(line)) = self.units.get(&id).map(|u| &u.body) else {
            self.portal_ends(tree, id, work);
            return;
        };
        let Some((from, to)) = line.ends.filter(|_| line.drawn) else {
            return;
        };
        self.deps.record(
            Dependent::new(id, Stage::Geometry),
            &[
                ProjectionInput::Geometry(from),
                ProjectionInput::Geometry(to),
                ProjectionInput::Reach(from),
                ProjectionInput::Reach(to),
            ],
        );
        let (Some(a), Some(b)) = (self.reach(from), self.reach(to)) else {
            return;
        };
        // How far apart two paths' first glyphs must leave one point to
        // clear each other by this one's own gap, and how far its first two
        // glyphs reach, which leave along its side before it bends.
        let (gap, size) = line.flows[0].flow.as_ref().map_or((0.0, 0.0), |f| {
            let spacing = line.resolved.as_ref().map_or(0.0, |(r, _)| r.spacing);
            pattern_gap(&f.shaped, spacing).unwrap_or((spacing, f.shaped.line_height))
        });
        let (spread, lead) = (geometry::spread(gap, size), 2.0 * (gap + size));
        let ends = [(from, a, anchors.0, b), (to, b, anchors.1, a)];
        let mut attach = ends.map(|(node, own, anchor, other)| {
            let side = geometry::side_toward(own, anchor, other.center());
            let (point, normal) = geometry::side_point(own, side);
            Attach {
                node,
                side,
                point,
                normal,
                toward: 0.0,
                spread,
                lead,
            }
        });
        attach[0].toward = geometry::turn_to(attach[0].normal, attach[1].point - attach[0].point);
        attach[1].toward = geometry::turn_to(attach[1].normal, attach[0].point - attach[1].point);
        let Some(UnitBody::Line(line)) = self.units.get_mut(&id).map(|u| &mut u.body) else {
            return;
        };
        let old = std::mem::replace(&mut line.attach, attach.map(Some));
        let key = |a: &Attach| (a.node, Some(a.side));
        for (was, now) in old.into_iter().zip(attach) {
            if was == Some(now) {
                continue;
            }
            // The side it left, unless its other end is there now.
            if let Some(was) = was.filter(|w| attach.iter().all(|a| key(a) != key(w))) {
                self.unjoin(key(&was), id, work);
            }
            self.join(key(&now), id, work);
        }
        self.queue(work, Dependent::new(id, Stage::Place));
    }

    /// Take the path `id` out of every junction it has an end at, each to
    /// be fanned again. O(the paths there).
    pub(crate) fn leave_junctions(&mut self, id: NodeId, work: &mut PhaseWork) {
        let keys: Vec<JunctionKey> = match self.units.get_mut(&id).map(|u| &mut u.body) {
            Some(UnitBody::Line(line)) => std::mem::take(&mut line.attach)
                .iter()
                .flatten()
                .map(|a| (a.node, Some(a.side)))
                .collect(),
            Some(UnitBody::Portal(portal)) => std::mem::take(&mut portal.at)
                .iter()
                .flatten()
                .map(|a| (a.node, None))
                .collect(),
            _ => return,
        };
        for key in keys {
            self.unjoin(key, id, work);
        }
    }

    /// Take the path `id` out of the junction `key`, to be fanned again.
    /// O(the paths there).
    pub(crate) fn unjoin(&mut self, key: JunctionKey, id: NodeId, work: &mut PhaseWork) {
        if let Some(joined) = self.junctions.get_mut(&key) {
            joined.retain(|&p| p != id);
            if joined.is_empty() {
                self.junctions.remove(&key);
            }
        }
        work.fans.insert(key);
    }

    /// Put the path `id` in the junction `key`, to be fanned again.
    /// O(the paths there).
    fn join(&mut self, key: JunctionKey, id: NodeId, work: &mut PhaseWork) {
        let joined = self.junctions.entry(key).or_default();
        if !joined.contains(&id) {
            joined.push(id);
        }
        work.fans.insert(key);
    }

    /// The box `node`'s paths attach to (#296): the box round its drawn
    /// rings' inks, or its drawn rectangle where it draws no ring. `None`
    /// for a node not drawn. O(its units).
    fn reach(&self, node: NodeId) -> Option<Rect> {
        let n = self.nodes.get(&node)?;
        let rings = n
            .units
            .iter()
            .filter_map(|u| match self.units.get(u).map(|u| &u.body) {
                Some(UnitBody::Ring(ring)) => ring.ink,
                _ => None,
            });
        Some(rings.reduce(|a, b| a.union(&b)).unwrap_or(n.rect))
    }

    /// The boxes a path's end glyph at `node` keeps its gap from: every
    /// drawn ring's inks, or its drawn rectangle where it draws no ring.
    /// O(its rings' glyphs).
    fn reach_inks(&self, node: NodeId) -> Vec<Rect> {
        let Some(n) = self.nodes.get(&node) else {
            return Vec::new();
        };
        let mut inks = Vec::new();
        for u in &n.units {
            if let Some(UnitBody::Ring(ring)) = self.units.get(u).map(|u| &u.body) {
                inks.extend_from_slice(&ring.inks);
            }
        }
        if inks.is_empty() {
            inks.push(n.rect);
        }
        inks
    }

    /// How far a path's end glyph at `attach` may move back past the
    /// side's midpoint to keep its gap: as far as the reach box lies past
    /// the node's drawn rectangle on that side — through its frame, never
    /// into the node. O(1).
    fn back(&self, attach: &Attach) -> f32 {
        let Some(rect) = self.nodes.get(&attach.node).map(|n| n.rect) else {
            return 0.0;
        };
        let inner = geometry::side_point(rect, attach.side).0;
        (attach.point - inner).dot(attach.normal).max(0.0)
    }

    /// Phase C, between the ends and the lays: fan the junction `key` once
    /// (#296, rule 4). The paths that leave one side of a node are ordered
    /// by their turn toward their other ends and given their turns and
    /// holds ([`geometry::fan`]); the portal ends with no `border_t` at a
    /// node, ordered round its outline, are held apart there
    /// ([`geometry::held_apart`]). Each member whose value changed is
    /// queued to be laid again, and no other: a drag that keeps the order
    /// lays the dragged path alone. O(the members, log them).
    fn refan(&mut self, key: JunctionKey, work: &mut PhaseWork) {
        let Some(members) = self.junctions.get(&key) else {
            return;
        };
        let at = |p: &NodeId| self.units.get(p).map(|u| &u.body);
        let mut changed: Vec<NodeId> = Vec::new();
        match key.1 {
            Some(side) => {
                // (toward, path, end, spread, lead)
                let mut fan: Vec<(f32, NodeId, usize, f32, f32)> = members
                    .iter()
                    .filter_map(|p| match at(p) {
                        Some(UnitBody::Line(line)) => line.attach.iter().enumerate().find_map(|(k, a)| {
                            a.filter(|a| a.node == key.0 && a.side == side)
                                .map(|a| (a.toward, *p, k, a.spread, a.lead))
                        }),
                        _ => None,
                    })
                    .collect();
                fan.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                let turns = geometry::fan(&fan.iter().map(|m| (m.3, m.4)).collect::<Vec<_>>());
                for (m, value) in fan.iter().zip(turns) {
                    if let Some(UnitBody::Line(line)) = self.units.get_mut(&m.1).map(|u| &mut u.body) {
                        if line.fans[m.2] != value {
                            line.fans[m.2] = value;
                            changed.push(m.1);
                        }
                    }
                }
            }
            None => {
                // (along, path, end, width, perimeter)
                let mut ends: Vec<(f32, NodeId, usize, f32, f32)> = members
                    .iter()
                    .filter_map(|p| match at(p) {
                        Some(UnitBody::Portal(portal)) => portal.at.iter().enumerate().find_map(|(k, a)| {
                            a.filter(|a| a.node == key.0)
                                .map(|a| (a.along, *p, k, a.width, a.perimeter))
                        }),
                        _ => None,
                    })
                    .collect();
                ends.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                let perimeter = ends.first().map_or(0.0, |e| e.4);
                let held =
                    geometry::held_apart(&ends.iter().map(|e| (e.0, e.3)).collect::<Vec<_>>(), perimeter);
                for (e, along) in ends.iter().zip(held) {
                    if let Some(UnitBody::Portal(portal)) = self.units.get_mut(&e.1).map(|u| &mut u.body) {
                        if portal.held[e.2] != along {
                            portal.held[e.2] = along;
                            changed.push(e.1);
                        }
                    }
                }
            }
        }
        for id in changed {
            self.queue(work, Dependent::new(id, Stage::Place));
        }
    }

    /// Phase C, a connector's or line link's lay: its curves from end to
    /// end, each end leaving its side turned by its fan (#296), through
    /// its control points; its body along them — an ornament `repeat` as
    /// whole glyphs gapped evenly from frame to frame, anything else as its
    /// fit mode walks it — its caps at its ends, its label at its position
    /// along it.
    fn line_place(&mut self, tree: &Tree<DocElement>, id: NodeId, fonts: &mut FontSystemLease) {
        let Some(path) = element(tree, id).and_then(DocElement::as_path).cloned() else {
            return;
        };
        let Some(unit) = self.units.get(&id) else {
            return;
        };
        let owner = unit.node;
        let UnitBody::Line(line) = &unit.body else {
            return;
        };
        let (Some((from, to)), [Some(start), Some(end)]) = (line.ends, line.attach) else {
            return;
        };
        let Some(opacity) = self.nodes.get(&owner).map(|o| o.opacity) else {
            return;
        };
        let mut reads = vec![ProjectionInput::Opacity(owner)];
        let fans = line.fans;
        let turned = |a: &Attach, turn: f32| Vec2::from_angle(turn).rotate(a.normal);
        let curves = geometry::path_through(
            (start.point, turned(&start, fans[0].0), start.lead + fans[0].1),
            (end.point, turned(&end, fans[1].0), end.lead + fans[1].1),
            &path.control_points,
        );
        let label_at = line
            .label
            .id
            .map(|label| resolve_label(tree, label, &mut Reads(&mut reads)));
        self.deps.record(Dependent::new(id, Stage::Place), &reads);
        let Some((resolved, text)) = line.resolved.clone() else {
            return;
        };
        let flows = line.flows.clone();
        let label_size = line
            .label
            .flow
            .as_ref()
            .map(|f| (f.shaped.clone(), f.shaped.size));
        let mut placed: [Option<Placed>; 3] = [None, None, None];
        if let Some(input) = flows[0].flow.as_ref().map(|f| f.shaped.clone()) {
            let pitched = resolved.fit == FitMode::Repeat && pattern_gap(&input, resolved.spacing).is_some();
            let (replaced, p) = if pitched {
                let inks = [self.reach_inks(from), self.reach_inks(to)];
                let backs = [self.back(&start), self.back(&end)];
                let params = (&curves, resolved.spacing, &inks, backs, fans);
                memo::placed(&input, &params, || {
                    let ends = [0, 1].map(|k| PathEnd {
                        inks: &inks[k],
                        back: backs[k],
                        hold: fans[k].1,
                    });
                    (None, place_pitched(&input, &curves, resolved.spacing, ends))
                })
            } else {
                let params = (&curves, &resolved.fit, resolved.spacing, &text);
                memo::placed(&input, &params, || {
                    place_on_open(&input, &curves, &resolved.fit, resolved.spacing, &text, fonts)
                })
            };
            placed[0] = Some(settle(flows[0].placed.as_ref(), &input, replaced, p));
        }
        for (k, at) in [(1, 0.0), (2, 1.0)] {
            if flows[k].id.is_none() {
                continue;
            }
            if let (Some(input), Some(point)) = (
                flows[k].flow.as_ref().map(|f| f.shaped.clone()),
                chain_point(&curves, at),
            ) {
                let p = place_at_point(&input, point);
                placed[k] = Some(settle(flows[k].placed.as_ref(), &input, None, p));
            }
        }
        let label_center_at = label_at.map(|label| label_center(&curves, label.t, label.offset));
        let Some(unit) = self.units.get_mut(&id) else {
            return;
        };
        let UnitBody::Line(line) = &mut unit.body else {
            return;
        };
        line.path = curves;
        for (k, placed) in placed.iter_mut().enumerate() {
            unit.slots[k].want = placed.as_ref().and_then(|p| {
                run_data(
                    RunShape::Path {
                        run: p.run.clone(),
                        placed: p.placed.clone(),
                    },
                    Vec2::ZERO,
                    opacity,
                    path.zoom,
                )
            });
            line.flows[k].placed = placed.take();
        }
        if let Some(label) = label_at {
            line.label.t = label.t;
            line.label.offset = label.offset;
        }
        line.label.center = label_center_at;
        unit.slots[3].want = match (label_center_at, label_size) {
            (Some(center), Some((shaped, size))) => {
                run_data(RunShape::Box(shaped), center - size * 0.5, opacity, path.zoom)
            }
            _ => None,
        };
    }

    /// Phase C for a portal link's geometry: where along its node's
    /// outline each end with no `border_t` faces the other node, and how
    /// wide its marker and label are, so the ends that face one way at a
    /// node are held apart (#296). Marks the junctions an end moved at,
    /// and queues its own lay. Cost: an outline walk and a nearest-point
    /// search an end.
    fn portal_ends(&mut self, tree: &Tree<DocElement>, id: NodeId, work: &mut PhaseWork) {
        let Some(UnitBody::Portal(portal)) = self.units.get(&id).map(|u| &u.body) else {
            return;
        };
        let Some((from, to)) = portal.ends.filter(|_| portal.drawn) else {
            return;
        };
        let (Some(a), Some(b)) = (self.nodes.get(&from), self.nodes.get(&to)) else {
            return;
        };
        let mut reads = vec![ProjectionInput::Geometry(from), ProjectionInput::Geometry(to)];
        let sides = [
            (from, a.outline, b.rect.center()),
            (to, b.outline, a.rect.center()),
        ];
        let mut at: [Option<PortalAt>; 2] = [None, None];
        for (k, (node, outline, toward)) in sides.into_iter().enumerate() {
            let end = &portal.end[k];
            let Some(end_id) = end.marker.id else {
                continue;
            };
            let border = resolve_portal_end(
                tree,
                end_id,
                &mut Filtered {
                    reads: &mut reads,
                    drop: |i| matches!(i, DocInput::Style(_) | DocInput::Row(_)),
                },
            )
            .border_t;
            if border != BorderPoint::Auto {
                continue;
            }
            let marker = end.marker.flow.as_ref().map_or(Vec2::ZERO, |f| {
                Vec2::new(
                    f.shaped.glyphs.iter().map(|g| g.advance).sum(),
                    f.shaped.line_height,
                )
            });
            let label = end.label.flow.as_ref().map_or(Vec2::ZERO, |f| f.shaped.size);
            let (along, perimeter) = geometry::portal_along(&outline, border, toward);
            at[k] = Some(PortalAt {
                node,
                along,
                perimeter,
                width: marker.max(label).max_element(),
            });
        }
        self.deps.record(Dependent::new(id, Stage::Geometry), &reads);
        let Some(UnitBody::Portal(portal)) = self.units.get_mut(&id).map(|u| &mut u.body) else {
            return;
        };
        let old = std::mem::replace(&mut portal.at, at);
        for node in [from, to] {
            let was = old.iter().flatten().find(|e| e.node == node);
            let now = at.iter().flatten().find(|e| e.node == node);
            if was == now {
                continue;
            }
            match now {
                Some(_) => self.join((node, None), id, work),
                None => self.unjoin((node, None), id, work),
            }
        }
        self.queue(work, Dependent::new(id, Stage::Place));
    }

    /// A portal link's placement: each end's marker on its node's outline,
    /// where its `border_t` says, or facing the other node, held apart
    /// from the other ends that face one way there; pushed off the outline
    /// by its offset and its own reach; its label beside it.
    fn portal_place(&mut self, tree: &Tree<DocElement>, id: NodeId) {
        let Some(unit) = self.units.get(&id) else {
            return;
        };
        let owner = unit.node;
        let UnitBody::Portal(portal) = &unit.body else {
            return;
        };
        let Some((from, to)) = portal.ends else {
            return;
        };
        let (Some(a), Some(b), Some(o)) =
            (self.nodes.get(&from), self.nodes.get(&to), self.nodes.get(&owner))
        else {
            return;
        };
        let (opacity, zoom) = (o.opacity, o.zoom);
        let mut reads = vec![
            ProjectionInput::Geometry(from),
            ProjectionInput::Geometry(to),
            ProjectionInput::Opacity(owner),
            ProjectionInput::Visibility(owner),
        ];
        let sides = [(a.outline, b.rect.center()), (b.outline, a.rect.center())];
        let mut wants: [Option<GlyphRunData>; 4] = [None, None, None, None];
        let mut placed_markers: [Option<Placed>; 2] = [None, None];
        let mut centers: [(Option<Vec2>, Option<Vec2>); 2] = [(None, None); 2];
        for k in 0..2 {
            let end = &portal.end[k];
            let Some(end_id) = end.marker.id else {
                continue;
            };
            let resolved = resolve_portal_end(
                tree,
                end_id,
                &mut Filtered {
                    reads: &mut reads,
                    drop: |i| matches!(i, DocInput::Style(_) | DocInput::Row(_)),
                },
            );
            let (own_outline, toward) = sides[k];
            let along = match &portal.at[k] {
                Some(_) => portal.held[k],
                None => geometry::portal_along(&own_outline, resolved.border_t, toward).0,
            };
            let (point, normal) = geometry::portal_point(&own_outline, along, resolved.offset);
            let Some(input) = end.marker.flow.as_ref().map(|f| f.shaped.clone()) else {
                continue;
            };
            let marker_size = Vec2::new(input.glyphs.iter().map(|g| g.advance).sum(), input.line_height);
            let marker_reach = reach(normal, marker_size);
            let center = point + normal * marker_reach;
            let p = settle(
                end.marker.placed.as_ref(),
                &input,
                None,
                place_at_point(&input, center),
            );
            wants[2 * k] = run_data(
                RunShape::Path {
                    run: p.run.clone(),
                    placed: p.placed.clone(),
                },
                Vec2::ZERO,
                opacity,
                zoom,
            );
            placed_markers[k] = Some(p);
            centers[k].0 = Some(center);
            if let (Some(label), Some(flow)) = (end.label.id, end.label.flow.as_ref()) {
                let label_offset = resolve_label(tree, label, &mut Reads(&mut reads)).offset;
                let label_offset = if label_offset.is_finite() {
                    label_offset
                } else {
                    0.0
                };
                let size = flow.shaped.size;
                let at = center + normal * (marker_reach + label_offset + reach(normal, size));
                wants[2 * k + 1] =
                    run_data(RunShape::Box(flow.shaped.clone()), at - size * 0.5, opacity, zoom);
                centers[k].1 = Some(at);
            }
        }
        self.deps.record(Dependent::new(id, Stage::Place), &reads);
        let Some(unit) = self.units.get_mut(&id) else {
            return;
        };
        let UnitBody::Portal(portal) = &mut unit.body else {
            return;
        };
        for (k, want) in wants.into_iter().enumerate() {
            unit.slots[k].want = want;
        }
        for k in 0..2 {
            portal.end[k].marker.placed = placed_markers[k].take();
            portal.end[k].center = centers[k].0;
            portal.end[k].label.center = centers[k].1;
        }
    }
}

/// A node's own opacity as its carry reads it: clamped to `0..=1`, and
/// `1` for a value that is not finite.
pub(crate) fn own_opacity(opacity: f32) -> f32 {
    if opacity.is_finite() {
        opacity.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// The canvas box of each glyph of `placed` that draws, by `run`'s
/// metrics: its ink, or where none was measured its box, its advance by
/// its line height. A space draws nothing. O(its glyphs).
fn placed_inks<'a>(run: &'a ShapedRun, placed: &'a PlacedRun) -> impl Iterator<Item = Rect> + 'a {
    let baseline = run.baseline();
    placed.glyphs.iter().filter_map(move |g| {
        let metrics = run.glyphs.get(g.glyph as usize)?;
        if metrics.blank {
            return None;
        }
        Some(match metrics.ink {
            Some(ink) => Rect::new(g.pos + Vec2::new(0.0, baseline) + ink.min, ink.max - ink.min),
            None => Rect::new(g.pos, Vec2::new(metrics.advance, run.line_height)),
        })
    })
}

/// A cap of a line, resolved: its element, its spans' ids, and their
/// texts and styles.
type CapSpans = (NodeId, Vec<NodeId>, Vec<(String, ResolvedText)>);

/// `flow::ring_offset` over a ring's inner extent: `inset`, the summed line
/// heights of the rings inside it, and a `gap` after each of them. The
/// same arithmetic in the same order — the sum is a prefix kept ring by
/// ring from the innermost — so the offset is the one the slice form
/// gives.
fn clearance(inset: f32, gap: f32, (heights, rings): (f32, u32)) -> f32 {
    inset + heights + rings as f32 * gap
}

/// The oracle's shaping memo (#241, Tests). A test's check compares the
/// layers an update kept with a full projection of the same document,
/// after every change, and a full projection shapes every flow: in a
/// debug build that shaping is most of the suite's time. Shaping is a
/// pure function of the shape key, and the oracle compares shapes by
/// value, so the full projection a check makes may share the shaping of
/// an equal key it shaped before (`memo::sharing`). Within one projector
/// the shaping cache (`projection::shapes`, #304) shares equal keys in
/// every build; the memo is the same sharing across projectors, which
/// only the oracle's fresh full projections want, and the tests of the
/// projector's reuse (pointer-equal `Arc`s) never run inside `sharing`.
/// Outside the test build the memo is not there: a key the cache does
/// not hold is shaped.
pub(crate) mod memo {
    use std::sync::Arc;

    use crate::flow::{ShapedBox, ShapedRun};
    use crate::projection::state::{BoxKey, RunKey};

    /// Run `f` with equal keys sharing their shaping on this thread; a
    /// `sharing` inside another leaves it on when it returns.
    #[cfg(test)]
    pub(crate) fn sharing<R>(f: impl FnOnce() -> R) -> R {
        let was = shared::ON.with(|on| on.replace(true));
        let out = f();
        shared::ON.with(|on| on.set(was));
        out
    }

    /// The shaping of `key`: `shape`'s, or inside `sharing` an equal
    /// key's.
    pub(crate) fn run(key: &RunKey, shape: impl FnOnce() -> Arc<ShapedRun>) -> Arc<ShapedRun> {
        #[cfg(test)]
        if shared::ON.with(|on| on.get()) {
            let name = format!("{key:?}");
            if let Some(hit) = shared::RUNS.with(|m| m.borrow().get(&name).cloned()) {
                return hit;
            }
            let shaped = shape();
            shared::RUNS.with(|m| m.borrow_mut().insert(name, shaped.clone()));
            return shaped;
        }
        let _ = key;
        shape()
    }

    /// A placement of `input` with `params`: `place`'s, or inside
    /// `sharing` that of an equal placement of the very same input.
    pub(crate) fn placed<P: std::fmt::Debug>(
        input: &Arc<ShapedRun>,
        params: &P,
        place: impl FnOnce() -> (Option<ShapedRun>, crate::flow::PlacedRun),
    ) -> (Option<ShapedRun>, crate::flow::PlacedRun) {
        #[cfg(test)]
        if shared::ON.with(|on| on.get()) {
            let name = format!("{:p} {params:?}", Arc::as_ptr(input));
            let hit = shared::PLACED.with(|m| {
                m.borrow()
                    .get(&name)
                    .filter(|(kept, _, _)| Arc::ptr_eq(kept, input))
                    .map(|(_, run, placed)| (run.clone(), placed.clone()))
            });
            if let Some(hit) = hit {
                return hit;
            }
            let out = place();
            shared::PLACED.with(|m| {
                m.borrow_mut()
                    .insert(name, (input.clone(), out.0.clone(), out.1.clone()))
            });
            return out;
        }
        let _ = (input, params);
        place()
    }

    /// [`run`] for a box.
    pub(crate) fn boxed(key: &BoxKey, shape: impl FnOnce() -> Arc<ShapedBox>) -> Arc<ShapedBox> {
        #[cfg(test)]
        if shared::ON.with(|on| on.get()) {
            let name = format!("{key:?}");
            if let Some(hit) = shared::BOXES.with(|m| m.borrow().get(&name).cloned()) {
                return hit;
            }
            let shaped = shape();
            shared::BOXES.with(|m| m.borrow_mut().insert(name, shaped.clone()));
            return shaped;
        }
        let _ = key;
        shape()
    }

    #[cfg(test)]
    mod shared {
        use std::cell::{Cell, RefCell};
        use std::sync::Arc;

        use rustc_hash::FxHashMap;

        use crate::flow::{ShapedBox, ShapedRun};

        /// A placement kept: the shaping it was of, the run it returned
        /// and its positions.
        type Kept = (Arc<ShapedRun>, Option<ShapedRun>, crate::flow::PlacedRun);

        thread_local! {
            pub(super) static ON: Cell<bool> = const { Cell::new(false) };
            pub(super) static RUNS: RefCell<FxHashMap<String, Arc<ShapedRun>>> = RefCell::default();
            pub(super) static BOXES: RefCell<FxHashMap<String, Arc<ShapedBox>>> = RefCell::default();
            pub(super) static PLACED: RefCell<FxHashMap<String, Kept>> = RefCell::default();
        }
    }
}
