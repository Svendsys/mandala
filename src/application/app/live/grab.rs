// SPDX-License-Identifier: MPL-2.0

//! What a press grabs, and the [`Gesture`] a drag of it past the
//! threshold starts, with the values it starts from.
//!
//! **A handle first.** The chrome is drawn over the map, so a press asks
//! the handle chrome before it asks the projector what it landed on
//! (`MapView::handle_at`, then `MapView::hit`): a resize handle, then a
//! selected path's handle. Then a label or a portal end the press landed
//! on, which a drag slides. Every other press grabs nothing here, and its
//! drag moves the node it landed on, or is whatever `LeftDrag` is bound
//! to (`event_cursor_moved`).
//!
//! The values a gesture starts from are read once, here: positions and
//! control points from the document, drawn rectangles and curves from the
//! projector's geometry (design §12.2; nothing reads a glyph layer).

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::ids::Document;
use baumhard::projection::{Hit, Projector};
use glam::Vec2;

use super::{offset_of, position_of, Gesture, PathStart};
use crate::application::document::selection::kind_of;
use crate::application::view::handles::{self, Handle, PathHandle, ResizeSide};
use crate::application::view::MapView;

/// What a press grabbed, beyond the element a click there selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum Grab {
    /// A handle of the chrome.
    Handle(Handle),
    /// A label on a path.
    Label {
        /// The label.
        label: NodeId,
        /// The connector or line link it is on.
        path: NodeId,
    },
    /// A portal end, by its marker or its label.
    PortalEnd {
        /// The end.
        end: NodeId,
        /// The end's label, when the press landed on it rather than on
        /// the marker.
        label: Option<NodeId>,
    },
}

impl Grab {
    /// Whether it is a handle: a drag affordance with no click meaning of
    /// its own, which takes no part in a double-click (#218), unlike a
    /// label. O(1).
    pub(in crate::application::app) fn is_handle(self) -> bool {
        matches!(self, Grab::Handle(_))
    }

    /// The grab's name, as `query.mode` reports it. O(1).
    pub(in crate::application::app) fn word(self) -> &'static str {
        match self {
            Grab::Handle(Handle::Node { .. }) => "node_resize_handle",
            Grab::Handle(Handle::Section { .. }) => "section_resize_handle",
            Grab::Handle(Handle::Path { .. }) => "path_handle",
            Grab::Label { .. } => "label",
            Grab::PortalEnd { .. } => "portal_end",
        }
    }
}

/// What a press at `canvas` grabs: a handle within `tolerance` (canvas
/// units) of the pointer, or else the label or portal end `landed` — the
/// press's one hit test — is on. `None` for anything else.
///
/// Cost: O(the shown handles); O(1) for the hit.
pub(in crate::application::app) fn press_grab(
    doc: &Document,
    view: &MapView,
    landed: Option<&Hit>,
    canvas: Vec2,
    tolerance: f32,
) -> Option<Grab> {
    if let Some(handle) = view.handle_at(canvas, tolerance) {
        return Some(Grab::Handle(handle));
    }
    let element = landed?.element;
    match kind_of(doc, element)? {
        DocKind::Label => {
            let host = doc.parent(element)?;
            match kind_of(doc, host)? {
                DocKind::Connector | DocKind::Link => Some(Grab::Label {
                    label: element,
                    path: host,
                }),
                DocKind::PortalEnd => Some(Grab::PortalEnd {
                    end: host,
                    label: Some(element),
                }),
                _ => None,
            }
        }
        DocKind::PortalEnd => Some(Grab::PortalEnd {
            end: element,
            label: None,
        }),
        _ => None,
    }
}

/// The gesture a drag of `grab` starts, with its start values read now:
/// `None` when what it grabbed is gone or not drawn — a press whose
/// handle's target was deleted between the press and the threshold
/// starts no drag at all, rather than turn into another (as on `main`).
///
/// Cost: O(the node's sections) for a node's resize; O(the path's
/// control points) for a path handle; O(1) otherwise.
pub(in crate::application::app) fn gesture_for(
    grab: Grab,
    doc: &Document,
    view: &MapView,
) -> Option<Gesture> {
    let projector = view.projector();
    match grab {
        Grab::Handle(Handle::Node { node, side }) => resize_node(doc, projector, node, side),
        Grab::Handle(Handle::Section { section, side }) => resize_section(doc, projector, section, side),
        Grab::Handle(handle @ Handle::Path { path, handle: which }) => {
            let at = view.handle_center(handle)?;
            path_handle(doc, projector, path, which, at)
        }
        Grab::Label { label, path } => (doc.get(label).is_some() && projector.curve(path).is_some())
            .then(|| projector.label_pos(label))
            .flatten()
            .map(|start| Gesture::Label { label, path, start }),
        Grab::PortalEnd { end, label } => {
            let start = match label {
                None => projector.portal_end_pos(doc, end),
                Some(label) => projector.label_pos(label),
            }?;
            Some(Gesture::PortalEnd {
                end,
                by_label: label.is_some(),
                start,
            })
        }
    }
}

