// SPDX-License-Identifier: MPL-2.0

//! **The picker's session on the one writer** (design §9, D28; V1-C7
//! #248, its ruling on C3).
//!
//! The **contextual** picker is an interactive transaction:
//!
//! ```text
//! open: begin ─▶ hover / pick / nudge: stage the color once a frame ─▶ ࿕, Enter: commit (one entry)
//!                                                                  └▶ Esc, outside, close: cancel (as before)
//! ```
//!
//! - **Opening** it ([`open_contextual`]) begins the transaction
//!   (`ChangeSource::Input`, [`Interaction::ColorPicker`]); one already
//!   open refuses it (D28), and the picker does not open.
//! - **A hover, a pick, a nudge, a paste** change the color the wheel
//!   shows ([`OpenPicker::shown_hsv`]) and nothing else; **once a frame**
//!   the drain ([`drain`]) stages that color on the target — the
//!   selection as it stood at the opening, on its axis
//!   (`fields::style::color_of`) — when it differs from the one staged.
//!   Under load the stage runs every `n`th frame, as a drag's does.
//! - **Commit** ([`commit`]) stages the *picked* color — what a click
//!   on a cell or a nudge set, not the one under the pointer — and
//!   commits it, as on `main`: a picker opened and committed without a
//!   pick changes nothing and makes no entry. **Cancel** ([`cancel`])
//!   cancels: the target is byte-for-byte as it was. **Undo** while it is
//!   open cancels its transaction (R36), and the picker closes at the
//!   next frame.
//!
//! The **standalone** palette (`color picker on`) holds no transaction:
//! while it floats, the map is edited as ever, and its ࿕ click applies
//! the picked color to the selection as one `apply` ([`commit`]).

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::resolve::resolve_text;
use baumhard::document::values::{ColorRef, Rgba};
use baumhard::document::writer::transaction::TransactionKind;
use baumhard::document::writer::{ChangeSource, EditRequest};
use baumhard::util::color_conversion::{hsv_to_rgb, rgb_to_hsv};

use super::super::dispatch::cross_dispatch::DispatchOutcome;
use super::super::input_context::InputHandlerContext;
use super::key::{apply_picker_nudge, picker_decline_reason, picker_op_for, PickerOp};
use super::open::{open_color_picker_standalone, open_picker_inner};
use super::rebuild::rebuild_color_picker_overlay;
use crate::application::color_picker::{ColorPickerState, OpenPicker, PickerMode, PickerTarget};
use crate::application::document::selection::kind_of;
use crate::application::document::{Interaction, MindMapDocument, Selection};
use crate::application::fields::style;
use crate::application::keybinds::{Action, ColorAxis};

/// The history entry's label.
const LABEL: &str = "color";

/// Why a contextual picker did not open — the console line's error.
pub(in crate::application::app) struct PickerRefusal;

impl PickerRefusal {
    /// Nothing selected takes the axis's color.
    pub(in crate::application::app) const NOTHING: &'static str = "nothing selected takes this color";
    /// No map is open.
    pub(in crate::application::app) const NO_DOCUMENT: &'static str = "no map is open";
    /// Another interactive transaction is open (D28); the status line
    /// says so too.
    pub(in crate::application::app) const BUSY: &'static str =
        crate::application::document::TRANSACTION_OPEN_STATUS;
}

/// The color `hsv` names, opaque. O(1).
fn color_of(hsv: (f32, f32, f32)) -> ColorRef {
    let [r, g, b] = hsv_to_rgb(hsv.0, hsv.1, hsv.2);
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    ColorRef::Rgba(Rgba::new(byte(r), byte(g), byte(b), 0xFF))
}

/// The steps that set `target`'s color to `hsv`. `Err` for a target
/// nothing of which takes the axis's color.
fn steps(
    doc: &MindMapDocument,
    target: &PickerTarget,
    hsv: (f32, f32, f32),
) -> crate::application::fields::Steps {
    style::color_of(doc.doc(), &target.selection, target.axis, Some(color_of(hsv)))
}

