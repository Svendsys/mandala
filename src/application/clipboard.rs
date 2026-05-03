// SPDX-License-Identifier: MPL-2.0

//! Platform clipboard abstraction. Native uses `arboard`; WASM is a
//! logged stub pending an async-browser integration.
//!
//! In addition to the OS-level text clipboard, this module owns an
//! in-process **structured section buffer**: a single-slot snapshot
//! of the last `MindSection` payload that `clipboard_copy` /
//! `clipboard_cut` produced. The OS clipboard always carries the
//! plain section text (so cross-app paste sees something sensible);
//! the structured buffer carries the full `text_runs` / `offset` /
//! `size` / `channel` / `trigger_bindings` set so within-app
//! section→section paste round-trips per-run formatting and section
//! chrome instead of falling back to template inheritance via
//! `set_section_text`. The two halves stay coherent through a
//! consistency check on read: structured paste only fires when the
//! OS clipboard's current text matches the buffer's `text` snapshot,
//! so a user who copies from another app between Mandala
//! copy-and-paste falls through to the plain-text branch
//! automatically.

use crate::application::console::traits::SectionPayload;
use std::sync::Mutex;

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

/// In-process structured section clipboard. Single-slot. Latest
/// `write_section_clipboard` overwrites any prior entry — matches
/// the OS clipboard's "one-thing-at-a-time" mental model.
struct SectionBufferEntry {
    text: String,
    payload: SectionPayload,
}

/// `Mutex` so concurrent reads/writes from the event loop's section-
/// copy and section-paste arms can't race; in practice the editor is
/// single-threaded but the lock is cheap and removes the assumption
/// from the contract.
static SECTION_BUFFER: Mutex<Option<SectionBufferEntry>> = Mutex::new(None);

/// Stash a structured section payload in the in-process buffer
/// alongside the OS clipboard's plain text. The `text` argument
/// must be byte-equal to what the caller wrote to the OS clipboard
/// for `read_section_clipboard` to ever return this payload (see
/// the `probe_text` consistency check there).
pub fn write_section_clipboard(text: String, payload: SectionPayload) {
    if let Ok(mut slot) = SECTION_BUFFER.lock() {
        *slot = Some(SectionBufferEntry { text, payload });
    }
}

/// Return the buffered structured payload **iff** the buffer's
/// `text` snapshot matches `probe_text` exactly (typically the
/// content of the current OS clipboard read). The match guards
/// against the user copying from another app between Mandala
/// copy-and-paste: when the OS text changed, the structured buffer
/// is stale and structured paste falls through to the plain-text
/// branch. Returns `None` when the buffer is empty, the lock is
/// poisoned (defensive), or the texts differ.
pub fn read_section_clipboard(probe_text: &str) -> Option<SectionPayload> {
    let slot = SECTION_BUFFER.lock().ok()?;
    let entry = slot.as_ref()?;
    if entry.text == probe_text {
        Some(entry.payload.clone())
    } else {
        None
    }
}

/// Clear the in-process structured section buffer. Used by tests
/// to reset state between runs; not part of the production
/// copy/paste flow (the buffer self-invalidates via the
/// `probe_text` consistency check on read).
#[cfg(test)]
pub fn clear_section_clipboard_for_tests() {
    if let Ok(mut slot) = SECTION_BUFFER.lock() {
        *slot = None;
    }
}

/// Serialise structured-clipboard tests so the shared
/// `SECTION_BUFFER` global doesn't race across `cargo test`'s
/// default parallel test threads. Tests that touch the buffer
/// `let _g = section_clipboard_test_guard();` at the top — the
/// guard scopes to the test body and releases on drop. Cheap
/// std `Mutex` (no extra dep), and the structured-clipboard
/// tests are <10 in number so contention is irrelevant.
#[cfg(test)]
pub fn section_clipboard_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL_LOCK: Mutex<()> = Mutex::new(());
    // `unwrap_or_else` so a poisoned mutex (a prior test
    // panicked while holding the guard) doesn't cascade-fail
    // every subsequent run — recover into a usable guard since
    // the protected `()` data has no invariant to corrupt.
    SERIAL_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
