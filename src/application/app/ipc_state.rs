// SPDX-License-Identifier: MPL-2.0

//! IPC projections of the app shell's own state (`format/ipc.md`
//! §`query`): the interaction mode, the drag state machine, the open
//! modals. The camera's projection is the protocol's own coordinate-space
//! transform and lives with it (`ipc/space.rs`); the document-layer half
//! lives beside the document (`document/ipc_state.rs`); this half lives
//! in `app` because `DragState` is private to it.
//!
//! The rules are the document half's. Every projection is an inherent
//! method or function over `&` — a read cannot mutate — and every
//! `match` over an app-state enum is **exhaustive with no wildcard
//! arm**, so a new `InteractionMode` or `DragState` variant breaks the
//! build here, on purpose, until its wire form is decided and written
//! into `format/ipc.md`. Each projection's `kind` word is the variant's
//! own name. Where a type owns internals with no wire meaning (cached
//! layout, scratch buffers) the projection is a stated subset and the
//! omission is named at its method.
//!
//! **Two targets, one wire shape.** The reply's keys are assembled here
//! once ([`mode_to_ipc_json`], [`editors_to_ipc_json`]) from values each
//! target projects for itself: native projects its drag state machine,
//! its console and its picker; the browser, which has no drag machine,
//! no console shell and no picker, reports `{"kind": "None"}` and `null`
//! for them. The interaction mode is both targets', and so is the key
//! routing, which is over open-flags. The editors on the map — the text
//! and single-line editors — return with V1-C8 (#249).

use serde_json::{json, Value};

#[cfg(not(target_arch = "wasm32"))]
use crate::application::color_picker::{ColorPickerState, OpenPicker, PickerGesture, PickerMode};
#[cfg(not(target_arch = "wasm32"))]
use crate::application::console::{ConsoleLine, ConsoleState};
#[cfg(not(target_arch = "wasm32"))]
use crate::application::view::inspect::f32_json;

use super::InteractionMode;
#[cfg(not(target_arch = "wasm32"))]
use super::{DragState, HeldInput, PanOwner};

/// How many scrollback lines `query.editors` reports for an open
/// console: the tail, which is what is on screen. The total is
/// `scrollback_len`.
#[cfg(not(target_arch = "wasm32"))]
pub(super) const CONSOLE_SCROLLBACK_TAIL: usize = 50;

/// `{"x", "y"}`.
fn xy_f64(point: (f64, f64)) -> Value {
    json!({ "x": point.0, "y": point.1 })
}

/// The drag state a target with no drag state machine reports: the
/// browser's, always (`DragState` is native). The same words native's
/// `DragState::None` projects to, so `query.mode` reads alike.
#[cfg(target_arch = "wasm32")]
pub(super) fn no_drag_state_json() -> Value {
    json!({ "kind": "None" })
}

