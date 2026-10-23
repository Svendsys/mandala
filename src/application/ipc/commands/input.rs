// SPDX-License-Identifier: MPL-2.0

//! The `input` family: synthetic raw input below the Action funnel
//! (`format/ipc.md` §`input`).
//!
//! `act.action` applies an Action. These commands are the hand on
//! the mouse and the keyboard instead: a pointer move, a button, the
//! wheel, a key, a typed string, a finger. Each becomes the payload a
//! winit event carries ([`InputEvent`]), and the app delivers it to
//! the entry point that event reaches. So the click ladder, the drag
//! thresholds and state machines, double-click timing, the modal
//! steals and the keybind resolution behave exactly as under real
//! input, and no handler knows the difference.
//!
//! Two commands are sugar over the primitives and add no semantics
//! of their own: `input.drag` is a move, a press, a run of moves and
//! a release, paced one per frame so the throttled drag drains
//! between them; `input.click_node` is a move to a node's center and
//! a click. `input.pending` reads the injected-input queue's depth.
//!
//! Every handler validates the whole request before it touches the
//! app, so a refused request injects nothing.

use baumhard::util::grapheme_chad::{count_grapheme_clusters, split_graphemes_owned, take_graphemes};
use serde_json::{json, Value};

use crate::application::ipc::envelope::{code, Reply};
use crate::application::ipc::injected::{InputEvent, Pacing, Sample, INJECTED_QUEUE_CAP};
use crate::application::ipc::registry::{Call, CommandSpec, Family, ParamSpec, ParamType, ResultSpec};
use crate::application::ipc::space::{Point, Space};
use crate::application::ipc::Injected;
use crate::application::platform::input::{
    ElementState, Key, Modifiers, MouseButton, MouseScrollDelta, NamedKey, SmolStr, TouchPhase,
};

use super::act::parse_point;
use super::refusal::no_loaded_document;

/// The most graphemes one `input.text` carries: each is a press and a
/// release, and no command produces more samples than the queue holds.
const MAX_TEXT_GRAPHEMES: usize = INJECTED_QUEUE_CAP / 2;

/// The most intermediate moves one `input.drag` carries: the move to
/// `from`, the press and the release take the other three samples.
const MAX_DRAG_STEPS: u64 = (INJECTED_QUEUE_CAP - 3) as u64;

/// `input.drag`'s default number of intermediate moves.
const DEFAULT_DRAG_STEPS: u64 = 10;

const X: ParamSpec = ParamSpec::new("x", ParamType::Number, true, "Pointer x, in `space`.");
const Y: ParamSpec = ParamSpec::new("y", ParamType::Number, true, "Pointer y, in `space`.");
const AT_X: ParamSpec = ParamSpec::new(
    "x",
    ParamType::Number,
    false,
    "Move the pointer here first, in `space`. Sent with `y`; without them the event lands where the \
     pointer already is.",
);
const AT_Y: ParamSpec = ParamSpec::new("y", ParamType::Number, false, "Sent with `x`.");
const SPACE: ParamSpec = ParamSpec::new(
    "space",
    ParamType::String,
    false,
    "Coordinate space of the position: \"surface\" (default) or \"canvas\". A canvas position is \
     converted through the camera as it stands when the command starts applying.",
);
const MODS: ParamSpec = ParamSpec::new(
    "mods",
    ParamType::Array,
    false,
    "Modifiers held for this command's events: any of \"ctrl\", \"shift\", \"alt\", \"super\". \
     Omitted, the modifiers the OS reports stay in force; [] holds none.",
);
const BUTTON: ParamSpec = ParamSpec::new(
    "button",
    ParamType::String,
    false,
    "\"left\" (default), \"right\" or \"middle\".",
);

const APPLIED: ResultSpec = ResultSpec::new(
    "applied",
    ParamType::Boolean,
    "true when the events reached the handlers before the reply; false when they joined the \
     injected-input queue, which the heartbeat drains one sample per frame.",
);
const PENDING: ResultSpec = ResultSpec::new(
    "pending",
    ParamType::Integer,
    "Samples still queued after this command; 0 means everything sent so far has landed.",
);

pub(super) const FAMILY: Family = Family {
    name: "input",
    commands: &[
        CommandSpec {
            name: "input.mouse_move",
            summary: "Move the pointer, as a CursorMoved event: hover, drag-threshold and drag-update \
                      behavior follow.",
            params: &[X, Y, SPACE, MODS],
            result: &[APPLIED, PENDING],
            run: mouse_move,
        },
        CommandSpec {
            name: "input.mouse_button",
            summary: "Press, release or click a mouse button, as MouseInput events.",
            params: &[
                ParamSpec::new(
                    "button",
                    ParamType::String,
                    true,
                    "\"left\", \"right\" or \"middle\".",
                ),
                ParamSpec::new(
                    "state",
                    ParamType::String,
                    true,
                    "\"press\", \"release\" or \"click\" (a press, then a release).",
                ),
                AT_X,
                AT_Y,
                SPACE,
                MODS,
            ],
            result: &[APPLIED, PENDING],
            run: mouse_button,
        },
        CommandSpec {
            name: "input.wheel",
            summary: "Turn the mouse wheel, as a MouseWheel event: the bound WheelUp/WheelDown \
                      Action, or the console's scrollback while it is open.",
            params: &[
                ParamSpec::new(
                    "dy",
                    ParamType::Number,
                    true,
                    "Lines, not 0; positive scrolls up (WheelUp), negative down (WheelDown).",
                ),
                AT_X,
                AT_Y,
                SPACE,
                MODS,
            ],
            result: &[APPLIED, PENDING],
            run: wheel,
        },
        CommandSpec {
            name: "input.key",
            summary: "Press, release or tap a key, as KeyboardInput: the modal steals, the keybind \
                      table, macros and custom mutations resolve it as they do a keystroke.",
            params: &[
                ParamSpec::new(
                    "key",
                    ParamType::String,
                    true,
                    "The logical key: one typed character (\"a\", \"/\", \"é\"), or a named key as \
                     winit names it, case-exact (\"Enter\", \"Tab\", \"Escape\", \"ArrowLeft\", \
                     \"Backspace\", \"F2\"). Not a modifier key: hold those with `mods`.",
                ),
                ParamSpec::new(
                    "state",
                    ParamType::String,
                    false,
                    "\"tap\" (default: a press, then a release), \"press\" or \"release\".",
                ),
                MODS,
            ],
            result: &[APPLIED, PENDING],
            run: key,
        },
        CommandSpec {
            name: "input.text",
            summary: "Type a string, one key tap per grapheme cluster, into whatever has the keyboard: \
                      an open editor or console, or the keybind table when none is open.",
            params: &[ParamSpec::new(
                "text",
                ParamType::String,
                true,
                "What to type; at most 512 graphemes. A space, a newline and a tab arrive as the \
                 Space, Enter and Tab keys, as winit delivers them.",
            )],
            result: &[APPLIED, PENDING],
            run: text,
        },
        CommandSpec {
            name: "input.touch",
            summary: "One finger event, as a Touch event, into the touch-gesture recognizer.",
            params: &[
                ParamSpec::new(
                    "phase",
                    ParamType::String,
                    true,
                    "\"started\", \"moved\", \"ended\" or \"canceled\".",
                ),
                ParamSpec::new(
                    "pointer_id",
                    ParamType::Integer,
                    true,
                    "The finger's identifier, ≥ 0; the same value for one finger's whole stroke.",
                ),
                X,
                Y,
                SPACE,
            ],
            result: &[APPLIED, PENDING],
            run: touch,
        },
        CommandSpec {
            name: "input.drag",
            summary: "Drag with a mouse button: a move to `from`, a press, `steps` moves to `to` and a \
                      release, queued one per frame so the throttled drag drains between them.",
            params: &[
                ParamSpec::new("from_x", ParamType::Number, true, "Start x, in `space`."),
                ParamSpec::new("from_y", ParamType::Number, true, "Start y, in `space`."),
                ParamSpec::new("to_x", ParamType::Number, true, "End x, in `space`."),
                ParamSpec::new("to_y", ParamType::Number, true, "End y, in `space`."),
                SPACE,
                BUTTON,
                ParamSpec::new(
                    "steps",
                    ParamType::Integer,
                    false,
                    "Moves from `from` to `to`, evenly spaced, the last at `to`; 1 to 1021, default 10.",
                ),
                MODS,
            ],
            result: &[APPLIED, PENDING],
            run: drag,
        },
        CommandSpec {
            name: "input.click_node",
            summary: "Move the pointer to a node's (or one section's) center in the model and click \
                      there: sugar for input.mouse_move plus input.mouse_button click.",
            params: &[
                ParamSpec::new("node_id", ParamType::String, true, "The node to click."),
                ParamSpec::new(
                    "section",
                    ParamType::Integer,
                    false,
                    "Click this section's center instead of the node's.",
                ),
                BUTTON,
                MODS,
            ],
            result: &[
                APPLIED,
                PENDING,
                ResultSpec::new("x", ParamType::Number, "Surface x the click landed at."),
                ResultSpec::new("y", ParamType::Number, "Surface y the click landed at."),
            ],
            run: click_node,
        },
        CommandSpec {
            name: "input.pending",
            summary: "How many injected samples are still queued: poll it to 0 before reading what a \
                      queued gesture did.",
            params: &[],
            result: &[PENDING],
            run: pending,
        },
    ],
};

