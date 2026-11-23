// SPDX-License-Identifier: MPL-2.0

//! **The section text editor** (V1-C8, #249; design §9): a multi-line
//! editor on one section's spans, as a transaction on the document's
//! writer. Both targets.
//!
//! ```text
//! open    ─▶ begin (Interaction::TextEdit)
//! a change ─▶ stage(replace_spans(section, buffer.spans()))   one request, never a list
//! commit  ─▶ one history entry for the whole session (an unchanged one cancels)
//! cancel  ─▶ the document exactly as before the editor opened
//! ```
//!
//! The editor holds an [`EditBuffer`] — the working copy of the
//! section's spans — and a caret and a shift-select anchor in grapheme
//! clusters of it. It never writes a glyph: what is typed is staged into
//! the document, and the projector and the renderer draw it like any
//! change; the caret and the selected range are the view's chrome
//! (`MapView::follow_editor`), placed from the projector's shaping of the
//! section. Nothing reads the document back from glyphs.
//!
//! `CreateOrphanNodeAndEdit` is one session too ([`TextSession::open_orphan`]):
//! the node is applied once into the transaction's base, where it keeps
//! its `NodeId` and its ident, and each keystroke restages only its
//! section's spans (ruling R7 on #238); commit is one entry holding the
//! node and its text, cancel leaves no node.
//!
//! **Keys** (CODE_CONVENTIONS §3's modal carve-out, as on `main`): a key
//! bound in `InputContext::TextEdit` runs its `TextEdit*` Action through
//! the funnel ([`TextSession::apply_action`]); any other key is a literal
//! — Enter a line break, Tab a tab, a `Key::Character` payload its
//! printable characters as one counted splice ([`typed_payload`]), which
//! is how an IME or dead-key cluster arrives, exactly as `main`'s IME
//! primitive took it.
//!
//! The renderer-free core is here, in [`draft`] — what both text
//! editors are made of — and in [`session`]; the arms that run it are the
//! text family's (`dispatch::families::text`).

pub(in crate::application::app) mod draft;
mod session;

#[cfg(test)]
pub(in crate::application::app) mod tests;

pub(in crate::application::app) use draft::Draft;
pub(in crate::application::app) use session::TextSession;

use baumhard::util::grapheme_chad;

use crate::application::keybinds::{Action, InputContext, ResolvedKeybinds};
use crate::application::platform::input::{Key, Modifiers};

/// The section text editor: closed, or one session. See the
/// [module docs](self).
#[derive(Debug, Default)]
pub(in crate::application::app) enum TextEditor {
    /// No editor is open.
    #[default]
    Closed,
    /// One session, on one section; boxed, so a closed editor is small.
    Open(Box<TextSession>),
}

impl TextEditor {
    /// Whether an editor is open. O(1).
    pub(in crate::application::app) fn is_open(&self) -> bool {
        matches!(self, TextEditor::Open(_))
    }

    /// The open session. O(1).
    pub(in crate::application::app) fn session(&self) -> Option<&TextSession> {
        match self {
            TextEditor::Open(session) => Some(&**session),
            TextEditor::Closed => None,
        }
    }

    /// The open session, mutably. O(1).
    pub(in crate::application::app) fn session_mut(&mut self) -> Option<&mut TextSession> {
        match self {
            TextEditor::Open(session) => Some(&mut **session),
            TextEditor::Closed => None,
        }
    }

    /// Close the editor and hand back its session, if one was open.
    /// O(1).
    pub(in crate::application::app) fn take(&mut self) -> Option<TextSession> {
        match std::mem::take(self) {
            TextEditor::Open(session) => Some(*session),
            TextEditor::Closed => None,
        }
    }
}

/// What one input did to a session's buffer and caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum Edit {
    /// Nothing: the key or the Action did not apply.
    None,
    /// The caret or the anchor moved; the text did not change, so
    /// nothing is staged and only the view's chrome follows.
    Caret,
    /// The text changed: the buffer is staged.
    Text,
}

/// The printable characters of a `Key::Character` payload — control
/// characters dropped, as `main` dropped them — or `None` for a key that
/// carries none. An IME commit or a dead-key sequence arrives as one
/// such payload, and is inserted whole. O(the payload).
pub(in crate::application::app) fn typed_payload(logical_key: &Key) -> Option<String> {
    let Key::Character(chars) = logical_key else {
        return None;
    };
    let printable: String = chars.as_str().chars().filter(|ch| !ch.is_control()).collect();
    (!printable.is_empty()).then_some(printable)
}

/// The `TextEdit*` Action `key` is bound to in `InputContext::TextEdit`,
/// with `modifiers`, or `None` when the key is a literal. O(the context's
/// bindings).
pub(in crate::application::app) fn bound_action(
    key_name: Option<&str>,
    modifiers: &Modifiers,
    keybinds: &ResolvedKeybinds,
) -> Option<Action> {
    key_name.and_then(|name| {
        keybinds.action_for_context(
            InputContext::TextEdit,
            name,
            modifiers.control_key(),
            modifiers.shift_key(),
            modifiers.alt_key(),
        )
    })
}

/// The grapheme index of the start of the line holding `caret`.
fn line_start(text: &str, caret: usize) -> usize {
    grapheme_chad::line_bounds_at(text, caret).0
}

/// The grapheme index of the end of the line holding `caret`.
fn line_end(text: &str, caret: usize) -> usize {
    grapheme_chad::line_bounds_at(text, caret).1
}

/// `caret` one line up, keeping its column where the line above is long
/// enough; unmoved on the first line. As on `main`.
fn line_up(text: &str, caret: usize) -> usize {
    let start = line_start(text, caret);
    if start == 0 {
        return caret;
    }
    let prev_end = start - 1;
    let prev_start = line_start(text, prev_end);
    prev_start + (caret - start).min(prev_end - prev_start)
}

/// `caret` one line down, keeping its column where the line below is
/// long enough; unmoved on the last line. As on `main`.
fn line_down(text: &str, caret: usize) -> usize {
    let total = grapheme_chad::count_grapheme_clusters(text);
    let (start, end) = grapheme_chad::line_bounds_at(text, caret);
    if end >= total {
        return caret;
    }
    let next_start = end + 1;
    let next_end = line_end(text, next_start);
    next_start + (caret - start).min(next_end - next_start)
}
