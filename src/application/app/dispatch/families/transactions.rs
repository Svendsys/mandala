// SPDX-License-Identifier: MPL-2.0

//! Live interaction on transactions (V1-C7, #248): the resize mode, the
//! fast resize, and the color picker's Actions. The drags themselves are
//! not Actions — they are what a press becomes past the threshold
//! (`event_cursor_moved`, `app::live`) — but the two Actions that start a
//! resize are here, with the picker's, which open, stage, commit and
//! cancel its transaction (`app::color_picker_flow`).
//!
//! Native-only, as the drag-state machine and the picker are (CLAUDE.md
//! "Dual-target status"): every Action here is `NativeOnly`, and the
//! browser answers each `Unhandled` with its one-shot warning.

#![cfg(not(target_arch = "wasm32"))]

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;

use super::super::super::input_context::InputHandlerContext;
use super::super::super::interaction_mode::{InteractionMode, ResizeTarget};
use super::super::super::live::grab;
use super::super::cross_dispatch::{DispatchHit, DispatchOutcome};
use crate::application::document::selection::kind_of;
use crate::application::keybinds::Action;
use crate::application::platform::input::MouseButton;
use crate::application::view::handles::ResizeSide;

/// Why `EnterResizeMode` or `FastResizeStart` started nothing — one
/// sentence each, the reason IPC `act.action` returns. Named so the tests
/// hold the reply to the arm without copying a string.
pub(in crate::application::app) struct ResizeRefusal;

impl ResizeRefusal {
    /// The resize mode resizes one thing.
    pub(in crate::application::app) const NOT_ONE: &'static str = "select one node or section to resize";
    /// `FastResizeStart` reached with no pointer gesture behind it.
    pub(in crate::application::app) const NO_POINTER: &'static str =
        "FastResizeStart needs a pointer gesture to anchor at";
    /// No map.
    pub(in crate::application::app) const NO_DOCUMENT: &'static str = "no document is loaded";
    /// A hit no button holds: no release could commit the drag.
    pub(in crate::application::app) const NO_HOLDER: &'static str =
        "FastResizeStart needs a held mouse button to finalize it, and nothing holds this gesture";
    /// A button whose release commits no drag.
    pub(in crate::application::app) const UNFINALIZING_BUTTON: &'static str =
        "only the left and right buttons finalize a drag";
    /// The press landed on no node.
    pub(in crate::application::app) const NOT_ON_A_NODE: &'static str = "the press did not land on a node";
    /// The pressed node is not drawn.
    pub(in crate::application::app) const NODE_GONE: &'static str = "the pressed node is not drawn";
    /// Another interactive transaction is open (D28); the status line
    /// says so too.
    pub(in crate::application::app) const BUSY: &'static str = crate::application::document::TRANSACTION_OPEN_STATUS;
}

/// The family's answer to `action`, or `None` for an Action it does not
/// own. Asked by the native funnel after the cross-platform stage
/// (`dispatch::native::dispatch_action`).
pub(in crate::application::app) fn dispatch(
    action: &Action,
    ctx: &mut InputHandlerContext<'_>,
    hit: Option<&DispatchHit>,
) -> Option<DispatchOutcome> {
    match action {
        Action::EnterResizeMode => Some(enter_resize_mode(ctx)),
        Action::FastResizeStart => Some(match fast_resize_start(ctx, hit) {
            Ok(()) => DispatchOutcome::Handled,
            Err(reason) => {
                log::debug!("dispatch: FastResizeStart refused: {reason}");
                DispatchOutcome::Refused(reason)
            }
        }),
        _ => super::super::super::color_picker_flow::dispatch(action, ctx),
    }
}

/// `EnterResizeMode`: the handles are shown on the one selected node or
/// section, until a click that grabs none or `ExitMode` leaves the mode.
/// The view follows the mode at the next frame.
fn enter_resize_mode(ctx: &mut InputHandlerContext<'_>) -> DispatchOutcome {
    let Some(doc) = ctx.document.as_ref() else {
        return DispatchOutcome::Refused(ResizeRefusal::NO_DOCUMENT);
    };
    match ResizeTarget::of(doc.doc(), &doc.selection) {
        Some(target) => {
            *ctx.interaction_mode = InteractionMode::Resize { target };
            DispatchOutcome::Handled
        }
        None => DispatchOutcome::Refused(ResizeRefusal::NOT_ONE),
    }
}

/// `FastResizeStart`: a resize of the node the press landed on, by the
/// corner nearest the press, held by the button that pressed — a drag
/// like a handle's from there on (`app::live`). Everything comes off
/// `hit`, which the drag's threshold cross builds from the press (#194).
fn fast_resize_start(ctx: &mut InputHandlerContext<'_>, hit: Option<&DispatchHit>) -> Result<(), &'static str> {
    let hit = hit.ok_or(ResizeRefusal::NO_POINTER)?;
    let held_by = match hit.held_by {
        Some(button @ (MouseButton::Left | MouseButton::Right)) => button,
        Some(_) => return Err(ResizeRefusal::UNFINALIZING_BUTTON),
        None => return Err(ResizeRefusal::NO_HOLDER),
    };
    let (Some(doc), Some(view)) = (ctx.document.as_ref(), ctx.view.as_ref()) else {
        return Err(ResizeRefusal::NO_DOCUMENT);
    };
    let node: NodeId = match hit.click_hit {
        super::super::super::ClickHit::Element(id) if kind_of(doc.doc(), id) == Some(DocKind::Node) => id,
        _ => return Err(ResizeRefusal::NOT_ON_A_NODE),
    };
    let rect = view.projector().node_rect(node).ok_or(ResizeRefusal::NODE_GONE)?;
    let side = ResizeSide::corner_for(hit.canvas_pos, rect);
    let gesture = grab::resize_node(doc.doc(), view.projector(), node, side).ok_or(ResizeRefusal::NODE_GONE)?;
    let cursor = ctx
        .renderer
        .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
    super::super::super::event_cursor_moved::start_drag(ctx, gesture, held_by, cursor - hit.canvas_pos, cursor);
    if matches!(*ctx.drag_state, super::super::super::DragState::Dragging(_)) {
        Ok(())
    } else {
        Err(ResizeRefusal::BUSY)
    }
}
