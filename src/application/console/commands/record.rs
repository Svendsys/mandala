// SPDX-License-Identifier: MPL-2.0

//! `record start [dir=] [fps=] [max_frames=] [width= height=] [scale=]
//! [frame=] [space=]` and `record stop` — record a frame sequence of
//! what the canvas shows.
//!
//! The human's door to the recorder the IPC `capture.record_start` /
//! `capture.record_stop` commands use (`crate::application::capture::
//! record`, native-only): the verb only reads the line, into a
//! [`RecordLine`] side effect, and the console's executor hands it to
//! that engine, which plans, draws, writes and reports. As with
//! `screenshot`, the two surfaces differ only where a person is typing:
//! a relative `dir` is read against the working directory, no `dir`
//! means `mandala-recording-<unix ms>` there rather than the IPC
//! artifacts dir (which is removed when the app exits), and the console
//! is left out of the frames.
//!
//! The view keys are `screenshot`'s, read by the same code
//! ([`super::screenshot::read_view`]).

use super::screenshot::{read_view, ConsoleView, VIEW_KEYS};
use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::{free, kvs, usage, Form, Grammar, Key, Subverb};
use crate::application::console::{ConsoleEffects, ConsoleSideEffect, ExecResult};

/// One `record start` line, read: what the executor hands the engine.
/// Syntax only — whether `fps` and `max_frames` are in range, the
/// directory usable, or the region framable, is the engine's to say.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordStartLine {
    /// As typed; `None` for the working-directory default.
    pub dir: Option<String>,
    pub fps: Option<u32>,
    pub max_frames: Option<u32>,
    pub view: ConsoleView,
}

/// One `record` line, read.
#[derive(Debug, Clone, PartialEq)]
pub enum RecordLine {
    Start(RecordStartLine),
    Stop,
}

const RECORD_KEYS: &[Key] = &[
    Key::new(
        "dir",
        "directory for the frames, new or empty (default ./mandala-recording-<time>)",
        free("path"),
    ),
    Key::new("fps", "frames per second, 1 to 60 (default 30)", free("n")),
    Key::new(
        "max_frames",
        "stop by itself after this many frames, 1 to 3600 (default 600)",
        free("n"),
    ),
];

const START_KEYS: &[&str] = &[
    "dir",
    "fps",
    "max_frames",
    "width",
    "height",
    "scale",
    "frame",
    "space",
];

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("start", "recording", "start recording frames and a manifest")
        .taking(&[Form::opt(START_KEYS)]),
    Subverb::bare("stop", "recording", "stop recording and finish the manifest"),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "record",
    subverb_sets: &[SUBVERBS],
    key_sets: &[RECORD_KEYS, VIEW_KEYS],
    bare: None,
};

pub const COMMAND: Command = Command {
    name: "record",
    aliases: &[],
    summary: "Record the canvas as a frame sequence (start / stop)",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["capture", "video", "gif", "movie", "frames"],
    execute: execute_record,
};

fn execute_record(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = descend(&GRAMMAR, args.tokens());
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    let line = match descent.stop {
        Stop::Matched(subverb) if subverb.name == "start" => {
            match read_start(|name| kvs::value(&pairs, name)) {
                Ok(line) => RecordLine::Start(line),
                Err(message) => return ExecResult::err(message),
            }
        }
        Stop::Matched(_) => RecordLine::Stop,
        Stop::Bare => return ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
        _ => {
            return ExecResult::err(usage::unknown_subverb_message(
                descent.level,
                descent.typed.unwrap_or_default(),
            ))
        }
    };
    eff.side_effect = Some(ConsoleSideEffect::Record(line));
    ExecResult::ok_empty()
}

/// The `start` line's words, checked for syntax.
fn read_start<'a>(value: impl Fn(&str) -> Option<&'a str>) -> Result<RecordStartLine, String> {
    let count = |name: &str| -> Result<Option<u32>, String> {
        value(name)
            .map(|v| {
                v.parse::<u32>()
                    .map_err(|_| format!("record: {name}='{v}' is not a whole number"))
            })
            .transpose()
    };
    Ok(RecordStartLine {
        dir: value("dir").map(str::to_string),
        fps: count("fps")?,
        max_frames: count("max_frames")?,
        view: read_view("record", &value)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::commands::screenshot::ScreenshotFrame;
    use crate::application::console::tests::fixtures::assert_exec_err_contains;
    use crate::application::document::MindMapDocument;

    /// Run `record` with `args`: the result, the side effect, and
    /// whether the verb took the document.
    fn run(args: &[&str]) -> (ExecResult, Option<ConsoleSideEffect>, bool) {
        let tokens: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let mut doc = MindMapDocument::new_blank(None);
        let mut eff = ConsoleEffects::new(&mut doc);
        let result = execute_record(&Args::new(&tokens), &mut eff);
        let mutated = eff.document_mutated();
        (result, eff.side_effect.take(), mutated)
    }

    fn line(args: &[&str]) -> RecordLine {
        match run(args) {
            (ExecResult::Ok(s), Some(ConsoleSideEffect::Record(line)), false) if s.is_empty() => line,
            other => panic!("`record {args:?}` must hand the engine a line silently, got {other:?}"),
        }
    }

    /// A bare `start` is the defaults, left to the engine to fill in;
    /// `stop` is `stop`; neither touches the document.
    #[test]
    fn test_start_and_stop_reach_the_engine_as_what_they_say() {
        assert_eq!(
            line(&["start"]),
            RecordLine::Start(RecordStartLine {
                dir: None,
                fps: None,
                max_frames: None,
                view: ConsoleView {
                    size: None,
                    scale: None,
                    frame: ScreenshotFrame::View
                },
            })
        );
        assert_eq!(line(&["stop"]), RecordLine::Stop);
        assert_eq!(
            line(&["START"]),
            line(&["start"]),
            "subverbs are case-insensitive"
        );
    }

    #[test]
    fn test_every_start_key_reaches_the_line() {
        assert_eq!(
            line(&[
                "start",
                "dir=frames/zoom",
                "fps=12",
                "max_frames=240",
                "width=640",
                "height=360",
                "scale=0.5",
                "frame=fit"
            ]),
            RecordLine::Start(RecordStartLine {
                dir: Some("frames/zoom".into()),
                fps: Some(12),
                max_frames: Some(240),
                view: ConsoleView {
                    size: Some((640, 360)),
                    scale: Some(0.5),
                    frame: ScreenshotFrame::Fit
                },
            })
        );
    }

    /// A malformed line is refused with the reason and hands the engine
    /// nothing.
    #[test]
    fn test_a_malformed_line_is_refused_with_its_reason() {
        for (args, reason) in [
            (&[][..], "start"),
            (&["pause"][..], "pause"),
            (&["start", "fps=fast"][..], "not a whole number"),
            (&["start", "fps=-1"][..], "not a whole number"),
            (&["start", "max_frames=1.5"][..], "not a whole number"),
            (&["start", "width=10"][..], "given together"),
            (&["start", "frame=1,2,3"][..], "view, fit, or x,y,w,h"),
            (&["start", "space=surface"][..], "give it with one"),
            (&["start", "color=red"][..], "color"),
            (&["stop", "fps=30"][..], "fps"),
        ] {
            let (result, side, _) = run(args);
            assert!(side.is_none(), "`record {args:?}` must not reach the engine");
            assert_exec_err_contains(result, reason);
        }
    }
}
