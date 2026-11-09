// SPDX-License-Identifier: MPL-2.0

//! Structure editing on the one writer (V1-C4, #245): create, delete,
//! orphan, reparent, connect, and the clipboard.
//!
//! **One Action, one request, one undo entry**, whatever the selection
//! holds: each body builds its steps from the composite builders
//! (`baumhard::document::mutation::compose`) and the clipboard's
//! (`crate::application::clipboard`), with every element the Action
//! reaches as an anchor of one walk, and hands them to
//! [`MindMapDocument::apply`] as `ChangeSource::Action` — the keystroke,
//! a macro step and IPC `act.action` alike. A refusal (an open
//! interactive transaction, D28) is reported there, on the status line.
//! The selection then names what the user expects, by `NodeId`: the new
//! node, the moved nodes, the new links; a delete leaves what is still
//! in the tree, and a paste its targets.
//!
//! Reparent and connect are modes, entered from the selection and ended
//! by the next click (`Action::ReparentToTarget` /
//! `Action::ConnectToTarget`, which the click handler dispatches). While
//! one is active the pointer's valid target is tinted
//! (`MapView::set_hover_target`). Both are native-only, as on `main`:
//! their clicks and hover come from native's pointer handlers, so the
//! browser answers their Actions `Unhandled`.

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::mutation::compose;
use baumhard::document::mutation::error::MutationError;
use baumhard::document::mutation::step::DocStep;
use baumhard::document::writer::{ChangeSource, EditRequest};
use glam::Vec2;

use crate::application::clipboard;
use crate::application::document::selection::{child_nodes, kind_of};
use crate::application::document::{MindMapDocument, TRANSACTION_OPEN_STATUS};
use crate::application::keybinds::Action;
use baumhard::document::writer::error::ApplyError;

use super::super::cross_dispatch::{view_ctx, DispatchHit, DispatchOutcome};
use crate::application::app::input_context_core::InputContextCore;

/// What a delete of nothing but connectors says: a connector is the line
/// to its node's parent, and goes only with the node or its parent.
pub(in crate::application::app) const CONNECTOR_STATUS: &str =
    "a connector cannot be deleted: orphan or delete its node";

/// What a click on a node a reparent cannot land on says.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) const INVALID_TARGET_STATUS: &str =
    "cannot reparent a node under itself or below it";

/// The structure family's answer to `action`, or `None` for an Action it
/// does not own. `hit` places a new node where a pointer dispatch landed.
pub(in crate::application::app) fn dispatch(
    action: &Action,
    core: &mut InputContextCore<'_>,
    hit: Option<&DispatchHit>,
) -> Option<DispatchOutcome> {
    let outcome = match action {
        Action::CreateOrphanNode => {
            let at = hit.map_or(core.renderer.camera().position, |hit| hit.canvas_pos);
            edit(core, |doc| create_orphan(doc, at))
        }
        Action::DeleteSelection => edit(core, delete_selection),
        Action::OrphanSelection => edit(core, orphan_selection),
        Action::Copy => edit(core, |doc| copy(doc, false)),
        Action::Cut => edit(core, |doc| copy(doc, true)),
        Action::Paste => edit(core, paste),
        #[cfg(not(target_arch = "wasm32"))]
        Action::EnterReparentMode | Action::EnterConnectMode => modes::enter(action, core),
        #[cfg(not(target_arch = "wasm32"))]
        Action::ReparentToTarget(target) | Action::ConnectToTarget(target) => {
            modes::finish(action, *target, core)
        }
        #[cfg(target_arch = "wasm32")]
        Action::EnterReparentMode
        | Action::EnterConnectMode
        | Action::ReparentToTarget(_)
        | Action::ConnectToTarget(_) => DispatchOutcome::Unhandled,
        _ => return None,
    };
    Some(outcome)
}

/// What a body did: `Ok(true)` when it changed something, `Ok(false)`
/// when nothing applied, `Err(reason)` when it declined or the writer
/// refused for a reason the user is told on the status line — which
/// `act.action` answers too (`DispatchOutcome::Refused`, #194).
type Ran = Result<bool, &'static str>;