/// The color the picker opens on for `axis` of `selection`: the first
/// selected element's own, as the map resolves it — a node's frame for
/// its border color — with the presentation layer set aside, so the
/// wheel opens on the map's color, never a frame of a performance.
fn seed_of(doc: &mut MindMapDocument, selection: &Selection, axis: ColorAxis) -> (f32, f32, f32) {
    let map = doc.suspended();
    let d = map.doc();
    let Some(first) = selection.iter().next() else {
        return (0.0, 1.0, 1.0);
    };
    let owner: NodeId = match (kind_of(d, first), axis) {
        (Some(DocKind::Node), ColorAxis::Border | ColorAxis::Ring) => {
            style::frame_of(d, first).unwrap_or(first)
        }
        _ => first,
    };
    let [r, g, b, _] = resolve_text(d.tree(), owner, &mut ()).color.0;
    rgb_to_hsv(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
}

/// Open the contextual picker on the selection's `axis` color: begin
/// its transaction and open the wheel on the color the target has. The
/// picker does not open when nothing selected takes the color, or when
/// the writer refuses the transaction (D28) — reported on the status
/// line as every refusal is. An open picker of either mode is replaced:
/// a contextual one is cancelled first.
pub(in crate::application::app) fn open_contextual(
    ctx: &mut InputHandlerContext<'_>,
    axis: ColorAxis,
) -> Result<(), &'static str> {
    close(ctx);
    let Some(doc) = ctx.document.as_mut() else {
        return Err(PickerRefusal::NO_DOCUMENT);
    };
    let target = PickerTarget {
        axis,
        selection: doc.selection.clone(),
    };
    if steps(doc, &target, (0.0, 0.0, 0.0)).map_or(true, |steps| steps.is_empty()) {
        return Err(PickerRefusal::NOTHING);
    }
    let seed_hsv = seed_of(doc, &target.selection, axis);
    if !doc.begin(
        ChangeSource::Input,
        TransactionKind::Interactive,
        Interaction::ColorPicker,
    ) {
        return Err(PickerRefusal::BUSY);
    }
    open_picker_inner(
        PickerMode::Contextual { seed_hsv, target },
        ctx.color_picker_state,
        ctx.app_scene,
        ctx.renderer,
    );
    Ok(())
}

/// The frame's drain: for a contextual picker whose transaction is
/// open, stage the color the wheel shows on its target when it differs
/// from the one staged and the frame is due — always when `throttled` is
/// false (a virtual clock). A contextual picker whose transaction was
/// closed under it — an undo cancelled it — closes. Returns when it
/// staged, for the throttle to be told what the frame cost.
pub(in crate::application::app) fn drain(
    state: &mut ColorPickerState,
    document: &mut Option<MindMapDocument>,
    throttled: bool,
) -> Option<std::time::Instant> {
    let doc = document.as_mut()?;
    let picker = state.as_open_mut()?;
    picker.target()?;
    if doc.open_interaction() != Some(Interaction::ColorPicker) {
        *state = ColorPickerState::Closed;
        return None;
    }
    let shown = picker.shown_hsv();
    if picker.staged == Some(shown) || (throttled && !picker.throttle.should_drain()) {
        return None;
    }
    let started = std::time::Instant::now();
    stage(doc, picker, shown);
    Some(started)
}

/// Stage `hsv` on `picker`'s target, and remember it staged. A refusal
/// is reported by [`MindMapDocument::stage`]; the stage before it stands.
fn stage(doc: &mut MindMapDocument, picker: &mut OpenPicker, hsv: (f32, f32, f32)) {
    picker.staged = Some(hsv);
    let Some(target) = picker.target() else {
        return;
    };
    match steps(doc, target, hsv) {
        Ok(steps) => {
            let _ = doc.stage(Interaction::ColorPicker, EditRequest::new(steps, LABEL));
        }
        Err(why) => log::debug!("color picker: not staged: {why}"),
    }
}

/// Commit the picker (`PickerCommit`, the ࿕ click). Contextual: stage
/// the picked color and commit it as one entry, or — when nothing was
/// picked, the picked color still the one it opened on — cancel, making
/// none; then close. Standalone: apply the picked color to the
/// selection's text color as one `apply`, and stay open. Returns whether
/// the document changed.
pub(in crate::application::app) fn commit(ctx: &mut InputHandlerContext<'_>) -> bool {
    let Some(doc) = ctx.document.as_mut() else {
        return false;
    };
    let Some(picker) = ctx.color_picker_state.as_open_mut() else {
        return false;
    };
    let picked = (picker.hue_deg, picker.sat, picker.val);
    let seed = match &picker.mode {
        PickerMode::Contextual { seed_hsv, .. } => Some(*seed_hsv),
        PickerMode::Standalone => None,
    };
    let changed = match seed {
        Some(seed) if bits(picked) == bits(seed) => {
            doc.cancel(Interaction::ColorPicker);
            false
        }
        Some(_) => {
            if picker.staged != Some(picked) {
                stage(doc, picker, picked);
            }
            doc.commit(Interaction::ColorPicker).is_some()
        }
        None => {
            let target = PickerTarget {
                axis: ColorAxis::Text,
                selection: doc.selection.clone(),
            };
            let applied = match steps(doc, &target, picked) {
                Ok(steps) if !steps.is_empty() => doc
                    .apply(EditRequest::new(steps, LABEL), ChangeSource::Input)
                    .is_ok(),
                _ => false,
            };
            if !applied {
                crate::application::color_picker::request_error_flash(
                    ctx.color_picker_state,
                    crate::application::color_picker::FlashKind::Error,
                );
            }
            applied
        }
    };
    if seed.is_some() {
        *ctx.color_picker_state = ColorPickerState::Closed;
    }
    finish(ctx);
    changed
}

/// Cancel the contextual picker (`PickerCancel`, Esc, a click outside
/// it): its transaction is cancelled, and the target is as it was; the
/// picker closes. Returns whether one was open.
pub(in crate::application::app) fn cancel(ctx: &mut InputHandlerContext<'_>) -> bool {
    let contextual = ctx
        .color_picker_state
        .as_open()
        .is_some_and(|picker| picker.target().is_some());
    if !contextual {
        return false;
    }
    close(ctx);
    finish(ctx);
    true
}

