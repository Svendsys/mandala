// SPDX-License-Identifier: MPL-2.0

//! Console line execution and Ctrl+S save. Split from the dispatcher
//! so the command-runner concern (parse → execute → drain effects)
//! lives independently from the per-keystroke edit logic.

use crate::application::color_picker::ColorPickerState;
use crate::application::console::commands::Command;
use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleEffects, ConsoleSideEffect, ConsoleState, ExecResult, LineOutcome};
use crate::application::document::MindMapDocument;
use crate::application::view::MapView;

use super::super::input_context::InputHandlerContext;
use super::super::DragState;
use super::{push_scrollback_error, push_scrollback_output, push_scrollback_output_in_font};

/// What one [`execute_console_line`] did.
#[must_use = "a console line's outcome is its only report: print it, log it, or return it"]
pub(in crate::application::app) struct LineRun {
    /// What the verb produced.
    pub(in crate::application::app) outcome: LineOutcome,
    /// Whether anything the window draws may have changed: a synced
    /// change, any side effect (the FPS overlay included), or the
    /// console closing. A caller deciding whether to request a redraw
    /// reads this rather than assuming one.
    pub(in crate::application::app) changed_view: bool,
}

/// The verbs that leave an animated edit running: `clock`, which
/// drives it (a step is how a virtual clock moves it); `help`, `fps`,
/// `mode`, `screenshot` and `record`, which change nothing of the map;
/// `save`, which writes the map as committed, a running edit not yet in
/// it (§9); and `edit`, whose `apply` settles in the edit funnel and
/// whose `inspect` reports the animation. Every other verb settles it.
pub(in crate::application::app) const LEAVE_ANIMATION_RUNNING: [&str; 8] = [
    "clock",
    "help",
    "fps",
    "mode",
    "screenshot",
    "record",
    "save",
    "edit",
];

