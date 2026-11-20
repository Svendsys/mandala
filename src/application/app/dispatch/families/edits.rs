// SPDX-License-Identifier: MPL-2.0

//! Edits — named, saved, undoable map changes (design §10.1, V1-D1) —
//! and **the one funnel every route runs them through**: a key bound in
//! `edit_bindings`, a macro's `Edit` step (and so IPC `act.macro`), and
//! the console's `edit apply`, and a map verb's `edit` step
//! (`families::verbs`, V1-D3), recorded as the verb's.
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
//! (D28), the presentation layer is set aside (`dispatch::the_map`), and
//! the view is brought up to the document before anything is read: a
//! filter reads the values the last edit left, and a handler reads
//! where they are drawn — the step before it in a macro included —
//! never a frame of an animation, a performance of the map, or a
//! projection a change has not reached.

use baumhard::arena::NodeId;
use baumhard::document::edit::{Animate, EditDef};
use baumhard::document::ids::{Document, NodeIdent};
use baumhard::document::values::EditId;
use baumhard::document::writer::{ChangeSource, EditRequest};
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
    run_which(
        doc,
        drawn,
        edits,
        WhichEdit::Named(edit_id),
        anchors,
        source,
        now_ms,
    )
}

/// Run one of the map's own edits, `id`, as [`run_edit`] runs one by
/// name: what a verb's `edit` step runs (ruling D3.3 item 2 on #252). The
/// definition is the map's at that index, read in O(1): it never goes
/// through the tiers' lookup by name (`EditRegistry::find`), so it is
/// neither a scan of the map's edits nor able to reach a user's or the
/// app's (amendment 2).
pub(in crate::application) fn run_map_edit(
    doc: &mut MindMapDocument,
    drawn: &mut dyn Drawn,
    edits: &EditRegistry,
    id: EditId,
    anchors: &[NodeId],
    source: ChangeSource,
    now_ms: f64,
) -> Result<(), String> {
    run_which(
        doc,
        drawn,
        edits,
        WhichEdit::Map(id),
        Some(anchors),
        source,
        now_ms,
    )
}

/// Which edit a run takes.
#[derive(Copy, Clone)]
enum WhichEdit<'a> {
    /// One named by id, through the tiers: the map's, else the user's,
    /// else the app's.
    Named(&'a str),
    /// One of the map's own, by its index.
    Map(EditId),
}

impl WhichEdit<'_> {
    /// The definition, on `doc`.
    fn find<'d>(self, doc: &'d Document, edits: &'d EditRegistry) -> Option<&'d EditDef> {
        match self {
            WhichEdit::Named(id) => edits.find(doc, id).map(|(def, _)| def),
            WhichEdit::Map(id) => doc.definitions().edits.get(usize::from(id.0)),
        }
    }

    /// How a refusal names it.
    fn named(self, doc: &Document) -> String {
        match self {
            WhichEdit::Named(id) => id.to_string(),
            WhichEdit::Map(id) => doc
                .definitions()
                .edits
                .get(usize::from(id.0))
                .map_or_else(|| format!("{id:?}"), |def| def.id.clone()),
        }
    }
}

/// The run [`run_edit`] and [`run_map_edit`] make: the lookup `which`
/// names, then [`request_of`] on the definition it found, then the
/// apply.
fn run_which(
    doc: &mut MindMapDocument,
    drawn: &mut dyn Drawn,
    edits: &EditRegistry,
    which: WhichEdit<'_>,
    anchors: Option<&[NodeId]>,
    source: ChangeSource,
    now_ms: f64,
) -> Result<(), String> {
    doc.finish_animation();
    // The map itself, and where it is drawn without a presentation on
    // it: the edit's filter, and a handler's layout, read neither a
    // lifted node nor a concealed one (`dispatch::the_map`).
    let mut doc = doc.suspended();
    let geometry = drawn.synced(&mut doc);
    let selection: Vec<NodeId>;
    let anchors = match anchors {
        Some(anchors) => anchors,
        None => {
            selection = doc.selection.iter().collect();
            &selection
        }
    };
    let Some(def) = which.find(doc.doc(), edits) else {
        return Err(match which {
            WhichEdit::Named(id) => format!("no edit is called `{id}`"),
            WhichEdit::Map(id) => format!("no edit at {id:?}: a run asks only for an edit the map defines"),
        });
    };
    let (request, animate) = request_of(doc.doc(), geometry, edits, def, anchors)?;
    doc.run_edit(request, animate, source, now_ms)
        .map_err(|error| format!("edit `{}`: {error}", which.named(doc.doc())))
}

/// The request the edit `def` makes at `anchors` on `doc`, drawn as
/// `geometry`, and how it animates: the body of a run, on the definition
/// whichever lookup found. Errors, naming the edit: a name it writes the
/// map lacks, a handler that declined.
fn request_of(
    doc: &Document,
    geometry: &dyn Geometry,
    edits: &EditRegistry,
    def: &EditDef,
    anchors: &[NodeId],
) -> Result<(EditRequest, Option<Animate>), String> {
    let request = def
        .request(doc, geometry, edits.handlers(), anchors)
        .map_err(|error| error.to_string())?;
    Ok((request, def.animate))
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
