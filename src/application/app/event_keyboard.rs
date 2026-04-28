// SPDX-License-Identifier: MPL-2.0

//! Keyboard-event dispatch. Routes `KeyboardInput` (Pressed only)
//! through the modal-steal ladder (console, color picker, label /
//! portal / node text editors), then the action table, then the
//! custom-mutation key bindings.

#![cfg(not(target_arch = "wasm32"))]

use super::input_context::InputHandlerContext;
use super::*;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::Key;

pub(super) fn handle_keyboard_input(
    logical_key: Key,
    _event_loop: &ActiveEventLoop,
    ctx: &mut InputHandlerContext<'_>,
) {
    let key_name = crate::application::keybinds::key_to_name(&logical_key);

    // When the console is open, it steals all
    // keyboard input. Character keys insert at the
    // cursor, Tab triggers completion, Up/Down walks
    // history, Enter parses + executes, Escape
    // closes. Regular hotkeys are suppressed until
    // the console closes.
    if ctx.console_state.is_open() {
        handle_console_key(
            &key_name,
            &logical_key,
            ctx.modifiers.control_key(),
            ctx.modifiers.shift_key(),
            ctx.modifiers.alt_key(),
            ctx.console_state,
            ctx.console_history,
            ctx.label_edit_state,
            ctx.portal_text_edit_state,
            ctx.color_picker_state,
            ctx.document,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
            ctx.keybinds,
            ctx.macros,
        );
        return;
    }

    // Glyph-wheel color picker key handling.
    // Mutually exclusive with console and label-edit
    // for the keys it claims (Esc, Enter, h/s/v/
    // H/S/V). Any other key — notably the console
    // trigger `/` — falls through so the Standalone
    // persistent palette doesn't deadlock the user
    // out of the normal keybind dispatch.
    if ctx.color_picker_state.is_open() {
        let consumed = if let Some(doc) = ctx.document.as_mut() {
            handle_color_picker_key(
                &key_name,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
                ctx.keybinds,
                ctx.color_picker_state,
                doc,
                ctx.mindmap_tree,
                ctx.picker_hover,
                ctx.app_scene,
                ctx.renderer,
                ctx.scene_cache,
            )
        } else {
            false
        };
        if consumed {
            return;
        }
    }

    // Inline label edit modal. Steals keys the same way
    // the console does. Escape discards, Enter commits,
    // Backspace pops, character keys append.
    if ctx.label_edit_state.is_open() {
        if let Some(doc) = ctx.document.as_mut() {
            handle_label_edit_key(
                &key_name,
                &logical_key,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
                ctx.keybinds,
                ctx.label_edit_state,
                doc,
                ctx.mindmap_tree,
                ctx.app_scene,
                ctx.renderer,
                ctx.scene_cache,
            );
        }
        return;
    }

    // Inline portal-text edit modal — parallel to the
    // edge label editor but keyed to
    // `(edge_ref, endpoint_node_id)`. Same keystroke
    // routing via `InputContext::LabelEdit`.
    if ctx.portal_text_edit_state.is_open() {
        if let Some(doc) = ctx.document.as_mut() {
            handle_portal_text_edit_key(
                &key_name,
                &logical_key,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
                ctx.keybinds,
                ctx.portal_text_edit_state,
                doc,
                ctx.mindmap_tree,
                ctx.app_scene,
                ctx.renderer,
                ctx.scene_cache,
            );
        }
        return;
    }

    // Inline node text editor. Steals keys the same way
    // the console / label-edit modals do. Enter and Tab
    // are literal characters inside the editor — this is
    // a multi-line paragraph editor, not an outliner.
    // Esc cancels; commit is via click-outside in the
    // mouse handler.
    if ctx.text_edit_state.is_open() {
        if let Some(doc) = ctx.document.as_mut() {
            handle_text_edit_key(
                &key_name,
                &logical_key,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
                ctx.keybinds,
                ctx.text_edit_state,
                doc,
                ctx.mindmap_tree,
                ctx.app_scene,
                ctx.renderer,
                ctx.scene_cache,
            );
        }
        return;
    }

    let action = key_name.as_deref().and_then(|k| {
        ctx.keybinds.action_for_context(
            crate::application::keybinds::InputContext::Document,
            k,
            ctx.modifiers.control_key(),
            ctx.modifiers.shift_key(),
            ctx.modifiers.alt_key(),
        )
    });

    // Type-to-edit on edge / portal label selections: when an
    // editable selection is active, no editor is open, no action
    // claims the key (so custom mutations / keybind rebinds always
    // win), and the user types a printable character (no Ctrl /
    // Alt — Shift is OK so capital letters and shifted symbols
    // still type), open the right inline editor and replay the
    // keystroke through it so the typed character lands in the
    // buffer as the first edit. This makes the gesture symmetric
    // with what the node editor offers via `EditSelectionClean` /
    // typing on a freshly-selected node. The action-first check
    // means rebinding `'a'` to a Document action keeps that
    // binding alive even when an edge label is selected.
    if action.is_none()
        && !ctx.modifiers.control_key()
        && !ctx.modifiers.alt_key()
    {
        if let Key::Character(ref c) = logical_key {
            // Reject empty payloads and pure-control payloads up
            // front so single-char shortcuts that the keybind table
            // hasn't claimed don't accidentally open an editor.
            let has_printable = c.as_str().chars().any(|ch| !ch.is_control());
            if has_printable {
                if let Some(doc) = ctx.document.as_mut() {
                    let opened = match doc.selection.clone() {
                        SelectionState::EdgeLabel(s) => {
                            open_label_edit(
                                &s.edge_ref,
                                doc,
                                ctx.label_edit_state,
                                ctx.app_scene,
                                ctx.renderer,
                            );
                            ctx.label_edit_state.is_open()
                        }
                        SelectionState::PortalLabel(s)
                        | SelectionState::PortalText(s) => {
                            let er = s.edge_ref();
                            open_portal_text_edit(
                                &er,
                                &s.endpoint_node_id,
                                doc,
                                ctx.portal_text_edit_state,
                                ctx.app_scene,
                                ctx.renderer,
                            );
                            ctx.portal_text_edit_state.is_open()
                        }
                        _ => false,
                    };
                    if opened {
                        // Replay the typed character through the
                        // newly-opened editor so the first key
                        // ends up in the buffer instead of being
                        // swallowed by the open gesture.
                        if ctx.label_edit_state.is_open() {
                            handle_label_edit_key(
                                &key_name,
                                &logical_key,
                                ctx.modifiers.control_key(),
                                ctx.modifiers.shift_key(),
                                ctx.modifiers.alt_key(),
                                ctx.keybinds,
                                ctx.label_edit_state,
                                doc,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                        } else if ctx.portal_text_edit_state.is_open() {
                            handle_portal_text_edit_key(
                                &key_name,
                                &logical_key,
                                ctx.modifiers.control_key(),
                                ctx.modifiers.shift_key(),
                                ctx.modifiers.alt_key(),
                                ctx.keybinds,
                                ctx.portal_text_edit_state,
                                doc,
                                ctx.mindmap_tree,
                                ctx.app_scene,
                                ctx.renderer,
                                ctx.scene_cache,
                            );
                        }
                        return;
                    }
                    // `open_*` silently returned without opening an
                    // editor — the selection's target evaporated
                    // (edge deleted by a background undo, portal
                    // edge flipped to line mode, etc). Log and drop
                    // the keystroke rather than falling through to
                    // action dispatch with a stale selection — the
                    // user's mental model was "I'm about to type
                    // into this selected thing", not "trigger a
                    // Document action".
                    log::warn!(
                        "type-to-edit: selected edge / portal endpoint \
                         vanished before editor could open; keystroke dropped"
                    );
                    return;
                }
            }
        }
    }

    if let Some(a) = action {
        // Action body lives in `super::dispatch::dispatch_action`.
        let _ = super::dispatch::dispatch_action(a, ctx, None);
    } else {
        // No built-in action matched — try ctx.macros first, then custom
        // mutations. Resolution order is documented in CONCEPTS.md §5
        // "Action dispatch": Action -> Macro -> CustomMutation.
        if let Some(k) = key_name.as_deref() {
            let macro_id = ctx.keybinds
                .macro_for(
                    k,
                    ctx.modifiers.control_key(),
                    ctx.modifiers.shift_key(),
                    ctx.modifiers.alt_key(),
                )
                .map(|s| s.to_string());
            // If a macro is bound but its id isn't in the registry
            // (typo'd config, half-loaded ctx.macros file, etc.),
            // `dispatch_macro` returns false. Fall through to the
            // custom-mutation tier so the keystroke still has a
            // chance to do something — better UX than swallowing
            // silently.
            let macro_handled = if let Some(id) = macro_id {
                super::dispatch::dispatch_macro(&id, ctx)
            } else {
                false
            };
            if !macro_handled {
                // Custom mutation fall-through (Phase-7 parity:
                // animation-timing aware, always invokes
                // `apply_document_actions`).
                let _ = super::dispatch::dispatch_custom_mutation_for_key(
                    ctx,
                    k,
                    ctx.modifiers.control_key(),
                    ctx.modifiers.shift_key(),
                    ctx.modifiers.alt_key(),
                );
            }
        }
    }
}