/// Run `body` on the document, then sync the view — a status line it
/// left is drawn even when nothing changed.
fn edit(core: &mut InputContextCore<'_>, body: impl FnOnce(&mut MindMapDocument) -> Ran) -> DispatchOutcome {
    let Some(doc) = core.document.as_deref_mut() else {
        return DispatchOutcome::Unhandled;
    };
    let ran = body(doc);
    if let Some(mut vc) = view_ctx!(core, doc) {
        vc.sync();
    }
    match ran {
        Ok(true) => DispatchOutcome::Handled,
        Ok(false) => DispatchOutcome::Unhandled,
        Err(reason) => DispatchOutcome::Refused(reason),
    }
}

/// Apply `steps` as one request called `label`, from an Action. Nothing
/// applies for no steps, or a builder that refused (logged); a writer
/// refusal is reported by [`MindMapDocument::apply`], and an open
/// interactive transaction's (D28) is the reason returned.
fn apply(doc: &mut MindMapDocument, steps: Result<Vec<DocStep>, MutationError>, label: &str) -> Ran {
    match steps {
        Ok(steps) if steps.is_empty() => Ok(false),
        Ok(steps) => match doc.apply(EditRequest::new(steps, label), ChangeSource::Action) {
            Ok(_) => Ok(true),
            Err(ApplyError::TransactionOpen) => Err(TRANSACTION_OPEN_STATUS),
            Err(_) => Ok(false),
        },
        Err(error) => {
            log::warn!("structure: {label}: {error}");
            Ok(false)
        }
    }
}

/// Decline with `why`: the status line says it, and so does the reply.
fn decline(doc: &mut MindMapDocument, why: &'static str) -> Ran {
    doc.decline(why);
    Err(why)
}

/// The selected elements of `kind`, in selection order.
fn selected(doc: &MindMapDocument, kind: DocKind) -> Vec<NodeId> {
    let of_kind = |&id: &NodeId| kind_of(doc.doc(), id) == Some(kind);
    doc.selection.iter().filter(of_kind).collect()
}

/// `CreateOrphanNode`: a root node at `at`, with one empty section,
/// selected.
fn create_orphan(doc: &mut MindMapDocument, at: Vec2) -> Ran {
    if !apply(doc, compose::create_root(doc.doc(), at), "create node")? {
        return Ok(false);
    }
    let map = doc.doc().root();
    if let Some(new) = child_nodes(doc.doc(), map).last() {
        doc.selection.select_only(new);
    }
    Ok(true)
}

/// `DeleteSelection`: the selected labels, sections, links and nodes (with
/// their subtrees, and the links into them), in that order, so nothing is
/// named after its owner went. A selected connector is not deleted; with
/// nothing else to delete, the status line says so.
fn delete_selection(doc: &mut MindMapDocument) -> Ran {
    let d = doc.doc();
    let steps = (|| {
        let mut steps = Vec::new();
        for kind in [DocKind::Label, DocKind::Section, DocKind::Link] {
            steps.extend(compose::remove(d, kind, &selected(doc, kind))?);
        }
        steps.extend(compose::delete(d, &selected(doc, DocKind::Node))?);
        Ok(steps)
    })();
    if matches!(&steps, Ok(steps) if steps.is_empty()) && !selected(doc, DocKind::Connector).is_empty() {
        return decline(doc, CONNECTOR_STATUS);
    }
    let deleted = apply(doc, steps, "delete");
    doc.retain_live_selection();
    deleted
}

/// `OrphanSelection`: the selected nodes that are not roots become roots,
/// where they are on the canvas, and are the selection.
fn orphan_selection(doc: &mut MindMapDocument) -> Ran {
    let map = doc.doc().root();
    let nodes: Vec<NodeId> = selected(doc, DocKind::Node)
        .into_iter()
        .filter(|&node| doc.doc().parent(node) != Some(map))
        .collect();
    reparent(doc, nodes, map)
}

