// SPDX-License-Identifier: MPL-2.0

//! The commands this client knows by name: what `--help` prints, how
//! a positional argument maps to a parameter, and which type a typed
//! `name=value` is coerced to.
//!
//! **A verb is a wire command, spelled as the wire spells it.** There
//! is no client-side vocabulary (`format/ipc.md` is the one): the
//! name is the registry's `family.verb`, a parameter is the
//! registry's parameter, a type word is `describe`'s. What the table
//! adds is only what the wire cannot say: a one-line meaning written
//! for an agent, which parameters may be given positionally, whether
//! the command changes the app (so `--wait-settled` has something to
//! wait for), how to print its result, and a command line to copy.
//!
//! **The table is held to the registry, not trusted.** A test in the
//! app crate (`ipc/commands/mod.rs`) walks the real registry against
//! [`VERBS`] in both directions — every registered command has a verb,
//! every verb a command, and parameter names, types and required
//! flags agree — so a server command that lands without its verb
//! fails *that* PR's `./test.sh`, and a verb whose command changed
//! shape cannot ship. A command missing from here still works —
//! [`crate::run()`] types its arguments from `describe` — it is only
//! absent from `--help`.

/// One JSON shape a parameter can take; `describe`'s type words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    String,
    Integer,
    Number,
    Boolean,
    Object,
    Array,
    /// A type word this build does not know — a newer server's.
    /// Never in [`VERBS`]; arguments of this shape are sent as the
    /// JSON they parse as, or as a string when they are not JSON.
    Other,
}

impl Shape {
    /// The type word `describe` publishes for this shape.
    pub fn word(self) -> &'static str {
        match self {
            Shape::String => "string",
            Shape::Integer => "integer",
            Shape::Number => "number",
            Shape::Boolean => "boolean",
            Shape::Object => "object",
            Shape::Array => "array",
            Shape::Other => "any",
        }
    }

    /// The shape a `describe` type word names ([`Shape::Other`] for
    /// a word this build has not heard of).
    pub fn from_word(word: &str) -> Shape {
        match word {
            "string" => Shape::String,
            "integer" => Shape::Integer,
            "number" => Shape::Number,
            "boolean" => Shape::Boolean,
            "object" => Shape::Object,
            "array" => Shape::Array,
            _ => Shape::Other,
        }
    }
}

/// A parameter's type: the shapes it admits, `describe`'s `a|b` form.
pub type Ty = &'static [Shape];

const STRING: Ty = &[Shape::String];
const INTEGER: Ty = &[Shape::Integer];
const NUMBER: Ty = &[Shape::Number];
const BOOLEAN: Ty = &[Shape::Boolean];
const OBJECT: Ty = &[Shape::Object];
const ARRAY: Ty = &[Shape::Array];
const STRING_OR_OBJECT: Ty = &[Shape::String, Shape::Object];

/// The `describe` spelling of a type: its shapes' words joined by `|`.
pub fn type_word(ty: &[Shape]) -> String {
    ty.iter().map(|s| s.word()).collect::<Vec<_>>().join("|")
}

/// One parameter of a verb, as `describe` reports it.
#[derive(Debug)]
pub struct Param {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
    /// A filesystem path the app opens. The app requires it absolute
    /// and resolves nothing against a working directory of its own, so
    /// the client, which is the one that knows the shell's, makes a
    /// relative `name=value` absolute before sending. The app crate's
    /// tests hold this flag to the registry's own "Absolute path" docs.
    pub path: bool,
}

const fn param(name: &'static str, ty: Ty, required: bool) -> Param {
    Param {
        name,
        ty,
        required,
        path: false,
    }
}

const fn path_param(name: &'static str, required: bool) -> Param {
    Param {
        name,
        ty: STRING,
        required,
        path: true,
    }
}

/// Whether a command changes what the app shows or holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Reads, or writes only outside the app (a screenshot file).
    /// `--wait-settled` is refused: there is nothing to settle.
    Reads,
    /// Acts on the app. `--wait-settled` waits for its consequences.
    Mutates,
}