/// Parse and execute a console line against `ctx`, and return what the
/// verb produced. `None` when no document is loaded, because every verb
/// runs against one. Takes the one `ConsoleSideEffect` the verb
/// returned — an FPS mode, a mode flip, a document replacement, a
/// recording or a clock step — and performs it here, then brings the
/// view up to date with whatever the line changed (`MapView::sync`,
/// which does nothing when the line changed nothing).
///
/// The one entry point for every caller: the console prompt, the
/// file-path Actions, a macro `ConsoleLine` step, and the IPC
/// `act.console` command. Each takes the same `InputHandlerContext`,
/// so a field the drain needs is threaded in one place, not four.
///
/// **Writes nothing to the scrollback.** Rendering the outcome is the
/// caller's decision, because the callers do not agree on where it
/// goes. Only the prompt prints the outcome into the console, through
/// [`print_console_line`]. The others log a failure through
/// [`log_console_line`], and IPC also returns the outcome in its
/// reply. No caller but the prompt touches the scrollback
/// (`format/ipc.md` §act).
///
/// An animated edit still running is settled — fast-forwarded and
/// committed — before the verb runs, so a verb that reads a value to
/// build its change reads the map the edit leaves (D28), unless the
/// verb is one of [`LEAVE_ANIMATION_RUNNING`].
///
/// `#[must_use]` here as well as on [`LineRun`]: the lint does not
/// reach through the `Option` this returns, so without it a bare call
/// that drops the line's only report would compile clean.
#[must_use = "a console line's outcome is its only report: print it, log it, or return it"]
pub(in crate::application::app) fn execute_console_line(
    ctx: &mut InputHandlerContext<'_>,
    line: &str,
) -> Option<LineRun> {
    let doc = ctx.document.as_mut()?;
    if line.trim().is_empty() {
        return Some(LineRun {
            outcome: LineOutcome::Blank,
            changed_view: false,
        });
    }
    let (cmd, args) = match parse(line) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        ParseResult::Empty => {
            return Some(LineRun {
                outcome: LineOutcome::Blank,
                changed_view: false,
            })
        }
        ParseResult::Unknown(head) => {
            return Some(LineRun {
                outcome: LineOutcome::Unknown(head),
                changed_view: false,
            })
        }
    };
    let cmd: &'static Command = cmd;
    // Settle an animated edit first, as `dispatch_compatible` does for
    // an Action, unless the verb leaves it running.
    if !LEAVE_ANIMATION_RUNNING.contains(&cmd.name) {
        doc.finish_animation();
    }
    let (result, side_effect, close_after) = {
        let mut effects = ConsoleEffects::new(doc);
        let result = (cmd.execute)(&Args::new(&args), &mut effects);
        (result, effects.side_effect.take(), effects.close_console)
    };

    // A recording's report is the line's output, and is taken here: it
    // starts or ends a recording and changes nothing the window draws,
    // so there is no console state to move.
    if let Some(ConsoleSideEffect::Record(line)) = &side_effect {
        let outcome = LineOutcome::Ran(super::record::take(ctx, line));
        return Some(LineRun {
            outcome,
            changed_view: false,
        });
    }

    // So is the clock's. A step runs frames, so it owes the window one
    // showing where it left the app; a read or a switch shows nothing
    // new.
    if let Some(ConsoleSideEffect::Clock(line)) = &side_effect {
        let (result, changed_view) = super::clock::take(ctx, *line);
        return Some(LineRun {
            outcome: LineOutcome::Ran(result),
            changed_view,
        });
    }

    // So is an edit's: what it lists, describes or ran. One that ran
    // changed the map, which the sync below draws.
    let mut edit_report = None;
    if let Some(ConsoleSideEffect::Edit(line)) = &side_effect {
        edit_report = Some(super::edits::take(ctx, line));
    }
    if let Some(result) = edit_report {
        let synced = match (ctx.document.as_mut(), ctx.view.as_mut()) {
            (Some(doc), Some(view)) => view.sync(doc, ctx.renderer),
            _ => false,
        };
        return Some(LineRun {
            outcome: LineOutcome::Ran(result),
            changed_view: synced,
        });
    }

    // A screenshot's report is the line's output, so it is taken here,
    // before anything else could run, and replaces the verb's silent
    // `Ok`. It draws through a camera of its own and changes nothing
    // the window draws.
    if let Some(ConsoleSideEffect::Screenshot(screenshot)) = &side_effect {
        let outcome = LineOutcome::Ran(super::screenshot::take(ctx, screenshot));
        return Some(LineRun {
            outcome,
            changed_view: false,
        });
    }

    let had_side_effect = side_effect.is_some();
    if let Some(effect) = side_effect {
        apply_side_effect(ctx, effect);
    }
    // Whatever the line changed — an edit through the writer, nothing
    // at all — the view catches up from the change feed. A swapped
    // document was projected whole as it was installed, so its sync
    // finds nothing to do.
    let synced = match (ctx.document.as_mut(), ctx.view.as_mut()) {
        (Some(doc), Some(view)) => view.sync(doc, ctx.renderer),
        _ => false,
    };
    if close_after {
        *ctx.console_state = ConsoleState::Closed;
        ctx.renderer.rebuild_console_overlay_buffers(ctx.app_scene, None);
    }
    Some(LineRun {
        outcome: LineOutcome::Ran(result),
        changed_view: synced || had_side_effect || close_after,
    })
}

/// Perform the side effect a verb asked for. The four that are a
/// line's whole report (`Record`, `Clock`, `Edit`, `Screenshot`) are taken by
/// [`execute_console_line`] before this is reached.
fn apply_side_effect(ctx: &mut InputHandlerContext<'_>, effect: ConsoleSideEffect) {
    match effect {
        ConsoleSideEffect::ReplaceDocument(new_doc) => install_document(ctx, *new_doc),
        // The decree bus clears the overlay buffers when toggled off;
        // the renderer re-shapes them on the next frame when toggled on.
        ConsoleSideEffect::SetFpsDisplay(mode) => ctx.renderer.set_fps_display(mode),
        ConsoleSideEffect::SetInteractionMode(mode) => {
            *ctx.interaction_mode = mode;
            ctx.renderer.set_mode_status_text(None);
        }
        // Per CODE_CONVENTIONS §9 (interactive paths must not panic),
        // log + soft-skip instead of `unreachable!`.
        ConsoleSideEffect::Screenshot(_)
        | ConsoleSideEffect::Record(_)
        | ConsoleSideEffect::Clock(_)
        | ConsoleSideEffect::Edit(_) => {
            log::error!("console: {effect:?} reached the side-effect handler; it is taken before — ignoring");
        }
    }
}

