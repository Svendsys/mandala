// SPDX-License-Identifier: MPL-2.0

//! `clock status`, `clock real`, `clock virtual` and `clock step
//! ms=<n>` / `clock step frames=<n>` — read, switch and step the app
//! clock (`crate::application::clock`).
//!
//! The human's door to what the IPC `clock` family drives: the verb
//! only reads the line, into a [`ClockLine`] side effect, and the
//! console's executor hands it to the same switch and the same step
//! `clock.set_mode` and `clock.step` call (`app::heartbeat`), with the
//! same bounds ([`StepRequest::of`]). A person stepping an animation
//! frame by frame and an agent scripting one meet the same clock.

use super::Command;
use crate::application::clock::{ClockMode, StepRequest};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::{free, kvs, usage, Form, Grammar, Key, Subverb};
use crate::application::console::{ConsoleEffects, ConsoleSideEffect, ExecResult};

/// One `clock` line, read: what the executor hands the clock. Syntax
/// and bounds only — whether a step can run (a real clock, a step
/// already running) is the clock's to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockLine {
    /// Report the mode and the reading.
    Status,
    /// Run the clock in this mode from now on.
    Mode(ClockMode),
    /// Advance a virtual clock this far.
    Step(StepRequest),
}

const STEP_KEYS: &[Key] = &[
    Key::new("ms", "virtual milliseconds to advance, 1 to 1600000", free("n")),
    Key::new("frames", "heartbeats to run, 16 ms apart, 1 to 100000", free("n")),
];

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("status", "clock", "show the clock's mode and reading"),
    Subverb::bare("real", "clock", "run at the wall clock's rate"),
    Subverb::bare(
        "virtual",
        "clock",
        "stop the clock; it advances only by `clock step`",
    ),
    Subverb::bare(
        "step",
        "clock",
        "advance a virtual clock, running every frame on the way",
    )
    .taking(&[Form::opt(&["ms", "frames"])]),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "clock",
    subverb_sets: &[SUBVERBS],
    key_sets: &[STEP_KEYS],
    bare: None,
};

pub const COMMAND: Command = Command {
    name: "clock",
    aliases: &[],
    summary: "Read, switch or step the app clock (status / real / virtual / step)",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["time", "virtual", "step", "frame", "animation"],
    execute: execute_clock,
};

fn execute_clock(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = descend(&GRAMMAR, args.tokens());
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    let line = match descent.stop {
        Stop::Matched(subverb) => match subverb.name {
            "status" => ClockLine::Status,
            "real" => ClockLine::Mode(ClockMode::Real),
            "virtual" => ClockLine::Mode(ClockMode::Virtual),
            _ => match read_step(|name| kvs::value(&pairs, name)) {
                Ok(request) => ClockLine::Step(request),
                Err(message) => return ExecResult::err(message),
            },
        },
        Stop::Bare => return ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
        _ => {
            return ExecResult::err(usage::unknown_subverb_message(
                descent.level,
                descent.typed.unwrap_or_default(),
            ))
        }
    };
    eff.side_effect = Some(ConsoleSideEffect::Clock(line));
    ExecResult::ok_empty()
}

/// A `step` line's `ms=` / `frames=`, checked: whole numbers, exactly
/// one of them, within the bounds IPC holds `clock.step` to.
fn read_step<'a>(value: impl Fn(&str) -> Option<&'a str>) -> Result<StepRequest, String> {
    let count = |name: &str| -> Result<Option<u64>, String> {
        value(name)
            .map(|v| {
                v.parse::<u64>()
                    .map_err(|_| format!("clock: {name}='{v}' is not a whole number"))
            })
            .transpose()
    };
    StepRequest::of(count("ms")?, count("frames")?).map_err(|(_, message)| format!("clock step: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::MindMapDocument;

    fn run(args: &[&str]) -> (ExecResult, Option<ConsoleSideEffect>, bool) {
        let tokens: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let mut doc = MindMapDocument::new_blank(None);
        let mut eff = ConsoleEffects::new(&mut doc);
        let result = execute_clock(&Args::new(&tokens), &mut eff);
        let mutated = eff.document_mutated();
        (result, eff.side_effect.take(), mutated)
    }

    fn line(args: &[&str]) -> ClockLine {
        match run(args) {
            (ExecResult::Ok(s), Some(ConsoleSideEffect::Clock(line)), false) if s.is_empty() => line,
            other => panic!("`clock {args:?}` must hand the clock a line silently, got {other:?}"),
        }
    }

    fn refusal(args: &[&str]) -> String {
        match run(args) {
            (ExecResult::Err(message), None, false) => message,
            other => panic!("`clock {args:?}` must be refused, got {other:?}"),
        }
    }

    #[test]
    fn test_each_subverb_reaches_the_clock_as_what_it_says() {
        assert_eq!(line(&["status"]), ClockLine::Status);
        assert_eq!(line(&["real"]), ClockLine::Mode(ClockMode::Real));
        assert_eq!(line(&["virtual"]), ClockLine::Mode(ClockMode::Virtual));
        assert_eq!(
            line(&["VIRTUAL"]),
            line(&["virtual"]),
            "subverbs are case-insensitive"
        );
        assert_eq!(line(&["step", "ms=500"]), ClockLine::Step(StepRequest::Ms(500)));
        assert_eq!(
            line(&["step", "frames=30"]),
            ClockLine::Step(StepRequest::Frames(30))
        );
    }

    /// The step's bounds are IPC's, by the same check, so a person and
    /// an agent are refused at the same numbers.
    #[test]
    fn test_a_step_is_held_to_the_bounds_ipc_holds_it_to() {
        assert!(refusal(&["step"]).contains("needs one of"));
        assert!(refusal(&["step", "ms=16", "frames=1"]).contains("not both"));
        assert!(refusal(&["step", "ms=0"]).contains("1..=1600000"));
        assert!(refusal(&["step", "frames=100001"]).contains("1..=100000"));
        assert!(refusal(&["step", "ms=-5"]).contains("not a whole number"));
        assert!(refusal(&["step", "ms=fast"]).contains("not a whole number"));
    }

    #[test]
    fn test_a_bare_or_unknown_clock_line_is_refused_with_its_usage() {
        assert!(refusal(&[]).contains("status | real | virtual | step"));
        assert!(refusal(&["pause"]).contains("unknown subverb 'pause'"));
        assert!(refusal(&["real", "ms=5"]).contains("unknown key 'ms'"));
    }
}
