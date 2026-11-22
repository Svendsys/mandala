// SPDX-License-Identifier: MPL-2.0

//! **Live interaction on transactions** (design §9, D28; V1-C7 #248): a
//! drag that edits the map is a [writer transaction](baumhard::document::writer::transaction),
//! and what the window draws while it runs is the document itself.
//!
//! ```text
//! press ─▶ threshold: begin ─▶ every frame: stage(start + total) ─▶ release: commit (one entry)
//!                                                                 └▶ Escape:  cancel (as before)
//! ```
//!
//! - **The press, then the threshold**: a press past the drag threshold
//!   resolves what it grabbed (`event_cursor_moved`), records the
//!   values the drag starts from — the moved nodes' positions, a box's
//!   drawn rectangle, a path's control points and anchors — and opens an
//!   interactive transaction ([`Live::begin`]). An interactive
//!   transaction already open — a frame preview, an editor — refuses it
//!   (D28): nothing is committed, no drag starts, and the status line
//!   says why.
//! - **Every cursor move** updates the drag's total since the press, and
//!   nothing else ([`Live::moved`]): input never writes the document. A
//!   label and a portal end are moved by it too, from where they were
//!   drawn at the press — what the press grabbed goes where the pointer
//!   goes, at the place on it the press held, so a press that does not
//!   move moves nothing.
//! - **Once per frame** the drain stages the request for that total
//!   ([`Live::drain`]): built from the values recorded at the press, so
//!   each stage is absolute from the press and never accumulates, and
//!   staged on the one transaction, which reverts the previous stage
//!   first. Under load the stage runs every `n`th frame
//!   ([`MutationFrequencyThrottle`]), as `main`'s drags did; under a
//!   virtual clock, every frame.
//! - **The release** stages what is still pending and commits: one
//!   history entry ([`Live::release`]). **Escape**, and a lost focus,
//!   cancel: the document is exactly as it was ([`Live::cancel`]).
//!   **Undo** during a drag cancels its transaction (the writer's ruling
//!   R36), and the drag ends at its next frame.
//!
//! There is no preview of any kind: no slot, no overlay that imitates a
//! change, no glyph written directly. The stage goes through the
//! projector and the renderer like every change, so a presentation
//! playing on a dragged node goes on playing on top of it (§10.2).
//!
//! Native-only, with the drag-state machine (CLAUDE.md "Dual-target
//! status").

#![cfg(not(target_arch = "wasm32"))]

pub(in crate::application::app) mod grab;
#[cfg(test)]
mod tests;

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, DocKind};
use baumhard::document::ids::Document;
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{DocMutation, LabelOp, NodeOp, PathOp, PortalEndOp, SectionOp};
use baumhard::document::mutation::step::DocStep;
use baumhard::document::values::Anchor;
use baumhard::document::writer::transaction::TransactionKind;
use baumhard::document::writer::{ChangeSource, EditRequest, Revision};
use baumhard::flow::Rect;
use baumhard::projection::Projector;
use glam::Vec2;

use crate::application::document::{Interaction, MindMapDocument};
use crate::application::frame_throttle::MutationFrequencyThrottle;
use crate::application::platform::input::MouseButton;
use crate::application::view::handles::{PathHandle, ResizeSide};

/// A drag's perpendicular offset nearer its path or outline than this,
/// in canvas units, is written as none — the flush default — so a label
/// or an end shaken back onto its line loses its offset, as on `main`.
const OFFSET_SNAP: f32 = 0.5;