/// Install `new_doc` in place of the open document — `open` and `new`
/// — and project it whole (`MapView::load`).
///
/// Every interaction in flight names elements, a hit or a layer of the
/// document just replaced, so none of it survives the swap: a pending
/// drag, a remembered click, an open picker, the interaction mode.
/// Reachable whenever the swap does not come from the prompt, which
/// the key-stealing modals otherwise guard: an IPC `act.console
/// "open …"`, or a file Action bound to a mouse gesture.
///
/// The window's cursor icon is deliberately left alone: it can still
/// show a button hand from the old document until the pointer next
/// moves, when `event_cursor_moved` recomputes it.
fn install_document(ctx: &mut InputHandlerContext<'_>, new_doc: MindMapDocument) {
    let doc = ctx.document.insert(new_doc);
    *ctx.view = Some(MapView::load(doc, ctx.renderer));
    *ctx.color_picker_state = ColorPickerState::Closed;
    *ctx.drag_state = DragState::None;
    *ctx.last_click = None;
    *ctx.interaction_mode = super::super::InteractionMode::Default;
    ctx.renderer.set_mode_status_text(None);
}

/// Print what [`execute_console_line`] produced to the console
/// prompt that submitted the line: output lines in their pinned
/// faces, a failure in the error color.
///
/// **The scrollback is the prompt's own transcript.** This is its only
/// writer. Every other caller of [`execute_console_line`] reports
/// through [`log_console_line`] instead: a key-bound file Action, a
/// macro step, and the IPC `act.console` / `act.action` /
/// `act.macro` commands. Those callers do not own the console. They
/// can fire while it is open (an IPC command, or a mouse binding),
/// and a line pushed into the scrollback from outside would show
/// output the user never typed for, in an overlay nothing rebuilds
/// until their next keystroke.
pub(in crate::application::app) fn print_console_line(
    outcome: LineOutcome,
    console_state: &mut ConsoleState,
) {
    if let Some(failure) = outcome.failure() {
        push_scrollback_error(console_state, failure);
        return;
    }
    match outcome {
        LineOutcome::Ran(ExecResult::Ok(s)) if !s.is_empty() => push_scrollback_output(console_state, s),
        LineOutcome::Ran(ExecResult::Lines(lines)) => {
            for line in lines {
                push_scrollback_output_in_font(console_state, line.text, line.font_family);
            }
        }
        // Blank lines, empty successes, and the failures handled above.
        _ => {}
    }
}

/// Report what [`execute_console_line`] produced for a caller that is
/// not the console prompt (see [`print_console_line`] for why such a
/// caller never writes the scrollback).
///
/// A failure is `warn!`ed, so a bound `open` of a missing file still
/// says why it did nothing (§9). Under `--ipc` the same line is the
/// command's reply warning. Successful output has no reader and is
/// dropped.
pub(in crate::application::app) fn log_console_line(line: &str, outcome: &LineOutcome) {
    if let Some(failure) = outcome.failure() {
        // The line is the user's (or a macro author's) text, the
        // failure echoes the token it rejected, and either may name a
        // file: a release build keeps this record, so both halves are
        // home-collapsed and excerpted — the failure at the wider
        // `FAILURE_WIDTH`, since it carries the diagnosis (§9).
        log::warn!(
            code = crate::application::console::LINE_FAILED_CODE;
            "console: `{}` failed: {}",
            baumhard::util::log::console_line_excerpt(line),
            baumhard::util::log::failure_excerpt(&failure)
        );
    }
}

