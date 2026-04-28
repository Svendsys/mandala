// SPDX-License-Identifier: MPL-2.0

//! `KeyBind` parser/matcher and the two `winit::Key` ↔ binding-string
//! shims (`normalize_key_name`, `key_to_name`). Pure data — no
//! platform-specific concerns.
//!
//! Mouse gestures share this same parser. A binding string like
//! `"DoubleClick"` or `"Shift+MiddleClick"` parses into the same
//! [`KeyBind`] struct as a keyboard binding, with the gesture's
//! canonical lowercase name in the `key` field. Mouse handlers
//! synthesize the same name via [`gesture_key_name`] before calling
//! `ResolvedKeybinds::action_for_context`, so the lookup table is
//! universal across input devices.

use winit::keyboard::Key;

/// A parsed keybinding: a logical key name plus modifier flags. Key names
/// are normalized to lowercase during parsing so comparisons are
/// case-insensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBind {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// User-driven mouse gestures that participate in the keybind lookup.
///
/// Each variant has a canonical binding-string form ([`gesture_key_name`])
/// that mouse handlers feed through `KeyBind::matches` exactly the way
/// keyboard names go through it.
///
/// **Dispatch status (current branch).** `DoubleClick`, `MiddleClick`,
/// `LeftDrag`, `WheelUp`, `WheelDown` are dispatched through
/// `dispatch_action` from their respective handlers (`event_mouse_click`,
/// `event_cursor_moved`, `run_native` wheel branch). `LeftClick` and
/// `RightClick` are **parsed but not yet dispatched** — they're
/// reserved tokens so user keybind files don't fail validation, but
/// no handler currently looks up an Action for them. Adding a
/// `LeftClick` dispatch site is non-trivial because a single
/// left-press is already consumed by the selection state machine;
/// any future binding on `LeftClick` would need a clear post-
/// selection point in the mouse handler. `RightClick` has no
/// non-color-picker dispatch site at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseGesture {
    /// Single left-button press. **Reserved, not dispatched.** Parsed
    /// so user configs can name it; no current handler looks it up.
    LeftClick,
    /// Left-button held down + cursor movement past the drag threshold,
    /// only when the press landed on empty canvas. Continuous: the bound
    /// action's body runs for the duration of the press. Dispatched
    /// from `event_cursor_moved` for `Action::PanCanvas` only.
    LeftDrag,
    /// Two left-button presses within the double-click time + distance
    /// window with matching `ClickHit`. Dispatched.
    DoubleClick,
    /// Single middle-button press. Dispatched.
    MiddleClick,
    /// Single right-button press. **Reserved, not dispatched.** The
    /// only handler that currently consumes a right-press is the
    /// color picker (`event_mouse_click.rs` color-picker branch),
    /// which doesn't go through the dispatch funnel.
    RightClick,
    /// One mouse-wheel tick upward (zoom-in by convention). Dispatched
    /// when the console isn't open.
    WheelUp,
    /// One mouse-wheel tick downward (zoom-out by convention). Same.
    WheelDown,
}

/// Canonical lowercase binding-string token for a [`MouseGesture`].
/// The same token `KeyBind::parse` produces from `"DoubleClick"`,
/// `"MiddleClick"`, etc. Mouse handlers feed this directly into
/// `ResolvedKeybinds::action_for_context`.
pub fn gesture_key_name(g: MouseGesture) -> &'static str {
    match g {
        MouseGesture::LeftClick => "leftclick",
        MouseGesture::LeftDrag => "leftdrag",
        MouseGesture::DoubleClick => "doubleclick",
        MouseGesture::MiddleClick => "middleclick",
        MouseGesture::RightClick => "rightclick",
        MouseGesture::WheelUp => "wheelup",
        MouseGesture::WheelDown => "wheeldown",
    }
}

/// PascalCase emit form for a recognised gesture token. Used by
/// `to_binding_string` so a parsed-then-emitted gesture round-trips
/// to its canonical capitalisation rather than the lowercased
/// internal form.
fn gesture_emit_form(lower: &str) -> Option<&'static str> {
    match lower {
        "leftclick" => Some("LeftClick"),
        "leftdrag" => Some("LeftDrag"),
        "doubleclick" => Some("DoubleClick"),
        "middleclick" => Some("MiddleClick"),
        "rightclick" => Some("RightClick"),
        "wheelup" => Some("WheelUp"),
        "wheeldown" => Some("WheelDown"),
        _ => None,
    }
}

impl KeyBind {
    /// Parse a binding string like `"Ctrl+Z"`, `"Shift+Alt+Delete"`, or
    /// just `"Escape"`. Modifier order doesn't matter; whitespace is
    /// tolerated; key names are matched case-insensitively.
    pub fn parse(input: &str) -> Result<Self, String> {
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut key: Option<String> = None;

        for raw in input.split('+') {
            let part = raw.trim().to_ascii_lowercase();
            if part.is_empty() {
                continue;
            }
            match part.as_str() {
                "ctrl" | "control" | "cmd" | "command" | "meta" | "super" => ctrl = true,
                "shift" => shift = true,
                "alt" | "option" => alt = true,
                _ => {
                    if key.is_some() {
                        return Err(format!(
                            "keybind '{}' has multiple non-modifier keys",
                            input
                        ));
                    }
                    key = Some(part);
                }
            }
        }

        match key {
            Some(key) => Ok(KeyBind { key, ctrl, shift, alt }),
            None => Err(format!("keybind '{}' has no key", input)),
        }
    }

    /// Returns true if this binding matches the given logical key name and
    /// modifier state. The caller is expected to have normalized `key_name`
    /// to lowercase via `normalize_key_name`.
    pub fn matches(&self, key_name: &str, ctrl: bool, shift: bool, alt: bool) -> bool {
        self.key == key_name && self.ctrl == ctrl && self.shift == shift && self.alt == alt
    }

    /// Render the binding back to a `Ctrl+Shift+Alt+Key` string form.
    /// Inverse of `parse` up to modifier-order normalisation — parsing
    /// this output must produce an equal `KeyBind`, which is locked in
    /// by `test_keybind_string_round_trip`.
    pub fn to_binding_string(&self) -> String {
        let mut parts: Vec<&str> = Vec::with_capacity(4);
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        // Recognised mouse gestures emit in PascalCase so a parsed-
        // then-emitted binding string round-trips to its canonical
        // form. Other keys emit lowercase as stored.
        let key_display: String = gesture_emit_form(&self.key)
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.key.clone());
        let joined = parts.join("+");
        if joined.is_empty() {
            key_display
        } else {
            format!("{}+{}", joined, key_display)
        }
    }
}

/// Normalize a winit logical-key representation to the same lowercase form
/// `KeyBind::parse` uses. The caller passes the string form it extracted
/// from its key event (character or named-key debug name) and this function
/// lowercases and trims it.
pub fn normalize_key_name(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

/// Convert a winit `Key` into the lowercase string form that
/// `KeyBind::parse` produces, so keybind comparison is symmetric.
/// Pairs with `normalize_key_name`; the two together produce comparable
/// strings from either the stored-config side or the live-event side.
pub fn key_to_name(key: &Key) -> Option<String> {
    match key {
        Key::Character(c) => Some(normalize_key_name(c.as_ref())),
        Key::Named(named) => Some(normalize_key_name(&format!("{:?}", named))),
        _ => None,
    }
}
