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
/// Each variant carries a `#[strum(serialize = "<lowercase>")]` —
/// the canonical lowercase token `KeyBind::parse` produces and
/// that mouse handlers feed into
/// `ResolvedKeybinds::action_for_context`. `IntoStaticStr` exposes
/// it via `<&'static str>::from(self)`, surfaced as
/// [`MouseGesture::key_name`]. The PascalCase emit form (the
/// shape the user types in `keybinds.json`) is the variant name
/// itself; [`pascal_form`] returns it via the same `EnumIter`
/// walk both directions share.
///
/// `LeftClick` and `RightClick` were previously reserved-but-not-
/// dispatched; per CODE_CONVENTIONS §5 (no half-features) they were
/// removed. A future commit that adds a real dispatch site can
/// reintroduce the variant in the same patch as its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum_macros::EnumIter, strum_macros::IntoStaticStr)]
pub enum MouseGesture {
    /// Left-button held down + cursor movement past the drag threshold,
    /// only when the press landed on empty canvas. Continuous: the bound
    /// action's body runs for the duration of the press. Dispatched
    /// from `event_cursor_moved` for `Action::PanCanvas` only.
    #[strum(serialize = "leftdrag")]
    LeftDrag,
    /// Two left-button presses within the double-click time + distance
    /// window with matching `ClickHit`. Dispatched.
    #[strum(serialize = "doubleclick")]
    DoubleClick,
    /// Single middle-button press. Dispatched.
    #[strum(serialize = "middleclick")]
    MiddleClick,
    /// One mouse-wheel tick upward (zoom-in by convention). Dispatched
    /// when the console isn't open.
    #[strum(serialize = "wheelup")]
    WheelUp,
    /// One mouse-wheel tick downward (zoom-out by convention). Same.
    #[strum(serialize = "wheeldown")]
    WheelDown,
}

impl MouseGesture {
    /// Canonical lowercase binding-string token for this gesture.
    /// The same token `KeyBind::parse` produces from `"DoubleClick"`,
    /// `"MiddleClick"`, etc. Mouse handlers feed this directly into
    /// `ResolvedKeybinds::action_for_context`. Backed by strum's
    /// `IntoStaticStr` derive — the per-variant `#[strum(serialize)]`
    /// attribute is the source of truth.
    pub fn key_name(self) -> &'static str {
        self.into()
    }

    /// PascalCase emit form for this gesture — the variant name
    /// itself, which is the shape the user types in `keybinds.json`.
    /// Used by [`KeyBind::to_binding_string`] so a parsed-then-
    /// emitted gesture round-trips to its canonical capitalisation
    /// rather than the lowercased internal form.
    pub fn pascal_form(self) -> &'static str {
        match self {
            MouseGesture::LeftDrag => "LeftDrag",
            MouseGesture::DoubleClick => "DoubleClick",
            MouseGesture::MiddleClick => "MiddleClick",
            MouseGesture::WheelUp => "WheelUp",
            MouseGesture::WheelDown => "WheelDown",
        }
    }
}

/// Look up the [`MouseGesture::pascal_form`] for a known
/// `lower`-case token, or `None` for keyboard names. Walks
/// `MouseGesture::iter()` so adding a new gesture variant
/// auto-extends the round-trip without touching this fn.
fn gesture_emit_form(lower: &str) -> Option<&'static str> {
    use strum::IntoEnumIterator;
    MouseGesture::iter()
        .find(|g| g.key_name() == lower)
        .map(MouseGesture::pascal_form)
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
                        return Err(format!("keybind '{}' has multiple non-modifier keys", input));
                    }
                    key = Some(part);
                }
            }
        }

        match key {
            Some(key) => Ok(KeyBind {
                key,
                ctrl,
                shift,
                alt,
            }),
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
