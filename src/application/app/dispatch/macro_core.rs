// SPDX-License-Identifier: MPL-2.0

//! Cross-platform macro dispatch. The privilege-gating step loop
//! that drives a `Macro` (a `Vec<MacroStep>`) lives here, abstracted
//! over a [`MacroDispatchTarget`] trait so native and WASM share
//! the same body byte-for-byte. Re-implementing the loop on
//! either target is **forbidden** — the privilege gate
//! (`SourceTier::allows_action`, `allows_console_line`) is the
//! threat-model defence and must be single-sourced.
//!
//! - **Native** impl lives in
//!   `super::dispatch::NativeMacroDispatchTarget` wrapping
//!   `&mut InputHandlerContext`.
//! - **WASM** impl lives in
//!   `super::run_wasm::WasmMacroDispatchTarget` wrapping
//!   `&mut WasmInputState` (added in Track B Commit 5).
//!
//! The trait surface is intentionally small: the four operations
//! the macro body needs (Action, CustomMutation, ConsoleLine,
//! selection-resolve) plus a registry borrow for the id lookup at
//! the head of the loop.

use crate::application::keybinds::Action;
use crate::application::macros::{MacroRegistry, GATE_REJECTED_CODE};
use crate::application::source_tier::SourceTier;

use super::cross_dispatch::DispatchOutcome;

/// Per-target operations the cross-platform [`dispatch_macro`]
/// step loop calls. Each implementor wraps a context type
/// (`InputHandlerContext` on native, `WasmInputState` on WASM)
/// and forwards each operation to the right platform-specific
/// helper.
///
/// **Privilege gating happens in [`dispatch_macro`], not in the
/// impl.** The impl just provides the mechanics; the policy is
/// in the loop body so it can't drift between targets.
pub(in crate::application::app) trait MacroDispatchTarget {
    /// Borrow the macro registry for the id lookup at the head of
    /// the loop.
    fn registry(&self) -> &MacroRegistry;

    /// Run a single Action step against the target's full state.
    /// Returns whether the action was recognised and dispatched
    /// (mirrors `dispatch::dispatch_action`'s `DispatchOutcome`).
    fn dispatch_action(&mut self, action: Action) -> DispatchOutcome;

    /// Execute a free-form console line. Reaches the loop ONLY
    /// after the privilege gate (`SourceTier::allows_console_line`)
    /// approved the step — non-User tiers fail-closed-abort the
    /// macro before this method is called.
    ///
    /// Returns `true` when the line actually ran (the macro's
    /// `any_ran` flag bumps), `false` on a soft-skip path that
    /// looks like execution but didn't change document state — e.g.
    /// no document loaded on native, or the WASM no-console-runtime
    /// log path. Pinning `any_ran` to actual execution preserves
    /// pre-Track-B native semantics (a `[ConsoleLine("open foo")]`
    /// macro fired before any document loads doesn't consume the
    /// keystroke event).
    fn execute_console_line(&mut self, line: &str) -> bool;

    /// A step has run to completion (a skipped or gate-rejected one
    /// has not). The seam an observer hangs between steps on: the IPC
    /// `verbose` event level reports one change per step instead of
    /// one per macro. Nothing else needs it, so the default does
    /// nothing, and the WASM target keeps it.
    fn step_finished(&mut self) {}
}

/// Run a macro by id against a [`MacroDispatchTarget`]. Returns
/// `true` iff at least one step actually executed (the same
/// semantics native callers rely on for "macro keybind was
/// handled").
///
/// Looks the macro up, then runs its steps through
/// [`run_macro_steps`] under the tier its loader pinned.
pub(in crate::application::app) fn dispatch_macro<T: MacroDispatchTarget>(
    macro_id: &str,
    target: &mut T,
) -> bool {
    let (mac, source) = match target.registry().get_with_source(macro_id) {
        Some((m, s)) => (m.clone(), s),
        None => {
            log::warn!("macros: dispatch of unknown macro id '{}'", macro_id);
            return false;
        }
    };
    run_macro_steps(macro_id, &mac.steps, source, target)
}

