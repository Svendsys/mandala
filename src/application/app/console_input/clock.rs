// SPDX-License-Identifier: MPL-2.0

//! The executor's half of the `clock` verb: the line the verb read
//! (`console::commands::clock`), handed to the switch and the step the
//! IPC `clock` family calls (`app::heartbeat`), and what they did,
//! worded for the scrollback.

use crate::application::clock::{AppClock, ClockMode};
use crate::application::console::commands::clock::ClockLine;
use crate::application::console::ExecResult;

use super::super::input_context::InputHandlerContext;

/// Do what `line` asks of the clock, and say what happened. The flag
/// is whether the window owes a frame: true after a step that ran.
pub(super) fn take(ctx: &mut InputHandlerContext<'_>, line: ClockLine) -> (ExecResult, bool) {
    match line {
        ClockLine::Status => (ExecResult::ok_msg(status(ctx.clock)), false),
        ClockLine::Mode(mode) => match ctx.set_clock_mode(mode) {
            Ok(true) => (
                ExecResult::ok_msg(format!("clock: now {}", status(ctx.clock))),
                false,
            ),
            Ok(false) => (
                ExecResult::ok_msg(format!("clock: already {}", status(ctx.clock))),
                false,
            ),
            Err(refusal) => (ExecResult::err(format!("clock: {}", refusal.message())), false),
        },
        ClockLine::Step(request) => match ctx.step(request) {
            Ok(stepped) => (
                ExecResult::ok_msg(format!(
                    "clock: stepped {} frame{} to {} ms",
                    stepped.frames_run,
                    if stepped.frames_run == 1 { "" } else { "s" },
                    stepped.now_ms
                )),
                true,
            ),
            Err(refusal) => (
                ExecResult::err(format!("clock step: {}", refusal.message())),
                false,
            ),
        },
    }
}

/// The clock's mode and reading, in the scrollback's words.
fn status(clock: &AppClock) -> String {
    let mode = match clock.mode() {
        ClockMode::Real => "real",
        ClockMode::Virtual => "virtual (advances only by `clock step`)",
    };
    format!("{mode}, at {:.0} ms", clock.now_ms())
}