/// Move `nodes` under `parent` — a node, or the map's root — as one
/// request, and select them.
fn reparent(doc: &mut MindMapDocument, nodes: Vec<NodeId>, parent: NodeId) -> Ran {
    if !apply(doc, compose::reparent(doc.doc(), &nodes, parent), "reparent")? {
        return Ok(false);
    }
    doc.selection.set(nodes);
    Ok(true)
}

/// Whether `target` is a node `sources` may move under: not one of them,
/// and not below one. O(the target's depth × the sources): one climb.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn valid_reparent_target(
    doc: &baumhard::document::ids::Document,
    sources: &[NodeId],
    target: NodeId,
) -> bool {
    kind_of(doc, target) == Some(DocKind::Node)
        && !std::iter::successors(Some(target), |&at| doc.parent(at)).any(|at| sources.contains(&at))
}

/// `Copy` or `Cut`: the selection's clipboard form to the platform's
/// clipboard and the document's; a cut also clears it, as one request.
/// A cut of a range leaves the range empty where it was.
fn copy(doc: &mut MindMapDocument, cut: bool) -> Ran {
    let taken = match clipboard::take(doc.doc(), &doc.selection, cut) {
        Ok(Some(taken)) => taken,
        Ok(None) => return Ok(false),
        Err(error) => {
            log::warn!("structure: cut: {error}");
            return Ok(false);
        }
    };
    let (content, steps) = taken;
    if cut && !apply(doc, Ok(steps), "cut")? {
        return Ok(false);
    }
    clipboard::write_clipboard(&content.text);
    doc.clipboard = Some(content);
    if let (true, Some((section, start, _))) = (cut, doc.selection.range()) {
        doc.selection.select_range(section, start, start);
    }
    Ok(true)
}

/// `Paste`: the platform's clipboard text — or, where it has none, the
/// app's — into the selection, as one request. A pasted range covers what
/// was pasted.
fn paste(doc: &mut MindMapDocument) -> Ran {
    let Some(text) = clipboard::read_clipboard().or_else(|| doc.clipboard.as_ref().map(|c| c.text.clone()))
    else {
        return Ok(false);
    };
    let steps = clipboard::paste(doc.doc(), &doc.selection, &text, doc.clipboard.as_ref());
    if !apply(doc, steps, "paste")? {
        return Ok(false);
    }
    if let Some((section, start, _)) = doc.selection.range() {
        let pasted = baumhard::util::grapheme_chad::count_grapheme_clusters(&text);
        doc.selection.select_range(section, start, start + pasted);
    }
    Ok(true)
}

/// The two target-picking modes: entering them, and the click that ends
/// them. Native-only, with the pointer handlers that drive them.
#[cfg(not(target_arch = "wasm32"))]
mod modes {
    use baumhard::document::ids::NodeIdent;

    use super::*;
    use crate::application::app::InteractionMode;

    /// `EnterReparentMode` / `EnterConnectMode`: the selected nodes are
    /// the sources. The view follows the mode at the next frame: the
    /// sources' tint and the mode's status line (`MapView::follow_mode`).
    /// `Unhandled` with none selected.
    pub(super) fn enter(action: &Action, core: &mut InputContextCore<'_>) -> DispatchOutcome {
        let Some(doc) = core.document.as_deref() else {
            return DispatchOutcome::Unhandled;
        };
        let sources = selected(doc, DocKind::Node);
        if sources.is_empty() {
            return DispatchOutcome::Unhandled;
        }
        *core.interaction_mode = match action {
            Action::EnterReparentMode => InteractionMode::Reparent { sources },
            _ => InteractionMode::Connect { sources },
        };
        *core.last_click = None;
        DispatchOutcome::Handled
    }