/// The gated step loop: run `steps` against `target` as a macro from
/// tier `source`. Returns `true` iff at least one step actually
/// executed. `label` names the macro in diagnostics: the registered
/// id, or a description of where an anonymous sequence came from.
///
/// This is the only body that runs macro steps, whether the macro
/// was looked up by id ([`dispatch_macro`]) or arrived anonymously
/// (the IPC `act.macro` command's `steps`, at the `User` tier
/// `format/ipc.md` §Trust model assigns IPC). The privilege gate
/// lives here and nowhere else, so no caller can reach the steps
/// around it.
///
/// **Privilege gating** runs per-step. A non-User tier's first
/// destructive Action or any ConsoleLine step **fail-closed
/// aborts** the remaining steps so a hostile pattern like
/// `[BenignAction, RejectedAction, SaveDocument]` can't smuggle
/// post-rejected destructive steps past the gate. Both rejections
/// are logged under [`GATE_REJECTED_CODE`], so a caller collecting
/// diagnostics can tell the abort from an ordinary degrade.
pub(in crate::application::app) fn run_macro_steps<T: MacroDispatchTarget>(
    label: &str,
    steps: &[crate::application::macros::MacroStep],
    source: SourceTier,
    target: &mut T,
) -> bool {
    use crate::application::macros::MacroStep;

    let mut any_ran = false;
    for step in steps {
        match step {
            MacroStep::Action { action } => {
                // Privilege gate symmetric with `ConsoleLine` below.
                // Non-User tiers cannot fire destructive / clipboard /
                // I/O Actions. Fail-closed: a rejected privileged step
                // aborts the rest of the macro so a `[DeleteSelection,
                // ConsoleLine(rejected), SaveDocument]` pattern can't
                // sneak its outer steps past the gate.
                if !source.allows_action(action) {
                    log::warn!(
                        code = GATE_REJECTED_CODE;
                        "macros: macro '{}' (source {:?}): Action {:?} rejected — \
                         tier may not invoke destructive / I/O Actions; \
                         aborting remaining steps",
                        label,
                        source,
                        action,
                    );
                    return any_ran;
                }
                let outcome = target.dispatch_action(action.clone());
                if matches!(outcome, DispatchOutcome::Handled) {
                    any_ran = true;
                }
            }
            MacroStep::CustomMutation { id, .. } => {
                // The step fires an edit; edits are V1-D1's (#250),
                // and until then it runs nothing and bumps nothing.
                if super::families::edits::fire(id) == DispatchOutcome::Handled {
                    any_ran = true;
                }
            }
            MacroStep::ConsoleLine { line } => {
                // **Privilege gate.** `ConsoleLine` runs an arbitrary
                // console verb, including filesystem-touching ones.
                // Only `SourceTier::User` macros may carry it — every
                // other tier comes from a source the user did not
                // necessarily author, so it cannot do file I/O via a
                // macro.
                if !source.allows_console_line() {
                    // Fail-closed: a tier that's not allowed to run
                    // console verbs aborts the rest of the macro.
                    // `continue` would let post-gate Action steps
                    // still run, which combined with destructive
                    // Actions could leave the user in an unexpected
                    // state (e.g. `[DeleteSelection,
                    // ConsoleLine(rejected), SaveDocument]` would
                    // persist the post-delete state without consent).
                    log::warn!(
                        code = GATE_REJECTED_CODE;
                        "macros: macro '{}' (source {:?}): ConsoleLine step rejected — \
                         only User-tier macros may run console verbs; \
                         aborting remaining steps",
                        label,
                        source,
                    );
                    return any_ran;
                }
                if target.execute_console_line(line) {
                    any_ran = true;
                }
            }
        }
        target.step_finished();
    }
    any_ran
}

#[cfg(test)]
mod tests {
    //! Mock-target tests for the privilege-gating step loop. Drives
    //! `run_macro_steps` and `dispatch_macro` against a recording mock
    //! so the gate contract is verified at the actual loop body, not
    //! just through the per-step simulator in `macros/tests.rs`.

    use super::*;
    use crate::application::keybinds::Action;
    use crate::application::macros::{Macro, MacroRegistry, MacroStep, MacroTarget};
    use crate::application::source_tier::SourceTier;

    /// Mock target: records every method invocation in order.
    struct MockTarget {
        registry: MacroRegistry,
        calls: Vec<String>,
        action_outcome: DispatchOutcome,
        console_line_executed: bool,
    }

    impl MockTarget {
        fn new(registry: MacroRegistry) -> Self {
            Self {
                registry,
                calls: Vec::new(),
                action_outcome: DispatchOutcome::Handled,
                console_line_executed: true,
            }
        }
    }

    impl MacroDispatchTarget for MockTarget {
        fn registry(&self) -> &MacroRegistry {
            &self.registry
        }
        fn dispatch_action(&mut self, action: Action) -> DispatchOutcome {
            self.calls.push(format!("action:{:?}", action));
            self.action_outcome
        }
        fn execute_console_line(&mut self, line: &str) -> bool {
            self.calls.push(format!("console:{}", line));
            self.console_line_executed
        }
    }

    fn registry_with(macros: Vec<(Macro, SourceTier)>) -> MacroRegistry {
        let mut r = MacroRegistry::new();
        for (m, s) in macros {
            r.insert(m, s);
        }
        r
    }

    fn macro_with_steps(id: &str, steps: Vec<MacroStep>) -> Macro {
        Macro {
            id: id.into(),
            name: String::new(),
            description: String::new(),
            steps,
        }
    }

