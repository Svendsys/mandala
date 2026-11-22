// SPDX-License-Identifier: MPL-2.0

//! The live interactions' renderer-free parts: the resize arithmetic,
//! the side a dragged end attaches to, and a drag's input since the
//! press. What needs the projector and the writer together — every
//! interaction through synthetic input — is `app::tests_gpu_live`'s
//! (TEST_CONVENTIONS §T8).

use baumhard::document::values::Anchor;
use baumhard::flow::Rect;
use glam::Vec2;

use super::nearest_side;
use crate::application::view::handles::ResizeSide;

/// The box every resize below starts from: 100 × 50 at (10, 20).
fn start() -> Rect {
    Rect::new(Vec2::new(10.0, 20.0), Vec2::new(100.0, 50.0))
}

/// **Each handle moves the edges it is on and no other**: the south-east
/// corner grows the size and leaves the top-left; the north-west corner
/// moves the top-left by the drag and shrinks the size by as much, so the
/// far corner stays where it was; an edge handle moves one axis.
///
/// Control: every side dragged by nothing is the start box, so a side
/// that moved what it should not is told from one that moved nothing.
#[test]
fn test_each_resize_handle_moves_its_own_edges() {
    let d = Vec2::new(7.0, 3.0);
    let r = |side: ResizeSide| side.resolve(start(), d, Vec2::ZERO);
    assert_eq!(r(ResizeSide::SE), Rect::new(Vec2::new(10.0, 20.0), Vec2::new(107.0, 53.0)));
    assert_eq!(r(ResizeSide::NW), Rect::new(Vec2::new(17.0, 23.0), Vec2::new(93.0, 47.0)));
    assert_eq!(r(ResizeSide::NW).max(), start().max(), "the far corner stays put");
    assert_eq!(r(ResizeSide::N), Rect::new(Vec2::new(10.0, 23.0), Vec2::new(100.0, 47.0)));
    assert_eq!(r(ResizeSide::E), Rect::new(Vec2::new(10.0, 20.0), Vec2::new(107.0, 50.0)));
    assert_eq!(r(ResizeSide::NE), Rect::new(Vec2::new(10.0, 23.0), Vec2::new(107.0, 47.0)));
    assert_eq!(r(ResizeSide::SW), Rect::new(Vec2::new(17.0, 20.0), Vec2::new(93.0, 53.0)));
    assert_eq!(r(ResizeSide::S), Rect::new(Vec2::new(10.0, 20.0), Vec2::new(100.0, 53.0)));
    assert_eq!(r(ResizeSide::W), Rect::new(Vec2::new(17.0, 20.0), Vec2::new(93.0, 50.0)));
    for side in ResizeSide::ALL {
        assert_eq!(side.resolve(start(), Vec2::ZERO, Vec2::ZERO), start(), "{side:?}");
    }
}

/// **A box cannot be dragged smaller than its floor, and a top or left
/// handle then stops rather than slide the box**: pulling the north-west
/// corner far past the content holds the size at the floor, and the
/// top-left where that size puts it against the far corner, which does
/// not move.
///
/// Control: the same drag with no floor shrinks the box to nothing.
#[test]
fn test_a_resize_stops_at_its_floor_without_sliding() {
    let floor = Vec2::new(40.0, 30.0);
    let held = ResizeSide::NW.resolve(start(), Vec2::new(500.0, 500.0), floor);
    assert_eq!(held.size, floor);
    assert_eq!(held.max(), start().max(), "the far corner holds");
    let unheld = ResizeSide::NW.resolve(start(), Vec2::new(500.0, 500.0), Vec2::ZERO);
    assert_eq!(unheld.size, Vec2::ZERO);
    let grown = ResizeSide::SE.resolve(start(), Vec2::new(-500.0, -500.0), floor);
    assert_eq!(grown, Rect::new(start().origin, floor), "a south-east drag inward stops too");
}

/// **A fast resize grabs the corner of the quadrant it is pressed in**;
/// a press on the center lines rounds south and east, as on `main`.
#[test]
fn test_a_fast_resize_grabs_the_corner_of_its_quadrant() {
    let rect = start();
    let c = rect.center();
    assert_eq!(ResizeSide::corner_for(rect.origin, rect), ResizeSide::NW);
    assert_eq!(ResizeSide::corner_for(Vec2::new(rect.max().x, rect.origin.y), rect), ResizeSide::NE);
    assert_eq!(ResizeSide::corner_for(Vec2::new(rect.origin.x, rect.max().y), rect), ResizeSide::SW);
    assert_eq!(ResizeSide::corner_for(rect.max(), rect), ResizeSide::SE);
    assert_eq!(ResizeSide::corner_for(c, rect), ResizeSide::SE, "the center rounds south-east");
}

/// **A dragged path end attaches to the side of its node nearest the
/// pointer**: each side's midpoint, approached from outside, picks that
/// side.
#[test]
fn test_a_dragged_end_attaches_to_the_nearest_side() {
    let rect = start();
    let c = rect.center();
    assert_eq!(nearest_side(rect, Vec2::new(c.x, rect.origin.y - 30.0)), Anchor::Top);
    assert_eq!(nearest_side(rect, Vec2::new(rect.max().x + 30.0, c.y)), Anchor::Right);
    assert_eq!(nearest_side(rect, Vec2::new(c.x, rect.max().y + 30.0)), Anchor::Bottom);
    assert_eq!(nearest_side(rect, Vec2::new(rect.origin.x - 30.0, c.y)), Anchor::Left);
}
