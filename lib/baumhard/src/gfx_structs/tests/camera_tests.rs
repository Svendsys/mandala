// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::gfx_structs::camera::Camera2D`] — viewport math
//! fundamentals (§T1).
//!
//! Camera is platform-shared logic that must behave identically on
//! native and WASM. These tests cover the canvas↔screen coordinate
//! round-trip, zoom-at-cursor invariant, pan, fit-to-bounds (including
//! degenerate inputs), `CameraMutation` dispatch, and the pixel ratio a
//! supersampled capture draws through.
//!
//! Follows the `do_*()` / `test_*()` split from §T2.2: every `do_*`
//! body is benchmarkable from `benches/test_bench.rs`.

use glam::Vec2;

use crate::gfx_structs::camera::{Camera2D, CameraMutation, Visibility};
use crate::gfx_structs::zoom_visibility::ZoomVisibility;
use crate::util::geometry::almost_equal_vec2;

// ── round-trip ──────────────────────────────────────────────────────

#[test]
fn test_canvas_to_screen_round_trips() {
    do_canvas_to_screen_round_trips();
}

/// `canvas_to_screen` → `screen_to_canvas` gives back the original
/// point within floating-point epsilon, across several camera states.
pub fn do_canvas_to_screen_round_trips() {
    let configs: Vec<(Vec2, f32)> = vec![
        (Vec2::ZERO, 1.0),
        (Vec2::new(100.0, -200.0), 1.5),
        (Vec2::new(-500.0, 300.0), 0.25),
        (Vec2::new(0.0, 0.0), 3.0),
        (Vec2::new(120.0, -80.0), 0.5),
    ];
    let test_points = vec![
        Vec2::new(0.0, 0.0),
        Vec2::new(50.0, 75.0),
        Vec2::new(-300.0, 400.0),
        Vec2::new(1000.0, -1000.0),
    ];

    for (pos, zoom) in &configs {
        let mut cam = Camera2D::new(1920, 1080);
        cam.position = *pos;
        cam.zoom = *zoom;

        for pt in &test_points {
            let screen = cam.canvas_to_screen(*pt);
            let back = cam.screen_to_canvas(screen);
            assert!(
                almost_equal_vec2(back, *pt),
                "Round-trip failed for camera pos={pos}, zoom={zoom}, point={pt}: got {back}"
            );
        }
    }
}

// ── identity at default ─────────────────────────────────────────────

#[test]
fn test_screen_to_canvas_identity_at_default() {
    do_screen_to_canvas_identity_at_default();
}

/// With default camera (position=0, zoom=1), screen coords equal
/// canvas coords offset by the viewport center — i.e.
/// `screen_to_canvas(screen_center) == Vec2::ZERO`.
pub fn do_screen_to_canvas_identity_at_default() {
    let cam = Camera2D::new(800, 600);
    let screen_center = Vec2::new(400.0, 300.0);

    // The viewport center maps to canvas origin.
    let canvas = cam.screen_to_canvas(screen_center);
    assert!(
        almost_equal_vec2(canvas, Vec2::ZERO),
        "Expected canvas origin at screen center, got {canvas}"
    );

    // An arbitrary screen point maps to (screen - center) in canvas.
    let screen_pt = Vec2::new(500.0, 350.0);
    let expected_canvas = screen_pt - screen_center; // (100, 50)
    let actual_canvas = cam.screen_to_canvas(screen_pt);
    assert!(
        almost_equal_vec2(actual_canvas, expected_canvas),
        "Expected {expected_canvas}, got {actual_canvas}"
    );
}

// ── zoom-at preserves point under cursor ────────────────────────────

#[test]
fn test_zoom_at_preserves_point_under_cursor() {
    do_zoom_at_preserves_point_under_cursor();
}

/// After `zoom_at(cursor, factor)`, the canvas point that was under
/// the cursor before the zoom is still under the cursor after.
pub fn do_zoom_at_preserves_point_under_cursor() {
    let factors = [1.5_f32, 2.0, 0.5, 0.8, 3.5];
    let cursors = [
        Vec2::new(200.0, 150.0),
        Vec2::new(0.0, 0.0),
        Vec2::new(799.0, 599.0),
        Vec2::new(400.0, 300.0),
    ];

    for factor in &factors {
        for cursor in &cursors {
            let mut cam = Camera2D::new(800, 600);
            cam.position = Vec2::new(50.0, -30.0);
            cam.zoom = 1.2;

            let canvas_before = cam.screen_to_canvas(*cursor);
            cam.zoom_at(*cursor, *factor);
            let canvas_after = cam.screen_to_canvas(*cursor);

            assert!(
                almost_equal_vec2(canvas_before, canvas_after),
                "zoom_at({cursor}, {factor}) moved canvas point under cursor: \
                 {canvas_before} → {canvas_after}"
            );
        }
    }
}

