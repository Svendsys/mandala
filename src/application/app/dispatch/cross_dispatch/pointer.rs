// SPDX-License-Identifier: MPL-2.0

//! Pointer-gesture helpers: the payload a pointer dispatch carries
//! ([`DispatchHit`]), and the touch route — the recognizer's output
//! resolved to a dispatch or to a camera or selection effect.
//!
//! [`DispatchHit`] lives here rather than in the native dispatcher
//! because both targets populate one: the payload is what a
//! keyboard-driven dispatch cannot carry (what the press landed on, and
//! where it landed in canvas space), and it is made of plain values that
//! exist on both targets.

use glam::Vec2;

use crate::application::common::RenderDecree;
use crate::application::keybinds::{Action, MouseGesture, ResolvedKeybinds};

use crate::application::app::dispatch::families::verbs;
use crate::application::app::input_context_core::InputContextCore;
use crate::application::app::touch_gesture::{Phase, RecognizedGesture, TouchGestureRecognizer};
use crate::application::app::{compute_click_hit, ClickHit};

use super::click_selection;

/// What a pointer press landed on, carried into a dispatch.
#[derive(Debug, Clone, Copy)]
pub struct DispatchHit {
    /// What the press hit (`compute_click_hit`).
    pub click_hit: ClickHit,
    /// Where it landed, in canvas space.
    pub canvas_pos: Vec2,
    /// The mouse button holding the press down, when one does: the
    /// release that ends a pan armed by this dispatch (#153). Native
    /// only: the browser has no drag state to hand a pan to.
    #[cfg(not(target_arch = "wasm32"))]
    pub held_by: Option<crate::application::platform::input::MouseButton>,
}

/// winit's touch phase as the recognizer reads it.
pub(in crate::application::app) fn touch_phase(phase: winit::event::TouchPhase) -> Phase {
    match phase {
        winit::event::TouchPhase::Started => Phase::Started,
        winit::event::TouchPhase::Moved => Phase::Moved,
        winit::event::TouchPhase::Ended => Phase::Ended,
        winit::event::TouchPhase::Cancelled => Phase::Cancelled,
    }
}

/// A touch gesture that resolved through the keybind table: the one the
/// recognizer emits as a `MouseGesture` (the long-press), with the Action
/// it is bound to, if any.
#[derive(Debug)]
pub(in crate::application::app) struct TouchGestureDispatch {
    /// Where the gesture was, on the surface.
    pub(in crate::application::app) cursor_pos: (f64, f64),
    /// The gesture's name, as the browser's one-shot warning names it.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(in crate::application::app) gesture_name: &'static str,
    /// The Action it is bound to, if any.
    pub(in crate::application::app) action: Option<Action>,
}

/// A touch gesture that reaches no Action: it is applied directly, on
/// both targets ([`apply_touch_effect`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::application::app) enum TouchEffect {
    /// A tap: select what is under it, as a click does.
    TapSelect { screen_pos: (f64, f64) },
    /// A one-finger pan or a two-finger pinch step: move the camera.
    CameraStep {
        anchor: (f64, f64),
        pan: (f64, f64),
        scale: f64,
    },
}

/// What one touch sample produced.
#[derive(Debug)]
pub(in crate::application::app) enum TouchStep {
    /// A gesture to dispatch through the funnel.
    Dispatch(TouchGestureDispatch),
    /// An effect to apply directly.
    Effect(TouchEffect),
}

/// Feed one touch sample to `recognizer` and resolve what it emits.
pub(in crate::application::app) fn drive_touch_event(
    recognizer: &mut TouchGestureRecognizer,
    keybinds: &ResolvedKeybinds,
    phase: Phase,
    id: u64,
    pos: (f64, f64),
    now: web_time::Instant,
) -> Option<TouchStep> {
    let from_ingest = recognizer.ingest(phase, id, pos, now);
    let from_tick = recognizer.tick(now);
    Some(match from_ingest.or(from_tick)? {
        RecognizedGesture::LongPress { pos } => {
            let gesture_name = MouseGesture::LongPress.key_name();
            TouchStep::Dispatch(TouchGestureDispatch {
                cursor_pos: pos,
                gesture_name,
                action: keybinds.action_for_gesture(gesture_name, false, false, false),
            })
        }
        RecognizedGesture::Tap { pos } => TouchStep::Effect(TouchEffect::TapSelect { screen_pos: pos }),
        RecognizedGesture::Pan { pos, delta } => TouchStep::Effect(TouchEffect::CameraStep {
            anchor: pos,
            pan: delta,
            scale: 1.0,
        }),
        RecognizedGesture::PinchStep { center, pan, scale } => TouchStep::Effect(TouchEffect::CameraStep {
            anchor: center,
            pan,
            scale,
        }),
    })
}

/// Apply a touch effect: a tap selects, a pan or pinch moves the camera.
pub(in crate::application::app) fn apply_touch_effect(effect: TouchEffect, core: &mut InputContextCore<'_>) {
    match effect {
        TouchEffect::TapSelect { screen_pos } => {
            *core.cursor_pos = screen_pos;
            apply_tap_select(screen_pos, core);
        }
        TouchEffect::CameraStep { anchor, pan, scale } => {
            *core.cursor_pos = anchor;
            let (translate, zoom) = camera_step_decrees(anchor, pan, scale);
            core.renderer.process_decree(translate);
            if let Some(zoom) = zoom {
                core.renderer.process_decree(zoom);
            }
        }
    }
}

/// The decrees a camera step makes: the pan, then the zoom about the
/// anchor when the step scales.
fn camera_step_decrees(anchor: (f64, f64), pan: (f64, f64), scale: f64) -> (RenderDecree, Option<RenderDecree>) {
    let translate = RenderDecree::CameraPan(pan.0 as f32, pan.1 as f32);
    let zoom = (scale != 1.0).then_some(RenderDecree::CameraZoom {
        screen_x: anchor.0 as f32,
        screen_y: anchor.1 as f32,
        factor: scale as f32,
    });
    (translate, zoom)
}

/// A tap selects as a click without Shift does, through the same hit
/// chain (`compute_click_hit`), and syncs the view when the selection
/// changed.
fn apply_tap_select(screen_pos: (f64, f64), core: &mut InputContextCore<'_>) {
    let canvas_pos = core
        .renderer
        .screen_to_canvas(screen_pos.0 as f32, screen_pos.1 as f32);
    let Some(doc) = core.document.as_deref_mut() else {
        return;
    };
    let hit = compute_click_hit(Some(&*doc), core.view.as_mut(), canvas_pos);
    if let ClickHit::Element(element) = hit {
        verbs::node_clicked(doc.doc(), element);
    }
    if click_selection(&mut doc.selection, hit, false) {
        if let Some(mut vc) = super::view_ctx!(core, doc) {
            vc.sync();
        }
    }
}
