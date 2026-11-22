// SPDX-License-Identifier: MPL-2.0

//! Console line-editor: per-keystroke dispatch, completion-popup
//! helpers, command execution, overlay rebuild, and history
//! persistence. `mod.rs` owns `rebuild_console_overlay` (shared by
//! dispatch + exec) and the scrollback push helpers.

#![cfg(not(target_arch = "wasm32"))]

mod clock;
mod completion;
mod dispatch;
mod edit;
mod edits;
pub(in crate::application::app) mod exec;
mod history;
mod presentations;
mod record;
mod screenshot;
mod verbs;

pub(in crate::application::app) use dispatch::dispatch_console_action;
pub(super) use dispatch::handle_console_key;
pub(super) use exec::save_document_to_bound_path;
pub(super) use history::{load_console_history, save_console_history};

/// Open the missing-font prompt (#329) when `missing` holds a family:
/// the console, opened if it is closed, with the report
/// (`missing_fonts::report_lines`) in its scrollback and
/// `font replace "<family>" ` on its input line, the completion showing
/// the fonts it can be replaced with. The map stays loaded and drawn
/// behind it; Escape closes it as it closes any console line. Returns
/// whether it changed the console, which the caller then draws
/// ([`rebuild_console_overlay`]). Nothing for a map no font is missing
/// from.
pub(in crate::application::app) fn open_missing_font_prompt(
    console_state: &mut ConsoleState,
    history: &[String],
    doc: &MindMapDocument,
    missing: &[baumhard::document::fonts::MissingFamily],
) -> bool {
    let Some(line) = crate::application::missing_fonts::replace_line(missing) else {
        return false;
    };
    if !console_state.is_open() {
        *console_state = ConsoleState::open(history.to_vec());
    }
    for report in crate::application::missing_fonts::report_lines(missing) {
        push_scrollback_output_with_sample(console_state, report.text, report.sample_face);
    }
    if let ConsoleState::Open { input, cursor, .. } = console_state {
        *cursor = baumhard::util::grapheme_chad::count_grapheme_clusters(&line);
        *input = line;
    }
    completion::recompute_console_completions(console_state, Some(doc));
    true
}

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
            sample_face: None,
        });
        // New output should be visible — pin the view to the bottom
        // so the user's eyes don't have to chase it.
        *scroll_offset = 0;
    }
}

/// Push an output line, in the console's face, followed by a
/// [`FONT_SAMPLE`](crate::application::console::FONT_SAMPLE) in the
/// family `sample_face` names (none when `None`). The font dispatch uses
/// this for `font list`, so each compiled family shows what it looks
/// like.
pub(super) fn push_scrollback_output_with_sample(
    state: &mut ConsoleState,
    text: String,
    sample_face: Option<String>,
) {
    if let ConsoleState::Open {
        scrollback,
        scroll_offset,
        ..
    } = state
    {
        scrollback.push(ConsoleLine::Output { text, sample_face });
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
                sample_face: None,
            },
            ConsoleLine::Output { text, sample_face } => ConsoleOverlayLine {
                text: text.clone(),
                kind: ConsoleOverlayLineKind::Output,
                sample_face: sample_face.clone(),
            },
            ConsoleLine::Error(t) => ConsoleOverlayLine {
                text: t.clone(),
                kind: ConsoleOverlayLineKind::Error,
                sample_face: None,
            },
        })
        .collect();
    let completion_geo: Vec<ConsoleOverlayCompletion> = completions
        .iter()
        .map(|c| ConsoleOverlayCompletion {
            text: c.text.clone(),
            hint: c.hint.clone(),
            sample_face: c.sample_face.clone(),
        })
        .collect();
    let geometry = ConsoleOverlayGeometry {
        input: input.clone(),
        cursor_grapheme: cursor,
        scrollback: scrollback_lines,
        completions: completion_geo,
        selected_completion,
        default_font: keybinds.console_font,
        font_size: keybinds.console_font_size,
    };
    renderer.rebuild_console_overlay_buffers(app_scene, Some(&geometry));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The missing-font prompt opens the console on the replacement**
    /// (#329): the report in the scrollback, `font replace "<family>" `
    /// on the input line with the cursor at its end, and the completion
    /// offering the fonts a map can name, each by name with a sample in
    /// its face where the face can draw one. A map
    /// with nothing missing leaves a closed console closed. Controls:
    /// the second half, and the completion's rows being the replacement
    /// slot's (no subverb row), so the line is the one the user goes on
    /// from.
    #[test]
    fn test_the_missing_font_prompt_opens_on_the_replacement() {
        let doc = MindMapDocument::from_json_str(
            r#"{"format":1,"name":"f","nodes":[{"id":1,"position":[0.0,0.0],"font":"Nope Sans"}]}"#,
            None,
            50,
        )
        .expect("the map loads");
        let missing = baumhard::document::fonts::missing_families(doc.doc());
        let mut state = ConsoleState::Closed;
        assert!(open_missing_font_prompt(
            &mut state,
            &["earlier".to_string()],
            &doc,
            &missing
        ));
        let ConsoleState::Open {
            input,
            cursor,
            history,
            scrollback,
            completions,
            ..
        } = &state
        else {
            panic!("the prompt opens the console");
        };
        assert_eq!(input, "font replace \"Nope Sans\" ");
        assert_eq!(*cursor, input.chars().count(), "the cursor ends the line");
        assert_eq!(history, &["earlier".to_string()], "the history is kept");
        let report = crate::application::missing_fonts::report_lines(&missing);
        assert_eq!(scrollback.len(), report.len(), "the report, line for line");
        assert!(
            completions
                .iter()
                .any(|c| c.display == "Public Pixel" && c.sample_face.as_deref() == Some("Public Pixel")),
            "the completion offers the compiled families, each with a sample in its face"
        );
        assert!(
            completions
                .iter()
                .all(|c| c.sample_face.is_none() || c.sample_face.as_deref() == Some(c.display.as_str())),
            "every row is a family, its sample in its own face or no sample"
        );
        assert!(
            completions.iter().all(|c| c.display != "reset"),
            "the replacement slot offers no sentinel"
        );
        let mut closed = ConsoleState::Closed;
        assert!(!open_missing_font_prompt(&mut closed, &[], &doc, &[]));
        assert!(!closed.is_open(), "control: nothing missing, nothing opens");
    }
}