// ── pan shifts viewport ─────────────────────────────────────────────

#[test]
fn test_pan_shifts_viewport() {
    do_pan_shifts_viewport();
}

/// `pan(dx, dy)` moves the camera offset by `-delta / zoom` in canvas
/// space (dragging right moves the view right, so the camera position
/// moves left).
pub fn do_pan_shifts_viewport() {
    let mut cam = Camera2D::new(800, 600);
    cam.zoom = 2.0;
    let pos_before = cam.position;

    let delta = Vec2::new(100.0, -50.0);
    cam.pan(delta);

    let expected = pos_before - delta / cam.zoom;
    assert!(
        almost_equal_vec2(cam.position, expected),
        "Expected position {expected}, got {}",
        cam.position
    );
}

// ── fit_to_bounds with a single element ─────────────────────────────

#[test]
fn test_fit_to_bounds_with_single_element() {
    do_fit_to_bounds_with_single_element();
}

/// `fit_camera_to_bounds` with a small bounding box produces a zoom
/// level and offset that centre the content and scale it to fill the
/// usable viewport area.
pub fn do_fit_to_bounds_with_single_element() {
    let mut cam = Camera2D::new(800, 600);
    let min = Vec2::new(90.0, 190.0);
    let max = Vec2::new(110.0, 210.0);
    let padding = 0.05_f32;

    cam.fit_to_bounds(min, max, padding);

    // Camera position should be at the centre of the bounding box.
    let expected_center = (min + max) / 2.0; // (100, 200)
    assert!(
        almost_equal_vec2(cam.position, expected_center),
        "Expected position {expected_center}, got {}",
        cam.position
    );

    // The zoom should scale the 20x20 box into the usable viewport
    // area. Usable width = 800 * 0.9 = 720, usable height = 600 * 0.9
    // = 540. Zoom = min(720/20, 540/20) = 27.0, but clamped to
    // MAX_ZOOM.
    assert_eq!(cam.zoom, Camera2D::MAX_ZOOM);
}

// ── fit_to_bounds with empty/zero-size box ──────────────────────────

#[test]
fn test_fit_to_bounds_empty() {
    do_fit_to_bounds_empty();
}

/// Fitting to an empty (zero-size) bounding box must not panic and
/// should produce a sane camera state — position centred on the
/// degenerate point, zoom unchanged (the `if canvas_size > 0` guard
/// skips the zoom calculation).
pub fn do_fit_to_bounds_empty() {
    let mut cam = Camera2D::new(800, 600);
    let original_zoom = cam.zoom;

    let point = Vec2::new(42.0, -17.0);
    cam.fit_to_bounds(point, point, 0.1);

    // Position should centre on the single point.
    assert!(
        almost_equal_vec2(cam.position, point),
        "Expected position {point}, got {}",
        cam.position
    );

    // Zoom should be unchanged — the zero-size guard leaves it alone.
    assert_eq!(cam.zoom, original_zoom);
}

// ── CameraMutation::Pan ─────────────────────────────────────────────

#[test]
fn test_camera_mutation_pan() {
    do_camera_mutation_pan();
}

/// Applying `CameraMutation::Pan { screen_delta }` via
/// `apply_mutation` shifts the camera position by
/// `-screen_delta / zoom`, identical to calling `pan()` directly.
pub fn do_camera_mutation_pan() {
    let mut cam = Camera2D::new(800, 600);
    cam.position = Vec2::new(10.0, 20.0);
    cam.zoom = 1.5;
    let pos_before = cam.position;

    let delta = Vec2::new(30.0, -45.0);
    cam.apply_mutation(&CameraMutation::Pan { screen_delta: delta });

    let expected = pos_before - delta / 1.5;
    assert!(
        almost_equal_vec2(cam.position, expected),
        "Pan mutation: expected {expected}, got {}",
        cam.position
    );
}

// ── CameraMutation::ZoomAt ──────────────────────────────────────────

#[test]
fn test_camera_mutation_zoom() {
    do_camera_mutation_zoom();
}