/// What a drag edits, with everything it was recorded with at the press.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::application::app) enum Gesture {
    /// Nodes moved by the drag's travel: each with its position at the
    /// press. A moved node's descendants are moved with it unless the
    /// drag was alone (Alt), as on `main`.
    MoveNodes {
        /// Every node moved, each once, with where it started.
        nodes: Vec<(NodeId, Vec2)>,
    },
    /// A section moved within its node: its offset at the press.
    MoveSection {
        /// The section.
        section: NodeId,
        /// Its offset at the press.
        start: Vec2,
    },
    /// A node resized by one of its handles.
    ResizeNode {
        /// The node.
        node: NodeId,
        /// The handle.
        side: ResizeSide,
        /// Its drawn rectangle at the press (D5: its size, not its
        /// minimum, is what the handle is on).
        start: Rect,
        /// The size it cannot draw below — its sections' extent — which
        /// the resize holds its minimum to, so a top or left handle
        /// moves the node only as far as it shrinks.
        floor: Vec2,
    },
    /// A section resized by one of its handles.
    ResizeSection {
        /// The section.
        section: NodeId,
        /// The handle.
        side: ResizeSide,
        /// Its box at the press, in its node: its offset and its drawn
        /// size.
        start: Rect,
        /// The size of its text, which its box cannot draw below.
        floor: Vec2,
    },
    /// One of a path's handles.
    PathHandle {
        /// The connector or line link.
        path: NodeId,
        /// Its kind: what its path ops are written to.
        kind: DocKind,
        /// The handle: a midpoint grabbed is the one control point it
        /// adds, from the press on.
        handle: PathHandle,
        /// The path as it was at the press.
        start: PathStart,
    },
    /// A label slid along its path and off it.
    Label {
        /// The label.
        label: NodeId,
        /// The path it is on.
        path: NodeId,
        /// Where its box centered at the press.
        start: Vec2,
    },
    /// A portal end slid round its node's outline and off it, by its
    /// marker or by its label.
    PortalEnd {
        /// The end.
        end: NodeId,
        /// Whether the press grabbed its label rather than its marker:
        /// what the drag puts where the pointer goes.
        by_label: bool,
        /// Where what the press grabbed centered at the press.
        start: Vec2,
    },
}

/// A path as a handle drag found it at the press.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::application::app) struct PathStart {
    /// Where the grabbed handle centered.
    pub(in crate::application::app) at: Vec2,
    /// Its control points: for a midpoint grabbed, the one it adds, at
    /// where the path passed (D34: an offset from the chord's middle).
    pub(in crate::application::app) points: Vec<Vec2>,
    /// Its anchors.
    pub(in crate::application::app) anchors: (Anchor, Anchor),
    /// The node the grabbed end is at; `None` for a control point.
    pub(in crate::application::app) node: Option<NodeId>,
}

