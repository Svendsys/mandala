// SPDX-License-Identifier: MPL-2.0

//! `screenshot [<path>] [width= height=] [scale=] [frame=] [space=]
//! [overwrite=] [sidecar=]` — write what the canvas shows as a PNG.
//!
//! The human's door to the screenshot engine the IPC
//! `capture.screenshot` command uses (`crate::application::capture`,
//! native-only): the verb only reads the line, into a
//! [`ScreenshotLine`] side effect, and the console's executor hands it
//! to that engine, which plans, draws, writes and reports. The two
//! surfaces take the same shots with the same bounds and the same
//! write guards; they differ in three ways, each because a person is
//! typing:
//!
//! - **The path is the shell's.** A relative path is read against the
//!   working directory, as `save` and `open` read theirs, and with no
//!   path the PNG goes there as `mandala-<unix ms>.png`. The IPC
//!   default, a bounded per-session dir removed at exit, would hand a
//!   person a file that vanishes when they close the app.
//! - **The console is not in the picture.** The prompt that asked for
//!   the screenshot is not what it is of, so an open console is left
//!   out of the frame; every other overlay is drawn as the window
//!   draws it.
//! - **Words, not JSON.** `frame=fit` or `frame=x,y,w,h` rather than a
//!   `fit` flag and a `region` object; `on` / `off` for the switches.

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::descend;
use crate::application::console::spec::{
    free, free_words, kvs, Bare, Form, Grammar, Key, Slot, Vocabulary, Word,
};
use crate::application::console::{ConsoleEffects, ConsoleSideEffect, ExecResult};

/// Where the shot is framed, as typed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScreenshotFrame {
    /// The camera as it stands.
    View,
    /// The whole drawn document.
    Fit,
    /// The smallest aspect-matching view containing this rect, in
    /// canvas units — or in live-window pixels with `surface`.
    Region {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        surface: bool,
    },
}

/// One `screenshot` line, read: what the executor hands the engine.
/// Syntax only — whether the size fits the GPU, or the region can be
/// framed, is the engine's to say, in the words IPC hears too.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenshotLine {
    /// As typed; `None` for the working-directory default.
    pub path: Option<String>,
    pub size: Option<(u32, u32)>,
    pub scale: Option<f32>,
    pub frame: ScreenshotFrame,
    pub overwrite: bool,
    pub sidecar: bool,
}

const SWITCH: &[Word] = &[Word::bare("on"), Word::bare("off")];

const FRAMES: &[Word] = &[
    Word::new("view", "the camera as it stands (default)"),
    Word::new("fit", "the whole drawn document"),
];

const SPACES: &[Word] = &[
    Word::new("canvas", "region in canvas units (default)"),
    Word::new("surface", "region in window pixels"),
];

const KEYS: &[Key] = &[
    Key::new("width", "image width in window pixels (with height)", free("px")),
    Key::new("height", "image height in window pixels (with width)", free("px")),
    Key::new("scale", "image pixels per window pixel, 0.1 to 4", free("factor")),
    Key::new(
        "frame",
        "what to frame: view, fit, or a rect x,y,w,h",
        free_words("x,y,w,h", FRAMES),
    ),
    Key::new(
        "space",
        "the space frame=x,y,w,h is in",
        Vocabulary::Words(SPACES),
    ),
    Key::new(
        "overwrite",
        "replace an existing file (default off)",
        Vocabulary::Words(SWITCH),
    ),
    Key::new(
        "sidecar",
        "also write <path>.geometry.json mapping pixels to nodes (default on)",
        Vocabulary::Words(SWITCH),
    ),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "screenshot",
    subverb_sets: &[],
    key_sets: &[KEYS],
    bare: Some(Bare::new(
        "file",
        &[Form::slots(&[Slot::opt(free("path"))]).reading(&[
            "width",
            "height",
            "scale",
            "frame",
            "space",
            "overwrite",
            "sidecar",
        ])],
    )),
};

pub const COMMAND: Command = Command {
    name: "screenshot",
    aliases: &[],
    summary: "Write what the canvas shows to a PNG",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["capture", "png", "image", "export", "snapshot"],
    execute: execute_screenshot,
};

fn execute_screenshot(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // A level with no subverbs always descends to its bare form; the
    // path, if any, is that form's slot.
    let descent = descend(&GRAMMAR, args.tokens());
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    match read_line(args.positional(0), |name| kvs::value(&pairs, name)) {
        Ok(line) => {
            eff.side_effect = Some(ConsoleSideEffect::Screenshot(line));
            ExecResult::ok_empty()
        }
        Err(message) => ExecResult::err(message),
    }
}

