// SPDX-License-Identifier: MPL-2.0

//! `ResolvedKeybinds` — the runtime lookup table the event loop calls
//! into. Built once via `KeybindConfig::resolve`, then queried per
//! input event.

use baumhard::font::fonts::AppFont;

use super::action::Action;
use super::bind::KeyBind;
use super::context::InputContext;

/// The resolved form of a `KeybindConfig`: a flat list of `(Action,
/// KeyBind)` pairs. Lookup is linear — the list is small enough
/// (under 50 entries) that a hash map would only add overhead.
#[derive(Debug, Clone)]
pub struct ResolvedKeybinds {
    binds: Vec<(Action, KeyBind)>,
    /// Parsed `(KeyBind, edit_id)` pairs from
    /// `KeybindConfig::edit_bindings`. Checked after the
    /// built-in `action_for` lookup in the event loop — a key combo
    /// bound to both a built-in action and an edit resolves to the
    /// built-in action (action_for runs first).
    edit_binds: Vec<(KeyBind, String)>,
    /// Parsed `(KeyBind, macro_id)` pairs from
    /// `KeybindConfig::macro_bindings`. Resolved BEFORE edits and
    /// AFTER built-in actions, so a key bound to both a macro and an
    /// edit runs the macro.
    macro_binds: Vec<(KeyBind, String)>,
    /// The console's default face, resolved from `console_font` by
    /// `config::resolve_console_font`. `None` means cosmic-text's
    /// default face: either nothing was configured, or the configured
    /// family is not loaded (which warned at resolve time).
    // Native-driver-only, with `console_font_size`: both are read by
    // `app::console_input`, which is native-gated with the modal
    // shell.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub console_font: Option<AppFont>,
    /// Console overlay font size in pixels.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub console_font_size: f32,
}

impl ResolvedKeybinds {
    /// Construct a resolved table — called from `KeybindConfig::resolve`,
    /// which owns the validation + parsing of binding strings.
    pub(super) fn new(
        binds: Vec<(Action, KeyBind)>,
        edit_binds: Vec<(KeyBind, String)>,
        macro_binds: Vec<(KeyBind, String)>,
        console_font: Option<AppFont>,
        console_font_size: f32,
    ) -> Self {
        Self {
            binds,
            edit_binds,
            macro_binds,
            console_font,
            console_font_size,
        }
    }

    /// Resolve a mouse gesture to an action with modifier-fallback
    /// semantics. Tries the exact `(key, ctrl, shift, alt)` binding
    /// first; if no match, falls back to the unmodified `(key, false,
    /// false, false)` binding.
    ///
    /// Mouse gestures use this instead of `action_for_context` because
    /// modifiers on mouse gestures are typically decorations rather
    /// than distinct bindings — pre-branch behaviour was that
    /// `Ctrl+Wheel` zoomed exactly the same as a bare `Wheel`. Strict
    /// modifier matching would silently break that. Users who *do*
    /// want a modified gesture to mean something different just bind
    /// the modified form explicitly; the exact-match check above
    /// honours it.
    ///
    /// Always resolves in the `Document` context — the modal-steal
    /// cascade in `event_keyboard.rs` returns before any mouse
    /// handler runs, so mouse gestures only ever fire in Document
    /// context today.
    pub fn action_for_gesture(&self, key: &str, ctrl: bool, shift: bool, alt: bool) -> Option<Action> {
        if let Some(a) = self.action_for_context(InputContext::Document, key, ctrl, shift, alt) {
            return Some(a);
        }
        if ctrl || shift || alt {
            return self.action_for_context(InputContext::Document, key, false, false, false);
        }
        None
    }

    /// Return `true` if the action has at least one binding in the
    /// resolved table. Used by the dispatcher to gate "off-by-default"
    /// gesture sub-actions: the empty-canvas branch of
    /// `DoubleClickActivate` only fires `CreateOrphanNodeAndEdit` when
    /// the user has enabled it via any binding.
    #[cfg(test)]
    pub fn has_any_binding_for(&self, action: Action) -> bool {
        self.binds.iter().any(|(a, _)| *a == action)
    }

    /// Every `Action` variant kind that has at least one binding in
    /// the resolved table, regardless of combo or context.
    ///
    /// Test-gated: runtime lookups all go through a key event, and
    /// this asks the opposite question — *which actions came out of
    /// the config at all*. That is what the coverage test over the
    /// `keybind_surface!` table needs, because a declared field that
    /// produces no binding is exactly the silent-dead-keybind bug
    /// (issue #32) and cannot be seen from a per-combo lookup.
    #[cfg(test)]
    pub fn bound_action_kinds(&self) -> std::collections::HashSet<super::action::ActionKind> {
        self.binds
            .iter()
            .map(|(action, _)| super::action::ActionKind::from(action))
            .collect()
    }

