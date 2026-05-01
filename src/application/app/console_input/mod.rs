// SPDX-License-Identifier: MPL-2.0

//! Console line-editor: per-keystroke dispatch, completion-popup
//! helpers, command execution, overlay rebuild, and history
//! persistence. `mod.rs` owns `rebuild_console_overlay` (shared by
//! dispatch + exec) and the scrollback push helpers.

#![cfg(not(target_arch = "wasm32"))]

mod completion;
mod dispatch;
mod edit;
pub(in crate::application::app) mod exec;
mod history;

pub(in crate::application::app) use dispatch::dispatch_console_action;
pub(super) use dispatch::handle_console_key;
pub(super) use exec::save_document_to_bound_path;
pub(super) use history::{load_console_history, save_console_history};

/// Wheel-driven scrollback adjustment. Used by the platform event
/// loop (native today; WASM in a future commit) to translate
/// accumulated wheel deltas into integer-line scroll steps. Same
/// clamp as the keyboard path.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn scroll_console_by_lines(
    state: &mut crate::application::console::ConsoleState,
    delta: i32,
) {
    edit::scroll_by_lines(state, delta);
}

/// Pure accumulator: fold a wheel delta into the running fractional
/// residue and return the integer line steps to apply. The event
/// loop calls this on every wheel event so slow scrolls accumulate
/// across ticks instead of rounding to zero.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn accumulate_wheel_lines(accum: &mut f32, dy: f32) -> i32 {
    edit::accumulate_wheel_lines(accum, dy)
}

use crate::application::console::{ConsoleLine, ConsoleState};
use crate::application::document::MindMapDocument;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::renderer::Renderer;

pub(super) fn push_scrollback_output(state: &mut ConsoleState, text: String) {
    if let ConsoleState::Open {
        scrollback,
        scroll_offset,
        ..
    } = state
    {
        scrollback.push(ConsoleLine::Output {
            text,
            font_family: None,
        });
        // New output should be visible — pin the view to the bottom
        // so the user's eyes don't have to chase it.
        *scroll_offset = 0;
    }
}

/// Push an output line whose text shapes in `font_family` (or the
/// console default when `None`). The font dispatch uses this for
/// `font list` so each row renders in its own face.
pub(super) fn push_scrollback_output_in_font(
    state: &mut ConsoleState,
    text: String,
    font_family: Option<String>,
) {
    if let ConsoleState::Open {
        scrollback,
        scroll_offset,
        ..
    } = state
    {
        scrollback.push(ConsoleLine::Output { text, font_family });
        *scroll_offset = 0;
    }
}

pub(super) fn push_scrollback_error(state: &mut ConsoleState, text: String) {
    if let ConsoleState::Open {
        scrollback,
        scroll_offset,
        ..
    } = state
    {
        scrollback.push(ConsoleLine::Error(text));
        *scroll_offset = 0;
    }
}

/// Build the console overlay geometry from the current state and
/// push it to the renderer. Called whenever the console opens, the
/// input changes, or scrollback / completions update.
pub(super) fn rebuild_console_overlay(
    console_state: &ConsoleState,
    _document: &MindMapDocument,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    keybinds: &ResolvedKeybinds,
) {
    use crate::application::renderer::{
        ConsoleOverlayCompletion, ConsoleOverlayGeometry, ConsoleOverlayLine, ConsoleOverlayLineKind,
        MAX_CONSOLE_SCROLLBACK_ROWS,
    };
    let (input, cursor, scrollback, completions, selected_completion, scroll_offset) = match console_state {
        ConsoleState::Closed => {
            renderer.rebuild_console_overlay_buffers(app_scene, None);
            return;
        }
        ConsoleState::Open {
            input,
            cursor,
            scrollback,
            completions,
            completion_idx,
            scroll_offset,
            ..
        } => (
            input,
            *cursor,
            scrollback,
            completions,
            *completion_idx,
            *scroll_offset,
        ),
    };
    // Clamp the scroll offset against the maximum reachable
    // position so a window-shrink or scrollback-shorten can never
    // strand the offset beyond the actual history.
    let max_offset = scrollback.len().saturating_sub(MAX_CONSOLE_SCROLLBACK_ROWS);
    let offset = scroll_offset.min(max_offset);
    // Slice "tail N starting from `len - offset`" so the
    // bottom-anchored rendering shape is preserved — the drawn
    // region is always the trailing window from the geometry's
    // perspective, while `offset` controls which window slides
    // under it.
    let end = scrollback.len().saturating_sub(offset);
    let start = end.saturating_sub(MAX_CONSOLE_SCROLLBACK_ROWS);
    let scrollback_lines: Vec<ConsoleOverlayLine> = scrollback[start..end]
        .iter()
        .map(|l| match l {
            ConsoleLine::Input(t) => ConsoleOverlayLine {
                text: t.clone(),
                kind: ConsoleOverlayLineKind::Input,
                font_family: None,
            },
            ConsoleLine::Output { text, font_family } => ConsoleOverlayLine {
                text: text.clone(),
                kind: ConsoleOverlayLineKind::Output,
                font_family: font_family.clone(),
            },
            ConsoleLine::Error(t) => ConsoleOverlayLine {
                text: t.clone(),
                kind: ConsoleOverlayLineKind::Error,
                font_family: None,
            },
        })
        .collect();
    let completion_geo: Vec<ConsoleOverlayCompletion> = completions
        .iter()
        .map(|c| ConsoleOverlayCompletion {
            text: c.text.clone(),
            hint: c.hint.clone(),
            font_family: c.font_family.clone(),
        })
        .collect();
    let geometry = ConsoleOverlayGeometry {
        input: input.clone(),
        cursor_grapheme: cursor,
        scrollback: scrollback_lines,
        completions: completion_geo,
        selected_completion,
        font_family: keybinds.console_font.clone(),
        font_size: keybinds.console_font_size,
    };
    renderer.rebuild_console_overlay_buffers(app_scene, Some(&geometry));
}
