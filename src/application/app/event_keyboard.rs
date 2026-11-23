// SPDX-License-Identifier: MPL-2.0

//! Keyboard-event dispatch. Routes `KeyboardInput` (Pressed only)
//! through the modal-steal ladder — the console, the color picker, the
//! single-line editor, the section text editor, as on `main` — then the
//! action table (NodeEdit's context first while it is active), then
//! type-to-edit on a selected path, then the macro bindings, then the
//! edit bindings.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::platform::input::Key;

use super::color_picker_flow::{handle_color_picker_clipboard_key, picker_op_for};
use super::console_input::handle_console_key;
use super::dispatch::families::{edits, text};
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
                if super::dispatch::dispatch_action(a, ctx, None) == super::dispatch::DispatchOutcome::Handled
                {
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

    // The text editors steal every key while open, the single-line one
    // first, as on `main`. A key bound in the editor's context runs its
    // Action through the funnel — commit, cancel, every cursor and delete
    // primitive — so a macro, the console and IPC reach the same bodies;
    // a key bound to anything else is swallowed, since the editor owns the
    // keyboard; any other key is a literal (`families::text`).
    if ctx.single_line_edit.is_open() {
        match crate::application::app::single_line_edit::bound_action(
            key_name.as_deref(),
            ctx.modifiers,
            ctx.keybinds,
        ) {
            Some(a) => {
                let _ = super::dispatch::dispatch_action(a, ctx, None);
            }
            None => {
                text::type_line(ctx, &logical_key);
            }
        }
        return;
    }
    if ctx.text_edit.is_open() {
        match text::text_key(key_name.as_deref(), ctx.modifiers, ctx.keybinds) {
            Some(a) => {
                let _ = super::dispatch::dispatch_action(a, ctx, None);
            }
            None => {
                let (mut core, _) = ctx.split_borrow();
                text::type_literal(&mut core, key_name.as_deref(), &logical_key);
            }
        }
        return;
    }

    let Some(k) = key_name.as_deref() else {
        return;
    };
    let (ctrl, shift, alt) = (
        ctx.modifiers.control_key(),
        ctx.modifiers.shift_key(),
        ctx.modifiers.alt_key(),
    );
    // In NodeEdit its own context is asked first, and falls through to
    // the document's (`InputContext::NodeEdit::falls_through`).
    let context = match ctx.interaction_mode.node_edit() {
        Some(_) => crate::application::keybinds::InputContext::NodeEdit,
        None => crate::application::keybinds::InputContext::Document,
    };
    let action = ctx.keybinds.action_for_context(context, k, ctrl, shift, alt);
    if let Some(a) = action {
        let _ = super::dispatch::dispatch_held_action(a, ctx, None, HeldInput::Key(k.to_string()));
        return;
    }
    // Type-to-edit on a selected path's label, only for a key no Action
    // claims, as on `main`.
    if text::type_to_edit(ctx, &logical_key) {
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