impl InteractionMode {
    /// The mode as `query.mode` reports it: `kind` is the variant's
    /// name. Lossless.
    pub(in crate::application) fn to_ipc_json(&self) -> Value {
        match self {
            InteractionMode::Default => json!({ "kind": "Default" }),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl DragState {
    /// The drag state machine as `query.mode` reports it. `kind` is the
    /// variant's name. Positions are what each variant carries:
    /// `start_pos` in surface pixels, `*_canvas` in canvas space; a
    /// pending press names the element it landed on, or `null`.
    pub(super) fn to_ipc_json(&self, doc: Option<&crate::application::document::MindMapDocument>) -> Value {
        let element = |hit: super::ClickHit| match (hit.element(), doc) {
            (Some(element), Some(doc)) => crate::application::view::inspect::element_ref(doc.doc(), element),
            _ => Value::Null,
        };
        let canvas = |point: glam::Vec2| json!({ "x": f32_json(point.x), "y": f32_json(point.y) });
        match self {
            DragState::None => json!({ "kind": "None" }),
            DragState::Pending { start_pos, hit } => json!({
                "kind": "Pending",
                "start_pos": xy_f64(*start_pos),
                "hit": element(*hit),
            }),
            DragState::PendingRight {
                start_pos,
                start_canvas,
                hit,
            } => json!({
                "kind": "PendingRight",
                "start_pos": xy_f64(*start_pos),
                "start_canvas": canvas(*start_canvas),
                "hit": element(*hit),
            }),
            DragState::Panning { owner } => match owner {
                PanOwner::Toggled => json!({ "kind": "Panning", "owner": "toggled" }),
                PanOwner::Held(HeldInput::Button(button)) => json!({
                    "kind": "Panning",
                    "owner": "button",
                    "button": crate::application::ipc::injected::button_word(*button),
                }),
                PanOwner::Held(HeldInput::Key(key)) => json!({
                    "kind": "Panning",
                    "owner": "key",
                    "key": key,
                }),
            },
            DragState::SelectingRect {
                start_canvas,
                current_canvas,
            } => json!({
                "kind": "SelectingRect",
                "start_canvas": canvas(*start_canvas),
                "current_canvas": canvas(*current_canvas),
            }),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn console_line_json(line: &ConsoleLine) -> Value {
    match line {
        ConsoleLine::Input(text) => json!({ "kind": "Input", "text": text }),
        ConsoleLine::Output { text, .. } => json!({ "kind": "Output", "text": text }),
        ConsoleLine::Error(text) => json!({ "kind": "Error", "text": text }),
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// The console as `query.editors` reports it: `null` when closed.
/// `cursor` is a grapheme index into `input`. The scrollback is its
/// last [`CONSOLE_SCROLLBACK_TAIL`] lines, the rest counted in
/// `scrollback_len`.
///
/// **Omitted by name:** each output line's pinned `font_family`, the
/// completion rows (only their count and the highlighted index are
/// reported) and the wheel accumulator.
pub(super) fn console_to_ipc_json(state: &ConsoleState) -> Value {
    match state {
        ConsoleState::Closed => Value::Null,
        ConsoleState::Open {
            input,
            cursor,
            history,
            history_idx,
            scrollback,
            completions,
            completion_idx,
            scroll_offset,
            wheel_accum: _,
        } => json!({
            "input": input,
            "cursor": cursor,
            "history_len": history.len(),
            "history_idx": history_idx,
            "scrollback_len": scrollback.len(),
            "scrollback": scrollback
                .iter()
                .skip(scrollback.len().saturating_sub(CONSOLE_SCROLLBACK_TAIL))
                .map(console_line_json)
                .collect::<Vec<_>>(),
            "completion_count": completions.len(),
            "completion_idx": completion_idx,
            "scroll_offset": scroll_offset,
        }),
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl OpenPicker {
    /// An open picker as `query.editors` reports it. `hue_deg` is in
    /// `[0, 360)`, `sat` and `val` in `[0, 1]`: the HSV the wheel would
    /// commit. `mode` is `Standalone` or `Contextual`.
    ///
    /// **Omitted by name:** the cached layout, the glyph-measurement
    /// caches, the wheel's screen placement and the dynamic-apply key.
    /// That is all geometry.
    fn to_ipc_json(&self) -> Value {
        let mode = match &self.mode {
            PickerMode::Standalone => json!({ "kind": "Standalone" }),
            PickerMode::Contextual { seed_hsv } => json!({
                "kind": "Contextual",
                "seed_hsv": { "h": f32_json(seed_hsv.0), "s": f32_json(seed_hsv.1), "v": f32_json(seed_hsv.2) },
            }),
        };
        let gesture = self.gesture.as_ref().map(|g| match g {
            PickerGesture::Move { .. } => "Move",
            PickerGesture::Resize { .. } => "Resize",
        });
        json!({
            "mode": mode,
            "hue_deg": f32_json(self.hue_deg),
            "sat": f32_json(self.sat),
            "val": f32_json(self.val),
            "size_scale": f32_json(self.size_scale),
            "gesture": gesture,
            "hover_preview": self.hover_preview.map(|(h, s, v)| json!({
                "h": f32_json(h), "s": f32_json(s), "v": f32_json(v),
            })),
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// The color picker as `query.editors` reports it: `null` when closed.
pub(super) fn picker_to_ipc_json(state: &ColorPickerState) -> Value {
    match state {
        ColorPickerState::Closed => Value::Null,
        ColorPickerState::Open(open) => open.to_ipc_json(),
    }
}

/// Which modals are open, as plain flags: what the key routing and
/// `query.mode`'s `modals` read. The browser has neither.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct OpenModals {
    pub(super) console: bool,
    pub(super) picker: bool,
}

/// Where keystrokes go, as the keyboard handler routes them
/// (`event_keyboard.rs`): the **console**, when open, takes every key;
/// the **color picker**, when open, claims only its own keys, so it
/// appears in `modals` and is never the `key_owner`; with neither, a key
/// reaches normal dispatch (`"none"`).
fn key_routing(open: OpenModals) -> (Vec<&'static str>, &'static str) {
    let mut modals = Vec::new();
    if open.console {
        modals.push("console");
    }
    if open.picker {
        modals.push("color_picker");
    }
    let key_owner = if open.console { "console" } else { "none" };
    (modals, key_owner)
}

/// `query.mode`'s reply: the interaction mode and the drag state
/// machine (`drag`, as the target projects it — `no_drag_state_json`
/// where there is none), which modals are open and who receives an
/// ordinary key ([`key_routing`]), and where the pointer is, in
/// **surface** pixels.
pub(super) fn mode_to_ipc_json(
    mode: &InteractionMode,
    drag: Value,
    open: OpenModals,
    cursor_pos: (f64, f64),
) -> Value {
    let (modals, key_owner) = key_routing(open);
    json!({
        "interaction_mode": mode.to_ipc_json(),
        "drag_state": drag,
        "modals": modals,
        "key_owner": key_owner,
        "cursor_pos": xy_f64(cursor_pos),
    })
}

/// `query.editors`' reply: each modal's live state, as the target
/// projects it (`null` when closed, and always `null` for one the target
/// does not have).
pub(super) fn editors_to_ipc_json(console: Value, picker: Value) -> Value {
    json!({
        "console": console,
        "color_picker": picker,
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// The console steals every key; the picker is a modal that never
    /// owns an ordinary key; with neither, keys reach dispatch.
    #[test]
    fn test_key_routing_names_the_modals_and_the_key_owner() {
        assert_eq!(key_routing(OpenModals::default()), (vec![], "none"));
        assert_eq!(
            key_routing(OpenModals {
                console: false,
                picker: true
            }),
            (vec!["color_picker"], "none")
        );
        assert_eq!(
            key_routing(OpenModals {
                console: true,
                picker: true
            }),
            (vec!["console", "color_picker"], "console")
        );
    }

    /// Every drag state projects with its own kind, and a pending press
    /// names its element only when there is one.
    #[test]
    fn test_drag_states_project_with_their_kind() {
        use super::super::ClickHit;
        let cases = [
            (DragState::None, "None"),
            (
                DragState::Pending {
                    start_pos: (1.0, 2.0),
                    hit: ClickHit::Empty,
                },
                "Pending",
            ),
            (
                DragState::Panning {
                    owner: PanOwner::Toggled,
                },
                "Panning",
            ),
            (
                DragState::SelectingRect {
                    start_canvas: glam::Vec2::ZERO,
                    current_canvas: glam::Vec2::ONE,
                },
                "SelectingRect",
            ),
        ];
        for (drag, kind) in cases {
            let json = drag.to_ipc_json(None);
            assert_eq!(json["kind"], json!(kind), "{json}");
        }
        let pending = DragState::Pending {
            start_pos: (1.0, 2.0),
            hit: ClickHit::Empty,
        };
        assert_eq!(pending.to_ipc_json(None)["hit"], Value::Null);
    }

    /// A closed console and a closed picker are `null`; an open console
    /// reports its input and only the tail of its scrollback.
    #[test]
    fn test_editors_report_the_open_console_and_null_for_closed_ones() {
        assert_eq!(console_to_ipc_json(&ConsoleState::Closed), Value::Null);
        assert_eq!(picker_to_ipc_json(&ColorPickerState::Closed), Value::Null);
        let mut open = ConsoleState::open(vec!["fps on".into()]);
        if let ConsoleState::Open {
            input, scrollback, ..
        } = &mut open
        {
            input.push_str("he");
            for i in 0..CONSOLE_SCROLLBACK_TAIL + 5 {
                scrollback.push(ConsoleLine::Input(format!("line {i}")));
            }
        }
        let json = console_to_ipc_json(&open);
        assert_eq!(json["input"], json!("he"));
        assert_eq!(json["history_len"], json!(1));
        assert_eq!(json["scrollback_len"], json!(CONSOLE_SCROLLBACK_TAIL + 5));
        assert_eq!(
            json["scrollback"].as_array().unwrap().len(),
            CONSOLE_SCROLLBACK_TAIL
        );
        assert_eq!(
            editors_to_ipc_json(json.clone(), Value::Null),
            json!({ "console": json, "color_picker": null })
        );
    }

    fn open_picker(mode: PickerMode) -> ColorPickerState {
        use crate::application::color_picker::CROSSHAIR_CENTER_CELL;
        ColorPickerState::Open(Box::new(OpenPicker {
            mode,
            hue_deg: 120.0,
            sat: 0.5,
            val: 0.25,
            last_cursor_pos: None,
            max_cell_advance: 16.0,
            max_ring_advance: 24.0,
            measurement_font_size: 16.0,
            arm_top_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_bottom_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_left_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_right_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            preview_ink_offset: (0.0, 0.0),
            layout: None,
            center_override: None,
            size_scale: 1.0,
            gesture: None,
            hovered_hit: None,
            hover_preview: None,
            pending_error_flash: false,
            last_dynamic_apply: None,
        }))
    }

    /// An open picker reports its mode, the HSV it would commit and its
    /// gesture; a contextual one carries the seed it opened on. Control:
    /// the closed picker is `null`, so the open reply is the open state's.
    #[test]
    fn test_an_open_picker_reports_its_mode_and_hsv() {
        let json = picker_to_ipc_json(&open_picker(PickerMode::Standalone));
        assert_eq!(json["mode"], json!({ "kind": "Standalone" }));
        assert_eq!(json["hue_deg"], json!(120.0));
        assert_eq!(json["sat"], json!(0.5));
        assert_eq!(json["val"], json!(0.25));
        assert_eq!(json["gesture"], Value::Null);
        let seeded = picker_to_ipc_json(&open_picker(PickerMode::Contextual {
            seed_hsv: (10.0, 0.5, 0.75),
        }));
        assert_eq!(
            seeded["mode"],
            json!({ "kind": "Contextual", "seed_hsv": { "h": 10.0, "s": 0.5, "v": 0.75 } })
        );
        assert_eq!(picker_to_ipc_json(&ColorPickerState::Closed), Value::Null);
    }
}
