// SPDX-License-Identifier: MPL-2.0

//! `dispatch_action` — the single entry point that runs `Action`
//! bodies on native. Mouse handlers and the keyboard handler funnel
//! through here. The cross-platform stage (`action_core`, which asks
//! the owner families first) runs before the native-only arms below:
//! the console, the pan the drag state machine holds, and the files.
//! Adding a new behavior is variant + default + arm, in that order;
//! never inline a body in a handler.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::keybinds::Action;

use super::super::console_input::{
    rebuild_console_overlay, save_console_history, save_document_to_bound_path,
};
use super::super::input_context::InputHandlerContext;
use super::super::{DragState, HeldInput, PanOwner};
use crate::application::console::ConsoleState;

pub(in crate::application::app) use super::cross_dispatch::DispatchHit;
use super::DispatchOutcome;

/// Quote a free-form string (typically a filesystem path) so the
/// console parser sees it as a single token. Wraps with `"..."`
/// unconditionally and escapes both `\` (→ `\\`) and `"` (→ `\"`)
/// so Windows-style paths and embedded quotes round-trip cleanly
/// through `parser::tokenize`'s quoted-string handling. Order
/// matters: backslash MUST be escaped before quote, otherwise a
/// path ending in `\` produces an unterminated quoted token.
/// Used by the parametric filesystem Action arms.
fn quote_console_arg(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() + 2);
    escaped.push('"');
    for ch in s.chars() {
        if ch == '\\' || ch == '"' {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push('"');
    escaped
}

/// What [`Action::PanCanvas`] does with the drag state it finds.
///
/// Pure so the arm is pinnable at `DragState` level — the arm itself
/// takes an `InputHandlerContext`, i.e. a live wgpu device, which only
/// the suite's GPU-backed tier stands up (TEST_CONVENTIONS §T8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanCanvasRoute {
    /// Nothing is in flight that a pan would destroy — arm a
    /// [`PanOwner::Toggled`] pan. A held input that dispatched this
    /// claims it next ([`dispatch_held_action`]); anything else leaves
    /// it toggled on.
    Arm,
    /// A toggled pan is running: this dispatch is the toggle, and ends
    /// it.
    ToggleOff,
    /// A pan some input is holding is running. It ends on that input's
    /// release, so leave it exactly as it is — this includes a key's own
    /// auto-repeat, which re-dispatches `PanCanvas` while the key that
    /// holds the pan is down.
    Held,
    /// Another gesture owns the drag state with a user-named effect
    /// still to fire (`DragState::would_abandon_gesture`). Leave it
    /// exactly as it is, and do nothing else.
    Refuse,
}

/// Route one `Action::PanCanvas` dispatch against the drag state it
/// finds. The guard belongs on the Action rather than on any one of the
/// gestures that reach it (CODE_CONVENTIONS §3): the middle button, a
/// left drag, and any key, macro step, console line or IPC
/// `act.action` naming `pan_canvas`.
///
/// **Toggle or hold is the input's, not the arm's** (#153,
/// [`PanOwner`]). A toggled pan was armed by an input with no release,
/// so the next `pan_canvas` is the only dispatch that can end it; a held
/// one already has its end coming and a second dispatch must not
/// pre-empt it.
fn route_pan_canvas(drag_state: &DragState) -> PanCanvasRoute {
    match drag_state {
        DragState::Panning {
            owner: PanOwner::Toggled,
        } => PanCanvasRoute::ToggleOff,
        DragState::Panning {
            owner: PanOwner::Held(_),
        } => PanCanvasRoute::Held,
        state if state.would_abandon_gesture() => PanCanvasRoute::Refuse,
        _ => PanCanvasRoute::Arm,
    }
}

/// Run `action` through the funnel: the cross-platform stage first
/// (`action_core::dispatch_compatible`, which asks the owner families
/// before anything), then the native-only arms.
pub(in crate::application::app) fn dispatch_action(
    action: Action,
    ctx: &mut InputHandlerContext<'_>,
    hit: Option<&DispatchHit>,
) -> DispatchOutcome {
    let cross_outcome = {
        let (mut core, _) = ctx.split_borrow();
        super::action_core::dispatch_compatible(&action, &mut core, hit)
    };
    if !matches!(cross_outcome, DispatchOutcome::Unhandled) || super::families::dispatch(&action).is_some() {
        return cross_outcome;
    }
    dispatch_native_arms(action, ctx)
}