/// Applying `CameraMutation::ZoomAt` via `apply_mutation` adjusts
/// the zoom level and preserves the canvas point under the focus.
pub fn do_camera_mutation_zoom() {
    let mut cam = Camera2D::new(800, 600);
    cam.position = Vec2::new(50.0, -30.0);
    cam.zoom = 1.0;

    let focus = Vec2::new(300.0, 200.0);
    let canvas_before = cam.screen_to_canvas(focus);

    cam.apply_mutation(&CameraMutation::ZoomAt {
        screen_focus: focus,
        factor: 2.0,
    });

    // Zoom should have doubled.
    assert!(
        (cam.zoom - 2.0).abs() < 1e-6,
        "Expected zoom 2.0, got {}",
        cam.zoom
    );

    // The canvas point under the focus should be unchanged.
    let canvas_after = cam.screen_to_canvas(focus);
    assert!(
        almost_equal_vec2(canvas_before, canvas_after),
        "ZoomAt mutation moved focus point: {canvas_before} → {canvas_after}"
    );
}

// ── CameraMutation::SetZoom clamps to bounds ────────────────────────

#[test]
fn test_camera_mutation_set_zoom_clamps_to_bounds() {
    do_camera_mutation_set_zoom_clamps_to_bounds();
}

/// `SetZoom` clamps to `MIN_ZOOM..=MAX_ZOOM` so an animation tween
/// that overshoots the bounds doesn't produce a degenerate camera.
pub fn do_camera_mutation_set_zoom_clamps_to_bounds() {
    let mut cam = Camera2D::new(800, 600);
    cam.apply_mutation(&CameraMutation::SetZoom { factor: 100.0 });
    assert_eq!(cam.zoom, Camera2D::MAX_ZOOM);
    cam.apply_mutation(&CameraMutation::SetZoom { factor: 0.0001 });
    assert_eq!(cam.zoom, Camera2D::MIN_ZOOM);
}

// ── CameraMutation::FitToBounds matches imperative fit_to_bounds ────

#[test]
fn test_camera_mutation_fit_to_bounds_matches_imperative() {
    do_camera_mutation_fit_to_bounds_matches_imperative();
}

pub fn do_camera_mutation_fit_to_bounds_matches_imperative() {
    let mut imperative = Camera2D::new(800, 600);
    let mut via_mutation = Camera2D::new(800, 600);
    let min = Vec2::new(-500.0, -400.0);
    let max = Vec2::new(500.0, 400.0);
    imperative.fit_to_bounds(min, max, 0.05);
    via_mutation.apply_mutation(&CameraMutation::FitToBounds {
        min,
        max,
        padding_fraction: 0.05,
    });
    assert_eq!(imperative.position, via_mutation.position);
    assert_eq!(imperative.zoom, via_mutation.zoom);
}

// ── CameraMutation::SetPosition is direct assignment ────────────────

#[test]
fn test_camera_mutation_set_position_assigns_directly() {
    do_camera_mutation_set_position_assigns_directly();
}

/// `SetPosition` is a clean assignment, no clamping or transform —
/// used by fit-to-bounds and animation snapshot restore.
pub fn do_camera_mutation_set_position_assigns_directly() {
    let mut cam = Camera2D::new(800, 600);
    let target = Vec2::new(-150.0, 425.5);
    cam.apply_mutation(&CameraMutation::SetPosition { canvas_pos: target });
    assert_eq!(cam.position, target);
}

// ── pan / set_position / set_viewport_size leave neighbours alone ───

/// Pan changes only the position. The renderer's camera-pan perf fix
/// skips the canvas rebuild on this invariant — a future Pan variant
/// that also adjusted zoom would silently desync the scene.
#[test]
fn test_pan_leaves_zoom_and_viewport_size_untouched() {
    let mut cam = Camera2D::new(800, 600);
    cam.zoom = 2.5;
    let initial_zoom = cam.zoom;
    let initial_viewport = cam.viewport_size;
    cam.apply_mutation(&CameraMutation::Pan {
        screen_delta: Vec2::new(15.0, -22.5),
    });
    assert_eq!(cam.zoom, initial_zoom);
    assert_eq!(cam.viewport_size, initial_viewport);
}

/// `SetPosition` (used by `set_camera_center` for portal double-click
/// pan-to) must also leave zoom / viewport_size alone.
#[test]
fn test_set_position_leaves_zoom_and_viewport_size_untouched() {
    let mut cam = Camera2D::new(800, 600);
    cam.zoom = 0.75;
    let initial_zoom = cam.zoom;
    let initial_viewport = cam.viewport_size;
    cam.apply_mutation(&CameraMutation::SetPosition {
        canvas_pos: Vec2::new(-150.0, 425.5),
    });
    assert_eq!(cam.zoom, initial_zoom);
    assert_eq!(cam.viewport_size, initial_viewport);
}