impl Gesture {
    /// The history entry's label. O(1).
    pub(in crate::application::app) fn label(&self) -> &'static str {
        match self {
            Gesture::MoveNodes { .. } => "move",
            Gesture::MoveSection { .. } => "move section",
            Gesture::ResizeNode { .. } => "resize",
            Gesture::ResizeSection { .. } => "resize section",
            Gesture::PathHandle { .. } => "path handle",
            Gesture::Label { .. } => "label",
            Gesture::PortalEnd { .. } => "portal end",
        }
    }

    /// The gesture's name, as `query.mode` reports it. O(1).
    pub(in crate::application::app) fn word(&self) -> &'static str {
        match self {
            Gesture::MoveNodes { .. } => "MoveNodes",
            Gesture::MoveSection { .. } => "MoveSection",
            Gesture::ResizeNode { .. } => "ResizeNode",
            Gesture::ResizeSection { .. } => "ResizeSection",
            Gesture::PathHandle { .. } => "PathHandle",
            Gesture::Label { .. } => "Label",
            Gesture::PortalEnd { .. } => "PortalEnd",
        }
    }

    /// The steps the drag stages for `total`, its travel since the press
    /// in canvas units: built from what the press recorded, so the same
    /// total always stages the same request (never an accumulation). A
    /// label and a portal end are staged where they draw what the press
    /// grabbed at `start + total`, through the projector's inverse of
    /// where it draws them. Geometry the request needs — a path's curve,
    /// a node's outline — is the projector's. `Err` names why it cannot
    /// be built (an element gone, a path no longer drawn).
    ///
    /// Cost: one step per node moved; O(1) otherwise, and for a label or
    /// a portal end one nearest-point search (and for a portal end one of
    /// its offset curve).
    pub(in crate::application::app) fn steps(
        &self,
        doc: &Document,
        projector: &Projector,
        total: Vec2,
    ) -> Result<Vec<DocStep>, String> {
        let set = |kind, mutations: Vec<DocMutation>, element: NodeId| {
            compose::set(doc, kind, mutations, &[element]).map_err(|e| e.to_string())
        };
        match self {
            Gesture::MoveNodes { nodes } => {
                let mut steps = Vec::with_capacity(nodes.len());
                for &(node, start) in nodes {
                    steps.extend(set(
                        DocKind::Node,
                        vec![DocMutation::Node(NodeOp::SetPosition(start + total))],
                        node,
                    )?);
                }
                Ok(steps)
            }
            Gesture::MoveSection { section, start } => set(
                DocKind::Section,
                vec![DocMutation::Section(SectionOp::SetOffset(*start + total))],
                *section,
            ),
            Gesture::ResizeNode {
                node,
                side,
                start,
                floor,
            } => {
                let rect = side.resolve(*start, total, *floor);
                let mut ops = vec![DocMutation::Node(NodeOp::SetMinSize(Some(rect.size)))];
                if moves_origin(*side) {
                    ops.push(DocMutation::Node(NodeOp::SetPosition(rect.origin)));
                }
                set(DocKind::Node, ops, *node)
            }
            Gesture::ResizeSection {
                section,
                side,
                start,
                floor,
            } => {
                let rect = side.resolve(*start, total, *floor);
                let mut ops = vec![DocMutation::Section(SectionOp::SetMinSize(Some(rect.size)))];
                if moves_origin(*side) {
                    ops.push(DocMutation::Section(SectionOp::SetOffset(rect.origin)));
                }
                set(DocKind::Section, ops, *section)
            }
            Gesture::PathHandle {
                path,
                kind,
                handle,
                start,
            } => {
                let op = match handle {
                    PathHandle::Control(k) => {
                        let mut points = start.points.clone();
                        let Some(point) = points.get_mut(*k) else {
                            return Err(format!("the path has no control point {k}"));
                        };
                        *point += total;
                        PathOp::SetControlPoints(points)
                    }
                    PathHandle::Start | PathHandle::End => {
                        let node = start.node.ok_or("the end's node is not known")?;
                        let rect = projector.node_rect(node).ok_or("the end's node is not drawn")?;
                        let side = nearest_side(rect, start.at + total);
                        let (from, to) = start.anchors;
                        if *handle == PathHandle::Start {
                            PathOp::SetAnchors(side, to)
                        } else {
                            PathOp::SetAnchors(from, side)
                        }
                    }
                    // A midpoint grabbed becomes control point 0 at the
                    // press (`grab`), so a stage never sees one.
                    PathHandle::Midpoint => {
                        return Err("a midpoint is grabbed as control point 0".to_string())
                    }
                };
                set(*kind, vec![DocMutation::Path(op)], *path)
            }
            Gesture::Label { label, path, start } => {
                let (t, offset) = projector
                    .label_place_at(*path, *start + total)
                    .ok_or("the label's path is not drawn")?;
                set(
                    DocKind::Label,
                    vec![
                        DocMutation::Label(LabelOp::SetT(Some(t))),
                        DocMutation::Label(LabelOp::SetOffset(snapped(offset))),
                    ],
                    *label,
                )
            }
            Gesture::PortalEnd { end, by_label, start } => {
                let (border_t, offset) = projector
                    .portal_place_at(doc, *end, *by_label, *start + total)
                    .ok_or("the portal end is not drawn")?;
                set(
                    DocKind::PortalEnd,
                    vec![
                        DocMutation::PortalEnd(PortalEndOp::SetBorderT(Some(border_t))),
                        DocMutation::PortalEnd(PortalEndOp::SetOffset(snapped(offset))),
                    ],
                    *end,
                )
            }
        }
    }
}

