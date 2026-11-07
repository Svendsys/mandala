// SPDX-License-Identifier: MPL-2.0

//! Clipboard operations on `ColorPickerState` — copy produces the
//! current hex color, paste accepts a hex string and sets the HSV
//! state, cut behaves identically to copy (the picker always shows a
//! color; there is nothing to "remove").

use baumhard::util::color::{hex_to_hsv_safe, hsv_to_hex};

use super::state::{ColorPickerState, OpenPicker};
/// What a copy or a cut of the picker produced.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipboardContent {
    /// The picker's color, as hex.
    Text(String),
    /// The picker is closed: there is nothing to copy.
    NotApplicable,
}

/// What a paste into the picker did.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The color changed.
    Applied,
    /// The pasted color is the one already shown.
    Unchanged,
    /// The picker is closed.
    NotApplicable,
    /// The text is not a hex color; the message says so.
    Invalid(String),
}

impl Outcome {
    fn applied(changed: bool) -> Self {
        if changed {
            Outcome::Applied
        } else {
            Outcome::Unchanged
        }
    }
}

impl ColorPickerState {
    /// The picker's color as hex, or [`ClipboardContent::NotApplicable`]
    /// while it is closed.
    pub fn clipboard_copy(&self) -> ClipboardContent {
        match self.as_open() {
            Some(OpenPicker {
                hue_deg, sat, val, ..
            }) => ClipboardContent::Text(hsv_to_hex(*hue_deg, *sat, *val)),
            None => ClipboardContent::NotApplicable,
        }
    }

    /// Set the picker's color from a hex string.
    pub fn clipboard_paste(&mut self, content: &str) -> Outcome {
        let Some(OpenPicker {
            hue_deg,
            sat,
            val,
            hover_preview,
            ..
        }) = self.as_open_mut()
        else {
            return Outcome::NotApplicable;
        };

        let trimmed = content.trim();
        let Some((h, s, v)) = hex_to_hsv_safe(trimmed) else {
            return Outcome::Invalid(format!("not a hex color: {}", trimmed));
        };

        let changed =
            hue_deg.to_bits() != h.to_bits() || sat.to_bits() != s.to_bits() || val.to_bits() != v.to_bits();

        *hue_deg = h;
        *sat = s;
        *val = v;
        *hover_preview = None;

        Outcome::applied(changed)
    }

    /// As [`Self::clipboard_copy`].
    pub fn clipboard_cut(&mut self) -> ClipboardContent {
        // The picker always displays a color — "cutting" it has no
        // clearable source state. Behave identically to copy.
        self.clipboard_copy()
    }
}