/// `set_viewport_size` (used for window resize) is a pure viewport
/// change — the resize-doesn't-rebuild path depends on it leaving
/// zoom and position untouched.
#[test]
fn test_set_viewport_size_leaves_zoom_and_position_untouched() {
    let mut cam = Camera2D::new(800, 600);
    cam.zoom = 1.5;
    cam.position = Vec2::new(100.0, 200.0);
    let initial_zoom = cam.zoom;
    let initial_position = cam.position;
    cam.set_viewport_size(1920, 1080);
    assert_eq!(cam.zoom, initial_zoom);
    assert_eq!(cam.position, initial_position);
}

// ── visibility verdict ──────────────────────────────────────────────

#[test]
fn test_visibility_names_each_reason() {
    do_visibility_names_each_reason();
}

/// One element per verdict, at a camera that sees canvas
/// `[-400, 400] × [-300, 300]`: on screen, panned away, and outside
/// its zoom window. The zoom window wins over the pan when both fail,
/// because no pan brings that element back.
pub fn do_visibility_names_each_reason() {
    let cam = Camera2D::new(800, 600);
    let size = Vec2::new(50.0, 20.0);
    let unbounded = ZoomVisibility::unbounded();
    let zoomed_in_only = ZoomVisibility::from_pair(Some(2.0), None);

    assert_eq!(cam.visibility(Vec2::ZERO, size, unbounded), Visibility::Visible);
    assert_eq!(
        cam.visibility(Vec2::new(5000.0, 0.0), size, unbounded),
        Visibility::Offscreen
    );
    assert_eq!(
        cam.visibility(Vec2::ZERO, size, zoomed_in_only),
        Visibility::OutsideZoomWindow
    );
    assert_eq!(
        cam.visibility(Vec2::new(5000.0, 0.0), size, zoomed_in_only),
        Visibility::OutsideZoomWindow,
        "outside the window and off-screen reports the window"
    );
    assert!(Visibility::Visible.is_visible());
    assert!(!Visibility::Offscreen.is_visible());
    assert!(!Visibility::OutsideZoomWindow.is_visible());
}

#[test]
fn test_visibility_agrees_with_the_conjunction_it_replaced() {
    do_visibility_agrees_with_the_conjunction_it_replaced();
}

/// `visibility(..).is_visible()` is exactly `is_visible && contains`
/// — the expression the renderer's two `visible_at` culls used to
/// spell out — across a sweep of positions straddling every viewport
/// edge, at zooms below, inside and above a bounded window.
pub fn do_visibility_agrees_with_the_conjunction_it_replaced() {
    let windows = [
        ZoomVisibility::unbounded(),
        ZoomVisibility::from_pair(Some(0.5), Some(2.0)),
        ZoomVisibility::from_pair(None, Some(0.75)),
    ];
    let size = Vec2::new(40.0, 40.0);
    for zoom in [0.25_f32, 0.5, 1.0, 2.0, 3.0] {
        let mut cam = Camera2D::new(640, 480);
        cam.zoom = zoom;
        cam.position = Vec2::new(30.0, -10.0);
        for window in windows {
            for xi in -12..=12 {
                for yi in -12..=12 {
                    let pos = Vec2::new(xi as f32 * 70.0, yi as f32 * 55.0);
                    let expected = cam.is_visible(pos, size) && window.contains(cam.zoom);
                    assert_eq!(
                        cam.visibility(pos, size, window).is_visible(),
                        expected,
                        "zoom {zoom}, window {window:?}, pos {pos}"
                    );
                }
            }
        }
    }
}

/// The verdicts serialize as the snake-case words `format/ipc.md`
/// documents for the `scene` family.
#[test]
fn test_visibility_serializes_as_its_wire_words() {
    for (verdict, word) in [
        (Visibility::Visible, "visible"),
        (Visibility::OutsideZoomWindow, "outside_zoom_window"),
        (Visibility::Offscreen, "offscreen"),
    ] {
        assert_eq!(
            serde_json::to_value(verdict).expect("serializes"),
            serde_json::Value::String(word.into())
        );
    }
}

// ── pixel ratio ─────────────────────────────────────────────────────

#[test]
fn test_pixel_ratio_scales_pixels_not_zoom_windows() {
    do_pixel_ratio_scales_pixels_not_zoom_windows();
}

