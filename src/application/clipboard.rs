// SPDX-License-Identifier: MPL-2.0

//! Platform clipboard abstraction. Native uses `arboard`; WASM is a
//! logged stub pending an async-browser integration.

/// Read text from the system clipboard, or `None` on failure.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_clipboard() -> Option<String> {
    arboard::Clipboard::new()
        .ok()
        .and_then(|mut cb| cb.get_text().ok())
        .filter(|s| !s.is_empty())
}

/// Write text to the system clipboard; ignores failures (interactive
/// paths must not panic — §9).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_clipboard(text: &str) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text);
    }
}

#[cfg(target_arch = "wasm32")]
pub fn read_clipboard() -> Option<String> {
    log::debug!("clipboard read not yet supported on WASM");
    None
}

#[cfg(target_arch = "wasm32")]
pub fn write_clipboard(_text: &str) {
    log::debug!("clipboard write not yet supported on WASM");
}