    /// Return the action bound to the given key event, if any. The caller
    /// passes the normalized key name (see `normalize_key_name`) and the
    /// current modifier state. Searches all actions regardless of context —
    /// use `action_for_context` for context-aware resolution.
    ///
    /// Test-gated: every runtime lookup is context-aware and goes
    /// through [`Self::action_for_context`]. This context-blind
    /// spelling is how the keybind tests assert "this combo is bound
    /// at all" without naming a context.
    #[cfg(test)]
    pub fn action_for(&self, key: &str, ctrl: bool, shift: bool, alt: bool) -> Option<Action> {
        for (action, bind) in &self.binds {
            if bind.matches(key, ctrl, shift, alt) {
                return Some(action.clone());
            }
        }
        None
    }

    /// Resolve an action for a key event within a given input context.
    /// Tries context-specific actions first. If the context allows
    /// fallthrough and no context-specific action matched, tries
    /// the parent context.
    pub fn action_for_context(
        &self,
        context: InputContext,
        key: &str,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> Option<Action> {
        for (action, bind) in &self.binds {
            if bind.matches(key, ctrl, shift, alt) && action.context() == context {
                return Some(action.clone());
            }
        }
        if context.falls_through() {
            // Fallthrough target is always `Document` — see
            // `InputContext::falls_through` doc.
            for (action, bind) in &self.binds {
                if bind.matches(key, ctrl, shift, alt) && action.context() == InputContext::Document {
                    return Some(action.clone());
                }
            }
        }
        None
    }

    /// Return the macro id bound to the given key event, if any.
    /// Resolved AFTER built-in `action_for` and BEFORE
    /// `edit_for` — macros override edits on the same combo, so a
    /// user replacing a single-edit shortcut with a multi-step macro
    /// just adds the macro entry without un-binding the edit.
    pub fn macro_for(&self, key: &str, ctrl: bool, shift: bool, alt: bool) -> Option<&str> {
        for (bind, id) in &self.macro_binds {
            if bind.matches(key, ctrl, shift, alt) {
                return Some(id.as_str());
            }
        }
        None
    }

    /// Return the edit id bound to the given key event,
    /// if any. Called after `action_for` returns `None` — built-in
    /// actions win on a collision.
    pub fn edit_for(&self, key: &str, ctrl: bool, shift: bool, alt: bool) -> Option<&str> {
        for (bind, id) in &self.edit_binds {
            if bind.matches(key, ctrl, shift, alt) {
                return Some(id.as_str());
            }
        }
        None
    }

    /// Set or replace an edit binding at runtime. Returns
    /// the previous mutation id bound to the same combo, if any.
    /// The `combo_string` is re-parsed through `KeyBind::parse` so
    /// invalid inputs are rejected uniformly with the resolve-time
    /// path.
    ///
    /// Test-gated with its sibling
    /// [`Self::remove_edit_binding`]: edit
    /// bindings are established at resolve time from the config
    /// files and nothing edits them at runtime yet, so the pair's
    /// only callers are the tests that pin replace-and-return
    /// semantics for the console verb that will.
    #[cfg(test)]
    pub fn set_edit_binding(
        &mut self,
        combo_string: &str,
        edit_id: String,
    ) -> Result<Option<String>, String> {
        let bind = KeyBind::parse(combo_string)?;
        let mut prev = None;
        self.edit_binds.retain(|(b, id)| {
            if b == &bind {
                prev = Some(id.clone());
                false
            } else {
                true
            }
        });
        self.edit_binds.push((bind, edit_id));
        Ok(prev)
    }

    /// Remove the edit binding for the given combo.
    /// Returns the removed mutation id, if one was bound. Test-gated
    /// — see [`Self::set_edit_binding`].
    #[cfg(test)]
    pub fn remove_edit_binding(&mut self, combo_string: &str) -> Result<Option<String>, String> {
        let bind = KeyBind::parse(combo_string)?;
        let mut prev = None;
        self.edit_binds.retain(|(b, id)| {
            if b == &bind {
                prev = Some(id.clone());
                false
            } else {
                true
            }
        });
        Ok(prev)
    }
}