/// Write the document to its bound `file_path` (`MindMapDocument::save_to`:
/// its `save_json`, written through a temporary file and a rename, then
/// `mark_saved`), and report the outcome to the log. This is the body of
/// `Action::SaveDocument` (`Ctrl+S` by default). When no path is bound
/// it `warn!`s a hint pointing at `save <path>` and leaves the document
/// as it was.
///
/// It reports through the log only, never the console scrollback, for
/// the reason [`print_console_line`] gives: the Action is not the
/// console prompt, and under `--ipc` it can fire while the console is
/// open. The `save` verb typed at the prompt reports through its own
/// `ExecResult`. Each failure carries a code, so a caller collecting
/// diagnostics can tell "nothing is bound" (fixable with a save-as)
/// from a write that failed.
///
/// Returns whether the document was written.
pub(in crate::application::app) fn save_document_to_bound_path(doc: &mut MindMapDocument) -> bool {
    use crate::application::console::{SAVE_FAILED_CODE, SAVE_UNBOUND_CODE};
    let Some(path) = doc.file_path.clone() else {
        log::warn!(code = SAVE_UNBOUND_CODE; "save: no file path bound; use `save <path>` to choose one");
        return false;
    };
    match doc.save_to(&path) {
        Ok(()) => true,
        Err(e) => {
            log::error!(code = SAVE_FAILED_CODE; "save: {}", baumhard::util::log::collapse_home_in(&e));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    /// Every verb that leaves an animated edit running is a verb the
    /// console has, by its own name: a renamed verb cannot silently
    /// start settling. Control: a misspelled entry is not found.
    #[test]
    fn test_every_verb_that_leaves_an_animation_running_is_a_verb() {
        let named = |name: &str| {
            crate::application::console::commands::command_by_name(name).is_some_and(|cmd| cmd.name == name)
        };
        for name in super::LEAVE_ANIMATION_RUNNING {
            assert!(named(name), "`{name}` is a verb");
        }
        assert!(!named("clocks"), "control");
    }

    use super::*;

    /// A failed line run by anything but the prompt is logged under
    /// `LINE_FAILED_CODE`, the code an IPC reply carries it as. A line
    /// that worked logs nothing.
    #[test]
    fn test_log_console_line_warns_a_failure_under_its_code_and_nothing_else() {
        use baumhard::util::log::capture::capture;

        baumhard::util::test_logger::install();
        let ((), captured) =
            capture(|| log_console_line("frobnicate-5d1e", &LineOutcome::Unknown("frobnicate-5d1e".into())));
        assert_eq!(captured.records.len(), 1, "{captured:?}");
        assert_eq!(
            captured.records[0].code.as_deref(),
            Some(crate::application::console::LINE_FAILED_CODE)
        );
        assert!(captured.records[0]
            .message
            .contains("unknown command: frobnicate-5d1e"));

        let ((), captured) = capture(|| {
            log_console_line("help", &LineOutcome::Ran(ExecResult::Ok("fine".into())));
            log_console_line("", &LineOutcome::Blank);
        });
        assert!(
            captured.records.is_empty(),
            "success and blank lines report nothing: {captured:?}"
        );
    }

    /// `SaveDocument`'s three outcomes, each with the code a caller can
    /// branch on: no bound path, a write that fails, and a write that
    /// succeeds and leaves the document clean.
    #[test]
    fn test_save_to_bound_path_reports_each_outcome_under_its_code() {
        use crate::application::console::{SAVE_FAILED_CODE, SAVE_UNBOUND_CODE};
        use baumhard::util::log::capture::capture;

        baumhard::util::test_logger::install();
        let mut doc = MindMapDocument::new_blank(None, 50);
        doc.create_root_for_test();
        let (saved, captured) = capture(|| save_document_to_bound_path(&mut doc));
        assert!(!saved);
        assert_eq!(captured.records[0].code.as_deref(), Some(SAVE_UNBOUND_CODE));

        let dir = baumhard::util::test_temp::TempDir::new("save-codes");
        doc.file_path = Some(
            dir.path()
                .join("missing-dir/m.mandala.json")
                .display()
                .to_string(),
        );
        let (saved, captured) = capture(|| save_document_to_bound_path(&mut doc));
        assert!(!saved, "a parent that does not exist cannot be written into");
        assert_eq!(captured.records[0].code.as_deref(), Some(SAVE_FAILED_CODE));
        assert!(doc.is_dirty(), "a failed save leaves the work unsaved");

        doc.file_path = Some(dir.path().join("m.mandala.json").display().to_string());
        let (saved, captured) = capture(|| save_document_to_bound_path(&mut doc));
        assert!(
            saved && !doc.is_dirty(),
            "a written save leaves the document clean"
        );
        assert!(captured.records.is_empty(), "{captured:?}");
    }
}