fn mouse_move(call: &mut Call<'_>) -> Reply {
    let point = match required_point(call) {
        Ok(point) => point,
        Err(reply) => return reply,
    };
    let mods = match parse_mods(call) {
        Ok(mods) => mods,
        Err(reply) => return reply,
    };
    inject(
        call,
        Pacing::Burst,
        vec![Sample::new(InputEvent::CursorMoved(point), mods)],
    )
}

fn mouse_button(call: &mut Call<'_>) -> Reply {
    let button = match parse_button(call) {
        Ok(button) => button,
        Err(reply) => return reply,
    };
    let states: &[ElementState] = match call.str_param("state") {
        Some("press") => &[ElementState::Pressed],
        Some("release") => &[ElementState::Released],
        Some("click") => &[ElementState::Pressed, ElementState::Released],
        other => {
            return call.invalid_params(
                "state",
                format!("`state` must be \"press\", \"release\" or \"click\", got {other:?}"),
            )
        }
    };
    let (at, mods) = match (parse_point(call, ("x", "y"), Space::Surface), parse_mods(call)) {
        (Ok(at), Ok(mods)) => (at, mods),
        (Err(reply), _) | (_, Err(reply)) => return reply,
    };
    let mut samples = move_to(at, mods);
    samples.extend(
        states
            .iter()
            .map(|&state| Sample::new(InputEvent::MouseButton { button, state }, mods)),
    );
    inject(call, Pacing::Burst, samples)
}

fn wheel(call: &mut Call<'_>) -> Reply {
    let Some(dy) = call.param("dy").and_then(Value::as_f64) else {
        return call.invalid_params("dy", "missing `dy`");
    };
    let dy = dy as f32;
    if !dy.is_finite() {
        return call.invalid_params("dy", "`dy` is out of range");
    }
    // A zero delta has no direction, and the wheel arm reads one
    // into it: `wheel_gesture` maps `lines <= 0` to WheelDown.
    if dy == 0.0 {
        return call.invalid_params(
            "dy",
            "`dy` is 0: a wheel turn has a direction, positive up or negative down",
        );
    }
    let (at, mods) = match (parse_point(call, ("x", "y"), Space::Surface), parse_mods(call)) {
        (Ok(at), Ok(mods)) => (at, mods),
        (Err(reply), _) | (_, Err(reply)) => return reply,
    };
    let mut samples = move_to(at, mods);
    samples.push(Sample::new(
        InputEvent::Wheel(MouseScrollDelta::LineDelta(0.0, dy)),
        mods,
    ));
    inject(call, Pacing::Burst, samples)
}

fn key(call: &mut Call<'_>) -> Reply {
    let Some(name) = call.str_param("key") else {
        return call.invalid_params("key", "missing `key`");
    };
    let key = match parse_key(name) {
        Ok(key) => key,
        Err(message) => return call.invalid_params("key", message),
    };
    let states: &[ElementState] = match call.str_param("state") {
        None | Some("tap") => &[ElementState::Pressed, ElementState::Released],
        Some("press") => &[ElementState::Pressed],
        Some("release") => &[ElementState::Released],
        Some(other) => {
            return call.invalid_params(
                "state",
                format!("`state` must be \"tap\", \"press\" or \"release\", got {other:?}"),
            )
        }
    };
    let mods = match parse_mods(call) {
        Ok(mods) => mods,
        Err(reply) => return reply,
    };
    let samples: Vec<Sample> = states
        .iter()
        .map(|&state| {
            Sample::new(
                InputEvent::Key {
                    key: key.clone(),
                    state,
                },
                mods,
            )
        })
        .collect();
    inject(call, Pacing::Burst, samples)
}

fn text(call: &mut Call<'_>) -> Reply {
    let Some(text) = call.str_param("text") else {
        return call.invalid_params("text", "missing `text`");
    };
    let keys = match typed_keys(text) {
        Ok(keys) => keys,
        Err(message) => return call.invalid_params("text", message),
    };
    let samples: Vec<Sample> = keys
        .into_iter()
        .flat_map(|key| {
            [ElementState::Pressed, ElementState::Released].map(|state| {
                Sample::new(
                    InputEvent::Key {
                        key: key.clone(),
                        state,
                    },
                    None,
                )
            })
        })
        .collect();
    inject(call, Pacing::Burst, samples)
}

fn touch(call: &mut Call<'_>) -> Reply {
    let phase = match call.str_param("phase") {
        Some("started") => TouchPhase::Started,
        Some("moved") => TouchPhase::Moved,
        Some("ended") => TouchPhase::Ended,
        Some("canceled") => TouchPhase::Cancelled,
        other => {
            return call.invalid_params(
                "phase",
                format!("`phase` must be \"started\", \"moved\", \"ended\" or \"canceled\", got {other:?}"),
            )
        }
    };
    let Some(pointer_id) = call.param("pointer_id").and_then(Value::as_u64) else {
        return call.invalid_params("pointer_id", "`pointer_id` must be an integer ≥ 0");
    };
    let point = match required_point(call) {
        Ok(point) => point,
        Err(reply) => return reply,
    };
    inject(
        call,
        Pacing::Burst,
        vec![Sample::new(
            InputEvent::Touch {
                phase,
                pointer_id,
                at: point,
            },
            None,
        )],
    )
}