/// Close whatever picker is open — a contextual one cancelled — with no
/// sync: what replacing it, and `CloseColorPicker`, start with.
fn close(ctx: &mut InputHandlerContext<'_>) {
    let contextual = ctx
        .color_picker_state
        .as_open()
        .is_some_and(|picker| picker.target().is_some());
    if contextual {
        if let Some(doc) = ctx.document.as_mut() {
            doc.cancel(Interaction::ColorPicker);
        }
    }
    *ctx.color_picker_state = ColorPickerState::Closed;
}

/// After a commit, a cancel or a close: the overlay follows the picker's
/// state — gone when it closed — and the view draws the map.
fn finish(ctx: &mut InputHandlerContext<'_>) {
    rebuild_color_picker_overlay(ctx.color_picker_state, ctx.app_scene, ctx.renderer);
    if let (Some(doc), Some(view)) = (ctx.document.as_mut(), ctx.view.as_mut()) {
        view.follow_picker(picking(ctx.color_picker_state));
        view.sync(doc, ctx.renderer);
    }
}

/// What the view's selection tint stands down for
/// (`MapView::follow_picker`): an open contextual picker's axis and the
/// elements it colors; `None` for the standalone palette, which previews
/// nothing on the map, and with no picker open. O(1).
pub(in crate::application::app) fn picking(
    state: &ColorPickerState,
) -> Option<(ColorAxis, &[baumhard::arena::NodeId])> {
    let target = state.as_open()?.target()?;
    Some((target.axis, target.selection.items()))
}

/// An HSV triple bit for bit, so a picker that was never moved compares
/// equal to its seed even where `==` on floats is fragile. O(1).
fn bits(hsv: (f32, f32, f32)) -> (u32, u32, u32) {
    (hsv.0.to_bits(), hsv.1.to_bits(), hsv.2.to_bits())
}

/// The picker's Actions: `OpenColorPicker` (the standalone palette),
/// `CloseColorPicker`, and every one [`picker_op_for`] names. `None` for
/// any other Action. A picker Action with nothing to act on — the picker
/// closed, a cancel of the standalone palette, no map — is `Unhandled`,
/// so a macro step reports that nothing ran.
pub(in crate::application::app) fn dispatch(
    action: &Action,
    ctx: &mut InputHandlerContext<'_>,
) -> Option<DispatchOutcome> {
    match action {
        Action::OpenColorPicker => {
            close(ctx);
            open_color_picker_standalone(ctx.color_picker_state, ctx.app_scene, ctx.renderer);
            finish(ctx);
            return Some(DispatchOutcome::Handled);
        }
        Action::CloseColorPicker => {
            if !ctx.color_picker_state.is_open() {
                return Some(DispatchOutcome::Unhandled);
            }
            close(ctx);
            finish(ctx);
            return Some(DispatchOutcome::Handled);
        }
        _ => {}
    }
    let op = picker_op_for(action)?;
    if picker_decline_reason(
        op,
        ctx.color_picker_state.is_open(),
        ctx.color_picker_state.is_standalone(),
        ctx.document.is_some(),
    )
    .is_some()
    {
        return Some(DispatchOutcome::Unhandled);
    }
    match op {
        PickerOp::Cancel => {
            cancel(ctx);
        }
        PickerOp::Commit => {
            commit(ctx);
        }
        PickerOp::Nudge(nudge) => {
            apply_picker_nudge(nudge, ctx.color_picker_state);
            rebuild_color_picker_overlay(ctx.color_picker_state, ctx.app_scene, ctx.renderer);
        }
    }
    Some(DispatchOutcome::Handled)
}

/// A `color` line's picker subverb (`commands::color::PickerLine`): open
/// the contextual picker on the selection's axis, open the standalone
/// palette, or close the open picker. The report is the line's output: a
/// refusal names why nothing opened.
pub(in crate::application::app) fn take(
    ctx: &mut InputHandlerContext<'_>,
    line: crate::application::console::commands::color::PickerLine,
) -> crate::application::console::ExecResult {
    use crate::application::console::commands::color::PickerLine;
    use crate::application::console::ExecResult;
    match line {
        PickerLine::Open(axis) => match open_contextual(ctx, axis) {
            Ok(()) => ExecResult::ok_empty(),
            Err(why) => ExecResult::err(format!("color: {why}")),
        },
        PickerLine::Palette => {
            close(ctx);
            open_color_picker_standalone(ctx.color_picker_state, ctx.app_scene, ctx.renderer);
            finish(ctx);
            ExecResult::ok_empty()
        }
        PickerLine::Close => {
            close(ctx);
            finish(ctx);
            ExecResult::ok_empty()
        }
    }
}

/// The steps a contextual picker on `target` stages for `hsv`: for the
/// tests that hold a stage to one `apply` of the same request.
#[cfg(test)]
pub(in crate::application::app) fn steps_for_test(
    doc: &MindMapDocument,
    target: &PickerTarget,
    hsv: (f32, f32, f32),
) -> crate::application::fields::Steps {
    steps(doc, target, hsv)
}