/// Whether a handle on `side` moves its box's top-left: a top or a left
/// handle, which also stages the position (or offset). O(1).
fn moves_origin(side: ResizeSide) -> bool {
    let (fx, fy) = side.axis_factors();
    fx < 0 || fy < 0
}

/// An offset, or none when it is within [`OFFSET_SNAP`] of the line.
fn snapped(offset: f32) -> Option<f32> {
    (offset.abs() >= OFFSET_SNAP).then_some(offset)
}

/// The side of `rect` whose midpoint is nearest `point`: where a dragged
/// path end attaches. Ties go to the first of top, right, bottom, left.
/// O(1).
pub(in crate::application::app) fn nearest_side(rect: Rect, point: Vec2) -> Anchor {
    let (min, max, c) = (rect.origin, rect.max(), rect.center());
    [
        (Anchor::Top, Vec2::new(c.x, min.y)),
        (Anchor::Right, Vec2::new(max.x, c.y)),
        (Anchor::Bottom, Vec2::new(c.x, max.y)),
        (Anchor::Left, Vec2::new(min.x, c.y)),
    ]
    .into_iter()
    .min_by(|a, b| {
        a.1.distance_squared(point)
            .total_cmp(&b.1.distance_squared(point))
    })
    .map_or(Anchor::Auto, |(side, _)| side)
}

/// The drag in progress: what it edits, its input since the press, and
/// the throttle its drain is paced by. Held by the drag state
/// (`DragState::Dragging`); the transaction it stages into is the
/// document's, opened for [`Interaction::Drag`].
#[derive(Debug)]
pub(in crate::application::app) struct Live {
    /// What it edits.
    pub(in crate::application::app) gesture: Gesture,
    /// The button whose release commits it.
    pub(in crate::application::app) held_by: MouseButton,
    /// Its travel since the press, in canvas units.
    total: Vec2,
    /// Whether input moved it since the last stage.
    dirty: bool,
    /// Paces the drain under load.
    throttle: MutationFrequencyThrottle,
}

/// What a frame's drain did with a drag.
#[derive(Debug, PartialEq)]
pub(in crate::application::app) enum Drained {
    /// Nothing was pending, or the throttle held this frame back.
    Idle,
    /// It staged its total, this long ago, for the throttle to be told
    /// what the frame cost once the view has drawn it.
    Staged(std::time::Instant),
    /// Its transaction is no longer open — an undo canceled it — so the
    /// drag is over.
    Ended,
}

impl Live {
    /// Start a drag of `gesture` held by `held_by`: open its interactive
    /// transaction (`ChangeSource::Input`). `None` when the writer
    /// refuses it — another interactive transaction is open (D28) — which
    /// [`MindMapDocument::begin`] has reported on the status line. With
    /// `total`, its travel to the crossing, pending, so the frame that
    /// follows stages it.
    pub(in crate::application::app) fn begin(
        doc: &mut MindMapDocument,
        gesture: Gesture,
        held_by: MouseButton,
        total: Vec2,
    ) -> Option<Live> {
        if !doc.begin(
            ChangeSource::Input,
            TransactionKind::Interactive,
            Interaction::Drag,
        ) {
            return None;
        }
        Some(Live {
            gesture,
            held_by,
            total,
            dirty: true,
            throttle: MutationFrequencyThrottle::with_default_budget(),
        })
    }

    /// The pointer moved from `prev` to `cursor`, both on the canvas: the
    /// total follows, and the next frame stages it. Nothing is written
    /// here. O(1).
    pub(in crate::application::app) fn moved(&mut self, prev: Vec2, cursor: Vec2) {
        let step = cursor - prev;
        if step == Vec2::ZERO {
            return;
        }
        self.total += step;
        self.dirty = true;
    }

