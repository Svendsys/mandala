// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::KeyboardInput` arm (Pressed only). Routes through
//! the document-context Action lookup, then the macro fallback, then
//! the edit-binding tier: the same Action → Macro → edit chain native
//! runs in `event_keyboard::handle_keyboard_input`. The browser has no
//! console or color picker to steal keys first, so the ladder starts at
//! the keybind table.

#![cfg(target_arch = "wasm32")]

use crate::application::platform::input::{ElementState, Key};

use super::WasmMacroDispatchTarget;
use crate::application::app::dispatch;
use crate::application::app::dispatch::families::edits;

impl super::Borrowed<'_> {
    /// `WindowEvent::KeyboardInput`. Returns whether to request a
    /// redraw: a press always asks for one, as native's does; a release
    /// does nothing here (the browser has no key-held pan to end).
    pub(super) fn on_keyboard_input(&mut self, logical_key: Key, state: ElementState) -> bool {
        if state != ElementState::Pressed {
            return false;
        }
        let Some(k) = crate::application::keybinds::key_to_name(&logical_key) else {
            return true;
        };
        let input = &mut *self.input;
        let renderer = &mut *self.renderer;
        let keybinds = self.keybinds;
        let (ctrl, shift, alt) = (
            input.modifiers.control_key(),
            input.modifiers.shift_key(),
            input.modifiers.alt_key(),
        );
        let action = keybinds.action_for_context(
            crate::application::keybinds::InputContext::Document,
            &k,
            ctrl,
            shift,
            alt,
        );
        // The body lives in the unified `action_core::dispatch_compatible`,
        // which native's `dispatch::dispatch_action` delegates to as well:
        // one source of truth for every Compatible arm across both targets.
        if let Some(a) = action {
            let mut core = input.input_context_core(renderer);
            let _ = dispatch::action_core::dispatch_compatible(&a, &mut core, None);
            return true;
        }
        // No built-in Action bound to this combo — fall through the rest
        // of native's chain: Action → Macro → edit (CONCEPTS §5).
        // Privilege gating runs inside `dispatch::macro_core::dispatch_macro`
        // so a hostile tier's macro can't slip destructive Actions or
        // ConsoleLine past. A macro bound to an id that isn't in the
        // registry (typo'd config, half-loaded macros file) returns false
        // and falls through to the edit tier, exactly as native does.
        if let Some(id) = keybinds.macro_for(&k, ctrl, shift, alt).map(str::to_string) {
            let mut target = WasmMacroDispatchTarget {
                input,
                renderer,
                after_step: None,
            };
            if dispatch::macro_core::dispatch_macro(&id, &mut target) {
                return true;
            }
        }
        if let Some(edit) = keybinds.edit_for(&k, ctrl, shift, alt) {
            let now_ms = input.clock.now_ms();
            let (document, registry) = (&mut input.document, &input.edits);
            edits::with_drawn(input.view.as_mut(), renderer, |drawn| {
                edits::run_key(document, drawn, registry, edit, now_ms)
            });
        }
        true
    }
}
