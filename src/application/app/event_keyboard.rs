// SPDX-License-Identifier: MPL-2.0

//! Keyboard-event dispatch. Routes `KeyboardInput` (Pressed only)
//! through the console's steal, then the color picker's, then the
//! action table, then the macro bindings, then the edit bindings.
//!
//! The map's own editors (V1-C8, #249) return with the issue that
//! rebuilds them on the one writer, and take their place in this ladder
//! then.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::platform::input::Key;

use super::color_picker_flow::{handle_color_picker_clipboard_key, picker_op_for};
use super::console_input::handle_console_key;
use super::dispatch::families::edits;
use super::input_context::InputHandlerContext;
use super::HeldInput;

/// A key release ends a pan the key holds (#153), and does nothing else.
pub(super) fn handle_key_release(logical_key: &Key, ctx: &mut InputHandlerContext<'_>) {
    if let Some(name) = crate::application::keybinds::key_to_name(logical_key) {
        ctx.drag_state.end_pan_held_by(&HeldInput::Key(name));
    }
}

/// A key press: the console steals every key while it is open; otherwise
/// the Action bound to it runs, held by the key, or else the macro bound
/// to it, or else the edit bound to it.
pub(super) fn handle_keyboard_input(logical_key: Key, ctx: &mut InputHandlerContext<'_>) {
    let key_name = crate::application::keybinds::key_to_name(&logical_key);

    // When the console is open, it steals all keyboard input. Character
    // keys insert at the cursor, Tab triggers completion, Up/Down walks
    // history, Enter parses + executes, Escape closes. Regular hotkeys
    // are suppressed until the console closes.
    if ctx.console_state.is_open() {
        handle_console_key(&key_name, &logical_key, ctx);
        return;
    }

    // The color picker steals the keys it claims (Esc, Enter, the
    // nudges, and its clipboard's Copy / Paste / Cut); any other key —
    // the console's `/` among them — falls through, so the standalone
    // palette never locks the keyboard out. Commit, cancel and the
    // nudges go through the funnel (`picker_op_for`), as on `main`;
    // `Unhandled` comes back where the picker declines — a cancel of the
    // standalone palette — and the key falls through to the document.
    if ctx.color_picker_state.is_open() {
        let picker_action = key_name.as_deref().and_then(|n| {
            ctx.keybinds.action_for_context(
                crate::application::keybinds::InputContext::ColorPicker,
                n,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
            )
        });
        match picker_action {
            Some(a) if picker_op_for(&a).is_some() => {
                if super::dispatch::dispatch_action(a, ctx, None) == super::dispatch::DispatchOutcome::Handled {
                    return;
                }
            }
            other => {
                if handle_color_picker_clipboard_key(other.as_ref(), ctx.color_picker_state) {
                    return;
                }
            }
        }
    }

    let Some(k) = key_name.as_deref() else {
        return;
    };
    let (ctrl, shift, alt) = (
        ctx.modifiers.control_key(),
        ctx.modifiers.shift_key(),
        ctx.modifiers.alt_key(),
    );
    let action = ctx.keybinds.action_for_context(
        crate::application::keybinds::InputContext::Document,
        k,
        ctrl,
        shift,
        alt,
    );
    if let Some(a) = action {
        let _ = super::dispatch::dispatch_held_action(a, ctx, None, HeldInput::Key(k.to_string()));
        return;
    }
    let macro_id = ctx.keybinds.macro_for(k, ctrl, shift, alt).map(str::to_string);
    if let Some(id) = macro_id {
        if super::dispatch::dispatch_macro(&id, ctx, None) {
            return;
        }
    }
    if let Some(edit) = ctx.keybinds.edit_for(k, ctrl, shift, alt) {
        if let Some(doc) = ctx.document.as_mut() {
            let (registry, now_ms) = (ctx.edits, ctx.clock.now_ms());
            edits::with_drawn(ctx.view.as_mut(), ctx.renderer, |drawn| {
                edits::run_key(doc, drawn, registry, edit, now_ms)
            });
        }
    }
}
