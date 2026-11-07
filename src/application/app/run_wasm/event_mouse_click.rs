// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::MouseInput` arm. The left button splits into
//! Pressed and Released paths via [`super::Borrowed::on_mouse_input`];
//! the right button reaches [`super::Borrowed::handle_right_button`],
//! and any other button is dropped, as native drops it:
//!
//! - **Pressed** ([`super::Borrowed::handle_mouse_pressed`]): runs the
//!   cross-platform `compute_click_hit` — the projector's hit test, the
//!   chain native's press runs — distinguishes single vs double click,
//!   dispatches the `DoubleClick` binding on a double, and otherwise
//!   holds what the press hit (`WasmInputState::pending_click`) plus a
//!   `LastClick` snapshot for the eventual release.
//! - **Released** ([`super::Borrowed::handle_mouse_released`]): clicks
//!   what the press hit — the selection updates through the shared
//!   `click_selection` and the view syncs, native's `handle_click`.
//!
//! Mirrors the native shape at `event_mouse_click.rs`; the WASM
//! variant has no console / color-picker / drag-state intercepts so
//! the early-return ladder is shorter.

#![cfg(target_arch = "wasm32")]

use crate::application::platform::input::{ElementState, MouseButton};

use crate::application::app::dispatch::cross_dispatch::click_selection;
use crate::application::app::dispatch::families::verbs;
use crate::application::app::dispatch::DispatchOutcome;
use crate::application::app::{compute_click_hit, dispatch, is_double_click, ClickHit, LastClick};
use std::sync::atomic::AtomicBool;

/// One-shot latch for the double-click input class. See
/// [`crate::application::app::warn_unhandled_native_only_once`]; per-class so a dead
/// double-click binding and a dead wheel binding each get their own
/// line in the console.
static WARNED_NATIVE_ONLY_DOUBLE_CLICK: AtomicBool = AtomicBool::new(false);

impl super::Borrowed<'_> {
    /// `WindowEvent::MouseInput`, by button. Returns whether to
    /// request a redraw: a left or right button always asks for one,
    /// as native's `on_mouse_input` does; a button the browser routes
    /// nowhere asks for nothing.
    pub(super) fn on_mouse_input(&mut self, state: ElementState, button: MouseButton) -> bool {
        match button {
            MouseButton::Left => {
                if state == ElementState::Pressed {
                    self.handle_mouse_pressed();
                } else {
                    self.handle_mouse_released();
                }
                true
            }
            MouseButton::Right => {
                self.handle_right_button(state);
                true
            }
            _ => false,
        }
    }

    /// WASM right-button handler.
    ///
    /// `DragState::PendingRight` and the right-drag gesture pipeline are
    /// `cfg(not(target_arch = "wasm32"))`: the browser has no drag-state
    /// machine. What this shim *does* do is keep the right-button event
    /// from being silently dropped, and surface a one-time warn so a user
    /// who binds a right-button gesture sees evidence in the console
    /// rather than wondering why the binding is dead. Single-press
    /// `RightClick` actions stay unreachable on WASM until the dispatch
    /// grows a path for non-Action `MouseGesture` lookups.
    pub(super) fn handle_right_button(&mut self, state: ElementState) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WARNED: AtomicBool = AtomicBool::new(false);
        if state == ElementState::Released && !WARNED.swap(true, Ordering::Relaxed) {
            log::warn!(
                "wasm: right-button gestures are a no-op until the browser gains a drag-state \
                 machine; browser context menu suppressed via canvas oncontextmenu, \
                 Shift+RightClick bypasses the suppression"
            );
        }
    }

    fn handle_mouse_pressed(&mut self) {
        let input = &mut *self.input;
        let renderer = &mut *self.renderer;
        let canvas_pos = renderer.screen_to_canvas(input.cursor_pos.0 as f32, input.cursor_pos.1 as f32);
        // The one hit chain both targets' presses run.
        let hit = compute_click_hit(Some(&input.document), input.view.as_mut(), canvas_pos);
        let now = input.clock.now_ms();
        let is_dblclick = input
            .last_click
            .as_ref()
            .is_some_and(|prev| is_double_click(prev, now, input.cursor_pos, &hit));

        if is_dblclick {
            input.last_click = None;
            input.pending_click = None;
            // The binding native's press looks up, with the same
            // modifier fallback, so rebinding `double_click_activate`
            // takes effect in the browser too.
            let dblclick_name = crate::application::keybinds::MouseGesture::DoubleClick.key_name();
            let action = self.keybinds.action_for_gesture(
                dblclick_name,
                input.modifiers.control_key(),
                input.modifiers.shift_key(),
                input.modifiers.alt_key(),
            );
            if let Some(a) = action {
                let dispatch_hit = dispatch::DispatchHit {
                    click_hit: hit,
                    canvas_pos,
                };
                let outcome = {
                    let mut core = input.input_context_core(renderer);
                    dispatch::action_core::dispatch_compatible(&a, &mut core, Some(&dispatch_hit))
                };
                if matches!(outcome, DispatchOutcome::Unhandled) {
                    crate::application::app::warn_unhandled_native_only_once(
                        &WARNED_NATIVE_ONLY_DOUBLE_CLICK,
                        dblclick_name,
                        &a,
                        "Rebind double_click_activate to a Compatible action, or wait \
                         for the browser port of the native-only body \
                         (work_plans/WASM_CONVERGENCE.md).",
                    );
                }
            }
            // No Action bound to DoubleClick: silently no-op. Either
            // way the double-click consumed `last_click`; it does not
            // fall through to the single-click path.
            return;
        }

        input.pending_click = Some(hit);
        input.last_click = Some(LastClick {
            time: now,
            screen_pos: input.cursor_pos,
            hit,
        });
    }

    /// A release clicks what its press hit: the selection updates as
    /// native's `handle_click` updates it — Shift held or not — and the
    /// view syncs when it changed. A click on a node that names an
    /// `on.click` verb notes it for V1-D3 (#252), which runs it.
    fn handle_mouse_released(&mut self) {
        let Some(hit) = self.input.pending_click.take() else {
            return;
        };
        let shift = self.input.modifiers.shift_key();
        let doc = &mut self.input.document;
        if let ClickHit::Element(element) = hit {
            verbs::node_clicked(doc.doc(), element);
        }
        if click_selection(&mut doc.selection, hit, shift) {
            self.sync_view();
        }
    }
}