/// The line's words, checked for syntax.
fn read_line<'a>(
    path: Option<&str>,
    value: impl Fn(&str) -> Option<&'a str>,
) -> Result<ScreenshotLine, String> {
    let side = |name: &str| -> Result<Option<u32>, String> {
        value(name)
            .map(|v| {
                v.parse::<u32>()
                    .map_err(|_| format!("screenshot: {name}='{v}' is not a whole number of pixels"))
            })
            .transpose()
    };
    let size = match (side("width")?, side("height")?) {
        (Some(w), Some(h)) => Some((w, h)),
        (None, None) => None,
        _ => return Err("screenshot: width= and height= are given together".into()),
    };
    let scale = value("scale")
        .map(|v| {
            v.parse::<f32>()
                .ok()
                .filter(|s| s.is_finite())
                .ok_or_else(|| format!("screenshot: scale='{v}' is not a number"))
        })
        .transpose()?;
    let switch = |name: &str, default: bool| -> Result<bool, String> {
        match value(name) {
            None => Ok(default),
            Some(v) if v.eq_ignore_ascii_case("on") => Ok(true),
            Some(v) if v.eq_ignore_ascii_case("off") => Ok(false),
            Some(v) => Err(format!("screenshot: {name}='{v}' must be on or off")),
        }
    };
    let surface = match value("space") {
        None => None,
        Some(v) if v.eq_ignore_ascii_case("canvas") => Some(false),
        Some(v) if v.eq_ignore_ascii_case("surface") => Some(true),
        Some(v) => return Err(format!("screenshot: space='{v}' must be canvas or surface")),
    };
    let frame = match value("frame") {
        None => ScreenshotFrame::View,
        Some(v) if v.eq_ignore_ascii_case("view") => ScreenshotFrame::View,
        Some(v) if v.eq_ignore_ascii_case("fit") => ScreenshotFrame::Fit,
        Some(v) => {
            let numbers: Vec<f32> = v.split(',').map_while(|n| n.trim().parse::<f32>().ok()).collect();
            let [x, y, w, h] = numbers[..] else {
                return Err(format!("screenshot: frame='{v}' must be view, fit, or x,y,w,h"));
            };
            ScreenshotFrame::Region {
                x,
                y,
                w,
                h,
                surface: surface.unwrap_or(false),
            }
        }
    };
    if surface.is_some() && !matches!(frame, ScreenshotFrame::Region { .. }) {
        return Err("screenshot: space= says which space frame=x,y,w,h is in; give it with one".into());
    }
    Ok(ScreenshotLine {
        path: path.map(str::to_string),
        size,
        scale,
        frame,
        overwrite: switch("overwrite", false)?,
        sidecar: switch("sidecar", true)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::tests::fixtures::assert_exec_err_contains;
    use crate::application::document::MindMapDocument;

    /// Run `screenshot` with `args`: the result, the side effect, and
    /// whether the verb took the document.
    fn run(args: &[&str]) -> (ExecResult, Option<ConsoleSideEffect>, bool) {
        let tokens: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let mut doc = MindMapDocument::new_blank(None);
        let mut eff = ConsoleEffects::new(&mut doc);
        let result = execute_screenshot(&Args::new(&tokens), &mut eff);
        let mutated = eff.document_mutated();
        (result, eff.side_effect.take(), mutated)
    }

    fn line(args: &[&str]) -> ScreenshotLine {
        match run(args) {
            (ExecResult::Ok(s), Some(ConsoleSideEffect::Screenshot(line)), false) if s.is_empty() => line,
            other => panic!("`screenshot {args:?}` must hand the engine a line silently, got {other:?}"),
        }
    }

    /// No arguments is the view, at the window's size, into the
    /// working-directory default, with a sidecar — and the verb never
    /// touches the document, so the line costs no scene rebuild.
    #[test]
    fn test_a_bare_screenshot_is_the_view_with_a_sidecar() {
        assert_eq!(
            line(&[]),
            ScreenshotLine {
                path: None,
                size: None,
                scale: None,
                frame: ScreenshotFrame::View,
                overwrite: false,
                sidecar: true,
            }
        );
    }

    /// Every key reaches the line as what it says, in any casing of its
    /// words.
    #[test]
    fn test_every_key_reaches_the_line() {
        assert_eq!(
            line(&[
                "shots/a.png",
                "width=1920",
                "height=1080",
                "scale=1.5",
                "frame=FIT",
                "overwrite=ON",
                "sidecar=off"
            ]),
            ScreenshotLine {
                path: Some("shots/a.png".into()),
                size: Some((1920, 1080)),
                scale: Some(1.5),
                frame: ScreenshotFrame::Fit,
                overwrite: true,
                sidecar: false,
            }
        );
        assert_eq!(
            line(&["frame=-10,20.5, 300,200"]).frame,
            ScreenshotFrame::Region {
                x: -10.0,
                y: 20.5,
                w: 300.0,
                h: 200.0,
                surface: false
            }
        );
        assert_eq!(
            line(&["frame=0,0,640,360", "space=surface"]).frame,
            ScreenshotFrame::Region {
                x: 0.0,
                y: 0.0,
                w: 640.0,
                h: 360.0,
                surface: true
            }
        );
    }

    /// A malformed line is refused with the reason, hands the engine
    /// nothing, and leaves the console open on the message.
    #[test]
    fn test_a_malformed_line_is_refused_with_its_reason() {
        for (args, reason) in [
            (&["width=10"][..], "given together"),
            (&["width=ten", "height=10"][..], "whole number"),
            (&["width=-1", "height=10"][..], "whole number"),
            (&["scale=big"][..], "not a number"),
            (&["scale=inf"][..], "not a number"),
            (&["frame=1,2,3"][..], "view, fit, or x,y,w,h"),
            (&["frame=1,2,3,4,5"][..], "view, fit, or x,y,w,h"),
            (&["frame=fit", "space=surface"][..], "give it with one"),
            (&["space=world", "frame=0,0,1,1"][..], "canvas or surface"),
            (&["overwrite=yes"][..], "on or off"),
            (&["a.png", "b.png"][..], "b.png"),
            (&["colour=red"][..], "colour"),
        ] {
            let (result, side, _) = run(args);
            assert!(side.is_none(), "`screenshot {args:?}` must not reach the engine");
            assert_exec_err_contains(result, reason);
        }
    }
}