    /// `ReparentToTarget` / `ConnectToTarget`: leave the mode, and move or
    /// link its sources to the node `target` names — `None`, a click on
    /// empty canvas, makes a reparent's sources roots and ends a connect
    /// with nothing linked. `Unhandled` outside the matching mode, for an
    /// id no node has, and when nothing changed.
    pub(super) fn finish(
        action: &Action,
        target: Option<u32>,
        core: &mut InputContextCore<'_>,
    ) -> DispatchOutcome {
        let reparenting = matches!(action, Action::ReparentToTarget(_));
        let sources = match (std::mem::take(core.interaction_mode), reparenting) {
            (InteractionMode::Reparent { sources }, true) | (InteractionMode::Connect { sources }, false) => {
                sources
            }
            (mode, _) => {
                *core.interaction_mode = mode;
                return DispatchOutcome::Unhandled;
            }
        };
        super::edit(core, |doc| {
            let target = match target.map(|ident| doc.doc().node_by_ident(NodeIdent(ident))) {
                Some(Some(node)) => Some(node),
                Some(None) => return Ok(false),
                None => None,
            };
            let sources: Vec<NodeId> = sources
                .into_iter()
                .filter(|&s| doc.doc().get(s).is_some())
                .collect();
            match (reparenting, target) {
                (true, Some(target)) if !valid_reparent_target(doc.doc(), &sources, target) => {
                    decline(doc, INVALID_TARGET_STATUS)
                }
                (true, Some(target)) => reparent(doc, sources, target),
                (true, None) => {
                    doc.selection.set(sources.iter().copied());
                    orphan_selection(doc)
                }
                (false, Some(target)) => connect(doc, &sources, target),
                (false, None) => Ok(false),
            }
        })
    }

