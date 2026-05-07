// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::KeyboardInput` arm (Pressed only). Routes
//! through the editor-steal pre-filter (`TextEditCommit` /
//! `TextEditCancel` first, then the modal `handle_text_edit_key`
//! body), then the document-context Action lookup, then the
//! Macro fallback. Mirrors the Action -> Macro chain native
//! exposes via `event_keyboard::handle_keyboard_input`; the WASM
//! side has no console / color-picker / label / portal edit
//! modals so the pre-filter ladder is simpler.

#![cfg(target_arch = "wasm32")]

use crate::application::platform::input::Key;

use super::WasmMacroDispatchTarget;
use crate::application::app::dispatch;
use crate::application::app::text_edit::handle_text_edit_key;
use crate::application::keybinds::Action;

impl super::WasmApp {
    pub(super) fn handle_keyboard_input(&mut self, logical_key: Key) {
        let key_name = crate::application::keybinds::key_to_name(&logical_key);

        let mut input_borrow = self.input.borrow_mut();
        let mut renderer_borrow = self.renderer.borrow_mut();
        let (Some(input), Some(renderer)) = (input_borrow.as_mut(), renderer_borrow.as_mut()) else {
            return;
        };

        // Editor keyboard-steal: if open, route all keys
        // to the editor so hotkeys don't collide with typed text.
        //
        // Commit/cancel pre-filter: dispatch through the funnel
        // (`Action::TextEditCommit` / `TextEditCancel`) BEFORE
        // calling the modal handler. Mirrors the native shape
        // at `event_keyboard.rs`.
        if input.text_edit_state.is_open() {
            let action = key_name.as_deref().and_then(|n| {
                self.keybinds.action_for_context(
                    crate::application::keybinds::InputContext::TextEdit,
                    n,
                    input.modifiers.control_key(),
                    input.modifiers.shift_key(),
                    input.modifiers.alt_key(),
                )
            });
            if let Some(modal_action @ (Action::TextEditCommit | Action::TextEditCancel)) = &action {
                let mut core = input.input_context_core(renderer, &self.keybinds);
                let _ = dispatch::action_core::dispatch_compatible(modal_action, &mut core);
                self.suppress_keys.set(input.text_edit_state.is_open());
                return;
            }
            handle_text_edit_key(
                &key_name,
                &logical_key,
                input.modifiers.control_key(),
                input.modifiers.shift_key(),
                input.modifiers.alt_key(),
                &self.keybinds,
                &mut input.text_edit_state,
                &mut input.document,
                &mut input.mindmap_tree,
                &mut input.app_scene,
                renderer,
                &mut input.scene_cache,
            );
            self.suppress_keys.set(input.text_edit_state.is_open());
            return;
        }

        // Hotkey dispatch via keybinds.
        let action = key_name.as_deref().and_then(|k| {
            self.keybinds.action_for_context(
                crate::application::keybinds::InputContext::Document,
                k,
                input.modifiers.control_key(),
                input.modifiers.shift_key(),
                input.modifiers.alt_key(),
            )
        });
        // WASM dispatch ladder. Action body lives in the
        // unified `dispatch_action_core::dispatch_compatible`
        // (Track C) — both the keyboard path here and the
        // `WasmMacroDispatchTarget::dispatch_action` impl
        // reach the same body. Native's `dispatch::dispatch_action`
        // delegates to it as well; one source of truth for
        // every Compatible arm across both targets.
        //
        // After the action lookup completes, fall through to
        // macro lookup — the same Action → Macro →
        // CustomMutation chain native uses (`event_keyboard.rs`).
        if let Some(a) = action.clone() {
            // Pin "did the user just trigger an EditSelection?"
            // before the dispatch — `self.suppress_keys` is
            // ONLY updated for that pair (pre-Track-B, the
            // suppress call lived inside the EditSelection
            // pre-filter arm). Other Compatible Actions don't
            // touch suppress.
            let was_edit_selection = matches!(a, Action::EditSelection | Action::EditSelectionClean);
            let _ = {
                let mut core = input.input_context_core(renderer, &self.keybinds);
                dispatch::action_core::dispatch_compatible(&a, &mut core)
            };
            if was_edit_selection {
                // Mirror pre-Track-B `set(is_open())`: flip
                // suppress to whatever the modal state ended
                // up at — true if the editor opened, false if
                // it didn't (e.g. selection wasn't Single).
                // Always-set, NOT gated on dispatch outcome.
                // Track-C-Commit-3 incorrectly gated on
                // `Handled` which left suppress stuck-true on a
                // non-Single EditSelection (Unhandled outcome);
                // restored here per the design reviewer's flag.
                self.suppress_keys.set(input.text_edit_state.is_open());
            }
        } else {
            // No built-in Action bound to this combo — fall
            // through to macro lookup. Mirrors native's
            // `event_keyboard.rs` chain: Action → Macro →
            // (CustomMutation tier on native; macros only on
            // WASM today). Privilege gate runs inside
            // `dispatch_macro_core::dispatch_macro` so a
            // hostile Map / Inline tier macro can't slip
            // destructive Actions or ConsoleLine past.
            if let Some(macro_id) = key_name.as_deref().and_then(|k| {
                self.keybinds.macro_for(
                    k,
                    input.modifiers.control_key(),
                    input.modifiers.shift_key(),
                    input.modifiers.alt_key(),
                )
            }) {
                let macro_id = macro_id.to_string();
                let mut target = WasmMacroDispatchTarget {
                    input,
                    renderer,
                    keybinds: &self.keybinds,
                };
                let _ = dispatch::macro_core::dispatch_macro(&macro_id, &mut target);
            }
        }
    }
}