fn drag(call: &mut Call<'_>) -> Reply {
    let from = match parse_point(call, ("from_x", "from_y"), Space::Surface) {
        Ok(Some(point)) => point,
        // Both are required, so the registry has already refused a
        // request without them.
        Ok(None) => return call.invalid_params("from_x", "missing `from_x` and `from_y`"),
        Err(reply) => return reply,
    };
    let to = match parse_point(call, ("to_x", "to_y"), Space::Surface) {
        Ok(Some(point)) => point,
        Ok(None) => return call.invalid_params("to_x", "missing `to_x` and `to_y`"),
        Err(reply) => return reply,
    };
    let button = match parse_button(call) {
        Ok(button) => button,
        Err(reply) => return reply,
    };
    let steps = match call.param("steps") {
        None => DEFAULT_DRAG_STEPS,
        Some(value) => match value.as_u64() {
            Some(steps) if (1..=MAX_DRAG_STEPS).contains(&steps) => steps,
            _ => {
                return call.invalid_params(
                    "steps",
                    format!("`steps` must be an integer from 1 to {MAX_DRAG_STEPS}, got {value}"),
                )
            }
        },
    };
    let mods = match parse_mods(call) {
        Ok(mods) => mods,
        Err(reply) => return reply,
    };
    inject(
        call,
        Pacing::PerFrame,
        drag_samples(from, to, steps, button, mods),
    )
}

fn click_node(call: &mut Call<'_>) -> Reply {
    let Some(node_id) = call.str_param("node_id") else {
        return call.invalid_params("node_id", "missing `node_id`");
    };
    let section = match call.param("section") {
        None => None,
        Some(value) => match value.as_u64().and_then(|i| usize::try_from(i).ok()) {
            Some(index) => Some(index),
            None => {
                return call.invalid_params("section", format!("`section` must be an index ≥ 0, got {value}"))
            }
        },
    };
    let (button, mods) = match (parse_button(call), parse_mods(call)) {
        (Ok(button), Ok(mods)) => (button, mods),
        (Err(reply), _) | (_, Err(reply)) => return reply,
    };
    let aim = {
        let Some(app) = call.observe() else {
            return call.no_document();
        };
        // The aim is checked against what is drawn now. Behind queued
        // samples the click would land later, after they may have
        // moved the camera, and the reply could not say where.
        let pending = app.injected_pending();
        if pending > 0 {
            return call.error(
                code::BUSY,
                format!(
                    "{pending} injected samples are still queued, and `input.click_node` aims at what is \
                     drawn now; poll `input.pending` to 0 and resend"
                ),
                Some(json!({ "pending": pending })),
            );
        }
        let Some(view) = app.scene() else {
            return no_loaded_document(call);
        };
        match node_target(view.document, node_id, section, view.camera.zoom) {
            Ok(canvas) => {
                let (x, y) = app.surface_point(Point {
                    x: canvas.x,
                    y: canvas.y,
                    space: Space::Canvas,
                });
                let (width, height) = view.camera.viewport_size;
                if !(0.0..f64::from(width)).contains(&x) || !(0.0..f64::from(height)).contains(&y) {
                    return call.error(
                        code::NOT_FOUND,
                        format!(
                            "node '{node_id}' is not on screen: its center lands at surface ({x:.1}, {y:.1}), \
                             outside the {width}×{height} surface; bring it into view first (act.action \
                             with a pan or zoom Action)"
                        ),
                        None,
                    );
                }
                (x, y)
            }
            Err(Aim::Missing) => {
                return call.error(
                    code::NOT_FOUND,
                    format!("no node '{node_id}' in the document"),
                    None,
                )
            }
            Err(Aim::Folded) => {
                return call.error(
                    code::NOT_FOUND,
                    format!(
                        "node '{node_id}' is hidden by a folded ancestor: nothing of it is drawn to click"
                    ),
                    None,
                )
            }
            Err(Aim::OutsideZoomWindow) => {
                return call.error(
                    code::NOT_FOUND,
                    format!(
                        "node '{node_id}' is not drawn at zoom {}: it is outside its \
                         min/max_zoom_to_render window; zoom into the window first",
                        view.camera.zoom
                    ),
                    None,
                )
            }
            Err(Aim::NoSection(count)) => {
                return call.invalid_params(
                    "section",
                    format!("node '{node_id}' has {count} section(s); `section` is an index below that"),
                )
            }
        }
    };
    let (x, y) = aim;
    // Surface positions, exactly the ones the reply reports.
    let at = Point {
        x: x as f32,
        y: y as f32,
        space: Space::Surface,
    };
    let mut samples = vec![Sample::new(InputEvent::CursorMoved(at), mods)];
    samples.extend(
        [ElementState::Pressed, ElementState::Released]
            .map(|state| Sample::new(InputEvent::MouseButton { button, state }, mods)),
    );
    match deliver(call, Pacing::Burst, samples) {
        Ok(injected) => call.ok(json!({
            "applied": injected.applied,
            "pending": injected.pending,
            "x": x,
            "y": y,
        })),
        Err(reply) => reply,
    }
}

fn pending(call: &mut Call<'_>) -> Reply {
    let Some(app) = call.observe() else {
        return call.no_document();
    };
    let pending = app.injected_pending();
    call.ok(json!({ "pending": pending }))
}

/// Hand `samples` to the app and reply with what it did with them.
fn inject(call: &mut Call<'_>, pacing: Pacing, samples: Vec<Sample>) -> Reply {
    match deliver(call, pacing, samples) {
        Ok(injected) => call.ok(json!({ "applied": injected.applied, "pending": injected.pending })),
        Err(reply) => reply,
    }
}

/// Hand `samples` to the app: the one path every injecting command
/// shares. Their positions are resolved app-side, when the command
/// starts applying (`ipc::injected`'s module doc).
fn deliver(call: &mut Call<'_>, pacing: Pacing, samples: Vec<Sample>) -> Result<Injected, Reply> {
    let Some(app) = call.drive() else {
        return Err(call.no_document());
    };
    let count = samples.len();
    app.inject(samples, pacing).map_err(|full| {
        call.error(
            code::OVERLOADED,
            format!(
                "the injected-input queue holds {} of its {INJECTED_QUEUE_CAP} samples and cannot take \
                 this command's {count}; nothing was queued. Resend once `input.pending` has dropped",
                full.pending
            ),
            Some(json!({ "pending": full.pending, "cap": INJECTED_QUEUE_CAP })),
        )
    })
}

/// A required position: `x` and `y` in `space`, surface by default.
fn required_point(call: &Call<'_>) -> Result<Point, Reply> {
    match parse_point(call, ("x", "y"), Space::Surface)? {
        Some(point) => Ok(point),
        // `x` and `y` are required, so the registry has already
        // refused a request without them.
        None => Err(call.invalid_params("x", "missing `x` and `y`")),
    }
}

/// The move that stands the pointer at `at` before the event that
/// follows it, or nothing when no position was sent.
fn move_to(at: Option<Point>, mods: Option<Modifiers>) -> Vec<Sample> {
    at.map(|point| Sample::new(InputEvent::CursorMoved(point), mods))
        .into_iter()
        .collect()
}

/// The `button` parameter; `left` when omitted.
fn parse_button(call: &Call<'_>) -> Result<MouseButton, Reply> {
    match call.str_param("button") {
        None | Some("left") => Ok(MouseButton::Left),
        Some("right") => Ok(MouseButton::Right),
        Some("middle") => Ok(MouseButton::Middle),
        Some(other) => Err(call.invalid_params(
            "button",
            format!("`button` must be \"left\", \"right\" or \"middle\", got {other:?}"),
        )),
    }
}

/// The `mods` parameter: `None` when omitted, so the modifiers the OS
/// reports stay in force.
fn parse_mods(call: &Call<'_>) -> Result<Option<Modifiers>, Reply> {
    let Some(words) = call.param("mods").and_then(Value::as_array) else {
        return Ok(None);
    };
    let mut mods = Modifiers::empty();
    for word in words {
        mods |= match word.as_str() {
            Some("ctrl") => Modifiers::CONTROL,
            Some("shift") => Modifiers::SHIFT,
            Some("alt") => Modifiers::ALT,
            Some("super") => Modifiers::SUPER,
            _ => {
                return Err(call.invalid_params(
                    "mods",
                    format!("each of `mods` is \"ctrl\", \"shift\", \"alt\" or \"super\", got {word}"),
                ))
            }
        };
    }
    Ok(Some(mods))
}