    /// The anonymous-sequence entry runs the same gated loop as a
    /// registered macro. At `User`, the tier IPC's `act.macro steps`
    /// pins, an Action and a console line run; at `App` the same
    /// console line is refused and the rest aborted.
    #[test]
    fn test_run_macro_steps_gates_an_anonymous_sequence_by_the_tier_it_is_given() {
        let steps = vec![
            MacroStep::Action { action: Action::Undo },
            MacroStep::ConsoleLine {
                line: "fps on".into(),
            },
        ];
        let mut user = MockTarget::new(MacroRegistry::new());
        assert!(run_macro_steps(
            "(ipc steps)",
            &steps,
            SourceTier::User,
            &mut user
        ));
        assert_eq!(
            user.calls,
            vec!["action:Undo".to_string(), "console:fps on".to_string()]
        );

        let mut app = MockTarget::new(MacroRegistry::new());
        assert!(run_macro_steps("(ipc steps)", &steps, SourceTier::App, &mut app));
        assert_eq!(
            app.calls,
            vec!["action:Undo".to_string()],
            "the console line is refused"
        );
    }

    /// Both rejections are logged under `GATE_REJECTED_CODE`, so a
    /// caller collecting diagnostics can tell the abort from a degrade.
    #[test]
    fn test_a_gate_rejection_is_logged_with_its_code() {
        use baumhard::util::log::capture::capture;
        baumhard::util::test_logger::install();
        for (label, step) in [
            ("console-c0de", MacroStep::ConsoleLine { line: "save".into() }),
            (
                "action-c0de",
                MacroStep::Action {
                    action: Action::SaveDocument,
                },
            ),
        ] {
            let mut t = MockTarget::new(MacroRegistry::new());
            let (ran, captured) = capture(|| run_macro_steps(label, &[step], SourceTier::App, &mut t));
            assert!(!ran);
            assert!(t.calls.is_empty(), "{label}: nothing ran");
            let record = captured
                .records
                .iter()
                .find(|r| r.message.contains(label))
                .unwrap_or_else(|| panic!("{label} is logged: {captured:?}"));
            assert_eq!(record.code.as_deref(), Some(GATE_REJECTED_CODE));
        }
    }

    /// Fail-closed: `[Undo, ConsoleLine, SaveDocument]` from the App
    /// tier runs Undo, refuses the console line and never reaches the
    /// save, which it would also have refused.
    #[test]
    fn test_a_rejected_step_aborts_the_rest_of_the_macro() {
        let m = macro_with_steps(
            "m",
            vec![
                MacroStep::Action { action: Action::Undo },
                MacroStep::ConsoleLine {
                    line: "save /tmp/evil".into(),
                },
                MacroStep::Action {
                    action: Action::SaveDocument,
                },
            ],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::App)]));
        assert!(dispatch_macro("m", &mut t), "the Undo ran");
        assert_eq!(t.calls, vec!["action:Undo".to_string()]);
    }

    /// The edit step runs nothing until V1-D1 (#250) rebuilds edits, and
    /// so bumps nothing: a macro of only that step is not handled.
    /// Control: the same macro with an Action beside it is.
    #[test]
    fn test_an_edit_step_runs_nothing_until_edits_are_rebuilt() {
        let edit = MacroStep::CustomMutation {
            id: "nudge".into(),
            target: MacroTarget::CurrentSelection,
        };
        let only = macro_with_steps("only", vec![edit.clone()]);
        let mixed = macro_with_steps("mixed", vec![edit, MacroStep::Action { action: Action::Undo }]);
        let mut t = MockTarget::new(registry_with(vec![
            (only, SourceTier::User),
            (mixed, SourceTier::User),
        ]));
        assert!(!dispatch_macro("only", &mut t));
        assert!(t.calls.is_empty());
        assert!(dispatch_macro("mixed", &mut t));
        assert_eq!(t.calls, vec!["action:Undo".to_string()]);
    }

    /// An Action that does not apply, and a console line that soft-skips,
    /// do not count as having run.
    #[test]
    fn test_unhandled_steps_do_not_count_as_run() {
        let m = macro_with_steps(
            "m",
            vec![
                MacroStep::Action { action: Action::Undo },
                MacroStep::ConsoleLine {
                    line: "fps on".into(),
                },
            ],
        );
        let mut t = MockTarget::new(registry_with(vec![(m, SourceTier::User)]));
        t.action_outcome = DispatchOutcome::Unhandled;
        t.console_line_executed = false;
        assert!(!dispatch_macro("m", &mut t));
        assert_eq!(t.calls.len(), 2, "both steps were offered");
    }

    /// An unknown id runs nothing, and neither does a known macro with
    /// no steps. Control: the same registry runs a macro that has one.
    #[test]
    fn test_an_unknown_macro_runs_nothing() {
        let empty = macro_with_steps("empty", Vec::new());
        let one = macro_with_steps("one", vec![MacroStep::Action { action: Action::Undo }]);
        let mut t = MockTarget::new(registry_with(vec![
            (empty, SourceTier::User),
            (one, SourceTier::User),
        ]));
        assert!(!dispatch_macro("no-such-macro", &mut t));
        assert!(!dispatch_macro("empty", &mut t), "an empty macro ran nothing");
        assert!(t.calls.is_empty());
        assert!(dispatch_macro("one", &mut t), "control");
        assert_eq!(t.calls, vec!["action:Undo".to_string()]);
    }
}