/// [`dispatch_action`] on behalf of a *held* input — a mouse button
/// or a key that is down now and will come up — so a pan the
/// dispatch arms ends on `input`'s release (#153). A dispatch with no
/// release behind it (a macro step, a console line, IPC `act.action`)
/// calls [`dispatch_action`] and leaves its pan toggled.
pub(in crate::application::app) fn dispatch_held_action(
    action: Action,
    ctx: &mut InputHandlerContext<'_>,
    hit: Option<&DispatchHit>,
    input: HeldInput,
) -> DispatchOutcome {
    let was_panning = ctx.drag_state.is_panning();
    let outcome = dispatch_action(action, ctx, hit);
    ctx.drag_state.hand_armed_pan_to(was_panning, input);
    outcome
}

/// The native-only arms: what the cross-platform stage leaves.
fn dispatch_native_arms(action: Action, ctx: &mut InputHandlerContext<'_>) -> DispatchOutcome {
    match action {
        // The loop ends at its next exit check (`app::pump::iterate`),
        // through the same shutdown a closed window takes.
        Action::Quit => {
            *ctx.quit_requested = true;
            DispatchOutcome::Handled
        }
        Action::OpenConsole => {
            if ctx.console_state.is_open() {
                save_console_history(ctx.console_history);
                *ctx.console_state = ConsoleState::Closed;
                ctx.renderer.rebuild_console_overlay_buffers(ctx.app_scene, None);
            } else {
                *ctx.console_state = ConsoleState::open(ctx.console_history.clone());
                if let Some(doc) = ctx.document.as_ref() {
                    rebuild_console_overlay(
                        ctx.console_state,
                        doc,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.keybinds,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        // Every `Console*` variant funnels here; the fan-out body lives
        // in `console_input::dispatch::dispatch_console_action`.
        Action::ConsoleClose
        | Action::ConsoleSubmit
        | Action::ConsoleTabComplete
        | Action::ConsoleHistoryUp
        | Action::ConsoleHistoryDown
        | Action::ConsoleCursorLeft
        | Action::ConsoleCursorRight
        | Action::ConsoleCursorHome
        | Action::ConsoleCursorEnd
        | Action::ConsoleDeleteBack
        | Action::ConsoleDeleteForward
        | Action::ConsoleInsertSpace
        | Action::ConsoleClearLine
        | Action::ConsoleJumpStart
        | Action::ConsoleJumpEnd
        | Action::ConsoleKillToStart
        | Action::ConsoleKillWord
        | Action::ConsoleScrollUp
        | Action::ConsoleScrollDown
        | Action::ConsoleScrollPageUp
        | Action::ConsoleScrollPageDown
        | Action::ConsoleScrollEnd
        | Action::ConsoleScrollHome => {
            // Every Console* body edits the open modal. With the console
            // closed there is nothing for one to act on, so a macro step
            // or an IPC `act.action` sending one reports that nothing ran.
            if !ctx.console_state.is_open() {
                return DispatchOutcome::Unhandled;
            }
            super::super::console_input::dispatch_console_action(&action, ctx);
            DispatchOutcome::Handled
        }
        Action::SaveDocument => match ctx.document.as_mut().map(save_document_to_bound_path) {
            Some(true) => DispatchOutcome::Handled,
            Some(false) | None => DispatchOutcome::Unhandled,
        },
        Action::PanCanvas => match route_pan_canvas(ctx.drag_state) {
            // Armed toggled; a held input claims it on the way out of
            // `dispatch_held_action`, and its release ends it there.
            PanCanvasRoute::Arm => {
                *ctx.drag_state = DragState::Panning {
                    owner: PanOwner::Toggled,
                };
                DispatchOutcome::Handled
            }
            PanCanvasRoute::ToggleOff => {
                *ctx.drag_state = DragState::None;
                DispatchOutcome::Handled
            }
            PanCanvasRoute::Held => {
                log::debug!("dispatch: PanCanvas ignored (a held input owns the pan); state stays put");
                DispatchOutcome::Refused(
                    "a pan is already running, held by a button or key; its release ends it",
                )
            }
            PanCanvasRoute::Refuse => {
                log::debug!(
                    "dispatch: PanCanvas ignored (a pending gesture owns the pointer); state stays put"
                );
                DispatchOutcome::Refused(
                    "a press is pending a gesture that has not fired; its release ends it",
                )
            }
        },
        // The file Actions run as the console line they mirror, so the
        // console's path checks, extension check and reporting are the
        // one body every surface reaches.
        Action::NewDocument => run_console_line_for("new", None, ctx),
        Action::OpenDocument(ref path) => run_console_line_for("open", Some(path), ctx),
        Action::SaveDocumentAs(ref path) => run_console_line_for("save", Some(path), ctx),
        Action::NewDocumentAt(ref path) => run_console_line_for("new", Some(path), ctx),
        _ => {
            log::debug!("dispatch: {:?} not handled at Document context", action);
            DispatchOutcome::Unhandled
        }
    }
}

/// Run the console line `verb [path]` for a file Action. The Action
/// applied only if its line did: a missing file or an unwritable path is
/// `Unhandled`, so a caller that reads the outcome (a macro's `any_ran`,
/// an IPC reply) is not told the map was opened or saved when it was not.
fn run_console_line_for(
    verb: &str,
    path: Option<&String>,
    ctx: &mut InputHandlerContext<'_>,
) -> DispatchOutcome {
    let line = match path {
        Some(path) => format!("{verb} {}", quote_console_arg(path)),
        None => verb.to_string(),
    };
    match crate::application::app::console_input::exec::execute_console_line(ctx, &line) {
        Some(run) => {
            crate::application::app::console_input::exec::log_console_line(&line, &run.outcome);
            if run.outcome.applied() {
                DispatchOutcome::Handled
            } else {
                DispatchOutcome::Unhandled
            }
        }
        None => {
            log::warn!(
                "dispatch: {}: no document loaded; skipping '{}'",
                verb,
                baumhard::util::log::console_line_excerpt(&line)
            );
            DispatchOutcome::Unhandled
        }
    }
}

/// Run a macro by id through the cross-platform step loop
/// ([`super::macro_core::dispatch_macro`]), which owns the privilege
/// gate. Returns `true` if any step ran.
///
/// `after_step`, when given, is called with the app as it stands after
/// each step that ran (see [`MacroStepObserver`]).
pub(in crate::application::app) fn dispatch_macro(
    macro_id: &str,
    ctx: &mut InputHandlerContext<'_>,
    after_step: Option<MacroStepObserver<'_>>,
) -> bool {
    let mut target = NativeMacroDispatchTarget { ctx, after_step };
    super::macro_core::dispatch_macro(macro_id, &mut target)
}

/// Run an anonymous step sequence through the same gated step loop
/// [`dispatch_macro`] runs a registered macro through, as a macro
/// from tier `source`. `label` names the sequence in diagnostics.
/// `after_step` is as for [`dispatch_macro`].
pub(in crate::application::app) fn run_macro_steps(
    label: &str,
    steps: &[crate::application::macros::MacroStep],
    source: crate::application::source_tier::SourceTier,
    ctx: &mut InputHandlerContext<'_>,
    after_step: Option<MacroStepObserver<'_>>,
) -> bool {
    let mut target = NativeMacroDispatchTarget { ctx, after_step };
    super::macro_core::run_macro_steps(label, steps, source, &mut target)
}

/// An observer called between macro steps, handed the handler
/// context as it stands after the step. It reads the context and
/// changes nothing.
pub(in crate::application::app) type MacroStepObserver<'o> = &'o mut dyn FnMut(&InputHandlerContext<'_>);

/// Native impl of [`super::macro_core::MacroDispatchTarget`]: wraps
/// `&mut InputHandlerContext` and forwards each operation to the native
/// helpers (`dispatch_action`, `execute_console_line`).
struct NativeMacroDispatchTarget<'a, 'b, 'o> {
    ctx: &'a mut InputHandlerContext<'b>,
    after_step: Option<MacroStepObserver<'o>>,
}

impl super::macro_core::MacroDispatchTarget for NativeMacroDispatchTarget<'_, '_, '_> {
    fn registry(&self) -> &crate::application::macros::MacroRegistry {
        self.ctx.macros
    }

    fn dispatch_action(&mut self, action: Action) -> DispatchOutcome {
        dispatch_action(action, self.ctx, None)
    }

    fn execute_console_line(&mut self, line: &str) -> bool {
        // Every verb runs against a document, so a macro fired before
        // one loads soft-skips. A line that ran but failed did not apply
        // either. Neither bumps the macro's `any_ran`.
        match crate::application::app::console_input::exec::execute_console_line(self.ctx, line) {
            Some(run) => {
                crate::application::app::console_input::exec::log_console_line(line, &run.outcome);
                run.outcome.applied()
            }
            None => {
                log::warn!(
                    "macros: ConsoleLine step: no document loaded; skipping '{}'",
                    baumhard::util::log::console_line_excerpt(line)
                );
                false
            }
        }
    }

    fn run_edit(&mut self, id: &str, at: &crate::application::macros::MacroTarget) -> bool {
        let ctx = &mut *self.ctx;
        let Some(doc) = ctx.document.as_mut() else {
            return false;
        };
        super::families::edits::run_step(doc, ctx.view.as_ref(), ctx.edits, id, at, ctx.clock.now_ms())
    }

    fn step_finished(&mut self) {
        if let Some(observe) = self.after_step.as_mut() {
            observe(self.ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The arms touch the renderer (wgpu), which only the GPU-backed
    // tier of `TEST_CONVENTIONS.md §T8` stands up — `app/tests_gpu.rs`
    // drives `Undo`, `Redo` and the selection Actions through the real
    // arms there. The pan's route is a pure helper, tested here.

    /// A press pending a gesture that has not fired yet (`PendingRight`'s
    /// `RightClick` / `RightDrag`) refuses a pan; every other state that
    /// owes nothing arms one.
    #[test]
    fn test_pan_canvas_arms_from_every_state_that_owes_nothing_and_refuses_a_pending_right_press() {
        for drag in [
            DragState::None,
            DragState::SelectingRect {
                start_canvas: glam::Vec2::ZERO,
                current_canvas: glam::Vec2::new(10.0, 10.0),
            },
            DragState::Pending {
                start_pos: (0.0, 0.0),
                hit: crate::application::app::ClickHit::Empty,
            },
        ] {
            assert_eq!(route_pan_canvas(&drag), PanCanvasRoute::Arm);
        }
        let right = DragState::PendingRight {
            start_pos: (0.0, 0.0),
            start_canvas: glam::Vec2::ZERO,
            hit: crate::application::app::ClickHit::Empty,
        };
        assert_eq!(route_pan_canvas(&right), PanCanvasRoute::Refuse);
        assert!(
            right.right_drag_hit().is_some(),
            "a pending right press builds its drag hit"
        );
        assert!(DragState::None.right_drag_hit().is_none());
    }

    /// **A `pan_canvas` with no release behind it toggles** (#153): the
    /// next `pan_canvas` that finds a toggled pan ends it.
    #[test]
    fn test_pan_canvas_ends_a_toggled_pan() {
        assert_eq!(
            route_pan_canvas(&DragState::Panning {
                owner: PanOwner::Toggled,
            }),
            PanCanvasRoute::ToggleOff
        );
    }

    /// **...but leaves a held pan to the input holding it.** The
    /// negative control for the toggle above: a key holding a pan
    /// auto-repeats, and each repeat re-dispatches `PanCanvas`, so a
    /// toggle here would flip the pan off and on for as long as the key
    /// is down.
    #[test]
    fn test_pan_canvas_leaves_a_held_pan_to_its_input() {
        for input in [
            HeldInput::Key("space".into()),
            HeldInput::Button(crate::application::platform::input::MouseButton::Middle),
            HeldInput::Button(crate::application::platform::input::MouseButton::Left),
        ] {
            assert_eq!(
                route_pan_canvas(&DragState::Panning {
                    owner: PanOwner::Held(input.clone()),
                }),
                PanCanvasRoute::Held,
                "a pan held by {input:?} ends on its release, not on a dispatch"
            );
        }
    }

    #[test]
    fn test_quote_console_arg_wraps_plain_path_in_double_quotes() {
        assert_eq!(super::quote_console_arg("/tmp/x.json"), "\"/tmp/x.json\"");
    }

    #[test]
    fn test_quote_console_arg_handles_paths_with_spaces() {
        // Embedded whitespace is the whole reason quoting exists —
        // the tokenizer would otherwise split the path into multiple
        // positionals.
        assert_eq!(
            super::quote_console_arg("/tmp/some dir/x.json"),
            "\"/tmp/some dir/x.json\"",
        );
    }

    #[test]
    fn test_quote_console_arg_escapes_embedded_double_quotes() {
        assert_eq!(
            super::quote_console_arg(r#"/tmp/he said "hi"/x.json"#),
            r#""/tmp/he said \"hi\"/x.json""#,
        );
    }

    #[test]
    fn test_quote_console_arg_escapes_backslashes_for_windows_paths() {
        assert_eq!(
            super::quote_console_arg(r"C:\Users\foo\map.json"),
            r#""C:\\Users\\foo\\map.json""#,
        );
    }

    #[test]
    fn test_quote_console_arg_handles_path_ending_in_backslash() {
        // Pre-fix this would produce `"C:\\foo\"` — an unterminated
        // quoted token. With the backslash escape it produces
        // `"C:\\foo\\"` which round-trips cleanly.
        assert_eq!(super::quote_console_arg(r"C:\foo\"), r#""C:\\foo\\""#);
    }
}