/// How a verb's result is printed in the default (non-`--json`) mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Render {
    /// One `path: value` line per leaf (see `render`).
    Flat,
    /// The named array of strings as bare lines, then the rest flat.
    Lines(&'static str),
    /// `describe`'s command table, compact.
    Describe,
}

/// A command the client knows by name.
#[derive(Debug)]
pub struct Verb {
    /// The wire command, `family.verb`, or `describe` / `ping`.
    pub name: &'static str,
    /// What it does, in one line, for an agent choosing a command.
    pub summary: &'static str,
    /// Every parameter the server registers, in registry order.
    pub params: &'static [Param],
    /// Parameters that may be given bare, in argv order. Each names
    /// an entry of `params`.
    pub positional: &'static [&'static str],
    pub effect: Effect,
    pub render: Render,
    /// A complete command line that works as printed (a test parses
    /// every one).
    pub example: &'static str,
}

const MODS: Param = param("mods", ARRAY, false);
const SPACE: Param = param("space", STRING, false);
const X: Param = param("x", NUMBER, true);
const Y: Param = param("y", NUMBER, true);
const AT_X: Param = param("x", NUMBER, false);
const AT_Y: Param = param("y", NUMBER, false);
const BUTTON: Param = param("button", STRING, false);

/// Every command the connected build's registry holds, as of this
/// commit — see the module header for how that is enforced.
pub const VERBS: &[Verb] = &[
    Verb {
        name: "describe",
        summary:
            "List the commands the connected build serves, with parameters and results; name one for just it.",
        params: &[param("command", STRING, false)],
        positional: &["command"],
        effect: Effect::Reads,
        render: Render::Describe,
        example: "mandalactl describe act.action",
    },
    Verb {
        name: "ping",
        summary: "Liveness probe: returns the app clock (now_ms). Proves the socket and the app answer.",
        params: &[],
        positional: &[],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl ping",
    },
    Verb {
        name: "act.action",
        summary: "Fire one Action through the funnel a key binding uses: undoable, gated, same as by hand.",
        params: &[param("action", STRING_OR_OBJECT, true), AT_X, AT_Y, SPACE],
        positional: &["action"],
        effect: Effect::Mutates,
        render: Render::Flat,
        example: "mandalactl act.action ZoomIn",
    },
    Verb {
        name: "act.console",
        summary: "Run one console line without opening the console; prints the verb's output lines.",
        params: &[param("line", STRING, true)],
        positional: &["line"],
        effect: Effect::Mutates,
        render: Render::Lines("output"),
        example: "mandalactl act.console \"section show\"",
    },
    Verb {
        name: "act.macro",
        summary:
            "Run a registered macro by macro_id (its own tier), or an inline step list at the User tier.",
        params: &[param("macro_id", STRING, false), param("steps", ARRAY, false)],
        positional: &["macro_id"],
        effect: Effect::Mutates,
        render: Render::Flat,
        example: "mandalactl act.macro steps='[{\"kind\":\"Action\",\"action\":\"ZoomFit\"}]'",
    },
    Verb {
        name: "query.document",
        summary:
            "The open document: path, dirty flag, undo depth, counts; pointer= or fields= reads the model.",
        params: &[
            param("fields", ARRAY, false),
            param("pointer", STRING, false),
            path_param("to_file", false),
            param("overwrite", BOOLEAN, false),
        ],
        positional: &["pointer"],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl query.document /nodes/0/position",
    },
    Verb {
        name: "query.selection",
        summary: "The current selection: its kind (None, Single, Multi, Edge, ...) and the ids it holds.",
        params: &[],
        positional: &[],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl query.selection",
    },
    Verb {
        name: "query.camera",
        summary:
            "The camera: canvas center, zoom and its limits, surface size; enough to convert coordinates.",
        params: &[],
        positional: &[],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl query.camera",
    },
    Verb {
        name: "query.mode",
        summary: "Interaction mode, drag state, open modals, who owns the next key, hovered node, pointer.",
        params: &[],
        positional: &[],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl query.mode",
    },
    Verb {
        name: "query.status",
        summary:
            "The mode-status line the screen shows (\"editing: 1.2 ...\"), or null when none is showing.",
        params: &[],
        positional: &[],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl query.status",
    },
    Verb {
        name: "query.animations",
        summary: "Running animations with their phase and progress; active=false means they have settled.",
        params: &[],
        positional: &[],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl query.animations",
    },
    Verb {
        name: "query.editors",
        summary: "Live state of the console, color picker, text editors and the document's active previews.",
        params: &[],
        positional: &[],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl query.editors",
    },
    Verb {
        name: "query.undo",
        summary: "The undo stack: its depth and its newest entries, newest first (default 20).",
        params: &[param("limit", INTEGER, false)],
        positional: &["limit"],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl query.undo 5",
    },
    Verb {
        name: "scene.dump",
        summary:
            "What is drawn: nodes, edges, labels, portals with canvas/surface rects, visibility and text.",
        params: &[
            param("include", STRING, false),
            param("kinds", ARRAY, false),
            path_param("to_file", false),
            param("overwrite", BOOLEAN, false),
        ],
        positional: &[],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl scene.dump kinds=nodes include=all",
    },
    Verb {
        name: "scene.visible_set",
        summary: "Just the ids of what the camera shows right now, by kind; no text, no rects.",
        params: &[],
        positional: &[],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl scene.visible_set",
    },
    Verb {
        name: "scene.node_geometry",
        summary: "One node: model position and size, whether it is drawn and where, what overlaps it.",
        params: &[param("node_id", STRING, true)],
        positional: &["node_id"],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl scene.node_geometry 1.2",
    },
    Verb {
        name: "scene.hit_test",
        summary: "What a click at a point would hit: the click ladder's node, portal, label and edge rungs.",
        params: &[X, Y, SPACE],
        positional: &["x", "y"],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl scene.hit_test 412 96 space=surface",
    },
    Verb {
        name: "scene.find_text",
        summary:
            "Find drawn text (sections, edge labels, portal captions), on screen or not, with where it is.",
        params: &[
            param("text", STRING, true),
            param("exact", BOOLEAN, false),
            param("case_sensitive", BOOLEAN, false),
        ],
        positional: &["text"],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl scene.find_text Introduction exact=true",
    },
    Verb {
        name: "input.mouse_move",
        summary: "Move the pointer as a real mouse would: hover, drag threshold and drag updates follow.",
        params: &[X, Y, SPACE, MODS],
        positional: &["x", "y"],
        effect: Effect::Mutates,
        render: Render::Flat,
        example: "mandalactl input.mouse_move 640 360",
    },
    Verb {
        name: "input.mouse_button",
        summary: "Press, release or click a mouse button (left, right, middle), optionally at x= y=.",
        params: &[
            param("button", STRING, true),
            param("state", STRING, true),
            AT_X,
            AT_Y,
            SPACE,
            MODS,
        ],
        positional: &["button", "state"],
        effect: Effect::Mutates,
        render: Render::Flat,
        example: "mandalactl input.mouse_button left click x=412 y=96",
    },
    Verb {
        name: "input.wheel",
        summary: "Turn the mouse wheel by dy lines (positive up): zooms at the pointer, scrolls the console.",
        params: &[param("dy", NUMBER, true), AT_X, AT_Y, SPACE, MODS],
        positional: &["dy"],
        effect: Effect::Mutates,
        render: Render::Flat,
        example: "mandalactl input.wheel 3 x=640 y=360",
    },
    Verb {
        name: "input.key",
        summary: "Tap, press or release one key through the whole keystroke ladder; mods= holds modifiers.",
        params: &[param("key", STRING, true), param("state", STRING, false), MODS],
        positional: &["key"],
        effect: Effect::Mutates,
        render: Render::Flat,
        example: "mandalactl input.key z mods=ctrl",
    },
    Verb {
        name: "input.text",
        summary: "Type a string, one key tap per grapheme, into whatever has the keyboard (up to 512).",
        params: &[param("text", STRING, true)],
        positional: &["text"],
        effect: Effect::Mutates,
        render: Render::Flat,
        example: "mandalactl input.text \"hello 👋\"",
    },
    Verb {
        name: "input.touch",
        summary: "One finger event (started, moved, ended, canceled) into the touch-gesture recognizer.",
        params: &[
            param("phase", STRING, true),
            param("pointer_id", INTEGER, true),
            X,
            Y,
            SPACE,
        ],
        positional: &["phase", "pointer_id", "x", "y"],
        effect: Effect::Mutates,
        render: Render::Flat,
        example: "mandalactl input.touch started 1 640 360",
    },
    Verb {
        name: "input.drag",
        summary:
            "Drag from one point to another, one sample per frame: moves a node, pans, resizes, selects.",
        params: &[
            param("from_x", NUMBER, true),
            param("from_y", NUMBER, true),
            param("to_x", NUMBER, true),
            param("to_y", NUMBER, true),
            SPACE,
            BUTTON,
            param("steps", INTEGER, false),
            MODS,
        ],
        positional: &["from_x", "from_y", "to_x", "to_y"],
        effect: Effect::Mutates,
        render: Render::Flat,
        example: "mandalactl input.drag 400 300 640 300",
    },
    Verb {
        name: "input.click_node",
        summary: "Click the center of a node (or of one section=) by id; mods=shift extends the selection.",
        params: &[
            param("node_id", STRING, true),
            param("section", INTEGER, false),
            BUTTON,
            MODS,
        ],
        positional: &["node_id"],
        effect: Effect::Mutates,
        render: Render::Flat,
        example: "mandalactl input.click_node 1.2",
    },
    Verb {
        name: "input.pending",
        summary:
            "How many injected input samples are still queued; poll to 0 before reading a drag's result.",
        params: &[],
        positional: &[],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl input.pending",
    },
    Verb {
        name: "capture.screenshot",
        summary:
            "Render the current state offscreen to a PNG plus a geometry sidecar mapping pixels to node ids.",
        params: &[
            path_param("path", false),
            param("overwrite", BOOLEAN, false),
            param("width", INTEGER, false),
            param("height", INTEGER, false),
            param("region", OBJECT, false),
            SPACE,
            param("fit", BOOLEAN, false),
            param("scale", NUMBER, false),
            param("format", STRING, false),
            param("sidecar", BOOLEAN, false),
        ],
        positional: &["path"],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl capture.screenshot /tmp/shot.png fit=true",
    },
    Verb {
        name: "capture.record_start",
        summary:
            "Start recording frames (PNGs + manifest.json) at fps= into dir=; `record assemble <dir>` makes the GIF.",
        params: &[
            path_param("dir", false),
            param("fps", INTEGER, false),
            param("max_frames", INTEGER, false),
            param("width", INTEGER, false),
            param("height", INTEGER, false),
            param("region", OBJECT, false),
            SPACE,
            param("fit", BOOLEAN, false),
            param("scale", NUMBER, false),
        ],
        positional: &["dir"],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl capture.record_start /tmp/zoom fps=20 max_frames=120",
    },
    Verb {
        name: "capture.record_stop",
        summary:
            "Stop the recording and finish its manifest (or collect the report of one that stopped itself).",
        params: &[],
        positional: &[],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl capture.record_stop",
    },
    Verb {
        name: "clock.status",
        summary: "The app clock: its mode (real or virtual) and its reading in ms.",
        params: &[],
        positional: &[],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl clock.status",
    },
    Verb {
        name: "clock.set_mode",
        summary:
            "Run the app clock in real time, or in virtual time that advances only by clock.step (refused while recording).",
        params: &[param("mode", STRING, true)],
        positional: &["mode"],
        effect: Effect::Mutates,
        render: Render::Flat,
        example: "mandalactl clock.set_mode virtual",
    },
    Verb {
        name: "clock.step",
        summary:
            "Advance the virtual clock by ms= (or frames= heartbeats, 16 ms apart), running every frame on the way.",
        params: &[param("ms", INTEGER, false), param("frames", INTEGER, false)],
        positional: &["ms"],
        effect: Effect::Mutates,
        render: Render::Flat,
        example: "mandalactl clock.step 500",
    },
    Verb {
        name: "clock.wait",
        summary:
            "Wait until the app is settled (or animations_complete); timeout_ms= is wall-clock, default 10000.",
        params: &[param("until", STRING, true), param("timeout_ms", INTEGER, false)],
        positional: &["until"],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl clock.wait settled",
    },
    Verb {
        name: "events.subscribe",
        summary:
            "Subscribe to change events and stay connected, printing each as it happens; --count N stops after N.",
        params: &[param("classes", ARRAY, true), param("verbose", BOOLEAN, false)],
        positional: &["classes"],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl events.subscribe document_changed,selection_changed --count 1",
    },
    Verb {
        name: "events.unsubscribe",
        summary:
            "Drop event classes (all when none named) on this connection; a one-shot invocation has none to drop.",
        params: &[param("classes", ARRAY, false)],
        positional: &["classes"],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl events.unsubscribe",
    },
    Verb {
        name: "events.revision",
        summary: "The revision counters: revision (any change) and document_revision (the model's), no subscription.",
        params: &[],
        positional: &[],
        effect: Effect::Reads,
        render: Render::Flat,
        example: "mandalactl events.revision",
    },
];