/// A camera at `pixel_ratio` 2 over a surface twice the size is the
/// same view at twice the pixels: every canvas point lands at twice its
/// ratio-1 position, a round trip still returns it, and every
/// visibility verdict is the ratio-1 one — including for an element
/// whose zoom window holds the camera's `zoom` but not its pixels per
/// unit, which is the case a supersampled capture would otherwise cull.
///
/// Fails when: a conversion reads `zoom` instead of
/// `pixels_per_unit` (the positions stop doubling), or the zoom-window
/// test reads `pixels_per_unit` (the `[1.5, 3]` window, which holds
/// zoom 2 and not 4, starts reporting `OutsideZoomWindow`).
pub fn do_pixel_ratio_scales_pixels_not_zoom_windows() {
    let mut view = Camera2D::new(800, 600);
    view.position = Vec2::new(120.0, -45.0);
    view.zoom = 2.0;
    let supersampled = Camera2D {
        viewport_size: (1600, 1200),
        pixel_ratio: 2.0,
        ..view
    };
    assert_eq!(supersampled.pixels_per_unit(), 4.0);

    let windows = [
        ZoomVisibility::unbounded(),
        ZoomVisibility::from_pair(Some(1.5), Some(3.0)),
        ZoomVisibility::from_pair(Some(3.0), None),
    ];
    let size = Vec2::new(30.0, 12.0);
    for xi in -10..=10 {
        for yi in -10..=10 {
            let pos = Vec2::new(xi as f32 * 25.0 + 120.0, yi as f32 * 20.0 - 45.0);
            assert!(
                almost_equal_vec2(
                    supersampled.canvas_to_screen(pos),
                    view.canvas_to_screen(pos) * 2.0
                ),
                "{pos} lands at twice its ratio-1 position"
            );
            assert!(almost_equal_vec2(
                supersampled.screen_to_canvas(supersampled.canvas_to_screen(pos)),
                pos
            ));
            for window in windows {
                assert_eq!(
                    supersampled.visibility(pos, size, window),
                    view.visibility(pos, size, window),
                    "{pos} under {window:?}"
                );
            }
        }
    }
    assert_eq!(
        supersampled.visibility(Vec2::new(120.0, -45.0), size, windows[1]),
        Visibility::Visible,
        "zoom 2 is inside [1.5, 3] whatever the pixel ratio"
    );
}

#[test]
fn test_fit_zoom_reads_the_view_not_the_surface() {
    do_fit_zoom_reads_the_view_not_the_surface();
}

/// `fit_zoom` is the zoom `fit_to_bounds` would clamp: unclamped, the
/// same for a supersampled camera as for the ratio-1 view it renders,
/// and `None` for a box with no area. A box that needs more zoom than
/// `MAX_ZOOM` reports it, which is how a caller tells that the clamp
/// still shows the whole box (zoomed out from the fit, so it fits) from
/// one that would cut it off (below `MIN_ZOOM`).
pub fn do_fit_zoom_reads_the_view_not_the_surface() {
    let view = Camera2D::new(800, 600);
    let supersampled = Camera2D {
        viewport_size: (2400, 1800),
        pixel_ratio: 3.0,
        ..view
    };
    let (min, max) = (Vec2::new(-100.0, -50.0), Vec2::new(300.0, 250.0));
    // Usable 720 × 540 against a 400 × 300 box: min(1.8, 1.8).
    let fit = view.fit_zoom(min, max, 0.05).expect("the box has area");
    assert!((fit - 1.8).abs() < 1e-5, "{fit}");
    assert_eq!(supersampled.fit_zoom(min, max, 0.05), Some(fit));

    let tiny = view
        .fit_zoom(Vec2::ZERO, Vec2::new(2.0, 1.0), 0.0)
        .expect("the box has area");
    assert!(tiny > Camera2D::MAX_ZOOM, "unclamped: {tiny}");
    let vast = view
        .fit_zoom(Vec2::ZERO, Vec2::new(1.0e6, 1.0), 0.0)
        .expect("the box has area");
    assert!(vast < Camera2D::MIN_ZOOM, "unclamped: {vast}");

    assert_eq!(view.fit_zoom(min, Vec2::new(min.x, max.y), 0.05), None);

    let mut fitted = supersampled;
    fitted.fit_to_bounds(min, max, 0.05);
    assert_eq!(fitted.zoom, fit, "fit_to_bounds applies exactly that zoom");
    assert!(almost_equal_vec2(fitted.position, (min + max) / 2.0));
}
