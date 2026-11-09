// SPDX-License-Identifier: MPL-2.0

//! Field editing I on the one writer (V1-C5, #246): the Actions that set
//! the fields of nodes, sections, frames and their rings, and the style
//! of every selected element.
//!
//! **One Action, one request, one undo entry**, whatever the selection
//! holds: each body reads its payload and hands the selection to
//! `crate::application::fields`, whose steps are one request through
//! [`MindMapDocument::apply`] as `ChangeSource::Action` — the keystroke,
//! a macro step and IPC `act.action` alike. A value that does not read,
//! or a selection the Action reaches nothing in, applies nothing and is
//! logged; a D28 refusal is the status line's, and `act.action`'s
//! `reason` (`DispatchOutcome::Refused`, #194).
//!
//! The frame preview is a transaction (§9): `SetFramePreview` opens it
//! and stages, `CommitFramePreview` makes the stage one entry, and
//! `CancelFramePreview` — or `ExitMode` — reverts it. Every Action here
//! runs on both targets.

use baumhard::document::writer::ChangeSource;
use glam::Vec2;

use crate::application::document::{Interaction, MindMapDocument, TRANSACTION_OPEN_STATUS};
use crate::application::fields::{self, frame, section, style, values, Refused, Steps};
use crate::application::keybinds::{Action, FontSlot};

use super::super::cross_dispatch::{view_ctx, DispatchOutcome};
use crate::application::app::input_context_core::InputContextCore;

/// The field family's answer to `action`, or `None` for an Action it
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
        Action::SetColor { .. }
            | Action::SetFont { .. }
            | Action::SetFontFamily(_)
            | Action::SetZoom { .. }
            | Action::ClearZoom
            | Action::SetFrameField { .. }
            | Action::CycleFrameContent
            | Action::ToggleFrameVisible
            | Action::SetFramePreview { .. }
            | Action::CommitFramePreview
            | Action::CancelFramePreview
            | Action::SetSectionOffsetDelta { .. }
            | Action::SetSectionOffsetAbs { .. }
            | Action::SetSectionSizeAbs { .. }
            | Action::SetSectionSizeFillParent
            | Action::SetSectionText { .. }
            | Action::AddSection { .. }
            | Action::DeleteSection
            | Action::SplitSection { .. }
    )
}

/// Run `action`, one of this family's, on `doc`. Pure of the view, so
/// the tests drive it on a document alone.
pub(in crate::application::app) fn run(action: &Action, doc: &mut MindMapDocument) -> DispatchOutcome {
    let result = match action {
        Action::CommitFramePreview => return answer(Ok(doc.commit(Interaction::FramePreview).is_some())),
        Action::CancelFramePreview => return answer(Ok(doc.cancel(Interaction::FramePreview))),
        Action::SetFramePreview {
            target_kind,
            field,
            value,
        } => frame::preview(doc, *target_kind, &[(field, value)], ChangeSource::Action).map(|()| true),
        other => {
            let (steps, label) = steps(other, doc);
            fields::apply(doc, steps, label, ChangeSource::Action)
        }
    };
    answer(result)
}

/// The steps of a one-request Action, and the label its entry carries.
fn steps(action: &Action, doc: &MindMapDocument) -> (Steps, &'static str) {
    let pair = |x: &str, y: &str| Ok::<_, String>(Vec2::new(values::number(x)?, values::number(y)?));
    match action {
        Action::SetColor { axis, value } => (
            values::color(doc.doc(), value).and_then(|color| style::color(doc, *axis, color)),
            "color",
        ),
        Action::SetFontFamily(family) => (
            values::font(family).and_then(|family| {
                style::font(
                    doc,
                    style::Font {
                        family: Some(family),
                        ..Default::default()
                    },
                )
            }),
            "font",
        ),
        Action::SetFont { slot, value } => (
            font_slot(*slot, value).and_then(|font| style::font(doc, font)),
            "font",
        ),
        Action::SetZoom { bound, value } => (
            values::zoom_bound(value).and_then(|value| style::zoom(doc, style::Zoom::bound(*bound, value))),
            "zoom",
        ),
        Action::ClearZoom => (style::zoom(doc, style::Zoom::CLEAR), "zoom"),
        Action::SetFrameField { field, value } => (
            frame::FrameField::read(doc.doc(), field, value).and_then(|field| frame::set(doc, &[field])),
            "frame",
        ),
        Action::CycleFrameContent => (frame::cycle(doc), "frame cycle"),
        Action::ToggleFrameVisible => (frame::toggle(doc), "frame toggle"),
        Action::SetSectionOffsetDelta { dx, dy } => (
            pair(dx, dy).and_then(|d| section::set(doc, None, section::Field::Nudge(d))),
            "section move",
        ),
        Action::SetSectionOffsetAbs { x, y } => (
            pair(x, y).and_then(|at| section::set(doc, None, section::Field::Offset(at))),
            "section move",
        ),
        Action::SetSectionSizeAbs { w, h } => (
            pair(w, h).and_then(|size| section::set(doc, None, section::Field::MinSize(Some(size)))),
            "section resize",
        ),
        Action::SetSectionSizeFillParent => (
            section::set(doc, None, section::Field::MinSize(None)),
            "section resize",
        ),
        Action::SetSectionText { text } => (section::text(doc, None, text), "section text"),
        Action::AddSection { at, text } => {
            let at = (!at.is_empty()).then(|| values::index(at)).transpose();
            (at.and_then(|at| section::add(doc, at, text)), "section add")
        }
        Action::DeleteSection => (section::delete(doc, None), "section delete"),
        Action::SplitSection { at_grapheme } => (
            values::index(at_grapheme).and_then(|at| section::split(doc, None, at)),
            "section split",
        ),
        _ => (Ok(Vec::new()), ""),
    }
}

/// `SetFont`'s field, read from its value.
fn font_slot(slot: FontSlot, value: &str) -> Result<style::Font, String> {
    let mut font = style::Font::default();
    match slot {
        FontSlot::Size => font.size = Some(values::or_reset(value, values::number)?),
        FontSlot::Weight => font.weight = Some(values::or_reset(value, values::weight)?),
        FontSlot::Italic => font.italic = Some(values::or_reset(value, values::boolean)?),
    }
    Ok(font)
}

/// What a body's result answers: `Handled` when it changed something,
/// `Refused` with the status line for a D28 refusal, and `Unhandled` —
/// logged — when nothing applied.
fn answer(result: Result<bool, Refused>) -> DispatchOutcome {
    match result {
        Ok(true) => DispatchOutcome::Handled,
        Ok(false) => DispatchOutcome::Unhandled,
        Err(Refused::Busy) => DispatchOutcome::Refused(TRANSACTION_OPEN_STATUS),
        Err(Refused::Because(why)) => {
            log::warn!("field edit: {why}");
            DispatchOutcome::Unhandled
        }
    }
}

#[cfg(test)]
mod tests;