    /// A line link from each of `sources` to `target`, as one request;
    /// the selection becomes the line links between them.
    fn connect(doc: &mut MindMapDocument, sources: &[NodeId], target: NodeId) -> Ran {
        if !apply(doc, compose::connect(doc.doc(), sources, target), "connect")? {
            return Ok(false);
        }
        let d = doc.doc();
        let links: Vec<NodeId> = sources
            .iter()
            .flat_map(|&source| d.children(source))
            .filter(|&id| {
                matches!(d.get(id), Some(baumhard::document::element::DocElement::Link(link))
                    if link.to == target && link.kind == baumhard::document::values::LinkKind::Line)
            })
            .collect();
        doc.selection.set(links);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    //! The bodies, renderer-free: the clipboard's text paths and the D28
    //! refusal. Every Action through the real app is
    //! `app::tests_gpu_structure`'s.

    use baumhard::document::element::{DocElement, TextStyle};
    use baumhard::document::format;
    use baumhard::document::ids::NodeIdent;
    use baumhard::document::writer::transaction::TransactionKind;
    use baumhard::document::writer::{ChangeSource, DEFAULT_HISTORY_LIMIT};
    use serde_json::json;

    use super::*;
    use crate::application::clipboard::Clipboard;
    use crate::application::document::TRANSACTION_OPEN_STATUS;
    use crate::application::view::inspect::{children_of_kind, element_text};

    /// The family emoji: one grapheme cluster of five code points.
    const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    /// An `e` with two combining marks: one cluster of three code points.
    const MARKED: &str = "e\u{0301}\u{0302}";

    fn load(map: serde_json::Value) -> MindMapDocument {
        MindMapDocument::from_json_str(&map.to_string(), None, DEFAULT_HISTORY_LIMIT).expect("the map loads")
    }

    fn structure() -> MindMapDocument {
        let text = include_str!("fixtures/structure.mandala.json");
        MindMapDocument::from_json_str(text, None, DEFAULT_HISTORY_LIMIT).expect("the fixture loads")
    }

    fn node(doc: &MindMapDocument, ident: u32) -> NodeId {
        doc.doc().node_by_ident(NodeIdent(ident)).expect("the node")
    }

    fn section(doc: &MindMapDocument, node: NodeId, index: usize) -> NodeId {
        children_of_kind(doc.doc(), node, DocKind::Section)
            .nth(index)
            .expect("the section")
    }

    fn bytes(doc: &MindMapDocument) -> String {
        format::save(doc.doc()).expect("saves")
    }

    /// The spans of `section`: text, and whether it is bold.
    fn spans(doc: &MindMapDocument, section: NodeId) -> Vec<(String, bool)> {
        doc.doc()
            .children(section)
            .filter_map(|id| match doc.doc().get(id) {
                Some(DocElement::Span(span)) => Some((span.text.clone(), span.style.weight.is_some())),
                _ => None,
            })
            .collect()
    }

    /// **A range cut and its paste round-trip text across clusters of
    /// several code points, a span edge among them.** The range is
    /// clusters 1..3 of `x👨‍👩‍👧` + `é̂y` — the emoji (five code points)
    /// and the marked `e` (three), on both sides of the span edge. The
    /// cut takes exactly those two clusters, leaves `xy` and an empty
    /// range at 1; the paste puts them back at 1 and selects them. Each
    /// is one entry, and two undos give back the original bytes. A count
    /// in code points instead would cut inside the emoji.
    #[test]
    fn test_a_range_cut_and_paste_round_trip_across_multi_codepoint_clusters() {
        let mut doc = load(json!({"format": 1, "name": "range", "nodes": [{
            "id": 1,
            "position": [0.0, 0.0],
            "sections": [{"spans": [
                {"text": format!("x{FAMILY}"), "weight": "bold"},
                {"text": format!("{MARKED}y")}
            ]}]
        }]}));
        let original = bytes(&doc);
        let s = section(&doc, node(&doc, 1), 0);
        doc.selection.select_range(s, 1, 3);

        assert_eq!(copy(&mut doc, true), Ok(true), "the cut ran");
        let taken = doc.clipboard.as_ref().map(|c| c.text.clone());
        assert_eq!(
            taken,
            Some(format!("{FAMILY}{MARKED}")),
            "the cut took the two clusters whole"
        );
        assert_eq!(element_text(doc.doc(), s), "xy");
        assert_eq!(
            doc.selection.range(),
            Some((s, 1, 1)),
            "an empty range where the text was"
        );
        assert!(
            doc.clipboard.as_ref().is_some_and(|c| c.section.is_none()),
            "a range carries no fragment"
        );

        assert_eq!(paste(&mut doc), Ok(true), "the paste ran");
        assert_eq!(element_text(doc.doc(), s), format!("x{FAMILY}{MARKED}y"));
        assert_eq!(
            doc.selection.range(),
            Some((s, 1, 3)),
            "the pasted text is selected"
        );
        assert_eq!(doc.doc().history().count(), 2, "one entry each");

        doc.undo();
        doc.undo();
        assert_eq!(bytes(&doc), original);
    }

    /// **A section paste whose text is not the copied fragment's pastes
    /// the text, as one unstyled span**, and leaves the target's style
    /// alone: the platform's clipboard was written by another app since.
    /// The control is the same paste with the fragment's own text, which
    /// pastes the fragment: its two spans, the first bold.
    #[test]
    fn test_other_text_than_the_fragments_pastes_as_one_unstyled_span() {
        let mut doc = structure();
        let n2 = node(&doc, 2);
        let (one, two) = (section(&doc, n2, 0), section(&doc, n2, 1));
        doc.selection.select_only(two);
        assert_eq!(copy(&mut doc, false), Ok(true));
        let clipboard: Clipboard = doc.clipboard.clone().expect("copied");
        assert_eq!(clipboard.text, "two parts");
        doc.selection.select_only(one);

        let steps = clipboard::paste(doc.doc(), &doc.selection, "other", Some(&clipboard));
        assert_eq!(apply(&mut doc, steps, "paste"), Ok(true));
        assert_eq!(spans(&doc, one), vec![("other".to_string(), false)]);
        let Some(DocElement::Section(data)) = doc.doc().get(one) else { panic!("a section") };
        assert_eq!(
            (data.align, &data.text),
            (None, &TextStyle::default()),
            "its style is its own"
        );
        doc.undo();

        let steps = clipboard::paste(doc.doc(), &doc.selection, "two parts", Some(&clipboard));
        assert_eq!(apply(&mut doc, steps, "paste"), Ok(true));
        assert_eq!(
            spans(&doc, one),
            vec![("two ".to_string(), true), ("parts".to_string(), false)],
            "control: the fragment"
        );
    }

    /// **Several copied elements paste one piece each onto as many
    /// targets** — the texts of nodes 1 and 3 onto sections of 5 and 6 —
    /// **and the whole text onto any other count** (the control: three
    /// targets each get it all).
    #[test]
    fn test_a_multi_element_copy_pastes_a_piece_onto_each_of_as_many_targets() {
        let mut doc = structure();
        let (n1, n3, n5, n6) = (node(&doc, 1), node(&doc, 3), node(&doc, 5), node(&doc, 6));
        doc.selection.set([n1, n3]);
        assert_eq!(copy(&mut doc, false), Ok(true));
        assert_eq!(doc.clipboard.as_ref().map(|c| c.text.as_str()), Some("A\n\nC"));
        let clipboard = doc.clipboard.clone();

        doc.selection.set([n5, n6]);
        assert_eq!(paste(&mut doc), Ok(true));
        let text = |doc: &MindMapDocument, n| element_text(doc.doc(), section(doc, n, 0));
        assert_eq!(
            (text(&doc, n5), text(&doc, n6)),
            ("A".to_string(), "C".to_string())
        );
        assert_eq!(doc.doc().history().count(), 1, "one entry for both");
        doc.undo();

        doc.selection.set([n5, n6, n3]);
        let steps = clipboard::paste(doc.doc(), &doc.selection, "A\n\nC", clipboard.as_ref());
        assert_eq!(apply(&mut doc, steps, "paste"), Ok(true));
        assert_eq!(
            text(&doc, n6),
            "A\n\nC",
            "control: three targets each get the whole text"
        );
    }

    /// **An open interactive transaction refuses a structure action**
    /// (D28): the delete applies nothing and the status line says why.
    /// Control: once the transaction is gone, the same delete applies.
    #[test]
    fn test_an_open_transaction_refuses_a_structure_action() {
        let mut doc = structure();
        let before = bytes(&doc);
        doc.selection.select_only(node(&doc, 6));
        assert!(doc.begin(ChangeSource::Input, TransactionKind::Interactive));
        assert_eq!(
            delete_selection(&mut doc),
            Err(TRANSACTION_OPEN_STATUS),
            "refused, saying why"
        );
        assert_eq!(doc.status(), Some(TRANSACTION_OPEN_STATUS));
        assert_eq!(bytes(&doc), before);

        doc.undo();
        assert_eq!(
            delete_selection(&mut doc),
            Ok(true),
            "control: with none open, it deletes"
        );
        assert!(doc.selection.iter().next().is_none());
    }

    /// The color a paste names: hex, a theme variable by name, empty for
    /// none; anything else is not a color.
    #[test]
    fn test_a_pasted_color_is_hex_a_variable_or_nothing() {
        let mut doc = load(json!({"format": 1, "name": "colors",
        "theme": {"active": "dark", "variants": {"dark": {"accent": "#d4a24c"}}},
        "nodes": [
            {"id": 1, "position": [0.0, 0.0], "sections": [{"spans": [{"text": "a"}]}]},
            {"id": 2, "parent": 1, "position": [200.0, 0.0], "sections": [{"spans": [{"text": "b"}]}]}
        ]}));
        let connector = children_of_kind(doc.doc(), node(&doc, 2), DocKind::Connector)
            .next()
            .expect("a connector");
        doc.selection.select_only(connector);
        let own = |doc: &MindMapDocument| match doc.doc().get(connector) {
            Some(DocElement::Connector(c)) => c.path.flow.color,
            _ => panic!("a connector"),
        };
        for (text, want) in [
            (
                "#102030",
                Some(baumhard::document::values::ColorRef::Rgba(
                    baumhard::document::values::Rgba::opaque(0x10, 0x20, 0x30),
                )),
            ),
            (
                "var(accent)",
                Some(baumhard::document::values::ColorRef::Var(
                    baumhard::document::values::VarId(0),
                )),
            ),
            ("  ", None),
        ] {
            let steps = clipboard::paste(doc.doc(), &doc.selection, text, None);
            let _ = apply(&mut doc, steps, "paste");
            assert_eq!(own(&doc), want, "{text:?}");
        }
        for text in ["red", "var(nope)", "#zz"] {
            let steps = clipboard::paste(doc.doc(), &doc.selection, text, None).expect("builds");
            assert!(steps.is_empty(), "{text:?} is not a color");
        }
    }
}