/// `input.key`'s `key`: one typed character, delivered as winit
/// delivers that keystroke ([`typed_key`]), or a named key in
/// winit's own spelling.
fn parse_key(name: &str) -> Result<Key, String> {
    if count_grapheme_clusters(name) == 1 {
        return typed_key(name).ok_or_else(|| {
            format!(
                "{name:?} is a control character; send the named key instead (\"Backspace\", \"Delete\", …)"
            )
        });
    }
    let named = serde_json::from_value::<NamedKey>(Value::String(name.to_owned())).map_err(|_| {
        format!(
            "{name:?} is neither one character nor a key name; named keys are spelled as winit names \
             them, case-exact: \"Enter\", \"Tab\", \"Escape\", \"Backspace\", \"ArrowUp\", \"F2\""
        )
    })?;
    if is_modifier_key(named) {
        return Err(format!(
            "{name:?} is a modifier key: winit reports a modifier through a separate modifier-state \
             event, not this key, so pressing it alone would change nothing a later event reads. \
             Hold it with `mods` on the event it modifies instead (`\"mods\": [\"shift\"]`)"
        ));
    }
    Ok(Key::Named(named))
}

/// Whether `key` is one of the keys winit folds into its modifier
/// state (`ModifiersChanged`) rather than delivering as a keystroke
/// the app acts on.
fn is_modifier_key(key: NamedKey) -> bool {
    matches!(
        key,
        NamedKey::Shift
            | NamedKey::Control
            | NamedKey::Alt
            | NamedKey::AltGraph
            | NamedKey::Super
            | NamedKey::Meta
            | NamedKey::Hyper
    )
}

/// The keys typing `text` presses, one per grapheme cluster.
fn typed_keys(text: &str) -> Result<Vec<Key>, String> {
    if text.is_empty() {
        return Err("`text` is empty: there is nothing to type".to_owned());
    }
    let (typed, cut) = take_graphemes(text, MAX_TEXT_GRAPHEMES);
    if cut {
        return Err(format!(
            "`text` is longer than {MAX_TEXT_GRAPHEMES} graphemes; send it in pieces"
        ));
    }
    split_graphemes_owned(typed)
        .iter()
        .enumerate()
        .map(|(index, grapheme)| {
            typed_key(grapheme).ok_or_else(|| {
                format!(
                    "grapheme {index} of `text` is a control character ({grapheme:?}), which no key types"
                )
            })
        })
        .collect()
}

/// The logical key winit reports for typing `grapheme`: the space
/// bar, Enter and Tab arrive as named keys, everything printable as
/// the character itself. `None` for a control character no key
/// types.
fn typed_key(grapheme: &str) -> Option<Key> {
    match grapheme {
        " " => Some(Key::Named(NamedKey::Space)),
        "\n" | "\r\n" | "\r" => Some(Key::Named(NamedKey::Enter)),
        "\t" => Some(Key::Named(NamedKey::Tab)),
        g if g.chars().any(char::is_control) => None,
        g => Some(Key::Character(SmolStr::new(g))),
    }
}

/// A drag's samples: a move to `from`, the press, `steps` moves
/// evenly spaced up to and ending exactly on `to`, and the release.
/// Every one carries `mods`, so a modifier read at any point of the
/// gesture — the rubber band's Shift at the threshold cross — sees
/// them.
///
/// Interpolated in the space the drag was sent in: the transform to
/// the surface is affine for the one camera a command resolves
/// through, so the steps stay evenly spaced on screen either way.
fn drag_samples(
    from: Point,
    to: Point,
    steps: u64,
    button: MouseButton,
    mods: Option<Modifiers>,
) -> Vec<Sample> {
    let at = |(x, y): (f32, f32)| {
        Sample::new(
            InputEvent::CursorMoved(Point {
                x,
                y,
                space: from.space,
            }),
            mods,
        )
    };
    let mut samples = vec![
        at((from.x, from.y)),
        Sample::new(
            InputEvent::MouseButton {
                button,
                state: ElementState::Pressed,
            },
            mods,
        ),
    ];
    for step in 1..steps {
        let t = step as f32 / steps as f32;
        samples.push(at((from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t)));
    }
    // The last move is `to` itself, not `from` plus a rounded fraction.
    samples.push(at((to.x, to.y)));
    samples.push(Sample::new(
        InputEvent::MouseButton {
            button,
            state: ElementState::Released,
        },
        mods,
    ));
    samples
}

/// Why `input.click_node` has nowhere to click.
#[derive(Debug, PartialEq)]
enum Aim {
    Missing,
    Folded,
    /// The camera's zoom is outside the node's `min/max_zoom_to_render`
    /// window, so the scene does not draw it.
    OutsideZoomWindow,
    /// The node has this many sections, and `section` is not below it.
    NoSection(usize),
}

