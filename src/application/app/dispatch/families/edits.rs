// SPDX-License-Identifier: MPL-2.0

//! Edits — named, saved, undoable map changes (design §10.1, V1-D1) —
//! and **the one funnel every route runs them through**: a key bound in
//! `edit_bindings`, a macro's `Edit` step (and so IPC `act.macro`), and
//! the console's `edit apply`. A verb step (V1-D3) is the next caller.
//!
//! [`run_edit`] finds the edit by id across its tiers
//! (`application::edits`), resolves it against the open map at the
//! anchors — the selection, or nodes the caller names — and hands the
//! request to [`MindMapDocument::run_edit`]: one `Document::apply`, or
//! an animated transaction that the frame's tick (`app::animation`)
//! writes on the app clock and commits as one entry.

use baumhard::arena::NodeId;
use baumhard::document::writer::ChangeSource;
use baumhard::flow::{Curve, Rect, ShapedBox};
use baumhard::projection::Geometry;
use glam::Vec2;
use std::sync::Arc;

use baumhard::document::ids::NodeIdent;

use crate::application::document::MindMapDocument;
use crate::application::edits::EditRegistry;
use crate::application::macros::MacroTarget;
use crate::application::view::MapView;

/// Run the edit `edit_id` at `anchors` — the selection when `None` —
/// declared by `source`, on the app clock's reading `now_ms`. `view` is
/// where the map is drawn, which a handler reads; without one a handler
/// places nothing.
///
/// Errors, each with the message the caller shows or logs, and nothing
/// applied: no edit has the id; the edit names something the map lacks
/// (`EditError`, naming the edit, the field and the name); the writer
/// refused it (an open interactive transaction, D28, also on the status
/// line).
///
/// Cost: the edit's resolution and request (`EditDef::request`).
pub(in crate::application) fn run_edit(
    doc: &mut MindMapDocument,
    view: Option<&MapView>,
    edits: &EditRegistry,
    edit_id: &str,
    anchors: Option<&[NodeId]>,
    source: ChangeSource,
    now_ms: f64,
) -> Result<(), String> {
    let selection: Vec<NodeId>;
    let anchors = match anchors {
        Some(anchors) => anchors,
        None => {
            selection = doc.selection.iter().collect();
            &selection
        }
    };
    let Some((def, _)) = edits.find(doc.doc(), edit_id) else {
        return Err(format!("no edit is called `{edit_id}`"));
    };
    let geometry: &dyn Geometry = match view {
        Some(view) => view.projector(),
        None => &Undrawn,
    };
    let request = def
        .request(doc.doc(), geometry, edits.handlers(), anchors)
        .map_err(|error| error.to_string())?;
    let animate = def.animate;
    doc.run_edit(request, animate, source, now_ms)
        .map_err(|error| format!("edit `{edit_id}`: {error}"))
}

/// Run the edit a key is bound to (`edit_bindings`), at the selection.
/// A refusal is logged; the key is consumed either way.
pub(in crate::application::app) fn run_key(
    doc: &mut MindMapDocument,
    view: Option<&MapView>,
    edits: &EditRegistry,
    edit_id: &str,
    now_ms: f64,
) {
    if let Err(message) = run_edit(doc, view, edits, edit_id, None, ChangeSource::Edit, now_ms) {
        log::warn!("edit key: {message}");
    }
}

/// Run a macro's `Edit` step at its target: the selection, or the node
/// with the ident it names — a node the map lacks skips the step, with a
/// warning. Returns whether the edit ran: a refusal is logged, and does
/// not count.
pub(in crate::application::app) fn run_step(
    doc: &mut MindMapDocument,
    view: Option<&MapView>,
    edits: &EditRegistry,
    edit_id: &str,
    target: &MacroTarget,
    now_ms: f64,
) -> bool {
    let named: [NodeId; 1];
    let anchors = match target {
        MacroTarget::CurrentSelection => None,
        MacroTarget::NodeId(ident) => match doc.doc().node_by_ident(NodeIdent(*ident)) {
            Some(node) => {
                named = [node];
                Some(&named[..])
            }
            None => {
                log::warn!("macros: Edit step `{edit_id}`: no node has the id {ident}; skipping");
                return false;
            }
        },
    };
    match run_edit(doc, view, edits, edit_id, anchors, ChangeSource::Edit, now_ms) {
        Ok(()) => true,
        Err(message) => {
            log::warn!("macros: Edit step: {message}");
            false
        }
    }
}

/// A map that is not drawn: nothing has a rectangle, so a handler places
/// nothing.
struct Undrawn;

impl Geometry for Undrawn {
    fn node_rect(&self, _: NodeId) -> Option<Rect> {
        None
    }
    fn section_box(&self, _: NodeId) -> Option<(Vec2, &Arc<ShapedBox>)> {
        None
    }
    fn curve(&self, _: NodeId) -> Option<Curve> {
        None
    }
    fn label_pos(&self, _: NodeId) -> Option<Vec2> {
        None
    }
}