/// A resize of `node` by its `side` handle, from its drawn rectangle,
/// held to its sections' extent. `None` for a node not drawn, or drawn
/// with no area.
pub(in crate::application::app) fn resize_node(
    doc: &Document,
    projector: &Projector,
    node: NodeId,
    side: ResizeSide,
) -> Option<Gesture> {
    let start = projector
        .node_rect(node)
        .filter(|r| r.size.x > 0.0 && r.size.y > 0.0)?;
    let floor = doc
        .children(node)
        .filter_map(|section| projector.section_rect(section))
        .map(|rect| rect.max() - start.origin)
        .fold(Vec2::ZERO, Vec2::max);
    Some(Gesture::ResizeNode {
        node,
        side,
        start,
        floor,
    })
}

/// A resize of `section` by its `side` handle, from its box in its node,
/// held to its text. `None` for a section not drawn.
pub(in crate::application::app) fn resize_section(
    doc: &Document,
    projector: &Projector,
    section: NodeId,
    side: ResizeSide,
) -> Option<Gesture> {
    let offset = offset_of(doc, section)?;
    let drawn = projector.section_rect(section)?;
    let floor = projector
        .section_box(section)
        .and_then(|(_, shaped)| shaped.text_box())
        .map_or(Vec2::ZERO, |text| text.max());
    Some(Gesture::ResizeSection {
        section,
        side,
        start: baumhard::flow::Rect::new(offset, drawn.size),
        floor,
    })
}

/// A drag of `path`'s `which` handle, which centered at `at` at the
/// press: its control points and anchors as they are now, and for an
/// end the node it is at. A midpoint grabbed is control point 0 from the
/// press on, at the offset that puts it where the path passed (#248's
/// ruling on its C1). `None` for a path not drawn.
pub(in crate::application::app) fn path_handle(
    doc: &Document,
    projector: &Projector,
    path: NodeId,
    which: PathHandle,
    at: Vec2,
) -> Option<Gesture> {
    let kind = kind_of(doc, path)?;
    let data = handles::path_data(doc, path)?;
    let curves = projector.curve(path)?;
    let (mut handle, mut points, mut node) = (which, data.control_points.clone(), None);
    match which {
        PathHandle::Midpoint => {
            let (first, last) = (curves.first()?, curves.last()?);
            let chord_middle = (first.point(0.0) + last.point(1.0)) * 0.5;
            points = vec![handles::midpoint(curves)? - chord_middle];
            handle = PathHandle::Control(0);
        }
        PathHandle::Start | PathHandle::End => {
            let (from, to) = projector.path_ends(path)?;
            node = Some(if which == PathHandle::Start { from } else { to });
        }
        PathHandle::Control(k) if k >= points.len() => return None,
        PathHandle::Control(_) => {}
    }
    Some(Gesture::PathHandle {
        path,
        kind,
        handle,
        start: PathStart {
            at,
            points,
            anchors: data.anchors,
            node,
        },
    })
}

/// A move of `pressed` — and, for a Shift drag, every node of `with`
/// (the selection as it was before the press touched it) — each with its
/// position now; and unless the drag is `alone` (Alt), every descendant
/// of each, each node once. As on `main`.
///
/// Cost: O(the moved nodes and their subtrees); one set to keep each
/// node once.
pub(in crate::application::app) fn move_nodes(
    doc: &Document,
    pressed: NodeId,
    with: &[NodeId],
    alone: bool,
) -> Option<Gesture> {
    let mut seen = rustc_hash::FxHashSet::default();
    let mut nodes = Vec::new();
    let mut stack: Vec<NodeId> = Vec::new();
    for &top in with.iter().chain(std::iter::once(&pressed)) {
        stack.push(top);
        while let Some(at) = stack.pop() {
            let Some(position) = position_of(doc, at) else {
                continue;
            };
            if !seen.insert(at) {
                continue;
            }
            nodes.push((at, position));
            if !alone {
                stack.extend(
                    doc.children(at)
                        .filter(|&c| kind_of(doc, c) == Some(DocKind::Node)),
                );
            }
        }
    }
    (!nodes.is_empty()).then_some(Gesture::MoveNodes { nodes })
}

/// A move of `section` within its node, from its offset now: what a
/// section drag in node editing starts (V1-C8 enters it, #248's ruling
/// on its C2). `None` for an element that is not a section.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "node editing (V1-C8, #249) enters a section drag; #248's ruling C2 gives it no Default-mode entry"
    )
)]
pub(in crate::application::app) fn move_section(doc: &Document, section: NodeId) -> Option<Gesture> {
    let start = offset_of(doc, section)?;
    Some(Gesture::MoveSection { section, start })
}