    /// Its travel since the press. O(1).
    pub(in crate::application::app) fn total(&self) -> Vec2 {
        self.total
    }

    /// Whether input moved it since its last stage: a stage the next
    /// frame owes, which keeps the loop running. O(1).
    pub(in crate::application::app) fn owes_stage(&self) -> bool {
        self.dirty
    }

    /// The frame's drain: when input moved the drag and the frame is
    /// due — always when `throttled` is false (a virtual clock) — stage
    /// the request for its total, once. See [`Drained`].
    ///
    /// Cost: building the request ([`Gesture::steps`]) and the stage's
    /// revert and run.
    pub(in crate::application::app) fn drain(
        &mut self,
        doc: &mut MindMapDocument,
        projector: &Projector,
        throttled: bool,
    ) -> Drained {
        if doc.open_interaction() != Some(Interaction::Drag) {
            return Drained::Ended;
        }
        if !self.dirty || (throttled && !self.throttle.should_drain()) {
            return Drained::Idle;
        }
        let started = std::time::Instant::now();
        self.stage(doc, projector);
        Drained::Staged(started)
    }

    /// Tell the throttle what a frame that staged cost, from `started`
    /// to now: the stage and the view's projection of it. Only under a
    /// real clock, where a frame's cost is what the user waits on.
    pub(in crate::application::app) fn record_frame(&mut self, started: std::time::Instant) {
        self.throttle.record_work_duration(started.elapsed());
    }

    /// Stage the request for the pending total, and mark it staged. A
    /// request that cannot be built, or that the writer refuses, is
    /// logged (the writer's refusal through
    /// [`MindMapDocument::report_refusal`]) and the stage before it
    /// stands.
    fn stage(&mut self, doc: &mut MindMapDocument, projector: &Projector) {
        self.dirty = false;
        let steps = match self.gesture.steps(doc.doc(), projector, self.total) {
            Ok(steps) => steps,
            Err(why) => {
                log::debug!("drag: {} not staged: {why}", self.gesture.label());
                return;
            }
        };
        let _ = doc.stage(Interaction::Drag, EditRequest::new(steps, self.gesture.label()));
    }

    /// The release: stage what input left pending, then commit the
    /// transaction as one history entry. A drag that went nowhere — a
    /// travel of nothing — is canceled instead and makes no entry.
    /// Returns the entry's revision, or `None`.
    pub(in crate::application::app) fn release(
        mut self,
        doc: &mut MindMapDocument,
        projector: &Projector,
    ) -> Option<Revision> {
        if doc.open_interaction() != Some(Interaction::Drag) {
            return None;
        }
        if self.total == Vec2::ZERO {
            doc.cancel(Interaction::Drag);
            return None;
        }
        if self.dirty {
            self.stage(doc, projector);
        }
        doc.commit(Interaction::Drag)
    }

    /// Escape, or a lost focus: cancel the transaction, and the document
    /// is exactly as it was before the press. Returns whether it was
    /// open.
    pub(in crate::application::app) fn cancel(self, doc: &mut MindMapDocument) -> bool {
        doc.cancel(Interaction::Drag)
    }
}

/// The position of `node` in `doc`, if it is a node. O(1).
pub(in crate::application::app) fn position_of(doc: &Document, node: NodeId) -> Option<Vec2> {
    match doc.get(node)? {
        DocElement::Node(data) => Some(data.position),
        _ => None,
    }
}

/// The offset of `section` in `doc`, if it is a section. O(1).
pub(in crate::application::app) fn offset_of(doc: &Document, section: NodeId) -> Option<Vec2> {
    match doc.get(section)? {
        DocElement::Section(data) => Some(data.offset),
        _ => None,
    }
}