/// The canvas point `input.click_node` aims at: the center of the
/// node's model AABB, or of one section's, by the tree builder's own
/// section rule. A node the scene does not draw at `zoom` — hidden
/// by a fold, or outside its zoom window — has nothing to click.
fn node_target(
    document: &crate::application::document::MindMapDocument,
    node_id: &str,
    section: Option<usize>,
    zoom: f32,
) -> Result<glam::Vec2, Aim> {
    let map = &document.mindmap;
    let node = map.nodes.get(node_id).ok_or(Aim::Missing)?;
    if map.is_hidden_by_fold(node) {
        return Err(Aim::Folded);
    }
    if !node.zoom_window().contains(zoom) {
        return Err(Aim::OutsideZoomWindow);
    }
    let Some(index) = section else {
        return Ok(node.center_vec2());
    };
    let section = node
        .sections
        .get(index)
        .ok_or(Aim::NoSection(node.sections.len()))?;
    let ((x, y), (width, height)) = baumhard::mindmap::tree_builder::section_aabb(
        section,
        node.position.x as f32,
        node.position.y as f32,
        node.size.width as f32,
        node.size.height as f32,
    );
    Ok(glam::Vec2::new(x + width / 2.0, y + height / 2.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::gfx_structs::camera::Camera2D;
    use baumhard::mindmap::tree_builder::MindMapTree;
    use glam::Vec2;

    use crate::application::console::LineOutcome;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::MindMapDocument;
    use crate::application::ipc::injected::{resolve_command, Surface};
    use crate::application::ipc::injected::{Delivery, InjectedQueue, Overloaded};
    use crate::application::ipc::registry::tests::{run_attached, run_detached};
    use crate::application::ipc::{CommandTarget, SceneView, ShellState};
    use crate::application::keybinds::Action;
    use crate::application::macros::MacroStep;
    use crate::application::scene_host::AppScene;

    /// The app behind the `CommandTarget` seam, as far as the `input`
    /// family reaches it: a camera to convert canvas positions, the
    /// real injected-input queue to route through, and a record of
    /// every sample that would have reached a handler. The handlers
    /// themselves take a live renderer (TEST_CONVENTIONS §T8); what
    /// they do with a sample is theirs to test, and that they receive
    /// it unchanged is `app/ipc_target.rs`'s one match.
    struct InputApp {
        doc: Option<MindMapDocument>,
        tree: Option<MindMapTree>,
        scene: AppScene,
        camera: Camera2D,
        queue: InjectedQueue,
        applied: Vec<Sample<Surface>>,
    }

    impl InputApp {
        /// The fixture, projected, with the camera centered on its
        /// first node at zoom 2 so canvas and surface differ.
        fn with_fixture() -> Self {
            let doc = load_test_doc();
            let (tree, scene) = crate::application::app::project_scene_for_tests(&doc, 2.0);
            let mut camera = Camera2D::new(1280, 720);
            camera.zoom = 2.0;
            camera.position = doc.mindmap.nodes[&first_node(&doc)].center_vec2();
            InputApp {
                doc: Some(doc),
                tree: Some(tree),
                scene,
                camera,
                queue: InjectedQueue::default(),
                applied: Vec::new(),
            }
        }
    }

    fn first_node(doc: &MindMapDocument) -> String {
        doc.mindmap.nodes.keys().min().cloned().expect("fixture nodes")
    }

    impl CommandTarget for InputApp {
        fn dispatch_action(&mut self, _: Action, _: Option<Point>) -> bool {
            panic!("raw input must not dispatch an Action itself");
        }
        fn run_console_line(&mut self, _: &str) -> Option<LineOutcome> {
            panic!("raw input must not run a console line");
        }
        fn has_macro(&self, _: &str) -> bool {
            panic!("raw input must not look up a macro");
        }
        fn run_macro(&mut self, _: &str) -> bool {
            panic!("raw input must not run a macro");
        }
        fn run_macro_steps(&mut self, _: &[MacroStep]) -> bool {
            panic!("raw input must not run macro steps");
        }
        fn changed_view(&self) -> bool {
            !self.applied.is_empty()
        }
        fn document(&self) -> Option<&MindMapDocument> {
            self.doc.as_ref()
        }
        fn animation_pause(&self) -> Option<u64> {
            None
        }
        fn shell_state(&self, _: ShellState) -> Value {
            panic!("raw input must not read the shell state");
        }
        fn scene(&self) -> Option<SceneView<'_>> {
            Some(SceneView {
                document: self.doc.as_ref()?,
                mindmap_tree: self.tree.as_ref(),
                app_scene: &self.scene,
                camera: &self.camera,
            })
        }
        fn hit_test(&mut self, _: Point) -> Option<Value> {
            panic!("raw input must not probe the hit test");
        }
        fn surface_point(&self, point: Point) -> (f64, f64) {
            through(&self.camera, point)
        }
        fn inject(&mut self, samples: Vec<Sample>, pacing: Pacing) -> Result<Injected, Overloaded> {
            let applied = match self.queue.admit(samples, pacing)? {
                Delivery::Now(samples) => {
                    let camera = &self.camera;
                    self.applied
                        .extend(resolve_command(samples, |point| through(camera, point)));
                    true
                }
                Delivery::Queued => false,
            };
            Ok(Injected {
                applied,
                pending: self.queue.len(),
            })
        }
        fn injected_pending(&self) -> usize {
            self.queue.len()
        }
        fn capture(
            &mut self,
            _: &crate::application::capture::plan::CaptureRequest,
            _: crate::application::capture::Destination,
            _: bool,
        ) -> Option<Result<crate::application::ipc::Screenshot, crate::application::capture::CaptureError>>
        {
            panic!("an input command must not take a screenshot");
        }
    }

    /// Where `point` lands on the surface through `camera`: the
    /// app-side resolution, with the stand-in's camera.
    fn through(camera: &Camera2D, point: Point) -> Surface {
        let surface = match point.space {
            Space::Surface => Vec2::new(point.x, point.y),
            Space::Canvas => camera.canvas_to_screen(Vec2::new(point.x, point.y)),
        };
        (f64::from(surface.x), f64::from(surface.y))
    }

    fn moved(x: f64, y: f64, mods: Option<Modifiers>) -> Sample<Surface> {
        Sample::new(InputEvent::CursorMoved((x, y)), mods)
    }

    fn button(button: MouseButton, state: ElementState, mods: Option<Modifiers>) -> Sample<Surface> {
        Sample::new(InputEvent::MouseButton { button, state }, mods)
    }

    fn key_sample(key: Key, state: ElementState, mods: Option<Modifiers>) -> Sample<Surface> {
        Sample::new(InputEvent::Key { key, state }, mods)
    }

    fn character(c: &str) -> Key {
        Key::Character(SmolStr::new(c))
    }

    const EVERY_COMMAND: [&str; 9] = [
        r#"{"id":1,"cmd":"input.mouse_move","x":1,"y":2}"#,
        r#"{"id":1,"cmd":"input.mouse_button","button":"left","state":"click"}"#,
        r#"{"id":1,"cmd":"input.wheel","dy":1}"#,
        r#"{"id":1,"cmd":"input.key","key":"a"}"#,
        r#"{"id":1,"cmd":"input.text","text":"hi"}"#,
        r#"{"id":1,"cmd":"input.touch","phase":"started","pointer_id":0,"x":1,"y":2}"#,
        r#"{"id":1,"cmd":"input.drag","from_x":0,"from_y":0,"to_x":10,"to_y":0}"#,
        r#"{"id":1,"cmd":"input.click_node","node_id":"0"}"#,
        r#"{"id":1,"cmd":"input.pending"}"#,
    ];

    /// Every command needs the app, and says so before it exists
    /// without asking for a frame.
    #[test]
    fn test_every_input_command_answers_no_document_before_the_app_exists() {
        for frame in EVERY_COMMAND {
            let (reply, repaint) = run_detached(frame);
            assert_eq!(reply["error"]["code"], json!("no_document"), "{frame}");
            assert!(!repaint, "{frame}");
        }
    }

    /// Every refusal names its parameter and happens before the app
    /// is reached, so it injects nothing.
    #[test]
    fn test_malformed_input_is_refused_naming_the_parameter_and_injects_nothing() {
        let mut app = InputApp::with_fixture();
        for (frame, param) in [
            (r#"{"id":1,"cmd":"input.mouse_move","x":1}"#, "y"),
            (
                r#"{"id":1,"cmd":"input.mouse_move","x":1,"y":2,"space":"screen"}"#,
                "space",
            ),
            (r#"{"id":1,"cmd":"input.mouse_move","x":1e300,"y":2}"#, "x"),
            (
                r#"{"id":1,"cmd":"input.mouse_move","x":1,"y":2,"mods":["control"]}"#,
                "mods",
            ),
            (
                r#"{"id":1,"cmd":"input.mouse_move","x":1,"y":2,"mods":[1]}"#,
                "mods",
            ),
            (
                r#"{"id":1,"cmd":"input.mouse_button","button":"left","state":"double"}"#,
                "state",
            ),
            (
                r#"{"id":1,"cmd":"input.mouse_button","button":"back","state":"click"}"#,
                "button",
            ),
            (
                r#"{"id":1,"cmd":"input.mouse_button","button":"left","state":"click","x":3}"#,
                "y",
            ),
            (
                r#"{"id":1,"cmd":"input.mouse_button","button":"left","state":"click","space":"canvas"}"#,
                "space",
            ),
            (r#"{"id":1,"cmd":"input.wheel","dy":1e300}"#, "dy"),
            (r#"{"id":1,"cmd":"input.wheel","dy":0}"#, "dy"),
            (r#"{"id":1,"cmd":"input.wheel","dy":-0.0}"#, "dy"),
            (r#"{"id":1,"cmd":"input.key","key":"Shift"}"#, "key"),
            (
                r#"{"id":1,"cmd":"input.key","key":"Control","state":"press"}"#,
                "key",
            ),
            (
                r#"{"id":1,"cmd":"input.touch","phase":"cancelled","pointer_id":0,"x":1,"y":2}"#,
                "phase",
            ),
            (r#"{"id":1,"cmd":"input.key","key":"enter"}"#, "key"),
            (r#"{"id":1,"cmd":"input.key","key":"\u0008"}"#, "key"),
            (r#"{"id":1,"cmd":"input.key","key":""}"#, "key"),
            (r#"{"id":1,"cmd":"input.key","key":"a","state":"hold"}"#, "state"),
            (r#"{"id":1,"cmd":"input.text","text":""}"#, "text"),
            (r#"{"id":1,"cmd":"input.text","text":"a\u0007b"}"#, "text"),
            (
                r#"{"id":1,"cmd":"input.touch","phase":"began","pointer_id":0,"x":1,"y":2}"#,
                "phase",
            ),
            (
                r#"{"id":1,"cmd":"input.touch","phase":"started","pointer_id":-1,"x":1,"y":2}"#,
                "pointer_id",
            ),
            (
                r#"{"id":1,"cmd":"input.drag","from_x":0,"from_y":0,"to_x":1,"to_y":1,"steps":0}"#,
                "steps",
            ),
            (
                r#"{"id":1,"cmd":"input.drag","from_x":0,"from_y":0,"to_x":1,"to_y":1,"steps":1022}"#,
                "steps",
            ),
            (
                r#"{"id":1,"cmd":"input.drag","from_x":0,"from_y":0,"to_x":1,"to_y":1,"button":"x"}"#,
                "button",
            ),
            (
                r#"{"id":1,"cmd":"input.click_node","node_id":"0","section":-1}"#,
                "section",
            ),
            (
                r#"{"id":1,"cmd":"input.click_node","node_id":"0","mods":["hyper"]}"#,
                "mods",
            ),
        ] {
            let (reply, repaint) = run_attached(frame, &mut app);
            assert_eq!(
                reply["error"]["code"],
                json!("invalid_params"),
                "{frame}: {reply}"
            );
            assert_eq!(reply["error"]["data"]["param"], json!(param), "{frame}: {reply}");
            assert!(!repaint, "{frame}");
        }
        assert!(
            app.applied.is_empty() && app.queue.is_empty(),
            "nothing reached the app"
        );
    }

    #[test]
    fn test_a_text_longer_than_half_the_queue_is_refused() {
        let mut app = InputApp::with_fixture();
        let fits = "a".repeat(MAX_TEXT_GRAPHEMES);
        let (reply, _) = run_attached(
            &json!({"id":1,"cmd":"input.text","text":fits}).to_string(),
            &mut app,
        );
        assert_eq!(reply["result"]["applied"], json!(true), "{reply}");
        assert_eq!(app.applied.len(), 2 * MAX_TEXT_GRAPHEMES);
        let over = "👋".repeat(MAX_TEXT_GRAPHEMES + 1);
        let (reply, _) = run_attached(
            &json!({"id":1,"cmd":"input.text","text":over}).to_string(),
            &mut app,
        );
        assert_eq!(reply["error"]["data"]["param"], json!("text"), "{reply}");
    }

    /// A move lands where it was sent: surface positions as given,
    /// canvas positions through the camera, with the modifiers held.
    #[test]
    fn test_mouse_move_lands_in_surface_pixels_from_either_space() {
        let mut app = InputApp::with_fixture();
        let (reply, repaint) = run_attached(
            r#"{"id":1,"cmd":"input.mouse_move","x":12.5,"y":40,"mods":["shift","ctrl"]}"#,
            &mut app,
        );
        assert_eq!(reply["result"], json!({"applied": true, "pending": 0}));
        assert!(repaint);
        let center = app.camera.position;
        run_attached(
            &json!({"id":2,"cmd":"input.mouse_move","x":center.x,"y":center.y,"space":"canvas"}).to_string(),
            &mut app,
        );
        assert_eq!(
            app.applied,
            vec![
                moved(12.5, 40.0, Some(Modifiers::SHIFT | Modifiers::CONTROL)),
                moved(640.0, 360.0, None),
            ],
            "the camera's center lands mid-surface"
        );
    }

    #[test]
    fn test_mouse_button_click_at_a_position_is_a_move_a_press_and_a_release() {
        let mut app = InputApp::with_fixture();
        run_attached(
            r#"{"id":1,"cmd":"input.mouse_button","button":"right","state":"click","x":5,"y":6,"mods":[]}"#,
            &mut app,
        );
        let none = Some(Modifiers::empty());
        assert_eq!(
            app.applied,
            vec![
                moved(5.0, 6.0, none),
                button(MouseButton::Right, ElementState::Pressed, none),
                button(MouseButton::Right, ElementState::Released, none),
            ]
        );
        app.applied.clear();
        run_attached(
            r#"{"id":2,"cmd":"input.mouse_button","button":"middle","state":"press"}"#,
            &mut app,
        );
        run_attached(
            r#"{"id":3,"cmd":"input.mouse_button","button":"middle","state":"release"}"#,
            &mut app,
        );
        assert_eq!(
            app.applied,
            vec![
                button(MouseButton::Middle, ElementState::Pressed, None),
                button(MouseButton::Middle, ElementState::Released, None),
            ],
            "press and release alone stand where the pointer already is"
        );
    }

    /// The wheel's `dy` is lines, the vertical component `wheel_lines`
    /// reads (its sign picks WheelUp / WheelDown, pinned in
    /// `app/tests.rs`); a position moves the pointer first so a zoom
    /// focuses there.
    #[test]
    fn test_wheel_is_a_line_delta_after_an_optional_move() {
        let mut app = InputApp::with_fixture();
        run_attached(r#"{"id":1,"cmd":"input.wheel","dy":-2,"x":100,"y":50}"#, &mut app);
        assert_eq!(
            app.applied,
            vec![
                moved(100.0, 50.0, None),
                Sample::new(InputEvent::Wheel(MouseScrollDelta::LineDelta(0.0, -2.0)), None),
            ]
        );
    }

    #[test]
    fn test_key_taps_by_default_and_presses_or_releases_on_request() {
        let mut app = InputApp::with_fixture();
        run_attached(
            r#"{"id":1,"cmd":"input.key","key":"z","mods":["ctrl"]}"#,
            &mut app,
        );
        run_attached(
            r#"{"id":2,"cmd":"input.key","key":"Escape","state":"press"}"#,
            &mut app,
        );
        run_attached(
            r#"{"id":3,"cmd":"input.key","key":"Escape","state":"release"}"#,
            &mut app,
        );
        let ctrl = Some(Modifiers::CONTROL);
        assert_eq!(
            app.applied,
            vec![
                key_sample(character("z"), ElementState::Pressed, ctrl),
                key_sample(character("z"), ElementState::Released, ctrl),
                key_sample(Key::Named(NamedKey::Escape), ElementState::Pressed, None),
                key_sample(Key::Named(NamedKey::Escape), ElementState::Released, None),
            ]
        );
    }

    /// A key is spelled as winit spells it: a character as itself, a
    /// named key by its `NamedKey` name, and a character a named key
    /// types as that named key.
    #[test]
    fn test_parse_key_speaks_winits_vocabulary() {
        assert_eq!(parse_key("a"), Ok(character("a")));
        assert_eq!(parse_key("A"), Ok(character("A")));
        assert_eq!(parse_key("/"), Ok(character("/")));
        assert_eq!(
            parse_key("e\u{301}"),
            Ok(character("e\u{301}")),
            "one grapheme, two scalars"
        );
        assert_eq!(parse_key("👨‍👩‍👧"), Ok(character("👨‍👩‍👧")), "a ZWJ sequence is one key");
        assert_eq!(parse_key(" "), Ok(Key::Named(NamedKey::Space)));
        assert_eq!(parse_key("Enter"), Ok(Key::Named(NamedKey::Enter)));
        assert_eq!(parse_key("ArrowLeft"), Ok(Key::Named(NamedKey::ArrowLeft)));
        assert_eq!(parse_key("F2"), Ok(Key::Named(NamedKey::F2)));
        assert!(
            parse_key("enter").is_err(),
            "named keys are case-exact, as winit names them"
        );
        assert!(parse_key("ab").is_err());
        assert!(parse_key("\u{7f}").is_err(), "DEL is a control character");
    }

    /// Every named key the app reads round-trips: the name a binding
    /// is written with (`key_to_name`, lowercased) is the one
    /// `input.key` parses (as winit names it), so whatever a keybind
    /// matches an agent can press.
    #[test]
    fn test_a_parsed_named_key_reaches_the_keybind_name_it_is_bound_by() {
        for (name, bound) in [
            ("Enter", "enter"),
            ("Escape", "escape"),
            ("Tab", "tab"),
            ("Backspace", "backspace"),
            ("Delete", "delete"),
            ("ArrowUp", "arrowup"),
            ("F2", "f2"),
        ] {
            let key = parse_key(name).expect(name);
            assert_eq!(
                crate::application::keybinds::key_to_name(&key).as_deref(),
                Some(bound),
                "{name}"
            );
        }
    }

    /// "hello 👋" is seven key taps: the space arrives as the Space
    /// key and the emoji as one character, as a keyboard types them.
    #[test]
    fn test_text_types_one_tap_per_grapheme() {
        assert_eq!(
            typed_keys("hello 👋"),
            Ok(vec![
                character("h"),
                character("e"),
                character("l"),
                character("l"),
                character("o"),
                Key::Named(NamedKey::Space),
                character("👋"),
            ])
        );
        assert_eq!(
            typed_keys("a\r\nb\tc\nd"),
            Ok(vec![
                character("a"),
                Key::Named(NamedKey::Enter),
                character("b"),
                Key::Named(NamedKey::Tab),
                character("c"),
                Key::Named(NamedKey::Enter),
                character("d"),
            ]),
            "CRLF is one grapheme and one Enter"
        );
        let mut app = InputApp::with_fixture();
        let (reply, _) = run_attached(r#"{"id":1,"cmd":"input.text","text":"é👋"}"#, &mut app);
        assert_eq!(reply["result"], json!({"applied": true, "pending": 0}));
        assert_eq!(
            app.applied,
            vec![
                key_sample(character("é"), ElementState::Pressed, None),
                key_sample(character("é"), ElementState::Released, None),
                key_sample(character("👋"), ElementState::Pressed, None),
                key_sample(character("👋"), ElementState::Released, None),
            ]
        );
    }

    #[test]
    fn test_touch_is_one_finger_event_at_a_surface_position() {
        let mut app = InputApp::with_fixture();
        run_attached(
            r#"{"id":1,"cmd":"input.touch","phase":"canceled","pointer_id":7,"x":3,"y":4}"#,
            &mut app,
        );
        assert_eq!(
            app.applied,
            vec![Sample::new(
                InputEvent::Touch {
                    phase: TouchPhase::Cancelled,
                    pointer_id: 7,
                    at: (3.0, 4.0)
                },
                None
            )]
        );
    }

    /// A drag is a move to `from`, the press, `steps` evenly spaced
    /// moves ending exactly on `to`, and the release, each holding the
    /// drag's modifiers.
    #[test]
    fn test_drag_samples_end_exactly_on_the_target_with_the_modifiers_held() {
        let surface = |x: f32, y: f32| Point {
            x,
            y,
            space: Space::Surface,
        };
        let shift = Some(Modifiers::SHIFT);
        let unresolved = drag_samples(
            surface(0.0, 10.0),
            surface(30.0, -20.0),
            3,
            MouseButton::Left,
            shift,
        );
        let samples: Vec<Sample<Surface>> =
            resolve_command(unresolved, |p| (f64::from(p.x), f64::from(p.y))).collect();
        assert_eq!(
            samples,
            vec![
                moved(0.0, 10.0, shift),
                button(MouseButton::Left, ElementState::Pressed, shift),
                moved(10.0, 0.0, shift),
                moved(20.0, -10.0, shift),
                moved(30.0, -20.0, shift),
                button(MouseButton::Left, ElementState::Released, shift),
            ]
        );
        // Steps that do not divide evenly still end on the target
        // itself, not on the start plus a rounded fraction.
        let long = drag_samples(
            surface(0.1, 0.0),
            surface(1.0, 3.3),
            MAX_DRAG_STEPS,
            MouseButton::Left,
            None,
        );
        assert_eq!(
            long.len(),
            INJECTED_QUEUE_CAP,
            "the longest drag fills the queue exactly"
        );
        assert_eq!(
            long[long.len() - 2],
            Sample::new(InputEvent::CursorMoved(surface(1.0, 3.3)), None)
        );
    }

    /// The drag goes through the queue — nothing reaches a handler
    /// inside the command — and input sent after it waits behind it.
    #[test]
    fn test_drag_is_paced_through_the_queue_and_later_input_waits_behind_it() {
        let mut app = InputApp::with_fixture();
        let (reply, repaint) = run_attached(
            r#"{"id":1,"cmd":"input.drag","from_x":0,"from_y":0,"to_x":40,"to_y":0,"steps":4,"mods":["shift"]}"#,
            &mut app,
        );
        assert_eq!(reply["result"], json!({"applied": false, "pending": 7}));
        assert!(
            !repaint,
            "the heartbeat, not the command, asks for the drag's frames"
        );
        assert!(app.applied.is_empty());

        let (reply, _) = run_attached(r#"{"id":2,"cmd":"input.key","key":"Escape"}"#, &mut app);
        assert_eq!(
            reply["result"],
            json!({"applied": false, "pending": 9}),
            "a key sent mid-drag lands after the release, not before"
        );
        let (reply, _) = run_attached(r#"{"id":3,"cmd":"input.pending"}"#, &mut app);
        assert_eq!(reply["result"], json!({"pending": 9}));
        let camera = &app.camera;
        let order: Vec<Sample<Surface>> =
            std::iter::from_fn(|| app.queue.pop(|point| through(camera, point))).collect();
        assert_eq!(
            order[6],
            button(MouseButton::Left, ElementState::Released, Some(Modifiers::SHIFT))
        );
        assert_eq!(
            order[7],
            key_sample(Key::Named(NamedKey::Escape), ElementState::Pressed, None)
        );
    }

    /// A drag that does not fit is `overloaded` with the queue's
    /// state, and leaves the queue exactly as it was.
    #[test]
    fn test_an_overflowing_drag_is_overloaded_and_queues_nothing() {
        let mut app = InputApp::with_fixture();
        run_attached(
            r#"{"id":1,"cmd":"input.drag","from_x":0,"from_y":0,"to_x":1,"to_y":0,"steps":1000}"#,
            &mut app,
        );
        let (reply, _) = run_attached(
            r#"{"id":2,"cmd":"input.drag","from_x":0,"from_y":0,"to_x":1,"to_y":0,"steps":30}"#,
            &mut app,
        );
        assert_eq!(reply["error"]["code"], json!("overloaded"), "{reply}");
        assert_eq!(reply["error"]["data"], json!({"pending": 1003, "cap": 1024}));
        assert_eq!(app.queue.len(), 1003);
    }

    /// `click_node` aims at the center of the node's model AABB, or of
    /// the section's, and clicks there: a move, a press, a release,
    /// reported in surface pixels.
    #[test]
    fn test_click_node_clicks_the_model_center_through_the_camera() {
        let mut app = InputApp::with_fixture();
        let id = first_node(app.doc.as_ref().expect("doc"));
        let (reply, repaint) = run_attached(
            &json!({"id":1,"cmd":"input.click_node","node_id":id,"mods":["ctrl"]}).to_string(),
            &mut app,
        );
        assert!(repaint);
        assert_eq!(
            reply["result"],
            json!({"applied": true, "pending": 0, "x": 640.0, "y": 360.0}),
            "the camera is centered on the node"
        );
        let ctrl = Some(Modifiers::CONTROL);
        assert_eq!(
            app.applied,
            vec![
                moved(640.0, 360.0, ctrl),
                button(MouseButton::Left, ElementState::Pressed, ctrl),
                button(MouseButton::Left, ElementState::Released, ctrl),
            ]
        );
    }

    #[test]
    fn test_click_node_aims_at_a_section_by_the_tree_builders_rule() {
        let mut doc = load_test_doc();
        let id = first_node(&doc);
        {
            let node = doc.mindmap.nodes.get_mut(&id).expect("node");
            let mut second = node.sections[0].clone();
            second.offset = baumhard::mindmap::model::Position { x: 10.0, y: 20.0 };
            second.size = Some(baumhard::mindmap::model::Size {
                width: 30.0,
                height: 40.0,
            });
            node.sections.push(second);
        }
        let node = &doc.mindmap.nodes[&id];
        assert_eq!(
            node_target(&doc, &id, Some(1), 1.0),
            Ok(Vec2::new(
                (node.position.x + 25.0) as f32,
                (node.position.y + 40.0) as f32
            )),
            "an explicit section size: the offset plus half the size"
        );
        assert_eq!(
            node_target(&doc, &id, Some(0), 1.0),
            Ok(node.center_vec2()),
            "a section that fills its node aims at the node's center"
        );
        assert_eq!(node_target(&doc, &id, Some(2), 1.0), Err(Aim::NoSection(2)));
        assert_eq!(node_target(&doc, "no.such.node", None, 1.0), Err(Aim::Missing));
        let node = doc.mindmap.nodes.get_mut(&id).expect("node");
        node.min_zoom_to_render = Some(1.5);
        node.max_zoom_to_render = Some(3.0);
        assert_eq!(
            node_target(&doc, &id, None, 1.0),
            Err(Aim::OutsideZoomWindow),
            "below the window the scene does not draw it"
        );
        assert_eq!(
            node_target(&doc, &id, None, 4.0),
            Err(Aim::OutsideZoomWindow),
            "nor above it"
        );
        assert!(
            node_target(&doc, &id, None, 2.0).is_ok(),
            "inside it, it is drawn"
        );
    }

    #[test]
    fn test_click_node_refuses_what_a_hand_could_not_click() {
        let mut app = InputApp::with_fixture();
        let doc = app.doc.as_ref().expect("doc");
        let parent = doc
            .mindmap
            .nodes
            .values()
            .find_map(|n| n.parent_id.clone())
            .expect("the fixture has a child node");
        let child = doc
            .mindmap
            .nodes
            .values()
            .find(|n| n.parent_id.as_deref() == Some(parent.as_str()))
            .map(|n| n.id.clone())
            .expect("child");
        app.doc
            .as_mut()
            .expect("doc")
            .mindmap
            .nodes
            .get_mut(&parent)
            .expect("parent")
            .folded = true;
        for (frame, code, says) in [
            (
                json!({"id":1,"cmd":"input.click_node","node_id":"no.such.node"}),
                "not_found",
                "no node",
            ),
            (
                json!({"id":1,"cmd":"input.click_node","node_id":child}),
                "not_found",
                "folded",
            ),
            (
                json!({"id":1,"cmd":"input.click_node","node_id":parent,"section":99}),
                "invalid_params",
                "section",
            ),
        ] {
            let (reply, repaint) = run_attached(&frame.to_string(), &mut app);
            assert_eq!(reply["error"]["code"], json!(code), "{frame}: {reply}");
            assert!(
                reply["error"]["message"]
                    .as_str()
                    .is_some_and(|m| m.contains(says)),
                "{frame}: {reply}"
            );
            assert!(!repaint);
        }
        app.camera.position += Vec2::new(1.0e6, 0.0);
        let (reply, _) = run_attached(
            &json!({"id":1,"cmd":"input.click_node","node_id":parent}).to_string(),
            &mut app,
        );
        assert_eq!(reply["error"]["code"], json!("not_found"), "{reply}");
        assert!(
            reply["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("not on screen")),
            "{reply}"
        );
        assert!(app.applied.is_empty(), "nothing was clicked");
    }

    /// A node outside its zoom window is not drawn, so a click at
    /// its center would land on empty canvas: refused, nothing sent.
    #[test]
    fn test_click_node_refuses_a_node_outside_its_zoom_window() {
        let mut app = InputApp::with_fixture();
        let id = first_node(app.doc.as_ref().expect("doc"));
        let node = app
            .doc
            .as_mut()
            .expect("doc")
            .mindmap
            .nodes
            .get_mut(&id)
            .expect("node");
        node.min_zoom_to_render = Some(3.0);
        let (reply, repaint) = run_attached(
            &json!({"id":1,"cmd":"input.click_node","node_id":id}).to_string(),
            &mut app,
        );
        assert_eq!(reply["error"]["code"], json!("not_found"), "{reply}");
        assert!(
            reply["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("zoom")),
            "{reply}"
        );
        assert!(!repaint);
        assert!(app.applied.is_empty());
    }

    /// Behind queued samples a click would land after they may have
    /// moved the camera, and the reply could not say where: it is
    /// refused `busy` with the queue depth, and queues nothing.
    #[test]
    fn test_click_node_is_busy_while_samples_are_queued() {
        let mut app = InputApp::with_fixture();
        let id = first_node(app.doc.as_ref().expect("doc"));
        run_attached(
            r#"{"id":1,"cmd":"input.drag","from_x":0,"from_y":0,"to_x":40,"to_y":0,"steps":2}"#,
            &mut app,
        );
        let (reply, repaint) = run_attached(
            &json!({"id":2,"cmd":"input.click_node","node_id":id}).to_string(),
            &mut app,
        );
        assert_eq!(reply["error"]["code"], json!("busy"), "{reply}");
        assert_eq!(reply["error"]["data"], json!({"pending": 5}));
        assert!(!repaint);
        assert_eq!(app.queue.len(), 5, "nothing joined the queue");
    }

    /// A modifier key alone would change nothing a later event reads —
    /// winit reports modifiers through a separate state event — so it
    /// is refused, pointing at `mods`.
    #[test]
    fn test_a_modifier_key_is_refused_toward_mods() {
        for name in ["Shift", "Control", "Alt", "AltGraph", "Super", "Meta", "Hyper"] {
            let refusal = parse_key(name).expect_err(name);
            assert!(refusal.contains("`mods`"), "{name}: {refusal}");
        }
        assert_eq!(
            parse_key("CapsLock"),
            Ok(Key::Named(NamedKey::CapsLock)),
            "a lock key is a keystroke"
        );
    }

    #[test]
    fn test_click_node_needs_a_loaded_document() {
        let mut app = InputApp::with_fixture();
        app.doc = None;
        let (reply, _) = run_attached(r#"{"id":1,"cmd":"input.click_node","node_id":"0"}"#, &mut app);
        assert_eq!(reply["error"]["code"], json!("no_document"));
        assert!(reply["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("open <path>")));
        let (reply, _) = run_attached(r#"{"id":2,"cmd":"input.mouse_move","x":1,"y":1}"#, &mut app);
        assert_eq!(
            reply["result"]["applied"],
            json!(true),
            "raw input needs no document: a hand can click an empty window"
        );
    }
}
