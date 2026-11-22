// SPDX-License-Identifier: MPL-2.0

//! Field editing II on the one writer (V1-C6, #247): the Actions that
//! set the fields of the paths between nodes — connectors and links —
//! their caps and labels, and a link's kind. `SetEdgeType` is gone: a
//! connector is always a line, and a cross-link is a link (D22).
//!
//! **One Action, one request, one undo entry**, whatever the selection
//! holds: each body reads its payload as the console verb that sets the
//! same field reads it, and hands the selection to
//! [`crate::application::fields::path`], whose steps are one request
//! through [`MindMapDocument::apply`] as `ChangeSource::Action` — the
//! keystroke, a macro step and IPC `act.action` alike. A value that does
//! not read, or a selection the Action reaches nothing in, applies
//! nothing and is logged; a refusal the user is told of — `SetLinkKind`
//! on a connector, a label Action on a portal link, a D28 refusal — is
//! the status line's, and `act.action`'s `reason`
//! (`DispatchOutcome::Refused`, #194). Every Action here runs on both
//! targets.

use baumhard::document::writer::ChangeSource;

use crate::application::document::{MindMapDocument, TRANSACTION_OPEN_STATUS};
use crate::application::fields::path::{self, Geometry, LabelFields, PathFields};
use crate::application::fields::{self, values, Refused};
use crate::application::keybinds::Action;

use super::super::cross_dispatch::{view_ctx, DispatchOutcome};
use crate::application::app::input_context_core::InputContextCore;

/// The path family's answer to `action`, or `None` for an Action it
/// does not own. The view is synced after the body, so a status line it
/// left is drawn even when nothing changed.
pub(in crate::application::app) fn dispatch(
    action: &Action,
    core: &mut InputContextCore<'_>,
) -> Option<DispatchOutcome> {
    if !owns(action) {
        return None;
    }
    let doc = core.document.as_deref_mut()?;
    let outcome = run(action, doc);
    if let Some(mut vc) = view_ctx!(core, doc) {
        vc.sync();
    }
    Some(outcome)
}

/// Whether `action` is this family's.
fn owns(action: &Action) -> bool {
    matches!(
        action,
        Action::SetPathAnchor { .. }
            | Action::SetPathContent(_)
            | Action::SetPathCap { .. }
            | Action::SetLinkKind(_)
            | Action::ResetPath
            | Action::SetLabelText(_)
            | Action::SetLabelPosition { .. }
            | Action::SetPathSpacing(_)
    )
}

/// Run `action`, one of this family's, on `doc`. Pure of the view, so
/// the tests drive it on a document alone.
pub(in crate::application::app) fn run(action: &Action, doc: &mut MindMapDocument) -> DispatchOutcome {
    let (steps, label) = steps(action, doc);
    answer(fields::apply(doc, steps, label, ChangeSource::Action))
}

/// The steps of a one-request Action, and the label its entry carries.
fn steps(action: &Action, doc: &MindMapDocument) -> (path::Built, &'static str) {
    let fields = |fields: Result<PathFields, String>| -> path::Built {
        Ok(path::set(doc, &fields?)?)
    };
    match action {
        Action::SetPathAnchor { from, to } => (
            fields(values::anchor(from).and_then(|from| {
                Ok(PathFields {
                    anchors: Some((from, values::anchor(to)?)),
                    ..PathFields::default()
                })
            })),
            "path anchors",
        ),
        Action::SetPathContent(content) => (
            fields(Ok(PathFields {
                content: Some(values::content(content)),
                ..PathFields::default()
            })),
            "path content",
        ),
        Action::SetPathSpacing(spacing) => (
            fields(values::or_reset(spacing, values::number).map(|spacing| PathFields {
                spacing: Some(spacing),
                ..PathFields::default()
            })),
            "path spacing",
        ),
        Action::SetPathCap { from, to } => {
            // An empty end is left as it is.
            let end = |text: &str| (!text.is_empty()).then(|| values::content(text));
            let (start, end) = (end(from), end(to));
            (
                path::caps(
                    doc,
                    start.as_ref().map(Option::as_deref),
                    end.as_ref().map(Option::as_deref),
                )
                .map_err(Refused::from),
                "path cap",
            )
        }
        Action::ResetPath => (
            path::geometry(doc, Geometry::Reset).map_err(Refused::from),
            "path reset",
        ),
        Action::SetLinkKind(kind) => (
            values::link_kind(kind)
                .map_err(Refused::from)
                .and_then(|kind| path::link_kind(doc, kind)),
            "link kind",
        ),
        Action::SetLabelText(text) if text.is_empty() => (path::delete_label(doc), "label delete"),
        Action::SetLabelText(text) => (
            path::label(
                doc,
                &LabelFields {
                    text: Some(text.clone()),
                    ..LabelFields::default()
                },
            ),
            "label text",
        ),
        Action::SetLabelPosition { t, offset } => (
            label_position(t, offset)
                .map_err(Refused::from)
                .and_then(|fields| path::label(doc, &fields)),
            "label position",
        ),
        _ => (Ok(Vec::new()), ""),
    }
}

/// `SetLabelPosition`'s fields: each of `t` and `offset` left as it is
/// when empty, inherited for `reset`, and otherwise read.
fn label_position(t: &str, offset: &str) -> Result<LabelFields, String> {
    let given = |text: &str, read: fn(&str) -> Result<f32, String>| {
        (!text.is_empty())
            .then(|| values::or_reset(text, read))
            .transpose()
    };
    let fields = LabelFields {
        text: None,
        t: given(t, values::label_t)?,
        offset: given(offset, values::number)?,
    };
    if fields.t.is_none() && fields.offset.is_none() {
        return Err("label position: give t or offset".to_string());
    }
    Ok(fields)
}

/// What a body's result answers: `Handled` when it changed something,
/// `Refused` with the status line for a refusal the user is told of,
/// and `Unhandled` — logged — when nothing applied.
fn answer(result: Result<bool, Refused>) -> DispatchOutcome {
    match result {
        Ok(true) => DispatchOutcome::Handled,
        Ok(false) => DispatchOutcome::Unhandled,
        Err(Refused::Busy) => DispatchOutcome::Refused(TRANSACTION_OPEN_STATUS),
        Err(Refused::Declined(why)) => DispatchOutcome::Refused(why),
        Err(Refused::Because(why)) => {
            log::warn!("path edit: {why}");
            DispatchOutcome::Unhandled
        }
    }
}

#[cfg(test)]
mod tests;
