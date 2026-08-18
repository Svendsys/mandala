// SPDX-License-Identifier: MPL-2.0

//! `fps on` / `fps off` / `fps debug` — toggle the yellow
//! screen-space FPS readout in the upper-left corner.
//!
//! - `fps on` → `FpsDisplayMode::Snapshot` — one frame's interval,
//!   re-sampled every ~200 frames. Quiet and stable for normal use.
//! - `fps debug` → `FpsDisplayMode::Debug` — rolling average over the
//!   last ~200 frames, updated every frame. Reacts live to load, for
//!   diagnosing perf regressions.
//! - `fps off` → `FpsDisplayMode::Off` — hide the readout.
//!
//! Signals the dispatcher via `ConsoleEffects::set_fps_display`; the
//! actual mode change reaches the renderer in `exec.rs` which calls
//! `Renderer::set_fps_display`.

use super::Command;
use crate::application::common::FpsDisplayMode;
use crate::application::console::completion::{
    prefix_filter, Completion, CompletionContext, CompletionState,
};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};

pub const COMMAND: Command = Command {
    name: "fps",
    aliases: &[],
    summary: "Toggle the FPS overlay (on | off | debug)",
    usage: "fps on | fps off | fps debug",
    tags: &["fps", "debug", "overlay", "hud", "perf"],
    applicable: always,
    grammar: None,
    synonyms: &[],
    complete: Some(complete_fps),
    execute: execute_fps,
};

fn complete_fps(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index: 0 } => prefix_filter(&["on", "off", "debug"], state.partial),
        _ => Vec::new(),
    }
}

fn execute_fps(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Subverb names are case-insensitive console-wide — see
    // `commands/mod.rs` § Casing.
    let mode = match args.positional(0).map(str::to_ascii_lowercase).as_deref() {
        Some("on") => FpsDisplayMode::Snapshot,
        Some("off") => FpsDisplayMode::Off,
        Some("debug") => FpsDisplayMode::Debug,
        _ => return ExecResult::err("usage: fps on | fps off | fps debug"),
    };
    eff.side_effect = Some(super::super::ConsoleSideEffect::SetFpsDisplay(mode));
    eff.close_console = true;
    ExecResult::ok_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::tests::fixtures::assert_exec_err_contains;
    use crate::application::console::ConsoleSideEffect;
    use crate::application::document::MindMapDocument;

    /// Run `fps` with `args`, returning the result alongside the two
    /// dispatcher-visible outputs: the side effect and the
    /// close-the-console flag.
    fn run_fps(args: &[&str]) -> (ExecResult, Option<ConsoleSideEffect>, bool) {
        let tokens: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let mut doc = MindMapDocument::new_blank(None);
        let mut eff = ConsoleEffects::new(&mut doc);
        let result = execute_fps(&Args::new(&tokens), &mut eff);
        (result, eff.side_effect.take(), eff.close_console)
    }

    /// Each subverb selects its own display mode, in any casing, and
    /// the console closes so the overlay it just enabled is visible.
    ///
    /// Fails when: two subverbs collapse onto one mode (a stub
    /// returning `Snapshot` for everything fails the `off` and
    /// `debug` rows), when the casing fold is dropped (the `ON` /
    /// `Debug` / `OFF` rows fall into the usage arm), or when
    /// `close_console` is left unset (the overlay lands behind the
    /// console it was requested from).
    ///
    /// The table is the control for its own distinctness: three rows
    /// asserting three different modes cannot all pass against a
    /// constant.
    #[test]
    fn test_fps_maps_each_subverb_to_its_display_mode() {
        let cases = [
            ("on", FpsDisplayMode::Snapshot),
            ("off", FpsDisplayMode::Off),
            ("debug", FpsDisplayMode::Debug),
            ("ON", FpsDisplayMode::Snapshot),
            ("OFF", FpsDisplayMode::Off),
            ("Debug", FpsDisplayMode::Debug),
        ];
        for (arg, expected) in cases {
            let (result, side, close) = run_fps(&[arg]);
            assert!(
                matches!(result, ExecResult::Ok(ref s) if s.is_empty()),
                "`fps {arg}` must succeed silently, got {result:?}"
            );
            match side {
                Some(ConsoleSideEffect::SetFpsDisplay(mode)) => assert_eq!(
                    mode, expected,
                    "`fps {arg}` must request {expected:?}, got {mode:?}"
                ),
                other => panic!("`fps {arg}` must request an FPS mode, got {other:?}"),
            }
            assert!(close, "`fps {arg}` must close the console");
        }
    }

    /// Anything that is not one of the three subverbs — including no
    /// subverb at all — reports the usage line, changes no mode, and
    /// leaves the console open so the user can retype.
    ///
    /// Fails when: a missing or unrecognized argument falls through
    /// to a default mode (the `side.is_none()` assertion goes), or
    /// when the error path also closes the console, hiding the
    /// message it just wrote. `"onn"` is in the table because a
    /// prefix match rather than an equality match would accept it.
    #[test]
    fn test_fps_without_a_recognized_subverb_reports_usage() {
        for args in [&[][..], &["onn"][..], &["sideways"][..], &[""][..]] {
            let (result, side, close) = run_fps(args);
            assert_exec_err_contains(result, "usage: fps on | fps off | fps debug");
            assert!(side.is_none(), "`fps {args:?}` must not change the display mode");
            assert!(!close, "`fps {args:?}` must leave the console open");
        }
    }
}