/// The verb named `name`, if the table has it.
pub fn lookup(name: &str) -> Option<&'static Verb> {
    VERBS.iter().find(|verb| verb.name == name)
}

/// The family a command belongs to: the text before its dot, or
/// `bootstrap` for the two unnamespaced commands.
pub fn family(name: &str) -> &str {
    name.split_once('.').map_or("bootstrap", |(family, _)| family)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verb_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for verb in VERBS {
            assert!(seen.insert(verb.name), "`{}` is listed twice", verb.name);
        }
    }

    #[test]
    fn test_every_positional_names_a_declared_parameter_once() {
        for verb in VERBS {
            let mut seen = std::collections::HashSet::new();
            for name in verb.positional {
                assert!(
                    verb.params.iter().any(|p| p.name == *name),
                    "`{}`: positional `{name}` is not a parameter",
                    verb.name
                );
                assert!(
                    seen.insert(name),
                    "`{}`: positional `{name}` is listed twice",
                    verb.name
                );
            }
        }
    }

    #[test]
    fn test_no_verb_declares_a_reserved_envelope_key_as_a_parameter() {
        for verb in VERBS {
            for param in verb.params {
                assert!(
                    param.name != "id" && param.name != "cmd",
                    "`{}`: `{}` collides with the envelope (format/ipc.md, reserved-word rule)",
                    verb.name,
                    param.name
                );
            }
        }
    }

    #[test]
    fn test_required_parameters_come_before_optional_positionals() {
        for verb in VERBS {
            let mut optional_seen = false;
            for name in verb.positional {
                let required = verb.params.iter().any(|p| p.name == *name && p.required);
                assert!(
                    !(required && optional_seen),
                    "`{}`: required `{name}` follows an optional positional",
                    verb.name
                );
                optional_seen |= !required;
            }
        }
    }

    #[test]
    fn test_family_is_the_text_before_the_dot_or_bootstrap() {
        assert_eq!(family("act.action"), "act");
        assert_eq!(family("capture.screenshot"), "capture");
        assert_eq!(family("ping"), "bootstrap");
        assert_eq!(family("describe"), "bootstrap");
    }

    #[test]
    fn test_type_words_round_trip_through_shapes() {
        for word in ["string", "integer", "number", "boolean", "object", "array"] {
            assert_eq!(Shape::from_word(word).word(), word);
        }
        assert_eq!(Shape::from_word("duration"), Shape::Other);
        assert_eq!(type_word(STRING_OR_OBJECT), "string|object");
    }

    #[test]
    fn test_lookup_finds_a_verb_and_refuses_a_stranger() {
        assert_eq!(lookup("ping").map(|v| v.name), Some("ping"));
        assert!(lookup("future.verb").is_none());
    }
}
