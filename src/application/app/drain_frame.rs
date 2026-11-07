// SPDX-License-Identifier: MPL-2.0

//! The frame's input half on native: what runs once per loop iteration
//! after the queued input has been handled — the rubber band, which
//! follows the pointer every frame.
//!
//! **The rubber band's authority is the drag state**
//! (`DragState::SelectingRect`). Each frame re-derives its two artifacts
//! from it — the rectangle drawn on the canvas and the covered set the
//! selection highlight tints (`MapView::set_band_targets`) — so no exit
//! from the gesture has to remember to clean either up. The covered set
//! is the projector's answer (`MapView::nodes_in`, through its body
//! index), and the highlight is re-stamped only when that set changes.

#![cfg(not(target_arch = "wasm32"))]

use baumhard::flow::Rect;
use glam::Vec2;

use super::dispatch::cross_dispatch::view_ctx;
use super::input_context::InputHandlerContext;
use super::DragState;
use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;
use crate::application::view::MapView;

/// Bring the rubber band's two artifacts up to date with the drag state:
/// while it is out, the rectangle follows the pointer and the highlight
/// tints what it covers; once it is not, both are gone.
pub(super) fn drain_rect_select(
    drag_state: &DragState,
    document: &mut Option<MindMapDocument>,
    view: &mut Option<MapView>,
    renderer: &mut Renderer,
) {
    let (Some(doc), Some(view)) = (document.as_mut(), view.as_mut()) else {
        return;
    };
    match rect_select_frame(drag_state) {
        RectSelectFrame::Track { rect } => {
            renderer.rebuild_selection_rect_overlay(rect.origin, rect.max());
            if view.set_band_targets(Some(view.nodes_in(rect))) {
                view.sync(doc, renderer);
            }
        }
        RectSelectFrame::End => {
            if view.band_targets().is_some() {
                renderer.clear_selection_rect_overlay();
                view.set_band_targets(None);
                view.sync(doc, renderer);
            }
        }
    }
}

/// What a frame owes the rubber band, given the drag state.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RectSelectFrame {
    /// A band is out over `rect`.
    Track { rect: Rect },
    /// No band is out: whatever one left is ended.
    End,
}

/// Pure, so every way into and out of the band is pinnable without a
/// renderer (TEST_CONVENTIONS §T8).
fn rect_select_frame(drag_state: &DragState) -> RectSelectFrame {
    match *drag_state {
        DragState::SelectingRect {
            start_canvas,
            current_canvas,
        } => RectSelectFrame::Track {
            rect: band(start_canvas, current_canvas),
        },
        _ => RectSelectFrame::End,
    }
}

/// The rectangle between two corners, either way round.
fn band(a: Vec2, b: Vec2) -> Rect {
    let min = a.min(b);
    Rect::new(min, a.max(b) - min)
}

/// The rubber band's release: it selects what it covers — replacing the
/// selection — and the band's tint stands down. The rectangle goes at the
/// next frame (`drain_rect_select`).
pub(super) fn end_rect_select_gesture(
    ctx: &mut InputHandlerContext<'_>,
    start_canvas: Vec2,
    current_canvas: Vec2,
) {
    let Some(doc) = ctx.document.as_mut() else {
        return;
    };
    let Some(view) = ctx.view.as_mut() else {
        return;
    };
    let covered = view.nodes_in(band(start_canvas, current_canvas));
    view.set_band_targets(None);
    doc.selection.set(covered);
    ctx.renderer.clear_selection_rect_overlay();
    if let Some(mut vc) = view_ctx!(ctx, doc) {
        vc.sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::{ClickHit, PanOwner};

    /// A live band tracks the pointer, and the rectangle is the same
    /// whichever way it was dragged.
    #[test]
    fn test_a_live_rubber_band_tracks_its_rectangle_either_way_round() {
        let forward = DragState::SelectingRect {
            start_canvas: Vec2::new(1.0, 2.0),
            current_canvas: Vec2::new(30.0, 40.0),
        };
        let backward = DragState::SelectingRect {
            start_canvas: Vec2::new(30.0, 40.0),
            current_canvas: Vec2::new(1.0, 2.0),
        };
        let want = RectSelectFrame::Track {
            rect: Rect::new(Vec2::new(1.0, 2.0), Vec2::new(29.0, 38.0)),
        };
        assert_eq!(rect_select_frame(&forward), want);
        assert_eq!(rect_select_frame(&backward), want);
    }

    /// Every state that is not a band ends whatever a band left behind:
    /// the band targets are derived from the drag state, so no exit has to
    /// clean it up.
    #[test]
    fn test_every_exit_from_a_rubber_band_ends_it() {
        for drag in [
            DragState::Pending {
                start_pos: (0.0, 0.0),
                hit: ClickHit::Empty,
            },
            DragState::Panning {
                owner: PanOwner::Toggled,
            },
            DragState::None,
            DragState::PendingRight {
                start_pos: (0.0, 0.0),
                start_canvas: Vec2::ZERO,
                hit: ClickHit::Empty,
            },
        ] {
            assert_eq!(rect_select_frame(&drag), RectSelectFrame::End);
        }
    }
}
