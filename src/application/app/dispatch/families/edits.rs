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
//!
//! **It resolves against the map as it stands, drawn as it stands.** An
//! edit that is still animating is fast-forwarded and committed first
//! (D28), and the view is brought up to the document before anything is
//! read: a filter reads the values the last edit left, and a handler
//! reads where they are drawn — the step before it in a macro included —
//! never a frame of an animation or a projection a change has not
//! reached.

use baumhard::arena::NodeId;
use baumhard::document::ids::NodeIdent;
use baumhard::document::writer::ChangeSource;
use baumhard::flow::{Curve, Rect, ShapedBox};
use baumhard::projection::Geometry;
use glam::Vec2;
use std::sync::Arc;

use crate::application::document::MindMapDocument;
use crate::application::edits::EditRegistry;
use crate::application::macros::MacroTarget;
use crate::application::renderer::Renderer;
use crate::application::view::MapView;

/// Where the map is drawn, as an edit's resolution reads it.
pub(in crate::application) trait Drawn {
    /// Bring the drawn geometry up to `doc` — the view's sync, which does
    /// nothing when the change feed is empty — then lend it.
    fn synced(&mut self, doc: &mut MindMapDocument) -> &dyn Geometry;
}

/// The view and the renderer it syncs into: what every route has.
pub(in crate::application) struct Projected<'a> {
    /// The view.
    pub(in crate::application) view: &'a mut MapView,
    /// The renderer its sync hands the layer changes to.
    pub(in crate::application) renderer: &'a mut Renderer,
}

impl Drawn for Projected<'_> {
    fn synced(&mut self, doc: &mut MindMapDocument) -> &dyn Geometry {
        self.view.sync(doc, self.renderer);
        self.view.projector()
    }
}

/// A view without a renderer, for the tests that read what the pipeline
/// wrote (`MapView::sync_for_test`).
#[cfg(test)]
impl Drawn for MapView {
    fn synced(&mut self, doc: &mut MindMapDocument) -> &dyn Geometry {
        let _ = self.sync_for_test(doc);
        self.projector()
    }
}

/// A map that is not drawn: nothing has a rectangle, so a handler places
/// nothing.
pub(in crate::application) struct Undrawn;

impl Drawn for Undrawn {
    fn synced(&mut self, _: &mut MindMapDocument) -> &dyn Geometry {
        self
    }
}

impl Geometry for Undrawn {
    fn node_rect(&self, _: NodeId) -> Option<Rect> {
        None
    }
    fn section_box(&self, _: NodeId) -> Option<(Vec2, &Arc<ShapedBox>)> {
        None
    }
    fn curve(&self, _: NodeId) -> Option<&[Curve]> {
        None
    }
    fn label_pos(&self, _: NodeId) -> Option<Vec2> {
        None
    }
    fn nodes_meeting(&self, _: Rect) -> Vec<NodeId> {
        Vec::new()
    }
}

/// Run `f` with what draws the map: the view and `renderer`, or
/// [`Undrawn`] with no view.
pub(in crate::application) fn with_drawn<R>(
    view: Option<&mut MapView>,
    renderer: &mut Renderer,
    f: impl FnOnce(&mut dyn Drawn) -> R,
) -> R {
    match view {
        Some(view) => f(&mut Projected { view, renderer }),
        None => f(&mut Undrawn),
    }
}

/// Run the edit `edit_id` at `anchors` — the selection when `None` —
/// declared by `source`, on the app clock's reading `now_ms`. `drawn` is
/// where the map is drawn, which a handler reads.
///
/// First an edit still animating is fast-forwarded and committed, and
/// the view synced to the document, so the edit resolves against the map
/// as it stands (see the module docs).
///
/// Errors, each with the message the caller shows or logs, and nothing
/// applied: no edit has the id; the edit names something the map lacks
/// (`EditError`, naming the edit, the field and the name); a handler
/// declined; the writer refused it (an open interactive transaction,
/// D28, also on the status line).
///
/// Cost: the view's sync, and the edit's resolution and request
/// (`EditDef::request`).
pub(in crate::application) fn run_edit(
    doc: &mut MindMapDocument,
    drawn: &mut dyn Drawn,
    edits: &EditRegistry,
    edit_id: &str,
    anchors: Option<&[NodeId]>,
    source: ChangeSource,
    now_ms: f64,
) -> Result<(), String> {
    doc.finish_animation();
    let geometry = drawn.synced(doc);
    let Some((def, _)) = edits.find(doc.doc(), edit_id) else {
        return Err(format!("no edit is called `{edit_id}`"));
    };
    let selection: Vec<NodeId>;
    let anchors = match anchors {
        Some(anchors) => anchors,
        None => {
            selection = doc.selection.iter().collect();
            &selection
        }
    };
    let animate = def.animate;
    let request = def
        .request(doc.doc(), geometry, edits.handlers(), anchors)
        .map_err(|error| error.to_string())?;
    doc.run_edit(request, animate, source, now_ms)
        .map_err(|error| format!("edit `{edit_id}`: {error}"))
}

/// Run the edit a key is bound to (`edit_bindings`), at the selection.
/// A refusal is logged; the key is consumed either way.
pub(in crate::application::app) fn run_key(
    doc: &mut MindMapDocument,
    drawn: &mut dyn Drawn,
    edits: &EditRegistry,
    edit_id: &str,
    now_ms: f64,
) {
    if let Err(message) = run_edit(doc, drawn, edits, edit_id, None, ChangeSource::Edit, now_ms) {
        log::warn!("edit key: {message}");
    }
}

/// Run a macro's `Edit` step at its target: the selection, or the node
/// with the ident it names — a node the map lacks skips the step, with a
/// warning. Returns whether the edit ran: a refusal is logged, and does
/// not count.
pub(in crate::application::app) fn run_step(
    doc: &mut MindMapDocument,
    drawn: &mut dyn Drawn,
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
    match run_edit(doc, drawn, edits, edit_id, anchors, ChangeSource::Edit, now_ms) {
        Ok(()) => true,
        Err(message) => {
            log::warn!("macros: Edit step: {message}");
            false
        }
    }
}
